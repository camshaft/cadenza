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
