use crate::registry::{Registry, Report};
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{env, fs, thread};

pub fn socket_path() -> PathBuf {
    if let Ok(p) = env::var("MIAMI_SOCK") {
        return PathBuf::from(p);
    }
    let dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| env::temp_dir().display().to_string());
    Path::new(&dir).join("miami.sock")
}

/// Bind the socket and serve connections on background threads.
/// Stale socket files from an unclean exit are removed first.
pub fn serve(path: &Path, reg: Arc<Mutex<Registry>>) -> std::io::Result<()> {
    if path.exists() && UnixStream::connect(path).is_err() {
        let _ = fs::remove_file(path);
    }
    let listener = UnixListener::bind(path)?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let reg = Arc::clone(&reg);
            thread::spawn(move || handle(stream, reg));
        }
    });
    Ok(())
}

fn handle(stream: UnixStream, reg: Arc<Mutex<Registry>>) {
    // Strict JSONL: split on \n only, tolerate a trailing \r.
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.strip_suffix('\r').unwrap_or(&line);
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Report>(line) {
            Ok(report) => {
                if let Ok(mut r) = reg.lock() {
                    r.apply(report);
                }
            }
            // A malformed line must never kill the connection.
            Err(_) => continue,
        }
    }
}
