//! Shared test fixture: an in-memory [`crate::log_store::LogSink`] that captures the events written
//! through it, so a replay-equivalence test reads its input from the durable-log SOURCE (the sink) rather
//! than the resident log Vec — mirroring PRODUCTION recovery, which reads from `LogStore::recover`, never
//! from an in-memory Vec. This is the log-decouple I5 step-3 seam: once the resident Vec is dropped, the
//! full log is only available from the attached sink, exactly as it is on a real recovery. The fixture
//! seeds itself with the session's genesis on attach (the constructor puts genesis in the Vec, but a
//! write-through sink attached afterward must be seeded with it — precisely what the host session factory
//! does, `sink.append(&genesis)` before attaching), then captures every subsequent append.
//!
//! Crate-level (not nested in `kernel`'s test module) so BOTH the `kernel` unit tests AND the
//! `kernel_e2e_tests` suites reach it via `crate::test_log_source` — the one recording-sink seam every
//! replay/audit test reads its durable log from.

use crate::event::Event;
use crate::kernel::Session;
use std::cell::RefCell;
use std::rc::Rc;

/// The captured event buffer, shared between the attached sink and the test that reads it back. `Rc`
/// (not `Arc`) + `RefCell` — the kernel is single-threaded by design, matching the `?Send` backends.
pub(crate) type CapturedLog = Rc<RefCell<Vec<Event>>>;

struct MemLogSink {
    captured: CapturedLog,
}

#[async_trait::async_trait(?Send)]
impl crate::log_store::LogSink for MemLogSink {
    async fn append(&mut self, event: &Event) -> std::io::Result<()> {
        self.captured.borrow_mut().push(event.clone());
        Ok(())
    }
}

/// Attach a fresh recording sink to `session`, seeded with its genesis, and return the shared buffer.
/// After this, every appended event is captured; the test replays from [`replay_input`] to prove the
/// durable-log source (not the resident Vec) reconstructs the session — the recovery-equivalence the
/// Vec-drop rests on. Call this on a FRESH `genesis` session (before delivering events).
pub(crate) fn attach_recording_sink(session: &mut Session) -> CapturedLog {
    let captured: CapturedLog = Rc::new(RefCell::new(vec![session.genesis_ref().clone()]));
    session.attach_sink(Box::new(MemLogSink {
        captured: Rc::clone(&captured),
    }));
    captured
}

/// Attach a recording sink PRE-SEEDED with an existing durable log `prefix`, for the recover-then-mutate
/// pattern: a session reconstructed via [`Session::replay`] has no sink, but a test that then drives it
/// further (e.g. `time_out_effect`) wants the FULL durable log = the replayed prefix + the new appends.
/// Seed the buffer with `prefix` (the events the session was replayed from) so subsequent write-through
/// appends extend it into the complete durable log. (Unlike [`attach_recording_sink`], which seeds only
/// genesis for a fresh session.)
pub(crate) fn attach_recording_sink_seeded(
    session: &mut Session,
    prefix: Vec<Event>,
) -> CapturedLog {
    let captured: CapturedLog = Rc::new(RefCell::new(prefix));
    session.attach_sink(Box::new(MemLogSink {
        captured: Rc::clone(&captured),
    }));
    captured
}

/// The captured durable log as an owned `Vec<Event>` — what a test hands to [`Session::replay`] in
/// place of `session.log().to_vec()`. Reading from the SOURCE, not the resident Vec.
pub(crate) fn replay_input(captured: &CapturedLog) -> Vec<Event> {
    captured.borrow().clone()
}
