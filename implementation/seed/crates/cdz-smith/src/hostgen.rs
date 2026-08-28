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
//! <body>)) (export main))`. The op arg/result types come from a FULLY-ALGEBRAIC WIT-type generator
//! ([`gen_wit`]) — composed RECURSIVELY over the WIT type constructors (scalars + list/tuple/option/record,
//! arbitrarily nested), NOT a hard-coded shape list (operator directive on #4924) — so the fuzzer explores
//! the whole WIT shape space. Every generated program is well-formed + type-correct, so the compiler
//! COMPILES it (a supported boundary) or cleanly DECLINES it (a "not this slice" gap) — never a crash /
//! invalid wasm. Unlike [`astgen`] this is NOT tuned for oracle-comparability (host programs need host-fn
//! modeling); its product is the DECLINE, captured for the breaker hand-off. The gradeable Unit-effect
//! subset ([`generate_host_unit_effect`]) instead feeds the VALUE differential (v-lean-oracle H1a).

use core::fmt::Write as _;

use crate::generator::Program;

/// Max nesting depth of a composed WIT type — bounds program size while allowing arbitrary nestings.
const MAX_TYPE_DEPTH: usize = 3;

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

/// A generated WIT type in three consistent forms: the Cadenza source type `ty` (e.g. `Int64`,
/// `(List Int64)`), the WIT-LEVEL type `wit` (e.g. `s64`, `(list s64)` — the vocabulary a `(world …)`
/// interface member uses), and a matching unambiguous VALUE literal `lit` (for a host-call argument).
struct WitType {
    ty: String,
    wit: String,
    lit: String,
}

/// FULLY-ALGEBRAIC WIT-type generator (operator directive on #4924: "generate arbitrary types … make the
/// generator fully algebraic — NOT a hard-coded list"). Composes the WIT type algebra RECURSIVELY over
/// its constructors so the fuzzer explores the whole shape space, not a curated catalog:
/// * scalars — `Int64` / `Bool` / `String` / `Unit` (each with a value literal);
/// * `(List T)` / `(Tuple T U)` / `(Option T)` / `(Record (: a T) (: b U))` — arbitrarily NESTED to
///   `MAX_TYPE_DEPTH`, each with a matching literal (`(list …)` / `(tuple …)` / `(Some …)` /
///   `(record (= a …) (= b …))`).
///
/// Deliberately unambiguous LITERALS only (so a value type-infers, reaching the compiler as a decline
/// rather than a spurious inference error): `(Some v)` not bare `(None)`. Constructors whose bare value
/// needs a type annotation to infer (`Result`/`Variant`/`Enum`/`Flags`) are a follow-up — they slot in as
/// more `gen_wit` arms once value generation for them is settled. Types with no WIT equivalent
/// (BigInt/Rational/Map/Set) stay out of scope per the same ruling.
fn gen_wit(c: &mut Cursor, depth: usize) -> WitType {
    // At depth 0 only scalars (4 arms); deeper also the compound constructors (4 more).
    let compound = if depth == 0 { 0 } else { 4 };
    match c.pick(4 + compound) {
        0 => WitType {
            ty: "Int64".into(),
            wit: "s64".into(),
            lit: int_literal(c),
        },
        1 => WitType {
            ty: "Bool".into(),
            wit: "bool".into(),
            lit: (if c.pick(2) == 0 { "true" } else { "false" }).into(),
        },
        2 => WitType {
            ty: "String".into(),
            wit: "string".into(),
            lit: "\"x\"".into(),
        },
        3 => WitType {
            ty: "Unit".into(),
            wit: "unit".into(),
            lit: "unit".into(),
        },
        // (List T)
        4 => {
            let e = gen_wit(c, depth - 1);
            WitType {
                ty: format!("(List {})", e.ty),
                wit: format!("(list {})", e.wit),
                lit: format!("(list {} {})", e.lit, e.lit),
            }
        }
        // (Tuple T U)
        5 => {
            let a = gen_wit(c, depth - 1);
            let b = gen_wit(c, depth - 1);
            WitType {
                ty: format!("(Tuple {} {})", a.ty, b.ty),
                wit: format!("(tuple {} {})", a.wit, b.wit),
                lit: format!("(tuple {} {})", a.lit, b.lit),
            }
        }
        // (Option T) — `(Some v)` (unambiguous); bare `(None)` would need an annotation.
        6 => {
            let e = gen_wit(c, depth - 1);
            WitType {
                ty: format!("(Option {})", e.ty),
                wit: format!("(option {})", e.wit),
                lit: format!("(Some {})", e.lit),
            }
        }
        // (Record (: a T) (: b U)) — WIT record fields are `(field wit-ty)` (no `:`, no `=`).
        _ => {
            let a = gen_wit(c, depth - 1);
            let b = gen_wit(c, depth - 1);
            WitType {
                ty: format!("(Record (: a {}) (: b {}))", a.ty, b.ty),
                wit: format!("(record (a {}) (b {}))", a.wit, b.wit),
                lit: format!("(record (= a {}) (= b {}))", a.lit, b.lit),
            }
        }
    }
}

