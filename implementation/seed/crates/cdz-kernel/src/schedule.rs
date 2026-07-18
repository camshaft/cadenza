//! Scheduled events in the log (minimal-kernel re-charter, scheduling substrate — operator ruling).
//!
//! Operator ruling (Slack, relayed with the wall-clock ask): "the log needs to have a wall clock. That's fine.
//! We also need it for one shot and periodic events as well." This module is the LOG side of scheduling — the
//! counterpart of [`crate::policy`] for capabilities: a set of reserved event kinds, appenders, and the pure
//! `due`-fold the daemon consults each tick to decide which schedules must FIRE now. Time is passed in as a
//! `now_ms` argument (read once by the daemon from [`crate::clock::now_millis`]), so every fold here stays pure
//! and deterministically testable (pass a fixed `now_ms`), exactly like [`crate::policy::effective_policy`].
//!
//! The model (three reserved kinds, all data-in-the-log):
//!   • [`SCHEDULE_CREATE`] — register a schedule: an id + an absolute first-fire time + an optional period
//!     (one-shot if absent, periodic if present) + the TRIGGER event (kind + payload) to emit when it fires. A
//!     later create with the SAME id SUPERSEDES the prior (self-superseding, same shape as `program`/`policy`).
//!   • [`SCHEDULE_CANCEL`] — payload is a schedule id; [`active_schedules`] drops it (an operator or a program
//!     cancels a periodic timer, or a one-shot before it fires).
//!   • [`SCHEDULE_FIRE`] — the daemon's OWN record that it fired occurrence N of a schedule (payload is the id).
//!     Counting these per id is how the pure fold knows the NEXT occurrence without any mutable state: a
//!     one-shot with one fire is done; a periodic with k fires is next due at `first_ms + k*period_ms`.
//!
//! This is the pure substrate slice (types + codec + appenders + the [`due`] fold) — the daemon wiring (fire
//! due schedules each tick, emitting the trigger + recording a [`SCHEDULE_FIRE`]) is a later increment, exactly
//! as [`crate::policy`] landed its log-store half before [`crate::daemon`] evaluated it.

use crate::{Log, Seq};
use anyhow::Result;

/// The event `kind` for registering a schedule — payload is a [`Schedule`] encoded by [`Schedule::encode`]. A
/// later create with the same [`Schedule::id`] supersedes the prior (the fold keeps the newest per id).
/// Reserved like `program`/`policy`: an operator/program emits it via a dedicated path, not the free-`emit` door.
pub const SCHEDULE_CREATE: &str = "schedule-create";

/// The event `kind` for cancelling a schedule — payload is the schedule id (UTF-8). [`active_schedules`] drops
/// any schedule whose id has been cancelled (a later create with that id re-registers it — cancel is not
/// permanent, it removes the currently-active schedule of that id).
pub const SCHEDULE_CANCEL: &str = "schedule-cancel";

/// The event `kind` the daemon appends when it FIRES an occurrence of a schedule — payload is the schedule id.
/// The count of these per id IS the occurrence counter the pure [`due`] fold uses to compute the next fire time
/// (no mutable scheduler state — the log is the state). Written only by the daemon, never by an operator.
pub const SCHEDULE_FIRE: &str = "schedule-fire";

/// A schedule registered in the log: fire the TRIGGER event (`trigger_kind` + `payload`) at `first_ms`, then —
/// if `period_ms` is `Some` — every `period_ms` thereafter until cancelled (one-shot if `None`). `id` is the
/// identity: a later [`SCHEDULE_CREATE`] with the same id supersedes, and a [`SCHEDULE_CANCEL`] names it. All
/// times are absolute ms since the Unix epoch (the [`crate::clock`] unit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// The schedule's identity (supersede-by-id on re-create, cancel-by-id). Must not contain a newline (the
    /// codec is line-delimited); [`Schedule::encode`] is the writer and [`Schedule::decode`] the reader.
    pub id: String,
    /// Absolute ms-since-epoch of the FIRST fire. A periodic schedule's k-th occurrence is `first_ms + k*period`.
    pub first_ms: u64,
    /// The period in ms for a PERIODIC schedule; `None` = a ONE-SHOT (fires once at `first_ms`, then done).
    pub period_ms: Option<u64>,
    /// The event `kind` the daemon emits when this schedule fires (a TRIGGER the interpret plan then handles).
    pub trigger_kind: String,
    /// The payload the daemon emits with the fired trigger event.
    pub payload: Vec<u8>,
}

