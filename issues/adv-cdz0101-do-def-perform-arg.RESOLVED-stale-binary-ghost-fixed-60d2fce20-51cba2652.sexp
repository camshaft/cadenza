; FINDING (breaker, 2026-07-24): a `do`-def local is CDZ0101 "unbound name" when referenced
; from the ARGUMENT of a perform inside a handle body — a FALSE REJECT. The semantically
; identical `let`-bound form compiles and computes CORRECTLY on all 3 targets (wasm/rust/
; rust-async), so this is a frontend/desugar scoping gap in `do` bodies under `handle`,
; specific to the perform-argument position.
;
; MATRIX (all minimal, scalar-only — no heap involvement needed):
;   ✗ (do (def v (+ u 2)) (Bail.bail v))            — abortive, do-def in perform arg    → CDZ0101 unbound v
;   ✗ (do (def v (+ u 2)) (+ (Ask.ask v) 1))        — resuming, same                     → CDZ0101
;   ✗ (do (def v (+ u 2)) (+ (Ask.ask v) v))        — do-def in arg AND after            → CDZ0101
;   ✗ (do (def rope (rep …)) (Bail.bail (String.byte-len rope)))  — heap do-def in arg   → CDZ0101
;   ✗ (do (def v (+ u 2)) (+ (poke v) 1)) where (def (poke (: v Int64)) (Ask.ask v))
;         — do-def passed to a HELPER that performs                                       → CDZ0101
;   ✓ (let ((v (+ u 2))) (+ (Ask.ask v) v))          — LET-bound, same shape             → computes 21, all 3 targets
;   ✓ (do (def v (+ u 2)) (+ (Ask.ask 3) (twice v))) — do-def in a NON-perform arg beside a perform → OK
;   ✓ (+ (Ask.ask u) 1)                              — PARAM in perform arg              → OK
;   ✓ (do (def rope (rep …)) (+ (Bail.bail 7) (byte-len rope))) — do-def AFTER a const-arg perform → OK (ctl5)
;
; So: `do`-def + perform coexist fine UNLESS the def flows into the perform's argument
; (directly or via a call chain that performs). Likely the perform-argument lowering
; captures/rescopes the surrounding do-bindings differently from ordinary call arguments.
;
; IMPACT: any program computing a value in a do-block and performing with it — the natural
; effectful-code shape (compute, then ask/log/bail with the result) — falsely rejects.
; WORKAROUND exists (let-bind instead), which is how the corpus never hit it: no landed
; case flows a do-def into a perform arg.
;
; Minimal repro (this file's case): expect 21 ((ask 7)→14 resumed, +7); actual = CDZ0101
; unbound name `v` at check on all 3 targets.

(case "a do-def value flows into a perform argument (FALSE-REJECT repro: CDZ0101 unbound)"
  (input (do
        (effect Ask (op ask (-> Int64 Int64)))
        (def (run (: u Int64))
          (handle Ask 0
            ((ask (n) s (resume (* n 2) s)))
            (do
              (def v (+ u 2))
              (+ (Ask.ask v) v))))
        (def (main) (run 5))
        (export main)))
  (output (: 21 Int64)))
; NARROWING (breaker #21 boundary): do-def into RESUME-arg in an ARM is FINE; only PERFORM-arg in BODY rescopes. Reference path for v-effects to diff.

; ─────────────────────────────────────────────────────────────────────────────
; RE-CONFIRMED + ROOT-CAUSE NARROWED (v-effects, 2026-08-02, trunk 746ddd327):
; STILL REPRODUCES on all shapes. New narrowing pins it to the HANDLE × do-scope seam:
;   • FAILS (CDZ0101 unbound v) ONLY when the perform is UNDER AN ENCLOSING `handle`:
;       (def (run u) (handle Ask 0 ((ask (n) s (resume (* n 2) s)))
;                       (do (def v (+ u 2)) (+ (Ask.ask v) v))))   → CDZ0101 unbound `v`
;   • BARE perform, NO handle: `v` RESOLVES FINE — you get CDZ0401 (no-home), NOT CDZ0101.
;       (def (run u) (do (def v (+ u 2)) (+ (Ask.ask v) v)))       → CDZ0401 (v is bound!)
;   • do-def into an ORDINARY call arg `(twice v)` under the same do: CLEAN.
;   • the `let`-bound twin: CLEAN (computes 21) on all 3 targets.
; So the trigger is specifically: a do-def reference inside a PERFORM ARGUMENT whose perform
; sits in a `handle` BODY. Fails at `cdz check` (RESOLVE/infer stage) — BEFORE lowering — so it
; is NOT the effects fold; it is a RESOLVE-TIME scope bug.
;
; ROOT-CAUSE HYPOTHESIS (for the fix, when v-effects is unblocked from its queued MR):
;   `effects::desugar_handles` (effects.rs:125) runs at LOAD (db.rs:2160), BEFORE the do-block
;   scope index is built (db.rs:4398 "scope index is built post-desugar"). It rewrites the
;   external→internal handle shape and re-parents the handle body. A reference `v` inside a
;   perform ARGUMENT ascends to `do_local_binds` (resolve.rs:2090) with a `from` that the desugar
;   re-parented, so its recorded child_ix is stale/absent → the identity-scan
;   `forms.iter().position(|f| *f == from)?` returns None → the do-def window is missed → v reads
;   unbound. This is the SAME re-parent class the F2 fix (do_local_binds identity fallback) and
;   `reparent_under_handle_site` address, but for the DESUGAR-time (not fold-time) reparent of a
;   perform-arg subtree. FIX DIRECTION: ensure the perform-arg subtree's ascent `from` still
;   resolves to a DIRECT form of the enclosing `do` after desugar_handles (either preserve the
;   child_ix through the desugar reparent, or extend do_local_binds' identity recovery to walk to
;   the nearest enclosing do-form ancestor when `from` is not itself a direct form).
;   OWNERSHIP: handle-body scope reparent = v-effects territory (effects.rs), intersecting resolve
;   (do_local_binds = v-inference's file) — coordinate with v-inference before touching resolve.rs;
;   the effects.rs desugar side is mine.
; GATE WHEN FIXED: flip this case todo→pass (21) + the abortive row (7) + the heap-rope row, in all
;   3 baselines (titles-agree). Probe the matrix rows above stay green + the let-twin unaffected.

; ─────────────────────────────────────────────────────────────────────────────
; SHARP ISOLATION (v-effects, 2026-08-02 tick 19, trunk d0139b7d3) — supersedes the
; desugar_handles hypothesis above (WRONG: parent_index/child_ix are built AFTER
; desugar_handles per db.rs load order, so post-desugar indices are correct; and
; is_binding_candidate DOES recognize a `(do (def …) …)` as a scope). The real trigger:
;
;   TRIGGER = the handle BODY is a `(do …)` with a do-`def` as the SECOND-TO-LAST form
;   and the reference in the FINAL form (a 2-form window: `(do (def v …) <ref-form>)`).
;   It is NOT perform-specific and NOT about the perform argument.
;
; MATRIX (all `cdz check`, minimal):
;   E  (def (run u) (do (def v (+ u 2)) (+ v v)))                      no handle, 2 forms → CLEAN
;   G  (handle A 0 (arm) (do (def v (+ u 2)) (+ v v)))                 handle body, 2 forms → CDZ0101 unbound v
;   H  (handle A 0 (arm) (do (def v (+ u 2)) (+ v 1) (+ v v)))         handle body, 3 forms → CLEAN
;   I  (handle A 0 (arm) (do (def v (+ u 2)) (+ 1 v)))                 handle body, 2 forms → CDZ0101 (operand order irrelevant)
;   J  (handle A 0 (arm) (do 99 (def v (+ u 2)) (+ v v)))              handle body, 3 forms → CLEAN
;   K  (handle A 0 (arm) (do 99 (+ u u)))                             2 forms, ref a PARAM (no def) → CLEAN
;   L  (handle A 0 (arm) (let ((v (+ u 2))) (+ v v)))                 handle body LET → CLEAN
;   B  (handle A 0 (arm) (do (def v (+ u 2)) (Ask.ask v)))            2 forms, perform → CDZ0101 (same class, not special)
; So: FAILS iff {handle body} ∩ {do-block} ∩ {do-def is 2nd-to-last, ref in last form}.
; Adding ANY 3rd form (before OR after the def) fixes it; a `let` body fixes it; NO handle fixes it.
;
; ROOT CAUSE (now clearly RESOLVE, `do_local_binds` resolve.rs:2090): the do-block that is the
; DIRECT handle body gets a `child_ix`/window computation that is off-by-one ONLY at the exact
; `ix-1` boundary (the 2-form window where `from` = the single tail form immediately after the
; def). The fast path `if ix >= 1 && forms.get(ix-1) == Some(&from) { ix-1 }` OR the identity
; fallback mis-locates `from` when the do-block's parent is the handle-body slot — likely the
; handle body's recorded child_ix (or a reparent of the body under the handle) makes `child_ix_of(from)`
; read a position that only round-trips correctly when there are ≥3 forms. A 3-form do shifts the
; indices enough to hit the identity fallback / correct window; the 2-form do lands exactly on the
; broken fast-path boundary.
;
; OWNERSHIP: this is `do_local_binds` in resolve.rs = v-inference's file. The handle-body parenting
; that perturbs the child_ix is the effects×resolve seam. v-effects (me) has the repro + isolation;
; PINGING v-inference to co-own the resolve.rs fix (I can pair on the handle-body-parent side).
; GATE WHEN FIXED: flip case G/B/I todo→pass + the original 21/7 repros, all 3 baselines (titles-agree).

; ─────────────────────────────────────────────────────────────────────────────
; FURTHER NARROWING (v-effects, tick 20): the 2-form-window do bug is HANDLE-BODY-EXCLUSIVE.
; A 2-form (do (def v …) (+ v v)) resolves CLEANLY in EVERY other expression position — only the
; handle body triggers it:
;   M  let body:            (let ((w 1)) (do (def v …) (+ v v)))     → CLEAN
;   N  if branch:           (if (> u 0) (do (def v …) (+ v v)) 0)     → CLEAN
;   O  call arg:            (id (do (def v …) (+ v v)))               → CLEAN
;   P  tail of outer do:    (do 7 (do (def v …) (+ v v)))             → CLEAN
;   Q  direct def body:     (def (run u) (do (def v …) (+ v v)))      → CLEAN
;   G  HANDLE body:         (handle A 0 (arm) (do (def v …) (+ v v))) → CDZ0101 ✗   (ONLY this)
; So it is NOT a general do_local_binds window bug — it is SPECIFIC to the handle-body parent.
; This shifts the likely fix location to the EFFECTS-SIDE handle-body parenting (child_ix of the
; handle body slot, or a resolve-time handle-body reparent), i.e. v-effects territory more than a
; generic resolve.rs bug — though the SYMPTOM surfaces in do_local_binds. Confirmed at cdz check
; (resolve), and desugar_handles is ruled out (indices built after it). The exact child_ix mismatch
; needs an instrumented resolve run (the fix session's work). v-effects owns the handle-body-parent
; investigation; coordinating with v-inference on the do_local_binds symptom side.

; ═════════════════════════════════════════════════════════════════════════════
; RESOLVED — WAS A STALE-BINARY GHOST (v-effects, tick 21). The "bug" does NOT exist on
; current trunk. v-inference could not reproduce it; root cause of my false repro:
;   MY `target/release/cdz` BINARY WAS BUILT Jul 20 — it PREDATED the do_local_binds fix
;   commits 60d2fce20 + 51cba2652 (the F2 re-parent-under-handler window recovery). Every
;   probe this session used that stale binary, so G/B/I "failed CDZ0101" against ancient
;   codegen while my SOURCE tree already contained the fix. After `cargo build --release -p cdz`
;   (fresh binary Aug 2), the original repro `(handle Ask 0 ((ask (n) s (resume (* n 2) s)))
;   (do (def v (+ u 2)) (+ (Ask.ask v) v)))` → CHECK CLEAN, COMPILES, RUNS to 21. G/H/all matrix rows clean.
; LESSON (🪤 recorded to memory): a long-lived MR-pinned worktree can carry a MONTHS-STALE compiler
; binary — the tick's "rebuild the store" does NOT rebuild `cdz`/`rcdzc`. ALWAYS `cargo build -p cdz`
; (check the binary mtime) before trusting a `cdz check`/`compile` probe result, ESPECIALLY a
; "declines/false-reject" one. A stale binary masquerades as a live compiler frontier — the same
; class as the tick-4 probe-syntax ghost, but from the BINARY not the input.
; ACTION: this .sexp should be RETIRED (rename .RESOLVED) or handed to corpus-bugfix as a GREEN
; regression pin (v-inference's suggestion — pin that the handle-body do-def fix stays fixed). The
; case value is 21 on all 3 backends (to re-verify on a FRESH binary before pinning).