/// An edge-biased `Int64` literal (boundaries where width/marshalling bugs cluster).
fn int_literal(c: &mut Cursor) -> String {
    format!("{}", HOST_INT_BOUNDARIES[c.pick(HOST_INT_BOUNDARIES.len())])
}

/// Coerce arbitrary entropy into a valid HOST/EFFECT program (always well-formed + type-correct), with
/// op signatures drawn from the FULLY-ALGEBRAIC WIT-type generator [`gen_wit`] — so the compiler COMPILES
/// it (a supported boundary shape) or cleanly DECLINES it (a "not this slice" gap); the decline is the
/// product. Optionally emits a second effect (multi-interface delegation is itself a tracked gap).
pub fn generate_host(entropy: &[u8]) -> Program {
    let mut c = Cursor::new(entropy);
    let mut source = String::from("(do ");

    // Effect `e` with op `o`: an arbitrary WIT arg type + an arbitrary WIT result type.
    let arg = gen_wit(&mut c, MAX_TYPE_DEPTH);
    let ret = gen_wit(&mut c, MAX_TYPE_DEPTH);
    write!(source, "(effect e (op o (-> {} {}))) ", arg.ty, ret.ty).ok();

    let two = c.pick(2) == 1;
    let arg2 = if two {
        let a = gen_wit(&mut c, MAX_TYPE_DEPTH);
        let r = gen_wit(&mut c, MAX_TYPE_DEPTH);
        write!(source, "(effect e2 (op p (-> {} {}))) ", a.ty, r.ty).ok();
        Some(a)
    } else {
        None
    };

    // The perform passes the arg's value literal; a `Unit` arg takes no value (`(e.o)`), matching the
    // corpus `(-> Unit …)` call shape. `main` is the perform (or a `do`-sequence of both) → its value.
    let call = |name: &str, a: &WitType| {
        if a.ty == "Unit" {
            format!("({name})")
        } else {
            format!("({name} {})", a.lit)
        }
    };
    source.push_str("(def (main) ");
    match &arg2 {
        Some(a2) => {
            write!(
                source,
                "(host (e e2) (do {} {}))",
                call("e.o", &arg),
                call("e2.p", a2)
            )
            .ok();
        }
        None => {
            write!(source, "(host (e) {})", call("e.o", &arg)).ok();
        }
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

/// Coerce entropy into a MODULE library + importing ENTRY pair for the CROSS-MODULE WIT-binding decline
/// surface (via [`crate::oracle::compile_modules_catching`]). The module `lib` exports an identity
/// `(def (f (: x T)) x)` over a FULLY-ALGEBRAIC WIT type `T` ([`gen_wit`]); the entry imports `f` and
/// calls it with a matching `T` literal — so `T` crosses the module link (as both the param and the
/// return). A not-yet-emitted cross-module type-crossing cleanly DECLINES (a WIT-binding gap); a supported
/// one compiles. Returns `(module_src, entry_src)`.
pub fn generate_module_program(entropy: &[u8]) -> (String, String) {
    let mut c = Cursor::new(entropy);
    let t = gen_wit(&mut c, MAX_TYPE_DEPTH);
    let module_src = format!("(do (def (f (: x {})) x) (export f))", t.ty);
    let entry_src = format!(
        "(do (import \"lib\" (f)) (def (main) (f {})) (export main))",
        t.lit
    );
    (module_src, entry_src)
}

/// Coerce entropy into a WIT-WORLD guest + world pair for the per-cell WIT-BINDING decline surface (via
/// [`crate::oracle::compile_world_catching`]). The world declares interface `iface` with one member `f`
/// of an ARBITRARY WIT type `T` ([`gen_wit`]'s `wit` form): `(world w (export iface (member f (func
/// (param m <wit>) (result <wit>)))))`; the guest is the IDENTITY over `T`'s Cadenza type: `(module m
/// (def (f (: m <ty>)) m) (export f))`. So `T` crosses the WIT ABI boundary as both param + result — a
/// WIT type the boundary does not yet marshal cleanly DECLINES (a per-cell gap); a supported one compiles.
/// Returns `(guest_src, iface, world_src)` for `compile_world_catching(guest_src, iface, world_src)`.
pub fn generate_world_program(entropy: &[u8]) -> (String, String, String) {
    let mut c = Cursor::new(entropy);
    let t = gen_wit(&mut c, MAX_TYPE_DEPTH);
    let world_src = format!(
        "(world w (export iface (member f (func (param m {}) (result {})))))",
        t.wit, t.wit
    );
    let guest_src = format!("(module m (def (f (: m {})) m) (export f))", t.ty);
    (guest_src, "cadenza:demo/iface".to_string(), world_src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{
        Verdict, compile_catching, compile_modules_catching, compile_world_catching,
    };

    /// Every `generate_world_program` triple is CLEANLY HANDLED by the wit-world oracle — it COMPILES (a
    /// WIT type the ABI boundary marshals) or cleanly DECLINES (a per-cell WIT-binding gap) — never a
    /// crash / invalid wasm / parse error. The decline-hunting invariant for the WIT-world surface.
    #[test]
    fn world_programs_are_cleanly_handled() {
        for seed in 0u64..96 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(17);
            let mut bytes = Vec::new();
            for _ in 0..12 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let (guest, iface, world) = generate_world_program(&bytes);
            let verdict = compile_world_catching(&guest, &iface, &world);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "world program must be cleanly handled, got {verdict:?}\nworld: {world}\nguest: {guest}"
            );
        }
    }

    /// Every `generate_module_program` pair is CLEANLY HANDLED by the multi-module oracle — it COMPILES
    /// (a cross-module type-crossing the linker supports) or cleanly DECLINES (a WIT-binding gap) — never
    /// a crash / invalid wasm / parse error. The decline-hunting invariant for the cross-module surface.
    #[test]
    fn module_programs_are_cleanly_handled() {
        for seed in 0u64..128 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(11);
            let mut bytes = Vec::new();
            for _ in 0..12 {
                x ^= x >> 30;
                x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                bytes.push((x >> 24) as u8);
            }
            let (module_src, entry_src) = generate_module_program(&bytes);
            let verdict = compile_modules_catching(&[("lib".to_string(), module_src)], &entry_src);
            assert!(
                matches!(verdict, Verdict::Compiled { .. } | Verdict::Declined { .. }),
                "module program must be cleanly handled, got {verdict:?} for entry: {entry_src}"
            );
        }
    }

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
