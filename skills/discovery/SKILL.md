---
name: discovery
description: >-
  Run autonomous discovery to move a GitHub Project card from Backlog
  to Ready: triage, repo grounding, external research, brainstorming, ai-review on
  spec, writing-plans, ai-review on plan, update the issue, set Status=Ready.
  Use when the user says discover/discovery, "deixar Ready", or asks to prepare
  a backlog card for implementation.
---

# Discovery (Backlog → Ready)

Empacota **PIPELINE F1→F2** + board GitHub. Termina com spec/plano revisados,
publicados e card **Ready** — **sem** implementar produto ou fazer deploy.

**Invariante B (Assumptions):** perguntas reversíveis e não sensíveis não
bloqueiam Ready. Registre hipótese + risco + como invalidar; implementação trata
a hipótese como requisito até `human` contradizer. Decisões de billing/preço,
segurança, compliance/retenção, contrato externo, custo material ou efeito
irreversível continuam bloqueantes.

## When to use

- "Roda discovery no #N" / "prepara esse card pra Ready"
- Card em Backlog no board sem spec/plano linkados
- Antes de Subagent-Driven / executing-plans num tema ainda nebuloso

## Inputs

- Número da issue no repo do board
- Board configurado no `comms.db` (defaults: project `1`, owner `@me`, repo do cwd):
  `comms set board.project N` / `board.owner` / `board.repo`

Se `gh` falhar com missing `read:project` / `project`:

```bash
gh auth refresh -h github.com -s read:project,project
```

## Board

Não montar chamadas `gh project` na mão: o `comms` resolve project-id,
field-id e option-ids pela API, então nenhum ID vive nesta skill.

```bash
gh issue view N --json number,title,body,labels,url   # -R só se o board apontar outro repo
comms board-item N          # item-id + coluna atual (erro se o card não existe)
comms board-set N Ready     # move o card
```

`board-item` falha se a issue não estiver no board — nesse caso, adicionar o
card e consultar de novo. Se o board tiver cards duplicados para a mesma issue,
parar e reconciliar.

## Hard constraints

- Não escrever código de produto nem fazer deploy.
- Não reabrir cards Done — follow-up = issue nova.
- Antes de qualquer envio externo, limitar o alvo, inspecionar segredos/PII e
  obter confirmação explícita para alvo + provedor. Usar `ai-review --redact`;
  se o conteúdo não puder ser saneado, parar. Nunca enviar conteúdo não
  redigido sem autorização explícita.
- Graves/médios do review **devem ser incorporados** antes de avançar (G1/G2).
- Artefatos em `docs/specs/` e `docs/plans/` (padrão do repo).
- Commits/push dos docs: só com GO do `human`; sem publicação acessível ao próximo
  agente, o card não pode virar Ready.
- Registrar gates/handoffs no `comms.db` e reconciliar comms + docs + board em
  toda transição de estado.

---

## Step 0 — Intake + triage

1. Ler issue (title, body, labels `area/*` `horizon/*` `type/*`, deps citadas).
2. Classificar:

| Ramo | Quando |
|---|---|
| **lite** | `type/ops` **e** Size XS/S **e** sem mudança de invariante/código/contrato |
| **full** | default (`type/feature`, Size M+, billing/recap/compliance de produto) |

3. Se o card mistura **2+ subsistemas independentes** (ex.: Stripe + templates Meta + privacy):
   - decompor em issues filhas;
   - Ready só na unidade que cabe num único plano; pai pode ficar epic em Backlog.

---

## Ramo lite (ops curto)

1. Repo grounding mínimo (docs/comandos já existentes).
2. MCP só se o bloqueio for externo (status Meta, Policy Strava, etc.).
3. Atualizar body: **Outcome**, **Passos**, **Critério de pronto**, **Assumptions** (se houver).
4. Resolver um único `ITEM_ID` e ajustar Priority/Size se necessário.
5. Na mesma ação, atualizar docs aplicáveis, registrar o gate no `comms.db` e
   mover o board para **Ready**.
6. Parar. Não rodar brainstorm/writing-plans.

