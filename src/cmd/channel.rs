use crate::cli::{IdArgs, PostArgs, SetArgs, TurnsArgs};
use crate::db::{self, Kv, Turn};
use crate::error::{Code, CommsError, Result};
use crate::guard::{handoff_missing, pii_check};
use crate::out::Out;
use crate::policy::BROADCAST;
use crate::roster::Roster;
use rusqlite::{Connection, params, params_from_iter};
use serde_json::json;
use std::io::{IsTerminal, Read};

pub fn post(conn: &Connection, roster: &Roster, args: &PostArgs, out: &mut Out) -> Result<u8> {
    roster.check_sender("--de", &args.de)?;
    roster.check_recipient("--para", &args.para)?;
    let body = match &args.corpo {
        Some(body) => body.clone(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(CommsError::new(Code::Invalid, "ERRO: informe --corpo ou envie o corpo pelo stdin."));
            }
            let mut bytes = Vec::new();
            std::io::stdin().lock().read_to_end(&mut bytes)?;
            String::from_utf8(bytes).map_err(|_| CommsError::new(Code::Invalid, "ERRO: corpo do stdin não é UTF-8."))?
        }
    };
    pii_check(&[Some(&args.titulo), Some(&body), args.refs.as_deref()], args.force)?;
    if args.tipo == "handoff" && !args.force {
        let missing = handoff_missing(&body);
        if !missing.is_empty() {
            return Err(CommsError::new(
                Code::Guard,
                format!("ERRO: handoff sem os campos: {}. O corpo deve nomear 'Objetivo:', 'Critérios:' e 'Fronteiras:' (delegação vaga vira retrabalho). Use --force se for exceção consciente.", missing.join(", ")),
            ));
        }
    }
    conn.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES (?1,?2,?3,?4,?5,?6)",
        params![args.de, args.para, args.tipo, args.titulo, body, args.refs],
    )?;
    let id = conn.last_insert_rowid();
    out.say(format!("turno {id} registrado ({} → {}, {})", args.de, args.para, args.tipo));
    let turn = db::turn_by_id(conn, id)?.ok_or_else(|| CommsError::new(Code::Internal, "ERRO SQLite: turno inserido não encontrado"))?;
    out.set(json!({"turn": turn}));
    Ok(0)
}

pub fn fetch_turns(conn: &Connection, all: bool, para: Option<&str>) -> Result<Vec<Turn>> {
    let mut clauses = Vec::new();
    let mut values: Vec<String> = Vec::new();
    if !all {
        clauses.push("resolved_at IS NULL");
    }
    if let Some(para) = para {
        clauses.push("recipient IN (?, ?)");
        values.push(para.to_string());
        values.push(BROADCAST.to_string());
    }
    let where_clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
    let sql = format!("SELECT * FROM turns {where_clause} ORDER BY id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), Turn::from_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn print_turns(turns: &[Turn], all: bool, out: &mut Out) {
    for turn in turns {
        let mark = if turn.resolved_at.is_none() { "aberto" } else { "resolvido" };
        out.say(format!("#{} [{mark}] {}→{} ({}, {}): {}", turn.id, turn.sender, turn.recipient, turn.kind, turn.created_at, turn.title));
    }
    if turns.is_empty() {
        out.say(format!("(nenhum turno{})", if all { "" } else { " aberto" }));
    }
}

pub fn turns(conn: &Connection, roster: &Roster, args: &TurnsArgs, out: &mut Out) -> Result<u8> {
    if let Some(para) = &args.para {
        roster.check_recipient("--para", para)?;
    }
    let turns = fetch_turns(conn, args.all, args.para.as_deref())?;
    print_turns(&turns, args.all, out);
    out.set(json!({"turns": turns}));
    Ok(0)
}

pub fn show(conn: &Connection, args: &IdArgs, out: &mut Out) -> Result<u8> {
    let turn = db::turn_by_id(conn, args.id)?.ok_or_else(|| CommsError::new(Code::NotFound, format!("turno {} não existe", args.id)))?;
    let status = turn.resolved_at.as_ref().map(|date| format!("resolvido em {date}")).unwrap_or_else(|| "aberto".to_string());
    out.say(format!("#{} {} → {} · {} · {} · {status}", turn.id, turn.sender, turn.recipient, turn.kind, turn.created_at));
    if let Some(refs) = turn.refs.as_deref().filter(|refs| !refs.is_empty()) {
        out.say(format!("refs: {refs}"));
    }
    out.say(format!("\n{}\n\n{}", turn.title, turn.body));
    out.set(json!({"turn": turn}));
    Ok(0)
}

pub fn resolve(conn: &Connection, args: &IdArgs, out: &mut Out) -> Result<u8> {
    let changed = conn.execute(
        "UPDATE turns SET resolved_at = datetime('now','localtime') WHERE id=?1 AND resolved_at IS NULL",
        [args.id],
    )?;
    let turn = db::turn_by_id(conn, args.id)?.ok_or_else(|| CommsError::new(Code::NotFound, format!("turno {} não existe", args.id)))?;
    if changed == 0 {
        out.warn(format!("turno {} já estava resolvido em {}", args.id, turn.resolved_at.as_deref().unwrap_or("desconhecido")));
    } else {
        out.say(format!("turno {} resolvido", args.id));
    }
    out.set(json!({"turn": turn}));
    Ok(0)
}

pub fn set(conn: &Connection, args: &SetArgs, out: &mut Out) -> Result<u8> {
    pii_check(&[Some(&args.value)], args.force)?;
    conn.execute(
        "INSERT INTO kv (key, value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now','localtime')",
        params![args.key, args.value],
    )?;
    out.say(format!("{} = {}", args.key, args.value));
    let kv = conn.query_row("SELECT key, value, updated_at FROM kv WHERE key=?1", [&args.key], Kv::from_row)?;
    out.set(json!({"kv": kv}));
    Ok(0)
}

pub fn state(conn: &Connection, out: &mut Out) -> Result<u8> {
    out.say("== Estado atual ==");
    let mut stmt = conn.prepare("SELECT key, value, updated_at FROM kv ORDER BY key")?;
    let kv = stmt.query_map([], Kv::from_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
    for row in &kv {
        out.say(format!("{}: {}  ({})", row.key, row.value, row.updated_at));
    }
    out.say("\n== Itens pendentes ==");
    let items = super::items::fetch_items(conn, false)?;
    super::items::print_items(&items, false, out);
    out.say("\n== Turnos abertos ==");
    let turns = fetch_turns(conn, false, None)?;
    print_turns(&turns, false, out);
    out.set(json!({"kv": kv, "items": items, "turns": turns}));
    Ok(0)
}
