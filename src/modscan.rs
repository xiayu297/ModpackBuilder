use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use zip::ZipArchive;

use crate::i18n::Lang;
use crate::types::{BUILTIN_MODS, InstalledMod, Marker, ModInfo, ModSource, builtin_info};

const MAX_INFO_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub mods: Vec<InstalledMod>,
    pub warnings: Vec<String>,
}

impl ScanResult {
    pub fn find(&self, name: &str) -> Option<&InstalledMod> {
        self.mods
            .iter()
            .find(|installed| installed.name() == name)
            .or_else(|| {
                self.mods
                    .iter()
                    .find(|installed| installed.name().eq_ignore_ascii_case(name))
            })
    }

    pub fn contains_name(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    pub fn installed_map(&self) -> BTreeMap<String, &InstalledMod> {
        self.mods
            .iter()
            .map(|installed| (installed.name().to_ascii_lowercase(), installed))
            .collect()
    }

    pub fn zip_packs(&self) -> Vec<&InstalledMod> {
        self.mods
            .iter()
            .filter(|installed| installed.is_pack() && installed.source.is_zip())
            .collect()
    }

    pub fn folder_pack_count(&self) -> usize {
        self.mods
            .iter()
            .filter(|installed| installed.is_folder_pack())
            .count()
    }

    pub fn compatible_count(&self) -> usize {
        self.mods
            .iter()
            .filter(|installed| !installed.is_builtin() && installed.info.is_target_version())
            .count()
    }

    pub fn incompatible_count(&self) -> usize {
        self.mods
            .iter()
            .filter(|installed| !installed.is_builtin() && !installed.info.is_target_version())
            .count()
    }

    pub fn suggest_internal_name(&self) -> String {
        let base = format!("modpack_{}", chrono::Local::now().format("%Y%m%d"));
        if !self.contains_name(&base) {
            return base;
        }
        for suffix in 2..=999 {
            let candidate = format!("{base}_{suffix}");
            if !self.contains_name(&candidate) {
                return candidate;
            }
        }
        format!("modpack_{}", chrono::Local::now().timestamp())
    }
}

#[derive(Debug)]
struct ZipCandidate {
    depth: usize,
    root: String,
    bytes: Vec<u8>,
}

pub fn scan_mods(mods_dir: &Path, enabled: &BTreeMap<String, bool>, lang: Lang) -> ScanResult {
    let mut warnings = Vec::new();
    let mut mods = Vec::new();

    let entries = match fs::read_dir(mods_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "{}: {error}",
                lang.t("cannot read mods directory", "无法读取模组目录")
            ));
            return ScanResult { mods, warnings };
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                warnings.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            match read_zip_mod(&path, lang) {
                Ok((info, marker, extra_warnings)) => {
                    for warning in extra_warnings {
                        warnings.push(format!("{}: {warning}", file_label(&path)));
                    }
                    let expected = expected_zip_name(&info);
                    let actual = path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !actual.eq_ignore_ascii_case(&expected) {
                        warnings.push(format!(
                            "{}: {} {}",
                            file_label(&path),
                            lang.t(
                                "zip filename does not match the mod name and version (expected",
                                "压缩包文件名与模组名称/版本不一致（应为"
                            ),
                            expected
                        ));
                    }
                    let enabled_state = enabled
                        .get(&info.name.to_ascii_lowercase())
                        .copied()
                        .unwrap_or(false);
                    let marker = marker.filter(|marker| marker_is_valid(marker, &info));
                    mods.push(InstalledMod {
                        info,
                        source: ModSource::Zip(path.clone()),
                        enabled: enabled_state,
                        marker,
                        duplicate_count: 1,
                    });
                }
                Err(error) => warnings.push(format!(
                    "{}: {}{error}",
                    file_label(&path),
                    lang.t("skipped: ", "已跳过：")
                )),
            }
        } else if file_type.is_dir() && path.join("info.json").is_file() {
            match read_folder_mod(&path, lang) {
                Ok((info, marker, extra_warnings)) => {
                    for warning in extra_warnings {
                        warnings.push(format!("{}: {warning}", file_label(&path)));
                    }
                    let enabled_state = enabled
                        .get(&info.name.to_ascii_lowercase())
                        .copied()
                        .unwrap_or(false);
                    let marker = marker.filter(|marker| marker_is_valid(marker, &info));
                    mods.push(InstalledMod {
                        info,
                        source: ModSource::Folder(path.clone()),
                        enabled: enabled_state,
                        marker,
                        duplicate_count: 1,
                    });
                }
                Err(error) => warnings.push(format!(
                    "{}: {}{error}",
                    file_label(&path),
                    lang.t("skipped: ", "已跳过：")
                )),
            }
        }
    }

    let mut grouped: BTreeMap<String, Vec<InstalledMod>> = BTreeMap::new();
    for installed in mods {
        grouped
            .entry(installed.name().to_ascii_lowercase())
            .or_default()
            .push(installed);
    }
    let mut chosen_mods = Vec::new();
    for (_, mut group) in grouped {
        group.sort_by(|left, right| {
            right
                .info
                .version
                .cmp(&left.info.version)
                .then_with(|| left.source.is_zip().cmp(&right.source.is_zip()).reverse())
                .then_with(|| {
                    left.source
                        .path()
                        .map(|path| path.to_string_lossy().to_ascii_lowercase())
                        .cmp(
                            &right
                                .source
                                .path()
                                .map(|path| path.to_string_lossy().to_ascii_lowercase()),
                        )
                })
        });
        let mut chosen = group.remove(0);
        if !group.is_empty() {
            let mut names = vec![source_label(&chosen.source)];
            names.extend(group.iter().map(|item| source_label(&item.source)));
            warnings.push(format!(
                "{}: {} {}",
                chosen.name(),
                lang.t(
                    "duplicate versions found; using the newest:",
                    "发现重复版本，使用最新版本："
                ),
                names.join(", ")
            ));
            chosen.duplicate_count = names.len();
        }
        chosen_mods.push(chosen);
    }

    for (builtin, _, _) in BUILTIN_MODS {
        if chosen_mods
            .iter()
            .any(|installed| installed.name().eq_ignore_ascii_case(builtin))
        {
            continue;
        }
        if let Some(info) = builtin_info(builtin) {
            chosen_mods.push(InstalledMod {
                enabled: enabled.get(*builtin).copied().unwrap_or(false),
                info,
                source: ModSource::Builtin,
                marker: None,
                duplicate_count: 1,
            });
        }
    }

    let installed_names: std::collections::BTreeSet<String> = chosen_mods
        .iter()
        .map(|installed| installed.name().to_ascii_lowercase())
        .collect();
    for name in enabled.keys() {
        if !installed_names.contains(name) {
            warnings.push(format!(
                "{}: {}",
                name,
                lang.t(
                    "listed in mod-list.json but no matching mod is installed",
                    "在 mod-list.json 中存在，但没有安装对应模组"
                )
            ));
        }
    }

    chosen_mods.sort_by(|left, right| {
        let left_title = left.title().to_ascii_lowercase();
        let right_title = right.title().to_ascii_lowercase();
        left_title.cmp(&right_title).then_with(|| {
            left.name()
                .to_ascii_lowercase()
                .cmp(&right.name().to_ascii_lowercase())
        })
    });

    ScanResult {
        mods: chosen_mods,
        warnings,
    }
}

