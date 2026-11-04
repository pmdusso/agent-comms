mod common;

use common::{run, run_db};
use serde_json::Value;
use std::fs;

#[test]
fn bootstrap_never_targets_db_discovered_in_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("parent");
    let nested = parent.join("nested");
    common::init_repo(&nested);
    let ancestral = parent.join("comms.db");
    fs::write(&ancestral, "sentinel ancestral").unwrap();

    let first = run(&nested, &["bootstrap"]);
    assert!(first.status.success(), "stderr={}", String::from_utf8_lossy(&first.stderr));
    assert!(nested.join("comms.db").is_file());
    assert_eq!(fs::read_to_string(&ancestral).unwrap(), "sentinel ancestral");

    let forced = run(&nested, &["bootstrap", "--force"]);
    assert!(forced.status.success(), "stderr={}", String::from_utf8_lossy(&forced.stderr));
    assert!(nested.join("comms.db").is_file());
    assert_eq!(fs::read_to_string(&ancestral).unwrap(), "sentinel ancestral");
}

#[test]
fn gate_empty_refs_means_unfiltered_like_python() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("comms.db");
    let added = run_db(tmp.path(), &db, &["item-add", "--titulo", "[grave] sem refs", "--dono", "dev"]);
    assert!(added.status.success());

    let gate = run_db(tmp.path(), &db, &["gate", "--refs", "", "--json"]);

    assert_eq!(gate.status.code(), Some(1), "stdout={} stderr={}", String::from_utf8_lossy(&gate.stdout), String::from_utf8_lossy(&gate.stderr));
    let doc: Value = serde_json::from_slice(&gate.stdout).unwrap();
    assert_eq!(doc["checked"], 1);
    assert!(doc["refs"].is_null());
}

#[test]
fn testcmd_reads_makefile_with_invalid_utf8_in_comment() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("Makefile"), b"# coment\xe1rio\ntest:\n\ttrue\n").unwrap();

    let output = run(tmp.path(), &["testcmd"]);

    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "make test\n");
}

#[test]
fn empty_optional_strings_are_invisible_in_text_output() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("comms.db");
    for args in [
        &["post", "--de", "arch", "--para", "dev", "--tipo", "mensagem", "--titulo", "vazio", "--corpo", "b", "--refs", ""][..],
        &["item-add", "--titulo", "item vazio", "--dono", "dev", "--bloqueio", "", "--refs", ""][..],
    ] {
        let output = run_db(tmp.path(), &db, args);
        assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    }

    for args in [&["show", "1"][..], &["items"][..], &["state"][..], &["brief", "dev"][..]] {
        let output = run_db(tmp.path(), &db, args);
        assert!(output.status.success(), "args={args:?} stderr={}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("refs: "), "args={args:?} output={text}");
        assert!(!text.contains("bloqueio: "), "args={args:?} output={text}");
        assert!(!text.contains("[]"), "args={args:?} output={text}");
    }
}
