use crate::cli::GateArgs;
use crate::db::WorkItem;
use crate::error::Result;
use crate::guard::refs_has_token;
use crate::out::Out;
use regex::Regex;
use rusqlite::Connection;
use serde_json::json;
use std::sync::LazyLock;

static FINDING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\[(grave|medio|pequeno)\]").unwrap());

pub fn gate(conn: &Connection, args: &GateArgs, out: &mut Out) -> Result<u8> {
    let mut stmt = conn.prepare("SELECT * FROM work_items ORDER BY id")?;
    let items = stmt.query_map([], WorkItem::from_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
    let mut checked = 0usize;
    let mut failures = Vec::new();
    let mut messages = Vec::new();
    let refs_filter = args.refs.as_deref().filter(|refs| !refs.is_empty());
    for item in items {
        if refs_filter.is_some_and(|refs| !refs_has_token(item.refs.as_deref(), refs)) {
            continue;
        }
        let Some(captures) = FINDING_RE.captures(&item.title) else { continue };
        checked += 1;
        let severity = captures[1].to_lowercase();
        let (reason, message) = match item.status.as_str() {
            "aberto" | "bloqueado" => (
                Some("sem_destino"),
                format!("#{} [{severity}] sem destino (status={}): {}", item.id, item.status, item.title),
            ),
            "debito" if severity == "grave" => (
                Some("grave_em_debito"),
                format!("#{} grave como débito — a política proíbe: {}", item.id, item.title),
            ),
            "debito" if severity == "medio" && item.blocker.as_deref().unwrap_or("").trim().is_empty() => (
                Some("medio_sem_justificativa"),
                format!("#{} médio em débito sem justificativa (item-set {} --status debito --bloqueio \"motivo\"): {}", item.id, item.id, item.title),
            ),
            _ => (None, String::new()),
        };
        if let Some(reason) = reason {
            failures.push(json!({"id": item.id, "severity": severity, "status": item.status, "title": item.title, "reason": reason}));
            messages.push(message);
        }
    }
    let scope = refs_filter.map(|refs| format!(" em {refs}")).unwrap_or_default();
    let ok = failures.is_empty();
    if ok {
        out.say(format!("GATE OK — {checked} findings{scope}, todas com destino."));
    } else {
        out.say(format!("GATE REPROVADO — {} de {checked} findings{scope}:", failures.len()));
        for message in messages {
            out.say(format!("  {message}"));
        }
    }
    out.set(json!({"ok": ok, "checked": checked, "refs": refs_filter, "failures": failures}));
    Ok(if ok { 0 } else { 1 })
}
