//! The requested optimization level — the stable `OptLevel` enum the `--opt-level` flag / manifest
//! profile parses into, and which a compile threads to the backend-independent pass manager.
//!
//! The taxonomy is the operator's decision (2026-07-15): Rust-style `O0`/`O1`/`O2`/`O3`, default `O1`.
//! Every level MUST produce OBSERVABLY-IDENTICAL behavior — only compile time / output speed / size
//! differ, never semantics. This is a plain boundary type; the `CorePass`/`PassManager` framework that
//! consumes it (and reads/refills a live `Db`) stays in `rcdzc`.

use std::fmt;
use std::str::FromStr;

/// The requested optimization level. Higher = more compile time spent for a faster/smaller artifact,
/// with IDENTICAL observable behavior. Ordered so a pass declares a MINIMUM level and the pass manager
/// runs it iff `requested >= min` — raising the level only ADDS behavior-preserving transformations,
/// never changes a result.
///
/// The taxonomy is the operator's decision (2026-07-15): Rust-style four levels, default [`OptLevel::O1`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum OptLevel {
    /// `-O0` — CANONICALIZATION only: the minimum to emit a correct, well-formed artifact (constant
    /// folding, admin-redex elimination, trivial dead-binding elimination). This is NOT "no passes" —
    /// it is the cheapest CORRECT emit; skipping these would mis-emit, not merely under-optimize. The
    /// max-dev-speed path.
    O0,
    /// `-O1` — the DEFAULT. `O0` plus cheap LOCAL cleanups: copy propagation, algebraic identities,
    /// local common-subexpression elimination, unreachable-arm removal. All per-node / per-region, no
    /// whole-function dataflow — good dev speed with meaningful wins.
    #[default]
    O1,
    /// `-O2` — the RELEASE default. `O1` plus WHOLE-FUNCTION analyses: loop-invariant code motion,
    /// global (dominator) common-subexpression elimination, accumulator introduction, non-trivial
    /// inlining. These are the expensive passes a dev iteration skips.
    O2,
    /// `-O3` — `O2` plus AGGRESSIVE / speculative passes: whole-program inlining, cross-function
    /// specialization, transformations whose cost is superlinear or whose payoff is workload-dependent.
    O3,
}

impl OptLevel {
    /// All levels, lowest to highest — for a CLI to enumerate (`--opt-level` value hints) and for the
    /// level-equivalence gate to sweep every level and assert an identical result at each.
    pub const ALL: [OptLevel; 4] = [OptLevel::O0, OptLevel::O1, OptLevel::O2, OptLevel::O3];

    /// The CLI-friendly canonical name — `"O0"`/`"O1"`/`"O2"`/`"O3"`. The inverse of [`FromStr`], so a
    /// round-trip `OptLevel::from_str(l.as_str()) == Ok(l)` holds for every level.
    pub fn as_str(self) -> &'static str {
        match self {
            OptLevel::O0 => "O0",
            OptLevel::O1 => "O1",
            OptLevel::O2 => "O2",
            OptLevel::O3 => "O3",
        }
    }
}

impl fmt::Display for OptLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OptLevel {
    type Err = String;

    /// Parse a level name for the `--opt-level` flag and the `Project.cdz` profile key. Accepts the
    /// canonical `O0..O3` (case-insensitive) plus the bare digit forms `0..3` (so `-O2` / `--opt-level
    /// 2` both work). An unknown value is a coded error naming the accepted set, so the CLI reports it
    /// rather than silently defaulting.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "o0" | "0" => Ok(OptLevel::O0),
            "o1" | "1" => Ok(OptLevel::O1),
            "o2" | "2" => Ok(OptLevel::O2),
            "o3" | "3" => Ok(OptLevel::O3),
            other => Err(format!(
                "unknown optimization level `{other}` — expected one of O0, O1, O2, O3"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_totally_ordered_low_to_high() {
        assert!(OptLevel::O0 < OptLevel::O1);
        assert!(OptLevel::O1 < OptLevel::O2);
        assert!(OptLevel::O2 < OptLevel::O3);
        // The ALL table is in ascending order.
        let mut sorted = OptLevel::ALL;
        sorted.sort();
        assert_eq!(sorted, OptLevel::ALL);
    }

    #[test]
    fn default_is_o1() {
        assert_eq!(OptLevel::default(), OptLevel::O1);
    }

    #[test]
    fn from_str_accepts_canonical_and_digit_forms() {
        for (input, want) in [
            ("O0", OptLevel::O0),
            ("o0", OptLevel::O0),
            ("0", OptLevel::O0),
            ("O1", OptLevel::O1),
            ("1", OptLevel::O1),
            ("O2", OptLevel::O2),
            ("2", OptLevel::O2),
            ("O3", OptLevel::O3),
            ("3", OptLevel::O3),
            (" o2 ", OptLevel::O2), // trimmed
        ] {
            assert_eq!(OptLevel::from_str(input), Ok(want), "parsing {input:?}");
        }
    }

    #[test]
    fn from_str_rejects_unknown_naming_the_set() {
        let err = OptLevel::from_str("O9").unwrap_err();
        assert!(
            err.contains("O0, O1, O2, O3"),
            "message names the set: {err}"
        );
        assert!(OptLevel::from_str("fast").is_err());
        assert!(OptLevel::from_str("").is_err());
    }

    #[test]
    fn display_and_from_str_round_trip() {
        for level in OptLevel::ALL {
            assert_eq!(OptLevel::from_str(level.as_str()), Ok(level));
            assert_eq!(level.to_string(), level.as_str());
        }
    }
}
