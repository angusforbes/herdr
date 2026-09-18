import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { mkdtempSync, writeFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const source = await readFile(new URL("./herdr-fork-prefill.ts", import.meta.url), "utf8");
const { default: install } = await import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);

function setup(t, { directory, ticket = true, token = "launch", mode = "tui", hasUI = true, id = "fork", markerId = "fork", markerToken = "launch", position = "rewrite", text = "draft", existing = "", progressed = false, marker = true } = {}) {
  if (!directory) {
    directory = mkdtempSync(join(tmpdir(), "herdr-prefill-test-"));
    t.after(() => rmSync(directory, { recursive: true, force: true }));
  }
  if (ticket && !existsSync(join(directory, "herdr-prefill-ticket.json"))) {
    writeFileSync(join(directory, "herdr-prefill-ticket.json"), JSON.stringify({ sessionId: id, launchToken: token }), { mode: 0o600, flag: "wx" });
  }
  let handler;
  const writes = [];
  install({ on(name, callback) { assert.equal(name, "session_start"); handler = callback; } });
  const entries = marker ? [{ type: "custom", customType: "herdr.fork", data: {
    schemaVersion: 1, sessionId: markerId, launchToken: markerToken, position, draftText: text,
  } }] : [];
  if (progressed) entries.push({ type: "message", message: { role: "user", content: "new work" } });
  const ctx = {
    mode, hasUI,
    sessionManager: {
      getHeader: () => ({ id }),
      getSessionFile: () => join(directory, "fork.jsonl"),
      getBranch: () => entries,
    },
    ui: { getEditorText: () => existing, setEditorText: (value) => writes.push(value) },
  };
  return { directory, fire: (reason = "startup") => handler({ reason }, ctx), writes };
}

test("only matching initial TUI startup prefills once, without other APIs", (t) => {
  const fixture = setup(t);
  fixture.fire(); fixture.fire(); fixture.fire("reload");
  assert.deepEqual(fixture.writes, ["draft"]);
  assert.ok(existsSync(join(fixture.directory, "herdr-prefill-consumed")));
});

test("wrong session/token, absent ticket, modes and continuation do nothing", (t) => {
  for (const options of [{ id: "descendant" }, { markerToken: "different" }, { ticket: false },
    { mode: "rpc" }, { mode: "print", hasUI: false }, { hasUI: false },
    { position: "continue" }, { existing: "already typed" }, { text: null }, { marker: false }]) {
    const fixture = setup(t, options);
    fixture.fire();
    assert.deepEqual(fixture.writes, [], JSON.stringify(options));
    assert.ok(!existsSync(join(fixture.directory, "herdr-prefill-consumed")));
  }
});

test("reload, resume and fork events do not restore draft", (t) => {
  for (const reason of ["reload", "resume", "fork", "new"]) {
    const fixture = setup(t); fixture.fire(reason);
    assert.deepEqual(fixture.writes, []);
  }
});

test("new process cannot reuse consumed ticket", (t) => {
  const first = setup(t); first.fire();
  const restarted = setup(t, { directory: first.directory }); restarted.fire();
  assert.deepEqual(first.writes, ["draft"]);
  assert.deepEqual(restarted.writes, []);
});

test("unconsumed ticket cannot resurrect draft after conversation progressed", (t) => {
  const fixture = setup(t, { progressed: true }); fixture.fire();
  assert.deepEqual(fixture.writes, []);
});

test("newline and slash text stay inert", (t) => {
  const fixture = setup(t, { text: "/not-a-command\nline two" });
  fixture.fire();
  assert.deepEqual(fixture.writes, ["/not-a-command\nline two"]);
});
