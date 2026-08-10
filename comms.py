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
  ./comms.py agents                                 # roster (= docs/prompts/*.md)
  ./comms.py brief dev                              # prompt + estado, num blob só
  ./comms.py gate [--refs issue:42]                 # F4/G3/G4; exit 1 se reprovar
  ./comms.py trunk    ./comms.py testcmd            # tronco e suíte, sem chutar
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
import unicodedata
from pathlib import Path
from string import Template

# ==============================================================================
# CONFIGURAÇÃO DE COMUNICAÇÃO DE AGENTES (SQLITE CHATTER / SYNC)
# ==============================================================================
__version__ = "4.0.0"

# O ROSTER DE AGENTES É O CONTEÚDO DE docs/prompts/. Onboardear um agente ad-hoc
# no meio da sessão = criar docs/prompts/<nome>.md. Nada aqui precisa mudar, e o
# banco não precisa ser recriado: sender/recipient/owner NÃO têm CHECK no DDL
# justamente para o roster poder crescer sem migração destrutiva. A proteção
# contra typo vive no argparse (`choices`), que lê o roster a cada invocação.
#
# RECOMENDAÇÃO: mantenha um agente central de coordenação ("arch") encarregado de
# ler/escrever tarefas, interagir com Jira/Github/PRs e delegar escopos.
DEFAULT_ROSTER = ("arch", "pm", "dev", "tester", "reviewer")
HUMAN = "human"  # dono da decisão final (merge/deploy); não tem prompt, é gente
BROADCAST = "todos"  # destinatário coletivo, nunca remetente

# ATENÇÃO: as tuplas abaixo VIRAM CHECKs no DDL — são política fechada, não roster.
# Mudá-las exige ./comms.py bootstrap --force (zera dados) ou ./comms.py migrate.
KINDS = ("mensagem", "decisao", "bloqueio", "handoff", "pergunta", "gate")
STATUSES = ("aberto", "bloqueado", "feito", "debito")

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
PROMPTS_DIR = Path(DB_PATH).parent / "docs" / "prompts"


def agent_roster():
    """Papéis que podem postar no canal: um por arquivo em docs/prompts/.

    Onboardear um agente ad-hoc é criar o prompt dele — o passo que você ia dar de
    qualquer jeito. Assim o prompt e a permissão de postar nunca divergem.

    Fallback para DEFAULT_ROSTER quando docs/prompts/ não existe (repositório ainda
    não instalado), para o CLI continuar utilizável antes do primeiro prompt.
    """
    names = (
        sorted(p.stem for p in PROMPTS_DIR.glob("*.md")) if PROMPTS_DIR.is_dir() else []
    )
    if not names:
        names = list(DEFAULT_ROSTER)
    if HUMAN not in names:
        names.append(HUMAN)
    return tuple(names)


SENDERS = agent_roster()
RECIPIENTS = SENDERS + (BROADCAST,)
WORK_ITEM_SCOPES = SENDERS + ("cross",)


def default_branch(repo_root):
    """Tronco do repositório. Nunca assumir 'main': o default aqui é `dev`.

    origin/HEAD é a resposta certa quando existe. Sem remote (repo local, clone
    raso, CI sem fetch de HEAD), cai para o primeiro tronco convencional que
    existir de fato, e só então para o branch atual.
    """
    res = subprocess.run(
        ["git", "symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        cwd=repo_root,
        capture_output=True,
        text=True,
    )
    if res.returncode == 0 and res.stdout.strip():
        return res.stdout.strip().split("/", 1)[-1]  # "origin/dev" -> "dev"
    for cand in ("main", "master", "dev"):
        if (
            subprocess.run(
                ["git", "rev-parse", "--verify", "--quiet", cand],
                cwd=repo_root,
                capture_output=True,
            ).returncode
            == 0
        ):
            return cand
    res = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        cwd=repo_root,
        capture_output=True,
        text=True,
    )
    return res.stdout.strip() or "main"


