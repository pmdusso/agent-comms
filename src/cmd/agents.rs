use crate::cli::{AgentAddArgs, AgentNameArgs, AgentSetArgs, AgentShowArgs, BriefArgs};
use crate::db::{AgentRow, Kv, Turn, WorkItem};
use crate::error::{Code, CommsError, Result};
use crate::guard::pii_check;
use crate::out::Out;
use crate::policy::{BROADCAST, HUMAN};
use crate::roster::{self, Agent, Origin, Roster};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Read};
use std::path::Path;

const AGENT_PROMPT_TEMPLATE: &str = include_str!("../../templates/agent_prompt.md");

fn row_by_name(conn: &Connection, name: &str) -> Result<Option<AgentRow>> {
    Ok(conn.query_row(
        "SELECT name, description, prompt, created_by, created_at, updated_at FROM agents WHERE name=?1",
        [name],
        |row| Ok(AgentRow {
            name: row.get(0)?, description: row.get(1)?, prompt: row.get(2)?,
            created_by: row.get(3)?, created_at: row.get(4)?, updated_at: row.get(5)?,
        }),
    ).optional()?)
}

fn agent_from_row(row: &AgentRow) -> Agent {
    Agent {
        name: row.name.clone(),
        origin: Origin::AdHoc {
            description: row.description.clone(), created_by: row.created_by.clone(),
            created_at: row.created_at.clone(), updated_at: row.updated_at.clone(),
        },
    }
}

pub fn origin_name(origin: &Origin) -> &'static str {
    match origin {
        Origin::Padrao(_) => "padrao",
        Origin::AdHoc { .. } => "adhoc",
        Origin::Humano => "humano",
        Origin::Fallback => "fallback",
    }
}

fn agent_value(agent: &Agent, description: String, turns: Option<i64>) -> Value {
    let (source, created_by, created_at, updated_at) = match &agent.origin {
        Origin::Padrao(_) => (Some(format!("docs/prompts/{}.md", agent.name)), None, None, None),
        Origin::AdHoc { created_by, created_at, updated_at, .. } => (
            Some("comms.db".to_string()), Some(created_by.clone()), Some(created_at.clone()), Some(updated_at.clone()),
        ),
        Origin::Humano | Origin::Fallback => (None, None, None, None),
    };
    let mut value = json!({
        "name": agent.name,
        "origin": origin_name(&agent.origin),
        "source": source,
        "description": description,
        "created_by": created_by,
        "created_at": created_at,
        "updated_at": updated_at,
    });
    if let Some(turns) = turns {
        value["turns"] = json!(turns);
    }
    value
}

fn normalize_description(value: &str) -> String {
    value.trim().replace(['\r', '\n'], " ")
}

fn read_prompt(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        if std::io::stdin().is_terminal() {
            return Err(CommsError::new(Code::Invalid, "ERRO: stdin é um terminal; envie o prompt por pipe ou use um arquivo."));
        }
        let mut bytes = Vec::new();
        std::io::stdin().lock().read_to_end(&mut bytes).map_err(|error| CommsError::new(Code::Invalid, format!("ERRO: não deu para ler '-': {error}")))?;
        return String::from_utf8(bytes).map_err(|error| CommsError::new(Code::Invalid, format!("ERRO: não deu para ler '-': {error}")));
    }
    fs::read_to_string(path).map_err(|error| CommsError::new(Code::Invalid, format!("ERRO: não deu para ler '{}': {error}", path.display())))
}

fn validate_prompt(prompt: &str, force: bool) -> Result<()> {
    let problems = roster::prompt_problems(prompt);
    if !force && !problems.is_empty() {
        return Err(CommsError::new(
            Code::Guard,
            format!("ERRO: prompt fora do esqueleto: {}. Use --force se for exceção consciente.", problems.join("; ")),
        ));
    }
    Ok(())
}

fn default_conflict(roster: &Roster, name: &str) -> bool {
    roster.agents.iter().any(|agent| agent.name == name && matches!(agent.origin, Origin::Padrao(_)))
}

fn default_conflict_error(name: &str) -> CommsError {
    CommsError::new(Code::Conflict, format!("ERRO: '{name}' é persona padrão (docs/prompts/{name}.md); edite o arquivo."))
}

fn generated_prompt(name: &str, created_by: &str, description: &str) -> String {
    roster::render(AGENT_PROMPT_TEMPLATE, &[
        ("NOME", &name.to_uppercase()), ("nome", name), ("por", created_by), ("descricao", description),
    ])
}

