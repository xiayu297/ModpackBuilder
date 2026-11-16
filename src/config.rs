use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub backup_days: Option<u32>,
}

impl Config {
    pub fn lang_code(&self) -> Option<&str> {
        self.lang.as_deref()
    }

    pub fn backup_days(&self, fallback: u32) -> u32 {
        self.backup_days.unwrap_or(fallback)
    }
}

pub fn config_path(program_dir: &Path) -> PathBuf {
    program_dir.join(CONFIG_FILE)
}

pub fn load(program_dir: &Path) -> (Config, Option<String>) {
    let path = config_path(program_dir);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Config::default(), None);
        }
        Err(error) => {
            return (
                Config::default(),
                Some(format!("cannot read {}: {error}", path.display())),
            );
        }
    };
    match serde_json::from_slice::<Config>(&bytes) {
        Ok(config) => (config, None),
        Err(error) => (
            Config::default(),
            Some(format!("cannot parse {}: {error}", path.display())),
        ),
    }
}

pub fn save(program_dir: &Path, config: &Config) -> Result<()> {
    let path = config_path(program_dir);
    let bytes = serde_json::to_vec_pretty(config)?;
    let mut temp = tempfile::NamedTempFile::new_in(program_dir).with_context(|| {
        format!(
            "cannot create a temporary file in {}",
            program_dir.display()
        )
    })?;
    temp.write_all(&bytes)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(&path)
        .map_err(|error| anyhow::anyhow!("cannot save {}: {}", path.display(), error.error))?;
    Ok(())
}
