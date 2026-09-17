#!/usr/bin/env python3
"""Opt-in real-Pi OLD -> candidate handoff. No installation or production access.

Keeps only allowlisted evidence; temporary auth/config/session data is deleted.
A starts with the receiver and opt-in; B starts without either. Neither Pi is
restarted. B hot-loads via /reload then explicitly /room-enable. A streams a real
model response through handoff; a real Herdr TUI explicitly reattaches on its
owned PTY (the installed client exits on handoff, it does not auto-reconnect).
Linux-only ownership checks: explicit socket + SO_PEERCRED + /proc start ticks.
"""
import ctypes
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import threading
import time

import disposable
from room import call

OLD = Path.home() / '.local/bin/herdr'
NEW = disposable.BINARY.resolve()
ROOT = disposable.ROOT


def wait(check, seconds, description):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.2)
    raise RuntimeError('timeout: ' + description)


def proc(pid):
    try:
        root = Path('/proc') / str(pid)
        stat = (root / 'stat').read_text().rsplit(')', 1)[1].split()
        return {'pid': int(pid), 'start_ticks': int(stat[19]), 'state': stat[0],
                'ppid': int(stat[1]), 'comm': (root / 'comm').read_text().strip(),
                'exe': os.readlink(root / 'exe'), 'stdin': os.readlink(root / 'fd/0')}
    except (OSError, ValueError):
        return None


def owned_processes(env):
    # Read only for ownership; never return/log an environment or command line.
    wanted = ('HERDR_SOCKET_PATH=' + env['HERDR_SOCKET_PATH']).encode()
    found = []
    for p in Path('/proc').iterdir():
        if not p.name.isdigit():
            continue
        try:
            if wanted not in (p / 'environ').read_bytes().split(b'\0'):
                continue
            item = proc(p.name)
            if item:
                found.append(item)
        except OSError:
            pass
    return found


def same_process(item):
    now = proc(item['pid'])
    return bool(now and now['start_ticks'] == item['start_ticks'] and now['state'] != 'Z')


class TuiClient:
    """Real installed Herdr client, owned PTY; bounded drain prevents backpressure."""
    def __init__(self, env, cwd):
        self.master, self.slave = pty.openpty()
        self.env, self.cwd = {**env, 'TERM': 'xterm-256color'}, cwd
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 45, 160, 0, 0))
        self.attach()
        self.output = bytearray()
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.drain, daemon=True)
        self.thread.start()

    def attach(self):
        self.process = subprocess.Popen([str(OLD)], env=self.env, cwd=self.cwd,
                                        stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                        start_new_session=True)

    def drain(self):
        while not self.stop.is_set():
            if not select.select([self.master], [], [], .1)[0]:
                continue
            try:
                data = os.read(self.master, 65536)
            except OSError:
                return
            if not data:
                return
            with self.lock:
                self.output.extend(data)
                if len(self.output) > 2 * 1024 * 1024:
                    del self.output[:-2 * 1024 * 1024]

    def snapshot(self):
        with self.lock:
            return bytes(self.output)

    def rendered(self, token):
        raw = self.snapshot().decode('utf-8', errors='replace')
        plain = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)', '', raw)
        return token in plain

    def send(self, text):
        assert self.process.poll() is None, 'attached Herdr client exited'
        os.write(self.master, b'\x1b[200~' + text.encode() + b'\x1b[201~')
        time.sleep(.2)
        os.write(self.master, b'\r')

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        self.stop.set()
        self.thread.join(timeout=2)
        os.close(self.master)
        os.close(self.slave)


