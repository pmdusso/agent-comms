#!/usr/bin/env python3
"""comms — canal estruturado dos agentes do projeto (SQLite em comms.db).

Uso rápido:
  ./comms.py state                                  # estado atual (kv + itens + turnos abertos)
  ./comms.py post --de arch --para todos --tipo mensagem \\
      --titulo "..." --corpo "..." [--refs "TRAG-POC.md MKT-153"]
  ./comms.py turns [--all]        ./comms.py show N        ./comms.py resolve N
  ./comms.py items [--all]        ./comms.py item-add --titulo "..." --dono tester
  ./comms.py item-set N --status feito [--bloqueio "..."]
  ./comms.py set bola_com "tester"
  ./comms.py bootstrap                              # inicializar repositório (db + manual)
  ./comms.py check                                  # self-test em banco temporário

Regra de ouro LGPD: nada de telefone/CPF/coordenada de cliente aqui (guard abaixo).
"""

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
from pathlib import Path
from string import Template

# ==============================================================================
# CONFIGURAÇÃO DE COMUNICAÇÃO DE AGENTES (SQLITE CHATTER / SYNC)
# ==============================================================================
__version__ = "3.0.0"

# ATENÇÃO: todas as tuplas desta seção viram CHECKs no DDL. Banco existente mantém
# os CHECKs antigos — após mudar tuplas, rode ./comms.py bootstrap --force (zera
# dados). v3.0.0 renomeou o sender "pedro" para "owner": bancos criados em v2.x
# rejeitam "owner" até serem recriados.

# RECOMENDAÇÃO: Sempre defina um agente central de coordenação ("arch") encarregado
# de ler/escrever tarefas, interagir com Jira/Github/PRs, e delegar escopos, e
# adicione os agentes especialistas do seu ecossistema.
# "owner" é o humano dono da decisão final (merge/deploy). Não confundir com a
# coluna work_items.owner (papel responsável por um item) nem com a chave kv
# board.owner (dono do GitHub Project) — três coisas distintas, mesmo nome.
SENDERS = ("arch", "pm", "dev", "tester", "reviewer", "owner")
RECIPIENTS = SENDERS + ("todos",)  # "todos" é broadcast, nunca remetente

KINDS = ("mensagem", "decisao", "bloqueio", "handoff", "pergunta", "gate")
STATUSES = ("aberto", "bloqueado", "feito", "debito")
WORK_ITEM_SCOPES = ("arch", "pm", "dev", "tester", "reviewer", "cross")

# Binding papel→CLI do pipeline de entrega (PIPELINE.md). Troque os CLIs livremente
# para testar modelos diferentes — a política não muda, só o binding.
ROLES = {
    "executor": "claude",  # escreve spec, plano e código
    "panel": (
        "gemini",
        "codex",
        "claude",
        "minimax",
        "qwen",
    ),  # ai-review nos gates 1-3
    "pr_bot": "auto",  # gate 4: babysit detecta o bot do repo
    "advisor": "claude",  # taste pass opcional pré-merge
}
SEVERITIES = ("grave", "medio", "pequeno")  # prefixo [sev] no título do work_item

# ponytail: heurística simples — sequência de 10+ dígitos pega E.164/CPF; --force é a escape hatch
PII_RE = re.compile(r"\+?\d{10,}")


def resolve_db_path() -> str:
    """Resolve o caminho absoluto do comms.db principal do repositório.

    Procura a raiz do repositório git principal executando `git rev-parse --git-common-dir`
    ancorado no diretório do próprio script (`cwd=script_dir`).

    Funciona a partir da raiz, de subdiretórios, de git worktrees e de chamadas feitas a
    partir de outros repositórios git ou diretórios arbitrários, garantindo que todos os
    agentes interajam com o mesmo banco principal do repositório.

    Fallback: se git não estiver disponível ou falhar, usa o diretório do script e emite aviso no stderr.
    COMMS_DB: se a variável de ambiente estiver definida, tem precedência total.
    """
    env_override = os.environ.get("COMMS_DB")
    if env_override:
        return env_override
    script_dir = Path(__file__).resolve().parent
    try:
        res = subprocess.run(
            ["git", "rev-parse", "--git-common-dir"],
            cwd=script_dir,
            capture_output=True,
            text=True,
            check=True,
        )
        common_dir = res.stdout.strip()
        if common_dir:
            # (script_dir / common_dir) converte caminho relativo (.git) ancorando em script_dir,
            # mantendo caminho absoluto caso common_dir já seja absoluto.
            repo_root = (script_dir / common_dir).resolve().parent
            return str(repo_root / "comms.db")
    except (subprocess.CalledProcessError, FileNotFoundError, OSError):
        print(
            "AVISO: git indisponível ou falhou, usando o diretório do script para o comms.db",
            file=sys.stderr,
        )
    return str(script_dir / "comms.db")


DB_PATH = resolve_db_path()


def _sql_in(values):
    if not values:
        raise ValueError("Valores para cláusula IN SQL não podem ser vazios.")
    for v in values:
        if not re.match(r"^[a-zA-Z0-9_\-]+$", v):
            raise ValueError(f"Identificador inválido configurado: {v}")
    return "(" + ", ".join(f"'{v}'" for v in values) + ")"


