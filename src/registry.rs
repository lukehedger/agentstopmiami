use serde::Deserialize;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// One NDJSON line from the pi extension.
#[derive(Debug, Deserialize)]
pub struct Report {
    pub pid: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub state: Option<String>,
    pub cwd: Option<String>,
    #[serde(rename = "sessionFile")]
    pub session_file: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: Option<String>,
    pub task: Option<String>,
    pub activity: Option<String>,
    pub ticket: Option<String>,
    /// Ghostty surface uuid, discovered by the extension's title handshake.
    pub terminal: Option<String>,
    #[serde(rename = "contextTokens")]
    pub context_tokens: Option<u64>,
    #[serde(rename = "contextPercent")]
    pub context_percent: Option<f64>,
    pub ts: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Starting,
    Idle,
    Running,
    Gone,
}

impl State {
    pub fn label(&self) -> &'static str {
        match self {
            State::Starting => "starting",
            State::Idle => "idle",
            State::Running => "running",
            State::Gone => "gone",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub pid: u32,
    pub state: State,
    pub cwd: Option<String>,
    pub session_file: Option<String>,
    pub session_id: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    pub task: Option<String>,
    pub activity: Option<String>,
    /// Jira-style ticket key (`COIN-1234`) sniffed from name / task / cwd.
    /// Sticky: `buck` agents mention it once, in the `/buck ticket COIN-1234`
    /// turn, and later turns must not blank it out.
    pub ticket: Option<String>,
    /// Ghostty surface (split) this agent's pane is, for `focus`.
    ///
    /// Sticky like the other identity fields: the handshake runs once, at
    /// session_start, so heartbeats carry it but must not be required to.
    /// `None` means "not discoverable" (non-Ghostty terminal, older extension,
    /// handshake failed) and focus is refused rather than guessed.
    pub terminal: Option<String>,
    pub context_tokens: Option<u64>,
    pub context_percent: Option<f64>,
    pub updated: u64,
}

impl Agent {
    /// Row label: session name, else basename of cwd, else pid.
    pub fn label(&self) -> String {
        if let Some(n) = self.name.as_ref().filter(|s| !s.is_empty()) {
            return n.clone();
        }
        if let Some(cwd) = &self.cwd {
            if let Some(base) = cwd.rsplit('/').next().filter(|s| !s.is_empty()) {
                return base.to_string();
            }
        }
        format!("pid {}", self.pid)
    }

