use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::Local;

pub const BACKUP_DIR: &str = "backups";

pub fn backup_file(backup_dir: &Path, source: &Path) -> Result<PathBuf> {
    fs::create_dir_all(backup_dir)
        .with_context(|| format!("cannot create backup directory {}", backup_dir.display()))?;
    let file_name = source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("cannot back up {}", source.display()))?
        .to_string_lossy();
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut attempt = 0u32;
    let target = loop {
        let suffix = if attempt == 0 {
            format!("{file_name}.{stamp}.bak")
        } else {
            format!("{file_name}.{stamp}-{attempt}.bak")
        };
        let candidate = backup_dir.join(suffix);
        if !candidate.exists() {
            break candidate;
        }
        attempt += 1;
        if attempt > 1000 {
            anyhow::bail!("cannot allocate a unique backup file name");
        }
    };
    fs::copy(source, &target).with_context(|| {
        format!(
            "cannot back up {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(target)
}

pub fn cleanup(backup_dir: &Path, days: u32) -> Result<usize> {
    if days == 0 {
        return Ok(0);
    }
    let entries = match fs::read_dir(backup_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read backup directory {}", backup_dir.display()));
        }
    };
    let retention = Duration::from_secs(u64::from(days) * 24 * 60 * 60);
    let now = SystemTime::now();
    let mut removed = 0usize;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("bak") {
            continue;
        }
        let modified = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        let age = now.duration_since(modified).unwrap_or_default();
        if age > retention && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
