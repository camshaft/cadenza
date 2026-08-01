//! Disk-backed durable log — turning "recovery logic exists" into "recovery persists" (§14a, §16c-S1).
//!
//! The authoritative session log is append-only (§3). This is its on-disk form: a single file to
//! which each event is appended, length-framed, using the frozen event codec ([`Event::encode`] /
//! [`Event::decode`]). Recovery reads the file back into the event vector that [`crate::kernel::Session::replay`]
//! reconstructs state from.
//!
//! **Durability shape (S1):** append is `write(frame) + flush`, and the durable-dispatch invariant
//! requires the `Dispatched` frame to be persisted before its effect is routed — so the kernel driver
//! calls `append` (which flushes) before handing the effect to an executor. A crash between the two
//! leaves a `Dispatched` with no result on disk: exactly the open obligation recovery re-drives.
//!
//! **Torn-write tolerance (totality, §17):** a crash mid-append can leave a partial final frame. The
//! reader stops cleanly at the first frame it can't fully read (short length prefix, short body, or a
//! body the codec rejects) and returns every whole event before it — the log survives a torn tail
//! instead of refusing to open. A frame that is *complete on disk but internally corrupt* (not just
//! truncated) is surfaced as an error, since that's real corruption, not an interrupted write.
//!
//! Framing: each record is `u32 little-endian length` followed by that many bytes of `Event::encode`
//! output. The length prefix lets the reader know a frame's extent before trusting its contents.

use crate::event::Event;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// A durable append-only event log backed by a single file.
pub struct LogStore {
    path: PathBuf,
    file: File,
}

/// How a recovery read ended — the three tail states are mutually exclusive, so an ENUM type-enforces
/// that (PR#993 #4 / operator r3695380903 — was two `mutually exclusive` bools). The driver reacts to
/// this: `Clean`/`TornTail` are normal (a torn tail is a benign crash-mid-append), `Corrupt` is an
/// alarm (possible data loss — a fully-present frame that didn't decode, NOT a clean end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryKind {
    /// The file ended exactly on a frame boundary — a clean log end.
    Clean,
    /// The file ended with a partial/torn final frame that was skipped (crash mid-append — benign).
    TornTail,
    /// A COMPLETE frame failed to decode — genuine corruption, not a clean EOF or torn write.
    Corrupt,
}

/// What a recovery read found: the good prefix of whole events, plus how the read ended.
#[derive(Debug)]
pub struct Recovered {
    /// Every whole, well-formed event read in order (the good prefix, in all three [`RecoveryKind`]s).
    pub events: Vec<Event>,
    /// How the read ended (clean / torn tail / corrupt).
    pub kind: RecoveryKind,
}

impl Recovered {
    /// Did the read end in genuine corruption (an alarm-worthy state, vs. a clean/torn end)?
    pub fn is_corrupt(&self) -> bool {
        self.kind == RecoveryKind::Corrupt
    }
}

impl LogStore {
    /// Open (creating if absent) the log file for appending. Existing content is preserved; use
    /// [`LogStore::recover`] first to read it back.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        Ok(LogStore { path, file })
    }

    /// Append one event, length-framed, and flush to the OS. Returns once the bytes are handed to the
    /// OS (a stronger fsync is a later durability-tier knob; append+flush is the v0 contract).
    ///
    /// The S1 ordering contract lives in the *caller*: append a `Dispatched` event here, and only
    /// after this returns route the effect to an executor.
    pub fn append(&mut self, event: &Event) -> io::Result<()> {
        let body = event.encode();
        let len = u32::try_from(body.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "event too large to frame"))?;
        // One write of a contiguous buffer (len prefix + body) so a frame is as atomic as the OS
        // makes a single write — minimizing the torn-frame window.
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&body);
        self.file.write_all(&frame)?;
        self.file.flush()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the whole log back into events, tolerating a torn final frame (see module docs). Opens the
    /// file independently so it works before/without an appending handle.
    pub fn recover(path: impl AsRef<Path>) -> io::Result<Recovered> {
        let mut bytes = Vec::new();
        match File::open(path.as_ref()) {
            Ok(mut f) => {
                f.read_to_end(&mut bytes)?;
            }
            // No file yet = an empty log, not an error.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Recovered {
                    events: Vec::new(),
                    kind: RecoveryKind::Clean,
                });
            }
            Err(e) => return Err(e),
        }
        Ok(decode_frames(&bytes))
    }
}