    /// Readable model name. Uses the display name pi reports; falls back to
    /// tidying the raw id for reports predating `model`.
    pub fn model_label(&self) -> String {
        if let Some(m) = self.model.as_ref().filter(|s| !s.is_empty()) {
            return m.clone();
        }
        match self.model_id.as_deref() {
            Some(id) => prettify_model_id(id),
            None => "-".into(),
        }
    }
}

/// First Jira-style ticket key in `s`, e.g. `COIN-1234`.
///
/// Shape: 2+ uppercase alphanumerics starting with a letter, `-`, then digits.
/// Must sit on a word boundary so identifiers like `x_COIN-1` don't match.
/// Hand-rolled to keep the dependency list at zero regex.
///
/// Deliberately case-sensitive. Matching lowercase too turns `gpt-5.6-sol-2026`
/// into `GPT-5`, `lego-claude/opus-5` into `OPUS-5` and `utf-8` into `UTF-8`.
/// Lower-kebab buck worktrees are handled by [`find_worktree_ticket`] instead.
///
/// Only a fallback: `Report.ticket` from the extension is authoritative.
pub fn find_ticket(s: &str) -> Option<String> {
    let ch: Vec<char> = s.chars().collect();
    let boundary = |c: char| !(c.is_alphanumeric() || c == '-' || c == '_');
    let mut i = 0;
    while i < ch.len() {
        // must start at a word boundary on an uppercase letter
        if !ch[i].is_ascii_uppercase() || (i > 0 && !boundary(ch[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < ch.len() && (ch[j].is_ascii_uppercase() || ch[j].is_ascii_digit()) {
            j += 1;
        }
        let project_len = j - start;
        // PROJ-123: hyphen, then at least one digit
        if project_len >= 2 && j < ch.len() && ch[j] == '-' {
            let digits_start = j + 1;
            let mut k = digits_start;
            while k < ch.len() && ch[k].is_ascii_digit() {
                k += 1;
            }
            if k > digits_start && (k == ch.len() || boundary(ch[k])) {
                return Some(ch[start..k].iter().collect());
            }
        }
        // Skip the whole run: no shorter match can start inside it.
        i = j.max(start + 1);
    }
    None
}

/// Ticket key out of a buck worktree path, which is lower-kebab and prefixed:
/// `…/.worktrees/buck-coin-2695-pin-actions` -> `COIN-2695`.
///
/// Anchored on the literal `buck-` segment prefix rather than matching lowercase
/// generally, which would false-positive on any hyphenated path component.
pub fn find_worktree_ticket(path: &str) -> Option<String> {
    for seg in path.split('/') {
        let Some(rest) = seg.strip_prefix("buck-") else {
            continue;
        };
        let mut it = rest.splitn(3, '-');
        let Some(proj) = it.next() else { continue };
        let Some(num) = it.next() else { continue };
        if proj.len() >= 2
            && proj.chars().all(|c| c.is_ascii_lowercase())
            && !num.is_empty()
            && num.chars().all(|c| c.is_ascii_digit())
        {
            return Some(format!("{}-{num}", proj.to_ascii_uppercase()));
        }
    }
    None
}

/// `lego-openai/gpt-5.6-sol-2026-07-09` -> `gpt-5.6-sol`
fn prettify_model_id(id: &str) -> String {
    // drop provider prefix
    let mut tail = id.rsplit('/').next().unwrap_or(id);
    // drop vendor namespacing (eu.anthropic.claude-…), but not version dots
    // (gpt-5.6-sol), so only strip leading segments that contain no digits.
    while let Some((head, rest)) = tail.split_once('.') {
        if head.is_empty() || head.chars().any(|c| c.is_ascii_digit()) {
            break;
        }
        tail = rest;
    }
    // drop a trailing vN:N revision (…-v1:0)
    let tail = tail.split(':').next().unwrap_or(tail);
    // truncate at the first date-ish run of digits: -2026-07-09 or -20251001
    let parts: Vec<&str> = tail.split('-').collect();
    let mut end = parts.len();
    for (i, p) in parts.iter().enumerate() {
        if p.len() >= 4 && p.chars().all(|c| c.is_ascii_digit()) {
            end = i;
            break;
        }
    }
    let out = parts[..end].join("-");
    if out.is_empty() { tail.to_string() } else { out }
}

#[derive(Default)]
pub struct Registry {
    agents: HashMap<u32, Agent>,
}

impl Registry {
    pub fn apply(&mut self, r: Report) {
        let now = now_ms();
        if r.kind == "gone" {
            if let Some(a) = self.agents.get_mut(&r.pid) {
                a.state = State::Gone;
                // A dead agent isn't running a tool; leaving the last activity
                // on screen reads as though it still were.
                a.activity = None;
                a.updated = now;
            }
            return;
        }
        let state = match r.state.as_deref() {
            Some("running") => State::Running,
            Some("idle") => State::Idle,
            _ => State::Starting,
        };
        let e = self.agents.entry(r.pid).or_insert_with(|| Agent {
            pid: r.pid,
            state: State::Starting,
            cwd: None,
            session_file: None,
            session_id: None,
            name: None,
            model: None,
            model_id: None,
            thinking_level: None,
            task: None,
            activity: None,
            ticket: None,
            terminal: None,
            context_tokens: None,
            context_percent: None,
            updated: now,
        });
        e.state = state;
        e.updated = r.ts.unwrap_or(now);
        // Only overwrite when the report carries a value.
        if r.cwd.is_some() {
            e.cwd = r.cwd;
        }
        if r.session_file.is_some() {
            e.session_file = r.session_file;
        }
        if r.session_id.is_some() {
            e.session_id = r.session_id;
        }
        if r.name.is_some() {
            e.name = r.name;
        }
        if r.model.is_some() {
            e.model = r.model;
        }
        if r.model_id.is_some() {
            e.model_id = r.model_id;
        }
        if r.thinking_level.is_some() {
            e.thinking_level = r.thinking_level;
        }
        if r.terminal.is_some() {
            e.terminal = r.terminal;
        }
        if r.task.is_some() {
            e.task = r.task;
        }
        // Activity is cleared by omission: absent means "no tool running", so
        // unlike the other fields it must not be sticky.
        e.activity = r.activity;
        if r.context_tokens.is_some() {
            e.context_tokens = r.context_tokens;
        }
        if r.context_percent.is_some() {
            e.context_percent = r.context_percent;
        }
        // The extension's structured ticket (read from buck's own session
        // entries) is authoritative. Only fall back to sniffing text when it's
        // absent, e.g. an older extension or a non-buck agent whose cwd is a
        // ticket-named worktree.
        if let Some(t) = r.ticket.filter(|s| !s.is_empty()).or_else(|| {
            e.name
                .as_deref()
                .and_then(find_ticket)
                .or_else(|| e.task.as_deref().and_then(find_ticket))
                .or_else(|| e.cwd.as_deref().and_then(find_worktree_ticket))
                .or_else(|| e.cwd.as_deref().and_then(find_ticket))
        }) {
            e.ticket = Some(t);
        }
    }

    /// Mark agents whose process no longer exists. Covers `pi` being SIGKILLed
    /// or its pane closed, where `session_shutdown` never fires.
    pub fn reap(&mut self) {
        for a in self.agents.values_mut() {
            if a.state != State::Gone && !pid_alive(a.pid) {
                a.state = State::Gone;
                a.activity = None;
            }
        }
    }

    pub fn forget_dead(&mut self) {
        self.agents.retain(|_, a| a.state != State::Gone);
    }

    /// Stable ordering so rows don't jump around under the cursor.
    pub fn sorted(&self) -> Vec<Agent> {
        let mut v: Vec<Agent> = self.agents.values().cloned().collect();
        v.sort_by(|a, b| a.label().cmp(&b.label()).then(a.pid.cmp(&b.pid)));
        v
    }
}

fn pid_alive(pid: u32) -> bool {
    // signal 0 => existence/permission check only
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal report; every optional field absent unless overridden.
    fn report(pid: u32, kind: &str, state: Option<&str>) -> Report {
        Report {
            pid,
            kind: kind.into(),
            state: state.map(Into::into),
            cwd: None,
            session_file: None,
            session_id: None,
            name: None,
            model: None,
            model_id: None,
            thinking_level: None,
            task: None,
            activity: None,
            ticket: None,
            terminal: None,
            context_tokens: None,
            context_percent: None,
            ts: None,
        }
    }

    /// The handshake only runs at session_start, so the surface id arrives on
    /// one report and must survive every heartbeat that omits it — otherwise
    /// focus would work for a moment and then stop.
    #[test]
    fn terminal_id_is_sticky() {
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("idle"));
        r.terminal = Some("2B846B1E-24D2".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].terminal.as_deref(), Some("2B846B1E-24D2"));

        // a later heartbeat without it
        reg.apply(report(1, "status", Some("running")));
        assert_eq!(
            reg.sorted()[0].terminal.as_deref(),
            Some("2B846B1E-24D2"),
            "surface id must not be cleared by omission"
        );
    }

    #[test]
    fn find_ticket_matches_jira_keys() {
        // the shape buck agents are driven with
        assert_eq!(
            find_ticket("/buck ticket COIN-1234").as_deref(),
            Some("COIN-1234")
        );
        // mid-sentence, and in a branch-ish path
        assert_eq!(
            find_ticket("fix the thing for PAY-77 please").as_deref(),
            Some("PAY-77")
        );
        assert_eq!(
            find_ticket("/Users/x/dev/wt/COIN-9/api").as_deref(),
            Some("COIN-9")
        );
        // digits are allowed inside the project key, but it must start alpha
        assert_eq!(find_ticket("A1B-12").as_deref(), Some("A1B-12"));
        // first match wins
        assert_eq!(
            find_ticket("COIN-1 then COIN-2").as_deref(),
            Some("COIN-1")
        );
    }

    #[test]
    fn find_ticket_rejects_non_tickets() {
        assert_eq!(find_ticket(""), None);
        assert_eq!(find_ticket("no ticket here"), None);
        // single-letter project key
        assert_eq!(find_ticket("A-1"), None);
        // lowercase: handled by find_worktree_ticket, not here. Matching it
        // here would turn model ids and `utf-8` into tickets.
        assert_eq!(find_ticket("coin-1234"), None);
        assert_eq!(find_ticket("lego-claude/opus-5"), None);
        assert_eq!(find_ticket("utf-8"), None);
        assert_eq!(find_ticket("/Users/x/dev/lego/brickbank"), None);
        // no digits after the hyphen
        assert_eq!(find_ticket("COIN-abc"), None);
        assert_eq!(find_ticket("COIN-"), None);
        // model ids and dates must not read as tickets
        assert_eq!(find_ticket("gpt-5.6-sol-2026-07-09"), None);
        // must start on a word boundary
        assert_eq!(find_ticket("xCOIN-1"), None);
        assert_eq!(find_ticket("FOO_COIN-1"), None);
        // trailing word char after the digits disqualifies it
        assert_eq!(find_ticket("COIN-12x"), None);
    }

    #[test]
    fn find_worktree_ticket_reads_buck_worktrees() {
        // the real shape buck creates
        assert_eq!(
            find_worktree_ticket(
                "/Users/g/dev/lego/brickbank/.worktrees/buck-coin-2695-pin-actions"
            )
            .as_deref(),
            Some("COIN-2695")
        );
        // no trailing task slug
        assert_eq!(
            find_worktree_ticket("/x/buck-coin-2695").as_deref(),
            Some("COIN-2695")
        );
        // plain repo checkout, and non-buck dirs, must not match
        assert_eq!(find_worktree_ticket("/Users/g/dev/lego/brickbank"), None);
        assert_eq!(find_worktree_ticket("/x/some-other-2695-thing"), None);
        // needs a numeric second segment and a 2+ char project
        assert_eq!(find_worktree_ticket("/x/buck-notanum-abc"), None);
        assert_eq!(find_worktree_ticket("/x/buck-c-1"), None);
    }

    /// The extension reads buck's own `custom` session entries, so the reported
    /// ticket wins over anything sniffed out of text.
    #[test]
    fn reported_ticket_is_authoritative() {
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("running"));
        r.ticket = Some("COIN-2695".into());
        // stale ids in the prose must not win
        r.task = Some("compare against COIN-1111".into());
        r.cwd = Some("/x/.worktrees/buck-coin-9999-old".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].ticket.as_deref(), Some("COIN-2695"));

        // An empty reported ticket falls through to sniffing.
        let mut r = report(2, "status", Some("running"));
        r.ticket = Some(String::new());
        r.cwd = Some("/x/.worktrees/buck-coin-4242-thing".into());
        reg.apply(r);
        let a = reg.sorted().into_iter().find(|a| a.pid == 2).unwrap();
        assert_eq!(a.ticket.as_deref(), Some("COIN-4242"));
    }

