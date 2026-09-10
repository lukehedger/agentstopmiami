use crate::registry::{Agent, State, now_ms};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

/// Chrome around the agent list: what the footer offers and what the last
/// action said. Grouped into a struct so adding a line doesn't re-thread every
/// call site and test.
pub struct Chrome<'a> {
    pub sock: &'a str,
    /// Result of the last focus / new-tab attempt, shown in the footer until
    /// the next one. `None` shows the key hints instead.
    pub status: Option<&'a str>,
    /// Whether Ghostty scripting is usable. Gates the focus/new-tab hints:
    /// under tmux or iTerm those keys can only fail, so they aren't advertised.
    pub ghostty: bool,
}

pub fn draw(f: &mut Frame, agents: &[Agent], selected: usize, chrome: &Chrome) {
    let sock = chrome.sock;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(12),
            Constraint::Length(1),
        ])
        .split(f.area());

    let running = agents.iter().filter(|a| a.state == State::Running).count();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " miami ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {} agent{}  ·  {} running",
                agents.len(),
                if agents.len() == 1 { "" } else { "s" },
                running
            )),
        ])),
        chunks[0],
    );

    if agents.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from("  No agents reporting."),
                Line::from(""),
                Line::from(Span::styled(
                    format!("  listening on {sock}"),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "  start pi in a Ghostty pane; the extension connects on session_start",
                    Style::default().fg(Color::DarkGray),
                )),
            ])
            .block(Block::default().borders(Borders::ALL).title(" agents ")),
            chunks[1],
        );
    } else {
        // Width available for the task line, minus the 4-space indent and
        // the list block's borders.
        let task_width = chunks[1].width.saturating_sub(7) as usize;
        // Reserve the ticket column only when something has a ticket, so a
        // non-buck dashboard doesn't lose width to an always-blank column.
        let ticket_width = agents
            .iter()
            .filter_map(|a| a.ticket.as_deref())
            .map(|t| t.chars().count())
            .max()
            .map(|w| w.clamp(10, 16))
            .unwrap_or(0);
        let items: Vec<ListItem> = agents
            .iter()
            .map(|a| {
                let (dot, colour) = match a.state {
                    State::Running => ("●", Color::Green),
                    State::Idle => ("●", Color::Magenta),
                    State::Starting => ("◌", Color::Blue),
                    State::Gone => ("○", Color::DarkGray),
                };
                let mut head = vec![
                    Span::styled(format!(" {dot} "), Style::default().fg(colour)),
                    Span::styled(
                        format!("{:<24}", truncate(&a.label(), 24)),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ];
                if ticket_width > 0 {
                    // Ticket ids are the primary handle on a buck agent, so they
                    // get a colour of their own rather than the dim metadata grey.
                    head.push(Span::styled(
                        format!(
                            "{:<w$}",
                            truncate(a.ticket.as_deref().unwrap_or(""), ticket_width),
                            w = ticket_width + 1
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                head.extend([
                    Span::styled(
                        format!("{:<9}", a.state.label()),
                        Style::default().fg(colour),
                    ),
                    Span::styled(
                        format!("{:<24}", truncate(&a.model_label(), 24)),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(ctx_label(a), Style::default().fg(Color::DarkGray)),
                ]);
                let head = Line::from(head);
                // Task is sticky: always shown, so you never lose sight of what
                // the agent was asked to do.
                let task = Line::from(Span::styled(
                    format!(
                        "    {}",
                        truncate(a.task.as_deref().unwrap_or("—"), task_width)
                    ),
                    Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::ITALIC),
                ));
                // The activity line is always reserved, blank when no tool is
                // running: variable-height rows made the list jump as tool calls
                // came and went.
                let activity = match a.activity.as_deref() {
                    Some(act) => Line::from(vec![
                        Span::styled("    ↳ ", Style::default().fg(Color::Cyan)),
                        Span::styled(
                            truncate(act, task_width.saturating_sub(2)),
                            Style::default().fg(Color::Cyan),
                        ),
                    ]),
                    None => Line::from(""),
                };
                // Trailing blank separates agents; also constant, so no jumping.
                ListItem::new(vec![head, task, activity, Line::from("")])
            })
            .collect();

        let mut st = ListState::default();
        st.select(Some(selected.min(agents.len().saturating_sub(1))));
        f.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" agents "))
                .highlight_style(Style::default().bg(Color::Rgb(30, 40, 55)))
                .highlight_symbol(""),
            chunks[1],
            &mut st,
        );
    }

    detail(f, agents.get(selected), chunks[2]);

    // The status of the last Ghostty action displaces the hints: it's the
    // answer to the key you just pressed, so it matters more than the menu.
    let footer = match chrome.status {
        Some(msg) => Span::styled(format!(" {msg}"), Style::default().fg(Color::Yellow)),
        None if chrome.ghostty => Span::styled(
            " ↑/↓ move · enter focus pane · n new pi tab · d hide dead · q quit",
            Style::default().fg(Color::DarkGray),
        ),
        None => Span::styled(
            " ↑/↓ move · d hide dead · q quit",
            Style::default().fg(Color::DarkGray),
        ),
    };
    f.render_widget(Paragraph::new(footer), chunks[3]);
}

fn detail(f: &mut Frame, agent: Option<&Agent>, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" detail ");
    let Some(a) = agent else {
        f.render_widget(Paragraph::new("").block(block), area);
        return;
    };
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!(" {k:<10}"), Style::default().fg(Color::DarkGray)),
            Span::raw(v),
        ])
    };
    let age = now_ms().saturating_sub(a.updated) / 1000;
    f.render_widget(
        Paragraph::new(vec![
            // Ticket rides on the label row rather than taking one of its own:
            // detail height is fixed, and list rows are scarcer than this line.
            row(
                "label",
                match a.ticket.as_deref() {
                    Some(t) => format!("{}   {t}", a.label()),
                    None => a.label(),
                },
            ),
            row("state", format!("{} ({age}s ago)", a.state.label())),
            // Pane rides on the pid row rather than taking one of its own: both
            // answer "which process/window is this", and detail height is fixed
            // — a row here is a row taken from the agent list.
            row(
                "pid",
                match a.terminal.as_deref() {
                    Some(t) => format!("{}   pane {t}", a.pid),
                    None => format!("{}   pane -  (cannot focus)", a.pid),
                },
            ),
            row("cwd", a.cwd.clone().unwrap_or_else(|| "-".into())),
            row(
                "model",
                format!(
                    "{}{}",
                    a.model_label(),
                    a.thinking_level
                        .as_deref()
                        .map(|t| format!("  thinking:{t}"))
                        .unwrap_or_default()
                ),
            ),
            row(
                "model id",
                a.model_id.clone().unwrap_or_else(|| "-".into()),
            ),
            row("context", ctx_label(a)),
            row("task", a.task.clone().unwrap_or_else(|| "-".into())),
            row("doing", a.activity.clone().unwrap_or_else(|| "-".into())),
            row("session", a.session_id.clone().unwrap_or_else(|| "-".into())),
        ])
        .block(block),
        area,
    );
}

