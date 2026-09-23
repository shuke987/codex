#!/usr/bin/env python3
"""Exercise the extracted release package with a local model and isolated state."""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from app_server_harness import MockResponsesServer
from app_server_harness import ev_completed
from app_server_harness import ev_response_created
from app_server_harness import sse


def main() -> None:
    package = Path(sys.argv[1]).resolve()
    manifest = json.loads((package / "codex-package.json").read_text())
    cli = package / manifest["entrypoint"]
    version = subprocess.check_output([str(cli), "--version"], text=True).strip()
    assert version == f"codex-cli {manifest['version']}", version
    assert "--goal" in subprocess.check_output([str(cli), "exec", "--help"], text=True)
    assert os.access(package / "bin/codex-code-mode-host", os.X_OK)
    for relative in [
        "codex-path/rg",
        "codex-resources/bwrap",
        "codex-resources/zsh/bin/zsh",
    ]:
        subprocess.run([str(package / relative), "--version"], check=True, timeout=30)

    with (
        tempfile.TemporaryDirectory(prefix="release-smoke-") as directory,
        MockResponsesServer() as server,
    ):
        root = Path(directory)
        home = root / "home"
        home.mkdir()
        (home / "config.toml").write_text(
            f'''model = "package-smoke"
model_provider = "package_smoke"
approval_policy = "never"
sandbox_mode = "workspace-write"
suppress_unstable_features_warning = true
[features]
code_mode_only = true
code_mode_host = true
memories = false
apps = false
plugins = false
[analytics]
enabled = false
[otel]
exporter = "none"
trace_exporter = "none"
metrics_exporter = "none"
[model_providers.package_smoke]
name = "package smoke"
base_url = "{server.url}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
'''
        )
        environment = dict(os.environ)
        environment.update(
            CODEX_HOME=str(home),
            CODEX_SQLITE_HOME=str(home),
            CODEX_API_KEY="dummy",
            NO_PROXY="127.0.0.1,localhost",
            ZDOTDIR=str(root),
        )
        environment.pop("BASH_ENV", None)
        server.enqueue_sse(
            sse(
                [
                    {
                        "type": "response.failed",
                        "response": {
                            "id": "capacity",
                            "error": {
                                "code": "server_is_overloaded",
                                "message": "Selected model is at capacity. Please try a different model.",
                            },
                        },
                    }
                ]
            )
        )
        command = [str(cli), "exec", "--skip-git-repo-check", "--json", "--goal"]
        first = subprocess.run(
            [*command, "finish the package smoke goal"],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            timeout=120,
        )
        assert first.returncode == 1, first
        events = [json.loads(line) for line in first.stdout.splitlines()]
        assert events[-1]["type"] == "turn.failed", first
        thread = events[0]
        shell = json.dumps(
            {
                "cmd": "command -v rg && rg --version",
                "login": False,
                "yield_time_ms": 10000,
            }
        )
        server.enqueue_sse(
            sse(
                [
                    ev_response_created("recovered"),
                    {
                        "type": "response.output_item.done",
                        "item": {
                            "type": "custom_tool_call",
                            "call_id": "package-smoke",
                            "name": "exec",
                            "input": f'const result = await tools.exec_command({shell}); if (result.exit_code !== 0) throw new Error(JSON.stringify(result)); text(JSON.stringify(result)); text(JSON.stringify(await tools.update_goal({{status: "complete"}}))); text("release-smoke-tools-ok");',
                        },
                    },
                    ev_completed("recovered"),
                ]
            )
        )
        server.enqueue_assistant_message("package goal complete", response_id="done")
        resumed = subprocess.run(
            [
                *command,
                "resume",
                thread["thread_id"],
                "continue the package smoke goal",
            ],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            timeout=120,
        )
        assert resumed.returncode == 0, resumed
        events = [json.loads(line) for line in resumed.stdout.splitlines()]
        assert events[0] == thread, resumed
        assert any(event["type"] == "turn.started" for event in events), resumed
        assert events[-1]["type"] == "turn.completed", resumed
        requests = [
            request for request in server.requests() if request.path == "/v1/responses"
        ]
        assert len(requests) == 3, len(requests)
        output = next(
            item["output"]
            for item in requests[-1].input()
            if item.get("type") == "custom_tool_call_output"
            and item.get("call_id") == "package-smoke"
        )
        output_text = output if isinstance(output, str) else json.dumps(output)
        assert str(package / "codex-path/rg") in output_text, output
        assert "ripgrep" in output_text and "release-smoke-tools-ok" in output_text, (
            output
        )
    print(
        "PASS: package identity, helpers, code-mode execution and same-thread capacity recovery"
    )


if __name__ == "__main__":
    main()
