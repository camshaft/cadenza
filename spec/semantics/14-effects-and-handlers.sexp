; Effects and handlers — witnesses capabilities-and-effects.md. An effect is declared with (effect
; <name> (op <op> <type>)…): a ROUTING-AGNOSTIC CONTRACT that names the effect and types its operations
; and says NOTHING about where it is discharged. Routing is decided by the nearest enclosing router: a
; (handle <effect> <init> ((<op> (params…) <state> body)…) body) discharges ONE effect IN-PROGRAM — its
; head names that effect and every arm is one of that effect's operations (discharging several effects is
; NESTED handles) — and it does NOT appear in the manifest, while an entrypoint (host (<effect>…) body)
; DELEGATES it to the component
; boundary as a plain imported-function call the host resolves (the host is its terminal handler; it enters
; the manifest as the escaping row; the delegation is the grant). The SAME declared effect may be handled in
; one program and delegated in another — there is no (host) marker on the declaration and no separate import
; form. An operation is performed and handled as <name>.<op>.
;
; A HANDLER FOLDS STATE (capabilities-and-effects.md #A Handler Threads State Across The Operations It
; Discharges). Every handle SEEDS an initial state — `(handle <effect> <init> (arms…) body)` — fixed
; where the handler is installed, so nothing is ambient. Every arm names one of the effect's operations
; and binds the CURRENT state after its operation's parameters — `(<op> (params…) <state> body)` — and
; resume carries BOTH outputs:
; `(resume <value> <next-state>)` returns <value> to the point that performed the operation (one-shot) and
; threads <next-state> forward to the rest of the sub-computation. A handle EVALUATES TO THE VALUE OF ITS
; BODY; the accumulated state is observable only through the effect's own operations (a read-out is an
; ordinary operation whose arm resumes the state — there is no separate return clause). A "stateless"
; handler is the degenerate case: seed `unit`, thread `s` unchanged (Unit carries no bytes, so it costs
; nothing). Mutation is the instance that reads and updates the threaded state — the value heap stays
; immutable.
;
; HANDLER RESOLUTION IS DYNAMIC IN EXTENT (capabilities-and-effects.md #Handler Resolution Is Dynamic In
; Extent And Statically Determined): a performed operation is discharged by the nearest handler active
; along the CALL CHAIN, not the nearest one lexically enclosing the performing function's definition, so a
; function may perform an operation its CALLER discharges and the same function called under two handlers is
; discharged by each in turn — which handler is fixed statically by monomorphizing the handler context.
;
; These exercise the effect surface, which a later generation realizes; the seed realizes the mandatory
; capability floor but not the effect surface or the algebraic-handler layer (so it declines these). A
; response-returning delegated call fixes
; its response with (host-responses …) so the run is a deterministic function of input and that response.

(case "a run's result is a deterministic function of a host call's recorded response"
  (doc    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: `ask` is a routing-agnostic effect the entrypoint delegates to the host, so
           `ask.ask` is a plain imported-function call returning its response at the boundary. The
           (host-responses …) fixture supplies the response in call order; given that response the run
           deterministically computes 100. How the host produces the response — inline, fiber-suspend, or
           re-derive from the recorded responses — is host policy the program does not observe.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (* (ask.ask) 10))) (export main)))
  (host-responses (respond ask.ask (: 10 Int64)))
  (host-calls (call ask.ask))
  (output (: 100 Int64)))

(case "a host op whose result is a QUANTITY crosses the boundary as its inner scalar (unit erased)"
  (doc    "The runtime-parameter `@param` Quantity host path: a host-delegated op whose declared result is a
           `(Qty T u)` — `Env.width : Unit -> (Qty Int64 meter)` — crosses the host boundary as its INNER
           scalar (`Int64`), because a unit is a COMPILE-TIME value ERASED before codegen (`Ty::Qty` has the
           SAME runtime rep as its inner). The host supplies the magnitude (`42`) as that scalar; the guest's
           static `(Qty Int64 meter)` type carries the unit, so `Qty.value` reads the magnitude back → 42 —
           no runtime reconstruction, and a wrong-DIMENSION host value is inexpressible (the host has no unit
           channel; the unit is fixed guest-side by the op's declared type). This is what lets a
           `@param(...) width : Length` generate a Qty-result host op that binds to a browser/CLI/notebook
           value at run time (v-cad Length dimensions, v-notebook). A Qty whose inner is a heap Rational/BigInt
           still declines (a num/den boundary pair is a later increment); a scalar-inner Qty rides the existing
           scalar host boundary.")
  (input  (do
            (effect Env (op width (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main)
              (host (Env)
                (Qty.value (Env.width)))) (export main)))
  (host-responses (respond env.width (: 42 Int64)))
  (host-calls (call env.width))
  (output (: 42 Int64)))

(case "an exact RATIONAL host value crosses as two scalar num/den ops the guest recombines (#13)"
  (doc    "The num/den Qty ABI (#13): a host cannot supply a heap `Rational` directly (a compound has no host
           boundary form), so an exact-rational runtime value crosses as TWO SCALAR host ops — `rate-num :
           Unit -> Int64` and `rate-den : Unit -> Int64` — and the GUEST recombines them with `Rational.of
           (num, den)`. This reuses the fully-supported scalar host boundary (no tuple/memory/resource
           envelope surgery) and is exactly what a `@param(...) rate : Rational` (or a Rational-magnitude
           `Length`) desugars to: two scalar accessors + a guest `Rational.of`. With the host responding num=7,
           den=2, the guest builds the exact rational 7/2 (normalized). Pins that a Rational runtime value is
           expressible over the scalar host path — the operator-ruled minimal boundary form for #13 (a single
           atomic Rational host op is a documented future path, unbuilt — no consumer needs it). The result is
           a heap Rational, so `main` crosses it via the resource-escape value path.")
  (input  (do
            (effect Env (op rate-num (-> Unit Int64)) (op rate-den (-> Unit Int64)))
            (def (main)
              (host (Env)
                (Rational.of (Env.rate-num) (Env.rate-den)))) (export main)))
  (host-responses (respond env.rate-num (: 7 Int64)) (respond env.rate-den (: 2 Int64)))
  (host-calls (call env.rate-num) (call env.rate-den))
  (output (: 7/2 Rational)))

(case "a Rational-MAGNITUDE Quantity host value composes the num/den ops with the unit erasure (#13, B2)"
  (doc    "#13 B2 — the actual `@param(...) : Length` shape: a Quantity whose MAGNITUDE is an exact Rational.
           The magnitude crosses as the same TWO SCALAR num/den host ops (B1), the guest recombines them with
           `Rational.of(num, den)`, and `Qty.of(…, meter)` attaches the unit GUEST-SIDE — the unit is a
           compile-time value erased at the boundary (layer-2, the scalar-inner Qty host path), so a
           Rational-magnitude Qty needs NO extra boundary channel beyond the two scalars. Two same-unit
           `(Qty Rational meter)` values ADD (dimension-checked) — `x + x` for `x = 7/2 meter` → `7/1 meter` —
           and `Qty.value` names the result; its VALUE FORM is the bare exact rational `7/1` (the unit is a
           compile-time value, erased from the runtime value). Pins that a Rational magnitude flows through Qty
           construction + same-unit arithmetic over the scalar host path (num=7, den=2 → 7/2 meter; doubled →
           7/1). This is what a v-cad `@param Length` desugars to.")
  (input  (do
            (effect Env (op rate-num (-> Unit Int64)) (op rate-den (-> Unit Int64)))
            (def (main)
              (host (Env)
                (let ((x (Qty.of (Rational.of (Env.rate-num) (Env.rate-den)) (Unit.base #"meter"))))
                  (Qty.value (+ x x))))) (export main)))
  (host-responses (respond env.rate-num (: 7 Int64)) (respond env.rate-den (: 2 Int64)))
  (host-calls (call env.rate-num) (call env.rate-den))
  (output (: 7/1 Rational)))

; The case above fixes ONE response. On its own it cannot distinguish a run that genuinely CONSUMES the
; response value from a compiler that hardcoded 100 — both produce 100. This pair pins that the response
; VALUE flows into the result: the SAME program with a DIFFERENT response produces a DIFFERENT (but
; deterministic) result, so the run is a function OF the response, not a constant
; (capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And Responses). The third
; case pins that MULTIPLE responses combine in call order through a NON-commutative operator — swapping the
; consumption order would give -18, not 18 — so the ordered response fixture feeds the computation as
; recorded.

(case "the same program with a different response gives a different deterministic result"
  (doc    "The discriminating companion of the determinism case above: the identical program `(* (ask.ask)
           10)` with the response fixed at 7 (not 10) deterministically computes 70. Together with the
           10 → 100 case, this pins that the run genuinely CONSUMES the response value (a compiler that
           hardcoded 100 would fail here) — the result is a function OF the response, deterministic given it.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (* (ask.ask) 10))) (export main)))
  (host-responses (respond ask.ask (: 7 Int64)))
  (host-calls (call ask.ask))
  (output (: 70 Int64)))

(case "two host responses combine in call order through a non-commutative operator"
  (doc    "`(- (io.get) (io.get))` performs `io.get` twice; the ordered fixture supplies 30 then 12, so the
           FIRST call consumes 30 and the second 12, and `30 - 12` = 18. `-` is non-commutative, so a run
           that consumed the responses in the wrong order would compute `12 - 30` = -18 — the recorded 18
           pins that the two responses feed the computation in the order the fixture records them
           (capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And Responses; the
           ordered-consumption companion of the two-calls-in-order observation below).")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (main)
              (host (io)
                (- (io.get) (io.get)))) (export main)))
  (host-responses (respond io.get (: 30 Int64)) (respond io.get (: 12 Int64)))
  (host-calls (call io.get) (call io.get))
  (output (: 18 Int64)))

(case "two host calls consume their responses in order"
  (doc    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: two host calls consume two responses in the order made; the sum is a deterministic
           function of input and the ordered response sequence.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (+ (ask.ask) (ask.ask)))) (export main)))
  (host-responses (respond ask.ask (: 3 Int64))
                  (respond ask.ask (: 4 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 7 Int64)))

(case "an effectful host arg to a multi-use function parameter is evaluated ONCE, not re-performed per use"
  (doc    "Witnesses core-semantics.md #Applying A Function (the parameter binds to a single evaluated
           argument value) + capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses (the host-call sequence is deterministic). `(mk (ask.ask))` passes a HOST perform as the
           argument to `mk`, whose parameter `s` is used THREE times. Strict by-value binding evaluates the
           argument ONCE at the call and binds its value to `s` — so the run makes exactly ONE host call
           (consuming the single response 5) and the three uses read the bound 5: (+ (+ 5 5) 5) = 15. A
           call-by-name substitution would re-perform `ask.ask` per use (three calls) — a duplicated
           observable effect, which this pins against.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (mk (: s Int64)) (+ (+ s s) s))
            (def (main)
              (host (ask)
                (mk (ask.ask)))) (export main)))
  (host-responses (respond ask.ask (: 5 Int64)))
  (host-calls (call ask.ask))
  (output (: 15 Int64)))

(case "an effectful host arg flowing into a compound then a destructuring match is evaluated ONCE"
  (doc    "The compound-into-destructuring-match companion of the multi-use evaluate-once case. `(mk (ask.ask))`
           passes ONE host perform to `mk`, which builds `(T s s s)` (the arg reused three times), and `sum3`
           DESTRUCTURES that with a match binding a, b, c. Strict by-value binding + a single-materialized
           match scrutinee mean the host op runs EXACTLY ONCE (response 5, so s = 5) and the three payload
           binders read the stored value: (+ (+ 5 5) 5) = 15. A per-use re-perform (call-by-name) or a
           per-payload-binder re-emission of the match scrutinee would make three host calls — this pins one.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (type Trip (T Int64 Int64 Int64))
            (def (mk (: s Int64)) (T s s s))
            (def (sum3 (: t Trip)) (match t ((T a b c) (+ (+ a b) c))))
            (def (main)
              (host (ask)
                (sum3 (mk (ask.ask))))) (export main)))
  (host-responses (respond ask.ask (: 5 Int64)))
  (host-calls (call ask.ask))
  (output (: 15 Int64)))

(case "an abortive perform in a connective that is an if-condition abandons the computation when the connective reaches it"
  (doc    "The abortive analogue of the connective-in-condition threading, for a NON-resuming handler. `(and
           b (> (Bail.bail 7) 0))` is the CONDITION of `(if _ 100 200)`; when `b` is true the connective
           evaluates its right operand, performing the abortive `Bail.bail 7`, which abandons the whole
           computation — the handle's value is the arm value 7. Witnesses capabilities-and-effects.md
           short-circuit evaluation + abortive-handler semantics: the abort in a taken connective operand
           abandons regardless of its nesting under the enclosing if. A regression against the
           connective-in-condition abort over-declining. (The b=false short-circuit companion — the abort
           never performed — is the sibling case just below.)")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: b Bool)) (handle Bail 0 ((bail (n) s n)) (if (and b (> (Bail.bail 7) 0)) 100 200)))
            (def (main) (run true)) (export main)))
  (output (: 7 Int64)))

(case "an abortive perform short-circuited out of a connective condition is never performed"
  (doc    "The short-circuit companion: with `b` false, `(and b …)` never evaluates its right operand, so the
           abortive `Bail.bail 7` is NOT performed — no abandonment — and the outer `if` takes its else
           branch, 200. Pins that the connective-condition abort fold preserves short-circuit semantics (the
           abort does not fire on the untaken operand).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: b Bool)) (handle Bail 0 ((bail (n) s n)) (if (and b (> (Bail.bail 7) 0)) 100 200)))
            (def (main) (run false)) (export main)))
  (output (: 200 Int64)))

(case "a ctl-style arm that applies its continuation lexically folds through the delimited context"
  (doc    "The E5 within-activation continuation surface: a 5-part handler arm `(flip () s k body)` binds the
           delimited continuation `k` as a value and APPLIES it as `(k v)`. When `k` is applied lexically
           (never stored or passed on), `(k v)` returns into the delimited context — semantically identical
           to `(resume v)`. Over the whole-body perform `(Amb.flip)`, the continuation is `C = (+ □ 1)`, so
           `(k 10)` = `C[10]` = `(+ 10 1)` = 11. Witnesses capabilities-and-effects.md continuation semantics
           (a handler receives the continuation and resumes it) for the lexical `ctl` surface, distinct from
           the implicit-continuation `resume`. A `k` that ESCAPES (stored/resumed later) is a separate,
           later increment; this pins the within-activation case.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main) (handle Amb 0 ((flip () s k (+ (k 10) 1))) (Amb.flip))) (export main)))
  (output (: 11 Int64)))

(case "an abortive handler arm performed with a RUNTIME argument abandons the computation and returns it"
  (doc    "The runtime-argument companion of the constant-abort case. An abortive arm `(bail (n) s n)` never
           resumes, so performing `(Bail.bail k)` — with `k` a RUNTIME parameter, not a constant — abandons
           the enclosing `(+ 1 …)` and makes the arm value (the op argument `k`) the handle's value. Reading
           `run(7)` → 7 (the `+ 1` is discarded; the abort returns the runtime k). Witnesses that the abort
           value's type is grounded from the runtime perform argument (a reference to the enclosing param),
           so the handle result has a machine representation on both backends — a regression against the
           abort-value orphan reading its free reference unbound (a wasm-declines / rust-computes split).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: k Int64)) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail k))))
            (def (main) (run 7)) (export main)))
  (output (: 7 Int64)))

(case "a nested effectful let inlined into a re-performing body keeps its inner binder"
  (doc    "A cross-function effectful-let inline: `inner` binds an effect result in a local `let` and reads
           it in a match arm; `outer` binds `inner()` then PERFORMS AGAIN in its body. The fold inlines
           `inner` into `outer`, producing a nested `let` whose inner binder `a` must stay in scope for the
           outer body's continuation (whose threaded out-state references it). Witnesses core-semantics.md
           #Bindings Introduced By A Pattern Are Scoped To Its Branch + the strict left-fold of handler
           state: get()=10 binds a=10, put(a) sets state to 10, inner()=10; then outer adds a second get()
           =10 → 20. A regression against the nested-let inline dropping the inner binder (a spanless
           CDZ0101).")
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (inner) (let ((a (St.get))) (match (St.put a) (_ a))))
            (def (outer) (let ((b (inner))) (+ b (St.get))))
            (def (main)
              (handle St 10
                ((get (u) s (resume s s)) (put (v) s (resume unit v)))
                (outer))) (export main)))
  (output (: 20 Int64)))

(case "a delegated effect performed inside an intra-program handler"
  (doc    "Witnesses the composition of the two routings (capabilities-and-effects.md
           #A Run Is A Deterministic Function Of Its Input And Responses with #An Effect That Does Not
           Escape Is Discharged By A Handler): the entrypoint delegates `ask` to the host, and `ask.ask`
           is performed as the argument to the intra-program `Scale.by`, so a single host call returns 21,
           then the enclosing handler discharges `Scale.by 21` by resuming with `(* 21 2)` = 42. The
           `Scale` handler carries no state (seed `unit`, arm threads `s` unchanged), so its state slot is
           the degenerate Unit case. The handler is tail-resumptive, so it reifies no continuation on the
           stack — which is what lets a host that re-derives the run reconstruct the (dynamic) handler
           context by re-execution (a host that answers inline or fiber-suspends needs no reconstruction).
           The manifest is exactly `{ask}` — the delegated effect escapes and is enumerated (host-calls
           asserts the one call), while the handled `Scale` does not appear. This is the invariant that a
           host call arises only under a tail-resumptive/abortive intra-program handler, never spanning a
           reified continuation a re-derivation could not reconstruct.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Scale (op by (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle Scale unit ((by (n) s (resume (* n 2) s))) (Scale.by (ask.ask))))) (export main)))
  (host-responses (respond ask.ask (: 21 Int64)))
  (host-calls (call ask.ask))
  (output (: 42 Int64)))

(case "an in-program handler OVERRIDES an effect's peer binding (the test-mock, no peer call)"
  (doc    "Witnesses the U-pivot headline (DESIGN-cross-component-interop-rcdzc.md #UNIFY cross-component
           interop WITH EFFECTS): an effect bound to a PEER contract by a top-level `(bind Math
           \"cadenza:math/api\")` directive is normally a peer CALL — but a NEARER in-program `(handle Math
           …)` DISCHARGES it before it escapes, so the peer binding is OVERRIDDEN and no peer/host call is
           made. This is the free test-mock the unification gives: routing precedence is in-program handler
           > peer binding, exactly as an in-program handler beats a `(host …)` delegation. The mock arm
           computes `a + b + 100`, so `(Math.add 2 3)` = 105 — the handler's answer, not the peer's — and
           the empty `(host-calls)` fixture pins that the bound peer is never reached (the effect does not
           escape). Pins that `(handle E …)` is the unit-test override for a peer dependency, reusing the
           complete E0–E5 handler machinery with no peer needed.")
  (input  (do
            (effect Math (op add (-> Int64 Int64 Int64)))
            (bind Math "cadenza:math/api")
            (def (main)
              (handle Math 0 ((add (a b) s (resume (+ (+ a b) 100) s)))
                (Math.add 2 3))) (export main)))
  (output (: 105 Int64))
  (host-calls))

(case "a bound effect performed with neither a handler nor a host delegation has no home"
  (doc    "The companion to the override case above, pinning the ROUTING MODEL: `(bind Math
           \"cadenza:math/api\")` is a routing TABLE — it names WHERE the effect goes once it is delegated —
           NOT itself a delegation. What DELEGATES an effect to the boundary (and declares the capability in
           the program's manifest) is a `(host (Math) …)` clause, exactly as for a plain host effect
           (capabilities-and-effects.md #A Host Import Is A Boundary Effect And The Manifest Is Its Row;
           cross-component-interop.md #A Cross-Component Import Grants No Host Authority — a component that
           reaches a boundary operation MUST declare that capability in its OWN manifest rather than acquire
           it implicitly). So a bare `(Math.add 2 3)` performed with NO enclosing handler AND no `(host
           Math …)` is an effect reached with no home — rejected CDZ0401 — even though `Math` is bound to a
           peer: the binding says where a delegation WOULD route, but nothing here delegates. This guards
           that a `(bind …)` does not silently grant an implicit peer capability; a peer call is still an
           explicit `(host (Math) (Math.add …))` [the working peer-call path] or an in-program `(handle Math
           …)` override [the case above]. (If the routing model is ever revised so a binding itself
           delegates, this pinned rejection is the case that must be knowingly flipped.)")
  (input  (do
            (effect Math (op add (-> Int64 Int64 Int64)))
            (bind Math "cadenza:math/api")
            (def (main) (Math.add 2 3)) (export main)))
  (error  CDZ0401))

(case "an intra-program handler interposes on a delegated effect, counts it, and forwards to the boundary"
  (doc    "Witnesses capabilities-and-effects.md #A Handler May Interpose On An Effect An Entrypoint Would
           Delegate: the entrypoint delegates `ask` to the host (outermost router), but an inner handler
           arm intercepts every `ask.ask`, records it via the intra-program `Count.tick`, and re-performs
           `(ask.ask)` in tail position. The re-performance resolves against the routers enclosing the
           interposing handler's OWN declaration (the under-frame) — the `Count` handler (no match) then the
           entrypoint `host` delegation — so it forwards past this arm to the boundary rather than recursing
           into itself. The interposing arm is tail-resumptive (FORWARDING / effectful-tail: `resume` once in
           tail position, resumed expression itself performs), so it reifies no continuation: `resume e s`
           lowers to `e` (state `s` threaded) emitted under the arm's definition-site stack. `ask` still
           reaches the boundary undischarged-by-forwarding, so it IS in the manifest and host-calls asserts
           the two real host calls; `Count` is intra-program and never escapes. Both handlers here are
           stateless (seed `unit`, thread `s` unchanged): `tick` is a record-and-continue observation
           (resume-with-unit, the `Diag` idiom — a real accumulating counter would seed a non-unit state and
           thread it, per the `Diag` case below), and being intra-program it re-derives within the run, so it
           stays correct even if a host resolves the forwarded call by re-deriving the run (a host-side
           counter would instead over-count once per re-derivation). Note the router nesting is load-bearing:
           the `host (ask)` delegation and the `Count` handler must both ENCLOSE the `ask` interposer, since
           the interposer's arm performs `Count.tick` (resolved at its def site) and forwards `ask` (resolved
           past itself to the delegation); putting `Count` inside the `ask` handler would make `Count`
           out-of-scope at the arm's def site (CDZ0401). This is the test-harness / metering power move —
           observe host I/O without the performing code knowing — and the first case exercising
           re-perform-to-parent, guarding the under-frame.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Count (op tick (-> Unit Unit)))
            (def (main)
              (host (ask)
                (handle Count unit ((tick (u) s (resume unit s))) (handle ask unit ((ask () s (do (Count.tick) (resume (ask.ask) s)))) (+ (ask.ask) (ask.ask)))))) (export main)))
  (host-responses (respond ask.ask (: 3 Int64))
                  (respond ask.ask (: 4 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 7 Int64)))

(case "a handler arm interposes on another intra-program effect and forwards"
  (doc    "The purely INTRA-PROGRAM analogue of the host-forwarding interpose above (no `host` boundary):
           `A`'s arm performs an OUTER effect `Count.tick` (a record-and-continue observation), then resumes.
           The re-performed `Count.tick` resolves against the routers enclosing `A`'s handler — the outer
           `Count` handler — the under-frame discipline, exactly as with host forwarding but discharged
           in-program. `A` is seeded 5 and its arm resumes `s` (=5) unchanged; `Count` seeded 0 threads its
           counter. `(A.a)` evaluates to 5 (the outer `Count.tick` is observed as a side effect, not part of
           the value). Witnesses #A Handler May Interpose On An Effect with BOTH effects intra-program.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect Count (op tick (-> Unit Int64)))
            (def (main)
              (handle Count 0 ((tick (u) c (resume c (+ c 1))))
                (handle A 5 ((a (u) s (do (Count.tick) (resume s s)))) (A.a)))) (export main)))
  (output (: 5 Int64)))

(case "a handler arm forwarding an effect its enclosing scope does not hold is rejected"
  (doc    "Witnesses capabilities-and-effects.md #Capabilities Attenuate: A Handler Forwards A Narrower Row
           (2nd sentence — attenuation never WIDENS): a handler MUST NOT grant its sub-computation an effect
           row label it does not itself hold. `A`'s arm forwards `B` (performs `(B.b)` as its resume value),
           but `B` is neither handled by an enclosing handler nor delegated at the entrypoint anywhere in
           `main`'s scope — so the arm reaches an effect its enclosing scope does not hold. Rejected at
           compile time (CDZ0401, the no-home check): an arm cannot forward a capability the enclosing row
           does not carry, keeping 'no ambient authority' transitive across the handle. The over-broad
           forward is a compile-time rejection, not a runtime failure.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main)
              (handle A 0 ((a (u) s (resume (B.b) s))) (A.a))) (export main)))
  (error  CDZ0401))

(case "a handler arm forwards an effect its enclosing scope DOES hold and runs"
  (doc    "The positive companion (attenuation NARROWS within what is held — 1st sentence): the SAME arm that
           forwards `B` is accepted once an enclosing handler HOLDS `B`. `main` wraps the `A`-handler in a
           `B`-handler, so `A`'s arm forwarding `(B.b)` reaches a held effect: `B` seeded 100 resumes `s`
           (=100), so `(B.b)` is 100, `A`'s arm resumes 100, and `(A.a)` = 100. Pins that the forward is
           legal exactly when the enclosing scope carries the label — the row a handler forwards is a SUBSET
           of the row it holds, checked statically (the reject above is the same check failing).")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main)
              (handle B 100 ((b (u) s (resume s s)))
                (handle A 0 ((a (u) s (resume (B.b) s))) (A.a)))) (export main)))
  (output (: 100 Int64)))

(case "an arm resuming with a re-perform of its OWN effect forwards to an outer handler of that effect"
  (doc    "The SAME-effect forwarding case: an arm resuming with a fresh perform of the effect IT discharges
           re-performs OUTWARD — a handler arm's own-effect perform forwards to the next-OUTER handler of that
           effect, not back into itself (`check_no_home` walks arm bodies under the OUTER handled set). Inner
           `Inner`'s arm resumes with `(Outer.i-style)`… here spelled with two effects to show the forward
           reaches an ENCLOSING handler: `Inner`'s arm resumes `(Outer.o)`, and `Outer` is handled outside —
           `Outer` seeded 50 resumes its state, so `(Outer.o)` = 50, `Inner.i` resumes 50, `(+ 1 (Inner.i))` =
           51. Pins that a resume value performing an effect handled FURTHER OUT folds (the forward reaches an
           enclosing home) — the mechanism the interpose cases rely on, isolated to the resume-value position.")
  (input  (do
            (effect Outer (op o (-> Unit Int64)))
            (effect Inner (op i (-> Unit Int64)))
            (def (main)
              (handle Outer 50 ((o (u) t (resume t t)))
                (handle Inner 0 ((i (u) s (resume (Outer.o) s)))
                  (+ 1 (Inner.i))))) (export main)))
  (output (: 51 Int64)))

(case "an arm re-performing its own effect with no outer handler has no home"
  (doc    "The reject companion of the forwarding case above: when an arm resumes with a fresh perform of the
           effect it discharges — `(flip (u) s (resume (Amb.flip) s))` — that own-effect perform re-performs
           OUTWARD (arm bodies resolve under the outer handled set), so it needs an ENCLOSING `Amb` handler.
           Here there is none (this is the only `Amb` handler), so the re-perform has no home: CDZ0401. This
           is NOT a misleading message — under the forwarding model an arm's own-effect perform genuinely
           escapes to an outer handler, and the outermost one has nowhere to forward. (A bare self-resume like
           this would also be a non-terminating re-perform loop were it to fold; the no-home reject is the
           correct diagnosis, the same check that flags forwarding a DIFFERENT unheld effect above.)")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (resume (Amb.flip) s)))
                (+ 1 (Amb.flip)))) (export main)))
  (error  CDZ0401))

