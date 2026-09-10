# miami — design

Dashboard for `pi` processes running in Ghostty tabs/panes.

## Shape
```
~/.pi/agent/extensions/miami-report.ts   ← global, auto-loads in every pi
        │ unix socket, NDJSON
        ▼
miami                                     ← single ratatui binary
```

- **Ghostty owns panes.** `cmd+d` / `cmd+w` / `cmd+[` `]` untouched. No Cmd rebinds, no
  kitty-keyboard dependency, no PTY wrapper, no `miami run`. You launch `pi` as you do today.
- **Extension reports, dashboard renders.** `miami-report.ts` pushes lifecycle events to
  `$XDG_RUNTIME_DIR/miami.sock`; `miami` is the server + TUI. No separate daemon.
- **Status is authoritative, not guessed:**
  `session_start`→register `{pid,cwd,sessionFile,sessionId,name,model}` ·
  `agent_start`→Running · `agent_settled`→Idle (not `agent_end`: retry/compaction may follow) ·
  `turn_end`→tokens/cost · `session_shutdown`→deregister.
- **Session-file tailing as enrichment.** `~/.pi/agent/sessions/--<cwd-slug>--/*.jsonl` is
  live-appended JSONL; used to backfill agents that predate the extension. No PID, status inferred.
- **Read-only v1.** List + detail (cwd, model, state, tokens/cost, last assistant line).
  Plain keys: `j/k` select, `enter` detail, `q` quit.

## Not possible / out
- **Cannot focus or spawn a Ghostty pane** — no IPC on macOS. Human navigates; miami reports.
- **Cannot attach to a running interactive `pi`** — `--mode rpc` is a mutually exclusive mode over
  stdin/stdout; an interactive `pi` has stdio bound to the TTY. Hence the extension.
- No session persistence, no worktree management in v1.

## Open
1. ~~Verify global extension loading~~ — verified true.
2. Send input to an agent from miami? Needs a writable channel (extension-side), not v1 read-only.
3. ~~Worktree isolation~~ — resolved: plain `cwd`, labels handle identity.
4. ~~Feature list~~ — dropped.

---

## Status: v1 built

- `src/registry.rs` — agent state keyed by pid; partial updates retain prior fields; `reap()`
  marks agents whose pid is gone (`kill(pid,0)`).
- `src/server.rs` — unix socket, NDJSON, one thread per connection; malformed lines skipped,
  never fatal; stale socket files reclaimed on bind.
- `src/ui.rs` — header / list / detail / hints.
- `extension/miami-report.ts` — verified against real `pi`: reports pid, cwd, sessionId, model,
  thinkingLevel, context; `gone` on shutdown. Socket is optional & unref'd, so miami being
  absent or dead never disturbs a pi session.
- `tests/wire.rs` — 2 tests, headless (no TTY needed).

Resolved: extensions auto-load globally (verified). Labels = session name → cwd basename → pid,
so plain `cwd` is fine and no worktree management is needed.

## Possible next
- Send prompts to an agent (needs a writable channel extension-side; breaks read-only).
- Session-file tailing to backfill agents predating the extension, + last assistant line.

## Reconnect / heartbeat

Restarting miami used to lose agents. Reconnection was only attempted inside `send()`, and an
idle agent sends nothing — so only agents that happened to fire an event next came back.

Fixed in the extension:
- 3s heartbeat (`setInterval`, `unref`'d) re-reports current status and reconnects if needed.
- On `connect`, immediately re-announce; a fresh miami has no state.
- Dropped the offline queue: status is a snapshot, so replaying stale lines is worse than
  waiting for the next beat.

Verified: heartbeat gaps `[3000, 3001, 3001]` ms; an untouched idle agent reappears in a
restarted dashboard within one beat.

## Rejected: jump-to-tab (built, then removed)

Attempted: extension stamps `miami:<pid>` into the terminal title; miami synthesizes cmd+1..8
via osascript, reads the AX window title after each, builds pid -> tab index, caches it.

Removed. Three sweeps of unchanged tabs gave three different maps, one of which reported a pid
in a tab it did not occupy — a confident jump to the wrong agent.

- Ghostty's AX tree exposes no tab elements (one opaque `AXGroup`); the window title is the only
  readable signal, and it lags tab switches unpredictably (0.35/0.5/0.8s settle all varied).
- Ghostty's AX window list is empty while backgrounded — must `activate` first.
- Fatal: pi owns the title. `interactive-mode.js:updateTerminalTitle()` sets
  `π - <session> - <cwd>` from 4 call sites, clobbering any extension `setTitle`. The pid stamp
  cannot survive, and the whole scheme depends on reading it back.

Worth keeping in mind: macOS digit key codes are not sequential — `[18,19,20,21,23,22,26,28]`
for 1..8. Using `17+i` sends cmd+= (font size) for 7.

Cheap alternative not yet built: show `tty` (`ps -o tty= -p <pid>`) in the detail pane as exact
pane identity, and jump manually with cmd+N.

## Task + activity

Two lines per agent, both reported by the extension:

- **task** — latest user turn from `sessionManager.getBranch()`, capped 160 chars. Recomputed on
  `session_start` / `agent_start` / `turn_end` only; walking ~300 entries every 3s heartbeat would
  be wasteful. Sticky: survives reports that omit it.
- **activity** — current tool call from `tool_execution_start`, formatted `<tool>: <arg>` where the
  arg is `command` / `path` / `pattern` (names verified via `pi.getAllTools()`). Cleared on
  `tool_execution_end` and `agent_settled`.

Activity is deliberately **not sticky** in `Registry::apply`: absent means "no tool running", so it
is assigned unconditionally while every other field is only overwritten when present. Covered by
`tests/wire.rs::activity_is_cleared_by_omission`.

Rejected: LLM-generated summaries (option 4). Verified workable —
`completeSimple(model, {messages}, {apiKey, headers})` with auth from
`ctx.modelRegistry.getProviderAuth()`, ~1.2s per call — but costs money per agent per turn for
marginal gain over the raw user turn.

## Row layout

Three lines per agent: header, task, activity. Task is sticky (always visible); the activity line
exists only while a tool runs, so rows grow and shrink. Earlier the activity *replaced* the task,
which lost sight of the goal whenever a tool was running.

`Registry::apply` clears `activity` on `gone`, and `reap()` clears it too — a dead agent showing
`↳ bash: cargo test` reads as though it were still working. Covered by
`tests/wire.rs::gone_clears_activity`.

## Testing without real agents

- `scripts/fake-agents.py` — emits fake agents at the socket, each owning a real `sleep` child so
  pids exist and the reaper behaves normally. Mirrors the extension's 3s heartbeat.
- `src/bin/miami-tail.rs` — headless miami; same server and registry, prints a table instead of
  drawing a TUI. Verifies the wire in environments with no terminal.
