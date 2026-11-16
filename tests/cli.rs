use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_the_portable_cli_options() {
    let mut command = Command::cargo_bin("ModpackBuilder").unwrap();
    command.arg("--help").assert().success().stdout(
        predicate::str::contains("Create, edit and delete Factorio 2.1 modpacks")
            .and(predicate::str::contains("--mods-dir"))
            .and(predicate::str::contains("--lang"))
            .and(predicate::str::contains("--backup-days")),
    );
}

#[test]
fn rejects_an_unsupported_language_before_starting_the_tui() {
    let mut command = Command::cargo_bin("ModpackBuilder").unwrap();
    command.args(["--lang", "fr"]).assert().failure().stderr(
        predicate::str::contains("unsupported --lang value")
            .and(predicate::str::contains("en"))
            .and(predicate::str::contains("zh-CN")),
    );
}

#[test]
fn reports_a_missing_mods_directory_without_starting_the_tui() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-mods");
    let mut command = Command::cargo_bin("ModpackBuilder").unwrap();
    command
        .args(["--lang", "en", "--mods-dir"])
        .arg(&missing)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Factorio mods directory not found",
        ));
}
