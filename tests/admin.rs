mod common;

use common::{json, run, run_db};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

fn setup() -> (TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("comms.db");
    (tmp, db)
}

fn ok(tmp: &TempDir, db: &Path, args: &[&str]) -> Output {
    ok_at(tmp.path(), db, args)
}

fn ok_at(cwd: &Path, db: &Path, args: &[&str]) -> Output {
    let output = run_db(cwd, db, args);
    assert!(output.status.success(), "args={args:?} stderr={}", String::from_utf8_lossy(&output.stderr));
    output
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

fn stderr_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).unwrap()
}

#[test]
fn gate_enforces_every_finding_exit_and_token_scoping() {
    let (tmp, db) = setup();
    ok(&tmp, &db, &["item-add", "--titulo", "#42: feature", "--dono", "dev"]);
    assert!(ok(&tmp, &db, &["gate"]).status.success());

    ok(&tmp, &db, &["item-add", "--titulo", "[pequeno] typo", "--dono", "dev"]);
    assert_eq!(run_db(tmp.path(), &db, &["gate"]).status.code(), Some(1));
    ok(&tmp, &db, &["item-set", "2", "--status", "debito"]);
    assert!(ok(&tmp, &db, &["gate"]).status.success());

    ok(&tmp, &db, &["item-add", "--titulo", "[medio] n+1", "--dono", "dev", "--status", "debito"]);
    assert_eq!(run_db(tmp.path(), &db, &["gate"]).status.code(), Some(1));
    ok(&tmp, &db, &["item-set", "3", "--bloqueio", "fora do hot path"]);
    assert!(ok(&tmp, &db, &["gate"]).status.success());

    ok(&tmp, &db, &["item-add", "--titulo", "[grave] sqli", "--dono", "dev", "--status", "debito"]);
    assert_eq!(run_db(tmp.path(), &db, &["gate"]).status.code(), Some(1));
    ok(&tmp, &db, &["item-set", "4", "--status", "feito"]);
    assert!(ok(&tmp, &db, &["gate"]).status.success());

    ok(&tmp, &db, &["item-add", "--titulo", "[grave] outra", "--dono", "dev", "--refs", "issue:99"]);
    assert_eq!(run_db(tmp.path(), &db, &["gate"]).status.code(), Some(1));
    assert!(ok(&tmp, &db, &["gate", "--refs", "issue:42"]).status.success());

    ok(&tmp, &db, &["item-add", "--titulo", "[grave] quatro", "--dono", "dev", "--refs", "issue:4"]);
    ok(&tmp, &db, &["item-add", "--titulo", "[grave] quarenta", "--dono", "dev", "--refs", "issue:42"]);
    let four = run_db(tmp.path(), &db, &["gate", "--refs", "issue:4", "--json"]);
    assert_eq!(four.status.code(), Some(1));
    let four_doc = stdout_json(&four);
    assert_eq!(four_doc["checked"], 1);
    assert_eq!(four_doc["failures"][0]["title"], "[grave] quatro");
    let forty_two = run_db(tmp.path(), &db, &["gate", "--refs", "ISSUE:42", "--json"]);
    assert_eq!(forty_two.status.code(), Some(1));
    assert_eq!(stdout_json(&forty_two)["checked"], 1);
}

