use crate::db::{self, AgentRow};
use crate::error::{Code, CommsError, Result};
use crate::policy::{BROADCAST, CROSS, DEFAULT_ROSTER, HUMAN, MAX_NUNCA, PROMPT_SECTIONS, RESERVED_NAMES};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub enum Origin {
    Padrao(PathBuf),
    AdHoc { description: String, created_by: String, created_at: String, updated_at: String },
    Humano,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub origin: Origin,
}

#[derive(Debug)]
pub struct Roster {
    pub agents: Vec<Agent>,
    pub conflicts: Vec<String>,
}

impl Roster {
    pub fn load(prompts_dir: Option<&Path>, conn: Option<&Connection>) -> Result<Self> {
        let mut agents = Vec::new();
        if let Some(dir) = prompts_dir
            && let Ok(entries) = fs::read_dir(dir)
        {
            let mut paths: Vec<PathBuf> = entries
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path.extension().and_then(|ext| ext.to_str()) == Some("md")
                        && !path.file_name().and_then(|name| name.to_str()).unwrap_or("").starts_with('.')
                })
                .collect();
            paths.sort();
            agents.extend(paths.into_iter().map(|path| Agent {
                name: path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                origin: Origin::Padrao(path),
            }));
        }
        if agents.is_empty() {
            agents.extend(DEFAULT_ROSTER.iter().map(|name| Agent {
                name: (*name).to_string(),
                origin: Origin::Fallback,
            }));
        }

        let mut conflicts = Vec::new();
        if let Some(conn) = conn.filter(|conn| db::table_exists(conn, "agents").unwrap_or(false)) {
            let mut stmt = conn.prepare("SELECT name, description, created_by, created_at, updated_at FROM agents ORDER BY name")?;
            let rows = stmt.query_map([], |row| {
                Ok(AgentRow {
                    name: row.get(0)?, description: row.get(1)?, prompt: String::new(),
                    created_by: row.get(2)?, created_at: row.get(3)?, updated_at: row.get(4)?,
                })
            })?;
            for row in rows {
                let row = row?;
                let origin = Origin::AdHoc {
                    description: row.description, created_by: row.created_by,
                    created_at: row.created_at, updated_at: row.updated_at,
                };
                if let Some(index) = agents.iter().position(|agent| agent.name == row.name) {
                    match agents[index].origin {
                        Origin::Padrao(_) => conflicts.push(row.name),
                        Origin::Fallback => agents[index] = Agent { name: row.name, origin },
                        _ => {}
                    }
                } else {
                    agents.push(Agent { name: row.name, origin });
                }
            }
        }
        if !agents.iter().any(|agent| agent.name == HUMAN) {
            agents.push(Agent { name: HUMAN.to_string(), origin: Origin::Humano });
        }
        Ok(Self { agents, conflicts })
    }

    pub fn names(&self) -> Vec<&str> {
        self.agents.iter().map(|agent| agent.name.as_str()).collect()
    }

    pub fn description(&self, name: &str) -> String {
        match self.agents.iter().find(|agent| agent.name == name).map(|agent| &agent.origin) {
            Some(Origin::Padrao(path)) => fs::read_to_string(path).map(|text| describe(&text)).unwrap_or_default(),
            Some(Origin::AdHoc { description, .. }) => description.clone(),
            _ => String::new(),
        }
    }

    pub fn prompt(&self, name: &str, conn: Option<&Connection>) -> Result<Option<String>> {
        match self.agents.iter().find(|agent| agent.name == name).map(|agent| &agent.origin) {
            Some(Origin::Padrao(path)) => Ok(Some(fs::read_to_string(path)?)),
            Some(Origin::AdHoc { .. }) => match conn {
                Some(conn) => Ok(conn.query_row("SELECT prompt FROM agents WHERE name=?1", [name], |row| row.get(0)).optional()?),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    pub fn check_sender(&self, flag: &str, value: &str) -> Result<()> {
        self.check(flag, value, self.names())
    }

    pub fn check_recipient(&self, flag: &str, value: &str) -> Result<()> {
        let mut valid = self.names();
        valid.push(BROADCAST);
        self.check(flag, value, valid)
    }

    pub fn check_scope(&self, flag: &str, value: &str) -> Result<()> {
        let mut valid = self.names();
        valid.push(CROSS);
        self.check(flag, value, valid)
    }

    fn check(&self, flag: &str, value: &str, valid: Vec<&str>) -> Result<()> {
        if valid.contains(&value) {
            return Ok(());
        }
        let hint = if RESERVED_NAMES.contains(&value) {
            String::new()
        } else {
            format!(" Papel novo? comms agent-add {value} --descricao \"...\" --por <papel>")
        };
        Err(CommsError::new(
            Code::Usage,
            format!("ERRO: valor inválido para {flag}: '{value}'. Válidos: {}.{hint}", valid.join(", ")),
        ))
    }
}

pub fn describe(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')) else {
        return String::new();
    };
    let mut chars = line.chars();
    let prefix: String = chars.by_ref().take(140).collect();
    if chars.next().is_some() { format!("{prefix}…") } else { prefix }
}

pub fn valid_agent_name(name: &str) -> bool {
    static NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_-]{1,31}$").unwrap());
    NAME_RE.is_match(name) && !RESERVED_NAMES.contains(&name)
}

pub fn prompt_problems(text: &str) -> Vec<String> {
    let mut problems: Vec<String> = PROMPT_SECTIONS
        .iter()
        .filter(|section| !text.contains(**section))
        .map(|section| format!("seção '{section}' ausente"))
        .collect();
    let count = text.matches("NUNCA").count();
    if count > MAX_NUNCA {
        problems.push(format!("{count}x NUNCA — se tudo grita, nada grita (máx 5)"));
    }
    problems
}

pub fn render(template: &str, values: &[(&str, &str)]) -> String {
    values.iter().fold(template.to_string(), |text, (key, value)| text.replace(&format!("{{{{{key}}}}}"), value))
}
