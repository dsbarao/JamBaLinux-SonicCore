#!/usr/bin/env bash
# Abre uma conversa interativa do Codex Sol com o handoff de recuperação.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
handoff_file="$project_dir/docs/handoffs/sol-equalizer-recovery.md"

if ! command -v codex >/dev/null 2>&1; then
  printf 'Codex CLI não foi encontrado no PATH. Abra o Codex Desktop e escolha GPT-5.6 Sol, ou instale/habilite a CLI.\n' >&2
  exit 1
fi

if [[ ! -r "$handoff_file" ]]; then
  printf 'Handoff não encontrado: %s\n' "$handoff_file" >&2
  exit 1
fi

cd "$project_dir"
exec codex --model gpt-5.6-sol "$(<"$handoff_file")"
