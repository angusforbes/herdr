import type { ExtensionAPI, ExtensionContext, ExtensionCommandContext } from '@earendil-works/pi-coding-agent';
import { Type } from 'typebox';
import { BorderedLoader } from '@earendil-works/pi-coding-agent';
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { contactsFromMarkdown, liveContacts, parseArgs, resolveTargets, newRequest, envelope, replyEnvelope, senderLabel } from './core.mjs';
import { startCollector } from './collector.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const DIRECTORY = join(homedir(), 'Obsidian', 'Agent Message Board', '_contacts.md');
const STORE = join(homedir(), '.pi', 'agent', 'peer-messages');

export default function (pi: ExtensionAPI) {
  const collectors = new Set<any>();
  const operations = new Set<AbortController>();
  let alive = true;
  pi.on('session_start', () => { alive = true; });
  pi.on('session_shutdown', () => {
    alive = false;
    for (const operation of operations) operation.abort();
    for (const collector of collectors) collector.close('session-closed');
    collectors.clear(); operations.clear();
  });

  const directory = () => {
    try { return contactsFromMarkdown(readFileSync(DIRECTORY, 'utf8')); }
    catch (e: any) { if (e.code === 'ENOENT') return []; throw e; }
  };
  const list = async (signal?: AbortSignal) => {
    const result = await pi.exec('herdr', ['agent', 'list'], { timeout: 10000, signal });
    if (result.code !== 0) throw new Error(result.stderr || 'Could not list Herdr agents.');
    const data = JSON.parse(result.stdout);
    if (!Array.isArray(data.result?.agents)) throw new Error('Unexpected Herdr agent-list response.');
    return liveContacts(data.result.agents, directory());
  };
  const display = (content: string) => {
    if (alive) pi.sendMessage({ customType: 'peer-chat', content, display: true }, { triggerTurn: false });
  };
  function summary(request: any) {
    const lines = [`/${request.mode} ${request.id}: ${request.status}`];
    for (const peer of request.recipients) {
      const reply = request.replies[peer.session];
      lines.push(reply ? `✓ ${peer.name}: replied` : `○ ${peer.name}: ${request.dispatch[peer.session]?.detail ?? 'no response'}`);
    }
    lines.push(`Record: ${join(STORE, request.id, 'request.json')}`);
    return lines.join('\n');
  }

  type ToolInput = { recipients: string[]; message: string; timeout_seconds?: number };
  async function run(mode: 'talk' | 'demand', raw: string, ctx: ExtensionContext,
    toolInput?: ToolInput, signal?: AbortSignal, onProgress?: (text: string) => void) {
    const isTool = toolInput !== undefined;
    if ((!isTool && ctx.mode !== 'tui') || process.env.HERDR_ENV !== '1') {
      const error = 'Peer messaging needs Herdr; slash commands also need the interactive Pi TUI.';
      if (isTool) throw new Error(error);
      ctx.ui.notify(error, 'error'); return;
    }
    const operation = new AbortController(); operations.add(operation);
    let collector: any;
    const abort = () => { operation.abort(); collector?.close('cancelled'); };
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    try {
      if (operation.signal.aborted) throw new Error('Peer request cancelled before dispatch.');
      const args = isTool ? { names: toolInput.recipients, timeout: toolInput.timeout_seconds } : parseArgs(raw);
      const own = ctx.sessionManager.getSessionId();
      const initial = await list(operation.signal);
      const sender = initial.find((a: any) => a.session === own && a.pane === process.env.HERDR_PANE_ID)
        ?? initial.find((a: any) => a.session === own);
      if (!sender) throw new Error('Cannot locate this session in the live Herdr directory.');
      const recipients = resolveTargets(args.names, initial, own);
      if (!recipients.length) throw new Error('No other live agents found.');
      if (recipients.some((p: any) => p.kind !== 'pi')) throw new Error('This initial version supports Pi recipients only (reply session identification is Pi-specific).');
      const names = recipients.map((p: any) => p.name).join(', ');
      const message = isTool ? toolInput.message : await ctx.ui.editor(`/${mode} → ${names}\nWrite your message (cancel to send nothing)`, '');
      if (!message?.trim()) {
        if (isTool) throw new Error('Message must not be empty.');
        return;
      }
      if (operation.signal.aborted) throw new Error('Peer request cancelled before dispatch.');
      if (Buffer.byteLength(message) > 12000) throw new Error('Message is too large; limit is 12KB.');
      if (/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(message)) throw new Error('Message contains terminal control characters.');
      if (/\[(?:BEGIN|END) \|/.test(message)) throw new Error('Reserved framing markers in body; remove [BEGIN | / [END | markers.');
      if (!isTool && args.names[0].toLowerCase() === 'all' && !await ctx.ui.confirm('Send to all live peers?', `${recipients.length} recipients: ${names}`)) return;
      if (operation.signal.aborted) return;
      const request = newRequest(mode, sender, recipients, message, args.timeout ?? (mode === 'demand' ? 120 : 1800), isTool ? 'agent' : 'human');
      collector = await startCollector(request, STORE, {
        onReply: (r: any, peer: any) => {
          if (!alive) return;
          const content = replyEnvelope(r, peer.session);
          // A custom message keeps agent-originated replies distinct from human input.
          pi.sendMessage({ customType: 'peer-chat-reply', content, display: true },
            { triggerTurn: mode === 'talk', deliverAs: 'followUp' });
          const progress = `${peer.name} replied (${Object.keys(r.replies).length}/${r.recipients.length})`;
          onProgress?.(progress);
          if (ctx.hasUI) ctx.ui.notify(progress, 'info');
        },
        onClose: () => { if (collector) collectors.delete(collector); },
      });
      collectors.add(collector);
      if (operation.signal.aborted) collector.close('cancelled');
      display(`${isTool ? 'Tool ' : '/'}${mode} from ${senderLabel(request)} → ${names}\nRequest: ${request.id}\n\n${message}`);
      const dispatch = async () => {
        try {
          // Re-resolve by stable session AFTER composition; never trust a stale pane snapshot.
          const fresh = await list(operation.signal);
          await Promise.all(recipients.map(async (peer: any) => {
            if (operation.signal.aborted || request.status !== 'open') return;
            const matches = fresh.filter((a: any) => a.session === peer.session);
            if (matches.length !== 1 || matches[0].state === 'blocked' || matches[0].state === 'unknown') {
              collector.recordDispatch(peer.session, 'skipped', 'Unavailable, ambiguous, blocked, or unknown; no send.'); return;
            }
            peer.pane = matches[0].pane;
            collector.recordDispatch(peer.session, 'sending', 'Dispatch started.');
            try {
              const result = await pi.exec('herdr', ['agent', 'prompt', peer.pane, envelope(request, peer, collector.requestPath, join(HERE, 'reply.mjs'))], { timeout: 10000, signal: operation.signal });
              let accepted = false;
              try { accepted = JSON.parse(result.stdout).result?.type === 'agent_prompted'; } catch {}
              if (result.code !== 0 || !accepted) throw new Error(result.stderr || result.stdout || 'No dispatch receipt.');
              collector.recordDispatch(peer.session, 'accepted', 'Dispatch accepted; awaiting reply (not execution proof).');
            } catch (e: any) {
              collector.recordDispatch(peer.session, 'failed', `Delivery unconfirmed; no automatic retry: ${String(e.message).slice(0, 500)}`);
            }
          }));
        } catch (e: any) {
          for (const peer of recipients) if (!request.dispatch[peer.session]) collector.recordDispatch(peer.session, 'failed', String(e.message));
        } finally { collector.finishDispatch(); }
      };
      if (mode === 'talk') {
        await dispatch();
        display(`Dispatch results:\n${summary(request)}\nNo waiting for replies. Optional reply window: ${args.timeout ?? 1800}s; closes on reload/exit.`);
        // Do not hold the command/tool open for optional replies.
      } else if (isTool) {
        onProgress?.(`Waiting for ${recipients.length} peer responses; timeout ${args.timeout ?? 120}s. Cancellation stops this wait, not delivered remote work.`);
        await dispatch();
        await collector.finished;
      } else {
        await ctx.ui.custom<void>((tui, theme, _kb, done) => {
          const loader = new BorderedLoader(tui, theme, `Waiting for ${recipients.length} replies (up to ${args.timeout ?? 120}s). Esc cancels waiting, not remote work.`);
          let finished = false;
          const end = () => { if (!finished) { finished = true; done(); } };
          loader.onAbort = () => { operation.abort(); collector.close('cancelled'); end(); };
          collector.finished.then(end);
          void dispatch().catch(() => collector.close('error'));
          return loader;
        });
        display(summary(request));
      }
      return request;
    } catch (e: any) {
      if (collector) collector.close('error');
      if (isTool) throw e;
      if (alive) ctx.ui.notify(String(e.message ?? e), 'error');
    } finally {
      signal?.removeEventListener('abort', abort);
      operations.delete(operation);
    }
  }

  for (const mode of ['talk', 'demand'] as const) {
    pi.registerCommand(mode, {
      description: `${mode === 'talk' ? 'Message peers without waiting' : 'Request peer replies and wait (Esc cancels)'}: names | all [--timeout seconds]`,
      getArgumentCompletions: prefix => {
        const options = ['all', ...directory().map(c => c.name.includes(' ') ? `"${c.name}"` : c.name)];
        return options.filter(n => n.toLowerCase().startsWith(prefix.toLowerCase())).map(value => ({ value, label: value }));
      },
      handler: async (args, ctx) => { await run(mode, args, ctx); },
    });
    pi.registerTool({
      name: mode,
      label: mode === 'talk' ? 'Talk to peers' : 'Request peer responses',
      description: mode === 'talk'
        ? 'Send a bounded message as this agent to existing live Pi peers by name (or ["all"] only for an explicitly requested broadcast). Returns after dispatch, without waiting for replies. Replies arrive asynchronously as labeled follow-ups. No human composition dialog. Consult peers for relevant existing context; do not start unsolicited conversation loops.'
        : 'Send a request as this agent to existing live Pi peers by name and wait for request-specific answers/refusals, not pane-idle status. Use only when their responses block your next decision; otherwise use talk. Default timeout 120 seconds, cancellable. Do not create reciprocal waits; a peer answering a pending request must use its supplied reply helper. No human composition dialog. Return text capped at 32KB; full answers are saved in the request record.',
      parameters: Type.Object({
        recipients: Type.Array(Type.String({ minLength: 1 }), { minItems: 1, description: 'Exact agent names; ["all"] means every other live Pi peer. Multi-word names are single array elements.' }),
        message: Type.String({ minLength: 1, description: 'Your agent-authored message. Sender identity is derived automatically; do not claim to be Angus.' }),
        timeout_seconds: Type.Optional(Type.Integer({ minimum: 1, maximum: 3600, description: mode === 'talk' ? 'Optional reply window; default 1800s. Does not block.' : 'Maximum wait; default 120s.' })),
      }),
      async execute(_id, params, signal, onUpdate, ctx) {
        // Do not retain the tool's progress callback for later asynchronous talk replies.
        const request = await run(mode, '', ctx, params, signal,
          mode === 'demand' ? text => onUpdate?.({ content: [{ type: 'text', text }] }) : undefined);
        if (!request) throw new Error('No peer request created.');
        let text = summary(request);
        if (mode === 'demand') for (const peer of request.recipients) {
          if (request.replies[peer.session]) text += '\n\n' + replyEnvelope(request, peer.session);
        }
        if (Buffer.byteLength(text) > 32000) text = Buffer.from(text).subarray(0, 31000).toString('utf8') + `\n[Truncated; full replies: ${join(STORE, request.id, 'request.json')}]`;
        return { content: [{ type: 'text', text }], details: { requestId: request.id, status: request.status, record: join(STORE, request.id, 'request.json') } };
      },
    });
  }
}
