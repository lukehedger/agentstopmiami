//! Headless miami: same socket server and registry, but prints the agent table
//! to stdout instead of drawing a TUI. Useful for verifying the wire in a pipe,
//! a CI job, or anywhere without a terminal.
//!
//!     miami-tail            # redraw the table on change
//!     miami-tail --raw      # dump every report line as it arrives

use miami::registry::Registry;
use miami::server;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
    let raw = std::env::args().any(|a| a == "--raw");
    let sock = server::socket_path();
    let reg = Arc::new(Mutex::new(Registry::default()));

    if let Err(e) = server::serve(&sock, Arc::clone(&reg)) {
        eprintln!("miami-tail: cannot bind {}: {e}", sock.display());
        eprintln!("is miami (or another miami-tail) already running?");
        std::process::exit(1);
    }
    println!("listening on {}", sock.display());

    let mut last = String::new();
    loop {
        if let Ok(mut r) = reg.lock() {
            r.reap();
        }
        let agents = reg.lock().map(|r| r.sorted()).unwrap_or_default();

        let mut out = String::new();
        for a in &agents {
            out.push_str(&format!(
                "{:<7} {:<20} {:<8} {:<22} {:>16}\n",
                a.pid,
                a.label(),
                a.state.label(),
                a.model_label(),
                match (a.context_tokens, a.context_percent) {
                    (Some(t), Some(p)) => format!("{t} tok ({p:.0}%)"),
                    _ => "-".into(),
                },
            ));
            // Task is sticky; activity only while a tool runs. Same shape as the TUI.
            out.push_str(&format!(
                "        {}\n",
                a.task.as_deref().unwrap_or("—")
            ));
            if let Some(act) = a.activity.as_deref() {
                out.push_str(&format!("        ↳ {act}\n"));
            }
        }
        if out != last {
            if raw {
                print!("{out}");
            } else {
                // clear + home, so it behaves like a live table
                print!("\x1b[2J\x1b[H");
                println!("{} agent(s)  ·  socket {}\n", agents.len(), sock.display());
                print!("{out}");
            }
            use std::io::Write;
            let _ = std::io::stdout().flush();
            last = out;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