---

## Ramo full

### 1) Repo grounding (antes da web)

Ler o que for pertinente: specs de arquitetura do repo, specs/planos irmãos,
código citado, `comms turns|items|state`, issues linkadas no board.
Anotar invariantes e gaps.

### 2) Pesquisa externa

Consultar fontes primárias oficiais com uma ferramenta disponível quando o tema
for API/docs externas (Meta, Stripe, Strava, Cloudflare, libs). Usar Context7
para documentação de bibliotecas; adicionar uma segunda fonte apenas quando a
afirmação for sensível ou incerta. Não substituir o contrato do repo. Registrar
fontes úteis no spec.

### 3) Brainstorming → spec

Seguir skill `brainstorming` (hard-gate: sem implementação).

- Spec → `docs/specs/YYYY-MM-DD-<slug>-design.md`
- Se travar em pergunta factual/técnica: sparring pontual

```bash
ai-review --redact -a fast -s "Sparring curto: responda só a pergunta; liste riscos." -p "<pergunta>"
# ou -a balanced se a dúvida for arquitetural
```

### 4) ai-review do spec (G1) → incorporar

```bash
ai-review --redact -f docs/specs/<spec>.md -t architecture -a balanced
# billing / compliance / retenção / auth: -a full
```

Incorporar findings **graves e médios** no spec. Só então avançar.

### 5) writing-plans → plano

Seguir skill `writing-plans`.

- Plano → `docs/plans/YYYY-MM-DD-<slug>.md`
- Header + tasks checkbox + TDD + sem placeholders.

### 6) ai-review do plano (G2) → incorporar

```bash
ai-review --redact -f docs/plans/<plan>.md -t plan-review -a balanced
# temas sensíveis: -a full
```

Incorporar graves/médios. Re-rodar review só se o plano mudou materialmente.

### 7) Exit checklist + board Ready

Atualizar a issue (`gh issue edit`) com:

```markdown
## Outcome
…

## Artefatos
- Spec: `docs/specs/…`
- Plano: `docs/plans/…`
- Reviews: (paths ou resumo consenso)

## Dependências
- Bloqueia / bloqueado por #…

## Critério de pronto
- [ ] …

## Assumptions
1. **Pergunta:** …
   - **Hipótese adotada:** …
   - **Risco se errada:** …
   - **Como invalidar:** `human` / dado / spike
(Se não houver perguntas abertas: `Nenhuma — spec/plano fechados.`)

## Sequência
1. …
```

Depois:

1. Resolver exatamente um item no project (`gh project item-add` se faltar).
2. Verificar que spec/plano estão publicados em commit autorizado ou storage
   compartilhado e acessíveis ao próximo agente.
3. Na mesma ação de fechamento:
   - atualizar o status declarado nos docs aplicáveis;
   - atualizar a issue e labels `horizon/now` ou `horizon/next`;
   - registrar o gate no canal;
   - mover o board para **Ready**.

```bash
comms post --de arch --para todos --tipo gate \
  --titulo "discovery Ready #N" \
  --corpo "spec+plan reviewed; assumptions no body; Status=Ready"
```

4. Se qualquer fonte divergir, corrigir antes de reportar Ready.
5. Reportar ao `human`: URL da issue, paths, Assumptions em destaque, próximo passo (SDD / executing-plans).

---

## Stop conditions (não virar Ready “cego”)

- Review G1/G2 ainda com grave **não** incorporado → corrigir antes.
- Decompose incompleto (escopo ainda multi-subsistema) → não Ready na issue inchada.
- Falta de artefato publicado (spec ou plano no ramo full) → não Ready.
- Assumption sensível, material ou irreversível → esperar decisão do `human`.
- Comms, docs e board divergentes → reconciliar antes.

**Não** é stop condition: espera por resposta do `human` sobre hipótese
reversível e não sensível (invariante B).

## Related

- `docs/core/PIPELINE.md` — F1/F2 e política de débito
- Skills: `brainstorming`, `writing-plans`, `ai-review`
- Board: `comms board-sync` (config em `board.project` / `board.owner`)
