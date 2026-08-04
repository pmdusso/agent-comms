#!/usr/bin/env bash
# ==============================================================================
# Agent Comms & Orchestration Infrastructure Installer
# Installs the multi-agent orchestration toolset into any target repository.
# Usage: ./install.sh [/path/to/target/repo] [--update]
#
#   (default)  instala; NÃO sobrescreve prompts e skills já presentes
#   --update   sobrescreve tudo, inclusive customizações locais
#
# O comms.py é sempre sobrescrito — é o engine, e re-rodar o installer é o
# caminho de atualização. Nada de configuração vive nele desde que o roster
# passou a sair de docs/prompts/, então sobrescrever é seguro.
# ==============================================================================

set -euo pipefail

SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR="."
UPDATE=0
for arg in "$@"; do
    case "$arg" in
        --update) UPDATE=1 ;;
        *) TARGET_DIR="$arg" ;;
    esac
done
TARGET_DIR="$(cd "$TARGET_DIR" && pwd)"

# -n não sobrescreve o que existe; com --update, sobrescreve.
CP_CONTENT="-R"
[ "$UPDATE" -eq 0 ] && CP_CONTENT="-Rn"

if [ "$UPDATE" -eq 1 ]; then
    echo "🚀 Updating Agent Comms in: $TARGET_DIR (sobrescreve customizações)"
else
    echo "🚀 Installing Agent Comms into: $TARGET_DIR"
fi

# 1. Copy CLI Orchestrator
cp "$SOURCE_DIR/comms.py" "$TARGET_DIR/comms.py"
chmod +x "$TARGET_DIR/comms.py"

# 2. Prompts + Setup guide. Prompts são o ROSTER de agentes e vivem editados —
#    sobrescrever silenciosamente apagaria customização e mudaria quem posta.
#    AGENT_COMMS.md e PIPELINE.md não são copiados: o bootstrap os gera a partir
#    dos templates do comms.py, refletindo o roster daquele repositório.
mkdir -p "$TARGET_DIR/docs/prompts" "$TARGET_DIR/docs/core"
cp $CP_CONTENT "$SOURCE_DIR/prompts/"*.md "$TARGET_DIR/docs/prompts/"
cp $CP_CONTENT "$SOURCE_DIR/docs/SETUP_GUIDE.md" "$TARGET_DIR/docs/core/"

# 3. Skills: uma cópia real em .claude/skills, symlinks para os outros CLIs.
#    Cópias paralelas divergem — foi assim que nasceram os aliases duplicados.
mkdir -p "$TARGET_DIR/.claude/skills"
cp $CP_CONTENT "$SOURCE_DIR/skills/"* "$TARGET_DIR/.claude/skills/"
for cli in .cursor .agents; do
    mkdir -p "$TARGET_DIR/$cli/skills"
    for skill in "$SOURCE_DIR/skills/"*/; do
        name="$(basename "$skill")"
        ln -sfn "../../.claude/skills/$name" "$TARGET_DIR/$cli/skills/$name"
    done
done
echo "✓ Skills discovery/delivery em .claude/skills/ (symlink em .cursor/ e .agents/)"

# 4. Bootstrap: cria comms.db (WAL), gera docs/core/{AGENT_COMMS,PIPELINE}.md e
#    adiciona o banco ao .gitignore.
#    NÃO usar `state` aqui: é read-only e aborta se o banco ainda não existe.
cd "$TARGET_DIR"
python3 comms.py bootstrap

echo "✅ Agent Comms successfully installed in $TARGET_DIR!"
echo ""
echo "Quick start in target project:"
echo "  1. Config board: ./comms.py set board.project <N>   # default: 1 / @me / repo do cwd"
echo "  2. Ver roster:   ./comms.py agents                  # = docs/prompts/*.md"
echo "  3. Subir agente: ./comms.py brief dev               # prompt + estado, num blob"
echo "  4. Start card:   ./comms.py start <ISSUE_NUMBER>"
echo "  5. Gate:         ./comms.py gate                    # exit 1 se finding sem destino"
echo "  6. Finish card:  ./comms.py finish <ISSUE_NUMBER> --pr <PR_URL>"
echo ""
echo "Atualizar depois: ./install.sh <dir> --update   (sobrescreve prompts/skills)"