(case "an abortive handler abandons a host call in the path it discards"
  (doc    "Witnesses that an abortive perform's abandonment extends to a DELEGATED host call in the
           discarded continuation (capabilities-and-effects.md #A Handler Arm May Abandon The Computation It
           Discharges, composed with #Host Delegation Is An Entrypoint's Prerogative). The body `(+
           (Bail.bail 7) (ask.ask))` evaluates LEFT-TO-RIGHT: the first operand `(Bail.bail 7)` is abortive
           (its arm never resumes), so it abandons the whole `+` — the handle evaluates to 7 and the second
           operand `(ask.ask)` is NEVER reached. Because it is not reached, the host call is NOT issued: the
           observed host-call sequence is EMPTY. A run's host I/O is exactly the calls on the taken path,
           never a call in an abandoned one — so an abort that jumps past a would-be host call elides it.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (host (ask) (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 7) (ask.ask))))) (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls)
  (output (: 7 Int64)))

(case "a delegated host effect composes with the value-heap runtime"
  (doc    "Witnesses that a program may BOTH delegate an effect to the host AND use the value-heap runtime
           in one component (capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses composed with the runtime's collection operations). `ask` is delegated to the host; its
           returned value is used as a KEY inserted into a runtime map. The component imports TWO interfaces —
           the effect (as `host`) and the value-heap runtime (as `heap`) — and the boundary threads both: the
           host response for `ask.ask` and the runtime's `map-insert`/`map-size` ops. With `ask.ask`
           responding 2, inserting key 2 into the map {1: 10} yields two distinct keys, so `Map.len` is 2 —
           a deterministic function of the input, the recorded response, and the runtime's semantics.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (Map.len (Map.insert (map (1 10)) (ask.ask) 20)))) (export main)))
  (host-responses (respond ask.ask (: 2 Int64)))
  (host-calls (call ask.ask))
  (output (: 2 Int64)))

(case "an effect discharged by a handler does not escape to the manifest"
  (doc    "Witnesses capabilities-and-effects.md #An Effect That Does Not Escape Is Discharged By A
           Handler and #An Effect Discharged By An In-Program Handler Does Not Appear In The Manifest:
           the `Choose` effect is declared with a nullary operation `pick`, raised in the body as
           `(Choose.pick)`, and discharged by an enclosing handler that resumes it with 5, so the effect
           never reaches a host function. The handler is stateless (seed `unit`, thread `s` unchanged). The
           program imports no host function, so its manifest is empty (host-calls asserts none), yet it uses
           an effect internally. Operations are qualified by their declaring effect (#An Effect Declaration
           Names The Effect And Types Its Operations).")
  (input  (do
            (effect Choose (op pick (-> Unit Int64)))
            (def (main)
              (handle Choose unit ((pick () s (resume 5 s))) (+ (Choose.pick) 1))) (export main)))
  (output (: 6 Int64))
  (host-calls))

(case "a handler resumes its continuation at most once by default"
  (doc    "Witnesses capabilities-and-effects.md #A Continuation Is One-Shot By Default: the handler
           resumes the continuation exactly once, so the affine discipline holds and the result is a
           single value (the resumed computation is not duplicated). `Get` is declared with a nullary
           operation `get` returning Int64, performed as `(Get.get)`; the handler is stateless.")
  (input  (do
            (effect Get (op get (-> Unit Int64)))
            (def (main)
              (handle Get unit ((get () s (resume 41 s))) (+ (Get.get) 1))) (export main)))
  (output (: 42 Int64))
  (host-calls))

(case "an abortive handler arm never resumes, so its value becomes the handle's value"
  (doc    "Witnesses capabilities-and-effects.md #A Handler Arm May Abandon The Computation It Discharges:
           `Bail` declares `bail : Int64 -> Int64`, and the handler's arm `(Bail.bail (n) s n)` NEVER
           resumes — it yields `n` as the arm body's value and discards the continuation. So performing
           `(Bail.bail 7)` inside `(+ 1 (Bail.bail 7))` ABANDONS the surrounding `+ 1` (control never
           returns to it) and the handle evaluates to the arm value 7, NOT 8. This is the abortive class
           — a typed early-exit / 'bail and catch at the top' — realized as a control block the perform
           `br`s out of, carrying the arm value (`DESIGN-effects-rcdzc.md` §4.2). Contrast the tail-
           resumptive `Get` above (resumes, so `+ 1` runs): the arm's resume DISCIPLINE, not the operator,
           decides whether the surrounding computation survives.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail 7)))) (export main)))
  (output (: 7 Int64)))

; The abortive case above performs with a CONSTANT argument `(Bail.bail 7)`, which folds. The runtime
; companion: the abort argument is a boundary parameter `k`. The abortive arm's value is the handle's
; value = k, and the surrounding `+ 1` is abandoned, decided at run time. This pins the abort control
; block carrying a RUNTIME arm value out of the perform (breaker: fixed by v-effects `bd6ff9bd2`
; "reparent an abortive arm's value → grounds a runtime-arg abort" — the wasm lower previously
; re-derived the handle result as Any for a non-const abort arg, declining "no machine representation";
; the reparent grounds it, and wasm now matches rust).

(case "an abortive handler arm with a runtime perform argument yields that runtime value as the handle's value"
  (doc    "The runtime-argument companion of the abortive-arm case above (which uses a CONST `(Bail.bail
           7)`). Here the bail argument is the boundary parameter `k`: the arm `(bail (n) s n)` never
           resumes, so it abandons the surrounding `+ 1` and the handle evaluates to the arm value n = k.
           run(7) = 7, run(42) = 42 — the abort carries a RUNTIME value out of the perform via the control
           block, not only a constant. Pins that the abortive early-exit grounds its arm value when the
           perform argument is decided at run time.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: k Int64)) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail k))))
            (export run)))
  (call run (: 7 Int64)) (output (: 7 Int64))
  (call run (: 42 Int64)) (output (: 42 Int64)))

(case "an abortive perform deep in a call chain unwinds every intervening frame to the top handler"
  (doc    "The 'bail and catch at the top' pattern across FUNCTIONS (DESIGN-effects-rcdzc.md §4.2 cross-
           function non-local exit): the abort is performed three calls deep and abandons EVERY pending
           frame between it and the handler. `main` handles `Bail` and calls `(a 5)`; `a n = (+ 1 (b n))`,
           `b n = (+ 1 (c n))`, `c n = (+ n (Bail.bail 99))`. Performing `(Bail.bail 99)` at the base
           abandons `c`'s `(+ n …)`, `b`'s `(+ 1 …)`, and `a`'s `(+ 1 …)` — none of the pending additions
           runs — so the handle evaluates to the arm value 99, NOT 5+99+1+1. Witnesses that abortion is a
           non-local exit over the whole call chain, not a per-frame return that the intervening arithmetic
           could observe. (The callees are non-recursive, so the inline trigger makes the abort unconditional
           in the inlined body.)")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (c (: n Int64)) (+ n (Bail.bail 99)))
            (def (b (: n Int64)) (+ 1 (c n)))
            (def (a (: n Int64)) (+ 1 (b n)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (a 5))) (export main)))
  (output (: 99 Int64)))

(case "an abortive perform under THREE nested handlers abandons the two resumptive frames above it"
  (doc    "The abortive class composed with DEEP nesting: an abort fires inside a body that also performs two
           OTHER effects (`A`, `B`) discharged by enclosing resumptive handlers. `(+ (A.a) (+ (B.b)
           (Bail.bail 99)))` under `handle A … (handle B … (handle Bail …))`: `A.a` resumes (=1), `B.b`
           resumes (=2), then `Bail.bail 99` — a NON-resuming arm — ABANDONS the pending `(+ (A.a) (+ (B.b)
           …)))` frames and yields the arm value 99 as the whole handle's value (NOT 1+2+99). Pins that a
           non-local exit unwinds past the resumptive frames of OTHER, differently-effect handlers stacked
           above it — the abort is the value of the outermost handle, and the intervening resumptive
           computations (already run for their effect) do not observe it.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 1 ((a (u) s (resume s s)))
                (handle B 2 ((b (u) s (resume s s)))
                  (handle Bail 0 ((bail (n) s n))
                    (+ (A.a) (+ (B.b) (Bail.bail 99))))))) (export main)))
  (output (: 99 Int64)))

(case "when two abortive performs sit on one spine the FIRST (leftmost) abort wins"
  (doc    "Refines the abortive class for MULTIPLE performs. Operands evaluate LEFT-TO-RIGHT, and an
           abortive perform ABANDONS the rest of the computation, so on `(+ (Bail.bail 7) (Bail.bail 9))` the
           FIRST operand `(Bail.bail 7)` fires first and abandons everything — the handle evaluates to 7, and
           the second `(Bail.bail 9)` never runs. The result is the leftmost abort's value, never the second,
           mirroring the left-to-right evaluation order the strict operator imposes.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 7) (Bail.bail 9)))) (export main)))
  (output (: 7 Int64)))

(case "an abortive perform in the tail of an if branch abandons only that branch"
  (doc    "Refines the abortive class for a CONDITIONAL early-exit. `Bail.bail` is abortive (its arm never
           resumes). The handle body is `(if true (Bail.bail 7) 99)` — the `if` IS the handle's value, so an
           abort in a branch's TAIL is LOCAL to that branch: the true branch aborts, yielding the arm value
           7; the false branch, had it run, would yield 99 (its sibling survives — the abort does not
           collapse the whole handle). This is the 'bail on one path, fall through on the other' shape a
           validation routine takes. Contrast a NON-tail conditional abort (`(+ 1 (if c (Bail.bail 7) 0))`),
           where the abort must escape the enclosing `+` — that needs a control block the perform `br`s out
           of and is not yet reducible. Here the branch tail is the handle value, so the fold is per-branch:
           `(if true 7 99)` → 7 (`DESIGN-effects-rcdzc.md` §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (if true (Bail.bail 7) 99))) (export main)))
  (output (: 7 Int64)))

(case "an abortive perform in the NON-taken if branch is never evaluated (no speculation)"
  (doc    "The soundness complement of the taken-branch abort above, and a pin against SPECULATIVE branch
           evaluation (e.g. a branchless-`select` lowering that would eagerly compute both arms): the abort
           sits in the branch that is NOT taken and MUST NOT fire. `Bail.bail` is abortive (its arm `(bail
           (n) s n)` never resumes, yielding `n` as the handle value). The body `(if (< 3 5) 10 (Bail.bail
           99))` takes the true branch (`3 < 5`), so the handle evaluates to `10`; the else-branch's abort is
           dead code that never runs. Were the compiler to evaluate both branches (speculating the abort),
           the handle would wrongly collapse to `99`. Pins that an abortive perform in a non-taken branch is
           genuinely conditional — only the taken path's effects occur — which the branch-local fold and any
           branchless-select conversion must both preserve. The control (flip to `(> 3 5)`, abort taken)
           yields 99.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (if (< 3 5) 10 (Bail.bail 99)))) (export main)))
  (output (: 10 Int64)))

(case "an abortive perform in the tail of an if branch inside a let body abandons only that branch"
  (doc    "The branch-tail abort composes through a `let`: a `let`'s VALUE is its BODY's value, so a `let`
           body is in the same tail position as the `let` itself. `(let ((k 5)) (if true (Bail.bail 7) k))`
           — the `if` is the let body's tail, which is the handle's value — so the abort in the true branch
           is LOCAL to that branch (yields the arm value 7); the false branch, had it run, would yield the
           bound `k` = 5 (the sibling survives). Pins that the abortive fold's tail-position reasoning
           descends into a `let` body, not just a bare `if` (`DESIGN-effects-rcdzc.md` §4.2). Contrast an
           abort in a NON-tail `let` INIT (`(let ((k (if c (Bail.bail 7) 0))) …)`), which must escape into
           `k` and is not yet reducible.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (let ((k 5)) (if true (Bail.bail 7) k)))) (export main)))
  (output (: 7 Int64)))

(case "a handle body reads an enclosing function parameter"
  (doc    "The handle body is not closed — it may reference a binding from the enclosing scope, exactly as
           any other expression does. `main`'s parameter `x` is read directly in the handle body `(+ x
           (Get.get 0))`: the `Get` handler resumes 5, so the body is `x + 5`. Called with `x = 10` the
           result is 15. Pins that the tail-resumptive fold's rewritten body still resolves a FREE variable
           up the original lexical chain — the fold synthesizes a fresh body subtree, which must remain
           anchored where the `handle` sat so `x` reaches `main`'s parameter binder (not a spurious unbound
           name). Runtime parameters are what make an effectful body more than a constant.")
  (input  (do
            (effect Get (op get (-> Int64 Int64)))
            (def (main (: x Int64))
              (handle Get 0 ((get (n) s (resume 5 s))) (+ x (Get.get 0)))) (export main)))
  (call   main (: 10 Int64))
  (output (: 15 Int64)))

(case "a runtime condition selects an abortive branch reading an enclosing parameter"
  (doc    "The branch-tail abort with a RUNTIME condition over an enclosing parameter — the shape a
           validation routine takes: `(handle Bail 0 ((bail (n) s n)) (if (< x 5) (Bail.bail 7) x))`. The
           `if` is the handle's value, so an abort in a branch tail is local to that branch (yields the arm
           value); the other branch reads the parameter `x` and falls through. Called with `x = 9` (not <
           5), the false branch yields `x` = 9 — no abort. This composes the branch-tail abortive fold with
           a free parameter reference and a runtime condition (`DESIGN-effects-rcdzc.md` §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: x Int64))
              (handle Bail 0 ((bail (n) s n)) (if (< x 5) (Bail.bail 7) x))) (export main)))
  (call   main (: 9 Int64))
  (output (: 9 Int64)))

(case "an abortive perform under a non-tail conditional abandons the enclosing computation"
  (doc    "The abortive early-exit from MID-EXPRESSION, not just a tail branch. `(+ 100 (if (< x 5)
           (Bail.bail 7) 50))` — the abort is a strict OPERAND of `+`, not the handle's tail. Because an
           abort ABANDONS the enclosing computation, the surrounding `+ 100` is dead on the aborting path,
           so the expression is equivalent to `(if (< x 5) (Bail.bail 7) (+ 100 50))`: distributing the
           pure enclosing op into both branches lifts the abort to a branch tail (value-preserving because
           the sibling operand `100` is pure). Called with `x = 3` (< 5) the abort fires, discarding the
           `+ 100` → 7; with `x = 9` the false branch runs → `100 + 50` = 150. This is the 'validate an
           argument, bail out of the whole computation on failure' shape (`DESIGN-effects-rcdzc.md` §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: x Int64))
              (handle Bail 0 ((bail (n) s n)) (+ 100 (if (< x 5) (Bail.bail 7) 50)))) (export main)))
  (call   main (: 3 Int64))
  (output (: 7 Int64)))

(case "an abortive perform in an if condition abandons the computation before branching"
  (doc    "The abort sits in the `if` CONDITION — `(if (< (Bail.bail 7) 5) 1 2)` — which is evaluated
           FIRST, before either branch is chosen. Because an abort ABANDONS the enclosing computation, the
           `if` never branches: the whole handle yields the arm value 7, regardless of which branch the
           condition would have selected. Contrast an abort in a branch TAIL (local to that branch): a
           condition abort is unconditional (the condition always runs). Both the abort arm value and the
           `if` result type are Int64 — the handle body types compatibly. Pins that the abortive fold's
           type-consistency check compares by COMPATIBILITY (an undetermined `Int` agrees with `Int64`), not
           structural equality (`DESIGN-effects-rcdzc.md` §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (if (< (Bail.bail 7) 5) 1 2))) (export main)))
  (output (: 7 Int64)))

(case "an abortive perform in a short-circuit connective's right operand abandons the computation"
  (doc    "A short-circuit connective is a conditional in disguise — `(and lhs rhs)` evaluates `rhs` only
           when `lhs` is true — so an abort in the right operand is a conditional abort, equivalent to
           `(if lhs rhs false)`. `(and (< x 5) (Bail.bail 7))`: when `x < 5` the right operand runs and the
           abort fires, abandoning the computation and yielding the arm value; when `x >= 5` the connective
           short-circuits to false without performing. Here `Bail.bail : Int64 -> Bool` and the arm yields a
           Bool (`(< n 100)`), so the abort value is Bool — consistent with the connective's Bool result.
           Called with `x = 3` (< 5) the abort fires → `(< 7 100)` = true. Witnesses that the abortive fold
           reaches a short-circuit operand by desugaring it to the `if` form (`DESIGN-effects-rcdzc.md`
           §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Bool)))
            (def (main (: x Int64))
              (handle Bail false ((bail (n) s (< n 100))) (and (< x 5) (Bail.bail 7)))) (export main)))
  (call   main (: 3 Int64))
  (output (: true Bool)))

(case "a perform SHORT-CIRCUITED out of an or's right operand is NOT executed (empty host-calls)"
  (doc    "The soundness half of the short-circuit connective: when the LEFT operand short-circuits, the
           RIGHT operand's perform MUST NOT run — short-circuit evaluation elides it. `(or (> 5 3)
           (> (Amb.flip) 0))` — the left `(> 5 3)` is true, so `or` short-circuits to true and the right
           operand `(> (Amb.flip) 0)` never evaluates, so `Amb.flip` is never performed. With `Amb`
           HOST-DELEGATED, that elision is OBSERVABLE: the run makes NO host call. The empty `(host-calls)`
           fixture pins it — a perform in a skipped operand produces no observable effect. (Contrast the
           existing right-operand-RUNS case where the left selects the right; here the left short-circuits
           past it.)")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (host (Amb) (or (> 5 3) (> (Amb.flip) 0)))) (export main)))
  (output (: true Bool))
  (host-calls))

(case "an abortive perform in a conditional let binding abandons the computation"
  (doc    "The abortive early-exit from a `let` INITIALIZER — the 'bind the validated value or bail' shape.
           `(let ((k (if (< x 5) (Bail.bail 7) 0))) (+ 1 k))`: the binding's init aborts when `x < 5`. An
           init is a non-tail position (its value feeds `k`), but an abort ABANDONS the computation, so the
           `if` lifts out of the `let` — `(if (< x 5) (Bail.bail 7) (let ((k 0)) (+ 1 k)))` — value-
           preserving because the condition (and any earlier binding) is pure. Called with `x = 9` (not <
           5), the false branch binds `k = 0` and returns `1 + 0` = 1; with `x = 3` the abort fires,
           discarding the binding and the body, yielding the arm value (`DESIGN-effects-rcdzc.md` §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: x Int64))
              (handle Bail 0 ((bail (n) s n)) (let ((k (if (< x 5) (Bail.bail 7) 0))) (+ 1 k)))) (export main)))
  (call   main (: 9 Int64))
  (output (: 1 Int64)))

(case "a handler arm that resumes NON-tail folds when the perform is the whole body"
  (doc    "The GENERAL one-shot arm — a `resume` NOT in tail position, so the arm does work AFTER resuming
           (`(Amb.flip (u) s (+ 1 (resume 10 s)))` adds 1 to whatever the continuation returns). This is
           the powerful case (capabilities-and-effects.md #A Handler May Resume Anywhere). Its full form
           needs a reified continuation, but when the performed operation is the WHOLE handle body its
           continuation is the IDENTITY (nothing runs after the perform), so `(resume 10 s)` yields 10 in
           place and the arm evaluates to `(+ 1 10)` = 11 — no continuation object needed. Witnesses the
           identity-continuation sliver of the general-resume class; a non-tail resume whose perform sits
           inside a larger expression (a non-identity continuation) still awaits the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (Amb.flip))) (export main)))
  (output (: 11 Int64)))

(case "a handler arm consumes its resume value through an effect-free helper call"
  (doc    "The arm's work AFTER resuming may be a call to a NON-RECURSIVE, effect-free USER function, not
           only a primitive: `(dbl (resume 10 s))` where `dbl x = x*2`. The perform's continuation is the
           handle body `C = (+ 1 [])`, so `(resume 10 s)` yields `C[10]` = 11, and the arm evaluates to
           `(dbl 11)` = 22. The helper is applied to the continuation RESULT in the arm body (distinct from
           an effect-free call INSIDE the continuation `C`); it runs once per resume, effect-free, so no
           reified continuation is needed.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (dbl (: n Int64)) (* n 2))
            (def (main)
              (handle Amb 0 ((flip (u) s (dbl (resume 10 s)))) (+ 1 (Amb.flip)))) (export main)))
  (output (: 22 Int64)))

(case "a handler arm that resumes NON-tail folds through a PURE one-hole continuation"
  (doc    "The general one-shot arm generalizes past the identity-continuation sliver: when the performed
           operation sits inside a larger PURE expression its delimited continuation is a pure one-hole
           context `C = body[perform := []]` (capabilities-and-effects.md #A Handler May Resume Anywhere).
           Here the body is `(+ 100 (Amb.flip))`, so `C = (+ 100 [])` — effect-free — and `(resume 10 s)`
           returns into it, yielding `C[10] = (+ 100 10)`. The arm `(+ 1 (resume 10 s))` then evaluates to
           `(+ 1 (+ 100 10))` = 111. No reified continuation object is needed while `C` is pure (it may even
           be duplicated by a multi-shot resume with no effect change); a perform in a conditional BRANCH — a
           non-uniform continuation — still awaits the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 100 (Amb.flip)))) (export main)))
  (output (: 111 Int64)))

(case "a one-shot handler arm folds a body with TWO performs by re-reducing the continuation"
  (doc    "The general one-shot arm extends past a single hole to a body with SEVERAL discharged performs,
           when the arm resumes EXACTLY ONCE. In a DEEP handler `resume v s'` returns into the continuation
           `C[v]` with the handler STILL ACTIVE, so a further perform in `C[v]` is handled too — the resume
           re-reduces the continuation: `resume v s' = handle(s', arms, C[v])`. Here the body is
           `(+ (Amb.flip) (Amb.flip))`: the leading flip has continuation `C = (+ [] (Amb.flip))`;
           `(resume 10 s)` re-reduces `C[10] = (+ 10 (Amb.flip))`, itself a pure one-hole context that folds
           to `(+ 1 (+ 10 10))` = 21; the outer arm `(+ 1 (resume 10 s))` then evaluates to `(+ 1 21)` = 22.
           Each re-reduction removes one perform, so it terminates. Because the arm resumes ONCE, the
           continuation is spliced once — no effect is duplicated, so no reified continuation is needed. A
           MULTI-shot arm over a performing continuation still awaits the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ (Amb.flip) (Amb.flip)))) (export main)))
  (output (: 22 Int64)))

