# Guia de Configuração e Uso do Agent Comms em Novos Projetos

Este guia integra a infraestrutura de comunicação e orquestração **Agent Comms** a um repositório novo ou existente.

## Passo 1: Instalação

Pré-requisitos: Rust/Cargo via `rustup` e GitHub CLI `gh` autenticado. Clone o pacote uma vez e aponte o installer para o projeto:

```bash
git clone git@github.com:pmdusso/agent-comms.git ~/code/shared/agent-comms
~/code/shared/agent-comms/install.sh ~/code/meu-novo-projeto
```

O installer compila o engine Rust e instala o binário `comms` em `~/.local/bin`; defina `COMMS_BIN_DIR` para escolher outro diretório. Ele também copia prompts e skills, executa `comms bootstrap` e `comms migrate`, e avisa sobre instruções que ainda chamam o engine antigo.

Para atualizar, rode o installer novamente. Se o destino ainda contém o script Python legado, use `--update`: essa flag é obrigatória para remover o legado e regerar os manuais, e também sobrescreve prompts e skills locais.

## Passo 2: Regra de ouro no `AGENTS.md`

Adicione ao `AGENTS.md` do projeto:

```markdown
## Comunicação & Separação de Canais

Ao iniciar uma sessão neste repositório:
1. Execute `comms brief <seu-papel>` — traz prompt, estado e seus turnos.
2. Sincronize o Board do GitHub com o banco local via `comms board-sync`.
3. Os papéis válidos são os de `comms agents` (padrão em docs/prompts + ad-hoc no banco).
4. Registre decisões, bloqueios e handoffs no `comms.db`.
5. Resolva os turnos atendidos com `comms resolve <ID>`.

### Regra de Ouro de Canais

- **GitHub Projects Board / Issues / PRs / Chat da Sessão:** Canal Humano Oficial.
- **`comms.db`:** Canal Operacional Local (Agente ↔ Agente).

### Orquestração Unificada

- **Ao iniciar uma issue:** `comms start <ISSUE_NUMBER>`
- **Ao concluir e abrir PR:** `comms finish <ISSUE_NUMBER> --pr <PR_URL>`
```

## Passo 3: Subir sessões e personas

Um terminal por agente. `brief` monta prompt, estado, itens e o corpo dos turnos endereçados ao papel:

```bash
comms agents
comms brief dev
comms brief tester | claude -p
comms brief reviewer | gemini -p
```

O roster padrão vem de `docs/prompts/`. Para uma persona nova em runtime, crie o registro ad-hoc — sem editar o repositório:

```bash
comms agent-add dba --descricao "tuning de queries e índices" --por arch
comms brief dba
comms post --de dba --para arch --tipo pergunta --titulo "..." --corpo "..."
```

`agent-set` atualiza uma persona ad-hoc, `agent-show` inspeciona seu prompt e `agent-rm` a aposenta preservando o histórico. Para criar um padrão versionado, edite diretamente `docs/prompts/<nome>.md`.

## Passo 4: Fluxo de entrega completo

```bash
# 1. Sincronizar o Board:
comms board-sync

# 2. Iniciar issue (Board -> In progress + Worktree efêmero + comms.db):
comms start 42

# 3. Desenvolver e testar na worktree criada em .worktrees/issue-42-...:
$(comms testcmd)

# 4. Toda finding do review tem que ter destino antes do PR:
comms gate --refs issue:42

# 5. Enviar PR e finalizar entrega:
git push -u origin HEAD
gh pr create --title "feat: minha feature (#42)" --body "..."
comms finish 42 --pr "https://github.com/usuario/repo/pull/123"
```

Para consumo por outros agentes, acrescente `--json` antes ou depois de qualquer subcomando. Erros estruturados saem em stderr; exit `1` indica erro operacional ou gate reprovado, e exit `2` indica erro de uso.
