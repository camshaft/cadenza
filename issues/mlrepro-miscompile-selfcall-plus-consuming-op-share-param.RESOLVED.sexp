;; ✅ FIXED (2026-07-15, seed rcdzc). `main 0` now returns 3. ROOT CAUSE was NOT the self-call arg emit as
;; originally guessed — it is a GENERAL Perceus RETAIN gap: a heap binding/param CONSUMED (by `List.push`/
;; `Map.insert`/a call arg) while it has a LATER live use on the same eval path got no `dup`, so the
;; consuming op's in-place FBIP reuse (uniquely-owned handle) mutated the value the later use read. It
;; reproduces in STRAIGHT-LINE non-recursive code too (`(let ((e L)) (+ (List.len (List.push e 9))
;; (List.len e)))`), so it was never self-call-specific. FIX: `backend/wasm/select.rs` `collect_dup_sites`/
;; `mark_binder_dups` marks each consuming occurrence with a later use, and `emit_binder_ref` emits a `dup`
;; there (the FBIP single-use fast path is untouched — no later use ⇒ no dup). Corpus witnesses in
;; `spec/semantics/05-compound-types.sexp` ("a runtime list/map/set … consumed … is unchanged/not
;; mutated"). KEPT as a regression repro; run `cdz compile` and check `main 0` == 3.
;;
;; ORIGINAL REPORT (2026-07-14, seed rcdzc). MINIMAL, SHARPENED form of the
;; shared-recursive-param collection-mutation bug (`miscompile-map-insert-mutates-shared-recursive-param`
;; is the interpreter-shaped version). `cdz check` CLEAN; `cdz compile` SUCCEEDS; runs WRONG.
;;
;; A parameter that is (a) consumed by a persistent-collection op (`List.push`/`Map.insert`/`Set.insert`)
;; in ONE operand AND (b) passed to a SELF-RECURSIVE call of the SAME function in a SIBLING operand of the
;; same strict expression, is MUTATED by the consuming op — the self-call then sees the corrupted handle.
;;
;; `f 1 [7]` should be `len([7,9]) + len([7])` = 2 + 1 = 3 (the `List.push xs 9` builds a NEW list; the
;; self-call `(f 0 xs)` reads the ORIGINAL `xs = [7]`). It returns 4 (= 2 + 2): `List.push` consumed the
;; shared `xs`, so the self-call's `xs` is `[7,9]`, len 2.
;;
;; SHARP BISECTION (2026-07-14):
;;   - The SAME shape with two SEPARATE callees (`(+ (g 1 xs) (g 0 xs))`, `g` a distinct fn, one path
;;     pushing) computes CORRECTLY (3) — the caller dups `xs` for the two calls.
;;   - A single NON-recursive fn using the param consuming + borrowing (`(+ (len (push xs 9)) (len xs))`)
;;     computes CORRECTLY (3).
;;   - A TAIL self-recursive loop that consumes one param and leaves another untouched computes correctly.
;;   - Only a SELF-CALL of the SAME fn, sharing a param with a consuming op in the same expression, fails.
;; So the trigger is the SELF-CALL path specifically (lowered via the self-recursion loop-transform): its
;; arg-passing does NOT `dup` a param that a sibling operand consumes — the loop back-edge / self-call
;; overwrites the param slot with the consumed (mutated) handle. An ORDINARY call to a different fn dups
;; correctly; the self-call path is the gap. Fix locus: the self-call arg emit in the loop-transform
;; (`backend/wasm/select.rs`) must dup a param arg that is also consumed by a sibling op (the same
;; caller-side dup an ordinary `Core::Call` gets). Reproduces identically for `Map.insert`/`Set.insert`.
(do
  (def (f (: n Int64) (: xs (List Int64)))
    (if (= n 0)
      (List.len xs)
      (+ (List.len (List.push xs 9)) (f 0 xs))))
  (def (main (: d Int64)) (f 1 (List.push (list) 7)))
  (export main))

;; RESOLVED 2026-07-15 (trunk@1f3b3c348): file self-annotated ✅ FIXED (still-live-binding family closed).
