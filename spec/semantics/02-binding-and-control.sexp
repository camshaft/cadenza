; Binding, scope, and control flow — witnesses core-semantics.md. Cases are s-expressions
; in the canonical homoiconic representation (options/code-shape/); a result is (: <value> <Type>),
; a rejected program records its diagnostic code (options/diagnostics-schema/), a runtime halt
; records a trap. See README.md for the case vocabulary.

(case "a let binding is in scope in its body"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical — a name resolves to its enclosing binding.")
  (input  (let ((x 10)) x))
  (output (: 10 Int64)))

(case "a name resolves to the nearest enclosing binding"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical.")
  (input  (let ((x 1)) (let ((x 2)) x)))
  (output (: 2 Int64)))

(case "an inner binding shadows an outer one only within its scope"
  (doc    "Witnesses core-semantics.md #Shadowing Is Well-Defined (which defers to the corpus):
           the inner x is 2 inside its let; the outer x is still 1 outside it, so the sum is 3.")
  (input  (+ (let ((x 2)) x) (let ((x 1)) x)))
  (output (: 3 Int64)))

; A `let` may shadow a FUNCTION PARAMETER with a value of a DIFFERENT TYPE, and the inner binding's type
; governs its references (core-semantics.md #Shadowing Is Well-Defined: a shadowing binding takes effect
; for references in its scope). `(def (f x) (let ((x true)) x))` binds parameter `x` (used at type Int64
; by the call `(f 99)`) and then shadows it with `x = true` (Bool); the body returns the inner `x`, so
; `f` returns the Bool `true` regardless of its argument. The shadow is well-defined and the program is
; well-typed — the different-name analogue `(def (f x) (let ((y true)) y))` returns `true`, and a
; non-parameter nested shadow `(let ((x 99)) (let ((x true)) x))` returns `true`. A compiler that reuses
; the parameter's local SLOT (typed for the parameter, e.g. i64 for an Int64 argument) for the shadowing
; binding's differently-typed value emits a component that fails wasm validation — an invalid component,
; the worst outcome (self-hosting-and-bootstrap.md #An Unsupported Construct Is Declined, Not Miscompiled:
; a not-yet-handled construct MUST decline, never emit an invalid or divergent component). A generation
; that cannot yet allocate a fresh slot for a differently-typed shadow of a parameter declines rather than
; emitting an invalid component.

(case "a let shadowing a parameter with a differently-typed value is not an invalid component"
  (doc    "`(def (f x) (let ((x true)) x))` shadows the Int64 parameter `x` with the Bool `x = true`; the
           body returns the inner `x`, so `(f 99)` = `true`. The shadow is well-defined (core-semantics.md
           #Shadowing Is Well-Defined) and the program is well-typed — the different-name form `(let ((y
           true)) y)` and the non-parameter nested shadow both return `true`. The compiler MUST compute
           `true` or DECLINE, never emit a component that fails wasm validation by reusing the parameter's
           local slot for the differently-typed shadow. Pins that a differently-typed shadow of a parameter
           gets its own slot rather than colliding with the parameter's, so the result is a valid component
           (the inline and different-name shadows already work; this is the same-name parameter-shadow
           case). A generation that cannot yet do so declines rather than emitting an invalid component.")
  (input  (do
            (def (f x) (let ((x true)) x))
            (def (main) (f 99)) (export main)))
  (output (: true Bool)))

(case "a let shadowing a parameter with a same-typed value runs, not miscompiles"
  (doc    "The same-type companion of the differently-typed shadow above: `(def (f x) (let ((x 7)) x))`
           shadows the Int64 parameter `x` with another Int64 `x = 7`, so `(f 99)` = 7. Distinct from
           the Bool shadow because the types AGREE, yet it exercises the same binder-substitution hazard:
           when a function is inlined, β-reduction must NOT substitute the argument into the let's BINDER
           occurrence `x` (which resolves up to the same-named parameter). A generation that did so turned
           the binding into `(99 7)` — losing the name — so the body's `x` found no binding; here it
           additionally reused the parameter's slot, an outcome that MISCOMPILED to an invalid component.
           A binder names a binding and is copied, never substituted, so the inner `7` is returned.")
  (input  (do
            (def (f x) (let ((x 7)) x))
            (def (main) (f 99)) (export main)))
  (output (: 7 Int64)))

(case "a match-arm binder shadowing a parameter binds the scrutinee, not the argument"
  (doc    "A match-arm PATTERN binder is a binding site, like a let binder: `(def (f x) (match 5 (x x)))`
           binds `x` to the scrutinee 5 for the arm's scope (core-semantics.md #Bindings Introduced By A
           Pattern Are Scoped To Its Branch), shadowing the parameter `x`; `(f 99)` = 5. When `f` inlines,
           β-reduction must copy the arm's binder occurrence `x` rather than substitute the argument for
           it (the binder resolves up to the same-named param) — else the arm binds nothing and the body's
           `x` is spuriously unbound. Pins that binder protection covers match-arm patterns, not only let
           bindings.")
  (input  (do
            (def (f x) (match 5 (x x)))
            (def (main) (f 99)) (export main)))
  (output (: 5 Int64)))

(case "a let shadowing a parameter whose initializer references that parameter computes"
  (doc    "The demanding shadow: the shadowing binding's INITIALIZER references the shadowed parameter.
           `(def (f x) (let ((x (+ x 1))) (* x 2)))` — the initializer `(+ x 1)` is written before the
           new `x` binding takes effect, so its `x` is the PARAMETER (core-semantics.md:53: an
           initializer observes the bindings written before it, not the one it introduces); the body's
           `(* x 2)` then reads the new local. `(f 20)` = (20+1)*2 = 42. This combines the two β-reduce
           hazards: the binder occurrence `x` must be copied not substituted (else the binding name is
           lost), AND the initializer's `x` reference must still be substituted with the argument (it IS
           a value reference to the param). A generation that lost the parameter binding when the local
           shared its name rejected CDZ0101 'unbound name `x`'. The different-name form (`(let ((y (+ x
           1))) …)`) and the let-over-let form both worked — only a same-name PARAM shadow broke.")
  (input  (do
            (def (f x) (let ((x (+ x 1))) (* x 2)))
            (def (main (: n Int64)) (f n))
            (export main)))
  (call   main (: 20 Int64))
  (output (: 42 Int64)))

(case "a param-shadowing let with a param-referencing initializer folds at a constant argument"
  (doc    "The constant-argument companion of the case above: `(f 20)` folds to 42 the same way, so the
           fix is not specific to a runtime argument — the parameter binding survives β-reduction for
           the initializer whether the argument is constant or runtime. Pins the fold path of the
           binder-copy / reference-substitute split.")
  (input  (do
            (def (f x) (let ((x (+ x 1))) (* x 2)))
            (def (main) (f 20)) (export main)))
  (output (: 42 Int64)))

(case "a let binding whose value references a parameter compiles under a call"
  (doc    "`(def (g n) (let ((x (+ n 1))) (+ x x)))` — the `let` value USES the parameter `n` (not a
           shadow). Calling `(g 10)` inlines g's body; the reduction must substitute `n`→`10` in the
           binding's initializer AND keep the body's references to the binding pointing at that
           substituted initializer. `x = 10+1 = 11`, so `(+ x x)` = 22. Pins that β-reduction copies a
           `let` inside a called function consistently — the body's binding references resolve to the
           COPY's substituted initializer, not the original (a name occurrence carried through a copy must
           re-resolve against the copied scope). A generation that shared the original unsubstituted
           initializer would surface an unsubstituted parameter with no local slot.")
  (input  (do
            (def (g n) (let ((x (+ n 1))) (+ x x)))
            (def (main) (g 10)) (export main)))
  (output (: 22 Int64)))

(case "a nested if on the same condition collapses the inner test to the known branch"
  (doc    "core-semantics.md #Conditionals Evaluate One Branch: inside the ELSE of `(if c … …)` the
           condition `c` is known false, so a nested `(if c B D)` there always takes `D`. `(if c 1 (if c 2
           3))` therefore never yields 2: `c` = true → 1, `c` = false → the outer else, where the inner `c`
           is false → 3. A compiler that constant-propagates the outer condition into the nested test folds
           the inner `if` away to `D`; this pins the observable result of that propagation is the same as
           re-evaluating `c` — the inner branch `2` is dead.")
  (input  (do
            (def (main (: c Bool)) (if c 1 (if c 2 3)))
            (export main)))
  (call   main (: false Bool))
  (output (: 3 Int64)))

(case "conditional propagation respects a shadowing rebind of the condition variable"
  (doc    "The propagation must track the condition's VALUE in scope, not match its text: `(let ((c (< n
           5))) (if c 1 (let ((c true)) (if c 2 3))))` with n = 10 has the OUTER `c` = false (10 < 5 is
           false), so the outer `if` takes its else; there the INNER `c` is a fresh binding = true, so the
           inner `if` takes 2. The two `c`s are textually identical but denote different values — a
           propagation that folded the inner `(if c …)` to the outer `c`'s known-false value would wrongly
           yield 3. Pins that the constant propagation is scope-aware (it stops at a rebinding of the
           condition name), the control-flow analogue of the lexical-shadowing binding rule.")
  (input  (do
            (def (main (: n Int64))
              (let ((c (< n 5))) (if c 1 (let ((c true)) (if c 2 3)))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 2 Int64)))

; A lexical binding wins in APPLICATION-HEAD position too, not only value position — even when its
; name coincides with a built-in constructor form like `list`/`tuple`/`record`/`map`. core-semantics.md
; #Binding Is Lexical: "A name MUST resolve to the nearest enclosing binding of that name." A `let`
; binding named `list` shadows the built-in list constructor for the extent of its scope, so `(list 3 4)`
; in its body applies the bound function (the nearest binding), yielding 7 — not the built-in list value
; `(list 3 4)`. A compiler that dispatches an application by matching the head STRING against its built-in
; forms before consulting the environment never sees the shadowing binding in head position: it resolves
; a bare `list` reference to the binding (value position) but silently prefers the built-in when `list`
; heads an application — resolving one name two ways by syntactic position, which #Binding Is Lexical
; forbids. (The value-position companion — `(let ((list 99)) list)` = 99 — already holds; this pins the
; head-position half.)

(case "a let binding shadows a built-in constructor name in application-head position"
  (doc    "`(let ((list (fn (a b) (+ a b)))) (list 3 4))` binds `list` to a function, then applies it in
           head position: `list` MUST resolve to the nearest enclosing binding (core-semantics.md #Binding
           Is Lexical), so `(list 3 4)` = 7, NOT the built-in list value `(list 3 4)`. Pins that
           application-head resolution consults the lexical environment before the built-in constructor
           forms — a compiler matching the head name `list` against its built-ins first ignores the
           shadowing binding and builds a two-element list, resolving `list` to the binding in value
           position but to the built-in in head position (the same name, two ways). A generation that does
           not realize a shadowing built-in name declines rather than choosing the built-in.")
  (input  (let ((list (fn (a b) (+ a b)))) (list 3 4)))
  (output (: 7 Int64)))

; The SAME head-position shadowing holds for `tuple` and `record` — the compound-VALUE constructors.
; They are ordinary shadowable names bound in the prelude (aliases for the primitive symbol
; constructors `(,)` and `{}`, which a program cannot spell), so a `let`/`def`/parameter binding of
; `tuple`/`record` shadows the built-in exactly as one of `list` does: `(tuple 3 4)` in the binding's
; scope applies the bound function, yielding 7 — NOT the built-in tuple value `(tuple 3 4)`. A compiler
; that dispatches a `tuple`/`record` head STRUCTURALLY (matching the head name before consulting the
; environment) resolves the name two ways by syntactic position — the built-in in head position, the
; binding in value position — which #Binding Is Lexical forbids. (core-semantics.md §A Compound Value
; Has A Symbol Constructor And A Shadowable Alias: the name is looked up like any other; the primitive
; is the symbol.)

(case "a let binding shadows the tuple constructor in application-head position"
  (doc    "The `tuple` sibling of the recorded `list` head-position shadow: `(let ((tuple (fn (a b) (+ a
           b)))) (tuple 3 4))` applies the nearest binding, yielding 7 — not the built-in tuple value
           `(tuple 3 4)`. `tuple` is a shadowable prelude alias for the primitive symbol constructor
           `(,)`, so a binding named `tuple` shadows it; head-position resolution consults the lexical
           environment first. Earlier the seed answered `(tuple 3 4)` — the structural grammar dispatch
           on the head name won over the binding (a wrong value, the one-name-two-resolutions bug).")
  (input  (do (def (main) (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4))) (export main)))
  (output (: 7 Int64)))

(case "a let binding shadows the record constructor in application-head position"
  (doc    "The `record` sibling: `(let ((record (fn (a b) (+ a b)))) (record 3 4))` applies the bound
           function in its scope, yielding 7 — `record` is a shadowable prelude alias for the primitive
           symbol constructor `{}`. Earlier the seed instead REJECTED with CDZ0201 'record field must be
           (key value)' — the built-in record form's shape check fired on an application of a lexically
           bound function: a spurious rejection of a well-typed program, the same head-vs-value split.")
  (input  (do (def (main) (let ((record (fn (a b) (+ a b)))) (record 3 4))) (export main)))
  (output (: 7 Int64)))

(case "a parameter named tuple is applied as the bound function"
  (doc    "The parameter companion: `(def (f tuple) (tuple 3 4))` — the formal `tuple` is the nearest
           binding, so applying it calls the argument function. `(f (fn (a b) (* a b)))` = 12. Pins that
           a parameter shadows the `tuple` alias exactly as a `let` binding does — the name resolves to
           the parameter in head position, not the built-in constructor.")
  (input  (do (def (f tuple) (tuple 3 4)) (def (main) (f (fn (a b) (* a b)))) (export main)))
  (output (: 12 Int64)))

(case "a shadowed-constructor application types at the binding's return type"
  (doc    "The head-position misresolution was a TYPE-soundness bug too: the shadowing binding returns
           Int64, so `(+ (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4)) 1)` = (3+4)+1 = 8. Earlier the
           seed REJECTED with CDZ0203 'cannot unify Int64 with (Tuple Int64 Int64)' — inference resolved
           the head to the built-in tuple constructor, typing the application as a Tuple, so the outer
           `+ … 1` failed to unify. Resolving the head to the lexical binding fixes the value AND the
           type: the same name no longer has two types by syntactic position.")
  (input  (do (def (main) (+ (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4)) 1)) (export main)))
  (output (: 8 Int64)))

; --- The bindings of one `let` take effect in order (let*, not parallel) --------------------
; core-semantics.md #The Bindings Of One `let` Take Effect In Order: each binding's initializer sees
; the bindings written before it in the SAME let, so `(let ((x 1) (y (+ x 1))) y)` is 2 — `y`'s
; initializer observes `x`. Under a PARALLEL reading `y`'s initializer would evaluate in the enclosing
; scope where `x` is unbound (a CDZ0101 rejection); the sequential reading, which the seed realizes,
; is the recorded oracle.

(case "a later let binding sees an earlier one in the same let"
  (doc    "`(let ((x 1) (y (+ x 1))) y)` = 2: the second binding's initializer `(+ x 1)` observes the
           first binding `x`, so the bindings of one `let` take effect in order (core-semantics.md
           #The Bindings Of One `let` Take Effect In Order), not in parallel where `x` would be unbound
           in `y`'s initializer.")
  (input  (let ((x 1) (y (+ x 1))) y))
  (output (: 2 Int64)))

(case "a repeated let binding shadows the earlier one for what follows"
  (doc    "`(let ((x 1) (x (+ x 10))) x)` = 11: the second binding of `x` shadows the first for the
           initializers and body that follow, and its initializer `(+ x 10)` sees the first `x` = 1
           (core-semantics.md #The Bindings Of One `let` Take Effect In Order + #Shadowing Is
           Well-Defined). The sequential companion of the case above at a repeated name.")
  (input  (let ((x 1) (x (+ x 10))) x))
  (output (: 11 Int64)))

(case "a nested-let chain that reuses each binding folds to one value"
  (doc    "Each `let` binding is referenced TWICE by the next, ten deep: `a = 1+1`, `b = a+a`, …,
           result `j+j`. Every binding is used more than once, so a compiler that re-evaluates a
           binding's initializer on each reference does exponential (2^depth) work; folding each
           binding ONCE and reusing its value is linear. `(+ j j)` = 2·2^10 = 2048. Pins that a
           `let` binding denotes a single value shared by all its references (core-semantics.md
           #The Bindings Of One `let` Take Effect In Order) — the same value whether read once or
           ten times — so the answer is independent of how the compiler memoizes the fold. (The
           observable is the value; the doubling structure is what makes a non-memoizing fold blow
           up, so this doubles as a compile-time-cost regression guard.)")
  (input  (let ((a (+ 1 1)))
          (let ((b (+ a a)))
          (let ((c (+ b b)))
          (let ((d (+ c c)))
          (let ((e (+ d d)))
          (let ((f (+ e e)))
          (let ((g (+ f f)))
          (let ((h (+ g g)))
          (let ((i (+ h h)))
          (let ((j (+ i i)))
            (+ j j))))))))))))
  (output (: 2048 Int64)))

(case "a deep chain of runtime-list let-bindings compiles and returns the final length"
  (doc    "The RUNTIME (heap-valued) companion of the fold above: twelve nested `let`s, each binding a
           runtime `list` grown from the previous by `List.push`, ending in `(List.len l12)` = 12. Each
           binding is a genuine value-heap handle (not a compile-time constant), so it is materialized
           as a real local — but the compiler captures the enclosing scope at each `let` for name
           resolution, and if that capture DEEP-CLONES the environment, the nested captures nest
           ~2^depth copies and compilation blows its memory (the 'compile is 2ⁿ in `let` nesting'
           ceiling). Sharing the captured environment makes the cost linear in depth. Pins that a deep
           chain of runtime-compound `let`s compiles at all (and to the right value) — the shape a
           compiler's threaded state / accumulator passes take. The observable is 12; the DEPTH is the
           compile-time-cost regression guard (this depth exhausted memory before the fix).")
  (input  (let ((l1  (List.push (list) 1)))
          (let ((l2  (List.push l1 2)))
          (let ((l3  (List.push l2 3)))
          (let ((l4  (List.push l3 4)))
          (let ((l5  (List.push l4 5)))
          (let ((l6  (List.push l5 6)))
          (let ((l7  (List.push l6 7)))
          (let ((l8  (List.push l7 8)))
          (let ((l9  (List.push l8 9)))
          (let ((l10 (List.push l9 10)))
          (let ((l11 (List.push l10 11)))
          (let ((l12 (List.push l11 12)))
            (List.len l12))))))))))))))
  (output (: 12 Int64)))

(case "resolving a name in a shadowing environment returns the innermost binding's slot"
  (doc    "The compiler-internal SCOPE-RESOLUTION idiom behind lexical shadowing (the value-level cases
           above pin the observable; this pins how a name resolver realizes it). A name environment is a
           list of bound names in scope order (a self-hosted compiler holds parameters and `let`
           bindings this way, resolving a name reference to a local slot). When a name is bound twice —
           an inner `let` shadowing an outer binding of the same name — resolution must return the
           INNERMOST (latest, highest-slot) binding, not the first. `pos` searches the environment
           deepest-first and returns the last matching position: for env `[5, 7, 5]` (name 5 bound at
           slot 0, shadowed at slot 2), looking up 5 yields 2 — the shadowing binding — not 0. Pins that
           a recursive deepest-first environment search realizes lexical shadowing correctly (a
           first-match search would wrongly return the shadowed outer slot 0). An absent name yields -1.
           This is the `bytes → local-slot` name resolution a reader performs, the runtime dual of the
           `let`-shadowing value semantics above.")
  (input  (do
            (type Env ENil (ECons (Tuple Int64 Env)))
            (def (pos xs target k)
              (match xs
                ((Env.ENil _) (- 0 1))
                ((Env.ECons (tuple h t))
                  (let ((deeper (pos t target (+ k 1))))
                    (if (= deeper (- 0 1))
                        (if (= h target) k (- 0 1))
                        deeper)))))
            (def (main) (pos (Env.ECons (tuple 5 (Env.ECons (tuple 7 (Env.ECons (tuple 5 (Env.ENil ()))))))) 5 0)) (export main)))
  (output (: 2 Int64)))

(case "a reference to an unbound name is rejected before running"
  (doc    "Witnesses core-semantics.md #Binding Is Lexical: a reference to a name with no enclosing
           binding is refused. This is a front-end rejection every generation makes — scope resolution
           needs no static typing — so (error CDZ0101) is the recorded outcome.")
  (input  y)
  (error  CDZ0101))

; The unbound-name check (and well-formedness generally) applies to EVERY definition in a module, not
; only the ones reachable from `main`. core-semantics.md #Binding Is Lexical: "A reference to a name with
; no enclosing binding MUST be a compile-time error" — unconditionally, with no reachability qualifier.
; And a module's definitions are its EXPORTS, each reachable by member access (#A Module Evaluates To A
; Record Of Its Exports; #A Module's Exported Definition Is Reachable By Member Access), so a `(def (bad)
; nonexistent)` is not dead code — it is an export `(. m bad)` whose body must resolve. A compiler that
; type-checks and scope-checks only the functions transitively CALLED by `main` lets an ill-formed unused
; definition through: `(module m (def (bad) nonexistent) (def (main) 42))` compiles and runs to 42, its
; `bad` body's unbound `nonexistent` never checked. That contradicts the unconditional binding rule (and
; #A Program That Is Not Well-Typed Is Rejected — "every expression has a statically determined type",
; which includes every definition's body). An inner-module sibling in the same shape IS checked today
; (its unused ill-typed export is rejected), so this is specifically the TOP-LEVEL module's uncalled defs.

(case "an unbound name in an uncalled sibling definition is still rejected"
  (doc    "`(def (bad) nonexistent)` references the unbound name `nonexistent`; even though `main` never
           calls `bad`, the program MUST be rejected (CDZ0101, core-semantics.md #Binding Is Lexical — the
           unbound-name rule is unconditional, not gated on reachability from `main`). A module's
           definitions are its exports, each reachable by member access, so `bad`'s body is not dead code
           and must resolve. A compiler that scope-checks only the functions `main` transitively calls lets
           an ill-formed uncalled definition through, running to 42 instead of rejecting. Pins that every
           top-level definition's body is checked, exactly as an inner-module sibling's already is.")
  (input  (do
            (def (bad)  nonexistent)
            (def (main) 42) (export main)))
  (error  CDZ0101))

; The unbound-name check also reaches into an UNSELECTED conditional branch, not only uncalled top-level
; definitions. core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing binding
; MUST be a compile-time error" (unconditional); #Conditionals Evaluate One Branch: "Every branch … MUST
; be type-checked whether or not it is evaluated." So `(if true 1 undefined-name)` MUST be rejected
; (CDZ0101) even though the constant condition selects the `1` branch and the `undefined-name` branch is
; never evaluated — an unevaluated branch cannot carry a deferred scope error any more than a deferred
; type error. A compiler that const-folds the conditional to its taken branch and scope-checks only that
; branch lets the unbound reference in the dropped branch slip through, running to 1. (The `if` form
; already catches a TYPE error in an unselected branch — `(if true 1 (+ 1 true))` is rejected — so the
; scope check must reach the same unselected branch the type check already does.)

(case "an unbound name in an unselected conditional branch is still rejected"
  (doc    "`(if true 1 undefined-name)` references the unbound name `undefined-name` in the else-branch;
           even though the constant condition `true` selects the `1` branch, the program MUST be rejected
           (CDZ0101, core-semantics.md #Binding Is Lexical — unconditional — with #Conditionals Evaluate
           One Branch: every branch type-checked whether or not evaluated). An unevaluated branch cannot
           carry a deferred scope error. A compiler that const-folds the conditional to its taken branch and
           scope-checks only that branch runs to 1 instead of rejecting. Pins that scope resolution reaches
           an unselected branch, exactly as the type check already does (`(if true 1 (+ 1 true))` is
           rejected). A generation that does not yet scope-check the dropped branch declines.")
  (input  (if true 1 undefined-name))
  (error  CDZ0101))

; The same unbound-name check reaches a boolean connective's SHORT-CIRCUITED operand, exactly as it
; reaches an unselected conditional branch — the spec makes the two identical. core-semantics.md #Boolean
; Connectives Short-Circuit: "a connective shields a trapping or effectful right operand exactly as the
; unselected branch of a conditional does", and "Each operand of a boolean connective MUST be type-checked
; as a boolean whether or not it is evaluated, so that an unevaluated operand cannot carry a deferred
; error, exactly as every branch of a conditional is type-checked." So `(and false undefined-name)` MUST
; be rejected (CDZ0101): `false` short-circuits the conjunction so the right operand is not evaluated, but
; an unevaluated operand cannot carry a deferred SCOPE error any more than a deferred type error. The seed
; already type-checks the dead operand — `(and false (+ 1 1))` is rejected "operand is not a Bool" and
; `(and false (+ 1 true))` "operation on mismatched types" — so, exactly as for the `if` case above
; (`(if true 1 (+ 1 true))` type-checks the dropped branch, and `(if true 1 undefined-name)` now
; scope-checks it too), the scope check MUST reach the same short-circuited operand the type check already
; does. A compiler that emits only the taken side of the short-circuit and scope-checks only that operand
; lets the unbound reference in the dead operand slip through, running `(and false undefined-name)` to
; `false`. A generation that does not yet scope-check the dead operand declines.

(case "an unbound name in a short-circuited boolean operand is still rejected"
  (doc    "`(and false undefined-name)` references the unbound name `undefined-name` in the conjunction's
           right operand; even though the constant left operand `false` short-circuits the `and` so the
           right is never evaluated, the program MUST be rejected (CDZ0101, core-semantics.md #Binding Is
           Lexical — unconditional — with #Boolean Connectives Short-Circuit: each operand is type-checked
           whether or not it is evaluated, EXACTLY AS every branch of a conditional is). An unevaluated
           operand cannot carry a deferred scope error, the connective companion of the unselected-branch
           case above (`(if true 1 undefined-name)`). The seed already type-checks the dead operand
           (`(and false (+ 1 1))` rejects \"operand is not a Bool\"), so the scope check must reach the
           same operand the type check does. A compiler that scope-checks only the evaluated side runs to
           `false` instead of rejecting. A generation that does not yet scope-check the short-circuited
           operand declines rather than answering `false`.")
  (input  (and false undefined-name))
  (error  CDZ0101))

(case "a let-bound variable is in scope inside a boolean connective operand"
  (doc    "The complement of the short-circuited-unbound case above, and the boundary its scope check
           must not over-reach into: a `let`-bound (or parameter) name used in an `and`/`or` operand is
           IN SCOPE and resolves normally (core-semantics.md #Binding Is Lexical: a name resolves to its
           nearest enclosing binding). `(let ((x 3)) (and (> x 0) (< x 9)))` binds `x` and uses it in
           BOTH conjuncts, yielding true. A compiler that scope-checks a connective operand against a
           scope MISSING the enclosing `let`/parameter binders (e.g. a whole-tree type-check pass that
           does not thread block-local bindings) wrongly rejects `x` as unbound — the pair to the
           unbound case: an unbound name is rejected, a bound one is NOT. This idiom (`(let (…) (and
           (>= i 0) …))`) is pervasive in a self-hosting compiler's bounds/range guards, so the scope
           check must run where the operand's lexical environment is complete.")
  (input  (do
            (def (f k) (let ((x k)) (and (> x 0) (< x 9))))
            (def (main) (if (f 3) 1 0)) (export main)))
  (output (: 1 Int64)))

; --- Boolean connectives SHORT-CIRCUIT at RUN TIME --------------------------------------------------
; The cases above pin that a short-circuited operand is still SCOPE- and TYPE-checked (a dead operand
; carries no deferred error). This pins the RUNTIME half of #Boolean Connectives Short-Circuit: when the
; LEFT operand determines the result — `false` for `and`, `true` for `or` — the RIGHT operand is NOT
; EVALUATED, so its side effects (here, a runtime trap) do not occur; when the left does NOT determine the
; result, the right IS evaluated and its trap fires. The right operand is `(< (/ 10 d) 5)` with `d` a
; boundary parameter, so a `d`=0 divide traps at RUN TIME — a genuinely runtime trap (a CONSTANT `(/ 10 0)`
; would be the compile-time `CDZ0304`, caught before any short-circuit, so the divisor MUST be a parameter
; to reach the runtime connective). Each case pairs the two paths: the short-circuit path (left decides →
; right skipped → `d`=0 does NOT trap, the `if` yields the left-decided branch) and the evaluate path (left
; does not decide → right runs → `d`=0 TRAPS). A compiler that eagerly evaluated both operands would trap
; on the skip path; one that never evaluated the right would answer wrong on the evaluate path.

(case "and short-circuits at run time: a false left operand skips the trapping right operand"
  (doc    "`(and b (< (/ 10 d) 5))` with `b`=false short-circuits — the right operand is NOT evaluated — so
           the runtime divide `(/ 10 d)` with `d`=0 does NOT trap, and the whole `and` is `false`, taking
           the `if`'s else branch → 0 (core-semantics.md #Boolean Connectives Short-Circuit). With `b`=true
           the left does not decide the conjunction, so the right IS evaluated and `(/ 10 0)` TRAPS at run
           time. Pins the runtime short-circuit of `and`: a `false` left skips the right operand's effects,
           a `true` left reaches them. The divisor is a parameter so the divide is a RUNTIME trap, not the
           compile-time CDZ0304 a constant `(/ 10 0)` would raise before the connective runs.")
  (input  (do (def (main (: b Bool) (: d Int64)) (if (and b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call   main (: false Bool) (: 0 Int64)) (output (: 0 Int64))
  (call   main (: true Bool)  (: 0 Int64)) (trap   "division by zero"))

(case "and evaluates the right operand when the left is true"
  (doc    "The non-short-circuit path of `and` with a SAFE divisor: `b`=true so the right operand runs and
           the result DEPENDS on it — `d`=5 makes `(/ 10 5)` = 2 < 5 true, so the conjunction is true → 1;
           `d`=2 makes `(/ 10 2)` = 5, and `5 < 5` is false, so the conjunction is false → 0. Pins that a
           `true` left operand genuinely evaluates the right (the two divisors give different answers), the
           value companion of the trap-fires path above — the right operand is reached, not skipped.")
  (input  (do (def (main (: b Bool) (: d Int64)) (if (and b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call   main (: true Bool) (: 5 Int64)) (output (: 1 Int64))
  (call   main (: true Bool) (: 2 Int64)) (output (: 0 Int64)))

(case "or short-circuits at run time: a true left operand skips the trapping right operand"
  (doc    "`(or b (< (/ 10 d) 5))` with `b`=true short-circuits — the right operand is NOT evaluated — so
           the runtime divide with `d`=0 does NOT trap, and the whole `or` is `true`, taking the `if`'s then
           branch → 1. With `b`=false the left does not decide the disjunction, so the right IS evaluated and
           `(/ 10 0)` TRAPS. The `or` mirror of the `and` case: a `true` left skips the right operand's
           effects, a `false` left reaches them.")
  (input  (do (def (main (: b Bool) (: d Int64)) (if (or b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call   main (: true Bool)  (: 0 Int64)) (output (: 1 Int64))
  (call   main (: false Bool) (: 0 Int64)) (trap   "division by zero"))

(case "or evaluates the right operand when the left is false"
  (doc    "The non-short-circuit path of `or` with a SAFE divisor: `b`=false so the right operand runs and
           the result DEPENDS on it — `d`=5 makes `(/ 10 5)` = 2 < 5 true, so the disjunction is true → 1;
           `d`=2 makes `(/ 10 2)` = 5, and `5 < 5` is false, so the disjunction is false → 0. Pins that a
           `false` left operand genuinely evaluates the right (the two divisors give different answers), the
           value companion of the `or` trap-fires path above.")
  (input  (do (def (main (: b Bool) (: d Int64)) (if (or b (< (/ 10 d) 5)) 1 0)) (export main)))
  (call   main (: false Bool) (: 5 Int64)) (output (: 1 Int64))
  (call   main (: false Bool) (: 2 Int64)) (output (: 0 Int64)))

(case "a sequencing block yields the value of its last form"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (2nd sentence:
           a block evaluates to its last form's value). The earlier forms are pure here, so the block's
           only observable result is the last form; ordering of effects is witnessed in
           03-equality-and-observation.sexp.")
  (input  (do 1 2 3))
  (output (: 3 Int64)))

(case "a sequencing block discards a pure compound intermediate"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order (\"evaluate each
           of its forms\" then \"evaluate to the value of its last form\"): a non-final form is
           evaluated and its value discarded, whatever its type. A pure compound value — a record here —
           in a non-final position has no observable effect, so the block yields its last form (42). The
           earlier `do` cases only drop scalars; this pins that a COMPOUND intermediate is dropped the
           same way rather than blocking the block.")
  (input  (do (record (a 1)) 42))
  (output (: 42 Int64)))

(case "a sequencing block discards a pure list intermediate"
  (doc    "Companion of the case above with a list intermediate: `(do (list 1 2 3) 7)` evaluates the
           list, discards it (no effect), and yields the last form 7.")
  (input  (do (list 1 2 3) 7))
  (output (: 7 Int64)))

; --- A declaration in a sequencing block binds for the following forms -------------------
; core-semantics.md #A Declaration In A Sequencing Block Is Scoped To The Forms That Follow It:
; "A declaration form in a sequencing block MUST bind its name for the forms that follow it in
; that block, so that a name a declaration introduces is in scope without a separate binding
; form." This is how a module declaration binds its name (11-modules.sexp relies on it for
; `(do (module m …) <uses-m>)`), and it applies to a `def` declaration too — a `def` in a `do`
; binds its name for the later forms, no enclosing `let` needed. The seed does not yet recognize
; `def` as a declaration in do-block position: it treats the `def` head as a name to resolve and
; declines "unbound name: def" (a misleading code — `def` is a declaration keyword, not a name).

(case "a value declaration in a do block is in scope for the following forms"
  (doc    "Witnesses core-semantics.md #A Declaration In A Sequencing Block Is Scoped To The Forms
           That Follow It: `(def x 5)` as a form of a `do` binds `x` for the following form, so
           `(+ x 1)` sees it without a `let`. The block yields the last form's value, 6. This is the
           same declaration-binds-its-name rule a module declaration uses; a `def` declaration in a
           sequencing block is in scope exactly like one.")
  (input  (do (def x 5) (+ x 1)))
  (output (: 6 Int64)))

(case "a function declaration in a do block is callable by the following forms"
  (doc    "The function-declaration companion: `(def (f n) (+ n 1))` in a `do` binds `f` for the
           following forms, so `(f 9)` calls it and the block yields 10. A declaration introduces its
           name into the rest of the block without a separate binding form, whether it declares a
           value or a function.")
  (input  (do (def (f n) (+ n 1)) (f 9)))
  (output (: 10 Int64)))

; The two cases above declare ONE name and use it in a later form. The scoping rule is that a
; declaration binds its name for EVERY following form — including a LATER DECLARATION, so a chain of
; `def`s each sees the ones before it (core-semantics.md #A Declaration In A Sequencing Block Is Scoped
; To The Forms That Follow It). These pin the chain: a `def` whose value references an earlier `def`, a
; `def`-fn whose body calls an earlier sibling `def`, and a `def` that shadows an outer `let` binding —
; the declaration-scope behavior a prelude or a group of top-level helpers relies on.

(case "a later declaration in a do block sees an earlier one"
  (doc    "`(do (def x 5) (def y (+ x 1)) y)`: the second declaration's value `(+ x 1)` references `x`
           from the first declaration, so `y` = 6 and the block yields 6. Pins that a declaration is in
           scope for a LATER DECLARATION, not only for a plain expression form — the chaining that makes
           a sequence of `def`s (a prelude) resolve.")
  (input  (do (def x 5) (def y (+ x 1)) y))
  (output (: 6 Int64)))

(case "a function declaration in a do block calls an earlier sibling declaration"
  (doc    "`(do (def base 10) (def (add-base n) (+ n base)) (add-base 5))`: the function `add-base`
           closes over the earlier declaration `base`, so `(add-base 5)` = 15. Pins that a `def`-fn's
           body sees the declarations that precede it in the block, exactly as a module function sees
           its siblings.")
  (input  (do (def base 10) (def (add-base n) (+ n base)) (add-base 5)))
  (output (: 15 Int64)))

; A do-local FUNCTION declaration is in scope in its OWN body (self-recursion) and in a sibling
; function's body regardless of order (mutual recursion) — a function group in a `do` is mutually
; visible, exactly like a module's members or the top-level defs, not strictly sequential like a VALUE
; binding (whose scope stays backward-only: `(do (def x 5) (def x (+ x 10)) x)` = 15, the second `x`
; seeing only the first). A recursive do-local function is registered as a standalone emittable function,
; so its recursive call lowers to a runtime call — the same lowering a top-level or module-member
; recursive function gets. A compiler that scopes a do-local declaration strictly sequentially reports
; the self-name (or a forward sibling) unbound; one that models the function group runs the recursion.

(case "a do-local function declaration is recursive"
  (doc    "A do-local `(def (fac n) …)` calls ITSELF: the function is in scope in its own body (like a
           top-level or module-member recursive def), and the self-call lowers to a runtime call. fac(5)
           = 120. Pins that a do-local function group is self-visible, not strictly sequential — a value
           declaration's backward-only scope does not constrain a function's recursion.")
  (input  (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (fac 5)))
  (output (: 120 Int64)))

(case "two do-local function declarations are mutually recursive"
  (doc    "`ev` calls `od`, `od` calls `ev` — a do-local function is visible in a sibling function's body
           regardless of declaration order (mutual visibility, like a module's members). Neither reaches
           a normal form by inlining, so both lower to standalone runtime functions calling each other.
           ev(10) is true → 1. Pins that a do-local function group is mutually visible, so `ev`'s body
           sees `od` declared AFTER it (a forward reference a strictly-sequential scope would reject).")
  (input  (do (def (ev n) (if (= n 0) true (od (- n 1))))
              (def (od n) (if (= n 0) false (ev (- n 1))))
              (if (ev 10) 1 0)))
  (output (: 1 Int64)))

; A recursive do-local function nested INSIDE a HELPER that is itself INLINED at its call site still
; recurses: β-reduction COPIES the helper's body (fresh occurrences), so the copied recursive self-call
; must still lower to a runtime call — the copy's do-local function is registered as an emittable function
; exactly as the original is. A compiler that registers only the LOAD-TIME occurrence declines the copied
; call "needs runtime specialization"; one that registers the reduced copy's do-local functions runs it.

(case "a recursive do-local function nested in an inlined helper recurses"
  (doc    "`helper` carries a do-local recursive `fac`; `(helper 5)` inlines `helper`, COPYING its body —
           so the copied `(fac (- n 1))` self-call must still resolve to an emittable function and lower to
           a runtime call. fac(5) = 120, and `helper` folds away. Pins that recursion survives β-copy of an
           enclosing function: the reduced copy's do-local function is registered like the original, not
           left as an un-lowerable copy.")
  (input  (do (def (helper x)
                (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1)))))
                    (fac x)))
              (def (main) (helper 5)) (export main)))
  (output (: 120 Int64)))

(case "a recursive do-local function survives two inlinings of its helper"
  (doc    "The helper is called TWICE — `(helper 5)` and `(helper 3)` — so its body (with the do-local
           recursive `fac`) is copied twice, each copy's `fac` its own emittable function. fac(5)+fac(3) =
           120 + 6 = 126. Pins that EACH β-copy of the enclosing helper registers its own copy of the
           recursive function (one call site's copy is not confused for another's).")
  (input  (do (def (helper x)
                (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1)))))
                    (fac x)))
              (def (main) (+ (helper 5) (helper 3))) (export main)))
  (output (: 126 Int64)))

; The recursive cases above run at a small CONSTANT depth (fac(5)), which the compiler may fold. A
; self-hosted compiler instead recurses over the SIZE of the program it compiles — a depth decided at run
; time and often large. These drive a recursion to a LARGE N supplied as a boundary argument (so it cannot
; fold), pinning that the compiled recursion runs at scale in CONSTANT STACK: the wasm-backend loop
; transform turns a tail-recursive (and an accumulable non-tail) self-call into a loop, so 100000–1000000
; iterations complete without exhausting the wasm stack. A generation that lowered the self-call as a plain
; recursive wasm CALL would overflow the stack at these depths; the recorded value is the exact accumulation.

(case "a tail-recursive accumulator loop runs to a large runtime N in constant stack"
  (doc    "`(go i n acc) = (if (< i n) (go (+ i 1) n (+ acc i)) acc)` summed over 0..n-1, driven to n =
           100000 by a boundary argument — so it cannot fold and runs as an emitted loop. The sum
           0+1+…+99999 = 4999950000 (which exceeds Int32, so it also pins the Int64 accumulator). Completing
           without a stack overflow pins that the tail-recursive self-call became a CONSTANT-STACK loop (a
           plain recursive call would blow the wasm stack at 100000 deep). n=0 and n=1 pin the empty and
           single-step boundaries (both 0, since the last index summed is n-1).")
  (input  (do (def (go i n acc) (if (< i n) (go (+ i 1) n (+ acc i)) acc))
              (def (main (: n Int64)) (go 0 n 0)) (export main)))
  (call   main (: 100000 Int64)) (output (: 4999950000 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: 1 Int64)) (output (: 0 Int64)))

(case "a tail-recursive countdown loop runs to a very large runtime N"
  (doc    "`(go n acc) = (if (= n 0) acc (go (- n 1) (+ acc 1)))` counts down from n = 1000000, incrementing
           the accumulator each step → 1000000. A million-deep tail recursion completing pins the constant-
           stack loop at an order of magnitude beyond the sum case — the self-hosting scale where a
           per-call stack frame would certainly overflow.")
  (input  (do (def (go n acc) (if (= n 0) acc (go (- n 1) (+ acc 1))))
              (def (main (: n Int64)) (go n 0)) (export main)))
  (call   main (: 1000000 Int64)) (output (: 1000000 Int64)))

(case "a non-tail accumulable recursion runs to a large runtime N in constant stack"
  (doc    "`(go n) = (if (= n 0) 0 (+ 1 (go (- n 1))))` — the self-call is NOT in tail position (its result
           is fed to `(+ 1 …)`), but the accumulation is associative, so the backend's accumulator
           introduction turns it into a constant-stack loop too. Driven to n = 100000 it returns 100000
           without a stack overflow. Pins that the loop transform covers the accumulable non-tail shape (not
           only strict tail calls) at scale — the shape a naive `1 + recurse` count/length takes.")
  (input  (do (def (go n) (if (= n 0) 0 (+ 1 (go (- n 1)))))
              (def (main (: n Int64)) (go n)) (export main)))
  (call   main (: 100000 Int64)) (output (: 100000 Int64)))

(case "a nullary do-local def followed by a use of it computes over its result"
  (doc    "The `def helper … then use it` idiom: `main`'s body is a do-block with a nullary do-local
           `(def (a) 10)` followed by `(+ (a) 5)` = 15. Pins the intended semantics of a def-body sequence
           whose declaration ends in a NUMBER and whose next statement STARTS with a name — the exact shape
           the ML surface's unit-quantity sugar (`5 feet` → Qty) corrupted by greedily reading the def RHS
           `10` and the next statement's leading `a` as one quantity `(Qty.of 10 (Unit.of #\"a\"))`, dropping
           main's real tail. The ML reader now gates that sugar to a single line (no crossing a newline /
           statement boundary), so this program's ML spelling parses like this s-expr and runs to 15. The
           s-expr surface was always correct (it has no juxtaposition sugar); this is the semantics witness.")
  (input  (do (def (main) (do (def (a) 10) (+ (a) 5))) (export main)))
  (call   main)
  (output (: 15 Int64)))