/// Decode a framed byte stream into events, stopping cleanly at a torn tail. Total: never panics.
fn decode_frames(bytes: &[u8]) -> Recovered {
    let mut events = Vec::new();
    let mut pos = 0usize;
    loop {
        // Need the 4-byte length prefix.
        let Some(len_bytes) = bytes.get(pos..pos + 4) else {
            // Fewer than 4 trailing bytes: a torn tail iff there are leftover bytes, else a clean end.
            let kind = if pos < bytes.len() {
                RecoveryKind::TornTail
            } else {
                RecoveryKind::Clean
            };
            return Recovered { events, kind };
        };
        let len =
            u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        let body_start = pos + 4;
        let Some(body) = bytes.get(body_start..body_start + len) else {
            // Length prefix present but body truncated: a torn final frame — stop cleanly.
            return Recovered {
                events,
                kind: RecoveryKind::TornTail,
            };
        };
        match Event::decode(body) {
            // The frame length told us the body's extent; decode must consume exactly that. A frame
            // that is fully present but doesn't decode (or under/over-consumes) is real corruption.
            Ok((event, consumed)) if consumed == len => {
                events.push(event);
                pos = body_start + len;
            }
            _ => {
                // Complete-but-invalid frame: genuine corruption, not a torn write. `Corrupt` (distinct
                // from Clean/TornTail) so the driver can alarm / hard-fail rather than mistake data loss
                // for a clean log end (PR#990 #4).
                return Recovered {
                    events,
                    kind: RecoveryKind::Corrupt,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectId, Payload};
    use crate::event::{ContentType, EventBody};
    use crate::hash::Hash;
    use std::path::PathBuf;

    /// A unique temp path per test (no external tempfile dep). Uses the process id + a per-test tag.
    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("cdz-kernel-log-{}-{}.log", std::process::id(), tag));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn genesis() -> Event {
        Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer: Hash::of(b"r"),
            },
        }
    }

    fn inbound(seq: u64) -> Event {
        Event {
            seq,
            cause: None,
            body: EventBody::Inbound {
                content_type: ContentType {
                    family: "m".into(),
                    version: 1,
                },
                payload: Payload::Inline(vec![seq as u8]),
            },
        }
    }

    fn dispatched(seq: u64) -> Event {
        Event {
            seq,
            cause: None,
            body: EventBody::Dispatched {
                id: EffectId(seq),
                kind: crate::effect::EffectKind::Http,
                target: "https://ok.host/x".into(),
                idempotency_key: Hash::of(b"k"),
                deadline_ms: None,
            },
        }
    }

    #[test]
    fn append_then_recover_round_trips_in_order() {
        let path = temp_path("roundtrip");
        {
            let mut store = LogStore::open(&path).unwrap();
            store.append(&genesis()).unwrap();
            store.append(&inbound(1)).unwrap();
            store.append(&dispatched(2)).unwrap();
        }
        let rec = LogStore::recover(&path).unwrap();
        assert_eq!(rec.kind, RecoveryKind::Clean);
        assert_eq!(rec.events, vec![genesis(), inbound(1), dispatched(2)]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recover_of_missing_file_is_empty_not_error() {
        let path = temp_path("missing");
        let rec = LogStore::recover(&path).unwrap();
        assert!(rec.events.is_empty());
        assert_eq!(rec.kind, RecoveryKind::Clean);
    }

    #[test]
    fn reopen_appends_do_not_clobber() {
        let path = temp_path("reopen");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
        }
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&inbound(1)).unwrap();
        }
        let rec = LogStore::recover(&path).unwrap();
        assert_eq!(rec.events, vec![genesis(), inbound(1)]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn torn_tail_is_tolerated_and_flagged() {
        // Write two whole events, then append a partial third frame (a crash mid-append).
        let path = temp_path("torn");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
            s.append(&inbound(1)).unwrap();
        }
        // Manually append a truncated frame: a length prefix claiming 100 bytes but only 3 present.
        {
            use std::io::Write;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&100u32.to_le_bytes()).unwrap();
            f.write_all(&[1, 2, 3]).unwrap();
        }
        let rec = LogStore::recover(&path).unwrap();
        // The two whole events survive; the torn tail is skipped and flagged.
        assert_eq!(rec.events, vec![genesis(), inbound(1)]);
        assert_eq!(rec.kind, RecoveryKind::TornTail);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn short_length_prefix_tail_is_torn_not_corrupt() {
        let path = temp_path("shortprefix");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
        }
        {
            use std::io::Write;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&[0xAB, 0xCD]).unwrap(); // 2 bytes — not even a full length prefix
        }
        let rec = LogStore::recover(&path).unwrap();
        assert_eq!(rec.events, vec![genesis()]);
        assert_eq!(rec.kind, RecoveryKind::TornTail);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn complete_but_corrupt_frame_is_reported_as_corruption_not_torn() {
        // A full frame (length matches body length) whose body is not a valid event → corruption,
        // distinct from a torn tail so the driver can hard-fail if it wants.
        let path = temp_path("corrupt");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
        }
        {
            use std::io::Write;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            let garbage = [0xFFu8; 5]; // 5 bytes, not a decodable event
            f.write_all(&(garbage.len() as u32).to_le_bytes()).unwrap();
            f.write_all(&garbage).unwrap();
        }
        let rec = LogStore::recover(&path).unwrap();
        assert_eq!(rec.events, vec![genesis()]); // the good prefix is preserved
                                                 // PR#990 #4 / #993 #4: corruption is DISTINGUISHABLE from a clean EOF — a dedicated enum variant.
        assert_eq!(
            rec.kind,
            RecoveryKind::Corrupt,
            "a complete-but-invalid frame must be Corrupt (not look like a clean EOF or torn tail)"
        );
        assert!(rec.is_corrupt());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clean_and_torn_tails_are_not_flagged_corrupt() {
        // The three tail states are distinct: a clean log is Clean (neither torn nor corrupt).
        let path = temp_path("clean-not-corrupt");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
            s.append(&inbound(1)).unwrap();
        }
        let rec = LogStore::recover(&path).unwrap();
        assert_eq!(rec.kind, RecoveryKind::Clean);
        assert!(!rec.is_corrupt(), "a clean log is not corrupt");
        let _ = std::fs::remove_file(&path);
    }
}
