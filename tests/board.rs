mod common;

use common::json;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output};
use tempfile::TempDir;

const GH_SCRIPT: &str = r#"#!/bin/sh
printf '%s %s\n' "$1" "$2" >> "$FAKE_GH_LOG"
printf '%s\n' "$*" >> "$FAKE_GH_LOG.args"
case "$1 $2" in
  "repo view") echo "${FAKE_GH_REPO:-acme/app}"; exit 0 ;;
  "project item-edit") exit 0 ;;
  "issue comment") exit 0 ;;
  "api graphql")
    case "$*" in
      *--paginate*) cat "$FAKE_GH_PAGES"; exit 0 ;;
      *projectItems*)
        if [ -n "${FAKE_GH_FAIL:-}" ]; then echo "gh: Could not resolve to an Issue with the number of 7." >&2; exit 1; fi
        cat "$FAKE_GH_CARD"; exit 0 ;;
    esac ;;
esac
echo "fake gh: args inesperados: $*" >&2; exit 1
"#;

const CARD: &str = r#"{
  "data": {
    "viewer": {"login": "octo"},
    "repository": {"issue": {
      "title": "Fix: Login É quebrado!!",
      "projectItems": {"nodes": [
        {"id":"PVTI_x","project":{"id":"PVT_2","number":2,"owner":{"login":"octo"},"field":null},"fieldValueByName":{"name":"Done"}},
        {"id":"PVTI_7","project":{"id":"PVT_1","number":1,"owner":{"login":"octo"},"field":{"id":"F1","options":[
          {"name":"Backlog","id":"opt_backlog"},{"name":"Ready","id":"opt_ready"},
          {"name":"In progress","id":"opt_progress"},{"name":"In review","id":"opt_review"},{"name":"Done","id":"opt_done"}
        ]}},"fieldValueByName":{"name":"Ready"}}
      ]}
    }}
  }
}"#;

const PAGES: &str = r#"[
  {"data":{"viewer":{"projectV2":{"items":{"pageInfo":{"hasNextPage":true,"endCursor":"c1"},"nodes":[
    {"fieldValueByName":{"name":"Ready"},"content":{"number":7,"title":"Fix: Login É quebrado!!","repository":{"nameWithOwner":"acme/app"}}},
    {"fieldValueByName":{"name":"Done"},"content":{"number":9,"title":"Other","repository":{"nameWithOwner":"acme/other"}}},
    {"fieldValueByName":null,"content":{"number":8,"title":"Sem status","repository":{"nameWithOwner":"acme/app"}}}
  ]}}}}},
  {"data":{"viewer":{"projectV2":{"items":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[
    {"fieldValueByName":{"name":"Backlog"},"content":{"number":150,"title":"Último card","repository":{"nameWithOwner":"ACME/app"}}},
    {"fieldValueByName":{"name":"Ready"},"content":{}}
  ]}}}}}
]"#;

struct Fake {
    tmp: TempDir,
    db: PathBuf,
    card: PathBuf,
    pages: PathBuf,
    log: PathBuf,
    args: PathBuf,
}

impl Fake {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        fs::write(&gh, GH_SCRIPT).unwrap();
        let mut permissions = fs::metadata(&gh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gh, permissions).unwrap();
        let card = tmp.path().join("card.json");
        let pages = tmp.path().join("pages.json");
        fs::write(&card, CARD).unwrap();
        fs::write(&pages, PAGES).unwrap();
        let db = tmp.path().join("comms.db");
        let log = tmp.path().join("gh.log");
        let args = tmp.path().join("gh.log.args");
        Self { tmp, db, card, pages, log, args }
    }

    fn root(&self) -> &Path { self.tmp.path() }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn command(&self, args: &[&str]) -> assert_cmd::Command {
        let mut command = common::comms();
        command.current_dir(self.root())
            .env("COMMS_DB", &self.db)
            .env("PATH", format!("{}:{}", self.root().join("bin").display(), std::env::var("PATH").unwrap()))
            .env("FAKE_GH_LOG", &self.log)
            .env("FAKE_GH_CARD", &self.card)
            .env("FAKE_GH_PAGES", &self.pages)
            .args(args);
        command
    }

    fn ok(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(output.status.success(), "args={args:?} stderr={}", String::from_utf8_lossy(&output.stderr));
        output
    }

    fn set_repo(&self) {
        self.ok(&["set", "board.repo", "acme/app"]);
    }

    fn log_lines(&self) -> Vec<String> {
        fs::read_to_string(&self.log).unwrap_or_default().lines().map(str::to_string).collect()
    }
}

