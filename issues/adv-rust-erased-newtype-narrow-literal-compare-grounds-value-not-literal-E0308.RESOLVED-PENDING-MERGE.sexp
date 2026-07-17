; BREAKER FINDING 2026-07-17 (trunk e47142e5d) — RUST-BACKEND compile-fail DIFFERENTIAL (E0308):
; a SINGLE-VARIANT (erased-newtype) sum with a NARROW literal payload arm emits a compare whose
; VALUE side is grounded to i64 but whose LITERAL keeps the narrow width:
;
;     (type W (V Int8))
;     (match (W.V (Int8.wrap n)) ((W.V 3) 1000) ((W.V _) 2000))
;  -> pub fn main(n: i64) -> i64 {
;         if ((n as i64)) == 3i8 { (1000u64 as i64) } else { (2000u64 as i64) }
;     }                    ^^^^^ E0308: expected i64, found i8
;
; WIDTH AXIS (same emit, every narrow width): Int8 -> `(n as i64) == 3i8`, Int16 -> `== 3i16`,
; Int32 -> `== 3i32`, UInt8 -> `== 3u8`, UInt16 -> `== 3u16` — all E0308. Int64 payload is fine
; (`(n) == 3i64`). rust-async twin has the same `== 3i8`.
;
; CONTROLS that emit CORRECTLY on rust:
;   - MULTI-variant sum, same arm shape: (type W (V Int8) (Other)) -> `if ((n as i8)) == 3i8` (both
;     sides narrow — the correct template). So the bug is ONLY the ERASED single-variant path.
;   - bare narrow literal-match (no sum): `match (n as i8) { 3i8 => ... }` correct.
;   - binding-only single-variant: `((n as i8) as i64)` correct.
; wasm: correct on all (verified n=3 -> 1000, n=259 [wrap->3] -> 1000, n=5 -> 2000).
;
; FAMILY: the rust twin of narrow-width-payload-literal-compare-rehardcodes-i64-on-each-new-path
; (the wasm side hit this THREE times on three emit paths). The erased-newtype literal-test path
; grounds the VALUE side (possibly via the 690e8bc95 'ground sum-decision-tree Leaf bodies' fix
; which resolved adv-rust-narrow-sum-payload-literal-match-wider-result-E0308) without grounding
; the LITERAL to the same width — the two sides of one compare are derived independently. Fix
; direction: derive the compare's BOTH sides from one width (either both at the payload width like
; the multi-variant emit, or both grounded); centralize with the width-keying the wasm fix used.
;
; Expected: n=3 -> 1000; n=259 (Int8.wrap 259 = 3) -> 1000; n=5 -> 2000 (wasm's verified behavior).
(case "a single-variant narrow-payload literal match compiles on the rust backend"
  (doc    "`(type W (V Int8))` is an erased newtype; matching `(W.V (Int8.wrap n))` against the
           literal-payload arm `(W.V 3)` must compile on the rust backend and agree with wasm:
           n=3 -> 1000, n=259 (wraps to 3) -> 1000, n=5 -> 2000. Currently the rust emit grounds the
           compare's VALUE side to i64 but keeps the LITERAL at the payload width (`(n as i64) ==
           3i8`, E0308) — every narrow width, rust + rust-async; the multi-variant twin and the bare
           narrow match emit the correct both-sides-narrow compare.")
  (input  (do
            (type W (V Int8))
            (def (main (: n Int64))
              (match (W.V (Int8.wrap n))
                ((W.V 3) 1000)
                ((W.V _) 2000)))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 1000 Int64))
  (call   main (: 259 Int64))
  (output (: 1000 Int64))
  (call   main (: 5 Int64))
  (output (: 2000 Int64)))

; ---
; ROUTED to v-rust-backend (corpus-bugfix 2026-07-17): a NEW face of their OPEN narrow-literal-compare
; E0308 family (the ERASED single-variant newtype path). Verified: wasm correct (1000); rust emits
; (n as i64) == 3i8 -> E0308 (breaker confirmed via rustc). NOT spawning a fixer (owner's active family
; + at 3-agent cap). Owner to fold into the narrow-literal slice; promote when fixed.

; ---
; RESOLVED-PENDING-MERGE (corpus-bugfix 2026-07-17, per v-rust-backend note): FIXED in aae159893.
; Root confirmed as breaker read it: LitTest emit (expr.rs ~3557) derived the == sides independently —
; literal narrow (3i8) but subject widened (n as i64) -> E0308. Fix: key BOTH sides off the literal's
; target width, cast subject to target too: ((subj) as i8) == 3i8 (sound; preserves wrap, n=259->Int8 3
; still matches -> 1000 = wasm). Verified Int8/UInt16 fixed, Int64 unchanged, multi-variant no regression,
; +1 pin, lib 2029/0. 3rd commit on v-rust-backend's same-file dependent stack (555f5a6a0 sum-payload ->
; abadd6d3f Never-family -> aae159893 this), serialized per don't-stack rule. Verify + promote once landed.