SCHEMA = f"""
CREATE TABLE IF NOT EXISTS turns (
  id INTEGER PRIMARY KEY,
  sender TEXT NOT NULL CHECK (sender IN {_sql_in(SENDERS)}),
  recipient TEXT NOT NULL CHECK (recipient IN {_sql_in(RECIPIENTS)}),
  kind TEXT NOT NULL CHECK (kind IN {_sql_in(KINDS)}),
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  refs TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  resolved_at TEXT
);
CREATE TABLE IF NOT EXISTS work_items (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  owner TEXT NOT NULL CHECK (owner IN {_sql_in(SENDERS)}),
  scope TEXT NOT NULL CHECK (scope IN {_sql_in(WORK_ITEM_SCOPES)}),
  status TEXT NOT NULL DEFAULT 'aberto' CHECK (status IN {_sql_in(STATUSES)}),
  blocker TEXT,
  refs TEXT,
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_turns_recipient ON turns(recipient);
CREATE INDEX IF NOT EXISTS idx_turns_sender ON turns(sender);
CREATE INDEX IF NOT EXISTS idx_turns_resolved ON turns(resolved_at);
CREATE INDEX IF NOT EXISTS idx_work_items_owner ON work_items(owner);
CREATE INDEX IF NOT EXISTS idx_work_items_status ON work_items(status);
"""

AGENT_COMMS_TEMPLATE = """# AGENT_COMMS — Canal de Comunicação Estruturado (v$VERSION)

Este arquivo serve como manual técnico de instrução para os agentes operarem no canal de comunicação do repositório.

## Como Funciona
As reuniões estruturadas, pendências e chaves de estado de progresso deste repositório vivem em `comms.db` (SQLite executado sob PRAGMA WAL).

Sempre que um agente ou humano iniciar uma sessão ou turno neste repositório, ele **deve**:
1. Rodar `./comms.py state` para sincronizar e carregar todo o contexto local.
2. Ler turnos abertos usando `./comms.py turns`.
3. Responder as solicitações e fechar turnos respondidos com `./comms.py resolve <ID>`.

---

## Estrutura do Canal
* **Remetentes Permitidos (Senders):** $SENDERS
* **Destinatários Permitidos (Recipients):** $RECIPIENTS
* **Tipos de Turnos (Kinds):** $KINDS
* **Status de Tarefas (Statuses):** $STATUSES
* **Escopos de Trabalho (Scopes):** $SCOPES

## Responsabilidades

| Papel | Responsabilidade principal |
|---|---|
| `arch` | arquitetura, priorização, decisões e integração |
| `pm` | discovery, brainstorms, roadmap, backlog e critérios de aceite |
| `dev` | implementação e documentação técnica |
| `tester` | estratégia de testes, validação executável e aceite |
| `reviewer` | revisão independente de correção, segurança e simplicidade |
| `owner` | direção de produto e decisões humanas finais |

---

## Uso Rápido CLI
```bash
./comms.py state                             # Ver estado completo consolidado
./comms.py post --de <REMETENTE> --para <DESTINATARIO> --tipo <TIPO> --titulo "..." --corpo "..."
./comms.py turns                             # Listar turnos pendentes
./comms.py show <ID>                         # Ver corpo do turno completo
./comms.py resolve <ID>                      # Marcar turno resolvido
./comms.py items                             # Listar itens de trabalho
./comms.py item-add --titulo "..." --dono <RESPONSAVEL>
```

> **Regra de ouro:** auto-contido. Um agente novo, sem memória, entende o estado atual com `./comms.py state` e este manual.
> **LGPD/PII:** nunca registrar telefone, CPF, coordenada bruta de cliente ou qualquer PII. Referenciar por IDs lógicos. O `comms.py` possui guarda automático que rejeita sequências de 10+ dígitos.
"""

