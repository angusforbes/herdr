"""Real installed Pi loader/TUI startup, fake socket, zero model requests.

Run explicitly: python3 -m unittest discover -s integrations/room -p test_pi_startup.py -v
The temporary PI_CODING_AGENT_DIR is populated exactly like the preview launcher.
"""
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest
import fcntl
import termios

import disposable


class PiStartupTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("pi"), "installed Pi required")
    def test_real_loader_discovers_both_hooks_without_agent_flags_or_global_hooks(self):
        self.check_loader(startup_optin=True)

    @unittest.skipUnless(shutil.which("pi"), "installed Pi required")
    def test_real_loader_registers_commands_with_optout_then_enable_disable_reload(self):
        self.check_loader(startup_optin=False)

    @unittest.skipUnless(shutil.which("pi"), "installed Pi required")
    def test_hot_update_cached_optin_receiver_then_reload(self):
        self.check_loader(startup_optin=True, hot_update=True)

    @unittest.skipUnless(shutil.which("pi"), "installed Pi required")
    def test_hot_update_preserves_explicit_optout(self):
        self.check_loader(startup_optin=False, hot_update=True)

    def check_loader(self, startup_optin, hot_update=False):
        with tempfile.TemporaryDirectory(prefix="room-pi-load-") as directory:
            base = Path(directory)
            source = base / "source"
            source.mkdir()
            # Prove global exclusions/packages are not copied into this runtime.
            (source / "settings.json").write_text(json.dumps({"extensions": ["!**"], "packages": ["npm:not-installed-do-not-load"]}))
            env = disposable.environment(base, source)
            if not startup_optin:
                env["HERDR_ROOM_ENABLED"] = "0"
            env.update(HERDR_ENV="1", HERDR_PANE_ID="w1.p1", HERDR_WORKSPACE_ID="w1", TERM="xterm-256color")
            env.pop("PI_PACKAGE_DIR", None)
            if hot_update:
                extension = Path(env["PI_CODING_AGENT_DIR"]) / "extensions/herdr-room"
                entry = extension / "index.ts"
                receiver = extension / "receiver.mjs"
                current_entry = entry.read_text()
                current_receiver = receiver.read_text()
                # Simulate the pre-autojoin deployment, not a fresh process
                # loading an already-updated dependency. Keep its real transport.
                old_policy = 'env.HERDR_ROOM_ENABLED === "1"'
                default_policy = ('env.HERDR_ROOM_ENABLED !== "0"\n'
                                  '      && env.HERDR_SOCKET_PATH?.startsWith("/") === true\n'
                                  '      && string(env.HERDR_PANE_ID)')
                self.assertIn(default_policy, current_receiver)
                constructor_log = base / "constructors.jsonl"
                def instrument(text, revision):
                    return ('import { appendFileSync } from "node:fs";\n' + text.replace(
                        '    this.pi = pi;',
                        '    appendFileSync(' + json.dumps(str(constructor_log)) + ', JSON.stringify('
                        + '{ revision: ' + json.dumps(revision)
                        + ', enabled: env.HERDR_ROOM_ENABLED ?? null }) + "\\n");\n'
                        + '    this.pi = pi;'))
                receiver.write_text(instrument(current_receiver.replace(default_policy, old_policy), "old"))
                current_receiver = instrument(current_receiver, "new")
                entry.write_text('import { RoomReceiver, registerRoomLifecycle } from "./receiver.mjs";\n'
                                 'import { contextualRoomTransport } from "./room-context.mjs";\n'
                                 'export default function(pi) {\n'
                                 '  const context = contextualRoomTransport(pi);\n'
                                 '  const receiver = new RoomReceiver({...pi, sendMessage:context.sendMessage}, process.env, {call:context.call});\n'
                                 '  registerRoomLifecycle(pi, receiver);\n'
                                 '}\n')
                if startup_optin:
                    env.pop("HERDR_ROOM_ENABLED", None)
            endpoint = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            endpoint.bind(env["HERDR_SOCKET_PATH"])
            endpoint.listen()
            endpoint.settimeout(0.1)
            stop = threading.Event()
            ready = threading.Event()
            requests = []
            arrivals = {}
            failures = []
            member = {"pane_id": "w1.p1", "terminal_id": "t1", "agent": "pi", "name": "startup", "session": None}

            def serve():
                while not stop.is_set():
                    try:
                        stream, _ = endpoint.accept()
                    except socket.timeout:
                        continue
                    except OSError:
                        return
                    with stream:
                        stream.settimeout(2)
                        try:
                            raw = stream.makefile("rb").readline(65537)
                            req = json.loads(raw)
                            requests.append(req["method"])
                            params = req["params"]
                            if req["method"] == "pane.report_agent_session":
                                member["session"] = "Path:" + params["agent_session_path"] if params.get("agent_session_path") else "Id:" + params["agent_session_id"]
                                result = {}
                            elif req["method"] == "room.get":
                                if params != {"workspace_id": env["HERDR_WORKSPACE_ID"]}:
                                    raise AssertionError("receiver did not use explicit workspace")
                                result = {"members": [dict(member)] if member["session"] else []}
                            elif req["method"] == "room.delivery.register":
                                if (params["workspace_id"] != env["HERDR_WORKSPACE_ID"] or
                                    any(params[key] != member[key] for key in ("pane_id", "terminal_id", "session"))):
                                    raise AssertionError("receiver did not use exact live hook identity")
                                result = {"receiver_id": "r1", "server_epoch": "e1"}
                            elif req["method"] == "room.agent.post":
                                if (params["workspace_id"] != env["HERDR_WORKSPACE_ID"] or
                                    any(params[key] != member[key] for key in ("pane_id", "terminal_id", "session")) or
                                    params.get("arrival", False) or params["text"] != "Hi, I'm startup."):
                                    raise AssertionError("arrival did not use captured binding/deterministic text")
                                key = len(arrivals)
                                arrivals[key] = key + 1
                                result = {"persistence": "saved", "sequence": arrivals[key]}
                            elif req["method"] == "room.delivery.claim":
                                result = {"delivery": None}
                                if params["ready"]:
                                    ready.set()
                            elif req["method"] == "pane.report_agent":
                                result = {}
                            else:
                                raise AssertionError("unexpected startup API: " + req["method"])
                            stream.sendall(json.dumps({"id": req["id"], "result": result}).encode() + b"\n")
                        except Exception as error:
                            failures.append(type(error).__name__)

            thread = threading.Thread(target=serve, daemon=True)
            thread.start()
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
            process = subprocess.Popen([shutil.which("pi")], cwd=directory, env=env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            output = bytearray()
            def drain_for(seconds):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    if select.select([master], [], [], .1)[0]:
                        output.extend(os.read(master, 65536))

            def command(text):
                os.write(master, text.encode())
                drain_for(.2)
                os.write(master, b'\r')

            try:
                if hot_update:
                    drain_for(5)
                    self.assertIn("pane.report_agent_session", requests)
                    self.assertFalse(any(r.startswith("room.") for r in requests))
                    receiver.write_text(current_receiver)
                    entry.write_text(current_entry)
                    command('/reload')
                    drain_for(4)
                    self.assertIn(b'Reloaded keybindings', output)
                    constructors = [json.loads(line) for line in constructor_log.read_text().splitlines()]
                    self.assertEqual([c["revision"] for c in constructors], ["old", "old"],
                                     "installed Pi keeps the native .mjs constructor cached across reload")
                    self.assertEqual(constructors[0]["enabled"], None if startup_optin else "0")
                    self.assertEqual(constructors[1]["enabled"], "1" if startup_optin else "0")
                if not startup_optin:
                    drain_for(5)
                    self.assertIn("pane.report_agent_session", requests)
                    self.assertFalse(any(r.startswith("room.") for r in requests))
                    command('/room-enable')
                deadline = time.monotonic() + 30
                while not ready.is_set() and time.monotonic() < deadline and process.poll() is None:
                    readable, _, _ = select.select([master], [], [], 0.1)
                    if readable:
                        try:
                            output.extend(os.read(master, 65536))
                            # Never print terminal contents (may contain config). Private file only.
                            if len(output) > 512 * 1024:
                                del output[:len(output) - 512 * 1024]
                        except OSError:
                            break
                if not ready.is_set():
                    # Only report known loader errors, not arbitrary terminal output.
                    hints = [phrase for phrase in ("Cannot find module", "Failed to load extension", "SyntaxError", "No models available") if phrase.encode() in output]
                    self.fail(f"Pi hooks not ready; exit={process.poll()}, APIs={requests}, loader hints={hints}, errors={failures}")
                self.assertIn("pane.report_agent_session", requests)
                self.assertIn("room.delivery.register", requests)
                self.assertEqual(requests.count("room.agent.post"), 1)
                self.assertEqual(len(arrivals), 1)
                self.assertNotIn("room.reply", requests)
                self.assertNotIn("room.post", requests)
                self.assertNotIn("room.read", requests)
                self.assertNotIn("workspace.list", requests)
                self.assertTrue(member["session"].startswith("Path:" + directory))
                self.assertEqual(failures, [])
                if not startup_optin:
                    command('/room-disable')
                    drain_for(1)
                    count = sum(r.startswith('room.') for r in requests)
                    drain_for(2)
                    self.assertEqual(sum(r.startswith('room.') for r in requests), count)
                    command('/reload')
                    drain_for(4)
                    self.assertEqual(sum(r.startswith('room.') for r in requests), count)
                    ready.clear()
                    command('/room-enable')
                    drain_for(3)
                    self.assertTrue(ready.is_set())
                    self.assertIsNone(process.poll())
                    self.assertEqual(requests.count("room.agent.post"), 2)
                    self.assertEqual(len(arrivals), 2, "reload/re-enable must introduce again")
                else:
                    ready.clear()
                    command('/reload')
                    drain_for(4)
                    self.assertTrue(ready.is_set())
                    self.assertEqual(requests.count("room.agent.post"), 2)
                    self.assertEqual(len(arrivals), 2, "same session reload must introduce again")
                self.assertEqual(failures, [])
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
                os.close(master)
                stop.set()
                endpoint.close()
                thread.join(timeout=3)


if __name__ == "__main__":
    unittest.main()