impl Schedule {
    /// Encode to the [`SCHEDULE_CREATE`] event payload: four newline-delimited header lines
    /// (`id`, `first_ms`, `period_ms` — empty for a one-shot, `trigger_kind`) followed by the raw `payload`
    /// bytes after the fourth newline. Line-delimited (not a nested format) to stay dependency-free and total —
    /// the header fields are constrained (an id/kind without a newline, decimal times), the payload is opaque
    /// bytes carried verbatim.
    pub fn encode(&self) -> Vec<u8> {
        let period = self.period_ms.map(|p| p.to_string()).unwrap_or_default();
        let header = format!(
            "{}\n{}\n{}\n{}\n",
            self.id, self.first_ms, period, self.trigger_kind
        );
        let mut out = header.into_bytes();
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decode a [`SCHEDULE_CREATE`] payload written by [`Schedule::encode`], or `None` if malformed (fewer than
    /// four header lines, or a non-decimal time). Total: a corrupt payload yields `None` rather than panicking,
    /// so the fold simply skips it (a create the daemon can't parse never fires — fail-safe, not fail-open).
    pub fn decode(bytes: &[u8]) -> Option<Schedule> {
        // Split off exactly four header lines; the remainder (after the 4th '\n') is the raw payload.
        let mut idx = 0usize;
        let mut lines: Vec<&[u8]> = Vec::with_capacity(4);
        for _ in 0..4 {
            let rel = bytes[idx..].iter().position(|&b| b == b'\n')?;
            lines.push(&bytes[idx..idx + rel]);
            idx += rel + 1;
        }
        let id = String::from_utf8(lines[0].to_vec()).ok()?;
        let first_ms = std::str::from_utf8(lines[1]).ok()?.trim().parse().ok()?;
        let period_line = std::str::from_utf8(lines[2]).ok()?.trim();
        let period_ms = if period_line.is_empty() {
            None
        } else {
            Some(period_line.parse().ok()?)
        };
        let trigger_kind = String::from_utf8(lines[3].to_vec()).ok()?;
        Some(Schedule {
            id,
            first_ms,
            period_ms,
            trigger_kind,
            payload: bytes[idx..].to_vec(),
        })
    }
}

/// Append a [`SCHEDULE_CREATE`] event registering `schedule`, returning its `seq`. A later create with the same
/// `id` supersedes (the fold keeps the newest); this is how a schedule enters the log (an operator via the CLI,
/// or a program's emitted create).
pub fn append_create(log: &mut impl Log, schedule: &Schedule) -> Result<Seq> {
    log.append(SCHEDULE_CREATE, &schedule.encode())
}

/// Append a [`SCHEDULE_CANCEL`] for schedule `id` — [`active_schedules`] thereafter excludes it.
pub fn append_cancel(log: &mut impl Log, id: &str) -> Result<Seq> {
    log.append(SCHEDULE_CANCEL, id.as_bytes())
}

/// Append a [`SCHEDULE_FIRE`] record for schedule `id` (the daemon calls this when it fires an occurrence). The
/// count of these per id is the occurrence counter [`due`] reads to compute the next fire time.
pub fn append_fire(log: &mut impl Log, id: &str) -> Result<Seq> {
    log.append(SCHEDULE_FIRE, id.as_bytes())
}

/// The ACTIVE schedules in `events`: the newest [`SCHEDULE_CREATE`] per id (self-superseding), MINUS any id
/// that a later [`SCHEDULE_CANCEL`] names. Order-preserving by first-appearance of each surviving id. This is
/// the set [`due`] then filters by time; separated so a caller can inspect the active set independently (e.g. a
/// `schedule-list` CLI verb later).
pub fn active_schedules(events: &[crate::Event]) -> Vec<Schedule> {
    // Cancels win only over creates that PRECEDE them; a create after a cancel re-registers. Walk in order,
    // keeping the latest create per id and forgetting an id when cancelled.
    use std::collections::HashMap;
    let mut latest: HashMap<String, Schedule> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for e in events {
        if e.kind == SCHEDULE_CREATE {
            if let Some(s) = Schedule::decode(&e.payload) {
                if !latest.contains_key(&s.id) {
                    order.push(s.id.clone());
                }
                latest.insert(s.id.clone(), s);
            }
        } else if e.kind == SCHEDULE_CANCEL {
            if let Ok(id) = String::from_utf8(e.payload.clone()) {
                latest.remove(&id);
                order.retain(|o| o != &id);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|id| latest.remove(&id))
        .collect()
}

/// The schedules DUE to fire as of `now_ms`: for each [`active_schedules`] entry, count its [`SCHEDULE_FIRE`]
/// records (`k` = occurrences already fired) and compute the next occurrence:
///   • ONE-SHOT (`period_ms == None`): next occurrence is `first_ms` if `k == 0`, else already fired (never due
///     again).
///   • PERIODIC (`period_ms == Some(p)`): the k-th occurrence is `first_ms + k*p`; due when `now_ms >=` that.
/// A schedule is DUE when its next occurrence is `<= now_ms`. Pure (time is the `now_ms` argument), so the
/// daemon computes this deterministically each tick, fires each due schedule (emit trigger + record a
/// [`SCHEDULE_FIRE`]), and re-derives the same answer on replay. A single tick reports each due schedule ONCE
/// (the daemon's fire record advances `k`, so the next occurrence moves past `now_ms` for the following tick);
/// catch-up of multiple missed periodic occurrences in one tick is a later refinement (one-per-tick for now).
pub fn due(events: &[crate::Event], now_ms: u64) -> Vec<Schedule> {
    use std::collections::HashMap;
    let mut fired: HashMap<String, u64> = HashMap::new();
    for e in events.iter().filter(|e| e.kind == SCHEDULE_FIRE) {
        if let Ok(id) = String::from_utf8(e.payload.clone()) {
            *fired.entry(id).or_insert(0) += 1;
        }
    }
    active_schedules(events)
        .into_iter()
        .filter(|s| {
            let k = fired.get(&s.id).copied().unwrap_or(0);
            match s.period_ms {
                None => k == 0 && now_ms >= s.first_ms,
                Some(p) => {
                    let next = s.first_ms.saturating_add(k.saturating_mul(p));
                    now_ms >= next
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileLog;

    fn temp_log() -> (std::path::PathBuf, FileLog) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!(
            "cdz-kernel-schedule-{}-{n}.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        (p.clone(), FileLog::open(&p).unwrap())
    }

    fn sched(id: &str, first_ms: u64, period_ms: Option<u64>) -> Schedule {
        Schedule {
            id: id.to_string(),
            first_ms,
            period_ms,
            trigger_kind: "tick".to_string(),
            payload: b"body".to_vec(),
        }
    }

    #[test]
    fn encode_decode_round_trips_a_one_shot_and_a_periodic() {
        // The codec is total + round-trips both shapes, including a payload that contains newlines (only the
        // FOUR header lines are structured; the payload after the 4th '\n' is carried verbatim).
        let one = Schedule {
            payload: b"multi\nline\npayload".to_vec(),
            ..sched("a", 1000, None)
        };
        let per = sched("b", 500, Some(250));
        assert_eq!(Schedule::decode(&one.encode()).as_ref(), Some(&one));
        assert_eq!(Schedule::decode(&per.encode()).as_ref(), Some(&per));
    }

    #[test]
    fn decode_is_total_on_garbage() {
        assert_eq!(Schedule::decode(b""), None, "empty → None, not a panic");
        assert_eq!(
            Schedule::decode(b"only\ntwo\n"),
            None,
            "too few header lines → None"
        );
        assert_eq!(
            Schedule::decode(b"id\nnotanumber\n\nkind\n"),
            None,
            "non-decimal first_ms → None"
        );
    }

    #[test]
    fn active_schedules_supersedes_by_id_and_drops_cancelled() {
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("a", 100, None)).unwrap();
        append_create(&mut log, &sched("b", 200, Some(50))).unwrap();
        // Supersede a with a later first_ms.
        append_create(&mut log, &sched("a", 999, None)).unwrap();
        // Cancel b.
        append_cancel(&mut log, "b").unwrap();
        let active = active_schedules(&log.tail(0).unwrap());
        assert_eq!(
            active.len(),
            1,
            "b cancelled, a superseded to one entry: {active:?}"
        );
        assert_eq!(active[0].id, "a");
        assert_eq!(active[0].first_ms, 999, "the LATER create wins");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_create_after_a_cancel_reregisters() {
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("a", 100, None)).unwrap();
        append_cancel(&mut log, "a").unwrap();
        append_create(&mut log, &sched("a", 300, None)).unwrap();
        let active = active_schedules(&log.tail(0).unwrap());
        assert_eq!(active.len(), 1, "a re-registered after its cancel");
        assert_eq!(active[0].first_ms, 300);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn due_one_shot_fires_once_then_never_again() {
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("a", 100, None)).unwrap();
        // Before first_ms: not due.
        assert!(
            due(&log.tail(0).unwrap(), 50).is_empty(),
            "before first_ms → not due"
        );
        // At/after first_ms with no fire yet: due.
        let d = due(&log.tail(0).unwrap(), 100);
        assert_eq!(d.len(), 1, "at first_ms with no fire → due");
        // Record the fire → no longer due (a one-shot fires exactly once).
        append_fire(&mut log, "a").unwrap();
        assert!(
            due(&log.tail(0).unwrap(), 10_000).is_empty(),
            "after its single fire, a one-shot is never due again"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn due_periodic_advances_with_each_fire() {
        // Periodic first_ms=100, period=100 → occurrences at 100, 200, 300, ... Each recorded fire advances the
        // next occurrence by one period, so the schedule is due again only once now passes the next boundary.
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("p", 100, Some(100))).unwrap();

        // k=0, next=100: due at now=150.
        assert_eq!(due(&log.tail(0).unwrap(), 150).len(), 1, "occurrence 0 due");
        append_fire(&mut log, "p").unwrap();
        // k=1, next=200: NOT due at 150, due at 200.
        assert!(
            due(&log.tail(0).unwrap(), 150).is_empty(),
            "occurrence 1 not yet (next=200)"
        );
        assert_eq!(
            due(&log.tail(0).unwrap(), 200).len(),
            1,
            "occurrence 1 due at 200"
        );
        append_fire(&mut log, "p").unwrap();
        // k=2, next=300: not due at 250.
        assert!(
            due(&log.tail(0).unwrap(), 250).is_empty(),
            "occurrence 2 not yet (next=300)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn due_excludes_a_cancelled_schedule_even_when_its_time_has_come() {
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("a", 100, Some(50))).unwrap();
        append_cancel(&mut log, "a").unwrap();
        assert!(
            due(&log.tail(0).unwrap(), 10_000).is_empty(),
            "a cancelled schedule is never due"
        );
        let _ = std::fs::remove_file(&path);
    }
}