PIPELINE_TEMPLATE = """# PIPELINE — Política de Entrega por Gates (v$VERSION)

Política para grandes atividades deste repositório. Padrão invariante por fase:
**artefato → gate → registro no comms.db**. Modelos nunca aparecem na política —
apenas papéis; o binding papel→CLI vive no topo do `comms.py` (dict `ROLES`).

## Binding atual de papéis

| Papel    | CLI | Responsabilidade |
|----------|-----|------------------|
| executor | $EXECUTOR | escreve spec, plano e código |
| panel    | $PANEL | revisão multi-modelo (ai-review) nos gates 1–3 |
| pr_bot   | $PR_BOT | review automatizado do PR (gate 4) |
| advisor  | $ADVISOR | taste pass opcional pré-merge |

No início de cada rodada, registre o binding para o experimento ser analisável:

    ./comms.py set pipe.<slug>.binding "executor=$EXECUTOR;panel=$PANEL"

## Board (GitHub Projects)

O `comms.py` descobre project-id, field-id e option-ids pela API do `gh` — nada
hardcoded. Só o número/dono do board são configuráveis, e só se fugirem do default:

    ./comms.py set board.project 1      # default: 1
    ./comms.py set board.owner @me      # default: @me
    ./comms.py set board.repo dono/repo # default: repo do diretório atual

    ./comms.py board-item <N>           # item-id + coluna atual do card
    ./comms.py board-set <N> Ready      # move o card de coluna

| Transição | Skill |
|---|---|
| Backlog → Ready | `discovery` |
| Ready → In review (PR) | `delivery` |
| Merge / deploy | **humano** (G5) — agente não mergeia sozinho |

## Fases

| # | Fase | Ferramenta → Artefato | Gate | Registro no comms.db |
|---|------|-----------------------|------|----------------------|
| F0.5 | Discovery | skill `discovery` → spec + plano publicados (+ reviews) | G0: exit checklist Ready no board | turno `gate`: "discovery Ready #N" |
| F0 | Setup | worktree + feature branch | — | `set pipe.<slug>.phase spec` |
| F1 | Spec | brainstorm → `docs/specs/<slug>-spec.md` | G1: ai-review no spec | turno `gate`: "spec aprovado" |
| F2 | Plano | write-plan → `docs/plans/<slug>-plan.md` | G2: ai-review no plano | turno `gate`: "plano aprovado" |
| F3 | Execução | execute-plan → diff na worktree | G3: review-pr local | 1 work_item por finding: `[$SEVERITIES] título` |
| F4 | Saneamento | corrigir ou registrar débito | zero findings `aberto` | `item-set N --status feito` ou `--status debito` |
| F5 | PR | `gh pr create` | G4: review humano + bot | `set pipe.<slug>.pr <n>`; turnos do PR resolvidos |
| F6 | Merge | decisão humana — nunca do agente | G5: aprovação humana | `set pipe.<slug>.phase done` |

Se a discovery rodou, F0.5 já executou F1+F2 com reviews — não repetir G1/G2 do
zero; só reabrir se o escopo mudou.

## Critérios de saída (binários — nunca "parece bom")

- **G0 (Ready):** artefatos publicados e linkados no body; Assumptions não sensíveis
  preenchidas ou "nenhuma"; graves incorporados; comms + docs + board reconciliados.
- **G1/G2:** painel rodou; findings graves e médios incorporados ao artefato antes de avançar.
- **G3:** suíte de testes do repositório verde localmente, **e** toda finding com destino.
- **G4:** required checks do PR verdes, **e** toda finding com destino.
- **G5:** somente humano decide merge.

## Política de débito

Nenhuma finding morre em silêncio. Cada uma sai do gate por exatamente uma porta:

1. **Corrigida** → commit atômico + `item-set N --status feito`.
2. **Débito deliberado** → `item-set N --status debito` (+ comentário `ponytail:` no código quando aplicável).

Regras por severidade: **grave** nunca vira débito; **medio** só com justificativa
registrada (`item-set N --status debito --bloqueio "motivo"`); **pequeno** pode.

Verificação do gate F4: `./comms.py items` sem nenhuma finding em `aberto`.
"""


def _write_doc(path, content, force=False):
    if os.path.exists(path):
        print(
            f"Aviso: '{path}' já existe. Preservando documento mantido no repositório (sobrescrita por template desativada para proteger a fonte de verdade)."
        )
        return
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)
    print(f"Arquivo de instruções '{path}' gerado com sucesso.")


def connect(path=None, read_only=False):
    db_path = path or DB_PATH
    if read_only:
        if not os.path.exists(db_path):
            sys.exit(
                f"ERRO: Banco de dados '{db_path}' não existe. Por favor, execute './comms.py bootstrap' primeiro."
            )
        db = sqlite3.connect(db_path, timeout=30.0)
    else:
        db = sqlite3.connect(db_path, timeout=30.0)
        db.execute("PRAGMA journal_mode=WAL")
        db.execute("PRAGMA wal_autocheckpoint=100")
        db.executescript(SCHEMA)
    db.row_factory = sqlite3.Row
    return db


def guard_pii(*texts, force=False):
    if force:
        return
    for t in texts:
        if t and PII_RE.search(t):
            sys.exit(
                "ERRO: possível PII (sequência de 10+ dígitos). Referencie por ID lógico. Use --force se for falso positivo."
            )


def cmd_post(db, a):
    body = a.corpo if a.corpo is not None else sys.stdin.read()
    guard_pii(a.titulo, body, a.refs, force=a.force)
    cur = db.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES (?,?,?,?,?,?)",
        (a.de, a.para, a.tipo, a.titulo, body, a.refs),
    )
    db.commit()
    print(f"turno {cur.lastrowid} registrado ({a.de} → {a.para}, {a.tipo})")


def cmd_turns(db, a):
    where = "" if a.all else "WHERE resolved_at IS NULL"
    rows = db.execute(f"SELECT * FROM turns {where} ORDER BY id DESC").fetchall()
    for r in rows:
        mark = "aberto" if r["resolved_at"] is None else "resolvido"
        print(
            f"#{r['id']} [{mark}] {r['sender']}→{r['recipient']} ({r['kind']}, {r['created_at']}): {r['title']}"
        )
    if not rows:
        print("(nenhum turno" + ("" if a.all else " aberto") + ")")


def cmd_show(db, a):
    r = db.execute("SELECT * FROM turns WHERE id=?", (a.id,)).fetchone()
    if not r:
        sys.exit(f"turno {a.id} não existe")
    status = (
        "aberto" if r["resolved_at"] is None else f"resolvido em {r['resolved_at']}"
    )
    print(
        f"#{r['id']} {r['sender']} → {r['recipient']} · {r['kind']} · {r['created_at']} · {status}"
    )
    if r["refs"]:
        print(f"refs: {r['refs']}")
    print(f"\n{r['title']}\n\n{r['body']}")


def cmd_resolve(db, a):
    n = db.execute(
        "UPDATE turns SET resolved_at = datetime('now','localtime') WHERE id=? AND resolved_at IS NULL",
        (a.id,),
    ).rowcount
    db.commit()
    print(
        f"turno {a.id} resolvido" if n else f"turno {a.id} não existe ou já resolvido"
    )


