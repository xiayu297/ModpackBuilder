use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sysinfo::System;

use crate::i18n::Lang;

pub fn default_mods_dir() -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| {
        anyhow::anyhow!(
            "%APPDATA% is not defined; pass --mods-dir with the Factorio mods directory"
        )
    })?;
    let path = PathBuf::from(appdata).join("Factorio").join("mods");
    validate_mods_dir(&path)?;
    Ok(path)
}

pub fn resolve_mods_dir(cli_value: Option<&Path>) -> Result<PathBuf> {
    match cli_value {
        Some(path) => {
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .context("cannot read the current directory")?
                    .join(path)
            };
            validate_mods_dir(&absolute)?;
            Ok(absolute)
        }
        None => default_mods_dir(),
    }
}

fn validate_mods_dir(path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Factorio mods directory not found: {}", path.display()))?;
    if !metadata.is_dir() {
        bail!("Factorio mods path is not a directory: {}", path.display());
    }
    Ok(())
}

pub fn is_factorio_running() -> Result<bool> {
    let system = System::new_all().context("cannot enumerate running processes")?;
    Ok(system.processes().values().any(|process| {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        stem == "factorio"
    }))
}

pub fn ensure_not_running(lang: Lang) -> Result<()> {
    if is_factorio_running()? {
        bail!(
            "{}",
            lang.t(
                "Factorio is running. Close the game completely before changing mod files.",
                "Factorio 正在运行。请在修改模组文件前完全关闭游戏。"
            )
        );
    }
    Ok(())
}