(case "a MULTI-shot handler arm folds a two-hole body by re-reducing per resume"
  (doc    "The re-reducing fold extends to a MULTI-shot arm — one that resumes more than once — when the
           body's performs are all discharged BY THIS handler (no effect escapes to be re-issued). The fold
           rewrites EACH `resume` occurrence to its own re-reduction of the continuation, which is exactly the
           deep-handler multi-shot semantics: every resumption independently continues, re-handling the
           discharged effect in the continuation. Here the arm `(+ (resume 1 s) (resume 2 s))` resumes twice
           and the body `(+ (Amb.flip) (Amb.flip))` performs twice: the leading flip's continuation `C = (+
           [] (Amb.flip))` is re-reduced at 1 and at 2 — `C[1]` folds the second flip to `(+ (+ 1 1) (+ 1 2))`
           = 5, `C[2]` to `(+ (+ 2 1) (+ 2 2))` = 7 — so the arm yields `(+ 5 7)` = 12. Re-running a
           DISCHARGED in-program effect per resumption is sound (it is folded away, leaving pure code); a
           continuation that reached a HOST-delegated or outer-handler effect would violate the
           host-composition invariant and stays a clean decline.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Amb.flip) (Amb.flip)))) (export main)))
  (output (: 12 Int64)))

(case "a MULTI-shot arm whose FIRST resume value is chosen by an if on the state"
  (doc    "Composes multi-shot resumption with a conditional-resume value: the arm resumes TWICE, and the
           FIRST resume's value is chosen by an `if` on the handler state. `(flip (u) s (+ (resume (if (> s
           2) 10 20) s) (resume 1 s)))` over the body `(+ 100 (Amb.flip))` — the pure one-hole continuation
           `C = (+ 100 [])` is spliced per resume: seeded 3, `(> 3 2)` holds so the first resume value is 10
           → `C[10]` = 110, and the second is 1 → `C[1]` = 101, so the arm yields `(+ 110 101)` = 211. Pins
           that a multi-shot arm's per-resume continuation splice composes with a resume value COMPUTED by a
           branch on the state — each resumption independently folds `C` at its own (state-derived) value.
           Both backends agree.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 3 ((flip (u) s (+ (resume (if (> s 2) 10 20) s) (resume 1 s))))
                (+ 100 (Amb.flip)))) (export main)))
  (output (: 211 Int64)))

(case "a MULTI-shot arm folds a perform wrapped in an inline lambda application"
  (doc    "A perform WRAPPED IN A LAMBDA APPLICATION folds under a multi-shot arm. `((fn (x) (+ x (Amb.flip)))
           100)` is a β-redex: applying the lambda substitutes `x := 100`, giving `(+ 100 (Amb.flip))` — a
           single perform in a pure one-hole context `C = (+ 100 [])`. The fold PRE-REDUCES applied-lambda
           redexes before classifying (`reduce_applied_lambdas`), so the multi-shot path serves it exactly as
           the reduced body: the arm `(+ (resume 1 s) (resume 2 s))` yields `(+ (+ 100 1) (+ 100 2))` = 203.
           (The one-shot/threading path already inlines such a call via its cross-function inline arm; this
           extends the same β-reduction to the multi-shot pure-one-hole path.) A lambda VALUE is pure at
           construction — its body's effects fire only when APPLIED — so duplicating the reduced context per
           resumption duplicates no closure effect.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s))))
                ((fn (x) (+ x (Amb.flip))) 100))) (export main)))
  (output (: 203 Int64)))

(case "a MULTI-shot arm folds a perform in a let-bound lambda applied in the body"
  (doc    "The let-bound form of the preceding case, composing the applied-lambda pre-reduction with the
           lambda-value-is-pure purity rule. `(let ((f (fn (x) (+ x (Amb.flip))))) (f 100))` binds a
           performing lambda (pure at construction) and applies it; pre-reduction β-reduces `(f 100)` to
           `(+ 100 (Amb.flip))`, leaving the now-unused binding whose lambda init is strongly pure (the
           purity walk does not descend a lambda body). `C = (+ 100 [])` under the multi-shot arm yields
           `(+ (+ 100 1) (+ 100 2))` = 203. Pins that a let-bound performing lambda folds under a multi-shot
           resume, not only a one-shot one.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s))))
                (let ((f (fn (x) (+ x (Amb.flip))))) (f 100)))) (export main)))
  (output (: 203 Int64)))

(case "a MULTI-shot arm keeps a pure applied lambda in its duplicated continuation"
  (doc    "The soundness anchor for the lambda-value purity rule under a MULTI-shot resume: an EFFECT-FREE
           let-bound lambda `k = (fn (y) (* y 2))` is APPLIED in the continuation `C` alongside the single
           perform. `C = (+ (k 3) [])` is strongly pure — `(k 3)` re-runs an effect-free computation, and the
           lambda value itself carries no effect — so duplicating `C` per resumption is safe: `(k 3)` = 6, and
           the arm yields `(+ (+ 6 1) (+ 6 2))` = 15. Confirms the purity walk skipping a lambda body does NOT
           over-admit — a performing applied lambda (a genuine second hole) still declines as non-uniform,
           while a pure applied lambda folds.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s))))
                (let ((k (fn (y) (* y 2)))) (+ (k 3) (Amb.flip))))) (export main)))
  (output (: 15 Int64)))

; The multi-shot cases above duplicate a continuation containing only SCALARS or a pure lambda. These pin the
; Perceus × multi-shot intersection: a continuation that reads or CONSUMES a captured HEAP value, re-reduced
; per resumption, must give EACH resume its own valid copy — the multi-shot duplication must `dup` the
; captured heap value, not share one that the first resume frees (or FBIP-mutates in place at rc==1) out from
; under the second. A shared-and-freed heap value would use-after-free / corrupt the second resumption.

(case "a MULTI-shot arm re-reduces a continuation that reads a captured heap list per resume"
  (doc    "The arm `(+ (resume 1 s) (resume 2 s))` resumes TWICE; the continuation `(+ (Amb.flip) (List.len
           xs))` reads a captured heap list `xs = [10 20 30]`. Each re-reduction must see `xs` alive:
           resume-1 → (1 + 3), resume-2 → (2 + 3), so `(+ 4 5)` = 9. Pins the captured heap value is retained
           (dup'd) across BOTH continuation re-reductions — a value freed after the first resume would make
           `List.len xs` in the second read freed memory (a wrong length / crash).")
  (input  (do (effect Amb (op flip (-> Unit Int64)))
              (def (main)
                (let ((xs (list 10 20 30)))
                  (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Amb.flip) (List.len xs)))))
              (export main)))
  (output (: 9 Int64)))

(case "a MULTI-shot arm whose each resume CONSUMES a captured heap list dups it per resume"
  (doc    "The sharper case: each resumption CONSUMES the captured `xs = [1 2]` via `List.push` (a persistent
           op that FBIP-mutates in place at rc==1). Under multi-shot, both re-reductions consume `xs`, so the
           duplication MUST dup it — else the first resume's `List.push` grows the shared `xs` in place and
           the second resume sees `[1 2 99]` (len 4, wrong). `List.len (List.push xs 99)` = 3 each resume:
           resume-1 → (1 + 3), resume-2 → (2 + 3) → `(+ 4 5)` = 9. Pins the multi-shot duplication dups a
           CONSUMED captured heap value, the Perceus-correct multi-shot semantics.")
  (input  (do (effect Amb (op flip (-> Unit Int64)))
              (def (main)
                (let ((xs (list 1 2)))
                  (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Amb.flip) (List.len (List.push xs 99))))))
              (export main)))
  (output (: 9 Int64)))

(case "a MULTI-shot arm folds a perform under a CURRIED lambda applied to pure arguments"
  (doc    "The applied-lambda pre-reduction reduces a CURRIED redex — nested applications — as long as each
           argument is pure. `(((fn (a) (fn (b) (+ a (+ b (Amb.flip))))) 10) 20)` applies the outer lambda to
           `10` (yielding the inner `(fn (b) …)`) then to `20`, β-reducing to `(+ 10 (+ 20 (Amb.flip)))` =
           `(+ 30 (Amb.flip))` — a single perform in a pure one-hole context `C = (+ 30 [])`. Both arguments
           are pure literals, so the substitution (into params each used once) duplicates no effect, and the
           reduced body folds under the multi-shot arm: `(+ (+ 30 1) (+ 30 2))` = 63. Pins that pre-reduction
           follows a curried application chain, not only a single β-redex.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s))))
                (((fn (a) (fn (b) (+ a (+ b (Amb.flip))))) 10) 20))) (export main)))
  (output (: 63 Int64)))

(case "a pure lambda passed as an argument to a performing callee folds"
  (doc    "A HIGHER-ORDER call whose function ARGUMENT is a pure lambda and whose CALLEE performs. `apply1 g n
           = (+ (g n) (Amb.flip))` takes a function `g` and performs; called with `g = (fn (z) (* z 2))` (an
           effect-free lambda) and `n = 10`. The argument lambda is strongly pure (a lambda VALUE carries no
           effect), so the pre-reduction inlines the call — `(g 10)` reduces to `(* 10 2)` = 20, leaving
           `(+ 20 (Amb.flip))`, a single perform in a pure one-hole context. The handler resumes 5, so the
           result is `(+ 20 5)` = 25. Pins that a pure function-valued argument does not block the fold — the
           closure is passed and applied inside the reduced body with no effect duplication.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (apply1 (: g (-> Int64 Int64)) (: n Int64)) (+ (g n) (Amb.flip)))
            (def (main)
              (handle Amb 0 ((flip (u) s (resume 5 s)))
                (apply1 (fn (z) (* z 2)) 10))) (export main)))
  (output (: 25 Int64)))

(case "an arm that binds its resume in a lambda and applies it immediately folds"
  (doc    "An arm that names its continuation as a LAMBDA and APPLIES it in place — `(flip (u) s (let ((k (fn
           (x) (resume (* x 2) s)))) (k 5)))`. This LOOKS like the captured-continuation frontier (a `k`
           bound to the resume), but `k` does NOT escape — it is applied immediately, `(k 5)`. So the
           applied-lambda pre-reduction inlines it to `(resume (* 5 2) s)` = `(resume 10 s)`, an ORDINARY
           non-tail resume the pure one-hole fold serves: `C = (+ 100 [])` over the body `(+ 100 (Amb.flip))`,
           so the handle yields `(+ 100 10)` = 110. Pins that binding the resume in a lambda and applying it
           in-arm is NOT the hard captured-`k` case (which needs a reified continuation) — an immediately-
           applied continuation-lambda reduces away, distinguishing 'names k' from 'k escapes'.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (let ((k (fn (x) (resume (* x 2) s)))) (k 5))))
                (+ 100 (Amb.flip)))) (export main)))
  (output (: 110 Int64)))

(case "an applied lambda whose body enters a mutually-recursive performing group folds"
  (doc    "Composes the applied-lambda pre-reduction with mutual-recursion specialization: the handle body
           is `((fn (m) (ev m)) 4)`, a lambda applied to a pure literal whose body ENTERS the
           mutually-recursive performing group `ev`/`od`. Pre-reduction inlines the pure-arg redex to
           `(ev 4)`, then the mutual pair specializes under the state handler exactly as a direct `(ev 4)`
           would — the two folds compose. Seeded 7, threading `s - 1`, the ticks read 7 then 6, so `ev(4)` =
           `7 + 6 + 0` = 13. Pins that an applied lambda is a transparent wrapper over a recursive-effectful
           call, folding to the same result as the unwrapped call.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (ev (: n Int64)) (if (= n 0) 0 (+ (Ctr.tick) (od (- n 1)))))
            (def (od (: n Int64)) (ev (- n 1)))
            (def (main)
              (handle Ctr 7 ((tick (u) s (resume s (- s 1))))
                ((fn (m) (ev m)) 4))) (export main)))
  (output (: 13 Int64)))

(case "an applied lambda whose body performs an ABORTIVE op abandons the enclosing computation"
  (doc    "Composes the applied-lambda pre-reduction with an ABORTIVE (non-resuming) handler. The body
           `(+ 100 ((fn (x) (Bail.bail x)) 42))` wraps the abortive perform in a lambda application in a
           STRICT operand position. Pre-reduction inlines the pure-arg redex to `(+ 100 (Bail.bail 42))`,
           where the abort abandons the surrounding `(+ 100 …)` — the abortive arm's value 42 becomes the
           whole handle's value (NOT 142). Pins that an abort reached through an applied-lambda wrapper still
           unwinds the enclosing strict context, the abortive analogue of the resumptive compositions above.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (handle Bail 0 ((bail (n) s n))
                (+ 100 ((fn (x) (Bail.bail x)) 42)))) (export main)))
  (output (: 42 Int64)))

(case "a performing argument to a multiply-using performing callee is not duplicated"
  (doc    "The SOUNDNESS ANCHOR for the applied-lambda pre-reduction: a call is β-reduced early (before the
           pure-one-hole classifier) ONLY when its arguments are strongly PURE. Here the argument itself
           PERFORMS — `(mixed (Amb.flip))` where `mixed x = (+ x (+ x (Amb.flip)))` uses its parameter `x`
           TWICE. β-substituting the performing argument textually would run `(Amb.flip)` once PER use of
           `x` — three performs instead of two — a miscompile. Cadenza is strict (call-by-value): the
           argument evaluates EXACTLY ONCE to a value the two uses of `x` share. The pre-reduction declines
           this redex (its argument is not strongly pure) and the state-threading path binds the argument's
           single resume value once. Handler seed 0, `flip` resumes `s+1` threading `s+1`: the argument flip
           reads 0→1 (so `x` = 1, state→1), the body flip reads 1→2, giving `(+ 1 (+ 1 2))` = 4. Pins that a
           performing argument is evaluated once, never duplicated by early β-reduction.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (mixed (: x Int64)) (+ x (+ x (Amb.flip))))
            (def (main)
              (handle Amb 0 ((flip (u) s (resume (+ s 1) (+ s 1))))
                (mixed (Amb.flip)))) (export main)))
  (output (: 4 Int64)))

(case "a one-shot two-hole body folds across a let binding"
  (doc    "The one-shot re-reducing fold descends the STRICT spine of a `let` (its inits then its body, run
           unconditionally in sequence), so a body with a perform in the let INIT and another in the let
           BODY folds. Here `(let ((x (Amb.flip))) (+ x (Amb.flip)))`: the leading flip is the INIT, with
           continuation `C = (let ((x [])) (+ x (Amb.flip)))`; `(resume 10 s)` re-reduces `C[10] = (let ((x
           10)) (+ x (Amb.flip)))` — the binding fixes `x = 10` and the body's remaining flip is a pure
           one-hole context, folding to `(+ 1 (+ 10 10))` = 21; the outer arm `(+ 1 (resume 10 s))` then
           evaluates to `(+ 1 21)` = 22. The whole `let` is copied into `C`, so its binder re-binds
           independently; one resume, so the continuation is spliced once.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (let ((x (Amb.flip))) (+ x (Amb.flip))))) (export main)))
  (output (: 22 Int64)))

(case "a one-shot two-hole body folds with the leading perform in an if condition"
  (doc    "The one-shot re-reducing fold descends an `if` CONDITION — the strict, evaluated-first position —
           for its leading hole, and a further perform in a BRANCH is served when the re-reduced condition
           selects that branch. Here `(if (< (Amb.flip) 50) (+ 1 (Amb.flip)) 0)`: the leading flip is the
           condition, `C = (if (< [] 50) (+ 1 (Amb.flip)) 0)`; `(resume 10 s)` re-reduces `C[10] = (if (< 10
           50) (+ 1 (Amb.flip)) 0)` — the condition is now the constant `(< 10 50)` = true, so the then-branch
           is taken and its remaining flip folds (by handler distribution over the now-constant conditional):
           `(+ 1 (+ 1 10))` = 12; the outer arm `(+ 1 (resume 10 s))` → `(+ 1 12)` = 13. The condition runs
           once (one resume), so no effect is duplicated.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< (Amb.flip) 50) (+ 1 (Amb.flip)) 0))) (export main)))
  (output (: 13 Int64)))

(case "TWO performs in an if condition both fold on the strict-first spine"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (resume 1 s)))
                (if (= (Amb.flip) (Amb.flip)) 100 200))) (export main)))
  (doc    "Both operands of an `if` CONDITION perform — `(if (= (Amb.flip) (Amb.flip)) 100 200)`. The
           condition is a strict, evaluated-first position and `=`'s two operands are strict-first
           sub-positions, so BOTH flips lie on the uniform strict spine and fold: each resumes 1 (a
           tail-resumptive arm, seed 0 read twice — no state advance), so the condition is `(= 1 1)` = true
           and the handle yields the then-branch 100. Extends the single-perform-in-a-condition case to two
           performs in the SAME condition — a compiler pass that reads two fresh values to decide a branch.")
  (output (: 100 Int64)))

(case "a handler arm that resumes NON-tail folds when the perform is in an if condition"
  (doc    "The pure one-hole continuation extends into an `if` CONDITION — a strict, always-evaluated-first
           position, so the continuation `C = (if (< [] 5) 1 2)` is uniform (the branches run only AFTER the
           condition and are pure). `(resume 10 s)` returns into it: `C[10] = (if (< 10 5) 1 2)` = 2, and the
           arm `(+ 1 (resume 10 s))` evaluates to `(+ 1 2)` = 3. Both branches are effect-free, so a
           multi-shot resume could duplicate the whole `if` with no effect change. A perform in a conditional
           BRANCH (a non-uniform continuation) still declines — that needs the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< (Amb.flip) 5) 1 2))) (export main)))
  (output (: 3 Int64)))

(case "a handler arm that resumes NON-tail folds when the perform is in a match scrutinee"
  (doc    "The pure one-hole continuation extends into a `match` SCRUTINEE — a strict, always-evaluated-first
           position (like an `if` condition), so `C = (match [] (0 100) (_ 2))` is uniform (the arms run only
           after the scrutinee and are pure). `(resume 10 s)` → `C[10]` selects the `_` arm → 2, and the arm
           `(+ 1 (resume 10 s))` evaluates to `(+ 1 2)` = 3. Every arm BODY is effect-free; a perform in an
           arm body (a non-uniform continuation) still declines — that needs the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (match (Amb.flip) (0 100) (_ 2)))) (export main)))
  (output (: 3 Int64)))

(case "a handler arm that resumes NON-tail folds when the perform is in an and lhs"
  (doc    "The pure one-hole continuation extends into a short-circuit connective's LHS — a strict,
           always-evaluated-first position. `C = (and (< [] 5) true)`; the arm `(not (resume 10 s))` produces
           a Bool: `(resume 10 s)` → `C[10] = (and (< 10 5) true)` = false, and `(not false)` = true. The rhs
           `true` runs only on the taken path and is pure (copied into `C`); a perform in the RHS — a
           conditionally-run position — still declines.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (not (resume 10 s)))) (and (< (Amb.flip) 5) true))) (export main)))
  (output (: true Bool)))

(case "a handler arm that resumes NON-tail folds when the perform is in a let init"
  (doc    "The pure one-hole continuation extends into a `let` INIT — a `let` runs its inits and its body
           UNCONDITIONALLY, in sequence, so an init is a strict-spine position and the continuation
           `C = (let ((x [])) (+ x x))` is uniform. `(resume 10 s)` returns into it: `C[10] = (let ((x 10))
           (+ x x))` = 20, and the arm `(+ 1 (resume 10 s))` evaluates to `(+ 1 20)` = 21. The whole `let` is
           copied per resume, so the binder re-binds independently in each copy — a multi-shot resume is
           safe. A second perform elsewhere in the `let` (a two-hole context) still declines.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (let ((x (Amb.flip))) (+ x x)))) (export main)))
  (output (: 21 Int64)))

(case "a handler arm that resumes NON-tail folds through a pure continuation containing an effect-free call"
  (doc    "The pure one-hole continuation `C` may contain a NON-RECURSIVE user CALL whose body reaches no
           effect — not only primitive operators. Cadenza is strict, so the call evaluates its argument
           exactly once before running, and an effect-free callee adds no effect of its own: `C = (dbl [])`
           where `dbl x = x*2` is a uniform, effect-free continuation. `(resume 10 s)` returns into it:
           `C[10] = (dbl 10)` = 20, and the arm `(+ 1 (resume 10 s))` evaluates to `(+ 1 20)` = 21. Splicing
           the pure call (once here, or many times for a multi-shot resume) re-runs an effect-free
           computation — observationally identical to running it once — so no reified continuation is
           needed. A call whose body ITSELF performs makes the continuation non-uniform and still declines.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (dbl (: x Int64)) (* x 2))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (dbl (Amb.flip)))) (export main)))
  (output (: 21 Int64)))

(case "an effect result drives the bound of a PURE recursive helper called in the handle body"
  (doc    "The effect-free callee the fold treats as opaque may itself be RECURSIVE, as long as its
           recursion reaches NO effect — the companion of the non-recursive effect-free-call cases above.
           The perform is discharged ONCE in the handle body and its result becomes the ARGUMENT to a pure
           recursive helper whose whole recursion is effect-free: `(sum-to (Cfg.limit))` where `sum-to n =
           (if (= n 0) 0 (+ n (sum-to (- n 1))))`. `Cfg.limit` resumes 4, so `(sum-to 4)` = `4 + 3 + 2 + 1`
           = 10. Pins that the fold discharges the single perform to its resume value and then runs the pure
           recursion as an ordinary effect-free computation on that value — the effect does not enter the
           helper's recursion at all (the helper is a separate, self-contained pure function the perform
           merely feeds). Distinct from the effect-context-SPECIALIZED recursive walks (where the recursion
           ITSELF performs): here the recursion is effect-free and only its INPUT comes from an effect.")
  (input  (do
            (effect Cfg (op limit (-> Unit Int64)))
            (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (- n 1)))))
            (def (main)
              (handle Cfg 0 ((limit (u) s (resume 4 s)))
                (sum-to (Cfg.limit)))) (export main)))
  (output (: 10 Int64)))

(case "a let-bound lambda whose body performs is applied in the handle body"
  (doc    "A LAMBDA VALUE is pure at CONSTRUCTION — its body's effects fire only when it is APPLIED. So a
           `let` binding a performing lambda is a pure binding, and the discharged op surfaces at the
           APPLICATION `(f 10)`, which the fold inlines: `f = (fn (x) (+ x (Ask.get)))` inlines to
           `(+ 10 (Ask.get))`, a single perform in a pure one-hole context `C = (+ 10 [])`. The handler
           resumes 5 (a countdown seed 0 read once), so `(f 10)` = `(+ 10 5)` = 15. Pins that the fold's
           effect-reachability walk does NOT descend into a lambda body when deciding a subterm is pure —
           constructing the closure performs nothing, and its one application is where the op is handled.
           Before the fix, the pure binding was misclassified as effectful and the case declined.")
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Ask 0 ((get (u) s (resume 5 s)))
                (let ((f (fn (x) (+ x (Ask.get))))) (f 10)))) (export main)))
  (output (: 15 Int64)))

(case "a pure let-bound lambda and a performing one are both applied in the handle body"
  (doc    "Composes the preceding case with a SIBLING pure lambda binding — two let-bound lambdas, one
           effect-free (`g x = x*2`) and one performing (`f x = x + (Ask.get)`), both applied in a strict
           sum. Neither binding performs at construction; the pure `g` is spliced verbatim into the
           continuation and the performing `f`'s application `(f 10)` inlines to `(+ 10 (Ask.get))` — the
           single hole. `C = (+ (g 3) (+ 10 []))`; the handler resumes 5, so the body is `(+ 6 (+ 10 5))`
           = 21. Pins that skipping a lambda body in the purity walk still admits a genuinely pure
           sibling-lambda continuation (the fix does not over-admit — a lambda that were APPLIED to a
           performing argument would still surface that perform at the application node).")
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Ask 0 ((get (u) s (resume 5 s)))
                (let ((g (fn (y) (* y 2))) (f (fn (x) (+ x (Ask.get))))) (+ (g 3) (f 10))))) (export main)))
  (output (: 21 Int64)))

(case "a handler arm that resumes NON-tail folds a perform in an if branch by handler distribution"
  (doc    "A perform in an `if` BRANCH (a CONDITIONALLY-run position) folds when the CONDITION is pure, by
           HANDLER DISTRIBUTION — a commuting conversion: `(handle E s arms (if c t e))` is equivalent to
           `(if c (handle E s arms t) (handle E s arms e))`. The condition runs exactly once (it is pure, so
           it advances no handler state), and each branch becomes a smaller handle body the pure one-hole
           fold already serves — only the taken branch runs, seeing the seed state. Here `(if (< 3 5) (+ 1
           (Amb.flip)) 0)` distributes: the true branch `(handle … (+ 1 (Amb.flip)))` has `C = (+ 1 [])`, so
           `(resume 10 s)` = `C[10]` = 11 and the arm `(+ 1 (resume 10 s))` = `(+ 1 11)` = 12; the false
           branch is a pure body. `(< 3 5)` is true → 12. A perform in the CONDITION itself (not a pure
           condition) still declines — distributing it would need the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< 3 5) (+ 1 (Amb.flip)) 0))) (export main)))
  (output (: 12 Int64)))

(case "a handler arm that resumes NON-tail folds a perform in a match arm body by handler distribution"
  (doc    "The commuting conversion of the preceding case, over a `match` with a pure SCRUTINEE:
           `(handle E s arms (match k (p b)…))` is equivalent to `(match k (p (handle E s arms b))…)`. The
           scrutinee runs exactly once (pure, evaluated before any arm, advancing no state), and each arm
           body becomes a smaller handle body the pure one-hole fold serves — only the matched arm runs. A
           pattern binder still scopes its (reduced) arm body. Here `(match 1 (0 5) (_ (+ 1 (Amb.flip))))`
           distributes: scrutinee `1` selects the `_` arm → `(handle … (+ 1 (Amb.flip)))` has `C = (+ 1 [])`,
           so `(resume 10 s)` = 11 and the arm `(+ 1 (resume 10 s))` = 12. A perform in the SCRUTINEE itself
           (not pure) still declines — that needs the frame machinery.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (match 1 (0 5) (_ (+ 1 (Amb.flip)))))) (export main)))
  (output (: 12 Int64)))

(case "a handler arm that resumes NON-tail folds a perform in a short-circuit connective right operand"
  (doc    "A perform in an `and`/`or` RIGHT operand (a conditionally-run position) folds by composition: the
           connective desugars to `if` — `(and l r)` is `(if l r false)`, `(or l r)` is `(if l true r)` —
           and the `if`-branch perform then distributes (the pure-conditioned tail conditional case). The
           short-circuit is preserved because the right operand becomes a conditionally-taken branch: it runs
           only when the left operand selects it. Here `(and (< 3 5) (< (Amb.flip) 5))` with arm `(not (resume
           10 s))`: the left `(< 3 5)` is true, so the right runs — `C = (< [] 5)`, `(resume 10 s)` = `(< 10
           5)` = false, and `(not false)` = true.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main)
              (handle Amb 0 ((flip (u) s (not (resume 10 s)))) (and (< 3 5) (< (Amb.flip) 5)))) (export main)))
  (output (: true Bool)))

; --- A handler folds state across the operations it discharges ----------------------------------
; capabilities-and-effects.md #A Handler Threads State Across The Operations It Discharges. Every handle
; seeds an initial state; each arm binds the current state and resume threads the next state forward; the
; handle evaluates to its body's value. These cases witness the fold with a genuine (non-unit) accumulator —
; a scalar counter and a growing list — and show that reading the accumulated state out is an ordinary
; operation, not a separate result form. This is the state model a self-hosting compiler is authored
; against: a fresh-name supply (a counter) and diagnostic accumulation (a list).

(case "a handler folds a counter across the operations it discharges"
  (doc    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges: `Fresh` declares `next : Unit -> Int64` — a fresh-name supply, ONE intention
           'read the current value and advance'. The handler is seeded with 0 at the handle site (the
           initial state is explicit, not ambient), and its arm `(Fresh.next (u) s (resume s (+ s 1)))`
           hands back the current state `s` as the operation's value and threads `s + 1` forward as the
           next state. Three performs therefore see 0, 1, 2, and the `do` yields the last, 2. The handle
           evaluates to the value of its body — the final counter 3 is NOT part of the result, because the
           body never reads it. Contrast a stateless handler (seed unit, thread s unchanged): this one
           genuinely folds. This upgrades the fresh-name idiom from a pure function of its argument to a
           real supply — the compiler's `Fresh` state model.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (do (Fresh.next)
                    (Fresh.next)
                    (Fresh.next)))) (export main)))
  (output (: 2 Int64)))

(case "a performed operation is the scrutinee of a match that dispatches on its result"
  (doc    "Witnesses that an effect operation composes as a match SCRUTINEE, exactly as it composes as an
           `if` condition or an arithmetic operand: `(match (Fresh.next) (0 100) (_ 200))`. The scrutinee is
           evaluated FIRST — `Fresh.next` reads the current counter (seeded 0), hands it back as the
           operation's value, and threads `s + 1` forward — then the match dispatches on that value. Seeded
           0, the first read is 0, so the `0` arm is selected and the handle yields 100. The state threads
           through the scrutinee before the arm bodies run, the same evaluation order any strict operand
           sees; the pattern engine then lowers the (rewritten) match by its ordinary path.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (match (Fresh.next) (0 100) (_ 200)))) (export main)))
  (output (: 100 Int64)))

(case "a performed operation whose result is a sum is matched on its variant"
  (doc    "Extends the match-scrutinee composition to an operation whose declared RESULT is a SUM type — the
           resume value is a compound sum, not a scalar. `Look.find : Int64 -> (Option Int64)`; the arm
           resumes with `(Some (+ k 1))`, a constructed `Option` value carrying the incremented key. The
           handle body `(match (Look.find 41) ((Some v) v) (None 0))` performs, folds the resume value into
           the scrutinee position, and dispatches on its variant: `Look.find 41` yields `(Some 42)`, the
           `(Some v)` arm binds `v = 42`. Pins that the pure one-hole fold substitutes a SUM-typed resume
           value soundly (the value column carries the compound through the match), the effect analogue of a
           sum-typed handler return — a compiler pass performing a lookup that returns an optional result.")
  (input  (do
            (effect Look (op find (-> Int64 (Option Int64))))
            (def (main)
              (handle Look 0 ((find (k) s (resume (Some (+ k 1)) s)))
                (match (Look.find 41) ((Some v) v) (None 0)))) (export main)))
  (output (: 42 Int64)))

(case "an effect op whose declared RESULT is a QUANTITY resumes with a Qty value"
  (doc    "An operation whose declared result is a QUANTITY type `(Qty T u)` — the resume value is a
           unit-carrying `Qty`, not a bare scalar. `Env.width : Unit -> (Qty Int64 meter)`; the arm resumes
           `(Qty.of 5 (Unit.base #\"meter\"))`, and the body reads the magnitude back with `Qty.value` → 5.
           Pins that a Qty-typed operation result flows through the effect machinery: the op's `(meta t)` arrow
           `(-> Unit (Qty Int64 meter))` must reduce to a determined `Ty::Qty` RESULT (the scheme path
           `type_in_env` gained a `QtyCtor` arm; without it the arrow collapsed and the result read as the raw
           op-value record → CDZ0203 'not fully determined'). This is the guest-side of the runtime-parameter
           `@param` Quantity path — a `@param(...) width : Length` generates exactly this Qty-result op (the
           host-boundary num/den ABI for it is a separate later increment; here it is discharged by an
           in-program handler). Identical on both backends.")
  (input  (do
            (effect Env (op width (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main)
              (handle Env unit ((width (u) s (resume (Qty.of 5 (Unit.base #"meter")) s)))
                (Qty.value (Env.width)))) (export main)))
  (output (: 5 Int64)))

(case "a RESULT-returning effect op is matched on Ok / Err — the fallible-step idiom"
  (doc    "The `Result` companion of the Option-result case: an operation whose declared result is a
           `(Result Int64 Int64)`, resumed with an `Ok` or `Err` chosen by the arm, and the body dispatches
           on the variant — the fallible-parser-step shape (a step returns `Ok value` on success or `Err
           code` on failure). `Parse.step : Int64 -> (Result Int64 Int64)`, arm `(step (n) s (resume (if (>
           n 0) (Ok (+ n s)) (Err 99)) (+ s 1)))` — the RESUME value itself branches on the argument. Seeded
           0, `(Parse.step 5)` (n > 0) resumes `(Ok (+ 5 0))` = `(Ok 5)`, and `(match … ((Ok v) v) ((Err e)
           e))` binds `v = 5`. Pins that a `Result`-typed resume value — constructed by an `if` INSIDE the
           arm — folds into the scrutinee and dispatches on Ok/Err (the control the fallible pass runs on;
           the Err path, `(Parse.step -3)` → `(Err 99)` → 99, is its complement). Both backends agree.")
  (input  (do
            (effect Parse (op step (-> Int64 (Result Int64 Int64))))
            (def (main)
              (handle Parse 0 ((step (n) s (resume (if (> n 0) (Ok (+ n s)) (Err 99)) (+ s 1))))
                (match (Parse.step 5) ((Ok v) v) ((Err e) e)))) (export main)))
  (output (: 5 Int64)))

(case "a TUPLE-returning operation resumes a pair built from the handler state, then projected"
  (doc    "An operation whose declared RESULT is a `(Tuple Int64 Int64)`, resumed with a pair BUILT from the
           handler state. `P.pair : Unit -> (Tuple Int64 Int64)`; the arm resumes `(tuple s (+ s 1))` — a
           pair of the current state and its successor. Seeded 5, `(P.pair)` yields `(5, 6)`, and `(. (P.pair)
           1)` projects the second element, 6. Pins that a compound (tuple) resume value built from the
           handler state crosses the pure one-hole fold and is projectable — the tuple companion of the
           sum-result case above, the shape of a stateful op returning several derived values at once.")
  (input  (do
            (effect P (op pair (-> Unit (Tuple Int64 Int64))))
            (def (main)
              (handle P 5 ((pair (u) s (resume (tuple s (+ s 1)) s)))
                (. (P.pair) 1))) (export main)))
  (output (: 6 Int64)))

(case "a TUPLE-result perform's projected field feeds a SECOND perform's argument, threading state across both"
  (doc    "The chained-compound-result shape: a perform returning a TUPLE has one of its fields projected
           and fed as the ARGUMENT to a SECOND perform, with the handler state threading across BOTH. Two
           ops on one effect: `St.pair : Unit -> (Tuple Int64 Int64)` resumes `(tuple s (+ s 1))` and
           advances the state by 10; `St.add : Int64 -> Int64` resumes `(+ n s)` (state held). Seeded 5:
           `(St.pair)` yields `(5, 6)` and threads state → 15; `(. (St.pair) 1)` projects 6; then `(St.add
           6)` reads n = 6 and the ADVANCED state s = 15, resuming `6 + 15` = 21. Pins that a COMPOUND
           perform result flows through a projection into a later perform's argument AND the state threads
           inner-to-outer across the two performs (the pair's +10 advance is visible to the add) — the
           compound-result companion of the nested/argument-position scalar sequencing cases, and the shape a
           pass takes when one effectful step returns a bundle a later step consumes. Both backends agree.")
  (input  (do
            (effect St (op pair (-> Unit (Tuple Int64 Int64))) (op add (-> Int64 Int64)))
            (def (main)
              (handle St 5 ((pair (u) s (resume (tuple s (+ s 1)) (+ s 10)))
                            (add (n) s (resume (+ n s) s)))
                (St.add (. (St.pair) 1)))) (export main)))
  (output (: 21 Int64)))

(case "a handler whose STATE is a sum destructures it in the arm"
  (doc    "The handler's threaded STATE is a SUM (`Option Int64`), and the arm DESTRUCTURES it with a `match`
           to decide the resume value — the state-as-sum analogue of the scalar-countdown handlers. Seeded
           `(Some 5)`, the `get` arm matches its state `s`: `(Some n)` resumes with the payload `n`, `None`
           resumes `0` (a total handler over the state's variants). The body `(+ 1 (St.get))` performs once,
           reads `5` from the `(Some 5)` state, and yields `(+ 1 5)` = 6. Pins that a handler's state slot
           carries a compound sum through the fold and the arm may pattern-match it — the shape of a pass
           threading an optional/typed piece of context (a `Maybe`-valued accumulator) across performs.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (handle St (Some 5) ((get (u) s (match s ((Some n) (resume n s)) (None (resume 0 s)))))
                (+ 1 (St.get)))) (export main)))
  (output (: 6 Int64)))

(case "a handler whose STATE is a TUPLE reads both fields and rebuilds the pair per performance"
  (doc    "The handler's threaded state is a TUPLE packing TWO independent slots — a running accumulator and
           a fixed base — and the arm READS BOTH components (via projection) and REBUILDS the pair to thread
           a modified state, a read-modify-write on a compound state slot. `Acc.step : Int64 -> Int64`, arm
           `(step (v) p (resume (+ (. p 0) (. p 1)) (tuple (+ (. p 0) v) (. p 1))))`: it resumes with the sum
           of the two fields and threads a new tuple advancing only field 0 by `v` (field 1 held). Seeded
           `(0, 100)`: `(Acc.step 1)` reads `(0, 100)` → resumes `0 + 100` = 100, state → `(1, 100)`; `(Acc.step
           2)` reads `(1, 100)` → resumes `1 + 100` = 101, state → `(3, 100)`; so `(+ 100 101)` = 201. Pins
           that a handler state slot carries a TUPLE through the fold — the arm projects its fields and
           reconstructs it — the compound-scalar-pair companion of the sum-state and list-state cases (two
           independent scalar sub-states threaded in one tuple slot, not one shared counter).")
  (input  (do
            (effect Acc (op step (-> Int64 Int64)))
            (def (main)
              (handle Acc (tuple 0 100) ((step (v) p (resume (+ (. p 0) (. p 1)) (tuple (+ (. p 0) v) (. p 1)))))
                (+ (Acc.step 1) (Acc.step 2)))) (export main)))
  (output (: 201 Int64)))

(case "a handler whose STATE is a RECORD combining a scalar counter and a heap LIST field"
  (doc    "The handler state is a RECORD with a scalar field AND a HEAP field (a list) — the AST-node
           accumulator shape (a record of results one of whose fields is a heap value). Each `push` arm READS
           both fields and REBUILDS the record: it increments the scalar `n` and conses the value onto the
           list `xs`, threading the new record; the `count` arm reads back the scalar `n`. `Acc.push : Int64
           -> Int64`, `Acc.count : Unit -> Int64`, seeded `{n: 0, xs: []}`: `(Acc.push 10)` → `{n: 1, xs:
           [10]}`, `(Acc.push 20)` → `{n: 2, xs: [20, 10]}`, `(Acc.count)` reads `n` = 2. Pins that a handler
           state slot carries a RECORD with a nested HEAP field through the fold — the arm projects its
           fields (scalar and heap) and reconstructs the record, so the value-heap field is correctly
           threaded read-modify-write across performs (the compound-with-heap-field companion of the
           scalar-pair tuple-state and the Set-state cases). Both backends agree (the readout is the scalar
           field).")
  (input  (do
            (effect Acc (op push (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main)
              (handle Acc (record (n 0) (xs (list)))
                ((push (v) st (resume v (record (n (+ (. st n) 1)) (xs ((. List push) (. st xs) v)))))
                 (count (u) st (resume (. st n) st)))
                (let ((a (Acc.push 10))) (let ((b (Acc.push 20))) (Acc.count))))) (export main)))
  (output (: 2 Int64)))

(case "an arm chooses its resume value by an if on the handler state"
  (doc    "A handler arm whose body is NOT a bare `(resume …)` but an `if` on the STATE that resumes a
           different value per branch — a CONDITIONAL resume. `(get (u) s (if (> s 5) (resume 100 s) (resume
           200 s)))`: the arm inspects its state `s` and resumes 100 when `s > 5`, else 200. Seeded 7,
           `7 > 5` holds, so `(Ask.get)` resumes 100 and the body `(+ 1 (Ask.get))` = `(+ 1 100)` = 101.
           Pins that the fold serves an arm that branches on its state to pick the resume value (each branch
           a tail resume) — the scalar-`if` companion of the sum-state `match` arm above, the shape of a
           handler that answers differently depending on the accumulated context.")
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Ask 7 ((get (u) s (if (> s 5) (resume 100 s) (resume 200 s))))
                (+ 1 (Ask.get)))) (export main)))
  (output (: 101 Int64)))

(case "a performed operation composes under a projection and a negation"
  (doc    "Witnesses that an effect operation composes under the STRICT one-operand forms — a tuple
           projection and a boolean negation — exactly as under arithmetic. `(. (tuple (Fresh.next)
           (Fresh.next)) 1)` builds a pair from two successive reads (seeded 0 → 0 and 1) and projects the
           second, 1; `(not (= … 0))` negates a comparison of a performed value. Both operands are evaluated
           left to right, threading the counter, before the enclosing op applies. This pins that the fold
           threads through projection/negation, not only conditionals and arithmetic — a performed value is
           an ordinary sub-expression everywhere it appears.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (. (tuple (Fresh.next) (Fresh.next)) 1))) (export main)))
  (output (: 1 Int64)))

(case "a perform composes as the SOURCE of a pipeline"
  (doc    "An effect operation composes as the LHS value of a `|>` pipeline — the common surface form for
           `f(perform())`. `(|> (Fresh.next) (+ 100))` desugars to the application `(+ (Fresh.next) 100)`
           (the pipeline splices its value as the first argument of the rhs application), so the perform is
           an ordinary strict operand the fold threads. `Fresh.next` seeded 5 resumes 5, and `5 + 100` = 105.
           Pins that the pipeline desugar preserves the perform's strict-operand position — a performed value
           flows through `|>` exactly as through a direct application, the way an effectful pass reads
           `input |> transform`.")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (main)
              (handle Fresh 5 ((next () s (resume s (+ s 1))))
                (|> (Fresh.next) (+ 100)))) (export main)))
  (output (: 105 Int64)))

(case "performs in the ELEMENTS of a tuple / list CONSTRUCTOR thread left-to-right"
  (doc    "A perform in a tuple or list CONSTRUCTOR element is a strict, ordered position — each element is
           evaluated exactly once, left to right, before the compound is built — so the fold threads it like
           an arithmetic operand or a call argument. This pins the STRING-HEADED constructor primitive
           `(\"tuple\" …)` / `(\"list\" …)`, which is what the ML surface's tuple/list literal `(a, b)` /
           `[a, b]` lowers to (a bare `tuple` NAME reduces via `(meta apply)` and threads through the call
           path; the string-head ctor is the primitive and reaches the compound-constructor fold arm). Two
           `Fresh.next` reads in a tuple, projected and summed: seeded 0, the elements read 0 then 1, so `(+
           (. p 0) (. p 1))` = `0 + 1` = 1. Before this, a perform in a tuple/list/record element declined
           ('not yet reducible by the tail-resumptive fold') — the ML surface (which always emits the
           string-head ctor) could not build a tuple/list from performed values without a manual prefetch;
           now the fold hoists the perform out of the element position like the operand/arg/sum-payload
           cases it already handled. (Record fields — a `(label value)` pair structure — are a follow-up.)")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1))))
                (let ((p ("tuple" (Fresh.next) (Fresh.next)))) (+ (. p 0) (. p 1))))) (export main)))
  (output (: 1 Int64)))

(case "performs in the FIELD VALUES of a RECORD constructor thread in written order"
  (doc    "The record companion of the tuple/list-element case: a perform in a RECORD field VALUE is a
           strict, ordered position — each field value is evaluated in WRITTEN order before the record is
           built — so the fold threads it and rebuilds the `(\"record\" (label rvalue)…)` form, keeping the
           labels. This pins the STRING-HEADED record ctor primitive `(\"record\" …)`, what the ML record
           literal `{ a = …, b = … }` lowers to (its `(label value)` pair args). The fields are WRITTEN `b`
           then `a` (reverse of sorted order) to pin that the VALUES evaluate in written order, not the
           record's canonical sorted order: seeded 0, `b`'s value reads 0 and `a`'s reads 1, so `(- (. r a)
           (. r b))` = `1 - 0` = 1 (had it evaluated `a` first — sorted order — it would be `0 - 1` = -1).
           Before this the record-field perform declined; the fold now hoists it like the tuple/list element
           and the operand/arg/sum-payload cases. Completes the compound-constructor element threading
           (tuple / list / record).")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1))))
                (let ((r ("record" (b (Fresh.next)) (a (Fresh.next))))) (- (. r a) (. r b))))) (export main)))
  (output (: 1 Int64)))

(case "performs in the VALUES of a MAP constructor thread and the entry reads back correctly"
  (doc    "The map completion of the compound-constructor element threading (tuple/list/record above). A
           perform in a map entry's VALUE is a strict, ordered position — each entry is evaluated in written
           order, and within an entry the key then the value — so the fold threads it and rebuilds the
           `(\"map\" (key rvalue)…)` string-headed ctor. Two `Fresh.next` VALUES under keys 10 and 20: seeded
           0, the first entry's value reads 0 (under key 10), the second reads 1 (under key 20); looking up
           key 20 returns `(Some 1)`, matched to 1. Pins that a map built from performed values threads the
           reads in entry order and stores each under its key correctly (a lookup confirms key 20 holds the
           second read, 1, not the first). Completes tuple / list / record / MAP — an effectful program can
           build any compound from performed values directly. (wasm: rust declines — value-heap/Map emission
           parity gap, not the effects fold.)")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1))))
                (let ((m ("map" (10 (Fresh.next)) (20 (Fresh.next)))))
                  (match (Map.lookup m 20) ((Some v) v) (None 99))))) (export main)))
  (output (: 1 Int64)))

