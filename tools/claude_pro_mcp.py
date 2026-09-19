#!/usr/bin/env python3
"""Read-only Claude Code MCP bridge for JamBaLinux.

Authentication remains inside the locally logged-in Claude Code CLI.  This
server deliberately refuses API-key environments and exposes only an audit
tool, so Codex can request an independent report without delegating writes.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

PROJECT = Path(__file__).resolve().parent.parent
MAX_TASK_CHARS = 24_000
TIMEOUT_SECONDS = 600

TOOL = {
    "name": "claude_pro_audit",
    "description": (
        "Run an independent, read-only Claude Code audit in the JamBaLinux "
        "workspace. Uses the local Claude Pro OAuth session, never an API key, "
        "and returns Claude Code's JSON result. It cannot edit files, run "
        "network actions, commit, install packages, or change system state."
    ),
    "inputSchema": {
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": "Concrete audit, diagnosis, or acceptance-test task.",
                "minLength": 1,
                "maxLength": MAX_TASK_CHARS,
            }
        },
        "required": ["task"],
        "additionalProperties": False,
    },
}


def send(message: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def error(request_id: Any, code: int, message: str) -> None:
    send({"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}})


def claude_ready() -> tuple[bool, str]:
    if os.environ.get("ANTHROPIC_API_KEY"):
        return False, "ANTHROPIC_API_KEY está definido; ponte recusada para evitar cobrança por API."
    if not shutil.which("claude"):
        return False, "Claude Code CLI não foi encontrado no PATH."
    status = subprocess.run(
        ["claude", "auth", "status"],
        cwd=PROJECT,
        text=True,
        capture_output=True,
        timeout=15,
        check=False,
    )
    if status.returncode != 0:
        return False, "Não foi possível consultar a autenticação do Claude Code."
    try:
        auth = json.loads(status.stdout)
    except json.JSONDecodeError:
        return False, "Claude Code retornou um estado de autenticação inválido."
    if not auth.get("loggedIn") or auth.get("authMethod") != "claude.ai":
        return False, "Claude Code não está autenticado via claude.ai/Pro. Execute `claude /login`."
    return True, ""


def audit(task: str) -> dict[str, Any]:
    ready, reason = claude_ready()
    if not ready:
        return {"ok": False, "error": reason}
    if not isinstance(task, str) or not task.strip() or len(task) > MAX_TASK_CHARS:
        return {"ok": False, "error": "A tarefa deve ter entre 1 e 24000 caracteres."}

    prompt = f"""Você é um revisor independente e SOMENTE LEITURA no projeto JamBaLinux.
Não edite arquivos, não faça commit, não instale dependências, não use rede e não execute ações que alterem o sistema.
Leia AGENTS.md e os documentos relevantes antes de avaliar. Não considere algo entregue sem evidência executada.

Tarefa:
{task.strip()}

Responda em JSON com: status (pass|fail|blocked), resumo, evidencias (lista), riscos (lista), arquivos_relevantes (lista), proximas_acoes (lista)."""
    command = [
        "claude",
        "--print",
        "--output-format",
        "json",
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--no-session-persistence",
        "--allowedTools",
        "Read,Glob,Grep,Bash(rg *),Bash(git status --short),Bash(git diff -- *)",
        prompt,
    ]
    try:
        result = subprocess.run(
            command,
            cwd=PROJECT,
            text=True,
            capture_output=True,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return {"ok": False, "error": "Claude Code excedeu o limite de 10 minutos."}
    if result.returncode != 0:
        return {"ok": False, "error": "Claude Code falhou.", "details": result.stderr[-2000:]}
    try:
        return {"ok": True, "claude": json.loads(result.stdout)}
    except json.JSONDecodeError:
        return {"ok": False, "error": "Claude Code não retornou JSON válido.", "details": result.stdout[-2000:]}


def handle(request: dict[str, Any]) -> None:
    method = request.get("method")
    request_id = request.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": request.get("params", {}).get("protocolVersion", "2025-03-26"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "claude-pro-jambalinux", "version": "1.0.0"},
            },
        })
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": [TOOL]}})
    elif method == "tools/call":
        params = request.get("params", {})
        if params.get("name") != TOOL["name"]:
            error(request_id, -32602, "Ferramenta desconhecida.")
            return
        report = audit(params.get("arguments", {}).get("task", ""))
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {"content": [{"type": "text", "text": json.dumps(report, ensure_ascii=False)}]},
        })
    elif request_id is not None:
        error(request_id, -32601, "Método não suportado.")


def main() -> None:
    for line in sys.stdin:
        try:
            handle(json.loads(line))
        except json.JSONDecodeError:
            error(None, -32700, "JSON-RPC inválido.")
        except Exception as exc:  # Never leak stack traces into an MCP transcript.
            error(None, -32603, f"Erro interno da ponte: {exc}")


if __name__ == "__main__":
    main()
