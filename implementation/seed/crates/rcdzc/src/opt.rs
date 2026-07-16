//! Cost-tiered optimization levels — the `OptLevel` enum and the `PassManager` that gates each pass by
//! its declared tier. This is the BACKEND-INDEPENDENT optimization framework: it runs on the shared Core
//! column ABOVE the backend split, so every backend (rust, wasm, any future one) inherits its passes.
//!
//! The design is `implementation/design/DESIGN-tiered-optimization-levels-rcdzc.md`. The taxonomy is the
//! operator's decision (2026-07-15): Rust-style `O0`/`O1`/`O2`/`O3`, default `O1`.
//!
//! **The one rule (correctness bar).** Every level MUST produce OBSERVABLY-IDENTICAL behavior — only
//! compile time / output speed / size differ, never semantics. A higher level that changes a result is a
//! miscompile (`core-semantics.md` §Observable Behavior Is A Defined Projection Of A Run). This is why a
//! pass declares a MINIMUM level and the manager runs it iff `requested >= min`: raising the level only
//! ever ADDS behavior-preserving transformations.
//!
//! **Why tier-by-construction.** A pass is registered with its `min_level`; the manager runs only the
//! passes whose tier the requested level reaches. Adding a pass to the pipeline forces a tier choice (the
//! registration takes one), so the fast path (`O0`/`O1`) stays fast as passes accumulate — the reason to
//! build this now, before there are many untiered passes to retrofit.
//!
//! This slice lands the LEVEL + MANAGER skeleton (the stable `OptLevel` enum v-cdz-tooling's
//! `--opt-level` flag / manifest profile parses into, and an empty-but-typed pass pipeline). Migrating
//! the existing `lower.rs` folds and lifting the wasm-backend whole-function passes (LICM, global CSE,
//! accumulator introduction) up to Core O2 passes are later slices, each landed with a level-equivalence
//! gate case proving behavior is unchanged at every level on both backends.

use std::fmt;
use std::str::FromStr;
use tracing::trace;

/// The requested optimization level. Higher = more compile time spent for a faster/smaller artifact,
/// with IDENTICAL observable behavior. Ordered so a pass declares a MINIMUM level and the
/// [`PassManager`] runs it iff `requested >= min` — raising the level only ADDS behavior-preserving
/// transformations, never changes a result.
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

    /// Parse a level name for v-cdz-tooling's `--opt-level` flag and the `Project.cdz` profile key.
    /// Accepts the canonical `O0..O3` (case-insensitive) plus the bare digit forms `0..3` (so
    /// `-O2` / `--opt-level 2` both work). An unknown value is a coded error naming the accepted set,
    /// so the CLI reports it rather than silently defaulting.
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

/// One backend-independent optimization pass over the shared Core column. A pass declares the LOWEST
/// [`OptLevel`] at which it runs (`O0` = always-on canonicalization) and transforms the `Db`'s core
/// column in place. Every pass MUST be behavior-preserving at every level (the correctness bar).
///
/// This trait is the seam the migration fills: the existing `lower.rs` folds become `O0`/`O1` passes and
/// the lifted wasm whole-function analyses become `O2` passes, each registered with the manager. It takes
/// `&mut crate::db::Db` so a pass reads/refills the core column through the normal query producers.
pub trait CorePass {
    /// The lowest level at which this pass runs. `requested >= min_level()` gates it in the manager.
    fn min_level(&self) -> OptLevel;
    /// A short stable identifier for tracing / the pass-timing report.
    fn name(&self) -> &'static str;
    /// Transform the core column in place. MUST preserve observable behavior at every level.
    fn run(&self, db: &mut crate::db::Db);
}

/// Sequences the registered [`CorePass`]es and runs those the requested [`OptLevel`] enables. The manager
/// only sequences + tier-gates passes; it does not implement them. Construct with [`PassManager::for_level`]
/// (registers the standard pipeline) and drive with [`PassManager::run`].
pub struct PassManager {
    level: OptLevel,
    passes: Vec<Box<dyn CorePass>>,
}

impl PassManager {
    /// Build the manager for a requested level, registering the standard Core-pass pipeline in canonical
    /// order. (This slice registers NO passes yet — the pipeline is empty, so `run` is a no-op and the
    /// emitted artifact is byte-identical at every level. Passes are added under this seam in later
    /// slices, each with its declared tier and a level-equivalence gate case.)
    pub fn for_level(level: OptLevel) -> Self {
        PassManager {
            level,
            passes: Vec::new(),
        }
    }

    /// The level this manager runs at.
    pub fn level(&self) -> OptLevel {
        self.level
    }

    /// Register a pass. Kept `pub` so the migration slices can add passes incrementally; the standard
    /// pipeline is assembled in [`PassManager::for_level`].
    pub fn register(&mut self, pass: Box<dyn CorePass>) {
        self.passes.push(pass);
    }

    /// The passes this level ENABLES, in pipeline order — those whose `min_level <= self.level`.
    /// Exposed so a caller / test can see exactly which passes a level selects (and so the pass-timing
    /// report and the level-equivalence gate can enumerate the active set).
    pub fn enabled(&self) -> impl Iterator<Item = &dyn CorePass> {
        self.passes
            .iter()
            .filter(move |p| self.level >= p.min_level())
            .map(|p| p.as_ref())
    }