(case "a SUM constructor payload that is a compound built from performs threads and destructures"
  (doc    "The composition of the sum-constructor payload path with the compound-constructor element
           threading: the payload of `Some` is a TUPLE built from two performs — `(Some (\"tuple\"
           (Fresh.next) (Fresh.next)))` — using the STRING-HEADED tuple ctor the ML surface emits. The tuple
           threads its two performs (reads 0 then 1), the sum ctor wraps the threaded `(0, 1)`, and the
           enclosing match destructures it: `(Some p)` → `(+ (. p 0) (. p 1))` = `0 + 1` = 1. Pins that a
           compound built from performs composes INSIDE a sum constructor payload (a scalar sum payload
           `W.Mk(Fresh.next())` already worked; this is the compound-payload companion) — the fold threads
           the nested compound-ctor element positions and the sum ctor is a transparent wrapper over the
           threaded value. The shape a real pass builds when it returns `Some((id, node))` from an effectful
           walk. Both backends agree.")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1))))
                (match (Some ("tuple" (Fresh.next) (Fresh.next)))
                  ((Some p) (+ (. p 0) (. p 1)))
                  (None 99)))) (export main)))
  (output (: 1 Int64)))

(case "a performed operation composes as a RECORD field value that is then projected"
  (doc    "The record-constructor companion of the tuple/projection case: a perform in a RECORD FIELD VALUE
           is a strict, unconditional position, so it composes and the surrounding projection is a pure
           one-hole context. `(. (record (x (Ask.get)) (y 3)) x)` builds a record whose `x` field is the
           performed value, then projects `x` — `C = (. (record (x []) (y 3)) x)`, a strongly-pure context
           around the single perform. `Ask.get` resumes 7, so the record is `{x: 7, y: 3}` and the projection
           yields 7. Pins that the fold threads through a record field the same as a tuple element — a
           performed value is an ordinary sub-expression in a compound constructor.")
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Ask 0 ((get (u) s (resume 7 s)))
                (. (record (x (Ask.get)) (y 3)) x))) (export main)))
  (output (: 7 Int64)))

(case "a FLOAT-result effect op threads a float state and its resume folds under a float-dispatched operator"
  (doc    "The value and state columns of the effect fold are TYPE-AGNOSTIC: an operation whose result and
           whose handler state are both Float64 thread through the same machinery as the Int64 cases, and the
           `+` in the continuation — which now dispatches on operand TYPE (float `+`, no separate `+.`) —
           resolves to the float add inside the folded body. `Rng.next : Unit -> Float64`, arm `(next (u) s
           (resume s (+ s 2.0)))`, seeded 1.5: `(Rng.next)` reads 1.5 (state → 3.5), the second reads 3.5
           (state → 5.5), so `(+ 1.5 3.5)` = 5.0. Pins that (i) a non-Int64 SCALAR result/state slot threads
           correctly (the fold copies values by identity, indifferent to their type) and (ii) the unified
           numeric `+` picks the Float64 add within a folded continuation — the float companion of the
           two-lets / operator-operand Int64 sequencing cases.")
  (input  (do
            (effect Rng (op next (-> Unit Float64)))
            (def (main)
              (handle Rng 1.5 ((next (u) s (resume s (+ s 2.0))))
                (+ (Rng.next) (Rng.next)))) (export main)))
  (output (: 5.0 Float64)))

(case "a BOOL-result effect op threads state across two performs on a boolean connective's operands"
  (doc    "The Bool companion of the float/Int64 sequencing cases: a `Unit -> Bool` operation whose resume
           value is derived from the handler state, performed on BOTH operands of an `and` (the left operand
           is true, so the connective does NOT short-circuit and the right also runs). `Coin.flip : Unit ->
           Bool`, arm `(flip (u) s (resume (= s 0) (+ s 1)))`, seeded 0: the first `(Coin.flip)` reads `(= 0
           0)` = true (state → 1), the second reads `(= 1 0)` = false (state → 2), so `(and true (not
           false))` = `(and true true)` = true. Pins that (i) a Bool result/state column threads through the
           fold like any scalar and (ii) when the connective's LEFT operand is true the RIGHT-operand perform
           genuinely runs and reads the ADVANCED state (had it not threaded, the second would read `(= 0 0)` =
           true too and `(not true)` = false → the whole `and` false). Distinct from the abortive-connective
           and pure-one-hole-in-an-and-lhs cases: here BOTH operands perform and thread tail-resumptively.")
  (input  (do
            (effect Coin (op flip (-> Unit Bool)))
            (def (main)
              (handle Coin 0 ((flip (u) s (resume (= s 0) (+ s 1))))
                (and (Coin.flip) (not (Coin.flip))))) (export main)))
  (output (: true Bool)))

(case "a connective-wrapped perform in an if condition threads its state advance to the taken branch"
  (doc    "A short-circuit connective `(and b (> (St.tick) 0))` sitting DIRECTLY in an `if` CONDITION — the
           condition's `tick` advances the handler state, and the taken branch's `tick` must READ that advance.
           Seeded 0, arm `(tick (u) s (resume (+ s 1) (+ s 1)))`; with `b = true` the condition's `tick` resumes
           1 (state → 1), so the then-branch `(St.tick)` resumes 2. Had the condition's advance been dropped (the
           connective → `if`-desugar's out-state is the post-CONDITION state, which the `If` thread arm does not
           observe per-branch), the then-branch would read the seed and resume 1 — the silent miscompile this
           pins. FIXED by hoist Site 5: a conditional whose CONDITION/SCRUTINEE itself performs in a branch is
           bound to a `let` (`(if C t e)` ≡ `(let ((#cv C)) (if #cv t e))`), turning C into a `let`-init that
           Site 4 distributes so each branch threads under C's advanced state. Controls that already threaded
           (bare effectful compare in the cond, a LET-bound connective) are unaffected; `not` is not part of the
           broken desugar. b=true → 2.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main (: b Bool))
              (handle St 0
                ((tick (u) s (resume (+ s 1) (+ s 1))))
                (if (and b (> (St.tick) 0)) (St.tick) -99)))
            (export main)))
  (call   main (: true Bool))
  (output (: 2 Int64)))

(case "two performs bound by nested lets thread the handler state in order"
  (doc    "Two performs on the strict spine, each BOUND by its own `let`, thread the handler state in
           evaluation order across the binds. `(let ((a (Ask.get))) (let ((b (Ask.get))) (+ a b)))` under a
           counter that hands back `s` and threads `s + 10` (seeded 0): the first `Ask.get` binds `a = 0`
           (state → 10), the second binds `b = 10` (state → 20), so `(+ a b)` = `(+ 0 10)` = 10. The `let`
           inits run unconditionally in sequence — a strict spine the threading fold walks left to right —
           so each perform sees the state the previous one advanced, not the seed. Pins sequential
           state-threading through a chain of let bindings (had the state not threaded, both reads would be
           0 and the sum 0), the essential shape of a pass pulling several fresh values in a row.")
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Ask 0 ((get (u) s (resume s (+ s 10))))
                (let ((a (Ask.get))) (let ((b (Ask.get))) (+ a b))))) (export main)))
  (output (: 10 Int64)))

(case "two performs of a MULTI-parameter op each combine both args with the state advancing between them"
  (doc    "A two-scalar-parameter operation whose arm combines BOTH arguments with the threaded state, called
           twice on one strict spine so each perform reads the state the previous one advanced. `Acc.add2 :
           Int64 -> Int64 -> Int64`, arm `(add2 (a b) s (resume (+ (+ a b) s) (+ s 1)))` — sums its two args
           plus the current state, threading `s + 1`. Seeded 100: `(Acc.add2 1 2)` = `1 + 2 + 100` = 103
           (state → 101), then `(Acc.add2 10 20)` = `10 + 20 + 101` = 131 (state → 102), so `(+ 103 131)` =
           234. Pins that a multi-parameter op's arm binds ALL its parameters AND the state, and that the
           state advances between successive performs on the spine (had it not threaded, the second would
           read 100 too, giving 233) — the multi-arg companion of the sequential-state-threading case above.")
  (input  (do
            (effect Acc (op add2 (-> Int64 Int64 Int64)))
            (def (main)
              (handle Acc 100 ((add2 (a b) s (resume (+ (+ a b) s) (+ s 1))))
                (+ (Acc.add2 1 2) (Acc.add2 10 20)))) (export main)))
  (output (: 234 Int64)))

(case "a THREE-parameter op arm binds all three parameters and the state"
  (doc    "The arity extension of the two-parameter case: an operation with THREE scalar parameters whose
           arm binds all three plus the state. `Acc.add3 : Int64 -> Int64 -> Int64 -> Int64`, arm `(add3 (a
           b c) s (resume (+ (+ (+ a b) c) s) (+ s 1)))` — sums its three args plus the current state,
           threading `s + 1`. Seeded 1000: `(Acc.add3 1 2 3)` = `1 + 2 + 3 + 1000` = 1006 (state → 1001),
           then `(Acc.add3 10 20 30)` = `10 + 20 + 30 + 1001` = 1061 (state → 1002), so `(+ 1006 1061)` =
           2067. Pins that arm-parameter binding scales past two — all three op parameters AND the state
           binder resolve in the arm body, and the state still threads between successive performs on the
           spine.")
  (input  (do
            (effect Acc (op add3 (-> Int64 Int64 Int64 Int64)))
            (def (main)
              (handle Acc 1000 ((add3 (a b c) s (resume (+ (+ (+ a b) c) s) (+ s 1))))
                (+ (Acc.add3 1 2 3) (Acc.add3 10 20 30)))) (export main)))
  (output (: 2067 Int64)))

(case "a perform's result flowing as the ARGUMENT of an enclosing perform threads state inner-to-outer"
  (doc    "The data dependency runs THROUGH the argument position rather than through a let: the inner
           perform's result is the very argument the outer perform consumes — `(Acc.step (Acc.step 1))`.
           Because an argument is evaluated before its call, the INNER perform runs first and advances the
           state the OUTER one then reads, so the two are still sequenced left-of-the-arrow / inner-first.
           `Acc.step : Int64 -> Int64`, arm `(step (a) s (resume (+ a s) (+ s 1)))`, seeded 100: inner
           `(Acc.step 1)` = `1 + 100` = 101 (state → 101), outer `(Acc.step 101)` = `101 + 101` = 202 (state
           → 102), so the result is 202. Pins that state threads through nested-perform ARGUMENT evaluation
           in inner-to-outer order (had the outer read the seed 100 instead of the inner's advanced 101 it
           would be 201) — the argument-position companion of the two-lets and multi-param cases above, with
           the added twist that one perform's OUTPUT is the other's INPUT.")
  (input  (do
            (effect Acc (op step (-> Int64 Int64)))
            (def (main)
              (handle Acc 100 ((step (a) s (resume (+ a s) (+ s 1))))
                (Acc.step (Acc.step 1)))) (export main)))
  (output (: 202 Int64)))

(case "two performs as the two ARGUMENTS of a pure USER function thread the state left-to-right"
  (doc    "The performs sit in the argument list of a non-primitive, effect-free USER function, whose call
           evaluates its arguments left-to-right before applying — so the two reads are sequenced by the
           call's own argument evaluation, not by an operator or a let. `sub a b = a - b`, `Acc.get : Unit
           -> Int64`, arm `(get (u) s (resume s (+ s 5)))`, seeded 10: `(sub (Acc.get) (Acc.get))` reads the
           first arg as 10 (state → 15) and the second as 15 (state → 20), so `(sub 10 15)` = -5. Pins that
           the fold sequences performs across a user call's ARGUMENT list identically to operator operands
           (had the args not threaded, both would read 10 → 0) — the user-call companion of the operator-
           operand and nested-perform cases, and distinct from the arms that call an effect-free helper on a
           resume RESULT (the performs here are the call's inputs, sequenced at the call site).")
  (input  (do
            (effect Acc (op get (-> Unit Int64)))
            (def (sub a b) (- a b))
            (def (main)
              (handle Acc 10 ((get (u) s (resume s (+ s 5))))
                (sub (Acc.get) (Acc.get)))) (export main)))
  (output (: -5 Int64)))

(case "a do-sequence of unit-returning performs runs each for effect, then yields the tail value"
  (input  (do
            (effect Log (op w (-> Int64 Unit)))
            (def (main)
              (handle Log 0 ((w (n) s (resume unit (+ s n))))
                (do (Log.w 3) (Log.w 4) 99))) (export main)))
  (doc    "The side-effect-only sequencing shape: a `do` of two UNIT-returning performs run purely for
           effect, then a tail value. `Log.w : Int64 -> Unit` accumulates its argument into the handler
           state (seeded 0, threads `s + n`); `(do (Log.w 3) (Log.w 4) 99)` performs `Log.w 3` (state 0 → 3)
           then `Log.w 4` (state 3 → 7) — each yields unit, discarded — and the sequence's value is the tail
           `99`. Pins that a chain of unit-op performs threads state in order while their unit results are
           dropped, the essential shape of a compiler pass that EMITS several diagnostics (each advancing an
           accumulator) then returns its result. The handler's threaded total is observed only through the
           state; the handle's value is the body's tail.")
  (output (: 99 Int64)))

(case "textually-identical performs are DISTINCT state-advancing reads, not a common subexpression"
  (doc    "A soundness pin against the backend optimizer's CSE/value-numbering: four TEXTUALLY-IDENTICAL
           `(C.t)` performs are FOUR DISTINCT reads that each advance the handler state, NOT a common
           subexpression to dedup. `(+ (* (C.t) (C.t)) (* (C.t) (C.t)))` seeded 0, arm `(resume s (+ s 1))`:
           evaluated left-to-right, the four reads are 0, 1, 2, 3, so it is `(+ (* 0 1) (* 2 3))` = `(+ 0 6)`
           = 6. Were the compiler to treat the identical `(C.t)` as a common subexpression and compute it
           ONCE (a CSE that ignores effect ordering), the answer would be wrong (e.g. `(* 0 0) + (* 0 0)` =
           0). The effect fold discharges each perform to its own distinct state-advancing read BEFORE the
           optimizer runs, so straight-line CSE never sees a shared effectful node — pinned here at 6.")
  (input  (do
            (effect C (op t (-> Unit Int64)))
            (def (main)
              (handle C 0 ((t (u) s (resume s (+ s 1))))
                (+ (* (C.t) (C.t)) (* (C.t) (C.t))))) (export main)))
  (output (: 6 Int64)))

(case "DOMINATOR CSE does not reuse a condition's perform-product in the taken branch"
  (doc    "The conditional companion of the straight-line CSE pin above, against the backend's DOMINATOR CSE
           (which hoists a subexpression computed in an `if` CONDITION into a branch it dominates). The
           condition and the taken branch each contain the TEXTUALLY-IDENTICAL product `(* (C.t) (C.t))`, but
           they are DISTINCT state-advancing reads — the branch must recompute, NOT reuse the condition's
           value. `C` seeded 1, arm `(resume s (+ s 1))`: the condition `(* (C.t) (C.t))` reads 1 then 2 = 2,
           and `2 > 0` is true; the taken then-branch `(* (C.t) (C.t))` reads 3 then 4 = 12. So the result is
           12. Were dominator CSE to hoist the condition's product and reuse it in the branch (ignoring that
           each `(C.t)` is a distinct effectful read), the branch would wrongly yield 2. Sound because the
           fold discharges every perform to its own distinct read BEFORE the optimizer runs, so no effectful
           node is ever shared for CSE to hoist — pinned at 12 across an `if` this time, not just a
           straight-line spine.")
  (input  (do
            (effect C (op t (-> Unit Int64)))
            (def (main)
              (handle C 1 ((t (u) s (resume s (+ s 1))))
                (if (> (* (C.t) (C.t)) 0) (* (C.t) (C.t)) 99))) (export main)))
  (output (: 12 Int64)))

(case "an if→SELECT-eligible conditional with a performed condition and pure branches stays sound"
  (doc    "A soundness pin against the backend's if→SELECT conversion (which turns a small trap-free `if`
           into a branchless `select` that evaluates BOTH arms eagerly). The condition performs and the two
           branches are pure scalar values — exactly the shape the conversion targets. `C` seeded 3, arm
           `(resume s (+ s 1))`: the condition `(C.t)` reads 3 (state → 4), `3 < 5` is true, so the pure
           then-branch `10` is the value. Sound because the perform is discharged to a single sequenced read
           in the CONDITION before the optimizer runs — the branches carry no effectful node, so converting
           the `if` to a branchless `select` (eager on both scalar arms) cannot duplicate, reorder, or drop
           the perform. Pinned at 10 — the perform runs exactly once, in the condition, regardless of the
           if/select lowering. (Distinct from the CSE pins: here the concern is the branchless-select
           transform evaluating both arms, not a shared subexpression being hoisted.)")
  (input  (do
            (effect C (op t (-> Unit Int64)))
            (def (main)
              (handle C 3 ((t (u) s (resume s (+ s 1))))
                (if (< (C.t) 5) 10 20))) (export main)))
  (output (: 10 Int64)))

(case "a 2-arm match→SELECT with a performed scrutinee and pure arms stays sound"
  (doc    "The match companion of the if→SELECT pin, against the backend's 2-arm match→SELECT conversion
           (which lowers a small trap-free two-arm `match` to a branchless `select` evaluating both arm
           values eagerly). The SCRUTINEE performs and the two arm bodies are pure scalar values — the shape
           the conversion targets. `C` seeded 7, arm `(resume s (+ s 1))`: the scrutinee `(C.t)` reads 7
           (state → 8), which does not match the `0` arm, so the `_` arm's `200` is the value. Sound because
           the perform is discharged to a single sequenced read in the SCRUTINEE before the optimizer runs —
           the arm bodies carry no effectful node, so converting the match to a branchless `select` (eager on
           both scalar arms) cannot duplicate, reorder, or drop the perform. Pinned at 200 — the perform runs
           exactly once, in the scrutinee, regardless of the match/select lowering. The control (seed 0 so
           the scrutinee reads 0) selects the `0` arm → 100.")
  (input  (do
            (effect C (op t (-> Unit Int64)))
            (def (main)
              (handle C 7 ((t (u) s (resume s (+ s 1))))
                (match (C.t) (0 100) (_ 200)))) (export main)))
  (output (: 200 Int64)))

; --- A perform inside an if/match BRANCH threads its state OUT to the continuation after the conditional.
; A branch's state advance is not local to the branch: the code following the conditional must run against
; the branch's POST-state, not the pre-branch state. Because only one branch runs, the state after the
; conditional is a runtime PHI of the branches — realized by distributing the continuation into each
; branch (`(do (if c t e) k)` ≡ `(if c (do t k) (do e k))`), so the conditional ends up in tail position
; where the fold threads correctly. The condition/scrutinee is evaluated exactly once (never duplicated);
; a short-circuit connective is the same shape via its if-desugar. Contrast: a conditional in TAIL position
; (no continuation) and a perform in the CONDITION both already threaded — the gap was specifically a
; branch perform whose advance must flow OUT to a continuation.

(case "a perform in a taken if-branch threads its state to the continuation after the if"
  (doc    "The then-branch performs `Fresh.next` (reads 0, threads 0->1); the continuation `(Fresh.next)`
           after the `if` reads 1. The branch's state advance is NOT lost — it flows out to the code after
           the conditional. `if` in tail position and a perform in the condition both thread already; this
           pins the branch-then-continuation case, realized by lifting the `if` to tail position and
           distributing the continuation into each branch.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (do (if true (Fresh.next) 99) (Fresh.next)))) (export main)))
  (output (: 1 Int64)))

(case "a perform in a taken match-arm threads its state to the continuation after the match"
  (doc    "Same threading via `match`: the `0` arm performs `Fresh.next` (reads 0, threads 0->1); the
           continuation `(Fresh.next)` reads 1. Confirms the phi-out-of-branch threading is in the shared
           conditional fold, not `if`-specific — a `match` arm body is a branch position exactly like an
           `if` branch.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (do (match 0 (0 (Fresh.next)) (_ 99)) (Fresh.next)))) (export main)))
  (output (: 1 Int64)))

(case "a performing match SCRUTINEE threads its state into a performing arm body"
  (doc    "Both the match SCRUTINEE and the selected ARM BODY perform, and the arm's perform reads the state
           the SCRUTINEE's perform advanced — the two-hole shape through a performing `match`. `Ask` seeded
           3, `get` hands back `s` and threads `s - 1`: the scrutinee `(Ask.get)` reads 3 (state -> 2) and
           binds the `n` arm (`3 != 0`); the arm body `(+ n (Ask.get))` performs again, reading the advanced
           state 2, so it is `(+ 3 2)` = 5. Pins that state threaded THROUGH a performing scrutinee reaches a
           performing arm body — the scrutinee is a strict-first position whose effect is sequenced before
           the arm runs, exactly as an operator operand's is. Distinct from the constant-scrutinee arm-thread
           case above (there the scrutinee is the literal `0`; here it performs).")
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Ask 3 ((get (u) s (resume s (- s 1))))
                (match (Ask.get) (0 (Ask.get)) (n (+ n (Ask.get)))))) (export main)))
  (output (: 5 Int64)))

(case "a match arm's DESTRUCTURED payload is the argument to a perform in that arm's body"
  (doc    "A match arm destructures a sum constructor, binding its payload, and that BOUND VALUE is the
           argument to a perform in the arm body — the binder-into-perform-argument shape. The scrutinee
           `(Some 5)` is a pure literal, so the `(Some n)` arm binds `n = 5` and its body `(Ctr.tick n)`
           performs `Ctr.tick` with that bound payload. `Ctr.tick : Int64 -> Int64`, arm `(tick (d) s (resume
           (+ d s) (+ s 1)))`, seeded 100: `(Ctr.tick 5)` = `5 + 100` = 105. Pins that a value bound by a
           constructor pattern in a match arm flows correctly as a perform's argument (the arm binder is in
           scope for the perform, and the fold threads the handler state through it) — distinct from the
           performing-scrutinee case above (there the scrutinee performs and the arm reads STATE; here the
           scrutinee is pure and the arm feeds its BOUND payload into the op).")
  (input  (do
            (effect Ctr (op tick (-> Int64 Int64)))
            (def (main)
              (handle Ctr 100 ((tick (d) s (resume (+ d s) (+ s 1))))
                (match (Some 5) ((Some n) (Ctr.tick n)) (None 0)))) (export main)))
  (output (: 105 Int64)))

(case "the else-branch of an if threads a performed state to the continuation"
  (doc    "The else-branch (taken, cond false) performs once (reads 0, threads 0->1); the continuation reads
           1. Pins that BOTH arms thread out, not just the then-arm — the distribution wraps the continuation
           into each branch.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (do (if false 99 (Fresh.next)) (Fresh.next)))) (export main)))
  (output (: 1 Int64)))

(case "a short-circuit connective threads a branch perform to the continuation"
  (doc    "`(and (= (Fresh.next) 0) (= (Fresh.next) 1))` desugars to `(if (= (Fresh.next) 0) (= (Fresh.next)
           1) false)`; both reads happen (0, then 1), threading 0->2. The continuation `(Fresh.next)` reads
           2. The connective's rhs is a branch (runs only on the taken path), so its perform's advance must
           flow out to the continuation exactly as an explicit `if` branch's does — even though the condition
           itself performs (the condition threads, then the branch, then the distributed continuation).")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (do (and (= (Fresh.next) 0) (= (Fresh.next) 1)) (Fresh.next)))) (export main)))
  (output (: 2 Int64)))

(case "branches performing DIFFERENT counts each thread their own post-state to the continuation"
  (doc    "The two branches advance the state by different amounts — the then-branch reads once (0->1), the
           else-branch reads twice (0->1->2). With cond true the then-branch runs, so the continuation reads
           1; the continuation is threaded independently through each branch's own post-state, not a single
           merged one. Pins the phi is per-branch: the distributed continuation sees whichever branch ran.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (do (if true (Fresh.next) (do (Fresh.next) (Fresh.next))) (Fresh.next)))) (export main)))
  (output (: 1 Int64)))

(case "a branch perform under two nested handlers threads the inner state to the continuation"
  (doc    "The branch performs the INNER effect `A` (reads 0, threads 0->1); the continuation `(A.an)` reads
           1. The outer handler `B` is present but unperformed. Pins that the branch-to-continuation
           threading composes with nested handlers — the distribution preserves each effect's own state
           slot, so the inner effect's branch advance still reaches the continuation under the outer fold.")
  (input  (do
            (effect A (op an (-> Unit Int64)))
            (effect B (op bn (-> Unit Int64)))
            (def (main)
              (handle B 100 ((bn (u) t (resume t (+ t 1))))
                (handle A 0 ((an (u) s (resume s (+ s 1))))
                  (do (if true (A.an) 0) (A.an))))) (export main)))
  (output (: 1 Int64)))

(case "a handler accumulates into a list and a read-out operation reads it back"
  (doc    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges and #A Handler Evaluates To The Value Of Its Body: `Diag` declares two operations —
           `emit : Int64 -> Unit` (record a diagnostic) and `collect : Unit -> (List Int64)` (read the
           accumulated diagnostics). The handler is seeded with the empty list; each `(Diag.emit code)`
           resumes with `unit` and threads `(List.push s code)` forward, accumulating `(list 201 210)`;
           then `(Diag.collect)` reads the accumulated list back — its arm `(resume s s)` hands the state
           out as the operation's value and threads it unchanged. Because the read-out is an ORDINARY
           OPERATION, the handler needs no separate return clause: the body pulls the accumulator into its
           own value by performing `collect`, and the handle evaluates to that body value `(list 201 210)`.
           This is the compiler's diagnostics idiom as a real accumulator (the earlier record-and-continue
           `Diag.emit` that resumed unit and discarded the code was the stateless placeholder for it), and
           it needs the list-growth capability to build the accumulator.")
  (input  (do
            (effect Diag (op emit (-> Int64 Unit))
                         (op collect (-> Unit (List Int64))))
            (def (main)
              (handle Diag (list) ((emit (code) s (resume unit (List.push s code))) (collect (u) s (resume s s))) (do (Diag.emit 201)
                    (Diag.emit 210)
                    (Diag.collect)))) (export main)))
  (output (: (list 201 210) (List Int64))))

(case "a handler threads a SET as its state — the seen-set idiom, deduping across performs"
  (doc    "The Set analogue of the list-accumulator handler: the threaded state is a SET (a `seen`/`visited`
           set), and an `add` operation inserts into it while a `count` operation reads its size. Because a
           Set DEDUPES, two `(Seen.add 2)` performs of the same key leave the set unchanged after the first.
           `Seen.add : Int64 -> Int64`, arm `(add (k) m (resume k (Set.insert m k)))`; `Seen.count : Unit ->
           Int64`, arm `(count (u) m (resume (Set.len m) m))`. Seeded `{1}`: `(Seen.add 2)` → `{1, 2}` (2
           elements), `(Seen.add 2)` inserts the duplicate 2 → still `{1, 2}`, and `(Seen.count)` reads
           `Set.len {1,2}` = 2. Pins that a handler state slot carries a persistent SET through the fold —
           the arm reads it (`Set.len`) and rebuilds it (`Set.insert`) per performance, and the set's
           set-semantics (dedup) hold across the threaded reads — the visited-set idiom a graph/AST walk
           needs. (wasm: the rust target declines — it lacks the value-heap/Set emission the component-model
           backend has, the same backend-parity gap as the list-state cases, not an effects-fold limitation.)")
  (input  (do
            (effect Seen (op add (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main)
              (handle Seen (Set.of (list 1)) ((add (k) m (resume k (Set.insert m k))) (count (u) m (resume (Set.len m) m)))
                (let ((a (Seen.add 2))) (let ((b (Seen.add 2))) (Seen.count))))) (export main)))
  (output (: 2 Int64)))

(case "a handler threads a MAP as its state — a key-value store deduping keys across performs"
  (doc    "The Map analogue of the Set-state seen-set: the threaded state is a MAP (a key→value store), and a
           `put` operation inserts a key while a `count` operation reads its size — exercising the CHAMP
           key→value path through the effect fold, distinct from the key-only Set case. Because a Map keys
           uniquely, `put`ting the same key twice leaves one entry. `Store.put : Int64 -> Unit`, arm `(put (k)
           m (resume unit (Map.insert m k k)))`; `Store.count : Unit -> Int64`, arm `(count (u) m (resume
           (Map.len m) m))`. Seeded empty: `(Store.put 1)` → `{1:1}`, `(Store.put 2)` → `{1:1, 2:2}`,
           `(Store.put 1)` re-inserts key 1 → still two keys, and `(Store.count)` reads `Map.len` = 2. Pins
           that a handler state slot carries a persistent MAP through the fold across MULTIPLE performs — the
           arm reads it (`Map.len`) and rebuilds it (`Map.insert`) per performance, and the map's key-dedup
           holds across the threaded reads (the keyed-store idiom, and a guard that a Map.lookup/insert CSE
           change cannot regress a Map threaded as effect state). (wasm: the rust target declines — it lacks
           the value-heap/Map emission the component-model backend has, the same backend-parity gap as the
           list-state and Set-state cases, not an effects-fold limitation.)")
  (input  (do
            (effect Store (op put (-> Int64 Unit)) (op count (-> Unit Int64)))
            (def (main)
              (handle Store (Map.empty) ((put (k) m (resume unit (Map.insert m k k))) (count (u) m (resume (Map.len m) m)))
                (do (Store.put 1) (Store.put 2) (Store.put 1) (Store.count)))) (export main)))
  (output (: 2 Int64)))

(case "sequenced memoize helpers with a local let thread the Map-state out-state (the memo-spine shape)"
  (doc    "The real memoized-query-DB spine (the shape compiler-ml's #4 hardening needs): a cross-function
           helper `store(k)` that BINDS A LOCAL `let vv = k*10` and performs `Db.put((k, vv))` returning `vv`
           — the memoize combinator's on-miss arm — called TWICE in a `do` SEQUENCE before a final read. The
           first `(store 3)` is a NON-FINAL do item: `put`'s next-state (`Map.insert m k vv`) threads FORWARD
           to `(store 5)` and the trailing `(Db.tot)`, and it references `vv`, which the helper's `let` binds
           LOCAL to the first item. Without the `do`-arm LET-LIFT this leaked `CDZ0101 unbound vv` (the
           out-state spliced past the `let` scope); the fix lifts `(let ((vv …)) lbody)` to wrap the whole
           continuation so `vv` stays in scope. Both stores insert their key → `Db.tot` reads `Map.len` = 2.
           Pins that a memoize helper (local let + get/put) composes in a sequence — the substrate for an
           effect-based salsa-style Db (a FINAL-position such call always worked; the sequenced case is the fix).")
  (input  (do
            (effect Db (op put (-> (Tuple Int64 Int64) Unit)) (op tot (-> Unit Int64)))
            (def (store (: k Int64)) (let ((vv (* k 10))) (do (Db.put (tuple k vv)) vv)))
            (def (main)
              (handle Db (Map.empty)
                ((tot (u) s (resume (Map.len s) s))
                 (put (kv) s (match kv ((tuple k v) (resume unit (Map.insert s k v))))))
                (do (store 3) (store 5) (Db.tot)))) (export main)))
  (output (: 2 Int64)))

(case "a RECURSIVE effectful walk accumulates into a list-state handler"
  (doc    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges with the state on the VALUE HEAP and the performer RECURSIVE — the compiler's real
           diagnostics shape: a recursive program walk emitting into a `list<diagnostic>` threaded as
           handler state. `walk` performs `(Diag.emit n)` at each step of a recursive descent and reads
           the accumulator back with `(Diag.collect)` at the base; the handler seeds `(list)` and threads
           `(List.push s v)`, so `(walk 3)` accumulates `(list 3 2 1)`, whose length is 3. This is the
           combination — recursion AND a runtime-compound handler state — that the effect-context
           monomorphization must lower as a real specialized function (its state lives on the value heap,
           threaded as trailing params/returns), not only the self-contained scalar case. `List.len`
           makes `main` return a scalar so the whole program is the runtime-scalar path.")
  (input  (do
            (effect Diag (op emit (-> Int64 Unit))
                         (op collect (-> Unit (List Int64))))
            (def (walk n)
              (if (< n 1)
                  (Diag.collect unit)
                  (do (Diag.emit n) (walk (- n 1)))))
            (def (main)
              (handle Diag (list) ((emit (v) s (resume unit (List.push s v))) (collect (u) s (resume s s))) (List.len (walk 3)))) (export main)))
  (output (: 3 Int64)))

(case "a recursive effectful walk accumulates into a STRING-state handler"
  (doc    "The rope-STRING analogue of the list-state accumulator above: the handler's threaded state is a
           heap STRING built with `String.concat` across a recursive descent, exercising the value-heap
           runtime's rope path (canonicalized at each construction site) rather than a list. `Log` declares
           `emit : String -> Unit` (append a piece) and `dump : Unit -> String` (read the accumulator);
           the handler seeds `\"\"` and each `(Log.emit \"x\")` resumes `unit` and threads `(String.concat
           s m)`, so a recursive `walk` performing three emits builds `\"xxx\"`, whose byte length is 3.
           `String.byte-len` makes `main` a runtime-scalar so the whole program stays on the scalar path.
           Pins that a handler's threaded state may be a heap STRING carried through the recursive-effectful
           specialization — the String-STATE companion of the list-state accumulator and the String-RESULT
           resume-value case, guarding the effect-mechanism × rope-runtime seam. (wasm: the rust target
           declines — it lacks the value-heap/String emission the component-model backend has, the same
           backend-parity gap as the list-state and String-result cases, not an effects-fold limitation.)")
  (input  (do
            (effect Log (op emit (-> String Unit))
                        (op dump (-> Unit String)))
            (def (walk (: n Int64))
              (if (= n 0)
                  (Log.dump)
                  (do (Log.emit "x") (walk (- n 1)))))
            (def (main)
              (handle Log ""
                ((emit (m) s (resume unit (String.concat s m))) (dump (u) s (resume s s)))
                (String.byte-len (walk 3)))) (export main)))
  (output (: 3 Int64)))

(case "a recursive effectful walk BUILDS a list as its return value, one fresh element per step"
  (doc    "The list is the recursion's RETURN VALUE (not handler state, unlike the accumulator case above):
           a recursive `build` reads a fresh index and CONSES it onto the list the rest of the walk returns.
           The perform is bound in a `let` BEFORE the self-call — `(let ((v (Idx.next))) ((. List push)
           (build (- n 1)) v))` — so `v` reads PRE-recursion state (the sound ordering; a perform AFTER the
           self-call would read the recursion's out-state, which the single-return specialization cannot
           carry and correctly declines). `Idx` seeded 1 threads `s + 1`, so the three steps read 1, 2, 3 —
           three fresh elements — and `(List.len (build 3))` = 3. Pins that effect-context specialization
           lowers a list-BUILDING recursive walk (the shape of a compiler pass collecting fresh names into a
           list as it descends), with the built list crossing to a `List.len` readout via the value heap.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (build (: n Int64))
              (if (= n 0)
                  (list)
                  (let ((v (Idx.next)))
                    ((. List push) (build (- n 1)) v))))
            (def (main)
              (handle Idx 1 ((next (u) s (resume s (+ s 1))))
                ((. List len) (build 3)))) (export main)))
  (output (: 3 Int64)))

(case "the heap list a handle BUILDS escapes the handle and is consumed outside it"
  (doc    "The handle's VALUE is a heap list, and it flows OUT of the handle into the enclosing scope. Unlike
           the case above (which reads `List.len` INSIDE the handle body), here the `handle` expression is
           bound to `xs` in an enclosing `let` and consumed AFTER the handle: `(let ((xs (handle Idx 1 …
           (build 3)))) ((. List len) xs))`. So the effect-built list is the handle's result value, lives on
           the value heap, and is a first-class value the surrounding computation reads — the essential shape
           of a compiler PHASE that runs an effectful walk and hands its collected result (a list of fresh
           names / diagnostics) to the next phase. `Idx` seeded 1 threads `s + 1`, the walk collects three
           elements, and the outside `List.len xs` = 3. Pins that a handle's heap-value result crosses the
           handle boundary intact.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (build (: n Int64))
              (if (= n 0)
                  (list)
                  (let ((v (Idx.next)))
                    ((. List push) (build (- n 1)) v))))
            (def (main)
              (let ((xs (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (build 3))))
                ((. List len) xs))) (export main)))
  (output (: 3 Int64)))

