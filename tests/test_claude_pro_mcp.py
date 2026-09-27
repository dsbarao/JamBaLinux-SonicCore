#!/usr/bin/env python3
"""Unit tests for the read-only Claude Pro MCP bridge."""
from __future__ import annotations

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[1] / "tools" / "claude_pro_mcp.py"
SPEC = importlib.util.spec_from_file_location("claude_pro_mcp", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
BRIDGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BRIDGE)


class ClaudeReadyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.environ = {key: value for key, value in BRIDGE.os.environ.items()
                        if key != "ANTHROPIC_API_KEY"}

    def test_timeout_is_reported_without_raising(self) -> None:
        with (
            patch.dict(BRIDGE.os.environ, self.environ, clear=True),
            patch.object(BRIDGE.shutil, "which", return_value="/usr/bin/claude"),
            patch.object(
                BRIDGE.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired(["claude", "auth", "status"], 15),
            ),
        ):
            ready, reason = BRIDGE.claude_ready()

        self.assertFalse(ready)
        self.assertIn("excedeu o limite", reason)

    def test_os_error_is_reported_without_raising(self) -> None:
        with (
            patch.dict(BRIDGE.os.environ, self.environ, clear=True),
            patch.object(BRIDGE.shutil, "which", return_value="/usr/bin/claude"),
            patch.object(BRIDGE.subprocess, "run", side_effect=OSError("process unavailable")),
        ):
            ready, reason = BRIDGE.claude_ready()

        self.assertFalse(ready)
        self.assertIn("Não foi possível executar", reason)

    def test_auth_check_has_timeout(self) -> None:
        completed = subprocess.CompletedProcess(
            ["claude", "auth", "status"], 0, '{"loggedIn": true, "authMethod": "claude.ai"}', ""
        )
        with (
            patch.dict(BRIDGE.os.environ, self.environ, clear=True),
            patch.object(BRIDGE.shutil, "which", return_value="/usr/bin/claude"),
            patch.object(BRIDGE.subprocess, "run", return_value=completed) as run,
        ):
            ready, reason = BRIDGE.claude_ready()

        self.assertTrue(ready)
        self.assertEqual(reason, "")
        self.assertEqual(run.call_args.kwargs["timeout"], 15)


class AllowedToolsTests(unittest.TestCase):
    def test_search_is_covered_by_grep_without_bash_rg(self) -> None:
        completed = subprocess.CompletedProcess(["claude"], 0, "{}", "")
        with (
            patch.object(BRIDGE, "claude_ready", return_value=(True, "")),
            patch.object(BRIDGE.subprocess, "run", return_value=completed) as run,
        ):
            report = BRIDGE.audit("verifique as ferramentas permitidas")

        self.assertTrue(report["ok"])
        command = run.call_args.args[0]
        allowed_tools = command[command.index("--allowedTools") + 1]

        self.assertEqual(
            allowed_tools,
            "Read,Glob,Grep,Bash(git status --short),Bash(git diff -- *)",
        )
        self.assertNotIn("Bash(rg", allowed_tools)


if __name__ == "__main__":
    unittest.main()
