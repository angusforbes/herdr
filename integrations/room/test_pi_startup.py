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
        with tempfile.TemporaryDirectory(prefix="room-pi-load-") as directory:
            base = Path(directory)
            source = base / "source"
            source.mkdir()
            # Prove global exclusions/packages are not copied into this runtime.
            (source / "settings.json").write_text(json.dumps({"extensions": ["!**"], "packages": ["npm:not-installed-do-not-load"]}))
            env = disposable.environment(base, source)
            env.update(HERDR_ENV="1", HERDR_PANE_ID="w1.p1", HERDR_WORKSPACE_ID="w1", TERM="xterm-256color")
            env.pop("PI_PACKAGE_DIR", None)
            endpoint = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            endpoint.bind(env["HERDR_SOCKET_PATH"])
            endpoint.listen()
            endpoint.settimeout(0.1)
            stop = threading.Event()
            ready = threading.Event()
            requests = []
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
                                result = {"members": [dict(member)] if member["session"] else []}
                            elif req["method"] == "room.delivery.register":
                                if params["session"] != member["session"]:
                                    raise AssertionError("receiver did not use live hook identity")
                                result = {"receiver_id": "r1", "server_epoch": "e1"}
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
            try:
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
                self.assertNotIn("room.reply", requests)
                self.assertNotIn("room.post", requests)
                self.assertTrue(member["session"].startswith("Path:" + directory))
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
