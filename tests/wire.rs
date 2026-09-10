// Exercises the socket + registry path without a TTY.
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::{thread, time::Duration};

use miami::registry::{Registry, State};
use miami::server;

fn wait_for<F: Fn() -> bool>(f: F) -> bool {
    for _ in 0..100 {
        if f() { return true; }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn wire_end_to_end() {
    let sock = std::env::temp_dir().join(format!("miami-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, Arc::clone(&reg)).expect("bind");

    let mut s = UnixStream::connect(&sock).expect("connect");
    let me = std::process::id();

    // initial registration
    writeln!(s, r#"{{"pid":{me},"type":"status","state":"idle","cwd":"/Users/x/dev/foo","sessionId":"abc","model":"lego-claude/opus-5","contextTokens":48213,"contextPercent":24}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().len() == 1), "agent registered");
    {
        let a = &reg.lock().unwrap().sorted()[0];
        assert_eq!(a.state, State::Idle);
        assert_eq!(a.label(), "foo", "labels by cwd basename when unnamed");
        assert_eq!(a.model.as_deref(), Some("lego-claude/opus-5"));
    }

    // malformed line must not drop the connection
    s.write_all(b"{ not json at all\n").unwrap();

    // partial update: name arrives, cwd/model must be retained
    writeln!(s, r#"{{"pid":{me},"type":"status","state":"running","name":"auth refactor"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted()[0].state == State::Running), "survives malformed line");
    {
        let a = &reg.lock().unwrap().sorted()[0];
        assert_eq!(a.label(), "auth refactor", "name wins over cwd");
        assert_eq!(a.model.as_deref(), Some("lego-claude/opus-5"), "retains model across partial update");
        assert_eq!(a.cwd.as_deref(), Some("/Users/x/dev/foo"), "retains cwd");
    }

    // a dead pid should be reaped
    writeln!(s, r#"{{"pid":999999,"type":"status","state":"idle","cwd":"/Users/x/dev/bar"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().len() == 2));
    reg.lock().unwrap().reap();
    {
        let r = reg.lock().unwrap();
        let dead = r.sorted().into_iter().find(|a| a.pid == 999999).unwrap();
        assert_eq!(dead.state, State::Gone, "nonexistent pid reaped");
        let live = r.sorted().into_iter().find(|a| a.pid == me).unwrap();
        assert_eq!(live.state, State::Running, "live pid untouched by reap");
    }

    // explicit shutdown
    writeln!(s, r#"{{"pid":{me},"type":"gone"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().iter().find(|a| a.pid == me).unwrap().state == State::Gone));

    reg.lock().unwrap().forget_dead();
    assert_eq!(reg.lock().unwrap().sorted().len(), 0, "forget_dead clears");

    let _ = std::fs::remove_file(&sock);
}

#[test]
fn stale_socket_file_is_reclaimed() {
    let sock = std::env::temp_dir().join(format!("miami-stale-{}.sock", std::process::id()));
    std::fs::write(&sock, b"leftover").unwrap();  // not a live socket
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, reg).expect("should reclaim stale socket path");
    assert!(UnixStream::connect(&sock).is_ok());
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn model_label_prefers_display_name() {
    let sock = std::env::temp_dir().join(format!("miami-model-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, Arc::clone(&reg)).unwrap();
    let mut s = UnixStream::connect(&sock).unwrap();

    writeln!(s, r#"{{"pid":1,"type":"status","state":"idle","model":"GPT-5.6 Sol (LEGO)","modelId":"lego-openai/gpt-5.6-sol-2026-07-09"}}"#).unwrap();
    // no display name -> falls back to a tidied id
    writeln!(s, r#"{{"pid":2,"type":"status","state":"idle","modelId":"lego-openai/gpt-5.6-sol-2026-07-09"}}"#).unwrap();
    writeln!(s, r#"{{"pid":3,"type":"status","state":"idle","modelId":"lego-claude/eu.anthropic.claude-haiku-4-5-20251001-v1:0"}}"#).unwrap();
    writeln!(s, r#"{{"pid":4,"type":"status","state":"idle"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().len() == 4));

    let r = reg.lock().unwrap();
    let by = |pid: u32| r.sorted().into_iter().find(|a| a.pid == pid).unwrap();
    assert_eq!(by(1).model_label(), "GPT-5.6 Sol (LEGO)", "display name wins");
    assert_eq!(by(2).model_label(), "gpt-5.6-sol", "date stamp stripped");
    assert_eq!(by(3).model_label(), "claude-haiku-4-5", "vendor ns + date stripped");
    assert_eq!(by(4).model_label(), "-", "nothing known");
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn activity_is_cleared_by_omission() {
    let sock = std::env::temp_dir().join(format!("miami-act-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, Arc::clone(&reg)).unwrap();
    let mut s = UnixStream::connect(&sock).unwrap();

    writeln!(s, r#"{{"pid":1,"type":"status","state":"running","task":"do the thing","activity":"bash: cargo test"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().first().and_then(|a| a.activity.clone()).is_some()));

    // No activity field => tool finished. Must NOT stay sticky like other fields.
    writeln!(s, r#"{{"pid":1,"type":"status","state":"idle","task":"do the thing"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted()[0].activity.is_none()), "activity cleared");
    // task remains sticky
    assert_eq!(reg.lock().unwrap().sorted()[0].task.as_deref(), Some("do the thing"));
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn gone_clears_activity() {
    let sock = std::env::temp_dir().join(format!("miami-gone-act-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, Arc::clone(&reg)).unwrap();
    let mut s = UnixStream::connect(&sock).unwrap();

    writeln!(s, r#"{{"pid":4242,"type":"status","state":"running","activity":"bash: cargo test"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().first().and_then(|a| a.activity.clone()).is_some()));
    writeln!(s, r#"{{"pid":4242,"type":"gone"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted()[0].state == State::Gone));
    // a dead agent must not still show a running tool
    assert!(reg.lock().unwrap().sorted()[0].activity.is_none(), "activity cleared on gone");

    // same for the reaper path (nonexistent pid, no explicit gone)
    writeln!(s, r#"{{"pid":999998,"type":"status","state":"running","activity":"edit: x.rs"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted().len() == 2));
    reg.lock().unwrap().reap();
    let reaped = reg.lock().unwrap().sorted().into_iter().find(|a| a.pid == 999998).unwrap();
    assert_eq!(reaped.state, State::Gone);
    assert!(reaped.activity.is_none(), "reaper clears activity too");
    let _ = std::fs::remove_file(&sock);
}

/// The extension reports buck's structured ticket id; it must survive the wire
/// and take precedence over ids sitting in the task text.
#[test]
fn ticket_arrives_over_the_socket() {
    let sock = std::env::temp_dir().join(format!("miami-ticket-{}.sock", std::process::id()));
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, Arc::clone(&reg)).unwrap();
    let mut s = UnixStream::connect(&sock).unwrap();

    writeln!(s, r#"{{"pid":1,"type":"status","state":"running","task":"implement","ticket":"COIN-2695","cwd":"/Users/g/dev/lego/brickbank"}}"#).unwrap();
    assert!(wait_for(|| reg
        .lock()
        .unwrap()
        .sorted()
        .first()
        .and_then(|a| a.ticket.clone())
        .is_some()));
    assert_eq!(
        reg.lock().unwrap().sorted()[0].ticket.as_deref(),
        Some("COIN-2695"),
        "structured ticket, not sniffed from the task"
    );

    // buck moves the pane to the next ticket without a restart.
    writeln!(s, r#"{{"pid":1,"type":"status","state":"running","task":"implement","ticket":"COIN-2841"}}"#).unwrap();
    assert!(wait_for(
        || reg.lock().unwrap().sorted()[0].ticket.as_deref() == Some("COIN-2841")
    ));

    // A report with no ticket field must not clear it (sticky).
    writeln!(s, r#"{{"pid":1,"type":"status","state":"idle","task":"implement"}}"#).unwrap();
    assert!(wait_for(
        || reg.lock().unwrap().sorted()[0].state == State::Idle
    ));
    assert_eq!(
        reg.lock().unwrap().sorted()[0].ticket.as_deref(),
        Some("COIN-2841"),
        "ticket is sticky across reports that omit it"
    );
    let _ = std::fs::remove_file(&sock);
}

/// The surface id crosses the wire as `terminal` and survives heartbeats that
/// omit it. Without stickiness, focus would work once and then stop, because
/// the handshake only runs at session_start.
#[test]
fn terminal_id_arrives_and_persists() {
    let sock = std::env::temp_dir().join(format!("miami-term-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let reg = Arc::new(Mutex::new(Registry::default()));
    server::serve(&sock, Arc::clone(&reg)).unwrap();
    let mut s = UnixStream::connect(&sock).unwrap();
    let me = std::process::id();

    writeln!(
        s,
        r#"{{"pid":{me},"type":"status","state":"idle","terminal":"2B846B1E-24D2-42D9-8F8F-21A63CA0543D"}}"#
    )
    .unwrap();
    assert!(wait_for(|| reg
        .lock()
        .unwrap()
        .sorted()
        .first()
        .and_then(|a| a.terminal.clone())
        .is_some()));
    assert_eq!(
        reg.lock().unwrap().sorted()[0].terminal.as_deref(),
        Some("2B846B1E-24D2-42D9-8F8F-21A63CA0543D")
    );

    // a heartbeat with no terminal field
    writeln!(s, r#"{{"pid":{me},"type":"status","state":"running"}}"#).unwrap();
    assert!(wait_for(|| reg.lock().unwrap().sorted()[0].state == State::Running));
    assert_eq!(
        reg.lock().unwrap().sorted()[0].terminal.as_deref(),
        Some("2B846B1E-24D2-42D9-8F8F-21A63CA0543D"),
        "surface id must survive heartbeats that omit it"
    );

    let _ = std::fs::remove_file(&sock);
}
