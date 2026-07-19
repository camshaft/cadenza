# MISCOMPILE (wrong value): rust const-fold (Int N).wrap does NOT sign-extend for UNUSUAL widths (Int4/Int12)

From breaker 2026-07-19. WRONG-VALUE miscompile on the rust backend — HIGH severity (violates a pinned semantic).
Reproduced by corpus-bugfix (near-trunk build, HEAD 160 behind but design confirms still-present on trunk).

## Repro matrix (const-fold path)
- `(def (main) ((. (Int 4) wrap) 8))` → wasm -8 (correct; 8=0b1000, bit3=sign → 8-16), rust +8 (WRONG)
- Int4.wrap 15 → wasm -1, rust 15 (WRONG)
- Int12.wrap 2048 → wasm -2048, rust 2048 (WRONG)
- Int12.wrap 4095 → wasm -1, rust 4095 (WRONG)
Emitted rust: `pub fn main() -> i8 { (8u8 as i8) }` — casts at the i8 STORAGE width (8 bits) instead of
sign-extending from the declared 4-bit top bit (bit N-1).

## Root
rust/expr.rs Convert(.wrap) emits an `as iN` cast. The module doc (backend/rust/expr.rs:8-22) claims `.wrap`
via `as` is "bit-identical to IntValue::wrap_to" — TRUE for a MACHINE width (i8/i16/i32/i64) but FALSE for a
NON-machine declared width (Int4/Int12): `as i8` reinterprets at the 8-bit storage width, so bit-3 of an Int4
is NOT treated as the sign bit → no sign-extend from the declared width.

## Scope (breaker, confirmed)
1. CONST-FOLD path ONLY — the RUNTIME path is CORRECT (pinned :4470 "a runtime SIGNED nibble truncation
   sign-extends" PASSES on rust with -8/7/-1). rust const-fold and rust runtime DISAGREE.
2. UNUSUAL widths ONLY — Int8.wrap 128=-128, Int16.wrap 32768=-32768 correct on both (machine widths).
HIGH severity: the corpus PINS (Int 4).wrap 8 = -8 (:4470, runtime), but the const path silently gives +8 —
a wrong value violating a pinned semantic, INVISIBLE to the runtime-only pin.

## Fix direction
rust const-fold wrap_to(signed, N) for a non-machine N must SIGN-EXTEND from bit N-1 (match IntValue::wrap_to
/ the rust runtime path) — e.g. emit `((v << (STORAGE-N)) >> (STORAGE-N))` at the storage type, or compute the
wrapped constant in the const-folder (IntValue::wrap_to) and emit the already-correct literal. A const-fold +
runtime differential pin at an unusual width would guard it.

## Routing
implementation/seed/crates/rcdzc/src/backend/rust/* = v-rust-backend. ROUTED. Wrong-value miscompile, fix
proactively (top severity). VERIFIED repro + trunk design confirms still-present.

---
## REFINEMENT (breaker, 2026-07-19) — UNSIGNED wrap is CORRECT, isolates the bug to the SIGN-EXTEND step
The UNSIGNED companion is CORRECT on rust const-fold: (UInt 4).wrap 8=8, 15=15, 16=0, 17=1; (UInt 12).wrap
4096=0 — all wasm==rust. So the low-bits MASK is done right (unsigned proves it); the bug is PRECISELY the
signed-only sign-extend that follows: "if bit N-1 set, subtract 2^N". rust const-fold omits that
reinterpretation for signed NON-MACHINE widths (unsigned wrap_to already correct; IntValue::wrap_to on
wasm + rust-RUNTIME both do the sign-extend — only the rust CONST-FOLD path skips it). FIX is narrow: after
the mask, signed wrap_to(N) for non-machine N sign-extends from bit N-1. Forwarded to v-rust-backend.

---
FIXED by v-rust-backend (MR 41946d40a, "rcdzc rust-backend: const-fold .wrap to a signed UNUSUAL width
sign-extends (fix a wrong-value miscompile)"), PENDING MERGE (corpus-bugfix 2026-07-19). emit_const_int_at now
emits the true signed decimal for signed non-machine widths (lower already folded the correct value via
wrap_to; only the EMIT dropped the sign). All cases verified matching wasm: (Int 4).wrap 8→-8/15→-1, (Int 12).wrap
2048→-2048/4095→-1. Added the const+runtime differential pin at unusual widths (my suggestion). MR real (cites
the wrong-value miscompile), not yet on trunk. Tracked-to-close on land; content-confirm the signed emit + pin.
Renamed .RESOLVED-PENDING-MERGE.
