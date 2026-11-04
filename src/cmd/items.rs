use crate::cli::{ItemAddArgs, ItemSetArgs, ItemsArgs};
use crate::db::{self, WorkItem};
use crate::error::{Code, CommsError, Result};
use crate::guard::pii_check;
use crate::out::Out;
use crate::roster::Roster;
use rusqlite::{Connection, params, params_from_iter};
use serde_json::json;

pub fn fetch_items(conn: &Connection, all: bool) -> Result<Vec<WorkItem>> {
    let where_clause = if all { "" } else { "WHERE status != 'feito'" };
    let mut stmt = conn.prepare(&format!("SELECT * FROM work_items {where_clause} ORDER BY status, id"))?;
    let rows = stmt.query_map([], WorkItem::from_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn print_items(items: &[WorkItem], all: bool, out: &mut Out) {
    for item in items {
        let extra = item.blocker.as_deref().filter(|blocker| !blocker.is_empty()).map(|blocker| format!(" — bloqueio: {blocker}")).unwrap_or_default();
        let refs = item.refs.as_deref().filter(|refs| !refs.is_empty()).map(|refs| format!(" [{refs}]")).unwrap_or_default();
        out.say(format!("#{} [{}] ({}/{}) {}{}{}", item.id, item.status, item.owner, item.scope, item.title, extra, refs));
    }
    if items.is_empty() {
        out.say(format!("(nenhum item{})", if all { "" } else { " pendente" }));
    }
}

pub fn items(conn: &Connection, args: &ItemsArgs, out: &mut Out) -> Result<u8> {
    let items = fetch_items(conn, args.all)?;
    print_items(&items, args.all, out);
    out.set(json!({"items": items}));
    Ok(0)
}

pub fn item_add(conn: &Connection, roster: &Roster, args: &ItemAddArgs, out: &mut Out) -> Result<u8> {
    roster.check_sender("--dono", &args.dono)?;
    roster.check_scope("--escopo", &args.escopo)?;
    pii_check(&[Some(&args.titulo), args.bloqueio.as_deref(), args.refs.as_deref()], args.force)?;
    conn.execute(
        "INSERT INTO work_items (title, owner, scope, status, blocker, refs) VALUES (?1,?2,?3,?4,?5,?6)",
        params![args.titulo, args.dono, args.escopo, args.status, args.bloqueio, args.refs],
    )?;
    let id = conn.last_insert_rowid();
    out.say(format!("item {id} criado"));
    let item = db::item_by_id(conn, id)?.ok_or_else(|| CommsError::new(Code::Internal, "ERRO SQLite: item inserido não encontrado"))?;
    out.set(json!({"item": item}));
    Ok(0)
}

pub fn item_set(conn: &Connection, roster: &Roster, args: &ItemSetArgs, out: &mut Out) -> Result<u8> {
    if let Some(owner) = &args.dono {
        roster.check_sender("--dono", owner)?;
    }
    let fields = [
        ("status", args.status.as_ref()),
        ("blocker", args.bloqueio.as_ref()),
        ("title", args.titulo.as_ref()),
        ("owner", args.dono.as_ref()),
        ("refs", args.refs.as_ref()),
    ];
    let mut sets = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for (column, value) in fields {
        if let Some(value) = value {
            pii_check(&[Some(value)], args.force)?;
            sets.push(format!("{column}=?"));
            values.push(value.clone());
        }
    }
    if sets.is_empty() {
        return Err(CommsError::new(Code::Invalid, "nada para atualizar"));
    }
    values.push(args.id.to_string());
    let changed = conn.execute(
        &format!("UPDATE work_items SET {}, updated_at=datetime('now','localtime') WHERE id=?", sets.join(", ")),
        params_from_iter(values.iter()),
    )?;
    if changed == 0 {
        return Err(CommsError::new(Code::NotFound, format!("item {} não existe", args.id)));
    }
    out.say(format!("item {} atualizado", args.id));
    let item = db::item_by_id(conn, args.id)?.ok_or_else(|| CommsError::new(Code::NotFound, format!("item {} não existe", args.id)))?;
    out.set(json!({"item": item}));
    Ok(0)
}
