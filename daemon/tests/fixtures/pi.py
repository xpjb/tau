#!/usr/bin/env python3
import json
import os
import sys
import uuid

session_dir = sys.argv[sys.argv.index("--session-dir") + 1]
os.makedirs(session_dir, exist_ok=True)
with open(os.path.join(session_dir, "spawn-args"), "a") as marker:
    marker.write(json.dumps(sys.argv[1:]) + "\n")
model = sys.argv[sys.argv.index("--model") + 1] if "--model" in sys.argv else "test/model"
provider, model_id = model.split("/", 1)
session_file = sys.argv[sys.argv.index("--session") + 1] if "--session" in sys.argv else os.path.join(session_dir, str(uuid.uuid4()) + ".jsonl")
if not os.path.exists(session_file):
    with open(session_file, "w") as file:
        file.write(json.dumps({"type": "session", "version": 3, "id": "mock"}) + "\n")
with open(session_file) as file:
    entries = [entry for line in file if (entry := json.loads(line)).get("type") != "session"]
head = entries[-1]["id"] if entries else None
generation = str(uuid.uuid4())
sequence = 0
live = []
queue = []
run_id = None
paused = False
control = None
pending_prompt = None
subscribed = False


def output(value):
    print(json.dumps(value), flush=True)


def queue_state():
    return {"queuedRequests": queue, "runId": run_id, "paused": paused, "control": control,
            "capabilities": ["queue_edit", "queue_delete", "queue_run_prefix", "queue_pause", "queue_resume", "queue_cancel_control"],
            "boundaries": ["now", "reasoning_checkpoint", "turn"]}


def update(change, skip=0):
    global sequence
    sequence += 1 + skip
    if subscribed:
        output({"type": "transcript_update", "sessionId": "mock", "generation": generation, "sequence": sequence, "change": change})


def append(text, role="user", origin=None):
    global head
    entry = {"type": "message", "id": str(uuid.uuid4()), "parentId": head,
             "message": {"role": role, "content": text, "timestamp": 1}, "origin": origin or {}}
    entries.append(entry)
    head = entry["id"]
    with open(session_file, "a") as file:
        file.write(json.dumps(entry) + "\n")
    update({"type": "append", "entry": entry, "leafId": head})
    return entry


