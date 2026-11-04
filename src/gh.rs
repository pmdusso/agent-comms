use crate::db;
use crate::error::{Code, CommsError, Result};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use std::process::Command;

pub const CARD_QUERY: &str = r#"query($repoOwner: String!, $repoName: String!, $issue: Int!) {
  viewer { login }
  repository(owner: $repoOwner, name: $repoName) {
    issue(number: $issue) {
      title
      projectItems(first: 50) {
        nodes {
          id
          project {
            id
            number
            owner { ... on User { login } ... on Organization { login } }
            field(name: "Status") { ... on ProjectV2SingleSelectField { id options { id name } } }
          }
          fieldValueByName(name: "Status") { ... on ProjectV2ItemFieldSingleSelectValue { name } }
        }
      }
    }
  }
}"#;

pub const ITEMS_QUERY_VIEWER: &str = r#"query($number: Int!, $endCursor: String) { viewer { projectV2(number: $number) { items(first: 100, after: $endCursor) { pageInfo { hasNextPage endCursor } nodes { fieldValueByName(name: "Status") { ... on ProjectV2ItemFieldSingleSelectValue { name } } content { ... on Issue { number title repository { nameWithOwner } } ... on PullRequest { number title repository { nameWithOwner } } } } } } } }"#;

pub const ITEMS_QUERY_OWNER: &str = r#"query($login: String!, $number: Int!, $endCursor: String) { repositoryOwner(login: $login) { ... on ProjectV2Owner { projectV2(number: $number) { items(first: 100, after: $endCursor) { pageInfo { hasNextPage endCursor } nodes { fieldValueByName(name: "Status") { ... on ProjectV2ItemFieldSingleSelectValue { name } } content { ... on Issue { number title repository { nameWithOwner } } ... on PullRequest { number title repository { nameWithOwner } } } } } } } } }"#;

pub fn run<S: AsRef<str>>(args: &[S]) -> Result<String> {
    let output = Command::new("gh")
        .args(args.iter().map(|arg| arg.as_ref()))
        .output()
        .map_err(|_| CommsError::new(Code::External, "ERRO: gh CLI não encontrado no PATH."))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let message = if stderr.is_empty() { stdout } else { stderr };
        return Err(CommsError::new(Code::External, format!("ERRO gh CLI: {message}")));
    }
    Ok(stdout)
}

pub fn repo(conn: &Connection) -> Result<(String, String, String)> {
    let full = match db::kv_get(conn, "board.repo")? {
        Some(repo) => repo,
        None => run(&["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"] )?,
    };
    if full.matches('/').count() != 1 {
        return Err(CommsError::new(Code::Invalid, format!("ERRO: board.repo deve ser dono/repo (atual: '{full}')")));
    }
    let (owner, name) = full.split_once('/').unwrap();
    Ok((owner.to_string(), name.to_string(), full))
}

pub struct BoardCfg {
    pub num: u64,
    pub owner: String,
}

pub fn board_cfg(conn: &Connection) -> Result<BoardCfg> {
    let raw_num = db::kv_get(conn, "board.project")?.unwrap_or_else(|| "1".to_string());
    let num = raw_num.parse::<u64>().map_err(|_| CommsError::new(Code::Invalid, format!("ERRO: board.project deve ser o número do project (atual: '{raw_num}')")))?;
    let owner = db::kv_get(conn, "board.owner")?.unwrap_or_else(|| "@me".to_string());
    Ok(BoardCfg { num, owner })
}

#[derive(Debug)]
pub struct Card {
    pub item_id: String,
    pub status: Option<String>,
    pub project_id: String,
    pub field_id: String,
    pub options: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct CardResponse { data: CardData }
#[derive(Deserialize)]
struct CardData { viewer: Viewer, repository: CardRepository }
#[derive(Deserialize)]
struct Viewer { login: String }
#[derive(Deserialize)]
struct CardRepository { issue: Option<CardIssue> }
#[derive(Deserialize)]
struct CardIssue {
    title: String,
    #[serde(rename = "projectItems")]
    project_items: ProjectItems,
}
#[derive(Deserialize)]
struct ProjectItems { nodes: Vec<CardNode> }
#[derive(Deserialize)]
struct CardNode {
    id: String,
    project: CardProject,
    #[serde(rename = "fieldValueByName")]
    field_value: Option<StatusValue>,
}
#[derive(Deserialize)]
struct CardProject {
    id: String,
    number: u64,
    owner: ProjectOwner,
    field: Option<SelectField>,
}
#[derive(Deserialize)]
struct ProjectOwner { login: String }
#[derive(Deserialize)]
struct SelectField { id: String, options: Vec<SelectOption> }
#[derive(Deserialize)]
struct SelectOption { id: String, name: String }
#[derive(Debug, Deserialize)]
struct StatusValue { name: String }

fn parse_json<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|error| CommsError::new(Code::External, format!("ERRO gh CLI: resposta JSON inválida: {error}")))
}

