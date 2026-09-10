#!/usr/bin/env python3
"""Emit fake agents at miami's socket so the dashboard can be exercised
without waiting on real pi turns.

    ./scripts/fake-agents.py           # 4 agents, churn forever
    ./scripts/fake-agents.py -n 8      # 8 agents
    MIAMI_SOCK=/tmp/x.sock ./scripts/fake-agents.py

Each fake agent owns a real `sleep` child process, so its pid genuinely exists
and miami's kill(pid,0) reaper behaves as it would for real agents. Ctrl+C
sends `gone` for each and kills the children.
"""
import argparse
import json
import os
import random
import signal
import socket
import subprocess
import sys
import tempfile
import time

SOCK = os.environ.get("MIAMI_SOCK") or os.path.join(
    os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir(), "miami.sock"
)

NAMES = [
    ("auth refactor", "/Users/you/dev/backend"),
    ("api ticket", "/Users/you/dev/api"),
    ("flaky test hunt", "/Users/you/dev/web"),
    (None, "/Users/you/dev/brickbank"),  # unnamed -> labels by cwd basename
    ("docs pass", "/Users/you/dev/docs"),
    ("perf spike", "/Users/you/dev/engine"),
    ("release prep", "/Users/you/dev/infra"),
    (None, "/Users/you/dev/scratch"),
]
MODELS = [
    ("Opus 5 (LEGO)", "lego-claude/eu.anthropic.claude-opus-5"),
    ("GPT-5.6 Sol (LEGO)", "lego-openai/gpt-5.6-sol-2026-07-09"),
    ("Claude Sonnet 5", "anthropic/claude-sonnet-5"),
    ("Haiku 4.5 (LEGO)", "lego-claude/eu.anthropic.claude-haiku-4-5-20251001-v1:0"),
]
TASKS = [
    "add in the tool call activity line",
    "figure out why the reconnect drops idle agents",
    "rip out the jump-to-tab sweep, it was unreliable",
    "make idle purple and running green",
    "parse the model id into a readable name",
    "why does restarting the dashboard lose agents?",
    "ship it",
]
ACTIVITIES = [
    ("bash", "cargo test --all"),
    ("bash", "git status --short"),
    ("read", "src/registry.rs"),
    ("edit", "src/ui.rs"),
    ("write", "docs/design.md"),
    ("grep", "tool_execution_start"),
    ("find", "*.jsonl"),
    ("ls", "/Users/you/dev/backend/src"),
]


def connect():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    return s


class Fake:
    def __init__(self, idx):
        name, cwd = NAMES[idx % len(NAMES)]
        model, model_id = MODELS[idx % len(MODELS)]
        # A real child process => a real pid for miami's reaper.
        self.proc = subprocess.Popen(
            ["sleep", "86400"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        self.pid = self.proc.pid
        self.name = name
        self.cwd = cwd
        self.model = model
        self.model_id = model_id
        self.task = random.choice(TASKS)
        self.state = "idle"
        self.activity = None
        self.tokens = random.randint(4_000, 30_000)
        self.next_flip = time.time() + random.uniform(1, 5)

    def payload(self):
        d = {
            "pid": self.pid,
            "type": "status",
            "state": self.state,
            "cwd": self.cwd,
            "sessionId": f"01a0{self.pid:04x}-fake",
            "model": self.model,
            "modelId": self.model_id,
            "thinkingLevel": "medium",
            "task": self.task,
            "contextTokens": self.tokens,
            "contextPercent": round(self.tokens / 200_000 * 100, 1),
            "ts": int(time.time() * 1000),
        }
        if self.name:
            d["name"] = self.name
        # Absent activity is how "no tool running" is signalled.
        if self.activity:
            d["activity"] = self.activity
        return d

    def tick(self, now):
        if now < self.next_flip:
            return False
        if self.state == "idle":
            self.state = "running"
            self.task = random.choice(TASKS)
            self.next_flip = now + random.uniform(2, 8)
        elif self.activity is None:
            tool, arg = random.choice(ACTIVITIES)
            self.activity = f"{tool}: {arg}"
            self.tokens += random.randint(500, 4_000)
            self.next_flip = now + random.uniform(0.8, 2.5)
        else:
            # finish the tool; sometimes settle, sometimes run another
            self.activity = None
            if random.random() < 0.4:
                self.state = "idle"
                self.next_flip = now + random.uniform(2, 9)
            else:
                self.next_flip = now + random.uniform(0.3, 1.2)
        return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", "--agents", type=int, default=4)
    args = ap.parse_args()

    try:
        sock = connect()
    except (FileNotFoundError, ConnectionRefusedError):
        print(f"no miami listening at {SOCK}", file=sys.stderr)
        print("start `miami` first (it binds the socket)", file=sys.stderr)
        sys.exit(1)

    fakes = [Fake(i) for i in range(args.agents)]
    print(f"{len(fakes)} fake agents -> {SOCK}   (ctrl+c to stop)")
    for f in fakes:
        print(f"  pid {f.pid}  {f.name or os.path.basename(f.cwd)}")

    def send(d):
        sock.sendall((json.dumps(d) + "\n").encode())

    def bye(*_):
        for f in fakes:
            try:
                send({"pid": f.pid, "type": "gone", "ts": int(time.time() * 1000)})
            except OSError:
                pass
            f.proc.kill()
        try:
            sock.close()
        except OSError:
            pass
        print("\nstopped")
        sys.exit(0)

    signal.signal(signal.SIGINT, bye)
    signal.signal(signal.SIGTERM, bye)

    for f in fakes:
        send(f.payload())

    last_beat = 0.0
    while True:
        now = time.time()
        changed = [f for f in fakes if f.tick(now)]
        for f in changed:
            send(f.payload())
        # heartbeat, mirroring the real extension's 3s re-announce
        if now - last_beat >= 3:
            for f in fakes:
                send(f.payload())
            last_beat = now
        time.sleep(0.1)


if __name__ == "__main__":
    main()
