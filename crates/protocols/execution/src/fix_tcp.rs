//! FIX 4.2 framing over TCP and a paper acceptor for loopback / local demos.
//!
//! The OMS venue is always a FIX **initiator**. Modes:
//! - **Loopback**: spawn a paper acceptor thread, then connect to it over TCP.
//! - **Connect**: connect to an external acceptor (vendor / counterparty).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::error::ExecutionError;
use crate::fix::{decode_fix, encode_fix, with_poss_dup, FixConfig};

const SOH: u8 = 0x01;

/// Blocking read of one SOH-delimited FIX message (through trailer `10=`).
///
/// # Errors
///
/// Returns [`ExecutionError::Transport`] on EOF or I/O failure.
pub fn read_fix_message(reader: &mut impl Read) -> Result<String, ExecutionError> {
    let mut buf = Vec::with_capacity(256);
    let mut chunk = [0_u8; 512];
    loop {
        if let Some(end) = complete_message_len(&buf) {
            let msg = String::from_utf8(buf[..end].to_vec())
                .map_err(|e| ExecutionError::Transport(format!("fix utf8: {e}")))?;
            return Ok(msg);
        }
        let n = reader
            .read(&mut chunk)
            .map_err(|e| ExecutionError::Transport(format!("fix read: {e}")))?;
        if n == 0 {
            return Err(ExecutionError::Transport("fix eof".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 1024 * 1024 {
            return Err(ExecutionError::Transport("fix message too large".into()));
        }
    }
}

/// Writes one FIX message (must already include trailing SOH).
///
/// # Errors
///
/// Returns [`ExecutionError::Transport`] on I/O failure.
pub fn write_fix_message(writer: &mut impl Write, wire: &str) -> Result<(), ExecutionError> {
    writer
        .write_all(wire.as_bytes())
        .map_err(|e| ExecutionError::Transport(format!("fix write: {e}")))?;
    writer
        .flush()
        .map_err(|e| ExecutionError::Transport(format!("fix flush: {e}")))?;
    Ok(())
}

fn complete_message_len(buf: &[u8]) -> Option<usize> {
    // Complete when we see \x0110=XXX\x01
    let mut i = 0;
    while i + 5 < buf.len() {
        if buf[i] == SOH && buf.get(i + 1..i + 4) == Some(b"10=") {
            let cs = i + 4;
            if cs + 3 < buf.len() && buf[cs + 3] == SOH {
                return Some(cs + 4);
            }
        }
        i += 1;
    }
    None
}

/// Connects a TCP initiator stream with a short connect timeout.
///
/// # Errors
///
/// Returns transport errors when the address is invalid or connect fails.
pub fn connect_fix(addr: &str, timeout: Duration) -> Result<TcpStream, ExecutionError> {
    let mut last = ExecutionError::Transport(format!("fix resolve failed: {addr}"));
    let addrs = addr
        .to_socket_addrs()
        .map_err(|e| ExecutionError::Transport(format!("fix resolve {addr}: {e}")))?;
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                s.set_read_timeout(Some(Duration::from_secs(5)))
                    .map_err(|e| ExecutionError::Transport(format!("fix set timeout: {e}")))?;
                s.set_write_timeout(Some(Duration::from_secs(5)))
                    .map_err(|e| ExecutionError::Transport(format!("fix set timeout: {e}")))?;
                return Ok(s);
            }
            Err(e) => {
                last = ExecutionError::Transport(format!("fix connect {a}: {e}"));
            }
        }
    }
    Err(last)
}