fn read_zip_mod(path: &Path, lang: Lang) -> Result<(ModInfo, Option<Marker>, Vec<String>)> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("cannot open zip {}", path.display()))?;
    let mut info_candidates = Vec::new();
    let mut marker_candidates = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .with_context(|| format!("cannot read entry #{index}"))?;
        let entry_name = entry
            .name()
            .with_context(|| format!("cannot read entry #{index} name"))?
            .replace('\\', "/");
        if entry_name.ends_with('/') {
            continue;
        }
        let depth = entry_name.matches('/').count();
        if depth > 1 {
            continue;
        }
        let Some(base_name) = entry_name.rsplit('/').next() else {
            continue;
        };
        if base_name != "info.json" && base_name != Marker::FILE_NAME {
            continue;
        }
        if entry.size() > MAX_INFO_BYTES {
            continue;
        }
        let root = entry_name
            .strip_suffix(base_name)
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string();
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .take(MAX_INFO_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("cannot read {entry_name}"))?;
        if bytes.len() as u64 > MAX_INFO_BYTES {
            continue;
        }
        let candidate = ZipCandidate { depth, root, bytes };
        if base_name == "info.json" {
            info_candidates.push(candidate);
        } else {
            marker_candidates.push(candidate);
        }
    }

    info_candidates.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.root.cmp(&right.root))
    });
    let info_candidate = info_candidates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("zip has no top-level info.json"))?;
    let info = ModInfo::parse(&info_candidate.bytes)
        .with_context(|| format!("invalid info.json in {}", path.display()))?;

    let mut warnings = Vec::new();
    let marker = marker_candidates
        .into_iter()
        .find(|candidate| candidate.root == info_candidate.root)
        .and_then(
            |candidate| match serde_json::from_slice::<Marker>(&candidate.bytes) {
                Ok(marker) => Some(marker),
                Err(error) => {
                    warnings.push(format!(
                        "{} {error}",
                        lang.t("invalid modpack marker:", "模组包标识文件无效：")
                    ));
                    None
                }
            },
        );
    if let Some(marker) = &marker {
        if !marker.is_recognized() {
            warnings.push(format!(
                "{} {}",
                lang.t("unknown modpack marker format in", "未知的模组包标识格式："),
                path.display()
            ));
        } else if !marker.internal_name.eq_ignore_ascii_case(&info.name) {
            warnings.push(format!(
                "{} {}",
                lang.t(
                    "modpack marker internal name does not match info.json:",
                    "模组包标识文件内部名与 info.json 不一致："
                ),
                info.name
            ));
        } else if marker.version != info.version {
            warnings.push(format!(
                "{} {}",
                lang.t(
                    "modpack marker version differs from info.json:",
                    "模组包标识文件版本与 info.json 不一致："
                ),
                info.version
            ));
        }
    }
    Ok((info, marker, warnings))
}

