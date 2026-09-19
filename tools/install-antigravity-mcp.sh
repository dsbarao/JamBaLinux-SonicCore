#!/usr/bin/env bash
# Registers the local, read-only Antigravity audit bridge with Codex.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bridge="$project_dir/tools/antigravity_mcp.py"

if ! command -v codex >/dev/null 2>&1 || ! command -v agy >/dev/null 2>&1; then
  printf 'Codex CLI ou Antigravity CLI (`agy`) não foi encontrado no PATH.\n' >&2
  exit 1
fi
if [[ -n "${GEMINI_API_KEY:-}" ]]; then
  printf 'GEMINI_API_KEY está definido; remova-o desta sessão para evitar uso da API.\n' >&2
  exit 1
fi
if ! agy models >/dev/null; then
  printf 'Antigravity não está autenticado. Abra `agy` interativamente e conclua o login.\n' >&2
  exit 1
fi

exec codex mcp add antigravity-jambalinux -- python3 "$bridge"
