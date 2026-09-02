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
; `(resume <value> <next-state>)` returns <value> to the point that performed the operation and
; threads <next-state> forward to the rest of the sub-computation. RESUME is ONE-SHOT by default: an arm
; resumes at most once. Multi-shot is not a supported feature (operator-punted, 2026-08-28: no immediate
; use); the invariant that matters is a clean DECLINE for a single-shot violation — an arm that resumes
; more than once when the continuation is NOT safely re-runnable (a HEAP-CAPTURING continuation, whose
; second resume would double-use the captured heap) DECLINES rather than miscompiles (case `mrs1`). A
; PURE, heap-free continuation happens to be safely re-runnable, so a second resume over it re-computes
; and is left to fold (case `mrs2` = 100) — kept only because it is free + sound, NOT promoted.
; A handle EVALUATES TO THE VALUE OF ITS
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
(case
  "a run's result is a deterministic function of a host call's recorded response"
  (doc
    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: `ask` is a routing-agnostic effect the entrypoint delegates to the host, so
           `ask.ask` is a plain imported-function call returning its response at the boundary. The
           (host-responses …) fixture supplies the response in call order; given that response the run
           deterministically computes 100. How the host produces the response — inline, fiber-suspend, or
           re-derive from the recorded responses — is host policy the program does not observe.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (* (ask.ask) 10)))
      (export main)))
  (host-responses (respond ask.ask (: 10 Int64)))
  (host-calls (call ask.ask))
  (output (: 100 Int64)))

; An operation is PERFORMED like a function, so its declared type MUST be an arrow `(-> Arg… Result)` (a
; nullary op is `(-> Result)`). A WELL-FORMED non-arrow type `(op get Int64)` was silently accepted and
; leaked the internal op-record on perform ("(effect-op Any)"). Rejected AT THE DECLARATION (CDZ0201 "an
; operation's type must be an arrow") with a wrap-into-arrow fix; a bare-type op that is ALSO performed
; reports the ONE decl-site error (the leak is deduped → (no-other-errors)). A canonical `(-> …)` op
; compiles; an UNKNOWN op-type NAME keeps its more-actionable CDZ0101, not a spurious arrow reject. (migrated
; from rcdzc a_non_arrow_effect_operation_type_is_rejected_with_a_wrap_fix.)
(case
  "a non-arrow effect operation type is rejected with a wrap-into-arrow fix"
  (input (do (effect E (op get Int64)) (def (main) 0) (export main)))
  (error CDZ0201 (message "an operation's type must be an arrow") (fix (kind wrap))))

(case
  "a non-arrow generic effect operation type is rejected"
  (input (do (effect E (op get (Option Int64))) (def (main) 0) (export main)))
  (error CDZ0201 (message "an operation's type must be an arrow")))

(case
  "a performed non-arrow operation reports one decl-site error, not the internal op-record leak"
  (input
    (do
      (effect E (op get Int64))
      (def (main) (handle E 0 ((get (u) s (resume 5 s))) (+ (E.get) 1)))
      (export main)))
  (error CDZ0201 (message "an operation's type must be an arrow"))
  (no-other-errors))

(case
  "a canonical nullary arrow operation type compiles (the control)"
  (input (do (effect E (op get (-> Int64))) (def (main) 0) (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "an unknown operation-type name keeps its unbound CDZ0101, not a spurious arrow reject"
  (input (do (effect E (op get Nonesuch)) (def (main) 0) (export main)))
  (error CDZ0101))

; An operation declared with NO type at all — `(op get)` — is rejected at the declaration (CDZ0201 "this
; operation has no type"). Performing such an op used to leak the internal op-record "(effect-op Any)" plus a
; no-home CDZ0401 consequent; those are deduped so ONE primary decl-site error remains → (no-other-errors).
; (migrated from rcdzc an_operation_declared_with_no_type_is_rejected_cdz0201.)
(case
  "an operation declared with no type is rejected"
  (input (do (effect E (op get)) (def (main) 1) (export main)))
  (error CDZ0201 (message "this operation has no type")))

(case
  "a performed no-type operation reports one decl-site error, deduping the op-record + CDZ0401 leaks"
  (input (do (effect E (op get)) (def (main) (E.get)) (export main)))
  (error CDZ0201 (message "this operation has no type"))
  (no-other-errors))

(case
  "an effect operation with no name is rejected"
  (doc
    "An operation clause is `(op <name> <type>)`; the name must be a bare name. `(op (-> Unit Int64))` puts
        the TYPE where the name belongs, silently registering a nameless (unreachable) op → now CDZ0201: an
        operation must be named, like a def or a variant. (migrated from rcdzc
        an_effect_operation_with_no_name_is_rejected.)")
  (input (do (effect E (op (-> Unit Int64))) (def (main) 5) (export main)))
  (error CDZ0201 (message "named") (message "op")))

(case
  "a branch-dead host-call leaks no import or host-call at any optimization level"
  (doc
    "Capability-safety fence at the #4805 (force-lower-all POST-layout) seam: an effectful helper `io`
           that delegates `ask.ask` to the host is referenced ONLY inside a `(if false …)` branch that const-
           fold eliminates, so `io` becomes DEAD. The module must NOT acquire the host-call boundary for dead
           code — a leaked import would spuriously require the `ask` capability for a program that never
           performs it. The case runs to 42 WITHOUT any `(host-responses …)` fixture: if force-lower-all (or
           DCE) leaked the `ask.ask` call, `main` would require a response at the boundary and the run would
           fail rather than return 42. Verified opt-level-equivalent (identical 42 across O0..O3) and import-
           free (a `wasm-tools` import count of 0 at every level, vs 2 for the reachable host-call twin). A
           regression that force-lowered the dead effectful def into the boundary would either decline on the
           unhandled effect at O2 or emit a spurious host import.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (io) (host (ask) (ask.ask)))
      (def (main) (if false (io) 42))
      (export main)))
  (output (: 42 Int64)))

(case
  "a host op whose result is a QUANTITY crosses the boundary as its inner scalar (unit erased)"
  (doc
    "The runtime-parameter `@param` Quantity host path: a host-delegated op whose declared result is a
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
  (input
    (do
      (effect Env (op width (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def (main) (host (Env) (Qty.value (Env.width))))
      (export main)))
  (host-responses (respond env.width (: 42 Int64)))
  (host-calls (call env.width))
  (output (: 42 Int64)))

; The Int64-inner case above pins the magnitude crossing. These pin the SOUNDNESS half of the unit
; erasure: the unit is erased at the BOUNDARY but preserved GUEST-SIDE by the op's declared type — so
; two same-unit host results combine as a valid same-dimension add, a cross-unit combine REJECTS at
; compile time (a wrong-dimension host value is inexpressible: the host has no unit channel), and a
; Float64-inner Qty rides the same erased-scalar path as Int64.
(case
  "two same-unit Qty host results combine guest-side as a same-dimension add"
  (doc
    "`(+ (Env.width) (Env.width))` where both results are `(Qty Int64 meter)`: each host call crosses
           as a bare Int64 magnitude (42 + 42), but the guest's static types carry `meter` on BOTH operands,
           so the `+` is a valid same-dimension combine → `Qty.value` reads 84. Pins that the boundary
           erasure does not LOSE the unit guest-side — the add type-checks as Qty+Qty, not as bare ints that
           happen to work. Two calls consume two responses in order. Expected: 84.")
  (input
    (do
      (effect Env (op width (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def (main) (host (Env) (Qty.value (+ (Env.width) (Env.width)))))
      (export main)))
  (host-responses (respond env.width (: 42 Int64)) (respond env.width (: 42 Int64)))
  (host-calls (call env.width) (call env.width))
  (output (: 84 Int64)))

(case
  "cross-unit Qty host results reject at the guest-side add"
  (doc
    "The load-bearing soundness face: `(+ (Env.w) (Env.t))` where w yields `(Qty Int64 meter)` and t
           `(Qty Int64 second)` — the units are erased at the boundary, but the guest-side static types
           still carry the dimensions, so the add is a dimension MISMATCH and rejects CDZ0501 at compile
           time. This is exactly the fix's soundness claim: a wrong-dimension host value is INEXPRESSIBLE
           (the host supplies only magnitudes; units are fixed guest-side by each op's declared type), so
           erasure cannot smuggle a meter into a second. Rejects on every backend (frontend-shared).")
  (input
    (do
      (effect
        Env
        (op w (-> Unit (Qty Int64 (Unit.base #"meter"))))
        (op t (-> Unit (Qty Int64 (Unit.base #"second")))))
      (def (main) (host (Env) (Qty.value (+ (Env.w) (Env.t)))))
      (export main)))
  (error CDZ0501))

(case
  "a Float64-inner Qty host result crosses as its float magnitude"
  (doc
    "The float-inner axis of the Qty host ABI: `Env.w : Unit -> (Qty Float64 meter)` crosses as a bare
           Float64 (3.5), the guest's static type carrying the unit — `Qty.value` reads 3.5 back. Pins that
           `abi_val_type` resolves a Qty to its INNER's ABI type for a float inner exactly as for Int64 (the
           landed case above); a heap-inner (Rational) Qty rides the num/den pair instead (#13 cases at the
           file top). Expected: 3.5.")
  (input
    (do
      (effect Env (op w (-> Unit (Qty Float64 (Unit.base #"meter")))))
      (def (main) (host (Env) (Qty.value (Env.w))))
      (export main)))
  (host-responses (respond env.w (: 3.5 Float64)))
  (host-calls (call env.w))
  (output (: 3.5 Float64)))

(case
  "an exact RATIONAL host value crosses as two scalar num/den ops the guest recombines (#13)"
  (doc
    "The num/den Qty ABI (#13): a host cannot supply a heap `Rational` directly (a compound has no host
           boundary form), so an exact-rational runtime value crosses as TWO SCALAR host ops — `rate-num :
           Unit -> Int64` and `rate-den : Unit -> Int64` — and the GUEST recombines them with `Rational.of
           (num, den)`. This reuses the fully-supported scalar host boundary (no tuple/memory/resource
           envelope surgery) and is exactly what a `@param(...) rate : Rational` (or a Rational-magnitude
           `Length`) desugars to: two scalar accessors + a guest `Rational.of`. With the host responding num=7,
           den=2, the guest builds the exact rational 7/2 (normalized). Pins that a Rational runtime value is
           expressible over the scalar host path — the operator-ruled minimal boundary form for #13 (a single
           atomic Rational host op is a documented future path, unbuilt — no consumer needs it). The result is
           a heap Rational, so `main` crosses it via the resource-escape value path.")
  (input
    (do
      (effect Env (op rate-num (-> Unit Int64)) (op rate-den (-> Unit Int64)))
      (def (main) (host (Env) (Rational.of (Env.rate-num) (Env.rate-den))))
      (export main)))
  (host-responses (respond env.rate-num (: 7 Int64)) (respond env.rate-den (: 2 Int64)))
  (host-calls (call env.rate-num) (call env.rate-den))
  (output (: 7/2 Rational))
  (live-objects known-leak))

(case
  "a Rational-MAGNITUDE Quantity host value composes the num/den ops with the unit erasure (#13, B2)"
  (doc
    "#13 B2 — the actual `@param(...) : Length` shape: a Quantity whose MAGNITUDE is an exact Rational.
           The magnitude crosses as the same TWO SCALAR num/den host ops (B1), the guest recombines them with
           `Rational.of(num, den)`, and `Qty.of(…, meter)` attaches the unit GUEST-SIDE — the unit is a
           compile-time value erased at the boundary (layer-2, the scalar-inner Qty host path), so a
           Rational-magnitude Qty needs NO extra boundary channel beyond the two scalars. Two same-unit
           `(Qty Rational meter)` values ADD (dimension-checked) — `x + x` for `x = 7/2 meter` → `7/1 meter` —
           and `Qty.value` names the result; its VALUE FORM is the bare exact rational `7/1` (the unit is a
           compile-time value, erased from the runtime value). Pins that a Rational magnitude flows through Qty
           construction + same-unit arithmetic over the scalar host path (num=7, den=2 → 7/2 meter; doubled →
           7/1). This is what a v-cad `@param Length` desugars to.")
  (input
    (do
      (effect Env (op rate-num (-> Unit Int64)) (op rate-den (-> Unit Int64)))
      (def
        (main)
        (host
          (Env)
          (let
            ((x (Qty.of (Rational.of (Env.rate-num) (Env.rate-den)) (Unit.base #"meter"))))
            (Qty.value (+ x x)))))
      (export main)))
  (host-responses (respond env.rate-num (: 7 Int64)) (respond env.rate-den (: 2 Int64)))
  (host-calls (call env.rate-num) (call env.rate-den))
  (output (: 7/1 Rational))
  (live-objects known-leak))

; The case above fixes ONE response. On its own it cannot distinguish a run that genuinely CONSUMES the
; response value from a compiler that hardcoded 100 — both produce 100. This pair pins that the response
; VALUE flows into the result: the SAME program with a DIFFERENT response produces a DIFFERENT (but
; deterministic) result, so the run is a function OF the response, not a constant
; (capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And Responses). The third
; case pins that MULTIPLE responses combine in call order through a NON-commutative operator — swapping the
; consumption order would give -18, not 18 — so the ordered response fixture feeds the computation as
; recorded.
(case
  "the same program with a different response gives a different deterministic result"
  (doc
    "The discriminating companion of the determinism case above: the identical program `(* (ask.ask)
           10)` with the response fixed at 7 (not 10) deterministically computes 70. Together with the
           10 → 100 case, this pins that the run genuinely CONSUMES the response value (a compiler that
           hardcoded 100 would fail here) — the result is a function OF the response, deterministic given it.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (* (ask.ask) 10)))
      (export main)))
  (host-responses (respond ask.ask (: 7 Int64)))
  (host-calls (call ask.ask))
  (output (: 70 Int64)))

(case
  "two host responses combine in call order through a non-commutative operator"
  (doc
    "`(- (io.get) (io.get))` performs `io.get` twice; the ordered fixture supplies 30 then 12, so the
           FIRST call consumes 30 and the second 12, and `30 - 12` = 18. `-` is non-commutative, so a run
           that consumed the responses in the wrong order would compute `12 - 30` = -18 — the recorded 18
           pins that the two responses feed the computation in the order the fixture records them
           (capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And Responses; the
           ordered-consumption companion of the two-calls-in-order observation below).")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def (main) (host (io) (- (io.get) (io.get))))
      (export main)))
  (host-responses (respond io.get (: 30 Int64)) (respond io.get (: 12 Int64)))
  (host-calls (call io.get) (call io.get))
  (output (: 18 Int64)))

(case
  "an EMPTY Bytes arg crosses the host boundary"
  (doc
    "The zero-length edge of the Bytes host-ARG marshal (#1640's wasm face): an empty Bytes
           value crosses as an empty list<u8> and the op fires normally. (rust-async: todo pending its
           host-arg path; wasm + rust pin the pass.)")
  (input
    (do
      (effect io (op sink (-> Bytes Int64)))
      (def (main (: k Int64)) (host (io) (io.sink (Bytes.of #list()))))
      (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call main (: 0 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a ROPE Bytes arg (recursive concat, uncompacted) crosses the host boundary"
  (doc
    "The representation edge: a 50-leaf rope built by recursive Bytes.concat crosses the
           boundary — the marshal must flatten/walk the rope rep, not assume a flat leaf. (rust-async:
           todo pending; wasm + rust pin.)")
  (input
    (do
      (effect io (op sink (-> Bytes Int64)))
      (def
        (build (: n Int64) (: acc Bytes))
        (if (> n 0) (build (- n 1) (Bytes.concat acc (Bytes.of #list((UInt8.wrap 65))))) acc))
      (def (main (: n Int64)) (host (io) (io.sink (build n (Bytes.of #list())))))
      (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call main (: 50 Int64))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a Bytes value SENT to the host is still readable after the call (the arg marshal borrows)"
  (doc
    "The consuming-op discipline at the ARG site: `b` is passed to io.sink AND re-read by
           Bytes.len after — the marshal must borrow/copy, not consume (a consuming marshal would
           leave the later len reading freed memory, the adv-54/66 class at the boundary). 7 + 50.
           (rust-async: todo pending; wasm + rust pin.)")
  (input
    (do
      (effect io (op sink (-> Bytes Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (let
            ((b (String.to-bytes (String.concat "ab" (if (> k 100) "z" "cde")))))
            (+ (io.sink b) (* 10 (Bytes.len b))))))
      (export main)))
  (host-responses (respond io.sink (: 7 Int64)))
  (host-calls (call io.sink))
  (call main (: 0 Int64))
  (output (: 57 Int64))
  (live-objects known-leak))

(case
  "a runtime Bytes host-arg BEFORE a scalar arg keeps distinct core slots (no width-clobber)"
  (doc
    "The multi-arg slot-threading edge of the host-ARG marshal: a RUNTIME String/Bytes arg reserves
           i32 rope/len/pos scratch slots (at `base.max(high)`) and bumps `high`, but the emit arm formerly
           reused the STALE `base` for the FOLLOWING arg — so a subsequent scalar's i64 checked-arith guard
           teed into a slot the marshal had declared i32, one wasm local at two widths → an INVALID module
           (`func failed to validate: expected i64, found i32`). Only the marshalled-arg-BEFORE-scalar order
           tripped it; scalar-before-marshalled worked because the scalar bumped `high` first. Fixed by
           threading a rising `arg_base` (as the ordinary call arg loop does). Here `n = k+7` is BOTH the
           scalar arg AND re-read after the call (`10*n`), so a clobbered slot would corrupt the output, not
           just fail to validate: send responds 5, so 5 + 10*7 = 75. (rust-async: todo pending its host-arg
           path; wasm + rust pin the pass.)")
  (input
    (do
      (effect io (op send (-> Bytes Int64 Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (let
            ((n (+ k 7)))
            (+ (io.send (String.to-bytes (String.concat "ab" (if (> k 100) "z" "cd"))) n) (* 10 n)))))
      (export main)))
  (host-responses (respond io.send (: 5 Int64)))
  (host-calls (call io.send))
  (call main (: 0 Int64))
  (output (: 75 Int64))
  (live-objects known-leak))

(case
  "one host effect with TWO ops interleaves its calls in program order"
  (doc
    "The per-run response cursor over a MULTI-OP effect: geta, getb, geta consume rows 1,2,3 in
           the order made — the cursor is per-RUN, not per-op (a per-op cursor would give the second
           geta row 2's value... the harness rows are per-call-order). 1 + 20 + 300 = 321. Pins the
           multi-op single-effect composition the adv-65 fix's lone-op cases don't. (rust-async: todo
           pending; wasm + rust pin.)")
  (input
    (do
      (effect AB (op geta (-> Unit Int64)) (op getb (-> Unit Int64)))
      (def
        (main (: k Int64))
        (host (AB) (+ (AB.geta unit) (+ (* 10 (AB.getb unit)) (* 100 (AB.geta unit))))))
      (export main)))
  (host-responses
    (respond a-b.geta (: 1 Int64))
    (respond a-b.getb (: 2 Int64))
    (respond a-b.geta (: 3 Int64)))
  (host-calls (call a-b.geta) (call a-b.getb) (call a-b.geta))
  (call main (: 0 Int64))
  (output (: 321 Int64)))

(case
  "a 60-key trie captured ACROSS a host call reads intact after the response folds in"
  (doc
    "The deep-heap survival face of host delegation: a 60-key multi-level trie is built BEFORE the
           host block, the delegation fires (response 7), and the trie reads len + a checked interior
           entry AFTER the response is consumed (7·1000 + 60·10 + 1 = 7601). The guest heap must survive
           the boundary crossing untouched — a delegation that reset or corrupted live heap state (or a
           marshal that clobbered the trie's handle slot) would break a read. The trie-scale companion
           of the scalar-arg re-read pin at :251.")
  (input
    (do
      (effect io (op ping (-> Unit Int64)))
      (def
        (fill (: i Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 3)))))
      (def
        (main (: n Int64))
        (do
          (def m (fill n Map.empty))
          (host
            (io)
            (+
              (* 1000 (io.ping))
              (+
                (* 10 (Map.len m))
                (match (Map.lookup m 37) ((Some v) (if (= v 111) 1 0)) ((None _u) -1)))))))
      (export main)))
  (call main (: 60 Int64))
  (host-responses (respond io.ping (: 7 Int64)))
  (host-calls (call io.ping))
  (output (: 7601 Int64)))

(case
  "a deep trie built BETWEEN two host calls reads correctly after the second"
  (doc
    "The interleave face: the first response is consumed, a 50-key trie is built ENTIRELY between
           the two delegations (the response cursor mid-flight), and the second response arrives before
           the trie is read — (3+4)·1000 + 50 + 42 = 7092. Pins that heap construction interleaves with
           the per-run response cursor without either corrupting the other (a cursor implementation
           sharing scratch state with the allocator, or a build that disturbed the pending-delegation
           frame, would flip a component).")
  (input
    (do
      (effect io (op ping (-> Unit Int64)))
      (def
        (fill (: i Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
      (def
        (main (: n Int64))
        (host
          (io)
          (do
            (def a (io.ping))
            (def m (fill n Map.empty))
            (def b (io.ping))
            (+
              (* 1000 (+ a b))
              (+ (Map.len m) (match (Map.lookup m 42) ((Some v) v) ((None _u) -1)))))))
      (export main)))
  (call main (: 50 Int64))
  (host-responses (respond io.ping (: 3 Int64)) (respond io.ping (: 4 Int64)))
  (host-calls (call io.ping) (call io.ping))
  (output (: 7092 Int64)))

(case
  "a String host RESULT crosses the boundary and is read twice (byte-len + scalar-len of a multibyte response)"
  (doc
    "The String-RESULT boundary face (H7's marshal reached through H9's unit-arg emit): `io.fetch :
           (-> Unit String)` returns the recorded multibyte response \"héllo\" (6 bytes, 5 scalars), which
           the guest let-binds and reads TWICE — byte-len then scalar-len — so the crossed String is a
           live guest value under the consuming-op discipline (the binding must be kept; a per-read
           re-fetch would consume a second, unsupplied response and trap). 6 + 100·5 = 506. This is the
           shape that was DECLINING arg-side pre-H9 while the String-result emit arm sat unreachable —
           the pin keeps it reachable. (wasm/rust-async: todo until their unit-arg + String-result host
           paths land; the rust baseline pins the pass.)")
  (input
    (do
      (effect io (op fetch (-> Unit String)))
      (def
        (main (: k Int64))
        (host
          (io)
          (let ((s (io.fetch unit))) (+ (String.byte-len s) (* 100 (String.scalar-len s))))))
      (export main)))
  (host-responses (respond io.fetch (: "héllo" String)))
  (host-calls (call io.fetch))
  (call main (: 0 Int64))
  (output (: 506 Int64)))

(case
  "two host calls consume their responses in order"
  (doc
    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: two host calls consume two responses in the order made; the sum is a deterministic
           function of input and the ordered response sequence.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (+ (ask.ask) (ask.ask))))
      (export main)))
  (host-responses (respond ask.ask (: 3 Int64)) (respond ask.ask (: 4 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 7 Int64)))

(case
  "an effectful host arg to a multi-use function parameter is evaluated ONCE, not re-performed per use"
  (doc
    "Witnesses core-semantics.md #Applying A Function (the parameter binds to a single evaluated
           argument value) + capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses (the host-call sequence is deterministic). `(mk (ask.ask))` passes a HOST perform as the
           argument to `mk`, whose parameter `s` is used THREE times. Strict by-value binding evaluates the
           argument ONCE at the call and binds its value to `s` — so the run makes exactly ONE host call
           (consuming the single response 5) and the three uses read the bound 5: (+ (+ 5 5) 5) = 15. A
           call-by-name substitution would re-perform `ask.ask` per use (three calls) — a duplicated
           observable effect, which this pins against.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (mk (: s Int64)) (+ (+ s s) s))
      (def (main) (host (ask) (mk (ask.ask))))
      (export main)))
  (host-responses (respond ask.ask (: 5 Int64)))
  (host-calls (call ask.ask))
  (output (: 15 Int64)))

(case
  "an effectful host arg flowing into a compound then a destructuring match is evaluated ONCE"
  (doc
    "The compound-into-destructuring-match companion of the multi-use evaluate-once case. `(mk (ask.ask))`
           passes ONE host perform to `mk`, which builds `(T s s s)` (the arg reused three times), and `sum3`
           DESTRUCTURES that with a match binding a, b, c. Strict by-value binding + a single-materialized
           match scrutinee mean the host op runs EXACTLY ONCE (response 5, so s = 5) and the three payload
           binders read the stored value: (+ (+ 5 5) 5) = 15. A per-use re-perform (call-by-name) or a
           per-payload-binder re-emission of the match scrutinee would make three host calls — this pins one.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (type Trip (T Int64 Int64 Int64))
      (def (mk (: s Int64)) (T s s s))
      (def (sum3 (: t Trip)) (match t ((T a b c) (+ (+ a b) c))))
      (def (main) (host (ask) (sum3 (mk (ask.ask)))))
      (export main)))
  (host-responses (respond ask.ask (: 5 Int64)))
  (host-calls (call ask.ask))
  (output (: 15 Int64))
  (live-objects 0))

(case
  "performs in DISCARDED do positions still run — the effect count is the observable"
  (doc
    "The side-effect-only statement face (the evaluate-ONCE pins above bound the count from ABOVE;
           this bounds it from BELOW): three bare `(St.a)` statements whose results nothing binds or
           consumes, followed by an observer. Each statement must still perform and advance — the observer
           reads 8, not the seed 5. An optimizer that reasoned 'result unused → drop the call' would
           silently skip the advances; the statement position's effect is the whole point of writing it.")
  (input
    (do
      (effect St (op a (-> Unit Int64)) (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((a (u) s (resume 0 (+ s 1))) (get (u) s (resume s s)))
          (do (St.a) (St.a) (St.a) (St.get))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 8 Int64)))

(case
  "an abortive perform in a connective that is an if-condition abandons the computation when the connective reaches it"
  (doc
    "The abortive analogue of the connective-in-condition threading, for a NON-resuming handler. `(and
           b (> (Bail.bail 7) 0))` is the CONDITION of `(if _ 100 200)`; when `b` is true the connective
           evaluates its right operand, performing the abortive `Bail.bail 7`, which abandons the whole
           computation — the handle's value is the arm value 7. Witnesses capabilities-and-effects.md
           short-circuit evaluation + abortive-handler semantics: the abort in a taken connective operand
           abandons regardless of its nesting under the enclosing if. A regression against the
           connective-in-condition abort over-declining. (The b=false short-circuit companion — the abort
           never performed — is the sibling case just below.)")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (run (: b Bool))
        (handle Bail 0 ((bail (n) s n)) (if (and b (> (Bail.bail 7) 0)) 100 200)))
      (def (main) (run true))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abortive perform short-circuited out of a connective condition is never performed"
  (doc
    "The short-circuit companion: with `b` false, `(and b …)` never evaluates its right operand, so the
           abortive `Bail.bail 7` is NOT performed — no abandonment — and the outer `if` takes its else
           branch, 200. Pins that the connective-condition abort fold preserves short-circuit semantics (the
           abort does not fire on the untaken operand).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (run (: b Bool))
        (handle Bail 0 ((bail (n) s n)) (if (and b (> (Bail.bail 7) 0)) 100 200)))
      (def (main) (run false))
      (export main)))
  (output (: 200 Int64)))

(case
  "an `and` whose first RESUMING perform is true evaluates the second: both advances land"
  (doc
    "The RESUMPTIVE face of connective sequencing (the Bail pins above cover abortive operands): both
           operands of `(and (> (St.get) 3) (> (St.get) 10))` perform an ADVANCING op. The first reads 5
           (true → the right operand runs), the second reads 6 (false), and the trailing observer reads 7 —
           both advances landed. 10 + 7 = 17. A fold that skipped the second operand despite the first being
           true, or double-ran either, shifts the observer's read.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((get (u) s (resume s (+ s 1))))
          (+ (if (and (> (St.get) 3) (> (St.get) 10)) 100 10) (St.get))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 17 Int64)))

; ── Handler-state threading through helper calls, if-conditions, cross-fn folds, and successive folds ──
; These pin that the tail-resumptive fold threads its advancing state across the places a naive lowering
; dropped it: a handle held in a HELPER seeded by the caller's runtime param; a performing connective in an
; if-CONDITION whose advance must reach the taken branch; that condition composed with a cross-fn fold and a
; trailing perform; a trap-init let short-circuiting one branch while the sibling threads; and two successive
; self-recursive folds where the second threads against the first's advanced state (not the seed).
(case
  "a handle held in a helper seeded by the caller's runtime param resumes the seed"
  (doc
    "The handle lives in a helper `run` seeded by the caller's runtime argument `k`; the identity arm
           `(get (u) s (resume s s))` resumes the seed unchanged, so `run(k)` = k. run(9) -> 9. A regression
           against a bogus CDZ0101 'unbound k' when the helper-held handle's reduced body lost its chain to
           the caller's binder.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (run (: s0 Int64)) (handle St s0 ((get (u) s (resume s s))) (St.get)))
      (def (main (: k Int64)) (run k))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "a handle as an exported fn's DIRECT body resumes the caller's runtime-param seed"
  (doc
    "The identity-arm-resumes-seed pin's DIRECT-in-export face (the helper-held case seeded by a
           caller param covers the helper variant). Here the handle IS the export's direct body and the
           threaded resume result is a BARE NAME referencing main's OWN param k — the reduced body must
           reparent to the export def's param so k re-resolves, else it read UNBOUND and the fn declined
           'no machine representation' / a bogus CDZ0101. The identity arm (get (u) s (resume s s)) resumes
           the seed unchanged, so main(k) = k. k=9 -> 9. This pure pass-through was the gap the advancing/
           compound/helper-held variants each hid.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (main (: k Int64)) (handle St k ((get (u) s (resume s s))) (St.get)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "a resuming perform in an if-condition threads its state advance to the taken branch"
  (doc
    "`(if (and b (> (St.tick) 0)) (St.tick) -99)` seeded 0, arm `(tick (u) s (resume (+ s 1) (+ s 1)))`.
           With b=true the condition's tick advances state 0->1, so the then-branch `(St.tick)` reads 1 and
           resumes 2 — the condition's advance must reach the branch (a naive lowering dropped it, so the
           branch re-read the pre-condition seed and returned 1). b=false short-circuits: the condition's tick
           never runs, so the else -99. The resuming companion of the abortive connective-in-condition cases
           above.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main (: b Bool))
        (handle
          St
          0
          ((tick (u) s (resume (+ s 1) (+ s 1))))
          (if (and b (> (St.tick) 0)) (St.tick) -99)))
      (export main)))
  (call main (: true Bool))
  (output (: 2 Int64))
  (call main (: false Bool))
  (output (: -99 Int64)))

(case
  "a performing if-condition composes with a cross-fn fold and a trailing perform"
  (doc
    "Composition: the condition `(and true (> (St.tick) 0))` performs tick #1 (state 0->1); the
           then-branch `(do (run-ops (list 1 2 3)) (St.tick))` runs a cross-fn recursive fold doing 3 ticks
           (state 1->4) then a trailing tick #5 reads state 4 and resumes 5. Pins that the condition's
           advance, the cross-fn fold's out-state, and the trailing perform all thread in one composition ->
           5.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (run-ops (: ops (List Int64)))
        (match ops (#list(h (.. rest)) (do (St.tick) (run-ops rest))) (_ 0)))
      (def
        (main)
        (handle
          St
          0
          ((tick (u) s (resume (+ s 1) (+ s 1))))
          (if (and true (> (St.tick) 0)) (do (run-ops #list(1 2 3)) (St.tick)) -99)))
      (export main)))
  (call main)
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a trap-init let in one handler branch short-circuits while the sibling threads state"
  (doc
    "`(if b (let ((it (trap \"dead\"))) (+ it (St.tick))) (St.tick))` seeded 0, arm `(tick (u) s (resume
           (+ s 1) (+ s 1)))`. b=false threads the tick normally -> 1. b=true binds a trap in the let-init, so
           the whole let folds to the trap (the `(+ it (St.tick))` and its perform never run) -> traps as a
           raw `unreachable` (a user `trap`'s message is not surfaced on the wasm/rust backends — the
           observable reason is 'unreachable', not the source 'dead', matching every other user-`trap`
           case; see 07-type-system's `(let ((x (trap \"boom\"))) …)` -> `(trap \"unreachable\")`). Pins
           that the trap-init short-circuit does not perturb the sibling branch's state threading nor force
           the dead branch's perform.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main (: b Bool))
        (handle
          St
          0
          ((tick (u) s (resume (+ s 1) (+ s 1))))
          (if b (let ((it (trap "dead"))) (+ it (St.tick))) (St.tick))))
      (export main)))
  (call main (: false Bool))
  (output (: 1 Int64))
  (call main (: true Bool))
  (trap "unreachable"))

(case
  "two successive self-recursive folds thread state between them"
  (doc
    "Two successive `(dn 2)` folds under one Counter handler: `dn n = if n==0 then 0 else dn(n-1) +
           Counter.bump()` bumps state s->s+1 resuming the current s. The first `(dn 2)` bumps at s=0,1 -> 1
           (state now 2); the second `(dn 2)` bumps at s=2,3 -> 5. `(+ (* 1000 (dn 2)) (dn 2))` = 1000*1 + 5 =
           1005. Pins that the caller-observed out-state threads the FIRST fold's advance into the SECOND (a
           state reset would give 1001).")
  (input
    (do
      (effect Counter (op bump (-> Int64)))
      (def (dn (: n Int64)) (if (= n 0) 0 (+ (dn (- n 1)) (Counter.bump))))
      (def (main) (handle Counter 0 ((bump () s (resume s (+ s 1)))) (+ (* 1000 (dn 2)) (dn 2))))
      (export main)))
  (call main)
  (output (: 1005 Int64)))

(case
  "an `or` short-circuit SKIPS a resuming perform, and the skip is observable through the state"
  (doc
    "The skip-observability pin: `(or (> (St.get) 3) (> (St.get) 0))` — the first operand reads 5
           (true), so the second perform MUST NOT run. The proof is the trailing observer: it reads 6 (one
           advance), not 7 (two). An eager lowering that evaluated both operands and discarded the second's
           result would still pick the right branch (100) but betray itself in the state — this pins the
           EFFECT COUNT of short-circuiting, not just the boolean result. 100 + 6 = 106.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((get (u) s (resume s (+ s 1))))
          (+ (if (or (> (St.get) 3) (> (St.get) 0)) 100 10) (St.get))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 106 Int64)))

(case
  "a handler arm applies a closure capturing a heap map and the map outlives the handle"
  (doc
    "EFFECTS × CAPTURE: the `look` arm applies f — a closure whose capture cell holds main's
           heap map — so the arm runs in the HANDLER's frame while the capture belongs to the
           performer's. The body performs TWICE, so arm + closure apply twice through the
           perform/resume machinery (each round-trip suspends and re-enters frames), and m is read
           AFTER the handle exits — the capture must survive every suspension. r = look(2)·100 +
           look(1) = 20·100 + 10 = 2010 via the arm's (resume (f k) s); post-handle c = m[3]=30
           hit (mode 1, sentinel-safe +1 → 31) or m[9] miss → 0 (mode 2): mode 1 → 2041,
           mode 2 → 2010.")
  (input
    (do
      (effect Look (op look (-> Int64 Int64)))
      (def
        (build (: i Int64) (: n Int64) (: acc (Map Int64 Int64)))
        (if (> i n) acc (build (+ i 1) n (Map.insert acc i (* i 10)))))
      (def
        (get (: m (Map Int64 Int64)) (: k Int64))
        (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
      (def
        (main (: mode Int64))
        (do
          (def m (build 1 3 Map.empty))
          (def f (fn ((: k Int64)) (get m k)))
          (def
            r
            (handle Look 0 ((look (k) s (resume (f k) s))) (+ (* (Look.look 2) 100) (Look.look 1))))
          (def c (get m (if (= mode 1) 3 9)))
          (+ r (if (>= c 0) (+ c 1) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2041 Int64))
  (call main (: 2 Int64))
  (output (: 2010 Int64)))

(case
  "a NON-LAST handler arm whose body is a MATCH round-trips through the ML printer"
  (doc
    "The regression witness for the arm-extent printer fix (v-syntax, batch #136): a NON-LAST
           handler arm whose body is a match, followed by a sibling arm — pre-fix the inner match's
           pipe-arms absorbed the next handler arm on ML re-read (AST mismatch); print_handle_arm
           now paren-guards greedy block bodies. Exercises ml_surface_round_trips_the_corpus
           end-to-end (the lib-side printer test uses hand-built ASTs). Both dispatch faces compute.")
  (input
    (do
      (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          0
          ((a (v) s (match v (0 (resume 1 s)) (_ (resume 2 s)))) (b (v) s (resume v s)))
          (+ (S.a n) (S.b 10))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 11 Int64))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

(case
  "a rope built before a perform survives the resume and the arm's own heap does not leak into it"
  (doc
    "RESUME-boundary heap liveness, both directions: the performing BODY holds a rope (passed
           in as a param) across the suspension — read AFTER the resume returns, so the suspended
           frame's heap must stay live through the arm's execution; meanwhile the ARM builds its
           OWN rope (a do-def, folded into the resume value via byte-len) — arm-frame heap that must
           reclaim at arm exit without leaking into or freeing the resumed frame's. (The arm's
           do-def flows into the RESUME argument, which works — only the body-side PERFORM-argument
           path has the #21 do-def scoping gap, hence rope arrives as a param.) r = look(2)·10 +
           (byte-len rope − 6) + byte-len rope = (20+2)·10 + 0 + 6 = 226; post-handle c = m[1]=10
           hit (mode 1 → +11) or m[9] miss → 0: mode 1 → 2271, mode 2 → 2260.")
  (input
    (do
      (effect Look (op look (-> Int64 Int64)))
      (def
        (build (: i Int64) (: n Int64) (: acc (Map Int64 Int64)))
        (if (> i n) acc (build (+ i 1) n (Map.insert acc i (* i 10)))))
      (def
        (get (: m (Map Int64 Int64)) (: k Int64))
        (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (body (: rope String))
        (+ (* (Look.look 2) 10) (+ (- (String.byte-len rope) 6) (String.byte-len rope))))
      (def
        (main (: mode Int64))
        (do
          (def m (build 1 2 Map.empty))
          (def
            r
            (handle
              Look
              0
              ((look
                  (k)
                  s
                  (do (def arope (rep "z" 2 "")) (resume (+ (get m k) (String.byte-len arope)) s))))
              (body (rep "ab" 3 ""))))
          (def c (get m (if (= mode 1) 1 9)))
          (+ (* r 10) (if (>= c 0) (+ c 1) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2271 Int64))
  (call main (: 2 Int64))
  (output (: 2260 Int64))
  (live-objects 0))

(case
  "an abortive handler discards a suspended body holding live rope and map handles"
  (doc
    "The ABORT companion of the resume-boundary pin above: the body builds a rope (a do-def —
           exercising the #21 abortive-face fix, v-effects 0d382e3f4) and performs with its byte-len;
           the `bail` arm NEVER resumes, so the suspended body — which still holds the rope AND a
           borrowed read of the caller's map queued after the perform — is DISCARDED. The abandoned
           frame's heap must reclaim exactly once (no leak, no double-free), and the caller's map
           must survive the abandonment: c reads m AFTER the aborted handle. r = arm value =
           byte-len \"ababab\" = 6; mode 1 c = m[2]=20 (+1 sentinel-safe → 21), mode 2 c = m[9]
           miss → 0: mode 1 → 621, mode 2 → 600.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (build (: i Int64) (: n Int64) (: acc (Map Int64 Int64)))
        (if (> i n) acc (build (+ i 1) n (Map.insert acc i (* i 10)))))
      (def
        (get (: m (Map Int64 Int64)) (: k Int64))
        (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
      (def
        (rep (: s String) (: n Int64) (: acc String))
        (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
      (def
        (run (: m (Map Int64 Int64)))
        (handle
          Bail
          0
          ((bail (n) s n))
          (do (def rope (rep "ab" 3 "")) (+ (Bail.bail (String.byte-len rope)) (get m 1)))))
      (def
        (main (: mode Int64))
        (do
          (def m (build 1 3 Map.empty))
          (def r (run m))
          (def c (get m (if (= mode 1) 2 9)))
          (+ (* r 100) (if (>= c 0) (+ c 1) 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 621 Int64))
  (call main (: 2 Int64))
  (output (: 600 Int64))
  (live-objects 0))

(case
  "a single-task DES scheduler sleeps a task and fast-forwards the clock to its wake instant"
  (doc
    "The discrete-event-simulation single-task gate (v-discrete-event-sim's step-3 forcing repro,
           minimal distillation). A `worker` task sleeps then returns its label; the `Sim` handler's `sleep`
           arm computes the wake instant and resumes with the clock advanced (a `let`-wrapped tail resume;
           the `k` binder is the scheduler ABI's reified-continuation slot, unused in the single-task case
           which resumes in place). Witnesses capabilities-and-effects.md continuation/resume semantics for
           the sleep/fast-forward idiom: the task sleeps 3s, the clock fast-forwards, the continuation
           resumes and returns \"done\". Value-grades the sleep-wake fold (a todo→fail flip here = k not
           resumed / clock not advanced). The full multi-task pqueue interleave (stored k across activations)
           is v-discrete-event-sim's follow-on gate.")
  (input
    (do
      (type Duration (Duration UInt64))
      (type Instant (Instant UInt64))
      (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (dur-ns (: d Duration)) (match d ((Duration.Duration n) n)))
      (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
      (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
      (def (worker (: label String) (: d Duration)) (do (Sim.sleep d) label))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s)) (sleep (d) s k (let ((wake (at s d))) (resume unit wake))))
          (worker "done" (secs 3))))
      (export main)))
  (output (: "done" String)))

(case
  "a ctl-style arm whose continuation ESCAPES to another function reifies it as a closure and applies it there"
  (doc
    "E5 step-3: a general `ctl`-style arm may let its continuation `k` ESCAPE — pass it to another
           function that applies it — not just apply it lexically in place. `(f () s k (use-k k))` hands `k`
           to `use-k`, which applies `(stored-k 10)`. Over the pure delimited continuation `C = (+ 1 □)`, the
           reified `k` is the closure `(fn (kv) (+ 1 kv))`; `use-k` applies it to 10 → (+ 1 10) = 11.
           Witnesses that a reified continuation over a pure continuation is a first-class value (an ordinary
           closure) that can cross a function boundary and be resumed there — the escaping-continuation
           capability a scheduler builds on. (A continuation that itself re-performs the handled effect is a
           further increment — it must re-enter its handler at apply.)")
  (input
    (do
      (effect A (op f (-> Unit Int64)))
      (def (use-k (: stored-k (-> Int64 Int64))) (stored-k 10))
      (def (main) (handle A 0 ((f () s k (use-k k))) (+ 1 (A.f))))
      (export main)))
  (output (: 11 Int64))
  (live-objects 0))

(case
  "an escaping continuation that itself RE-PERFORMS the handled effect re-enters its handler at apply"
  (doc
    "E5 step-3 (FACE-1 B2): the escaping-`k` case whose delimited continuation `C` itself RE-PERFORMS
           the handled effect. `(a () s k (use-k k))` over `(+ (A.a) (A.a))`: after the leading `(A.a)` the
           continuation `C = (+ □ (A.a))` performs `A.a` AGAIN. A pure-continuation closure reification does
           not serve it — applying `k` runs `C` in a SEPARATE activation where the re-performed op has no
           home. So `k` reifies as a SELF-RE-INSTALLING handler-wrapped closure `k = (fn (kv) (handle A 5
           (arm) (+ kv (A.a))))` — the continuation carries the handler around itself. `use-k` applies it to
           10: the re-installed handle folds `(+ 10 (A.a))` (one remaining perform) → (+ 10 10) = 20. Each
           re-install removes one perform (N→N-1), bottoming out at the pure-one-hole fold — no bespoke frame
           chain. The state-oblivious 2-perform case; a state-advancing arm or a deeper continuation is a
           further increment (declines cleanly). The re-entry-at-apply the DES scheduler's stored-k builds on.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (def (use-k (: stored-k (-> Int64 Int64))) (stored-k 10))
      (def (main) (handle A 5 ((a () s k (use-k k))) (+ (A.a) (A.a))))
      (export main)))
  (output (: 20 Int64))
  (live-objects 0))

(case
  "a re-performing escaping continuation over a `do`-sequenced body re-installs its handler at apply"
  (doc
    "The `do`-body variant of the escaping-`k` reinstall above (the DES scheduler's body shape): the
           handle body is a `(do (A.a) (A.a))` SEQUENCE rather than an arithmetic `(+ (A.a) (A.a))`. After
           the leading `(A.a)`, the continuation `C = (do □ (A.a))` re-performs, so `k` reifies as the
           self-re-installing handler-wrapped closure `k = (fn (kv) (handle A 5 (arm) (do kv (A.a))))`.
           `use-k` applies it to 7: the re-installed handle folds the inner `(A.a)` → `(k 7)` = 7, and the
           `do` yields its last item → 7. Pins that the re-perform reinstall folds over a `do`-sequenced
           continuation (not only an arithmetic one) — never a 'value is not applyable' decline.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (def (use-k (: k (-> Int64 Int64))) (k 7))
      (def (main) (handle A 5 ((a (u) s k (use-k k))) (do (A.a) (A.a))))
      (export main)))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "a DEFERRED resume-thunk escaping to another function re-installs the handler at apply, over a re-performing do-continuation"
  (doc
    "E5 step-3 (the DES scheduler's `sleep`/`now` step-3 shape, contract-A1). The escaping continuation
           is a DEFERRED RESUME-THUNK: the `set` arm hands `(fn (_u) (resume w w))` to `run-thunk`, which
           applies it cross-activation. `resume`'s SECOND arg `w` is the NEW handler state (the advance),
           EXPRESSED in the program — no op-arg magic. The handle BODY `(do (A.set 42) (A.get))` is a
           SEQUENCE whose continuation `C = (do □ (A.get))` itself RE-PERFORMS a different op (`get`) that
           reads the advanced state. The two-hole general-one-shot refold re-reduces `C[w]` under the handler
           re-seeded with `w` (the resume's new-state), so `get` reads 42. The `do` yields its LAST item, so
           `set`'s result is discarded → the value is `get` = 42. Witnesses the deferred-resume-thunk /
           cross-activation resume the DES scheduler builds on (its `sleep` fast-forwards the clock to the
           wake instant via exactly this shape, then observes it with `now`). Distinct from the escaping-`k`
           reify (that one has an explicit `k` binder); here the escape is a `resume`-bearing lambda.")
  (input
    (do
      (effect A (op set (-> Int64 Int64)) (op get (-> Unit Int64)))
      (def (run-thunk thunk) (thunk unit))
      (def
        (main)
        (handle
          A
          0
          ((get (u) s (resume s s)) (set (w) s (run-thunk (fn (_u) (resume w w)))))
          (do (A.set 42) (A.get))))
      (export main)))
  (output (: 42 Int64)))

(case
  "a deferred resume-thunk STORED IN A SUM and match-extracted through a helper before apply folds"
  (doc
    "E5 step-3 (the DES multi-task scheduler's pqueue store→pop→apply reach). The escaping resume-thunk
           is STORED in a compound (`Box.Box (fn (_u) (resume unit wake))`) and MATCH-EXTRACTED through a
           helper `unbox-apply(b) = match b ((Box.Box th) (th unit))` before being applied — exactly how a
           real scheduler stores (waketime, k) in a pqueue, pops the min via match, and applies the popped k.
           The `resume` is buried behind the constructor + the match, so the fold's classifiers see no tail
           resume. It is exposed by reducing the arm body: β-reduce the one-shot `unbox-apply` (its param is
           used once — the DES one-shot contract, so the resume-thunk is not duplicated), then case-of-known-
           constructor fold the `(match (Box.Box (fn..)) ((Box.Box th) (th unit)))` (a SumPayload-aware
           substitution — the pattern binder resolves to a payload-read, not a plain reference), then
           β-reduce the exposed `((fn (_u) (resume unit wake)) unit)` to `(resume unit wake)` — the resume-in-
           place form the fold serves. The `now` arm reads the wake-seeded clock → 5s. Distinct from the
           escaping-`k` reify (that has an explicit `k` binder + is left un-reduced); this 4-part arm's
           resume is reached through the store→match round-trip.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (type Box (Box (-> Unit Instant)))
      (def (unbox-apply (: b Box)) (match b ((Box.Box th) (th unit))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep (wake) s (unbox-apply (Box.Box (fn (_u) (resume unit wake))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "a deferred resume-thunk stored in a MULTI-PAYLOAD pqueue entry and tuple-match-extracted folds"
  (doc
    "E5 step-3, the DES multi-task scheduler's REAL pqueue shape. The prior case stored the resume-thunk
           in a single-payload `Box`; a genuine pqueue entry carries `(waketime, k, rest)` — a MULTI-payload
           node `(PQCons (Tuple Instant KBox PQ))` popped by a tuple pattern `(PQCons (tuple wake kb rest))`,
           then the popped `kb` (a `KBox`) unboxed and applied. The resume is buried behind TWO constructors
           (the pqueue node + the KBox) and a tuple destructure. The fold reaches it in two steps its
           classifier loop drives: case-of-known-constructor fold the outer `(match (PQCons (tuple …)) ((PQCons
           (tuple wake kb rest)) …))` — a MULTI-payload SumPayload-aware substitution, each tuple binder
           `wake`/`kb`/`rest` resolving to a `[Payload, Elem(i)]` payload-read projected into the visible
           tuple's element `i` — exposing `(match kb ((KBox.KBox k) (k unit)))`, then the inner single-payload
           fold + β-reduce as before. Distinct from the single-payload `Box` case above: the payload is a
           TUPLE destructured by the pattern, so the binders read `[Payload, Elem(i)]` rather than `[Payload]`.
           The `now` arm reads the wake-seeded clock → 5s. This is the pqueue pop-min-apply shape whole.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def
        (pop-apply (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep
              (wake)
              s
              (pop-apply
                (PQ.PQCons #tuple(wake (KBox.KBox (fn (_u) (resume unit wake))) (PQ.PQNil ()))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "a continuation filed through a RECURSIVE pqueue insert (base arm) declines cleanly, never miscompiles"
  (doc
    "The DES inc-4 recursive-insert reach. A boxed continuation `(KBox (fn (_u) (resume unit wake)))`
           is filed into a pqueue via a RECURSIVE sorted-insert `pins`, then popped + applied by `sched-step`.
           The direct-entry companion already folds to 5e9 (the multi-payload pqueue case above); this variant
           differs only in that the entry flows through `pins`'s recursion before the pop. The concrete arg
           `(PQ.PQNil ())` selects `pins`'s NON-recursive base arm, so the stored KBox survives to the pop —
           its oracle is the same 5e9. Today the deferred-resume fold refuses to symbolically evaluate a
           recursive helper, so this DECLINES cleanly (a folds-or-declines-never-miscompiles guard); when the
           base-arm unfold lands it must fold to exactly 5e9.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def
        (pins (: q PQ) (: t Instant) (: kb KBox))
        (match
          q
          ((PQ.PQNil _) (PQ.PQCons #tuple(t kb (PQ.PQNil ()))))
          ((PQ.PQCons #tuple(ht hk r))
            (if
              (before? t ht)
              (PQ.PQCons #tuple(t kb (PQ.PQCons #tuple(ht hk r))))
              (PQ.PQCons #tuple(ht hk (pins r t kb)))))))
      (def
        (sched-step (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep
              (wake)
              s
              (sched-step (pins (PQ.PQNil ()) wake (KBox.KBox (fn (_u) (resume unit wake)))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (call main)
  (output (: 5000000000 UInt64)))

(case
  "a GENUINELY-recursive pqueue insert (recursion taken) declines cleanly, never folds the wrong entry"
  (doc
    "The complement of the base-arm pin: a genuinely-recursive insert where the recursion is actually
           TAKEN. `pins` is handed a NON-empty queue whose head has an EARLIER waketime (Instant 1) than the
           inserted continuation's (wake = 5e9), so `before? wake 1` is false and the `(pins r t kb)` self-call
           fires — the entry is placed AFTER the head. The recursion-unfold accept guard must REFUSE this: a
           one-level unfold would drop the remaining insertions and pop the WRONG entry (a miscompile), so the
           fold declines cleanly. It must NEVER fold to the later inserted 5e9 entry; if a future increment
           folds it, it must pop the earlier head (waketime 1) and read 1.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def
        (pins (: q PQ) (: t Instant) (: kb KBox))
        (match
          q
          ((PQ.PQNil _) (PQ.PQCons #tuple(t kb (PQ.PQNil ()))))
          ((PQ.PQCons #tuple(ht hk r))
            (if
              (before? t ht)
              (PQ.PQCons #tuple(t kb (PQ.PQCons #tuple(ht hk r))))
              (PQ.PQCons #tuple(ht hk (pins r t kb)))))))
      (def
        (sched-step (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep
              (wake)
              s
              (sched-step
                (pins
                  (PQ.PQCons
                    #tuple((Instant.Instant 1)
                      (KBox.KBox (fn (_z) (resume unit (Instant.Instant 1))))
                      (PQ.PQNil ())))
                  wake
                  (KBox.KBox (fn (_u) (resume unit wake)))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (call main)
  (output (: 1 UInt64)))

(case
  "a two-entry directly-built pqueue pops the HEAD continuation, not the tail"
  (doc
    "The multi-TASK scheduler shape: the pqueue holds TWO entries and the pop must bind the HEAD
           entry's continuation, ignoring the tail. `sched-step` matches `(PQCons (tuple wake kb rest))`
           and applies the head's `kb` — a `KBox` boxing `(fn (_u) (resume unit wake))` — while the tail
           entry (waketime 9, its own resume-thunk) is never reached. Seeded 0, `(Sim.sleep 5e9)` files the
           head thunk that resumes the clock to the head's wake 5e9; `(Sim.now)` then reads 5e9. If the pop
           bound the tail instead, the clock would read 9. Pins head-entry selection on a multi-entry
           directly-built pqueue (the pop-min shape whole), distinct from the single-entry pop-apply above.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def
        (sched-step (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep
              (wake)
              s
              (sched-step
                (PQ.PQCons
                  #tuple(wake
                    (KBox.KBox (fn (_u) (resume unit wake)))
                    (PQ.PQCons
                      #tuple((Instant.Instant 9)
                        (KBox.KBox (fn (_v) (resume unit (Instant.Instant 9))))
                        (PQ.PQNil ()))))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "a deferred resume-thunk filed by a non-recursive helper then popped folds"
  (doc
    "The pqueue entry is built by a NON-RECURSIVE helper `mk1` rather than inline: `(sched-step (mk1
           wake (KBox.KBox (fn (_u) (resume unit wake)))))`. The fold reduces the outer `sched-step` arm to
           `(match (mk1 wake kb) …)`, then must reduce the nested `(mk1 …)` scrutinee to its `(PQCons (tuple
           wake kb PQNil))` body so the case-of-known-constructor pop exposes the boxed continuation — a
           regression pin for the nested-helper-call-in-scrutinee reduction (before it, the pop's binders
           resolved to Poison and the fold declined). The popped continuation resumes the wake-seeded clock
           → 5e9.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def (mk1 (: t Instant) (: kb KBox)) (PQ.PQCons #tuple(t kb (PQ.PQNil ()))))
      (def
        (sched-step (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep (wake) s (sched-step (mk1 wake (KBox.KBox (fn (_u) (resume unit wake)))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "a multi-arg non-recursive helper filing a resume-thunk folds, substituting all its args"
  (doc
    "The multi-argument helper twin of the pqueue-entry-via-helper case: `mk2` takes a pure `base`
           arg (unused in the built entry) plus the `(t, kb)` pair. `(sched-step (mk2 (Instant 7) wake
           (KBox …)))` — the arm reduction must substitute ALL of mk2's args (including the unused pure
           `base`) when reducing the nested scrutinee, so the pop exposes the boxed continuation; the
           resume seeds the clock to the wake → 5e9. Pins that a multi-arg (some-unused) helper folds
           through the same nested-scrutinee reduction as the single-arg one.")
  (input
    (do
      (type Instant (Instant UInt64))
      (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
      (type KBox (KBox (-> Unit Unit)))
      (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
      (def (mk2 (: base Instant) (: t Instant) (: kb KBox)) (PQ.PQCons #tuple(t kb (PQ.PQNil ()))))
      (def
        (sched-step (: q PQ))
        (match
          q
          ((PQ.PQNil _) unit)
          ((PQ.PQCons #tuple(wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
      (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
      (def
        (main)
        (handle
          Sim
          (Instant.Instant 0)
          ((now (u) s (resume s s))
            (sleep
              (wake)
              s
              (sched-step (mk2 (Instant.Instant 7) wake (KBox.KBox (fn (_u) (resume unit wake)))))))
          (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
      (export main)))
  (output (: 5000000000 Int64)))

(case
  "a performing closure passed to a function that applies it UNDER a handler is homed at the apply site"
  (doc
    "The `handler runs a passed-in closure` idiom: `with-seed(body) = handle Rand … (body unit)` runs
           its `body` PARAM under the `Rand` handler, and `main` passes `(fn (u) (Rand.roll))`. The lambda's
           `Rand.roll` is homed at the APPLICATION site (inside `with-seed`, under the handler), not at its
           definition site in `main`. The no-home check computes, per callee param, the effects the callee
           applies it under (here `Rand`), and homes a lambda argument's performs against THAT set — so this
           compiles rather than a false CDZ0401. Distinct from the escaping-closure-BODY-performs reject
           (`04-capabilities` \"an ungranted effect hidden in a closure passed to a HOF is still rejected\":
           `apply-fn = (body unit)` with NO handler adds no grant, so an ungranted effect there STAYS
           CDZ0401). The `roll` arm resumes with the seed 5 → `(body unit)` reads 5. Regression pin for the
           apply-site-homing analysis (root-caused from v-cad's passed-closure-under-handler codegen bug).")
  (input
    (do
      (effect Rand (op roll (-> Unit Int64)))
      (def
        (with-seed (: body (-> Unit Int64)))
        (handle Rand 5 ((roll (u) s (resume s s))) (body unit)))
      (def (main) (with-seed (fn (u) (Rand.roll))))
      (export main)))
  (output (: 5 Int64)))

(case
  "a performing closure homed TRANSITIVELY through a pass-through function is not falsely rejected"
  (doc
    "Apply-site homing propagated ONE call deeper: `outer(b) = inner(b)` is a PASS-THROUGH — it hands
           its `b` param onward to `inner`, which applies it under `handle R`. `main` passes `(fn (u)
           (R.roll))` to `outer`. The lambda's `R.roll` is homed where `inner` applies the param (under the
           handler), so `outer`'s `b` inherits `inner`'s granted effect `{R}` — the program compiles rather
           than a false CDZ0401. The no-home analysis, computing per callee param the effects it is applied
           under, follows a param passed as an argument to a known sub-callee and inherits the sub-callee's
           extra-handled set. SOUNDNESS twin: if the pass-through's target applied the param under NO handler,
           nothing propagates and an ungranted effect STAYS rejected (`04-capabilities`). The `roll` arm
           resumes with the seed 5.")
  (input
    (do
      (effect R (op roll (-> Unit Int64)))
      (def (inner (: b (-> Unit Int64))) (handle R 5 ((roll (u) s (resume s s))) (b unit)))
      (def (outer (: b (-> Unit Int64))) (inner b))
      (def (main) (outer (fn (u) (R.roll))))
      (export main)))
  (output (: 5 Int64)))

(case
  "an apply-site-homed lambda's perform result composes in the caller's arithmetic"
  (doc
    "The apply-site-homing case above pins the bare homed value; this pins that the homed lambda's
           perform result is a FIRST-CLASS value the caller composes with. `with-seed` applies its `body`
           param under `handle Rand` and adds 100 to the result — `(+ (body unit) 100)`; `main` passes
           `(fn (u) (Rand.roll))`, whose `Rand.roll` is homed at the apply site and resumes the seed 5, so
           the caller computes 5 + 100 = 105. (The bare-value form is pinned above; this adds the
           result-composition face.)")
  (input
    (do
      (effect Rand (op roll (-> Unit Int64)))
      (def
        (with-seed (: body (-> Unit Int64)))
        (handle Rand 5 ((roll (u) s (resume s s))) (+ (body unit) 100)))
      (def (main) (with-seed (fn (u) (Rand.roll))))
      (export main)))
  (output (: 105 Int64)))

(case
  "a transitively-homed lambda's perform result composes in the pass-through caller"
  (doc
    "The transitive-homing companion of the result-composition case: through a PASS-THROUGH
           (`outer(b) = inner(b)`, `inner(b) = handle R … (+ (b unit) 100)`), the homed lambda's perform
           result flows into the caller's `+ 100`. `main` passes `(fn (u) (R.roll))`; the homed `R.roll`
           resumes seed 5, so 5 + 100 = 105. Pins that transitive apply-site homing yields a first-class
           value the pass-through caller composes with, not just a bare result.")
  (input
    (do
      (effect R (op roll (-> Unit Int64)))
      (def (inner (: b (-> Unit Int64))) (handle R 5 ((roll (u) s (resume s s))) (+ (b unit) 100)))
      (def (outer (: b (-> Unit Int64))) (inner b))
      (def (main) (outer (fn (u) (R.roll))))
      (export main)))
  (output (: 105 Int64)))

(case
  "a performing closure called TWICE observes the state advance between its calls"
  (doc
    "The state-threading face of the performing closure (the homing pins above call the closure
           once): `f = (fn (u) (Ctr.next unit))` is let-bound under the handler and applied TWICE in one
           expression — the first call reads the seed `n` and threads `n+1`, the second reads `n+1`.
           Encodes `10·first + second` = 10n + (n+1) → 34 at n = 3. Pins that each APPLICATION of a
           performing closure is a fresh perform against the CURRENT handler state (a closure that captured
           its perform's result at creation, or replayed the first discharge, would give 33).")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Ctr
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((f (fn ((: u Unit)) (Ctr.next unit)))) (+ (* 10 (f unit)) (f unit)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 34 Int64)))

(case
  "a performing closure applied twice DIRECTLY in the handle body threads state per call"
  (doc
    "The arg-passing face of the performing closure applied twice: `g = (fn (n) (Src.read n))` is
           let-bound under `handle Src 100` whose arm resumes `(+ s n)` as BOTH the op value and the next
           state. Called directly in `(+ (g 1) (g 2))`: `g 1` reads s=100 → 100+1 = 101 (state → 101),
           `g 2` reads s=101 → 101+2 = 103; 101 + 103 = 204. Each application is a fresh perform against the
           CURRENT handler state (a closure replaying its first discharge would give a different value).")
  (input
    (do
      (effect Src (op read (-> Int64 Int64)))
      (def
        (main)
        (handle
          Src
          100
          ((read (n) s (resume (+ s n) (+ s n))))
          (let ((g (fn ((: n Int64)) (Src.read n)))) (+ (g 1) (g 2)))))
      (export main)))
  (call main)
  (output (: 204 Int64)))

(case
  "a performing closure passed to a helper that applies it never miscompiles (folds to 204 or declines)"
  (doc
    "The cross-function face of the direct case above: the SAME performing closure `g` is passed into
           a helper `(apply-twice g) = (+ (g 1) (g 2))` that applies it — so the perform crosses an INDIRECT
           (cross-function) call boundary. If it folds it MUST equal the direct form's value (204); it must
           never yield a WRONG value. A generation that cannot yet thread the perform across the helper
           boundary DECLINES cleanly (scored todo) rather than miscompiling — reject-don't-miscompile
           (self-hosting-and-bootstrap.md). Pinning the sound value 204 makes any future fold to a different
           value a caught miscompile, a stronger guard than omitting the case.")
  (input
    (do
      (effect Src (op read (-> Int64 Int64)))
      (def (apply-twice (: g (-> Int64 Int64))) (+ (g 1) (g 2)))
      (def
        (main)
        (handle
          Src
          100
          ((read (n) s (resume (+ s n) (+ s n))))
          (let ((g (fn ((: n Int64)) (Src.read n)))) (apply-twice g))))
      (export main)))
  (call main)
  (output (: 204 Int64)))

(case
  "a matching-width handler state folds across two sequential performs"
  (doc
    "Two sequential performs against a handler whose state advances by a RUNTIME amount: seed 10,
           the op `next` resumes the current state `s` and threads `(+ s x)` as the next state.
           `(do (def a (Src.next)) (def b (Src.next)) (+ a b))` — `a` reads 10, the state advances to
           `10 + x`, `b` reads `10 + x`; with x = 5, a = 10, b = 15 → 25. The state slot, op result, and
           next-state expression are all Int64 (matching width), so the two-perform fold threads cleanly.
           (The width-MISMATCHED companion — a narrow-int state seeded into an Int64 op result — declines
           cleanly rather than emitting invalid wasm, the compiler-side safe floor for that not-yet-
           reducible case.)")
  (input
    (do
      (effect Src (op next (-> Unit Int64)))
      (def
        (main (: x Int64))
        (handle
          Src
          10
          ((next (u) s (resume s (+ s x))))
          (do (def a (Src.next)) (def b (Src.next)) (+ a b))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 25 Int64)))

(case
  "a stateful handler threads its state across three sequential performs, the do yielding the last"
  (doc
    "A handler that FOLDS state — `(resume s (+ s 1))` hands back the current state and threads `s+1`
           forward — over THREE sequential performs in a `do`. Seed 0: the three `Fresh.next` reads see
           0, 1, 2, and the `do` yields its last statement's value → 2. Pins that the threaded state
           advances once per perform and the do-block's value is the final perform's result (the earlier
           two are sequenced for their state effect and discarded).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (Fresh.next) (Fresh.next) (Fresh.next))))
      (export main)))
  (output (: 2 Int64)))

(case
  "a cross-function perform is discharged by the caller's handler"
  (doc
    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent: a perform in a
           CALLEE `gen` is discharged by the handler enclosing `gen`'s CALL, `(handle … (gen))` — the fold
           inlines `gen` into the handled region so its perform `(Bump.by 41)` resolves to the arm, which
           resumes `(+ 41 1)` = 42. A function performing an operation its caller discharges is well-formed
           (its home is the caller's handler, not itself), so `gen` is not independently faulted.")
  (input
    (do
      (effect Bump (op by (-> Int64 Int64)))
      (def (gen) (Bump.by 41))
      (def (main) (handle Bump unit ((by (n) s (resume (+ n 1) s))) (gen)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a PURE closure iterated by a recursive combinator composes with a perform in the same body"
  (doc
    "The effects-adjacent face of the iterate combinator (09-functions pins it pure): under a
           handler, the body BOTH performs (`Ctr.next` reads the seed 0 and threads 1) AND runs a
           recursive `times` combinator over a PURE closure `(fn (u) 5)` a RUNTIME number of times —
           `(+ (Ctr.next unit) (times (fn 5) n 0))` at n=3 is 0 + 15 = 15. The combinator's fn-param
           application must not be mistaken for a performing call (no false CDZ0401 on the pure closure,
           no spurious state advance from its iterations), and the sibling perform must still thread the
           handler state. (A PERFORMING closure through the same combinator still declines — the homing
           analysis grants effects where the callee applies its param under a handler, and here the
           handler sits at the combinator's CALL site, not inside it — so this pins the pure half.)")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (times (: f (-> Unit Int64)) (: n Int64) (: acc Int64))
        (if (< n 1) acc (times f (- n 1) (+ acc (f unit)))))
      (def
        (main (: n Int64))
        (handle
          Ctr
          0
          ((next (u) s (resume s (+ s 1))))
          (+ (Ctr.next unit) (times (fn ((: u Unit)) 5) n 0))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 15 Int64))
  (live-objects known-leak))

(case
  "a 100k-iteration pure tail loop under a handler runs in constant stack"
  (doc
    "The SCALE face of loops-under-handlers (existing loop pins are ≤33 deep): a 100000-iteration
           tail-recursive accumulator inside a handle body, plus one perform reading the seed. The
           handler context must not break tail-call frame reuse — a lowering that let the handler frame
           capture the loop (or reified a frame per iteration) overflows long before 100k. 0 + 100000.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def (loop (: n Int64) (: acc Int64)) (if (< n 1) acc (loop (- n 1) (+ acc 1))))
      (def
        (main (: n Int64))
        (handle Ctr 0 ((next (u) s (resume s (+ s 1)))) (+ (Ctr.next unit) (loop n 0))))
      (export main)))
  (call main (: 100000 Int64))
  (output (: 100000 Int64)))

(case
  "a PERFORMING tail loop of 10000 iterations threads state in constant space"
  (doc
    "The sharper scale face: every iteration PERFORMS (`(+ acc (Ctr.next unit))`), so the
           tail-resumptive arm discharges 10000 performs — each must resume without reifying a
           continuation (10k reified frames would exhaust memory/stack). The state threads 0..9999,
           summing to 49995000. The constant-space guarantee of the E4 tail-resumptive lowering at a
           scale the ≤33-deep pins cannot witness.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def (go (: n Int64) (: acc Int64)) (if (< n 1) acc (go (- n 1) (+ acc (Ctr.next unit)))))
      (def (main (: n Int64)) (handle Ctr 0 ((next (u) s (resume s (+ s 1)))) (go n 0)))
      (export main)))
  (call main (: 10000 Int64))
  (output (: 49995000 Int64)))

(case
  "an OBSERVED performing tail loop of 20k iterations keeps constant stack (repackage-tail-call)"
  (doc
    "The observed-out-state scale face (breaker #16). `grow` is a source-tail-recursive PERFORMER whose
           out-state is OBSERVED after the recursion (`(+ g (Acc.size))`), so it is multi-value-upgraded to
           return `(value, out-state)` and its tail self-call is rewritten into `(let ((t (grow …))) (tuple
           (. t 0) (. t 1)))` — the call moves into the let INIT and the body re-packages `t`. That
           identity-repackage IS a tail call; the wasm loop transform must recognize it
           (multivalue_repackage_tail_call) or the upgraded def recurses one wasm frame per iteration and
           traps `call stack exhausted` (observed ~5-8k). 20000 iterations is well beyond that naive-frame
           limit, so passing PROVES the loop transform fired (constant stack). Each `push` advances the state
           by 1 (resume s ; s+1), so after 20k pushes the state is 20000; `grow` returns 0 at the base, and
           `(+ g (Acc.size))` = 0 + 20000 = 20000.")
  (input
    (do
      (effect Acc (op push (-> Int64 Int64)) (op size (-> Int64)))
      (def (grow (: n Int64)) (if (< n 1) 0 (match (Acc.push n) (_ (grow (- n 1))))))
      (def
        (main)
        (handle
          Acc
          0
          ((push (v) s (resume s (+ s 1))) (size () s (resume s s)))
          (let ((g (grow 20000))) (+ g (Acc.size)))))
      (export main)))
  (call main)
  (output (: 20000 Int64)))

(case
  "an effect op RESUMED with a slice-view Bytes crosses the arm boundary intact"
  (doc
    "A heap VIEW as the resume value: the arm builds a `Bytes.slice` window and resumes with it;
           the body indexes the escaped view (byte 0 of (20,30) = 20, +22 = 42). The view's re-based
           coordinates must survive the continuation crossing — composing the slice-view machinery with
           the effects lowering (scalars/strings/sums as resume values are pinned; a VIEW is the shape
           a zero-copy parser hands back).")
  (input
    (do
      (effect Src (op read (-> Unit Bytes)))
      (def
        (main (: a Int64))
        (handle
          Src
          0
          ((read
              (u)
              s
              (match
                (Bytes.slice (Bytes.of #list(9 20 30 8)) 1 2)
                ((Some w) (resume w s))
                ((None x) (resume (Bytes.of #list()) s)))))
          (+ (match (Bytes.at (Src.read unit) 0) ((Some v) v) ((None u) -1)) a)))
      (export main)))
  (call main (: 22 Int64))
  (output (: 42 Int64)))

(case
  "an arm that matches over a COMPUTATION on the op-param and resumes a match binder folds (nv1e/nvC)"
  (doc
    "The op-param twin of the slice-view case above: the arm's match scrutinee is a `Bytes.slice`
           over the OP-PARAM `b` (not a constant), and the resume value `w` is the match binder bound by
           that scrutinee. The pure-one-hole refold β-reduces the arm body substituting `b := (the pure
           copy of) the op arg`; the binder `w` carries a `SumPayload` reading the scrutinee, which MENTIONS
           the substituted `b`. If the fold SHARED `w` as a capture, its payload would keep reading the
           original scrutinee's raw `b` — an op-param with no slot in the folded body → the 'parameter
           reference has no local slot' backend decline (nv1e minimized to nvC). The fix COPIES such a
           binder so it re-resolves against the substituted-scrutinee copy. Here `b = (20,30,40)`, `slice b
           1 2 = (30,40)`, `Bytes.len = 2`, `+ a(7) = 9`. The control `(cut (b) t (resume b t))` — hand `b`
           straight back, no match — always folded; this pins the match-over-computed-op-param shape.")
  (input
    (do
      (effect B (op cut (-> Bytes Bytes)))
      (def
        (main (: a Int64))
        (handle
          B
          0
          ((cut
              (b)
              t
              (match
                (Bytes.slice b 1 2)
                ((Some w) (resume w t))
                ((None x) (resume (Bytes.of #list()) t)))))
          (+ (Bytes.len (B.cut (Bytes.of #list(20 30 40)))) a)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 9 Int64)))

(case
  "an effect op RESUMED with a constructed Ast node crosses the arm boundary and matches in the body"
  (doc
    "An AST as the resume value — the first Ast crossing an effect boundary in the corpus: the arm
           constructs `(Ast.Int (BigInt.of x))` from the op param and resumes with it; the body pattern-matches
           the node back out and extracts the boxed BigInt payload (25N). Ast is a recursive sum with a
           BigInt-boxed leaf, a representation the scalar/string/sum/view resume-value pins don't reach — the
           template-provider idiom (a handler that answers with syntax) rests on this crossing.")
  (input
    (do
      (effect Tmpl (op get (-> Int64 Ast)))
      (def
        (main (: n Int64))
        (handle
          Tmpl
          0
          ((get (x) s (resume (Ast.Int (BigInt.of x)) s)))
          (match (Tmpl.get n) ((Ast.Int b) b) (_ -1N))))
      (export main)))
  (call main (: 25 Int64))
  (output (: 25 BigInt))
  (live-objects known-leak))

(case
  "an effect op resumed with a whole MAP threads the CHAMP through the continuation"
  (doc
    "A collection HANDLE as the resume value: the arm resumes with a 2-entry map, and the body
           looks it up at the boundary parameter — k=2 → 20, k=9 → None → -1. The CHAMP handle rides the
           continuation like any value; the body's descent runs on the arm-built trie. (Map-STATE
           handlers are pinned nearby; this is the map-as-RESULT face.)")
  (input
    (do
      (effect Env (op vars (-> Unit (Map Int64 Int64))))
      (def
        (main (: k Int64))
        (handle
          Env
          0
          ((vars (u) s (resume #map((= 1 10) (= 2 20)) s)))
          (match (Map.lookup (Env.vars unit) k) ((Some v) v) ((None u) -1))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 20 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64)))

(case
  "a LIST OF SUMS resumed from a handler feeds an event-sourcing fold in the body"
  (doc
    "The heap-of-variants resume: the arm resumes with `(list (Add n) (Reset) (Add 40) (Add 2))` —
           a list whose ELEMENTS are sum values with mixed payload/nullary variants — and the body runs
           the apply-events fold over it (the Reset discards the runtime n; 40+2 = 42 regardless).
           Composes the sum-list construction in an ARM, the crossing of variant-tagged heap elements
           through the continuation, and the per-variant dispatch fold in the body — the config-provider
           idiom (a handler supplying a program's event stream).")
  (input
    (do
      (type Ev (Add Int64) (Reset))
      (effect Src (op events (-> Unit (List Ev))))
      (def (apply-ev (: acc Int64) (: e Ev)) (match e ((Add v) (+ acc v)) ((Reset) 0)))
      (def
        (run (: evs (List Ev)) (: acc Int64))
        (match evs (#list() acc) (#list(h (.. t)) (run t (apply-ev acc h)))))
      (def
        (main (: n Int64))
        (handle
          Src
          0
          ((events (u) s (resume #list((Add n) (Reset) (Add 40) (Add 2)) s)))
          (run (Src.events unit) 0)))
      (export main)))
  (call main (: 999 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a handler arm RECURSES through a named helper before resuming"
  (doc
    "The arm-calls-a-def face: `tally`'s arm computes `(triangle v 0)` — a RECURSIVE tail loop over
           the op argument — before resuming with its result (4 → 10, 10 → 55). The arm body is not a
           plain expression context: the recursive call runs under the handler's dispatch frame, and its
           result feeds the resume. An arm lowering that couldn't re-enter user code (or that confused
           the helper's frames with the handler's) breaks the larger input.")
  (input
    (do
      (effect Sum (op tally (-> Int64 Int64)))
      (def (triangle (: n Int64) (: acc Int64)) (if (< n 1) acc (triangle (- n 1) (+ acc n))))
      (def
        (main (: n Int64))
        (handle Sum 0 ((tally (v) s (resume (triangle v 0) s))) (Sum.tally n)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 10 Int64))
  (call main (: 10 Int64))
  (output (: 55 Int64)))

(case
  "an arm's NEXT-STATE expression saturates via a conditional on the current state"
  (doc
    "The state-transition-function face: the arm's next-state is `(if (>= s 3) 3 (+ s 1))` — a
           CLAMP, not a plain increment. Four bumps from seed 0 read 0,1,2,3 with the final read at the
           ceiling (3); from seed 5 the first transition already clamps (5 → 3, reads 5 then 3,3,3 —
           final 3). Pins that the next-state slot accepts arbitrary expressions over the current state
           (the existing arms all use unconditional arithmetic) and that the transition applies AFTER the
           read, per resume.")
  (input
    (do
      (effect Clamp (op bump (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Clamp
          n
          ((bump (u) s (resume s (if (>= s 3) 3 (+ s 1)))))
          (do (Clamp.bump unit) (Clamp.bump unit) (Clamp.bump unit) (Clamp.bump unit))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (call main (: 5 Int64))
  (output (: 3 Int64)))

(case
  "a ctl-style arm applying its continuation inside a match scrutinee resolves and folds"
  (doc
    "The continuation binder `k` of a `ctl`-style arm must be in scope everywhere in the arm body,
           including inside a MATCH scrutinee. `(flip () s k (match (k 10) (z (* z 2))))` applies `k`
           lexically as the scrutinee of a match; `(k 10)` returns 10 into the delimited context, the match
           binds it to `z` and doubles it → 20. Regression pin: `k` used inside a match scrutinee previously
           reported a spurious CDZ0101 (the continuation binder occurrence was not recognized as a binder on
           that resolution path), while `k` applied directly in an operator operand worked.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip () s k (match (k 10) (z (* z 2))))) (Amb.flip)))
      (export main)))
  (output (: 20 Int64)))

(case
  "a ctl-style arm that applies its continuation lexically folds through the delimited context"
  (doc
    "The E5 within-activation continuation surface: a 5-part handler arm `(flip () s k body)` binds the
           delimited continuation `k` as a value and APPLIES it as `(k v)`. When `k` is applied lexically
           (never stored or passed on), `(k v)` returns into the delimited context — semantically identical
           to `(resume v)`. Over the whole-body perform `(Amb.flip)`, the continuation is `C = (+ □ 1)`, so
           `(k 10)` = `C[10]` = `(+ 10 1)` = 11. Witnesses capabilities-and-effects.md continuation semantics
           (a handler receives the continuation and resumes it) for the lexical `ctl` surface, distinct from
           the implicit-continuation `resume`. A `k` that ESCAPES (stored/resumed later) is a separate,
           later increment; this pins the within-activation case.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip () s k (+ (k 10) 1))) (Amb.flip)))
      (export main)))
  (output (: 11 Int64)))

(case
  "a ctl-style arm that reads the STATE binder AROUND its continuation application folds"
  (doc
    "The state-referencing companion of the lexical-`ctl` fold above: the 5-part arm body references
           the STATE binder `s` (and could reference an op parameter) in a position OUTSIDE the `(k v)`
           application — `(+ s (k x))`, where the `+`'s LEFT operand is the state and the RIGHT is the
           continuation result. When `k` is applied lexically, `(k v)` = `(resume v s)`, so the arm becomes
           `(+ s (resume x s))` — a NON-tail resume with a live `s`/`x` sibling. Seeded 100 over `(G.y 5)`:
           `x` = 5, `C = □` (the whole body is the perform), so `(k 5)` = 5 and the arm value is `(+ 100 5)`
           = 105. Pins that the lexical-`ctl`→`resume` rewrite preserves the arm's state/param binder
           resolution for references OUTSIDE the `(k v)` call — the rewrite rebuilds the arm body, and
           without pinning those sibling references first they were re-pushed UNPINNED into a detached tree,
           lost their parent-walk to the arm binder, and leaked `unbound name s`/`x` at lowering (a CDZ0101
           on a valid program — strictly worse than a clean decline). A reference INSIDE the `(k v)` argument
           (`(k (+ x s))`) was spliced verbatim and always folded; this is the sibling-position face.")
  (input
    (do
      (effect G (op y (-> Int64 Int64)))
      (def (main) (handle G 100 ((y (x) s k (+ s (k x)))) (G.y 5)))
      (export main)))
  (output (: 105 Int64)))

(case
  "a ctl-style arm that LET-BINDS its continuation result then reads the state binder folds"
  (doc
    "The let/do-bound-continuation companion of the two folds above. The arm binds the continuation
           result in a local `let` (or `do`-def) and reads it alongside the STATE binder in the let body —
           `(let ((r (k x))) (+ r s))`. When `k` is applied lexically, `(k v)` = `(resume v s)`, so the arm
           becomes `(let ((r (resume x s))) (+ r s))` — a NON-tail resume BOUND IN A LET-INIT, with the
           body-local `r` and the arm's state `s` both live. Seeded 100 over `(G.y 5)`: `r` = `(k 5)` = 5,
           `(+ r s)` = `(+ 5 100)` = 105. Pins that the lexical-`ctl`→`resume` rewrite handles a `(k v)`
           bound in a `let`/`do`-def INIT (not just an operand): the rewrite must pin the arm's STATE/param
           binder references (which resolve OUTSIDE the rebuilt tree) while leaving the BODY-LOCAL binder `r`
           unpinned (so it re-resolves to the REWRITTEN local init) — a blunt whole-body pin kept `r` pointing
           at the orphaned original `(k x)` init (`value is not applyable`), a blunt no-pin re-leaked the
           state binder. The explicit-resume twin `(let ((r (resume x s))) (+ r s))` already folded; this
           makes the lexical-`k` spelling match it.")
  (input
    (do
      (effect G (op y (-> Int64 Int64)))
      (def (main) (handle G 100 ((y (x) s k (let ((r (k x))) (+ r s)))) (G.y 5)))
      (export main)))
  (output (: 105 Int64)))

(case
  "an abortive handler arm performed with a RUNTIME argument abandons the computation and returns it"
  (doc
    "The runtime-argument companion of the constant-abort case. An abortive arm `(bail (n) s n)` never
           resumes, so performing `(Bail.bail k)` — with `k` a RUNTIME parameter, not a constant — abandons
           the enclosing `(+ 1 …)` and makes the arm value (the op argument `k`) the handle's value. Reading
           `run(7)` → 7 (the `+ 1` is discarded; the abort returns the runtime k). Witnesses that the abort
           value's type is grounded from the runtime perform argument (a reference to the enclosing param),
           so the handle result has a machine representation on both backends — a regression against the
           abort-value orphan reading its free reference unbound (a wasm-declines / rust-computes split).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (run (: k Int64)) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail k))))
      (def (main) (run 7))
      (export main)))
  (output (: 7 Int64)))

(case
  "a handle body reads an enclosing function parameter beside a resuming perform"
  (doc
    "A handle body is not closed — it may read a free variable up the enclosing lexical chain.
           `(+ x (Get.get 0))` under a handler that resumes 5 is `x + 5`; with x = 10 → 15. Pins that the
           fold's rewritten body re-anchors UNDER the original `handle` node so the free `x` still reaches
           `main`'s parameter binder rather than a spurious CDZ0101 (before the reparent fix, ANY handle
           body referencing an enclosing function parameter failed to compile).")
  (input
    (do
      (effect Get (op get (-> Int64 Int64)))
      (def (main (: x Int64)) (handle Get 0 ((get (n) s (resume 5 s))) (+ x (Get.get 0))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 15 Int64)))

(case
  "a runtime-conditioned abortive branch and its fall-through both read an enclosing parameter"
  (doc
    "Composes the branch-tail abort with a free-variable read up the enclosing chain: `(if (< x 5)
           (Bail.bail 7) x)` under an abortive Bail handler — the true branch aborts to the arm value 7,
           the false branch reads the enclosing parameter `x`. x = 3 (< 5) → 7 (abort); x = 9 → 9 (fall
           through). Exercises the reparent fix (free `x`) together with the branch-tail abort threading.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main (: x Int64)) (handle Bail 0 ((bail (n) s n)) (if (< x 5) (Bail.bail 7) x)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 7 Int64))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "a nested effectful let inlined into a re-performing body keeps its inner binder"
  (doc
    "A cross-function effectful-let inline: `inner` binds an effect result in a local `let` and reads
           it in a match arm; `outer` binds `inner()` then PERFORMS AGAIN in its body. The fold inlines
           `inner` into `outer`, producing a nested `let` whose inner binder `a` must stay in scope for the
           outer body's continuation (whose threaded out-state references it). Witnesses core-semantics.md
           #Bindings Introduced By A Pattern Are Scoped To Its Branch + the strict left-fold of handler
           state: get()=10 binds a=10, put(a) sets state to 10, inner()=10; then outer adds a second get()
           =10 → 20. A regression against the nested-let inline dropping the inner binder (a spanless
           CDZ0101).")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def (inner) (let ((a (St.get))) (match (St.put a) (_ a))))
      (def (outer) (let ((b (inner))) (+ b (St.get))))
      (def (main) (handle St 10 ((get (u) s (resume s s)) (put (v) s (resume unit v))) (outer)))
      (export main)))
  (output (: 20 Int64)))

(case
  "an inner handle of the SAME delegated effect discharges in-program inside the host block"
  (doc
    "The SHADOW face beside the interpose-and-forward pin: the entrypoint delegates `A`, and INSIDE
           the host block an inner `(handle A 500 …)` re-binds the same effect — the inner perform
           discharges IN-PROGRAM (reads the handler seed 500), while the OUTER perform (outside the
           handle) still delegates to the host (7). Exactly ONE host call. A routing that let the inner
           perform escape to the host would consume a second (unsupplied) response; one that captured the
           outer perform into the handler would read 500 twice. 7 + 500 = 507. (rust-async: todo until
           its host+handle composition lands; wasm + rust pin the pass.)")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (def
        (main (: k Int64))
        (host (A) (+ (A.get unit) (handle A 500 ((get (_u) s (resume s s))) (A.get unit)))))
      (export main)))
  (host-responses (respond a.get (: 7 Int64)))
  (host-calls (call a.get))
  (call main (: 0 Int64))
  (output (: 507 Int64)))

(case
  "a delegated effect performed inside an intra-program handler"
  (doc
    "Witnesses the composition of the two routings (capabilities-and-effects.md
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
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect Scale (op by (-> Int64 Int64)))
      (def
        (main)
        (host (ask) (handle Scale unit ((by (n) s (resume (* n 2) s))) (Scale.by (ask.ask)))))
      (export main)))
  (host-responses (respond ask.ask (: 21 Int64)))
  (host-calls (call ask.ask))
  (output (: 42 Int64)))

(case
  "an in-program handler OVERRIDES an effect's peer binding (the test-mock, no peer call)"
  (doc
    "Witnesses the U-pivot headline (DESIGN-cross-component-interop-rcdzc.md #UNIFY cross-component
           interop WITH EFFECTS): an effect bound to a PEER contract by a top-level `(bind Math
           \"cadenza:math/api\")` directive is normally a peer CALL — but a NEARER in-program `(handle Math
           …)` DISCHARGES it before it escapes, so the peer binding is OVERRIDDEN and no peer/host call is
           made. This is the free test-mock the unification gives: routing precedence is in-program handler
           > peer binding, exactly as an in-program handler beats a `(host …)` delegation. The mock arm
           computes `a + b + 100`, so `(Math.add 2 3)` = 105 — the handler's answer, not the peer's — and
           the empty `(host-calls)` fixture pins that the bound peer is never reached (the effect does not
           escape). Pins that `(handle E …)` is the unit-test override for a peer dependency, reusing the
           complete E0–E5 handler machinery with no peer needed.")
  (input
    (do
      (effect Math (op add (-> Int64 Int64 Int64)))
      (bind Math "cadenza:math/api")
      (def (main) (handle Math 0 ((add (a b) s (resume (+ (+ a b) 100) s))) (Math.add 2 3)))
      (export main)))
  (output (: 105 Int64))
  (host-calls))

(case
  "a bound effect performed with neither a handler nor a host delegation has no home"
  (doc
    "The companion to the override case above, pinning the ROUTING MODEL: `(bind Math
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
  (input
    (do
      (effect Math (op add (-> Int64 Int64 Int64)))
      (bind Math "cadenza:math/api")
      (def (main) (Math.add 2 3))
      (export main)))
  (error CDZ0401))

(case
  "an intra-program handler interposes on a delegated effect, counts it, and forwards to the boundary"
  (doc
    "Witnesses capabilities-and-effects.md #A Handler May Interpose On An Effect An Entrypoint Would
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
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect Count (op tick (-> Unit Unit)))
      (def
        (main)
        (host
          (ask)
          (handle
            Count
            unit
            ((tick (u) s (resume unit s)))
            (handle
              ask
              unit
              ((ask () s (do (Count.tick) (resume (ask.ask) s))))
              (+ (ask.ask) (ask.ask))))))
      (export main)))
  (host-responses (respond ask.ask (: 3 Int64)) (respond ask.ask (: 4 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 7 Int64)))

(case
  "a handler arm interposes on another intra-program effect and forwards"
  (doc
    "The purely INTRA-PROGRAM analogue of the host-forwarding interpose above (no `host` boundary):
           `A`'s arm performs an OUTER effect `Count.tick` (a record-and-continue observation), then resumes.
           The re-performed `Count.tick` resolves against the routers enclosing `A`'s handler — the outer
           `Count` handler — the under-frame discipline, exactly as with host forwarding but discharged
           in-program. `A` is seeded 5 and its arm resumes `s` (=5) unchanged; `Count` seeded 0 threads its
           counter. `(A.a)` evaluates to 5 (the outer `Count.tick` is observed as a side effect, not part of
           the value). Witnesses #A Handler May Interpose On An Effect with BOTH effects intra-program.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect Count (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Count
          0
          ((tick (u) c (resume c (+ c 1))))
          (handle A 5 ((a (u) s (do (Count.tick) (resume s s)))) (A.a))))
      (export main)))
  (output (: 5 Int64)))

(case
  "a nested handler arm whose RESUME-VALUE performs the outer effect threads the advance to the continuation"
  (doc
    "The stateful analogue of the interpose-and-forward case above, and the WORKING boundary of the
           recursive-nested-arm miscompile family (v-effects self-probe): the inner `B` handler's arm resumes
           with a VALUE that performs the OUTER `A` effect — `(step (u) t (resume (A.tick) t))`. `A.tick` reads
           the outer state (10) and advances it (→11); the resume VALUE is that 10, so `(B.step)` = 10. The
           continuation `(A.get)` then reads the ADVANCED 11 → `(+ 10 11)` = 21. Pins that an outer-effect
           advance made INSIDE a nested handler's resume-value threads correctly to a sibling reading the outer
           effect after — the shape folds via the inside-out path when the `B.step` caller is DIRECT (not
           behind a recursive callee). The recursive-caller variant of this shape currently drops the advance
           (a separate known miscompile, merged_nested_ctx merge-skip); this case + its non-recursive-helper
           twin below pin the folding boundary.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (handle B 0 ((step (u) t (resume (A.tick) t))) (+ (B.step) (A.get)))))
      (export main)))
  (output (: 21 Int64)))

(case
  "the agent-harness loop spine runs model-ask then value-dispatches a tool over turns"
  (doc
    "The native agent-harness loop SPINE (DESIGN-agent-harness.md) as a single-shot tail-resumptive
           program with NO ABI dependency: a recursive `loop` drives N turns; each turn performs `Model.ask`
           (a MOCK handler standing in for the Bedrock peer — the nearer-handler-wins override swaps the real
           peer in later) then DISPATCHES a tool BY VALUE — an `= 0` check on the answer routes to `Tools.stop`
           (return the accumulator) vs `Tools.step` (accumulate + recurse). Both effects are handled IN-PROGRAM
           via NESTED handlers; the Tools handler threads the running total. main runs 3 turns: turn i asks→i,
           i≠0 so step accumulates i and recurses; at i=0 ask→0 so stop returns 3+2+1 = 6. Exercises
           nested-handler dispatch + single-shot resume + handler-state threading + value-dispatch in one
           spine. Relocated from rcdzc an_agent_loop_shape_runs_model_ask_then_tool_dispatch_over_turns.")
  (input
    (do
      (effect Model (op ask (-> Int64 Int64)))
      (effect Tools (op step (-> Int64 Int64)) (op stop (-> Int64 Int64)))
      (def
        (loop (: i Int64) (: acc Int64))
        (if (= (Model.ask i) 0) (Tools.stop acc) (loop (- i 1) (Tools.step (+ acc i)))))
      (def
        (main)
        (handle
          Model
          0
          ((ask (q) s (resume q s)))
          (handle Tools 0 ((step (a) s (resume a a)) (stop (a) s (resume a a))) (loop 3 0))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a SIX-deep alternating A-B perform chain threads both nested states through every crossing"
  (doc
    "The deep-interleave stress of the two-frame nesting above: six performs alternate A-B-A-B-A-B
           where each perform's ARGUMENT is the previous perform's result — `(B.b (A.a (B.b (A.a (B.b
           (A.a 0))))))`. Both arms fold the argument into the resume value AND advance their own state
           (`a` adds s then s+=1; `b` adds t then t+=10), so every crossing must read the value produced
           under the OTHER handler's frame and its own CURRENT state: 0→5→105→111→221→228→348 (s walks
           5,6,7; t walks 100,110,120). One wrong state snapshot or one stale intermediate anywhere in
           the six-step chain lands off the checksum. Pins the data-dependency chain BETWEEN two live
           handler frames at depth six — prior nesting pins cross at most twice.")
  (input
    (do
      (effect A (op a (-> Int64 Int64)))
      (effect B (op b (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a (v) s (resume (+ v s) (+ s 1))))
          (handle B 100 ((b (v) t (resume (+ v t) (+ t 10)))) (B.b (A.a (B.b (A.a (B.b (A.a 0)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 348 Int64)))

(case
  "a recursive nested-op performer whose resume-VALUE reads the inner state around the outer perform folds (the inner state re-binds to the slot param)"
  (doc
    "The state-reading companion of the fold above. Here the inner `B.step` arm's resume VALUE reads the
           inner state binder `t` around the outer perform — `(step (u) t (resume (A.tick (+ t u)) t))`. The
           pre-spec-lift now lifts it: alongside substituting the op params, it RE-BINDS `arm.state` (`t`) to
           the inner slot's threaded state PARAM (`state_names[k]`, keyed by the inner op's decl), so the lifted
           `(A.tick (+ t u))` reads the spec's inner-slot state instead of an orphaned `t` (the #2077 orphan
           this used to decline to avoid). Sound because the `next == arm.state` guard ensures the inner op
           does not ADVANCE its state, so the incoming slot param is the value to read. Answers 218.")
  (input
    (do
      (effect A (op tick (-> Int64 Int64)))
      (effect B (op step (-> Int64 Int64)))
      (def (loop (: n Int64)) (if (= n 0) 0 (+ (B.step n) (loop (- n 1)))))
      (def
        (main)
        (handle
          A
          100
          ((tick (a) s (resume (+ a s) (+ s 1))))
          (handle B 7 ((step (b) t (resume (A.tick (+ t b)) t))) (loop 2))))
      (export main)))
  (output (: 218 Int64)))

(case
  "nested recursive performers — an inner walk's out-state threads across the outer recursion boundary"
  (doc
    "Finding #19 (breaker/corpus-bugfix): two composed recursive performers. `outer` draws a depth via
           `S.depth` per iteration and adds `(inner d 0)`; `inner` performs `S.tick` per hop. `inner`'s state
           advances must thread back across the OUTER recursion so the next `outer` iteration's `S.depth`
           reads the advanced state — the composed-nested-loop shape. The SINGLE-return fold dropped `inner`'s
           out-state every outer iteration (a silent wrong value: 9 not 7). The multi-value marking now
           recognizes that a recursive-effectful callee (`inner`) whose out-state feeds an enclosing recursive
           def's self-call must be threaded, and both are upgraded to multi-value — so the shape no longer
           SILENTLY MISCOMPILES. Threading it to the final value — the full cross-def recursion-boundary fold —
           now FOLDS: `thread_returning_tuple`'s let-dispatch arm routes a `(let ((d (E.op))) (self-call …))`
           body (was dropped to the leaf arm, which orphaned `d`) so the let-init perform's binder + advance
           thread, and the cross-def callee's out-state projects across `outer`'s recursion under multi-value
           mode. Correct value pinned: main(1)=7, main(0)=2. Each walk also folds ALONE. Uniform on all 3 backends.")
  (input
    (do
      (effect S (op depth (-> Int64)) (op tick (-> Int64)))
      (def (inner (: k Int64) (: acc Int64)) (if (< k 1) acc (inner (- k 1) (+ acc (S.tick)))))
      (def
        (outer (: k Int64) (: acc Int64))
        (if (< k 1) acc (let ((d (S.depth))) (outer (- k 1) (+ acc (inner d 0))))))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((depth () s (resume (% s 3) (+ s 1))) (tick () s (resume s (+ s 1))))
          (outer 3 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "nested recursive performers THROUGH A NON-RECURSIVE PASS-THROUGH also fold (the pass-through inlines to the recursive leaf)"
  (doc
    "The INDIRECTION face of finding #19: `outer`'s self-call reaches the recursive performer `inner`
           through a NON-RECURSIVE helper `via` (`(def (via k) (inner k 0))`, outer adds `(via d)`). The
           cross-def recursion-boundary fold now handles it: the let-dispatch arm threads `(let ((d (S.depth)))
           (outer … (via d)))`, `via` inlines to `(inner d 0)`, and `inner`'s out-state projects across `outer`'s
           recursion under multi-value mode — folding to the correct value instead of the earlier silent wrong
           value (9 not 7) the direct-only checks let slip. A STRAIGHT-LINE performing helper that reaches NO
           recursive performer (a `pair-draw` doing two ticks) folds by the ordinary inline (breaker s19g).
           Correct value pinned main(1)=7 / main(0)=2. Uniform 3 backends.")
  (input
    (do
      (effect S (op depth (-> Int64)) (op tick (-> Int64)))
      (def (inner (: k Int64) (: acc Int64)) (if (< k 1) acc (inner (- k 1) (+ acc (S.tick)))))
      (def (via (: k Int64)) (inner k 0))
      (def
        (outer (: k Int64) (: acc Int64))
        (if (< k 1) acc (let ((d (S.depth))) (outer (- k 1) (+ acc (via d))))))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((depth () s (resume (% s 3) (+ s 1))) (tick () s (resume s (+ s 1))))
          (outer 3 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "a straight-line non-recursive performing helper under an outer recursion folds (finding #19 boundary)"
  (doc
    "The FOLD boundary of finding #19's indirection decline (breaker s19g): `outer`'s self-call arg calls
           a NON-recursive helper `pair-draw` that performs two `S.tick`s but reaches NO recursive performer.
           The transitive recursion-boundary marking must NOT flag it (only a helper transitively reaching a
           RECURSIVE performer declines) — so this FOLDS to a value, it is not over-declined. Pins that the
           indirection decline is precise: straight-line helpers stay folding. main(1) = 33 (outer3 seed1:
           depth s→1 tick 1+tick 2 = 3; depth s→... the two ticks per iteration accumulate), main(0) exercised
           by the direct/indirection pins. Uniform 3 backends.")
  (input
    (do
      (effect S (op depth (-> Int64)) (op tick (-> Int64)))
      (def (pair-draw (: x Int64)) (+ (S.tick) (S.tick)))
      (def
        (outer (: k Int64) (: acc Int64))
        (if (< k 1) acc (let ((d (S.depth))) (outer (- k 1) (+ acc (pair-draw d))))))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((depth () s (resume (% s 3) (+ s 1))) (tick () s (resume s (+ s 1))))
          (outer 3 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 33 Int64)))

(case
  "a recursive performer whose self-call arg calls a non-recursive helper that PERFORMS the discharged op folds"
  (doc
    "Coverage complement to the s19g boundary above (v-effects self-select, finding-#19 family). Here the
           non-recursive helper `bump` PERFORMS the discharged op `S.tick` DIRECTLY (not a recursive performer,
           so the recursion-boundary decline does NOT fire — s19g/s19f only decline a helper transitively
           reaching a RECURSIVE performer). The helper's perform sits on the self-call's ARGUMENT, i.e. on the
           strict spine BEFORE the recursion advances, so single-return threading is correct: `bump acc` reads
           the current tick then the self-call carries the incoming state forward, per iteration. This FOLDS to
           a value (it is not the dropped-out-state shape — the helper is not recursive, so there is no callee
           out-state to thread across the boundary). Pins that a NON-recursive performing helper in a recursive
           self-call arg keeps folding (guards against the transitive marking over-reaching to it). `loop 3`
           seed 1: bump reads tick 1 (s→2) +0 = 1; +tick 2 (s→3) = 3; +tick 3 (s→4) = 6 → 6. main(0): 0+1+... = 3.")
  (input
    (do
      (effect S (op tick (-> Int64)))
      (def (bump (: x Int64)) (+ (S.tick) x))
      (def (loop (: k Int64) (: acc Int64)) (if (< k 1) acc (loop (- k 1) (bump acc))))
      (def (main (: n Int64)) (handle S n ((tick () s (resume s (+ s 1)))) (loop 3 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 6 Int64))
  (call main (: 0 Int64))
  (output (: 3 Int64)))

(case
  "a non-recursive helper that performs the discharged op UNDER a conditional, in a recursive self-call arg, folds"
  (doc
    "The CONDITIONAL twin of the case above (v-effects self-select): `bump`'s perform of `S.tick` sits
           UNDER an `if` (`(if (< k 3) (+ (S.tick) x) x)`) rather than on the helper's unconditional spine. This
           was historically a clean DECLINE ('not yet reducible' — the branch-local threading of a perform under
           an inlined `if` left a state ref unbound in the specialized def's sig); it now FOLDS correctly. The
           helper is non-recursive so it inlines; the `if` lifts and each branch threads the incoming state, so
           the perform-taking branch advances state while the pass-through branch carries it unchanged. `loop 4`
           seed n: k=4,3 skip (k<3 false, no tick); k=2 ticks (reads n, s→n+1) +0 = n; k=1 ticks (reads n+1,
           s→n+2) +n = 2n+1; k=0 returns acc = 2n+1. So main(1)=3, main(0)=1. Pins that a conditional perform in
           a non-recursive helper on a recursive self-call arg no longer declines — the branch state-threading
           binds correctly through the specialization.")
  (input
    (do
      (effect S (op tick (-> Int64)))
      (def (bump (: k Int64) (: x Int64)) (if (< k 3) (+ (S.tick) x) x))
      (def (loop (: k Int64) (: acc Int64)) (if (< k 1) acc (loop (- k 1) (bump k acc))))
      (def (main (: n Int64)) (handle S n ((tick () s (resume s (+ s 1)))) (loop 4 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

; A recursive effectful walk whose SELF-CALL precedes an OUT-STATE-OBSERVING perform folds via the
; multi-value return (the callee returns `(value, out-state)`, the self-call is let-bound and its out-state
; threads into the following perform). The single-return specialization threads only the INCOMING state, so
; folding against it gives a wrong value; the multi-value marking fixes it. These pin the fold across the
; syntactic SITE of the self-call: a let-init, a do-sequence, two let-sequenced siblings, a match scrutinee,
; and an if condition — each seeded 0 so the drawn ids are observable in the result.
(case
  "a let-bound self-call preceding an out-state-observing perform folds via multi-value return"
  (doc
    "`(let ((rest (walk (- n 1)))) (+ rest (Ctr.tick)))` — the self-call is let-bound in the init and
           the following `Ctr.tick` reads the recursion's OUT-state. `walk 3` seeded 0 draws ids 0,1,2 down
           the recursion; the value is `rest + tick` = walk2's value (1) + this level's tick (2) = 3. A
           single-return fold against the incoming state would give the wrong value.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (let ((rest (walk (- n 1)))) (+ rest (Ctr.tick)))))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a do-sequenced self-call preceding a perform folds via multi-value return"
  (doc
    "`(do (walk (- n 1)) (Ctr.tick))` — the self-call runs first in a do-sequence, then the trailing
           `Ctr.tick` reads its OUT-state and the `do` yields that last tick. `walk 3` seeded 0 runs three
           ticks drawing 0,1,2 → the do yields the last, 2. Pins the do-sequence site of the multi-value fold.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (do (walk (- n 1)) (Ctr.tick))))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "two let-sequenced sibling self-calls thread out-state via multi-value return"
  (doc
    "The effectful TREE WALK: `(let ((a (walk l))) (let ((b (walk r))) (+ a b)))` — the first sibling's
           OUT-state threads into the second (each leaf draws a fresh id). `walk (Node Leaf Leaf)` seeded 0
           draws 0 for the left leaf and 1 for the right → 0 + 1 = 1. A fold that threaded the second sibling
           against the INCOMING state would draw 0 twice → 0 (a silent wrong value).")
  (input
    (do
      (type T (Leaf) (Node T T))
      (effect Fresh (op next (-> Int64)))
      (def
        (walk (: t T))
        (match
          t
          ((T.Leaf) (Fresh.next))
          ((T.Node l r) (let ((a (walk l))) (let ((b (walk r))) (+ a b))))))
      (def
        (main)
        (handle Fresh 0 ((next () s (resume s (+ s 1)))) (walk (T.Node (T.Leaf) (T.Leaf)))))
      (export main)))
  (call main)
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a match-scrutinee self-call preceding an arm-body perform folds via multi-value return"
  (doc
    "`(match (walk (- n 1)) (_ (Ctr.tick)))` — the self-call is the match scrutinee and the arm body
           performs, reading the scrutinee's OUT-state. `walk 3` seeded 0 draws 0,1,2 and the arm yields the
           last tick → 2. Pins the match-scrutinee site of the multi-value fold.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (match (walk (- n 1)) (_ (Ctr.tick)))))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "an if-condition self-call preceding a branch perform folds via multi-value return"
  (doc
    "`(if (< (walk (- n 1)) 100) (Ctr.tick) 99)` — the self-call is in the if condition and the taken
           branch performs, reading the condition's OUT-state. `walk 3` seeded 0: each level's drawn id is
           under 100 so the `<` holds and the taken branch draws 0,1,2 → 2. Pins the if-condition site of the
           multi-value fold (the condition self-call is drained around the whole if).")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (if (< (walk (- n 1)) 100) (Ctr.tick) 99)))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 2 Int64)))

; The DIRECT-sibling-operand site of the multi-value fold (companion of the let-init/do-seq/match/if sites
; above): the recursive self-call and a following perform are the two STRICT OPERANDS of one form — the
; arguments of a call (`(. List push)`) or the operands of an arithmetic op (`-`) — with NO intervening
; `let`. The self-call is drained around the operand list and its out-state threads into the sibling
; perform. A single-return fold against the incoming state would miscompute the sibling's draw.
(case
  "a self-call and a sibling perform as direct list-push operands thread out-state via multi-value return"
  (doc
    "`((. List push) (build (- n 1)) (Idx.next))` — the recursive `build` and the `(Idx.next)` perform
           are the two arguments of `List.push`, no let. `build 3` seeded 1 pushes one element per level
           (3 draws) → the list has length 3. Pins the list-push direct-operand site of the multi-value
           fold (the self-call's out-state threads into the sibling push-argument perform).")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def (build (: n Int64)) (if (= n 0) #list() (List.push (build (- n 1)) (Idx.next))))
      (def (main) (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (List.len (build 3))))
      (export main)))
  (call main)
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a self-call and a sibling perform as direct subtraction operands thread out-state via multi-value return"
  (doc
    "`(- (build (- n 1)) (Idx.next))` — the recursive self-call is the LEFT operand and the perform the
           RIGHT operand of an order-sensitive `-`, no let. Seeded 1, the deepest recursion draws first so
           ids go 1,2,3 bottom-up: build 1 = 0-1 = -1, build 2 = -1-2 = -3, build 3 = -3-3 = -6. A fold that
           threaded the perform against the incoming (pre-recursion) state would miscompute. Pins the
           arithmetic-operand direct-sibling site.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def (build (: n Int64)) (if (= n 0) 0 (- (build (- n 1)) (Idx.next))))
      (def (main) (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (build 3)))
      (export main)))
  (call main)
  (output (: -6 Int64)))

(case
  "a perform BEFORE the self-call in a subtraction folds via the single-return path"
  (doc
    "The mirror of the arithmetic direct-operand case: the perform is the LEFT operand and the
           self-call the RIGHT — `(- (Idx.next) (build (- n 1)))` — so the perform reads the PRE-recursion
           (incoming) state and the single-return path suffices (no out-state threading needed). Seeded 1:
           the outermost `(Idx.next)` draws 1, and `build 2` recurses drawing before its own subtractions;
           the whole folds to 2. Pins that the perform-before-self-call order still folds (single-return),
           bracketing the multi-value cases above.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def (build (: n Int64)) (if (= n 0) 0 (- (Idx.next) (build (- n 1)))))
      (def (main) (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (build 3)))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a NON-recursive helper calling a nested op whose resume performs the outer effect folds"
  (doc
    "The non-recursive-helper twin of the resume-value-performs-outer case above (v-effects self-probe).
           A non-recursive `helper` calls the inner `B.step` (whose arm resumes with `(A.tick)`, performing the
           outer `A`), and the continuation reads `(A.get)`. Because `helper` is NON-recursive it INLINES, so
           the outer advance threads correctly: `(helper)` = 10, `A.tick` advanced A to 11, `(A.get)` = 11 →
           `(+ 10 11)` = 21. Pins the RECURSION boundary of the recursive-nested-arm miscompile: the SAME body
           behind a RECURSIVE caller drops the advance (merged_nested_ctx skips the merge because the
           accum-transformed recursive callee reads non-recursive at the merge decision), but a non-recursive
           caller folds — so the discriminator is specifically the recursive-specialization path, not the
           nested-arm-outer-perform shape itself.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def (helper) (B.step))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (handle B 0 ((step (u) t (resume (A.tick) t))) (+ (helper) (A.get)))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a HOST-delegated perform in a nested arm's NEXT-STATE slot is served (sequences at the boundary)"
  (doc
    "The host-routing boundary of the next-state-slot outer-perform family (v-effects self-probe, breaker
           as-class radius). The IN-PROGRAM sibling — an outer HANDLER's effect performed directly in a nested
           arm's next-state expr, `(step (u) t (resume t (+ t (A.get))))` — is a not-yet-foldable safe-decline:
           the next-state threads forward as a state EXPRESSION, so a handler-routed perform embedded there
           would be dropped or duplicated by the fold (the honest todo; the correct fold is a later increment).
           But when that same slot performs a HOST-delegated op — `(resume t (+ t (ask.ask)))` under an
           entrypoint `(host (ask) …)` — it is SERVED, because a host call is a plain boundary function call
           sequenced at its evaluation point, NOT threaded through the state-expression the fold rewrites — so
           it never had the drop/duplicate hazard and folds strict.
           Trace: `B` seeds 0. The arm resumes the PRE-advance `t` as the value, and advances the slot to
           `(+ t (ask.ask))`. Body `(+ (* 10 (B.step)) (B.step))`: the first `B.step` reads t=0 → value 0,
           advancing the slot to `(+ 0 (ask.ask))` = 100 (this is the ONE host call — `ask.ask`=100); the
           second `B.step` reads the advanced 100 → value 100. Its own next-state `(+ 100 (ask.ask))` is never
           evaluated (nothing after the last step reads the state), so NO second host call fires. Result
           `(+ (* 10 0) 100)` = 100, exactly ONE `ask.ask`. Pins the host-vs-in-program ROUTING discriminator
           on the next-state slot: the fold's decline is scoped to IN-PROGRAM foreign performs; a host-delegated
           slot perform is served (and a dead trailing next-state fires no spurious host call).")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (host
          (ask)
          (handle B 0 ((step (u) t (resume t (+ t (ask.ask))))) (+ (* 10 (B.step)) (B.step)))))
      (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 100 Int64)))

(case
  "an outer effect performed DIRECTLY in a nested arm's next-state slot folds to 6 (as2)"
  (doc
    "breaker as-family. An inner handler arm whose NEXT-STATE slot performs an OUTER effect directly —
           `(step (u) t (resume t (+ t (A.get))))` — was once a silent miscompile (the next-state threads
           forward as a state EXPRESSION, so the embedded `(A.get)` was DROPPED — returned 5, must be 6) and
           then a clean decline. It now FOLDS correctly: `hoist_next_state_foreign_perform` lifts the
           next-state foreign perform to a dispatch-time `let`-init before the resume — `(let ((_cdz_ns0
           (A.get))) (resume t (+ t _cdz_ns0)))`, the proven value-equivalent as7 shape — so `(A.get)` runs
           once per dispatch and its PURE result threads forward. Single B.step reads the seed 5, its
           next-state runs A.get (5, A->6) → 5, body `(+ (* 10 0) …)`… trace: B.step reads t=0 → 0, next-state
           `0 + (A.get)=5`; body `(+ (* 10 0) (A.get))` reads the advanced A=6 → `0 + 6` = 6. Contrast the
           HOST-delegated slot perform above, served identically (a host call is sequenced at its evaluation
           point, not threaded through the state expression).")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          5
          ((get (u) s (resume s (+ s 1))))
          (handle B 0 ((step (u) t (resume t (+ t (A.get))))) (+ (* 10 (B.step)) (A.get)))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "an outer effect in a nested arm's RESUME-VALUE slot is served (as3)"
  (doc
    "The served control: an outer perform in the RESUME-VALUE slot `(resume (+ t (A.get)) t)` runs at
           dispatch (unlike the next-STATE slot, which declines). B.step reads t=0; the resume value runs
           A.get (5, A->6); B.step returns 5; body A.get reads the advanced 6; `(+ (* 10 5) 6)` = 56.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          5
          ((get (u) s (resume s (+ s 1))))
          (handle B 0 ((step (u) t (resume (+ t (A.get)) t))) (+ (* 10 (B.step)) (A.get)))))
      (export main)))
  (output (: 56 Int64)))

(case
  "an outer perform LET-LIFTED out of the next-state slot folds strict (as7 workaround)"
  (doc
    "The user workaround for the next-state-slot decline: lift the outer perform to a let so it runs at
           dispatch. `(let ((x (A.get))) (resume t (+ t x)))` — A.get runs (5, A->6); body A.get reads 6;
           B.step returns 0; `(+ (* 10 0) 6)` = 6. Pins the let-lift semantics-preserving equivalence.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          5
          ((get (u) s (resume s (+ s 1))))
          (handle
            B
            0
            ((step (u) t (let ((x (A.get))) (resume t (+ t x)))))
            (+ (* 10 (B.step)) (A.get)))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a performing condition's advance survives an inner abort and is read by a post-handle observer"
  (doc
    "A performing CONDITION on outer effect A guards an `if` whose taken branch ABORTS the inner B
           handle. The cond A.tick reads the seed and advances A; the inner B-abort collapses to 99; then a
           SECOND A.tick OUTSIDE the B handle must read the ADVANCED A state. n=5: cond A.tick reads 5 (A->6);
           branch aborts inner=99; outer A.tick reads 6 → 99 + 6 = 105 (dropping the advance would give 104).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)))
      (effect B (op bail (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((tick (u) s (resume s (+ s 1))))
          (+ (handle B 0 ((bail (u) t 99)) (if (> (A.tick) 0) (B.bail) -1)) (A.tick))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "a conditional-resume (two-site) arm folds across a SINGLE perform"
  (doc
    "A handler arm with a branching (two-site) resume `(if (> s 5) (resume v s) (resume -1 s))` folds
           over a single perform. Seed 7, `(> 7 5)` true → the arm resumes the op arg v; state passes through
           unchanged. `(Src.read n)` at n=5 → resumes v = 5.")
  (input
    (do
      (effect Src (op read (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle Src 7 ((read (v) s (if (> s 5) (resume v s) (resume -1 s)))) (Src.read n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a conditional-resume (two-site) arm folds across TWO performs via the two-hole refold"
  (doc
    "The same branching two-site resume arm folds when the body performs TWICE (the two-hole refold
           re-serves a both-branch resume across the second perform). Both reads see seed 7 (state passthrough),
           each resumes its own arg: read(5)=5, read(6)=6 → `5 + 10*6` = 65.")
  (input
    (do
      (effect Src (op read (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Src
          7
          ((read (v) s (if (> s 5) (resume v s) (resume -1 s))))
          (+ (Src.read n) (* 10 (Src.read (+ n 1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 65 Int64)))

(case
  "an outer effect in a nested arm's next-state slot across THREE chained performs folds to 61 (as1)"
  (doc
    "The multi-DISPATCH face of the next-state-slot outer-perform fold: three chained `B.step` dispatches
           each re-run the next-state's outer `(A.get)`. Once a silent miscompile / clean decline, it now
           FOLDS via `hoist_next_state_foreign_perform` (the `let`-lift runs A.get once PER dispatch, threading
           its pure result — the ordinary `_cdz_ns` binder name is load-bearing: a `#`-prefixed one trips the
           fold's growing-state heuristics and mis-threads across dispatches). Over an A seed of 5, the three
           B.step read 0, 5, 11 (each dispatch's next-state `(+ t (A.get))` bumps A: 5→6→7 read as 5,6,7), so
           the body `(+ (* 100 (B.step)) (+ (* 10 (B.step)) (B.step)))` = `100·0 + (10·5 + 11)` = 61.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          5
          ((get (u) s (resume s (+ s 1))))
          (handle
            B
            0
            ((step (u) t (resume t (+ t (A.get)))))
            (+ (* 100 (B.step)) (+ (* 10 (B.step)) (B.step))))))
      (export main)))
  (call main)
  (output (: 61 Int64)))

(case
  "an outer effect in BOTH the resume value and next-state slot folds to 57 (asb)"
  (doc
    "The both-slots face: the outer effect is performed in BOTH the resume VALUE and the NEXT-STATE —
           `(resume (A.get) (A.get))`. Once a silent 56 / clean decline, it now FOLDS to the correct 57:
           `hoist_next_state_foreign_perform` lifts BOTH performs to `let`-inits, the VALUE's FIRST then the
           next-state's — `(let ((_cdz_ns0 (A.get)) (_cdz_ns1 (A.get))) (resume _cdz_ns0 _cdz_ns1))` — so the
           value-slot A.get sequences BEFORE the next-state's (value-then-next-state order is load-bearing: a
           next-state-first order gives the wrong 67). Trace over A seed 5: the value A.get reads 5 (A→6), the
           next-state A.get reads 6 (A→7); B.step returns the value 5; body `(+ (* 10 (B.step)) (A.get))` =
           `10·5 + (A.get=7)` = 57.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          5
          ((get (u) s (resume s (+ s 1))))
          (handle B 0 ((step (u) t (resume (A.get) (A.get)))) (+ (* 10 (B.step)) (A.get)))))
      (export main)))
  (call main)
  (output (: 57 Int64)))

(case
  "a handler arm forwarding an effect its enclosing scope does not hold is rejected"
  (doc
    "Witnesses capabilities-and-effects.md #Capabilities Attenuate: A Handler Forwards A Narrower Row
           (2nd sentence — attenuation never WIDENS): a handler MUST NOT grant its sub-computation an effect
           row label it does not itself hold. `A`'s arm forwards `B` (performs `(B.b)` as its resume value),
           but `B` is neither handled by an enclosing handler nor delegated at the entrypoint anywhere in
           `main`'s scope — so the arm reaches an effect its enclosing scope does not hold. Rejected at
           compile time (CDZ0401, the no-home check): an arm cannot forward a capability the enclosing row
           does not carry, keeping 'no ambient authority' transitive across the handle. The over-broad
           forward is a compile-time rejection, not a runtime failure.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def (main) (handle A 0 ((a (u) s (resume (B.b) s))) (A.a)))
      (export main)))
  (error CDZ0401))

(case
  "a handler arm forwards an effect its enclosing scope DOES hold and runs"
  (doc
    "The positive companion (attenuation NARROWS within what is held — 1st sentence): the SAME arm that
           forwards `B` is accepted once an enclosing handler HOLDS `B`. `main` wraps the `A`-handler in a
           `B`-handler, so `A`'s arm forwarding `(B.b)` reaches a held effect: `B` seeded 100 resumes `s`
           (=100), so `(B.b)` is 100, `A`'s arm resumes 100, and `(A.a)` = 100. Pins that the forward is
           legal exactly when the enclosing scope carries the label — the row a handler forwards is a SUBSET
           of the row it holds, checked statically (the reject above is the same check failing).")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def
        (main)
        (handle B 100 ((b (u) s (resume s s))) (handle A 0 ((a (u) s (resume (B.b) s))) (A.a))))
      (export main)))
  (output (: 100 Int64)))

(case
  "an arm resuming with a re-perform of its OWN effect forwards to an outer handler of that effect"
  (doc
    "The SAME-effect forwarding case: an arm resuming with a fresh perform of the effect IT discharges
           re-performs OUTWARD — a handler arm's own-effect perform forwards to the next-OUTER handler of that
           effect, not back into itself (`check_no_home` walks arm bodies under the OUTER handled set). Inner
           `Inner`'s arm resumes with `(Outer.i-style)`… here spelled with two effects to show the forward
           reaches an ENCLOSING handler: `Inner`'s arm resumes `(Outer.o)`, and `Outer` is handled outside —
           `Outer` seeded 50 resumes its state, so `(Outer.o)` = 50, `Inner.i` resumes 50, `(+ 1 (Inner.i))` =
           51. Pins that a resume value performing an effect handled FURTHER OUT folds (the forward reaches an
           enclosing home) — the mechanism the interpose cases rely on, isolated to the resume-value position.")
  (input
    (do
      (effect Outer (op o (-> Unit Int64)))
      (effect Inner (op i (-> Unit Int64)))
      (def
        (main)
        (handle
          Outer
          50
          ((o (u) t (resume t t)))
          (handle Inner 0 ((i (u) s (resume (Outer.o) s))) (+ 1 (Inner.i)))))
      (export main)))
  (output (: 51 Int64)))

(case
  "an arm re-performing its own effect with no outer handler has no home"
  (doc
    "The reject companion of the forwarding case above: when an arm resumes with a fresh perform of the
           effect it discharges — `(flip (u) s (resume (Amb.flip) s))` — that own-effect perform re-performs
           OUTWARD (arm bodies resolve under the outer handled set), so it needs an ENCLOSING `Amb` handler.
           Here there is none (this is the only `Amb` handler), so the re-perform has no home: CDZ0401. This
           is NOT a misleading message — under the forwarding model an arm's own-effect perform genuinely
           escapes to an outer handler, and the outermost one has nowhere to forward. (A bare self-resume like
           this would also be a non-terminating re-perform loop were it to fold; the no-home reject is the
           correct diagnosis, the same check that flags forwarding a DIFFERENT unheld effect above.)")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (resume (Amb.flip) s))) (+ 1 (Amb.flip))))
      (export main)))
  (error CDZ0401))

(case
  "an abortive handler abandons a host call in the path it discards"
  (doc
    "Witnesses that an abortive perform's abandonment extends to a DELEGATED host call in the
           discarded continuation (capabilities-and-effects.md #A Handler Arm May Abandon The Computation It
           Discharges, composed with #Host Delegation Is An Entrypoint's Prerogative). The body `(+
           (Bail.bail 7) (ask.ask))` evaluates LEFT-TO-RIGHT: the first operand `(Bail.bail 7)` is abortive
           (its arm never resumes), so it abandons the whole `+` — the handle evaluates to 7 and the second
           operand `(ask.ask)` is NEVER reached. Because it is not reached, the host call is NOT issued: the
           observed host-call sequence is EMPTY. A run's host I/O is exactly the calls on the taken path,
           never a call in an abandoned one — so an abort that jumps past a would-be host call elides it.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (host (ask) (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 7) (ask.ask)))))
      (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls)
  (output (: 7 Int64)))

(case
  "an abortive handler ISSUES a host call sequenced BEFORE the abort in its discarded body"
  (doc
    "The complement of the case above (abort ELIDES a host call AFTER it): a delegated host call
           sequenced BEFORE the abort on the strict do-spine IS issued — its effect is committed before the
           abort abandons the rest. `(do (ask.ask) (Bail.bail 7))` under `Bail`: `ask.ask` runs (the host
           call is issued, response 100 discarded — the `do` evaluates it for effect), THEN `Bail.bail 7` —
           a non-resuming arm — abandons the `do`, so the handle value is the abort 7. The observed host-call
           sequence is `(call ask.ask)` (issued), NOT empty. Pins that the do-shape abort-fold preserves a
           FOREIGN HOST perform in the pre-abort prefix (the host analogue of the outer-effect pre-abort
           preservation): the discarded continuation drops only what comes AFTER the abort, never a
           side-effect already committed before it.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (host (ask) (handle Bail 0 ((bail (n) s n)) (do (ask.ask) (Bail.bail 7)))))
      (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))

(case
  "a host-delegated result SEEDS an in-program handler's initial state"
  (doc
    "The host-to-handler data flow: the handle's SEED expression is itself a host-delegated perform —
           `(handle Ctr (Env.seed unit) …)` — so the host response (5) becomes the in-program handler's
           initial state, evaluated once before the handle's region runs. The two in-program ticks then
           read 5 and 6 (the seeded state advancing normally) → 56. Pins that the seed position accepts a
           performing expression whose own effect discharges at the ENCLOSING (here host) level — the
           config-fetch-then-run idiom (read a setting from the host, seed a counter/limiter with it).")
  (input
    (do
      (effect Env (op seed (-> Unit Int64)))
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (main)
        (host
          (Env)
          (handle
            Ctr
            (Env.seed unit)
            ((next (u) s (resume s (+ s 1))))
            (+ (* 10 (Ctr.next unit)) (Ctr.next unit)))))
      (export main)))
  (call main)
  (host-responses (respond Env.seed (: 5 Int64)))
  (output (: 56 Int64)))

(case
  "an inner handler's SEED is a perform of an OUTER in-program effect"
  (doc
    "The in-program analogue of the host-delegated-seed case above: an inner handler's SEED expression
           is itself a perform of an OUTER in-program effect — `(handle B (A.base) …)` where `A.base` homes
           to the enclosing `A` handler (not the host). The outer `A.base` reads A's state 5 (its arm resumes
           `s` unchanged) → 5 becomes B's initial state, evaluated once before B's region. B's two ticks then
           read 5 and 6 (B-state advancing) → `(+ 5 6)` = 11. Pins that the seed position accepts a performing
           expression whose effect discharges at an ENCLOSING in-program handler — the intra-program
           config-fetch-then-run idiom (the host-seed case's non-host twin).")
  (input
    (do
      (effect A (op base (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          5
          ((base (u) s (resume s s)))
          (handle B (A.base) ((step (u) t (resume t (+ t 1)))) (+ (B.step) (B.step)))))
      (export main)))
  (output (: 11 Int64)))

(case
  "a delegated host effect composes with the value-heap runtime"
  (doc
    "Witnesses that a program may BOTH delegate an effect to the host AND use the value-heap runtime
           in one component (capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses composed with the runtime's collection operations). `ask` is delegated to the host; its
           returned value is used as a KEY inserted into a runtime map. The component imports TWO interfaces —
           the effect (as `host`) and the value-heap runtime (as `heap`) — and the boundary threads both: the
           host response for `ask.ask` and the runtime's `map-insert`/`map-size` ops. With `ask.ask`
           responding 2, inserting key 2 into the map {1: 10} yields two distinct keys, so `Map.len` is 2 —
           a deterministic function of the input, the recorded response, and the runtime's semantics.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (Map.len (Map.insert #map((= 1 10)) (ask.ask) 20))))
      (export main)))
  (host-responses (respond ask.ask (: 2 Int64)))
  (host-calls (call ask.ask))
  (output (: 2 Int64)))

(case
  "a bare effect declaration that is never performed is well-formed"
  (doc
    "Witnesses capabilities-and-effects.md #An Effect Declaration Names The Effect: an `(effect …)`
           declaration is a routing-agnostic contract — declaring it grants nothing and performs nothing.
           A program that declares an effect but never performs it is well-formed and `main` returns its
           ordinary value 1, the effect decl contributing no behavior (it imports no host function and
           needs no handler).")
  (input (do (effect E (op f (-> Int64 Int64))) (def (main) 1) (export main)))
  (output (: 1 Int64)))

(case
  "an effect discharged by a handler does not escape to the manifest"
  (doc
    "Witnesses capabilities-and-effects.md #An Effect That Does Not Escape Is Discharged By A
           Handler and #An Effect Discharged By An In-Program Handler Does Not Appear In The Manifest:
           the `Choose` effect is declared with a nullary operation `pick`, raised in the body as
           `(Choose.pick)`, and discharged by an enclosing handler that resumes it with 5, so the effect
           never reaches a host function. The handler is stateless (seed `unit`, thread `s` unchanged). The
           program imports no host function, so its manifest is empty (host-calls asserts none), yet it uses
           an effect internally. Operations are qualified by their declaring effect (#An Effect Declaration
           Names The Effect And Types Its Operations).")
  (input
    (do
      (effect Choose (op pick (-> Unit Int64)))
      (def (main) (handle Choose unit ((pick () s (resume 5 s))) (+ (Choose.pick) 1)))
      (export main)))
  (output (: 6 Int64))
  (host-calls))

(case
  "a handler resumes its continuation at most once by default"
  (doc
    "Witnesses capabilities-and-effects.md #A Continuation Is One-Shot By Default: the handler
           resumes the continuation exactly once, so the affine discipline holds and the result is a
           single value (the resumed computation is not duplicated). `Get` is declared with a nullary
           operation `get` returning Int64, performed as `(Get.get)`; the handler is stateless.")
  (input
    (do
      (effect Get (op get (-> Unit Int64)))
      (def (main) (handle Get unit ((get () s (resume 41 s))) (+ (Get.get) 1)))
      (export main)))
  (output (: 42 Int64))
  (host-calls))

(case
  "an abortive handler arm never resumes, so its value becomes the handle's value"
  (doc
    "Witnesses capabilities-and-effects.md #A Handler Arm May Abandon The Computation It Discharges:
           `Bail` declares `bail : Int64 -> Int64`, and the handler's arm `(Bail.bail (n) s n)` NEVER
           resumes — it yields `n` as the arm body's value and discards the continuation. So performing
           `(Bail.bail 7)` inside `(+ 1 (Bail.bail 7))` ABANDONS the surrounding `+ 1` (control never
           returns to it) and the handle evaluates to the arm value 7, NOT 8. This is the abortive class
           — a typed early-exit / 'bail and catch at the top' — realized as a control block the perform
           `br`s out of, carrying the arm value (`DESIGN-effects-rcdzc.md` §4.2). Contrast the tail-
           resumptive `Get` above (resumes, so `+ 1` runs): the arm's resume DISCIPLINE, not the operator,
           decides whether the surrounding computation survives.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail 7))))
      (export main)))
  (output (: 7 Int64)))

; The abortive case above performs with a CONSTANT argument `(Bail.bail 7)`, which folds. The runtime
; companion: the abort argument is a boundary parameter `k`. The abortive arm's value is the handle's
; value = k, and the surrounding `+ 1` is abandoned, decided at run time. This pins the abort control
; block carrying a RUNTIME arm value out of the perform (breaker: fixed by v-effects `bd6ff9bd2`
; "reparent an abortive arm's value → grounds a runtime-arg abort" — the wasm lower previously
; re-derived the handle result as Any for a non-const abort arg, declining "no machine representation";
; the reparent grounds it, and wasm now matches rust).
(case
  "an abortive handler arm with a runtime perform argument yields that runtime value as the handle's value"
  (doc
    "The runtime-argument companion of the abortive-arm case above (which uses a CONST `(Bail.bail
           7)`). Here the bail argument is the boundary parameter `k`: the arm `(bail (n) s n)` never
           resumes, so it abandons the surrounding `+ 1` and the handle evaluates to the arm value n = k.
           run(7) = 7, run(42) = 42 — the abort carries a RUNTIME value out of the perform via the control
           block, not only a constant. Pins that the abortive early-exit grounds its arm value when the
           perform argument is decided at run time.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (run (: k Int64)) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail k))))
      (export run)))
  (call run (: 7 Int64))
  (output (: 7 Int64))
  (call run (: 42 Int64))
  (output (: 42 Int64)))

(case
  "an abortive perform deep in a call chain unwinds every intervening frame to the top handler"
  (doc
    "The 'bail and catch at the top' pattern across FUNCTIONS (DESIGN-effects-rcdzc.md §4.2 cross-
           function non-local exit): the abort is performed three calls deep and abandons EVERY pending
           frame between it and the handler. `main` handles `Bail` and calls `(a 5)`; `a n = (+ 1 (b n))`,
           `b n = (+ 1 (c n))`, `c n = (+ n (Bail.bail 99))`. Performing `(Bail.bail 99)` at the base
           abandons `c`'s `(+ n …)`, `b`'s `(+ 1 …)`, and `a`'s `(+ 1 …)` — none of the pending additions
           runs — so the handle evaluates to the arm value 99, NOT 5+99+1+1. Witnesses that abortion is a
           non-local exit over the whole call chain, not a per-frame return that the intervening arithmetic
           could observe. (The callees are non-recursive, so the inline trigger makes the abort unconditional
           in the inlined body.)")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (c (: n Int64)) (+ n (Bail.bail 99)))
      (def (b (: n Int64)) (+ 1 (c n)))
      (def (a (: n Int64)) (+ 1 (b n)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (a 5)))
      (export main)))
  (output (: 99 Int64)))

(case
  "an abortive perform under THREE nested handlers abandons the two resumptive frames above it"
  (doc
    "The abortive class composed with DEEP nesting: an abort fires inside a body that also performs two
           OTHER effects (`A`, `B`) discharged by enclosing resumptive handlers. `(+ (A.a) (+ (B.b)
           (Bail.bail 99)))` under `handle A … (handle B … (handle Bail …))`: `A.a` resumes (=1), `B.b`
           resumes (=2), then `Bail.bail 99` — a NON-resuming arm — ABANDONS the pending `(+ (A.a) (+ (B.b)
           …)))` frames and yields the arm value 99 as the whole handle's value (NOT 1+2+99). Pins that a
           non-local exit unwinds past the resumptive frames of OTHER, differently-effect handlers stacked
           above it — the abort is the value of the outermost handle, and the intervening resumptive
           computations (already run for their effect) do not observe it.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          1
          ((a (u) s (resume s s)))
          (handle
            B
            2
            ((b (u) s (resume s s)))
            (handle Bail 0 ((bail (n) s n)) (+ (A.a) (+ (B.b) (Bail.bail 99)))))))
      (export main)))
  (output (: 99 Int64)))

(case
  "FOUR nested resumptive frames dispatched innermost-out"
  (doc
    "The resumptive nesting pins stop at two frames; this stacks FOUR (distinct effects, distinct
           seeds at distinct place values) and dispatches innermost-out — D, C, B, A — so each perform
           is served by its own frame with zero escaping: 4000 + 300 + 20 + 5 = 4325. With the
           outermost-first sibling below, pins depth-4 frame bookkeeping in both traversal orders.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect C (op c (-> Unit Int64)))
      (effect D (op d (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a (u) s (resume s (+ s 1))))
          (handle
            B
            20
            ((b (u) s (resume s (+ s 1))))
            (handle
              C
              300
              ((c (u) s (resume s (+ s 1))))
              (handle D 4000 ((d (u) s (resume s (+ s 1)))) (+ (D.d) (+ (C.c) (+ (B.b) (A.a)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4325 Int64)))

(case
  "four nested frames dispatched OUTERMOST-first — every outer perform escapes live inner frames"
  (doc
    "The escape-order stress of the depth-4 stack: dispatching A, B, C, D means the `A.a` perform
           must route past THREE live inner frames (B, C, D) to its handler, `B.b` past two, `C.c`
           past one — the maximal-escape traversal. Same checksum as the innermost-out sibling (4325):
           the answer must not depend on which frame order the body dispatches.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect C (op c (-> Unit Int64)))
      (effect D (op d (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a (u) s (resume s (+ s 1))))
          (handle
            B
            20
            ((b (u) s (resume s (+ s 1))))
            (handle
              C
              300
              ((c (u) s (resume s (+ s 1))))
              (handle D 4000 ((d (u) s (resume s (+ s 1)))) (+ (A.a) (+ (B.b) (+ (C.c) (D.d)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4325 Int64)))

(case
  "an inner abortive handler preserves an OUTER effect's advance committed before the abort (do-shape)"
  (doc
    "The abort-fold's outer-advance preservation (v-effects, breaker ao1). An inner abortive `B`-handle
           runs a FOREIGN perform `(A.tick)` — an OUTER `A` handler's op — on its strict do-spine BEFORE it
           aborts: `(handle B 0 ((bail (v) s v)) (do (A.tick) (B.bail 99)))`. `A.tick` resumes, COMMITTING
           A-state 10→11; then `B.bail` — a non-resuming arm — abandons B's OWN handle (`b` = 99). B's abort
           is B's control over the INNER handle only; it must NOT roll back A's already-committed advance. So
           the outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fold the bare-abort collapse discarded
           the whole inner body — INCLUDING `A.tick` — so `A.get` read the seed 10 and the run yielded 109 (a
           silent cross-backend wrong value). The do-arm keeps the pre-abort foreign item and appends the abort
           value as the tail, `(do (A.tick) 99)`, whose foreign prefix the OUTER fold discharges before the
           value. Contrast the after-abort-dead control below (foreign perform AFTER the abort is genuinely
           unreachable and correctly elided).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (B.bail 99))))) (+ b (A.get)))))
      (export main)))
  (output (: 110 Int64)))

(case
  "an inner abort preserves TWO outer advances committed before it (multi-step outer trace)"
  (doc
    "The multi-advance face of the outer-advance preservation (breaker ao4): the FULL outer trace of the
           aborted inner computation is preserved, not just the last step. TWO foreign `(A.tick)` performs run
           on the inner `B`-handle's do-spine before the abort: `(do (A.tick) (A.tick) (B.bail 99))`. Each
           advances A-state (10→11→12); then `B.bail` abandons B. The outer `(A.get)` must read 12 → `(+ 99
           12)` = 111. Before the fold BOTH advances were discarded (A.get read 10 → 109). Pins that the abort-
           fold threads every pre-abort foreign step, not only the final one.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (A.tick) (B.bail 99))))) (+ b (A.get)))))
      (export main)))
  (output (: 111 Int64)))

(case
  "an inner abort ELIDES an outer perform sequenced AFTER it (dead-path control)"
  (doc
    "The control companion of the outer-advance preservation above: a foreign `(A.tick)` sequenced AFTER
           the abort in the inner do-spine — `(do (B.bail 99) (A.tick))` — is genuinely UNREACHABLE (the abort
           abandons the rest of the sequence), so it is correctly ELIDED: A-state is NOT advanced, the outer
           `(A.get)` reads the seed 10 → `(+ 99 10)` = 109. Pins that the abort-fold preserves only the pre-
           abort prefix (a committed advance) and drops the post-abort dead tail — the discriminator is
           evaluation ORDER relative to the abort, not the mere presence of a foreign perform in the body.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (do (B.bail 99) (A.tick))))) (+ b (A.get)))))
      (export main)))
  (output (: 109 Int64)))

(case
  "an inner abort in a NON-FINAL do-statement elides the DEAD suffix after it (pre-abort advance kept)"
  (doc
    "The dead-suffix control for the do-shape abort-fold (github-liaison review follow-on on #2002/#2014,
           self-probed). The aborting `(B.bail 99)` is a NON-FINAL do-statement with a foreign `(A.tick)` BOTH
           before AND after it — `(do (A.tick) (B.bail 99) (A.tick))` under B. The PRE-abort `A.tick` commits
           A-state 10→11 (kept); the abort abandons the rest, so the trailing `(A.tick)` is DEAD and must NOT
           run. Value is the abort 99, outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fix the do-arm
           kept threading past the abort and set `last` to the DEAD final `(A.tick)` (dropping the abort value
           and FORCING the dead tick) → 23; a multivalue self-call in the dead suffix was likewise forced (34).
           Fixed by BREAKING the do-item loop when a non-final item fires the abort: the abort value is the do's
           value, the dead suffix is never threaded. Composes with the pre-abort-prefix preservation (the
           kept `A.tick` still advances) — this pins BOTH halves in one body.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (B.bail 99) (A.tick))))) (+ b (A.get)))))
      (export main)))
  (output (: 110 Int64)))

(case
  "an inner abort preserves an OUTER advance committed in a MATCH-SCRUTINEE before it (scrutinee collapse)"
  (doc
    "The MATCH-SCRUTINEE face of the outer-advance preservation (breaker ao9). The foreign `(A.tick)`
           and the abort sit on the strict do-spine of a `match` SCRUTINEE — `(match (do (A.tick) (B.bail 99))
           (x x))` under B. The scrutinee is evaluated BEFORE any arm; it ABORTS, so no arm runs and the match
           collapses to the scrutinee's value — but the pre-abort `A.tick` committed A-state 10→11 and must
           survive → outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fix the `Match` thread arm wrapped
           the aborted scrutinee in a dead `(match (do (A.tick) 99) (x x))` whose bare-abort collapse dropped
           `A.tick` → 109. Fixed by collapsing the match to the scrutinee rewrite when threading the scrutinee
           fires a NEW abort (no arm runs), so the enclosing fold discharges the `(do (A.tick) 99)` prefix.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (match (do (A.tick) (B.bail 99)) (x x)))))
            (+ b (A.get)))))
      (export main)))
  (output (: 110 Int64)))

(case
  "an inner abort preserves an OUTER advance committed in a STRICT OPERAND before it (operand-lift)"
  (doc
    "The STRICT-OPERAND face of the outer-advance preservation (breaker ao5; the do-shape face is pinned
           above). The foreign `(A.tick)` is a strict `+` OPERAND evaluated before the abort — `(+ (A.tick)
           (B.bail 99))` under B — not a `do`-statement. `A.tick` resumes, COMMITTING A-state 10→11; then
           `B.bail` (non-resuming) abandons B's OWN handle, so the `+` never completes and `b` = the abort
           value 99. The committed A-advance must survive → outer `(A.get)` reads 11 → `(+ 99 11)` = 110.
           Before the operand-lift the bare-abort collapse discarded `(A.tick)` (a dead `+` wrapper), reading
           the seed 10 → 109. Fixed by lifting the pre-abort foreign operand into a for-effect `do` prefix
           `(do (A.tick) 99)` — the same shape the do-arm produces — which the do-shape abort-fold then
           preserves. Distinct from the do-shape only in the CONSUMING form (`+` operand vs `do` statement).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (+ (A.tick) (B.bail 99))))) (+ b (A.get)))))
      (export main)))
  (output (: 110 Int64)))

(case
  "an inner abort whose ARGUMENT performs the outer effect commits BOTH the pre-abort and the in-arg advance"
  (doc
    "The outer-advance-preservation family above pins a CONSTANT abort arg `(B.bail 99)` with the outer
           perform as a pre-abort SIBLING. This is the distinct variant where the outer perform is INSIDE the
           abort ARGUMENT — `(+ (A.tick) (B.bail (+ 50 (A.tick))))` under B — so the abandoned VALUE is COMPUTED
           from an outer perform, not merely sequenced beside one. TWO A-advances must both commit and both be
           observed: the leading `+`-operand `(A.tick)` reads 10 → commits 10→11 (survives the abort via the
           operand-lift), then the abort-arg `(A.tick)` reads 11 → commits 11→12 while EVALUATING the arg, so
           `B.bail (+ 50 11)` = 61 abandons B's handle (`b` = 61). The outer `(A.get)` then reads 12 →
           `(+ 61 12)` = 73. A drop of either advance (the pre-abort operand OR the in-arg perform) or a failure
           to evaluate the abort arg before abandoning would shift the value; a bare-abort collapse that discarded
           the arg's perform would read a stale A-state. Pins that the abort ARG's own foreign perform is
           evaluated + committed on the strict spine before the arm abandons, composing the operand-lift with an
           arg-position perform (v-effects self-probe, adjacent to breaker ao5).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (+ (A.tick) (B.bail (+ 50 (A.tick)))))))
            (+ b (A.get)))))
      (export main)))
  (output (: 73 Int64)))

(case
  "a strict-operand abort in a DEEP-nested handler stack keeps its 99 when the advances are UNOBSERVED"
  (doc
    "The soundness control for the operand-lift: the SAME strict-operand-abort-with-foreign-prefix shape
           `(+ (A.a) (+ (B.b) (Bail.bail 99)))` under `handle A…(handle B…(handle Bail…))`, but here the outer
           advances are UNOBSERVED (nothing reads A/B after the aborted handle). `A.a` and `B.b` resume (their
           arms pass state through, no increment); `Bail.bail 99` abandons; the value is the abort value 99.
           The operand-lift rewrites the dead `+` nest into `(do (A.a) (do (B.b) 99))` — the foreign prefix
           runs for effect (unobserved) and the value stays 99. Pins that the lift is sound BOTH ways: it
           preserves an OBSERVED advance (the 110 case above) AND leaves an UNOBSERVED one at the correct value
           (this case), because a for-effect `do` prefix only runs the performs — it never changes the abort
           value. (Distinguishes the lift from a naive rewrite that would leak the prefix into the value.)")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          1
          ((a (u) s (resume s s)))
          (handle
            B
            2
            ((b (u) s (resume s s)))
            (handle Bail 0 ((bail (n) s n)) (+ (A.a) (+ (B.b) (Bail.bail 99)))))))
      (export main)))
  (output (: 99 Int64)))

(case
  "an inner abort preserves an outer advance committed in a NESTED strict operand (deeper-operand-lift)"
  (doc
    "The NESTED-OPERAND face of the outer-advance preservation (breaker ax4; the FLAT strict-operand
           face is pinned above). The foreign `(A.tick)` + abort sit one operand DEEPER — `(+ 999 (+ (A.tick)
           (B.bail 99)))` under B. The inner `+` threads to `(do (A.tick) 99)` (the flat operand-lift). But
           the OUTER `(+ 999 …)` around it, whose sibling `999` is pure, would then rebuild `(+ 999 (do
           (A.tick) 99))` — burying the foreign `do` prefix inside a DEAD arithmetic wrapper the bare-abort
           collapse discards, dropping A's advance → outer `(A.get)` reads the seed 10 → 109 (a SILENT
           cross-backend wrong value, breaker MED). Fixed: when the aborting operand's tail is ALREADY a
           lifted `(do …)`, the outer collapse drops the dead pure siblings and keeps the tail directly, so
           the foreign prefix survives → `(A.get)` reads 11 → `(+ 99 11)` = 110. Narrow to a `do`-tail so the
           bare-value bare-abort collapse (and the `#seed`-let scoping) is untouched.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (+ 999 (+ (A.tick) (B.bail 99)))))) (+ b (A.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "a nested-operand abort preserves a HEAP-state advance across a mid-mutation frame drop"
  (doc
    "The heap-state face of the deeper-operand-lift (breaker ax1): the outer effect `Log` threads a
           LIST state (each `note` pushes + returns the pre-push length), and the aborting `(Bail.stop 3)` +
           foreign `(Log.note 7)` sit in a nested `+` under `(+ 999 …)`. The abort drops the mid-mutation
           frame; the committed `Log.note 7` push (advancing the list) must survive so the trailing
           `(Log.note 8)` reads the fully-advanced state. Same lift as ax4 but the surviving advance is a heap
           mutation, not a scalar increment — confirms the do-tail collapse preserves a heap-state advance.
           note n=5 gives len 0, note 7 gives len 1, Bail.stop aborts inner 6 which the abort collapses,
           note 8 gives len 2: 100*0 + 10*6 + 2 = 62.")
  (input
    (do
      (effect Bail (op stop (-> Int64 Int64)))
      (effect Log (op note (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Log
          #list()
          ((note (v) s (resume (List.len s) (List.push s v))))
          (+
            (* 100 (Log.note n))
            (+
              (* 10 (handle Bail 0 ((stop (v) s (* v 2))) (+ 999 (+ (Log.note 7) (Bail.stop 3)))))
              (Log.note 8)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 62 Int64)))

(case
  "an inner abort in a LET-INIT preserves the outer advance committed before it (let-init collapse)"
  (doc
    "The LET-INIT face of the outer-advance preservation (breaker ax7). The foreign `(A.tick)` + abort
           sit in a `let`-INIT — `(let ((x (+ (A.tick) (B.bail 99)))) (+ x 1))` under B. `A.tick` resumes,
           committing A-state 10 to 11; then `B.bail` abandons B's handle, so the `let` never binds `x` and
           the body `(+ x 1)` is DEAD. Before the fix the let-arm threaded the init to `(do (A.tick) 99)` but
           bound it to `x` and ran `(+ x 1)` on the do-tail (a wrong value) while burying the foreign prefix
           in a dead binding the collapse discards, dropping A's advance to the seed 10 → 109. Fixed: when a
           let-INIT fires the abort, collapse the `let` to the init's rewrite directly (its `(do (A.tick) 99)`
           preserving the committed advance), abandoning the body → outer `(A.get)` reads 11 → `(+ 99 11)` =
           110. The let-arm analog of the strict-operand and do-item abort collapses.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (let ((x (+ (A.tick) (B.bail 99)))) (+ x 1)))))
            (+ b (A.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "a do-spine item abort ABANDONS its enclosing operator, not splicing the value (do-item abort-abandon)"
  (doc
    "The do-spine SPLICE face, breaker ax9. A do-item `+ 999 <bail>` — a pure sibling 999 plus an
           aborting operand — sits on the do-spine after a committed A.tick, under B. The abort abandons the
           whole `+`, so the do-item's value IS the abort value 99, with A.tick's advance preserved as the
           do-prefix. Before the fix the Apply-arm rebuilt `+ 999 99` — SPLICING the abort value into the dead
           arithmetic — and as a do-ITEM nothing collapsed it: the reduce_handle top-level bare-abort collapse
           only fires at the whole-body position, so b was `do A.tick then 1098` = 1098 giving 1109. Fixed:
           when an operator's operand aborts and its other operands are pure, with no foreign prefix to keep,
           the Apply-arm collapses to the abort value directly, dropping the dead pure siblings, so b = 99 and
           the outer A.get reads the advanced 11 giving 99 + 11 = 110.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (+ 999 (B.bail 99)))))) (+ b (A.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "a later let-binding abort preserves an EARLIER binding's committed advance (multi-binding prefix)"
  (doc
    "The MULTI-BINDING face of the let-init abort collapse, breaker ax12. Two bindings: `y` = A.tick,
           then `x` = an aborting init `+ 1 <B.bail 99>`. The FIRST binding commits A-state 10 to 11 before
           the SECOND binding aborts B. The abort abandons the let so b = the abort value 99 — but the
           earlier `y = A.tick` advance must SURVIVE. Before the fix the let-init abort collapse returned just
           the aborting init's rewrite, dropping the earlier bindings, so the outer A.get read the seed 10
           giving 109. Fixed: the collapse sequences the earlier bindings' foreign inits as a for-effect do
           prefix before the abort value, so A.tick runs and A.get reads 11 giving 99 + 11 = 110. A `let`-wrap
           would not do: an unused binding whose init performs is dead-code dropped.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (let ((y (A.tick)) (x (+ 1 (B.bail 99)))) (+ x y)))))
            (+ b (A.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "an inner abort preserves an OUTER advance committed in an IF-BRANCH before it (branch do-shape)"
  (doc
    "The IF-BRANCH face of the outer-advance preservation (v-effects self-probe; the direct do-shape and
           strict-operand faces are pinned above). The foreign `(A.tick)` and the abort sit on the strict
           do-spine of an `if` BRANCH — `(if true (do (A.tick) (B.bail 99)) 5)` under B. The branch's abort is
           branch-local (the `if` is the inner handle's value), but the pre-abort `A.tick` committed A-state
           10→11 and must survive → outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fix the
           branch-local collapse (`thread_branch_local_abort_with_out`) returned the BARE abort value,
           discarding the do-arm's sound `(do (A.tick) 99)` branch rewrite → 109. Fixed by the same do-shape
           gate as the direct fold, applied to the branch rewrite (the `if` condition is pure — a performing
           condition with a second branch advance is a separate face).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (if true (do (A.tick) (B.bail 99)) 5))))
            (+ b (A.get)))))
      (export main)))
  (output (: 110 Int64)))

(case
  "an inner abort preserves BOTH a PERFORMING-CONDITION advance AND an if-branch advance before it (ao10)"
  (doc
    "The performing-condition face — the separate face the if-branch case above flagged. The `if`
           CONDITION performs the outer `(A.tick)` AND the taken branch performs another `(A.tick)` before the
           abort: `(if (> (A.tick) 5) (do (A.tick) (B.bail 99)) 5)` under B. BOTH A-advances must survive: the
           condition `A.tick` reads 10, commits 10→11 (10>5 true → then-branch); the branch `A.tick` reads 11,
           commits 11→12; then `B.bail` aborts B (b = 99). Outer `(A.get)` reads 12 → `(+ 99 12)` = 111. Before
           the fix the branch advance was DROPPED (110): the if-arm state-merge skips a performing condition
           (`cond_pure=false`), so the branch's A-advance never threads to the continuation. Fixed by extending
           the Site-5 `#cv`-lift to bind a performing condition to `(let ((#cv (> (A.tick) 5))) (if #cv …))`
           when a branch also performs AND the one-shot refold does not serve the body — making the condition
           pure so the branch-advance merge proceeds, exactly as a hand let-bound condition already did. The
           refold-servability gate keeps this off the E5 leading-hole refold shapes (which the more-specific
           refold serves). Distinct from the pure-condition if-branch face above (which the do-shape gate
           already handled) — here the CONDITION itself performs.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (if (> (A.tick) 5) (do (A.tick) (B.bail 99)) 5))))
            (+ b (A.get)))))
      (export main)))
  (output (: 111 Int64)))

(case
  "an inner abort preserves an OUTER advance committed in a MATCH-ARM body before it (arm do-shape)"
  (doc
    "The MATCH-ARM-BODY face of the outer-advance preservation, sharing the branch-local abort helper
           with the if-branch face above. The foreign `(A.tick)` and the abort sit on the strict do-spine of a
           `match` ARM body — `(match 0 (_ (do (A.tick) (B.bail 99))))` under B. The arm's abort is arm-local,
           but `A.tick` committed A-state 10→11 → outer `(A.get)` reads 11 → 110. Same fix + gate as the
           if-branch (both route through `thread_branch_local_abort_with_out`, so one fix covers both branch
           and arm-body positions); before it the arm collapse dropped `A.tick` → 109.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((bail (v) s v)) (match 0 (_ (do (A.tick) (B.bail 99)))))))
            (+ b (A.get)))))
      (export main)))
  (output (: 110 Int64)))

(case
  "a single handler with both a resuming and an abortive arm dispatches each op to its own arm kind"
  (doc
    "One handler for ONE effect `E` declaring TWO operations whose arms are DIFFERENT KINDS — `get`
           resumes, `bail` abandons — so the fold must dispatch each performed op to its own arm kind within
           a single handler context (distinct from the nested three-separate-handler abort above, where each
           kind is its own handler). Body `(+ (E.get) (E.bail 7))` seeded 0: `E.get` resumes with 5, then
           `E.bail 7` — a NON-resuming arm — ABANDONS the pending `(+ 5 …)` and yields the arm value 7 as the
           whole handle's value (NOT 5+7). Pins that a mixed-arm handler routes the resuming op through the
           resume fold AND the abortive op through the non-local exit, in one handler.")
  (input
    (do
      (effect E (op get (-> Unit Int64)) (op bail (-> Int64 Int64)))
      (def (main) (handle E 0 ((get (u) s (resume 5 s)) (bail (b) s b)) (+ (E.get) (E.bail 7))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a single mixed handler uses only its resuming arm when the abortive op is never performed"
  (doc
    "The control companion of the mixed-arm case above: the SAME two-op handler (`get` resuming,
           `bail` abortive) but the body performs ONLY the resuming op — the abortive arm is present but
           never reached, so nothing abandons. Body `(+ (E.get) 100)` seeded 0: `E.get` resumes with 5,
           `(+ 5 100)` = 105. Pins that the mere PRESENCE of an abortive arm does not perturb the resuming
           path — the handle folds to the ordinary resumed value when the abortive op is not performed.")
  (input
    (do
      (effect E (op get (-> Unit Int64)) (op bail (-> Int64 Int64)))
      (def (main) (handle E 0 ((get (u) s (resume 5 s)) (bail (b) s b)) (+ (E.get) 100)))
      (export main)))
  (output (: 105 Int64)))

(case
  "when two abortive performs sit on one spine the FIRST (leftmost) abort wins"
  (doc
    "Refines the abortive class for MULTIPLE performs. Operands evaluate LEFT-TO-RIGHT, and an
           abortive perform ABANDONS the rest of the computation, so on `(+ (Bail.bail 7) (Bail.bail 9))` the
           FIRST operand `(Bail.bail 7)` fires first and abandons everything — the handle evaluates to 7, and
           the second `(Bail.bail 9)` never runs. The result is the leftmost abort's value, never the second,
           mirroring the left-to-right evaluation order the strict operator imposes.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 7) (Bail.bail 9))))
      (export main)))
  (output (: 7 Int64)))

(case
  "with three abortive performs on a strict spine the leftmost still wins"
  (doc
    "The deeper form of the first-wins rule (a regression pin against a shared-abort-cell that kept
           threading past the first abort and let a later one overwrite it). `(+ (Bail.bail 7) (+ (Bail.bail
           8) (Bail.bail 9)))` evaluates left-to-right, so the leftmost `(Bail.bail 7)` fires first and
           abandons everything → 7; neither `(Bail.bail 8)` nor `(Bail.bail 9)` runs. Pins that once the
           abort value is set, a later abort at any nesting depth does NOT overwrite it.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 7) (+ (Bail.bail 8) (Bail.bail 9)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "the winning abort's value reads the op arg and the seed state"
  (doc
    "The first-wins rule with an abort arm that READS both the op arg and the handler state:
           `(bail (n) s (+ n s))` seeded 5, body `(+ (Bail.bail 7) (Bail.bail 9))`. The leftmost
           `(Bail.bail 7)` fires first, its arm value `(+ 7 5)` = 12 becomes the handle value; the second
           abort is dead. Pins that the winning abort's value is computed from its own op arg and the live
           seed state, and the loser never perturbs it.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 5 ((bail (n) s (+ n s))) (+ (Bail.bail 7) (Bail.bail 9))))
      (export main)))
  (output (: 12 Int64)))

(case
  "an unconditional cross-function abort folds via inline"
  (doc
    "A helper whose body is a BARE abort — `(def (boom n) (Bail.bail n))` — called in a non-tail
           strict position `(+ 10 (boom 99))`. Inlining `boom` yields `(+ 10 (Bail.bail 99))`, a plain
           unconditional strict abort that abandons the enclosing `+ 10`, so the handle yields the arm value
           99. Pins that an UNCONDITIONAL cross-function abort folds via inline (distinct from a CONDITIONAL
           cross-function abort, which declines pending the non-local-exit convention).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (boom (: n Int64)) (Bail.bail n))
      (def (main) (handle Bail 0 ((bail (n) s n)) (+ 10 (boom 99))))
      (export main)))
  (output (: 99 Int64)))

(case
  "an abortive perform in the tail of an if branch abandons only that branch"
  (doc
    "Refines the abortive class for a CONDITIONAL early-exit. `Bail.bail` is abortive (its arm never
           resumes). The handle body is `(if true (Bail.bail 7) 99)` — the `if` IS the handle's value, so an
           abort in a branch's TAIL is LOCAL to that branch: the true branch aborts, yielding the arm value
           7; the false branch, had it run, would yield 99 (its sibling survives — the abort does not
           collapse the whole handle). This is the 'bail on one path, fall through on the other' shape a
           validation routine takes. Contrast a NON-tail conditional abort (`(+ 1 (if c (Bail.bail 7) 0))`),
           where the abort must escape the enclosing `+` — that needs a control block the perform `br`s out
           of and is not yet reducible. Here the branch tail is the handle value, so the fold is per-branch:
           `(if true 7 99)` → 7 (`DESIGN-effects-rcdzc.md` §4.2).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (if true (Bail.bail 7) 99)))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abortive perform in the NON-taken if branch is never evaluated (no speculation)"
  (doc
    "The soundness complement of the taken-branch abort above, and a pin against SPECULATIVE branch
           evaluation (e.g. a branchless-`select` lowering that would eagerly compute both arms): the abort
           sits in the branch that is NOT taken and MUST NOT fire. `Bail.bail` is abortive (its arm `(bail
           (n) s n)` never resumes, yielding `n` as the handle value). The body `(if (< 3 5) 10 (Bail.bail
           99))` takes the true branch (`3 < 5`), so the handle evaluates to `10`; the else-branch's abort is
           dead code that never runs. Were the compiler to evaluate both branches (speculating the abort),
           the handle would wrongly collapse to `99`. Pins that an abortive perform in a non-taken branch is
           genuinely conditional — only the taken path's effects occur — which the branch-local fold and any
           branchless-select conversion must both preserve. The control (flip to `(> 3 5)`, abort taken)
           yields 99.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (if (< 3 5) 10 (Bail.bail 99))))
      (export main)))
  (output (: 10 Int64)))

(case
  "a runtime-conditioned if-branch perform distributes the handler per branch"
  (doc
    "A RESUMING handler over an `if` whose condition is a RUNTIME parameter (a genuine PHI, not
           const-folded): the handle distributes into each branch — `(if (< x 5) (handle … (+ 1 (Amb.flip)))
           (handle … (* 2 (Amb.flip))))` — and each sub-handle folds against its own continuation. The
           handler `(flip (u) s (+ 1 (resume 10 s)))` resumes 10 and adds 1 to the continuation result. x=3
           (< 5) → then-branch continuation `C = (+ 1 □)` → `(+ 1 (+ 1 10))` = 12; x=9 → else-branch `C =
           (* 2 □)` → `(+ 1 (* 2 10))` = 21. Pins that the condition is evaluated once at run time and only
           the taken branch's fold runs (no speculation of the other branch's perform).")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main (: x Int64))
        (handle
          Amb
          0
          ((flip (u) s (+ 1 (resume 10 s))))
          (if (< x 5) (+ 1 (Amb.flip)) (* 2 (Amb.flip)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 12 Int64))
  (call main (: 9 Int64))
  (output (: 21 Int64)))

(case
  "an abortive perform in the tail of an if branch inside a let body abandons only that branch"
  (doc
    "The branch-tail abort composes through a `let`: a `let`'s VALUE is its BODY's value, so a `let`
           body is in the same tail position as the `let` itself. `(let ((k 5)) (if true (Bail.bail 7) k))`
           — the `if` is the let body's tail, which is the handle's value — so the abort in the true branch
           is LOCAL to that branch (yields the arm value 7); the false branch, had it run, would yield the
           bound `k` = 5 (the sibling survives). Pins that the abortive fold's tail-position reasoning
           descends into a `let` body, not just a bare `if` (`DESIGN-effects-rcdzc.md` §4.2). Contrast an
           abort in a NON-tail `let` INIT (`(let ((k (if c (Bail.bail 7) 0))) …)`), which must escape into
           `k` and is not yet reducible.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (let ((k 5)) (if true (Bail.bail 7) k))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a handle body reads an enclosing function parameter"
  (doc
    "The handle body is not closed — it may reference a binding from the enclosing scope, exactly as
           any other expression does. `main`'s parameter `x` is read directly in the handle body `(+ x
           (Get.get 0))`: the `Get` handler resumes 5, so the body is `x + 5`. Called with `x = 10` the
           result is 15. Pins that the tail-resumptive fold's rewritten body still resolves a FREE variable
           up the original lexical chain — the fold synthesizes a fresh body subtree, which must remain
           anchored where the `handle` sat so `x` reaches `main`'s parameter binder (not a spurious unbound
           name). Runtime parameters are what make an effectful body more than a constant.")
  (input
    (do
      (effect Get (op get (-> Int64 Int64)))
      (def (main (: x Int64)) (handle Get 0 ((get (n) s (resume 5 s))) (+ x (Get.get 0))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 15 Int64)))

(case
  "a handler ARM gates its answer on a CAPTURED Set from the enclosing scope"
  (doc
    "The arm-side twin of the body-reads-enclosing-parameter case above: it is the ARM (not the body)
           that reaches a heap value defined in `main`'s scope. `allow = Set.of [2 5 9]` is captured by the
           `check` arm, which answers `(if (Set.contains allow v) 1 0)` per op ARGUMENT. Three membership
           probes (5 ∈, 3 ∉, 9 ∈) place-value to 101. Pins that the fold keeps the arm anchored where the
           handler sat lexically, so a free heap binding resolves up the original chain from inside the arm.")
  (input
    (do
      (effect St (op check (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (do
          (def allow #set(2 5 9))
          (handle
            St
            0
            ((check (v) s (resume (if (Set.contains allow v) 1 0) s)))
            (+ (* 100 (St.check n)) (+ (* 10 (St.check 3)) (St.check 9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 101 Int64)))

(case
  "Set.contains on a Map-looked-up Set with a perform-threaded element"
  (doc
    "The set/elem same-base emit witnessed clean (the mixed-width siblings of this shape — a
           looked-up closure applied to a perform result, and Bytes.slice of a looked-up Bytes with
           perform operands — were i32/i64 scratch-alias miscompiles, both fixed and pinned): the Set
           comes back through `Map.lookup` and the membership PROBE is a perform result. Same-width
           slots cannot type-collide; this pins no value clobber either — 5 ∈ {2 5 9} → 10, 6 ∉ → 0 →
           10.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def table #map((= 1 #set(2 5 9))))
          (handle
            St
            n
            ((next (u) s (resume s (+ s 1))))
            (match
              (Map.lookup table 1)
              ((Some st)
                (+ (if (Set.contains st (St.next)) 10 0) (if (Set.contains st (St.next)) 1 0)))
              ((None _u) -200)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

(case
  "two sequential lookups on the same Map with perform-threaded keys stay independent"
  (doc
    "The map/key same-base emit witnessed clean (see the sibling pin above for the fixed
           mixed-width class): two `Map.lookup inner (St.next)` calls in one sum, each key a fresh
           perform result (5 → 100, 6 → 250 → 350). Pins that consecutive lookup emits with live
           perform-threaded key operands do not share (or clobber) scratch state.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def inner #map((= 5 100) (= 6 250)))
          (handle
            St
            n
            ((next (u) s (resume s (+ s 1))))
            (+
              (match (Map.lookup inner (St.next)) ((Some v) v) ((None _u) -1))
              (match (Map.lookup inner (St.next)) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 350 Int64)))

(case
  "a TWO-resume-site arm branching on a CAPTURED Map folds — the branch reads heap, not state"
  (doc
    "The first-served face of the multi-resume-site family: an arm with two resume sites carrying
           DIFFERENT states per site — the hit path advances the count `(resume v (+ s 1))`, the miss path
           holds it `(resume 0 s)` — folds through FOUR performs, branching on a CAPTURED Map
           (`Map.lookup table k`). (Historically the state-reading sibling declined and this captured-heap
           face pinned the boundary; the two-hole refold re-anchor now serves state-reading conditions
           too — the match-arm and state-condition faces are pinned nearby.) Lookups: 1→100 (s→1),
           7→miss→0 (s stays 1), 2→250 (s→2), then `hits` reports 2 → 100+0+250+2000 = 2350. Pins the
           captured-table routing idiom — a real-world lookup-with-hit-count handler.")
  (input
    (do
      (effect St (op price (-> Int64 Int64)) (op hits (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def table #map((= 1 100) (= 2 250)))
          (handle
            St
            0
            ((price
                (k)
                s
                (match (Map.lookup table k) ((Some v) (resume v (+ s 1))) ((None _u) (resume 0 s))))
              (hits (u) s (resume s s)))
            (+ (St.price 1) (+ (St.price 7) (+ (St.price 2) (* 1000 (St.hits))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2350 Int64)))

(case
  "a two-site arm branching on the STATE folds (the refold re-anchor serves state-reading conditions)"
  (doc
    "Historically THE decline face of the multi-site family — `(if (> s 5) …)` reads the state binder
           and the arm resumes in both branches — now served by the two-hole refold re-anchor (the
           #2305-era fix; condition-agnostic). Seed 7 never changes (`(resume v s)` / `(resume -1 s)`), so
           both reads take the true branch: 5 + 10·6 = 65. The never-miscompile lib pin asserts this same
           fold; the corpus case pins it end-to-end on all three targets.")
  (input
    (do
      (effect Src (op read (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Src
          7
          ((read (v) s (if (> s 5) (resume v s) (resume -1 s))))
          (+ (Src.read n) (* 10 (Src.read (+ n 1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 65 Int64)))

(case
  "a trailing state-REPLACING single-site op is served after a two-site arm's performs"
  (doc
    "The arm-shape MIXING boundary, trailing-served face: the refold serves any mix of MULTI-site
           arms in any dispatch order, but a SINGLE-site arm (like `reset` here) dispatched among
           multi-site performs declines — UNLESS it trails, as here: sift 20 → 20 (s 1), sift 30 → 30
           (s 2), reset → 2 (state becomes 100, unobserved) → 52. A trailing dispatch sits outside the
           multi-site continuation chain and folds; the same reset dispatched before or between the
           sifts declines (that face is pinned as a todo-witness nearby). Making the interleaved arm
           itself multi-site serves the same order — the rule is arm-shape uniformity at the handler's
           own frame, not dispatch position per se.")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)) (op reset (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))) (reset (u) s (resume s 100)))
          (+ (St.sift 20) (+ (St.sift n) (St.reset)))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 52 Int64)))

(case
  "a trailing state-READING single-site op is served after one two-site perform"
  (doc
    "The minimal trailing-served face of the arm-shape mixing boundary: ONE two-site sift then a
           trailing single-site peek — sift 20 passes (s → 1), peek reads 1 → 21. With the
           multi-perform sibling above, pins that trailing serves regardless of perform count — while
           even a single LEADING single-site dispatch (peek first) declines. The full rule: multi-site
           arms mix freely; single-site arms among multi-site performs decline except trailing.")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)) (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))) (peek (u) s (resume s s)))
          (+ (St.sift 20) (St.peek))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 21 Int64)))

(case
  "TWO different two-site ops dispatched in segments fold (multi-multi mixing, A A B B)"
  (doc
    "Arm-shape uniformity, the two-op face: BOTH arms are two-site, dispatched as two siftAs then
           two siftBs — 20 pass (s 1), 3 fail, 7 pass ×2 (s 11), 4 fail → 20 + 0 + 14 − 1 = 33. With
           the interleaved sibling below, pins that any mix of MULTI-site arms folds regardless of
           dispatch grouping — the single-site-among-multi decline is about arm SHAPE, not op count.")
  (input
    (do
      (effect St (op siftA (-> Int64 Int64)) (op siftB (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((siftA (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
            (siftB (v) s (if (> v 5) (resume (* v 2) (+ s 10)) (resume -1 s))))
          (+ (St.siftA 20) (+ (St.siftA 3) (+ (St.siftB 7) (St.siftB 4))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 33 Int64)))

(case
  "two two-site ops INTERLEAVED A-B-A fold (order does not matter when all arms are multi-site)"
  (doc
    "The interleave face of multi-multi mixing: siftA, then siftB, then siftA again — the exact
           dispatch pattern that DECLINES when the middle arm is single-site folds when it is two-site.
           20 pass (s 1), 7 doubled (s 11), 30 pass (s 12) → 20 + 14 + 30 = 64. The strongest witness
           that the boundary is arm-shape uniformity, not dispatch position.")
  (input
    (do
      (effect St (op siftA (-> Int64 Int64)) (op siftB (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((siftA (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
            (siftB (v) s (if (> v 5) (resume (* v 2) (+ s 10)) (resume -1 s))))
          (+ (St.siftA 20) (+ (St.siftB 7) (St.siftA 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 64 Int64)))

(case
  "the interleaved middle op SERVES when made two-site itself (the shape-not-position witness)"
  (doc
    "The decisive discriminator: the sift-peek-sift order whose single-site peek declines is
           served verbatim once peek's arm is TWO-site — `(if (> s 0) (resume s s) (resume -1 s))`.
           sift 20 → 20 (s 1), peek → 1 (s > 0 path), sift 30 → 30 (s 2) → 51. Same program order,
           only the arm shape changed: the refold rebuilds all dispatched arms in one pass and serves
           any all-multi-site mix.")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)) (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
            (peek (u) s (if (> s 0) (resume s s) (resume -1 s))))
          (+ (St.sift 20) (+ (St.peek) (St.sift 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 51 Int64)))

(case
  "a THREE-site arm (nested if, three resume sites) folds"
  (doc
    "The refold generalizes past two resume sites: a nested-if arm with THREE resumes — >20 pays
           ×10 and jumps the state, >10 passes and counts, else zero-holds. rank 25 → 250 (s 100),
           rank 15 → 15 (s 101), rank 5 → 0 → 265. Site count is not the boundary; arm-shape mixing
           is.")
  (input
    (do
      (effect St (op rank (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((rank
              (v)
              s
              (if
                (> v 20)
                (resume (* v 10) (+ s 100))
                (if (> v 10) (resume v (+ s 1)) (resume 0 s)))))
          (+ (St.rank 25) (+ (St.rank 15) (St.rank n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 265 Int64)))

(case
  "a MATCH-shaped arm with three resume sites folds (sum-dispatch, not if)"
  (doc
    "The refold is not if-specific: the arm dispatches on `(% v 3)` through a MATCH with a resume
           in every branch — 6 → ×10 (s+1), 7 → identity, 5 → negated (s+100): 60 + 7 − 5 = 62. Pins
           multi-site service for match-shaped arm bodies alongside the nested-if face above.")
  (input
    (do
      (effect St (op class (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((class
              (v)
              s
              (match
                (% v 3)
                (0 (resume (* v 10) (+ s 1)))
                (1 (resume v s))
                (_ (resume (- 0 v) (+ s 100))))))
          (+ (St.class 6) (+ (St.class 7) (St.class n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 62 Int64)))

(case
  "a match-shaped arm body peels its resume per branch when performed in a sequence"
  (doc
    "A two-op `Db` handler where the `put` arm DESTRUCTURES its arg with a `match` and resumes inside
           the branch, performed in a `do`-SEQUENCE with a later `get` that READS the state `put` wrote.
           The `put` arm `(match kv ((tuple k v) (resume unit (+ k v))))` threads a new state from its
           PATTERN BINDERS (`(+ k v)`); the resume-peel must handle a MATCH-shaped arm body (not just a
           bare or `do`-shaped one), keeping the branch next-state scoped to its binders. `(do (Db.put
           (tuple 1 41)) (Db.get 0))`: put threads state 1+41 = 42, get reads it → 42.")
  (input
    (do
      (effect Db (op put (-> (Tuple Int64 Int64) Unit)) (op get (-> Int64 Int64)))
      (def
        (main)
        (handle
          Db
          0
          ((put (kv) s (match kv (#tuple(k v) (resume unit (+ k v))))) (get (k) s (resume s s)))
          (do (Db.put #tuple(1 41)) (Db.get 0))))
      (export main)))
  (output (: 42 Int64)))

(case
  "a THREE-site and a TWO-site arm interleaved fold (site counts mix freely)"
  (doc
    "Exact site-count uniformity is NOT required — a 3-site rank and a 2-site sift interleave
           (rank, sift, rank) and fold: 250 (s 100), 7 (s 110), 15 (s 111) → 272. Only the
           multi-vs-single distinction gates the mix.")
  (input
    (do
      (effect St (op rank (-> Int64 Int64)) (op sift (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((rank
              (v)
              s
              (if
                (> v 20)
                (resume (* v 10) (+ s 100))
                (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
            (sift (v) s (if (> v 5) (resume v (+ s 10)) (resume -1 s))))
          (+ (St.rank 25) (+ (St.sift 7) (St.rank n)))))
      (export main)))
  (call main (: 15 Int64))
  (output (: 272 Int64)))

(case
  "a trailing ABORT after multi-site performs reads the fully-advanced state"
  (doc
    "The abort corollary of arm-shape mixing: an aborting arm has ZERO resume sites, so it counts
           as non-multi — dispatched between multi-site performs it declines, but TRAILING it folds:
           sift 20 (s 1), sift 30 (s 2), then bail aborts with s·10 = 20 and the +1000 shell proves
           the continuation is discarded → 1020. The abort must read the state BOTH sifts advanced.")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)) (op bail (-> Unit Int64)))
      (def
        (main (: n Int64))
        (+
          1000
          (handle
            St
            0
            ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))) (bail (u) s (* s 10)))
            (+ (St.sift 20) (+ (St.sift n) (St.bail))))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 1020 Int64)))

(case
  "the arm-shape rule is FRAME-RELATIVE: a nested single-site handler dispatching mid-sequence is invisible"
  (doc
    "A multi-site OUTER handler folds even though a nested single-site handler (a SEPARATE
           effect) dispatches between the outer sifts — from the outer refold's frame its own dispatch
           sequence is contiguous sift-sift; the inner bump belongs to the nested frame below. 20 (s 1)
           + 100 (inner, t → 110) + 30 (s 2) → 150. (The inverse — an OUTER perform escaping through a
           multi-site INNER handler's chain — declines: that dispatch IS foreign at the inner frame.)")
  (input
    (do
      (effect Out (op sift (-> Int64 Int64)))
      (effect In (op bump (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Out
          0
          ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
          (handle
            In
            100
            ((bump (u) t (resume t (+ t 10))))
            (+ (Out.sift 20) (+ (In.bump) (Out.sift n))))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 150 Int64)))

(case
  "a two-site arm branching on the OP ARGUMENT with a hit-count state folds (threshold sift)"
  (doc
    "The op-argument face of the served multi-site family: `(if (> v 10) …)` reads the op PARAM;
           the pass path resumes the value and counts it, the fail path resumes 0 and holds. Three sifts
           (20 pass, 5 fail, 30 pass) then the count: 20 + 0 + 30 + 2·1000 = 2050. With the state-reading
           face above and the captured-heap face before it, the family folds regardless of WHAT the
           condition reads.")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))) (count (u) s (resume s s)))
          (+ (St.sift 20) (+ (St.sift n) (+ (St.sift 30) (* 1000 (St.count)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2050 Int64)))

(case
  "a two-site arm whose condition reads the ARG AND the STATE together folds"
  (doc
    "The compound face: `(> v s)` compares the op argument against the CURRENT state, so the branch
           decision itself depends on how many hits came before — sift 5 at s=0 passes (s→1), sift 0 at
           s=1 fails, sift 3 at s=1 passes (s→2): 5 + 0 + 3 + 2·1000 = 2008. The strongest single witness
           that the refold's re-anchored continuation sees the LIVE state at every dispatch.")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sift (v) s (if (> v s) (resume v (+ s 1)) (resume 0 s))) (count (u) s (resume s s)))
          (+ (St.sift n) (+ (St.sift 0) (+ (St.sift 3) (* 1000 (St.count)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2008 Int64)))

(case
  "a multi-site arm folds while the handle BODY reads an enclosing binding (the re-anchored free var)"
  (doc
    "REPRO of the body-free-var orphan (breaker pm-family; fixed by the two-hole refold re-anchor):
           with a two-site arm and ≥2 performs, the multi-perform continuation-rebuild used to copy the
           surrounding body WITHOUT re-anchoring `n`, so this valid program hit a false CDZ0101 'unbound
           name n'. Now `n` resolves up the original chain: 5 + 100 + 111 + 111 = 327. The single-perform
           and single-site siblings never broke; ≥2 performs × multi-site × a body free-var was the exact
           conjunct.")
  (input
    (do
      (effect St (op price (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
          (+ n (+ (St.price 1) (+ (St.price 7) (St.price 2))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 327 Int64)))

(case
  "a LET-bound local (not a param) survives the multi-site continuation rebuild"
  (doc
    "The let-binder face of the body-free-var repro above: `m = n·2` is a derived local read after
           three performs through a two-site arm. The orphan hit ANY enclosing binder (params and lets
           alike); the re-anchor restores both. 10 + 100 + 111 + 111 = 332.")
  (input
    (do
      (effect St (op price (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (let
          ((m (* n 2)))
          (handle
            St
            0
            ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
            (+ m (+ (St.price 1) (+ (St.price 7) (St.price 2)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 332 Int64)))

(case
  "a two-site arm with HEAP resume values (empty vs two-element list) folds"
  (doc
    "The heap-payload face of the served multi-site family: the branches resume DIFFERENT list
           shapes — `(list v v)` on pass, `(list)` on fail — and the body consumes lengths at place
           values: grab 5 → len 2, grab 1 → len 0, grab 4 → len 2 → 2 + 0 + 200 = 202. Pins that the
           refold is not scalar-only: each dispatch's resume value allocates (or not) per its own branch.")
  (input
    (do
      (effect St (op grab (-> Int64 (List Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grab (v) s (if (> v 1) (resume #list(v v) (+ s 1)) (resume #list() s))))
          (+
            (List.len (St.grab n))
            (+ (* 10 (List.len (St.grab 1))) (* 100 (List.len (St.grab 4)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 202 Int64)))

(case
  "a two-site arm over a HEAP STATE with a body free-var and a second state-reading op folds"
  (doc
    "The heap-STATE face of the body-free-var family (breaker ts1, fixed #2336). Unlike the
           heap-resume-value case above, here the STATE itself is a `List` threaded through `s`, the
           arm has TWO resume sites, and a SECOND state-reading op (`tally`) reads the advanced state
           mid-chain — while the body reads main's param `n`. Before the fix the two-hole refold rebuilt
           the arm's `if` condition `(> v 10)` (op-arg `v`↦`n`) via push_list, overwriting the shared `n`
           node's parent and detaching it → false CDZ0101 'unbound n'. Now the substituted arm body is
           anchored + resolved before the refold rebuild (pin-before-copy). feed 20 pass ([20]), feed n=5
           miss (hold), feed 30 pass ([20,30]), tally = len 2 → 20 + 0 + 30 + 1000·2 = 2050.")
  (input
    (do
      (effect St (op feed (-> Int64 Int64)) (op tally (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #list()
          ((feed (v) s (if (> v 10) (resume v (List.push s v)) (resume 0 s)))
            (tally (u) s (resume (List.len s) s)))
          (+ (St.feed 20) (+ (St.feed n) (+ (St.feed 30) (* 1000 (St.tally)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2050 Int64)))

(case
  "a two-site arm with a second op that replaces the state mid-chain folds"
  (doc
    "A two-site arm mixed with a single-site op that replaces the state mid-chain (breaker sy1). `emit`
           is a two-site (multi-site) arm; the second op `flip` — `(flip (u) s (resume 0 (Symbol.of
           \"quiet\")))` — is SINGLE-site (resumes once, replacing the state), dispatched BETWEEN the two
           `emit` performs. This now FOLDS: `emit`'s arm resumes PER `if`-BRANCH — `(if (= s (Symbol.of
           \"loud\")) (resume (* v 100) s) (resume v s))` — which `peel_resume_from_arm_body` handles via the
           `if`-peel (the `if` analogue of the existing `match` peel: rebuild the value `(if cond v0 v1)` and
           the next-state `(if cond s0 s1)` over the same condition). Before that peel the whole handler
           declined because the arm's `if`-of-resumes was unpeelable. The fold: emit(5) sees `loud` → 5·100 =
           500, flip sets state `quiet` and resumes 0, emit(3) sees `quiet` → 3 → 500 + 0 + 3 = 503.")
  (input
    (do
      (effect St (op emit (-> Int64 Int64)) (op flip (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Symbol.of "loud")
          ((emit (v) s (if (= s (Symbol.of "loud")) (resume (* v 100) s) (resume v s)))
            (flip (u) s (resume 0 (Symbol.of "quiet"))))
          (+ (St.emit n) (+ (St.flip) (St.emit 3)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 503 Int64)))

(case
  "a state-destructuring arm under a multi-perform body folds to 6"
  (doc
    "A handler arm that DESTRUCTURES its state with a `match` and resumes inside EACH branch —
           `(get (u) s (match s ((Some n) (resume n s)) (None (resume 0 s))))` — folds under a MULTI-perform
           body when the branches thread the SAME next-state. The perform arm peels the resume from each match
           branch and rebuilds BOTH a value-match and a next-state-match over the (pure) scrutinee `s`, so the
           match-valued next-state threads forward to the next perform (`peel_resume_from_arm_body`). Over a
           `(Some 5)` seed, both `get`s read `(Some 5)` -> 5 (both branches thread `s` unchanged), so
           `(+ 1 5)` = 6.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          (Some 5)
          ((get (u) s (match s ((Some n) (resume n s)) (None (resume 0 s)))))
          (do (St.get) (+ 1 (St.get)))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "a single-perform body over a state-destructuring arm folds to 6"
  (doc
    "The single-perform companion of the multi-perform state-destructuring fold: one `(St.get)` under the
           same match-shaped resume arm over a `(Some 5)` seed. The `Some` branch resumes `n` = 5 threading `s`
           unchanged, so `(+ 1 5)` = 6. Confirms the match-shaped resume peel folds the base single-perform
           shape as well as the multi-perform one.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          (Some 5)
          ((get (u) s (match s ((Some n) (resume n s)) (None (resume 0 s)))))
          (+ 1 (St.get))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "a branch-divergent next-state under a multi-perform body folds to 18"
  (doc
    "A state-destructuring arm whose branches thread a DIFFERENT advanced state per branch —
           `(get (u) s (match s ((Some n) (resume n (Some (+ n 1)))) (None (resume 0 s))))` — folds, and to
           the CORRECT value: the match-valued next-state threads forward as a `(match arg (pat s)...)`
           expression whose branches carry each branch's own advanced state, so a subsequent perform reading
           the state re-evaluates the match against the (pure) arg and sees the right per-branch state. Over a
           `(Some 5)` seed, L->R the seed advances 5->6->7 (each `get` reads then the `(Some (+ n 1))`
           next-state bumps it), so `5 + (6 + 7)` = 18. A regression that re-declines OR folds to a wrong value
           (the state-threading ledger's wrong-branch hazard) is caught here.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          (Some 5)
          ((get (u) s (match s ((Some n) (resume n (Some (+ n 1)))) (None (resume 0 s)))))
          (+ (St.get) (+ (St.get) (St.get)))))
      (export main)))
  (call main)
  (output (: 18 Int64)))

(case
  "a non-tail mutual-recursive group observing a partner's out-state folds via the group multi-value fold"
  (doc
    "The GROUP-AWARE multi-value fold over a mutually-recursive SCC. `typeof` and `compute` mutually
           recurse, and `compute` performs `put` then reads the accumulated state via `get` AFTER recursing
           into `typeof` `(let ((child (typeof (- n 1)))) (+ child (St.get)))`. SINGLE-return specialization
           would thread the mutual-partner call with the incoming state and drop its advance — the post-call
           `get` would read a stale pre-recursion state (a dropped-advance miscompile). The group fold
           reserves the WHOLE SCC as multi-value: each member returns `(value, out-state)`, and a cross-def
           partner call is let-bound + out-state-projected (`(. t 1)`) like a self-call, so `compute`'s
           post-recursion `get` reads `typeof`'s ADVANCED out-state. `main(2)`: typeof(0)=get=0; compute(1)
           puts 1 (state 0->1), reads get=1, returns 0+1=1; compute(2) puts 2 (state 1->3), reads get=3,
           returns 1+3 = 4. Verifies the mutual partner's state advance threads to a later observer.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def (typeof (: n Int64)) (if (= n 0) (St.get) (compute n)))
      (def
        (compute (: n Int64))
        (let ((child (typeof (- n 1)))) (match (St.put n) (_ (+ child (St.get))))))
      (def
        (main (: k Int64))
        (handle St 0 ((get (u) s (resume s s)) (put (v) s (resume unit (+ s v)))) (typeof k)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 4 Int64)))

(case
  "a recursive performer whose recursion result feeds a HELPER call folds — no slot-less spec-body param"
  (doc
    "The specializer's spec-body must not SHARE an original-def param occurrence into the synthesized
           body. `walk` recurses, then a match arm feeds the recursion result AND a fresh `St.get` to a
           separate helper `combine`. Threading returns unchanged subtrees as-is, so the spec body could
           SHARE the original `n` occurrence (reached through the perform-arm-threaded `(St.get)` state
           `(+ (. t 1) n)`); `core_of` memoizes by node, so a shared original node carries a
           `Core::Param{ORIGINAL binder}` with no slot in the specialized function → 'parameter reference
           has no local slot' at emit when `combine` inlines + lowers it. Fixed by deep-fresh-copying the
           threaded spec body so every node is a fresh id that re-resolves against the spec signature. This
           is the shape EVERY real demand-query compiler takes (a node's children demanded, then combined by
           a pure helper), so it must fold. `main(3)`: walk(0)=get=0; then put(1) state 0->1, walk(1)=0+get(1)
           =1; put(2) state 1->3, walk(2)=1+get(3)=4; put(3) state 3->6, walk(3)=4+get(6)=10.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def
        (walk (: n Int64))
        (if
          (= n 0)
          (St.get)
          (let ((lt (walk (- n 1)))) (match (St.put n) (_ (combine lt (St.get)))))))
      (def (combine (: a Int64) (: b Int64)) (+ a b))
      (def
        (main (: k Int64))
        (handle St 0 ((get (u) s (resume s s)) (put (v) s (resume unit (+ s v)))) (walk k)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 10 Int64)))

(case
  "a recursive performer whose recursion result is fed DIRECTLY to a helper arg folds"
  (doc
    "The DIRECT-WRAP variant of the helper-fed-recursion-result shape: the recursion result is fed to
           the helper DIRECTLY as an argument inside a match arm — `(match (St.put n) (_ (double (loop (- n
           1)))))` — with NO intermediate `let` binding the result (the sibling `walk`/`combine` case above
           let-binds `lt` first). Same slot-fix root (deep-fresh-copy of the threaded spec body keeps the
           recursion-result arg from sharing a slot-less original param node), but exercises the recursion
           result as a bare helper ARG rather than a let-init. `loop 0 = St.get`; each level performs `St.put`
           then doubles the recursion result. main(3): put 3,2,1 threads state 0->6, loop0=get=6, then
           double x3: 6->12->24->48. main(1): put 1 -> state 1; loop0=get=1; double -> 2.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def (loop (: n Int64)) (if (= n 0) (St.get) (match (St.put n) (_ (double (loop (- n 1)))))))
      (def (double (: a Int64)) (+ a a))
      (def
        (main (: k Int64))
        (handle St 0 ((get (u) s (resume s s)) (put (v) s (resume unit (+ s v)))) (loop k)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 48 Int64))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "a tail mutual-recursive group where both partners perform folds"
  (doc
    "A TAIL mutually-recursive group where BOTH partners perform the discharged state op. `ping`/`pong`
           alternate, each performing `St.put n` (advancing the state) then TAIL-calling its partner; the base
           case reads the accumulated state via `St.get`. Every partner call is on the TAIL — nothing observes
           its out-state — so single-return specialization is sound: the partner's state advance is passed
           forward as its trailing state argument, and the base case's `get` reads the fully-accumulated
           state. main(4): ping puts 4, pong puts 3, ping puts 2, pong puts 1, base reads get -> 4+3+2+1 = 10.
           A dropped mutual-partner advance (a single-return miscompile) would not reach 10.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def (ping (: n Int64)) (if (= n 0) (St.get) (match (St.put n) (_ (pong (- n 1))))))
      (def (pong (: n Int64)) (match (St.put n) (_ (ping (- n 1)))))
      (def
        (main (: k Int64))
        (handle St 0 ((get (u) s (resume s s)) (put (v) s (resume unit (+ s v)))) (ping k)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 10 Int64)))

(case
  "an op-arg match outer with a state-match inner reading the payload computes per dispatch"
  (doc
    "An op arm whose OUTER match is over the op ARG and whose INNER match is over the STATE, where the
           inner branches read the outer op-arg payload binder DIRECTLY, computes the RIGHT value across
           multiple dispatches. The op-arg match must be folded (the arg is consumed at THIS dispatch, not
           threaded) BEFORE the state-match peel, or dispatch-2's own payload `k` would be conflated with
           dispatch-1's state-threaded value. main(5): dispatch1 Go 15, state Idle -> resume 15, state Run 15;
           dispatch2 Go 7, state Run(15) -> resume 15+7=22; sum 15+22 = 37 (a stale-payload freeze gives 45).
           main(0): Go 10 state Idle -> 10 (state Run 10); Go 7 state Run(10) -> 10+7=17; sum 10+17 = 27.")
  (input
    (do
      (type Mode (Idle) (Run Int64))
      (type Cmd (Go Int64))
      (effect M (op step (-> Cmd Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          (Mode.Idle)
          ((step
              (c)
              s
              (match
                c
                ((Cmd.Go k)
                  (match
                    s
                    ((Mode.Idle) (resume k (Mode.Run k)))
                    ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))))
          (+ (M.step (Cmd.Go (+ 10 n))) (M.step (Cmd.Go 7)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 37 Int64))
  (call main (: 0 Int64))
  (output (: 27 Int64)))

(case
  "a handler arm resuming per if-branch over the op arg folds"
  (doc
    "A handler ARM that RESUMES PER `if`-BRANCH, where the condition is over the OP ARG: `(get (nid) s
           (if (= nid 0) (resume 100 s) (resume 200 s)))` selects the resume value by a condition over the op
           arg. The `if`-peel rebuilds two `if`s over the same condition — the value `(if cond v0 v1)` and the
           next-state `(if cond s0 s1)` — the `if` analogue of the match peel. main(0) -> the nid==0 branch ->
           resume 100; main(7) -> the else branch -> resume 200.")
  (input
    (do
      (effect St (op get (-> Int64 Int64)))
      (def
        (main (: k Int64))
        (handle St 0 ((get (nid) s (if (= nid 0) (resume 100 s) (resume 200 s)))) (St.get k)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 100 Int64))
  (call main (: 7 Int64))
  (output (: 200 Int64)))

(case
  "a three-member memo group with a let-var-body recursion arm folds — the group threads the cache state"
  (doc
    "REGRESSION PIN for the specialized group-fold let-var-body stack-overflow (fixed on trunk by the
           mutual-group demand-perform-demand + nested-let state-fork landings). A memo-DB shape: `type-of`
           reads a cached value (`St.get`); on a miss it `cache-type`s the `compute-type` result, where
           `cache-type` WRITES the cache (`St.put`) and `compute-type`'s recursion arm is a LET-VAR-BODY
           `(let _v = type-of(..) in (let b = type-of(..) in b))` — a `let` whose body is a bound VARIABLE,
           not a dispatch. Pre-fix the group pre-check declined the nested-let/bare-var body and the fold fell
           to a single-return that dropped the cache-write's out-state, so the next `get` re-demanded forever
           (call-stack-exhausted); the group multi-value fold now threads each member's out-state across the
           three-def SCC. A LITERAL let-body always folded; only a VAR body tripped it — this pins the var
           case. `main(2)`: type-of(2) get=0 → compute-type(2) recurses type-of(1) twice (its var-body binds
           the second) → base compute-type(0)=5; cache-type puts along the way; the block yields 5. main(0):
           type-of(0) get=0 → compute-type(0)=5 → cache-type puts 5, returns 5. It must terminate with a
           value, not exhaust the stack. (The self-hosted compiler-ml `type_of` demand loop is this shape.)")
  (input
    (do
      (effect St (op get (-> Int64 Int64)) (op put (-> Int64 Int64)))
      (def
        (type-of (: id Int64))
        (let ((cur (St.get id))) (if (= cur 0) (cache-type id (compute-type id)) cur)))
      (def (cache-type (: id Int64) (: t Int64)) (let ((w (St.put t))) t))
      (def
        (compute-type (: id Int64))
        (if (= id 0) 5 (let ((_v (type-of (- id 1)))) (let ((b (type-of (- id 1)))) b))))
      (def
        (main (: k Int64))
        (handle St 0 ((get (id) s (resume s s)) (put (v) s (resume v v))) (type-of k)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 5 Int64))
  (call main (: 0 Int64))
  (output (: 5 Int64)))

; A do-local value def in a handle body must stay in scope for a perform's ARGUMENT (breaker FINDING;
; v-effects e49c698a1). The handle-body folds dropped non-final do-items and re-spliced only a survivor,
; orphaning a `(def v e)` that a later perform arg referenced → a false CDZ0101 'unbound name'; the
; semantically identical `let`-bound form rebuilt its scope and worked. The fix normalizes a leading
; do-local value def to a `let` up front in reduce_handle, so every consumer — including the perform-arg
; path — sees the scoped binding. Pinned as a REPRO + let-twin regression pair. (A do-def flowing into a
; RESUME arg in an arm, and a do-def in a NON-perform arg, were always fine — this was specific to the
; perform-arg path in the body.)
(case
  "a do-def value in a handle body flows into a perform's argument and stays in scope"
  (doc
    "FINDING repro (v-effects e49c698a1). Inside the handle body, `(def v (+ u 2))` is referenced from
           the ARGUMENT of `(Ask.ask v)` and again after it; before the fix the body fold orphaned `v` →
           CDZ0101 unbound. Now the do-local value def is scoped for the perform-arg path. `run 5`: v = 7,
           `(Ask.ask 7)` resumes 7·2 = 14, plus v = 7 → 21. Both backends.")
  (input
    (do
      (effect Ask (op ask (-> Int64 Int64)))
      (def
        (run (: u Int64))
        (handle Ask 0 ((ask (n) s (resume (* n 2) s))) (do (def v (+ u 2)) (+ (Ask.ask v) v))))
      (def (main) (run 5))
      (export main)))
  (output (: 21 Int64)))

(case
  "the let-twin of the do-def-into-perform-arg body computes the same value"
  (doc
    "The always-worked `let` oracle for the do-def-into-perform-arg case above: `(let ((v (+ u 2)))
           (+ (Ask.ask v) v))` — the `let` form was never orphaned, so it pins the target value the do-form
           must match. `run 5`: v = 7, `(Ask.ask 7)` resumes 14, + 7 = 21.")
  (input
    (do
      (effect Ask (op ask (-> Int64 Int64)))
      (def
        (run (: u Int64))
        (handle Ask 0 ((ask (n) s (resume (* n 2) s))) (let ((v (+ u 2))) (+ (Ask.ask v) v))))
      (def (main) (run 5))
      (export main)))
  (output (: 21 Int64)))

(case
  "a do-def NOT in the perform argument stays in scope for a later reference"
  (doc
    "The const-arg control of the do-def-into-perform-arg case: the do-def `v` is NOT in the perform
           argument (`(Ask.ask 3)` takes a constant), but `v` is still referenced AFTER the perform. `run 5`:
           v = 7, `(Ask.ask 3)` resumes 6, + v = 7 → 13. Pins that the do-def scoping holds when the perform
           takes a constant arg, not only when the perform consumes the do-def.")
  (input
    (do
      (effect Ask (op ask (-> Int64 Int64)))
      (def
        (run (: u Int64))
        (handle Ask 0 ((ask (n) s (resume (* n 2) s))) (do (def v (+ u 2)) (+ (Ask.ask 3) v))))
      (def (main) (run 5))
      (export main)))
  (output (: 13 Int64)))

(case
  "a do-def passed to a performing helper stays in scope"
  (doc
    "The via-helper face of the do-def-into-perform-arg case: the do-def `v` is passed to a HELPER
           that performs — `(poke v)` where `poke a = Ask.ask a`. Inlining `poke` threads its performing
           body with `v` as the arg; the do-def must stay in scope across the inline. `run 5`: v = 7,
           `(poke 7)` = `(Ask.ask 7)` resumes 14, + 1 = 15.")
  (input
    (do
      (effect Ask (op ask (-> Int64 Int64)))
      (def (poke (: v Int64)) (Ask.ask v))
      (def
        (run (: u Int64))
        (handle Ask 0 ((ask (n) s (resume (* n 2) s))) (do (def v (+ u 2)) (+ (poke v) 1))))
      (def (main) (run 5))
      (export main)))
  (output (: 15 Int64)))

(case
  "a do-def value in a handle body flows into an ABORTIVE perform's argument and stays in scope"
  (doc
    "The ABORTIVE companion of the resume-arg pin above (breaker matrix row 1, previously held as a
           separate bug). Inside the handle body, `(def v (+ u 2))` is referenced from the ARGUMENT of an
           ABORTIVE `(Bail.bail v)` — the arm `(bail (v) s (* 100 v))` never resumes, so performing it collapses
           the handle to the arm value. The do-def must stay in scope for the abort perform's arg exactly as for
           a resuming one; before the fix this false-rejected CDZ0101 'unbound v' (the resuming row was pinned
           separately, this abortive row was still held). `run 5`: v = 7, `(Bail.bail 7)` aborts, arm yields
           7·100 = 700. Same do-def-to-`let` normalization in reduce_handle covers the abort-arm path.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (run (: u Int64))
        (handle Bail 0 ((bail (v) s (* 100 v))) (do (def v (+ u 2)) (Bail.bail v))))
      (def (main) (run 5))
      (export main)))
  (output (: 700 Int64)))

(case
  "a chain of perform-fed let inits — each binding feeds the next perform's argument"
  (doc
    "The sequential-dependency face of let × effects: three lets where each init's perform takes
           the PREVIOUS binding as its argument — a = add(5) = 5 (s 1), b = add(a) = 6 (s 2),
           c = add(b) = 8 (s 3) → 5 + 6 + 8 = 19. Each binding must be fully resolved before the
           next dispatch marshals it; a stale or reordered binding read skews the whole chain. (The
           pinned let cases cover bindings used AFTER performs; this pins bindings FEEDING the next.)")
  (input
    (do
      (effect St (op add (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((add (v) s (resume (+ v s) (+ s 1))))
          (let ((a (St.add n))) (let ((b (St.add a))) (let ((c (St.add b))) (+ a (+ b c)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 19 Int64)))

(case
  "a perform in the body's IF CONDITION gates a second perform in the branch"
  (doc
    "Effect-gated dispatch, the true path: the condition's `(St.check)` fires (reads 5, state →
           6), 5 > 3 holds, so the branch's second check fires and reads the ADVANCED 6. The
           condition's dispatch must complete (and its advance commit) before the branch's dispatch
           reads the state.")
  (input
    (do
      (effect St (op check (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle St n ((check (u) s (resume s (+ s 1)))) (if (> (St.check) 3) (St.check) 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "the false branch: the condition's perform fires, the branch's does NOT (same program)"
  (doc
    "The other runtime path of the effect-gated dispatch above: at seed 1 the condition's check
           reads 1, 1 > 3 fails, and the untaken branch's perform must NOT fire — the answer is the
           else's 0, and a speculative or hoisted dispatch of the branch perform would be observable
           as a state advance (or a wrong value) here.")
  (input
    (do
      (effect St (op check (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle St n ((check (u) s (resume s (+ s 1)))) (if (> (St.check) 3) (St.check) 0)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 0 Int64)))

(case
  "performs in BOTH and-operands — the second fires when the first passes"
  (doc
    "Short-circuit booleans × effects, the fire-both path: both `and` operands perform. At seed 5
           the first check reads 5 (> 3 passes, state → 6), so the SECOND fires and reads 6 (> 4
           passes, state → 7); the trailing count reads the DOUBLE advance → 700. The state is a
           dispatch counter — the checksum encodes exactly how many operand performs ran.")
  (input
    (do
      (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((check (u) s (resume s (+ s 1))) (count (u) s (resume s s)))
          (if (and (> (St.check) 3) (> (St.check) 4)) (* 100 (St.count)) (St.count))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 700 Int64)))

(case
  "the and SHORT-CIRCUITS: the second operand's perform must NOT fire (state proves it)"
  (doc
    "The elision path of the same program: at seed 1 the first check reads 1 (> 3 FAILS), the
           `and` short-circuits, and the second operand's perform must NOT fire — the count reads
           exactly ONE advance (2). Short-circuit evaluation is the language's only by-value runtime
           expression elision; an eager or reordered boolean lowering would fire the second dispatch
           and read 3.")
  (input
    (do
      (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((check (u) s (resume s (+ s 1))) (count (u) s (resume s s)))
          (if (and (> (St.check) 3) (> (St.check) 4)) (* 100 (St.count)) (St.count))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "the OR short-circuits on a true first operand — the second perform must NOT fire"
  (doc
    "The `or` elision path: at seed 5 the first check reads 5 (> 3 holds), the `or`
           short-circuits, the second operand's perform never fires — the count reads exactly one
           advance (6). The or-twin of the and-elision pin above.")
  (input
    (do
      (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((check (u) s (resume s (+ s 1))) (count (u) s (resume s s)))
          (if (or (> (St.check) 3) (> (St.check) 0)) (St.count) (* 100 (St.count)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a false first operand falls through — the OR's second perform fires (same program)"
  (doc
    "The fall-through path: at seed 1 the first check reads 1 (> 3 fails), so the `or`
           evaluates its second operand — that perform fires (reads 2, > 0 holds) and the count
           reads TWO advances (3). With the three sibling pins, all four short-circuit paths carry
           dispatch-count-proven witnesses.")
  (input
    (do
      (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((check (u) s (resume s (+ s 1))) (count (u) s (resume s s)))
          (if (or (> (St.check) 3) (> (St.check) 0)) (St.count) (* 100 (St.count)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a perform under NOT in a condition (the negated dispatch gate)"
  (doc
    "The remaining boolean operator: the condition wraps the perform in `not` — check reads 1
           (> 3 fails), the not flips it, the then-branch runs and count reads the single advance
           (100·2 = 200). Completes the boolean-op set (and/or short-circuits + not) over effect
           dispatches.")
  (input
    (do
      (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((check (u) s (resume s (+ s 1))) (count (u) s (resume s s)))
          (if (not (> (St.check) 3)) (* 100 (St.count)) (St.count))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 200 Int64)))

; ============ Optimizer-exclusion chapter: LICM / CSE / DCE / inlining each have extensive pins
; for PURE code; these six pin the EFFECT-dispatch exclusion boundary that keeps those
; optimizations sound — a perform is never invariant, never a common subexpression, never dead,
; and never shared across inlined call sites. ============
(case
  "a recursive loop whose CONDITION performs — each iteration RE-dispatches (never hoisted)"
  (doc
    "The LICM exclusion: the loop bound is a perform against a SHRINKING quota (the arm
           decrements per read: 5, 4, 3, 2), so the loop terminates when i catches the falling bound
           — acc 0+1+2 = 3. A hoist that treated the 'invariant-looking' condition as pure would
           read the quota once (5) and run five iterations (acc 10). The pure-invariant LICM pins
           (incl. trap-equivalence) live in 02-binding; this is their effect-side boundary.")
  (input
    (do
      (effect St (op quota (-> Unit Int64)))
      (def (go (: i Int64) (: acc Int64)) (if (< i (St.quota)) (go (+ i 1) (+ acc i)) acc))
      (def (main (: n Int64)) (handle St n ((quota (u) s (resume s (- s 1)))) (go 0 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64)))

(case
  "two IDENTICAL performs are distinct dispatches — never CSE'd into one"
  (doc
    "The CSE exclusion, minimal form: `(+ (St.next) (St.next))` — two textually identical
           performs read 5 then 6 → 11. A common-subexpression merge would compute one dispatch and
           double it (10).")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle St n ((next (u) s (resume s (+ s 1)))) (+ (St.next) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "identical PURE subterms around distinct performs — pure sharing must not merge dispatches"
  (doc
    "The subtler CSE face: `(+ n 1)` appears identically in both products and is legitimately
           shareable — but the sharing must not merge or reorder the two dispatches between them:
           6·5 + 6·6 = 66. Pure value-numbering composes with effect sequencing.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (* (+ n 1) (St.next)) (* (+ n 1) (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 66 Int64)))

(case
  "a perform bound to an UNUSED binding still dispatches (DCE must not eliminate it)"
  (doc
    "The DCE exclusion: `_unused`'s VALUE is dead but its dispatch is not — the bump advances
           the state and the peek observes 6. A use-count-based eliminator that removed the dead-bound
           perform would read 5. (The do-spine discard pins cover syntactic discard; this is the
           bound-but-dead face DCE actually inspects.)")
  (input
    (do
      (effect St (op bump (-> Unit Int64)) (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((bump (u) s (resume s (+ s 1))) (peek (u) s (resume s s)))
          (let ((_unused (St.bump))) (St.peek))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a PURE dead binding beside a perform is harmless (the eliminable control)"
  (doc
    "The control for the DCE exclusion above: `_dead = n·999` is pure and genuinely
           eliminable — removing it changes nothing observable; the peek reads the untouched seed
           (5). The exclusion is about effects, not dead bindings generally.")
  (input
    (do
      (effect St (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle St n ((peek (u) s (resume s s))) (let ((_dead (* n 999))) (St.peek))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a performing helper called from TWO sites — each call site is its own dispatch"
  (doc
    "The inlining exclusion: `step k = k + (St.next)` is called twice; inline duplication of
           the performing body must keep PER-SITE dispatch — 1+5 = 6 (state → 6), then 2+6 = 8 →
           608. Sharing one dispatch across the inlined sites would read 5 twice (606). (The crash
           face of this shape — the eval-once inline's binder orphans — is the en1 family, tracked
           separately; this pins the VALUES when the inline works.)")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def (step (: k Int64)) (+ k (St.next)))
      (def
        (main (: n Int64))
        (handle St n ((next (u) s (resume s (+ s 1)))) (+ (* 100 (step 1)) (step 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 608 Int64)))

(case
  "a performing helper whose body BINDS the result and BRANCHES on it, called from two sites, folds"
  (doc
    "The en1 fix (breaker MED). The crash face of the two-site inline above: the helper's body binds the
           perform result in a `let` AND reads that binder in an `if` CONDITION — `f(x) = let r = x + St.bump
           in if r >= 100 then r else 0`. Inlined at TWO sites, the `if`-arm's per-branch state-merge used to
           build a state selector `(if cond r 0)` from the PURE value branches (neither performs) — because the
           merge gate was a node-IDENTITY compare (`then_out != cur`) that `deep_fresh_copy` makes ALWAYS true —
           so the value-`if` rode forward as the next-STATE, a later dispatch spliced it back as its resume
           value, the let-body `if` landed in the let's BINDINGS, and `r` (read in the if-cond) resolved UNBOUND
           → false CDZ0101. The fix gates the merge on an ACTUAL branch perform: neither branch performs → no
           state advance → keep the incoming state. f(5)=5+100=105, 105>=100 → 105 (state→101); f(2)=2+101=103,
           103>=100 → 103 → 105 + 103 = 208. The single-call + pure-init analogs always folded; the [let-bound
           perform result] × [if-cond reads it] × [≥2 inlined sites] conjunct was the exact crash.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (def (f (: x Int64)) (let ((r (+ x (St.bump)))) (if (>= r 100) r 0)))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (+ (f n) (f 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 208 Int64)))

(case
  "the en1 helper called ONCE folds (the single-site control)"
  (doc
    "The single-call control for the en1 fix: the SAME helper `f(x) = let r = x + St.bump in if r >= 100
           then r else 0` called ONCE always folded (one inline = no cross-site state-merge) — pins that the fix
           does not disturb it. f(5) = 5 + 100 = 105, 105 >= 100 → 105.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (def (f (: x Int64)) (let ((r (+ x (St.bump)))) (if (>= r 100) r 0)))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (f n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "constant conditions simplify around performs — kept branches dispatch, dropped ones do not"
  (doc
    "Constant folding × effects: `(if true (St.next) 999)` and `(if false 999 (St.next))` both
           have compile-time-constant conditions — the simplification keeps each surviving branch's
           dispatch, in order: 5 + 6 = 11. (Dropped branches here carry no performs; the
           dropped-branch-with-perform elisions are the short-circuit and if-gate pins above.)")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (if true (St.next) 999) (if false 999 (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a String op RESULT selected by the op argument, composed via concat"
  (doc
    "String-valued op results beyond the interner pins: the arm selects between literals by the
           op argument (positive → \\\"hi\\\", zero → \\\"lo\\\"), two dispatches compose through a concat
           chain, and the byte-length consumes the assembled \\\"hi-lo\\\" → 5. The message-building
           idiom.")
  (input
    (do
      (effect St (op word (-> Int64 String)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((word (k) s (resume (if (> k 0) "hi" "lo") (+ s 1))))
          (String.byte-len (String.concat (St.word n) (String.concat "-" (St.word 0))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a heterogeneous TUPLE as op ARGUMENT — the arm destructures both components"
  (doc
    "The argument-direction twin of the heterogeneous-tuple RESULT pin: `(Tuple String Int64)`
           in the op signature's argument position, destructured by the arm — byte-len \\\"abc\\\" +
           10·5 = 53. Mixed-type tuples now carry witnesses in both marshal directions, like records
           and user sums.")
  (input
    (do
      (effect St (op score (-> (Tuple String Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((score
              (p)
              s
              (match p (#tuple(name pts) (resume (+ (String.byte-len name) (* pts 10)) s)))))
          (St.score #tuple("abc" n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 53 Int64)))

(case
  "one perform result flows through let, record, projection, tuple, destructure, and match"
  (doc
    "The deep-composition smoke: a single effect-derived value travels the full consumer
           gauntlet — bound (v = 5), stored in a record field, projected twice, packed into a tuple
           with a derived companion (5, 15), destructured, compared (15 > 10), summed → 20. Each
           consumer kind is individually pinned; this chains them all on one dispatch's result to
           catch composition seams between the verified paths.")
  (input
    (do
      (effect St (op seed (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((seed (u) s (resume s (+ s 1))))
          (let
            ((v (St.seed)))
            (let
              ((r #record((= base v) (= scale 3))))
              (let
                ((p #tuple(r.base (* r.base r.scale))))
                (match p (#tuple(lo hi) (match (> hi 10) (true (+ lo hi)) (false 0)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64)))

(case
  "a pure HELPER's arguments evaluate left-to-right when each performs"
  (doc
    "The calling-convention face of dispatch order: a pure place-value function called with
           THREE performing arguments — they evaluate strictly left-to-right (5, 6, 7 → 567). The
           positional pins cover effect-OP operands; this pins a plain function call's argument
           evaluation order where each argument's dispatch makes the order observable.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def (place (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
      (def
        (main (: n Int64))
        (handle St n ((next (u) s (resume s (+ s 1)))) (place (St.next) (St.next) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 567 Int64)))

(case
  "a SET op RESULT crosses resume — membership-probed and measured per dispatch"
  (doc
    "The Set completion of the collection RESULT-direction crossings (Map, List, and Bytes op
           results carry pins; Set appeared only as handler STATE): the arm resumes a per-dispatch
           set — populated for a positive op argument, empty otherwise. The body membership-probes
           the populated one (contains 5 → 10) and measures the empty one (len 0) → 10. A CHAMP set
           marshaled out of the arm must support both query kinds on the resume side.")
  (input
    (do
      (effect St (op allowed (-> Int64 (Set Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((allowed (k) s (resume (if (> k 0) #set(2 5 9) #set()) s)))
          (+ (if (Set.contains (St.allowed n) 5) 10 0) (Set.len (St.allowed 0)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a SET as op ARGUMENT — the arm measures and probes the set it is handed"
  (doc
    "The argument-direction twin: a body-constructed `(Set.of (list n 2 9))` rides the op
           argument INTO the arm, which measures it (len 3) and membership-probes it (contains 5 →
           100) → 103. With this pair the collection crossing matrix — Map, List, Bytes, Set — has
           witnesses in both marshal directions.")
  (input
    (do
      (effect St (op tally (-> (Set Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((tally (xs) s (resume (+ (Set.len xs) (if (Set.contains xs 5) 100 0)) s)))
          (St.tally #set(n 2 9))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 103 Int64)))

(case
  "a LIST OF SETS op result — the body indexes, measures, and probes the nested elements"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects known-leak)
  (doc
    "NESTED collection crossings: every flat collection has both-direction witnesses; a
           collection INSIDE a collection riding the boundary (two heap layers, RRB list over CHAMP
           sets) had none. The arm resumes `(list (Set.of (list 1 2)) (Set.of (list 3 4 n)))`; the
           body indexes both elements, measuring one (len 2) and membership-probing the other
           (contains 5 → 100) → 102. Both layers must survive the resume marshal intact.")
  (input
    (do
      (effect St (op groups (-> Unit (List (Set Int64)))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((groups (u) s (resume #list(#set(1 2) #set(3 4 n)) s)))
          (let
            ((r (St.groups)))
            (+
              (match (List.at r 0) ((Some a) (Set.len a)) ((None _u) -1))
              (match (List.at r 1) ((Some b) (if (Set.contains b 5) 100 0)) ((None _u) -1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 102 Int64)))

(case
  "a LIST OF SETS as op ARGUMENT — the arm indexes into the nested payload it is handed"
  ; interim known-leak: #6022/#6049 closure / fold-list-reclaim / effects (v-mem adjudicated 2026-08-30); real fix -> 0
  (live-objects known-leak)
  (doc
    "The argument-direction twin of the nested-result pin: a body-built list of sets rides the
           op argument INTO the arm, which indexes both elements — 10·2 + 100 (contains 5) + 1 →
           121. The arm-side unbox of a two-layer payload.")
  (input
    (do
      (effect St (op weigh (-> (List (Set Int64)) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((weigh
              (xs)
              s
              (resume
                (+
                  (match
                    (List.at xs 0)
                    ((Some a) (+ (* 10 (Set.len a)) (if (Set.contains a 5) 100 0)))
                    ((None _u) -1))
                  (match (List.at xs 1) ((Some b) (Set.len b)) ((None _u) -1)))
                s)))
          (St.weigh #list(#set(n 2) #set(7)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 121 Int64)))

(case
  "a MAP OF LISTS op result — the body looks up a key and folds the inner list"
  (doc
    "The keyed face of nested crossings: a `(Map String (List Int64))` op result — the body
           looks up both keys and reads through the inner lists (len 3 + element 5 + element 40 →
           48). CHAMP-over-RRB, the inverse layering of the list-of-sets pins.")
  (input
    (do
      (effect St (op index (-> Unit (Map String (List Int64)))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((index
              (u)
              s
              (resume #map((= "a" #list(1 2 n)) (= "b" #list(40))) s)))
          (let
            ((m (St.index)))
            (+
              (match
                (Map.lookup m "a")
                ((Some xs) (+ (List.len xs) (match (List.at xs 2) ((Some v) v) ((None _u) -1))))
                ((None _u) -100))
              (match
                (Map.lookup m "b")
                ((Some ys) (match (List.at ys 0) ((Some w) w) ((None _u) -1)))
                ((None _u) -100))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 48 Int64))
  (live-objects known-leak))

(case
  "a record with a LIST field crosses resume — the body projects and folds the collection field"
  (doc
    "Record crossings carry all-scalar pins both ways plus a rope-String field on the argument
           side; a COLLECTION-typed field (CHAMP/RRB nested inside the record box) was unpinned in
           either direction. The arm resumes `(record (total 50) (items (list 5 6 7)))`; the body
           projects the scalar and folds the list field — 50 + 3 + 7 → 60.")
  (input
    (do
      (effect St (op page (-> Int64 (Record (: total Int64) (: items (List Int64))))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((page (k) s (resume #record((= total (* k 10)) (= items #list(k (+ k 1) (+ k 2)))) s)))
          (let
            ((r (St.page n)))
            (+
              r.total
              (+ (List.len r.items) (match (List.at r.items 2) ((Some v) v) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 60 Int64)))

(case
  "a record with a SET field as op ARGUMENT — the arm probes the collection beside the scalar"
  (doc
    "The argument-direction twin: the body hands `(record (want n) (seen (Set.of …)))` to the
           op and the ARM uses one field to query the other — contains(seen, want) → 100, plus len 3
           → 103. The collection field must arrive beside the scalar with both intact.")
  (input
    (do
      (effect St (op audit (-> (Record (: want Int64) (: seen (Set Int64))) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((audit
              (r)
              s
              (resume (+ (* 100 (if (Set.contains r.seen r.want) 1 0)) (Set.len r.seen)) s)))
          (St.audit #record((= want n) (= seen #set(2 n 9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 103 Int64)))

(case
  "a 40-element list op RESULT crosses resume — a multi-leaf RRB payload survives the marshal"
  (doc
    "The SIZE axis of collection crossings: the existing crossing pins carry small literal
           collections (single-leaf structures); a 40-element recursively-built list exercises the
           multi-leaf RRB spine through the resume marshal — len 40 and a deep index (element 36 at
           index 35) → 4036. Structure sharing across the boundary must survive past the
           single-node fast path.")
  (input
    (do
      (effect St (op range (-> Int64 (List Int64))))
      (def
        (build (: i Int64) (: k Int64) (: acc (List Int64)))
        (if (> i k) acc (build (+ i 1) k (List.push acc i))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((range (k) s (resume (build 1 k #list()) s)))
          (let
            ((xs (St.range (* n 8))))
            (+ (* 100 (List.len xs)) (match (List.at xs 35) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4036 Int64)))

(case
  "a 40-element list as op ARGUMENT — the arm folds a multi-leaf RRB payload"
  (doc
    "The argument-direction twin of the multi-leaf crossing: a 40-element body-built list
           rides INTO the arm, which runs a full indexed fold over it — sum 1..40 → 820. The
           arm-side traversal of a spine that crossed the perform.")
  (input
    (do
      (effect St (op total (-> (List Int64) Int64)))
      (def
        (build (: i Int64) (: k Int64) (: acc (List Int64)))
        (if (> i k) acc (build (+ i 1) k (List.push acc i))))
      (def
        (sum-l (: xs (List Int64)) (: i Int64) (: acc Int64))
        (match (List.at xs i) ((Some v) (sum-l xs (+ i 1) (+ acc v))) ((None _u) acc)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((total (xs) s (resume (sum-l xs 0 0) s)))
          (St.total (build 1 (* n 8) #list()))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 820 Int64))
  (live-objects known-leak))

(case
  "a 40-element SET op result — a multi-node CHAMP payload crosses resume"
  (doc
    "The CHAMP sibling of the multi-leaf RRB pins: a 40-element recursively-built set (spaced
           keys ×3 force node splits) crosses resume, then len + a positive and a negative
           membership probe — 4000 + 10 (60 ∈) + 0 (61 ∉) → 4010. The multi-node trie must arrive
           intact, not just its root.")
  (input
    (do
      (effect St (op universe (-> Int64 (Set Int64))))
      (def
        (fill (: i Int64) (: k Int64) (: acc (Set Int64)))
        (if (> i k) acc (fill (+ i 1) k (Set.insert acc (* i 3)))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((universe (k) s (resume (fill 1 k #set()) s)))
          (let
            ((xs (St.universe (* n 8))))
            (+
              (* 100 (Set.len xs))
              (+ (if (Set.contains xs 60) 10 0) (if (Set.contains xs 61) 1 0))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4010 Int64)))

(case
  "a LIST OF STRINGS op result — the body indexes and measures rope elements after the marshal"
  (doc
    "The ELEMENT-type axis of collection crossings (the crossing pins carry scalar elements):
           a `(List String)` op result mixing a rope-built element, a branch-selected one, and a
           literal — the body indexes elements 0 and 1 and byte-measures them after the marshal:
           100·(List.len 3) + 10·(byte-len \"alpha\" 5) + (byte-len \"beta\" 4) → 354. Heap-boxed
           elements inside a crossing list payload.")
  (input
    (do
      (effect St (op names (-> Int64 (List String))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((names
              (k)
              s
              (resume #list((String.concat "al" "pha") (if (> k 0) "beta" "x") "gamma") s)))
          (let
            ((xs (St.names n)))
            (+
              (* 100 (List.len xs))
              (+
                (* 10 (match (List.at xs 0) ((Some a) (String.byte-len a)) ((None _u) -1)))
                (match (List.at xs 1) ((Some b) (String.byte-len b)) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 354 Int64)))

(case
  "a LIST OF BIGINTS as op ARGUMENT — the arm folds heap-numeric elements it is handed"
  (doc
    "The argument-direction heap-element face: a body-built `(List BigInt)` rides INTO the arm,
           which runs an indexed fold accumulating a BigInt — 5 + 100 + 3000 → 3105, narrowed once
           through checked Int64.of. Heap-numeric boxes must survive inside the crossing payload.")
  (input
    (do
      (effect St (op total (-> (List BigInt) Int64)))
      (def
        (sum-b (: xs (List BigInt)) (: i Int64) (: acc BigInt))
        (match (List.at xs i) ((Some v) (sum-b xs (+ i 1) (+ acc v))) ((None _u) acc)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((total (xs) s (resume (Int64.of (sum-b xs 0 (BigInt.of 0))) s)))
          (St.total #list((BigInt.of n) (BigInt.of 100) (BigInt.of 3000)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3105 Int64))
  (live-objects known-leak))

(case
  "a list of RATIONALS op result — exact fractions cross resume and fold to a canonical sum"
  (doc
    "The exact-arithmetic element face: `(list 1/2 1/3 1/30)` crosses resume and the body folds
           it — the sum must arrive gcd-canonical (13/15, not an unreduced spelling) for the num/den
           digit encode to read 10·13 + 15 → 145. Rational normalization must survive both the
           marshal and the fold.")
  (input
    (do
      (effect St (op parts (-> Int64 (List Rational))))
      (def
        (sum-r (: xs (List Rational)) (: i Int64) (: acc Rational))
        (match (List.at xs i) ((Some v) (sum-r xs (+ i 1) (+ acc v))) ((None _u) acc)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((parts
              (k)
              s
              (resume #list((Rational.of 1 2) (Rational.of 1 3) (Rational.of 1 (* k 6))) s)))
          (let
            ((r (sum-r (St.parts n) 0 (Rational.of 0 1))))
            (+ (* 10 (Int64.of (Rational.numerator r))) (Int64.of (Rational.denominator r))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 145 Int64))
  (live-objects known-leak))

(case
  "a list-to-list TRANSFORMER op — heap payloads cross BOTH slots of one dispatch"
  (doc
    "Every crossing pin carries heap in ONE slot per dispatch (scalar the other way); a
           transformer signature `(-> (List Int64) (List Int64))` moves heap BOTH directions through
           the same perform — the arm extends the very list it received (push len·10, push n) and
           resumes it; the body reads len 4 and both appended elements → 6005.")
  (input
    (do
      (effect St (op grow (-> (List Int64) (List Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grow (xs) s (resume (List.push (List.push xs (* (List.len xs) 10)) n) s)))
          (let
            ((out (St.grow #list(7 8))))
            (+
              (* 1000 (List.len out))
              (+
                (* 100 (match (List.at out 2) ((Some a) a) ((None _u) -1)))
                (match (List.at out 3) ((Some b) b) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6005 Int64)))

(case
  "a map-to-map transformer op CHAINED — the second dispatch receives the first's result"
  (doc
    "The re-crossing composition: a `(Map String Int64) → (Map String Int64)` transformer
           called on its OWN result — a heap value that already crossed the boundary once crosses
           again as the next dispatch's argument. State-keyed inserts (first at s=0, second at s=1)
           make the two dispatches distinguishable: {seed, first:5, second:6} → 356.")
  (input
    (do
      (effect St (op stamp (-> (Map String Int64) (Map String Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((stamp (m) s (resume (Map.insert m (if (= s 0) "first" "second") (+ s n)) (+ s 1))))
          (let
            ((m2 (St.stamp (St.stamp #map((= "seed" 1))))))
            (+
              (* 100 (Map.len m2))
              (+
                (* 10 (match (Map.lookup m2 "first") ((Some a) a) ((None _u) -1)))
                (match (Map.lookup m2 "second") ((Some b) b) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 356 Int64)))

(case
  "a TUPLE-keyed Map op result — the body looks up by a reconstructed compound key"
  (doc
    "Compound STRUCTURAL keys across the boundary (tuple-keyed collections exist only in pure
           pins): the arm resumes a `(Map (Tuple Int64 Int64) Int64)`; the body reconstructs compound
           keys to look up — `(tuple 1 2)` hits (50), the order-flipped `(tuple 4 3)` misses (-1),
           len 2 → 249. Structural key equality must survive the marshal.")
  (input
    (do
      (effect St (op grid (-> Int64 (Map (Tuple Int64 Int64) Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grid
              (k)
              s
              (resume (Map.insert (Map.insert Map.empty #tuple(1 2) (* k 10)) #tuple(3 4) 7) s)))
          (let
            ((m (St.grid n)))
            (+
              (* 100 (Map.len m))
              (+
                (match (Map.lookup m #tuple(1 2)) ((Some a) a) ((None _u) -1))
                (match (Map.lookup m #tuple(4 3)) ((Some b) b) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 249 Int64)))

(case
  "a SET of tuples as op ARGUMENT — the arm probes compound membership including order sensitivity"
  (doc
    "The argument-direction compound-key face: a body-built `(Set (Tuple Int64 Int64))` rides
           into the arm, which probes `(tuple 1 n)` (hit, 100) and the order-flipped `(tuple n 1)`
           (miss, 0) plus len 2 → 102. Tuple component ORDER must survive as part of the key's
           identity through the crossing.")
  (input
    (do
      (effect St (op check (-> (Set (Tuple Int64 Int64)) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((check
              (xs)
              s
              (resume
                (+
                  (* 100 (if (Set.contains xs #tuple(1 n)) 1 0))
                  (+ (* 10 (if (Set.contains xs #tuple(n 1)) 1 0)) (Set.len xs)))
                s)))
          (St.check #set(#tuple(1 n) #tuple(2 8)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 102 Int64)))

(case
  "a two-op composition where the second op's String argument is BUILT from the first's result"
  (doc
    "An effect-derived key crossing back in: op-1 returns a String (branch-selected \\\"hot\\\"),
           the body concat-extends it, and op-2 receives the assembled \\\"hot-path\\\" as its
           argument — byte-len 8 + 10·(state 1) → 18. A dispatch's result feeding the next
           dispatch's compound-built argument.")
  (input
    (do
      (effect St (op tag (-> Int64 String)) (op fetch (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((tag (k) s (resume (if (> k 0) "hot" "cold") (+ s 1)))
            (fetch (name) s (resume (+ (String.byte-len name) (* s 10)) (+ s 1))))
          (St.fetch (String.concat (St.tag n) "-path"))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 18 Int64)))

(case
  "a SYMBOL as op ARGUMENT — the arm compares interned identity against its own intern"
  (doc
    "Symbol's ARGUMENT direction (the interner/gensym pins cover only `-> String Symbol`
           results): a rope-built `(Symbol.of (String.concat …))` and a flat intern each cross as op
           arguments; the arm interns its own comparators — content equality must hold across the
           boundary (100 for alpha, 10 for beta → 110).")
  (input
    (do
      (effect St (op classify (-> Symbol Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((classify
              (sym)
              s
              (resume
                (+
                  (* 100 (if (= sym (Symbol.of "alpha")) 1 0))
                  (* 10 (if (= sym (Symbol.of "beta")) 1 0)))
                s)))
          (+ (St.classify (Symbol.of (String.concat "al" "pha"))) (St.classify (Symbol.of "beta")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "a SYMBOL handler STATE threads dispatches — each resume reads the prior symbol's identity"
  (doc
    "Symbol's STATE slot (completing its three effect positions, like records and sums): the
           state starts as the `start` symbol; each dispatch compares the PRIOR symbol's identity
           (10 for start, 20 otherwise) and swaps in the next — 100·10 + 20 → 1020.")
  (input
    (do
      (effect St (op swap (-> Symbol Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Symbol.of "start")
          ((swap (next) prev (resume (if (= prev (Symbol.of "start")) 10 20) next)))
          (+ (* 100 (St.swap (Symbol.of "mid"))) (St.swap (Symbol.of "end")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1020 Int64)))

(case
  "a fallible helper with a `?` called from INSIDE a handler ARM (success path)"
  (doc
    "The 23-try corpus pins `?` composition from the handle-BODY side; here the fallible
           helper runs inside the ARM — the `?` desugar's abortive Core::Block boundary nests while
           the dispatch machinery is live mid-arm. Two dispatches: bump(5)=105 then bump(6)=106 →
           10·105 + 106 = 1156. The two abortive machineries must not confuse their exit paths in
           arm position.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def (bump (: v Int64)) (let ((x (try (Some v)))) (Some (+ x 100))))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume (match (bump s) ((Some v) v) ((None _u) -1)) (+ s 1))))
          (+ (* 10 (St.next)) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1156 Int64)))

(case
  "a CONSTANT-failure `?` inside the arm's helper — the cut stays in the helper, dispatch unharmed"
  (doc
    "The failure face of the arm-side `?`: the helper's `(try (None unit))` short-circuits the
           HELPER (returning None), not the arm or the dispatch — both dispatches observe the -1
           fallback and the state advance is unharmed → 10·(-1) + (-1) = -11. (A runtime-disc `?`
           here hits the BRICK-3b constant-operand boundary, pinned in 23-try.)")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def (probe (: v Int64)) (let ((x (try (None unit)))) (Some (+ x v))))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume (match (probe s) ((Some v) v) ((None _u) -1)) (+ s 7))))
          (+ (* 10 (St.next)) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: -11 Int64)))

(case
  "a FLOAT64 as op ARGUMENT — the arm accumulates fractional values into Float64 state"
  (doc
    "Float64's ARGUMENT direction (result + state are pinned): fractional literals cross as op
           arguments and accumulate into Float64 state across two dispatches — a = 1.25+0.5 = 1.75,
           b = 0.25+1.75 = 2.0 → 3.75, read back as a Float64 (Int64.of over a runtime Float64
           rejects by design, per the numeric model).")
  (input
    (do
      (effect St (op weigh (-> Float64 Float64)))
      (def
        (main (: n Int64))
        (handle
          St
          0.5
          ((weigh (x) s (resume (+ x s) (+ s x))))
          (let ((a (St.weigh 1.25))) (let ((b (St.weigh 0.25))) (+ a b)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3.75 Float64)))

(case
  "a TUPLE mixing Float64 and Int64 crosses as op ARGUMENT — the arm scales by the int"
  (doc
    "The mixed-width marshal box: an f64 and an i64 in ONE tuple op argument, destructured by
           the arm and combined via Float64.of-int — 2.5 · 10 → 25.0. The two lanes must not
           corrupt each other through the crossing.")
  (input
    (do
      (effect St (op scale (-> (Tuple Float64 Int64) Float64)))
      (def
        (main (: n Int64))
        (handle
          St
          0.0
          ((scale (p) s (match p (#tuple(f k) (resume (* f (Float64.of-int k)) s)))))
          (St.scale #tuple(2.5 (* n 2)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 25.0 Float64)))

(case
  "the ARM decodes a Bytes op argument with a bin pattern and resumes a parsed field"
  (doc
    "The arm as the DECODE site (the bin×effects pins put the codec in the body): a body-built
           frame crosses the op argument and the ARM runs the `(bin (u8 tag) (u16 val))` match —
           1000·7 + 500 → 7500. Binary parsing composes with dispatch in arm position.")
  (input
    (do
      (effect Codec (op parse (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          Codec
          0
          ((parse
              (frame)
              s
              (match
                frame
                ((bin (u8 tag) (u16 val)) (resume (+ (* 1000 tag) val) s))
                (_other (resume -1 s)))))
          (Codec.parse (bin (u8 (UInt8.wrap 7)) (u16 (UInt16.wrap (* n 100)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7500 Int64)))

(case
  "the ARM ENCODES its scalar op argument into framed Bytes and resumes them — body decodes"
  (doc
    "The inverse arm-codec direction: the arm bin-ENCODES its scalar argument into a framed
           payload, resumes it, and the BODY decodes — 1000·9 + 150 → 9150. Round-trip with the
           encode inside the arm and the decode outside.")
  (input
    (do
      (effect Codec (op frame (-> Int64 Bytes)))
      (def
        (main (: n Int64))
        (handle
          Codec
          0
          ((frame (v) s (resume (bin (u8 (UInt8.wrap 9)) (u16 (UInt16.wrap (* v 3)))) s)))
          (match (Codec.frame (* n 10)) ((bin (u8 tag) (u16 val)) (+ (* 1000 tag) val)) (_other -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 9150 Int64)))

(case
  "a Bytes-to-Bytes transformer op — the arm frames the payload it received and the body re-reads"
  (doc
    "The byte-rope transformer face (hb pins cover List/Map transformers): the arm
           length-prefixes the frame it received via `Bytes.concat` of a fresh bin over the crossed
           payload — a NON-FLAT byte-rope result — and the body re-reads prefix + first payload byte:
           10000·3 + 100·2 + 40 → 30240.")
  (input
    (do
      (effect Codec (op wrap (-> Bytes Bytes)))
      (def
        (main (: n Int64))
        (handle
          Codec
          0
          ((wrap (b) s (resume (Bytes.concat (bin (u8 (UInt8.wrap (Bytes.len b)))) b) s)))
          (let
            ((out (Codec.wrap (bin (u8 (UInt8.wrap (* n 8))) (u8 (UInt8.wrap 3))))))
            (+
              (* 10000 (Bytes.len out))
              (+
                (* 100 (match (Bytes.at out 0) ((Some h) h) ((None _u) -1)))
                (match (Bytes.at out 1) ((Some p) p) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 30240 Int64)))

(case
  "a String-to-String transformer op — a rope argument crosses in, a wrapped rope crosses back"
  (doc
    "The text transformer face: a concat-built rope ARGUMENT crosses in, the arm wraps it in
           brackets via nested concats (another rope), and the result crosses back — byte-len
           \\\"[abcde]\\\" → 7. Rope structure survives both marshal directions of one dispatch.")
  (input
    (do
      (effect Fmt (op brack (-> String String)))
      (def
        (main (: n Int64))
        (handle
          Fmt
          0
          ((brack (t) s (resume (String.concat "[" (String.concat t "]")) s)))
          (String.byte-len (Fmt.brack (String.concat "ab" (if (> n 0) "cde" "z"))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))

(case
  "an abortive arm READS the heap LIST op argument it was handed — the payload survives the abort"
  (doc
    "The abort×heap pins cover arm-BUILT lists and heap STATE reads; an abortive arm CONSUMING
           its heap op-argument payload was unpinned. The crossed list must stay live on the abort
           path — 100·3 + 42 → 342, plus the outer 1000; the discarded continuation's 999 never
           adds → 1342.")
  (input
    (do
      (effect Bail (op stop (-> (List Int64) Int64)))
      (def
        (main (: n Int64))
        (+
          1000
          (handle
            Bail
            0
            ((stop
                (xs)
                s
                (+ (* 100 (List.len xs)) (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))
            (+ 999 (Bail.stop #list(n 42 7))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1342 Int64)))

(case
  "an abortive arm returns a MAP built FROM its heap op argument as the handle's value"
  (doc
    "Heap-in via the op argument AND heap-out via the abort branch, one arm: the abortive arm
           folds its list argument into a fresh Map that becomes the handle's value — {sum: 35},
           10·1 + 35 → 45. Both heap directions on the abort path.")
  (input
    (do
      (effect Bail (op stop (-> (List Int64) (Map String Int64))))
      (def
        (main (: n Int64))
        (let
          ((m
              (handle
                Bail
                0
                ((stop
                    (xs)
                    s
                    (Map.insert
                      Map.empty
                      "sum"
                      (+
                        (match (List.at xs 0) ((Some a) a) ((None _u) 0))
                        (match (List.at xs 1) ((Some b) b) ((None _u) 0))))))
                (do (Bail.stop #list(n 30)) Map.empty))))
          (+ (* 10 (Map.len m)) (match (Map.lookup m "sum") ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 45 Int64)))

(case
  "200 recursive dispatches each crossing a heap LIST argument — the marshal at depth"
  (doc
    "The depth axis of heap-argument crossings (existing depth pins carry scalars): 200
           iterations each build a fresh two-element list, cross it, and the arm folds it — the
           per-dispatch marshal alloc/free churn must stay exact: Σ(i + 1) for i in 1..200 →
           20300.")
  (input
    (do
      (effect St (op scan (-> (List Int64) Int64)))
      (def
        (loop (: i Int64) (: acc Int64))
        (if (> i 200) acc (loop (+ i 1) (+ acc (St.scan #list(i 1))))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((scan
              (xs)
              s
              (resume
                (+
                  (match (List.at xs 0) ((Some a) a) ((None _u) 0))
                  (match (List.at xs 1) ((Some b) b) ((None _u) 0)))
                s)))
          (loop 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20300 Int64)))

(case
  "a handler state GROWS a list across 100 dispatches — the accumulated spine reads back intact"
  (doc
    "The growing-spine RC discipline across suspensions: each dispatch pushes onto the list
           state and resumes the length BEFORE its push, so the checksum verifies every intermediate
           spine — 100·Σ(0..99) → 495000, not just the final length.")
  (input
    (do
      (effect Log (op note (-> Int64 Int64)))
      (def (loop (: i Int64) (: acc Int64)) (if (> i 100) acc (loop (+ i 1) (+ acc (Log.note i)))))
      (def
        (main (: n Int64))
        (handle
          Log
          #list()
          ((note (v) s (resume (List.len s) (List.push s v))))
          (+ (* 100 (loop 1 0)) 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 495000 Int64))
  (live-objects known-leak))

(case
  "a Bytes.slice VIEW crosses as op ARGUMENT — the arm reads through the window it was handed"
  (doc
    "A body-built slice VIEW (not a copy) crossing INTO a dispatch (the existing view pins put
           the slice in the resume value or slice the arm's own param): the arm reads len + both
           bytes through the window — 100·2 + 20 + 30 → 250. The view's backing buffer must stay
           live through the marshal.")
  (input
    (do
      (effect St (op sum2 (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sum2
              (w)
              s
              (resume
                (+
                  (* 100 (Bytes.len w))
                  (+
                    (match (Bytes.at w 0) ((Some a) a) ((None _u) -1))
                    (match (Bytes.at w 1) ((Some b) b) ((None _u) -1))))
                s)))
          (match
            (Bytes.slice (Bytes.of #list(9 20 30 8)) 1 2)
            ((Some w) (St.sum2 w))
            ((None _u) -999))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 250 Int64)))

(case
  "a String.slice VIEW built in the ARM crosses back through resume — the body measures it"
  (doc
    "The arm-built STRING view crossing OUT: the arm slices the rope argument it received
           (start 1, end 4 → \\\"bcd\\\") and resumes the window — byte-len 3. An arm-created view
           over a crossed payload must survive the return marshal.")
  (input
    (do
      (effect St (op mid (-> String String)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((mid (t) s (resume (match (String.slice t 1 4) ((Some w) w) ((None _u) "?")) s)))
          (String.byte-len (St.mid (String.concat "ab" "cdef")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64)))

(case
  "an IN-PROGRAM arm resumes a Qty built in the arm — the erased-scalar crossing without a host"
  (doc
    "The pure in-program Qty handler (the existing Qty effect pins are host-delegated): the arm
           builds `(Qty.of (* k 2) meter)` and resumes it; two dispatches sum under the unit type and
           `Qty.value` reads 30. The compile-time-erased unit must type the arm/body agreement with
           no host boundary involved.")
  (input
    (do
      (effect Env (op width (-> Int64 (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: n Int64))
        (handle
          Env
          0
          ((width (k) s (resume (Qty.of (* k 2) (Unit.base #"meter")) s)))
          (Qty.value (+ (Env.width n) (Env.width 10)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 30 Int64)))

(case
  "a Qty STATE threads via a def-bound arm computation — the workaround shape runs end to end"
  (doc
    "Qty as handler STATE with the arm computing the next state through an arm-local `def` —
           each dispatch resumes the PRIOR quantity and doubles the state: 5m + 10m → 15. (The
           sibling pin below covers the inline-slot spelling.)")
  (input
    (do
      (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: n Int64))
        (handle
          Acc
          (Qty.of n (Unit.base #"meter"))
          ((step (u) s (do (def t (+ s s)) (resume s t))))
          (Qty.value (+ (Acc.step) (Acc.step)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a Qty state's next-state slot computes (+ s s) INLINE — the formerly-rejected shape runs"
  (doc
    "This exact spelling — Qty-state arithmetic INSIDE the next-state slot — used to falsely
           reject (the state binder typed at the erased Int64 in slot position; an 18-units
           provenance note documents the old behavior and its def-workaround). Fixed on trunk; this
           pins the flip: seed 5m, `(resume s (+ s s))` threads 5+10 → 15 with values verified.")
  (input
    (do
      (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: n Int64))
        (handle
          Acc
          (Qty.of n (Unit.base #"meter"))
          ((step (u) s (resume s (+ s s))))
          (Qty.value (+ (Acc.step) (Acc.step)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "a guard destructures a perform-result TUPLE and its condition reads both binders"
  (doc
    "The compound-pattern face of the guarded perform-scrutinee family (the ag5 pins use
           scalar guard binders): `(guard (tuple a b) (> (+ a b) 10))` over a perform-result tuple —
           the guard-desugar's arm copy composes with the destructure; hit path 100·5 + 10 → 510.")
  (input
    (do
      (effect St (op pair (-> Unit (Tuple Int64 Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((pair (u) s (resume #tuple(s (* s 2)) (+ s 1))))
          (match
            (St.pair)
            ((guard #tuple(a b) (> (+ a b) 10)) (+ (* 100 a) b))
            (#tuple(a b) (+ a b)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 510 Int64))
  (live-objects 0))

(case
  "the guard-MISS path re-performs in the fallback arm — dispatch continues past a failed guard"
  (doc
    "The miss path of the compound guard: `(> (+ a b) 100)` fails at 15, the fallback arm
           RE-PERFORMS, and the second dispatch reads the advanced state — 10·15 + 18 → 168. A
           failed compound guard must leave the dispatch machinery able to serve the fallback's
           perform.")
  (input
    (do
      (effect St (op pair (-> Unit (Tuple Int64 Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((pair (u) s (resume #tuple(s (* s 2)) (+ s 1))))
          (match
            (St.pair)
            ((guard #tuple(a b) (> (+ a b) 100)) (+ (* 100 a) b))
            (#tuple(a b) (match (St.pair) (#tuple(c d) (+ (* 10 (+ a b)) (+ c d))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 168 Int64))
  (live-objects 0))

(case
  "an arm re-performs its OWN effect to a SAME-EFFECT outer handler — the true self-shadow forward"
  (doc
    "The existing forwarding pin uses two DISTINCT effects; here the inner handler of `Ctr`
           re-performs `Ctr` against a same-effect OUTER handler with a DIFFERENT arm shape — inner
           multiplies-and-forwards, outer adds-with-state: bump(5) → outer bump(50) → 50+100 = 150.
           The forward must reach the outer arm's semantics, not re-enter the inner's.")
  (input
    (do
      (effect Ctr (op bump (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Ctr
          100
          ((bump (v) t (resume (+ v t) (+ t 1))))
          (handle Ctr 0 ((bump (v) s (resume (Ctr.bump (* v 10)) s))) (Ctr.bump n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 150 Int64)))

(case
  "both same-effect handlers STATEFUL — the outer's advance survives the inner's forwards"
  (doc
    "The stateful composition of the self-shadow forward: two inner-region dispatches each
           forward to the stateful outer (t advances 100→101→102), then a POST-region perform reads
           the accumulated t — (150 + 111) + 104 → 365. The outer state must thread across forwards
           originating in the inner arm.")
  (input
    (do
      (effect Ctr (op bump (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Ctr
          100
          ((bump (v) t (resume (+ v t) (+ t 1))))
          (+
            (handle
              Ctr
              0
              ((bump (v) s (resume (Ctr.bump (* v 10)) (+ s 1))))
              (+ (Ctr.bump n) (Ctr.bump 1)))
            (Ctr.bump 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 365 Int64)))

(case
  "a CLOSURE handler state captures the enclosing function's parameter and applies per dispatch"
  (doc
    "The existing closure-state pins seed with parameter-FREE closures; here the seed closure
           captures the enclosing function's `n` — `(fn (x) (* x n))` applied in the arm reads the
           capture (10·5 → 50). Single-shot dispatch with a param-capturing closure state (the
           multi-shot sibling is a known open capture-locus).")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle St (fn ((: x Int64)) (* x n)) ((next (u) f (resume (f 10) f))) (St.next)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64)))

(case
  "the closure state is REPLACED per dispatch by one capturing the arm's OWN binder"
  (doc
    "State replacement with an arm-frame capture: each dispatch builds a FRESH closure over the
           arm's own let-binder `r` and installs it as the next state — d1: f = x+5, r = 105, next
           f = x+105; d2: r = 205 → 1000·105 + 205 = 105205. The replacement closure's environment
           must be the arm frame's, rebuilt per dispatch.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (fn ((: x Int64)) (+ x n))
          ((next (u) f (let ((r (f 100))) (resume r (fn ((: x Int64)) (+ x r))))))
          (+ (* 1000 (St.next)) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105205 Int64)))

(case
  "the natural invariant construction over a VIOLATING perform result traps through the handler"
  (doc
    "The body-side invariant × effects composition (the arm-side pin lives in
           26-program-conditions): `(Percent.Pct (St.next))` where the RESUMED VALUE itself decides —
           in-range 42 constructs and unwraps; an out-of-range 200 violates `[0,100]` and traps at
           the establish-divert THROUGH the handler's resume path.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
      (def (unwrap (: p Percent)) (match p ((Percent.Pct n) n)))
      (def
        (main (: n Int64))
        (handle St n ((next (u) s (resume s (+ s 1)))) (unwrap (Percent.Pct (St.next)))))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64))
  (call main (: 200 Int64))
  (trap "unreachable"))

(case
  "the arm DECODES a Bytes op argument to a String — multibyte UTF-8 survives the crossing"
  (doc
    "String.from-bytes validation in ARM position over a crossed payload (the validation pins
           are body-side): \\\"héllo\\\" (6 bytes, one 2-byte scalar) crosses as the op argument and
           the arm's decode validates it → byte-len 6.")
  (input
    (do
      (effect Codec (op read (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          Codec
          0
          ((read
              (b)
              s
              (resume (match (String.from-bytes b) ((Some t) (String.byte-len t)) ((None _u) -1)) s)))
          (Codec.read (String.to-bytes "héllo"))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "INVALID UTF-8 crosses as a Bytes op argument — the arm's decode declines with None"
  (doc
    "The invalid-bytes face: `0xFF 0xFE` crosses the boundary and the arm's
           `String.from-bytes` must actually validate the crossed payload (not trust it) —
           None → -1.")
  (input
    (do
      (effect Codec (op read (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          Codec
          0
          ((read
              (b)
              s
              (resume (match (String.from-bytes b) ((Some t) (String.byte-len t)) ((None _u) -1)) s)))
          (Codec.read (bin (u8 (UInt8.wrap 255)) (u8 (UInt8.wrap 254))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: -1 Int64)))

(case
  "TWO sequential handles of the same effect — the second starts fresh, no state bleed"
  (doc
    "Handler LIFECYCLE isolation (the existing pins nest but never SEQUENCE): one helper
           instantiates the same handler twice in sequence — run(5) = 5+6 = 11, then run(10) =
           10+11 = 21, each from its OWN seed → 100·11 + 21 = 1121. No state bleeds between
           instantiations.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (run (: seed Int64))
        (handle St seed ((next (u) s (resume s (+ s 1)))) (+ (St.next) (St.next))))
      (def (main (: n Int64)) (+ (* 100 (run n)) (run 10)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1121 Int64)))

(case
  "an ABORT in the first handle leaves the SECOND handle's dispatch untouched"
  (doc
    "Post-abort isolation: the first handle aborts (5·2 = 10, its 999 continuation dropped);
           a SECOND, separate handle then dispatches normally (7+8 = 15) → 10·10 + 15 = 115. The
           abort's unwind must not corrupt a sibling handler's dispatch or state.")
  (input
    (do
      (effect Bail (op stop (-> Int64 Int64)))
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (+
          (* 10 (handle Bail 0 ((stop (v) s (* v 2))) (+ 999 (Bail.stop n))))
          (handle St 7 ((next (u) s (resume s (+ s 1)))) (+ (St.next) (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 115 Int64)))

(case
  "a bare BIGINT as op ARGUMENT — the arm does exact wide arithmetic on the crossed box"
  (doc
    "BigInt's ARGUMENT direction (results/state/list-elements are pinned): the arm multiplies
           the crossed box by 10^6 and integer-divides by 999999999 — exact wide arithmetic on a
           value that crossed the boundary → 1000, narrowed once through checked Int64.of.")
  (input
    (do
      (effect St (op grow (-> BigInt Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grow (b) s (resume (Int64.of (/ (* b (BigInt.of 1000000)) (BigInt.of 999999999))) s)))
          (St.grow (BigInt.of (* n 200000)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1000 Int64)))

(case
  "a bare RATIONAL as op ARGUMENT — the arm reads exact numerator/denominator off the crossed value"
  (doc
    "Rational's ARGUMENT direction: 1/3 crosses, the arm adds 1/6 and reads num/den off the
           gcd-canonical sum (1/2) → 10·1 + 2 = 12. Exact-fraction identity must survive the
           marshal into the arm.")
  (input
    (do
      (effect St (op mix (-> Rational Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((mix
              (r)
              s
              (let
                ((q (+ r (Rational.of 1 6))))
                (resume
                  (+ (* 10 (Int64.of (Rational.numerator q))) (Int64.of (Rational.denominator q)))
                  s))))
          (St.mix (Rational.of 1 (- n 2)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

; ============ Narrow-width effect-op literals (breaker FINDING nw, operator-confirmed soundness →
; fixed on trunk). The effect-op signature positions (argument AND result-via-resume) skipped the
; CDZ0302 literal fit-check every sibling position enforces — an out-of-range literal observably
; inhabited the narrow type, including across the HOST boundary in a declared-width slot. The fix
; range-checks both marshal directions (and descends compounds: tuple/record/list). These pin the
; served class: the in-range pass, the bare-arg + resume-result + record-field rejects, and the
; runtime-argument control (a TYPE mismatch, not a width fault). ============
(case
  "an in-range literal to a narrow effect-op parameter crosses and the arm observes it"
  (doc
    "The pass face of the narrow-op range check: `(Send.put 42)` against `(-> UInt8 Int64)`
           fits, crosses, and the arm reads 42 back via checked Int64.of.")
  (input
    (do
      (effect Send (op put (-> UInt8 Int64)))
      (def (main (: n Int64)) (handle Send 0 ((put (v) s (resume (Int64.of v) s))) (Send.put 42)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

(case
  "an OVERFLOWING literal to a narrow effect-op parameter is rejected"
  (doc
    "The argument-direction reject: `(Send.put 999)` against a UInt8 parameter (0..=255) is
           CDZ0302 — the same fit-check plain-fn params and annotated literals enforce. Before the
           fix this compiled and the arm observed 999.")
  (input
    (do
      (effect Send (op put (-> UInt8 Int64)))
      (def (main (: n Int64)) (handle Send 0 ((put (v) s (resume (Int64.of v) s))) (Send.put 999)))
      (export main)))
  (error CDZ0302))

(case
  "an arm resuming an OVERFLOWING literal into a narrow op RESULT is rejected"
  (doc
    "The result-direction reject: the op's declared result is UInt8 and the arm resumes 999 —
           CDZ0302 at the resume site. Before the fix the body observed 999 through the narrow
           result type.")
  (input
    (do
      (effect Give (op get (-> Unit UInt8)))
      (def (main (: n Int64)) (handle Give 0 ((get (u) s (resume 999 s))) (Int64.of (Give.get))))
      (export main)))
  (error CDZ0302))

(case
  "an overflowing literal in a RECORD op argument's narrow field is rejected"
  (doc
    "The compound-descent face: the width check must recurse into a Record op argument's
           fields — `(record (small 999) …)` against `(Record (: small UInt8) …)` is CDZ0302. (Tuple
           and List elements were covered by the same descent from the start; the Record row arm
           was a fold-in.)")
  (input
    (do
      (effect Send (op put (-> (Record (: small UInt8) (: big Int64)) Int64)))
      (def
        (main (: n Int64))
        (handle
          Send
          0
          ((put (r) s (resume (+ (Int64.of r.small) r.big) s)))
          (Send.put #record((= small 999) (= big 5)))))
      (export main)))
  (error CDZ0302))

(case
  "a RUNTIME Int64 argument to a narrow effect-op parameter is rejected as a type mismatch"
  (doc
    "The control distinguishing the width fault from ordinary typing: a RUNTIME Int64 arg to a
           UInt8 op parameter is CDZ0301 (type mismatch — no silent narrowing), NOT CDZ0302 (which
           is literal-fit). The two rejects must not blur.")
  (input
    (do
      (effect Send (op put (-> UInt8 Int64)))
      (def (main (: n Int64)) (handle Send 0 ((put (v) s (resume 7 s))) (Send.put n)))
      (export main)))
  (error CDZ0301))

(case
  "a FULL handle expression in the resume-value slot — the arm runs a nested handler per dispatch"
  (doc
    "Arms performing INTO enclosing handlers is well-pinned; here the arm INSTANTIATES its own
           complete handler: `(resume (handle In 100 … (In.small (* v 2))) s)` — the nested handle
           runs to completion inside the arm and its result becomes the resume value (10 + 100 →
           110).")
  (input
    (do
      (effect Out (op big (-> Int64 Int64)))
      (effect In (op small (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Out
          0
          ((big
              (v)
              s
              (resume (handle In 100 ((small (w) t (resume (+ w t) t))) (In.small (* v 2))) s)))
          (Out.big n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "the arm's nested handler is instantiated FRESH per dispatch — independent inner state"
  (doc
    "The per-dispatch lifecycle of an arm-instantiated handler: each outer dispatch seeds a NEW
           inner handler from its op argument (v=5 → inner 5+6=11; v=20 → inner 20+21=41) → 100·11 +
           41 = 1141. No inner state survives between the arm's instantiations.")
  (input
    (do
      (effect Out (op big (-> Int64 Int64)))
      (effect In (op small (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Out
          0
          ((big
              (v)
              s
              (resume (handle In v ((small (u) t (resume t (+ t 1)))) (+ (In.small) (In.small))) s)))
          (+ (* 100 (Out.big n)) (Out.big 20))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1141 Int64)))

; ============ Multi-shot × enclosing-param capture (breaker FINDING mv, fixed in two slices: the
; continuation's captures pinned before the per-resume splice, then the ARM BODY's captures pinned
; before beta-reduce for the resume-value face + after substitution for the seed face). The class
; was [multi-shot arm] × [any let/def binding in the handle body] × [an enclosing-param reference] →
; false CDZ0101; match-binder consumers and no-binding bodies were always immune. These pin the
; VALUE-verified faces: param in the resume value, param as the handle seed, and the always-immune
; match-binder control. ============
(case
  "a multi-shot arm's resume VALUE reads the enclosing param — the let-bound body folds correctly"
  (doc
    "FINDING repro (mv7, fixed): `(pick (u) s (+ (resume (+ n 1) s) (resume 2 s)))` with the
           body let-binding the perform result — the resume-value's `n` is spliced into the
           continuation's hole per resume site and used to orphan. Now folds with the right VALUES:
           k(v) = 11v, so 11·6 + 11·2 = 88.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
          (let ((x (Amb.pick))) (+ (* 10 x) x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 88 Int64)))

(case
  "a multi-shot handle SEEDED by the enclosing param folds with a let-bound body"
  (doc
    "The seed face (mv11, fixed): `(handle Amb n …)` where the param enters the arm via the
           state binder substitution — the second capture path of the mv class. Same fold values:
           seed 5, resume (s+1)=6 then 2 → 11·6 + 11·2 = 88.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          n
          ((pick (u) s (+ (resume (+ s 1) s) (resume 2 s))))
          (let ((x (Amb.pick))) (+ (* 10 x) x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 88 Int64)))

(case
  "a multi-shot arm with an enclosing-param resume value and a MATCH-binder consumer folds"
  (doc
    "The always-immune control of the mv class: a match BINDER consumes the perform result
           (binding without a let) — this shape never orphaned, and it must keep folding identically
           now that the let shapes are fixed: 88.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
          (match (Amb.pick) (v (+ (* 10 v) v)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 88 Int64)))

(case
  "an @ensures on a PERFORMING def called TWICE under one handler — both effectful results checked"
  (doc
    "The @ensures SURFACE face of the en1 class (the minimal let-if-inline shape is pinned
           above; 26-program-conditions marks the multi-call face as future): a postcondition on a
           performing def called twice under one handler — verify_enforce wraps each inline, both
           effectful results check `(>= ret 100)`: f(5)=105, f(2)=103 → 208.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (+ (f n) (f 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 208 Int64)))

(case
  "an OPTION as op ARGUMENT — the arm matches Some/None it was handed, per dispatch"
  (doc
    "Option's ARGUMENT direction (results + state are pinned): body-built `(Some n)` and
           `(None unit)` each ride into the arm, which matches — Some(5) → 50, None → -1 →
           100·50 - 1 = 4999. The std-sum tag must survive the crossing per dispatch.")
  (input
    (do
      (effect St (op weigh (-> (Option Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((weigh (o) s (resume (match o ((Some v) (* v 10)) ((None _u) -1)) s)))
          (+ (* 100 (St.weigh (Some n))) (St.weigh (None unit)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4999 Int64)))

(case
  "a RESULT as op ARGUMENT — the arm branches on Ok/Err payloads it was handed"
  (doc
    "Result's ARGUMENT direction: `(Result.Ok n)` and `(Result.Err 7)` cross into the arm,
           which branches — Ok(5) → 50, Err(7) → -7 → 100·50 - 7 = 4993. Completes the std-sum
           pair's three effect positions.")
  (input
    (do
      (effect St (op judge (-> (Result Int64 Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((judge (r) s (resume (match r ((Result.Ok v) (* v 10)) ((Result.Err e) (- 0 e))) s)))
          (+ (* 100 (St.judge (Result.Ok n))) (St.judge (Result.Err 7)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4993 Int64)))

(case
  "a scrutinee FAILING the pattern reaches the catch-all WITHOUT running the guard's perform"
  (doc
    "The pattern-MISS soundness face of the refutable performing guard (the sibling pins test
           guard-matches-then-false): a None scrutinee against `(guard (Some v) (> v (St.quota)))`
           must reach the catch-all with the guard's perform NEVER evaluated — witnessed by a
           post-match `St.quota` reading the UNADVANCED state: 100·99 + 5 = 9905. (The keep-the-match
           hoist guarantees this; an if-only rewrite would have run the guard on the miss.)
           UPDATE (guards-side-effect-free, CDZ0407): `(St.quota)` in the guard cond is NOW a COMPILE ERROR —
           the pattern-miss soundness this pinned is moot once a performing guard cannot exist.")
  (input
    (do
      (effect St (op quota (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((quota (u) s (resume s (+ s 1))))
          (+
            (* 100 (match (None unit) ((guard (Some v) (> v (St.quota))) v) (_other 99)))
            (St.quota))))
      (export main)))
  (error CDZ0407))

(case
  "a MULTI-argument op mixing a heap list and two scalars — the arm consumes all three"
  (doc
    "Multi-argument op signatures are pinned scalar-only; here a `(List Int64)` crosses beside
           two scalar INDICES into it — the arm indexes the list by both and measures it:
           100·7 + 10·9 + 3 → 793. Positional integrity across a mixed heap/scalar marshal.")
  (input
    (do
      (effect St (op pick (-> (List Int64) Int64 Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((pick
              (xs lo hi)
              s
              (resume
                (+
                  (* 100 (match (List.at xs lo) ((Some a) a) ((None _u) -1)))
                  (+ (* 10 (match (List.at xs hi) ((Some b) b) ((None _u) -1))) (List.len xs)))
                s)))
          (St.pick #list(7 n 9) 0 2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 793 Int64)))

(case
  "a multi-argument op with TWO heap arguments — a String key and a Map to search"
  (doc
    "Two heap values in ONE op signature (the lookup-service idiom): a rope-built String key
           and a Map cross together; the arm looks the key up in the map it was handed —
           10·5 + 2 → 52. Two independent heap handles must both survive the same marshal.")
  (input
    (do
      (effect St (op find (-> String (Map String Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((find
              (k m)
              s
              (resume (+ (* 10 (match (Map.lookup m k) ((Some v) v) ((None _u) -1))) (Map.len m)) s)))
          (St.find (String.concat "k" "1") #map((= "k1" n) (= "k2" 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 52 Int64)))

; ============ Empty-collection match-join grounding (breaker FINDING ms/ej, fixed in three parts:
; the front-end grounds an open-Var join arm to the determined-collection shell — all three
; collection kinds; the rust emit reconstructs the solved map type at Map.lookup for scalar values;
; and the rust emit annotates a collection-valued join's solved OUTER shape with holed interior
; (`Vec<_>`) rather than grounding — the nested face is WHY: the join under-approximates nested
; element types, and a ground would break exactly where a hole lets rustc solve). These pin the
; served class: the pure minimal, the Set sibling, the IF-join face, and the two-layer upsert
; idiom. The empty-MAP-fallback sibling is a known loud E0282 follow-up on the rust backends. ============
(case
  "an empty (list) match-fallback beside an unsolved-Var arm grounds to the join's list type"
  (doc
    "FINDING repro (ms13, fixed): `(match (Map.lookup m \\\"k\\\") ((Some ys) ys) ((None _u)
           (list)))` — the Some arm binds an open Var (the empty map's value type is only fixed
           downstream) and the fallback is an empty literal; the join must ground both arms to the
           downstream-determined list type. Runs 1 (one push onto the empty fallback).")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m Map.empty))
          (let
            ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) #list()))))
            (let ((nxs (List.push xs n))) (List.len nxs)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "an empty SET literal in a match-Option fallback grounds through the join"
  (doc
    "The Set sibling of the empty-literal join class: the fallback is `(Set.of (list))` and
           the downstream `Set.insert` fixes the element type — 1. The join ground must cover all
           collection kinds, not just List.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m Map.empty))
          (let
            ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) #set()))))
            (Set.len (Set.insert xs n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "an IF-join with an unsolved-Var arm and an empty-list sibling grounds like a match join"
  (doc
    "The join kind is irrelevant (a concrete-sibling IF always worked — the sibling supplied
           the evidence): an IF whose then-arm is a Map-lookup payload (open Var) and whose else is
           an empty `(list)` must ground through the same machinery as the match join — 1.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m Map.empty))
          (let
            ((xs (if (> n 0) (match (Map.lookup m "k") ((Some ys) ys) ((None _u) #list())) #list())))
            (List.len (List.push xs n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a MAP-OF-LISTS handler state accumulates per dispatch — the upsert idiom end to end"
  (doc
    "The real-world shape that found the class: a `(Map String (List Int64))` handler state
           with the lookup-fallback-push upsert arm — key a gets 3 appends, key b one; each resume
           returns the new inner length (1,1,2,3 → 1123). The two-layer state (CHAMP over RRB) must
           path-copy across resume cycles, and the empty-fallback join must ground (nested: the join
           sees only `List Any`, the emit's interior hole lets rustc solve `Vec<Vec<i64>>`).")
  (input
    (do
      (effect Db (op add (-> (Tuple String Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          Db
          Map.empty
          ((add
              (p)
              m
              (match
                p
                (#tuple(k v)
                  (let
                    ((xs (match (Map.lookup m k) ((Some ys) ys) ((None _u) #list()))))
                    (let ((nxs (List.push xs v))) (resume (List.len nxs) (Map.insert m k nxs))))))))
          (+
            (* 1000 (Db.add #tuple("a" n)))
            (+
              (* 100 (Db.add #tuple("b" 7)))
              (+ (* 10 (Db.add #tuple("a" 6))) (Db.add #tuple("a" 9)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1123 Int64))
  (live-objects known-leak))

(case
  "an empty MAP fallback beside an unsolved-Var arm grounds — the Map-of-Maps face"
  (doc
    "The last face of the empty-collection join class (fixed after the sibling pins above):
           when the joined collection is itself a MAP, the enclosing `Map.insert` types the JOIN
           result — the scrutinee's own lookup map must not inherit that typing (the leak sent its
           constructor down an unannotated branch → E0282). Now the lookup map annotates its outer
           key with the inner map holed for the downstream insert — runs 1.")
  (input
    (do
      (def
        (main (: n Int64))
        (let
          ((m Map.empty))
          (let
            ((inner (match (Map.lookup m "k") ((Some ys) ys) ((None _u) Map.empty))))
            (Map.len (Map.insert inner "x" n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a handler state SHRINKS per dispatch — Map.remove down to empty across resume cycles"
  (doc
    "The shrink direction of heap state (the growth pins push/insert): three evictions read
           lengths 2, 1, 0 — the removal path-copies and node-collapses must survive resume
           suspensions all the way to the empty map (210).")
  (input
    (do
      (effect Db (op evict (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          Db
          #map((= "a" n) (= "b" 7) (= "c" 9))
          ((evict (k) m (resume (Map.len (Map.remove m k)) (Map.remove m k))))
          (+ (* 100 (Db.evict "a")) (+ (* 10 (Db.evict "b")) (Db.evict "c")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 210 Int64)))

(case
  "a SET state churns — inserts and removes interleave across dispatches, canonical at each read"
  (doc
    "State churn including the re-insert-after-remove cycle on one key: flip 2 removes (len 2),
           flip 2 again re-inserts (len 3), flip 9 inserts fresh (len 4) → 234. The
           contains-conditional arm reads the CURRENT state each dispatch.")
  (input
    (do
      (effect St (op flip (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #set(1 2 3)
          ((flip
              (k)
              s
              (resume
                (Set.len (if (Set.contains s k) (Set.remove s k) (Set.insert s k)))
                (if (Set.contains s k) (Set.remove s k) (Set.insert s k)))))
          (+ (* 100 (St.flip 2)) (+ (* 10 (St.flip 2)) (St.flip 9)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 234 Int64)))

(case
  "the arm ENUMERATES its Map state to a list of tuples and resumes the enumeration"
  (doc
    "The enumeration itself crossing resume (the to-list pins fold INSIDE the arm and resume a
           scalar): `Map.to-list` of the state becomes the resume value; the body measures it and
           folds the values — 100·2 + 35 → 235.")
  (input
    (do
      (effect Db (op dump (-> Unit (List (Tuple String Int64)))))
      (def
        (sum-snd (: xs (List (Tuple String Int64))) (: i Int64) (: acc Int64))
        (match
          (List.at xs i)
          ((Some p) (match p (#tuple(k v) (sum-snd xs (+ i 1) (+ acc v)))))
          ((None _u) acc)))
      (def
        (main (: n Int64))
        (handle
          Db
          #map((= "a" n) (= "b" 30))
          ((dump (u) m (resume (Map.to-list m) m)))
          (let ((xs (Db.dump))) (+ (* 100 (List.len xs)) (sum-snd xs 0 0)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 235 Int64))
  (live-objects known-leak))

(case
  "Set.to-list of the state crosses resume ORDERED — the body reads elements positionally"
  (doc
    "The total-order contract surviving the marshal: the set {30, 5, 9} enumerates sorted
           [5, 9, 30], crosses resume, and the body reads each position — 1000·5 + 10·9 + 30 →
           5120. An unordered or order-scrambled marshal breaks the positional reads.")
  (input
    (do
      (effect St (op dump (-> Unit (List Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          #set(30 n 9)
          ((dump (u) s (resume (Set.to-list s) s)))
          (let
            ((xs (St.dump)))
            (+
              (* 1000 (match (List.at xs 0) ((Some a) a) ((None _u) -1)))
              (+
                (* 10 (match (List.at xs 1) ((Some b) b) ((None _u) -1)))
                (match (List.at xs 2) ((Some c) c) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5120 Int64)))

(case
  "a crossed rope String compares EQUAL to an arm-local flat literal — content equality over the marshal"
  (doc
    "Comparison ACROSS the marshal (the equality pins are body-side): a concat-built rope
           crosses as the op argument and the arm compares it against flat literals — content-equal
           (100), and lexicographic order both directions (\\\"abcde\\\" < \\\"abd\\\" holds at the
           third character, before length) → 110.")
  (input
    (do
      (effect St (op check (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((check
              (t)
              s
              (resume
                (+
                  (* 100 (if (= t "abcde") 1 0))
                  (+ (* 10 (if (< t "abd") 1 0)) (if (< "abd" t) 1 0)))
                s)))
          (St.check (String.concat "ab" (if (> n 0) "cde" "z")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "two crossed LISTS compare structurally in the arm — same content from different builders"
  (doc
    "Structural equality across the marshal AND across construction paths: a literal
           `(list 1 2 3)` and a recursively-built copy cross together in a tuple; the arm compares
           them equal (10), while different content compares unequal (0) → 10.")
  (input
    (do
      (effect St (op pair (-> (Tuple (List Int64) (List Int64)) Int64)))
      (def
        (build (: i Int64) (: k Int64) (: acc (List Int64)))
        (if (> i k) acc (build (+ i 1) k (List.push acc i))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((pair (p) s (match p (#tuple(xs ys) (resume (if (= xs ys) 1 0) s)))))
          (+
            (* 10 (St.pair #tuple(#list(1 2 3) (build 1 3 #list()))))
            (St.pair #tuple(#list(1 2) #list(1 2 9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (live-objects known-leak))

(case
  "the arm slices a crossed multibyte String at a scalar boundary — the slice window respects UTF-8"
  (doc
    "Char-indexed slicing over a marshaled rope: \\\"aédc\\\" crosses as the op argument and the
           arm slices chars [1,3) — \\\"éd\\\", whose two-byte é makes the byte-length 3. The
           char/byte distinction must survive the crossing.")
  (input
    (do
      (effect St (op cut (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((cut
              (t)
              s
              (resume (match (String.slice t 1 3) ((Some w) (String.byte-len w)) ((None _u) -1)) s)))
          (St.cut (String.concat "a" "édc"))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a mid-scalar byte window crosses to the arm and String.from-bytes declines it"
  (doc
    "The adversarial byte/char face: slicing é's continuation byte alone (`Bytes.slice b 1 1`
           of the 2-byte encoding) produces invalid UTF-8; the arm's `String.from-bytes` must
           validate the crossed window and decline None → -7, never construct a torn String.")
  (input
    (do
      (effect St (op cut (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((cut
              (b)
              s
              (resume
                (match
                  (Bytes.slice b 1 1)
                  ((Some w)
                    (match (String.from-bytes w) ((Some t) (String.byte-len t)) ((None _u) -7)))
                  ((None _u) -1))
                s)))
          (St.cut (String.to-bytes "é"))))
      (export main)))
  (call main (: 5 Int64))
  (output (: -7 Int64)))

(case
  "a pre-abort HOST call inside a NESTED strict operand is ISSUED before the abort abandons"
  (doc
    "The host face of the nested-operand abort-collapse fix (breaker ah-x1; the in-program
           twin is the ax4 pin): `(+ 999 (+ (ask.ask) (Bail.bail 7)))` — the pre-abort host call
           MUST be issued (an externally-visible side effect, committed before the abort), matching
           the do-spine guarantee. Before the fix the collapse dropped the call entirely (observed
           host-calls were empty).")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (host (ask) (handle Bail 0 ((bail (n) s n)) (+ 999 (+ (ask.ask) (Bail.bail 7))))))
      (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))

(case
  "an inner ABORT handle inside a MULTI-SHOT region — each forked branch runs its own abort"
  (doc
    "Multi-shot × abort composition: the body forks 2×2 (pick1 branches, pick2 re-forks inside
           each), and every fork's Bail handle aborts INDEPENDENTLY (stop(1)→3, stop(2)→6). Per
           pick1-branch k(v) = (10v+3)+(10v+6) = 20v+9; k(1)+k(2) → 78. The multi-shot fold's
           per-branch continuation copies must each carry their own abort machinery.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (effect Bail (op stop (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((pick (u) s (+ (resume 1 s) (resume 2 s))))
          (+
            (* 10 (Amb.pick))
            (handle Bail 0 ((stop (v) t (* v 3))) (+ 999 (Bail.stop (Amb.pick)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 78 Int64)))

(case
  "a single MUTUAL-recursion chain performs at its base — the cross-function fold serves it"
  (doc
    "Mutual recursion crossing the fold boundary: `ev ↔ od` alternate down to the base, which
           performs — ev(4) → od(3) → ev(2) → od(1) → ev(0) → count reads the seed 0. (TWO calls
           into the mutual pair under one handler is the current honest decline — the fold serves
           one mutual chain; the self-recursive twin folds at two sites.)")
  (input
    (do
      (effect St (op count (-> Unit Int64)))
      (def (ev (: k Int64)) (if (= k 0) (St.count) (od (- k 1))))
      (def (od (: k Int64)) (if (= k 0) (+ 100 (St.count)) (ev (- k 1))))
      (def (main (: n Int64)) (handle St 0 ((count (u) s (resume s (+ s 1)))) (ev 4)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a host response captured BEFORE a multi-shot region is shared by both branches — one host call"
  (doc
    "The safe complement of the host-composition invariant (a host call INSIDE a multi-shot
           region stays a decline — it would re-issue per fork): captured BEFORE the region, the
           response is a plain value both forks share — h=100, k(v) = 10v + 100, 110+120 → 230,
           with exactly ONE observed host call.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main)
        (host
          (ask)
          (let
            ((h (ask.ask)))
            (handle Amb 0 ((pick (u) s (+ (resume 1 s) (resume 2 s)))) (+ (* 10 (Amb.pick)) h)))))
      (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 230 Int64)))

(case
  "an inner arm performs TWO DISTINCT outer effects in one resume value — both thread per dispatch"
  (doc
    "One arm consulting two separate outer services (the config+counter idiom; the
           single-outer-effect arm-perform is pinned): each dispatch performs A AND B, and both
           states advance independently — d1: 5+100 = 105 (A 5→6, B 100→110), d2: 6+110 = 116 →
           105116.")
  (input
    (do
      (effect A (op geta (-> Unit Int64)))
      (effect B (op getb (-> Unit Int64)))
      (effect In (op go (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((geta (u) s (resume s (+ s 1))))
          (handle
            B
            100
            ((getb (u) t (resume t (+ t 10))))
            (handle In 0 ((go (u) w (resume (+ (A.geta) (B.getb)) w))) (+ (* 1000 (In.go)) (In.go))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105116 Int64)))

(case
  "the state-SWAP idiom — the op argument becomes the state and the OLD state returns"
  (doc
    "A full heap-state exchange per dispatch: `(swap (xs) s (resume s xs))` — the resumed
           value is the PRIOR state and the argument is installed as the next. Two swaps: the seed
           [1,2] comes back first (len 2), then the installed 4-element list (len 4) → 204. Both
           heap handles change hands without copies colliding.")
  (input
    (do
      (effect St (op swap (-> (List Int64) (List Int64))))
      (def
        (main (: n Int64))
        (handle
          St
          #list(1 2)
          ((swap (xs) s (resume s xs)))
          (let
            ((old (St.swap #list(n 7 8 9))))
            (let ((cur (St.swap #list()))) (+ (* 100 (List.len old)) (List.len cur))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 204 Int64)))

(case
  "a LIST built from three performs ESCAPES the handle — read intact outside the region"
  (doc
    "The region-lifetime face of heap × handlers (the escaping-closure pins cover captured
           VALUES): a list assembled from three advancing performs becomes the handle's RESULT and
           is read OUTSIDE the region — len 3, element [2] = 7 → 307. The handler's heap
           allocations must outlive its region.")
  (input
    (do
      (effect Cfg (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (let
          ((xs (handle Cfg n ((get (u) s (resume s (+ s 1)))) #list((Cfg.get) (Cfg.get) (Cfg.get)))))
          (+ (* 100 (List.len xs)) (match (List.at xs 2) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 307 Int64)))

(case
  "a MAP valued with perform results ESCAPES the handle — looked up outside the region"
  (doc
    "The keyed sibling of the escaping-list pin: a Map whose two values are advancing perform
           results (5, then 15 at +10 per dispatch) escapes as the handle's result and both keys
           look up correctly outside — 10·5 + 15 → 65.")
  (input
    (do
      (effect Cfg (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (let
          ((m
              (handle
                Cfg
                n
                ((get (u) s (resume s (+ s 10))))
                (Map.insert (Map.insert Map.empty "a" (Cfg.get)) "b" (Cfg.get)))))
          (+
            (* 10 (match (Map.lookup m "a") ((Some a) a) ((None _u) -1)))
            (match (Map.lookup m "b") ((Some b) b) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 65 Int64)))

(case
  "Record.with over a perform result — the ORIGINAL record survives beside the update"
  (doc
    "Record persistence under a handler (the pure-side Record.with pins live in 15-rows): both
           the original field and the update value are perform results, and the ORIGINAL record
           observably survives the update — r.b stays 100 while r2.b is the second dispatch's 6 →
           100·5 + 6 + 100 = 606. Persistent update, not in-place.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((r #record((= a (St.next)) (= b 100))))
            (let ((r2 (Record.with r #"b" (St.next)))) (+ (* 100 r2.a) (+ r2.b r.b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 606 Int64)))

(case
  "a FRAMED-Bytes handler state decoded and re-encoded by the arm per dispatch"
  (doc
    "The protocol-state-machine idiom (the Bytes-state pin is append-only): the state is a
           binary frame the arm bin-DECODES, transforms, and RE-ENCODES each dispatch — [3,500] →
           resume 3500, install [4,510]; second dispatch parses its predecessor's encoding → 4510 →
           35004510. Each dispatch must parse the previous dispatch's own encoding.")
  (input
    (do
      (effect Wire (op recv (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Wire
          (bin (u8 (UInt8.wrap 3)) (u16 (UInt16.wrap 500)))
          ((recv
              (u)
              s
              (match
                s
                ((bin (u8 tag) (u16 val))
                  (resume
                    (+ (* 1000 tag) val)
                    (bin (u8 (UInt8.wrap (+ tag 1))) (u16 (UInt16.wrap (+ val 10))))))
                (_other (resume -1 s)))))
          (+ (* 10000 (Wire.recv)) (Wire.recv))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 35004510 Int64)))

(case
  "arm-interned symbols keep content identity ACROSS dispatches — same content = same symbol"
  (doc
    "Interner coherence across dispatch round-trips: three dispatches each intern a
           branch-selected rope — the first and third produce the SAME content (\\\"id-hi\\\") and
           must compare equal across the two crossings (100); the middle differs (0); and the
           distinct symbols order content-lexicographically (\\\"id-hi\\\" < \\\"id-lo\\\", 1) →
           101.")
  (input
    (do
      (effect St (op tag (-> Int64 Symbol)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((tag (k) s (resume (Symbol.of (String.concat "id-" (if (> k 0) "hi" "lo"))) (+ s 1))))
          (let
            ((a (St.tag n)))
            (let
              ((b (St.tag 0)))
              (let
                ((c (St.tag 7)))
                (+ (* 100 (if (= a c) 1 0)) (+ (* 10 (if (= a b) 1 0)) (if (< a b) 1 0))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 101 Int64)))

(case
  "a complete handler inside a CLOSURE body — each application instantiates fresh"
  (doc
    "The handler-factory idiom (the arm-instantiated pins cover a handle in an ARM): the
           closure body IS a full handle, deferred until application and re-instantiated per apply
           from the argument seed — f(5) = 5+6 = 11, f(20) = 20+21 = 41 → 1141. No handler state
           survives between applications.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (let
          ((f
              (fn
                ((: k Int64))
                (handle St k ((next (u) s (resume s (+ s 1)))) (+ (St.next) (St.next))))))
          (+ (* 100 (f n)) (f 20))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1141 Int64)))

(case
  "the closure's inner arm performs an OUTER effect through the closure boundary"
  (doc
    "Cross-boundary arm-performs from a deferred handler: the closure's inner arm performs
           `Out.base` — resolved through the closure boundary to the enclosing handler, whose state
           threads ACROSS the two applications (f(1): base=5, Out 5→105; f(2): base=105) →
           1000·6 + 107 = 6107.")
  (input
    (do
      (effect Out (op base (-> Unit Int64)))
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Out
          n
          ((base (u) s (resume s (+ s 100))))
          (let
            ((f
                (fn
                  ((: k Int64))
                  (handle St k ((next (u) s (resume (+ s (Out.base)) (+ s 1)))) (St.next)))))
            (+ (* 1000 (f 1)) (f 2)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6107 Int64)))

(case
  "a record STATE with a Set field rebuilt per dispatch — dedup observed through the field"
  (doc
    "A named-field collection inside a record state (the split-state pins use positional
           tuples): the arm projects the Set field, rebuilds it, and packages a fresh record —
           dedup observes through the field: add 5 → len 1, duplicate 5 → still 1, add 7 → 2 →
           112. The seen-set + total accumulator idiom.")
  (input
    (do
      (effect Db (op add (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Db
          #record((= seen #set()) (= total 0))
          ((add
              (v)
              st
              (let
                ((ns (Set.insert st.seen v)))
                (resume (Set.len ns) #record((= seen ns) (= total (+ st.total v)))))))
          (+ (* 100 (Db.add n)) (+ (* 10 (Db.add n)) (Db.add 7)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 112 Int64)))

(case
  "a perform-seeded inner handle composed with a SECOND same-effect instantiation after it"
  (doc
    "The seed-position perform pin composed with sequential same-effect handles: B's seed is
           `(A.tick)` (evaluated in A's scope, advancing A 5→6), the region computes 5+15 = 20, and
           a SECOND fresh A after the region reads its own seed 50, no leftover state → 2050.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)))
      (effect B (op tock (-> Unit Int64)))
      (def
        (main (: n Int64))
        (+
          (*
            100
            (handle
              A
              n
              ((tick (u) s (resume s (+ s 1))))
              (handle B (A.tick) ((tock (u) t (resume t (+ t 10)))) (+ (B.tock) (B.tock)))))
          (handle A 50 ((tick (u) s (resume s (+ s 1)))) (A.tick))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2050 Int64)))

(case
  "a recursive TREE as a transformer op — crosses IN, the arm wraps it, crosses back OUT"
  (doc
    "The transformer face of recursive-sum crossings (result-only + abort-value are pinned): a
           body-built Tree spine crosses INTO the arm, which wraps it — embedding the crossed
           subtree BY REFERENCE in a new Node — and the wrapped spine crosses back; the recursive
           fold reads all three leaves: 5+7+10 → 22.")
  (input
    (do
      (type Tree (Leaf Int64) (Node (Tuple Tree Tree)))
      (effect St (op grow (-> Tree Tree)))
      (def
        (sum-t (: t Tree))
        (match t ((Tree.Leaf v) v) ((Tree.Node p) (match p (#tuple(l r) (+ (sum-t l) (sum-t r)))))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grow (t) s (resume (Tree.Node #tuple(t (Tree.Leaf 10))) s)))
          (sum-t (St.grow (Tree.Node #tuple((Tree.Leaf n) (Tree.Leaf 7)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 22 Int64))
  (live-objects known-leak))

(case
  "a heap result of effect A pipes directly into effect B's argument — cross-effect heap flow"
  (doc
    "Two marshals back-to-back on one heap value through two DIFFERENT handlers: `(B.use
           (A.mk n))` — the list exits A's arm and immediately enters B's, no intermediate binding.
           B reads len 2 and element [1] → 30. (The scalar pipe is pinned; the heap payload was
           not.)")
  (input
    (do
      (effect A (op mk (-> Int64 (List Int64))))
      (effect B (op use (-> (List Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          0
          ((mk (k) s (resume #list(k (* k 2)) s)))
          (handle
            B
            0
            ((use
                (xs)
                t
                (resume
                  (+ (* 10 (List.len xs)) (match (List.at xs 1) ((Some v) v) ((None _u) -1)))
                  t)))
            (B.use (A.mk n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 30 Int64)))

(case
  "Rational.of over TWO perform results — ctor-arg order observable through the dispatches"
  (doc
    "A heap-numeric CTOR consuming two perform results in one call: the numerator draws first
           (4), the denominator second (5) — strict LTR ctor-argument order made observable by the
           advancing state → 10·4 + 5 = 45 (consecutive draws are always coprime; the gcd face is
           the sibling below).")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((q (Rational.of (St.next) (St.next))))
            (+ (* 10 (Int64.of (Rational.numerator q))) (Int64.of (Rational.denominator q))))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 45 Int64)))

(case
  "the gcd face: a reducible num/den pair from performs canonicalizes (4/8 → 1/2)"
  (doc
    "The canonicalization sibling: a DOUBLING state makes the two draws reducible (4 then 8),
           and the constructed rational must arrive gcd-canonical — 1/2, not 4/8 → 12.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (* s 2))))
          (let
            ((q (Rational.of (St.next) (St.next))))
            (+ (* 10 (Int64.of (Rational.numerator q))) (Int64.of (Rational.denominator q))))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 12 Int64)))

(case
  "a tuple scrutinee built from TWO performs — the guard relates both dispatch results"
  (doc
    "Cross-dispatch guard conditions (the guard pins destructure a SINGLE perform-result
           tuple): the scrutinee assembles two draws — `(tuple (St.next) (St.next))` — and the
           guard relates them: `(= (+ a 1) b)` holds for consecutive draws (a=5, b=6) → 506. The
           strict tuple-operand order, the guard desugar, and the between-draws state advance
           compose.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (match
            #tuple((St.next) (St.next))
            ((guard #tuple(a b) (= (+ a 1) b)) (+ (* 100 a) b))
            (#tuple(a b) (- 0 (+ a b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 506 Int64))
  (live-objects 0))

(case
  "recursively STACKED same-effect handlers — the perform resolves to the DEEPEST frame"
  (doc
    "A DYNAMICALLY-built shadow stack (the shadow pins are lexical 2-deep literals): the
           recursion installs a fresh same-effect handler per level — walk 3 stacks handlers seeded
           3, 2, 1 — and the base perform resolves to the nearest (deepest, k=1) frame → 1.")
  (input
    (do
      (effect St (op depth (-> Unit Int64)))
      (def
        (walk (: k Int64))
        (if (= k 0) (St.depth) (handle St k ((depth (u) s (resume s s))) (walk (- k 1)))))
      (def (main (: n Int64)) (handle St 100 ((depth (u) s (resume s s))) (walk 3)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "the shadow stack UNWINDS — each level's post-region perform reaches ITS enclosing frame"
  (doc
    "The unwind discipline of the dynamic shadow stack: after each recursive region closes,
           the SAME textual `(St.depth)` resolves one frame further out — the base reads 1 (k=1
           frame), the next level's post-region read gets 2 (k=2 frame), and the outermost reads
           the root 100 → (1+2) + 100 = 103. One call site, three different homes across the
           unwind.")
  (input
    (do
      (effect St (op depth (-> Unit Int64)))
      (def
        (walk (: k Int64))
        (if
          (= k 0)
          (St.depth)
          (+ (handle St k ((depth (u) s (resume s s))) (walk (- k 1))) (St.depth))))
      (def (main (: n Int64)) (handle St 100 ((depth (u) s (resume s s))) (walk 2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 103 Int64)))

(case
  "a perform-capturing closure passed to a HIGHER-ORDER helper applies twice under the handler"
  (doc
    "A perform-capture flowing through a HOF parameter INSIDE the live region (the escaping
           pins apply OUTSIDE; performing closures through combinators are the documented decline):
           `apply2 = f∘f` composes the capture twice — base = 5, f(f(100)) → 110.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def (apply2 (: f (-> Int64 Int64)) (: x Int64)) (f (f x)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((base (St.next))) (apply2 (fn ((: x Int64)) (+ x base)) 100))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "a perform-capture through a TUPLE-returning HOF — both results carry the capture"
  (doc
    "The multi-result HOF face: `map2` applies the capture to two arguments and returns both
           in a tuple — k = 50, (1+50, 2+50) → 5152. One capture environment, two applications, two
           results.")
  (input
    (do
      (effect St (op scale (-> Int64 Int64)))
      (def (map2 (: f (-> Int64 Int64)) (: a Int64) (: b Int64)) #tuple((f a) (f b)))
      (def
        (main (: n Int64))
        (handle
          St
          10
          ((scale (v) s (resume (* v s) s)))
          (let
            ((k (St.scale n)))
            (match (map2 (fn ((: x Int64)) (+ x k)) 1 2) (#tuple(p q) (+ (* 100 p) q))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5152 Int64)))

(case
  "a TWO-argument capture closure through a fold-style HOF — the capture rides both applications"
  (doc
    "The multi-parameter closure face: a two-arg fn capturing the perform result threads
           through a fold-style HOF (`fold3 f a b c = f (f a b) c`) — w = 5, f(1,2) = 7, f(7,3) →
           38. The capture environment must sit beside TWO positional params per application.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def (fold3 (: f (-> Int64 Int64 Int64)) (: a Int64) (: b Int64) (: c Int64)) (f (f a b) c))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((w (St.next))) (fold3 (fn ((: x Int64) (: y Int64)) (+ (* x w) y)) 1 2 3))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 38 Int64)))

(case
  "a perform-derived list SHARED by two tuples — both readers see one allocation's content"
  (doc
    "A perform-derived allocation aliased into TWO containers (RC ≥ 2) inside the live region
           (the aliased-heap pins are pure-side): both tuples wrap the same list; one reads element
           0 (5), the other reads the length (2) → 502.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((shared #list((St.next) 100)))
            (let
              ((t1 #tuple(shared 1)))
              (let
                ((t2 #tuple(shared 2)))
                (+
                  (*
                    100
                    (match t1 (#tuple(xs _k) (match (List.at xs 0) ((Some v) v) ((None _u) -1)))))
                  (match t2 (#tuple(ys _k) (List.len ys)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 502 Int64)))

(case
  "pushing onto a SHARED perform-built list — the original stays len 2 beside the grown copy"
  (doc
    "Persistence under sharing inside the region: the aliased list is pushed onto WHILE a
           second reference holds it — the original must stay len 2 beside the grown len-3 copy
           (path-copy, not in-place) → 203. The second push draw (6) also witnesses the state
           advancing between the two performs.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((shared #list((St.next) 100)))
            (let
              ((grown (List.push shared (St.next))))
              (+ (* 100 (List.len shared)) (List.len grown))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 203 Int64)))

(case
  "identical PURE reads of a perform-built list — safe to share, values agree"
  (doc
    "The complement of the dispatch-CSE exclusions (identical PERFORMS must stay distinct):
           identical PURE reads over an effect-derived value MAY share and must agree — `List.len
           xs` twice over the same perform-built list → 100·2 + 2 = 202. A wrongly-shared read
           holding a stale heap snapshot would diverge.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((xs #list((St.next) (St.next)))) (+ (* 100 (List.len xs)) (List.len xs)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 202 Int64)))

(case
  "a region's RESULT seeds a second same-effect region with a DIFFERENT arm shape"
  (doc
    "The pipeline-of-interpreters idiom (the sequential pins prove fresh-start with the SAME
           arm): region 1's add-arm computes 11, which seeds region 2 under a DOUBLING arm — the
           same op name means different things per region: 11 + 22 → 33.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (let
          ((total (handle St n ((next (u) s (resume s (+ s 1)))) (+ (St.next) (St.next)))))
          (handle St total ((next (u) s (resume s (* s 2)))) (+ (St.next) (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 33 Int64)))

(case
  "a PARAMETERIZED handler helper chained through itself — the step size is a function param"
  (doc
    "An arm referencing the enclosing FUNCTION's parameter directly (the closure pins
           parameterize via captures): `run seed mul` steps by `mul` per dispatch, chained —
           `(run (run n 1) 10)` = inner 5+6 = 11, outer 11+21 → 32. Two instantiations, two
           different step sizes through one textual arm.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (run (: seed Int64) (: mul Int64))
        (handle St seed ((next (u) s (resume s (+ s mul)))) (+ (St.next) (St.next))))
      (def (main (: n Int64)) (run (run n 1) 10))
      (export main)))
  (call main (: 5 Int64))
  (output (: 32 Int64)))

(case
  "the SAME op name on two DIFFERENT effects — each qualified perform routes to its own handler"
  (doc
    "Name-collision routing: effects A and B both declare `get`; the qualified performs
           `(A.get)` and `(B.get)` each resolve to their OWN effect's handler (5 and 100) → 150.
           Routing is by effect identity, not op-name string.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((get (u) s (resume s (+ s 1))))
          (handle B 100 ((get (u) t (resume t (+ t 10)))) (+ (* 10 (A.get)) (B.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 150 Int64)))

(case
  "a SYMBOL-keyed Map built from performs — interned keys look up across separate interns"
  (doc
    "Interner-hash-keyed CHAMP coherence under a handler (the Symbol-key pins are pure-side):
           the map's values come from performs, and the lookups re-intern \\\"a\\\"/\\\"b\\\" as
           SEPARATE Symbol.of calls — content identity must route to the stored slots: 10·5 + 6 →
           56.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((m
                (Map.insert
                  (Map.insert Map.empty (Symbol.of "a") (St.next))
                  (Symbol.of "b")
                  (St.next))))
            (+
              (* 10 (match (Map.lookup m (Symbol.of "a")) ((Some v) v) ((None _u) -1)))
              (match (Map.lookup m (Symbol.of "b")) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "a SET of arm-interned symbols dedups across dispatches — warm appears twice, stored once"
  (doc
    "Dedup of symbols that each crossed the boundary in DIFFERENT dispatches: three dispatches
           intern cold, warm, warm (branch-selected on the advancing state) — the Set stores the
           content-identical pair once → len 2.")
  (input
    (do
      (effect St (op label (-> Unit Symbol)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((label (u) s (resume (Symbol.of (if (> s 0) "warm" "cold")) (+ s 1))))
          (let ((xs #set((St.label) (St.label) (St.label)))) (Set.len xs))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "a symbol round-trip between TWO ops of one effect — interned by one, judged by the other"
  (doc
    "The two-op interner service: op 1 interns and returns the symbol; op 2 receives it BACK
           as an argument and compares against its own intern — the round-tripped symbol matches
           (10 at s=1), a different content does not (-1) → 999.")
  (input
    (do
      (effect Reg (op intern (-> String Symbol)) (op which (-> Symbol Int64)))
      (def
        (main (: n Int64))
        (handle
          Reg
          0
          ((intern (t) s (resume (Symbol.of t) (+ s 1)))
            (which (sym) s (resume (if (= sym (Symbol.of "hot")) (* s 10) (- 0 s)) s)))
          (let ((a (Reg.intern "hot"))) (+ (* 100 (Reg.which a)) (Reg.which (Symbol.of "cold"))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 999 Int64)))

(case
  "an indexed list walk performing per element — element × advancing draw, summed"
  (doc
    "The zip-with-effects idiom (the walk pins are draw-only or element-only): each element
           multiplies an ADVANCING draw — element order and dispatch order must stay locked:
           1·5 + 2·6 + 3·7 → 38.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (go (: xs (List Int64)) (: i Int64) (: acc Int64))
        (match (List.at xs i) ((Some v) (go xs (+ i 1) (+ acc (* v (St.next))))) ((None _u) acc)))
      (def (main (: n Int64)) (handle St n ((next (u) s (resume s (+ s 1)))) (go #list(1 2 3) 0 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 38 Int64))
  (live-objects known-leak))

(case
  "a map-via-effects walk — each element transformed by a dispatch, output order preserved"
  (doc
    "The MAP direction of the element×dispatch pairing (il-fold pins the accumulate
           direction): each element crosses as an op argument and its transform is pushed to an
           output list — ORDER preserved: [3,1,2]·10 → [30,10,20] → 3120. A dispatch-reordering
           bug scrambles the list, not just a sum.")
  (input
    (do
      (effect Pick (op at (-> Int64 Int64)))
      (def
        (build (: xs (List Int64)) (: i Int64) (: out (List Int64)))
        (match
          (List.at xs i)
          ((Some v) (build xs (+ i 1) (List.push out (Pick.at v))))
          ((None _u) out)))
      (def
        (main (: n Int64))
        (handle
          Pick
          0
          ((at (v) s (resume (* v 10) (+ s 1))))
          (let
            ((out (build #list(3 1 2) 0 #list())))
            (+
              (* 100 (match (List.at out 0) ((Some a) a) ((None _u) -1)))
              (+
                (* 10 (match (List.at out 1) ((Some b) b) ((None _u) -1)))
                (match (List.at out 2) ((Some c) c) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3120 Int64))
  (live-objects known-leak))

(case
  "filter-via-effects — a STATEFUL predicate dispatch decides each element's survival"
  (doc
    "The FILTER direction: each element crosses to a predicate arm whose threshold ADVANCES
           per dispatch (0, 2, 4, 6) — 1>0 keep, 4>2 keep, 2>4 drop, 9>6 keep → [1,4,9], len 3,
           element [1] = 4 → 304. Survival depends on the dispatch-ordered state, so a reorder
           changes WHICH elements survive.")
  (input
    (do
      (effect Keep (op test (-> Int64 Int64)))
      (def
        (sift (: xs (List Int64)) (: i Int64) (: out (List Int64)))
        (match
          (List.at xs i)
          ((Some v) (sift xs (+ i 1) (if (> (Keep.test v) 0) (List.push out v) out)))
          ((None _u) out)))
      (def
        (main (: n Int64))
        (handle
          Keep
          0
          ((test (v) s (resume (if (> v s) 1 0) (+ s 2))))
          (let
            ((out (sift #list(1 4 2 9) 0 #list())))
            (+ (* 100 (List.len out)) (match (List.at out 1) ((Some b) b) ((None _u) -1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 304 Int64))
  (live-objects known-leak))

(case
  "a byte-walk lexer performing per byte — Bytes.at pairs with an advancing draw per position"
  (doc
    "The iteration-triad discipline extended to BYTES walks (the bin-decode pins parse frames
           whole): each Bytes.at read pairs with an advancing draw — digits 1,2,3 + draws 0,1,2
           positionally encoded → 135. The incremental-lexer idiom.")
  (input
    (do
      (effect Tok (op take (-> Unit Int64)))
      (def
        (lex (: b Bytes) (: i Int64) (: acc Int64))
        (match
          (Bytes.at b i)
          ((Some c) (lex b (+ i 1) (+ (* acc 10) (+ c (Tok.take)))))
          ((None _u) acc)))
      (def
        (main (: n Int64))
        (handle
          Tok
          0
          ((take (u) s (resume s (+ s 1))))
          (lex (bin (u8 (UInt8.wrap 1)) (u8 (UInt8.wrap 2)) (u8 (UInt8.wrap 3))) 0 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 135 Int64))
  (live-objects known-leak))

(case
  "an emit/flush byte-writer seeded EMPTY — three emits accumulate, flush reads the frame back"
  (doc
    "The empty-seed writer (the wire-accumulator pin seeds non-empty and returns lengths):
           three Unit-returning emits accumulate onto an empty `(bin)` state and a separate flush op
           reads the whole frame — [5,9,2], len 3, byte [1] = 9 → 309.")
  (input
    (do
      (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
      (def
        (main (: n Int64))
        (handle
          Sink
          (bin)
          ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
            (flush (u) b (resume b b)))
          (do
            (Sink.emit n)
            (Sink.emit 9)
            (Sink.emit 2)
            (let
              ((out (Sink.flush)))
              (+ (* 100 (Bytes.len out)) (match (Bytes.at out 1) ((Some x) x) ((None _u) -1)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 309 Int64)))

(case
  "a reader→writer PUMP then a LET-bound flush reads the FULL accumulation (breaker tk3d class)"
  (doc
    "FINDING repro (tk3d, fixed): a cross-function performing helper (`pump` emits 3 draws
           onto the Sink state) followed by a LET-BOUND flush — the perform-walk once mis-treated
           the let's bindings as an Apply and never saw the flush observing pump's out-state, so
           the let read the SEED (len 0, or -1 through the at-readout). Now the full pipeline
           reads [5,6,7]: len 3, byte [2] = 7 → 307. (The scalar face is pinned alongside by the
           fix; the strict-operand and no-helper shapes always worked.)")
  (input
    (do
      (effect Src (op read (-> Unit Int64)))
      (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
      (def (pump (: k Int64)) (if (= k 0) unit (do (Sink.emit (Src.read)) (pump (- k 1)))))
      (def
        (main (: n Int64))
        (handle
          Src
          n
          ((read (u) s (resume s (+ s 1))))
          (handle
            Sink
            (bin)
            ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
              (flush (u) b (resume b b)))
            (do
              (pump 3)
              (let
                ((out (Sink.flush)))
                (+ (* 100 (Bytes.len out)) (match (Bytes.at out 2) ((Some x) x) ((None _u) -1))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 307 Int64))
  (live-objects known-leak))

(case
  "a BigInt arithmetic tower over one perform draw — cube then divide, narrowed once"
  (doc
    "Multi-step exact arithmetic downstream of a single crossing (the argument pins do one op
           per crossing): the draw cubes then integer-divides — 5³/5 = 25, narrowed once through
           checked Int64.of.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((b (BigInt.of (St.next))))
            (let ((big (* b (* b b)))) (Int64.of (/ big (BigInt.of 5)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 25 Int64)))

(case
  "a Rational SQUARE over a perform draw — 1/5 squared stays exactly 1/25"
  (doc
    "The exact-fraction tower sibling: 1/draw squared must stay exact — (1/5)² = 1/25 →
           10·1 + 25 = 35.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((r (Rational.of 1 (St.next))))
            (let
              ((sq (* r r)))
              (+ (* 10 (Int64.of (Rational.numerator sq))) (Int64.of (Rational.denominator sq)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 35 Int64)))

(case
  "a perform-derived rope SELF-concatenated — the shared subtree measures correctly"
  (doc
    "The DAG-sharing face of ropes under effects (the sharing pins are pure-side): a rope whose
           content was BRANCH-SELECTED by a perform is concatenated with ITSELF — one subtree
           referenced twice — and both the doubled rope and the original measure right:
           \\\"xbigxbig\\\" len 8 beside \\\"xbig\\\" len 4 → 84.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((w (String.concat "x" (if (> (St.next) 4) "big" "sm"))))
            (let
              ((again (String.concat w w)))
              (+ (* 10 (String.byte-len again)) (String.byte-len w))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 84 Int64)))

(case
  "a perform-derived byte-rope SELF-concatenated — both halves read the same draw"
  (doc
    "The byte-rope sibling: a bin frame built from a let-lifted draw (the inline-perform bin
           segment is the pinned strict-segment decline) concatenated with itself — [5,5], len 2,
           both bytes the same draw → 255.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((v (St.next)))
            (let
              ((b (bin (u8 (UInt8.wrap v)))))
              (let
                ((dbl (Bytes.concat b b)))
                (+
                  (* 100 (Bytes.len dbl))
                  (+
                    (* 10 (match (Bytes.at dbl 0) ((Some x) x) ((None _u) -1)))
                    (match (Bytes.at dbl 1) ((Some y) y) ((None _u) -1)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 255 Int64)))

(case
  "a nested-list DAG — one perform-derived inner list aliased TWICE in the outer"
  (doc
    "The container-DAG face: an inner list built from a draw sits at BOTH positions of an
           outer list — the alias reads through either path: nested[1][0] = 5, outer len 2 →
           205.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((v (St.next)))
            (let
              ((xs #list(v v)))
              (let
                ((nested #list(xs xs)))
                (+
                  (* 100 (List.len nested))
                  (match
                    (List.at nested 1)
                    ((Some inner) (match (List.at inner 0) ((Some x) x) ((None _u) -1)))
                    ((None _u) -1))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 205 Int64)))

(case
  "two perform-drawn quantities of one dimension ADD — same-unit combine over two crossings"
  (doc
    "Dimension arithmetic over draws (the qy pins do crossings, not arithmetic): two separately
           drawn meter quantities add — the erased-unit typing must agree across two crossings:
           5m + 6m → 11.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((q1 (Qty.of (St.next) (Unit.base #"meter"))))
            (let ((q2 (Qty.of (St.next) (Unit.base #"meter")))) (Qty.value (+ q1 q2))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a perform-drawn quantity MULTIPLIES across dimensions — meter·second product value"
  (doc
    "The free-abelian product over an effect-derived magnitude: 5m · 2s = 10 m·s — the
           dimension algebra composes with a drawn operand.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((d (Qty.of (St.next) (Unit.base #"meter"))))
            (let ((t (Qty.of 2 (Unit.base #"second")))) (Qty.value (* d t))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a perform-drawn quantity DIVIDES across dimensions — the meter/second quotient value"
  (doc
    "The quotient face (velocity dimension): 30m / 3s = 10 m/s over a drawn dividend — the
           dimension quotient composes with effect-derived magnitudes.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((d (Qty.of (* (St.next) 6) (Unit.base #"meter"))))
            (let ((t (Qty.of 3 (Unit.base #"second")))) (Qty.value (/ d t))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "host calls in the seed AND the next-state slot — the FINAL dispatch's state call is elided"
  (doc
    "The seed-position host pin composed with the state-slot host pin in ONE handler: the seed
           consumes response 1 (100), the first dispatch's state advance consumes response 2 (7 →
           state 107), and the SECOND dispatch's own next-state is never evaluated (nothing after
           reads it) so its host call is correctly ELIDED — 10·100 + 107 → 1107, exactly TWO calls.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (host
          (ask)
          (handle
            St
            (ask.ask)
            ((next (u) s (resume s (+ s (ask.ask)))))
            (+ (* 10 (St.next)) (St.next)))))
      (export main)))
  (call main (: 0 Int64))
  (host-responses (respond ask.ask (: 100 Int64)) (respond ask.ask (: 7 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 1107 Int64)))

(case
  "two host-seeded SIBLING handlers — each region's seed consumes its response in evaluation order"
  (doc
    "Two sibling regions each seeded by a host call: strict left-to-right evaluation routes
           response 1 (7) to the left region's seed and response 2 (9) to the right's — 100·7 + 9 →
           709, exactly two calls.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect A (op get (-> Unit Int64)))
      (effect B (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (host
          (ask)
          (+
            (* 100 (handle A (ask.ask) ((get (u) s (resume s s))) (A.get)))
            (handle B (ask.ask) ((get (u) t (resume t t))) (B.get)))))
      (export main)))
  (call main (: 0 Int64))
  (host-responses (respond ask.ask (: 7 Int64)) (respond ask.ask (: 9 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 709 Int64)))

(case
  "a host call SANDWICHED between two in-program dispatches — state survives the boundary crossing"
  (doc
    "The interleaving face: dispatch (a=0, state→1), HOST call (h=7), dispatch (b=1) — the
           in-program handler's state survives the host boundary crossing between its dispatches →
           10·7 + 1 = 71.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (host
          (ask)
          (handle
            St
            0
            ((next (u) s (resume s (+ s 1))))
            (let
              ((a (St.next)))
              (let ((h (ask.ask))) (let ((b (St.next))) (+ (* 100 a) (+ (* 10 h) b))))))))
      (export main)))
  (call main (: 0 Int64))
  (host-responses (respond ask.ask (: 7 Int64)))
  (host-calls (call ask.ask))
  (output (: 71 Int64)))

; ============ Performing LITERAL scrutinees by kind (breaker rw/gv survey). Every BY-POSITION
; destructure — tuple, user sum, std sum, list — evaluates a performing literal scrutinee ONCE and
; binds the materialized value. The RECORD pattern (the sole by-NAME projection kind) is the known
; exception: its projection desugar re-lowers a literal scrutinee per bound field (a rejected
; conjunction pending the bind-once fold fix — the fix's acceptance flips it to these siblings'
; semantics). These pin the four correct kinds' single-eval discipline. ============
(case
  "a TUPLE-literal scrutinee with performing fields — draws fire once, bind by position"
  (doc
    "The tuple kind: `(match (tuple (St.next) (St.next)) ((tuple x y) …))` — the literal
           evaluates once, x=5 y=6 → 56.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (match #tuple((St.next) (St.next)) (#tuple(x y) (+ (* 10 x) y)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "a USER-SUM-literal scrutinee with a performing payload — the ctor pattern binds once"
  (doc
    "The user-sum kind, with a dispatch-count witness: `(Box.Box (St.next))` matched by the
           ctor pattern binds the payload from ONE evaluation (v=5 → 500) and the post-match draw
           reads the committed advance (6) → 506.")
  (input
    (do
      (type Box (Box Int64))
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (* 100 (match (Box.Box (St.next)) ((Box.Box v) v))) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 506 Int64)))

(case
  "a STD-SUM (Option) literal scrutinee — single eval, payload binds by position"
  (doc
    "The std-sum kind with the same witness: `(Some (St.next))` binds x=5 from one evaluation
           (→ 5000) and the post-draw reads 6 → 5006.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (* 100 (match (Some (St.next)) ((Some x) (* x 10)) ((None _u) -1))) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5006 Int64)))

(case
  "a LIST-literal scrutinee with performing elements — draws fire once, bind by position"
  (doc
    "The list kind: `(match (list (St.next) (St.next)) ((list x y) …))` evaluates once —
           x=5 y=6 → 56.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (match #list((St.next) (St.next)) (#list(x y) (+ (* 10 x) y)) (_other -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "a performing record nested in a TUPLE scrutinee — bound by position, record-matched after"
  (doc
    "The safe composition (and the workaround for the record-literal conjunction): the tuple
           destructure binds the record by POSITION from one evaluation, and the record-match of
           the BOUND value projects without re-evaluation — x=5 y=6 → 56.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (match
            #tuple(#record((= a (St.next))) (St.next))
            (#tuple(r y) (match r (#record((= a x)) (+ (* 10 x) y)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

; ============ Guarded match × effects (breaker FINDING, ag5 → fixed #2333). A guarded match on a
; perform-result scrutinee whose FALLBACK arm also performs used to leak the fold-synthesized #seed
; binder as a false CDZ0101: the guard desugar's arm-body copy reparented a reused (shared) body
; without the seed-lift let, stranding the reference. The fix pins reused guarded-match arm bodies
; at desugar entry (and drops the blanket forget). These four pin the served class: the repro, the
; guard-TRUE runtime path, the performing-guard-CONDITION position, and the multi-guard chain. The composed
; face — a guard FALLBACK containing a two-site multi-perform arm — is a separate machinery
; composition that declines cleanly (guard-desugar copy × two-hole refold). ============
(case
  "a guarded match on a perform-result scrutinee with a PERFORMING fallback arm folds"
  (doc
    "FINDING repro (ag5, fixed #2333): `(match (St.roll) ((guard v (> v 6)) …) (v (+ (* 10
           (St.roll)) v)))` — the scrutinee is a perform result AND the fallback arm performs again.
           This exact conjunct leaked `#seed` as a false CDZ0101 before the fix (either alone was
           fine — the controls below). roll → 5 (state 5→8), guard 5>6 misses, fallback: 10·8 + 5 =
           85.")
  (input
    (do
      (effect St (op roll (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((roll (u) s (resume s (+ s 3))))
          (match (St.roll) ((guard v (> v 6)) (* v 100)) (v (+ (* 10 (St.roll)) v)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 85 Int64)))

(case
  "the guard-TRUE path of the perform-scrutinee match (no fallback entry)"
  (doc
    "The same shape called with a guard-passing input: roll → 9 (state 9→12), 9 > 6 holds, so
           the guarded arm answers 900 and the performing fallback is never entered. With the repro
           above, pins BOTH runtime paths of the served shape — the fallback's perform must neither
           fire on this path nor confuse the fold on the other.")
  (input
    (do
      (effect St (op roll (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((roll (u) s (resume s (+ s 3))))
          (match (St.roll) ((guard v (> v 6)) (* v 100)) (v v))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 900 Int64)))

(case
  "a guard whose CONDITION itself performs (pure scrutinee) folds"
  (doc
    "The third position a perform can occupy in a guarded match: the GUARD CONDITION `(> (St.roll)
           4)` — scrutinee (`n`, pure) and arm bodies effect-free. roll → 5 (once; the guard evaluates
           only after its pattern matches), 5 > 4 holds → 5·100 = 500. Completes the position triple
           with the scrutinee-perform and fallback-perform pins above.
           UPDATE (guards-side-effect-free, CDZ0407): `(St.roll)` in the guard cond is NOW a COMPILE ERROR.")
  (input
    (do
      (effect St (op roll (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((roll (u) s (resume s (+ s 3))))
          (match n ((guard v (> (St.roll) 4)) (* v 100)) (v v))))
      (export main)))
  (error CDZ0407))

(case
  "a MULTI-guard chain on a perform-result scrutinee with a performing fallback folds"
  (doc
    "The chain face of the fixed class: TWO guarded arms cascade over the perform-result
           scrutinee before the performing fallback — the arm-body pinning must hold across every
           reused body in the cascade, not just one. roll → 5, 5>20 misses, 5>6 misses, fallback:
           10·8 + 5 = 85.")
  (input
    (do
      (effect St (op roll (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((roll (u) s (resume s (+ s 3))))
          (match
            (St.roll)
            ((guard v (> v 20)) (* v 1000))
            ((guard v (> v 6)) (* v 100))
            (v (+ (* 10 (St.roll)) v)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 85 Int64)))

(case
  "a perform result LET-bound then fed to a bin segment builds Bytes under a handler"
  (doc
    "bin × effects: a `bin` integer segment is a STRICT operand position (a perform INLINE in the
           segment is the not-yet-reducible strict-ctor boundary, like try operands), but the LET-BOUND
           route folds — `(let ((v (UInt8.wrap (St.next)))) (bin (u8 v)))` discharges the perform first
           and feeds the pure UInt8. Seed 5 → byte 5 read back via `Bytes.at` → 5. Pins the
           wire-protocol-under-effects authoring idiom (bind performs, then construct).")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((v (UInt8.wrap (St.next))))
            (match (Bytes.at (bin (u8 v)) 0) ((Some b) (Int64.of b)) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "performs INLINE in record fields fold (records are not a strict-ctor boundary)"
  (doc
    "The record-vs-bin ctor CONTRAST: unlike a bin segment (strict — inline performs decline,
           see the let-bound pin above), a record constructor's fields accept performs INLINE —
           `(record (lo (St.next)) (hi (St.next)))` folds, and the checksum doubles as a left-to-right
           field-evaluation witness: lo gets the FIRST dispatch (5), hi the second (6) → 506. Same
           shape, different constructor class, opposite result — pins the boundary's exact extent.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((r #record((= lo (St.next)) (= hi (St.next))))) (+ (* 100 r.lo) r.hi))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 506 Int64)))

(case
  "let-bound perform results stored into record fields"
  (doc
    "The conservative route beside the inline pin above: both performs discharge into lets first,
           then the record is built from pure bindings — same 506. Both routes fold for records; only
           bin requires the let-bound spelling.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let
            ((a (St.next)))
            (let ((b (St.next))) (let ((r #record((= lo a) (= hi b)))) (+ (* 100 r.lo) r.hi))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 506 Int64)))

(case
  "TWO closures capture one let-bound perform result — the effect fires ONCE"
  (doc
    "The single-firing guarantee of a shared capture: `v = (St.pull)` fires once (reading 40),
           and BOTH closures close over the same `v` — f(1) = 41, g(2) = 80 → 121. A desugar that
           re-fired the perform per capturing closure would give g a 41 (→ 82, total 123). The
           sharing shape the host-closure machinery relies on, in-program form.")
  (input
    (do
      (effect St (op pull (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          40
          ((pull (u) s (resume s (+ s 1))))
          (let
            ((v (St.pull)))
            (let ((f (fn ((: x Int64)) (+ x v))) (g (fn ((: x Int64)) (* x v)))) (+ (f 1) (g 2))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 121 Int64)))

(case
  "a closure's captured perform result survives a LATER state advance (capture-time, not re-read)"
  (doc
    "The temporal face of eval-once capture: `v` captures 40, a DIFFERENT op then advances the
           state (+10), and only then does the closure fire — the captured 40 survives (41). A lazy
           capture that re-evaluated the perform (or re-read the state) at application would give 52.
           With the single-firing pin above, the capture-semantics pair.")
  (input
    (do
      (effect St (op pull (-> Unit Int64)) (op bump (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          40
          ((pull (u) s (resume s (+ s 1))) (bump (u) s (resume s (+ s 10))))
          (let ((v (St.pull))) (let ((f (fn ((: x Int64)) (+ x v)))) (do (St.bump) (f 1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 41 Int64)))

(case
  "a constant bin construction folds alongside performs in the same handle body"
  (doc
    "The pure-construction control of the bin × effects pair: `(bin (u16 258) (u8 7))` has only
           literal segments, so it is a pure Bytes value the fold treats as opaque data while the sibling
           `(St.next)` discharges normally — 3 + 5 = 8. Pins that a bin ctor's presence does not
           de-classify the body (the effect-reachability walk sees the ctor as pure).")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (Bytes.len (bin (u16 258) (u8 7))) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 8 Int64)))

(case
  "String.slice of a Map-looked-up String with perform-threaded start and end folds"
  (doc
    "The String sibling of the looked-up-Bytes slice shape (whose wasm scratch-alias miscompile is
           separately pinned in 10-bytes): the string comes back through `Map.lookup` and BOTH slice
           operands are perform results — start 1, end 2 → slice \"b\", byte-len 1. Note String.slice is
           (start, END) where Bytes.slice is (start, LEN), and returns Option. Pins the looked-up-payload
           × perform-operand shape folding for the String emit.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def table (Map.insert Map.empty 1 "abcdefgh"))
          (handle
            St
            n
            ((next (u) s (resume s (+ s 1))))
            (match
              (Map.lookup table 1)
              ((Some str)
                (match
                  (String.slice str (St.next) (St.next))
                  ((Some sl) (String.byte-len sl))
                  ((None _u) -100)))
              ((None _u) -200)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "a let-bound value in a handle body flows into a perform's argument (the always-worked twin)"
  (doc
    "The let-twin of the do-def perform-arg repro above — the semantically identical shape with the
           value `let`-bound instead of do-def. This ALWAYS computed correctly (the let rebuilt its scope,
           so the perform-arg path saw the binding); it's the reference the fix normalized the do-def form
           to match. `run 5`: v = 7, `(Ask.ask 7)`→14, +7 → 21. Both backends. Pinned as the regression
           twin so a future fold change that re-breaks the do form (but not the let) is caught by the pair
           diverging.")
  (input
    (do
      (effect Ask (op ask (-> Int64 Int64)))
      (def
        (run (: u Int64))
        (handle Ask 0 ((ask (n) s (resume (* n 2) s))) (let ((v (+ u 2))) (+ (Ask.ask v) v))))
      (def (main) (run 5))
      (export main)))
  (output (: 21 Int64)))

(case
  "a do-def shared across BOTH resume slots stays in scope (the accumulator-arm shape)"
  (doc
    "The RESUME-arg companion of the #21 perform-arg pins above (v-effects 500e59d51 — the multi-use
           residue of the do→let normalization e49c698a1). A handler arm's leading `(def s2 …)` referenced
           in BOTH resume operands — the value arg AND the next-state arg — was CDZ0101 'unbound' in a LIVE
           handler: `peel_resume_from_arm_body` wrapped only the resume VALUE in the leading do-defs and
           returned the next-state BARE, so a do-def feeding both slots orphaned. The fix wraps BOTH slots
           in the leading defs (mirroring the let/match peels — why the let-form below always worked). The
           natural accumulator arm: compute the new state once, resume the derived value + the state.
           `(note (v) s (do (def s2 (List.push s v)) (resume (List.len s2) s2)))` — main(5): note 5 →
           s2=[5], resume len 1 + state [5]; note 20 → s2=[5,20], resume len 2; (1*10 + 2) = 12. All
           backends.")
  (input
    (do
      (effect L (op note (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          L
          #list()
          ((note (v) s (do (def s2 (List.push s v)) (resume (List.len s2) s2))))
          (+ (* (L.note n) 10) (L.note 20))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

(case
  "the let-form of the dual-resume-slot arm computes (the always-worked oracle twin)"
  (doc
    "The let-twin of the do-def dual-resume-slot pin above — semantically identical with `s2`
           let-bound. This ALWAYS compiled (the let rebuilt its scope so both resume operands saw the
           binding); it's the reference 500e59d51 normalized the do-form to match. main(5) → 12, same as
           the do-form. Pinned as the regression twin so a future peel change that re-breaks the do form
           (but not the let) is caught by the pair diverging. All backends.")
  (input
    (do
      (effect L (op note (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          L
          #list()
          ((note (v) s (let ((s2 (List.push s v))) (resume (List.len s2) s2))))
          (+ (* (L.note n) 10) (L.note 20))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

(case
  "a scalar do-def shared across both resume slots in a state-advancing add handler"
  (doc
    "The scalar dual-slot fix over a STATE-ADVANCING handler: the arm `(add (v) s (do (def d (+ s v))
           (resume d d)))` shares the do-def `d` as BOTH the resume value AND the next-state, and the state
           advances by the accumulated `d` each dispatch. `(+ (St.add n) (St.add 1))` seeded 0, go 5: first
           `add 5` reads state 0, d = 0+5 = 5, resumes 5 and threads state 5; then `add 1` reads state 5,
           d = 5+1 = 6, resumes 6; 5 + 6 = 11. Pins the cross-slot peel wraps the do-def into BOTH operands
           even when the handler threads a non-trivial next-state.")
  (input
    (do
      (effect St (op add (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle St 0 ((add (v) s (do (def d (+ s v)) (resume d d)))) (+ (St.add n) (St.add 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a SCALAR do-def resumed in both slots stays in scope (scalar twin of the dual-slot fix)"
  (doc
    "The scalar twin of the dual-resume-slot fix: a scalar `(def d (+ s v))` resumed as BOTH the
           value and the next-state — `(resume d d)`. Before 500e59d51 this was CDZ0101 (the bare
           next-state slot orphaned `d`); now it stays in scope. `handle L 0`, main(5): note 5 → d=5,
           resume value 5 + state 5; note 20 → d=25, resume 25; (5*10 + 25) = 75. Confirms the fix covers
           the scalar shape, not just heap payloads. All backends.")
  (input
    (do
      (effect L (op note (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          L
          0
          ((note (v) s (do (def d (+ s v)) (resume d d))))
          (+ (* (L.note n) 10) (L.note 20))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 75 Int64)))

(case
  "a do-def referenced twice WITHIN one resume operand compiles (the within-slot control)"
  (doc
    "The discriminator control (breaker #24 perimeter): multi-reference of a do-def WITHIN a single
           resume operand `(resume (+ d d) s)` ALWAYS compiled — the break was STRICTLY CROSS-slot (a
           shared def spanning the value-arg AND state-arg), because the two operands were lowered as
           separate scopes and only the value arg carried the leading defs. This pins that within-slot
           multi-reference is not the bug: `(def d (+ v 1))` used as `(+ d d)` in the value slot, state
           `s` bare. main(5): note 5 → d=6, resume (6+6)=12 + state 0; note 20 → d=21, resume 42; (12*10 +
           42) = 162. All backends. Triangulates the fix to the cross-slot peel, not do-defs in general.")
  (input
    (do
      (effect L (op note (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          L
          0
          ((note (v) s (do (def d (+ v 1)) (resume (+ d d) s))))
          (+ (* (L.note n) 10) (L.note 20))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 162 Int64)))

(case
  "an abortive perform in a body tail referencing a do-local binding stays in scope"
  (doc
    "The abortive companion of the resuming do-def-in-perform-arg pin above (v-effects 0d382e3f4 —
           a SEPARATE bug from the resuming do→let fix e49c698a1, which is why the let form CDZ0101'd
           identically before this fix). On abort, `reduce_handle` collapsed the handle to the abort value
           and DISCARDED the body's binding scope, so an abort value referencing a body-local `(def v e)`
           orphaned it → CDZ0101 unbound. The fix re-wraps the abort value in its bindings when the body
           fires an abort. `run 5`: v = u+2 = 7, `(Bail.bail v)` abandons the computation → the handle's
           value is 7. Both backends.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (run (: u Int64)) (handle Bail 0 ((bail (n) s n)) (do (def v (+ u 2)) (Bail.bail v))))
      (def (main) (run 5))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abortive perform in a STRICT OPERAND referencing a let-local binding stays in scope"
  (doc
    "The strict-operand face of the abortive scope fix (v-effects 0d382e3f4) — the row that CDZ0101'd
           on BOTH the do and let forms before the fix, proving it independent of the resuming do→let
           normalization. The abort perform sits in a strict `+` operand referencing a body-local `let`
           binding: `(let ((v (+ u 2))) (+ (Bail.bail v) 100))`. The abort abandons before the `+`, so the
           `+ 100` never runs; the handle value is the abort value 7. `run 5` → 7. Both backends. Pinned
           beside the resuming pair so the full do-def/abort-in-perform matrix (resuming e49c698a1 +
           abortive 0d382e3f4) has durable corpus coverage.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (run (: u Int64))
        (handle Bail 0 ((bail (n) s n)) (let ((v (+ u 2))) (+ (Bail.bail v) 100))))
      (def (main) (run 5))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abortive perform in a let body tail referencing the let binding stays in scope"
  (doc
    "The tail-position let face of the abortive body-local scope fix: the abort is the let body's
           TAIL and its argument reads the let binding — `(let ((v (+ u 2))) (Bail.bail v))`. The abort
           abandons the computation and the handle's value is the arm value; the let binding must stay in
           scope for the abort perform's argument. `run 5`: v = 7, `(Bail.bail 7)` → 7.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (run (: u Int64)) (handle Bail 0 ((bail (n) s n)) (let ((v (+ u 2))) (Bail.bail v))))
      (def (main) (run 5))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abortive perform of a bare parameter argument abandons the surrounding operator"
  (doc
    "The bare-parameter control of the abortive body-local family (no body-local binding — the
           always-worked baseline): `(+ (Bail.bail u) 100)` performs the abort with `run`'s parameter `u`
           directly, and the abort abandons the enclosing `+ 100`, so the handle is the arm value `u`.
           `run 5` → 5. Pins that the body-local scope fix does not perturb the bare-param baseline.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (run (: u Int64)) (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail u) 100)))
      (def (main) (run 5))
      (export main)))
  (output (: 5 Int64)))

(case
  "a runtime condition selects an abortive branch reading an enclosing parameter"
  (doc
    "The branch-tail abort with a RUNTIME condition over an enclosing parameter — the shape a
           validation routine takes: `(handle Bail 0 ((bail (n) s n)) (if (< x 5) (Bail.bail 7) x))`. The
           `if` is the handle's value, so an abort in a branch tail is local to that branch (yields the arm
           value); the other branch reads the parameter `x` and falls through. Called with `x = 9` (not <
           5), the false branch yields `x` = 9 — no abort. This composes the branch-tail abortive fold with
           a free parameter reference and a runtime condition (`DESIGN-effects-rcdzc.md` §4.2).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main (: x Int64)) (handle Bail 0 ((bail (n) s n)) (if (< x 5) (Bail.bail 7) x)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "an abortive perform under a non-tail conditional abandons the enclosing computation"
  (doc
    "The abortive early-exit from MID-EXPRESSION, not just a tail branch. `(+ 100 (if (< x 5)
           (Bail.bail 7) 50))` — the abort is a strict OPERAND of `+`, not the handle's tail. Because an
           abort ABANDONS the enclosing computation, the surrounding `+ 100` is dead on the aborting path,
           so the expression is equivalent to `(if (< x 5) (Bail.bail 7) (+ 100 50))`: distributing the
           pure enclosing op into both branches lifts the abort to a branch tail (value-preserving because
           the sibling operand `100` is pure). Called with `x = 3` (< 5) the abort fires, discarding the
           `+ 100` → 7; with `x = 9` the false branch runs → `100 + 50` = 150. This is the 'validate an
           argument, bail out of the whole computation on failure' shape (`DESIGN-effects-rcdzc.md` §4.2).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main (: x Int64))
        (handle Bail 0 ((bail (n) s n)) (+ 100 (if (< x 5) (Bail.bail 7) 50))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 7 Int64)))

(case
  "an abortive perform in an if condition abandons the computation before branching"
  (doc
    "The abort sits in the `if` CONDITION — `(if (< (Bail.bail 7) 5) 1 2)` — which is evaluated
           FIRST, before either branch is chosen. Because an abort ABANDONS the enclosing computation, the
           `if` never branches: the whole handle yields the arm value 7, regardless of which branch the
           condition would have selected. Contrast an abort in a branch TAIL (local to that branch): a
           condition abort is unconditional (the condition always runs). Both the abort arm value and the
           `if` result type are Int64 — the handle body types compatibly. Pins that the abortive fold's
           type-consistency check compares by COMPATIBILITY (an undetermined `Int` agrees with `Int64`), not
           structural equality (`DESIGN-effects-rcdzc.md` §4.2).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (if (< (Bail.bail 7) 5) 1 2)))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abortive perform in a short-circuit connective's right operand abandons the computation"
  (doc
    "A short-circuit connective is a conditional in disguise — `(and lhs rhs)` evaluates `rhs` only
           when `lhs` is true — so an abort in the right operand is a conditional abort, equivalent to
           `(if lhs rhs false)`. `(and (< x 5) (Bail.bail 7))`: when `x < 5` the right operand runs and the
           abort fires, abandoning the computation and yielding the arm value; when `x >= 5` the connective
           short-circuits to false without performing. Here `Bail.bail : Int64 -> Bool` and the arm yields a
           Bool (`(< n 100)`), so the abort value is Bool — consistent with the connective's Bool result.
           Called with `x = 3` (< 5) the abort fires → `(< 7 100)` = true. Witnesses that the abortive fold
           reaches a short-circuit operand by desugaring it to the `if` form (`DESIGN-effects-rcdzc.md`
           §4.2).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Bool)))
      (def
        (main (: x Int64))
        (handle Bail false ((bail (n) s (< n 100))) (and (< x 5) (Bail.bail 7))))
      (export main)))
  (call main (: 3 Int64))
  (output (: true Bool)))

(case
  "a constant-lhs and whose right operand aborts desugars and folds to the arm value"
  (doc
    "The constant-condition fold of the short-circuit-operand abort (the runtime companion is above):
           `(and true (Bail.bail 7))` with `Bail.bail : Int64 -> Bool` and arm `(< n 0)` — the constant-true
           left selects the right operand, which aborts, so the handle folds to the arm value `(< 7 0)` =
           false. Pins the const-fold path of the connective-right-operand abort desugar.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Bool)))
      (def (main) (handle Bail true ((bail (n) s (< n 0))) (and true (Bail.bail 7))))
      (export main)))
  (output (: false Bool)))

(case
  "a non-tail conditional abort hoists out of a strict operand and folds per branch"
  (doc
    "An abortive perform under a NON-tail conditional — `(+ 1 (if c (Bail.bail 7) 0))` — is lifted by
           distributing the enclosing strict `+` into both branches: `(if c (+ 1 (Bail.bail 7)) (+ 1 0))`.
           The abort then sits in a branch tail, so the true branch yields the arm value 7 (the abort
           abandons the `+ 1`); the false branch is `(+ 1 0)` = 1. Sound because the sibling `1` is pure.
           Two constant-condition cases pin both branches.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 99 ((bail (n) s n)) (+ 1 (if true (Bail.bail 7) 0))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a non-tail conditional abort in the untaken branch folds to the pure sibling sum"
  (doc
    "The false-branch companion of the non-tail conditional abort hoist: `(+ 1 (if false (Bail.bail 7)
           0))` distributes to `(if false (+ 1 (Bail.bail 7)) (+ 1 0))`; the false branch is taken, no abort
           fires, and the handle folds to `(+ 1 0)` = 1. Pins that the hoist leaves the non-aborting branch
           computing its ordinary value.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 99 ((bail (n) s n)) (+ 1 (if false (Bail.bail 7) 0))))
      (export main)))
  (output (: 1 Int64)))

(case
  "an abortive perform beside an effectful sibling under a nested handle folds via inner pre-reduction"
  (doc
    "The abortive hoist requires PURE siblings, but here the sibling `(Get.get 0)` is effectful — under
           a NESTED inner `Get` handle, beneath an abortive outer `Bail`. `reduce_handle` PRE-REDUCES the
           inner `Get` handle first, folding `(Get.get 0)` to the constant 5; the body becomes `(+ 5 (if true
           (Bail.bail 7) 50))`, a pure sibling 5 alongside the conditional abort, which hoists to `(if true
           (+ 5 (Bail.bail 7)) (+ 5 50))`. The abort homes to `Bail` (arm value 7), and `Get.get` runs
           exactly once (folded before the abort). Pins the inner-handle pre-reduction that unblocks the
           hoist without a miscompile.")
  (input
    (do
      (effect Get (op get (-> Int64 Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          Bail
          0
          ((bail (n) s n))
          (handle Get 0 ((get (n) s (resume 5 s))) (+ (Get.get 0) (if true (Bail.bail 7) 50)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a perform SHORT-CIRCUITED out of an or's right operand is NOT executed (empty host-calls)"
  (doc
    "The soundness half of the short-circuit connective: when the LEFT operand short-circuits, the
           RIGHT operand's perform MUST NOT run — short-circuit evaluation elides it. `(or (> 5 3)
           (> (Amb.flip) 0))` — the left `(> 5 3)` is true, so `or` short-circuits to true and the right
           operand `(> (Amb.flip) 0)` never evaluates, so `Amb.flip` is never performed. With `Amb`
           HOST-DELEGATED, that elision is OBSERVABLE: the run makes NO host call. The empty `(host-calls)`
           fixture pins it — a perform in a skipped operand produces no observable effect. (Contrast the
           existing right-operand-RUNS case where the left selects the right; here the left short-circuits
           past it.)")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (host (Amb) (or (> 5 3) (> (Amb.flip) 0))))
      (export main)))
  (output (: true Bool))
  (host-calls))

(case
  "an abortive perform in a conditional let binding abandons the computation"
  (doc
    "The abortive early-exit from a `let` INITIALIZER — the 'bind the validated value or bail' shape.
           `(let ((k (if (< x 5) (Bail.bail 7) 0))) (+ 1 k))`: the binding's init aborts when `x < 5`. An
           init is a non-tail position (its value feeds `k`), but an abort ABANDONS the computation, so the
           `if` lifts out of the `let` — `(if (< x 5) (Bail.bail 7) (let ((k 0)) (+ 1 k)))` — value-
           preserving because the condition (and any earlier binding) is pure. Called with `x = 9` (not <
           5), the false branch binds `k = 0` and returns `1 + 0` = 1; with `x = 3` the abort fires,
           discarding the binding and the body, yielding the arm value (`DESIGN-effects-rcdzc.md` §4.2).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main (: x Int64))
        (handle Bail 0 ((bail (n) s n)) (let ((k (if (< x 5) (Bail.bail 7) 0))) (+ 1 k))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 1 Int64)))

(case
  "a handler arm that resumes NON-tail folds when the perform is the whole body"
  (doc
    "The GENERAL one-shot arm — a `resume` NOT in tail position, so the arm does work AFTER resuming
           (`(Amb.flip (u) s (+ 1 (resume 10 s)))` adds 1 to whatever the continuation returns). This is
           the powerful case (capabilities-and-effects.md #A Handler May Resume Anywhere). Its full form
           needs a reified continuation, but when the performed operation is the WHOLE handle body its
           continuation is the IDENTITY (nothing runs after the perform), so `(resume 10 s)` yields 10 in
           place and the arm evaluates to `(+ 1 10)` = 11 — no continuation object needed. Witnesses the
           identity-continuation sliver of the general-resume class; a non-tail resume whose perform sits
           inside a larger expression (a non-identity continuation) still awaits the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (Amb.flip)))
      (export main)))
  (output (: 11 Int64)))

(case
  "a handler arm consumes its resume value through an effect-free helper call"
  (doc
    "The arm's work AFTER resuming may be a call to a NON-RECURSIVE, effect-free USER function, not
           only a primitive: `(dbl (resume 10 s))` where `dbl x = x*2`. The perform's continuation is the
           handle body `C = (+ 1 [])`, so `(resume 10 s)` yields `C[10]` = 11, and the arm evaluates to
           `(dbl 11)` = 22. The helper is applied to the continuation RESULT in the arm body (distinct from
           an effect-free call INSIDE the continuation `C`); it runs once per resume, effect-free, so no
           reified continuation is needed.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (dbl (: n Int64)) (* n 2))
      (def (main) (handle Amb 0 ((flip (u) s (dbl (resume 10 s)))) (+ 1 (Amb.flip))))
      (export main)))
  (output (: 22 Int64)))

(case
  "a handler arm computes its RESUME VALUE with a deeply RECURSIVE pure helper"
  (doc
    "The recursive upgrade of the effect-free-helper arm (the `dbl` case above is explicitly
           non-recursive): the arm's resume value is `(fib s)` — a doubly-recursive pure function run on
           the handler STATE inside the arm — so the arm's evaluation nests an unbounded pure recursion
           between the perform and the resume. Seeded 10, `fib 10` = 55 resumes to the body. Pins that a
           handler arm may run arbitrary recursive computation to produce its resume value (the arm is an
           ordinary expression context, not a restricted position).")
  (input
    (do
      (effect Fib (op get (-> Unit Int64)))
      (def (fib (: n Int64)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
      (def (main (: n Int64)) (handle Fib n ((get (u) s (resume (fib s) s))) (Fib.get unit)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 55 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "a handler arm computes its NEXT-STATE with a recursive pure helper"
  (doc
    "The next-state twin: the arm threads `(double-up s 2)` — a tail-recursive helper quadrupling the
           state — as its NEXT-STATE argument. Seeded 1: the first `next` reads 1 and threads `double-up 1
           2` = 4; the second reads 4. `(do (Tw.next) (Tw.next))` = 4. Pins that the resume's SECOND
           argument (the state advance) may be an arbitrary recursive computation over the current state,
           not only a primitive step like `(+ s 1)`.")
  (input
    (do
      (effect Tw (op next (-> Unit Int64)))
      (def (double-up (: n Int64) (: k Int64)) (if (= k 0) n (double-up (* n 2) (- k 1))))
      (def
        (main (: n Int64))
        (handle Tw n ((next (u) s (resume s (double-up s 2)))) (do (Tw.next unit) (Tw.next unit))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 4 Int64)))

(case
  "a handler arm that resumes NON-tail folds through a PURE one-hole continuation"
  (doc
    "The general one-shot arm generalizes past the identity-continuation sliver: when the performed
           operation sits inside a larger PURE expression its delimited continuation is a pure one-hole
           context `C = body[perform := []]` (capabilities-and-effects.md #A Handler May Resume Anywhere).
           Here the body is `(+ 100 (Amb.flip))`, so `C = (+ 100 [])` — effect-free — and `(resume 10 s)`
           returns into it, yielding `C[10] = (+ 100 10)`. The arm `(+ 1 (resume 10 s))` then evaluates to
           `(+ 1 (+ 100 10))` = 111. No reified continuation object is needed while `C` is pure (it may even
           be duplicated by a multi-shot resume with no effect change); a perform in a conditional BRANCH — a
           non-uniform continuation — still awaits the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 100 (Amb.flip))))
      (export main)))
  (output (: 111 Int64)))

(case
  "a one-shot handler arm folds a body with TWO performs by re-reducing the continuation"
  (doc
    "The general one-shot arm extends past a single hole to a body with SEVERAL discharged performs,
           when the arm resumes EXACTLY ONCE. In a DEEP handler `resume v s'` returns into the continuation
           `C[v]` with the handler STILL ACTIVE, so a further perform in `C[v]` is handled too — the resume
           re-reduces the continuation: `resume v s' = handle(s', arms, C[v])`. Here the body is
           `(+ (Amb.flip) (Amb.flip))`: the leading flip has continuation `C = (+ [] (Amb.flip))`;
           `(resume 10 s)` re-reduces `C[10] = (+ 10 (Amb.flip))`, itself a pure one-hole context that folds
           to `(+ 1 (+ 10 10))` = 21; the outer arm `(+ 1 (resume 10 s))` then evaluates to `(+ 1 21)` = 22.
           Each re-reduction removes one perform, so it terminates. Because the arm resumes ONCE, the
           continuation is spliced once — no effect is duplicated, so no reified continuation is needed. A
           MULTI-shot arm over a performing continuation still awaits the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ (Amb.flip) (Amb.flip))))
      (export main)))
  (output (: 22 Int64)))

(case
  "a THREE-hole one-shot handle body folds via the recursing refold"
  (doc
    "Three performs on a strict spine under a ONE-SHOT non-tail arm `(+ 1 (resume 10 s))` fold by the
           refold recursing once per perform. Each flip resumes 10 into `C = (+ 1 □)`: `(+ (Amb.flip) (+
           (Amb.flip) (Amb.flip)))` = 1 + (1 + (1 + 30))... each flip = 10 and the outer arm adds 1 per level
           → 33. Each refold removes one perform, so it terminates.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ (Amb.flip) (+ (Amb.flip) (Amb.flip)))))
      (export main)))
  (output (: 33 Int64)))

(case
  "a foreign-performing op ARGUMENT bound to a multiply-used arm param runs the effect exactly once"
  (doc
    "OP-ARG LET-LIFT: an operation whose ARGUMENT carries a FOREIGN perform, bound to an arm param the
           arm uses MORE THAN ONCE, folds by binding the arg to a fresh let ONCE and duplicating the pure
           reference, so the foreign effect runs EXACTLY once. `Add.sum`'s arm reads `(. p 0)` and `(. p 1)`;
           the argument `(tuple (Ask.get) (Ask.get))` carries two `Ask` gets foreign to `Add`. The op arg is
           evaluated once (seeded 10 → 10 then 11, tuple (10,11)), then the arm reads that ONE tuple twice →
           `(+ (* 10 100) 11)` = 1011. A naive β-copy would run four gets (wrong); this pins run-once.")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (effect Add (op sum (-> (Tuple Int64 Int64) Int64)))
      (def
        (main)
        (handle
          Ask
          10
          ((get (u) s (resume s (+ s 1))))
          (handle
            Add
            0
            ((sum (p) s (resume (+ (* (. p 0) 100) (. p 1)) s)))
            (Add.sum #tuple((Ask.get) (Ask.get))))))
      (export main)))
  (output (: 1011 Int64)))

(case
  "a NON-tail one-shot arm that ADVANCES the state threads the advance through the re-reduced continuation"
  (doc
    "The two-perform re-reducing fold above holds the state CONSTANT (`(resume 10 s)`); this pins the
           sharper composition where the arm is BOTH non-tail (work wraps the resume) AND state-advancing.
           Arm `(tick (u) s (+ 100 (resume s (+ s 1))))` resumes with the CURRENT state `s` and threads
           `s+1` forward, over the body `(+ (St.tick) (St.tick))`, seeded 0. The leading tick's continuation
           `C = (+ [] (St.tick))`; `(resume 0 1)` re-reduces `C[0] = (+ 0 (St.tick))` under state 1 — the
           inner tick reads 1, resumes `(resume 1 2)` into its own continuation `(+ 0 [])` = 1, its arm
           yields `(+ 100 1)` = 101, so `C[0]` = `(+ 0 101)` = 101; the outer arm then yields `(+ 100 101)`
           = 201. Pins that the `(+ s 1)` advance survives EACH continuation re-reduction — a fold that
           dropped the advance would resume the second tick with 0 too and compute 200, not 201. The state
           threads 0->1->2 across the nested re-reductions while every resume is wrapped by pure work.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def (main) (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (+ (St.tick) (St.tick))))
      (export main)))
  (output (: 201 Int64)))

(case
  "a NON-tail state-advancing arm threads through a NON-commutative continuation"
  (doc
    "The non-commutative companion of the case above — it pins BOTH the continuation nesting AND the
           left-to-right state advance at once, since a fold that dropped the advance lands on a different
           value. Same arm `(tick (u) s (+ 100 (resume s (+ s 1))))` seeded 0, but the body subtracts:
           `(- (St.tick) (St.tick))`. The leading tick's continuation `C = (- [] (St.tick))`; `(resume 0 1)`
           re-reduces `C[0] = (- 0 (St.tick))` under state 1 — the inner tick reads 1, resumes into its own
           continuation `(- 0 [])` = -1, its arm yields `(+ 100 -1)` = 99, so `C[0]` = 99; the outer arm
           then yields `(+ 100 99)` = 199. A fold that read both ticks at the SAME state (advance dropped)
           would resume the second tick with 0 and compute 200, not 199. Both backends agree.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def (main) (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (- (St.tick) (St.tick))))
      (export main)))
  (output (: 199 Int64)))

(case
  "a NON-tail state-advancing arm threads the advance through a perform in an if CONDITION"
  (doc
    "The two cases above pin the non-tail state-advancing arm `(tick (u) s (+ 100 (resume s (+ s 1))))`
           over a FLAT operator body; this pins the same arm when the leading perform sits in an if
           CONDITION and the branch performs again — the handler-distribution seam composed with the advance.
           Body `(if (< (St.tick) 50) (+ 2000 (St.tick)) 999)`, seed 0. The condition's tick reads 0, its
           continuation `C = (if (< [] 50) (+ 2000 (St.tick)) 999)`; `(resume 0 1)` re-reduces `C[0] =
           (if (< 0 50) (+ 2000 (St.tick)) 999)` under state 1 — the guard is true, so the taken branch
           `(+ 2000 (St.tick))` runs: the inner tick reads 1 (the ADVANCED state), resumes into its own
           continuation `(+ 2000 [])` = 2001, its arm yields `(+ 100 2001)` = 2101, so `C[0]` = 2101; the
           outer arm then yields `(+ 100 2101)` = 2201. Pins that the `(+ s 1)` advance reaches the BRANCH
           tick across the condition re-reduction — a constant-state arm `(resume s s)` (advance dropped)
           would read the branch tick at 0 and compute 2200, not 2201.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          0
          ((tick (u) s (+ 100 (resume s (+ s 1)))))
          (if (< (St.tick) 50) (+ 2000 (St.tick)) 999)))
      (export main)))
  (output (: 2201 Int64)))

(case
  "a NON-tail state-advancing arm threads the advance through a perform in a let INIT"
  (doc
    "The let-init companion of the if-condition case above: the same non-tail state-advancing arm
           `(tick (u) s (+ 100 (resume s (+ s 1))))` with the leading perform as a `let` INIT whose binding
           is reused, and a SECOND perform in the let body. Body `(let ((x (St.tick))) (+ (* 1000 (+ x 1))
           (St.tick)))`, seed 0. The init tick reads 0, its continuation `C = (let ((x [])) (+ (* 1000
           (+ x 1)) (St.tick)))`; `(resume 0 1)` re-reduces `C[0]` under state 1 with `x` bound to 0 — the
           body `(+ (* 1000 (+ 0 1)) (St.tick))` = `(+ 1000 (St.tick))`: the inner tick reads 1 (advanced),
           resumes into its continuation `(+ 1000 [])` = 1001, its arm yields `(+ 100 1001)` = 1101, so
           `C[0]` = 1101; the outer arm yields `(+ 100 1101)` = 1201. Pins that the advance survives the
           let-init re-reduction AND that the bound `x` reads the PRE-advance state 0 (a fold that let `x`
           see the advanced 1 would compute `(* 1000 2)` = 2000-based, not 1000-based).")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          0
          ((tick (u) s (+ 100 (resume s (+ s 1)))))
          (let ((x (St.tick))) (+ (* 1000 (+ x 1)) (St.tick)))))
      (export main)))
  (output (: 1201 Int64)))

(case
  "a NON-tail state-advancing arm threads the advance through a perform in a match SCRUTINEE and its arm body"
  (doc
    "The match-scrutinee companion of the if-condition + let-init distribution cases above — the third
           strict-first seam. Same non-tail state-advancing arm `(tick (u) s (+ 100 (resume s (+ s 1))))`,
           body `(match (St.tick) (0 111) (_ (+ 1 (St.tick))))`, seed 5. The scrutinee tick reads 5, its
           continuation `C = (match [] (0 111) (_ (+ 1 (St.tick))))`; `(resume 5 6)` re-reduces `C[5]` under
           state 6 — 5 is not the `0` literal so the `_` arm `(+ 1 (St.tick))` runs: the inner tick reads 6
           (the ADVANCED state), resumes into its own continuation `(+ 1 [])` = 7, its arm yields
           `(+ 100 7)` = 107, so `C[5]` = 107; the outer arm then yields `(+ 100 107)` = 207. Pins that the
           `(+ s 1)` advance reaches the arm-body tick across the scrutinee re-reduction — a constant-state
           arm `(resume s s)` (advance dropped) would read the arm-body tick at 5 and compute 206, not 207.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          5
          ((tick (u) s (+ 100 (resume s (+ s 1)))))
          (match (St.tick) (0 111) (_ (+ 1 (St.tick))))))
      (export main)))
  (output (: 207 Int64)))

(case
  "a NON-tail state-advancing arm threads the advance through a perform in a matched literal arm reached via the scrutinee"
  (doc
    "The literal-arm face of the match-scrutinee case above: the scrutinee dispatch selects a SCALAR
           LITERAL arm that itself performs (not the wildcard). Same arm `(tick (u) s (+ 100 (resume s
           (+ s 1))))`, body `(match (St.tick) (0 (+ 7 (St.tick))) (_ 222))`, seed 0. The scrutinee tick
           reads 0, `C = (match [] (0 (+ 7 (St.tick))) (_ 222))`; `(resume 0 1)` re-reduces `C[0]` under
           state 1 — 0 matches the `0` literal arm `(+ 7 (St.tick))`: the inner tick reads 1 (advanced),
           resumes into `(+ 7 [])` = 8, its arm yields `(+ 100 8)` = 108, so `C[0]` = 108; the outer arm
           yields `(+ 100 108)` = 208. Pins the advance reaching a performing LITERAL arm (not just the
           wildcard) selected by the re-reduced scrutinee.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          0
          ((tick (u) s (+ 100 (resume s (+ s 1)))))
          (match (St.tick) (0 (+ 7 (St.tick))) (_ 222))))
      (export main)))
  (output (: 208 Int64)))

(case
  "a MULTI-shot handler arm folds a two-hole body by re-reducing per resume"
  (doc
    "The re-reducing fold extends to a MULTI-shot arm — one that resumes more than once — when the
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
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Amb.flip) (Amb.flip))))
      (export main)))
  (output (: 12 Int64)))

(case
  "a MULTI-shot arm whose FIRST resume value is chosen by an if on the state"
  (doc
    "Composes multi-shot resumption with a conditional-resume value: the arm resumes TWICE, and the
           FIRST resume's value is chosen by an `if` on the handler state. `(flip (u) s (+ (resume (if (> s
           2) 10 20) s) (resume 1 s)))` over the body `(+ 100 (Amb.flip))` — the pure one-hole continuation
           `C = (+ 100 [])` is spliced per resume: seeded 3, `(> 3 2)` holds so the first resume value is 10
           → `C[10]` = 110, and the second is 1 → `C[1]` = 101, so the arm yields `(+ 110 101)` = 211. Pins
           that a multi-shot arm's per-resume continuation splice composes with a resume value COMPUTED by a
           branch on the state — each resumption independently folds `C` at its own (state-derived) value.
           Both backends agree.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          3
          ((flip (u) s (+ (resume (if (> s 2) 10 20) s) (resume 1 s))))
          (+ 100 (Amb.flip))))
      (export main)))
  (output (: 211 Int64)))

(case
  "a MULTI-shot arm folds a perform wrapped in an inline lambda application"
  (doc
    "A perform WRAPPED IN A LAMBDA APPLICATION folds under a multi-shot arm. `((fn (x) (+ x (Amb.flip)))
           100)` is a β-redex: applying the lambda substitutes `x := 100`, giving `(+ 100 (Amb.flip))` — a
           single perform in a pure one-hole context `C = (+ 100 [])`. The fold PRE-REDUCES applied-lambda
           redexes before classifying (`reduce_applied_lambdas`), so the multi-shot path serves it exactly as
           the reduced body: the arm `(+ (resume 1 s) (resume 2 s))` yields `(+ (+ 100 1) (+ 100 2))` = 203.
           (The one-shot/threading path already inlines such a call via its cross-function inline arm; this
           extends the same β-reduction to the multi-shot pure-one-hole path.) A lambda VALUE is pure at
           construction — its body's effects fire only when APPLIED — so duplicating the reduced context per
           resumption duplicates no closure effect.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) ((fn (x) (+ x (Amb.flip))) 100)))
      (export main)))
  (output (: 203 Int64)))

(case
  "a MULTI-shot arm folds a perform in a let-bound lambda applied in the body"
  (doc
    "The let-bound form of the preceding case, composing the applied-lambda pre-reduction with the
           lambda-value-is-pure purity rule. `(let ((f (fn (x) (+ x (Amb.flip))))) (f 100))` binds a
           performing lambda (pure at construction) and applies it; pre-reduction β-reduces `(f 100)` to
           `(+ 100 (Amb.flip))`, leaving the now-unused binding whose lambda init is strongly pure (the
           purity walk does not descend a lambda body). `C = (+ 100 [])` under the multi-shot arm yields
           `(+ (+ 100 1) (+ 100 2))` = 203. Pins that a let-bound performing lambda folds under a multi-shot
           resume, not only a one-shot one.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (+ (resume 1 s) (resume 2 s))))
          (let ((f (fn (x) (+ x (Amb.flip))))) (f 100))))
      (export main)))
  (output (: 203 Int64)))

(case
  "a multi-shot ctl arm whose continuation reads an ENCLOSING-fn param folds"
  (doc
    "A multi-shot E5 within-activation arm `(pick (u) s k (+ (k 1) (k 2)))` — `k` (the reified
           delimited continuation) applied TWICE — over a handle body `(let ((y 3)) (+ n (Amb.pick)))` whose
           continuation `C = (+ n [])` reads an ENCLOSING function param `n`. The fold splices a FRESH copy
           of `C` per `k`-application (2 copies), so `C[1]` = `(+ n 1)` and `C[2]` = `(+ n 2)`, and the arm
           yields `(+ (+ n 1) (+ n 2))`; with `n = 5` that is `(+ 6 7)` = 13. Pins that the per-resume splice
           PRESERVES `C`'s enclosing captures: without pinning `n` before the splice each copy re-resolves it
           against its own orphan and reports a false CDZ0101 'unbound n' (breaker mv-class). The arm's own
           `k`/state binders and the body-local `let` binder `y` are unaffected — only the enclosing capture
           needed pinning. Both backends agree.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle Amb 0 ((pick (u) s k (+ (k 1) (k 2)))) (let ((y 3)) (+ n (Amb.pick)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 13 Int64)))

(case
  "a ONE-shot ctl arm whose continuation reads an enclosing-fn param folds (mv single-splice control)"
  (doc
    "The single-resume control for the multi-shot enclosing-capture case: the SAME body
           `(let ((y 3)) (+ n (Amb.pick)))` but a ONE-shot arm `(pick (u) s k (k 1))` — `k` applied once, so
           `C = (+ n [])` is spliced a SINGLE time → `C[1]` = `(+ n 1)`; with `n = 5` that is `6`. A single
           splice never needed the capture pin (one copy, one resolution), so this held before the mv-class
           fix; it stays green after, confirming the fix does not disturb the single-splice path.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def (main (: n Int64)) (handle Amb 0 ((pick (u) s k (k 1))) (let ((y 3)) (+ n (Amb.pick)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a multi-shot ctl arm reading an enclosing param with NO let frame folds (mv no-let control)"
  (doc
    "The no-let control: the multi-shot arm `(pick (u) s k (+ (k 1) (k 2)))` over a body with NO
           intervening `let` — `(+ n (Amb.pick))` directly — so `C = (+ n [])` reading the enclosing param
           `n`. Isolates that the fix is about preserving the enclosing capture `n` across the per-resume
           splice, independent of a body-local binding frame: `(+ (+ n 1) (+ n 2))` with `n = 5` = 13,
           matching the let-wrapped case.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def (main (: n Int64)) (handle Amb 0 ((pick (u) s k (+ (k 1) (k 2)))) (+ n (Amb.pick))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 13 Int64)))

(case
  "a two-site resume arm whose resume VALUE reads an enclosing-fn param folds"
  (doc
    "The ARM-SIDE enclosing-capture face (breaker mv-class, distinct from the continuation-C face): a
           two-site resume arm `(+ (resume (+ n 1) s) (resume 2 s))` whose FIRST resume VALUE `(+ n 1)` reads
           an ENCLOSING function param `n`. The arm body is β-substituted then its resume occurrences rewrite
           to `C[value]` per site — so the resume VALUE (carrying free `n`) is copied per resume. Without
           pinning `n` in the arm body BEFORE the β-substitution (which detaches it — the copied `n` loses
           its binder), each per-site copy re-resolves `n` unbound → false CDZ0101. Here `C = (let ((x []))
           (+ (* 10 x) x))`: resume 1 with `(+ n 1)` = 6 → `x=6` → 66, resume 2 with 2 → `x=2` → 22, arm =
           `(+ 66 22)` = 88 (n = 5). Pins the resume-value enclosing-capture preservation across the
           multi-site splice.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
          (let ((x (Amb.pick))) (+ (* 10 x) x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 88 Int64)))

(case
  "a two-site resume arm reading the enclosing SEED param in its resume value folds"
  (doc
    "The SEED face of the arm-side enclosing capture: the handle is seeded by the enclosing param
           `(handle Amb n …)`, and the arm's first resume value `(+ s 1)` reads the state `s` (= the seed on
           first entry). The seed `n` reaches the arm via the state-binder substitution, so it appears in the
           β-substituted arm body (not the original) — pinned there so the per-site splice shares it. `C =
           (let ((x [])) (+ (* 10 x) x))`, seed 5: resume 1 value `(+ s 1)` = 6 → 66, resume 2 value 2 → 22,
           arm = 88.")
  (input
    (do
      (effect Amb (op pick (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          n
          ((pick (u) s (+ (resume (+ s 1) s) (resume 2 s))))
          (let ((x (Amb.pick))) (+ (* 10 x) x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 88 Int64)))

(case
  "a two-site resume arm whose resume value is a heap LIST reading an enclosing param folds"
  (doc
    "The heap-payload variant of the arm-side enclosing-capture face: the resume value is a `(list n 2
           9)` reading the enclosing param `n`, so a multi-node list payload carrying an enclosing capture
           crosses the per-site splice. `C = (let ((xs [])) (List.len xs))`: resume 1 value `(list n 2 9)` =
           a 3-element list → len 3, resume 2 value `(list 7)` → len 1, arm = `(+ 3 1)` = 4. Confirms the
           enclosing-capture pin works when the resume value is a heap constructor, not just a scalar.")
  (input
    (do
      (effect Amb (op pick (-> Unit (List Int64))))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((pick (u) s (+ (resume #list(n 2 9) s) (resume #list(7) s))))
          (let ((xs (Amb.pick))) (List.len xs))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4 Int64)))

(case
  "a MULTI-shot arm keeps a pure applied lambda in its duplicated continuation"
  (doc
    "The soundness anchor for the lambda-value purity rule under a MULTI-shot resume: an EFFECT-FREE
           let-bound lambda `k = (fn (y) (* y 2))` is APPLIED in the continuation `C` alongside the single
           perform. `C = (+ (k 3) [])` is strongly pure — `(k 3)` re-runs an effect-free computation, and the
           lambda value itself carries no effect — so duplicating `C` per resumption is safe: `(k 3)` = 6, and
           the arm yields `(+ (+ 6 1) (+ 6 2))` = 15. Confirms the purity walk skipping a lambda body does NOT
           over-admit — a performing applied lambda (a genuine second hole) still declines as non-uniform,
           while a pure applied lambda folds.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (+ (resume 1 s) (resume 2 s))))
          (let ((k (fn (y) (* y 2)))) (+ (k 3) (Amb.flip)))))
      (export main)))
  (output (: 15 Int64)))

; The multi-shot cases above duplicate a continuation containing only SCALARS or a pure lambda. These pin the
; Perceus × multi-shot intersection: a continuation that reads or CONSUMES a captured HEAP value, re-reduced
; per resumption, must give EACH resume its own valid copy — the multi-shot duplication must `dup` the
; captured heap value, not share one that the first resume frees (or FBIP-mutates in place at rc==1) out from
; under the second. A shared-and-freed heap value would use-after-free / corrupt the second resumption.
(case
  "a MULTI-shot arm re-reduces a continuation that reads a captured heap list per resume"
  (doc
    "The arm `(+ (resume 1 s) (resume 2 s))` resumes TWICE; the continuation `(+ (Amb.flip) (List.len
           xs))` reads a captured heap list `xs = [10 20 30]`. Each re-reduction must see `xs` alive:
           resume-1 → (1 + 3), resume-2 → (2 + 3), so `(+ 4 5)` = 9. Pins the captured heap value is retained
           (dup'd) across BOTH continuation re-reductions — a value freed after the first resume would make
           `List.len xs` in the second read freed memory (a wrong length / crash).")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (let
          ((xs #list(10 20 30)))
          (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Amb.flip) (List.len xs)))))
      (export main)))
  (output (: 9 Int64)))

(case
  "a MULTI-shot arm whose each resume CONSUMES a captured heap list dups it per resume"
  (doc
    "The sharper case: each resumption CONSUMES the captured `xs = [1 2]` via `List.push` (a persistent
           op that FBIP-mutates in place at rc==1). Under multi-shot, both re-reductions consume `xs`, so the
           duplication MUST dup it — else the first resume's `List.push` grows the shared `xs` in place and
           the second resume sees `[1 2 99]` (len 4, wrong). `List.len (List.push xs 99)` = 3 each resume:
           resume-1 → (1 + 3), resume-2 → (2 + 3) → `(+ 4 5)` = 9. Pins the multi-shot duplication dups a
           CONSUMED captured heap value, the Perceus-correct multi-shot semantics.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (let
          ((xs #list(1 2)))
          (handle
            Amb
            0
            ((flip (u) s (+ (resume 1 s) (resume 2 s))))
            (+ (Amb.flip) (List.len (List.push xs 99))))))
      (export main)))
  (output (: 9 Int64)))

(case
  "multi-shot resumes carry DIVERGENT states; two performs branch 2x2"
  (doc
    "Every multi-shot pin above resumes with the SAME state; here the two resumes carry DIFFERENT
           next-states (`(+ s 10)` vs `(+ s 20)`), and a second perform re-branches each path under its
           own inherited state — a 2×2 tree where each leaf's value reflects its lineage. Per branch:
           k(v) = v + (1 + 2) under that branch's state = 2v + 3; k(1) = 5, k(2) = 7 → 12. Pins that
           each re-reduction threads ITS OWN state forward, not a shared or last-written one.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((flip (u) s (+ (resume 1 (+ s 10)) (resume 2 (+ s 20)))))
          (+ (Amb.flip) (Amb.flip))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

(case
  "each multi-shot branch OBSERVES its own divergent state via a trailing peek"
  (doc
    "The observability face of divergent multi-shot states: branch k(1) inherits state 10, branch
           k(2) inherits 20, and each branch's trailing `peek` reads its own — 10·1 + 10 = 20 and
           10·2 + 20 = 40 → 60. A shared-state implementation (both branches seeing one cell) would
           yield 50 or 70; the checksum separates the worlds.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)) (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((flip (u) s (+ (resume 1 (+ s 10)) (resume 2 (+ s 20)))) (peek (u) s (resume s s)))
          (+ (* 10 (Amb.flip)) (Amb.peek))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 60 Int64)))

(case
  "divergent multi-shot states carry HEAP lineages — each branch grows its own list"
  (doc
    "The Perceus-critical SHAPE: the two resumes push DIFFERENT elements onto the list state, so
           each branch of the 2×2 tree owns an independent heap lineage. The body is `(+ (* 10 flip₁)
           flip₂)`: the outer flip branches k(1) and k(2), and inside each, the second flip re-branches
           to (1 + 2) = 3 — so k(v) = 10v + 10v + 3 = 20v + 3; k(1) = 23, k(2) = 43 → 66. NOTE this
           case's resumed values are constants and nothing here reads the list, so its checksum alone
           cannot detect a shared in-place list — it pins the divergent-push shape compiling and
           running; the SIBLING below (per-branch `size`) is the case that OBSERVES the lineage
           separation.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          #list()
          ((flip (u) s (+ (resume 1 (List.push s 10)) (resume 2 (List.push s 20)))))
          (+ (* 10 (Amb.flip)) (Amb.flip))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 66 Int64)))

(case
  "each multi-shot branch observes its own heap-lineage length"
  (doc
    "The heap observability face: each branch's trailing `size` reads the length of ITS list —
           both branches see exactly one element (their own push), never the sibling's: 10·1 + 1 = 11
           and 10·2 + 1 = 21 → 32. A shared list would read length 2 on the second branch.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)) (op size (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          #list()
          ((flip (u) s (+ (resume 1 (List.push s 10)) (resume 2 (List.push s 20))))
            (size (u) s (resume (List.len s) s)))
          (+ (* 10 (Amb.flip)) (Amb.size))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 32 Int64)))

(case
  "divergent multi-shot STRING lineages — each branch observes its own byte-length"
  (doc
    "The rope-representation twin of the list-lineage pins above (Strings are rope-backed, a
           DIFFERENT heap representation from RRB lists): branch k(1) concats \\\"a\\\" (byte-len 1),
           k(2) concats \\\"bb\\\" (byte-len 2), and each branch's trailing `len` reads ITS OWN —
           10·1 + 1 = 11 and 10·2 + 2 = 22 → 33. A rope in-place append shared across branches would
           read 3 on the sibling; the divergence property needs its own witness per representation.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)) (op len (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          ""
          ((flip (u) s (+ (resume 1 (String.concat s "a")) (resume 2 (String.concat s "bb"))))
            (len (u) s (resume (String.byte-len s) s)))
          (+ (* 10 (Amb.flip)) (Amb.len))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 33 Int64)))

(case
  "an arm SUMS one resumption with a constant (the 1.5-shot shape)"
  (doc
    "Between single-shot and multi-shot: the arm's value mixes ONE continuation result with a
           non-continuation term — `(+ (resume 1 s) 100)`. k(1) = 1 + 5 = 6, arm value 6 + 100 = 106.
           Pins that the arm's value expression composes a resumption result with ordinary arithmetic
           (the resume is not required to be the whole arm value).")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main (: n Int64)) (handle Amb 0 ((flip (u) s (+ (resume 1 s) 100))) (+ (Amb.flip) 5)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 106 Int64)))

(case
  "an arm CONDITIONS its shot count — the multi-shot branch"
  (doc
    "Every multi-shot pin above has a STATIC shot count; here the arm chooses AT RUN TIME —
           `(if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))` — two resumptions on one branch,
           one on the other. Seed 5 takes the multi-shot branch: k(1) = 6, k(2) = 7 → 13. The
           single-shot branch of the SAME program is pinned below; a dynamically-chosen shot count is
           the real shape of backtracking search.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          n
          ((flip (u) s (if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))))
          (+ (Amb.flip) 5)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 13 Int64)))

(case
  "the conditional-shot-count arm's SINGLE-shot branch (same program, other input)"
  (doc
    "The other runtime path of the conditional-count arm above: seed 2 fails `(> s 3)`, so the
           arm resumes ONCE with 9 → 9 + 5 = 14. Together the pair pins both dynamic outcomes of one
           compiled handler — the shot count is a runtime property of the dispatch, not a static
           property of the arm.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          n
          ((flip (u) s (if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))))
          (+ (Amb.flip) 5)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 14 Int64)))

(case
  "a multi-shot continuation contains a NESTED handler — each re-reduction re-enters it fresh"
  (doc
    "The continuation being re-reduced holds a whole nested `handle In` (a separate effect,
           seed 7): each of the two re-reductions must RE-INSTANTIATE the nested frame from its seed —
           both branches read 7 (k(10) = 17, k(20) = 27 → 44), never an 8 leaked from the sibling's
           instance. Pins per-re-reduction frame re-instantiation.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (effect In (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Amb
          0
          ((flip (u) s (+ (resume 10 s) (resume 20 s))))
          (+ (Amb.flip) (handle In 7 ((get (u) t (resume t (+ t 1)))) (In.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 44 Int64)))

(case
  "a MULTI-shot continuation APPLIES a captured closure per re-reduction"
  (doc
    "The closure composition of the captured-heap multi-shot cases above: the re-reduced continuation
           `(scale (+ (Go.fork) 10))` applies `scale = (fn (x) (* x n))` — a closure over `main`'s runtime
           parameter — once per resumption. Each re-reduction must find the closure (and its env) alive:
           k(1) → scale(11) = 55, k(2) → scale(12) = 60 → 115. A closure freed or its env dropped after
           the first resume breaks the second application.")
  (input
    (do
      (effect Go (op fork (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def scale (fn ((: x Int64)) (* x n)))
          (handle Go 0 ((fork (u) s (+ (resume 1 s) (resume 2 s)))) (scale (+ (Go.fork) 10)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 115 Int64)))

(case
  "a MULTI-shot continuation PUSHES onto a captured list per re-reduction (fresh copy each)"
  (doc
    "The double-consume composition: each re-reduction runs `(List.push (List.push (list n) (Go.fork))
           7)` — TWO pushes onto a list built from the captured `n` — and reports its length. Both
           re-reductions must see a fresh 1-element base (len 3 each → 6); an FBIP in-place grow shared
           across resumes would give the second a longer list. Extends the dup-per-resume pin above from
           one consuming op to a consuming CHAIN.")
  (input
    (do
      (effect Go (op fork (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Go
          0
          ((fork (u) s (+ (resume 1 s) (resume 2 s))))
          (List.len (List.push (List.push #list(n) (Go.fork)) 7))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a MULTI-shot arm folds a perform under a CURRIED lambda applied to pure arguments"
  (doc
    "The applied-lambda pre-reduction reduces a CURRIED redex — nested applications — as long as each
           argument is pure. `(((fn (a) (fn (b) (+ a (+ b (Amb.flip))))) 10) 20)` applies the outer lambda to
           `10` (yielding the inner `(fn (b) …)`) then to `20`, β-reducing to `(+ 10 (+ 20 (Amb.flip)))` =
           `(+ 30 (Amb.flip))` — a single perform in a pure one-hole context `C = (+ 30 [])`. Both arguments
           are pure literals, so the substitution (into params each used once) duplicates no effect, and the
           reduced body folds under the multi-shot arm: `(+ (+ 30 1) (+ 30 2))` = 63. Pins that pre-reduction
           follows a curried application chain, not only a single β-redex.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (+ (resume 1 s) (resume 2 s))))
          (((fn (a) (fn (b) (+ a (+ b (Amb.flip))))) 10) 20)))
      (export main)))
  (output (: 63 Int64)))

(case
  "a pure lambda passed as an argument to a performing callee folds"
  (doc
    "A HIGHER-ORDER call whose function ARGUMENT is a pure lambda and whose CALLEE performs. `apply1 g n
           = (+ (g n) (Amb.flip))` takes a function `g` and performs; called with `g = (fn (z) (* z 2))` (an
           effect-free lambda) and `n = 10`. The argument lambda is strongly pure (a lambda VALUE carries no
           effect), so the pre-reduction inlines the call — `(g 10)` reduces to `(* 10 2)` = 20, leaving
           `(+ 20 (Amb.flip))`, a single perform in a pure one-hole context. The handler resumes 5, so the
           result is `(+ 20 5)` = 25. Pins that a pure function-valued argument does not block the fold — the
           closure is passed and applied inside the reduced body with no effect duplication.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (apply1 (: g (-> Int64 Int64)) (: n Int64)) (+ (g n) (Amb.flip)))
      (def (main) (handle Amb 0 ((flip (u) s (resume 5 s))) (apply1 (fn (z) (* z 2)) 10)))
      (export main)))
  (output (: 25 Int64)))

(case
  "an arm that binds its resume in a lambda and applies it immediately folds"
  (doc
    "An arm that names its continuation as a LAMBDA and APPLIES it in place — `(flip (u) s (let ((k (fn
           (x) (resume (* x 2) s)))) (k 5)))`. This LOOKS like the captured-continuation frontier (a `k`
           bound to the resume), but `k` does NOT escape — it is applied immediately, `(k 5)`. So the
           applied-lambda pre-reduction inlines it to `(resume (* 5 2) s)` = `(resume 10 s)`, an ORDINARY
           non-tail resume the pure one-hole fold serves: `C = (+ 100 [])` over the body `(+ 100 (Amb.flip))`,
           so the handle yields `(+ 100 10)` = 110. Pins that binding the resume in a lambda and applying it
           in-arm is NOT the hard captured-`k` case (which needs a reified continuation) — an immediately-
           applied continuation-lambda reduces away, distinguishing 'names k' from 'k escapes'.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (let ((k (fn (x) (resume (* x 2) s)))) (k 5))))
          (+ 100 (Amb.flip))))
      (export main)))
  (output (: 110 Int64)))

(case
  "an applied lambda whose body enters a mutually-recursive performing group folds"
  (doc
    "Composes the applied-lambda pre-reduction with mutual-recursion specialization: the handle body
           is `((fn (m) (ev m)) 4)`, a lambda applied to a pure literal whose body ENTERS the
           mutually-recursive performing group `ev`/`od`. Pre-reduction inlines the pure-arg redex to
           `(ev 4)`, then the mutual pair specializes under the state handler exactly as a direct `(ev 4)`
           would — the two folds compose. Seeded 7, threading `s - 1`, the ticks read 7 then 6, so `ev(4)` =
           `7 + 6 + 0` = 13. Pins that an applied lambda is a transparent wrapper over a recursive-effectful
           call, folding to the same result as the unwrapped call.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (+ (Ctr.tick) (od (- n 1)))))
      (def (od (: n Int64)) (ev (- n 1)))
      (def (main) (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) ((fn (m) (ev m)) 4)))
      (export main)))
  (output (: 13 Int64)))

(case
  "an applied lambda whose body performs an ABORTIVE op abandons the enclosing computation"
  (doc
    "Composes the applied-lambda pre-reduction with an ABORTIVE (non-resuming) handler. The body
           `(+ 100 ((fn (x) (Bail.bail x)) 42))` wraps the abortive perform in a lambda application in a
           STRICT operand position. Pre-reduction inlines the pure-arg redex to `(+ 100 (Bail.bail 42))`,
           where the abort abandons the surrounding `(+ 100 …)` — the abortive arm's value 42 becomes the
           whole handle's value (NOT 142). Pins that an abort reached through an applied-lambda wrapper still
           unwinds the enclosing strict context, the abortive analogue of the resumptive compositions above.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (+ 100 ((fn (x) (Bail.bail x)) 42))))
      (export main)))
  (output (: 42 Int64)))

(case
  "a performing argument to a multiply-using performing callee is not duplicated"
  (doc
    "The SOUNDNESS ANCHOR for the applied-lambda pre-reduction: a call is β-reduced early (before the
           pure-one-hole classifier) ONLY when its arguments are strongly PURE. Here the argument itself
           PERFORMS — `(mixed (Amb.flip))` where `mixed x = (+ x (+ x (Amb.flip)))` uses its parameter `x`
           TWICE. β-substituting the performing argument textually would run `(Amb.flip)` once PER use of
           `x` — three performs instead of two — a miscompile. Cadenza is strict (call-by-value): the
           argument evaluates EXACTLY ONCE to a value the two uses of `x` share. The pre-reduction declines
           this redex (its argument is not strongly pure) and the state-threading path binds the argument's
           single resume value once. Handler seed 0, `flip` resumes `s+1` threading `s+1`: the argument flip
           reads 0→1 (so `x` = 1, state→1), the body flip reads 1→2, giving `(+ 1 (+ 1 2))` = 4. Pins that a
           performing argument is evaluated once, never duplicated by early β-reduction.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (mixed (: x Int64)) (+ x (+ x (Amb.flip))))
      (def (main) (handle Amb 0 ((flip (u) s (resume (+ s 1) (+ s 1)))) (mixed (Amb.flip))))
      (export main)))
  (output (: 4 Int64)))

(case
  "a one-shot two-hole body folds across a let binding"
  (doc
    "The one-shot re-reducing fold descends the STRICT spine of a `let` (its inits then its body, run
           unconditionally in sequence), so a body with a perform in the let INIT and another in the let
           BODY folds. Here `(let ((x (Amb.flip))) (+ x (Amb.flip)))`: the leading flip is the INIT, with
           continuation `C = (let ((x [])) (+ x (Amb.flip)))`; `(resume 10 s)` re-reduces `C[10] = (let ((x
           10)) (+ x (Amb.flip)))` — the binding fixes `x = 10` and the body's remaining flip is a pure
           one-hole context, folding to `(+ 1 (+ 10 10))` = 21; the outer arm `(+ 1 (resume 10 s))` then
           evaluates to `(+ 1 21)` = 22. The whole `let` is copied into `C`, so its binder re-binds
           independently; one resume, so the continuation is spliced once.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (let ((x (Amb.flip))) (+ x (Amb.flip)))))
      (export main)))
  (output (: 22 Int64)))

(case
  "a one-shot two-hole body folds with the leading perform in an if condition"
  (doc
    "The one-shot re-reducing fold descends an `if` CONDITION — the strict, evaluated-first position —
           for its leading hole, and a further perform in a BRANCH is served when the re-reduced condition
           selects that branch. Here `(if (< (Amb.flip) 50) (+ 1 (Amb.flip)) 0)`: the leading flip is the
           condition, `C = (if (< [] 50) (+ 1 (Amb.flip)) 0)`; `(resume 10 s)` re-reduces `C[10] = (if (< 10
           50) (+ 1 (Amb.flip)) 0)` — the condition is now the constant `(< 10 50)` = true, so the then-branch
           is taken and its remaining flip folds (by handler distribution over the now-constant conditional):
           `(+ 1 (+ 1 10))` = 12; the outer arm `(+ 1 (resume 10 s))` → `(+ 1 12)` = 13. The condition runs
           once (one resume), so no effect is duplicated.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< (Amb.flip) 50) (+ 1 (Amb.flip)) 0)))
      (export main)))
  (output (: 13 Int64)))

(case
  "a let-wrapped resume in the arm body threads a computed next-state (stateful PRNG)"
  (doc
    "The two-hole refold handles an arm body that is a `let` whose binder feeds BOTH the resume value
           and the resume next-state: `(roll (k) s (let ((s2 (* s 16807))) (resume (% s2 k) s2)))` — a linear-
           congruential PRNG draw. `resolved_of` peels the `let` and would hand back a `Resume` whose value
           `(% s2 k)` and next-state `s2` reference the let binder `s2` DANGLING (the enclosing `let` dropped),
           so the recursive re-seed would see `s2` unbound. The refold matches the `let` STRUCTURALLY before
           the resume check and INLINES the (pure) binding `s2 := (* s 16807)`, closing the resume's value and
           next-state so each draw re-seeds the recursive fold with the advanced state. Two sequential draws:
           seed 7 → s1 = 7*16807 = 117649, x = 117649 % 1000 = 649; s2 = 117649*16807, y = s2 % 1000 = 743;
           `(+ x y)` = 1392. One resume per arm activation, so each `C` is spliced once — the LCG step runs
           exactly once per draw (no effect duplication).")
  (input
    (do
      (effect Prng (op roll (-> Int64 Int64)))
      (def
        (main)
        (handle
          Prng
          7
          ((roll (k) s (let ((s2 (* s 16807))) (resume (% s2 k) s2))))
          (let ((x (Prng.roll 1000))) (let ((y (Prng.roll 1000))) (+ x y)))))
      (export main)))
  (output (: 1392 Int64)))

(case
  "a closure capturing an inner-handled perform result is applied under an OUTER handler of the same effect"
  (doc
    "A CLOSURE built inside an INNER handle captures a `let`-bound perform RESULT (`base`), then escapes
           to be applied under an OUTER handler of the SAME effect. The capture must be the inner-handled
           VALUE, NOT a re-perform: `base` is bound to `(Ctr.tick)` under `handle Ctr 50` (a get/set arm),
           so base = 50 and the closure is `(fn (x) (+ x 50))`. Applied under `handle Ctr 5` as `(f 3)`, the
           result must be 3 + 50 = 53. It MISCOMPILED to 8 = 3 + 5 (each apply RE-performed the tick at the
           apply site, re-homed by the OUTER handler) because the capture was compiled as the perform
           EXPRESSION, not its value — the closure-capture-reperform miscompile. Fixed by discharging the
           inner handle when reducing the returned closure (`lambda_of`) AND closing a pure captured binding
           into the closure body before threading detaches it (the let-thread capture-value inline), so the
           closure closes over the value 50, not the perform.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (let
            ((f
                (handle
                  Ctr
                  50
                  ((tick (u) s (resume s (+ s 1))))
                  (let ((base (Ctr.tick))) (fn ((: x Int64)) (+ x base))))))
            (f 3))))
      (export main)))
  (output (: 53 Int64)))

(case
  "a closure captures TWO inner-handled perform results across NESTED lets and escapes under an outer handler"
  (doc
    "The nested-`let` sibling of the capture case: the inner handle's body is a `(let ((a (Ctr.tick)))
           (let ((b (Ctr.tick))) (fn (x) (+ x (+ a b)))))` — an OUTER `let` binding `a` referenced by a
           closure buried in the INNER `let`. Both captures must close over their inner-handled VALUES (a =
           50, b = 51 under `handle Ctr 50`, threading state), so the closure is `(fn (x) (+ x 101))` and
           `(f 3)` under `handle Ctr 5` = 3 + 101 = 104. It over-declined CDZ0101 `unbound a` because the
           capture-value inline gated on the let body being DIRECTLY a lambda — here it is another `let`, so
           the outer capture `a` orphaned. Fixed by peeling let-chains (`body_returns_lambda`) so a closure
           reached through nested lets still closes over the outer capture.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (let
            ((f
                (handle
                  Ctr
                  50
                  ((tick (u) s (resume s (+ s 1))))
                  (let ((a (Ctr.tick))) (let ((b (Ctr.tick))) (fn ((: x Int64)) (+ x (+ a b))))))))
            (f 3))))
      (export main)))
  (output (: 104 Int64)))

(case
  "a closure capturing perform results escapes to NO handler at all and applies pure"
  (doc
    "The no-outer-handler sibling of the capture cases above (those apply the escapee under an OUTER
           handler of the same effect; here NOTHING handles the effect at the apply sites): the handle's
           RESULT is a closure whose captures are two perform results (x = 5, y = 6 under the advancing
           arm), applied TWICE after the handle fully exits. Both applications must read the captured
           VALUES — (f 10) = 56 and (f 100) = 506 → 562. A capture compiled as the perform expression
           would need a handler at apply and could only reject or re-home; the values must live in the
           closure env, independent of any handler existing.")
  (input
    (do
      (effect St (op a (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def
            f
            (handle
              St
              n
              ((a (u) s (resume s (+ s 1))))
              (do (def x (St.a)) (def y (St.a)) (fn ((: k Int64)) (+ (* k x) y)))))
          (+ (f 10) (f 100))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 562 Int64)))

(case
  "a handle's RESULT seeds the NEXT handle of the same effect — explicit state handoff between instances"
  (doc
    "Sequential same-effect handle instances share nothing implicitly (each seeds fresh); the ONLY
           state transfer is explicit value flow. The first instance's result (its last-read state 8, after
           a +5 advance) becomes the second instance's SEED, whose doubling arm then serves 8 and 16 →
           8 + 16 = 24. Pins the instance-lifecycle boundary: a leak of the first instance's live state
           into the second (rather than the passed value) or a stale-seed re-read would shift both reads.")
  (input
    (do
      (effect St (op a (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def r1 (handle St n ((a (u) s (resume s (+ s 5)))) (+ (* 0 (St.a)) (St.a))))
          (handle St r1 ((a (u) s (resume s (* s 2)))) (+ (St.a) (St.a)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 24 Int64)))

(case
  "a CURRIED closure capturing an inner-handled perform result closes over it through partial application"
  (doc
    "A curry sibling of the capture case: the inner handle returns `(fn (a) (fn (b) (+ (+ a b) base)))`
           where `base` is the inner-handled `(Ctr.tick)` = 50. Applied `((f 3) 4)` under `handle Ctr 5`, the
           OUTER lambda binds a=3 and returns the residual `(fn (b) (+ (+ 3 b) 50))` (base closed over the
           inner value), then the residual binds b=4 → 3+4+50 = 57. Exercises the closure-capture fix through
           `apply_lambda`'s partial-application/curry path (the reified closure is itself lambda-returning),
           distinct from the direct and nested-let cases. Pins that a captured perform result stays the VALUE
           across currying, never re-performed at either application.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (let
            ((f
                (handle
                  Ctr
                  50
                  ((tick (u) s (resume s (+ s 1))))
                  (let ((base (Ctr.tick))) (fn ((: a Int64)) (fn ((: b Int64)) (+ (+ a b) base)))))))
            ((f 3) 4))))
      (export main)))
  (output (: 57 Int64)))

(case
  "a closure capturing a value computed under a handler may escape the handle applied directly"
  (doc
    "The discharge-then-capture idiom (the 'configure a callback from handled state' pattern): the
           perform `(St.get)` runs INSIDE the handle body (a `let` init — the handler is live), and the
           ESCAPING closure captures only the resulting Int64 VALUE `v`. The closure performs nothing, so the
           escape is sound — `((handle St k (arm) (let ((v (St.get))) (fn (x) (+ x v)))) 10)` with k=7 folds
           v=7, closes over it, and applying the escaped closure to 10 yields 17. This was over-rejected
           CDZ0401 (the escape analysis conflated a lexically-inner perform with an escaping one); the
           `lambda_of` handler-discharge fix (the closure-capture-reperform family) folds the in-extent
           `St.get` to its value so the escaped closure is pure. The genuinely-unsound twin — the closure
           BODY performing `(fn (x) (+ x (St.get)))` — correctly STAYS rejected (its perform runs
           out-of-extent on outside-application); this case pins the sound half of that boundary.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: k Int64))
        ((handle St k ((get (u) s (resume s s))) (let ((v (St.get))) (fn ((: x Int64)) (+ x v))))
          10))
      (export main)))
  (call main (: 7 Int64))
  (output (: 17 Int64)))

(case
  "a BARE-param escaping closure capturing a handled value FOLDS to 17 (matches the annotated-param twin)"
  (doc
    "The BARE-parameter twin of the escaping-closure-captures-handled-value case above. The SAME sound
           shape — a closure capturing a `let`-bound in-extent perform result `v`, escaping the handle,
           applied outside — but the closure's parameter is BARE `(fn (x) (+ x v))` instead of annotated
           `(fn ((: x Int64)) …)`. Both now FOLD to 17 on all 3 backends. This BARE case previously DECLINED
           `parameter reference has no local slot` because the discharge copy FRESHENED the closure's bare
           head binder but SHARED its pinned body-refs (still resolving to the original head), so
           `apply_lambda` keyed substitution on the fresh head while the body referenced the original
           (count_refs 0 → the arg was never spliced → a slot-less `Core::Param`). The annotated twin was
           consistent-by-accident: `param_name_occ` peels `(: x T)` to the inner name occ, the same shared
           node the body-refs point at, so freshening the `(: …)` wrapper preserved the identity. FIXED by
           v-inference #3811 (99f63b7f5): a PINNED bare binder-occurrence is now SHARED (not freshened) in
           `beta_reduce`, matching the annotated path and the body-refs — scoped to pinned binders only
           (partial-app/mono copies still freshen). (Root-cause v-effects; fix v-inference.)")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        ((handle St 7 ((get (u) s (resume s s))) (let ((v (St.get))) (fn (x) (+ x v)))) 10))
      (export main)))
  (call main)
  (output (: 17 Int64)))

(case
  "ca1c a closure over a performing nested-let-init capture folds via the capture-once hoist"
  (doc
    "The capture-once/bind-once fold (v-effects). A closure whose init-let binds a PERFORMING draw
           referenced by the returned lambda — (let ((f (let ((a (St.next))) (fn (x) (* a x))))) (f 10))
           under a +1-stride St handler seeded n — FOLDS: reduce_handle hoists the performing init OUT of
           the closure's value-let to wrap the binding, so the draw is threaded ONCE and the lambda closes
           over the captured RESULT (rather than re-running the draw at each application, the old silent-60
           miscompile). a = St.next captured ONCE = seed n; (f 10) = 10*n. At n=5 that is 50.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let ((f (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64)))

(case
  "ca1m a capture-once closure applied TWICE shares the single draw across both applications"
  (doc
    "The multi-application face of the capture-once hoist (v-effects). The same performing-capture
           closure as ca1c, applied TWICE — (let ((f (let ((a (St.next))) (fn (x) (* a x))))) (+ (f 10)
           (f 20))). The draw must happen ONCE (a = seed n, SHARED across both applications), NOT per use:
           a naive inline-per-application would re-draw (reading n then the advanced n+1) and mis-fold. The
           hoist wraps the binding with the single threaded init, so both (f 10) and (f 20) close over the
           one captured a. At n=5: a=5 captured once, (f 10)=50 + (f 20)=100 = 150. Pins the shared-once
           invariant.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let ((f (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) (+ (f 10) (f 20)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 150 Int64)))

(case
  "cc3 a factory returning a closure over a performing-arg param folds via the capture-once hoist"
  (doc
    "The FACTORY-arg face of the capture-once fold (v-effects). A helper mk returns a closure over its
           param, fed by a performing arg — (let ((f (mk (St.next)))) (+ (f 10) (f (St.next)))) under a
           +1-stride St handler seeded n — FOLDS: the hoist lifts the performing arg to a fresh #cap wrapping
           the binding, inlines the factory call (mk #cap) to a pure closure over the drawn RESULT, and
           deep_fresh_copy gives the rewritten tree a coherent parent chain so #cap resolves. m = first
           St.next captured ONCE = seed n; (f 10) = 10*n; the second St.next in the body = n+1, (f (n+1)) =
           (n+1)*n; sum = n*n + 11*n. At n=5 that is 80 (NOT the old silent 116).")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (mk (: m Int64)) (fn ((: x Int64)) (* x m)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let ((f (mk (St.next)))) (+ (f 10) (f (St.next))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 80 Int64)))

(case
  "cp3 two performing draws in one capture-once closure init fold once each, in order"
  (doc
    "Two performing inits in the SAME closure-value-let, applied once: (let ((f (let ((a (St.next))
           (b (St.next))) (fn (x) (+ (* a x) b))))) (f 10)). The capture-once hoist lifts BOTH draws out to
           wrap the binding, each threaded ONCE in order — a = seed n, b = n+1. At n=5: a=5, b=6, (f 10) =
           5*10 + 6 = 56.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let ((f (let ((a (St.next)) (b (St.next))) (fn ((: x Int64)) (+ (* a x) b))))) (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "cp1 a capture-once closure CAPTURED by a nested closure folds via the deep-copy hoist hygiene"
  (doc
    "The nested-capture face of the capture-once fold (v-effects). The capture-once closure g is not
           applied directly but CAPTURED by a wrapping closure h: (let ((g (let ((a (St.next))) (fn (x) (* a
           x))))) (let ((h (fn (y) (+ (g y) 1)))) (h 10))). FOLDS: the hoist lifts g's draw out, and
           deep_fresh_copy of the rewritten tree gives every node a coherent parent chain so g's reference
           inside h resolves to the hoisted binder (without the deep copy, the reused subtree shared a
           load-time atom whose orphaned parent chain dead-ended the scope walk → a false unbound / a re-draw
           61). a = St.next captured ONCE = seed n = 5; h(10) = g(10)+1 = 50+1 = 51.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((g (let ((a (St.next))) (fn ((: x Int64)) (* a x)))))
            (let ((h (fn ((: y Int64)) (+ (g y) 1)))) (h 10)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 51 Int64)))

(case
  "TWO performs in an if condition both fold on the strict-first spine"
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (resume 1 s))) (if (= (Amb.flip) (Amb.flip)) 100 200)))
      (export main)))
  (doc
    "Both operands of an `if` CONDITION perform — `(if (= (Amb.flip) (Amb.flip)) 100 200)`. The
           condition is a strict, evaluated-first position and `=`'s two operands are strict-first
           sub-positions, so BOTH flips lie on the uniform strict spine and fold: each resumes 1 (a
           tail-resumptive arm, seed 0 read twice — no state advance), so the condition is `(= 1 1)` = true
           and the handle yields the then-branch 100. Extends the single-perform-in-a-condition case to two
           performs in the SAME condition — a compiler pass that reads two fresh values to decide a branch.")
  (output (: 100 Int64)))

(case
  "a handler arm that resumes NON-tail folds when the perform is in an if condition"
  (doc
    "The pure one-hole continuation extends into an `if` CONDITION — a strict, always-evaluated-first
           position, so the continuation `C = (if (< [] 5) 1 2)` is uniform (the branches run only AFTER the
           condition and are pure). `(resume 10 s)` returns into it: `C[10] = (if (< 10 5) 1 2)` = 2, and the
           arm `(+ 1 (resume 10 s))` evaluates to `(+ 1 2)` = 3. Both branches are effect-free, so a
           multi-shot resume could duplicate the whole `if` with no effect change. A perform in a conditional
           BRANCH (a non-uniform continuation) still declines — that needs the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< (Amb.flip) 5) 1 2)))
      (export main)))
  (output (: 3 Int64)))

(case
  "a handler arm that resumes NON-tail folds when the perform is in a match scrutinee"
  (doc
    "The pure one-hole continuation extends into a `match` SCRUTINEE — a strict, always-evaluated-first
           position (like an `if` condition), so `C = (match [] (0 100) (_ 2))` is uniform (the arms run only
           after the scrutinee and are pure). `(resume 10 s)` → `C[10]` selects the `_` arm → 2, and the arm
           `(+ 1 (resume 10 s))` evaluates to `(+ 1 2)` = 3. Every arm BODY is effect-free; a perform in an
           arm body (a non-uniform continuation) still declines — that needs the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (match (Amb.flip) (0 100) (_ 2))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a MULTI-shot arm over an if-condition hole duplicates the whole pure if safely"
  (doc
    "The one-hole if-condition continuation with a MULTI-SHOT arm: the arm resumes TWICE, so the whole
           pure `if` continuation `C = (if (< [] 5) 100 2)` is duplicated once per resume with no effect
           change (both branches are effect-free). `(* (resume 1 s) (resume 2 s))` → `(* C[1] C[2])` =
           `(* (if (< 1 5) 100 2) (if (< 2 5) 100 2))` = `(* 100 100)` = 10000. Pins that a multi-shot
           resume over a condition-hole context copies the pure branches soundly.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (* (resume 1 s) (resume 2 s)))) (if (< (Amb.flip) 5) 100 2)))
      (export main)))
  (output (: 10000 Int64)))

(case
  "a flat multi-shot arm with two performs on a strict spine folds the cross-product"
  (doc
    "A MULTI-SHOT arm resumes TWICE and SUMS its two continuations — `(flip (u) s (+ (resume 2 s)
           (resume 3 s)))` — over a body with TWO performs on a flat strict `*` spine `(* (Amb.flip)
           (Amb.flip))`. Each flip forks into resume-2 and resume-3, so the two flips produce the 4-path
           cross-product: (2*2)+(2*3)+(3*2)+(3*3) = 4+6+6+9 = 25. Pins the multi-shot refold on a flat
           spine (the recursive-cycle face is a separate not-yet-reducible decline, kept as a white-box
           rcdzc test).")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ (resume 2 s) (resume 3 s)))) (* (Amb.flip) (Amb.flip))))
      (export main)))
  (output (: 25 Int64)))

(case
  "a handler distributes into a match on a pure scrutinee whose selected arm performs"
  (doc
    "Handler distribution over a `match` whose SCRUTINEE is pure and whose selected ARM performs (the
           non-uniform-continuation twin of the pure-scrutinee-hole case). `(match (< 3 5) (true (Amb.flip))
           (false 2))` — the scrutinee `(< 3 5)` is pure → selects the `true` arm, which distributes to
           `(handle … (Amb.flip))` = an identity slice, so `(resume 10 s)` = 10 and the arm `(+ 1 (resume 10
           s))` = 11; the `false` arm is a pure body that does not run.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (+ 1 (resume 10 s))))
          (match (< 3 5) (true (Amb.flip)) (false 2))))
      (export main)))
  (output (: 11 Int64)))

(case
  "a handler arm that resumes NON-tail folds when the perform is in an and lhs"
  (doc
    "The pure one-hole continuation extends into a short-circuit connective's LHS — a strict,
           always-evaluated-first position. `C = (and (< [] 5) true)`; the arm `(not (resume 10 s))` produces
           a Bool: `(resume 10 s)` → `C[10] = (and (< 10 5) true)` = false, and `(not false)` = true. The rhs
           `true` runs only on the taken path and is pure (copied into `C`); a perform in the RHS — a
           conditionally-run position — still declines.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (not (resume 10 s)))) (and (< (Amb.flip) 5) true)))
      (export main)))
  (output (: true Bool)))

(case
  "a pure one-hole continuation folds with pure siblings around the hole"
  (doc
    "The pure one-hole context may have PURE siblings around the hole inside a nested strict operator
           tree. `(- (* 2 (Amb.flip)) 3)` has `C = (- (* 2 []) 3)`, effect-free; the arm `(+ 1 (resume 10
           s))` → `(+ 1 (- (* 2 10) 3))` = `(+ 1 17)` = 18. Pins that the hole is located correctly and the
           pure siblings (`2`, `3`) are preserved in the spliced continuation.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (- (* 2 (Amb.flip)) 3)))
      (export main)))
  (output (: 18 Int64)))

(case
  "a pure one-hole match-scrutinee fold selects a non-wildcard arm by the resume value"
  (doc
    "The match-scrutinee one-hole fold where the RESUME value selects a NON-wildcard arm. `C = (match
           [] (0 100) (_ 2))`; the arm resumes 0, so `C[0]` selects the literal `0` arm → 100, and the arm
           `(+ 1 (resume 0 s))` = `(+ 1 100)` = 101 (contrast a resume of 10, which selects the `_` arm →
           2 → 3). Pins that the re-reduced scrutinee dispatches to the matching arm, not always the
           wildcard.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 0 s)))) (match (Amb.flip) (0 100) (_ 2))))
      (export main)))
  (output (: 101 Int64)))

(case
  "a tail-resumptive arm is not hijacked by the pure one-hole fold when its body performs non-tail"
  (doc
    "Adversarial: the pure one-hole block must NOT hijack a TAIL-resumptive arm. The arm body
           `(resume s (+ s 1))` IS a tail resume, so the ordinary state-threading path runs, not the
           pure-continuation fold. `(+ 100 (Get.next))` seed 0: `Get.next` reads state 0 (the resume value
           is `s`) and threads `s+1` forward → `(+ 100 0)` = 100. Pins that a non-tail perform in the body
           does not tempt the one-hole fold when the arm is tail-resumptive.")
  (input
    (do
      (effect Get (op next (-> Unit Int64)))
      (def (main) (handle Get 0 ((next (u) s (resume s (+ s 1)))) (+ 100 (Get.next))))
      (export main)))
  (output (: 100 Int64)))

(case
  "a handler arm that resumes NON-tail folds when the perform is in a let init"
  (doc
    "The pure one-hole continuation extends into a `let` INIT — a `let` runs its inits and its body
           UNCONDITIONALLY, in sequence, so an init is a strict-spine position and the continuation
           `C = (let ((x [])) (+ x x))` is uniform. `(resume 10 s)` returns into it: `C[10] = (let ((x 10))
           (+ x x))` = 20, and the arm `(+ 1 (resume 10 s))` evaluates to `(+ 1 20)` = 21. The whole `let` is
           copied per resume, so the binder re-binds independently in each copy — a multi-shot resume is
           safe. A second perform elsewhere in the `let` (a two-hole context) still declines.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (let ((x (Amb.flip))) (+ x x))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a pure one-hole MATCH-scrutinee whose selected arm binds and uses the scrutinee re-resolves after the splice"
  (doc
    "A match scrutinee hole whose selected arm BINDS the scrutinee and USES the binder — the whole match
           is copied per resume, so the pattern binder `k` must re-resolve against the spliced scrutinee.
           `C = (match □ (0 100) (k (+ k k)))`, resume 10 → binder arm k=10 → `(+ 10 10)` = 20; arm
           `(+ 1 (resume 10 s))` → `(+ 1 20)` = 21.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (match (Amb.flip) (0 100) (k (+ k k)))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a pure one-hole conditional nested inside a strict operator folds"
  (doc
    "A conditional hole NESTED inside a strict operator: `(+ 1 (if (< (Amb.flip) 5) 10 20))`. The outer
           `+` and the `if` compose: `C = (+ 1 (if (< □ 5) 10 20))`, resume 10 → `(+ 1 (if (< 10 5) 10 20))`
           = `(+ 1 20)` = 21; arm `(+ 1 (resume 10 s))` → `(+ 1 21)` = 22.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 1 (if (< (Amb.flip) 5) 10 20))))
      (export main)))
  (output (: 22 Int64)))

(case
  "a pure one-hole fold passes a PURE perform-arg the op reads"
  (doc
    "The perform may take a PURE non-trivial ARG the op reads, substituting on the pure spine. `pick(x)`
           resumes `x*2`; body `(+ 1 (Amb.pick (+ 2 3)))`, arm `(+ 0 (resume (* x 2) s))`, x=5 → resume 10,
           C=(+ 1 □) → `(+ 0 (+ 1 10))` = 11.")
  (input
    (do
      (effect Amb (op pick (-> Int64 Int64)))
      (def (main) (handle Amb 0 ((pick (x) s (+ 0 (resume (* x 2) s)))) (+ 1 (Amb.pick (+ 2 3)))))
      (export main)))
  (output (: 11 Int64)))

(case
  "a pure one-hole arm reading the SEED state folds against a non-zero seed"
  (doc
    "The arm READS the state `(+ s (resume 10 s))` with a NON-ZERO seed 7. On a pure spine the state at
           the perform is the seed, so s = 7; C=(+ 100 □) → `(+ 7 (+ 100 10))` = 117.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 7 ((flip (u) s (+ s (resume 10 s)))) (+ 100 (Amb.flip))))
      (export main)))
  (output (: 117 Int64)))

(case
  "a pure one-hole locates the hole at a NON-LEADING operand"
  (doc
    "The hole may be at a non-leading operand: `C = (- 200 □)`, arm `(+ 1 (resume 10 s))` →
           `(+ 1 (- 200 10))` = 191. `splice_context` locates the sole perform by identity, preserving pure
           siblings.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (- 200 (Amb.flip))))
      (export main)))
  (output (: 191 Int64)))

(case
  "a pure one-hole locates the hole NESTED several operators deep"
  (doc
    "The hole may be nested several operators deep: `C = (+ 1 (* 3 □))`, arm `(+ 1 (resume 10 s))` →
           `(+ 1 (+ 1 (* 3 10)))` = 32.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 1 (* 3 (Amb.flip)))))
      (export main)))
  (output (: 32 Int64)))

(case
  "a perform threads through a NOT / boolean one-operand form"
  (doc
    "Strict one-operand forms (`not`, projection) thread their operand. `(not (= (Get.next) 0))` seed 0
           → Get reads 0, `= 0` true, `not` → false → the if takes the else arm 2.")
  (input
    (do
      (effect Get (op next (-> Unit Int64)))
      (def (main) (handle Get 0 ((next (u) s (resume s (+ s 1)))) (if (not (= (Get.next) 0)) 1 2)))
      (export main)))
  (output (: 2 Int64)))

(case
  "a perform threads through a tuple PROJECTION one-operand form"
  (doc
    "A projection threads its operand: `(. (tuple (Get.next) (Get.next)) 1)` seed 10 → the two Gets read
           10 then 11 in order, and `. _ 1` projects the second = 11.")
  (input
    (do
      (effect Get (op next (-> Unit Int64)))
      (def
        (main)
        (handle Get 10 ((next (u) s (resume s (+ s 1)))) (. #tuple((Get.next) (Get.next)) 1)))
      (export main)))
  (output (: 11 Int64)))

(case
  "a short-circuit connective with a perform in the RHS preserves short-circuit (rhs not run)"
  (doc
    "A connective's rhs runs only conditionally; the E-fold desugars `(or lhs rhs)` to `(if lhs true
           rhs)` so a rhs perform runs only on the taken path. `(or true (= (Get.next) 99))` short-circuits on
           `true`, so the rhs `Get.next` does NOT advance state — the following `(Get.next)` reads 0 (not 1).")
  (input
    (do
      (effect Get (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Get
          0
          ((next (u) s (resume s (+ s 1))))
          (if (or true (= (Get.next) 99)) (Get.next) 0)))
      (export main)))
  (output (: 0 Int64)))

(case
  "a pure one-hole in a let BODY (pure init) folds"
  (doc
    "A hole in the BODY of a let whose init is pure is a strict-spine position with a uniform
           continuation. `C = (let ((x 5)) (+ x □))`, resume 10 → `(+ 5 10)` = 15; arm `(+ 1 (resume 10 s))`
           → `(+ 1 15)` = 16.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (let ((x 5)) (+ x (Amb.flip)))))
      (export main)))
  (output (: 16 Int64)))

(case
  "a pure one-hole in a FIRST let init with a later init using its binder folds"
  (doc
    "A hole in the FIRST init of a let whose LATER init uses the bound binder: the whole let is copied
           per resume, so both binders re-resolve. `C = (let ((x □) (y (+ x 1))) (+ x y))`, resume 10 → x=10,
           y=11 → 21; arm `(+ 1 (resume 10 s))` → `(+ 1 21)` = 22.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (let ((x (Amb.flip)) (y (+ x 1))) (+ x y))))
      (export main)))
  (output (: 22 Int64)))

(case
  "a perform in an if-CONDITION with a NON-performing else selected folds via the one-shot refold"
  (doc
    "E5 two-hole with the leading hole in an if-CONDITION composing with distribution. `(if (< (Amb.flip)
           5) (+ 1 (Amb.flip)) 0)` performs in the condition and the then-branch. The refold takes the
           condition flip as the leading hole → `C = (if (< □ 5) (+ 1 (Amb.flip)) 0)`; `(resume 10 s)`
           re-reduces `C[10]` where `(< 10 5)` = false, so the ELSE branch (pure 0) is taken and the
           then-branch perform never runs → 0; the outer arm `(+ 1 (resume 10 s))` → `(+ 1 0)` = 1.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< (Amb.flip) 5) (+ 1 (Amb.flip)) 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a perform in an if-CONDITION with a PERFORMING taken branch folds via refold + distribution"
  (doc
    "The true direction of the condition-and-branch two-hole, where the taken (then) branch DOES perform:
           `(< 10 50)` = true → the then-branch `(+ 1 (Amb.flip))` is served by handler distribution →
           `(+ 1 (+ 1 10))` = 12; the outer arm `(+ 1 (resume 10 s))` → `(+ 1 12)` = 13.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< (Amb.flip) 50) (+ 1 (Amb.flip)) 0)))
      (export main)))
  (output (: 13 Int64)))

(case
  "an abortive perform in a MATCH ARM body is branch-local and folds per arm"
  (doc
    "A match on a pure scrutinee whose selected arm performs an ABORT — `(match x (0 (Bail.bail 7))
           (_ 42))` under an abortive `(bail (n) s n)` handler. The abort is branch-local: x=0 → the arm
           aborts to the arm value 7; x=1 → the wildcard arm yields the pure 42 (no perform).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main (: x Int64)) (handle Bail 0 ((bail (n) s n)) (match x (0 (Bail.bail 7)) (_ 42))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: 1 Int64))
  (output (: 42 Int64)))

(case
  "a handler arm that resumes NON-tail folds through a pure continuation containing an effect-free call"
  (doc
    "The pure one-hole continuation `C` may contain a NON-RECURSIVE user CALL whose body reaches no
           effect — not only primitive operators. Cadenza is strict, so the call evaluates its argument
           exactly once before running, and an effect-free callee adds no effect of its own: `C = (dbl [])`
           where `dbl x = x*2` is a uniform, effect-free continuation. `(resume 10 s)` returns into it:
           `C[10] = (dbl 10)` = 20, and the arm `(+ 1 (resume 10 s))` evaluates to `(+ 1 20)` = 21. Splicing
           the pure call (once here, or many times for a multi-shot resume) re-runs an effect-free
           computation — observationally identical to running it once — so no reified continuation is
           needed. A call whose body ITSELF performs makes the continuation non-uniform and still declines.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (dbl (: x Int64)) (* x 2))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (dbl (Amb.flip))))
      (export main)))
  (output (: 21 Int64)))

(case
  "an effect result drives the bound of a PURE recursive helper called in the handle body"
  (doc
    "The effect-free callee the fold treats as opaque may itself be RECURSIVE, as long as its
           recursion reaches NO effect — the companion of the non-recursive effect-free-call cases above.
           The perform is discharged ONCE in the handle body and its result becomes the ARGUMENT to a pure
           recursive helper whose whole recursion is effect-free: `(sum-to (Cfg.limit))` where `sum-to n =
           (if (= n 0) 0 (+ n (sum-to (- n 1))))`. `Cfg.limit` resumes 4, so `(sum-to 4)` = `4 + 3 + 2 + 1`
           = 10. Pins that the fold discharges the single perform to its resume value and then runs the pure
           recursion as an ordinary effect-free computation on that value — the effect does not enter the
           helper's recursion at all (the helper is a separate, self-contained pure function the perform
           merely feeds). Distinct from the effect-context-SPECIALIZED recursive walks (where the recursion
           ITSELF performs): here the recursion is effect-free and only its INPUT comes from an effect.")
  (input
    (do
      (effect Cfg (op limit (-> Unit Int64)))
      (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (- n 1)))))
      (def (main) (handle Cfg 0 ((limit (u) s (resume 4 s))) (sum-to (Cfg.limit))))
      (export main)))
  (output (: 10 Int64)))

(case
  "a MULTI-shot arm over a pure effect-free call in the continuation folds"
  (doc
    "A multi-shot arm duplicates `C = (dbl □)` safely because `dbl` is effect-free (splicing it per
           resume re-runs an effect-free computation). `(+ (resume 1 s) (resume 2 s))` over `(dbl (Amb.flip))`
           → `(+ (dbl 1) (dbl 2))` = `(+ 2 4)` = 6.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (dbl (: x Int64)) (* x 2))
      (def (main) (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (dbl (Amb.flip))))
      (export main)))
  (output (: 6 Int64)))

(case
  "NESTED effect-free calls in the continuation compose and fold"
  (doc
    "The pure one-hole continuation may nest effect-free calls: `C = (dbl (inc □))`, arm
           `(+ 1 (resume 10 s))` → `(+ 1 (dbl (inc 10)))` = `(+ 1 22)` = 23.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (dbl (: x Int64)) (* x 2))
      (def (inc (: y Int64)) (+ y 1))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (dbl (inc (Amb.flip)))))
      (export main)))
  (output (: 23 Int64)))

(case
  "a continuation call whose body ITSELF performs is not effect-free and declines"
  (doc
    "GUARD: a user call in `C` whose body performs the discharged effect is NOT effect-free — the
           continuation is not pure (a second effect on the spine), so it must decline cleanly (not
           miscompile). `bad x = (+ x (Amb.flip))` performs, so `(bad (Amb.flip))` has two performs.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (bad (: x Int64)) (+ x (Amb.flip)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (bad (Amb.flip))))
      (export main)))
  (call main)
  (output (: 22 Int64)))

(case
  "a NON-tail outer handler reduces a reducible inner handle first, then folds its own perform"
  (doc
    "An outer handler whose arm is non-tail-resumptive, over a body containing a reducible inner handle
           of a DIFFERENT effect. Reducing the inner B handle FIRST turns the body into `(+ (A.a) 20)`, a
           single A-perform in a pure one-hole context A's fold serves: B `(resume 20 t)` → 20; A arm
           `(+ 1 (resume 10 s))`, C = `(+ 10 □)` → `(+ 1 (+ 10 20))` = 31.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          0
          ((a (u) s (+ 1 (resume 10 s))))
          (handle B 0 ((b (u) t (resume 20 t))) (+ (A.a) (B.b)))))
      (export main)))
  (output (: 31 Int64)))

(case
  "two nested distinct effects both tail-resumptive fold via the threading path"
  (doc
    "The both-tail control for the nested-handle pre-reduction: both handlers are tail-resumptive so
           the existing threading path folds it. A `(resume 10 s)` → 10, B `(resume 20 t)` → 20, `(+ 10 20)`
           = 30.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          0
          ((a (u) s (resume 10 s)))
          (handle B 0 ((b (u) t (resume 20 t))) (+ (A.a) (B.b)))))
      (export main)))
  (output (: 30 Int64)))

(case
  "a non-tail inner handle with a foreign perform sibling stays declined (needs frames)"
  (doc
    "When the INNER handle is itself non-tail with a FOREIGN perform sibling in its body (`(A.a)` is
           undischarged by B), B cannot reduce — its continuation is not pure (a foreign effect would be
           duplicated by a multi-shot resume). This genuinely needs the frame vertical, so it declines
           cleanly rather than miscompile.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          0
          ((a (u) s (+ 1 (resume 10 s))))
          (handle B 0 ((b (u) t (+ 2 (resume 20 t)))) (+ (A.a) (B.b)))))
      (export main)))
  (call main)
  (output (: 33 Int64)))

(case
  "a recursive builder PERFORMS per step and a recursive pure fold consumes the built list"
  (doc
    "The two recursive helpers above composed, with the effect in the OPPOSITE one: here the
           recursion that performs is the BUILDER — `(grab k acc)` pushes one `(Cnt.bump)` result per
           step for four steps — and the CONSUMER `(suml xs)` is a pure generic match-recursion over the
           result. The counter arm resumes the current count and advances (seed 5 → resumes 5,6,7,8), so
           the built list is [5 6 7 8] and the pure fold sums it to 26. Pins the build-then-fold pipeline
           under ONE handle: an effect-specialized recursion hands a heap list across to an effect-FREE
           recursion, and each `bump`'s resume value must land in its own list slot (a re-served or
           re-ordered perform shifts a slot and breaks the sum).")
  (input
    (do
      (effect Cnt (op bump (-> Unit Int64)))
      (def (suml xs) (match xs (#list() 0) (#list(h (.. t)) (+ h (suml t)))))
      (def
        (grab (: k Int64) (: acc (List Int64)))
        (if (= k 0) acc (grab (- k 1) (List.push acc (Cnt.bump)))))
      (def
        (main (: n Int64))
        (handle Cnt n ((bump (u) s (resume s (+ s 1)))) (suml (grab 4 #list()))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 26 Int64))
  (live-objects 0))

(case
  "MUTUALLY recursive helpers BOTH perform against the same handler"
  (doc
    "The recursion pins above are all SINGLE functions; here `evens`/`odds` call each other and BOTH
           perform `(Cnt.tick)`, with different weights per side (×10 vs ×1) so a dispatch that specializes
           only one side of the cycle — or re-serves a tick to the wrong caller — lands off the checksum.
           Ticks walk 5,6,7,8 alternating sides: 10·5 + 6 + 10·7 + 8 = 134. Pins effect-specialization
           across a mutual-recursion CYCLE, not just self-recursion.")
  (input
    (do
      (effect Cnt (op tick (-> Unit Int64)))
      (def (evens (: k Int64)) (if (= k 0) 0 (+ (* 10 (Cnt.tick)) (odds (- k 1)))))
      (def (odds (: k Int64)) (if (= k 0) 0 (+ (Cnt.tick) (evens (- k 1)))))
      (def (main (: n Int64)) (handle Cnt n ((tick (u) s (resume s (+ s 1)))) (evens 4)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 134 Int64)))

(case
  "a mutual-recursion pair where each side performs against its OWN handler (two nested frames)"
  (doc
    "The two-frame composition of the mutual-cycle pin above: `pa` performs the OUTER `A`, `pb` the
           INNER `B`, so every hop around the cycle alternates WHICH live frame serves — and each frame
           advances independently (A: 5,6 stepped ×1; B: 100,110 stepped ×10). 10·5 + 100 + 10·6 + 110 =
           320. A cross-frame mixup (either handler serving the other's op, or an advance landing on the
           wrong state) breaks the place-value sum.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def (pa (: k Int64)) (if (= k 0) 0 (+ (* 10 (A.a)) (pb (- k 1)))))
      (def (pb (: k Int64)) (if (= k 0) 0 (+ (B.b) (pa (- k 1)))))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a (u) s (resume s (+ s 1))))
          (handle B 100 ((b (u) t (resume t (+ t 10)))) (pa 4))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 320 Int64)))

(case
  "a let-bound lambda whose body performs is applied in the handle body"
  (doc
    "A LAMBDA VALUE is pure at CONSTRUCTION — its body's effects fire only when it is APPLIED. So a
           `let` binding a performing lambda is a pure binding, and the discharged op surfaces at the
           APPLICATION `(f 10)`, which the fold inlines: `f = (fn (x) (+ x (Ask.get)))` inlines to
           `(+ 10 (Ask.get))`, a single perform in a pure one-hole context `C = (+ 10 [])`. The handler
           resumes 5 (a countdown seed 0 read once), so `(f 10)` = `(+ 10 5)` = 15. Pins that the fold's
           effect-reachability walk does NOT descend into a lambda body when deciding a subterm is pure —
           constructing the closure performs nothing, and its one application is where the op is handled.
           Before the fix, the pure binding was misclassified as effectful and the case declined.")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (def
        (main)
        (handle Ask 0 ((get (u) s (resume 5 s))) (let ((f (fn (x) (+ x (Ask.get))))) (f 10))))
      (export main)))
  (output (: 15 Int64)))

(case
  "a pure let-bound lambda and a performing one are both applied in the handle body"
  (doc
    "Composes the preceding case with a SIBLING pure lambda binding — two let-bound lambdas, one
           effect-free (`g x = x*2`) and one performing (`f x = x + (Ask.get)`), both applied in a strict
           sum. Neither binding performs at construction; the pure `g` is spliced verbatim into the
           continuation and the performing `f`'s application `(f 10)` inlines to `(+ 10 (Ask.get))` — the
           single hole. `C = (+ (g 3) (+ 10 []))`; the handler resumes 5, so the body is `(+ 6 (+ 10 5))`
           = 21. Pins that skipping a lambda body in the purity walk still admits a genuinely pure
           sibling-lambda continuation (the fix does not over-admit — a lambda that were APPLIED to a
           performing argument would still surface that perform at the application node).")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          Ask
          0
          ((get (u) s (resume 5 s)))
          (let ((g (fn (y) (* y 2))) (f (fn (x) (+ x (Ask.get))))) (+ (g 3) (f 10)))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a handler arm that resumes NON-tail folds a perform in an if branch by handler distribution"
  (doc
    "A perform in an `if` BRANCH (a CONDITIONALLY-run position) folds when the CONDITION is pure, by
           HANDLER DISTRIBUTION — a commuting conversion: `(handle E s arms (if c t e))` is equivalent to
           `(if c (handle E s arms t) (handle E s arms e))`. The condition runs exactly once (it is pure, so
           it advances no handler state), and each branch becomes a smaller handle body the pure one-hole
           fold already serves — only the taken branch runs, seeing the seed state. Here `(if (< 3 5) (+ 1
           (Amb.flip)) 0)` distributes: the true branch `(handle … (+ 1 (Amb.flip)))` has `C = (+ 1 [])`, so
           `(resume 10 s)` = `C[10]` = 11 and the arm `(+ 1 (resume 10 s))` = `(+ 1 11)` = 12; the false
           branch is a pure body. `(< 3 5)` is true → 12. A perform in the CONDITION itself (not a pure
           condition) still declines — distributing it would need the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< 3 5) (+ 1 (Amb.flip)) 0)))
      (export main)))
  (output (: 12 Int64)))

(case
  "a handler arm that resumes NON-tail folds a perform in a match arm body by handler distribution"
  (doc
    "The commuting conversion of the preceding case, over a `match` with a pure SCRUTINEE:
           `(handle E s arms (match k (p b)…))` is equivalent to `(match k (p (handle E s arms b))…)`. The
           scrutinee runs exactly once (pure, evaluated before any arm, advancing no state), and each arm
           body becomes a smaller handle body the pure one-hole fold serves — only the matched arm runs. A
           pattern binder still scopes its (reduced) arm body. Here `(match 1 (0 5) (_ (+ 1 (Amb.flip))))`
           distributes: scrutinee `1` selects the `_` arm → `(handle … (+ 1 (Amb.flip)))` has `C = (+ 1 [])`,
           so `(resume 10 s)` = 11 and the arm `(+ 1 (resume 10 s))` = 12. A perform in the SCRUTINEE itself
           (not pure) still declines — that needs the frame machinery.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (match 1 (0 5) (_ (+ 1 (Amb.flip))))))
      (export main)))
  (output (: 12 Int64)))

(case
  "a distributed match arm re-resolves its pattern binder inside the pushed handle"
  (doc
    "The distribution of the preceding match case, but the performing arm reads a PATTERN BINDER: the
           binder must re-resolve inside the pushed sub-handle, not be lost when the arm body is wrapped.
           `(match 7 (n (+ n (Amb.flip))))` — the wildcard-style binder `n` binds the scrutinee 7, and the
           arm distributes to `(handle … (+ n (Amb.flip)))` with `C = (+ 7 [])`, so `(resume 10 s)` = 10 and
           the arm `(+ 1 (+ 7 10))` = 18.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (match 7 (n (+ n (Amb.flip))))))
      (export main)))
  (output (: 18 Int64)))

(case
  "a handler distributes into an if whose ELSE branch performs"
  (doc
    "The else-branch face of the pure-if distribution: the taken branch is the ELSE, and it performs.
           `(if (< 5 3) 0 (+ 1 (Amb.flip)))` — `(< 5 3)` is false, so the else branch `(handle … (+ 1
           (Amb.flip)))` runs with `C = (+ 1 [])` → `(resume 10 s)` = 11, arm `(+ 1 11)` = 12; the then
           branch is a pure body that does not run. Pins that distribution serves a performing else branch
           exactly as it serves a performing then branch.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< 5 3) 0 (+ 1 (Amb.flip)))))
      (export main)))
  (output (: 12 Int64)))

(case
  "a handler distributes into an if whose BOTH branches perform"
  (doc
    "Both branches of a pure-condition `if` perform; distribution folds each into its own sub-handle
           and only the taken one runs. `(if (< 3 5) (+ 1 (Amb.flip)) (* 2 (Amb.flip)))` — `(< 3 5)` is true
           → the then sub-handle `(+ 1 (+ 1 10))` = 12; the else sub-handle `(+ 1 (* 2 10))` = 21 folds too
           but is dead. Pins that distribution handles a perform in EACH branch independently, taking only
           the selected branch's value.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (+ 1 (resume 10 s))))
          (if (< 3 5) (+ 1 (Amb.flip)) (* 2 (Amb.flip)))))
      (export main)))
  (output (: 12 Int64)))

(case
  "a handler arm that resumes NON-tail folds a perform in a short-circuit connective right operand"
  (doc
    "A perform in an `and`/`or` RIGHT operand (a conditionally-run position) folds by composition: the
           connective desugars to `if` — `(and l r)` is `(if l r false)`, `(or l r)` is `(if l true r)` —
           and the `if`-branch perform then distributes (the pure-conditioned tail conditional case). The
           short-circuit is preserved because the right operand becomes a conditionally-taken branch: it runs
           only when the left operand selects it. Here `(and (< 3 5) (< (Amb.flip) 5))` with arm `(not (resume
           10 s))`: the left `(< 3 5)` is true, so the right runs — `C = (< [] 5)`, `(resume 10 s)` = `(< 10
           5)` = false, and `(not false)` = true.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (not (resume 10 s)))) (and (< 3 5) (< (Amb.flip) 5))))
      (export main)))
  (output (: true Bool)))

(case
  "an or whose left operand short-circuits elides the performing right operand"
  (doc
    "The short-circuit-soundness twin of the and-right-operand case: `(or l r)` desugars to `(if l
           true r)`, so a TRUE left operand selects the `true` branch and the right operand — which here
           PERFORMS — is DEAD and must not run. `(or true (< (Amb.flip) 5))` under arm `(not (resume 10 s))`
           short-circuits on `true`, so `Amb.flip` never fires and the handle is its body value `true`.
           Pins that distribution preserves short-circuit: the elided branch's perform does not execute.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (not (resume 10 s)))) (or true (< (Amb.flip) 5))))
      (export main)))
  (output (: true Bool)))

(case
  "a two-hole perform in a match scrutinee AND an arm folds via the one-shot refold"
  (doc
    "A perform in BOTH a `match` scrutinee AND an arm body composes the one-shot refold with match
           distribution. `(match (Amb.flip) (0 5) (_ (+ 1 (Amb.flip))))` — the refold takes the SCRUTINEE
           flip as the leading hole `C = (match [] (0 5) (_ (+ 1 (Amb.flip))))`; `(resume 10 s)` re-reduces
           `C[10] = (match 10 (0 5) (_ (+ 1 (Amb.flip))))`, whose scrutinee is now the pure constant 10, so
           distribution fires over the arm perform: the `_` arm `(+ 1 (Amb.flip))` folds to `(+ 1 (+ 1 10))`
           = 12 and is selected → 12; the outer arm `(+ 1 (resume 10 s))` = `(+ 1 12)` = 13. One-shot, so no
           effect is duplicated.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          Amb
          0
          ((flip (u) s (+ 1 (resume 10 s))))
          (match (Amb.flip) (0 5) (_ (+ 1 (Amb.flip))))))
      (export main)))
  (output (: 13 Int64)))

(case
  "a MULTI-shot arm over a distributed match arm-body hole folds"
  (doc
    "A MULTI-shot arm (resumes twice) over a match whose selected arm body performs: each distributed
           arm's sub-handle duplicates its pure continuation safely. `(match 1 (0 5) (_ (Amb.flip)))` — the
           scrutinee 1 selects the `_` arm, an identity slice `(handle … (Amb.flip))`, so `(resume 1 s)` = 1
           and `(resume 2 s)` = 2; the arm `(* (resume 1 s) (resume 2 s))` = `(* 1 2)` = 2.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (* (resume 1 s) (resume 2 s)))) (match 1 (0 5) (_ (Amb.flip)))))
      (export main)))
  (output (: 2 Int64)))

; --- A handler folds state across the operations it discharges ----------------------------------
; capabilities-and-effects.md #A Handler Threads State Across The Operations It Discharges. Every handle
; seeds an initial state; each arm binds the current state and resume threads the next state forward; the
; handle evaluates to its body's value. These cases witness the fold with a genuine (non-unit) accumulator —
; a scalar counter and a growing list — and show that reading the accumulated state out is an ordinary
; operation, not a separate result form. This is the state model a self-hosting compiler is authored
; against: a fresh-name supply (a counter) and diagnostic accumulation (a list).
(case
  "a handler folds a counter across the operations it discharges"
  (doc
    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges: `Fresh` declares `next : Unit -> Int64` — a fresh-name supply, ONE intention
           'read the current value and advance'. The handler is seeded with 0 at the handle site (the
           initial state is explicit, not ambient), and its arm `(Fresh.next (u) s (resume s (+ s 1)))`
           hands back the current state `s` as the operation's value and threads `s + 1` forward as the
           next state. Three performs therefore see 0, 1, 2, and the `do` yields the last, 2. The handle
           evaluates to the value of its body — the final counter 3 is NOT part of the result, because the
           body never reads it. Contrast a stateless handler (seed unit, thread s unchanged): this one
           genuinely folds. This upgrades the fresh-name idiom from a pure function of its argument to a
           real supply — the compiler's `Fresh` state model.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (Fresh.next) (Fresh.next) (Fresh.next))))
      (export main)))
  (output (: 2 Int64)))

; Every handle seed in the cases above is a CONSTANT. A seed may be a RUNTIME value — a caller
; argument flowing into the handler's initial state — and the fold must genuinely START from it (a
; seed baked at compile time, or a let-bound handle whose runtime seed was mishandled by the fold,
; produces a value independent of the argument). Two calls with different seeds witness the
; dependence.
(case
  "a HEAP handler seed stays readable in the body after performs advance the state"
  (doc
    "The ALIAS face of a heap-valued handler seed: `seed` (a let-bound list) is BOTH the handler's
           initial state — which two performs then advance via `List.push` — AND a binding the body
           re-reads AFTER those performs. The state hand-off at the handler boundary must DUP the seed,
           not take it uniquely: a reuse that treated the seed as dead after seeding would let the
           state's pushes clobber the shared payload, and the body's `(List.at seed 0)` would read a
           pushed value instead of the original k. resume values are the PRE-push lengths (1, 2), so
           a = 1, b = 2, and the re-read gives k = 5 → 1 + 2 + 500 = 503. The heap-STATE pins nearby
           thread list/record/set states; the runtime-seed pins use scalars — this is the heap-seed
           aliased-and-re-read composition neither covers.")
  (input
    (do
      (effect Acc (op push (-> Int64 Int64)))
      (def
        (main (: k Int64))
        (let
          ((seed #list(k)))
          (handle
            Acc
            seed
            ((push (v) s (resume (List.len s) (List.push s v))))
            (let
              ((a (Acc.push 10)))
              (let
                ((b (Acc.push 20)))
                (+ (+ a b) (* 100 (match (List.at seed 0) ((Some v) v) ((None _u) -1)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 503 Int64)))

(case
  "a handle seeded from a runtime caller argument advances from that seed"
  (doc
    "`(handle Ctr seed …)` where `seed` is main's PARAMETER — the handler's initial state is a
           runtime value, not a compile-time constant. Two ticks encode 100·first + second: seeded 7 →
           7, 8 → 708; seeded 50 → 50, 51 → 5051. The two calls returning seed-dependent values pin
           that the fold starts from the LIVE argument (a compile-time-baked seed, or a state slot
           initialized before the argument arrives, returns the same value for both calls). The
           runtime-seed companion of the constant-seeded counter fold above.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: seed Int64))
        (handle Ctr seed ((tick (u) s (resume s (+ s 1)))) (+ (* 100 (Ctr.tick)) (Ctr.tick))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 708 Int64))
  (call main (: 50 Int64))
  (output (: 5051 Int64)))

(case
  "a let-bound handle with a runtime seed composes with arithmetic after the let"
  (doc
    "The let-bound face the runtime-seed fold fix targets: `(let ((r (handle Get seed … (+
           (Get.get) 1)))) (* r 2))` — the handle's value is bound, then consumed by later arithmetic.
           Seeded 20 the perform reads 20, the body yields 21, and the doubled result is 42. Pins that
           a let-bound handle whose seed is a caller runtime arg folds cleanly into the enclosing
           computation (the handle is not the def's tail, so its fold must compose, not just
           terminate). Expected: 42.")
  (input
    (do
      (effect Get (op get (-> Unit Int64)))
      (def
        (main (: seed Int64))
        (let ((r (handle Get seed ((get (u) s (resume s s))) (+ (Get.get) 1)))) (* r 2)))
      (export main)))
  (call main (: 20 Int64))
  (output (: 42 Int64)))

; The counter fold above SEQUENCES its performs with `do` — each perform is a separate statement, so
; the state advance is witnessed only through the last value. These pin the fold where the advancing
; state is observed by ORDER-SENSITIVE operand positions instead: two performs as SIBLING operands of
; one arithmetic expression. The values differ per site (the counter advances between them), so the
; operand evaluation ORDER is observable — an emit that evaluated the right operand first, or batched
; the two performs against one state read, would produce a different value, not just a different trace.
(case
  "a stateful counter is observed left-to-right by sibling performs in one expression"
  (doc
    "`(+ (* 100 (Fresh.next)) (Fresh.next))` under the counter arm seeded 0: the LEFT perform reads
           0 and advances to 1, the RIGHT reads 1 → 0·100 + 1 = 1. The `*100` weighting makes the order
           observable in the VALUE: right-first evaluation would give 1·100 + 0 = 100. The sibling-operand
           companion of the `do`-sequenced counter fold above — same arm, but the state advance is
           witnessed by strict left-to-right operand evaluation inside a single expression, the order
           #Operands Evaluate Left To Right fixes. Expected: 1.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (+ (* 100 (Fresh.next)) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

(case
  "sibling performs feeding a subtraction witness the advancing state non-commutatively"
  (doc
    "`(- (Fresh.next) (Fresh.next))` seeded 5: left reads 5, right reads 6 → 5 − 6 = −1. The
           non-commutative twin of the weighted-add case above — subtraction needs no weighting to expose
           a swapped order (it would flip the sign to +1), so this is the minimal order witness over an
           advancing handler state. Expected: -1.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def (main) (handle Fresh 5 ((next (u) s (resume s (+ s 1)))) (- (Fresh.next) (Fresh.next))))
      (export main)))
  (output (: -1 Int64)))

(case
  "a stateful counter threads through a RECURSIVE callee performing inside the handled region"
  (doc
    "`drain` is a self-recursive function performing `(Fresh.next)` once per level, called from the
           handle body with a RUNTIME depth: seeded 10 at n=3 the three activations read 10, 11, 12 →
           10+11+12 = 33; n=0 performs nothing → 0. The stateful-fold companion of the delegation-reaches-
           a-recursive-callee capability case (04-capabilities): there the effect is DELEGATED to the
           entrypoint, here it is DISCHARGED by an in-program handler whose state must thread OUT of one
           recursive activation and INTO the next — across call frames, not just across statements in one
           body. An emit that re-seeded the handler per activation (3×10=30) or read one stale state for
           all levels would miscount. Runtime `n` keeps the recursion out of the fold. Expected: 33 (n=3),
           0 (n=0).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def (drain (: n Int64)) (if (<= n 0) 0 (+ (Fresh.next) (drain (- n 1)))))
      (def (main (: n Int64)) (handle Fresh 10 ((next (u) s (resume s (+ s 1)))) (drain n)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 33 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a MODULE-exported recursive callee performing per step is homed by the importer's handler"
  (doc
    "The MODULE-EXPORT face of the recursive-callee-performing case above: `walk` is a self-recursive
           performer of `Ctr.next`, but it lives INSIDE `(module m …)` and is called through the projection
           `(. m walk)` from the importer's handle body. The handler-context monomorphization must reach the
           module-exported recursive callee — re-homing its per-step perform (and its recursive self-calls)
           under the importer's handler — exactly as it does for a bare-named recursive performer (case
           above). Seeded 1, main(3) reads 1,2,3 as `((10·acc)+next)` → 123. Previously DECLINED (`no
           enclosing handler here`): the effect-reduction's `callee_def_index_of` followed `Ref` but not
           `Resolved::Member`, so a module-qualified recursive callee was never specialized under the handler
           (the module × recursion × effect-context-mono composition gap). Fixed by following the `Member`
           projection there, mirroring `lower::callee_def_index`. (breaker mo1 witness.)")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (module m
        (def
          (walk (: n Int64) (: acc Int64))
          (if (= n 0) acc (walk (- n 1) (+ (* 10 acc) (Ctr.next unit)))))

        (export walk))
      (def (main (: k Int64)) (handle Ctr 1 ((next (u) s (resume s (+ s 1)))) (m.walk k 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 123 Int64)))

(case
  "a MODULE-exported NON-recursive performer is homed by the importer's handler (single perform)"
  (doc
    "The base-case sibling of the recursive module-performer above: `once` is a module-exported
           NON-recursive fn performing `Ctr.next` ONCE, called via `(. m once)` from the importer's handle
           body. This single-perform module case ALREADY worked (a non-recursive module callee inlines into
           the handler context at its one call site) — pinning it guards the module-member call → handler-
           homing path that the recursive fix's `callee_def_index_of` Member arm also serves, so a future
           change there can't silently regress the non-recursive module perform. Seeded 5, main(5) reads 5 →
           100+5 = 105. (breaker mo3 bisect witness.)")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (module m
        (def (once (: k Int64)) (+ k (Ctr.next unit)))

        (export once))
      (def (main (: n Int64)) (handle Ctr n ((next (u) s (resume s (+ s 1)))) (m.once 100)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "MUTUALLY-recursive MODULE-exported performers are both homed by the importer's handler"
  (doc
    "The mutual-recursion escalation of the module-performer fix: `ping`/`pong` are TWO module-exported
           functions that CROSS-CALL each other, each performing `Ctr.next` per step, entered via `(. m ping)`
           under the importer's handler. Both must home under that handler — so the effect-context reduction
           must resolve BOTH module-qualified recursive callees (through the `Member` projection) and
           specialize the mutual-recursion SCC as a unit. This is the deeper face of the single-recursive
           module case: the `callee_def_index_of` Member arm covers it because BOTH `ping` and `pong` resolve
           through the same Member path and the existing mutual-recursion specialization then handles the
           cross-calls — no separate SCC machinery needed. Seeded 1, the per-run cursor yields 1,2,3 across
           the three activations (ping→pong→ping); pong doubles its tick, so main(3) = 143. (breaker mm1
           escalation witness — flips together with mo1, confirming the fix generalizes past single
           self-recursion to the mutual-recursion SCC.)")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (module m
        (def
          (ping (: n Int64) (: acc Int64))
          (if (= n 0) acc (pong (- n 1) (+ (* 10 acc) (Ctr.next unit)))))

        (def
          (pong (: n Int64) (: acc Int64))
          (if (= n 0) acc (ping (- n 1) (+ (* 10 acc) (* 2 (Ctr.next unit))))))

        (export ping)

        (export pong))
      (def (main (: k Int64)) (handle Ctr 1 ((next (u) s (resume s (+ s 1)))) (m.ping k 0)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 143 Int64)))

(case
  "a module-exported recursive performer called from a HANDLER ARM homes under the outer handler"
  (doc
    "Composition escalation of the module-performer fix (breaker mo4): the module recursive performer
           `(. m walk)` is invoked NOT from the handle body directly but from INSIDE another effect's handler
           ARM — `(handle Ask 0 ((get (u) s (resume ((. m walk) k 0) s))) …)` nested under `(handle Ctr …)`.
           The `Ctr` performs inside `walk` must still home under the OUTER `Ctr` handler even though the
           module call originates in the `Ask` arm's resume expression. Confirms the Member-arm reduction
           reaches a module callee through an arm-nested call site, not just a handle-body one. Seeded 10,
           main(3) sums 10+11+12 = 33.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (effect Ask (op get (-> Unit Int64)))
      (module m
        (def
          (walk (: n Int64) (: acc Int64))
          (if (= n 0) acc (walk (- n 1) (+ acc (Ctr.next unit)))))

        (export walk))
      (def
        (main (: k Int64))
        (handle
          Ctr
          10
          ((next (u) s (resume s (+ s 1))))
          (handle Ask 0 ((get (u) s (resume (m.walk k 0) s))) (Ask.get))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 33 Int64)))

(case
  "TWO modules' recursive performers interleave under ONE handler's shared state"
  (doc
    "Cross-module state-continuity escalation (breaker mo5): TWO separate modules `ma`/`mb` each export
           a recursive performer of `Ctr.next`, both entered under ONE `Ctr` handler; the handler's per-run
           state must thread continuously ACROSS the module boundary — `wa`'s activations consume the first
           rows, then `wb`'s consume the next (mb scales its tick ×100). The Member-arm reduction must
           specialize BOTH modules' recursive callees under the same handler and keep one shared cursor. At
           k=2, seeded 1: wa reads 1,2 (→3), wb reads 3,4 ×100 (→700), sum 703.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (module ma
        (def (wa (: n Int64) (: acc Int64)) (if (= n 0) acc (wa (- n 1) (+ acc (Ctr.next unit)))))

        (export wa))
      (module mb
        (def
          (wb (: n Int64) (: acc Int64))
          (if (= n 0) acc (wb (- n 1) (+ acc (* 100 (Ctr.next unit))))))

        (export wb))
      (def
        (main (: k Int64))
        (handle Ctr 1 ((next (u) s (resume s (+ s 1)))) (+ (ma.wa k 0) (mb.wb k 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 703 Int64)))

(case
  "a performed operation is the scrutinee of a match that dispatches on its result"
  (doc
    "Witnesses that an effect operation composes as a match SCRUTINEE, exactly as it composes as an
           `if` condition or an arithmetic operand: `(match (Fresh.next) (0 100) (_ 200))`. The scrutinee is
           evaluated FIRST — `Fresh.next` reads the current counter (seeded 0), hands it back as the
           operation's value, and threads `s + 1` forward — then the match dispatches on that value. Seeded
           0, the first read is 0, so the `0` arm is selected and the handle yields 100. The state threads
           through the scrutinee before the arm bodies run, the same evaluation order any strict operand
           sees; the pattern engine then lowers the (rewritten) match by its ordinary path.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (match (Fresh.next) (0 100) (_ 200))))
      (export main)))
  (output (: 100 Int64)))

; The performed-scrutinee cases dispatch on the effect's result but keep the arm BODIES pure — so a
; state slot corrupted by the match lowering would go unobserved. These thread state through BOTH
; halves: the scrutinee performs (advancing the state), the match dispatches, and the SELECTED arm's
; body performs again and must read the post-scrutinee state.
(case
  "performs in match-arm bodies fire ONLY for the selected arm — counter witnesses the count"
  (doc
    "The untaken-arm face (the neighbors pin the taken arm's state read): three arms carry ZERO,
           ONE, and TWO performs respectively, and a FINAL perform reads the counter — so the result
           encodes exactly how many arm performs fired. n=0 (one-perform arm): 0 + 10·1 = 10. n=1
           (two-perform arm): (0+1) + 10·2 = 21. n=5 (zero-perform arm): 100 + 10·0 = 100. An emit that
           hoisted an arm's perform above the dispatch (or speculatively evaluated an untaken arm)
           drifts the counter at one of the three calls — the differential-count witness a single-arm
           case cannot give.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Ctr
          0
          ((next (u) s (resume s (+ s 1))))
          (+
            (match n (0 (Ctr.next unit)) (1 (+ (Ctr.next unit) (Ctr.next unit))) (_ 100))
            (* 10 (Ctr.next unit)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64))
  (call main (: 1 Int64))
  (output (: 21 Int64))
  (call main (: 5 Int64))
  (output (: 100 Int64)))

(case
  "a perform in the scrutinee fires exactly ONCE whichever of three arms is selected"
  (doc
    "The once-only guarantee at arm-count 3: the scrutinee's `(Ctr.next unit)` advances the state
           exactly once, the dispatch selects among three arms on its VALUE, and a second perform reads
           the post-scrutinee state — seed 0 → 0 dispatches arm-0, then reads 1 (1001); seed 1 → 2002;
           seed 7 → wildcard, 3008. A dispatch that re-evaluated the scrutinee per arm test (three
           probes = three performs) would read a drifted counter in the tail perform.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Ctr
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (match (Ctr.next unit) (0 1000) (1 2000) (_ 3000)) (Ctr.next unit))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1001 Int64))
  (call main (: 1 Int64))
  (output (: 2002 Int64))
  (call main (: 7 Int64))
  (output (: 3008 Int64)))

(case
  "a matched arm body performs and reads the state the scrutinee advanced"
  (doc
    "Seeded 5, the scrutinee `(Ctr.tick)` reads 5 (state → 6) and hits the literal-5 arm, whose
           BODY performs again: the second tick must read 6 — the state the scrutinee's discharge left —
           not the seed re-read (105) or a per-arm re-seed. 100 + 6 = 106. The arm-body companion of the
           performed-scrutinee case above (whose arms are pure). Expected: 106.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (match (Ctr.tick) (5 (+ 100 (Ctr.tick))) (_ -1))))
      (export main)))
  (output (: 106 Int64)))

(case
  "a fall-through arm body performs and reads the post-scrutinee state"
  (doc
    "The wildcard twin: seeded 9, the scrutinee reads 9 (state → 10) and MISSES the literal-5 arm;
           the fall-through arm's body performs and reads 10. Pins that the state threads to WHICHEVER
           arm is selected — the dispatch (hit or miss) does not fork or reset the handler state slot.
           Expected: 10.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle Ctr 9 ((tick (u) s (resume s (+ s 1)))) (match (Ctr.tick) (5 -1) (_ (Ctr.tick)))))
      (export main)))
  (output (: 10 Int64)))

(case
  "a performed operation whose result is a sum is matched on its variant"
  (doc
    "Extends the match-scrutinee composition to an operation whose declared RESULT is a SUM type — the
           resume value is a compound sum, not a scalar. `Look.find : Int64 -> (Option Int64)`; the arm
           resumes with `(Some (+ k 1))`, a constructed `Option` value carrying the incremented key. The
           handle body `(match (Look.find 41) ((Some v) v) (None 0))` performs, folds the resume value into
           the scrutinee position, and dispatches on its variant: `Look.find 41` yields `(Some 42)`, the
           `(Some v)` arm binds `v = 42`. Pins that the pure one-hole fold substitutes a SUM-typed resume
           value soundly (the value column carries the compound through the match), the effect analogue of a
           sum-typed handler return — a compiler pass performing a lookup that returns an optional result.")
  (input
    (do
      (effect Look (op find (-> Int64 (Option Int64))))
      (def
        (main)
        (handle
          Look
          0
          ((find (k) s (resume (Some (+ k 1)) s)))
          (match (Look.find 41) ((Some v) v) (None 0))))
      (export main)))
  (output (: 42 Int64)))

(case
  "an effect op whose declared RESULT is a QUANTITY resumes with a Qty value"
  (doc
    "An operation whose declared result is a QUANTITY type `(Qty T u)` — the resume value is a
           unit-carrying `Qty`, not a bare scalar. `Env.width : Unit -> (Qty Int64 meter)`; the arm resumes
           `(Qty.of 5 (Unit.base #\"meter\"))`, and the body reads the magnitude back with `Qty.value` → 5.
           Pins that a Qty-typed operation result flows through the effect machinery: the op's `(meta t)` arrow
           `(-> Unit (Qty Int64 meter))` must reduce to a determined `Ty::Qty` RESULT (the scheme path
           `type_in_env` gained a `QtyCtor` arm; without it the arrow collapsed and the result read as the raw
           op-value record → CDZ0203 'not fully determined'). This is the guest-side of the runtime-parameter
           `@param` Quantity path — a `@param(...) width : Length` generates exactly this Qty-result op (the
           host-boundary num/den ABI for it is a separate later increment; here it is discharged by an
           in-program handler). Identical on both backends.")
  (input
    (do
      (effect Env (op width (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main)
        (handle
          Env
          unit
          ((width (u) s (resume (Qty.of 5 (Unit.base #"meter")) s)))
          (Qty.value (Env.width))))
      (export main)))
  (output (: 5 Int64)))

(case
  "a unit MISMATCH in the resume value is rejected — the arm cannot resume a different unit"
  (doc
    "The NEGATIVE twin of the Qty-result pin above: the op declares `(Qty Int64 meter)` but the arm
           resumes a SECOND-typed quantity → CDZ0201. The compile-time unit discipline (units-of-measure's
           no-solver contract) must hold through the resume crossing — a marshalling path that erased the
           unit to a raw scalar at the boundary would let the wrong dimension through silently. The reject
           is at the RESULT position (0201), the resume-side twin of the arg-side reject below.")
  (input
    (do
      (effect St (op read (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((read (u) s (resume (Qty.of n (Unit.base #"second")) s)))
          (Qty.value (St.read))))
      (export main)))
  (error CDZ0201))

(case
  "irj1 an if-JOIN whose arms resume with MISMATCHED value types is rejected per-arm CDZ0201 (not escaped to InvalidWasm)"
  (doc
    "The resume-value/result-type check (CDZ0201) is applied PER-ARM through an if/match-join, not
           only at a top-level resume. `(if (<= v 8) (resume 1.5 v) (resume 1 v))` — one arm resumes a
           Float64, the other an Int64, while the op result is Int64. A SINGLE `(resume 1.5 v)` is correctly
           CDZ0201; before this fix the join was an `If` (not a `Resume`), so the check was SKIPPED and the
           Float resume reached emit as an INVALID module (v-cdz-smith fuzzer bucket, routed by v-inference).
           Now the check collects every tail resume value through the join and faults the mismatched arm.")
  (input
    (do
      (effect E (op ask (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle E 0 ((ask (v) s (if (<= v 8) (resume 1.5 v) (resume 1 v)))) (E.ask n)))
      (export main)))
  (error CDZ0201))

(case
  "irj2 an if-JOIN whose arms resume with MISMATCHED next-STATE types is rejected per-arm CDZ0201"
  (doc
    "The next-state twin of irj1: the seed fixes the state type (Int64 here), and each resume must
           thread that type. `(if (<= v 8) (resume 1 \"x\") (resume 1 v))` — one arm's next-state is a
           String, the other Int64 — escaped the same way (the join is an `If`, not a `Resume`), threading
           a wrong-typed state mid-fold. The per-arm collection now faults it CDZ0201 at the String
           next-state.")
  (input
    (do
      (effect E (op ask (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle E 0 ((ask (v) s (if (<= v 8) (resume 1 "x") (resume 1 v)))) (E.ask n)))
      (export main)))
  (error CDZ0201))

(case
  "irj3 a VALID if-JOIN where both arms resume the op-result type folds — the per-arm check does not over-reject"
  (doc
    "The positive control for irj1/irj2: both arms resume an Int64 value AND an Int64 next-state under
           an Int64 seed, so the per-arm resume-type check passes and the handler folds. main(5): v=5<=8 so
           the taken arm resumes 2 → the op result 2.")
  (input
    (do
      (effect E (op ask (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle E 0 ((ask (v) s (if (<= v 8) (resume 2 v) (resume 1 v)))) (E.ask n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "a unit MISMATCH in the op argument is rejected — the program cannot perform with a different unit"
  (doc
    "The op-ARG direction of the unit-safety pair: the op takes `(Qty Int64 meter)` but the program
           performs with a SECOND-typed quantity → CDZ0203 (the ARGUMENT-position code, vs the resume-side
           0201 above — the same result-vs-arg code split as ordinary typing). Neither effect-boundary
           direction erases units: the dimension is part of the op's contract both ways.")
  (input
    (do
      (effect St (op put (-> (Qty Int64 (Unit.base #"meter")) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((put (q) s (resume (Qty.value q) s)))
          (St.put (Qty.of n (Unit.base #"second")))))
      (export main)))
  (error CDZ0203))

(case
  "a narrow-width overflow in a handler arm resume value under a narrow annotation is rejected"
  (doc
    "The EFFECTS face of the width fit-check: the whole `handle` sits under a `UInt8` annotation, so
           the narrow width must propagate through the handle's result — the op's resume site — into the
           arm's resume VALUE, where the runtime-conditional branch literal `10000` overflows (0..=255) →
           CDZ0302. The resume value is the width descent's longest path yet: annotation → handle body's op
           result → arm resume → runtime `if` branch literal. Without it the overflow would slip into the
           resumed value exactly as the plain-`if` gap did. The fitting twin below computes.")
  (input
    (do
      (effect Pick (op get (-> Unit Int64)))
      (def
        (main (: c Bool))
        (: (handle Pick 0 ((get (u) s (resume (if c 10000 5) s))) (Pick.get unit)) UInt8))
      (export main)))
  (error CDZ0302))

(case
  "a fitting handler arm resume value computes under a narrow annotation"
  (doc
    "The no-over-reject control for the effects width face: the same handle shape resuming `(if c 100
           5)` — both branch literals fit UInt8 — computes 100/5 per the runtime condition, at UInt8
           end-to-end. Guards the resume-value width descent against rejecting every narrow-annotated
           handle.")
  (input
    (do
      (effect Pick (op get (-> Unit Int64)))
      (def
        (main (: c Bool))
        (: (handle Pick 0 ((get (u) s (resume (if c 100 5) s))) (Pick.get unit)) UInt8))
      (export main)))
  (call main (: true Bool))
  (output (: 100 UInt8))
  (call main (: false Bool))
  (output (: 5 UInt8)))

(case
  "a NARROW-width effect op parameter grounds a fitting perform argument"
  (doc
    "An op declared over a NARROW parameter — `Send.put : UInt8 -> Int64` — performed with a fitting
           literal `(Send.put 100)`: the op's declared parameter type grounds the argument (the effect-op
           analogue of the narrow function parameter), the perform crosses to the arm, and the arm resumes
           7 → 7. Pins the narrow-width op-argument path on its FITTING side. (The overflowing twin
           `(Send.put 999)` — expected CDZ0302 like every other narrow-parameter position — currently
           DECLINES rather than rejecting, and an arm that READS the binder `v` also declines: the
           effect-op width descent and the narrow-binder arm read are coverage-not-yet; their pins join
           this one when those land.)")
  (input
    (do
      (effect Send (op put (-> UInt8 Int64)))
      (def (main (: n Int64)) (handle Send 0 ((put (v) s (resume 7 s))) (Send.put 100)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a runtime Bool host arg crosses the boundary and drives two response bindings"
  (doc
    "The Bool scalar-ARG host-boundary face (v-rust-backend #1708 closed the rust arm: a bool
           marshals to i64 via i64::from, matching wasm's i32 rep). Two host calls each take a runtime
           bool computed from a comparison — `io.check (> n 5)` then `io.check (< n 5)` at n=7 → true then
           false — and each response (10, 20) sums to 30. Pins the bool arg crosses AND that two ops
           consume their rows in order. (breaker bh1, verified past its #1708 witness.) wasm + rust pass;
           rust-async todo pending its host-delegated op-arg path.")
  (input
    (do
      (effect io (op check (-> Bool Int64)))
      (def (main (: n Int64)) (host (io) (+ (io.check (> n 5)) (io.check (< n 5)))))
      (export main)))
  (host-responses (respond io.check (: 10 Int64)) (respond io.check (: 20 Int64)))
  (host-calls (call io.check) (call io.check))
  (call main (: 7 Int64))
  (output (: 30 Int64)))

(case
  "a Bool host arg BESIDE a scalar and a String composes in one mixed-arity op"
  (doc
    "The mixed-arity composition face: one op `(-> Bool Int64 String Int64)` takes a bool, a
           scalar, AND a string arg together — the Bool marshal (#1708) composing with the existing
           scalar and String-arg arms in a single arg list (the multi-arg slot-threading the
           host-arg-before-scalar fix hardened). `io.log (= n 3) n \"tag\"` at n=3 → host answers 42.
           (breaker bh2, verified.) wasm + rust pass; rust-async todo.")
  (input
    (do
      (effect io (op log (-> Bool Int64 String Int64)))
      (def (main (: n Int64)) (host (io) (io.log (= n 3) n "tag")))
      (export main)))
  (host-responses (respond io.log (: 42 Int64)))
  (host-calls (call io.log))
  (call main (: 3 Int64))
  (output (: 42 Int64)))

(case
  "a RECURSIVE-sum value of runtime depth rides a handler resume"
  (doc
    "An op whose declared result is a RECURSIVE sum (`Give.get : Unit -> Nat`) resumed with a
           runtime-depth spine `(mk a)`: the resume value is an unbounded heap structure, not a scalar or
           fixed-shape compound, and the body folds it back to its depth — 3 at `a = 3`, 0 at `a = 0`.
           Pins that the resume path carries a recursive sum intact through the handler machinery (the
           unbounded-depth companion of the Qty/Result resume-value cases).")
  (input
    (do
      (type Nat (Z) (S Nat))
      (effect Give (op get (-> Unit Nat)))
      (def (mk (: n Int64)) (if (= n 0) (Z) (S (mk (- n 1)))))
      (def (depth (: v Nat)) (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
      (def
        (main (: a Int64))
        (depth (handle Give 0 ((get (u) s (resume (mk a) s))) (Give.get unit))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (live-objects known-leak))

(case
  "a handler STATE that is a recursive sum GROWS one constructor per operation"
  (doc
    "The recursive-sum face of heap-valued handler state (list/record/set/string states are pinned;
           this state's SHAPE deepens per op): seeded `(Z)`, each `Acc.bump` arm resumes with next-state
           `(S s)` — wrapping the CURRENT state one level deeper — and `Acc.read` folds the accumulated
           spine to its depth. Two bumps then a read → 2. Pins that the threaded state may be a recursive
           sum whose depth is the operation COUNT (state evolution changes the value's structure, not just
           its contents), composing the state-threading discipline with unbounded recursive values.")
  (input
    (do
      (type Nat (Z) (S Nat))
      (effect Acc (op bump (-> Unit Int64)) (op read (-> Unit Int64)))
      (def (depth (: v Nat)) (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
      (def
        (main (: a Int64))
        (handle
          Acc
          (Z)
          ((bump (u) s (resume 0 (S s))) (read (u) s (resume (depth s) s)))
          (do (Acc.bump unit) (Acc.bump unit) (Acc.read unit))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "a LET-bound outer perform under an inner handle threads its state advance"
  (doc
    "An OUTER-handled effect performed INSIDE an inner (different-effect) handle, with the perform's
           value LET-BOUND before the next operation: `A.bump` (threads 0→1) let-bound, then `A.get` reads
           1 — the state advance crosses the inner `B` handler level intact. Pins the cross-level state
           threading for the value-consumed sequencing form. (The DO-discarded twin of this shape — `(do
           (A.bump unit) (A.get unit))` under the inner handle — currently DROPS the advance, a filed
           lowering bug; when it is fixed its case joins this one, and this pin guards the form that must
           keep working.)")
  (input
    (do
      (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op noop (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          0
          ((bump (u) s (resume 0 (+ s 1))) (get (u) s (resume s s)))
          (handle B 100 ((noop (u) t (resume t t))) (let ((x (A.bump unit))) (A.get unit)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a do-sequenced perform train under its OWN inner handle threads state"
  (doc
    "The inner-effect control for the cross-level threading: the do-sequenced bump/get train targets
           the INNER handler itself (`B`, seeded 100) while an outer `A` handler wraps it — `B.bump`
           (100→101 discarded) then `B.get` reads 101. Pins that do-sequencing is sound when the performs
           discharge at the NEAREST handler; combined with the let-bound case above it brackets the filed
           do-discarded CROSS-level state-drop precisely (same sequencing one level down: works; same
           crossing with let: works; do + crossing: the bug).")
  (input
    (do
      (effect A (op noop (-> Unit Int64)))
      (effect B (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          0
          ((noop (u) s (resume s s)))
          (handle
            B
            100
            ((bump (u) t (resume 0 (+ t 1))) (get (u) t (resume t t)))
            (do (B.bump unit) (B.get unit)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 101 Int64)))

(case
  "a do-discarded OUTER perform under an inner handle threads its state advance across the level"
  (doc
    "The FIXED cross-level case the two cases above bracket: a do-sequenced perform of an OUTER-handled
           effect, its value DISCARDED in a `(do …)` and crossing an INNER handler of a DIFFERENT effect,
           threads its state advance out to the outer handler. `A.bump` (0→1) is do-discarded under the
           inner `B` handle, then `A.get` reads 1 — NOT the stale seed 0. This was a silent wrong-value
           miscompile on all backends (`thread_bounded`'s `do` fold collapsed the sequence to only the last
           item, erasing the non-final FOREIGN perform the inner handler does not discharge); the fix
           preserves a non-final item still reaching a foreign perform. Completes the bracket: sequencing one
           level down works (train case), crossing with a let works (let-bound case), and NOW do + crossing
           works too — the discarded-value form is no longer a state-drop.")
  (input
    (do
      (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op noop (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          0
          ((bump (u) s (resume 0 (+ s 1))) (get (u) s (resume s s)))
          (handle B 100 ((noop (u) t (resume t t))) (do (A.bump unit) (A.get unit)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "do-discarded outer performs inside a HELPER under an inner handle thread across the level"
  (doc
    "The cross-function-inline face of the do-discarded outer perform above: the discarded `A.bump`s
           live in a HELPER `step` (`(do (A.bump) (A.bump) (A.get))`) called in the inner `B` handle body,
           rather than written inline. When `step` inlines, its non-final foreign performs must survive the
           `do` collapse exactly as the inline form does — two bumps advance A's outer state 0->1->2, then
           `A.get` reads 2. A fold that dropped the non-final foreign performs on the inline path would read
           the stale seed. Pins the do-discard threading composes with a cross-fn inline.")
  (input
    (do
      (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op noop (-> Unit Int64)))
      (def (step (: u Unit)) (do (A.bump unit) (A.bump unit) (A.get unit)))
      (def
        (main)
        (handle
          A
          0
          ((bump (u) s (resume 0 (+ s 1))) (get (u) s (resume s s)))
          (handle B 100 ((noop (u) t (resume t t))) (step unit))))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "interleaved do-discarded performs at BOTH nest levels thread their own states independently"
  (doc
    "The composition the single-effect fix case above doesn't reach: TWO counters at different nest
           levels, both advanced by DO-DISCARDED performs, interleaved in one `(do …)` — outer, inner,
           outer again — then read via a final `(+ outer-get inner-get)`. CountA (outer, seed 0, +1 per
           bump) is bumped twice; CountB (inner, seed 100, +10 per bump) once; expected 2 + 110 = 112.
           Each discarded perform crosses (or doesn't) the inner handler per ITS effect: the A bumps are
           foreign to the inner handle and must survive its do-fold, the B bump is discharged locally, and
           neither may clobber the other's threaded slot. The fixed collapse dropped exactly this class of
           non-final foreign perform; a partial fix that preserved only ONE crossing (or merged the two
           state slots) lands off by a bump-width at one counter.")
  (input
    (do
      (effect CountA (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect CountB (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          CountA
          0
          ((bump (u) s (resume 0 (+ s 1))) (get (u) s (resume s s)))
          (handle
            CountB
            100
            ((bump (u) t (resume 0 (+ t 10))) (get (u) t (resume t t)))
            (do
              (CountA.bump unit)
              (CountB.bump unit)
              (CountA.bump unit)
              (+ (CountA.get unit) (CountB.get unit))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 112 Int64)))

(case
  "an unread op-argument that performs a foreign effect still threads that perform's state advance"
  (doc
    "The op-ARGUMENT-position face of the do-discarded foreign-perform family above. `A.fire`'s arm
           IGNORES its parameter `v` (`(fire (v) s (resume 7 s))`), so a naive fold SUBSTITUTES the argument
           into the arm body and — since the body never reads `v` — DROPS it. But the argument is `(B.tick)`,
           a perform of the OUTER `B` effect: dropping it erases B's STATE ADVANCE (t -> t+1), so a LATER
           `(B.tick)` reads STALE state — a silent wrong value. B.tick is a DECLARED operation of B, so its
           advance is observable (capabilities-and-effects.md #A Handler Threads State), unlike a discarded
           PURE argument whose trap is unobserved and rightly elided (09-functions unused-parameter case).
           The fix let-lifts the unread foreign-performing argument so its perform runs EXACTLY ONCE for
           effect (the same `#cv` lift the twice-read foreign-arg case already uses). `(+ (A.fire (B.tick))
           (B.tick))` at n=3: first B.tick = 3 (t->4), A.fire ignores it and yields 7, second B.tick reads 4,
           so 7 + 4 = 11 (was 10 — the first advance lost). At n=0: 7 + 1 = 8 (was 7). The control where
           A.fire's arm READS `v` already threaded both ticks; this closes the UNREAD gap.")
  (input
    (do
      (effect A (op fire (-> Int64 Int64)))
      (effect B (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          n
          ((tick () t (resume t (+ t 1))))
          (handle A 0 ((fire (v) s (resume 7 s))) (+ (A.fire (B.tick)) (B.tick)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 11 Int64))
  (call main (: 0 Int64))
  (output (: 8 Int64)))

; The do-discarded family above concerns non-final FOREIGN PERFORMS (which the handle-body fold must
; preserve). Its complement: a discarded non-final PURE item in a host do-body is ELIDED per the
; §283 dead-init ruling (core-semantics.md §A Trap Occurs Only Where Its Computation Is Observed) —
; a discarded pure computation is unobserved, so its trap does not fire, EVEN when the do also makes a
; host call (the foreign-perform exception preserves performs, not pure siblings). adv-56: the rust
; backend's Core::Seq emit rendered every non-final statement as `let _ = <stmt>;`, which EVALUATES it,
; so a discarded pure `(/ 100 d)` beside an `io.put` host call spuriously trapped div-by-zero at d=0
; on rust — the fix (rcdzc, trunk fb078ac35) elides a discarded pure non-final Seq statement. wasm /
; rust-async currently DECLINE this host-delegated shape (todo, no cross-backend value split); the pin
; pass-grades rust (where the fix landed) and tracks the wasm/rust-async decline.
(case
  "a discarded pure trapping item in a host do-body is elided beside a host call (dead-init ruling)"
  (doc
    "`(host (io) (do (/ 100 d) (io.put 1) 42))` at d = 0: the non-final `(/ 100 d)` is a discarded
           PURE item — its value flows nowhere, so per the dead-init ruling it is unobserved and its
           divide-by-zero trap does NOT fire; the do makes a host call (`io.put 1`) and yields its last
           form, 42. The foreign-perform exception preserves the PERFORM (`io.put` still runs, host-call
           recorded), not the pure discarded sibling. Pins that the Core::Seq emit ELIDES a discarded pure
           non-final statement rather than force-evaluating it (adv-56 rust miscompile — `let _ = <stmt>;`
           ran the trap). The pure-only dead-init twin (02-binding-and-control) elides the same way with no
           host call; this is the host-call face. Rust + wasm pass (the wasm Core::Seq emit elides a
           non-host-reaching statement via the SAME `subtree_reaches_host_call` predicate CDZ0307 warns on);
           rust-async declines this host-delegated shape (todo).")
  (input
    (do
      (effect io (op put (-> Int64 Int64)))
      (def (main (: d Int64)) (host (io) (do (/ 100 d) (io.put 1) 42)))
      (export main)))
  (host-responses (respond io.put (: 0 Int64)))
  (host-calls (call io.put))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

(case
  "a value-leaving host-call statement in a do-body runs and its result is dropped (dead-init sibling)"
  (doc
    "The DROP face of the dead-init Core::Seq emit: `(do (io.put 1) (io.put 2) 42)` — the two non-final
           statements are VALUE-LEAVING host calls (`io.put : Int64 -> Int64`), not Unit. Each must RUN (both
           host calls fire, recorded in order) but its returned value is DISCARDED — the emit drops the
           leftover so the block stays stack-balanced and yields the tail, 42. Distinct from the pure-elide
           sibling above (which does NOT emit its statement at all): a host-reaching statement is always
           emitted; only a non-Unit RESULT is dropped. Pins the `Lir::Drop` arm the sibling's pure-elide path
           doesn't exercise. Rust + wasm pass; rust-async todo pending its host-delegated Seq emit.")
  (input
    (do
      (effect io (op put (-> Int64 Int64)))
      (def (main (: k Int64)) (host (io) (do (io.put 1) (io.put 2) 42)))
      (export main)))
  (host-responses (respond io.put (: 0 Int64)) (respond io.put (: 0 Int64)))
  (host-calls (call io.put) (call io.put))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

(case
  "a NOMINAL-Unit-typed host-call statement in a do-body is not spuriously dropped"
  (doc
    "The nominal-Unit edge of the Seq stmt-DROP arm (the sibling above): `(Done (io.fire k))` is a
           non-final statement that REACHES a host call AND is typed `Done` — a NEWTYPE over `Unit`
           (`(type Done (Done Unit))`). A nominal-Unit leaves NO machine value at the boundary just like a
           bare `Unit` (`valtype_of` is None), so it must NOT be dropped. The drop test must strip nominals
           (`type_of(..).strip_nominal() != Unit`) — WITHOUT that, `Done` ≠ `Ty::Unit` takes the drop branch
           and `Lir::Drop` underflows the empty stack → an invalid module (`wasm-tools: expected a type but
           nothing on stack`). io.fire still fires (recorded), the do yields io.get's response, 9. Mirrors
           the field-proj / tail-drop Unit checks that already strip_nominal. Rust + wasm pass; rust-async
           todo.")
  (input
    (do
      (type Done (Done Unit))
      (effect io (op fire (-> Int64 Unit)) (op get (-> Unit Int64)))
      (def (main (: k Int64)) (host (io) (do (Done (io.fire k)) (io.get unit))))
      (export main)))
  (host-responses (respond io.fire (: 0 Int64)) (respond io.get (: 9 Int64)))
  (host-calls (call io.fire) (call io.get))
  (call main (: 5 Int64))
  (output (: 9 Int64)))

; ── The effect-fold analogue of the dead-init elision family above (adv-56 / 09-functions unused-param) ──
; A handler's accumulated state is observable ONLY through the operations the effect declares
; (capabilities-and-effects.md #A Handler Threads State), so a resume NEXT-STATE that no later dispatch reads,
; and a handler SEED that no dispatch consumes, are UNOBSERVED — and an unread op ARGUMENT is the call-boundary
; analogue of the unused-parameter case. Per core-semantics.md #A Trap Occurs Only Where Its Computation Is
; Observed, an implementation MAY elide such an unobserved computation AND the trap it would raise. The three
; cases below pin that CURRENT lazy-effect-elision semantics: a trapping expression in an unconsumed
; next-state / unconsumed seed / unread op-arg is ELIDED, so the handle yields its value and does NOT trap.
; REVERSIBILITY (v-effects + concierge, 2026-08-11): these document the CURRENT conformant behavior; IF the
; operator later revises the spec to STRICT effect-evaluation (evaluate these positions for effect regardless
; of demand), these three flip to (trap "division by zero") and become the strict-fold witnesses — a
; cross-vertical spec-revision (spec text + these flips + 3-backend), NOT a unilateral compiler change. The
; CONSUMED twins already trap correctly (a 2nd dispatch that READS the poisoned state observes it); only the
; UNOBSERVED drop is elided. (breaker strict-fold #17 faces 1-3; face 4, the foreign-perform state-advance
; drop, WAS a genuine bug and is fixed on trunk — 5a0ceaf12.)
(case
  "an unconsumed resume next-state is unobserved, so its trapping expression is elided (lazy-effect)"
  (doc
    "The sole dispatch's resume threads a NEXT-STATE `(/ 100 (- s 4))` that no later dispatch reads —
           the identity arm returns the seed as the handle value. The next-state is unobserved
           (capabilities-and-effects.md #A Handler Threads State), so per core-semantics.md #A Trap Occurs Only
           Where Its Computation Is Observed the implementation MAY elide it and the trap it would raise at
           seed 4 (`100/0`). `main 4` = 4 (no trap), `main 6` = 6 (next-state 100/2=50 also elided). REVERSIBLE:
           documents CURRENT lazy semantics; flips to a trap under a future strict-effect spec revision.")
  (input
    (do
      (effect St (op step (-> Int64)))
      (def (main (: n Int64)) (handle St n ((step () s (resume s (/ 100 (- s 4))))) (St.step)))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64))
  (call main (: 6 Int64))
  (output (: 6 Int64)))

(case
  "an unconsumed handler seed is unobserved when the body never performs, so its trap is elided (lazy-effect)"
  (doc
    "The handle SEED `(/ 100 (- n 4))` is never consumed — the body `77` performs no operation, so no
           dispatch ever reads the seeded state. The seed is unobserved, so its trap at n = 4 (`100/0`) MAY be
           elided and the handle yields its body value 77. `main 4` = 77 (no trap), `main 6` = 77 (seed 100/2=50
           also unconsumed). REVERSIBLE: documents CURRENT lazy semantics; flips to a trap under strict.")
  (input
    (do
      (effect St (op step (-> Int64)))
      (def (main (: n Int64)) (handle St (/ 100 (- n 4)) ((step () s (resume s s))) 77))
      (export main)))
  (call main (: 4 Int64))
  (output (: 77 Int64))
  (call main (: 6 Int64))
  (output (: 77 Int64)))

(case
  "an unread op argument is unobserved (arm ignores its param), so its trapping expression is elided (lazy-effect)"
  (doc
    "The op argument `(/ 100 (- n 4))` is bound to the arm parameter `v`, which the arm `(fire (v) s
           (resume 7 s))` NEVER reads — the call-boundary analogue of an argument bound to an unused parameter
           (09-functions). The argument is unobserved, so its trap at n = 4 (`100/0`) MAY be elided; the arm
           resumes 7. `main 4` = 7 (no trap), `main 6` = 7 (arg 100/2=50 also unread). REVERSIBLE: documents
           CURRENT lazy semantics; flips to a trap under a future strict-effect spec revision.")
  (input
    (do
      (effect St (op fire (-> Int64 Int64)))
      (def (main (: n Int64)) (handle St 0 ((fire (v) s (resume 7 s))) (St.fire (/ 100 (- n 4)))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 7 Int64))
  (call main (: 6 Int64))
  (output (: 7 Int64)))

(case
  "a RESULT-returning effect op is matched on Ok / Err — the fallible-step idiom"
  (doc
    "The `Result` companion of the Option-result case: an operation whose declared result is a
           `(Result Int64 Int64)`, resumed with an `Ok` or `Err` chosen by the arm, and the body dispatches
           on the variant — the fallible-parser-step shape (a step returns `Ok value` on success or `Err
           code` on failure). `Parse.step : Int64 -> (Result Int64 Int64)`, arm `(step (n) s (resume (if (>
           n 0) (Ok (+ n s)) (Err 99)) (+ s 1)))` — the RESUME value itself branches on the argument. Seeded
           0, `(Parse.step 5)` (n > 0) resumes `(Ok (+ 5 0))` = `(Ok 5)`, and `(match … ((Ok v) v) ((Err e)
           e))` binds `v = 5`. Pins that a `Result`-typed resume value — constructed by an `if` INSIDE the
           arm — folds into the scrutinee and dispatches on Ok/Err (the control the fallible pass runs on;
           the Err path, `(Parse.step -3)` → `(Err 99)` → 99, is its complement). Both backends agree.")
  (input
    (do
      (effect Parse (op step (-> Int64 (Result Int64 Int64))))
      (def
        (main)
        (handle
          Parse
          0
          ((step (n) s (resume (if (> n 0) (Ok (+ n s)) (Err 99)) (+ s 1))))
          (match (Parse.step 5) ((Ok v) v) ((Err e) e))))
      (export main)))
  (output (: 5 Int64)))

(case
  "a TUPLE-returning operation resumes a pair built from the handler state, then projected"
  (doc
    "An operation whose declared RESULT is a `(Tuple Int64 Int64)`, resumed with a pair BUILT from the
           handler state. `P.pair : Unit -> (Tuple Int64 Int64)`; the arm resumes `(tuple s (+ s 1))` — a
           pair of the current state and its successor. Seeded 5, `(P.pair)` yields `(5, 6)`, and `(. (P.pair)
           1)` projects the second element, 6. Pins that a compound (tuple) resume value built from the
           handler state crosses the pure one-hole fold and is projectable — the tuple companion of the
           sum-result case above, the shape of a stateful op returning several derived values at once.")
  (input
    (do
      (effect P (op pair (-> Unit (Tuple Int64 Int64))))
      (def (main) (handle P 5 ((pair (u) s (resume #tuple(s (+ s 1)) s))) (. (P.pair) 1)))
      (export main)))
  (output (: 6 Int64)))

(case
  "an ABORTIVE arm whose value is a COMPOUND matches the handle body type and folds"
  (doc
    "The compound-valued abortive arm (the tuple companion of the scalar abort cases): an operation
           whose declared RESULT is a `(Tuple Int64 Int64)` is handled by an ABORTIVE arm (no `resume`) that
           yields a tuple — `(bail (n) s (tuple n n))`. The whole handle body IS the perform `(Bail.bail 7)`,
           so the arm value becomes the handle value: `(7, 7)`. This exercises the abortive type-consistency
           guard on the SOUND side — the arm body type `(Tuple Int64 Int64)` equals the op result type AND
           the handle body type, so it folds (a mismatch would decline, guarding the compound-body-abort
           miscompile where a scalar abort value disagreed with a compound position). Pins that a
           compound-valued abort matching its declared type folds rather than over-declining.")
  (input
    (do
      (effect Bail (op bail (-> Int64 (Tuple Int64 Int64))))
      (def (main) (handle Bail #tuple(0 0) ((bail (n) s #tuple(n n))) (Bail.bail 7)))
      (export main)))
  (output (: #tuple(7 7) (Tuple Int64 Int64))))

(case
  "an abortive arm yields a heap LIST built in the arm as the handle's value"
  (doc
    "The heap-collection abort (the tuple abort above is a fixed-shape compound; this arm BUILDS an
           RRB list): `(stop (v) s (list v v v))` never resumes — the 3-element list constructed in the arm
           becomes the handle's value, abandoning the body's continuation (`(list 1)` never evaluates). The
           abort's `br` must carry a live heap HANDLE out of the handler block (not a scalar), and the
           abandoned continuation's pending values must not corrupt it. `List.len` reads 3.")
  (input
    (do
      (effect Halt (op stop (-> Int64 (List Int64))))
      (def
        (main (: n Int64))
        (List.len (handle Halt 0 ((stop (v) s #list(v v v))) (do (Halt.stop n) #list(1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64)))

(case
  "an abortive arm that READS a heap-typed (Map) handler state folds — seed-let-lift on the abort path"
  (doc
    "breaker heap-abort-state. A HEAP-typed handler state (`Map.empty`) is not a shareable constant, so
           `reduce_handle` let-binds the seed to a fresh `#seed` and threads THAT (each state splice is a
           `#seed` ref). An ABORT arm (no resume) whose expression READS the state binder — `(halt (u) s (*
           1000 (+ (Map.len s) a)))` — carries `#seed` refs in the collapsed abort value. Before the fix the
           abort-collapse return path did NOT re-wrap the value in the `(let ((#seed Map.empty)) …)` (only the
           resumptive return did), so `#seed` read UNBOUND → CDZ0101 on a valid program. Fixed by applying the
           same seed-let-lift on the abortive returns. Seeded `Map.empty`, called `(main 2)`: the abort reads
           `(Map.len Map.empty)` = 0, so `(* 1000 (+ 0 2))` = 2000. Pins that a heap-state read in an abort arm
           folds (scalar-state reads already folded — a scalar seed is a shareable constant with no `#seed`;
           heap-state CONSTANT-answer abort arms already folded — no `#seed` ref survives).")
  (input
    (do
      (effect St (op halt (-> Unit Int64)))
      (def
        (main (: a Int64))
        (handle St Map.empty ((halt (u) s (* 1000 (+ (Map.len s) a)))) (St.halt)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2000 Int64)))

(case
  "a recursive same-effect advance observed by an ABORTIVE arm — must fold to the advanced state or decline, never the pre-recursion seed (sr5)"
  (doc
    "breaker sr5 (was HIGH silent-miscompile, now FOLDED). The ABORTIVE-arm sibling of the sr4
           control: a recursive loop of same-effect `(Acc.put)` performs each advance the state, then a LATER
           same-effect `(Acc.fin)` whose arm is ABORTIVE (`(fin (u) s s)`, no resume) reads it. The
           caller-observed-out-state machinery threads the advance to a RESUMING observer (sr4 -> 2); the
           abort collapse USED to materialize fin's value against the PRE-recursion seed slot, silently
           returning 0. FIXED: specialize_recursive now takes multi-value mode for a caller-observed callee
           even under an abortive observer (the `(grow k)` call returns `(value, out-state)`, threading its
           `(. t 1)` into `cur[slot]`), and the bare-abort return path drains the pending multi-value temp —
           so the abort reads the ADVANCED out-state. main(2) -> 2 (put ran twice -> state 2 -> fin reads 2).
           A callee the multi-value machinery cannot bind still declines cleanly (the non-threadable floor).
           The pin guards it is NEVER the silent 0 — a regression that drops the advance flips this to FAIL.")
  (input
    (do
      (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
      (def (grow (: n Int64)) (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
      (def
        (main (: k Int64))
        (handle
          Acc
          0
          ((put (u) s (resume 0 (+ s 1))) (fin (u) s s))
          (do (def _g (grow k)) (Acc.fin))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64)))

(case
  "a resuming observer of a recursive same-effect advance reads the advanced state (sr4)"
  (doc
    "breaker sr5-family CONTROL (sr4): a recursive loop of same-effect `(Acc.put)` performs each advance
           the handler state; a LATER same-effect `(Acc.fin)` whose arm RESUMES `(resume s s)` reads that
           advanced state. The caller-observed-out-state machinery threads the recursion's advance to the
           resuming observer. `(grow k)` puts k times (state 0->k), then `(Acc.fin)` reads k. main(2): put ran
           twice -> state 2 -> fin reads 2. (Contrast the ABORTIVE fin of the same shape, which declines
           cleanly rather than miscompile — a compiler-side decline pin, not corpus-expressible.)")
  (input
    (do
      (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
      (def (grow (: n Int64)) (if (= n 0) 0 (+ (Acc.put) (grow (- n 1)))))
      (def
        (main (: k Int64))
        (handle
          Acc
          0
          ((put (u) s (resume 0 (+ s 1))) (fin (u) s (resume s s)))
          (do (def _g (grow k)) (Acc.fin))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64)))

(case
  "a same-effect abortive arm with two INLINE advancing performs reads the advanced state (no recursion)"
  (doc
    "breaker sr5-family CONTROL: a same-effect ABORT arm (`(fin (u) s s)`, no resume) with NO recursion
           before it — two INLINE `(Acc.put)` performs advance the state 0->2, then `(Acc.fin)` reads 2. With
           no recursive advance the abort-collapse reads the state correctly, so this FOLDS (the sr5 guard
           keys on a RECURSIVE advance before a same-effect abort, so an inline-only shape is not swept).")
  (input
    (do
      (effect Acc (op put (-> Unit Int64)) (op fin (-> Unit Int64)))
      (def
        (main)
        (handle
          Acc
          0
          ((put (u) s (resume 0 (+ s 1))) (fin (u) s s))
          (do (def _a (Acc.put)) (def _b (Acc.put)) (Acc.fin))))
      (export main)))
  (call main)
  (output (: 2 Int64)))

(case
  "a DIFFERENT-effect abort after an inner recursive advance reads its own un-advanced seed (cx1)"
  (doc
    "breaker sr5-family CONTROL (cx1): a recursive INNER `(B.put)` loop advances B's state, then an
           OUTER-effect `(A.fin)` ABORT reads A's state. A's state was NEVER recursion-advanced (a DIFFERENT
           effect), so the abort correctly reads A's seed 700. The sr5 guard keys on THIS handler's abortive
           arms, so an abort of a DIFFERENT effect after the recursion is not swept — it folds. Pins the
           guard's effect-scoping: the live silent-wrong band is same-effect only.")
  (input
    (do
      (effect A (op fin (-> Unit Int64)))
      (effect B (op put (-> Unit Int64)))
      (def (grow (: n Int64)) (if (= n 0) 0 (+ (B.put) (grow (- n 1)))))
      (def
        (main (: k Int64))
        (handle
          A
          700
          ((fin (u) s s))
          (handle B 0 ((put (u) s (resume 0 (+ s 1)))) (do (def _g (grow k)) (A.fin)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 700 Int64)))

(case
  "an abortive arm that READS a heap-typed (List) handler state folds — the List face"
  (doc
    "The List face of the heap-abort-state fix above (breaker sk2g): same shape with a `(list)` seed and
           `(List.len s)` in the abort arm. `(list)` is a heap seed → `#seed` let-bound → the abort value's
           `#seed` ref is wrapped by the seed-let-lift on the abort path. `(main 2)`: `(List.len (list))` = 0
           → `(* 1000 (+ 0 2))` = 2000. Confirms the fix is state-shape-agnostic (Map + List).")
  (input
    (do
      (effect St (op halt (-> Unit Int64)))
      (def
        (main (: a Int64))
        (handle St #list() ((halt (u) s (* 1000 (+ (List.len s) a)))) (St.halt)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2000 Int64)))

(case
  "an abortive arm yields a RECURSIVE-SUM spine as the handle's value"
  (doc
    "The unbounded-shape abort: the arm yields `(S (S (Z)))` — a recursive-sum spine — and the
           abandoned body would have produced the different-depth `(Z)`. The abort path must carry the
           multi-node heap structure out intact; the fold reads depth 2 (a corrupted or body-value handle
           would read 0). With the list case above, pins that the abortive `br` carries every heap value
           class, completing the abort-value matrix (scalar/runtime-scalar/tuple/list/recursive-sum).")
  (input
    (do
      (type Nat (Z) (S Nat))
      (effect Halt (op stop (-> Int64 Nat)))
      (def (depth (: v Nat)) (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
      (def
        (main (: n Int64))
        (depth (handle Halt 0 ((stop (v) s (S (S (Z))))) (do (Halt.stop n) (Z)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "a mixed handler's ABORTIVE arm value reads BOTH the op arg and the state binder"
  (doc
    "The mixed resuming+abortive handler where the ABORTIVE arm's value is a function of BOTH the op
           ARGUMENT and the handler STATE binder — `(stop (code) s (* code s))` — reached after a resuming
           sibling `get` has run. Body `(+ (E.get) (E.stop 7))`: `E.get` resumes the seed s=n (identity
           resume), then `E.stop 7` ABANDONS the pending `(+ n …)` and the arm value `(* 7 n)` becomes the
           handle's value. Exercises the abort-value path reading its op arg AND the live state together.
           n=5 → 7*5 = 35 (the `+ n` is abandoned); n=3 → 21.")
  (input
    (do
      (effect E (op get (-> Int64)) (op stop (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle E n ((get () s (resume s s)) (stop (code) s (* code s))) (+ (E.get) (E.stop 7))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 35 Int64))
  (call main (: 3 Int64))
  (output (: 21 Int64)))

(case
  "an abortive perform in a conditional let-INIT hoists (aborting branch) and folds"
  (doc
    "E4 let-init hoist: an abort in a conditional `let` INIT is lifted out by distributing the whole let
           into each branch with the init replaced. The true branch's init is an unconditional abort the fold
           collapses (discarding the let body) → the Bail arm value 7; sound because the condition and any
           preceding bindings are pure.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 99 ((bail (n) s n)) (let ((k (if true (Bail.bail 7) 0))) (+ 1 k))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a conditional let-INIT whose abort branch is NOT taken folds the non-aborting branch"
  (doc
    "The non-aborting direction of the let-init hoist: the false branch keeps the let with init 0, so
           `(+ 1 0)` = 1.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 99 ((bail (n) s n)) (let ((k (if false (Bail.bail 7) 0))) (+ 1 k))))
      (export main)))
  (output (: 1 Int64)))

(case
  "an abortive let-init after an effectful binding folds via inner-handle pre-reduction"
  (doc
    "The preceding binding `a = (Get.get 0)` is effectful, but `Get` sits under a NESTED inner handle
           and the outer `Bail` is abortive, so the fold PRE-REDUCES the inner `Get` handle (folding `a` to
           the constant 5). With `a` pure, the conditional let-init abort hoists and homes to Bail. x<5: the
           k-init aborts → 7. x>=5: a=5, k=0 → 5.")
  (input
    (do
      (effect Get (op get (-> Int64 Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main (: x Int64))
        (handle
          Bail
          0
          ((bail (n) s n))
          (handle
            Get
            0
            ((get (n) s (resume 5 s)))
            (let ((a (Get.get 0)) (k (if (< x 5) (Bail.bail 7) 0))) (+ a k)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 7 Int64))
  (call main (: 9 Int64))
  (output (: 5 Int64)))

(case
  "two nested intra-program handlers compose inside-out"
  (doc
    "Two nested handles compose: the fold reduces the INNER handle first (discharging `A`), leaving `B`
           for the outer fold. `(A.a)` resumes 22, `(B.b)` resumes 20, so `(+ (A.a) (B.b))` = 42.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def
        (main)
        (handle
          B
          0
          ((b (u) s (resume 20 s)))
          (handle A 0 ((a (u) s (resume 22 s))) (+ (A.a) (B.b)))))
      (export main)))
  (output (: 42 Int64)))

(case
  "a recursive fn threads TWO nested stateful handlers' states at once"
  (doc
    "A recursive `loop` runs under two nested stateful handlers: `A` (countdown seeded 3, threads s-1)
           governs depth, `B` (accumulator seeded 0, threads s+10) folds across steps. `loop` performs BOTH,
           so neither handler alone can specialize it — the fold merges both contexts into one 2-slot context
           and threads each effect's state as its own trailing param. Ticks read 3,2,1,0; bumps read 0,10,20;
           sum 0+10+20+0 = 30.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)))
      (effect B (op bump (-> Unit Int64)))
      (def (loop) (if (= (A.tick) 0) 0 (+ (B.bump) (loop))))
      (def
        (main)
        (handle
          B
          0
          ((bump (u) s (resume s (+ s 10))))
          (handle A 3 ((tick (u) s (resume s (- s 1)))) (loop))))
      (export main)))
  (output (: 30 Int64)))

(case
  "a def-boundary conditional abort with PURE arguments folds and homes to its handler"
  (doc
    "A helper `unwrap` aborts in a match arm (`((None) (Bail.out tag))`), called with PURE arguments
           `(unwrap (if (> n 0) (Some n) (None)) 11)`. The scrutinee is pure, so the fold captures the
           conditional abort per-branch soundly and it homes to Bail correctly (the narrow #11-B gate fires
           only when an ARGUMENT directly performs a foreign op, so a pure-arg shape must keep folding).
           main(-1): None → Bail.out 11 → 500+11 = 511. main(4): Some 4 → a=4 → 10*4+3 = 43.")
  (input
    (do
      (effect Bail (op out (-> Int64 Int64)))
      (def
        (unwrap (: o (Option Int64)) (: tag Int64))
        (match o ((Some v) v) ((None) (Bail.out tag))))
      (def
        (main (: n Int64))
        (handle
          Bail
          0
          ((out (v) t (+ 500 v)))
          (let ((a (unwrap (if (> n 0) (Some n) (None)) 11))) (+ (* 10 a) 3))))
      (export main)))
  (call main (: -1 Int64))
  (output (: 511 Int64))
  (call main (: 4 Int64))
  (output (: 43 Int64)))

(case
  "a recursive abortive callee as the handle-body TAIL folds to the arm value"
  (doc
    "A tail-recursive callee `go` that bails at the base, called as the handle-body TAIL (no pending
           continuation): the abort propagates up the tail calls to the handle value. `go 2` counts down to
           `(Mx.bail 5)`; the arm `(* v 100)` → 500.")
  (input
    (do
      (effect Mx (op bail (-> Int64 Int64)))
      (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
      (def (main) (handle Mx 0 ((bail (v) s (* v 100))) (go 2)))
      (export main)))
  (output (: 500 Int64)))

(case
  "a tail-resumptive perform BEFORE an abort on the same spine abandons the pending continuation"
  (doc
    "get resumes (seed 5), then stop aborts → the arm value 99 becomes the handle value (the `(+ 5 …)`
           continuation is abandoned).")
  (input
    (do
      (effect E (op get (-> Unit Int64)) (op stop (-> Int64 Int64)))
      (def (main) (handle E 5 ((get (u) s (resume s s)) (stop (n) s2 n)) (+ (E.get) (E.stop 99))))
      (export main)))
  (output (: 99 Int64)))

(case
  "an abort BEFORE a tail-resumptive perform wins and the later perform never runs"
  (doc "The abort is evaluated first (left operand), so it wins and `get` never runs → 99.")
  (input
    (do
      (effect E (op get (-> Unit Int64)) (op stop (-> Int64 Int64)))
      (def (main) (handle E 5 ((get (u) s (resume s s)) (stop (n) s2 n)) (+ (E.stop 99) (E.get))))
      (export main)))
  (output (: 99 Int64)))

(case
  "an abort in a MATCH-SCRUTINEE abandons the whole match"
  (doc "The scrutinee performs an abort, abandoning the entire match → the arm value 7.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (match (Bail.bail 7) (0 100) (_ 200))))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abort value COMPUTED from the op argument folds"
  (doc
    "The abort arm value is a function of the op arg: `(* n 2)` with n=7 → 14 (the pending `(+ 1 …)` is
           abandoned).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s (* n 2))) (+ 1 (Bail.bail 7))))
      (export main)))
  (output (: 14 Int64)))

(case
  "an abort deeply nested under pure operators is still abandoned to the arm value"
  (doc "The abort sits several pure operators deep; it still abandons them all → the arm value 7.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (* 2 (+ 1 (- 10 (Bail.bail 7))))))
      (export main)))
  (output (: 7 Int64)))

(case
  "an abort under an OUTER tail-resumptive handler of a DIFFERENT effect abandons to the arm value"
  (doc
    "The inner Bail aborts → 7; the outer A handler's body value IS the reduced inner-handle value, so
           the whole program is 7.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          0
          ((a (u) s (resume 10 s)))
          (handle Bail 0 ((bail (n) s2 n)) (+ (A.a) (Bail.bail 7)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a conditional abort hoisted alongside a second abortive sibling declines cleanly"
  (doc
    "The hoist bails on an effectful sibling (a sound over-decline, never a mis-fold): `(+ (if (< 3 5)
           (Bail.bail 7) 0) (Bail.bail 9))` has a conditional abort next to another abort, so it declines.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) (+ (if (< 3 5) (Bail.bail 7) 0) (Bail.bail 9))))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a handle whose body is a closure is applyable (the handle result IS that closure)"
  (doc
    "regression 6373 (the handle-result twin of 6360): a `(handle …)` whose BODY is a bare lambda,
           applied directly, must be applyable. The handle discharges no perform (the body is a lambda), so
           its result IS that closure: `((handle Env 0 ((get (u) s (resume s s))) (fn (x) (+ x 1))) 10)` = 11.")
  (input
    (do
      (effect Env (op get (-> Unit Int64)))
      (def (main) ((handle Env 0 ((get (u) s (resume s s))) (fn ((: x Int64)) (+ x 1))) 10))
      (export main)))
  (output (: 11 Int64)))

(case
  "a recursive abortive callee feeding a PENDING handle-body continuation abandons it (folds to 507)"
  (doc
    "The recursive callee `go` is tail-recursive and bails, but is called at a NON-TAIL position in the
           handle body: `(+ (go 2) 999999)`. The abort must ABANDON the pending `(+ _ 999999)` (arm value
           500 → handle value), NOT let the pending `+` consume it (a silent 1000506). The non-local-exit
           TAGGED-RETURN CC folds it: `reduce_handle` detects the abortive-recursive callee at a non-tail
           handle-body operand, forces `go` into tagged mode (`go#eff` returns `#tuple(tag value)`), and
           short-circuits the pending op on the abort tag — `(let ((r (go#eff 2 0))) (if (= (. r 0) 1)
           (. r 1) (+ (. r 1) 999999)))` — so the abort value 500 becomes the handle value and `+ 7`
           outside → 507. (v1: a shareable-constant seed + a single pure-op pending continuation.)")
  (input
    (do
      (effect Mx (op bail (-> Int64 Int64)))
      (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
      (def (main) (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 2) 999999)) 7))
      (export main)))
  (call main)
  (output (: 507 Int64)))

(case
  "the zero-recursion abortive-callee shape with a pending continuation folds too (static shape)"
  (doc
    "The zero-dynamic-recursion twin `(go 0)` (base case hit immediately) folds the SAME way — the
           tagged-return CC keys on the static self-recursive SHAPE fed to a pending non-tail continuation,
           not actual recursion depth, so `(go 0)` bails at once and the pending `+ 999999` is abandoned →
           507.")
  (input
    (do
      (effect Mx (op bail (-> Int64 Int64)))
      (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
      (def (main) (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 0) 999999)) 7))
      (export main)))
  (call main)
  (output (: 507 Int64)))

(case
  "a def-boundary conditional abort with a FOREIGN op-result argument declines cleanly"
  (doc
    "A helper `unwrap` aborts in a MATCH arm, called with a FOREIGN op-result argument in a let-init:
           `(let ((a (unwrap (E.fetch) 11))) …)`. When `E.fetch` returns None the `Bail.out` must home to the
           Bail boundary, but the per-branch fold would thread the abort value into the continuation (a silent
           wrong value). The abort is opaque to the `if`-only hoist and the cross-fn guard (which walks
           `if`/`and`, not `match` arms), so this declines cleanly (the def-boundary non-local-exit is a later
           increment).")
  (input
    (do
      (effect E (op fetch (-> (Option Int64))))
      (effect Bail (op out (-> Int64 Int64)))
      (def
        (unwrap (: o (Option Int64)) (: tag Int64))
        (match o ((Some v) v) ((None) (Bail.out tag))))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((fetch () s (resume (if (= (% s 2) 0) (Some s) (None)) (+ s 2))))
          (+
            (* 10 (handle Bail 0 ((out (v) t (+ 500 v))) (let ((a (unwrap (E.fetch) 11))) (* a 2))))
            3)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 43 Int64))
  (call main (: 3 Int64))
  (output (: 5113 Int64)))

(case
  "a non-tail cross-function conditional abort declines cleanly"
  (doc
    "A helper that CONDITIONALLY aborts, called at a NON-TAIL position — `(+ 10 (check -1))` where
           `check n = (if (< n 0) (Bail.bail 99) n)`. The abort is opaque behind the call; a per-branch
           capture would treat the `if` as the handle tail → `10 + 99` = 109 instead of abandoning the `+ 10`
           to 99. The guard declines it (the non-local-exit convention is a later vertical); an UNCONDITIONAL
           cross-fn abort folds.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (check (: n Int64)) (if (< n 0) (Bail.bail 99) n))
      (def (main) (handle Bail 0 ((bail (n) s n)) (+ 10 (check -1))))
      (export main)))
  (call main)
  (output (: 99 Int64)))

(case
  "a non-tail recursive abort abandons the pending frames and folds to the abort value (99)"
  (doc
    "`walk` recurses at a NON-TAIL position — `(+ 1 (walk (- n 1)))` — and bails at the base. An abort
           ABANDONS the pending `+ 1` frames (result is the arm value 99, NOT 99 flowed back up each `+ 1` →
           102, which an ordinary return would give). The non-local-exit TAGGED-RETURN calling convention
           folds it: the specialized `walk#eff` returns a tagged tuple `#tuple(tag value)` (tag 1 = abort, 0
           = normal), the base abort yields `#tuple(1 99)`, and each non-tail self-call SHORT-CIRCUITS its
           pending frame on the abort tag — `(let ((r (walk#eff …))) (if (= (. r 0) 1) r #tuple(0 (+ 1 (. r
           1)))))` — so the abort tuple propagates up unchanged (99) instead of feeding the `+ 1`. The handle
           collapses to `(. r 1)`. Only the SELF-recursive, single-slot, tail-abort, state-oblivious-arm
           shape folds; a MUTUAL / accumulator-rewritten / pending-in-handle-body / state-reading-arm abort
           still declines cleanly (the neighbors below).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (walk (: n Int64)) (if (= n 0) (Bail.bail 99) (+ 1 (walk (- n 1)))))
      (def (main) (handle Bail 0 ((bail (n) s n)) (walk 3)))
      (export main)))
  (call main)
  (output (: 99 Int64)))

(case
  "a scalar abort in a TUPLE-typed handle body declines (type-consistency)"
  (doc
    "An abort makes its arm value the WHOLE handle's value, so it must have the body's type. `(tuple 1
           (Bail.bail 7))` has a compound body `(Tuple Int64 Int64)` but the abort yields a scalar Int64 —
           they disagree, so it is rejected CDZ0203 rather than miscompiled (a scalar substituted into the
           tuple → (1,7)). The ill-typed handler ALSO can't fold, so the emit path produces the uncoded
           HANDLER_NOT_REDUCIBLE decline (CDZ0900) as a CONSEQUENCE — and it anchors at the handle HEAD,
           sorting BEFORE the abort-value CDZ0203; `dedup_faults` drops it (has_abort_type_reject) so ONE
           primary CDZ0203 remains. `(no-other-errors)` pins that no CDZ0900 leaks alongside.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) #tuple(1 (Bail.bail 7))))
      (export main)))
  (error CDZ0203 (message "ABORTS with a value of type"))
  (no-other-errors))

(case
  "a scalar abort in a CONDITIONAL tuple operand declines (type-consistency)"
  (doc
    "The conditional twin: `(tuple 1 (if true (Bail.bail 7) 5))` — the scalar abort in a conditional
           tuple operand also disagrees with the tuple body type, so it declines rather than miscompile to
           (1,7).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle Bail 0 ((bail (n) s n)) #tuple(1 (if true (Bail.bail 7) 5))))
      (export main)))
  (error CDZ0203))

(case
  "a distributed if-branch that folds to an ill-typed arm composition declines"
  (doc
    "The type-consistency guard fires on a DISTRIBUTED branch: `(if (< 3 5) (< (Amb.flip) 5) false)` —
           the true branch folds to `(< 10 5)` = Bool, but the arm `(+ 1 (resume 10 s))` consumes the resume
           result at Int64, so the arm-over-Bool composition is ill-typed; the fold's re-run type check
           catches it and declines rather than emit invalid wasm.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< 3 5) (< (Amb.flip) 5) false)))
      (export main)))
  (error CDZ0203))

(case
  "a pure one-hole fold that would yield an ill-typed term is rejected, not miscompiled"
  (doc
    "The pure one-hole fold is a source-to-source rewrite that type-checks normally. `C = (< □ 5)` :
           Bool, so the arm `(+ 1 (resume 10 s))` folds to `(+ 1 (< 10 5))` — an integer `+` over a Bool —
           which the type checker rejects. Pins that the fold does not smuggle a type error past inference.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (< (Amb.flip) 5)))
      (export main)))
  (error CDZ0203))

(case
  "an abortive arm whose value type mismatches the op RESULT type declines"
  (doc
    "An abortive arm materializes its body as the abort value, which lands in the position the perform
           occupied — typed by the op's declared RESULT type (a perform is typed by its result, never by the
           arm value). If the body type differs — `bail : Int64 -> Bool` but the arm yields `n : Int64` — the
           abort value does not fit (in `(if c (Bail.bail 7) false)` it disagrees with the `false` sibling,
           an ill-typed `if`). The checker misses this gap, so the fold declines rather than emit invalid wasm.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Bool)))
      (def (main (: x Int64)) (handle Bail false ((bail (n) s n)) (if (< x 5) (Bail.bail 7) false)))
      (export main)))
  (error CDZ0203))

(case
  "a stray resume in a plain def body (no enclosing arm) is rejected CDZ0201"
  (doc
    "A `resume` hands a value back to the point that performed a handler arm's operation, so it is
           meaningful ONLY inside a handler arm's body. A `resume` in a plain def body — no enclosing arm —
           is malformed; `collect_faults` rejects it CDZ0201 naming the resume form (it used to resolve
           leniently and decline silently at lowering, a check≡compile gap).")
  (input (do (effect Amb (op flip (-> Unit Int64))) (def (main) (resume 1 0)) (export main)))
  (error CDZ0201 (message "resume")))

(case
  "a stray resume nested in a strict operand (no enclosing arm) is rejected CDZ0201"
  (doc
    "The nested face of the stray-resume reject: `(+ 1 (resume 2 0))` in a plain def body still has no
           enclosing arm to return into, so it is rejected CDZ0201 naming the resume form.")
  (input (do (effect Amb (op flip (-> Unit Int64))) (def (main) (+ 1 (resume 2 0))) (export main)))
  (error CDZ0201 (message "resume")))

(case
  "a pure one-hole continuation body reads an enclosing function parameter and folds"
  (doc
    "The pure one-hole fold synthesizes the folded body with the perform replaced by the resume value;
           an outer name in the body — the enclosing function PARAMETER `x` — must re-anchor to the handle's
           site so the type-consistency guard does not read it unbound and over-decline. Body `(+ x
           (Amb.flip))` with C = `(+ x □)`, arm `(+ 1 (resume 10 s))` → `(+ 1 (+ x 10))`. x=100 → 111.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def (main (: x Int64)) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ x (Amb.flip))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 111 Int64)))

(case
  "a distributed if-branch reading an enclosing parameter folds through the one-hole continuation"
  (doc
    "The distribution face of the reparent-before-guard fix: the perform sits in an if-BRANCH that reads
           the enclosing parameter — `(if (< 3 5) (+ x (Amb.flip)) 0)`. The same re-anchoring lets the taken
           branch's one-hole continuation fold: x=100 → the true branch `(+ 1 (+ 100 10))` = 111.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main (: x Int64))
        (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (if (< 3 5) (+ x (Amb.flip)) 0)))
      (export main)))
  (call main (: 100 Int64))
  (output (: 111 Int64)))

(case
  "a TUPLE-result perform's projected field feeds a SECOND perform's argument, threading state across both"
  (doc
    "The chained-compound-result shape: a perform returning a TUPLE has one of its fields projected
           and fed as the ARGUMENT to a SECOND perform, with the handler state threading across BOTH. Two
           ops on one effect: `St.pair : Unit -> (Tuple Int64 Int64)` resumes `(tuple s (+ s 1))` and
           advances the state by 10; `St.add : Int64 -> Int64` resumes `(+ n s)` (state held). Seeded 5:
           `(St.pair)` yields `(5, 6)` and threads state → 15; `(. (St.pair) 1)` projects 6; then `(St.add
           6)` reads n = 6 and the ADVANCED state s = 15, resuming `6 + 15` = 21. Pins that a COMPOUND
           perform result flows through a projection into a later perform's argument AND the state threads
           inner-to-outer across the two performs (the pair's +10 advance is visible to the add) — the
           compound-result companion of the nested/argument-position scalar sequencing cases, and the shape a
           pass takes when one effectful step returns a bundle a later step consumes. Both backends agree.")
  (input
    (do
      (effect St (op pair (-> Unit (Tuple Int64 Int64))) (op add (-> Int64 Int64)))
      (def
        (main)
        (handle
          St
          5
          ((pair (u) s (resume #tuple(s (+ s 1)) (+ s 10))) (add (n) s (resume (+ n s) s)))
          (St.add (. (St.pair) 1))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a handler arm with two lookup-matches and a computed perform key emits valid wasm (checked-arith scratch slot-width partition)"
  (doc
    "regression guard for breaker finding-21 (case mmlminT): a handler arm with TWO
           Option-match sites (one building new state, one building the resume value) over
           Map.lookup, plus a COMPUTED perform key ((+ n 1)) re-materialized inside the arm,
           formerly emitted invalid wasm on the wasm backend only (function slot-width alias:
           the i64 checked-add scratch temp for the re-materialized key aliased an i32
           Option-handle slot -> validator expected i64 found i32; rust/rust-async passed).
           Fixed by width-partitioning the checked-arith scratch claim in
           emit_checked_arith_to (v-effects, backend/wasm/select.rs). Now valid + passes x3:
           n=3 -> put key (+ 3 1)=4 value 3 -> m2 has 4->3 -> lookup 4 = Some 3 -> resume 3.
           A revert reintroduces the invalid-component wasm reject, caught here.")
  (input
    (do
      (effect S (op put (-> Int64 Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          Map.empty
          ((put
              (k v)
              m
              (let
                ((m2
                    (match
                      (Map.lookup m k)
                      ((Some x) (Map.insert m k v))
                      ((None u) (Map.insert m k v)))))
                (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
          (S.put (+ n 1) n)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "three list-append dispatches then a two-param range over a computed-index List.at emits valid wasm (ListAt index floated above the high-water)"
  (doc
    "regression guard for breaker finding-23 (case pfxmin5): a handler arm that reads the
           threaded list at a COMPUTED index ((- (List.len xs) 1)) via List.at AND List.push-es to
           the same list, dispatched THREE times, then a two-param range op whose resume value is a
           nested double-List.at match. Formerly emitted invalid wasm on the wasm backend only
           (Core::ListAt emitted its index operand at the bare scratch floor, not floor.max(high),
           unlike the sibling Core::BytesAt/StrAt fixed for finding-18; when the list operand is a
           live List.push-grown handle threaded across dispatches, the i64 computed index reset to the
           stale floor reused an i32-typed handle slot -> one wasm local at two widths -> validator
           expected i32 found i64; rust/rust-async passed). List-specific and dispatch-count-dependent
           (needs the arm to re-enter across the length-4 list boundary). Fixed by v-effects floating
           the ListAt index scratch above the high-water (d52544411, backend/wasm/select.rs, mirrors
           BytesAt). Now valid + passes x3: prefix table 0,3,7,16; range 0 3 answers 16. A revert
           reintroduces the invalid-component wasm reject, caught here.")
  (input
    (do
      (effect S (op add (-> Int64 Int64)) (op range (-> Int64 Int64 Int64)))
      (def
        (last (: xs (List Int64)))
        (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None u) 0)))
      (def
        (main (: n Int64))
        (handle
          S
          #list(0)
          ((add (v) pre (let ((t (+ (last pre) v))) (resume t (List.push pre t))))
            (range
              (i j)
              pre
              (resume
                (match
                  (List.at pre i)
                  ((Some a) (match (List.at pre j) ((Some b) (- b a)) ((None u) -1)))
                  ((None u) -1))
                pre)))
          (let ((_a (S.add n))) (let ((_b (S.add 4))) (let ((_c (S.add 9))) (S.range 0 3))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 16 Int64)))

(case
  "a computed-index List.update then push across three dispatches emits valid wasm (ListUpdate index scratch width-partitioned)"
  (doc
    "regression guard for breaker finding-23 residual (case pfxH): a handler arm that updates the
           threaded list at a COMPUTED index ((- (List.len pre) 1)) via List.update AND List.push-es to
           the grown list, dispatched three times. Sibling of the finding-23 ListAt face: the ListAt
           fix (d52544411) floated the ListAt index scratch, but Core::ListUpdate stashed its index in
           idx_slot=high with NO width-partition guard, so a floor reset landing high onto a live i32
           SumExpect-shell handle slot let the i64 index re-declare that local -> one wasm local two
           widths -> validator expected i64 found i32 (wasm-only; rust/rust-async passed). Fixed by
           v-effects width-partitioning the ListUpdate index scratch slot (f15cfb605, backend/wasm/select.rs,
           like ListAt/Let/emit_checked_arith_to). Now valid + passes x3: main 3 = 123. A revert
           reintroduces the invalid-component wasm reject, caught here.")
  (input
    (do
      (effect S (op add (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          #list(7)
          ((add
              (v)
              pre
              (let
                ((i (- (List.len pre) 1)))
                (let ((up (List.update pre i v))) (resume (List.len up) (List.push up v))))))
          (let
            ((a (S.add n)))
            (let ((b (S.add 4))) (let ((c (S.add 9))) (+ (* 100 a) (+ (* 10 b) c)))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 123 Int64)))

; -- breaker batch 427 (2026-08-26): ABORT-PATH reclaim + mixed-arm faces (the first live-objects
; clauses in an effects file). abr1: an abortive arm drops the LIST handler state (live 0); abr3: a
; heap-valued abort ARGUMENT is consumed by the arm and dropped (live 0) — wasm-only rows. abx1/2:
; two-op handlers, both-resuming and mixed resume+abortive with scalar state; abx4: an abortive
; arm's value REPLACES the whole body computation (list state ignored by the arm); abx5: a
; single-op abortive arm READS its list state. Filed adjacent: mixed arms + HEAP state + the abort
; arm READING the state mis-rejects CDZ0101 (abx3/ab4; scalar-state and single-op controls pass).
(case
  "abr1 an abortive arm drops the LIST handler state (no live objects)"
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Bail
          (if (> n 0) #list(n (+ n 1)) #list(9))
          ((bail (k) s (+ k (List.len s))))
          (+ 100 (Bail.bail 7))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 9 Int64))
  (live-objects 0))

(case
  "abr3 a heap-valued abort ARGUMENT is consumed by the arm and dropped"
  (input
    (do
      (effect Bail (op bail (-> (List Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          Bail
          0
          ((bail (xs) s (List.len xs)))
          (+ 100 (Bail.bail (if (> n 0) #list(n (+ n 1)) #list(9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "abx1 two-op effect, BOTH arms resuming (control)"
  (input
    (do
      (effect E (op step (-> Int64)) (op bump (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((step () s (resume s (+ s 1))) (bump () s (resume (* s 10) s)))
          (+ (E.step) (E.bump))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 65 Int64)))

(case
  "abx2 two-op effect, one resuming one ABORTIVE (scalar state)"
  (input
    (do
      (effect E (op step (-> Int64)) (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle E n ((step () s (resume s (+ s 1))) (bail (k) s (+ k s))) (+ (E.step) (E.bail 100))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 106 Int64)))

(case
  "abx4 mixed arms, list state, abort arm IGNORES the state"
  (input
    (do
      (effect E (op step (-> Int64)) (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          (if (> n 0) #list(n) #list(9 9))
          ((step () s (resume (List.len s) (List.prepend s 0))) (bail (k) s k))
          (+ (E.step) (E.bail 100))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 100 Int64)))

(case
  "abx5 single-op ABORTIVE with list state read by the arm (ab1 twin, resume absent entirely)"
  (input
    (do
      (effect E (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          (if (> n 0) #list(n) #list(9 9))
          ((bail (k) s (+ k (List.len s))))
          (+ (E.bail 100) 1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 101 Int64)))

; -- abx3 (breaker): mixed resume(step)+abortive(bail) arms over a GROWING LIST state whose ABORT arm READS
; the state. The step dispatch threads the growing state as a FINDING-24 `#st{node}_{slot}` name (bound only
; in the resume continuation's drain scope); the strict-op (`+`) abort collapse does NOT drain those binds
; (do-form only), so the abort arm's `(List.len #st…)` reference previously LEAKED unbound → a spurious
; CDZ0101 on a well-formed program. Now DECLINES CLEANLY (HANDLER_NOT_REDUCIBLE) — the abortive-arm-reads-a-
; #st-threaded-state guard in reduce_handle turns the wrong-diagnostic into an honest decline. Pinned as a
; decline-witness (verdict todo) with the CORRECT value 102 (n=5: step resumes List.len(list 5)=1; bail
; adds 100 + List.len of the grown [0,5]=2 → 100+2=102; 1 (dead, abandoned) is dropped) — flips to 102 PASS
; when the strict-op-abort fold increment lands (drain the `#st` binds around the abort value + the outer-
; observation soundness). Controls that FOLD: abx4 (abort ignores state), abx5 (abort-only, state ungrown).
(case
  "abx3 mixed resume+abortive arms over a growing LIST state whose ABORT arm reads the state declines cleanly (HANDLER_NOT_REDUCIBLE, not a CDZ0101 mis-reject)"
  (input
    (do
      (effect E (op step (-> Int64)) (op bail (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          (if (> n 0) #list(n) #list(9 9))
          ((step () s (resume (List.len s) (List.prepend s 0))) (bail (k) s (+ k (List.len s))))
          (+ (E.step) (E.bail 100))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 102 Int64)))

; -- breaker batch 428 (2026-08-26): RESUME-with-heap-ANSWER reclaim — arms resuming with LIST and
; arm-built STRING answers, across single and DOUBLE dispatches, and with heap STATE + heap ANSWER
; simultaneously: every consumed answer reclaims (live-objects 0). The complement of the batch-427
; abort-path faces; wasm-only rows per the live-objects convention.
(case
  "rh1 an arm resumes with a LIST answer and the body's read reclaims it"
  (input
    (do
      (effect E (op draw (-> (List Int64))))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((draw () s (resume (if (> s 0) #list(s (+ s 1)) #list(9)) (+ s 1))))
          (List.len (E.draw))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "rh2 an arm resumes with an arm-BUILT STRING answer, body measures it"
  (input
    (do
      (effect E (op name (-> String)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((name () s (resume (String.concat "ab" (if (> s 0) "c" "de")) s)))
          (String.byte-len (E.name))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "rh3 heap answers across TWO dispatches both reclaim"
  (input
    (do
      (effect E (op draw (-> (List Int64))))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((draw () s (resume (if (> s 0) #list(s) #list(9 9)) (+ s 1))))
          (+ (List.len (E.draw)) (* 10 (List.len (E.draw))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64))
  (live-objects 0))

(case
  "rh4 heap STATE and heap ANSWER together — both reclaim"
  (input
    (do
      (effect E (op draw (-> String)))
      (def
        (main (: n Int64))
        (handle
          E
          (if (> n 0) #list(n (+ n 1)) #list(9))
          ((draw () s (resume (String.concat "x" (if (= (List.len s) 2) "y" "zz")) s)))
          (String.byte-len (E.draw))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (live-objects 0))

; -- breaker batch 434 (2026-08-26): RESUMED ops with heap ARGUMENTS — the last quadrant of the
; effects heap matrix (state / abort-args / resume-answers pinned earlier): a LIST arg read by the
; arm, an in-program STRING arg, heap ARG + heap STATE in one dispatch, and the arg-BECOMES-state
; swap (the replaced state reclaims). All live-objects 0; wasm-only rows.
(case
  "rha1 a resumed op with a LIST argument — the arm reads it, everything reclaims"
  (input
    (do
      (effect E (op put (-> (List Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((put (xs) s (resume (+ (List.len xs) s) s)))
          (E.put (if (> n 0) #list(n (+ n 1)) #list(9 9 9)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (live-objects 0))

(case
  "rha2 a resumed op with an in-program STRING argument reclaims"
  (input
    (do
      (effect E (op put (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((put (t) s (resume (String.byte-len t) s)))
          (E.put (String.concat "ab" (if (> n 0) "c" "de")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "rha3 heap ARG and heap STATE together in one dispatch — both reclaim"
  (input
    (do
      (effect E (op put (-> (List Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          (if (> n 0) #list(n) #list(9 9))
          ((put (xs) s (resume (+ (List.len xs) (List.len s)) s)))
          (E.put #list(n (+ n 1) (+ n 2)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 4 Int64))
  (live-objects 0))

(case
  "rha4 an op ARGUMENT becomes the next STATE — the replaced state reclaims"
  (input
    (do
      (effect E (op swap (-> (List Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          (if (> n 0) #list(n) #list(9 9))
          ((swap (xs) s (resume (List.len s) xs)))
          (+ (E.swap #list(1 2 3)) (* 10 (E.swap #list(7))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 31 Int64))
  (live-objects 0))

; -- breaker batch 435 (2026-08-26): MULTI-LEVEL handler heap state — nested handlers with LIST
; state at BOTH levels, inner-arm DELEGATION to the outer with heap state both levels (the
; both-levels twin of lk5's outermost-only chain), and an inner handle RESULT (a heap list)
; consumed by the outer body. All live-objects 0; wasm-only rows.
(case
  "nh1 nested handlers with LIST state at BOTH levels — both reclaim"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          (if (> n 0) #list(n) #list(9 9))
          ((a () s (resume (List.len s) (List.prepend s 0))))
          (handle
            B
            (if (> n 0) #list(n (+ n 1)) #list(9))
            ((b () t (resume (List.len t) t)))
            (+ (A.a) (* 10 (B.b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 21 Int64))
  (live-objects 0))

(case
  "nh2 the inner arm DELEGATES to the outer with heap state at both levels"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          (if (> n 0) #list(n) #list(9 9))
          ((a () s (resume (List.len s) (List.prepend s 0))))
          (handle
            B
            (if (> n 0) #list(n (+ n 1)) #list(9))
            ((b () t (resume (+ (A.a) (List.len t)) t)))
            (B.b))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "nh3 the inner handle RESULT is a heap value consumed by the outer body"
  (input
    (do
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (List.len
          (handle
            B
            n
            ((b () t (resume t (+ t 1))))
            (if (> (B.b) 0) #list(n (+ n 1) (+ n 2)) #list(9)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects 0))

; -- breaker batch 441 (2026-08-26): multi-heap-arg ops + the arg-becomes-ANSWER flow — an op with
; TWO heap arguments (list + string) both read and reclaimed; the arm resuming with the op's OWN
; heap argument as the answer (ownership crosses from the call into the resumed body); and the arm
; TRANSFORMING the arg (prepend) into the answer. All live-objects 0; wasm-only rows.
(case
  "mha1 an op with TWO heap arguments — both read by the arm, both reclaim"
  (input
    (do
      (effect E (op put (-> (List Int64) String Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          0
          ((put (xs t) st (resume (+ (List.len xs) (String.byte-len t)) st)))
          (E.put (if (> n 0) #list(n (+ n 1)) #list(9)) (String.concat "ab" (if (> n 0) "c" "de")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "mha2 the arm resumes with the op's OWN heap argument as the answer (arg becomes answer)"
  (input
    (do
      (effect E (op echo (-> (List Int64) (List Int64))))
      (def
        (main (: n Int64))
        (handle
          E
          0
          ((echo (xs) st (resume xs st)))
          (List.len (E.echo (if (> n 0) #list(n (+ n 1) (+ n 2)) #list(9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "mha3 the arm TRANSFORMS the heap arg into the answer (prepend then resume)"
  (input
    (do
      (effect E (op grow (-> (List Int64) (List Int64))))
      (def
        (main (: n Int64))
        (handle
          E
          0
          ((grow (xs) st (resume (List.prepend xs 0) st)))
          (List.len (E.grow (if (> n 0) #list(n (+ n 1)) #list(9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects 0))

; -- breaker batch 479 (2026-08-27): the remaining capture-once edge from the cp probe set (cp1/cp3
; were pinned by v-effects with #3929; cp4 factory-body is a QUEUED pre-existing silent re-draw —
; not pinnable until fixed). cpc1 = the CONDITIONAL-selected performing capture — FLIPPED
; by #4038 (Form D branch-aware distribution: the closure-let distributes into the if's branches,
; each folding via the capture-once hoist; the draw fires only in the taken branch). Folds 50 on
; all targets, exactly the oracle pinned at the decline.
(case
  "cpc1 a conditionally-selected performing capture folds via the branch-aware distribution"
  (doc
    "A closure bound to an `if` whose branches select between a performing capture-once closure and a
           pure one — `(let ((f (if (> n 0) (let ((a (St.next))) (fn (x) (* a x))) (fn (x) x)))) (f 10))`.
           FOLDS: the branch-aware distribution rewrites `(let ((f (if C X Y))) BODY)` → `(if C (let ((f X))
           BODY) (let ((f Y)) BODY))`, so the performing branch becomes a plain capture-once closure the
           #3894 hoist threads once (draw fires only in the taken branch) and the pure branch folds directly.
           At n=5 the then-branch is taken: a = St.next = 5 captured once, (f 10) = 5*10 = 50.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((f (if (> n 0) (let ((a (St.next))) (fn ((: x Int64)) (* a x))) (fn ((: x Int64)) x))))
            (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64)))

; -- breaker batch 481 (2026-08-27): the SINGLE-application control for the cp4 factory-body
; sharing bug. A nullary factory whose BODY performs, called once and applied once, folds
; correctly (a = seed, 50) — the bug (v-effects isolation: the upstream inline pass duplicates
; the performing nullary-def-call across MULTIPLE use sites BEFORE reduce_handle, so one draw
; becomes two independent draws; filed to v-inference as the resolve/inline owner) is invisible
; with a single use. The multi-application shape (correct value 150, currently 170) stays
; UNPINNED until the inliner binds-once; this control fences the fix from breaking the
; single-use path.
(case
  "cpf1 a performing nullary factory called once and applied once folds capture-once"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (mk) (let ((a (St.next))) (fn ((: x Int64)) (* a x))))
      (def
        (main (: n Int64))
        (handle St n ((next () s (resume s (+ s 1)))) (let ((f (mk))) (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64)))

; -- breaker batch 489 (2026-08-27): two composition pairs nothing else exercises. hfr1 = the
; handler FOLD's compound result crossing the RETURN boundary — two sequential draws thread (seed,
; seed+1) into a returned list; census 2 = the reachable return cells (the crr contract: NOT a
; leak, flips to 0 with the reachability driver). hwp1 = a per-width (UInt8) list entry param read
; inside a handle body — #3852's width lift composed with the fold, 0-leak.
(case
  "hfr1 a handler fold's list result crosses the return boundary (two threaded draws, two reachable cells)"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle St n ((next () s (resume s (+ s 1)))) #list((St.next) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: #list(5 6) (List Int64)))
  (live-objects 2))

(case
  "hwp1 a UInt8-width list entry param read inside a handle body composes with the fold"
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: xs (List UInt8)))
        (handle St 3 ((get (u) s (resume s s))) (+ (St.get) (List.len xs))))
      (export main)))
  (call main (: #list(1 2 3) (List UInt8)))
  (output (: 6 Int64)))

; -- breaker batch 490→495 (2026-08-27): the cp4 arc, CLOSED. History: silent re-draw 170 (the
; pre-reduce_handle inline duplicated a performing nullary factory call per use) → fail-loud
; decline (#3996, v-inference's bind-once guard) → capture-once FOLD 150 (#4017). Oracle 150 =
; mk's body draws ONCE at the (mk) call, f shares a=seed; 5*10 + 5*20 at n=5 — adjudicated in the
; #3929 arc, pinned before either fix. Controls: cpf1 (single-app, 50) above, idc1/idc2
; (pure-alloc bind-once) in 09-functions.
(case
  "cp4 a performing nullary factory applied twice shares its single draw via the capture-once fold"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (mk) (let ((a (St.next))) (fn ((: x Int64)) (* a x))))
      (def
        (main (: n Int64))
        (handle St n ((next () s (resume s (+ s 1)))) (let ((f (mk))) (+ (f 10) (f 20)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 150 Int64)))

; -- breaker batch 493 (2026-08-27): fresh-surface pins the hour #4006 landed (deep_fresh_copy
; cracked the nested-capture resolution blocker; cp1/cc3 flipped). Two edges past the flipped
; rungs, both folding correctly: TRIPLE-nested capture (cnw1 — one draw shared through three
; closure layers; cp1 was depth two) and a factory taking TWO performing ARGS (cnw2 — each drawn
; once, in order, shared across both applications; cc3 was single-arg).
(case
  "cnw1 a capture-once closure wrapped by two nesting closures folds through all three layers"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((g (let ((a (St.next))) (fn ((: x Int64)) (* a x)))))
            (let
              ((h (fn ((: y Int64)) (+ (g y) 1))))
              (let ((k (fn ((: z Int64)) (* (h z) 2)))) (k 10))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 102 Int64)))

(case
  "cnw2 a factory taking TWO performing args draws each once and shares both across applications"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (mk2 (: m Int64) (: p Int64)) (fn ((: x Int64)) (+ (* x m) p)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let ((f (mk2 (St.next) (St.next)))) (+ (f 10) (f 20)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 162 Int64)))

; -- breaker batch 494 (2026-08-27): the closure-param face of the specialize_recursive family
; (sibling to xar5's arm-capture). A capture-once closure passed into a RECURSIVE HOF under the
; fold errors "parameter reference has no local slot" — an INTERNAL message (no CDZ code, no
; position; below the honest-decline bar; filed to v-effects). Both halves work alone: the same
; recursive HOF with a plain closure (no effects) answers 6375, and the same capture-once closure
; through a NON-recursive HOF answers 50. Oracle 6375 = capture-once a=seed, 50 frames of k*5.
(case
  "chr1 a capture-once closure driven by a RECURSIVE higher-order fn folds — its enclosing-param capture is threaded into the escaping closure env"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (drive (: f (-> Int64 Int64)) (: k Int64)) (if (= k 0) 0 (+ (f k) (drive f (- k 1)))))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let ((g (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) (drive g 50))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6375 Int64))
  ; The escaping capturing closure `g` (its env holds the once-drawn capture) is not reclaimed after
  ; the recursive `drive` returns — a Perceus reclaim gap for a capturing closure that escapes into a
  ; recursive HOF (a stable 1-object leak, independent of n; v-core-opt reclaim lane, same class as the
  ; xar5 resume-escape fence). The FOLD is correct (main(n) = n·1275); only the closure cell leaks.
  (live-objects known-leak))

; -- breaker batch 500 (2026-08-27): the BOTH-branches-performing edge of #4038's Form D
; distribution — each if-branch is its own performing creation-wrapper; only the TAKEN branch's
; draw fires (a=seed=5 → 50; the untaken branch's draw must not advance the state).
(case
  "cbb1 a conditional whose BOTH branches are performing captures folds with only the taken branch drawing"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((f
                (if
                  (> n 0)
                  (let ((a (St.next))) (fn ((: x Int64)) (* a x)))
                  (let ((b (St.next))) (fn ((: x Int64)) (+ b x))))))
            (f 10))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64)))

(case
  "mge1 an effect performed in a match GUARD is rejected (CDZ0407 — guards must be side-effect-free)"
  (doc
    "The soundness gate against the classic speculative-guard bug: a guard the pattern engine may
           evaluate speculatively/repeatedly must NOT perform an effect (else an earlier arm's guard could
           fire an effect even when a later arm matches, or fire it twice). `(guard x (> (C.bump) 100))`
           is rejected CDZ0407 naming the fix (lift to a `let` before the match). Pins that guards are
           enforced pure — a compiler that let this through would reintroduce speculative-effect hazards.")
  (input
    (do
      (effect C (op bump (-> Int64)))
      (def
        (classify (: v Int64))
        (handle
          C
          0
          ((bump () s (resume s (+ s 1))))
          (match v ((guard x (> (C.bump) 100)) (* x 1000)) (x (+ x 1)))))
      (def (main (: n Int64)) (classify n))
      (export main)))
  (error CDZ0407))

(case
  "mrs1 a two-resume arm over a HEAP-allocating body declines cleanly (non-tail resume, later increment)"
  (doc
    "The boundary of the tail-resumptive fold on a TWICE-resuming arm: `(+ (resume s s) (resume s s))`
           over a body that ALLOCATES heap before the perform (`(let ((xs (bld 3))) (+ (List.len xs)
           (E.ask)))`) declines honestly — 'this handler is not yet reducible by the tail-resumptive fold
           (cross-function or non-tail resume arrives in a later increment)'. Pins that a multi-resume over a
           non-re-computable (heap) continuation is REJECTED rather than double-freeing the captured heap —
           the safe boundary. (A two-resume over a PURE re-computable body IS accepted and re-runs it; that
           value's one-shot-vs-multi-shot intent is a v-effects semantic question, not pinned here.)")
  (input
    (do
      (effect E (op ask (-> Int64)))
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((ask () s (+ (resume s s) (resume s s))))
          (let ((xs (bld 3))) (+ (List.len xs) (E.ask)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64)))

(case
  "mrs2 a PURE-body two-resume arm re-computes the continuation and sums (multi-shot on a re-computable body = 100; v-effects-ruled intended)"
  (doc
    "v-effects owner ruling (2026-08-28, verified by running): a two-resume arm over a PURE
           re-computable body is INTENDED multi-shot — `(+ (resume s s) (resume s s))` with body `(* _ 10)`
           and ask->5 re-runs the pure continuation twice (50 + 50 = 100). The header 'one-shot' is a
           conservative SAFETY-FLOOR for HEAP-capturing continuations (where a 2nd resume would double-use
           captured heap — mrs1 correctly declines that). So the boundary is principled: a pure continuation
           is safely re-runnable, a heap one is not. Pins the intended pure-body multi-shot value.")
  (input
    (do
      (effect E (op ask (-> Int64)))
      (def
        (main (: n Int64))
        (handle E n ((ask () s (+ (resume s s) (resume s s)))) (* (E.ask) 10)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 100 Int64)))