pub fn agent_add(conn: &Connection, roster: &Roster, args: &AgentAddArgs, out: &mut Out) -> Result<u8> {
    roster.check_sender("--por", &args.por)?;
    if !roster::valid_agent_name(&args.nome) {
        return Err(CommsError::new(Code::Invalid, format!("ERRO: nome inválido '{}': use ^[a-z][a-z0-9_-]{{1,31}}$ e não use human/todos/cross.", args.nome)));
    }
    if default_conflict(roster, &args.nome) {
        return Err(default_conflict_error(&args.nome));
    }
    if row_by_name(conn, &args.nome)?.is_some() {
        return Err(CommsError::new(Code::Conflict, format!("ERRO: agente '{}' já existe (ad-hoc); use comms agent-set.", args.nome)));
    }
    let description = normalize_description(&args.descricao);
    if description.is_empty() {
        return Err(CommsError::new(Code::Invalid, "ERRO: descrição não pode ser vazia."));
    }
    let prompt = match &args.prompt_file {
        Some(path) => {
            let prompt = read_prompt(path)?;
            validate_prompt(&prompt, args.force)?;
            prompt
        }
        None => generated_prompt(&args.nome, &args.por, &description),
    };
    pii_check(&[Some(&description), Some(&prompt)], args.force)?;
    conn.execute(
        "INSERT INTO agents (name, description, prompt, created_by) VALUES (?1,?2,?3,?4)",
        params![args.nome, description, prompt, args.por],
    )?;
    out.say(format!("agente {} criado (ad-hoc, por {}) — suba com: comms brief {}", args.nome, args.por, args.nome));
    let row = row_by_name(conn, &args.nome)?.ok_or_else(|| CommsError::new(Code::Internal, "ERRO SQLite: agente inserido não encontrado"))?;
    let agent = agent_from_row(&row);
    let mut value = agent_value(&agent, row.description.clone(), None);
    value["prompt"] = json!(row.prompt);
    out.set(json!({"agent": value}));
    Ok(0)
}

pub fn agent_set(conn: &mut Connection, roster: &Roster, args: &AgentSetArgs, out: &mut Out) -> Result<u8> {
    agent_set_with_probe(conn, roster, args, out, || {})
}

fn agent_set_with_probe(conn: &mut Connection, roster: &Roster, args: &AgentSetArgs, out: &mut Out, probe: impl FnOnce()) -> Result<u8> {
    if default_conflict(roster, &args.nome) {
        return Err(default_conflict_error(&args.nome));
    }
    if args.descricao.is_none() && args.prompt_file.is_none() {
        return Err(CommsError::new(Code::Invalid, "nada para atualizar"));
    }
    let provided_prompt = args.prompt_file.as_ref().map(|path| -> Result<String> {
        let prompt = read_prompt(path)?;
        validate_prompt(&prompt, args.force)?;
        Ok(prompt)
    }).transpose()?;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let current = row_by_name(&tx, &args.nome)?.ok_or_else(|| CommsError::new(Code::NotFound, format!("ERRO: agente ad-hoc '{}' não existe.", args.nome)))?;
    let description = match &args.descricao {
        Some(value) => {
            let value = normalize_description(value);
            if value.is_empty() {
                return Err(CommsError::new(Code::Invalid, "ERRO: descrição não pode ser vazia."));
            }
            value
        }
        None => current.description.clone(),
    };
    let previous_default = generated_prompt(&args.nome, &current.created_by, &current.description);
    let prompt = match provided_prompt {
        Some(prompt) => prompt,
        None if args.descricao.is_some() && current.prompt == previous_default => generated_prompt(&args.nome, &current.created_by, &description),
        None => current.prompt.clone(),
    };
    pii_check(&[Some(&description), Some(&prompt)], args.force)?;
    probe();
    tx.execute(
        "UPDATE agents SET description=?2, prompt=?3, updated_at=datetime('now','localtime') WHERE name=?1",
        params![args.nome, description, prompt],
    )?;
    let row = row_by_name(&tx, &args.nome)?.ok_or_else(|| CommsError::new(Code::Internal, "ERRO SQLite: agente atualizado não encontrado"))?;
    let agent = agent_from_row(&row);
    let mut value = agent_value(&agent, row.description.clone(), None);
    value["prompt"] = json!(row.prompt);
    tx.commit()?;
    out.say(format!("agente {} atualizado", args.nome));
    out.set(json!({"agent": value}));
    Ok(0)
}

