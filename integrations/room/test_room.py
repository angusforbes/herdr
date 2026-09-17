"""Run: python3 -m unittest discover -s integrations/room -v

The end-to-end test starts only this checkout's binary, on temporary sockets,
with temporary config and agent resumption disabled. No live agent is prompted.
"""
from contextlib import redirect_stdout, redirect_stderr
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import disposable
import room


class HelperTests(unittest.TestCase):
    def test_reply_goes_directly_to_room_api_without_prompt_or_retry(self):
        member = {"pane_id": "w1.p1", "terminal_id": "t1", "session": "Path:/tmp/session"}
        with patch.object(room, "call", side_effect=[{"members": [member]}, {"type": "room_written"}]) as api:
            with redirect_stdout(io.StringIO()):
                room.main(["--socket", "/tmp/explicit.sock", "w1", "reply", "9", "--pane", "w1.p1", "I decline"])
            self.assertEqual([args.args[1] for args in api.call_args_list], ["room.get", "room.reply"])
            self.assertEqual(api.call_args.args[2]["request_sequence"], 9)
            self.assertEqual(api.call_args.args[2]["session"], member["session"])

    def test_unknown_member_does_not_post(self):
        with patch.object(room, "call", return_value={"members": []}) as api:
            with self.assertRaises(RuntimeError):
                room.main(["--socket", "/tmp/explicit.sock", "w1", "post", "--to", "w1.p9", "question"])
            self.assertEqual(api.call_count, 1)

    def test_socket_must_be_explicit(self):
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            room.main(["w1", "get"])

    def test_write_exception_is_not_retried(self):
        with patch.object(room, "call", side_effect=TimeoutError("unknown")) as api:
            with self.assertRaises(TimeoutError):
                room.main(["--socket", "/tmp/explicit.sock", "w1", "post", "note"])
            self.assertEqual(api.call_count, 1)


class DisposableServerTests(unittest.TestCase):
    def test_room_saved_before_ack_and_restored_without_agent_dispatch(self):
        self.assertTrue(disposable.BINARY.is_file(), "build target/debug/herdr first")
        with tempfile.TemporaryDirectory(prefix="herdr-rooms-smoke-") as directory:
            with disposable.server(directory) as env:
                socket = env["HERDR_SOCKET_PATH"]
                created = room.call(socket, "workspace.create", {"label": "room-smoke", "cwd": directory})
                workspace = created["workspace"]["workspace_id"]
                params = {"workspace_id": workspace}
                info = room.call(socket, "room.get", params)
                self.assertEqual(info["members"], [])
                self.assertEqual(info["outbound_delivery"], "disabled_manual_pull_only")
                first = room.call(socket, "room.post", {**params, "text": "human durable note"})
                self.assertEqual(first["persistence"], "saved")
                saved = json.loads((Path(directory) / "config/herdr-dev/session.json").read_text())
                stored = next(ws for ws in saved["workspaces"] if ws["id"] == workspace)
                self.assertEqual(stored["room"]["messages"][0]["text"], "human durable note")
                room.call(socket, "room.post", {**params, "text": "second"})
                page = room.call(socket, "room.read", {**params, "after_sequence": 1, "limit": 1})
                self.assertEqual([m["sequence"] for m in page["messages"]], [2])
            with disposable.server(directory) as env:
                socket = env["HERDR_SOCKET_PATH"]
                restored = room.call(socket, "room.get", params)
                self.assertEqual(restored["room_id"], info["room_id"])
                self.assertEqual(restored["members"], [])
                transcript = room.call(socket, "room.read", params)
                self.assertEqual([m["text"] for m in transcript["messages"]], ["human durable note", "second"])
                self.assertEqual(transcript["next_sequence"], 3)


if __name__ == "__main__":
    unittest.main()
