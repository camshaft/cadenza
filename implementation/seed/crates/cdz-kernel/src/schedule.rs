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
//!   • [`SCHEDULE_FIRE`] — the daemon's OWN record that it fired a schedule, carrying the occurrence count it
//!     advanced TO (`<id>\t<fired_to>`). The MAX `fired_to` per id is the occurrence counter the pure [`due`]
//!     fold reads (no mutable scheduler state — the log is the state): a one-shot with a fire is done; a periodic
//!     at `fired_to = k` is next due at `first_ms + k*period_ms`.
//!
//! COALESCE semantics (operator ruling): when a periodic falls behind by M missed occurrences (the daemon was
//! down), [`due`] returns ONE entry with `fire_to` jumped to the CURRENT occurrence — the daemon emits a single
//! trigger and records the jump, so there is NO backlog burst (a self-DoS a periodic-behind-by-1440 would cause
//! contradicts the capability model's can't-brick principle). An opt-in per-schedule `catch_up` flag (fire every
//! missed occurrence, for must-not-miss cron/billing jobs) is a clean future add — [`Due::fire_to`] is the seam.

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

/// The event `kind` the daemon appends when it FIRES a schedule — payload is `<id>\t<fired_to>` (the occurrence
/// count advanced to). The MAX `fired_to` per id IS the occurrence counter the pure [`due`] fold uses to compute
/// the next fire time (no mutable scheduler state — the log is the state). Written only by the daemon.
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

/// Append a [`SCHEDULE_FIRE`] record for schedule `id`, recording that occurrences THROUGH `fired_to` have now
/// fired (payload `<id>\t<fired_to>`, where `fired_to` is the NEW occurrence count k'). The daemon calls this
/// when it fires; [`due`] reads the MAX `fired_to` per id as the occurrence counter. A single record can jump
/// the counter past a whole backlog — that's how COALESCE works (a periodic behind by M occurrences fires ONE
/// trigger and records `fired_to = k + M`, so the next tick sees the counter already caught up, no burst).
pub fn append_fire(log: &mut impl Log, id: &str, fired_to: u64) -> Result<Seq> {
    log.append(SCHEDULE_FIRE, format!("{id}\t{fired_to}").as_bytes())
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

/// The occurrence count already fired per schedule id — the MAX `fired_to` across its [`SCHEDULE_FIRE`] records
/// (each records the resulting count, so max = furthest-advanced; replay-stable regardless of record order). A
/// legacy bare-id record (no `\t<fired_to>`) counts as one occurrence (`max(current, current+1)`), so an older
/// log still advances — but the daemon always writes the tab form now.
fn fired_counts(events: &[crate::Event]) -> std::collections::HashMap<String, u64> {
    use std::collections::HashMap;
    let mut fired: HashMap<String, u64> = HashMap::new();
    for e in events.iter().filter(|e| e.kind == SCHEDULE_FIRE) {
        let Ok(text) = String::from_utf8(e.payload.clone()) else {
            continue;
        };
        match text.split_once('\t') {
            Some((id, n)) => {
                if let Ok(fired_to) = n.trim().parse::<u64>() {
                    let slot = fired.entry(id.to_string()).or_insert(0);
                    *slot = (*slot).max(fired_to);
                }
            }
            // Legacy bare id: treat as a single occurrence (advance by one).
            None => {
                let slot = fired.entry(text).or_insert(0);
                *slot += 1;
            }
        }
    }
    fired
}

/// How many occurrences of a periodic schedule have come due as of `now_ms` (0 if before `first_ms`): one for
/// `first_ms`, plus one per elapsed period. `(now - first) / period + 1`. Used to compute the COALESCE target.
fn occurrences_due(first_ms: u64, period_ms: u64, now_ms: u64) -> u64 {
    if now_ms < first_ms {
        return 0;
    }
    (now_ms - first_ms) / period_ms + 1
}

/// One entry the daemon should FIRE this tick: the schedule + the occurrence count to advance TO (`fire_to`).
/// COALESCE default: a periodic behind by M occurrences yields ONE `Due` with `fire_to` jumped to the current
/// occurrence, so the daemon emits ONE trigger and records `append_fire(id, fire_to)` — no backlog burst. (A
/// future opt-in `catch_up` flag would instead yield the full missed range; the `fire_to` field is the seam.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub schedule: Schedule,
    /// The new occurrence count to record via [`append_fire`] after firing — coalesced to the current boundary.
    pub fire_to: u64,
}