def cmd_items(db, a):
    where = "" if a.all else "WHERE status != 'feito'"
    rows = db.execute(
        f"SELECT * FROM work_items {where} ORDER BY status, id"
    ).fetchall()
    for r in rows:
        extra = f" — bloqueio: {r['blocker']}" if r["blocker"] else ""
        refs = f" [{r['refs']}]" if r["refs"] else ""
        print(
            f"#{r['id']} [{r['status']}] ({r['owner']}/{r['scope']}) {r['title']}{extra}{refs}"
        )
    if not rows:
        print("(nenhum item" + ("" if a.all else " pendente") + ")")


def cmd_item_add(db, a):
    guard_pii(a.titulo, a.bloqueio, a.refs, force=a.force)
    cur = db.execute(
        "INSERT INTO work_items (title, owner, scope, status, blocker, refs) VALUES (?,?,?,?,?,?)",
        (a.titulo, a.dono, a.escopo, a.status, a.bloqueio, a.refs),
    )
    db.commit()
    print(f"item {cur.lastrowid} criado")


def cmd_item_set(db, a):
    sets, vals = [], []
    for col, val in (
        ("status", a.status),
        ("blocker", a.bloqueio),
        ("title", a.titulo),
        ("owner", a.dono),
        ("refs", a.refs),
    ):
        if val is not None:
            guard_pii(val, force=a.force)
            sets.append(f"{col}=?")
            vals.append(val)
    if not sets:
        sys.exit("nada para atualizar")
    n = db.execute(
        f"UPDATE work_items SET {', '.join(sets)}, updated_at=datetime('now','localtime') WHERE id=?",
        (*vals, a.id),
    ).rowcount
    db.commit()
    print(f"item {a.id} atualizado" if n else f"item {a.id} não existe")


def cmd_set(db, a):
    guard_pii(a.value, force=a.force)
    db.execute(
        "INSERT INTO kv (key, value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now','localtime')",
        (a.key, a.value),
    )
    db.commit()
    print(f"{a.key} = {a.value}")


def cmd_state(db, _a):
    print("== Estado atual ==")
    for r in db.execute("SELECT key, value, updated_at FROM kv ORDER BY key"):
        print(f"{r['key']}: {r['value']}  ({r['updated_at']})")
    print("\n== Itens pendentes ==")
    cmd_items(db, argparse.Namespace(all=False))
    print("\n== Turnos abertos ==")
    cmd_turns(db, argparse.Namespace(all=False))


# ==============================================================================
# INTEGRACAO GITHUB PROJECTS BOARD & WORKTREES (ORQUESTRAÇÃO V3)
# ==============================================================================
# Nada de ID hardcoded: repo vem de `gh repo view`, project-id/field-id/option-ids
# vêm de `gh project view` + `gh project field-list`. Só o número e o dono do
# project são configuráveis, via kv do próprio comms.db:
#   ./comms.py set board.project 1
#   ./comms.py set board.owner @me
#   ./comms.py set board.repo dono/repo     # opcional; default = repo do cwd


def _gh_run(cmd_args, check=True):
    res = subprocess.run(["gh"] + cmd_args, capture_output=True, text=True)
    if check and res.returncode != 0:
        sys.exit(f"ERRO gh CLI: {res.stderr.strip() or res.stdout.strip()}")
    return res.stdout.strip()


def _kv_get(db, key, default=None):
    r = db.execute("SELECT value FROM kv WHERE key=?", (key,)).fetchone()
    return r["value"] if r else default


def _repo(db):
    """Repo alvo no formato dono/nome — kv `board.repo` ou o repo do diretório atual."""
    return _kv_get(db, "board.repo") or _gh_run(
        ["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"]
    )


_BOARD_CACHE = {}


def _board(db):
    """Resolve project-id, field-id do Status e os option-ids pela API do gh.

    ponytail: cache de processo, não de disco — cada invocação do CLI é um processo
    novo, então 2 chamadas gh por comando. Se virar gargalo, cachear no kv.
    """
    if _BOARD_CACHE:
        return _BOARD_CACHE
    num = _kv_get(db, "board.project", "1")
    owner = _kv_get(db, "board.owner", "@me")
    proj = json.loads(
        _gh_run(["project", "view", num, "--owner", owner, "--format", "json"])
    )
    fields = json.loads(
        _gh_run(
            ["project", "field-list", num, "--owner", owner, "--limit", "100", "--format", "json"]
        )
    ).get("fields", [])
    status = next((f for f in fields if f.get("name") == "Status"), None)
    if not status:
        sys.exit(
            f"ERRO: project {owner}/{num} não tem campo 'Status'. "
            f"Campos encontrados: {[f.get('name') for f in fields]}"
        )
    _BOARD_CACHE.update(
        num=num,
        owner=owner,
        project_id=proj["id"],
        field_id=status["id"],
        options={o["name"]: o["id"] for o in status.get("options", [])},
    )
    return _BOARD_CACHE


def gh_get_issue_info(db, issue_num):
    out = _gh_run(
        [
            "issue",
            "view",
            str(issue_num),
            "-R",
            _repo(db),
            "--json",
            "number,title,body,labels,url",
        ]
    )
    return json.loads(out)


def gh_get_project_item_id(db, issue_num):
    board = _board(db)
    repo = _repo(db)
    out = _gh_run(
        [
            "project",
            "item-list",
            board["num"],
            "--owner",
            board["owner"],
            "--limit",
            "100",
            "--format",
            "json",
        ]
    )
    data = json.loads(out)
    for item in data.get("items", []):
        content = item.get("content", {})
        if content.get("repository") == repo and content.get("number") == int(
            issue_num
        ):
            return item.get("id"), item.get("status")
    return None, None