(case "an effect-built heap list bound in a let is USED TWICE and retained across both uses"
  (doc    "The DUP / retain shape for an effect-built heap value: the list a handle builds is bound to `xs`
           and consumed MORE THAN ONCE (`(+ ((. List len) xs) ((. List len) xs))`), so the binding is a
           shared owner the first use must NOT free out from under the second. Unlike the escapes-and-
           consumed-ONCE case above, this exercises the Perceus dup — a multiply-used heap binding must be
           RETAINED, not consumed by its first reader. `Idx` seeded 1, `build 3` collects three elements, and
           `len xs + len xs` = `3 + 3` = 6 (a use-after-free from the first `List.len` consuming `xs` would
           read a freed handle / wrong length). Pins that an effect-built heap value bound and used twice is
           reference-managed correctly across the uses — the effects × dup-retain composition. (wasm: rust
           declines — value-heap/List emission parity gap, not the effects fold.)")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (build (: n Int64))
              (if (= n 0)
                  (list)
                  (let ((v (Idx.next)))
                    ((. List push) (build (- n 1)) v))))
            (def (main)
              (handle Idx 1 ((next (u) s (resume s (+ s 1))))
                (let ((xs (build 3)))
                  (+ ((. List len) xs) ((. List len) xs))))) (export main)))
  (output (: 6 Int64)))

(case "a STRING-result effect op resumes with a string that folds through a concat"
  (doc    "The effect fold's value column carries a heap STRING: an operation returning `String` is resumed
           with a string literal, and that performed value flows into `String.concat` in the continuation.
           `Env.name : Unit -> String`, arm `(name (u) s (resume \"cdz\" s))`, so `(Env.name)` yields the heap
           string `\"cdz\"`; the body `(String.concat (Env.name) \"!\")` appends `\"!\"`, giving `\"cdz!\"`.
           Pins that a performed String resume value threads through the fold like any scalar and composes
           under a heap string operation — the String companion of the tuple/list value-column cases,
           exercising the value-heap runtime for the resume value rather than an immediate. (wasm: the rust
           target declines — it lacks the value-heap/String emission the component-model backend has, the
           same backend-parity gap as the list-building cases, not an effects-fold limitation.)")
  (input  (do
            (effect Env (op name (-> Unit String)))
            (def (main)
              (handle Env 0 ((name (u) s (resume "cdz" s)))
                (String.concat (Env.name) "!"))) (export main)))
  (output (: "cdz!" String)))

(case "a recursive walk threads TWO effects at once — a fresh-index counter and a running total"
  (doc    "The full compiler-pass shape: ONE recursive walk that reads a fresh index from `Idx` AND folds a
           running total through `Tot`, under TWO nested handlers, each threading its own state independently.
           `walk` at each step reads `v = (Idx.next)` (a fresh index) then `(Tot.add v)` (accumulate it), then
           recurses; at the base it reads back the total `(Tot.total)`. `Idx` seeded 1 threads `s + 1` so the
           three indices are 1, 2, 3; `Tot` seeded 0 threads `t + d` so the total is `1 + 2 + 3` = 6. Both
           states are live on the recursion stack simultaneously and thread through DISTINCT slots (the walk
           specializes once against the merged two-effect context — a single shared slot per effect would
           clobber on re-entry). Pins that effect-context monomorphization threads more than one effect
           through one recursive walk — a fresh-name counter AND a diagnostics/total accumulator — the exact
           combination a self-hosting compiler pass needs.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (effect Tot (op add (-> Int64 Int64)) (op total (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (= n 0)
                  (Tot.total unit)
                  (let ((v (Idx.next)))
                    (let ((u (Tot.add v)))
                      (walk (- n 1))))))
            (def (main)
              (handle Tot 0 ((add (d) t (resume t (+ t d))) (total (uu) t (resume t t)))
                (handle Idx 1 ((next (u) s (resume s (+ s 1))))
                  (walk 3)))) (export main)))
  (output (: 6 Int64)))

(case "one effect's result flows as the ARGUMENT to a DIFFERENT effect's op under nested handlers"
  (doc    "The cross-effect, non-recursive companion of the two-effects-in-one-walk case: the result of an
           INNER-handled effect's perform is the very argument an OUTER-handled effect's perform consumes —
           `(Dst.put (Src.get))`. The argument `(Src.get)` is discharged by the inner `Src` handler first
           (advancing the Src state), and its result feeds `Dst.put`, discharged by the outer `Dst` handler
           (advancing the Dst state independently). `Src.get : Unit -> Int64` seeded 5, arm `(get (u) s
           (resume s (+ s 1)))` → reads 5; `Dst.put : Int64 -> Int64` seeded 100, arm `(put (v) t (resume (+
           v t) (+ t 10)))` → `(Dst.put 5)` = `5 + 100` = 105. Pins that a value produced by discharging one
           effect crosses into a DIFFERENT effect's operation as its argument, each threading its own handler
           state through a distinct slot — the two folds compose along the data dependency without sharing or
           clobbering state (distinct from the SAME-effect nested-perform-argument case, where one handler's
           single state slot threads both reads).")
  (input  (do
            (effect Src (op get (-> Unit Int64)))
            (effect Dst (op put (-> Int64 Int64)))
            (def (main)
              (handle Src 5 ((get (u) s (resume s (+ s 1))))
                (handle Dst 100 ((put (v) t (resume (+ v t) (+ t 10))))
                  (Dst.put (Src.get))))) (export main)))
  (output (: 105 Int64)))

(case "a handle's TUPLE value pairing a scalar with a built list escapes and is destructured"
  (doc    "The handle's VALUE is a COMPOUND — a tuple pairing a scalar with an effect-built heap list — and
           the whole tuple escapes the handle to be destructured outside. `(handle Idx 1 … (tuple 42 (build
           2)))` evaluates to `(42, [2,1])`; bound to `r` in an enclosing `let`, `(+ (. r 0) ((. List len)
           (. r 1)))` reads the scalar 42 and the built list's length 2 → 44. Pins that a handle can return a
           MIXED compound (a scalar beside a heap value) as its result and hand it whole to the enclosing
           computation — a phase returning both a summary count and its collected list.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (build (: n Int64))
              (if (= n 0) (list) (let ((v (Idx.next))) ((. List push) (build (- n 1)) v))))
            (def (main)
              (let ((r (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (tuple 42 (build 2)))))
                (+ (. r 0) ((. List len) (. r 1))))) (export main)))
  (output (: 44 Int64)))

(case "an effect-built NESTED-compound value escapes the handle and a nested projection reads through it"
  (doc    "The nested-compound escape: the handle's VALUE is a TUPLE OF TUPLES built from performed reads,
           it escapes to an enclosing `let`, and a NESTED projection `(. (. r 0) 0/1)` reads through the
           outer then inner aggregate — the effect-produced companion of the plain nested-projection escape,
           and a memory-safety pin for the aggregate-projection-that-escapes path. `Idx` seeded 10, arm
           `(resume s (+ s 1))`: the inner tuple `(tuple (Idx.next) (Idx.next))` reads 10 then 11 = `(10,
           11)`, the outer third read is 12, so `r = ((10, 11), 12)`; the nested projection `(. (. r 0) 1)`
           reads the inner tuple's second field, 11. Pins that a nested compound the handle builds from
           performed values escapes intact AND a projection reaching THROUGH the outer aggregate into a
           nested one is correctly reference-managed (no use-after-free / double-free when the nested field
           outlives its parent aggregate) — the effects × nested-projection-escape composition.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (main)
              (let ((r (handle Idx 10 ((next (u) s (resume s (+ s 1))))
                         (tuple (tuple (Idx.next) (Idx.next)) (Idx.next)))))
                (. (. r 0) 1))) (export main)))
  (output (: 11 Int64)))

; A RECURSIVE effectful walk whose handler arm resumes WITH THE STATE ITSELF and threads a CHANGED state
; `(resume s (+ s 1))` — the exact combination (recursion × a state-threading arm whose resume VALUE is
; the state) that leaked a compiler-internal specialization name. The recursive-def specialization
; synthesizes a state-threading copy with a trailing `$s{k}` state param; the arm's resume value (`s`,
; substituted with a reference to that state param) was extracted straight off the discarded `resume`
; node, so its parent chain did not reach the specialized def — the reference resolved UNBOUND, surfacing
; the internal `walk#eff2$s0` name as a CDZ0101. Copying the extracted resume value/next-state (a
; re-parenting copy) attaches them to the threaded body, so the state-param reference resolves. Each
; factor alone already worked (the list-accumulator case above threads `(resume unit …)`; a non-recursive
; `(+ (Tick.tick) (Tick.tick))` threads fine; a recursive walk with a CONSTANT resume state compiles), so
; this pins their intersection.

(case "a recursive effectful walk under a state-threading handler compiles without leaking an internal name"
  (doc    "`(def (walk (: n Int64)) (if (< n 1) 0 (do (Tick.tick) (walk (- n 1)))))` performs `Tick.tick`
           at each of n recursive steps, under a handler that resumes with the state and threads a changed
           one `(resume s (+ s 1))`. The walk returns the base `0` (the ticks thread state but the value is
           the base). This must compile and run to 0 — the recursive counterpart of the non-recursive
           state-threading case and of the recursive constant-state case, which both work. The E3/E4
           specialization must not leak its internal `walk#eff{n}$s{k}` state-param name as an unbound-name
           error: the recursive self-call's threaded state, and the arm's resume value that references the
           state param, must resolve against the synthesized specialization, not dangle.")
  (input  (do
            (effect Tick (op tick (-> Unit Int64)))
            (def (walk (: n Int64)) (if (< n 1) 0 (do (Tick.tick) (walk (- n 1)))))
            (def (main)
              (handle Tick 0 ((tick (u) s (resume s (+ s 1)))) (walk 3))) (export main)))
  (call   main)
  (output (: 0 Int64)))

(case "two effects each declaring a same-named operation do not collide"
  (doc    "Witnesses capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
           Operations (2nd sentence): `Unify` and `Scope` each declare a `resolve` operation, reached as
           `Unify.resolve` and `Scope.resolve`; the qualified names disambiguate. A `handle` discharges
           exactly ONE effect — its head names that effect and every arm is one of that effect's
           operations — so discharging both effects is two NESTED handles: an outer `(handle Scope …)`
           and an inner `(handle Unify …)`, each binding its own effect's `resolve`. The body performs
           `Unify.resolve`, discharged by the inner handler and resumed with 5; `Scope` is installed but
           never performed. Both handlers are stateless (seed `unit`). Pins that an operation is reached
           through its declaring effect and a shared operation name is collision-free — the two `resolve`
           arms live under distinct handlers keyed to distinct effects.")
  (input  (do
            (effect Unify (op resolve (-> Int64 Int64)))
            (effect Scope (op resolve (-> Int64 Int64)))
            (def (main)
              (handle Scope unit ((resolve (x) s (resume x s)))
                (handle Unify unit ((resolve (x) s (resume (+ x 1) s)))
                  (Unify.resolve 4)))) (export main)))
  (output (: 5 Int64))
  (host-calls))

(case "an effect operation may be named `bind` — the interop directive keyword is not reserved for op names"
  (doc    "`bind` is the head of the top-level peer-binding DIRECTIVE `(bind Effect \"cadenza:pkg/iface\")`,
           but that keyword is reserved only at the top level — an effect operation, like any member, may
           be named `bind`. `(effect Scope (op bind (-> Int64 Int64)) (op depth (-> Unit Int64)))` declares
           a `bind` operation whose handler arm is the NESTED list `(bind (v) d (resume (+ v d) (+ d 1)))`.
           Seeded 0: `(Scope.bind 10)` reads d=0 → `10 + 0` = 10 (state → 1), `(Scope.bind 20)` reads d=1 →
           `20 + 1` = 21 (state → 2), `(Scope.depth)` reads 2, so `(+ 10 (+ 21 2))` = 33. Pins that the
           malformed-`(bind …)` diagnostic scopes to TOP-LEVEL directives only: an arena-wide scan misreads
           the arity-3 handler arm as a malformed peer-binding (wrong arity) and rejects the program with a
           spurious CDZ0201 — a false positive on a legal operation name, fixed by scoping the scan to
           top-level `(bind …)` forms.")
  (input  (do
            (effect Scope (op bind (-> Int64 Int64)) (op depth (-> Unit Int64)))
            (def (main)
              (handle Scope 0 ((bind (v) d (resume (+ v d) (+ d 1))) (depth (u) d (resume d d)))
                (let ((a (Scope.bind 10))) (let ((b (Scope.bind 20))) (+ a (+ b (Scope.depth))))))) (export main)))
  (output (: 33 Int64)))

; The dual of the collision-free cross-effect case: WITHIN one effect, an operation name declared TWICE
; is ill-formed. capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
; Operations: an effect declaration "binds each of its operations to an operation type, so that the set of
; operations an effect offers is a CLOSED, statically-known SET rather than an open collection of ad-hoc
; names." Two `(op f …)` in one effect bind the name `f` twice — the set is then not well-defined (which
; operation type governs a performance of `E.f`?), the same ill-formedness a record with a duplicate field
; (`(record (a 1) (a 2))`) and a module with a duplicate definition (`(module … (def (f) 1) (def (f) 2))`)
; are rejected for (CDZ0201): a fixed/closed set cannot name the same member twice. The effect MUST be
; rejected, not resolved by keeping one `f` and silently discarding the other. A compiler that registers
; each operation into the effect's table without checking for a name already bound silently keeps one and
; accepts the declaration — the effect-declaration sibling of the record-field and module-definition
; duplicate gaps. (Distinct from the cross-effect case above, where `Unify.resolve` and `Scope.resolve` are
; two operations of two effects, disambiguated by their effect — collision-free per the spec's 2nd
; sentence. Here it is one effect naming one operation twice.) A generation that does not yet check for a
; duplicate operation name declines rather than silently choosing one.

(case "an effect that declares an operation name twice is rejected"
  (doc    "`(effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64)))` declares the operation `f` twice —
           but an effect's operations are a CLOSED, statically-known SET, each name bound to one operation
           type (capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
           Operations). Binding `f` twice makes the set ill-defined, the same ill-formedness a record with a
           duplicate field name or a module with a duplicate definition is rejected for (CDZ0201) — a fixed
           set cannot name the same member twice. The effect MUST be rejected, not resolved by keeping one
           `f` and discarding the other. Pins that the duplicate-member check reaches an effect's operation
           set, the effect-declaration sibling of the record-field (`(record (a 1) (a 2))`) and module-
           definition (`(module … (def (f) 1) (def (f) 2))`) duplicate cases; distinct from the collision-
           free cross-effect case above (`Unify.resolve` / `Scope.resolve`), which is two effects' distinct
           operations. A generation that does not yet detect a duplicate operation name declines rather
           than silently choosing one.")
  (input  (do
            (effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64)))
            (def (main) 1) (export main)))
  (error  CDZ0201))

; --- Handler resolution is dynamic in extent, across function boundaries ------------------------
; capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically Determined. The cases
; above perform and handle inside one `main`, where dynamic and lexical resolution coincide. These cases
; SEPARATE them: the perform is in a callee and the handler is in a caller, so resolution MUST follow the
; call chain, not the performing function's definition site. Each of these would be an ungranted-effect
; rejection (CDZ0401) under definition-site (lexical) resolution — the performing function is defined at
; top level with no handler in scope — so a defined output is itself the witness that resolution is dynamic.
; Which handler discharges each performance is nonetheless fixed statically (by monomorphizing the handler
; context), preserving determinism (constitution III).

(case "an effect performed in a callee is discharged by the caller's handler"
  (doc    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined: `gen` performs `(Bump.by 41)` but installs no handler; `main` handles `Bump` around
           its CALL to `gen`. Resolution follows the call chain, so the perform in `gen` is discharged by
           `main`'s handler and the run computes 42. Under definition-site (lexical) resolution `gen` has no
           `Bump` handler in scope and the effect would be ungranted (CDZ0401) — the defined output 42 is
           the witness that a function may perform an operation its CALLER discharges. The handler is
           stateless (seed `unit`).")
  (input  (do
            (effect Bump (op by (-> Int64 Int64)))
            (def (gen) (Bump.by 41))
            (def (main)
              (handle Bump unit ((by (n) s (resume (+ n 1) s))) (gen))) (export main)))
  (output (: 42 Int64))
  (host-calls))

(case "an effect resolves past an intermediate frame that installs no handler"
  (doc    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined: the call chain is `main` (handles `Ping`) -> `mid` (no handler) -> `leaf`
           (performs `Ping.ping`). The perform in `leaf` searches OUTWARD along the call chain, past `mid`
           which installs no handler, to `main`'s handler, which resumes with 5; `mid` then computes
           `(+ 5 100)` = 105. An intermediate function that installs no handler is transparent to
           resolution — it is merely a frame on the chain. The handler is stateless.")
  (input  (do
            (effect Ping (op ping (-> Unit Int64)))
            (def (leaf) (Ping.ping))
            (def (mid)  (+ (leaf) 100))
            (def (main)
              (handle Ping unit ((ping () s (resume 5 s))) (mid))) (export main)))
  (output (: 105 Int64))
  (host-calls))

(case "a nearer handler on the call chain shadows an outer one"
  (doc    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined and #A Handler May Interpose On An Effect (a handler nearer the perform wins): the
           call chain is `main` (handler *100) -> `mid` (handler *10) -> `leaf` (performs `Mul.by 1`). The
           NEAREST active handler on the chain is `mid`'s, so `(Mul.by 1)` resolves to `(* 1 10)` = 10 and
           `main`'s outer *100 handler is never reached (the inner arm does not re-perform, so it does not
           forward). The result is 10, not 1000 — pinning that the nearest DYNAMIC handler discharges the
           operation and shadows the outer one. Both handlers are stateless.")
  (input  (do
            (effect Mul (op by (-> Int64 Int64)))
            (def (leaf) (Mul.by 1))
            (def (mid)  (handle Mul unit ((by (x) s (resume (* x 10) s))) (leaf)))
            (def (main) (handle Mul unit ((by (x) s (resume (* x 100) s))) (mid))) (export main)))
  (output (: 10 Int64))
  (host-calls))

(case "two LEXICALLY-NESTED handlers of the same effect partition the performs by region"
  (doc    "The lexical-nesting companion of the call-chain shadow above: two handlers of the SAME effect `E`
           nest in ONE expression, and TWO performs are partitioned by which handler's region they sit in. `(+
           (handle E 5 … (E.get)) (E.get))`: the FIRST `(E.get)` is inside the inner `handle E 5`, so it
           resolves to the inner seed 5; the SECOND `(E.get)` is OUTSIDE the inner handle (a sibling operand
           of the `+`), so it escapes the inner region and reaches the OUTER `handle E 100`, resolving to 100.
           Both arms resume with the state unchanged (`(get (u) s (resume s s))`), so `(+ 5 100)` = 105. Pins
           that lexical handler nesting of the same effect partitions performs by REGION — the inner handle
           discharges only the performs textually within its body, and a perform outside it reaches the next
           enclosing handler (distinct from the call-chain case, where the whole callee runs under the nearer
           handler). Both backends agree.")
  (input  (do
            (effect E (op get (-> Unit Int64)))
            (def (main)
              (handle E 100 ((get (u) s (resume s s)))
                (+ (handle E 5 ((get (u) s (resume s s))) (E.get)) (E.get)))) (export main)))
  (output (: 105 Int64)))

(case "the same function called under two handlers is discharged by each in turn"
  (doc    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined (the monomorphization property): a single function `ask` = `(+ (Get.get) 1)` is
           called under two DIFFERENT `Get` handlers — one resuming 10, one resuming 20. The first call
           yields `(+ 10 1)` = 11, the second `(+ 20 1)` = 21, and `main` sums them to 32. The same
           definition is discharged by whichever handler is active on the call chain at each call site, so
           a self-hosting compiler specializes (monomorphizes) `ask` once per handler context it is called
           under — the effect is an implicit parameter threaded from the caller that installed the handler.
           Under definition-site resolution `ask` has no `Get` handler in scope and both calls would be
           ungranted (CDZ0401); the defined output 32 is the witness for dynamic resolution. Both handlers
           are stateless.")
  (input  (do
            (effect Get (op get (-> Unit Int64)))
            (def (ask) (+ (Get.get) 1))
            (def (main)
              (+ (handle Get unit ((get () s (resume 10 s))) (ask))
                 (handle Get unit ((get () s (resume 20 s))) (ask)))) (export main)))
  (output (: 32 Int64))
  (host-calls))

(case "an effect resolves through a deep chain of intermediate functions"
  (doc    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined at depth: the chain is `main` (handles `Ask`) -> `a` -> `b` -> `c` -> `d`
           (performs `Ask.ask`), each of `a`/`b`/`c` adding 1 to its callee's result and installing no
           handler. The perform in `d` resolves past three intermediate frames to `main`'s handler, which
           resumes with 7; the +1s then compose back up the chain: d=7, c=8, b=9, a=10.
           Pins that dynamic resolution reaches an arbitrarily deep enclosing handler and that the
           intermediate frames are transparent. The handler is stateless.")
  (input  (do
            (effect Ask (op ask (-> Unit Int64)))
            (def (d) (Ask.ask))
            (def (c) (+ (d) 1))
            (def (b) (+ (c) 1))
            (def (a) (+ (b) 1))
            (def (main)
              (handle Ask unit ((ask () s (resume 7 s))) (a))) (export main)))
  (output (: 10 Int64))
  (host-calls))

(case "a stateful handler threads its counter across a function boundary"
  (doc    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges composed with #Handler Resolution Is Dynamic In Extent: the `Fresh` counter, seeded
           0 in `main`, is folded across performs that happen in a CALLEE. `label` performs `(Fresh.next)`;
           `pair-of` calls `label` twice to build `(tuple (label) (label))`. The handler discharges both
           performs — reached dynamically through `pair-of` and `label` — threading the counter across the
           function boundary: the first `label` sees 0, the second sees 1, giving `(tuple 0 1)`. Pins that
           the folded state is not a lexical-scope construct but a dynamic-extent one that persists across
           calls, exactly as the compiler's fresh-name supply must. The handle evaluates to the body's
           tuple; the final counter 2 is discarded.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (label)   (Fresh.next))
            (def (pair-of) (tuple (label) (label)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (pair-of))) (export main)))
  (output (: (tuple 0 1) (Tuple Int64 Int64))))

; --- A recursive function drives an effect (the state-machine idiom) --------------------------
; capabilities-and-effects.md #A Handler Threads State Across The Operations It Discharges composed
; with #Handler Resolution Is Dynamic In Extent, at the point a function RECURSES while performing.
; These are the shape a self-hosting compiler actually has — a recursive walk (over an AST, a token
; stream) that performs an effect (fresh name, diagnostic, unification) on each step. A recursive
; effectful function CANNOT be discharged by inlining it into the handled region (its body would
; inline without bound); it needs effect-context monomorphization — the function emitted once as a
; real wasm function that reads the discharging handler as an implicit evidence parameter
; (options/effects-model/lowering-to-wasm.md §Effect-context monomorphization, §Stage 3). A
; generation that resolves cross-function effects only by inlining DECLINES these (an honest todo,
; never a hang or a miscompile — reject-don't-miscompile); the recorded output is the semantics a
; monomorphizing generation realizes.

(case "a recursive function counts down through a stateful effect and bails at zero"
  (doc    "Witnesses the recursive-effect idiom: `loop` performs `(Countdown.tick)` and recurses
           until the tick reads 0. The handler is seeded with 3 and its arm
           `(Countdown.tick (u) s (resume s (- s 1)))` hands back the current counter and threads
           `s - 1` forward, so successive ticks read 3, 2, 1, 0. `loop` adds 1 for each non-zero
           tick and returns 0 at the zero tick: the four ticks (3,2,1,0) yield `1 + 1 + 1 + 0` = 3.
           The counter is folded across a RECURSIVE call chain (dynamic extent), exactly as a
           compiler's fresh-name/position counter is folded across a recursive AST walk. `loop`
           recurses while performing, so it cannot be inlined into the handle (non-terminating);
           discharging it needs effect-context monomorphization — until a generation realizes that,
           the compiler declines rather than inlines (reject-don't-miscompile). The recorded output
           3 is the semantics a monomorphizing generation produces.")
  (input  (do
            (effect Countdown (op tick (-> Unit Int64)))
            (def (loop)
              (if (= (Countdown.tick) 0)
                  0
                  (+ 1 (loop))))
            (def (main)
              (handle Countdown 3 ((tick (u) s (resume s (- s 1)))) (loop))) (export main)))
  (output (: 3 Int64)))

(case "a self-recursive effectful loop sums a fresh-id draw per step — the gensym idiom"
  (doc    "The compiler-ml port's fresh-id generator shape (`implementation/compiler-ml/src/fresh.cdz`, the
           self-host's first use of the effect system): `id-sum n = if n = 0 then 0 else (Fresh.next) +
           id-sum(n - 1)` draws one fresh id per recursion and sums them. The perform `(Fresh.next)` is the
           LEFT operand of the `+` and the self-call the RIGHT — a strict spine where the perform is
           evaluated BEFORE the self-call, so it reads the PRE-recursion (incoming) state, which the
           single-return effect-context specialization threads correctly. Seeded 0, the ids drawn are 0, 1,
           2, so `id-sum 3` = `0 + (1 + (2 + 0))` = 3. Pins the gensym idiom the self-hosted compiler uses to
           thread unique type-variable / name ids without a hand-plumbed counter. (Contrast the SELF-CALL-
           before-perform shape — two sibling recursive calls whose second reads the first's OUT-state —
           which the single-return spec cannot thread and declines cleanly, pending the multi-value-return
           increment.)")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (id-sum (: n Int64)) (if (= n 0) 0 (+ (Fresh.next) (id-sum (- n 1)))))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1)))) (id-sum 3))) (export main)))
  (output (: 3 Int64)))

(case "a MUTUALLY-recursive effectful group is specialized under a state handler"
  (doc    "Effect-context monomorphization extends past a SINGLE self-recursive function to a MUTUALLY-
           recursive group. `ev` and `od` call each other, and the effect `Ctr.tick` is reached by `ev`
           only THROUGH its partner `od` — so detecting that `ev` reaches the effect requires following the
           RECURSIVE partner call, and specializing it requires tying the two specializations' knot (each
           partner's recursive call resolves to the other's specialized copy). Seeded 7, `tick` hands back
           the counter and threads `s - 1`: `ev(4)`→`od(3)` reads 7, `ev(2)`→`od(1)` reads 6, `ev(0)`=0, so
           the sum is `7 + (6 + 0)` = 13. Recursive-while-performing across a MUTUAL cycle — the same
           dynamic-extent state fold as the single-recursion countdown, over a call graph rather than a
           single self-call.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (ev (: n Int64)) (if (= n 0) 0 (od (- n 1))))
            (def (od (: n Int64)) (+ (Ctr.tick) (ev (- n 1))))
            (def (main)
              (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) (ev 4))) (export main)))
  (output (: 13 Int64)))

(case "a mutually-recursive group performs in one branch and recurses in the OTHER"
  (doc    "The SPLIT-BRANCH mutual-recursion shape: unlike the case above (where the perform `(Ctr.tick)`
           and the mutual call `(ev …)` sit in the SAME strict expression `(+ (Ctr.tick) (ev …))`), here
           the perform is in a cycle def's BASE-CASE branch and the mutual call is in its RECURSIVE branch —
           `(def (ev n) (if (= n 0) (Fresh.next) (od (- n 1))))` with `(od n) = (if (= n 0) 0 (ev (- n 1)))`.
           Detecting that `ev` reaches `Fresh` still requires following the recursive partner, and the two
           specializations' knot must tie even though each def's perform and mutual call are in DIFFERENT
           branches (the branch-distributed state threading + cross-def memo knot). Seeded 0, `next` resumes
           `s + 1`: `(ev 2)` chains `ev2→od1→ev0`, and `ev0` hits its BASE branch `(Fresh.next)` which
           resumes the seed `0 + 1` = 1 — so the result is 1, a NON-ZERO value that witnesses the perform
           in the separate base-case branch actually fired (an odd start `(ev 3)`→`ev3→od2→ev1→od0` = 0
           never reaches it). This is the fresh-name / gensym shape an effectful AST-walking compiler pass
           needs (`relabel(node)` ↔ `relabel-list(children)`, the counter threaded as a `Fresh` effect
           rather than an explicit parameter). Pins that the mutual specialization ties the knot across the
           separate-branch case, not only the same-branch one. (This shape was previously a clean decline
           pending the fold work; it now specializes correctly.)")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
            (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
            (def (main)
              (handle Fresh 0 ((next () s (resume (+ s 1) s))) (ev 2))) (export main)))
  (output (: 1 Int64)))

(case "a mutually-recursive group performs through a shared non-recursive helper"
  (doc    "Composes the two cross-function triggers: a mutually-recursive group (`ev`/`od`) where the
           effect is performed inside a NON-recursive helper `h` that `od` calls, rather than syntactically
           in `od`'s own body. The helper INLINES (the non-recursive inline trigger) and the mutual pair
           SPECIALIZES (the recursive trigger), and they compose — `od`'s `(h)` is inlined to `(Ctr.tick)`
           within the specialized `od#ctx`. Seeded 7, threading `s - 1`, the ticks read 7 then 6, so `ev(4)`
           = `7 + (6 + 0)` = 13. Pins that specialization detecting the effect through a mutual partner and
           inlining a performing helper cooperate in one recursive group.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (h) (Ctr.tick))
            (def (ev (: n Int64)) (if (= n 0) 0 (od (- n 1))))
            (def (od (: n Int64)) (+ (h) (ev (- n 1))))
            (def (main)
              (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) (ev 4))) (export main)))
  (output (: 13 Int64)))

