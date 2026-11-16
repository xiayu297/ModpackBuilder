use std::io::{Seek, Write};

use anyhow::{Context, Result, bail};
use serde_json::json;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::types::{Marker, PackSpec, Version, is_builtin_name};

pub const CHANGELOG_DASHES: &str = "---------------------------------------------------------------------------------------------------";
pub const TARGET_FACTORIO_VERSION: &str = "2.1";
pub const BASE_DEPENDENCY: &str = "base >= 2.1.0";

pub fn validate_pack_name(name: &str) -> Result<()> {
    let length = name.len();
    if !(3..=50).contains(&length) {
        bail!("internal name must contain 3 to 50 characters");
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("internal name may only contain letters, digits, '-' and '_'");
    }
    if !name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
    {
        bail!("internal name must start with a letter or digit");
    }
    if is_builtin_name(name) || name.eq_ignore_ascii_case("core") {
        bail!("internal name is reserved by Factorio");
    }
    Ok(())
}

pub fn validate_spec(spec: &PackSpec) -> Result<()> {
    validate_pack_name(&spec.internal_name)?;
    let title = spec.title.trim();
    if title.is_empty() {
        bail!("title must not be empty");
    }
    if title.chars().count() > 100 {
        bail!("title must not exceed 100 characters");
    }
    if title.chars().any(char::is_control) {
        bail!("title must not contain control characters");
    }
    if spec.author.trim().is_empty() {
        bail!("author must not be empty");
    }
    if spec.author.chars().any(char::is_control) {
        bail!("author must not contain control characters");
    }
    if spec.members.is_empty() {
        bail!("a modpack must contain at least one member mod");
    }
    let mut seen = std::collections::BTreeSet::new();
    for member in &spec.members {
        let key = member.name.to_ascii_lowercase();
        if !seen.insert(key) {
            bail!("modpack members must be unique");
        }
        if is_builtin_name(&member.name) {
            bail!("built-in mods cannot be modpack members");
        }
        if member.name.eq_ignore_ascii_case(&spec.internal_name) {
            bail!("a modpack cannot contain itself");
        }
    }
    Ok(())
}

pub fn pack_file_name(spec: &PackSpec) -> String {
    format!("{}_{}.zip", spec.internal_name, spec.version)
}

pub fn build_pack_to_writer<W: Write + Seek>(writer: W, spec: &PackSpec) -> Result<W> {
    validate_spec(spec)?;
    let members = spec.sorted_members();
    let folder = format!("{}_{}", spec.internal_name, spec.version);
    let info = json!({
        "name": spec.internal_name,
        "version": spec.version.to_string(),
        "title": spec.title.trim(),
        "author": spec.author.trim(),
        "description": info_description(members.len()),
        "factorio_version": TARGET_FACTORIO_VERSION,
        "dependencies": pack_dependencies(&members),
    });
    let info_text = format!("{}\n", serde_json::to_string_pretty(&info)?);
    let changelog = changelog(&spec.version, &members);
    let marker = Marker::from_parts(
        &spec.internal_name,
        spec.title.trim(),
        spec.author.trim(),
        spec.version,
        &spec.created_at,
        &spec.updated_at,
        &members,
    );
    let marker_text = format!("{}\n", serde_json::to_string_pretty(&marker)?);
    let locale_en = locale_file(
        &spec.internal_name,
        spec.title.trim(),
        &locale_description_en(members.len()),
    );
    let locale_zh = locale_file(
        &spec.internal_name,
        spec.title.trim(),
        &locale_description_zh(members.len()),
    );

    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, contents) in [
        ("info.json", info_text),
        ("changelog.txt", changelog),
        (Marker::FILE_NAME, marker_text),
        ("locale/en/modpack.cfg", locale_en),
        ("locale/zh-CN/modpack.cfg", locale_zh),
    ] {
        zip.start_file(format!("{folder}/{name}"), options)
            .with_context(|| format!("cannot add {name} to the modpack zip"))?;
        zip.write_all(contents.as_bytes())?;
    }
    Ok(zip.finish()?)
}

fn pack_dependencies(members: &[crate::types::MemberRef]) -> Vec<String> {
    let mut dependencies = Vec::with_capacity(members.len() + 1);
    dependencies.push(BASE_DEPENDENCY.to_string());
    dependencies.extend(
        members
            .iter()
            .map(|member| format!("{} >= {}", member.name, member.version)),
    );
    dependencies
}

fn info_description(member_count: usize) -> String {
    format!(
        "A modpack with {member_count} member mods, created with ModpackBuilder.\n由 ModpackBuilder 创建，包含 {member_count} 个成员模组。"
    )
}

fn locale_description_en(member_count: usize) -> String {
    format!("A modpack with {member_count} member mods, created with ModpackBuilder.")
}

fn locale_description_zh(member_count: usize) -> String {
    format!("由 ModpackBuilder 创建，包含 {member_count} 个成员模组。")
}

