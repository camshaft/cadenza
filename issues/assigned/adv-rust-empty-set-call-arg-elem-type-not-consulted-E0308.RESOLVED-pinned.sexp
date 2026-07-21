; BREAKER FINDING 2026-07-20 (trunk 5a08db2c5, store b020075f) — RUST-BACKEND-ONLY BUILD FAILURE
; (differential: wasm computes, rust artifact does not build, E0308):
; an EMPTY `(Set.of (list))` passed as a CALL ARGUMENT whose parameter type fixes a NON-Int64
; element (e.g. `(: s (Set Float64))`) emits the DEFAULT `BTreeSet<i64>`:
;
;   r#loop(n, { let mut __s: BTreeSet<i64> = BTreeSet::new(); __s })
;                                ^^^ expected BTreeSet<__CdzF64>, found BTreeSet<i64>
;
; The earlier unconstrained-empty-Set E0282 fix (10926) grounds the open element var to the default
; and annotates `BTreeSet::<i64>` "unless an enclosing Set.insert/remove fixes it" — but a CALL-
; ARGUMENT position (the callee's declared param type) is not consulted, so a typed-but-non-default
; empty set mis-grounds. wasm handles all these correctly (runs, right answers).
;
; ISOLATION (all verified on both backends):
;   (Set.len (Set.insert (Set.of (list)) x))  x:Float64, non-recursive   -> BOTH OK (insert fixes it)
;   helper (add1 (: s (Set Float64)) ...) called with (Set.of (list))    -> BOTH OK
;   recursive (loop (: s (Set Float64))) seeded (Set.of (list 1.5 2.5))  -> BOTH OK (non-empty seed)
;   recursive fn, EMPTY seed, Float64 elem — NO insert at all            -> wasm OK / rust E0308  ✗
;   recursive fn, EMPTY seed, Float64 elem, const insert in body         -> wasm OK / rust E0308  ✗
;   recursive fn, EMPTY seed, INT elem (control)                         -> BOTH OK (default is right)
; So the trigger is: empty Set.of in a RECURSIVE call's argument position + a non-Int64 declared
; element type. (The non-recursive helper works — possibly inlined so the insert/param fixer sees it;
; the recursive callee's param type is never consulted.)
;
; NOTE: found while probing the FRESH Float to-list fix (d768d3625) — this blocks writing the
; "runtime-BUILT float set enumerates in canonical byte order" corpus case with an empty seed on
; rust; the same shape works for (Set Int64) (19-sets:805 uses exactly this ins-loop with an empty
; seed and an Int64 element).
;
; SEVERITY: not a miscompile (fails to BUILD, loudly) but a genuine backend gap on an idiomatic
; shape (build-a-set-by-recursion with an empty seed), and it blocks float-set corpus coverage.
; Same family as the fixed rust-backend-unconstrained-empty-set-E0282 — extend the grounding to
; consult the call-argument / declared-param type.
;
; Expected: both cases below compile+run on rust as they do on wasm.
(case "an empty set passed to a recursive callee with a Float64 element parameter builds on rust"
  (doc    "`(loop n (Set.of (list)))` where loop's param is `(: s (Set Float64))` — the empty set's
           element type is fixed by the callee's declared parameter, so the rust emit must ground
           BTreeSet<__CdzF64>, not the i64 default. No insert anywhere: the param type is the only
           fixer. wasm runs this (0); rust must build it. Expected: 0.")
  (input  (do
            (def (loop (: n Int64) (: s (Set Float64)))
              (if (< n 1) (Set.len s) (loop (- n 1) s)))
            (def (main (: n Int64))
              (loop n (Set.of (list))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 0 Int64)))

(case "a recursive float-set builder seeded with the empty set builds on rust"
  (doc    "The build-by-recursion idiom over floats: `(ins n (Set.of (list)))` with ins inserting a
           Float64 per level. The Int64 twin (19-sets:805) works on both backends; the Float64 elem
           must too. wasm runs it (dedup to 1 element); rust currently E0308s at the empty seed.
           Expected: 1.")
  (input  (do
            (def (ins (: n Int64) (: s (Set Float64)))
              (if (< n 1) s (ins (- n 1) (Set.insert s 1.5))))
            (def (main (: n Int64))
              (Set.len (ins n (Set.of (list)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1 Int64)))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk 5a08db2c5) — VERIFIED, routed v-rust-backend =====
; CONFIRMED: no-insert case — rust emit has BOTH BTreeSet<__CdzF64> (from param) AND BTreeSet<i64> (empty
; Set.of(list()) call-arg default) -> E0308 (5 rustc hits). wasm 0. Same family as fixed E0282 10926, but
; call-ARG position not consulted. ISOLATION add (me): the LIST twin (loop(n,(list)) : (List Float64))
; BUILDS FINE (Vec<f64>) -> SET-SPECIFIC, not a general empty-collection hole. run-rust masks as "declined"
; but GATE build is hard E0308 FAIL -> NOT pinnable-as-declines until fixed. Routed v-rust-backend
; (extend empty-Set grounding to consult callee param type at call-arg position). Pin both cases on land.

; ===== FIX QUEUED (v-rust-backend, 2026-07-21) — MR 9a9448f95, PIN PLAN ready =====
; FIXED + queued to pr-sync (9a9448f95): rust/rust-async/wasm all 0-fail, +1 rcdzc regression test, NO
; baseline flip (no corpus case witnesses it yet — that's my pin). NOT landed yet (still emits BTreeSet<i64>
; on trunk 2bc0ba7ea). PIN PLAN (corpus-bugfix, on land): pin the Set case + Map twin, both expect value 0 on
; ALL backends (was rust-E0308):
;   (module m (def (loop (: n Int64) (: s (Set Float64))) (if (= n 0) (Set.len s) (loop (- n 1) s)))
;             (def (run) (loop 3 (Set.of (list)))) (export run))  -> 0
;   Map twin: same shape with (Map Float64 Int64) param + Map.empty seed -> 0
; NB: (module …)-root cases need a LIB test not the do-wrapping corpus harness (per my memory trap
; [[gate-corpus-harness-do-wraps-every-input-cannot-test-module-root]]) — so author these as (do (def …)
; (export run)) with a nullary run, gradeable on all 3 backends. WATCH: pin the moment 9a9448f95 lands.
