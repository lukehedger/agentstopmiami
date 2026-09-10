use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use miami::registry::{Agent, Registry};
use miami::{ghostty, server, ui};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> std::io::Result<()> {
    let sock = server::socket_path();
    let reg = Arc::new(Mutex::new(Registry::default()));

    if let Err(e) = server::serve(&sock, Arc::clone(&reg)) {
        eprintln!("miami: cannot bind {}: {e}", sock.display());
        eprintln!("is another miami already running?");
        std::process::exit(1);
    }

    let mut term = ratatui::init();
    let res = run(&mut term, reg, &sock.display().to_string());
    ratatui::restore();
    // Leaving the socket behind would block the next run; serve() also clears
    // stale files, but clean up on the happy path.
    let _ = std::fs::remove_file(&sock);
    res
}

fn run(
    term: &mut ratatui::DefaultTerminal,
    reg: Arc<Mutex<Registry>>,
    sock: &str,
) -> std::io::Result<()> {
    let mut selected = 0usize;
    let mut last_reap = Instant::now();
    // Result of the last Ghostty action, shown in the footer until superseded.
    let mut status: Option<String> = None;
    let have_ghostty = ghostty::available();

    loop {
        // Detect panes closed without a clean session_shutdown.
        if last_reap.elapsed() >= Duration::from_secs(2) {
            if let Ok(mut r) = reg.lock() {
                r.reap();
            }
            last_reap = Instant::now();
        }

        let agents = reg.lock().map(|r| r.sorted()).unwrap_or_default();
        if selected >= agents.len() {
            selected = agents.len().saturating_sub(1);
        }
        let chrome = ui::Chrome {
            sock,
            status: status.as_deref(),
            ghostty: have_ghostty,
        };
        term.draw(|f| ui::draw(f, &agents, selected, &chrome))?;

        // Poll so the UI still refreshes when no keys are pressed.
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            // Any keypress retires the last action's message: it answered the
            // previous key, and the arms below set a new one if they have
            // something to say.
            status = None;
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                KeyCode::Down => {
                    if !agents.is_empty() {
                        selected = (selected + 1).min(agents.len() - 1);
                    }
                }
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Home => selected = 0,
                KeyCode::End => selected = agents.len().saturating_sub(1),
                KeyCode::Char('d') => {
                    if let Ok(mut r) = reg.lock() {
                        r.forget_dead();
                    }
                }
                // Raise the selected agent's pane. miami stays running in the
                // background; come back with cmd+[ or the tab bar.
                KeyCode::Enter => status = Some(focus(agents.get(selected))),
                // A fresh pi tab next to miami, in miami's own cwd.
                KeyCode::Char('n') => status = Some(new_tab()),
                _ => {}
            }
        }
    }
}

/// Raise the pane of `agent`, returning the footer message.
///
/// Refuses rather than guesses when the agent didn't report a surface id:
/// picking a pane by cwd or title would raise the wrong agent whenever two run
/// in the same repo, which is the normal case.
fn focus(agent: Option<&Agent>) -> String {
    let Some(a) = agent else {
        return "nothing selected".into();
    };
    let Some(id) = a.terminal.as_deref() else {
        return format!(
            "{}: no pane reported — update the extension and restart that pi",
            a.label()
        );
    };
    match ghostty::focus(id) {
        Ok(()) => format!("focused {}", a.label()),
        // Most often the pane was closed, or this isn't Ghostty.
        Err(e) => format!("cannot focus {}: {e}", a.label()),
    }
}

/// Open a new tab running `pi` in miami's own cwd.
fn new_tab() -> String {
    let cwd = match std::env::current_dir() {
        Ok(d) => d.display().to_string(),
        Err(e) => return format!("cannot read cwd: {e}"),
    };
    // Overridable so a wrapper, or `pi --model …`, can be launched instead.
    let cmd = std::env::var("MIAMI_NEW_TAB_CMD").unwrap_or_else(|_| "pi".into());
    match ghostty::new_tab(&cwd, &cmd) {
        Ok(_) => format!("opened a tab running {cmd}"),
        Err(e) => format!("cannot open tab: {e}"),
    }
}