#[test]
fn board_item_set_and_external_errors_use_graphql_card() {
    let fake = Fake::new();
    fake.set_repo();
    let item = fake.ok(&["board-item", "7"]);
    assert_eq!(String::from_utf8_lossy(&item.stdout), "PVTI_7\tReady\n");

    let invalid = fake.run(&["board-set", "7", "Doing"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("Backlog, Ready, In progress, In review, Done"));
    fake.ok(&["board-set", "7", "In progress"]);
    assert!(fs::read_to_string(&fake.args).unwrap().contains("--single-select-option-id opt_progress"));

    let mut card: Value = serde_json::from_str(CARD).unwrap();
    card["data"]["repository"]["issue"]["projectItems"]["nodes"] = serde_json::json!([card["data"]["repository"]["issue"]["projectItems"]["nodes"][0].clone()]);
    fs::write(&fake.card, serde_json::to_vec(&card).unwrap()).unwrap();
    let missing = fake.run(&["--json", "board-item", "7"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(serde_json::from_slice::<Value>(&missing.stderr).unwrap()["error"]["code"], "not_found");

    let failed = fake.command(&["--json", "board-item", "7"]).env("FAKE_GH_FAIL", "1").output().unwrap();
    assert_eq!(failed.status.code(), Some(1));
    let error = serde_json::from_slice::<Value>(&failed.stderr).unwrap();
    assert!(error["error"]["message"].as_str().unwrap().starts_with("ERRO gh CLI: gh: Could not resolve"));
}

#[test]
fn start_records_item_and_handoff_with_minimal_gh_calls() {
    let fake = Fake::new();
    let started = json(&fake.ok(&["start", "7", "--no-worktree", "--json"]));
    assert_eq!(started["board"]["from"], "Ready");
    assert_eq!(started["board"]["to"], "In progress");
    assert!(started["worktree"].is_null());
    assert_eq!(fake.log_lines(), vec!["repo view", "api graphql", "project item-edit"]);
    let items = json(&fake.ok(&["items", "--json"]));
    assert_eq!(items["items"][0]["title"], "#7: Fix: Login É quebrado!!");
    let turns = json(&fake.ok(&["turns", "--json"]));
    assert_eq!(turns["turns"][0]["kind"], "handoff");

    let fake = Fake::new();
    fake.set_repo();
    fake.ok(&["start", "7", "--no-worktree"]);
    assert_eq!(fake.log_lines(), vec!["api graphql", "project item-edit"]);
}

#[test]
fn start_creates_real_worktree_on_expected_branch() {
    let fake = Fake::new();
    fake.set_repo();
    common::init_repo(fake.root());
    fake.ok(&["start", "7"]);

    let worktree = fake.root().join(".worktrees/issue-7-fix-login-quebrado");
    assert!(worktree.is_dir());
    let output = ProcessCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&worktree)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "feat/7-fix-login-quebrado\n");
}

#[test]
fn finish_only_completes_exact_issue_refs_and_comments() {
    let fake = Fake::new();
    fake.set_repo();
    fake.ok(&["item-add", "--titulo", "#7: certo", "--dono", "dev", "--refs", "issue:7"]);
    fake.ok(&["item-add", "--titulo", "#70: errado", "--dono", "dev", "--refs", "issue:70"]);
    fake.ok(&["finish", "7", "--pr", "https://example.test/pr/7"]);

    let items = json(&fake.ok(&["items", "--all", "--json"]));
    let items = items["items"].as_array().unwrap();
    assert_eq!(items.iter().find(|item| item["id"] == 1).unwrap()["status"], "feito");
    assert_eq!(items.iter().find(|item| item["id"] == 2).unwrap()["status"], "aberto");
    let turns = json(&fake.ok(&["turns", "--all", "--json"]));
    assert_eq!(turns["turns"][0]["kind"], "gate");
    let args = fs::read_to_string(&fake.args).unwrap();
    assert!(args.contains("--single-select-option-id opt_review"));
    assert!(args.contains("issue comment 7"));
}

#[test]
fn board_sync_paginates_all_cards_and_supports_explicit_owner() {
    let fake = Fake::new();
    fake.set_repo();
    let doc = json(&fake.ok(&["board-sync", "--json"]));
    assert_eq!(doc["cards"], 5);
    assert_eq!(doc["synced"].as_array().unwrap().len(), 2);
    let state = json(&fake.ok(&["state", "--json"]));
    let kv = state["kv"].as_array().unwrap();
    assert!(kv.iter().any(|row| row["key"] == "board.issue.7.status" && row["value"] == "Ready"));
    assert!(kv.iter().any(|row| row["key"] == "board.issue.150.status" && row["value"] == "Backlog"));

    let owner_pages = fs::read_to_string(&fake.pages).unwrap().replace("viewer", "repositoryOwner");
    fs::write(&fake.pages, owner_pages).unwrap();
    fake.ok(&["set", "board.owner", "acme"]);
    fake.ok(&["board-sync"]);
    let args = fs::read_to_string(&fake.args).unwrap();
    assert!(args.contains("repositoryOwner"));
    assert!(args.contains("login=acme"));
}