fn read_folder_mod(path: &Path, lang: Lang) -> Result<(ModInfo, Option<Marker>, Vec<String>)> {
    let info_path = path.join("info.json");
    let metadata = fs::metadata(&info_path)?;
    if metadata.len() > MAX_INFO_BYTES {
        bail!("info.json is too large");
    }
    let bytes =
        fs::read(&info_path).with_context(|| format!("cannot read {}", info_path.display()))?;
    let info = ModInfo::parse(&bytes)?;
    let mut warnings = Vec::new();
    let marker_path = path.join(Marker::FILE_NAME);
    let marker = if marker_path.is_file() {
        match fs::read(&marker_path) {
            Ok(bytes) => match serde_json::from_slice::<Marker>(&bytes) {
                Ok(marker) => Some(marker),
                Err(error) => {
                    warnings.push(format!(
                        "{} {error}",
                        lang.t("invalid modpack marker:", "模组包标识文件无效：")
                    ));
                    None
                }
            },
            Err(error) => {
                warnings.push(format!(
                    "{} {error}",
                    lang.t("cannot read modpack marker:", "无法读取模组包标识文件：")
                ));
                None
            }
        }
    } else {
        None
    };
    Ok((info, marker, warnings))
}

fn marker_is_valid(marker: &Marker, info: &ModInfo) -> bool {
    marker.is_recognized() && marker.internal_name.eq_ignore_ascii_case(&info.name)
}

fn expected_zip_name(info: &ModInfo) -> String {
    format!("{}_{}.zip", info.name, info.version)
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn source_label(source: &ModSource) -> String {
    match source {
        ModSource::Zip(path) | ModSource::Folder(path) => file_label(path),
        ModSource::Builtin => "builtin".to_string(),
    }
}

pub fn marker_for_edit(installed: &InstalledMod) -> Option<&Marker> {
    installed.marker.as_ref().filter(|marker| {
        marker.is_recognized() && marker.internal_name.eq_ignore_ascii_case(installed.name())
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::types::Version;

    fn make_zip(path: &Path, info: serde_json::Value, marker: Option<&Marker>) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        let root = format!(
            "{}_{}",
            info["name"].as_str().unwrap(),
            info["version"].as_str().unwrap()
        );
        zip.start_file(format!("{root}/info.json"), options)
            .unwrap();
        zip.write_all(info.to_string().as_bytes()).unwrap();
        if let Some(marker) = marker {
            zip.start_file(format!("{root}/{}", Marker::FILE_NAME), options)
                .unwrap();
            zip.write_all(serde_json::to_string(marker).unwrap().as_bytes())
                .unwrap();
        }
        zip.finish().unwrap();
    }

    fn info(name: &str, version: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "version": version,
            "title": name,
            "author": "test",
            "description": "",
            "factorio_version": "2.1"
        })
    }

    #[test]
    fn scans_zip_with_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = Marker::from_parts(
            "test-pack",
            "Test Pack",
            "me",
            Version::new(1, 0, 0),
            "now",
            "now",
            &[crate::types::MemberRef {
                name: "alpha".to_string(),
                version: Version::new(1, 2, 3),
            }],
        );
        make_zip(
            &dir.path().join("test-pack_1.0.0.zip"),
            info("test-pack", "1.0.0"),
            Some(&marker),
        );
        let enabled = BTreeMap::new();
        let result = scan_mods(dir.path(), &enabled, Lang::En);
        let found = result.find("test-pack").unwrap();
        assert!(found.is_pack());
        assert_eq!(found.marker().unwrap().title, "Test Pack");
    }

    #[test]
    fn picks_highest_duplicate_version() {
        let dir = tempfile::tempdir().unwrap();
        make_zip(
            &dir.path().join("alpha_1.0.0.zip"),
            info("alpha", "1.0.0"),
            None,
        );
        make_zip(
            &dir.path().join("alpha_2.0.0.zip"),
            info("alpha", "2.0.0"),
            None,
        );
        let result = scan_mods(dir.path(), &BTreeMap::new(), Lang::En);
        let found = result.find("alpha").unwrap();
        assert_eq!(found.info.version, Version::new(2, 0, 0));
        assert_eq!(found.duplicate_count, 2);
    }
}
