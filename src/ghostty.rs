//! Ghostty control via its AppleScript dictionary.
//!
//! Ghostty 1.3 ships `Ghostty.sdef`, so focusing a surface and spawning a tab
//! are ordinary Apple events — no `System Events` keystroke faking, no Cmd
//! rebinds, no accessibility permission. The two things miami needs:
//!
//!   focus terminal id "<uuid>"            -- raise window, select tab, focus split
//!   new tab in front window with configuration <cfg>
//!
//! The surface uuid comes from the pi extension, which discovers its own
//! surface with a title handshake (see `extension/miami-report.ts`). Nothing
//! here guesses: an agent without a reported `terminal` cannot be focused, and
//! says so, rather than raising some other agent's pane.
//!
//! macOS only. On other platforms Ghostty has no Apple events and every call
//! returns [`Err`].

use std::process::Command;

/// Run an AppleScript, returning trimmed stdout.
///
/// Errors carry osascript's stderr, which is what tells a stale surface id
/// (`Can't get terminal id "…"`) apart from Ghostty not running at all.
fn osascript(script: &str) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("Ghostty scripting is macOS only".into());
    }
    let out = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("osascript: {e}"))?;
    if !out.status.success() {
        return Err(tidy_error(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Reduce osascript's stderr to the part worth putting in a footer.
///
/// It arrives as `…:34:36: execution error: Ghostty got an error: msg. (-1728)`,
/// of which only `msg` means anything to the reader. Unrecognised stderr is
/// passed through rather than blanked, so a surprise is still legible.
fn tidy_error(stderr: &str) -> String {
    let msg = stderr
        .rsplit_once("error: ")
        .map(|(_, m)| m)
        .unwrap_or(stderr)
        .trim()
        // trailing ". (-1728)"
        .trim_end_matches(|c: char| c.is_ascii_digit() || "().- ".contains(c));
    if msg.is_empty() {
        "osascript failed".into()
    } else {
        msg.to_string()
    }
}

/// Like [`quote`], but renders a trailing carriage return as `& return`.
///
/// AppleScript string literals can't hold a newline, so submitting a typed line
/// means concatenating the `return` constant.
fn quote_with_cr(s: &str) -> Result<String, String> {
    match s.strip_suffix('\r') {
        Some(head) => Ok(format!("{} & return", quote(head)?)),
        None => quote(s),
    }
}

/// Quote a Rust string as an AppleScript string literal.
///
/// AppleScript has no escape for anything but `\` and `"`, and a literal
/// newline inside quotes is a syntax error, so those are rejected rather than
/// mangled. Surface ids are uuids and cwds are paths; neither contains one.
fn quote(s: &str) -> Result<String, String> {
    if s.contains('\n') || s.contains('\r') {
        return Err("newline in AppleScript string".into());
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

/// Raise the window, select the tab and focus the split holding `id`.
///
/// One Apple event does all three: `focus` is defined on a terminal (surface),
/// and Ghostty walks up to the tab and window itself.
/// `id` is a Ghostty surface uuid, as reported by the extension.
pub fn focus(id: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"Ghostty\" to focus terminal id {}",
        quote(id)?
    );
    osascript(&script).map(|_| ())
}

/// Open a tab in the frontmost window and type `command` into its shell.
///
/// Uses `initial input` rather than the config's `command`, which looks like the
/// tidier option but isn't: a `command` surface skips the shell, so it inherits
/// *Ghostty's* environment. Launched from Finder or the Dock that's the bare
/// launchd PATH, and `pi` — a node script behind a version manager shim — isn't
/// on it. Typing into the login shell gets exactly the PATH you'd get by
/// opening a tab and typing it yourself, which is the promise this key makes.
///
/// It also means the shell survives `pi` exiting, leaving you a prompt in the
/// right directory instead of a closed tab.
///
/// `cwd` is passed explicitly because `window-inherit-working-directory` may be
/// off (it is in the author's config), in which case a new tab would otherwise
/// open in the configured default rather than next to miami.
///
/// Note `new tab` *requires* the `in` parameter; the bare form raises
/// errAEEventNotHandled (-1708).
pub fn new_tab(cwd: &str, command: &str) -> Result<String, String> {
    // A trailing return submits the line, as if typed.
    let input = format!("{command}\r");
    let script = format!(
        r#"tell application "Ghostty"
  set cfg to new surface configuration
  set initial working directory of cfg to {cwd}
  set initial input of cfg to {input}
  set t to new tab in front window with configuration cfg
  return id of t
end tell"#,
        cwd = quote(cwd)?,
        input = quote_with_cr(&input)?,
    );
    osascript(&script)
}

/// Whether Ghostty is the terminal miami is running in.
///
/// Gates the keys in the footer: on iTerm or tmux they'd only ever error.
pub fn available() -> bool {
    cfg!(target_os = "macos") && std::env::var("TERM_PROGRAM").as_deref() == Ok("ghostty")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_applescript_specials() {
        assert_eq!(quote("plain").unwrap(), "\"plain\"");
        assert_eq!(quote(r#"a"b"#).unwrap(), r#""a\"b""#);
        assert_eq!(quote(r"a\b").unwrap(), r#""a\\b""#);
        // A path with a space needs no escaping, only the surrounding quotes.
        assert_eq!(quote("/Users/x/my dir").unwrap(), "\"/Users/x/my dir\"");
    }

    #[test]
    fn quote_rejects_newlines() {
        // Would otherwise produce a syntax error mid-script, or worse, inject
        // a second statement.
        assert!(quote("a\nb").is_err());
        assert!(quote("a\rb").is_err());
    }

    /// Footer messages come from osascript's stderr, so the noise around the
    /// actual complaint has to go.
    #[test]
    fn tidy_error_keeps_only_the_message() {
        assert_eq!(
            tidy_error(
                "34:36: execution error: Ghostty got an error: Can't get terminal id \"DEAD\". (-1728)\n"
            ),
            "Can't get terminal id \"DEAD\""
        );
        // the -1708 seen when `new tab` is called without `in`
        assert_eq!(
            tidy_error(
                "225:255: execution error: Ghostty got an error: Can't continue new tab. (-1708)\n"
            ),
            "Can't continue new tab"
        );
        // Ghostty not running at all
        assert_eq!(
            tidy_error("execution error: Application isn't running. (-600)\n"),
            "Application isn't running"
        );
        // unrecognised shape passes through rather than being blanked
        assert_eq!(tidy_error("unexpected stderr\n"), "unexpected stderr");
        // nothing usable at all still says something
        assert_eq!(tidy_error(""), "osascript failed");
        assert_eq!(tidy_error("error: (-42)\n"), "osascript failed");
    }

    #[test]
    fn quote_with_cr_appends_the_return_constant() {
        // The submitted line: literal text, then AppleScript's `return`.
        assert_eq!(quote_with_cr("pi\r").unwrap(), "\"pi\" & return");
        // No trailing CR: plain literal.
        assert_eq!(quote_with_cr("pi").unwrap(), "\"pi\"");
        // An embedded CR is still refused.
        assert!(quote_with_cr("a\rb\r").is_err());
    }

    /// The uuids Ghostty hands out, and the paths agents run in, must survive
    /// quoting untouched.
    #[test]
    fn quote_leaves_real_ids_and_paths_alone() {
        let id = "2B846B1E-24D2-42D9-8F8F-21A63CA0543D";
        assert_eq!(quote(id).unwrap(), format!("\"{id}\""));
        let cwd = "/Users/g/dev/lego/brickbank/.worktrees/buck-coin-2695";
        assert_eq!(quote(cwd).unwrap(), format!("\"{cwd}\""));
    }
}
