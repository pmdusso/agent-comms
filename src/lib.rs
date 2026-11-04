pub mod cli;
pub mod cmd;
pub mod db;
pub mod error;
pub mod guard;
pub mod gh;
pub mod git;
pub mod out;
pub mod paths;
pub mod policy;
pub mod roster;

use clap::Parser;
use cli::{Cli, Cmd};
use error::{Code, CommsError, Result};
use out::Out;
use std::process::ExitCode;

pub fn run() -> ExitCode {
    let json_arg = std::env::args_os().any(|arg| arg == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(error.kind(), clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion) {
                error.exit();
            }
            if json_arg {
                out::emit_error(true, &CommsError::new(Code::Usage, error.to_string()), &[]);
                return ExitCode::from(2);
            }
            error.exit();
        }
    };
    let mut out = Out::new(cli.json);
    match execute(cli.cmd, &mut out) {
        Ok(code) => {
            out.flush();
            ExitCode::from(code)
        }
        Err(error) => {
            if !out.json {
                out.flush();
            }
            out::emit_error(out.json, &error, &out.warnings);
            ExitCode::from(error.exit_code())
        }
    }
}

fn execute(cmd: Cmd, out: &mut Out) -> Result<u8> {
    let ctx = paths::Ctx::discover()?;
    if matches!(cmd, Cmd::Version) {
        let conn = match &ctx.db_path {
            Some(path) if path.exists() => Some(db::open_ro(path)?),
            _ => None,
        };
        let prompts_dir = ctx.prompts_dir();
        let roster = roster::Roster::load(prompts_dir.as_deref(), conn.as_ref())?;
        return cmd::admin::version(&ctx, &roster, out);
    }
    if matches!(cmd, Cmd::Trunk) {
        return cmd::repo::trunk(&ctx, out);
    }
    if matches!(cmd, Cmd::Testcmd) {
        let conn = match &ctx.db_path {
            Some(path) if path.exists() => Some(db::open_ro(path)?),
            _ => None,
        };
        return cmd::repo::testcmd(&ctx, conn.as_ref(), out);
    }
    if let Cmd::Bootstrap(args) = &cmd {
        return cmd::admin::bootstrap(&ctx, args, out);
    }

    let write = matches!(cmd, Cmd::Post(_) | Cmd::Resolve(_) | Cmd::ItemAdd(_) | Cmd::ItemSet(_) | Cmd::Set(_) | Cmd::AgentAdd(_) | Cmd::AgentSet(_) | Cmd::AgentRm(_) | Cmd::Migrate | Cmd::Start(_) | Cmd::Finish(_) | Cmd::BoardSync);
    let mut conn = if write { db::open_rw(ctx.require_db()?)? } else { db::open_ro(ctx.require_db()?)? };
    let prompts_dir = ctx.prompts_dir();
    let roster = roster::Roster::load(prompts_dir.as_deref(), Some(&conn))?;
    let result = match cmd {
        Cmd::Post(args) => cmd::channel::post(&conn, &roster, &args, out),
        Cmd::Turns(args) => cmd::channel::turns(&conn, &roster, &args, out),
        Cmd::Show(args) => cmd::channel::show(&conn, &args, out),
        Cmd::Resolve(args) => cmd::channel::resolve(&conn, &args, out),
        Cmd::Items(args) => cmd::items::items(&conn, &args, out),
        Cmd::ItemAdd(args) => cmd::items::item_add(&conn, &roster, &args, out),
        Cmd::ItemSet(args) => cmd::items::item_set(&conn, &roster, &args, out),
        Cmd::Set(args) => cmd::channel::set(&conn, &args, out),
        Cmd::State => cmd::channel::state(&conn, out),
        Cmd::Agents => cmd::agents::agents(&conn, &roster, prompts_dir.as_deref(), out),
        Cmd::Brief(args) => cmd::agents::brief(&conn, &roster, &args, out),
        Cmd::AgentAdd(args) => cmd::agents::agent_add(&conn, &roster, &args, out),
        Cmd::AgentSet(args) => cmd::agents::agent_set(&conn, &roster, &args, out),
        Cmd::AgentRm(args) => cmd::agents::agent_rm(&conn, &args, out),
        Cmd::AgentShow(args) => cmd::agents::agent_show(&conn, &roster, &args, out),
        Cmd::Start(args) => cmd::board::start(&mut conn, &ctx, &roster, &args, out),
        Cmd::Finish(args) => cmd::board::finish(&mut conn, &args, out),
        Cmd::BoardSync => cmd::board::board_sync(&conn, out),
        Cmd::BoardItem(args) => cmd::board::board_item(&conn, &args, out),
        Cmd::BoardSet(args) => cmd::board::board_set(&conn, &args, out),
        Cmd::Gate(args) => cmd::gate::gate(&conn, &args, out),
        Cmd::Migrate => cmd::admin::migrate(&mut conn, out),
        Cmd::Trunk | Cmd::Testcmd | Cmd::Bootstrap(_) | Cmd::Version => unreachable!(),
    };
    if write {
        db::checkpoint(&conn);
    }
    result
}
