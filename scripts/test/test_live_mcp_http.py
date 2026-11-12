import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("live-mcp-http.py")
spec = importlib.util.spec_from_file_location("live_mcp_http", SCRIPT)
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def tool_result(command, data, error=None):
    envelope = {
        "ok": error is None,
        "command": command,
        "data": data,
        "duration_ms": 0,
        "warnings": [],
        "steps": [],
    }
    if error is not None:
        envelope["error"] = error
    return {
        "jsonrpc": "2.0",
        "id": {"push": 10, "check": 11, "test": 12}[command],
        "result": {"structuredContent": envelope, "isError": error is not None},
    }


class LiveMcpHttpTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        patches = [
            patch.object(smoke, "CONFIG_PATH", SCRIPT),
            patch.object(smoke, "SERVER_LOG", Path(directory.name) / "server.log"),
            patch.object(smoke, "ensure_binary"),
            patch.object(smoke.subprocess, "Popen"),
            patch.object(smoke, "wait_for_server"),
            patch.object(smoke, "initialize_session", return_value=(
                "test-session", {"result": {"capabilities": {"tools": {}}}},
            )),
            patch.object(smoke, "send_initialized"),
            patch.object(smoke, "tools_list", return_value={"result": {"tools": [
                {"name": "build_project"},
                {"name": "check_syntax_edt"},
                {"name": "run_module_tests"},
            ]}}),
        ]
        for patcher in patches:
            patcher.start()
            self.addCleanup(patcher.stop)

    def test_successful_sequence_with_no_changes_to_build(self):
        responses = [
            tool_result("push", {
                "ok": True, "provider_dispatched": False, "steps": [], "duration_ms": 0,
            }),
            tool_result("check", {"status": "clean"}),
            tool_result("test", {"ok": True}),
        ]
        with patch.object(smoke, "call_tool", side_effect=responses) as call:
            self.assertEqual(smoke.main(), 0)
        self.assertEqual(
            [invocation.args[3] for invocation in call.call_args_list],
            ["build_project", "check_syntax_edt", "run_module_tests"],
        )

    def test_protocol_error_stops_smoke_with_tool_and_server_reason(self):
        response = {
            "jsonrpc": "2.0", "id": 10,
            "error": {"code": -32603, "message": "MCP call cancelled while waiting"},
        }
        with patch.object(smoke, "call_tool", return_value=response) as call:
            with self.assertRaisesRegex(RuntimeError, "build_project: JSON-RPC error.*-32603.*cancelled while waiting"):
                smoke.main()
        self.assertEqual(call.call_count, 1)

    def test_business_failure_stops_smoke_with_tool_and_server_reason(self):
        responses = [
            tool_result("push", {"ok": True}),
            tool_result("check", {"status": "issues_found"}, {
                "code": "runtime_failure",
                "kind": "runtime",
                "message": "syntax check 'edt' finished with status IssuesFound (exit code 0)",
            }),
        ]
        with patch.object(smoke, "call_tool", side_effect=responses) as call:
            with self.assertRaisesRegex(RuntimeError, "check_syntax_edt: command failed.*runtime_failure.*IssuesFound"):
                smoke.main()
        self.assertEqual(call.call_count, 2)


class LiveMcpHttpStartupTest(unittest.TestCase):
    def test_script_without_config_exits_with_setup_message(self):
        env = os.environ.copy()
        env.pop("V8TR_REAL_CONFIG", None)
        result = subprocess.run(
            [sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True, timeout=10,
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("V8TR_REAL_CONFIG is not set.", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
