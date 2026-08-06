//! Disk-backed durable log — turning "recovery logic exists" into "recovery persists" (§14a, §16c-S1).
//!
//! The authoritative session log is append-only (§3). This is its on-disk form: a single file to
//! which each event is appended, length-framed, encoded through the SHARED cadenza-ast canonical codec
//! ([`crate::event_ast`] over `cadenza_ast::codec`, §19a/§S5 — the language-native wire format, not a
//! bespoke one). Recovery reads the file back into the event vector that
//! [`crate::kernel::Session::replay`] reconstructs state from.
//!
//! **Durability shape (S1):** append is `write(frame) + flush`, and the durable-dispatch invariant
//! requires the `Dispatched` frame to be persisted before its effect is routed — so the kernel driver
//! calls `append` (which flushes) before handing the effect to an executor. A crash between the two
//! leaves a `Dispatched` with no result on disk: exactly the open obligation recovery re-drives.
//!
//! **Torn-write tolerance (totality, §17):** a crash mid-append can leave a partial final frame. The
//! reader stops cleanly at the first frame it can't fully read (short length prefix, or a short body)
//! and returns every whole event before it — the log survives a torn tail instead of refusing to open.
//! A frame that is *complete on disk but internally corrupt* — a full-length body the codec rejects — is
//! NOT a torn tail: it's surfaced as `RecoveryKind::Corrupt` (see below), since that's real corruption,
//! not an interrupted write.
//!
//! Framing: each record is `u32 little-endian length` followed by that many bytes of
//! `event_ast::encode` output. The frame is kept because the shared codec is CONSUME-WHOLE (it decodes
//! an entire slice, reporting no bytes-consumed), so the frame layer — not the codec — delimits records
//! in the concatenated stream. The length prefix lets the reader know a frame's extent before trusting
//! its contents.

use crate::event::Event;
use crate::event_ast;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The write-through SINK a [`crate::kernel::Session`] persists appended events to (§16c-S1). Abstracted
/// as a trait — like the crate's other host backends ([`crate::executor::Executor`],
/// [`crate::authz::Authorize`], [`crate::blob::BlobStore`]) — so the durable log is swappable ([`LogStore`]
/// for disk today; a network/replicated sink later) AND so a persist FAILURE can be exercised in tests
/// (a `LogStore` over a real file can't be made to fail its append deterministically — a read-only file
/// fails at open, not append — so the S1 route-guard's "don't route on an un-durable dispatch" path is
/// otherwise untestable; a test impl that returns `Err` closes that gap).
/// **Async** (operator ruling: the storage backends are async — a `LogSink` may be a network/replicated
/// log whose append `.await`s I/O; the disk [`LogStore`] just returns a ready future). Object-safe via
/// `#[async_trait(?Send)]` (`?Send` to match the single-threaded kernel + the crate's other backend traits
/// [`crate::blob::BlobStore`]/[`crate::executor::Executor`]).
#[async_trait::async_trait(?Send)]
pub trait LogSink {
    /// Persist one event durably (append + flush/fsync as the impl's tier requires). `Err` = the event
    /// did NOT reach stable storage — the caller (`Session`) latches it and refuses to route (§16c-S1).
    async fn append(&mut self, event: &Event) -> io::Result<()>;
}

/// A durable append-only event log backed by a single file.
pub struct LogStore {
    path: PathBuf,
    file: File,
}

#[async_trait::async_trait(?Send)]
impl LogSink for LogStore {
    /// The disk backend is synchronous under the hood (std `File` write+flush); it satisfies the async
    /// trait by returning a ready future (no `.await` — an in-process file append never yields).
    async fn append(&mut self, event: &Event) -> io::Result<()> {
        LogStore::append(self, event)
    }
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
    /// Byte length of the good prefix — the offset just past the last whole, well-formed frame. On a
    /// `Clean` end this equals the file length; on `TornTail`/`Corrupt` it's where the garbage begins.
    /// This is what a driver truncates the log to ([`LogStore::truncate_to`]) so the torn/corrupt tail
    /// doesn't poison future appends (an append lands AFTER the garbage, and the next recovery would
    /// otherwise stop at the stale bad frame and silently drop everything appended since).
    pub good_prefix_len: u64,
}

impl Recovered {
    /// Did the read end in genuine corruption (an alarm-worthy state, vs. a clean/torn end)?
    pub fn is_corrupt(&self) -> bool {
        self.kind == RecoveryKind::Corrupt
    }

