//! Host-boundary helpers — the process/thread/stack concerns the pure compile core deliberately
//! excludes. NOT part of the portable core (`compile.rs` and the query passes): the Cadenza self-host
//! establishes its own runtime preconditions, so nothing here is ported. Kept in the lib (not the bin)
//! so the bin AND the tests establish the SAME precondition through one definition.
//!
//! ## Why compilation runs on its own stack
//!
//! The compiler's contract is DECLINE-DON'T-CRASH: on any well-formed input it either produces a
//! component or reports a decline — it never aborts (`reference-compiler.md` §Outcomes Are Ordered By
//! Safety). Pathologically deep input (`(+ 1 (+ 1 …))` thousands deep, or an unproductive
//! self-recursion) is caught by the recursive-descent depth guard at [`crate::db::DESCENT_DEPTH_LIMIT`]:
//! past that depth the demand queries decline instead of recursing further.
//!
//! But the guard can only fire if the process has enough NATIVE STACK to reach it — each descent level
//! is a real stack frame, and an unoptimized (debug) build's frames are fat (~11 KB/level measured), so
//! reaching depth 1024 costs ~12 MB. The main thread's stack (≈8 MB) and, worse, a `cargo test` worker
//! thread's default stack (≈2 MB) are too small: the native stack overflows and the process ABORTS
//! LONG BEFORE the semantic guard trips (a 2 MB thread dies at ~depth 179). That is not a compiler bug —
//! the guard is sound — it is a missing runtime precondition. The historical fix was the environment
//! (`RUST_MIN_STACK=64M`), which is invisible and easy to forget; establishing the stack HERE, sized
//! FROM the guard, makes the guard authoritative in every build profile with nothing to remember.
//!
//! So the host runs compilation on a worker thread whose stack is sized to comfortably reach the guard.
//! This is exactly what production compilers do (rustc's `run_in_thread_with_globals`); the recursion
//! bound is the policy, and the host guarantees the stack the policy needs.

use crate::db::DESCENT_DEPTH_LIMIT;

/// Native-stack budget reserved per recursive-descent level. The demand queries recurse one native
/// frame per descent level; a fat unoptimized (debug) frame measures ~11 KB, so 64 KB/level is a >5×
/// margin that holds across build profiles and platforms without over-reserving address space.
const STACK_BYTES_PER_DESCENT_LEVEL: usize = 64 * 1024;

/// The worker-thread stack size that guarantees the recursive-descent depth guard
/// ([`crate::db::DESCENT_DEPTH_LIMIT`]) is reached before the native stack is exhausted — derived FROM
/// the guard so the two stay in lockstep: raise the guard and the stack grows with it. (`1024 · 64 KB`
/// = 64 MB — the same order the repo already uses elsewhere for deep-recursion tests.)
pub fn compiler_stack_bytes() -> usize {
    DESCENT_DEPTH_LIMIT as usize * STACK_BYTES_PER_DESCENT_LEVEL
}

/// Run `f` on a worker thread with a stack large enough to reach the recursive-descent depth guard
/// (see the module docs). The result is returned to the caller; a panic inside `f` is propagated so
/// the caller observes it exactly as if `f` ran inline. This is the ONE place the compile-stack
/// precondition is established — the bin and the deep-input test both go through it.
pub fn run_with_compiler_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    // The closure and its result borrow the caller's frame for the duration of the join, so a scoped
    // thread lets `f` capture by reference (no `'static` bound) — the bin passes borrowed inputs.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("rcdzc-compile".into())
            .stack_size(compiler_stack_bytes())
            .spawn_scoped(scope, f)
            .expect("spawn compile worker thread")
            .join()
            // Re-raise a panic from the worker on the caller's thread, preserving the original payload
            // (so a test's `#[should_panic]` / assertion, or an operator's backtrace, is unchanged).
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    })
}
