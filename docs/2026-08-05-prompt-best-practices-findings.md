#needs-peer-review

# Melhores práticas para os prompts de papel — findings e gap analysis

**Data:** 2026-08-05
**Contexto:** Exploração web (3 agentes de pesquisa em paralelo) sobre (1) guias oficiais de agent prompting — Anthropic, OpenAI, Google; (2) system prompts reais de coding agents em produção — Claude Code, Codex CLI, SWE-agent, Aider, OpenHands, Cursor, Devin, Amp, Windsurf; (3) lições documentadas de orquestração multi-agente — Anthropic, Cognition, MAST/Berkeley, MetaGPT, ChatDev, LangChain. Objetivo: gap analysis contra os 6 prompts de `prompts/`.

---

## O que a pesquisa VALIDOU (não mexer)

O desenho atual bate com as práticas documentadas em todos os pontos estruturais:

- **Bootstrap fino + estado via `brief`** = o padrão "thin system prompt, rich instance prompt" do SWE-agent. Papel no prompt; ticket/estado buscados frescos do bus/board. Nunca colocar fatos de projeto no prompt (Anthropic context engineering; LangChain).
- **Bus tipado (`decisao`/`gate`/`bloqueio`)** = MetaGPT: mensagens estruturadas entre agentes reduzem alucinação vs. diálogo livre ("idle chatter").
- **Handoffs sequenciais, sem devs paralelos** = Cognition ("Don't build multi-agents") + Anthropic: código é o domínio que menos se beneficia de paralelismo.
- **Gatekeeper + merge exclusivo do humano** = a intervenção de maior ganho medido no MAST (+9.4% exigir aprovação de término por papel designado; +15.6% verificação contra o objetivo original).
- **Orçamento de 30–50 linhas por prompt** = mini-SWE-agent prova que ~1.200 palavras bastam; critério Anthropic: "remover esta linha causaria erro? Se não, corte."

## O mito que a pesquisa derrubou

**Mito:** o risco principal de prompts de papel é o agente "sair do personagem" (role drift).
**Fato:** no MAST (1.600+ traces, NeurIPS 2025), "disobey role specification" é só **1,5%** das falhas. As falhas dominantes são **especificação ambígua da tarefa (11,8%)**, **não saber quando terminou (12,4%)**, **verificação fraca (21%)** e **repetição de passos (15,7%)**. 42% do total é design de sistema, não fraqueza de modelo.
**Implicação:** gastar linhas em critérios de aceite, condição de término e escalação — não em mais policiamento de identidade.

---

## Mudanças propostas — transversais (todos os 6 prompts)

| # | Mudança | Evidência |
|---|---------|-----------|
| T1 | **Condição de término explícita + ato terminal canônico.** Cada prompt termina nomeando o estado exato de "pronto" e o comando que o materializa (ex.: dev: `./comms.py finish` + turno resolvido). | MAST 12,4% "unaware of termination"; mini-SWE-agent completion signal |
| T2 | **"Pergunte antes de assumir."** Se o handoff/card estiver ambíguo, poste `pergunta`/`bloqueio` ANTES de começar — nunca prossiga por suposição; se decidir prosseguir com uma premissa, registre-a no bus. | ChatDev "communicative dehallucination"; MAST 6,8%; GPT-5 guide |
| T3 | **Escalação com limite numérico.** "Após 3 tentativas falhas no mesmo obstáculo, poste `bloqueio` e pare" — condição contável, não vibes. | Cursor (3 loops de linter); OpenAI practical guide (failure thresholds) |
| T4 | **Decisões são vinculantes.** Antes de agir, ler as `decisao` abertas e tratá-las como restrição — evita saídas incompatíveis entre papéis. | Cognition: decisões implícitas conflitantes são o modo de falha nº 1 |
| T5 | **Evidência, não afirmação.** Toda mensagem de status/gate no bus carrega evidência (comando + saída resumida, arquivo:linha) e é condensada (cap de linhas). Referencie artefatos por caminho/PR/SHA — encaminhe, não parafraseie. | Claude Code best practices; LangChain (supervisor "telephone" perde ~50%); Anthropic multi-agent |
| T6 | **Hierarquia de ênfase.** No máx. 3–5 proibições absolutas (NUNCA/CAPS) por prompt, reservadas a ações destrutivas/irreversíveis (merge, commit sem autorização, segredos). O resto em frase positiva ("Prioridades vêm do pm; discordou → mensagem ao pm") com o "porquê" em uma oração quando a regra for contraintuitiva. | Anthropic 4.x best practices (positivo + motivação; menos MUST/CRITICAL); convergência dos 10 prompts de produção (CAPS só nas ~5 regras top) |
| T7 | **Ritual anti-drift barato.** Antes de cada post de `handoff`/`gate`, releia seu próprio prompt (1 linha no fechamento). | Persona drift >30% após 8–12 turnos (arXiv 2412.00804); "inception prompting" do ChatDev |

## Mudanças propostas — por prompt

- **`dev.md`**
  - Scope clamp estilo Aider (a frase mais copiada da indústria): "Faça o que o card pede, nada mais. Não melhore, comente ou corrija partes não relacionadas."
  - Anti-test-gaming: "Solução correta para todas as entradas válidas, não só para os testes; teste incorreto ou tarefa inviável → informe em vez de contornar."
  - Etiqueta git (3 linhas): sem force push, sem commit direto no tronco, branch da worktree criada pelo `start`.
  - Branch de caso-de-borda: "Suíte já vermelha ao pegar o card → `bloqueio` para arch, não conserte silenciosamente."
