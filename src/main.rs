use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use modpack_builder::ui::{self, UiOptions};

#[derive(Debug, Parser)]
#[command(
    name = "ModpackBuilder",
    version,
    about = "Create, edit and delete Factorio 2.1 modpacks"
)]
struct Cli {
    #[arg(
        long,
        value_name = "DIR",
        help = "Factorio mods directory (default: %APPDATA%\\Factorio\\mods)"
    )]
    mods_dir: Option<PathBuf>,

    #[arg(long, value_name = "LANG", help = "UI language: en or zh-CN")]
    lang: Option<String>,

    #[arg(
        long,
        value_name = "DAYS",
        help = "Delete backups older than DAYS days (0 keeps them forever)"
    )]
    backup_days: Option<u32>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error / 错误: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let executable =
        std::env::current_exe().context("cannot locate the ModpackBuilder executable")?;
    let program_dir = executable
        .parent()
        .context("cannot locate the ModpackBuilder program directory")?
        .to_path_buf();
    ui::run(UiOptions {
        mods_dir: cli.mods_dir,
        lang: cli.lang,
        backup_days: cli.backup_days,
        program_dir,
    })
}
