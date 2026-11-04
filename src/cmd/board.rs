use crate::cli::{BoardSetArgs, FinishArgs, IssueArgs, StartArgs};
use crate::error::{Code, CommsError, Result};
use crate::gh::{self, BoardCfg, Card};
use crate::git;
use crate::guard::refs_has_token;
use crate::out::Out;
use crate::paths::Ctx;
use crate::roster::Roster;
use regex::Regex;
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::sync::LazyLock;

pub fn slug_and_branch(issue: u64, title: &str) -> (String, String) {
    static NON_SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9\-]+").unwrap());
    let lowered = title.to_lowercase();
    let raw = NON_SLUG.replace_all(&lowered, "-");
    let raw = raw.trim_matches('-');
    let slug_raw: String = raw.chars().take(30).collect();
    let branch_raw: String = raw.chars().take(25).collect();
    (
        format!("issue-{issue}-{slug_raw}").trim_end_matches('-').to_string(),
        format!("feat/{issue}-{branch_raw}"),
    )
}

fn emit_warnings(out: &mut Out, warnings: Vec<String>) {
    for warning in warnings {
        out.warn(warning);
    }
}

fn board_transition(issue: u64, card: &Card, to: &str, out: &mut Out) -> Result<Value> {
    let from = card.status.clone();
    gh::set_status(card, to)?;
    out.say(format!("✓ Board GitHub atualizado: Issue #{issue} -> {to}"));
    Ok(json!({"item_id": card.item_id, "from": from, "to": to}))
}

pub fn start(conn: &mut Connection, ctx: &Ctx, roster: &Roster, args: &StartArgs, out: &mut Out) -> Result<u8> {
    if let Some(owner) = &args.dono {
        roster.check_sender("--dono", owner)?;
    }
    let owner = args.dono.as_deref().unwrap_or("dev");
    let repo = gh::repo(conn)?;
    let cfg = gh::board_cfg(conn)?;
    let (title, card, warnings) = gh::card_lookup(&cfg, &repo, args.issue)?;
    emit_warnings(out, warnings);
    let (slug, branch) = slug_and_branch(args.issue, &title);
    out.say(format!("=== Iniciando trabalho na Issue #{}: {} ===", args.issue, title));
    let board = match &card {
        Some(card) => Some(board_transition(args.issue, card, "In progress", out)?),
        None => {
            out.warn(format!("Aviso: Issue #{} não encontrada no board {}/{}.", args.issue, cfg.owner, cfg.num));
            None
        }
    };

    let mut worktree_doc = None;
    if !args.no_worktree {
        let root = ctx.project_root();
        let dir = root.join(".worktrees").join(&slug);
        let base = args.base.clone().unwrap_or_else(|| git::default_branch(root));
        let status = if dir.exists() {
            out.say(format!("✓ Git Worktree já existe em: .worktrees/{slug}"));
            "existed"
        } else {
            match git::worktree_add(root, &branch, &dir, &base) {
                Ok(()) => {
                    out.say(format!("✓ Git Worktree criado em: .worktrees/{slug} (branch: {branch}, base: {base})"));
                    "created"
                }
                Err(error) => {
                    out.warn(format!("Aviso ao criar worktree: {error}"));
                    "failed"
                }
            }
        };
        worktree_doc = Some(json!({"path": dir.display().to_string(), "branch": branch, "base": base, "status": status}));
    }

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO work_items (title, owner, scope, status, refs) VALUES (?1, ?2, 'cross', 'aberto', ?3)",
        params![format!("#{}: {}", args.issue, title), owner, format!("issue:{}", args.issue)],
    )?;
    let item_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES (?1, 'todos', 'handoff', ?2, ?3, ?4)",
        params![
            owner,
            format!("delivery In progress #{}", args.issue),
            format!("Iniciando desenvolvimento da issue #{}: {}. Worktree=.worktrees/{}", args.issue, title, slug),
            format!("issue:{}", args.issue),
        ],
    )?;
    let turn_id = tx.last_insert_rowid();
    tx.commit()?;
    out.say(format!("✓ Registro no comms.db concluído (Item #{item_id}, turno de handoff criado)."));
    out.say(format!("\nPronto para implementar! Acesse `.worktrees/{slug}` e execute o plano."));
    out.set(json!({
        "issue": args.issue, "title": title, "board": board, "worktree": worktree_doc,
        "item_id": item_id, "turn_id": turn_id,
    }));
    Ok(0)
}

