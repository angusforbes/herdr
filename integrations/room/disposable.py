#!/usr/bin/env python3
"""Run this fork in a temporary, isolated TUI session; never install or hand off."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import tomllib

from room import call

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/herdr"


def pi_config(base, source=None):
    """Private config, never symlink credentials or copy executable global hooks.

    Config resource lists/exclusions are deliberately NOT inherited: an inherited
    '!**' can disable the two preview hooks; packages can start real orchestration.
    Project discovery is declined by default, not globally --no-extensions (which
    would disable these auto-discovered hooks too). Explicit -ne opts out.
    """
    source = Path(source or os.environ.get("PI_CODING_AGENT_DIR") or Path.home() / ".pi/agent").expanduser()
    target = Path(base) / "pi-agent"
    target.mkdir(mode=0o700, parents=True, exist_ok=True)
    target.chmod(0o700)
    extensions = target / "extensions"
    extensions.mkdir(exist_ok=True)
    shutil.copy2(ROOT / "src/integration/assets/pi/herdr-agent-state.ts", extensions / "herdr-agent-state.ts")
    shutil.copytree(ROOT / "integrations/room/pi", extensions / "herdr-room", dirs_exist_ok=True)
    # Keep refreshed private credentials on a server restart. No contents logged.
    for name in ("auth.json", "models.json", "models-store.json"):
        destination = target / name
        if not destination.exists() and (source / name).is_file():
            with destination.open("xb") as stream:
                destination.chmod(0o600)
                stream.write((source / name).read_bytes())
    settings_path = target / "settings.json"
    if not settings_path.exists():
        settings = {}
        if (source / "settings.json").is_file():
            original = json.loads((source / "settings.json").read_text())
            for key in ("defaultProvider", "defaultModel", "defaultThinkingLevel", "modelThinkingLevels", "thinkingBudgets", "enabledModels", "transport", "httpProxy", "httpIdleTimeoutMs", "websocketConnectTimeoutMs", "hideThinkingBlock"):
                if key in original:
                    settings[key] = original[key]
        settings.update(defaultProjectTrust="never", packages=[], extensions=[], skills=[], prompts=[], themes=[],
                        enableInstallTelemetry=False, quietStartup=True, theme="dark")
        settings_path.write_text(json.dumps(settings))
        settings_path.chmod(0o600)
    return target


def herdr_config(source=None):
    """Copy presentation/keybindings, not live sessions, plugins or startup actions."""
    source = Path(source or os.environ.get("HERDR_CONFIG_PATH") or
                  Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "herdr/config.toml")
    original = tomllib.loads(source.read_text()) if source.is_file() else {}
    config = {key: original[key] for key in ("theme", "keys", "ui") if key in original}
    config.update(onboarding=False, session={"resume_agents_on_restore": False},
                  terminal={"default_shell": "/bin/sh", "new_cwd": original.get("terminal", {}).get("new_cwd", "follow")})

    def value(item):
        if isinstance(item, bool):
            return "true" if item else "false"
        if isinstance(item, str):
            return json.dumps(item, ensure_ascii=False)
        if isinstance(item, (int, float)):
            return str(item)
        if isinstance(item, list):
            return "[" + ", ".join(value(v) for v in item) + "]"
        if isinstance(item, dict):
            return "{ " + ", ".join(f"{json.dumps(k)} = {value(v)}" for k, v in item.items()) + " }"
        raise ValueError("Unsupported Herdr presentation config value")

    return "\n".join(f"{key} = {value(item)}" for key, item in config.items()) + "\n"


def environment(base, pi_source=None):
    base = Path(base)
    config = base / "config/herdr"
    config.mkdir(parents=True, exist_ok=True)
    runtime = base / "runtime"
    runtime.mkdir(exist_ok=True, mode=0o700)
    path = config / "config.toml"
    path.write_text(herdr_config())
    env = dict(os.environ)
    for key in list(env):
        if key.startswith("HERDR_") or key in ("PI_SESSION_ID", "PI_SESSION_FILE", "PI_PROVIDER", "PI_MODEL", "PI_REASONING_LEVEL", "PI_CODING_AGENT_SESSION_DIR"):
            del env[key]
    agent_dir = pi_config(base, pi_source)
    env.update(XDG_CONFIG_HOME=str(base / "config"), XDG_STATE_HOME=str(base / "state"), XDG_RUNTIME_DIR=str(runtime),
               HERDR_CONFIG_PATH=str(path), HERDR_SOCKET_PATH=str(runtime / "api.sock"),
               HERDR_CLIENT_SOCKET_PATH=str(runtime / "client.sock"), SHELL="/bin/sh",
               HERDR_ROOM_ENABLED="1", PI_CODING_AGENT_DIR=str(agent_dir),
               PI_CODING_AGENT_SESSION_DIR=str(agent_dir / "sessions"), PI_OFFLINE="1", PI_TELEMETRY="0")
    return env


@contextmanager
def server(directory, pi_source=None):
    env = environment(directory, pi_source)
    log_path = Path(directory) / "server.log"
    with log_path.open("a") as log:
        process = subprocess.Popen([str(BINARY), "server"], env=env, cwd=ROOT,
                                   stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while True:
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("disposable server did not start; inspect private log: " + str(log_path))
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
        print("Click room or press Ctrl+Alt+R. Add Pi agents normally (UI or type pi): both local hooks load automatically.", flush=True)
        print("Human posts broadcast to ready Pi receivers by default; optional single target. Replies do not fan out. Exit deletes this session.", flush=True)
        print("Global extensions/packages and project resources are not inherited; auth/model config is privately copied. Do not pass --no-extensions.", flush=True)
        subprocess.run([str(BINARY)], env=env, cwd=ROOT, check=False)


if __name__ == "__main__":
    main()
