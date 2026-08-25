//! Node-level configuration a platform run is tuned by (`design/cadenza-platform.md` §3). Kept as plain
//! values a node populates at assembly (from its own env/file/args) and threads into the pieces that need
//! them — never hard-coded caps baked into the platform source.

use std::time::Duration;

/// The per-reducer resource limits that keep one guest from taking down the host — the two ways it could:
/// monopolizing compute, and exhausting memory (`host::arm_store_safety` enforces them; the epoch ticker in
/// [`TaskSystem`](crate::TaskSystem) drives the compute half). A node sets these for itself; the platform only
/// provides the type, the threading, and the [`Default`] below. NOT hard-coded module constants — the operator
/// tunes them per node without editing platform source.
///
/// The compute bound is epoch-based: the engine's epoch is advanced every [`epoch_tick`](Self::epoch_tick),
/// and a guest fold yields to the async executor every [`yield_every`](Self::yield_every) ticks of compute
/// (so it can never monopolize a thread), trapping once it has yielded [`max_yields`](Self::max_yields) times
/// (its cumulative compute budget). The memory bound is a ceiling on each linear memory
/// ([`max_linear_memory_bytes`](Self::max_linear_memory_bytes)); a growth past it traps. Either breach is a
/// clean per-reducer `Crashed` (§7), never a process-wide failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    /// How often the wasm engine's epoch is advanced (the ticker cadence). Finer = prompter preemption at a
    /// slightly higher ticker cost; the budget below is denominated in these ticks.
    pub epoch_tick: Duration,
    /// Epoch ticks of guest compute between executor yields. A fold yields (never monopolizing a thread) each
    /// time this many ticks elapse inside it.
    pub yield_every: u64,
    /// How many times a single fold may yield before it traps — its cumulative compute budget is about
    /// `max_yields * yield_every * epoch_tick`. A real fold finishes in well under one tick, so this only ever
    /// bounds a runaway.
    pub max_yields: u64,
    /// The maximum size (bytes) each of a reducer's linear memories may grow to; a growth past it traps rather
    /// than exhausting host RAM.
    pub max_linear_memory_bytes: usize,
}

impl Default for ResourceLimits {
    /// Documented defaults, generous enough that they only ever bound a genuinely runaway guest — a real fold
    /// is sub-millisecond and needs far less than 256 MiB. A node overrides any of these; they are the
    /// starting point, not the only option.
    fn default() -> Self {
        Self {
            epoch_tick: Duration::from_millis(1),
            yield_every: 1,
            max_yields: 5_000,
            max_linear_memory_bytes: 256 * 1024 * 1024,
        }
    }
}

/// The per-spawn compute + memory budget a spawn may request (`design/cadenza-platform.md` §5 /
/// `DESIGN-per-spawn-limits-and-spawn-capability`): the two limits the operator named as per-spawn — a memory
/// ceiling and a max-yields (compute) budget. The epoch tick and yield cadence stay node-wide (a property of
/// the node's runtime, not of one reducer), so they are not per-spawn. A spawn that requests these is clamped
/// to the node's [`ResourceLimits`] ceiling by [`ResourceLimits::resolve_for_spawn`] — a spawn can lower its
/// own budget but never raise it above what the node permits (the host-side backstop). A spawn that requests
/// nothing inherits the node's limits unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnLimits {
    /// The maximum size (bytes) each of this reducer's linear memories may grow to. Clamped to the node's
    /// `max_linear_memory_bytes`.
    pub max_linear_memory_bytes: usize,
    /// This reducer's cumulative compute budget, as a max-yields count. Clamped to the node's `max_yields`.
    pub max_yields: u64,
}

impl ResourceLimits {
    /// The effective per-reducer limits for a spawn: the node limits, with the two per-spawn budgets (memory +
    /// max-yields) taken from `requested` but **clamped to the node ceiling** — a spawn can request a *smaller*
    /// budget for itself, never a larger one than the node permits (the host-side backstop, defense in depth
    /// under the admission reducer). `None` inherits the node limits unchanged. The node-wide cadence
    /// (`epoch_tick`, `yield_every`) is always the node's.
    #[must_use]
    pub fn resolve_for_spawn(&self, requested: Option<SpawnLimits>) -> ResourceLimits {
        match requested {
            None => *self,
            Some(req) => ResourceLimits {
                max_linear_memory_bytes: req
                    .max_linear_memory_bytes
                    .min(self.max_linear_memory_bytes),
                max_yields: req.max_yields.min(self.max_yields),
                ..*self
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResourceLimits, SpawnLimits};

    fn node() -> ResourceLimits {
        ResourceLimits {
            max_yields: 5_000,
            max_linear_memory_bytes: 256 * 1024 * 1024,
            ..ResourceLimits::default()
        }
    }

    #[test]
    fn no_request_inherits_the_node_limits_unchanged() {
        assert_eq!(node().resolve_for_spawn(None), node());
    }

    #[test]
    fn a_smaller_request_is_honored() {
        // A spawn may request a tighter budget than the node — both budgets take the requested (smaller) value,
        // while the node-wide cadence (epoch_tick/yield_every) stays the node's.
        let eff = node().resolve_for_spawn(Some(SpawnLimits {
            max_linear_memory_bytes: 1024 * 1024,
            max_yields: 100,
        }));
        assert_eq!(eff.max_linear_memory_bytes, 1024 * 1024);
        assert_eq!(eff.max_yields, 100);
        assert_eq!(eff.epoch_tick, node().epoch_tick);
        assert_eq!(eff.yield_every, node().yield_every);
    }

    #[test]
    fn a_request_over_the_node_ceiling_is_clamped_down() {
        // The host-side backstop: a spawn can NEVER raise its budget above what the node permits — a request
        // exceeding the ceiling is clamped to the node's, per budget independently.
        let eff = node().resolve_for_spawn(Some(SpawnLimits {
            max_linear_memory_bytes: usize::MAX,
            max_yields: u64::MAX,
        }));
        assert_eq!(eff.max_linear_memory_bytes, node().max_linear_memory_bytes);
        assert_eq!(eff.max_yields, node().max_yields);
    }

    #[test]
    fn each_budget_is_clamped_independently() {
        // Memory below the ceiling but yields above it (or vice versa): each budget clamps on its own, so a
        // spawn cannot smuggle a larger budget in one dimension by staying under in another.
        let eff = node().resolve_for_spawn(Some(SpawnLimits {
            max_linear_memory_bytes: 4096,
            max_yields: u64::MAX,
        }));
        assert_eq!(eff.max_linear_memory_bytes, 4096);
        assert_eq!(eff.max_yields, node().max_yields);
    }
}