(case "a mutually-recursive group performs in its entry def while its partner only dispatches"
  (doc    "The MIRROR of the case above, and the one that pins the mutual-group scheme fixpoint. Here the
           ENTRY def `ev` (the one the handle body calls, so its scheme is demanded FIRST) is the one that
           PERFORMS — it recurses through its partner `od`, which is a PURE DISPATCHER whose body is
           ENTIRELY the sibling call `(ev (- n 1))`. Computing `ev`'s scheme demands `od`'s mid-flight,
           while `ev`'s own signature is still provisional; `od`'s body — being only `(ev …)` — then reads
           that provisional `ev` and would type as an undetermined `Any`. The mutual-group scheme solve
           must NOT cache that provisional `None` for `od` (else the dispatcher is poisoned permanently and
           the whole group declines); once `ev` resolves via its base case, a re-demand computes `od`'s
           true `Int64 -> Int64`. Seeded 7, threading `s - 1`, the ticks read 7 then 6, so `ev(4)` =
           `(Ctr.tick) + od(3)` = `7 + ev(2)` = `7 + ((Ctr.tick) + od(1))` = `7 + (6 + ev(0))` =
           `7 + 6 + 0` = 13. Recursive-while-performing, so it needs effect-context specialization
           (`DESIGN-effects-rcdzc.md` §4.2, §4.3).")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (ev (: n Int64)) (if (= n 0) 0 (+ (Ctr.tick) (od (- n 1)))))
            (def (od (: n Int64)) (ev (- n 1)))
            (def (main)
              (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) (ev 4))) (export main)))
  (output (: 13 Int64)))

(case "a mutually-recursive group with the perform in a DIFFERENT branch from the mutual call folds"
  (doc    "The mutual-group shape where the perform and the mutual call sit in SEPARATE branches of a
           conditional — distinct from the cases above where they share one strict expression `(+ (Ctr.tick)
           (od …))`. `ev`'s base-case branch performs `(Fresh.next)` while its recursive branch calls the
           partner `(od …)`: `(def (ev n) (if (= n 0) (Fresh.next) (od (- n 1))))`, `(def (od n) (if (= n 0)
           0 (ev (- n 1))))`. Under the state-threading handler this recurses `ev 2 -> od 1 -> ev 0`, where
           the base case fires `Fresh.next`: seeded 42, the arm resumes `s + 1` = 43. Pins that effect-context
           specialization ties the `ev#ctx`/`od#ctx` memo knot even when each branch of a cycle def EMBEDS
           the threaded state independently (the performing branch substitutes it into the resume value, the
           mutual-call branch appends it as a trailing state argument) — each branch needs its own copy of
           the state reference, not a shared one. Was a compile-time leak of the internal `ev#eff…$s0`
           specialization name (a `cdz check`-clean / `compile`-fail gap); now folds to 43.")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
            (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
            (def (main)
              (handle Fresh 42 ((next () s (resume (+ s 1) s))) (ev 2))) (export main)))
  (output (: 43 Int64)))

(case "a MATCH-dispatched mutual group with the perform in one arm and the mutual call in another folds"
  (doc    "The `match` companion of the separate-branch mutual case above — the cycle dispatches on a
           `match` rather than an `if`, with the perform in one arm and the mutual call in another. `(def (ev
           n) (match n (0 (Fresh.next)) (_ (od (- n 1)))))`, `(def (od n) (match n (0 0) (_ (ev (- n 1)))))`.
           Same recursion `ev 2 -> od 1 -> ev 0`, the `0` arm fires `Fresh.next`: seeded 42, the arm resumes
           `s + 1` = 43. Pins that each MATCH ARM (like each `if` branch) gets its own copy of the threaded
           state reference — the performing arm substitutes it into the resume value while the mutual-call arm
           appends it as a trailing state argument, and a single-parent arena would otherwise orphan a shared
           state-ref node, leaking the internal `ev#eff…$s0` name. Was the match-dispatch analogue of the
           if-branch leak; now folds to 43.")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (ev (: n Int64)) (match n (0 (Fresh.next)) (_ (od (- n 1)))))
            (def (od (: n Int64)) (match n (0 0) (_ (ev (- n 1)))))
            (def (main)
              (handle Fresh 42 ((next () s (resume (+ s 1) s))) (ev 2))) (export main)))
  (output (: 43 Int64)))

(case "a mutually-recursive fresh-id walk assigns a fresh id at each node and sums them"
  (doc    "The essential compiler-PASS idiom the mutual-effect fixes unblock: a `Fresh` gensym threaded
           through a MUTUALLY-recursive walk over a tree — `node` visits a node (assigns it `(Fresh.next)`)
           and recurses into its `children`, which recurse back into `node`. This is exactly the shape an
           AST-relabelling pass takes (`relabel(node)` ↔ `relabel-list(children)`), with the fresh-id counter
           threaded by the handler rather than passed as an explicit parameter. `Fresh` seeded 0, arm `(next
           () s (resume s (+ s 1)))` hands back `s` and threads `s + 1`. `(node 5)` visits the node chain
           `node 5 -> children 4 -> node 3 -> children 2 -> node 1 -> children 0`, firing `Fresh.next` at
           each `node` step (n = 5, 3, 1) — reading 0, 1, 2 — and summing them along the way: `node 1` =
           `2 + 0` = 2, `node 3` = `1 + 2` = 3, `node 5` = `0 + 3` = 3. Pins that effect-context
           specialization threads a fresh-name generator through a mutual tree walk end to end — the pass a
           self-hosting compiler runs over its own AST (each perform reads the PRE-recursion state, the
           sound pre-order shape). Both backends agree.")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (node (: n Int64)) (if (= n 0) (Fresh.next) (+ (Fresh.next) (children (- n 1)))))
            (def (children (: n Int64)) (if (= n 0) 0 (node (- n 1))))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1)))) (node 5))) (export main)))
  (output (: 3 Int64)))

(case "a mutual walk where BOTH partners perform threads one shared counter across the cycle"
  (doc    "The fresh-id-walk case above reaches the effect through ONE partner (`node` performs, `children`
           only dispatches). Here BOTH cycle defs perform the same effect — each reads a fresh id before
           recursing into the other — so the shared handler counter is advanced by BOTH specializations
           (`node#ctx` AND `children#ctx`), and the reads interleave along the cycle. `Fresh` seeded 0, arm
           `(next () s (resume s (+ s 1)))`. `(node 3)`: `node` reads id 0 then `+ children 2`; `children 2`
           reads id 1 then `+ node 1`; `node 1` reads id 2 then `+ children 0`; `children 0` = 0. So `node 1`
           = `2 + 0` = 2, `children 2` = `1 + 2` = 3, `node 3` = `0 + 3` = 3. Pins that effect-context
           specialization threads ONE shared state slot correctly when BOTH members of a mutual group
           perform (not only when the effect is reached through a single partner) — each partner's
           specialization carries the threaded counter and the interleaved reads advance it in cycle order.
           Both backends agree.")
  (input  (do
            (effect Fresh (op next (-> Int64)))
            (def (node (: n Int64))
              (if (= n 0) (Fresh.next) (let ((v (Fresh.next))) (+ v (children (- n 1))))))
            (def (children (: n Int64))
              (if (= n 0) 0 (let ((w (Fresh.next))) (+ w (node (- n 1))))))
            (def (main)
              (handle Fresh 0 ((next () s (resume s (+ s 1)))) (node 3))) (export main)))
  (output (: 3 Int64)))

(case "a recursive function sums a range it walks by performing a fresh-index effect"
  (doc    "Witnesses the recursive-effect idiom folding a real accumulator across a self-recursive
           walk: `Idx` supplies a descending index (seeded 3, each `next` hands back `s` and threads
           `s - 1`), and `sum-down` recurses — performing `(Idx.next)` once per step and adding it
           to the sum of the rest — until the index reaches 0. The performs read 3, 2, 1, 0, so the
           walk computes `3 + 2 + 1 + 0` = 6. This is a self-recursive consumer driven entirely by a
           stateful effect (the counter is not a parameter — it is threaded by the handler across the
           recursion), the essential shape of a compiler pass that walks a structure while pulling
           fresh state. Being recursive-while-performing, it declines under inlining-only resolution
           and needs effect-context monomorphization; the recorded output 6 is the realized
           semantics.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (sum-down)
              (let ((i (Idx.next)))
                (if (= i 0)
                    0
                    (+ i (sum-down)))))
            (def (main)
              (handle Idx 3 ((next (u) s (resume s (- s 1)))) (sum-down))) (export main)))
  (output (: 6 Int64)))

(case "a recursive function with an annotated parameter walks and bails through an abortive handler"
  (doc    "The recursive-effect idiom with an ANNOTATED parameter and an ABORTIVE discharge. `walk` takes
           `(: n Int64)` and tail-recurses, counting `n` down; at zero it performs `(Bail.bail 99)`, whose
           handler arm never resumes — so the abort at the base ABANDONS the walk and its value 99 becomes
           the handle's value (propagating up the tail calls, no state threaded). Witnesses that recursive
           effect-context specialization handles an annotated parameter (not only a bare name) — the
           synthesized specialized function re-annotates the parameter with its solved type. `(walk 3)`
           ticks 3→2→1→0 then bails → 99 (`DESIGN-effects-rcdzc.md` §4.2, §4.3).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (walk (: n Int64))
              (if (= n 0) (Bail.bail 99) (walk (- n 1))))
            (def (main)
              (handle Bail 0 ((bail (n) s n)) (walk 3))) (export main)))
  (output (: 99 Int64)))

(case "a recursive function threads two nested handlers' states at once"
  (doc    "Witnesses that effect-context resolution threads EACH enclosing handler's state
           independently across a recursion (capabilities-and-effects.md #A Handler Threads State
           Across The Operations It Discharges composed with #Handler Resolution Is Dynamic In
           Extent): `loop` recurses under TWO nested stateful handlers — `A` (a countdown seeded 3,
           `tick` hands back `s` and threads `s - 1`) governs the recursion depth, and `B` (an
           accumulator seeded 0, `bump` hands back `s` and threads `s + 10`) is folded across the
           steps. Each non-zero tick performs `B.bump` and adds it to the recursion's tail: the ticks
           read 3, 2, 1, 0 (three non-zero), and the bumps read 0, 10, 20, so the sum is
           `0 + 10 + 20 + 0` = 30. Both states are live on the call stack SIMULTANEOUSLY — the
           mechanism must give each handler context its own threaded state (a single shared slot per
           effect would clobber when the recursion re-enters), which is exactly what threading each
           context as a distinct hidden parameter/return provides. This is the essential shape of a
           self-hosting compiler pass that walks a structure while folding more than one piece of
           state (a fresh-name counter AND a diagnostics list). Recursive-while-performing, so it
           needs effect-context monomorphization; the recorded output 30 is the realized semantics.")
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op bump (-> Unit Int64)))
            (def (loop)
              (if (= (A.tick) 0)
                  0
                  (+ (B.bump) (loop))))
            (def (main)
              (handle B 0 ((bump (u) s (resume s (+ s 10)))) (handle A 3 ((tick (u) s (resume s (- s 1)))) (loop)))) (export main)))
  (output (: 30 Int64)))

(case "a recursive walk threads THREE nested handlers' states at once"
  (doc    "Generalizes the two-nested-handler case to THREE: one recursive `walk` performs `A.a`, `B.b`, and
           `C.c` at each step, under three nested stateful handlers, and each handler's state threads
           INDEPENDENTLY — the merged effect context carries THREE distinct slots (a shared per-effect slot
           would clobber on re-entry). Each handler hands back `s` and threads `s + 1`: seeded A=100, B=200,
           C=300, over `(walk 2)`, the `A.a` reads are 100, 101 (sum 201), the `B.b` reads 200, 201 (401),
           the `C.c` reads 300, 301 (601), so the total is `201 + 401 + 601` = 1203. Pins that effect-context
           monomorphization scales past two effects — N handlers over one recursive walk thread N distinct
           states — the shape of a self-hosting pass folding several pieces of context (a name counter, a
           diagnostics list, a symbol table) at once. Identical on both backends.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect C (op c (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (= n 0)
                  0
                  (+ (A.a) (+ (B.b) (+ (C.c) (walk (- n 1)))))))
            (def (main)
              (handle A 100 ((a (u) s (resume s (+ s 1))))
                (handle B 200 ((b (u) s (resume s (+ s 1))))
                  (handle C 300 ((c (u) s (resume s (+ s 1))))
                    (walk 2))))) (export main)))
  (output (: 1203 Int64)))

(case "a handler arm capturing an enclosing fn param folds under a multi-arm nested handler"
  (doc    "A handler arm may reference a name bound by an ENCLOSING function — here `converse`'s arm
           `(resume p 0)` captures `run-with`'s parameter `p`, NOT the arm's own params/state. When the
           recursive driver `run` performs BOTH effects (so the fold takes the two-nested-states MERGE path)
           AND the inner handler is MULTI-ARM (`Tools` declares `dispatch`+`done`), the captured `p` used to
           be LOST — the synthesized `run#ctx` carried the driver's params and the threaded states but not
           `p`, so the spliced free `p` re-resolved against `run#ctx`'s signature (which lacked it) and the
           whole program declined `CDZ0101 unbound name p` (a valid program falsely refused; found by the
           agent-harness dogfood). The fix threads a captured enclosing-fn param as an EXTRA specialized
           parameter (after the originals, before the trailing states), passed UNCHANGED at every call since
           it is constant across the recursion. `run-with(3)` seeds `run(3,0)`; each step adds
           `converse→p (=3)` and `dispatch→1`, over three steps: `(0+3+1)+(3+1)+(3+1)` threaded = 12. This is
           the shape of a self-hosting pass whose handler closes over a config parameter (a routing table, a
           fuel budget) while walking a structure under more than one effect.")
  (input  (do
            (effect Model (op converse (-> Int64 Int64)))
            (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0)
                  acc
                  (run (- fuel 1) (+ acc (+ (Model.converse fuel) (Tools.dispatch fuel))))))
            (def (run-with (: p Int64))
              (handle Model 0 ((converse (q) s (resume p 0)))
                (handle Tools 0 ((dispatch (a) s (resume 1 0)) (done (a) s (resume a 0)))
                  (run 3 0))))
            (def (main) (run-with 3)) (export main)))
  (output (: 12 Int64)))

(case "an inner handler's INIT state is computed by performing an enclosing effect"
  (doc    "The seed of an inner handler is itself a PERFORM of an OUTER effect — the two handlers compose
           through the init position, not just the body. `(handle Seed 0 ((s (u) t (resume 50 t))) (handle
           Ask (Seed.s) …))`: the inner `Ask` handler's INIT is `(Seed.s)`, discharged by the enclosing
           `Seed` handler to 50. So `Ask` is seeded 50, and `(Ask.get)` (its arm resumes the state) reads 50.
           Pins that a handler init is an ordinary strict expression the outer handler's fold threads — the
           inner handler's starting state can be COMPUTED by an effect, the shape of a pass whose scratch
           state is initialized from a queried piece of outer context.")
  (input  (do
            (effect Seed (op s (-> Unit Int64)))
            (effect Ask (op get (-> Unit Int64)))
            (def (main)
              (handle Seed 0 ((s (u) t (resume 50 t)))
                (handle Ask (Seed.s) ((get (u) st (resume st st)))
                  (Ask.get)))) (export main)))
  (output (: 50 Int64)))

(case "a mutually-recursive group threads two nested handlers' states at once"
  (doc    "The two-nested-handler state-threading of the case above, but over a MUTUALLY-RECURSIVE group
           rather than a single self-recursive `loop` — composing merge (two effects, two handler contexts)
           WITH mutual specialization (`ev`/`od`, each performing a DIFFERENT effect). `ev` performs `A.tick`
           and recurses through `od`; `od` performs `B.bump` and recurses through `ev`; both handler
           contexts must thread INDEPENDENTLY across the alternation. `A` is a countdown seeded 3 (`tick`
           hands back `s`, threads `s - 1`), `B` an accumulator seeded 0 (`bump` hands back `s`, threads
           `s + 10`). Along `ev(4) → od(3) → ev(2) → od(1) → ev(0)=0`, the A-ticks read 3 then 2 (in `ev`)
           and the B-bumps read 0 then 10 (in `od`), so the strict-spine sum is `3 + 0 + 2 + 10 + 0` = 15.
           Each specialized function (`ev#ctx`/`od#ctx`) must carry BOTH threaded states as distinct hidden
           slots — a shared per-effect slot would clobber when the mutual recursion re-enters — pinning
           that merge (`merged_nested_ctx`) and mutual-group specialization cooperate. Recursive-while-
           performing across two effects, so it needs effect-context monomorphization
           (`DESIGN-effects-rcdzc.md` §4.2, §4.3).")
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op bump (-> Unit Int64)))
            (def (ev (: n Int64)) (if (= n 0) 0 (+ (A.tick) (od (- n 1)))))
            (def (od (: n Int64)) (+ (B.bump) (ev (- n 1))))
            (def (main)
              (handle A 3 ((tick (u) s (resume s (- s 1))))
                (handle B 0 ((bump (u) s (resume s (+ s 10))))
                  (ev 4)))) (export main)))
  (output (: 15 Int64)))

(case "a non-tail-resumptive outer handler reduces a reducible inner handle before its own fold"
  (doc    "Nested handlers of DISTINCT effects where the OUTER handler's arm resumes NON-tail. The
           inside-out reduction reduces the inner handle only while THREADING the outer body — which
           requires the outer arm to be tail-resumptive. When the outer arm is non-tail, its delimited
           continuation is the E5 pure one-hole fold, which sees the whole inner `handle` as an opaque
           non-uniform continuation and would decline. Reducing the inner (tail-resumptive) handler FIRST
           discharges `B`: `(handle B 0 ((b (u) t (resume 20 t))) (+ (A.a) (B.b)))` folds `B.b` to its
           resume value 20 (B threads no observable effect and A.a is left untouched as B does not discharge
           it), leaving `(+ (A.a) 20)`. That is a single `A`-perform in a pure one-hole context `C = (+ □
           20)`, so the outer arm `(+ 1 (resume 10 s))` folds to `(+ 1 (+ 10 20))` = 31. Sound and
           frame-free: reducing the inner handler is the same reduction the threading path performs, only
           sequenced before the outer fold rather than during it.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main)
              (handle A 0 ((a (u) s (+ 1 (resume 10 s))))
                (handle B 0 ((b (u) t (resume 20 t))) (+ (A.a) (B.b)))))
            (export main)))
  (output (: 31 Int64)))

(case "a recursive function that installs a fresh handler on each call grows its context without bound"
  (doc    "Witnesses the LIMIT of effect-context monomorphization: `loop` wraps its own recursive
           call in a FRESH `(handle …)`, so each recursive call runs under a handler context one
           frame deeper than the last. The performed `Fresh.next` at the base resolves to the
           INNERMOST (most recent) handler — a shadowing that is well-defined operationally (the run
           counts a fresh supply seeded 100, reads 100 once, so the result is 100) — but there is no
           FINITE set of handler contexts to specialize the function against: the context grows by
           one frame per recursion. A compiler that discharges recursive effects by
           effect-context monomorphization (emitting the function once per handler context) cannot
           cover an unbounded family of contexts, so it DECLINES rather than looping forever building
           specializations (reject-don't-miscompile; the seed bounds the handler-context depth and
           declines past the bound — it must never overflow the compiler, on any target). A generation
           that reifies
           continuations as data (a general one-shot / scheduler tier) discharges this; the recorded
           output 100 is that semantics. This case guards against the compiler crashing on
           unbounded context growth.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (loop n)
              (handle Fresh 100 ((next (u) s (resume s (+ s 1)))) (if (= n 0)
                    (Fresh.next)
                    (loop (- n 1)))))
            (def (main)
              (loop 2)) (export main)))
  (output (: 100 Int64)))

; --- Rejections the routing model introduces ----------------------------------------------------
; An effect declaration is the CLOSED set of an effect's operations, so a handler arm for an operation
; the effect does not declare is rejected (CDZ0403), and an operation reached with neither an enclosing
; handler nor an enclosing entrypoint delegation — so it would escape ungranted — is rejected (CDZ0401,
; the single "no home" check that merges the former undischarged-intra and undeclared-host rejections).
; These are the compile-time checks that keep "no ambient authority" a property of the source
; (capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time Error, #A Handler Arm Names An
; Operation Its Effect Declares).

; Performing an operation is TYPED exactly as an ordinary function application: its arguments are checked
; against the operation's declared parameter types (capabilities-and-effects.md #Performing An Operation Is
; Typed And Contributes To The Row: "Performing an operation MUST check its arguments against the operation's
; declared parameter types … so that an effect operation is typed exactly as an ordinary function
; application is"). So performing `E.op` — declared `(-> Int64 Int64)` — on a Bool argument is a type
; mismatch, rejected (CDZ0203) exactly as an ordinary `(f true)` on an Int64-parameter `f` is. A compiler
; that lowers the perform without checking the argument against the declared parameter type MISCOMPILES: it
; feeds the Bool (or worse, a String) through the op's Int64 slot and produces a garbage value rather than
; rejecting — `(E.op "str")` returns a nonsense integer. A generation that does not yet type-check a perform's
; arguments declines rather than emitting the mistyped operation.

(case "performing an operation with an argument of the wrong type is a type error"
  (doc    "`E.op` is declared `(-> Int64 Int64)`, so performing `(E.op true)` supplies a Bool where the
           operation's parameter type is Int64 — a type mismatch the compiler MUST reject (CDZ0203,
           capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row: a
           perform's arguments are checked against the declared parameter types, exactly as an ordinary
           function application's are — `(f true)` on an Int64-parameter `f` is rejected the same way,
           CDZ0203). Pins that an effect operation's arguments are type-checked: a compiler that lowers the
           perform without checking feeds the Bool through the op's Int64 slot and produces a wrong value
           (and a String argument yields a garbage integer). A generation that does not yet check a
           perform's arguments declines rather than emitting the mistyped operation.")
  (input  (do
            (effect E (op op (-> Int64 Int64)))
            (def (main)
              (handle E unit ((op (n) s (resume n s))) (E.op true))) (export main)))
  (error  CDZ0203))

; The perform-argument check must fire for EVERY declared parameter type, not only Int64. An operation
; declared `(-> String Unit)` performed on an Int64 argument — `(E.emit 42)` — is the same type mismatch
; as the Int64-parameter case above and MUST be rejected (CDZ0203). This is the STRING-parameter sibling:
; a compiler whose perform lowering dispatches on the DECLARED parameter type (routing a String-parameter
; op to a string-argument path) before checking the ARGUMENT's actual type skips the check when the
; declared parameter is String, and feeds the Int through the op's String slot — the handler arm binds `s`
; to `42` typed as a String, so `(E.emit 42)` runs to `unit` (and a downstream `(String.byte-len s)` in
; the arm reads a non-String value). The Int64-parameter op catches its bad argument (`(E.op true)` above);
; the String-parameter op must catch its bad argument identically, or the argument check is not "exactly as
; an ordinary function application" for every parameter type. A generation that does not yet check a
; String-parameter op's argument declines rather than binding the mistyped value into the handler arm.

