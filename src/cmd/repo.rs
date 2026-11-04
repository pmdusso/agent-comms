use crate::db;
use crate::error::{Code, CommsError, Result};
use crate::git;
use crate::out::Out;
use crate::paths::Ctx;
use crate::policy::TEST_CMD_HINTS;
use regex::Regex;
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::sync::LazyLock;

pub fn trunk(ctx: &Ctx, out: &mut Out) -> Result<u8> {
    let branch = git::default_branch(ctx.project_root());
    out.say(&branch);
    out.set(json!({"trunk": branch}));
    Ok(0)
}

fn test_command(ctx: &Ctx, conn: Option<&Connection>) -> Result<Option<String>> {
    if let Some(conn) = conn
        && let Some(command) = db::kv_get(conn, "test.cmd")?
        && !command.is_empty()
    {
        return Ok(Some(command));
    }
    static MAKE_TEST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^test\s*:").unwrap());
    let root = ctx.project_root();
    for (file, command) in TEST_CMD_HINTS {
        let path = root.join(file);
        if !path.exists() {
            continue;
        }
        if file == "Makefile" {
            let bytes = fs::read(&path)?;
            let content = String::from_utf8_lossy(&bytes);
            if !MAKE_TEST_RE.is_match(&content) {
                continue;
            }
        }
        if file == "package.json" {
            let Ok(content) = fs::read_to_string(&path) else { continue };
            let Ok(package) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
            if package.get("scripts").and_then(|scripts| scripts.get("test")).is_none() {
                continue;
            }
        }
        return Ok(Some(command.to_string()));
    }
    Ok(None)
}

pub fn testcmd(ctx: &Ctx, conn: Option<&Connection>, out: &mut Out) -> Result<u8> {
    let command = test_command(ctx, conn)?.ok_or_else(|| CommsError::new(
        Code::NotFound,
        "ERRO: não deu para inferir o comando de teste deste repositório.\n  comms set test.cmd \"<comando>\"",
    ))?;
    out.say(&command);
    out.set(json!({"testcmd": command}));
    Ok(0)
}
