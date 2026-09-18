#!/usr/bin/env python3
"""Opt-in isolated live smoke: server API -> new Pi pane -> unsent rewrite draft.

No model request is submitted. Uses a disposable config/runtime/Pi directory,
never the user's running server or native sessions. Pass --herdr <candidate>.
"""
import argparse
import hashlib
import fcntl
import pty
import select
import struct
import termios
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--herdr', required=True)
    parser.add_argument('--slow-shell', action='store_true', help='exercise a bash startup with a foreground sleep')
    args = parser.parse_args()
    binary = str(Path(args.herdr).resolve())
    with tempfile.TemporaryDirectory(prefix='herdr-branch-smoke-') as root:
        root = Path(root)
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        config = root / 'config'
        config.mkdir()
        config_file = config / 'config.toml'
        config_file.write_text('onboarding = false\n' + ('[terminal]\ndefault_shell = "/bin/bash"\n' if args.slow_shell else ''))
        home = root / 'home'
        home.mkdir()
        if args.slow_shell:
            (home / '.bashrc').write_text('echo HERDR_SHELL_STARTING\nsleep 2\necho HERDR_SHELL_READY\n')
        sock = root / 'api.sock'
        env = {k: v for k, v in os.environ.items() if not k.startswith('HERDR_') and k not in ('PI_SESSION_ID', 'PI_SESSION_FILE')}
        env.update(XDG_CONFIG_HOME=str(config), XDG_STATE_HOME=str(root / 'state'), XDG_RUNTIME_DIR=str(runtime), HERDR_CONFIG_PATH=str(config_file), HERDR_SOCKET_PATH=str(sock), SHELL='/bin/sh', PI_CODING_AGENT_DIR=str(root / 'pi'), PI_OFFLINE='1', PI_TELEMETRY='0', HERDR_DISABLE_SOUND='1')
        if args.slow_shell:
            env['HOME'] = str(home)
        log = (root / 'server.log').open('w+')
        server = subprocess.Popen([binary, 'server'], env=env, cwd=root, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)

        def request(method, params):
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(15)
                client.connect(str(sock))
                client.sendall((json.dumps({'id': method, 'method': method, 'params': params}) + '\n').encode())
                response = json.loads(client.makefile().readline())
                if 'error' in response:
                    raise RuntimeError(f'{method}: {response["error"]}')
                return response['result']

        def until(check, timeout=15):
            deadline = time.monotonic() + timeout
            last = None
            while time.monotonic() < deadline:
                try:
                    result = check()
                    if result:
                        return result
                except (OSError, RuntimeError) as error:
                    last = error
                time.sleep(.1)
            raise RuntimeError(f'timeout: {last}')

        try:
            until(lambda: sock.exists() and request('ping', {}))
            request('workspace.create', {'cwd': str(root), 'label': 'branch-fixture', 'focus': True})
            panes = request('pane.list', {})['panes']
            source_pane = panes[-1]['pane_id']
            source = root / 'source.jsonl'
            source.write_text('\n'.join(json.dumps(e) for e in [
                {'type': 'session', 'version': 3, 'id': '01999999-1111-4111-8111-111111111111', 'timestamp': '2026-09-14T20:00:00Z', 'cwd': str(root)},
                {'type': 'message', 'id': 'user0001', 'parentId': None, 'timestamp': '2026-09-14T20:00:01Z', 'message': {'role': 'user', 'content': 'HERDR_BRANCH_DRAFT_UNSENT', 'timestamp': 1789416001000}},
            ]) + '\n')
            if args.slow_shell:
                until(lambda: 'HERDR_SHELL_READY' in request('pane.read', {'pane_id': source_pane, 'source': 'recent_unwrapped', 'format': 'text'})['read']['text'])
            request('agent.start', {'name': 'fixture-pi', 'kind': 'pi', 'pane_id': source_pane, 'args': ['--offline', '--no-extensions', '--no-skills', '--no-context-files', '--no-prompt-templates', '--no-tools', '--session', str(source)]})
            until(lambda: 'HERDR_BRANCH_DRAFT_UNSENT' in request('agent.read', {'target': source_pane, 'source': 'recent_unwrapped', 'format': 'text'})['read']['text'])
            request('pane.report_agent_session', {'pane_id': source_pane, 'source': 'herdr:pi', 'agent': 'pi', 'seq': time.time_ns(), 'agent_session_path': str(source), 'session_start_source': 'startup'})
            before = hashlib.sha256(source.read_bytes()).hexdigest()
            read = request('agent.conversation', {'target': source_pane, 'query': 'HERDR_BRANCH_DRAFT_UNSENT'})
            reference = read['messages'][0]['reference']
            fork = request('agent.fork', {'target': source_pane, 'reference': reference, 'position': 'rewrite'})
            target = fork['pane']['pane_id']
            assert target != source_pane
            fork_path = Path(fork['session_path'])
            assert fork_path != source
            until(lambda: 'HERDR_BRANCH_DRAFT_UNSENT' in request('agent.read', {'target': target, 'source': 'recent_unwrapped', 'format': 'text'})['read']['text'], 25)
            records = [json.loads(line) for line in fork_path.read_text().splitlines()]
            assert not any(r.get('type') == 'message' for r in records), 'draft was submitted'
            assert hashlib.sha256(source.read_bytes()).hexdigest() == before, 'source modified'
            continued = request('agent.fork', {'target': source_pane, 'reference': reference, 'position': 'continue'})
            continue_path = Path(continued['session_path'])
            assert continue_path not in (source, fork_path)
            until(lambda: 'HERDR_BRANCH_DRAFT_UNSENT' in request('agent.read', {'target': continued['pane']['pane_id'], 'source': 'recent_unwrapped', 'format': 'text'})['read']['text'], 25)
            continue_records = [json.loads(line) for line in continue_path.read_text().splitlines()]
            messages = [r for r in continue_records if r.get('type') == 'message']
            assert len(messages) == 1 and messages[0]['id'] == 'user0001'
            count = len(request('pane.list', {})['panes'])
            try:
                request('agent.fork', {'target': source_pane, 'reference': dict(reference, ancestry_hash='stale'), 'position': 'rewrite'})
                raise AssertionError('stale fork accepted')
            except RuntimeError as error:
                assert 'conversation_stale' in str(error)
            assert len(request('pane.list', {})['panes']) == count
            assert hashlib.sha256(source.read_bytes()).hexdigest() == before
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
            client = subprocess.Popen([binary, 'client'], env=dict(env, TERM='xterm-256color'), cwd=root, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            captured = bytearray()
            def read_client_for(seconds):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline and client.poll() is None:
                    ready, _, _ = select.select([master], [], [], .1)
                    if ready:
                        try:
                            captured.extend(os.read(master, 65536))
                        except OSError:
                            break
            try:
                read_client_for(2)
                os.write(master, b'\x13')  # Ctrl+S: Herdr search, not source Pi input
                read_client_for(.3)
                os.write(master, b'HERDR_BRANCH_DRAFT_UNSENT\r')
                read_client_for(1)
                os.write(master, b'\x1b[B\r')  # Select first result and preview
                read_client_for(2)
                assert b'Pi tree' in captured, 'attached client did not render message preview'
            finally:
                client.terminate()
                client.wait(timeout=5)
                os.close(master)
            assert hashlib.sha256(source.read_bytes()).hexdigest() == before
            print('PASS: rewrite draft unsent; continue preserves selected message; stale request rejected; source byte-identical; attached TUI renders Pi tree')
        except Exception:
            log.flush()
            log.seek(0)
            print(log.read()[-6000:])
            raise
        finally:
            try:
                request('server.stop', {})
            except Exception:
                pass
            try:
                server.wait(timeout=8)
            except subprocess.TimeoutExpired:
                os.killpg(server.pid, signal.SIGTERM)
                server.wait(timeout=5)
            log.close()


if __name__ == '__main__':
    main()