    /// A buck pane's last user turn is "implement" / "go" / "yes", and its cwd is
    /// the plain repo, so text sniffing alone finds nothing. This is why the
    /// extension has to report the structured id.
    #[test]
    fn buck_pane_without_reported_ticket_has_nothing_to_sniff() {
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("running"));
        r.task = Some("implement".into());
        r.cwd = Some("/Users/g/dev/lego/brickbank".into());
        reg.apply(r);
        assert!(reg.sorted()[0].ticket.is_none());
    }

    #[test]
    fn ticket_is_sniffed_from_task_name_and_cwd() {
        let mut reg = Registry::default();

        // from the task
        let mut r = report(1, "status", Some("running"));
        r.task = Some("/buck ticket COIN-1234".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].ticket.as_deref(), Some("COIN-1234"));

        // from cwd, when the task has none
        let mut r = report(2, "status", Some("idle"));
        r.cwd = Some("/Users/x/dev/wt/PAY-42".into());
        r.task = Some("look at the tests".into());
        reg.apply(r);
        let a = reg.sorted().into_iter().find(|a| a.pid == 2).unwrap();
        assert_eq!(a.ticket.as_deref(), Some("PAY-42"));

        // from a buck worktree cwd
        let mut r = report(4, "status", Some("running"));
        r.task = Some("implement".into());
        r.cwd = Some("/x/brickbank/.worktrees/buck-coin-2695-pin-actions".into());
        reg.apply(r);
        let a = reg.sorted().into_iter().find(|a| a.pid == 4).unwrap();
        assert_eq!(a.ticket.as_deref(), Some("COIN-2695"));

        // name wins over cwd
        let mut r = report(3, "status", Some("idle"));
        r.name = Some("COIN-7 refactor".into());
        r.cwd = Some("/Users/x/dev/wt/PAY-42".into());
        reg.apply(r);
        let a = reg.sorted().into_iter().find(|a| a.pid == 3).unwrap();
        assert_eq!(a.ticket.as_deref(), Some("COIN-7"));
    }

    #[test]
    fn ticket_survives_later_turns_that_do_not_mention_it() {
        // The `/buck ticket COIN-1234` turn names the ticket once; follow-ups
        // like "now run the tests" must not clear it.
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("running"));
        r.task = Some("/buck ticket COIN-1234".into());
        reg.apply(r);

        let mut r = report(1, "status", Some("running"));
        r.task = Some("now run the tests".into());
        reg.apply(r);
        assert_eq!(
            reg.sorted()[0].ticket.as_deref(),
            Some("COIN-1234"),
            "ticket is sticky across turns"
        );

        // A new ticket does replace it.
        let mut r = report(1, "status", Some("running"));
        r.task = Some("/buck ticket COIN-999".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].ticket.as_deref(), Some("COIN-999"));
    }

    #[test]
    fn agents_without_tickets_have_none() {
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("idle"));
        r.name = Some("scratch".into());
        r.cwd = Some("/Users/x/dev/miami".into());
        r.task = Some("add a ticket column".into());
        reg.apply(r);
        assert!(reg.sorted()[0].ticket.is_none());
    }

    #[test]
    fn state_labels_are_stable() {
        // These strings are rendered in the TUI and asserted on elsewhere.
        assert_eq!(State::Starting.label(), "starting");
        assert_eq!(State::Idle.label(), "idle");
        assert_eq!(State::Running.label(), "running");
        assert_eq!(State::Gone.label(), "gone");
    }

    #[test]
    fn unknown_state_string_falls_back_to_starting() {
        let mut reg = Registry::default();
        reg.apply(report(1, "status", Some("wat")));
        assert_eq!(reg.sorted()[0].state, State::Starting);

        // A missing state field behaves the same way.
        reg.apply(report(2, "status", None));
        let a = reg.sorted().into_iter().find(|a| a.pid == 2).unwrap();
        assert_eq!(a.state, State::Starting);
    }

    #[test]
    fn gone_for_unknown_pid_does_not_create_an_agent() {
        let mut reg = Registry::default();
        reg.apply(report(4242, "gone", None));
        assert!(reg.sorted().is_empty(), "gone must not register a new agent");
    }

    #[test]
    fn explicit_ts_is_preferred_over_wall_clock() {
        let mut reg = Registry::default();
        let mut r = report(7, "status", Some("idle"));
        r.ts = Some(1_234_567);
        reg.apply(r);
        assert_eq!(reg.sorted()[0].updated, 1_234_567);
    }

    #[test]
    fn label_prefers_name_then_cwd_then_pid() {
        let mut reg = Registry::default();

        // pid only
        reg.apply(report(1, "status", Some("idle")));
        assert_eq!(reg.sorted()[0].label(), "pid 1");

        // cwd basename wins over pid
        let mut r = report(1, "status", Some("idle"));
        r.cwd = Some("/a/b/proj".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].label(), "proj");

        // name wins over cwd
        let mut r = report(1, "status", Some("idle"));
        r.name = Some("refactor".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].label(), "refactor");
    }

    #[test]
    fn empty_name_is_ignored_for_label() {
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("idle"));
        r.cwd = Some("/a/b/proj".into());
        r.name = Some(String::new());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].label(), "proj", "blank name must not win");
    }

    #[test]
    fn trailing_slash_cwd_falls_back_to_pid() {
        // rsplit('/') yields "" for a trailing slash, which the filter rejects.
        let mut reg = Registry::default();
        let mut r = report(9, "status", Some("idle"));
        r.cwd = Some("/a/b/".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].label(), "pid 9");
    }

    #[test]
    fn sorted_orders_by_label_then_pid() {
        let mut reg = Registry::default();
        for (pid, name) in [(3u32, "beta"), (1, "alpha"), (2, "alpha")] {
            let mut r = report(pid, "status", Some("idle"));
            r.name = Some(name.into());
            reg.apply(r);
        }
        let got: Vec<(String, u32)> = reg.sorted().iter().map(|a| (a.label(), a.pid)).collect();
        assert_eq!(
            got,
            vec![
                ("alpha".to_string(), 1),
                ("alpha".to_string(), 2),
                ("beta".to_string(), 3)
            ],
            "ties broken by pid so rows never jump"
        );
    }

    #[test]
    fn forget_dead_retains_live_agents() {
        let mut reg = Registry::default();
        reg.apply(report(1, "status", Some("running")));
        reg.apply(report(2, "status", Some("idle")));
        reg.apply(report(2, "gone", None));
        assert_eq!(reg.sorted().len(), 2);

        reg.forget_dead();
        let left = reg.sorted();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].pid, 1);
    }

    #[test]
    fn prettify_model_id_strips_noise() {
        // provider prefix + trailing date
        assert_eq!(
            prettify_model_id("lego-openai/gpt-5.6-sol-2026-07-09"),
            "gpt-5.6-sol"
        );
        // vendor namespace + compact date + vN:N revision
        assert_eq!(
            prettify_model_id("lego-claude/eu.anthropic.claude-haiku-4-5-20251001-v1:0"),
            "claude-haiku-4-5"
        );
        // version dots must survive namespace stripping
        assert_eq!(prettify_model_id("gpt-5.6-sol"), "gpt-5.6-sol");
        // nothing to strip
        assert_eq!(prettify_model_id("opus"), "opus");
    }

    #[test]
    fn prettify_model_id_never_returns_empty() {
        // A date-only tail would otherwise join to ""; the fallback keeps the tail.
        assert_eq!(prettify_model_id("prov/20251001"), "20251001");
        assert_eq!(prettify_model_id(""), "");
    }

    #[test]
    fn model_label_prefers_display_name_over_id() {
        let mut reg = Registry::default();
        let mut r = report(1, "status", Some("idle"));
        r.model = Some("GPT-5.6 Sol (LEGO)".into());
        r.model_id = Some("lego-openai/gpt-5.6-sol-2026-07-09".into());
        reg.apply(r);
        assert_eq!(reg.sorted()[0].model_label(), "GPT-5.6 Sol (LEGO)");

        // A blank display name must fall through to the id.
        let mut r = report(2, "status", Some("idle"));
        r.model = Some(String::new());
        r.model_id = Some("lego-openai/gpt-5.6-sol-2026-07-09".into());
        reg.apply(r);
        let a = reg.sorted().into_iter().find(|a| a.pid == 2).unwrap();
        assert_eq!(a.model_label(), "gpt-5.6-sol");
    }

    #[test]
    fn reap_leaves_already_gone_agents_alone() {
        let mut reg = Registry::default();
        reg.apply(report(std::process::id(), "status", Some("idle")));
        reg.apply(report(std::process::id(), "gone", None));
        reg.reap();
        assert_eq!(reg.sorted()[0].state, State::Gone);
    }
}
