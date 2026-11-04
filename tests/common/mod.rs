#![allow(dead_code)]

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command as ProcessCommand, Output};

pub fn comms() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_comms"));
    cmd.env_remove("COMMS_DB")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

pub fn run(cwd: &Path, args: &[&str]) -> Output {
    let mut cmd = comms();
    cmd.current_dir(cwd).args(args);
    cmd.output().unwrap()
}

pub fn run_db(cwd: &Path, db: &Path, args: &[&str]) -> Output {
    let mut cmd = comms();
    cmd.current_dir(cwd).env("COMMS_DB", db).args(args);
    cmd.output().unwrap()
}

pub fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub fn git(cwd: &Path, args: &[&str]) {
    let output = ProcessCommand::new("git")
        .arg("-c")
        .arg("user.name=t")
        .arg("-c")
        .arg("user.email=t@t")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q", "-b", "dev"]);
    git(
        root,
        &["commit", "--allow-empty", "--no-gpg-sign", "-m", "init"],
    );
}