; An ARGUMENT to a user-function call is an expression evaluated in the CALL SITE's scope, and its
; names bind there — a compiler that reduces a call by substituting the argument into the callee's
; body must not thereby resolve the argument's names in the callee's scope. The witnesses below pin
; a let-bound name, a let-bound lambda's argument, and a call's own result each passed as an argument
; to another user call: every one keeps the binding in effect where it was written (core-semantics.md
; #Binding Is Lexical). The passing anchors (a literal argument, a direct reference with no call) sit
; among the other let/def cases in this file; these add the call-argument position specifically.

(case "a let-bound variable passed as a function-call argument resolves at the call site"
  (doc    "`(let ((k 10)) (inc k))` binds `k` = 10, then applies the top-level `inc` to it, yielding
           11. The argument `k` is a reference to the caller's `let` binding; reducing `(inc k)` by
           substituting `k` into `inc`'s body must keep `k` bound at the call site, not resolve it in
           `inc`'s scope (where it is unbound). A literal argument `(inc 10)` and a direct reference
           `(let ((k 10)) (+ k 1))` both already resolve; this pins the call-argument position.")
  (input  (do (def (inc x) (+ x 1)) (def (main) (let ((k 10)) (inc k))) (export main)))
  (output (: 11 Int64)))

(case "a let-bound variable passed to a let-bound lambda resolves at the call site"
  (doc    "The lambda sibling: `(let ((k 10) (f (fn (x) (+ x 1)))) (f k))` applies the let-bound `f`
           to the let-bound `k`, yielding 11. Both names are bound by the same `let`; the argument `k`
           passed to `f` resolves against that `let`, not inside `f`'s body.")
  (input  (do (def (main) (let ((k 10) (f (fn (x) (+ x 1)))) (f k))) (export main)))
  (output (: 11 Int64)))

(case "a nested application of a let-bound lambda resolves each argument at its call site"
  (doc    "`(let ((f (fn (x) (+ x 1)))) (f (f 0)))` = 2: the inner `(f 0)` yields 1 and is the
           argument to the outer `f`. The inner call's result, substituted into the outer application,
           keeps `f` bound by the enclosing `let` — nesting one call as another's argument does not
           lose the binding.")
  (input  (do (def (main) (let ((f (fn (x) (+ x 1)))) (f (f 0)))) (export main)))
  (output (: 2 Int64)))

(case "a let-bound variable derived from a runtime parameter passed as a call argument"
  (doc    "The runtime companion: `(let ((k (+ n 1))) (inc k))` binds `k` from the runtime parameter
           `n` and passes it to `inc`; with n = 40, k = 41 and the result is 42. The binding is
           resolved at the call site whether the let value is a constant or a runtime expression — it
           is the call-argument resolution that matters, not the value's staticness.")
  (input  (do (def (inc x) (+ x 1)) (def (main (: n Int64)) (let ((k (+ n 1))) (inc k))) (export main)))
  (call   main (: 40 Int64))
  (output (: 42 Int64)))

(case "a declaration in a do block shadows an outer binding"
  (doc    "`(let ((x 1)) (do (def x 99) x))`: the `def x 99` inside the `do` shadows the outer `let`
           binding of `x` for the forms that follow it, so the block yields 99. Pins that a do-block
           declaration follows the same lexical shadowing rules as any other binding (core-semantics.md
           #Shadowing Is Well-Defined), taking effect for references in its scope.")
  (input  (let ((x 1)) (do (def x 99) x)))
  (output (: 99 Int64)))

(case "a single-form body admits a sequence by holding a do block"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order in a
           single-form body position: a `let` body is one form, so a sequence of forms is written as a
           `(do …)` there. The prefix form is pure, so the block yields the value of its last form (the
           binding x), showing the do is the sequencing point and let scope is unchanged.")
  (input  (let ((x 4))
            (do
              (+ x 1)
              x)))
  (output (: 4 Int64)))

(case "a sequencing block whose last form is unit yields unit"
  (doc    "Witnesses core-semantics.md #A Sequencing Block Evaluates Its Forms In Order together with
           #An Effect-Only Expression Yields The Unit Value: a `do` yields its last form's value, and
           when that is `unit` the block — and the program — yields the unit value. The earlier form is
           pure and dropped. This is the shape of every effect-only body: a sequence of effects ending
           in unit; it must run and yield unit as the normal-termination value.")
  (input  (do 1 unit))
  (output (: unit Unit)))

(case "a let body of unit yields unit"
  (doc    "Witnesses core-semantics.md #An Effect-Only Expression Yields The Unit Value: binding a
           value and then yielding `unit` produces the unit value as the program result. Unit is an
           ordinary value that a binding form can carry to the run boundary.")
  (input  (let ((x 1)) unit))
  (output (: unit Unit)))

(case "a conditional whose branches are unit yields unit"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch with a unit result: both
           branches yield the unit value, so the conditional yields unit whichever is taken. Pins that
           the unit value flows through `if` and crosses the run boundary as the program's result.")
  (input  (if true unit unit))
  (output (: unit Unit)))

(case "a conditional evaluates only the selected branch"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch. The unselected branch would
           trap on overflow if it were evaluated; the normal result proves it was not.")
  (input  (if true 1 (+ Int64.max 1)))
  (output (: 1 Int64)))

(case "a conditional selects the false branch when the condition is false"
  (doc    "Witnesses core-semantics.md #Conditionals Evaluate One Branch.")
  (input  (if false 1 2))
  (output (: 2 Int64)))

; The single-level case above shields a top-level unselected branch. The guarantee holds at DEPTH too:
; a trapping expression inside a NESTED unselected branch must not be evaluated either — and, dually, a
; conditional's CONDITION may itself be a conditional (an ordinary Bool-valued expression). These pin
; #Conditionals Evaluate One Branch where the single-level case cannot: the shielding is recursive, and
; the condition position accepts a computed Bool, not only a literal or a direct comparison.

(case "a conditional shields a trap in a nested unselected branch"
  (doc    "`(if true (if true 5 (/ 1 0)) 9)`: the outer `if` selects its then-branch, which is another
           `if` selecting 5; the innermost else `(/ 1 0)` (a division-by-zero trap) is in a branch that
           is never selected at either level, so it is NOT evaluated and the result is 5. Pins that
           #Conditionals Evaluate One Branch shields a trap NESTED two levels deep, not only a
           top-level unselected branch (the `(+ Int64.max 1)` case above).")
  (input  (if true (if true 5 (/ 1 0)) 9))
  (output (: 5 Int64)))

(case "a conditional's condition may itself be a conditional"
  (doc    "`(if (if true false true) 1 2)`: the condition is an `if` that evaluates to `false`, so the
           outer conditional selects its else-branch, yielding 2. Pins that the condition position
           accepts an arbitrary Bool-valued expression — here a nested `if` — not only a literal or a
           direct comparison (core-semantics.md #Conditionals Evaluate One Branch: a conditional selects
           by its condition, whatever Bool expression computes it).")
  (input  (if (if true false true) 1 2))
  (output (: 2 Int64)))

(case "a conditional whose condition folds to a constant still drops the untaken trapping branch"
  (doc    "`(if (< 1 2) 7 (% 5 0))`: the condition is a COMPARISON that a constant-folding compiler
           reduces to true at compile time, after which the conditional selects its then-branch (7) and
           the untaken else-branch `(% 5 0)` — a modulo-by-zero that would trap — is never evaluated,
           so the result is 7. Pins that folding a conditional whose CONDITION became a constant is
           short-circuit-preserving: it becomes the taken branch and DROPS the other, exactly as a
           run-time conditional shields an unselected branch (core-semantics.md #Conditionals Evaluate
           One Branch). This is the dual of the divisor-folds-to-zero case (06-numeric-model.sexp): there
           a fold must not ERASE a trap the source denotes; here a fold must not MANUFACTURE a trap the
           source shields. Distinct from the literal-`true` shielding case above in that the shielding
           holds only AFTER the condition itself folds — a fold that evaluated both branches, or kept
           the trapping one, would wrongly trap.")
  (input  (do (def (main) (if (< 1 2) 7 (% 5 0))) (export main)))
  (output (: 7 Int64)))

(case "a conditional selects a branch by a runtime value that is not known at compile time"
  (doc    "`(def (f x) (if (< x 10) x (* x 2)))`: the condition `(< x 10)` depends on the runtime
           parameter `x`, so it CANNOT fold — the conditional must emit a real runtime branch that
           selects `x` (then) or `(* x 2)` (else) by the value computed at run time. `f(21)`: 21 is not
           < 10, so the else-branch yields 42. Pins the runtime conditional — a condition that is a
           genuine runtime value, not a literal or a fold — which a compiler lowers to a structured
           branch (push the condition, then a then/else region each leaving one value of the branches'
           shared type on the stack). Distinct from every conditional case above, whose condition is
           known at compile time (a literal, a nested `if`, or a foldable comparison): here the selection
           happens at run time. The companion `f(3)` (3 < 10) takes the then-branch and yields 3.")
  (input  (do
            (def (f x) (if (< x 10) x (* x 2)))
            (def (main) (f 21)) (export main)))
  (output (: 42 Int64)))

(case "a runtime conditional selects its then-branch when the runtime condition holds"
  (doc    "The then-branch companion to the runtime-conditional case above: with `x` = 3, `(< x 10)` is
           true at run time, so `(if (< x 10) x (* x 2))` selects `x` and yields 3. Together the pair
           pins that a runtime conditional selects EITHER branch by the run-time condition value (42 when
           false, 3 when true), so the structured branch is a genuine two-way selection, not a folded
           constant.")
  (input  (do
            (def (f x) (if (< x 10) x (* x 2)))
            (def (main) (f 3)) (export main)))
  (output (: 3 Int64)))

(case "a conditional on a negated runtime condition selects the correct branch and shields the other"
  (doc    "A conditional whose condition is `(not c)` may be lowered by SWAPPING the then/else branches and
           dropping the negation (rather than computing `not` then branching): `(if (not c) T E)` becomes
           `(if c E T)`. That rewrite must preserve BOTH the selection and the shielding. `(if (not b) 7 (/
           1 z))` with `b` = false: `(not false)` is true, so the THEN branch (7) is selected and the else
           `(/ 1 z)` (a division by zero at z = 0) is NOT evaluated — the result is 7, not a trap. A swap
           that mis-mapped the branches would select `(/ 1 z)` and trap; one that evaluated both would trap
           too. The anchor: with `b` = true, `(not true)` is false, so the else `(/ 1 z)` IS selected and
           traps. Pins the negated-if branch swap keeps the untaken branch shielded and the condition
           correctly inverted.")
  (input  (do
            (def (main (: b Bool) (: z Int64)) (if (not b) 7 (/ 1 z)))
            (export main)))
  (call   main (: false Bool) (: 0 Int64))
  (output (: 7 Int64)))

(case "a conjunction guards a let over a runtime value inside a conditional"
  (doc    "An INTEGRATION case: several control constructs composed in one function over a runtime
           parameter, the way a real program (not an isolated feature test) uses the language.
           `classify x = (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0)` composes: a
           short-circuit `and` of two comparisons as the condition (each operand a runtime `>`/`<`), a
           `let` binding a RUNTIME value `(* x x)` in the then-branch (so it must emit a real local, not
           a compile-time alias), the outer conditional selecting Int64 branches, and the arithmetic —
           all driven by the runtime argument. `classify 4`: `0 < 4` and `4 < 10` both hold, so
           `(let ((y (* 4 4))) (- y 1))` = 16 - 1 = 15. Pins that these constructs COMPOSE in one
           function — the short-circuit `and` (which desugars to a nested conditional), a runtime `let`,
           and the enclosing `if` nest correctly and thread their values — not merely that each works in
           isolation. The out-of-range companion below takes the else-branch.")
  (input  (do
            (def (classify x) (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0))
            (def (main) (classify 4)) (export main)))
  (output (: 15 Int64)))

(case "the guarded-let conditional takes its else-branch when the conjunction is false"
  (doc    "The else companion of the integration case above: `classify 20` — `20 < 10` is false, so the
           short-circuit `and` is false and the outer conditional selects its else-branch 0, never
           evaluating the `let`. Together the pair pins that the composed `and`/`let`/`if` selects by the
           runtime value in both directions (15 in range, 0 out of range), and that the short-circuit
           `and` shields the `let`-bearing then-branch when the guard fails.")
  (input  (do
            (def (classify x) (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0))
            (def (main) (classify 20)) (export main)))
  (output (: 0 Int64)))

; --- A conditional's branches must have the same type ------------------------------------
; core-semantics.md #Conditionals Evaluate One Branch, 2nd sentence: "Every branch of a
; conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated
; branch cannot carry a deferred error." So a conditional whose branches have DIFFERENT types
; is ill-typed even when the condition is a compile-time constant that never evaluates the
; mismatched branch — the compiler MUST reject it (CDZ0203, a type mismatch). The rejection is the recorded
; outcome; the program does not run, so it has no branch value. A generation that does not yet
; type-check the unevaluated branch declines rather than emitting a component
; (reject-don't-miscompile).

(case "a conditional with an integer then-branch and a boolean else-branch is a type error"
  (doc    "The then-branch is Int64, the else-branch is Bool — different types. Even with a constant
           condition selecting the Int64 branch, the compiler MUST type-check BOTH branches and reject
           the mismatch (CDZ0203) rather than run the program.")
  (input  (if true 1 false))
  (error  CDZ0203))

(case "a conditional type error is caught even when the mismatched branch is the one taken"
  (doc    "The companion with the condition false, selecting the Bool branch: the branches still
           disagree in type (Int64 vs Bool), so the compiler MUST reject (CDZ0203). Pins that the
           check is on the pair of branch types, not on which branch would run.")
  (input  (if false 1 false))
  (error  CDZ0203))

(case "a conditional with a compound branch and a scalar branch is a type error even when the compound branch is dead"
  (doc    "`(if false (record (a 1)) 7)` — the then-branch is a compound (a record), the else-branch is a
           scalar (Int64); they have different types, so the conditional is ill-typed and the compiler MUST
           reject it (CDZ0203). The constant condition `false` selects the SCALAR branch, so a compiler that
           const-folds the conditional to its taken branch would discard the compound then-branch WITHOUT
           type-checking it and silently accept an ill-typed program — a miscompile. The type-check is on the
           PAIR of branches, so it must happen BEFORE (or independently of) any fold that eliminates a branch:
           an unevaluated branch cannot carry a deferred type error. This pins the compound-vs-scalar instance
           of the dead-branch check, which the scalar-vs-scalar cases above do not exercise (folding a compound
           branch away is where the check is easiest to skip).")
  (input  (if false (record (a 1)) 7))
  (error  CDZ0203))

; The branch-type-agreement check must fire when the conditional is INSIDE A FUNCTION BODY with a
; constant condition, not only at the top-level entry expression. The cases above pair mismatched
; branches in `main`'s own body and are correctly rejected; but the same mismatch inside a `def`ed
; function with a compile-time-constant condition SLIPS — the const-condition fold in a function body
; discards the untaken branch WITHOUT the pair-of-branches type-check that #Conditionals Evaluate One
; Branch requires ("every branch … type-checked whether or not it is evaluated"). `(def (f) (if true 1
; false))` pairs an Int64 then-branch with a Bool else-branch — ill-typed exactly as the top-level `(if
; true 1 false)` is (CDZ0203) — yet the seed accepts it and `f` returns 1, an ill-typed program run
; (and it composes: `(+ (f) 0)` = 1). Worse, when the surviving (taken) branch is a COMPUTED expression
; rather than a constant — `(def (f n) (if true (+ n 1) false))`, whose `(+ n 1)` cannot fold to a
; literal — the unchecked Int/Bool branch-representation mismatch makes the compiler emit an INVALID
; wasm component (it fails validation), not merely a wrong value: a fold that drops a branch dropped its
; type-check, and the two branches' incompatible representations reach code generation. The internal
; checks of the dropped branch DO survive the fold (an else `(+ 1 true)` is still rejected "operation on
; mismatched types", an unbound else name is still CDZ0101) — only the branch-type-AGREEMENT check is
; lost, which is the one these cases pin. This is the in-function companion of the top-level dead-branch
; cases: the fold that eliminates a branch must not eliminate the agreement check, wherever the `if` sits.
(case "a conditional inside a function with a constant condition and mismatched branches is a type error"
  (doc    "`(def (f) (if true 1 false))` pairs an Int64 then-branch with a Bool else-branch — different
           types, ill-typed exactly as the top-level `(if true 1 false)` above (CDZ0203). But the `if` is
           inside a function body with a constant condition, and the seed's const-condition fold in a
           function body discards the untaken `false` branch WITHOUT the pair-of-branches type-check
           (core-semantics.md #Conditionals Evaluate One Branch: every branch type-checked whether or not
           evaluated). The seed accepts it and `f` returns 1 — an ill-typed program run — where the
           identical `if` at the top-level entry expression is correctly rejected. (When the surviving
           branch is a COMPUTED expression, `(def (f n) (if true (+ n 1) false))`, the unchecked Int/Bool
           branch-representation mismatch makes the compiler emit an INVALID component rather than a wrong
           value.) Pins that the dead-branch agreement check fires inside a function body too, not only at
           the top-level entry — the internal checks of the dropped branch already survive the fold; only
           the branch-type-agreement check is lost. A generation that type-checks the pair of branches
           before folding declines rather than running the ill-typed program or emitting invalid code.")
  (input  (do
            (def (f) (if true 1 false))
            (def (main) (f)) (export main)))
  (error  CDZ0203))

(case "a conditional with integer and floating-point branches is a type error"
  (doc    "Int64 and Float64 are distinct numeric types that do not silently unify (numeric-model.md
           #Numeric Types Do Not Silently Promote). A conditional with an Int64 branch and a Float64
           branch is therefore ill-typed and the compiler MUST reject it (CDZ0201).")
  (input  (if true 1 3.5))
  (error  CDZ0201))

; The branch-type-agreement check must compare branches STRUCTURALLY, not only by coarse kind: two
; branches that are both tuples but of DIFFERENT ARITY are different types (a tuple's arity is part of
; its type, type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix), so the conditional
; is ill-typed even though both branches are "a tuple." `(if true (tuple 1 2) (tuple 3 4 5))` pairs a
; two-tuple with a three-tuple; the whole `if` has no single type, so the compiler MUST reject it
; (CDZ0203) — a check that compares only the branches' KIND (tuple vs tuple) and not their arity accepts
; the mismatch and returns whichever branch the constant condition selects, an unevaluated branch carrying
; a deferred type error (core-semantics.md #Conditionals Evaluate One Branch — every branch type-checked).
; A generation that does not yet compare branch shapes structurally declines rather than accepting.

(case "a conditional with two tuple branches of different arity is a type error"
  (doc    "`(if true (tuple 1 2) (tuple 3 4 5))` pairs a two-element tuple with a three-element tuple —
           different types, since a tuple's arity is part of its type. The whole conditional has no single
           type, so it is ill-typed and the compiler MUST reject it (CDZ0203), exactly as the Int/Bool and
           compound/scalar branch-mismatch cases above. Pins that branch-type agreement is checked
           STRUCTURALLY, not only at coarse kind (both branches being 'a tuple' is not enough) — a compiler
           comparing only branch kinds accepts this and returns the two-tuple, an ill-typed program run.")
  (input  (if true (tuple 1 2) (tuple 3 4 5)))
  (error  CDZ0203))

(case "a conditional with two tuple branches of different element type is a type error"
  (doc    "`(if true (tuple 1 2) (tuple 1 true))` pairs `(Tuple Int64 Int64)` with `(Tuple Int64 Bool)` —
           same arity but a different element type at position 1, so different types. The conditional is
           ill-typed (CDZ0203), the element-type companion of the arity case above. Pins that the structural
           branch-type comparison descends into a tuple's element types, not only its arity — the same
           depth the list-element homogeneity check already applies.")
  (input  (if true (tuple 1 2) (tuple 1 true)))
  (error  CDZ0203))

; The structural branch-type check must NOT treat a list's LENGTH as part of its type — unlike a tuple's
; arity. A list is a variable-length sequence typed by its element type (collections-and-text.md #A List
; Is An Ordered Homogeneous Sequence, #A List Is Grown By Functional Construction), so two list branches
; of the SAME element type but DIFFERENT LENGTH are the SAME type `(List Int64)`, and the conditional is
; well-typed — its value is whichever list the condition selects. `(if true (list 1 2) (list 3 4 5))`
; yields `(list 1 2)`. A compiler that reuses the tuple-arity branch-shape check on lists wrongly rejects
; this well-typed conditional as "branches have different shapes" — length is a tuple's type distinction,
; not a list's. (The genuinely ill-typed list-branch case is two lists of different ELEMENT type, e.g.
; `(if … (list 1 2) (list true false))` — those are different types and rejected, exactly as any type
; mismatch is; only the different-LENGTH same-element-type case must be accepted.)

(case "a conditional with two list branches of different length is well-typed"
  (doc    "`(if true (list 1 2) (list 3 4 5))` pairs a two-element list with a three-element list — the
           SAME type `(List Int64)`, since a list's length is not part of its type (a list is
           variable-length; collections-and-text.md #A List Is An Ordered Homogeneous Sequence). The
           conditional is well-typed and yields the selected branch `(list 1 2)`. This is the list
           counterpoint to the tuple-arity branch case above: a tuple's arity IS part of its type (so
           different-arity tuple branches are rejected), but a list's length is NOT, so different-length
           list branches MUST be accepted. Pins that the branch-shape check does not treat list length as
           a shape mismatch — a compiler reusing the tuple-arity check on lists wrongly rejects this.")
  (input  (if true (list 1 2) (list 3 4 5)))
  (output (: (list 1 2) (List Int64))))

; --- A conditional's condition must be a Bool --------------------------------------------
; core-semantics.md #Conditionals Evaluate One Branch: a conditional selects a branch by its
; condition, which is a Bool. A condition of any other type is ill-typed — the compiler MUST
; reject it (CDZ0203). A COMPOUND condition (a tuple/record/list) must be rejected as a not-a-Bool
; type error with the constructor `tuple`/`record`/`list` intact — it is a recognized form (it
; builds a value everywhere else), so a diagnostic of "unbound name: tuple" would be a misleading
; code (CDZ0101) for what is plainly a not-a-Bool type error, the same wrong-diagnostic class as an
; out-of-range integer literal reported as an unbound name (01-literals.sexp).

(case "an integer if condition is a type error, not a running conditional"
  (doc    "1 is Int64, not Bool. A conditional's condition selects a branch and MUST be a Bool; an
           Int64 condition is ill-typed (CDZ0203). A C-like language treats a nonzero int as true —
           Cadenza does not silently coerce (numeric-model.md #Numeric Types Do Not Silently
           Promote); there is no truthiness. A generation that does not yet wire the CDZ0203 code
           declines rather than running the program (reject-don't-miscompile).")
  (input  (if 1 10 20))
  (error  CDZ0203))

(case "a compound if condition is a type error, not an unbound name"
  (doc    "A tuple is not a Bool, so `(if (tuple 1 2) …)` is ill-typed (CDZ0203). The constructor
           `tuple` is a recognized form — `(tuple 1 2)` builds a value in every other position — so
           reporting `unbound name: tuple` (CDZ0101) would mistake a not-a-Bool type error for a name
           resolution failure. The condition's type is what is wrong, not the spelling of a name.
           Pins that a compound condition is rejected as a type error with the constructor intact,
           the same misleading-diagnostic class as an out-of-range literal reported as unbound.")
  (input  (if (tuple 1 2) 10 20))
  (error  CDZ0203))

(case "a pattern binds a name scoped to its branch"
  (doc    "Witnesses core-semantics.md #Bindings Introduced By A Pattern Are Scoped To Its Branch.
           Option is declared where used as (Some <value> | None) (options/code-shape/); the Some
           branch binds n to the payload, in scope only in that branch. Patterns are uniform:
           (Some n) for unary, (None _) for nullary — both single-arity.")
  (input  (match (Some 5)
            ((Some n) n)
            ((None _) 0)))
  (output (: 5 Int64)))

(case "matching on integer literals"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: a match can branch on
           literal values, not just constructors. Integer literal patterns match by equality. The
           compiler uses this to dispatch on instruction opcodes and section IDs.")
  (input  (match 2
            (0 "zero")
            (1 "one")
            (2 "two")
            (_ "many")))
  (output (: "two" String)))

; --- A literal pattern's type must match the scrutinee's type ----------------------------
; A literal pattern matches the scrutinee by equality (above), and equality is only defined between
; values of the SAME type (core-semantics.md #Equality Is Structural; a cross-type comparison is a
; type error). So a literal pattern whose type differs from the scrutinee's — a `true` (Bool) pattern
; against an Int64 scrutinee, an integer pattern against a Bool scrutinee — can never meaningfully
; match: it is a static type mismatch between the arm and the scrutinee, a type error (CDZ0201), the
; same class as a tuple pattern of the wrong arity or a `(Some x)` pattern against an Int64. The
; compiler rejects the ill-typed arm; a generation that does not yet check the pattern's type against
; the scrutinee's declines rather than running the program (reject-don't-miscompile).

(case "a boolean literal pattern against an integer scrutinee is a type error"
  (doc    "The scrutinee `5` is Int64; the pattern `true` is Bool. A literal pattern matches by
           equality, which is only defined within one type, so a Bool pattern can never match an Int64
           value — the arm is ill-typed and the compiler MUST reject the match (CDZ0201). Pins that a
           literal pattern's type is checked against the scrutinee's, not silently failed to match.")
  (input  (match 5 (true 1) (_ 0)))
  (error  CDZ0201))

(case "an integer literal pattern against a boolean scrutinee is a type error"
  (doc    "The mirror: scrutinee `true` is Bool, pattern `5` is Int64 — a type mismatch, so the arm is
           ill-typed (CDZ0201). Pins the check in both directions — the scrutinee and every literal
           pattern must share a type.")
  (input  (match true (5 1) (_ 0)))
  (error  CDZ0201))

(case "matching on string literals"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns
           match by equality. The compiler uses this heavily to dispatch on instruction tags like
           'i64.const', 'i64.add', etc. — replacing nested if/= chains with readable match.")
  (input  (match "hello"
            ("hello" 1)
            ("world" 2)
            (_    0)))
  (output (: 1 Int64)))

(case "matching on a string produced by an expression"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: string literal patterns match by
           equality against the scrutinee's VALUE, whether the scrutinee is written as a bare literal
           (the case above) or produced by an expression. `(String.concat \"a\" \"b\")` evaluates to
           \"ab\", which the \"ab\" arm matches, yielding 100 — not the wildcard. (That the two strings
           are equal is independently witnessed: `(= (String.concat \"a\" \"b\") \"ab\")` is true. A
           bare and a let-bound \"ab\" scrutinee already select the arm; a string-valued expression
           must behave identically — the common compiler idiom of dispatching on a computed
           instruction name.)")
  (input  (match (String.concat "a" "b")
            ("ab"  100)
            (_  200)))
  (output (: 100 Int64)))

(case "matching on a sliced string selects the literal arm"
  (doc    "Companion using another string-producing operation: `(String.slice \"hello\" 0 2)` yields Some
           \"he\"; `expect` unwraps the in-bounds slice to \"he\", which the \"he\" arm matches, yielding
           100. A slice result is fallible (collections-and-text.md #Indexing And Lookup Are Fallible,
           Not Trapping), so the program names the in-bounds expectation before matching the substring.")
  (input  (match (Option.expect (String.slice "hello" 0 2) "slice is in bounds")
            ("he"  100)
            (_  200)))
  (output (: 100 Int64)))

; --- Requiring the value of an optional at run time -------------------------------------------
; core-semantics.md #Requiring The Value Of An Optional Traps On Absence: `Option.expect` (and its
; Result twin) unwraps the present variant's payload or traps on absence. The cases above exercise
; it only on COMPILE-TIME-CONSTANT optionals (a literal slice/index). These pin it on a RUNTIME
; optional — a parameter, or a value a runtime operation produced — where present/absent is decided
; at run time by the sum's discriminant, not folded. This is the compiler's unwrap-or-trap idiom:
; assert a `List.at`/`Bytes.at`/`checked-*` result is present, taking its value or trapping.

(case "expect unwraps the present case of a runtime optional"
  (doc    "`(g (Some 7))` calls `(g o) = (Option.expect o \"m\")` on a RUNTIME optional (the parameter
           `o`, not a constant): the discriminant says Some at run time, so expect yields its payload 7.
           Pins expect on an optional whose present/absent is decided at run time — the unwrap-or-trap
           idiom over a value the compiler cannot fold, distinct from expect on a literal optional.")
  (input  (do
            (def (g o) (Option.expect o "m"))
            (def (main) (g (Some 7))) (export main)))
  (output (: 7 Int64)))

(case "expect traps on the absent case of a runtime optional"
  (doc    "The absent companion: `(g (None unit))` on the same `(Option.expect o \"m\")` sees the None
           discriminant at run time, so expect traps rather than producing a value (core-semantics.md
           #Requiring The Value Of An Optional Traps On Absence). The terminal condition is the trap.")
  (input  (do
            (def (g o) (Option.expect o "m"))
            (def (main) (g (None unit))) (export main)))
  (trap   "m"))

(case "expect on a RUNTIME-absent optional traps with the canonical unreachable kind"
  (doc    "The runtime (non-const-folded) absent expect: `main`'s parameter feeds a runtime `Option Int64`
           that is always `None`, so `(Option.expect o \"…\")` sees the None discriminant AT RUN TIME and
           traps (core-semantics.md #Requiring The Value Of An Optional Traps On Absence). The trap's
           canonical KIND is `unreachable` — the SAME on every backend: wasm's `SumExpect` absent branch is
           an `unreachable` instruction, and the Rust backend panics with a reason classifying as
           `unreachable` (matching the explicit-`trap` lowering). Pins that a RUNTIME expect-on-absent traps
           consistently across backends (distinct from the const-folded case above, whose recorded message
           is a custom string the trap-kind grader does not classify).")
  (input  (do
            (def (g (: o (Option Int64))) (Option.expect o "boom"))
            (def (main (: k Int64)) (g (if (> k 0) (Option.None) (Option.None))))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))

(case "expect makes a checked-arithmetic result trap on overflow"
  (doc    "The compiler idiom expect exists for: turn a non-trapping `Int64.checked-add` into a TRAPPING
           add. `(add-ck a b) = (Option.expect (Int64.checked-add a b) \"overflow\")` yields the sum when
           in range — `(add-ck 20 22)` = 42, usable directly in arithmetic. Pins expect on a RUNTIME
           `Option<Int64>` a runtime operation produced, unboxing to the Int64 payload.")
  (input  (do
            (def (add-ck a b) (Option.expect (Int64.checked-add a b) "overflow"))
            (def (main) (+ (add-ck 20 22) (add-ck 1 1))) (export main)))
  (output (: 44 Int64)))

(case "expect on an overflowing checked add traps"
  (doc    "The overflow companion: `(add-ck Int64.max 1)` computes a checked add that overflows, so its
           `Option<Int64>` is None and expect traps — the overflow-trapping arithmetic expect+checked
           compose into. Contrast `(Int64.wrapping-add Int64.max 1)`, which wraps to MIN without trapping.")
  (input  (do
            (def (add-ck a b) (Option.expect (Int64.checked-add a b) "overflow"))
            (def (main) (add-ck Int64.max 1)) (export main)))
  (trap   "overflow"))

(case "expect unwraps the ok case of a runtime result"
  (doc    "`Result.expect` is the Result twin of `Option.expect`: `(g (Ok 99))` on `(Result.expect r \"m\")`
           sees the Ok discriminant at run time and yields its payload 99; the Err case would trap. Pins
           expect on a runtime Result, the same unwrap-or-trap accessor over the two-variant Result sum.")
  (input  (do
            (def (g r) (Result.expect r "m"))
            (def (main) (g (Ok 99))) (export main)))
  (output (: 99 Int64)))

(case "expect traps on the err case of a RUNTIME result"
  (doc    "The Result absent companion (the Err twin of the Option-None expect-trap): a runtime `Result
           Int64 Int64` that is always `Err` feeds `(Result.expect r \"…\")`, which sees the Err
           discriminant AT RUN TIME and traps (core-semantics.md #Requiring The Value Of An Optional Traps
           On Absence, extended to Result's Err). The trap's canonical KIND is `unreachable` on every
           backend — wasm's `SumExpect` absent branch is an `unreachable` instruction and the Rust backend
           panics with a reason classifying the same way. Pins that `Result.expect` on Err traps
           consistently across backends, the two-variant-Result companion of the Option-None trap.")
  (input  (do
            (def (g (: r (Result Int64 Int64))) (Result.expect r "boom"))
            (def (main (: k Int64)) (g (if (> k 0) (Result.Err 1) (Result.Err 2))))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))

(case "matching falls through to else when no literal matches"
  (doc    "Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected: when no literal pattern
           matches, the else (wildcard) catches it. Without else, a non-exhaustive match traps.")
  (input  (match 99
            (0 "zero")
            (1 "one")
            (_ "other")))
  (output (: "other" String)))

; --- A match arm may carry a guard ---------------------------------------------------------
; core-semantics.md #Matching Is Exhaustive Or Rejected: an arm's pattern may carry a boolean GUARD
; `pattern if <guard>` — the arm is selected only when the pattern matches AND the guard, a pure
; expression evaluated with the pattern's bindings in scope, is true. A failing guard falls through
; to the following arms exactly as a non-matching pattern does. The guard is an ordinary expression
; (it can read the names the pattern binds); it refines WHICH values an arm accepts without changing
; the pattern's shape. A guard does NOT count toward exhaustiveness (a guarded arm might not fire), so
; a match whose only arms are guarded is non-exhaustive and rejected — the cases below pin selection,
; fall-through, binding-visibility, and the exhaustiveness rule.

(case "a guarded arm is selected when its guard holds"
  (doc    "The arm `x if x < 0` binds `x` to the scrutinee and is selected only if the guard `x < 0`
           is true. For scrutinee -5 the guard holds, so the arm fires and the result is -1. Pins that
           a guard `pattern if <expr>` gates its arm on a boolean condition evaluated with the
           pattern's bindings in scope (core-semantics.md #Matching Is Exhaustive Or Rejected).")
  (input  (match (- 0 5)
            ((guard x (< x 0)) (- 0 1))
            (_ 1)))
  (output (: -1 Int64)))

(case "a failing guard falls through to a later arm"
  (doc    "The mirror: for scrutinee 5 the guard `x < 0` is false, so the guarded arm does NOT fire and
           the match falls through to the wildcard, yielding 1 — exactly as a non-matching pattern
           falls through. Pins that a false guard skips its arm rather than trapping or forcing it.")
  (input  (match 5
            ((guard x (< x 0)) (- 0 1))
            (_ 1)))
  (output (: 1 Int64)))

(case "a guard sees the names its pattern binds and arms are tried in order"
  (doc    "Two guarded arms binding `n`: for scrutinee 7 the first guard `n = 0` is false, the second
           `n < 10` is true, so the second arm fires and returns `n` (7). Pins that a guard reads the
           pattern's binding (`n` is in scope in the guard) and that guarded arms are tried top-to-bottom,
           the first whose pattern-and-guard both hold winning.")
  (input  (match 7
            ((guard n (= n 0)) 100)
            ((guard n (< n 10)) n)
            (_ 999)))
  (output (: 7 Int64)))

(case "a match whose only arm is guarded is non-exhaustive"
  (doc    "A guard does not count toward exhaustiveness: a guarded arm might not fire (its guard may be
           false), so it cannot be the coverage for any value. A match on an Int64 whose sole arm is
           `x if x < 0` — with no unconditional arm or wildcard — therefore covers no value unconditionally
           and is non-exhaustive; the compiler MUST reject it (CDZ0210), the same rejection as a match
           missing a case. Pins that guarded arms are excluded from the exhaustiveness check. A generation
           that does not yet check runtime exhaustiveness declines rather than emitting a component.")
  (input  (match 5
            ((guard x (< x 0)) 1)))
  (error  CDZ0210))

(case "a match whose only arm is guarded by a literally-true condition is still non-exhaustive"
  (doc    "The exhaustiveness check treats EVERY guard as opaque — it does not reason about whether the
           guard condition is true. `(match 5 ((guard x true) 1))` has a guard whose condition is the
           literal `true`, so the arm always fires at run time; but the checker MUST still reject it
           (CDZ0210) as non-exhaustive, exactly as the `(< x 0)` case above. A checker that 'optimized' by
           recognizing a literally-true guard as an unconditional arm would wrongly ACCEPT this match, then
           the same reasoning would have to extend to arbitrarily complex always-true conditions — the
           conservative rule is simpler and sound: a guarded arm never counts toward coverage, whatever its
           condition. Pins that guard truth is not analyzed for exhaustiveness.")
  (input  (match 5
            ((guard x true) 1)))
  (error  CDZ0210))

; --- A guard may refine a VARIANT pattern ---------------------------------------------------------
; A guard composes with a variant (sum) pattern, not only a bare binder: `(guard (Some x) <cond>)`
; fires when the scrutinee is `Some` AND `<cond>` (which reads the payload binder `x`) holds. On a
; false guard the arm falls through to a LATER arm — including a later arm of the SAME variant — just
; as a scalar guard does. The payload binder is in scope for the guard cond (resolved through the
; `(guard …)` wrapper to the inner variant pattern), and a guarded variant arm does NOT count toward
; exhaustiveness (so a match whose only `Some` arm is guarded, with no `Some` fall-through, is
; non-exhaustive). These pin the guard-over-variant surface end to end.

(case "a guard over a variant pattern gates on the payload"
  (doc    "`(match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0))` — the natural `(Some x)
           if x > 0` shape: the arm fires when the Option is `Some` AND its payload is positive, binding
           `x` to the payload. For `(Some 5)` the guard `5 > 0` holds, so the arm returns x = 5. The
           payload binder `x` is in scope for the guard condition (through the `(guard …)` wrapper). Was a
           spurious CDZ0101 'unbound name x' before guarded sum-match support landed.")
  (input  (do
            (def (f (: o (Option Int64))) (match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0)))
            (def (main (: n Int64)) (f (Some n))) (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64)))

(case "a guarded variant arm falls through when the guard fails"
  (doc    "The fall-through face of the same program: for `(Some -3)` the guard `x > 0` is false, so the
           guarded `(Some x)` arm does NOT fire and the match falls through to the plain `(Some y)` arm,
           which negates: `-(-3)` = 3. Pins that a guarded VARIANT arm falls through to a LATER arm of the
           same variant exactly as a bare-binder guard falls through — the per-variant fall-through the
           decision tree threads.")
  (input  (do
            (def (f (: o (Option Int64))) (match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0)))
            (def (main (: n Int64)) (f (Some n))) (export main)))
  (call   main (: -3 Int64))
  (output (: 3 Int64)))

(case "chained guards of the same variant are tried in order"
  (doc    "Two guarded `Some` arms then a plain `(Some z)`: `(guard (Some x) (> x 10))`, `(guard (Some y)
           (> y 0))`, `(Some z)`. Each guard is tried top-to-bottom, falling through on failure. For
           `(Some 5)` the first guard `5 > 10` fails and the second `5 > 0` holds, so the result is 1.
           Pins that multiple guarded arms of the SAME variant chain their fall-through correctly.")
  (input  (do
            (def (f (: o (Option Int64)))
              (match o
                ((guard (Some x) (> x 10)) 100)
                ((guard (Some y) (> y 0)) 1)
                ((Some z) 0)
                ((None) (- 0 1))))
            (def (main (: n Int64)) (f (Some n))) (export main)))
  (call   main (: 5 Int64))
  (output (: 1 Int64)))

(case "a match whose only variant arm is guarded is non-exhaustive"
  (doc    "A guarded VARIANT arm covers no value unconditionally, so `(match o ((guard (Some x) (> x 0))
           x) ((None) 0))` — whose only `Some` arm is guarded, with no unguarded `Some` fall-through —
           leaves `Some` uncovered and is non-exhaustive: the compiler MUST reject it (CDZ0210), exactly
           as a guarded scalar arm is excluded from coverage. Pins that a guarded variant arm does not
           satisfy exhaustiveness for its variant.")
  (input  (do
            (def (f (: o (Option Int64))) (match o ((guard (Some x) (> x 0)) x) ((None) 0)))
            (def (main (: n Int64)) (f (Some n))) (export main)))
  (error  CDZ0210))

(case "a false variant guard shields its arm's trapping body"
  (doc    "A guarded arm's body runs only when the guard holds (core-semantics.md #Boolean Connectives
           Short-Circuit, applied to a guard): `(Some x) if x > 0` over `(Some 0)` must NOT evaluate its
           body `(/ 10 x)` — the guard `0 > 0` is false, so the arm is skipped and the match falls through
           to `(Some y) -1`. The division by the zero payload never happens. A generation that folds a
           guarded body regardless of its guard raises a spurious compile-time divide-by-zero (CDZ0304)
           for an arm that never runs; the fold must evaluate the guard FIRST and skip the body when it is
           false. The variant-guard sibling of the scalar shielding cases above.")
  (input  (match (Some 0)
            ((guard (Some x) (> x 0)) (/ 10 x))
            ((Some y) -1)
            ((None) -2)))
  (output (: -1 Int64)))

; --- A match must cover every value of the scrutinee's type ------------------------------
; core-semantics.md #Matching Is Exhaustive Or Rejected: "A match whose patterns do not cover
; every value of the scrutinee's type MUST be a compile-time error." A Bool has exactly two
; values, true and false, so a match on a Bool that arms only ONE of them (and has no wildcard)
; is non-exhaustive and the compiler MUST reject it (CDZ0210) — even though the missing case would
; only be reached for one of the two inputs. The rejection is the recorded outcome; the program
; does not run. A generation that does not yet check runtime-bool exhaustiveness declines rather
; than emitting a component (reject-don't-miscompile).

(case "a bool match missing the false arm is non-exhaustive"
  (doc    "The scrutinee `b` is a Bool — its type has exactly two values. A match arming only `true`
           leaves `false` uncovered and has no wildcard, so it is non-exhaustive and the compiler MUST
           reject it (CDZ0210, coded-span-record.md). The rejection is the recorded outcome; the
           program does not run. Pins runtime-bool exhaustiveness against a match whose scrutinee is a
           function parameter, not a compile-time constant.")
  (input  (do
            (def (f b) (match b (true 1)))
            (def (main) (f false)) (export main)))
  (error  CDZ0210))

(case "a bool match missing the true arm is non-exhaustive"
  (doc    "The mirror of the case above: a match on a Bool arming only `false` leaves `true`
           uncovered and the compiler MUST reject it as non-exhaustive (CDZ0210). Pins that
           exhaustiveness is checked for BOTH bool values, not only the one the sole arm happens to
           name.")
  (input  (do
            (def (f b) (match b (false 0)))
            (def (main) (f true)) (export main)))
  (error  CDZ0210))

(case "a bool match on a constant scrutinee is non-exhaustive even when the constant hits the sole arm"
  (doc    "`(match true (true 1))` — the scrutinee is the COMPILE-TIME CONSTANT `true`, and the sole arm
           `true` is exactly the value it holds. Exhaustiveness is still checked against the TYPE's value
           set (both `true` and `false`), not against which value the constant scrutinee happens to be:
           the arm set leaves `false` uncovered and there is no wildcard, so the match is non-exhaustive
           and the compiler MUST reject it (CDZ0210). This is the constant-scrutinee, present-arm form —
           distinct from the parameter-scrutinee cases above (a dynamic scrutinee) and the companion of
           the constant-sum present-arm case below: a static-scrutinee compile path that returns the arm
           the constant matches must NOT skip the arm-set-vs-type exhaustiveness check just because the
           constant hit a present arm. Exhaustiveness is a property of the arm set against the type, not
           of the scrutinee's value.")
  (input  (match true (true 1)))
  (error  CDZ0210))

; A sum type's value set is its variant set, so exhaustiveness for a sum match is checked against
; ALL its variants — not just the scrutinee's runtime value. `Option` has variants Some and None;
; a match arming only `Some` leaves `None` uncovered, so it is non-exhaustive and the compiler MUST
; reject it (CDZ0210) EVEN when the scrutinee happens to be a `Some`. Exhaustiveness is a
; compile-time property of the arm set against the sum's variant set, not of which variant the
; scrutinee holds. The bool cases above are the two-value instance of the same rule; these are the
; general sum instance.

(case "a sum match missing a variant is non-exhaustive even when the scrutinee is the covered one"
  (doc    "`Option` has variants Some and None. `(match (Some 5) ((Some x) x))` arms only Some, leaving
           None uncovered and having no wildcard — non-exhaustive, so the compiler MUST reject it
           (CDZ0210), independent of the scrutinee being a Some. Exhaustiveness is a compile-time
           property of the arm set against the sum's variant set, not of which variant the scrutinee
           holds.")
  (input  (match (Some 5) ((Some x) x)))
  (error  CDZ0210))

(case "a Sign match missing two of three variants is non-exhaustive"
  (doc    "Sign has three variants (Neg | Zero | Pos). `(match (Sign.Pos unit) ((Sign.Pos _) 1))`
           arms only Pos, leaving Neg and Zero uncovered — non-exhaustive, so the compiler MUST reject
           it (CDZ0210). Pins that a sum's exhaustiveness covers every declared variant, not only the
           one the constant scrutinee names — a three-variant sum with a single arm is rejected just
           as a two-variant one is.")
  (input  (match (Sign.Pos unit) ((Sign.Pos _) 1)))
  (error  CDZ0210))

; An Int64's value set is all 2^64 of its values, so no finite set of literal arms covers it — a match
; on an Int64 with only literal arms and no wildcard is non-exhaustive exactly as a Bool match missing an
; arm or a sum match missing a variant is. The rule is the same one the bool and sum cases above pin
; (core-semantics.md #Matching Is Exhaustive Or Rejected: "cover every value of the scrutinee's type"),
; applied to the third scrutinee kind: exhaustiveness is a property of the ARM SET against the TYPE, not
; of the scrutinee's value. In particular a COMPILE-TIME CONSTANT Int64 scrutinee that hits a present arm
; does NOT excuse the missing coverage: `(match 5 (5 1))` folds the scrutinee to `5` and the sole arm
; names `5`, but the arm set still leaves every other Int64 uncovered and there is no wildcard, so the
; match is non-exhaustive and MUST be rejected (CDZ0210). This is the Int64 companion of the
; constant-scrutinee, present-arm bool case (§"a bool match on a constant scrutinee is non-exhaustive
; even when the constant hits the sole arm") and sum case above: a static-scrutinee compile path that
; returns the arm the constant matches must NOT skip the arm-set-vs-type exhaustiveness check just
; because the constant hit a present arm. The DYNAMIC-scrutinee form — `(match x (5 1))` for a parameter
; `x` — is already rejected; the constant-scrutinee form is the one a value-driven shortcut mis-accepts.

(case "an int match on a constant scrutinee is non-exhaustive even when the constant hits the sole arm"
  (doc    "`(match 5 (5 1))` — the scrutinee is the COMPILE-TIME CONSTANT `5`, and the sole arm `5` is
           exactly the value it holds. Exhaustiveness is still checked against the TYPE's value set (all
           2^64 Int64 values), not against which value the constant scrutinee happens to be: a finite set
           of literal arms cannot cover Int64, and there is no wildcard, so the match is non-exhaustive
           and the compiler MUST reject it (CDZ0210). This is the Int64 companion of the constant-scrutinee
           present-arm bool case and sum case above (core-semantics.md #Matching Is Exhaustive Or
           Rejected). A static-scrutinee compile path that returns the arm the constant matches must NOT
           skip the arm-set-vs-type exhaustiveness check just because the constant hit a present arm — the
           same value-driven shortcut the bool path had before it was fixed. The dynamic-scrutinee form
           `(match x (5 1))` for a parameter `x` is already rejected; the constant-scrutinee present-arm
           form is the one this pins. A generation that does not yet check int-literal exhaustiveness on a
           constant scrutinee declines rather than emitting a component (reject-don't-miscompile).")
  (input  (match 5 (5 1)))
  (error  CDZ0210))

; Exhaustiveness composes into NESTED patterns, not only the top-level variant set. core-semantics.md
; #Patterns Compose (a constructor pattern's binder MAY itself be a constructor pattern, matched
; recursively) with #Matching Is Exhaustive Or Rejected ("cover every value of the scrutinee's type"):
; a value of type `Option (Option Int64)` ranges over `(Some (Some _))`, `(Some (None _))`, and `(None
; _)`, so a match arming `(Some (Some x))` and `(None _)` — but NOT `(Some (None _))` — leaves a value of
; the type uncovered and is non-exhaustive (CDZ0210), exactly as a flat match missing a top-level variant
; is. The check must descend into the nested constructor position: the OUTER `Some` is covered, but its
; payload's own variant set (`Some | None`) is not, so the composed arm set does not cover the type. A
; compiler that checks exhaustiveness only at the top level (outer `Some`/`None` both named) accepts the
; ill-typed program; worse, one that checks against the CONSTANT scrutinee's nested shape rather than the
; TYPE accepts `(match (Some (Some 5)) …)` because the constant hits `(Some (Some x))` — the same
; value-driven shortcut the constant-scrutinee cases above pin, here at the nested level. (The dynamic
; form and the constant-is-the-uncovered-case form are already rejected — `(match (Some (None unit))
; ((Some (Some x)) x) ((None _) -1))` declines; the constant-hits-a-covered-arm form is the one this pins.)
; A generation that does not yet check nested exhaustiveness declines rather than emitting a component.

(case "a nested sum match missing an inner variant is non-exhaustive"
  (doc    "`(match (Some (Some 5)) ((Some (Some x)) x) ((None _) -1))` arms the outer `Some` (with an inner
           `Some`) and the outer `None`, but leaves `(Some (None _))` uncovered — a value of the scrutinee
           type `Option (Option Int64)` that no arm matches and no wildcard catches, so the match is
           non-exhaustive and MUST be rejected (CDZ0210, core-semantics.md #Matching Is Exhaustive Or
           Rejected with #Patterns Compose: exhaustiveness composes into the nested constructor position).
           Pins that the exhaustiveness check descends into a nested pattern — the outer `Some` is covered,
           but its payload's variant set `Some | None` is not, so the composed arm set does not cover the
           type. This is the nested companion of the flat sum-missing-a-variant case above; a compiler that
           checks only the top-level variant set, or checks against the constant scrutinee's nested shape
           (`(Some (Some 5))` hits `(Some (Some x))`) rather than the type, accepts the ill-typed program.
           A generation that does not yet check nested exhaustiveness declines rather than emitting.")
  (input  (match (Some (Some 5))
            ((Some (Some x)) x)
            ((None _)        -1)))
  (error  CDZ0210))

(case "nested patterns deconstruct recursively"
  (doc    "Witnesses core-semantics.md #Pattern Matching: patterns can nest — a constructor pattern
           inside another constructor pattern. (Some (tuple a b)) matches a Some whose payload is a
           tuple, binding both elements. The compiler uses this to deconstruct nested AST structures.")
  (input  (match (Some (tuple 3 7))
            ((Some (tuple a b)) (+ a b))
            ((None _)           0)))
  (output (: 10 Int64)))

(case "nested patterns with literals"
  (doc    "Witnesses core-semantics.md #Pattern Matching: nested patterns can combine constructors
           and literals. (Some 0) matches Some carrying exactly 0 — the literal refines the match.")
  (input  (match (Some 0)
            ((Some 0) "zero")
            ((Some _) "nonzero")
            ((None _) "none")))
  (output (: "zero" String)))

(case "a literal inside a constructor pattern matches a runtime payload"
  (doc    "core-semantics.md #Pattern Matching + #Matching Is Exhaustive Or Rejected: a literal nested
           inside a constructor pattern must be tested against the payload's RUNTIME value, exactly as
           a top-level literal pattern is. Here the payload `n` is a function parameter (not known at
           compile time); `(Some n)` with n=0 must match `(Some 0)` and yield 100, not fall through to
           the binding arm `(Some k)`. Companion to \"nested patterns with literals\" above, whose
           scrutinee `(Some 0)` is a compile-time constant — this one pins the same refinement when the
           payload is only known at run time. The `((None _) …)` arm is present because exhaustiveness
           is against the TYPE's variant set, not the scrutinee's known variant (the sibling case \"a sum
           match missing a variant is non-exhaustive even when the scrutinee is the covered one\").")
  (input  (do
            (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k) ((None _) -1)))
            (def (main) (f 0)) (export main)))
  (output (: 100 Int64)))

(case "a non-matching literal inside a constructor pattern binds the runtime payload"
  (doc    "The companion of the case above: with n=7 the literal arm `(Some 0)` does not match, so the
           binding arm `(Some k)` binds k=7 and yields 7. Confirms the nested literal is a genuine
           runtime test (matching for 0, falling through otherwise) rather than always-taken or
           always-skipped. The `((None _) …)` arm keeps the match exhaustive against `Option`'s variant
           set (see the case above).")
  (input  (do
            (def (f n) (match (Some n) ((Some 0) 100) ((Some k) k) ((None _) -1)))
            (def (main) (f 7)) (export main)))
  (output (: 7 Int64)))

(case "a boolean literal inside a constructor pattern refines the match"
  (doc    "The bool-payload companion: a variant carrying a `Bool` payload can be matched against a
           boolean LITERAL. `(F.S true)` matches `F.S` carrying exactly `true`; `(F.S k)` binds otherwise
           (core-semantics.md #Pattern Matching, the literal refines the match). For a runtime `b=true`
           the `(F.S true)` arm fires → 1. Pins that a literal payload test works for a Bool payload, not
           only Int — the get-bool + i32 compare sibling of the Int literal test.")
  (input  (do
            (type F (S Bool) C)
            (def (f b) (match (F.S b) ((F.S true) 1) ((F.S k) 0) ((F.C _) -1)))
            (def (main) (f true)) (export main)))
  (output (: 1 Int64)))

(case "a literal inside an Ok pattern refines a Result match"
  (doc    "The Result companion: `(Ok 0)` matches `Ok` carrying exactly `0`, `(Ok k)` binds otherwise,
           `(Err e)` covers the error variant. For a runtime `n=3` the literal arm `(Ok 0)` does not
           match, so `(Ok k)` binds k=3 → 3. Pins that a literal payload test composes with the
           two-variant Result sum exactly as with Option.")
  (input  (do
            (def (f n) (match (Ok n) ((Ok 0) 100) ((Ok k) k) ((Err e) -1)))
            (def (main) (f 3)) (export main)))
  (output (: 3 Int64)))

(case "a literal inside a NESTED constructor pattern refines the match"
  (doc    "The nested-literal companion: `(Some (Some 0))` tests the INNER payload against the literal
           `0`. `(Some (Some 0))` fires only when the doubly-wrapped value is exactly 0; `(Some (Some x))`
           binds otherwise. For a runtime n=7 the literal arm does not match, so the binder arm yields 7.
           Pins that a literal test at a DEEP payload path (`[Payload, Payload]`) works — the literal
           refinement composes with the decision tree's nested descent.")
  (input  (do
            (def (f n) (match (Some (Some n))
                         ((Some (Some 0)) 99)
                         ((Some (Some x)) x)
                         ((Some (None _)) -1)
                         ((None _)        -2)))
            (def (main) (f 7)) (export main)))
  (output (: 7 Int64)))

(case "a literal inside a tuple pattern matches a runtime element"
  (doc    "core-semantics.md #Pattern Matching: the same refinement inside a tuple pattern. `(tuple n
           9)` with a runtime n; the arm `(tuple 0 y)` matches only when the first element is 0. With
           n=0 it matches and yields 100; the literal element is tested against the runtime value, not
           treated as a binder.")
  (input  (do
            (def (f n) (match (tuple n 9) ((tuple 0 y) 100) ((tuple x y) x)))
            (def (main) (f 0)) (export main)))
  (output (: 100 Int64)))

; --- A tuple pattern's arity must match the scrutinee's tuple arity ----------------------
; core-semantics.md #A Tuple Is Deconstructible By Pattern Matching (`(tuple a b)` binds the
; elements): a tuple pattern deconstructs a tuple of the SAME arity. A pattern `(tuple a b c)` has a
; three-element tuple shape, which can NEVER match a two-element tuple scrutinee — the pattern and
; scrutinee shapes are statically incompatible, a type error (CDZ0201), exactly as a `(Some x)`
; pattern against an Int64 scrutinee is. A wrong-arity tuple pattern is ill-typed, not a runtime
; non-match: the compiler rejects it, and a generation that does not yet check a tuple pattern's
; arity against the scrutinee's declines rather than running the program (reject-don't-miscompile).

(case "a tuple pattern of the wrong arity is a type error"
  (doc    "`(tuple a b c)` is a three-element tuple pattern; the scrutinee `(tuple 1 2)` is a
           two-tuple. A three-element pattern can never match a two-element tuple — their shapes are
           statically incompatible, so the arm is ill-typed and the compiler MUST reject the match
           (CDZ0201). Pins that a tuple pattern's arity is checked against the scrutinee's, not
           silently failed.")
  (input  (match (tuple 1 2) ((tuple a b c) a) (_ 0)))
  (error  CDZ0201))

(case "a one-element tuple pattern against a two-tuple is a type error"
  (doc    "The other direction: `(tuple a)` is a one-element tuple pattern, which cannot match the
           two-tuple `(tuple 1 2)` — a static shape mismatch, CDZ0201. Pins that BOTH too-many and
           too-few pattern elements are a type error, not a runtime non-match.")
  (input  (match (tuple 1 2) ((tuple a) a) (_ 0)))
  (error  CDZ0201))

; The tuple-pattern-arity rule applies RECURSIVELY, at every nesting depth, not only to the outermost
; tuple pattern. core-semantics.md #Patterns Compose: a tuple pattern MUST admit any pattern in each of
; its binder positions — its element "MAY itself be … a tuple pattern … matched recursively to any depth." So a
; nested `(tuple b c d)` at a position whose scrutinee element is a two-tuple `(tuple 2 3)` is the same
; wrong-arity shape mismatch the top-level cases above pin — a three-element tuple pattern can never
; match a two-element tuple — and MUST be rejected CDZ0201, not silently fail and fall through to a
; wildcard. A compiler that checks only the OUTERMOST tuple pattern's arity against the scrutinee's
; (and not the arity of each nested tuple pattern against the corresponding nested scrutinee element)
; lets the ill-typed nested arm slip past: `(match (tuple 1 (tuple 2 3)) ((tuple a (tuple b c d)) 9)
; (_ 0))` runs to 0 (the arm silently not-matching) where it MUST reject, exactly the "silent non-match"
; the flat cases forbid. A generation that does not yet check nested pattern arity declines rather than
; running the program (reject-don't-miscompile).

(case "a nested tuple pattern of the wrong arity is a type error"
  (doc    "`(tuple a (tuple b c d))` is a tuple pattern whose second element is a three-element tuple
           pattern; matched against `(tuple 1 (tuple 2 3))`, that nested pattern faces a two-element
           tuple — a static shape mismatch, CDZ0201, exactly as the flat `(tuple a b c)` vs `(tuple 1 2)`
           case above. The arity rule composes recursively (core-semantics.md #Patterns Compose — a tuple pattern's element MAY itself be a tuple
           pattern, matched to any depth), so the nested arm is ill-typed and MUST be rejected, not
           silently fail and fall through to the wildcard yielding 0. Pins that a compiler checking only
           the OUTERMOST tuple pattern's arity does not let a nested wrong-arity pattern slip past as a
           runtime non-match.")
  (input  (match (tuple 1 (tuple 2 3)) ((tuple a (tuple b c d)) 9) (_ 0)))
  (error  CDZ0201))

; The recursion covers a nested LITERAL pattern's type too, not only a nested tuple's arity. A literal
; pattern matches by equality, defined only WITHIN one type (core-semantics.md #Equality Is Structural),
; so a literal pattern whose type differs from the value at its position can never match — CDZ0201 at the
; top level (§"a literal pattern's type must match the scrutinee's"), and the same at every nested binder
; position (core-semantics.md #Patterns Compose — a tuple pattern's element MAY itself be a literal pattern,
; checked recursively). `(tuple true b)` puts a Bool literal `true` at position 0, whose scrutinee element
; is the Int64 `1`; the arm is ill-typed and MUST be rejected, not silently fail to the wildcard yielding 0.

(case "a nested literal pattern of the wrong type is a type error"
  (doc    "`(tuple true b)` matched against `(tuple 1 2)` puts the Bool literal `true` at a position whose
           scrutinee element is the Int64 `1` — a literal-pattern-type mismatch (core-semantics.md #Equality
           Is Structural: equality is within one type), CDZ0201, exactly as the top-level `(match 5 (true 1)
           …)` case is rejected. The rule composes to nested binder positions (core-semantics.md #Patterns
           Compose), so the nested literal type is checked against the corresponding scrutinee element, not
           only the outermost. Pins that a compiler checking only the top-level literal pattern's type does
           not let a nested wrong-type literal slip past as a runtime non-match falling to the wildcard.")
  (input  (match (tuple 1 2) ((tuple true b) 9) (_ 0)))
  (error  CDZ0201))

; The recursion must also enter a tuple pattern nested UNDER A CONSTRUCTOR pattern, not only one at the
; arm's root. A constructor pattern's binder MAY itself be a tuple pattern (core-semantics.md #Patterns
; Compose), so `(Some (tuple a b c))` carries a three-element tuple pattern in `Some`'s payload position.
; Matched against `(Some (tuple 1 2))`, whose payload is a two-element tuple, that nested tuple pattern is
; the same wrong-arity shape mismatch the flat and tuple-nested cases pin — CDZ0201 — reached through the
; constructor's binder rather than a tuple element. A compiler whose shape check descends only through
; tuple patterns (entering only when the arm's pattern is a `(tuple …)` at the root) never reaches a tuple
; pattern sitting under a `Some`/`Ok`/user constructor, and lets the ill-typed arm slip past to a wildcard.

(case "a wrong-arity tuple pattern nested under a constructor pattern is a type error"
  (doc    "`(Some (tuple a b c))` carries a three-element tuple pattern in `Some`'s payload binder; matched
           against `(Some (tuple 1 2))`, whose payload is a two-element tuple, the nested pattern faces a
           two-tuple — a static arity mismatch (CDZ0201), the same rule as the tuple-nested and flat cases,
           reached through a constructor's binder (core-semantics.md #Patterns Compose — a constructor
           pattern's binder MAY itself be a tuple pattern, matched to any depth). Pins that the recursive
           shape check enters a tuple pattern nested under a constructor pattern, not only one at the arm's
           root, so the ill-typed arm is rejected rather than silently failing to the wildcard yielding 0.")
  (input  (match (Some (tuple 1 2)) ((Some (tuple a b c)) 9) (_ 0)))
  (error  CDZ0201))

; A pattern's KIND must also match the scrutinee's kind, not only a tuple's arity: a tuple pattern
; against a SUM scrutinee (or a sum/constructor pattern against a tuple) is a static shape mismatch.
; A `(tuple a b)` pattern deconstructs a tuple; a `Some`/`Ok`/`Sign.Pos` value is a sum, so the tuple
; pattern can never match it — CDZ0201, the same shape-mismatch class as a wrong-arity tuple pattern
; or a type-mismatched literal pattern above. (A literal pattern vs a sum/tuple scrutinee, and a
; constructor pattern vs a tuple/scalar scrutinee, are already rejected; this pins the tuple-pattern-
; vs-sum-scrutinee direction.)

(case "a tuple pattern against a sum scrutinee is a type error"
  (doc    "`(tuple a b)` is a tuple pattern; the scrutinee `(Some 5)` is a sum value. A tuple pattern
           deconstructs a tuple, so it can never match a sum — the arm's shape is statically
           incompatible with the scrutinee, a type error (CDZ0201). Pins the pattern-KIND check
           (tuple vs sum), the companion of the tuple-ARITY check above.")
  (input  (match (Some 5) ((tuple a b) a) (_ 0)))
  (error  CDZ0201))

(case "a tuple pattern against a Sign scrutinee is a type error"
  (doc    "The companion with a user-facing sum: `(Sign.Pos unit)` is a sum value, so a `(tuple a b)`
           pattern against it is a shape mismatch (CDZ0201). Pins that the tuple-pattern-vs-sum check
           holds for every sum, not only Option.")
  (input  (match (Sign.Pos unit) ((tuple a b) a) (_ 0)))
  (error  CDZ0201))

(case "deeply nested pattern matching"
  (doc    "The compiler pattern-matches over nested AST: a list node containing a name node.
           Patterns nest arbitrarily deep.")
  (input  (do
            (type Expr (Lit Int64) (Add (Tuple Expr Expr)))
            (let ((e (Expr.Add (tuple (Expr.Lit 1) (Expr.Lit 2)))))
              (match e
                ((Expr.Lit n) n)
                ((Expr.Add (tuple (Expr.Lit a) (Expr.Lit b))) (+ a b))
                ((Expr.Add _) 0)))))
  (output (: 3 Int64)))

; --- Matching a RUNTIME scrutinee ---------------------------------------------------
; Witnesses core-semantics.md #Matching Is Exhaustive Or Rejected for scrutinees whose
; value is NOT known at compile time — a function parameter or a computed expression. The
; matching arm must be selected from the scrutinee's RUNTIME value, exactly as when the
; scrutinee is an inline literal (cases above). These are core (functions + match are core):
; the compiler that dispatches instruction opcodes matches on runtime-computed byte values.

(case "an integer literal pattern matches a runtime scrutinee"
  (doc    "The scrutinee `n` is a function parameter — its value (0) is not known until run
           time. The first arm's literal pattern 0 must match the runtime value 0 and select
           its body, exactly as it would for an inline literal scrutinee. This is the base-case
           dispatch every recursive function over integers relies on.")
  (input  (do
            (def (classify n) (match n (0 100) (1 200) (_ 900)))
            (def (main) (classify 0)) (export main)))
  (output (: 100 Int64)))

(case "a two-arm match does not evaluate the unselected arm's trapping body"
  (doc    "A 2-arm `match` with leaf-value bodies may be lowered to a branchless `select` (both bodies on
           the stack, the discriminant chooses) — but ONLY when both bodies are trap-free. `(match n (0 (/
           1 z)) (_ 99))` has a trapping body `(/ 1 z)` in the first arm, so it MUST keep the branch: with
           n = 5 the wildcard arm is selected → 99, and the first arm's division by zero (z = 0) is NOT
           evaluated. A naive branchless-select that evaluated both bodies would trap here. Pins that the
           2-arm-match-to-select optimization does not treat a trapping arm body as a select leaf — the
           match evaluates only the selected arm (core-semantics.md #Matching Is Exhaustive Or Rejected +
           the trap-observation rule). The anchor: with n = 0 the first arm IS selected and it traps.")
  (input  (do
            (def (main (: n Int64) (: z Int64)) (match n (0 (/ 1 z)) (_ 99)))
            (export main)))
  (call   main (: 5 Int64) (: 0 Int64))
  (output (: 99 Int64)))

(case "a runtime scrutinee selects a non-first literal arm"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: arms are tried top-to-bottom
           and the first whose pattern matches the runtime value wins. Here the runtime value 2
           skips the 0 and 1 arms and selects the 2 arm — not the else.")
  (input  (do
            (def (classify n) (match n (0 10) (1 20) (2 30) (_ 99)))
            (def (main) (classify 2)) (export main)))
  (output (: 30 Int64)))

(case "a negative integer literal pattern matches a runtime scrutinee"
  (doc    "A negative literal pattern matches by equality against the runtime value, like any
           other integer literal.")
  (input  (do
            (def (classify n) (match n (-1 100) (_ 200)))
            (def (main) (classify -1)) (export main)))
  (output (: 100 Int64)))

(case "an earlier literal arm is chosen over a later name-binding arm for a runtime scrutinee"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected + #Bindings Introduced By A
           Pattern Are Scoped To Its Branch: a bare name pattern `k` matches anything and binds
           the whole scrutinee, but only if reached. With the runtime value 0, the earlier
           literal arm `0` matches first, so the name arm is never entered.")
  (input  (do
            (def (f n) (match n (0 100) (k (+ k 1))))
            (def (main) (f 0)) (export main)))
  (output (: 100 Int64)))

(case "a name pattern binds the runtime scrutinee when no literal arm matches"
  (doc    "The companion to the case above: with the runtime value 41 no literal arm matches,
           so the name arm `k` binds k=41 and its body computes 42. Confirms the name arm and
           the literal arm are selected consistently from the same runtime value.")
  (input  (do
            (def (f n) (match n (0 100) (k (+ k 1))))
            (def (main) (f 41)) (export main)))
  (output (: 42 Int64)))

(case "a match on a computed runtime value dispatches on the result"
  (doc    "The scrutinee is the expression `(% n 2)`, computed at run time. Its value (0 for an
           even n) selects the literal arm 0. Exercises a match whose scrutinee is neither a
           literal nor a variable but an arbitrary runtime expression — the parity dispatch a
           LEB128 encoder performs.")
  (input  (do
            (def (parity n) (match (% n 2) (0 0) (_ 1)))
            (def (main) (parity 4)) (export main)))
  (output (: 0 Int64)))

(case "a match on a record-field-access scrutinee dispatches on the field value"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected + #Member Access Projects A Record
           Field: the match scrutinee is `(. r n)`, a member access whose value is 5. The literal arm
           5 must match that value and yield 100 — the scrutinee's value is what is matched, whether it
           is written as a literal, a variable, an arithmetic expression, or a field projection.
           (Binding the field to a name first and matching that already works; matching the projection
           directly must behave identically.)")
  (input  (let ((r (record (n 5))))
            (match (. r n)
              (5 100)
              (_ 200))))
  (output (: 100 Int64)))

(case "a match on a tuple-element-access scrutinee dispatches on the element value"
  (doc    "The tuple companion of the case above: the scrutinee `(. t 0)` projects element 0 (value
           5), which the literal arm 5 must match, yielding 100. A positional access is a scrutinee
           value like any other.")
  (input  (let ((t (tuple 5 9)))
            (match (. t 0)
              (5 100)
              (_ 200))))
  (output (: 100 Int64)))

(case "a match on a record field selects a later literal arm"
  (doc    "Confirms the field-access scrutinee is matched against EACH literal arm, not just skipped to
           the wildcard: with r.n = 6, the 5 arm is passed over and the 6 arm selected, yielding 300.")
  (input  (let ((r (record (n 6))))
            (match (. r n)
              (5 100)
              (6 300)
              (_ 200))))
  (output (: 300 Int64)))

(case "a nested match on a runtime scrutinee"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: a match body may itself be a
           match on the same runtime scrutinee. Both selections are driven by the runtime value
           0, so the inner match's 0 arm is chosen and the result is 7.")
  (input  (do
            (def (f n) (match n (0 (match n (0 7) (_ 8))) (_ 9)))
            (def (main) (f 0)) (export main)))
  (output (: 7 Int64)))

; The case above nests a match in a match ARM (both on the same scrutinee). A match may also take
; another match's RESULT as its SCRUTINEE — `(match (match …) …)` — the outer match dispatching on the
; value the inner match produced. This is the compiler idiom of dispatching on a sub-dispatch's result
; (classify, then act on the classification). The inner match's selected value crosses into the outer as
; an ordinary scrutinee value; core-semantics.md #Matching Is Exhaustive Or Rejected applies at each
; level. Distinct from the same-scrutinee nesting above: here the inner match is EVALUATED and its value
; consumed, not a body reached after the outer already matched.

(case "a match takes another match's result as its scrutinee"
  (doc    "The scrutinee of the outer match is itself a match: `(match 1 (1 (Some 7)) (_ (None unit)))`
           evaluates to `(Some 7)`, which the outer match deconstructs, binding x=7. Pins that a match's
           scrutinee may be a match RESULT — the sub-dispatch is evaluated and its value consumed as an
           ordinary scrutinee, the compiler idiom of dispatching on a classification.")
  (input  (match (match 1 (1 (Some 7)) (_ (None unit)))
            ((Some x) x)
            ((None _) 0)))
  (output (: 7 Int64)))

(case "a wildcard in a nested pattern position ignores that element"
  (doc    "core-semantics.md #Pattern Matching: a `_` wildcard may appear at a NESTED position, matching
           anything there without binding. `(Some (tuple _ b))` matches a Some whose payload is a pair,
           ignoring the first element and binding `b` to the second — here 2. Pins that the wildcard is
           positional inside a compound pattern, not only a top-level catch-all arm.")
  (input  (match (Some (tuple 1 2))
            ((Some (tuple _ b)) b)
            ((None _)           0)))
  (output (: 2 Int64)))

(case "a runtime scrutinee matching no arm traps"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: a match on an Int64 arming only 1
           and 2, with no wildcard/else, cannot be proven to cover every Int64 value, so it is
           non-exhaustive and the compiler MUST reject it at compile time (CDZ0210) rather than emit a
           component that could trap at run time. The rejection is the recorded outcome; the program
           does not run.")
  (input  (do
            (def (f n) (match n (1 10) (2 20)))
            (def (main) (f 3)) (export main)))
  (error  CDZ0210))

(case "a boolean literal pattern matches a runtime scrutinee"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected over the two Bool values, with
           the scrutinee a runtime function parameter. `not` is a total match on true/false —
           exhaustive, so no else is needed and no generation rejects it.")
  (input  (do
            (def (negate b) (match b (true false) (false true)))
            (def (main) (negate true)) (export main)))
  (output (: false Bool)))

(case "a two-arm Bool match selects its second (false) arm"
  (doc    "The else-branch companion of the `negate` case above: `(negate false)` takes the `false`
           arm, yielding `true`. A wildcard-less exhaustive Bool match emits its LAST arm as the
           unconditional else (once the `true` probe fails, `false` is the only value left), so this
           pins that the second arm's value is produced — not a dangling fallthrough. Together with the
           `(negate true)` case it exercises both selections of the two-arm Bool match.")
  (input  (do
            (def (negate b) (match b (true false) (false true)))
            (def (main) (negate false)) (export main)))
  (output (: true Bool)))

; A scalar `match` with MANY literal arms (≥4) lowers to a jump table rather than an if/probe chain. These
; pin the two positions the table lowering must get right: (1) in TAIL position — the match value IS the
; result — each arm is selected by the runtime scrutinee and produces its value; (2) in NON-TAIL position
; — the match value is CONSUMED by surrounding code — the match must YIELD into the enclosing expression,
; so the following code runs. The ≥4-arm NON-TAIL case was once miscompiled — a jump-table arm branched
; ONE BLOCK PAST the match's result-join to the function result, escaping the consumer; each arm now
; branches to the match's own `$join` block (`n_arms - k`, not `n_arms - k + 1`), so the following code
; runs in every position. These pin the ≥4-arm non-tail case (the fix), the ≤3-arm non-tail case (a
; distinct if/probe-chain lowering), and the ≥4-arm TAIL case — the whole boundary.

(case "a many-arm scalar match in tail position selects each arm by a runtime scrutinee"
  (doc    "A FOUR-arm scalar match `(match a (0 10) (1 20) (2 30) (_ 40))` as the whole function body (TAIL
           position), driven by a runtime scrutinee `a`: each literal arm and the wildcard is selected in
           turn — a=0 → 10, a=1 → 20, a=2 → 30, a=9 → 40. Pins that the many-arm (jump-table) lowering
           dispatches to the correct arm for every scrutinee and produces that arm's value as the result —
           the opcode/tag-dispatch idiom a compiler's evaluator leans on, exercised across all arms.")
  (input  (do
            (def (main (: a Int64)) (match a (0 10) (1 20) (2 30) (_ 40)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: 1 Int64)) (output (: 20 Int64))
  (call   main (: 2 Int64)) (output (: 30 Int64))
  (call   main (: 9 Int64)) (output (: 40 Int64)))

(case "a many-arm match consumed in non-tail position yields into the enclosing expression"
  (doc    "A FOUR-arm match `(match a (0 10) (1 20) (2 30) (_ 40))` (a jump-table lowering) consumed by
           `(+ … 100)` — its value is NOT the function result, so it must yield into the addition and
           `+ 100` must run: a=0 → 110, a=2 → 130, a=9 → 140. This was a SILENT WRONG-VALUE miscompile
           (valid wasm): a jump-table arm branched ONE BLOCK PAST the match's result-join to the FUNCTION
           result, so the arm value became the whole result and `+ 100` never ran (a=0 → 10). The default
           arm, which falls through to the join with no branch, was unaffected (a=9 → 140 was already
           right) — masking the bug. Fixed: each arm branches to the match's own `$join` block. The 3-arm
           operand case above (a different lowering) and the ≥4-arm TAIL case both worked throughout — this
           pins the ≥4-arm NON-tail position, the shape a compiler's 4+-way tag dispatch used as an operand
           takes.")
  (input  (do
            (def (main (: a Int64)) (+ (match a (0 10) (1 20) (2 30) (_ 40)) 100))
            (export main)))
  (call   main (: 0 Int64)) (output (: 110 Int64))
  (call   main (: 2 Int64)) (output (: 130 Int64))
  (call   main (: 9 Int64)) (output (: 140 Int64)))

(case "a many-arm match let-bound then consumed yields into the enclosing expression"
  (doc    "The same jump-table lowering reached through a LET binding: `(let ((m (match a …4 arms…)))
           (+ m 100))`. The escape was not operand-specific — a let-bound then-used ≥4-arm match dropped the
           `+ 100` too (a=1 → 20 instead of 120), because the arm branch still escaped the match's join.
           Fixed alongside the operand case. a=1 → 120, a=9 → 140. Pins that the fix covers a match whose
           value is bound and later consumed, not only one directly in an operator's operand slot.")
  (input  (do
            (def (main (: a Int64)) (let ((m (match a (0 10) (1 20) (2 30) (_ 40)))) (+ m 100)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 120 Int64))
  (call   main (: 9 Int64)) (output (: 140 Int64)))

(case "a three-arm match consumed as an operand yields into the enclosing expression"
  (doc    "A THREE-arm match `(match a (0 10) (1 20) (_ 40))` consumed by `(+ … 100)` — the match value is
           NOT the function result, so the match must yield into the addition and `+ 100` must run: a=0 →
           110, a=1 → 120, a=9 → 140. Pins that a match in NON-TAIL (operand) position produces its value
           into the enclosing expression rather than escaping — for the ≤3-arm (if/probe-chain) lowering, a
           DISTINCT path from the ≥4-arm jump table (the case above), so both lowerings are pinned in
           non-tail position.")
  (input  (do
            (def (main (: a Int64)) (+ (match a (0 10) (1 20) (_ 40)) 100))
            (export main)))
  (call   main (: 0 Int64)) (output (: 110 Int64))
  (call   main (: 1 Int64)) (output (: 120 Int64))
  (call   main (: 9 Int64)) (output (: 140 Int64)))

(case "a Bool match with its arms in either order is exhaustive"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: exhaustiveness of a Bool match is a
           property of the arm-value SET {true, false}, not the arm order. `(match b (false 2) (true
           1))` covers both values with the arms reversed, so it needs no wildcard. Exercised at BOTH
           runtime selections: `b` = true takes the `true` arm (1), `b` = false takes the `false` arm
           (2). Pins that the checker accepts the reversed order exactly as it accepts `(true …) (false
           …)` — the wildcard requirement is for OPEN types (Int64), never for a Bool covered by both
           literals — and that both branches select correctly at run time.")
  (input  (do (def (main (: b Bool)) (match b (false 2) (true 1))) (export main)))
  (call   main (: true Bool))
  (output (: 1 Int64))
  (call   main (: false Bool))
  (output (: 2 Int64)))

(case "a Bool match with only the true arm is non-exhaustive"
  (doc    "The negative control that pins the Bool-exhaustiveness relaxation does NOT over-accept:
           `(match b (true 1))` covers only `true`, leaving `false` unhandled — genuinely
           non-exhaustive, so it MUST reject (CDZ0210) exactly as an Int64 match without a wildcard
           does. A Bool match is exhaustive only when BOTH `true` and `false` arms are present; a single
           Bool literal is not enough. (An Int64 match without a wildcard likewise stays rejected — the
           relaxation is specific to a Bool scrutinee covered by both of its two values.)")
  (input  (do (def (main (: b Bool)) (match b (true 1))) (export main)))
  (error  CDZ0210))

(case "a match on a runtime integer scrutinee producing a boolean"
  (doc    "core-semantics.md #Matching Is Exhaustive Or Rejected: the scrutinee is a runtime integer
           but the arm bodies are Bool — a match is an expression of whatever type its arms yield,
           not restricted to the scrutinee's type. `is-zero` maps 0 → true, else → false; is-zero(0)
           = true. The Bool result must cross the run boundary as the program's value (compare the
           Bool-returning function cases in 09-functions.sexp — same result-kind requirement, reached
           through a match rather than a call).")
  (input  (do
            (def (is-zero n) (match n (0 true) (_ false)))
            (def (main) (is-zero 0)) (export main)))
  (output (: true Bool)))

(case "two sum arms with textually identical bodies still bind their own variant's payload"
  (doc    "An optimization that collapses a match whose arm bodies are all the same to that one body must
           not treat two arms as identical when their bodies REFERENCE a per-arm binder: `((N.I x) (+ x 1))`
           and `((N.J x) (+ x 1))` are textually the same `(+ x 1)`, but `x` binds the `I` payload in the
           first arm and the `J` payload in the second — they are NOT the same body. With `b` = false the
           scrutinee is `(N.J 9)`, so the taken arm's `x` is 9 and the result is 10; a collapse that fused
           the two arms and read the first arm's payload slot would wrongly yield 6 (the `I` payload 5 + 1).
           Pins that the all-same-body collapse is keyed on the body AFTER binder resolution, so an arm that
           binds a different sub-value is a distinct body.")
  (input  (do
            (type N (I Int64) (J Int64))
            (def (main (: b Bool))
              (match (if b (N.I 5) (N.J 9)) ((N.I x) (+ x 1)) ((N.J x) (+ x 1))))
            (export main)))
  (call   main (: false Bool))
  (output (: 10 Int64)))

; A match is an expression of ONE type — all its arm bodies must agree, exactly as a conditional's two
; branches must (core-semantics.md #Matching Is Exhaustive Or Rejected makes a match an expression whose
; type is what its arms yield; #Conditionals Evaluate One Branch requires "every branch … type-checked
; whether or not it is evaluated"). So arm bodies of DIFFERENT type — a `1` (Int64) arm and a `true`
; (Bool) arm — make the match ill-typed (CDZ0203), whether or not the constant scrutinee selects one of
; them. A compiler that CONST-FOLDS a match on a literal scrutinee to its matching arm and emits only that
; arm — without type-checking the OTHER arms — silently accepts `(match 5 (5 1) (_ true))` and runs it to
; 1, an unevaluated arm carrying a deferred type error. This is the match analogue of the conditional
; branch-agreement check (which rejects `(if (= 5 5) 1 true)` even though the constant condition selects
; the Int64 branch): the arm-type check is on the SET of arm bodies, independent of which the scrutinee
; hits. A RUNTIME-scrutinee match already checks this ("runtime match arms differ in kind"); the gap is the
; const-folded path. A generation that does not yet check the unselected arms' types declines rather than
; emitting the arm it folded to.

(case "a match whose arm bodies have different types is a type error even when a constant scrutinee selects one"
  (doc    "`(match 5 (5 1) (_ true))` has an Int64 arm body `1` and a Bool arm body `true` — a match is an
           expression of one type, so disagreeing arm bodies are ill-typed (CDZ0203), the match analogue of
           the conditional branch-agreement cases (`(if … 1 true)` is rejected). The constant scrutinee `5`
           selects the Int64 arm, so a compiler that const-folds the match to its matching arm and emits
           only that arm — without type-checking the other arms — silently accepts this and runs it to 1,
           an unevaluated arm carrying a deferred type error (core-semantics.md #Conditionals Evaluate One
           Branch: every branch is type-checked whether or not evaluated; the same for a match's arms). A
           runtime-scrutinee match already rejects arms that differ in kind; this pins the const-folded
           path. A generation that does not yet check the unselected arms declines rather than emitting the
           folded arm.")
  (input  (match 5 (5 1) (_ true)))
  (error  CDZ0203))

; The unselected-arm check must type-check each arm's BODY for internal errors, not only compare the arms'
; RESULT types. The case above pins arm-type-AGREEMENT (an Int64 arm vs a Bool arm); this pins that an
; unselected arm whose body is INTERNALLY ill-typed — `(+ 1 true)`, an Int64/Bool arithmetic mismatch —
; is rejected too, even though the constant scrutinee selects the other arm. core-semantics.md
; #Conditionals Evaluate One Branch: "Every branch … MUST be type-checked whether or not it is evaluated,
; so that an unevaluated branch cannot carry a deferred error" — the same for a match's arms. The `if`
; form already catches this in its unselected branch (`(if true 1 (+ 1 true))` is rejected "operation on
; mismatched types"); the const-folded match does not — it takes the unselected arm's result type
; superficially (here Int64, which agrees with the selected arm) WITHOUT checking the body, so the internal
; `(+ 1 true)` deferred type error slips through and the program runs to 1. A generation that does not yet
; type-check an unselected arm body declines rather than emitting the folded arm.

(case "an internally ill-typed unselected match arm body is a type error"
  (doc    "`(match 5 (5 1) (_ (+ 1 true)))` — the unselected `_` arm body `(+ 1 true)` mixes Int64 and Bool,
           an internal type error the compiler MUST reject (CDZ0203), even though the constant scrutinee `5`
           selects the `1` arm. Distinct from the arm-type-AGREEMENT case above: there the two arms' result
           types disagree; here an arm's BODY is internally ill-typed while its result type (Int64) agrees
           with the selected arm. core-semantics.md #Conditionals Evaluate One Branch requires every branch
           type-checked whether or not evaluated, and the same holds for a match's arms — the `if` form
           already rejects `(if true 1 (+ 1 true))`. Pins that the const-folded match type-checks each arm's
           BODY, not only compares arm result types. A generation that does not yet check the unselected
           arm's body declines rather than emitting the folded arm.")
  (input  (match 5 (5 1) (_ (+ 1 true))))
  (error  CDZ0203))

; The unselected-arm check must reach a SCOPE error, not only a type error. The two cases above pin that
; a const-folded match type-checks its unselected arms (agreement + internal type); this pins that it
; also SCOPE-checks them — an unbound name in an unselected arm is rejected (CDZ0101), exactly as the
; `if` form already rejects an unbound name in its unselected branch (`(if true 1 undefined-name)` above).
; core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing binding MUST be a
; compile-time error" (unconditional); #Conditionals Evaluate One Branch: every branch type-checked
; whether or not evaluated — the same for a match's arms, and scope resolution reaches an unselected arm
; as the type check does. `(match 2 (1 undefined-z) (_ 99))` selects the `_` arm (scrutinee 2 ≠ 1), but
; the `1` arm references the unbound `undefined-z`; the program MUST be rejected CDZ0101, not run to 99.
; The seed const-folds the match to its selected arm and scope-checks ONLY that arm, so the unbound
; reference in the dropped arm slips and it runs to 99 — the scope-check analogue of the type-check the
; case above pins, and the match companion of the `if` unselected-branch scope case. This is the more
; fundamental gap: it swallows CDZ0101, the front-end check every generation makes ("scope resolution
; needs no static typing"), on the const-folded match path. (The `if` form scope-checks its dropped
; branch correctly; only the const-folded match drops the unselected arm's scope check.) A generation
; that scope-checks every arm before folding declines rather than emitting the folded arm.
(case "an unbound name in an unselected match arm is still rejected"
  (doc    "`(match 2 (1 undefined-z) (_ 99))` references the unbound name `undefined-z` in the `1` arm;
           the scrutinee 2 selects the `_` arm, but the program MUST be rejected (CDZ0101,
           core-semantics.md #Binding Is Lexical — unconditional — with #Conditionals Evaluate One Branch:
           every arm checked whether or not evaluated). An unevaluated arm cannot carry a deferred scope
           error, exactly as an unevaluated `if` branch cannot (`(if true 1 undefined-name)` is rejected
           above). The seed const-folds the match to its selected arm and scope-checks only that arm, so
           the unbound reference in the dropped `1` arm slips and it runs to 99 — swallowing CDZ0101, the
           front-end check every generation makes. This is the match companion of the `if`
           unselected-branch scope case, and the scope-check analogue of the unselected-arm TYPE cases
           above (which the seed already enforces). A generation that scope-checks every arm before
           folding declines rather than emitting the folded arm.")
  (input  (match 2 (1 undefined-z) (_ 99)))
  (error  CDZ0101))

; The arm-type-agreement check must fire when a RUNTIME scrutinee's first arm body is a bare PAYLOAD
; BINDER — the const-folded cases above assume "a runtime-scrutinee match already rejects arms that
; differ in kind," but that check SLIPS when the first arm's body is just the payload variable it binds,
; and the match then MISCOMPILES rather than merely running. `(match o ((Some x) x) ((None _) true))`
; over a runtime `o : Option Int64` has a `Some` arm body `x` (Int64, the payload) and a `None` arm body
; `true` (Bool) — disagreeing arm types, so the match is ill-typed (CDZ0201) exactly as `(match 5 (5 1)
; (_ true))` is, and as the conditional `(if … 1 true)` is. But the seed accepts it and, worse, the
; Int64 payload is REINTERPRETED as a Bool across the run boundary: `(f (Some 5))` yields `true`,
; `(f (Some 42))` yields `false` — the payload's bits read as a boolean, neither the true Int value nor a
; rejection. The tell that isolates the gap: make the first arm body ANYTHING but a bare binder and the
; check fires — a literal `((Some x) 99)` rejects "match arm bodies have different types", an arithmetic
; `((Some x) (+ x 0))` rejects "runtime sum match arms differ in kind" — only the bare-binder first arm
; `((Some x) x)` slips. And an inline constant scrutinee (`(match (Some 5) ((Some x) x) ((None _) true))`)
; const-folds to the correct Int 5, so the defect is specific to the RUNTIME-scrutinee + bare-binder-first-
; arm path — the exact path the "runtime match already checks this" assumption relies on. It is a wrong
; VALUE, not only a missed rejection: an Int64 payload emerges as a Bool. A generation that checks the arm
; result types on this path declines the ill-typed program rather than reinterpreting the payload's bits.
(case "a runtime-scrutinee match with a bare-binder first arm and a differently-typed second arm is a type error"
  (doc    "`(match o ((Some x) x) ((None _) true))` over a runtime `o : Option Int64` has a `Some` arm
           body `x` of type Int64 (the payload) and a `None` arm body `true` of type Bool — disagreeing
           arm types, so the match is ill-typed (CDZ0203), the same arm-agreement rule as `(match 5 (5 1)
           (_ true))` and the conditional `(if … 1 true)`. The seed accepts it and REINTERPRETS the Int64
           payload as a Bool: `(f (Some 5))` yields `true`, `(f (Some 42))` yields `false` — a wrong value
           (the payload's bits read as a boolean), not merely a missed rejection. The check fires when the
           first arm body is a literal (`((Some x) 99)` → \"match arm bodies have different types\") or an
           expression (`((Some x) (+ x 0))` → \"runtime sum match arms differ in kind\"); ONLY a bare
           payload-binder first arm slips, and only on a runtime scrutinee (an inline constant scrutinee
           const-folds to the correct Int). Falsifies the assumption that a runtime-scrutinee match already
           checks arm-type agreement. A generation that checks the arm result types declines rather than
           reinterpreting the payload.")
  (input  (do
            (def (f o) (match o ((Some x) x) ((None _) true)))
            (def (main) (f (Some 5))) (export main)))
  (error  CDZ0203))

; --- Boolean connectives (short-circuit) -------------------------------------------------
; core-semantics.md #Boolean Connectives Short-Circuit: the language offers conjunction, disjunction,
; and negation over Bool. Conjunction evaluates its right operand ONLY when the left is true;
; disjunction ONLY when the left is false — so a connective shields a trapping or effectful right
; operand exactly as an unselected conditional branch does (#Conditionals Evaluate One Branch). Each
; operand is type-checked as a Bool whether or not it is evaluated. The seed does not yet realize
; `and`/`or`/`not`, so it DECLINES these until a generation adds them; they
; desugar to short-circuit conditionals (`(and a b)` = `(if a b false)`, `(or a b)` = `(if a true b)`,
; `(not a)` = `(if a false true)`), which the seed already lowers.

(case "conjunction is true exactly when both operands are true"
  (doc    "The `and` value table over the four Bool pairs, folded to one witness: only true∧true is
           true (core-semantics.md #Boolean Connectives Short-Circuit).")
  (input  (do
            (def (row a b) (if (and a b) 1 0))
            (def (main) (+ (+ (row true true) (row true false)) (+ (row false true) (row false false)))) (export main)))
  (output (: 1 Int64)))

(case "disjunction is false exactly when both operands are false"
  (doc    "The `or` value table: only false∨false is false, so three of the four pairs are true
           (core-semantics.md #Boolean Connectives Short-Circuit).")
  (input  (do
            (def (row a b) (if (or a b) 1 0))
            (def (main) (+ (+ (row true true) (row true false)) (+ (row false true) (row false false)))) (export main)))
  (output (: 3 Int64)))

(case "negation inverts a boolean"
  (doc    "`(not true)` is false and `(not false)` is true (core-semantics.md #Boolean Connectives
           Short-Circuit).")
  (input  (do (def (main) (if (not false) (not true) true)) (export main)))
  (output (: false Bool)))

(case "conjunction shields a trapping right operand when the left is false"
  (doc    "`(and false (< (/ 1 0) 2))`: `and` evaluates its right operand ONLY when the left is true,
           so with the left false the division-by-zero trap in the right operand is NOT evaluated and
           the result is false — the connective shields the trap exactly as an unselected conditional
           branch does (core-semantics.md #Boolean Connectives Short-Circuit). Without short-circuit
           this would trap.")
  (input  (and false (< (/ 1 0) 2)))
  (output (: false Bool)))

(case "disjunction shields a trapping right operand when the left is true"
  (doc    "`(or true (< (/ 1 0) 2))`: `or` evaluates its right operand ONLY when the left is false, so
           with the left true the trap in the right operand is NOT evaluated and the result is true.
           The dual of the `and` shielding case (core-semantics.md #Boolean Connectives Short-Circuit).")
  (input  (or true (< (/ 1 0) 2)))
  (output (: true Bool)))

(case "a runtime conjunction still shields a comparison right operand whose subexpression traps"
  (doc    "The shielding must survive the branchless emit: an `and`/`or` whose right operand is a
           trap-free COMPARISON may be lowered to a branchless `select` (both operands evaluated) — but
           ONLY when the comparison's own operands are trap-free. `(and (= a 1) (< (/ 1 z) 5))` has a
           right operand `(< (/ 1 z) 5)` whose subexpression `(/ 1 z)` can trap, so the connective MUST
           keep short-circuiting: with `a = 99` the left `(= a 1)` is false, so the right operand is NOT
           evaluated and the division by zero (z = 0) is shielded — the result is false, not a trap. Pins
           that the branchless-connective optimization does not treat a comparison with a trapping
           subexpression as a trap-free leaf; the left operand is a RUNTIME value, so this is the emit-path
           shielding the constant-fold cases above cannot witness.")
  (input  (do
            (def (main (: a Int64) (: z Int64)) (if (and (= a 1) (< (/ 1 z) 5)) 1 0))
            (export main)))
  (call   main (: 99 Int64) (: 0 Int64))
  (output (: 0 Int64)))

(case "a boolean connective with a non-boolean operand is a type error"
  (doc    "`(and true 1)` gives an Int64 where a Bool operand is required. core-semantics.md #Boolean
           Connectives Short-Circuit: each operand is type-checked as a Bool whether or not it is
           evaluated, so the compiler MUST reject the non-Bool operand (CDZ0201) rather than run — the
           same discipline as a conditional's branch type-check, applied to a connective's operand.")
  (input  (and true 1))
  (error  CDZ0201))

(case "a recursive function that threads a tuple accumulator returns it"
  (doc    "A recursive function whose result is a TUPLE in every branch — a `(value, cursor)` accumulator
           threaded through the recursion — MUST compile and return that tuple. `go` returns `(tuple acc 0)`
           at the base and, in the recursive branch, matches a helper's tuple `(pair n)` and recurses with an
           updated accumulator; the result kind is a tuple on both branches, so the function is tuple-valued
           throughout. `(go 3 0)` sums 3+2+1 into `acc`, yielding `(tuple 6 0)`, and `a` = 6. A generation
           whose return-kind inference does not recognize the recursive branch as tuple-valued declines
           (\"runtime sum match without a constructor arm\" — the tuple match is misread as a sum match when
           the tuple comes from a call and the arm recurses); but a tuple-threading recursion is an ordinary
           function, load-bearing for any recursive-descent walk that threads a (node, position) cursor.")
  (input  (do
            (def (go n acc)
              (if (= n 0)
                  (tuple acc 0)
                  (match (pair n) ((tuple v k) (go (- n 1) (+ acc v))))))
            (def (pair n) (tuple n n))
            (def (main) (match (go 3 0) ((tuple a b) a))) (export main)))
  (output (: 6 Int64)))

(case "a tail-recursive function returning a tuple is tuple-valued"
  (doc    "The MINIMAL isolation of the case above — no accumulator, no helper, no heap: a tail-recursive
           function whose branches are both a TUPLE MUST be tuple-valued, so a match on its result
           destructures the tuple. `(go 3)` recurses to the base `(tuple 0 0)`; `(+ a b)` = 0. A generation
           whose return-kind inference does not carry the base branch's tuple kind back through the
           tail-recursive call declines (\"runtime sum match without a constructor arm\" — the recursive call
           site is 'unknown tuple shape', so the result's tuple match is misread as a sum match). The trigger
           is precisely TAIL-RECURSION + a TUPLE return: a non-recursive tuple return compiles, and a
           NON-tail recursive function that WRAPS its recursive result in a new tuple compiles; only the
           tail-recursive tuple return does not. This is the return-kind companion of the tail-recursive
           SCALAR accumulator inference (realized) — a tuple result must infer the same way a scalar does.")
  (input  (do
            (def (go n) (if (< n 1) (tuple 0 0) (go (- n 1))))
            (def (main) (match (go 3) ((tuple a b) (+ a b)))) (export main)))
  (output (: 0 Int64)))

(case "a mutually-recursive decoder returns a heap value and cursor and its heap slot is dispatched"
  (doc    "The MUTUAL-RECURSION sibling of the tail-recursive tuple return above. `dn` (decode-node) and
           `dac` (decode-children) are mutually recursive: `dn` returns `(tuple <Ast> <cursor>)` — a HEAP
           sum value paired with an Int cursor — and `dac` matches `dn`'s tuple `((tuple child nx) …)` and
           recurses. `top` destructures `dn`'s result to the HEAP slot `ast`, and `main` dispatches on it
           with CONSTRUCTOR patterns. The return-kind inference must carry the tuple's per-slot kinds — a
           heap slot (the `Ast`) stays Heap, a scalar slot (the cursor) is Int — back through the MUTUAL
           recursion, so `top`'s result is a runtime sum (Heap) and the caller's `((AInt n) …)` takes the
           runtime-sum-match path. A generation that infers the heap slot as a scalar declines 'runtime
           match with a non-literal pattern' (the constructor pattern is read against a scalar) or 'cannot
           infer runtime compound result shape' — ask-77, the mutual-recursion face of the tail-recursive
           tuple return. `(dn (list 42 7) 0)` at i=0 yields `(tuple (AInt 42) 1)`; `top` returns `(AInt 42)`;
           `main` reads 42. `List.at`+`Option.expect` keeps the element a genuine runtime value (unfolded).")
  (input  (do
            (type Ast (AInt Int64) ALeaf (AList (List Ast)))
            (def (dn b i)
              (if (= i 0)
                  (tuple (AInt (Option.expect (List.at b 0) "in range")) (+ i 1))
                  (tuple (AList (dac b i (- i 1) (list))) (+ i 1))))
            (def (dac b i n acc)
              (if (< n 1)
                  acc
                  (match (dn b i) ((tuple child nx) (dac b nx (- n 1) (List.push acc child))))))
            (def (top b) (match (dn b 0) ((tuple ast pos) ast)))
            (def (main) (match (top (list 42 7)) ((AInt n) n) (_ -1))) (export main)))
  (output (: 42 Int64)))

; --- A binding position accepts an irrefutable pattern ---------------------------------------
; core-semantics.md #A Binding Position Accepts An Irrefutable Pattern: a `let` binder (and a parameter)
; MAY hold an irrefutable pattern in place of a bare name, binding the names it introduces to the
; corresponding sub-values of the bound value — exactly as the same pattern would in a single match arm
; over that value. A bare name and a wildcard are the trivial irrefutable patterns; a tuple pattern whose
; every element is irrefutable is irrefutable, recursively to any depth (#Patterns Compose). This is the
; ergonomic form of the bind-then-rematch idiom the decoder cases above pay by hand — `(let ((r v)) (match
; r ((tuple a b) …)))` becomes `(let (((tuple a b) v)) …)`.

(case "a let binder may be a tuple pattern that destructures the value"
  (doc    "`(let (((tuple a b) (tuple 3 4))) (+ a b))` binds `a` and `b` to the two elements of the bound
           pair (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern) — the same binding a
           `(match (tuple 3 4) ((tuple a b) (+ a b)))` arm makes, written at the binder. Pins that a tuple
           pattern in a `let` binder position destructures the value rather than requiring a bind-then-match.")
  (input  (let (((tuple a b) (tuple 3 4))) (+ a b)))
  (output (: 7 Int64)))

(case "a tuple binding pattern nests to any depth"
  (doc    "`(let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) …)` — a tuple pattern whose second element
           is itself a tuple pattern, bound recursively (core-semantics.md #A Binding Position Accepts An
           Irrefutable Pattern / #Patterns Compose: a binder position admits any pattern). Pins that a
           binding pattern composes to any depth, exactly as a match-arm pattern does.")
  (input  (let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) (+ a (+ b c))))
  (output (: 6 Int64)))

(case "a let binder may be a single-variant-sum pattern that destructures the payload"
  (doc    "A SINGLE-VARIANT sum's sole constructor ALWAYS matches, so it is an IRREFUTABLE pattern — valid
           in a `let` binder position (core-semantics.md #A Binding Position Accepts An Irrefutable
           Pattern), exactly as a tuple pattern is. `(let (((Id.Mk n) (Id.Mk 42))) n)` binds `n` to the
           `Mk` payload — the same binding a `(match (Id.Mk 42) ((Id.Mk n) n))` arm makes, written at the
           binder. Pins that a one-variant sum destructures in a binding position (a MULTI-variant sum
           there is refutable → CDZ0210, the rejection below), the sum companion of the tuple destructure.")
  (input  (do
            (type Id (Mk Int64))
            (def (main) (let (((Id.Mk n) (Id.Mk 42))) n))
            (export main)))
  (output (: 42 Int64)))

(case "a single-variant-sum binding pattern destructures a multi-payload constructor positionally"
  (doc    "The multi-payload companion: `(let (((P.Mk a b) (P.Mk 5 6))) (+ a b))` binds `a` and `b` to the
           two payloads of the single-variant `P.Mk` (its payloads box as one tuple, matched positionally,
           exactly as a `(P.Mk a b)` match arm does). Pins that a single-variant binding pattern binds each
           payload position, not only a one-payload newtype.")
  (input  (do
            (type P (Mk Int64 Int64))
            (def (main) (let (((P.Mk a b) (P.Mk 5 6))) (+ a b)))
            (export main)))
  (output (: 11 Int64)))

(case "a single-variant-sum binding pattern nests inside another"
  (doc    "A single-variant pattern nests, like a tuple one: `(let (((W.Wrap (Id.Mk n)) (W.Wrap (Id.Mk 9))))
           …)` destructures the outer `Wrap` then the inner `Mk`, binding `n` two payload levels deep
           (core-semantics.md #Patterns Compose). `n + 1` = 10.")
  (input  (do
            (type Id (Mk Int64))
            (type W (Wrap Id))
            (def (main) (let (((W.Wrap (Id.Mk n)) (W.Wrap (Id.Mk 9)))) (+ n 1)))
            (export main)))
  (output (: 10 Int64)))

(case "a multi-variant-sum binding pattern is refutable and rejected"
  (doc    "The contrast to the single-variant cases above: a MULTI-variant sum's constructor pattern in a
           binding position is REFUTABLE — the other variants are uncovered and there is no alternative arm
           — so it is rejected (CDZ0210), not accepted. `(let (((C.A n) (C.A 5))) n)` over `(type C (A
           Int64) B)` leaves `B` uncovered. Only a single-variant sum earns the binding-position exemption;
           a many-variant sum's destructure must be a `match`. Pins the refutability boundary.")
  (input  (do
            (type C (A Int64) B)
            (def (main) (let (((C.A n) (C.A 5))) n))
            (export main)))
  (error  CDZ0210))

(case "a later let binding sees an earlier pattern's binders"
  (doc    "`(let (((tuple a b) (tuple 3 4)) (c (+ a b))) c)` — the second binding's initializer `(+ a b)`
           references `a` and `b`, the binders the first (destructuring) binding introduced
           (core-semantics.md #The Bindings Of One `let` Take Effect In Order: each initializer observes the
           bindings written before it). Pins that a destructuring binder is in scope for the bindings that
           follow, the multi-binding-let idiom the decoder threads.")
  (input  (let (((tuple a b) (tuple 3 4)) (c (+ a b))) c))
  (output (: 7 Int64)))

(case "a destructuring let over a runtime value binds its parts"
  (doc    "`(def (f p) (let (((tuple a b) p)) (+ a b)))` destructures the RUNTIME parameter `p` (not a
           literal tuple) at the binder, then `(f (tuple 10 20))` = 30 (core-semantics.md #A Binding
           Position Accepts An Irrefutable Pattern). Pins that the destructure reads the bound value at run
           time, not only when it folds to a constant.")
  (input  (do (def (f p) (let (((tuple a b) p)) (+ a b))) (def (main) (f (tuple 10 20))) (export main)))
  (output (: 30 Int64)))

; A LIST binding pattern. A list pattern is irrefutable ONLY in the REST form `(list p… .. rest)` — it
; matches ANY length ≥ the leading count (and `(list .. all)` matches every list), so it may bind in a
; `let` binder or a `def`/`fn` parameter, exactly as a `(match v ((list x .. rest) …))` arm does. A leading
; element resolves to `SumPayload{Elem(i)}` and the rest binder to `SumPayload{RestFrom(lead)}` reading out
; of the bound value (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern / #A List Is
; Deconstructed By Element Patterns With An Optional Rest). A FIXED-ARITY `(list a b)` binding is refutable
; (it matches only its exact length) → CDZ0210, the rejection below.

(case "a def parameter may be a list rest pattern binding the head"
  (doc    "`(def (head (list x .. rest)) x)` names the head of its list argument directly — a list REST
           pattern is irrefutable (matches any non-empty list here), so it is a valid PARAMETER pattern
           (core-semantics.md #A Binding Position Accepts An Irrefutable Pattern). The parameter is
           desugared to a destructuring `let`, so `x` resolves to `SumPayload{Elem(0)}` reading the first
           element of the runtime list. `head` of `(list 7 8 9)` = 7.")
  (input  (do (def (head (list x .. rest)) x) (def (main) (head (list 7 8 9))) (export main)))
  (output (: 7 Int64)))

(case "a let binder may be a list rest pattern binding a leading element and the rest"
  (doc    "`(let (((list a b .. rest) xs)) …)` binds the first two elements of the runtime list `xs` and the
           remaining elements as the sublist `rest` (core-semantics.md #A List Is Deconstructed By Element
           Patterns With An Optional Rest) — the ergonomic form of the bind-then-`match` fold. Here `drop2`
           binds `a`/`b` (dropped) and sums `rest` via a recursive `match` consumer: over `(list 1 2 3 4)`,
           `rest` is `(list 3 4)` → 7. Pins that a rest binder in a BINDING position is a usable sublist,
           not only a match-arm one.")
  (input  (do
            (def (sum (: xs (List Int64))) (match xs ((list) 0) ((list x .. rest) (+ x (sum rest)))))
            (def (drop2 ys) (let (((list a b .. rest) ys)) (sum rest)))
            (def (main) (drop2 (list 1 2 3 4)))
            (export main)))
  (output (: 7 Int64)))

(case "a fixed-arity list binding pattern is refutable and rejected"
  (doc    "The contrast to the rest form: a FIXED-ARITY `(list a b)` binding pattern matches ONLY lists of
           that exact length, so it is REFUTABLE — a binding position has no alternative arm, so it is the
           non-exhaustive error the equivalent single-arm match raises (CDZ0210, core-semantics.md #A
           Binding Position Accepts An Irrefutable Pattern). Only the rest form `(list p… .. rest)`, which
           matches any length ≥ the leading count, earns the binding-position exemption; a length-fixed
           destructure must be a `match`. Pins the list refutability boundary.")
  (input  (do (def (main) (let (((list a b) (list 1 2))) (+ a b))) (export main)))
  (error  CDZ0210))

; The refutable / ill-shaped / non-linear rejections. A binding position has no alternative arm, so its
; pattern MUST be irrefutable and its shape MUST match the value's type (core-semantics.md #A Binding
; Position Accepts An Irrefutable Pattern).

(case "a refutable constructor pattern in a let binder is rejected"
  (doc    "`(let (((Some x) (Some 5))) x)` — a `Some` pattern is refutable (the `None` variant is
           uncovered), and a binding position has no alternative arm, so it is the non-exhaustive error the
           equivalent single-arm `(match (Some 5) ((Some x) x))` raises: CDZ0210 (core-semantics.md #A
           Binding Position Accepts An Irrefutable Pattern / #Matching Is Exhaustive Or Rejected). Pins that
           a multi-variant constructor cannot bind a value in a `let`.")
  (input  (let (((Some x) (Some 5))) x))
  (error  CDZ0210))

(case "a literal in a let binder is refutable and rejected"
  (doc    "`(let ((0 5)) 42)` — a literal pattern matches one value, not every value of its type, so it is
           refutable and rejected in a binding position (CDZ0210, core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern). Pins that a literal cannot stand where a binder is expected.")
  (input  (do (def (main) (let ((0 5)) 42)) (export main)))
  (error  CDZ0210))

; Refutability is checked RECURSIVELY, at every nesting depth — a refutable sub-pattern nested inside a
; tuple binding position is rejected exactly as the top-level one is (core-semantics.md #A Binding Position
; Accepts An Irrefutable Pattern: "a tuple pattern is irrefutable ONLY when every element is"). The
; refutability check must not stop at the top level: a literal or multi-variant-constructor element makes
; the whole binding refutable, so it is CDZ0210, not a silent no-op that drops the refutable sub-pattern.

(case "a literal nested in a tuple let-binder is refutable and rejected"
  (doc    "`(let (((tuple 0 b) (tuple 0 9))) b)` puts the literal `0` in the first element of a tuple
           BINDING pattern. A literal is refutable, so a binding position rejects it (CDZ0210) exactly as the
           top-level `(let ((0 5)) 42)` does — the check recurses into tuple sub-patterns. A compiler that
           stopped at the top level ran it to 9, silently treating the literal element as a no-op.")
  (input  (do (def (main) (let (((tuple 0 b) (tuple 0 9))) b)) (export main)))
  (error  CDZ0210))

(case "a literal nested in a tuple def-parameter is refutable and rejected"
  (doc    "`(def (f (tuple 0 b)) b)` — a tuple-pattern parameter desugars to a `(let (((tuple 0 b) p)) …)`
           binder, so the literal `0` in the first element is refutable and rejects CDZ0210. Calling
           `(f (tuple 9 5))` with a first element that does NOT equal 0 must not run to 5 (no compile
           rejection, no runtime trap) — the parameter's binding position enforces irrefutability like a
           `let` binder.")
  (input  (do (def (f (tuple 0 b)) b) (def (main) (f (tuple 9 5))) (export main)))
  (error  CDZ0210))

(case "a multi-variant constructor nested in a tuple let-binder is refutable and rejected"
  (doc    "`(let (((tuple (Some x) b) (tuple (Some 5) 9))) x)` puts the multi-variant constructor pattern
           `(Some x)` in a tuple binding element. A multi-variant ctor is refutable (the `None` variant is
           uncovered) — the top-level `(let (((Some x) (Some 5))) x)` rejects CDZ0210, so the nested form
           does too. The recursion classifies each element with the same rule the top-level binder uses.")
  (input  (do (def (main) (let (((tuple (Some x) b) (tuple (Some 5) 9))) x)) (export main)))
  (error  CDZ0210))

(case "a deeply nested literal in a tuple let-binder is refutable and rejected"
  (doc    "`(let (((tuple a (tuple 0 b)) (tuple 1 (tuple 0 3)))) (+ a b))` — the literal `0` is TWO tuple
           levels deep, in the second element's own tuple pattern. Refutability recurses to any depth, so
           the deep literal is CDZ0210 exactly as a top-level one is. Pins that the recursion does not stop
           after one tuple level (contrast the irrefutable `(tuple a (tuple b c))` binder above, which
           composes to any depth and RUNS).")
  (input  (do (def (main) (let (((tuple a (tuple 0 b)) (tuple 1 (tuple 0 3)))) (+ a b))) (export main)))
  (error  CDZ0210))

(case "a wrong-arity tuple binding pattern is a shape error"
  (doc    "`(let (((tuple a b c) (tuple 1 2))) a)` — a three-element tuple pattern cannot match a
           two-element value: a static shape mismatch (CDZ0201, core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern), the same code the wrong-arity tuple MATCH arm gets. Pins that a
           binding pattern's arity is checked against the bound value's type.")
  (input  (let (((tuple a b c) (tuple 1 2))) a))
  (error  CDZ0201))

(case "a tuple binding pattern against a non-tuple value is a shape error"
  (doc    "`(let (((tuple a b) 5)) a)` — a tuple pattern cannot match a scalar `Int64` value: a kind
           mismatch (CDZ0201, core-semantics.md #A Binding Position Accepts An Irrefutable Pattern). Pins
           that a tuple binding pattern requires a tuple value.")
  (input  (let (((tuple a b) 5)) a))
  (error  CDZ0201))

(case "a non-linear tuple binding pattern is rejected"
  (doc    "`(let (((tuple x x) (tuple 1 2))) x)` binds `x` twice in one binding pattern — not linear, so it
           is the same CDZ0102 error a non-linear MATCH pattern gets (core-semantics.md #A Binding Position
           Accepts An Irrefutable Pattern / #Bindings Introduced By A Pattern Are Scoped To Its Branch).
           Pins that linearity is enforced in binding position, not only in a match arm.")
  (input  (let (((tuple x x) (tuple 1 2))) x))
  (error  CDZ0102))

; A binding pattern MAY carry a type ANNOTATION `(: <pat> <Type>)` (type-system.md #Annotations Constrain,
; Never Contradict): the annotation constrains the bound value's type and the inner pattern is the real
; binder. A contradiction is CDZ0203, the same code any annotation-vs-value mismatch gets.

(case "an annotated let binder constrains the value's type"
  (doc    "`(let (((: x Int64) 5)) x)` — the binder `x` is annotated `Int64`, which agrees with the value
           `5`, so `x` binds 5 (type-system.md #Annotations Constrain, Never Contradict). Pins that a `let`
           binder MAY carry a `(: <name> <Type>)` annotation, the binder analogue of an annotated
           parameter `(def (f (: x Int64)) …)`.")
  (input  (let (((: x Int64) 5)) x))
  (output (: 5 Int64)))

(case "an annotated destructuring let binder"
  (doc    "`(let (((: (tuple a b) (Tuple Int64 Int64)) (tuple 3 4))) (+ a b))` — the annotation constrains
           the whole tuple before the pattern takes it apart, then `a`/`b` bind its elements (7). Pins that
           the annotation wraps a DESTRUCTURING binder, not only a bare name.")
  (input  (let (((: (tuple a b) (Tuple Int64 Int64)) (tuple 3 4))) (+ a b)))
  (output (: 7 Int64)))

(case "an annotated let binder that contradicts the value is rejected"
  (doc    "`(let (((: x Bool) 5)) x)` annotates `x` `Bool` but binds it to the Int64 `5` — a contradiction
           the compiler MUST reject (CDZ0203, type-system.md #Annotations Constrain, Never Contradict: an
           annotation participates in inference as a constraint, and a value that cannot satisfy it is a
           type error). Pins that a binder's annotation is CHECKED against the value, not merely recorded.")
  (input  (do (def (main) (let (((: x Bool) 5)) x)) (export main)))
  (error  CDZ0203))

; A FUNCTION PARAMETER is a binding position too (core-semantics.md #A Binding Position Accepts An
; Irrefutable Pattern): `(def (f (tuple a b)) …)` names the two halves of its single pair argument, keeping
; ARITY ONE. The compiler realizes this by a load-time rewrite to a fresh whole-value parameter + a
; destructuring `let` over the body — the SAME desugar the annotated variant `(: (tuple a b) T)` takes,
; keeping the annotation on the fresh binder and the tuple destructuring on its value. The ML syntax
; surface parses `def f((a, b)) = …` and its annotated form `def f((a, b): T) = …`, so these survive the
; `sexpr → ml → sexpr` round-trip gate.

(case "a tuple-pattern parameter binds the halves of its pair argument"
  (doc    "`(def (f (tuple a b)) (+ a b))` — a destructuring parameter names the two elements of its single
           pair argument, keeping arity one, exactly as the equivalent `let` binder `(let (((tuple a b) p))
           …)` does. Calling `(f (tuple 3 4))` binds `a`=3, `b`=4 and yields 7. Pins the parameter face of
           the binding-pattern capability the `let` cases above witness.")
  (input  (do
            (def (f (tuple a b)) (+ a b))
            (def (main) (f (tuple 3 4))) (export main)))
  (output (: 7 Int64)))

; The tuple-pattern-parameter cases here pass a CONSTANT tuple `(f (tuple 3 4))` from a nullary entry, so
; the tuple folds and the destructure is compile-time. These pin the RUNTIME face: the argument tuple is
; built from a boundary parameter (so it cannot fold — a real heap tuple), and the parameter pattern
; destructures it at run time (a `tuple<…>` read back into its binders). The runtime companion of the
; constant destructure, the shape a compiler pass takes when a callee receives a (node, cursor) pair.

(case "a tuple-pattern parameter destructures a runtime-built tuple"
  (doc    "`(def (add (tuple a b)) (+ a b))` applied to a tuple built from a boundary parameter
           `(add (tuple x (+ x 1)))` — the tuple cannot fold, so `add`'s parameter destructures a real heap
           tuple at run time, binding `a`=x and `b`=x+1. x=5 → 5+6 = 11; x=100 → 201. Pins the runtime face
           of tuple-parameter destructuring, distinct from the constant `(f (tuple 3 4))` fold above.")
  (input  (do
            (def (add (tuple a b)) (+ a b))
            (def (main (: x Int64)) (add (tuple x (+ x 1)))) (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64))
  (call   main (: 100 Int64)) (output (: 201 Int64)))

(case "a nested tuple-pattern parameter destructures a runtime tuple"
  (doc    "The nested form: `(def (f (tuple a (tuple b c))) …)` destructures a runtime `(tuple x (tuple (+ x
           1) (+ x 2)))` — the outer pair's second element is itself a pair, bound `b`=x+1, `c`=x+2. x=5 →
           5 + (6 + 7) = 18. Pins that a nested destructuring parameter reads a nested heap tuple at run
           time, its inner binders resolving down the extended access path.")
  (input  (do
            (def (f (tuple a (tuple b c))) (+ a (+ b c)))
            (def (main (: x Int64)) (f (tuple x (tuple (+ x 1) (+ x 2))))) (export main)))
  (call   main (: 5 Int64)) (output (: 18 Int64)))

(case "a tuple-pattern parameter over a tuple threaded from a helper call"
  (doc    "The destructured tuple arrives from ANOTHER function's return, not built inline: `mk(x)` returns
           `(tuple x (- 0 x))` and `sum`'s tuple parameter destructures it — `(sum (mk x))` = x + (-x) = 0
           for every x. Pins that a callee's tuple-pattern parameter destructures a tuple produced by a
           prior call (the (node, cursor)-pair-threaded-through-a-pass shape), the return-boundary companion
           of the inline runtime destructure.")
  (input  (do
            (def (mk (: x Int64)) (tuple x (- 0 x)))
            (def (sum (tuple a b)) (+ a b))
            (def (main (: x Int64)) (sum (mk x))) (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64))
  (call   main (: 40 Int64)) (output (: 0 Int64)))

(case "an annotated tuple-pattern parameter binds its pattern's names"
  (doc    "`(def (f (: (tuple a b) (Tuple Int64 Int64))) (+ a b))` is a destructuring tuple parameter that
           ALSO carries a type annotation. Its binders `a`/`b` must be in scope in the body, exactly as the
           un-annotated `(def (f (tuple a b)) …)` binds them. The annotated form desugars to a fresh
           annotated binder `(: p T)` plus a destructuring `let` over the inner tuple, so the annotation
           constrains the argument AND the halves bind. Calling `(f (tuple 3 4))` gives 7. (Without peeling
           the `(: pattern T)` annotation the desugar left `a`/`b` unbound — CDZ0101 — even though the
           un-annotated and the annotated-plain-binder forms both work; only their combination broke, and
           the ML printer emits exactly this form.)")
  (input  (do
            (def (f (: (tuple a b) (Tuple Int64 Int64))) (+ a b))
            (def (main) (f (tuple 3 4))) (export main)))
  (output (: 7 Int64)))

(case "an annotated tuple-pattern parameter still checks its annotation against the argument"
  (doc    "The annotation on a destructuring parameter is ENFORCED, not silently dropped: `(def (f (: (tuple
           a b) (Tuple Int64 Bool))) a)` declares the second element `Bool`, but `(f (tuple 3 4))` passes an
           Int64 there — a contradiction (CDZ0203, type-system.md #Annotations Constrain, Never Contradict),
           exactly as an annotated `let` binder `(let (((: x Bool) 5)) x)` is rejected. Pins that peeling the
           annotation to reach the tuple pattern keeps the annotation live on the fresh binder.")
  (input  (do
            (def (f (: (tuple a b) (Tuple Int64 Bool))) a)
            (def (main) (f (tuple 3 4))) (export main)))
  (error  CDZ0203))
