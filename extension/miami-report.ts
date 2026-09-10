import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

const SOCK =
  process.env.MIAMI_SOCK ??
  path.join(process.env.XDG_RUNTIME_DIR ?? os.tmpdir(), "miami.sock");

/** How often to re-announce. Also how fast a restarted miami repopulates. */
const HEARTBEAT_MS = 3000;

/**
 * Attempts at the surface handshake. More than one because pi's own title
 * writes race ours; three is enough in practice and costs ~450ms at worst,
 * off the critical path.
 */
const HANDSHAKE_TRIES = 3;

type State = "starting" | "idle" | "running";

/**
 * Which Ghostty surface (split) this pi runs in, so miami can focus it.
 *
 * There's no environment variable for it and no way to ask the terminal
 * directly, so it's discovered by handshake: set a unique window title, ask
 * Ghostty (over its AppleScript dictionary) which surface currently has that
 * title, then put the title back.
 *
 * The alternative — miami matching on cwd or title — is wrong exactly when it
 * matters: several agents in one repo all look identical, and focusing the
 * wrong pane is worse than not focusing at all.
 *
 * pi sets its own title during startup (interactive-mode's updateTerminalTitle,
 * called at the end of init and again on session_info_changed), which can land
 * between our write and our lookup and blank the nonce. So it retries: each
 * attempt rewrites the nonce, and whichever one isn't clobbered wins.
 *
 * The title is ours for ~150ms per attempt, a brief flicker in the tab bar.
 * Runs once per session, at session_start.
 */
async function discoverSurface(
  ctx: ExtensionContext,
  ui: { setTitle(t: string): void } | undefined,
): Promise<string | undefined> {
  // Ghostty's scripting dictionary is macOS-only, and absent before 1.3.
  if (process.platform !== "darwin") return undefined;
  if (process.env.TERM_PROGRAM !== "ghostty") return undefined;
  // Under a multiplexer the title belongs to it, not to a surface: the id we'd
  // find is the whole tmux pane, and focusing it wouldn't select this window.
  if (process.env.TMUX || process.env.STY) return undefined;
  if (!ui?.setTitle) return undefined;

  const nonce = `miami-surface-${process.pid}-${randomUUID()}`;
  try {
    for (let attempt = 0; attempt < HANDSHAKE_TRIES; attempt++) {
      ui.setTitle(nonce);
      // Give the escape sequence time to reach Ghostty and be parsed.
      await sleep(150);
      const id = await surfaceIdByTitle(nonce);
      if (id) return id;
    }
    return undefined;
  } catch {
    return undefined;
  } finally {
    // Always restore, including on throw: a pane left showing a nonce is far
    // worse than one that can't be focused.
    restoreTitle(ctx, ui);
  }
}

/**
 * Ask Ghostty for the id of the surface whose title is `title`.
 *
 * The title is passed as an argv parameter rather than interpolated into the
 * script, so no quoting or escaping of it is needed, and the comparison happens
 * inside AppleScript rather than shipping every title back to be matched here.
 */
function surfaceIdByTitle(title: string): Promise<string> {
  const script = [
    "on run argv",
    "  set want to item 1 of argv",
    '  tell application "Ghostty"',
    "    repeat with w in windows",
    "      repeat with t in tabs of w",
    "        repeat with s in terminals of t",
    "          if name of s is want then return id of s",
    "        end repeat",
    "      end repeat",
    "    end repeat",
    "  end tell",
    '  return ""',
    "end run",
  ].join("\n");
  return new Promise((resolve) => {
    execFile(
      "/usr/bin/osascript",
      ["-e", script, title],
      { timeout: 5000 },
      (err, stdout) => resolve(err ? "" : String(stdout).trim()),
    );
  });
}

/**
 * Put back the title pi sets for itself.
 *
 * Mirrors interactive-mode's updateTerminalTitle rather than trying to save and
 * restore the old one: the surface id isn't known until after the title has
 * already been overwritten, so there's nothing to have saved.
 */