(case "performing a string-parameter operation with a non-string argument is a type error"
  (doc    "`E.emit` is declared `(-> String Unit)`, so performing `(E.emit 42)` supplies an Int64 where the
           operation's parameter type is String — a type mismatch the compiler MUST reject (CDZ0203,
           capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row),
           exactly as the Int64-parameter case `(E.op true)` above is. Pins that the perform-argument check
           fires for a STRING-declared parameter too, not only Int64: a compiler that dispatches a perform
           on the declared parameter type — routing a String-parameter op to a string-argument path —
           before checking the argument's actual type skips the check for a String parameter and binds the
           Int `42` into the handler arm as a String, so `(E.emit 42)` runs to `unit` instead of being
           rejected. The argument check must be uniform across parameter types. A generation that does not
           yet check a String-parameter op's argument declines rather than binding the mistyped value.")
  (input  (do
            (effect E (op emit (-> String Unit)))
            (def (main)
              (handle E unit ((emit (s) st (resume unit st))) (E.emit 42))) (export main)))
  (error  CDZ0203))

; The perform-argument check must also fire for a COMPOUND declared parameter type, not only the scalar
; types Int64 (above) and String (above). An operation declared `(-> (List Int64) Unit)` performed on an
; Int64 argument — `(E.put 42)` — is the same type mismatch and MUST be rejected (CDZ0203). This is the
; COMPOUND-parameter sibling of the two scalar-parameter cases: a compiler whose perform check compares the
; argument only against a scalar Kind skips the check when the declared parameter is a compound, binds the
; Int `42` into the handler arm typed as a `List Int64`, and `(E.put 42)` runs to `unit` (a downstream
; `(List.len xs)` in the arm then reads a non-list value). A tuple argument where a list is declared, or a
; wrong element type, slips through the same way. The argument check must be uniform across ALL parameter
; type shapes — scalar and compound alike. A generation that does not yet check a compound-parameter op's
; argument declines rather than binding the mistyped value into the handler arm.

(case "performing an operation with a wrong-type argument for a compound parameter is a type error"
  (doc    "`E.put` is declared `(-> (List Int64) Unit)`, so performing `(E.put 42)` supplies an Int64 where
           the operation's parameter type is the compound `List Int64` — a type mismatch the compiler MUST
           reject (CDZ0203, capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To
           The Row), exactly as the Int64-parameter (`(E.op true)`) and String-parameter (`(E.emit 42)`)
           cases above are. Pins that the perform-argument check fires for a COMPOUND declared parameter
           too, not only scalars: a compiler that compares the argument only against a scalar Kind skips the
           check for a compound parameter and binds the Int `42` into the handler arm typed as a `List
           Int64`, so `(E.put 42)` runs to `unit` (a downstream `(List.len xs)` reads a non-list value). The
           argument check must be uniform across all parameter type shapes. A generation that does not yet
           check a compound-parameter op's argument declines rather than binding the mistyped value.")
  (input  (do
            (effect E (op put (-> (List Int64) Unit)))
            (def (main)
              (handle E unit ((put (xs) s (resume unit s))) (E.put 42))) (export main)))
  (error  CDZ0203))

(case "an operation with a TUPLE parameter binds the compound and the arm projects it"
  (doc    "The positive companion: an operation whose declared PARAMETER is a compound `(Tuple Int64
           Int64)` binds the whole tuple to the arm's parameter, which the arm projects. `Add.sum : (->
           (Tuple Int64 Int64) Int64)`; performed as `(Add.sum (tuple 3 4))`, the arm binds `p` to the pair
           and resumes with `(+ (. p 0) (. p 1))` = 7. Pins that a compound OP parameter threads through the
           fold and is projectable in the arm (the type-position spelling is capital `Tuple`, the type
           constructor — lowercase `tuple` is the value constructor). NOTE: the arm here projects `p` from a
           pure `(tuple 3 4)` argument; when the tuple argument itself PERFORMS and the arm uses `p` more
           than once, the fold declines rather than duplicate the perform (see the effect-duplication guard
           — a substituted performing argument copied per param-use would re-issue its effect).")
  (input  (do
            (effect Add (op sum (-> (Tuple Int64 Int64) Int64)))
            (def (main)
              (handle Add 0 ((sum (p) s (resume (+ (. p 0) (. p 1)) s)))
                (Add.sum (tuple 3 4)))) (export main)))
  (output (: 7 Int64)))

(case "an operation with a RECORD parameter binds the compound and the arm reads its fields"
  (doc    "The record companion of the tuple-parameter case: an operation whose declared PARAMETER is a
           `(Record (a Int64) (b Int64))` binds the whole record to the arm's parameter, whose fields the arm
           reads by member access. `Add.sum : (-> (Record (a Int64) (b Int64)) Int64)`; performed as
           `(Add.sum (record (a 3) (b 4)))`, the arm binds `p` and resumes with `(+ (. p a) (. p b))` = 7.
           The arm references `p` TWICE (once per field), but the argument is a PURE record — it reaches no
           perform — so substituting it into both uses duplicates no effect and the fold serves it (the
           effect-duplication guard only declines a param whose argument REACHES A PERFORM, not a pure
           compound; the precise perform-detector does not misread a record's field pairs as a call). Pins
           that a record OP parameter threads and is field-readable, matching the tuple parameter.")
  (input  (do
            (effect Add (op sum (-> (Record (a Int64) (b Int64)) Int64)))
            (def (main)
              (handle Add 0 ((sum (p) s (resume (+ (. p a) (. p b)) s)))
                (Add.sum (record (a 3) (b 4))))) (export main)))
  (output (: 7 Int64)))

(case "a NON-tail-resumptive arm projects a tuple parameter twice in its pure one-hole context"
  (doc    "The compound-parameter case through the NON-tail-resumptive (pure one-hole) fold path rather than
           the tail-resumptive threading path. The arm `(+ 1 (resume (+ (* (. p 0) 100) (. p 1)) s))` resumes
           NON-tail (the resume is inside `+ 1`), so its delimited continuation is folded as a pure one-hole
           context: the perform IS the whole body (`C = []`), so `(resume v s)` yields `v` and the arm value
           is `(+ 1 v)`. The resume value projects the bound tuple parameter `p` TWICE — `(* (. p 0) 100)`
           and `(. p 1)` — over the PURE argument `(tuple 3 4)`, so `v = 304` and the handle yields
           `(+ 1 304)` = 305. Pins that the pure-one-hole substitution binds a compound op parameter and
           tolerates projecting it multiple times when the argument is pure (a pure argument copied per
           projection duplicates no effect — the same soundness the tail-path duplication guard enforces,
           here satisfied because the argument reaches no perform).")
  (input  (do
            (effect Add (op sum (-> (Tuple Int64 Int64) Int64)))
            (def (main)
              (handle Add 0 ((sum (p) s (+ 1 (resume (+ (* (. p 0) 100) (. p 1)) s))))
                (Add.sum (tuple 3 4)))) (export main)))
  (output (: 305 Int64)))

; The SAME spec sentence has a second half: performing an operation must "YIELD the operation's declared
; RESULT type" (capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row).
; A handler arm resumes the continuation with the value the operation yields — `(resume <value> <state>)`
; "returns <value> to the point that performed the operation" (this file's header) — so the resume VALUE
; is what the operation yields, and it MUST have the operation's declared result type. For `E.op` declared
; `(-> Int64 Int64)`, `(resume true s)` resumes with a Bool where the declared result is Int64 — a type
; mismatch the compiler MUST reject (CDZ0201), exactly as feeding a Bool argument to the perform is (the
; case above) and exactly as an ordinary function whose body returns the wrong type is. A compiler that
; checks a perform's ARGUMENTS but not the resume value against the result type feeds the Bool back through
; the op's Int64-typed result slot and yields the wrong value — `(E.op 1)` returns `true` (and `(resume 99
; s)` for a Bool-result op returns the integer `99`) rather than rejecting. This is the result-type half of
; the perform-argument case above: the two halves of one spec sentence must both hold. A generation that
; does not yet check the resume value against the declared result type declines rather than yielding it.

(case "resuming with a value of the wrong type for the operation's result is a type error"
  (doc    "`E.op` is declared `(-> Int64 Int64)`, so its result type is Int64 and the value a handler
           resumes with — `(resume <value> <state>)`, the value returned to the perform site — MUST be an
           Int64. `(resume true s)` resumes with a Bool, a mismatch against the declared result type the
           compiler MUST reject (CDZ0201, capabilities-and-effects.md #Performing An Operation Is Typed And
           Contributes To The Row: a perform must 'yield the operation's declared result type', so an
           effect operation is typed exactly as an ordinary function application — whose body returning the
           wrong type is rejected the same way). This is the result-type companion of the argument-type
           case above (`(E.op true)`): the same spec sentence checks arguments against parameter types AND
           yields the declared result type. A compiler that checks the arguments but not the resume value
           feeds the Bool through the op's Int64 result slot and yields `true` from `(E.op 1)` rather than
           rejecting. A generation that does not yet check the resume value against the result type
           declines rather than yielding it.")
  (input  (do
            (effect E (op op (-> Int64 Int64)))
            (def (main)
              (handle E unit ((op (n) s (resume true s))) (E.op 1))) (export main)))
  (error  CDZ0201))

; The resume-value result-type check must hold when the declared result type is a COMPOUND, not only a
; scalar. `E.get` declared `(-> (List Int64))` has result type `List Int64`, so a handler resuming with an
; Int64 — `(resume 42 s)` — or a Bool, or a tuple, is the same result-type mismatch the scalar case above
; is, and MUST be rejected (CDZ0201). A compiler that checks the resume value against a SCALAR result type
; but not a compound one yields the mistyped value: `(E.get)` returns `42` for `(resume 42 s)`, and — worse
; — `(resume (tuple 7 8) s)` yields `(list)`, a TUPLE reinterpreted through the op's List result slot and
; rendered as an (empty) list, a type-confusion wrong value. This is the compound-result sibling of the
; scalar-result case above: the "yield the declared result type" check must be uniform across result types,
; not gated by whether the declared result is scalar. A generation that does not yet check a compound
; result type declines rather than yielding the mistyped value.

(case "resuming with a wrong-type value for a compound result type is a type error"
  (doc    "`E.get` is declared `(-> (List Int64))`, so its result type is the compound `List Int64` and the
           value a handler resumes with MUST be a `List Int64`. `(resume 42 s)` resumes with an Int64 — a
           mismatch against the declared compound result type the compiler MUST reject (CDZ0201,
           capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row: a
           perform must 'yield the operation's declared result type'). This is the compound-result-type
           companion of the scalar-result case above (`(resume true s)` for an Int64 result): the check
           must be uniform across result types. A compiler that checks a scalar result type but not a
           compound one yields the mistyped value — `(E.get)` returns `42`, and resuming with a tuple where
           a list is declared renders `(list)`, a type-confusion wrong value. A generation that does not yet
           check a compound result type declines rather than yielding the mistyped value.")
  (input  (do
            (effect E (op get (-> (List Int64))))
            (def (main)
              (handle E unit ((get () s (resume 42 s))) (E.get))) (export main)))
  (error  CDZ0201))

; A resume carries two values — `(resume <value> <state>)` — and BOTH are ordinary expressions subject to
; #Binding Is Lexical (core-semantics.md, unconditional): a reference to an unbound name in either is a
; compile-time error (CDZ0101). The resume VALUE position already rejects an unbound name (`(resume
; undefined-xyz s)` is caught), but the STATE position does not: `(resume unit undefined-xyz)` runs to the
; handler's result instead of rejecting the unbound `undefined-xyz`. A compiler that scope-checks only the
; resume value and not the resume state lets an unbound reference in the state slip through — the same
; unbound-name gap the unselected-conditional-branch and short-circuited-connective-operand cases closed,
; here in a resume's second argument. A generation that does not yet scope-check the resume state declines.

(case "an unbound name in a resume's state position is rejected"
  (doc    "`(resume unit undefined-xyz)` references the unbound name `undefined-xyz` in the resume's STATE
           position (its second argument); a resume's state is an ordinary expression, so an unbound name
           in it is a compile-time error (CDZ0101, core-semantics.md #Binding Is Lexical — unconditional),
           exactly as an unbound name in the resume VALUE position (`(resume undefined-xyz s)`) already is.
           Pins that scope resolution reaches the resume STATE, not only the resume value. A compiler that
           scope-checks the value but not the state runs to the handler's result instead of rejecting. A
           generation that does not yet scope-check the resume state declines rather than emitting a
           component.")
  (input  (do
            (effect E (op put (-> Int64 Unit)))
            (def (main)
              (handle E unit ((put (p) s (resume unit undefined-xyz))) (E.put 1))) (export main)))
  (error  CDZ0101))

(case "a handler arm for an operation the effect does not declare is rejected"
  (doc    "`Choose` declares only `pick`; a handler arm naming `Choose.guess` names an operation the
           effect does not declare, rejected at compile time (CDZ0403) because the declaration is the
           closed set of an effect's operations (capabilities-and-effects.md #A Handler Arm Names An
           Operation Its Effect Declares). A generation that does not yet check arm membership declines
           rather than running the program (reject-don't-miscompile).")
  (input  (do
            (effect Choose (op pick (-> Unit Int64)))
            (def (main)
              (handle Choose unit ((guess () s (resume 5 s))) (Choose.pick))) (export main)))
  (error  CDZ0403))

(case "a handler mixing arms of two different effects is rejected"
  (doc    "A handler discharges EXACTLY ONE effect — every arm names an operation of the handle head's
           declaring effect (capabilities-and-effects.md #A Handler Discharges Exactly One Effect).
           `(handle A … ((a …) (b …)) …)` mixes an arm for `A.a` with an arm for `b`, an operation of a
           DIFFERENT effect `B`; since `b` is not one of `A`'s declared operations, the arm is rejected
           CDZ0403 (the same closed-set check that rejects an undeclared operation name). Discharging two
           effects over one sub-computation is expressed by NESTING a handler per effect, not by enumerating
           two effects' operations in one handler's arms.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main)
              (handle A 0 ((a (u) s (resume 1 s)) (b (u) s (resume 2 s))) (A.a))) (export main)))
  (error  CDZ0403))

(case "a handler that does not discharge every operation of its effect is rejected"
  (doc    "`Diag` declares two operations, `emit` and `collect`; a `handle Diag` binding only `emit`
           leaves `collect` undischarged. A handle names ONE effect and its arms ARE that effect's
           operations, and an effect's operations are a CLOSED, statically-known SET (capabilities-and-
           effects.md #An Effect Declaration Names The Effect And Types Its Operations), so a handler must
           discharge the WHOLE set — the effect analogue of match exhaustiveness over a sum's variants. A
           handler missing an operation is rejected at compile time (CDZ0405): it would claim to discharge
           `Diag` while leaving `Diag.collect` without a home. Discharging a subset across LAYERS is nested
           handles, each exhaustive for its own effect (see the collision-free cross-effect case, which is
           two nested single-operation handlers). A generation that does not yet check handler
           exhaustiveness declines rather than running the partial handler (reject-don't-miscompile).")
  (input  (do
            (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64))))
            (def (main)
              (handle Diag (list) ((emit (code) s (resume unit (List.push s code))))
                (do (Diag.emit 1) 0))) (export main)))
  (error  CDZ0405))

(case "a resume outside any handler arm is rejected"
  (doc    "A `resume` hands a value back to the point that performed a handler arm's operation, so it is
           meaningful ONLY inside a handler arm's body (capabilities-and-effects.md #A Handler Arm May
           Resume). A `resume` in a plain definition body — with no enclosing handler arm to return into —
           is a malformed use of the control form, rejected at compile time (CDZ0201) rather than silently
           accepted and declined only at lowering. Pins that `cdz check` surfaces a stray resume (a
           well-formedness fault visible without emitting), not just the backend.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main) (resume 1 0)) (export main)))
  (error  CDZ0201))

(case "a host delegating a value definition rather than an effect is rejected"
  (doc    "A `host` delegates EFFECTS to the boundary — it grants exactly the effects its body reaches
           (capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative). `(host (foo) …)`
           where `foo` is a value definition names a VALUE, not an effect: there is nothing to delegate, so
           it is a malformed grant, rejected at compile time (CDZ0201) rather than silently accepted as a
           no-op that computes an empty manifest. Pins that a delegation names a declared effect.")
  (input  (do
            (def foo 5)
            (def (main) (host (foo) 5)) (export main)))
  (error  CDZ0201))

(case "a bind directive naming a value definition rather than an effect is rejected"
  (doc    "The `(bind …)` peer-binding analogue of the host-delegates-a-value reject above (the U-pivot
           unifies a peer dependency with an effect, so binding a peer names a declared EFFECT). `(bind foo
           \"cadenza:x/y\")` where `foo` is a value definition names a VALUE, not an effect — there is
           nothing to route to a peer, so it is a malformed binding rejected at compile time (CDZ0201)
           rather than SILENTLY DROPPED (the `bind` scan used to ignore a non-effect/malformed directive, so
           a typo'd binding quietly did nothing). Pins that a peer binding names a declared effect, the same
           bar the host delegation and the `(extern …)` interface hold.")
  (input  (do
            (def (foo) 5)
            (bind foo "cadenza:x/y")
            (def (main) 0) (export main)))
  (error  CDZ0201))

(case "a host delegating the same effect twice is rejected"
  (doc    "A `host`'s effect list is a SET — the manifest is the union of the effects that escape to the
           boundary (capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative). `(host (A
           A) …)` names the effect `A` twice: the same fixed-set-no-duplicates ill-formedness a duplicate
           operation in an effect declaration and a duplicate arm in a handler are rejected for (CDZ0201) —
           a closed set cannot name the same member twice. Rejected at compile time rather than
           double-imported at the boundary (which traps at run time).")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (def (main) (host (A A) (A.a))) (export main)))
  (error  CDZ0201))

(case "binding the same effect to a peer twice is rejected"
  (doc    "The `(bind …)` peer-routing analogue of the duplicate-host-delegation reject above (the U-pivot
           unifies a peer dependency with an effect). A `(bind E \"iface\")` route is a SET — one peer per
           effect — so `(bind E \"cadenza:a/x\") (bind E \"cadenza:b/y\")` binds `E` twice: the same
           fixed-set-no-duplicates ill-formedness (`scan_effect_bindings` silently keeps only the FIRST, so
           the second is a dead, ambiguous line — the author wrote two routes and only one takes). Rejected
           at compile time (CDZ0201) rather than silently dropped. (A compile-request `--bind` REBIND is a
           separate layer — merged after load — and is unaffected; this flags two SOURCE `(bind …)` for one
           effect.)")
  (input  (do
            (effect E (op e (-> Int64 Int64)))
            (bind E "cadenza:a/x")
            (bind E "cadenza:b/y")
            (def (main) (handle E 0 ((e (n) s (resume n s))) (E.e 1))) (export main)))
  (error  CDZ0201))

(case "a bind directive with a malformed peer interface name is rejected"
  (doc    "A `(bind Effect \"iface\")` INTERFACE STRING is a component-boundary name — it is emitted
           VERBATIM as the extern name the peer-instance import binds under, so it must be a valid
           component-model interface name `namespace:package/interface` in kebab-case (lowercase package),
           the same shape the runtime heap import and every provider export use. `\"Math/API\"` is not: an
           uppercase package segment. Without a compile-time check the string would `kebab_extern_name`-
           mangle to the INVALID extern name `math/-a-p-i` and produce a component wasmtime rejects at LOAD
           with NO diagnostic — a silent invalid-component miscompile. Rejected at compile time (CDZ0201)
           naming the offending string, the peer-binding analogue of the other bind rejects. A bare package
           name with no `/interface` projection (`cadenza:math`) is malformed the same way.")
  (input  (do
            (effect Math (op add (-> Int64 Int64 Int64)))
            (bind Math "Math/API")
            (def (main (: x Int64)) (host (Math) (Math.add x x))) (export main)))
  (error  CDZ0201))

(case "a bind directive to a package name with no interface projection is rejected"
  (doc    "The other common face of a malformed peer interface name (the sibling of the uppercase-package
           case above): a bare PACKAGE name with NO `/interface` projection. A peer binding imports an
           interface INSTANCE, whose component extern name is `namespace:package/interface` — the `/iface`
           projection is REQUIRED (component-abi.md; the same shape the runtime heap import
           `cadenza:runtime/heap` uses). `\"cadenza:math\"` names only the package, so it is not a valid
           interface name and the emitted component's import would fail to load. Rejected at compile time
           (CDZ0201) rather than a silent invalid-component miscompile — exercising the projection-required
           branch of the interface-name check, distinct from the kebab/lowercase branch the `Math/API` case
           covers. The likeliest author typo (forgetting the `/api`), so worth its own witness.")
  (input  (do
            (effect Math (op add (-> Int64 Int64)))
            (bind Math "cadenza:math")
            (def (main (: x Int64)) (host (Math) (Math.add x))) (export main)))
  (error  CDZ0201))

(case "a peer-bound operation cannot take or return a closure"
  (doc    "Peers exchange VALUE-HEAP HANDLES (a tuple/record/sum/list/map/string/…); a closure is not a
           value-heap value, so it has no peer-boundary form (a closure crosses the HOST boundary as a
           component-model resource, per closures-across-host, NOT a peer). Without a compile-time check a
           peer-bound op whose signature involves a function type — `(op mk (-> Int64 (-> Int64 Int64)))`
           bound to a peer — type-checks, then APPLYING the peer-returned closure declines deep in lowering
           with an opaque `value is not applyable`. Reject it at the binding (CDZ0201) with the real reason
           — the `(-> …)` in the operation's signature is the tell. Detected SYNTACTICALLY: a boundary
           position of the op's `(-> …)` arrow that is ITSELF a `(-> …)` list. Fires only for a peer-BOUND
           effect (a closure crossing the HOST boundary via `(host …)` is unaffected).")
  (input  (do
            (effect F (op mk (-> Int64 (-> Int64 Int64))))
            (bind F "cadenza:f/api")
            (def (main) 0) (export main)))
  (error  CDZ0201))

(case "a peer-bound operation takes a String argument (it crosses as a runtime handle)"
  (doc    "A String/Bytes ARGUMENT to a peer-bound op crosses the boundary as a runtime rope HANDLE, just
           like a compound (tuple/record) argument — both peers share one value-heap runtime, so the arg is
           an opaque u32 handle into it, never a marshaled component `string`. (This once DECLINED CDZ0201:
           the arg lowered as a component `string` needing a `mem` canonical option the runtime-only peer
           envelope never supplied, producing an invalid consumer component; the inbound-rope-handle emit is
           now wired — `collect_used_ops`/`collect_host_arg_strings` are peer-aware, so a peer String arg
           builds a rope while a HOST String arg still marshals as `(ptr,len)`.) This case pins that
           DECLARING and PERFORMING such an op now COMPILES + runs: an in-program handler overrides the peer
           binding (the free test-mock) and answers `blen(s) = 100` regardless of `s`, so `(S.blen \"hi\")`
           = 100 — proving the String-arg op type-checks and its argument flows without a live peer. The e2e
           crossing to a real peer (byte-len read there) is pinned by the `a_string_argument_crosses_to_a_
           peer_*` backend tests. Only the ARGUMENT direction changed; a String/Bytes RESULT already worked.")
  (input  (do
            (effect S (op blen (-> String Int64)))
            (bind S "cadenza:str/api")
            (def (main)
              (handle S 0 ((blen (s) k (resume 100 k)))
                (S.blen "hi"))) (export main)))
  (output (: 100 Int64))
  (host-calls))

; (The two "peer op whose compound/SUM RESULT escapes the entrypoint declines" corpus cases were REMOVED
;  once the resource-escape × peer-extern envelope FUSION landed — the shapes they witnessed as declines
;  now EMIT + run. The corpus gate cannot compose a live peer, so a peer-crossing RUN can't be a graded
;  case here; the crossings are pinned e2e by the backend `run_with_peers` tests
;  a_peer_{compound,option,list}_result_escapes_the_entrypoint_via_the_fused_envelope.)

(case "a handle whose head names a value rather than an effect is rejected"
  (doc    "A `handle`'s HEAD names the effect the handler discharges, and its arms ARE that effect's
           operations (capabilities-and-effects.md #A Handler Arm Names An Operation Its Effect Declares).
           `(handle foo 0 …)` where `foo` is a value definition names a VALUE, not an effect — a malformed
           handle. Rejected at compile time (CDZ0201) with a message naming the head, rather than surfacing
           as a leaky desugar artifact (the head folds into each arm's member-access projection). Pins that
           a handle head names a declared effect.")
  (input  (do
            (def foo 5)
            (def (main) (handle foo 0 ((x (u) s (resume 1 s))) 5)) (export main)))
  (error  CDZ0201))

(case "an effect operation declared with no name is rejected"
  (doc    "An operation clause is `(op <name> <type>)` — the name is a bare identifier, the type its arrow.
           `(op (-> Unit Int64))` puts the TYPE where the name belongs, declaring a NAMELESS operation:
           there is no `E.op` to project, so the operation is unreachable. An operation must be named, like a
           definition or a sum variant (an effect's operations are a closed, named set,
           capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its Operations), so
           this is rejected at compile time (CDZ0201) rather than silently registered with an empty name.")
  (input  (do
            (effect E (op (-> Unit Int64)))
            (def (main) 5) (export main)))
  (error  CDZ0201))

(case "an effect operation reached with neither a handler nor a delegation is rejected"
  (doc    "`Ask` is a routing-agnostic effect; `main` performs `(Ask.ask)` with no enclosing handler and
           no enclosing entrypoint `host` delegation, so the effect would escape ungranted — rejected at
           compile time (CDZ0401, capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time
           Error). This is the single 'no home for a reached effect' check: since host-binding is now an
           entrypoint routing decision rather than a declaration-time marker, the former CDZ0402
           (undischarged intra-program effect) and the former undeclared-host CDZ0401 are one condition.
           Contrast the interpose case above, where an enclosing `host (ask)` delegation gives the effect
           a home.")
  (input  (do
            (effect Ask (op ask (-> Unit Int64)))
            (def (main)
              (+ (Ask.ask) 1)) (export main)))
  (error  CDZ0401))

(case "the same declared effect is handled in-program by one entrypoint and delegated by another"
  (doc    "Host-binding is a ROUTING decision made at the entrypoint, not a declaration-time property
           (capabilities-and-effects.md #Host-Binding Is A Routing Decision Made At The Entrypoint): an
           effect declaration is a routing-agnostic contract, so ONE `(effect E …)` may be handled entirely
           in-program by one entrypoint AND delegated to the host by another, in the SAME program. Here
           `handled` wraps `(E.ask)` in a `(handle E …)` that resumes 42 — E is discharged in-program and
           does NOT enter the manifest for this entrypoint; `delegated` performs `(E.ask)` under `(host (E)
           …)` — E escapes to the boundary and IS a capability there. `handled()` = 42, deterministically,
           with no host response needed; the routing is decided by the enclosing handler/delegation, never by
           `E`'s declaration.")
  (input  (do
            (effect E (op ask (-> Unit Int64)))
            (def (handled) (handle E 0 ((ask (u) s (resume 42 s))) (E.ask)))
            (def (delegated) (host (E) (E.ask)))
            (export handled) (export delegated)))
  (call   handled)
  (output (: 42 Int64)))

(case "a program that delegates no effect is pure and never suspends"
  (doc    "Witnesses capabilities-and-effects.md #Purity Is The Empty Effect Row: a program that reaches
           no effect it must route runs straight to normal termination, makes no host call, and has an
           empty manifest. This is the same property the compiler component itself has.")
  (input  (do
            (def (main) (+ 20 22)) (export main)))
  (output (: 42 Int64))
  (host-calls))

(case "two effects declared with the same name are distinct, not one merged effect"
  (doc    "Two `(effect Log …)` declarations name the SAME bare `Log` but declare DIFFERENT operation
           sets — the first only `emit`, the second only `record`. They are two DISTINCT effects
           (capabilities-and-effects.md #An Effect's Operations Are A Closed Set: an effect's identity is
           its declaration, not its name), NOT one effect merging both operation sets. A bare `Log`
           reference resolves the first-declared, whose closed operation set is `{emit}`; so a handler arm
           naming `record` — the SECOND Log's operation — names an operation the first Log does not
           declare, rejected CDZ0403. Pins that a same-name second declaration never leaks its operations
           into the first (were the two conflated into one effect declaring `{emit, record}`, the `record`
           arm would be accepted). This is the effect twin of the duplicate-definition rule (11-modules):
           a name resolves to one declaration, never a silent union across same-named declarations.")
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (effect Log (op record (-> Int64 Int64)))
            (def (main)
              (handle Log 0 ((record (n) s (resume n s))) 0)) (export main)))
  (error  CDZ0403))

(case "an effect operation returning a SUM is resumed with a sum value and matched"
  (doc    "The effect/sum intersection: `Ask`'s operation `query` is typed `(-> Int64 Resp)` where `Resp`
           is a user sum `(Yes Int64) | No`. An in-program handler discharges it by RESUMING with a
           constructed sum value `(Resp.Yes n)`, and the body MATCHES the operation's result on `Resp`'s
           variants. `(handle Ask unit ((query (n) s (resume (Resp.Yes n) s))) (match (Ask.query 5) …))`
           resumes with `(Yes 5)`, the match binds `v = 5` → 5. Pins that a sum flows through an effect
           operation's result — constructed in the handler arm, resumed, and deconstructed at the perform
           site — the sum companion of the Int/Unit-resuming handler cases (none of which resume a sum).")
  (input  (do
            (type Resp (Yes Int64) (No))
            (effect Ask (op query (-> Int64 Resp)))
            (def (main (: k Int64))
              (handle Ask unit ((query (n) s (resume (Resp.Yes n) s)))
                (match (Ask.query k) ((Resp.Yes v) v) ((Resp.No) -1))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64)))

(case "a performed sum carrying a TUPLE payload is matched and the arm reads MULTIPLE payload elements"
  (doc    "The effect × sum-with-compound-payload intersection, and a soundness pin against the backend's
           per-arm-body CSE (which shares the sum-payload prefix when an arm reads more than one payload
           element). The performed op returns `(Option (Tuple Int64 Int64))`, the handler resumes a `(Some
           (k, k+1))`, and the matching arm reads BOTH tuple elements. `Look.find : Int64 -> (Option (Tuple
           Int64 Int64))`, arm `(find (k) s (resume (Some (tuple k (+ k 1))) s))`; `(Look.find 5)` resumes
           `(Some (5, 6))`, so the `(Some p)` arm computes `(+ (. p 0) (. p 1))` = `5 + 6` = 11. Pins that a
           sum carrying a TUPLE payload flows through an effect op's result and the arm's two payload
           projections (`.0`, `.1`) — which the per-arm-body CSE folds to a shared payload load — stay sound
           over the effect-produced value, because the fold discharges the perform to a concrete resumed
           value before the optimizer runs. Both backends → 11. The compound-payload companion of the
           scalar-payload sum-resume case above.")
  (input  (do
            (effect Look (op find (-> Int64 (Option (Tuple Int64 Int64)))))
            (def (main)
              (handle Look 0 ((find (k) s (resume (Some (tuple k (+ k 1))) s)))
                (match (Look.find 5)
                  ((Some p) (+ (. p 0) (. p 1)))
                  (None 0)))) (export main)))
  (output (: 11 Int64)))

(case "an effect operation taking a SUM parameter matches it in the handler arm"
  (doc    "The mirror of the sum-RESULT case: `Exec.run` is typed `(-> Cmd Int64)` where `Cmd` is a user
           sum `(Add Int64) | (Mul Int64)`. The PERFORM passes a runtime-built sum `(Exec.run (Cmd.Mul k))`,
           and the handler arm MATCHES the operation's `Cmd` parameter to dispatch — `Cmd.Mul n` resumes
           `(* n 2)`, `Cmd.Add n` resumes `(+ n 1)`. `(Exec.run (Cmd.Mul 5))` → `2*5` = 10. Pins that a sum
           flows INTO an effect operation as its argument, built at the perform site and deconstructed by
           the handler — the operand companion of the sum-result case above.")
  (input  (do
            (type Cmd (Add Int64) (Mul Int64))
            (effect Exec (op run (-> Cmd Int64)))
            (def (main (: k Int64))
              (handle Exec unit
                ((run (c) s (match c ((Cmd.Add n) (resume (+ n 1) s)) ((Cmd.Mul n) (resume (* n 2) s)))))
                (Exec.run (Cmd.Mul k))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 10 Int64)))

(case "a SUM is threaded as a handler's folded state across operations"
  (doc    "A handler's STATE is a user sum `St = (Cnt Int64)` — the value threaded across the operations it
           discharges (capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges). Seeded `(St.Cnt 0)`, each `bump` arm reads the current count out of the sum state
           (`cur`), resumes with it, and threads the incremented sum (`nxt`) as the new state — so two
           `(Tick.bump)`s see 0 then 1. `(+ (bump) (bump))` = 0 + 1 = 1. Pins that a sum is a valid handler
           state value, deconstructed and rebuilt across resumes (the sum companion of the Int-state cases).")
  (input  (do
            (type St (Cnt Int64))
            (effect Tick (op bump (-> Unit Int64)))
            (def (cur (: s St)) (match s ((St.Cnt c) c)))
            (def (nxt (: s St)) (match s ((St.Cnt c) (St.Cnt (+ c 1)))))
            (def (main (: k Int64))
              (handle Tick (St.Cnt 0) ((bump (u) s (resume (cur s) (nxt s))))
                (+ (Tick.bump) (Tick.bump))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 1 Int64)))

(case "a perform in a match-arm guard is discharged by the enclosing handle"
  (doc    "`(handle Ask 5 ((get () s (resume s (- s 1)))) (match 9 ((guard n (> (Ask.get) 3)) 100) (n 200)))`
           — a perform `(Ask.get)` inside a match-arm GUARD condition, discharged by an intra-program
           `handle`. A perform in the SCRUTINEE, ARM BODY, or an IF CONDITION under the same handle all fold,
           and NOW so does a guard condition — for the SOUND, NARROW shape: a guarded arm whose inner pattern
           is IRREFUTABLE (a bare name / `_`) followed by an irrefutable catch-all. Such a match is selected
           iff the guard holds, so `reduce_handle` desugars it to `(if <guard> <arm-body> <catch-all-body>)`
           (each binder let-bound to the scrutinee), where the guard is an `if` CONDITION — a strict-first
           position the if-condition fold routes through the enclosing handle. The guard reads the seed 5,
           `5 > 3` holds, so the first arm fires → 100. (A REFUTABLE guarded pattern, or MULTIPLE guarded
           arms — which sequence handler state per arm-test — is not this narrow shape and still declines
           cleanly, an honest 'not yet reducible' todo, never the misleading 'no enclosing handler'.)")
  (input  (do
            (effect Ask (op get (-> Int64)))
            (def (main)
              (handle Ask 5 ((get () s (resume s (- s 1))))
                (match 9
                  ((guard n (> (Ask.get) 3)) 100)
                  (n 200))))
            (export main)))
  (output (: 100 Int64)))

(case "a performing match-arm guard folds with WILDCARD patterns (no binder to let-bind)"
  (doc    "The wildcard spelling of the guard-desugar above: both the guarded arm's inner pattern and the
           catch-all are `_` (bind nothing), so the desugar to `(if <guard> <arm-body> <catch-all-body>)`
           needs NO enclosing `let` — the bare `if` suffices (the `binders.is_empty()` path). `Ask` seeded 5,
           `(> (Ask.get) 3)` reads 5 → true, so the first arm fires → 100. Pins that the guard-routing
           desugar handles a wildcard-patterned guarded arm (no scrutinee binder) as well as a named one.")
  (input  (do
            (effect Ask (op get (-> Int64)))
            (def (main)
              (handle Ask 5 ((get () s (resume s (- s 1))))
                (match 9
                  ((guard _ (> (Ask.get) 3)) 100)
                  (_ 200))))
            (export main)))
  (output (: 100 Int64)))

(case "an effectful condition of a same-constructor if is performed exactly once"
  (doc    "The evaluate-ONCE pin for the common-constructor if-arm hoist, observable through handler
           state: `(if (< (Ctr.tick) 1) (tuple 1 2) (tuple 3 4))` — both arms build a same-arity tuple,
           so the hoist rewrites to per-element selections over ONE condition value. The counter arm
           `(tick (_) s (resume s (+ s 1)))` returns the current count and threads +1. First perform
           returns 0 → the condition is TRUE → t = (1, 2); the trailing `(Ctr.tick)` then returns 1 (the
           state advanced exactly once by the condition). So 100·1 + 10·2 + 1 = 121. A hoist that
           DUPLICATED the condition per payload slot would perform tick twice (returns 0 then 1 — the
           two element selections disagree, t = (1, 4), and the trailing tick returns 2 → 142); one that
           re-evaluated it once more for the second element still skews the trailing read. Pins that the
           rewrite binds the condition to ONE evaluation whose value feeds every payload selection.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 0 ((tick (_) s (resume s (+ s 1))))
                (let ((t (if (< (Ctr.tick unit) 1) (tuple 1 2) (tuple 3 4))))
                  (+ (+ (* 100 (. t 0)) (* 10 (. t 1))) (Ctr.tick unit)))))
            (export main)))
  (call   main)
  (output (: 121 Int64)))

; --- An abort abandons frames holding LIVE HEAP operands (the Perceus face of the abortive class) --
; The abortive cases above pin CONTROL (which value wins, what unwinds); these pin MEMORY: a pending
; frame abandoned by an abort may hold heap operands — a consuming op's result, a borrowed lookup —
; whose owners are still live OUTSIDE the handle. The abandoned operands must be reclaimed exactly
; once and the owners left intact: an unwind that double-frees (or skips a retain) corrupts the
; owner's later read; one that leaks is invisible here but the owner-read pins the correctness half.

(case "an abort abandons a pending consuming op and the shared binding survives"
  (doc    "`(+ (List.len (List.push xs 9)) (Bail.bail 3))` under `handle Bail` — the LEFT operand has
           already run when the abort fires: `(List.push xs 9)` consumed the still-live `xs` (retain →
           path-copy) and its result sits in the abandoned frame. The abort discards the pending `+`
           and yields 3; the outer `(List.len xs)` then reads the ORIGINAL `xs` → 1, so 3 + 1 = 4. A
           lowering that unwinds without dropping the abandoned push-result leaks it (unobservable
           here), but one that double-drops — or that skipped the retain because the consume 'would be
           abandoned' — corrupts `xs` and misses 4. Pins the retain-then-abandon interaction.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: d Int64))
              (let ((xs (List.push (list) d)))
                (+ (handle Bail 0 ((bail (n) s n))
                     (+ (List.len (List.push xs 9)) (Bail.bail 3)))
                   (List.len xs))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 4 Int64)))

(case "heap-valued handler state above an inner abort keeps threading"
  (doc    "Nested handles where the OUTER handler's state is a HEAP value (a list accumulator) and the
           INNER handle aborts: `(+ (handle Bail … (Bail.bail 10)) (Acc.add 5))` under `handle Acc
           (list) ((add (n) s (resume (List.len s) (List.push s n))))`. The inner abort yields 10 and
           unwinds ONLY its own handle — the outer Acc handler's list state must survive the unwind
           untouched, so the subsequent `(Acc.add 5)` reads len [] = 0 and 10 + 0 = 10. An unwind that
           reclaimed the outer handler's state cell (or reset its threading) corrupts the later perform.
           The heap-state companion of the scalar three-nested-handlers abort case above.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (effect Acc (op add (-> Int64 Int64)))
            (def (main (: d Int64))
              (handle Acc (list) ((add (n) s (resume (List.len s) (List.push s n))))
                (+ (handle Bail 0 ((bail (n) s2 n)) (Bail.bail 10))
                   (Acc.add 5))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 10 Int64)))

(case "an abort abandons a pending borrowed map lookup and the map survives"
  (doc    "The borrowed-operand face: `(+ (Option.expect (Map.lookup m \"k\") \"v\") (Bail.bail 20))` —
           the lookup's extracted value (from the still-live `m`) is pending in the abandoned frame when
           the abort fires. The handle yields 20; the outer `(Map.len m)` must still see the intact map
           → 1, so 21. An unwind that dropped the abandoned lookup result as if OWNED would free the
           value `m` still holds — the abort-path twin of the borrowed-key ownership discipline the
           lookup/contains emits observe on the normal path.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: d Int64))
              (let ((m (Map.insert Map.empty "k" 1)))
                (+ (handle Bail 0 ((bail (n) s n))
                     (+ (Option.expect (Map.lookup m "k") "v") (Bail.bail 20)))
                   (Map.len m))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 21 Int64)))

(case "a fresh-id supply threads state across two sibling recursive calls in one arm"
  (doc    "The natural effectful TREE WALK — `relabel(Node l r) = relabel(l) + relabel(r)` with the
           `Fresh.next` gensym at the leaf. Two SIBLING self-recursive calls in one `match` arm: the
           handler state the FIRST sibling advances must be visible to the SECOND (each leaf draws the
           next id). Under a 0-based counter a 3-leaf tree draws 0, 1, 2 → 3. The single-return
           specialization threaded only the INCOMING state to each self-call, so both siblings drew the
           same id (a state-reset miscompile) and the shape was DECLINED; the multi-value-return
           specialization (`f#ctx` yields `(value, out-state)`, each self-call let-bound and its out-state
           threaded to the next sibling) folds it correctly. The canonical compiler-pass gensym over a
           tree (node numbering, SSA names, type-variable ids).")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Tree (Leaf) (Node Tree Tree))
            (def (relabel (: t Tree))
              (match t
                ((Leaf) (Fresh.next))
                ((Node l r) (+ (relabel l) (relabel r)))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (relabel (Node (Node (Leaf) (Leaf)) (Leaf)))))
            (export main)))
  (output (: 3 Int64)))

(case "sibling-recursive effect threading is left-to-right (order-observing)"
  (doc    "The same tree walk but with a NON-COMMUTATIVE combiner `(- (relabel l) (relabel r))`, so the
           result witnesses the EVALUATION ORDER of the two siblings: the LEFT sibling draws first (the
           smaller id). `(Node (Leaf) (Leaf))` → left id 0, right id 1 → 0 - 1 = -1. A right-first or
           state-reset threading would give 0 - 0 = 0 or 1 - 0 = 1; -1 pins strict left-to-right
           out-state threading between the siblings.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Tree (Leaf) (Node Tree Tree))
            (def (relabel (: t Tree))
              (match t
                ((Leaf) (Fresh.next))
                ((Node l r) (- (relabel l) (relabel r)))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (relabel (Node (Leaf) (Leaf)))))
            (export main)))
  (output (: -1 Int64)))

(case "a perform BETWEEN two sibling recursive calls threads the intervening state"
  (doc    "`relabel(Node l r) = (relabel l) + Fresh.next() + (relabel r)` — a discharged perform sits
           BETWEEN the two sibling self-calls on the strict spine, so it draws the id the LEFT sibling
           left and hands the advanced state to the RIGHT sibling. `(Node (Leaf) (Leaf))`: left draws 0,
           the middle perform draws 1, right draws 2 → 0 + 1 + 2 = 3. Exercises the multi-value out-state
           threading interleaved with an ordinary perform in one arm.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Tree (Leaf) (Node Tree Tree))
            (def (relabel (: t Tree))
              (match t
                ((Leaf) (Fresh.next))
                ((Node l r) (+ (+ (relabel l) (Fresh.next)) (relabel r)))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (relabel (Node (Leaf) (Leaf)))))
            (export main)))
  (output (: 3 Int64)))

(case "sibling recursive calls sequenced through let bindings thread state"
  (doc    "The `let`-sequenced form of the sibling walk — `(Node l r) => let a = relabel l in let b =
           relabel r in a - b` — the shape a hand-written SSA linearizer uses (bind the left result, then
           the right, threading the id counter through the RESULT). The second binding's init must thread
           against the state the first advanced. `(Node (Leaf) (Leaf))` → a = 0, b = 1 → -1. Confirms the
           multi-value out-state threads through `let` inits, not only bare operator operands.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Tree (Leaf) (Node Tree Tree))
            (def (relabel (: t Tree))
              (match t
                ((Leaf) (Fresh.next))
                ((Node l r)
                  (let ((a (relabel l)))
                    (let ((b (relabel r)))
                      (- a b))))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (relabel (Node (Leaf) (Leaf)))))
            (export main)))
  (output (: -1 Int64)))

(case "a sibling-recursive walk threads a HEAP list accumulator across the siblings"
  (doc    "The ssa/collect face of the multi-value-return walk: each leaf draws a fresh id into a
           singleton list and a Node CONCATENATES its two sibling walks' lists — `collect(Node l r) =
           List.concat (collect l) (collect r)`. The out-state a self-call advances is threaded to its
           sibling, and the VALUE carried back through the tuple return is now a HEAP value (a List), not
           a scalar — so this pins that the multi-value return threads a heap-allocated result across the
           siblings correctly (a `.0` projection off the runtime tuple, not just an Int64). A 3-leaf tree
           draws ids 0,1,2 into a length-3 list. Regression guard for the real SSA-linearizer shape,
           where the accumulated instruction list is the threaded heap value.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Tree (Leaf) (Node Tree Tree))
            (def (collect (: t Tree))
              (match t
                ((Leaf) ((. List push) (list) (Fresh.next)))
                ((Node l r) ((. List concat) (collect l) (collect r)))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                ((. List len) (collect (Node (Node (Leaf) (Leaf)) (Leaf))))))
            (export main)))
  (output (: 3 Int64)))

(case "a post-order effectful walk draws each node's id AFTER both children (SSA reg-alloc shape)"
  (doc    "The exact SSA register-allocation shape: a node's own id is drawn AFTER lowering both children
           — `lower(Bin l r) = let a = lower l in let b = lower r in Fresh.next()`, so the parent register
           number follows its subtrees'. The two sibling self-calls (`lower l`, `lower r`) each advance
           the id supply, then the node itself draws the NEXT id — the multi-value return must thread the
           counter through BOTH children and leave the parent's draw last. `Bin (Lit) (Bin (Lit) (Lit))`
           over a 0-based counter: left Lit=0, right subtree (Lit=1, Lit=2, its Bin=3), root Bin=4 → the
           root's result register is 4. Pins the natural post-order gensym the compiler-ml SSA linearizer
           writes (its hand-threaded counter can become this effectful walk once repro-1 landed).")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Expr (Lit Int64) (Bin Expr Expr))
            (def (lower (: e Expr))
              (match e
                ((Lit v) (Fresh.next))
                ((Bin l r) (let ((a (lower l))) (let ((b (lower r))) (Fresh.next))))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (lower (Bin (Lit 1) (Bin (Lit 2) (Lit 3))))))
            (export main)))
  (output (: 4 Int64)))

(case "a cross-function recursive fold's out-state threads to a later perform in the caller's continuation"
  (doc    "The CALLER-observed out-state face of the multi-value return (the recursive analogue of the
           inlined helper-call out-state threading). `run-ops` recursively performs `Prim.run` per list
           element, advancing the handler state s -> s+1 each time; the handle body is `(do (run-ops [1 2
           3]) (Prim.run 0))`, so a TRAILING perform in the caller's `do` — AFTER the recursive fold
           returns — must observe the state the recursion accumulated. Three performs advance 0 -> 3, and
           the trailing `(Prim.run 0)` resumes with s = 3. The single-return specialization drops the
           recursion's final out-state (returns the incoming state unchanged), silently miscompiling the
           trailing perform to the PRE-recursion 0; forcing MULTI-VALUE specialization when the caller's
           spine observes the out-state threads the advance through. Regression guard for task #15.")
  (input  (do
            (effect Prim (op run (-> Int64 Int64)))
            (def (run-ops (: ops (List Int64)))
              (match ops
                ((list h .. rest) (do (Prim.run h) (run-ops rest)))
                (_ 0)))
            (def (main)
              (handle Prim 0 ((run (tag) s (resume s (+ s 1))))
                (do (run-ops (list 1 2 3)) (Prim.run 0))))
            (export main)))
  (output (: 3 Int64)))

(case "a nested inner handler that re-threads its own state folds (merged-context seed from init)"
  (doc    "Two NESTED handlers over a cross-function recursive loop that performs BOTH effects — the
           merged-context signature. The INNER handler `Tools` re-threads its OWN bound state in the arm
           `(step (a) s (resume a s))` — the resume's next-state is the state BINDER `s`, not a fresh
           value. `type_of` of a bare state binder alone is `Any` (its type is the seed's), so deriving
           the merged inner slot's type from the arms' next-states ALONE yielded `Any` and DECLINED the
           merge — while the SAME handler standalone folded (single-handler `reduce_handle` seeds the slot
           type from the init). The merged path now seeds identically from the inner `init` (`Tools 0` →
           Int64), so a stateful inner handler re-threading `s` folds. `loop 3 0` draws step ids handing
           back the accumulator each turn: 3, then 2, then 1 → stop(6) → 6. (Reported by v-agent-harness
           Inc-2; the fix mirrors the single-handler init-seeded state-type derivation.)")
  (input  (do
            (effect Model (op ask (-> Int64 Int64)))
            (effect Tools (op step (-> Int64 Int64)) (op stop (-> Int64 Int64)))
            (def (loop (: i Int64) (: acc Int64))
              (if (= (Model.ask i) 0)
                  (Tools.stop acc)
                  (loop (- i 1) (Tools.step (+ acc i)))))
            (def (main)
              (handle Model 0 ((ask (q) s (resume q q)))
                (handle Tools 0 ((step (a) s (resume a s)) (stop (a) s (resume a s)))
                  (loop 3 0))))
            (export main)))
  (output (: 6 Int64)))

(case "a post-order labeling walk returns a labeled tree (heap result, id drawn after children)"
  (doc    "The canonical compiler NODE-NUMBERING pass: walk a `Tree`, draw a fresh `Fresh.next` id per node,
           and RETURN a new labeled `Ann` tree (a HEAP result, not a scalar sum). Post-order — each node's
           own id is drawn AFTER labeling both children via `let`-bound sibling recursion, so the two
           sibling self-calls thread the id supply and the parent's id follows its subtrees'. Exercises the
           multi-value return carrying a heap-constructed result across siblings PLUS the parent draw last.
           Tree `(Node (Node Leaf Leaf) Leaf)`: inner-left Leaf=0, inner-right Leaf=1, inner Node=2, outer
           Leaf=3, root Node=4 → the root's label is 4. The real 'label every node, return the labeled
           tree' shape the compiler-ml port's numbering pass writes.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Tree (Leaf) (Node Tree Tree))
            (type Ann (ALeaf Int64) (ANode Int64 Ann Ann))
            (def (relabel (: t Tree))
              (match t
                ((Leaf) (ALeaf (Fresh.next)))
                ((Node l r)
                  (let ((la (relabel l)))
                    (let ((ra (relabel r)))
                      (ANode (Fresh.next) la ra))))))
            (def (root-id (: a Ann)) (match a ((ALeaf i) i) ((ANode i l r) i)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (root-id (relabel (Node (Node (Leaf) (Leaf)) (Leaf))))))
            (export main)))
  (output (: 4 Int64)))

(case "a fresh-id walk over a THREE-constructor sum threads state across mixed-arity arms"
  (doc    "The gensym walk generalized to a real `Expr` sum with THREE constructors of DIFFERENT arities —
           `Lit` (nullary, one id), `Neg` (one child + its own id), `Add` (two children + its own id). Each
           arm performs `Fresh.next` and recurses on its children with the id supply threaded left-to-right
           across the (0, 1, or 2) sibling self-calls. Confirms the multi-value sibling-threading is not
           special to a 2-constructor Leaf/Node tree — a match arm with a perform-then-N-siblings folds for
           any arity. `Add (Neg Lit) Lit`: Add=0, Neg=1, Lit-under-Neg=2, right-Lit=3 → sum 6.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Expr (Lit) (Add Expr Expr) (Neg Expr))
            (def (count-ids (: e Expr))
              (match e
                ((Lit) (Fresh.next))
                ((Neg x) (+ (Fresh.next) (count-ids x)))
                ((Add l r) (+ (Fresh.next) (+ (count-ids l) (count-ids r))))))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (count-ids (Add (Neg (Lit)) (Lit)))))
            (export main)))
  (output (: 6 Int64)))

(case "an effect-performing helper called inside a recursive self-call's argument folds"
  (doc    "A recursive driver `run` whose SELF-CALL argument contains a call to a separate effect-performing
           HELPER `turn` — `(run (- fuel 1) (+ acc (turn fuel)))`, where `turn a = Tools.dispatch a`. Threading
           the self-call's arg inlines `turn` (β-reduces + threads its performing body); the inlined
           `Tools.dispatch` resumes `(a a)` (hands its arg back AND as the next state). The resume VALUE and
           NEXT-STATE are the SAME substituted-arg node, and it is RESOLVE-PINNED (a bare param occurrence),
           so the ordinary copy SHARED one node across the two splice positions — a single-parent-arena
           orphan that surfaced the driver's own params as CDZ0101 `unbound name fuel`/`acc` (reported by
           v-agent-harness Inc-3). A DEEP-FRESH copy of the resume value/next-state gives each splice its own
           subtree, re-resolving against the specialized def's sig. `run 4 0` accumulates dispatch(4..1) =
           4+3+2+1 → done(10) → 10. Pins the effectful-helper-in-a-self-call-arg shape a self-hosted pass
           writes (a per-node effectful helper threaded through a recursive walk).")
  (input  (do
            (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (turn (: a Int64)) (Tools.dispatch a))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (+ acc (turn fuel)))))
            (def (main)
              (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a)))
                (run 4 0)))
            (export main)))
  (output (: 10 Int64)))

(case "an effectful helper that also reads an outer/driver parameter folds in a self-call arg"
  (doc    "The follow-up to the single-param effectful-helper-in-a-self-call-arg case: here the helper
           `turn` performs AND references a DRIVER parameter in its own body — `turn(a, acc) = acc +
           Tools.dispatch a`, called as `(run (- fuel 1) (turn fuel acc))`, where `acc` is also `run`'s
           param. Inlining `turn` β-substitutes the driver's `acc` into the helper body by returning the arg
           node AS-IS (the pinned-name fast path), so that `acc` kept a pin to `run`'s now-dead scope; when
           the inline happens INSIDE the recursive self-call's arg, the reduced body lands in the synthesized
           `f#ctx` def where the pinned `acc` no longer resolves → CDZ0101 `unbound name acc`. Deep-fresh-
           copying the reduced inline body drops the stale pins so every name re-resolves against the
           specialized def's sig (carrying the driver's params). `run 4 0` = 4+3+2+1 = 10. (v-agent-harness
           Inc-3 follow-up.)")
  (input  (do
            (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (turn (: a Int64) (: acc Int64)) (+ acc (Tools.dispatch a)))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn fuel acc))))
            (def (main)
              (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a)))
                (run 4 0)))
            (export main)))
  (output (: 10 Int64)))

; NEIGHBORS of the effectful-helper-in-a-self-call-arg deep-fresh-copy fix (breaker): the case above pins
; ONE driver param (acc) read by the inlined helper. These push the same deep-fresh-copy path: TWO driver
; params read at once, the helper called TWICE (nested) in the self-call arg (each inline must get fresh
; pins), and a helper whose OWN param NAME shadows a driver param. All fold cleanly and re-resolve against
; the specialized def's sig — a stale pin from any of these shapes would surface as CDZ0101 unbound name.

(case "an effectful helper reading TWO driver parameters folds in a self-call arg"
  (doc    "The two-driver-param extension: `turn(a, acc, fuel) = acc + fuel + Tools.dispatch a` reads BOTH
           `acc` and `fuel` (both `run`'s params), called `(run (- fuel 1) (turn fuel acc fuel))`. Inlining
           β-substitutes two driver pins into the helper body inside the self-call arg; the deep-fresh-copy
           must drop BOTH stale pins so each re-resolves against the specialized def's sig. With dispatch a →
           a, turn = acc + 2*fuel; run 3 0 = 6, 10, 12 → 12. A copy that missed one pin → CDZ0101.")
  (input  (do
            (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (turn (: a Int64) (: acc Int64) (: fuel Int64)) (+ (+ acc fuel) (Tools.dispatch a)))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn fuel acc fuel))))
            (def (main)
              (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a)))
                (run 3 0)))
            (export main)))
  (output (: 12 Int64)))

(case "an effectful helper called twice (nested) in a self-call arg folds each inline independently"
  (doc    "The helper appears TWICE in the self-call arg — `(turn fuel (turn fuel acc))` — so the inliner
           reduces two copies of the effectful body into the same self-call arg. Each inline must be
           deep-fresh-copied independently; a shared or stale pin across the two copies would collide or fail
           to resolve. With turn(a,acc) = acc + a: run 2 0 → inner turn(2,0)=2, outer turn(2,2)=4; then
           inner turn(1,4)=5, outer turn(1,5)=6 → done 6. Pins that repeated inlining in one arg is sound.")
  (input  (do
            (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (turn (: a Int64) (: acc Int64)) (+ acc (Tools.dispatch a)))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn fuel (turn fuel acc)))))
            (def (main)
              (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a)))
                (run 2 0)))
            (export main)))
  (output (: 6 Int64)))

