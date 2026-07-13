//! Bolero property target: a byte seed, through the generator, should never PANIC the compiler.
//!
//! This is the same generator + oracle the continuous driver uses, wired to `bolero` so we get, for
//! free: (1) shrinking — a failing seed is minimized to the smallest byte string that still panics;
//! (2) `cargo test` integration — it runs as a bounded randomized property in the normal suite,
//! guarding against a regression that reintroduces a crash the driver already cleared; and
//! (3) coverage-guided fuzzing — `cargo bolero test cdz_smith_never_panics -p cdz-smith` (nightly +
//! `cargo install cargo-bolero`) drives it with libFuzzer/AFL and its own persistent corpus.
//!
//! The property is deliberately ONLY "does not panic". A decline or a coded rejection is expected,
//! correct output — never a failure. A found panic is a compiler bug; bolero prints the seed, which
//! `cdz-smith gen <seed>` / `cdz-smith once <seed>` reproduce. (Timeouts are out of scope here —
//! a hang cannot be caught in-process; the continuous driver's watchdog owns that oracle.)

use bolero::check;
use cdz_smith::generator::generate;
use cdz_smith::oracle::{Verdict, compile_catching};

// `#[ignore]` by default: the plain `cargo test` random engine has NO in-process hang protection,
// and this generator is known to be able to emit programs that make the compiler loop (the very
// class of bug the continuous driver's watchdog is built to catch). Running it unguarded in the
// workspace `cargo test` — which the gate/CI invoke — could wedge the suite. So it runs only when
// asked for explicitly:
//   cargo test -p cdz-smith --test fuzz -- --ignored          # bounded random property
//   cargo bolero test cdz_smith_never_panics -p cdz-smith     # coverage-guided (nightly)
// The unattended, timeout-protected fuzzing path is `cdz-smith fuzz` (the cron), not this test.
#[test]
#[ignore = "run explicitly (cargo bolero / --ignored); no in-process hang guard — see comment"]
fn cdz_smith_never_panics() {
    check!().with_type::<Vec<u8>>().for_each(|seed: &Vec<u8>| {
        let program = generate(seed);
        match compile_catching(&program.source) {
            // A crash escaping the compile path is a bug: fail the property so bolero shrinks the
            // seed and reports it.
            Verdict::Crash(info) => panic!(
                "compiler panicked at {}: {}\nprogram:\n{}",
                info.site.as_deref().unwrap_or("<unknown>"),
                info.message.lines().next().unwrap_or(""),
                program.source
            ),
            // Everything else is expected output — not a finding.
            Verdict::Compiled { .. } | Verdict::Declined { .. } | Verdict::ParseError(_) => {}
        }
    });
}
