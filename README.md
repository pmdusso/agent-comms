# Agent Comms & Orchestration Infrastructure

**Agent Comms** é a infraestrutura de comunicação, orquestração, skills de pipeline e gates de qualidade para agentes AI de programação.

O engine `comms` é um binário Rust global. O estado operacional fica em SQLite (`comms.db`); o GitHub Projects Board continua sendo o canal humano oficial. Nenhum ID de repositório ou de Project fica embutido no código: as chamadas GraphQL do `gh` resolvem os IDs em runtime.

## Recursos

- Turnos, decisões, bloqueios, handoffs, work items e chaves de estado em SQLite WAL.
- Roster versionado em `docs/prompts/*.md` mais personas ad-hoc registradas no banco.
- `start`/`finish` integrando Board, worktrees, handoffs e gates.
- Saída texto para humanos e `--json` global com erros estruturados para agentes.
- Skills `discovery` e `delivery`, sem duplicar esses fluxos como remetentes do canal.
- Gate executável: findings abertas ou débito incompatível com a severidade reprovam o comando.

## Estrutura

```text
agent-comms/
├── Cargo.toml              # Pacote comms v5.0.0
├── Cargo.lock              # Dependências reproduzíveis
├── src/                    # CLI, SQLite, roster, saída, Git e GitHub
├── templates/              # Manual, pipeline e prompt ad-hoc embutidos
├── tests/                  # Integração, regressões e lint documental
├── install.sh              # Build, instalação e upgrade
├── prompts/                # Roster padrão: um arquivo por agente
├── skills/
│   ├── discovery/
│   └── delivery/
└── docs/
    └── SETUP_GUIDE.md
```

`docs/core/AGENT_COMMS.md` e `docs/core/PIPELINE.md` são gerados no repositório de destino pelos templates embutidos no binário.

## Instalação

Pré-requisitos: Rust/Cargo via `rustup` e GitHub CLI `gh` autenticado.

```bash
rustup toolchain install stable --profile default
git clone git@github.com:pmdusso/agent-comms.git ~/code/shared/agent-comms
~/code/shared/agent-comms/install.sh /caminho/do/seu-projeto
```

O installer:

1. Executa `cargo build --release --locked` e instala `comms` em `~/.local/bin` (ou em `COMMS_BIN_DIR`).
2. Copia os prompts e o guia, instala as skills em `.claude/skills/` e cria symlinks em `.cursor/skills/` e `.agents/skills/`.
3. Executa `comms bootstrap` e `comms migrate` no destino.
4. Avisa se o diretório do binário não estiver no `PATH` e lista instruções locais que ainda chamam o engine antigo.

Para atualizar uma instalação que ainda contém o script Python legado, é obrigatório usar `--update`: o installer remove esse script, regera os manuais e mantém as versões anteriores no Git. Sem essa flag, ele sai antes de copiar qualquer arquivo.

## Qual `comms.db`?

A descoberta parte do diretório atual:

1. `COMMS_DB` não vazio tem precedência; caminho relativo é resolvido contra o diretório atual.
2. Sem override, o `comms` localiza a raiz Git principal sem spawn, inclusive ao entrar por uma worktree ligada.
3. Ele sobe da raiz Git (ou do diretório atual fora de Git) até a raiz do filesystem e usa o primeiro `comms.db` encontrado.
4. Sem banco, comandos que dependem dele falham com `db_missing`; apenas `bootstrap` ou um banco explícito em `COMMS_DB` pode criá-lo.

`trunk` e `testcmd` não exigem banco. `trunk`, `testcmd` e `start` operam na raiz Git principal do diretório atual. O `bootstrap` grava banco, `docs/core/` e `.gitignore` no diretório do banco, não necessariamente no diretório de onde foi chamado.

## Roster de agentes

O roster é a união de duas fontes:

- **Padrão, versionado:** `docs/prompts/<nome>.md`; editar a persona é editar o arquivo.
- **Ad-hoc, em runtime:** linhas da tabela `agents` do `comms.db`, sem alterar o repositório.

