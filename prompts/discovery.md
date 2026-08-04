# Guia de Execução — Fase de Discovery (Idealização ➔ Spec ➔ Card Ready)

A fase de **Discovery** tem como objetivo transformar necessidades de negócio, ideias de UX ou débitos técnicos em especificações claras, testáveis e prontas para implementação (**Status `Ready`**).

---

## 🎯 Objetivos do Discovery

1. **Entendimento & Alinhamento:** Definir o problema real do usuário e a solução mínima viável.
2. **Análise de Viabilidade Técnica:** Avaliar impactos em arquitetura, banco de dados, APIs de terceiros e orçamento de rede (ex: regra Zero-GET/Summary-First).
3. **Especificação & Invariantes:** Escrever a spec técnica em `docs/specs/` ou na própria Issue, definindo critérios de aceite verificáveis e complexidade ciclomática esperada ($C901 \le 10$).
4. **Prontidão (Ready):** Promover o card no GitHub Projects Board de `Backlog` para `Ready` e registrar a diretiva de arquitetura no `comms.db`.

---

## 📋 Checklist de Execução do Discovery

### 1. Pesquisa & Inspeção de Código
- [ ] Consulte o Knowledge Graph do projeto (ex: `graphify query "<tema>"`).
- [ ] Inspecione os pontos de toque no código e schemas existentes sem tirar conclusões precipitadas.
- [ ] Verifique se existem premissas declaradas nos manuais de arquitetura (`docs/architecture/` ou `docs/specs/`).

### 2. Elaboração da Especificação
- [ ] Crie ou atualize o documento de spec em `docs/specs/YYYY-MM-DD-titulo-design.md`.
- [ ] Declare explicitamente:
  - **Contexto & Escopo.**
  - **Budget de Rede / GETs:** Explicitar o pior caso de chamadas HTTP externas por fluxo.
  - **Postura de Dados & Privacidade (PII):** Garantir cifragem de segredos e ausência de PII em logs.
  - **Invariantes & Casos de Borda:** Como o sistema se comporta sob falha, timeout ou retry.

### 3. Registro e Transição para Ready
- [ ] Crie a Issue no GitHub: `gh issue create --title "..." --body "..." --label "horizon/now,area/ux"`.
- [ ] Sincronize o Board e o banco local: `./comms.py board-sync`.
- [ ] Registre a diretiva arquitetural no canal operacional:
  ```bash
  ./comms.py post --de arch --para todos --tipo decisao --titulo "Diretiva ARCH #N: <Titulo>" --corpo "<Instrucoes>" --refs "issue:<N>"
  ```

---

## 🤖 Persona de Referência
Siga as instruções do prompt de persona em [`docs/prompts/arch.md`](file:///Users/pmdusso/code/shared/agent-comms/prompts/arch.md) e [`docs/prompts/pm.md`](file:///Users/pmdusso/code/shared/agent-comms/prompts/pm.md).
