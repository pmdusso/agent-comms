mod common;

use common::{init_repo, json, run};
use std::fs;
use tempfile::tempdir;

#[test]
fn version_discovers_main_db_from_root_subdir_and_worktrees() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("repo");
    common::init_repo(&root);
    fs::write(root.join("comms.db"), "").unwrap();
    let subdir = root.join("a/b");
    fs::create_dir_all(&subdir).unwrap();

    let nested = root.join(".worktrees/w");
    let external = tmp.path().join("wt");
    common::git(
        &root,
        &["worktree", "add", "-q", "-b", "nested", nested.to_str().unwrap()],
    );
    common::git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "external",
            external.to_str().unwrap(),
        ],
    );

    let expected = root.join("comms.db").canonicalize().unwrap();
    for cwd in [&root, &subdir, &nested, &external] {
        let doc = json(&run(cwd, &["--json", "version"]));
        assert_eq!(doc["db_path"], expected.to_str().unwrap(), "cwd={cwd:?}");
    }
}

#[test]
fn nested_git_repo_finds_db_in_non_git_parent() {
    let tmp = tempdir().unwrap();
    let parent = tmp.path().join("parent");
    let nested = parent.join("nested");
    init_repo(&nested);
    fs::write(parent.join("comms.db"), "").unwrap();

    let doc = json(&run(&nested, &["--json", "version"]));

    assert_eq!(
        doc["db_path"],
        parent.join("comms.db").canonicalize().unwrap().to_str().unwrap()
    );
}

#[test]
fn missing_db_is_null_and_roster_uses_embedded_default() {
    let tmp = tempdir().unwrap();

    let doc = json(&run(tmp.path(), &["--json", "version"]));
    assert!(doc["db_path"].is_null());

    let output = run(tmp.path(), &["version"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("SENDERS: arch, pm, dev, tester, reviewer, human"));
}

#[test]
fn clap_version_reports_package_version() {
    let tmp = tempdir().unwrap();
    let output = run(tmp.path(), &["--version"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "comms 5.0.0\n");
}
