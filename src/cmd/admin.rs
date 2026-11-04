use crate::cli::BootstrapArgs;
use crate::db;
use crate::error::Result;
use crate::out::Out;
use crate::paths::Ctx;
use crate::policy::{KINDS, ROLES, SEVERITIES, STATUSES, VERSION};
use crate::roster::{self, Roster};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const AGENT_COMMS_TEMPLATE: &str = include_str!("../../templates/AGENT_COMMS.md");
const PIPELINE_TEMPLATE: &str = include_str!("../../templates/PIPELINE.md");
const INDEXES: [&str; 5] = [
    "idx_turns_recipient", "idx_turns_sender", "idx_turns_resolved",
    "idx_work_items_owner", "idx_work_items_status",
];
const MIGRATE_TABLES: [&str; 2] = ["turns", "work_items"];

pub fn version(ctx: &Ctx, roster: &Roster, out: &mut Out) -> Result<u8> {
    let senders = roster.names();
    let mut recipients = senders.clone();
    recipients.push("todos");
    let mut scopes = senders.clone();
    scopes.push("cross");
    out.say(format!("comms v{VERSION}"));
    out.say(format!("SENDERS: {}", senders.join(", ")));
    out.say(format!("RECIPIENTS: {}", recipients.join(", ")));
    out.say(format!("KINDS: {}", KINDS.join(", ")));
    out.say(format!("WORK_ITEM_SCOPES: {}", scopes.join(", ")));
    out.say(format!("ROLES: executor={} panel={} pr_bot={} advisor={}", ROLES.executor, ROLES.panel.join(","), ROLES.pr_bot, ROLES.advisor));
    let db_line = match &ctx.db_path {
        Some(path) if path.exists() => path.display().to_string(),
        Some(path) => format!("{} (ainda não criado)", path.display()),
        None => "(não encontrado — rode comms bootstrap)".to_string(),
    };
    out.say(format!("DB Path: {db_line}"));
    out.set(json!({
        "version": VERSION,
        "db_path": ctx.db_path.as_ref().map(|path| path.display().to_string()),
        "senders": senders,
        "recipients": recipients,
        "kinds": KINDS,
        "statuses": STATUSES,
        "scopes": scopes,
        "roles": {"executor": ROLES.executor, "panel": ROLES.panel, "pr_bot": ROLES.pr_bot, "advisor": ROLES.advisor},
    }));
    Ok(0)
}

