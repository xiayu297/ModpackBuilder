use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::backup;
use crate::modlist::{ModList, write_atomic};
use crate::pack;
use crate::types::{InstalledMod, Marker, ModSource, PackSpec};

#[derive(Debug, Clone)]
pub struct OpReport {
    pub zip_path: PathBuf,
    pub backups: Vec<PathBuf>,
}

pub fn create_pack(
    mods_dir: &Path,
    modlist: &ModList,
    backup_dir: &Path,
    spec: &PackSpec,
) -> Result<OpReport> {
    pack::validate_spec(spec)?;
    if modlist.has_entry(&spec.internal_name) {
        bail!(
            "{} is already registered in mod-list.json",
            spec.internal_name
        );
    }
    let file_name = pack::pack_file_name(spec);
    let target = mods_dir.join(&file_name);
    if target.exists() {
        bail!("target file already exists: {}", target.display());
    }

    let mut temp = tempfile::NamedTempFile::new_in(mods_dir)
        .with_context(|| format!("cannot create a temporary file in {}", mods_dir.display()))?;
    pack::build_pack_to_writer(temp.as_file_mut(), spec)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(&target)
        .map_err(|error| anyhow::anyhow!("cannot write {}: {}", target.display(), error.error))?;

    let mut backups = Vec::new();
    let result = (|| -> Result<()> {
        let new_bytes = modlist
            .append_disabled(&spec.internal_name)?
            .ok_or_else(|| anyhow::anyhow!("mod-list entry already exists"))?;
        let modlist_backup = backup::backup_file(backup_dir, modlist.path())?;
        backups.push(modlist_backup);
        write_atomic(modlist.path(), &new_bytes)?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(rollback_error) = fs::remove_file(&target) {
            return Err(error.context(format!(
                "additionally, the partially created pack could not be removed ({}): {rollback_error}",
                target.display()
            )));
        }
        return Err(error);
    }

    Ok(OpReport {
        zip_path: target,
        backups,
    })
}

pub fn update_pack(
    mods_dir: &Path,
    modlist: &ModList,
    backup_dir: &Path,
    old: &InstalledMod,
    spec: &PackSpec,
) -> Result<OpReport> {
    pack::validate_spec(spec)?;
    if !spec.internal_name.eq_ignore_ascii_case(old.name()) {
        bail!("the internal modpack name cannot be changed");
    }
    let marker = old
        .marker()
        .ok_or_else(|| anyhow::anyhow!("the selected mod is not a recognized modpack"))?;
    if !marker.is_recognized() {
        bail!("the selected modpack has an unrecognized marker");
    }
    let old_path = match &old.source {
        ModSource::Zip(path) => path.clone(),
        _ => bail!("only zip modpacks can be updated"),
    };
    if !old_path.is_file() {
        bail!("modpack file no longer exists: {}", old_path.display());
    }

    let new_path = mods_dir.join(pack::pack_file_name(spec));
    let same_file = new_path
        .file_name()
        .zip(old_path.file_name())
        .is_some_and(|(left, right)| {
            left.to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy())
        });
    if !same_file && new_path.exists() {
        bail!("target file already exists: {}", new_path.display());
    }

    let mut temp = tempfile::NamedTempFile::new_in(mods_dir)
        .with_context(|| format!("cannot create a temporary file in {}", mods_dir.display()))?;
    pack::build_pack_to_writer(temp.as_file_mut(), spec)?;
    temp.as_file().sync_all()?;

    let mut backups = vec![backup::backup_file(backup_dir, &old_path)?];

    if !modlist.has_entry(&spec.internal_name) {
        let new_bytes = modlist
            .append_disabled(&spec.internal_name)?
            .ok_or_else(|| anyhow::anyhow!("mod-list entry unexpectedly exists"))?;
        backups.push(backup::backup_file(backup_dir, modlist.path())?);
        write_atomic(modlist.path(), &new_bytes)?;
    }

    if same_file {
        temp.persist(&new_path).map_err(|error| {
            anyhow::anyhow!("cannot replace {}: {}", new_path.display(), error.error)
        })?;
    } else {
        temp.persist_noclobber(&new_path).map_err(|error| {
            anyhow::anyhow!("cannot write {}: {}", new_path.display(), error.error)
        })?;
        if let Err(remove_error) = fs::remove_file(&old_path) {
            let rollback = fs::remove_file(&new_path);
            let mut message = format!(
                "the new modpack was written but the old file {} could not be removed: {remove_error}",
                old_path.display()
            );
            if let Err(rollback_error) = rollback {
                message.push_str(&format!(
                    "; additionally, the new file could not be removed: {rollback_error}"
                ));
            }
            bail!(message);
        }
    }

    Ok(OpReport {
        zip_path: new_path,
        backups,
    })
}

