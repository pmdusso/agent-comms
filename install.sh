#!/usr/bin/env bash
# ==============================================================================
# Agent Comms & Orchestration Infrastructure Installer
# Installs the multi-agent orchestration toolset into any target repository.
# Usage: ./install.sh [/path/to/target/repo] [--update]
#
#   (default)  instala; NÃO sobrescreve prompts e skills já presentes
#   --update   sobrescreve tudo, inclusive customizações locais
#
# O engine Rust é compilado e instalado globalmente. Upgrades de instalações que
# ainda têm comms.py exigem --update porque prompts, skills e docs podem chamar o
# engine antigo.
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

# 1. Guarda de legado: precisa acontecer antes de qualquer cópia.
if [ -f "$TARGET_DIR/comms.py" ] && [ "$UPDATE" -eq 0 ]; then
    echo "ERRO: upgrade do engine Python exige --update (prompts, skills e docs/core ainda chamam ./comms.py)." >&2
    exit 1
fi

# -n não sobrescreve o que existe; com --update, sobrescreve.
CP_CONTENT="-R"
[ "$UPDATE" -eq 0 ] && CP_CONTENT="-Rn"

if [ "$UPDATE" -eq 1 ]; then
    echo "🚀 Updating Agent Comms in: $TARGET_DIR (sobrescreve customizações)"
else
    echo "🚀 Installing Agent Comms into: $TARGET_DIR"
fi

# 2. Engine Rust: build reproduzível e binário global no PATH.
BIN_DIR="${COMMS_BIN_DIR:-$HOME/.local/bin}"
if ! command -v cargo >/dev/null 2>&1; then
    echo "ERRO: cargo não encontrado — instale Rust via rustup e rode de novo." >&2
    exit 1
fi
CARGO_BUILD_DIR="${CARGO_TARGET_DIR:-$SOURCE_DIR/target}"
HOST_TARGET=''
while read -r key value; do
    if [ "$key" = 'host:' ]; then
        HOST_TARGET="$value"
        break
    fi
done < <(rustc -vV)
if [ -z "$HOST_TARGET" ]; then
    echo "ERRO: não deu para determinar o target host via rustc -vV." >&2
    exit 1
fi
cargo build --release --locked --manifest-path "$SOURCE_DIR/Cargo.toml" --target-dir "$CARGO_BUILD_DIR" --target "$HOST_TARGET"
mkdir -p "$BIN_DIR"
BIN_DIR="$(cd "$BIN_DIR" && pwd)"
install -m 0755 "$CARGO_BUILD_DIR/$HOST_TARGET/release/comms" "$BIN_DIR/comms"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "AVISO: $BIN_DIR não está no PATH — adicione ao seu shell rc." ;;
esac
"$BIN_DIR/comms" --version

# 3. Prompts + Setup guide. Prompts são o ROSTER padrão e vivem editados —
#    sobrescrever silenciosamente apagaria customização e mudaria quem posta.
#    AGENT_COMMS.md e PIPELINE.md não são copiados: o bootstrap os gera a partir
#    dos templates embutidos no binário `comms`, refletindo o roster daquele repo.
mkdir -p "$TARGET_DIR/docs/prompts" "$TARGET_DIR/docs/core"
cp $CP_CONTENT "$SOURCE_DIR/prompts/"*.md "$TARGET_DIR/docs/prompts/"
cp $CP_CONTENT "$SOURCE_DIR/docs/SETUP_GUIDE.md" "$TARGET_DIR/docs/core/"

# 4. Skills: uma cópia real em .claude/skills, symlinks para os outros CLIs.
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

# 5. Remove o legado somente no upgrade explícito; o git preserva versões antigas.
if [ "$UPDATE" -eq 1 ]; then
    rm -f "$TARGET_DIR/comms.py" "$TARGET_DIR/docs/core/AGENT_COMMS.md" "$TARGET_DIR/docs/core/PIPELINE.md"
    echo "✓ comms.py legado removido; docs/core/{AGENT_COMMS,PIPELINE}.md serão regerados (versões antigas ficam no git)."
fi

# 6. Bootstrap e migração: banco WAL, docs/core e roster dinâmico.
cd "$TARGET_DIR"
env -u COMMS_DB "$BIN_DIR/comms" bootstrap
env -u COMMS_DB "$BIN_DIR/comms" migrate

# 7. Detecta instruções locais que continuariam chamando o engine removido.
LEGACY_DOCS="$(grep -rIl --exclude-dir=.git --exclude-dir=.worktrees --exclude-dir=node_modules -e './comms.py' -e 'python3 comms.py' . || true)"
if [ -n "$LEGACY_DOCS" ]; then
    echo "AVISO: estes arquivos ainda chamam ./comms.py — troque por 'comms':"
    while IFS= read -r file; do
        printf '  %s\n' "$file"
    done <<< "$LEGACY_DOCS"
fi

echo "✅ Agent Comms successfully installed in $TARGET_DIR!"
echo ""
echo "Quick start in target project:"
echo "  1. Config board: comms set board.project <N>   # default: 1 / @me / repo do cwd"
echo "  2. Ver roster:   comms agents                  # = docs/prompts/*.md + ad-hoc"
echo "  3. Subir agente: comms brief dev               # prompt + estado, num blob"
echo "  4. Start card:   comms start <ISSUE_NUMBER>"
echo "  5. Gate:         comms gate                    # exit 1 se finding sem destino"
echo "  6. Finish card:  comms finish <ISSUE_NUMBER> --pr <PR_URL>"
echo ""
echo "Atualizar depois: ./install.sh <dir> --update   (sobrescreve prompts/skills)"