Se o mesmo nome existir nas duas fontes, o arquivo vence e `comms agents` mostra o conflito. `human` sempre entra no roster; `todos` é apenas destinatário e `cross` apenas escopo.

```bash
comms agents
comms agent-add dba --descricao "tuning de queries e índices" --por arch
comms agent-add dev-front --descricao "..." --por arch --prompt-file prompt.md
comms brief dba
comms agent-show dba --adhoc
comms agent-set dba --prompt-file novo.md
comms agent-rm dba                 # preserva turnos e itens históricos
```

Prompts fornecidos precisam das seções `## Antes de agir`, `## Fluxo`, `## Limites` e `## Término`, com no máximo cinco ocorrências de `NUNCA`; `--force` registra uma exceção consciente. Para promover uma persona útil ao padrão:

```bash
comms agent-show dba --adhoc > dba.md.tmp
mv dba.md.tmp docs/prompts/dba.md
comms agent-rm dba
```

Bancos anteriores à v4 podem conter CHECKs de roster. `comms migrate` recria as tabelas preservando turnos e work items. Em repos v2 com papéis embutidos na antiga tupla, esses papéis aparecem como órfãos após o upgrade; registre os que continuam em uso com `comms agent-add <nome> --descricao "..." --por human`.

## Board GitHub Projects

Defaults: project `1`, owner `@me`, repo do diretório atual. Uma query GraphQL (`gh api graphql`) resolve item, project, campo `Status` e opções por issue; `board-sync` pagina todos os cards, sem o antigo teto de 100.

```bash
comms set board.project 3
comms set board.owner minha-org
comms set board.repo dono/repo
comms board-item <N>
comms board-set <N> Ready
comms board-sync
```

O Board precisa de um campo `Status` de seleção única. Colunas inexistentes retornam erro com as opções disponíveis.

O tronco vem de `origin/HEAD`, com fallback para branches locais; `comms trunk` evita assumir um nome fixo. A suíte vem do kv `test.cmd` ou das convenções em `Makefile`, `pyproject.toml`, `package.json`, `Cargo.toml` e `go.mod`, nessa ordem:

```bash
git diff $(comms trunk)...HEAD
$(comms testcmd)
comms set test.cmd "just ci"
```

## Fluxo de entrega

```bash
comms start <ISSUE_NUMBER>
# implementar o plano e validar na worktree
$(comms testcmd)
comms gate --refs issue:42
git push -u origin HEAD
gh pr create --title "feat: minha feature (#42)" --body "..."
comms finish 42 --pr "https://github.com/usuario/repo/pull/123"
```

`start` move o card para `In progress`, cria a worktree quando solicitado e registra work item + handoff na mesma transação. `finish` move para `In review`, conclui apenas refs com o token exato da issue, registra o gate e comenta na issue. Merge e deploy continuam sendo decisões humanas.

Para subir uma sessão:

```bash
comms agents
comms brief dev
comms turns --para dev
comms resolve <ID>
```

## Saída para agentes (`--json`)

A flag global funciona antes ou depois do subcomando. Em sucesso, stdout contém um único documento JSON compacto; avisos entram em `warnings`. Comandos sem documento próprio produzem `{}`.

Em erro, stdout fica vazio e stderr recebe:

```json
{"error":{"code":"not_found","message":"turno 999 não existe"}}
```

Códigos de erro estruturados: `usage`, `invalid`, `not_found`, `db_missing`, `guard`, `conflict`, `external` e `internal`. Exit codes:

- `0`: sucesso, inclusive `resolve` de turno já resolvido (com aviso).
- `1`: erro operacional ou gate reprovado com relatório.
- `2`: erro de uso, incluindo papel ou escopo desconhecido.

Exemplos:

```bash
comms --json state
comms --json show 999
comms post --de dba --para arch --tipo mensagem --titulo oi --corpo x --json
```

## Desenvolvimento

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

A suíte cobre descoberta de banco/worktrees, canal, guards, roster, migração, bootstrap, gate, GitHub com `gh` falso e portabilidade dos documentos. O antigo self-test do CLI foi substituído por esses testes.

## Licença

MIT — veja [LICENSE](LICENSE).
