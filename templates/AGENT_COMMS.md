# AGENT_COMMS — Canal de Comunicação Estruturado (v{{VERSION}})

Este arquivo serve como manual técnico de instrução para os agentes operarem no canal de comunicação do repositório.

## Como Funciona
As reuniões estruturadas, pendências e chaves de estado de progresso deste repositório vivem em `comms.db` (SQLite executado sob PRAGMA WAL).

Sempre que um agente ou humano iniciar uma sessão ou turno neste repositório, ele **deve**:
1. Rodar `comms state` para sincronizar e carregar todo o contexto local.
2. Ler turnos abertos usando `comms turns`.
3. Responder as solicitações e fechar turnos respondidos com `comms resolve <ID>`.

---

## Estrutura do Canal
* **Remetentes (Senders):** {{SENDERS}} — derivado de `docs/prompts/*.md` (padrão) + tabela `agents` (ad-hoc)
* **Destinatários (Recipients):** os senders + `todos` (broadcast, nunca remetente)
* **Tipos de Turnos (Kinds):** {{KINDS}}
* **Status de Tarefas (Statuses):** {{STATUSES}}
* **Escopos de Trabalho (Scopes):** os senders + `cross`

## Onboardear um agente ad-hoc

O roster tem duas fontes:

- **Padrão (versionado):** um arquivo por papel em `docs/prompts/` — editar a persona é editar o arquivo.
- **Ad-hoc (runtime):** linhas da tabela `agents` do `comms.db`, criadas por qualquer agente do roster, sem tocar no repositório.

```bash
comms agent-add dba --descricao "tuning de queries e índices" --por arch   # prompt gerado do template
comms agent-add dev-front --descricao "..." --por arch --prompt-file prompt.md
comms agents                       # roster, origem e turnos
comms brief dba                    # prompt + estado para subir a sessão
comms agent-set dba --prompt-file novo.md
comms agent-rm dba                 # aposenta; histórico preservado
```

Prompt fornecido precisa das seções `## Antes de agir`, `## Fluxo`, `## Limites` e `## Término` (no máximo 5 `NUNCA`); `--force` aceita exceção consciente. Nome ad-hoc não repete um padrão. Persona que se provou útil vira padrão: `comms agent-show dba --adhoc > dba.md.tmp && mv dba.md.tmp docs/prompts/dba.md && comms agent-rm dba`.

Não há CHECK de roster no schema. Bancos criados antes da v4 congelam o roster no DDL — `comms migrate` libera preservando turnos e itens.

## Responsabilidades

Papéis padrão. Agentes ad-hoc adicionados depois não entram nesta tabela — o
roster vivo é `comms agents`.

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
comms state                             # Ver estado completo consolidado
comms post --de <REMETENTE> --para <DESTINATARIO> --tipo <TIPO> --titulo "..." --corpo "..."
comms turns                             # Listar turnos pendentes
comms show <ID>                         # Ver corpo do turno completo
comms resolve <ID>                      # Marcar turno resolvido
comms items                             # Listar itens de trabalho
comms item-add --titulo "..." --dono <RESPONSAVEL>
comms brief <PAPEL>                     # Prompt + estado para subir uma sessão
comms agents                            # Roster padrão + ad-hoc
comms --json state                      # saída para agentes
```

> **Regra de ouro:** auto-contido. Um agente novo, sem memória, entende o estado atual com `comms state` e este manual.
> **LGPD/PII:** nunca registrar telefone, CPF, coordenada bruta de cliente ou qualquer PII. Referenciar por IDs lógicos. O `comms` possui guarda automático que rejeita sequências de 10+ dígitos.