def gh_set_project_status(db, item_id, status_name):
    board = _board(db)
    opt_id = board["options"].get(status_name)
    if not opt_id:
        sys.exit(
            f"ERRO: status '{status_name}' não existe neste board. "
            f"Disponíveis: {', '.join(board['options'])}"
        )
    _gh_run(
        [
            "project",
            "item-edit",
            "--project-id",
            board["project_id"],
            "--id",
            item_id,
            "--field-id",
            board["field_id"],
            "--single-select-option-id",
            opt_id,
        ]
    )


def cmd_start(db, a):
    issue_num = a.issue
    issue = gh_get_issue_info(db, issue_num)
    title = issue["title"]

    raw_slug = re.sub(r"[^a-zA-Z0-9\-]+", "-", title.lower()).strip("-")
    slug = f"issue-{issue_num}-{raw_slug[:30]}".rstrip("-")

    print(f"=== Iniciando trabalho na Issue #{issue_num}: {title} ===")

    # 1. Update Board status to "In progress"
    item_id, current_status = gh_get_project_item_id(db, issue_num)
    if item_id:
        gh_set_project_status(db, item_id, "In progress")
        print(f"✓ Board GitHub atualizado: Issue #{issue_num} -> In progress")
    else:
        board = _board(db)
        print(
            f"Aviso: Issue #{issue_num} não encontrada no board {board['owner']}/{board['num']}."
        )

    # 2. Create git worktree
    if not a.no_worktree:
        repo_root = Path(DB_PATH).parent
        wt_dir = repo_root / ".worktrees" / slug
        if not wt_dir.exists():
            branch_name = f"feat/{issue_num}-{raw_slug[:25]}"
            res = subprocess.run(
                ["git", "worktree", "add", "-b", branch_name, str(wt_dir), "main"],
                cwd=repo_root,
                capture_output=True,
                text=True,
            )
            if res.returncode == 0:
                print(
                    f"✓ Git Worktree criado em: .worktrees/{slug} (branch: {branch_name})"
                )
            else:
                print(f"Aviso ao criar worktree: {res.stderr.strip()}")
        else:
            print(f"✓ Git Worktree já existe em: .worktrees/{slug}")

    # 3. Create work item in comms.db
    cur = db.execute(
        "INSERT INTO work_items (title, owner, scope, status, refs) VALUES (?, ?, 'cross', 'aberto', ?)",
        (f"#{issue_num}: {title}", a.dono or "dev", f"issue:{issue_num}"),
    )
    item_db_id = cur.lastrowid

    # 4. Post handoff turn in comms.db
    db.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES (?, 'todos', 'handoff', ?, ?, ?)",
        (
            a.dono or "dev",
            f"delivery In progress #{issue_num}",
            f"Iniciando desenvolvimento da issue #{issue_num}: {title}. Worktree=.worktrees/{slug}",
            f"issue:{issue_num}",
        ),
    )
    db.commit()

    print(
        f"✓ Registro no comms.db concluído (Item #{item_db_id}, turno de handoff criado)."
    )
    print(
        f"\nPronto para implementar! Acesse `.worktrees/{slug}` e execute o plano."
    )


def cmd_finish(db, a):
    issue_num = a.issue
    pr_url = a.pr

    issue = gh_get_issue_info(db, issue_num)
    title = issue["title"]

    print(f"=== Concluindo entrega da Issue #{issue_num}: {title} ===")

    # 1. Board -> In review
    item_id, _ = gh_get_project_item_id(db, issue_num)
    if item_id:
        gh_set_project_status(db, item_id, "In review")
        print(f"✓ Board GitHub atualizado: Issue #{issue_num} -> In review")

    # 2. Update work items in comms.db
    db.execute(
        "UPDATE work_items SET status='feito', updated_at=datetime('now','localtime') WHERE refs LIKE ?",
        (f"%issue:{issue_num}%",),
    )

    # 3. Post gate turn in comms.db
    db.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES ('dev', 'reviewer', 'gate', ?, ?, ?)",
        (
            f"PR pronto #{issue_num}",
            f"PR={pr_url}; entrega concluída e aguardando G4/G5 (review + merge do owner).",
            f"issue:{issue_num} pr:{pr_url}",
        ),
    )
    db.commit()

    # 4. Comment on GitHub Issue
    _gh_run(
        [
            "issue",
            "comment",
            str(issue_num),
            "-R",
            _repo(db),
            "-b",
            f"🚀 **Entrega em review!** PR aberto: {pr_url}",
        ]
    )
    print(f"✓ Comentário adicionado na Issue #{issue_num} no GitHub.")
    print(f"✓ Registro no comms.db e transição para In review concluída.")


def cmd_board_sync(db, _a):
    board = _board(db)
    repo = _repo(db)
    out = _gh_run(
        [
            "project",
            "item-list",
            board["num"],
            "--owner",
            board["owner"],
            "--limit",
            "100",
            "--format",
            "json",
        ]
    )
    data = json.loads(out)
    items = data.get("items", [])
    print(
        f"=== Sincronizando {repo} no board {board['owner']}/{board['num']} "
        f"({len(items)} cards) com comms.db ==="
    )
    synced = 0
    for item in items:
        content = item.get("content", {})
        if content.get("repository") == repo:
            issue_num = content.get("number")
            title = content.get("title")
            status = item.get("status")
            if issue_num and status:
                db.execute(
                    "INSERT INTO kv (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now','localtime')",
                    (f"board.issue.{issue_num}.status", status),
                )
                synced += 1
                print(
                    f"  Issue #{issue_num} ({title[:40]}...): Status Board = {status}"
                )
    db.commit()
    print(
        f"\n✓ {synced} itens sincronizados do GitHub Projects para o comms.db KV."
    )


