# Guia de Configuração e Uso do Agent Comms em Novos Projetos

Este guia explica como integrar a infraestrutura de comunicação e orquestração **Agent Comms** em qualquer novo repositório ou projeto existente na máquina.

---

## 🛠️ Passo 1: Instalação no Repositório Target

Clone o pacote uma vez e aponte o installer para o seu projeto:

```bash
git clone git@github.com:pmdusso/agent-comms.git ~/code/shared/agent-comms
~/code/shared/agent-comms/install.sh ~/code/meu-novo-projeto
```

Para atualizar o engine depois, re-rode o installer: ele sobrescreve o
`comms.py` e **preserva** prompts e skills editados localmente. Use
`--update` se quiser sobrescrever esses também.

---

## 📋 Passo 2: Adicionar a Regra de Ouro no `AGENTS.md` do Projeto

Crie ou edite o arquivo `AGENTS.md` (ou `AGENTS.md` global) do seu novo projeto e adicione a seção de comunicação:

```markdown
## Comunicação & Separação de Canais

Ao iniciar uma sessão neste repositório:
1. Execute `./comms.py brief <seu-papel>` — traz prompt, estado e seus turnos.
2. Sincronize o Board do GitHub com o banco local via `./comms.py board-sync`.
3. Os papéis válidos são os de `./comms.py agents` (= um arquivo em `docs/prompts/`).
4. Registre decisões, bloqueios e handoffs no `comms.db`.
5. Resolva os turnos atendidos com `./comms.py resolve <ID>`.

### Regra de Ouro de Canais

- **GitHub Projects Board / Issues / PRs / Chat da Sessão:** Canal Humano Oficial.
- **`comms.db`:** Canal Operacional Local (Agente ↔ Agente).

### Orquestração Unificada

- **Ao iniciar uma issue:** `./comms.py start <ISSUE_NUMBER>`
- **Ao concluir e abrir PR:** `./comms.py finish <ISSUE_NUMBER> --pr <PR_URL>`
```

---

## 🚀 Passo 3: Subir as Sessões de Agentes

Um terminal por agente. `brief` monta o contexto inteiro — prompt, estado do
canal, itens do agente e o corpo dos turnos endereçados a ele:

```bash
./comms.py agents               # quem existe

./comms.py brief dev            # copiar e colar na sessão
./comms.py brief tester | claude -p
./comms.py brief reviewer | gemini -p
```

Não há lista fixa de personas aqui: o roster é `docs/prompts/`. Para subir um
agente que ainda não existe, crie o prompt dele e ele já pode postar:

```bash
cp docs/prompts/dev.md docs/prompts/dba.md   # e edite
./comms.py brief dba
```

---

## 🎯 Passo 4: Fluxo de Entrega Completo

```bash
# 1. Sincronizar o Board:
./comms.py board-sync

# 2. Iniciar issue (Board -> In progress + Worktree efêmero + comms.db):
./comms.py start 42

# 3. Desenvolver e testar na worktree criada em .worktrees/issue-42-...:
$(./comms.py testcmd)   # make test / uv run pytest / npm test — conforme o repo

# 4. Toda finding do review tem que ter destino antes do PR (exit 1 se não):
./comms.py gate --refs issue:42

# 5. Enviar PR e finalizar entrega:
git push -u origin HEAD
gh pr create --title "feat: minha feature (#42)" --body "..."
./comms.py finish 42 --pr "https://github.com/usuario/repo/pull/123"
```
