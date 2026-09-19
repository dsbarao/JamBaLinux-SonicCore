#!/usr/bin/env python3
"""Read-only Antigravity CLI MCP bridge for JamBaLinux."""
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
    "name": "antigravity_audit",
    "description": (
        "Run an independent, read-only Antigravity audit in JamBaLinux using "
        "the local authenticated account session. Returns the CLI JSON report. "
        "The bridge uses plan mode and sandboxing; it cannot edit, commit, "
        "install packages, use the network, or change system state."
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


def antigravity_ready() -> tuple[bool, str]:
    if os.environ.get("GEMINI_API_KEY"):
        return False, "GEMINI_API_KEY está definido; ponte recusada para evitar uso de API."
    if not shutil.which("agy"):
        return False, "Antigravity CLI (`agy`) não foi encontrado no PATH."
    try:
        models = subprocess.run(
            ["agy", "models"], cwd=PROJECT, text=True, capture_output=True,
            timeout=30, check=False,
        )
    except subprocess.TimeoutExpired:
        return False, "A verificação de autenticação do Antigravity expirou."
    if models.returncode != 0 or not models.stdout.strip():
        return False, "Antigravity não está autenticado ou não retornou modelos disponíveis."
    return True, ""


def audit(task: str) -> dict[str, Any]:
    ready, reason = antigravity_ready()
    if not ready:
        return {"ok": False, "error": reason}
    if not isinstance(task, str) or not task.strip() or len(task) > MAX_TASK_CHARS:
        return {"ok": False, "error": "A tarefa deve ter entre 1 e 24000 caracteres."}
    prompt = f"""Você é um revisor independente SOMENTE LEITURA no projeto JamBaLinux.
Execute em modo de planejamento: não edite arquivos, não faça commit, não instale dependências, não use rede e não execute ações que alterem o sistema.
Leia AGENTS.md e os documentos relevantes antes de avaliar. Não considere algo entregue sem evidência executada.

Tarefa:
{task.strip()}

Responda em JSON com: status (pass|fail|blocked), resumo, evidencias (lista), riscos (lista), arquivos_relevantes (lista), proximas_acoes (lista)."""
    command = [
        "agy", f"--print={prompt}", "--output-format", "json", "--mode", "plan", "--sandbox",
        "--print-timeout", "10m",
    ]
    try:
        result = subprocess.run(
            command, cwd=PROJECT, text=True, capture_output=True,
            timeout=TIMEOUT_SECONDS + 30, check=False,
        )
    except subprocess.TimeoutExpired:
        return {"ok": False, "error": "Antigravity excedeu o limite de 10 minutos."}
    if result.returncode != 0:
        return {"ok": False, "error": "Antigravity falhou.", "details": result.stderr[-2000:]}
    try:
        return {"ok": True, "antigravity": json.loads(result.stdout)}
    except json.JSONDecodeError:
        return {"ok": False, "error": "Antigravity não retornou JSON válido.", "details": result.stdout[-2000:]}


def handle(request: dict[str, Any]) -> None:
    method = request.get("method")
    request_id = request.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": request_id, "result": {
            "protocolVersion": request.get("params", {}).get("protocolVersion", "2025-03-26"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "antigravity-jambalinux", "version": "1.0.0"},
        }})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": [TOOL]}})
    elif method == "tools/call":
        params = request.get("params", {})
        if params.get("name") != TOOL["name"]:
            error(request_id, -32602, "Ferramenta desconhecida.")
            return
        report = audit(params.get("arguments", {}).get("task", ""))
        send({"jsonrpc": "2.0", "id": request_id, "result": {
            "content": [{"type": "text", "text": json.dumps(report, ensure_ascii=False)}]
        }})
    elif request_id is not None:
        error(request_id, -32601, "Método não suportado.")


def main() -> None:
    for line in sys.stdin:
        try:
            handle(json.loads(line))
        except json.JSONDecodeError:
            error(None, -32700, "JSON-RPC inválido.")
        except Exception as exc:
            error(None, -32603, f"Erro interno da ponte: {exc}")


if __name__ == "__main__":
    main()
