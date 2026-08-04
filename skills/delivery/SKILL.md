---
name: delivery
description: >-
  Execute a Ready GitHub Project card through implementation: worktree,
  follow the linked plan via subagent-driven-development (preferred) or
  executing-plans, run tests/ruff, Cubic pre-flight, CodeRabbit + optional
  ai-review, open a PR, move the card to In review. Use when the user says
  deliver/execute/implement a Ready ticket, "pega o #N Ready", or run the plan
  for a board card.
---

# Delivery (Ready → In review / PR)

Metade **downstream** do par `discovery`. Empacota **PIPELINE F0→F5**:
worktree → execução do plano → review → PR. **Não faz merge (F6)** nem deploy
sem GO explícito do `owner`.

## Opinionated defaults (nesta skill)

| Escolha | Default | Por quê |
|---|---|---|
| Isolamento | **worktree** (`.worktrees/…`) | Protege `main`; casa com SDD |
| Executor de plano | **subagent-driven-development** | Review por task; melhor que sessão monolítica |
| Fallback | **executing-plans** | Só se subagents indisponíveis ou `owner` pedir inline |
| Pre-flight local | **Cubic** (`cubic review --base main`) | Consultivo; rápido; não bloqueia se indisponível |
| Review de código | **CodeRabbit** (`coderabbit review --agent`) | Diff-focused; skill `code-review` |
| Review extra | **ai-review** `-a balanced` no diff | Só se tocar auth/billing/compliance/purge/TTL |
| Integração | **PR** (`gh pr create`) + Status **In review** | Merge = `owner` (G5) |
| Assumptions (B) | Tratar como **requisitos** | Até `owner` invalidar; não reinventar discovery |

## When to use

- "Entrega o #N" / "implementa o Ready" / "roda o plano do card"
- Issue com Status **Ready**, body com links de Spec/Plano (ramo full) ou
  checklist (ramo lite)

## Inputs

- Número da issue no repo do board
- Board configurado no `comms.db` (defaults: project `1`, owner `@me`, repo do cwd)

Nenhum ID de board vive nesta skill: o `comms.py` resolve project-id, field-id e
option-ids pela API do `gh`.

```bash
gh auth refresh -h github.com -s read:project,project   # se faltar scope
gh issue view N --json number,title,body,labels,url
./comms.py board-item N          # item-id + coluna atual
```

Confirmar `status == "Ready"` antes de seguir. Se `board-item` falhar, o card não
está no board — adicionar e consultar de novo.

---

## Hard constraints (AGENTS.md + PIPELINE)

- Commits: **GO `owner`** (pedido nesta sessão ou GO escopado à rodada).
- Gate de **reviewer** (humano/canal) para commits que toquem `scripts/`,
  `tests/`, copy visível, ou spec que crie/relaxe invariante — docs-only de
  finding acordado pode ir sem esse gate.
- Testes:

```bash
uvx --with-requirements requirements.txt pytest tests/ -q
ruff check scripts tests
```

- Comandos de cloud (`gcloud`, `aws`, `firebase`) sempre com conta e projeto
  explícitos — nunca no default ambiente. Deploy só com GO.
- Nunca `git clean -x`. Sem force-push em `main`.
- Antes de Cubic, CodeRabbit ou ai-review, limitar o alvo, inspecionar
  segredos/PII e obter confirmação explícita para alvo + provedor. Se não puder
  sanear, parar; conteúdo não redigido exige autorização explícita.
- Registrar gates/handoffs no `comms.db` e reconciliar comms + docs + board em
  toda transição de estado.
- Não “corrigir” Assumptions inventando produto novo — se a hipótese quebrar
  testes/invariantes, for sensível, material ou irreversível, **STOP** e
  perguntar ao `owner`.

---

## Step 0 & Step 1 — Intake e Inicialização Unificada (Board -> In progress + Worktree + Comms)

Rode o comando de orquestração unificada v3:

```bash
./comms.py start <ISSUE_NUMER>
```

Esse comando executa atomicamente em 1 único passo:
1. Altera o status no GitHub Projects Board para **In progress**.
2. Cria o git worktree em `.worktrees/issue-<N>-<slug>` a partir da `main` atualizada.
3. Cria o item de trabalho e grava o turno de `handoff` no `comms.db`.

---

---

## Step 2 — Executar o plano

### Preferido: Subagent-Driven Development

Skill `subagent-driven-development`:

- 1 subagent implementer por task do plano
- review spec+quality por task
- ledger de progresso
- commits por task (com GO)
- no final: whole-branch review

### Fallback: executing-plans

