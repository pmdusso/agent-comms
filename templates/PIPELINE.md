# PIPELINE — Política de Entrega por Gates (v{{VERSION}})

Política para grandes atividades deste repositório. Padrão invariante por fase:
**artefato → gate → registro no comms.db**. Modelos nunca aparecem na política —
apenas papéis; o binding papel→CLI vive em `ROLES` (`src/policy.rs` do pacote agent-comms).

## Binding atual de papéis

| Papel    | CLI | Responsabilidade |
|----------|-----|------------------|
| executor | {{EXECUTOR}} | escreve spec, plano e código |
| panel    | {{PANEL}} | revisão multi-modelo (ai-review) nos gates 1–3 |
| pr_bot   | {{PR_BOT}} | review automatizado do PR (gate 4) |
| advisor  | {{ADVISOR}} | taste pass opcional pré-merge |

No início de cada rodada, registre o binding para o experimento ser analisável:

    comms set pipe.<slug>.binding "executor={{EXECUTOR}};panel={{PANEL}}"

## Board (GitHub Projects)

O `comms` descobre project-id, field-id e option-ids via uma query GraphQL (`gh api graphql`) — nada
hardcoded. Só o número/dono do board são configuráveis, e só se fugirem do default:

    comms set board.project 1      # default: 1
    comms set board.owner @me      # default: @me
    comms set board.repo dono/repo # default: repo do diretório atual

    comms board-item <N>           # item-id + coluna atual do card
    comms board-set <N> Ready      # move o card de coluna

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
| F3 | Execução | execute-plan → diff na worktree | G3: review-pr local | 1 work_item por finding: `[{{SEVERITIES}}] título` |
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

Verificação do gate F4: `comms gate` (exit 1 se alguma finding ficou sem destino).