pub fn agent_rm(conn: &Connection, args: &AgentNameArgs, out: &mut Out) -> Result<u8> {
    if row_by_name(conn, &args.nome)?.is_none() {
        return Err(CommsError::new(Code::NotFound, format!("ERRO: agente ad-hoc '{}' não existe.", args.nome)));
    }
    conn.execute("DELETE FROM agents WHERE name=?1", [&args.nome])?;
    out.say(format!("agente {} removido (histórico preservado)", args.nome));
    out.set(json!({"removed": args.nome}));
    Ok(0)
}

pub fn agent_show(conn: &Connection, roster: &Roster, args: &AgentShowArgs, out: &mut Out) -> Result<u8> {
    let (agent, description, prompt) = if args.adhoc {
        let row = row_by_name(conn, &args.nome)?.ok_or_else(|| CommsError::new(Code::NotFound, format!("ERRO: agente ad-hoc '{}' não existe.", args.nome)))?;
        (agent_from_row(&row), row.description.clone(), Some(row.prompt))
    } else {
        let agent = roster.agents.iter().find(|agent| agent.name == args.nome).cloned();
        let prompt = roster.prompt(&args.nome, Some(conn))?;
        let description = roster.description(&args.nome);
        match agent {
            Some(agent) => (agent, description, prompt),
            None => return Err(CommsError::new(Code::NotFound, format!("ERRO: agente '{}' não tem prompt.", args.nome))),
        }
    };
    let prompt = prompt.ok_or_else(|| CommsError::new(Code::NotFound, format!("ERRO: agente '{}' não tem prompt.", args.nome)))?;
    out.raw(&prompt);
    let mut value = agent_value(&agent, description, None);
    value["prompt"] = json!(prompt);
    out.set(json!({"agent": value}));
    Ok(0)
}

pub fn frozen_roster_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type='table' AND name IN ('turns','work_items')")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))?;
    let mut frozen = Vec::new();
    for row in rows {
        let (name, sql) = row?;
        if sql.is_some_and(|sql| sql.contains("CHECK (sender IN") || sql.contains("CHECK (owner IN")) {
            frozen.push(name);
        }
    }
    Ok(frozen)
}

pub fn agents(conn: &Connection, roster: &Roster, prompts_dir: Option<&Path>, out: &mut Out) -> Result<u8> {
    let mut counts = HashMap::new();
    let mut stmt = conn.prepare("SELECT sender, COUNT(*) FROM turns GROUP BY sender")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    for row in rows {
        let (sender, count) = row?;
        counts.insert(sender, count);
    }
    let prompts_label = prompts_dir.map(|path| path.display().to_string()).unwrap_or_else(|| "docs/prompts".to_string());
    out.say(format!("== Roster ({prompts_label} + comms.db) =="));
    let mut docs = Vec::new();
    for agent in &roster.agents {
        let count = counts.get(&agent.name).copied().unwrap_or(0);
        let description = roster.description(&agent.name);
        let label = match &agent.origin {
            Origin::Padrao(_) => format!("docs/prompts/{}.md — {}", agent.name, description),
            Origin::AdHoc { created_by, created_at, .. } => format!("ad-hoc (por {created_by}, {created_at}) — {description}"),
            Origin::Humano => "humano".to_string(),
            Origin::Fallback => "padrão embutido (sem prompt)".to_string(),
        };
        out.say(format!("  {:12} {:>4} turnos   {}", agent.name, count, label));
        docs.push(agent_value(agent, description, Some(count)));
    }
    out.say(format!("\nBroadcast: {BROADCAST} (só destinatário)"));
    out.say("Agente novo: comms agent-add <nome> --descricao \"...\" --por <papel>   (padrão versionado: docs/prompts/<nome>.md)");
    let mut orphans: Vec<String> = counts.keys().filter(|name| !roster.agents.iter().any(|agent| &agent.name == *name)).cloned().collect();
    orphans.sort();
    if !orphans.is_empty() {
        out.say(format!("\nSem prompt, mas presentes no histórico: {}", orphans.join(", ")));
    }
    if !roster.conflicts.is_empty() {
        out.say(format!("\nAVISO: {} existem em docs/prompts/ e no banco; vale o arquivo. Remova a cópia: comms agent-rm <nome>", roster.conflicts.join(", ")));
    }
    let frozen = frozen_roster_tables(conn)?;
    if !frozen.is_empty() {
        out.say(format!("\nAVISO: {} ainda congelam o roster num CHECK do schema (banco criado em versão <= 3). Agentes novos serão rejeitados até você rodar comms migrate (preserva os dados).", frozen.join(", ")));
    }
    out.set(json!({
        "prompts_dir": prompts_label,
        "agents": docs,
        "broadcast": BROADCAST,
        "orphans": orphans,
        "conflicts": roster.conflicts,
        "frozen_tables": frozen,
    }));
    Ok(0)
}

