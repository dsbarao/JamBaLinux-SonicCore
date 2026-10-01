#!/usr/bin/env bash
# Registers the local, read-only Claude Pro audit bridge with Codex.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bridge="$project_dir/tools/claude_pro_mcp.py"

if ! command -v codex >/dev/null 2>&1; then
  printf 'Codex CLI não foi encontrado no PATH.\n' >&2
  exit 1
fi

if ! command -v claude >/dev/null 2>&1; then
  printf 'Claude Code CLI não foi encontrado no PATH.\n' >&2
  exit 1
fi

if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
  printf 'ANTHROPIC_API_KEY está definido; removê-lo desta sessão evita uso da API.\n' >&2
  exit 1
fi

auth_json="$(claude auth status)"
if ! grep -q '"loggedIn": true' <<<"$auth_json" || ! grep -q '"authMethod": "claude.ai"' <<<"$auth_json"; then
  printf 'Claude Code não está autenticado via claude.ai. Execute `claude /login` primeiro.\n' >&2
  exit 1
fi

exec codex mcp add claude-pro-jambalinux -- python3 "$bridge"