for line in sys.stdin:
    command = json.loads(line)
    ident = command.get("id")
    kind = command.get("type")
    response = {"id": ident, "type": "response", "command": kind, "success": True}
    if kind == "get_state":
        response["data"] = {"sessionFile": session_file, "isStreaming": run_id is not None, "isCompacting": False,
                            "model": {"provider": provider, "id": model_id}, **queue_state()}
    elif kind == "get_transcript":
        subscribed = True
        response["data"] = {"sessionId": "mock", "generation": generation, "sequence": sequence,
                            "entries": entries, "leafId": head, "live": live, **queue_state()}
    elif kind == "get_entries":
        response["data"] = {"entries": entries, "leafId": head}
    elif kind == "get_commands":
        response["data"] = {"commands": [{"name": name, "source": source} for name, source in
                                         [("choose", "extension"), ("review", "prompt"), ("skill:search", "skill"), ("tau-fork-at", "extension")]]}
    elif kind == "get_available_models":
        response["data"] = {"models": [{"provider": "test", "id": "model", "name": "Test Model"}]}
    elif kind == "get_available_thinking_levels":
        response["data"] = {"levels": ["low", "high"]}
    elif kind == "set_model":
        provider, model_id = command["provider"], command["modelId"]
        response["data"] = {"provider": provider, "id": model_id}
    elif kind == "prompt":
        assert command.get("streamingBehavior") == "steer"
        request_id = command.get("requestId")
        if command["message"] == "/choose":
            pending_prompt = ident
            output({"type": "extension_ui_request", "id": "dialog-1", "method": "select", "options": ["One", "Two"]})
            continue
        if run_id is not None or paused:
            queue.append({"requestId": request_id, "revision": 0, "kind": "steer",
                          "message": {"role": "user", "content": [{"type": "text", "text": command["message"]}], "timestamp": 2}})
            update({"type": "queue", **queue_state()})
            response["data"] = {"requestId": request_id, "disposition": "queued"}
        else:
            append(command["message"], origin={"requestId": request_id, "requestRevision": 0})
            run_id = str(uuid.uuid4())
            output({"type": "agent_start"})
            update({"type": "queue", **queue_state()})
            stream_id = str(uuid.uuid4())
            live = [{"streamId": stream_id, "parentId": head, "timestamp": "2025-01-01T00:00:00Z",
                     "message": {"role": "assistant", "content": [{"type": "thinking", "thinking": "Checking"}]}}]
            update({"type": "live", "entry": live[0]})
            response["data"] = {"requestId": request_id, "disposition": "submitted"}
            output(response)
            if command["message"] != "hold":
                append([{"type": "thinking", "thinking": "Checking"}, {"type": "text", "text": "Hello from Tau"}],
                       role="assistant", origin={"streamId": stream_id})
                live = []
                run_id = None
                update({"type": "queue", **queue_state()})
                output({"type": "agent_settled"})
            continue
    elif kind == "extension_ui_response":
        with open(os.path.join(session_dir, "extension-response"), "w") as marker:
            marker.write(command.get("value", "cancelled"))
        output({"id": pending_prompt, "type": "response", "command": "prompt", "success": True, "data": {"disposition": "handled"}})
        pending_prompt = None
        continue
    elif kind in ("edit_queued_message", "delete_queued_message"):
        selected = next((request for request in queue if request["requestId"] == command["requestId"]), None)
        outcome = "not_queued" if selected is None else "conflict" if selected["revision"] != command["revision"] else None
        if outcome is None:
            if kind == "edit_queued_message":
                selected["revision"] += 1
                selected["message"]["content"] = [{"type": "text", "text": command["message"]}]
                outcome = "edited"
            else:
                queue.remove(selected)
                outcome = "deleted"
            update({"type": "queue", **queue_state()})
        response["data"] = {"outcome": outcome}
    elif kind in ("run_queue_prefix", "pause_queue", "resume_queue"):
        if command["runId"] != run_id:
            response.update(success=False, error="Active run changed")
        else:
            action = {"run_queue_prefix": "prefix", "pause_queue": "pause", "resume_queue": "resume"}[kind]
            control = {"commandId": command["controlId"], "runId": run_id, "action": action,
                       "boundary": command.get("boundary"), "requests": command.get("requests", []),
                       "status": "waiting" if run_id is not None and action != "resume" else "applied"}
            if control["status"] == "applied":
                paused = action != "resume"
            update({"type": "queue", **queue_state()})
            response["data"] = queue_state()
    elif kind == "cancel_queue_control":
        outcome = "not_pending"
        if control and control["commandId"] == command["controlId"] and control["status"] == "waiting":
            control["status"] = "cancelled"
            update({"type": "queue", **queue_state()})
            outcome = "cancelled"
        response["data"] = {"outcome": outcome}
    elif kind == "abort":
        run_id = None
        update({"type": "queue", **queue_state()})
        output({"type": "agent_settled"})
    elif kind in ("mock_gap", "mock_lag"):
        for index in range(2100 if kind == "mock_lag" else 1):
            live[0]["message"]["content"][0]["thinking"] += "."
            update({"type": "delta", "streamId": live[0]["streamId"],
                    "event": {"assistantMessageEvent": {"type": "thinking_delta", "contentIndex": 0, "delta": "."}}},
                   skip=1 if kind == "mock_gap" else 0)
    elif kind == "mock_exit":
        os._exit(0)
    elif kind == "mock_reject":
        response.update(success=False, error="Rejected on purpose")
    elif kind in ("fork", "clone"):
        selected = entries
        if kind == "fork":
            selected = entries[:next(index for index, entry in enumerate(entries) if entry["id"] == command["entryId"])]
            response["data"] = {"text": "fork draft"}
        session_file = os.path.join(session_dir, str(uuid.uuid4()) + ".jsonl")
        with open(session_file, "w") as file:
            file.write(json.dumps({"type": "session", "version": 3, "id": "child"}) + "\n")
            for entry in selected:
                file.write(json.dumps(entry) + "\n")
        entries = selected
        head = entries[-1]["id"] if entries else None
    output(response)
