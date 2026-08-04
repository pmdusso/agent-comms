# Agent Comms & Orchestration Infrastructure

**Agent Comms** é a infraestrutura de comunicação, orquestração, skills de pipeline e gates de qualidade para agentes AI de programação (Antigravity, Gemini CLI, Claude Code, Cursor, Codex).

Portável por construção: nenhum ID de repositório ou de GitHub Project vive no código — tudo é resolvido pela API do `gh` em tempo de execução.

Ela permite rodar sessões multi-agente eficientes, paralelas e determinísticas, com **dupla camada de sincronia** entre o trabalho humano (GitHub Projects Board) e o canal operacional local (`comms.db`).

---

## 🌟 Recursos Principais

- 🔄 **Orquestração Unificada:** Comandos CLI únicos (`./comms.py start`, `./comms.py finish`, `./comms.py board-sync`) que automatizam movimentação de cards no Board, criação de Git Worktrees efêmeros e registros de handoff.
- 🗄️ **Banco SQLite Durável (`comms.db`):** Armazena turnos, decisões, bloqueios, itens pendentes e históricos de gates entre personas de agentes sem poluir o repositório Git.
- 🛠️ **Skills Autónomas de Pipeline:**
  - **`discovery`:** Automação do funil F1→F2 (Backlog ➔ Ready). Triagem, alinhamento, especificação, invariantes e publicação do card como `Ready`.
  - **`delivery`:** Automação do funil F3→F5 (Ready ➔ In review / PR). Worktree efêmera, ciclo TDD/testes, revisão de código, CodeRabbit, abertura de PR e movimentação do board.
- 🎭 **Prompts de Personas Especializadas:**
  - `arch.md`: Arquiteto responsável pelas decisões de design, invariantes e diretivas.
  - `pm.md`: Product Manager responsável pelos refinamentos de backlog e PRD.
  - `dev.md`: Desenvolvedor focado em implementação limpa e sem over-engineering.
  - `tester.md`: Validador de testes e garantidor do piso de cobertura de testes.
  - `reviewer.md`: Auditor independente de qualidade e busca de regressões.
  - `gatekeeper.md`: Validador final de CI, PRs e autorização de merge.
  - `discovery.md`: Condução do funil Backlog ➔ Ready.
- 🚦 **Sistema de Gates Unificado:** Validação de transições de entrega (G1..G5) exigindo verificação de testes verdes e cobertura mínima antes de autorizar merge.

---

## 📂 Estrutura do Repositório Exportado

```text
/Users/pmdusso/code/shared/agent-comms/
├── comms.py                # Engine CLI principal de orquestração e banco SQLite
├── install.sh              # Script de installation de 1 linha para novos repositórios
├── README.md               # Documentação principal
├── skills/                 # Skills autónomas registradas para agentes AI
│   ├── discovery/          # Skill de Discovery (Backlog -> Ready)
│   └── delivery/           # Skill de Delivery (Ready -> In review / PR)
├── prompts/                # Prompts de personas prontas para colar nas sessões
│   ├── arch.md
│   ├── dev.md
│   ├── discovery.md
│   ├── gatekeeper.md
│   ├── pm.md
│   ├── reviewer.md
│   └── tester.md
├── docs/                   # Especificações de protocolo e guiagem
│   └── SETUP_GUIDE.md      # Guia de instalação e configuração rápida
                            # AGENT_COMMS.md e PIPELINE.md são GERADOS pelo
                            # bootstrap, a partir dos templates no comms.py
└── scripts/                # Scripts utilitários de suporte
    └── start.sh            # Script de inicialização da sessão
```

---

## 📦 Como Instalar em um Novo Projeto

Para equipar qualquer repositório existente ou novo com a infraestrutura de comunicação e skills:

```bash
/Users/pmdusso/code/shared/agent-comms/install.sh /caminho/do/seu-projeto
```

O instalador vai:
1. Copiar `comms.py` para a raiz do seu repositório.
2. Copiar os prompts para `docs/prompts/` e o `SETUP_GUIDE.md` para `docs/core/`.
3. Instalar as skills `discovery` e `delivery` em `.claude/skills/`, com symlink em `.cursor/skills/` e `.agents/skills/`.
4. Rodar `comms.py bootstrap`: cria o `comms.db` (WAL), gera `docs/core/AGENT_COMMS.md` e `docs/core/PIPELINE.md` refletindo os `SENDERS`/`ROLES` daquele repositório, e adiciona o banco ao `.gitignore`.

Documentos já existentes nunca são sobrescritos pelo bootstrap.

---

## 👥 Roster de agentes (onboarding ad-hoc)

**O roster é `docs/prompts/`.** Um arquivo `.md` = um agente autorizado a postar no
canal. Subir um agente novo no meio da sessão não exige editar código nem recriar o
banco:

```bash
cp docs/prompts/dev.md docs/prompts/dba.md   # e edite o prompt
./comms.py agents                            # roster + turnos por agente
./comms.py post --de dba --para arch --tipo pergunta --titulo "..." --corpo "..."
```

`sender`, `recipient` e `work_items.owner` são `TEXT` sem `CHECK` justamente para o
roster poder crescer em runtime. A proteção contra typo fica no argparse, que relê o
roster a cada invocação — `--de dvv` continua sendo rejeitado listando os válidos.

`human` está sempre no roster (é gente, não tem prompt) e `todos` só existe como
destinatário. `kind` e `status` seguem fechados por `CHECK`: são política, não roster.

**Bancos criados antes da v4** congelam o roster no DDL e rejeitam agentes novos:

```bash
./comms.py migrate    # remove os CHECKs preservando turnos e work items
```

---

## 🎛️ Configuração do Board

Defaults: project `1`, owner `@me`, repo do diretório atual. Nenhum ID é
hardcoded — `comms.py` resolve project-id, field-id do `Status` e os option-ids
via `gh project view` / `gh project field-list`. Só configure o que fugir do default:

```bash
./comms.py set board.project 3          # número do GitHub Project
./comms.py set board.owner minha-org    # @me ou org
./comms.py set board.repo dono/repo     # útil quando o repo não está clonado local

./comms.py board-item <N>               # item-id + coluna atual do card
./comms.py board-set <N> Ready          # move o card de coluna
```

O board precisa de um campo `Status`; colunas inexistentes retornam erro
listando as disponíveis.

---

## 💻 Guia Rápido de Comandos CLI (`comms.py`)

### 1. Iniciar Trabalho em um Card
Conecta o card do GitHub Projects, move para `In progress`, cria um Git Worktree efêmero e registra o handoff no `comms.db`:
```bash
./comms.py start <ISSUE_NUMBER>
```

### 2. Concluir Trabalho e Abrir PR
Move o card no GitHub Projects para `In review`, comenta na Issue, abre a PR e grava o gate de aprovação no `comms.db`:
```bash
./comms.py finish <ISSUE_NUMBER> --pr "https://github.com/usuario/repo/pull/<PR_NUMBER>"
```

### 3. Sincronizar Board ↔ Banco Local
Sincroniza os status das issues do GitHub com o `comms.db`:
```bash
./comms.py board-sync
```

### 4. Consultar o Estado Atual
Exibe resumos, itens pendentes e turnos abertos:
```bash
./comms.py state
./comms.py turns
```

---

## ⚖️ Licença
MIT — veja [LICENSE](LICENSE).