def cmd_board_item(db, a):
    """Localiza o card de uma issue no board. Usado pelas skills no lugar de gh cru."""
    item_id, status = gh_get_project_item_id(db, a.issue)
    if not item_id:
        board = _board(db)
        sys.exit(
            f"ERRO: issue #{a.issue} não está no board {board['owner']}/{board['num']}. "
            f"Adicione o card e tente de novo."
        )
    print(f"{item_id}\t{status}")


def cmd_board_set(db, a):
    """Move o card de uma issue para outra coluna do board."""
    item_id, old = gh_get_project_item_id(db, a.issue)
    if not item_id:
        sys.exit(f"ERRO: issue #{a.issue} não está no board.")
    gh_set_project_status(db, item_id, a.status)
    print(f"✓ Issue #{a.issue}: {old} → {a.status}")


def cmd_bootstrap(a):
    db_path = DB_PATH
    if a.force and os.path.exists(db_path):
        for ext in ("", "-wal", "-shm"):
            p = db_path + ext
            if os.path.exists(p):
                try:
                    os.remove(p)
                except Exception as e:
                    print(f"Aviso ao remover {p}: {e}")

    db = connect(read_only=False)
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.close()
    print(f"Banco de dados inicializado com sucesso em '{db_path}' (modo WAL ativado).")

    os.makedirs("docs/core", exist_ok=True)

    _write_doc(
        "docs/core/AGENT_COMMS.md",
        Template(AGENT_COMMS_TEMPLATE).substitute(
            SENDERS=", ".join(SENDERS),
            RECIPIENTS=", ".join(RECIPIENTS),
            KINDS=", ".join(KINDS),
            STATUSES=", ".join(STATUSES),
            SCOPES=", ".join(WORK_ITEM_SCOPES),
            VERSION=__version__,
        ),
        a.force,
    )

    _write_doc(
        "docs/core/PIPELINE.md",
        Template(PIPELINE_TEMPLATE).substitute(
            EXECUTOR=ROLES["executor"],
            PANEL=", ".join(ROLES["panel"]),
            PR_BOT=ROLES["pr_bot"],
            ADVISOR=ROLES["advisor"],
            SEVERITIES="/".join(SEVERITIES),
            VERSION=__version__,
        ),
        a.force,
    )

    gitignore_path = ".gitignore"
    entries = ["comms.db", "comms.db-wal", "comms.db-shm"]
    existing_entries = set()
    if os.path.exists(gitignore_path):
        with open(gitignore_path, "r", encoding="utf-8") as f:
            existing_entries = {line.strip() for line in f}

    to_add = [e for e in entries if e not in existing_entries]
    if to_add:
        # Garante nova linha final se arquivo existente não terminar com uma
        append_prefix = ""
        if os.path.exists(gitignore_path) and os.path.getsize(gitignore_path) > 0:
            with open(gitignore_path, "rb") as f:
                f.seek(-1, os.SEEK_END)
                if f.read(1) != b"\n":
                    append_prefix = "\n"
        with open(gitignore_path, "a", encoding="utf-8") as f:
            f.write(append_prefix + "\n".join(to_add) + "\n")
        print(f"Entradas adicionadas ao .gitignore: {', '.join(to_add)}")
    else:
        print("Entradas do comms.db já presentes no .gitignore.")


def cmd_version(db, a):
    print(f"comms.py v{__version__}")
    print(f"SENDERS: {', '.join(SENDERS)}")
    print(f"RECIPIENTS: {', '.join(RECIPIENTS)}")
    print(f"KINDS: {', '.join(KINDS)}")
    print(f"WORK_ITEM_SCOPES: {', '.join(WORK_ITEM_SCOPES)}")
    print(
        f"ROLES: executor={ROLES['executor']} panel={','.join(ROLES['panel'])} pr_bot={ROLES['pr_bot']} advisor={ROLES['advisor']}"
    )
    print(f"DB Path: {DB_PATH}")