#[test]
fn legacy_roster_check_becomes_conflict_and_migrate_preserves_rows() {
    let (tmp, db) = setup();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(r#"
        CREATE TABLE turns (
          id INTEGER PRIMARY KEY,
          sender TEXT NOT NULL CHECK (sender IN ('arch','dev')),
          recipient TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
          body TEXT NOT NULL, refs TEXT,
          created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
          resolved_at TEXT);
        CREATE TABLE work_items (
          id INTEGER PRIMARY KEY, title TEXT NOT NULL,
          owner TEXT NOT NULL CHECK (owner IN ('arch','dev')),
          scope TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'aberto',
          blocker TEXT, refs TEXT,
          updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')));
        CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL,
          updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')));
        CREATE INDEX idx_turns_sender ON turns(sender);
        INSERT INTO turns (sender,recipient,kind,title,body)
          VALUES ('arch','dev','mensagem','historico','preservar');
        INSERT INTO work_items (title,owner,scope) VALUES ('item antigo','dev','dev');
    "#).unwrap();
    drop(conn);
    let prompts = tmp.path().join("docs/prompts");
    fs::create_dir_all(&prompts).unwrap();
    fs::write(prompts.join("dba.md"), "# DBA\n").unwrap();

    let agents = json(&ok(&tmp, &db, &["agents", "--json"]));
    assert_eq!(agents["frozen_tables"], serde_json::json!(["turns", "work_items"]));
    let rejected = run_db(tmp.path(), &db, &["--json", "post", "--de", "dba", "--para", "todos", "--tipo", "pergunta", "--titulo", "t", "--corpo", "b"]);
    assert_eq!(rejected.status.code(), Some(1));
    assert_eq!(stderr_json(&rejected)["error"]["code"], "conflict");
    assert!(stderr_json(&rejected)["error"]["message"].as_str().unwrap().contains("comms migrate"));

    let migrated = json(&ok(&tmp, &db, &["migrate", "--json"]));
    assert_eq!(migrated["migrated"], serde_json::json!(["turns", "work_items"]));
    assert_eq!(migrated["rows"]["turns"], 1);
    assert_eq!(migrated["rows"]["work_items"], 1);
    let shown = json(&ok(&tmp, &db, &["show", "1", "--json"]));
    assert_eq!(shown["turn"]["body"], "preservar");
    ok(&tmp, &db, &["post", "--de", "dba", "--para", "todos", "--tipo", "pergunta", "--titulo", "t", "--corpo", "b"]);

    let again = json(&ok(&tmp, &db, &["--json", "migrate"]));
    assert_eq!(again["migrated"].as_array().unwrap().len(), 0);
    assert_eq!(again["rows"]["turns"], 0);
    assert_eq!(again["rows"]["work_items"], 0);
}

#[test]
fn bootstrap_writes_next_to_db_preserves_docs_and_deduplicates_gitignore() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let other = tmp.path().join("other");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&other).unwrap();
    let db = home.join("comms.db");

    let first = run_db(&other, &db, &["bootstrap", "--json"]);
    assert!(first.status.success(), "stderr={}", String::from_utf8_lossy(&first.stderr));
    assert!(db.exists());
    assert!(home.join("docs/core/AGENT_COMMS.md").exists());
    assert!(home.join("docs/core/PIPELINE.md").exists());
    assert!(!other.join("docs").exists());
    let agent_doc = fs::read_to_string(home.join("docs/core/AGENT_COMMS.md")).unwrap();
    assert!(agent_doc.contains("arch, pm, dev, tester, reviewer, human"));
    assert!(agent_doc.contains("todos"));
    assert!(!agent_doc.contains("./comms.py"));
    let pipeline = fs::read_to_string(home.join("docs/core/PIPELINE.md")).unwrap();
    assert!(pipeline.contains("claude"));
    assert!(pipeline.contains("gemini, codex, claude, minimax, qwen"));

    fs::write(home.join("docs/core/AGENT_COMMS.md"), "modificado").unwrap();
    ok_at(&other, &db, &["bootstrap"]);
    ok_at(&other, &db, &["bootstrap", "--force"]);
    assert_eq!(fs::read_to_string(home.join("docs/core/AGENT_COMMS.md")).unwrap(), "modificado");
    let gitignore = fs::read_to_string(home.join(".gitignore")).unwrap();
    for entry in ["comms.db", "comms.db-wal", "comms.db-shm"] {
        assert_eq!(gitignore.lines().filter(|line| *line == entry).count(), 1);
    }
}

#[test]
fn trunk_uses_real_default_branch_from_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    common::git(&repo, &["init", "-q", "-b", "tronco-esquisito"]);
    common::git(&repo, &["commit", "--allow-empty", "--no-gpg-sign", "-m", "init"]);
    let subdir = repo.join("a/b");
    fs::create_dir_all(&subdir).unwrap();

    for cwd in [&repo, &subdir] {
        let output = run(cwd, &["trunk"]);
        assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "tronco-esquisito\n");
    }
}

#[test]
fn trunk_preserves_nested_origin_head() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    common::init_repo(&repo);
    common::git(&repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/team/main"]);

    let output = run(&repo, &["trunk"]);

    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "team/main\n");
}

#[test]
fn testcmd_honors_override_and_ecosystem_markers() {
    let (tmp, db) = setup();
    ok(&tmp, &db, &["set", "test.cmd", "make custom"]);
    assert_eq!(String::from_utf8_lossy(&ok(&tmp, &db, &["testcmd"]).stdout), "make custom\n");

    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("Makefile"), "all:\n\ttrue\n").unwrap();
    fs::write(tmp.path().join("pyproject.toml"), "[project]\n").unwrap();
    assert_eq!(String::from_utf8_lossy(&run(tmp.path(), &["testcmd"]).stdout), "uv run pytest -q\n");

    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("package.json"), "{\"scripts\":{}}").unwrap();
    fs::write(tmp.path().join("Cargo.toml"), "[package]\n").unwrap();
    assert_eq!(String::from_utf8_lossy(&run(tmp.path(), &["testcmd"]).stdout), "cargo test\n");

    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("package.json"), "not json").unwrap();
    let missing = run(tmp.path(), &["testcmd"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("comms set test.cmd"));
}
