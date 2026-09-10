# miami

Dashboard for `pi` agents running in Ghostty tabs/panes.

Ghostty owns the panes (`cmd+d`, `cmd+w`, `cmd+[`/`]`). miami tells you what every `pi` is
doing, and will jump you to one (`enter`) or start a new one (`n`). No wrapper, no PTY, no
Cmd rebinds — you launch `pi` exactly as you do today.

```
~/.pi/agent/extensions/miami-report.ts   ← global, auto-loads in every pi
        │ unix socket, NDJSON
        ▼
miami                                     ← ratatui dashboard (server + TUI)
        │ AppleScript (Ghostty.sdef)
        ▼
Ghostty                                   ← focus a surface / open a tab
```

## Install

```bash
make install
```

Installs `miami` and `miami-tail` to `~/.cargo/bin` and copies the reporting extension into
`~/.pi/agent/extensions/`. Then run `miami` in one pane and `pi` in the others.

Editing the extension needs a re-install and a restart of any running `pi` panes:

```bash
make ext        # re-copy the extension only
make bin        # re-install the binaries only
make uninstall
```

Socket defaults to `$XDG_RUNTIME_DIR/miami.sock`, else `$TMPDIR/miami.sock`.
Override with `MIAMI_SOCK` (must match for both miami and pi).

## Keys

| key | action |
|---|---|
| `↑` / `↓` | move |
| `home` / `end` | first / last |
| `enter` | focus the selected agent's pane |
| `n` | new tab running `pi`, in miami's cwd |
| `d` | hide dead agents |
| `q` / `esc` | quit (agents keep running) |

`enter` and `n` need Ghostty 1.3+ on macOS and are only offered there; see
[Focus and spawn](#focus-and-spawn).

## Focus and spawn

Ghostty 1.3 ships an AppleScript dictionary (`Ghostty.sdef`), so both are plain
Apple events — no `System Events` keystroke faking, no accessibility permission,
no Cmd rebinds:

```
focus terminal id "<uuid>"                    ← raises window, selects tab, focuses split
new tab in front window with configuration …  ← the `in` parameter is required
```

`enter` needs to know *which* surface an agent occupies. Ghostty exports no
environment variable for it, so the extension discovers its own by handshake at
`session_start`: set a unique window title, ask Ghostty which surface currently
has that title, then restore the title. The uuid is reported as `terminal` and
shown on the detail pane's `pid` row.

The handshake retries (3×150ms) because pi sets its own title during startup and
that write races the nonce. It shows as a brief flicker in the tab bar, once per
session.

miami never guesses a pane from cwd or title: several agents in one repo look
identical, and raising the wrong one is worse than raising none. An agent with no
reported surface says so in the footer instead.

`n` types `pi` into a new tab's login shell rather than using the surface
configuration's `command`, which skips the shell and so inherits *Ghostty's*
environment — launched from the Dock that's the bare launchd `PATH`, without the
node version manager shim `pi` needs. Typing it gets the same `PATH` you'd get by
opening a tab yourself, and leaves you a prompt when `pi` exits. Set
`MIAMI_NEW_TAB_CMD` to launch something else (`pi --model …`, a wrapper).

## Status

Reported by the extension, not guessed:

| pi event | shown as |
|---|---|
| `session_start` | registers `{pid, cwd, sessionFile, sessionId, name, model}`, then the surface handshake adds `terminal` |
| `agent_start` | `running` |
| `agent_settled` | `idle` |
| `turn_end` | refreshes context tokens / % |
| `tool_execution_start` | live activity, e.g. `bash: cargo test` |
| `tool_execution_end` | clears activity |
| `session_shutdown` | `gone` |

`agent_settled` rather than `agent_end`, because `agent_end` can be followed by automatic
retry, compaction, or queued follow-ups. Panes killed without a clean shutdown are detected
by polling `kill(pid, 0)` every 2s.

The extension also re-announces every 3s (heartbeat). This is what lets you restart miami and
have existing agents reappear — events alone would only bring back agents that happen to do
something next, leaving idle ones invisible.

## Ticket ids

Jira-style keys (`COIN-1234`) get their own column plus a detail row. Aimed at `buck` agents
driven with `/buck ticket COIN-1234`, where every pane is labelled `buck` and the ticket is the
only thing telling them apart. The column only takes up width when at least one agent has a
ticket.

The id comes from buck's own `custom` session entries, which is where it actually lives:

| customType | field |
|---|---|
| `buck-ticket-workflow` | `data.ticketId` |
| `buck-managed-agent` | `data.agent.ticketId` |

The extension walks the branch backwards and takes the newest — a buck pane works tickets
sequentially, so the latest entry is the current one. Re-read on `agent_start` and `turn_end`,
since buck advances its workflow mid-turn.

**Not** scraped from the user turn: by the time a buck pane is working, the last thing the user
typed is `implement` / `go` / `yes`, and the cwd is the plain repo. There's no ticket in either.

miami falls back to text sniffing when no id is reported (older extension, or a non-buck agent):
session name, task, then cwd — including buck worktrees like
`.worktrees/buck-coin-2695-pin-actions`. That sniffing is case-sensitive on purpose; accepting
lowercase turns `gpt-5.6-sol-2026`, `lego-claude/opus-5` and `utf-8` into "tickets".

The id is sticky: reports that omit it don't clear it, so it survives turns that don't mention
the ticket. Changing to a new ticket replaces it.

Each row is up to three lines:

```
 ● auth refactor      running  Opus 5 (LEGO)      7,250 tok (4%)
     why does restarting the dashboard lose agents?      ← task (sticky)
     ↳ edit: src/ui.rs                                   ← only while a tool runs
```

The **task** is the latest user turn — what the agent was last asked to do — and is always
shown. The **activity** line appears underneath only while a tool is executing.

Task text is capped at 160 chars and the activity line at 120, computed only on session/turn
boundaries rather than per heartbeat.

Agents are labelled by session name (`/name`), falling back to the `cwd` basename, then pid.

Model names come from pi's own display name (`Opus 5 (LEGO)`, `GPT-5.6 Sol (LEGO)`) rather than
being parsed from ids. The raw `provider/id` is in the detail pane. If a report arrives without a
display name, the id is tidied as a fallback (`lego-openai/gpt-5.6-sol-2026-07-09` → `gpt-5.6-sol`).

## Limits

- **Focus needs the extension, and Ghostty 1.3+** — the surface uuid comes from the
  handshake above, so an agent started under an older extension, under tmux, or in another
  terminal can't be focused. Everything else about it still reports normally.
- **Cannot attach to an already-running `pi`** — `--mode rpc` is a separate mode speaking
  JSONL over stdin/stdout; an interactive `pi` has its stdio bound to the TTY. Hence the
  extension.
- Read-only: no sending prompts, no kill/restart. Nothing is written to your sessions.
- `contextTokens` is `null` right after compaction until the next assistant response.

## Test

```bash
cargo test    # wire protocol, partial updates, reaping, activity clearing, rendering
```

To exercise the dashboard without waiting on real agents, run `miami` in one pane and:

```bash
make fake                          # or: ./scripts/fake-agents.py -n 5
```

Each fake owns a real `sleep` child, so its pid genuinely exists and the reaper behaves as it
would for real agents. Ctrl+C sends `gone` for each.

There is also a headless viewer, handy in a pipe or without a TTY:

```bash
miami-tail          # live table on stdout
miami-tail --raw    # append-only, no screen clearing
```

`miami` and `miami-tail` both bind the socket, so run only one at a time (or set `MIAMI_SOCK`).
