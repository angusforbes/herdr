# This fork's Pi setup (opt-in, development)

The room UI, Pi room receiver, session-name bridge and naming helper are a
single operational setup. The ordinary `herdr integration install pi` command
currently installs only the built-in state reporter. It does **not** install
this whole profile.

This directory packages the additional naming files previously kept only on
the development machine, and provides a conservative installer for all seven
required files. No personal configuration, credentials, transcripts, global
agent instructions, systemd services or caches are included.

## Contents

| Installed path | Source / purpose |
| --- | --- |
| `PI_DIR/extensions/herdr-agent-state.ts` | Bundled Pi v9 session/state reporter; required for room identity |
| `PI_DIR/extensions/herdr-room/index.ts` | Room tools and Pi lifecycle integration |
| `PI_DIR/extensions/herdr-room/receiver.mjs` | Room membership and delivery |
| `PI_DIR/extensions/herdr-room/room-context.mjs` | Room context handling |
| `PI_DIR/extensions/herdr-room/transport.mjs` | Socket transport |
| `PI_DIR/extensions/name-sync.ts` | `/name` to Herdr identity bridge and `rename_self` tool |
| `BIN_DIR/herdr-name` | Pane metadata and single-pane tab naming helper |

## Prerequisites and boundaries

- A compatible build of **this fork**, including the room API and Awaiting
  state. Do not install the v9 reporter against the older live v8-only build.
- Pi 0.85.1 (the tested version), Bash, Python 3 and the standard Unix tools
  used by `herdr-name`.
- `herdr` on PATH must be the intended compatible binary. Its socket must be
  the intended server, not an inherited production socket in a sandbox.
- The helper directory must also be on PATH for Pi to invoke `herdr-name`.
- Set `PI_CODING_AGENT_DIR` to the destination Pi directory before launching
  Pi. A fresh directory has **no authentication or personal settings**. The
  installer neither copies those nor logs in to a provider.
- Launch Pi inside the target Herdr pane, with its exact `HERDR_SOCKET_PATH`
  and `HERDR_PANE_ID`. Missing identity/routing leaves room membership inactive.
- Do not use `--no-extensions` unless explicitly loading every required
  extension. Loading only the state reporter does not enable rooms or naming.
- Sidebar layout/preferences remain ordinary Herdr configuration. This setup
  does not overwrite them, and fresh sandbox defaults may look different.

## Safe installation workflow

From this directory, create a temporary destination:

`P=$(mktemp -d)`

Preview the complete file plan (this writes nothing):

`python3 setup.py --pi-dir "$P/pi" --bin-dir "$P/bin"`

Apply that same plan only after inspecting it:

`python3 setup.py --pi-dir "$P/pi" --bin-dir "$P/bin" --apply`

The helper deliberately requires both destination arguments. There is no
implicit global installation. Identical files are accepted without rewriting;
any differing existing file, symlink destination, or invalid parent is refused
before writes start. There is no force/overwrite flag. Back up and reconcile an
existing installation deliberately rather than deleting it to make this pass.
Exclusive file creation also refuses files that appear after preflight. An I/O
failure can leave some **new** files installed; inspect before retrying.

Installing files does not launch/reload agents or change a running server.
Before updating a real installation, back up its files and obtain approval.
Only reload the specific Pi sessions whose owners approved the update.

## Optional Pi Twin package

The native tab menu supports **Agent Split** and **Agent Merge** for eligible Pi
panes. This requires the separate [pi-twin package](https://github.com/angusforbes/pi-twin)
and its `pi-twin` CLI on the server's PATH; the seven-file setup above does not
install that package. The integration was checked against package commit
`c011fa37b63b6aaee309d10e4f4d650fd2ebb869` with Pi 0.85.1.

Load the package in the relevant Pi sessions at an approved idle boundary. Its
capability/session metadata controls menu availability; stale targets are
rejected. `/twin-split` and `/twin-merge` remain available through Pi independently
of the native menu. Errors and timeouts do not authorize automatic retries.

Twin names such as `Parent[a]` require helpers that preserve the closing bracket.
The bundled `herdr-name` does so; an independently installed name-keeper must
also be updated deliberately. Existing differing helper files are still refused
by setup rather than silently overwritten.

Twins share files: splitting does not create a Git worktree, rewind disk state,
or isolate edits. Reviewed merge-back imports conversation context, not files.
Do not apply the package's standalone Herdr menu patch again on a build already
containing this integration. Installing or reloading Pi Twin is not permission
to restart the shared Herdr server.

## Validation status

`python3 test_setup.py`

These tests cover file completeness, byte identity, idempotence, no-write plans,
conflict preflight, preservation of unrelated settings and symlink refusal.
They do not prove end-to-end room delivery or naming.

Basic complete-profile sandbox validation passed with Pi 0.85.1 and runtime
commit `4e518da`:

- The state reporter, room receiver and naming extension loaded without
  reported errors; the room API confirmed the exact session binding and an
  available receiver.
- `herdr-name ProfileTest` renamed the tab. After `/reload`, the room showed
  its automatic named introduction.
- A fresh session received an authorized human room question and successfully
  called `room_reply`, producing the requested attributed response in the room.
- `/name ProfileCheck` renamed the tab and produced a room introduction/rename
  notice, as confirmed by the human tester.

These checks do not establish every sidebar styling variant, child-session
isolation, fork behavior, or server restart/handoff recovery. Those remain
separate regression/deployment checks. No live installation was replaced.

Important test-harness lessons:

- Loading only the state reporter does not test rooms or naming.
- `--no-tools` disables extension tools too. A restricted test may allow
  `room_read,room_post,room_reply,rename_self` with `--tools` instead.
- Pi's `--offline` disables startup network operations, **not model requests**.
  The sandbox inherited an existing NVIDIA API key from its environment; no
  credential file was copied or committed. A real room reply uses a model and
  requires explicit authorization and appropriate provider access.
- The initial tool-disabled session printed imitation tool-call JSON. After
  enabling tools, that history still produced imitation JSON. A fresh `/new`
  session successfully called the tool. Printed JSON is not delivery evidence.

Name sync currently treats `parentSession` (including forks) as a child guard;
that known limitation remains. The name helper's optional persistent republish
service is **not** installed here. Its saved names live under
`XDG_STATE_HOME/herdr-names`, falling back to `~/.local/state/herdr-names`.
Use a private `XDG_STATE_HOME` in tests. This packaging change does not perform
or claim a successful live deployment; only the basic sandbox checks above
have been verified.

Room behavior and routing details: [rooms prototype](../../docs/next/rooms-prototype.md).
