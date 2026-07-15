; ADVERSARIAL FINDING (producer, iter-385, 2026-07-14) — 🟠 FALSE REJECT (CDZ0101 over-rejection, NOT a
; regression — reproduces at 170bf40a, the parent of the ddf41b52 perf commit; ddf41b52 preserved it): a
; user `(guard …)` on a list pattern that contains a LITERAL element cannot see a `let`-bound enclosing name
; from its guard cond — the compiler falsely reports CDZ0101 "unbound name". The SAME guard reading the SAME
; enclosing name works on every OTHER pattern shape (plain binder-head list, scalar, variant), and reads of a
; PARAM or a TOP-LEVEL def work even on the literal-list pattern, and the arm BODY (not the guard) sees the
; `let` name fine. Only the combination {literal element in the list pattern} × {user guard cond} × {enclosing
; `let` binding} triggers the false reject.
;
; A list pattern with a literal element is desugared by `lower::desugar_refutable_literal_list_elements` into
; a SYNTH `(guard (list __le0 x .. r) (and (= __le0 0) …))` — an eq-check guard chain. The user's own guard
; cond is then nested inside this synth guard structure. Its scope-skip ascent to the enclosing `let` is
; SEVERED by the synth wrapping (the ascent that would reach the `let` binder is cut), while a param /
; top-level name (resolved via a different lookup path) is still found. So the guard cond resolves `x` (the
; pattern binder) and params/top-levels, but not an enclosing `let`.
;
; REPRODUCER (FALSE REJECT — should return 7):
;   (do (def (f (: xs (List Int64)))
;         (let ((lim 5))
;           (match xs ((guard (list 0 x .. r) (> x lim)) x) (_ -1))))
;       (def (main) (f (list 0 7 2)))
;       (export main))
;   → cdz: error [CDZ0101] (node 46): unbound name `lim`   (WRONG — `lim` is the enclosing let binding)
;
; ISOLATION (the trigger is a LITERAL element in the pattern × a user guard × an enclosing `let`):
;   PATTERN SHAPE (guard reads enclosing let `lim`):
;     (guard (list x .. r) (> x lim))       [binder-head, NO literal]      → 7      [OK]
;     (guard x (> x lim))                    [scalar]                       → 7      [OK — 93cb707d pins this]
;     (guard (V x) (> x lim))                [variant]                      → 7      [OK — 93cb707d pins this]
;     (guard (list 0 x .. r) (> x lim))      [literal-leading + splat]      → 🟠 CDZ0101 unbound `lim`
;     (guard (list 0 x) (> x lim))           [literal-leading, NO splat]    → 🟠 CDZ0101 unbound `lim`
;     (guard (list 0 1) (> lim 3))           [all-literal fixed]            → 🟠 CDZ0101 unbound `lim`
;     → the SPLAT is irrelevant; a LITERAL element (→ the synth eq-guard desugar) is the trigger.
;   ENCLOSING BINDING KIND (literal-leading pattern, guard reads it):
;     enclosing is a PARAM `lim`                                            → 7      [OK]
;     enclosing is a TOP-LEVEL def `(lim)`                                  → 7      [OK]
;     enclosing is a `let` binding `lim`                                    → 🟠 CDZ0101 unbound `lim`
;     → only a `let`-bound name is lost; params / top-levels still resolve.
;   POSITION (literal-leading pattern, enclosing let `lim`):
;     the arm BODY reads `lim`:  (list 0 x .. r) → (+ x lim)                → 12     [OK]
;     the user GUARD COND reads `lim`                                       → 🟠 CDZ0101
;     → only the guard cond loses the `let`; the body sees it.
;
; ROOT CAUSE (hypothesis, lower.rs + resolve.rs scope-skip): when `desugar_refutable_literal_list_elements`
; wraps the arm in a synth `(guard … (and (= __leK litK) …))`, the user guard cond becomes a child of the
; synth guard structure. The scope-skip chain built over the synth nodes (the same machinery ddf41b52 tuned
; with `extend_scope_skip_into_subtree`) routes the guard-cond references' ascent through the synth guard
; CANDIDATE and its `and`-chain, and the hop lands at a point that no longer reaches the enclosing `let`'s
; binding scope — whereas a param/top-level (module- or def-level lookup) is found regardless of the skip
; chain. The arm body is wired outside the synth guard so it keeps the correct ascent.
;
; FIX (hypothesis): when synthesizing the eq-guard for literal elements, preserve the user guard cond's
; original enclosing scope — the synth `(guard …)` must not sever the ascent to the enclosing `let`; the
; guard cond should skip PAST the synth eq-chain to the same scope the arm body sees (which resolves the
; `let`). I.e. the scope-skip for a user-guard-cond node nested in the synth guard should target the arm's
; enclosing scope, not stop at the synth guard candidate.
;
; SEVERITY: 🟠 FALSE REJECT (over-rejection, CDZ0101) — a valid, well-typed program is rejected at compile
; time. Not a miscompile (no wrong value, no crash) and not a regression (pre-existing at 170bf40a). Reachable
; from the idiomatic "match a list whose head is a known literal tag, guard the rest against a locally-bound
; threshold" — e.g. `(let ((limit …)) (match tokens ((guard (list 0 n .. rest) (> n limit)) …) …))`. Grades
; Todo (over-rejection) — a valid program the compiler declines with a spurious unbound-name error.

(case "a guard on a literal-element list pattern reads an enclosing let binding"
  (doc    "`(let ((lim 5)) (match xs ((guard (list 0 x .. r) (> x lim)) x) (_ -1)))` — a user guard on a list
           pattern with a LITERAL leading element (`0`), whose guard cond `(> x lim)` reads the enclosing
           `let` binding `lim`. Must match [0,7,2] (0 literal ok, x=7) and return 7 (7 > 5). Instead the
           compiler falsely rejects with CDZ0101 `unbound name lim`. The literal element forces the
           refutable-literal desugar to wrap the arm in a synth `(guard … (and (= __le0 0) …))`, and the
           user guard cond nested inside it loses its scope-skip ascent to the enclosing `let` — while a
           PARAM or TOP-LEVEL `lim` resolves fine, and the arm BODY sees the `let` `lim` fine (→12), and the
           same guard on a binder-head/scalar/variant pattern (no literal, no synth guard) resolves `lim`
           (→7). Pre-existing (reproduces at 170bf40a); ddf41b52 preserved it. Fix: the synth eq-guard must
           not sever the user guard cond's ascent to the enclosing scope. Expected: 7.")
  (input  (do
            (def (f (: xs (List Int64)))
              (let ((lim 5))
                (match xs ((guard (list 0 x .. r) (> x lim)) x) (_ -1))))
            (def (main) (f (list 0 7 2)))
            (export main)))
  (output (: 7 Int64)))