- **`arch.md`**
  - Heurística de dimensionamento: quando um card dispensa o pipeline completo (typo de docs → dev direto) vs. quando exige todos os gates — sem isso o coordenador sobre-orquestra trabalho trivial (Anthropic mediu 15× de overhead).
  - Encaminhar, não resumir: ao rotear saída de um papel a outro, aponte para o artefato original (PR/arquivo/mensagem), nunca reescreva.
  - Cap de rodadas: ciclo reviewer↔dev máx. 3; depois, escalar a `human`.
- **`pm.md`**
  - Template de handoff com campos obrigatórios: objetivo, critérios de aceite verificáveis, fronteiras (o que NÃO tocar), formato de saída esperado. (Anthropic multi-agent: delegação vaga é a falha nº 1 de orquestração.)
- **`tester.md`**
  - Verificar contra o texto original do card, critério por critério, citando-os na mensagem de `gate` — nunca contra o resumo do dev. (Maior ganho isolado medido no MAST: +15,6%.)
- **`reviewer.md`**
  - Anti-churn: "Aponte só lacunas que afetem correção ou requisitos declarados — um revisor instruído a achar problemas sempre acha algum." (Anthropic 4.x best practices.)
  - "Nunca opine sobre código que não abriu; cite arquivo:linha do que leu."
- **`gatekeeper.md`**
  - Já é o prompt mais forte (double-check independente, evidência, veredito binário). Só alinhar com T1–T7 e adicionar autocorreção: "Aprovou sem evidência re-executada por você → reabra e corrija no próximo turno." (Cursor.)

## Estruturais (opcionais, prioridade média)

- **Headers de seção explícitos** nos prompts (`## Papel`, `## Antes de agir`, `## Fluxo`, `## Limites`, `## Término`) — custo zero, melhora parsing (Anthropic, Google, Cursor/OpenHands usam containers nomeados).
- **Esqueleto compartilhado**: os 6 prompts repetem o mesmo bloco "Antes de agir" à mão. O padrão Codex (fragmentos compostos) resolveria drift, mas adiciona máquina — YAGNI enquanto o `check()` puder guardar a consistência.

## Follow-ups fora dos prompts — IMPLEMENTADOS (2026-08-05)

1. **Template de handoff imposto no `comms.py`** ✅ — `cmd_post` rejeita `--tipo handoff` cujo corpo não nomeie `Objetivo:`, `Critérios:` e `Fronteiras:` (acento-insensível; `--force` como escape consciente). Código > prompt para regra sem exceção (Anthropic: advisory vs deterministic). Handoffs gerados internamente por `start`/`finish` não passam pelo validador.
2. **Lint de coerência no `check()`** ✅ — versão determinística do "lint de contradições": todo prompt tem o esqueleto completo (`## Antes de agir`/`## Fluxo`/`## Limites`/`## Término`), no máximo 5 `NUNCA` (orçamento de ênfase), e o `pm.md` cita os mesmos campos que o `cmd_post` exige (sincronia prompt↔código). Contradição *semântica* continua sendo julgamento — auditada à mão nesta rodada (a do `dev.md` commit vs. fluxo foi resolvida).
3. **Campo "alternativas rejeitadas"** ✅ — no `## Término` do `dev.md`, o handoff de conclusão pede alternativas rejeitadas junto com evidência e riscos (é o que resumos ingênuos sempre dropam — Cognition/Anthropic). Fica como conteúdo pedido pelo prompt, não campo imposto no código: nem todo handoff rejeitou alternativas.

---

## Fontes principais

- Anthropic: [Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) · [Built multi-agent research system](https://www.anthropic.com/engineering/built-multi-agent-research-system) · [Claude Code best practices](https://code.claude.com/docs/en/best-practices) · [Claude 4.x prompting best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-4-best-practices) · [Building effective agents](https://www.anthropic.com/engineering/building-effective-agents)
- OpenAI: [GPT-5 prompting guide](https://developers.openai.com/cookbook/examples/gpt-5/gpt-5_prompting_guide) · [A practical guide to building agents (PDF)](https://cdn.openai.com/business-guides-and-resources/a-practical-guide-to-building-agents.pdf)
- Google: [Gemini prompt design strategies](https://ai.google.dev/gemini-api/docs/prompting-strategies)
- Pesquisa: [MAST — Why Do Multi-Agent LLM Systems Fail? (arXiv 2503.13657)](https://arxiv.org/abs/2503.13657) · [MetaGPT (arXiv 2308.00352)](https://arxiv.org/pdf/2308.00352) · [ChatDev (arXiv 2307.07924)](https://arxiv.org/abs/2307.07924) · [Agentless (arXiv 2407.01489)](https://arxiv.org/abs/2407.01489) · [Identity drift (arXiv 2412.00804)](https://arxiv.org/pdf/2412.00804)
- Indústria: [Cognition — Don't Build Multi-Agents](https://cognition.com/blog/dont-build-multi-agents) · [LangChain — Benchmarking multi-agent architectures](https://www.langchain.com/blog/benchmarking-multi-agent-architectures) · Prompts de produção via [system-prompts-and-models-of-ai-tools](https://github.com/x1xhlol/system-prompts-and-models-of-ai-tools), [SWE-agent](https://github.com/SWE-agent/mini-swe-agent/blob/main/src/minisweagent/config/mini.yaml), [Aider](https://github.com/Aider-AI/aider/blob/main/aider/coders/editblock_prompts.py), [OpenHands](https://github.com/All-Hands-AI/OpenHands/blob/0.30.0/openhands/agenthub/codeact_agent/prompts/system_prompt.j2), [Codex CLI](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/prompts/base_instructions/default.md)