/// The schedules DUE to fire as of `now_ms`, with the COALESCE target (operator ruling: coalesce is the default,
/// no backlog burst — matches the can't-brick / no-self-DoS principle). For each [`active_schedules`] entry, read
/// its already-fired occurrence count `k` ([`fired_counts`]) and compute:
///   • ONE-SHOT (`period_ms == None`): due iff `k == 0 && now_ms >= first_ms`; `fire_to = 1`.
///   • PERIODIC (`period_ms == Some(p)`): `occ` = occurrences come due by now ([`occurrences_due`]); due iff
///     `occ > k` (at least one un-fired occurrence); `fire_to = occ` — COALESCE: one fire jumps straight to the
///     current occurrence, collapsing any backlog of missed occurrences into a single trigger.
/// Pure (time is the `now_ms` argument), so the daemon re-derives the same answer on replay. The daemon fires
/// each `Due` once (emit trigger + [`append_fire`]`(id, fire_to)`); the recorded jump means the following tick
/// sees the counter already at `fire_to`, so no re-fire and no accumulated burst.
pub fn due(events: &[crate::Event], now_ms: u64) -> Vec<Due> {
    let fired = fired_counts(events);
    active_schedules(events)
        .into_iter()
        .filter_map(|s| {
            let k = fired.get(&s.id).copied().unwrap_or(0);
            match s.period_ms {
                None => (k == 0 && now_ms >= s.first_ms).then_some(Due {
                    schedule: s,
                    fire_to: 1,
                }),
                Some(p) => {
                    let occ = occurrences_due(s.first_ms, p, now_ms);
                    (occ > k).then_some(Due {
                        schedule: s,
                        fire_to: occ,
                    })
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
        // At/after first_ms with no fire yet: due, fire_to = 1.
        let d = due(&log.tail(0).unwrap(), 100);
        assert_eq!(d.len(), 1, "at first_ms with no fire → due");
        assert_eq!(d[0].fire_to, 1, "one-shot fires occurrence 1");
        // Record the fire → no longer due (a one-shot fires exactly once).
        append_fire(&mut log, "a", 1).unwrap();
        assert!(
            due(&log.tail(0).unwrap(), 10_000).is_empty(),
            "after its single fire, a one-shot is never due again"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn due_periodic_advances_with_each_fire() {
        // Periodic first_ms=100, period=100 → occurrences at 100, 200, 300, ... Each recorded fire advances the
        // occurrence counter, so the schedule is due again only once now passes the next boundary.
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("p", 100, Some(100))).unwrap();

        // k=0, occurrence 1 due at now=150 (fire_to=1).
        let d = due(&log.tail(0).unwrap(), 150);
        assert_eq!(d.len(), 1, "occurrence 1 due");
        assert_eq!(d[0].fire_to, 1);
        append_fire(&mut log, "p", 1).unwrap();
        // k=1, occurrence 2 boundary at 200: NOT due at 150, due at 200.
        assert!(
            due(&log.tail(0).unwrap(), 150).is_empty(),
            "occurrence 2 not yet (boundary 200)"
        );
        let d = due(&log.tail(0).unwrap(), 200);
        assert_eq!(d.len(), 1, "occurrence 2 due at 200");
        assert_eq!(d[0].fire_to, 2);
        append_fire(&mut log, "p", 2).unwrap();
        // k=2, occurrence 3 boundary at 300: not due at 250.
        assert!(
            due(&log.tail(0).unwrap(), 250).is_empty(),
            "occurrence 3 not yet (boundary 300)"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn due_coalesces_a_periodic_backlog_into_one_fire() {
        // COALESCE (operator ruling): a periodic behind by MANY occurrences fires ONCE with fire_to jumped to the
        // current occurrence — no backlog burst. first_ms=0, period=1 → by now=100 there are 101 occurrences
        // (0,1,..,100) come due; with k=0 the single Due has fire_to=101, and recording it clears the backlog.
        let (path, mut log) = temp_log();
        append_create(&mut log, &sched("p", 0, Some(1))).unwrap();

        let d = due(&log.tail(0).unwrap(), 100);
        assert_eq!(d.len(), 1, "one Due entry, not 101 (coalesced)");
        assert_eq!(
            d[0].fire_to, 101,
            "fire_to jumps to the current occurrence count (0..=100 = 101)"
        );
        // Record the coalesced fire → the counter catches up, nothing due until the NEXT future boundary.
        append_fire(&mut log, "p", d[0].fire_to).unwrap();
        assert!(
            due(&log.tail(0).unwrap(), 100).is_empty(),
            "after the coalesced fire, not due again at the same now"
        );
        // Advancing past the next boundary (occurrence 101 at ms 101) makes it due once more, fire_to=102.
        let d2 = due(&log.tail(0).unwrap(), 101);
        assert_eq!(d2.len(), 1);
        assert_eq!(d2[0].fire_to, 102, "next occurrence after catch-up");
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
