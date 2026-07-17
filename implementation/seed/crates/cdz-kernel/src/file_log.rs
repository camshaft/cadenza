//! A deterministic file-backed [`Log`] (agent-runtime L1a).
//!
//! Events are appended to a single file as length-prefixed records:
//! `kind_len: u32-LE | kind bytes (UTF-8) | payload_len: u32-LE | payload bytes`.
//! The on-disk record ORDER is the seq order (seq = the record's 0-based index), so a fresh process
//! re-reads the identical sequence — the property the replay/re-fold thesis (vision §2.3, §3) needs. No
//! network, no async: this is the CI-safe stand-in for the DynamoDB log (L1d) while the fold owner (L1b)
//! and the replay-determinism gate (L1c) are built against the [`Log`] trait.
//!
//! CONCURRENCY CONTRACT: `FileLog` is SINGLE-PROCESS / externally-synchronized. An `append` is one
//! `write_all`, not a cross-process-atomic write, so a reader in another process tailing concurrently
//! could observe a partial trailing record (`read_all` then errors "truncated log"). The L1 fold owner is
//! single-threaded and the sole reader+writer, so it never races itself — that is what makes the file log
//! sound here. The real MULTI-WRITER ordering authority (vision §2.1: many writers append concurrently) is
//! DynamoDB at L1d, whose conditional write is the actual concurrency primitive; this file log deliberately
//! does not reimplement it.

use crate::{Event, Log, Seq};
use anyhow::{anyhow, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A [`Log`] backed by one append-only file at `path`.
pub struct FileLog {
    path: PathBuf,
    /// The next seq to assign = the count of records already in the file. Cached so `append` doesn't
    /// re-scan the file each call; seeded by a scan at `open` (so it is correct across process restarts).
    next_seq: Seq,
}

impl FileLog {
    /// Open (creating if absent) the log file at `path`, scanning any existing records so appends continue
    /// the sequence rather than overwrite it (a restarted owner re-attaches to the same log).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        // Create the file if it doesn't exist; its record count seeds next_seq.
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open log file {}", path.display()))?;
        let existing = read_all(&path)?;
        Ok(Self {
            next_seq: existing.len() as Seq,
            path,
        })
    }
}

impl Log for FileLog {
    fn append(&mut self, kind: &str, payload: &[u8]) -> Result<Seq> {
        // A kind/payload whose length can't be a u32 is a programming error, not a runtime input — bail
        // clearly rather than silently truncate the length prefix.
        let kind_bytes = kind.as_bytes();
        let kind_len: u32 = kind_bytes
            .len()
            .try_into()
            .map_err(|_| anyhow!("event kind too long ({} bytes) to encode", kind_bytes.len()))?;
        let payload_len: u32 = payload
            .len()
            .try_into()
            .map_err(|_| anyhow!("event payload too long ({} bytes) to encode", payload.len()))?;

        let mut record = Vec::with_capacity(8 + kind_bytes.len() + payload.len());
        record.extend_from_slice(&kind_len.to_le_bytes());
        record.extend_from_slice(kind_bytes);
        record.extend_from_slice(&payload_len.to_le_bytes());
        record.extend_from_slice(payload);

        // Assemble the whole record first, then one `write_all` in append mode. NOTE: this is NOT a
        // cross-process atomicity guarantee — `write_all` may issue multiple syscalls, so a reader in
        // ANOTHER process/`FileLog` tailing concurrently could observe a partial trailing record (and
        // `read_all` would then error "truncated log"). `FileLog` is the SINGLE-PROCESS / externally-
        // synchronized L1a stand-in: the L1 fold owner is single-threaded and the sole reader+writer, so it
        // never races itself. The real MULTI-WRITER ordering authority is DynamoDB at L1d (vision §2.1),
        // whose conditional write is the actual concurrency primitive; this file log deliberately does not
        // reimplement that. (Assembling the full record up front still keeps each write a single call, which
        // minimizes — but does not eliminate — a torn read; the single-process contract is what makes it
        // sound here.)
        let mut f = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .with_context(|| format!("append to log file {}", self.path.display()))?;
        f.write_all(&record)
            .with_context(|| format!("write record to {}", self.path.display()))?;

        let seq = self.next_seq;
        self.next_seq += 1;
        Ok(seq)
    }

    fn tail(&self, from: Seq) -> Result<Vec<Event>> {
        let all = read_all(&self.path)?;
        Ok(all.into_iter().filter(|e| e.seq >= from).collect())
    }
}

