#!/usr/bin/env python3
import json
from pathlib import Path
import sys
import time
import uuid

assert sys.argv[1:] == ["acp", "--sandbox", "--disable-telemetry"]


def send(**frame):
    print(json.dumps(dict(jsonrpc="2.0", **frame)), flush=True)


session = None
model = "default"
prompt_id = None


def options():
    return {
        "configOptions": [
            {
                "id": "model",
                "category": "model",
                "currentValue": model,
                "options": [{"value": "default", "name": "Default"}, {"value": "gemini-fixture", "name": "Fixture Gemini"}],
            },
            {
                "id": "thought_level",
                "category": "thought_level",
                "currentValue": "low",
                "options": [{"value": "low", "name": "Low"}],
            },
        ],
    }


def update(kind, **data):
    send(method="session/update", params={"sessionId": session, "update": {"sessionUpdate": kind, **data}})


for line in sys.stdin:
    request = json.loads(line)
    with open("wire.jsonl", "a") as log:
        log.write(line)
    method = request.get("method")
    params = request.get("params", {})
    call = request.get("id")
    if method == "initialize":
        assert params["protocolVersion"] == 1
        send(id=call, result={"protocolVersion": 1, "agentCapabilities": {"sessionCapabilities": {"resume": {}, "close": {}}}})
    elif method in ("session/new", "session/resume"):
        assert Path(params["cwd"]) == Path.cwd() and params["mcpServers"] == []
        if Path("needs-login").exists():
            send(id=call, error={"code": -32000, "message": "Authentication required"})
        elif method == "session/resume" and Path("missing-cache").exists():
            send(id=call, error={"code": -32603, "message": "Failed to read session fixture; retry, or load it with session/load"})
        else:
            session = params.get("sessionId") or str(uuid.uuid4())
            send(id=call, result=dict(sessionId=session, **options()))
    elif method == "session/set_config_option":
        assert params["sessionId"] == session and params["configId"] == "model"
        model = params["value"]
        send(id=call, result=options())
    elif method == "session/prompt":
        assert params["sessionId"] == session
        prompt_id = call
        text = params["prompt"][-1]["text"]
        update("agent_thought_chunk", content={"type": "text", "text": "Thinking π"})
        update("tool_call", toolCallId="fixture", title="Run fixture", status="pending")
        send(id="permission", method="session/request_permission", params={"sessionId": session, "options": [
            {"optionId": "allow", "kind": "allow_always"}, {"optionId": "deny", "kind": "reject_once"},
        ]})
    elif call == "permission":
        assert request["result"]["outcome"] == {"outcome": "selected", "optionId": "deny"}
        update("tool_call_update", toolCallId="fixture", status="failed")
        update("agent_message_chunk", content={"type": "text", "text": f"Streamed {text} with {model}"})
        if text == "hold first":
            deadline = time.monotonic() + 10
            while not Path("release-first").exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            assert Path("release-first").exists()
        if text != "wait for stop":
            update("agent_message_chunk", content={"type": "text", "text": " — done"})
            update("usage_update", used=24, size=1000)
            send(id=prompt_id, result={"stopReason": "end_turn"})
    elif method == "session/close":
        assert params["sessionId"] == session
        send(id=call, result={})
    elif method == "session/cancel":
        assert params["sessionId"] == session
        # Deliberately do not reply. Tau still has to stop and settle the chat.
    else:
        raise AssertionError(request)