fn locale_file(internal_name: &str, title: &str, description: &str) -> String {
    format!(
        "[mod-name]\n{internal_name}={title}\n\n[mod-description]\n{internal_name}={description}\n"
    )
}

fn changelog(version: &Version, members: &[crate::types::MemberRef]) -> String {
    let date = chrono::Local::now().format("%d. %m. %Y");
    let mut entries = String::from("    - Modpack generated by ModpackBuilder.\n");
    for member in members {
        entries.push_str(&format!(
            "    - Includes {} >= {}.\n",
            member.name, member.version
        ));
    }
    format!("{CHANGELOG_DASHES}\nVersion: {version}\nDate: {date}\n\n  Features:\n{entries}\n")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Cursor, Read};

    use super::*;
    use crate::types::{MemberRef, ModInfo};
    use zip::ZipArchive;

    fn spec() -> PackSpec {
        PackSpec {
            internal_name: "test-pack".to_string(),
            title: "Test Pack".to_string(),
            version: Version::new(1, 2, 3),
            author: "Tester".to_string(),
            members: vec![
                MemberRef {
                    name: "beta-mod".to_string(),
                    version: Version::new(2, 0, 0),
                },
                MemberRef {
                    name: "alpha-mod".to_string(),
                    version: Version::new(1, 0, 0),
                },
            ],
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }

    fn entries(spec: &PackSpec) -> BTreeMap<String, Vec<u8>> {
        let cursor = build_pack_to_writer(Cursor::new(Vec::new()), spec).unwrap();
        let bytes = cursor.into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut result = BTreeMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            result.insert(entry.name().unwrap().to_string(), bytes);
        }
        result
    }

    #[test]
    fn writes_expected_files_and_no_lua() {
        let spec = spec();
        let files = entries(&spec);
        let prefix = "test-pack_1.2.3";
        assert!(files.contains_key(&format!("{prefix}/info.json")));
        assert!(files.contains_key(&format!("{prefix}/changelog.txt")));
        assert!(files.contains_key(&format!("{prefix}/module-set-builder.json")));
        assert!(files.contains_key(&format!("{prefix}/locale/en/modpack.cfg")));
        assert!(files.contains_key(&format!("{prefix}/locale/zh-CN/modpack.cfg")));
        assert!(!files.keys().any(|name| name.ends_with(".lua")));
    }

    #[test]
    fn info_has_target_version_and_sorted_dependencies() {
        let files = entries(&spec());
        let info = ModInfo::parse(&files["test-pack_1.2.3/info.json"]).unwrap();
        assert_eq!(info.factorio_version, "2.1");
        assert_eq!(info.name, "test-pack");
        assert_eq!(info.version, Version::new(1, 2, 3));
        let dependency_names: Vec<&str> = info
            .dependencies
            .iter()
            .map(|dependency| dependency.name.as_str())
            .collect();
        assert_eq!(dependency_names, vec!["base", "alpha-mod", "beta-mod"]);
        assert_eq!(
            info.dependencies[1].req.as_ref().unwrap().version,
            Version::new(1, 0, 0)
        );
    }

    #[test]
    fn marker_round_trips_and_changelog_is_well_formed() {
        let files = entries(&spec());
        let marker: Marker =
            serde_json::from_slice(&files["test-pack_1.2.3/module-set-builder.json"]).unwrap();
        assert!(marker.is_recognized());
        assert_eq!(marker.internal_name, "test-pack");
        assert_eq!(marker.members.len(), 2);
        assert_eq!(marker.members[0].name, "alpha-mod");
        let changelog = String::from_utf8(files["test-pack_1.2.3/changelog.txt"].clone()).unwrap();
        let first = changelog.lines().next().unwrap();
        assert_eq!(first.len(), 99);
        assert!(first.bytes().all(|byte| byte == b'-'));
        assert!(changelog.contains("  Features:\n    - "));
        assert!(changelog.contains("Includes alpha-mod >= 1.0.0."));
        assert!(changelog.contains("Includes beta-mod >= 2.0.0."));
        assert!(!changelog.contains('\t'));
        assert!(!changelog.lines().any(|line| line.ends_with(' ')));
    }

    #[test]
    fn rejects_invalid_specs() {
        let mut empty = spec();
        empty.members.clear();
        assert!(validate_spec(&empty).is_err());

        let mut builtin = spec();
        builtin.members.push(MemberRef {
            name: "base".to_string(),
            version: Version::new(2, 1, 0),
        });
        assert!(validate_spec(&builtin).is_err());

        let mut long_title = spec();
        long_title.title = "x".repeat(101);
        assert!(validate_spec(&long_title).is_err());

        let mut bad_name = spec();
        bad_name.internal_name = "bad name".to_string();
        assert!(validate_spec(&bad_name).is_err());

        let mut bad_author = spec();
        bad_author.author = "Bad\nAuthor".to_string();
        assert!(validate_spec(&bad_author).is_err());
    }

    #[test]
    fn pack_filename_contains_version() {
        assert_eq!(pack_file_name(&spec()), "test-pack_1.2.3.zip");
    }
}
