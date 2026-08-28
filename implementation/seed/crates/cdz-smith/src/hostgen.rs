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

/// Host-op ARGUMENT types — each with a matching literal ([`arg_literal`]) the body passes. Spans the
/// scalar shapes that cross (`Unit`, `Int64`, `String`), flat compound / higher-order shapes that are
/// KNOWN gaps, AND NESTED/wider compounds (`(List (List Int64))`, `(Tuple Int64 (List Int64))`) — the
/// "later increment" slices the boundary model calls out — so the arg surface straddles the frontier
/// across distinct "not this slice" faces.
const ARG_TYPES: [&str; 8] = [
    "Unit",
    "Int64",
    "String",
    "(Tuple Int64 Int64)",
    "(List Int64)",
    "(-> Int64 Int64)",
    "(List (List Int64))",
    "(Tuple Int64 (List Int64))",
];

/// Host-op RESULT types — scalar shapes the bare-effect boundary DOES emit (`Int64`, `Unit`, `String`),
/// flat compounds it does NOT yet (`Bytes`, `(Tuple Int64 Int64)`, `(List Int64)` → world-driven path),
/// and NESTED/wider compounds (`(List (List Int64))`, `(List (Tuple Int64 Int64))`, `(Tuple Int64 Int64
/// Int64)`, `(Tuple Int64 (List Int64))`) — the explicitly-named `list<list<…>>`/`list<tuple<…>>` "later
/// increment" result slices — so the generator surfaces the full compound-result gap ladder.
const RET_TYPES: [&str; 10] = [
    "Int64",
    "Unit",
    "String",
    "Bytes",
    "(Tuple Int64 Int64)",
    "(List Int64)",
    "(List (List Int64))",
    "(List (Tuple Int64 Int64))",
    "(Tuple Int64 Int64 Int64)",
    "(Tuple Int64 (List Int64))",
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
        "(Tuple Int64 Int64)" => " (tuple 1 2)",
        "(List Int64)" => " (list 1 2 3)",
        "(-> Int64 Int64)" => " (fn (x) x)",
        "(List (List Int64))" => " (list (list 1 2) (list 3 4))",
        "(Tuple Int64 (List Int64))" => " (tuple 1 (list 2 3))",
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

/// Boundary Int64 literals for the perform ARGUMENT — where arg-marshalling width/wrap bugs cluster.
const HOST_INT_BOUNDARIES: [i64; 8] = [0, 1, -1, i64::MAX, i64::MIN, 127, -128, 4_294_967_295];

/// Coerce entropy into a GRADEABLE Unit-effect host program — the exact shape the H1a oracle value-grades:
/// `(do (effect e (op o (-> <arg> Unit))) (def (main) (host (e) (e.o <arg-lit>))) (export main))` where the
/// perform IS the whole `main` body (so `main : Unit`) and `<arg>` is `Unit` or `Int64` (a heap-free scalar
/// — `String`/`Bytes` args need the value-heap store the differential runs without). rcdzc runs it to
/// `unit` and the oracle grades it `unit` → the L2 differential VALUE-checks host-perform lowering (a
/// non-unit / trapping / crashing lowering would diverge). Int64 args are edge-biased to stress the
/// perform's arg-marshalling path across width boundaries.
pub fn generate_host_unit_effect(entropy: &[u8]) -> Program {
    let mut c = Cursor::new(entropy);
    // Arg type: Unit (no arg) or Int64 (an edge-biased literal).
    let (arg_ty, arg_lit) = if c.pick(2) == 0 {
        ("Unit", String::new())
    } else {
        let n = HOST_INT_BOUNDARIES[c.pick(HOST_INT_BOUNDARIES.len())];
        ("Int64", format!(" {n}"))
    };
    let source = format!(
        "(do (effect e (op o (-> {arg_ty} Unit))) (def (main) (host (e) (e.o{arg_lit}))) (export main))"
    );
    Program { source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{Verdict, compile_catching};

    /// Every `generate_host_unit_effect` program COMPILES to a value (the gradeable Unit-effect shape rcdzc
    /// lowers + runs to `unit`) — so a Lean value-differential campaign over it actually GRADES (not
    /// declines/skips). Guards that the arg-marshalling variants (Unit + edge Int64) all stay compilable.
    #[test]
    fn host_unit_effect_programs_compile() {
        for seed in 0u64..64 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(3);
            let mut bytes = Vec::new();
            for _ in 0..8 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let program = generate_host_unit_effect(&bytes);
            assert!(
                matches!(compile_catching(&program.source), Verdict::Compiled { .. }),
                "gradeable Unit-effect host program must compile: {}",
                program.source
            );
        }
    }

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