function restoreTitle(ctx: ExtensionContext, ui: { setTitle(t: string): void }) {
  try {
    const base = path.basename(ctx.sessionManager?.getCwd?.() ?? process.cwd());
    const name = ctx.sessionManager?.getSessionName?.();
    // "π" is pi's APP_TITLE for an unbranded build. A rebranded one uses its
    // own name, so the title is briefly wrong there until pi rewrites it on the
    // next session_info_changed.
    ui.setTitle(name ? `π - ${name} - ${base}` : `π - ${base}`);
  } catch {
    // never let reporting break the session
  }
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

/** Hard cap on task text crossing the socket. */
const TASK_MAX = 160;
/** Hard cap on the activity line (tool name + one arg). */
const ACTIVITY_MAX = 120;

/**
 * Most informative single argument per builtin tool. Param names verified
 * against pi.getAllTools(): read/edit/write/ls use `path`, bash `command`,
 * grep/find `pattern`.
 */
function activityOf(toolName: string, args: any): string {
  const pick = (k: string) => (typeof args?.[k] === "string" ? args[k] : undefined);
  const detail =
    pick("command") ??
    pick("path") ??
    pick("pattern") ??
    pick("file_path") ??
    pick("query");
  if (!detail) return toolName;
  const flat = detail.replace(/\s+/g, " ").trim();
  const room = ACTIVITY_MAX - toolName.length - 2;
  return `${toolName}: ${flat.length > room ? flat.slice(0, room - 1) + "\u2026" : flat}`;
}

/**
 * Ticket id for `buck` agents (specialist pi agents driven with
 * `/buck ticket COIN-1234`).
 *
 * buck persists its workflow state as `custom` session entries, so the id is a
 * structured field rather than something to regex out of prose:
 *
 *   buck-ticket-workflow  -> data.ticketId
 *   buck-managed-agent    -> data.agent.ticketId
 *
 * Read the branch backwards and take the newest: one buck pane works tickets
 * sequentially, so the latest entry is the current ticket. The user turn is no
 * use here — it's usually "implement" / "go" / "yes".
 */
function ticketOf(ctx: ExtensionContext): string | undefined {
  try {
    const branch: any[] = ctx.sessionManager?.getBranch?.() ?? [];
    for (let i = branch.length - 1; i >= 0; i--) {
      const e: any = branch[i];
      if (e?.type !== "custom" && e?.type !== "custom_message") continue;
      if (typeof e.customType !== "string" || !e.customType.startsWith("buck-")) continue;
      const d = e.data ?? e.details;
      const id = d?.ticketId ?? d?.agent?.ticketId;
      if (typeof id === "string" && id) return id;
    }
  } catch {
    // never let reporting break the session
  }
  return undefined;
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .filter((c: any) => c?.type === "text")
      .map((c: any) => c.text)
      .join(" ");
  }
  return "";
}