pub fn brief(conn: &Connection, roster: &Roster, args: &BriefArgs, out: &mut Out) -> Result<u8> {
    roster.check_sender("agente", &args.agente)?;
    let prompt = roster.prompt(&args.agente, Some(conn))?;
    if let Some(prompt) = &prompt {
        out.raw(prompt.trim_end());
    } else if args.agente != HUMAN {
        out.say(format!("(sem prompt em docs/prompts/{}.md — agente só no histórico)", args.agente));
    }
    out.say(format!("\n{}\n== Estado do canal para `{}` ==\n", "=".repeat(70), args.agente));
    let mut stmt = conn.prepare("SELECT key, value, updated_at FROM kv ORDER BY key")?;
    let kv = stmt.query_map([], Kv::from_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
    for row in &kv {
        out.say(format!("{}: {}", row.key, row.value));
    }

    out.say("\n-- Roster --");
    let mut roster_doc = Vec::new();
    for agent in &roster.agents {
        if agent.name == HUMAN {
            out.say("  human — humano (decisão final: merge/deploy)");
        } else {
            let description = roster.description(&agent.name);
            if description.is_empty() {
                out.say(format!("  {}", agent.name));
            } else {
                out.say(format!("  {} — {}", agent.name, description));
            }
        }
        roster_doc.push(json!({"name": agent.name, "origin": origin_name(&agent.origin), "description": roster.description(&agent.name)}));
    }

    out.say("\n-- Seus itens abertos --");
    let mut stmt = conn.prepare("SELECT * FROM work_items WHERE owner=?1 AND status != 'feito' ORDER BY id")?;
    let items = stmt.query_map([&args.agente], WorkItem::from_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
    for item in &items {
        let extra = item.blocker.as_deref().filter(|blocker| !blocker.is_empty()).map(|blocker| format!(" — bloqueio: {blocker}")).unwrap_or_default();
        out.say(format!("#{} [{}] {}{}", item.id, item.status, item.title, extra));
    }
    out.say(if items.is_empty() { "(nenhum)" } else { "" });
    out.say("-- Turnos abertos para você --");
    let mut stmt = conn.prepare("SELECT * FROM turns WHERE resolved_at IS NULL AND recipient IN (?1, ?2) ORDER BY id")?;
    let turns = stmt.query_map(params![args.agente, BROADCAST], Turn::from_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
    for turn in &turns {
        let refs = turn.refs.as_deref().filter(|refs| !refs.is_empty()).map(|refs| format!("\nrefs: {refs}")).unwrap_or_default();
        out.say(format!("\n#{} {} → {} · {}{}", turn.id, turn.sender, turn.recipient, turn.kind, refs));
        out.say(format!("{}\n\n{}", turn.title, turn.body));
    }
    out.say(if turns.is_empty() { "(nenhum)" } else { "" });
    out.say("\nResolva o que atender: comms resolve <ID>");
    let kv_doc: Vec<Value> = kv.iter().map(|row| json!({"key": row.key, "value": row.value})).collect();
    out.set(json!({
        "agent": args.agente,
        "prompt": prompt,
        "kv": kv_doc,
        "roster": roster_doc,
        "items": items,
        "turns": turns,
    }));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::time::Duration;

    #[test]
    fn agent_set_holds_write_lock_from_read_through_update() {
        let tmp = tempfile::tempdir().unwrap();
        let mut conn = db::open_rw(&tmp.path().join("comms.db")).unwrap();
        conn.execute(
            "INSERT INTO agents (name,description,prompt,created_by) VALUES ('dba','old','prompt','arch')",
            [],
        ).unwrap();
        let roster = Roster::load(None, Some(&conn)).unwrap();
        let args = AgentSetArgs { nome: "dba".into(), descricao: Some("new".into()), prompt_file: None, force: false };
        let other = Connection::open(tmp.path().join("comms.db")).unwrap();
        other.busy_timeout(Duration::ZERO).unwrap();
        let mut blocked = false;

        agent_set_with_probe(&mut conn, &roster, &args, &mut Out::new(true), || {
            blocked = other.execute("UPDATE agents SET prompt='concurrent' WHERE name='dba'", []).is_err();
        }).unwrap();

        assert!(blocked, "outra conexão escreveu entre a leitura e o UPDATE de agent-set");
    }
}
