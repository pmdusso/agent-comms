#!/usr/bin/env bash
set -e

echo "=== 🚀 Sincronizando repositório e Board de Projetos ==="
git checkout main
git pull || echo "⚠️  Aviso: git pull pendente de commit local."
./comms.py board-sync
./comms.py state