(case "an effectful helper whose own parameter name shadows a driver parameter folds in a self-call arg"
  (doc    "Name-collision edge: the helper's OWN param is named `acc` — the same name as `run`'s driver
           param — and the helper performs on it: `turn(acc) = acc + Tools.dispatch acc`, called `(turn acc)`
           in the self-call arg. The deep-fresh-copy + re-resolve must bind the inlined body's `acc` to the
           helper's param, not leave a stale pin to `run`'s `acc`. With dispatch acc → acc, turn doubles:
           run 3 1 = 2, 4, 8 → done 8. A mis-resolution to the driver's `acc` (or a stale pin) would give a
           wrong value or CDZ0101.")
  (input  (do
            (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (turn (: acc Int64)) (+ acc (Tools.dispatch acc)))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn acc))))
            (def (main)
              (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a)))
                (run 3 1)))
            (export main)))
  (output (: 8 Int64)))

(case "a state-advancing helper called before a later read threads its write through the continuation"
  (doc    "A memoized-DB shape (a self-hosting compiler's `demand`): the helper `demand` reads state, and on a
           MISS writes it (`(do (Db.put …) compute)`) before returning; a LATER read in the caller's
           continuation must SEE that write. `demand` inlines into the handle body, and its `None` arm's
           effectful `(Db.put …)` sits inside a `do` under a `match`. Two composed loci made this a silent
           miscompile (→ 99): (1) inlining `demand` collapsed its `(do (Db.put …) compute)` to bare `compute`
           (a `do` resolves to its last form — dropping the effectful intermediate on the substituting inline
           path); (2) even preserved, a branch's state advance was dropped as the conditional's out-state. The
           fix preserves the `do` on inline and re-hoists the exposed conditional to tail position so the
           branch `put` threads. `demand 5 25` misses → writes 5→25 → returns 25; the later `Db.get 5` now
           HITS (Some 25) → 25 + 25 = 50. A drop of the write takes the None arm → 99.")
  (input  (do
            (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
            (def (demand (: k Int64) (: compute Int64))
              (match (Db.get k)
                (((. Option Some) v) v)
                (((. Option None) u) (do (Db.put (tuple k compute)) compute))))
            (def (run-then-get)
              (handle Db (Map.empty)
                ((get (k) s (resume (Map.lookup s k) s))
                 (put (kv) s (match kv ((tuple k v) (resume unit (Map.insert s k v))))))
                (let ((a (demand 5 25)))
                  (match (Db.get 5) (((. Option Some) v) (+ a v)) (((. Option None) u) 99)))))
            (export run-then-get)))
  (output (: 50 Int64)))

(case "a helper called TWICE threads its first write so the second call HITS the memoized value"
  (doc    "The cumulative-loss companion of the single-demand case above — the WIDER witness that a
           state-advancing helper's write survives across MULTIPLE later calls, not just one. `demand`
           is a memoizing `demand`: on a MISS it `(Db.put …)` then returns; on a HIT it returns the
           stored value. `demand 5 25` misses → writes 5↦25 → returns 25. Then `demand 5 999` must HIT
           the FIRST call's put (`Db.get 5` → Some 25) and return 25 — NOT re-miss and recompute 999. So
           `a + b` = 25 + 25 = 50. A drop of the FIRST call's out-state across the SECOND call (the
           cumulative-loss bug the single-demand case cannot catch — it only reads state once) would make
           the second demand miss → recompute 999 → 1024. Pins that the handler state threads through a
           CHAIN of helper calls, each seeing every prior call's writes.")
  (input  (do
            (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
            (def (demand (: k Int64) (: compute Int64))
              (match (Db.get k)
                (((. Option Some) v) v)
                (((. Option None) u) (do (Db.put (tuple k compute)) compute))))
            (def (run-twice)
              (handle Db (Map.empty)
                ((get (k) s (resume (Map.lookup s k) s))
                 (put (kv) s (match kv ((tuple k v) (resume unit (Map.insert s k v))))))
                (let ((a (demand 5 25)) (b (demand 5 999)))
                  (+ a b))))
            (export run-twice)))
  (output (: 50 Int64)))