Mesma sessão, checkpoints; anunciar a skill. Preferir se `owner` pediu inline
ou subagents falharam.

### Por task (ambos os modos)

1. TDD quando o plano exige (RED → GREEN).
2. Suite focada + ruff nos arquivos tocados; suite relevante antes do PR.
3. Commit atômico (mensagem no estilo do repo); sem `--no-verify`.
4. Se finding grave no review de task → fix antes da próxima.

---

## Step 3 — Review de código (G3)

Após o plano completo (ou antes do PR):

### 3a) Preflight de segurança (obrigatório)

Inspecionar somente `git diff main...HEAD` e os arquivos alvo. Não incluir
untracked alheio. Verificar segredos, credenciais e PII antes de qualquer envio;
se houver dúvida ou a redação mudar o sentido do review, parar e pedir decisão.

### 3b) Cubic (consultivo se houver diff de código)

Rodar uma vez antes do CodeRabbit, se instalado e autenticado:

```bash
cubic review --base main --json
```

Validar cada finding antes de corrigir e repetir no máximo uma vez. Falha de
instalação, autenticação ou rate limit não bloqueia G3: registrar o skip e seguir.
Na versão instalada, `--prompt` pode complementar `--base`; confirmar em
`cubic review --help` após upgrades, pois a CLI ainda evolui rapidamente.

### 3c) CodeRabbit (obrigatório se houver diff de código)

Skill `code-review`:

```bash
coderabbit --version 2>/dev/null || echo "NOT_INSTALLED"
coderabbit auth status 2>&1
# na worktree / repo:
coderabbit review --agent --base main
# ou: coderabbit review --agent -t uncommitted
```

Corrigir **Critical** e **Warning**. Re-rodar até limpo ou só Info.

### 3d) ai-review no diff (condicional)

Rodar se o diff tocar área sensível: auth, billing, purge/TTL, KMS, webhooks,
mensagens ao usuário final, ou invariantes de arquitetura documentados no repo:

```bash
git diff main...HEAD | ai-review --redact -a balanced -t pr-review \
  -s "Review delivery #N; foque regressões e invariantes de arquitetura."
```

Incorporar graves/médios.

### 3e) Gate reviewer do canal

Se o commit exigir gate (scripts/tests/copy/spec): preparar digest/turno para
`reviewer` conforme prática do repo — **não** fingir APPROVE.

---

## Step 4 — PR (F5) + board In review

Usar skill `finishing-a-development-branch` com opção **Push + PR** (não merge).

```bash
git push -u origin HEAD
gh pr create --title "…" --body "$(cat <<'EOF'
## Summary
- Closes #N (ou Fixes #N)
- …

## Artefatos
- Spec: …
- Plano: …
- Assumptions honoradas: (sim / lista)

## Test plan
- [ ] suíte de testes do repo verde

EOF
)"
```

Após criar o PR com `gh pr create`:

Rode o comando de conclusão unificada:

```bash
./comms.py finish <ISSUE_NUMBER> --pr <PR_URL>
```

Esse comando executa em 1 único passo:
1. Altera o status no GitHub Projects Board para **In review**.
2. Atualiza os itens no `comms.db` e posta o turno de `gate`.
3. Comenta automaticamente na Issue do GitHub com o link da PR.

**Done** no board só após merge (`owner`) — agente pode oferecer, não executar F6 sozinho.

---

## Step 5 — Parar e escalar

STOP e perguntar ao `owner` se:

- Assumption invalida comportamento observado / teste
- Plano tem gap que impede a task
- Precisa deploy, secret, Meta/Strava, ou mudança de invariante não prevista
- CodeRabbit/ai-review exigem redesign (não um fix local)
- Comms, docs e board não podem ser reconciliados

---

## Lite path (resumo)

1. In progress  
2. Executar checklist do body (docs/gcloud/ops)  
3. Evidência no comentário da issue  
4. PR mínimo **ou** handoff “feito, falta GO merge/deploy”  
5. In review (se PR) / deixar Ready→comentário se foi só ops externo (ex.:
   “template Meta aprovado”) sem código

---

## Pairing

| Fase board | Skill |
|---|---|
| Backlog → Ready | `discovery` |
| Ready → In review | **`delivery`** (esta) |
| Merge / deploy | humano (`owner`) + AGENTS.md |

Related: `docs/core/PIPELINE.md`, `brainstorming` (só se escopo mudar — senão
voltar discovery), `writing-plans` (não reescrever plano sem pedido),
`subagent-driven-development`, `executing-plans`, `using-git-worktrees`,
`code-review`, `finishing-a-development-branch`, `ai-review`, Cubic CLI.
