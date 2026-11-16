use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Local;
use inquire::{Confirm, InquireError, MultiSelect, Select, Text};

use crate::backup;
use crate::compat::{self, Issue, IssueKind, hard_count, warning_count};
use crate::config;
use crate::factorio;
use crate::i18n::Lang;
use crate::modlist::ModList;
use crate::modscan::{self, ScanResult};
use crate::operations;
use crate::pack;
use crate::types::{InstalledMod, MemberRef, PackSpec, Version};

pub struct UiOptions {
    pub mods_dir: Option<PathBuf>,
    pub lang: Option<String>,
    pub backup_days: Option<u32>,
    pub program_dir: PathBuf,
}

#[derive(Debug)]
struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("operation cancelled")
    }
}

impl std::error::Error for Cancelled {}

fn ask<T>(result: std::result::Result<T, InquireError>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            Err(anyhow::Error::new(Cancelled))
        }
        Err(error) => Err(anyhow::anyhow!("terminal prompt failed: {error}")),
    }
}

fn is_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Cancelled>().is_some()
}

struct AppContext {
    lang: Lang,
    mods_dir: PathBuf,
    modlist: ModList,
    scan: ScanResult,
    backup_dir: PathBuf,
    backup_days: u32,
}

impl AppContext {
    fn load(lang: Lang, mods_dir: PathBuf, backup_dir: PathBuf, backup_days: u32) -> Result<Self> {
        let modlist = load_mod_list(lang, &mods_dir)?;
        let scan = modscan::scan_mods(&mods_dir, &modlist.enabled_map(), lang);
        Ok(Self {
            lang,
            mods_dir,
            modlist,
            scan,
            backup_dir,
            backup_days,
        })
    }

    fn reload(&mut self) -> Result<()> {
        self.modlist = load_mod_list(self.lang, &self.mods_dir)?;
        self.scan = modscan::scan_mods(&self.mods_dir, &self.modlist.enabled_map(), self.lang);
        Ok(())
    }

    fn cleanup_backups(&self) {
        if let Err(error) = backup::cleanup(&self.backup_dir, self.backup_days) {
            println!(
                "{} {error:#}",
                self.lang
                    .t("Could not clean old backups:", "无法清理过期备份：")
            );
        }
    }
}

fn load_mod_list(lang: Lang, mods_dir: &Path) -> Result<ModList> {
    let path = mods_dir.join("mod-list.json");
    if !path.is_file() {
        bail!(
            "{} {}",
            lang.t("mod-list.json not found at", "找不到 mod-list.json："),
            path.display()
        );
    }
    ModList::load(&path).with_context(|| {
        format!(
            "{} {}",
            lang.t("cannot load mod-list.json:", "无法加载 mod-list.json："),
            path.display()
        )
    })
}