/// Read + decode every record in the file into `Event`s (seq = index). A truncated/corrupt trailing record
/// is an error, not a silent drop — the log is the source of truth, so a partial read must be loud.
fn read_all(path: &Path) -> Result<Vec<Event>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("open log file {}", path.display()))?
        .read_to_end(&mut bytes)
        .with_context(|| format!("read log file {}", path.display()))?;

    let mut out = Vec::new();
    let mut i = 0usize;
    let mut seq: Seq = 0;
    while i < bytes.len() {
        let kind_len = read_u32(&bytes, &mut i)? as usize;
        let kind = take(&bytes, &mut i, kind_len)?;
        let kind = String::from_utf8(kind.to_vec())
            .map_err(|e| anyhow!("record {seq}: kind is not valid UTF-8: {e}"))?;
        let payload_len = read_u32(&bytes, &mut i)? as usize;
        let payload = take(&bytes, &mut i, payload_len)?.to_vec();
        out.push(Event { seq, kind, payload });
        seq += 1;
    }
    Ok(out)
}

/// Read a little-endian u32 at `*i`, advancing `*i` by 4. Errors if fewer than 4 bytes remain (a truncated
/// length prefix — a corrupt log tail).
fn read_u32(bytes: &[u8], i: &mut usize) -> Result<u32> {
    let end = *i + 4;
    let slice = bytes
        .get(*i..end)
        .ok_or_else(|| anyhow!("truncated log: expected a 4-byte length at offset {i}"))?;
    let v = u32::from_le_bytes(slice.try_into().expect("slice is exactly 4 bytes"));
    *i = end;
    Ok(v)
}

/// Borrow `len` bytes at `*i`, advancing `*i` by `len`. Errors if fewer than `len` bytes remain.
fn take<'a>(bytes: &'a [u8], i: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = *i + len;
    let slice = bytes
        .get(*i..end)
        .ok_or_else(|| anyhow!("truncated log: expected {len} bytes at offset {i}"))?;
    *i = end;
    Ok(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temp path per test (no external tempfile dep; pid + a per-call counter keeps it distinct).
    fn temp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("cdz-kernel-{tag}-{}-{n}.log", std::process::id()))
    }

    #[test]
    fn append_assigns_monotonic_gap_free_seqs_from_zero() {
        let p = temp_path("seq");
        let _ = std::fs::remove_file(&p);
        let mut log = FileLog::open(&p).unwrap();
        assert_eq!(log.append("a", b"x").unwrap(), 0);
        assert_eq!(log.append("b", b"yy").unwrap(), 1);
        assert_eq!(
            log.append("c", b"").unwrap(),
            2,
            "an empty payload still gets a seq"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn tail_round_trips_events_in_order() {
        let p = temp_path("rt");
        let _ = std::fs::remove_file(&p);
        let mut log = FileLog::open(&p).unwrap();
        log.append("model-request", b"prompt").unwrap();
        log.append("model-response", b"answer").unwrap();
        let events = log.tail(0).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            events,
            vec![
                Event {
                    seq: 0,
                    kind: "model-request".into(),
                    payload: b"prompt".to_vec()
                },
                Event {
                    seq: 1,
                    kind: "model-response".into(),
                    payload: b"answer".to_vec()
                },
            ],
            "tail returns every event, in seq order, with kind+payload intact"
        );
    }

    #[test]
    fn tail_from_a_cursor_returns_only_later_events() {
        let p = temp_path("cursor");
        let _ = std::fs::remove_file(&p);
        let mut log = FileLog::open(&p).unwrap();
        for k in 0..5 {
            log.append("e", format!("{k}").as_bytes()).unwrap();
        }
        let from2 = log.tail(2).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            from2.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(from2[0].payload, b"2");
    }

    #[test]
    fn a_reopened_log_continues_the_sequence_and_replays_identically() {
        // The replay/re-fold property in miniature: close + reopen the same file, and the SAME ordered
        // events come back, with the next append continuing the seq (a restarted owner re-attaches).
        let p = temp_path("reopen");
        let _ = std::fs::remove_file(&p);
        {
            let mut log = FileLog::open(&p).unwrap();
            log.append("x", b"1").unwrap();
            log.append("x", b"2").unwrap();
        }
        let mut reopened = FileLog::open(&p).unwrap();
        let replayed = reopened.tail(0).unwrap();
        assert_eq!(replayed.len(), 2, "reopened log replays the same events");
        assert_eq!(replayed[1].payload, b"2");
        assert_eq!(
            reopened.append("x", b"3").unwrap(),
            2,
            "the next append continues the sequence past the reloaded records"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn payload_bytes_are_opaque_and_binary_safe() {
        // The payload is arbitrary bytes (an event body a later rung encodes) — NUL, high bytes, and
        // embedded length-prefix-looking bytes must round-trip, since the record framing is length-based.
        let p = temp_path("binary");
        let _ = std::fs::remove_file(&p);
        let mut log = FileLog::open(&p).unwrap();
        let nasty = vec![0u8, 4, 0, 0, 0, 255, b'"', b'\n', 200];
        log.append("blob", &nasty).unwrap();
        let got = log.tail(0).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            got[0].payload, nasty,
            "arbitrary bytes round-trip via length-framed records"
        );
    }
}
