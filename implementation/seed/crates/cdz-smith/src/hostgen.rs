//! A HOST/EFFECT program generator for WIT/host-boundary DECLINE (gap) hunting.
//!
//! Operator directive (via concierge): bubble declines up to breaker as a corpus gap-inventory, "esp.
//! from WIT/host fuzzing". The [`astgen`](crate::astgen) coercing generator produces valid Int64/compound
//! programs and so declines RARELY; the rich decline/gap surface is the HOST boundary — a program that
//! declares an `(effect …)` and calls its operations through a `(host …)` block. Many op signatures are
//! NOT-yet-emitted increments the compiler cleanly DECLINES (e.g. a compound host RESULT on a bare effect,
//! multi-effect delegation, a higher-order host argument) — exactly the gaps breaker tracks.
//!
//! Shape: `(do (effect e (op o (-> <arg> <ret>))) [ (effect e2 (op o2 …)) ] (def (main) (host (e [e2])
//! <body>)) (export main))`. Every generated program is well-formed and type-correct, so the compiler
//! either COMPILES it (a supported boundary shape) or cleanly DECLINES it (a gap) — never a crash / invalid
//! wasm. Unlike [`astgen`] this is NOT tuned for oracle-comparability (host programs need host-fn modeling);
//! its product is the DECLINE, captured for the breaker hand-off.

use core::fmt::Write as _;

use crate::generator::Program;

/// Host-op ARGUMENT types the generator uses — each with a matching literal the body passes. Kept to
/// shapes with a known literal form (no bytes-literal syntax guessing): `Unit` (no arg), `Int64`, `String`.
/// (A compound / higher-order ARGUMENT is a known gap; RESULT types below cover the compound-crossing gaps.)
const ARG_TYPES: [&str; 3] = ["Unit", "Int64", "String"];

/// Host-op RESULT types — a mix of shapes the bare-effect boundary DOES emit (`Int64`, `Unit`, `String`)
/// and ones it does NOT yet (`Bytes`, `(Tuple Int64 Int64)`, `(List Int64)` → the world-driven path), so
/// the generator straddles the supported/declined frontier and surfaces the compound-result gaps.
const RET_TYPES: [&str; 6] = [
    "Int64",
    "Unit",
    "String",
    "Bytes",
    "(Tuple Int64 Int64)",
    "(List Int64)",
];

/// A minimal byte-cursor over the entropy: yields `0` once spent (so choices bottom out deterministically).
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Cursor<'a> {
        Cursor { bytes, pos: 0 }
    }
    /// A choice in `0..n` (`0` when `n == 0` or entropy is spent).
    fn pick(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let b = self.bytes.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        (b as usize) % n
    }
}

/// The literal a host call passes for an argument of type `arg` (matching [`ARG_TYPES`]); `""` for `Unit`
/// (no argument). Kept to literals with no runtime/store dependency.
fn arg_literal(arg: &str) -> &'static str {
    match arg {
        "Int64" => " 5",
        "String" => " \"x\"",
        _ => "", // Unit: no argument
    }
}

/// Emit one `(effect <name> (op <op> (-> <arg> <ret>)))` declaration into `out`.
fn emit_effect(name: &str, op: &str, arg: &str, ret: &str, out: &mut String) {
    write!(out, "(effect {name} (op {op} (-> {arg} {ret}))) ").ok();
}

/// Coerce arbitrary entropy into a valid HOST/EFFECT program (always well-formed + type-correct). The
/// compiler COMPILES it (supported boundary) or cleanly DECLINES it (a gap) — the decline is the product.
pub fn generate_host(entropy: &[u8]) -> Program {
    let mut c = Cursor::new(entropy);
    let mut source = String::from("(do ");

    // Effect `e` with op `o`.
    let arg = ARG_TYPES[c.pick(ARG_TYPES.len())];
    let ret = RET_TYPES[c.pick(RET_TYPES.len())];
    emit_effect("e", "o", arg, ret, &mut source);

    // Optionally a SECOND effect `e2` — delegating >1 host effect is itself a known gap (declines), and
    // when it is supported this exercises the multi-interface boundary. Only `e2`'s ARG type flows to the
    // body (its result type is consumed by `emit_effect`); `arg2` is `""` (Unit call) when absent.
    let two = c.pick(2) == 1;
    let arg2 = if two {
        let a = ARG_TYPES[c.pick(ARG_TYPES.len())];
        let r = RET_TYPES[c.pick(RET_TYPES.len())];
        emit_effect("e2", "p", a, r, &mut source);
        a
    } else {
        "Unit"
    };

    source.push_str("(def (main) ");
    if two {
        // A two-effect host body — `(do (o …) (p …))` sequences both calls (delegating >1 host effect is
        // itself a known gap that declines; when supported it exercises the multi-interface boundary).
        write!(
            source,
            "(host (e e2) (do (e.o{}) (e2.p{})))",
            arg_literal(arg),
            arg_literal(arg2)
        )
        .ok();
    } else {
        write!(source, "(host (e) (e.o{}))", arg_literal(arg)).ok();
    }
    source.push_str(") (export main))");
    Program { source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{Verdict, compile_catching};

    /// ANY entropy coerces to a well-formed host/effect program the compiler CLEANLY handles — it either
    /// COMPILES (a supported boundary shape) or DECLINES (a gap) — never a crash / invalid wasm / parse
    /// error. This is the decline-hunting invariant (the decline is the product, not a failure).
    #[test]
    fn any_entropy_is_a_cleanly_handled_host_program() {
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut bytes = Vec::new();
            for _ in 0..12 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = generate_host(&bytes);
            assert!(
                program.source.starts_with("(do (effect e ")
                    && program.source.ends_with("(export main))"),
                "shape: {}",
                program.source
            );
            let verdict = compile_catching(&program.source);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "host program must be cleanly handled (Compiled/Declined), got {verdict:?} for: {}",
                program.source
            );
        }
    }

    /// The generator REACHES both outcomes across varied entropy: at least one COMPILES (a supported
    /// boundary, e.g. an Int64 result) and at least one DECLINES (a gap, e.g. a compound result) — so a
    /// decline campaign over this generator actually surfaces host-boundary gaps.
    #[test]
    fn reaches_both_compiled_and_declined_host_shapes() {
        let mut saw_compiled = false;
        let mut saw_declined = false;
        for seed in 0u64..256 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(7);
            let mut bytes = Vec::new();
            for _ in 0..12 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            match compile_catching(&generate_host(&bytes).source) {
                Verdict::Compiled { .. } => saw_compiled = true,
                Verdict::Declined { .. } => saw_declined = true,
                _ => {}
            }
        }
        assert!(saw_compiled, "some host shape should COMPILE (a supported boundary)");
        assert!(saw_declined, "some host shape should DECLINE (a gap) — the point of the generator");
    }
}