pub fn run(options: UiOptions) -> Result<()> {
    let (mut config, config_warning) = config::load(&options.program_dir);
    if factorio::is_factorio_running()? {
        bail!(
            "Factorio is running. Close the game completely before using ModpackBuilder. / Factorio 正在运行，请先完全关闭游戏。"
        );
    }

    let mut save_language = false;
    let lang = if let Some(code) = options.lang.as_deref() {
        Lang::parse(code).ok_or_else(|| {
            anyhow::anyhow!("unsupported --lang value {code:?}; use \"en\" or \"zh-CN\"")
        })?
    } else if let Some(lang) = config.lang_code().and_then(Lang::parse) {
        lang
    } else {
        let choice = match ask(Select::new(
            "Select language / 选择语言",
            vec!["English".to_string(), "简体中文".to_string()],
        )
        .with_help_message("↑↓ to move / 移动 · Enter to select / 回车选择 · Esc to cancel / 取消")
        .prompt())
        {
            Ok(choice) => choice,
            Err(error) if is_cancelled(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        save_language = true;
        if choice == "简体中文" {
            Lang::Zh
        } else {
            Lang::En
        }
    };

    factorio::ensure_not_running(lang)?;
    if save_language {
        config.lang = Some(lang.code().to_string());
        if let Err(error) = config::save(&options.program_dir, &config) {
            println!(
                "{} {error:#}",
                lang.t("Could not save the language setting:", "无法保存语言设置：")
            );
        }
    }

    let mods_dir = factorio::resolve_mods_dir(options.mods_dir.as_deref()).context(lang.t(
        "Cannot locate the Factorio mods folder.",
        "无法找到 Factorio 模组文件夹。",
    ))?;
    let backup_dir = options.program_dir.join(backup::BACKUP_DIR);
    let backup_days = options.backup_days.unwrap_or_else(|| config.backup_days(7));
    let mut ctx = AppContext::load(lang, mods_dir, backup_dir, backup_days)?;

    print_header(&ctx);
    if let Some(warning) = config_warning {
        println!(
            "{} {warning}",
            lang.t("Configuration warning:", "配置警告：")
        );
    }
    ctx.cleanup_backups();
    print_scan_summary(&ctx);

    loop {
        let lang = ctx.lang;
        let message = match lang {
            Lang::En => format!(
                "Main menu — {} selectable mods · {} modpacks",
                ctx.scan.compatible_count(),
                ctx.scan.zip_packs().len()
            ),
            Lang::Zh => format!(
                "主菜单 — {} 个可选模组 · {} 个模组包",
                ctx.scan.compatible_count(),
                ctx.scan.zip_packs().len()
            ),
        };
        let choices = vec![
            lang.t("Create a modpack", "创建模组包").to_string(),
            lang.t("Edit a modpack", "编辑模组包").to_string(),
            lang.t("Delete a modpack", "删除模组包").to_string(),
            lang.t("Settings", "设置").to_string(),
            lang.t("Exit", "退出").to_string(),
        ];
        let selected = match ask(Select::new(&message, choices)
            .with_help_message(lang.t(
                "↑↓ to move · Enter to select · Type to filter · Esc to exit",
                "↑↓ 移动 · 回车选择 · 输入以筛选 · Esc 退出",
            ))
            .raw_prompt())
        {
            Ok(selected) => selected,
            Err(error) if is_cancelled(&error) => {
                println!("{}", lang.t("Cancelled.", "已取消。"));
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let result = match selected.index {
            0 => create_flow(&mut ctx),
            1 => edit_flow(&mut ctx),
            2 => delete_flow(&mut ctx),
            3 => settings_flow(&mut ctx, &mut config, &options),
            _ => break,
        };
        match result {
            Ok(()) => {}
            Err(error) if is_cancelled(&error) => {
                println!("{}", lang.t("Cancelled.", "已取消。"));
            }
            Err(error) => return Err(error),
        }
    }

    println!("{}", ctx.lang.t("Goodbye.", "再见。"));
    Ok(())
}

fn settings_flow(
    ctx: &mut AppContext,
    config: &mut config::Config,
    options: &UiOptions,
) -> Result<()> {
    loop {
        let lang = ctx.lang;
        let language_override = if options.lang.is_some() {
            lang.t(" (--lang override)", "（--lang 覆盖）")
        } else {
            ""
        };
        let backup_override = if options.backup_days.is_some() {
            lang.t(" (--backup-days override)", "（--backup-days 覆盖）")
        } else {
            ""
        };
        let backup_value = if ctx.backup_days == 0 {
            lang.t("0 (keep forever)", "0（永久保留）").to_string()
        } else {
            ctx.backup_days.to_string()
        };
        let choices = vec![
            format!(
                "{}: {}{}",
                lang.t("Interface language", "界面语言"),
                lang.display_name(),
                language_override
            ),
            format!(
                "{}: {}{}",
                lang.t("Backup retention (days)", "备份保留天数"),
                backup_value,
                backup_override
            ),
        ];
        let selected = match ask(Select::new(lang.t("Settings", "设置"), choices)
            .with_help_message(lang.t(
                "↑↓ to move · Enter to select · Esc to go back",
                "↑↓ 移动 · 回车选择 · Esc 返回",
            ))
            .raw_prompt())
        {
            Ok(selected) => selected,
            Err(error) if is_cancelled(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        let result = match selected.index {
            0 => language_setting(ctx, config, options),
            1 => backup_days_setting(ctx, config, options),
            _ => return Ok(()),
        };
        match result {
            Ok(()) => {}
            Err(error) if is_cancelled(&error) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn language_setting(
    ctx: &mut AppContext,
    config: &mut config::Config,
    options: &UiOptions,
) -> Result<()> {
    let lang = ctx.lang;
    let saved_lang = config.lang_code().and_then(Lang::parse).unwrap_or(ctx.lang);
    let selected = ask(Select::new(
        lang.t("Interface language", "界面语言"),
        vec![
            Lang::En.display_name().to_string(),
            Lang::Zh.display_name().to_string(),
        ],
    )
    .with_starting_cursor(match saved_lang {
        Lang::En => 0,
        Lang::Zh => 1,
    })
    .with_help_message(lang.t(
        "↑↓ to move · Enter to select · Esc to go back",
        "↑↓ 移动 · 回车选择 · Esc 返回",
    ))
    .raw_prompt())?;
    let new_lang = if selected.index == 1 {
        Lang::Zh
    } else {
        Lang::En
    };

    let mut updated = config.clone();
    updated.lang = Some(new_lang.code().to_string());
    if !save_settings(lang, options, config, updated) {
        return Ok(());
    }

    if let Some(code) = options.lang.as_deref() {
        match lang {
            Lang::En => println!(
                "Saved as the new default language. This run continues to use --lang={code}; the new default will apply on a future start without that option."
            ),
            Lang::Zh => println!(
                "已保存为新的默认语言。本次运行仍使用 --lang={code}；以后不带该参数启动时生效。"
            ),
        }
    } else {
        ctx.lang = new_lang;
        println!(
            "{}",
            new_lang.t("Interface language updated.", "界面语言已更新。")
        );
    }
    Ok(())
}

fn backup_days_setting(
    ctx: &mut AppContext,
    config: &mut config::Config,
    options: &UiOptions,
) -> Result<()> {
    let lang = ctx.lang;
    let configured_days = config.backup_days(ctx.backup_days);
    let value = prompt_text(
        lang,
        lang.t("Backup retention in days", "备份保留天数"),
        &configured_days.to_string(),
        Some(lang.t(
            "0 keeps backups forever. Changing this value never deletes backups immediately.",
            "0 表示永久保留；修改后不会立即删除备份。",
        )),
        |value| validate_backup_days(lang, value),
    )?;
    let days = value.parse::<u32>().context("invalid backup retention")?;

    let mut updated = config.clone();
    updated.backup_days = Some(days);
    if !save_settings(lang, options, config, updated) {
        return Ok(());
    }

    if let Some(cli_days) = options.backup_days {
        match lang {
            Lang::En => println!(
                "Saved as the new default. This run continues to use --backup-days={cli_days}; the new default will apply on a future start without that option."
            ),
            Lang::Zh => println!(
                "已保存为新的默认值。本次运行仍使用 --backup-days={cli_days}；以后不带该参数启动时生效。"
            ),
        }
    } else {
        ctx.backup_days = days;
        println!(
            "{}",
            lang.t(
                "Backup retention updated. Old backups will still be cleaned only at startup and after successful write operations.",
                "备份保留天数已更新。旧备份仍只会在启动及成功增删改后清理。"
            )
        );
    }
    Ok(())
}

fn save_settings(
    lang: Lang,
    options: &UiOptions,
    config: &mut config::Config,
    updated: config::Config,
) -> bool {
    match config::save(&options.program_dir, &updated) {
        Ok(()) => {
            *config = updated;
            true
        }
        Err(error) => {
            println!(
                "{} {error:#}",
                lang.t("Could not save settings:", "无法保存设置：")
            );
            false
        }
    }
}

fn print_header(ctx: &AppContext) {
    println!("{}", "=".repeat(68));
    println!(
        " ModpackBuilder {} — {}",
        env!("CARGO_PKG_VERSION"),
        ctx.lang.t("Modpack Builder", "模组包创建器")
    );
    println!(
        " {}",
        ctx.lang.t("Target: Factorio 2.1", "目标版本：Factorio 2.1")
    );
    println!(
        " {}: {}",
        ctx.lang.t("Mods folder", "模组目录"),
        ctx.mods_dir.display()
    );
    println!(
        " {}: {}",
        ctx.lang.t("Backups", "备份目录"),
        ctx.backup_dir.display()
    );
    println!("{}", "=".repeat(68));
}

fn print_scan_summary(ctx: &AppContext) {
    let lang = ctx.lang;
    let builtins = ctx
        .scan
        .mods
        .iter()
        .filter(|installed| installed.is_builtin())
        .count();
    let total = ctx.scan.mods.len();
    println!(
        "{} {} {}",
        lang.t("Scanned", "已扫描"),
        total,
        lang.t("mods", "个模组")
    );
    match lang {
        Lang::En => println!(
            "  built-in: {builtins} · compatible 2.1: {} · incompatible: {}",
            ctx.scan.compatible_count(),
            ctx.scan.incompatible_count()
        ),
        Lang::Zh => println!(
            "  内置：{builtins} · 兼容 2.1：{} · 不兼容：{}",
            ctx.scan.compatible_count(),
            ctx.scan.incompatible_count()
        ),
    }
    match lang {
        Lang::En => println!(
            "  modpacks: {} zip · {} folder",
            ctx.scan.zip_packs().len(),
            ctx.scan.folder_pack_count()
        ),
        Lang::Zh => println!(
            "  模组包：{} 个压缩包 · {} 个文件夹",
            ctx.scan.zip_packs().len(),
            ctx.scan.folder_pack_count()
        ),
    }
    if ctx.scan.folder_pack_count() > 0 {
        println!(
            "{}",
            lang.t(
                "Folder modpacks are recognized but cannot be edited or deleted by this tool.",
                "文件夹形式的模组包可被识别，但本工具不支持编辑或删除。"
            )
        );
    }
    if ctx.scan.compatible_count() == 0 {
        println!(
            "{}",
            lang.t(
                "No selectable Factorio 2.1 mods were found.",
                "没有找到可选的 Factorio 2.1 模组。"
            )
        );
    }
    if !ctx.scan.warnings.is_empty() {
        println!(
            "{} ({})",
            lang.t("Scan warnings", "扫描警告"),
            ctx.scan.warnings.len()
        );
        for warning in ctx.scan.warnings.iter().take(25) {
            println!("  - {warning}");
        }
        if ctx.scan.warnings.len() > 25 {
            println!("  … {}", lang.t("more warnings", "更多警告"));
        }
    }
    println!();
}

fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

fn create_flow(ctx: &mut AppContext) -> Result<()> {
    let lang = ctx.lang;
    if ctx.scan.compatible_count() == 0 {
        println!(
            "{}",
            lang.t(
                "There are no compatible Factorio 2.1 mods to select.",
                "没有兼容 Factorio 2.1 的模组可供选择。"
            )
        );
        return Ok(());
    }
    let members = match choose_members(
        ctx,
        &BTreeSet::new(),
        &[],
        lang.t(
            "Select the member mods for the new modpack",
            "选择新模组包的成员模组",
        ),
    )? {
        Some(members) => members,
        None => return Ok(()),
    };

    let title = prompt_modpack_title(lang, lang.t("My Modpack", "我的模组包"))?;
    let existing = existing_names(ctx);
    let suggested_name = ctx.scan.suggest_internal_name();
    let internal_name = prompt_text(
        lang,
        lang.t("Internal name", "内部名"),
        &suggested_name,
        Some(lang.t(
            "Letters, digits, '-' and '_'; 3 to 50 characters. Used in the zip filename and dependencies.",
            "仅限字母、数字、'-' 和 '_'，3 至 50 个字符。用于 zip 文件名和依赖声明。",
        )),
        |value| validate_internal_name(lang, value, &existing),
    )?;
    let author = prompt_text(
        lang,
        lang.t("Author", "作者"),
        "ModpackBuilder",
        Some(lang.t(
            "Shown in the mod info and mod list.",
            "会显示在模组信息和模组列表中。",
        )),
        |value| validate_author(lang, value),
    )?;
    let version_text = prompt_text(
        lang,
        lang.t("Modpack version", "模组包版本"),
        "1.0.0",
        Some(lang.t(
            "Exactly three numbers, each between 0 and 65535.",
            "必须为三个数字，每段 0 到 65535。",
        )),
        |value| validate_version(lang, value),
    )?;
    let version = Version::parse(&version_text)?;
    let now = now_rfc3339();
    let spec = PackSpec {
        internal_name,
        title,
        version,
        author,
        members,
        created_at: now.clone(),
        updated_at: now,
    };
    let target = ctx.mods_dir.join(pack::pack_file_name(&spec));
    if target.exists() {
        println!(
            "{} {}",
            lang.t("Target file already exists:", "目标文件已存在："),
            target.display()
        );
        println!(
            "{}",
            lang.t(
                "Nothing was changed. Use Edit for an existing modpack, or choose another name or version.",
                "未做任何修改。已有模组包请使用“编辑”，或选择其他名称或版本。"
            )
        );
        return Ok(());
    }

    print_pack_summary(ctx, &spec);
    println!("{}: {}", lang.t("New file", "新文件"), target.display());
    println!(
        "{}",
        lang.t(
            "mod-list.json will receive one new disabled entry.",
            "mod-list.json 将追加一个禁用状态的新条目。"
        )
    );
    if !confirm(
        lang,
        lang.t("Create this modpack?", "创建这个模组包吗？"),
        true,
    )? {
        return Ok(());
    }

    factorio::ensure_not_running(lang)?;
    let report = match operations::create_pack(&ctx.mods_dir, &ctx.modlist, &ctx.backup_dir, &spec)
    {
        Ok(report) => report,
        Err(error) => {
            println!();
            println!(
                "{} {error:#}",
                lang.t(
                    "Creation failed; returning to the main menu:",
                    "创建失败，将返回主菜单："
                )
            );
            return Ok(());
        }
    };
    println!();
    println!(
        "{} {}",
        lang.t("Created", "已创建"),
        report.zip_path.display()
    );
    println!(
        "{}",
        lang.t(
            "The new entry is disabled. Enable the modpack in Factorio's Mods screen.",
            "新条目为禁用状态，请在 Factorio 的模组界面中启用该模组包。"
        )
    );
    println!(
        "{}",
        lang.t(
            "Member mods are not bundled. Share the modpack zip only.",
            "成员模组不会被打包。分享时只需传送这个模组包 zip。"
        )
    );
    println!(
        "{}: {}",
        lang.t("Backups", "备份目录"),
        ctx.backup_dir.display()
    );
    ctx.reload()?;
    ctx.cleanup_backups();
    Ok(())
}

fn edit_flow(ctx: &mut AppContext) -> Result<()> {
    let lang = ctx.lang;
    if ctx.scan.folder_pack_count() > 0 {
        println!(
            "{}",
            lang.t(
                "Folder modpacks are listed as installed but cannot be edited by this tool.",
                "文件夹形式的模组包会被识别，但本工具无法编辑。"
            )
        );
    }
    let packs: Vec<InstalledMod> = ctx.scan.zip_packs().into_iter().cloned().collect();
    if packs.is_empty() {
        println!(
            "{}",
            lang.t(
                "No editable modpacks were found.",
                "没有找到可编辑的模组包。"
            )
        );
        return Ok(());
    }
    let labels: Vec<String> = packs
        .iter()
        .map(|installed| pack_label(lang, installed))
        .collect();
    let selected = ask(Select::new(
        lang.t("Select a modpack to edit", "选择要编辑的模组包"),
        labels,
    )
    .with_help_message(lang.t(
        "↑↓ to move · Enter to select · Type to filter · Esc to go back",
        "↑↓ 移动 · 回车选择 · 输入以筛选 · Esc 返回",
    ))
    .raw_prompt())?;
    let pack = packs
        .get(selected.index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("invalid modpack selection"))?;
    let marker = match modscan::marker_for_edit(&pack) {
        Some(marker) => marker.clone(),
        None => {
            println!(
                "{}",
                lang.t(
                    "The selected mod is not a recognized modpack.",
                    "选中的模组不是可识别的模组包。"
                )
            );
            return Ok(());
        }
    };
    print_pack_details(ctx, &pack);

    let mut preselected = BTreeSet::new();
    let mut retained = Vec::new();
    let mut incompatible = Vec::new();
    for member in &marker.members {
        match ctx.scan.find(&member.name) {
            None => retained.push(MemberRef {
                name: member.name.clone(),
                version: member.version,
            }),
            Some(installed) if !installed.info.is_target_version() => incompatible.push(format!(
                "{} (factorio_version {})",
                member.name, installed.info.factorio_version
            )),
            Some(_) => {
                preselected.insert(member.name.to_ascii_lowercase());
            }
        }
    }
    if !retained.is_empty() {
        let names: Vec<&str> = retained.iter().map(|member| member.name.as_str()).collect();
        println!(
            "{} {}",
            lang.t(
                "Not installed but kept in the modpack:",
                "以下成员未安装，将保留在合集内："
            ),
            names.join(", ")
        );
    }
    if !incompatible.is_empty() {
        println!(
            "{} {}",
            lang.t(
                "Incompatible with Factorio 2.1:",
                "与 Factorio 2.1 不兼容："
            ),
            incompatible.join(", ")
        );
        if !confirm(
            lang,
            lang.t(
                "Remove these incompatible members from the modpack?",
                "要从模组包中移除这些不兼容成员吗？",
            ),
            true,
        )? {
            return Ok(());
        }
    }

    let title_default = if marker.title.trim().is_empty() {
        pack.title()
    } else {
        marker.title.as_str()
    };
    let title = prompt_modpack_title(lang, title_default)?;
    let version_default = pack.info.version.to_string();
    let version_text = prompt_text(
        lang,
        lang.t("Modpack version", "模组包版本"),
        &version_default,
        Some(lang.t(
            "Exactly three numbers, each between 0 and 65535.",
            "必须为三个数字，每段 0 到 65535。",
        )),
        |value| validate_version(lang, value),
    )?;
    let version = Version::parse(&version_text)?;
    let author_default = if marker.author.trim().is_empty() {
        "ModpackBuilder"
    } else {
        marker.author.as_str()
    };
    let author = prompt_text(
        lang,
        lang.t("Author", "作者"),
        author_default,
        Some(lang.t(
            "Shown in the mod info and mod list.",
            "会显示在模组信息和模组列表中。",
        )),
        |value| validate_author(lang, value),
    )?;

    let change_members = confirm(
        lang,
        lang.t("Change the member selection?", "要更改成员选择吗？"),
        !incompatible.is_empty(),
    )?;
    let members = if change_members {
        match choose_members(
            ctx,
            &preselected,
            &retained,
            lang.t(
                "Select the member mods for this modpack",
                "选择此模组包的成员模组",
            ),
        )? {
            Some(members) => members,
            None => return Ok(()),
        }
    } else {
        let mut members = retained.clone();
        let mut seen: BTreeSet<String> = members
            .iter()
            .map(|member| member.name.to_ascii_lowercase())
            .collect();
        for member in &marker.members {
            if let Some(installed) = ctx.scan.find(&member.name)
                && installed.info.is_target_version()
            {
                let key = installed.name().to_ascii_lowercase();
                if seen.insert(key) {
                    members.push(MemberRef {
                        name: installed.name().to_string(),
                        version: installed.info.version,
                    });
                }
            }
        }
        members.sort_by_key(|member| member.name.to_ascii_lowercase());
        if members.is_empty() {
            println!(
                "{}",
                lang.t(
                    "A modpack must contain at least one member.",
                    "模组包必须至少包含一个成员。"
                )
            );
            return Ok(());
        }
        let issues = compat::check(&members, &ctx.scan);
        print_issues(lang, &issues);
        if hard_count(&issues) > 0 {
            println!(
                "{}",
                lang.t(
                    "Resolve the blocking issues by changing the member selection.",
                    "请通过更改成员选择来解决阻止性问题。"
                )
            );
            return Ok(());
        }
        if warning_count(&issues) > 0
            && !confirm(
                lang,
                lang.t("Continue despite these warnings?", "仍要继续吗？"),
                true,
            )?
        {
            return Ok(());
        }
        members
    };

    let now = now_rfc3339();
    let created_at = if marker.created_at.trim().is_empty() {
        now.clone()
    } else {
        marker.created_at.clone()
    };
    let spec = PackSpec {
        internal_name: pack.name().to_string(),
        title,
        version,
        author,
        members,
        created_at,
        updated_at: now,
    };
    let current_path = pack
        .source
        .path()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let new_path = ctx.mods_dir.join(pack::pack_file_name(&spec));
    let same_file = current_path
        .file_name()
        .zip(new_path.file_name())
        .is_some_and(|(left, right)| {
            left.to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy())
        });
    if !same_file && new_path.exists() {
        println!(
            "{} {}",
            lang.t("Target file already exists:", "目标文件已存在："),
            new_path.display()
        );
        println!(
            "{}",
            lang.t(
                "Nothing was changed. Choose another version, or remove the conflicting file manually after inspection.",
                "未做任何修改。请选择其他版本，或在检查后手工处理冲突文件。"
            )
        );
        return Ok(());
    }

    print_pack_summary(ctx, &spec);
    println!(
        "{}: {}",
        lang.t("Current file", "当前文件"),
        current_path.display()
    );
    println!("{}: {}", lang.t("New file", "新文件"), new_path.display());
    if !confirm(
        lang,
        lang.t("Update this modpack?", "更新这个模组包吗？"),
        true,
    )? {
        return Ok(());
    }

    factorio::ensure_not_running(lang)?;
    let report =
        match operations::update_pack(&ctx.mods_dir, &ctx.modlist, &ctx.backup_dir, &pack, &spec) {
            Ok(report) => report,
            Err(error) => {
                println!();
                println!(
                    "{} {error:#}",
                    lang.t(
                        "Update failed; returning to the main menu:",
                        "更新失败，将返回主菜单："
                    )
                );
                return Ok(());
            }
        };
    println!();
    println!(
        "{} {}",
        lang.t("Updated", "已更新"),
        report.zip_path.display()
    );
    println!(
        "{}",
        lang.t(
            "Existing mod-list entries keep their enabled state; a missing entry is added disabled.",
            "已有的 mod-list 条目会保持启用状态；缺失的条目会以禁用状态追加。"
        )
    );
    println!(
        "{}: {}",
        lang.t("Backups", "备份目录"),
        ctx.backup_dir.display()
    );
    ctx.reload()?;
    ctx.cleanup_backups();
    Ok(())
}

fn delete_flow(ctx: &mut AppContext) -> Result<()> {
    let lang = ctx.lang;
    if ctx.scan.folder_pack_count() > 0 {
        println!(
            "{}",
            lang.t(
                "Folder modpacks are recognized but cannot be deleted by this tool.",
                "文件夹形式的模组包会被识别，但本工具无法删除。"
            )
        );
    }
    let packs: Vec<InstalledMod> = ctx.scan.zip_packs().into_iter().cloned().collect();
    if packs.is_empty() {
        println!(
            "{}",
            lang.t(
                "No deletable modpacks were found.",
                "没有找到可删除的模组包。"
            )
        );
        return Ok(());
    }
    let labels: Vec<String> = packs
        .iter()
        .map(|installed| pack_label(lang, installed))
        .collect();
    let selected = ask(Select::new(
        lang.t("Select a modpack to delete", "选择要删除的模组包"),
        labels,
    )
    .with_help_message(lang.t(
        "↑↓ to move · Enter to select · Type to filter · Esc to go back",
        "↑↓ 移动 · 回车选择 · 输入以筛选 · Esc 返回",
    ))
    .raw_prompt())?;
    let pack = packs
        .get(selected.index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("invalid modpack selection"))?;
    if modscan::marker_for_edit(&pack).is_none() {
        println!(
            "{}",
            lang.t(
                "The selected mod is not a recognized modpack.",
                "选中的模组不是可识别的模组包。"
            )
        );
        return Ok(());
    }
    print_pack_details(ctx, &pack);
    println!(
        "{}",
        lang.t(
            "Deleting removes the modpack zip, its mod-list entry and nothing else. Member mods are not changed.",
            "删除只会移除模组包 zip 及其 mod-list 条目，不会修改任何成员模组。"
        )
    );
    if !confirm(
        lang,
        lang.t(
            "Delete this modpack permanently? A backup will be kept.",
            "永久删除这个模组包吗？仍会保留备份。",
        ),
        false,
    )? {
        return Ok(());
    }

    factorio::ensure_not_running(lang)?;
    let report = match operations::delete_pack(&ctx.modlist, &ctx.backup_dir, &pack) {
        Ok(report) => report,
        Err(error) => {
            println!();
            println!(
                "{} {error:#}",
                lang.t(
                    "Deletion failed; returning to the main menu:",
                    "删除失败，将返回主菜单："
                )
            );
            return Ok(());
        }
    };
    println!();
    println!(
        "{} {}",
        lang.t("Deleted", "已删除"),
        report.zip_path.display()
    );
    println!(
        "{}: {}",
        lang.t("Backups", "备份目录"),
        ctx.backup_dir.display()
    );
    ctx.reload()?;
    ctx.cleanup_backups();
    Ok(())
}

fn choose_members(
    ctx: &AppContext,
    preselected: &BTreeSet<String>,
    retained: &[MemberRef],
    message: &str,
) -> Result<Option<Vec<MemberRef>>> {
    let lang = ctx.lang;
    loop {
        let selectable: Vec<&InstalledMod> = ctx
            .scan
            .mods
            .iter()
            .filter(|installed| !installed.is_builtin() && installed.info.is_target_version())
            .collect();
        if selectable.is_empty() {
            if retained.is_empty() {
                println!(
                    "{}",
                    lang.t(
                        "There are no compatible Factorio 2.1 mods to select.",
                        "没有兼容 Factorio 2.1 的模组可供选择。"
                    )
                );
                return Ok(None);
            }
            println!(
                "{}",
                lang.t(
                    "No compatible installed mods are available. The existing missing members will be kept.",
                    "没有可用的兼容模组。将保留现有的缺失成员。"
                )
            );
            if confirm(
                lang,
                lang.t("Keep the existing members?", "保留现有成员吗？"),
                true,
            )? {
                return Ok(Some(retained.to_vec()));
            }
            return Ok(None);
        }

        let labels: Vec<String> = selectable
            .iter()
            .map(|installed| member_label(lang, installed))
            .collect();
        let defaults: Vec<usize> = selectable
            .iter()
            .enumerate()
            .filter(|(_, installed)| preselected.contains(&installed.name().to_ascii_lowercase()))
            .map(|(index, _)| index)
            .collect();
        let help = lang.t(
            "Type to filter · Space selects · Enter confirms · Esc to go back",
            "输入以筛选 · 空格选择 · 回车确认 · Esc 返回",
        );
        let mut prompt = MultiSelect::new(message, labels)
            .with_help_message(help)
            .with_page_size(20);
        if !defaults.is_empty() {
            prompt = prompt.with_default(&defaults);
        }
        let response = ask(prompt.raw_prompt())?;
        if response.is_empty() {
            println!(
                "{}",
                lang.t(
                    "Select at least one member mod.",
                    "请至少选择一个成员模组。"
                )
            );
            continue;
        }

        let mut members: Vec<MemberRef> = Vec::new();
        let mut seen = BTreeSet::new();
        for selected in &response {
            let Some(installed) = selectable.get(selected.index) else {
                continue;
            };
            let key = installed.name().to_ascii_lowercase();
            if seen.insert(key) {
                members.push(MemberRef {
                    name: installed.name().to_string(),
                    version: installed.info.version,
                });
            }
        }
        for member in retained {
            let key = member.name.to_ascii_lowercase();
            if seen.insert(key) {
                members.push(member.clone());
            }
        }
        members.sort_by_key(|member| member.name.to_ascii_lowercase());

        let issues = compat::check(&members, &ctx.scan);
        print_issues(lang, &issues);
        if hard_count(&issues) > 0 {
            println!(
                "{}",
                lang.t(
                    "Blocking issues must be resolved before the modpack can be created.",
                    "必须先解决阻止性问题才能创建模组包。"
                )
            );
            if confirm(
                lang,
                lang.t("Choose members again?", "重新选择成员吗？"),
                true,
            )? {
                continue;
            }
            return Ok(None);
        }
        if warning_count(&issues) > 0
            && !confirm(
                lang,
                lang.t("Continue despite these warnings?", "仍要继续吗？"),
                true,
            )?
        {
            return Ok(None);
        }
        return Ok(Some(members));
    }
}

fn member_label(lang: Lang, installed: &InstalledMod) -> String {
    let mut suffix = String::new();
    if installed.enabled {
        suffix.push_str(if lang == Lang::En {
            " · enabled"
        } else {
            " · 已启用"
        });
    }
    if installed.is_pack() {
        suffix.push_str(if lang == Lang::En {
            " · modpack"
        } else {
            " · 模组包"
        });
    }
    if installed.duplicate_count > 1 {
        suffix.push_str(&format!(" · x{}", installed.duplicate_count));
    }
    format!(
        "{} ({}) v{}{}",
        installed.title(),
        installed.name(),
        installed.info.version,
        suffix
    )
}

fn pack_label(lang: Lang, installed: &InstalledMod) -> String {
    let state = if installed.enabled {
        lang.t("enabled", "已启用")
    } else {
        lang.t("disabled", "已禁用")
    };
    format!(
        "{} ({}) v{} · {}",
        installed.title(),
        installed.name(),
        installed.info.version,
        state
    )
}

fn print_pack_details(ctx: &AppContext, pack: &InstalledMod) {
    let lang = ctx.lang;
    println!();
    println!(
        "{}: {} ({})",
        lang.t("Modpack", "模组包"),
        pack.title(),
        pack.name()
    );
    println!("{}: {}", lang.t("Version", "版本"), pack.info.version);
    println!(
        "{}: {}",
        lang.t("File", "文件"),
        pack.source
            .path()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    );
    println!(
        "{}: {}",
        lang.t("Members", "成员数"),
        pack.marker()
            .map(|marker| marker.members.len())
            .unwrap_or(0)
    );
    println!(
        "{}: {}",
        lang.t("Status", "状态"),
        if pack.enabled {
            lang.t("enabled", "已启用")
        } else {
            lang.t("disabled", "已禁用")
        }
    );
    if let Some(marker) = pack.marker() {
        let names: Vec<&str> = marker
            .members
            .iter()
            .take(12)
            .map(|member| member.name.as_str())
            .collect();
        let mut text = names.join(", ");
        if marker.members.len() > 12 {
            text.push_str(", …");
        }
        println!("{}: {text}", lang.t("Member list", "成员列表"));
    }
    println!();
}

fn print_pack_summary(ctx: &AppContext, spec: &PackSpec) {
    let lang = ctx.lang;
    println!();
    println!("{}: {}", lang.t("Title", "标题"), spec.title);
    println!(
        "{}: {}",
        lang.t("Internal name", "内部名"),
        spec.internal_name
    );
    println!("{}: {}", lang.t("Version", "版本"), spec.version);
    println!("{}: {}", lang.t("Author", "作者"), spec.author);
    println!("{} ({}):", lang.t("Members", "成员"), spec.members.len());
    for member in spec.sorted_members() {
        println!("  - {} >= {}", member.name, member.version);
    }
    println!(
        "{}: {}",
        lang.t("Factorio version", "Factorio 版本"),
        pack::TARGET_FACTORIO_VERSION
    );
    println!();
}

fn print_issues(lang: Lang, issues: &[Issue]) {
    if issues.is_empty() {
        println!(
            "{}",
            lang.t(
                "No static compatibility issues found.",
                "静态兼容性检查通过，未发现问题。"
            )
        );
        return;
    }
    let hard: Vec<&Issue> = issues.iter().filter(|issue| issue.is_hard()).collect();
    let warnings: Vec<&Issue> = issues.iter().filter(|issue| !issue.is_hard()).collect();
    match lang {
        Lang::En => println!(
            "Compatibility check: {} blocking · {} warning(s)",
            hard.len(),
            warnings.len()
        ),
        Lang::Zh => println!(
            "兼容性检查：{} 个阻止性问题 · {} 个警告",
            hard.len(),
            warnings.len()
        ),
    }
    const MAX_PRINTED: usize = 40;
    for (printed, issue) in hard.iter().chain(warnings.iter()).enumerate() {
        if printed >= MAX_PRINTED {
            break;
        }
        println!("  - {}", format_issue(lang, issue));
    }
    if issues.len() > MAX_PRINTED {
        match lang {
            Lang::En => println!("  … {} more issue(s)", issues.len() - MAX_PRINTED),
            Lang::Zh => println!("  … 另有 {} 个问题", issues.len() - MAX_PRINTED),
        }
    }
}

fn format_issue(lang: Lang, issue: &Issue) -> String {
    let severity = if issue.is_hard() {
        lang.t("[blocking]", "[阻止]")
    } else {
        lang.t("[warning]", "[警告]")
    };
    let state = |enabled: bool| {
        if enabled {
            lang.t("currently enabled", "当前已启用")
        } else {
            lang.t("currently disabled", "当前已禁用")
        }
    };
    let detail = match &issue.kind {
        IssueKind::IncompatibleTarget { found } => format!(
            "{} {found}, {} 2.1",
            lang.t("factorio_version is", "factorio_version 为"),
            lang.t("not", "不是")
        ),
        IssueKind::MissingMember { version } => match lang {
            Lang::En => {
                format!("not installed (marker version {version}); Factorio will report it missing")
            }
            Lang::Zh => format!("未安装（标识记录版本 {version}），游戏内会提示缺失"),
        },
        IssueKind::InvalidDependency { raw } => {
            format!(
                "{} {raw}",
                lang.t(
                    "unparseable dependency declaration:",
                    "无法解析的依赖声明："
                )
            )
        }
        IssueKind::MissingRequired { dep, required } => {
            let req = required
                .as_deref()
                .map(|req| format!(" ({req})"))
                .unwrap_or_default();
            format!(
                "{} {dep}{req}",
                lang.t("missing required dependency", "缺少必需依赖")
            )
        }
        IssueKind::DependencyNotIncluded { dep, enabled } => match lang {
            Lang::En => format!(
                "required dependency {dep} is installed ({}) but not included in the pack; sharing recipients will need it",
                state(*enabled)
            ),
            Lang::Zh => format!(
                "必需依赖 {dep} 已安装（{}），但未包含在合集内；分享时接收方需要安装它",
                state(*enabled)
            ),
        },
        IssueKind::DependencyVersion {
            dep,
            required,
            found,
        } => match lang {
            Lang::En => {
                format!("dependency {dep} requires {required}, but version {found} is selected")
            }
            Lang::Zh => format!("依赖 {dep} 需要 {required}，但所选版本为 {found}"),
        },
        IssueKind::OptionalVersion {
            dep,
            required,
            found,
        } => match lang {
            Lang::En => format!("optional dependency {dep} requires {required}, found {found}"),
            Lang::Zh => format!("可选依赖 {dep} 需要 {required}，当前为 {found}"),
        },
        IssueKind::RecommendedMissing { dep } => format!(
            "{} {dep}",
            lang.t("missing recommended dependency", "缺少推荐依赖")
        ),
        IssueKind::RecommendedVersion {
            dep,
            required,
            found,
        } => match lang {
            Lang::En => format!("recommended dependency {dep} requires {required}, found {found}"),
            Lang::Zh => format!("推荐依赖 {dep} 需要 {required}，当前为 {found}"),
        },
        IssueKind::ConflictSelected { dep } => format!(
            "{} {dep} (!)",
            lang.t("conflicts with selected member", "与选中的模组冲突")
        ),
        IssueKind::ConflictInstalled { dep, enabled } => match lang {
            Lang::En => format!("conflicts with installed mod {dep} ({})", state(*enabled)),
            Lang::Zh => format!("与已安装模组 {dep} 冲突（{}）", state(*enabled)),
        },
    };
    format!("{severity} {}: {detail}", issue.member)
}

const TITLE_PREFIX: &str = "⚙️";

fn prompt_modpack_title(lang: Lang, default: &str) -> Result<String> {
    let mut default = default.to_string();
    loop {
        let entered = prompt_text(
            lang,
            lang.t("Modpack title", "模组包标题"),
            &default,
            Some(lang.t(
                "Up to 100 characters including the optional prefix; shown in Factorio's mod list.",
                "包括可选前缀在内最多 100 个字符，会显示在 Factorio 模组列表中。",
            )),
            |value| validate_title(lang, value),
        )?;
        let add_prefix = confirm(
            lang,
            lang.t(
                "Add the ⚙️ prefix to the display title?",
                "要在显示标题前添加 ⚙️ 前缀吗？",
            ),
            entered.starts_with(TITLE_PREFIX),
        )?;
        let title = set_title_prefix(&entered, add_prefix);
        match validate_title(lang, &title) {
            Ok(()) => return Ok(title),
            Err(error) => {
                println!("{}: {error}", lang.t("Invalid input", "输入无效"));
                default = entered;
            }
        }
    }
}

fn set_title_prefix(title: &str, add_prefix: bool) -> String {
    let title = title
        .strip_prefix(TITLE_PREFIX)
        .map(str::trim_start)
        .unwrap_or(title);
    if add_prefix {
        if title.is_empty() {
            TITLE_PREFIX.to_string()
        } else {
            format!("{TITLE_PREFIX} {title}")
        }
    } else {
        title.to_string()
    }
}

fn prompt_text(
    lang: Lang,
    message: &str,
    default: &str,
    help: Option<&str>,
    validate: impl Fn(&str) -> std::result::Result<(), String>,
) -> Result<String> {
    loop {
        let mut prompt = Text::new(message).with_default(default);
        if let Some(help) = help {
            prompt = prompt.with_help_message(help);
        }
        let value = ask(prompt.prompt())?.trim().to_string();
        match validate(&value) {
            Ok(()) => return Ok(value),
            Err(error) => println!("{}: {error}", lang.t("Invalid input", "输入无效")),
        }
    }
}

fn confirm(_lang: Lang, message: &str, default: bool) -> Result<bool> {
    ask(Confirm::new(message).with_default(default).prompt())
}

fn existing_names(ctx: &AppContext) -> BTreeSet<String> {
    ctx.scan
        .mods
        .iter()
        .map(|installed| installed.name().to_ascii_lowercase())
        .chain(
            ctx.modlist
                .entries()
                .iter()
                .map(|entry| entry.name.to_ascii_lowercase()),
        )
        .collect()
}

fn validate_title(lang: Lang, value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err(lang
            .t("The title must not be empty.", "标题不能为空。")
            .to_string());
    }
    if value.chars().count() > 100 {
        return Err(lang
            .t(
                "The title must not exceed 100 characters.",
                "标题不能超过 100 个字符。",
            )
            .to_string());
    }
    if value.chars().any(char::is_control) {
        return Err(lang
            .t(
                "The title must not contain control characters.",
                "标题不能包含控制字符。",
            )
            .to_string());
    }
    Ok(())
}

fn validate_internal_name(
    lang: Lang,
    value: &str,
    existing: &BTreeSet<String>,
) -> std::result::Result<(), String> {
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(lang
            .t(
                "Use only ASCII letters, digits, '-' and '_'.",
                "只能使用 ASCII 字母、数字、'-' 和 '_'。",
            )
            .to_string());
    }
    let length = value.len();
    if !(3..=50).contains(&length) {
        return Err(lang
            .t(
                "The internal name must contain 3 to 50 characters.",
                "内部名必须为 3 至 50 个字符。",
            )
            .to_string());
    }
    if !value
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
    {
        return Err(lang
            .t(
                "The internal name must start with a letter or digit.",
                "内部名必须以字母或数字开头。",
            )
            .to_string());
    }
    if crate::types::is_builtin_name(value) || value.eq_ignore_ascii_case("core") {
        return Err(lang
            .t(
                "This name is reserved by Factorio.",
                "该名称被 Factorio 保留。",
            )
            .to_string());
    }
    if existing.contains(&value.to_ascii_lowercase()) {
        return Err(lang
            .t(
                "This internal name already exists. Use Edit for an existing modpack.",
                "该内部名已存在。编辑已有模组包请使用“编辑”功能。",
            )
            .to_string());
    }
    Ok(())
}

fn validate_backup_days(lang: Lang, value: &str) -> std::result::Result<(), String> {
    value.parse::<u32>().map(|_| ()).map_err(|_| {
        lang.t(
            "Enter a whole number between 0 and 4294967295.",
            "请输入 0 到 4294967295 之间的整数。",
        )
        .to_string()
    })
}

fn validate_version(lang: Lang, value: &str) -> std::result::Result<(), String> {
    Version::parse(value).map(|_| ()).map_err(|_| {
        lang.t(
            "Use exactly three numbers, for example 1.0.0.",
            "请使用三个数字，例如 1.0.0。",
        )
        .to_string()
    })
}

fn validate_author(lang: Lang, value: &str) -> std::result::Result<(), String> {
    if value.trim().is_empty() {
        return Err(lang
            .t("The author must not be empty.", "作者不能为空。")
            .to_string());
    }
    if value.chars().any(char::is_control) {
        return Err(lang
            .t(
                "The author must not contain control characters.",
                "作者不能包含控制字符。",
            )
            .to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn options(program_dir: &Path) -> UiOptions {
        UiOptions {
            mods_dir: None,
            lang: None,
            backup_days: None,
            program_dir: program_dir.to_path_buf(),
        }
    }

    #[test]
    fn validates_backup_retention_days() {
        for value in ["0", "7", "30", "4294967295"] {
            assert!(validate_backup_days(Lang::En, value).is_ok());
        }
        for value in ["", "-1", "1.5", "seven", "4294967296"] {
            assert!(validate_backup_days(Lang::Zh, value).is_err());
        }
    }

    #[test]
    fn adds_normalizes_and_removes_title_prefix() {
        assert_eq!(set_title_prefix("My Pack", true), "⚙️ My Pack");
        assert_eq!(set_title_prefix("⚙️ My Pack", true), "⚙️ My Pack");
        assert_eq!(set_title_prefix("⚙️My Pack", true), "⚙️ My Pack");
        assert_eq!(set_title_prefix("⚙️ My Pack", false), "My Pack");
        assert_eq!(set_title_prefix("⚙️", false), "");
    }

    #[test]
    fn saves_settings_before_applying_them() {
        let temp = tempfile::tempdir().unwrap();
        let options = options(temp.path());
        let mut current = config::Config {
            lang: Some("en".to_string()),
            backup_days: Some(7),
        };
        let updated = config::Config {
            lang: Some("zh-CN".to_string()),
            backup_days: Some(30),
        };

        assert!(save_settings(Lang::En, &options, &mut current, updated));
        assert_eq!(current.lang_code(), Some("zh-CN"));
        assert_eq!(current.backup_days(7), 30);

        let (saved, warning) = config::load(temp.path());
        assert!(warning.is_none());
        assert_eq!(saved.lang_code(), Some("zh-CN"));
        assert_eq!(saved.backup_days(7), 30);
    }

    #[test]
    fn keeps_current_settings_when_saving_fails() {
        let temp = tempfile::tempdir().unwrap();
        let missing_dir = temp.path().join("missing");
        let options = options(&missing_dir);
        let mut current = config::Config {
            lang: Some("en".to_string()),
            backup_days: Some(7),
        };
        let updated = config::Config {
            lang: Some("zh-CN".to_string()),
            backup_days: Some(0),
        };

        assert!(!save_settings(Lang::En, &options, &mut current, updated));
        assert_eq!(current.lang_code(), Some("en"));
        assert_eq!(current.backup_days(7), 7);
        assert!(!missing_dir.exists());
    }
}
