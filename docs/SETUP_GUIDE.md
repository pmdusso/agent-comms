# Guia de Configuração e Uso do Agent Comms em Novos Projetos

Este guia explica como integrar a infraestrutura de comunicação e orquestração **Agent Comms** em qualquer novo repositório ou projeto existente na máquina.

---

## 🛠️ Passo 1: Instalação no Repositório Target

No terminal, execute o script de instalação apontando para o seu projeto:

```bash
/Users/pmdusso/code/shared/agent-comms/install.sh ~/code/meu-novo-projeto
```

---

## 📋 Passo 2: Adicionar a Regra de Ouro no `AGENTS.md` do Projeto

Crie ou edite o arquivo `AGENTS.md` (ou `AGENTS.md` global) do seu novo projeto e adicione a seção de comunicação:

```markdown
## Comunicação & Separação de Canais

Ao iniciar uma sessão neste repositório:
1. Execute `./comms.py state`.
2. Leia os turnos abertos com `./comms.py turns`.
3. Sincronize o Board do GitHub com o banco local via `./comms.py board-sync`.
4. Use o papel atribuído: `arch`, `pm`, `dev`, `tester` ou `reviewer`.
5. Registre decisões, bloqueios e handoffs no `comms.db`.
6. Resolva os turnos atendidos com `./comms.py resolve <ID>`.

### Regra de Ouro de Canais

- **GitHub Projects Board / Issues / PRs / Chat da Sessão:** Canal Humano Oficial.
- **`comms.db`:** Canal Operacional Local (Agente ↔ Agente).

### Orquestração Unificada

- **Ao iniciar uma issue:** `./comms.py start <ISSUE_NUMBER>`
- **Ao concluir e abrir PR:** `./comms.py finish <ISSUE_NUMBER> --pr <PR_URL>`
```

---

## 🚀 Passo 3: Iniciar as Personas nas Sessões de Agentes

Cada agente (ou aba de terminal / subagente) deve assumir uma persona usando os one-liners dos prompts em `docs/prompts/`:

### **Prompt para Agente Implementador (`dev`)**
```text
Você é dev neste repositório. Entre na worktree criada pelo ./comms.py start, leia docs/prompts/dev.md, execute ./comms.py state e implemente a funcionalidade mantendo a suíte de testes verde.
```

### **Prompt para Agente Validador (`tester`)**
```text
Você é tester neste repositório. Entre na worktree do card, leia docs/prompts/tester.md, execute a suíte de testes e confirme 100% de aprovação e cobertura.
```

### **Prompt para Agente Auditor (`reviewer`)**
```text
Você é reviewer neste repositório. Entre na worktree do card, leia docs/prompts/reviewer.md, audite as modificações de código e busque por potenciais regressões ou sobre-engenharia.
```

---

## 🎯 Passo 4: Fluxo de Entrega Completo

```bash
# 1. Sincronizar o Board:
./comms.py board-sync

# 2. Iniciar issue (Board -> In progress + Worktree efêmero + comms.db):
./comms.py start 42

# 3. Desenvolver e testar na worktree criada em .worktrees/issue-42-...:
make test          # ou a suíte do seu repo: ./test-all, uv run pytest, npm test

# 4. Enviar PR e finalizar entrega:
git push -u origin HEAD
gh pr create --title "feat: minha feature (#42)" --body "..."
./comms.py finish 42 --pr "https://github.com/usuario/repo/pull/123"
```
