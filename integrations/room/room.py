#!/usr/bin/env python3
"""Direct Herdr room client. Never injects terminal input or starts a server.

Human posts enqueue automatic Pi delivery on a delivery-enabled server; reads and
agent replies never dispatch. Socket access is trusted, not a security sandbox.

Always require an explicit socket so a disposable prototype cannot accidentally
address an inherited production Herdr socket. Local socket access is trusted,
not authenticated per-agent provenance. Never retry a write automatically.
"""
import argparse
import json
import socket
import sys
import uuid

MAX_RESPONSE = 2 * 1024 * 1024


def call(path, method, params):
    request_id = str(uuid.uuid4())
    request = {"id": request_id, "method": method, "params": params}
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        stream.settimeout(10)
        stream.connect(path)
        stream.sendall(json.dumps(request).encode() + b"\n")
        with stream.makefile("rb") as reader:
            raw = reader.readline(MAX_RESPONSE + 1)
        if not raw.endswith(b"\n") or len(raw) > MAX_RESPONSE:
            raise RuntimeError("missing or oversized response; write outcome unknown, do not blindly retry")
    response = json.loads(raw)
    if response.get("id") != request_id:
        raise RuntimeError("response id mismatch; write outcome unknown")
    if "error" in response:
        raise RuntimeError(json.dumps(response["error"]))
    return response["result"]


def recipient(path, workspace, pane):
    info = call(path, "room.get", {"workspace_id": workspace})
    member = next((m for m in info["members"] if m["pane_id"] == pane), None)
    if not member or not member.get("session"):
        raise RuntimeError("recipient is not a current session-identified room member")
    return {key: member[key] for key in ("pane_id", "terminal_id", "session")}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", required=True, help="explicit disposable server API socket")
    parser.add_argument("workspace", help="workspace public id (not a grouped worktree space)")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("get")
    read = commands.add_parser("read")
    read.add_argument("--after", type=int, default=0)
    read.add_argument("--limit", type=int, default=100)
    post = commands.add_parser("post")
    post.add_argument("--to", help="one current pane id; omitted means all current workspace agents")
    post.add_argument("text", help="literal message, or - to read stdin")
    reply = commands.add_parser("reply")
    reply.add_argument("request", type=int, help="original human request sequence")
    reply.add_argument("--pane", required=True, help="replying current pane id")
    reply.add_argument("text", help="reply/refusal, or - to read stdin")
    args = parser.parse_args(argv)
    params = {"workspace_id": args.workspace}
    if args.command == "read":
        params.update(after_sequence=args.after, limit=args.limit)
    elif args.command in ("post", "reply"):
        text = sys.stdin.read(8193) if args.text == "-" else args.text
        if not text.strip() or len(text.encode()) > 8192:
            parser.error("text must contain 1..8192 UTF-8 bytes")
        params["text"] = text
        if args.command == "post" and args.to:
            params["recipient"] = recipient(args.socket, args.workspace, args.to)
        elif args.command == "reply":
            params.update(recipient(args.socket, args.workspace, args.pane))
            params["request_sequence"] = args.request
    result = call(args.socket, "room." + args.command, params)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError) as error:
        print(f"room: {error} (no automatic retry)", file=sys.stderr)
        sys.exit(1)