pub fn finish(conn: &mut Connection, args: &FinishArgs, out: &mut Out) -> Result<u8> {
    let repo = gh::repo(conn)?;
    let cfg = gh::board_cfg(conn)?;
    let (title, card, warnings) = gh::card_lookup(&cfg, &repo, args.issue)?;
    emit_warnings(out, warnings);
    out.say(format!("=== Concluindo entrega da Issue #{}: {} ===", args.issue, title));
    let board = match &card {
        Some(card) => Some(board_transition(args.issue, card, "In review", out)?),
        None => None,
    };

    let pattern = format!("%issue:{}%", args.issue);
    let token = format!("issue:{}", args.issue);
    let mut stmt = conn.prepare("SELECT id, refs FROM work_items WHERE refs LIKE ?1")?;
    let ids: Vec<i64> = stmt.query_map([pattern], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, refs)| refs_has_token(refs.as_deref(), &token))
        .map(|(id, _)| id)
        .collect();
    drop(stmt);

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for id in &ids {
        tx.execute("UPDATE work_items SET status='feito', updated_at=datetime('now','localtime') WHERE id=?1", [id])?;
    }
    tx.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES ('dev', 'reviewer', 'gate', ?1, ?2, ?3)",
        params![
            format!("PR pronto #{}", args.issue),
            format!("PR={}; entrega concluída e aguardando G4/G5 (review + merge do human).", args.pr),
            format!("issue:{} pr:{}", args.issue, args.pr),
        ],
    )?;
    let turn_id = tx.last_insert_rowid();
    tx.commit()?;

    gh::issue_comment(&repo.2, args.issue, &format!("🚀 **Entrega em review!** PR aberto: {}", args.pr))?;
    out.say(format!("✓ Comentário adicionado na Issue #{} no GitHub.", args.issue));
    out.say("✓ Registro no comms.db e transição para In review concluída.");
    out.set(json!({
        "issue": args.issue, "title": title, "pr": args.pr, "board": board,
        "items_done": ids.len(), "turn_id": turn_id, "commented": true,
    }));
    Ok(0)
}

pub fn board_sync(conn: &Connection, out: &mut Out) -> Result<u8> {
    let repo = gh::repo(conn)?;
    let cfg = gh::board_cfg(conn)?;
    let nodes = gh::board_items(&cfg)?;
    let cards = nodes.len();
    out.say(format!("=== Sincronizando {} no board {}/{} ({} cards) com comms.db ===", repo.2, cfg.owner, cfg.num, cards));
    let mut synced = Vec::new();
    for node in nodes {
        let status = node.status().map(str::to_string);
        let Some(content) = node.content else { continue };
        let Some(number) = content.number else { continue };
        let Some(status) = status else { continue };
        let Some(repository) = content.repository else { continue };
        if !repository.name_with_owner.eq_ignore_ascii_case(&repo.2) {
            continue;
        }
        let title = content.title.unwrap_or_default();
        conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now','localtime')",
            params![format!("board.issue.{number}.status"), status],
        )?;
        let short_title: String = title.chars().take(40).collect();
        out.say(format!("  Issue #{number} ({short_title}...): Status Board = {status}"));
        synced.push(json!({"issue": number, "status": status, "title": title}));
    }
    out.say(format!("\n✓ {} itens sincronizados do GitHub Projects para o comms.db KV.", synced.len()));
    out.set(json!({"repo": repo.2, "project": cfg.num, "owner": cfg.owner, "cards": cards, "synced": synced}));
    Ok(0)
}

pub fn board_item(conn: &Connection, args: &IssueArgs, out: &mut Out) -> Result<u8> {
    let repo = gh::repo(conn)?;
    let cfg = gh::board_cfg(conn)?;
    let (_, card, warnings) = gh::card_lookup(&cfg, &repo, args.issue)?;
    emit_warnings(out, warnings);
    let card = card.ok_or_else(|| not_on_board(args.issue, &cfg))?;
    out.say(format!("{}\t{}", card.item_id, card.status.as_deref().unwrap_or("None")));
    out.set(json!({"issue": args.issue, "item_id": card.item_id, "status": card.status}));
    Ok(0)
}

fn not_on_board(issue: u64, cfg: &BoardCfg) -> CommsError {
    CommsError::new(Code::NotFound, format!("ERRO: issue #{issue} não está no board {}/{}. Adicione o card e tente de novo.", cfg.owner, cfg.num))
}

pub fn board_set(conn: &Connection, args: &BoardSetArgs, out: &mut Out) -> Result<u8> {
    let repo = gh::repo(conn)?;
    let cfg = gh::board_cfg(conn)?;
    let (_, card, warnings) = gh::card_lookup(&cfg, &repo, args.issue)?;
    emit_warnings(out, warnings);
    let card = card.ok_or_else(|| CommsError::new(Code::NotFound, format!("ERRO: issue #{} não está no board.", args.issue)))?;
    let old = card.status.clone();
    gh::set_status(&card, &args.status)?;
    out.say(format!("✓ Issue #{}: {} → {}", args.issue, old.as_deref().unwrap_or("None"), args.status));
    out.set(json!({"issue": args.issue, "item_id": card.item_id, "from": old, "to": args.status}));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::slug_and_branch;

    #[test]
    fn slug_and_branch_normalize_unicode_and_boundaries() {
        assert_eq!(
            slug_and_branch(7, "Fix: Login É quebrado!!"),
            ("issue-7-fix-login-quebrado".to_string(), "feat/7-fix-login-quebrado".to_string())
        );
        assert_eq!(
            slug_and_branch(7, &format!("{} b", "a".repeat(29))).0,
            format!("issue-7-{}", "a".repeat(29))
        );
        assert_eq!(
            slug_and_branch(7, &format!("{} b", "a".repeat(24))).1,
            format!("feat/7-{}-", "a".repeat(24))
        );
    }
}