pub fn migrate(conn: &mut Connection, out: &mut Out) -> Result<u8> {
    let frozen = super::agents::frozen_roster_tables(conn)?;
    if frozen.is_empty() {
        out.say("Nada a migrar: o schema já aceita roster dinâmico.");
        out.set(json!({"migrated": [], "rows": {"turns": 0, "work_items": 0}}));
        return Ok(0);
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for index in INDEXES {
        tx.execute(&format!("DROP INDEX IF EXISTS {index}"), [])?;
    }
    for table in MIGRATE_TABLES {
        tx.execute(&format!("ALTER TABLE {table} RENAME TO {table}_old"), [])?;
    }
    for statement in db::schema_sql().split(';').map(str::trim).filter(|statement| !statement.is_empty()) {
        tx.execute(statement, [])?;
    }
    let columns = [
        ("turns", "id, sender, recipient, kind, title, body, refs, created_at, resolved_at"),
        ("work_items", "id, title, owner, scope, status, blocker, refs, updated_at"),
    ];
    let mut turns = 0usize;
    let mut work_items = 0usize;
    for (table, columns) in columns {
        let count = tx.execute(&format!("INSERT INTO {table} ({columns}) SELECT {columns} FROM {table}_old"), [])?;
        if table == "turns" { turns = count; } else { work_items = count; }
    }
    for table in MIGRATE_TABLES {
        tx.execute(&format!("DROP TABLE {table}_old"), [])?;
    }
    tx.commit()?;
    out.say(format!("  turns: {turns} linhas preservadas"));
    out.say(format!("  work_items: {work_items} linhas preservadas"));
    out.say("✓ Roster liberado. Agentes novos entram sem recriar o banco.");
    out.set(json!({"migrated": frozen, "rows": {"turns": turns, "work_items": work_items}}));
    Ok(0)
}

fn write_doc(home: &Path, relative: &str, content: &str, out: &mut Out) -> Result<Value> {
    let path = home.join(relative);
    if path.exists() {
        out.warn(format!("Aviso: '{relative}' já existe. Preservando documento mantido no repositório (sobrescrita por template desativada para proteger a fonte de verdade)."));
        return Ok(json!({"path": relative, "written": false}));
    }
    fs::write(&path, content)?;
    out.say(format!("Arquivo de instruções '{relative}' gerado com sucesso."));
    Ok(json!({"path": relative, "written": true}))
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

pub fn bootstrap(ctx: &Ctx, args: &BootstrapArgs, out: &mut Out) -> Result<u8> {
    let target = ctx.bootstrap_db();
    if args.force && target.exists() {
        for suffix in ["", "-wal", "-shm"] {
            let path = suffixed(&target, suffix);
            if path.exists()
                && let Err(error) = fs::remove_file(&path)
            {
                out.warn(format!("Aviso ao remover {}: {error}", path.display()));
            }
        }
    }
    let conn = db::open_rw(&target)?;
    db::checkpoint(&conn);
    out.say(format!("Banco de dados inicializado com sucesso em '{}' (modo WAL ativado).", target.display()));
    let home = target.parent().unwrap_or(&ctx.cwd);
    fs::create_dir_all(home.join("docs/core"))?;
    let prompts_dir = home.join("docs/prompts");
    let roster = Roster::load(Some(&prompts_dir), Some(&conn))?;
    let senders = roster.names().join(", ");
    let agent_doc = roster::render(AGENT_COMMS_TEMPLATE, &[
        ("SENDERS", &senders), ("KINDS", &KINDS.join(", ")),
        ("STATUSES", &STATUSES.join(", ")), ("VERSION", VERSION),
    ]);
    let panel = ROLES.panel.join(", ");
    let pipeline_doc = roster::render(PIPELINE_TEMPLATE, &[
        ("EXECUTOR", ROLES.executor), ("PANEL", &panel), ("PR_BOT", ROLES.pr_bot),
        ("ADVISOR", ROLES.advisor), ("SEVERITIES", &SEVERITIES.join("/")), ("VERSION", VERSION),
    ]);
    let docs = vec![
        write_doc(home, "docs/core/AGENT_COMMS.md", &agent_doc, out)?,
        write_doc(home, "docs/core/PIPELINE.md", &pipeline_doc, out)?,
    ];

    let gitignore_path = home.join(".gitignore");
    let existing = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let existing_entries: Vec<&str> = existing.lines().map(str::trim).collect();
    let entries = ["comms.db", "comms.db-wal", "comms.db-shm"];
    let to_add: Vec<&str> = entries.into_iter().filter(|entry| !existing_entries.contains(entry)).collect();
    if !to_add.is_empty() {
        let bytes = fs::read(&gitignore_path).unwrap_or_default();
        let prefix = if !bytes.is_empty() && !bytes.ends_with(b"\n") { "\n" } else { "" };
        let mut file = fs::OpenOptions::new().create(true).append(true).open(&gitignore_path)?;
        file.write_all(format!("{}{}\n", prefix, to_add.join("\n")).as_bytes())?;
        out.say(format!("Entradas adicionadas ao .gitignore: {}", to_add.join(", ")));
    } else {
        out.say("Entradas do comms.db já presentes no .gitignore.");
    }
    out.set(json!({"db_path": target.display().to_string(), "docs": docs, "gitignore_added": to_add}));
    Ok(0)
}
