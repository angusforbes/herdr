#!/usr/bin/env python3
"""Plan or install this fork's Pi integrations into explicit destinations.

No credentials, personal settings, services, or running sessions are modified.
Existing differing files and symlink destinations are refused, never overwritten.
"""
import argparse
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]


def bundle(pi_dir, bin_dir):
    extensions = Path(pi_dir).absolute() / 'extensions'
    files = [
        (REPO / 'src/integration/assets/pi/herdr-agent-state.ts', extensions / 'herdr-agent-state.ts'),
        (HERE / 'name-sync.ts', extensions / 'name-sync.ts'),
        (HERE / 'herdr-name', Path(bin_dir).absolute() / 'herdr-name'),
    ]
    for name in ('index.ts', 'receiver.mjs', 'room-context.mjs', 'transport.mjs'):
        files.append((REPO / 'integrations/room/pi' / name, extensions / 'herdr-room' / name))
    return files


def preflight(files):
    pending = []
    for source, target in files:
        if not source.is_file():
            raise ValueError(f'Missing bundled source: {source}')
        for part in (target, *target.parents):
            if part.is_symlink():
                raise ValueError(f'Refusing symlink destination: {part}')
        for parent in target.parents:
            if parent.exists() and not parent.is_dir():
                raise ValueError(f'Destination parent is not a directory: {parent}')
        if target.exists():
            if not target.is_file() or target.read_bytes() != source.read_bytes():
                raise ValueError(f'Refusing to overwrite differing file: {target}')
            if target.name == 'herdr-name' and not (target.stat().st_mode & 0o111):
                raise ValueError(f'Existing helper is not executable: {target}')
        else:
            pending.append((source, target))
    return pending


def install(pi_dir, bin_dir, apply=False):
    files = bundle(pi_dir, bin_dir)
    pending = preflight(files)  # Check the entire plan before creating anything.
    if apply:
        for source, target in pending:
            target.parent.mkdir(parents=True, exist_ok=True)
            # Exclusive creation also refuses a file appearing after preflight.
            with target.open('xb') as output:
                output.write(source.read_bytes())
            target.chmod(0o700 if target.name == 'herdr-name' else 0o600)
    return files, pending


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pi-dir', required=True, type=Path, help='Explicit Pi agent directory, not the extensions subdirectory')
    parser.add_argument('--bin-dir', required=True, type=Path, help='Explicit helper directory to put on PATH')
    parser.add_argument('--apply', action='store_true', help='Create missing files; otherwise only show the plan')
    args = parser.parse_args()
    try:
        files, pending = install(args.pi_dir, args.bin_dir, args.apply)
    except (OSError, ValueError) as error:
        parser.exit(1, f'{error}\nNo existing files were overwritten. Inspect the destination before retrying.\n')
    new = {target for _, target in pending}
    for _, target in files:
        label = ('created' if args.apply else 'would create') if target in new else 'identical'
        print(f'{label}: {target}')
    print('Requires a compatible build of this fork (Pi integration v9), Pi 0.85.1, Bash and Python 3.')
    print('Select this Pi directory with PI_CODING_AGENT_DIR; put the helper directory and intended Herdr binary on PATH.')
    print('No agent was reloaded. No credentials, personal settings or services were installed.')


if __name__ == '__main__':
    main()
