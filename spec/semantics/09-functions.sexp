; Functions and closures — witnesses core-semantics.md §Functions. Functions are
; first-class values (fn), applied by (fn-expr arg), capturing their enclosing
; scope. Functions are SINGLE-ARITY: each function takes exactly one argument.
; Multi-parameter syntax (fn (x y) body) is sugar for currying: (fn x (fn y body)).
; Application (f a b) is sugar for ((f a) b). The seed realizes these, because a compiler authored
; in Cadenza is built from functions and closures. Results are (: <value> <Type>).
(diagnostic-quality)

(case
  "a function applied to an argument"
  (doc
    "Witnesses core-semantics.md §A Function Is A First-Class Value and §Applying A Function
           Binds Its Parameters To Its Arguments: an inline fn is applied to 5, binding x to 5.")
  (input ((fn (x) (+ x 1)) 5))
  (output (: 6 Int64)))

(case
  "a function bound to a name and then applied"
  (doc
    "Witnesses core-semantics.md §A Function Is A First-Class Value: a fn is an ordinary value
           bindable by let, then applied by naming it in head position.")
  (input (let ((inc (fn (x) (+ x 1)))) (inc 10)))
  (output (: 11 Int64)))

(case
  "an annotated lambda parameter binds"
  (doc
    "`((fn ((: x Int64)) (+ x 1)) 5)` = 6 — a parameter annotation `(: x Int64)` binds `x` exactly
           as the unannotated `(fn (x) …)` does: the body's `x` resolves through the annotated binder (not
           UNBOUND) and the application folds. Pins that the annotation is transparent to binding — it
           constrains the type without changing that the parameter is bound.")
  (input ((fn ((: x Int64)) (+ x 1)) 5))
  (output (: 6 Int64)))

(case
  "a def with a mix of annotated and unannotated parameters binds and folds"
  (doc
    "`(def (w (: a Int64) b) (+ a b))` called `(w 20 22)` = 42 — the annotated binder `(: a Int64)`
           and the bare binder `b` both bind in the same parameter list, the body's `a`/`b` resolve, and
           the call folds. Pins that annotating SOME parameters does not disturb the binding of the rest.")
  (input (do (def (w (: a Int64) b) (+ a b)) (def (main) (w 20 22)) (export main)))
  (output (: 42 Int64)))

(case
  "a closure captures the binding in scope where it was created"
  (doc
    "Witnesses core-semantics.md §A Function Is A First-Class Value (2nd sentence): the fn
           captures y=3 from its creation scope; applying it later observes the captured y even though
           the application site has its own y=100.")
  (input (let ((add-y (let ((y 3)) (fn (x) (+ x y))))) (let ((y 100)) (add-y 4))))
  (output (: 7 Int64)))

; The case above captures a CONSTANT `y` and folds. These pin closure capture semantics at RUN TIME (a
; boundary parameter flows into the capture, so nothing folds) and with two closures alive at once — the
; cases the single-closure constant case cannot: capture is BY VALUE at creation (a later same-named
; binding does NOT rebind an existing closure's capture) and each closure holds its OWN captured
; environment (two closures from one factory do not share a capture slot). A representation that captured
; by reference / late-bound the name, or that shared one environment cell across closures, would give a
; different — and here numerically distinct — answer.
(case
  "one staged partial application fans out to TWO second stages sharing the first capture"
  (doc
    "The fan-out face of currying (the chains above stage one path): ONE stage-1 closure
           f1 = (f 2) feeds TWO stage-2 closures g1/g2 that are alive SIMULTANEOUSLY — the x=2
           capture cell is shared by two second-stage environments, so a stage-2 build that MOVED
           rather than borrowed the stage-1 environment kills its sibling before both apply.
           (g1 5)+(g2 5) = 305+405 = 710; the n=0 face zeroes g2's middle coordinate → 510.")
  (input
    (do
      (def (f (: x Int64)) (fn ((: y Int64)) (fn ((: z Int64)) (+ (* x 100) (+ (* y 10) z)))))
      (def
        (main (: n Int64))
        (do (def f1 (f 2)) (def g1 (f1 10)) (def g2 (f1 n)) (+ (g1 5) (g2 5))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 710 Int64))
  (call main (: 0 Int64))
  (output (: 510 Int64)))

(case
  "a captured lambda applied at two shadowing sites resolves each to its DEF-SITE capture (reduce-cache share soundness)"
  (doc
    "Pins the SOUNDNESS of the β-reduce cache sharing (rcdzc `eval` keys a reduction on the RESOLVED
           lambda BODY so a fan-out over one shared lambda de-duplicates the reduction — the seq-203
           handler-fn-per-closure-arg 2^N fix). `g = (fn (q) (+ q a))` captures its DEF-SITE `a = 100`;
           it is then applied `(g p)` from TWO nested scopes that SHADOW `a` (500, then 900). Both `(g p)`
           calls carry the SAME `(body, args)` and so SHARE ONE reduced node — but a lambda's capture is
           lexical to its DEFINITION, so each MUST use `a = 100`: `(p+100) + (p+100) = 2p+200`. If sharing
           the reduced node RE-PARENTED it under a call site (so its captured `a` resolved through that
           site's shadow), the value would corrupt to `2p+1000` / `2p+1400` — a miscompile. This is the
           adversarial witness that the reduce-cache share is sound: β-reduce PINS the captured subtree, so
           the re-parent-under-site sets only the ROOT parent, which a pinned capture never resolves
           through. The `(: p …)` boundary parameter keeps it runtime (nothing folds), so the shared
           runtime reduction is exercised. (Each shadowing `a` is unused BY `g` — that CDZ0306 is expected
           and is itself the signal that the captures are pinned to the def site, not the call site.)")
  (input
    (do
      (def
        (main (: p Int64))
        (let ((a 100) (g (fn (q) (+ q a)))) (let ((a 500)) (+ (g p) (let ((a 900)) (g p))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 202 Int64))
  (call main (: 50 Int64))
  (output (: 300 Int64)))

(case
  "a factory-returned closure that BOTH escapes into a list AND is directly applied at the same binder"
  (doc
    "The adversarial CALL-BOTH-WAYS witness for a REDUCES-TO-LAMBDA binding: `f = (mk-adder k)` — a
           factory CALL that reduces to the capturing closure `(fn (x) (+ x k))` — is used TWO ways at once:
           its handle ESCAPES-WHOLE into `#list(f f)` (which the runtime must materialize), AND it is
           DIRECTLY applied `(f 2)`. Both must observe the SAME captured `k`: `(f 5) + (f 2)` from the list
           slot and the direct call = `(5+k) + (2+k) = 7 + 2k`. k=3 → 13; k=0 → 7; k=-4 → -1.

           Pins the fix for a SHARED backend miscompile (invalid on BOTH targets before it): the adv-50
           CALL-BOTH-WAYS force-keep matched only a LITERAL `(fn …)` binding, so a factory-CALL binding that
           REDUCES to a capturing lambda slipped past it and copy-propagated — the escape LIFTED the reduced
           closure (recording its captured `k` occurrence) while the direct `(f 2)` β-FOLDED to `(+ 2 k)`,
           reusing that occurrence as a capture-env read in the ENCLOSING scope (which has no closure env):
           the wasm module failed to compile (a bad `call_indirect`/type mismatch on the lifted body) and
           the rust emit referenced an unbound capture parameter (`__cap0`, E0425). The fix force-keeps the
           reduced closure as ONE materialized `Core::Let` slot so the direct call `call_indirect`s that one
           cell (both uses share it) — no fold reuses a poisoned occurrence. The `(: k Int64)` boundary
           parameter keeps the capture RUNTIME (nothing folds), so the lift-and-share path is exercised; the
           list holding two references to the one closure exercises the escape+direct intersection.")
  (input
    (do
      (def (mk-adder (: n Int64)) (fn ((: x Int64)) (+ x n)))
      (def
        (main (: k Int64))
        (let
          ((f (mk-adder k)))
          (let ((bag #list(f f))) (+ ((Option.expect (List.at bag 0) "g") 5) (f 2)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 13 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: -4 Int64))
  (output (: -1 Int64))
  ; INTERIM re-pin (v-rust-backend, 2026-08-30): #6049's CALL-BOTH-WAYS force-keep materializes the
  ; reduced closure as one Core::Let slot; on the escape-into-#list + direct-apply path the slot's
  ; surviving ref is not yet dropped (the surviving-owned-ref-drop reclaim class, same as cdzw66/#6022).
  ; LEAK-side (values correct, no trap; seq-278). Real fix = the dup_sites-3690-3694 surviving-slot-drop
  ; (v-mem co-design, in flight) → tightens to 0; #5766 tolerate-fewer auto-passes the collapse. Was (0).
  (live-objects known-leak))

(case
  "a partial application captures a runtime parameter in the residual closure"
  (doc
    "Partially applying to a VARIABLE reference must CAPTURE it in the residual lambda: `((sub n) 3)`
           curries `(sub a b) = (- a b)` to `(fn (b) (- n b))`, and `n` — a caller-pinned free variable —
           must keep its binding across the residual's later application. n = 10 → (- 10 3) = 7. Pins the
           free-variable currying capture (a re-copy that re-resolved `n` in the residual's scope, where it
           is unbound, spuriously rejected it); a constant capture always worked — this is the variable case.")
  (input (do (def (sub a b) (- a b)) (def (main (: n Int64)) ((sub n) 3)) (export main)))
  (call main (: 10 Int64))
  (output (: 7 Int64)))

(case
  "a partial application captures a let-bound variable in the residual closure"
  (doc
    "The let-bound companion: the captured value is any in-scope binding, not only a parameter.
           `(let ((m 10)) ((sub m) 3))` curries `sub` capturing the let-local `m` in the residual → (- 10
           3) = 7.")
  (input (do (def (sub a b) (- a b)) (def (main) (let ((m 10)) ((sub m) 3))) (export main)))
  (output (: 7 Int64)))

(case
  "a body-internal let-local is unaffected by the free-variable currying capture"
  (doc
    "The negative control: a body-internal `let`-local is NOT resolve-pinned, so it still re-resolves
           against the copied scope normally. `(inc n) = (let ((k 10)) (+ n k))`, applied `(inc v)` with
           v = 5 → 5 + 10 = 15. Pins that the free-variable capture fix does not disturb ordinary
           body-local resolution.")
  (input (do (def (inc n) (let ((k 10)) (+ n k))) (def (main (: v Int64)) (inc v)) (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

; A partial application capturing a COMPOUND (heap-value) argument in the residual closure — the compound
; sibling of the scalar-capture cases above. Formerly DECLINED ("a partial application of a runtime closure
; … is not supported") pending the capture-escape dup wiring: the residual lambda captures the compound and
; passes it to the underlying fn as a consuming arg, so the capture ESCAPES and needs a dup to balance the
; closure-cell drop. That wiring (#5007 `collect_captured_escape_dup_sites`) now runs on the synthesized
; eta-closure body too, so the compound capture is DUP'd per escaping occurrence — leak-free. Each pins
; `(live-objects 0)`: the escape dup exactly balances the cell drop. (v-effects co-verified the 3 shapes.)
(case
  "a partial application capturing a fresh-literal List in the residual closure runs and reclaims (live-objects 0)"
  (input
    (do
      (def (f (: a (List Int64)) (: b (List Int64))) (List.len (List.concat a b)))
      (def (main) (do (def g (f #list(1 2))) (g #list(3 4 5))))
      (export main)))
  (call main)
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a partial application whose residual reads the compound capture in TWO escaping positions reclaims cleanly (per-occurrence dup)"
  (doc
    "The multi-occurrence face: the residual reads the captured List `a` in two consuming positions
           (`List.len a` + `List.concat a b`). The capture-escape dup is PER-OCCURRENCE, not one-total, so both
           reads are balanced — the hczm1 face. `((g #list(1 2)) #list(3 4 5))` = len[1,2] + len[1,2,3,4,5] = 7.")
  (input
    (do
      (def (g (: a (List Int64)) (: b (List Int64))) (+ (List.len a) (List.len (List.concat a b))))
      (def (main) (do (def p (g #list(1 2))) (p #list(3 4 5))))
      (export main)))
  (call main)
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a compound-capturing residual closure built once and CALLED TWICE keeps the capture live across both calls"
  (doc
    "The called-twice face: the residual `g` (capturing `#list(1 2)`) is applied twice — the captured
           List's refcount must survive repeated reads. `(g #list(3))` = len[1,2,3] = 3, `(g #list(4 5))` =
           len[1,2,4,5] = 4, sum 7. A one-shot capture would UAF/under-free on the second call.")
  (input
    (do
      (def (f (: a (List Int64)) (: b (List Int64))) (List.len (List.concat a b)))
      (def (main) (do (def g (f #list(1 2))) (+ (g #list(3)) (g #list(4 5)))))
      (export main)))
  (call main)
  (output (: 7 Int64))
  (live-objects 0))

; --- A BUILT-IN OPERATION partially applied CURRIES — completing it yields a value (should-work) ---
;    (migrated from rcdzc a_partial_builtin_operation_as_an_unconsumed_value_is_rejected_not_silently_shipped)
; Like a USER function, a BUILT-IN OPERATION is a first-class value (core-semantics L291) and partial application
; MUST be natural — applying fewer than its arity returns a CLOSURE awaiting the rest (core-semantics L73), which
; completes to a value when the remaining args arrive. The first-class treatment of a built-in-operation value
; (storage, partial application) is to-be-specified-as-realized (core-semantics L295): a compiler that does not
; yet synthesize the runtime closure for a built-in-used-as-a-value DECLINES rather than miscompile
; (declined(PrimAsValueNeedsClosure), owner v-compiler-primitives) — so these SHOULD-WORK cases are TODOs that
; auto-pass when that closure synth lands, however the spine is spelled (flat `(String.slice s 0)` or nested
; `((String.slice s) 0)` — flattened to its bottom head). CONTRAST: OVER-application (too many args) is a PERMANENT
; CDZ0203 error (core-semantics L293); full application, unary negation (prefix `-` = Sub at arity 1), and a
; completing constructor spine all compile today.
(case
  "a partial built-in operation (slice at 2 of 3 args) curries — completing it yields a value (should-work)"
  (doc
    "`(String.slice s 0)` is slice partially applied (start given, end missing) — it SHOULD curry to a
           closure awaiting the end index (core-semantics L73/L295), completing to a substring. Now CURRIES +
           computes (the built-in-as-value closure synth landed); the captured runtime String leaks by 1 —
           tracked `(live-objects known-leak)`, v-memory-safety's borrow-only-heap-capture reclaim follow-up.
           f holds the partial; `((f \"abcdef\") 4)` completes it to slice(\"abcdef\",0,4). `String.slice` is TOTAL —
           collections-and-text.md §134 MUSTs a sub-sequence slice yield an OPTIONAL value (present in bounds,
           absent out of bounds), so the result is `(Option String)` (the sibling of `String.at`), not a bare
           String — the in-bounds slice is `Some \"abcd\"` (end EXCLUSIVE), unwrapped to byte-len 4.")
  (input
    (do
      (def (f (: s String)) (String.slice s 0))
      (def (main) (match ((f "abcdef") 4) ((Some sub) (String.byte-len sub)) ((None _u) -1)))
      (export main)))
  (call main)
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "a partial built-in operation (at at 1 of 2 args) curries — completing it yields a value (should-work)"
  (doc
    "`(String.at s)` is at partially applied (index missing) — it SHOULD curry to a closure awaiting the
           index (core-semantics L73/L295). Now CURRIES + computes (the built-in-as-value closure synth landed);
           the captured runtime String leaks by 1 — tracked `(live-objects known-leak)`, v-memory-safety's
           borrow-only-heap-capture reclaim follow-up. `((f \"hi\") 0)` completes it to String.at(\"hi\",0) =
           Some \"h\" — String.at yields an (Option String) 1-scalar substring (not a Char); the arm returns 1.")
  (input
    (do
      (def (f (: s String)) (String.at s))
      (def (main) (match ((f "hi") 0) ((Some c) (if (= c "h") 1 0)) ((None _u) -1)))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a partial built-in operation spelled as a NESTED spine curries identically to the flat form (should-work)"
  (doc
    "`((String.slice s) 0)` is the same application as the flat `(String.slice s 0)` (`(f a b)` desugars
           to `((f a) b)`), so it curries identically — the spine is flattened to its bottom head, so the nested
           and flat surfaces are treated the same. Completing `((f \"abcdef\") 4)` = slice(\"abcdef\",0,4) =
           `Some \"abcd\"` — `String.slice` is TOTAL, returning `(Option String)` (collections-and-text.md §134),
           so it is unwrapped like the flat form; byte-len 4, same value. Now CURRIES + computes (the closure
           synth landed); the captured runtime String leaks by 1 — tracked `(live-objects known-leak)`,
           v-memory-safety's borrow-only-heap-capture reclaim follow-up.")
  (input
    (do
      (def (f (: s String)) ((String.slice s) 0))
      (def (main) (match ((f "abcdef") 4) ((Some sub) (String.byte-len sub)) ((None _u) -1)))
      (export main)))
  (call main)
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "a partial built-in operation (List.at at 1 of 2 args) curries — completing it yields a value (should-work)"
  (doc
    "The HEAP-COLLECTION sibling of the String partials above: `(List.at l)` is at partially applied
           (index missing) — it SHOULD curry to a closure awaiting the index (core-semantics L73/L295).
           Now CURRIES + computes + RECLAIMS cleanly (live-objects 0): the built-in-as-value closure synth
           landed and part-1 (CallClosure-result=Owned) reclaims the scalar-payload Option shell. No spec carve-out makes a
           heap-collection op differ from a String op: a captured List is just a heap handle, captured in the
           pending closure by the same mechanism as a captured String. `f` holds the partial; `((f #list(10
           20 30)) 1)` completes it to List.at([10,20,30],1) = Some 20, and the arm returns 20. (Migrated
           from rcdzc a_partial_application_of_a_builtin_operation_declines_honestly — v-spec-oracle ruled
           SHOULD-WORK, so this asserts the idealistic curry rather than pinning the decline.)")
  (input
    (do
      (def (f (: l (List Int64))) (List.at l))
      (def (main) (match ((f #list(10 20 30)) 1) ((Some x) x) ((None _u) -1)))
      (export main)))
  (call main)
  (output (: 20 Int64))
  (live-objects 0))

(case
  "a FULLY applied built-in operation is not flagged as a partial"
  (doc
    "The no-false-positive control: a built-in op applied to exactly its arguments is fine.
           `(String.byte-len \"hi\")` (arity 1, fully applied) → 2.")
  (input (do (def (main) (String.byte-len "hi")) (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a partially applied USER function is legitimate and is not flagged as a wrong-arity built-in"
  (doc
    "The distinguishing contrast: a USER function is single-arity and freely partially-applicable, so
           `(g 1)` held in `h` (a residual closure over the first argument) is legitimate and must NOT draw
           the built-in-operation wrong-arity reject. The program compiles; `main` returns 0. A module-member
           partial `((. lib g) 1)` is the same mechanism and is likewise fine.")
  (input
    (do (def (g (: x Int64) (: y Int64)) (+ x y)) (def (h) (g 1)) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

; --- OVER-application is a PERMANENT CDZ0203 (delete-surplus fix); UNDER-application CURRIES (should-work) ---
;    (migrated from rcdzc over_applying_a_builtin_operation_reports_one_error_with_the_delete_fix)
; A built-in operation OVER-applied (too many args — `(Map.len m 99)`, size takes one) is a PERMANENT error
; (core-semantics L293): infer draws the coded CDZ0203 over-application (with a delete-surplus fix), and
; `dedup_faults` drops the weaker uncoded wrong-arity decline so EXACTLY ONE primary error reports — the CDZ0203
; carrying the delete fix. An UNDER-application (`(List.at l)`, missing the index) is the OPPOSITE: partial
; application, which SHOULD curry to a closure awaiting the rest (core-semantics L73/L295) — a should-work gap
; that declines(PrimAsValueNeedsClosure) only until the built-in-as-value closure synth is realized, not a
; permanent error. Completing the partial yields a value.
(case
  "an over-applied built-in operation is ONE CDZ0203 with a delete-surplus fix, not doubled with the wrong-arity decline"
  (input (do (def (main) (Map.len #map((= 1 2)) 99)) (export main)))
  (error CDZ0203 (fix (kind delete)) (count 1) (exact-code)))

; The USER-FUNCTION twin of the built-in over-application above (breaker): over-applying a plain user `def`
; — `(f 1 2)` where `f` is arity 1 — is ALSO a permanent CDZ0203, but with a DISTINCT diagnostic from the
; built-in-operation case: "applied 2 arguments to a function of arity 1 — it is not a function after its
; arguments are consumed" (a VALUE-application-over-arity — after f consumes its 1 arg the result is an Int64,
; which is not applicable — vs the built-in's delete-surplus wrong-arity). Consistent CDZ0203 across
; wasm+rust+cadenza (a front-end arity error, backend-independent). Contrast the LEGITIMATE user-function
; UNDER-application (partial/curry) above (~321) — under-applying curries, over-applying rejects. Pins the
; user-fn over-application diagnostic (v-corpus-harness diagnostic-quality) — distinct from the built-in path.
(case
  "an over-applied USER function is CDZ0203 — applying past its arity hits a non-function result"
  (input (do (def (f (: x Int64)) x) (def (main) (f 1 2)) (export main)))
  (error
    CDZ0203
    (message "applied 2 arguments to a function of arity 1")
    (message "not a function after its arguments are consumed")))

(case
  "an under-applied built-in operation curries — completing it yields a value (should-work)"
  (doc
    "`(List.at l)` is List.at partially applied (index missing) — the OPPOSITE of the over-application
           above: it SHOULD curry to a closure awaiting the index (core-semantics L73/L295), not stay a decline.
           Now CURRIES + computes + RECLAIMS cleanly (live-objects 0): the built-in-as-value closure synth landed
           and part-1 (CallClosure-result=Owned) reclaims the scalar-payload Option shell.
           `((List.at #list(10 20 30)) 1)` completes it to List.at(l,1) =
           Some 20 (0-indexed); the arm returns 20.")
  (input
    (do
      (def (main) (match ((List.at #list(10 20 30)) 1) ((Some v) v) ((None _u) -1)))
      (export main)))
  (call main)
  (output (: 20 Int64))
  (live-objects 0))

; An OVER-APPLIED binary OPERATOR (`+`/`<`/float `+` given 3 operands) is the binop-arity twin of the
; over-applied member op above: it reports EXACTLY ONE CDZ0201 "takes exactly 2 operands" with a delete-the-
; -extra-element fix (the dedup drops the un-deduped CDZ0203 sibling that lower + infer would otherwise BOTH
; raise). Integer, comparison, and float arithmetic share the `binop_arity_reject` path. A zero-operand `(+)`
; also faults CDZ0201. (Migrated from rcdzc over_application_offers_a_delete_the_extra_argument_fix.)
(case
  "an over-applied integer operator is one CDZ0201 takes-exactly-2 with a delete fix"
  (input (do (def (main) (+ 1 2 3)) (export main)))
  (error CDZ0201 (message "takes exactly 2 operands") (count 1) (fix (kind delete))))

(case
  "an over-applied comparison operator is one CDZ0201 takes-exactly-2 with a delete fix"
  (input (do (def (main) (< 1 2 3)) (export main)))
  (error CDZ0201 (message "takes exactly 2 operands") (count 1) (fix (kind delete))))

(case
  "an over-applied FLOAT arithmetic operator is one CDZ0201 takes-exactly-2 with a delete fix"
  (input (do (def (main) (+ 1.0 2.0 3.0)) (export main)))
  (error CDZ0201 (message "takes exactly 2 operands") (count 1) (fix (kind delete))))

(case
  "a zero-operand binary operator faults CDZ0201 takes-exactly-2"
  (input (do (def (main) (+)) (export main)))
  (error CDZ0201 (message "takes exactly 2 operands")))

(case
  "an UNDER-applied binary operator curries — completing it yields a value"
  (doc
    "The OPERATOR twin of the under-applied member-op currying above: `(+ 1)` is `+` partially applied, a
           closure awaiting the second operand; applying it completes the sum. `((+ 1) 2)` = 3.")
  (input (do (def (main) ((+ 1) 2)) (export main)))
  (output (: 3 Int64)))

(case
  "an unannotated tuple-SWAP instantiates at two mixed scalar-heap element pairings in one program"
  (doc
    "The mixed scalar-heap instantiation face: swap p = (b a) at (Int64, String-rope) AND
           ((List Int64), Int64) in one program — the specializer must produce two layouts where the
           heap handle sits in OPPOSITE tuple slots, with a runtime-built rope riding through; the
           n=0 face sends an EMPTY rope through the swap.")
  (input
    (do
      (def (swap p) (match p (#tuple(a b) #tuple(b a))))
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (main (: n Int64))
        (do
          (def r1 (swap #tuple(3 (rep "ab" n ""))))
          (def r2 (swap #tuple(#list(1 2 3) 7)))
          (match
            r1
            (#tuple(s x)
              (match
                r2
                (#tuple(y xs) (+ (* (String.byte-len s) 100) (+ (* x 10) (+ (List.len xs) y)))))))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 440 Int64))
  (call main (: 0 Int64))
  (output (: 40 Int64))
  (live-objects known-leak))

(case
  "a closure captures its environment by value at creation, unaffected by a later same-named binding"
  (doc
    "`(let ((k n)) (let ((f (fn (x) (+ x k)))) (let ((k 1000)) (f 1))))` — `f` captures `k = n` at
           creation; the INNER `(let ((k 1000)) …)` introduces a NEW `k` in scope at the APPLICATION site,
           but `f` observes the `k` it captured, not the later one. So `(f 1)` = `1 + n`, NOT `1 + 1000`:
           n=5 → 6, n=40 → 41 (core-semantics.md §A Function Value Captures The Bindings In Scope Where It
           Is Created — capture is by value at creation). A compiler that late-bound the free `k` to the
           nearest binding at APPLICATION time would answer 1001. The runtime companion of the
           application-site-shadowing case above, with the shadowing binding sitting BETWEEN creation and
           application.")
  (input
    (do
      (def (main (: n Int64)) (let ((k n)) (let ((f (fn (x) (+ x k)))) (let ((k 1000)) (f 1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: 40 Int64))
  (output (: 41 Int64)))

(case
  "two closures from one factory capture distinct values"
  (doc
    "`(adder k) = (fn (x) (+ x k))` built twice — `add3` captures 3, `add10` captures 10 — both alive
           at once. `(- (add10 n) (add3 n))` = `(n+10) - (n+3)` = 7 for EVERY `n` (n=5 and n=0 both → 7).
           Pins that each closure holds its OWN captured environment: a representation that shared one
           capture cell across the two closures (both ending at the last-built 10, or both at 3) would give
           0, not 7. The two captures are distinct and independent.")
  (input
    (do
      (def (adder k) (fn (x) (+ x k)))
      (def (main (: n Int64)) (let ((add3 (adder 3)) (add10 (adder 10))) (- (add10 n) (add3 n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a closure captures a runtime-depth RECURSIVE SUM and folds it at call time"
  (doc
    "The unbounded-heap capture face (the scalar captures above hold Int/Bool; the host-closure file
           pins list/rope captures across the HOST boundary — this is the pure in-guest twin): `make-reader`
           captures a runtime-depth Peano spine `(mk a)` in the closure cell and returns `(fn (u) (depth
           v))`; the caller invokes it AFTER make-reader's activation is gone, so the captured spine must
           outlive its builder (capture-cell dup) and the fold must walk the live heap value at call time
           → 3. A capture that shallow-copied one node or dropped the spine at return would break it.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (mk (: n Int64)) (if (= n 0) (Z) (S (mk (- n 1)))))
      (def (depth (: v Nat)) (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
      (def (make-reader (: v Nat)) (fn ((: u Unit)) (depth v)))
      (def (main (: a Int64)) ((make-reader (mk a)) unit))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a closure captures a MAP and serves lookups at call time"
  (doc
    "The CHAMP-capture face: `make-getter` captures a two-entry map and returns a lookup closure; the
           caller invokes it twice with different keys — a hit path (`g a` at a=1 → 10) and the fixed `g 2`
           → 20, summing 30; at a=9 the first lookup misses (-1) → 19. The captured CHAMP must stay live
           across both calls and serve genuine per-call lookups (a snapshot of one lookup result, or a
           dropped map, would break a call). The environment-closure idiom an interpreter's `eval`
           closes over.")
  (input
    (do
      (def
        (make-getter (: m (Map Int64 Int64)))
        (fn ((: k Int64)) (match (Map.lookup m k) ((Some v) v) ((None u) -1))))
      (def (main (: a Int64)) (let ((g (make-getter #map((= 1 10) (= 2 20))))) (+ (g a) (g 2))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 30 Int64))
  (call main (: 9 Int64))
  (output (: 19 Int64)))

(case
  "a list of closures each keeps its own capture, selected by a runtime index"
  (doc
    "Three closures `(mk 10)`, `(mk 20)`, `(mk 30)` — each `(mk k) = (fn (x) (+ x k))` capturing its
           own `k` — are stored in a LIST and one is selected by a runtime index, then applied. `apply-at
           fs i 1` = `(elem i)(1)` = `1 + (10|20|30)`: i=0 → 11, i=2 → 31, an out-of-bounds index → -1. Pins
           that closures carried in a collection each retain their distinct capture (the list does not
           collapse them to one environment), and that indexing selects the intended one at run time — the
           collection companion of the two-factory-closures case.")
  (input
    (do
      (def (mk k) (fn (x) (+ x k)))
      (def (apply-at fs i x) (match (List.at fs i) ((Some f) (f x)) (None -1)))
      (def (main (: i Int64)) (apply-at #list((mk 10) (mk 20) (mk 30)) i 1))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (call main (: 2 Int64))
  (output (: 31 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64))
  ; #6049 FIXED (v-core-opt 2026-08-30, v-mem co-verified): the borrowed-extracted closure Some-shell is now
  ; reclaimed after the borrowing apply — applying a closure BORROWS its callee (CallClosure), so the owned
  ; List.at Some-shell deep-drops post-apply and its cascade reclaims the closure cell + captures. Was known-leak-2.
  (live-objects 0))

; A CAPTURING closure whose HANDLE both ESCAPES WHOLE (stored into a heap collection / sum payload) AND is
; ALSO DIRECTLY CALLED — the "call BOTH ways" shape. The pinned idioms above call a stored closure via
; LOOKUP/select (`List.at`/`Map.lookup`) or a `match` extract — never ALSO directly — so they fold or
; `call_indirect` on ONE path. When the SAME binding is used both ways, the store LIFTS the closure
; (materializing its capture env), while the direct call would β-FOLD it inline — reusing the SAME captured
; occurrence, now an env-read in the enclosing ENV-LESS scope: an INVALID artifact on BOTH backends (wasm
; `invalid component … wasm[0]::function[N]`, rust `error[E0425]: cannot find value __cap0`), a reject-don't-
; miscompile violation at the artifact level (breaker adv-50). The fix FORCE-KEEPS such a binding as ONE
; materialized runtime closure and routes the direct call through it via `call_indirect`, so no fold reuses
; the capture occurrence. `k` = 100, `f1 v = k + v`; `main 5` inserts/stores `f1` (result discarded) then
; calls `f1 5` = 105. The store is not CHAMP-specific — list / set / map / sum-payload all share the boxed-
; cell rep and all miscompiled the same way; a TUPLE/RECORD element (fixed-shape unboxed) always survived.
(case
  "a capturing closure stored in a map and also called directly emits a valid artifact"
  (doc
    "A capturing `f1 = (fn (v) (+ k v))` inserted into a `Map` (result DISCARDED) AND called directly
           `(f1 d)`. The store lifts `f1`; the direct call must NOT re-fold it and reuse the lifted capture
           occurrence (which produced an invalid module / rust `__cap0`) — it force-keeps the closure and
           `call_indirect`s it. `main 5` → discarded insert, then `f1 5 = 105`.")
  (input
    (do
      (def
        (main (: d Int64))
        (let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do #map((= 1 f1)) (f1 d)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64))
  (live-objects known-leak))

(case
  "a capturing closure stored in a list and also called directly emits a valid artifact"
  (doc
    "The list-container companion of the map case — same root (the collection-element boxed-fn
           materialization vs the direct-call fold), same fix (force-keep + `call_indirect`). `main 5` →
           discarded `(list f1)`, then `f1 5 = 105`.")
  (input
    (do
      (def
        (main (: d Int64))
        (let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do #list(f1) (f1 d)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64))
  (live-objects known-leak))

(case
  "a capturing closure stored in a sum payload and also called directly emits a valid artifact"
  (doc
    "The SUM-PAYLOAD companion (`(Some f1)`, boxed-cell rep like the collections) — confirms the root
           is the boxed/descriptor rep, not collection-specific. `main 5` → discarded `(Some f1)`, then
           `f1 5 = 105`.")
  (input
    (do
      (def
        (main (: d Int64))
        (let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do (Some f1) (f1 d)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64))
  (live-objects known-leak))

(case
  "a capturing closure whose surviving map store and direct call both feed the result"
  (doc
    "The store SURVIVES to the result (its `Map.len` is added), not merely discarded — so the lifted
           closure and the directly-called one are the SAME force-kept cell used both ways in ONE
           expression. `main 5` → `(f1 5) + (Map.len (Map.insert Map.empty 1 f1))` = 105 + 1 = 106.")
  (input
    (do
      (def
        (main (: d Int64))
        (let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (+ (f1 d) (Map.len #map((= 1 f1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 106 Int64))
  ; INTERIM re-pin (v-rust-backend, 2026-08-30): #6049's force-keep now materializes f1 as one Core::Let
  ; slot; on the surviving-MAP-STORE + direct-call path the slot's surviving ref is not yet dropped (the
  ; map-store consumes a DUP so the slot original survives dead-after; list/sum-payload siblings MOVE so
  ; balance). LEAK-side (value correct, no trap; seq-278). Real fix = the dup_sites-3690-3694
  ; surviving-slot-drop (v-mem co-design, in flight) → tightens to 0. Was (0, unpinned).
  (live-objects known-leak))

(case
  "a capturing closure stored in a tuple and also called directly folds through its capture"
  (doc
    "The SURVIVOR control: a TUPLE element is a fixed-shape UNBOXED rep, so storing `f1` there needs
           no boxed-cell materialization and the direct call folds cleanly (this always compiled). Pins that
           the force-keep does NOT over-fire on the tuple/record shapes — they stay on the fold path. `main
           5` → discarded `(tuple f1 9)`, then `f1 5 = 105`.")
  (input
    (do
      (def
        (main (: d Int64))
        (let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do #tuple(f1 9) (f1 d)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64))
  (live-objects known-leak))

(case
  "a non-capturing closure stored and also called directly emits a valid artifact"
  (doc
    "The non-capturing control: `f1 = (fn (v) (+ 1 v))` closes over nothing, so it needs no env cell
           and was never subject to the capture-occurrence poison — it stays on its existing path. Pins that
           the force-keep is scoped to CAPTURING closures. `main 5` → discarded insert, then `f1 5 = 6`.")
  (input
    (do
      (def (main (: d Int64)) (let ((f1 (fn ((: v Int64)) (+ 1 v)))) (do #map((= 1 f1)) (f1 d))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

; A lambda that references an ENCLOSING binding and is applied INSIDE that binding's scope — the capture
; is a free variable bound further out, not inside the lambda's own body. core-semantics.md §A Function
; Value Captures The Bindings In Scope Where It Is Created: `(+ x k)` reads `k` from the enclosing `let`.
; Applying the lambda β-reduces `(+ 5 k)` and `k` must still resolve to that enclosing `k` — the free
; variable is PRESERVED across the reduction, not lost. (A generation that copied the free name into an
; orphan scope would report `k` unbound; this pins that a captured enclosing binding survives.)
(case
  "a lambda applied in the scope of the binding it captures observes that binding"
  (doc
    "`(let ((k 10)) ((fn (x) (+ x k)) 5))` — the lambda captures `k` from the enclosing `let` and is
           applied to 5 inside that `let`. The application reduces to `(+ 5 k)` with `k = 10`, yielding
           15. The captured free variable `k` binds OUTSIDE the lambda body, so β-reducing the application
           must preserve its resolution to the enclosing `let`, not lose it.")
  (input (let ((k 10)) ((fn ((: x Int64)) (+ x k)) 5)))
  (output (: 15 Int64)))

(case
  "a lambda captures an enclosing function parameter and is applied in its body"
  (doc
    "The same capture over a def PARAMETER rather than a `let`: `(def (f k) ((fn (x) (+ x k)) 5))`
           — the lambda captures `f`'s parameter `k` and is applied inside `f`'s body. `f(10)` reduces
           `(+ 5 k)` with `k = 10` = 15. Pins that an enclosing PARAMETER is captured and preserved
           across the β-reduction exactly as an enclosing `let` binding is.")
  (input
    (do
      (def (f (: k Int64)) ((fn ((: x Int64)) (+ x k)) 5))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 10 Int64))
  (output (: 15 Int64)))

(case
  "an inner lambda captures an enclosing match-arm binder and is applied"
  (doc
    "The same capture over a MATCH-ARM binder rather than a `let` or a parameter: the `A` arm binds
           `m` = 7, then an inner lambda `(fn (x) (+ x m))` captures `m` and is applied to 3, giving
           3 + 7 = 10. A match-arm binder must be visible to an inner lambda's capture exactly as a `let`
           binding or a parameter is (both above), and as `m` is when used directly `(+ m 3)`. The binder
           resolves to a `SumPayload` reading the arm's scrutinee; that resolution must be PINNED as a
           capture so β-reducing the applied inner lambda preserves it, rather than copying the reference
           into the reduced body where it re-resolves unbound (which rejected a valid program CDZ0101).")
  (input
    (do
      (type C (A Int64) (B))
      (def (main) (match (A 7) ((A m) ((fn (x) (+ x m)) 3)) ((B) 0)))
      (export main)))
  (output (: 10 Int64)))

(case
  "an inner lambda captures an enclosing tuple-pattern binder and is applied"
  (doc
    "The tuple-pattern companion: matching `(tuple 7 9)` binds `a` = 7 (an `Elem`-path binder), and an
           inner lambda `(fn (x) (+ x a))` captures `a` and is applied to 3 → 10. Pins that the capture of a
           pattern binder is general to a tuple-slot binder, not only a variant payload — both resolve to a
           `SumPayload` (bare `Elem` vs `Payload` path) and both must be pinned as a capture.")
  (input (do (def (main) (match #tuple(7 9) (#tuple(a b) ((fn (x) (+ x a)) 3)))) (export main)))
  (output (: 10 Int64)))

; A capturing lambda BOUND to a name (a `let` binding) and then applied — `(let ((g (fn (x) (+ x k))))
; (g 5))` where `g` closes over an enclosing `k`. Binding the closure to a name does not change that it
; folds when applied: `g` is copy-propagated (a lambda value is never kept as a runtime slot), so `(g 5)`
; β-reduces to `(+ 5 k)` and `k` resolves to its enclosing binding. Pins that a NAMED capturing closure
; applied directly folds exactly as the anonymous form does.
(case
  "a named capturing closure applied directly folds through its capture"
  (doc
    "`(let ((k 10)) (let ((g (fn (x) (+ x k)))) (g 5)))` — `g` is a let-bound closure capturing the
           outer `k`; applying it yields (+ 5 10) = 15. A NAMED capturing closure applied directly must
           fold like the anonymous `((fn (x) (+ x k)) 5)` form — the name binding is transparent.")
  (input (let ((k 10)) (let ((g (fn ((: x Int64)) (+ x k)))) (g 5))))
  (output (: 15 Int64)))

(case
  "a named capturing closure applied more than once folds at each use"
  (doc
    "The same named closure `g` applied twice — `(+ (g 5) (g 6))` with `g = (fn (x) (+ x k))`,
           k = 10 — folds each application: (5+10) + (6+10) = 31. Two uses of a capturing closure each
           β-reduce independently; the closure value is not built at run time.")
  (input (let ((k 10)) (let ((g (fn ((: x Int64)) (+ x k)))) (+ (g 5) (g 6)))))
  (output (: 31 Int64)))

; A closure factory — a function RETURNING a capturing closure — whose result is applied at the call
; site. `(mk k)` returns `(fn (x) (+ x k))` closing over `k`; `((mk 10) 5)` applies that returned
; closure. core-semantics.md §A Function Is A First-Class Value ("returned as a result") composed with
; capture: the returned closure carries `mk`'s parameter `k`. The whole chain folds — `mk` inlines,
; the returned lambda β-reduces — so no runtime closure survives.
(case
  "a closure factory's returned capturing closure is applied at the call site"
  (doc
    "`(def (mk k) (fn (x) (+ x k)))` returns a closure over `k`; `((mk 10) 5)` = (+ 5 10) = 15. The
           returned closure captures the factory's parameter and applies correctly — a returned closure
           composed with a capture, both folded away.")
  (input
    (do (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k))) (def (main) ((mk 10) 5)) (export main)))
  (output (: 15 Int64)))

; The SAME returned capturing closure, but BOUND with `let` before it is applied. `(let ((f (mk n))) (f 3))`
; must compute exactly as the inline `((mk n) 3)` above — the binding names the closure value but does not
; change its meaning. This was a MISCOMPILE (invalid wasm, silently written at exit 0): `should_keep_binding`
; short-circuits a syntactic `Resolved::Lambda` init to avoid a speculative lift that pollutes the capture
; set, but `(mk n)` is an `Apply` that REDUCES to a capturing lambda — it slipped past, was lifted, and
; recorded the captured `n`; the copy-propagated `((mk n) 3)` then β-reduced to `(+ n 3)` with the shared `n`
; lowered to a `Core::Captured` env-read in `main` (no env) → an i32/i64 slot mismatch. The fix propagates a
; binding whose value reduces to a lambda, so it folds inline like the un-bound form. Every relaxation of the
; trigger (inline application, a higher-order argument, a non-capturing closure) already worked; this pins the
; let-bound one.
(case
  "a returned capturing closure bound with let and applied folds like the inline form"
  (doc
    "`(mk n)` returns `(fn (x) (+ n x))` capturing the parameter `n`; `(let ((f (mk n))) (f 3))` binds
           that closure to `f` and applies it, so with `n` = 10 the result is 10 + 3 = 13 — identical to the
           inline `((mk n) 3)`. A `let`-bound closure value round-trips through its binding at the value's
           own representation, not the default scalar width; a binding whose init reduces to a lambda is
           copy-propagated so its application folds inline rather than mis-lifting the closure into the local.")
  (input
    (do
      (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x)))
      (def (main (: n Int64)) (let ((f (mk n))) (f 3)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 13 Int64)))

(case
  "a returned capturing closure bound with let and applied twice folds each application independently"
  (doc
    "The same `(mk n)` returning `(fn (x) (+ n x))`, bound once with `let` and applied TWICE — each
           application folds independently: `(let ((f (mk n))) (+ (f 3) (f 4)))` with n = 10 is (10+3) +
           (10+4) = 27. Pins that one let-bound closure binding folds correctly across multiple uses.")
  (input
    (do
      (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x)))
      (def (main (: n Int64)) (let ((f (mk n))) (+ (f 3) (f 4))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 27 Int64)))

(case
  "a returned multi-parameter capturing closure applied at full arity (flat) folds"
  (doc
    "`(mk n)` returns a TWO-parameter capturing closure `(fn (x y) (+ n (+ x y)))`; bound with `let`
           and applied at full arity flat `(f 3 4)` with n = 10 is 10 + 3 + 4 = 17.")
  (input
    (do
      (def (mk (: n Int64)) (fn (x y) (+ n (+ x y))))
      (def (main (: n Int64)) (let ((f (mk n))) (f 3 4)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 17 Int64)))

(case
  "a returned multi-parameter capturing closure applied curried folds"
  (doc
    "The curried-application face of the returned two-parameter capturing closure: `((f 3) 4)`
           reaches full arity through a partial application; `(fn (x y) (+ n (+ x y)))` with n = 10 is
           10 + 3 + 4 = 17 — identical to the flat `(f 3 4)`.")
  (input
    (do
      (def (mk (: n Int64)) (fn (x y) (+ n (+ x y))))
      (def (main (: n Int64)) (let ((f (mk n))) ((f 3) 4)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 17 Int64)))

; A closure STORED IN A VARIANT FIELD, then matched out and applied — the ad-hoc-polymorphism dispatch
; shape (a "protocol" = a variant/record of closures). The closure captures `k`, so it CANNOT β-reduce
; away (it survives as a runtime closure value flowing through the `Box.Mk` destructure); yet its
; CONSTRUCTOR SITE is statically visible at the call (`mk` inlines, so the `Box.Mk(<closure>)` payload
; the `match` binds is a compile-time-known `Core::Closure`). The KNOWN-CLOSURE DEVIRTUALIZATION resolves
; the funcref table slot at compile time and emits a DIRECT call to the lifted function instead of a
; `call_indirect` (the wasm-side witness pins 0 `call_indirect` in a rcdzc unit test; here the value
; parity is the behavior witness that devirtualizing keeps the exact result). Zero-cost ad-hoc-poly
; dispatch — the mechanism the whole const-record/variant-of-closures language model rests on.
(case
  "a capturing closure stored in a variant is applied via a devirtualized direct call"
  (doc
    "`(mk 5)` builds `Box.Mk((fn (n) (+ n k)))` capturing k=5; `(use2 (mk 5))` matches the variant to
           bind the closure `f` and applies it twice: (10+5) + (20+5) = 15 + 25 = 40. The closure captures
           `k` so it survives to run time (does not β-reduce), but its constructor site is visible at the
           call, so the call devirtualizes to a direct call of the lifted function (no `call_indirect`).
           Value parity (40) proves the devirtualized direct call computes the identical result.")
  (input
    (do
      (type Box (Mk (-> Int64 Int64)))
      (def (mk (: k Int64)) (Box.Mk (fn ((: n Int64)) (+ n k))))
      (def (use2 (: b Box)) (match b ((Box.Mk f) (+ (f 10) (f 20)))))
      (def (main) (use2 (mk 5)))
      (export main)))
  (output (: 40 Int64)))

; A recursive driver with a `const` CLOSURE parameter that re-passes the closure to ITSELF unchanged — the
; iterator-fold shape. A const-param function is specialize-at-each-call: at the concrete call the const
; closure binds to `step`, and the self-recursive identity re-pass threads that SAME closure through the
; recursion, so it specializes + devirtualizes (S1) + fuses. The standalone generic body cannot bind the
; unbound const param, but that is not an ill-formedness — a false-positive decline that the identity-re-pass
; exemption turns into a plain decline (not a fault), so the program compiles + runs. Value parity is the
; behavior witness (a wrong specialization would miscompute); the 0-`call_indirect` fusion is pinned by a
; rcdzc emit-shape unit test. `count` sums a list via `step` = pop-head: 1+2+3+4 = 10.
(case
  "a const closure re-passed through a recursive driver specializes and computes correctly"
  (doc
    "`count` is a recursive fold whose `step` is a `const` closure it re-passes to itself unchanged;
           `main` calls it with a pop-head closure over `(list 1 2 3 4)`, summing to 10. The const-closure
           self-recursive identity re-pass used to DECLINE (CDZ0201) because the standalone generic body
           can't bind the unbound `step` — a false positive, since every concrete call specializes it. The
           identity-re-pass exemption makes that a plain decline (callers specialize), so this compiles and
           runs; the concrete call threads the closure through the recursion and fuses. 1+2+3+4 = 10.")
  (input
    (do
      (def
        (count
          (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
          (: s (List Int64))
          (: acc Int64))
        (match
          (step s)
          ((Option.None) acc)
          ((Option.Some p) (match p (#tuple(x s2) (count step s2 (+ acc x)))))))
      (def
        (main)
        (count
          (fn
            ((: s (List Int64)))
            (match s (#list() (Option.None)) (#list(h (.. t)) (Option.Some #tuple(h t)))))
          #list(1 2 3 4)
          0))
      (export main)))
  (output (: 10 Int64))
  (live-objects known-leak))

; TWO NESTED recursive const-closure drivers — `filter-step` re-passes its const `step` closure AND is
; itself the const step a `drive` fold consumes. This composition (a const-driver whose closure is
; another const-driver's specialized output) emitted INVALID WASM ("function index out of bounds") — a
; nested specialization the layout reachability walk appended to `order` without walking ITS callees, so a
; `Core::Call` targeted an un-laid-out function slot. The layout fix (a joint fixpoint of the call- and
; lifted-closure worklists) reaches the nested spec's callees. Value parity is the witness the fusion is
; correct; that the module is valid wasm is the primary fix (a rcdzc test asserts 0 call_indirect + it
; validates). `[1,2,3,4,5]` kept (> 2) then summed: 3+4+5 = 12.
(case
  "two nested recursive const-closure drivers (filter under fold) emit valid wasm and compute correctly"
  (doc
    "A `filter` adapter whose recursive `filter-step` takes a `const` step closure, consumed by a
           recursive `drive` fold that ALSO takes its step `const` — two nested recursive const-closure
           specializations. This used to emit INVALID WASM (a nested spec's `Core::Call` referenced an
           un-laid-out function index) because the layout reachability walk appended the nested spec to the
           emission order without closing over its own callees. The joint call/lifted-closure fixpoint
           reaches them. Filters `(list 1 2 3 4 5)` to elements > 2 then sums: 3 + 4 + 5 = 12.")
  (input
    (do
      (type It (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
      (def
        (from-list (: xs (List Int64)))
        (It.Mk
          xs
          (fn
            ((: s (List Int64)))
            (match s (#list() (Option.None)) (#list(h (.. t)) (Option.Some #tuple(h t)))))))
      (def
        (filter-step
          (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
          (: s (List Int64))
          (const (: p (-> Int64 Bool))))
        (match
          (step s)
          ((Option.None) (Option.None))
          ((Option.Some pr)
            (match pr (#tuple(x s2) (if (p x) (Option.Some #tuple(x s2)) (filter-step step s2 p)))))))
      (def
        (filter (: it It) (: p (-> Int64 Bool)))
        (match it ((It.Mk s0 step) (It.Mk s0 (fn ((: s (List Int64))) (filter-step step s p))))))
      (def
        (drive
          (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
          (: s (List Int64))
          (: acc Int64))
        (match
          (step s)
          ((Option.None) acc)
          ((Option.Some p) (match p (#tuple(x s2) (drive step s2 (+ acc x)))))))
      (def (sum (: it It)) (match it ((It.Mk s step) (drive step s 0))))
      (def (main) (sum (filter (from-list #list(1 2 3 4 5)) (fn ((: x Int64)) (> x 2)))))
      (export main)))
  (output (: 12 Int64))
  (live-objects known-leak))

; TWO specializations of ONE driver on DIFFERENT closures must not collide in the spec memo. Both `em`
; and `ef` call the same `sum`→`drive` (a `const`-closure recursive fold), but with different wrapped step
; closures — `em` a `map` chain, `ef` a `filter` chain. The const-arg memo fingerprint used the AST of the
; arg expression, which for the bare `step` reference is just the NAME (identical for both), collapsing the
; two specializations to one key: the SECOND export reused the FIRST's spec → a WRONG VALUE (`ef` returned
; `em`'s result). Fixed by keying a function-typed const arg's fingerprint on its resolved closure identity
; (lifted `code` + captures). Each export must compute its OWN result: `em` = map(+10)|>sum = 11+12+13 = 36;
; `ef` = filter(>2)|>sum = 3+4+5 = 12. A collision returns one export's value for the other.
(case
  "two specializations of one driver on different closures do not collide in the spec memo"
  (doc
    "`em` and `ef` both drive the same `sum`/`drive` const-closure fold but over a `map` chain vs a
           `filter` chain — two specializations of `drive` on DIFFERENT step closures. A spec-memo collision
           (the const arg fingerprinted by its AST name `step`, identical for both) made the second export
           reuse the first's specialization → wrong value. Keyed on the resolved closure identity, each
           computes its own: `em` = sum(map([1,2,3], +10)) = 36; `ef` = sum(filter([1..5], >2)) = 12.")
  (input
    (do
      (type It (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
      (def
        (from-list (: xs (List Int64)))
        (It.Mk
          xs
          (fn
            ((: s (List Int64)))
            (match s (#list() (Option.None)) (#list(h (.. t)) (Option.Some #tuple(h t)))))))
      (def
        (map (: it It) (: f (-> Int64 Int64)))
        (match
          it
          ((It.Mk s0 step)
            (It.Mk
              s0
              (fn
                ((: s (List Int64)))
                (match
                  (step s)
                  ((Option.None) (Option.None))
                  ((Option.Some p) (match p (#tuple(x s2) (Option.Some #tuple((f x) s2)))))))))))
      (def
        (filter-step
          (: step (-> (List Int64) (Option (Tuple Int64 (List Int64)))))
          (: s (List Int64))
          (const (: p (-> Int64 Bool))))
        (match
          (step s)
          ((Option.None) (Option.None))
          ((Option.Some pr)
            (match pr (#tuple(x s2) (if (p x) (Option.Some #tuple(x s2)) (filter-step step s2 p)))))))
      (def
        (filter (: it It) (: p (-> Int64 Bool)))
        (match it ((It.Mk s0 step) (It.Mk s0 (fn ((: s (List Int64))) (filter-step step s p))))))
      (def
        (drive
          (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
          (: s (List Int64))
          (: acc Int64)
          (const (: g (-> Int64 Int64 Int64))))
        (match
          (step s)
          ((Option.None) acc)
          ((Option.Some p) (match p (#tuple(x s2) (drive step s2 (g acc x) g))))))
      (def
        (sum (: it It))
        (match it ((It.Mk s step) (drive step s 0 (fn ((: a Int64) (: x Int64)) (+ a x))))))
      (def (em) (sum (map (from-list #list(1 2 3)) (fn ((: x Int64)) (+ x 10)))))
      (def (ef) (sum (filter (from-list #list(1 2 3 4 5)) (fn ((: x Int64)) (> x 2)))))
      (export em)
      (export ef)))
  (call em)
  (output (: 36 Int64))
  (call ef)
  (output (: 12 Int64))
  (live-objects known-leak))

; A CLOSED literal closure FORWARDED through several `const`-parameter call hops (`sum` → `fold` →
; `drive`'s `const g`) — the const-WRAPPER-CHAIN. This used to be a false CDZ0201 reject: a β-substitution
; splices the source lambda `(fn (a x) (+ a x))` into a specialization copy and STEALS its single arena
; parent pointer (one mutable parent per node), so `arg_captures_runtime_binding`'s `is_within` walk sees
; the lambda's OWN params `a`/`x` as "outside" it → a spurious runtime-capture → a reject on a genuinely
; compile-time-known closure. Fixed by a theft-immune lexical free-NAME closedness cross-check (a bare name
; that is neither the lambda's own param nor a resolvable global is the only real capture). Distinct from
; the "re-passed through a recursive driver" case above (that is a bare self-repass of the callee's OWN
; const param; this FORWARDS a closed literal down two intermediate const params before the recursion). The
; derived-closure divergence guard (a re-pass building a NEW closure each depth) stays intact — only a
; genuinely CLOSED forwarded closure is accepted. `sum(from-list([1,2,3]))` folds with `+`: 1+2+3 = 6.
(case
  "a closed literal closure forwarded through const-wrapper hops is not a false reject"
  (doc
    "A closed closure `(fn (a x) (+ a x))` forwarded through `sum` → `fold` → `drive`'s `const g`
           parameter. The const-wrapper chain used to falsely reject CDZ0201: a β-substitution spliced the
           source lambda into a spec copy and stole its arena parent pointer, so the capture check saw the
           lambda's own params `a`/`x` as an outside (runtime) capture. A theft-immune lexical free-name
           closedness check accepts the genuinely-closed forwarded closure, so it specializes and computes.
           sum(from-list([1,2,3])) = 1+2+3 = 6.")
  (input
    (do
      (type Iter (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
      (def
        (from-list (: xs (List Int64)))
        (Iter.Mk
          xs
          (fn
            ((: s (List Int64)))
            (match s (#list() (Option.None)) (#list(h (.. t)) (Option.Some #tuple(h t)))))))
      (def
        (drive
          (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64))))))
          (: s (List Int64))
          (: acc Int64)
          (const (: g (-> Int64 Int64 Int64))))
        (match
          (step s)
          ((Option.None) acc)
          ((Option.Some p) (match p (#tuple(x s2) (drive step s2 (g acc x) g))))))
      (def
        (fold (: it Iter) (: acc Int64) (const (: g (-> Int64 Int64 Int64))))
        (match it ((Iter.Mk s step) (drive step s acc g))))
      (def (sum (: it Iter)) (fold it 0 (fn ((: a Int64) (: x Int64)) (+ a x))))
      (def (main) (sum (from-list #list(1 2 3))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a let-bound returned closure applied twice folds each application independently"
  (doc
    "The multi-use companion: `(let ((f (mk n))) (+ (f 3) (f 4)))` binds the returned capturing closure
           once and applies it twice; each application folds independently, so with `n` = 10 the result is
           (10 + 3) + (10 + 4) = 27. Pins that binding a closure value and applying it more than once keeps
           each use correct — the multi-reference case of the let-bound-closure fold.")
  (input
    (do
      (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x)))
      (def (main (: n Int64)) (let ((f (mk n))) (+ (f 3) (f 4))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 27 Int64)))

(case
  "a capturing closure stored in a tuple is extracted and applied"
  (doc
    "A capturing closure `(fn (x) (+ x k))` (over an enclosing `k = 7`) stored as a tuple element,
           projected out, and applied: `((. (tuple (fn (x) (+ x k)) 9) 0) 5)` = (+ 5 7) = 12. Storing a
           capturing closure in a data structure and reading it back preserves its capture — the whole
           thing folds (the tuple projection reaches the closure, which β-reduces).")
  (input (let ((k 7)) ((. #tuple((fn ((: x Int64)) (+ x k)) 9) 0) 5)))
  (output (: 12 Int64)))

; The LET-BOUND compound face of the same fold. A capturing closure stored in a compound that is BOUND with
; `let`, projected, and applied — `(let ((r (record (f (fn (x) (+ x n)))))) ((. r f) 10))` — must behave
; EXACTLY like the inline-compound case above and the direct-let closure. This was a BOTH-BACKEND MISCOMPILE
; (invalid wasm at exit 0 — `func … type mismatch: expected i32, found i64` — and rust E0425 on an unbound
; `__cap0`): the `let` keep-analysis (`should_keep_binding`) already short-circuits a `Resolved::Lambda` init
; and one that REDUCES to a lambda, precisely to avoid a speculative `core_of(init)` that LIFTS the closure
; and pollutes the capture set — but a COMPOUND (record/tuple) init CONTAINING a lambda is neither, so it
; slipped past, was lowered (lifting the contained closure), and recorded the captured `n`. The projection
; `(. r f)` then β-reduced inline to `(+ 10 n)` reusing the SHARED `n` occurrence, which the stale
; `captured_ref` entry lowered to a `Core::Captured` env-read in `main` (no closure env) → the slot mismatch.
; The fix propagates a projection-only compound-holding-a-closure binding (`compound_contains_lambda`, a
; lift-free reduce), so each projection folds through exactly as the inline-compound and direct-let forms do.
(case
  "a capturing closure stored in a let-bound record is projected and applied"
  (doc
    "`(let ((r (record (f (fn (x) (+ x n)))))) ((. r f) 10))` — the closure captures the def parameter
           `n`, is stored in a record field, the record is `let`-bound, and the projected function is applied.
           Must fold exactly like the inline-record and direct-let controls (the call sees the capture):
           n = 1 → 10 + 1 = 11. Pins the both-backend miscompile where the compound binding's contained
           closure was speculatively lifted, poisoning the inline fold of the projected closure.")
  (input
    (do (def (main (: n Int64)) (let ((r #record((= f (fn (x) (+ x n)))))) (r.f 10))) (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64)))

(case
  "a capturing closure stored in a let-bound tuple is projected and applied"
  (doc
    "The TUPLE face of the let-bound compound fold: `(let ((r (tuple (fn (x) (+ x n)) 9))) ((. r 0) 10))`
           stores the capturing closure as tuple element 0, `let`-binds the tuple, projects, and applies —
           n = 1 → 11. Same root cause as the record case (a compound init holding a closure must fold
           through, not lift); pins the tuple projection path alongside the record one.")
  (input
    (do (def (main (: n Int64)) (let ((r #tuple((fn (x) (+ x n)) 9))) ((. r 0) 10))) (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64)))

(case
  "a let-bound record's projected capturing closure applied twice folds each application"
  (doc
    "The compound-held closure projected and applied MORE THAN ONCE through the one binding — each
           projection folds independently: `(let ((r (record (= f (fn (x) (+ x n)))))) (+ ((. r f) 10) ((. r
           f) 20)))` with n = 1 is (10+1) + (20+1) = 32. Pins the compound-binding fold across repeated
           projections, alongside the single-projection record/tuple cases above.")
  (input
    (do
      (def (main (: n Int64)) (let ((r #record((= f (fn (x) (+ x n)))))) (+ (r.f 10) (r.f 20))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 32 Int64)))

(case
  "a let-bound record with an extra field capturing a let-local is projected and applied"
  (doc
    "Two variants of the compound-held-closure fold in one program: the capture is a LET-LOCAL (`k`)
           rather than a def parameter, and the record carries an EXTRA plain field beside the closure.
           `(let ((k 7)) (let ((r (record (= f (fn (x) (+ x k))) (= g 99)))) ((. r f) 5)))` = 5 + 7 = 12.
           Pins that the projection fold is agnostic to the capture's binder kind and to sibling fields.")
  (input
    (do
      (def (main) (let ((k 7)) (let ((r #record((= f (fn (x) (+ x k))) (= g 99)))) (r.f 5))))
      (export main)))
  (output (: 12 Int64)))

; The CONTROL-FLOW face of the let-bound-closure fold: a `let` whose init is an `if`-JOIN of two CAPTURING
; lambdas, then called — `(let ((f (if b (fn (x) (+ x n)) (fn (x) (* x n))))) (f 10))`, the runtime-select
; twin of the compound-held-closure binding above. This was the 4th face of the both-backend
; speculative-lift MISCOMPILE (wasm: b=true trapped `unreachable`, b=false returned 0 — the capture env
; lost; rust: E0425 on an unbound `__cap0`): `should_keep_binding` short-circuits a `Resolved::Lambda`
; init, a lambda-reducing init, and a compound-holding-a-closure init — but an `if`/`match` whose ARMS are
; lambdas is none of those, so it slipped past, `core_of` LOWERED the conditional (lifting each arm's
; closure, recording the captured `n`), and the case-of-case rewrite β-reduced the selected arm inline
; (`(if b (+ 10 n) (* 10 n))`) reusing the SHARED `n` occurrences — which the stale `captured_ref` lowered
; to a `Core::Captured` env-read in `main` (no closure env). The fix adds an `if_or_match_selects_lambda`
; short-circuit (a lift-free reduce), so a conditionally-chosen closure is copy-propagated and its
; application folds inline through the case-of-case rewrite exactly as the non-capturing-join,
; record-field-held, and direct-let controls do. n = 5: b=true → 10 + 5 = 15, b=false → 10 * 5 = 50.
(case
  "an if-join of two capturing lambdas let-bound and called applies the selected closure"
  (doc
    "`(let ((f (if b (fn (x) (+ x n)) (fn (x) (* x n))))) (f 10))` with `n` a def param — conditional
           closure choice, the runtime-select twin of the pinned single-lambda and compound-held-lambda
           bindings. Both arms capture the enclosing `n`; the selected one β-reduces inline against the call
           arg. n = 5: b=true → 10 + 5 = 15, b=false → 10 * 5 = 50. Pins the 4th face of the speculative-lift
           family, where an `if`-of-lambdas init was lowered (lifting the arm closures) and the inline fold
           of the selected arm read a nonexistent capture env (wasm trap / wrong value, rust unbound cap).")
  (input
    (do
      (def
        (main (: b Bool) (: n Int64))
        (let ((f (if b (fn ((: x Int64)) (+ x n)) (fn ((: x Int64)) (* x n))))) (f 10)))
      (export main)))
  (call main (: true Bool) (: 5 Int64))
  (output (: 15 Int64))
  (call main (: false Bool) (: 5 Int64))
  (output (: 50 Int64)))

(case
  "an if-join of two identical capturing lambdas still reads the capture on the false arm"
  (doc
    "The same-body face of the if-join fold: BOTH arms are `(fn (x) (+ x n))`, so the selected
           closure is `(+ 10 n)` regardless of `b`. A speculative lift that dropped the capture read `n`
           as 0 on the false arm — b=false → 10 instead of 15. n = 5, b=false → 10 + 5 = 15 (identical to
           b=true). Pins that both identical arms fold with the capture intact (a breaker-listed regression).")
  (input
    (do
      (def
        (main (: b Bool) (: n Int64))
        (let ((f (if b (fn ((: x Int64)) (+ x n)) (fn ((: x Int64)) (+ x n))))) (f 10)))
      (export main)))
  (call main (: false Bool) (: 5 Int64))
  (output (: 15 Int64)))

; The MATCH-JOIN twin of the case above — the SAME `if_or_match_selects_lambda` short-circuit covers it. A
; `let` whose init is a `match` selecting one of two capturing lambdas, then called. On trunk this was even
; WORSE than the if-join: INVALID WASM (`wasm[0]::function[2]`) for BOTH selectors, where the if-join at
; least ran one arm and returned a wrong value. Same root cause (the join arms' closures were speculatively
; lifted, then β-inlined at the call site reading a nonexistent capture env); the fix keeps a conditionally-
; chosen closure lift-free whether the conditional is an `if` or a `match`. The scrutinee is a custom sum
; built inside the module (`(pick b)` → `Sel.A`/`Sel.B`) so the case reaches full lowering — an int-literal
; match pattern isn't lowered yet, and a sum entry-arg doesn't cross the host boundary, both pre-existing
; gaps unrelated to the lambda-join. n = 5: A → 10 + 5 = 15, B → 10 * 5 = 50.
(case
  "a match-join of two capturing lambdas let-bound and called applies the selected closure"
  (doc
    "`(let ((f (match (pick b) ((Sel.A _) (fn (x) (+ x n))) ((Sel.B _) (fn (x) (* x n)))))) (f 10))`
           — the `match` analogue of the if-join, both arms capturing the def parameter `n`; the case-of-
           match rewrite β-reduces the selected arm inline. `pick` returns a custom sum so the match reaches
           full lowering (a supported pattern kind). n = 5: A → 10 + 5 = 15, B → 10 * 5 = 50. On trunk this
           was INVALID WASM for both selectors — the match-join cell of the speculative-lift family, closed
           by the same `if_or_match_selects_lambda` fix as the if-join.")
  (input
    (do
      (type Sel (A) (B))
      (def (pick (: b Bool)) (if b (Sel.A) (Sel.B)))
      (def
        (main (: b Bool) (: n Int64))
        (let
          ((f
              (match
                (pick b)
                ((Sel.A _) (fn ((: x Int64)) (+ x n)))
                ((Sel.B _) (fn ((: x Int64)) (* x n))))))
          (f 10)))
      (export main)))
  (call main (: true Bool) (: 5 Int64))
  (output (: 15 Int64))
  (call main (: false Bool) (: 5 Int64))
  (output (: 50 Int64)))

; The case above FOLDS (a constant tuple whose closure β-reduces through the projection). These two exercise
; the RUNTIME closure-in-heap-compound REP: closures bound via `let` into a runtime list/tuple, extracted by
; `List.at`/a pattern binder, and applied — a `call_indirect` on a heap-stored closure cell, not a fold. This
; path declined on the rust backend ("a closure whose function type is not fully solved here has no native
; Rust representation") until the closure's dyn-Fn type was grounded from the LIFTED LAMBDA's own concrete
; types (not `type_of`, which is non-concrete at a compound-element position). Now it runs on wasm + the
; RUST backend, keeping each closure's distinct capture. (rust-ASYNC still declines at emit_lifted_lambda — a
; distinct not-yet-built path — so these grade `todo` there, pending that increment.)
(case
  "a capturing closure stored as a runtime list element is extracted by List.at and applied"
  (doc
    "`(adder n) = (fn (x) (+ x n))`; a runtime list `(list (adder 1) (adder 2))` holds two capturing
           closures; `List.at fs 0` reads the first out as an Option and the arm applies it: `((adder 1)
           10)` = 11. Exercises the runtime closure-in-LIST-element representation — a heap-stored closure
           cell read back and applied (`call_indirect`), NOT a foldable constant projection. Pins that a
           closure survives being stored in and read back out of a runtime list on wasm and rust (the
           rep-grounding fix); rust-async declines at emit_lifted_lambda (a later increment).")
  (input
    (do
      (def (adder n) (fn (x) (+ x n)))
      (def
        (main)
        (let
          ((fs #list((adder 1) (adder 2))))
          (match (List.at fs 0) ((Some f) (f 10)) ((None u) -1))))
      (export main)))
  (output (: 11 Int64))
  ; #6049 FIXED (v-core-opt 2026-08-30, v-mem co-verified): a closure extracted from a runtime LIST + applied is now
  ; reclaimed to 0. #6022 (is_heap_type Ty::Fn=>true) made the closure a Perceus retain candidate (closing the earlier
  ; UAF); the residual leak was the owned List.at Some-shell left un-dropped because arm_borrows_heap_subvalue treated
  ; the closure-apply (f 10) as a consuming escape. FIX: applying a closure BORROWS its callee (CallClosure), so the
  ; owned Some-shell now deep-drops after the apply and its cascade reclaims the closure cell + boxed capture. Was
  ; known-leak-2; verified 0 on the debug-counters runtime + the corpus live-objects gate.
  (live-objects 0))

(case
  "a closure stored as a MAP value is looked up by a runtime key and applied"
  (doc
    "The dispatch-TABLE idiom: closures stored as MAP values, one selected by a runtime KEY and
           applied — `{1 → (· 10), 2 → (+ 100)}`, `(match (Map.lookup m k) ((Some f) (f x)) …)`. k=1
           applies the multiplier (50), k=2 the adder (105), and a missing key falls to the None arm
           (-1) — the lookup's Option wraps a FUNCTION payload, so the Some binder carries an applyable
           closure out of the CHAMP. The map twin of the list-of-closures index dispatch above: there
           selection is positional, here it is by key hash/compare, and the closure must survive the
           CHAMP value cell (boxed function handle) round-trip. Expected: 50, 105, -1.")
  (input
    (do
      (def
        (main (: k Int64) (: x Int64))
        (let
          ((m (Map.insert #map((= 1 (fn ((: v Int64)) (* v 10)))) 2 (fn ((: v Int64)) (+ v 100)))))
          (match (Map.lookup m k) ((Some f) (f x)) ((None u) -1))))
      (export main)))
  (call main (: 1 Int64) (: 5 Int64))
  (output (: 50 Int64))
  (call main (: 2 Int64) (: 5 Int64))
  (output (: 105 Int64))
  (call main (: 9 Int64) (: 5 Int64))
  (output (: -1 Int64))
  ; #6049 FIXED (v-core-opt 2026-08-30, v-mem co-verified): a closure looked up from a CHAMP MAP value + applied is
  ; now reclaimed to 0 (values 50/105/-1, incl the None arm). UAF was closed by #6022's Ty::Fn retain candidate; the
  ; residual leak was the owned Map.lookup Some-shell left un-dropped because arm_borrows_heap_subvalue treated the
  ; closure-apply (f x) as a consuming escape. FIX: applying a closure BORROWS its callee (CallClosure), so the owned
  ; Some-shell deep-drops after the apply and its cascade reclaims the closure cell + captures. Was known-leak-2;
  ; verified 0 on the debug-counters runtime + the corpus live-objects gate.
  (live-objects 0))

(case
  "two capturing closures stored as runtime tuple elements keep distinct captures"
  (doc
    "The tuple-element runtime companion: `(tuple (adder 1) (adder 2))` bound via `let` holds two
           closures with DISTINCT captures (n=1 and n=2); a tuple binder extracts both and each is applied to
           10 → `(+ 11 12)` = 23. Pins that two runtime closures in a heap tuple keep their SEPARATE captures
           (not unified/aliased to one) and each dispatches correctly — the runtime rep, not the folded
           constant-tuple projection above. Runs on wasm + rust; rust-async declines (later increment).")
  (input
    (do
      (def (adder n) (fn (x) (+ x n)))
      (def (main) (let ((#tuple(f g) #tuple((adder 1) (adder 2)))) (+ (f 10) (g 10))))
      (export main)))
  (output (: 23 Int64)))

(case
  "a closure carried in a sum payload is extracted by a match and applied"
  (doc
    "core-semantics.md §A Function Is A First-Class Value: a function stored in a SUM variant's
           payload — the callback-in-a-variant shape — is extracted by a match binder and applied.
           `(Some (fn (n) (* n 2)))` carries a closure; `(match … ((Some f) (f 5)) …)` binds `f` to the
           payload and applies it, yielding 10. The closure is reached through the variant PAYLOAD (a
           `sum-payload` heap read), not a `let`/tuple projection the fold reduces through, so its
           application is a runtime `call_indirect` on the extracted closure cell — the payload-binder
           analogue of applying a function-typed PARAMETER. Pins that a closure survives being stored in
           and read back out of a sum variant, and that a match binder over a function-typed payload is a
           callable runtime function-value source (not merely a foldable projection).")
  (input (match (Some (fn ((: n Int64)) (* n 2))) ((Some f) (f 5)) ((None _) 0)))
  (output (: 10 Int64)))

(case
  "a CAPTURING closure carried in a sum payload keeps its capture through the match binder"
  (doc
    "The capturing companion: the closure stored in the sum payload closes over a RUNTIME value, and
           that capture must survive being boxed into the variant and read back out. `(mk k)` returns
           `(Some (fn (x) (+ x k)))` capturing the parameter `k`; `(match (mk k) ((Some f) (f 5)) …)`
           extracts `f` and applies it, so with `k` = 100 the result is 5 + 100 = 105. The closure cell
           carried in the `Some` payload must retain its captured environment (not just the code pointer):
           a lowering that stored the function but dropped the capture would compute 5 (or read garbage).
           Pins that a closure's captured environment round-trips through a sum-variant payload, the
           capturing extension of the non-capturing payload-closure case above.")
  (input
    (do
      (def (mk (: k Int64)) (Some (fn ((: x Int64)) (+ x k))))
      (def (main (: k Int64)) (match (mk k) ((Some f) (f 5)) ((None _) -1)))
      (export main)))
  (call main (: 100 Int64))
  (output (: 105 Int64)))

(case
  "a closure carried in a USER-declared sum's payload is extracted and applied"
  (doc
    "The USER-SUM companion of the built-in-payload closure case: `(type T (Mk (-> Int64 Int64)))`
           declares a variant carrying a FUNCTION, and `(T.Mk (fn (n) (* n 2)))` stores a closure in it.
           `(match … ((T.Mk f) (f 5)))` extracts and applies it → 10. Unlike a built-in `Some`/`Ok`
           (whose ctor scheme threads the payload type so the extracted closure's application types
           directly), a USER variant's payload is a declared arrow `(-> Int64 Int64)` reached through the
           payload binder; applying it must peel that arrow to type the result. Pins that a closure
           carried in a user-declared sum applies exactly as one in a built-in sum — the callback-in-a-
           variant idiom a user's own event/AST types rely on. A generation without sum-type
           declaration declines it.")
  (input
    (do
      (type T (Mk (-> Int64 Int64)))
      (def (main) (match (T.Mk (fn ((: n Int64)) (* n 2))) ((T.Mk f) (f 5))))
      (export main)))
  (output (: 10 Int64)))

(case
  "a closure-payload sum picked by an if-helper, applied by a match-consumer, with a binding reused as both args"
  (doc
    "The β-copy face of the closure-in-sum idiom: a helper `mk` PICKS a closure-carrying variant via
           `if` (`(if true (Box.Fn g) (Box.Const 0))`), a consumer `run` MATCHES the sum and APPLIES the
           extracted closure `(f arg)`, and the caller reuses ONE binding `k` in BOTH the closure-producing
           `(mk (fn (x) (+ x k)))` AND the apply-arg `k`. This exact shape emitted a spurious CDZ0101 `unbound
           name k` FROM THE COMPILE BACKEND (`cdz check` passed — inference bound `k` fine — but `cdz compile`
           failed with no source location) because the closure-specialization β-copy of `mk`'s body inlined at
           the call site dropped `k` from scope; the fix preserves the reused binding through the β-copy.
           Drop any one leg (single-variant sum, a scalar `(V Int64)` payload, a direct `(Box.Fn (fn …))` with
           no `mk`/`if`, a literal 2nd arg, or a non-closure `(+ (dbl k) k)`) and it always compiled — the
           load-bearing combination is a closure-payload variant picked by an if-helper whose result is
           match-applied while the caller's binding flows into BOTH the helper and the apply. `mk` picks
           `Box.Fn(fn (x) (+ x 3))`, `run` applies it to 3 → 6.")
  (input
    (do
      (type Box (Fn (-> Int64 Int64)) (Const Int64))
      (def (mk (: g (-> Int64 Int64))) (if true (Box.Fn g) (Box.Const 0)))
      (def (run (: b Box) (: arg Int64)) (match b ((Box.Fn f) (f arg)) ((Box.Const c) c)))
      (def (main) (let ((k 3)) (run (mk (fn ((: x Int64)) (+ x k))) k)))
      (export main)))
  (output (: 6 Int64)))

; Two variants of ONE sum each box a closure of a DIFFERENT function type — a BINARY `(-> Int64 Int64
; Int64)` (`Bin`) and a UNARY `(-> Int64 Int64)` (`Un`). `run` matches the sum and, in EACH arm, applies
; that arm's boxed closure (`(f 2 3)` in `Bin`, `(g 9)` in `Un`) — so both arms carry a runtime
; `call_indirect`. But `main` constructs ONLY the `Bin` variant, so the only closure the program ever
; LIFTS is the binary one; no unary `(-> Int64 Int64)` closure value is ever built. A runtime closure
; value arises ONLY from a lambda lift, so the `Un` arm's `call_indirect` — dispatching a unary closure
; the program can never construct — is PROVABLY DEAD (its scrutinee can never be a `Un`). The backend
; must still EMIT that dead arm (selection is total over the match), which it does as an inert
; `unreachable`; a lowering that instead DEMANDED a matching lifted function type for the dead arm
; declined the whole program "a runtime closure application has no matching function type", even though
; the reachable `Bin` path is well-formed. Pins that coexisting boxed closures of DISTINCT function types
; in one sum compile as long as each APPLIED-and-reachable one is lifted — the unbuilt sibling's dispatch
; is dead, not a compile blocker (the shape lazy-iterator libraries hit when `scan` and `flat-map`
; combinators share one `Iter` sum).
(case
  "boxed closures of two fn types in one sum: the unbuilt variant's dispatch is dead"
  (doc
    "`(type Box (Bin (-> Int64 Int64 Int64)) (Un (-> Int64 Int64)))` — one sum boxing a BINARY and a
           UNARY closure. `run` applies the boxed closure in EACH arm, but `main` builds only `Bin`, so no
           unary closure is ever lifted and the `Un` arm's `call_indirect` is provably dead. `(Bin (fn (a
           x) (+ a x)))` run → `f 2 3` = 5. A backend that required a matching lifted function type for the
           dead `Un` arm declined the program; emitting the dead arm as `unreachable` lets the live `Bin`
           path compile and run.")
  (input
    (do
      (type Box (Bin (-> Int64 Int64 Int64)) (Un (-> Int64 Int64)))
      (def (run b) (match b ((Box.Bin f) (f 2 3)) ((Box.Un g) (g 9))))
      (def (main) (run (Box.Bin (fn ((: a Int64) (: x Int64)) (+ a x)))))
      (export main)))
  (output (: 5 Int64)))

; --- An UNANNOTATED closure typed from its STORAGE CONTEXT's declared arrow -----------------------
; The payload-closure cases above ANNOTATE the lambda's parameter (`(fn ((: n Int64)) …)`). But when a
; closure is stored in a position whose type is DECLARED — a variant constructor's payload
; `(-> Int64 C)`, a built-in `Some`/`Ok` payload — the parameter type need not be repeated: it is the
; arrow's parameter, threaded from the storage site into the lambda. core-semantics.md §A Function Is A
; First-Class Value + §Applying A Function Binds Its Parameter To Its Argument: a closure typed against
; the function type its context requires. (`type_of` computes a lambda's type bottom-up, so a bare `(fn
; (n) …)` whose body does not otherwise pin `n` stayed `Any` and declined "a closure's parameter type has
; no machine representation" / "a tuple element of type Any"; the expected-arrow fallback closes that.)
(case
  "an unannotated closure in a user variant payload is typed from the declared arrow"
  (doc
    "`(type T (Susp (-> Int64 C)))` declares a variant carrying a function `Int64 → C`. Storing
           `(T.Susp (fn (n) (C.A n)))` — the lambda's parameter UNANNOTATED — types `n : Int64` from the
           payload's declared arrow, not from a repeated annotation. Extracted by the match binder `f` and
           applied to 7, its `C.A` result matches the `(C.A m)` arm → 7. Pins that a closure stored in a
           declared-function-typed payload takes its parameter type from that declaration — the callback-
           in-a-variant idiom without redundant annotations.")
  (input
    (do
      (type C (A Int64) B)
      (type T (Susp (-> Int64 C)))
      (def
        (main)
        (match (T.Susp (fn (n) (C.A n))) ((T.Susp f) (match (f 7) ((C.A m) m) ((C.B) 0)))))
      (export main)))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "an unannotated closure in a Some payload is typed from the Option's element arrow"
  (doc
    "The built-in companion: `(Some (fn (n) (C.A n)))` carries an unannotated closure whose element
           type the `Some` payload fixes to the function `Int64 → C`, so `n : Int64` without annotation.
           Applied to 7 through the match binder → its `C.A` result yields 7. Pins the expected-arrow
           threading works for a built-in Option payload exactly as for a user variant.")
  (input
    (do
      (type C (A Int64) B)
      (def
        (main)
        (match (Some (fn (n) (C.A n))) ((Some f) (match (f 7) ((C.A m) m) ((C.B) 0))) ((None) 0)))
      (export main)))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "an unannotated closure with an unused parameter in a payload takes the declared parameter type"
  (doc
    "The lambda's parameter is not used by its body — `(fn (n) (C.B))` ignores `n` — so the body
           cannot constrain `n` at all; its type comes SOLELY from the payload's declared arrow `(-> Int64
           C)`. Without the expected-arrow fallback this declined 'a closure's parameter type has no
           machine representation' (nothing pinned `n`). Applied to 7, the body yields `C.B` → the `(C.B)`
           arm → 0. Pins that the declared arrow types even a body-unconstrained parameter.")
  (input
    (do
      (type C (A Int64) B)
      (type T (Susp (-> Int64 C)))
      (def (main) (match (T.Susp (fn (n) (C.B))) ((T.Susp f) (match (f 7) ((C.A m) m) ((C.B) 0)))))
      (export main)))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a capturing unannotated closure in a payload is typed from the declared arrow"
  (doc
    "The capturing extension: `(mk k)` returns `(T.Susp (fn (n) (C.A (+ n k))))` — an unannotated
           closure that CAPTURES the runtime parameter `k` AND takes its own parameter type from the
           payload arrow `(-> Int64 C)`. Extracted and applied to 7 with k = 100 → `C.A (7 + 100)` → 107.
           Pins that the storage-context parameter typing composes with capture — the closure retains its
           environment through the variant payload and still types its parameter from the declaration.")
  (input
    (do
      (type C (A Int64) B)
      (type T (Susp (-> Int64 C)))
      (def (mk (: k Int64)) (T.Susp (fn (n) (C.A (+ n k)))))
      (def (main (: k Int64)) (match (mk k) ((T.Susp f) (match (f 7) ((C.A m) m) ((C.B) 0)))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 107 Int64))
  (live-objects known-leak))

(case
  "an unannotated closure typed Int8 from context overflows a constant like an explicit Int8 param"
  (doc
    "The NARROW-WIDTH edge of context typing: `app : ((-> Int8 Int8)) -> Int8` applied `(app (fn (n)
           (+ n 1)))`, where `g` is applied to the constant 127. The unannotated `n` is typed Int8 from
           app's declared `(-> Int8 Int8)` arrow, so `(+ n 1)` with n=127 is `127 + 1 = 128`, which
           OVERFLOWS Int8 (max 127) — a constant OPERATION with no value → the SAME CDZ0304 (ConstTrap)
           the explicit `(fn ((: n Int8)) (+ n 1))` gives on the same constant. The recovered narrow width
           must reach the body's CONST-FOLD, not only the runtime path: without it the fold ran at the
           default Int64 and returned 128 (a wrong value where an overflow is due). A RUNTIME argument
           traps for both the annotated and unannotated forms; this pins that the compile-time const-fold
           carries the context width too.")
  (input
    (do (def (app (: g (-> Int8 Int8))) (g 127)) (def (main) (app (fn (n) (+ n 1)))) (export main)))
  (error CDZ0304))

(case
  "an unannotated closure typed Int8 from context computes an in-range constant"
  (doc
    "The value companion: the SAME `(app (fn (n) (+ n 1)))` but `g` applied to 5 — `5 + 1 = 6` fits
           Int8, so the context-Int8 closure computes 6 rather than over-rejecting. Together with the
           overflow case above this pins that the recovered narrow width is applied to the const-fold in
           BOTH directions — an out-of-range constant rejects, an in-range one computes — exactly as an
           explicit Int8 param does.")
  (input
    (do (def (app (: g (-> Int8 Int8))) (g 5)) (def (main) (app (fn (n) (+ n 1)))) (export main)))
  (output (: 6 Int8)))

; --- A lambda that MATCHES ITS OWN PARAMETER, passed through a higher-order function -------------
; core-semantics.md §A Function Is A First-Class Value + §Applying A Function Binds Its Parameter To Its
; Argument: a callback that DESTRUCTURES its argument (`(fn (c) (match c …))`) is an ordinary first-class
; value — passed to a HOF that applies it to the HOF's own argument. When the HOF is itself inlined, the
; callback is applied through a NESTED β-reduction: the callback's parameter IS the match scrutinee, and
; the reduction substitutes the argument for it. A pattern binder in the callback body reads the scrutinee
; via a `SumPayload` (resolve Case 6); the reduction must re-resolve that binder against the SUBSTITUTED
; scrutinee, not share its pre-substitution occurrence (which, lowered standalone, is a slot-less
; parameter — the "no local slot" decline this pins closed). Distinct from a callback that only RETURNS
; or PROJECTS its parameter (no scrutinee materialization); the destructuring match is the exercised path.
(case
  "a higher-order function applies a callback that matches its own sum argument"
  (doc
    "`apply-to` takes a callback `f` and a `C` value `c`, applying `(f c)`. The callback `(fn (p)
           (match p ((C.A n) n) ((C.B) 0)))` destructures its OWN parameter. Because `apply-to` inlines,
           the callback is applied to `c` through a nested β-reduction where `p` — the match scrutinee — is
           substituted; the `n` binder must re-resolve against the substituted scrutinee. `apply-to`
           applied to `(C.A 9)` yields 9. Was 'parameter reference has no local slot' when the substituted
           scrutinee's pattern binder kept its pre-substitution occurrence.")
  (input
    (do
      (type C (A Int64) B)
      (def (apply-to f (: c C)) (f c))
      (def (main) (apply-to (fn ((: p C)) (match p ((C.A n) n) ((C.B) 0))) (C.A 9)))
      (export main)))
  (output (: 9 Int64)))

(case
  "a HOF callback matching its sum argument reaches the nullary arm"
  (doc
    "The companion selecting the OTHER variant: the same callback applied (through the inlined HOF)
           to `(C.B)` takes the nullary arm → 0. Pins that the through-a-HOF nested reduction dispatches
           correctly across variants, not just the payload one.")
  (input
    (do
      (type C (A Int64) B)
      (def (apply-to f (: c C)) (f c))
      (def (main) (apply-to (fn ((: p C)) (match p ((C.A n) n) ((C.B) 0))) (C.B)))
      (export main)))
  (output (: 0 Int64)))

; A scalar Int64-literal `match` nested as an OPERAND of an op whose SIBLING operand computed an i32 heap
; handle. The classic idiom is an emitter: `emit(CBin op l r)` concatenates the recursive `emit(l)`/`emit(r)`
; byte buffers (each yields an i32 handle spilled to a scratch slot) with `b1(wop op)` — where `wop` dispatches
; the op-code via `(match op (43 124) …)`. The recursive-arg payload handles (i32) and the scalar match
; scrutinee (i64) have DISJOINT liveness — the handles are dead by the time the match runs — but the scalar
; match spilled its scrutinee into the SAME scratch slot the sibling arg's i32 handle had already recorded, so
; the one wasm local carried two widths → `type mismatch: expected i32, found i64`, an INVALID module. The fix
; spills the scalar scrutinee to a guaranteed-fresh high-water slot when its `base` already carries a
; conflicting width (mirroring the `MatchSum` scrutinee-spill discipline). Regression pin for the emit-db
; `wasm-op` idiomatic-`match` bug (invalid wasm at compiler-ml module scale; valid standalone). `emit` over
; `(CBin 43 (CNum 1) (CBin 45 (CNum 2) (CNum 3)))` yields 5 bytes (3 nums + 2 op-codes), so `Bytes.len` = 5.
(case
  "a scalar Int64 match nested as an op operand beside an i32-handle sibling emits valid wasm"
  (doc
    "A scalar `(match op (43 124) …)` (an i64 scrutinee) nested as an operand of `Bytes.concat`
           whose sibling operand computed an i32 heap handle (a recursive `emit` result). The scrutinee
           spill must NOT reuse the scratch slot the sibling's i32 handle typed — a wasm local carries one
           width, so reusing it re-typed the local (`expected i32, found i64`) and the module failed to
           validate. Regression pin for the emit-db `wasm-op` idiomatic-`match` invalid-wasm bug (valid
           standalone, invalid at module scale). `emit` produces 5 bytes → `Bytes.len` = 5.")
  (input
    (do
      (type Core (CNum Int64) (CBin Int64 Core Core))
      (def (b1 (: x Int64)) (Bytes.of #list((UInt8.wrap x))))
      (def (wop (: op Int64)) (match op (43 124) (45 125) (_ 0)))
      (def
        (emit (: c Core))
        (match
          c
          ((Core.CNum v) (b1 v))
          ((Core.CBin op l r) (Bytes.concat (Bytes.concat (emit l) (emit r)) (b1 (wop op))))))
      (def
        (main)
        (Bytes.len (emit (Core.CBin 43 (Core.CNum 1) (Core.CBin 45 (Core.CNum 2) (Core.CNum 3))))))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

; The multi-dispatch companion: the SAME recursive emitter, but the `CBin` arm dispatches through BOTH a
; Bool-returning `(match op (43 true) …)` (an i32 result) and, in the `if`'s else branch, a multi-arm
; Int64 `(match op (60 83) …)` (an i64 scrutinee, ≥3 arms so it spills a probe-chain slot rather than
; folding to a branchless `select`). This is the actual `emit-db` op-cluster idiom (`is-arith-op` +
; `cmp-op`/`wasm-op`): two scalar matches of DIFFERENT scrutinee/result widths, both nested beside the
; recursive `emit(l)`/`emit(r)` i32 heap handles in one function — so the emitted function declares mixed
; i32/i64 scratch locals and every scalar-match spill must land on a width-correct slot. Pins that the
; fresh-slot-on-width-conflict spill holds when MULTIPLE scalar matches coexist with the i32 handles in a
; single recursive frame, not just one. `emit (CBin 60 (CNum 1) (CBin 43 (CNum 2) (CNum 3)))` = 3 nums +
; 2 op-codes = 5 bytes.
(case
  "two scalar matches of different widths beside i32 handles in one recursive emit frame stay valid"
  (doc
    "The emit-db op-cluster idiom: a `CBin` arm dispatches through a Bool `(match op (43 true) …)`
           (i32) AND a multi-arm Int64 `(match op (60 83) …)` (i64, ≥3 arms → a spilled probe chain),
           both nested beside the recursive `emit(l)`/`emit(r)` i32 heap handles in one function. Each
           scalar-match scrutinee spill must land on a width-correct slot even with mixed i32/i64 scratch
           locals coexisting in one recursive frame. Pins the fresh-slot-on-width-conflict fix under
           MULTIPLE coexisting scalar matches, the multi-dispatch companion of the single-match case
           above. `emit` = 5 bytes → `Bytes.len` = 5.")
  (input
    (do
      (type Core (CNum Int64) (CBin Int64 Core Core))
      (def (b1 (: x Int64)) (Bytes.of #list((UInt8.wrap x))))
      (def (cmpop (: op Int64)) (match op (60 83) (61 81) (62 85) (_ 0)))
      (def (isarith (: op Int64)) (match op (43 true) (45 true) (_ false)))
      (def
        (emit (: c Core))
        (match
          c
          ((Core.CNum v) (b1 v))
          ((Core.CBin op l r)
            (Bytes.concat
              (Bytes.concat (emit l) (emit r))
              (if (isarith op) (b1 op) (b1 (cmpop op)))))))
      (def
        (main)
        (Bytes.len (emit (Core.CBin 60 (Core.CNum 1) (Core.CBin 43 (Core.CNum 2) (Core.CNum 3))))))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a self-tail-recursive fn with a MIXED Option-returning innermost match emits valid wasm"
  (doc
    "A self-tail-recursive `drive` whose INNERMOST match is MIXED — one arm RECURSES (the tail call
           `(drive s2)`) beside a sibling arm that RETURNS a value (`(Option.Some (tuple x s2))`) — and
           whose recursive arm returns an `Option`-typed value. This is the idiomatic filter-map worker:
           step the iterator, keep the first element passing the predicate, else recurse to skip. Under the
           tail-loop conversion (emit_tail) the `br` (loop-continue) from the recursive arm used to leave the
           enclosing `if (result i32)` block stack-UNBALANCED → invalid wasm (`func N failed to validate,
           values remaining on stack at end of block`). Non-const (plain `step`/recursion), so distinct from
           the const-closure driver cases above. Pins that a mixed-match Option-returning recursive tail arm
           lowers to VALID wasm and computes: `drive [1,2,3,4]` keeps the first x>2 → `Some (3, [4])`, and
           `main` reads its head → 3. Gate coverage for the emit_tail/tail-loop conversion so a future
           select.rs/lower change cannot silently re-break it.")
  (input
    (do
      (def
        (step (: s (List Int64)))
        (match s (#list() (Option.None)) (#list(h (.. t)) (Option.Some #tuple(h t)))))
      (def
        (drive (: s (List Int64)))
        (match
          (step s)
          ((Option.None) (Option.None))
          ((Option.Some p)
            (match p (#tuple(x s2) (if (> x 2) (Option.Some #tuple(x s2)) (drive s2)))))))
      (def
        (main)
        (match
          (drive #list(1 2 3 4))
          ((Option.None) -1)
          ((Option.Some p) (match p (#tuple(x s2) x)))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

; --- Tail recursion compiles to a constant-stack loop -------------------------------------------
; core-semantics.md: a SELF tail-call updates the parameter locals and `br`s back to the function's own
; `loop` — no call frame grows, so a tail-recursive count runs in O(1) stack. A frame-per-iteration
; `call` would trap far below a million. These run a MILLION iterations to completion: reaching the
; result (rather than a stack-overflow trap) is the observable proof the tail-call became a loop.
(case
  "a self-tail-recursive accumulator runs a million iterations in constant stack"
  (doc
    "`(f n acc)` decrements `n` and increments `acc` in a SELF tail position; `(f n 0)` counts `n`
           up from 0. The self tail-call compiles to a `loop` (args update the param locals, `br` back —
           no frame), so `main 1000000` completes returning 1000000. A stack-growing recursive `call`
           would trap long before a million frames.")
  (input
    (do
      (def (f (: n Int64) (: acc Int64)) (if (= n 0) acc (f (- n 1) (+ acc 1))))
      (def (main (: n Int64)) (f n 0))
      (export main)))
  (call main (: 1000000 Int64))
  (output (: 1000000 Int64)))

(case
  "a self-tail-recursive SUM consumer loops (tail call in a MatchSum arm) and computes the fold"
  (doc
    "The sum-type companion of the scalar tail loop above: `count` over `(type Nat (Zero) (Succ Nat))` self-tail-calls from INSIDE its `((Succ m) (count m (+ acc 1)))` match arm — the tail call sits in a sum decision-tree leaf, so the loop transform must thread tail position through the `MatchSum` (not only a bare / `if` tail), and the fold must compute correctly. `build` makes a depth-1000 Nat and `count` folds it → 1000. (That this compiles to a `loop` rather than a stack-growing `call` — the constant-stack STRUCTURE — is unit-pinned at the Lir level in rcdzc; here we pin the value parity of the MatchSum-arm tail fold.) The walked `Succ` spine reclaims per iteration, leaving one live cell.")
  (input
    (do
      (type Nat (Zero) (Succ Nat))
      (def (build (: i Int64) (: acc Nat)) (if (< i 1) acc (build (- i 1) (Succ acc))))
      (def (count (: n Nat) (: acc Int64)) (match n ((Zero) acc) ((Succ m) (count m (+ acc 1)))))
      (def (main) (count (build 1000 (Zero)) 0))
      (export main)))
  (call main)
  (output (: 1000 Int64))
  (live-objects 0))

(case
  "a same-signature mutual tail cycle shares one constant-stack loop"
  (doc
    "`even`/`odd` are a SAME-SIGNATURE mutual tail-recursive pair: each cross-call is in tail
           position, so the group compiles to ONE shared `loop` with a `which` dispatch — a cross-call
           sets `which` and `br`s back (no frame). `main 1000000` = even parity = 1; `main 999999` = 0.
           A million cross-calls complete in O(1) stack.")
  (input
    (do
      (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))
      (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1))))
      (def (main (: n Int64)) (even n))
      (export main)))
  (call main (: 1000000 Int64))
  (output (: 1 Int64))
  (call main (: 999999 Int64))
  (output (: 0 Int64)))

(case
  "a three-member mutual tail cycle shares one loop"
  (doc
    "A 3-cycle mutual tail group `g0 -> g1 -> g2 -> g0`, same signature — all three compile into
           shared loops over the SAME member set {g0,g1,g2}, dispatched by a `which`. `(g0 n)` counts
           down through the cycle to a base at k=0 and returns 0 regardless of which member the last hop
           lands on. `main 1000002` = 0 (a million hops in constant stack); `main 0` = 0.")
  (input
    (do
      (def (g0 (: k Int64)) (if (< k 1) k (g1 (- k 1))))
      (def (g1 (: k Int64)) (if (< k 1) k (g2 (- k 1))))
      (def (g2 (: k Int64)) (if (< k 1) k (g0 (- k 1))))
      (def (main (: n Int64)) (g0 n))
      (export main)))
  (call main (: 1000002 Int64))
  (output (: 0 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a match-based self-tail-recursion loops in constant stack"
  (doc
    "The base case via a MATCH (idiomatic): `(match n (0 acc) (_ (f (- n 1) (+ acc 1))))`. The
           self tail-call sits in a match ARM; the arm's call becomes a loop `br` (not a frame). `main
           1000000` = 1000000, completing in O(1) stack.")
  (input
    (do
      (def (f (: n Int64) (: acc Int64)) (match n (0 acc) (_ (f (- n 1) (+ acc 1)))))
      (def (main (: n Int64)) (f n 0))
      (export main)))
  (call main (: 1000000 Int64))
  (output (: 1000000 Int64)))

(case
  "a match-based tail recursion with an earlier literal arm loops correctly"
  (doc
    "A match with an EARLIER literal arm before the recursive one: `(match n (0 acc) (1 (+ acc
           100)) (_ (g (- n 1) (+ acc 1))))`. The recursive arm is nested one probe deeper, so its call
           `br`s to the depth+1 loop target. `g 500000 0` counts down and, at n==1, adds 100 instead of
           1: 499999 increments (n=500000..2) + 100 = 500099.")
  (input
    (do
      (def
        (g (: n Int64) (: acc Int64))
        (match n (0 acc) (1 (+ acc 100)) (_ (g (- n 1) (+ acc 1)))))
      (def (main (: n Int64)) (g n 0))
      (export main)))
  (call main (: 500000 Int64))
  (output (: 500099 Int64)))

; --- Accumulator introduction: a linear non-tail recursion becomes a constant-stack loop -----------
; A LINEAR non-tail recursion — one self-call whose result feeds a single enclosing ASSOCIATIVE op with
; an identity base — is rewritten to a tail-recursive accumulator (accum::introduce) and then to a loop,
; so a million-deep sum/product that would overflow the stack as pending `call` frames runs in O(1)
; stack with the value unchanged. Reaching the result at a million deep (rather than a stack-overflow
; trap) is the observable proof the transform fired.
(case
  "a linear non-tail sum is accumulator-transformed into a constant-stack loop"
  (doc
    "`(sm n) = (if (= n 0) 0 (+ n (sm (- n 1))))` — the self-call is an OPERAND of `+`, so it is
           NOT a tail call and would compile to a stack-growing `call`. Because `+` is associative with
           identity 0, accumulator introduction rewrites it to a tail accumulator, which the loop
           transform compiles to a `loop`. sm(5)=15, sm(100)=5050, and sm(1000000)=500000500000 runs in
           O(1) stack — a frame-per-level recursion would overflow far below a million.")
  (input (do (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1))))) (export sm)))
  (call sm (: 5 Int64))
  (output (: 15 Int64))
  (call sm (: 100 Int64))
  (output (: 5050 Int64))
  (call sm (: 1000000 Int64))
  (output (: 500000500000 Int64)))

(case
  "the accumulator transform preserves a product (factorial) shape"
  (doc
    "`(fac n) = (if (= n 0) 1 (* n (fac (- n 1))))` — the same non-tail linear shape over `*`
           (associative, identity 1). The transform must preserve the exact product: fac(5)=120,
           fac(10)=3628800. (Large-n is exercised by the sum cases; a large factorial overflows i64 by
           design — see the overflow-traps pin.)")
  (input (do (def (fac (: n Int64)) (if (= n 0) 1 (* n (fac (- n 1))))) (export fac)))
  (call fac (: 5 Int64))
  (output (: 120 Int64))
  (call fac (: 10 Int64))
  (output (: 3628800 Int64)))

(case
  "the accumulator transform applies with the self-call in either operand"
  (doc
    "`(sm2 n) = (if (= n 0) 0 (+ (sm2 (- n 1)) n))` — the self-call is the FIRST operand of `+`
           (vs `(+ n (sm ...))` above). Accumulator introduction still applies (commutative-associative
           `+`), so sm2(1000000)=500000500000 also runs in O(1) stack.")
  (input (do (def (sm2 (: n Int64)) (if (= n 0) 0 (+ (sm2 (- n 1)) n))) (export sm2)))
  (call sm2 (: 1000000 Int64))
  (output (: 500000500000 Int64)))

(case
  "the accumulator transform declines when the base is not the operator's identity"
  (doc
    "`(sm n) = (if (= n 0) 100 (+ n (sm (- n 1))))` — the linear non-tail `+` shape, but the base
           is 100, NOT `+`'s identity 0. Reassociating into an accumulator seeded at 100 would fold the
           base in once per level and change the result, so the transform must DECLINE (stay a plain
           recursion) and still compute the right value: sm(3) = 3+2+1+100 = 106. A misfiring transform
           that reassociated regardless would yield a different number — this pins it does not.")
  (input (do (def (sm (: n Int64)) (if (= n 0) 100 (+ n (sm (- n 1))))) (export sm)))
  (call sm (: 3 Int64))
  (output (: 106 Int64)))

(case
  "the accumulator transform declines a non-associative combine with the self-call first"
  (doc
    "`(f n) = (if (= n 0) 0 (- (f (- n 1)) 1))` — the self-call is the FIRST operand of a
           NON-associative `-`, so reassociating into an accumulator would change the result; the
           transform must decline and preserve the exact left-nested value: f(0)=0, f(1)=-1, f(2)=-2,
           f(3)=-3. Companion of the right-nested non-associative case; here the recursive call sits in
           the operator's first operand.")
  (input (do (def (f (: n Int64)) (if (= n 0) 0 (- (f (- n 1)) 1))) (export f)))
  (call f (: 3 Int64))
  (output (: -3 Int64)))

; --- Loop-invariant code motion preserves the computed value --------------------------------------
; A loop-invariant subexpression (one whose operands do not change across iterations) is hoisted out of
; the loop and computed once. These pin the VALUE PARITY of that hoist end-to-end: whatever the compiler
; moves, the program still returns the same result it would recomputing each iteration. (The structural
; placement — the invariant lands BEFORE the loop, not inside — is a white-box compiler check.)
(case
  "a loop-invariant bitwise op hoisted out of a tail loop preserves the value"
  (doc
    "`(& k 255)` over the pass-through param `k` is loop-invariant (k threads unchanged), so it is
           hoisted out of the tail loop and computed once. `(go n k 0)` adds `(& k 255)` each of n
           iterations; `f(999) = 10 * (999 & 255 = 231) = 2310`. Pins that hoisting the invariant
           bitwise-and does not change the accumulated result.")
  (input
    (do
      (def
        (go (: n Int64) (: k Int64) (: acc Int64))
        (if (= n 0) acc (go (- n 1) k (+ acc (& k 255)))))
      (def (f (: k Int64)) (go 10 k 0))
      (export f)))
  (call f (: 999 Int64))
  (output (: 2310 Int64)))

(case
  "a loop-invariant multiply in the loop condition preserves the value"
  (doc
    "`(* n 2)` is loop-invariant in the exit condition `(< i (* n 2))`, so it is hoisted before the
           loop (computed once). `(go 0 x 0)` sums i over [0, x*2): f(3) sums 0+1+2+3+4+5 = 15; f(0) runs
           zero iterations = 0. Pins the hoisted bound is the same value the loop would recompute.")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: acc Int64))
        (if (< i (* n 2)) (go (+ i 1) n (+ acc i)) acc))
      (def (f (: x Int64)) (go 0 x 0))
      (export f)))
  (call f (: 3 Int64))
  (output (: 15 Int64))
  (call f (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a loop-invariant used in both the condition and the body is hoisted once with the same value"
  (doc
    "The same invariant `(* n 2)` appears in the exit condition `(< i (* n 2))` AND the body `(+
           acc (* n 2))`; the compiler value-numbers the hoist so both read one pre-loop slot. `(go 0 x
           0)` runs x*2 iterations, adding (x*2) each: f(3) = 6 iterations * (3*2) = 36. Pins that the
           single hoisted value feeds both uses correctly.")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: acc Int64))
        (if (< i (* n 2)) (go (+ i 1) n (+ acc (* n 2))) acc))
      (def (f (: x Int64)) (go 0 x 0))
      (export f)))
  (call f (: 3 Int64))
  (output (: 36 Int64)))

(case
  "a HOF callback matching a tuple argument computes through the nested reduction"
  (doc
    "The same shape with a TUPLE-destructuring callback — `(fn (p) (match p ((tuple a b) (+ a b))))`
           — passed to a HOF. The tuple-pattern binders `a`/`b` read the substituted scrutinee just as a
           sum-variant binder does, so this pins the fix is over any compound-match scrutinee, not sums
           alone. `(tuple 3 4)` → 3 + 4 = 7.")
  (input
    (do
      (def (apply-to f (: t (Tuple Int64 Int64))) (f t))
      (def
        (main)
        (apply-to (fn ((: p (Tuple Int64 Int64))) (match p (#tuple(a b) (+ a b)))) #tuple(3 4)))
      (export main)))
  (output (: 7 Int64)))

; --- A RECURSIVE higher-order function with an UNANNOTATED function-typed parameter --------------
; core-semantics.md §A Function Is A First-Class Value: a recursive traversal takes a CALLBACK and applies
; it per element — `map`/`fold` over a recursive sum. The callback parameter `f` need not be annotated:
; its type is inferred from its USE as a call head (`(f h)` ⇒ `f : (-> typeof(h) result)`), the function
; analogue of inferring a data parameter's type from a pattern match. The recursive-parameter solve gives
; each fn-typed parameter its arrow shape before collecting constraints, so `(+ (f h) …)` flows the result
; type back to the arrow. Without it a recursive HOF's callback stayed unconstrained → the recursive-def
; guard declined "annotate its parameters"; annotating was the only recourse. These pin that the
; annotation is now optional — the recursion-over-a-sum-with-a-callback idiom compiles bare.
(case
  "a recursive fold over a sum list infers its unannotated callback parameter"
  (doc
    "`sum-f` recurses over `(type L Nil (Cons Int64 L))`, applying an UNANNOTATED callback `f` to
           each head and summing: `(+ (f h) (sum-f f t))`. `f`'s type is inferred `(-> Int64 Int64)` from
           `(f h)` (h : Int64) and the `+` that consumes its result — no annotation on `f`. Applied with
           `(fn (x) (+ x 1))` over `[1, 2]` → (1+1) + (2+1) = 5. Was 'a recursive function with an
           unannotated parameter is not yet inferred' before fn-typed recursive params were solved.")
  (input
    (do
      (type L Nil (Cons Int64 L))
      (def (sum-f f (: l L)) (match l ((L.Nil) 0) ((L.Cons h t) (+ (f h) (sum-f f t)))))
      (def (main) (sum-f (fn ((: x Int64)) (+ x 1)) (L.Cons 1 (L.Cons 2 L.Nil))))
      (export main)))
  (output (: 5 Int64))
  (live-objects known-leak))

(case
  "a recursive map rebuilding a sum list infers its unannotated callback"
  (doc
    "The map companion: `map-f` REBUILDS the list, applying an unannotated `f` to each element —
           `(L.Cons (f h) (map-f f t))`. `f` infers `(-> Int64 Int64)` from `(f h)` in a `Cons`-payload
           position. `(fn (x) (* x 2))` over `[3, 4]` yields `[6, 8]`; the caller reads the head → 6. Pins
           the inference works when the callback's result feeds a CONSTRUCTOR payload, not only an operator.")
  (input
    (do
      (type L Nil (Cons Int64 L))
      (def (map-f f (: l L)) (match l ((L.Nil) L.Nil) ((L.Cons h t) (L.Cons (f h) (map-f f t)))))
      (def
        (main)
        (match
          (map-f (fn ((: x Int64)) (* x 2)) (L.Cons 3 (L.Cons 4 L.Nil)))
          ((L.Cons h t) h)
          ((L.Nil) 0)))
      (export main)))
  (output (: 6 Int64))
  (live-objects known-leak))

(case
  "a mutually-recursive decoder infers its params from the call site and emits valid wasm"
  (doc
    "Composes two fixes: (1) TRANSITIVE call-site inference — `dn`'s param `b` (`(List Int64)`) is
           decided only via `main → top → dn` / `dac → dn`, threaded through the pass-through params by
           seeding `dac`/`top` from THEIR call sites; (2) an EMIT scratch-floor — a `SumExpect`
           (Option.expect) handle slot reserved above the running high-water and each `tuple` element's
           scratch advanced past the prior element's, so an i32 heap handle never re-types an i64 slot a
           sibling element uses (`(tuple (AInt (expect …)) (+ i 1))` clashed → 'expected i64, found i32').
           The decoder normalizes `(list 42 7)` → `(AInt 42)`, matched to 42.")
  (input
    (do
      (type Ast (AInt Int64) ALeaf (AList (List Ast)))
      (def
        (dn b i)
        (if
          (= i 0)
          #tuple((AInt (Option.expect (List.at b 0) "in range")) (+ i 1))
          #tuple((AList (dac b i (- i 1) #list())) (+ i 1))))
      (def
        (dac b i n acc)
        (if
          (< n 1)
          acc
          (match (dn b i) (#tuple(child nx) (dac b nx (- n 1) (List.push acc child))))))
      (def (top b) (match (dn b 0) (#tuple(ast pos) ast)))
      (def (main) (match (top #list(42 7)) ((AInt n) n) (_ -1)))
      (export main)))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a recursive fold with an unannotated two-argument callback parameter"
  (doc
    "The callback takes TWO arguments — `(fn (a b) (+ a b))` — and `fold` threads an accumulator:
           `(fold f (f acc h) t)`. `f` infers `(-> Int64 (-> Int64 Int64))` from the two-argument
           application `(f acc h)`, so a multi-argument callback param is inferred at its full arity, not
           just unary. `1 + 2 + 3` = 6. Pins the arrow-shaping is over the application's argument COUNT.")
  (input
    (do
      (type L Nil (Cons Int64 L))
      (def
        (fold f (: acc Int64) (: l L))
        (match l ((L.Nil) acc) ((L.Cons h t) (fold f (f acc h) t))))
      (def
        (main)
        (fold (fn ((: a Int64) (: b Int64)) (+ a b)) 0 (L.Cons 1 (L.Cons 2 (L.Cons 3 L.Nil)))))
      (export main)))
  (output (: 6 Int64))
  (live-objects known-leak))

(case
  "a recursive HOF infers a callback whose RESULT is a sum matched in the body"
  (doc
    "The callback's RESULT type is inferred too, not only its parameter: `find` applies an
           unannotated `f` and MATCHES its result — `(match (f h) ((C.A n) …) ((C.B) …))`. The `C.A`/`C.B`
           arm patterns pin `f`'s result to the sum `C`, so `f : (-> Int64 C)` with no annotation. `find`
           returns the first element for which `f` yields `C.A`: over `[0, 5]` with `f x = (if (> x 1) (C.A
           x) (C.B))`, element 5 gives `(C.A 5)` → 5. Pins that a fn-param's result is solved from a match
           on its application, the result-side companion of inferring the parameter from `(f h)`.")
  (input
    (do
      (type L Nil (Cons Int64 L))
      (type C (A Int64) B)
      (def
        (find f (: l L))
        (match l ((L.Nil) (C.B)) ((L.Cons h t) (match (f h) ((C.A n) (C.A n)) ((C.B) (find f t))))))
      (def
        (main)
        (match
          (find (fn ((: x Int64)) (if (> x 1) (C.A x) (C.B))) (L.Cons 0 (L.Cons 5 L.Nil)))
          ((C.A n) n)
          ((C.B) 0)))
      (export main)))
  (output (: 5 Int64))
  (live-objects known-leak))

(case
  "a branching recursive tree fold infers its unannotated callback across both arms"
  (doc
    "A tree `(type T (Leaf Int64) (Node (Tuple T T)))` folded by an unannotated callback `f` with
           BRANCHING recursion — the `Node` arm makes TWO self-calls `(+ (fold-t f l) (fold-t f r))`. The
           `Leaf` arm returns `(f n)` DIRECTLY, so `f`'s result type is fixed only by the arms agreeing:
           the `Node` arm is Int64, so the `Leaf` arm — hence `f`'s result — is Int64. Pins that the
           arms-agree constraint reaches a fn-param's result var when an arm body is a bare callback
           application, the branching-recursion companion of the single-recursion fold. `(1 + 2) · 10`
           applied per leaf → 10 + 20 = 30.")
  (input
    (do
      (type T (Leaf Int64) (Node (Tuple T T)))
      (def
        (fold-t f (: t T))
        (match t ((T.Leaf n) (f n)) ((T.Node #tuple(l r)) (+ (fold-t f l) (fold-t f r)))))
      (def (main) (fold-t (fn ((: x Int64)) (* x 10)) (T.Node #tuple((T.Leaf 1) (T.Leaf 2)))))
      (export main)))
  (output (: 30 Int64))
  (live-objects known-leak))

(case
  "a recursive fold infers a callback applied to the RECURSIVE-CALL RESULT"
  (doc
    "The callback is applied not to a payload but to the RESULT OF THE RECURSIVE CALL — `(f (foldn f
           z m))` over Peano `(type N Z (S N))`. `f`'s parameter is that recursive result and its result is
           the `S` arm's value; the arms agree (the `Z` arm returns the accumulator `z : Int64`), so `f`'s
           result — hence its whole arrow `(-> Int64 Int64)` — is inferred with no annotation on `f`. This
           is the general recursive-fold shape (fold right, applying the callback to the sub-fold), the
           companion of applying the callback to a payload element. `f = (+ x 1)` applied twice to z = 0
           → 2. (The accumulator `z` is annotated: a pure pass-through parameter has no INTERNAL constraint
           to infer from, so it is annotated, exactly as a non-callback accumulator is.)")
  (input
    (do
      (type N Z (S N))
      (def (foldn f (: z Int64) (: n N)) (match n ((N.Z) z) ((N.S m) (f (foldn f z m)))))
      (def (main) (foldn (fn ((: x Int64)) (+ x 1)) 0 (N.S (N.S (N.Z)))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a closure capturing two enclosing bindings folds through nested arithmetic"
  (doc
    "`(fn (x) (+ (* x a) b))` captures BOTH `a` and `b` from enclosing lets; applied to 5 with
           a = 2, b = 3 → (5·2)+3 = 13. Pins that MULTIPLE distinct captures from different enclosing
           `let`s are each preserved and folded through a nested arithmetic body.")
  (input (let ((a 2) (b 3)) ((fn ((: x Int64)) (+ (* x a) b)) 5)))
  (output (: 13 Int64)))

; A closure that CAPTURES ANOTHER CLOSURE and applies it — a higher-order capture. `twice` closes over
; `inc` (itself a closure) and applies it twice; `(twice 5)` = inc(inc(5)) = 7. core-semantics.md §A
; Function Is A First-Class Value: a function value can be captured like any other. Both closures fold —
; the captured `inc` inlines at each application inside `twice`'s body.
(case
  "a closure captures another closure and applies it"
  (doc
    "`inc = (fn (x) (+ x 1))`; `twice = (fn (y) (inc (inc y)))` captures `inc` and applies it twice;
           `(twice 5)` = inc(inc(5)) = 7. A closure captured by another closure is applied correctly —
           the captured function value folds at each use.")
  (input
    (let
      ((inc (fn ((: x Int64)) (+ x 1))))
      (let ((twice (fn ((: y Int64)) (inc (inc y))))) (twice 5))))
  (output (: 7 Int64)))

(case
  "a closure captures another closure and applies it at RUNTIME"
  (doc
    "The same higher-order capture but with a RUNTIME argument, so nothing folds: `(def (main (: n
           Int64)) …)` binds `inc = (fn (x) (+ x 1))` and `twice = (fn (y) (inc (inc y)))` (which CAPTURES
           `inc`), then `(twice n)`. With `n = 5` → inc(inc(5)) = 7, computed at run time — `twice`'s cell
           holds the captured `inc` handle, dispatched via `call_indirect` at each use. Complements the folded
           case above: the captured closure value survives on the heap and is applied without inlining.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((inc (fn ((: x Int64)) (+ x 1))))
          (let ((twice (fn ((: y Int64)) (inc (inc y))))) (twice n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))

(case
  "a factory RETURNS a closure that captures a let-bound inner closure"
  (doc
    "`(def (mk (: k Int64)) (let ((g (fn (y) (+ y k)))) (fn (x) (g x))))` — `mk` binds an inner closure
           `g` (capturing `k`), then RETURNS an outer closure that captures `g`. `((mk 10) n)` with n = 5 →
           the returned closure applies `g` to 5 = 5 + 10 = 15, at runtime. Pins a returned closure capturing
           a LET-bound closure (a two-level capture: the outer holds `g`, `g` holds `k`).")
  (input
    (do
      (def (mk (: k Int64)) (let ((g (fn ((: y Int64)) (+ y k)))) (fn ((: x Int64)) (g x))))
      (def (main (: n Int64)) ((mk 10) n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

; A returned lambda capturing the def's SCALAR parameter (the C-HOST-2 make-forwarding shape at the def
; level): the scalar argument substitutes cleanly into the returned lambda's cell.
(case
  "a factory RETURNS a closure capturing the def's SCALAR parameter"
  (doc
    "`(def (mk (: k Int64)) (fn (x) (+ x k)))` — the returned closure captures the def's SCALAR param
           `k`. Applied `((mk 10) n)` with n = 5 → 5 + 10 = 15. The scalar argument `10` substitutes cleanly
           into the returned lambda's cell.")
  (input
    (do
      (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (main (: n Int64)) ((mk 10) n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

; A nested lambda capturing a closure-typed DEF PARAMETER now works too, including when the def is applied to
; an INLINE lambda argument. The fix (`eval::apply_lambda`): a lambda ARGUMENT is pinned by its FREE variables
; only (`pin_free_vars`, excluding the arg lambda's own params) rather than by a blunt whole-subtree
; `resolve_subtree` — so its own-param body references stay unpinned and re-substitute when the arg lambda is
; later applied inside the returned lambda that lifts (previously they dangled as slot-less `Core::Param`, the
; "parameter reference has no local slot" decline). A def-ref or a let-bound lambda already worked; this
; brings the INLINE lambda argument to parity.
(case
  "a nested lambda captures+applies a closure-typed def PARAMETER (inline lambda argument)"
  (doc
    "`(def (mk (: g (-> Int64 Int64))) (fn (x) (g x)))` returns a closure that captures the def's
           CLOSURE-typed parameter `g`, applied to an INLINE lambda `((mk (fn (y) (+ y 1))) n)`. The returned
           lambda captures `g` (= the arg lambda) and dispatches it; with n = 5 → `(fn y -> y+1)` applied to 5
           = 6. The arg lambda's own param `y` re-substitutes correctly inside the lifted returned body (the
           free-vars-only pinning fix). A higher-order (closure-arg) FACTORY at runtime.")
  (input
    (do
      (def (mk (: g (-> Int64 Int64))) (fn ((: x Int64)) (g x)))
      (def (main (: n Int64)) ((mk (fn (y) (+ y 1))) n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

; The same factory with the closure argument supplied three OTHER ways — all equivalent now: a TOP-LEVEL def
; (a global ref), and (below) a LET-bound lambda. These already worked before the inline-arg fix; kept as
; coverage that the closure-arg factory is uniform across argument spellings.
(case
  "a returned lambda captures+applies a closure param bound to a TOP-LEVEL def"
  (doc
    "The same `(def (mk (: g (-> Int64 Int64))) (fn (x) (g x)))` returning a closure that captures its
           closure param `g` — but here `g`'s argument is a TOP-LEVEL def `inc`, not an inline lambda.
           `((mk inc) n)` with n = 5 → the returned closure applies `inc` to 5 = 6. Works: a def reference is a
           global (re-resolves by name, no pinned own-param), so it captures + dispatches cleanly — isolating
           the decline above to the INLINE-lambda argument specifically.")
  (input
    (do
      (def (inc (: y Int64)) (+ y 1))
      (def (mk (: g (-> Int64 Int64))) (fn ((: x Int64)) (g x)))
      (def (main (: n Int64)) ((mk inc) n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

; The third argument spelling: a LET-bound lambda. Equivalent to the inline and top-level-def forms above —
; all three now capture + dispatch the closure argument through the returned lambda cleanly.
(case
  "a let-bound lambda passed to a returned-closure factory"
  (doc
    "The SAME `(def (mk (: g (-> Int64 Int64))) (fn (x) (g x)))` returned-closure factory, with the
           lambda argument LET-BOUND first: `(let ((f (fn (y) (+ y 1)))) ((mk f) n))`. `main(5)` → the returned
           closure applies `f` to 5 = 6. Equivalent to the inline and def-ref argument spellings above.")
  (input
    (do
      (def (mk (: g (-> Int64 Int64))) (fn ((: x Int64)) (g x)))
      (def (main (: n Int64)) (let ((f (fn ((: y Int64)) (+ y 1)))) ((mk f) n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a closure argument is another closure's result"
  (doc
    "The argument to one closure is the result of applying another: `((fn (x) (+ x k)) ((fn (y)
           (* y 2)) 3))` with k = 10 → (fn x)(6) = 16. Composing two closure applications — the inner
           `(* 3 2) = 6` feeds the outer `(+ 6 10) = 16` — both fold.")
  (input (let ((k 10)) ((fn ((: x Int64)) (+ x k)) ((fn ((: y Int64)) (* y 2)) 3))))
  (output (: 16 Int64)))

(case
  "a function is passed as an argument (higher-order)"
  (doc
    "Witnesses core-semantics.md §A Function Is A First-Class Value: apply-twice takes a function
           f and a value v and applies f to the result of applying f to v.")
  (input (let ((apply-twice (fn (f v) (f (f v))))) (apply-twice (fn (x) (+ x 3)) 1)))
  (output (: 7 Int64)))

; The higher-order function above is LET-BOUND (a lambda); a NAMED-def higher-order function must be
; able to receive a function argument just the same — core-semantics.md §A Function Is A First-Class
; Value places no restriction on how the receiving function is bound. The seed resolves a lambda
; argument to a let-bound HOF (compile-time beta reduction, above) but NOT to a NAMED-def HOF: `(def
; (ap g v) (g v))` applied to a lambda declines "bare lambda in scalar position". The same inlining
; that handles the let-bound HOF must apply when the HOF is a top-level def.
(case
  "a named higher-order function receives a lambda argument"
  (doc
    "`ap` is a named-def higher-order function taking a function `g` and a value `v`, applying g
           to v; `(ap (fn (x) (* x 2)) 7)` = 14. A named HOF must accept a function argument exactly as
           the let-bound `apply-twice` above does — the difference is only whether the HOF is named or
           let-bound. The seed declines the named case (\"bare lambda in scalar position\"): it inlines
           a lambda argument into a let-bound HOF but not into a named-def HOF.")
  (input (do (def (ap g v) (g v)) (def (main) (ap (fn (x) (* x 2)) 7)) (export main)))
  (output (: 14 Int64)))

; An OPERATOR is a first-class value too — a HOF can receive `+`/`<`/… and apply it. Wrapped in an
; explicit lambda `(fn (a b) (+ a b))` it applies inside an ordinary closure grounded by the HOF's
; annotated function parameter; a BARE operator in the same position is equivalent and also works. When the
; HOF INLINES, the β-reduction substitutes the operator for the annotated parameter, wrapping it as
; `(: + (-> …))`, and the meta-apply dispatch sees THROUGH that annotation so the reduced `(+ x y)` folds at
; compile time — no runtime closure.
(case
  "an operator applied inside a lambda passed to an annotated named HOF"
  (doc
    "A named higher-order function whose function parameter is ANNOTATED `(-> Int64 Int64 Int64)`
           receives `(fn (a b) (+ a b))` — a closure that APPLIES the `+` operator — and applies it:
           `(apply2 (fn (a b) (+ a b)) 3 4)` = 7. The annotation grounds `g` as a function (unlike the
           bare-param named HOF above), so the operator-applying closure lowers and runs.")
  (input
    (do
      (def (apply2 (: g (-> Int64 Int64 Int64)) (: x Int64) (: y Int64)) (g x y))
      (def (main) (apply2 (fn (a b) (+ a b)) 3 4))
      (export main)))
  (output (: 7 Int64)))

(case
  "a bare primitive operator passed as a first-class function value folds via the annotation see-through"
  (doc
    "Passing a bare operator `+` where a function VALUE is expected — `(apply2 + 3 4)` — is equivalent
           to passing `(fn (a b) (+ a b))` (the case above), i.e. 7. The non-recursive HOF INLINES, and the
           β-reduction wraps the operator for the annotated parameter as `(: + (-> Int64 Int64 Int64))`; the
           meta-apply dispatch peels that annotation so the reduced `(+ 3 4)` dispatches as the operator and
           folds to 7. (Previously declined 'value is not applyable' — the annotated operator head had no
           reachable `(meta apply)`.)")
  (input
    (do
      (def (apply2 (: g (-> Int64 Int64 Int64)) (: x Int64) (: y Int64)) (g x y))
      (def (main) (apply2 + 3 4))
      (export main)))
  (output (: 7 Int64)))

; A bare operator passed to a HOF that emits a REAL call (a recursive HOF, not inlined) is ETA-EXPANDED to
; the equivalent lambda `(fn (a b) (+ a b))` — grounded by the HOF's annotated function-parameter type —
; and passed as the runtime closure. (A non-recursive HOF that fully INLINES a bare operator argument is a
; separate reduce-path case, still pending — the `(apply2 + 3 4)` todo above.)
(case
  "a recursive HOF applies a bare operator passed as its function argument"
  (doc
    "`ap2` is a RECURSIVE named higher-order function (so it emits a real call rather than inlining):
           its function parameter `g` is annotated `(-> Int64 Int64 Int64)`, and it is passed the bare
           operator `+`. `(ap2 + 3 4 n)` with a runtime `n` recurses `n` times then applies `g` — the bare
           `+` is eta-expanded to `(fn (a b) (+ a b))` and passed as the runtime closure, so `n=0` yields
           `(+ 3 4)` = 7. A bare operator has no runtime closure form of its own; without the eta this
           declined 'value is not applyable' when `ap2` applied `g`.")
  (input
    (do
      (def
        (ap2 (: g (-> Int64 Int64 Int64)) (: x Int64) (: y Int64) (: n Int64))
        (if (< n 1) (g x y) (ap2 g x y (- n 1))))
      (def (main (: n Int64)) (ap2 + 3 4 n))
      (export main)))
  (call main 0)
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "an inline HOF applies a bare operator passed as its function argument"
  (doc
    "A second operator witness: `ap` is a non-recursive named HOF, so `(ap * 6 7)` INLINES,
           substituting `*` for the annotated parameter `g` — wrapped as `(: * (-> Int64 Int64 Int64))` —
           and the meta-apply see-through lets the reduced `(* 6 7)` fold to 42.")
  (input
    (do
      (def (ap (: g (-> Int64 Int64 Int64)) (: x Int64) (: y Int64)) (g x y))
      (def (main) (ap * 6 7))
      (export main)))
  (output (: 42 Int64)))

; A PARTIALLY-applied operator `(+ 10)` — a curried operator (ch01e: operators curry) — passed to a named
; HOF whose function parameter is the RESULT arrow `(-> Int64 Int64)`. The partial IS a first-class function
; `\b. 10 + b`; passing it to `apply1` and applying it to `5` yields `10 + 5 = 15`. The HOF INLINES, and the
; β-reduction substitutes the partial for the annotated parameter RAW (not wrapped `(: (+ 10) …)`, which
; would block the fold) — the partial-application twin of the bare-operator see-through above.
(case
  "a partially-applied operator passed to an annotated named HOF applies its remaining operand"
  (doc
    "`(+ 10)` curries to `\\b. 10 + b`; `(apply1 (+ 10) 5)` hands that partial to a HOF taking `(->
           Int64 Int64)` and applies it: 10 + 5 = 15. The inlined β-reduction substitutes the partial RAW
           into the annotated parameter, so the reduced `((+ 10) 5)` folds like the bare `(+ 10 5)`.")
  (input
    (do
      (def (apply1 (: f (-> Int64 Int64)) (: x Int64)) (f x))
      (def (main) (apply1 (+ 10) 5))
      (export main)))
  (output (: 15 Int64)))

; The operator-as-value support is not arithmetic-specific: a Bool-returning COMPARISON operator
; (`<`/`>`/`=`) passed to a HOF works the same, through both the inline fold and the emitted-call eta. The
; HOF's function parameter is annotated with the comparison arrow `(-> Int64 Int64 Bool)`, which grounds the
; operator; the result type is Bool, not the operand type.
(case
  "a Bool-returning comparison operator passed to an inline HOF"
  (doc
    "`(ap < 3 4)` passes the bare comparison operator `<` to a non-recursive HOF `ap` whose parameter
           is annotated `(-> Int64 Int64 Bool)`. The call INLINES, substituting `<` for the annotated param;
           the reduced `(< 3 4)` dispatches as the comparison and folds to `true`. Pins that operator-value
           support covers a Bool-returning comparison, not only same-type arithmetic (`+`/`*`).")
  (input
    (do
      (def (ap (: g (-> Int64 Int64 Bool)) (: x Int64) (: y Int64)) (g x y))
      (def (main) (ap < 3 4))
      (export main)))
  (output (: true Bool)))

(case
  "a comparison operator passed to a recursive HOF (emit-call)"
  (doc
    "The emitted-call twin: `ap2` is RECURSIVE, so it emits a real call and the bare `>` is
           eta-expanded to a runtime closure at the call. `(ap2 > 7 3 n)` with runtime `n` recurses then
           applies `g`; `n=0` yields `(> 7 3)` = `true`. Pins the comparison operator through the eta path.")
  (input
    (do
      (def
        (ap2 (: g (-> Int64 Int64 Bool)) (: x Int64) (: y Int64) (: n Int64))
        (if (< n 1) (g x y) (ap2 g x y (- n 1))))
      (def (main (: n Int64)) (ap2 > 7 3 n))
      (export main)))
  (call main 0)
  (output (: true Bool))
  (live-objects known-leak))

(case
  "a function is returned as a result"
  (doc
    "Witnesses core-semantics.md §A Function Is A First-Class Value: adder returns a closure over
           its parameter n; the returned function is then applied.")
  (input (let ((adder (fn (n) (fn (x) (+ x n))))) ((adder 10) 5)))
  (output (: 15 Int64)))

(case
  "a multi-parameter closure keeps its captured environment distinct from its arguments"
  (doc
    "A closure that BOTH captures multiple variables AND takes multiple parameters must keep the two
           sets of slots distinct — the captured environment (`a`, `b`) and the applied arguments (`x`, `y`)
           must not be confused by the closure calling convention. `(mk a b)` returns `(fn (x y) (+ (* a x)
           (* b y)))`; with distinguishable powers-of-ten weights any env/arg swap changes the result:
           `((mk 1 1000) 7 3)` = 1·7 + 1000·3 = 3007. A convention that read an argument where a capture
           belongs (or vice versa) would give a different number (7·1 + 3·1000, or 1·1 + 1000·1). Pins that
           a multi-param closure's environment cells and argument slots are separately addressed — captures
           first, then the full-arity arguments.")
  (input
    (do
      (def (mk (: a Int64) (: b Int64)) (fn (x y) (+ (* a x) (* b y))))
      (def (main) ((mk 1 1000) 7 3))
      (export main)))
  (output (: 3007 Int64)))

; A function SELECTED BY A RUNTIME CONDITION and then applied — `((if b f g) x)`. `core-semantics.md`
; §A Function Is A First-Class Value: a function is a value an `if` may return, so applying the `if`'s
; result must run whichever function the runtime condition chose. The condition here is a RUNTIME
; parameter (`b`), so the choice is not known at compile time — the application is pushed into each
; branch (a case-of-case / commuting conversion `((if b f g) x)` → `(if b (f x) (g x))`), where each
; branch's function applies. Both branches must yield the same type (Int64), which is the application's
; type. A generation that cannot select a runtime function value declines rather than running.
(case
  "a function chosen by a runtime condition is applied (true branch)"
  (doc
    "`choose` returns one of two functions by its Bool argument; `((choose b) 5)` applies the
           chosen one. With b=true the chosen function is `(fn (x) (+ x 1))`, so the result is 6. The
           condition is a runtime parameter, so the function is selected at run time, not folded.")
  (input
    (do
      (def (choose (: b Bool)) (if b (fn (x) (+ x 1)) (fn (x) (+ x 10))))
      (def (main (: b Bool)) ((choose b) 5))
      (export main)))
  (call main (: true Bool))
  (output (: 6 Int64)))

(case
  "a function chosen by a runtime condition is applied (false branch)"
  (doc
    "The false branch of the case above: with b=false the chosen function is `(fn (x) (+ x 10))`,
           so `((choose false) 5)` = 15. The SAME program, run with the other runtime input, takes the
           other branch — pinning that the selection is genuinely by the runtime condition.")
  (input
    (do
      (def (choose (: b Bool)) (if b (fn (x) (+ x 1)) (fn (x) (+ x 10))))
      (def (main (: b Bool)) ((choose b) 5))
      (export main)))
  (call main (: false Bool))
  (output (: 15 Int64)))

(case
  "a runtime-selected function chosen directly at the application head is applied"
  (doc
    "The commuting conversion at the application head directly: `((if b (fn (x) (+ x 1)) (fn (x)
           (- x 1))) 10)`. No intervening def — the `if` sits in head position and the application is
           pushed into its branches. With b=true the result is 11.")
  (input (do (def (main (: b Bool)) ((if b (fn (x) (+ x 1)) (fn (x) (- x 1))) 10)) (export main)))
  (call main (: true Bool))
  (output (: 11 Int64)))

(case
  "a runtime-selected function chosen directly at the application head is applied (false branch)"
  (doc
    "The false branch of the head-position commuting conversion above: `((if b (fn (x) (+ x 1)) (fn
           (x) (- x 1))) 10)` with b=false takes the `(fn (x) (- x 1))` branch → 10 - 1 = 9. The same
           program run with the other runtime input, pinning the head-position selection is genuinely by
           the runtime condition.")
  (input (do (def (main (: b Bool)) ((if b (fn (x) (+ x 1)) (fn (x) (- x 1))) 10)) (export main)))
  (call main (: false Bool))
  (output (: 9 Int64)))

; The COMMUTING CONVERSION also applies to a `match` head, not only an `if`: `((match c (p0 f0) (p1 f1)…)
; args…)` pushes the application into each ARM body → `(match c (p0 (f0 args…)) (p1 (f1 args…))…)` (a
; "case-of-match", the sum analogue of case-of-case). A match whose arms return CLOSURES — the dispatch-
; table idiom `(match c ((C.A n) (fn (x) …)) …)` — then folds each arm's lambda in place, INCLUDING one
; that CAPTURES the arm's payload binder (`(fn (x) (+ x n))`), because the arm's pattern is reused so `n`
; stays in scope for the rewritten body. Sound: only the taken arm runs, so applying in that arm is what
; the original did.
(case
  "applying the result of a match whose arms return payload-capturing closures"
  (doc
    "A `match` selects a closure per variant and the result is applied: `((mk (C.A 10)) 5)` where
           `mk` returns `(fn (x) (+ x n))` from the `C.A n` arm — the closure CAPTURES the arm's payload
           `n`. The application pushes into each arm (case-of-match), and the `C.A` arm's lambda folds
           against `5` with `n` = 10 → 15. Was 'value is not applyable' (a match result was not recognized
           as an applyable head — only an `if` head commuted); now the match head commutes like an `if`.")
  (input
    (do
      (type C (A Int64) B)
      (def (mk (: c C)) (match c ((C.A n) (fn ((: x Int64)) (+ x n))) ((C.B) (fn ((: x Int64)) x))))
      (def (main) ((mk (C.A 10)) 5))
      (export main)))
  (output (: 15 Int64)))

(case
  "a match-of-closures on a runtime-selected variant is applied per arm"
  (doc
    "The runtime companion: the scrutinee is a runtime-selected variant `(if b (C.A 10) (C.B))`, so
           WHICH closure `mk` returns is decided at run time; applying `((mk …) 5)` dispatches to the taken
           arm's closure. b=true → the `C.A 10` arm → `(+ 5 10)` = 15; b=false → the `C.B` identity arm → 5.
           Pins the case-of-match commuting conversion over a runtime scrutinee, not only a constant one.")
  (input
    (do
      (type C (A Int64) B)
      (def (mk (: c C)) (match c ((C.A n) (fn ((: x Int64)) (+ x n))) ((C.B) (fn ((: x Int64)) x))))
      (def (main (: b Bool)) ((mk (if b (C.A 10) (C.B))) 5))
      (export main)))
  (call main (: true Bool))
  (output (: 15 Int64))
  (call main (: false Bool))
  (output (: 5 Int64)))

(case
  "a match returning multi-argument closures applies at full arity"
  (doc
    "The arms return TWO-argument closures — `(fn (x y) (+ (+ x y) n))` — and the result is applied
           to both args at once: `((mk (C.A 100)) 3 4)`. Case-of-match pushes the full multi-argument
           application into each arm, so the taken arm's lambda folds against `[3, 4]` with `n` = 100 →
           107. Pins the commuting conversion carries ALL arguments, not just one.")
  (input
    (do
      (type C (A Int64) B)
      (def
        (mk (: c C))
        (match
          c
          ((C.A n) (fn ((: x Int64) (: y Int64)) (+ (+ x y) n)))
          ((C.B) (fn ((: x Int64) (: y Int64)) (+ x y)))))
      (def (main) ((mk (C.A 100)) 3 4))
      (export main)))
  (output (: 107 Int64)))

; A function stored in a RECORD FIELD, where that record is a SUM's payload, and CALLED after a match
; binds the payload — `(match h ((H.M rec) ((. rec f) x)))`. The projected `(. rec f)` reads a fn value
; off a RUNTIME record (the payload survives the match as a heap value, so it does not fold to the
; lambda), so it must apply via `call_indirect` like any runtime closure. This was declined "value is
; not applyable" — a record-field projection was not recognized as a runtime function-value head the way
; a tuple-element projection (`Proj`) or a payload binder (`SumPayload`) already were. Pins that a fn
; reached through a record field of a sum payload is a first-class callable (the record-field analogue of
; the closure-in-a-sum-payload case above), while a DATA field read and a `(. Sum Variant)` constructor —
; both also member projections — keep their own paths.
(case
  "a function stored in a record field of a sum payload is called after a match"
  (doc
    "`(type H (M (Record (: f (-> Int64 Int64)) (: n Int64))))` carries a record with a FUNCTION field
           `f` and a data field `n`. Matching binds the whole record to `rec`; `((. rec f) rec.n)` projects
           the fn field off the runtime payload record and applies it to the data field — `(fn (x) (+ x 1))`
           applied to 41 → 42. Pins that a fn projected from a record that is a sum payload dispatches via
           call_indirect (it cannot fold — the record is a runtime heap value behind the match), while the
           sibling `rec.n` data read folds as usual.")
  (input
    (do
      (type H (M (Record (: f (-> Int64 Int64)) (: n Int64))))
      (def (run (: h H)) (match h ((H.M rec) (rec.f rec.n))))
      (def (main) (run (H.M #record((= f (fn ((: x Int64)) (+ x 1))) (= n 41)))))
      (export main)))
  (call main)
  (output (: 42 Int64))
  (live-objects known-leak))

; The runtime-condition selection above FOLDS because the chosen function is applied AT the selection
; site — `((if b f g) 5)` commutes the application into each branch, so no function value survives. But
; when the runtime-selected function is instead THREADED THROUGH A RECURSIVE HOF — chosen by `if`, then
; passed to `applyer` and applied inside the recursion — the `if` CANNOT commute into the recursive
; callee, so the selected closure must survive as a genuine runtime heap VALUE and dispatch via
; `call_indirect`. This is the runtime-selected companion to the recursive-HOF case below: the closure's
; identity is decided at run time, yet it is still applied indirectly at each recursion step.
(case
  "a runtime-selected closure survives as a value threaded through a recursive HOF (true branch)"
  (doc
    "`(if b (fn (x) (+ x 10)) (fn (x) (* x 10)))` is selected by the runtime Bool `b`, then passed
           to the recursive `applyer` and applied at each step — the `if` cannot commute into the
           recursion, so the chosen closure is a real runtime value dispatched via call_indirect. With
           b=true the closure is `(+ x 10)`: applyer sums (3+10)+(2+10)+(1+10) = 36.")
  (input
    (do
      (def
        (applyer (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (applyer g (- n 1)))))
      (def
        (main (: b Bool))
        (applyer (if b (fn ((: x Int64)) (+ x 10)) (fn ((: x Int64)) (* x 10))) 3))
      (export main)))
  (call main (: true Bool))
  (output (: 36 Int64))
  (live-objects known-leak))

(case
  "a runtime-selected closure survives as a value threaded through a recursive HOF (false branch)"
  (doc
    "The false branch of the case above: with b=false the chosen closure is `(* x 10)`, so applyer
           sums (3·10)+(2·10)+(1·10) = 60. The SAME program with the other runtime input dispatches the
           other lifted closure through the same recursive indirect-call site — the table slot carried by
           the runtime-selected closure cell selects which code runs.")
  (input
    (do
      (def
        (applyer (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (applyer g (- n 1)))))
      (def
        (main (: b Bool))
        (applyer (if b (fn ((: x Int64)) (+ x 10)) (fn ((: x Int64)) (* x 10))) 3))
      (export main)))
  (call main (: false Bool))
  (output (: 60 Int64))
  (live-objects known-leak))

; A function argument passed to a RECURSIVE higher-order function, applied inside the recursion. This
; is the case a function value MUST exist at run time: the recursive `apply-sum` cannot be inlined
; away (it recurses), so its function parameter `g` is a genuine runtime CLOSURE VALUE — the lambda is
; lambda-lifted to a standalone function and applied through an indirect call, not folded. The whole
; point of first-class functions for a compiler (`core-semantics.md` §A Function Is A First-Class
; Value): a pass maps a function over a recursive structure. `apply-sum g n = g(n)+g(n-1)+…+g(1)`.
(case
  "a function argument is applied through a recursive higher-order function"
  (doc
    "`apply-sum` sums `g` applied to each of n, n-1, …, 1 — a recursive HOF. Its `g` parameter is
           a runtime function value (the recursion prevents inlining `g` away), applied via an indirect
           call. With `g = (fn (x) (* x 2))` and n=3: g(3)+g(2)+g(1) = 6+4+2 = 12. The lambda is lifted
           to a standalone function; a generation with no runtime function representation declines.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: n Int64)) (apply-sum (fn ((: x Int64)) (* x 2)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 12 Int64))
  (live-objects known-leak))

(case
  "a different function argument through the same recursive higher-order function"
  (doc
    "The companion pinning that the closure carries the RIGHT code — a DIFFERENT lambda `(fn (x)
           (+ x 100))` through the same `apply-sum`, so the indirect call must dispatch to THIS
           function, not a fixed one. n=3: (3+100)+(2+100)+(1+100) = 306.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: n Int64)) (apply-sum (fn ((: x Int64)) (+ x 100)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 306 Int64))
  (live-objects known-leak))

; A CAPTURING closure through the recursive HOF — the lambda closes over a free variable `k` from its
; creation scope. `core-semantics.md` §A Function Value Captures The Bindings In Scope Where It Is
; Created: `k` is captured BY VALUE into the closure, so each `g(i)` observes the captured `k`. The
; closure is a heap cell (the code pointer + the captured `k`); applying it reads `k` back from the
; cell. `apply-sum (fn (x) (+ x k)) 3 = (3+k)+(2+k)+(1+k) = 6 + 3k`.
(case
  "a capturing closure is applied through a recursive higher-order function"
  (doc
    "The lambda `(fn (x) (+ x k))` CAPTURES `k` from `main`'s scope — a genuine runtime closure
           with an environment, not just a code pointer. Passed to the recursive `apply-sum` and applied
           at each step, every application observes the captured `k`. With k=10: (3+10)+(2+10)+(1+10) =
           36. A generation that cannot store a captured value in the closure declines.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: k Int64)) (apply-sum (fn ((: x Int64)) (+ x k)) 3))
      (export main)))
  (call main (: 10 Int64))
  (output (: 36 Int64))
  (live-objects known-leak))

; The same runtime closure, but capturing TWO enclosing bindings rather than one — a MULTI-SLOT
; environment. `(fn (x) (+ (+ x a) b))` closes over both `main`'s parameter `a` and the let-bound `b`,
; so the lifted closure cell must carry two captured slots, not one. Threaded through the recursive
; `apply-sum` and applied at each step, every indirect call observes both captured values. This pins
; that the closure environment generalizes past a single capture — the environment product holds an
; arbitrary number of captured slots, read back positionally in the lifted body.
(case
  "a closure capturing two enclosing bindings threads a multi-slot environment through a recursive HOF"
  (doc
    "`(fn (x) (+ (+ x a) b))` captures BOTH `a` (main's parameter) and `b` (an enclosing `let`) —
           a two-slot closure environment, not the single capture of the case above. Passed to the
           recursive `apply-sum` and applied at each step, every application observes both captured
           values. With a=10, b=100: (3+10+100)+(2+10+100)+(1+10+100) = 336. Pins that a runtime
           closure's environment holds MORE THAN ONE captured slot, read back positionally.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: a Int64)) (let ((b 100)) (apply-sum (fn ((: x Int64)) (+ (+ x a) b)) 3)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 336 Int64))
  (live-objects known-leak))

; HIGHER-ORDER CAPTURE — a closure whose captured free variable is ITSELF A FUNCTION. `(fn (b) (g b))`
; closes over `g`, a fn-typed parameter of the enclosing recursive `rec`; the closure cell must store
; `g`'s closure HANDLE as a captured slot and, in the lifted body, read it back and apply it via
; `call_indirect`. `core-semantics.md` §A Function Is A First-Class Value composed with capture: a
; captured value may be any first-class value, a function included. Two subtleties this pins: because
; `rec` recurses, `g` threads through the recursive specialization as a synthesized parameter, so a
; capture whose target is that synthesized param must still be recognized (not mistaken for a global);
; and a `Ty::Fn` capture is a u32 cell handle stored/read AS-IS, like any compound handle, not boxed as
; a scalar. `rec` builds `(fn (b) (g b))`, hands it to the recursive `sumapply` (applied at 2 and 1),
; and sums over its own recursion — each level contributes g(2)+g(1).
(case
  "a closure captures a function value and applies it through a recursive HOF"
  (doc
    "The captured free variable is a FUNCTION: `(fn (b) (g b))` closes over `g`, itself a runtime
           fn parameter, so the closure cell stores `g`'s handle and the lifted body applies it via an
           indirect call. `rec` passes that closure to the recursive `sumapply` (which applies it at 2
           and 1) and repeats over its own recursion. With `g = (fn (x) (+ x 1))`: each level is
           g(2)+g(1) = (2+1)+(1+1) = 5, and over n=3 levels the total is 15. Pins that a closure can
           capture and apply another closure — higher-order capture through a call_indirect.")
  (input
    (do
      (def
        (sumapply (: h (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
      (def
        (rec (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g b)) 2) (rec g (- n 1)))))
      (def (main (: n Int64)) (rec (fn ((: x Int64)) (+ x 1)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 15 Int64))
  (live-objects known-leak))

; A lambda that FORWARDS to a fn parameter SUBSTITUTED into a NON-recursive HOF. `twice` is non-recursive so
; it inlines and its `g` is substituted by `main`'s concrete lambda; the inner `(fn (b) (g b))` is passed to
; the recursive `sumapply`, so it must survive as a runtime closure. (This declined for several ticks on a
; spurious self-capture: descending the nested `(fn …)` in the lifted body tripped the capture-collector's
; own-param-binder guard; it now descends a nested lambda's body with the inner params excluded.)
(case
  "a lambda forwarding to a substituted fn param runs through a recursive HOF"
  (doc
    "`twice` (non-recursive) inlines, substituting `g` = `(fn (x) (+ x 1))`; the inner `(fn (b) (g b))`
           forwards to it and escapes into the recursive `sumapply`. sumapply (fn (b) (g b)) 3 with g=(+1):
           (3+1)+(2+1)+(1+1) = 9.")
  (input
    (do
      (def
        (sumapply (: h (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
      (def (twice (: g (-> Int64 Int64))) (sumapply (fn ((: b Int64)) (g b)) 3))
      (def (main) (twice (fn ((: x Int64)) (+ x 1))))
      (export main)))
  (call main)
  (output (: 9 Int64))
  (live-objects known-leak))

(case
  "a lambda applying its forwarded fn param TWICE runs through a recursive HOF"
  (doc
    "The double-apply face: the inner lambda is `(fn (b) (g (g b)))`, so h(b) = g(g(b)) = b+2 with
           g=(+1); sumapply h 3 = (3+2)+(2+2)+(1+2) = 12. Pins that a nested lambda applying its forwarded
           param more than once still lowers.")
  (input
    (do
      (def
        (sumapply (: h (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
      (def (twice (: g (-> Int64 Int64))) (sumapply (fn ((: b Int64)) (g (g b))) 3))
      (def (main) (twice (fn ((: x Int64)) (+ x 1))))
      (export main)))
  (call main)
  (output (: 12 Int64))
  (live-objects known-leak))

(case
  "a compose combinator captures TWO function values and applies them in declared order"
  (doc
    "The two-function capture face: `(compose f g)` returns `(fn (x) (f (g x)))` — ONE closure whose
           env holds TWO fn handles, applied inner-to-outer. Order is witnessed by non-commuting operands:
           `inc∘dbl` at 5 = (5·2)+1 = 11 but `dbl∘inc` = (5+1)·2 = 12 — a compose that swapped its captured
           slots (or shared one cell for both) would transpose the tuple. The one-capture HOF case above
           holds a single fn; this pins two independently-read fn cells in one env.")
  (input
    (do
      (def (compose (: f (-> Int64 Int64)) (: g (-> Int64 Int64))) (fn ((: x Int64)) (f (g x))))
      (def
        (main (: n Int64))
        (let
          ((inc (fn ((: x Int64)) (+ x 1))) (dbl (fn ((: x Int64)) (* x 2))))
          #tuple(((compose inc dbl) n) ((compose dbl inc) n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 11 12) (Tuple Int64 Int64)))
  (live-objects known-leak))

(case
  "a compose combinator over HEAP-typed functions pipelines list transformers"
  (doc
    "The compose pins are scalar-arrow; this combinator is over (-> (List Int64) (List Int64))
           stages from a closure factory — each stage's result becomes the next's borrowed argument,
           an ownership hand-off chained through closure arrows. seed=0 face.")
  (input
    (do
      (def
        (compose (: f (-> (List Int64) (List Int64))) (: g (-> (List Int64) (List Int64))))
        (fn ((: xs (List Int64))) (f (g xs))))
      (def (pusher (: v Int64)) (fn ((: xs (List Int64))) (List.push xs v)))
      (def
        (sum-l (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: seed Int64))
        (do
          (def p (compose (pusher 2) (pusher 1)))
          (def r (p #list(seed)))
          (+ (* (sum-l r 0) 10) (List.len r))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 123 Int64))
  (call main (: 0 Int64))
  (output (: 33 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "a RUNTIME-selected combiner closure crosses a join and drives a fold"
  (doc
    "The fold-fn pins take the closure as a CONST param (devirtualizable); this SELECTS the
           combiner at a runtime join then folds with it — no devirtualization, N indirect applies.
           The order-sensitive mode-2 combiner catches an arg swap in the indirect-call ABI.")
  (input
    (do
      (def
        (foldc (: xs (List Int64)) (: acc Int64) (: g (-> Int64 Int64 Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (foldc t (g acc h) g))))
      (def
        (main (: mode Int64))
        (do
          (def
            g
            (if
              (= mode 1)
              (fn ((: a Int64) (: h Int64)) (+ a h))
              (fn ((: a Int64) (: h Int64)) (+ (* a 10) h))))
          (foldc #list(1 2 3) 0 g)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 6 Int64))
  (call main (: 2 Int64))
  (output (: 123 Int64))
  (live-objects known-leak))

(case
  "a list of closure-carrying variants interprets as a command pipeline over an accumulator"
  (doc
    "The payload pin extracts ONE closure; this folds a heterogeneous (List Op) of
           closure-carrying variants + unit Skips through a recursive interpreter; ordering matters.
           All-skip face → identity.")
  (input
    (do
      (type Op (Apply (-> Int64 Int64)) (Skip Unit))
      (def
        (run (: ops (List Op)) (: acc Int64))
        (match
          ops
          (#list() acc)
          (#list(h (.. t)) (match h ((Op.Apply f) (run t (f acc))) ((Op.Skip _u) (run t acc))))))
      (def
        (main (: mode Int64))
        (do
          (def
            ops
            (if
              (= mode 1)
              #list((Op.Apply (fn ((: a Int64)) (+ a 3)))
                (Op.Skip unit)
                (Op.Apply (fn ((: a Int64)) (* a 2))))
              #list((Op.Skip unit) (Op.Skip unit))))
          (run ops 10)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 26 Int64))
  (call main (: 2 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

(case
  "a THREE-deep transitive closure-capture chain applies with each level's own capture live"
  (doc
    "The 2-deep capture pin extended: h→g→f where EACH level adds its OWN scalar capture —
           each closure cell holds the previous closure handle + a fresh scalar; each capture
           resolves from ITS definition scope. Digit-separated 326/321.")
  (input
    (do
      (def
        (main (: a Int64))
        (let
          ((c1 1))
          (let
            ((f (fn ((: x Int64)) (+ x c1))))
            (let
              ((c2 20))
              (let
                ((g (fn ((: x Int64)) (+ (f x) c2))))
                (let ((c3 300)) (let ((h (fn ((: x Int64)) (+ (g x) c3)))) (h a))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 326 Int64))
  (call main (: 0 Int64))
  (output (: 321 Int64)))

(case
  "a THREE-DEEP compose chain stacks closure envs with a runtime capture mid-chain"
  (doc
    "The two-fn compose above holds two handles in ONE env; stacking compose THREE deep makes a
           closure whose captured `g` is ITSELF a compose result holding another — application walks
           a three-level env chain inner-to-outer (id, then ·2+1, then ·2+n, then ·2+3). The MIDDLE
           stage captures the runtime n, so the chain isn't const-foldable end-to-end and the n cell
           must survive inside the SECOND env level while the outer level applies after it. f(5) =
           ((5·2+1)·2+n)·2+3: n=0 → 47, n=10 → 67 (the +20 delta = n routed through exactly ONE
           doubling — a capture landing one level off doubles it twice or not at all, shifting the
           delta to 40 or 10). NB the same chain built by a RECURSIVE fold over an op list declines
           (the fn-typed accumulator through recursion is the known inference frontier); this pins
           the unrolled form that must keep working.")
  (input
    (do
      (def (compose (: f (-> Int64 Int64)) (: g (-> Int64 Int64))) (fn ((: x Int64)) (f (g x))))
      (def
        (main (: n Int64))
        (do
          (def id (fn ((: x Int64)) x))
          (def s1 (compose (fn ((: x Int64)) (+ (* x 2) 1)) id))
          (def s2 (compose (fn ((: x Int64)) (+ (* x 2) n)) s1))
          (def s3 (compose (fn ((: x Int64)) (+ (* x 2) 3)) s2))
          (s3 5)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 47 Int64))
  (call main (: 10 Int64))
  (output (: 67 Int64)))

(case
  "a SELF-RECURSIVE named def passed as a value is applied twice through the parameter"
  (doc
    "The recursive upgrade of first-class named functions: `fact` — a self-recursive def — is passed
           BY NAME to `apply-twice`, which applies it through its fn PARAMETER twice: `(fact (fact 3))` =
           `fact 6` = 720. Each indirect application must re-enter fact's own recursion (the inner call runs
           4 recursive levels, the outer 6) — the callee reference reaching the recursive def through a
           first-class fn value, not a direct call site. A lift that specialized the parameter to a
           non-recursive snapshot (or bound the self-call to the wrong frame) would break the outer
           application.")
  (input
    (do
      (def (fact (: n Int64)) (if (< n 2) 1 (* n (fact (- n 1)))))
      (def (apply-twice (: f (-> Int64 Int64)) (: x Int64)) (f (f x)))
      (def (main (: n Int64)) (apply-twice fact n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 720 Int64)))

(case
  "a runtime BRANCH selects between two named functions and the selected one applies"
  (doc
    "The function-valued conditional: `((if b inc dbl) x)` — the `if` yields a FUNCTION value chosen
           by a runtime Bool, immediately applied. true → inc(5) = 6, false → dbl(5) = 10. The conditional's
           result type is the arrow `(-> Int64 Int64)` both arms share; the application must dispatch to
           whichever function the branch selected at run time (an emit binding the call to one arm's callee
           statically would break the other call). The strategy-selection idiom.")
  (input
    (do
      (def (inc (: x Int64)) (+ x 1))
      (def (dbl (: x Int64)) (* x 2))
      (def (main (: b Bool) (: x Int64)) ((if b inc dbl) x))
      (export main)))
  (call main (: true Bool) (: 5 Int64))
  (output (: 6 Int64))
  (call main (: false Bool) (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a branch selects between MUTUALLY-recursive defs and the selected one runs its cycle"
  (doc
    "The mutual-recursion upgrade: `pick` returns `even` or `odd` — each half of a mutually-recursive
           PAIR — as a first-class value, and the caller applies the selection. `(pick true) 4` runs
           even→odd→even→odd→even = 1; `(pick false) 4` starts the cycle at odd = 0. The selected function
           value must carry its whole mutual-recursion group (a reference resolving only the named def
           without its partner would break the first cross-call).")
  (input
    (do
      (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))
      (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1))))
      (def (pick (: b Bool)) (if b even odd))
      (def (main (: b Bool) (: n Int64)) ((pick b) n))
      (export main)))
  (call main (: true Bool) (: 4 Int64))
  (output (: 1 Int64))
  (call main (: false Bool) (: 4 Int64))
  (output (: 0 Int64)))

(case
  "a recursive combinator applies a captured closure a RUNTIME number of times"
  (doc
    "The iterate combinator: `times f n x` re-applies its fn PARAMETER `f` per recursive step, the
           count a boundary parameter — `times (·2) 5 1` doubles five times (32), `times (·2) 0 1` applies
           zero times (the seed, 1). The apply-twice case above fixes the application count at compile
           time; here the SAME closure value is applied a runtime-decided number of times through the
           parameter, so the indirect call sits on the loop's spine (a lower that unrolled or specialized
           per count could not — n arrives per call).")
  (input
    (do
      (def
        (times (: f (-> Int64 Int64)) (: n Int64) (: x Int64))
        (if (< n 1) x (times f (- n 1) (f x))))
      (def (main (: n Int64)) (times (fn ((: v Int64)) (* v 2)) n 1))
      (export main)))
  (call main (: 5 Int64))
  (output (: 32 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a closure built OVER another closure applies both capture layers"
  (doc
    "Nested capture: `twice` captures a FUNCTION VALUE `f` (itself a closure capturing the boundary
           `k`) and returns `(fn (x) (f (f x)))` — a closure whose captured cell holds another closure.
           `(twice (adder k)) 100` = 100+k+k: k=3 → 106, k=-50 → 0. Both layers must survive — the outer
           closure's environment carries the inner closure handle, and each application descends through
           BOTH captures (an environment layout that inlined or flattened the inner capture into the outer
           would break the second call's k). The compose case pins two peer captures; this pins capture
           NESTING.")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (twice (: f (-> Int64 Int64))) (fn ((: x Int64)) (f (f x))))
      (def (main (: k Int64)) ((twice (adder k)) 100))
      (export main)))
  (call main (: 3 Int64))
  (output (: 106 Int64))
  (call main (: -50 Int64))
  (output (: 0 Int64)))

(case
  "a runtime branch selects between two ANONYMOUS closures fed to the iterate combinator"
  (doc
    "The composition of the two pins above with the branch-select idiom: `(if (= pick 0) (fn +1)
           (fn ·3))` yields one of two ANONYMOUS closures (the named-def branch case above selects defs;
           anonymous fns lower through the closure path with their own environments), which the `times`
           combinator then applies n times. pick=0 → 1+1·4 = 5; pick=1 → 3⁴ = 81. The selected closure
           must ride through the combinator's fn parameter regardless of which arm built it — one call
           site, two possible environments, a runtime iteration count.")
  (input
    (do
      (def
        (times (: f (-> Int64 Int64)) (: n Int64) (: x Int64))
        (if (< n 1) x (times f (- n 1) (f x))))
      (def
        (main (: pick Int64) (: n Int64))
        (times (if (= pick 0) (fn ((: v Int64)) (+ v 1)) (fn ((: v Int64)) (* v 3))) n 1))
      (export main)))
  (call main (: 0 Int64) (: 4 Int64))
  (output (: 5 Int64))
  (call main (: 1 Int64) (: 4 Int64))
  (output (: 81 Int64))
  (live-objects known-leak))

; NESTED CAPTURING CLOSURES — a closure captures another closure that ITSELF captures. `g = (fn (x) (f
; (+ x 1)))` captures `f`, and `f = (fn (y) (+ y k))` captures `k`. Inside `g`'s lifted body, `f` is a
; runtime closure HANDLE read from `g`'s env cell — NOT the compile-time lambda it was defined from — so
; `(f …)` must apply via `call_indirect` (which threads `f`'s OWN env, carrying `k`), not β-reduce to the
; original definition. This pins that a captured value that happens to be a function is applied as a
; runtime closure (its own environment preserved), rather than followed back to its definition and folded.
(case
  "a closure captures a capturing closure and calls it through a recursive HOF"
  (doc
    "`g = (fn (x) (f (+ x 1)))` captures `f`, itself the capturing closure `(fn (y) (+ y k))` over
           `k`. Inside `g`'s lifted body `f` is a runtime handle applied via an indirect call that threads
           `f`'s own env (carrying `k`), not the original lambda. `ap g 2` = g(2)+g(1) = (2+1+k)+(1+1+k);
           with k=100 that is 103+102 = 205. Pins nested capturing closures — a captured function is
           called as a runtime closure with its own environment intact.")
  (input
    (do
      (def (ap (: g (-> Int64 Int64)) (: n Int64)) (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
      (def
        (main (: k Int64))
        (let ((f (fn ((: y Int64)) (+ y k)))) (ap (fn ((: x Int64)) (f (+ x 1))) 2)))
      (export main)))
  (call main (: 100 Int64))
  (output (: 205 Int64))
  (live-objects known-leak))

; A NESTED LAMBDA inside a lifted closure body. `g = (fn (x) ((fn (y) (+ y k)) x))` is a runtime closure
; (passed to the recursive `ap`) whose body applies an inner lambda `(fn (y) (+ y k))` in place. The inner
; application must β-REDUCE during lowering — `((fn (y) (+ y k)) x)` → `(+ x k)` — so the lifted body is a
; simple capturing closure over `k`, NOT a body carrying an un-lowered nested lambda. (Analyzing the outer
; body must descend a nested lambda with its OWN params excluded — the inner `y` is bound locally, neither
; a capture of the outer nor a self-reference — so the nested lambda does not spuriously decline the lift.)
; `ap g 2` with k=10 = (2+10)+(1+10) = 23.
(case
  "a closure whose body applies a nested lambda in place runs through a recursive HOF"
  (doc
    "`(fn (x) ((fn (y) (+ y k)) x))` is a runtime closure over `k` whose body applies an inner
           lambda to `x`; the inner application β-reduces to `(+ x k)` during lowering, so the lifted body
           is a plain capturing closure. `ap g 2` with k=10 = (2+10)+(1+10) = 23. Pins that a nested lambda
           inside a lifted closure body reduces rather than declining the lift.")
  (input
    (do
      (def (ap (: g (-> Int64 Int64)) (: n Int64)) (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
      (def (main (: k Int64)) (ap (fn ((: x Int64)) ((fn ((: y Int64)) (+ y k)) x)) 2))
      (export main)))
  (call main (: 10 Int64))
  (output (: 23 Int64))
  (live-objects known-leak))

; A runtime closure whose body CALLS A RECURSIVE TOP-LEVEL FUNCTION. `(fn (x) (fact x))` is lifted (it is
; passed to the recursive `ap`, so it cannot fold), and its body invokes the recursive `fact` — a
; `Core::Call` to a standalone wasm function nested inside a `call_indirect`ed closure body. This is the
; canonical "map a recursive function over a structure" shape a real compiler needs: the closure survives
; as a runtime value AND its body drives an ordinary recursive call. `ap (fn (x) (fact x)) 3` sums
; fact(3)+fact(2)+fact(1) = 6+2+1 = 9.
(case
  "a runtime closure whose body calls a recursive top-level function"
  (doc
    "`(fn (x) (fact x))` is a runtime closure (passed to the recursive `ap`) whose body calls the
           recursive `fact`. The lifted closure body holds a `Core::Call` to `fact` — a recursive wasm
           function invoked from inside a call_indirect'd closure. `ap g 3` = fact(3)+fact(2)+fact(1) =
           6+2+1 = 9. Pins that a lifted closure's body can drive an ordinary recursive call.")
  (input
    (do
      (def (fact (: m Int64)) (if (= m 0) 1 (* m (fact (- m 1)))))
      (def (ap (: g (-> Int64 Int64)) (: n Int64)) (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
      (def (main (: n Int64)) (ap (fn ((: x Int64)) (fact x)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

; A runtime closure that COMPARES its argument to a CAPTURED value in an `if`. `(fn (x) (if (= x k) 1 0))`
; captures `k` and branches on `x == k` — the captured `k` feeds a comparison whose boolean drives an `if`
; inside the lifted body. `ap g 3` with k=2 counts how many of 3,2,1 equal 2, weighted 1 each = 1.
(case
  "a runtime closure compares its argument to a captured value in a branch"
  (doc
    "`(fn (x) (if (= x k) 1 0))` captures `k` and compares its parameter against it, branching on
           the result. Through the recursive `ap` with k=2 over 3,2,1: only x=2 matches, so the sum is 1.
           Pins that a captured value drives a comparison + branch inside a lifted closure body.")
  (input
    (do
      (def (ap (: g (-> Int64 Int64)) (: n Int64)) (if (= n 0) 0 (+ (g n) (ap g (- n 1)))))
      (def (main (: k Int64)) (ap (fn ((: x Int64)) (if (= x k) 1 0)) 3))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; MANUAL ETA-WRAP of a genuinely-RUNTIME function value. `g` is a runtime two-parameter fn PARAMETER (of
; the recursive `ap`), so it has no compile-time lambda to partially apply. Writing `(fn (b) (g n b))`
; captures `g` (a runtime closure handle) AND `n`, and applies `g` at full arity inside — the eta-wrapper
; is an ordinary capturing closure whose body is a full-arity `call_indirect` on the captured `g`. This is
; the composition of two runtime paths: an outer closure that captures a runtime fn value and CALLS it,
; passed to a second recursive HOF. Both `ap` and `sumapply` recurse, so nothing folds — the program runs
; on TWO nested indirect calls (ap→the eta-wrapper, the eta-wrapper→g). `ap g n` sums over i=n…1 of
; `sumapply((fn (b) (g i b)), 2)` = (g(i,2))+(g(i,1)) = (i+2)+(i+1) = 2i+3; for n=3: 9+7+5 = 21.
(case
  "a runtime function value is manually eta-wrapped and applied through nested recursive HOFs"
  (doc
    "`g` is a runtime two-parameter fn parameter; `(fn (b) (g n b))` captures `g` and `n` and applies
           `g` at full arity inside — a capturing closure whose body is an indirect call on the captured
           runtime `g`. Passed to the recursive `sumapply`, itself driven by the recursive `ap`, so nothing
           folds: two nested call_indirects (ap→wrapper, wrapper→g). `ap g 3` = sum over i=3,2,1 of
           (g(i,2)+g(i,1)) = (2i+3) = 9+7+5 = 21. Pins that a genuinely-runtime fn value can be captured by
           an eta-wrapper and applied — the manual form of runtime currying, on the capture + full-arity
           machinery.")
  (input
    (do
      (def
        (sumapply (: h (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
      (def
        (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
        (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g n b)) 2) (ap g (- n 1)))))
      (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 21 Int64))
  (live-objects known-leak))

; A CURRIED lambda STORED in a tuple, then PROJECTED and applied through both levels. `(fn (a) (fn (b) (+
; (+ a b) x)))` (curried, capturing `x`) is stored in `ops`; `(((. ops 0) 3) 4)` projects it and applies
; through both arrows → (3+4)+5 = 12. This shape USED TO DECLINE at emit ("parameter reference has no local
; slot") while `cdz check` passed — a check-vs-compile gap: a projection-only compound holding the lambda
; folded through, β-reduced the OUTER lambda, yielded the INNER `(fn (b) …)`, and inlining that returned
; lambda dangled its param `b` (no closure frame). The fix keeps the compound (materialized) when its
; element is a CURRIED lambda, so the element lifts as a runtime closure and applies via `call_indirect`.
; A FLAT/single-arg fn element still folds (no regression). Pins that a stored curried closure is a
; callable first-class value across both currying levels.
(case
  "a curried lambda stored in a tuple is projected and applied through both levels"
  (doc
    "`(let ((x 5)) (let ((ops (tuple (fn (a) (fn (b) (+ (+ a b) x)))))) (((. ops 0) 3) 4)))` stores a
           curried capturing lambda in a tuple, projects it, and applies through both arrows: (3+4)+5 = 12.
           A projection-only compound holding a CURRIED lambda must be KEPT (materialized) so the element
           lifts as a runtime closure — folding it through β-reduced the outer lambda and dangled the inner
           lambda's param at emit ('no local slot') though check passed. A flat/single-arg fn element still
           folds unchanged. Pins the stored-curried-closure-projected-and-applied path.")
  (input
    (do
      (def
        (main)
        (let ((x 5)) (let ((ops #tuple((fn (a) (fn (b) (+ (+ a b) x)))))) (((. ops 0) 3) 4))))
      (export main)))
  (call main)
  (output (: 12 Int64))
  (live-objects known-leak))

(case
  "a flat multi-param lambda stored in a tuple is applied through curried syntax"
  (doc
    "`(let ((t (tuple (fn (a b) (+ a b))))) (((. t 0) 3) 4))` stores a FLAT 2-param lambda, projects
           it, and applies it through CURRIED syntax `((f a) b)` → 7. The inner `((. t 0) 3)` is a PARTIAL
           application of a projected fn (its result is still a function); β-reducing it dangled the residual
           at emit. Instead the whole curried spine routes to one runtime `CallClosure` (materialize the
           element + call_indirect at full arity). Distinct from a DIRECT full application `(f a b)`, and from
           the capturing-single FULL-apply that must stay FOLDED (the case above / #A Capturing Closure …).
           The flat-lambda companion of the curried-lambda case above.")
  (input (do (def (main) (let ((t #tuple((fn (a b) (+ a b))))) (((. t 0) 3) 4))) (export main)))
  (call main)
  (output (: 7 Int64)))

; An IF-OF-PARTIAL-CTORS applied — the eta-closure shape whose `CallClosure` operand is an `Core::If` whose
; arms are each a fresh, partially-applied constructor. Selecting the ctor by an `if` then completing the
; application distributes the final projection into both arms, so the operand the `CallClosure` emit sees is
; an `If` joining two owned closure cells. `heap_operand_ownership` classifies each `Core::Closure` arm as
; Owned (a freshly-built cell, like `SumNew`/`Tuple`), and the `Core::If` join carries that Owned through — a
; value-correctness precondition for the SITE-A reclaim work in the effects/rc layer (the cell must be a
; genuine owned temporary before its reclaim can be gated). This pins the emitted VALUE stays correct on BOTH
; arms and across O0..O3, so the ownership-classification + eventual cell-drop transition can't silently
; miscompute the completed constructor.
(case
  "an if-selected partially-applied constructor is completed by application (eta-closure), both arms"
  (doc
    "`((if c (T.Mk 0) (T.Mk 10)) 5)` selects one of two PARTIAL applications of the 2-arg ctor `Mk`
           under an `if`, then applies the result to `5` to complete it — an eta-closure whose `CallClosure`
           operand is an `If` of two fresh owned closure cells. `c=true` builds `Mk 0 5` → `0+5=5`; `c=false`
           builds `Mk 10 5` → `10+5=15`. Pins the completed constructor computes the right value on both arms
           (the `Core::Closure`-Owned classification + `If`-join must preserve the arm's partial args), and —
           anchored by the opt-sweep — that it stays correct at every optimization level.")
  (input
    (do
      (type T (Mk Int64 Int64))
      (def (main (: c Bool)) (match ((if c (T.Mk 0) (T.Mk 10)) 5) ((Mk a b) (+ a b))))
      (export main)))
  (call main (: true Bool))
  (output (: 5 Int64))
  (call main (: false Bool))
  (output (: 15 Int64)))

; The BORROWED-operand companion of the inline eta-closure pin above. When the if-of-partial-ctors is
; LET-BOUND first (`(let ((g (if c (T.Mk 1) (T.Mk 10)))) (g 5))`), the `CallClosure` operand reaches emit as
; a `Core::LocalRef` to `g`, which `heap_operand_ownership` classifies BORROWED (select.rs) — NOT Owned. So
; SITE-A part b (`0c0adc7e4`) does NOT drop the cell here (its gate requires an Owned operand); the `let`'s
; own drop reclaims it instead. This exercises the OTHER reclaim path than the inline pin (Owned → part-b
; drops): a part-b that ignored the Owned gate and dropped a borrowed LocalRef cell would DOUBLE-FREE it (the
; let drops it too), corrupting the value or trapping. Pins that the let-bound closure computes correctly AND
; is reclaimed exactly once. `c=true` → `Mk 1 5` → 6; `c=false` → `Mk 10 5` → 15.
(case
  "a let-bound if-selected partial constructor is applied and reclaimed once (borrowed-operand path)"
  (doc
    "The let-bound companion of the inline eta-closure pin: `(let ((g (if c (T.Mk 1) (T.Mk 10)))) (g
           5))` binds the if-of-partial-ctors to `g`, so the `CallClosure` operand is a `Core::LocalRef`
           (BORROWED, not Owned) — SITE-A part b leaves the cell to the `let`'s drop rather than dropping it
           itself. A part-b that dropped the borrowed cell would double-free it (the let drops it too). Pins
           the value is correct AND the env cell is reclaimed exactly once: `c=true` → `Mk 1 5` → 6; `c=false`
           → `Mk 10 5` → 15. Complements the inline `((if c …) 5)` Owned-operand pin above.")
  (input
    (do
      (type T (Mk Int64 Int64))
      (def (main (: c Bool)) (let ((g (if c (T.Mk 1) (T.Mk 10)))) (match (g 5) ((Mk a b) (+ a b)))))
      (export main)))
  (call main (: true Bool))
  (output (: 6 Int64))
  (call main (: false Bool))
  (output (: 15 Int64)))

(case
  "a partial constructor stored in a runtime tuple is projected and completed by application"
  (doc
    "The runtime-tuple-stored companion of the if-selected eta-closure cases above: `mk` returns
           `(tuple (T.Mk 10) n)` — a partially-applied 2-arg ctor `(T.Mk 10)` held in a runtime tuple — and
           `main` PROJECTS element 0 and applies it to 5, completing `(T.Mk 10 5)` → a+b = 15. The
           CallClosure operand reaches emit via a TUPLE PROJECTION (not inline, nor a let LocalRef), a path a
           newtype-erasing partial lowering once collapsed to the bare 10 → invalid wasm. Relocated from rcdzc
           a_partial_ctor_in_a_runtime_tuple_completes_via_an_eta_closure_lift.")
  (input
    (do
      (type T (Mk Int64 Int64))
      (def (mk (: n Int64)) (if (< n 0) #tuple((T.Mk 0) 0) #tuple((T.Mk 10) n)))
      (def (main) (let ((p (mk 1))) (match ((. p 0) 5) ((T.Mk a b) (+ a b)))))
      (export main)))
  (call main)
  (output (: 15 Int64))
  (live-objects known-leak))

; A PREDICATE closure — a runtime closure whose RESULT TYPE is Bool. `(fn (x) (= x k))` is a `(-> Int64
; Bool)` value threaded through the recursive `anyp` ("does any i in n…1 satisfy the predicate?"), which
; SHORT-CIRCUITS on the first `true`. The closure's result crosses the `call_indirect` boundary as a
; boolean (an i32 the lifted signature returns), and drives `anyp`'s `if`. This complements the Int-result
; closures above: a lifted closure may return a Bool, and an "exists" HOF consumes it with early exit.
(case
  "a predicate closure returning Bool drives an early-exit recursive HOF"
  (doc
    "`(fn (x) (= x k))` is a `(-> Int64 Bool)` closure over `k`; `anyp` applies it down n…1 and
           returns true on the first match (short-circuit). With k=2 over 3,2,1 the predicate holds at
           x=2, so `anyp` is true and `main` yields 100; with a k absent from 3,2,1 it is false → 0. Pins
           that a runtime closure whose RESULT is Bool applies via call_indirect and its boolean drives the
           caller's branch.")
  (input
    (do
      (def
        (anyp (: g (-> Int64 Bool)) (: n Int64))
        (if (= n 0) false (if (g n) true (anyp g (- n 1)))))
      (def (main (: k Int64)) (if (anyp (fn ((: x Int64)) (= x k)) 3) 100 0))
      (export main)))
  (call main (: 2 Int64))
  (output (: 100 Int64))
  (live-objects known-leak))

; A closure that captures a BOOLEAN. The captured value's TYPE decides the runtime op that unboxes it
; from the env cell — an integer capture reads `get-int`, a boolean reads `get-bool`. That op is emitted
; ONLY inside the LIFTED closure body, never in a top-level def, so the module's import set (which is
; walked to fix each op's import index) must include ops used only in lifted bodies — else `get-bool`
; resolves to a bogus index and the component is invalid. This case exercises a boolean capture read
; back inside the closure: `(fn (x) (if flag (* x 2) x))` closes over the boolean `flag`.
(case
  "a closure captures a boolean and reads it back inside its lifted body through a recursive HOF"
  (doc
    "`(fn (x) (if flag (* x 2) x))` captures the boolean `flag` from `main`'s scope; the lifted
           closure body unboxes it with `get-bool` (an op used ONLY in the lifted body, so it must be
           collected into the import set from the lifted bodies, not just the top-level defs). Passed to
           the recursive `apply-sum` and applied at each step. With flag=true the closure doubles, so
           apply-sum over 3,2,1 = 6+4+2 = 12. Pins that a captured boolean round-trips through the env
           cell and that a lifted-body-only runtime op is imported.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: flag Bool)) (apply-sum (fn ((: x Int64)) (if flag (* x 2) x)) 3))
      (export main)))
  (call main (: true Bool))
  (output (: 12 Int64))
  (live-objects known-leak))

; A closure that captures a COMPOUND value — a tuple — and projects it inside the body. The captured
; value is a u32 heap HANDLE (not a boxed scalar), stored into the env cell as-is and read back as-is;
; the projections `(. p 0)`/`(. p 1)` then index the captured tuple. This pins that a capture slot holds
; a compound handle (the tuple), distinct from a scalar capture (an int/bool boxed into the slot), and
; that reading it back and projecting it works through the recursive indirect-call boundary.
(case
  "a closure captures a tuple and projects it inside its lifted body through a recursive HOF"
  (doc
    "`(fn (x) (+ (+ x (. p 0)) (. p 1)))` captures the tuple `p = (tuple 10 20)` — a compound heap
           handle stored in the closure's env cell as-is — and projects both elements inside the body.
           Passed to the recursive `apply-sum`: each application adds 10+20=30, so over 3,2,1 the total
           is (3+30)+(2+30)+(1+30) = 96. Pins that a captured compound (a tuple handle) round-trips
           through the env cell and its projections work at run time.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def
        (main)
        (let ((p #tuple(10 20))) (apply-sum (fn ((: x Int64)) (+ (+ x (. p 0)) (. p 1))) 3)))
      (export main)))
  (output (: 96 Int64))
  (live-objects known-leak))

; A closure that captures a SUM value and MATCHES it inside the body. The captured `(Some 100)` is a sum
; handle stored in the env cell; the body's `match` reads it back and switches on its discriminant. This
; pins that a captured sum survives the env round-trip AND that a match whose scrutinee is a CAPTURED
; free variable (not a param or a local) lowers correctly inside a lifted closure body.
(case
  "a closure captures a sum value and matches it inside its lifted body through a recursive HOF"
  (doc
    "`(fn (x) (match o ((Some v) (+ x v)) (None x)))` captures the sum `o = (Some 100)` and matches
           it in the body — the scrutinee is a CAPTURED free variable read from the env cell. Passed to
           the recursive `apply-sum`: each application takes the `Some` arm and adds 100, so over 3,2,1
           the total is (3+100)+(2+100)+(1+100) = 306. Pins that a captured sum round-trips through the
           env cell and a match over a captured scrutinee works inside a lifted closure body.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def
        (main)
        (let
          ((o (Some 100)))
          (apply-sum (fn ((: x Int64)) (match o ((Some v) (+ x v)) (None x))) 3)))
      (export main)))
  (output (: 306 Int64))
  (live-objects known-leak))

; PERCEUS RETAIN ACROSS A CAPTURE: a heap list `xs` is CAPTURED by a closure whose body CONSUMES it
; (`List.push xs k` — a persistent op that FBIP-mutates its operand in place when it holds the sole
; reference), and the ORIGINAL `xs` is READ AGAIN after the closure is applied. The capture and the later
; read share `xs`, so the consuming push inside the closure must leave `xs` unchanged — the capture stored
; in the env cell is a live reference that must be retained, exactly as a repeated in-body use is (the
; still-live-binding retain, extended across the closure-capture boundary). Pins that the persistence
; guarantee holds when the shared reader is a captured free variable, not a second in-body occurrence.
(case
  "a list captured by a closure that consumes it is unchanged for a later read of the binding"
  (doc
    "`xs = build 0 3` = `[0 1 2]`; a closure `(fn (k) (List.len (List.push xs k)))` captures `xs` and
           pushes to it (length → 4 when applied with 99), and after the application the ORIGINAL `xs` is
           read (`List.len xs` → 3), so 4 + 3 = 7. If the captured `xs` were not retained, the closure's
           `List.push` would FBIP-mutate the shared backing in place and the later `List.len xs` would read
           the grown list (→ 8). `build` makes `xs` a genuine runtime list (no const-fold). Pins that a
           persistent op on a CAPTURED heap value leaves the still-live original binding unchanged.")
  (input
    (do
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
      (def (apply-it (: f (-> Int64 Int64)) (: x Int64)) (f x))
      (def
        (main (: n Int64))
        (let
          ((xs (build 0 n #list())))
          (+ (apply-it (fn ((: k Int64)) (List.len (List.push xs k))) 99) (List.len xs))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 7 Int64))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

; An UNANNOTATED closure parameter — `(fn (x) …)` with no `(: x T)` — is grounded from its USES in the
; body, exactly as a recursive def's unannotated parameter is (`type-system.md`: a parameter's type is
; solved from how it is used). `(fn (x) (* x 2))` uses `x` as an integer operand, so `x : Int64` falls
; out; the closure lifts with that machine type, needing no annotation. Same runtime path as the
; annotated case above, only the parameter's type is inferred rather than declared.
(case
  "an unannotated closure parameter is grounded from its body and applied at runtime"
  (doc
    "`(fn (x) (* x 2))` has no annotation on `x`; its type is solved from the body's `(* x 2)`
           (an integer operand → `x : Int64`). Passed to the recursive `apply-sum` and applied via the
           indirect call, `apply-sum (fn (x) (* x 2)) 3 = 6+4+2 = 12`. Pins that a bare-parameter lambda
           lifts to a runtime closure without requiring an explicit parameter type.")
  (input
    (do
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: n Int64)) (apply-sum (fn (x) (* x 2)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 12 Int64))
  (live-objects known-leak))

; The same UNANNOTATED closure, but now the recursive HOF's FUNCTION PARAMETER is ALSO unannotated — the
; case above declares `(: g (-> Int64 Int64))`, whose concrete arrow fed the closure's param type. Here
; `mapsum`'s `f` has no annotation, so its solved type is a fully-generic arrow `(-> _ Int64)` — the
; closure's storage context is a HOLE, not a concrete arrow. The closure must therefore be grounded from
; its OWN body alone: `(fn (x) (+ x 1))` uses `x` as an integer operand → `x : Int64`. A generation that
; let the generic HOF param's unsolved-var domain preempt that body-solve DECLINED "a closure's parameter
; type has no machine representation"; the closure's own use must win. `mapsum f acc xs = acc + Σ f(xᵢ)`,
; with a BOUNDARY `acc = n` so nothing folds (a real `call_indirect` over the lifted closure): over 5,7,30
; the result is `n + (5+1)+(7+1)+(30+1) = n + 45`.
(case
  "an unannotated closure is inferred through an unannotated recursive HOF parameter"
  (doc
    "Both the closure `(fn (x) (+ x 1))` AND the recursive HOF's function parameter `f` are
           unannotated, so `f`'s solved type is a generic `(-> _ Int64)` — the closure's context arrow is
           an unsolved hole. The closure's parameter is grounded from its own body's `(+ x 1)` (`x :
           Int64`) rather than from that hole. `mapsum f acc xs = acc + Σ f(xᵢ)`; with a runtime `acc = n`
           the fold runs via call_indirect over the lifted closure, `n + (5+1)+(7+1)+(30+1) = n + 45`.
           Pins that a bare closure passed to a bare recursive HOF param infers from its body, not the
           generic context — the idiomatic fully-inferred `fold`, previously declined 'no machine
           representation'.")
  (input
    (do
      (def (mapsum f acc xs) (match xs (#list() acc) (#list(h (.. t)) (mapsum f (+ acc (f h)) t))))
      (def (main (: n Int64)) (mapsum (fn (x) (+ x 1)) n #list(5 7 30)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 45 Int64))
  (call main (: 100 Int64))
  (output (: 145 Int64))
  (live-objects known-leak))

; The MULTI-PARAMETER twin: the idiomatic two-argument left fold. The closure `(fn (x a) (+ a x))` and the
; recursive HOF's `f` are BOTH unannotated. `fold-list` is generic in `f`, so the call MONOMORPHIZES — the
; specialized copy re-annotates `f` with the argument's type. A bare closure types `(-> Any (-> Any Int64))`
; bottom-up, and a nested `Any` in a type-value encodes as `Unit`, so the copy would get `f : (-> Unit (->
; Unit Int64))` and its `(f h acc)` conflict CDZ0203 (it `check`ed clean but declined at emit). The closure's
; OWN body determines its params (`(+ a x)` → both Int64); solving them before the specialized annotation is
; built gives the concrete `(-> Int64 (-> Int64 Int64))`. `fold-list f acc xs = acc folded with f over xs`;
; BOUNDARY `acc = n` so nothing folds — `n + 5 + 7 + 30 = n + 42`.
(case
  "an unannotated two-argument closure is inferred through a generic recursive HOF"
  (doc
    "The two-argument left-fold callback `(fn (x a) (+ a x))` and the HOF param `f` are both
           unannotated; `fold-list` is generic in `f` so the call monomorphizes. The specialized copy must
           re-annotate `f` with the closure's CONCRETE type — solved from its body (`(+ a x)` → `(-> Int64
           (-> Int64 Int64))`), not the bottom-up `(-> Any (-> Any Int64))` whose `Any` holes encode as
           `Unit` and mistype the copy. With a runtime `acc = n` the fold runs via call_indirect,
           `n + 5 + 7 + 30 = n + 42`. Pins the idiomatic fully-inferred two-arg `foldl`; previously it
           `check`ed clean but declined CDZ0203 at emit with `f : (-> Unit (-> Unit Int64))`.")
  (input
    (do
      (def
        (fold-list f acc xs)
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))
      (def (main (: n Int64)) (fold-list (fn (x a) (+ a x)) n #list(5 7 30)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64))
  (call main (: 100 Int64))
  (output (: 142 Int64))
  (live-objects known-leak))

; The same monomorphized fold, but the two closure parameters have DISTINCT types — the accumulator is
; `Int64` and the element is `String`. The closure `(fn (acc s) (+ acc (String.byte-len s)))` must solve
; BOTH params from its body (`acc : Int64` from `(+ acc …)`, `s : String` from `(String.byte-len s)`), so
; the monomorphized copy's `f` is `(-> Int64 (-> String Int64))` — proving the closure-arg param-solve is
; per-parameter and type-directed, not a single uniform type smeared across the slots. Folds each string's
; byte length into the accumulator: `n + 2 + 4 + 1 = n + 7`.
(case
  "an unannotated closure with distinct-typed params is inferred through a generic recursive HOF"
  (doc
    "A two-argument fold callback whose params have DIFFERENT types — `(fn (acc s) (+ acc
           (String.byte-len s)))`, `acc : Int64` and `s : String` — passed unannotated to the generic
           recursive `foldstr`. Each closure param is solved from its OWN use in the body, so the
           monomorphized copy gets `f : (-> Int64 (-> String Int64))`; a uniform-type solve would mistype
           one slot. With runtime `acc = n` the fold sums the byte lengths of `ab`,`abcd`,`x`:
           `n + 2 + 4 + 1 = n + 7`. Pins that closure-argument inference is per-parameter and type-directed
           through monomorphization.")
  (input
    (do
      (def (foldstr f acc xs) (match xs (#list() acc) (#list(h (.. t)) (foldstr f (f acc h) t))))
      (def
        (main (: n Int64))
        (foldstr (fn (acc s) (+ acc (String.byte-len s))) n #list("ab" "abcd" "x")))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: 100 Int64))
  (output (: 107 Int64))
  (live-objects known-leak))

; A single-argument unannotated closure whose RESULT is Bool — a predicate threaded through a recursive
; HOF that counts how many elements satisfy it. `(fn (x) (< x 10))` solves `x : Int64` from `(< x 10)` and
; its result is `Bool`; the recursive `countif` uses that boolean to drive an `if`. Confirms the inferred
; closure's result type (not just its params) crosses the runtime `call_indirect` correctly for a
; non-numeric result. Over `5,20,7` two elements are `< 10`, so `n + 2`.
(case
  "an unannotated predicate closure is inferred through a recursive counting HOF"
  (doc
    "A bare predicate `(fn (x) (< x 10))` — result type `Bool`, param solved `Int64` from the
           comparison — passed unannotated to the recursive `countif`, which increments an accumulator when
           the predicate holds. `5` and `7` are `< 10` (`20` is not), so with runtime `acc = n` the count is
           `n + 2`. Pins that an inferred closure with a BOOLEAN result applies via call_indirect and drives
           the HOF's branch, the result-type companion of the arithmetic-callback fold cases above.")
  (input
    (do
      (def
        (countif f acc xs)
        (match xs (#list() acc) (#list(h (.. t)) (countif f (if (f h) (+ acc 1) acc) t))))
      (def (main (: n Int64)) (countif (fn (x) (< x 10)) n #list(5 20 7)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (call main (: 100 Int64))
  (output (: 102 Int64))
  (live-objects known-leak))

; The cases above each instantiate a closure-taking recursive HOF at a SINGLE closure type. This one
; instantiates the SAME generic recursive HOF `fold-list` at TWO distinct closure types in one program:
; an Int64-element sum `(fn (x a) (+ a x))` (`f : (-> Int64 (-> Int64 Int64))`) AND a String-element
; byte-length fold `(fn (s a) (+ a (String.byte-len s)))` (`f : (-> String (-> Int64 Int64))`). The
; closure PARAMETER's type differs per instantiation, so `fold-list` monomorphizes into two functions
; with distinct machine signatures — the closure-carrying twin of the plain-value `loopn`-at-two-types
; case below. This is the shape adjacent to the still-open recursive-generic DRIVER tie (a closure param
; + accumulator threaded through recursion): the transformer/HOF form monomorphizes cleanly TODAY, so
; pin it so a future inference change to the driver-tie family cannot silently regress the working HOF
; case. Int64 fold: `5 + 7 + 30 = 42`; String fold: `2 + 4 + 1 = 7`; total `49`.
(case
  "a generic recursive HOF taking a closure is monomorphized at two distinct closure types"
  (doc
    "The same generic recursive `fold-list` is instantiated at TWO closure types in one program:
           an Int64-element sum closure (`f : (-> Int64 (-> Int64 Int64))`) and a String-element
           byte-length closure (`f : (-> String (-> Int64 Int64))`). The closure PARAMETER's type
           differs per call, so `fold-list` monomorphizes into two functions with distinct machine
           signatures — the closure-carrying twin of `loopn`-at-two-types. `5+7+30 = 42` and
           `2+4+1 = 7`, total `49`. Pins that a closure-taking recursive-generic HOF composes at two
           element types (guards the working HOF case against a regression from the open driver-tie
           family — a closure param + accumulator threaded through recursion).")
  (input
    (do
      (def
        (fold-list f acc xs)
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))
      (def
        (main)
        (+
          (fold-list (fn (x a) (+ a x)) 0 #list(5 7 30))
          (fold-list (fn (s a) (+ a (String.byte-len s))) 0 #list("ab" "abcd" "x"))))
      (export main)))
  (output (: 49 Int64))
  (live-objects known-leak))

; The count-past-two companion: the SAME closure-taking `fold-list` instantiated at THREE distinct closure
; types in one program — an Int64-element sum, a String-element byte-length fold, AND a Bool-element
; predicate count (`f : (-> Bool (-> Int64 Int64))`). Three closure PARAMETER types → three monomorphized
; functions with distinct machine signatures (i64 slot / i32 heap handle / i32 discriminant element). This
; is the closure-carrying twin of the `loopn`-at-three-machine-shapes case — extending the two-closure-type
; pin to confirm closure-taking recursive-generic monomorphization is not capped at two and that a
; discriminant-element instantiation coexists with the scalar + heap-handle ones. Int64 fold `5+7+30 = 42`;
; String fold `2+4+1 = 7`; Bool count (2 trues) `2`; total `51`.
(case
  "a generic recursive HOF taking a closure is monomorphized at three distinct closure types"
  (doc
    "The count-past-two companion of the two-closure-type case: the same generic recursive
           `fold-list` instantiated at THREE closure types in one program — an Int64-element sum, a
           String-element byte-length fold, and a Bool-element predicate count (`f : (-> Bool (-> Int64
           Int64))`). Three monomorphized functions with distinct machine signatures (i64 / i32 heap
           handle / i32 discriminant element). Confirms closure-taking recursive-generic monomorphization
           scales past two and a discriminant-element instantiation coexists with the scalar + heap ones —
           the closure twin of `loopn`-at-three-machine-shapes. `42 + 7 + 2 = 51`.")
  (input
    (do
      (def
        (fold-list f acc xs)
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))
      (def
        (main)
        (+
          (fold-list (fn (x a) (+ a x)) 0 #list(5 7 30))
          (+
            (fold-list (fn (s a) (+ a (String.byte-len s))) 0 #list("ab" "abcd" "x"))
            (fold-list (fn (b a) (if b (+ a 1) a)) 0 #list(true false true)))))
      (export main)))
  (output (: 51 Int64))
  (live-objects known-leak))

; A MULTI-PARAMETER runtime closure, applied at FULL arity. `core-semantics.md` §Functions Are
; Single-Arity says a multi-param `(fn (a b) …)` is curried sugar; when the whole function is applied to
; all its arguments at once through a recursive HOF, it lifts to one `(env, a, b) → result` function and
; applies via a single indirect call (no intermediate closure). `ap2 (fn (a b) (+ a b)) n` sums
; `(g i i)` for i = n…1, i.e. `2·(n + … + 1) = n·(n+1)`.
(case
  "a two-parameter closure is applied at full arity through a recursive HOF"
  (doc
    "`ap2` applies its two-argument function `g` to `(g i i)` at each recursion level and sums the
           results. `g = (fn (a b) (+ a b))` lifts to a two-parameter closure `(env, a, b) → result`
           applied at full arity; with n=3 the sum is (3+3)+(2+2)+(1+1) = 12. Pins that a multi-parameter
           lambda VALUE runs at run time when applied to all its arguments at once.")
  (input
    (do
      (def
        (ap2 (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
        (if (= n 0) 0 (+ (g n n) (ap2 g (- n 1)))))
      (def (main (: n Int64)) (ap2 (fn ((: a Int64) (: b Int64)) (+ a b)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 12 Int64))
  (live-objects known-leak))

; A THREE-parameter runtime closure at full arity — the multi-param lift generalizes past two params.
; `(fn (a b c) …)` lifts to `(env, a, b, c) → result` and applies via one `call_indirect` with all three
; arguments. `ap3 g n` sums `(g i i i) = 3·i` for i = n…1, so with n=3 the total is 3·(3+2+1) = 18.
(case
  "a three-parameter closure is applied at full arity through a recursive HOF"
  (doc
    "`ap3` applies its three-argument function `g` to `(g i i i)` at each recursion level and sums
           the results. `g = (fn (a b c) (+ (+ a b) c))` lifts to a three-parameter closure applied at
           full arity via one indirect call; with n=3 the sum is (3+3+3)+(2+2+2)+(1+1+1) = 18. Pins that
           the multi-parameter lift is not special-cased to two params.")
  (input
    (do
      (def
        (ap3 (: g (-> Int64 (-> Int64 (-> Int64 Int64)))) (: n Int64))
        (if (= n 0) 0 (+ (g n n n) (ap3 g (- n 1)))))
      (def (main (: n Int64)) (ap3 (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 18 Int64))
  (live-objects known-leak))

; CURRIED-SYNTAX application of a runtime multi-param closure. `core-semantics.md` §Functions Are
; Single-Arity: `(fn (a b) …)` is single-arity curried sugar, so `((g n) 1)` — apply `g` to `n`, then
; apply THAT to `1` — is the SAME full-arity application as `(g n 1)`, only written with nested parens.
; When `g` is a RUNTIME fn value (a recursive HOF's parameter), the two-paren spine must flatten to one
; `call_indirect` on `g` with both arguments — NOT decline as an unbuilt intermediate closure. This is
; "runtime currying reaches full arity": the application SPINE is peeled and its arguments gathered
; left-to-right, so a curried call site behaves identically to the flat one. (A partial that never
; reaches full arity would still need a heap partial-closure cell; here every use completes the arity.)
(case
  "a curried-syntax application of a runtime closure flattens to one full-arity indirect call"
  (doc
    "`((g n) 1)` where `g` is the recursive `ap`'s runtime two-parameter fn parameter — the curried
           spelling of `(g n 1)`. The nested application spine flattens so `g` is applied to both `n` and
           `1` in ONE indirect call; with `g = (fn (a b) (+ a b))` and n=3 the sum is (3+1)+(2+1)+(1+1) =
           9. Pins that a curried call site of a runtime closure reaches full arity via one call_indirect,
           identical to the flat form — it does not decline as an unbuilt intermediate closure.")
  (input
    (do
      (def
        (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64))
        (if (= n 0) 0 (+ ((g n) 1) (ap g (- n 1)))))
      (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

; A CURRIED-SYNTAX application of a multi-payload VARIANT CONSTRUCTOR — the constructor analogue of the
; curried-closure case above. `(T.Mk (Int64 Int64))` is a two-payload ctor; `((T.Mk n) 2)` applies it to
; `n` then `2` (the curried spelling of `(T.Mk n 2)`). The nested application spine flattens so the
; constructor reaches FULL arity in one construction and builds the `T` value, which the surrounding
; `match` then destructures — `(T.Mk a b) → (+ a b)`. With a runtime `n` the construction is not folded.
; Pins that a curried constructor application reaches full arity and constructs, identical to the flat
; `(T.Mk n 2)` — the multi-payload ctor twin of the curried runtime-closure flattening.
(case
  "a curried-syntax application of a multi-payload constructor reaches full arity and constructs"
  (doc
    "`((T.Mk n) 2)` — the two-payload constructor `T.Mk` applied to `n` then `2`, the curried spelling
           of `(T.Mk n 2)`. The application spine flattens so the ctor reaches full arity in one
           construction, building `(T.Mk n 2)`, which the `match` destructures to `(+ n 2)`. With runtime
           `n` nothing folds: n=5 → 7, n=40 → 42. Pins the curried constructor-application spine (the
           multi-payload ctor analogue of the curried runtime-closure case above).")
  (input
    (do
      (type T (Mk Int64 Int64))
      (def (main (: n Int64)) (match ((T.Mk n) 2) ((T.Mk a b) (+ a b))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

; A PARTIAL APPLICATION that escapes short of full arity, then runs as a runtime closure. Here `g` is
; `main`'s statically-known two-parameter lambda, so `(g n)` — applied to ONE arg — PARTIALLY APPLIES at
; compile time (`core-semantics.md` §Functions Are Single-Arity: applying a curried function to fewer args
; returns a closure awaiting the rest) into a residual `(fn (b) (+ 5 b))`. That residual then escapes as a
; VALUE passed to the recursive `sumapply`, which cannot inline it — so it survives as a genuine runtime
; closure applied via `call_indirect` at each step. The partial-application fold + the runtime-closure lift
; compose: `sumapply (partial) 2 = (5+2)+(5+1) = 13`. (Pins the fix that made a partially-applied residual's
; parameter annotation survive the β-copy that carries it into the recursive callee — before it, the
; residual's awaited parameter lost its declared type and the closure declined.)
(case
  "a partially-applied function escapes as a value and runs through a recursive HOF"
  (doc
    "`(g n)` where `g` is `main`'s two-parameter lambda applied to ONE arg partially applies to the
           residual `(fn (b) (+ 5 b))`, which escapes as a value into the recursive `sumapply` (applied at
           2 and 1) and runs as a runtime closure via call_indirect. `sumapply (g 5) 2 = (5+2)+(5+1) = 13`.
           Pins that a partial application escaping short of full arity survives as a runtime closure when
           it crosses into a recursive HOF.")
  (input
    (do
      (def
        (sumapply (: h (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (h n) (sumapply h (- n 1)))))
      (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) (sumapply (g n) 2))
      (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 13 Int64))
  (live-objects known-leak))

; The NAMED-DEF twin: a partial application of a top-level def `(add 5)` (rather than a lambda) passed to a
; recursive HOF. `add`'s parameters are annotated, and the residual `(fn (b) (+ 5 b))` must keep those
; declared types across the beta-copy that carries it into the recursive `apply-sum` — a residual that lost
; its annotated param type declined. `apply-sum (add 5) 3` = (5+3)+(5+2)+(5+1) = 21.
(case
  "a partial application of a named def keeps its annotated param type across the recursive-HOF beta copy"
  (doc
    "`(add 5)` partially applies the top-level `add` to its first arg; the residual `(fn (b) (+ 5 b))`
           escapes into the recursive `apply-sum` and runs as a runtime closure, its second parameter's
           declared Int64 type preserved through the beta-copy into the callee. apply-sum (add 5) 3 = 21.")
  (input
    (do
      (def (add (: a Int64) (: b Int64)) (+ a b))
      (def
        (apply-sum (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1)))))
      (def (main (: n Int64)) (apply-sum (add 5) n))
      (export main)))
  (call main (: 3 Int64))
  (output (: 21 Int64))
  (live-objects known-leak))

; The complement of the escaping-partial case above: the partial is over a RUNTIME closure (boxed in a sum,
; extracted by a match), NOT a statically-known lambda, and a `let` breaks the curried-spine flatten —
; `(match p ((Box.C f) (let ((g (f 3))) (g 4))))`. That gathers 1 of the closure's 2 curried args and now
; builds a RESIDUAL CLOSURE (eta-abstract the missing param into `(fn (b) (f 3 b))`, whose lift captures `f`
; and `3`), so `(g 4)` completes it — `f(3)(4) = 7`, uniform across backends. (Previously this declined: an
; under-arity `CallClosure` mis-emitted an invalid module; the residual-closure lift synthesizes a FULL
; application, so the reps agree.) The DIRECT full-arity `((f 3) 4)` = 7 flattens to one call, still covered.
(case
  "a let-bound partial application of a runtime closure builds a residual closure and completes"
  (doc
    "A closure boxed in a sum and applied at partial arity through a `let` that breaks the spine
           flatten gathers 1 of 2 curried args: `(f 3)` is a 1-of-2 partial of the runtime closure `f`,
           bound to `g`, then completed by `(g 4)`. The partial now EMITS a RESIDUAL CLOSURE — it
           eta-abstracts the missing param into a synthesized `(fn (b) (f 3 b))` whose lift captures `f`
           and `3` into the residual closure's env, so `(g 4)` runs `f(3)(4) = 3 + 4 = 7`. (Previously this
           declined — an under-arity `CallClosure` mis-emitted an invalid module; the residual-closure lift
           builds a FULL application in the synthesized body, so the machine reps agree, uniform across
           backends.) The DIRECT full-arity `((f 3) 4)` = 7 flattens the same.")
  (input
    (do
      (type Box (C (-> Int64 (-> Int64 Int64))))
      (def (mk) (Box.C (fn ((: a Int64)) (fn ((: b Int64)) (+ a b)))))
      (def (main) (let ((p (mk))) (match p ((Box.C f) (let ((g (f 3))) (g 4))))))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a user-written MAP combinator builds a transformed list through its fn parameter"
  (doc
    "The list-BUILDING HOF (the fold-list pins reduce to a scalar): `map-l` pushes `(f h)` per
           element, the closure capturing the boundary `k` — element 2 of the result is 3·14 = 42. The
           map half of the user HOF library; the combinator's output list must hold the closure's
           per-element results in order.")
  (input
    (do
      (def
        (map-l (: f (-> Int64 Int64)) (: xs (List Int64)) (: acc (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (map-l f t (List.push acc (f h))))))
      (def
        (main (: k Int64))
        (match
          (List.at (map-l (fn ((: v Int64)) (* v k)) #list(1 2 3) #list()) 2)
          ((Some v) v)
          ((None u) -1)))
      (export main)))
  (call main (: 14 Int64))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a user-written FILTER combinator keeps elements passing a captured predicate"
  (doc
    "The Bool-returning fn param: `filter-l` keeps `h` when `(p h)` — the predicate closure
           captures the runtime cutoff, selecting 3 of 4 at cut=10 and 1 at cut=30. The filter half;
           a combinator inverting the predicate (or evaluating it once) drifts a length.")
  (input
    (do
      (def
        (filter-l (: p (-> Int64 Bool)) (: xs (List Int64)) (: acc (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (filter-l p t (if (p h) (List.push acc h) acc)))))
      (def
        (main (: cut Int64))
        (List.len (filter-l (fn ((: v Int64)) (> v cut)) #list(5 15 25 35) #list())))
      (export main)))
  (call main (: 10 Int64))
  (output (: 3 Int64))
  (call main (: 30 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "map THEN filter compose — one combinator's output list is the next's input"
  (doc
    "The pipeline: `filter-l (>10) (map-l (·k) [1,2,3,4])` — the map's freshly-built list feeds
           the filter, each with its own captured closure. k=5 maps to [5,10,15,20], filters to len 2;
           k=2 maps to [2,4,6,8], filters to len 0 (the all-rejected empty). One compiled pipeline, two
           selectivities; the collection-pipeline shape every list-processing program takes.")
  (input
    (do
      (def
        (map-l (: f (-> Int64 Int64)) (: xs (List Int64)) (: acc (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (map-l f t (List.push acc (f h))))))
      (def
        (filter-l (: p (-> Int64 Bool)) (: xs (List Int64)) (: acc (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (filter-l p t (if (p h) (List.push acc h) acc)))))
      (def
        (main (: k Int64))
        (List.len
          (filter-l
            (fn ((: v Int64)) (> v 10))
            (map-l (fn ((: v Int64)) (* v k)) #list(1 2 3 4) #list())
            #list())))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (call main (: 2 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a middle curry STAGE is reused — one s1 residual yields an s2 applied twice"
  (doc
    "Three-stage curry with STAGE REUSE: `s1 = (add3 x)` (captures the boundary x), `s2 = (s1 20)`
           (captures x AND 20), then s2 applies TWICE with different finals — (1+20+300) + (1+20+400) =
           742. Each stage's environment layers over the previous (x, then x+20); reusing s2 must re-read
           BOTH captured layers per application, not consume them on first apply (a one-shot environment
           or a stage that re-captured from the outer scope on second use drifts the sum).")
  (input
    (do
      (def (add3 (: a Int64)) (fn ((: b Int64)) (fn ((: c Int64)) (+ a (+ b c)))))
      (def (main (: x Int64)) (let ((s1 (add3 x))) (let ((s2 (s1 20))) (+ (s2 300) (s2 400)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 742 Int64)))

(case
  "two residuals from ONE curry source hold distinct first-stage captures"
  (doc
    "The sibling-residual face: `(mul2 x)` and `(mul2 (* x 10))` build two residuals from one
           curried def with DIFFERENT first args (3 and 30); each applied to 2 must use ITS capture —
           6 + 60 = 66. A closure cache keyed on the def (rather than the applied argument) would alias
           the two environments.")
  (input
    (do
      (def (mul2 (: a Int64)) (fn ((: b Int64)) (* a b)))
      (def (main (: x Int64)) (let ((f (mul2 x))) (let ((g (mul2 (* x 10)))) (+ (f 2) (g 2)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 66 Int64)))

; A closure RETURNED from a RECURSIVE function, then applied through a recursive HOF — two runtime
; function paths composed. `core-semantics.md` §A Function Is A First-Class Value lists both "returned
; as a result" and "passed as an argument"; here they meet at run time. Because `pick` is RECURSIVE it
; cannot be inlined away, so the closure it returns is a genuine runtime value (not folded at the call
; site the way a non-recursive factory folds), and it then crosses into the recursive `applyer` and
; dispatches via `call_indirect` at each step. Pins that a lifted closure produced by one runtime
; function survives being handed to another and applied indirectly.
(case
  "a closure returned from a recursive function is applied through a recursive HOF"
  (doc
    "`pick` recurses to its base case and returns the closure `(fn (x) (+ x 1))`; because `pick`
           recurses it cannot fold, so its returned closure is a real runtime value. That value is passed
           to the recursive `applyer` and applied at each step via an indirect call. `pick n` always
           reaches `(+ x 1)`, so `applyer (pick n) 3 = (3+1)+(2+1)+(1+1) = 9` regardless of the runtime
           `n` fed to pick. Pins that a returned-then-passed runtime closure dispatches correctly.")
  (input
    (do
      (def (pick (: n Int64)) (if (= n 0) (fn ((: x Int64)) (+ x 1)) (pick (- n 1))))
      (def
        (applyer (: g (-> Int64 Int64)) (: n Int64))
        (if (= n 0) 0 (+ (g n) (applyer g (- n 1)))))
      (def (main (: n Int64)) (applyer (pick n) 3))
      (export main)))
  (call main (: 5 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

; The DIRECT-APPLICATION-HEAD twin of the case above: a RECURSIVE def whose result is a closure, applied
; directly as the head `((selfp 2) 5)` (not passed to a HOF). `selfp` recurses to its base case returning
; `(fn (x) (+ x 100))`; the head `(selfp 2)` is a recursive Core::Call returning a closure that cannot
; beta-reduce (the reduction hits the depth guard), so its result is a runtime closure HANDLE applied via
; call_indirect. Previously declined "value is not applyable" (an application head that was a non-reducible
; recursive Call of Ty::Fn result was not recognized as a runtime fn value).
(case
  "a recursive def returning a closure is applied directly as the call head"
  (doc
    "`(selfp n)` recurses to `(selfp 0)` = `(fn (x) (+ x 100))`, and `((selfp 2) 5)` applies that
           returned closure directly = 105. The recursive Call head cannot fold, so it is a runtime fn
           value applied indirectly — the direct-head twin of the returned-then-passed-to-a-HOF case.")
  (input
    (do
      (def (selfp (: n Int64)) (if (= n 0) (fn ((: x Int64)) (+ x 100)) (selfp (- n 1))))
      (def (main) ((selfp 2) 5))
      (export main)))
  (call main)
  (output (: 105 Int64)))

; core-semantics.md §A Function Is A First-Class Value: a function can be "stored in a data structure."
; A tuple and a list are data structures exactly as a record is, so a function stored in a tuple
; element (or list element) must be extractable and callable, exactly as one stored in a record field
; is. The compiler resolves a function through record member access `.` (the control below runs); the
; same projection-to-lambda resolution must extend to the positional/indexed accessors `(. x N)` and
; `List.at`. A generation that does not yet resolve a stored lambda through those accessors declines
; rather than running the program (reject-don't-miscompile).
(case
  "a function stored in a tuple element is called after extraction"
  (doc
    "A function is a first-class value storable in any data structure. `(tuple (fn (x) (+ x 1))
           9)` stores a function as element 0; `(. … 0)` extracts it and applying it to 5 yields 6.
           This must behave exactly as the record-field companion below — a tuple is a data structure
           like a record. A generation that does not yet resolve the stored lambda through `(. x N)`
           the way it does through `.` declines rather than running the program.")
  (input ((. #tuple((fn (x) (+ x 1)) 9) 0) 5))
  (output (: 6 Int64)))

(case
  "a function stored in a record field is called after extraction"
  (doc
    "The control the case above must match: `(record (f (fn (x) (+ x 1))))` stores a function in
           field `f`; `(. … f)` extracts it and applying it to 5 yields 6. The seed runs this — a
           function stored in a record is resolved and called. The tuple case must behave identically.")
  (input ((. #record((= f (fn (x) (+ x 1)))) f) 5))
  (output (: 6 Int64)))

(case
  "a field is projected from a record returned by a function"
  (doc
    "Witnesses core-semantics.md §A Function Is A First-Class Value + #Member Access Projects A
           Record Field: a function may return a record, and its caller projects a field from the
           result. `((fn (x) (record (v x))) 7)` builds the record {v: 7}; projecting `v` yields 7.
           Accessing a field inside the lambda body already works, and accessing a directly-written or
           let-bound record works — projecting the record a lambda RETURNS must behave the same, not
           trap. This is the record-builder idiom a compiler uses constantly.")
  (input (. ((fn (x) #record((= v x))) 7) v))
  (output (: 7 Int64)))

(case
  "an element is projected from a tuple returned by a function"
  (doc
    "The tuple companion: `((fn (x) (tuple x 9)) 7)` returns the pair (7, 9); projecting element 0
           yields 7. A positional access on a function's tuple result must project it, not trap.")
  (input (. ((fn (x) #tuple(x 9)) 7) 0))
  (output (: 7 Int64)))

(case
  "a field is projected from a record returned by a let-bound function"
  (doc
    "The same record-builder reached through a named binding: `mk` is a lambda returning a
           record; `(mk 7)` builds {v: 7} and `(. (mk 7) v)` projects 7. Binding the builder to a name
           does not change that its result is an accessible record.")
  (input (let ((mk (fn (x) #record((= v x))))) (. (mk 7) v)))
  (output (: 7 Int64)))

; A NULLARY function returning a SCALAR is callable: `(def (g) <scalar>)` defines a zero-argument
; function, and `(g)` — a zero-argument application — invokes it, yielding the scalar. Applying a value
; to no arguments is the identity, so a nullary call reduces to the function's body (a bare reference
; `g`, with no call, denotes that same body value). These pin the scalar case the compound-projection
; cases below build on — a nullary call must be recognized as a CALL, not misread as applying the
; body value to zero arguments.
(case
  "a nullary function returning a scalar is callable"
  (doc
    "`(def (mk) 42)` is a nullary function; `(mk)` calls it and yields the scalar 42. A
           zero-argument application invokes the function — it is not an attempt to apply the body
           value 42 to no arguments. A bare reference `mk` (no call) denotes the same value, so `(mk)`
           and `mk` agree; the parenthesized form is the call.")
  (input (do (def (mk) 42) (def (main) (mk)) (export main)))
  (output (: 42 Int64)))

(case
  "a nullary helper called and used in arithmetic"
  (doc
    "`(def (g) 7)` and `(def (main) (+ (g) 5))`: the nullary `g` is called and its result 7
           added to 5, yielding 12. A nullary call composes in an ordinary expression like any other
           call — its result is a plain value the enclosing operation consumes.")
  (input (do (def (g) 7) (def (main) (+ (g) 5)) (export main)))
  (output (: 12 Int64)))

(case
  "a nullary function called from another function's body"
  (doc
    "`(def (g) 7)`, `(def (f x) (+ x (g)))`, `(def (main) (f 5))`: `f` calls the nullary `g` in
           its body; `(f 5)` = 5 + 7 = 12. A nullary call works inside a non-entry function body, not
           only at the top level — the callee is reached and reduced wherever the call appears.")
  (input (do (def (g) 7) (def (f x) (+ x (g))) (def (main) (f 5)) (export main)))
  (output (: 12 Int64)))

(case
  "a nullary lambda applied yields its body"
  (doc
    "`((fn () 7))` = 7 — a zero-parameter lambda applied to no arguments β-reduces to its body, the
           lambda face of the nullary-def call above. Applying to no arguments is the identity, so the
           result is the body value, not an attempt to apply 7 to zero arguments.")
  (input ((fn () 7)))
  (output (: 7 Int64)))

(case
  "a bare reference to a nullary function denotes its body value"
  (doc
    "`(def (g) 7)` then `(def (main) g)` — a bare reference to the nullary `g` (no call parens)
           denotes its body value 7, agreeing with the call form `(g)`. Pins that `g` and `(g)` are the
           same value for a nullary def: the reference IS the body, and the parens are the (identity) call.")
  (input (do (def (g) 7) (def (main) g) (export main)))
  (output (: 7 Int64)))

; A NULLARY function that returns a compound value must be projectable exactly as a unary one is.
; The cases above return a structure from a function of one parameter; a nullary function `(def (mk)
; <compound>)` called as `(mk)` returns the same kind of value, and projecting a field/element from
; it must yield the value, not trap. The seed projects a UNARY function's structure result correctly
; (above) but TRAPS on a NULLARY function's structure result — a nullary call `(mk)` is not reduced
; to its body for projection the way a unary call `(mk arg)` is, so the access finds no compile-time
; structure and traps at run time. (A nullary function returning a SCALAR works — `(mk)` → 42; only
; a projected compound result traps.)
(case
  "an element is projected from a tuple returned by a nullary function"
  (doc
    "`mk` is a nullary function returning the pair (7, 9); `(mk)` calls it and `(. (mk) 1)`
           projects element 1, yielding 9. A positional access on a nullary function's tuple result
           must project it, exactly as it does for a unary function's result (above) — not trap. The
           seed traps: it does not reduce the nullary call `(mk)` to its tuple body for the access.")
  (input (do (def (mk) #tuple(7 9)) (def (main) (. (mk) 1)) (export main)))
  (output (: 9 Int64)))

(case
  "a field is projected from a record returned by a nullary function"
  (doc
    "The record companion: `mk` is a nullary function returning {a: 5}; `(. (mk) a)` projects
           the field, yielding 5. Projecting a field of a nullary function's record result must behave
           like projecting a unary function's record result (above), not trap. The seed traps on the
           nullary case.")
  (input (do (def (mk) #record((= a 5))) (def (main) (. (mk) a)) (export main)))
  (output (: 5 Int64)))

(case
  "applying a non-function is a type error"
  (doc
    "Witnesses core-semantics.md §Applying A Function Binds Its Parameter To Its Argument:
           applying a value that is not a function has no defined result. The callee's type is not a
           function type, so the compiler MUST reject it at compile time (CDZ0201) rather than emit a
           component. With curried functions, partial application is natural (returns a closure), so
           the error case is applying a non-function like an integer.")
  (input (5 3))
  (error CDZ0201))

(case
  "applying a boolean is a type error"
  (doc
    "Companion of the case above for another non-function scalar: a Bool is not a function, so
           applying it (`(true 1)`) is a type error the compiler MUST reject (CDZ0201).")
  (input (true 1))
  (error CDZ0201))

(case
  "applying a float is a type error"
  (doc
    "Companion for a Float callee: `(3.5 1)` applies a non-function, a type error the compiler
           MUST reject (CDZ0201).")
  (input (3.5 1))
  (error CDZ0201))

; APPLYING A NULLARY FUNCTION to arguments — a nullary def `(def (g) …)` resolves its name straight to its
; body value, so `(g 5)` is genuinely applying a non-function; but the author WROTE `g` with a `()` signature
; and CALLED it, so the terse "cannot apply a value of type Int64" hides both the name and the cause. The
; message names `g`, says it takes no arguments, and spells the fix `(g)` — the nullary companion of the
; over-application naming. A plain VALUE def `(def v 5)` (no callable signature) keeps the type-named message.
; (Migrated from rcdzc applying_a_nullary_function_says_it_takes_no_arguments.)
(case
  "applying a nullary function names it and says it takes no arguments"
  (doc
    "`(g 5)` where `g` is a nullary FUNCTION `(def (g) 5)`: rejected CDZ0201 with a message naming `g`
           and stating it takes no arguments (spelling the call-as-`(g)` fix), rather than the opaque
           value-type message a bare value def would get.")
  (input (do (def (g) 5) (def (main) (g 5)) (export main)))
  (error CDZ0201 (message "takes no arguments")))

(case
  "applying a nullary function with two surplus arguments pluralizes the count"
  (doc
    "`(g 5 6)` on the same nullary `g`: the surplus-argument count is pluralized — 'but 2 were applied'
           — so the message counts the actual arguments, not a fixed singular.")
  (input (do (def (g) 5) (def (main) (g 5 6)) (export main)))
  (error CDZ0201 (message "but 2 were applied")))

(case
  "applying a plain value def keeps the type-named message, not the nullary-function wording"
  (doc
    "The contrast: `(v 5)` where `v` is a plain VALUE def `(def v 5)` — its name resolves to the same
           value, but it was NOT written as a callable, so the useful fact is its VALUE type: the message is
           'cannot apply a value of type Int64', NOT the nullary-function 'takes no arguments' wording (which
           is reserved for a name written with a `()` signature).")
  (input (do (def v 5) (def (main) (v 5)) (export main)))
  (error CDZ0201 (message "cannot apply a value of type Int64")))

; --- Over-applying a single-arity constructor is applying a non-function -----------------
; core-semantics.md #A Sum Type Constructor Is A Single-Arity Function (applied to EXACTLY ONE
; argument) together with #Functions Are Single-Arity (`(f a b)` desugars to `((f a) b)`): a
; constructor takes one argument, so `(Some 1 2)` desugars to `((Some 1) 2)` — applying the Sum
; value `(Some 1)`, which is NOT a function, to `2`. That is the apply-a-non-function error above,
; so the compiler MUST reject it (CDZ0201), exactly as `((Some 1) 2)` written explicitly is rejected.
; An over-applied constructor is arity-checked the same way an over-applied user function is (`(f 5
; 99)` on a unary `f`), so the ill-formed application never slips through with a wrong (truncated)
; value; a generation that does not yet check it declines rather than running the program.
(case
  "over-applying a constructor is a type error, not a silent argument drop"
  (doc
    "`(Some 1 2)` desugars to `((Some 1) 2)`: the constructor `Some` is single-arity, so
           `(Some 1)` is a complete Sum value, and applying it to `2` applies a non-function — a type
           error (CDZ0203), the same as `(5 3)` above. The compiler MUST reject it rather than drop
           the `2` and yield `(Some 1)`, which would silently accept the ill-formed application. Carries a
           DELETE fix on the surplus argument (heuristic/unverified — which callee the author meant is a
           guess). Fix-quality migrated from rcdzc over_application_offers_a_delete_the_extra_argument_fix.")
  (input (Some 1 2))
  (error CDZ0203 (fix (kind delete) (unverified)) (exact-code)))

(case
  "over-applying a constructor by several arguments is a type error"
  (doc
    "The same shape with more extra arguments: `(Some 1 2 3)` desugars to `(((Some 1) 2) 3)`,
           applying the Sum value `(Some 1)` to `2` (already a non-function application). The compiler
           MUST reject it (CDZ0203). Pins that the arity check is on the constructor's single-argument
           application, not forgiving of any number of trailing arguments.")
  (input (Some 1 2 3))
  (error CDZ0203 (exact-code)))

; The LOW-arity mirror: UNDER-applying a unary constructor. A payload-carrying variant produces its value
; only when applied to its argument (§A Sum Type Constructor Is A Single-Arity Function), so `(Some)` — the
; constructor applied to ZERO arguments — is CDZ0201, NOT a decline (a fabricated `(Some unit)` would slip a
; value the program never wrote past the payload check). The message names the constructor + how to apply it;
; a generic payload omits the "it carries X" clause (it would read `_`), a concrete-payload ctor names its
; type. Migrated from rcdzc under_applying_a_unary_variant_constructor_is_a_type_error.
(case
  "under-applying a unary constructor (Some) is a type error, not a fabricated unit payload"
  (input (do (def (main) (Some)) (export main)))
  (error CDZ0201 (message "`Some` needs its payload argument") (message "`(Some <value>)`")))

(case
  "under-applying a concrete-payload constructor names its payload type"
  (input (do (type T (Wrap Int64)) (def (main) (T.Wrap)) (export main)))
  (error CDZ0201 (message "`Wrap` needs its payload argument") (message "it carries an Int64")))

(case
  "a NULLARY variant applied to nothing (None) is not under-applied — it constructs its value"
  (input (do (def (main) (match (None) ((Some x) x) ((None _) 0))) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a correctly-applied unary constructor (Some 5) compiles and matches (the control)"
  (input (do (def (main) (match (Some 5) ((Some x) x) ((None _) 0))) (export main)))
  (call main)
  (output (: 5 Int64)))

; Over-applying a USER FUNCTION is arity-checked the SAME way — the case the comment above references
; ("an over-applied constructor is arity-checked the same way an over-applied user function is"). A
; lambda / named def of arity N applied to more than N arguments applies the fully-consumed result
; (which is NOT a function) to the surplus — a type error (CDZ0203), never a silent argument drop.
; `((fn (x) (+ x 1)) 5 9)` desugars to `(((fn (x) (+ x 1)) 5) 9)`: `(fn (x)…) 5` = 6 (an Int64, not a
; function), applied to `9` — the apply-a-non-function error. This pins the over-applied-function half
; that the constructor cases above pin for constructors.
(case
  "over-applying a lambda by an extra argument is a type error"
  (doc
    "`((fn (x) (+ x 1)) 5 9)` — a unary lambda applied to two arguments. Desugars to `(((fn (x)
           (+ x 1)) 5) 9)`: the inner application yields the Int64 6, and applying 6 to 9 applies a
           non-function → CDZ0203. The compiler MUST reject it, not drop the 9 and yield 6.")
  (input (do (def (main) ((fn ((: x Int64)) (+ x 1)) 5 9)) (export main)))
  (error CDZ0203 (exact-code)))

(case
  "over-applying a named function by an extra argument is a type error"
  (doc
    "The named-def companion: `(def (f x) (+ x 1))`, `(f 5 9)` applies the unary `f` to two args.
           By §Functions Are Single-Arity this desugars to `((f 5) 9)` — `(f 5)` = 6, applied to 9 is a
           non-function application → CDZ0203. Arity is checked for a named function exactly as for a
           lambda or a constructor. Carries a DELETE fix on the surplus argument (heuristic/unverified).
           Fix-quality migrated from rcdzc over_application_offers_a_delete_the_extra_argument_fix.
           Also pins the DEDUP (migrated from rcdzc over_applying_a_function_reports_one_error_not_a_shadowing_decline):
           over-application is EXACTLY ONE error — the coded CDZ0203 — not that reject PLUS the evaluator's
           uncoded 'applied more arguments than the function accepts' decline for the same node, which
           dedup_faults drops when the coded reject is present. Hence (count 1).")
  (input (do (def (f (: x Int64)) (+ x 1)) (def (main) (f 5 9)) (export main)))
  (error CDZ0203 (fix (kind delete) (unverified)) (count 1) (exact-code)))

; The arity check has a lower end too: a UNARY variant applied to ZERO arguments is under-applied. A
; sum type constructor is a single-arity function that produces the tagged variant "when applied to
; EXACTLY ONE argument" (core-semantics.md #A Sum Type Constructor Is A Single-Arity Function). `Some`
; is unary (Option's non-nullary variant, argument type the payload T), so `(Some)` supplies no
; argument — the mirror of the over-application above. A compiler that fabricates a Unit payload for a
; missing argument produces `(Some unit)` — a value of type `Option Unit` the program never wrote,
; observable by matching `(Some x)` binding x=unit, and one that slips past the payload-annotation check
; (`(: (Some) (Option Int64))` yields `(Some unit)` where `(: (Some unit) (Option Int64))` is correctly
; rejected — a Unit payload under an `Int64` annotation). The Unit filler is right only for a NULLARY
; variant, whose argument type IS Unit; a unary variant applied to zero arguments MUST be rejected
; (CDZ0203), exactly as over-application is. A generation that does not yet check the low end declines
; rather than fabricating the payload (reject-don't-miscompile).
(case
  "under-applying a unary constructor is a type error, not a fabricated unit payload"
  (doc
    "`(Some)` applies the unary constructor `Some` to zero arguments — under-application, the
           mirror of `(Some 1 2)` over-application. `Some` produces its Sum value only when applied to
           exactly one argument (core-semantics.md #A Sum Type Constructor Is A Single-Arity Function),
           so `(Some)` MUST be rejected (CDZ0201). A compiler that fabricates a Unit payload yields
           `(Some unit)` — a value of type `Option Unit` the program never wrote, observable by matching
           `(Some x)` and slipping past the payload-annotation check. The Unit filler is correct only for
           a NULLARY variant (argument type Unit); a unary variant demands its one argument. A generation
           that does not yet check the low arity end declines rather than fabricating the payload.")
  (input (Some))
  (error CDZ0201))

; --- A function value is not matchable -----------------------------------------------------
; A `match` deconstructs a DATA value by its cases (core-semantics.md §Patterns Compose — a literal, a
; tuple, or a constructor). A FUNCTION value has no cases to deconstruct, so `(match g …)` where `g` is a
; function/closure is a type error (CDZ0203), not a runtime match — the compiler names the real cause and
; points at the fix (call it, or match on the value it RETURNS). This is the match-position companion of
; the apply-a-non-function errors above: there a non-function was applied, here a function is matched.
; The reject is up-front on a `Ty::Fn` scrutinee, so it is a coded diagnostic, not an internal-sounding
; closure-boundary decline about a machine representation the author never asked about.
(case
  "matching on a function value is a type error"
  (doc
    "`(match g (v v))` where `g` is a function (a def) — a match deconstructs a data value by its
           cases (a literal, tuple, or constructor), and a function has none, so it is rejected (CDZ0203).
           The author who meant to match the function's RESULT must call it first `(match (g x) …)`. Pins
           that a `Ty::Fn` scrutinee is a coded type error, not an internal decline about a closure's
           parameter representation.")
  (input (do (def (g (: x Int64)) (+ x 1)) (def (main) (match g (v v))) (export main)))
  (error CDZ0203 (message "function value cannot be matched") (exact-code)))

(case
  "matching on a partial application is a type error"
  (doc
    "`(match (add 1) (v 0))` where `add` is binary — `(add 1)` is a PARTIAL application, still a
           function value (awaiting its second argument), so it is not matchable either (CDZ0203). Pins
           that the not-matchable check covers a partial application, not only a bare def name — any
           `Ty::Fn` scrutinee, however produced, is rejected.")
  (input
    (do
      (def (add (: a Int64) (: b Int64)) (+ a b))
      (def (main) (match (add 1) (v 0)))
      (export main)))
  (error CDZ0203 (message "function value cannot be matched") (exact-code)))

(case
  "a recursive def computes over its argument"
  (doc
    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments:
           sum-to counts down to 0 through direct self-recursion. sum-to(3) = 3 + 2 + 1 + 0 = 6.")
  (input
    (do
      (def (sum-to n) (if (= n 0) 0 (+ n (sum-to (+ n -1)))))
      (def (main) (sum-to 3))
      (export main)))
  (output (: 6 Int64)))

; ROBUSTNESS: a compiler must DECLINE (or complete), never ABORT, on any well-formed input
; (self-hosting-and-bootstrap.md §An Unsupported Construct Is Declined, Not Miscompiled). Two shapes
; that a naive recursive-descent compiler crashes on — an unproductive compile-time recursion, and a
; deeply nested expression — must instead stop at a recursion/resource bound and decline. A generation
; that cannot reduce such input declines; it does not overflow its own stack.
(case
  "an unproductive self-recursion is declined, not a compiler crash"
  (doc
    "`(def (f) (f))` — a nullary self-call with no base case — cannot be reduced to a value: the
           compile-time evaluator would inline it without end. The compiler must DECLINE it (a
           recursive function it cannot specialize), exactly as an unproductive PARAMETERIZED recursion
           declines, and MUST NOT abort with a native stack overflow. A generation that does not realize
           runtime specialization of such a function declines; the point of the case is 'never crash'.")
  (input (do (def (f) (f)) (def (main) (f)) (export main)))
  (error CDZ0999))

(case
  "an unproductive PARAMETERIZED self-recursion is a coded CDZ0204 — no base case, result never concrete"
  (doc
    "`(def (f (: n Int64)) (f n))` — a parameterized self-call whose EVERY path recurses with no base
           case — never returns a concrete value, so its result is undeterminable. A PERMANENT correct-reject
           of a user bug (the fix is 'add a base case'), coded CDZ0204 NonProductiveRecursion. DISTINCT from
           the NULLARY `(def (f) (f))` case above (CDZ0999 RecursionBound — a compile-time inline-to-reduction-
           bound decline): here the fault is a never-productive result the type/value analysis cannot resolve,
           not a reduction-budget exhaustion. Also DISTINCT from CDZ0203 TypeMismatch — no two types disagree;
           the single result type is simply undeterminable (nothing to unify against). The pair pins the
           decline-vs-reject + code-per-invariant taxonomy: both refuse cleanly, with the specific code naming
           exactly why (reduction bound vs non-productive recursion).")
  (input (do (def (f (: n Int64)) (f n)) (def (main) (f 0)) (export main)))
  (error CDZ0204))

(case
  "a self-applying term is declined at the reduction budget, not hung on"
  (doc
    "`((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))` — a self-application whose argument applies itself
           — has NO normal form: each β-reduction produces a larger term. It is NOT statically recursive
           (the lambdas call a PARAMETER, not a named def, so the call-graph recursion check finds no
           cycle) and each reduction stays within the depth limit, so the depth guard alone does not stop
           it — the term roughly DOUBLES each step and the compiler's reduction/type walk would attempt an
           exponential number of reductions and appear to HANG. The evaluator bounds its TOTAL reduction
           work (`enter_reduction` counts attempts against a budget): past it the reduction DECLINES (a
           resource-limit rejection), so a non-normalizing term is a clean decline in a fraction of a
           second, never a compiler hang. The point of the case is 'never hang' — a compiler completes or
           declines on any input.")
  (input (do (def (main) ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))) (export main)))
  (error CDZ0999))

(case
  "an if-wrapped self-application is rejected in bounded time, not an inference hang"
  (doc
    "`(fn v (if (v v) 1 (v v)))` applied to a copy of itself has no normal form: the self-app in the
           if CONDITION forces β-reduction, which reduces the branch's self-app, and applied to itself the
           term grows exponentially. The plain self-app declines at the reduction budget (above), but this
           if-wrapped variant HUNG type INFERENCE through a DIFFERENT path — the lambda-parameter context
           recovery (`expected_arrow_for_lambda` → `type_of` → …) re-derives the growing term's types
           without going through the β-reduction budget, so it stayed within the descent-depth limit while
           attempting an exponential number of context lookups. Charging that recovery against the SAME
           cumulative work budget makes it terminate: inference gives up the context hint past the budget,
           and the program is REJECTED in a fraction of a second (the self-app's Int64 result used as an if
           condition is CDZ0203 'if condition must be Bool'). The point is 'never hang' — a compiler
           completes or declines on any input, regardless of the syntactic form the divergence hides in.")
  (input
    (do
      (def (main) ((fn (v0) (if (v0 v0) 1 (v0 v0))) (fn (v2) (if (v2 v2) 1 (v2 v2)))))
      (export main)))
  (error CDZ0203 (exact-code)))

(case
  "a tuple-wrapped self-application is rejected in bounded time, not a compiler stack overflow"
  (doc
    "`(fn v (tuple (v v) 1))` applied to a copy of itself has no normal form: the self-app `(v v)` in
           a tuple slot grows the term exponentially. Here the reduction BUDGET already terminates inference
           (β-reduction gives up past the work budget) — but that leaves a MEMOIZED core chain thousands of
           nodes deep, `Tuple[Tuple[…poison…, 1], 1]`, bottoming out in the reduction-bound poison. That
           chain is built bottom-up at shallow demand depths, so lowering's own descent guard never fires on
           it; the REACHED-POISON walk (`collect_reached_poisons`, which reports a provable trap that a
           program unconditionally reaches) then descended the whole pre-built chain in ONE native recursion
           and OVERFLOWED THE COMPILER'S STACK — a process abort on a small valid-to-parse program. Giving
           that walk the same recursive-descent depth guard lowering has makes it surface the reduction-bound
           poison (CDZ0999) past the limit instead of crashing. The guard sits at the walk's single recursive
           entry and the walk dispatches structurally, so the whole compound-construction class (a self-app
           in a tuple / record / list / sum / map / set slot) is covered by ONE guard — not one syntactic
           wrapper at a time. The point is 'never crash' — a compiler completes or declines on any input,
           regardless of the syntactic form the divergence hides in.")
  (input (do (def (main) ((fn (v0) #tuple((v0 v0) 1)) (fn (v2) #tuple((v2 v2) 1)))) (export main)))
  (error CDZ0999))

(case
  "a sum-payload-wrapped self-application is rejected in bounded time, not a compiler stack overflow"
  (doc
    "The SUM-CONSTRUCTOR-payload sibling of the tuple-wrapped case above: `(fn v (Some (v v)))` applied
           to a copy of itself. `cdz check` (inference) already declines CDZ0999 (the reduction work budget),
           but `cdz compile` HUNG at a later phase — the LAYOUT reachability walks (`collect_call_callees` /
           `collect_closure_codes`) descend a `Core::SumNew` payload by calling `core_of`, which β-reduces one
           more level per call WITHOUT holding the reduction-DEPTH guard (unlike tuple lowering), so the walk
           materializes an unbounded `Core::SumNew` chain and descends it in ONE native recursion until the
           stack OVERFLOWS. The tuple/record/list walks were bounded earlier; this bounds the sum path too, by
           a DEDICATED walk-depth counter (kept separate from `core_of`'s descent counter, which the walk also
           drives — sharing would spuriously decline a valid moderately-deep program). Past the limit the walk
           stops descending and `collect_faults` reports the coded CDZ0999. Also `(Ok (v v))` and a user
           multi-payload `(P (v v) 1)`. The point is 'never crash' — a compiler completes or declines on any
           input from BOTH check and compile, regardless of the compound the divergence hides in.")
  (input (do (def (main) ((fn (v0) (Some (v0 v0))) (fn (v2) (Some (v2 v2))))) (export main)))
  (error CDZ0999))

(case
  "a deeply nested constant expression compiles or declines without crashing"
  (doc
    "A 64-deep nest of `(+ 1 …)` folds to 65 — well within any reasonable bound. The point is the
           companion the gate cannot record: the SAME shape thousands deep must DECLINE (a
           recursion/resource-limit rejection) rather than overflow the compiler's stack and abort. This
           anchors the shallow end; the compiler bounds its own recursive descent and declines when the
           bound is reached, so a pathological depth is a decline, never a process crash.")
  (input
    (do
      (def
        (main)
        (+
          1
          (+
            1
            (+
              1
              (+
                1
                (+
                  1
                  (+
                    1
                    (+
                      1
                      (+
                        1
                        (+
                          1
                          (+
                            1
                            (+
                              1
                              (+
                                1
                                (+
                                  1
                                  (+
                                    1
                                    (+
                                      1
                                      (+
                                        1
                                        (+
                                          1
                                          (+
                                            1
                                            (+
                                              1
                                              (+
                                                1
                                                (+
                                                  1
                                                  (+
                                                    1
                                                    (+
                                                      1
                                                      (+
                                                        1
                                                        (+
                                                          1
                                                          (+
                                                            1
                                                            (+
                                                              1
                                                              (+
                                                                1
                                                                (+
                                                                  1
                                                                  (+
                                                                    1
                                                                    (+
                                                                      1
                                                                      (+
                                                                        1
                                                                        (+
                                                                          1
                                                                          (+
                                                                            1
                                                                            (+
                                                                              1
                                                                              (+
                                                                                1
                                                                                (+
                                                                                  1
                                                                                  (+
                                                                                    1
                                                                                    (+
                                                                                      1
                                                                                      (+
                                                                                        1
                                                                                        (+
                                                                                          1
                                                                                          (+
                                                                                            1
                                                                                            (+
                                                                                              1
                                                                                              (+
                                                                                                1
                                                                                                (+
                                                                                                  1
                                                                                                  (+
                                                                                                    1
                                                                                                    (+
                                                                                                      1
                                                                                                      (+
                                                                                                        1
                                                                                                        (+
                                                                                                          1
                                                                                                          (+
                                                                                                            1
                                                                                                            (+
                                                                                                              1
                                                                                                              (+
                                                                                                                1
                                                                                                                (+
                                                                                                                  1
                                                                                                                  (+
                                                                                                                    1
                                                                                                                    (+
                                                                                                                      1
                                                                                                                      (+
                                                                                                                        1
                                                                                                                        (+
                                                                                                                          1
                                                                                                                          (+
                                                                                                                            1
                                                                                                                            (+
                                                                                                                              1
                                                                                                                              (+
                                                                                                                                1
                                                                                                                                (+
                                                                                                                                  1
                                                                                                                                  (+
                                                                                                                                    1
                                                                                                                                    (+
                                                                                                                                      1
                                                                                                                                      (+
                                                                                                                                        1
                                                                                                                                        1)))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))
      (export main)))
  (output (: 65 Int64)))

(case
  "a deeply nested expression is diagnosed by the parser, never crashes it"
  (doc
    "The PARSER's recursive descent (both the s-expr reader and the ML Pratt parser) must return a
           clean diagnostic on pathologically deep nesting, not overflow the native stack and abort the
           process (SIGABRT). The COMPILER already guards this — the case above declines a deep nest at
           the descent-depth bound — but the parser, which runs FIRST on any source-ingesting path
           (`convert`/`check`/`fix`, and critically the guide's `cdz-wasm` on untrusted browser input at
           a ~1MB stack), had no equivalent limit: a depth ≳25000 source crashed with 'thread main has
           overflowed its stack' where `cdz compile` on the same shape cleanly rejects. Both readers now
           carry a nesting-depth guard (mirroring the compiler's limit) that returns a parse error past
           the bound. This small depth-8 witness parses and evaluates fine (=> 9), pinning the SHAPE; the
           crash needs a depth-25000 generator, impractical to inline. Fix: a parse-time depth guard, the
           read-side analogue of the compiler's descent-depth limit.")
  (input (do (def (main) (+ (+ (+ (+ (+ (+ (+ (+ 1 1) 1) 1) 1) 1) 1) 1) 1)) (export main)))
  (output (: 9 Int64)))

; --- A nested CALL chain compiles in roughly LINEAR time, never exponential ----------------------
; The deeply-nested-CONSTANT case above declines cleanly at a pathological depth (the descent-depth
; guard). A nested CALL chain `(f (f (f … 0)))` is a DIFFERENT cost: each level β-inlines the callee
; body, and both `infer` and `lower` reduce every call, recursing into the reduced (fault + type) walk.
; A generation that did not MEMOIZE the reduction and the fault walk re-analyzed each cached-but-shared
; reduced term per enclosing level — EXPONENTIAL in the depth (×2 per level; far worse — 2^depth — when
; the callee DUPLICATES its parameter, so the substituted term doubles each level). A ~20-deep chain
; took seconds, ~50 never finished: a compiler HANG on a trivial, well-formed program. Memoizing the
; β-reduction (a call site reduces once) and the fault collection (a node's faults are collected once)
; makes the chain LINEAR, so it folds to its constant. These pin the folded value at a depth that would
; have taken exponential time unmemoized; the pathology was the GROWTH RATE, so a linear-time compile is
; the property. (A chain nested deeper than the inliner reduces is a resource-limit DECLINE, not a hang.)
(case
  "a nested chain of function calls compiles in linear time and folds to a constant"
  (doc
    "`(f (f (f … (f 0))))` — a depth-18 chain of `(def (f n) (+ n 1))`. Each level inlines the
           callee; the emitted program is a single constant (18). Unmemoized this took time EXPONENTIAL
           in the depth (167ms@16, 652ms@18, 10s@22, never finishing by depth 50) — a hang on a trivial
           program. With the reduction and the fault walk memoized it compiles in milliseconds and folds
           to 18 (0, then +1 eighteen times). Pins that a nested call chain is compiled in roughly linear
           time, never exponentially; the value triangulates the fold is correct, and the depth is chosen
           to be far past where the unmemoized compile was already seconds.")
  (input
    (do
      (def (f n) (+ n 1))
      (def (main) (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f 0)))))))))))))))))))
      (export main)))
  (call main)
  (output (: 18 Int64)))

(case
  "a nested call chain whose callee duplicates its parameter folds without exponential blowup"
  (doc
    "The worse shape: `(def (g n) (+ n n))` DUPLICATES its parameter, so each inline DOUBLES the
           substituted term — a depth-d chain is 2^d nodes if re-analyzed naively. Unmemoized, depth 15
           already took ~17s and depth 18 never finished. `(g (g … (g 1)))` at depth 12 computes
           1·2^12 = 4096. Pins that parameter DUPLICATION under nesting does not make the compile
           exponential — the classic β-reduction size explosion a real compiler bounds by memoizing the
           reduction and the per-node analyses. Folds to 4096.")
  (input
    (do
      (def (g n) (+ n n))
      (def (main) (g (g (g (g (g (g (g (g (g (g (g (g 1)))))))))))))
      (export main)))
  (call main)
  (output (: 4096 Int64)))

(case
  "McCarthy 91 — a self-call nested INSIDE the argument of another self-call"
  (doc
    "The literal chains above nest a FIXED depth the compiler can see; here the nesting is
           DATA-DEPENDENT: `(m91 (m91 (+ x 11)))` recurses with a self-call as the argument of
           another self-call, and how deep the double descent goes depends on the runtime x (the
           inner call is NON-TAIL — its result feeds the outer call, so each frame holds a live
           continuation). The function is the classic total-but-tricky recursion: m91(x) = x-10 for
           x > 100, and exactly 91 for EVERY x ≤ 100 (the plateau). Probes walk the boundary: 1, 99,
           -50 (deep plateau), 100 (the boundary hop — 100 → m91(m91(111)) → m91(101) → 91), 101 (the
           first direct exit), then 102 → 92 and 200 → 190 on the linear side. A specialization or
           inline pass that unrolled the visible double-nest but mis-fixed the recursion's exit
           condition flattens the plateau or shifts the boundary.")
  (input
    (do
      (def (m91 (: x Int64)) (if (> x 100) (- x 10) (m91 (m91 (+ x 11)))))
      (def (main (: x Int64)) (m91 x))
      (export main)))
  (call main (: 1 Int64))
  (output (: 91 Int64))
  (call main (: 99 Int64))
  (output (: 91 Int64))
  (call main (: 100 Int64))
  (output (: 91 Int64))
  (call main (: 101 Int64))
  (output (: 91 Int64))
  (call main (: 102 Int64))
  (output (: 92 Int64))
  (call main (: 200 Int64))
  (output (: 190 Int64))
  (call main (: -50 Int64))
  (output (: 91 Int64)))

(case
  "the COLLATZ walk counts steps to 1 and tracks the trajectory peak"
  (doc
    "The 3n+1 iteration — like McCarthy above, a recursion whose DEPTH is data-dependent and
           non-obvious from the argument (27 takes 111 steps and peaks at 9232; 97 takes 118 to the
           SAME peak — the two trajectories join). The walk threads (steps, peak) as a pair, the
           peak read from the FRESH next value (a stale-peak read misses a spike hit on the final
           odd step before a descent). The parity branch alternates `/2` and `3n+1` in a
           data-dependent pattern no unrolling predicts. Faces: 1 (zero steps — the tuple returns
           before any iteration; peak = the seed, 1); 6 → 8 steps peak 16 (80016); 27 → the famous
           long trajectory, 111 steps peak 9232 (1110232); 97 → 118 steps to the same 9232
           (1180232 — two different step counts converging on one peak pins both fields
           independently). Encoding: steps·10000 + peak mod 1000.")
  (input
    (do
      (def (max2 (: a Int64) (: b Int64)) (if (> a b) a b))
      (def
        (go (: n Int64) (: steps Int64) (: peak Int64))
        (if
          (= n 1)
          #tuple(steps peak)
          (do (def nx (if (= (% n 2) 0) (/ n 2) (+ (* 3 n) 1))) (go nx (+ steps 1) (max2 peak nx)))))
      (def
        (main (: n Int64))
        (match (go n 0 n) (#tuple(steps peak) (+ (* steps 10000) (% peak 1000)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 6 Int64))
  (output (: 80016 Int64))
  (call main (: 27 Int64))
  (output (: 1110232 Int64))
  (call main (: 97 Int64))
  (output (: 1180232 Int64))
  (live-objects 0))

(case
  "the JOSEPHUS survivor agrees between the modular recurrence and a list elimination simulation"
  (doc
    "Two totally different computations of the same survivor, cross-checked: the O(n) MODULAR
           RECURRENCE `r' = (r + k) mod i` folded over ring sizes 2..n (pure index arithmetic — no
           ring exists), against a LIST SIMULATION that builds the ring 1..n and eliminates every
           k-th by `drop-at` (a positional filter rebuilding the spine each round, the cursor
           wrapping by `mod len` on the SHRINKING list). The two share no code path — recurrence
           bugs (a mod at the wrong ring size) and simulation bugs (a cursor not adjusted for the
           removed slot) land on different wrong answers, so the agreement bit is a strong witness.
           Faces: the classic n=7,k=3 → survivor 4 (41); n=1 (both degenerate — the recurrence loop
           never runs, the simulation matches `(list survivor)` immediately → 11); n=5,k=2 → 3 (31);
           k=1 (eliminate-in-order — the survivor is the LAST position, n=10 → 101).")
  (input
    (do
      (def
        (go (: i Int64) (: n Int64) (: k Int64) (: r Int64))
        (if (> i n) (+ r 1) (go (+ i 1) n k (% (+ r k) i))))
      (def (josephus (: n Int64) (: k Int64)) (go 2 n k 0))
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (> i n) acc (build (+ i 1) n (List.push acc i))))
      (def
        (drop-at (: xs (List Int64)) (: i Int64) (: j Int64) (: acc (List Int64)))
        (match
          xs
          (#list() acc)
          (#list(h (.. t)) (drop-at t i (+ j 1) (if (= j i) acc (List.push acc h))))))
      (def
        (sim (: ring (List Int64)) (: idx Int64) (: k Int64))
        (match
          ring
          (#list(survivor) survivor)
          (_
            (do
              (def len (List.len ring))
              (def hit (% (+ idx (- k 1)) len))
              (sim (drop-at ring hit 0 #list()) hit k)))))
      (def
        (main (: n Int64) (: k Int64))
        (do
          (def fast (josephus n k))
          (def slow (sim (build 1 n #list()) 0 k))
          (+ (* fast 10) (if (= fast slow) 1 0))))
      (export main)))
  (call main (: 7 Int64) (: 3 Int64))
  (output (: 41 Int64))
  (call main (: 1 Int64) (: 5 Int64))
  (output (: 11 Int64))
  (call main (: 5 Int64) (: 2 Int64))
  (output (: 31 Int64))
  (call main (: 10 Int64) (: 1 Int64))
  (output (: 101 Int64))
  (live-objects known-leak))

; The FAULT WALK over a nested call chain must be LINEAR too, not just the reduction. `type_errors`
; checks each call at its site AND collects the reduced body — and it separately descended each raw
; ARGUMENT for its own faults. On a chain `(f (f … (f 0)))` (where each argument IS the next call) that
; per-level argument descent RE-WALKED the whole remaining chain, and — because a resource-limit-clipped
; walk is not cached — restarted from scratch at every enclosing level, so REACHING the answer was O(N³)
; (a depth-30 chain folded in ms, but a deeper one took seconds→minutes just to decline). The redundant
; descent is dropped for a lambda head whose parameter the body USES (its argument is already in the
; reduced body); only a DEAD argument the body ignores is still descended (its faults are not otherwise
; seen). This case folds a chain at the deepest value-producing depth (just under the inliner's reduce
; limit), exercising the now-linear fault walk near the boundary; a deeper chain is a clean resource-limit
; DECLINE, reached in linear time rather than a hang.
(case
  "a deeper nested call chain still folds in linear time near the inliner limit"
  (doc
    "A depth-30 chain of the incrementing `f` — near the inliner's reduce limit, the deepest that
           still folds to a value: 0, then +1 thirty times = 30. The reduction was already memoized and
           linear, but the FAULT WALK re-descended each raw argument, which on a call chain re-walked the
           remaining chain per level — cubic to reach the answer. Dropping that redundant descent for a
           used parameter, whose argument is already in the reduced body, makes the whole compile linear.
           Pins the fold at a depth the cubic fault walk handled only slowly; a deeper chain declines
           cleanly at a resource limit rather than hanging.")
  (input
    (do
      (def (f n) (+ n 1))
      (def
        (main)
        (f
          (f
            (f
              (f
                (f
                  (f
                    (f
                      (f
                        (f
                          (f
                            (f
                              (f
                                (f
                                  (f
                                    (f
                                      (f (f (f (f (f (f (f (f (f (f (f (f (f (f (f 0)))))))))))))))))))))))))))))))
      (export main)))
  (call main)
  (output (: 30 Int64)))

; A match/pattern BINDER used more than once must NOT re-emit its whole scrutinee per use. When the
; scrutinee is a RECURSIVE CALL, a binder used K times re-runs that call K times per recursion level →
; 2^depth runtime recompute (the pattern-binder twin of the tuple-match fall-through exponential the
; decision-tree fix closed). Here `f` recurses to its base at n=0, and the recursive arm binds `a` from
; `(match (f (+ n 1)) ((Mk a _) …))` and USES IT TWICE (`(Mk a a)`). The `MatchSum` wrapper materializes
; the recursive scrutinee into ONE slot read by every binder (A-normal form), so it runs once per level.
; This pins the VALUE across both backends at a moderate depth (both run it); the DEEP exponential-
; regression catch is the wasm unit test `a_recursive_match_binder_scrutinee_is_materialized_once`
; (BUILD/emit-count, no runtime trap). Value: base `(Mk 1 1)`, every arm rebuilds `(Mk a a)` with `a`=1.
(case
  "a match binder used twice binds its recursive scrutinee once (A-normalized, value-correct)"
  (doc
    "`f` recurses to `(Mk 1 1)` at n=0; each recursive arm matches `(f (+ n 1))`, binds `a`, and uses
           it TWICE in `(Mk a a)`. `(f -20)` returns 1 (the first field, always 1) on both backends. The
           recursive scrutinee is materialized ONCE per level (the `MatchSum` wrapper's slot), so a payload
           binder's multiple uses read the slot, not re-run the call. Pins the value; the linear-vs-2^depth
           regression is caught by the wasm build-count unit test.")
  (input
    (do
      (type P (Mk Int64 Int64))
      (def (f (: n Int64)) (if (= n 0) (Mk 1 1) (match (f (+ n 1)) ((Mk a _) (Mk a a)))))
      (def (main) (match (f -20) ((Mk x _) x)))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

; --- A recursive Bool-returning function used as a condition, in BOTH branch orders --------------
; A recursive predicate — "all elements from i satisfy P" — is a byte/element loop whose recursive
; self-call sits in one branch of an inner `if` and a Bool literal in the other: `(if guard (recurse …)
; false)` (all-so-far, else fail) or its mirror `(if guard false (recurse …))`. Both denote a Bool and
; must type as a Bool CONDITION regardless of which branch holds the self-call — the recursive
; function's return kind is inferred from its body, and a still-unsolved self-call must NOT let branch
; ORDER decide the kind (a Bool-literal branch pins the result to Bool). This is the return-kind
; companion of the recursion cases above, and the exact shape of a reader's byte-by-byte name matcher.
(case
  "a recursive predicate with the self-call in the then branch is a Bool condition"
  (doc
    "`all-lt` tests that every element from i is < the bound: `(if (< i n) (if (< i bound)
           (all-lt (+ i 1) n bound) false) true)` — the recursive self-call is the THEN branch, the
           `false` is the ELSE. Used as an `if` condition, `all-lt` MUST type as Bool; with n=3 and a
           bound of 5 over indices 0,1,2 (all < 5) it is true, so the outer `if` yields 1. Pins that a
           recursive Bool function whose self-call is the then-branch infers a Bool return regardless of
           branch order — the shape a reader's name matcher takes ('all bytes equal so far, else fail').")
  (input
    (do
      (def (all-lt i n bound) (if (< i n) (if (< i bound) (all-lt (+ i 1) n bound) false) true))
      (def (main) (if (all-lt 0 3 5) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a recursive predicate with the self-call in the else branch is a Bool condition"
  (doc
    "The mirror of the case above: the self-call is the ELSE branch and `false` the THEN —
           `(if (< i n) (if (< i bound) false (all-ge (+ i 1) n bound)) true)`, testing every element
           from i is NOT < the bound. With n=3, bound=0 over indices 0,1,2 (none < 0) it is true → 1.
           Pins that BOTH branch orders of a recursive Bool predicate type identically as a Bool
           condition (the return-kind inference is order-independent).")
  (input
    (do
      (def (all-ge i n bound) (if (< i n) (if (< i bound) false (all-ge (+ i 1) n bound)) true))
      (def (main) (if (all-ge 0 3 0) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "a recursive def with a match base case computes over its argument"
  (doc
    "Witnesses core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments,
           with the base case expressed
           as a `match` on the argument rather than an `if`. This is the canonical functional idiom:
           sum-to(n) matches 0 → 0, else n + sum-to(n-1). The base-case arm must be selected from
           the RUNTIME value of n; sum-to(3) = 3 + 2 + 1 + 0 = 6. Companion to the if-based
           `sum-to` above — both must agree.")
  (input
    (do
      (def (sum-to n) (match n (0 0) (_ (+ n (sum-to (- n 1))))))
      (def (main) (sum-to 3))
      (export main)))
  (output (: 6 Int64)))

(case
  "recursive factorial with a match base case"
  (doc
    "core-semantics.md §Recursion: factorial via a match on the argument. The 0 arm is the
           base case, reached only from the runtime value hitting 0 after counting down; without
           selecting the 0 arm at run time the recursion would never terminate. fact(5) = 120.")
  (input
    (do (def (fact n) (match n (0 1) (_ (* n (fact (- n 1)))))) (def (main) (fact 5)) (export main)))
  (output (: 120 Int64)))

(case
  "recursive fibonacci with literal match base cases"
  (doc
    "core-semantics.md §Recursion: two literal base-case arms (0 and 1) matched against the
           runtime argument, and a recursive arm summing the two predecessors. fib(10) = 55.
           Exercises multiple literal arms dispatching on a runtime scrutinee within a recursion.")
  (input
    (do
      (def (fib n) (match n (0 0) (1 1) (_ (+ (fib (- n 1)) (fib (- n 2))))))
      (def (main) (fib 10))
      (export main)))
  (output (: 55 Int64)))

(case
  "recursive fibonacci over a RUNTIME argument computes the predecessors correctly"
  (doc
    "`(def (fib (: n Int64)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))` called with a RUNTIME
           argument x = 10 (not a constant, so the recursion runs at run time, not folded). Each recursive
           call passes a computed argument `(- n 1)` / `(- n 2)`; under the `n >= 2` branch refinement the
           subtraction cannot underflow, so its overflow guard is elided and the argument value flows
           straight into the call (no dead spill slot). fib(10) = 55. Pins that a guard-elided computed
           call argument is passed correctly.")
  (input
    (do
      (def (fib (: n Int64)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
      (def (main (: x Int64)) (fib x))
      (export main)))
  (call main (: 10 Int64))
  (output (: 55 Int64)))

(case
  "FAST-DOUBLING fib recurses on the halved index returning a pair, checked against linear iteration"
  (doc
    "The O(log n) sibling of the naive fibs above: ONE recursive call on the HALVED index returns
           the PAIR (fib(k), fib(k+1)), and the parity of n selects which doubling identity rebuilds
           the answer — even keeps (c,d) = (a(2b−a), a²+b²), odd shifts to (d, c+d). The recursion is
           non-tail (both identities read the returned pair), the intermediate c/d arithmetic runs at
           every level, and a parity branch taken wrong at ANY level lands on a NEARBY fibonacci —
           which the DIFFERENTIAL oracle (a linear two-accumulator iteration, agreement bit in the
           output) catches even when the wrong answer looks plausible. Faces: n=0 (the base pair,
           loop never entered → 1), n=1 (one odd step → 11), n=10 (55 → 551, even top), n=31 (all-ONES
           binary — every level takes the ODD branch → 13462691), n=64 (single-bit index — every level
           but the last takes the EVEN branch; fib(64)=10610209857723, the doubling arithmetic runs
           near the top of Int64's comfortable range → 106102098577231).")
  (input
    (do
      (def
        (fd (: n Int64))
        (if
          (= n 0)
          #tuple(0 1)
          (match
            (fd (/ n 2))
            (#tuple(a b)
              (do
                (def c (* a (- (* 2 b) a)))
                (def d (+ (* a a) (* b b)))
                (if (= (% n 2) 0) #tuple(c d) #tuple(d (+ c d))))))))
      (def
        (lin (: i Int64) (: n Int64) (: a Int64) (: b Int64))
        (if (>= i n) a (lin (+ i 1) n b (+ a b))))
      (def
        (main (: n Int64))
        (do
          (def fast (match (fd n) (#tuple(a _b) a)))
          (def slow (lin 0 n 0 1))
          (+ (* fast 10) (if (= fast slow) 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 10 Int64))
  (output (: 551 Int64))
  (call main (: 31 Int64))
  (output (: 13462691 Int64))
  (call main (: 64 Int64))
  (output (: 106102098577231 Int64))
  (live-objects 0))

(case
  "a 2x2 MATRIX POWER by squaring recovers fibonacci and satisfies the determinant identity"
  (doc
    "The third fibonacci computation (naive recursion, fast-doubling above, now the LINEAR-MAP
           form): the Q-matrix (1 1 / 1 0) raised by ITERATIVE square-and-multiply — an accumulator
           seeded with the IDENTITY matrix, squaring the base each halving and folding it in on odd
           bits (the tail-loop dual of modpow's recursive form). The 2x2 multiply reads all FOUR
           slots of both flat 4-tuples in the 8-product pattern — a slot transposition anywhere
           produces a plausible-but-wrong matrix. TWO independent certificates: Q^k's off-diagonal
           IS fib(k), and det(Q^k) = (-1)^k (the determinant is multiplicative, so a single slot
           error breaks it at every k where it fires). Faces: k=1 (11), k=10 → 55 (551 — agreeing
           with BOTH other fib pins), k=30 → 832040 (8320401), k=0 → the untouched IDENTITY (fib 0,
           det +1 → 1).")
  (input
    (do
      (def
        (mm (: a (Tuple Int64 Int64 Int64 Int64)) (: b (Tuple Int64 Int64 Int64 Int64)))
        (match
          a
          (#tuple(a11 a12 a21 a22)
            (match
              b
              (#tuple(b11 b12 b21 b22)
                #tuple((+ (* a11 b11) (* a12 b21))
                  (+ (* a11 b12) (* a12 b22))
                  (+ (* a21 b11) (* a22 b21))
                  (+ (* a21 b12) (* a22 b22))))))))
      (def
        (mpow
          (: m (Tuple Int64 Int64 Int64 Int64))
          (: k Int64)
          (: r (Tuple Int64 Int64 Int64 Int64)))
        (if (= k 0) r (mpow (mm m m) (/ k 2) (if (= (% k 2) 1) (mm r m) r))))
      (def
        (det (: m (Tuple Int64 Int64 Int64 Int64)))
        (match m (#tuple(a b c d) (- (* a d) (* b c)))))
      (def
        (main (: k Int64))
        (do
          (def f (mpow #tuple(1 1 1 0) k #tuple(1 0 0 1)))
          (match
            f
            (#tuple(_a fib _c _d) (+ (* fib 10) (if (= (det f) (if (= (% k 2) 0) 1 -1)) 1 0))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 10 Int64))
  (output (: 551 Int64))
  (call main (: 30 Int64))
  (output (: 8320401 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; --- Overflow checking holds THROUGH a recursive call chain, not only at the top level ----
; numeric-model.md #Overflow Is Defined: an integer operation that overflows traps under the checked
; default. The `(+ Int64.max 1)` and `(* Int64.max 2)` cases (06-numeric-model) pin this for a top-level
; operation on constant operands; here the overflowing `*` is buried inside a RECURSION, reached only
; after the call chain unwinds. `fact(20)` = 2432902008176640000 is the largest factorial that fits
; Int64; `fact(21)` = 21·fact(20) ≈ 5.1e19 overflows, and the checked `*` MUST trap when the recursion
; multiplies up to it — not wrap to a garbage value. A generation that emits a checked `*` at the top
; level but an unchecked one inside a recursive helper would compute a wrong `fact(21)` and pass every
; small-input recursion case; this pins the boundary.
(case
  "the largest factorial that fits the integer type computes exactly"
  (doc
    "fact(20) = 2432902008176640000, the largest factorial within Int64 (fact(21) overflows). The
           recursion multiplies 20·19·…·1 with the checked `*`, and every intermediate product stays in
           range, so it computes the exact value — the passing companion of the overflow case below.")
  (input
    (do
      (def (fact n) (match n (0 1) (_ (* n (fact (- n 1))))))
      (def (main) (fact 20))
      (export main)))
  (output (: 2432902008176640000 Int64)))

(case
  "a factorial that overflows the integer type traps through the recursion"
  (doc
    "fact(21) = 21·fact(20) ≈ 5.1e19, which overflows Int64. The overflowing `*` sits INSIDE the
           recursion, reached as the call chain unwinds; the checked-Int64 default MUST trap there
           (numeric-model.md #Overflow Is Defined), not wrap to a wrong value. Pins that overflow
           checking is emitted on the recursive arithmetic path, not only for a top-level constant
           operation — the recursion companion of `(* Int64.max 2)`.")
  (input
    (do
      (def (fact n) (match n (0 1) (_ (* n (fact (- n 1))))))
      (def (main) (fact 21))
      (export main)))
  (trap "integer overflow"))

(case
  "a linear non-tail recursion over a non-associative operator preserves its exact result"
  (doc
    "A compiler may turn a LINEAR non-tail recursion — one self-call whose result feeds a single
           enclosing operation — into an accumulator TAIL LOOP (accumulator introduction), so deep
           recursion runs in constant stack. That rewrite must preserve the EXACT result, including for a
           NON-ASSOCIATIVE operator where the evaluation ORDER matters. `(alt n) = n - (alt (n-1))`, base
           `(alt 0) = 0`, is right-nested subtraction: alt(5) = 5−(4−(3−(2−(1−0)))) = 5−(4−(3−(2−1))) =
           5−(4−(3−1)) = 5−(4−2) = 5−2 = 3. A transform that naively accumulated `acc − n` left-to-right
           would give a DIFFERENT number; the loop must reproduce the right-nested value 3. Pins that
           accumulator introduction is result-preserving for a non-associative step, not only for `+`/`*`.")
  (input
    (do
      (def (alt (: n Int64)) (if (= n 0) 0 (- n (alt (- n 1)))))
      (def (main (: n Int64)) (alt n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64)))

(case
  "a recursive def named a target-language keyword runs"
  (doc
    "`loop` is a valid Cadenza identifier but a keyword in some backends (Rust). A RECURSIVE def named
           `loop` SURVIVES as a real function (a non-recursive one inlines away), so a backend that emits the
           source name verbatim as its function identifier would produce `fn loop(…)` — invalid in a language
           where `loop` is reserved. `loop(3)` counts down to 42. Pins that a def whose name collides with a
           target keyword is emitted as an escaped identifier (a raw identifier `r#loop` on the Rust backend;
           the wasm backend is unaffected — function names there are indices, not identifiers), so the same
           program runs on every backend. Also covers while/for/type/mut/impl/… as surviving function names.")
  (input
    (do (def (loop (: n Int64)) (if (= n 0) 42 (loop (- n 1)))) (def (main) (loop 3)) (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "accumulator introduction threads a transformed extra parameter through a multi-parameter recursion"
  (doc
    "Accumulator introduction generalizes to a MULTI-parameter linear recursion: an extra parameter
           that is TRANSFORMED at each recursive step (not merely carried) must be threaded correctly by
           the loop. `(f n m) = if n=0 then 0 else m + (f (n-1) (m*2))` sums a geometric sequence — each
           step adds the current `m` and doubles it for the next: f(4,1) = 1 + 2 + 4 + 8 = 15 = m·(2^n − 1).
           The transform must carry `m` through the accumulator loop applying the per-step `m*2` in the
           right order; a rewrite that dropped the transformation (kept `m` constant) would compute n·m = 4,
           and one that mis-ordered the doublings would differ. Pins that a per-step-transformed threaded
           parameter is preserved by the multi-parameter accumulator loop, the multi-param extension of the
           single-parameter accumulator cases.")
  (input
    (do
      (def (f (: n Int64) (: m Int64)) (if (= n 0) 0 (+ m (f (- n 1) (* m 2)))))
      (def (main (: n Int64) (: m Int64)) (f n m))
      (export main)))
  (call main (: 4 Int64) (: 1 Int64))
  (output (: 15 Int64)))

; --- Two functions may recurse through EACH OTHER (mutual recursion) ----------------------
; core-semantics.md §Recursion + §A Function Is A First-Class Value: recursion need not be
; self-recursion — two top-level defs may call each other, each in scope in the other's body (the same
; lexical resolution that makes a single recursive def work, extended to a pair). `even`/`odd` count
; down through one another; the base case is reached only after the mutual chain unwinds. The existing
; recursion cases are all SELF-recursive; this pins that a mutually-recursive pair resolves and
; terminates too, and returns the Bool result faithfully.
(case
  "two functions defined by mutual recursion compute a result"
  (doc
    "`even` and `odd` are mutually recursive: each calls the other with n-1 until n reaches 0.
           even(10) counts 10→9→…→0 alternating between the two defs and returns true (10 is even). Pins
           that mutual recursion resolves (each def is in scope in the other's body) and terminates via
           the shared base case, carrying the Bool result across the run boundary.")
  (input
    (do
      (def (even n) (if (= n 0) true (odd (- n 1))))
      (def (odd n) (if (= n 0) false (even (- n 1))))
      (def (main) (even 10))
      (export main)))
  (output (: true Bool)))

(case
  "the other parity of a mutually-recursive pair"
  (doc
    "The companion on the other outcome: even(7) alternates even→odd→…→base and returns false (7
           is odd). Confirms the mutual recursion follows the runtime count to the correct base-case
           result for both parities, not a fixed answer.")
  (input
    (do
      (def (even n) (if (= n 0) true (odd (- n 1))))
      (def (odd n) (if (= n 0) false (even (- n 1))))
      (def (main) (even 7))
      (export main)))
  (output (: false Bool)))

; --- A mutually-recursive pair returning a heap SUM, with a helper's `if`-condition being a CALL ---
; The parity pair above is Int64/Bool-only and tail-recursive (a shared loop). This pins the HEAP-SUM,
; NON-tail mutual shape a real type-checker takes: `check` factors the `If` typing rule into a helper
; `check-if`, and each returns a `(Result Ty TErr)`. Two joint bugs used to sink it:
;   (A) INFERENCE — `check`/`check-if` never build an `Err(...)` on the recursive spine, so the error
;       slot stays a free var `(Result Ty ?err)`. Re-wrapping `(Result.Err et)` in a match arm read `et`
;       via the `Err` ctor scheme instantiated FROM ZERO, colliding with the scrutinee's free `?err = ?0`,
;       so `et` solved to the FIRST type arg `Ty` and the arm typed `(Result Ty Ty)` → a spurious CDZ0203
;       "if branches differ". (Fixed by seeding the ctor instantiation past the scrutinee's vars.)
;   (B) BACKEND — monomorphizing the generic helper `check-if` copied its body, but a pinned match-arm
;       binder (`ct`) whose SCRUTINEE `(check c)` lies WITHIN the copied body was SHARED rather than copied,
;       so it read the original, now-orphaned scrutinee whose param `c` had no slot in the copy → "parameter
;       reference has no local slot". (Fixed by copying such a binder when its scrutinee is within the copy.)
; `main` runs `check(If(Num 1, Num 2))`: `check(Num 1)` = `Ok(TInt)`, `is-bool(TInt)` = false, so `check-if`
; yields `Err(CondNotBool)` and `main` takes the `Err` arm → 0. Also exercises bug (C) below: `TErr` is a
; SINGLE nullary variant (erased to a `Ty::Nominal { inner: Unit }`), so `Result.Err`'s payload is Unit —
; a Unit-typed sum payload built + re-wrapped inside a recursive function.
(case
  "a mutually-recursive check/check-if returning a Result compiles and runs (if-cond is a call)"
  (doc
    "`check` ↔ `check-if` mutually recurse and each returns `(Result Ty TErr)`. `check-if` gates a
           nested recursive `(check t)` on `(if (is-bool ct) …)` — a CALL condition on the match-bound
           payload `ct` — and re-wraps `(Result.Err et)`. This is the shape that jointly hit a fresh-var
           inference collision (spurious CDZ0203), a monomorphization slot bug ('parameter reference has no
           local slot'), and a Unit-payload construction gap. `check(If(Num 1, Num 2))`: the condition
           `Num 1` types `TInt`, `is-bool(TInt)` is false, so the result is `Err(CondNotBool)` → main → 0.")
  (input
    (do
      (type Ty TInt TBool)
      (type TErr CondNotBool)
      (type Exp (Num Int64) (If Exp Exp))
      (def (is-bool (: t Ty)) (match t ((TBool) true) ((TInt) false)))
      (def (check (: e Exp)) (match e ((Exp.Num _) (Result.Ok TInt)) ((Exp.If c t) (check-if c t))))
      (def
        (check-if (: c Exp) (: t Exp))
        (match
          (check c)
          ((Result.Ok ct)
            (if
              (is-bool ct)
              (match (check t) ((Result.Ok tt) (Result.Ok tt)) ((Result.Err et) (Result.Err et)))
              (Result.Err CondNotBool)))
          ((Result.Err ec) (Result.Err ec))))
      (def
        (main)
        (match (check (Exp.If (Exp.Num 1) (Exp.Num 2))) ((Result.Ok _) 1) ((Result.Err _) 0)))
      (export main)))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "the well-typed branch of the mutual check returns the Ok result"
  (doc
    "The companion outcome of the check/check-if pair on a well-typed input. `check(Num 5)` =
           `Ok(TInt)` directly (no `If`, so no condition check), so `main` takes the `Ok` arm → 1. Confirms
           the mutual Result-returning shape carries BOTH the `Ok`-spine value and (above) the `Err`-spine
           value across the run boundary, not a fixed answer.")
  (input
    (do
      (type Ty TInt TBool)
      (type TErr CondNotBool)
      (type Exp (Num Int64) (If Exp Exp))
      (def (is-bool (: t Ty)) (match t ((TBool) true) ((TInt) false)))
      (def (check (: e Exp)) (match e ((Exp.Num _) (Result.Ok TInt)) ((Exp.If c t) (check-if c t))))
      (def
        (check-if (: c Exp) (: t Exp))
        (match
          (check c)
          ((Result.Ok ct)
            (if
              (is-bool ct)
              (match (check t) ((Result.Ok tt) (Result.Ok tt)) ((Result.Err et) (Result.Err et)))
              (Result.Err CondNotBool)))
          ((Result.Err ec) (Result.Err ec))))
      (def (main) (match (check (Exp.Num 5)) ((Result.Ok _) 1) ((Result.Err _) 0)))
      (export main)))
  (output (: 1 Int64))
  (live-objects 0))

; --- A rest-pattern head binder read inside an INLINED match-arg callee's scrutinee ------------
; The monomorphization pair above pins one face of the "re-parent / orphaned-binder" class: a match-arm
; payload binder whose scrutinee lies WITHIN a COPIED generic body was wrongly SHARED (read the orphaned
; original). This pins the FUSION face of the same class — the one an inliner + match-fusion triggers,
; not monomorphization. A rest-pattern HEAD binder `c` from `(list c .. t)` is referenced INSIDE a
; nested-match scrutinee `(match (at0 dp (- i c)) …)`, and that whole nested match is the ARGUMENT to a
; callee (`omin`) which MATCHES its own parameter — so inlining `omin` fuses the two matches and CLONES
; the arm carrying the nested match. The clone helper must COPY a payload binder OF the match being
; cloned (so it re-resolves against the branch scrutinee) but SHARE one whose scrutinee is OUTSIDE the
; clone — here `c` reads the ENCLOSING `(match cs …)` scrutinee, not the fused match, so it must be
; SHARED. Copying it fresh re-resolved it lexically against the orphaned clone → a spurious CDZ0101
; `unbound c` REJECT of a valid program on all backends (an operator-escalated inliner miscompile-class
; bug that also pushed authors toward the sentinel `-1` anti-pattern the fleet is eradicating). Fixed by
; threading the clone ROOT and applying the `is_within(scrutinee, clone_root)` test (the `beta_reduce`
; analogue) — the same within-vs-enclosing distinction as the F2 do-def and Finding-46 guard-desugar
; fixes. `f` is a coin-change-DP min fold: cs=(5 10), dp=(Some 0), i=1, best=None; c=5 and c=10 both fail
; the `(<= c i)` gate (5>1, 10>1) so `best` stays `None` through both cons steps → main's `None` arm → -1.
; A regression that re-orphans `c` reappears as a compile-time REJECT (no value), not a wrong value.
(case
  "a rest-pattern head binder read inside an inlined match-arg callee's scrutinee resolves"
  (doc
    "The FUSION face of the re-parent/orphaned-binder class (companion to the monomorphization-copy
           pair above). A rest-pattern head binder `c` from `(list c .. t)` is read inside a nested-match
           scrutinee `(match (at0 dp (- i c)) …)` that is the ARGUMENT to `omin`, a callee that matches its
           own parameter and gets INLINED — triggering a match-fusion that clones the arm carrying the
           nested match. `c`'s scrutinee is the ENCLOSING `(match cs …)`, OUTSIDE the cloned fused match, so
           the clone must SHARE `c` (copying it fresh re-resolves it against the orphaned clone → a spurious
           CDZ0101 `unbound c`). Fixed by the `is_within(scrutinee, clone_root)` test. This coin-DP min fold:
           cs=(5 10), i=1 — both coins exceed `i` so `best` stays `None` and main's `None` arm yields -1. A
           re-orphaning regression reappears as a REJECT, not a wrong value.")
  (input
    (do
      (def (at0 (: xs (List (Option Int64))) (: i Int64)) (Option.expect (List.at xs i) "x"))
      (def
        (omin (: a (Option Int64)) (: b (Option Int64)))
        (match a ((None _u) b) ((Some av) (match b ((None _u) a) ((Some bv) (if (< av bv) a b))))))
      (def
        (f (: cs (List Int64)) (: dp (List (Option Int64))) (: i Int64) (: best (Option Int64)))
        (match
          cs
          (#list() best)
          (#list(c (.. t))
            (f
              t
              dp
              i
              (if
                (<= c i)
                (omin
                  best
                  (match (at0 dp (- i c)) ((None _u) (None unit)) ((Some v) (Some (+ v 1)))))
                best)))))
      (def (main) (match (f #list(5 10) #list((Some 0)) 1 (None unit)) ((None _u) -1) ((Some r) r)))
      (export main)))
  (output (: -1 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

; --- A TAIL call runs in constant stack ---------------------------------------------------------
; A recursive call in TAIL position (the function's result is exactly that call) must reuse the
; caller's stack frame rather than pushing a new one — otherwise a tail-recursive loop over a RUNTIME
; count grows the wasm call stack one frame per iteration and TRAPS (stack exhausted) on a valid,
; finite input, which the emitted component must be able to complete. The cases above recurse over
; CONSTANT arguments (folded away at compile time, so no runtime frame is ever emitted); these run the
; SAME shapes over a `(call …)` runtime argument, where the self-call is a real emitted call. A
; tail-recursive accumulator counting a million down, and a mutually-tail-recursive even/odd at 100000,
; both complete in O(1) stack — the self-recursive and the cross-function (mutual) tail-call shapes.
(case
  "a tail-recursive accumulator over a large runtime count iterates in constant stack"
  (doc
    "`(def (f n acc) (if (= n 0) acc (f (- n 1) (+ acc 1))))` counted down from a runtime `n` =
           1000000, accumulating +1 each step. The self-call is in TAIL position (it is the `if`'s
           result), so it reuses the frame and the loop runs in constant stack, yielding 1000000. A
           frame-per-iteration recursive call would trap by stack exhaustion well before a million —
           the recorded outcome is the value, not a trap.")
  (input
    (do
      (def (f n acc) (if (= n 0) acc (f (- n 1) (+ acc 1))))
      (def (main (: n Int64)) (f n 0))
      (export main)))
  (call main (: 1000000 Int64))
  (output (: 1000000 Int64)))

(case
  "a NON-tail recursion 10000 deep computes through real stack frames"
  (doc
    "The non-tail counterpart of the constant-stack pins (which all convert to loops): `(+ n
           (sum-to (- n 1)))` keeps the `+` PENDING across every level, so 10000 genuine frames must
           coexist — no loop rewrite applies. 50005000. Pins the compiled stack budget at a depth a
           small default (or a frame far fatter than needed) would overflow; the runtime companion of
           the small-n sum-to fold pins. NOTE (pending v-core-opt/v-effects reconciliation on unpause):
           accumulator introduction now FIRES for this exact `(+ n (rec (- n 1)))` shape — empirically
           `sm(1000000)` completes in O(1) stack (see 'a linear non-tail sum is accumulator-transformed
           into a constant-stack loop' above), so the 'no loop rewrite applies / 10000 genuine frames'
           mechanism claim is superseded. The case still passes (50005000 is identical whether via
           frames or a loop) but no longer exercises a 10000-frame stack budget; the owner should
           re-base it on a genuinely non-transformable shape or retire it.")
  (input
    (do
      (def (sum-to (: n Int64)) (if (< n 1) 0 (+ n (sum-to (- n 1)))))
      (def (main (: n Int64)) (sum-to n))
      (export main)))
  (call main (: 10000 Int64))
  (output (: 50005000 Int64)))

(case
  "a NON-tail heap-spine build 5000 deep constructs and drains through real frames"
  (doc
    "The heap companion: `(Cons n (build (- n 1)))` is NON-tail (the constructor wraps the
           recursive result), so the build holds 5000 pending frames each owning a fresh heap node; the
           equally non-tail `(+ 1 (len t))` walk re-descends. Composes deep control stack with deep heap
           allocation in one program — a frame layout that spilled the pending Cons operand wrong at
           depth corrupts a node. 5000.")
  (input
    (do
      (type L (Nil) (Cons Int64 L))
      (def (build (: n Int64)) (if (< n 1) (Nil) (Cons n (build (- n 1)))))
      (def (len (: xs L)) (match xs ((Nil) 0) ((Cons h t) (+ 1 (len t)))))
      (def (main (: n Int64)) (len (build n)))
      (export main)))
  (call main (: 5000 Int64))
  (output (: 5000 Int64))
  (live-objects 0))

(case
  "a tail-recursive HEAP accumulator builds and folds a 10000-deep spine in constant stack"
  (doc
    "The heap twin of the scalar tail-accumulator above: `mk-tail` threads a RECURSIVE-SUM
           accumulator (`(S acc)` wraps the heap value one level per step) through its tail call, and
           `depth-tail` consumes the built spine with a tail-recursive scalar count — both loops at a
           runtime depth of 10000. The tail-loop conversion must handle an accumulator that is a heap
           HANDLE (dup/drop across the loop back-edge, not a scalar register), and the 10000-node spine
           must build and fold without frame growth. A frame-per-iteration emit or a leaked/dropped
           handle at the back-edge would trap or misdepth well before 10000. → 10000.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (mk-tail (: n Int64) (: acc Nat)) (if (= n 0) acc (mk-tail (- n 1) (S acc))))
      (def
        (depth-tail (: v Nat) (: acc Int64))
        (match v ((S rest) (depth-tail rest (+ acc 1))) ((Z u) acc)))
      (def (main (: a Int64)) (depth-tail (mk-tail a (Z)) 0))
      (export main)))
  (call main (: 10000 Int64))
  (output (: 10000 Int64))
  (live-objects 0))

; A recursive function with TWO OR MORE NARROW-WIDTH parameters (UInt8/Int8/UInt16/…) threading a narrow
; accumulator through the recursive call. A narrow value lives in an i32 machine slot (a wide Int64 is
; i64); a bare-literal argument (`(f n 0)` — the `0` for a UInt8 `acc`) defaults to Int64, so passing it
; unnormalized pushed an i64 into the i32 parameter slot and rcdzc emitted a STRUCTURALLY INVALID wasm
; module ("expected i32, found i64"). Every call argument must be grounded to its PARAMETER's machine
; width — the same narrow-normalization the operator/if-branch sites already apply, at the call boundary.
; A single narrow parameter and an Int64 two-parameter recursion both worked; the gap was a narrow value
; threaded as the 2nd+ recursive argument. A well-typed narrow-accumulator recursion must never emit
; invalid wasm.
(case
  "a narrow-width two-parameter recursion compiles to valid wasm and computes"
  (doc
    "`(def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) (+ acc 1))))` — a UInt8
           accumulator counting n down while adding 1 to acc. `f(10, 0)` = 10. The narrow `acc`'s
           bare-literal seed `0` (and each recursive `(+ acc 1)`) must be emitted at the parameter's i32
           width, not the default i64, or the call pushes a mismatched slot and the module fails wasm
           validation. The Int64 control above compiles at i64 slots; this pins the narrow width threads
           a recursive argument correctly. Expected: 10.")
  (input
    (do
      (def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) (+ acc 1))))
      (def (go (: n UInt8)) (f n 0))
      (export go)))
  (call go (: 10 UInt8))
  (output (: 10 UInt8)))

(case
  "a narrow-width accumulator that never changes threads through the recursion"
  (doc
    "The minimal narrow-threading shape: the accumulator is passed UNCHANGED — `(f (- n 1) acc)` —
           so the only narrow argument at the recursive call is the parameter `acc` itself (no `(+ acc
           1)` to widen it). `f(10, 0)` = 0 (acc starts 0, never incremented). Pins that even a bare
           narrow PARAMETER reference threaded as a recursive argument is emitted at its i32 slot, not
           widened to i64. Expected: 0.")
  (input
    (do
      (def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) acc)))
      (def (go (: n UInt8)) (f n 0))
      (export go)))
  (call go (: 10 UInt8))
  (output (: 0 UInt8)))

(case
  "a recursive call in TAIL position ascribed to a NARROWER int width wraps its result (valid wasm)"
  (doc
    "`main` TAIL-CALLS the recursive `v1` (result Int64 = i64) but ASCRIBES the result to `UInt32`
           (= i32), so `main`'s wasm result type is i32 while the callee returns i64. A `return_call` would
           return the callee's i64 DIRECTLY as main's i32 result — eliding the `i32.wrap_i64` the narrowing
           requires — producing INVALID wasm (`current function requires result type i32 but callee returns
           i64`; fuzzer 38551). The fix: a tail call whose callee result valtype differs from the enclosing
           function's result falls back to a non-tail `call` + the width conversion. `v1 3` recurses
           3->2->1->0 adding 127 per level = 508; ascribed UInt32 -> 508. Pins the
           narrow-int-ascription-over-a-recursive-tail-call class emits valid wasm and computes.")
  (input
    (do
      (def (v1 (: v2 Int64)) (if (<= v2 0) 127 (+ (v1 (- v2 1)) 127)))
      (def (main) (: (v1 3) UInt32))
      (export main)))
  (call main)
  (output (: 508 UInt32)))

(case
  "a recursive-call result ascribed to a wider int width is coerced when used as an arithmetic operand"
  (doc
    "The OPERAND-position sibling of the tail-call narrow-int case (fuzzer 38592 / note 38738). `v3`
           returns Int64 (= rust i64); its result is ascribed to `UInt64` (= u64) and then used as the LEFT
           operand of `+`. The ascription `(: (v3 2) UInt64)` is absorbed as type-only (no cast node), so the
           rust backend emitted the callee's i64 value directly into the u64 `+` — `(v3(..)).checked_add(3u64)`
           = `i64 .checked_add u64` → rustc E0308 mismatched types (the wasm backend, with a uniform i64
           machine width, ran it fine = a backend DIVERGENCE below the shared front-end). The fix coerces an
           arithmetic operand whose ACTUAL emitted int type (a `Core::Call`'s callee-result type) differs from
           the op's int type with an `as` cast — `((v3(..)) as u64).checked_add(3u64)`. `v3 2` recurses
           2->1->0 returning 5 at the base; `5 + 3 = 8`, ascribed UInt64 -> 8. Pins the
           ascription-over-a-recursive-call-as-a-binary-op-operand class emits well-typed rust (+ valid wasm).")
  (input
    (do
      (def (v3 (: v4 Int64)) (if (<= v4 0) 5 (v3 (- v4 1))))
      (def (main) (+ (: (v3 2) UInt64) 3))
      (export main)))
  (call main)
  (output (: 8 UInt64)))

(case
  "a mutually tail-recursive even/odd over a large runtime count iterates in constant stack"
  (doc
    "The cross-function shape: `even` and `odd` each end in a tail call to the OTHER. At a runtime
           depth of 100000 the alternating tail calls run in constant stack and yield 1 (100000 is
           even). A self-tail-call→loop optimization would not cover this — the tail calls cross between
           two functions — so this pins that a genuine cross-function tail call reuses the frame, not
           only direct self-recursion.")
  (input
    (do
      (def (even n) (if (= n 0) 1 (odd (- n 1))))
      (def (odd n) (if (= n 0) 0 (even (- n 1))))
      (def (main (: n Int64)) (even n))
      (export main)))
  (call main (: 100000 Int64))
  (output (: 1 Int64)))

(case
  "a self-recursive Bool-returning function whose recursive call is the then-branch"
  (doc
    "A self-recursive function that returns Bool, whose `if` body puts the recursive SELF-CALL in
           the THEN branch and a Bool literal in the ELSE — the `all …` / `every-so-far` shape a reader's
           name matcher takes (`(if (< i n) (if guard (recurse (+ i 1)) false) true)` = \"all positions
           satisfy the guard\"). `(go 0 3)` recurses to the base case and returns true. Pins that a
           recursive function's RETURN KIND settles to Bool regardless of whether the self-call (whose
           kind is a placeholder until the function's kind is known) is the then-branch or the else-branch:
           a Bool-literal sibling must pin the `if`'s result kind to Bool, so the result does not depend on
           branch ORDER. The mutually-recursive `even`/`odd` above already returns Bool, but there each
           branch is a Bool literal or the OTHER function's call; here the branch is the function's OWN
           call, which is the order-dependent kind-inference case (the Bool analogue of the recursive
           heap-accumulator kind race). The mirror shape — self-call in the ELSE, literal in the THEN —
           and an Int-returning self-recursive function both settle correctly; this pins the Bool + then
           combination that does not yet.")
  (input
    (do
      (def (go i n) (if (< i n) (go (+ i 1) n) true))
      (def (main) (if (go 0 3) 1 0))
      (export main)))
  (output (: 1 Int64)))

(case
  "several same-shaped recursive defs each resolve their own parameter"
  (doc
    "N recursive defs of IDENTICAL shape — `(sm_i n acc) = if n=0 then acc else sm_i(n-1, acc+i)` —
           each fold over their OWN parameter. Resolving which def owns a parameter must attribute each
           reference to the RIGHT def, never a same-shaped sibling; a mis-attribution would thread the
           wrong per-def increment. sm0..sm4 at (3, 0) each add their index three times, so the sum is
           3·(0+1+2+3+4) = 30. Distinct per-def answers prove no parameter is cross-attributed across the
           identical-shaped group.")
  (input
    (do
      (def (sm0 n acc) (if (= n 0) acc (sm0 (- n 1) (+ acc 0))))
      (def (sm1 n acc) (if (= n 0) acc (sm1 (- n 1) (+ acc 1))))
      (def (sm2 n acc) (if (= n 0) acc (sm2 (- n 1) (+ acc 2))))
      (def (sm3 n acc) (if (= n 0) acc (sm3 (- n 1) (+ acc 3))))
      (def (sm4 n acc) (if (= n 0) acc (sm4 (- n 1) (+ acc 4))))
      (def (main) (+ (sm0 3 0) (+ (sm1 3 0) (+ (sm2 3 0) (+ (sm3 3 0) (sm4 3 0))))))
      (export main)))
  (output (: 30 Int64)))

; A self-tail call (or any call) evaluates ALL its arguments onto the operand stack simultaneously — a
; parallel move into the parameter slots (the self-tail-loop back-edge) or the call's argument sequence.
; Each argument's scratch is live until the store, so sibling arguments must occupy DISJOINT scratch
; slots. A HEAP-scrutinee `match` argument (Option/List/sum) evaluates its non-reusable scrutinee into an
; i32 handle slot; an arithmetic argument's overflow guard uses an i64 slot. When both shared the same
; scratch `base`, one wasm local was `local.set` at two widths (i64 then i32) and rcdzc emitted a
; STRUCTURALLY INVALID module ("expected i32, found i64"). A match as an OPERAND worked (its i32 slot
; nested above the arith i64 slots), as did a scalar-scrutinee match (it reuses the param, claims no
; slot); the gap was a heap-match sitting DIRECTLY in a call/tail-call argument. Each argument's scratch
; must float above the running high-water — the same disjoint-slot discipline the checked-arith operands
; and the sum-match arms already apply — so a well-typed tail-recursive accumulate-a-matched-value never
; emits invalid wasm. Sibling of the narrow-two-parameter invalid-wasm regression above (both are a
; call-boundary machine-slot mismatch).
(case
  "a self-tail call passing a heap-match argument compiles to valid wasm"
  (doc
    "`(def (f n acc) (if (= n 0) acc (f (- n 1) (match (if (> n 0) (Some n) (None)) ((Some x) (+ acc
           x)) ((None) acc)))))` — a tail-recursive accumulator whose self-call's second argument is a
           `match` over a heap Option. `f(5, 0)` sums 5+4+3+2+1 = 15. rcdzc emitted a STRUCTURALLY INVALID
           wasm module: the self-tail-loop back-edge slot received the heap-match value at a width
           (i32 handle) colliding with the first argument's i64 arith-guard slot. The same shape with the
           match as an OPERAND `(+ acc (match …))` works (15) and a SCALAR-scrutinee match in the same
           argument works, so the machinery is right; the gap is a heap-scrutinee match in a self-tail-call
           argument. Expected: 15.")
  (input
    (do
      (def
        (f (: n Int64) (: acc Int64))
        (if
          (= n 0)
          acc
          (f (- n 1) (match (if (> n 0) (Some n) (None)) ((Some x) (+ acc x)) ((None) acc)))))
      (def (main) (f 5 0))
      (export main)))
  (call main)
  (output (: 15 Int64)))

(case
  "a non-tail call passing a heap-match argument compiles to valid wasm"
  (doc
    "The same scratch-slot collision on the ORDINARY (non-tail) call path: `g(a, m) = a + m`, called
           `(g (- 6 1) (match (Some 10) ((Some x) x) ((None) 0)))`. Argument 0 `(- 6 1)` claims an i64
           arith-guard slot; argument 1's heap-match scrutinee claims an i32 handle slot — they must be
           disjoint. 5 + 10 = 15. Companion of the self-tail-call case; pins that the disjoint-slot fix
           covers a plain call's argument sequence, not only the self-tail-loop back-edge.")
  (input
    (do
      (def (g (: a Int64) (: m Int64)) (+ a m))
      (def (main) (g (- 6 1) (match (Some 10) ((Some x) x) ((None) 0))))
      (export main)))
  (call main)
  (output (: 15 Int64)))

(case
  "a self-tail-recursive mixed match whose recursive arm returns an Option compiles to valid wasm"
  (doc
    "A DISTINCT tail-loop invalid-wasm shape from the accumulator/argument scratch cases above: a
           self-tail-recursive function whose innermost match is MIXED — a value-returning arm beside a
           recursive-tail arm — and whose recursive arm returns an OPTION-typed value (not a scalar
           accumulator). `twostep` pulls `(x, s2)` from `step(s)`; if `x > 2` it RETURNS `(Some (x, s2))`,
           else it TAIL-RECURSES `(twostep s2)`. Under the tail-loop conversion the recursive arm's `br`
           (loop-continue) left the enclosing `if (result i32)` block stack-unbalanced → `func N failed to
           validate: values remaining on stack at end of block`. A SCALAR-returning variant (arms yield
           bare Int64) compiled fine, so the trigger is the Option-typed result of a mixed-match tail arm,
           not the recursion. This is exactly v-iterators' filter-map shape (keep = return Some, drop =
           recurse). `(twostep [1,2,3,4])` skips 1,2 and returns `(Some (3, [4]))`; reading the first
           element of the pair = 3. Expected: 3.")
  (input
    (do
      (def
        (step (: xs (List Int64)))
        (match xs (#list() (None)) (#list(h (.. t)) (Some #tuple(h t)))))
      (def
        (twostep (: s (List Int64)))
        (match
          (step s)
          ((None) (None))
          ((Some pair) (match pair (#tuple(x s2) (if (> x 2) (Some #tuple(x s2)) (twostep s2)))))))
      (def (main) (match (twostep #list(1 2 3 4)) ((Some #tuple(y r)) y) ((None) 0)))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

; The GUARD companion of the Option-returning mixed-match tail case above. The prior cases that combine a
; `(guard …)` arm with a tail-recursive fall-through (03-equality: guarded-wildcard / literal-probe /
; sum-match arms driving a loop) all yield a SCALAR — and the Option-returning mixed-match tail case above
; uses a plain `if`, not a guard, to select its heap-typed result. This pins their INTERSECTION: a `match`
; whose KEPT arm is a `(guard (list h .. t) <cond>)` returning an OPTION-typed value (`(Some (tuple …))`)
; while its fall-through arm TAIL-RECURSES. The guard adds its own `if (result i32)` block nesting on top of
; the tail-loop conversion, so both the guard `if`'s fall-through and the recursive arm's `br` must leave
; the enclosing heap-result block stack-balanced — the same stack-balance discipline as the plain-`if`
; case, now under a guarded list-splat arm. Built from a RUNTIME `lim` so the guard cannot fold.
(case
  "a guarded list-splat arm returning an Option beside a tail-recursive arm compiles to valid wasm"
  (doc
    "`find` walks a list: its FIRST non-empty arm is a GUARDED splat `(guard (list h .. t) (> h lim))`
           that RETURNS `(Some (tuple h t))` when the head clears a RUNTIME threshold `lim`; otherwise the
           unguarded splat arm TAIL-RECURSES on the tail. This is the guard companion of the plain-`if`
           Option-returning mixed-match tail case above — the guard's `if (result i32)` block wraps a
           heap-typed (Option) result, so both the guard fall-through and the recursive tail `br` must keep
           the enclosing block stack-balanced or wasm rejects the module. Over `[1,2,3,4]` with `lim`
           runtime: `lim=0` keeps head 1 → 1; `lim=2` skips 1,2 keeps 3 → 3; `lim=3` skips to 4 → 4;
           `lim=9` keeps nothing → `(None)` → 0. Pins the guard×heap-return×tail-loop intersection.")
  (input
    (do
      (def
        (find (: xs (List Int64)) (: lim Int64))
        (match
          xs
          (#list() (None))
          ((guard #list(h (.. t)) (> h lim)) (Some #tuple(h t)))
          (#list(_ (.. t)) (find t lim))))
      (def (main (: lim Int64)) (match (find #list(1 2 3 4) lim) ((Some #tuple(y r)) y) ((None) 0)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

; A CONTROL-FLOW companion of the guarded-tail cases: the guard EXPRESSION itself calls a (non-tail)
; RECURSIVE helper. Every other guard×tail-loop pin uses a flat guard condition (a comparison or a
; `value-eq`); here the first arm's guard is `(= (sumto x) target)` where `sumto` is a self-recursive
; sum-to-`k`. This exercises the recursion machinery on BOTH axes at once: the guard subexpression drives
; `apply_lambda`'s recursion-decline → runtime-specialization for `sumto`, WHILE the enclosing `scan` match
; is itself a self-tail loop (fall-through arm tail-recurses). A miscompile in either — a guard evaluated
; against a stale loop-carried local, or the guard's own recursive call corrupting the driver's tail-loop
; slots — would surface here. Runtime `target` so nothing folds. `scan 0 target` climbs `x` until
; `sumto(x)` hits `target`: target=6 → sumto(3)=6 → x=3; target=10 → sumto(4)=10 → x=4; target=0 → x=0.
(case
  "a guard expression calling a recursive helper inside a self-tail-loop match compiles"
  (doc
    "`scan` is a self-tail-recursive driver whose FIRST arm is `(guard x (= (sumto x) target))` — the
           guard CONDITION calls a self-recursive helper `sumto` (sum 1..x) — and whose fall-through arm
           TAIL-RECURSES `(scan (+ n 1) target)`. This pins the guard×recursion×tail-loop corner distinct
           from the flat-condition guarded-tail pins: the guard subexpression exercises the recursive-lambda
           decline/runtime-specialization path while the enclosing match runs as a tail loop, and the two
           must not corrupt each other's slots. Built from a RUNTIME `target` so neither the guard call nor
           the loop folds. `scan 0 target` returns the least `x` with `sumto(x) == target`: target=6 → 3
           (1+2+3); target=10 → 4 (1+2+3+4); target=0 → 0. Expected (target=6): 3.")
  (input
    (do
      (def (sumto (: k Int64)) (if (<= k 0) 0 (+ k (sumto (- k 1)))))
      (def
        (scan (: n Int64) (: target Int64))
        (match n ((guard x (= (sumto x) target)) x) (_ (scan (+ n 1) target))))
      (def (main (: target Int64)) (scan 0 target))
      (export main)))
  (call main (: 6 Int64))
  (output (: 3 Int64)))

; Two more faces of the Option-returning mixed-match tail loop the scalar-tuple pin above cannot witness.
; FIRST: the Option payload is a HEAP STRING (not a scalar tuple) — the loop-continue `br` and the
; value-returning arm must balance a block whose result is a boxed heap handle, and the found element is
; then consumed by a String op after the loop exits (the payload must be a live, readable heap value, not
; a stale slot). SECOND: TWO such loops NESTED — an outer accumulator loop whose per-iteration work calls
; the inner seeking loop; both convert to wasm loops independently, and the inner loop's Option result
; feeds the outer loop's mixed match (its None ends the outer loop, its Some both accumulates and advances
; the outer state). A tail-loop conversion that leaked stack values across either boundary would fail to
; validate or misread the composed state.
; The guard-calls-a-recursive-helper case above exercises guard×recursion; these pin two more guard
; CONDITION compositions with the value world: a guard applying a LET-BOUND CLOSURE (the guard's
; condition is a first-class function application, not a direct call to a def), and a guard reading a
; HEAP list built OUTSIDE the match (the condition captures an enclosing heap binding and performs a
; fallible indexed read under the pattern machinery).
(case
  "a guard condition applies a let-bound closure"
  (doc
    "`(guard n (big? n))` where `big?` is a LET-bound `(fn (v) (> v 10))` — the guard's condition
           dispatches through a first-class closure value, not a def call: the closure handle is read
           from the enclosing let scope inside the guard's evaluation context and applied to the binder.
           x=15 passes the guard → 1; x=5 fails → the wildcard 0. Pins that guard evaluation composes
           with closure application (a guard lowering that only supported direct calls or inline
           comparisons would reject or mis-dispatch this). Expected: 1, 0.")
  (input
    (do
      (def
        (main (: x Int64))
        (let ((big? (fn ((: v Int64)) (> v 10)))) (match x ((guard n (big? n)) 1) (_ 0))))
      (export main)))
  (call main (: 15 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a guard condition reads a heap list bound outside the match"
  (doc
    "`(guard n (= (Option.expect (List.at xs n) …) 2))` — the guard indexes a HEAP list bound in
           the ENCLOSING let, using the arm's own binder as the index: the condition needs the live list
           handle (captured across the match boundary), a fallible List.at, an Option.expect unwrap, and
           an equality — all inside guard evaluation. n=1 finds element 2 → 1; n=0 finds 1 ≠ 2 → 0. Pins
           the guard's evaluation context sees enclosing heap bindings and composes with the fallible-read
           idiom (the shape a lookup-table-driven match arm takes). Expected: 1, 0.")
  (input
    (do
      (def
        (main (: x Int64))
        (let
          ((xs #list(1 2 3)))
          (match x ((guard n (= (Option.expect (List.at xs n) "oob") 2)) 1) (_ 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a mixed-match tail loop whose Option payload is a heap String finds and reads the element"
  (doc
    "`firstlong` walks a `(List String)` via `step`: the keep-arm returns `(Some x)` where `x` is a
           heap STRING pulled from the list, the drop-arm tail-recurses. Over `(list \"ab\" \"c\" \"abcd\"
           \"zz\")` the first element with byte-len > 2 is \"abcd\"; reading `String.byte-len` of the found
           element after the loop = 4. The heap-payload companion of the scalar `(Some (tuple x s2))` pin
           above: the mixed match's value arm carries a BOXED heap handle through the tail-loop conversion's
           result block, and the handle must still address the live string after the loop exits. Expected: 4.")
  (input
    (do
      (def
        (step (: xs (List String)))
        (match xs (#list() (None)) (#list(h (.. t)) (Some #tuple(h t)))))
      (def
        (firstlong (: s (List String)))
        (match
          (step s)
          ((None) (None))
          ((Some pair)
            (match pair (#tuple(x s2) (if (> (String.byte-len x) 2) (Some x) (firstlong s2)))))))
      (def
        (main (: n Int64))
        (match (firstlong #list("ab" "c" "abcd" "zz")) ((Some s) (String.byte-len s)) ((None) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "nested mixed-match tail loops compose — an outer accumulator loop driving an inner seeking loop"
  (doc
    "`inner` is the Option-returning mixed-match tail loop of the pin above (skip elements ≤ 2, return
           `(Some (tuple x s2))` at the first match); `outer` is a SECOND such loop whose per-iteration work
           CALLS `inner`: a `None` ends the outer loop (yielding the accumulator), a `Some` adds the found
           element and tail-recurses on the advanced state. Over `(list 1 3 2 4 5)` the inner loop finds
           3, then 4, then 5 (skipping 1 and 2) → 12. Both functions convert to wasm loops; the inner
           loop's Option result must arrive intact as the outer loop's scrutinee each iteration (no stack
           leakage across the composed loop boundaries), and the outer loop's accumulator threads through
           its own recursive arm. Expected: 12.")
  (input
    (do
      (def
        (step (: xs (List Int64)))
        (match xs (#list() (None)) (#list(h (.. t)) (Some #tuple(h t)))))
      (def
        (inner (: s (List Int64)))
        (match
          (step s)
          ((None) (None))
          ((Some pair) (match pair (#tuple(x s2) (if (> x 2) (Some #tuple(x s2)) (inner s2)))))))
      (def
        (outer (: s (List Int64)) (: acc Int64))
        (match
          (inner s)
          ((None) acc)
          ((Some pair) (match pair (#tuple(x s2) (outer s2 (+ acc x)))))))
      (def (main (: n Int64)) (outer #list(1 3 2 4 5) 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 12 Int64))
  (live-objects known-leak))

; The same i32/i64 scratch-slot-aliasing family at a HIGHER local count, in a decode-loop shape the
; self-hosted compiler's reader is written in: a self-tail loop whose position advance projects BOTH
; fields of a tuple returned by a recursive helper, accumulating compound-payload sum nodes into a list.
; Over enough locals the loop function reused one slot for an i64 arithmetic temp AND an i32 heap handle
; (an invalid module, `expected i32 found i64`). Root-caused to the same slot-reservation weakness as the
; let-bound if-compound miscompile (a persistent slot must be reserved BEFORE the sub-expressions that
; float their scratch off the high-water) — the fix there cleared this too. A now-passing regression guard.
(case
  "a self-tail loop advancing by a tuple projection while accumulating compound-sum nodes compiles"
  (doc
    "A decode loop `read-leaves` advances its position via `leaf-end`, which projects BOTH fields of
           the tuple returned by the recursive `read-varu` (`(+ (. v 1) (. v 0))`), and pushes `Ast` sum
           nodes (a type with a `(List Ast)` variant — a compound payload) into a `(List Ast)` accumulator.
           Over `b\"\\x00\\x01\\x05\"` it reads ONE leaf, an `(Ast.Int …)`, and `nc` of an `Ast.Int` is 1.
           This emitted INVALID WASM (`expected i32, found i64`) — a threshold-dependent slot-aliasing bug
           in the loop transform (one local held both an i64 arithmetic temp and the i32 handle from
           `read-varu`), the same scratch-slot family as the let-bound if-compound miscompile; the
           slot-reservation fix cleared both. Expected: 1.")
  (input
    (do
      (type Ast (Int Int64) (List (List Ast)))
      (def
        (read-varu (: b Bytes) (: p Int64) (: a Int64) (: s Int64))
        (let
          ((byte (Option.expect (Bytes.at b p) "v")))
          (let
            ((a2 (+ a (<< (& byte 127) s))))
            (if (= (& byte 128) 0) #tuple(a2 (+ p 1)) (read-varu b (+ p 1) a2 (+ s 7))))))
      (def
        (read-mag (: b Bytes) (: p Int64) (: len Int64) (: acc Int64))
        (if
          (= len 0)
          acc
          (read-mag b (+ p 1) (- len 1) (+ (* acc 256) (Option.expect (Bytes.at b p) "m")))))
      (def
        (read-leaf (: b Bytes) (: pos Int64))
        (Ast.Int (read-mag b (+ pos 1) (. (read-varu b (+ pos 1) 0 0) 0) 0)))
      (def
        (leaf-end (: b Bytes) (: pos Int64))
        (let ((v (read-varu b (+ pos 1) 0 0))) (+ (. v 1) (. v 0))))
      (def
        (read-leaves (: b Bytes) (: pos Int64) (: count Int64) (: acc (List Ast)))
        (if
          (= count 0)
          acc
          (read-leaves b (leaf-end b pos) (- count 1) (List.push acc (read-leaf b pos)))))
      (def (nc (: n Ast)) (match n ((Ast.Int _) 1) ((Ast.List _) 9)))
      (def (main) (nc (Option.expect (List.at (read-leaves b"\x00\x01\x05" 0 1 #list()) 0) "at")))
      (export main)))
  (output (: 1 Int64))
  (live-objects known-leak))

; A further residual of the SAME i32/i64 slot family, on the `if`-BRANCH axis: a self-recursive function
; CARRYING a heap collection whose BASE arm materializes a fallible-read Option HANDLE. The two `if`
; branches are mutually exclusive, so the emit reused one scratch slot index across them — but the base
; arm wants it as an i32 Option handle (from `Bytes.at`/`List.at`) while the recursive arm's `(- n 1)`
; wants it as an i64 temp; a slot's type is recorded ONCE, so the local was declared at one width and used
; at the other (`expected i32, found i64`). The fix starts the ELSE branch's scratch above the THEN
; branch's high-water (disjoint by width), like the tuple/list examples above. These pin both the Bytes
; and the List carry, and both the `Option.expect` and the raw `match` fallible read.
(case
  "a collection-carrying recursion whose base arm does a fallible indexed read compiles"
  (doc
    "A self-recursive `loop` carries a `Bytes` parameter and at its base arm (n=0) performs a FALLIBLE
           read `(Option.expect (Bytes.at b p) …)` — byte `p` of `b\"\\x05\"` at p=0 = 5. This emitted
           INVALID WASM (`expected i32, found i64`): the base arm's i32 Option handle and the recursive
           arm's i64 `(- n 1)` temp collided on one scratch slot (the two mutually-exclusive `if` branches
           shared a slot index recorded at a single width). The SAME fallible read WITHOUT the recursion
           compiles and runs to 5, and a scalar `(Bytes.len b)` base (no handle) also compiles — pinning the
           trigger to a handle-materializing base arm of a collection-carrying recursion. Fix: the else
           branch's scratch starts above the then branch's high-water, so the i32 handle stays disjoint from
           the i64 temps. Expected: 5.")
  (input
    (do
      (def
        (loop (: b Bytes) (: p Int64) (: n Int64))
        (if (= n 0) (Option.expect (Bytes.at b p) "v") (loop b p (- n 1))))
      (def (main (: p Int64)) (loop b"\x05" p 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a list-carrying recursion whose base arm does a fallible indexed read compiles"
  (doc
    "The LIST companion of the Bytes case above — the fault is not Bytes-specific. A self-recursive
           `loop` carries a `(List Int64)` and its base arm reads `(Option.expect (List.at xs i) …)`. With
           `xs`=(list 7 8 9) and i=1 the base yields 8. Same slot collision (i32 List-element Option handle
           vs i64 recursion temp), same fix. Expected: 8.")
  (input
    (do
      (def
        (loop (: xs (List Int64)) (: i Int64) (: n Int64))
        (if (= n 0) (Option.expect (List.at xs i) "v") (loop xs i (- n 1))))
      (def (main (: i Int64)) (loop #list(7 8 9) i 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 8 Int64))
  (live-objects 0))

(case
  "a collection-carrying recursion whose base arm reads via a raw match compiles"
  (doc
    "The raw-`match` face of the fallible-base collection-recursion above (companion to the two
           Option.expect cases): the base arm materializes the i32 Option handle via a plain `(match
           (Bytes.at b p) ((Some x) x) ((None) -1))` rather than `Option.expect` — a distinct arm body
           (returns -1 on None instead of trapping), but the SAME i32-handle-vs-i64-recursion-temp scratch
           collision, so it too must emit valid wasm. byte 0 of `b\"\\x05\"` = 5.")
  (input
    (do
      (def
        (loop (: b Bytes) (: p Int64) (: n Int64))
        (if (= n 0) (match (Bytes.at b p) ((Some x) x) ((None) -1)) (loop b p (- n 1))))
      (def (main (: p Int64)) (loop b"\x05" p 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "functions are single-arity and curried"
  (doc
    "Witnesses core-semantics.md §Functions Are Single-Arity: a function takes exactly one
           argument. Multi-parameter syntax (fn (x y) body) desugars to (fn x (fn y body)). Partial
           application is natural: applying a two-param function to one argument returns a closure.")
  (input (let ((add (fn (x y) (+ x y)))) (let ((add3 (add 3))) (add3 7))))
  (output (: 10 Int64)))

(case
  "multi-argument application is curried application"
  (doc
    "Witnesses core-semantics.md §Functions Are Single-Arity: application (f a b) desugars
           to ((f a) b). Each application passes one argument; the result of (f a) is a closure
           that accepts b.")
  (input ((fn (x y) (+ x y)) 2 3))
  (output (: 5 Int64)))

(case
  "a curried function can be partially applied"
  (doc
    "Witnesses core-semantics.md §Functions Are Single-Arity: since functions are single-arity
           and multi-param is sugar for currying, partial application works naturally. map-inc applies
           inc to each element — inc is (add 1), a partial application of add.")
  (input (let ((add (fn (x y) (+ x y)))) (let ((inc (add 1))) (inc 41))))
  (output (: 42 Int64)))

(case
  "a named multi-argument function applies in explicit curried form"
  (doc
    "Witnesses core-semantics.md §Functions Are Single-Arity (\"Multi-argument application (f a b)
           MUST desugar to curried application ((f a) b)\"): `(add 3 4)` and `((add 3) 4)` are the SAME
           program by that desugaring, so both must yield 7. This pins the rule for a NAMED def (the
           cases above use lambda values); a def is single-arity and curried just like a lambda, so
           `(add 3)` is a closure `((add 3) 4)` then applies.")
  (input (do (def (add x y) (+ x y)) (def (main) ((add 3) 4)) (export main)))
  (output (: 7 Int64)))

; Partial application to a VARIABLE reference (a runtime parameter, a let-bound value) must CAPTURE it in
; the residual (partially-applied) lambda — the primary use of currying: fixing a function's first
; argument to a runtime value. `((sub n) 3)` curries to a residual `(fn (b) (- n b))` whose body
; references `n`; `n`'s binding (the caller's parameter/`let`) must be carried into the residual's scope
; (closed over), exactly as the non-partial `(sub n 3)` has `n` in scope. A currying copy that substitutes
; the name occurrence WITHOUT capturing its binding leaves `n` unbound (CDZ0101). A CONSTANT capture (`(add
; 3)` above) has no free variable to capture and already worked; these pin the variable-reference case.
(case
  "partial application captures a runtime parameter in the residual lambda"
  (doc
    "`(sub a b)` = a − b. Partially applying it to a runtime PARAMETER — `((sub n) 3)` with `n` a
           parameter — curries to a residual lambda that CAPTURES `n`, then subtracts: `n` = 10 gives
           `(sub 10 3)` = 7. The residual body references `n`, so `n`'s binding is carried into the
           residual's scope (closed over), exactly as the non-partial `(sub n 3)`. Was CDZ0101 'unbound
           name n' — the currying copy substituted the name occurrence without capturing its binding.")
  (input (do (def (sub a b) (- a b)) (def (main (: n Int64)) ((sub n) 3)) (export main)))
  (call main (: 10 Int64))
  (output (: 7 Int64)))

(case
  "a TWO-of-THREE partial application binds a prefix and the LET-BOUND residual completes"
  (doc
    "The multi-arg prefix face (the pins above partially apply ONE of two args): `add3` takes three
           parameters, `(add3 x 10)` binds the first TWO — one a runtime parameter, one a literal — and the
           residual one-param closure is LET-BOUND before its completing application `(add-x 5)` = x+10+5 =
           115 at x=100. The residual must capture BOTH prefix arguments (the runtime x and the 10) in one
           environment and survive the let binding — the config-first-then-apply idiom (fix a function's
           settings, hand the residual around).")
  (input
    (do
      (def (add3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
      (def (main (: x Int64)) (let ((add-x (add3 x 10))) (add-x 5)))
      (export main)))
  (call main (: 100 Int64))
  (output (: 115 Int64)))

(case
  "partial application captures a let-bound value in the residual lambda"
  (doc
    "The let-binding companion: `(let ((m 10)) ((sub m) 3))` partially applies `sub` to the
           let-bound `m`, currying to a residual lambda that captures `m` = 10, so `(sub 10 3)` = 7. Pins
           that the captured argument may be any in-scope binding (a `let` name, not only a parameter or a
           constant) — the residual closes over it.")
  (input (do (def (sub a b) (- a b)) (def (main) (let ((m 10)) ((sub m) 3))) (export main)))
  (output (: 7 Int64)))

(case
  "a partially-applied RUNTIME closure binds its residual to a let, then completes it"
  (doc
    "The residual-closure-lift capability: a FACTORY `mk` RETURNS a runtime closure value `(fn (a b)
           (+ (+ a b) k))`; `(f 3)` partially applies THAT RUNTIME CLOSURE to 1 of its 2 args, producing a
           residual that captures the supplied `3` (and, transitively, the factory's `k`); the residual is
           BOUND to `g` in a `let`, then COMPLETED by `(g 4)`. `(mk n)` with n = 10 → `(3 + 4 + 10)` = 17.
           Unlike `((sub m) 3)` above (a partial of a top-level DEF), `f` here is a genuine RUNTIME CLOSURE
           VALUE, so `(f 3)` is an UNDER-ARITY `CallClosure` that must build a residual closure cell (capturing
           the supplied arg) and complete it via `call_indirect`. This partial-of-a-runtime-closure emitted
           INVALID WASM before the residual-closure lift landed (the under-arity `CallClosure`'s residual rep
           disagreed with the later completing `call_indirect` — `expected i64 found i32`); a stopgap DECLINE
           guarded it until the genuine lift replaced it. Pins that a let-bound residual from a partially-
           applied runtime closure now COMPUTES the correct value (was the decline-stopgap's exact repro).")
  (input
    (do
      (def (mk (: k Int64)) (fn ((: a Int64) (: b Int64)) (+ (+ a b) k)))
      (def (go (: n Int64)) (let ((f (mk n))) (let ((g (f 3))) (g 4))))
      (def (main) (go 10))
      (export main)))
  (output (: 17 Int64)))

(case
  "a named multi-argument function applies to all its arguments at once"
  (doc
    "The DIRECT multi-argument application `(add a b)` — not the explicit curried `((add a) b)` of
           the case above — of a named two-parameter def, at a module entrypoint. `(add2 20 22)` = 42.
           By §Functions Are Single-Arity these are the same program (`(f a b)` desugars to `((f a) b)`),
           but the direct form is the surface shape a program (and a self-hosted compiler reading a call
           node with an argument list) actually writes, and it exercises the N-ary-call lowering — the
           arguments read into an argument list, then pushed left-to-right before the `call` (wasm's
           calling convention) — rather than the nested single-application form. The three-argument
           companion `(add3 10 20 12) = 42` pins that an arbitrary arity, not just two, applies at once.")
  (input
    (do
      (def (add2 a b) (+ a b))
      (def (add3 a b c) (+ a (+ b c)))
      (def (main) (+ (add2 20 22) (- (add3 10 20 12) 42)))
      (export main)))
  (output (: 42 Int64)))

(case
  "the module entrypoint is the def named main regardless of its position"
  (doc
    "The module entrypoint is the def NAMED `main` — its position among the defs does not matter.
           Here `main` is the FIRST def and calls a helper `f` DEFINED AFTER it: `(def (main) (f 41))`
           then `(def (f x) (+ x 1))`, so f(41) = 42. This pins two things at once: a forward reference
           (a call to a def that appears later in source order resolves) and, more pointedly, that entry
           selection is by NAME, not by position — the companion cases in this file all place `main`
           last, so nothing else pins that a main-first module has the same entry. A compiler that
           instead took the FIRST def as the nullary entry would lift the parameter-taking `f` as the
           entry and miscompile (or must decline); selecting `main` by name reorders it to the entry
           slot no matter where it sits. The call itself is the ordinary N-ary call lowering — the
           argument `41` pushed before the `call` — exercised across the forward edge.")
  (input (do (def (main) (f 41)) (def (f x) (+ x 1)) (export main)))
  (output (: 42 Int64)))

(case
  "a named function is partially applied, bound, and used"
  (doc
    "core-semantics.md §Functions Are Single-Arity: partial application is natural for a named
           def too — `(add 3)` returns a closure awaiting the second argument, bound to `inc` and then
           applied to 4, yielding 7. The lambda form of this already holds; a named def must behave
           identically since multi-param defs desugar to curried single-arity functions.")
  (input (do (def (add x y) (+ x y)) (def (main) (let ((inc (add 3))) (inc 4))) (export main)))
  (output (: 7 Int64)))

; --- A function's result type is not restricted to Int64 --------------------------------
; core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments: a function's
; result is whatever value its body evaluates to. A predicate returns Bool; nothing in the
; semantics restricts a `def`'s return type to integers. These call a non-Int64-returning
; function and observe its result AS the program's result — the value must cross the run
; boundary faithfully (a Bool run returns a Bool). The point is well-formed programs: each
; must produce its recorded value, never an unrunnable artifact. (Contrast the cases above
; where a Bool result is consumed internally by `if`/`=`; here it is the program's result.)
(case
  "a function returning a boolean predicate result"
  (doc
    "`is-zero` is an ordinary predicate: it returns the Bool `(= n 0)`. Calling it from
           `main` yields that Bool as the program's result. is-zero(0) = true.")
  (input (do (def (is-zero n) (= n 0)) (def (main) (is-zero 0)) (export main)))
  (output (: true Bool)))

(case
  "a boolean-returning function called with a false result"
  (doc
    "The companion to the case above: is-zero(5) = false. Confirms the Bool result is carried
           faithfully across the run boundary for both truth values, not coerced or truncated.")
  (input (do (def (is-zero n) (= n 0)) (def (main) (is-zero 5)) (export main)))
  (output (: false Bool)))

(case
  "a comparison-predicate function returns its boolean result"
  (doc
    "core-semantics.md §Ordering Where Offered Is Total, as a function result: `lt5` returns
           `(< n 5)`. lt5(3) = true. A comparison predicate is the most common Bool-returning
           helper a compiler writes (bounds checks, dispatch guards).")
  (input (do (def (lt5 n) (< n 5)) (def (main) (lt5 3)) (export main)))
  (output (: true Bool)))

(case
  "a boolean result threaded through a second function"
  (doc
    "core-semantics.md §A Function Is A First-Class Value: `b` forwards `a`'s Bool result, and
           `main` returns `b`'s. The Bool return type propagates through the call chain; b(1) = false.")
  (input (do (def (a n) (= n 0)) (def (b n) (a n)) (def (main) (b 1)) (export main)))
  (output (: false Bool)))

(case
  "a boolean result propagates through a three-deep chain of forwarding functions"
  (doc
    "core-semantics.md §A Function Is A First-Class Value, one level deeper than the two-function
           case above: `a` forwards `b`'s result, `b` forwards `c`'s, and `c` is the only function with
           a directly Bool body (`(= n 0)`). So a's and b's return types are Bool only TRANSITIVELY —
           neither has a Bool-shaped body; each just returns a call whose callee's return type must
           already be known. Determining every function's result type is therefore a FIXPOINT over the
           call graph, not a single pass: the first pass learns `c` returns Bool, the second propagates
           that to `b`, the third to `a` and `main`. A single-pass return-type computation (enough for
           the two-function case, where one propagation step suffices) leaves `a`/`b` unresolved — and a
           compiler that defaults an unresolved function result to the integer type would give `a` and
           `b` mismatched result kinds versus the `i32`/Bool value they actually forward. a(0) = true.
           This pins that result-type resolution iterates to convergence across an arbitrary-depth chain,
           the companion of the two-deep case and of the recursive Bool cases earlier in this file.")
  (input
    (do (def (main) (a 0)) (def (a n) (b n)) (def (b n) (c n)) (def (c n) (= n 0)) (export main)))
  (output (: true Bool)))

(case
  "a boolean function result bound by let is still a boolean"
  (doc
    "core-semantics.md §Binding Is Lexical: binding a predicate's result to a name and
           returning that name does not change its type. The program's result is the Bool true.")
  (input (do (def (is-zero n) (= n 0)) (def (main) (let ((r (is-zero 0))) r)) (export main)))
  (output (: true Bool)))

; --- A function's PARAMETER type is not restricted to Int64 -----------------------------
; core-semantics.md §Applying A Function Binds Its Parameters To Its Arguments: a parameter is
; bound to whatever argument value it is applied to — a Bool or a Float just as well as an
; Int64. Nothing in the semantics restricts a `def`'s parameter to integers. These pass a
; non-Int64 argument to a user function and observe the ordinary result. (Companion to the
; result-type cases above; together they say a function is polymorphic in neither direction
; artificially — the seed must handle a Bool/Float on both sides of a call.)
(case
  "a function takes a boolean parameter and branches on it"
  (doc
    "`f` takes a Bool `b` and returns 10 or 20 via `if`. Applying it to `true` binds b=true,
           selecting the then-branch: f(true) = 10. The parameter is a Bool, not an Int64.")
  (input (do (def (f b) (if b 10 20)) (def (main) (f true)) (export main)))
  (output (: 10 Int64)))

(case
  "a boolean-parameter function applied to false"
  (doc
    "The companion of the case above: f(false) = 20. Confirms both Bool argument values are
           bound and dispatched correctly through a call.")
  (input (do (def (f b) (if b 10 20)) (def (main) (f false)) (export main)))
  (output (: 20 Int64)))

(case
  "a boolean parameter forwarded to a conditional result"
  (doc
    "core-semantics.md §A Function Is A First-Class Value: `both` takes two Bools and returns
           `b` when `a` is true, else false — a logical AND. both(true, true) = true. Exercises two
           Bool parameters in one signature, curried.")
  (input (do (def (both a b) (if a b false)) (def (main) (both true true)) (export main)))
  (output (: true Bool)))

; --- A parameter whose type the body does not constrain is polymorphic -------------------
; The cases above pin a parameter's type via a use in the body (`(if b …)` forces Bool). The
; identity function `(def (id x) x)` uses `x` only by returning it, so nothing in the body
; constrains its type: `id` is polymorphic (∀a. a → a) and applies to a value of ANY type,
; returning it unchanged (core-semantics.md §Applying A Function Binds Its Parameters To Its
; Arguments — the parameter is bound to whatever argument it is applied to; type-system.md
; §Inference — an unconstrained parameter generalizes to a type variable rather than defaulting
; to Int64). Inference realizes this: an unconstrained parameter generalizes to a type variable,
; so `id : ∀a. a → a` accepts `(id 42)` AND `(id true)`, each application instantiating `a` at
; its argument's type. These pin the polymorphic case; the Int64 companion is the control.
(case
  "the identity function applied to an integer returns the integer"
  (doc
    "The control: `(def (id x) x)` applied to an Int64 returns it. id(42) = 42. The body does
           not constrain `x`'s type; applying to an integer determines it here.")
  (input (do (def (id x) x) (def (main) (id 42)) (export main)))
  (output (: 42 Int64)))

(case
  "the identity function applied to a boolean returns the boolean"
  (doc
    "The polymorphic case: the same `(def (id x) x)` applied to a Bool returns the Bool.
           id(true) = true. Nothing in `id`'s body restricts `x` to Int64 — it is returned
           unchanged — so `id` is polymorphic and accepts a Bool argument exactly as it accepts an
           Int64. Inference generalizes the unconstrained parameter to a type variable (`id : ∀a. a →
           a`), so both `(id 42)` and `(id true)` type-check, each application instantiating `a` at its
           argument's type.")
  (input (do (def (id x) x) (def (main) (id true)) (export main)))
  (output (: true Bool)))

(case
  "a parameter passed to a polymorphic callee is not over-constrained"
  (doc
    "The argument-position constraint is PRECISE — it fires only when the callee's k-th parameter is
           DETERMINED. `g`'s param `v` is passed to the polymorphic `id` (whose param is unconstrained),
           so `g` gets NO spurious constraint from that call and stays usable at any type: `(+ (g 3) (g
           4))` = 7, `g`'s param inlining from each concrete argument. A generation that constrained `g`'s
           param from the `id` call would pin `g` to one type and reject (or miscompile) the second use.")
  (input (do (def (id x) x) (def (g v) (id v)) (def (main) (+ (g 3) (g 4))) (export main)))
  (output (: 7 Int64)))

(case
  "one generic identity instantiated at a scalar, a heap string, and a compound in one program"
  (doc
    "The three-representation stress of the id pins above (each instantiates at ONE type): the SAME
           `id` applied to a runtime Int64 (scalar), a String literal (heap rope handle), and a tuple
           (compound) in one body — n + byte-len(\"abc\") + (1+2) = 36+3+3 = 42. The three
           monomorphizations carry values of DIFFERENT machine representation (i64 / heap handle /
           multi-slot compound) through the same source def; a lowering that shared one specialized copy
           across representations (or mis-slotted the compound's fields through the pass-through) would
           corrupt one of the three reads.")
  (input
    (do
      (def (id x) x)
      (def
        (main (: n Int64))
        (+ (id n) (+ (String.byte-len (id "abc")) (match (id #tuple(1 2)) (#tuple(a b) (+ a b))))))
      (export main)))
  (call main (: 36 Int64))
  (output (: 42 Int64)))

(case
  "a generic pair helper returns a mixed-type tuple projected through a match"
  (doc
    "`(def (pair a b) (tuple a b))` is generic in both element types; `(pair 3 true)` instantiates it
           at (Int64, Bool), and the match projects the pair, using the Bool `y` to select the Int64 `x` →
           3. Pins that generic-result inference produces a sound MIXED-type tuple whose elements are read
           back at their distinct types.")
  (input
    (do
      (def (pair a b) #tuple(a b))
      (def (main) (match (pair 3 true) (#tuple(x y) (if y x 0))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a generic result flows into a second generic call"
  (doc
    "`(id (wrap 7))` composes two generic functions: `wrap` builds `(Some 7)`, and `id` returns it
           unchanged, its result type inferred as `Option Int64` from the argument; the outer match then
           unwraps → 7. Pins that a generic function's inferred result flows soundly into another generic
           call (compose), not just a single instantiation.")
  (input
    (do
      (def (id x) x)
      (def (wrap y) (Some y))
      (def (main) (match (id (wrap 7)) ((Some v) v) ((None _u) 0)))
      (export main)))
  (output (: 7 Int64)))

(case
  "a nested Option is double-matched to its inner value"
  (doc
    "`(Some (Some 9))` has inferred type `Option (Option Int64)` (no annotation); matching the outer
           `Some` binds `inner = (Some 9)`, and a second match unwraps it → 9. Pins that inference builds
           the nested Option and both match levels read through to the innermost value.")
  (input
    (do
      (def
        (main)
        (match
          (Some (Some 9))
          ((Some inner) (match inner ((Some v) v) ((None _u) 0)))
          ((None _u) -1)))
      (export main)))
  (output (: 9 Int64)))

(case
  "a generic pair-swap crosses a scalar and a heap payload between positions"
  (doc
    "The MIXED-representation structural generic: `swap (tuple n \"x\")` puts a runtime scalar and a
           heap string in one tuple and swaps them — position 0 (was i64) now holds the rope handle,
           position 1 (was handle) now holds the i64. Reading both back (`m + byte-len s` = 41+1 = 42)
           witnesses that the swap moved the VALUES, not just retyped the slots: a lowering with
           positional physical slots typed by the INPUT tuple would put a handle in an i64 slot or vice
           versa. The identity pins pass one value THROUGH; this permutes two differently-represented
           values WITHIN a compound.")
  (input
    (do
      (def (swap p) (match p (#tuple(a b) #tuple(b a))))
      (def (main (: n Int64)) (match (swap #tuple(n "x")) (#tuple(s m) (+ m (String.byte-len s)))))
      (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64)))

(case
  "a generic fold instantiated at an Int64 AND a String accumulator in one program"
  (doc
    "The accumulator-representation face of the monomorphized fold (the closure-arg fold pins below
           vary the closure's ARGUMENT types; here the ACCUMULATOR type differs per instantiation): one
           `fold-list` runs with an Int64 accumulator (n+1+2+3) AND a String accumulator (concat over
           \"ab\",\"cd\" → byte-len 4) in one program — 32+6+4 = 42. The two specialized copies carry the
           accumulator in different representations (i64 vs rope handle) through the same recursive
           spine; a shared copy would mis-carry one accumulator across the recursive call.")
  (input
    (do
      (def
        (fold-list f acc xs)
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f acc h) t))))
      (def
        (main (: n Int64))
        (+
          (fold-list (fn ((: a Int64)) (fn ((: x Int64)) (+ a x))) n #list(1 2 3))
          (String.byte-len
            (fold-list
              (fn ((: a String)) (fn ((: x String)) (String.concat a x)))
              ""
              #list("ab" "cd")))))
      (export main)))
  (call main (: 32 Int64))
  (output (: 42 Int64))
  (live-objects known-leak))

; --- A bare parameter PROJECTED in the body is constrained only at the call site ------------------
; A companion of the polymorphic-parameter cases above, for a STRUCTURAL use: a bare (unannotated)
; parameter that the body PROJECTS — `(. r field)` / `(. t N)` — is unconstrained in the standalone
; body (its type is `Any` until the def inlines), exactly as an arithmetic use `(+ r 1)` leaves it
; `Any`. A non-recursive def inlines at its call site, so the projection's real check runs THERE,
; where the argument's compound type flows in — the same way the identity function's parameter type is
; determined by the argument. Earlier the seed rejected the body standalone with a self-contradictory
; CDZ0201 "requires a record/tuple, found Any" (an `Any` operand is unconstrained, not a proven
; non-compound), spuriously failing a well-typed helper; arithmetic on an `Any` parameter never
; faulted, so projection was the outlier. A genuinely non-compound argument (an Int64) is still
; rejected at the call site (the reduced body projects a non-record) — the check is deferred, not
; dropped.
(case
  "a helper projects a record parameter constrained by its argument"
  (doc
    "`(def (get-x r) (. r x))` reads field `x` of its bare parameter `r`. `r` is unconstrained in
           the body (typed `Any` — nothing pins it until `get-x` inlines), so the field read is NOT a
           fault there; the argument `(mk v)` is a runtime `(record (x v) (y 2))`, so at the call site
           `r` is that record and `(. r x)` is `v`. With v=41 the result is 41. Pins that a bare
           parameter projected in the body types like an arithmetic use of it — constrained at the call
           site, not spuriously rejected standalone.")
  (input
    (do
      (def (get-x r) r.x)
      (def (mk n) #record((= x n) (= y 2)))
      (def (main (: v Int64)) (get-x (mk v)))
      (export main)))
  (call main (: 41 Int64))
  (output (: 41 Int64)))

(case
  "a helper projects a tuple parameter constrained by its argument"
  (doc
    "The tuple companion: `(def (fst t) (. t 0))` projects element 0 of its bare parameter. `t` is
           unconstrained in the body; the argument `(mk v)` is a runtime `(tuple v 2)`, so `(. t 0)` is
           `v`. With v=9 the result is 9. The positional analogue of the record helper — a bare
           parameter projected by position is likewise constrained at the call site.")
  (input
    (do
      (def (fst t) (. t 0))
      (def (mk n) #tuple(n 2))
      (def (main (: v Int64)) (fst (mk v)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "a helper sums two fields of a record parameter"
  (doc
    "The body uses the parameter's fields in ARITHMETIC: `(+ (. r x) (. r y))`. Both field reads
           are on the unconstrained `r`, and both feed `+`; at the call site `r` is `(record (x v) (y
           2))`, so the sum is v+2. With v=7 the result is 9. Pins that MULTIPLE projections of one bare
           compound parameter all resolve at the call site and compose with arithmetic on the results.")
  (input
    (do
      (def (sum-xy r) (+ r.x r.y))
      (def (mk n) #record((= x n) (= y 2)))
      (def (main (: v Int64)) (sum-xy (mk v)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 9 Int64)))

(case
  "projecting a field of a non-compound argument is rejected at the call site"
  (doc
    "The deferral is not a drop: `(def (get-x r) (. r x))` is well-formed standalone (its `r` is
           unconstrained), but applying it to an Int64 — `(get-x v)` with `v : Int64` — makes the
           reduced body project a field of an integer, which has no defined result. type-system.md
           §Member Access Projects A Record Field: the seed rejects CDZ0201 at the call site (the
           argument's Int64 type flows into `r`), so a bad structural use is still caught — just where
           the concrete type is known, not in the polymorphic body.")
  (input (do (def (get-x r) r.x) (def (main (: v Int64)) (get-x v)) (export main)))
  (error CDZ0201))

(case
  "a dead (unreferenced) argument is still checked for its own fault"
  (doc
    "Application checking collects each argument's OWN faults even when the parameter is DEAD — the
           body never references it — because a dead argument is not covered by the body's use. `(def (f a
           b c) a)` uses only `a`; passing an unbound name `zzz` for the dead parameter `c` must reject
           CDZ0101, not be silently accepted because `c` is unused. Pins that ignoring a parameter does not
           excuse a malformed argument in its position.")
  (input (do (def (f a b c) a) (def (main) (f 7 2 zzz)) (export main)))
  (error CDZ0101))

; The USED-parameter counterpart of the dead-argument case above. The linear-fault-walk optimization DROPS
; the raw-argument descent for a parameter the body USES — its argument is substituted into the reduced body,
; so the body walk sees the fault there instead. This must NOT lose the fault: an unbound name passed to a
; USED parameter (`(f frobnicate)` for `(def (f a) (+ a 1))`, which references `a`) still rejects CDZ0101,
; surfaced through the reduced body rather than the dropped raw-argument descent. Together with the dead case
; this pins that an argument fault is reported whether or not the parameter is used. (Migrated from rcdzc
; an_argument_fault_is_reported_whether_or_not_the_parameter_is_used.)
(case
  "an unbound argument to a USED parameter still rejects (surfaced via the reduced body)"
  (input (do (def (f a) (+ a 1)) (def (main) (f frobnicate)) (export main)))
  (error CDZ0101))

(case
  "a malformed application in a DEAD parameter's argument still rejects (a non-unbound fault kind)"
  (doc
    "A dead argument is descended for ALL its own faults, not only unbound names: a malformed
           application `(5 3)` (a non-callable literal in head position) passed to the dead parameter of
           `(def (f a) 0)` rejects CDZ0201, proving the dead-argument descent catches a structural fault the
           body — which never references the parameter — could not otherwise see.")
  (input (do (def (f a) 0) (def (main) (f (5 3))) (export main)))
  (error CDZ0201))

(case
  "a function using only its first parameter accepts all its well-typed arguments"
  (doc
    "The accept companion: the same `(def (f a b c) a)` applied to all well-typed arguments compiles
           and returns the used one — `(f 7 2 3)` = 7. Pins that a body referencing only a subset of its
           parameters does not over-reject the unused (dead) arguments when they are well-formed.")
  (input (do (def (f a b c) a) (def (main) (f 7 2 3)) (export main)))
  (output (: 7 Int64)))

; --- A recursive parameter used ONLY as a call argument infers from the callee -----------------------
; A RECURSIVE def's parameter that no primitive operator ever touches — it is only PASSED AS AN ARGUMENT
; to another def, threaded unchanged through the recursion — is still determined: its type is the
; callee's parameter type at that position. `(def (f a n) (… (twice a) … (f a (- n 1))))` uses `a` only
; in `(twice a)`, so `a`'s type is `twice`'s parameter type (Int64, pinned by `twice`'s own `(+ a a)`).
; The recursive-parameter solver reads that argument-position constraint; without it `a` stayed
; unconstrained and the def declined "a recursive function with an unannotated parameter is not yet
; inferred", refusing a well-typed program (annotating `a` compiled the same program — inference, not
; codegen, was the gap). The constraint is precise: a parameter passed to a POLYMORPHIC callee (whose
; parameter is itself unconstrained) is NOT pinned, so a generic position stays generic. This is the last
; inference piece the byte-walking reader family (a `Bytes` param threaded through a recursive walk via a
; helper) needs — see the CBOR-reader cases in 10-bytes.sexp.
(case
  "a recursive parameter used only as a call argument infers from the callee's parameter type"
  (doc
    "`f` is recursive; its parameter `a` is threaded unchanged through the recursion and used ONLY
           as the argument of `(twice a)` — no primitive operator touches `a` directly. Its type is
           `twice`'s parameter type: `twice`'s body `(+ a a)` pins that parameter to Int64, so `a` infers
           Int64 without an annotation. Was declined ('a recursive function with an unannotated parameter
           is not yet inferred') because the solver derived a constraint only from an operator applied to
           the parameter or the self-call, never from an argument position. `f(5, 3)` sums `twice(5)` =
           10 three times → 30. Inference, not codegen, was the only gap.")
  (input
    (do
      (def (twice a) (+ a a))
      (def (f a n) (if (< n 1) 0 (+ (twice a) (f a (- n 1)))))
      (def (main) (f 5 3))
      (export main)))
  (call main)
  (output (: 30 Int64)))

(case
  "a recursive byte walk threading a Bytes parameter through a helper infers without annotation"
  (doc
    "The motivating instance (the CBOR-reader family): `be` is recursive; its `Bytes` parameter `b`
           is threaded unchanged and used only as the first argument of `(byte-at b i)`. `byte-at`'s body
           `(match (Bytes.at b i) …)` pins its first parameter to `Bytes`, so `b` infers `Bytes` from that
           argument position — no annotation needed. The non-recursive helper `byte-at` itself needs no
           annotation. The bytes 1, 2, 3 are read and summed over three steps → 6. Was declined for want
           of the argument-position constraint.")
  (input
    (do
      (def (byte-at b i) (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
      (def (be b i n) (if (< n 1) 0 (+ (byte-at b i) (be b (+ i 1) (- n 1)))))
      (def (main) (be (Bytes.of #list(1 2 3)) 0 3))
      (export main)))
  (call main)
  (output (: 6 Int64))
  (live-objects 0))

; A function's name is an ordinary lexical binding, and #Binding Is Lexical resolves a reference to the
; NEAREST enclosing binding of that name — regardless of the name's capitalization. So a `def` whose name
; happens to start with an uppercase letter binds that name exactly as a lowercase one does, and a call to
; it MUST invoke the defined function, not be reinterpreted as a constructor of some tagged variant. A
; compiler that treats any capitalized name in call position as an ad-hoc constructor — synthesizing
; `(Foo 10)` for `(Foo 10)` — silently IGNORES the user's `(def (Foo x) …)` binding and returns a
; constructor value instead of the function's result: a wrong value that contradicts #Binding Is Lexical
; (the nearest binding of `Foo` is the `def`, not a prelude constructor, and there is no `Foo` variant
; declared) and #A Module Binds Its Name In Its Enclosing Scope. The lowercase companion `(def (bar) …)`
; is called correctly; the uppercase one must be too — capitalization is not a binding-precedence rule.
(case
  "a function whose name is capitalized is called, not treated as a constructor"
  (doc
    "`(def (Foo x) (+ x 1))` binds the name `Foo` to a function in the module's scope; `(Foo 10)`
           MUST resolve to that binding (core-semantics.md #Binding Is Lexical: a name resolves to the
           nearest enclosing binding) and invoke it, yielding 11. `Foo` is not a variant of any declared
           sum type, and even if it were, the user's `def` is the nearest binding. A compiler that treats
           a capitalized name in call position as an ad-hoc constructor synthesizes the value `(Foo 10)`
           and IGNORES the `def` — a wrong value (the function computing x+1 is bypassed). Capitalization
           is not a binding-precedence rule: the lowercase `(def (bar) …)` companion is called correctly,
           and the uppercase one must be too. A generation that does not resolve a capitalized name to its
           user binding declines rather than answering `(Foo 10)` (reject-don't-miscompile).")
  (input (do (def (Foo x) (+ x 1)) (def (main) (Foo 10)) (export main)))
  (output (: 11 Int64)))

(case
  "a parameter carries a type annotation in the signature"
  (doc
    "A `def` parameter may be written `(: name Type)` in the signature — an annotation in BINDER
           position. `(: a Int64)` binds `a` (the annotation names the binder, not an opaque form) and
           constrains its type to Int64, per type-system.md §Annotations Constrain, Never Contradict:
           the annotation is an additional unification constraint on the parameter, not an override. The
           body references `a` exactly as an unannotated parameter — the annotation is transparent to the
           value. `(annotated 20 22)` = 42. Pins that a signature reads through a `(: name Type)` binder
           to the name it binds, so an author can pin a parameter's type where inference would otherwise
           leave it open — the disambiguation an ambiguous runtime parameter requires.")
  (input (do (def (annotated (: a Int64) b) (+ a b)) (def (main) (annotated 20 22)) (export main)))
  (output (: 42 Int64)))

(case
  "a parameter annotation contradicting its use is rejected"
  (doc
    "An annotation constrains and MUST NOT contradict (type-system.md §Annotations Constrain,
           Never Contradict): a parameter annotated `(: a Bool)` but used where an Int64 is required —
           `(+ a 1)` unifies `a` with the integer operand of `+` — cannot be reconciled, so the program
           is rejected (CDZ0203) rather than having the annotation silently replace the inferred type or
           the use silently reinterpret the annotation. The contradiction is between the WRITTEN Bool and
           the INFERRED Int64 at the same binding, exactly the conflicting-annotation shape.")
  (input (do (def (bad (: a Bool)) (+ a 1)) (def (main) (bad true)) (export main)))
  (error CDZ0203 (exact-code)))

; A function's RETURN TYPE is declared by ascribing its body: `(def (f …) (: body R))` constrains the
; result to `R` exactly as a parameter binder `(: name T)` constrains a parameter and a value annotation
; `(: expr T)` constrains an expression (type-system.md §Annotations Constrain, Never Contradict). The
; ML surface writes this as `def f(x) -> R = body` (and `fn(x) -> R => body`), which desugars to this
; body ascription — no dedicated return-type node; the arrow is surface sugar over the annotation the
; cases below pin. A return type that AGREES with the body is transparent (the case below); one that
; CONTRADICTS the body's inferred type is rejected (CDZ0203), the result-position companion of the
; parameter-annotation-contradiction case above.
(case
  "a function's return type ascription agreeing with the body is transparent"
  (doc
    "`(def (add (: x Int64) (: y Int64)) (: (+ x y) Int64))` declares the result type by ascribing
           the body `(+ x y)` to `Int64` — the desugaring of the ML `def add(x: Int64, y: Int64) -> Int64
           = x + y`. The ascription agrees with the body's inferred Int64, so it is transparent and the
           function computes normally: `(add 20 22)` = 42. Pins that a return-type annotation constrains
           without changing a well-typed result — the result-position analogue of a matching parameter or
           value annotation.")
  (input
    (do
      (def (add (: x Int64) (: y Int64)) (: (+ x y) Int64))
      (def (main) (add 20 22))
      (export main)))
  (output (: 42 Int64)))

(case
  "a function's return type contradicting the body is rejected"
  (doc
    "`(def (f (: x Int64)) (: (+ x 1) Bool))` declares the return type `Bool` by ascribing the body,
           but `(+ x 1)` is Int64 — the declared result and the inferred result disagree, so the program
           is rejected (CDZ0203), exactly as a contradicting parameter or value annotation is. This is the
           desugaring of the ML `def f(x: Int64) -> Bool = x + 1`: a return-type annotation is an ordinary
           body ascription, and a return type that contradicts the body cannot be reconciled. The
           result-position companion of the parameter-annotation-contradiction case above.")
  (input (do (def (f (: x Int64)) (: (+ x 1) Bool)) (def (main) (f 5)) (export main)))
  (error CDZ0203 (exact-code)))

(case
  "a lambda's return type ascription agreeing with the body is transparent"
  (doc
    "The lambda companion: `(fn (x) (: (* x 2) Int64))` ascribes the lambda body to `Int64` — the
           desugaring of `fn(x) -> Int64 => x * 2`. The ascription agrees with the body, so applying the
           lambda computes normally: `((fn (x) (: (* x 2) Int64)) 21)` = 42. A lambda's return type is a
           body ascription exactly as a named def's is.")
  (input ((fn (x) (: (* x 2) Int64)) 21))
  (output (: 42 Int64)))

; The case above contradicts the annotation via the BODY (`(: a Bool)` then `(+ a 1)`). The dual is a
; contradiction via the ARGUMENT: a parameter whose annotation and body AGREE, called with an argument
; of a conflicting type. An argument's type MUST be checked against its parameter's type at the call
; (type-system.md §Annotations Constrain, Never Contradict; core-semantics.md — a well-typed program
; does not go wrong). A compiler that reduces a call by substituting the argument into the body erases
; the parameter↔argument relationship, so this check must be made at the call site, not left to the
; reduced body — else a mistyped argument is silently accepted (and, once the mis-accepted value is
; USED at its claimed type, miscompiled). These pin the argument side, the complement of the body side.
(case
  "an Int argument to a Bool-annotated parameter is rejected"
  (doc
    "`(def (f (: x Bool)) x)` annotates `x` as Bool and returns it (body agrees with the
           annotation). `(f 5)` passes an Int64 where a Bool is required — a type error (CDZ0203). The
           argument's type is checked against the parameter's ANNOTATION at the call, not silently
           accepted; the degenerate identity body would otherwise let the mis-accepted 5 flow back out
           as a returned value. Distinct from the body-contradiction case above: here the annotation and
           body agree and it is the ARGUMENT that disagrees.")
  (input (do (def (f (: x Bool)) x) (def (main) (f 5)) (export main)))
  (error CDZ0203 (exact-code)))

(case
  "an Int argument to a parameter used as a Bool condition is rejected"
  (doc
    "`(def (f x) (if x 1 2))` uses the unannotated `x` as a Bool condition, so `x : Bool` is
           inferred from its use. `(f 5)` passes an Int64 — a type error (CDZ0203). Reducing the call
           substitutes 5 into `(if x 1 2)`, giving `(if 5 1 2)` whose condition is a non-Bool — the
           reduced body's fault is reported, so the program is rejected rather than miscompiled to an
           invalid component. The correctly-typed `(f true)` yields 1.")
  (input (do (def (f x) (if x 1 2)) (def (main) (f 5)) (export main)))
  (error CDZ0203 (exact-code)))

(case
  "a Bool argument to a parameter used in integer addition is rejected"
  (doc
    "The mirror direction: `(def (f x) (+ x x))` infers `x : Int64` from the addition; `(f true)`
           passes a Bool — a type error (CDZ0203). The reduced body `(+ true true)` faults on the
           non-integer operand, so the call is rejected, not miscompiled. The correctly-typed `(f 5)`
           yields 10. Pins that an argument is checked against a body-INFERRED parameter type, not only
           an explicit annotation.")
  (input (do (def (f x) (+ x x)) (def (main) (f true)) (export main)))
  (error CDZ0203 (exact-code)))

; --- A FUNCTION-TYPED parameter annotation is checked against the passed function, RESULT included -
; The higher-order analogue of the scalar arg-vs-param checks above. A parameter annotated with a
; function type `(-> A B)` constrains the ARGUMENT to a function of that type — parameter AND result
; (type-system.md §Annotations Constrain, Never Contradict). Passing an `A -> B'` function whose result
; `B'` disagrees with the annotated `B` is a type error, and the check must descend through NESTED
; arrows (a curried `(-> A (-> C D))` checks the inner result too). A passed lambda is typed as its own
; arrow type — a bare parameter contributes `Any` (so it unifies with any expected parameter type, no
; over-rejection), only a definite RESULT disagreement faults. The scalar-vs-function mismatch (`(f 5)`
; to a function parameter) is already caught; this closes the function-vs-function deep-result hole.
(case
  "a function-typed parameter annotation's result type is checked against the argument"
  (doc
    "`(def (f (: g (-> Int64 Bool))) (g 41))` declares `g` as `Int64 -> Bool`, but `(f (fn (x) (+
           x 1)))` passes an `Int64 -> Int64` function — the RESULT types disagree (Bool vs Int64), a
           type error (CDZ0203). The annotation must not be silently dropped: the passed lambda is typed
           as its arrow type `Int64 -> Int64` and unified against the declared `Int64 -> Bool`, so the
           result mismatch faults. The higher-order analogue of the scalar `(f 5)`-to-a-Bool-parameter
           rejection above.")
  (input
    (do (def (f (: g (-> Int64 Bool))) (g 41)) (def (main) (f (fn (x) (+ x 1)))) (export main)))
  (error CDZ0203 (exact-code)))

(case
  "a function-typed parameter annotation is not silently discarded in the body"
  (doc
    "The witness that the annotation CONSTRAINS the body, not merely the call: `(def (f (: g (->
           Int64 Bool))) (+ (g 41) 1))` — if `g`'s result were the annotated Bool, `(+ (g 41) 1)` would
           be `(+ Bool 1)` and reject. It must reject (CDZ0203): `g`'s result is fixed to Bool by the
           annotation, so using it as an integer operand contradicts. A generation that dropped the
           annotation typed `(g 41)` as the actual Int64 and computed 43 — the annotation having no
           effect. Pins that the fn-type annotation governs `g`'s result type throughout the body.")
  (input
    (do
      (def (f (: g (-> Int64 Bool))) (+ (g 41) 1))
      (def (main) (f (fn (x) (+ x 1))))
      (export main)))
  (error CDZ0203 (exact-code)))

(case
  "a curried function-type annotation checks its inner result type against the argument"
  (doc
    "The annotation check descends through NESTED arrows: `(def (f (: g (-> Int64 (-> Int64
           Bool)))) ((g 1) 2))` annotates `g` as `Int64 -> Int64 -> Bool`, but `(fn (a) (fn (b) (+ a
           b)))` is `Int64 -> Int64 -> Int64` — the INNER result types disagree (Bool vs Int64). Must
           reject (CDZ0203). The function-type unification is structural, so a mismatch at any arrow
           depth is caught, not only the outermost result.")
  (input
    (do
      (def (f (: g (-> Int64 (-> Int64 Bool)))) ((g 1) 2))
      (def (main) (f (fn (a) (fn (b) (+ a b)))))
      (export main)))
  (error CDZ0203 (exact-code)))

(case
  "a correctly-annotated function parameter is accepted"
  (doc
    "The passing boundary: `(def (f (: g (-> Int64 Int64))) (g 41))` with the matching `Int64 ->
           Int64` function `(fn (x) (+ x 1))` yields 42. Pins that a CORRECT function-type annotation is
           accepted — the fix REJECTS a mismatched annotation without over-rejecting a matching one. A
           bare-param lambda's parameter type is `Any`, so it unifies with the declared `Int64`
           parameter freely; only a result disagreement faults, and here there is none.")
  (input
    (do (def (f (: g (-> Int64 Int64))) (g 41)) (def (main) (f (fn (x) (+ x 1)))) (export main)))
  (output (: 42 Int64)))

(case
  "a function-typed parameter with a matching Bool-returning argument is accepted"
  (doc
    "A matching non-Int result: `(def (f (: g (-> Int64 Bool))) (g 41))` applied to `(fn (x) (< x
           5))` — an `Int64 -> Bool` function that agrees with the annotation — yields `(< 41 5)` =
           false. Complements the rejection cases: when the passed function's result type MATCHES the
           annotated one, the program is accepted and runs, confirming the check is a genuine agreement
           test, not a blanket rejection of function-typed parameters.")
  (input
    (do (def (f (: g (-> Int64 Bool))) (g 41)) (def (main) (f (fn (x) (< x 5)))) (export main)))
  (output (: false Bool)))

; --- Runtime arguments to the entrypoint: (call <export> <arg>…) --------------------------------
; Every case above calls a parameterized function with CONSTANT arguments, so the compiler folds the
; whole program to a value at compile time — a real strength (a compile-provable trap fails the build),
; but it means the emitted component's runtime machinery (parameter slots, `local.get`, a genuine
; runtime `+`/`*`/comparison, a branch on a runtime value) is never exercised. A value that arrives at
; RUN TIME — an argument supplied to the exported entry from outside the component — cannot be folded:
; the entry becomes `input -> output` and its parameter crosses the boundary as a lifted value
; (contracts/component-abi.md §The Entry Is A Plain Function; §The Exported Interface Is The Declared
; Signature — the interface is read from the export's declared PARAMETER and result types). These cases
; use the `(call <export> <arg>…)` clause to run the exported entry with runtime arguments, so the
; operation over the parameter is emitted as real instructions rather than constant-folded. Each `<arg>`
; is a `(: <value> <Type>)` value-form; the runner coerces it to the export's declared parameter type.
; The parameter MUST be annotated (`(: x Int64)`) — an entry's boundary representation follows its
; declared signature, and an unannotated parameter has no boundary width, so the compiler declines it.
; The seed realizes a parameterized export, because a compiler authored in
; Cadenza is itself a component whose entry takes its input as a runtime argument.
(case
  "the entrypoint returns its runtime argument unchanged"
  (doc
    "The identity entry: `(def (main (: x Int64)) x)` exported and called with the runtime
           argument 42. The argument arrives from OUTSIDE the component (not a compile-time constant), so
           it cannot be folded — the body is a bare parameter reference lowered to a `local.get` of the
           entry's one parameter slot, lifted back across the boundary. Pins that a parameterized entry
           receives a runtime value and returns it, the minimal exercise of the boundary parameter path
           the folded nullary cases never reach (contracts/component-abi.md §The Entry Is A Plain
           Function — an entry is `input -> output`, its parameter type carrying a boundary form).")
  (input (do (def (main (: x Int64)) x) (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64)))

(case
  "the entrypoint adds one to its runtime argument"
  (doc
    "`(def (main (: x Int64)) (+ x 1))` exported and called with 41. One operand of `+` is the
           runtime parameter `x`, so the addition CANNOT fold to a constant — it is emitted as a genuine
           runtime `i64.add` over the parameter's local slot and the literal 1. (Contrast the folded
           `(+ 2 3)` in 06-numeric-model, which the compiler reduces to 5 at build time.) This is the
           smallest case that exercises the runtime arithmetic path a program's machinery actually runs;
           41 + 1 = 42.")
  (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64)))

(case
  "the entrypoint multiplies its runtime argument"
  (doc
    "`(def (main (: x Int64)) (* x 3))` called with 7 — a runtime `i64.mul` over the parameter and
           the literal 3, yielding 21. Companion to the runtime `+` case, pinning that multiplication too
           is emitted as a real instruction (not folded) when an operand is a runtime argument.")
  (input (do (def (main (: x Int64)) (* x 3)) (export main)))
  (call main (: 7 Int64))
  (output (: 21 Int64)))

(case
  "a runtime const-multiply at its exact fitting boundary does not trap"
  (doc
    "`(def (main (: x Int64)) (* x 3))` called with MAX/3 = 3074457345618258602 — the LARGEST `x`
           whose product `x*3` still fits Int64 (= 9223372036854775806, one below Int64.max). The
           checked multiply's overflow guard is a single UNSIGNED range check `(x - MIN/3) >ᵤ
           (MAX/3 - MIN/3) → trap`; at the exact upper endpoint the shifted value equals the interval
           width, so `>ᵤ` is false and the product is returned. Pins the guard admits its own boundary.")
  (input (do (def (main (: x Int64)) (* x 3)) (export main)))
  (call main (: 3074457345618258602 Int64))
  (output (: 9223372036854775806 Int64)))

(case
  "a runtime const-multiply one past its fitting boundary traps"
  (doc
    "`(def (main (: x Int64)) (* x 3))` called with MAX/3 + 1 = 3074457345618258603 — one past the
           largest fitting `x`, so `x*3` overflows Int64 and MUST trap. The single unsigned range check
           `(x - MIN/3) >ᵤ (MAX/3 - MIN/3)`: shifting `x` one past the top wraps just above the interval
           width, so `>ᵤ` fires. Companion to the boundary-fits case — together they pin the exact
           inclusive interval endpoint the collapsed one-compare guard admits (parity with the two-compare
           form it replaced, gate-verified both signs of the multiplier).")
  (input (do (def (main (: x Int64)) (* x 3)) (export main)))
  (call main (: 3074457345618258603 Int64))
  (trap "integer overflow"))

(case
  "a runtime negative-const-multiply one past its lower fitting boundary traps"
  (doc
    "`(def (main (: x Int64)) (* x -3))` called with MAX/3 + 1 = 3074457345618258603 — a NEGATIVE
           multiplier flips the fitting interval to `[MAX/-3, MIN/-3]`, and this `x` overshoots it, so
           `x*-3` overflows and traps. Pins that the collapsed unsigned range check is correct for a
           negative constant too (its `lo`/`hi` endpoints swap, the single `>ᵤ` still decides both sides).")
  (input (do (def (main (: x Int64)) (* x -3)) (export main)))
  (call main (: 3074457345618258603 Int64))
  (trap "integer overflow"))

(case
  "a repeated runtime subexpression is computed once (CSE) and reused across an operator"
  (doc
    "`(def (main (: x Int64)) (+ (& x 7) (& x 7)))` — the subexpression `(& x 7)` appears twice, so
           common-subexpression elimination computes it ONCE into a slot and both operands of the `+` read
           that slot (no recompute, no redundant spill). Called with 13: `13 & 7 = 5`, and `5 + 5 = 10`.
           Pins that a value-equal repeated operand is shared and that the shared value feeds the addition
           correctly (the CSE slot is read directly as the operand source).")
  (input (do (def (main (: x Int64)) (+ (& x 7) (& x 7))) (export main)))
  (call main (: 13 Int64))
  (output (: 10 Int64)))

(case
  "a repeated runtime checked-multiply is computed once and still traps on overflow"
  (doc
    "`(def (main (: x Int64)) (+ (* x x) (* x x)))` — the checked square `(* x x)` is CSE'd (computed
           once with its overflow guard) and the outer `+` doubles it. Called with a large x whose square
           already overflows Int64, so the shared `(* x x)` traps — CSE preserves the trap (the shared
           computation traps at its single evaluation point, exactly as an un-shared one would). Pins that
           sharing a CHECKED operation keeps its overflow semantics.")
  (input (do (def (main (: x Int64)) (+ (* x x) (* x x))) (export main)))
  (call main (: 9223372036 Int64))
  (trap "integer overflow"))

(case
  "an inlined multi-use checked-arith argument is shared and computed directly into its slot"
  (doc
    "`(def (f (: a Int64) (: b Int64)) (+ (* a b) (- a b)))` applied as `(f x (+ x 1))` — the
           argument `(+ x 1)` binds `b`, which `f` uses TWICE (in `(* a b)` and `(- a b)`), so it is
           computed ONCE (shared) rather than re-evaluated. The shared checked add writes its result
           directly into its slot (its overflow-guard scratch IS the shared slot — no temp-then-copy).
           Called with x = 5: b = 6, so `(5*6) + (5-6) = 30 + -1 = 29`. Pins that a shared checked-arith
           argument computes correctly when its result is stored directly into the shared slot.")
  (input
    (do
      (def (f (: a Int64) (: b Int64)) (+ (* a b) (- a b)))
      (def (main (: x Int64)) (f x (+ x 1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 29 Int64)))

(case
  "the entrypoint sums its two runtime arguments"
  (doc
    "A two-parameter entry `(def (main (: a Int64) (: b Int64)) (+ a b))` called with 20 and 22.
           BOTH operands are runtime arguments, so the `+` is a runtime `i64.add` over two parameter
           slots — nothing is constant. Pins that an entry takes MORE than one boundary argument, each in
           its own local slot in signature order, and the arguments are supplied in order; 20 + 22 = 42.")
  (input (do (def (main (: a Int64) (: b Int64)) (+ a b)) (export main)))
  (call main (: 20 Int64) (: 22 Int64))
  (output (: 42 Int64)))

(case
  "the entrypoint returns its runtime boolean argument"
  (doc
    "`(def (main (: b Bool)) b)` called with the runtime boolean `true`. Pins that a Bool crosses
           the entry boundary as a runtime argument (not only an integer) and lifts back unchanged — the
           boolean boundary representation on the parameter side, mirroring the Bool result cases.")
  (input (do (def (main (: b Bool)) b) (export main)))
  (call main (: true Bool))
  (output (: true Bool)))

(case
  "the entrypoint compares its runtime argument to a bound"
  (doc
    "`(def (main (: x Int64)) (< x 10))` called with 5 — a runtime `<` comparison between the
           parameter and the literal 10, producing a Bool. The comparison cannot fold (one operand is a
           runtime value), so it is emitted as a real runtime comparison. 5 < 10 is true. Pins that a
           relational operator over a runtime argument runs as an instruction and yields a boundary Bool.")
  (input (do (def (main (: x Int64)) (< x 10)) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case
  "the entrypoint branches on its runtime argument"
  (doc
    "`(def (main (: x Int64)) (if (< x 0) 0 x))` — clamp-to-zero — called with -3. The `if`
           condition is a runtime comparison on the parameter, so the branch is a genuine runtime
           structured `if` (not a compile-time choice of arm): with x = -3 the condition holds and the
           entry yields 0. Pins that control flow driven by a runtime argument is emitted as a real
           branch, the last piece of the runtime machinery the folded nullary cases skip. (A negative
           argument also exercises the runner taking a leading-`-` value as the argument, not a flag.)")
  (input (do (def (main (: x Int64)) (if (< x 0) 0 x)) (export main)))
  (call main (: -3 Int64))
  (output (: 0 Int64)))

(case
  "an if over two trap-free bitwise arms selects branchlessly — then arm"
  (doc
    "`(def (main (: x Int64)) (if (< x 0) (& x 7) (| x 8)))` called with -3. Both arms are small
           TRAP-FREE bitwise ops, so the compiler emits a BRANCHLESS `select` (evaluate both arms, pick by
           the condition) instead of a structured `if`/`else` — sound because a bitwise op can neither trap
           nor allocate on the untaken path. x = -3 is < 0 so the then arm is taken: -3 & 7 = 5. Pins the
           widened if-conversion (a select arm need not be a bare leaf, just a small trap-free scalar).")
  (input (do (def (main (: x Int64)) (if (< x 0) (& x 7) (| x 8))) (export main)))
  (call main (: -3 Int64))
  (output (: 5 Int64)))

(case
  "an if over two trap-free bitwise arms selects branchlessly — else arm"
  (doc
    "The companion selecting the OTHER arm of `(if (< x 0) (& x 7) (| x 8))`: called with 4 (not
           < 0), so the else arm is taken: 4 | 8 = 12. Together with the then-arm case this pins that the
           branchless select computes BOTH bitwise arms and returns the one the runtime condition picks —
           value parity with the structured `if` it replaces, on both condition polarities.")
  (input (do (def (main (: x Int64)) (if (< x 0) (& x 7) (| x 8))) (export main)))
  (call main (: 4 Int64))
  (output (: 12 Int64)))

(case
  "an if whose untaken arm would trap keeps the branch (no select of checked arith)"
  (doc
    "`(def (main (: x Int64)) (if (< x 0) x (* x 1000000000000)))` called with -5. The else arm is a
           CHECKED multiply that overflows for a large x — NOT trap-free — so the compiler must keep the
           structured `if` and NOT convert to a branchless `select` (which would evaluate the else arm even
           when the then arm is taken). x = -5 is < 0 → the then arm `x` is returned = -5, and the
           would-overflow else arm is never evaluated (no trap). Pins the trap-freedom guard on the
           if-conversion: only an arm that cannot trap on the untaken path may become a select operand.")
  (input (do (def (main (: x Int64)) (if (< x 0) x (* x 1000000000000))) (export main)))
  (call main (: -5 Int64))
  (output (: -5 Int64)))

(case
  "a nested conditional (the sign idiom) folds to branchless nested selects — negative"
  (doc
    "`(def (main (: x Int64)) (if (< x 0) -1 (if (> x 0) 1 0)))` — the three-way sign function —
           called with -42. The else arm is itself a conditional over trap-free parts (a comparison and
           constants), so the whole thing compiles to fully BRANCHLESS nested `select`s (no `if`/`else`
           block). x = -42 < 0 → -1. Pins the nested-conditional widening of the if→select conversion.")
  (input (do (def (main (: x Int64)) (if (< x 0) -1 (if (> x 0) 1 0))) (export main)))
  (call main (: -42 Int64))
  (output (: -1 Int64)))

(case
  "a nested conditional (the sign idiom) folds to branchless nested selects — zero"
  (doc
    "The zero case of the sign function `(if (< x 0) -1 (if (> x 0) 1 0))`: called with 0, neither
           `< 0` nor `> 0` holds, so the innermost else `0` is selected. Confirms the branchless nested
           selects reproduce the middle arm's value exactly.")
  (input (do (def (main (: x Int64)) (if (< x 0) -1 (if (> x 0) 1 0))) (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a nested conditional (the sign idiom) folds to branchless nested selects — positive"
  (doc
    "The positive case of the sign function `(if (< x 0) -1 (if (> x 0) 1 0))`: called with 42
           (> 0, not < 0) → 1. Together with the negative and zero cases this pins value parity of the
           branchless nested selects with the structured nested `if` across all three sign regions.")
  (input (do (def (main (: x Int64)) (if (< x 0) -1 (if (> x 0) 1 0))) (export main)))
  (call main (: 42 Int64))
  (output (: 1 Int64)))

; --- Narrow-width runtime arguments cross as their FAITHFUL component primitive -------------------
; The eight aliased widths (Int8/16/32, UInt8/16/32/64 and their `(Int N)` expansions) each have a
; component boundary representation: they cross as `s8`/`u8`/`s16`/`u16`/`s32`/`u32`/`s64`/`u64`, NOT as
; a wider machine slot. So a `(: n UInt8)` entry parameter takes a `u8` at the edge — the host cannot
; pass 300 for it (wasmtime rejects an out-of-range u8), which is exactly the safety a narrow width buys.
; These `(call …)` cases run a narrow-width entry over a runtime argument, exercising the faithful
; boundary lift on the parameter side and the emitted narrow (i32-slot, range-checked) operation. The
; seed realizes the aliased widths' boundary forms.
(case
  "an unsigned-byte entrypoint takes and returns a u8 at the boundary"
  (doc
    "`(def (main (: n UInt8)) n)` exported and called with 200. The parameter crosses as the
           component `u8` (its faithful width, not a machine s32/u32), lifts to the i32 slot the body
           reads, and lowers back to `u8` — 200. Pins that an aliased narrow width has a boundary form
           and that a UInt8 argument round-trips through the component edge unchanged.")
  (input (do (def (main (: n UInt8)) n) (export main)))
  (call main (: 200 UInt8))
  (output (: 200 UInt8)))

(case
  "a runtime unsigned-byte addition traps on overflow of its width"
  (doc
    "`(def (main (: a UInt8) (: b UInt8)) (+ a b))` called with (200, 55) = 255, which fits UInt8
           (max 255). The `+` is emitted (both operands runtime) as the width-generic checked op: it
           computes in the i32 slot and range-checks the result back to 0..=255. 200+55 fits, so it
           returns 255 — the companion overflow (200+56=256) is the trap case pinned in 06-numeric-model.
           Pins that a NARROW runtime arithmetic op runs over faithful-u8 boundary arguments.")
  (input (do (def (main (: a UInt8) (: b UInt8)) (+ a b)) (export main)))
  (call main (: 200 UInt8) (: 55 UInt8))
  (output (: 255 UInt8)))

; The addition above has two NARROW runtime operands; the far more common shape is a narrow parameter
; and a BARE INTEGER LITERAL — incrementing a byte, comparing a narrow counter to a bound. A bare
; literal is width-polymorphic (it defaults to Int64 on its own), so it MUST take the width of the
; operand it is combined with — `(+ x 1)` with `x : UInt8` treats `1` as a UInt8. The operands of a
; binary op share one machine representation; a literal left at its Int64 default beside a narrow
; (i32-slot) parameter is a width clash the emitted op cannot express. These pin that a narrow-param-
; plus-literal op computes (the literal grounded to the operand's width), for `+`, `*`, and comparison.
(case
  "a narrow-width parameter plus a bare literal computes at the parameter width"
  (doc
    "`(def (main (: x UInt8)) (+ x 1))` called with 100 = 101. The bare literal `1` takes `x`'s
           UInt8 width (a literal is width-polymorphic until an operand constrains it), so the addition
           is a homogeneous narrow op — not a UInt8-plus-Int64 clash. The annotated form `(+ x (: 1
           UInt8))` and two-narrow-param `(+ a b)` (above) already compute; this pins the bare-literal
           operand, the common increment-a-byte shape.")
  (input (do (def (main (: x UInt8)) (+ x 1)) (export main)))
  (call main (: 100 UInt8))
  (output (: 101 UInt8)))

(case
  "a signed narrow-width parameter plus a bare literal computes at the parameter width"
  (doc
    "The signed sibling: `(def (main (: x Int8)) (+ x 1))` called with 50 = 51. The literal takes
           the Int8 width, so the op is a homogeneous narrow (i32-slot) addition. Pins that the
           literal-width unification is not UInt8-specific.")
  (input (do (def (main (: x Int8)) (+ x 1)) (export main)))
  (call main (: 50 Int8))
  (output (: 51 Int8)))

(case
  "a narrow-width parameter compared to a bare literal computes at the parameter width"
  (doc
    "The comparison face: `(def (main (: x UInt8)) (> x 50))` called with 100 = true. The literal
           `50` takes `x`'s UInt8 width, so the comparison's operands share one machine slot. Pins that
           the bare-literal width unification applies to every binary op over a narrow parameter, not
           only `+`.")
  (input (do (def (main (: x UInt8)) (> x 50)) (export main)))
  (call main (: 100 UInt8))
  (output (: true Bool)))

(case
  "a narrow-width parameter plus a bare literal computes inside a helper function"
  (doc
    "`(def (bump (: x UInt8)) (+ x 1))` called via `(bump y)` where `y : UInt8` = 101. The
           narrow-param-plus-literal op computes in a non-entry function body exactly as in the entry —
           the literal takes the parameter's width wherever the operation appears.")
  (input (do (def (bump (: x UInt8)) (+ x 1)) (def (main (: y UInt8)) (bump y)) (export main)))
  (call main (: 100 UInt8))
  (output (: 101 UInt8)))

; A CONSTANT ARGUMENT passed to a NARROW-typed parameter must be RANGE-CHECKED against the parameter's
; declared width, exactly as a direct annotation `(: 200 Int8)` is (→ CDZ0302). β-reduction substitutes
; the argument for the parameter, and the parameter's annotation `(: a T)` constrains what its argument
; may be — so an out-of-range constant is rejected, not laundered into a narrow type and run to a value
; the type cannot hold. This is enforced by carrying the annotation onto the substituted argument
; (`(: arg T)`), so the same fit-check fires on `def`, `fn`, curried, and let-binder substitution alike.
; (An IN-range constant round-trips unchanged; a RUNTIME argument keeps its own already-checked type.)
(case
  "a constant argument out of a narrow parameter's range is rejected, not laundered"
  (doc
    "`(def (f (: a Int8)) a)` returns its Int8 parameter; `(f 200)` passes 200, OUT of Int8's
           -128..127 range. The parameter annotation constrains the argument exactly as a direct
           `(: 200 Int8)` does — CDZ0302. Without the check the constant is β-reduced into the body with
           its annotation discarded and the program runs to 200, a value no Int8 can hold (the boundary is
           sharp: `(f 127)` gives 127, `(f 128)` would wrongly give 128).")
  (input (do (def (f (: a Int8)) a) (def (main) (f 200)) (export main)))
  (error CDZ0302))

(case
  "a negative constant argument to an unsigned parameter is rejected, not laundered"
  (doc
    "`(def (f (: a UInt8)) a)` with `(f -1)`: a UInt8 has no negative representation (0..255), so a
           direct `(: -1 UInt8)` rejects CDZ0302 and the parameter path must too. The unsigned case is the
           sharpest witness — not a wrap-around near a boundary but a sign the type does not have.")
  (input (do (def (f (: a UInt8)) a) (def (main) (f -1)) (export main)))
  (error CDZ0302))

(case
  "a narrow-body arithmetic on an in-range constant arg overflows the parameter width"
  (doc
    "`(def (f (: a Int8)) (+ a a))` with `(f 100)`: 100 IS in Int8 range, but `(+ a a)` = 200
           OVERFLOWS Int8 (max 127). The argument carries the Int8 annotation into the body, so the
           addition is a homogeneous Int8 op whose CONSTANT operands fold and the compiler proves the
           overflow at compile time — a constant OPERATION with no value → CDZ0304 (ConstTrap), exactly
           as `(+ (: 100 Int8) (: 100 Int8))` and the wide `(+ Int64.max 1)` do. Pins that the width
           constraint is not dropped by inlining: the wide 200 is never kept as the result.")
  (input (do (def (f (: a Int8)) (+ a a)) (def (main) (f 100)) (export main)))
  (error CDZ0304))

(case
  "an in-range constant argument to a narrow parameter computes at the parameter width"
  (doc
    "The complement — the check does NOT over-reject. `(def (f (: a Int8)) (+ a 10))` with `(f 100)`
           = 110, which fits Int8, so it computes normally at the Int8 width. Pins that carrying the
           annotation onto the argument range-checks WITHOUT breaking a legitimate in-range call.")
  (input (do (def (f (: a Int8)) (+ a 10)) (def (main) (f 100)) (export main)))
  (output (: 110 Int8)))

(case
  "an annotated let binder range-checks its narrow-width bound value"
  (doc
    "`(let (((: a Int8) 200)) a)` — the annotated let binder `(: a Int8)` constrains the bound
           value's TYPE (a `(let (((: a Bool) 5)) …)` correctly rejects CDZ0203) AND range-checks the
           narrow-width value: 200 is out of Int8's -128..127 range, so — exactly as the value annotation
           `(let ((a (: 200 Int8))) a)` gives CDZ0302 — the binder-annotation form does too. A binder
           annotation applies its type's fit-check to the bound value, like a value annotation.")
  (input (do (def (main) (let (((: a Int8) 200)) a)) (export main)))
  (error CDZ0302))

; A `match` over a NARROW-width scrutinee whose arms include both a bare-literal arm and a binder (or a
; narrow value) arm must reconcile the arm widths: every arm produces the match's RESULT type, so a
; bare-literal arm (which defaults to Int64 on its own) takes the result's narrow width — otherwise a
; default-Int64 arm beside a narrow arm pushes a mismatched machine slot and wasm rejects the block.
; This is the match-arm analogue of the bare-literal-operand width reconciliation above. The corpus
; gates match binders only over Int64; these pin the narrow-scrutinee binder path.
(case
  "a match binder over a narrow scrutinee returns the bound value"
  (doc
    "`(match x (0 100) (n n))` with `x : UInt8`, called with 5, binds the non-zero scrutinee to
           `n` and returns it = 5. The literal arm `100` takes the match's UInt8 result width (so both
           arms share the i32 slot); the binder arm returns the scrutinee at its UInt8 width. A binder
           over an Int64 scrutinee already works; this pins the narrow scrutinee's binder.")
  (input (do (def (main (: x UInt8)) (match x (0 100) (n n))) (export main)))
  (call main (: 5 UInt8))
  (output (: 5 UInt8)))

(case
  "a signed narrow match binder returns the bound value"
  (doc
    "The signed sibling: `(match x (0 100) (n n))` with `x : Int8`, called with 5 = 5. Confirms
           the narrow-arm-width reconciliation spans every aliased narrow width, not just UInt8.")
  (input (do (def (main (: x Int8)) (match x (0 100) (n n))) (export main)))
  (call main (: 5 Int8))
  (output (: 5 Int8)))

(case
  "a narrow match binder used in arithmetic with the scrutinee"
  (doc
    "`(match x (0 0) (n (+ n x)))` with `x : UInt8`, called with 50 = 100. The binder `n` is the
           narrow scrutinee, and the arithmetic arm combines it with `x` at UInt8; the zero-arm literal
           `0` takes the UInt8 result width. Pins that the bound value is usable in a downstream op, not
           only returned directly.")
  (input (do (def (main (: x UInt8)) (match x (0 0) (n (+ n x)))) (export main)))
  (call main (: 50 UInt8))
  (output (: 100 UInt8)))

(case
  "matching against zero probes the normalized narrow value, not the raw wide slot"
  (doc
    "A match probe against the literal 0 may be emitted as wasm `eqz` (a single zero test rather than
           `const 0 ; eq`). It MUST test the NORMALIZED narrow value, not the raw machine slot that carries
           it: `(match (UInt8.wrap n) (0 100) (_ 200))` with `n = 2^32` truncates to the UInt8 0 — its low
           8 bits are zero — so the `0` arm fires and the result is 100, EVEN THOUGH the wide i64 slot
           holding 2^32 is non-zero. An `eqz` applied to the un-masked wide slot would see 2^32 ≠ 0 and
           wrongly take the `_` arm (200). Pins that the zero-probe operates on the value at its width (the
           `UInt8.wrap` result masked to 8 bits), the match-probe companion of the narrow-operand
           normalization the arithmetic cases require.")
  (input (do (def (main (: n Int64)) (match (UInt8.wrap n) (0 100) (_ 200))) (export main)))
  (call main (: 4294967296 Int64))
  (output (: 100 Int64)))

; An `if` whose branches MIX a narrow-width value and a bare integer literal must reconcile the branch
; widths: both branches produce the `if`'s RESULT type, so a bare-literal branch (which defaults to
; Int64 on its own) takes the result's narrow width — otherwise a default-Int64 branch beside a narrow
; branch pushes a mismatched machine slot into the block. This is the `if`-branch analogue of the
; bare-literal-operand (`(+ x 1)`) and bare-literal-match-arm reconciliations above. The corpus gates
; `if` over narrow conditions but never an `if` whose branches mix a narrow value and a bare literal.
(case
  "an if with a narrow branch and a bare-literal branch computes at the narrow width"
  (doc
    "`(if c x 0)` with `x : UInt8` and `c : Bool`: the then-branch is the UInt8 param, the
           else-branch a bare literal `0`. With c = true the result is x = 200. The literal branch takes
           the `if`'s UInt8 result width so both branches share the i32 slot — not a UInt8-vs-Int64
           machine-type clash. The annotated form `(if c x (: 0 UInt8))` and the both-same-param form
           already compute; this pins the bare-literal branch.")
  (input (do (def (main (: x UInt8) (: c Bool)) (if c x 0)) (export main)))
  (call main (: 200 UInt8) (: true Bool))
  (output (: 200 UInt8)))

(case
  "a signed narrow if-branch opposite a bare literal computes at the narrow width"
  (doc
    "The signed sibling: `(if c x 0)` with `x : Int8`, c = true → 50. Confirms the `if`-branch
           width reconciliation spans every aliased narrow width, not just UInt8.")
  (input (do (def (main (: x Int8) (: c Bool)) (if c x 0)) (export main)))
  (call main (: 50 Int8) (: true Bool))
  (output (: 50 Int8)))

(case
  "a narrow value in the else branch opposite a bare literal computes at the narrow width"
  (doc
    "Branch-position independence: `(if c 0 x)` puts the bare literal in the THEN branch and the
           narrow `x` in the ELSE; with c = false the result is x = 200. The reconciliation grounds
           whichever branch is the bare literal, so both orders compute identically.")
  (input (do (def (main (: x UInt8) (: c Bool)) (if c 0 x)) (export main)))
  (call main (: 200 UInt8) (: false Bool))
  (output (: 200 UInt8)))

(case
  "a signed-byte entrypoint returns its runtime argument"
  (doc
    "`(def (main (: n Int8)) n)` called with -128 (Int8.min). The parameter crosses as the
           component `s8`, so the sign is preserved at the boundary (an s8 -128, not a widened s32). Pins
           the signed narrow-width boundary form and that Int8.min round-trips.")
  (input (do (def (main (: n Int8)) n) (export main)))
  (call main (: -128 Int8))
  (output (: -128 Int8)))

; --- Truncating conversion `T.wrap` over a runtime operand: the emitted mask-and-reinterpret ------
; `T.wrap` truncates any integer to width T, keeping the low bits of its two's-complement value — the
; principled, TYPE-directed form of a byte-truncation (the width comes from the type `UInt8`, not a magic
; op name). It is TOTAL — it never traps, whatever the input (the checked companion `T.of`, which reports
; an out-of-range value rather than truncating, returns an Option and arrives with sum types). On a
; runtime operand it cannot fold, so the conversion is EMITTED (a slot move + a mask, + a sign-extend for
; a signed target). These `(call …)` cases run `wrap` over a runtime Int64 argument, pinning that the
; emitted path agrees with the constant fold across the slot-crossing (i64 source → narrow target) the
; folded cases never reach. The seed realizes `wrap` for the aliased widths.
(case
  "a runtime truncation to an unsigned byte keeps the low bits, total on negatives"
  (doc
    "`(def (main (: n Int64)) (UInt8.wrap n))` — a runtime truncating conversion (a self-hosted
           encoder truncating a computed value to a byte), emitted as an `i32.wrap_i64` of the parameter
           then a mask. `wrap` keeps the low 8 bits and is TOTAL (never traps, unlike the checked
           `T.of`). Exercised at two operands: n = 300 = 0x12C keeps 0x2C = 44 : UInt8; n = -1 keeps the
           low 8 bits of -1's two's-complement (all ones) = 255 : UInt8, WITHOUT trapping on the negative
           value — the emitted conversion reinterprets the low bits exactly as the constant fold does.")
  (input (do (def (main (: n Int64)) (UInt8.wrap n)) (export main)))
  (call main (: 300 Int64))
  (output (: 44 UInt8))
  (call main (: -1 Int64))
  (output (: 255 UInt8)))

(case
  "a runtime truncation into a signed byte sign-extends"
  (doc
    "`(def (main (: n Int64)) (Int8.wrap n))` called with 200. The low 8 bits (0xC8) have bit 7
           set, so as a SIGNED Int8 the value is -56 (sign-extended) — crossing the boundary as s8. Pins
           that a signed target's `wrap` sign-extends from the target's high bit, distinct from the
           unsigned truncation above.")
  (input (do (def (main (: n Int64)) (Int8.wrap n)) (export main)))
  (call main (: 200 Int64))
  (output (: -56 Int8)))

; ── An argument bound to an unused parameter is UNOBSERVED, so its trap is not raised ────────────────
; core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed: an argument whose value the
; function body never uses is unobserved — its value reaches neither the result nor a host call — so an
; implementation MAY decline to evaluate it, eliding the trap it would have raised. The dual anchor pins
; that the moment the body USES the parameter, the argument is observed and its trap fires. This is the
; call-boundary companion of the un-projected tuple element in 05-compound-types.sexp. (An argument that
; PROVABLY traps and is elided also earns a non-error diagnostic — CDZ0305 — asserted by a compiler unit
; test; the gate observes the run, and the build succeeds.)
(case
  "an argument bound to an unused parameter is not evaluated, so its trap does not occur"
  (doc
    "`(def (f x y) x)` ignores its second parameter `y`. Calling `(f 7 (/ 1 d))` with d = 0 passes
           a division by zero as the unused argument. `y`'s value is never observed in the body, so the
           argument need not be evaluated and its trap does not occur — the program yields 7. Uses a
           runtime (parameter-driven) div0 so this is a genuine emitted-code question, not a constant
           fold. The anchor below pins that a USED argument's trap DOES fire.")
  (input (do (def (f x y) x) (def (main (: d Int64)) (f 7 (/ 1 d))) (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a function with an unused parameter compiles and runs but the build surfaces a CDZ0306 unused-parameter warning"
  (doc
    "The parameter face of the unused-binding warning (02-binding-and-control.sexp pins the `let`
           face): `(def (f x y) x)` never references its second parameter `y`, so the program still runs
           (`(f 7 8)` = 7) but the compiler emits a CDZ0306 `unused parameter` WARNING rather than silently
           keeping the dead parameter — the same code-quality/dead-code band as the unused let binding. The
           parameter NAME is in the warning's dynamic tail (`unused parameter `y``), so only the stable lead
           `unused parameter` is pinned. Wasm-graded (warnings ride the shared compile stage = target-
           independent; the rust/rust-async run paths cannot observe compile stderr, so the (warns ..) check
           is skipped there, not failed). Portable companion of the rcdzc unused-parameter warning assertion.
           The confident `_y` prefix FIX is the first MACHINE-APPLICABLE (VERIFIED) fix — the `_`-silencing
           rule makes renaming the binder to `_y` behaviour-preserving and clears the warning by
           construction, so an agent applies it without review — now expressed via the diagnostic-quality
           `(warning …)` clause. (fix migrated from rcdzc an_unused_binding_carries_a_verified_underscore_prefix_fix.)")
  (input (do (def (f x y) x) (def (main) (f 7 8)) (export main)))
  (output (: 7 Int64))
  ; exactly ONE CDZ0306 — the def-parameter loop fires once and is NOT double-reported by the
  ; anonymous-lambda unused-parameter pass on the def's signature (rcdzc an_unused_anonymous_lambda…'s
  ; no-double-report facet, migrated here).
  (count 1)
  (warning CDZ0306 (message "unused parameter") (fix (kind replace) (replacement "_y") (verified))))

(case
  "an unused ANONYMOUS-LAMBDA parameter surfaces the CDZ0306 unused-parameter warning + `_`-prefix fix"
  (doc
    "The closure face of the unused-parameter warning: an anonymous `(fn ((: x Int64)) 5)` lambda never
           references its parameter `x`, so the program still runs (`(f 3)` = 5) but the compiler emits the
           SAME CDZ0306 `unused parameter` warning + `_x` fix as a def parameter — the lambda-parameter usage
           scan is a DISTINCT code path from the def-parameter loop, so it earns its own pin. Only the stable
           lead `unused parameter` is pinned (the name `x` is in the dynamic tail). Wasm-graded (warnings ride
           the shared compile stage; rust/rust-async run paths skip the warns check). (Migrated from rcdzc
           an_unused_anonymous_lambda_parameter_warns.)")
  (input (do (def (main) (let ((f (fn ((: x Int64)) 5))) (f 3))) (export main)))
  (output (: 5 Int64))
  (warning CDZ0306 (message "unused parameter") (fix (kind replace) (replacement "_x"))))

(case
  "a USED anonymous-lambda parameter is clean — no unused-parameter warning"
  (doc
    "The false-positive guard for the lambda-parameter scan: `(fn ((: x Int64)) x)` references `x` in its
           body, so no CDZ0306 fires. (Migrated from rcdzc an_unused_anonymous_lambda_parameter_warns clean case.)")
  (input (do (def (main) (let ((f (fn ((: x Int64)) x))) (f 3))) (export main)))
  (output (: 3 Int64))
  (no-diagnostic "unused parameter"))

(case
  "an `_`-prefixed anonymous-lambda parameter is silenced — no unused-parameter warning"
  (doc
    "The `_`-silencing convention applies to lambda parameters too: `(fn ((: _x Int64)) 5)` intentionally
           ignores `_x`, so no CDZ0306 fires. (Migrated from rcdzc an_unused_anonymous_lambda_parameter_warns
           clean case.)")
  (input (do (def (main) (let ((f (fn ((: _x Int64)) 5))) (f 3))) (export main)))
  (output (: 5 Int64))
  (no-diagnostic "unused parameter"))

(case
  "applying the unused-parameter underscore fix (`y` to `_y`) silences the warning"
  (doc
    "The VERIFIED fix's correctness, demonstrated: renaming the unused parameter to `_y` compiles +
           runs identically (`(f 7 8)` = 7). The `_`-prefix is the silencing rule, so the completed form is
           behaviour-preserving — what makes the fix machine-applicable.")
  (input (do (def (f x _y) x) (def (main) (f 7 8)) (export main)))
  (output (: 7 Int64)))

; The DEFINITION face of the unused-binding warning: a top-level `def` that NOTHING references and is NOT
; exported is dead code → CDZ0306 `unused definition` + a `_`-prefix fix. A def that IS referenced, or that is
; exported (a reachable entry), is used and does not warn. (Migrated from rcdzc
; an_unused_nonexported_definition_warns_but_a_used_or_exported_one_does_not.)
(case
  "an unused non-exported definition compiles and runs but the build surfaces a CDZ0306 unused-definition warning"
  (input (do (def (helper) (: 9 Int64)) (def (main) 42) (export main)))
  (output (: 42 Int64))
  (count 1)
  (warning CDZ0306 (message "unused definition") (fix (kind replace) (replacement "_helper"))))

(case
  "a REFERENCED non-exported definition is used and does not warn"
  (input (do (def (helper) (: 9 Int64)) (def (main) (helper)) (export main)))
  (output (: 9 Int64))
  (no-diagnostic "unused definition"))

(case
  "an EXPORTED definition is a reachable entry and does not warn unused"
  (input (do (def (helper) (: 9 Int64)) (export helper)))
  (call helper)
  (output (: 9 Int64))
  (no-diagnostic "unused definition"))

(case
  "a RECURSIVE function's parameter used only in the recursive call is NOT falsely flagged unused"
  (doc
    "The used-parameter analysis must count a reference inside the function's OWN recursive call: `sm`'s
           parameter `n` is used in `(= n 0)`, `(+ n …)`, and `(sm (- n 1))` — recursion must not confuse the
           usage scan into a spurious CDZ0306. `(sm 5)` = 5+4+3+2+1 = 15. (Migrated from rcdzc
           a_recursive_functions_used_parameter_is_not_flagged_unused; the truly-unused-param face is the
           unused-parameter warning case above.)")
  (input
    (do (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1))))) (def (main) (sm 5)) (export main)))
  (output (: 15 Int64))
  (no-diagnostic "unused"))

(case
  "an argument bound to a used parameter IS observed, so its trap occurs (the anchor)"
  (doc
    "The control: `(def (f x y) y)` returns its SECOND parameter, so `(f 7 (/ 1 d))` with d = 0
           observes the trapping argument — its value flows out as the result — and must trap. Pins that
           the elision above is specifically about an argument whose parameter is UNUSED; the trap fires
           the moment the argument is observed. The call-boundary dual of the projected-tuple-element
           anchor in 05-compound-types.sexp.")
  (input (do (def (f x y) y) (def (main (: d Int64)) (f 7 (/ 1 d))) (export main)))
  (call main (: 0 Int64))
  (trap "division by zero"))

; ── The pipeline operator `|>` threads a value into a function ───────────────────────────────────────
; `|>` is a REAL operator (arena head `|>`), not surface sugar: it round-trips through both syntaxes and
; the resolver rewrites `(|> L R)` into an ordinary application, threading `L` as `R`'s FIRST argument —
; `(|> x f)` = `(f x)`, and `(|> x (f a))` = `(f x a)`. Because the rewrite yields a plain application,
; the value flows through the same typing, folding, and emission as a written-out call; the two forms are
; INDISTINGUISHABLE downstream. Threading first (not last) matches the collection-first argument order of
; the built-in operations (`(List.map xs f)`), so `(|> xs (List.map f))` reads as "xs, mapped by f".
; `|>` binds looser than every operator but ascription and is left-associative, so a chain reads left to
; right: `(|> (|> x f) g)` = `g(f(x))`.
(case
  "the pipeline operator threads a value into a named function"
  (doc
    "`(|> 5 double)` resolves to the application `(double 5)`: the piped value becomes the sole
           argument. `|>` is the pipeline operator — a real form the resolver rewrites into an ordinary
           application, so the value is typed and folded exactly as a written-out `(double 5)` is.")
  (input (do (def (double n) (* n 2)) (def (main) (|> 5 double)) (export main)))
  (output (: 10 Int64)))

(case
  "the pipeline operator splices the value as a call's first argument"
  (doc
    "`(|> 3 (add 10))` resolves to `(add 3 10)`: when the right operand is already an application,
           the piped value is spliced in as its FIRST argument and the written arguments follow. This is
           the argument order that lets `(|> xs (op …))` read as an operation on `xs`.")
  (input (do (def (add a b) (+ a b)) (def (main) (|> 3 (add 10))) (export main)))
  (output (: 13 Int64)))

(case
  "a pipeline chain applies its stages left to right"
  (doc
    "`(|> (|> 5 double) (add 1))` = `(add (double 5) 1)` = 11. `|>` is left-associative and looser
           than the other operators, so a chain of pipes reads as a left-to-right sequence of stages —
           the value out of one stage is the value into the next.")
  (input
    (do
      (def (double n) (* n 2))
      (def (add a b) (+ a b))
      (def (main) (|> (|> 5 double) (add 1)))
      (export main)))
  (output (: 11 Int64)))

; RECURSIVE-GENERIC MONOMORPHIZATION — a recursive function used at more than one type is INSTANTIATED
; more than once. A non-recursive generic function already monomorphizes by inlining (β-reduction at each
; call site IS specialization); a RECURSIVE one cannot inline (it would not terminate), so it lowers to a
; real function. When such a function is GENERIC — a parameter the body only threads, never constraining
; to a concrete type — the compiler synthesizes ONE specialized copy per distinct concrete instantiation
; (`glossary.md §Monomorphization`: "concrete specializations by the same compile-time reduction … done
; before emitting a component interface because generics do not cross the boundary"). Each copy emits as
; an ordinary monomorphic function with its own machine valtypes; two calls at the SAME type share one.
; A NON-RECURSIVE generic PRODUCER — a function that BUILDS a generic value from a generic input but does
; NOT recurse — monomorphizes by ordinary inlining (β-reduction at the call site IS specialization), so it
; works at MULTIPLE element types with no special machinery: `(wrap1 x) = (Box.Wrap x)` is `∀a. a → Box a`,
; and `(wrap1 5)` / `(wrap1 "ab")` each inline to a concrete construction. This is the contrast case to the
; RECURSIVE generic producer (which cannot inline and needs the result-element tie the monomorphizer does
; not yet make — the `List a -> Iter a` ≥2-type limit): non-recursive producers are unaffected because the
; call site's concrete argument type flows straight into the inlined body. Pins that a non-recursive generic
; producer composes at ≥2 element types (Int64 + String) — the boundary of the recursive-producer gap.
(case
  "a non-recursive generic producer composes at two element types via inlining"
  (doc
    "`wrap1 : a -> Box a` builds a user generic sum WITHOUT recursing, so it monomorphizes by
           inlining at each call site — no result-element tie needed. Used at Int64 (`(wrap1 n)`) AND
           String (`(wrap1 \"ab\")`) in one program, each `(unwrap (wrap1 …))` inlines to its concrete type.
           `unwrap(wrap1(n)) + byte-len(unwrap(wrap1(\"ab\")))` = `n + 2`. Pins that a NON-recursive generic
           producer composes at ≥2 element types (unlike the recursive `List a -> Iter a` producer, which is
           gated on the not-yet-built result-element tie) — the boundary of that gap.")
  (input
    (do
      (type Box (Wrap a))
      (def (wrap1 x) (Box.Wrap x))
      (def (unwrap b) (match b ((Box.Wrap v) v)))
      (def (main (: n Int64)) (+ (unwrap (wrap1 n)) (String.byte-len (unwrap (wrap1 "ab")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

(case
  "a generic newtype instantiated at a FUNCTION type carries a closure through erasure"
  (doc
    "The generic single-variant newtype `(type Box (Wrap a))` above erases its payload (`newtype_inner`
           decodes `a` at each instantiation). The Int64/String cases pin SCALAR/heap instantiations; this
           pins `a` bound to a FUNCTION type — `(Box (-> Int64 Int64))`. The parameter must decode to an
           ARROW at instantiation, and the closure the newtype wraps must survive the erasure (the newtype
           carries no runtime box, so the closure value IS the erased inner) and remain applyable after the
           `(Box.Wrap f)` match binder extracts it. `(applyBox (Box.Wrap (fn (x) (+ x 1))) 41)` extracts the
           wrapped closure and applies it → 42. Distinct from the declared-arrow VARIANT payload
           `(type T (Mk (-> Int64 Int64)))` above: there the arrow is the variant's own declared payload,
           whereas here it is a generic type PARAMETER decoded to an arrow through the erasable-newtype
           instantiation — the newtype-erasure-over-a-function-type path. A generation that read the type
           param as a nullary/scalar, or that boxed the erased newtype, would mis-apply or invalid-wasm.")
  (input
    (do
      (type Box (Wrap a))
      (def (applyBox (: b (Box (-> Int64 Int64))) (: n Int64)) (match b ((Box.Wrap f) (f n))))
      (def (main) (applyBox (Box.Wrap (fn ((: x Int64)) (+ x 1))) 41))
      (export main)))
  (output (: 42 Int64)))

(case
  "a recursive generic function is instantiated at two different types"
  (doc
    "`loopn` counts `n` down, threading `x` UNCHANGED — so `x` is generic (the body never fixes its
           type). Called at Int64 (`(loopn 3 40)` → 40, an i64 slot) AND at String (`(loopn 2 \"hi\")` →
           \"hi\", an i32 heap handle), it is MONOMORPHIZED into two functions with distinct machine
           signatures. Before recursive-generic monomorphization the second use was rejected CDZ0203
           (`x` pinned to Int64 by the first call). `byte-len(\"hi\") = 2`, so `40 + 2 = 42`.")
  (input
    (do
      (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))
      (def (main) (+ (loopn 3 40) (String.byte-len (loopn 2 "hi"))))
      (export main)))
  (output (: 42 Int64)))

(case
  "a recursive generic function called at one type twice shares a single instantiation"
  (doc
    "The dedup companion: `loopn` called at Int64 in BOTH `(loopn 3 40)` and `(loopn 2 2)` is
           instantiated ONCE — the two calls share a single monomorphic function (keyed by the concrete
           type), not two copies. `40 + 2 = 42`. Pins that monomorphization is per-TYPE, not per-call:
           the same instantiation is reused, so a program that calls a generic recursive helper at one
           type many times emits one function for it.")
  (input
    (do
      (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))
      (def (main) (+ (loopn 3 40) (loopn 2 2)))
      (export main)))
  (output (: 42 Int64)))

; The same generic recursive `loopn`, now instantiated at THREE distinct machine shapes in one program —
; Int64 (an i64 slot), String (an i32 heap handle), and Bool (an i32 discriminant). Each is monomorphized
; into its own function with the matching valtypes; the three copies coexist. Extends the two-type case to
; confirm the per-type specialization count is not capped at two and that a heap-handle (String) and a
; discriminant (Bool) instantiation live alongside the scalar one. `loopn 2 k = k`; `byte-len(loopn 1
; "ab") = 2`; `loopn 1 true` is true → the `if` takes 100. So `k + 2 + 100 = k + 102`.
(case
  "a recursive generic function is instantiated at three distinct machine shapes"
  (doc
    "`loopn` (threads its second arg unchanged, so it is generic) is called at Int64, String, AND
           Bool in one program — three distinct machine shapes (i64 slot / i32 heap handle / i32
           discriminant), each monomorphized into its own function. `loopn 2 k = k`; `byte-len(loopn 1
           \"ab\") = 2`; `loopn 1 true` → true so the `if` takes 100. With runtime `k`: `k + 2 + 100`.
           Pins that recursive-generic monomorphization scales past two instantiations and that a
           heap-handle and a discriminant copy coexist with the scalar one.")
  (input
    (do
      (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x)))
      (def
        (main (: k Int64))
        (+ (loopn 2 k) (+ (String.byte-len (loopn 1 "ab")) (if (loopn 1 true) 100 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 107 Int64))
  (call main (: 40 Int64))
  (output (: 142 Int64)))

; TRANSITIVE recursive-generic monomorphization — a generic recursive function that CALLS another
; generic recursive function, threading its own generic parameter, is itself generic (its result type is
; the callee's, which is the threaded param's). Genericity propagates through the call graph: the inner
; `idr` is called at only ONE syntactic site (`(idr 2 y)`), yet is generic because `wrap` feeds it a
; generic value, and `wrap`'s result stays connected to its parameter so `wrap` too is generic. Both are
; then monomorphized per concrete type at the OUTERMOST call sites.
(case
  "a recursive generic function threading another generic is itself generic at two types"
  (doc
    "`wrap` recurses on `m`, threading `y` UNCHANGED, and at its base calls a SECOND generic
           recursive function `idr` (also threading its arg). `wrap`'s result is `idr`'s result is
           `y`'s type — so `wrap` is generic in `y`, even though `idr` has a single call site. Called at
           Bool (`(wrap 1 true)`) and Int64 (`(wrap 2 40)`), BOTH `wrap` and `idr` are monomorphized at
           each type (four specialized functions). Before transitive genericity, `idr`'s param pinned to
           the first type and the second use was rejected CDZ0203. `(wrap 1 true)` is true → `(wrap 2 40)`
           = 40.")
  (input
    (do
      (def (idr (: n Int64) x) (if (= n 0) x (idr (- n 1) x)))
      (def (wrap (: m Int64) y) (if (= m 0) (idr 2 y) (wrap (- m 1) y)))
      (def (main) (if (wrap 1 true) (wrap 2 40) 99))
      (export main)))
  (output (: 40 Int64)))

; Recursive-generic monomorphization reaches EVERY recursive-def flavor, not just top-level defs: a
; MUTUALLY-recursive generic group and a DO-LOCAL generic function are each instantiated once per
; concrete type at their call sites, exactly as a top-level generic is. The do-local case needs the
; specialized copy's self-call to stay resolved to the original def (a do-local name resolves by lexical
; scope, which the re-parented copy escapes) — the copy SHARES the pinned self-call occurrence.
(case
  "a mutually-recursive generic group is instantiated per type"
  (doc
    "`ping`/`pong` mutually recurse, each threading a generic second argument unchanged. Called at
           Bool (`(ping 3 true)`) and Int64 (`(pong 2 40)`), BOTH functions are monomorphized at BOTH
           types — the cross-calls re-resolve by name and re-enter specialization at the same
           instantiation. `(ping 3 true)` bounces ping→pong→ping→pong ending at the base with `true`, so
           the `if` takes `(pong 2 40)` = 40.")
  (input
    (do
      (def (ping (: n Int64) x) (if (= n 0) x (pong (- n 1) x)))
      (def (pong (: n Int64) x) (if (= n 0) x (ping (- n 1) x)))
      (def (main) (if (ping 3 true) (pong 2 40) 99))
      (export main)))
  (output (: 40 Int64)))

(case
  "a do-local generic function is instantiated per type"
  (doc
    "A do-local `(def (idr n x) …)` threading a generic `x`, called at Bool and Int64 within the
           same `do` block, is monomorphized per type. A do-local name resolves by LEXICAL do-scope, so
           the specialized copy's self-call must stay resolved to the original def (the re-parented copy
           escapes that scope) — the copy shares the pinned self-call. `(idr 1 true)` = true → `(idr 2
           40)` = 40.")
  (input
    (do
      (def
        (main)
        (do
          (def (idr (: n Int64) x) (if (= n 0) x (idr (- n 1) x)))
          (if (idr 1 true) (idr 2 40) 99)))
      (export main)))
  (output (: 40 Int64)))

; The canonical generic-recursion idiom: a recursive function over a USER-DEFINED GENERIC RECURSIVE SUM
; type (a polymorphic linked list `(type Lst Nil (Cons a (Lst a)))`), called at more than one element
; type. `len` threads down the list's tail generically — its element type is never fixed by the body —
; so it is monomorphized once per element type at its call sites (a `Lst Int64` length and a `Lst String`
; length), exactly as a generic scalar-threading function is. This is recursive-generic monomorphization
; over the real recursive-data idiom, not just a scalar pass-through. (An explicit polymorphic annotation
; `(: l (Lst a))` is a SEPARATE not-yet-built feature — binding a type variable in a signature; here `len`
; is unannotated and inference carries the element type, which is the idiomatic form.)
(case
  "a recursive function over a generic recursive sum is monomorphized per element type"
  (doc
    "`(type Lst Nil (Cons a (Lst a)))` is a polymorphic linked list; `len` counts its elements,
           recursing on the tail without ever constraining the element type. Called on a `Lst Int64`
           (length 2) and a `Lst String` (length 3), `len` is monomorphized into one function per element
           type — the recursive-data analogue of the scalar `loopn` case. 2 + 3 = 5.")
  (input
    (do
      (type Lst Nil (Cons a (Lst a)))
      (def (len l) (match l ((Lst.Nil) 0) ((Lst.Cons h t) (+ 1 (len t)))))
      (def
        (main)
        (+
          (len (Lst.Cons 1 (Lst.Cons 2 Lst.Nil)))
          (len (Lst.Cons "a" (Lst.Cons "b" (Lst.Cons "c" Lst.Nil))))))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

; The companion of the case above: writing the EXPLICIT polymorphic annotation `(: l (Lst a))` on the
; same generic `len` — a type VARIABLE `a` nested inside the generic constructor `Lst` in a parameter
; annotation. Cadenza has no ∀-binder in an annotation (an annotation names an EXISTING type), so the
; lowercase `a` in type position is an unbound name → CDZ0101. This is NOT the "type, not a function"
; misread it once gave (which read `(Lst a)` as a call); the diagnostic now names the real situation and
; the route — leave the parameter UNANNOTATED (inference already carries the element type, as the case
; above shows) or annotate a concrete type. Pins the rejection of the "type-variable-in-signature"
; not-yet-built feature at its nested-in-a-generic form (the issue repro), so the good diagnostic and the
; decline are locked against regression; the bare-type-var forms `(: 5 foo)` / `(: 5 Foo)` are pinned in
; 07-type-system.sexp.
(case
  "a type variable nested in a generic parameter annotation is an unbound name"
  (doc
    "`(def (len (: l (Lst a))) …)` annotates the parameter with `(Lst a)`, where `a` is meant as a
           type variable — but Cadenza binds a signature's type variables through NO form: an annotation's
           type position names an existing type, and there is no ∀-binder. The lowercase `a` is therefore
           an unbound name in type position (CDZ0101), carrying the generic-route hint (leave it
           unannotated — inference is already polymorphic — or write a concrete type). The idiomatic
           spelling is the unannotated `(def (len l) …)` of the case above, which monomorphizes per element
           type. Pins the explicit-polymorphic-annotation rejection at its nested-in-a-generic form.")
  (input
    (do
      (type Lst (Nil) (Cons a (Lst a)))
      (def (len (: l (Lst a))) (match l ((Lst.Nil) 0) ((Lst.Cons _ t) (+ 1 (len t)))))
      (def (main) (len (Lst.Cons 1 (Lst.Nil))))
      (export main)))
  (error CDZ0101))

; A recursive-generic PRODUCER composed with an element-CONSUMING consumer. The cases above thread a
; generic value UNCHANGED or CONSUME a generic input to a concrete result; this one BUILDS a generic
; recursive result (`mapl : (a -> b) -> List a -> List b`, transforming each element through a callback)
; and then consumes that result at a concrete element type (`suml` sums it). The producer's result-list
; element must be shaped from its scrutinee's element (the list-pattern shape) so the consumer can pin it:
; a producer whose match pattern left the parameter unshaped grounded to `Any` and the whole scheme
; DECLINED (the parameter-shape gap this exercises). `mapl (fn (x) (+ x 1)) [n,n,n]` = `[n+1,n+1,n+1]`,
; and `suml` of that is `3·(n+1)` — with a runtime boundary `n`, so the map + fold run at run time (a real
; call_indirect over the lifted callback, no fold). Single element type here (Int64); the ≥2-element-type
; producer instantiation is a separate not-yet-built monomorphization tie.
(case
  "a recursive-generic producer's result is consumed by an element-typed consumer"
  (doc
    "`mapl` builds a `List b` by applying a callback to each element of a `List a` (a recursive-
           generic PRODUCER — its result element `b` comes from the callback, not threaded), and `suml`
           then sums that result list — consuming its element at a concrete type. The producer's parameter
           and result-list must be shaped `List _` from the list pattern for the composition to type; an
           unshaped parameter grounded to `Any` and declined. `suml(mapl(fn(x) => x+1, [n,n,n]))` =
           `3·(n+1)`; with runtime `n` the map + fold execute (call_indirect over the lifted callback).
           Pins the recursive-generic producer→consumer composition at a single element type.")
  (input
    (do
      (def (mapl f xs) (match xs (#list() #list()) (#list(h (.. t)) (List.push (mapl f t) (f h)))))
      (def (suml xs) (match xs (#list() 0) (#list(h (.. t)) (+ h (suml t)))))
      (def (main (: n Int64)) (suml (mapl (fn (x) (+ x 1)) #list(n n n))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 6 Int64))
  (call main (: 4 Int64))
  (output (: 15 Int64))
  (live-objects known-leak))

; The same single-instantiation recursive-generic producer→consumer composition, but the produced element
; is a USER-DEFINED generic sum (`(type Box (Wrap a))`) rather than the built-in `List` — `wrapall : List a
; -> List (Box a)` wraps each element, and `sumfirst` unwraps + sums them. This exercises the same
; list-pattern shaping (the producer's `xs` shapes `List _`, and its result `List (Box _)` carries the
; element through the `Box` wrap) over a USER sum's payload, not only the built-in collection element. At a
; single element type (Int64) it monomorphizes and runs: `sumfirst(wrapall([n,n,n]))` = `3·n`.
(case
  "a recursive-generic producer wrapping elements in a user sum is consumed at one type"
  (doc
    "`wrapall : List a -> List (Box a)` wraps each list element in a USER generic sum `(Box a)` (a
           recursive-generic PRODUCER whose result element is `Box a`), and `sumfirst` unwraps each `Box`
           and sums the payloads. The producer's parameter shapes `List _` from its list pattern and its
           result carries the element through the `Box.Wrap` construction; at a single element type the
           composition monomorphizes and runs. `sumfirst(wrapall([n,n,n]))` = `3·n`. Pins the
           producer→consumer path over a USER sum's payload (not just the built-in List element), the
           user-type companion of the `mapl`/`suml` case above.")
  (input
    (do
      (type Box (Wrap a))
      (def
        (wrapall xs)
        (match xs (#list() #list()) (#list(h (.. t)) (List.push (wrapall t) (Box.Wrap h)))))
      (def (unwrap1 b) (match b ((Box.Wrap v) v)))
      (def (sumfirst xs) (match xs (#list() 0) (#list(h (.. t)) (+ (unwrap1 h) (sumfirst t)))))
      (def (main (: n Int64)) (sumfirst (wrapall #list(n n n))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 6 Int64))
  (call main (: 4 Int64))
  (output (: 12 Int64))
  (live-objects known-leak))

; THE RECURSIVE-GENERIC PRODUCER TIE (was the known ≥2-type limit; LANDED): `from-list : List a -> Iter a`
; builds a generic `Iter` from a generic `List` — a recursive-generic PRODUCER. Its result element must be
; TIED to its argument element (`List a -> Iter a`, ONE `a`), so it can be MONOMORPHIZED independently at
; each element type. The tie was previously SEVERED — `compute_def_scheme`'s body solve types the arm
; `(Iter.Cons h (from-list t))` through `apply_type`, whose recursive-call arg-freshen renamed the param's
; `a` to a fresh var → the scheme inferred `∀a b. List a -> Iter b`, and composing `icount(from-list(xs))`
; at BOTH Int64 AND String had no single value for the loose result var → CDZ0201. FIX: during the scheme's
; body solve, the def's OWN parameter type vars are RIGID (`db.scheme_rigid_vars` → `freshen_free_except`
; preserves them), so the recursive call keeps `a` tied while a genuinely-fresh local placeholder (`(None)`,
; `Map.empty`) still freshens — the var-PROVENANCE distinction. Now `from-list` infers `∀a. List a -> Iter
; a` and composes at ≥2 element types, compiling + running. (The mutual-recursion group variant is a
; strictly-harder follow-up — no shared subst across the group yet.)
(case
  "a recursive-generic producer composes at two element types (the tied result var)"
  (doc
    "`from-list : List a -> Iter a` builds a generic `Iter` from a generic `List` — a recursive-generic
           PRODUCER whose result element is TIED to its argument's (`∀a. List a -> Iter a`, was the severed
           `∀a b` ≥2-type limit). Composing `icount(from-list(xs))` at BOTH Int64 (a runtime `[n, n+1]`) AND
           String (`[\"a\",\"b\",\"c\"]`) in one program now monomorphizes `from-list`/`icount` independently
           at each element type and runs: with n=5 → icount[5,6]=2 + icount[\"a\",\"b\",\"c\"]=3 = 5; n=10 →
           2 + 3 = 5. Pins the producer element tie (the scheme-solve rigid-param-var fix): the recursive
           call keeps `a` tied, so the two instantiations each bind a concrete element.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main (: n Int64))
        (+ (icount (from-list #list(n (+ n 1)))) (icount (from-list #list("a" "b" "c")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 10 Int64))
  (output (: 5 Int64))
  (live-objects known-leak))

; A recursive-generic PRODUCER applied to a list whose ELEMENTS are themselves results of the SAME
; producer — `(from-list (list (inner) (inner)))` where `(inner) = (from-list (list 1 2)) : Iter Int64`,
; so the outer call is `from-list` at element type `Iter Int64` (a self-nested `Iter (Iter Int64)`). This
; used to DECLINE (CDZ0201) and now RUNS (`icount` → 2): typing the outer arg re-enters `from-list`'s OWN
; scheme/param solve (still on the stack), so the re-entry guard types the element `(inner)` as `Any` → the
; list is momentarily `List Any`. The re-entrancy FIX (v-inference): `type_of`'s memo guard does NOT cache a
; provisional NESTED-`Any` in a DATA-CONTAINER element (`(List Any)`) born while a def's solve is on the
; stack AND the node is EXTERNAL to every in-flight def's body (a caller re-entering the producer's solve,
; here in `main`) — so a later CLEAN read, after `from-list`'s scheme completes, recomputes the grounded
; `(List (Iter Int64))` and the outer call monomorphizes. Scoped so it never touches a MONOMORPHIC recursive
; def's OWN self-call result (typed INSIDE that def's body — internal), whose concrete `Ty::Sum` the rust
; sum-match emit depends on (a blunter skip turned a bottom-up fold's clean decline into a rust miscompile);
; and gated to a data element (not a function arrow, whose `Any` is a closure hole the transformer-closure
; tie grounds). A CONCRETE-element list (`(list 1 2)`) was always fine. This is v-iterators' scan/flatten
; nested-generic residual root cause — the un-annotated form now joins the annotated twin below at 2.
(case
  "a recursive-generic producer over a list of its own generic-producer results monomorphizes and runs"
  (doc
    "`(from-list (list (inner) (inner)))` where `(inner) = (from-list (list 1 2)) : Iter Int64` applies
           `from-list` at element type `Iter Int64` (a self-nested `Iter (Iter Int64)`). Typing the outer
           arg re-enters `from-list`'s own in-flight scheme solve, so the element `(inner)` momentarily types
           `Any` and the list `(List Any)`; the memo guard does NOT cache that provisional data-element `Any`
           born external to the in-flight producer's body, so a later clean read (after the scheme completes)
           grounds it to `(List (Iter Int64))` and the outer call monomorphizes. `icount` of the doubly-nested
           iter (2 outer elements) = 2. Previously an honest CDZ0201 decline pending this re-entrancy fix; now
           it joins the annotated twin below (an explicit annotation is no longer required to ground it).")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def (inner) (from-list #list(1 2)))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (from-list #list((inner) (inner)))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "an annotation on a nested generic-call argument grounds the self-nested producer and it runs"
  (doc
    "The WORKAROUND for the decline above (the one the CDZ0201 message names): annotating a nested
           generic-call argument with its concrete type grounds the element BEFORE the outer producer's
           scheme solve needs it, so the program compiles and runs. `(from-list (list (: (inner) (Iter
           Int64)) (inner)))` — the first `(inner)` carries `(: … (Iter Int64))`, which pins the list's
           element to `Iter Int64`, so the outer `from-list` is `from-list` at `Iter Int64` and `icount` of
           the 2-element doubly-nested iter is 2. Pins that the annotation escape hatch works (the decline is
           NOT a hard wall — an author can ground it), the runnable twin of the decline case above; when the
           re-entrancy fix lands, the un-annotated form joins this one at 2.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def (inner) (from-list #list(1 2)))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (from-list #list((: (inner) (Iter Int64)) (inner)))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

; A recursive-generic TRANSFORMER whose ELEMENT is itself the SAME nested generic — `flatten : Iter(Iter a)
; -> Iter a`, `(match it ((Iter.Nil)(Iter.Nil)) ((Iter.Cons h rest)(append h (flatten rest))))`, chaining
; each inner `Iter a` via `append`. At a SINGLE element type (Int64) the emit path monomorphizes it and it
; RUNS: `flatten([[1,2],[3,4,5]])` = `[1,2,3,4,5]`, `icount` = 5. This is the recursive-transformer-over-
; nested-generic shape — the transformer analogue of the self-nested PRODUCER above, and the single-type
; base of v-iterators' flatten. (Flatten at TWO element types in one program is a KNOWN residual decline —
; the recursive-transformer element/closure tie at ≥2 instantiations — a tracked inference follow-up; this
; case pins the single-type composition, which the emit path realizes correctly.)
(case
  "a recursive-generic flatten over a nested generic iterator monomorphizes and runs at one element type"
  (doc
    "`flatten : Iter(Iter a) -> Iter a` concatenates a nested iterator's inner iterators via a
           recursive `append`, the archetypal monadic-join / `concatMap` core. At a single element type
           (Int64) it monomorphizes and runs: `flatten([[1,2],[3,4,5]])` flattens to a 5-element iter, so
           `icount` = 5. Pins the recursive-generic TRANSFORMER-over-nested-generic shape (the transformer
           analogue of the self-nested producer above) at a single type; the TWO-element-type composition
           is pinned in the case just below (it now runs too).")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def (append xs ys) (match xs ((Iter.Nil) ys) ((Iter.Cons h t) (Iter.Cons h (append t ys)))))
      (def
        (flatten it)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (append h (flatten rest)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (icount (flatten (from-list #list((from-list #list(1 2)) (from-list #list(3 4 5)))))))
      (export main)))
  (output (: 5 Int64))
  (live-objects known-leak))

; A recursive-generic flatten `Iter(Iter a) -> Iter a` composing at TWO element types in one program — the
; transformer-over-nested-generic tie at ≥2 instantiations. This DECLINED (the untied nested-generic tie:
; `flatten`'s scheme inferred `(-> (Iter a) (Iter b))`, the domain-inner element and the result element as
; disconnected vars). FIXED by the payload-binder-to-callee constraint: `flatten`'s Cons arm passes `h`
; (its element, an `Iter a`) to the generic `append`, whose domain (`Iter _`) now unifies with `h`'s type
; through the param solve — so `it`'s element is pinned `Iter _` (`it : Iter(Iter _)`) and, via append's
; result=domain-element tie, the result is `Iter _`, giving the correct scheme `(-> (Iter (Iter a)) (Iter
; a))`. Both element types monomorphize independently: `flatten([[1,2],[3,4,5]])` (Int, icount 5) plus
; `flatten([["a","b"],["c"]])` (String, icount 3) = 8. The append-delegate member of the recursive-generic
; transitive-tie family (its accumulator-seeded siblings reduce/reverse remain a tracked residual).
(case
  "a recursive-generic flatten over a nested generic iterator composes at two element types"
  (doc
    "`flatten : Iter(Iter a) -> Iter a` at TWO element types in one program. Its Cons arm threads its
           element `h` (an `Iter a`) into the generic `append`; the scheme solve ties `append`'s domain to
           `h`'s type, so `it`'s element is pinned to `Iter _` (`it : Iter(Iter _)`) and the result to
           `Iter _` — the correct `(-> (Iter (Iter a)) (Iter a))` scheme, monomorphized per element type.
           `flatten([[1,2],[3,4,5]])` (Int64, icount 5) plus `flatten([[\"a\",\"b\"],[\"c\"]])` (String,
           icount 3) = 8. Was an untied-nested-generic decline; now composes.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def (append xs ys) (match xs ((Iter.Nil) ys) ((Iter.Cons h t) (Iter.Cons h (append t ys)))))
      (def
        (flatten it)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (append h (flatten rest)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (+
          (icount (flatten (from-list #list((from-list #list(1 2)) (from-list #list(3 4 5))))))
          (icount (flatten (from-list #list((from-list #list("a" "b")) (from-list #list("c"))))))))
      (export main)))
  (output (: 8 Int64))
  (live-objects known-leak))

; The SOUNDNESS SENTINEL for the tie above: broadening `collect_param_constraints` to accept a SumPayload-of-
; a-param arg (so the element flows into the callee's domain) must NOT loosen type-safety — a threaded element
; whose type is INCOMPATIBLE with the callee's declared parameter must still be REJECTED, not wrongly accepted.
(case
  "a recursive transformer passing its element to an incompatibly-typed callee is rejected"
  (doc
    "`flatten-bad`'s `Cons` arm passes its element `h` (an `Iter a`) to `sum-ints : (Iter Int64) → Int64`,
           but `flatten-bad` is applied at `Iter String`, so `h` is an `Iter String` — incompatible with the
           `Iter Int64` `sum-ints` demands. The SumPayload-of-a-param constraint (which lets the composing
           `flatten` above tie correctly) propagates this as a genuine TYPE ERROR (CDZ0203), NOT a silent
           accept: the broadening ties where the types connect and REJECTS where they clash. Pins that the
           recursive-transformer element-tie did not weaken the callee-domain check — the negative companion of
           the composing case above, on both backends.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (sum-ints (: it (Iter Int64)))
        (match it ((Iter.Nil) 0) ((Iter.Cons h t) (+ h (sum-ints t)))))
      (def
        (flatten-bad it)
        (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ (sum-ints h) (flatten-bad rest)))))
      (def (main) (flatten-bad (from-list #list((from-list #list("a" "b"))))))
      (export main)))
  (error CDZ0203 (exact-code)))

; The COMPLEMENT of the producer tie above — a recursive-generic TRANSFORMER that threads a CLOSURE:
; `(gmap it f) = (Cons (f h) (gmap rest f))` maps `f` over each element, principal type
; `∀a b. (Iter a) → (a → b) → (Iter b)` with the closure DOMAIN `a` tied to the element `it` carries. The
; scheme-solve makes that tie (the `(Iter.Cons h rest)` pattern binds `h` to `it`'s element var, `(f h)`
; unifies `f`'s domain with it, and a cross-parameter seed-unify pins the shared element var CONSISTENTLY
; across the `it` and `f` parameters), so a `gmap`/`filter` threading a BODY-TYPED closure composes at TWO
; element types. ONE residual gap remains a DECLINE: a closure whose result the body cannot type bottom-up
; — the pure IDENTITY `(fn (s) s)`, whose result is determined ONLY by its domain — still declines CDZ0201
; (the closure-body result←domain flow is the narrower not-yet-built follow-up). The two positive cases
; below pin the fixed tie (body-typed map + predicate filter at two types); the decline case pins the
; residual identity-closure gap (flips to a run when that lands).
(case
  "a recursive-generic transformer threading a closure composes at two element types"
  (doc
    "`gmap : (Iter a) → (a → b) → (Iter b)` maps a closure over a generic `Iter`, recursing
           `(Iter.Cons (f h) (gmap rest f))`. The closure's DOMAIN is tied to the element `it` carries (the
           scheme-solve closure-domain tie), so `gmap` composes at BOTH Int64 (mapping `(+ x 1)` over
           `[1,2,3]`) AND String (mapping `String.concat s s` over `[\"a\",\"b\"]`) in one program, each
           monomorphized independently. `icount` of each mapped iter: 3 + 2 = 5. Pins the recursive-generic
           transformer closure tie — the closure-carrying sibling of the producer element tie.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (+
          (icount (gmap (from-list #list(1 2 3)) (fn (x) (+ x 1))))
          (icount (gmap (from-list #list("a" "b")) (fn (s) (String.concat s s))))))
      (export main)))
  (output (: 5 Int64))
  (live-objects known-leak))

; The AGGREGATE-RESULT face: the threaded closure's RESULT is a compound (a tuple), not a scalar.
; `(fn (x) (tuple x x))` maps Int64 -> (Tuple Int64 Int64) while `(fn (s) (String.concat s s))` maps
; String -> String in the same program, so `gmap` instantiates at two distinct domains AND a closure whose
; body builds a compound. This mis-grounded the closure-result tuple elements to Unit at the OUTER
; generic-call node (rust E0308) until v-inference propagated the structural-aggregate closure-result
; element type (PR 4319); rust acceptance is pinned by a dedicated rcdzc unit test.
(case
  "a recursive-generic transformer maps a closure to an aggregate result at two distinct domains"
  (doc
    "`gmap` threads a closure whose RESULT is an aggregate: `(fn (x) (tuple x x))` (Int64 -> tuple)
           over [1,2] and `(fn (s) (String.concat s s))` (String -> String) over [\"a\",\"b\"], counting
           each mapped iterator = 2 + 2 = 4. Pins the closure-aggregate-result tie at two domains.")
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def
        (from-list xs)
        (match xs (#list() (GIter.Nil)) (#list(h (.. t)) (GIter.Cons h (from-list t)))))
      (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest)))))
      (def
        (gmap it f)
        (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f)))))
      (def
        (main)
        (+
          (count (gmap (from-list #list(1 2)) (fn (x) #tuple(x x))))
          (count (gmap (from-list #list("a" "b")) (fn (s) (String.concat s s))))))
      (export main)))
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "a recursive-generic filter threading a predicate closure composes at two element types"
  (doc
    "`filt : (Iter a) → (a → Bool) → (Iter a)` keeps the elements a predicate closure accepts,
           recursing over a generic `Iter`. The predicate's domain is tied to the element type (the same
           closure-domain tie), so `filt` composes at Int64 (`(> x 1)` over `[1,2,3]` keeps `[2,3]`) AND
           String (`(> (String.byte-len s) 1)` over `[\"a\",\"bb\"]` keeps `[\"bb\"]`) in one program. Counts:
           2 + 1 = 3. Pins that the transformer closure tie covers a PREDICATE closure (Bool result) too, not
           only an element-mapping one.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (filt it p)
        (match
          it
          ((Iter.Nil) (Iter.Nil))
          ((Iter.Cons h rest) (if (p h) (Iter.Cons h (filt rest p)) (filt rest p)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (+
          (icount (filt (from-list #list(1 2 3)) (fn (x) (> x 1))))
          (icount (filt (from-list #list("a" "bb")) (fn (s) (> (String.byte-len s) 1))))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a recursive-generic transformer with a bare-Nil STOP branch composes at two element types"
  (doc
    "`take-while : (Iter a) → (a → Bool) → (Iter a)` keeps a leading run, STOPPING at a bare
           `(Iter.Nil)` when the predicate fails: `(if (p h) (Iter.Cons h (take-while rest p)) (Iter.Nil))`.
           The stop branch is a bare nullary constructor whose element is untied ON THAT PATH; the `if`'s
           result-type join must keep the RESULT element tied to the parameter's (the rigid-biased join),
           else the two coexisting monomorphizations at ≥2 types have a disconnected result var and decline
           CDZ0201. Composes at Int64 (`(< x 3)` over `[1,2,3]` keeps `[1,2]`) AND String
           (`(< byte-len 3)` over `[\"a\",\"bb\",\"ccc\"]` keeps `[\"a\",\"bb\"]`): 2 + 2 = 4. Pins that a
           bare-nullary-leaf stop branch (v-iterators' take-while) does not sever the result-element tie.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (take-while it p)
        (match
          it
          ((Iter.Nil) (Iter.Nil))
          ((Iter.Cons h rest) (if (p h) (Iter.Cons h (take-while rest p)) (Iter.Nil)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (+
          (icount (take-while (from-list #list(1 2 3)) (fn (x) (< x 3))))
          (icount (take-while (from-list #list("a" "bb" "ccc")) (fn (s) (< (String.byte-len s) 3))))))
      (export main)))
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "a type-valued parameter under a function-arrow annotation dispatches an ad-hoc-polymorphic dict"
  (doc
    "AD-HOC POLYMORPHISM via a record of functions, generic over the element type — a `(: t Type)`
           parameter used in the DOMAIN of a function ARROW inside another parameter's annotation:
           `show-with(t: Type, dict: Record(describe: t → Int64), x: t) = dict.describe(x)`. The tie is that
           `t` must reduce to the SAME type variable in the arrow `(-> t Int64)` as in the bare `(: x t)` —
           a `(: g (-> t …))` arrow annotation previously collapsed `t` to `Unit` (the `encode_ty`/`decode_ty`
           round-trip for a built arrow type-value had no `Ty::Var` arm, so the def scheme read `(-> (-> Unit
           Int64) …)` and a real closure argument mismatched). Now the scheme is `(-> Type (-> (Record
           (: describe (-> a Int64))) (-> a Int64)))`, and `show-with` dispatches over BOTH an Int64 instance
           (`describe-int`) AND a Bool instance (`describe-bool`) through the dict in one program:
           `describe-int(5)=5` + `describe-bool(true)=1` = 6. Pins the type-valued-parameter-under-an-arrow
           substitution the ad-hoc-polymorphism chapter (traits = records of functions) depends on.")
  (input
    (do
      (def (describe-int (: n Int64)) n)
      (def (describe-bool (: b Bool)) (if b 1 0))
      (def
        (show-with (: t Type) (: dict (Record (: describe (-> t Int64)))) (: x t))
        (dict.describe x))
      (def
        (main)
        (+
          (show-with Int64 #record((= describe describe-int)) 5)
          (show-with Bool #record((= describe describe-bool)) true)))
      (export main)))
  (output (: 6 Int64))
  (live-objects known-leak))

; take-while BEHAVIORAL edges (breaker): the case above pins the inference TIE (bare-Nil stop branch keeps
; the result-element tie at ≥2 types); these pin the runtime BEHAVIOR the landed giter.cdz @tests (ints,
; strings, one leading-run) don't cover as corpus: the whole-list case where NO element fails (the stop
; branch is never taken before the natural Nil), the empty case where the FIRST element fails (the else
; fires immediately), the CONTENT (a sum, not just a count — the failing element AND its tail must be
; excluded, not merely counted out), and take-while composed AFTER a map (two closure-threading transformers).
(case
  "take-while where every element satisfies the predicate keeps the whole run"
  (doc
    "The all-pass edge: when NO element fails `p`, take-while never takes its bare-Nil stop branch
           before the natural end — it returns the whole list. `take-while [1,2,3] (< 100)` keeps all three,
           icount 3. Pins that the stop branch is reached only on a genuine predicate failure, not spuriously.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (take-while it p)
        (match
          it
          ((Iter.Nil) (Iter.Nil))
          ((Iter.Cons h rest) (if (p h) (Iter.Cons h (take-while rest p)) (Iter.Nil)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (take-while (from-list #list(1 2 3)) (fn (x) (< x 100)))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "take-while where the first element fails the predicate returns the empty run"
  (doc
    "The immediate-stop edge: when the FIRST element fails `p`, the `else` bare-Nil branch fires at once
           and take-while returns the empty iterator. `take-while [5,1,2] (< 3)` — 5 fails, so the run is
           empty, icount 0. Pins that the first-element failure yields Nil (not the whole list, not a
           one-element run) — the boundary the leading-run case above cannot witness.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (take-while it p)
        (match
          it
          ((Iter.Nil) (Iter.Nil))
          ((Iter.Cons h rest) (if (p h) (Iter.Cons h (take-while rest p)) (Iter.Nil)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (take-while (from-list #list(5 1 2)) (fn (x) (< x 3)))))
      (export main)))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "take-while excludes the failing element and everything after it (content, not just count)"
  (doc
    "The content edge: take-while must drop the first failing element AND its entire tail, not merely
           count them out. `take-while [10,20,3,100,5] (> 5)` — 10,20 pass, 3 fails → the run is [10,20];
           SUM = 30 (not 30+100+5, and not including 3). A stop branch that kept the tail, or off-by-one on
           the failing element, would sum wrong. Pins the exact leading-run CONTENT via sum, which the
           count-only landed @tests cannot distinguish from a wrong-but-same-length run.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (take-while it p)
        (match
          it
          ((Iter.Nil) (Iter.Nil))
          ((Iter.Cons h rest) (if (p h) (Iter.Cons h (take-while rest p)) (Iter.Nil)))))
      (def (isum it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ h (isum rest)))))
      (def (main) (isum (take-while (from-list #list(10 20 3 100 5)) (fn (x) (> x 5)))))
      (export main)))
  (output (: 30 Int64))
  (live-objects known-leak))

(case
  "take-while composed after a map threads both transformers' closures"
  (doc
    "take-while consuming the output of another closure-threading transformer: map `(* 10)` over
           [1,2,3,4] gives [10,20,30,40], then `take-while (< 35)` keeps [10,20,30] (40 fails); SUM = 60. Pins
           that two recursive-generic transformers compose — the mapped iterator's element type flows into
           take-while's predicate and result — a stronger tie than take-while over a bare from-list.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (imap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (imap rest f)))))
      (def
        (take-while it p)
        (match
          it
          ((Iter.Nil) (Iter.Nil))
          ((Iter.Cons h rest) (if (p h) (Iter.Cons h (take-while rest p)) (Iter.Nil)))))
      (def (isum it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ h (isum rest)))))
      (def
        (main)
        (isum (take-while (imap (from-list #list(1 2 3 4)) (fn (x) (* x 10))) (fn (y) (< y 35)))))
      (export main)))
  (output (: 60 Int64))
  (live-objects known-leak))

; scan (RUNNING FOLD) behavioral edges (breaker): the just-landed giter.cdz scan (slice 7) emits each
; intermediate accumulator — the seed first, one per element, so n+1 outputs — threading an accumulator AND
; a closure. The landed giter.cdz @tests cover running-sum at two types; these pin the uncovered behavior as
; corpus: the n+1 count, the accumulator threaded LEFT-TO-RIGHT (an order-sensitive folder, which a plain
; sum can't distinguish from a wrong order), a specific intermediate by index, and the empty-input base case
; (emits just the seed — needs an annotated element type, since a bare empty (list) leaves `a` unconstrained).
(case
  "scan emits the seed plus one accumulator per element (n+1 outputs)"
  (doc
    "`scan` is a running fold that emits the seed THEN one accumulator per element, so over a 4-element
           list it yields 5 outputs. `scan [1,2,3,4] 0 (+)` → [0,1,3,6,10], icount 5. Pins the n+1 length —
           the seed is emitted (not skipped) and each element contributes exactly one accumulator.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (scan it acc f)
        (match
          it
          ((Iter.Nil) (Iter.Cons acc (Iter.Nil)))
          ((Iter.Cons h rest) (Iter.Cons acc (scan rest (f acc h) f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (scan (from-list #list(1 2 3 4)) 0 (fn (a x) (+ a x)))))
      (export main)))
  (output (: 5 Int64))
  (live-objects known-leak))

(case
  "scan threads its accumulator left-to-right (an order-sensitive folder)"
  (doc
    "The accumulator is threaded LEFT-TO-RIGHT: with an order-sensitive folder `(a*10 + x)` over
           [1,2,3] the running accumulators are [0,1,12,123], whose LAST is 123. A running SUM cannot
           distinguish left-to-right from right-to-left (addition is commutative); this folder can, so it
           pins the fold direction — a scan that threaded the accumulator the other way would yield a
           different last value.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (scan it acc f)
        (match
          it
          ((Iter.Nil) (Iter.Cons acc (Iter.Nil)))
          ((Iter.Cons h rest) (Iter.Cons acc (scan rest (f acc h) f)))))
      (def
        (last it)
        (match it ((Iter.Nil) -1) ((Iter.Cons h rest) (match rest ((Iter.Nil) h) (_ (last rest))))))
      (def (main) (last (scan (from-list #list(1 2 3)) 0 (fn (a x) (+ (* a 10) x)))))
      (export main)))
  (output (: 123 Int64))
  (live-objects known-leak))

(case
  "scan's kth emitted accumulator is the running fold of the first k elements"
  (doc
    "Indexing INTO the running fold: the accumulator at index 2 of `scan [10,20,30] 0 (+)` is the sum
           of the first TWO elements (10+20 = 30) — index 0 is the seed 0, index 1 is 10, index 2 is 30. Pins
           that each intermediate is the fold of exactly the elements consumed before it, not an off-by-one.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (scan it acc f)
        (match
          it
          ((Iter.Nil) (Iter.Cons acc (Iter.Nil)))
          ((Iter.Cons h rest) (Iter.Cons acc (scan rest (f acc h) f)))))
      (def
        (nth it n)
        (match it ((Iter.Nil) -1) ((Iter.Cons h rest) (if (= n 0) h (nth rest (- n 1))))))
      (def (main) (nth (scan (from-list #list(10 20 30)) 0 (fn (a x) (+ a x))) 2))
      (export main)))
  (output (: 30 Int64))
  (live-objects known-leak))

(case
  "scan over an empty iterator emits just the seed"
  (doc
    "The base case: `scan` over an EMPTY iterator emits only the seed accumulator — one output, length
           1 (the `(Iter.Nil)` arm returns `(Cons acc Nil)`). The empty list is annotated `(List Int64)` so
           the element type `a` is determined (a bare empty `(list)` leaves `a` unconstrained → CDZ0201, the
           general empty-polymorphic-literal ambiguity, not a scan issue). Pins that even with no elements
           the seed is still emitted — scan never yields the empty iterator.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list (: xs (List Int64)))
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (scan it acc f)
        (match
          it
          ((Iter.Nil) (Iter.Cons acc (Iter.Nil)))
          ((Iter.Cons h rest) (Iter.Cons acc (scan rest (f acc h) f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (scan (from-list #list()) 0 (fn (a x) (+ a x)))))
      (export main)))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a recursive-generic transformer threading an IDENTITY closure composes at a single element type"
  (doc
    "A pure IDENTITY closure `(fn (s) s)`, whose RESULT is determined only by its DOMAIN (not fixed by
           the body bottom-up), now composes through a recursive-generic transformer at a single element
           type: `gmap` maps `(fn (s) s)` over `[\"a\",\"b\"]` and `icount` of the result is 2. The
           closure-body result←domain flow — solving the pass-through body UNDER the domain the closure tie
           pins — is what carries the identity result to `gmap`'s `Iter b` result. Pins the identity/
           pass-through closure case (the domain tie alone left the RESULT free; this adds the result flow).")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (gmap (from-list #list("a" "b")) (fn (s) s))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a recursive-generic transformer threading an IDENTITY closure composes at TWO element types"
  (doc
    "The multi-instantiation closure tie: a pure IDENTITY closure `(fn (s) s)` composes at a SINGLE
           element type (above) AND, mixed with another instantiation — Int64 `(fn (x) (+ x 1))` AND String
           `(fn (s) s)` in one program — now RUNS (3 + 2 = 5). Previously DECLINED CDZ0201 (the two coexisting
           monomorphizations of `gmap` did not both bind their result element from their own closure). FIXED
           by tying an unannotated closure's result to its expected DOMAIN in `solved_lambda_arrow_under`
           (seeding the concrete domain into `db.param_types` so an aggregate/pass-through body reads the
           param at its domain type, not the bottom-up `Any`) — the same fix that lets a closure map to an
           AGGREGATE result (tuple/user-sum) across ≥2 distinct domains. Pins that the multi-instantiation
           closure-result tie composes; this was v-iterators' misnamed \"instantiation-pressure ceiling\".")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (+
          (icount (gmap (from-list #list(1 2 3)) (fn (x) (+ x 1))))
          (icount (gmap (from-list #list("a" "b")) (fn (s) s)))))
      (export main)))
  (output (: 5 Int64))
  (live-objects known-leak))

; A TRANSITIVE recursive-generic tie: a `reduce`-shaped WRAPPER (`reduce1`) whose `Cons` arm seeds a
; SECOND recursive-generic helper (`go`) with the HEAD element, used at TWO element types in one program
; (Int64 sum + String concat), DECLINES. Distinct from the CDZ0201 producer/transformer ties above: it is
; an uncoded decline ("this recursive function has a parameter whose type could not be inferred …") — the
; delegated helper `go`'s parameter is not tied across the wrapper's two monomorphizations. A structurally
; identical DIRECT fold with an element-typed accumulator at two types WORKS; the trigger is the wrapper→
; second-helper delegation. NO annotation escape hatch exists: `go`'s param is a GENERIC element, and a
; lowercase `(GIter a)` in an annotation is unbound (CDZ0101, no forall-binder), so `(: it GIter)` fails
; CDZ0203 — the decline message names this honestly (it does NOT tell the author to annotate a generic
; helper). Single element type compiles+runs; only ≥2 instantiations decline. A tracked inference follow-up
; (the recursive-generic monomorphization tie family); pinned as a clean decline so a future fix flips it to
; a run (6 + byte-len("ab")=2 = 8) and NEVER a miscompile meanwhile. (v-iterators' reduce/reduce1 residual.)
(case
  "a reduce-shaped wrapper seeding a second generic helper at two element types monomorphizes via the transitive-binder tie"
  (doc
    "`reduce1` (a `reduce`-shaped wrapper) matches its iterator's `Cons` arm and seeds a SECOND
           recursive-generic helper `go` with the head element as the initial accumulator; used at TWO
           element types (Int64 `(+ x y)` folding [1,2,3], String `String.concat` folding [\"a\",\"b\"])
           in one program. This USED to decline (the delegated `go`'s parameter type was not tied across
           the wrapper's two monomorphizations — a TRANSITIVE recursive-generic tie). The tie now lands:
           `go`'s seeds at reduce1's `(go rest h f)` are `h`/`rest`, the `Cons` HEAD/TAIL BINDERS of
           reduce1's generic iterator param `it` — the transitive-genericity walk now traces a match-BINDER
           seed back to the caller PARAM it destructures and PROJECTS that param's per-call-site concrete
           types (`GIter Int64` + `GIter String`) through the pattern path to the binder's sub-position
           (`h`→`{Int64,String}`), so `go`'s accumulator is detected genuinely generic and monomorphizes per
           call. Result = the Int64 reduce (1+2+3=6) plus the byte length of the String reduce (\"ab\"=2) = 8.")
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def
        (from-list xs)
        (match xs (#list() (GIter.Nil)) (#list(h (.. t)) (GIter.Cons h (from-list t)))))
      (def (go it acc f) (match it ((GIter.Nil) acc) ((GIter.Cons h rest) (go rest (f acc h) f))))
      (def
        (reduce1 it f)
        (match it ((GIter.Nil) (Option.None)) ((GIter.Cons h rest) (Option.Some (go rest h f)))))
      (def
        (main)
        (+
          (match
            (reduce1 (from-list #list(1 2 3)) (fn (x y) (+ x y)))
            ((Option.None) 0)
            ((Option.Some v) v))
          (match
            (reduce1 (from-list #list("a" "b")) (fn (x y) (String.concat x y)))
            ((Option.None) 0)
            ((Option.Some v) (String.byte-len v)))))
      (export main)))
  (output (: 8 Int64))
  (live-objects known-leak))

; SECOND witness of the same transitive-delegate tie — the seed is an ACCUMULATOR, not the head. `reverse`
; is a wrapper `(reverse it) = (rev-onto it (GIter.Nil))` that seeds a SECOND recursive-generic helper
; `rev-onto` with a bare `GIter.Nil` accumulator; at TWO element types it declines (CDZ0201 here — a
; different manifestation than the reduce/go message, same family). This GENERALIZES the finding: the
; trigger is ANY element-typed value threaded through a second recursive-generic delegate whose parameter
; is not tied across the wrapper's two monomorphizations — the seed can be the head element (reduce1/go
; above) OR a bare-Nil accumulator (reverse/rev-onto here). A DIRECT element-typed fold composes at two
; types; only the wrapper→second-helper delegation is unrealized. Single element type compiles+runs (icount
; of the reversed 3-list = 3). Pinned as a clean decline so a future transitive-tie fix flips it to a run
; (3 + 2 = 5) and never a miscompile. (v-iterators' reverse residual, sibling of the reduce case above.)
(case
  "a reverse wrapper seeding a second generic helper with an accumulator at two element types declines pending the transitive tie"
  (doc
    "`reverse` wraps a second recursive-generic helper `rev-onto` seeded with a bare `GIter.Nil`
           accumulator: `(reverse it) = (rev-onto it (GIter.Nil))`. Used at TWO element types (reversing
           an Int64 list [1,2,3] and a String list [\"a\",\"b\"]) in one program, it declines — `rev-onto`'s
           accumulator element is not tied across `reverse`'s two monomorphizations. The ACCUMULATOR-seeded
           sibling of the head-seeded reduce case above: together they show the transitive-delegate tie is
           not head-specific — it is any element-typed value threaded through a second recursive-generic
           helper. A DIRECT fold at two types composes; only this wrapper→second-helper delegation is
           unrealized. Rejects CDZ0201 (the monomorphizer cannot bind the untied element — a CODED
           rejection here, vs the reduce/go sibling's uncoded 'annotate' decline; same family, two
           manifestations). Single element type compiles and runs (icount of the reversed 3-element list =
           3). Intended value when the tie lands: icount of the reversed Int64 list (3) plus the reversed
           String list (2) = 5.")
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def
        (from-list xs)
        (match xs (#list() (GIter.Nil)) (#list(h (.. t)) (GIter.Cons h (from-list t)))))
      (def
        (rev-onto it acc)
        (match it ((GIter.Nil) acc) ((GIter.Cons h rest) (rev-onto rest (GIter.Cons h acc)))))
      (def (reverse it) (rev-onto it (GIter.Nil)))
      (def (icount it) (match it ((GIter.Nil) 0) ((GIter.Cons h rest) (+ 1 (icount rest)))))
      (def
        (main)
        (+
          (icount (reverse (from-list #list(1 2 3))))
          (icount (reverse (from-list #list("a" "b"))))))
      (export main)))
  (error CDZ0201))

; TYPE-VALUED PARAMETERS — the spec's model for a generic definition (`type-system.md §Generics Are
; Type-Valued Parameters`): a generic def takes the TYPE as an ordinary parameter (annotated `(: t Type)`,
; the kind of types), uses it as a type-constructor argument in a later parameter's annotation `(Box t)`,
; and the caller passes the concrete type as a regular argument `(unbox Int64 …)`. `t` resolves by
; ordinary lexical scope (an earlier parameter is visible in a later parameter's annotation); the type
; argument is compile-time-only and consumed by monomorphization (erased before run time), so `unbox` is
; specialized per passed type exactly as an inferred generic is. NOT implicit type variables — the type is
; a first-class value passed explicitly.
(case
  "a generic definition takes the type as a type-valued parameter"
  (doc
    "`unbox` takes `(: t Type)` — a type-valued parameter — and `(: b (Box t))`, then unwraps the
           box. The caller passes the concrete element type as an ordinary argument: `(unbox Int64 (Box.Mk
           40))` and `(unbox String (Box.Mk \"hi\"))`. `unbox` is monomorphized per passed type (the type
           argument is compile-time-only, erased before run time). 40 + byte-len(\"hi\")=2 = 42.")
  (input
    (do
      (type Box (Mk a))
      (def (unbox (: t Type) (: b (Box t))) (match b ((Box.Mk v) v)))
      (def (main) (+ (unbox Int64 (Box.Mk 40)) (String.byte-len (unbox String (Box.Mk "hi")))))
      (export main)))
  (output (: 42 Int64)))

; A RECURSIVE generic definition with a TYPE-VALUED PARAMETER — the type-valued-parameter model over the
; recursive-data idiom. `len` takes `(: t Type)` and `(: l (Lst t))` (a polymorphic linked list applied
; to the type parameter), recurses on the tail passing `t` along. Called with the concrete element type
; as an argument at Int64 and String, `len` is monomorphized per type — and because the type argument is
; compile-time-only, it is ERASED from each specialized function's signature (each `len` takes just the
; list handle, not the type) and from the recursive self-call. This is the recursive analogue of the
; `unbox` type-valued-parameter case.
(case
  "a recursive generic with a type-valued parameter monomorphizes per type, erasing the type argument"
  (doc
    "`len` takes a type-valued `(: t Type)` and `(: l (Lst t))`, recursing on the tail with `(len t
           tl)`. Called `(len Int64 …)` over a two-element `Lst Int64` (length 2) and `(len String …)`
           over a three-element `Lst String` (length 3). `len` is monomorphized into one function per
           element type; the type argument is compile-time-only, erased from the specialized signature and
           the self-call (each `len` takes only the list handle). 2 + 3 = 5.")
  (input
    (do
      (type Lst Nil (Cons a (Lst a)))
      (def
        (len (: t Type) (: l (Lst t)))
        (match l ((Lst.Nil) 0) ((Lst.Cons h tl) (+ 1 (len t tl)))))
      (def
        (main)
        (+
          (len Int64 (Lst.Cons 1 (Lst.Cons 2 Lst.Nil)))
          (len String (Lst.Cons "a" (Lst.Cons "b" (Lst.Cons "c" Lst.Nil))))))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

; AD-HOC POLYMORPHISM via a DICTIONARY RECORD — a record of functions passed as an ordinary argument,
; the body projecting and calling its fields. No trait resolution, no orphan rule, no coherence: it is
; just records + functions + application. A NON-recursive consumer inlines the dict (β-folds away); a
; RECURSIVE consumer is monomorphized per distinct dictionary — each field function INLINED directly (no
; `call_indirect`, no runtime record) and the dictionary argument ERASED from the emitted signature, the
; same "inline a compile-time-known argument, drop the param" rule that erases a type-valued parameter.
(case
  "a recursive consumer of a dictionary record inlines and erases the dictionary"
  (doc
    "`fold-n` takes a dictionary `(Record (: op (-> Int64 Int64)))` and applies its `op` `n` times.
           Called with `(record (op (fn (x) (+ x 10))))`, the dictionary is compile-time-known, so
           `fold-n` is monomorphized with the `op` inlined directly (`(. d op)` folds to `(+ acc 10)` —
           no call_indirect, no runtime record) and the dictionary argument erased. Folding `+10` from 0
           three times = 30.")
  (input
    (do
      (def
        (fold-n (const (: d (Record (: op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
        (if (= n 0) acc (fold-n d (- n 1) (d.op acc))))
      (def (main) (fold-n #record((= op (fn (x) (+ x 10)))) 3 0))
      (export main)))
  (output (: 30 Int64)))

(case
  "a dictionary consumer called at two dictionaries is monomorphized per dictionary"
  (doc
    "The same `fold-n` called with TWO distinct dictionaries — `(+ x 10)` and `(* x 2)` — is
           monomorphized into two functions, each with its own `op` inlined (per-dictionary
           specialization, the ad-hoc-polymorphism analogue of per-type monomorphization). `(+10)` folded
           from 0 thrice = 30; `(*2)` folded from 1 thrice = 8; 30 + 8 = 38.")
  (input
    (do
      (def
        (fold-n (const (: d (Record (: op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
        (if (= n 0) acc (fold-n d (- n 1) (d.op acc))))
      (def
        (main)
        (+
          (fold-n #record((= op (fn (x) (+ x 10)))) 3 0)
          (fold-n #record((= op (fn (x) (* x 2)))) 3 1)))
      (export main)))
  (output (: 38 Int64)))

; A `const` parameter DECLARES its argument must be compile-time-known: the compiler inlines + erases it,
; and REJECTS an argument that depends on runtime data (the author's contract, enforced). Here the dict's
; `op` captures `main`'s runtime parameter `k`, so the dictionary is NOT compile-time-known — a coded
; CDZ0201 rejection, not a silent runtime fallback.
(case
  "a const parameter rejects an argument that depends on runtime data"
  (doc
    "`fold-n`'s dictionary parameter is `const`, so its argument must be compile-time-known. `main`
           passes `(record (op (fn (x) (+ x k))))` whose `op` captures `main`'s RUNTIME parameter `k` —
           the dictionary is not a compile-time value, violating the `const` contract. The compiler
           rejects it (CDZ0201, 'must be compile-time-known'), rather than silently passing it at runtime.
           The rejection is the program's outcome; there is no value.")
  (input
    (do
      (def
        (fold-n (const (: d (Record (: op (-> Int64 Int64))))) (: n Int64) (: acc Int64))
        (if (= n 0) acc (fold-n d (- n 1) (d.op acc))))
      (def (main (: k Int64)) (fold-n #record((= op (fn (x) (+ x k)))) 3 0))
      (export main)))
  (error CDZ0201 (message "const") (message "compile-time-known") (message "runtime data")))

(case
  "a const collection recursively folded UNROLLS to its result at compile time"
  (doc
    "A `const` COLLECTION parameter (here a `(List Int64)`) consumed by a SELF-RECURSIVE fold FULLY
           UNROLLS at compile time (P2 recursive const-fold): `s [1,2,3] 0` = 6. Each recursion `(s t …)`
           passes a SHORTER derived list `t`, so the fold is BOUNDED — the const scrutinee selects the
           `(list h .. t)` step arm until the `(list)` base arm yields the accumulator. The lowerer does one
           β-level for a recursive call INTO a `const`-param callee whose args are all compile-time
           constants, then folds the reduced body; the residual self-call re-enters with its now-const-folded
           arguments → the next level → the base. The `const` param is the const-DEMAND signal, so ordinary
           recursive-generic / collection-building recursions (no `const` param) are unaffected — they still
           emit a runtime call. The whole descent runs under the reduction node-budget, so a NON-shrinking
           const recursion exhausts it and declines (never hangs). The RUNTIME-list version (drop `const`)
           still compiles to a proper `loop` (the case below).")
  (input
    (do
      (def
        (s (const (: xs (List Int64))) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (s t (+ acc h)))))
      (def (main) (s #list(1 2 3) 0))
      (export main)))
  (output (: 6 Int64)))

(case
  "a const list whose elements are BYTES LITERALS recursively folds — a BytesOf is a const value"
  (doc
    "The recursive const-fold unroll (case above) accepts a `const (List …)` whose elements are
           compile-time constants. A `b\"…\"` bytes LITERAL lowers to a `Core::BytesOf` (of constant byte
           elements), which IS a constant value — like `ConstBytes`. So a const list of bytes literals folds
           the same as a const list of ints: `cat [b\"ab\", b\"cd\"] b\"\"` concatenates to a 4-byte constant.
           Pins that `is_const_value` recognizes `BytesOf` (of consts): without it, a const list carrying any
           bytes literal was wrongly deemed non-const, so the unroll declined — the exact gap that blocked a
           recursive fold over a REFLECTED module's forms (`Ast.module`) or a QUOTE containing a `b\"…\"`
           (e.g. a userspace contract-id transform building tagged bytes with `Bytes.concat(b\"\\x01\", …)`).")
  (input
    (do
      (def
        (cat (const (: xs (List Bytes))) (: acc Bytes))
        (match xs (#list() acc) (#list(h (.. t)) (cat t (Bytes.concat acc h)))))
      (def (main) (Bytes.len (cat #list(b"ab" b"cd") b"")))
      (export main)))
  (output (: 4 Int64)))

(case
  "a const list recursively FILTERED (conditional include) folds to a constant"
  (doc
    "A recursive const-fold whose step CONDITIONALLY includes an element in the built list — a FILTER
           `let tail = keep t in (if (p h) (List.prepend tail h) tail)` — folds to a constant, like the
           UNCONDITIONAL rebuild cases above. The recursed `tail` is bound in a `let` and read from BOTH arms
           of the `if` (the then-arm prepends onto it, the else-arm returns it whole), so ordinary `let`-
           lowering keeps it as a multi-use `Core::Let` slot — a residual the fold's constant-value check
           rejects, declining a program that genuinely folds. Under the const-fold unroll a `let` binding
           whose init folds to a CONSTANT is inlined at every use (a constant is free to duplicate, no
           recompute / no effect), so the whole filter collapses: `keep-pos [1 -2 3]` = `[1 3]`, length 2. The
           `const` parameter forces compile time — a non-folding filter would REJECT, not run — so a passing
           output witnesses the fold. This is the FILTER piece the general self-reflected contract-id transform
           needs (`collect-types` filters `Ast.module`'s forms to the `type` declarations before hashing).")
  (input
    (do
      (def
        (keep-pos (const (: xs (List Int64))))
        (match
          xs
          (#list() (: #list() (List Int64)))
          (#list(h (.. t)) (let ((tail (keep-pos t))) (if (> h 0) (List.prepend tail h) tail)))))
      (def (main) (List.len (keep-pos #list(1 -2 3))))
      (export main)))
  (output (: 2 Int64)))

(case
  "a NESTED const recursion — a recursive helper called per element inside a recursive build — folds"
  (doc
    "A recursive build over a `const (List …)` that calls ANOTHER recursive `const`-param helper PER
           ELEMENT — the nested-recursion composition. `build` recurses down the list and, for each head, calls
           the recursive `dec` (which counts a `const` scalar down to a base case); the results assemble into a
           constant list. This folds because the const-fold unroll accepts a `const` parameter of ANY shape a
           total recursion shrinks — a `(List …)` OR a bare-name type (here `dec`'s `const (: n Int64)` scalar)
           — not only a `(List …)`. Before that, `dec`'s per-element call declined (`(dec h)`'s arg reaches a
           const scalar param but `dec` is recursive, so it took the recursive-decline path → CDZ0201), which
           blocked the general self-reflected transform's `rebuild`-of-`unwrap` (a recursive comment-peeler
           called per form). `build [3 2 1]` = `[0 0 0]` (each `dec h` = 0), length 3. The `const` parameters
           force compile time, so a passing output witnesses the whole nested fold reaching a constant.")
  (input
    (do
      (def (dec (const (: n Int64))) (if (= n 0) 0 (dec (- n 1))))
      (def
        (build (const (: xs (List Int64))))
        (match
          xs
          (#list() (: #list() (List Int64)))
          (#list(h (.. t)) (List.prepend (build t) (dec h)))))
      (def (main) (List.len (build #list(3 2 1))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a NON-TAIL recursive helper called per element inside a recursive build folds (general const-eval)"
  (doc
    "The NESTED case above uses a TAIL-recursive helper (`dec`); this uses a NON-TAIL one — `tri`'s
           self-call sits inside `(+ n (tri (- n 1)))`, accumulating a sum. The general const-EVALUATOR
           (DESIGN-general-const-eval.md) interprets a total function applied to compile-time-constant
           arguments to a constant VALUE, so it composes natively: `rb` evaluates `tri h` to a constant for
           each element and assembles the constant list — where the unroll-and-refold could not (a non-tail
           recursion const-folded inside another recursion's fold left a residual it could not collapse). The
           evaluator is bounded by a step budget (a non-terminating fold declines, never hangs) and fires only
           under the same `const`-parameter demand as the unroll, so nothing outside a genuine const fold is
           affected. `rb [1 2 3]` = `[tri 1, tri 2, tri 3]` = `[1 3 6]`, length 3. The `const` parameters force
           compile time, so a passing output witnesses the whole non-tail nested fold reaching a constant.")
  (input
    (do
      (def (tri (const (: n Int64))) (if (= n 0) 0 (+ n (tri (- n 1)))))
      (def
        (rb (const (: xs (List Int64))))
        (match
          xs
          (#list() (: #list() (List Int64)))
          (#list(h (.. t)) (List.prepend (rb t) (tri h)))))
      (def (main) (List.len (rb #list(1 2 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "the runtime-list version of a tail fold compiles and folds correctly"
  (doc
    "The correct alternative to the const-collection reject above: the SAME tail fold over a RUNTIME
           `(List Int64)` parameter (no `const`) compiles to a proper `loop` whose `br_if` exit is the real
           length/nil test, and runs — `s [1,2,3] 0` = 6. Pins that dropping `const` (so the list is an
           ordinary runtime value the loop iterates) is the working form, and that the reject above is
           specific to the const-erasure × tail-loop composition, not to tail-folding a list.")
  (input
    (do
      (def
        (s (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (s t (+ acc h)))))
      (def (main) (s #list(1 2 3) 0))
      (export main)))
  (output (: 6 Int64))
  ; RECLAIMS to 0 (was interim known-leak #6022/#6049): INC1 pt3's self-loop-tail shell reclaim frees the
  ; runtime-list spine per iteration — the (List Int64) sibling of the user-Cons `len` (09) + tree `inorder`
  ; (05) self-tail-loop witnesses. Confirmed 0 by v-runtime faithful census. A regression guard: live-objects
  ; > 0 here is a pt3 reclaim regression on the runtime-list tail fold.
  (live-objects 0))

; The tail fold above folds a built-in list with a fixed `+`. The general HIGHER-ORDER left fold
; (`foldl f acc xs`) takes a COMBINING FUNCTION `f` as a parameter — the single most common higher-order
; list function, and how a compiler folds over AST children / a symbol list / a constraint set. These pin
; `foldl` over a built-in `(List Int64)` (the sum-list HOF folds elsewhere use a `(type L …)` cons type;
; this is the runtime heap vector matched by `(list h .. t)`). Both annotation forms that anchor inference
; are pinned: annotating the CLOSURE's params, and annotating the HOF's `f` parameter with its arrow type.
; (The fully-INFERRED spelling — both unannotated — does not yet infer the closure's param types through
; the recursion; that inference gap is the generics workstream's, tracked in the port's repros.)
(case
  "a higher-order left fold over a built-in list with an annotated callback"
  (doc
    "`foldl` over a built-in `(List Int64)` taking a combining function `f`: `(fold-list f (f h acc)
           t)` threads the accumulator, matching the list by `(list h .. t)`. The closure `(fn ((: x Int64)
           (: a Int64)) (+ a x))` — its params ANNOTATED — sums `[5,7,30]` from acc 0 → 42. Pins the
           higher-order left fold over a runtime heap list (the fold-over-AST-children idiom), with the
           closure's own annotations anchoring inference.")
  (input
    (do
      (def
        (fold-list f (: acc Int64) (: xs (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))
      (def (main) (fold-list (fn ((: x Int64) (: a Int64)) (+ a x)) 0 #list(5 7 30)))
      (export main)))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a higher-order left fold over a built-in list with an annotated fn parameter"
  (doc
    "The other anchor: the closure is UNannotated `(fn (x a) (+ a x))`, but the HOF's `f` parameter
           carries its arrow type `(: f (-> Int64 (-> Int64 Int64)))`, which flows into the closure's params.
           Same fold, same result 42. Pins that annotating the fold's function parameter (rather than the
           closure) is an equivalent working spelling — the form a compiler pass writes when the fold is a
           named reusable combinator.")
  (input
    (do
      (def
        (fold-list (: f (-> Int64 (-> Int64 Int64))) (: acc Int64) (: xs (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))
      (def (main) (fold-list (fn (x a) (+ a x)) 0 #list(5 7 30)))
      (export main)))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a higher-order left fold applies its combiner, not a fixed operator"
  (doc
    "The discriminator that the fold genuinely APPLIES `f` (not a hardcoded `+`): the same `fold-list`
           with a MAX combiner `(fn ((: x Int64) (: a Int64)) (if (> x a) x a))` over `[5,30,7]` from acc 0
           yields 30 (the maximum), not 42 (their sum). Pins that the combining function is the parameter
           driving the fold — a fold that ignored `f` and summed would give 42.")
  (input
    (do
      (def
        (fold-list f (: acc Int64) (: xs (List Int64)))
        (match xs (#list() acc) (#list(h (.. t)) (fold-list f (f h acc) t))))
      (def (main) (fold-list (fn ((: x Int64) (: a Int64)) (if (> x a) x a)) 0 #list(5 30 7)))
      (export main)))
  (output (: 30 Int64))
  (live-objects known-leak))

; INLINE POLICY — the `@inline-never` / `@inline-always` ANNOTATIONS (`DESIGN-…-monomorphization`
; Addendum 4). `@name form` is the general-purpose annotation sigil (canonical `(@ name form)`); these are
; the two names the compiler consumes today. The compiler lowers by β-reduction, so the DEFAULT is
; always-inline; `@inline-never` forces a def to be emitted as ONE real function and CALLED (never
; inlined), controlling code size. It COMPOSES with `const`/generics — "avoid the inline but still get
; polymorphism": an `@inline-never` def with a `const` dictionary param still inlines the dict into a
; per-instantiation specialized copy (direct op, no runtime dispatch) and emits that copy once.
; `@inline-always` is the (currently inert) opposite; on a recursive def it is a contradiction (recursion
; can't inline) → rejected.
(case
  "an inline-never definition is emitted once and called"
  (doc
    "`big` is annotated `@inline-never`, so instead of β-reducing at each call site it is emitted as
           one real function and called. Observable via the VALUE (the emission strategy does not change
           semantics): `big(x) = x*7 + x*11 + x*13`, `big(2) + big(3)` = 62 + 93 = 155. The point is that
           `big`'s body is emitted ONCE (one function, two calls) rather than duplicated per call site.")
  (input
    (do
      (@ inline-never (def (big (: x Int64)) (+ (* x 7) (+ (* x 11) (* x 13)))))
      (def (main) (+ (big 2) (big 3)))
      (export main)))
  (output (: 155 Int64)))

(case
  "an inline-never definition with a const dictionary still monomorphizes the dictionary"
  (doc
    "`@inline-never` COMPOSES with a `const` dictionary parameter (`avoid the inline but keep
           polymorphism`): `apply2` is emitted ONCE per distinct dictionary with the dictionary's `op`
           INLINED (no runtime record, no indirect dispatch) — the dictionary is compile-time-erased — and
           that specialized function is CALLED at each use rather than the whole body being inlined.
           `apply2` applies `d.op` twice; with `op = (+ n 10)`: `5 → 25`, `100 → 120`; 25 + 120 = 145.")
  (input
    (do
      (@
        inline-never
        (def (apply2 (const (: d (Record (: op (-> Int64 Int64))))) (: x Int64)) (d.op (d.op x))))
      (def
        (main)
        (+
          (apply2 #record((= op (fn (n) (+ n 10)))) 5)
          (apply2 #record((= op (fn (n) (+ n 10)))) 100)))
      (export main)))
  (output (: 145 Int64)))

(case
  "inline-always on a recursive definition is rejected"
  (doc
    "`@inline-always` asks the compiler to always fold a def at its call sites, but a RECURSIVE def
           cannot inline (it would inline without end; it is always emitted as one function). The
           annotation is therefore a contradiction and is rejected (CDZ0201). The rejection is the program's
           outcome.")
  (input
    (do
      (@ inline-always (def (loop-n (: n Int64)) (if (= n 0) 0 (loop-n (- n 1)))))
      (def (main) (loop-n 5))
      (export main)))
  (error CDZ0201 (message "recursive") (message "inline-always")))

; The `@` sigil is GENERAL (`@name form` — "future annotations `@deprecated`, `@test` layer in with no new
; lexer/parser/resolver rules"). An annotation name the compiler does NOT model is TRANSPARENT: the strip
; pass unwraps `(@ <name> (def …))` to the def just as it does a known one, recording nothing — so the def
; takes effect, the unmodeled name is simply ignored. (Previously an unmodeled `(@ …)` node survived to
; resolve, where the head `@` is no declaration → the wrapped def was DROPPED with a misleading "unbound
; name `@`" plus a phantom unbound-name for the def.) A future annotation gains meaning by joining the
; compiler's known set; until then it is an inert marker that never breaks the def it annotates.
(case
  "an unrecognized annotation leaves its wrapped definition in effect"
  (doc
    "`(@ deprecated (def (f) 5))` — the `@name` annotation sigil with a name OTHER than the modeled
           inline policies / test marker. The def must still register and `main` → 5: an unmodeled
           annotation is transparent (unwrapped to the def, its name ignored), not a rejection. This holds
           for ANY unknown name (`@deprecated`, `@lint`, …) and for a def USED by another def; the modeled
           `@inline-never`/`@inline-always` retain their emission policy. A generation that unwraps an
           unknown annotation runs `f` (→ 5) rather than dropping it with 'unbound name @'.")
  (input (do (@ deprecated (def (f) 5)) (def (main) (f)) (export main)))
  (output (: 5 Int64)))

; A MALFORMED top-level `(@ …)` — one that wraps NO well-formed definition — is the counterpart reject: the
; strip pass unwraps every def-wrapping annotation IN PLACE (even an unknown name, above), so any SURVIVING
; top-level `(@ …)` wrapped no def to unwrap. Left alone the head `@` resolved as the misleading "unbound name
; `@`"; instead each is CDZ0201 naming the `(@ <name> (def …))` shape. Migrated from rcdzc
; a_malformed_top_level_annotation_names_the_annotation_shape_not_an_unbound_at (the five malformed shapes).
(case
  "a name-only top-level annotation wraps no definition and is rejected"
  (input (do (@ test) (def (main) 0) (export main)))
  (error CDZ0201 (message "annotation wraps no definition") (message "`(@ <name> (def …))`")))

(case
  "an empty top-level annotation wraps no definition and is rejected"
  (input (do (@) (def (main) 0) (export main)))
  (error CDZ0201 (message "annotation wraps no definition") (message "`(@ <name> (def …))`")))

(case
  "a top-level annotation over a non-form target wraps no definition and is rejected"
  (input (do (@ test 5) (def (main) 0) (export main)))
  (error CDZ0201 (message "annotation wraps no definition") (message "`(@ <name> (def …))`")))

(case
  "a top-level annotation over a non-def list target wraps no definition and is rejected"
  (input (do (@ test (foo 1)) (def (main) 0) (export main)))
  (error CDZ0201 (message "annotation wraps no definition") (message "`(@ <name> (def …))`")))

(case
  "a top-level annotation over a malformed inner def wraps no definition and is rejected"
  (input (do (@ test (def)) (def (main) 0) (export main)))
  (error CDZ0201 (message "annotation wraps no definition") (message "`(@ <name> (def …))`")))

; The `@tag(<string>)` annotation takes EXACTLY ONE STRING argument — a non-string (number / bare name),
; zero args, or two args is CDZ0201 naming the contract; a valid `@tag("string")` is accepted silently and
; transparently unwraps to its def. Migrated from rcdzc a_malformed_tag_annotation_is_rejected_not_silently_dropped
; (the four malformed shapes + the valid control; the no-double dedup on a @tag over a NON-def is the
;  `(count 1) (no-other-errors)` case below).
(case
  "a @tag annotation over a non-string number is rejected"
  (input (do (@ (tag 5) (def (c) 3)) (export c)))
  (error CDZ0201 (message "`@tag` annotation takes exactly one STRING")))

(case
  "a @tag annotation over a non-string bare name is rejected"
  (input (do (@ (tag foo) (def (c) 3)) (export c)))
  (error CDZ0201 (message "`@tag` annotation takes exactly one STRING")))

(case
  "a @tag annotation with zero arguments is rejected"
  (input (do (@ (tag) (def (c) 3)) (export c)))
  (error CDZ0201 (message "`@tag` annotation takes exactly one STRING")))

(case
  "a @tag annotation with two arguments is rejected"
  (input (do (@ (tag "a" "b") (def (c) 3)) (export c)))
  (error CDZ0201 (message "`@tag` annotation takes exactly one STRING")))

(case
  "a valid @tag string annotation is accepted and unwraps to its def"
  (input (do (@ (tag "slow") (def (c) 3)) (export c)))
  (call c)
  (output (: 3 Int64)))

; A malformed @tag wrapping a NON-def — `(@ (tag 5) 5)` — is ONE mistake: "annotation wraps no definition".
; The `@tag` contract applies only to a def, and `strip_annotations` records the malformed-tag AFTER the
; def check, so a @tag on a non-def must NOT ALSO record the "takes exactly one STRING" malformed-tag fault
; (no double diagnostic). `(count 1)` pins exactly ONE CDZ0201 (the wraps-no-definition one), so a regressed
; second CDZ0201 would fail. (migrated from rcdzc a_malformed_tag_annotation_on_a_non_def_is_not_a_double_diagnostic.)
(case
  "a malformed @tag wrapping a non-def is one wraps-no-definition error, not a double"
  (input (do (@ (tag 5) 5) (def (main) 0) (export main)))
  (error CDZ0201 (count 1) (message "annotation wraps no definition"))
  (no-other-errors))

; A NESTED `(@ …)` in EXPRESSION position (inside a `do`-block / an `(: … T)` annotation), NOT wrapping a
; top-level def, is a MISPLACED annotation: `strip_annotations` unwraps only def-wrapping annotations, so a
; nested survivor is CDZ0201 "annotation cannot appear here" — general to all annotation names (@param, @tag).
; Migrated from rcdzc a_nested_annotation_reports_no_unbound_name_cascade (reject + message + the NO-cascade
; invariant): the misplacement must NOT also spuriously report `unbound name @` or the annotation's internal
; tokens (`param`/`widget`/`slider`, `tag`/`foo`) as unbound-name errors. `(no-other-errors)` pins that no
; OTHER coded error (the CDZ0101 unbound-name cascade) accompanies the single CDZ0201.
(case
  "a nested @param annotation in expression position cannot appear here"
  (input (do (def (main) (do (: (@ (param (: widget slider)) width) Int64) 1)) (export main)))
  (error CDZ0201 (message "annotation cannot appear here"))
  (no-other-errors))

(case
  "a nested @tag annotation in expression position cannot appear here"
  (input (do (def (main) (do (: (@ (tag x) foo) Int64) 1)) (export main)))
  (error CDZ0201 (message "annotation cannot appear here"))
  (no-other-errors))

; COST HEURISTIC (Addendum 4). The UNANNOTATED default is always-inline, but a LARGE, MULTIPLY-CALLED def
; whose call has a runtime-dependent argument is emitted ONCE and called instead of duplicated at each site.
; This is an EMISSION-STRATEGY choice — it does NOT change semantics — so it is observable only via the
; VALUE being unchanged: `big(x) = x*7 + x*11 + x*13 + x*17` = 48x. `main a b = big(a) + big(b)`; the export
; is called with runtime args by the harness. The heuristic emits `big` once and calls it twice; the
; `@inline-never` case above forces the same emission, and both agree on the value. `big(2)+big(3)` = 96 +
; 144 = 240. (The floor is deliberately conservative; small helpers stay inlined.)
(case
  "a large multiply-called definition is emitted once by the cost heuristic"
  (doc
    "A def large enough (past the inline-cost floor) and called at multiple sites with a runtime
           argument is emitted as ONE function and called, not inlined per site — the cost heuristic's
           duplication win. Semantics are unchanged (emission strategy only): with runtime args a=2, b=3,
           `big(x)=48x`, so `big(2)+big(3)` = 96 + 144 = 240.")
  (input
    (do
      (def (big (: x Int64)) (+ (* x 7) (+ (* x 11) (+ (* x 13) (* x 17)))))
      (def (main (: a Int64) (: b Int64)) (+ (big a) (big b)))
      (export main)))
  (call main (: 2 Int64) (: 3 Int64))
  (output (: 240 Int64)))

; --- A closure applied through a variant whose closure type is NEVER BUILT still gets its call type ------
; A runtime closure application lowers to a `call_indirect` through the funcref table, which needs a
; TYPE-SECTION functype of the closure's `(env, args…) -> result` shape. The functypes come from the lifted
; lambda bodies + defined functions — so an applied closure whose LIFTED BODY is never built (no
; `Core::Closure` of that type is ever constructed) had NO functype to reference and DECLINED "a runtime
; closure application has no matching function type". This happens when one sum boxes TWO distinctly-typed
; closures (a `Unary (Int64->Int64)` and a `Binary (Int64->Int64->Int64)`), a match applies BOTH arms'
; closures, but only ONE variant is constructed: the other arm's `call_indirect` is statically emitted yet
; its closure type is dynamically dead. The fix registers an extra functype for each reachable
; closure-application shape no lifted lambda supplies. (The two arms differ in ARITY here, so the shapes are
; genuinely distinct; this is the minimized form of iterator `scan` + `flat-map` coexisting.)
(case
  "a closure applied through an unbuilt sibling variant compiles and runs"
  (doc
    "`T` boxes a `Unary (Int64->Int64)` and a `Binary (Int64->Int64->Int64)`; `apply-it` matches both
           and applies each arm's closure (`(f x)` / `(g x y)`). `main` builds only the `Unary` variant, so
           the `Binary` arm's `(g x y)` is statically emitted but its `(env,i64,i64)->i64` closure type is
           never constructed — no lifted lambda has that shape. It used to DECLINE 'a runtime closure
           application has no matching function type'; the fix registers the missing call functype so the
           `call_indirect` resolves. `main` runs the `Unary` arm: `(fn (n) (* n 2))` at 5 → 10.")
  (input
    (do
      (type T (Unary (-> Int64 Int64)) (Binary (-> Int64 (-> Int64 Int64))))
      (def
        (apply-it (: t T) (: x Int64) (: y Int64))
        (match t ((Unary f) (f x)) ((Binary g) (g x y))))
      (def (main) (apply-it (T.Unary (fn ((: n Int64)) (* n 2))) 5 9))
      (export main)))
  (output (: 10 Int64)))

(case
  "the built variant is the multi-arg one, the unbuilt arm still gets its call type"
  (doc
    "The symmetric direction: `main` builds the `Binary` variant, so the `Unary` arm's `(f x)` —
           an `(env,i64)->i64` closure type — is the statically-emitted-but-never-built application whose
           functype must be registered. Runs the `Binary` arm: `(fn (a b) (+ a b))` at 5,9 → 14. Confirms
           the fix covers a missing call functype regardless of which sibling variant is constructed.")
  (input
    (do
      (type T (Unary (-> Int64 Int64)) (Binary (-> Int64 (-> Int64 Int64))))
      (def
        (apply-it (: t T) (: x Int64) (: y Int64))
        (match t ((Unary f) (f x)) ((Binary g) (g x y))))
      (def (main) (apply-it (T.Binary (fn ((: a Int64) (: b Int64)) (+ a b))) 5 9))
      (export main)))
  (output (: 14 Int64)))

(case
  "a boxed nested-unary curried closure applies without trapping"
  (doc
    "A closure of type (-> Int64 (-> Int64 Int64)) written nested-unary (fn (n) (fn (m) (+ n m))),
           boxed in a sum, extracted and applied curried ((f x) y). The two arrows are distinct lifted
           lambdas; spine-flattening the call into one 2-arg call_indirect emits a functype no lifted body
           implements → 'indirect call type mismatch' at runtime. Must return 7 (add 3 4).")
  (input
    (do
      (type Box (C (-> Int64 (-> Int64 Int64))))
      (def (run (: b Box) (: x Int64) (: y Int64)) (match b ((Box.C f) ((f x) y))))
      (def (add (: n Int64)) (fn (m) (+ n m)))
      (def (main) (run (Box.C add) 3 4))
      (export main)))
  (output (: 7 Int64)))

(case
  "a boxed Unit-parameter closure (a lazy thunk) is forced and runs"
  (doc
    "The canonical lazy THUNK `Thunk = Susp(Unit -> Int64)`: a closure with a UNIT parameter boxed in
           a sum, extracted by a match, and FORCED `(f unit)`. valtype_of(Unit) = None, so the boxed-closure
           lift elides the Unit param from the closure functype (in lockstep across the lift guard, slot
           assignment, the Core::Param read of a Unit binder, and the call-sig collection). Unlike a
           Unit-RESULT face the forced call is NOT dead — its result is observed — so a real call_indirect
           runs and returns 42.")
  (input
    (do
      (type Thunk (Susp (-> Unit Int64)))
      (def (force (: t Thunk)) (match t ((Thunk.Susp f) (f unit))))
      (def (mk) (Thunk.Susp (fn ((: u Unit)) 42)))
      (def (main) (force (mk)))
      (export main)))
  (output (: 42 Int64)))

; --- Curried-closure representation unification: the persistence and depth faces -------------------
; ed3b7503e flattened a nested-unary curried closure to the same one-lift machine rep the multi-param
; sugar gets (the two spellings of one arrow type now share a representation; the boxed-apply trap is
; pinned above). These pin what the unification must PRESERVE, promoted from passing breaker probes:
; capture persistence across REPEATED applications, depth-3 spines, HOF transport of a partial
; application, and heap-valued captures.
(case
  "a partial application persists its capture across two applications"
  (doc
    "`h = (add 10)` where `add n = (fn (m) (+ n m))` — the def-returning-lambda spelling — is
           applied TWICE: `(h 1) + (h 2)` = 11 + 12 = 23. The captured n = 10 must survive the first
           application untouched (an env consumed or mutated by the first call skews the second). The
           repeated-application companion of the single-apply partial-application case above, over the
           nested-unary spelling the flattening unifies.")
  (input
    (do
      (def (add (: n Int64)) (fn (m) (+ n m)))
      (def (main (: d Int64)) (let ((h (add 10))) (+ (h 1) (h 2))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 23 Int64)))

(case
  "a three-level curried spine applies through both intermediate closures"
  (doc
    "`add3 n = (fn (m) (fn (k) (+ (+ n m) k)))` applied `(((add3 1) 2) 3)` = 6 — TWO
           intermediate closure values, each capturing the previous level's environment. The depth-3
           face of the spine flattening: a caller that flattens `((f x) y)` to one two-arg call must
           either flatten the 3-spine to one three-arg call or nest correctly — mixing the two (a
           flattened outer over a chained inner) reintroduces the arity-mismatch trap the fix closed.")
  (input
    (do
      (def (add3 (: n Int64)) (fn (m) (fn (k) (+ (+ n m) k))))
      (def (main (: d Int64)) (((add3 1) 2) 3))
      (export main)))
  (call main (: 0 Int64))
  (output (: 6 Int64)))

(case
  "a def-returning-lambda partial application passes through a HOF parameter"
  (doc
    "`(apply-to (add 10) 5)` — the partial application (nested-unary spelling) crosses a
           function-typed PARAMETER boundary `(-> Int64 Int64)` and is applied indirectly inside the
           callee → 15. Pins the unified representation surviving the calling-convention seam (the
           HOF's indirect call must agree with the lifted signature — the exact mismatch class the
           boxed-apply trap exposed, here through a param instead of a sum).")
  (input
    (do
      (def (add (: n Int64)) (fn (m) (+ n m)))
      (def (apply-to (: f (-> Int64 Int64)) (: x Int64)) (f x))
      (def (main (: d Int64)) (apply-to (add 10) 5))
      (export main)))
  (call main (: 0 Int64))
  (output (: 15 Int64)))

(case
  "a heap capture persists across two applications of a curried closure"
  (doc
    "`cat s = (fn (t) (String.byte-len (String.concat s t)))`; `h = (cat \"ab\")` applied twice:
           `(h \"c\")` = 3, `(h \"de\")` = 4 → 7. The captured HEAP string must survive the first
           application's consuming concat (the env's `s` is concat's operand each call — a capture
           dropped or consumed by call one corrupts call two). The heap-env companion of the scalar
           persistence case.")
  (input
    (do
      (def (cat (: s String)) (fn (t) (String.byte-len (String.concat s t))))
      (def (main (: d Int64)) (let ((h (cat "ab"))) (+ (h "c") (h "de"))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

; The curried case above captures a SCALAR-derived heap string built at the closure's construction. Two
; sharper Perceus lifetime faces (breaker, both backends): (1) a FACTORY that RETURNS a closure over its
; own list PARAMETER — the captured list must outlive the factory's frame, then be read on each of two
; applications (a drop at factory-return, or a consume on the first application, corrupts the second);
; (2) a captured list that is ALSO structurally consumed by a rotate loop (`List.concat t (list h)`)
; between the closure's construction and its application — the shared source must survive the rebuild
; intact, so the closure still sees the original bytes.
(case
  "a factory-returned closure over its list parameter keeps the capture live across two applications"
  (doc
    "`mkf xs = (fn (k) (+ k (sum xs)))` RETURNS a closure capturing its list parameter `xs`; the
           factory frame is gone by the time the closure runs. Applied twice — `(+ (f 1) (f 1))` with
           `xs = [n, n]` — each application re-reads the captured list (sum = 2n), so run(4) = (1+8) +
           (1+8) = 18. Pins that a capture escaping its constructing frame stays live and is not consumed
           by the first application (the Perceus refcount must keep the list alive for the closure's whole
           lifetime, across every call).")
  (input
    (do
      (def (sum (: xs (List Int64))) (match xs (#list() 0) (#list(h (.. t)) (+ h (sum t)))))
      (def (mkf (: xs (List Int64))) (fn ((: k Int64)) (+ k (sum xs))))
      (def (main (: n Int64)) (let ((f (mkf #list(n n)))) (+ (f 1) (f 1))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 18 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "a captured list survives a structural rebuild of the same source between capture and application"
  (doc
    "`f = (fn (k) (+ k (isum xs)))` captures `xs = [1,2,3]`; BEFORE `f` runs, `xs` is also fed to a
           `rebuild` rotate loop (`List.concat t (list h)`, n times) producing `rot`. The capture and the
           rebuild share the original `xs`, so the rebuild must not consume or mutate what the closure
           holds. `(+ (* 100 (f 0)) (+ (* 10 (isum rot)) (isum xs)))` at n=2 = 100·6 + 10·6 + 6 = 666 —
           the closure still sums the original 1+2+3, `rot` sums the rotated (same 6), and the third read
           of the still-live `xs` also sums 6. Pins that a shared heap source drives an escaping capture
           AND a structural transform without either corrupting the other.")
  (input
    (do
      (def (isum (: xs (List Int64))) (match xs (#list() 0) (#list(h (.. t)) (+ h (isum t)))))
      (def
        (rebuild (: xs (List Int64)) (: n Int64))
        (if
          (< n 1)
          xs
          (rebuild (match xs (#list(h (.. t)) (List.concat t #list(h))) (_ xs)) (- n 1))))
      (def
        (main (: n Int64))
        (let
          ((xs #list(1 2 3)))
          (let
            ((f (fn ((: k Int64)) (+ k (isum xs)))))
            (let ((rot (rebuild xs n))) (+ (* 100 (f 0)) (+ (* 10 (isum rot)) (isum xs)))))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 666 Int64))
  (call main (: 0 Int64))
  (output (: 666 Int64))
  (live-objects known-leak))

; --- The recursive-generic element tie: value-flow and composition faces ----------------------------
; 7793d4841 (Part C) ties a recursive-generic producer's result element to its argument's (the
; from-list/icount pin counts at two types). These pin the tie carrying VALUES, promoted from
; passing breaker probes: the produced element must be USABLE at each concrete type, survive
; producer self-composition, and tie a sum payload — each a face where a loose var would either
; reject at the second type or mistype the element.
(case
  "a recursive-generic producer's element is usable at each instantiation"
  (doc
    "`wrap` recursively rebuilds its list; ONE program instantiates it at Int64 and String and
           READS an element from each: `(List.at (wrap (list 7 8)) 0)` → 7 and `(String.byte-len
           (List.at (wrap (list \"ab\")) 0))` → 2 → 9. Beyond the landed counting pin: the element
           value flows at its concrete type through the tied result var (a severed var would type the
           element Any and reject the byte-len — or worse, mistype it).")
  (input
    (do
      (def
        (wrap xs)
        (match xs (#list() #list()) (#list(h (.. t)) (List.concat (List.push #list() h) (wrap t)))))
      (def
        (main (: d Int64))
        (+
          (Option.expect (List.at (wrap #list(7 8)) 0) "i")
          (String.byte-len (Option.expect (List.at (wrap #list("ab")) 0) "s"))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

(case
  "the element tie survives producer self-composition"
  (doc
    "`(wrap (wrap xs))` — the producer feeds ITSELF, so the inner instantiation's result
           element must tie through to the outer's argument, at two types in one program: 7 + 3 = 10.
           The composition face (an inner tie that grounds too early mono-locks the outer; one that
           stays loose severs at the seam — the exact failure that kept the iterator library
           mono-Int64).")
  (input
    (do
      (def
        (wrap xs)
        (match xs (#list() #list()) (#list(h (.. t)) (List.concat (List.push #list() h) (wrap t)))))
      (def
        (main (: d Int64))
        (+
          (Option.expect (List.at (wrap (wrap #list(7))) 0) "i")
          (String.byte-len (Option.expect (List.at (wrap (wrap #list("abc"))) 0) "s"))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

(case
  "an Option-producing recursive generic ties its payload to the list element"
  (doc
    "`last : List a → Option a` (recursive, the base arm builds None, the singleton arm wraps
           the element): at Int64 the payload is 2, at String its byte-len is 2 → 4. The sum-payload
           face of the tie — the produced OPTION's payload var must be the argument's element var
           through both the recursive arm and the constructor arm (a per-arm freshen severs one).")
  (input
    (do
      (def
        (last xs)
        (match xs (#list() (None unit)) (#list(h) (Some h)) (#list(h (.. t)) (last t))))
      (def
        (main (: d Int64))
        (+
          (match (last #list(1 2)) ((Some v) v) ((None _) -1))
          (match (last #list("a" "bc")) ((Some s) (String.byte-len s)) ((None _) -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 4 Int64))
  (live-objects known-leak))

; --- Recursive-generic transformer closure-tie: the element-change and self-compose faces -----------
; 7b67724e5 ties a recursive-generic transformer's closure domain to the mapped element (its pins
; cover map/filter composed at two types). These pin the faces its pins don't, promoted from passing
; breaker probes.
(case
  "a recursive-generic transformer with an element-CHANGING closure ties domain to codomain"
  (doc
    "`gmap (fn s → String.byte-len s) [\"abc\"]` → [3]: the closure maps String → Int64, so the
           tie is between DIFFERENT types — `f`'s domain to the input element (String) and the result
           list's element to `f`'s codomain (Int64). The fix's own pins keep the element type fixed
           (Int→Int, String→String); this pins the type-CHANGING map, where a severed tie leaves both
           the domain and the result element undetermined.")
  (input
    (do
      (def
        (gmap f xs)
        (match
          xs
          (#list() #list())
          (#list(h (.. t)) (List.concat (List.push #list() (f h)) (gmap f t)))))
      (def
        (main (: d Int64))
        (Option.expect (List.at (gmap (fn (s) (String.byte-len s)) #list("abc")) 0) "i"))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a recursive-generic transformer composes with itself"
  (doc
    "`gmap (*2) (gmap (+1) [3])` = [(3+1)·2] = [8]: the inner map's RESULT element must tie to
           the outer map's closure domain across the composition seam (both Int64 here, but the tie
           is what lets the outer instantiation resolve its element from the inner's result rather
           than a free var). The transformer analogue of the producer self-composition pin.")
  (input
    (do
      (def
        (gmap f xs)
        (match
          xs
          (#list() #list())
          (#list(h (.. t)) (List.concat (List.push #list() (f h)) (gmap f t)))))
      (def
        (main (: d Int64))
        (Option.expect (List.at (gmap (fn (x) (* x 2)) (gmap (fn (x) (+ x 1)) #list(3))) 0) "i"))
      (export main)))
  (call main (: 0 Int64))
  (output (: 8 Int64))
  (live-objects known-leak))

; --- Identity/constant closures through a recursive-generic HOF -----------------------------------
; An IDENTITY closure through a recursive-generic list HOF composes at ONE element type AND at TWO
; (the transformer closure-tie fix — `solved_lambda_arrow_under` — ties the closure's result to its
; domain, so the two coexisting monomorphizations each bind their own element). Both faces pin here.
(case
  "an identity closure threads through a recursive-generic HOF at one element type"
  (doc
    "`gmap (fn (x) x) [3 4]` → [3 4] (len 2): a pure IDENTITY closure works when the HOF is used
           at ONE element type — the result var ties from the single call site's element.")
  (input
    (do
      (def
        (gmap f xs)
        (match
          xs
          (#list() #list())
          (#list(h (.. t)) (List.concat (List.push #list() (f h)) (gmap f t)))))
      (def (main (: d Int64)) (List.len (gmap (fn (x) x) #list(3 4))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "an identity closure threads through a recursive-generic HOF at TWO element types"
  (doc
    "The multi-instantiation face: the SAME `gmap` list HOF used with a pure IDENTITY closure at
           Int64 `[3 4]` (len 2) AND String `[\"a\" \"b\" \"c\"]` (len 3) in one program → 2 + 3 = 5.
           Previously this DECLINED (the two identity instantiations' result vars could not unify Int
           and String); now composes — the transformer closure-tie ties each closure's result to its
           OWN domain, so each monomorphization binds its element independently. Runs on both backends.")
  (input
    (do
      (def
        (gmap f xs)
        (match
          xs
          (#list() #list())
          (#list(h (.. t)) (List.concat (List.push #list() (f h)) (gmap f t)))))
      (def
        (main (: d Int64))
        (+ (List.len (gmap (fn (x) x) #list(3 4))) (List.len (gmap (fn (s) s) #list("a" "b" "c")))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64))
  (live-objects known-leak))

(case
  "a constant-returning closure threads through a recursive-generic HOF"
  (doc
    "`gmap (fn (x) 9) [1 2]` → [9 9], element 0 = 9: a closure that IGNORES its parameter and
           returns a constant — its domain ties to the element (Int64) while its codomain is fixed
           (Int64) independent of the domain. Pins the ignore-param face (distinct from identity: the
           result does NOT come from the domain, so a different corner of the tie).")
  (input
    (do
      (def
        (gmap f xs)
        (match
          xs
          (#list() #list())
          (#list(h (.. t)) (List.concat (List.push #list() (f h)) (gmap f t)))))
      (def (main (: d Int64)) (Option.expect (List.at (gmap (fn (x) 9) #list(1 2)) 0) "i"))
      (export main)))
  (call main (: 0 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

; --- PASS-THROUGH closure NEIGHBORS of the domain-tie fix (single element type) ----------------------
; The domain-tie fix ties a pass-through closure's RESULT to its DOMAIN, so `(fn (s) s)` composes through
; a recursive-generic transformer at a SINGLE element type (landed above). These pin the UNPINNED
; single-type neighbors of that face: two chained maps, a compound (tuple) element, an Int element (the
; landed case used String), and a pass-through whose body threads its arg through a trivial `let`. Like
; the landed single-type case these are TODO on the rust backend (the whole recursive-generic-closure-
; through-Iter family is a known rust gap) — they pin the WASM result-flow.
(case
  "an identity closure composes through TWO chained recursive-generic maps at one element type"
  (doc
    "The composition face of the domain-tie fix: `(fn (s) s)` threaded through gmap TWICE must keep
           its result←domain flow across the composition. gmap(gmap [1,2,3] id) id then icount = 3. If the
           result tie held for one map but not a second (the closure re-instantiated at the outer map with
           a free result), the outer gmap's `Iter b` result would go unsolved and decline.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (gmap (gmap (from-list #list(1 2 3)) (fn (s) s)) (fn (s) s))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "an identity closure composes over a COMPOUND (tuple) element at one element type"
  (doc
    "The pass-through result←domain flow over a COMPOUND element, not a scalar: `(fn (s) s)` over an
           Iter of tuples `[(1,\"x\"),(2,\"y\")]`, icount = 2. Pins that the domain tie carries a tuple-typed
           domain to the result, not only a primitive — the closure's result is the whole tuple type.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (gmap (from-list #list(#tuple(1 "x") #tuple(2 "y"))) (fn (s) s))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "an identity closure composes at a single Int element type (not only String)"
  (doc
    "The landed single-type case used a String element; this pins an Int element so the tie is shown
           element-type-agnostic, not String-specific. gmap [10,20] id, icount = 2.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (gmap (from-list #list(10 20)) (fn (s) s))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a pass-through closure that returns its arg via a trivial let composes at one element type"
  (doc
    "The pass-through body need not be a BARE variable: `(fn (s) (let ((x s)) x))` still has its
           result determined by its domain (through the let-bound copy). gmap [\"p\",\"q\"] over it, icount = 2.
           Pins that the result←domain flow survives a let in the closure body, not only a bare identity.")
  (input
    (do
      (type Iter (Nil) (Cons a (Iter a)))
      (def
        (from-list xs)
        (match xs (#list() (Iter.Nil)) (#list(h (.. t)) (Iter.Cons h (from-list t)))))
      (def
        (gmap it f)
        (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
      (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
      (def (main) (icount (gmap (from-list #list("p" "q")) (fn (s) (let ((x s)) x)))))
      (export main)))
  (output (: 2 Int64))
  (live-objects known-leak))

; ============================================================================================
; NON-SCALAR entry arguments across the exported-entry / (call …) boundary. The rust GATE DRIVER used to
; write the RAW Cadenza literal/expr text (`100N`, `1R`, `((. Bytes of) …)`, `(list …)`, `(Some …)`) into
; Rust source — none valid Rust (invalid `N`/`R` suffix, a stray `.`, a bare list) — while the emitted
; LIBRARY was correct. Fixed in the rust gate harness (`rust_call_arg` now lowers each non-scalar arg via
; the SAME construction the library body uses: `cdz_num::Big`/`Rational`, a `Vec<u8>`/`vec!`, `Option`).
; breaker-found CLUSTER (corpus-bugfix); the surface was wholly untested (every case built the value INSIDE
; the program). On wasm the picture is now SPLIT (the non-scalar entry lift was realized for the leaf/collection
; shapes): the Bytes / List / Option entry args CROSS (copied into a value-heap value at the entry boundary),
; while the BigInt / Rational / Symbol entry args STILL DECLINE (a sound todo — their non-scalar leaf is not yet
; lifted). On rust they all run via the marshal, matching the recorded value. (The String member — likewise
; read-only-crosses on wasm now — is pinned in 13-strings.sexp.)
(case
  "a BigInt entry argument is marshalled through the value's own constructor"
  (doc
    "`(def (main (: a BigInt)) (= a 100N))` called with `100N` → true. The rust driver marshals the
           BigInt arg as `cdz_num::Big::from_i64(100)` (the in-body constructor), NOT the raw `100N` text
           (which is an invalid Rust suffix). wasm declines the BigInt entry arg (a sound todo).")
  (input (do (def (main (: a BigInt)) (= a 100N)) (export main)))
  (call main (: 100N BigInt))
  (output (: true Bool)))

(case
  "a BigInt entry parameter is COMPUTED on, not just compared"
  (doc
    "The compute face of the BigInt-entry-arg marshal (rust fix c8457fbf5). Where the case above only
           EQUALITY-checks the arg, this MULTIPLIES it: `(def (main (: a BigInt)) (* a (BigInt.of 1000000)))`.
           This is the shape that exposed an E0308 artifact-no-build (breaker-found) — `cdz check` accepted
           it but the emitted rust driver marshaled the arg with a type that did not match the fn signature.
           The rust gate driver now reads the emitted fn's param types and marshals the bare-decimal arg as an
           owned `cdz_num::Big` (the BigInt twin of the String `.to_string()` marshal), so main(5)=5000000 and
           main(-3)=-3000000 build + run. wasm declines the BigInt entry arg (a sound todo, like the case
           above).")
  (input (do (def (main (: a BigInt)) (* a (BigInt.of 1000000))) (export main)))
  (call main (: 5 BigInt))
  (output (: 5000000 BigInt))
  (call main (: -3 BigInt))
  (output (: -3000000 BigInt)))

(case
  "a BigInt entry parameter added to a beyond-i64 annotated literal"
  (doc
    "The companion face: a BigInt entry param added to a body literal that EXCEEDS i64/i128 range —
           `(def (main (: a BigInt)) (+ a (: 100000000000000000000 BigInt)))` (10^20 > i64::MAX). Verified
           that the entry-arg marshal fix (c8457fbf5) covers this too: the beyond-i64 body literal is lowered
           through the same owned-BigInt path, so main(1) = 100000000000000000001 builds + runs on rust. wasm
           declines the BigInt entry arg (sound todo).")
  (input (do (def (main (: a BigInt)) (+ a (: 100000000000000000000 BigInt))) (export main)))
  (call main (: 1 BigInt))
  (output (: 100000000000000000001 BigInt)))

(case
  "a Rational entry argument is marshalled through Rational::new"
  (doc
    "`(def (main (: r Rational)) (= r 1R))` called with `1R` → true. The driver marshals it as
           `cdz_num::Rational::new(Big::from_i64(1), Big::from_i64(1))`, not the invalid `1R` literal.")
  (input (do (def (main (: r Rational)) (= r 1R)) (export main)))
  (call main (: 1R Rational))
  (output (: true Bool)))

(case
  "a Bytes entry argument is marshalled as a Vec<u8>"
  (doc
    "`(def (main (: b Bytes)) (Bytes.len b))` called with the byte list `(list 1 2 3)` annotated as
           `Bytes` → 3. The driver marshals it as `vec![1u8, 2u8, 3u8]` — the byte-list literal form both
           backends accept for a `Bytes` argument (the canonical `(: (list …) Bytes)` spelling, as the ep5
           multi-param case uses); the wasm entry-param lift copies those bytes into a value-heap Bytes.")
  (input (do (def (main (: b Bytes)) (Bytes.len b)) (export main)))
  (call main (: #list(1 2 3) Bytes))
  (output (: 3 Int64)))

(case
  "a List entry argument is marshalled as a vec!"
  (doc
    "`(def (main (: xs (List Int64))) (List.len xs))` called with `(list 1 2 3)` → 3. The driver
           marshals it as `vec![1, 2, 3]`, not the bare `(list 1 2 3)` text; the wasm entry lift copies the
           elements into a value-heap List, so it crosses on both backends.")
  (input (do (def (main (: xs (List Int64))) (List.len xs)) (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 3 Int64)))

(case
  "an Option (sum) entry argument is marshalled as a native Option"
  (doc
    "`(def (main (: o (Option Int64))) (match o ((Some n) n) ((None _) -1)))` called with `(Some 5)`
           → 5. The driver marshals it as `Some(5)` (the native enum the backend emits), not the bare
           `(Some 5)` text; the wasm entry lift copies the sum into a value-heap Option, so it crosses on
           both backends.")
  (input (do (def (main (: o (Option Int64))) (match o ((Some n) n) ((None _) -1))) (export main)))
  (call main (: (Some 5) (Option Int64)))
  (output (: 5 Int64)))

(case
  "a Symbol entry parameter compares to an interned constant"
  (doc
    "`(def (main (: s Symbol)) (= s (Symbol.of \"read\")))` called with `(: #\"read\" Symbol)` → true.
           The rust driver marshals the `#\"read\"` symbol literal as `\"read\".to_string()` (a Symbol
           param emits as an owned String in the rust backend — strip the `#` sigil, marshal like the String
           entry arm; the driver used to emit the raw `#\"read\"` Cadenza text → a rustc syntax-error
           no-build, breaker-found, same family as the BigInt entry marshal). wasm declines the Symbol entry
           arg (a sound todo). Completes the entry-param-marshal family: String / BigInt / Symbol.")
  (input (do (def (main (: s Symbol)) (= s (Symbol.of "read"))) (export main)))
  (call main (: #"read" Symbol))
  (output (: true Bool)))

; ============================================================================================
; MATCH-INTO-IF fusion (backend-independent Core opt, v-core-opt): a `match` over a SUM built through an
; `if` pushes the match INTO each branch so each branch's constant constructor folds to the arm body —
; `(match (if c (Some x) (None)) ((Some v) v) ((None) 0))` → `(if c x 0)`. The un-fused form built +
; deconstructed a THROWAWAY sum per if-branch (heap alloc + box + arr-get/unbox to read the payload back);
; the fused form is heap-free. These cases pin the OBSERVABLE VALUE unchanged on BOTH backends across both
; branches (a miscompile if the fold altered a result); the byte/heap-count win is asserted in the lib test
; `a_match_over_an_if_selected_sum_pushes_into_the_branches`.
(case
  "a match over an if-selected Option folds through the then branch (Some)"
  (doc
    "`(match (if (> x 0) (Some x) (None)) ((Some v) v) ((None) 0))` at x=5 → the `if` selects `(Some
           5)`, which the pushed-in match folds to the payload `5`. Value unchanged by the fusion.")
  (input
    (do
      (type Option (Some Int64) None)
      (def
        (f (: x Int64))
        (match (if (> x 0) (Option.Some x) Option.None) ((Option.Some v) v) (Option.None 0)))
      (export f)))
  (call f (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a match over an if-selected Option folds through the else branch (None)"
  (doc
    "`(match (if (> x 0) (Some x) (None)) ((Some v) v) ((None) 0))` at x=-3 → the `if` selects
           `(None)`, which the pushed-in match folds to the None arm body `0`. Value unchanged.")
  (input
    (do
      (type Option (Some Int64) None)
      (def
        (f (: x Int64))
        (match (if (> x 0) (Option.Some x) Option.None) ((Option.Some v) v) (Option.None 0)))
      (export f)))
  (call f (: -3 Int64))
  (output (: 0 Int64)))

; ============================================================================================
; CASE-OF-MATCH fusion (the twin of match-into-if over a MATCH scrutinee, v-core-opt): a `match` over a sum
; built through an INNER match pushes the outer match INTO each inner-arm body so each inner arm's constant
; constructor folds — (match (match (> n 0) (true (Some n)) (false (None))) ((Some v) v) ((None) 0)) ->
; (match (> n 0) (true n) (false 0)). The inner match built a throwaway sum per arm purely to be
; deconstructed by the outer match; the fused form is heap-free. These pin the observable value unchanged on
; BOTH backends across both inner arms; the heap-free win is asserted in the lib test
; a_match_over_a_match_selected_sum_pushes_into_the_arms.
(case
  "a match over a match-selected Option folds through the true arm (Some)"
  (doc
    "`(match (match (> n 0) (true (Some n)) (false (None))) ((Some v) v) ((None) 0))` at n=5 → the
           inner match selects `(Some 5)`, which the pushed-in outer match folds to the payload `5`. Value
           unchanged by the fusion (case-of-match twin of match-into-if).")
  (input
    (do
      (type Option (Some Int64) None)
      (def
        (f (: n Int64))
        (match
          (match (> n 0) (true (Option.Some n)) (false Option.None))
          ((Option.Some v) v)
          (Option.None 0)))
      (export f)))
  (call f (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a match over a match-selected Option folds through the false arm (None)"
  (doc
    "`(match (match (> n 0) (true (Some n)) (false (None))) ((Some v) v) ((None) 0))` at n=-3 → the
           inner match selects `(None)`, which the pushed-in outer match folds to the None arm body `0`.
           Value unchanged.")
  (input
    (do
      (type Option (Some Int64) None)
      (def
        (f (: n Int64))
        (match
          (match (> n 0) (true (Option.Some n)) (false Option.None))
          ((Option.Some v) v)
          (Option.None 0)))
      (export f)))
  (call f (: -3 Int64))
  (output (: 0 Int64)))

(case
  "a recursive walker applies two different capture closures per element"
  (doc
    "Higher-order iteration: a TOP-LEVEL recursive walker `each` takes the closure as a PARAM
           and applies it per element through the recursion (the recursive-fn-as-VALUE spelling
           declines; recursion at the def level with a closure param is the supported idiom this
           pins). TWO different k-capturing closures over one list: k=2 -> sum(v·2)=12 and
           sum(v+2)=12 (1212); k=0 separates them -> 0 and 6 (6). The closure crosses the recursive
           call boundary each step — an env lost or re-bound mid-recursion breaks the sum.")
  (input
    (do
      (def
        (each (: xs (List Int64)) (: f (-> Int64 Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (each t f (+ acc (f h))))))
      (def
        (main (: k Int64))
        (do
          (def xs (List.push (List.push (List.push #list() 1) 2) 3))
          (+ (* 100 (each xs (fn ((: v Int64)) (* v k)) 0)) (each xs (fn ((: v Int64)) (+ v k)) 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1212 Int64))
  (call main (: 0 Int64))
  (output (: 6 Int64))
  (live-objects known-leak))

(case
  "compose builds closures capturing TWO function values in one env, both orders"
  (doc
    "The two-fn-capture face (the :1397 pin stores ONE fn handle in a closure cell): `compose`
           returns `(fn (x) (f (g x)))` whose env holds BOTH parameter closures, one of which itself
           captures the runtime k. Both orders applied: (add-k ∘ dbl)(5) = 13 and (dbl ∘ add-k)(5) = 16
           at k=3 (1316); k=0 collapses both to 10 (1010) separating capture from composition. An env
           that stored one handle and re-derived the other (or swapped the slots) flips the digits.")
  (input
    (do
      (def (compose (: f (-> Int64 Int64)) (: g (-> Int64 Int64))) (fn ((: x Int64)) (f (g x))))
      (def
        (main (: k Int64))
        (do
          (def add-k (fn ((: x Int64)) (+ x k)))
          (def dbl (fn ((: x Int64)) (* x 2)))
          (def fg (compose add-k dbl))
          (def gf (compose dbl add-k))
          (+ (* 100 (fg 5)) (gf 5))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1316 Int64))
  (call main (: 0 Int64))
  (output (: 1010 Int64)))

; A do-def shadowing a function PARAMETER is a well-defined shadow (the def rebinds the name for
; later refs). Regression (breaker #37): the resolver recorded the shadow def's binding at the wrong
; scope level, so a def-over-param UNBOUND the name (false CDZ0101 'unbound name v') for all later
; references. Fixed by v-inference 6566bff81 (binder-copy scope-level fix). def-over-def (:1267) +
; def-over-match-binder already worked; this is the param-shadow companion.
(case
  "a do-def shadowing a function parameter is a well-defined shadow"
  (input
    (do (def (f (: v Int64)) (do (def v (* v 2)) v)) (def (main (: k Int64)) (f k)) (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

; A do-def shadowing a LET binding (inside a fn) rebinds and its init EVALUATES. Regression (breaker
; #37, soundness): a def-over-let-in-fn was silently DROPPED — the shadow def was dead-code-eliminated
; wholesale, so the value stayed the let's AND a trapping init (def v (/ 1 0)) did NOT trap. Same
; wrong-scope-level root as the param face; fixed by the same v-inference 6566bff81 — now rebinds
; (10) and a trapping init surfaces CDZ0304 at compile.
(case
  "a do-def shadowing a LET binding rebinds and its init evaluates"
  (input
    (do
      (def (f (: k Int64)) (let ((v k)) (do (def v (* v 2)) v)))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "an unannotated generic instantiates at a FUNCTION type and a scalar in one body"
  (doc
    "The fn-type instantiation of a value generic: `dup x = (tuple x x)` is used at
           `(-> Int64 Int64)` (duplicating a CLOSURE — both tuple slots hold the same fn handle,
           each applied independently) AND at Int64, in one body. (p.0)(k) + (p.1)(3) + q.0 =
           2k·100 + 6 + k → 1011 at k=5, 6 at k=0. A specialization that boxed fn-typed x as a
           scalar (or shared one instantiation across the two types) breaks an application or the
           scalar read.")
  (input
    (do
      (def (dup x) #tuple(x x))
      (def
        (main (: k Int64))
        (do
          (def p (dup (fn ((: y Int64)) (* y 2))))
          (def q (dup k))
          (+ (* 100 ((. p 0) k)) (+ ((. p 1) 3) (. q 0)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1011 Int64))
  (call main (: 0 Int64))
  (output (: 6 Int64)))

; --- Higher-order shapes: the unannotated generic compose at two arrow types, and the twice-
; combinator tower (a closure in a closure's env in another's). ---
(case
  "an UNANNOTATED generic compose instantiates at a scalar arrow AND a String arrow in one program"
  (doc
    "The compose pins above are explicitly type-annotated monomorphic; this is the UNANNOTATED (def (compose f g) …) instantiated at TWO arrow types in one program — Int64→Int64 twice with order-sensitivity intact through the generic (inc∘dbl vs dbl∘inc) AND String→String (shout∘shout, byte-len read). The higher-order generic must monomorphize per instantiation; a single-instantiation specialization mistypes the String site.")
  (input
    (do
      (def (compose f g) (fn (x) (f (g x))))
      (def (inc (: x Int64)) (+ x 1))
      (def (dbl (: x Int64)) (* x 2))
      (def (shout (: s String)) (String.concat s "!"))
      (def
        (main (: k Int64))
        (do
          (def inc-then-dbl (compose dbl inc))
          (def dbl-then-inc (compose inc dbl))
          (def excite (compose shout shout))
          (+ (* 1000 (inc-then-dbl k)) (+ (* 10 (dbl-then-inc k)) (String.byte-len (excite "hi"))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12114 Int64)))

(case
  "a TWICE combinator stacks — a closure capturing a closure capturing a closure applies through the tower"
  (doc
    "The compose pins hold two INDEPENDENT fns in one env; this TOWER puts a closure IN a closure's env slot IN another's (twice(twice(adder k))): applying add-4k dispatches call_indirect→env-read→call_indirect→env-read→leaf, four leaf applications. An env-slot type confusion or one-level-deep env copy breaks the second twice. Both levels read (13/7 at k=3).")
  (input
    (do
      (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))
      (def (twice f) (fn ((: x Int64)) (f (f x))))
      (def
        (main (: k Int64))
        (do
          (def add-k (adder k))
          (def add-2k (twice add-k))
          (def add-4k (twice add-2k))
          (+ (* 100 (add-4k 1)) (add-2k 1))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1307 Int64)))

; --- The recursion-shape completions: a divide-and-conquer sort (recursion TREE with a compound
; intermediate) and Ackermann (NESTED recursion — a self-call in a self-call's argument). ---
(case
  "a full MERGE SORT (alternating split + ordered merge) sorts a runtime list, duplicates kept"
  (doc
    "The divide-and-conquer sort (corpus sorts are single-pass insorts): msort calls two helpers — msplit(alternating flip) and merge(3-way match), each independently self-recursive — plus itself, a TUPLE-of-lists intermediate crossing the recursion, TWO recursive msort calls per frame (a real recursion tree, not linear — and NOT a mutual-recursion cycle: the helpers never call back to msort), and a digit-encoded FULL-order read. The k=9 face keeps a duplicate — two 9s must both survive the merge.")
  (input
    (do
      (def
        (msplit (: xs (List Int64)) (: a (List Int64)) (: b (List Int64)) (: flip Int64))
        (match
          xs
          (#list() #tuple(a b))
          (#list(h (.. t))
            (if (= flip 0) (msplit t (List.push a h) b 1) (msplit t a (List.push b h) 0)))))
      (def
        (merge (: a (List Int64)) (: b (List Int64)) (: acc (List Int64)))
        (match
          a
          (#list() (List.concat acc b))
          (#list(ha (.. ta))
            (match
              b
              (#list() (List.concat acc a))
              (#list(hb (.. tb))
                (if (< hb ha) (merge a tb (List.push acc hb)) (merge ta b (List.push acc ha))))))))
      (def
        (msort (: xs (List Int64)))
        (if
          (< (List.len xs) 2)
          xs
          (match (msplit xs #list() #list() 0) (#tuple(a b) (merge (msort a) (msort b) #list())))))
      (def
        (digits (: xs (List Int64)) (: i Int64) (: acc Int64))
        (match
          (List.at xs i)
          ((Option.Some v) (digits xs (+ i 1) (+ (* acc 10) v)))
          ((Option.None _u) acc)))
      (def (main (: k Int64)) (digits (msort #list(5 k 8 1 9 3 7)) 0 0))
      (export main)))
  (call main (: 2 Int64))
  (output (: 1235789 Int64))
  (call main (: 9 Int64))
  (output (: 1357899 Int64))
  (live-objects known-leak))

(case
  "ACKERMANN evaluates — a recursive call in the ARGUMENT of a recursive call (not primitive-recursive)"
  (doc
    "The recursion pins cover tail/accumulable-non-tail/mutual/tree — this is NESTED recursion: a self-call in a self-call's ARGUMENT ((ack (- m 1) (ack m (- n 1)))). The inner call fully evaluates mid-argument-list with the outer frame's operands live; the outer call LOOKS accumulable but is not, so a misfiring tail-transform or argument-slot reuse corrupts the tower. ack(3,3)=61 is a deep non-tail evaluation tower.")
  (input
    (do
      (def
        (ack (: m Int64) (: n Int64))
        (if (= m 0) (+ n 1) (if (= n 0) (ack (- m 1) 1) (ack (- m 1) (ack m (- n 1))))))
      (def (main (: m Int64) (: n Int64)) (ack m n))
      (export main)))
  (call main (: 2 Int64) (: 3 Int64))
  (output (: 9 Int64))
  (call main (: 3 Int64) (: 3 Int64))
  (output (: 61 Int64)))

; --- Closure handles swapping parameter slots through repeated tail calls. ---
(case
  "two closure handles swap parameter slots through repeated tail calls"
  (doc
    "The FN-HANDLE member of the tail-call permutation family (scalar, heap-list and 3-cycle
           are its siblings): `(spin (- n 1) g f)` swaps two closure values per iteration — parity
           decides which body answers each slot (even k → 12, odd k → 21, k=0 identity → 12). A
           call_indirect table slot cached per PARAMETER position (rather than per VALUE) answers
           the same body from both slots after a swap; the parity split catches it.")
  (input
    (do
      (def
        (spin (: n Int64) (: f (-> Int64 Int64)) (: g (-> Int64 Int64)))
        (if (= n 0) (+ (* 10 (f 0)) (g 0)) (spin (- n 1) g f)))
      (def (main (: k Int64)) (spin k (fn ((: x Int64)) (+ x 1)) (fn ((: x Int64)) (+ x 2))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 12 Int64))
  (call main (: 3 Int64))
  (output (: 21 Int64))
  (call main (: 0 Int64))
  (output (: 12 Int64))
  (live-objects known-leak))

; --- Tail-call parameter permutations (scalar swap, 3-cycle rotation, heap-slot swap) and the
; generation-capture-before-shadow closure. ---
(case
  "an iterative fibonacci swaps two accumulators through the tail call"
  (doc
    "The ARGUMENT-PERMUTATION hazard of tail recursion (the tree-recursive fib is :2954):
           `(fib (- n 1) b (+ a b))` passes b INTO a's slot while a still feeds b's new value —
           a tail-call lowering that assigns parameter slots IN ORDER without temporaries clobbers
           a before (+ a b) reads it, degenerating the sequence (fib(10) = 55; the correct swap
           is what makes 55 ≠ a power of b's seed). k=0 reads the initial a (0) untouched.")
  (input
    (do
      (def (fib (: n Int64) (: a Int64) (: b Int64)) (if (= n 0) a (fib (- n 1) b (+ a b))))
      (def (main (: k Int64)) (fib k 0 1))
      (export main)))
  (call main (: 10 Int64))
  (output (: 55 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a THREE-cycle argument rotation permutes correctly through repeated tail calls"
  (doc
    "The full-cycle sharpening of the tail-call permutation pins (the 2-swap needs one temp;
           a 3-CYCLE a←c, b←a, c←b defeats single-temp AND any fixed assignment order — some order
           always reads a clobbered slot without proper temp discipline): one rotation reads 312,
           two 231, three restores 123 (= the k=0 identity, the cycle-length-3 witness). Any
           in-order or single-temp lowering produces a repeated digit; the three distinct
           permutations plus the fixpoint pin the whole cycle group.")
  (input
    (do
      (def
        (rot (: n Int64) (: a Int64) (: b Int64) (: c Int64))
        (if (= n 0) (+ (* 100 a) (+ (* 10 b) c)) (rot (- n 1) c a b)))
      (def (main (: k Int64)) (rot k 1 2 3))
      (export main)))
  (call main (: 1 Int64))
  (output (: 312 Int64))
  (call main (: 2 Int64))
  (output (: 231 Int64))
  (call main (: 3 Int64))
  (output (: 123 Int64))
  (call main (: 0 Int64))
  (output (: 123 Int64)))

(case
  "two heap-list accumulators swap slots through every tail call"
  (doc
    "The HEAP-handle variant of the tail-call arg-permutation pin: `(shuffle (- n 1) b
           (List.push a n))` swaps the two LIST handles per iteration while pushing into the
           outgoing one — the alternation interleaves pushes across both lists ([99,2]/[3,1]
           at k=3 → lens 2/2 → 22; k=0 → 1). A slot assignment that wrote b's handle over a's
           before the push read a (or a Perceus insertion that dropped the swapped-out handle a
           beat early) corrupts a length. The reference-arg twin of the scalar fib swap.")
  (input
    (do
      (def
        (shuffle (: n Int64) (: a (List Int64)) (: b (List Int64)))
        (if (= n 0) (+ (* 10 (List.len a)) (List.len b)) (shuffle (- n 1) b (List.push a n))))
      (def (main (: k Int64)) (shuffle k #list() #list(99)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 22 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a closure captures the param generation BEFORE a shadow and applies after"
  (doc
    "The capture × param-shadow face of the def-shadow fix (the generations pin covers
           def-over-def): `g` closes over the ORIGINAL param v, THEN the shadow rebinds v to 10v,
           and both generations are read — (g 1) = k+1 from the env, + the shadowed 10k (56 at
           k=5, 1 at k=0). A shadow fix that redirected the EXISTING capture cell to the new
           binding (instead of leaving the env on the old generation) reads 10k+1 and breaks the
           first addend.")
  (input
    (do
      (def (f (: v Int64)) (do (def g (fn ((: y Int64)) (+ v y))) (def v (* v 10)) (+ (g 1) v)))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; --- Emit-once memoization under MID-SOLVE schemes: the per-callee emit-once eligibility decision is
; memoized (perf), but MUST NOT memoize while the callee's scheme is still solving — a poisoned memo
; keys the emit to the first (incomplete) instantiation and mis-types a later one. These pin the seam
; from the outside at its two hardest shapes. ---
(case
  "a generic tuple-wrapper referenced mid-solve in a recursive group then at a second type"
  (doc
    "`wrap` (generic, tuple-building) is FIRST referenced inside recursive `go`'s body — while
           go's scheme is mid-solve — then instantiated OUTSIDE at String. A memoized emit-once
           decision taken during the mid-solve sighting would key wrap's emit to the Int64
           instantiation and mis-emit the String one (or vice versa). go 3 = 3+2+1 = 6, byte-len
           \"ab\" = 2 → 62. The corpus-tier witness of the don't-memoize-mid-solve rule (its lib
           lock-in is in rcdzc tests; this pins the observable value).")
  (input
    (do
      (def (wrap x) #tuple(x x))
      (def (go (: n Int64) (: acc Int64)) (if (= n 0) acc (go (- n 1) (+ acc (. (wrap n) 0)))))
      (def (main) (+ (* (go 3 0) 10) (String.byte-len (. (wrap "ab") 1))))
      (export main)))
  (output (: 62 Int64)))

(case
  "two mutually-recursive functions instantiate one generic chooser at different types"
  (doc
    "The mutual-recursion face: `ev` and `od` co-solve as one recursive group, and BOTH call the
           generic `pick2` — ev at Int64, od at String — while the group's schemes are still open. A
           memo poisoned by either sighting leaks one instantiation into the other (a String byte-len
           read of an Int64-keyed emit, or a numeric add of a String-keyed one). Runtime flag defeats
           folding: f=1 → ev 4 = 4·? … chain gives 40+3=43; f=0 → 10·10+1=101.")
  (input
    (do
      (def (pick2 (: f Int64) a b) (if (> f 0) a b))
      (def (ev (: f Int64) (: n Int64)) (if (= n 0) 0 (+ (od f (- n 1)) (pick2 f 2 5))))
      (def
        (od (: f Int64) (: n Int64))
        (if (= n 0) (String.byte-len (pick2 f "xyz" "q")) (ev f (- n 1))))
      (def (main (: f Int64)) (+ (* (ev f 4) 10) (od f 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 43 Int64))
  (call main (: 0 Int64))
  (output (: 101 Int64)))

(case
  "a closure built in a fused-match arm captures the payload binder and reads it after the match"
  (doc
    "The DEFERRED-read face of the fused-clone seam: each arm of a match on a CALL result builds
           a closure capturing that arm's SumPayload binder, and the closure is invoked only AFTER the
           match completes — the env must capture the CLONED binder (re-resolved against the branch
           value), not the original now-detached switch. k=7 → Hi arm's fn(+10h) → 73; k=2 → Lo arm's
           fn(+100w) → 203. The escaping-closure companion of the direct-read fused pins (a capture of
           the detached original reads garbage or CDZ0101s only on the deferred path).")
  (input
    (do
      (type Sz (Hi Int64) (Lo Int64))
      (def (mk x) (if (> x 5) (Hi x) (Lo x)))
      (def
        (main (: k Int64))
        (let
          ((f
              (match
                (mk k)
                ((Hi h) (fn ((: d Int64)) (+ (* 10 h) d)))
                ((Lo w) (fn ((: d Int64)) (+ (* 100 w) d))))))
          (f 3)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 73 Int64))
  (call main (: 2 Int64))
  (output (: 203 Int64)))

(case
  "two adapter records with different state types drive a take-while fold in one program"
  (doc
    "The {state, step} iterator-adapter protocol at the corpus tier (the v-iterators giter
           modules pin it in-library only): TWO adapters with DIFFERENT state types — an Int64
           counter (yields s, next s+1) and a tuple fib-state (yields a, next (b, a+b)) — each driven
           by a generic take-while fold (the stepf param is a closure returning Option(tuple elem
           state)). Counter from 1 while <4 → 1+2+3 = 6; fib from (1,1) while <=3 → 1+1+2+3 = 7 →
           67. Per-instantiation monomorphization of the generic state param is the seam (a shared
           emit mistypes one state). wasm computes; rust targets todo (higher-order closure-param
           family).")
  (input
    (do
      (def
        (sum-while (: st Int64) stepf (: lim Int64) (: acc Int64) (: fuel Int64))
        (if
          (= fuel 0)
          acc
          (match
            (stepf st)
            ((Some p)
              (match
                p
                (#tuple(e s2) (if (< e lim) (sum-while s2 stepf lim (+ acc e) (- fuel 1)) acc))))
            ((None u) acc))))
      (def
        (sum-while-t st stepf (: lim Int64) (: acc Int64) (: fuel Int64))
        (if
          (= fuel 0)
          acc
          (match
            (stepf st)
            ((Some p)
              (match
                p
                (#tuple(e s2) (if (<= e lim) (sum-while-t s2 stepf lim (+ acc e) (- fuel 1)) acc))))
            ((None u) acc))))
      (def
        (main)
        (+
          (* 10 (sum-while 1 (fn ((: s Int64)) (Some #tuple(s (+ s 1)))) 4 0 20))
          (sum-while-t
            #tuple(1 1)
            (fn (p) (match p (#tuple(a b) (Some #tuple(a #tuple(b (+ a b)))))))
            3
            0
            20)))
      (export main)))
  (output (: 67 Int64))
  (live-objects known-leak))

(case
  "closures extracted from a list by RUNTIME index and applied dispatch correctly"
  (doc
    "The callback-REGISTRY idiom: the list-of-closures pins above store + call directly;
           THIS case pins index-EXTRACTION + application — three distinct closures in a list, one selected by a fixed
           index and one by a runtime-computed index `(% k 3)`, each applied to a live argument. k=4 →
           fns[1](4)=40 ·100 + fns[1](7)=70 → 4070; k=3 → 30·100 + fns[0](7)=8 → 3008. A registry whose
           boxed-fn slots decayed to one shared code pointer (or whose index resolution mis-mapped the
           funcref table) dispatches the wrong callback — visible because all three closures compute
           different shapes.")
  (input
    (do
      (def
        (main (: k Int64))
        (do
          (def
            fns
            #list((fn ((: x Int64)) (+ x 1)) (fn ((: x Int64)) (* x 10)) (fn ((: x Int64)) (- x 5))))
          (+
            (* 100 (match (List.at fns 1) ((Some f) (f k)) ((None _u) -1)))
            (match (List.at fns (% k 3)) ((Some g) (g 7)) ((None _u) -1)))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4070 Int64))
  (call main (: 3 Int64))
  (output (: 3008 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "a pipeline chain threads handler STATE left-to-right through effectful stages"
  (doc
    "`|>` composed with effects: two chained pipe stages each perform `(Ctr.tick)` — the desugar
           to nested applications must evaluate stages LEFT-TO-RIGHT so the first stage reads state 1
           and the second reads the advanced 2 (5 + 100·1 + 10000·2 = 20105). A desugar that reordered
           or duplicated a stage's evaluation shifts a digit. Composes the pipe rewrite (pinned above as
           value-only) with handler state threading.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (stage1 (: v Int64)) (+ v (* 100 (Ctr.tick))))
      (def (stage2 (: v Int64)) (+ v (* 10000 (Ctr.tick))))
      (def
        (main (: k Int64))
        (handle Ctr 1 ((tick (u) s (resume s (+ s 1)))) (|> (|> k stage1) stage2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20105 Int64)))

(case
  "a fold whose accumulator applies each list closure IN SEQUENCE (apply-all chain)"
  (doc
    "The registry pins above extract ONE closure per read; this folds over the WHOLE list applying
           each closure to a threaded value — apply-all, the middleware-chain shape. [+1, *10, -5] over
           k: ((k+1)*10)-5 at k=5 → 55; at k=0 → 5. Each step reads its closure from the list spine and
           applies it to the previous step's result — a wrong fold order (or a re-read of closure 0) is
           immediately visible in the composed value.")
  (input
    (do
      (def
        (apply-all (: fs (List (-> Int64 Int64))) (: v Int64))
        (match fs (#list() v) (#list(h (.. t)) (apply-all t (h v)))))
      (def
        (main (: k Int64))
        (apply-all
          #list((fn ((: x Int64)) (+ x 1)) (fn ((: x Int64)) (* x 10)) (fn ((: x Int64)) (- x 5)))
          k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 55 Int64))
  (call main (: 0 Int64))
  (output (: 5 Int64))
  (live-objects known-leak))

; -- operators CURRY (#3633): a bare operator through a CURRIED-annotated HOF applied ((g a) b), and a let-bound partial ((+ 1)) applied — the partial-application witnesses (breaker batch 394) --
(case
  "ch01 a bare prim passed as a function value applies"
  (input
    (do
      (def (ap (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) ((g a) b))
      (def (main (: n Int64)) (ap + n 2))
      (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

(case
  "ch01e bare operator PARTIALLY applied — (let ((inc (+ 1))) (inc n))"
  (input (do (def (main (: n Int64)) (let ((inc (+ 1))) (inc n))) (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64)))

(case
  "a let-bound value inside a function taking a parameter compiles and runs"
  (doc
    "The beta-reduction atom-copy behavior: `(g 10)` inlines g's body `(let ((x (+ n 1))) (+ x x))`,
           substituting n->10 in the binding init; the let-local x must re-resolve against the substituted
           init. Runs to (+ 11 11) = 22.")
  (input (do (def (g (: n Int64)) (let ((x (+ n 1))) (+ x x))) (def (main) (g 10)) (export main)))
  (call main)
  (output (: 22 Int64)))

(case
  "a record field key coinciding with a parameter name survives the call, projected"
  (doc
    "A field key `(record (x 5))` and projection key `(. r x)` are LABELS, immune to beta-substitution
           of the param `x`; the record is not corrupted -> projecting x reads 5.")
  (input (do (def (f (: x Int64)) (. #record((= x 5)) x)) (def (main) (f 7)) (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a param-colliding record key does not corrupt a non-colliding field"
  (doc
    "Multi-field: the colliding key `x` (= param name) does not corrupt the record; project the
           non-colliding field y -> 2.")
  (input (do (def (f (: x Int64)) (. #record((= x 1) (= y 2)) y)) (def (main) (f 7)) (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a record field VALUE referencing the parameter still substitutes"
  (doc
    "The complement of key-immunity: a field VALUE that references the param STILL beta-substitutes
           (immunity is key-only). `(record (y x))` with x=7, projected y -> 7.")
  (input (do (def (f (: x Int64)) (. #record((= y x)) y)) (def (main) (f 7)) (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "each parameter of a wide six-param signature resolves to its own binder"
  (doc
    "`f` takes six params and references three out of order; a wrong binder index would compute a
           wrong result. (f 1 2 4 8 16 32) -> p1 + p3 + p5 = 2+8+32 = 42.")
  (input
    (do
      (def
        (f (: p0 Int64) (: p1 Int64) (: p2 Int64) (: p3 Int64) (: p4 Int64) (: p5 Int64))
        (+ p1 (+ p3 p5)))
      (def (main) (f 1 2 4 8 16 32))
      (export main)))
  (call main)
  (output (: 42 Int64)))

; -- partial-operator COMPOSITIONS (post-#3641): partial to a recursive HOF, runtime-operand partial through a record field, comparison-operator partial as a predicate (breaker batch 396) --
(case
  "cpp1 a PARTIAL operator passed directly to a recursive HOF"
  (input
    (do
      (def
        (each3 (: k Int64) (: acc Int64) (: g (-> Int64 Int64)))
        (if (= k 0) acc (each3 (- k 1) (+ acc (g k)) g)))
      (def (main (: n Int64)) (each3 3 0 (+ n)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 36 Int64))
  (live-objects known-leak))

(case
  "cpp2 a partial built from a RUNTIME operand, stored in a record, projected, applied"
  (input (do (def (main (: n Int64)) (let ((r #record((= f (+ n))))) (r.f 2))) (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

(case
  "cpp3 partials of a COMPARISON operator select via a HOF"
  (input
    (do
      (def (count3 (: p (-> Int64 Bool))) (+ (if (p 1) 1 0) (+ (if (p 5) 1 0) (if (p 9) 1 0))))
      (def (main (: n Int64)) (count3 (< n)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 2 Int64)))

(case
  "an exported two-Int64 addition runs over runtime args, not folded"
  (doc
    "An exported `(add (: a Int64) (: b Int64)) (+ a b)` is a real wasm function taking two s64 params;
           the body does not fold (params unknown) -> `local.get 0; local.get 1; i64.add`. add(20,22)=42.")
  (input (do (def (add (: a Int64) (: b Int64)) (+ a b)) (export add)))
  (call add (: 20 Int64) (: 22 Int64))
  (output (: 42 Int64)))

(case
  "the same exported addition over a second runtime arg pair recomputes"
  (doc
    "The SAME add export over a different pair proves the value is genuinely runtime, not a fold:
           add(100, -1) = 99.")
  (input (do (def (add (: a Int64) (: b Int64)) (+ a b)) (export add)))
  (call add (: 100 Int64) (: -1 Int64))
  (output (: 99 Int64)))

; -- breaker batch 403 (2026-08-26): higher-order APPLICATION flip pins — closures as first-class
; runtime values through storage and selection shapes (record field, list element, runtime branch),
; the eta-expanded prim-as-value control, a bare operator through an uncurried-annotated HOF, and
; String.from-bytes validating runtime-built bytes. All were filed as declines; flipped on trunk.
; Siblings ch01/ch01e (bare prim as value, partial application) pinned above.
(case
  "ch02 a closure stored in a record field projects and applies"
  (input
    (do (def (main (: n Int64)) (let ((r #record((= f (fn (x) (+ x n)))))) (r.f 2))) (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

(case
  "ch03 a closure stored in a LIST is fetched and applied"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (List.at #list((fn (x) (+ x n)) (fn (x) (* x n))) 1)
          ((Option.Some g) (g 3))
          ((Option.None) -1)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 30 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "ch04 a runtime-branch-selected closure applies"
  (input
    (do
      (def (main (: n Int64)) (let ((g (if (> n 5) (fn (x) (+ x 1)) (fn (x) (* x 2))))) (g n)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 8 Int64)))

(case
  "ch08 String.from-bytes of RUNTIME-built bytes validates"
  (input
    (do
      (def
        (main (: k Int64))
        (match
          (String.from-bytes (Bytes.of #list((UInt8.wrap k) (UInt8.wrap 98))))
          ((Option.Some s) (String.byte-len s))
          ((Option.None) -1)))
      (export main)))
  (call main (: 97 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "ch01b eta-expanded closure control for prim-as-value"
  (input
    (do
      (def (ap (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) ((g a) b))
      (def (main (: n Int64)) (ap (fn (a) (fn (b) (+ a b))) n 2))
      (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

(case
  "ch01d bare operator to an UNCURRIED-annotated HOF applied (g a b)"
  (input
    (do
      (def (ap (: g (-> Int64 Int64 Int64)) (: a Int64) (: b Int64)) (g a b))
      (def (main (: n Int64)) (ap + n 2))
      (export main)))
  (call main (: 40 Int64))
  (output (: 42 Int64)))

; -- breaker batch 432 (2026-08-26): the ENTRY-PARAM Slice-1 EDGE ladder (v-rb's String/Bytes
; param-lift, ~4-slice plan behind one admission gate). The pinned 14-row family covers the base
; faces; these pin the edges: TWO String params in order, a String BETWEEN scalars (positional
; correctness), one param used twice (len + content compare), the EMPTY string (ptr,0), Bytes+String
; in one signature, and pass-through to a helper. Rows wasm-todo / rust-pass — oracles are
; machine-verified against the rust reference; every row auto-flips when Slice 1 lands.
(case
  "ep1 TWO String entry params concat in declaration order"
  (input
    (do (def (main (: a String) (: b String)) (String.byte-len (String.concat a b))) (export main)))
  (call main (: "abc" String) (: "de" String))
  (output (: 5 Int64)))

(case
  "ep2 a String param BETWEEN two scalars keeps positional correctness"
  (input
    (do
      (def (main (: x Int64) (: s String) (: y Int64)) (+ (* x 100) (+ (String.byte-len s) y)))
      (export main)))
  (call main (: 3 Int64) (: "abcd" String) (: 7 Int64))
  (output (: 311 Int64)))

(case
  "ep3 one String param used TWICE (byte-len and content compare)"
  (input (do (def (main (: s String)) (+ (String.byte-len s) (if (= s "hi") 100 0))) (export main)))
  (call main (: "hi" String))
  (output (: 102 Int64)))

(case
  "ep4 an EMPTY String entry arg has byte-len zero (the ptr,0 edge)"
  (input (do (def (main (: s String)) (String.byte-len s)) (export main)))
  (call main (: "" String))
  (output (: 0 Int64)))

(case
  "ep5 a Bytes and a String param in ONE signature"
  (input
    (do
      (def (main (: b Bytes) (: s String)) (+ (* 10 (Bytes.len b)) (String.byte-len s)))
      (export main)))
  (call main (: #list(1 2 3) Bytes) (: "ab" String))
  (output (: 32 Int64)))

(case
  "ep6 a String entry param passed THROUGH to a helper"
  (input
    (do
      (def (measure (: t String)) (String.byte-len t))
      (def (main (: s String)) (+ 1 (measure s)))
      (export main)))
  (call main (: "xyz" String))
  (output (: 4 Int64)))

; -- breaker batch 433 (2026-08-26): the entry-param Slice 2-4 EDGE ladders (companion to the
; slice-1 batch 432). Slice 2 (List): indexed-walk sum, TWO list params, the EMPTY list. Slice 3
; (Option): Some payload, the None arm, Option BETWEEN scalars. Slice 4: BigInt beyond-i64
; arithmetic on the param, Rational exact arithmetic (n/d call spelling), Symbol keying a Map.
; Rows wasm-todo / rust-pass, oracles machine-verified on the rust reference — each slice's landing
; auto-flips its rung set.
(case
  "el1 a List entry param summed by an indexed walk"
  (input
    (do
      (def
        (suml (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) (+ v (suml xs (+ i 1)))) ((Option.None) 0)))
      (def (main (: xs (List Int64))) (suml xs 0))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 6 Int64)))

(case
  "el2 TWO List entry params measure independently"
  (input
    (do
      (def (main (: a (List Int64)) (: b (List Int64))) (+ (* 10 (List.len a)) (List.len b)))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)) (: #list(9) (List Int64)))
  (output (: 31 Int64)))

(case
  "el3 an EMPTY List entry param has length zero"
  (input (do (def (main (: xs (List Int64))) (List.len xs)) (export main)))
  (call main (: #list() (List Int64)))
  (output (: 0 Int64)))

; -- breaker batch 444 (2026-08-27): slice-2a boundary witness, cut the hour #3836 landed. A
; non-recursive List.at ELEMENT READ through the lifted value-heap vec passes 0-leak (verified on
; the debug-counters runtime) — pinning that element access is IN slice 2a while el1's recursive
; walk (xs escaping into the self-call) stays the declined side of the same boundary.
(case
  "el4 a List entry param's element read by a non-recursive List.at"
  (input
    (do
      (def (main (: xs (List Int64))) (match (List.at xs 1) ((Option.Some v) v) ((Option.None) -1)))
      (export main)))
  (call main (: #list(7 42 9) (List Int64)))
  (output (: 42 Int64)))

; -- breaker batch 448 (2026-08-27): the slice-2a FOLLOWUP rungs — per-width elements and a
; compound element, each len+element-read over the lifted vec. Slice 2a admits list<Int64> only
; (one load width); these decline on wasm today ("no component boundary representation") and pass
; on the rust reference, so each width/compound followup lands against a ready witness and
; auto-flips its rung. (List UInt8) is included deliberately: the u8 BOUNDARY shape is owned by
; Bytes, so a (List UInt8) param is a real decline, not covered by b"…".
(case
  "el5 a List-of-UInt8 entry param measures and reads an element"
  (input
    (do
      (def
        (main (: xs (List UInt8)))
        (+
          (* 100 (List.len xs))
          (match (List.at xs 1) ((Option.Some v) (Int64.of v)) ((Option.None) -1))))
      (export main)))
  (call main (: #list(7 9 5) (List UInt8)))
  (output (: 309 Int64)))

(case
  "el6 a List-of-Int32 entry param measures and reads an element"
  (input
    (do
      (def
        (main (: xs (List Int32)))
        (+
          (* 100 (List.len xs))
          (match (List.at xs 1) ((Option.Some v) (Int64.of v)) ((Option.None) -1))))
      (export main)))
  (call main (: #list(4 8) (List Int32)))
  (output (: 208 Int64)))

(case
  "el7 a List-of-Float64 entry param measures and compares an element"
  (input
    (do
      (def
        (main (: xs (List Float64)))
        (+
          (* 100 (List.len xs))
          (match (List.at xs 1) ((Option.Some v) (if (> v 2.5) 1 0)) ((Option.None) -1))))
      (export main)))
  (call main (: #list(1.5 3.5) (List Float64)))
  (output (: 201 Int64)))

(case
  "el8 a nested List-of-List entry param measures both levels"
  (input
    (do
      (def
        (main (: xs (List (List Int64))))
        (+
          (* 100 (List.len xs))
          (match (List.at xs 1) ((Option.Some inner) (List.len inner)) ((Option.None) -1))))
      (export main)))
  (call main (: #list(#list(1) #list(2 3)) (List (List Int64))))
  (output (: 202 Int64)))

; -- breaker batch 449 (2026-08-27): slice-2a COMPOSITION probes — what the escape-gate admits and
; declines when the lifted list param meets other subsystems. Admitted (borrow-shaped): a local
; closure CAPTURING the param (elc1), and that capture crossing a higher-order call boundary (elc5)
; — both reclaim to zero. Declined on wasm (consumer-shaped, the List analogs of el1's recursive
; escape): List.concat consuming two lifted params (elc4) and Map.insert keying by the param (elc2)
; — rungs for the escaping/consuming slice, rust-pass auto-flip rows.
(case
  "elc1 a local closure captures the lifted List entry param and is invoked twice"
  (input
    (do
      (def
        (main (: xs (List Int64)))
        (let ((f (fn ((: k Int64)) (+ k (List.len xs))))) (+ (f 10) (f 100))))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 116 Int64)))

(case
  "elc5 the capturing closure crosses a higher-order call boundary and answers from both invocations"
  (input
    (do
      (def (twice (: f (-> Int64 Int64)) (: k Int64)) (+ (f k) (f (* k 10))))
      (def (main (: xs (List Int64))) (twice (fn ((: k Int64)) (+ k (List.len xs))) 1))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 17 Int64)))

(case
  "elc6 a MIXED scalar+non-scalar bare export param list still gates B2 off (opt-level equivalent)"
  (doc
    "The MIXED-param sibling of elc1 fencing the #4793 B2-bare-entry gate's `at least ONE non-scalar
           param` condition: `main` takes BOTH a non-scalar `(List Int64)` entry param AND a scalar `Int64`
           one, a single export, its List param captured by a local closure invoked twice. The gate
           `b2_excluded_bare_entry_export` must fire on the PRESENCE of a non-scalar param even though a scalar
           param is also present (a regression that required ALL params non-scalar, or keyed only the first/
           sole param, would let B2's `Core::Let`-wrap reshape the body → the `try_bare_entry_param_component`
           None → the `non-scalar entry parameter not emitted` DECLINE at O2/O3 while O0/O1 compile = an opt-
           level-equivalence violation). `(f base) + (f 100)` with base=10, xs=(1 2 3) → 13 + 103 = 116; the
           OBSERVABLE outcome is identical across O0/O1/O2/O3 (verified by an opt-sweep).")
  (input
    (do
      (def
        (main (: xs (List Int64)) (: base Int64))
        (let ((f (fn ((: k Int64)) (+ k (List.len xs))))) (+ (f base) (f 100))))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)) (: 10 Int64))
  (output (: 116 Int64)))

(case
  "elc7 TWO closures capturing the same lifted List entry param keep B2 gated off (opt-level equivalent)"
  (doc
    "The MULTI-CAPTURE sibling of elc1: a single bare export whose `(List Int64)` entry param is captured
           by TWO distinct local closures (a `+` reader and a `*` reader), each invoked once. B2 sharing-aware-
           emit has two shared-param reads it could `Core::Let`-wrap — the #4793 gate skips the B2 plan for the
           whole export body so `try_bare_entry_param_component` still sees the RAW shape. `(f 10) + (g 3)` with
           xs=(1 2 3) → (10+3) + (3*3) = 13 + 9 = 22, identical across all four opt levels (a residual B2 that
           reshaped either capture at O2 would decline where O0/O1 compile).")
  (input
    (do
      (def
        (main (: xs (List Int64)))
        (let
          ((f (fn ((: k Int64)) (+ k (List.len xs)))) (g (fn ((: j Int64)) (* j (List.len xs)))))
          (+ (f 10) (g 3))))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 22 Int64)))

(case
  "an UNREFERENCED non-scalar-param helper stays opt-level-equivalent under force-lower-all"
  (doc
    "Fences the #4805 (Core-opt PassManager POST-layout — force-lower-all timing) × #4793 (non-scalar
           entry param declines on the EXPORT boundary) interaction: a module carries an UNREFERENCED helper
           `deadhelper` whose param is a non-scalar `(List Int64)` — the exact shape that declines on an EXPORT
           path — while the sole export `main` is a trivial scalar doubler. force-lower-all could lower the dead
           helper at O2/O3 (where O0/O1's cheaper pipeline skips or DCEs it); the pin asserts the OBSERVABLE
           outcome is identical across O0..O3 (main 5 → 10) — i.e. the non-scalar-param decline is EXPORT-
           boundary-scoped and does NOT leak onto an internal/dead helper under the new post-layout timing. A
           regression that force-lowered the dead helper into the export-path decline, or that broke its DCE,
           would decline at O2 where O0/O1 compile = an opt-level-equivalence violation.")
  (input
    (do
      (def (deadhelper (: xs (List Int64))) (List.len xs))
      (def (main (: n Int64)) (* n 2))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "elc2 a Map keyed by the lifted List entry param answers its value on lookup"
  (input
    (do
      (def
        (main (: xs (List Int64)))
        (let
          ((m (Map.insert (Map.insert Map.empty xs 7) #list(9 9) 5)))
          (match (Map.lookup m xs) ((Option.Some v) v) ((Option.None) -1))))
      (export main)))
  (call main (: #list(4 5 6) (List Int64)))
  (output (: 7 Int64)))

(case
  "elc4 List.concat consumes two lifted List entry params into one measured list"
  (input
    (do
      (def (main (: a (List Int64)) (: b (List Int64))) (List.len (List.concat a b)))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)) (: #list(9 8) (List Int64)))
  (output (: 5 Int64)))

; -- breaker batch 450 (2026-08-27): slice-2a × the effects fold. The lifted list param read from
; each position of a guest handler — the HANDLE BODY, the HANDLER ARM (a fold-scope closure over a
; boundary-lifted vec, the seam the escaped-closure recovery fixes hardened), and the SEED state.
; All three fold, answer exactly, and reclaim to zero.
(case
  "ele1 the lifted List entry param is read in a handle BODY alongside a dispatch"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: xs (List Int64)))
        (handle St 3 ((get (u) s (resume s s))) (+ (St.get) (List.len xs))))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 6 Int64)))

(case
  "ele2 the lifted List entry param is read inside a handler ARM feeding the resume value"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: xs (List Int64)))
        (handle St 10 ((get (u) s (resume (+ s (List.len xs)) s))) (St.get)))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 13 Int64)))

(case
  "ele3 the lifted List entry param SEEDS the handler state"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (main (: xs (List Int64))) (handle St (List.len xs) ((get (u) s (resume s s))) (St.get)))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 3 Int64)))

; -- breaker batch 451 (2026-08-27): boundary VALUES through #3852's per-width list lift — the
; classic width traps. u8 element loads must ZERO-extend (255 stays 255, not -1), i32 loads must
; SIGN-extend (-5 stays -5), three widths coexist in one wrapper with per-param strides, and an
; f64 element survives load+return exactly.
(case
  "wl1 a List-of-UInt8 entry param's extreme elements read back exactly (255 zero-extends, 0 reads 0)"
  (input
    (do
      (def
        (main (: xs (List UInt8)))
        (+
          (* 1000 (match (List.at xs 0) ((Option.Some v) (Int64.of v)) ((Option.None) -1)))
          (match (List.at xs 2) ((Option.Some v) (Int64.of v)) ((Option.None) -1))))
      (export main)))
  (call main (: #list(255 7 0) (List UInt8)))
  (output (: 255000 Int64)))

(case
  "wl2 a List-of-Int32 entry param's negative element sign-extends on read"
  (input
    (do
      (def
        (main (: xs (List Int32)))
        (match (List.at xs 0) ((Option.Some v) (Int64.of v)) ((Option.None) 0)))
      (export main)))
  (call main (: #list(-5 3) (List Int32)))
  (output (: -5 Int64)))

(case
  "wl3 three list widths (UInt8, Float64, Int64) coexist as params of one entry wrapper"
  (input
    (do
      (def
        (main (: a (List UInt8)) (: b (List Float64)) (: c (List Int64)))
        (+ (* 100 (List.len a)) (+ (* 10 (List.len b)) (List.len c))))
      (export main)))
  (call
    main
    (: #list(1 2 3) (List UInt8))
    (: #list(0.5 1.5) (List Float64))
    (: #list(9) (List Int64)))
  (output (: 321 Int64)))

(case
  "wl4 a List-of-Float64 entry param's element survives load and return exactly"
  (input
    (do
      (def
        (main (: xs (List Float64)))
        (match (List.at xs 1) ((Option.Some v) v) ((Option.None) -1.0)))
      (export main)))
  (call main (: #list(0.5 2.25 9.0) (List Float64)))
  (output (: 2.25 Float64)))

; -- breaker batch 452 (2026-08-27): the GENERAL-recursion-gate edge ladder, pre-delivered for the
; consuming slice. The slice's enabler is a general call-graph cycle check (tail-only
; mutual_loop_group misses el1's non-tail suml). Post-slice contract: grx1/grx2 must FLIP to pass
; (non-recursive consumption through a deep chain; recursion elsewhere must not poison the param) —
; grx3/grx4 must STAY todo (non-tail MUTUAL recursion; transitive reach through a non-recursive
; relay). A flip on grx3/grx4 is OVER-ADMISSION — the miscompiling recursive-param-slot shape.
(case
  "grx1 the entry List param flows through a two-deep non-recursive helper chain into a consuming concat"
  (input
    (do
      (def (finish (: ys (List Int64))) (List.len (List.concat ys #list(7))))
      (def (relay (: ys (List Int64))) (finish ys))
      (def (main (: xs (List Int64))) (relay xs))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 4 Int64)))

(case
  "grx2 a recursive scalar helper coexists with a non-recursive consuming use of the entry List param"
  (input
    (do
      (def (fact (: k Int64)) (if (= k 0) 1 (* k (fact (- k 1)))))
      (def
        (main (: xs (List Int64)))
        (let
          ((m (Map.insert Map.empty xs (fact 4))))
          (match (Map.lookup m xs) ((Option.Some v) v) ((Option.None) -1))))
      (export main)))
  (call main (: #list(1 2 3) (List Int64)))
  (output (: 24 Int64)))

(case
  "grx3 the entry List param threads a NON-TAIL mutual recursion (suma under +, sumb under *2+)"
  (input
    (do
      (def
        (suma (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) (+ v (sumb xs (+ i 1)))) ((Option.None) 0)))
      (def
        (sumb (: xs (List Int64)) (: i Int64))
        (match (List.at xs i) ((Option.Some v) (+ (* 2 v) (suma xs (+ i 1)))) ((Option.None) 0)))
      (def (main (: xs (List Int64))) (suma xs 0))
      (export main)))
  (call main (: #list(5 6 7) (List Int64)))
  (output (: 24 Int64)))

(case
  "grx4 the entry List param reaches a self-recursive summer through a non-recursive relay"
  (input
    (do
      (def
        (suml (: ys (List Int64)) (: i Int64))
        (match (List.at ys i) ((Option.Some v) (+ v (suml ys (+ i 1)))) ((Option.None) 0)))
      (def (relay (: ys (List Int64))) (suml ys 0))
      (def (main (: xs (List Int64))) (relay xs))
      (export main)))
  (call main (: #list(5 6 7) (List Int64)))
  (output (: 18 Int64)))

; -- breaker batch 453 (2026-08-27): the NESTED-list edge ladder for el8's recursive element lift
; (the outer element is a (ptr,len) descriptor whose inner vec must itself be lifted). Boundary
; rows the recursive lift must get right: an EMPTY outer list (no elements to descend into), an
; EMPTY inner list among non-empty ones (a zero-len descriptor mid-walk), and THREE-level nesting
; (the recursion must actually recurse, not special-case depth two). wasm-todo / rust-pass rungs.
(case
  "eln1 an EMPTY nested-list entry param has outer length zero"
  (input (do (def (main (: xs (List (List Int64)))) (List.len xs)) (export main)))
  (call main (: #list() (List (List Int64))))
  (output (: 0 Int64)))

(case
  "eln2 a nested-list entry param whose FIRST inner list is empty reads a zero inner length"
  (input
    (do
      (def
        (main (: xs (List (List Int64))))
        (+
          (* 100 (List.len xs))
          (match (List.at xs 0) ((Option.Some inner) (List.len inner)) ((Option.None) -1))))
      (export main)))
  (call main (: #list(#list() #list(5)) (List (List Int64))))
  (output (: 200 Int64)))

(case
  "eln3 a THREE-level nested-list entry param reads a leaf through two descents"
  (input
    (do
      (def
        (main (: xs (List (List (List Int64)))))
        (+
          (* 100 (List.len xs))
          (match
            (List.at xs 0)
            ((Option.Some mid)
              (+
                (* 10 (List.len mid))
                (match
                  (List.at mid 1)
                  ((Option.Some inner)
                    (match (List.at inner 0) ((Option.Some v) v) ((Option.None) -1)))
                  ((Option.None) -2))))
            ((Option.None) -3))))
      (export main)))
  (call main (: #list(#list(#list(9) #list(42 8))) (List (List (List Int64)))))
  (output (: 162 Int64)))

(case
  "eo1 an Option entry param delivers its Some payload"
  (input
    (do
      (def (main (: o (Option Int64))) (match o ((Option.Some v) v) ((Option.None) -1)))
      (export main)))
  (call main (: (Some 42) (Option Int64)))
  (output (: 42 Int64)))

(case
  "eo2 an Option entry param takes the None arm"
  (input
    (do
      (def (main (: o (Option Int64))) (match o ((Option.Some v) v) ((Option.None) -1)))
      (export main)))
  (call main (: (None unit) (Option Int64)))
  (output (: -1 Int64)))

(case
  "eo3 an Option param BETWEEN scalars keeps positions"
  (input
    (do
      (def
        (main (: x Int64) (: o (Option Int64)) (: y Int64))
        (+ (* x 100) (+ (match o ((Option.Some v) v) ((Option.None) 0)) y)))
      (export main)))
  (call main (: 3 Int64) (: (Some 20) (Option Int64)) (: 7 Int64))
  (output (: 327 Int64)))

(case
  "eb1 a BigInt entry param in beyond-i64 arithmetic"
  (input (do (def (main (: b BigInt)) (= (* b 2N) 24691357024641975308642N)) (export main)))
  (call main (: 12345678512320987654321N BigInt))
  (output (: true Bool)))

(case
  "er1 a Rational entry param in exact arithmetic"
  (input
    (do (def (main (: r Rational)) (= (+ r (Rational.of 1 6)) (Rational.of 1 2))) (export main)))
  (call main (: 1/3 Rational))
  (output (: true Bool)))

(case
  "ey1 a Symbol entry param keys a Map"
  (input
    (do
      (def
        (main (: s Symbol))
        (match
          (Map.lookup (Map.insert #map() (Symbol.of "hot") 42) s)
          ((Option.Some v) v)
          ((Option.None) -1)))
      (export main)))
  (call main (: #"hot" Symbol))
  (output (: 42 Int64)))

; -- breaker batch 436 (2026-08-26): CLOSURE-ENVIRONMENT reclaim — heap values captured by
; closures: invoked once, SHARED across two closures, built-but-NEVER-invoked (the discarded-env
; face), and RETURNED from a helper (the escaping-env face). All live-objects 0; wasm-only rows.
(case
  "cle1 a closure capturing a branch-selected list reclaims after invocation"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((xs (if (> n 0) #list(n (+ n 1)) #list(9))))
          (let ((f (fn (k) (+ k (List.len xs))))) (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64))
  (live-objects 0))

(case
  "cle2 TWO closures sharing one captured list both invoke and everything reclaims"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((xs (if (> n 0) #list(n) #list(9 9))))
          (let
            ((f (fn (k) (+ k (List.len xs)))))
            (let ((g (fn (k) (* k (List.len xs))))) (+ (f 10) (g 10))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 21 Int64))
  (live-objects 0))

(case
  "cle3 a heap-capturing closure built but NEVER invoked still reclaims"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((xs (if (> n 0) #list(n (+ n 1) (+ n 2)) #list(9))))
          (let ((f (fn (k) (+ k (List.len xs))))) (if (> n 100) (f 1) 7))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "cle4 a closure RETURNED from a helper (escaping env) reclaims after invocation"
  (input
    (do
      (def
        (mk (: n Int64))
        (let ((xs (if (> n 0) #list(n (+ n 1)) #list(9)))) (fn (k) (+ k (List.len xs)))))
      (def (main (: n Int64)) ((mk n) 10))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64))
  (live-objects 0))

; -- breaker batch 437 (2026-08-26): PER-FRAME leak AMPLIFICATION — 50-frame recursions that build
; and consume heap EVERY frame (list build+reduce, string concat+measure, closure construct+invoke,
; effect dispatch with a fresh heap answer). A one-cell-per-frame leak would read >=50; all read 0.
; The amplification detector complements the single-shot leak clauses. wasm-only rows.
(case
  "frl1 fifty frames each build and reduce a list — zero residue"
  (input
    (do
      (def
        (walk (: k Int64) (: acc Int64))
        (if (= k 0) acc (walk (- k 1) (+ acc (List.len (if (> k 0) #list(k (+ k 1)) #list(9)))))))
      (def (main (: n Int64)) (walk 50 n))
      (export main)))
  (call main (: 0 Int64))
  (output (: 100 Int64))
  (live-objects 0))

(case
  "frl2 fifty frames each build and measure a string — zero residue"
  (input
    (do
      (def
        (walk (: k Int64) (: acc Int64))
        (if
          (= k 0)
          acc
          (walk (- k 1) (+ acc (String.byte-len (String.concat "ab" (if (> k 25) "c" "de")))))))
      (def (main (: n Int64)) (walk 50 n))
      (export main)))
  (call main (: 0 Int64))
  (output (: 175 Int64))
  (live-objects 0))

(case
  "frl3 fifty frames each construct and invoke a heap-capturing closure — zero residue"
  (input
    (do
      (def
        (walk (: k Int64) (: acc Int64))
        (if
          (= k 0)
          acc
          (let
            ((xs (if (> k 0) #list(k) #list(9 9))))
            (let ((f (fn (j) (+ j (List.len xs))))) (walk (- k 1) (+ acc (f 0)))))))
      (def (main (: n Int64)) (walk 50 n))
      (export main)))
  (call main (: 0 Int64))
  (output (: 50 Int64))
  (live-objects 0))

(case
  "frl4 fifty dispatches each resume a fresh heap answer — zero residue"
  (input
    (do
      (effect E (op draw (-> (List Int64))))
      (def
        (walk (: k Int64) (: acc Int64))
        (if (= k 0) acc (walk (- k 1) (+ acc (List.len (E.draw))))))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((draw () s (resume (if (> s -1) #list(s (+ s 1)) #list(9)) (+ s 1))))
          (walk 50 0)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 100 Int64))
  (live-objects 0))

; -- breaker batch 438 (2026-08-26): closure captures across the OTHER heap kinds (batch 436
; covered lists) — a MAP capture queried through the closure, a SET membership closure invoked on
; hit+miss, a STRING capture measured, and a runtime AST capture matched. All live-objects 0 under
; the new default-enforcement regime (#3808); wasm-only rows.
(case
  "clk1 a closure capturing a MAP reclaims after invocation"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m (Map.insert (Map.insert #map() n 10) (+ n 1) 20)))
          (let
            ((f (fn (k) (match (Map.lookup m k) ((Option.Some v) v) ((Option.None) -1)))))
            (f (+ n 1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64))
  (live-objects 0))

(case
  "clk2 a closure capturing a SET reclaims after invocation"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((s #set(n (+ n 1) (+ n 2))))
          (let ((f (fn (k) (if (Set.contains s k) 1 0)))) (+ (f n) (f 99)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "clk3 a closure capturing a STRING reclaims after invocation"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((s (String.concat "ab" (if (> n 0) "c" "de"))))
          (let ((f (fn (k) (+ k (String.byte-len s))))) (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 13 Int64))
  (live-objects 0))

(case
  "clk4 a closure capturing a runtime AST reclaims after invocation"
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((a (Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of n))))))
          (let ((f (fn (k) (match a ((Ast.List xs) (+ k (List.len xs))) (_ -1))))) (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64))
  (live-objects 0))

; -- breaker batch 439 (2026-08-26): slice-1 INTEGRATION composites on #3813 — lifted String AND
; Bytes params feed an effect arm's answer (0-leak); the ORIGINAL ch05 face resurrected (literal
; string MATCH dispatch on a lifted param — the tick-110 'pattern decline' that was really the
; entry-param gap, now end-to-end); and the viachain if-chain shape on a lifted param. All
; live-objects 0 under default enforcement; wasm-only rows.
(case
  "icp1 lifted String AND Bytes params feed an effect arm's answer"
  (input
    (do
      (effect E (op q (-> Int64)))
      (def
        (main (: s String) (: b Bytes))
        (handle E 0 ((q () st (resume (+ (String.byte-len s) (Bytes.len b)) st))) (+ 100 (E.q))))
      (export main)))
  (call main (: "abc" String) (: #list(1 2) Bytes))
  (output (: 105 Int64))
  (live-objects 0))

(case
  "icp2 the original ch05 face RESURRECTED — literal string MATCH on a lifted param"
  (input (do (def (main (: s String)) (match s ("alpha" 1) ("beta" 2) (_ 0))) (export main)))
  (call main (: "beta" String))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "icp3 a lifted String compared and measured through an if-chain (the viachain shape on a param)"
  (input
    (do
      (def (main (: s String)) (if (= s "add") 1 (if (= s "sub") 2 (String.byte-len s))))
      (export main)))
  (call main (: "other" String))
  (output (: 5 Int64))
  (live-objects 0))

; -- breaker batch 440 (2026-08-26): the slice-1 escape-analysis BOUNDARY from the admitted side —
; a lifted String CAPTURED by a closure and a lifted String as an EFFECT-OP argument are both
; classified borrowed (admitted, wrapper-drop reclaims to 0); SLICING a lifted Bytes param declines
; conservatively (wasm-todo / rust-pass — an escaping-slice auto-flip row alongside ep1/ckr2).
(case
  "eab1 a lifted String param CAPTURED by a closure is admitted as borrowed and reclaims"
  (input
    (do
      (def (main (: s String)) (let ((f (fn (k) (+ k (String.byte-len s))))) (f 10)))
      (export main)))
  (call main (: "abcd" String))
  (output (: 14 Int64))
  (live-objects 0))

(case
  "eab2 a lifted String param as an EFFECT-OP argument is admitted and reclaims"
  (input
    (do
      (effect E (op put (-> String Int64)))
      (def
        (main (: s String))
        (handle E 0 ((put (t) st (resume (String.byte-len t) st))) (E.put s)))
      (export main)))
  (call main (: "abcd" String))
  (output (: 4 Int64))
  (live-objects 0))

(case
  "eab3 a lifted Bytes param sliced through Option.expect is admitted and reclaims"
  (input
    (do
      (def (main (: b Bytes)) (Bytes.len (Option.expect (Bytes.slice b 1 2) "in bounds")))
      (export main)))
  (call main (: #list(9 1 2 7) Bytes))
  (output (: 2 Int64))
  (live-objects 0))

; ── Reclaim: a borrowed heap-payload sum param in a self-recursive fn is dropped at the loop exit (migrated from rcdzc) ──
(case
  "a borrowed heap-payload sum param in a self-recursive fn is reclaimed at the loop exit (no leak)"
  (doc
    "A self-recursive `walk` holds a sum with a HEAP (BigInt) payload as a BORROWED param, threading
           it down the recursion and matching it only at the base case. The owned-heap-param drop epilogue
           reclaims the frame-owned param at the loop exit (a looped body's dead-at-exit, identity-carried
           heap param), so it nets ZERO live cells — it used to leak one cell (nothing reclaimed the shell;
           the base `match` only borrows it). walk 1 (mk 3): (>= n 0) recurses n=1->0->-1 then the base
           `(match w ((Mk x) (Int64.of x)))` reads 3. Value-correct (3, no UAF/double-free). Pins the
           drop-epilogue reclaim: a regression reintroducing the leak reds on live-objects (1), a NEW leak
           (>1) reds, and an over-firing drop (double-free) reds the value. The reclaim is DEPTH-INDEPENDENT
           (O(1), not per-level): the deep n=50 call nets the same ZERO — a per-recursion-level allocation
           regression would surface as a leak > 0 at depth.")
  (input
    (do
      (type W (Mk BigInt))
      (def (mk (: k Int64)) (Mk (BigInt.of k)))
      (def
        (walk (: n Int64) (: w W))
        (if (>= n 0) (walk (- n 1) w) (match w ((Mk x) (Int64.of x)))))
      (def (main (: n Int64)) (walk n (mk 3)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 50 Int64))
  (output (: 3 Int64))
  (live-objects 0))

; ── Reclaim (O(N)): a recursive linked-list fold leaks every heap node — the pervasive Perceus gap (migrated from rcdzc) ──
(case
  "a recursive linked-list fold leaks every heap node (O(N) reclaim gap)"
  (doc
    "A user recursive sum `L = Cons(Int64, L) | Nil` built to 3 elements and folded by `len` — a
           SELF-TAIL-RECURSIVE fold (the `(len t …)` tail call sits in the Cons arm). This WAS the
           pervasive O(N)-in-data Perceus-in-recursion leak witness: every matched Cons shell + its
           (Int64, L) tuple leaked (7 cells for a 3-element list), the reclaim gap the doc below once
           tracked. INC1 pt3's SELF-LOOP-TAIL shell reclaim (v-core-opt emit: a per-iteration deep
           `op_drop` on the loop-param shell; v-memory-safety dup-side: dup the moved tail child before
           the drop so the cascade nets without a double-free) now RECLAIMS the traversed spine per
           iteration: `(live-objects 0)` — the 3 Cons shells + 3 tuples are freed, and the payloadless
           `Nil` terminal is an immortal 0-heap constant (not a leaked cell), confirmed by faithful
           rctrace on landed #7399. Value-correct: `len (build 3)` = 3 (a freed-early node would
           trap/garble). The pin is now a REGRESSION GUARD for the self-loop-tail reclaim — any
           `live-objects > 0` is a pt3 reclaim regression (a re-leaked spine).")
  (input
    (do
      (type L (Cons (Tuple Int64 L)) Nil)
      (def
        (len (: xs L) (: acc Int64))
        (match xs ((L.Cons #tuple(h t)) (len t (+ acc 1))) ((L.Nil _) acc)))
      (def (build (: n Int64)) (if (< n 1) (L.Nil ()) (L.Cons #tuple(n (build (- n 1))))))
      (def (main) (len (build 3) 0))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

; ── Reclaim (no-double-free): a CONSUMED heap param on a self-recursive non-tail spine is not dropped (migrated from rcdzc) ──
(case
  "a consumed heap param on a self-recursive loop's non-tail spine is not double-freed"
  (doc
    "`sink` CONSUMES its list param (passing xs INTO the call is a consuming call-arg under
           callee-owns-args). `walk` recurses on scalar n; in the NON-tail `(+ (sink xs) (walk (- n 1) xs))`
           it BOTH consumes xs (via `sink xs`) AND identity-passes it on the recursive back-edge — so xs is
           not borrow-only, and the narrow owned-heap-param drop gate must NOT drop it at the loop exit
           (dropping it would free a handle the back-edge/caller still holds -> double-free). The Perceus
           retain dups xs for the sink consume so the recursion's xs survives. VALUE is the double-free/UAF
           guard: len=3 summed over n=2,1,0 = 9; a wrongly-dropped xs would trap or read a freed handle
           (garbled length). Value-correct + no double-free, with a residual known-leak of 2 cells (the
           dup/shell reclaim gap, flips to 0 when the general Perceus drop pass lands).")
  (input
    (do
      (def (sink (: ys (List Int64))) (List.len ys))
      (def (walk (: n Int64) (: xs (List Int64))) (if (>= n 0) (+ (sink xs) (walk (- n 1) xs)) 0))
      (def (main (: n Int64)) (walk n (List.push (List.push (List.push #list() 1) 2) 3)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 9 Int64))
  (live-objects 0))

; -- breaker batch 468 (2026-08-27): compile-robustness pins from the depth/width sweep. wep1 pins
; the SIXTEEN-param entry edge — the component-model max-flat-params boundary; 17+ currently emits
; an INVALID component (silent bad artifact, filed to v-rust-backend with the 16/17 bisection —
; the canonical ABI wants the memory-indirect convention past 16). rbw1-rbw3 pin wide/deep COMPILE
; robustness: a 300-arm integer match dispatches, a 400-binding let chain threads, and a
; 1,000,000-frame +1 recursion answers (the non-tail spine is loop-converted rather than
; exhausting the wasm stack).
(case
  "wep1 SIXTEEN scalar entry params cross the boundary and sum (the max-flat-params edge)"
  (input
    (do
      (def
        (main
          (: p0 Int64)
          (: p1 Int64)
          (: p2 Int64)
          (: p3 Int64)
          (: p4 Int64)
          (: p5 Int64)
          (: p6 Int64)
          (: p7 Int64)
          (: p8 Int64)
          (: p9 Int64)
          (: p10 Int64)
          (: p11 Int64)
          (: p12 Int64)
          (: p13 Int64)
          (: p14 Int64)
          (: p15 Int64))
        (+
          (+
            (+
              (+
                (+ (+ (+ (+ (+ (+ (+ (+ (+ (+ (+ p0 p1) p2) p3) p4) p5) p6) p7) p8) p9) p10) p11)
                p12)
              p13)
            p14)
          p15))
      (export main)))
  (call
    main
    (: 0 Int64)
    (: 1 Int64)
    (: 2 Int64)
    (: 3 Int64)
    (: 4 Int64)
    (: 5 Int64)
    (: 6 Int64)
    (: 7 Int64)
    (: 8 Int64)
    (: 9 Int64)
    (: 10 Int64)
    (: 11 Int64)
    (: 12 Int64)
    (: 13 Int64)
    (: 14 Int64)
    (: 15 Int64))
  (output (: 120 Int64)))

; -- breaker batch 470 (2026-08-27): the >16-flat-param DECLINE (#3906 fixed the silent invalid
; component) pinned from both sides. wfp1 = the 17-scalar rung (declines with the precise ABI
; message; auto-flips when the memory-indirect convention lands). wfp2 = EIGHT list params = 16
; flat values (each lifted list is ptr+len, 2 flat) — the boundary edge compiles, runs, and
; reclaims. wfp3 = NINE list params = 18 flat — declines (currently via the generic width message
; from the lift-wrapper path, not the flat-param message; diagnostic nit noted to v-rust-backend).
(case
  "wfp1 SEVENTEEN scalar entry params decline pending the memory-indirect convention"
  (input
    (do
      (def
        (main
          (: p0 Int64)
          (: p1 Int64)
          (: p2 Int64)
          (: p3 Int64)
          (: p4 Int64)
          (: p5 Int64)
          (: p6 Int64)
          (: p7 Int64)
          (: p8 Int64)
          (: p9 Int64)
          (: p10 Int64)
          (: p11 Int64)
          (: p12 Int64)
          (: p13 Int64)
          (: p14 Int64)
          (: p15 Int64)
          (: p16 Int64))
        (+
          (+
            (+
              (+
                (+
                  (+ (+ (+ (+ (+ (+ (+ (+ (+ (+ (+ p0 p1) p2) p3) p4) p5) p6) p7) p8) p9) p10) p11)
                  p12)
                p13)
              p14)
            p15)
          p16))
      (export main)))
  (call
    main
    (: 0 Int64)
    (: 1 Int64)
    (: 2 Int64)
    (: 3 Int64)
    (: 4 Int64)
    (: 5 Int64)
    (: 6 Int64)
    (: 7 Int64)
    (: 8 Int64)
    (: 9 Int64)
    (: 10 Int64)
    (: 11 Int64)
    (: 12 Int64)
    (: 13 Int64)
    (: 14 Int64)
    (: 15 Int64)
    (: 16 Int64))
  (output (: 136 Int64)))

(case
  "wfp2 EIGHT list entry params are sixteen flat values — the lifted boundary edge sums their lengths"
  (input
    (do
      (def
        (main
          (: p0 (List Int64))
          (: p1 (List Int64))
          (: p2 (List Int64))
          (: p3 (List Int64))
          (: p4 (List Int64))
          (: p5 (List Int64))
          (: p6 (List Int64))
          (: p7 (List Int64)))
        (+
          (+
            (+
              (+ (+ (+ (+ (List.len p0) (List.len p1)) (List.len p2)) (List.len p3)) (List.len p4))
              (List.len p5))
            (List.len p6))
          (List.len p7)))
      (export main)))
  (call
    main
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64)))
  (output (: 16 Int64)))

(case
  "wfp3 NINE list entry params are eighteen flat values and decline"
  (input
    (do
      (def
        (main
          (: p0 (List Int64))
          (: p1 (List Int64))
          (: p2 (List Int64))
          (: p3 (List Int64))
          (: p4 (List Int64))
          (: p5 (List Int64))
          (: p6 (List Int64))
          (: p7 (List Int64))
          (: p8 (List Int64)))
        (+
          (+
            (+
              (+
                (+
                  (+ (+ (+ (List.len p0) (List.len p1)) (List.len p2)) (List.len p3))
                  (List.len p4))
                (List.len p5))
              (List.len p6))
            (List.len p7))
          (List.len p8)))
      (export main)))
  (call
    main
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64))
    (: #list(1 2) (List Int64)))
  (output (: 18 Int64)))

; -- breaker batch 471 (2026-08-27): COMPOUND RETURNS from an export — the formerly un-checkable
; class, pinned with exact live-object clauses. A returned compound is REACHABLE at report time,
; so the reading equals the value's representation cell count (list-of-3-scalars = shell+node = 2;
; tuple/Some/String/record = 1) — these are NOT leaks and must NOT carry known-leak markers. When
; the reachability-aware live-objects driver lands (subtract reachable-from-return), these clauses
; flip to (live-objects 0) — that flip IS the driver's acceptance.
(case
  "crr1 an export returns a three-element list (reachable return = shell plus node, two cells)"
  (input (do (def (main (: n Int64)) #list(n (+ n 1) (* n 2))) (export main)))
  (call main (: 5 Int64))
  (output (: #list(5 6 10) (List Int64)))
  (live-objects 2))

(case
  "crr2 an export returns a pair tuple (one reachable cell)"
  (input (do (def (main (: n Int64)) #tuple(n (+ n 1))) (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 5 6) (Tuple Int64 Int64)))
  (live-objects 1))

(case
  "crr3 an export returns a Some-wrapped scalar (one reachable cell)"
  (input (do (def (main (: n Int64)) (if (> n 0) (Option.Some n) Option.None)) (export main)))
  (call main (: 5 Int64))
  (output (: (Some 5) (Option Int64)))
  (live-objects 1))

(case
  "crr4 an export returns a runtime-concatenated String (one reachable cell)"
  (input (do (def (main (: n Int64)) (if (> n 0) (String.concat "ab" "c") "z")) (export main)))
  (call main (: 5 Int64))
  (output (: "abc" String))
  (live-objects 1))

(case
  "crr5 an export returns a two-field record (one reachable cell)"
  (input (do (def (main (: n Int64)) #record((= x n) (= y (+ n 1)))) (export main)))
  (call main (: 5 Int64))
  (output (: (record (= x 5) (= y 6)) (Record (: x Int64) (: y Int64))))
  (live-objects 1))

; -- breaker batch 472 (2026-08-27): NESTED compound returns (extends crr1-5). The result-side
; lowering handles nesting the param side needed a recursive lift for, and the reachable-cell
; counts are COMPOSITIONAL — N = the sum of the component cells (list-of-2-tuples = 2+1+1 = 4;
; Some-of-list = 1+2 = 3; record-with-list-field = 1+2 = 3; list-of-lists = 2+1+3... measured 6;
; tuple-with-String = 1+1 = 2). Same contract as crr: NOT leaks, flip to (live-objects 0) with the
; reachability-aware driver. Note nrr3's record ascription renders in the lowercase structural
; form, unlike crr5's scalar-only record — pinned as observed.
(case
  "nrr1 an export returns a list of two tuples (four reachable cells, compositional)"
  (input (do (def (main (: n Int64)) #list(#tuple(n 1) #tuple((+ n 1) 2))) (export main)))
  (call main (: 5 Int64))
  (output (: #list(#tuple(5 1) #tuple(6 2)) (List (Tuple Int64 Int64))))
  (live-objects 4))

(case
  "nrr2 an export returns a Some-wrapped list (three reachable cells)"
  (input
    (do
      (def (main (: n Int64)) (if (> n 0) (Option.Some #list(n (+ n 1))) Option.None))
      (export main)))
  (call main (: 5 Int64))
  (output (: (Some #list(5 6)) (Option (List Int64))))
  (live-objects 3))

(case
  "nrr3 an export returns a record with a list field (three reachable cells)"
  (input (do (def (main (: n Int64)) #record((= k n) (= xs #list(n (+ n 1))))) (export main)))
  (call main (: 5 Int64))
  (output (: #record((= k 5) (= xs #list(5 6))) (record (k Int64) (xs (List Int64)))))
  (live-objects 3))

(case
  "nrr4 an export returns a list of lists (six reachable cells)"
  (input (do (def (main (: n Int64)) #list(#list(n) #list(n (+ n 1)))) (export main)))
  (call main (: 5 Int64))
  (output (: #list(#list(5) #list(5 6)) (List (List Int64))))
  (live-objects 6))

(case
  "nrr5 an export returns a tuple carrying a runtime String (two reachable cells)"
  (input
    (do (def (main (: n Int64)) #tuple(n (if (> n 0) (String.concat "ab" "c") "z"))) (export main)))
  (call main (: 5 Int64))
  (output (: #tuple(5 "abc") (Tuple Int64 String)))
  (live-objects 2))

; The O(n) shell-ACCUMULATION known gap (more severe than the O(1) borrowed-param twin above): a
; self-recursive `f` returns an OWNED sum `(Mk list)`, and every returning frame `(match (f …) ((Mk t)
; (Mk t)))` destructures the owned recursive-call result and RE-WRAPS its payload into a FRESH `(Mk t)`.
; The incoming shell (the callee's owned return) is DEAD after `t` is extracted, but no drop is inserted,
; so each frame leaks its shell — the leak GROWS WITH RECURSION DEPTH (the distinguishing property). Root:
; no drop of an OWNED match scrutinee's sum shell after its payload is extracted (the general Perceus
; match/sum-shell-drop pass). VALUE-CORRECT (2) at every depth — a pure leak, no UAF. The two cases below
; (2 frames vs 6 frames) pin the depth-monotone leak; flip both to 0 when the sum-shell-drop pass lands.
(case
  "a recursive re-wrap of a matched owned sum child accumulates shells -- known gap, SHALLOW (2 frames)"
  (doc
    "`f` recurses up to the base (n==0), builds one owned `(Mk (bl 0 2 (list)))`, then every returning
           frame `(match (f (+ n 1)) ((Mk t) (Mk t)))` extracts the owned child and re-wraps it in a fresh
           shell, leaking the dead incoming shell. `main(-2)` = 2 frames above the base; `(List.len t)` = 2
           (value-correct, no UAF). The leak is the un-dropped per-frame shells; see the DEEP twin for the
           depth-monotone growth that distinguishes this O(n) gap from the O(1) borrowed-param leak.")
  (input
    (do
      (type Box (Mk (List Int64)))
      (def
        (bl (: i Int64) (: n Int64) (: a (List Int64)))
        (if (< i n) (bl (+ i 1) n (List.push a i)) a))
      (def (f (: n Int64)) (if (= n 0) (Mk (bl 0 2 #list())) (match (f (+ n 1)) ((Mk t) (Mk t)))))
      (def (main (: n Int64)) (match (f n) ((Mk t) (List.len t))))
      (export main)))
  (call main (: -2 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "a recursive re-wrap of a matched owned sum child accumulates shells -- known gap, DEEP (6 frames, leaks strictly more)"
  (doc
    "The SAME program as the shallow twin, driven to 6 re-wrap frames (`main(-6)`) instead of 2. Value
           is still 2 (depth-independent, no UAF), but the leaked-shell count is STRICTLY GREATER than the
           shallow case -- the O(n) accumulation that distinguishes this gap from the O(1) borrowed-param
           leak. Deeper recursion re-wraps more owned shells, each left un-dropped after extraction.")
  (input
    (do
      (type Box (Mk (List Int64)))
      (def
        (bl (: i Int64) (: n Int64) (: a (List Int64)))
        (if (< i n) (bl (+ i 1) n (List.push a i)) a))
      (def (f (: n Int64)) (if (= n 0) (Mk (bl 0 2 #list())) (match (f (+ n 1)) ((Mk t) (Mk t)))))
      (def (main (: n Int64)) (match (f n) ((Mk t) (List.len t))))
      (export main)))
  (call main (: -6 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

; -- breaker batch 473 (2026-08-27): the heap-return × lifted-param COMPOSITION gate (found probing
; param→return flow). Lifted list param + scalar return works (el2); scalar param + heap return
; works (crr/nrr); their COMBINATION declines with its own precise message ("a parameterized
; heap-return export forwards scalar params and fixed-shape scalar tuple/record params only").
; phr2 is the over-conservative ADMISSION edge: a FRESH compound return with the param merely
; measured (no flow) still declines — the gate keys on param-type × return-kind, not actual flow.
; phr3 adds extract-into-return (real flow, payload copied). phr1 is the true-escape FENCE: an
; identity return is ownership TRANSFER to the harness — the wrapper must not drop what it
; returns, so this rung stays declined until transfer semantics exist; a flip without them would
; be a use-after-free in the harness read.
(case
  "phr1 an export returning its lifted List param verbatim declines (ownership-transfer fence)"
  (input (do (def (main (: xs (List Int64))) xs) (export main)))
  (call main (: #list(4 5 6) (List Int64)))
  (output (: (list 4 5 6) (List Int64))))

(case
  "phr2 an export with a lifted List param returning a FRESH list declines (the no-flow admission edge)"
  (input (do (def (main (: xs (List Int64))) #list((List.len xs) 7)) (export main)))
  (call main (: #list(4 5 6) (List Int64)))
  (output (: (list 3 7) (List Int64))))

(case
  "phr3 an export extracting a param element into a fresh returned list declines (copied flow)"
  (input
    (do
      (def
        (main (: xs (List Int64)))
        (match (List.at xs 1) ((Option.Some v) #list(v v)) ((Option.None) #list(-1))))
      (export main)))
  (call main (: #list(4 5 6) (List Int64)))
  (output (: (list 5 5) (List Int64))))

; -- breaker batch 474 (2026-08-27): SHARING through the return boundary, discriminated by the
; census (extends the crr/nrr calibration corpus; drift-guards the sharing-aware emit). shr1 and
; shr3 RENDER identically but differ in reachable cells: a let-bound inner list referenced twice
; stays ONE object (4 = outer 2 + inner 2 once), while two separately-built identical lists are
; distinct (6). A future emit change that breaks sharing flips shr1/shr2 upward — a correlated
; census delta with no value change, exactly what these pins exist to catch.
(case
  "shr1 a let-bound inner list referenced twice in a returned list stays one shared object (four cells)"
  (input (do (def (main (: n Int64)) (let ((ys #list(n (+ n 1)))) #list(ys ys))) (export main)))
  (call main (: 5 Int64))
  (output (: #list(#list(5 6) #list(5 6)) (List (List Int64))))
  (live-objects 4))

(case
  "shr2 a let-bound inner list referenced twice in a returned tuple stays one shared object (three cells)"
  (input (do (def (main (: n Int64)) (let ((ys #list(n (+ n 1)))) #tuple(ys ys))) (export main)))
  (call main (: 5 Int64))
  (output (: #tuple(#list(5 6) #list(5 6)) (Tuple (List Int64) (List Int64))))
  (live-objects 3))

(case
  "shr3 two separately-built identical inner lists are distinct objects (six cells, the unshared control)"
  (input (do (def (main (: n Int64)) #list(#list(n (+ n 1)) #list(n (+ n 1)))) (export main)))
  (call main (: 5 Int64))
  (output (: #list(#list(5 6) #list(5 6)) (List (List Int64))))
  (live-objects 6))

; SIMPLEST instance of the compound-shell reclaim gap (the before/after witness for the future
; broadening): a SINGLE non-recursive match over an OWNED compound-payload sum whose payload child is
; only BORROWED (read by List.len -> scalar, never moved out) and does NOT escape the arm, STILL leaks
; the shell + payload — the MatchSum reclaim gates require all-scalar payloads, and a (List Int64)
; payload fails that floor, so the owned Box.Wrap shell is left un-dropped. Value-correct, no UAF. The
; scrutinee (mk n) takes a RUNTIME n so the match can't const-fold away (a constant would eliminate the
; MatchSum and mask the leak). Flip to 0 when the sound no-arm-child-escapes compound-shell reclaim lands.
(case
  "a borrow-only compound-payload match shell is left un-dropped -- known gap (all-scalar reclaim floor)"
  (doc
    "`(type Box (Wrap (List Int64)) Empty)`; `mk n` builds a fresh owned `Box.Wrap [0..n)`; `main n`
           matches it, binds `xs`, and reads `(List.len xs)` — a BORROW to a scalar, `xs` never escapes the
           arm. `mk 3` -> `Wrap [0,1,2]`, len = 3 (value-correct, no UAF). But the owned Box.Wrap shell +
           its payload list are left un-dropped: the MatchSum shell-reclaim gate requires all-scalar
           payloads and a `(List Int64)` payload fails it, so 3 cells stay live (shell + list spine + boxed
           element) even though the borrowed non-escaping child would make the drop sound. Flip to 0 when
           the no-arm-child-escapes compound-shell reclaim broadening lands — this is the first shape it fixes.")
  (input
    (do
      (type Box (Wrap (List Int64)) Empty)
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
      (def (mk (: n Int64)) (if (< n 0) (Box.Empty ()) (Box.Wrap (build 0 n #list()))))
      (def (main (: n Int64)) (match (mk n) ((Box.Wrap xs) (List.len xs)) ((Box.Empty _) 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (live-objects 0))

; UAF GUARD (minimized from a real cad snowflake miscompile a rejected shell-reclaim shipped): an owned
; COMPOUND-payload sum whose child is a COMPOUND (a List) BORROWED OUT of the shell via a match binder,
; then read. The extracted `hi` ALIASES INTO the Box.Bx shell (a sum-payload borrow, not a copy), so a
; reclaim that frees the shell after the outer match while `hi` is still read would be a USE-AFTER-FREE.
; VALUE CORRECTNESS is the guard: correct len means no early free. The all-scalar-payload reclaim floor
; keeps it safe today (the outer Box has a compound List payload -> reclaim=false), leaking the shell +
; children -- a future compound-shell reclaim broadening that is unsound for this alias-out shape would
; fail HERE (value must stay correct; a scalar-result heuristic that ignored the alias shipped the cad UAF).
(case
  "a compound child borrowed out of a sum shell via a match binder is not use-after-freed -- UAF guard + known leak"
  (doc
    "`(type Box (Bx (List Int64) (List Int64)) Empty)`; `mk n` builds a fresh owned `Box.Bx` of two
           runtime lists; `main n` matches it, binds the compound child `hi` (a List ALIASING into the Box
           shell -- a sum-payload borrow, NOT a copy) and reads `(List.len hi)`. `mk 3` -> hi = [0,1,2,3,4],
           len 5. Value 5 is the UAF guard: an over-eager reclaim freeing the shell while `hi` aliases into
           it would trap OOB or read garbage. The owned Box.Bx shell + its two List children are left
           un-dropped (compound-payload reclaim floor) -- a known leak; flip to 0 only under a reclaim
           broadening that stays SOUND for this alias-out shape (value must remain 5).")
  (input
    (do
      (type Box (Bx (List Int64) (List Int64)) Empty)
      (def
        (bl (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (< i n) (bl (+ i 1) n (List.push acc i)) acc))
      (def
        (mk (: n Int64))
        (if (< n 0) (Box.Empty ()) (Box.Bx (bl 0 n #list()) (bl 0 (+ n 2) #list()))))
      (def (main (: n Int64)) (match (mk n) ((Box.Bx lo hi) (List.len hi)) ((Box.Empty _) 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 5 Int64))
  (live-objects 0))

; -- breaker batch 476 (2026-08-27): the return-side TYPE matrix completed (extends crr/nrr).
; Collection and value-form returns from a scalar-param export: Set (its constructor-application
; encode form), Map, BigInt, Rational all cross and reclaim to their reachable cells. Symbol is
; the ODD ONE OUT: a parameterized export cannot yet return it (the value form is emitted only
; for a nullary constant export; a runtime value-encode render needs a Shape::Sym, routed to
; v-runtime). The original guard FALSELY blamed the scalar param — fixed in #3932 to a truthful
; result-naming message ("the scalar parameters are fine"); trm5 stays the decline rung and flips
; when the runtime Sym render lands. The no-param constant Symbol works (deforested).
(case
  "trm1 an export returns a two-element Set (one reachable cell, constructor-application render)"
  (input (do (def (main (: n Int64)) #set(n (+ n 1))) (export main)))
  (call main (: 5 Int64))
  (output (: #set(5 6) (Set Int64)))
  (live-objects 1))

(case
  "trm2 an export returns a one-entry Map (one reachable cell)"
  (input (do (def (main (: n Int64)) (Map.insert Map.empty n (+ n 1))) (export main)))
  (call main (: 5 Int64))
  (output (: #map((= 5 6)) (Map Int64 Int64)))
  (live-objects 1))

(case
  "trm3 an export returns a BigInt built from the scalar param (one reachable cell)"
  (input (do (def (main (: n Int64)) (BigInt.of n)) (export main)))
  (call main (: 5 Int64))
  (output (: 5 BigInt))
  (live-objects 1))

(case
  "trm4 an export returns an exact Rational from the scalar param (three reachable cells)"
  (input (do (def (main (: n Int64)) (Rational.of n 2)) (export main)))
  (call main (: 5 Int64))
  (output (: 5/2 Rational))
  (live-objects 3))

(case
  "trm5 a parameterized export returning a Symbol declines truthfully pending the runtime Sym render"
  (input (do (def (main (: n Int64)) #"hot") (export main)))
  (call main (: 5 Int64))
  (output (: (Symbol.of "hot") Symbol)))

; -- breaker batch 477 (2026-08-27): the Option-entry slice (#3923) flip-verified + its edges.
; eo1-3 rows flipped todo->pass. eop1 pins the Option-param x effects-fold composition (0-leak).
; The landing note claimed option AND result params lift, but a Result param still DECLINES
; ("crosses the host boundary only as a single nullary export's result" — honest message, scope
; gap reported): erp1 pins that rung. eop2 pins the Option-of-COMPOUND payload rung (behind the
; nested lift). Both wasm-todo / rust-pass.
(case
  "eop1 an Option entry param read inside a handle body composes with the fold"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: o (Option Int64)))
        (handle
          St
          10
          ((get (u) s (resume s s)))
          (+ (St.get) (match o ((Option.Some v) v) ((Option.None) 0)))))
      (export main)))
  (call main (: (Some 7) (Option Int64)))
  (output (: 17 Int64)))

(case
  "erp1 a Result entry param delivers its Ok payload"
  (input
    (do
      (def (main (: r (Result Int64 String))) (match r ((Ok v) (* v 2)) ((Err _) -1)))
      (export main)))
  (call main (: (Ok 21) (Result Int64 String)))
  (output (: 42 Int64)))

(case
  "eop2 an Option-of-list entry param measures its Some payload"
  (input
    (do
      (def
        (main (: o (Option (List Int64))))
        (match o ((Option.Some xs) (List.len xs)) ((Option.None) -1)))
      (export main)))
  (call main (: (Some #list(4 5 6)) (Option (List Int64))))
  (output (: 3 Int64)))

; -- breaker batch 478 (2026-08-27): Option-param COMPOSITIONS (the elc/grx lenses applied to the
; #3923 lift). All three ADMITTED and 0-leak — including the cross-def relay (eoc2), which the
; borrowed LIST lift declines: the Option lift REBUILDS an owned guest sum, so passing it to
; another def is free. (CORRECTED batch 485: a borrowed list RELAYS fine too when the callee only
; BORROWS — lbr1; the escape-gate is USE-KIND-based (consume/recursive-reach decline, borrows pass),
; not call-boundary-based. The asymmetry vs lists is about consuming uses only.) eoc3 confirms the repeat-unwrap family needs a HEAP payload — a
; twice-matched boundary Option with a scalar payload reclaims.
(case
  "eoc1 an Option entry param captured by a local closure applied twice"
  (input
    (do
      (def
        (main (: o (Option Int64)))
        (let
          ((f (fn ((: k Int64)) (+ k (match o ((Option.Some v) v) ((Option.None) 0))))))
          (+ (f 10) (f 100))))
      (export main)))
  (call main (: (Some 7) (Option Int64)))
  (output (: 124 Int64)))

(case
  "eoc2 an Option entry param relayed to a helper def and unwrapped there (owned sum, unlike the borrowed list)"
  (input
    (do
      (def (unwrap (: p (Option Int64))) (match p ((Option.Some v) v) ((Option.None) -1)))
      (def (main (: o (Option Int64))) (* 2 (unwrap o)))
      (export main)))
  (call main (: (Some 7) (Option Int64)))
  (output (: 14 Int64)))

(case
  "eoc3 an Option entry param matched twice reclaims (the repeat-unwrap family needs a heap payload)"
  (input
    (do
      (def
        (main (: o (Option Int64)))
        (+
          (match o ((Option.Some v) v) ((Option.None) -1))
          (* 10 (match o ((Option.Some v) v) ((Option.None) -1)))))
      (export main)))
  (call main (: (Some 7) (Option Int64)))
  (output (: 77 Int64)))

; -- v-rust-backend (2026-08-27): the #3923 Option-lift INTERLEAVE invariants (from a post-land safety
; sweep, 0 miscompiles found). eoi1 pins the flattened-leaf CURSOR threading across TWO consecutive sum
; params (each option flattens to `(disc, payload)`; the wrapper must advance the cursor by the full
; flattened width of the first before reading the second). eoi2 pins a MEM-LEAF param (String → (ptr,len))
; and a SUM param COMPOSING in one signature — the wrapper lifts the string leaf THEN reads the option's
; disc/payload at the advanced cursor, the two lift mechanisms interleaving without clobbering each other's
; leaf indices. Both are v-rust-backend's own emit invariants (distinct from the breaker's eoc/eop
; composition rows); they guard the cursor arithmetic a future param-shape change could silently break.
(case
  "eoi1 TWO Option entry params thread the flattened-leaf cursor independently"
  (input
    (do
      (def
        (main (: a (Option Int64)) (: b (Option Int64)))
        (+
          (match a ((Option.Some v) v) ((Option.None) 0))
          (match b ((Option.Some v) v) ((Option.None) 0))))
      (export main)))
  (call main (: (Some 10) (Option Int64)) (: (Some 5) (Option Int64)))
  (output (: 15 Int64)))

(case
  "eoi2 a String (mem-leaf) param and an Option (sum) param interleave in one signature"
  (input
    (do
      (def
        (main (: s String) (: o (Option Int64)))
        (+ (String.byte-len s) (match o ((Option.Some v) v) ((Option.None) 0))))
      (export main)))
  (call main (: "abc" String) (: (Some 10) (Option Int64)))
  (output (: 13 Int64)))

; eoi3 pins the NARROW-int Option payload: an `Option Int8` Some payload boxes an i32-slot value the sum
; lift must i32->i64 EXTEND before `box-int` (the `SumArmPayload::Scalar{extend}` path; eo1-3 used full-width
; Int64, which never exercises the extend). v-rust-backend confirmed (with the breaker) this emits+runs
; correctly — the earlier apparent failure was only the cdz-run ARG coerce of the ASCRIBED `(Some (: 5 Int8))`
; spelling; the BARE `(Some 5)` (the arg type already fixes the width) coerces and runs. Guards the extend path.
(case
  "eoi3 an Option<Int8> entry param's narrow Some payload widens correctly"
  (input
    (do
      (def (main (: o (Option Int8))) (match o ((Option.Some v) v) ((Option.None) (: -1 Int8))))
      (export main)))
  (call main (: (Some 5) (Option Int8)))
  (output (: 5 Int8)))

; -- breaker batch 482 (2026-08-27): sharing through DEF-CALL bindings — the pure-allocation
; contrast to the cp4 inline-duplication (a PERFORMING nullary call inlined per use loses its
; one-draw sharing; filed to v-inference). A def-call whose body merely ALLOCATES binds once:
; both cells read 3 (tuple 1 + list 2 shared once), same as the inline-let shr2 baseline — so the
; per-use inline engages only for performing bodies (or under handle reduction), which narrows
; the inliner fix's scope. These controls must stay 3 through that fix.
(case
  "idc1 a unary def-call's allocated list bound once and referenced twice stays one shared object"
  (input
    (do
      (def (mk2 (: n Int64)) #list(n (+ n 1)))
      (def (main (: n Int64)) (let ((x (mk2 n))) #tuple(x x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: #tuple(#list(5 6) #list(5 6)) (Tuple (List Int64) (List Int64))))
  (live-objects 3))

(case
  "idc2 a NULLARY def-call's allocated list bound once and referenced twice stays one shared object"
  (input
    (do (def (mk0) #list(7 8)) (def (main (: n Int64)) (let ((x (mk0))) #tuple(x x))) (export main)))
  (call main (: 5 Int64))
  (output (: #tuple(#list(7 8) #list(7 8)) (Tuple (List Int64) (List Int64))))
  ; WIT static encoding: the collection-return assembler now hoists the shared constant list build-once
  ; (census-excluded immortal), so only the outer tuple is a mortal per-eval allocation: 3→1.
  (live-objects 1))

; -- breaker batch 483 (2026-08-27): the result<scalar,scalar> param rung — the cell my original
; Result probe MISSED (rp1 used a String error payload, which declined; the scalar-scalar shape
; was ADMITTED by #3923 and emitted an INVALID component until #3937 made it decline honestly —
; the same silent-bad-artifact class as the 17-param bug). Pinned so the eventual Result slice
; flips BOTH payload shapes and any regression to invalid-wasm hits a row. Coverage lesson: probe
; a type family's payload MATRIX, not one representative.
(case
  "erp2 a Result entry param with two scalar payloads declines pending the Result lift"
  (input
    (do
      (def (main (: r (Result Int64 Int64))) (match r ((Ok v) (* v 2)) ((Err e) (- 0 e))))
      (export main)))
  (call main (: (Ok 21) (Result Int64 Int64)))
  (output (: 42 Int64)))

; -- breaker batch 485 (2026-08-27): String-param compositions (the eoc lenses on slice-1's String
; lift) + the borrow-relay cell that CORRECTS the eoc2 comment. All pass 0-leak: capture (ssc1),
; cross-def borrow relay (ssc2 — and lbr1 proves the same for LISTS: the escape-gate keys on the
; USE (consume/recursive-reach decline; borrows pass), not the call boundary), and the effects-fold
; read (ssc3).
(case
  "ssc1 a String entry param captured by a local closure applied twice"
  (input
    (do
      (def
        (main (: s String))
        (let ((f (fn ((: k Int64)) (+ k (String.byte-len s))))) (+ (f 10) (f 100))))
      (export main)))
  (call main (: "hello" String))
  (output (: 120 Int64)))

(case
  "ssc2 a String entry param relayed to a helper def that borrows its byte-length"
  (input
    (do
      (def (measure (: t String)) (String.byte-len t))
      (def (main (: s String)) (* 2 (measure s)))
      (export main)))
  (call main (: "hello" String))
  (output (: 10 Int64)))

(case
  "ssc3 a String entry param read inside a handle body alongside a dispatch"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: s String))
        (handle St 10 ((get (u) st (resume st st))) (+ (St.get) (String.byte-len s))))
      (export main)))
  (call main (: "hello" String))
  (output (: 15 Int64)))

(case
  "lbr1 a List entry param relayed to a helper def that only borrows its length"
  (input
    (do
      (def (measure (: t (List Int64))) (List.len t))
      (def (main (: xs (List Int64))) (* 2 (measure xs)))
      (export main)))
  (call main (: #list(4 5 6) (List Int64)))
  (output (: 6 Int64)))

; -- breaker batch 486 (2026-08-27): the scalar entry-param matrix completed. Bool, UInt64, and
; the narrow widths (Int8/Int16, sign-extending correctly) all cross the boundary; Char declines
; with an honest precise message (names the type, lists what crosses, notes an annotation cannot
; fix it) — the Char family's param-side rung, joining the char-walk/ckr1 runtime declines.
(case
  "spx1 a Bool entry param selects a branch"
  (input (do (def (main (: b Bool)) (if b 7 3)) (export main)))
  (call main (: true Bool))
  (output (: 7 Int64)))

(case
  "spx2 a Char entry param declines pending a boundary representation"
  (input (do (def (main (: c Char)) (Char.to-int c)) (export main)))
  (call main (: #\a Char))
  (output (: 97 Int64)))

(case
  "spx3 a UInt64 entry param increments in its own width"
  (input (do (def (main (: u UInt64)) (: (+ u 1) UInt64)) (export main)))
  (call main (: 5 UInt64))
  (output (: 6 UInt64)))

(case
  "spx4 Int8 and Int16 entry params sign-extend and sum"
  (input (do (def (main (: a Int8) (: b Int16)) (+ (Int64.of a) (Int64.of b))) (export main)))
  (call main (: -5 Int8) (: 300 Int16))
  (output (: 295 Int64)))

; SITE-A CallClosure owned-temp env-cell reclaim: an owned partial-ctor closure held in a compound and
; APPLIED in-guest. The CallClosure emit drops the owned-temp env cell after the borrowing call, so the
; closure-specific cells (env + boxed captures) reclaim; the residual is the general tuple/match-shell gap.
; Value-correct; runtime `mk n` keeps the producer from const-folding. Flip to 0 when the shell-reclaim lands.
(case
  "a SITE-A eta-closure held in a tuple is applied and its owned env cell reclaims (residual = shell gap)"
  (doc
    "`mk n` returns `(tuple (T.Mk 10) n)` (a partial ctor `(T.Mk 10)` awaiting one arg, tuple-stored so
           it isn't beta-reduced); `((. p 0) 5)` applies it -> `(T.Mk 10 5)`, matched to `(+ a b)` = 15. The
           CallClosure drops the owned env cell + boxed capture after the borrowing call; the remaining 2 are
           the general tuple + match-shell reclaim gap (shared with a no-closure control), not closure-specific.")
  (input
    (do
      (type T (Mk Int64 Int64))
      (def (mk (: n Int64)) (if (< n 0) #tuple((T.Mk 0) 0) #tuple((T.Mk 10) n)))
      (def (main) (let ((p (mk 1))) (match ((. p 0) 5) ((T.Mk a b) (+ a b)))))
      (export main)))
  (call main)
  (output (: 15 Int64))
  (live-objects known-leak))

(case
  "a SITE-A eta-closure stored in a SUM is applied and its owned env cell reclaims (residual = shell gap)"
  (doc
    "`mk n` returns `(Box.B (T.Mk 10))` behind a runtime if; the boxed partial ctor is extracted by the
           `Box.B` match and applied `(f 5)` -> `(T.Mk 10 5)` -> 15. The CallClosure operand joins to Owned so
           its cell is dropped; residual 2 = the general sum/tuple-shell gap.")
  (input
    (do
      (type T (Mk Int64 Int64))
      (type Box (B (-> Int64 T)))
      (def (mk (: n Int64)) (if (< n 0) (Box.B (T.Mk 0)) (Box.B (T.Mk 10))))
      (def (main) (let ((b (mk 1))) (match (match b ((Box.B f) (f 5))) ((T.Mk a c) (+ a c)))))
      (export main)))
  (call main)
  (output (: 15 Int64))
  (live-objects known-leak))

(case
  "a SITE-A two-arg-payload eta-closure is applied and its owned env cell reclaims (wider capture)"
  (doc
    "`(T.Mk 10 20)` awaits its 3rd arg (a WIDER two-slot capture than the single-arg shape); `((. p 0) 5)`
           -> `(T.Mk 10 20 5)`, summed = 35. Confirms the CallClosure owned-env drop is arity-agnostic over the
           boxed captures; residual 3 = the general tuple/match-shell gap (one more shell than the single-arg).")
  (input
    (do
      (type T (Mk Int64 Int64 Int64))
      (def (mk (: n Int64)) (if (< n 0) #tuple((T.Mk 0 0)) #tuple((T.Mk 10 20))))
      (def (main) (let ((p (mk 1))) (match ((. p 0) 5) ((T.Mk a b c) (+ a (+ b c))))))
      (export main)))
  (call main)
  (output (: 35 Int64))
  (live-objects known-leak))

; -- breaker batch 488 (2026-08-27): BYTES entry params — the list<u8> boundary shape. A
; borrow-only lift exists: Bytes.len over the param works and reclaims (byp1; the harness arg
; spelling is the list-of-bytes literal). Deeper uses decline with the generic width message:
; a runtime bin-match destructure of the param (byp2) and a slice extraction (byp3) — rungs for
; whichever slice extends the Bytes lift past the length borrow. Both rust-pass.
(case
  "byp1 a Bytes entry param measured by Bytes.len reclaims"
  (input (do (def (main (: b Bytes)) (Bytes.len b)) (export main)))
  (call main (: #list(104 105) Bytes))
  (output (: 2 Int64)))

(case
  "byp2 a Bytes entry param destructured by a runtime bin match declines"
  (input
    (do
      (def
        (main (: b Bytes))
        (match b ((bin (u8 x) (u8 y)) (+ (* 100 (Int64.of x)) (Int64.of y))) (_ -1)))
      (export main)))
  (call main (: #list(7 9) Bytes))
  (output (: 709 Int64)))

(case
  "byp3 a Bytes entry param sliced declines (extraction past the length borrow)"
  (input
    (do
      (def
        (main (: b Bytes))
        (match (Bytes.slice b 1 2) ((Option.Some s) (Bytes.len s)) ((Option.None) -1)))
      (export main)))
  (call main (: #list(5 6 7 8) Bytes))
  (output (: 2 Int64)))

; -- breaker batch 496→499 (2026-08-27): RECORD/TUPLE entry params — all decline (rungs standing).
; The original diagnostic was DOUBLY wrong (result-phrased for a param; "multi-export" for one
; export — the shared export_result_valtype error surfaced in the param loop); FIXED in #4031 to
; the truthful param message ("parameter … has no scalar boundary representation — a non-scalar
; entry parameter is not yet emitted on this export path"). Admission is a deferred feature —
; v-rust-backend's admit attempt itself emitted an INVALID component (the envelope does not
; flatten a record param), vindicating the decline. Rungs: scalar record (rpp1), an open-row
; helper over the boundary record (rpp2), record with a heap field (rpp3), scalar tuple (rpp4);
; all rust-pass, auto-flip when the flatten slice lands.
(case
  "rpp1 a scalar-fielded Record entry param projects both fields"
  (input (do (def (main (: r (Record (: x Int64) (: y Int64)))) (+ (* 100 r.x) r.y)) (export main)))
  (call main (: #record((= x 5) (= y 7)) (Record (: x Int64) (: y Int64))))
  (output (: 507 Int64)))

(case
  "rpp2 an open-row helper projects a field of the boundary Record param"
  (input
    (do
      (def (get-x r) r.x)
      (def (main (: r (Record (: x Int64) (: y Int64)))) (* 2 (get-x r)))
      (export main)))
  (call main (: #record((= x 5) (= y 7)) (Record (: x Int64) (: y Int64))))
  (output (: 10 Int64)))

(case
  "rpp3 a Record entry param with a heap list field measures both"
  (input
    (do
      (def (main (: r (Record (: k Int64) (: xs (List Int64))))) (+ r.k (List.len r.xs)))
      (export main)))
  (call main (: #record((= k 5) (= xs #list(1 2 3))) (Record (: k Int64) (: xs (List Int64)))))
  (output (: 8 Int64)))

(case
  "rpp4 a scalar Tuple entry param projects both positions"
  (input (do (def (main (: t (Tuple Int64 Int64))) (+ (* 100 (. t 0)) (. t 1))) (export main)))
  (call main (: #tuple(5 7) (Tuple Int64 Int64)))
  (output (: 507 Int64)))

(case
  "a tuple-destructuring lambda parameter binds like a def param"
  (doc
    "An irrefutable tuple pattern as a `fn` parameter binds x,y: (fn ((tuple x y)) (+ (* x 10) y))
           applied to (tuple 3 4) = 34.")
  (input
    (do (def (main) (let ((f (fn (#tuple(x y)) (+ (* x 10) y)))) (f #tuple(3 4)))) (export main)))
  (output (: 34 Int64)))

(case
  "a deep 50-argument curried application spine reduces to the right value"
  (doc
    "A 50-arg curried spine ((((f 0) 1) 2)…49) sums 0+1+…+49 = 1225 (regression: declined CDZ0999
           at N~32 before the spine reduction was flattened).")
  (input
    (do
      (def
        (f
          (: p0 Int64)
          (: p1 Int64)
          (: p2 Int64)
          (: p3 Int64)
          (: p4 Int64)
          (: p5 Int64)
          (: p6 Int64)
          (: p7 Int64)
          (: p8 Int64)
          (: p9 Int64)
          (: p10 Int64)
          (: p11 Int64)
          (: p12 Int64)
          (: p13 Int64)
          (: p14 Int64)
          (: p15 Int64)
          (: p16 Int64)
          (: p17 Int64)
          (: p18 Int64)
          (: p19 Int64)
          (: p20 Int64)
          (: p21 Int64)
          (: p22 Int64)
          (: p23 Int64)
          (: p24 Int64)
          (: p25 Int64)
          (: p26 Int64)
          (: p27 Int64)
          (: p28 Int64)
          (: p29 Int64)
          (: p30 Int64)
          (: p31 Int64)
          (: p32 Int64)
          (: p33 Int64)
          (: p34 Int64)
          (: p35 Int64)
          (: p36 Int64)
          (: p37 Int64)
          (: p38 Int64)
          (: p39 Int64)
          (: p40 Int64)
          (: p41 Int64)
          (: p42 Int64)
          (: p43 Int64)
          (: p44 Int64)
          (: p45 Int64)
          (: p46 Int64)
          (: p47 Int64)
          (: p48 Int64)
          (: p49 Int64))
        (+
          p0
          (+
            p1
            (+
              p2
              (+
                p3
                (+
                  p4
                  (+
                    p5
                    (+
                      p6
                      (+
                        p7
                        (+
                          p8
                          (+
                            p9
                            (+
                              p10
                              (+
                                p11
                                (+
                                  p12
                                  (+
                                    p13
                                    (+
                                      p14
                                      (+
                                        p15
                                        (+
                                          p16
                                          (+
                                            p17
                                            (+
                                              p18
                                              (+
                                                p19
                                                (+
                                                  p20
                                                  (+
                                                    p21
                                                    (+
                                                      p22
                                                      (+
                                                        p23
                                                        (+
                                                          p24
                                                          (+
                                                            p25
                                                            (+
                                                              p26
                                                              (+
                                                                p27
                                                                (+
                                                                  p28
                                                                  (+
                                                                    p29
                                                                    (+
                                                                      p30
                                                                      (+
                                                                        p31
                                                                        (+
                                                                          p32
                                                                          (+
                                                                            p33
                                                                            (+
                                                                              p34
                                                                              (+
                                                                                p35
                                                                                (+
                                                                                  p36
                                                                                  (+
                                                                                    p37
                                                                                    (+
                                                                                      p38
                                                                                      (+
                                                                                        p39
                                                                                        (+
                                                                                          p40
                                                                                          (+
                                                                                            p41
                                                                                            (+
                                                                                              p42
                                                                                              (+
                                                                                                p43
                                                                                                (+
                                                                                                  p44
                                                                                                  (+
                                                                                                    p45
                                                                                                    (+
                                                                                                      p46
                                                                                                      (+
                                                                                                        p47
                                                                                                        (+
                                                                                                          p48
                                                                                                          p49))))))))))))))))))))))))))))))))))))))))))))))))))
      (def
        (main)
        ((((((((((((((((((((((((((((((((((((((((((((((((((f 0) 1) 2) 3) 4) 5) 6) 7) 8) 9) 10) 11)
                                                                                    12)
                                                                                  13)
                                                                                14)
                                                                              15)
                                                                            16)
                                                                          17)
                                                                        18)
                                                                      19)
                                                                    20)
                                                                  21)
                                                                22)
                                                              23)
                                                            24)
                                                          25)
                                                        26)
                                                      27)
                                                    28)
                                                  29)
                                                30)
                                              31)
                                            32)
                                          33)
                                        34)
                                      35)
                                    36)
                                  37)
                                38)
                              39)
                            40)
                          41)
                        42)
                      43)
                    44)
                  45)
                46)
              47)
            48)
          49))
      (export main)))
  (output (: 1225 Int64)))

; -- const-closure specialization regressions (behavioral halves migrated from rcdzc 2026-08-27; the
; white-box "must compile / coded-reject" checks stay wasmtime-free rcdzc unit tests). These pin the
; VALUE of const-closure/higher-order recursion shapes whose regressions produced a spurious CDZ0101
; decline or a wrong value — the value the compile-only half cannot witness.
(case
  "a const param re-passed on a mixed-match recursive arm does not drop, single and two-const forms (value parity)"
  (doc
    "Regression (const-param-drop): a `const` param re-passed on a SELF-RECURSIVE call sitting on a
           MIXED innermost match arm (a recursive arm beside a value-returning sibling) used to decline
           CDZ0101. filter-map shape (keep = return a value, drop = recurse): `twostep` over 0.. keeping
           the first `> 2` yields 3, both for a single const `step` and for two const params `step` + `f`.")
  (input
    (do
      (type Option (Some Int64) None)
      (def (mk (: n Int64)) (Option.Some n))
      (def
        (twostep (const (: step (-> Int64 Option))) (: s Int64))
        (match (step s) ((Option.None) 0) ((Option.Some x) (if (> x 2) x (twostep step (+ s 1))))))
      (def
        (twostep2 (const (: step (-> Int64 Option))) (: s Int64) (const (: f (-> Int64 Bool))))
        (match (step s) ((Option.None) 0) ((Option.Some x) (if (f x) x (twostep2 step (+ s 1) f)))))
      (def (single) (twostep (fn ((: n Int64)) (mk n)) 0))
      (def (both) (twostep2 (fn ((: n Int64)) (mk n)) 0 (fn ((: x Int64)) (> x 2))))
      (export single)
      (export both)))
  (call single)
  (output (: 3 Int64))
  (call both)
  (output (: 3 Int64)))

(case
  "a closure-payload sum built by an if-helper with a reused arg compiles and runs, not CDZ0101 (value parity)"
  (doc
    "Regression (fuse_match_into_if clone): `(run (mk k) k)` reuses `k` in BOTH arg positions; `mk k`
           reduces to an `if`, so `run`'s `(match (mk k) …)` triggers fuse_match_into_if, deep-copying the
           arm body `(f arg)` where `arg` = `k` is a beta-bound capture. A fresh copy re-resolved `k`
           lexically against the grafted branch (where `k` is invisible) → a spurious CDZ0101; the fix
           shares the pinned non-payload capture. k=4 → Fn arm `(* 4 3)` = 12; k=-1 → Const = 77.")
  (input
    (do
      (type Box (Fn (-> Int64 Int64)) (Const Int64))
      (def (mk (: k Int64)) (if (> k 0) (Box.Fn (fn ((: x Int64)) (* x 3))) (Box.Const 77)))
      (def (run (: b Box) (: arg Int64)) (match b ((Box.Fn f) (f arg)) ((Box.Const c) c)))
      (def (pos) (let ((k 4)) (run (mk k) k)))
      (def (neg) (let ((k -1)) (run (mk k) k)))
      (export pos)
      (export neg)))
  (call pos)
  (output (: 12 Int64))
  (call neg)
  (output (: 77 Int64)))

(case
  "a multi-use param bound to a pure recursive-helper call inlines and computes"
  (doc
    "A helper `bc` using a param in >=2 match arms is inlined into a caller that binds it to a pure
           fuel-recursive resolver; the pure recursive arg substitutes correctly. resolve(3)=SFix(go 0 3)
           =SFix(6); bc(SFix 6, SFix 6)= fa+fb = 12.")
  (input
    (do
      (type S (SVar Int64) (SFix Int64))
      (def (go (: acc Int64) (: n Int64)) (if (= n 0) acc (go (+ acc n) (- n 1))))
      (def (resolve (: v Int64)) (S.SFix (go 0 v)))
      (def
        (bc (: ra S) (: rb S))
        (match
          ra
          ((S.SVar ia) (match rb ((S.SVar ib) (+ ia ib)) ((S.SFix fb) (+ ia fb))))
          ((S.SFix fa) (match rb ((S.SVar ib) (+ fa ib)) ((S.SFix fb) (+ fa fb))))))
      (def (mid (: a Int64) (: b Int64)) (bc (resolve a) (resolve b)))
      (def (main (: k Int64)) (mid k k))
      (export main)))
  (call main (: 3 Int64))
  (output (: 12 Int64)))

(case
  "a nested tuple-destructuring lambda binds both levels"
  (doc
    "An inner destructuring lambda inside an outer one: g(tuple 3 4) = 30 + (4 + h(tuple 1 2)) =
           30 + 4 + 3 = 37, where h((tuple c d)) = c+d.")
  (input
    (do
      (def
        (main)
        (let
          ((g
              (fn
                (#tuple(a b))
                (let ((h (fn (#tuple(c d)) (+ c d)))) (+ (* a 10) (+ b (h #tuple(1 2))))))))
          (g #tuple(3 4))))
      (export main)))
  (output (: 37 Int64)))

(case
  "a multi-param lambda mixes a tuple-destructuring param with a bare param"
  (doc "f((tuple x y) z) = (x+y)+z: f(tuple 3 4, 5) = 7+5 = 12.")
  (input
    (do (def (main) (let ((f (fn (#tuple(x y) z) (+ (+ x y) z)))) (f #tuple(3 4) 5))) (export main)))
  (output (: 12 Int64)))

(case
  "a rest-pattern head binder read by an inlined match-arg callee resolves"
  (doc
    "A coin-DP shape: the rest-pattern head `c` from `(list c .. t)` is read inside a nested-match
           scrutinee that is the argument to `omin` (which matches its param and inlines). The head binder
           must resolve through the inline (regression: was CDZ0101 unbound). main → -1.")
  (input
    (do
      (def (at0 (: xs (List (Option Int64))) (: i Int64)) (Option.expect (List.at xs i) "x"))
      (def
        (omin (: a (Option Int64)) (: b (Option Int64)))
        (match a ((None _u) b) ((Some av) (match b ((None _u) a) ((Some bv) (if (< av bv) a b))))))
      (def
        (f (: cs (List Int64)) (: dp (List (Option Int64))) (: i Int64) (: best (Option Int64)))
        (match
          cs
          (#list() best)
          (#list(c (.. t))
            (f
              t
              dp
              i
              (if
                (<= c i)
                (omin
                  best
                  (match (at0 dp (- i c)) ((None _u) (None unit)) ((Some v) (Some (+ v 1)))))
                best)))))
      (def (main) (match (f #list(5 10) #list((Some 0)) 1 (None unit)) ((None _u) -1) ((Some r) r)))
      (export main)))
  (output (: -1 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "a runtime value-eq in a tail-loop condition does not clash the arithmetic scratch slot"
  (doc
    "`find` compares `(N.I n)` against `(N.I 3)` (an i32 heap-handle compare) in the condition of a
           tail-recursive wasm loop whose other branch does `(+ n 1)` (i64). The compare's scratch slot must
           not alias the arith slot (regression: forced one wasm local to two types). find(0) = 3.")
  (input
    (do
      (type N (I Int64) (J Int64))
      (def (mk (: n Int64)) (N.I n))
      (def (find (: n Int64)) (if (= (mk n) (mk 3)) n (find (+ n 1))))
      (export find)))
  (call find (: 0 Int64))
  (output (: 3 Int64)))

; -- breaker batch 512→516 (2026-08-27): incr6 calibration, mechanism CORRECTED per v-static-data's
; root-cause. The discriminator is NOT embedded/constructor-arg position — it is the ASSEMBLER: a
; compound-RETURNING export routes through the value-ENCODE boundary assembler, which USED NOT to
; emit the build-once GLOBAL/START sections, so its embedded constants allocated per-eval.
; WIT-STATIC-ENCODING increment (v-static-data, 2026-08-27): the FIXED-SHAPE tuple/record assembler
; (`emit_runtime_resource`) now emits build-once GLOBAL/START sections, so an EMBEDDED markable
; constant in such a return hoists to a build-once immortal global — imc1 dropped 2→1 (the inner
; `(tuple 1 2)` is now census-excluded; only the outer `(tuple n …)`, which captures the runtime `n`,
; is a mortal per-eval allocation). The COLLECTION-return path (imc2 list, irb1 33-elem list — these
; route through the recursive-sum/`value-encode`-walker sibling assembler) and a TOP-LEVEL constant
; return tuple (irb2, whose whole result is the constant) do NOT yet hoist — a follow-up increment
; wires build-once into those sibling assemblers; they stay at their mortal counts here.
(case
  "imc1 an EMBEDDED constant tuple in a compound RETURN now hoists build-once (the fixed-shape value-encode assembler gained build-once sections); only the runtime-`n` outer tuple is mortal"
  (input (do (def (main (: n Int64)) #tuple(n #tuple(1 2))) (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 5 (tuple 1 2)) (Tuple Int64 (Tuple Int64 Int64))))
  (live-objects 1))

(case
  "imc2 a constant tuple as a returned LIST element now hoists build-once (the collection-return assembler gained build-once); only the list vec + the runtime-`n` tuple are mortal"
  (input (do (def (main (: n Int64)) #list(#tuple(1 2) #tuple(n 9))) (export main)))
  (call main (: 5 Int64))
  (output (: #list(#tuple(1 2) #tuple(5 9)) (List (Tuple Int64 Int64))))
  ; The embedded constant `(tuple 1 2)` is a census-excluded build-once immortal now: 4→3.
  (live-objects 3))

(case
  "a pass-through parameter re-passed unchanged across a tail loop is not corrupted"
  (doc
    "`go(n,k,acc)` re-passes k unchanged each iteration (the back-edge elides the k←k self-move);
           acc += k over n steps = n*k. main(100,2)=200; main(5,7)=35.")
  (input
    (do
      (def (go (: n Int64) (: k Int64) (: acc Int64)) (if (= n 0) acc (go (- n 1) k (+ acc k))))
      (def (main (: n Int64) (: k Int64)) (go n k 0))
      (export main)))
  (call main (: 100 Int64) (: 2 Int64))
  (output (: 200 Int64))
  (call main (: 5 Int64) (: 7 Int64))
  (output (: 35 Int64)))

(case
  "a recursive bool predicate types as Bool with the self-call in either branch order"
  (doc
    "all-lt has the self-call in the THEN branch, all-ge in the ELSE (the mirror); both type as Bool.
           main = 10*(all-lt 0 3 bound) + (all-ge 0 3 0): bound=5 → 11 (both true).")
  (input
    (do
      (def
        (all-lt (: i Int64) (: n Int64) (: bound Int64))
        (if (< i n) (if (< i bound) (all-lt (+ i 1) n bound) false) true))
      (def
        (all-ge (: i Int64) (: n Int64) (: bound Int64))
        (if (< i n) (if (< i bound) false (all-ge (+ i 1) n bound)) true))
      (def (main (: bound Int64)) (+ (* 10 (if (all-lt 0 3 bound) 1 0)) (if (all-ge 0 3 0) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a recursive param used only as a call argument infers its type from the callee"
  (doc
    "`a` is passed only to `(twice a)` (never touched by an operator, never annotated); it infers
           Int64 from twice's parameter. f(5,3) = twice(5)*3 summed = 30.")
  (input
    (do
      (def (twice (: a Int64)) (+ a a))
      (def (f (: a Int64) (: n Int64)) (if (< n 1) 0 (+ (twice a) (f a (- n 1)))))
      (def (main (: a Int64) (: n Int64)) (f a n))
      (export main)))
  (call main (: 5 Int64) (: 3 Int64))
  (output (: 30 Int64)))

; ── breaker batch 520: the generic-transformer closure-result family (siblings of the tracked
; issue BUG-generic-transformer-closure-compound-result-grounds-elements-to-unit, routed
; v-inference). Full 7-cell matrix probed on all three targets; the E0308 miscompile cells
; (structural aggregate result × discarding consumer × two domains: tuple/record/List/nested)
; CANNOT enter the corpus until fixed (they red the rust battery) — the map lives in the issue
; file. Pinned here: the three cells that are green/attributable today. All carry the
; recursive-sum known-leak (the rsl1 class): these clauses flip to 0 on the reclaim fix.
; RESOLVED #4319 (verified tick 301): gtx1/4/6/7 E0308→PASS and gtx2 decline→PASS(5) on both
; rust targets. RESIDUAL: gtx3 (single-domain) still declines on rust — over-conservative, wasm
; proves 2; flips whenever the single-domain tie lands. Leak clauses unchanged (rsl1 class).
(case
  "gtx2 a generic transformer's tuple-result closure with a consumer that PROJECTS the element (vars pinned by the consumer body)"
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def
        (from-list xs)
        (match xs (#list() (GIter.Nil)) (#list(h (.. t)) (GIter.Cons h (from-list t)))))
      (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest)))))
      (def
        (gmap it f)
        (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f)))))
      (def
        (sumfirst it)
        (match it ((GIter.Nil) 0) ((GIter.Cons h rest) (+ (. h 0) (sumfirst rest)))))
      (def
        (main)
        (+
          (sumfirst (gmap (from-list #list(1 2)) (fn (x) #tuple(x x))))
          (count (gmap (from-list #list("a" "b")) (fn (s) (String.concat s s))))))
      (export main)))
  (call main)
  (output (: 5 Int64))
  (live-objects known-leak))

(case
  "gtx3 a generic transformer's tuple-result closure at a SINGLE domain with a discarding consumer"
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def
        (from-list xs)
        (match xs (#list() (GIter.Nil)) (#list(h (.. t)) (GIter.Cons h (from-list t)))))
      (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest)))))
      (def
        (gmap it f)
        (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f)))))
      (def (main) (count (gmap (from-list #list(1 2)) (fn (x) #tuple(x x)))))
      (export main)))
  (call main)
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "gtx5 a generic transformer's Option-result closure with a discarding consumer at two domains (the nominal-sum cell is green on every target)"
  (input
    (do
      (type GIter (Nil) (Cons a (GIter a)))
      (def
        (from-list xs)
        (match xs (#list() (GIter.Nil)) (#list(h (.. t)) (GIter.Cons h (from-list t)))))
      (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest)))))
      (def
        (gmap it f)
        (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f)))))
      (def
        (main)
        (+
          (count (gmap (from-list #list(1 2)) (fn (x) (Option.Some x))))
          (count (gmap (from-list #list("a" "b")) (fn (s) (String.concat s s))))))
      (export main)))
  (call main)
  (output (: 4 Int64))
  (live-objects known-leak))

; ── breaker batch 532: constant-RETURN calibration extended to the immortal era (the imc family's
; size classes). Post #4330/#4354 (deep-mark hoisting for internal uses), a constant in RETURN
; position still routes through the value-encode boundary assembler (verified: 0 static globals)
; and builds MORTAL cells — plain exact-N reachable-return clauses, NOT leaks. These flip to 0
; alongside imc1/imc2 when that assembler gains build-once sections. Returning through a helper
; reads identically (probed, not separately pinned).
(case
  "irb1 a constant 33-element list in RETURN position now hoists build-once (the collection-return assembler gained build-once); both constant if-branch lists are census-excluded immortals → 0 mortal"
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (> n 0)
          #list(1
            2
            3
            4
            5
            6
            7
            8
            9
            10
            11
            12
            13
            14
            15
            16
            17
            18
            19
            20
            21
            22
            23
            24
            25
            26
            27
            28
            29
            30
            31
            32
            33)
          #list(9)))
      (export main)))
  (call main (: 1 Int64))
  (output
    (:
      #list(1
        2
        3
        4
        5
        6
        7
        8
        9
        10
        11
        12
        13
        14
        15
        16
        17
        18
        19
        20
        21
        22
        23
        24
        25
        26
        27
        28
        29
        30
        31
        32
        33)
      (List Int64)))
  (live-objects 0))

(case
  "irb2 a constant nested tuple in RETURN position builds mortal cells (outer+inner; same assembler path as imc1)"
  (input
    (do
      (def (main (: n Int64)) (if (> n 0) #tuple(1 #tuple(2 3)) #tuple(9 #tuple(9 9))))
      (export main)))
  (call main (: 1 Int64))
  (output (: (tuple 1 (tuple 2 3)) (Tuple Int64 (Tuple Int64 Int64))))
  (live-objects 2))

(case
  "a mixed-width mutual-recursion SCC emits valid wasm (per-member scratch floor)"
  (doc
    "A mutual-recursion SCC compiles all members into ONE shared dispatch loop sharing the function's
           local slots, so two members must not stash different-WIDTH temps in the SAME slot: r0's String `=`
           tees an i32 handle into the floor slot, while r1's multi-use i64 `let j` would claim that same slot
           → a local declared both i32 and i64 = invalid wasm. Per-member fresh scratch floors avoid the
           collision (the mutual-SCC companion of the self-tail-recursive different-width valid-emit cases
           above). r0(\"z\",4,6): \"zz\"≠\"xx\" → r1(3)→r0(2)→r1(1)→r0(0)→r1(-1): j=(-1+6)=5 → (+ j j)=10; the
           run itself re-verifies valid emit (an invalid module would not compose or run). Relocated from rcdzc
           a_mixed_width_mutual_scc_gives_each_member_a_fresh_scratch_floor.")
  (input
    (do
      (def
        (r0 (: s String) (: a Int64) (: b Int64))
        (if (= (String.concat s s) "xx") a (r1 s (- a 1) b)))
      (def
        (r1 (: s String) (: a Int64) (: b Int64))
        (let ((j (+ a b))) (if (< a 1) (+ j j) (r0 s (- a 1) b))))
      (def (main) (r0 "z" 4 6))
      (export main)))
  (output (: 10 Int64)))

(case
  "a self-referential-shaped HOF that is not actually recursive folds"
  (doc
    "`(twice f v) = (f (f v))` nests the SAME non-recursive function twice — a legitimate TERMINATING
           fold, NOT recursion. The static recursion check must NOT false-positive on it (an old
           body-on-stack set did, blocking a valid higher-order fold). `(twice (fn (x) (+ x 1)) 5)` folds to
           (+ (+ 5 1) 1) = 7.")
  (input
    (do (def (main) (let ((twice (fn (f v) (f (f v))))) (twice (fn (x) (+ x 1)) 5))) (export main)))
  (output (: 7 Int64)))

(case
  "a recursive match binder scrutinee is materialized once (linear, not 2^depth)"
  (doc
    "A match/pattern BINDER used more than once must NOT re-emit its whole scrutinee per use. When the
           scrutinee is a RECURSIVE CALL, a binder used K times would re-run that call K times per recursion
           level → 2^depth. `f` recurses to (Mk 1 1) at n=0; each arm matches (f (+ n 1)), binds `a`, and uses
           it TWICE in (Mk a a). Materialized-once keeps it LINEAR: (f -60) is 60 self-calls → 1. A regression
           to per-use re-emission is 2^60, which hits the run DEADLINE and TRAPS — so this case's RUN is the
           linear-vs-exponential witness (the gate's run-deadline is the perf catch). Relocated from the
           in-crate rcdzc a_recursive_match_binder_scrutinee_is_materialized_once.")
  (input
    (do
      (type P (Mk Int64 Int64))
      (def (f (: n Int64)) (if (= n 0) (Mk 1 1) (match (f (+ n 1)) ((Mk a _) (Mk a a)))))
      (def (main) (match (f -60) ((Mk x _) x)))
      (export main)))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "ftd1 many same-signature functions dispatched through a selector are value-exact (the #4754 functype-dedup witness)"
  (doc
    "#4754 collapses identical core functypes for closure-free programs. Five (Int64)->Int64 defs
           (a/b/c/d + the pick selector) dispatched via a runtime-selected chain must stay value-correct
           after the dedup: pick(5%4=1)=b(5)=7 + pick(6%4=2)=c(5)=8 = 15. Pins that collapsing the
           functype section preserves per-function BEHAVIOR (a dedup that merged the wrong bodies, or
           mis-indexed a call after collapsing types, would change this value).")
  (input
    (do
      (def (a (: x Int64)) (+ x 1))
      (def (b (: x Int64)) (+ x 2))
      (def (c (: x Int64)) (+ x 3))
      (def (d (: x Int64)) (* x 2))
      (def
        (pick (: n Int64) (: x Int64))
        (if (= n 0) (a x) (if (= n 1) (b x) (if (= n 2) (c x) (d x)))))
      (def (main (: n Int64)) (+ (pick (% n 4) n) (pick (% (+ n 1) 4) n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "hta1 a 24-field runtime tuple builds a VALID artifact and every field projects (high-arity, verified by running)"
  (doc
    "The in-guest high-arity seam (compile-success proves nothing — verify by RUNNING; the 17-flat-WIT-
           param bug produced silent invalid wasm). A 24-field tuple all bound to a runtime n, summed over all
           24 projections = 24n. n=5 -> 120. Pins that a wide tuple lowers to a runnable artifact and no field
           index aliases another.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((t #tuple(n n n n n n n n n n n n n n n n n n n n n n n n)))
          (+
            (+
              (+
                (+
                  (+
                    (+
                      (+
                        (+
                          (+
                            (+
                              (+
                                (+
                                  (+
                                    (+
                                      (+
                                        (+
                                          (+
                                            (+
                                              (+
                                                (+
                                                  (+ (+ (+ (. t 0) (. t 1)) (. t 2)) (. t 3))
                                                  (. t 4))
                                                (. t 5))
                                              (. t 6))
                                            (. t 7))
                                          (. t 8))
                                        (. t 9))
                                      (. t 10))
                                    (. t 11))
                                  (. t 12))
                                (. t 13))
                              (. t 14))
                            (. t 15))
                          (. t 16))
                        (. t 17))
                      (. t 18))
                    (. t 19))
                  (. t 20))
                (. t 21))
              (. t 22))
            (. t 23))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 120 Int64)))

(case
  "hta2 a 20-parameter function builds a VALID artifact and sums all params (high param-count, verified by running)"
  (input
    (do
      (def
        (wide
          (: p0 Int64)
          (: p1 Int64)
          (: p2 Int64)
          (: p3 Int64)
          (: p4 Int64)
          (: p5 Int64)
          (: p6 Int64)
          (: p7 Int64)
          (: p8 Int64)
          (: p9 Int64)
          (: p10 Int64)
          (: p11 Int64)
          (: p12 Int64)
          (: p13 Int64)
          (: p14 Int64)
          (: p15 Int64)
          (: p16 Int64)
          (: p17 Int64)
          (: p18 Int64)
          (: p19 Int64))
        (+
          (+
            (+
              (+
                (+
                  (+
                    (+
                      (+
                        (+
                          (+ (+ (+ (+ (+ (+ (+ (+ (+ (+ p0 p1) p2) p3) p4) p5) p6) p7) p8) p9) p10)
                          p11)
                        p12)
                      p13)
                    p14)
                  p15)
                p16)
              p17)
            p18)
          p19))
      (def (main (: n Int64)) (wide n n n n n n n n n n n n n n n n n n n n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 100 Int64)))

(case
  "immortal-nullary FBIP witness: a recursive Nat walk REBUILDS the spine (FBIP-reuses S cells) and returns a fresh terminal (Z), then re-folds it — value-correct + the rebuilt immortal Z is shared (0-leak after the immortal fix)"
  (doc
    "Witnesses the FBIP-immortal-no-mutate invariant for the mixed-sum nullary terminal: `rebuild` walks
        n, reusing each S cell in place (FBIP rc==1) and constructing a fresh (Nat.Z) terminal on the base
        arm; `depth` then folds the rebuilt spine. The immortal Z (build-once, u32::MAX rc) is never
        FBIP-reused-in-place (reuse gate is strictly rc==1) so it path-copies — value stays correct and the
        shared terminal is census-excluded (0-leak). Pre-fix this leaks the terminal(s); post-fix it is 0.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (mk (: k Int64)) (if (< k 1) (Nat.Z) (Nat.S (mk (- k 1)))))
      (def (rebuild (: n Nat)) (match n ((Nat.Z) (Nat.Z)) ((Nat.S m) (Nat.S (rebuild m)))))
      (def (depth (: n Nat) (: acc Int64)) (match n ((Nat.Z) acc) ((Nat.S m) (depth m (+ acc 1)))))
      (def (main (: k Int64)) (depth (rebuild (mk k)) 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "nts1 a non-tail recursive-sum consumer over a SHARED spine does not over-reclaim it (sum then len both read it)"
  (doc
    "Soundness fence for the #4857 non-tail recursive-sum spine reclaim (drop a CALLEE-OWNED, dead-after
           param's shell after its MatchSum). `sum` is a NON-tail consumer `(+ h (sum t))` — exactly the shape
           #4857 targets, so its param `l` takes the new shell-drop. But here the runtime-built list `xs` is
           SHARED: consumed by `sum` AND then read by `len`. The drop must NOT over-reclaim the shared spine —
           `xs` is dup'd for the two consumers (dup <=> drop coupled), so `len xs` reads it intact AFTER `sum`
           has consumed its copy: `1000*sum(1..4) + len = 1000*10 + 4` = 10004. A regression that narrowed the
           callee-owned exclusion (dropping a shared/boundary spine) would double-free → a wrong value or a
           trap in `len`. The residual `known-leak 8` is the tracked spine residue (the conservative fix reduces
           it, never increases; flips lower when the full §5 reclaim lands). release==debug (no latent UAF).")
  (input
    (do
      (type L (Cons Int64 L) (Nil))
      (def (bld (: n Int64)) (if (> n 0) (Cons n (bld (- n 1))) (Nil)))
      (def (sum (: l L)) (match l ((Cons h t) (+ h (sum t))) ((Nil) 0)))
      (def (len (: l L)) (match l ((Cons h t) (+ 1 (len t))) ((Nil) 0)))
      (def (main (: n Int64)) (let ((xs (bld n))) (+ (* 1000 (sum xs)) (len xs))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 10004 Int64))
  (live-objects 0))

(case
  "a heap-typed EXPORTED-entry param + a reachable RECURSIVE fn emits VALID wasm (the def-call index survives the entry's lift-op imports)"
  (doc
    "Fuzzer/breaker bucket-1 regression (rcdzc-wasm miscompile). An EXPORTED entry with a HEAP param
           (String/Bytes/List/Option) makes `try_bare_entry_param_component` APPEND lift-op imports
           (bytes-alloc/set for the String lift), which shifts every DEFINED func index up by `added`. The
           def bodies were selected with the ORIGINAL import_base, so a baked def-to-def call — here the
           recursive `v1`'s reachable call (recursion cannot be inlined away, so a REAL `Lir::Call`/
           `ReturnCall` survives) — pointed at the pre-shift index and resolved to an APPENDED import op,
           emitting INVALID wasm (`requires [i64] but callee returns [i32]`). The fix re-shifts baked
           def-target call indices (>= import_base) by `added`. `v1(byte-len \"hi\"=2)` = 2->1->0 = 0. A
           scalar-param entry (no lift ops, added=0) was always fine — the control below.")
  (input
    (do
      (def
        (main (: v0 String))
        (do (def (v1 v2) (if (<= v2 0) v2 (v1 (- v2 1)))) (v1 (String.byte-len v0))))
      (export main)))
  (call main (: "hi" String))
  (output (: 0 Int64)))

(case
  "control: a SCALAR-param entry + a reachable recursive fn (no lift-op imports, no index shift)"
  (input
    (do
      (def (main (: v0 Int64)) (do (def (v1 v2) (if (<= v2 0) v2 (v1 (- v2 1)))) (v1 v0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 0 Int64)))

(case
  "a LIST-param entry + a reachable recursive fn emits VALID wasm (a different lift-op set, a different index shift)"
  (doc
    "The List face of the index-shift pin above (which lifts a String): a `(List Int64)` entry param
           appends the ARR lift ops (arr-alloc/arr-set — a different op set, hence a different `added`
           shift) — the re-shift must be keyed on the actual appended count, not a constant. The nested
           `go` recursion keeps a real def-to-def call live, and the result mixes the recursion's value
           with a read of the lifted param (`go(3)=6 + len=3 → 9`), so a mis-resolved callee corrupts the
           value even where it happens to validate. Breaker acceptance-ladder face n1 (tick 462/468).")
  (input
    (do
      (def
        (main (: xs (List Int64)))
        (do (def (go (: i Int64)) (if (<= i 0) 0 (+ i (go (- i 1))))) (+ (go 3) (List.len xs))))
      (export main)))
  (call main (: #list(10 20 30) (List Int64)))
  (output (: 9 Int64)))

(case
  "must-hold: a heap-param entry + a NON-recursive nested fn CAPTURING the param compiles and runs"
  (doc
    "The capture-adjacent CONTROL of the index-shift class: a nested fn that reads the enclosing
           heap param but does NOT recurse is fully inlined at its call site, so no def-to-def call
           survives and no index is at risk — this must keep working. It is also the boundary fence for
           the SEPARATE recursive-capture gap (a RECURSIVE nested fn capturing the param is an uncoded
           no-local-slot error today, v-inference lane): when that fix lands, this control proves the
           non-recursive face never regressed. Breaker acceptance-ladder face n6 (tick 462/468).")
  (input
    (do
      (def (main (: xs (List Int64))) (do (def (peek) (List.len xs)) (+ (peek) 100)))
      (export main)))
  (call main (: #list(10 20 30) (List Int64)))
  (output (: 103 Int64)))

(case
  "an UNCALLED stored closure with a type-conflicting param is REJECTED at compile, not miscompiled"
  (doc
    "The uncalled-closure conflict-escape CLASS fence (#4980): `collect_node` historically relied on
           the beta-reduction CALL SITE to fault-check an inline closure body, so a STORED-but-uncalled
           closure whose param is used at two incompatible types escaped the checker and reached the
           lowering — `(. v 0)` got a per-op decline (#4970), but the `List.len`/`Bytes.len`/`Bytes.at`
           siblings emitted INVALID wasm. #4980 fault-checks uncalled bodies directly, so every sibling now
           rejects with the same positional type fault. This pins the `List.len` sibling (the first live
           escape found); the valid-closure control below must keep compiling.")
  (input (do (def (main (: n Int64)) (List.len #list((fn (v) (+ v (List.len v)))))) (export main)))
  (error CDZ0203 (exact-code)))

(case
  "control: an UNCALLED stored closure with a CONSISTENT param still compiles and runs"
  (doc
    "The must-hold twin of the conflict-escape fence: a stored-but-uncalled closure whose body is
           type-consistent (`(+ v 1)`) must keep compiling — the #4980 uncalled-body fault-check must fault
           only genuine conflicts, not the mere fact of being uncalled.")
  (input (do (def (main (: n Int64)) (List.len #list((fn (v) (+ v 1))))) (export main)))
  (call main (: 3 Int64))
  (output (: 1 Int64)))

(case
  "an extracted closure re-stored into a new list then re-extracted and applied reclaims"
  (doc
    "A closure `(mk k)` extracted from a source list via `List.at` is MOVED into a fresh `#list(f)`
           (a consuming container construction), re-extracted, and applied. The container-escape KEEPS the
           extraction dup (the closure is genuinely moved out of the source sum shell), so after the inner
           apply everything reclaims to zero — the UAF-safety companion of the closure-extraction fold: dup
           KEPT for a real move-out, no double-free if it is ever wrongly dropped. `main k` = 10 + k.")
  (input
    (do
      (def (mk k) (fn (x) (+ x k)))
      (def
        (main (: k Int64))
        (let
          ((fs #list((mk k) (mk 2))))
          (match
            (List.at fs 0)
            ((Some f) (let ((gs #list(f))) (match (List.at gs 0) ((Some g) (g 10)) (None -2))))
            (None -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 5 Int64))
  (output (: 15 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "an extracted closure placed into a tuple then projected and applied reclaims"
  (doc
    "A closure `(mk k)` extracted via `List.at` is placed into a TUPLE `(tuple f 99)` (a consuming
           construction), then projected back out `(. p 0)` and applied. Tuple-escape KEEPS the extraction
           dup (genuine move-out), reclaiming to zero after the apply — no double-free. `main k` = 10 + k.")
  (input
    (do
      (def (mk k) (fn (x) (+ x k)))
      (def
        (main (: k Int64))
        (let
          ((fs #list((mk k) (mk 2))))
          (match (List.at fs 0) ((Some f) (let ((p #tuple(f 99))) ((. p 0) 10))) (None -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 5 Int64))
  (output (: 15 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "a runtime-selected closure from a list of DISTINCT-BODY closures dispatches indirectly and reclaims"
  (doc
    "Unlike the same-factory runtime-index case, this list holds closures with DIFFERENT bodies —
           `(adder k)` and `(mul k)` — so the apply site cannot devirtualize to one known body and must emit
           a genuine indirect `call_indirect`. The selected closure is extracted and applied; the extraction
           reclaims to zero (the call BORROWS the env cell on the indirect path too, so removing the shell-
           reclaim dup is sound there as well). index `(if (> k 1) 1 0)`: k<2 → adder → 10+k; k>=2 → mul → 10*k.")
  (input
    (do
      (def (adder n) (fn (x) (+ x n)))
      (def (mul n) (fn (x) (* x n)))
      (def
        (main (: k Int64))
        (let
          ((fs #list((adder k) (mul k))))
          (match (List.at fs (if (> k 1) 1 0)) ((Some f) (f 10)) (None -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 2 Int64))
  (output (: 20 Int64))
  (call main (: 3 Int64))
  (output (: 30 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "an extracted closure passed through a helper then applied reclaims"
  (doc
    "A closure `(mk k)` extracted via `List.at` is passed as an ARGUMENT to a helper `(applyit g) =
           (g 10)` and applied there — the closure reaches an apply through a PARAMETER boundary. Reclaims to
           zero, no double-free. `main k` = 10 + k.")
  (input
    (do
      (def (mk k) (fn (x) (+ x k)))
      (def (applyit g) (g 10))
      (def
        (main (: k Int64))
        (let ((fs #list((mk k) (mk 2)))) (match (List.at fs 0) ((Some f) (applyit f)) (None -1))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 5 Int64))
  (output (: 15 Int64))
  ; interim known-leak: #6022/#6049 borrowed-env closure-application (v-mem adjudicated 2026-08-30); reclaim batch -> 0
  (live-objects 0))

(case
  "hc1 an empty-captures COMBINATOR boxed as a first-class value and applied via call_indirect reclaims its compound shells"
  (doc
    "The INC1 acceptance fence (v-core-opt owned-param compound-shell reclaim; v-runtime Q3 +
           v-mem disjointness co-verified, breaker two-sided sweep 2026-08-30): two empty-captures
           combinators over a compound BST — peel (rebuilds the node) and mirror (swaps children) —
           are BOXED into a variant payload, runtime-selected `(if (= mode 1) (Boxed peel) (Boxed
           mirror))`, extracted by match and applied → a GENUINE call_indirect (the runtime-selected
           funcref-through-variant shape is REQUIRED: a statically-known box DEVIRTUALIZES to a
           direct call and misses the path). The edge this pins: under call_indirect params stay
           CALLEE-owned; only the env CELL is caller-reclaimed (two DISJOINT drops — no double-free,
           release-trap clean both modes). depth counts nodes: 3 for the 3-node tree under both
           combinators.")
  (input
    (do
      (type BST (Empty) (Node (Tuple BST Int64 BST)))
      (type FnBox (Boxed (-> BST BST)))
      (def
        (peel (: t BST))
        (match t ((Empty _u) (Empty)) ((Node p) (match p (#tuple(l k r) (Node #tuple(l k r)))))))
      (def
        (mirror (: t BST))
        (match t ((Empty _u) (Empty)) ((Node p) (match p (#tuple(l k r) (Node #tuple(r k l)))))))
      (def (unbox-apply (: b FnBox) (: x BST)) (match b ((Boxed f) (f x))))
      (def
        (depth (: t BST))
        (match t ((Empty _u) 0) ((Node p) (match p (#tuple(l _k r) (+ 1 (+ (depth l) (depth r))))))))
      (def
        (main (: mode Int64))
        (do
          (def
            tree
            (Node #tuple((Node #tuple((Empty) 3 (Empty))) 5 (Node #tuple((Empty) 8 (Empty))))))
          (def b (if (= mode 1) (Boxed peel) (Boxed mirror)))
          (depth (unbox-apply b tree))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 2 Int64))
  (output (: 3 Int64))
  ; TIGHTENED to 0 (2026-09-01): INC1 increment-1 (#7342, self-recursion-gated owned-param-shell
  ; reclaim) legitimately collapsed this — v-memory-safety A/B + WAT-balance verified (drop-calls
  ; 1->2 balanced, guarded-all clean).
  (live-objects 0)
  only
  when
  a
  SOUND
  reclaim
  ; discriminator lands, with the owning lane's sign-off + a fresh census.
  (live-objects 0))

(case
  "hc2 a CAPTURING closure boxed as a value stays EXCLUDED from the combinator shell-reclaim (leak, never double-free)"
  (doc
    "The INC1 negative control (keeps-hold side of the census criterion): `(mk base)` returns a
           closure CAPTURING the free var `base` (non-empty captures → excluded by the
           captures.is_empty() discriminator), same variant-box + runtime-selected genuine
           call_indirect, same compound param + compound-rebuild arm. The exclusion must fail SAFE:
           retained cells (a leak the reclaim lane later collapses), NEVER a caller-side drop of a
           callee-owned param (double-free/UAF). Values correct + release-trap clean both modes;
           this pin must STAY a leak until the capturing-closure reclaim lane lands its own
           increment — a spontaneous drop to 0 here without that landing is the OVER-SUPPRESSION
           signal (UAF direction), investigate before tightening.")
  (input
    (do
      (type BST (Empty) (Node (Tuple BST Int64 BST)))
      (type FnBox (Boxed (-> BST BST)))
      (def (unbox-apply (: b FnBox) (: x BST)) (match b ((Boxed f) (f x))))
      (def
        (depth (: t BST))
        (match t ((Empty _u) 0) ((Node p) (match p (#tuple(l _k r) (+ 1 (+ (depth l) (depth r))))))))
      (def
        (mk (: base BST))
        (fn
          ((: t BST))
          (match t ((Empty _u) base) ((Node p) (match p (#tuple(l k r) (Node #tuple(l k r))))))))
      (def
        (main (: mode Int64))
        (do
          (def f1 (mk (Node #tuple((Empty) 99 (Empty)))))
          (def f2 (mk (Empty)))
          (def b (if (= mode 1) (Boxed f1) (Boxed f2)))
          (def tree (Node #tuple((Empty) 5 (Empty))))
          (depth (unbox-apply b tree))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 1 Int64))
  ; TIGHTENED to 0 (2026-09-01): the tripwire FIRED on #7342 and the investigation CORRECTED the
  ; original attribution — the 2 live cells were depth's unreclaimed tree spine (INC1's exact
  ; target class), NOT the capturing closure's env; the capturing exclusion holds structurally
  ; (body_is_capturing_lifted gate) and produces no measurable retention in this shape.
  ; v-memory-safety A/B pinned the flip to #7342, WAT dup/drop balanced, guarded-all UAF-clean.
  (live-objects 0))

; A RECURSIVE local function that CAPTURES a binding from its enclosing scope is LAMBDA-LIFTED: the local
; function is threaded the captured value as an explicit trailing parameter (its enclosing param's slot is
; added to the local's own frame, and every call site — the enclosing call and the recursive self-call —
; passes the captured value as that extra argument), so no closure/heap-env is needed and it compiles +
; runs. `rec` captures its enclosing `n`; the lift makes `rec` take `(x n)` and rewrites `(rec 5)` →
; `(rec 5 n)` and the self-call `(rec (- x 1))` → `(rec (- x 1) n)`. `rec` counts x down from 5 to -1 and
; then returns the captured n, so `(f 7)` = 7. The boundary is exactly RECURSION + CAPTURE: a non-recursive
; capturing local inlines (the binding flows in), and a non-capturing recursive local compiles standalone
; (the two positive companions below pin that only the combination needs the lift). A captured NON-parameter
; binding (an enclosing `let`-local, which has no signature slot to thread) is not yet lifted and still
; declines — this case is the parameter-capture increment.
(case
  "a recursive local function that captures its enclosing parameter is lambda-lifted and runs"
  (doc
    "The capturing-recursive local `rec` reads its enclosing function's parameter `n`. Lambda-lifting
           threads `n` in as an explicit trailing parameter of `rec` (rather than a heap closure), so both
           the enclosing call `(rec 5)` and the recursive self-call `(rec (- x 1))` forward the captured
           `n`. `rec` counts `x` down from 5 to -1 and returns the captured `n`, so `(f 7)` yields 7.")
  (input
    (do
      (def (f (: n Int64)) (do (def (rec (: x Int64)) (if (< x 0) n (rec (- x 1)))) (rec 5)))
      (export f)))
  (call f (: 7 Int64))
  (output (: 7 Int64)))

(case
  "the work-around — threading the captured value as an explicit parameter — compiles and runs"
  (doc
    "The positive companion of the recursive-local-capture decline: passing the captured `n` in as an
           explicit parameter of the local recursive function removes the capture, so no lambda-lift is
           needed and it compiles + runs. `(rec 5 7)` counts down x from 5 to -1 returning the threaded n=7.")
  (input
    (do
      (def
        (f (: n Int64))
        (do (def (rec (: x Int64) (: acc Int64)) (if (< x 0) acc (rec (- x 1) acc))) (rec 5 n)))
      (export f)))
  (call f (: 7 Int64))
  (output (: 7 Int64)))

(case
  "a non-capturing recursive local function compiles and runs (only capture+recursion declines)"
  (doc
    "The second positive companion: a recursive local function that captures NOTHING compiles
           standalone (no lift needed). Pins that RECURSION alone is fine — only recursion PLUS capture
           declines. `(rec 5)` counts x down to -1 returning 0.")
  (input
    (do
      (def (f (: n Int64)) (do (def (rec (: x Int64)) (if (< x 0) 0 (rec (- x 1)))) (rec n)))
      (export f)))
  (call f (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a recursive local function that captures TWO enclosing parameters is lambda-lifted and runs"
  (doc
    "Lambda-lift threads MORE THAN ONE captured parameter — `rec` reads both enclosing params `a` and
           `b`, so it is lifted to take `(x a b)` and every call forwards both. `rec` counts `x` down from
           3 to -1 then returns `(+ a b)`, so `(g 4 5)` yields 9. Pins that the capture set threads in
           signature order (positional), not just a single value.")
  (input
    (do
      (def
        (g (: a Int64) (: b Int64))
        (do (def (rec (: x Int64)) (if (< x 0) (+ a b) (rec (- x 1)))) (rec 3)))
      (export g)))
  (call g (: 4 Int64) (: 5 Int64))
  (output (: 9 Int64)))

(case
  "ilc1 an internal (non-exported) recursive-local-param-capture helper, called once, still compiles + runs"
  (doc
    "Inline-path regression fence for the recursive-local-capture decline. Every companion above
           `(export f)`s the def, which forces STANDALONE emission and hides the inline path; this pins the
           INLINE-eligible face. `helper` is NOT exported and has a single caller (`main`), so it is an
           inline candidate. Its body holds a recursive local `rec` that captures the enclosing param `n` —
           safe standalone (lambda-lifted, cf. the case above), but if the def were INLINED `n` would become
           an enclosing let-binding and `rec` would capture enclosing SCOPE, tripping the coded CDZ0900
           do-local-binding decline (the `llb1` gap face below). The invariant this fence locks in: INLINING
           MUST PRESERVE COMPILABILITY — it may never turn a runnable program into a decline. `rec` counts
           `x` down from 5 to -1 then returns the captured `n`, so `(main 7)` = 7. (Guards the #8018->#8058
           emit-once revert cycle in the corpus, not just a shred @test.)")
  (input
    (do
      (def
        (helper (: n Int64))
        (do (def (rec (: x Int64)) (if (< x 0) n (rec (- x 1)))) (rec 5)))
      (def (main (: seed Int64)) (helper seed))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64)))

(case
  "llb1 a recursive do-local fn capturing a do-LOCAL BINDING computes (should-work: lexical capture is uniform)"
  (doc
    "Idealistic TODO fence (corpus policy 2026-08-31; gap = the coded CDZ0900 'a recursive local
     function that captures a binding from its enclosing scope is not supported', routed
     v-inference — the #6879 lambda-lift landed the enclosing-PARAM face; this pins the do-local
     BINDING face, the staged residual): closures capture lexical scope uniformly, so `step` (a
     do-local def) must capture exactly like the param `n` does. Derivation: climb(4,0) adds
     (step + n) = (3 + n) four times: n=2 -> 20; n=0 -> 12. Auto-flips when the binding-capture
     lift lands.")
  (input
    (do
      (def
        (main (: n Int64))
        (do
          (def step 3)
          (def
            (climb (: k Int64) (: acc Int64))
            (if (> k 0) (climb (- k 1) (+ acc (+ step n))) acc))
          (climb 4 0)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 20 Int64))
  (call main (: 0 Int64))
  (output (: 12 Int64)))

; --- Variable-arity (varargs) functions: a `(.. binder)` REST parameter -----------------------
; DESIGN-variable-arity-functions.md §3.1: a function's LAST parameter may be a rest parameter
; `(.. (: xs (List T)))`, which GATHERS all trailing arguments into a single homogeneous `List T`
; value. The function keeps ONE runtime body over that list; a call supplies zero or more trailing
; arguments (each must have the element type T), so `count(1, 2, 3)`, `count()`, and `tagged(100, 7,
; 8, 9)` all resolve against the one definition. This is the parameter-side dual of the value-position
; `(.. v)` spread and reuses the same marker shape. (The heterogeneous tuple-rest is a later increment.)
(case
  "a list-rest parameter gathers the trailing arguments into a list"
  (doc
    "`(def (count (.. (: xs (List Int64)))) (List.len xs))` gathers every argument into `xs : List
           Int64`, so `(count 1 2 3)` binds `xs = [1, 2, 3]` and its length is 3. Witnesses that a rest
           parameter collects a variable number of arguments into one homogeneous list the body reads.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (count 1 2 3))
      (export main)))
  (output (: 3 Int64)))

(case
  "a list-rest parameter with zero trailing arguments is the empty list"
  (doc
    "The degenerate boundary: `(count)` supplies no trailing arguments, so the rest parameter binds the
           EMPTY list `[]` and its length is 0. Pins that a varargs call with no arguments is well-formed —
           the rest gathers nothing rather than being a missing argument.")
  (input
    (do (def (count (.. (: xs (List Int64)))) (List.len xs)) (def (main) (count)) (export main)))
  (output (: 0 Int64)))

(case
  "a fixed parameter before a list-rest parameter binds positionally"
  (doc
    "`(def (tagged (: tag Int64) (.. (: xs (List Int64)))) (+ tag (List.len xs)))` has one FIXED
           parameter then a rest: `(tagged 100 7 8 9)` binds `tag = 100` positionally and gathers `[7, 8,
           9]` into `xs` (length 3), giving 103. A call with only the fixed argument, `(tagged 5)`, gathers
           the empty list, giving 5. Pins that leading fixed parameters bind positionally and the rest
           absorbs exactly the surplus.")
  (input
    (do
      (def (tagged (: tag Int64) (.. (: xs (List Int64)))) (+ tag (List.len xs)))
      (def (main (: n Int64)) (tagged n 7 8 9))
      (export main)))
  (call main (: 100 Int64))
  (output (: 103 Int64)))

(case
  "a list-rest argument of the wrong element type is rejected"
  (doc
    "The rest parameter is HOMOGENEOUS — every trailing argument shares the element type. `(count 1 \"x\"
           3)` mixes an `Int64` and a `String`, so gathering them into one list is a type error (the same
           CDZ0201 a heterogeneous list literal gets). Pins that a list-rest enforces one element type across
           all the arguments it gathers.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (count 1 "x" 3))
      (export main)))
  (error CDZ0201 (message "list elements must share one type")))

; --- Tuple-rest: an UNANNOTATED `(.. xs)` gathers HETEROGENEOUS args into a tuple -------------
; DESIGN-variable-arity-functions.md §3.2/§3.3: a rest parameter with NO `(List T)` annotation —
; a bare `(.. xs)` (or `(: xs Tuple)`) — gathers the trailing arguments into a TUPLE, preserving each
; argument's own type. Because a tuple's arity + element types are static, the call is monomorphized per
; call-site: `Tuple.size xs`, projections `(. xs i)`, and `Type.try-as` on an element all fold at compile
; time — so the body can branch on WHAT TYPES were passed. This is the heterogeneous companion of the
; homogeneous list-rest above.
(case
  "an unannotated rest parameter gathers a heterogeneous tuple whose size is known"
  (doc
    "`(def (describe (.. xs)) (Tuple.size xs))` gathers a MIXED-type argument run into a tuple:
           `(describe 1 \"two\" true)` binds `xs = (1, \"two\", true) : (Tuple Int64 String Bool)`, whose
           `Tuple.size` is 3. Pins that an unannotated rest is heterogeneous (each argument keeps its type,
           unlike the homogeneous list-rest) and its arity is observable.")
  (input
    (do (def (describe (.. xs)) (Tuple.size xs)) (def (main) (describe 1 "two" true)) (export main)))
  (output (: 3 Int64)))

(case
  "a tuple-rest function branches on the type of a passed argument at compile time"
  (doc
    "The payoff: a tuple-rest body inspects the ACTUAL types passed. `(Type.try-as (. xs 0) : Option
           Int64)` tests whether the first argument is an `Int64`; here `(kind \"hello\" sel)` passes a
           `String` first, so the `Int64` view is `None` and the `String` view is `Some`, yielding 2. The
           whole ladder folds at compile time (the element's type is static), so no runtime type tag is
           emitted. Pins tuple-rest + `Type.try-as` composing into compile-time type-dispatch.")
  (input
    (do
      (def
        (kind (.. xs))
        (match
          (: (Type.try-as (. xs 0)) (Option Int64))
          ((Some n) 1)
          ((None u) (match (: (Type.try-as (. xs 0)) (Option String)) ((Some s) 2) ((None u) 0)))))
      (def (main (: sel Int64)) (kind "hello" sel))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

; --- Rest-parameter placement: a rest must be LAST, and at most one -----------------------------
; DESIGN-variable-arity-functions.md §2.2: a rest parameter absorbs the trailing arguments, so it must
; be the LAST parameter (a fixed parameter after it could never receive an argument), and a parameter
; list may hold at most one. These are compile-time placement errors with an actionable message.
(case
  "a rest parameter that is not last is rejected"
  (doc
    "`(def (bad (.. xs) y) …)` places a fixed parameter `y` AFTER the rest — but the rest already
           absorbed every trailing argument, so `y` could never be bound. Rejected with a message naming
           the fix (move the rest to the end), rather than the confusing downstream arity error.")
  (input (do (def (bad (.. xs) (: y Int64)) y) (def (main) (bad 1 2 3)) (export main)))
  (error CDZ0201 (message "must be the LAST parameter")))

(case
  "at most one rest parameter is allowed"
  (doc
    "`(def (bad (.. xs) (.. ys)) …)` declares two rest parameters — but a single trailing run cannot be
           split between two rests, so at most one is allowed. Rejected with the coded placement error.")
  (input (do (def (two (.. xs) (.. ys)) 0) (def (main) (two 1 2)) (export main)))
  (error CDZ0201 (message "AT MOST ONE rest parameter")))

; --- Call-site SPLAT: `(.. t)` spreads a tuple's elements into a call's argument list -----------
; DESIGN-variable-arity-functions.md addendum A.2: at a CALL, a `(.. t)` argument whose operand is a
; compile-time-known TUPLE spreads that tuple's elements into the argument list positionally —
; `f(.. (a, b, c))` ≡ `f(a, b, c)`. The tuple's arity is static (so the argument positions are fixed),
; but its element VALUES may be runtime. It composes with inline arguments and with a rest parameter on
; the callee. (The dual of the rest PARAMETER: the callee gathers, the caller spreads.)
(case
  "a tuple splat spreads its elements into a call's arguments"
  (doc
    "`(add3 (.. (tuple 1 2 3)))` spreads the 3-tuple into `add3`'s three parameters — `a=1, b=2, c=3`
           — giving 6. Pins the basic call-site splat: a tuple argument prefixed `..` supplies several
           positional arguments at once.")
  (input
    (do
      (def (add3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
      (def (main) (add3 (.. #tuple(1 2 3))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a tuple splat composes with inline arguments"
  (doc
    "`(add3 1 (.. (tuple 2 3)))` supplies the first argument inline and spreads the remaining two from a
           tuple — `a=1, b=2, c=3` → 6. Pins that inline arguments and a splat interleave positionally.")
  (input
    (do
      (def (add3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
      (def (main) (add3 1 (.. #tuple(2 3))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a tuple splat's element values may be runtime"
  (doc
    "The tuple's ARITY is static (fixing the argument positions) but its VALUES need not be constant:
           `(add2 (.. (tuple n 10)))` for a boundary parameter `n` spreads `n` and `10` into `add2`,
           giving `n + 10` (5 → 15). Pins that a splat spreads positions statically while carrying runtime
           element values through.")
  (input
    (do
      (def (add2 (: a Int64) (: b Int64)) (+ a b))
      (def (main (: n Int64)) (add2 (.. #tuple(n 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a tuple splat feeds a rest parameter — the caller spreads, the callee gathers"
  (doc
    "The two halves compose: `(count (.. (tuple 1 2 3)))` spreads the tuple into three arguments, which
           `count`'s list-rest parameter then gathers back into `xs : List Int64` — length 3. Pins that
           call-site splat and a rest parameter are duals that combine.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (count (.. #tuple(1 2 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "an EMPTY tuple splat into a rest parameter gathers to the EMPTY list"
  (doc
    "The empty-splat + rest-parameter corner: `(count (.. #tuple()))` spreads ZERO arguments, which
           `count`'s list-rest parameter gathers into the EMPTY `xs : List Int64` — length 0. The two
           empty-splat mechanisms compose (spread-to-zero then gather-nothing). Edge-hunt companion to the
           sole-arg-into-nullary + amid-positionals cases, guarding the empty-splat family across the varargs
           gather path.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (count (.. #tuple())))
      (export main)))
  (output (: 0 Int64)))

(case
  "an empty tuple splat as the sole argument to a nullary fn spreads to zero args"
  (doc
    "An empty tuple splat `(.. #tuple())` contributes ZERO positional arguments, so `(f (.. #tuple()))`
           for a nullary `f` is exactly `(f)` — the nullary's value (7), NOT a one-argument application.
           Pins the empty-splat-into-nullary case breaker + v-lean-oracle flagged: the arity was counted on
           the PRE-spread splat FORM (1 arg → a false CDZ0201 `f takes no arguments, but 1 was applied`)
           rather than the POST-spread count (0). Fixed on the two paths that bypass `apply_lambda`'s
           expansion — the `None`-scheme arity fault (a nullary def resolves to its body VALUE) and the
           non-lambda zero-arg identity in lowering.")
  (input
    (do
      (def (f) 7)
      (def (main) (f (.. #tuple())))
      (export main)))
  (output (: 7 Int64)))

(case
  "an empty tuple splat amid positional arguments vanishes"
  (doc
    "The companion to the sole-argument case: an empty tuple splat BETWEEN positionals contributes zero
           args, so `(g 1 (.. #tuple()) 2)` is `(g 1 2)` = 12. This path already worked (it routes through
           `apply_lambda`'s splat expansion); pinned alongside the nullary fix as a regression guard that the
           empty-splat handling stays uniform across argument positions.")
  (input
    (do
      (def (g (: a Int64) (: b Int64)) (+ (* a 10) b))
      (def (main) (g 1 (.. #tuple()) 2))
      (export main)))
  (output (: 12 Int64)))

(case
  "an empty tuple splat into a nullary fn threads the RESULT TYPE, not just the value"
  (doc
    "The result-type companion (breaker #8487 residual): `(f (.. #tuple()))` must not only reduce to the
           nullary's VALUE but also INFER its RESULT TYPE from the reduced `(f)`'s body (Int64). Before the
           `apply_type` splat-expansion, the raw splat arg made the nullary application type `Any` — a bare
           unannotated result then declined at the boundary (`function return type has no machine
           representation`). Here the splat result is BOUND in a `let` and returned, so main's result type is
           threaded from the splat-application node (Int64), = 7, on all three backends.")
  (input
    (do
      (def (f) 7)
      (def (main) (let ((x (f (.. #tuple())))) x))
      (export main)))
  (output (: 7 Int64)))

(case
  "a tuple splat spreads a tuple held in a variable"
  (doc
    "The splat operand need not be a tuple LITERAL — a bound `let`-local holding a tuple splats the same
           way: `let t = (1, 2, 3) in add3(.. t)` reads `t`'s three elements into `add3`'s parameters,
           giving 6. Pins that `(.. t)` spreads a tuple VALUE by reference (each argument position reads one
           element), not only a syntactic tuple.")
  (input
    (do
      (def (add3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
      (def (main) (let ((t #tuple(1 2 3))) (add3 (.. t))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a list splat feeds a whole list into a list-rest parameter"
  (doc
    "A runtime LIST spread into a call `(.. xs)` supplies that list's elements as the trailing arguments,
           which a list-rest parameter gathers back into its own list: `let ys = [1,2,3,4] in count(.. ys)`
           binds `count`'s rest to `ys` (length 4). Pins the runtime-list splat's single-argument form — a
           whole list flows through the call into the rest parameter.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (let ((ys #list(1 2 3 4))) (count (.. ys))))
      (export main)))
  (output (: 4 Int64)))

(case
  "a list splat composes with inline arguments into a rest parameter"
  (doc
    "Inline arguments and a list splat interleave: `count(10, 20, .. ys)` with `ys = [1, 2, 3]` gathers
           `[10, 20, 1, 2, 3]` into the rest — length 5. Pins that a splat is not restricted to the sole
           argument; leading inline arguments and the spread combine in order.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (let ((ys #list(1 2 3))) (count 10 20 (.. ys))))
      (export main)))
  (output (: 5 Int64)))

(case
  "several list splats concatenate into a rest parameter"
  (doc
    "Multiple splats compose: `count(.. a, .. b)` with `a = [1, 2]` and `b = [3, 4, 5]` gathers the
           concatenation `[1, 2, 3, 4, 5]` into the rest — length 5. Pins that more than one spread in a
           call combines left-to-right.")
  (input
    (do
      (def (count (.. (: xs (List Int64)))) (List.len xs))
      (def (main) (let ((a #list(1 2)) (b #list(3 4 5))) (count (.. a) (.. b))))
      (export main)))
  (output (: 5 Int64)))

; --- Call-site splat of a function's OWN tuple PARAMETER (the "param-relay" case, A.7) --------
; DESIGN-variable-arity-functions.md A.7: a `(.. t)` splat whose operand is a tuple-typed PARAMETER of the
; enclosing function, into a FIXED-arity (non-varargs) callee — `(def (relay (: t (Tuple …))) (a3 (.. t)))`.
; The splat expands to per-slot projections `(a3 (. t 0) (. t 1) (. t 2))` at BOTH the type-check and the
; lowering (one shared expansion), so the tuple's elements bind `a3`'s three fixed parameters positionally.
; Distinct from the splat-of-a-LOCAL cases above: here the tuple arrives as an abstract parameter, and the
; relay is itself called with a concrete tuple — the reduced body's substituted (annotated) tuple operand
; splices the same way.
(case
  "a tuple splat relays a function's own tuple parameter into a fixed-arity call"
  (doc
    "`relay` takes a `(Tuple Int64 Int64 Int64)` parameter `t` and forwards it into the three-parameter
           `a3` by a call-site splat `(a3 (.. t))` — the elements bind `a3`'s `x`, `y`, `z` positionally, so
           `a3` sums them. `relay(#tuple(10 20 30))` = 10 + 20 + 30 = 60. Pins that a `(.. t)` over a tuple
           PARAMETER splats into fixed parameters (not only a local or a literal), the param-relay dual of
           the caller-spreads/callee-gathers case above.")
  (input
    (do
      (def (a3 x y z) (+ (+ x y) z))
      (def (relay (: t (Tuple Int64 Int64 Int64))) (a3 (.. t)))
      (def (main) (relay #tuple(10 20 30)))
      (export main)))
  (output (: 60 Int64)))

; --- Call-site splat of a COMPUTED tuple (an effect-free call operand), A.7b -------------------
; DESIGN-variable-arity-functions.md A.7b: a `(.. (mk …))` splat whose operand is a CALL that computes a
; tuple — not a bare reference — expands into per-slot projections too, PROVIDED the operand is effect-free
; (a pure function returns the same tuple on each projection's re-evaluation). An operand that performs an
; effect is NOT expanded (it would duplicate the effect) — that single-eval materialize is a further step.
(case
  "a call-site splat of a computed (effect-free) tuple spreads its elements"
  (doc
    "The splat operand is a CALL `(mk k)` that computes `#tuple(k, k+1, k+2)`, spread into the three
           parameters of `a3`. `(a3 (.. (mk k)))` = k + (k+1) + (k+2) = 3k + 3; at k = 10 that is 33. Pins
           that a computed-tuple operand (not only a literal, local, or parameter) splats when it is pure.")
  (input
    (do
      (def (a3 x y z) (+ (+ x y) z))
      (def (mk (: n Int64)) #tuple(n (+ n 1) (+ n 2)))
      (def (main (: k Int64)) (a3 (.. (mk k))))
      (export main)))
  (call main 10)
  (output (: 33 Int64)))

; A tuple relayed through a CHAIN of plain parameter hops before the splatting callee — the tuple's type
; is concrete at every hop, so the splat at the end expands the same way. `hop1` forwards its tuple param
; to `hop2` (a plain call, no splat), and `hop2` splats it into `add3`. This is the depth-2 companion of
; the param-relay case (breaker probe): the pass-through hop must not defeat the call-site expansion.
(case
  "a tuple relayed through an intermediate parameter hop still splats at the callee"
  (doc
    "`hop1(t)` forwards its `(Tuple Int64 Int64 Int64)` parameter to `hop2(t)` unchanged, and `hop2` splats
           it into the three-parameter `add3` via `(add3 (.. t))`. `add3(a b c) = a + 10b + 100c`, so
           `hop1(#tuple(5 n 1))` = 5 + 10n + 100; at n = 2 that is 125. Pins that an intermediate plain
           parameter hop before the splatting callee does not defeat the expansion (the tuple type stays
           concrete through the chain).")
  (input
    (do
      (def (add3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ (* 10 b) (* 100 c))))
      (def (hop2 (: t (Tuple Int64 Int64 Int64))) (add3 (.. t)))
      (def (hop1 (: t (Tuple Int64 Int64 Int64))) (hop2 t))
      (def (main (: n Int64)) (hop1 #tuple(5 n 1)))
      (export main)))
  (call main 2)
  (output (: 125 Int64)))

; --- Call-site splat INSIDE an effect-handler body (idealistic; pending handler-fold expansion) ---
; A call-site splat `(.. t)` is an ordinary call-argument construct, so it must expand the same way
; when the call happens to sit lexically inside a `handle` body. `one(.. #tuple(7))` = `one(7)` = 21,
; and the enclosing `handle T` discharges the (never-performed) `T` effect, so `main(n) = 21` for any
; `n` — identical to the non-splat `(one 7)`. This is the IDEALISTIC spec: the splat SHOULD spread.
; The compiler does not YET expand a splat inside a handler body — the tail-resumptive fold types and
; lowers its body through a path that bypasses the shared call-splat expansion — so it SOUNDLY DECLINES
; (CDZ0900 "handler not reducible") rather than miscompile. That earlier shipped a BROKEN artifact on
; both backends (invalid wasm / non-compiling Rust); the decline is the safe reject (fix f043df7c6e,
; PR #7826). This case is graded `todo` while it declines, GUARDS that the decline never regresses back
; to a miscompile (a wrong value / trap would grade `fail`), and auto-flips to `pass` when the follow-up
; threads the call-splat expansion through the handler fold. (breaker EMIT-INVALID probe, varargs A.7)
(case
  "a tuple splat inside an effect-handler body spreads its elements"
  (doc
    "`main(n)` runs `(one (.. #tuple(7)))` inside `(handle T n …)`. The splat spreads the one-tuple into
           `one`'s single parameter — `one(7) = 7 * 3 = 21` — and the handler discharges the unperformed
           `T` effect, so the result is `21` regardless of `n`. Pins that a call-site splat expands the
           same inside a handler body as anywhere else. Currently DECLINES (CDZ0900) pending the
           handler-fold splat expansion; a sound decline, never a miscompile.")
  (input
    (do
      (effect T (op tick (-> Int64)))
      (def (one (: a Int64)) (* a 3))
      (def (main (: n Int64)) (handle T n ((tick () s (resume s (+ s 1)))) (one (.. #tuple(7)))))
      (export main)))
  (call main 0)
  (output (: 21 Int64)))

; The EFFECTFUL-operand companion of the splat-in-handle TODO above (#7861 pins the literal face):
; when the expansion reaches handler bodies, an operand that PERFORMS must be evaluated exactly ONCE
; — the single tick draws the state (n), so the result is 3n, pinning no-double-eval from day one.
; Today the specializer declines the handle (CDZ0900, #7826 — the coded floor that replaced an
; accept-to-invalid-artifact; breaker adv 2026-09-02).
(case
  "an effectful-operand splat inside a handle body performs exactly once (should-work; today the specializer declines)"
  (input
    (do
      (effect T (op tick (-> Int64)))
      (def (one (: a Int64)) (* a 3))
      (def
        (main (: n Int64))
        (handle T n ((tick () s (resume s (+ s 1)))) (one (.. #tuple((T.tick))))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 12 Int64))
  (call main (: 10 Int64))
  (output (: 30 Int64)))
