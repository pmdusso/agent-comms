#!/usr/bin/env bash
# ==============================================================================
# Agent Comms & Orchestration Infrastructure Installer
# Installs the multi-agent orchestration toolset into any target repository.
# Usage: ./install.sh [/path/to/target/repo]
# ==============================================================================

set -euo pipefail

SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR="${1:-.}"
TARGET_DIR="$(cd "$TARGET_DIR" && pwd)"

echo "🚀 Installing Agent Comms into: $TARGET_DIR"

# 1. Copy CLI Orchestrator
cp "$SOURCE_DIR/comms.py" "$TARGET_DIR/comms.py"
chmod +x "$TARGET_DIR/comms.py"

# 2. Copy Prompts + Setup guide
#    AGENT_COMMS.md e PIPELINE.md NÃO são copiados: o bootstrap os gera a partir
#    dos templates do comms.py, refletindo SENDERS/ROLES daquele repositório.
mkdir -p "$TARGET_DIR/docs/prompts" "$TARGET_DIR/docs/core"
cp "$SOURCE_DIR/prompts/"*.md "$TARGET_DIR/docs/prompts/"
cp "$SOURCE_DIR/docs/SETUP_GUIDE.md" "$TARGET_DIR/docs/core/"

# 3. Skills: uma cópia real em .claude/skills, symlinks para os outros CLIs.
#    Cópias paralelas divergem — foi assim que nasceram os aliases duplicados.
mkdir -p "$TARGET_DIR/.claude/skills"
cp -R "$SOURCE_DIR/skills/"* "$TARGET_DIR/.claude/skills/"
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
echo "  2. Sync board:   ./comms.py board-sync"
echo "  3. Start card:   ./comms.py start <ISSUE_NUMBER>"
echo "  4. Finish card:  ./comms.py finish <ISSUE_NUMBER> --pr <PR_URL>"
echo "  5. Check state:  ./comms.py state"
