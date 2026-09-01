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