export default function (pi: ExtensionAPI) {
  let sock: net.Socket | null = null;
  let connecting = false;
  let state: State = "starting";
  let live: ExtensionContext | null = null;
  let timer: NodeJS.Timeout | null = null;
  // Cached so the 3s heartbeat doesn't walk the whole branch every beat.
  let task: string | undefined;
  // Current tool call, if any. Cleared when the agent settles.
  let activity: string | undefined;
  // Current buck ticket, if this is a buck agent.
  let ticket: string | undefined;
  // Ghostty surface id, resolved once by handshake. Undefined until it lands,
  // and permanently so outside Ghostty.
  let terminal: string | undefined;

  /**
   * Latest user turn: what the agent was last asked to do. Recomputed only on
   * session/turn boundaries, never on the heartbeat.
   */
  function refreshTask(ctx: ExtensionContext) {
    try {
      const branch: any[] = ctx.sessionManager?.getBranch?.() ?? [];
      for (let i = branch.length - 1; i >= 0; i--) {
        const e: any = branch[i];
        if (e?.type !== "message" || e.message?.role !== "user") continue;
        const text = textOf(e.message.content).replace(/\s+/g, " ").trim();
        if (!text) continue;
        task = text.length > TASK_MAX ? text.slice(0, TASK_MAX - 1) + "…" : text;
        return;
      }
      task = undefined;
    } catch {
      // never let reporting break the session
    }
  }

  // Fire-and-forget connect. miami may not be running; that is fine and must
  // never disturb the pi session.
  function connect() {
    if (sock || connecting) return;
    connecting = true;
    const s = net.createConnection(SOCK);
    s.on("connect", () => {
      connecting = false;
      sock = s;
      // Re-announce immediately: a fresh miami knows nothing about us.
      if (live) report(live);
    });
    s.on("error", () => {
      connecting = false;
      sock = null;
    });
    s.on("close", () => {
      sock = null;
    });
    s.unref(); // never hold pi open
  }

  function send(msg: Record<string, unknown>) {
    if (!sock) {
      // Don't queue: state is a snapshot, and the heartbeat will resend the
      // current one once connected. Stale queued lines are worse than none.
      connect();
      return;
    }
    sock.write(JSON.stringify({ pid: process.pid, ...msg }) + "\n");
  }

  function report(ctx: ExtensionContext, next?: State) {
    live = ctx;
    if (next) state = next;
    const usage = ctx.getContextUsage?.();
    send({
      type: "status",
      state,
      cwd: ctx.sessionManager?.getCwd?.(),
      sessionFile: ctx.sessionManager?.getSessionFile?.(),
      sessionId: ctx.sessionManager?.getSessionId?.(),
      name: ctx.sessionManager?.getSessionName?.(),
      // pi already has a human display name ("GPT-5.6 Sol (LEGO)"); prefer it
      // over parsing ids. Raw id sent too, for the detail pane.
      model: ctx.model?.name,
      modelId: ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : undefined,
      thinkingLevel: ctx.thinkingLevel,
      task,
      activity,
      ticket,
      terminal,
      contextTokens: usage?.tokens ?? null,
      contextPercent: usage?.percent ?? null,
      ts: Date.now(),
    });
  }

  /**
   * Without this, an idle agent that never fires another event stays invisible
   * to a restarted miami: reconnects only happened on send(), and nothing sends
   * while idle. The heartbeat both reconnects and re-registers.
   */
  function startHeartbeat() {
    if (timer) return;
    timer = setInterval(() => {
      if (!live) return;
      if (sock) report(live);
      else connect();
    }, HEARTBEAT_MS);
    timer.unref?.(); // never hold pi open
  }

  pi.on("session_start", async (_e, ctx) => {
    live = ctx;
    connect();
    refreshTask(ctx);
    ticket = ticketOf(ctx);
    report(ctx, "idle");
    startHeartbeat();
    // Not awaited: the handshake takes ~300ms and must not delay the session.
    // The heartbeat carries the id to miami as soon as it resolves.
    void discoverSurface(ctx, ctx.ui).then((id) => {
      terminal = id;
      if (id && live) report(live);
    });
  });

  pi.on("session_info_changed", async (_e, ctx) => report(ctx));

  pi.on("agent_start", async (_e, ctx) => {
    refreshTask(ctx);
    ticket = ticketOf(ctx);
    report(ctx, "running");
  });

  // Live activity. tool_execution_start fires in assistant source order; with
  // parallel tools the last start wins, which is the best single-line answer.
  pi.on("tool_execution_start", async (event: any, ctx) => {
    activity = activityOf(event?.toolName ?? "tool", event?.args);
    report(ctx);
  });

  pi.on("tool_execution_end", async (_e, ctx) => {
    activity = undefined;
    report(ctx);
  });

  // agent_end is NOT terminal: retry / compaction / queued follow-ups may
  // still continue. agent_settled is the authoritative idle signal.
  pi.on("agent_settled", async (_e, ctx) => {
    activity = undefined;
    report(ctx, "idle");
  });

  // buck advances its workflow mid-turn, so the ticket is re-read here too:
  // a pane can move to the next ticket without a new agent_start.
  pi.on("turn_end", async (_e, ctx) => {
    refreshTask(ctx);
    ticket = ticketOf(ctx);
    report(ctx);
  });

  pi.on("session_shutdown", async () => {
    if (timer) clearInterval(timer);
    timer = null;
    send({ type: "gone", ts: Date.now() });
    sock?.end();
  });
}