def check():
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        # 1. Test DDL schemas and checks
        db_path = os.path.join(tmp, "test.db")
        db = connect(db_path, read_only=False)

        db.execute(
            "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('arch', 'todos', 'mensagem', 't', 'b')"
        )
        db.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('x', 'dev', 'dev', 'aberto')"
        )
        db.execute(
            "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('pm', 'arch', 'mensagem', 'pm online', 'b')"
        )
        db.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('roadmap', 'pm', 'pm', 'aberto')"
        )
        db.commit()

        assert (
            db.execute("SELECT recipient FROM turns WHERE id=1").fetchone()[0]
            == "todos"
        )
        assert (
            db.execute("SELECT status FROM work_items WHERE id=1").fetchone()[0]
            == "aberto"
        )

        # Convenções do pipeline: kind 'gate' e status 'debito' aceitos
        db.execute(
            "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('arch', 'todos', 'gate', 'spec aprovado', 'G1 ok')"
        )
        db.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('[pequeno] finding x', 'reviewer', 'reviewer', 'debito')"
        )
        db.commit()

        try:
            db.execute(
                "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('invalid_sender', 'todos', 'mensagem', 't', 'b')"
            )
            raise AssertionError("CHECK de sender inválido não disparou")
        except sqlite3.IntegrityError:
            pass

        try:
            db.execute(
                "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('arch', 'invalid_recipient', 'mensagem', 't', 'b')"
            )
            raise AssertionError("CHECK de recipient inválido não disparou")
        except sqlite3.IntegrityError:
            pass

        try:
            db.execute(
                "INSERT INTO work_items (title, owner, scope, status) VALUES ('y', 'dev', 'dev', 'status_invalido')"
            )
            raise AssertionError("CHECK de status inválido não disparou")
        except sqlite3.IntegrityError:
            pass

        try:
            db.execute(
                "INSERT INTO work_items (title, owner, scope, status) VALUES ('z', 'dev', 'escopo_invalido', 'aberto')"
            )
            raise AssertionError("CHECK de escopo inválido não disparou")
        except sqlite3.IntegrityError:
            pass

        db.close()

        # 1b. Config do board vem do kv, com default — nenhum ID de projeto no código
        db2 = connect(os.path.join(tmp, "kv.db"), read_only=False)
        assert _kv_get(db2, "board.project", "1") == "1"
        assert _kv_get(db2, "board.repo") is None
        db2.execute("INSERT INTO kv (key, value) VALUES ('board.project', '7')")
        assert _kv_get(db2, "board.project", "1") == "7"
        db2.close()

        # Agulhas montadas por concatenação: escritas inteiras, elas se encontrariam
        # a si mesmas no source e o teste falharia sempre.
        src = Path(__file__).read_text(encoding="utf-8")
        for hardcoded in ("gear" + "pal", "PVT" + "_kwHO", "PVTSS" + "F_"):
            assert hardcoded not in src, f"ID hardcoded voltou ao comms.py: {hardcoded}"

        # 2. Testar regex PII guard
        assert PII_RE.search("+5551999998888")
        assert PII_RE.search("32165498700")  # CPF sequence
        assert not PII_RE.search("activity_id 12345 MVP-153")

        # 3. Testar bootstrap mockado
        global DB_PATH
        orig_db_path = DB_PATH
        test_dir = os.path.join(tmp, "bootstrap_test")
        os.makedirs(test_dir)

        DB_PATH = os.path.join(test_dir, "comms.db")

        orig_cwd = os.getcwd()
        os.chdir(test_dir)
        try:

            class MockArgs:
                force = False

            cmd_bootstrap(MockArgs())

            assert os.path.exists("comms.db")
            assert os.path.exists("docs/core/AGENT_COMMS.md")
            assert os.path.exists("docs/core/PIPELINE.md")
            assert os.path.exists(".gitignore")

            with open("docs/core/PIPELINE.md", "r", encoding="utf-8") as f:
                pipe_content = f.read()
                assert ROLES["executor"] in pipe_content
                assert ", ".join(ROLES["panel"]) in pipe_content

            with open("docs/core/AGENT_COMMS.md", "r", encoding="utf-8") as f:
                md_content = f.read()
                assert "arch, pm, dev, tester, reviewer, owner" in md_content
                assert "todos" in md_content

            with open(".gitignore", "r", encoding="utf-8") as f:
                git_content = f.read()
                assert "comms.db" in git_content
                assert "comms.db-wal" in git_content
                assert "comms.db-shm" in git_content

            with open("docs/core/AGENT_COMMS.md", "w", encoding="utf-8") as f:
                f.write("modificado")
            cmd_bootstrap(MockArgs())
            with open("docs/core/AGENT_COMMS.md", "r", encoding="utf-8") as f:
                assert f.read() == "modificado"

            class MockArgsForce:
                force = True

            cmd_bootstrap(MockArgsForce())
            with open("docs/core/AGENT_COMMS.md", "r", encoding="utf-8") as f:
                assert f.read() == "modificado"

        finally:
            os.chdir(orig_cwd)
            DB_PATH = orig_db_path

        # 4. Testar resolução de caminho do comms.db (raiz, subdiretório, worktree e repo alheio) (#164)
        expected_db = resolve_db_path()
        assert expected_db.endswith("comms.db")

        # Todo caminho que NAO executa avisa no stderr: um teste que evapora em silencio
        # e exatamente a classe de bug que o #164 existe para matar.
        scripts_dir = os.path.join(orig_cwd, "scripts")
        if os.path.exists(scripts_dir):
            try:
                os.chdir(scripts_dir)
                assert resolve_db_path() == expected_db
            finally:
                os.chdir(orig_cwd)
        else:
            print(
                f"AVISO check(): caso 'subdiretorio' NAO testado — {scripts_dir} nao existe",
                file=sys.stderr,
            )

        # Teste dinâmico com worktree temporária e repo git alheio em tmp
        wt_tmp = tempfile.mkdtemp()
        try:
            wt_dir = os.path.join(wt_tmp, "test_wt")
            res_wt = subprocess.run(
                ["git", "worktree", "add", "--detach", wt_dir, "HEAD"],
                cwd=orig_cwd,
                capture_output=True,
                text=True,
            )
            if res_wt.returncode == 0:
                try:
                    os.chdir(wt_dir)
                    assert resolve_db_path() == expected_db
                finally:
                    os.chdir(orig_cwd)
                    subprocess.run(
                        ["git", "worktree", "remove", "--force", wt_dir],
                        cwd=orig_cwd,
                        capture_output=True,
                    )
            else:
                print(
                    "AVISO check(): caso 'worktree' NAO testado — 'git worktree add' falhou: "
                    f"{res_wt.stderr.strip()}",
                    file=sys.stderr,
                )

            # Teste de dentro de repo git alheio (CWD em outro repo não pode sequestrar o comms.db)
            other_repo = os.path.join(wt_tmp, "other_repo")
            os.makedirs(other_repo, exist_ok=True)
            res_init = subprocess.run(
                ["git", "init"], cwd=other_repo, capture_output=True, text=True
            )
            if res_init.returncode == 0:
                try:
                    os.chdir(other_repo)
                    assert resolve_db_path() == expected_db
                finally:
                    os.chdir(orig_cwd)
            else:
                print(
                    "AVISO check(): caso 'repo git alheio' NAO testado — 'git init' falhou: "
                    f"{res_init.stderr.strip()}",
                    file=sys.stderr,
                )
        finally:
            import shutil

            shutil.rmtree(wt_tmp, ignore_errors=True)
            os.chdir(orig_cwd)

    print("check OK")


