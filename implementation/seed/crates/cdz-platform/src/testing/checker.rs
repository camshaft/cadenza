//! Checkers — asserting over a run's observation log (`design/cadenza-platform.md` §9).
//!
//! A run produces a [`Run`] (the observation log plus the name→id assignment); a **checker** reads it and
//! decides pass/fail. This is the assertion side of the harness: the harness records *what happened* in a
//! language-neutral log, and a checker states *what should have happened* and verifies it.
//!
//! A checker is any `Fn(&Run) -> CheckOutcome` (the blanket impl below), so a test writes one inline; the
//! [`Checker`] trait is the contract. This is the **native** realization — the eventual wasm checker (an
//! opaque program the harness passes in) implements the same contract over a serialized log, so a run and
//! its checker stay language-neutral either way. A checker never mutates the run; it only reads and judges.

use super::harness::Run;
use super::observation::render;

/// A checker's verdict over a run: it passed, or it failed with one or more reasons. Reasons are plain
/// strings for a human or a report to read — a checker states, in its own words, what did not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Everything the checker asserted held.
    Pass,
    /// One or more assertions did not hold, each with a reason.
    Fail { reasons: Vec<String> },
}

impl CheckOutcome {
    /// A passing verdict.
    #[must_use]
    pub fn pass() -> Self {
        Self::Pass
    }

    /// A failing verdict with a single reason.
    #[must_use]
    pub fn fail(reason: impl Into<String>) -> Self {
        Self::Fail {
            reasons: vec![reason.into()],
        }
    }

    /// A verdict from a list of reasons: [`Pass`](CheckOutcome::Pass) if empty, else
    /// [`Fail`](CheckOutcome::Fail) carrying them. Handy for a checker that collects every failed
    /// assertion rather than stopping at the first.
    #[must_use]
    pub fn from_reasons(reasons: Vec<String>) -> Self {
        if reasons.is_empty() {
            Self::Pass
        } else {
            Self::Fail { reasons }
        }
    }

    /// Whether the verdict is a pass.
    #[must_use]
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// The failure reasons — empty on a pass.
    #[must_use]
    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Pass => &[],
            Self::Fail { reasons } => reasons,
        }
    }

    /// A human diagnostic for this verdict over `run`: on a pass, a one-line note; on a failure, the
    /// reasons followed by the rendered observation log ([`render`](super::render)), so a failed check
    /// shows *what actually happened* alongside *what should have* (§9). Handy as an assertion message:
    /// `assert!(v.is_pass(), "{}", v.report(&run))`.
    #[must_use]
    pub fn report(&self, run: &Run) -> String {
        match self {
            Self::Pass => "check passed".to_string(),
            Self::Fail { reasons } => {
                let bullets: String = reasons.iter().map(|r| format!("  - {r}\n")).collect();
                format!(
                    "check FAILED:\n{bullets}\nobservation log ({} records):\n{}",
                    run.records.len(),
                    render(&run.records)
                )
            }
        }
    }
}

/// A checker: reads a completed [`Run`]'s observation log and decides pass/fail. The native side of the
/// checker contract; a wasm checker (later) implements the same judgement over a serialized log. Any
/// `Fn(&Run) -> CheckOutcome` is a checker (blanket impl), so a test writes one inline.
pub trait Checker {
    /// Judge `run`, returning the verdict.
    fn check(&self, run: &Run) -> CheckOutcome;
}

impl<F: Fn(&Run) -> CheckOutcome> Checker for F {
    fn check(&self, run: &Run) -> CheckOutcome {
        self(run)
    }
}

#[cfg(test)]
mod tests {
    use super::CheckOutcome;
    use crate::testing::{Entry, EventOp, Record, Run};
    use crate::{Bytes, ContractId, HostId, Origin, ReducerId};
    use std::collections::BTreeMap;

    fn one_record_run() -> Run {
        Run {
            records: vec![Record {
                seq: 0,
                time_ns: 0,
                source: Origin {
                    reducer: ReducerId::of(b"agent"),
                    host: HostId::of(b"node"),
                },
                entry: Entry::Event(EventOp::Emitted {
                    contract: ContractId::of(b"http.get"),
                    payload: Bytes::new(),
                    continuation_token: Bytes::new(),
                    has_deadline: false,
                }),
            }],
            ids: BTreeMap::new(),
        }
    }

    #[test]
    fn report_notes_a_pass_and_renders_the_log_with_the_reasons_on_a_failure() {
        let run = one_record_run();
        assert_eq!(CheckOutcome::pass().report(&run), "check passed");

        let report = CheckOutcome::fail("emitter never performed http.get").report(&run);
        assert!(report.contains("check FAILED"), "report: {report}");
        assert!(
            report.contains("emitter never performed http.get"),
            "report carries the reason: {report}"
        );
        // The rendered observation log is included so a failed check shows what actually happened (§9).
        assert!(
            report.contains("observation log"),
            "report labels the log: {report}"
        );
        assert!(
            report.contains("emit "),
            "report renders the run's records: {report}"
        );
    }
}