def _sql_in(values):
    if not values:
        raise ValueError("Valores para cláusula IN SQL não podem ser vazios.")
    for v in values:
        if not re.match(r"^[a-zA-Z0-9_\-]+$", v):
            raise ValueError(f"Identificador inválido configurado: {v}")
    return "(" + ", ".join(f"'{v}'" for v in values) + ")"


# sender/recipient/owner/scope são TEXT puro de propósito: o roster muda em runtime
# (ver agent_roster) e um CHECK aqui congelaria os papéis no schema. Validação de
# typo é do argparse. kind/status seguem com CHECK — são política, não roster.
SCHEMA = f"""
CREATE TABLE IF NOT EXISTS turns (
  id INTEGER PRIMARY KEY,
  sender TEXT NOT NULL,
  recipient TEXT NOT NULL,
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
  owner TEXT NOT NULL,
  scope TEXT NOT NULL,
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
* **Remetentes (Senders):** $SENDERS — **derivado de `docs/prompts/*.md`**, não fixo
* **Destinatários (Recipients):** os senders + `todos` (broadcast, nunca remetente)
* **Tipos de Turnos (Kinds):** $KINDS
* **Status de Tarefas (Statuses):** $STATUSES
* **Escopos de Trabalho (Scopes):** os senders + `cross`

## Onboardear um agente ad-hoc

O roster é o conteúdo de `docs/prompts/`. Para subir um agente novo no meio da
sessão, crie o prompt dele — e só:

```bash
cp docs/prompts/dev.md docs/prompts/dba.md   # e edite
./comms.py agents                            # confere o roster
./comms.py post --de dba --para arch --tipo pergunta --titulo "..." --corpo "..."
```

Não há tuplas a editar nem banco a recriar: `sender`/`recipient`/`owner` não têm
CHECK no schema, de propósito. Bancos criados antes da v4 congelam o roster no
DDL — `./comms.py migrate` libera preservando turnos e itens.

## Responsabilidades

Papéis padrão. Agentes ad-hoc adicionados depois não entram nesta tabela — o
roster vivo é `./comms.py agents`.

| Papel | Responsabilidade principal |
|---|---|
| `arch` | arquitetura, priorização, decisões e integração |
| `pm` | discovery, brainstorms, roadmap, backlog e critérios de aceite |
| `dev` | implementação e documentação técnica |
| `tester` | estratégia de testes, validação executável e aceite |
| `reviewer` | revisão independente de correção, segurança e simplicidade |
| `human` | direção de produto e decisões humanas finais |

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


HANDOFF_CAMPOS = ("objetivo", "criterios", "fronteiras")


def handoff_faltando(body):
    """Campos obrigatórios ausentes num corpo de handoff (lista vazia = ok).

    Delegação vaga é a falha nº 1 de orquestração medida (MAST, Anthropic):
    objetivo, critérios verificáveis e fronteiras viajam COM o handoff, não na
    cabeça de quem delega. Acento-insensível: 'Critérios:' e 'Criterios:' valem.
    """
    plano = "".join(
        c
        for c in unicodedata.normalize("NFKD", (body or "").lower())
        if not unicodedata.combining(c)
    )
    return [campo for campo in HANDOFF_CAMPOS if f"{campo}:" not in plano]


def cmd_post(db, a):
    body = a.corpo if a.corpo is not None else sys.stdin.read()
    guard_pii(a.titulo, body, a.refs, force=a.force)
    if a.tipo == "handoff" and not a.force:
        faltando = handoff_faltando(body)
        if faltando:
            sys.exit(
                "ERRO: handoff sem os campos: "
                + ", ".join(faltando)
                + ". O corpo deve nomear 'Objetivo:', 'Critérios:' e 'Fronteiras:' "
                "(delegação vaga vira retrabalho). Use --force se for exceção consciente."
            )
    cur = db.execute(
        "INSERT INTO turns (sender, recipient, kind, title, body, refs) VALUES (?,?,?,?,?,?)",
        (a.de, a.para, a.tipo, a.titulo, body, a.refs),
    )
    db.commit()
    print(f"turno {cur.lastrowid} registrado ({a.de} → {a.para}, {a.tipo})")


def cmd_turns(db, a):
    clauses, params = [], []
    if not a.all:
        clauses.append("resolved_at IS NULL")
    # "todos" é broadcast: quem filtra por um papel também recebe o que foi para todos.
    if getattr(a, "para", None):
        clauses.append("recipient IN (?, ?)")
        params += [a.para, BROADCAST]
    where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
    rows = db.execute(
        f"SELECT * FROM turns {where} ORDER BY id DESC", params
    ).fetchall()
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
            base = a.base or default_branch(repo_root)
            res = subprocess.run(
                ["git", "worktree", "add", "-b", branch_name, str(wt_dir), base],
                cwd=repo_root,
                capture_output=True,
                text=True,
            )
            if res.returncode == 0:
                print(
                    f"✓ Git Worktree criado em: .worktrees/{slug} "
                    f"(branch: {branch_name}, base: {base})"
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
            f"PR={pr_url}; entrega concluída e aguardando G4/G5 (review + merge do human).",
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


FINDING_RE = re.compile(rf"^\[({'|'.join(SEVERITIES)})\]", re.IGNORECASE)


# Convenção por ecossistema, na ordem em que um repo poliglota deve ser lido.
# Só palpite: `./comms.py set test.cmd "..."` tem precedência e é a resposta certa
# em qualquer repo que fuja do padrão.
TEST_CMD_HINTS = (
    ("Makefile", "make test"),
    ("pyproject.toml", "uv run pytest -q"),
    ("package.json", "npm test"),
    ("Cargo.toml", "cargo test"),
    ("go.mod", "go test ./..."),
)


def test_command(db):
    """Comando que roda a suíte deste repositório, ou None se não der para saber."""
    override = _kv_get(db, "test.cmd")
    if override:
        return override
    root = Path(DB_PATH).parent
    for arquivo, cmd in TEST_CMD_HINTS:
        alvo = root / arquivo
        if not alvo.exists():
            continue
        # Makefile sem alvo `test` mandaria o agente rodar um comando que não existe.
        if arquivo == "Makefile" and not re.search(
            r"^test\s*:", alvo.read_text(encoding="utf-8", errors="ignore"), re.M
        ):
            continue
        # package.json sem script `test` tem o mesmo problema: `npm test` sai com
        # erro em vez de rodar suíte. Existir package.json prova que é um repo Node,
        # não que alguém escreveu testes nele.
        if arquivo == "package.json":
            try:
                scripts = json.loads(alvo.read_text(encoding="utf-8")).get("scripts", {})
            except (ValueError, OSError):
                continue
            if "test" not in scripts:
                continue
        return cmd
    return None


def cmd_testcmd(db, _a):
    """Suíte do repo, para prompts e skills não fixarem pytest/npm/make.

    Uso nos comandos: $(./comms.py testcmd)
    """
    cmd = test_command(db)
    if not cmd:
        sys.exit(
            "ERRO: não deu para inferir o comando de teste deste repositório.\n"
            '  ./comms.py set test.cmd "<comando>"'
        )
    print(cmd)


def cmd_trunk(_db, _a):
    """Nome do tronco, para os docs e skills não chutarem 'main'.

    Uso nos comandos: git diff $(./comms.py trunk)...HEAD
    """
    print(default_branch(Path(DB_PATH).parent))


def cmd_gate(db, a):
    """Critério de saída F4/G3/G4: nenhuma finding morre em silêncio.

    Só olha work_items cujo título tem prefixo de severidade — o resto é trabalho
    normal, não finding de gate. G1/G2 (painel rodou) e G5 (merge humano) não são
    verificáveis daqui: são julgamento, e fingir que são checagem seria pior.
    """
    q, params = "SELECT * FROM work_items", []
    if a.refs:
        q += " WHERE refs LIKE ?"
        params.append(f"%{a.refs}%")
    falhas, checadas = [], 0
    for r in db.execute(q + " ORDER BY id", params):
        m = FINDING_RE.match(r["title"])
        if not m:
            continue
        checadas += 1
        sev, status = m.group(1).lower(), r["status"]
        if status in ("aberto", "bloqueado"):
            falhas.append(f"#{r['id']} [{sev}] sem destino (status={status}): {r['title']}")
        elif status == "debito" and sev == "grave":
            falhas.append(f"#{r['id']} grave como débito — a política proíbe: {r['title']}")
        elif status == "debito" and sev == "medio" and not (r["blocker"] or "").strip():
            falhas.append(
                f"#{r['id']} médio em débito sem justificativa "
                f"(item-set {r['id']} --status debito --bloqueio \"motivo\"): {r['title']}"
            )
    escopo = f" em {a.refs}" if a.refs else ""
    if falhas:
        print(f"GATE REPROVADO — {len(falhas)} de {checadas} findings{escopo}:")
        for f in falhas:
            print(f"  {f}")
        sys.exit(1)
    # Zero findings passa, mas dizer quantas foram checadas evita ler "0 de 0" como aprovação real.
    print(f"GATE OK — {checadas} findings{escopo}, todas com destino.")


def cmd_brief(db, a):
    """Todo o contexto de um agente num blob só, para colar ou pipar no CLI dele."""
    prompt = PROMPTS_DIR / f"{a.agente}.md"
    if prompt.is_file():
        print(prompt.read_text(encoding="utf-8").rstrip())
    elif a.agente != HUMAN:
        print(f"(sem prompt em docs/prompts/{a.agente}.md — agente só no histórico)")
    print(f"\n{'=' * 70}\n== Estado do canal para `{a.agente}` ==\n")
    for r in db.execute("SELECT key, value FROM kv ORDER BY key"):
        print(f"{r['key']}: {r['value']}")
    print(f"\n-- Seus itens abertos --")
    rows = db.execute(
        "SELECT * FROM work_items WHERE owner=? AND status != 'feito' ORDER BY id",
        (a.agente,),
    ).fetchall()
    for r in rows:
        extra = f" — bloqueio: {r['blocker']}" if r["blocker"] else ""
        print(f"#{r['id']} [{r['status']}] {r['title']}{extra}")
    print("(nenhum)" if not rows else "")
    print("-- Turnos abertos para você --")
    rows = db.execute(
        "SELECT * FROM turns WHERE resolved_at IS NULL AND recipient IN (?, ?) ORDER BY id",
        (a.agente, BROADCAST),
    ).fetchall()
    for r in rows:
        refs = f"\nrefs: {r['refs']}" if r["refs"] else ""
        # Corpo inteiro: o ponto do brief é não precisar de um `show` por turno.
        print(f"\n#{r['id']} {r['sender']} → {r['recipient']} · {r['kind']}{refs}")
        print(f"{r['title']}\n\n{r['body']}")
    print("(nenhum)" if not rows else "")
    print(f"\nResolva o que atender: ./comms.py resolve <ID>")


_FROZEN_TABLES = ("turns", "work_items")
_INDEXES = (
    "idx_turns_recipient",
    "idx_turns_sender",
    "idx_turns_resolved",
    "idx_work_items_owner",
    "idx_work_items_status",
)


def _frozen_roster_tables(db):
    """Tabelas cujo DDL ainda congela o roster num CHECK (bancos <= v3)."""
    rows = db.execute(
        "SELECT name, sql FROM sqlite_master WHERE type='table' AND name IN (?,?)",
        _FROZEN_TABLES,
    ).fetchall()
    return [
        r["name"]
        for r in rows
        if r["sql"] and ("CHECK (sender IN" in r["sql"] or "CHECK (owner IN" in r["sql"])
    ]


def cmd_agents(db, _a):
    frozen = _frozen_roster_tables(db)
    print(f"== Roster ({PROMPTS_DIR}) ==")
    counts = {
        r["sender"]: r["n"]
        for r in db.execute("SELECT sender, COUNT(*) n FROM turns GROUP BY sender")
    }
    for name in SENDERS:
        origem = "humano" if name == HUMAN else f"docs/prompts/{name}.md"
        print(f"  {name:12} {counts.get(name, 0):>4} turnos   {origem}")
    print(f"\nBroadcast: {BROADCAST} (só destinatário)")
    print("Onboardear agente novo: crie docs/prompts/<nome>.md — nada mais.")
    # Remetentes já no histórico que sumiram do roster (prompt deletado/renomeado).
    orfaos = sorted(set(counts) - set(SENDERS))
    if orfaos:
        print(f"\nSem prompt, mas presentes no histórico: {', '.join(orfaos)}")
    if frozen:
        print(
            f"\nAVISO: {', '.join(frozen)} ainda congelam o roster num CHECK do schema "
            "(banco criado em versão <= 3). Agentes novos serão rejeitados até você "
            "rodar ./comms.py migrate (preserva os dados)."
        )


def cmd_migrate(db, _a):
    """Remove os CHECKs de roster de turns/work_items preservando as linhas.

    SQLite não altera CHECK: o caminho é recriar a tabela. Em vez de duplicar o DDL
    aqui, renomeia as antigas, deixa o SCHEMA criar as novas e copia as linhas.
    """
    frozen = _frozen_roster_tables(db)
    if not frozen:
        print("Nada a migrar: o schema já aceita roster dinâmico.")
        return
    cols = {
        "turns": "id, sender, recipient, kind, title, body, refs, created_at, resolved_at",
        "work_items": "id, title, owner, scope, status, blocker, refs, updated_at",
    }
    db.commit()  # fecha qualquer transação implícita aberta por um DML anterior
    try:
        db.execute("BEGIN")
        # Índices primeiro: um índice sobrevive ao RENAME grudado na tabela velha, e
        # aí o CREATE INDEX IF NOT EXISTS do SCHEMA vira no-op na tabela nova.
        for idx in _INDEXES:
            db.execute(f"DROP INDEX IF EXISTS {idx}")
        for t in _FROZEN_TABLES:
            db.execute(f"ALTER TABLE {t} RENAME TO {t}_old")
        # Statement a statement, não executescript: ele daria COMMIT implícito e
        # jogaria fora a transação que protege a cópia das linhas.
        for stmt in filter(None, (s.strip() for s in SCHEMA.split(";"))):
            db.execute(stmt)
        for t in _FROZEN_TABLES:
            n = db.execute(
                f"INSERT INTO {t} ({cols[t]}) SELECT {cols[t]} FROM {t}_old"
            ).rowcount
            print(f"  {t}: {n} linhas preservadas")
        for t in _FROZEN_TABLES:
            db.execute(f"DROP TABLE {t}_old")
        db.execute("COMMIT")
    except Exception:
        db.execute("ROLLBACK")
        raise
    print("✓ Roster liberado. Agentes novos entram sem recriar o banco.")


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
            KINDS=", ".join(KINDS),
            STATUSES=", ".join(STATUSES),
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

        # Agente ad-hoc entra sem tocar no schema: é o ponto do roster dinâmico.
        db.execute(
            "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('dba', 'arch', 'pergunta', 'indice em turns?', 'b')"
        )
        db.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('tuning', 'dba', 'dba', 'aberto')"
        )
        db.commit()
        assert (
            db.execute("SELECT sender FROM turns WHERE title LIKE 'indice%'").fetchone()[
                0
            ]
            == "dba"
        )

        # kind/status continuam fechados — são política, não roster.
        for sql, rotulo in (
            (
                "INSERT INTO turns (sender, recipient, kind, title, body) VALUES ('arch', 'todos', 'tipo_invalido', 't', 'b')",
                "kind",
            ),
            (
                "INSERT INTO work_items (title, owner, scope, status) VALUES ('y', 'dev', 'dev', 'status_invalido')",
                "status",
            ),
        ):
            try:
                db.execute(sql)
                raise AssertionError(f"CHECK de {rotulo} inválido não disparou")
            except sqlite3.IntegrityError:
                pass

        db.close()

        # 1a. Roster sai de docs/prompts/, e a migração de banco antigo não perde linha
        global PROMPTS_DIR
        orig_prompts = PROMPTS_DIR
        try:
            PROMPTS_DIR = Path(tmp) / "docs" / "prompts"
            assert agent_roster() == DEFAULT_ROSTER + (HUMAN,), "fallback sem diretório"
            PROMPTS_DIR.mkdir(parents=True)
            for nome in ("arch", "dev", "dba"):
                (PROMPTS_DIR / f"{nome}.md").write_text("x", encoding="utf-8")
            assert agent_roster() == ("arch", "dba", "dev", HUMAN), agent_roster()
        finally:
            PROMPTS_DIR = orig_prompts

        legacy_path = os.path.join(tmp, "legacy.db")
        legacy = sqlite3.connect(legacy_path)
        legacy.executescript(
            """
            CREATE TABLE turns (
              id INTEGER PRIMARY KEY,
              sender TEXT NOT NULL CHECK (sender IN ('arch','dev')),
              recipient TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
              body TEXT NOT NULL, refs TEXT,
              created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
              resolved_at TEXT);
            CREATE TABLE work_items (
              id INTEGER PRIMARY KEY, title TEXT NOT NULL,
              owner TEXT NOT NULL CHECK (owner IN ('arch','dev')),
              scope TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'aberto',
              blocker TEXT, refs TEXT,
              updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')));
            CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL,
              updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')));
            CREATE INDEX idx_turns_sender ON turns(sender);
            INSERT INTO turns (sender,recipient,kind,title,body)
              VALUES ('arch','dev','mensagem','historico','preservar');
            INSERT INTO work_items (title,owner,scope) VALUES ('item antigo','dev','dev');
            """
        )
        legacy.commit()
        legacy.close()

        legacy = connect(legacy_path, read_only=True)
        legacy.row_factory = sqlite3.Row
        assert _frozen_roster_tables(legacy) == ["turns", "work_items"]
        try:
            legacy.execute(
                "INSERT INTO turns (sender,recipient,kind,title,body) VALUES ('dba','arch','pergunta','t','b')"
            )
            raise AssertionError("banco legado deveria rejeitar agente novo")
        except sqlite3.IntegrityError:
            pass
        cmd_migrate(legacy, None)
        assert _frozen_roster_tables(legacy) == []
        # O ponto da migração: o agente novo entra E o histórico continua lá.
        legacy.execute(
            "INSERT INTO turns (sender,recipient,kind,title,body) VALUES ('dba','arch','pergunta','t','b')"
        )
        assert (
            legacy.execute(
                "SELECT body FROM turns WHERE title='historico'"
            ).fetchone()[0]
            == "preservar"
        )
        assert legacy.execute("SELECT COUNT(*) FROM work_items").fetchone()[0] == 1
        cmd_migrate(legacy, None)  # idempotente
        legacy.close()

        # 1c. Gate F4/G3/G4: cada porta de saída de uma finding
        g = connect(os.path.join(tmp, "gate.db"), read_only=False)
        args_gate = argparse.Namespace(refs=None)

        def gate_falha():
            try:
                cmd_gate(g, args_gate)
                return False
            except SystemExit:
                return True

        # Work item comum não é finding: não tem prefixo de severidade, não trava o gate.
        g.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('#42: feature', 'dev', 'cross', 'aberto')"
        )
        assert not gate_falha(), "item comum aberto não deveria reprovar o gate"

        g.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('[pequeno] typo', 'dev', 'dev', 'aberto')"
        )
        assert gate_falha(), "finding aberta tem que reprovar"
        g.execute("UPDATE work_items SET status='debito' WHERE title LIKE '[pequeno]%'")
        assert not gate_falha(), "pequeno pode virar débito sem justificativa"

        g.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('[medio] n+1', 'dev', 'dev', 'debito')"
        )
        assert gate_falha(), "medio em débito sem justificativa tem que reprovar"
        g.execute("UPDATE work_items SET blocker='fora do hot path' WHERE title LIKE '[medio]%'")
        assert not gate_falha(), "medio em débito com justificativa passa"

        g.execute(
            "INSERT INTO work_items (title, owner, scope, status) VALUES ('[grave] sqli', 'dev', 'dev', 'debito')"
        )
        assert gate_falha(), "grave nunca vira débito"
        g.execute("UPDATE work_items SET status='feito' WHERE title LIKE '[grave]%'")
        assert not gate_falha()

        # Escopo por refs isola findings de outra issue.
        g.execute(
            "INSERT INTO work_items (title, owner, scope, status, refs) VALUES ('[grave] outra', 'dev', 'dev', 'aberto', 'issue:99')"
        )
        assert gate_falha(), "finding aberta em qualquer escopo reprova o gate global"
        args_gate.refs = "issue:42"
        assert not gate_falha(), "gate com --refs não deve ver finding de outra issue"
        g.commit()
        g.close()

        # 1d. Tronco vem do git, nunca de um literal "main"
        branch_repo = os.path.join(tmp, "branch_repo")
        os.makedirs(branch_repo)
        env = {**os.environ, "GIT_CONFIG_GLOBAL": os.path.join(tmp, "nonexistent")}
        subprocess.run(
            ["git", "init", "-q", "-b", "tronco-esquisito", "."],
            cwd=branch_repo,
            capture_output=True,
            env=env,
        )
        subprocess.run(
            ["git", "commit", "-q", "--allow-empty", "-m", "x", "--no-gpg-sign"],
            cwd=branch_repo,
            capture_output=True,
            env=env,
        )
        assert default_branch(branch_repo) == "tronco-esquisito", default_branch(
            branch_repo
        )

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

        # Os docs também precisam ser portáteis. Este arquivo tem check(); prompts e
        # skills não têm nada — e é neles que um literal de um projeto específico
        # sobrevive em silêncio, porque Markdown nunca falha alto.
        pack = Path(__file__).resolve().parent
        alvos = sorted(pack.glob("prompts/*.md")) + sorted(
            pack.glob("skills/*/SKILL.md")
        )
        if alvos:
            proibidos = (
                (r"/Users/", "caminho absoluto de uma máquina"),
                (r"\b(main|master)\b", "nome de branch fixo — use $(./comms.py trunk)"),
                (r"-R\s+[\w.-]+/[\w.-]+", "repo fixo no gh — deixe o gh resolver"),
                (r"PVT[A-Z]*_", "node id de GitHub Project"),
                (r"--(project|field)-id\s+\S", "id de board fixo — use board-item/board-set"),
                (
                    # `\./test-all` fica fora do \b inicial: não há fronteira de palavra
                    # antes de um ponto, então dentro do grupo ele nunca casaria.
                    r"\b(pytest|uvx|ruff|make test|npm (test|run)|yarn test|go test"
                    r"|cargo test)\b|\./test-all\b",
                    "runner de teste fixo — use $(./comms.py testcmd)",
                ),
            )
            vazamentos = [
                f"{f.relative_to(pack)}:{i}: {porque} → {linha.strip()[:70]}"
                for f in alvos
                for i, linha in enumerate(
                    f.read_text(encoding="utf-8").splitlines(), start=1
                )
                for padrao, porque in proibidos
                # A linha que ensina o resolvedor cita o que ele substitui.
                if re.search(padrao, linha) and "comms.py testcmd" not in linha
            ]
            assert not vazamentos, "literais não-portáveis nos docs:\n" + "\n".join(
                vazamentos
            )

            # 1f. Coerência dos prompts: esqueleto completo, ênfase orçada e
            # template de handoff em sincronia entre prompt (pm) e código (post).
            # Contradição semântica é julgamento; isto guarda o que dá pra guardar.
            for f in sorted(pack.glob("prompts/*.md")):
                texto = f.read_text(encoding="utf-8")
                for secao in ("## Antes de agir", "## Fluxo", "## Limites", "## Término"):
                    assert secao in texto, f"{f.name}: seção '{secao}' ausente"
                gritos = texto.count("NUNCA")
                assert gritos <= 5, (
                    f"{f.name}: {gritos}x NUNCA — se tudo grita, nada grita (máx 5)"
                )
            pm_texto = (pack / "prompts" / "pm.md").read_text(encoding="utf-8").lower()
            for campo in ("objetivo", "critérios", "fronteiras"):
                assert campo in pm_texto, (
                    f"pm.md não cita o campo '{campo}' que cmd_post exige no handoff"
                )
            assert handoff_faltando("Objetivo: x\nCritérios: y\nFronteiras: z") == []
            assert handoff_faltando("Criterios: sem acento\nObjetivo: x") == ["fronteiras"]
            assert handoff_faltando("bora") == list(HANDOFF_CAMPOS)
        else:
            print(
                f"AVISO check(): docs NAO varridos — {pack}/prompts e {pack}/skills "
                "nao existem (rodando de uma instalacao, nao do pacote fonte)",
                file=sys.stderr,
            )

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
                # Roster vivo, não string fixa: SENDERS é congelado no import a partir
                # de docs/prompts/ do repo real, e o chdir para o tmp não o desfaz. Com
                # a string fixa, o check só passava em repo ainda sem prompts — instalar
                # o agent-comms quebrava o autoteste do próprio agent-comms.
                assert ", ".join(SENDERS) in md_content
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
    sp.add_argument(
        "--para", choices=RECIPIENTS, help="só turnos para este papel (inclui broadcast)"
    )
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
    sp.add_argument("--base", help="branch base da worktree (default: tronco do repo)")

    sp = sub.add_parser("finish", help="concluir trabalho em issue (In review + PR + comms)")
    sp.add_argument("issue", type=int, help="número da issue (ex: 12)")
    sp.add_argument("--pr", required=True, help="URL da PR criada (ex: https://github.com/dono/repo/pull/84)")

    sub.add_parser("board-sync", help="sincronizar status do GitHub Projects Board com comms.db")

    sub.add_parser("agents", help="listar o roster (um agente por docs/prompts/*.md)")
    sub.add_parser("trunk", help="imprimir o branch tronco do repo (nunca assume main)")
    sub.add_parser("testcmd", help="imprimir o comando de teste do repo (kv test.cmd ou convenção)")

    sp = sub.add_parser("brief", help="prompt + estado + turnos de um agente, num blob só")
    sp.add_argument("agente", choices=SENDERS)

    sp = sub.add_parser(
        "gate", help="verificar F4/G3/G4: toda finding tem destino (exit 1 se não)"
    )
    sp.add_argument("--refs", help="limitar a um escopo (ex: issue:42)")

    sub.add_parser(
        "migrate", help="liberar o roster em banco antigo (preserva turnos e itens)"
    )

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
    is_read_only = a.cmd in (
        "turns", "show", "items", "state", "board-item", "agents", "brief", "gate",
        "trunk", "testcmd",
    )
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
            "agents": cmd_agents,
            "migrate": cmd_migrate,
            "brief": cmd_brief,
            "gate": cmd_gate,
            "trunk": cmd_trunk,
            "testcmd": cmd_testcmd,
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