def main():
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = p.add_subparsers(dest="cmd", required=True)

    sp = sub.add_parser("post", help="registrar turno")
    sp.add_argument("--de", required=True, choices=SENDERS)
    sp.add_argument("--para", required=True, choices=RECIPIENTS)
    sp.add_argument("--tipo", required=True, choices=KINDS)
    sp.add_argument("--titulo", required=True)
    sp.add_argument("--corpo", help="markdown; omita para ler do stdin")
    sp.add_argument("--refs")
    sp.add_argument("--force", action="store_true")

    sp = sub.add_parser("turns", help="listar turnos (default: abertos)")
    sp.add_argument("--all", action="store_true")
    sp = sub.add_parser("show", help="ver turno completo")
    sp.add_argument("id", type=int)
    sp = sub.add_parser("resolve", help="marcar turno resolvido")
    sp.add_argument("id", type=int)

    sp = sub.add_parser("items", help="listar work items (default: pendentes)")
    sp.add_argument("--all", action="store_true")
    sp = sub.add_parser("item-add", help="criar work item")
    sp.add_argument("--titulo", required=True)
    sp.add_argument("--dono", required=True, choices=SENDERS)
    sp.add_argument("--escopo", default="cross", choices=WORK_ITEM_SCOPES)
    sp.add_argument("--status", default="aberto", choices=STATUSES)
    sp.add_argument("--bloqueio")
    sp.add_argument("--refs")
    sp.add_argument("--force", action="store_true")
    sp = sub.add_parser("item-set", help="atualizar work item")
    sp.add_argument("id", type=int)
    sp.add_argument("--status", choices=STATUSES)
    sp.add_argument("--bloqueio")
    sp.add_argument("--titulo")
    sp.add_argument("--dono", choices=SENDERS)
    sp.add_argument("--refs")
    sp.add_argument("--force", action="store_true")

    sp = sub.add_parser("set", help="setar chave de estado (ex: bola_com)")
    sp.add_argument("key")
    sp.add_argument("value")
    sp.add_argument("--force", action="store_true")

    sub.add_parser("state", help="estado atual consolidado")

    sp = sub.add_parser(
        "bootstrap", help="inicializar repositório para comunicação de agentes"
    )
    sp.add_argument(
        "--force",
        action="store_true",
        help="sobrescreve banco de dados e manual existentes",
    )

    sub.add_parser("version", help="exibir versão e configurações do script")
    sub.add_parser("check", help="self-test em banco temporário")

    # Orquestração v3
    sp = sub.add_parser("start", help="iniciar trabalho em issue do Board (In progress + worktree + comms)")
    sp.add_argument("issue", type=int, help="número da issue (ex: 12)")
    sp.add_argument("--dono", choices=SENDERS, default="dev", help="papel responsável (default: dev)")
    sp.add_argument("--no-worktree", action="store_true", help="não criar git worktree automático")

    sp = sub.add_parser("finish", help="concluir trabalho em issue (In review + PR + comms)")
    sp.add_argument("issue", type=int, help="número da issue (ex: 12)")
    sp.add_argument("--pr", required=True, help="URL da PR criada (ex: https://github.com/dono/repo/pull/84)")

    sub.add_parser("board-sync", help="sincronizar status do GitHub Projects Board com comms.db")

    sp = sub.add_parser("board-item", help="imprimir item-id e status do card de uma issue")
    sp.add_argument("issue", type=int)
    sp = sub.add_parser("board-set", help="mover card da issue para outra coluna")
    sp.add_argument("issue", type=int)
    sp.add_argument("status", help="nome da coluna (ex: Ready, 'In progress', Done)")

    a = p.parse_args()
    if a.cmd == "check":
        return check()
    if a.cmd == "bootstrap":
        return cmd_bootstrap(a)
    if a.cmd == "version":
        return cmd_version(None, a)

    # board-sync escreve no kv; board-item só lê.
    is_read_only = a.cmd in ("turns", "show", "items", "state", "board-item")
    db = connect(read_only=is_read_only)
    try:
        {
            "post": cmd_post,
            "turns": cmd_turns,
            "show": cmd_show,
            "resolve": cmd_resolve,
            "items": cmd_items,
            "item-add": cmd_item_add,
            "item-set": cmd_item_set,
            "set": cmd_set,
            "state": cmd_state,
            "start": cmd_start,
            "finish": cmd_finish,
            "board-sync": cmd_board_sync,
            "board-item": cmd_board_item,
            "board-set": cmd_board_set,
        }[a.cmd](db, a)
    finally:
        if not is_read_only:
            try:
                db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            except Exception:
                pass
        db.close()


if __name__ == "__main__":
    main()
