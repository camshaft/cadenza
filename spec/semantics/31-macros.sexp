; Macros — a macro is an ORDINARY FUNCTION over the AST (metaprogramming.md §Macros). No `defmacro`: a
; parameter marked `(quote p)` receives its argument UNEVALUATED, as the reflected `Ast` of the syntax
; written at the call site; the function returns an `Ast`; and the compiler EXPANDS the call — splices the
; returned syntax in the call's position and type-checks it as if written directly (§Expansion Precedes And
; Feeds The Core Guarantees). Expansion PRECEDES type checking (§Expansion Runs In Phases To A Fixpoint), so
; the call takes the EXPANSION's type, not the macro's declared `Ast` return. DESIGN-macro-system.md.
(case
  "a quote-parameter macro receives its argument as unevaluated Ast and returns it (identity expansion)"
  (doc
    "The minimal macro: `(def (q (quote x)) x)` — the `(quote x)` parameter binds the ARGUMENT'S SYNTAX
           (an `Ast`), and the body returns it. `(q 42)` reflects the literal `42` to its `Ast`, the macro
           returns that `Ast`, and the compiler SPLICES it back as the source `42`, which evaluates to `42 :
           Int64`. Pins that a macro is an ordinary function over the AST (metaprogramming.md §A Macro Is
           Dispatched By Binding), that a `quote` parameter is call-by-AST (§Expansion Operates On The
           Canonical Representation), and — crucially — that the CALL takes the EXPANSION's `Int64` type, not
           the macro's declared `Ast` return (expansion precedes type checking, §Expansion Runs In Phases).")
  (input (do (def (q (quote x)) x) (def (main) (q 42)) (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro builds new syntax from its argument via quasiquote and the expansion is type-checked directly"
  (doc
    "`(def (twice (quote x)) (quasiquote (+ (unquote x) (unquote x))))` builds the syntax `(+ x x)` with
           the argument's reflected AST spliced at each `(unquote x)`. `(twice 5)` expands to `(+ 5 5)` and
           evaluates to `10 : Int64` — the expansion is ordinary code, type-checked as if written directly
           (metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees). A macro-body literal and a
           spliced-argument literal must ground to the SAME `Int64` (no BigInt-vs-Int64 mismatch), so the
           `(+ 5 5)` type-checks — witnessing that expansion feeds ordinary inference cleanly.")
  (input
    (do
      (def (twice (quote x)) (quasiquote (+ (unquote x) (unquote x))))
      (def (main) (twice 5))
      (export main)))
  (call main)
  (output (: 10 Int64)))

(case
  "a macro mixing a body literal with an argument grounds both to Int64 (unless as a plain function)"
  (doc
    "The classic `unless`, as a plain function: `(def (unless (quote c) (quote body)) (quasiquote (if
           (unquote c) 0 (unquote body))))`. `(unless false 7)` expands to `(if false 0 7)` → `7`. Pins that
           a macro-BODY literal (`0`) and a spliced-ARGUMENT literal (`7`) both ground to `Int64` in the
           expansion (they are the two `if` branches — a BigInt-vs-Int64 grounding mismatch would reject
           CDZ0203), so a macro that mixes its own literals with the caller's arguments type-checks as
           ordinary code.")
  (input
    (do
      (def (unless (quote c) (quote body)) (quasiquote (if (unquote c) 0 (unquote body))))
      (def (main) (unless false 7))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a macro may INTRODUCE a local binding in its expansion and reference it within that expansion"
  (doc
    "An expansion is ordinary code, so it may DECLARE a new binding, not only splice into an expression
           (metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees). `(def (double-via-local
           (quote e)) (quasiquote (do (def t (unquote e)) (+ t t))))` builds `(do (def t E) (+ t t))` — a
           do-local `(def t …)` whose value is the spliced argument, referenced twice by the macro's OWN
           `(+ t t)`. `(double-via-local 21)` expands to `(do (def t 21) (+ t t))` and evaluates to `42 :
           Int64`. Pins that a macro-INTRODUCED binder resolves for a reference in the SAME expansion: the
           expander must SEED the spliced subtree's lexical scope so the do-local `def` binds `t` for the
           following `(+ t t)` — a binding introduced BY the macro (both binder and its references are
           macro-internal here, so no caller interaction / hygiene question arises). Without scope seeding
           for the spliced-in binder, the `t` references would spuriously unbind (CDZ0101).")
  (input
    (do
      (def (double-via-local (quote e)) (quasiquote (do (def t (unquote e)) (+ t t))))
      (def (main) (double-via-local 21))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro may introduce LET bindings in its expansion and reference them (a distinct binder form)"
  (doc
    "The macro-introduced binding need not be a do-local `def` — a `let` bindings-list works the same
           (metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees). `(def (let2 (quote a)
           (quote b)) (quasiquote (let ((u (unquote a)) (v (unquote b))) (+ u v))))` builds `(let ((u A) (v
           B)) (+ u v))` — TWO let binders whose inits are the spliced arguments, both referenced by the
           macro's own `(+ u v)`. `(let2 30 12)` expands to `(let ((u 30) (v 12)) (+ u v))` and evaluates to
           `42 : Int64`. Pins that the expander seeds the spliced subtree's scope for a `let` bindings-list
           (a DIFFERENT binder-candidate shape than a do-local `def`), so both introduced binders resolve
           for the macro's own references — the binders are macro-internal (no caller interaction / hygiene
           question). Without scope seeding for the spliced-in `let`, the `u`/`v` references would unbind
           (CDZ0101).")
  (input
    (do
      (def (let2 (quote a) (quote b)) (quasiquote (let ((u (unquote a)) (v (unquote b))) (+ u v))))
      (def (main) (let2 30 12))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a macro-introduced binder does NOT capture a caller identifier of the same name (hygiene, do-local def)"
  (doc
    "Macros are HYGIENIC — expansion PRESERVES the caller's bindings (metaprogramming.md §Macros Are
           Hygienic): a name a macro INTRODUCES in its expansion must not capture a same-named identifier
           the CALLER passed in. `(def (capture (quote body)) (quasiquote (do (def x 100) (unquote body))))`
           introduces a do-local `x`; `(capture x)` at a call site where the caller has its OWN `(def x 1)`
           passes the caller's `x` as `body`. Hygienically the caller's `x` still denotes the caller's `1`
           — the macro's introduced `x` is alpha-renamed so it does NOT shadow the spliced argument — so
           `(capture x)` evaluates to `1 : Int64`, NOT `100`. Pins capture-avoidance for a macro-introduced
           do-local `def` binder (a naive splice would produce `(do (def x 100) (+ 0 x))` reading the
           macro's `x` and wrongly yield 100). The `(+ 0 …)` wrapper keeps the caller's argument off the
           do's bare tail (an ML-surface round-trip detail) without changing the hygiene it witnesses.")
  (input
    (do
      (def (capture (quote body)) (quasiquote (do (def x 100) (+ 0 (unquote body)))))
      (def (main) (do (def x 1) (capture x)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a macro-introduced LET binder does NOT capture a caller identifier of the same name (hygiene)"
  (doc
    "Hygiene applies to every binder form, not only a do-local `def` (metaprogramming.md §Macros Are
           Hygienic). `(def (wrap (quote body)) (quasiquote (let ((x 100)) (unquote body))))` introduces a
           `let`-bound `x`; `(wrap x)` where the caller has its own `(def x 1)` passes the caller's `x` as
           `body`. The macro's `let`-bound `x` is alpha-renamed so it does not shadow the spliced argument,
           so the caller's `x` still denotes `1` and `(wrap x)` evaluates to `1 : Int64`, not `100`. Pins
           capture-avoidance for a macro-introduced `let` binder (parallel to the do-local `def` case).")
  (input
    (do
      (def (wrap (quote body)) (quasiquote (let ((x 100)) (unquote body))))
      (def (main) (do (def x 1) (wrap x)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a macro may introduce a HELPER FUNCTION and its parameter resolves in the function's body"
  (doc
    "An expansion may declare a nested FUNCTION, not only a value binding (metaprogramming.md §Expansion
           Precedes And Feeds The Core Guarantees). `(def (via-fn (quote arg)) (quasiquote (do (def (helper
           p) (+ p 100)) (helper (unquote arg)))))` introduces a do-local function `helper` whose body
           `(+ p 100)` references its OWN parameter `p`, then calls it with the spliced caller argument.
           `(via-fn 5)` expands to `(do (def (helper p) (+ p 100)) (helper 5))` and evaluates to `105 :
           Int64`. Pins that a macro-introduced function-def's PARAMETER resolves in its body: the expander
           seeds the spliced signature's parameter scope (a macro-introduced `(def (f p…) …)` is past the
           load-time binder index, so the resolver falls back to a live parameter scan) — otherwise the
           body's `p` would spuriously unbind (CDZ0101). The caller's argument is spliced at the CALL site
           (`(helper 5)`), evaluated in the caller's scope, so no capture question arises.")
  (input
    (do
      (def (via-fn (quote arg)) (quasiquote (do (def (helper p) (+ p 100)) (helper (unquote arg)))))
      (def (main) (via-fn 5))
      (export main)))
  (call main)
  (output (: 105 Int64)))

(case
  "a macro may introduce a RECURSIVE helper function and its recursive self-call lowers + its parameter is inferred"
  (doc
    "The recursive twin of the helper case above (metaprogramming.md §Expansion Precedes And Feeds The
           Core Guarantees): a macro introduces a do-local `(def (fact p) (if (> p 1) (* p (fact (- p 1)))
           1))` — a RECURSIVE function — then calls it with the spliced caller argument. `(via-fn 5)`
           expands to `(do (def (fact p) …) (fact 5))` and MUST evaluate to `120 : Int64`, exactly as the
           post-expansion-equivalent hand-written `(do (def (fact p) …) (fact 5))` does. Post-expansion
           equivalence is the core macro guarantee: expanded AST is compiled exactly as if written directly.
           Pins TWO mechanisms a macro-spliced recursive def needs that a load-time def gets for free — the
           spliced def is a FRESH post-load body absent from the load-time indexes:
             (1) the recursive self-call `(fact (- p 1))` must lower to a `Core::Call`, which needs the
                 spliced def registered as a callable (`register_reduced_callables` in `expand_macros`) —
                 else `callee_def_index` misses it and it declines CDZ0900 'needs runtime specialization';
             (2) the parameter `p` must be INFERRED (`solve_recursive_params`, A2), which needs `p`'s
                 signature occurrence to resolve as a `Resolved::Param` — that goes through
                 `resolve::is_param_occurrence`, which walks `parent_of`, so the spliced subtree's PARENT
                 index must be populated (`expand_macros` rebuilds it after the splice) — else `p` resolves
                 as an unbound `Poison`, `type_of`=`Any`, the solve never fires, and the recursive call
                 declines 'a parameter whose type could not be inferred'.
           (Was breaker-fenced `mrf1`; fixed by the two mechanisms above, both in `expand_macros`.)")
  (input
    (do
      (def
        (via-fn (quote arg))
        (quasiquote (do (def (fact p) (if (> p 1) (* p (fact (- p 1))) 1)) (fact (unquote arg)))))
      (def (main) (via-fn 5))
      (export main)))
  (call main)
  (output (: 120 Int64)))

(case
  "a NESTED macro expansion (macro whose output calls another macro) introduces a recursive helper that lowers + infers"
  (doc
    "The multi-ROUND twin of the recursive-helper case above: expansion runs to a FIXPOINT
           (metaprogramming.md §Expansion Runs In Phases To A Fixpoint), so a macro whose output CONTAINS
           ANOTHER macro call is expanded in a LATER round. Here `wrap` expands to `(+ 0 (mkrec x))`, then
           `mkrec` expands (a second round) to a do-local RECURSIVE `(def (fib n) …)` + `(fib x)`. The
           inner recursive helper must STILL lower (its self-call → `Core::Call`) and infer its param,
           exactly as a first-round recursive helper does — `(wrap 10)` must evaluate to `55 : Int64`
           (`fib 10`). Pins that the reduced-callable registration (`register_reduced_callables`) reaches a
           def spliced in a NON-FIRST round: the round-1 splice marks the (then plain) `(mkrec x)` call
           node walked, so the round-2 re-splice into a do-block must UN-MARK that node before the walk,
           else `collect_reduced_callables` returns early and the inner `fib` is never registered →
           CDZ0900 on its self-call. (mrf1 nested-macro follow-up.)")
  (input
    (do
      (def
        (mkrec (quote a))
        (quasiquote
          (do (def (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))) (fib (unquote a)))))
      (def (wrap (quote x)) (quasiquote (+ 0 (mkrec (unquote x)))))
      (def (main) (wrap 10))
      (export main)))
  (call main)
  (output (: 55 Int64)))

(case
  "a macro-introduced REFERENCE resolves in the macro's definition scope, not captured by a caller binder (hygiene dir-2)"
  (doc
    "Direction-2 hygiene (metaprogramming.md §Macros Are Hygienic): a name a macro INTRODUCES as a
           REFERENCE MUST NOT be CAPTURED by a same-named binder at the use site — the DUAL of the binder
           cases above (which pin that an introduced BINDER does not capture a caller reference). `(def g
           100)` binds `g` in the macro's DEFINITION scope; `(def (m (quote body)) (quasiquote (+ g (unquote
           body))))` references that `g`. The caller `(do (def g 1) (m 5))` has its OWN `g = 1`. Hygienically
           the macro's introduced `g` denotes the definition-scope `100`, NOT the caller's `1`, so `(m 5)`
           references the definition-site `g` and evaluates to `105 : Int64` (`(+ 100 5)`), not the captured
           `6` (`(+ 1 5)`). SHOULD-WORK, tracked known-fail: the reference-side dual is not yet implemented — a
           macro-introduced free reference is reified as bare syntax and re-resolves at the use site, so it is
           currently CAPTURED (value 6, a silent hygiene miscompile violating §Macros Are Hygienic). Pinned as
           a tracked known-fail (baseline `fail`, non-redding) until the definition-scope-capture mechanism
           lands (owner v-metaprogramming; overlaps the closure-capture work), then re-pinned pass.")
  (input
    (do
      (def g 100)
      (def (m (quote body)) (quasiquote (+ g (unquote body))))
      (def (main) (do (def g 1) (m 5)))
      (export main)))
  (call main)
  (output (: 105 Int64)))

; ── breaker: the post-expansion-equivalence gap. The identical do-local recursive fn compiles and
; computes 120 when written PLAINLY (09/02 pin the plain form), but the macro-introduced expansion of
; the SAME shape declines CDZ0900 "a recursive function needs runtime specialization" — the expander's
; live-parameter-scan fallback (#7529) doesn't feed the recursion machinery the way a load-time def
; does. metaprogramming.md §Expansion Precedes And Feeds The Core Guarantees says the expanded program
; is an ordinary program; this pins the should-work value and tracks the gap (v-metaprogramming).
(case
  "mrf1 a macro-introduced RECURSIVE helper computes like its plainly-written twin (should-work: expansion precedes the core guarantees)"
  (doc
    "`(via-rec 5)` expands to `(do (def (fact p) (if (> p 1) (* p (fact (- p 1))) 1)) (fact 5))` —
           byte-for-byte the plain do-local recursive fn that compiles and computes 120 when written
           directly. Post-expansion equivalence (metaprogramming.md §Expansion Precedes And Feeds The
           Core Guarantees) says the macro-introduced twin MUST behave identically. Today the expansion
           declines CDZ0900 (runtime-specialization not-yet) — the #7529 parameter-scope fallback covers
           the parameter but not the fn's own recursive self-reference. MUST be 120.")
  (input
    (do
      (def
        (via-rec (quote arg))
        (quasiquote (do (def (fact p) (if (> p 1) (* p (fact (- p 1))) 1)) (fact (unquote arg)))))
      (def (main) (via-rec 5))
      (export main)))
  (call main)
  (output (: 120 Int64)))

(case
  "eval sees through macro expansion — a macro producing a (quote …) evals as the plain form"
  (doc
    "`eval` and macro expansion are ONE compile-time tier (metaprogramming.md §Compile-Time Evaluation Is
           One Tier), so `(eval (MACRO …))` whose expansion is a `(quote …)` must behave as the plain
           `(eval (quote …))`. `(def (mk-quoted (quote e)) (quasiquote (quote (unquote e))))` expands
           `(mk-quoted (+ 2 3))` to `(quote (+ 2 3))`; `(eval (mk-quoted (+ 2 3)))` therefore evals `(+ 2 3)`
           to `5 : Int64` — byte-identical to the plain `(eval (quote (+ 2 3)))`. Pins that eval SEES THROUGH
           expansion: the quote/eval reconstruction is re-run over the EXPANDED program (the load-time passes
           precede expansion), so an eval-argument that only became a visible `(quote …)` via a macro is
           reconstructed + folded, not rejected CDZ0101 (a post-expansion-equivalence guarantee).")
  (input
    (do
      (def (mk-quoted (quote e)) (quasiquote (quote (unquote e))))
      (def (main) (eval (mk-quoted (+ 2 3))))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a non-terminating self-recursive macro is a hard CDZ0999 error, not a compiler hang"
  (doc
    "A macro whose expansion keeps producing another call to itself — `(def (m (quote e)) (quasiquote (m
           (unquote e))))` with `(m 5)` — has no fixpoint. The expander caps the expansion fuel and reports
           a hard CDZ0999 (the compile-time-reduction-bound band — macro expansion is compile-time
           evaluation) instead of LOOPING FOREVER, which hung the compiler AND `cdz check` (freezing the
           LSP / diagnostics-as-you-type on one accidental self-referential macro edit). Pins that a
           non-terminating macro is a DIAGNOSABLE error, not a hang — a robustness guarantee for the tool.")
  (input (do (def (m (quote e)) (quasiquote (m (unquote e)))) (def (main) (m 5)) (export main)))
  (error CDZ0999 (message "macro expansion did not terminate")))

(case
  "a macro wrapping the spliced body in a HANDLE keeps the caller's names in scope"
  (doc
    "A macro may WRAP the caller's expression in a handler and still see the caller's bindings — the
           with-default / with-handler macro class (metaprogramming.md §Expansion Precedes And Feeds The
           Core Guarantees). `(def (wrap (quote body)) (quasiquote (handle E 0 ((bail () s 99)) (unquote
           body))))` wraps the spliced body in a `handle`; `(wrap (+ n 5))` where the caller has `(def n 3)`
           expands to `(handle E 0 ((bail () s 99)) (+ n 3+…))` and the spliced `(+ n 5)` still resolves the
           caller's `n` → `8 : Int64`. Pins that (a) a macro-produced handle's nullary arm's EMPTY parameter
           list `()` reconstructs as an empty list (not a `(trap \"malformed AST\")`, which mis-read the arm
           CDZ0201), and (b) the spliced body inside the handle keeps caller scope (no spurious unbound).")
  (input
    (do
      (effect E (op bail (-> Int64)))
      (def (wrap (quote body)) (quasiquote (handle E 0 ((bail () s 99)) (unquote body))))
      (def (main) (do (def n 3) (wrap (+ n 5))))
      (export main)))
  (call main)
  (output (: 8 Int64)))

(case
  "a macro-introduced MATCH resolves its arm-pattern binders in the arm body (accessor macros)"
  (doc
    "The bread-and-butter accessor-macro class: a macro whose expansion is a `match` whose arm PATTERN
           binds names its body uses. `(def (fst (quote e)) (quasiquote (match (unquote e) (#tuple(a _b)
           a))))` expands `(fst #tuple(7 9))` to `(match #tuple(7 9) (#tuple(a _b) a))` → `7 : Int64`. Pins
           that a macro-introduced match arm's pattern binder (`a`) resolves in the arm body: the expander
           seeds the spliced subtree's scope AFTER rebuilding the parent index, so the arm — a headless
           `(pattern body)` recognized as a binding scope only via its `match` PARENT — is a candidate and
           its body binder resolves (without the after-parent ordering, the arm-body `a` unbinds CDZ0101).")
  (input
    (do
      (def (fst (quote e)) (quasiquote (match (unquote e) (#tuple(a _b) a))))
      (def (main) (fst #tuple(7 9)))
      (export main)))
  (call main)
  (output (: 7 Int64)))
