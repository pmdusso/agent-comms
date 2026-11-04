mod common;

use common::{json, run, run_db};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("comms.db");
    (tmp, db)
}

fn ok(tmp: &TempDir, db: &Path, args: &[&str]) -> std::process::Output {
    let output = run_db(tmp.path(), db, args);
    assert!(output.status.success(), "args={args:?} stderr={}", String::from_utf8_lossy(&output.stderr));
    output
}

fn error_doc(output: &std::process::Output) -> Value {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout={}", String::from_utf8_lossy(&output.stdout));
    serde_json::from_slice(&output.stderr).unwrap()
}

#[test]
fn post_turns_show_resolve_and_broadcast_flow() {
    let (tmp, db) = setup();
    ok(&tmp, &db, &["post", "--de", "arch", "--para", "dev", "--tipo", "mensagem", "--titulo", "t1", "--corpo", "b1"]);
    ok(&tmp, &db, &["post", "--de", "pm", "--para", "todos", "--tipo", "mensagem", "--titulo", "t2", "--corpo", "b2"]);

    let turns = json(&ok(&tmp, &db, &["--json", "turns", "--para", "dev"]));
    assert_eq!(turns["turns"].as_array().unwrap().len(), 2);
    assert_eq!(turns["turns"][0]["id"], 2);
    assert_eq!(turns["turns"][1]["body"], "b1");

    let shown = json(&ok(&tmp, &db, &["show", "1", "--json"]));
    assert_eq!(shown["turn"]["title"], "t1");
    ok(&tmp, &db, &["resolve", "1"]);

    let repeated = ok(&tmp, &db, &["--json", "resolve", "1"]);
    let doc = json(&repeated);
    assert_eq!(doc["turn"]["id"], 1);
    assert!(doc["warnings"][0].as_str().unwrap().contains("já estava resolvido"));
    let open = json(&ok(&tmp, &db, &["turns", "--json"]));
    assert_eq!(open["turns"].as_array().unwrap().len(), 1);
}

#[test]
fn missing_turn_and_item_ids_are_structured_not_found_errors() {
    let (tmp, db) = setup();
    ok(&tmp, &db, &["set", "init", "1"]);
    for args in [&["--json", "show", "999"][..], &["--json", "resolve", "999"][..], &["--json", "item-set", "999", "--status", "feito"][..]] {
        let doc = error_doc(&run_db(tmp.path(), &db, args));
        assert_eq!(doc["error"]["code"], "not_found");
    }
}

#[test]
fn handoff_guard_accepts_accents_and_force() {
    let (tmp, db) = setup();
    let rejected = run_db(tmp.path(), &db, &["post", "--de", "dev", "--para", "arch", "--tipo", "handoff", "--titulo", "h", "--corpo", "vago"]);
    assert_eq!(rejected.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("objetivo, criterios, fronteiras"));

    ok(&tmp, &db, &["post", "--de", "dev", "--para", "arch", "--tipo", "handoff", "--titulo", "h", "--corpo", "Objetivo: x\nCritérios: y\nFronteiras: z"]);
    ok(&tmp, &db, &["post", "--de", "dev", "--para", "arch", "--tipo", "handoff", "--titulo", "h2", "--corpo", "vago", "--force"]);
}

#[test]
fn pii_guard_rejects_digits_and_force_overrides() {
    let (tmp, db) = setup();
    let rejected = run_db(tmp.path(), &db, &["post", "--de", "arch", "--para", "dev", "--tipo", "mensagem", "--titulo", "x", "--corpo", "5551999998888"]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("possível PII"));
    ok(&tmp, &db, &["post", "--de", "arch", "--para", "dev", "--tipo", "mensagem", "--titulo", "x", "--corpo", "5551999998888", "--force"]);
}

#[test]
fn free_text_preserves_leading_hyphens() {
    let (tmp, db) = setup();
    ok(&tmp, &db, &["post", "--de", "arch", "--para", "dev", "--tipo", "mensagem", "--titulo", "-5% latência", "--corpo", "- a"]);
    let doc = json(&ok(&tmp, &db, &["--json", "show", "1"]));
    assert_eq!(doc["turn"]["title"], "-5% latência");
    assert_eq!(doc["turn"]["body"], "- a");
}

#[test]
fn items_state_and_set_obey_filters_and_json_shape() {
    let (tmp, db) = setup();
    ok(&tmp, &db, &["item-add", "--titulo", "um", "--dono", "dev"]);
    ok(&tmp, &db, &["item-add", "--titulo", "dois", "--dono", "tester", "--refs", "issue:42"]);
    ok(&tmp, &db, &["item-set", "1", "--status", "feito"]);
    let pending = json(&ok(&tmp, &db, &["items", "--json"]));
    assert_eq!(pending["items"].as_array().unwrap().len(), 1);
    assert_eq!(pending["items"][0]["id"], 2);
    let all = json(&ok(&tmp, &db, &["items", "--all", "--json"]));
    assert_eq!(all["items"].as_array().unwrap().len(), 2);

    let nothing = run_db(tmp.path(), &db, &["--json", "item-set", "1"]);
    assert_eq!(error_doc(&nothing)["error"]["code"], "invalid");
    ok(&tmp, &db, &["set", "bola_com", "tester"]);
    let state = json(&ok(&tmp, &db, &["state", "--json"]));
    assert!(state["kv"].is_array());
    assert!(state["items"].is_array());
    assert!(state["turns"].is_array());
}

#[test]
fn commands_without_db_fail_without_creating_file() {
    let tmp = tempfile::tempdir().unwrap();
    let state = run(tmp.path(), &["--json", "state"]);
    assert_eq!(state.status.code(), Some(1));
    assert_eq!(error_doc(&state)["error"]["code"], "db_missing");

    let post = run(tmp.path(), &["post", "--de", "arch", "--para", "dev", "--tipo", "mensagem", "--titulo", "x", "--corpo", "y"]);
    assert_eq!(post.status.code(), Some(1));
    assert!(!tmp.path().join("comms.db").exists());
}