    /// Run every enabled pass over the core column, in registration order. A pass whose `min_level`
    /// exceeds the requested level is skipped — so `O0` runs only the canonicalizations, `O3` runs the
    /// whole pipeline.
    pub fn run(&self, db: &mut crate::db::Db) {
        for pass in &self.passes {
            if self.level >= pass.min_level() {
                trace!(target: "rcdzc::opt", pass = pass.name(), level = %self.level, "running core pass");
                pass.run(db);
            }
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

    // A pass that records whether it ran, so we can assert the manager's tier gating.
    struct MarkerPass {
        min: OptLevel,
        name: &'static str,
    }
    impl CorePass for MarkerPass {
        fn min_level(&self) -> OptLevel {
            self.min
        }
        fn name(&self) -> &'static str {
            self.name
        }
        fn run(&self, _db: &mut crate::db::Db) {}
    }

    #[test]
    fn manager_enables_only_passes_at_or_below_the_level() {
        let mut pm = PassManager::for_level(OptLevel::O1);
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O0,
            name: "canon",
        }));
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O1,
            name: "local-cleanup",
        }));
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O2,
            name: "whole-fn",
        }));
        let enabled: Vec<&str> = pm.enabled().map(|p| p.name()).collect();
        // O1 enables the O0 + O1 passes, not the O2 one.
        assert_eq!(enabled, vec!["canon", "local-cleanup"]);
    }

    #[test]
    fn o0_enables_only_canonicalization() {
        let mut pm = PassManager::for_level(OptLevel::O0);
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O0,
            name: "canon",
        }));
        pm.register(Box::new(MarkerPass {
            min: OptLevel::O1,
            name: "local-cleanup",
        }));
        let enabled: Vec<&str> = pm.enabled().map(|p| p.name()).collect();
        assert_eq!(enabled, vec!["canon"]);
    }

    #[test]
    fn o3_enables_the_whole_pipeline() {
        let mut pm = PassManager::for_level(OptLevel::O3);
        for (min, name) in [
            (OptLevel::O0, "a"),
            (OptLevel::O1, "b"),
            (OptLevel::O2, "c"),
            (OptLevel::O3, "d"),
        ] {
            pm.register(Box::new(MarkerPass { min, name }));
        }
        assert_eq!(pm.enabled().count(), 4);
    }

    // ── The core-override seam (§9a) — the mechanism a real CorePass uses to rewrite the Core-IR ──
    // These prove: (1) an empty override map leaves `core_of` behavior unchanged (the level-equivalence
    // baseline — the whole corpus gate exercises this path); (2) an installed override WINS over the
    // lowered/memoized form; (3) an IDENTITY override (reinstalling the node's own core) is behavior-
    // preserving. Together they de-risk the seam before any real rewrite pass registers.

    #[test]
    fn empty_override_map_leaves_core_of_unchanged() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        assert!(!db.has_core_overrides(), "fresh Db has no overrides");
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let natural = crate::lower::core_of(&mut db, body);
        // Still no overrides after a normal lowering.
        assert!(!db.has_core_overrides());
        match natural {
            crate::core::Core::ConstInt(ref v) => assert_eq!(v.to_i64(), Some(42)),
            other => panic!("main's body lowers to ConstInt(42), got {other:?}"),
        }
    }

    #[test]
    fn an_installed_override_wins_over_the_lowered_form() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        // Lower it once (fills the column) so we prove the override wins even over a FILLED slot.
        let natural = crate::lower::core_of(&mut db, body);
        assert!(matches!(&natural, crate::core::Core::ConstInt(v) if v.to_i64() == Some(42)));
        // A pass installs a (deliberately distinct) override for this node.
        db.install_core_override(
            body,
            crate::core::Core::ConstInt(crate::ast::IntValue::from_i64(999)),
        );
        assert!(db.has_core_overrides());
        let after = crate::lower::core_of(&mut db, body);
        match after {
            crate::core::Core::ConstInt(ref v) => assert_eq!(
                v.to_i64(),
                Some(999),
                "core_of returns the pass-installed override"
            ),
            other => panic!("expected the override ConstInt(999), got {other:?}"),
        }
    }

    #[test]
    fn an_identity_override_is_behavior_preserving() {
        let mut db = crate::db::Db::load(crate::testkit::parse(
            "(module m (def (main) 42) (export main))",
        ));
        let d = db.def_by_name("main").expect("def main");
        let body = db.defs[d].body.expect("main body");
        let natural = crate::lower::core_of(&mut db, body);
        // An identity pass reinstalls the node's OWN core as its override — the seam is exercised but
        // the result is unchanged (the byte-identical de-risking case the design's slice-1 calls for).
        db.install_core_override(body, natural.clone());
        assert!(db.has_core_overrides());
        let after = crate::lower::core_of(&mut db, body);
        assert_eq!(
            format!("{natural:?}"),
            format!("{after:?}"),
            "an identity override leaves core_of's result unchanged"
        );
    }
}
