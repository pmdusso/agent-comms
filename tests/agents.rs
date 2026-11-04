mod common;

use common::{json, run_db};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

fn setup(with_prompts: bool) -> (TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("comms.db");
    if with_prompts {
        let prompts = tmp.path().join("docs/prompts");
        fs::create_dir_all(&prompts).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts");
        for name in ["arch", "dev"] {
            fs::copy(source.join(format!("{name}.md")), prompts.join(format!("{name}.md"))).unwrap();
        }
    }
    (tmp, db)
}

fn ok(tmp: &TempDir, db: &Path, args: &[&str]) -> Output {
    let output = run_db(tmp.path(), db, args);
    assert!(output.status.success(), "args={args:?} stderr={}", String::from_utf8_lossy(&output.stderr));
    output
}

fn error_doc(output: &Output) -> Value {
    assert!(!output.status.success());
    serde_json::from_slice(&output.stderr).unwrap()
}

#[test]
fn adhoc_agent_lifecycle_updates_roster_brief_and_orphans() {
    let (tmp, db) = setup(true);
    ok(&tmp, &db, &["agent-add", "dba", "--descricao", "tuning", "--por", "arch"]);
    ok(&tmp, &db, &["post", "--de", "dba", "--para", "arch", "--tipo", "mensagem", "--titulo", "oi", "--corpo", "x"]);

    let agents = json(&ok(&tmp, &db, &["agents", "--json"]));
    let dba = agents["agents"].as_array().unwrap().iter().find(|agent| agent["name"] == "dba").unwrap();
    assert_eq!(dba["origin"], "adhoc");
    assert_eq!(dba["created_by"], "arch");
    assert_eq!(dba["turns"], 1);

    ok(&tmp, &db, &["agent-set", "dba", "--descricao", "tuning e índices"]);
    let brief = ok(&tmp, &db, &["brief", "dba"]);
    assert!(String::from_utf8_lossy(&brief.stdout).contains("Você é `dba`"));
    let arch_brief = ok(&tmp, &db, &["brief", "arch"]);
    let arch_text = String::from_utf8_lossy(&arch_brief.stdout);
    assert!(arch_text.contains("-- Roster --"));
    assert!(arch_text.contains("dba — tuning e índices"));

    ok(&tmp, &db, &["agent-rm", "dba"]);
    let rejected = run_db(tmp.path(), &db, &["post", "--de", "dba", "--para", "arch", "--tipo", "mensagem", "--titulo", "x", "--corpo", "y"]);
    assert_eq!(rejected.status.code(), Some(2));
    let agents = json(&ok(&tmp, &db, &["agents", "--json"]));
    assert!(agents["orphans"].as_array().unwrap().contains(&Value::String("dba".into())));
}

#[test]
fn agent_add_validates_names_conflicts_and_prompt_skeleton() {
    let (tmp, db) = setup(true);
    let conflict = run_db(tmp.path(), &db, &["--json", "agent-add", "dev", "--descricao", "x", "--por", "arch"]);
    assert_eq!(error_doc(&conflict)["error"]["code"], "conflict");
    for name in ["todos", "Dba!"] {
        let invalid = run_db(tmp.path(), &db, &["--json", "agent-add", name, "--descricao", "x", "--por", "arch"]);
        assert_eq!(invalid.status.code(), Some(1));
        assert_eq!(error_doc(&invalid)["error"]["code"], "invalid");
    }

    let bad = tmp.path().join("bad.md");
    fs::write(&bad, "# sem esqueleto\n").unwrap();
    let guard = run_db(tmp.path(), &db, &["--json", "agent-add", "dba", "--descricao", "x", "--por", "arch", "--prompt-file", bad.to_str().unwrap()]);
    assert_eq!(error_doc(&guard)["error"]["code"], "guard");
    ok(&tmp, &db, &["agent-add", "dba", "--descricao", "x", "--por", "arch", "--prompt-file", bad.to_str().unwrap(), "--force"]);

    let typo = run_db(tmp.path(), &db, &["post", "--de", "dvv", "--para", "arch", "--tipo", "mensagem", "--titulo", "x", "--corpo", "y"]);
    assert_eq!(typo.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&typo.stderr).contains("comms agent-add dvv"));
}

#[test]
fn adhoc_replaces_fallback_in_same_position() {
    let (tmp, db) = setup(false);
    ok(&tmp, &db, &["agent-add", "dev", "--descricao", "x", "--por", "arch"]);
    let doc = json(&ok(&tmp, &db, &["agents", "--json"]));
    let agents = doc["agents"].as_array().unwrap();
    assert_eq!(agents.iter().filter(|agent| agent["name"] == "dev").count(), 1);
    assert_eq!(agents[2]["name"], "dev");
    assert_eq!(agents[2]["origin"], "adhoc");
}

#[test]
fn default_file_wins_conflict_and_adhoc_prompt_remains_addressable() {
    let (tmp, db) = setup(true);
    ok(&tmp, &db, &["agent-add", "dba", "--descricao", "banco", "--por", "arch"]);
    let file_prompt = "# DBA do arquivo\n\nPrompt padrão soberano.\n";
    fs::write(tmp.path().join("docs/prompts/dba.md"), file_prompt).unwrap();

    let agents = ok(&tmp, &db, &["agents"]);
    assert!(String::from_utf8_lossy(&agents.stdout).contains("AVISO: dba"));
    let brief = ok(&tmp, &db, &["brief", "dba"]);
    assert!(String::from_utf8_lossy(&brief.stdout).starts_with("# DBA do arquivo"));
    let adhoc = ok(&tmp, &db, &["agent-show", "dba", "--adhoc"]);
    assert!(String::from_utf8_lossy(&adhoc.stdout).starts_with("# Bootstrap — DBA"));

    ok(&tmp, &db, &["agent-rm", "dba"]);
    let doc = json(&ok(&tmp, &db, &["agents", "--json"]));
    assert!(doc["conflicts"].as_array().unwrap().is_empty());
    let brief = ok(&tmp, &db, &["brief", "dba"]);
    assert!(String::from_utf8_lossy(&brief.stdout).starts_with("# DBA do arquivo"));
}

#[test]
fn agent_set_requires_existing_adhoc_and_a_change() {
    let (tmp, db) = setup(true);
    let missing = run_db(tmp.path(), &db, &["--json", "agent-set", "dba", "--descricao", "x"]);
    assert_eq!(error_doc(&missing)["error"]["code"], "not_found");
    ok(&tmp, &db, &["agent-add", "dba", "--descricao", "x", "--por", "arch"]);
    let unchanged = run_db(tmp.path(), &db, &["--json", "agent-set", "dba"]);
    assert_eq!(error_doc(&unchanged)["error"]["code"], "invalid");
}
