#!/usr/bin/env python3
"""Run this fork in a temporary, isolated TUI session; never install or hand off."""
from contextlib import contextmanager
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time

from room import call

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/herdr"


def environment(base):
    base = Path(base)
    config = base / "config/herdr"
    config.mkdir(parents=True, exist_ok=True)
    runtime = base / "runtime"
    runtime.mkdir(exist_ok=True, mode=0o700)
    path = config / "config.toml"
    path.write_text('onboarding = false\n[session]\nresume_agents_on_restore = false\n[terminal]\ndefault_shell = "/bin/sh"\n')
    env = dict(os.environ)
    for key in list(env):
        if key.startswith("HERDR_"):
            del env[key]
    env.update(XDG_CONFIG_HOME=str(base / "config"), XDG_STATE_HOME=str(base / "state"), XDG_RUNTIME_DIR=str(runtime),
               HERDR_CONFIG_PATH=str(path), HERDR_SOCKET_PATH=str(runtime / "api.sock"),
               HERDR_CLIENT_SOCKET_PATH=str(runtime / "client.sock"), SHELL="/bin/sh")
    return env


@contextmanager
def server(directory):
    env = environment(directory)
    log_path = Path(directory) / "server.log"
    with log_path.open("a") as log:
        process = subprocess.Popen([str(BINARY), "server"], env=env, cwd=ROOT,
                                   stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while True:
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("disposable server did not start: " + log_path.read_text())
                try:
                    call(env["HERDR_SOCKET_PATH"], "ping", {})
                    break
                except (OSError, ValueError, RuntimeError):
                    time.sleep(0.05)
            yield env
        finally:
            if process.poll() is None:
                try:
                    call(env["HERDR_SOCKET_PATH"], "server.stop", {})
                    process.wait(timeout=10)
                except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired):
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


def main():
    if not BINARY.is_file():
        raise SystemExit("Build this checkout first: mise x zig@0.15.2 -- cargo build --locked")
    with tempfile.TemporaryDirectory(prefix="herdr-rooms-demo-") as directory, server(directory) as env:
        socket = env["HERDR_SOCKET_PATH"]
        print("Disposable room server data:", directory, flush=True)
        print("API helper (replace w1 with workspace id if needed):", flush=True)
        print(shlex.join(["python3", str(ROOT / "integrations/room/room.py"), "--socket", socket, "w1", "get"]), flush=True)
        print("Click room or press Ctrl+Alt+R. Outbound prompting is disabled. Exit deletes this disposable session.", flush=True)
        subprocess.run([str(BINARY)], env=env, cwd=ROOT, check=False)


if __name__ == "__main__":
    main()
