#!/usr/bin/env python3
"""Opt-in REAL model smoke test. Never run as part of the unit suite.

Example: python3 integrations/room/smoke.py --provider anthropic --model claude-haiku-4-5
Optional independent model: --provider2 ... --model2 ...
Uses installed Pi auth/models via private copies; no keys in arguments/output.
Build target/debug/herdr first. Starts only an owned disposable debug server.
"""
import argparse
import json
from pathlib import Path
import tempfile
import time
import uuid

import disposable
from room import call


def identity(member):
    return {key: member[key] for key in ("pane_id", "terminal_id", "session")}


def wait_until(check, deadline, description):
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.2)
    raise RuntimeError("Timed out waiting for " + description + "; no write was retried")


def saved_messages(directory, workspace):
    # Poll the persisted transcript rather than room.read/get: this proves that
    # dispatch/replies happen automatically, not as a side effect of an API read.
    file = Path(directory) / "config/herdr-dev/session.json"
    try:
        snapshot = json.loads(file.read_text())
        return next(w["room"]["messages"] for w in snapshot["workspaces"] if w["id"] == workspace)
    except (FileNotFoundError, json.JSONDecodeError, StopIteration, KeyError):
        return []


def run(args):
    if not disposable.BINARY.is_file():
        raise RuntimeError("Build this checkout's target/debug/herdr first")
    with tempfile.TemporaryDirectory(prefix="herdr-room-real-") as directory:
        with disposable.server(directory, args.pi_config) as env:
            socket = env["HERDR_SOCKET_PATH"]
            deadline = time.monotonic() + args.timeout
            created = call(socket, "workspace.create", {"label": "room-real-smoke", "cwd": directory})
            workspace = created["workspace"]["workspace_id"]
            second = call(socket, "tab.create", {"workspace_id": workspace, "cwd": directory, "label": "second"})
            panes = [created["root_pane"]["pane_id"], second["root_pane"]["pane_id"]]
            models = [(args.provider, args.model), (args.provider2 or args.provider, args.model2 or args.model)]
            # Only startup is performed via agent.start (normal Pi TUI). Questions
            # themselves never use agent.prompt, pane.send or PTY text injection.
            time.sleep(0.3)
            for n, (pane, (provider, model)) in enumerate(zip(panes, models), 1):
                call(socket, "agent.start", {"name": f"room-smoke-{n}", "kind": "pi", "pane_id": pane,
                    "timeout_ms": min(300000, max(5000, int(args.timeout * 1000))),
                    "args": ["--provider", provider, "--model", model, "--thinking", args.thinking,
                             "--no-approve", "--no-builtin-tools", "--system-prompt",
                             "You are a room messaging smoke-test participant. Answer only the active room question via room_reply once. Do not use shell, files, other agents or any other tools. Then stop."]})

            def ready_members():
                agents = call(socket, "agent.list", {})["agents"]
                live = [a for a in agents if a["pane_id"] in panes and (a.get("agent_session") or {}).get("source") == "herdr:pi"]
                if len(live) != 2:
                    return None
                info = call(socket, "room.get", {"workspace_id": workspace})
                members = [m for m in info["members"] if m["pane_id"] in panes and m.get("session")]
                ready = [r["member"] for r in info.get("receivers", []) if r.get("available")]
                if len(members) == 2 and all(any(identity(m) == identity(r) for r in ready) for m in members):
                    return sorted(members, key=lambda m: panes.index(m["pane_id"]))
                return None

            members = wait_until(ready_members, deadline, "two actual Herdr Pi hook identities and room receiver readiness")
            print("Two real Pi TUI receivers ready (private config, no global hooks).", flush=True)
            token = "room-smoke-" + uuid.uuid4().hex[:10]
            # Omitted recipient means broadcast, not a manually replicated post.
            broadcast = call(socket, "room.post", {"workspace_id": workspace, "text": f"Reply with exactly {token}-broadcast using room_reply."})
            if broadcast.get("persistence") != "saved":
                raise RuntimeError("Broadcast save not confirmed; no retry")
            request1 = broadcast["sequence"]

            def replies(request, count):
                found = [m for m in saved_messages(directory, workspace) if m.get("reply_to") == request]
                return found if len(found) >= count else None

            first = wait_until(lambda: replies(request1, 2), deadline, "two saved broadcast replies, without reading the room API")
            if len(first) != 2 or {m["text"].strip() for m in first} != {token + "-broadcast"}:
                raise RuntimeError("Broadcast did not produce exactly two expected model replies")
            if {json.dumps(identity(m["author"]), sort_keys=True) for m in first} != {json.dumps(identity(m), sort_keys=True) for m in members}:
                raise RuntimeError("Broadcast reply attribution mismatch")
            targeted = call(socket, "room.post", {"workspace_id": workspace, "recipient": identity(members[0]),
                "text": f"Reply with exactly {token}-targeted using room_reply."})
            if targeted.get("persistence") != "saved":
                raise RuntimeError("Targeted save not confirmed; no retry")
            request2 = targeted["sequence"]
            last = wait_until(lambda: replies(request2, 1), deadline, "one saved targeted reply, without reading the room API")
            if len(last) != 1 or identity(last[0]["author"]) != identity(members[0]) or last[0]["text"].strip() != token + "-targeted":
                raise RuntimeError("Targeted reply attribution/content mismatch")
            # Reading/joining cannot dispatch history or fan out agent replies.
            for _ in range(3):
                call(socket, "room.read", {"workspace_id": workspace})
                call(socket, "room.get", {"workspace_id": workspace})
                time.sleep(1)
            transcript = call(socket, "room.read", {"workspace_id": workspace})["messages"]
            arrivals = [m for m in transcript if m.get("arrival")]
            conversation = [m for m in transcript if not m.get("arrival")]
            if (len(arrivals) != 2 or any(m["text"] != "Joined the room." for m in arrivals) or
                {json.dumps(identity(m["author"]), sort_keys=True) for m in arrivals} !=
                {json.dumps(identity(m), sort_keys=True) for m in members}):
                raise RuntimeError("Expected one deterministic attributed arrival per session")
            if len(conversation) != 5 or sum(m.get("author") is not None for m in conversation) != 3:
                raise RuntimeError("Unexpected replay/reply fanout in transcript")
            info = call(socket, "room.get", {"workspace_id": workspace})
            if len(info["deliveries"]) != 3 or any(d["status"] != "replied" for d in info["deliveries"]):
                raise RuntimeError("Delivery statuses did not settle to exactly three replied entries")
            print("PASS: broadcast -> 2 attributed real replies; targeted -> 1; automatic delivery before reads; no replay/fanout.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--provider2")
    parser.add_argument("--model2")
    parser.add_argument("--thinking", default="low", choices=["off", "minimal", "low", "medium", "high"])
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--pi-config", help="installed Pi agent config to privately copy (default PI_CODING_AGENT_DIR or ~/.pi/agent)")
    args = parser.parse_args()
    if not 15 <= args.timeout <= 600:
        parser.error("--timeout must be 15..600 seconds")
    try:
        run(args)
    except (OSError, ValueError, RuntimeError, KeyError) as error:
        # Do not dump terminal, provider responses, environment or auth contents.
        raise SystemExit("Room smoke failed: " + str(error)) from None


if __name__ == "__main__":
    main()
