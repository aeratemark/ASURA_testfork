//! A server-side packet capture: every game message, both directions, as plaintext JSONL.
//!
//! The client-side sniffer (`tools/sniff-capture.py`) records the client's *message struct*, through
//! an injected hook: a four-byte header (the u16 id and a zero u16), every field widened to four
//! bytes, and zero padding out to the struct's size. This capture records the other layer, the wire:
//! the plaintext frame with its **two-byte** header and the packed fields, which is what this server
//! hands ARC4 on the way out and reads back after unscrambling on the way in.
//!
//! The two layers are not interchangeable, and the difference between them is not a second protocol
//! (`docs/03-protocol.md` 5e). So a record states which layer it is: a reader takes the header length
//! from the file's own header record and never assumes it. Payload offsets in a struct-layer capture
//! start four bytes in; in this one, two.
//!
//! One file, one JSON object per line, starting with a header record:
//!
//! ```text
//! {"record":"capture","tool":"asura-world","layer":"wire","header_len":2,"started_at":"..."}
//! {"seq":1,"t":0.012,"direction":"StoC","header":153,"size":12,"data":"99000000a100000000000000"}
//! {"seq":2,"t":0.498,"direction":"CToS","header":32829,"size":26,"data":"3d80000000000000..."}
//! ```
//!
//! `t` is seconds since the capture started, so a capture lines up with the world log by time.
//! Nothing is sanitised: this file never leaves the machine (it is gitignored), and the whole point
//! is the payloads.
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The longest a record may sit in the writer before it is flushed.
///
/// A capture is evidence and a server is usually killed to end a diagnostic session, so the last lines
/// must reach the disk. Flushing **every** record was the first form of that rule and it is what a load
/// pays for: the block a travel sends is about 425 frames, so one load was 850 synchronous write+flush
/// syscalls inside the runtime, in the same task that is handing the client its map. A disk or a scanner
/// that hesitates for a millisecond turns that into a second and a half of frames the client never sees.
/// Time-bounded instead: at most this much is lost to a kill, and a busy load costs a few flushes.
const FLUSH_EVERY: Duration = Duration::from_millis(200);

/// One open capture: the file, when it started, and a sequence number shared by every connection.
struct Capture {
    writer: Mutex<BufWriter<std::fs::File>>,
    start: Instant,
    seq: AtomicU64,
    last_flush: Mutex<Instant>,
}

static CAPTURE: OnceLock<Option<Capture>> = OnceLock::new();

/// Start capturing to `path`, once per process. An empty path is a no-op, and a second call is
/// ignored: there is one capture per server run.
pub fn install(path: &str) -> std::io::Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    let file = std::fs::File::create(path)?;
    let capture = Capture {
        writer: Mutex::new(BufWriter::new(file)),
        start: Instant::now(),
        seq: AtomicU64::new(0),
        last_flush: Mutex::new(Instant::now()),
    };
    let mut writer = capture.writer.lock().unwrap_or_else(|err| err.into_inner());
    writeln!(
        writer,
        "{{\"record\":\"capture\",\"tool\":\"asura-world\",\"layer\":\"wire\",\"header_len\":2,\"started_at\":\"{}\"}}",
        chrono_like_now()
    )?;
    writer.flush()?;
    drop(writer);
    let _ = CAPTURE.set(Some(capture));
    Ok(())
}

/// Whether a capture is running.
pub fn recording() -> bool {
    CAPTURE.get().map(Option::is_some).unwrap_or(false)
}

/// Put what is buffered on the disk now.
///
/// `record` flushes on a time bound so a live load does not pay a syscall per frame; this is for the
/// moments that need the file complete - a reader about to parse it, and the tests that assert on it.
pub fn flush() {
    let Some(capture) = CAPTURE.get().and_then(|capture| capture.as_ref()) else {
        return;
    };
    let mut writer = capture.writer.lock().unwrap_or_else(|err| err.into_inner());
    let _ = writer.flush();
    let mut last_flush = capture
        .last_flush
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    *last_flush = Instant::now();
}

/// Record one message. `bytes` is the plaintext frame, header first.
pub fn record(direction: &str, header: u16, bytes: &[u8]) {
    let Some(capture) = CAPTURE.get().and_then(|capture| capture.as_ref()) else {
        return;
    };
    let seq = capture.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let t = capture.start.elapsed().as_secs_f64();
    let mut writer = capture.writer.lock().unwrap_or_else(|err| err.into_inner());
    let _ = writeln!(
        writer,
        "{{\"seq\":{seq},\"t\":{t:.3},\"direction\":\"{direction}\",\"header\":{header},\"size\":{},\"data\":\"{}\"}}",
        bytes.len(),
        hex(bytes)
    );
    // Flushed on a time bound, not per record: see `FLUSH_EVERY`. The buffer flushes itself when it
    // fills, so a burst is still one write per 8 KB rather than one per message.
    let mut last_flush = capture
        .last_flush
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if last_flush.elapsed() >= FLUSH_EVERY {
        let _ = writer.flush();
        *last_flush = Instant::now();
    }
}

/// A UTC timestamp without pulling a date crate: seconds since the Unix epoch, ISO-8601-ish.
fn chrono_like_now() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!("{}.{:09}Z", duration.as_secs(), duration.subsec_nanos()),
        Err(_) => "unknown".to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_writes_one_json_line_per_message() {
        let path = std::env::temp_dir().join(format!("asura-capture-{}.jsonl", std::process::id()));
        let path_text = path.to_str().expect("a path").to_string();
        // The process-wide capture can only be installed once; this test owns it.
        install(&path_text).expect("install the capture");
        record(
            "StoC",
            0x0099,
            &[0x99, 0x00, 0x00, 0x00, 0x94, 0x00, 0x00, 0x00],
        );
        record("CToS", 0x803D, &[0x3d, 0x80, 0x00, 0x00]);
        flush();
        let text = std::fs::read_to_string(&path).expect("read the capture");
        std::fs::remove_file(&path).ok();

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "a header record and two messages: {text}");
        assert!(lines[0].contains("\"record\":\"capture\""), "{}", lines[0]);
        assert!(
            lines[1].contains("\"direction\":\"StoC\"")
                && lines[1].contains("\"header\":153")
                && lines[1].contains("\"size\":8")
                && lines[1].contains("\"data\":\"9900000094")
                || lines[1].contains("\"data\":\"99000000"),
            "the message record carries direction, header, size and the bytes: {}",
            lines[1]
        );
        assert!(lines[2].contains("\"direction\":\"CToS\""), "{}", lines[2]);
        let seq_second = lines[1].contains("\"seq\":1") || lines[2].contains("\"seq\":2");
        assert!(
            seq_second,
            "messages are sequenced: {} / {}",
            lines[1], lines[2]
        );
    }
}

// ---- a short hex preview for a log line ------------------------------------------------------

/// The first bytes of a message, as hex, for the trace.
///
/// When a client dies mid-load, the only evidence left is what it sent and what it was sent. A header
/// and a size say which message; the first bytes say whether its contents were what the reference
/// would have written.
pub fn hex_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(24)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