    /// Is there a torn/corrupt tail past the good prefix that a driver should truncate before appending?
    /// (True on `TornTail`/`Corrupt`; false on a `Clean` end.)
    pub fn has_trailing_garbage(&self) -> bool {
        self.kind != RecoveryKind::Clean
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
        // Encode through the SHARED cadenza-ast canonical codec (§19a/§S5) — the language-native wire
        // format, not a bespoke one. The u32 length frame stays (cadenza-ast's decode is consume-whole,
        // no bytes-consumed — v-syntax confirmed — so the frame layer, not the codec, delimits records).
        let body = event_ast::encode(event);
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

    /// Truncate the log file to `len` bytes — the heal for a torn/corrupt tail (see [`crate::log_store::Recovered`]).
    /// After a crash-mid-append, the file carries trailing garbage past the last whole frame; appending
    /// over it would land a good frame AFTER the garbage, and the next recovery would stop at the stale
    /// bad frame and silently drop everything appended since. A driver that recovers with
    /// `has_trailing_garbage()` truncates to `good_prefix_len` FIRST, then resumes appending cleanly.
    ///
    /// Pass the recovery's `good_prefix_len`. Truncating to a length ≥ the current file size is a no-op
    /// (never extends). fsyncs so the truncation is DURABLE before the next append — this is the heal
    /// path, so a crash right after must not resurrect the garbage tail.
    pub fn truncate_to(&mut self, len: u64) -> io::Result<()> {
        let current = self.file.metadata()?.len();
        if len < current {
            self.file.set_len(len)?;
            // sync_all (fsync), NOT flush(): `File::flush` is a NO-OP (std File is unbuffered), so it
            // gave no durability at all (Copilot PR#1018) — a crash after the heal could leave the
            // garbage tail on disk and the next recovery would re-see it. The truncation must reach
            // stable storage before we resume appending.
            self.file.sync_all()?;
        }
        Ok(())
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
                    good_prefix_len: 0,
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
        // Need the 4-byte length prefix. `pos` is always the offset just past the last WHOLE frame —
        // the good-prefix length returned in every branch so a driver can truncate a bad tail to it.
        let Some(len_bytes) = bytes.get(pos..pos + 4) else {
            // Fewer than 4 trailing bytes: a torn tail iff there are leftover bytes, else a clean end.
            let kind = if pos < bytes.len() {
                RecoveryKind::TornTail
            } else {
                RecoveryKind::Clean
            };
            return Recovered {
                events,
                kind,
                good_prefix_len: pos as u64,
            };
        };
        let len =
            u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
        let body_start = pos + 4;
        let Some(body) = bytes.get(body_start..body_start + len) else {
            // Length prefix present but body truncated: a torn final frame — stop cleanly.
            return Recovered {
                events,
                kind: RecoveryKind::TornTail,
                good_prefix_len: pos as u64,
            };
        };
        // The frame is EXACTLY `len` bytes (sliced above), and the shared codec is consume-whole — so a
        // complete frame either decodes as one whole event or is corrupt. TRUNCATION is caught by the
        // FRAME layer (a short body → TornTail, above); any decode failure HERE is on a fully-present
        // frame → genuine corruption (incl. cadenza-ast's own `Truncated`, which inside a complete frame
        // means the frame's length prefix over-claimed — still corruption, not an interrupted append).
        match event_ast::decode(body) {
            Ok(event) => {
                events.push(event);
                pos = body_start + len;
            }
            Err(_) => {
                // Complete-but-invalid frame: genuine corruption, not a torn write. `Corrupt` (distinct
                // from Clean/TornTail) so the driver can alarm / hard-fail rather than mistake data loss
                // for a clean log end (PR#990 #4). `pos` marks where the good prefix ends (before this
                // bad frame) so the driver can truncate to it if it chooses to heal + proceed.
                return Recovered {
                    events,
                    kind: RecoveryKind::Corrupt,
                    good_prefix_len: pos as u64,
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
                spawn_nonce: Hash::of(b"test-spawn-nonce"),
                parent: None,
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
                payload: Payload::Inline(vec![seq as u8].into()),
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
                family: crate::effect::EffectKind::Http.family().into(),
                target: "https://ok.host/x".into(),
                idempotency_key: Hash::of(b"k"),
                deadline_ms: None,
                token: None,
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
    fn truncate_to_heals_a_torn_tail_so_future_appends_survive_recovery() {
        // The bug this guards: after a torn-tail crash, appending over the garbage lands a good frame
        // AFTER the torn bytes; the next recovery stops at the torn frame and silently drops everything
        // appended since. truncate_to(good_prefix_len) heals it — the driver's recover-then-heal flow.
        let path = temp_path("heal");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
            s.append(&inbound(1)).unwrap();
        }
        // A crash mid-append leaves a torn final frame.
        {
            use std::io::Write;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&100u32.to_le_bytes()).unwrap(); // claims 100 bytes...
            f.write_all(&[1, 2, 3]).unwrap(); // ...only 3 present
        }

        // Recover: two whole events + a torn tail, and good_prefix_len points past the last whole frame.
        let rec = LogStore::recover(&path).unwrap();
        assert_eq!(rec.events, vec![genesis(), inbound(1)]);
        assert_eq!(rec.kind, RecoveryKind::TornTail);
        assert!(rec.has_trailing_garbage());

        // Heal: truncate to the good prefix, then append a NEW event and close.
        {
            let mut s = LogStore::open(&path).unwrap();
            s.truncate_to(rec.good_prefix_len).unwrap();
            s.append(&inbound(2)).unwrap();
        }

        // The new append survived recovery — a CLEAN log of all three events (torn bytes gone). Before
        // the heal this recovery would have stopped at the torn frame and never seen inbound(2).
        let rec2 = LogStore::recover(&path).unwrap();
        assert_eq!(rec2.kind, RecoveryKind::Clean);
        assert_eq!(rec2.events, vec![genesis(), inbound(1), inbound(2)]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncate_to_at_or_beyond_len_is_a_noop() {
        // Never extends / never drops good data: truncating to >= file size leaves the log intact.
        let path = temp_path("noop-trunc");
        {
            let mut s = LogStore::open(&path).unwrap();
            s.append(&genesis()).unwrap();
        }
        let before = LogStore::recover(&path).unwrap();
        assert_eq!(before.kind, RecoveryKind::Clean);
        {
            let mut s = LogStore::open(&path).unwrap();
            // A clean log's good_prefix_len == file length → truncate_to is a no-op.
            s.truncate_to(before.good_prefix_len).unwrap();
            s.truncate_to(u64::MAX).unwrap(); // beyond EOF → also a no-op
        }
        let after = LogStore::recover(&path).unwrap();
        assert_eq!(after.events, vec![genesis()]);
        assert_eq!(after.kind, RecoveryKind::Clean);
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
