//! The async-runtime primitives the reducer actors run on — task spawning and channels — selected at
//! compile time (`design/cadenza-platform.md` §9).
//!
//! Each reducer is its own task with a mailbox channel, so the runtime needs to spawn tasks and create
//! channels. Two executors provide those: **tokio** in production, and **bach** (the deterministic
//! discrete-event simulator) in tests. Their `spawn` and `mpsc` APIs are identical, but bach's channels add
//! cooperative-scheduling hooks the simulator needs to control interleaving — so a test build must route
//! through bach's channels, not tokio's, or it loses determinism. Since we only ever drive the reducers
//! with bach under test, this is a compile-time swap rather than a runtime abstraction: bach under
//! `cfg(any(test, feature = "testing"))`, tokio otherwise. The actor code above is written once against
//! this `rt` surface and is identical for both.

#[cfg(any(test, feature = "testing"))]
pub use bach::sync::mpsc;
#[cfg(any(test, feature = "testing"))]
pub use bach::task::spawn;

#[cfg(not(any(test, feature = "testing")))]
pub use tokio::spawn;
#[cfg(not(any(test, feature = "testing")))]
pub use tokio::sync::mpsc;