/// Guard that stops a loopback acceptor thread when dropped.
#[derive(Debug)]
pub struct FixTcpAcceptorGuard {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for FixTcpAcceptorGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

/// Spawns a single-connection paper FIX acceptor. Returns the bound address and a drop guard.
///
/// # Errors
///
/// Returns transport errors when bind fails.
pub fn spawn_paper_acceptor(
    bind: &str,
    config: FixConfig,
) -> Result<(String, FixTcpAcceptorGuard), ExecutionError> {
    let listener = TcpListener::bind(bind)
        .map_err(|e| ExecutionError::Transport(format!("fix acceptor bind {bind}: {e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| ExecutionError::Transport(format!("fix acceptor nonblocking: {e}")))?;
    let local = listener
        .local_addr()
        .map_err(|e| ExecutionError::Transport(format!("fix acceptor local_addr: {e}")))?;
    let addr = local.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let join = thread::Builder::new()
        .name("shinrai-fix-acceptor".into())
        .spawn(move || paper_acceptor_loop(&listener, &config, &stop_thread))
        .map_err(|e| ExecutionError::Transport(format!("fix acceptor spawn: {e}")))?;
    Ok((
        addr,
        FixTcpAcceptorGuard {
            stop,
            join: Some(join),
        },
    ))
}

fn paper_acceptor_loop(listener: &TcpListener, config: &FixConfig, stop: &Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let Some(mut stream) = accept_one(listener, stop) else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_nonblocking(false);
        run_session(&mut stream, config, stop);
    }
}

fn accept_one(listener: &TcpListener, stop: &Arc<AtomicBool>) -> Option<TcpStream> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        match listener.accept() {
            Ok((s, _)) => return Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

fn run_session(stream: &mut TcpStream, config: &FixConfig, stop: &Arc<AtomicBool>) {
    let mut next_out = 1_u64;
    let mut next_venue = 1_u64;
    let mut next_exec = 1_u64;
    let mut outbound: Vec<(u64, String)> = Vec::new();
    let mut working: HashMap<String, Working> = HashMap::new();

    while !stop.load(Ordering::SeqCst) {
        let Ok(wire) = read_fix_message(stream) else {
            break;
        };
        let Ok(map) = decode_fix(&wire) else {
            continue;
        };
        let Some(msg_type) = map.get("35").map(String::as_str) else {
            continue;
        };
        let done = dispatch_inbound(
            stream,
            config,
            &mut next_out,
            &mut outbound,
            &mut next_venue,
            &mut next_exec,
            &mut working,
            msg_type,
            &map,
        );
        if done {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_inbound(
    stream: &mut TcpStream,
    config: &FixConfig,
    next_out: &mut u64,
    outbound: &mut Vec<(u64, String)>,
    next_venue: &mut u64,
    next_exec: &mut u64,
    working: &mut HashMap<String, Working>,
    msg_type: &str,
    map: &std::collections::BTreeMap<String, String>,
) -> bool {
    match msg_type {
        "A" => {
            let _ = send_acceptor(
                stream,
                config,
                next_out,
                outbound,
                "A",
                &[("98", "0"), ("108", "30")],
            );
            false
        }
        "0" => {
            let _ = send_acceptor(stream, config, next_out, outbound, "0", &[]);
            false
        }
        "5" => {
            let _ = send_acceptor(stream, config, next_out, outbound, "5", &[]);
            true
        }
        "2" => {
            let _ = handle_resend_request(stream, config, next_out, outbound, map);
            false
        }
        "D" => {
            let _ = handle_new(
                stream,
                config,
                next_out,
                outbound,
                next_venue,
                next_exec,
                working,
                map,
            );
            false
        }
        "F" => {
            let _ = handle_cancel(stream, config, next_out, outbound, working, map);
            false
        }
        "G" => {
            let _ = handle_replace(stream, config, next_out, outbound, working, map);
            false
        }
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct Working {
    venue_order_id: String,
    order_qty: i64,
    cum_qty: i64,
    price: i64,
    side: String,
    canceled: bool,
}

fn send_acceptor(
    stream: &mut TcpStream,
    config: &FixConfig,
    next_out: &mut u64,
    outbound: &mut Vec<(u64, String)>,
    msg_type: &str,
    body: &[(&str, &str)],
) -> Result<(), ExecutionError> {
    let seq = *next_out;
    *next_out = next_out.saturating_add(1);
    // Acceptor: Sender=target_comp (SIM), Target=initiator (SHINRAI)
    let wire = encode_fix(
        &config.target_comp_id,
        &config.sender_comp_id,
        msg_type,
        seq,
        body,
    );
    outbound.push((seq, wire.clone()));
    write_fix_message(stream, &wire)
}

fn handle_resend_request(
    stream: &mut TcpStream,
    config: &FixConfig,
    next_out: &mut u64,
    outbound: &[(u64, String)],
    map: &std::collections::BTreeMap<String, String>,
) -> Result<(), ExecutionError> {
    let begin: u64 = map.get("7").and_then(|s| s.parse().ok()).unwrap_or(0);
    let end: u64 = map.get("16").and_then(|s| s.parse().ok()).unwrap_or(0);
    let end = if end == 0 {
        next_out.saturating_sub(1)
    } else {
        end
    };
    if begin == 0 || begin > end {
        return Ok(());
    }
    let mut cursor = begin;
    while cursor <= end {
        if let Some((_, wire)) = outbound.iter().find(|(s, _)| *s == cursor) {
            let poss = with_poss_dup(wire).map_err(ExecutionError::InvalidState)?;
            write_fix_message(stream, &poss)?;
            cursor = cursor.saturating_add(1);
        } else {
            let next_real = outbound
                .iter()
                .map(|(s, _)| *s)
                .filter(|s| *s > cursor)
                .min()
                .unwrap_or(end.saturating_add(1));
            let new_seq = next_real.min(end.saturating_add(1));
            let new_s = new_seq.to_string();
            let wire = encode_fix(
                &config.target_comp_id,
                &config.sender_comp_id,
                "4",
                cursor,
                &[("123", "Y"), ("36", new_s.as_str())],
            );
            write_fix_message(stream, &wire)?;
            cursor = new_seq;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_new(
    stream: &mut TcpStream,
    config: &FixConfig,
    next_out: &mut u64,
    outbound: &mut Vec<(u64, String)>,
    next_venue: &mut u64,
    next_exec: &mut u64,
    working: &mut HashMap<String, Working>,
    map: &std::collections::BTreeMap<String, String>,
) -> Result<(), ExecutionError> {
    let cl_ord = map.get("11").cloned().unwrap_or_default();
    let qty: i64 = map.get("38").and_then(|s| s.parse().ok()).unwrap_or(0);
    let px: i64 = map.get("44").and_then(|s| s.parse().ok()).unwrap_or(0);
    let side = map.get("54").cloned().unwrap_or_else(|| "1".into());
    let tif = map.get("59").map_or("1", String::as_str);
    if qty <= 0 || px <= 0 || cl_ord.is_empty() {
        return Ok(());
    }
    if tif == "4" && !config.auto_fill {
        let venue = format!("FIX-{next_venue}");
        *next_venue = next_venue.saturating_add(1);
        let body = [
            ("37", venue.as_str()),
            ("11", cl_ord.as_str()),
            ("150", "8"),
            ("39", "8"),
            ("54", side.as_str()),
            ("14", "0"),
            ("151", "0"),
            ("32", "0"),
            ("31", "0"),
            ("58", "fok cannot rest"),
        ];
        return send_acceptor(stream, config, next_out, outbound, "8", &body);
    }
    let venue = format!("FIX-{next_venue}");
    *next_venue = next_venue.saturating_add(1);
    working.insert(
        cl_ord.clone(),
        Working {
            venue_order_id: venue.clone(),
            order_qty: qty,
            cum_qty: 0,
            price: px,
            side: side.clone(),
            canceled: false,
        },
    );
    let qty_s = qty.to_string();
    let px_s = px.to_string();
    let leaves_s = qty.to_string();
    send_acceptor(
        stream,
        config,
        next_out,
        outbound,
        "8",
        &[
            ("37", venue.as_str()),
            ("11", cl_ord.as_str()),
            ("150", "0"),
            ("39", "0"),
            ("54", side.as_str()),
            ("14", "0"),
            ("151", leaves_s.as_str()),
            ("32", "0"),
            ("31", "0"),
        ],
    )?;
    if config.auto_fill {
        let exec = format!("FIX-E{next_exec}");
        *next_exec = next_exec.saturating_add(1);
        if let Some(w) = working.get_mut(&cl_ord) {
            w.cum_qty = qty;
        }
        send_acceptor(
            stream,
            config,
            next_out,
            outbound,
            "8",
            &[
                ("37", venue.as_str()),
                ("11", cl_ord.as_str()),
                ("17", exec.as_str()),
                ("150", "F"),
                ("39", "2"),
                ("54", side.as_str()),
                ("14", qty_s.as_str()),
                ("151", "0"),
                ("32", qty_s.as_str()),
                ("31", px_s.as_str()),
            ],
        )?;
    }
    Ok(())
}

fn handle_cancel(
    stream: &mut TcpStream,
    config: &FixConfig,
    next_out: &mut u64,
    outbound: &mut Vec<(u64, String)>,
    working: &mut HashMap<String, Working>,
    map: &std::collections::BTreeMap<String, String>,
) -> Result<(), ExecutionError> {
    let orig = map.get("41").cloned().unwrap_or_default();
    let Some(row) = working.get_mut(&orig) else {
        return Ok(());
    };
    let leaves = row.order_qty - row.cum_qty;
    let venue = row.venue_order_id.clone();
    let side = row.side.clone();
    let cum = row.cum_qty.to_string();
    let px = row.price.to_string();
    if leaves <= 0 {
        return send_acceptor(
            stream,
            config,
            next_out,
            outbound,
            "8",
            &[
                ("37", venue.as_str()),
                ("11", orig.as_str()),
                ("150", "8"),
                ("39", "8"),
                ("54", side.as_str()),
                ("14", cum.as_str()),
                ("151", "0"),
                ("32", "0"),
                ("31", "0"),
                ("58", "already filled"),
            ],
        );
    }
    row.canceled = true;
    let leaves_s = leaves.to_string();
    send_acceptor(
        stream,
        config,
        next_out,
        outbound,
        "8",
        &[
            ("37", venue.as_str()),
            ("11", orig.as_str()),
            ("150", "4"),
            ("39", "4"),
            ("54", side.as_str()),
            ("14", cum.as_str()),
            ("151", "0"),
            ("32", leaves_s.as_str()),
            ("31", px.as_str()),
        ],
    )
}

fn handle_replace(
    stream: &mut TcpStream,
    config: &FixConfig,
    next_out: &mut u64,
    outbound: &mut Vec<(u64, String)>,
    working: &mut HashMap<String, Working>,
    map: &std::collections::BTreeMap<String, String>,
) -> Result<(), ExecutionError> {
    let orig = map.get("41").cloned().unwrap_or_default();
    let new_qty: i64 = map.get("38").and_then(|s| s.parse().ok()).unwrap_or(0);
    let new_px: i64 = map.get("44").and_then(|s| s.parse().ok()).unwrap_or(0);
    let Some(row) = working.get_mut(&orig) else {
        return Ok(());
    };
    if row.canceled || new_qty < row.cum_qty || new_qty <= 0 || new_px <= 0 {
        return Ok(());
    }
    row.order_qty = new_qty;
    row.price = new_px;
    let leaves = new_qty - row.cum_qty;
    let venue = row.venue_order_id.clone();
    let side = row.side.clone();
    let cum = row.cum_qty.to_string();
    let leaves_s = leaves.to_string();
    let px_s = new_px.to_string();
    send_acceptor(
        stream,
        config,
        next_out,
        outbound,
        "8",
        &[
            ("37", venue.as_str()),
            ("11", orig.as_str()),
            ("150", "5"),
            ("39", "1"),
            ("54", side.as_str()),
            ("14", cum.as_str()),
            ("151", leaves_s.as_str()),
            ("32", "0"),
            ("31", px_s.as_str()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::encode_fix;

    #[test]
    fn frame_round_trip_tcp_loopback_acceptor() {
        let cfg = FixConfig::local_mock();
        let (addr, _guard) = spawn_paper_acceptor("127.0.0.1:0", cfg.clone()).expect("bind");
        // Retry connect — acceptor thread may need a moment.
        let mut stream = None;
        for _ in 0..50 {
            if let Ok(s) = connect_fix(&addr, Duration::from_millis(200)) {
                stream = Some(s);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let mut stream = stream.expect("connect");
        let logon = encode_fix(
            &cfg.sender_comp_id,
            &cfg.target_comp_id,
            "A",
            1,
            &[("98", "0"), ("108", "30")],
        );
        write_fix_message(&mut stream, &logon).expect("logon write");
        let ack = read_fix_message(&mut stream).expect("logon ack");
        let map = decode_fix(&ack).expect("decode");
        assert_eq!(map.get("35").map(String::as_str), Some("A"));

        let nos = encode_fix(
            &cfg.sender_comp_id,
            &cfg.target_comp_id,
            "D",
            2,
            &[
                ("11", "42"),
                ("55", "1"),
                ("54", "1"),
                ("38", "3"),
                ("40", "2"),
                ("44", "10000"),
                ("59", "1"),
            ],
        );
        write_fix_message(&mut stream, &nos).expect("nos");
        let er_new = read_fix_message(&mut stream).expect("er new");
        let m1 = decode_fix(&er_new).expect("d1");
        assert_eq!(m1.get("150").map(String::as_str), Some("0"));
        let er_fill = read_fix_message(&mut stream).expect("er fill");
        let m2 = decode_fix(&er_fill).expect("d2");
        assert_eq!(m2.get("150").map(String::as_str), Some("F"));
        assert_eq!(m2.get("32").map(String::as_str), Some("3"));

        // ResendRequest for the logon ack (seq 1) must return PossDupFlag=Y.
        let resend = encode_fix(
            &cfg.sender_comp_id,
            &cfg.target_comp_id,
            "2",
            3,
            &[("7", "1"), ("16", "1")],
        );
        write_fix_message(&mut stream, &resend).expect("resend write");
        let replay = read_fix_message(&mut stream).expect("poss dup");
        let mr = decode_fix(&replay).expect("decode replay");
        assert_eq!(mr.get("35").map(String::as_str), Some("A"));
        assert_eq!(mr.get("43").map(String::as_str), Some("Y"));
        assert_eq!(mr.get("34").map(String::as_str), Some("1"));
    }
}