pub fn delete_pack(
    modlist: &ModList,
    backup_dir: &Path,
    target: &InstalledMod,
) -> Result<OpReport> {
    let marker: &Marker = target
        .marker()
        .ok_or_else(|| anyhow::anyhow!("the selected mod is not a recognized modpack"))?;
    if !marker.is_recognized() {
        bail!("the selected modpack has an unrecognized marker");
    }
    let path = match &target.source {
        ModSource::Zip(path) => path.clone(),
        _ => bail!("only zip modpacks can be deleted"),
    };
    if !path.is_file() {
        bail!("modpack file no longer exists: {}", path.display());
    }

    let zip_backup = backup::backup_file(backup_dir, &path)?;
    let mut backups = vec![zip_backup.clone()];
    let new_bytes = modlist.remove_entry(target.name())?;
    if new_bytes.is_some() {
        backups.push(backup::backup_file(backup_dir, modlist.path())?);
    }
    fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    if let Some(bytes) = new_bytes
        && let Err(error) = write_atomic(modlist.path(), &bytes)
    {
        return match fs::copy(&zip_backup, &path) {
            Ok(_) => Err(error.context(format!(
                "the modpack entry could not be removed and the zip was restored from {}",
                zip_backup.display()
            ))),
            Err(restore_error) => Err(error.context(format!(
                "the modpack entry could not be removed and restoring the zip failed ({restore_error}); backup: {}",
                zip_backup.display()
            ))),
        };
    }

    Ok(OpReport {
        zip_path: path,
        backups,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::types::{MarkerMember, MemberRef, Version};
    use zip::ZipArchive;

    const MOD_LIST: &str = "\n{\n  \"mods\": \n  [\n    \n    {\n      \"name\": \"base\",\n      \"enabled\": true\n    }\n  ]\n}";

    fn write_mod_list(dir: &Path) -> PathBuf {
        let path = dir.join("mod-list.json");
        fs::write(&path, MOD_LIST).unwrap();
        path
    }

    fn spec(name: &str, version: &str) -> PackSpec {
        PackSpec {
            internal_name: name.to_string(),
            title: format!("{name} title"),
            version: Version::parse(version).unwrap(),
            author: "Tester".to_string(),
            members: vec![MemberRef {
                name: "alpha".to_string(),
                version: Version::new(1, 0, 0),
            }],
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }

    fn fake_installed(
        zip_path: PathBuf,
        name: &str,
        version: &str,
        marker: Marker,
    ) -> InstalledMod {
        InstalledMod {
            info: crate::types::ModInfo {
                name: name.to_string(),
                version: Version::parse(version).unwrap(),
                title: name.to_string(),
                author: String::new(),
                description: String::new(),
                factorio_version: "2.1".to_string(),
                dependencies: Vec::new(),
                invalid_dependencies: Vec::new(),
            },
            source: ModSource::Zip(zip_path),
            enabled: false,
            marker: Some(marker),
            duplicate_count: 1,
        }
    }

    fn marker(name: &str, version: Version, members: Vec<MarkerMember>) -> Marker {
        Marker {
            format: 1,
            tool: Marker::TOOL.to_string(),
            internal_name: name.to_string(),
            title: name.to_string(),
            author: "Tester".to_string(),
            version,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            members,
        }
    }

    #[test]
    fn creates_zip_and_appends_disabled_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir(&mods).unwrap();
        write_mod_list(&mods);
        let backups = dir.path().join("backups");
        let list = ModList::load(&mods.join("mod-list.json")).unwrap();
        let report = create_pack(&mods, &list, &backups, &spec("new-pack", "1.0.0")).unwrap();
        assert!(report.zip_path.is_file());
        assert!(!report.backups.is_empty());
        let reloaded = ModList::load(&mods.join("mod-list.json")).unwrap();
        assert_eq!(reloaded.is_enabled("new-pack"), Some(false));
    }

    #[test]
    fn rejects_duplicate_create() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir(&mods).unwrap();
        write_mod_list(&mods);
        let list = ModList::load(&mods.join("mod-list.json")).unwrap();
        create_pack(
            &mods,
            &list,
            &dir.path().join("backups"),
            &spec("new-pack", "1.0.0"),
        )
        .unwrap();
        let list = ModList::load(&mods.join("mod-list.json")).unwrap();
        let error = create_pack(
            &mods,
            &list,
            &dir.path().join("backups"),
            &spec("new-pack", "1.0.0"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn does_not_overwrite_an_unrecognized_target_file() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir(&mods).unwrap();
        let modlist_path = write_mod_list(&mods);
        let original_modlist = fs::read(&modlist_path).unwrap();
        let target = mods.join("new-pack_1.0.0.zip");
        fs::write(&target, b"not a modpack").unwrap();
        let list = ModList::load(&modlist_path).unwrap();

        let error = create_pack(
            &mods,
            &list,
            &dir.path().join("backups"),
            &spec("new-pack", "1.0.0"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("target file already exists"));
        assert_eq!(fs::read(&target).unwrap(), b"not a modpack");
        assert_eq!(fs::read(&modlist_path).unwrap(), original_modlist);
    }

    #[test]
    fn updates_version_and_preserves_enabled_state() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir(&mods).unwrap();
        let modlist_path = write_mod_list(&mods);
        let list = ModList::load(&modlist_path).unwrap();
        let create = create_pack(
            &mods,
            &list,
            &dir.path().join("backups"),
            &spec("new-pack", "1.0.0"),
        )
        .unwrap();
        let mut updated_list_bytes = fs::read(&modlist_path).unwrap();
        let enabled_text = String::from_utf8(updated_list_bytes.clone())
            .unwrap()
            .replace(
                "\"name\": \"new-pack\",\n      \"enabled\": false",
                "\"name\": \"new-pack\",\n      \"enabled\": true",
            );
        updated_list_bytes = enabled_text.into_bytes();
        fs::write(&modlist_path, &updated_list_bytes).unwrap();
        let list = ModList::load(&modlist_path).unwrap();
        assert_eq!(list.is_enabled("new-pack"), Some(true));

        let old = fake_installed(
            create.zip_path.clone(),
            "new-pack",
            "1.0.0",
            marker(
                "new-pack",
                Version::new(1, 0, 0),
                vec![MarkerMember {
                    name: "alpha".to_string(),
                    version: Version::new(1, 0, 0),
                }],
            ),
        );
        let mut new_spec = spec("new-pack", "1.1.0");
        new_spec.members[0].version = Version::new(1, 1, 0);
        let report =
            update_pack(&mods, &list, &dir.path().join("backups"), &old, &new_spec).unwrap();
        assert!(!create.zip_path.exists());
        assert!(report.zip_path.ends_with("new-pack_1.1.0.zip"));
        let reloaded = ModList::load(&modlist_path).unwrap();
        assert_eq!(reloaded.is_enabled("new-pack"), Some(true));
        let file = fs::File::open(&report.zip_path).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let mut marker_text = String::new();
        archive
            .by_name("new-pack_1.1.0/module-set-builder.json")
            .unwrap()
            .read_to_string(&mut marker_text)
            .unwrap();
        let marker: Marker = serde_json::from_str(&marker_text).unwrap();
        assert_eq!(marker.version, Version::new(1, 1, 0));
    }

    #[test]
    fn deletes_zip_and_entry_but_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        fs::create_dir(&mods).unwrap();
        let modlist_path = write_mod_list(&mods);
        let list = ModList::load(&modlist_path).unwrap();
        let create = create_pack(
            &mods,
            &list,
            &dir.path().join("backups"),
            &spec("new-pack", "1.0.0"),
        )
        .unwrap();
        let list = ModList::load(&modlist_path).unwrap();
        let target = fake_installed(
            create.zip_path.clone(),
            "new-pack",
            "1.0.0",
            marker(
                "new-pack",
                Version::new(1, 0, 0),
                vec![MarkerMember {
                    name: "alpha".to_string(),
                    version: Version::new(1, 0, 0),
                }],
            ),
        );
        let report = delete_pack(&list, &dir.path().join("backups"), &target).unwrap();
        assert!(!report.zip_path.exists());
        assert!(report.backups.iter().any(|path| path.is_file()));
        let reloaded = ModList::load(&modlist_path).unwrap();
        assert!(!reloaded.has_entry("new-pack"));
    }
}