pub fn card_lookup(cfg: &BoardCfg, repo: &(String, String, String), issue: u64) -> Result<(String, Option<Card>, Vec<String>)> {
    let query = format!("query={CARD_QUERY}");
    let owner_arg = format!("repoOwner={}", repo.0);
    let name_arg = format!("repoName={}", repo.1);
    let issue_arg = format!("issue={issue}");
    let output = run(&["api", "graphql", "-f", &query, "-f", &owner_arg, "-f", &name_arg, "-F", &issue_arg])?;
    let response: CardResponse = parse_json(&output)?;
    let issue_data = response.data.repository.issue.ok_or_else(|| CommsError::new(Code::External, format!("ERRO gh CLI: resposta sem a issue #{issue}.")))?;
    let target_login = if cfg.owner == "@me" { &response.data.viewer.login } else { &cfg.owner };
    let matches: Vec<CardNode> = issue_data.project_items.nodes.into_iter().filter(|node| {
        node.project.number == cfg.num && node.project.owner.login.eq_ignore_ascii_case(target_login)
    }).collect();
    let mut warnings = Vec::new();
    if matches.len() > 1 {
        warnings.push(format!("AVISO: issue #{issue} tem {} cards no board {}/{}; usando o primeiro.", matches.len(), cfg.owner, cfg.num));
    }
    let Some(node) = matches.into_iter().next() else { return Ok((issue_data.title, None, warnings)) };
    let field = node.project.field.ok_or_else(|| CommsError::new(Code::Invalid, format!("ERRO: project {}/{} não tem campo 'Status' de seleção única.", cfg.owner, cfg.num)))?;
    let card = Card {
        item_id: node.id,
        status: node.field_value.map(|value| value.name),
        project_id: node.project.id,
        field_id: field.id,
        options: field.options.into_iter().map(|option| (option.name, option.id)).collect(),
    };
    Ok((issue_data.title, Some(card), warnings))
}

pub fn set_status(card: &Card, status: &str) -> Result<()> {
    let Some((_, option_id)) = card.options.iter().find(|(name, _)| name == status) else {
        return Err(CommsError::new(
            Code::Invalid,
            format!("ERRO: status '{status}' não existe neste board. Disponíveis: {}", card.options.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ")),
        ));
    };
    run(&[
        "project", "item-edit", "--project-id", &card.project_id, "--id", &card.item_id,
        "--field-id", &card.field_id, "--single-select-option-id", option_id,
    ])?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct BoardNode {
    #[serde(rename = "fieldValueByName")]
    field_value: Option<StatusValue>,
    pub content: Option<BoardContent>,
}

#[derive(Debug, Deserialize)]
pub struct BoardContent {
    pub number: Option<u64>,
    pub title: Option<String>,
    pub repository: Option<BoardRepository>,
}

#[derive(Debug, Deserialize)]
pub struct BoardRepository { #[serde(rename = "nameWithOwner")] pub name_with_owner: String }

impl BoardNode {
    pub fn status(&self) -> Option<&str> {
        self.field_value.as_ref().map(|value| value.name.as_str())
    }
}

pub fn board_items(cfg: &BoardCfg) -> Result<Vec<BoardNode>> {
    let number = format!("number={}", cfg.num);
    let output = if cfg.owner == "@me" {
        let query = format!("query={ITEMS_QUERY_VIEWER}");
        run(&["api", "graphql", "--paginate", "--slurp", "-f", &query, "-F", &number])?
    } else {
        let query = format!("query={ITEMS_QUERY_OWNER}");
        let login = format!("login={}", cfg.owner);
        run(&["api", "graphql", "--paginate", "--slurp", "-f", &query, "-F", &number, "-f", &login])?
    };
    let pages: Vec<Value> = parse_json(&output)?;
    let mut nodes = Vec::new();
    for page in pages {
        let pointer = if cfg.owner == "@me" { "/data/viewer/projectV2/items/nodes" } else { "/data/repositoryOwner/projectV2/items/nodes" };
        let page_nodes = page.pointer(pointer).and_then(Value::as_array).ok_or_else(|| {
            CommsError::new(Code::External, format!("ERRO gh CLI: resposta sem o project {}/{}.", cfg.owner, cfg.num))
        })?;
        for node in page_nodes {
            nodes.push(serde_json::from_value(node.clone()).map_err(|error| CommsError::new(Code::External, format!("ERRO gh CLI: resposta JSON inválida: {error}")))?);
        }
    }
    Ok(nodes)
}

pub fn issue_comment(full_repo: &str, issue: u64, body: &str) -> Result<()> {
    let issue = issue.to_string();
    run(&["issue", "comment", &issue, "-R", full_repo, "-b", body])?;
    Ok(())
}