fn ctx_label(a: &Agent) -> String {
    match (a.context_tokens, a.context_percent) {
        (Some(t), Some(p)) => format!("{} tok ({p:.0}%)", thousands(t)),
        (Some(t), None) => format!("{} tok", thousands(t)),
        // null immediately after compaction, until a fresh assistant response.
        _ => "-".into(),
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    format!("{}…", s.chars().take(keep).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn agent(pid: u32, state: State) -> Agent {
        Agent {
            pid,
            state,
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
            updated: now_ms(),
        }
    }

    /// Flatten a rendered frame into one string per row.
    fn render(agents: &[Agent], selected: usize) -> Vec<String> {
        render_with(agents, selected, None, true)
    }

    fn render_with(
        agents: &[Agent],
        selected: usize,
        status: Option<&str>,
        ghostty: bool,
    ) -> Vec<String> {
        let chrome = Chrome {
            sock: "/tmp/miami.sock",
            status,
            ghostty,
        };
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|f| draw(f, agents, selected, &chrome)).unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(48_213), "48,213");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn truncate_is_char_aware() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly-10", 10), "exactly-10");
        assert_eq!(truncate("abcdefghijk", 10), "abcdefghi…");
        // Multi-byte input must not panic or split a codepoint.
        assert_eq!(truncate("ünïcödé-string-here", 8), "ünïcödé…");
    }

    #[test]
    fn ctx_label_covers_missing_values() {
        let mut a = agent(1, State::Idle);
        assert_eq!(ctx_label(&a), "-", "no context yet");

        a.context_tokens = Some(48_213);
        assert_eq!(ctx_label(&a), "48,213 tok", "percent absent after compaction");

        a.context_percent = Some(24.4);
        assert_eq!(ctx_label(&a), "48,213 tok (24%)", "percent is rounded");

        a.context_tokens = None;
        assert_eq!(ctx_label(&a), "-", "percent alone is not enough");
    }

    #[test]
    fn empty_state_shows_socket_hint() {
        let rows = render(&[], 0);
        let all = rows.join("\n");
        assert!(all.contains("0 agents"), "header pluralises zero");
        assert!(all.contains("No agents reporting."));
        assert!(all.contains("/tmp/miami.sock"), "shows where it listens");
    }

    #[test]
    fn header_pluralises_and_counts_running() {
        let one = render(&[agent(1, State::Idle)], 0).join("\n");
        assert!(one.contains("1 agent "), "singular for one agent");
        assert!(one.contains("0 running"));

        let many = render(
            &[
                agent(1, State::Running),
                agent(2, State::Running),
                agent(3, State::Idle),
            ],
            0,
        )
        .join("\n");
        assert!(many.contains("3 agents"));
        assert!(many.contains("2 running"), "only Running counts");
    }

    #[test]
    fn rows_render_label_state_and_model() {
        let mut a = agent(42, State::Running);
        a.name = Some("auth refactor".into());
        a.model = Some("lego-claude/opus-5".into());
        a.context_tokens = Some(48_213);
        a.context_percent = Some(24.0);

        let all = render(&[a], 0).join("\n");
        assert!(all.contains("auth refactor"));
        assert!(all.contains("running"));
        assert!(all.contains("lego-claude/opus-5"));
        assert!(all.contains("48,213 tok (24%)"));
    }

    #[test]
    fn ticket_shows_on_the_row_and_in_detail() {
        let mut a = agent(1, State::Running);
        a.name = Some("auth refactor".into());
        a.ticket = Some("COIN-1234".into());
        a.task = Some("/buck ticket COIN-1234".into());
        let rows = render(&[a], 0);

        let head = rows.iter().find(|r| r.contains("auth refactor")).unwrap();
        assert!(head.contains("COIN-1234"), "ticket on the row: {head:?}");
        assert!(head.contains("running"), "state still on the row: {head:?}");

        let label = rows
            .iter()
            .find(|r| r.contains("label"))
            .expect("detail label row");
        assert!(label.contains("COIN-1234"), "ticket in detail: {label:?}");
    }

    /// The ticket column is only worth its width when something has a ticket.
    #[test]
    fn ticketless_agents_reserve_no_ticket_column() {
        let mut a = agent(1, State::Running);
        a.name = Some("auth refactor".into());
        let plain = render(&[a.clone()], 0);

        a.ticket = Some("COIN-1234".into());
        let ticketed = render(&[a], 0);

        // The header also says "N running", so match the agent row by label.
        let col_of = |rows: &Vec<String>| {
            rows.iter()
                .find(|r| r.contains("auth refactor"))
                .and_then(|r| r.find("running"))
                .unwrap()
        };
        assert!(
            col_of(&ticketed) > col_of(&plain),
            "state shifts right only once a ticket exists"
        );
    }

    #[test]
    fn ticket_column_does_not_overflow_the_width() {
        let mut a = agent(1, State::Running);
        a.name = Some("a".repeat(40));
        a.ticket = Some("VERYLONGPROJECT-123456".into());
        a.task = Some("x".repeat(200));
        let rows = render(&[a], 0);
        assert!(rows.iter().all(|r| r.chars().count() <= 100));
    }

    #[test]
    fn detail_pane_shows_selected_agent() {
        let mut first = agent(1, State::Idle);
        first.name = Some("alpha".into());
        let mut second = agent(2, State::Running);
        second.name = Some("beta".into());
        second.cwd = Some("/Users/x/dev/beta".into());
        second.session_id = Some("sess-xyz".into());
        second.model = Some("opus".into());
        second.thinking_level = Some("high".into());

        let all = render(&[first, second], 1).join("\n");
        assert!(all.contains("sess-xyz"), "detail follows selection");
        assert!(all.contains("/Users/x/dev/beta"));
        assert!(all.contains("thinking:high"));
        assert!(!all.contains("alpha") || all.contains("beta"), "beta is selected");
    }

    /// The footer advertises the Ghostty keys only where they can work, and a
    /// focus/spawn result replaces the hints while it's showing.
    #[test]
    fn footer_shows_ghostty_keys_and_status() {
        let a = [agent(1, State::Idle)];

        let hints = render_with(&a, 0, None, true).join("\n");
        assert!(hints.contains("enter focus pane"), "{hints}");
        assert!(hints.contains("n new pi tab"));

        // Not Ghostty: those keys can only error, so they aren't offered.
        let bare = render_with(&a, 0, None, false).join("\n");
        assert!(!bare.contains("enter focus pane"), "{bare}");
        assert!(bare.contains("d hide dead"), "other keys still listed");

        // A status message displaces the hints: it answers the key just pressed.
        let msg = render_with(&a, 0, Some("focused alpha"), true).join("\n");
        assert!(msg.contains("focused alpha"), "{msg}");
        assert!(!msg.contains("enter focus pane"));
    }

    /// The pane id shares the pid row, and says so when it's missing — that's
    /// the only place you can find out why enter won't work.
    #[test]
    fn detail_shows_pane_id_beside_pid() {
        let mut a = agent(42, State::Idle);
        a.terminal = Some("2B846B1E-24D2-42D9".into());
        let with = render(&[a.clone()], 0).join("\n");
        assert!(with.contains("42   pane 2B846B1E-24D2-42D9"), "{with}");

        a.terminal = None;
        let without = render(&[a], 0).join("\n");
        assert!(without.contains("cannot focus"), "{without}");
    }

    #[test]
    fn missing_detail_fields_render_as_dashes() {
        let rows = render(&[agent(7, State::Starting)], 0);
        let all = rows.join("\n");
        assert!(all.contains("starting"));
        assert!(all.contains("pid 7"), "falls back to pid label");
        // cwd / model / session unset -> "-" on each of those detail rows.
        for key in ["cwd", "model", "session"] {
            let row = rows
                .iter()
                .find(|r| r.contains(key))
                .unwrap_or_else(|| panic!("no {key} row"));
            assert!(row.contains('-'), "{key} row should show a dash: {row:?}");
        }
    }

    #[test]
    fn out_of_range_selection_does_not_panic() {
        // main.rs clamps, but draw must be defensive on its own.
        let rows = render(&[agent(1, State::Idle)], 99);
        assert!(rows.join("\n").contains("1 agent "));
    }

    #[test]
    fn task_line_renders_under_the_row() {
        let mut a = agent(1, State::Running);
        a.name = Some("auth refactor".into());
        a.task = Some("add a task summary to the dashboard".into());
        let rows = render(&[a], 0);
        let joined = rows.join("\n");
        assert!(joined.contains("auth refactor"), "label present");
        assert!(
            joined.contains("add a task summary to the dashboard"),
            "task line present:\n{joined}"
        );
        // task must be on its own line, not appended to the header row
        let head = rows.iter().find(|r| r.contains("auth refactor")).unwrap();
        assert!(!head.contains("add a task summary"), "task not on header row");
    }

    #[test]
    fn missing_task_renders_a_dash() {
        let rows = render(&[agent(1, State::Idle)], 0);
        let joined = rows.join("\n");
        // em dash placeholder on the list line, "-" in the detail pane
        assert!(joined.contains("\u{2014}"), "placeholder for absent task");
        assert!(joined.contains("task"), "detail pane has a task row");
    }

    #[test]
    fn long_task_is_truncated_to_width() {
        let mut a = agent(1, State::Idle);
        a.task = Some("x".repeat(400));
        let rows = render(&[a], 0);
        // nothing may exceed the 100-col test backend
        assert!(rows.iter().all(|r| r.chars().count() <= 100));
        assert!(rows.iter().any(|r| r.contains('\u{2026}')), "ellipsised");
    }

    #[test]
    fn task_and_activity_show_together_while_running() {
        let mut a = agent(1, State::Running);
        a.name = Some("auth refactor".into());
        a.task = Some("add the tool call line".into());
        a.activity = Some("bash: cargo test".into());
        let rows = render(&[a], 0);

        let head = rows.iter().position(|r| r.contains("auth refactor")).unwrap();
        let task = rows
            .iter()
            .position(|r| r.contains("add the tool call line"))
            .expect("task line present");
        let act = rows
            .iter()
            .position(|r| r.contains("bash: cargo test"))
            .expect("activity line present");

        // three distinct lines, in order: header, task, activity
        assert_eq!(task, head + 1, "task directly under header");
        assert_eq!(act, task + 1, "activity directly under task");
        assert!(rows[act].contains('\u{21b3}'), "activity is marked");
    }

    #[test]
    fn idle_agent_reserves_a_blank_activity_line() {
        let mut a = agent(1, State::Idle);
        a.task = Some("waiting for input".into());
        let rows = render(&[a], 0);
        let task = rows
            .iter()
            .position(|r| r.contains("waiting for input"))
            .unwrap();
        // line is present but empty, and carries no marker.
        // Rows include the list block's border glyphs, so strip those first.
        let inner = rows[task + 1].replace('\u{2502}', "");
        assert!(inner.trim().is_empty(), "activity line blank when idle: [{}]", rows[task + 1]);
        assert!(!rows[task + 1].contains('\u{21b3}'));
    }

    /// Row height must not depend on whether a tool is running, or the list
    /// jumps under the cursor as activity appears and disappears.
    #[test]
    fn row_height_is_constant_regardless_of_activity() {
        let mut busy = agent(1, State::Running);
        busy.name = Some("one".into());
        busy.task = Some("first task".into());
        busy.activity = Some("bash: cargo test".into());
        let mut second = agent(2, State::Idle);
        second.name = Some("two".into());
        second.task = Some("second task".into());

        let with = render(&[busy.clone(), second.clone()], 0);
        // same pair, but the first agent's tool has finished
        busy.activity = None;
        let without = render(&[busy, second], 0);

        let row_of = |rows: &Vec<String>, needle: &str| {
            rows.iter().position(|r| r.contains(needle)).unwrap()
        };
        assert_eq!(
            row_of(&with, "two"),
            row_of(&without, "two"),
            "second agent must not move when the first agent's activity clears"
        );
    }
}