class Trial:
    def __init__(self, directory, evidence):
        self.base = Path(directory).resolve()
        self.evidence = evidence
        self.env = disposable.environment(self.base)
        self.env['PI_SKIP_VERSION_CHECK'] = '1'
        self.sock = self.env['HERDR_SOCKET_PATH']
        assert Path(self.sock).is_relative_to(self.base)
        assert self.sock != '/home/agf/.config/herdr/herdr.sock'
        assert not any(k in self.env for k in ('HERDR_SESSION', 'HERDR_ENV', 'HERDR_PANE_ID'))
        self.old = None
        self.client = None
        self.observer = self.base / 'observer.jsonl'
        self.known = {}
        self.data = {'base': str(self.base), 'api_socket': self.sock, 'events': []}

    def record(self, event, data):
        self.data['events'].append({'event': event, 'time_unix': time.time(), 'data': data})
        self.evidence.write_text(json.dumps(self.data, indent=2) + '\n')
        print(event, flush=True)

    def api(self, method, params=None):
        assert Path(self.sock).is_relative_to(self.base)
        return call(self.sock, method, params or {})

    def peer(self):
        with socket.socket(socket.AF_UNIX) as s:
            s.connect(self.sock)
            pid, uid, _ = struct.unpack('3i', s.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        assert uid == os.getuid()
        owned = {p['pid']: p for p in owned_processes(self.env)}
        assert pid in owned, 'socket peer is not our disposable process'
        item = owned[pid]
        assert item['exe'] in (str(OLD.resolve()), str(NEW))
        self.known[pid] = item
        return item

    def ping(self):
        try:
            return self.api('ping')
        except (OSError, ValueError, RuntimeError):
            return None

    def text(self, pane):
        return self.api('pane.read', {'pane_id': pane, 'source': 'detection', 'format': 'text'})['read']['text']

    def send(self, pane, text):
        self.api('pane.send_text', {'pane_id': pane, 'text': text})
        time.sleep(.2)
        self.api('pane.send_keys', {'pane_id': pane, 'keys': ['enter']})

    def agents(self):
        return [a for a in self.api('agent.list')['agents'] if a['pane_id'] in self.panes]

    def sessions(self):
        agents = self.agents()
        if len(agents) != 2 or any(not a.get('agent_session') for a in agents):
            return None
        result = []
        for pane in self.panes:
            a = next(a for a in agents if a['pane_id'] == pane)
            path = Path(a['agent_session']['value'])
            if not path.is_file():
                return None
            assert path.is_relative_to(self.base)
            entries = [json.loads(line) for line in path.read_text().splitlines()]
            header = next(e for e in entries if e['type'] == 'session')
            result.append({'pane_id': pane, 'terminal_id': a['terminal_id'],
                           'session_id': header['id'], 'session_file': str(path),
                           'agent_status': a['agent_status']})
        return result

    def observed(self):
        if not self.observer.exists():
            return []
        # Ignore only an unfinished final append, never a malformed completed row.
        return [json.loads(line) for line in self.observer.read_text().splitlines(keepends=True) if line.endswith('\n')]

    def named(self, session, name):
        return any(e.get('type') == 'session_info' and e.get('name') == name
                   for e in map(json.loads, Path(session['session_file']).read_text().splitlines()))

    def answer(self, session, token):
        entries = [json.loads(line) for line in Path(session['session_file']).read_text().splitlines()]
        return any(e.get('message', {}).get('role') == 'assistant' and any(
            c.get('type') == 'text' and c.get('text', '').strip() == token
            for c in e['message'].get('content', []) if isinstance(c, dict)) for e in entries)

    def run(self):
        for binary, label in ((OLD, 'old'), (NEW, 'candidate')):
            version = subprocess.run([str(binary), '--version'], env=self.env, cwd=self.base,
                                     text=True, capture_output=True, check=True).stdout.strip()
            help_result = subprocess.run([str(binary), 'server', 'live-handoff', '--help'],
                                        env=self.env, cwd=self.base, text=True, capture_output=True)
            self.record(label + '-binary', {'path': str(binary), 'version': version,
                'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'handoff_help': help_result.stdout + help_result.stderr})
        self.log = (self.base / 'server.log').open('w')
        self.old = subprocess.Popen([str(OLD), 'server'], env=self.env, cwd=self.base,
                                    stdin=subprocess.DEVNULL, stdout=self.log, stderr=self.log)
        wait(self.ping, 15, 'old server')
        self.record('old-server', {'process': self.peer(), 'ping': self.ping()})
        a = self.api('workspace.create', {'label': 'handoff-only', 'cwd': str(self.base)})
        self.workspace = a['workspace']['workspace_id']
        b = self.api('tab.create', {'workspace_id': self.workspace, 'cwd': str(self.base), 'label': 'no-opt-in'})
        self.panes = [a['root_pane']['pane_id'], b['root_pane']['pane_id']]
        second_dir = disposable.pi_config(self.base / 'second')
        shutil.rmtree(second_dir / 'extensions/herdr-room')
        time.sleep(.5)
        pi = shutil.which('pi')
        assert pi
        observer_extension = self.base / 'observer.ts'
        shutil.copy2(ROOT / 'integrations/room/handoff_observer.ts', observer_extension)
        args = [pi, '--extension', str(observer_extension), '--provider', 'anthropic', '--model', 'claude-haiku-4-5', '--thinking', 'low',
                '--no-approve', '--no-context-files', '--no-builtin-tools', '--system-prompt',
                'You are a disposable handoff test participant. Follow ordinary user output instructions exactly, no tools. For a BEGIN ROOM QUESTION use room_reply once. No other activity.']
        for n, pane in enumerate(self.panes):
            prefix = ['env']
            if n:
                prefix += ['-u', 'HERDR_ROOM_ENABLED', 'PI_CODING_AGENT_DIR=' + str(second_dir),
                           'PI_CODING_AGENT_SESSION_DIR=' + str(second_dir / 'sessions')]
            prefix += ['HANDOFF_OBSERVER_PATH=' + str(self.observer)]
            self.send(pane, 'exec ' + shlex.join(prefix + args))
        # First answers also force session JSONL files to be flushed to disk.
        time.sleep(5)
        for n, pane in enumerate(self.panes):
            self.send(pane, f'Reply with exactly BEFORE-{n}')
        before = wait(self.sessions, 45, 'two live Pi identities')
        for n, session in enumerate(before):
            wait(lambda: self.answer(session, f'BEFORE-{n}'), 90, f'before answer {n}')
        self.record('before-sessions', before)
        processes = owned_processes(self.env)
        self.known.update({p['pid']: p for p in processes})
        self.record('before-processes', processes)
        self.record('old-receiver-display', {'a': self.text(self.panes[0]), 'b': self.text(self.panes[1])})
        self.before_pi = [p for p in processes if p['stdin'].startswith('/dev/pts/')]
        assert len(self.before_pi) == 2
        self.api('pane.focus', {'pane_id': self.panes[1]})
        self.client = TuiClient(self.env, self.base)
        wait(lambda: self.client.rendered('BEFORE-1'), 20, 'real attached client render')
        self.client.send('/name CLIENT-BEFORE-HANDOFF')
        wait(lambda: self.named(before[1], 'CLIENT-BEFORE-HANDOFF'), 10, 'pre-handoff client input')
        client_before = proc(self.client.process.pid)
        self.record('client-before', {'process': client_before, 'rendered': 'BEFORE-1', 'input_saved': 'CLIENT-BEFORE-HANDOFF'})
        # Not a delayed tool or a new post-handoff prompt: observe actual streamed
        # provider text before handoff and that same message ending afterward.
        active_prompt = 'Print 180 lines numbered 001 through 180, each with the words harmless continuity check. End with a separate line INFLIGHT-FINISHED. No tools or commentary.'
        baseline_events = len(self.observed())
        self.send(self.panes[0], active_prompt)
        active = wait(lambda: next((e for e in self.observed()[baseline_events:]
                      if e['kind'] == 'first_text_delta' and e['session'] == before[0]['session_id']), None),
                      90, 'actual provider text stream in flight')
        def same_turn(e):
            return all(e[k] == active[k] for k in ('pid', 'session', 'run', 'message'))
        assert not any(e['kind'] == 'assistant_end' and same_turn(e) for e in self.observed()), 'turn already finished before handoff'
        handoff_started_ms = int(time.time() * 1000)
        self.record('inflight-before-handoff', active)
        command = [str(OLD), 'server', 'live-handoff', '--import-exe', str(NEW), '--expected-protocol', '20']
        handoff = subprocess.run(command, env=self.env, cwd=self.base, text=True, capture_output=True, timeout=75)
        self.record('handoff-cli', {'args': command, 'returncode': handoff.returncode,
                                   'stdout': handoff.stdout, 'stderr': handoff.stderr})
        if handoff.returncode:
            raise RuntimeError('handoff command failed; see evidence')
        wait(self.ping, 15, 'candidate server')
        peer = self.peer()
        assert peer['pid'] != self.old.pid and peer['exe'] == str(NEW)
        self.old.wait(timeout=10)
        self.record('candidate-server', {'process': peer, 'ping': self.ping(), 'old_exit': self.old.returncode})
        assert all(same_process(p) for p in self.before_pi), 'PTY/Pi process replaced or lost'
        after = wait(self.sessions, 15, 'identities immediately after handoff')
        self.record('after-sessions', after)
        assert [(s['pane_id'], s['session_id']) for s in before] == [(s['pane_id'], s['session_id']) for s in after]
        candidate_ready_ms = int(time.time() * 1000)
        end = wait(lambda: next((e for e in self.observed() if e['kind'] == 'assistant_end' and same_turn(e)), None),
                   90, 'same in-flight assistant message completed')
        assert end['stop'] == 'stop' and end['time_ms'] > candidate_ready_ms
        entries = list(map(json.loads, Path(after[0]['session_file']).read_text().splitlines()))
        prompts = [e for e in entries if e.get('message', {}).get('role') == 'user' and
                   any(c.get('text') == active_prompt for c in e['message'].get('content', []) if isinstance(c, dict))]
        assert len(prompts) == 1
        completed = [e for e in entries if e.get('parentId') == prompts[0]['id'] and e.get('message', {}).get('role') == 'assistant']
        assert len(completed) == 1 and completed[0]['message']['stopReason'] == 'stop'
        assert any(c.get('type') == 'text' and c.get('text', '').rstrip().endswith('INFLIGHT-FINISHED') for c in completed[0]['message']['content'])
        self.record('same-inflight-turn-finished', {'start': active, 'end': end, 'handoff_started_ms': handoff_started_ms,
                    'candidate_ready_ms': candidate_ready_ms, 'user_entry': prompts[0]['id'], 'assistant_entry': completed[0]['id'],
                    'assistant_parent': completed[0]['parentId'], 'matching_user_prompts': len(prompts)})
        # Installed Herdr deliberately exits on ServerShutdown. Reconnect the
        # client explicitly on the SAME owned PTY, never restart either Pi.
        client_exit = self.client.process.wait(timeout=5)
        assert not same_process(client_before)
        self.record('client-disconnected', {'exit': client_exit, 'automatic_reconnect': False,
                    'handoff_notice_rendered': self.client.rendered('live update in progress')})
        output_before_reattach = len(self.client.snapshot())
        self.client.attach()
        wait(lambda: len(self.client.snapshot()) > output_before_reattach and self.client.process.poll() is None, 15, 'reattached client render')
        time.sleep(1)
        self.client.send('/name CLIENT-AFTER-HANDOFF')
        wait(lambda: self.named(after[1], 'CLIENT-AFTER-HANDOFF'), 15, 'post-handoff input through reattached client PTY')
        wait(lambda: self.client.rendered('CLIENT-AFTER-HANDOFF'), 15, 'post-handoff client render')
        self.client.send('Reply with exactly AFTER-1')
        wait(lambda: self.answer(after[1], 'AFTER-1'), 90, 'post-handoff model answer via reattached client')
        self.record('client-reconnected-render-input', {'process': proc(self.client.process.pid), 'rendered': 'CLIENT-AFTER-HANDOFF',
                    'input_saved': 'CLIENT-AFTER-HANDOFF', 'model_answer': 'AFTER-1', 'output_bytes': len(self.client.snapshot()),
                    'automatic_reconnect': False, 'explicit_reattach_same_pty': proc(self.client.process.pid)['stdin'] == client_before['stdin']})
        self.record('both-answered-after-handoff', {'answers': ['INFLIGHT-FINISHED (same turn)', 'AFTER-1']})
        time.sleep(3)
        self.record('initial-room', self.api('room.get', {'workspace_id': self.workspace}))
        self.record('initial-receiver-display', {'a': self.text(self.panes[0]), 'b': self.text(self.panes[1])})
        # Supported TUI command; do not send /reload via Pi sendUserMessage/RPC.
        # A tests startup opt-in rebind; B hot discovery stays inactive until command.
        shutil.copytree(ROOT / 'integrations/room/pi', second_dir / 'extensions/herdr-room')
        for pane in self.panes:
            self.send(pane, '/reload')
        def a_ready():
            info = self.api('room.get', {'workspace_id': self.workspace})
            return info if any(r['available'] and r['member']['pane_id'] == self.panes[0]
                               for r in info.get('receivers', [])) else None
        wait(a_ready, 25, 'A ready after reload')
        time.sleep(3)
        info = self.api('room.get', {'workspace_id': self.workspace})
        self.record('room-after-reload', info)
        assert not any(r['available'] and r['member']['pane_id'] == self.panes[1] for r in info.get('receivers', []))
        self.record('receiver-display-after-reload', {'a': self.text(self.panes[0]), 'b': self.text(self.panes[1])})
        # Enabling and reads alone must not wake either model.
        starts_before = sum(e['kind'] == 'agent_start' for e in self.observed())
        self.send(self.panes[1], '/room-enable')
        def b_ready():
            current = self.api('room.get', {'workspace_id': self.workspace})
            return current if any(r['available'] and r['member']['pane_id'] == self.panes[1]
                                  for r in current.get('receivers', [])) else None
        info = wait(b_ready, 25, 'B explicitly enabled after hot reload')
        for _ in range(3):
            self.api('room.read', {'workspace_id': self.workspace})
            time.sleep(1)
        assert starts_before == sum(e['kind'] == 'agent_start' for e in self.observed())
        self.record('room-after-explicit-enable', info)
        member = next(m for m in info['members'] if m['pane_id'] == self.panes[1])
        recipient = {k: member[k] for k in ('pane_id', 'terminal_id', 'session')}
        post = self.api('room.post', {'workspace_id': self.workspace, 'recipient': recipient,
                                    'text': 'Reply with exactly ROOM-AFTER-HANDOFF using room_reply.'})
        self.record('room-post', post)
        def reply():
            # Evidence of actual persisted response, not just delivery status.
            messages = self.api('room.read', {'workspace_id': self.workspace})['messages']
            return [m for m in messages if m.get('reply_to') == post['sequence']]
        replies = wait(reply, 90, 'real room reply')
        assert len(replies) == 1 and replies[0]['text'].strip() == 'ROOM-AFTER-HANDOFF'
        assert replies[0]['author']['pane_id'] == self.panes[1]
        self.record('room-reply', replies)
        self.send(self.panes[1], '/room-disable')
        wait(lambda: 'room: disabled' in self.text(self.panes[1]), 10, 'B disabled')
        self.send(self.panes[1], '/room-enable')
        wait(b_ready, 15, 'B re-enabled after disable')
        self.send(self.panes[1], '/reload')
        wait(lambda: 'Reloaded' in self.text(self.panes[1]), 15, 'B reload after runtime opt-in')
        # Receiver TTL is 15 seconds: reload must not restore instance-local opt-in.
        time.sleep(17)
        info = self.api('room.get', {'workspace_id': self.workspace})
        assert not any(r['available'] and r['member']['pane_id'] == self.panes[1] for r in info.get('receivers', []))
        self.record('b-reload-optin-reset', {'inactive': True})
        final = self.sessions()
        assert [s['session_id'] for s in final] == [s['session_id'] for s in before]
        assert all(same_process(p) for p in self.before_pi)
        self.record('final-sessions', final)
        self.record('final-processes', owned_processes(self.env))
        self.record('PASS', {'survived': True, 'same_inflight_turn_finished': True, 'real_client_explicit_reattach_render_input': True,
                             'startup_optin_a_ready': True, 'b_hot_reload_explicit_enable_room_reply': True,
                             'enable_and_reads_no_inference': True, 'b_reload_resets_optin': True})

    def cleanup(self):
        # The old Popen exits on success. Never use its liveness to decide whether
        # the imported server needs stopping; authenticate current disposable peer.
        if self.client:
            self.client.close()
        owned = owned_processes(self.env)
        self.known.update({p['pid']: p for p in owned})
        stopped_peer = None
        try:
            stopped_peer = self.peer()
            self.api('server.stop')
        except (OSError, RuntimeError, ValueError):
            pass
        deadline = time.monotonic() + 10
        while any(same_process(p) for p in self.known.values()) and time.monotonic() < deadline:
            time.sleep(.1)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for p in self.known.values():
                if same_process(p):
                    os.kill(p['pid'], sig)
            time.sleep(.3)
        if self.old:
            self.old.wait(timeout=5)
        # We become subreaper before spawning, so imported server/orphan Pi can
        # be reaped here without depending on the old server's Popen handle.
        while True:
            try:
                pid, _ = os.waitpid(-1, os.WNOHANG)
                if not pid:
                    break
            except ChildProcessError:
                break
        survivors = [p for p in self.known.values() if same_process(p)]
        self.record('cleanup', {'stopped_peer': stopped_peer, 'survivors': survivors})
        assert not survivors
        if hasattr(self, 'log'):
            self.log.close()


def main():
    os.umask(0o077)
    # Linux PR_SET_CHILD_SUBREAPER; only affects this harness's descendants.
    if ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) != 0:
        raise RuntimeError('could not become child subreaper')
    evidence = ROOT / '.local/prd' / ('handoff-smoke-' + time.strftime('%Y%m%d-%H%M%S') + '.json')
    evidence.parent.mkdir(parents=True, exist_ok=True)
    print('Evidence:', evidence, flush=True)
    with tempfile.TemporaryDirectory(prefix='herdr-handoff-real-') as directory:
        trial = Trial(directory, evidence)
        try:
            trial.run()
        except Exception as error:
            trial.record('FAILED', {'type': type(error).__name__, 'message': str(error)})
            # Terminal contents have only our fixed test prompts. Never log env,
            # auth, model config, handoff token or provider network diagnostics.
            if hasattr(trial, 'panes'):
                for pane in trial.panes:
                    try:
                        trial.record('failure-pane-' + pane, trial.text(pane))
                    except Exception:
                        pass
            raise
        finally:
            trial.cleanup()
    print('Private credentials/config/sessions deleted; production untouched.', flush=True)


if __name__ == '__main__':
    main()
