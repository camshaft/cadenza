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

; The Int64-inner case above pins the magnitude crossing. These pin the SOUNDNESS half of the unit
; erasure: the unit is erased at the BOUNDARY but preserved GUEST-SIDE by the op's declared type — so
; two same-unit host results combine as a valid same-dimension add, a cross-unit combine REJECTS at
; compile time (a wrong-dimension host value is inexpressible: the host has no unit channel), and a
; Float64-inner Qty rides the same erased-scalar path as Int64.

(case "two same-unit Qty host results combine guest-side as a same-dimension add"
  (doc    "`(+ (Env.width) (Env.width))` where both results are `(Qty Int64 meter)`: each host call crosses
           as a bare Int64 magnitude (42 + 42), but the guest's static types carry `meter` on BOTH operands,
           so the `+` is a valid same-dimension combine → `Qty.value` reads 84. Pins that the boundary
           erasure does not LOSE the unit guest-side — the add type-checks as Qty+Qty, not as bare ints that
           happen to work. Two calls consume two responses in order. Expected: 84.")
  (input  (do
            (effect Env (op width (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main)
              (host (Env)
                (Qty.value (+ (Env.width) (Env.width))))) (export main)))
  (host-responses (respond env.width (: 42 Int64)) (respond env.width (: 42 Int64)))
  (host-calls (call env.width) (call env.width))
  (output (: 84 Int64)))

(case "cross-unit Qty host results reject at the guest-side add"
  (doc    "The load-bearing soundness face: `(+ (Env.w) (Env.t))` where w yields `(Qty Int64 meter)` and t
           `(Qty Int64 second)` — the units are erased at the boundary, but the guest-side static types
           still carry the dimensions, so the add is a dimension MISMATCH and rejects CDZ0501 at compile
           time. This is exactly the fix's soundness claim: a wrong-dimension host value is INEXPRESSIBLE
           (the host supplies only magnitudes; units are fixed guest-side by each op's declared type), so
           erasure cannot smuggle a meter into a second. Rejects on every backend (frontend-shared).")
  (input  (do
            (effect Env (op w (-> Unit (Qty Int64 (Unit.base #"meter")))) (op t (-> Unit (Qty Int64 (Unit.base #"second")))))
            (def (main)
              (host (Env)
                (Qty.value (+ (Env.w) (Env.t))))) (export main)))
  (error  CDZ0501))

(case "a Float64-inner Qty host result crosses as its float magnitude"
  (doc    "The float-inner axis of the Qty host ABI: `Env.w : Unit -> (Qty Float64 meter)` crosses as a bare
           Float64 (3.5), the guest's static type carrying the unit — `Qty.value` reads 3.5 back. Pins that
           `abi_val_type` resolves a Qty to its INNER's ABI type for a float inner exactly as for Int64 (the
           landed case above); a heap-inner (Rational) Qty rides the num/den pair instead (#13 cases at the
           file top). Expected: 3.5.")
  (input  (do
            (effect Env (op w (-> Unit (Qty Float64 (Unit.base #"meter")))))
            (def (main)
              (host (Env)
                (Qty.value (Env.w)))) (export main)))
  (host-responses (respond env.w (: 3.5 Float64)))
  (host-calls (call env.w))
  (output (: 3.5 Float64)))

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

(case "an EMPTY Bytes arg crosses the host boundary"
  (doc    "The zero-length edge of the Bytes host-ARG marshal (#1640's wasm face): an empty Bytes
           value crosses as an empty list<u8> and the op fires normally. (rust-async: todo pending its
           host-arg path; wasm + rust pin the pass.)")
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (io.sink (Bytes.of (list)))))
            (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call   main (: 0 Int64)) (output (: 42 Int64)))

(case "a ROPE Bytes arg (recursive concat, uncompacted) crosses the host boundary"
  (doc    "The representation edge: a 50-leaf rope built by recursive Bytes.concat crosses the
           boundary — the marshal must flatten/walk the rope rep, not assume a flat leaf. (rust-async:
           todo pending; wasm + rust pin.)")
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (build (: n Int64) (: acc Bytes))
              (if (> n 0) (build (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (host (io)
                (io.sink (build n (Bytes.of (list))))))
            (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call   main (: 50 Int64)) (output (: 42 Int64)))

(case "a Bytes value SENT to the host is still readable after the call (the arg marshal borrows)"
  (doc    "The consuming-op discipline at the ARG site: `b` is passed to io.sink AND re-read by
           Bytes.len after — the marshal must borrow/copy, not consume (a consuming marshal would
           leave the later len reading freed memory, the adv-54/66 class at the boundary). 7 + 50.
           (rust-async: todo pending; wasm + rust pin.)")
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((b (String.to-bytes (String.concat "ab" (if (> k 100) "z" "cde")))))
                  (+ (io.sink b)
                     (* 10 (Bytes.len b))))))
            (export main)))
  (host-responses (respond io.sink (: 7 Int64)))
  (host-calls (call io.sink))
  (call   main (: 0 Int64)) (output (: 57 Int64)))

(case "a runtime Bytes host-arg BEFORE a scalar arg keeps distinct core slots (no width-clobber)"
  (doc    "The multi-arg slot-threading edge of the host-ARG marshal: a RUNTIME String/Bytes arg reserves
           i32 rope/len/pos scratch slots (at `base.max(high)`) and bumps `high`, but the emit arm formerly
           reused the STALE `base` for the FOLLOWING arg — so a subsequent scalar's i64 checked-arith guard
           teed into a slot the marshal had declared i32, one wasm local at two widths → an INVALID module
           (`func failed to validate: expected i64, found i32`). Only the marshalled-arg-BEFORE-scalar order
           tripped it; scalar-before-marshalled worked because the scalar bumped `high` first. Fixed by
           threading a rising `arg_base` (as the ordinary call arg loop does). Here `n = k+7` is BOTH the
           scalar arg AND re-read after the call (`10*n`), so a clobbered slot would corrupt the output, not
           just fail to validate: send responds 5, so 5 + 10*7 = 75. (rust-async: todo pending its host-arg
           path; wasm + rust pin the pass.)")
  (input  (do
            (effect io (op send (-> Bytes Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((n (+ k 7)))
                  (+ (io.send (String.to-bytes (String.concat "ab" (if (> k 100) "z" "cd"))) n)
                     (* 10 n)))))
            (export main)))
  (host-responses (respond io.send (: 5 Int64)))
  (host-calls (call io.send))
  (call   main (: 0 Int64)) (output (: 75 Int64)))

(case "one host effect with TWO ops interleaves its calls in program order"
  (doc    "The per-run response cursor over a MULTI-OP effect: geta, getb, geta consume rows 1,2,3 in
           the order made — the cursor is per-RUN, not per-op (a per-op cursor would give the second
           geta row 2's value... the harness rows are per-call-order). 1 + 20 + 300 = 321. Pins the
           multi-op single-effect composition the adv-65 fix's lone-op cases don't. (rust-async: todo
           pending; wasm + rust pin.)")
  (input  (do
            (effect AB (op geta (-> Unit Int64)) (op getb (-> Unit Int64)))
            (def (main (: k Int64))
              (host (AB)
                (+ (AB.geta unit)
                   (+ (* 10 (AB.getb unit))
                      (* 100 (AB.geta unit))))))
            (export main)))
  (host-responses (respond a-b.geta (: 1 Int64)) (respond a-b.getb (: 2 Int64)) (respond a-b.geta (: 3 Int64)))
  (host-calls (call a-b.geta) (call a-b.getb) (call a-b.geta))
  (call   main (: 0 Int64)) (output (: 321 Int64)))

(case "a 60-key trie captured ACROSS a host call reads intact after the response folds in"
  (doc    "The deep-heap survival face of host delegation: a 60-key multi-level trie is built BEFORE the
           host block, the delegation fires (response 7), and the trie reads len + a checked interior
           entry AFTER the response is consumed (7·1000 + 60·10 + 1 = 7601). The guest heap must survive
           the boundary crossing untouched — a delegation that reset or corrupted live heap state (or a
           marshal that clobbered the trie's handle slot) would break a read. The trie-scale companion
           of the scalar-arg re-read pin at :251.")
  (input  (do
            (effect io (op ping (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 3)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (host (io)
                  (+ (* 1000 (io.ping))
                     (+ (* 10 (Map.len m))
                        (match (Map.lookup m 37) ((Some v) (if (= v 111) 1 0)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 60 Int64))
  (host-responses (respond io.ping (: 7 Int64)))
  (host-calls (call io.ping))
  (output (: 7601 Int64)))

(case "a deep trie built BETWEEN two host calls reads correctly after the second"
  (doc    "The interleave face: the first response is consumed, a 50-key trie is built ENTIRELY between
           the two delegations (the response cursor mid-flight), and the second response arrives before
           the trie is read — (3+4)·1000 + 50 + 42 = 7092. Pins that heap construction interleaves with
           the per-run response cursor without either corrupting the other (a cursor implementation
           sharing scratch state with the allocator, or a build that disturbed the pending-delegation
           frame, would flip a component).")
  (input  (do
            (effect io (op ping (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (main (: n Int64))
              (host (io)
                (do
                  (def a (io.ping))
                  (def m (fill n Map.empty))
                  (def b (io.ping))
                  (+ (* 1000 (+ a b))
                     (+ (Map.len m)
                        (match (Map.lookup m 42) ((Some v) v) ((None _u) -1)))))))
            (export main)))
  (call   main (: 50 Int64))
  (host-responses (respond io.ping (: 3 Int64)) (respond io.ping (: 4 Int64)))
  (host-calls (call io.ping) (call io.ping))
  (output (: 7092 Int64)))

(case "a String host RESULT crosses the boundary and is read twice (byte-len + scalar-len of a multibyte response)"
  (doc    "The String-RESULT boundary face (H7's marshal reached through H9's unit-arg emit): `io.fetch :
           (-> Unit String)` returns the recorded multibyte response \"héllo\" (6 bytes, 5 scalars), which
           the guest let-binds and reads TWICE — byte-len then scalar-len — so the crossed String is a
           live guest value under the consuming-op discipline (the binding must be kept; a per-read
           re-fetch would consume a second, unsupplied response and trap). 6 + 100·5 = 506. This is the
           shape that was DECLINING arg-side pre-H9 while the String-result emit arm sat unreachable —
           the pin keeps it reachable. (wasm/rust-async: todo until their unit-arg + String-result host
           paths land; the rust baseline pins the pass.)")
  (input  (do
            (effect io (op fetch (-> Unit String)))
            (def (main (: k Int64))
              (host (io)
                (let ((s (io.fetch unit)))
                  (+ (String.byte-len s)
                     (* 100 (String.scalar-len s))))))
            (export main)))
  (host-responses (respond io.fetch (: "héllo" String)))
  (host-calls (call io.fetch))
  (call   main (: 0 Int64)) (output (: 506 Int64)))

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

(case "performs in DISCARDED do positions still run — the effect count is the observable"
  (doc    "The side-effect-only statement face (the evaluate-ONCE pins above bound the count from ABOVE;
           this bounds it from BELOW): three bare `(St.a)` statements whose results nothing binds or
           consumes, followed by an observer. Each statement must still perform and advance — the observer
           reads 8, not the seed 5. An optimizer that reasoned 'result unused → drop the call' would
           silently skip the advances; the statement position's effect is the whole point of writing it.")
  (input  (do
            (effect St (op a (-> Unit Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (do
                  (St.a)
                  (St.a)
                  (St.a)
                  (St.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))

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

(case "an `and` whose first RESUMING perform is true evaluates the second: both advances land"
  (doc    "The RESUMPTIVE face of connective sequencing (the Bail pins above cover abortive operands): both
           operands of `(and (> (St.get) 3) (> (St.get) 10))` perform an ADVANCING op. The first reads 5
           (true → the right operand runs), the second reads 6 (false), and the trailing observer reads 7 —
           both advances landed. 10 + 7 = 17. A fold that skipped the second operand despite the first being
           true, or double-ran either, shifts the observer's read.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1))))
                (+ (if (and (> (St.get) 3) (> (St.get) 10)) 100 10)
                   (St.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 17 Int64)))

(case "an `or` short-circuit SKIPS a resuming perform, and the skip is observable through the state"
  (doc    "The skip-observability pin: `(or (> (St.get) 3) (> (St.get) 0))` — the first operand reads 5
           (true), so the second perform MUST NOT run. The proof is the trailing observer: it reads 6 (one
           advance), not 7 (two). An eager lowering that evaluated both operands and discarded the second's
           result would still pick the right branch (100) but betray itself in the state — this pins the
           EFFECT COUNT of short-circuiting, not just the boolean result. 100 + 6 = 106.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get (u) s (resume s (+ s 1))))
                (+ (if (or (> (St.get) 3) (> (St.get) 0)) 100 10)
                   (St.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64)))

(case "a handler arm applies a closure capturing a heap map and the map outlives the handle"
  (doc    "EFFECTS × CAPTURE: the `look` arm applies f — a closure whose capture cell holds main's
           heap map — so the arm runs in the HANDLER's frame while the capture belongs to the
           performer's. The body performs TWICE, so arm + closure apply twice through the
           perform/resume machinery (each round-trip suspends and re-enters frames), and m is read
           AFTER the handle exits — the capture must survive every suspension. r = look(2)·100 +
           look(1) = 20·100 + 10 = 2010 via the arm's (resume (f k) s); post-handle c = m[3]=30
           hit (mode 1, sentinel-safe +1 → 31) or m[9] miss → 0 (mode 2): mode 1 → 2041,
           mode 2 → 2010.")
  (input  (do
            (effect Look (op look (-> Int64 Int64)))
            (def (build (: i Int64) (: n Int64) (: acc (Map Int64 Int64)))
              (if (> i n) acc (build (+ i 1) n (Map.insert acc i (* i 10)))))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
            (def (main (: mode Int64))
              (do
                (def m (build 1 3 Map.empty))
                (def f (fn ((: k Int64)) (get m k)))
                (def r (handle Look 0
                         ((look (k) s (resume (f k) s)))
                         (+ (* (Look.look 2) 100) (Look.look 1))))
                (def c (get m (if (= mode 1) 3 9)))
                (+ r (if (>= c 0) (+ c 1) 0))))
            (export main)))
  (call main (: 1 Int64)) (output (: 2041 Int64))
  (call main (: 2 Int64)) (output (: 2010 Int64)))

(case "a NON-LAST handler arm whose body is a MATCH round-trips through the ML printer"
  (doc    "The regression witness for the arm-extent printer fix (v-syntax, batch #136): a NON-LAST
           handler arm whose body is a match, followed by a sibling arm — pre-fix the inner match's
           pipe-arms absorbed the next handler arm on ML re-read (AST mismatch); print_handle_arm
           now paren-guards greedy block bodies. Exercises ml_surface_round_trips_the_corpus
           end-to-end (the lib-side printer test uses hand-built ASTs). Both dispatch faces compute.")
  (input (do
        (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64)))
        (def (main (: n Int64))
          (handle S 0
            ((a (v) s (match v (0 (resume 1 s)) (_ (resume 2 s))))
             (b (v) s (resume v s)))
            (+ (S.a n) (S.b 10))))
        (export main)))
  (call main (: 0 Int64)) (output (: 11 Int64))
  (call main (: 5 Int64)) (output (: 12 Int64)))

(case "a rope built before a perform survives the resume and the arm's own heap does not leak into it"
  (doc    "RESUME-boundary heap liveness, both directions: the performing BODY holds a rope (passed
           in as a param) across the suspension — read AFTER the resume returns, so the suspended
           frame's heap must stay live through the arm's execution; meanwhile the ARM builds its
           OWN rope (a do-def, folded into the resume value via byte-len) — arm-frame heap that must
           reclaim at arm exit without leaking into or freeing the resumed frame's. (The arm's
           do-def flows into the RESUME argument, which works — only the body-side PERFORM-argument
           path has the #21 do-def scoping gap, hence rope arrives as a param.) r = look(2)·10 +
           (byte-len rope − 6) + byte-len rope = (20+2)·10 + 0 + 6 = 226; post-handle c = m[1]=10
           hit (mode 1 → +11) or m[9] miss → 0: mode 1 → 2271, mode 2 → 2260.")
  (input  (do
            (effect Look (op look (-> Int64 Int64)))
            (def (build (: i Int64) (: n Int64) (: acc (Map Int64 Int64)))
              (if (> i n) acc (build (+ i 1) n (Map.insert acc i (* i 10)))))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
            (def (rep (: s String) (: n Int64) (: acc String))
              (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
            (def (body (: rope String))
              (+ (* (Look.look 2) 10)
                 (+ (- (String.byte-len rope) 6) (String.byte-len rope))))
            (def (main (: mode Int64))
              (do
                (def m (build 1 2 Map.empty))
                (def r (handle Look 0
                         ((look (k) s
                           (do
                             (def arope (rep "z" 2 ""))
                             (resume (+ (get m k) (String.byte-len arope)) s))))
                         (body (rep "ab" 3 ""))))
                (def c (get m (if (= mode 1) 1 9)))
                (+ (* r 10) (if (>= c 0) (+ c 1) 0))))
            (export main)))
  (call main (: 1 Int64)) (output (: 2271 Int64))
  (call main (: 2 Int64)) (output (: 2260 Int64)))

(case "an abortive handler discards a suspended body holding live rope and map handles"
  (doc    "The ABORT companion of the resume-boundary pin above: the body builds a rope (a do-def —
           exercising the #21 abortive-face fix, v-effects 0d382e3f4) and performs with its byte-len;
           the `bail` arm NEVER resumes, so the suspended body — which still holds the rope AND a
           borrowed read of the caller's map queued after the perform — is DISCARDED. The abandoned
           frame's heap must reclaim exactly once (no leak, no double-free), and the caller's map
           must survive the abandonment: c reads m AFTER the aborted handle. r = arm value =
           byte-len \"ababab\" = 6; mode 1 c = m[2]=20 (+1 sentinel-safe → 21), mode 2 c = m[9]
           miss → 0: mode 1 → 621, mode 2 → 600.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (build (: i Int64) (: n Int64) (: acc (Map Int64 Int64)))
              (if (> i n) acc (build (+ i 1) n (Map.insert acc i (* i 10)))))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
            (def (rep (: s String) (: n Int64) (: acc String))
              (if (= n 0) acc (rep s (- n 1) (String.concat acc s))))
            (def (run (: m (Map Int64 Int64)))
              (handle Bail 0
                ((bail (n) s n))
                (do
                  (def rope (rep "ab" 3 ""))
                  (+ (Bail.bail (String.byte-len rope)) (get m 1)))))
            (def (main (: mode Int64))
              (do
                (def m (build 1 3 Map.empty))
                (def r (run m))
                (def c (get m (if (= mode 1) 2 9)))
                (+ (* r 100) (if (>= c 0) (+ c 1) 0))))
            (export main)))
  (call main (: 1 Int64)) (output (: 621 Int64))
  (call main (: 2 Int64)) (output (: 600 Int64)))

(case "a single-task DES scheduler sleeps a task and fast-forwards the clock to its wake instant"
  (doc    "The discrete-event-simulation single-task gate (v-discrete-event-sim's step-3 forcing repro,
           minimal distillation). A `worker` task sleeps then returns its label; the `Sim` handler's `sleep`
           arm computes the wake instant and resumes with the clock advanced (a `let`-wrapped tail resume;
           the `k` binder is the scheduler ABI's reified-continuation slot, unused in the single-task case
           which resumes in place). Witnesses capabilities-and-effects.md continuation/resume semantics for
           the sleep/fast-forward idiom: the task sleeps 3s, the clock fast-forwards, the continuation
           resumes and returns \"done\". Value-grades the sleep-wake fold (a todo→fail flip here = k not
           resumed / clock not advanced). The full multi-task pqueue interleave (stored k across activations)
           is v-discrete-event-sim's follow-on gate.")
  (input  (do
            (type Duration (Duration UInt64))
            (type Instant  (Instant  UInt64))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (inst-ns (: t Instant))  (match t ((Instant.Instant n) n)))
            (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
            (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
            (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
            (def (worker (: label String) (: d Duration)) (do (Sim.sleep d) label))
            (def (main)
              (handle Sim (Instant.Instant 0)
                ( (now (u) s (resume s s))
                  (sleep (d) s k (let ((wake (at s d))) (resume unit wake))) )
                (worker "done" (secs 3)))) (export main)))
  (output (: "done" String)))

(case "a ctl-style arm whose continuation ESCAPES to another function reifies it as a closure and applies it there"
  (doc    "E5 step-3: a general `ctl`-style arm may let its continuation `k` ESCAPE — pass it to another
           function that applies it — not just apply it lexically in place. `(f () s k (use-k k))` hands `k`
           to `use-k`, which applies `(stored-k 10)`. Over the pure delimited continuation `C = (+ 1 □)`, the
           reified `k` is the closure `(fn (kv) (+ 1 kv))`; `use-k` applies it to 10 → (+ 1 10) = 11.
           Witnesses that a reified continuation over a pure continuation is a first-class value (an ordinary
           closure) that can cross a function boundary and be resumed there — the escaping-continuation
           capability a scheduler builds on. (A continuation that itself re-performs the handled effect is a
           further increment — it must re-enter its handler at apply.)")
  (input  (do
            (effect A (op f (-> Unit Int64)))
            (def (use-k (: stored-k (-> Int64 Int64))) (stored-k 10))
            (def (main) (handle A 0 ((f () s k (use-k k))) (+ 1 (A.f)))) (export main)))
  (output (: 11 Int64)))

(case "an escaping continuation that itself RE-PERFORMS the handled effect re-enters its handler at apply"
  (doc    "E5 step-3 (FACE-1 B2): the escaping-`k` case whose delimited continuation `C` itself RE-PERFORMS
           the handled effect. `(a () s k (use-k k))` over `(+ (A.a) (A.a))`: after the leading `(A.a)` the
           continuation `C = (+ □ (A.a))` performs `A.a` AGAIN. A pure-continuation closure reification does
           not serve it — applying `k` runs `C` in a SEPARATE activation where the re-performed op has no
           home. So `k` reifies as a SELF-RE-INSTALLING handler-wrapped closure `k = (fn (kv) (handle A 5
           (arm) (+ kv (A.a))))` — the continuation carries the handler around itself. `use-k` applies it to
           10: the re-installed handle folds `(+ 10 (A.a))` (one remaining perform) → (+ 10 10) = 20. Each
           re-install removes one perform (N→N-1), bottoming out at the pure-one-hole fold — no bespoke frame
           chain. The state-oblivious 2-perform case; a state-advancing arm or a deeper continuation is a
           further increment (declines cleanly). The re-entry-at-apply the DES scheduler's stored-k builds on.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (def (use-k (: stored-k (-> Int64 Int64))) (stored-k 10))
            (def (main) (handle A 5 ((a () s k (use-k k))) (+ (A.a) (A.a)))) (export main)))
  (output (: 20 Int64)))

(case "a DEFERRED resume-thunk escaping to another function re-installs the handler at apply, over a re-performing do-continuation"
  (doc    "E5 step-3 (the DES scheduler's `sleep`/`now` step-3 shape, contract-A1). The escaping continuation
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
  (input  (do
            (effect A (op set (-> Int64 Int64)) (op get (-> Unit Int64)))
            (def (run-thunk thunk) (thunk unit))
            (def (main)
              (handle A 0 ((get (u) s (resume s s)) (set (w) s (run-thunk (fn (_u) (resume w w)))))
                (do (A.set 42) (A.get)))) (export main)))
  (output (: 42 Int64)))

(case "a deferred resume-thunk STORED IN A SUM and match-extracted through a helper before apply folds"
  (doc    "E5 step-3 (the DES multi-task scheduler's pqueue store→pop→apply reach). The escaping resume-thunk
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
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (type Box (Box (-> Unit Instant)))
            (def (unbox-apply (: b Box)) (match b ((Box.Box th) (th unit))))
            (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
            (def (main)
              (handle Sim (Instant.Instant 0)
                ( (now (u) s (resume s s))
                  (sleep (wake) s (unbox-apply (Box.Box (fn (_u) (resume unit wake))))) )
                (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now))))) (export main)))
  (output (: 5000000000 Int64)))

(case "a deferred resume-thunk stored in a MULTI-PAYLOAD pqueue entry and tuple-match-extracted folds"
  (doc    "E5 step-3, the DES multi-task scheduler's REAL pqueue shape. The prior case stored the resume-thunk
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
  (input  (do
            (type Instant (Instant UInt64))
            (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
            (type KBox (KBox (-> Unit Unit)))
            (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
            (def (pop-apply (: q PQ))
              (match q
                ((PQ.PQNil _) unit)
                ((PQ.PQCons (tuple wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
            (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
            (def (main)
              (handle Sim (Instant.Instant 0)
                ( (now (u) s (resume s s))
                  (sleep (wake) s (pop-apply (PQ.PQCons (tuple wake (KBox.KBox (fn (_u) (resume unit wake))) (PQ.PQNil ()))))) )
                (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now))))) (export main)))
  (output (: 5000000000 Int64)))

(case "a performing closure passed to a function that applies it UNDER a handler is homed at the apply site"
  (doc    "The `handler runs a passed-in closure` idiom: `with-seed(body) = handle Rand … (body unit)` runs
           its `body` PARAM under the `Rand` handler, and `main` passes `(fn (u) (Rand.roll))`. The lambda's
           `Rand.roll` is homed at the APPLICATION site (inside `with-seed`, under the handler), not at its
           definition site in `main`. The no-home check computes, per callee param, the effects the callee
           applies it under (here `Rand`), and homes a lambda argument's performs against THAT set — so this
           compiles rather than a false CDZ0401. Distinct from the escaping-closure-BODY-performs reject
           (`04-capabilities` \"an ungranted effect hidden in a closure passed to a HOF is still rejected\":
           `apply-fn = (body unit)` with NO handler adds no grant, so an ungranted effect there STAYS
           CDZ0401). The `roll` arm resumes with the seed 5 → `(body unit)` reads 5. Regression pin for the
           apply-site-homing analysis (root-caused from v-cad's passed-closure-under-handler codegen bug).")
  (input  (do
            (effect Rand (op roll (-> Unit Int64)))
            (def (with-seed (: body (-> Unit Int64)))
              (handle Rand 5 ((roll (u) s (resume s s))) (body unit)))
            (def (main) (with-seed (fn (u) (Rand.roll)))) (export main)))
  (output (: 5 Int64)))

(case "a performing closure homed TRANSITIVELY through a pass-through function is not falsely rejected"
  (doc    "Apply-site homing propagated ONE call deeper: `outer(b) = inner(b)` is a PASS-THROUGH — it hands
           its `b` param onward to `inner`, which applies it under `handle R`. `main` passes `(fn (u)
           (R.roll))` to `outer`. The lambda's `R.roll` is homed where `inner` applies the param (under the
           handler), so `outer`'s `b` inherits `inner`'s granted effect `{R}` — the program compiles rather
           than a false CDZ0401. The no-home analysis, computing per callee param the effects it is applied
           under, follows a param passed as an argument to a known sub-callee and inherits the sub-callee's
           extra-handled set. SOUNDNESS twin: if the pass-through's target applied the param under NO handler,
           nothing propagates and an ungranted effect STAYS rejected (`04-capabilities`). The `roll` arm
           resumes with the seed 5.")
  (input  (do
            (effect R (op roll (-> Unit Int64)))
            (def (inner (: b (-> Unit Int64))) (handle R 5 ((roll (u) s (resume s s))) (b unit)))
            (def (outer (: b (-> Unit Int64))) (inner b))
            (def (main) (outer (fn (u) (R.roll)))) (export main)))
  (output (: 5 Int64)))

(case "a performing closure called TWICE observes the state advance between its calls"
  (doc    "The state-threading face of the performing closure (the homing pins above call the closure
           once): `f = (fn (u) (Ctr.next unit))` is let-bound under the handler and applied TWICE in one
           expression — the first call reads the seed `n` and threads `n+1`, the second reads `n+1`.
           Encodes `10·first + second` = 10n + (n+1) → 34 at n = 3. Pins that each APPLICATION of a
           performing closure is a fresh perform against the CURRENT handler state (a closure that captured
           its perform's result at creation, or replayed the first discharge, would give 33).")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Ctr n
                ((next (u) s (resume s (+ s 1))))
                (let ((f (fn ((: u Unit)) (Ctr.next unit))))
                  (+ (* 10 (f unit)) (f unit)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))

(case "a PURE closure iterated by a recursive combinator composes with a perform in the same body"
  (doc    "The effects-adjacent face of the iterate combinator (09-functions pins it pure): under a
           handler, the body BOTH performs (`Ctr.next` reads the seed 0 and threads 1) AND runs a
           recursive `times` combinator over a PURE closure `(fn (u) 5)` a RUNTIME number of times —
           `(+ (Ctr.next unit) (times (fn 5) n 0))` at n=3 is 0 + 15 = 15. The combinator's fn-param
           application must not be mistaken for a performing call (no false CDZ0401 on the pure closure,
           no spurious state advance from its iterations), and the sibling perform must still thread the
           handler state. (A PERFORMING closure through the same combinator still declines — the homing
           analysis grants effects where the callee applies its param under a handler, and here the
           handler sits at the combinator's CALL site, not inside it — so this pins the pure half.)")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (times (: f (-> Unit Int64)) (: n Int64) (: acc Int64))
              (if (< n 1) acc (times f (- n 1) (+ acc (f unit)))))
            (def (main (: n Int64))
              (handle Ctr 0
                ((next (u) s (resume s (+ s 1))))
                (+ (Ctr.next unit) (times (fn ((: u Unit)) 5) n 0))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 15 Int64)))

(case "a 100k-iteration pure tail loop under a handler runs in constant stack"
  (doc    "The SCALE face of loops-under-handlers (existing loop pins are ≤33 deep): a 100000-iteration
           tail-recursive accumulator inside a handle body, plus one perform reading the seed. The
           handler context must not break tail-call frame reuse — a lowering that let the handler frame
           capture the loop (or reified a frame per iteration) overflows long before 100k. 0 + 100000.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (< n 1) acc (loop (- n 1) (+ acc 1))))
            (def (main (: n Int64))
              (handle Ctr 0
                ((next (u) s (resume s (+ s 1))))
                (+ (Ctr.next unit) (loop n 0))))
            (export main)))
  (call   main (: 100000 Int64))
  (output (: 100000 Int64)))

(case "a PERFORMING tail loop of 10000 iterations threads state in constant space"
  (doc    "The sharper scale face: every iteration PERFORMS (`(+ acc (Ctr.next unit))`), so the
           tail-resumptive arm discharges 10000 performs — each must resume without reifying a
           continuation (10k reified frames would exhaust memory/stack). The state threads 0..9999,
           summing to 49995000. The constant-space guarantee of the E4 tail-resumptive lowering at a
           scale the ≤33-deep pins cannot witness.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (go (: n Int64) (: acc Int64))
              (if (< n 1) acc (go (- n 1) (+ acc (Ctr.next unit)))))
            (def (main (: n Int64))
              (handle Ctr 0
                ((next (u) s (resume s (+ s 1))))
                (go n 0)))
            (export main)))
  (call   main (: 10000 Int64))
  (output (: 49995000 Int64)))

(case "an effect op RESUMED with a slice-view Bytes crosses the arm boundary intact"
  (doc    "A heap VIEW as the resume value: the arm builds a `Bytes.slice` window and resumes with it;
           the body indexes the escaped view (byte 0 of (20,30) = 20, +22 = 42). The view's re-based
           coordinates must survive the continuation crossing — composing the slice-view machinery with
           the effects lowering (scalars/strings/sums as resume values are pinned; a VIEW is the shape
           a zero-copy parser hands back).")
  (input  (do
            (effect Src (op read (-> Unit Bytes)))
            (def (main (: a Int64))
              (handle Src 0
                ((read (u) s
                  (match (Bytes.slice (Bytes.of (list 9 20 30 8)) 1 2)
                    ((Some w) (resume w s))
                    ((None x) (resume (Bytes.of (list)) s)))))
                (+ (match (Bytes.at (Src.read unit) 0) ((Some v) v) ((None u) -1)) a)))
            (export main)))
  (call   main (: 22 Int64))
  (output (: 42 Int64)))

(case "an effect op RESUMED with a constructed Ast node crosses the arm boundary and matches in the body"
  (doc    "An AST as the resume value — the first Ast crossing an effect boundary in the corpus: the arm
           constructs `(Ast.Int (BigInt.of x))` from the op param and resumes with it; the body pattern-matches
           the node back out and extracts the boxed BigInt payload (25N). Ast is a recursive sum with a
           BigInt-boxed leaf, a representation the scalar/string/sum/view resume-value pins don't reach — the
           template-provider idiom (a handler that answers with syntax) rests on this crossing.")
  (input  (do
            (effect Tmpl (op get (-> Int64 Ast)))
            (def (main (: n Int64))
              (handle Tmpl 0
                ((get (x) s (resume (Ast.Int (BigInt.of x)) s)))
                (match (Tmpl.get n)
                  ((Ast.Int b) b)
                  (_ -1N))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 25 BigInt)))

(case "an effect op resumed with a whole MAP threads the CHAMP through the continuation"
  (doc    "A collection HANDLE as the resume value: the arm resumes with a 2-entry map, and the body
           looks it up at the boundary parameter — k=2 → 20, k=9 → None → -1. The CHAMP handle rides the
           continuation like any value; the body's descent runs on the arm-built trie. (Map-STATE
           handlers are pinned nearby; this is the map-as-RESULT face.)")
  (input  (do
            (effect Env (op vars (-> Unit (Map Int64 Int64))))
            (def (main (: k Int64))
              (handle Env 0
                ((vars (u) s (resume (Map.insert (Map.insert Map.empty 1 10) 2 20) s)))
                (match (Map.lookup (Env.vars unit) k)
                  ((Some v) v) ((None u) -1))))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 20 Int64))
  (call   main (: 9 Int64))
  (output (: -1 Int64)))

(case "a LIST OF SUMS resumed from a handler feeds an event-sourcing fold in the body"
  (doc    "The heap-of-variants resume: the arm resumes with `(list (Add n) (Reset) (Add 40) (Add 2))` —
           a list whose ELEMENTS are sum values with mixed payload/nullary variants — and the body runs
           the apply-events fold over it (the Reset discards the runtime n; 40+2 = 42 regardless).
           Composes the sum-list construction in an ARM, the crossing of variant-tagged heap elements
           through the continuation, and the per-variant dispatch fold in the body — the config-provider
           idiom (a handler supplying a program's event stream).")
  (input  (do
            (type Ev (Add Int64) (Reset))
            (effect Src (op events (-> Unit (List Ev))))
            (def (apply-ev (: acc Int64) (: e Ev))
              (match e ((Add v) (+ acc v)) ((Reset) 0)))
            (def (run (: evs (List Ev)) (: acc Int64))
              (match evs
                ((list) acc)
                ((list h .. t) (run t (apply-ev acc h)))))
            (def (main (: n Int64))
              (handle Src 0
                ((events (u) s (resume (list (Add n) (Reset) (Add 40) (Add 2)) s)))
                (run (Src.events unit) 0)))
            (export main)))
  (call   main (: 999 Int64))
  (output (: 42 Int64)))

(case "a handler arm RECURSES through a named helper before resuming"
  (doc    "The arm-calls-a-def face: `tally`'s arm computes `(triangle v 0)` — a RECURSIVE tail loop over
           the op argument — before resuming with its result (4 → 10, 10 → 55). The arm body is not a
           plain expression context: the recursive call runs under the handler's dispatch frame, and its
           result feeds the resume. An arm lowering that couldn't re-enter user code (or that confused
           the helper's frames with the handler's) breaks the larger input.")
  (input  (do
            (effect Sum (op tally (-> Int64 Int64)))
            (def (triangle (: n Int64) (: acc Int64))
              (if (< n 1) acc (triangle (- n 1) (+ acc n))))
            (def (main (: n Int64))
              (handle Sum 0
                ((tally (v) s (resume (triangle v 0) s)))
                (Sum.tally n)))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 10 Int64))
  (call   main (: 10 Int64))
  (output (: 55 Int64)))

(case "an arm's NEXT-STATE expression saturates via a conditional on the current state"
  (doc    "The state-transition-function face: the arm's next-state is `(if (>= s 3) 3 (+ s 1))` — a
           CLAMP, not a plain increment. Four bumps from seed 0 read 0,1,2,3 with the final read at the
           ceiling (3); from seed 5 the first transition already clamps (5 → 3, reads 5 then 3,3,3 —
           final 3). Pins that the next-state slot accepts arbitrary expressions over the current state
           (the existing arms all use unconditional arithmetic) and that the transition applies AFTER the
           read, per resume.")
  (input  (do
            (effect Clamp (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Clamp n
                ((bump (u) s (resume s (if (>= s 3) 3 (+ s 1)))))
                (do (Clamp.bump unit)
                    (Clamp.bump unit)
                    (Clamp.bump unit)
                    (Clamp.bump unit))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 3 Int64))
  (call   main (: 5 Int64))
  (output (: 3 Int64)))

(case "a ctl-style arm applying its continuation inside a match scrutinee resolves and folds"
  (doc    "The continuation binder `k` of a `ctl`-style arm must be in scope everywhere in the arm body,
           including inside a MATCH scrutinee. `(flip () s k (match (k 10) (z (* z 2))))` applies `k`
           lexically as the scrutinee of a match; `(k 10)` returns 10 into the delimited context, the match
           binds it to `z` and doubles it → 20. Regression pin: `k` used inside a match scrutinee previously
           reported a spurious CDZ0101 (the continuation binder occurrence was not recognized as a binder on
           that resolution path), while `k` applied directly in an operator operand worked.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main) (handle Amb 0 ((flip () s k (match (k 10) (z (* z 2))))) (Amb.flip))) (export main)))
  (output (: 20 Int64)))

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

(case "a ctl-style arm that reads the STATE binder AROUND its continuation application folds"
  (doc    "The state-referencing companion of the lexical-`ctl` fold above: the 5-part arm body references
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
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main) (handle G 100 ((y (x) s k (+ s (k x)))) (G.y 5))) (export main)))
  (output (: 105 Int64)))

(case "a ctl-style arm that LET-BINDS its continuation result then reads the state binder folds"
  (doc    "The let/do-bound-continuation companion of the two folds above. The arm binds the continuation
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
  (input  (do
            (effect G (op y (-> Int64 Int64)))
            (def (main) (handle G 100 ((y (x) s k (let ((r (k x))) (+ r s)))) (G.y 5))) (export main)))
  (output (: 105 Int64)))

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

(case "an inner handle of the SAME delegated effect discharges in-program inside the host block"
  (doc    "The SHADOW face beside the interpose-and-forward pin: the entrypoint delegates `A`, and INSIDE
           the host block an inner `(handle A 500 …)` re-binds the same effect — the inner perform
           discharges IN-PROGRAM (reads the handler seed 500), while the OUTER perform (outside the
           handle) still delegates to the host (7). Exactly ONE host call. A routing that let the inner
           perform escape to the host would consume a second (unsupplied) response; one that captured the
           outer perform into the handler would read 500 twice. 7 + 500 = 507. (rust-async: todo until
           its host+handle composition lands; wasm + rust pin the pass.)")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (A)
                (+ (A.get unit)
                   (handle A 500 ((get (_u) s (resume s s)))
                     (A.get unit)))))
            (export main)))
  (host-responses (respond a.get (: 7 Int64)))
  (host-calls (call a.get))
  (call   main (: 0 Int64)) (output (: 507 Int64)))

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

(case "a nested handler arm whose RESUME-VALUE performs the outer effect threads the advance to the continuation"
  (doc    "The stateful analogue of the interpose-and-forward case above, and the WORKING boundary of the
           recursive-nested-arm miscompile family (v-effects self-probe): the inner `B` handler's arm resumes
           with a VALUE that performs the OUTER `A` effect — `(step (u) t (resume (A.tick) t))`. `A.tick` reads
           the outer state (10) and advances it (→11); the resume VALUE is that 10, so `(B.step)` = 10. The
           continuation `(A.get)` then reads the ADVANCED 11 → `(+ 10 11)` = 21. Pins that an outer-effect
           advance made INSIDE a nested handler's resume-value threads correctly to a sibling reading the outer
           effect after — the shape folds via the inside-out path when the `B.step` caller is DIRECT (not
           behind a recursive callee). The recursive-caller variant of this shape currently drops the advance
           (a separate known miscompile, merged_nested_ctx merge-skip); this case + its non-recursive-helper
           twin below pin the folding boundary.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (handle B 0 ((step (u) t (resume (A.tick) t)))
                  (+ (B.step) (A.get))))) (export main)))
  (output (: 21 Int64)))

(case "a SIX-deep alternating A-B perform chain threads both nested states through every crossing"
  (doc    "The deep-interleave stress of the two-frame nesting above: six performs alternate A-B-A-B-A-B
           where each perform's ARGUMENT is the previous perform's result — `(B.b (A.a (B.b (A.a (B.b
           (A.a 0))))))`. Both arms fold the argument into the resume value AND advance their own state
           (`a` adds s then s+=1; `b` adds t then t+=10), so every crossing must read the value produced
           under the OTHER handler's frame and its own CURRENT state: 0→5→105→111→221→228→348 (s walks
           5,6,7; t walks 100,110,120). One wrong state snapshot or one stale intermediate anywhere in
           the six-step chain lands off the checksum. Pins the data-dependency chain BETWEEN two live
           handler frames at depth six — prior nesting pins cross at most twice.")
  (input  (do
            (effect A (op a (-> Int64 Int64)))
            (effect B (op b (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a (v) s (resume (+ v s) (+ s 1))))
                (handle B 100
                  ((b (v) t (resume (+ v t) (+ t 10))))
                  (B.b (A.a (B.b (A.a (B.b (A.a 0)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 348 Int64)))

(case "a recursive nested-op performer whose resume-VALUE reads the inner state around the outer perform declines cleanly"
  (doc    "The state-reading companion of the fold above. Here the inner `B.step` arm's resume VALUE reads the
           inner state binder `t` around the outer perform — `(step (u) t (resume (A.tick (+ t u)) t))`. The
           pre-spec-lift only lifts an inner-op call whose resume value is FREE of the inner state binder (the
           lift substitutes op params but not the state); lifting a state-reading value would orphan `t` onto
           the outer body spine (a CDZ0101 leak on a valid program — the github-liaison/Copilot #2077 review).
           So this shape is left UN-lifted and declines cleanly (the honest not-yet-reducible todo — threading
           an inner-state-reading resume value onto the outer body is the full spec-lift fold, a later
           increment). Pins that the state-reading face declines rather than leaks.")
  (input  (do
            (effect A (op tick (-> Int64 Int64)))
            (effect B (op step (-> Int64 Int64)))
            (def (loop (: n Int64)) (if (= n 0) 0 (+ (B.step n) (loop (- n 1)))))
            (def (main)
              (handle A 100 ((tick (a) s (resume (+ a s) (+ s 1))))
                (handle B 7 ((step (b) t (resume (A.tick (+ t b)) t)))
                  (loop 2))))
            (export main)))
  (output (: 218 Int64)))

(case "a NON-recursive helper calling a nested op whose resume performs the outer effect folds"
  (doc    "The non-recursive-helper twin of the resume-value-performs-outer case above (v-effects self-probe).
           A non-recursive `helper` calls the inner `B.step` (whose arm resumes with `(A.tick)`, performing the
           outer `A`), and the continuation reads `(A.get)`. Because `helper` is NON-recursive it INLINES, so
           the outer advance threads correctly: `(helper)` = 10, `A.tick` advanced A to 11, `(A.get)` = 11 →
           `(+ 10 11)` = 21. Pins the RECURSION boundary of the recursive-nested-arm miscompile: the SAME body
           behind a RECURSIVE caller drops the advance (merged_nested_ctx skips the merge because the
           accum-transformed recursive callee reads non-recursive at the merge decision), but a non-recursive
           caller folds — so the discriminator is specifically the recursive-specialization path, not the
           nested-arm-outer-perform shape itself.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (helper) (B.step))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (handle B 0 ((step (u) t (resume (A.tick) t)))
                  (+ (helper) (A.get))))) (export main)))
  (output (: 21 Int64)))

(case "a HOST-delegated perform in a nested arm's NEXT-STATE slot is served (sequences at the boundary)"
  (doc    "The host-routing boundary of the next-state-slot outer-perform family (v-effects self-probe, breaker
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
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main)
              (host (ask)
                (handle B 0 ((step (u) t (resume t (+ t (ask.ask)))))
                  (+ (* 10 (B.step)) (B.step)))))
            (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 100 Int64)))

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

(case "an abortive handler ISSUES a host call sequenced BEFORE the abort in its discarded body"
  (doc    "The complement of the case above (abort ELIDES a host call AFTER it): a delegated host call
           sequenced BEFORE the abort on the strict do-spine IS issued — its effect is committed before the
           abort abandons the rest. `(do (ask.ask) (Bail.bail 7))` under `Bail`: `ask.ask` runs (the host
           call is issued, response 100 discarded — the `do` evaluates it for effect), THEN `Bail.bail 7` —
           a non-resuming arm — abandons the `do`, so the handle value is the abort 7. The observed host-call
           sequence is `(call ask.ask)` (issued), NOT empty. Pins that the do-shape abort-fold preserves a
           FOREIGN HOST perform in the pre-abort prefix (the host analogue of the outer-effect pre-abort
           preservation): the discarded continuation drops only what comes AFTER the abort, never a
           side-effect already committed before it.")
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main)
              (host (ask) (handle Bail 0 ((bail (n) s n)) (do (ask.ask) (Bail.bail 7))))) (export main)))
  (host-responses (respond ask.ask (: 100 Int64)))
  (host-calls (call ask.ask))
  (output (: 7 Int64)))

(case "a host-delegated result SEEDS an in-program handler's initial state"
  (doc    "The host-to-handler data flow: the handle's SEED expression is itself a host-delegated perform —
           `(handle Ctr (Env.seed unit) …)` — so the host response (5) becomes the in-program handler's
           initial state, evaluated once before the handle's region runs. The two in-program ticks then
           read 5 and 6 (the seeded state advancing normally) → 56. Pins that the seed position accepts a
           performing expression whose own effect discharges at the ENCLOSING (here host) level — the
           config-fetch-then-run idiom (read a setting from the host, seed a counter/limiter with it).")
  (input  (do
            (effect Env (op seed (-> Unit Int64)))
            (effect Ctr (op next (-> Unit Int64)))
            (def (main)
              (host (Env)
                (handle Ctr (Env.seed unit)
                  ((next (u) s (resume s (+ s 1))))
                  (+ (* 10 (Ctr.next unit)) (Ctr.next unit)))))
            (export main)))
  (call   main)
  (host-responses (respond Env.seed (: 5 Int64)))
  (output (: 56 Int64)))

(case "an inner handler's SEED is a perform of an OUTER in-program effect"
  (doc    "The in-program analogue of the host-delegated-seed case above: an inner handler's SEED expression
           is itself a perform of an OUTER in-program effect — `(handle B (A.base) …)` where `A.base` homes
           to the enclosing `A` handler (not the host). The outer `A.base` reads A's state 5 (its arm resumes
           `s` unchanged) → 5 becomes B's initial state, evaluated once before B's region. B's two ticks then
           read 5 and 6 (B-state advancing) → `(+ 5 6)` = 11. Pins that the seed position accepts a performing
           expression whose effect discharges at an ENCLOSING in-program handler — the intra-program
           config-fetch-then-run idiom (the host-seed case's non-host twin).")
  (input  (do
            (effect A (op base (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main)
              (handle A 5 ((base (u) s (resume s s)))
                (handle B (A.base)
                  ((step (u) t (resume t (+ t 1))))
                  (+ (B.step) (B.step)))))
            (export main)))
  (output (: 11 Int64)))

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

(case "FOUR nested resumptive frames dispatched innermost-out"
  (doc    "The resumptive nesting pins stop at two frames; this stacks FOUR (distinct effects, distinct
           seeds at distinct place values) and dispatches innermost-out — D, C, B, A — so each perform
           is served by its own frame with zero escaping: 4000 + 300 + 20 + 5 = 4325. With the
           outermost-first sibling below, pins depth-4 frame bookkeeping in both traversal orders.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect C (op c (-> Unit Int64)))
            (effect D (op d (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n ((a (u) s (resume s (+ s 1))))
                (handle B 20 ((b (u) s (resume s (+ s 1))))
                  (handle C 300 ((c (u) s (resume s (+ s 1))))
                    (handle D 4000 ((d (u) s (resume s (+ s 1))))
                      (+ (D.d) (+ (C.c) (+ (B.b) (A.a)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4325 Int64)))

(case "four nested frames dispatched OUTERMOST-first — every outer perform escapes live inner frames"
  (doc    "The escape-order stress of the depth-4 stack: dispatching A, B, C, D means the `A.a` perform
           must route past THREE live inner frames (B, C, D) to its handler, `B.b` past two, `C.c`
           past one — the maximal-escape traversal. Same checksum as the innermost-out sibling (4325):
           the answer must not depend on which frame order the body dispatches.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect C (op c (-> Unit Int64)))
            (effect D (op d (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n ((a (u) s (resume s (+ s 1))))
                (handle B 20 ((b (u) s (resume s (+ s 1))))
                  (handle C 300 ((c (u) s (resume s (+ s 1))))
                    (handle D 4000 ((d (u) s (resume s (+ s 1))))
                      (+ (A.a) (+ (B.b) (+ (C.c) (D.d)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4325 Int64)))

(case "an inner abortive handler preserves an OUTER effect's advance committed before the abort (do-shape)"
  (doc    "The abort-fold's outer-advance preservation (v-effects, breaker ao1). An inner abortive `B`-handle
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
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (B.bail 99)))))
                  (+ b (A.get))))) (export main)))
  (output (: 110 Int64)))

(case "an inner abort preserves TWO outer advances committed before it (multi-step outer trace)"
  (doc    "The multi-advance face of the outer-advance preservation (breaker ao4): the FULL outer trace of the
           aborted inner computation is preserved, not just the last step. TWO foreign `(A.tick)` performs run
           on the inner `B`-handle's do-spine before the abort: `(do (A.tick) (A.tick) (B.bail 99))`. Each
           advances A-state (10→11→12); then `B.bail` abandons B. The outer `(A.get)` must read 12 → `(+ 99
           12)` = 111. Before the fold BOTH advances were discarded (A.get read 10 → 109). Pins that the abort-
           fold threads every pre-abort foreign step, not only the final one.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (A.tick) (B.bail 99)))))
                  (+ b (A.get))))) (export main)))
  (output (: 111 Int64)))

(case "an inner abort ELIDES an outer perform sequenced AFTER it (dead-path control)"
  (doc    "The control companion of the outer-advance preservation above: a foreign `(A.tick)` sequenced AFTER
           the abort in the inner do-spine — `(do (B.bail 99) (A.tick))` — is genuinely UNREACHABLE (the abort
           abandons the rest of the sequence), so it is correctly ELIDED: A-state is NOT advanced, the outer
           `(A.get)` reads the seed 10 → `(+ 99 10)` = 109. Pins that the abort-fold preserves only the pre-
           abort prefix (a committed advance) and drops the post-abort dead tail — the discriminator is
           evaluation ORDER relative to the abort, not the mere presence of a foreign perform in the body.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (do (B.bail 99) (A.tick)))))
                  (+ b (A.get))))) (export main)))
  (output (: 109 Int64)))

(case "an inner abort in a NON-FINAL do-statement elides the DEAD suffix after it (pre-abort advance kept)"
  (doc    "The dead-suffix control for the do-shape abort-fold (github-liaison review follow-on on #2002/#2014,
           self-probed). The aborting `(B.bail 99)` is a NON-FINAL do-statement with a foreign `(A.tick)` BOTH
           before AND after it — `(do (A.tick) (B.bail 99) (A.tick))` under B. The PRE-abort `A.tick` commits
           A-state 10→11 (kept); the abort abandons the rest, so the trailing `(A.tick)` is DEAD and must NOT
           run. Value is the abort 99, outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fix the do-arm
           kept threading past the abort and set `last` to the DEAD final `(A.tick)` (dropping the abort value
           and FORCING the dead tick) → 23; a multivalue self-call in the dead suffix was likewise forced (34).
           Fixed by BREAKING the do-item loop when a non-final item fires the abort: the abort value is the do's
           value, the dead suffix is never threaded. Composes with the pre-abort-prefix preservation (the
           kept `A.tick` still advances) — this pins BOTH halves in one body.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (do (A.tick) (B.bail 99) (A.tick)))))
                  (+ b (A.get))))) (export main)))
  (output (: 110 Int64)))

(case "an inner abort preserves an OUTER advance committed in a MATCH-SCRUTINEE before it (scrutinee collapse)"
  (doc    "The MATCH-SCRUTINEE face of the outer-advance preservation (breaker ao9). The foreign `(A.tick)`
           and the abort sit on the strict do-spine of a `match` SCRUTINEE — `(match (do (A.tick) (B.bail 99))
           (x x))` under B. The scrutinee is evaluated BEFORE any arm; it ABORTS, so no arm runs and the match
           collapses to the scrutinee's value — but the pre-abort `A.tick` committed A-state 10→11 and must
           survive → outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fix the `Match` thread arm wrapped
           the aborted scrutinee in a dead `(match (do (A.tick) 99) (x x))` whose bare-abort collapse dropped
           `A.tick` → 109. Fixed by collapsing the match to the scrutinee rewrite when threading the scrutinee
           fires a NEW abort (no arm runs), so the enclosing fold discharges the `(do (A.tick) 99)` prefix.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (match (do (A.tick) (B.bail 99)) (x x)))))
                  (+ b (A.get))))) (export main)))
  (output (: 110 Int64)))

(case "an inner abort preserves an OUTER advance committed in a STRICT OPERAND before it (operand-lift)"
  (doc    "The STRICT-OPERAND face of the outer-advance preservation (breaker ao5; the do-shape face is pinned
           above). The foreign `(A.tick)` is a strict `+` OPERAND evaluated before the abort — `(+ (A.tick)
           (B.bail 99))` under B — not a `do`-statement. `A.tick` resumes, COMMITTING A-state 10→11; then
           `B.bail` (non-resuming) abandons B's OWN handle, so the `+` never completes and `b` = the abort
           value 99. The committed A-advance must survive → outer `(A.get)` reads 11 → `(+ 99 11)` = 110.
           Before the operand-lift the bare-abort collapse discarded `(A.tick)` (a dead `+` wrapper), reading
           the seed 10 → 109. Fixed by lifting the pre-abort foreign operand into a for-effect `do` prefix
           `(do (A.tick) 99)` — the same shape the do-arm produces — which the do-shape abort-fold then
           preserves. Distinct from the do-shape only in the CONSUMING form (`+` operand vs `do` statement).")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (+ (A.tick) (B.bail 99)))))
                  (+ b (A.get))))) (export main)))
  (output (: 110 Int64)))

(case "a strict-operand abort in a DEEP-nested handler stack keeps its 99 when the advances are UNOBSERVED"
  (doc    "The soundness control for the operand-lift: the SAME strict-operand-abort-with-foreign-prefix shape
           `(+ (A.a) (+ (B.b) (Bail.bail 99)))` under `handle A…(handle B…(handle Bail…))`, but here the outer
           advances are UNOBSERVED (nothing reads A/B after the aborted handle). `A.a` and `B.b` resume (their
           arms pass state through, no increment); `Bail.bail 99` abandons; the value is the abort value 99.
           The operand-lift rewrites the dead `+` nest into `(do (A.a) (do (B.b) 99))` — the foreign prefix
           runs for effect (unobserved) and the value stays 99. Pins that the lift is sound BOTH ways: it
           preserves an OBSERVED advance (the 110 case above) AND leaves an UNOBSERVED one at the correct value
           (this case), because a for-effect `do` prefix only runs the performs — it never changes the abort
           value. (Distinguishes the lift from a naive rewrite that would leak the prefix into the value.)")
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

(case "an inner abort preserves an OUTER advance committed in an IF-BRANCH before it (branch do-shape)"
  (doc    "The IF-BRANCH face of the outer-advance preservation (v-effects self-probe; the direct do-shape and
           strict-operand faces are pinned above). The foreign `(A.tick)` and the abort sit on the strict
           do-spine of an `if` BRANCH — `(if true (do (A.tick) (B.bail 99)) 5)` under B. The branch's abort is
           branch-local (the `if` is the inner handle's value), but the pre-abort `A.tick` committed A-state
           10→11 and must survive → outer `(A.get)` reads 11 → `(+ 99 11)` = 110. Before the fix the
           branch-local collapse (`thread_branch_local_abort_with_out`) returned the BARE abort value,
           discarding the do-arm's sound `(do (A.tick) 99)` branch rewrite → 109. Fixed by the same do-shape
           gate as the direct fold, applied to the branch rewrite (the `if` condition is pure — a performing
           condition with a second branch advance is a separate face).")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (if true (do (A.tick) (B.bail 99)) 5))))
                  (+ b (A.get))))) (export main)))
  (output (: 110 Int64)))

(case "an inner abort preserves BOTH a PERFORMING-CONDITION advance AND an if-branch advance before it (ao10)"
  (doc    "The performing-condition face — the separate face the if-branch case above flagged. The `if`
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
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v))
                           (if (> (A.tick) 5) (do (A.tick) (B.bail 99)) 5))))
                  (+ b (A.get))))) (export main)))
  (output (: 111 Int64)))

(case "an inner abort preserves an OUTER advance committed in a MATCH-ARM body before it (arm do-shape)"
  (doc    "The MATCH-ARM-BODY face of the outer-advance preservation, sharing the branch-local abort helper
           with the if-branch face above. The foreign `(A.tick)` and the abort sit on the strict do-spine of a
           `match` ARM body — `(match 0 (_ (do (A.tick) (B.bail 99))))` under B. The arm's abort is arm-local,
           but `A.tick` committed A-state 10→11 → outer `(A.get)` reads 11 → 110. Same fix + gate as the
           if-branch (both route through `thread_branch_local_abort_with_out`, so one fix covers both branch
           and arm-body positions); before it the arm collapse dropped `A.tick` → 109.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (match 0 (_ (do (A.tick) (B.bail 99)))))))
                  (+ b (A.get))))) (export main)))
  (output (: 110 Int64)))

(case "a single handler with both a resuming and an abortive arm dispatches each op to its own arm kind"
  (doc    "One handler for ONE effect `E` declaring TWO operations whose arms are DIFFERENT KINDS — `get`
           resumes, `bail` abandons — so the fold must dispatch each performed op to its own arm kind within
           a single handler context (distinct from the nested three-separate-handler abort above, where each
           kind is its own handler). Body `(+ (E.get) (E.bail 7))` seeded 0: `E.get` resumes with 5, then
           `E.bail 7` — a NON-resuming arm — ABANDONS the pending `(+ 5 …)` and yields the arm value 7 as the
           whole handle's value (NOT 5+7). Pins that a mixed-arm handler routes the resuming op through the
           resume fold AND the abortive op through the non-local exit, in one handler.")
  (input  (do
            (effect E (op get (-> Unit Int64)) (op bail (-> Int64 Int64)))
            (def (main)
              (handle E 0 ((get (u) s (resume 5 s)) (bail (b) s b)) (+ (E.get) (E.bail 7)))) (export main)))
  (output (: 7 Int64)))

(case "a single mixed handler uses only its resuming arm when the abortive op is never performed"
  (doc    "The control companion of the mixed-arm case above: the SAME two-op handler (`get` resuming,
           `bail` abortive) but the body performs ONLY the resuming op — the abortive arm is present but
           never reached, so nothing abandons. Body `(+ (E.get) 100)` seeded 0: `E.get` resumes with 5,
           `(+ 5 100)` = 105. Pins that the mere PRESENCE of an abortive arm does not perturb the resuming
           path — the handle folds to the ordinary resumed value when the abortive op is not performed.")
  (input  (do
            (effect E (op get (-> Unit Int64)) (op bail (-> Int64 Int64)))
            (def (main)
              (handle E 0 ((get (u) s (resume 5 s)) (bail (b) s b)) (+ (E.get) 100))) (export main)))
  (output (: 105 Int64)))

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

(case "a handler ARM gates its answer on a CAPTURED Set from the enclosing scope"
  (doc    "The arm-side twin of the body-reads-enclosing-parameter case above: it is the ARM (not the body)
           that reaches a heap value defined in `main`'s scope. `allow = Set.of [2 5 9]` is captured by the
           `check` arm, which answers `(if (Set.contains allow v) 1 0)` per op ARGUMENT. Three membership
           probes (5 ∈, 3 ∉, 9 ∈) place-value to 101. Pins that the fold keeps the arm anchored where the
           handler sat lexically, so a free heap binding resolves up the original chain from inside the arm.")
  (input  (do
            (effect St (op check (-> Int64 Int64)))
            (def (main (: n Int64))
              (do
                (def allow (Set.of (list 2 5 9)))
                (handle St 0
                  ((check (v) s (resume (if (Set.contains allow v) 1 0) s)))
                  (+ (* 100 (St.check n)) (+ (* 10 (St.check 3)) (St.check 9))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64)))

(case "Set.contains on a Map-looked-up Set with a perform-threaded element"
  (doc    "The set/elem same-base emit witnessed clean (the mixed-width siblings of this shape — a
           looked-up closure applied to a perform result, and Bytes.slice of a looked-up Bytes with
           perform operands — were i32/i64 scratch-alias miscompiles, both fixed and pinned): the Set
           comes back through `Map.lookup` and the membership PROBE is a perform result. Same-width
           slots cannot type-collide; this pins no value clobber either — 5 ∈ {2 5 9} → 10, 6 ∉ → 0 →
           10.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 (Set.of (list 2 5 9))))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some st)
                      (+ (if (Set.contains st (St.next)) 10 0)
                         (if (Set.contains st (St.next)) 1 0)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))

(case "two sequential lookups on the same Map with perform-threaded keys stay independent"
  (doc    "The map/key same-base emit witnessed clean (see the sibling pin above for the fixed
           mixed-width class): two `Map.lookup inner (St.next)` calls in one sum, each key a fresh
           perform result (5 → 100, 6 → 250 → 350). Pins that consecutive lookup emits with live
           perform-threaded key operands do not share (or clobber) scratch state.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def inner (Map.insert (Map.insert Map.empty 5 100) 6 250))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (+ (match (Map.lookup inner (St.next))
                       ((Some v) v)
                       ((None _u) -1))
                     (match (Map.lookup inner (St.next))
                       ((Some v) v)
                       ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 350 Int64)))

(case "a TWO-resume-site arm branching on a CAPTURED Map folds — the branch reads heap, not state"
  (doc    "The first-served face of the multi-resume-site family: an arm with two resume sites carrying
           DIFFERENT states per site — the hit path advances the count `(resume v (+ s 1))`, the miss path
           holds it `(resume 0 s)` — folds through FOUR performs, branching on a CAPTURED Map
           (`Map.lookup table k`). (Historically the state-reading sibling declined and this captured-heap
           face pinned the boundary; the two-hole refold re-anchor now serves state-reading conditions
           too — the match-arm and state-condition faces are pinned nearby.) Lookups: 1→100 (s→1),
           7→miss→0 (s stays 1), 2→250 (s→2), then `hits` reports 2 → 100+0+250+2000 = 2350. Pins the
           captured-table routing idiom — a real-world lookup-with-hit-count handler.")
  (input  (do
            (effect St (op price (-> Int64 Int64)) (op hits (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert (Map.insert Map.empty 1 100) 2 250))
                (handle St 0
                  ((price (k) s
                    (match (Map.lookup table k)
                      ((Some v) (resume v (+ s 1)))
                      ((None _u) (resume 0 s))))
                   (hits (u) s (resume s s)))
                  (+ (St.price 1) (+ (St.price 7) (+ (St.price 2) (* 1000 (St.hits))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2350 Int64)))

(case "a two-site arm branching on the STATE folds (the refold re-anchor serves state-reading conditions)"
  (doc    "Historically THE decline face of the multi-site family — `(if (> s 5) …)` reads the state binder
           and the arm resumes in both branches — now served by the two-hole refold re-anchor (the
           #2305-era fix; condition-agnostic). Seed 7 never changes (`(resume v s)` / `(resume -1 s)`), so
           both reads take the true branch: 5 + 10·6 = 65. The never-miscompile lib pin asserts this same
           fold; the corpus case pins it end-to-end on all three targets.")
  (input  (do
            (effect Src (op read (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Src 7
                ((read (v) s (if (> s 5) (resume v s) (resume (- 0 1) s))))
                (+ (Src.read n) (* 10 (Src.read (+ n 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 65 Int64)))

(case "a trailing state-REPLACING single-site op is served after a two-site arm's performs"
  (doc    "The arm-shape MIXING boundary, trailing-served face: the refold serves any mix of MULTI-site
           arms in any dispatch order, but a SINGLE-site arm (like `reset` here) dispatched among
           multi-site performs declines — UNLESS it trails, as here: sift 20 → 20 (s 1), sift 30 → 30
           (s 2), reset → 2 (state becomes 100, unobserved) → 52. A trailing dispatch sits outside the
           multi-site continuation chain and folds; the same reset dispatched before or between the
           sifts declines (that face is pinned as a todo-witness nearby). Making the interleaved arm
           itself multi-site serves the same order — the rule is arm-shape uniformity at the handler's
           own frame, not dispatch position per se.")
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op reset (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (reset (u) s (resume s 100)))
                (+ (St.sift 20) (+ (St.sift n) (St.reset)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 52 Int64)))

(case "a trailing state-READING single-site op is served after one two-site perform"
  (doc    "The minimal trailing-served face of the arm-shape mixing boundary: ONE two-site sift then a
           trailing single-site peek — sift 20 passes (s → 1), peek reads 1 → 21. With the
           multi-perform sibling above, pins that trailing serves regardless of perform count — while
           even a single LEADING single-site dispatch (peek first) declines. The full rule: multi-site
           arms mix freely; single-site arms among multi-site performs decline except trailing.")
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (peek (u) s (resume s s)))
                (+ (St.sift 20) (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21 Int64)))

(case "TWO different two-site ops dispatched in segments fold (multi-multi mixing, A A B B)"
  (doc    "Arm-shape uniformity, the two-op face: BOTH arms are two-site, dispatched as two siftAs then
           two siftBs — 20 pass (s 1), 3 fail, 7 pass ×2 (s 11), 4 fail → 20 + 0 + 14 − 1 = 33. With
           the interleaved sibling below, pins that any mix of MULTI-site arms folds regardless of
           dispatch grouping — the single-site-among-multi decline is about arm SHAPE, not op count.")
  (input  (do
            (effect St (op siftA (-> Int64 Int64)) (op siftB (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((siftA (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (siftB (v) s (if (> v 5) (resume (* v 2) (+ s 10)) (resume -1 s))))
                (+ (St.siftA 20) (+ (St.siftA 3) (+ (St.siftB 7) (St.siftB 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 33 Int64)))

(case "two two-site ops INTERLEAVED A-B-A fold (order does not matter when all arms are multi-site)"
  (doc    "The interleave face of multi-multi mixing: siftA, then siftB, then siftA again — the exact
           dispatch pattern that DECLINES when the middle arm is single-site folds when it is two-site.
           20 pass (s 1), 7 doubled (s 11), 30 pass (s 12) → 20 + 14 + 30 = 64. The strongest witness
           that the boundary is arm-shape uniformity, not dispatch position.")
  (input  (do
            (effect St (op siftA (-> Int64 Int64)) (op siftB (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((siftA (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (siftB (v) s (if (> v 5) (resume (* v 2) (+ s 10)) (resume -1 s))))
                (+ (St.siftA 20) (+ (St.siftB 7) (St.siftA 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 64 Int64)))

(case "the interleaved middle op SERVES when made two-site itself (the shape-not-position witness)"
  (doc    "The decisive discriminator: the sift-peek-sift order whose single-site peek declines is
           served verbatim once peek's arm is TWO-site — `(if (> s 0) (resume s s) (resume -1 s))`.
           sift 20 → 20 (s 1), peek → 1 (s > 0 path), sift 30 → 30 (s 2) → 51. Same program order,
           only the arm shape changed: the refold rebuilds all dispatched arms in one pass and serves
           any all-multi-site mix.")
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (peek (u) s (if (> s 0) (resume s s) (resume -1 s))))
                (+ (St.sift 20) (+ (St.peek) (St.sift 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64)))

(case "a THREE-site arm (nested if, three resume sites) folds"
  (doc    "The refold generalizes past two resume sites: a nested-if arm with THREE resumes — >20 pays
           ×10 and jumps the state, >10 passes and counts, else zero-holds. rank 25 → 250 (s 100),
           rank 15 → 15 (s 101), rank 5 → 0 → 265. Site count is not the boundary; arm-shape mixing
           is.")
  (input  (do
            (effect St (op rank (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((rank (v) s
                  (if (> v 20) (resume (* v 10) (+ s 100))
                    (if (> v 10) (resume v (+ s 1)) (resume 0 s)))))
                (+ (St.rank 25) (+ (St.rank 15) (St.rank n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 265 Int64)))

(case "a MATCH-shaped arm with three resume sites folds (sum-dispatch, not if)"
  (doc    "The refold is not if-specific: the arm dispatches on `(% v 3)` through a MATCH with a resume
           in every branch — 6 → ×10 (s+1), 7 → identity, 5 → negated (s+100): 60 + 7 − 5 = 62. Pins
           multi-site service for match-shaped arm bodies alongside the nested-if face above.")
  (input  (do
            (effect St (op class (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((class (v) s
                  (match (% v 3)
                    (0 (resume (* v 10) (+ s 1)))
                    (1 (resume v s))
                    (_ (resume (- 0 v) (+ s 100))))))
                (+ (St.class 6) (+ (St.class 7) (St.class n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64)))

(case "a THREE-site and a TWO-site arm interleaved fold (site counts mix freely)"
  (doc    "Exact site-count uniformity is NOT required — a 3-site rank and a 2-site sift interleave
           (rank, sift, rank) and fold: 250 (s 100), 7 (s 110), 15 (s 111) → 272. Only the
           multi-vs-single distinction gates the mix.")
  (input  (do
            (effect St (op rank (-> Int64 Int64)) (op sift (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((rank (v) s
                  (if (> v 20) (resume (* v 10) (+ s 100))
                    (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                 (sift (v) s (if (> v 5) (resume v (+ s 10)) (resume -1 s))))
                (+ (St.rank 25) (+ (St.sift 7) (St.rank n)))))
            (export main)))
  (call   main (: 15 Int64)) (output (: 272 Int64)))

(case "a trailing ABORT after multi-site performs reads the fully-advanced state"
  (doc    "The abort corollary of arm-shape mixing: an aborting arm has ZERO resume sites, so it counts
           as non-multi — dispatched between multi-site performs it declines, but TRAILING it folds:
           sift 20 (s 1), sift 30 (s 2), then bail aborts with s·10 = 20 and the +1000 shell proves
           the continuation is discarded → 1020. The abort must read the state BOTH sifts advanced.")
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (+ 1000
                (handle St 0
                  ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                   (bail (u) s (* s 10)))
                  (+ (St.sift 20) (+ (St.sift n) (St.bail))))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 1020 Int64)))

(case "the arm-shape rule is FRAME-RELATIVE: a nested single-site handler dispatching mid-sequence is invisible"
  (doc    "A multi-site OUTER handler folds even though a nested single-site handler (a SEPARATE
           effect) dispatches between the outer sifts — from the outer refold's frame its own dispatch
           sequence is contiguous sift-sift; the inner bump belongs to the nested frame below. 20 (s 1)
           + 100 (inner, t → 110) + 30 (s 2) → 150. (The inverse — an OUTER perform escaping through a
           multi-site INNER handler's chain — declines: that dispatch IS foreign at the inner frame.)")
  (input  (do
            (effect Out (op sift (-> Int64 Int64)))
            (effect In (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Out 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                (handle In 100
                  ((bump (u) t (resume t (+ t 10))))
                  (+ (Out.sift 20) (+ (In.bump) (Out.sift n))))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 150 Int64)))

(case "a two-site arm branching on the OP ARGUMENT with a hit-count state folds (threshold sift)"
  (doc    "The op-argument face of the served multi-site family: `(if (> v 10) …)` reads the op PARAM;
           the pass path resumes the value and counts it, the fail path resumes 0 and holds. Three sifts
           (20 pass, 5 fail, 30 pass) then the count: 20 + 0 + 30 + 2·1000 = 2050. With the state-reading
           face above and the captured-heap face before it, the family folds regardless of WHAT the
           condition reads.")
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (count (u) s (resume s s)))
                (+ (St.sift 20) (+ (St.sift n) (+ (St.sift 30) (* 1000 (St.count)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2050 Int64)))

(case "a two-site arm whose condition reads the ARG AND the STATE together folds"
  (doc    "The compound face: `(> v s)` compares the op argument against the CURRENT state, so the branch
           decision itself depends on how many hits came before — sift 5 at s=0 passes (s→1), sift 0 at
           s=1 fails, sift 3 at s=1 passes (s→2): 5 + 0 + 3 + 2·1000 = 2008. The strongest single witness
           that the refold's re-anchored continuation sees the LIVE state at every dispatch.")
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v s) (resume v (+ s 1)) (resume 0 s)))
                 (count (u) s (resume s s)))
                (+ (St.sift n) (+ (St.sift 0) (+ (St.sift 3) (* 1000 (St.count)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2008 Int64)))

(case "a multi-site arm folds while the handle BODY reads an enclosing binding (the re-anchored free var)"
  (doc    "REPRO of the body-free-var orphan (breaker pm-family; fixed by the two-hole refold re-anchor):
           with a two-site arm and ≥2 performs, the multi-perform continuation-rebuild used to copy the
           surrounding body WITHOUT re-anchoring `n`, so this valid program hit a false CDZ0101 'unbound
           name n'. Now `n` resolves up the original chain: 5 + 100 + 111 + 111 = 327. The single-perform
           and single-site siblings never broke; ≥2 performs × multi-site × a body free-var was the exact
           conjunct.")
  (input  (do
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
                (+ n (+ (St.price 1) (+ (St.price 7) (St.price 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 327 Int64)))

(case "a LET-bound local (not a param) survives the multi-site continuation rebuild"
  (doc    "The let-binder face of the body-free-var repro above: `m = n·2` is a derived local read after
           three performs through a two-site arm. The orphan hit ANY enclosing binder (params and lets
           alike); the re-anchor restores both. 10 + 100 + 111 + 111 = 332.")
  (input  (do
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (let ((m (* n 2)))
                (handle St 0
                  ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
                  (+ m (+ (St.price 1) (+ (St.price 7) (St.price 2)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 332 Int64)))

(case "a two-site arm with HEAP resume values (empty vs two-element list) folds"
  (doc    "The heap-payload face of the served multi-site family: the branches resume DIFFERENT list
           shapes — `(list v v)` on pass, `(list)` on fail — and the body consumes lengths at place
           values: grab 5 → len 2, grab 1 → len 0, grab 4 → len 2 → 2 + 0 + 200 = 202. Pins that the
           refold is not scalar-only: each dispatch's resume value allocates (or not) per its own branch.")
  (input  (do
            (effect St (op grab (-> Int64 (List Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((grab (v) s (if (> v 1) (resume (list v v) (+ s 1)) (resume (list) s))))
                (+ (List.len (St.grab n)) (+ (* 10 (List.len (St.grab 1))) (* 100 (List.len (St.grab 4)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 202 Int64)))

(case "a two-site arm over a HEAP STATE with a body free-var and a second state-reading op folds"
  (doc    "The heap-STATE face of the body-free-var family (breaker ts1, fixed #2336). Unlike the
           heap-resume-value case above, here the STATE itself is a `List` threaded through `s`, the
           arm has TWO resume sites, and a SECOND state-reading op (`tally`) reads the advanced state
           mid-chain — while the body reads main's param `n`. Before the fix the two-hole refold rebuilt
           the arm's `if` condition `(> v 10)` (op-arg `v`↦`n`) via push_list, overwriting the shared `n`
           node's parent and detaching it → false CDZ0101 'unbound n'. Now the substituted arm body is
           anchored + resolved before the refold rebuild (pin-before-copy). feed 20 pass ([20]), feed n=5
           miss (hold), feed 30 pass ([20,30]), tally = len 2 → 20 + 0 + 30 + 1000·2 = 2050.")
  (input  (do
            (effect St (op feed (-> Int64 Int64)) (op tally (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (list)
                ((feed (v) s (if (> v 10) (resume v (List.push s v)) (resume 0 s)))
                 (tally (u) s (resume (List.len s) s)))
                (+ (St.feed 20) (+ (St.feed n) (+ (St.feed 30) (* 1000 (St.tally)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2050 Int64)))

(case "a two-site arm with a second op that REPLACES the state mid-chain declines cleanly"
  (doc    "The decline BOUNDARY of the served family (breaker sy1). The rule is arm-shape UNIFORMITY at the
           handler's frame: multi-site arms mix freely (any dispatch order), and a single-site arm among
           multi-site performs declines except trailing. Here `emit` is a two-site (multi-site) arm but the
           second op `flip` — `(flip (u) s (resume 0 (Symbol.of \"quiet\")))` — is SINGLE-site (it resumes
           once, replacing the state with a different value), dispatched BETWEEN the two `emit` performs, so
           the refold must mix a one-hole and a two-hole rebuild mid-chain and declines. (Making `flip`
           itself multi-site would serve the same order — confirmed by re-derivation.) The two-hole refold
           cannot yet thread a non-uniform (single-site) op through the middle of a multi-site continuation
           chain. This is an HONEST decline (a fold-capability gap, never a wrong value), NOT the ts1/ag5
           false-CDZ0101 orphan (those were bugs, fixed). The eventual fold: emit(5) sees `loud` → 5·100 =
           500, flip sets state `quiet` and resumes 0, emit(3) sees `quiet` → 3 → 500 + 0 + 3 = 503. Pins
           the boundary as a clean todo, not a leak.")
  (input  (do
            (effect St (op emit (-> Int64 Int64)) (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "loud")
                ((emit (v) s (if (= s (Symbol.of "loud")) (resume (* v 100) s) (resume v s)))
                 (flip (u) s (resume 0 (Symbol.of "quiet"))))
                (+ (St.emit n) (+ (St.flip) (St.emit 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 503 Int64)))

; A do-local value def in a handle body must stay in scope for a perform's ARGUMENT (breaker FINDING;
; v-effects e49c698a1). The handle-body folds dropped non-final do-items and re-spliced only a survivor,
; orphaning a `(def v e)` that a later perform arg referenced → a false CDZ0101 'unbound name'; the
; semantically identical `let`-bound form rebuilt its scope and worked. The fix normalizes a leading
; do-local value def to a `let` up front in reduce_handle, so every consumer — including the perform-arg
; path — sees the scoped binding. Pinned as a REPRO + let-twin regression pair. (A do-def flowing into a
; RESUME arg in an arm, and a do-def in a NON-perform arg, were always fine — this was specific to the
; perform-arg path in the body.)
(case "a do-def value in a handle body flows into a perform's argument and stays in scope"
  (doc    "FINDING repro (v-effects e49c698a1). Inside the handle body, `(def v (+ u 2))` is referenced from
           the ARGUMENT of `(Ask.ask v)` and again after it; before the fix the body fold orphaned `v` →
           CDZ0101 unbound. Now the do-local value def is scoped for the perform-arg path. `run 5`: v = 7,
           `(Ask.ask 7)` resumes 7·2 = 14, plus v = 7 → 21. Both backends.")
  (input  (do
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

(case "a chain of perform-fed let inits — each binding feeds the next perform's argument"
  (doc    "The sequential-dependency face of let × effects: three lets where each init's perform takes
           the PREVIOUS binding as its argument — a = add(5) = 5 (s 1), b = add(a) = 6 (s 2),
           c = add(b) = 8 (s 3) → 5 + 6 + 8 = 19. Each binding must be fully resolved before the
           next dispatch marshals it; a stale or reordered binding read skews the whole chain. (The
           pinned let cases cover bindings used AFTER performs; this pins bindings FEEDING the next.)")
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((add (v) s (resume (+ v s) (+ s 1))))
                (let ((a (St.add n)))
                  (let ((b (St.add a)))
                    (let ((c (St.add b)))
                      (+ a (+ b c)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 19 Int64)))

(case "a perform in the body's IF CONDITION gates a second perform in the branch"
  (doc    "Effect-gated dispatch, the true path: the condition's `(St.check)` fires (reads 5, state →
           6), 5 > 3 holds, so the branch's second check fires and reads the ADVANCED 6. The
           condition's dispatch must complete (and its advance commit) before the branch's dispatch
           reads the state.")
  (input  (do
            (effect St (op check (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1))))
                (if (> (St.check) 3) (St.check) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "the false branch: the condition's perform fires, the branch's does NOT (same program)"
  (doc    "The other runtime path of the effect-gated dispatch above: at seed 1 the condition's check
           reads 1, 1 > 3 fails, and the untaken branch's perform must NOT fire — the answer is the
           else's 0, and a speculative or hoisted dispatch of the branch perform would be observable
           as a state advance (or a wrong value) here.")
  (input  (do
            (effect St (op check (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1))))
                (if (> (St.check) 3) (St.check) 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 0 Int64)))

(case "performs in BOTH and-operands — the second fires when the first passes"
  (doc    "Short-circuit booleans × effects, the fire-both path: both `and` operands perform. At seed 5
           the first check reads 5 (> 3 passes, state → 6), so the SECOND fires and reads 6 (> 4
           passes, state → 7); the trailing count reads the DOUBLE advance → 700. The state is a
           dispatch counter — the checksum encodes exactly how many operand performs ran.")
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (and (> (St.check) 3) (> (St.check) 4))
                  (* 100 (St.count))
                  (St.count))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 700 Int64)))

(case "the and SHORT-CIRCUITS: the second operand's perform must NOT fire (state proves it)"
  (doc    "The elision path of the same program: at seed 1 the first check reads 1 (> 3 FAILS), the
           `and` short-circuits, and the second operand's perform must NOT fire — the count reads
           exactly ONE advance (2). Short-circuit evaluation is the language's only by-value runtime
           expression elision; an eager or reordered boolean lowering would fire the second dispatch
           and read 3.")
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (and (> (St.check) 3) (> (St.check) 4))
                  (* 100 (St.count))
                  (St.count))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2 Int64)))

(case "the OR short-circuits on a true first operand — the second perform must NOT fire"
  (doc    "The `or` elision path: at seed 5 the first check reads 5 (> 3 holds), the `or`
           short-circuits, the second operand's perform never fires — the count reads exactly one
           advance (6). The or-twin of the and-elision pin above.")
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (or (> (St.check) 3) (> (St.check) 0))
                  (St.count)
                  (* 100 (St.count)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "a false first operand falls through — the OR's second perform fires (same program)"
  (doc    "The fall-through path: at seed 1 the first check reads 1 (> 3 fails), so the `or`
           evaluates its second operand — that perform fires (reads 2, > 0 holds) and the count
           reads TWO advances (3). With the three sibling pins, all four short-circuit paths carry
           dispatch-count-proven witnesses.")
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (or (> (St.check) 3) (> (St.check) 0))
                  (St.count)
                  (* 100 (St.count)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3 Int64)))

(case "a perform under NOT in a condition (the negated dispatch gate)"
  (doc    "The remaining boolean operator: the condition wraps the perform in `not` — check reads 1
           (> 3 fails), the not flips it, the then-branch runs and count reads the single advance
           (100·2 = 200). Completes the boolean-op set (and/or short-circuits + not) over effect
           dispatches.")
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (not (> (St.check) 3))
                  (* 100 (St.count))
                  (St.count))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 200 Int64)))

; ============ Optimizer-exclusion chapter: LICM / CSE / DCE / inlining each have extensive pins
; for PURE code; these six pin the EFFECT-dispatch exclusion boundary that keeps those
; optimizations sound — a perform is never invariant, never a common subexpression, never dead,
; and never shared across inlined call sites. ============

(case "a recursive loop whose CONDITION performs — each iteration RE-dispatches (never hoisted)"
  (doc    "The LICM exclusion: the loop bound is a perform against a SHRINKING quota (the arm
           decrements per read: 5, 4, 3, 2), so the loop terminates when i catches the falling bound
           — acc 0+1+2 = 3. A hoist that treated the 'invariant-looking' condition as pure would
           read the quota once (5) and run five iterations (acc 10). The pure-invariant LICM pins
           (incl. trap-equivalence) live in 02-binding; this is their effect-side boundary.")
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (go (: i Int64) (: acc Int64))
              (if (< i (St.quota)) (go (+ i 1) (+ acc i)) acc))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (- s 1))))
                (go 0 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64)))

(case "two IDENTICAL performs are distinct dispatches — never CSE'd into one"
  (doc    "The CSE exclusion, minimal form: `(+ (St.next) (St.next))` — two textually identical
           performs read 5 then 6 → 11. A common-subexpression merge would compute one dispatch and
           double it (10).")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))

(case "identical PURE subterms around distinct performs — pure sharing must not merge dispatches"
  (doc    "The subtler CSE face: `(+ n 1)` appears identically in both products and is legitimately
           shareable — but the sharing must not merge or reorder the two dispatches between them:
           6·5 + 6·6 = 66. Pure value-numbering composes with effect sequencing.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* (+ n 1) (St.next)) (* (+ n 1) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 66 Int64)))

(case "a perform bound to an UNUSED binding still dispatches (DCE must not eliminate it)"
  (doc    "The DCE exclusion: `_unused`'s VALUE is dead but its dispatch is not — the bump advances
           the state and the peek observes 6. A use-count-based eliminator that removed the dead-bound
           perform would read 5. (The do-spine discard pins cover syntactic discard; this is the
           bound-but-dead face DCE actually inspects.)")
  (input  (do
            (effect St (op bump (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1)))
                 (peek (u) s (resume s s)))
                (let ((_unused (St.bump)))
                  (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "a PURE dead binding beside a perform is harmless (the eliminable control)"
  (doc    "The control for the DCE exclusion above: `_dead = n·999` is pure and genuinely
           eliminable — removing it changes nothing observable; the peek reads the untouched seed
           (5). The exclusion is about effects, not dead bindings generally.")
  (input  (do
            (effect St (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((peek (u) s (resume s s)))
                (let ((_dead (* n 999)))
                  (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))

(case "a performing helper called from TWO sites — each call site is its own dispatch"
  (doc    "The inlining exclusion: `step k = k + (St.next)` is called twice; inline duplication of
           the performing body must keep PER-SITE dispatch — 1+5 = 6 (state → 6), then 2+6 = 8 →
           608. Sharing one dispatch across the inlined sites would read 5 twice (606). (The crash
           face of this shape — the eval-once inline's binder orphans — is the en1 family, tracked
           separately; this pins the VALUES when the inline works.)")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (step (: k Int64)) (+ k (St.next)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (step 1)) (step 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 608 Int64)))

(case "a performing helper whose body BINDS the result and BRANCHES on it, called from two sites, folds"
  (doc    "The en1 fix (breaker MED). The crash face of the two-site inline above: the helper's body binds the
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
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (f (: x Int64)) (let ((r (+ x (St.bump)))) (if (>= r 100) r 0)))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (+ (f n) (f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 208 Int64)))

(case "the en1 helper called ONCE folds (the single-site control)"
  (doc    "The single-call control for the en1 fix: the SAME helper `f(x) = let r = x + St.bump in if r >= 100
           then r else 0` called ONCE always folded (one inline = no cross-site state-merge) — pins that the fix
           does not disturb it. f(5) = 5 + 100 = 105, 105 >= 100 → 105.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (f (: x Int64)) (let ((r (+ x (St.bump)))) (if (>= r 100) r 0)))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))

(case "constant conditions simplify around performs — kept branches dispatch, dropped ones do not"
  (doc    "Constant folding × effects: `(if true (St.next) 999)` and `(if false 999 (St.next))` both
           have compile-time-constant conditions — the simplification keeps each surviving branch's
           dispatch, in order: 5 + 6 = 11. (Dropped branches here carry no performs; the
           dropped-branch-with-perform elisions are the short-circuit and if-gate pins above.)")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (if true (St.next) 999) (if false 999 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))

(case "a String op RESULT selected by the op argument, composed via concat"
  (doc    "String-valued op results beyond the interner pins: the arm selects between literals by the
           op argument (positive → \\\"hi\\\", zero → \\\"lo\\\"), two dispatches compose through a concat
           chain, and the byte-length consumes the assembled \\\"hi-lo\\\" → 5. The message-building
           idiom.")
  (input  (do
            (effect St (op word (-> Int64 String)))
            (def (main (: n Int64))
              (handle St 0
                ((word (k) s (resume (if (> k 0) "hi" "lo") (+ s 1))))
                (String.byte-len (String.concat (St.word n) (String.concat "-" (St.word 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))

(case "a heterogeneous TUPLE as op ARGUMENT — the arm destructures both components"
  (doc    "The argument-direction twin of the heterogeneous-tuple RESULT pin: `(Tuple String Int64)`
           in the op signature's argument position, destructured by the arm — byte-len \\\"abc\\\" +
           10·5 = 53. Mixed-type tuples now carry witnesses in both marshal directions, like records
           and user sums.")
  (input  (do
            (effect St (op score (-> (Tuple String Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((score (p) s (match p ((tuple name pts) (resume (+ (String.byte-len name) (* pts 10)) s)))))
                (St.score (tuple "abc" n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 53 Int64)))

(case "one perform result flows through let, record, projection, tuple, destructure, and match"
  (doc    "The deep-composition smoke: a single effect-derived value travels the full consumer
           gauntlet — bound (v = 5), stored in a record field, projected twice, packed into a tuple
           with a derived companion (5, 15), destructured, compared (15 > 10), summed → 20. Each
           consumer kind is individually pinned; this chains them all on one dispatch's result to
           catch composition seams between the verified paths.")
  (input  (do
            (effect St (op seed (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((seed (u) s (resume s (+ s 1))))
                (let ((v (St.seed)))
                  (let ((r (record (base v) (scale 3))))
                    (let ((p (tuple (. r base) (* (. r base) (. r scale)))))
                      (match p
                        ((tuple lo hi)
                          (match (> hi 10)
                            (true (+ lo hi))
                            (false 0)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20 Int64)))

(case "a pure HELPER's arguments evaluate left-to-right when each performs"
  (doc    "The calling-convention face of dispatch order: a pure place-value function called with
           THREE performing arguments — they evaluate strictly left-to-right (5, 6, 7 → 567). The
           positional pins cover effect-OP operands; this pins a plain function call's argument
           evaluation order where each argument's dispatch makes the order observable.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (place (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (place (St.next) (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64)))

(case "a SET op RESULT crosses resume — membership-probed and measured per dispatch"
  (doc    "The Set completion of the collection RESULT-direction crossings (Map, List, and Bytes op
           results carry pins; Set appeared only as handler STATE): the arm resumes a per-dispatch
           set — populated for a positive op argument, empty otherwise. The body membership-probes
           the populated one (contains 5 → 10) and measures the empty one (len 0) → 10. A CHAMP set
           marshaled out of the arm must support both query kinds on the resume side.")
  (input  (do
            (effect St (op allowed (-> Int64 (Set Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((allowed (k) s (resume (if (> k 0) (Set.of (list 2 5 9)) (Set.of (list))) s)))
                (+ (if (Set.contains (St.allowed n) 5) 10 0)
                   (Set.len (St.allowed 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))

(case "a SET as op ARGUMENT — the arm measures and probes the set it is handed"
  (doc    "The argument-direction twin: a body-constructed `(Set.of (list n 2 9))` rides the op
           argument INTO the arm, which measures it (len 3) and membership-probes it (contains 5 →
           100) → 103. With this pair the collection crossing matrix — Map, List, Bytes, Set — has
           witnesses in both marshal directions.")
  (input  (do
            (effect St (op tally (-> (Set Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((tally (xs) s (resume (+ (Set.len xs) (if (Set.contains xs 5) 100 0)) s)))
                (St.tally (Set.of (list n 2 9)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 103 Int64)))

(case "a LIST OF SETS op result — the body indexes, measures, and probes the nested elements"
  (doc    "NESTED collection crossings: every flat collection has both-direction witnesses; a
           collection INSIDE a collection riding the boundary (two heap layers, RRB list over CHAMP
           sets) had none. The arm resumes `(list (Set.of (list 1 2)) (Set.of (list 3 4 n)))`; the
           body indexes both elements, measuring one (len 2) and membership-probing the other
           (contains 5 → 100) → 102. Both layers must survive the resume marshal intact.")
  (input  (do
            (effect St (op groups (-> Unit (List (Set Int64)))))
            (def (main (: n Int64))
              (handle St 0
                ((groups (u) s (resume (list (Set.of (list 1 2)) (Set.of (list 3 4 n))) s)))
                (let ((r (St.groups)))
                  (+ (match (List.at r 0) ((Some a) (Set.len a)) ((None _u) -1))
                     (match (List.at r 1) ((Some b) (if (Set.contains b 5) 100 0)) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64)))

(case "a LIST OF SETS as op ARGUMENT — the arm indexes into the nested payload it is handed"
  (doc    "The argument-direction twin of the nested-result pin: a body-built list of sets rides the
           op argument INTO the arm, which indexes both elements — 10·2 + 100 (contains 5) + 1 →
           121. The arm-side unbox of a two-layer payload.")
  (input  (do
            (effect St (op weigh (-> (List (Set Int64)) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((weigh (xs) s
                  (resume (+ (match (List.at xs 0) ((Some a) (+ (* 10 (Set.len a)) (if (Set.contains a 5) 100 0))) ((None _u) -1))
                             (match (List.at xs 1) ((Some b) (Set.len b)) ((None _u) -1)))
                          s)))
                (St.weigh (list (Set.of (list n 2)) (Set.of (list 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 121 Int64)))

(case "a MAP OF LISTS op result — the body looks up a key and folds the inner list"
  (doc    "The keyed face of nested crossings: a `(Map String (List Int64))` op result — the body
           looks up both keys and reads through the inner lists (len 3 + element 5 + element 40 →
           48). CHAMP-over-RRB, the inverse layering of the list-of-sets pins.")
  (input  (do
            (effect St (op index (-> Unit (Map String (List Int64)))))
            (def (main (: n Int64))
              (handle St 0
                ((index (u) s (resume (Map.insert (Map.insert Map.empty "a" (list 1 2 n)) "b" (list 40)) s)))
                (let ((m (St.index)))
                  (+ (match (Map.lookup m "a")
                       ((Some xs) (+ (List.len xs) (match (List.at xs 2) ((Some v) v) ((None _u) -1))))
                       ((None _u) -100))
                     (match (Map.lookup m "b")
                       ((Some ys) (match (List.at ys 0) ((Some w) w) ((None _u) -1)))
                       ((None _u) -100))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 48 Int64)))

(case "a record with a LIST field crosses resume — the body projects and folds the collection field"
  (doc    "Record crossings carry all-scalar pins both ways plus a rope-String field on the argument
           side; a COLLECTION-typed field (CHAMP/RRB nested inside the record box) was unpinned in
           either direction. The arm resumes `(record (total 50) (items (list 5 6 7)))`; the body
           projects the scalar and folds the list field — 50 + 3 + 7 → 60.")
  (input  (do
            (effect St (op page (-> Int64 (Record (total Int64) (items (List Int64))))))
            (def (main (: n Int64))
              (handle St 0
                ((page (k) s (resume (record (total (* k 10)) (items (list k (+ k 1) (+ k 2)))) s)))
                (let ((r (St.page n)))
                  (+ (. r total)
                     (+ (List.len (. r items))
                        (match (List.at (. r items) 2) ((Some v) v) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64)))

(case "a record with a SET field as op ARGUMENT — the arm probes the collection beside the scalar"
  (doc    "The argument-direction twin: the body hands `(record (want n) (seen (Set.of …)))` to the
           op and the ARM uses one field to query the other — contains(seen, want) → 100, plus len 3
           → 103. The collection field must arrive beside the scalar with both intact.")
  (input  (do
            (effect St (op audit (-> (Record (want Int64) (seen (Set Int64))) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((audit (r) s (resume (+ (* 100 (if (Set.contains (. r seen) (. r want)) 1 0))
                                         (Set.len (. r seen)))
                              s)))
                (St.audit (record (want n) (seen (Set.of (list 2 n 9)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 103 Int64)))

(case "a 40-element list op RESULT crosses resume — a multi-leaf RRB payload survives the marshal"
  (doc    "The SIZE axis of collection crossings: the existing crossing pins carry small literal
           collections (single-leaf structures); a 40-element recursively-built list exercises the
           multi-leaf RRB spine through the resume marshal — len 40 and a deep index (element 36 at
           index 35) → 4036. Structure sharing across the boundary must survive past the
           single-node fast path.")
  (input  (do
            (effect St (op range (-> Int64 (List Int64))))
            (def (build (: i Int64) (: k Int64) (: acc (List Int64)))
              (if (> i k) acc (build (+ i 1) k (List.push acc i))))
            (def (main (: n Int64))
              (handle St 0
                ((range (k) s (resume (build 1 k (list)) s)))
                (let ((xs (St.range (* n 8))))
                  (+ (* 100 (List.len xs))
                     (match (List.at xs 35) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4036 Int64)))

(case "a 40-element list as op ARGUMENT — the arm folds a multi-leaf RRB payload"
  (doc    "The argument-direction twin of the multi-leaf crossing: a 40-element body-built list
           rides INTO the arm, which runs a full indexed fold over it — sum 1..40 → 820. The
           arm-side traversal of a spine that crossed the perform.")
  (input  (do
            (effect St (op total (-> (List Int64) Int64)))
            (def (build (: i Int64) (: k Int64) (: acc (List Int64)))
              (if (> i k) acc (build (+ i 1) k (List.push acc i))))
            (def (sum-l (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-l xs (+ i 1) (+ acc v)))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St 0
                ((total (xs) s (resume (sum-l xs 0 0) s)))
                (St.total (build 1 (* n 8) (list)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 820 Int64)))

(case "a 40-element SET op result — a multi-node CHAMP payload crosses resume"
  (doc    "The CHAMP sibling of the multi-leaf RRB pins: a 40-element recursively-built set (spaced
           keys ×3 force node splits) crosses resume, then len + a positive and a negative
           membership probe — 4000 + 10 (60 ∈) + 0 (61 ∉) → 4010. The multi-node trie must arrive
           intact, not just its root.")
  (input  (do
            (effect St (op universe (-> Int64 (Set Int64))))
            (def (fill (: i Int64) (: k Int64) (: acc (Set Int64)))
              (if (> i k) acc (fill (+ i 1) k (Set.insert acc (* i 3)))))
            (def (main (: n Int64))
              (handle St 0
                ((universe (k) s (resume (fill 1 k (Set.of (list))) s)))
                (let ((xs (St.universe (* n 8))))
                  (+ (* 100 (Set.len xs))
                     (+ (if (Set.contains xs 60) 10 0)
                        (if (Set.contains xs 61) 1 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4010 Int64)))

(case "a LIST OF STRINGS op result — the body indexes and measures rope elements after the marshal"
  (doc    "The ELEMENT-type axis of collection crossings (the crossing pins carry scalar elements):
           a `(List String)` op result mixing a rope-built element, a branch-selected one, and a
           literal — the body indexes elements 0 and 1 and byte-measures them after the marshal:
           100·(List.len 3) + 10·(byte-len \"alpha\" 5) + (byte-len \"beta\" 4) → 354. Heap-boxed
           elements inside a crossing list payload.")
  (input  (do
            (effect St (op names (-> Int64 (List String))))
            (def (main (: n Int64))
              (handle St 0
                ((names (k) s (resume (list (String.concat "al" "pha") (if (> k 0) "beta" "x") "gamma") s)))
                (let ((xs (St.names n)))
                  (+ (* 100 (List.len xs))
                     (+ (* 10 (match (List.at xs 0) ((Some a) (String.byte-len a)) ((None _u) -1)))
                        (match (List.at xs 1) ((Some b) (String.byte-len b)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 354 Int64)))

(case "a LIST OF BIGINTS as op ARGUMENT — the arm folds heap-numeric elements it is handed"
  (doc    "The argument-direction heap-element face: a body-built `(List BigInt)` rides INTO the arm,
           which runs an indexed fold accumulating a BigInt — 5 + 100 + 3000 → 3105, narrowed once
           through checked Int64.of. Heap-numeric boxes must survive inside the crossing payload.")
  (input  (do
            (effect St (op total (-> (List BigInt) Int64)))
            (def (sum-b (: xs (List BigInt)) (: i Int64) (: acc BigInt))
              (match (List.at xs i)
                ((Some v) (sum-b xs (+ i 1) (+ acc v)))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St 0
                ((total (xs) s (resume (Int64.of (sum-b xs 0 (BigInt.of 0))) s)))
                (St.total (list (BigInt.of n) (BigInt.of 100) (BigInt.of 3000)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3105 Int64)))

(case "a list of RATIONALS op result — exact fractions cross resume and fold to a canonical sum"
  (doc    "The exact-arithmetic element face: `(list 1/2 1/3 1/30)` crosses resume and the body folds
           it — the sum must arrive gcd-canonical (13/15, not an unreduced spelling) for the num/den
           digit encode to read 10·13 + 15 → 145. Rational normalization must survive both the
           marshal and the fold.")
  (input  (do
            (effect St (op parts (-> Int64 (List Rational))))
            (def (sum-r (: xs (List Rational)) (: i Int64) (: acc Rational))
              (match (List.at xs i)
                ((Some v) (sum-r xs (+ i 1) (+ acc v)))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St 0
                ((parts (k) s (resume (list (Rational.of 1 2) (Rational.of 1 3) (Rational.of 1 (* k 6))) s)))
                (let ((r (sum-r (St.parts n) 0 (Rational.of 0 1))))
                  (+ (* 10 (Int64.of (Rational.numerator r)))
                     (Int64.of (Rational.denominator r))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 145 Int64)))

(case "a list-to-list TRANSFORMER op — heap payloads cross BOTH slots of one dispatch"
  (doc    "Every crossing pin carries heap in ONE slot per dispatch (scalar the other way); a
           transformer signature `(-> (List Int64) (List Int64))` moves heap BOTH directions through
           the same perform — the arm extends the very list it received (push len·10, push n) and
           resumes it; the body reads len 4 and both appended elements → 6005.")
  (input  (do
            (effect St (op grow (-> (List Int64) (List Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((grow (xs) s (resume (List.push (List.push xs (* (List.len xs) 10)) n) s)))
                (let ((out (St.grow (list 7 8))))
                  (+ (* 1000 (List.len out))
                     (+ (* 100 (match (List.at out 2) ((Some a) a) ((None _u) -1)))
                        (match (List.at out 3) ((Some b) b) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6005 Int64)))

(case "a map-to-map transformer op CHAINED — the second dispatch receives the first's result"
  (doc    "The re-crossing composition: a `(Map String Int64) → (Map String Int64)` transformer
           called on its OWN result — a heap value that already crossed the boundary once crosses
           again as the next dispatch's argument. State-keyed inserts (first at s=0, second at s=1)
           make the two dispatches distinguishable: {seed, first:5, second:6} → 356.")
  (input  (do
            (effect St (op stamp (-> (Map String Int64) (Map String Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((stamp (m) s (resume (Map.insert m (if (= s 0) "first" "second") (+ s n)) (+ s 1))))
                (let ((m2 (St.stamp (St.stamp (Map.insert Map.empty "seed" 1)))))
                  (+ (* 100 (Map.len m2))
                     (+ (* 10 (match (Map.lookup m2 "first") ((Some a) a) ((None _u) -1)))
                        (match (Map.lookup m2 "second") ((Some b) b) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 356 Int64)))

(case "a TUPLE-keyed Map op result — the body looks up by a reconstructed compound key"
  (doc    "Compound STRUCTURAL keys across the boundary (tuple-keyed collections exist only in pure
           pins): the arm resumes a `(Map (Tuple Int64 Int64) Int64)`; the body reconstructs compound
           keys to look up — `(tuple 1 2)` hits (50), the order-flipped `(tuple 4 3)` misses (-1),
           len 2 → 249. Structural key equality must survive the marshal.")
  (input  (do
            (effect St (op grid (-> Int64 (Map (Tuple Int64 Int64) Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((grid (k) s (resume (Map.insert (Map.insert Map.empty (tuple 1 2) (* k 10)) (tuple 3 4) 7) s)))
                (let ((m (St.grid n)))
                  (+ (* 100 (Map.len m))
                     (+ (match (Map.lookup m (tuple 1 2)) ((Some a) a) ((None _u) -1))
                        (match (Map.lookup m (tuple 4 3)) ((Some b) b) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 249 Int64)))

(case "a SET of tuples as op ARGUMENT — the arm probes compound membership including order sensitivity"
  (doc    "The argument-direction compound-key face: a body-built `(Set (Tuple Int64 Int64))` rides
           into the arm, which probes `(tuple 1 n)` (hit, 100) and the order-flipped `(tuple n 1)`
           (miss, 0) plus len 2 → 102. Tuple component ORDER must survive as part of the key's
           identity through the crossing.")
  (input  (do
            (effect St (op check (-> (Set (Tuple Int64 Int64)) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((check (xs) s
                  (resume (+ (* 100 (if (Set.contains xs (tuple 1 n)) 1 0))
                             (+ (* 10 (if (Set.contains xs (tuple n 1)) 1 0))
                                (Set.len xs)))
                          s)))
                (St.check (Set.of (list (tuple 1 n) (tuple 2 8))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64)))

(case "a two-op composition where the second op's String argument is BUILT from the first's result"
  (doc    "An effect-derived key crossing back in: op-1 returns a String (branch-selected \\\"hot\\\"),
           the body concat-extends it, and op-2 receives the assembled \\\"hot-path\\\" as its
           argument — byte-len 8 + 10·(state 1) → 18. A dispatch's result feeding the next
           dispatch's compound-built argument.")
  (input  (do
            (effect St (op tag (-> Int64 String)) (op fetch (-> String Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((tag (k) s (resume (if (> k 0) "hot" "cold") (+ s 1)))
                 (fetch (name) s (resume (+ (String.byte-len name) (* s 10)) (+ s 1))))
                (St.fetch (String.concat (St.tag n) "-path"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 18 Int64)))

(case "a SYMBOL as op ARGUMENT — the arm compares interned identity against its own intern"
  (doc    "Symbol's ARGUMENT direction (the interner/gensym pins cover only `-> String Symbol`
           results): a rope-built `(Symbol.of (String.concat …))` and a flat intern each cross as op
           arguments; the arm interns its own comparators — content equality must hold across the
           boundary (100 for alpha, 10 for beta → 110).")
  (input  (do
            (effect St (op classify (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((classify (sym) s
                  (resume (+ (* 100 (if (= sym (Symbol.of "alpha")) 1 0))
                             (* 10 (if (= sym (Symbol.of "beta")) 1 0)))
                          s)))
                (+ (St.classify (Symbol.of (String.concat "al" "pha")))
                   (St.classify (Symbol.of "beta")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64)))

(case "a SYMBOL handler STATE threads dispatches — each resume reads the prior symbol's identity"
  (doc    "Symbol's STATE slot (completing its three effect positions, like records and sums): the
           state starts as the `start` symbol; each dispatch compares the PRIOR symbol's identity
           (10 for start, 20 otherwise) and swaps in the next — 100·10 + 20 → 1020.")
  (input  (do
            (effect St (op swap (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "start")
                ((swap (next) prev (resume (if (= prev (Symbol.of "start")) 10 20) next)))
                (+ (* 100 (St.swap (Symbol.of "mid")))
                   (St.swap (Symbol.of "end")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1020 Int64)))

(case "a fallible helper with a `?` called from INSIDE a handler ARM (success path)"
  (doc    "The 23-try corpus pins `?` composition from the handle-BODY side; here the fallible
           helper runs inside the ARM — the `?` desugar's abortive Core::Block boundary nests while
           the dispatch machinery is live mid-arm. Two dispatches: bump(5)=105 then bump(6)=106 →
           10·105 + 106 = 1156. The two abortive machineries must not confuse their exit paths in
           arm position.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (bump (: v Int64))
              (let ((x (try (Some v))))
                (Some (+ x 100))))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume (match (bump s) ((Some v) v) ((None _u) -1)) (+ s 1))))
                (+ (* 10 (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1156 Int64)))

(case "a CONSTANT-failure `?` inside the arm's helper — the cut stays in the helper, dispatch unharmed"
  (doc    "The failure face of the arm-side `?`: the helper's `(try (None unit))` short-circuits the
           HELPER (returning None), not the arm or the dispatch — both dispatches observe the -1
           fallback and the state advance is unharmed → 10·(-1) + (-1) = -11. (A runtime-disc `?`
           here hits the BRICK-3b constant-operand boundary, pinned in 23-try.)")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (probe (: v Int64))
              (let ((x (try (None unit))))
                (Some (+ x v))))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume (match (probe s) ((Some v) v) ((None _u) -1)) (+ s 7))))
                (+ (* 10 (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -11 Int64)))

(case "a FLOAT64 as op ARGUMENT — the arm accumulates fractional values into Float64 state"
  (doc    "Float64's ARGUMENT direction (result + state are pinned): fractional literals cross as op
           arguments and accumulate into Float64 state across two dispatches — a = 1.25+0.5 = 1.75,
           b = 0.25+1.75 = 2.0 → 3.75, read back as a Float64 (Int64.of over a runtime Float64
           rejects by design, per the numeric model).")
  (input  (do
            (effect St (op weigh (-> Float64 Float64)))
            (def (main (: n Int64))
              (handle St 0.5
                ((weigh (x) s (resume (+ x s) (+ s x))))
                (let ((a (St.weigh 1.25)))
                  (let ((b (St.weigh 0.25)))
                    (+ a b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3.75 Float64)))

(case "a TUPLE mixing Float64 and Int64 crosses as op ARGUMENT — the arm scales by the int"
  (doc    "The mixed-width marshal box: an f64 and an i64 in ONE tuple op argument, destructured by
           the arm and combined via Float64.of-int — 2.5 · 10 → 25.0. The two lanes must not
           corrupt each other through the crossing.")
  (input  (do
            (effect St (op scale (-> (Tuple Float64 Int64) Float64)))
            (def (main (: n Int64))
              (handle St 0.0
                ((scale (p) s (match p ((tuple f k) (resume (* f (Float64.of-int k)) s)))))
                (St.scale (tuple 2.5 (* n 2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25.0 Float64)))

(case "the ARM decodes a Bytes op argument with a bin pattern and resumes a parsed field"
  (doc    "The arm as the DECODE site (the bin×effects pins put the codec in the body): a body-built
           frame crosses the op argument and the ARM runs the `(bin (u8 tag) (u16 val))` match —
           1000·7 + 500 → 7500. Binary parsing composes with dispatch in arm position.")
  (input  (do
            (effect Codec (op parse (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((parse (frame) s
                  (match frame
                    ((bin (u8 tag) (u16 val))
                      (resume (+ (* 1000 tag) val) s))
                    (_other (resume -1 s)))))
                (Codec.parse (bin (u8 (UInt8.wrap 7)) (u16 (UInt16.wrap (* n 100)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7500 Int64)))

(case "the ARM ENCODES its scalar op argument into framed Bytes and resumes them — body decodes"
  (doc    "The inverse arm-codec direction: the arm bin-ENCODES its scalar argument into a framed
           payload, resumes it, and the BODY decodes — 1000·9 + 150 → 9150. Round-trip with the
           encode inside the arm and the decode outside.")
  (input  (do
            (effect Codec (op frame (-> Int64 Bytes)))
            (def (main (: n Int64))
              (handle Codec 0
                ((frame (v) s (resume (bin (u8 (UInt8.wrap 9)) (u16 (UInt16.wrap (* v 3)))) s)))
                (match (Codec.frame (* n 10))
                  ((bin (u8 tag) (u16 val)) (+ (* 1000 tag) val))
                  (_other -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9150 Int64)))

(case "a Bytes-to-Bytes transformer op — the arm frames the payload it received and the body re-reads"
  (doc    "The byte-rope transformer face (hb pins cover List/Map transformers): the arm
           length-prefixes the frame it received via `Bytes.concat` of a fresh bin over the crossed
           payload — a NON-FLAT byte-rope result — and the body re-reads prefix + first payload byte:
           10000·3 + 100·2 + 40 → 30240.")
  (input  (do
            (effect Codec (op wrap (-> Bytes Bytes)))
            (def (main (: n Int64))
              (handle Codec 0
                ((wrap (b) s (resume (Bytes.concat (bin (u8 (UInt8.wrap (Bytes.len b)))) b) s)))
                (let ((out (Codec.wrap (bin (u8 (UInt8.wrap (* n 8))) (u8 (UInt8.wrap 3))))))
                  (+ (* 10000 (Bytes.len out))
                     (+ (* 100 (match (Bytes.at out 0) ((Some h) h) ((None _u) -1)))
                        (match (Bytes.at out 1) ((Some p) p) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30240 Int64)))

(case "a String-to-String transformer op — a rope argument crosses in, a wrapped rope crosses back"
  (doc    "The text transformer face: a concat-built rope ARGUMENT crosses in, the arm wraps it in
           brackets via nested concats (another rope), and the result crosses back — byte-len
           \\\"[abcde]\\\" → 7. Rope structure survives both marshal directions of one dispatch.")
  (input  (do
            (effect Fmt (op brack (-> String String)))
            (def (main (: n Int64))
              (handle Fmt 0
                ((brack (t) s (resume (String.concat "[" (String.concat t "]")) s)))
                (String.byte-len (Fmt.brack (String.concat "ab" (if (> n 0) "cde" "z"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))

(case "an abortive arm READS the heap LIST op argument it was handed — the payload survives the abort"
  (doc    "The abort×heap pins cover arm-BUILT lists and heap STATE reads; an abortive arm CONSUMING
           its heap op-argument payload was unpinned. The crossed list must stay live on the abort
           path — 100·3 + 42 → 342, plus the outer 1000; the discarded continuation's 999 never
           adds → 1342.")
  (input  (do
            (effect Bail (op stop (-> (List Int64) Int64)))
            (def (main (: n Int64))
              (+ 1000
                 (handle Bail 0
                   ((stop (xs) s (+ (* 100 (List.len xs))
                                    (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))
                   (+ 999 (Bail.stop (list n 42 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1342 Int64)))

(case "an abortive arm returns a MAP built FROM its heap op argument as the handle's value"
  (doc    "Heap-in via the op argument AND heap-out via the abort branch, one arm: the abortive arm
           folds its list argument into a fresh Map that becomes the handle's value — {sum: 35},
           10·1 + 35 → 45. Both heap directions on the abort path.")
  (input  (do
            (effect Bail (op stop (-> (List Int64) (Map String Int64))))
            (def (main (: n Int64))
              (let ((m (handle Bail 0
                         ((stop (xs) s (Map.insert Map.empty "sum"
                                          (+ (match (List.at xs 0) ((Some a) a) ((None _u) 0))
                                             (match (List.at xs 1) ((Some b) b) ((None _u) 0))))))
                         (do (Bail.stop (list n 30)) Map.empty))))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m "sum") ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 45 Int64)))

(case "200 recursive dispatches each crossing a heap LIST argument — the marshal at depth"
  (doc    "The depth axis of heap-argument crossings (existing depth pins carry scalars): 200
           iterations each build a fresh two-element list, cross it, and the arm folds it — the
           per-dispatch marshal alloc/free churn must stay exact: Σ(i + 1) for i in 1..200 →
           20300.")
  (input  (do
            (effect St (op scan (-> (List Int64) Int64)))
            (def (loop (: i Int64) (: acc Int64))
              (if (> i 200) acc
                (loop (+ i 1) (+ acc (St.scan (list i 1))))))
            (def (main (: n Int64))
              (handle St 0
                ((scan (xs) s
                  (resume (+ (match (List.at xs 0) ((Some a) a) ((None _u) 0))
                             (match (List.at xs 1) ((Some b) b) ((None _u) 0)))
                          s)))
                (loop 1 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20300 Int64)))

(case "a handler state GROWS a list across 100 dispatches — the accumulated spine reads back intact"
  (doc    "The growing-spine RC discipline across suspensions: each dispatch pushes onto the list
           state and resumes the length BEFORE its push, so the checksum verifies every intermediate
           spine — 100·Σ(0..99) → 495000, not just the final length.")
  (input  (do
            (effect Log (op note (-> Int64 Int64)))
            (def (loop (: i Int64) (: acc Int64))
              (if (> i 100) acc
                (loop (+ i 1) (+ acc (Log.note i)))))
            (def (main (: n Int64))
              (handle Log (list)
                ((note (v) s (resume (List.len s) (List.push s v))))
                (+ (* 100 (loop 1 0))
                   0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 495000 Int64)))

(case "a Bytes.slice VIEW crosses as op ARGUMENT — the arm reads through the window it was handed"
  (doc    "A body-built slice VIEW (not a copy) crossing INTO a dispatch (the existing view pins put
           the slice in the resume value or slice the arm's own param): the arm reads len + both
           bytes through the window — 100·2 + 20 + 30 → 250. The view's backing buffer must stay
           live through the marshal.")
  (input  (do
            (effect St (op sum2 (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sum2 (w) s (resume (+ (* 100 (Bytes.len w))
                                        (+ (match (Bytes.at w 0) ((Some a) a) ((None _u) -1))
                                           (match (Bytes.at w 1) ((Some b) b) ((None _u) -1))))
                             s)))
                (match (Bytes.slice (Bytes.of (list 9 20 30 8)) 1 2)
                  ((Some w) (St.sum2 w))
                  ((None _u) -999))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 250 Int64)))

(case "a String.slice VIEW built in the ARM crosses back through resume — the body measures it"
  (doc    "The arm-built STRING view crossing OUT: the arm slices the rope argument it received
           (start 1, end 4 → \\\"bcd\\\") and resumes the window — byte-len 3. An arm-created view
           over a crossed payload must survive the return marshal.")
  (input  (do
            (effect St (op mid (-> String String)))
            (def (main (: n Int64))
              (handle St 0
                ((mid (t) s (resume (match (String.slice t 1 4) ((Some w) w) ((None _u) "?")) s)))
                (String.byte-len (St.mid (String.concat "ab" "cdef")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64)))

(case "an IN-PROGRAM arm resumes a Qty built in the arm — the erased-scalar crossing without a host"
  (doc    "The pure in-program Qty handler (the existing Qty effect pins are host-delegated): the arm
           builds `(Qty.of (* k 2) meter)` and resumes it; two dispatches sum under the unit type and
           `Qty.value` reads 30. The compile-time-erased unit must type the arm/body agreement with
           no host boundary involved.")
  (input  (do
            (effect Env (op width (-> Int64 (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Env 0
                ((width (k) s (resume (Qty.of (* k 2) (Unit.base #"meter")) s)))
                (Qty.value (+ (Env.width n) (Env.width 10)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64)))

(case "a Qty STATE threads via a def-bound arm computation — the workaround shape runs end to end"
  (doc    "Qty as handler STATE with the arm computing the next state through an arm-local `def` —
           each dispatch resumes the PRIOR quantity and doubles the state: 5m + 10m → 15. (The
           sibling pin below covers the inline-slot spelling.)")
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Acc (Qty.of n (Unit.base #"meter"))
                ((step (u) s (do (def t (+ s s)) (resume s t))))
                (Qty.value (+ (Acc.step) (Acc.step)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))

(case "a Qty state's next-state slot computes (+ s s) INLINE — the formerly-rejected shape runs"
  (doc    "This exact spelling — Qty-state arithmetic INSIDE the next-state slot — used to falsely
           reject (the state binder typed at the erased Int64 in slot position; an 18-units
           provenance note documents the old behavior and its def-workaround). Fixed on trunk; this
           pins the flip: seed 5m, `(resume s (+ s s))` threads 5+10 → 15 with values verified.")
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle Acc (Qty.of n (Unit.base #"meter"))
                ((step (u) s (resume s (+ s s))))
                (Qty.value (+ (Acc.step) (Acc.step)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))

(case "a guard destructures a perform-result TUPLE and its condition reads both binders"
  (doc    "The compound-pattern face of the guarded perform-scrutinee family (the ag5 pins use
           scalar guard binders): `(guard (tuple a b) (> (+ a b) 10))` over a perform-result tuple —
           the guard-desugar's arm copy composes with the destructure; hit path 100·5 + 10 → 510.")
  (input  (do
            (effect St (op pair (-> Unit (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((pair (u) s (resume (tuple s (* s 2)) (+ s 1))))
                (match (St.pair)
                  ((guard (tuple a b) (> (+ a b) 10)) (+ (* 100 a) b))
                  ((tuple a b) (+ a b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 510 Int64)))

(case "the guard-MISS path re-performs in the fallback arm — dispatch continues past a failed guard"
  (doc    "The miss path of the compound guard: `(> (+ a b) 100)` fails at 15, the fallback arm
           RE-PERFORMS, and the second dispatch reads the advanced state — 10·15 + 18 → 168. A
           failed compound guard must leave the dispatch machinery able to serve the fallback's
           perform.")
  (input  (do
            (effect St (op pair (-> Unit (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((pair (u) s (resume (tuple s (* s 2)) (+ s 1))))
                (match (St.pair)
                  ((guard (tuple a b) (> (+ a b) 100)) (+ (* 100 a) b))
                  ((tuple a b) (match (St.pair) ((tuple c d) (+ (* 10 (+ a b)) (+ c d))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 168 Int64)))

(case "an arm re-performs its OWN effect to a SAME-EFFECT outer handler — the true self-shadow forward"
  (doc    "The existing forwarding pin uses two DISTINCT effects; here the inner handler of `Ctr`
           re-performs `Ctr` against a same-effect OUTER handler with a DIFFERENT arm shape — inner
           multiplies-and-forwards, outer adds-with-state: bump(5) → outer bump(50) → 50+100 = 150.
           The forward must reach the outer arm's semantics, not re-enter the inner's.")
  (input  (do
            (effect Ctr (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Ctr 100
                ((bump (v) t (resume (+ v t) (+ t 1))))
                (handle Ctr 0
                  ((bump (v) s (resume (Ctr.bump (* v 10)) s)))
                  (Ctr.bump n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64)))

(case "both same-effect handlers STATEFUL — the outer's advance survives the inner's forwards"
  (doc    "The stateful composition of the self-shadow forward: two inner-region dispatches each
           forward to the stateful outer (t advances 100→101→102), then a POST-region perform reads
           the accumulated t — (150 + 111) + 104 → 365. The outer state must thread across forwards
           originating in the inner arm.")
  (input  (do
            (effect Ctr (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Ctr 100
                ((bump (v) t (resume (+ v t) (+ t 1))))
                (+ (handle Ctr 0
                     ((bump (v) s (resume (Ctr.bump (* v 10)) (+ s 1))))
                     (+ (Ctr.bump n) (Ctr.bump 1)))
                   (Ctr.bump 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 365 Int64)))

(case "a CLOSURE handler state captures the enclosing function's parameter and applies per dispatch"
  (doc    "The existing closure-state pins seed with parameter-FREE closures; here the seed closure
           captures the enclosing function's `n` — `(fn (x) (* x n))` applied in the arm reads the
           capture (10·5 → 50). Single-shot dispatch with a param-capturing closure state (the
           multi-shot sibling is a known open capture-locus).")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) (* x n))
                ((next (u) f (resume (f 10) f)))
                (St.next)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))

(case "the closure state is REPLACED per dispatch by one capturing the arm's OWN binder"
  (doc    "State replacement with an arm-frame capture: each dispatch builds a FRESH closure over the
           arm's own let-binder `r` and installs it as the next state — d1: f = x+5, r = 105, next
           f = x+105; d2: r = 205 → 1000·105 + 205 = 105205. The replacement closure's environment
           must be the arm frame's, rebuilt per dispatch.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) (+ x n))
                ((next (u) f
                  (let ((r (f 100)))
                    (resume r (fn ((: x Int64)) (+ x r))))))
                (+ (* 1000 (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105205 Int64)))

(case "the natural invariant construction over a VIOLATING perform result traps through the handler"
  (doc    "The body-side invariant × effects composition (the arm-side pin lives in
           26-program-conditions): `(Percent.Pct (St.next))` where the RESUMED VALUE itself decides —
           in-range 42 constructs and unwraps; an out-of-range 200 violates `[0,100]` and traps at
           the establish-divert THROUGH the handler's resume path.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (unwrap (Percent.Pct (St.next)))))
            (export main)))
  (call   main (: 42 Int64))
  (output (: 42 Int64))
  (call   main (: 200 Int64))
  (trap   "unreachable"))

(case "the arm DECODES a Bytes op argument to a String — multibyte UTF-8 survives the crossing"
  (doc    "String.from-bytes validation in ARM position over a crossed payload (the validation pins
           are body-side): \\\"héllo\\\" (6 bytes, one 2-byte scalar) crosses as the op argument and
           the arm's decode validates it → byte-len 6.")
  (input  (do
            (effect Codec (op read (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((read (b) s
                  (resume (match (String.from-bytes b)
                            ((Some t) (String.byte-len t))
                            ((None _u) -1))
                          s)))
                (Codec.read (String.to-bytes "héllo"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "INVALID UTF-8 crosses as a Bytes op argument — the arm's decode declines with None"
  (doc    "The invalid-bytes face: `0xFF 0xFE` crosses the boundary and the arm's
           `String.from-bytes` must actually validate the crossed payload (not trust it) —
           None → -1.")
  (input  (do
            (effect Codec (op read (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((read (b) s
                  (resume (match (String.from-bytes b)
                            ((Some t) (String.byte-len t))
                            ((None _u) -1))
                          s)))
                (Codec.read (bin (u8 (UInt8.wrap 255)) (u8 (UInt8.wrap 254))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -1 Int64)))

(case "TWO sequential handles of the same effect — the second starts fresh, no state bleed"
  (doc    "Handler LIFECYCLE isolation (the existing pins nest but never SEQUENCE): one helper
           instantiates the same handler twice in sequence — run(5) = 5+6 = 11, then run(10) =
           10+11 = 21, each from its OWN seed → 100·11 + 21 = 1121. No state bleeds between
           instantiations.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (run (: seed Int64))
              (handle St seed
                ((next (u) s (resume s (+ s 1))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (+ (* 100 (run n)) (run 10)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1121 Int64)))

(case "an ABORT in the first handle leaves the SECOND handle's dispatch untouched"
  (doc    "Post-abort isolation: the first handle aborts (5·2 = 10, its 999 continuation dropped);
           a SECOND, separate handle then dispatches normally (7+8 = 15) → 10·10 + 15 = 115. The
           abort's unwind must not corrupt a sibling handler's dispatch or state.")
  (input  (do
            (effect Bail (op stop (-> Int64 Int64)))
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (+ (* 10 (handle Bail 0
                         ((stop (v) s (* v 2)))
                         (+ 999 (Bail.stop n))))
                 (handle St 7
                   ((next (u) s (resume s (+ s 1))))
                   (+ (St.next) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64)))

(case "a bare BIGINT as op ARGUMENT — the arm does exact wide arithmetic on the crossed box"
  (doc    "BigInt's ARGUMENT direction (results/state/list-elements are pinned): the arm multiplies
           the crossed box by 10^6 and integer-divides by 999999999 — exact wide arithmetic on a
           value that crossed the boundary → 1000, narrowed once through checked Int64.of.")
  (input  (do
            (effect St (op grow (-> BigInt Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((grow (b) s (resume (Int64.of (/ (* b (BigInt.of 1000000)) (BigInt.of 999999999))) s)))
                (St.grow (BigInt.of (* n 200000)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1000 Int64)))

(case "a bare RATIONAL as op ARGUMENT — the arm reads exact numerator/denominator off the crossed value"
  (doc    "Rational's ARGUMENT direction: 1/3 crosses, the arm adds 1/6 and reads num/den off the
           gcd-canonical sum (1/2) → 10·1 + 2 = 12. Exact-fraction identity must survive the
           marshal into the arm.")
  (input  (do
            (effect St (op mix (-> Rational Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((mix (r) s
                  (let ((q (+ r (Rational.of 1 6))))
                    (resume (+ (* 10 (Int64.of (Rational.numerator q)))
                               (Int64.of (Rational.denominator q)))
                            s))))
                (St.mix (Rational.of 1 (- n 2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64)))

; ============ Narrow-width effect-op literals (breaker FINDING nw, operator-confirmed soundness →
; fixed on trunk). The effect-op signature positions (argument AND result-via-resume) skipped the
; CDZ0302 literal fit-check every sibling position enforces — an out-of-range literal observably
; inhabited the narrow type, including across the HOST boundary in a declared-width slot. The fix
; range-checks both marshal directions (and descends compounds: tuple/record/list). These pin the
; served class: the in-range pass, the bare-arg + resume-result + record-field rejects, and the
; runtime-argument control (a TYPE mismatch, not a width fault). ============

(case "an in-range literal to a narrow effect-op parameter crosses and the arm observes it"
  (doc    "The pass face of the narrow-op range check: `(Send.put 42)` against `(-> UInt8 Int64)`
           fits, crosses, and the arm reads 42 back via checked Int64.of.")
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume (Int64.of v) s)))
                (Send.put 42)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))

(case "an OVERFLOWING literal to a narrow effect-op parameter is rejected"
  (doc    "The argument-direction reject: `(Send.put 999)` against a UInt8 parameter (0..=255) is
           CDZ0302 — the same fit-check plain-fn params and annotated literals enforce. Before the
           fix this compiled and the arm observed 999.")
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume (Int64.of v) s)))
                (Send.put 999)))
            (export main)))
  (error  CDZ0302))

(case "an arm resuming an OVERFLOWING literal into a narrow op RESULT is rejected"
  (doc    "The result-direction reject: the op's declared result is UInt8 and the arm resumes 999 —
           CDZ0302 at the resume site. Before the fix the body observed 999 through the narrow
           result type.")
  (input  (do
            (effect Give (op get (-> Unit UInt8)))
            (def (main (: n Int64))
              (handle Give 0
                ((get (u) s (resume 999 s)))
                (Int64.of (Give.get))))
            (export main)))
  (error  CDZ0302))

(case "an overflowing literal in a RECORD op argument's narrow field is rejected"
  (doc    "The compound-descent face: the width check must recurse into a Record op argument's
           fields — `(record (small 999) …)` against `(Record (small UInt8) …)` is CDZ0302. (Tuple
           and List elements were covered by the same descent from the start; the Record row arm
           was a fold-in.)")
  (input  (do
            (effect Send (op put (-> (Record (small UInt8) (big Int64)) Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (r) s (resume (+ (Int64.of (. r small)) (. r big)) s)))
                (Send.put (record (small 999) (big 5)))))
            (export main)))
  (error  CDZ0302))

(case "a RUNTIME Int64 argument to a narrow effect-op parameter is rejected as a type mismatch"
  (doc    "The control distinguishing the width fault from ordinary typing: a RUNTIME Int64 arg to a
           UInt8 op parameter is CDZ0301 (type mismatch — no silent narrowing), NOT CDZ0302 (which
           is literal-fit). The two rejects must not blur.")
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume 7 s)))
                (Send.put n)))
            (export main)))
  (error  CDZ0301))

(case "a FULL handle expression in the resume-value slot — the arm runs a nested handler per dispatch"
  (doc    "Arms performing INTO enclosing handlers is well-pinned; here the arm INSTANTIATES its own
           complete handler: `(resume (handle In 100 … (In.small (* v 2))) s)` — the nested handle
           runs to completion inside the arm and its result becomes the resume value (10 + 100 →
           110).")
  (input  (do
            (effect Out (op big (-> Int64 Int64)))
            (effect In (op small (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Out 0
                ((big (v) s
                  (resume (handle In 100
                            ((small (w) t (resume (+ w t) t)))
                            (In.small (* v 2)))
                          s)))
                (Out.big n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64)))

(case "the arm's nested handler is instantiated FRESH per dispatch — independent inner state"
  (doc    "The per-dispatch lifecycle of an arm-instantiated handler: each outer dispatch seeds a NEW
           inner handler from its op argument (v=5 → inner 5+6=11; v=20 → inner 20+21=41) → 100·11 +
           41 = 1141. No inner state survives between the arm's instantiations.")
  (input  (do
            (effect Out (op big (-> Int64 Int64)))
            (effect In (op small (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Out 0
                ((big (v) s
                  (resume (handle In v
                            ((small (u) t (resume t (+ t 1))))
                            (+ (In.small) (In.small)))
                          s)))
                (+ (* 100 (Out.big n)) (Out.big 20))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1141 Int64)))

; ============ Multi-shot × enclosing-param capture (breaker FINDING mv, fixed in two slices: the
; continuation's captures pinned before the per-resume splice, then the ARM BODY's captures pinned
; before beta-reduce for the resume-value face + after substitution for the seed face). The class
; was [multi-shot arm] × [any let/def binding in the handle body] × [an enclosing-param reference] →
; false CDZ0101; match-binder consumers and no-binding bodies were always immune. These pin the
; VALUE-verified faces: param in the resume value, param as the handle seed, and the always-immune
; match-binder control. ============

(case "a multi-shot arm's resume VALUE reads the enclosing param — the let-bound body folds correctly"
  (doc    "FINDING repro (mv7, fixed): `(pick (u) s (+ (resume (+ n 1) s) (resume 2 s)))` with the
           body let-binding the perform result — the resume-value's `n` is spliced into the
           continuation's hole per resume site and used to orphan. Now folds with the right VALUES:
           k(v) = 11v, so 11·6 + 11·2 = 88.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
                (let ((x (Amb.pick)))
                  (+ (* 10 x) x))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))

(case "a multi-shot handle SEEDED by the enclosing param folds with a let-bound body"
  (doc    "The seed face (mv11, fixed): `(handle Amb n …)` where the param enters the arm via the
           state binder substitution — the second capture path of the mv class. Same fold values:
           seed 5, resume (s+1)=6 then 2 → 11·6 + 11·2 = 88.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((pick (u) s (+ (resume (+ s 1) s) (resume 2 s))))
                (let ((x (Amb.pick)))
                  (+ (* 10 x) x))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))

(case "a multi-shot arm with an enclosing-param resume value and a MATCH-binder consumer folds"
  (doc    "The always-immune control of the mv class: a match BINDER consumes the perform result
           (binding without a let) — this shape never orphaned, and it must keep folding identically
           now that the let shapes are fixed: 88.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
                (match (Amb.pick) (v (+ (* 10 v) v)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))

(case "an @ensures on a PERFORMING def called TWICE under one handler — both effectful results checked"
  (doc    "The @ensures SURFACE face of the en1 class (the minimal let-if-inline shape is pinned
           above; 26-program-conditions marks the multi-call face as future): a postcondition on a
           performing def called twice under one handler — verify_enforce wraps each inline, both
           effectful results check `(>= ret 100)`: f(5)=105, f(2)=103 → 208.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (+ (f n) (f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 208 Int64)))

(case "an OPTION as op ARGUMENT — the arm matches Some/None it was handed, per dispatch"
  (doc    "Option's ARGUMENT direction (results + state are pinned): body-built `(Some n)` and
           `(None unit)` each ride into the arm, which matches — Some(5) → 50, None → -1 →
           100·50 - 1 = 4999. The std-sum tag must survive the crossing per dispatch.")
  (input  (do
            (effect St (op weigh (-> (Option Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((weigh (o) s (resume (match o ((Some v) (* v 10)) ((None _u) -1)) s)))
                (+ (* 100 (St.weigh (Some n)))
                   (St.weigh (None unit)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4999 Int64)))

(case "a RESULT as op ARGUMENT — the arm branches on Ok/Err payloads it was handed"
  (doc    "Result's ARGUMENT direction: `(Result.Ok n)` and `(Result.Err 7)` cross into the arm,
           which branches — Ok(5) → 50, Err(7) → -7 → 100·50 - 7 = 4993. Completes the std-sum
           pair's three effect positions.")
  (input  (do
            (effect St (op judge (-> (Result Int64 Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((judge (r) s (resume (match r ((Result.Ok v) (* v 10)) ((Result.Err e) (- 0 e))) s)))
                (+ (* 100 (St.judge (Result.Ok n)))
                   (St.judge (Result.Err 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4993 Int64)))

(case "a scrutinee FAILING the pattern reaches the catch-all WITHOUT running the guard's perform"
  (doc    "The pattern-MISS soundness face of the refutable performing guard (the sibling pins test
           guard-matches-then-false): a None scrutinee against `(guard (Some v) (> v (St.quota)))`
           must reach the catch-all with the guard's perform NEVER evaluated — witnessed by a
           post-match `St.quota` reading the UNADVANCED state: 100·99 + 5 = 9905. (The keep-the-match
           hoist guarantees this; an if-only rewrite would have run the guard on the miss.)")
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (+ s 1))))
                (+ (* 100 (match (None unit)
                            ((guard (Some v) (> v (St.quota))) v)
                            (_other 99)))
                   (St.quota))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9905 Int64)))

(case "a MULTI-argument op mixing a heap list and two scalars — the arm consumes all three"
  (doc    "Multi-argument op signatures are pinned scalar-only; here a `(List Int64)` crosses beside
           two scalar INDICES into it — the arm indexes the list by both and measures it:
           100·7 + 10·9 + 3 → 793. Positional integrity across a mixed heap/scalar marshal.")
  (input  (do
            (effect St (op pick (-> (List Int64) Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((pick (xs lo hi) s
                  (resume (+ (* 100 (match (List.at xs lo) ((Some a) a) ((None _u) -1)))
                             (+ (* 10 (match (List.at xs hi) ((Some b) b) ((None _u) -1)))
                                (List.len xs)))
                          s)))
                (St.pick (list 7 n 9) 0 2)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 793 Int64)))

(case "a multi-argument op with TWO heap arguments — a String key and a Map to search"
  (doc    "Two heap values in ONE op signature (the lookup-service idiom): a rope-built String key
           and a Map cross together; the arm looks the key up in the map it was handed —
           10·5 + 2 → 52. Two independent heap handles must both survive the same marshal.")
  (input  (do
            (effect St (op find (-> String (Map String Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((find (k m) s
                  (resume (+ (* 10 (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
                             (Map.len m))
                          s)))
                (St.find (String.concat "k" "1")
                         (Map.insert (Map.insert Map.empty "k1" n) "k2" 30))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 52 Int64)))

; ============ Empty-collection match-join grounding (breaker FINDING ms/ej, fixed in three parts:
; the front-end grounds an open-Var join arm to the determined-collection shell — all three
; collection kinds; the rust emit reconstructs the solved map type at Map.lookup for scalar values;
; and the rust emit annotates a collection-valued join's solved OUTER shape with holed interior
; (`Vec<_>`) rather than grounding — the nested face is WHY: the join under-approximates nested
; element types, and a ground would break exactly where a hole lets rustc solve). These pin the
; served class: the pure minimal, the Set sibling, the IF-join face, and the two-layer upsert
; idiom. The empty-MAP-fallback sibling is a known loud E0282 follow-up on the rust backends. ============

(case "an empty (list) match-fallback beside an unsolved-Var arm grounds to the join's list type"
  (doc    "FINDING repro (ms13, fixed): `(match (Map.lookup m \\\"k\\\") ((Some ys) ys) ((None _u)
           (list)))` — the Some arm binds an open Var (the empty map's value type is only fixed
           downstream) and the fallback is an empty literal; the join must ground both arms to the
           downstream-determined list type. Runs 1 (one push onto the empty fallback).")
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (list)))))
                  (let ((nxs (List.push xs n)))
                    (List.len nxs)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))

(case "an empty SET literal in a match-Option fallback grounds through the join"
  (doc    "The Set sibling of the empty-literal join class: the fallback is `(Set.of (list))` and
           the downstream `Set.insert` fixes the element type — 1. The join ground must cover all
           collection kinds, not just List.")
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (Set.of (list))))))
                  (Set.len (Set.insert xs n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))

(case "an IF-join with an unsolved-Var arm and an empty-list sibling grounds like a match join"
  (doc    "The join kind is irrelevant (a concrete-sibling IF always worked — the sibling supplied
           the evidence): an IF whose then-arm is a Map-lookup payload (open Var) and whose else is
           an empty `(list)` must ground through the same machinery as the match join — 1.")
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (if (> n 0)
                              (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (list)))
                              (list))))
                  (List.len (List.push xs n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))

(case "a MAP-OF-LISTS handler state accumulates per dispatch — the upsert idiom end to end"
  (doc    "The real-world shape that found the class: a `(Map String (List Int64))` handler state
           with the lookup-fallback-push upsert arm — key a gets 3 appends, key b one; each resume
           returns the new inner length (1,1,2,3 → 1123). The two-layer state (CHAMP over RRB) must
           path-copy across resume cycles, and the empty-fallback join must ground (nested: the join
           sees only `List Any`, the emit's interior hole lets rustc solve `Vec<Vec<i64>>`).")
  (input  (do
            (effect Db (op add (-> (Tuple String Int64) Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((add (p) m
                  (match p
                    ((tuple k v)
                      (let ((xs (match (Map.lookup m k) ((Some ys) ys) ((None _u) (list)))))
                        (let ((nxs (List.push xs v)))
                          (resume (List.len nxs) (Map.insert m k nxs))))))))
                (+ (* 1000 (Db.add (tuple "a" n)))
                   (+ (* 100 (Db.add (tuple "b" 7)))
                      (+ (* 10 (Db.add (tuple "a" 6)))
                         (Db.add (tuple "a" 9)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1123 Int64)))

; ============ Guarded match × effects (breaker FINDING, ag5 → fixed #2333). A guarded match on a
; perform-result scrutinee whose FALLBACK arm also performs used to leak the fold-synthesized #seed
; binder as a false CDZ0101: the guard desugar's arm-body copy reparented a reused (shared) body
; without the seed-lift let, stranding the reference. The fix pins reused guarded-match arm bodies
; at desugar entry (and drops the blanket forget). These four pin the served class: the repro, the
; guard-TRUE runtime path, the performing-guard-CONDITION position, and the multi-guard chain. The composed
; face — a guard FALLBACK containing a two-site multi-perform arm — is a separate machinery
; composition that declines cleanly (guard-desugar copy × two-hole refold). ============

(case "a guarded match on a perform-result scrutinee with a PERFORMING fallback arm folds"
  (doc    "FINDING repro (ag5, fixed #2333): `(match (St.roll) ((guard v (> v 6)) …) (v (+ (* 10
           (St.roll)) v)))` — the scrutinee is a perform result AND the fallback arm performs again.
           This exact conjunct leaked `#seed` as a false CDZ0101 before the fix (either alone was
           fine — the controls below). roll → 5 (state 5→8), guard 5>6 misses, fallback: 10·8 + 5 =
           85.")
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 6)) (* v 100))
                  (v (+ (* 10 (St.roll)) v)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64)))

(case "the guard-TRUE path of the perform-scrutinee match (no fallback entry)"
  (doc    "The same shape called with a guard-passing input: roll → 9 (state 9→12), 9 > 6 holds, so
           the guarded arm answers 900 and the performing fallback is never entered. With the repro
           above, pins BOTH runtime paths of the served shape — the fallback's perform must neither
           fire on this path nor confuse the fold on the other.")
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 6)) (* v 100))
                  (v v))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 900 Int64)))

(case "a guard whose CONDITION itself performs (pure scrutinee) folds"
  (doc    "The third position a perform can occupy in a guarded match: the GUARD CONDITION `(> (St.roll)
           4)` — scrutinee (`n`, pure) and arm bodies effect-free. roll → 5 (once; the guard evaluates
           only after its pattern matches), 5 > 4 holds → 5·100 = 500. Completes the position triple
           with the scrutinee-perform and fallback-perform pins above.")
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match n
                  ((guard v (> (St.roll) 4)) (* v 100))
                  (v v))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 500 Int64)))

(case "a MULTI-guard chain on a perform-result scrutinee with a performing fallback folds"
  (doc    "The chain face of the fixed class: TWO guarded arms cascade over the perform-result
           scrutinee before the performing fallback — the arm-body pinning must hold across every
           reused body in the cascade, not just one. roll → 5, 5>20 misses, 5>6 misses, fallback:
           10·8 + 5 = 85.")
  (input  (do
            (effect St (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 20)) (* v 1000))
                  ((guard v (> v 6)) (* v 100))
                  (v (+ (* 10 (St.roll)) v)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64)))

(case "a perform result LET-bound then fed to a bin segment builds Bytes under a handler"
  (doc    "bin × effects: a `bin` integer segment is a STRICT operand position (a perform INLINE in the
           segment is the not-yet-reducible strict-ctor boundary, like try operands), but the LET-BOUND
           route folds — `(let ((v (UInt8.wrap (St.next)))) (bin (u8 v)))` discharges the perform first
           and feeds the pure UInt8. Seed 5 → byte 5 read back via `Bytes.at` → 5. Pins the
           wire-protocol-under-effects authoring idiom (bind performs, then construct).")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((v (UInt8.wrap (St.next))))
                  (match (Bytes.at (bin (u8 v)) 0)
                    ((Some b) (Int64.of b))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))

(case "performs INLINE in record fields fold (records are not a strict-ctor boundary)"
  (doc    "The record-vs-bin ctor CONTRAST: unlike a bin segment (strict — inline performs decline,
           see the let-bound pin above), a record constructor's fields accept performs INLINE —
           `(record (lo (St.next)) (hi (St.next)))` folds, and the checksum doubles as a left-to-right
           field-evaluation witness: lo gets the FIRST dispatch (5), hi the second (6) → 506. Same
           shape, different constructor class, opposite result — pins the boundary's exact extent.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((r (record (lo (St.next)) (hi (St.next)))))
                  (+ (* 100 (. r lo)) (. r hi)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))

(case "let-bound perform results stored into record fields"
  (doc    "The conservative route beside the inline pin above: both performs discharge into lets first,
           then the record is built from pure bindings — same 506. Both routes fold for records; only
           bin requires the let-bound spelling.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((a (St.next)))
                  (let ((b (St.next)))
                    (let ((r (record (lo a) (hi b))))
                      (+ (* 100 (. r lo)) (. r hi)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64)))

(case "TWO closures capture one let-bound perform result — the effect fires ONCE"
  (doc    "The single-firing guarantee of a shared capture: `v = (St.pull)` fires once (reading 40),
           and BOTH closures close over the same `v` — f(1) = 41, g(2) = 80 → 121. A desugar that
           re-fired the perform per capturing closure would give g a 41 (→ 82, total 123). The
           sharing shape the host-closure machinery relies on, in-program form.")
  (input  (do
            (effect St (op pull (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 40
                ((pull (u) s (resume s (+ s 1))))
                (let ((v (St.pull)))
                  (let ((f (fn ((: x Int64)) (+ x v)))
                        (g (fn ((: x Int64)) (* x v))))
                    (+ (f 1) (g 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 121 Int64)))

(case "a closure's captured perform result survives a LATER state advance (capture-time, not re-read)"
  (doc    "The temporal face of eval-once capture: `v` captures 40, a DIFFERENT op then advances the
           state (+10), and only then does the closure fire — the captured 40 survives (41). A lazy
           capture that re-evaluated the perform (or re-read the state) at application would give 52.
           With the single-firing pin above, the capture-semantics pair.")
  (input  (do
            (effect St (op pull (-> Unit Int64)) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 40
                ((pull (u) s (resume s (+ s 1)))
                 (bump (u) s (resume s (+ s 10))))
                (let ((v (St.pull)))
                  (let ((f (fn ((: x Int64)) (+ x v))))
                    (do (St.bump)
                        (f 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 41 Int64)))

(case "a constant bin construction folds alongside performs in the same handle body"
  (doc    "The pure-construction control of the bin × effects pair: `(bin (u16 258) (u8 7))` has only
           literal segments, so it is a pure Bytes value the fold treats as opaque data while the sibling
           `(St.next)` discharges normally — 3 + 5 = 8. Pins that a bin ctor's presence does not
           de-classify the body (the effect-reachability walk sees the ctor as pure).")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (Bytes.len (bin (u16 258) (u8 7))) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))

(case "String.slice of a Map-looked-up String with perform-threaded start and end folds"
  (doc    "The String sibling of the looked-up-Bytes slice shape (whose wasm scratch-alias miscompile is
           separately pinned in 10-bytes): the string comes back through `Map.lookup` and BOTH slice
           operands are perform results — start 1, end 2 → slice \"b\", byte-len 1. Note String.slice is
           (start, END) where Bytes.slice is (start, LEN), and returns Option. Pins the looked-up-payload
           × perform-operand shape folding for the String emit.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 "abcdefgh"))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some str)
                      (match (String.slice str (St.next) (St.next))
                        ((Some sl) (String.byte-len sl))
                        ((None _u) -100)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

(case "a let-bound value in a handle body flows into a perform's argument (the always-worked twin)"
  (doc    "The let-twin of the do-def perform-arg repro above — the semantically identical shape with the
           value `let`-bound instead of do-def. This ALWAYS computed correctly (the let rebuilt its scope,
           so the perform-arg path saw the binding); it's the reference the fix normalized the do-def form
           to match. `run 5`: v = 7, `(Ask.ask 7)`→14, +7 → 21. Both backends. Pinned as the regression
           twin so a future fold change that re-breaks the do form (but not the let) is caught by the pair
           diverging.")
  (input  (do
            (effect Ask (op ask (-> Int64 Int64)))
            (def (run (: u Int64))
              (handle Ask 0
                ((ask (n) s (resume (* n 2) s)))
                (let ((v (+ u 2)))
                  (+ (Ask.ask v) v))))
            (def (main) (run 5))
            (export main)))
  (output (: 21 Int64)))

(case "a do-def shared across BOTH resume slots stays in scope (the accumulator-arm shape)"
  (doc    "The RESUME-arg companion of the #21 perform-arg pins above (v-effects 500e59d51 — the multi-use
           residue of the do→let normalization e49c698a1). A handler arm's leading `(def s2 …)` referenced
           in BOTH resume operands — the value arg AND the next-state arg — was CDZ0101 'unbound' in a LIVE
           handler: `peel_resume_from_arm_body` wrapped only the resume VALUE in the leading do-defs and
           returned the next-state BARE, so a do-def feeding both slots orphaned. The fix wraps BOTH slots
           in the leading defs (mirroring the let/match peels — why the let-form below always worked). The
           natural accumulator arm: compute the new state once, resume the derived value + the state.
           `(note (v) s (do (def s2 (List.push s v)) (resume (List.len s2) s2)))` — main(5): note 5 →
           s2=[5], resume len 1 + state [5]; note 20 → s2=[5,20], resume len 2; (1*10 + 2) = 12. All
           backends.")
  (input  (do
            (effect L (op note (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L (list)
                ((note (v) s (do (def s2 (List.push s v)) (resume (List.len s2) s2))))
                (+ (* (L.note n) 10) (L.note 20))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 12 Int64)))

(case "the let-form of the dual-resume-slot arm computes (the always-worked oracle twin)"
  (doc    "The let-twin of the do-def dual-resume-slot pin above — semantically identical with `s2`
           let-bound. This ALWAYS compiled (the let rebuilt its scope so both resume operands saw the
           binding); it's the reference 500e59d51 normalized the do-form to match. main(5) → 12, same as
           the do-form. Pinned as the regression twin so a future peel change that re-breaks the do form
           (but not the let) is caught by the pair diverging. All backends.")
  (input  (do
            (effect L (op note (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L (list)
                ((note (v) s (let ((s2 (List.push s v))) (resume (List.len s2) s2))))
                (+ (* (L.note n) 10) (L.note 20))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 12 Int64)))

(case "a SCALAR do-def resumed in both slots stays in scope (scalar twin of the dual-slot fix)"
  (doc    "The scalar twin of the dual-resume-slot fix: a scalar `(def d (+ s v))` resumed as BOTH the
           value and the next-state — `(resume d d)`. Before 500e59d51 this was CDZ0101 (the bare
           next-state slot orphaned `d`); now it stays in scope. `handle L 0`, main(5): note 5 → d=5,
           resume value 5 + state 5; note 20 → d=25, resume 25; (5*10 + 25) = 75. Confirms the fix covers
           the scalar shape, not just heap payloads. All backends.")
  (input  (do
            (effect L (op note (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L 0
                ((note (v) s (do (def d (+ s v)) (resume d d))))
                (+ (* (L.note n) 10) (L.note 20))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 75 Int64)))

(case "a do-def referenced twice WITHIN one resume operand compiles (the within-slot control)"
  (doc    "The discriminator control (breaker #24 perimeter): multi-reference of a do-def WITHIN a single
           resume operand `(resume (+ d d) s)` ALWAYS compiled — the break was STRICTLY CROSS-slot (a
           shared def spanning the value-arg AND state-arg), because the two operands were lowered as
           separate scopes and only the value arg carried the leading defs. This pins that within-slot
           multi-reference is not the bug: `(def d (+ v 1))` used as `(+ d d)` in the value slot, state
           `s` bare. main(5): note 5 → d=6, resume (6+6)=12 + state 0; note 20 → d=21, resume 42; (12*10 +
           42) = 162. All backends. Triangulates the fix to the cross-slot peel, not do-defs in general.")
  (input  (do
            (effect L (op note (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L 0
                ((note (v) s (do (def d (+ v 1)) (resume (+ d d) s))))
                (+ (* (L.note n) 10) (L.note 20))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 162 Int64)))

(case "an abortive perform in a body tail referencing a do-local binding stays in scope"
  (doc    "The abortive companion of the resuming do-def-in-perform-arg pin above (v-effects 0d382e3f4 —
           a SEPARATE bug from the resuming do→let fix e49c698a1, which is why the let form CDZ0101'd
           identically before this fix). On abort, `reduce_handle` collapsed the handle to the abort value
           and DISCARDED the body's binding scope, so an abort value referencing a body-local `(def v e)`
           orphaned it → CDZ0101 unbound. The fix re-wraps the abort value in its bindings when the body
           fires an abort. `run 5`: v = u+2 = 7, `(Bail.bail v)` abandons the computation → the handle's
           value is 7. Both backends.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: u Int64))
              (handle Bail 0
                ((bail (n) s n))
                (do
                  (def v (+ u 2))
                  (Bail.bail v))))
            (def (main) (run 5))
            (export main)))
  (output (: 7 Int64)))

(case "an abortive perform in a STRICT OPERAND referencing a let-local binding stays in scope"
  (doc    "The strict-operand face of the abortive scope fix (v-effects 0d382e3f4) — the row that CDZ0101'd
           on BOTH the do and let forms before the fix, proving it independent of the resuming do→let
           normalization. The abort perform sits in a strict `+` operand referencing a body-local `let`
           binding: `(let ((v (+ u 2))) (+ (Bail.bail v) 100))`. The abort abandons before the `+`, so the
           `+ 100` never runs; the handle value is the abort value 7. `run 5` → 7. Both backends. Pinned
           beside the resuming pair so the full do-def/abort-in-perform matrix (resuming e49c698a1 +
           abortive 0d382e3f4) has durable corpus coverage.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: u Int64))
              (handle Bail 0
                ((bail (n) s n))
                (let ((v (+ u 2)))
                  (+ (Bail.bail v) 100))))
            (def (main) (run 5))
            (export main)))
  (output (: 7 Int64)))

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

(case "a handler arm computes its RESUME VALUE with a deeply RECURSIVE pure helper"
  (doc    "The recursive upgrade of the effect-free-helper arm (the `dbl` case above is explicitly
           non-recursive): the arm's resume value is `(fib s)` — a doubly-recursive pure function run on
           the handler STATE inside the arm — so the arm's evaluation nests an unbounded pure recursion
           between the perform and the resume. Seeded 10, `fib 10` = 55 resumes to the body. Pins that a
           handler arm may run arbitrary recursive computation to produce its resume value (the arm is an
           ordinary expression context, not a restricted position).")
  (input  (do
            (effect Fib (op get (-> Unit Int64)))
            (def (fib (: n Int64))
              (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
            (def (main (: n Int64))
              (handle Fib n
                ((get (u) s (resume (fib s) s)))
                (Fib.get unit)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 55 Int64))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

(case "a handler arm computes its NEXT-STATE with a recursive pure helper"
  (doc    "The next-state twin: the arm threads `(double-up s 2)` — a tail-recursive helper quadrupling the
           state — as its NEXT-STATE argument. Seeded 1: the first `next` reads 1 and threads `double-up 1
           2` = 4; the second reads 4. `(do (Tw.next) (Tw.next))` = 4. Pins that the resume's SECOND
           argument (the state advance) may be an arbitrary recursive computation over the current state,
           not only a primitive step like `(+ s 1)`.")
  (input  (do
            (effect Tw (op next (-> Unit Int64)))
            (def (double-up (: n Int64) (: k Int64))
              (if (= k 0) n (double-up (* n 2) (- k 1))))
            (def (main (: n Int64))
              (handle Tw n
                ((next (u) s (resume s (double-up s 2))))
                (do (Tw.next unit) (Tw.next unit))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 4 Int64)))

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

(case "a NON-tail one-shot arm that ADVANCES the state threads the advance through the re-reduced continuation"
  (doc    "The two-perform re-reducing fold above holds the state CONSTANT (`(resume 10 s)`); this pins the
           sharper composition where the arm is BOTH non-tail (work wraps the resume) AND state-advancing.
           Arm `(tick (u) s (+ 100 (resume s (+ s 1))))` resumes with the CURRENT state `s` and threads
           `s+1` forward, over the body `(+ (St.tick) (St.tick))`, seeded 0. The leading tick's continuation
           `C = (+ [] (St.tick))`; `(resume 0 1)` re-reduces `C[0] = (+ 0 (St.tick))` under state 1 — the
           inner tick reads 1, resumes `(resume 1 2)` into its own continuation `(+ 0 [])` = 1, its arm
           yields `(+ 100 1)` = 101, so `C[0]` = `(+ 0 101)` = 101; the outer arm then yields `(+ 100 101)`
           = 201. Pins that the `(+ s 1)` advance survives EACH continuation re-reduction — a fold that
           dropped the advance would resume the second tick with 0 too and compute 200, not 201. The state
           threads 0->1->2 across the nested re-reductions while every resume is wrapped by pure work.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main)
              (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (+ (St.tick) (St.tick)))) (export main)))
  (output (: 201 Int64)))

(case "a NON-tail state-advancing arm threads through a NON-commutative continuation"
  (doc    "The non-commutative companion of the case above — it pins BOTH the continuation nesting AND the
           left-to-right state advance at once, since a fold that dropped the advance lands on a different
           value. Same arm `(tick (u) s (+ 100 (resume s (+ s 1))))` seeded 0, but the body subtracts:
           `(- (St.tick) (St.tick))`. The leading tick's continuation `C = (- [] (St.tick))`; `(resume 0 1)`
           re-reduces `C[0] = (- 0 (St.tick))` under state 1 — the inner tick reads 1, resumes into its own
           continuation `(- 0 [])` = -1, its arm yields `(+ 100 -1)` = 99, so `C[0]` = 99; the outer arm
           then yields `(+ 100 99)` = 199. A fold that read both ticks at the SAME state (advance dropped)
           would resume the second tick with 0 and compute 200, not 199. Both backends agree.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main)
              (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (- (St.tick) (St.tick)))) (export main)))
  (output (: 199 Int64)))

(case "a NON-tail state-advancing arm threads the advance through a perform in an if CONDITION"
  (doc    "The two cases above pin the non-tail state-advancing arm `(tick (u) s (+ 100 (resume s (+ s 1))))`
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
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main)
              (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (if (< (St.tick) 50) (+ 2000 (St.tick)) 999))) (export main)))
  (output (: 2201 Int64)))

(case "a NON-tail state-advancing arm threads the advance through a perform in a let INIT"
  (doc    "The let-init companion of the if-condition case above: the same non-tail state-advancing arm
           `(tick (u) s (+ 100 (resume s (+ s 1))))` with the leading perform as a `let` INIT whose binding
           is reused, and a SECOND perform in the let body. Body `(let ((x (St.tick))) (+ (* 1000 (+ x 1))
           (St.tick)))`, seed 0. The init tick reads 0, its continuation `C = (let ((x [])) (+ (* 1000
           (+ x 1)) (St.tick)))`; `(resume 0 1)` re-reduces `C[0]` under state 1 with `x` bound to 0 — the
           body `(+ (* 1000 (+ 0 1)) (St.tick))` = `(+ 1000 (St.tick))`: the inner tick reads 1 (advanced),
           resumes into its continuation `(+ 1000 [])` = 1001, its arm yields `(+ 100 1001)` = 1101, so
           `C[0]` = 1101; the outer arm yields `(+ 100 1101)` = 1201. Pins that the advance survives the
           let-init re-reduction AND that the bound `x` reads the PRE-advance state 0 (a fold that let `x`
           see the advanced 1 would compute `(* 1000 2)` = 2000-based, not 1000-based).")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main)
              (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (let ((x (St.tick))) (+ (* 1000 (+ x 1)) (St.tick))))) (export main)))
  (output (: 1201 Int64)))

(case "a NON-tail state-advancing arm threads the advance through a perform in a match SCRUTINEE and its arm body"
  (doc    "The match-scrutinee companion of the if-condition + let-init distribution cases above — the third
           strict-first seam. Same non-tail state-advancing arm `(tick (u) s (+ 100 (resume s (+ s 1))))`,
           body `(match (St.tick) (0 111) (_ (+ 1 (St.tick))))`, seed 5. The scrutinee tick reads 5, its
           continuation `C = (match [] (0 111) (_ (+ 1 (St.tick))))`; `(resume 5 6)` re-reduces `C[5]` under
           state 6 — 5 is not the `0` literal so the `_` arm `(+ 1 (St.tick))` runs: the inner tick reads 6
           (the ADVANCED state), resumes into its own continuation `(+ 1 [])` = 7, its arm yields
           `(+ 100 7)` = 107, so `C[5]` = 107; the outer arm then yields `(+ 100 107)` = 207. Pins that the
           `(+ s 1)` advance reaches the arm-body tick across the scrutinee re-reduction — a constant-state
           arm `(resume s s)` (advance dropped) would read the arm-body tick at 5 and compute 206, not 207.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main)
              (handle St 5 ((tick (u) s (+ 100 (resume s (+ s 1))))) (match (St.tick) (0 111) (_ (+ 1 (St.tick)))))) (export main)))
  (output (: 207 Int64)))

(case "a NON-tail state-advancing arm threads the advance through a perform in a matched literal arm reached via the scrutinee"
  (doc    "The literal-arm face of the match-scrutinee case above: the scrutinee dispatch selects a SCALAR
           LITERAL arm that itself performs (not the wildcard). Same arm `(tick (u) s (+ 100 (resume s
           (+ s 1))))`, body `(match (St.tick) (0 (+ 7 (St.tick))) (_ 222))`, seed 0. The scrutinee tick
           reads 0, `C = (match [] (0 (+ 7 (St.tick))) (_ 222))`; `(resume 0 1)` re-reduces `C[0]` under
           state 1 — 0 matches the `0` literal arm `(+ 7 (St.tick))`: the inner tick reads 1 (advanced),
           resumes into `(+ 7 [])` = 8, its arm yields `(+ 100 8)` = 108, so `C[0]` = 108; the outer arm
           yields `(+ 100 108)` = 208. Pins the advance reaching a performing LITERAL arm (not just the
           wildcard) selected by the re-reduced scrutinee.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main)
              (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (match (St.tick) (0 (+ 7 (St.tick))) (_ 222)))) (export main)))
  (output (: 208 Int64)))

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

(case "a multi-shot ctl arm whose continuation reads an ENCLOSING-fn param folds"
  (doc    "A multi-shot E5 within-activation arm `(pick (u) s k (+ (k 1) (k 2)))` — `k` (the reified
           delimited continuation) applied TWICE — over a handle body `(let ((y 3)) (+ n (Amb.pick)))` whose
           continuation `C = (+ n [])` reads an ENCLOSING function param `n`. The fold splices a FRESH copy
           of `C` per `k`-application (2 copies), so `C[1]` = `(+ n 1)` and `C[2]` = `(+ n 2)`, and the arm
           yields `(+ (+ n 1) (+ n 2))`; with `n = 5` that is `(+ 6 7)` = 13. Pins that the per-resume splice
           PRESERVES `C`'s enclosing captures: without pinning `n` before the splice each copy re-resolves it
           against its own orphan and reports a false CDZ0101 'unbound n' (breaker mv-class). The arm's own
           `k`/state binders and the body-local `let` binder `y` are unaffected — only the enclosing capture
           needed pinning. Both backends agree.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0 ((pick (u) s k (+ (k 1) (k 2)))) (let ((y 3)) (+ n (Amb.pick))))) (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64)))

(case "a ONE-shot ctl arm whose continuation reads an enclosing-fn param folds (mv single-splice control)"
  (doc    "The single-resume control for the multi-shot enclosing-capture case: the SAME body
           `(let ((y 3)) (+ n (Amb.pick)))` but a ONE-shot arm `(pick (u) s k (k 1))` — `k` applied once, so
           `C = (+ n [])` is spliced a SINGLE time → `C[1]` = `(+ n 1)`; with `n = 5` that is `6`. A single
           splice never needed the capture pin (one copy, one resolution), so this held before the mv-class
           fix; it stays green after, confirming the fix does not disturb the single-splice path.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0 ((pick (u) s k (k 1))) (let ((y 3)) (+ n (Amb.pick))))) (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "a multi-shot ctl arm reading an enclosing param with NO let frame folds (mv no-let control)"
  (doc    "The no-let control: the multi-shot arm `(pick (u) s k (+ (k 1) (k 2)))` over a body with NO
           intervening `let` — `(+ n (Amb.pick))` directly — so `C = (+ n [])` reading the enclosing param
           `n`. Isolates that the fix is about preserving the enclosing capture `n` across the per-resume
           splice, independent of a body-local binding frame: `(+ (+ n 1) (+ n 2))` with `n = 5` = 13,
           matching the let-wrapped case.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0 ((pick (u) s k (+ (k 1) (k 2)))) (+ n (Amb.pick)))) (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64)))

(case "a two-site resume arm whose resume VALUE reads an enclosing-fn param folds"
  (doc    "The ARM-SIDE enclosing-capture face (breaker mv-class, distinct from the continuation-C face): a
           two-site resume arm `(+ (resume (+ n 1) s) (resume 2 s))` whose FIRST resume VALUE `(+ n 1)` reads
           an ENCLOSING function param `n`. The arm body is β-substituted then its resume occurrences rewrite
           to `C[value]` per site — so the resume VALUE (carrying free `n`) is copied per resume. Without
           pinning `n` in the arm body BEFORE the β-substitution (which detaches it — the copied `n` loses
           its binder), each per-site copy re-resolves `n` unbound → false CDZ0101. Here `C = (let ((x []))
           (+ (* 10 x) x))`: resume 1 with `(+ n 1)` = 6 → `x=6` → 66, resume 2 with 2 → `x=2` → 22, arm =
           `(+ 66 22)` = 88 (n = 5). Pins the resume-value enclosing-capture preservation across the
           multi-site splice.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
                (let ((x (Amb.pick))) (+ (* 10 x) x)))) (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))

(case "a two-site resume arm reading the enclosing SEED param in its resume value folds"
  (doc    "The SEED face of the arm-side enclosing capture: the handle is seeded by the enclosing param
           `(handle Amb n …)`, and the arm's first resume value `(+ s 1)` reads the state `s` (= the seed on
           first entry). The seed `n` reaches the arm via the state-binder substitution, so it appears in the
           β-substituted arm body (not the original) — pinned there so the per-site splice shares it. `C =
           (let ((x [])) (+ (* 10 x) x))`, seed 5: resume 1 value `(+ s 1)` = 6 → 66, resume 2 value 2 → 22,
           arm = 88.")
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((pick (u) s (+ (resume (+ s 1) s) (resume 2 s))))
                (let ((x (Amb.pick))) (+ (* 10 x) x)))) (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))

(case "a two-site resume arm whose resume value is a heap LIST reading an enclosing param folds"
  (doc    "The heap-payload variant of the arm-side enclosing-capture face: the resume value is a `(list n 2
           9)` reading the enclosing param `n`, so a multi-node list payload carrying an enclosing capture
           crosses the per-site splice. `C = (let ((xs [])) (List.len xs))`: resume 1 value `(list n 2 9)` =
           a 3-element list → len 3, resume 2 value `(list 7)` → len 1, arm = `(+ 3 1)` = 4. Confirms the
           enclosing-capture pin works when the resume value is a heap constructor, not just a scalar.")
  (input  (do
            (effect Amb (op pick (-> Unit (List Int64))))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (list n 2 9) s) (resume (list 7) s))))
                (let ((xs (Amb.pick))) (List.len xs)))) (export main)))
  (call   main (: 5 Int64)) (output (: 4 Int64)))

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

(case "multi-shot resumes carry DIVERGENT states; two performs branch 2x2"
  (doc    "Every multi-shot pin above resumes with the SAME state; here the two resumes carry DIFFERENT
           next-states (`(+ s 10)` vs `(+ s 20)`), and a second perform re-branches each path under its
           own inherited state — a 2×2 tree where each leaf's value reflects its lineage. Per branch:
           k(v) = v + (1 + 2) under that branch's state = 2v + 3; k(1) = 5, k(2) = 7 → 12. Pins that
           each re-reduction threads ITS OWN state forward, not a shared or last-written one.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 1 (+ s 10)) (resume 2 (+ s 20)))))
                (+ (Amb.flip) (Amb.flip))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64)))

(case "each multi-shot branch OBSERVES its own divergent state via a trailing peek"
  (doc    "The observability face of divergent multi-shot states: branch k(1) inherits state 10, branch
           k(2) inherits 20, and each branch's trailing `peek` reads its own — 10·1 + 10 = 20 and
           10·2 + 20 = 40 → 60. A shared-state implementation (both branches seeing one cell) would
           yield 50 or 70; the checksum separates the worlds.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 1 (+ s 10)) (resume 2 (+ s 20))))
                 (peek (u) s (resume s s)))
                (+ (* 10 (Amb.flip)) (Amb.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64)))

(case "divergent multi-shot states carry HEAP lineages — each branch grows its own list"
  (doc    "The Perceus-critical SHAPE: the two resumes push DIFFERENT elements onto the list state, so
           each branch of the 2×2 tree owns an independent heap lineage. The body is `(+ (* 10 flip₁)
           flip₂)`: the outer flip branches k(1) and k(2), and inside each, the second flip re-branches
           to (1 + 2) = 3 — so k(v) = 10v + 10v + 3 = 20v + 3; k(1) = 23, k(2) = 43 → 66. NOTE this
           case's resumed values are constants and nothing here reads the list, so its checksum alone
           cannot detect a shared in-place list — it pins the divergent-push shape compiling and
           running; the SIBLING below (per-branch `size`) is the case that OBSERVES the lineage
           separation.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb (list)
                ((flip (u) s (+ (resume 1 (List.push s 10)) (resume 2 (List.push s 20)))))
                (+ (* 10 (Amb.flip)) (Amb.flip))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 66 Int64)))

(case "each multi-shot branch observes its own heap-lineage length"
  (doc    "The heap observability face: each branch's trailing `size` reads the length of ITS list —
           both branches see exactly one element (their own push), never the sibling's: 10·1 + 1 = 11
           and 10·2 + 1 = 21 → 32. A shared list would read length 2 on the second branch.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)) (op size (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb (list)
                ((flip (u) s (+ (resume 1 (List.push s 10)) (resume 2 (List.push s 20))))
                 (size (u) s (resume (List.len s) s)))
                (+ (* 10 (Amb.flip)) (Amb.size))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 32 Int64)))

(case "divergent multi-shot STRING lineages — each branch observes its own byte-length"
  (doc    "The rope-representation twin of the list-lineage pins above (Strings are rope-backed, a
           DIFFERENT heap representation from RRB lists): branch k(1) concats \\\"a\\\" (byte-len 1),
           k(2) concats \\\"bb\\\" (byte-len 2), and each branch's trailing `len` reads ITS OWN —
           10·1 + 1 = 11 and 10·2 + 2 = 22 → 33. A rope in-place append shared across branches would
           read 3 on the sibling; the divergence property needs its own witness per representation.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)) (op len (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb ""
                ((flip (u) s (+ (resume 1 (String.concat s "a")) (resume 2 (String.concat s "bb"))))
                 (len (u) s (resume (String.byte-len s) s)))
                (+ (* 10 (Amb.flip)) (Amb.len))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 33 Int64)))

(case "an arm SUMS one resumption with a constant (the 1.5-shot shape)"
  (doc    "Between single-shot and multi-shot: the arm's value mixes ONE continuation result with a
           non-continuation term — `(+ (resume 1 s) 100)`. k(1) = 1 + 5 = 6, arm value 6 + 100 = 106.
           Pins that the arm's value expression composes a resumption result with ordinary arithmetic
           (the resume is not required to be the whole arm value).")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 1 s) 100)))
                (+ (Amb.flip) 5)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64)))

(case "an arm CONDITIONS its shot count — the multi-shot branch"
  (doc    "Every multi-shot pin above has a STATIC shot count; here the arm chooses AT RUN TIME —
           `(if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))` — two resumptions on one branch,
           one on the other. Seed 5 takes the multi-shot branch: k(1) = 6, k(2) = 7 → 13. The
           single-shot branch of the SAME program is pinned below; a dynamically-chosen shot count is
           the real shape of backtracking search.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((flip (u) s (if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))))
                (+ (Amb.flip) 5)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64)))

(case "the conditional-shot-count arm's SINGLE-shot branch (same program, other input)"
  (doc    "The other runtime path of the conditional-count arm above: seed 2 fails `(> s 3)`, so the
           arm resumes ONCE with 9 → 9 + 5 = 14. Together the pair pins both dynamic outcomes of one
           compiled handler — the shot count is a runtime property of the dispatch, not a static
           property of the arm.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((flip (u) s (if (> s 3) (+ (resume 1 s) (resume 2 s)) (resume 9 s))))
                (+ (Amb.flip) 5)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 14 Int64)))

(case "a multi-shot continuation contains a NESTED handler — each re-reduction re-enters it fresh"
  (doc    "The continuation being re-reduced holds a whole nested `handle In` (a separate effect,
           seed 7): each of the two re-reductions must RE-INSTANTIATE the nested frame from its seed —
           both branches read 7 (k(10) = 17, k(20) = 27 → 44), never an 8 leaked from the sibling's
           instance. Pins per-re-reduction frame re-instantiation.")
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (effect In (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 10 s) (resume 20 s))))
                (+ (Amb.flip)
                   (handle In 7
                     ((get (u) t (resume t (+ t 1))))
                     (In.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 44 Int64)))

(case "a MULTI-shot continuation APPLIES a captured closure per re-reduction"
  (doc    "The closure composition of the captured-heap multi-shot cases above: the re-reduced continuation
           `(scale (+ (Go.fork) 10))` applies `scale = (fn (x) (* x n))` — a closure over `main`'s runtime
           parameter — once per resumption. Each re-reduction must find the closure (and its env) alive:
           k(1) → scale(11) = 55, k(2) → scale(12) = 60 → 115. A closure freed or its env dropped after
           the first resume breaks the second application.")
  (input  (do
            (effect Go (op fork (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def scale (fn ((: x Int64)) (* x n)))
                (handle Go 0
                  ((fork (u) s (+ (resume 1 s) (resume 2 s))))
                  (scale (+ (Go.fork) 10)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64)))

(case "a MULTI-shot continuation PUSHES onto a captured list per re-reduction (fresh copy each)"
  (doc    "The double-consume composition: each re-reduction runs `(List.push (List.push (list n) (Go.fork))
           7)` — TWO pushes onto a list built from the captured `n` — and reports its length. Both
           re-reductions must see a fresh 1-element base (len 3 each → 6); an FBIP in-place grow shared
           across resumes would give the second a longer list. Extends the dup-per-resume pin above from
           one consuming op to a consuming CHAIN.")
  (input  (do
            (effect Go (op fork (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Go 0
                ((fork (u) s (+ (resume 1 s) (resume 2 s))))
                (List.len (List.push (List.push (list n) (Go.fork)) 7))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

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

(case "a let-wrapped resume in the arm body threads a computed next-state (stateful PRNG)"
  (doc    "The two-hole refold handles an arm body that is a `let` whose binder feeds BOTH the resume value
           and the resume next-state: `(roll (k) s (let ((s2 (* s 16807))) (resume (% s2 k) s2)))` — a linear-
           congruential PRNG draw. `resolved_of` peels the `let` and would hand back a `Resume` whose value
           `(% s2 k)` and next-state `s2` reference the let binder `s2` DANGLING (the enclosing `let` dropped),
           so the recursive re-seed would see `s2` unbound. The refold matches the `let` STRUCTURALLY before
           the resume check and INLINES the (pure) binding `s2 := (* s 16807)`, closing the resume's value and
           next-state so each draw re-seeds the recursive fold with the advanced state. Two sequential draws:
           seed 7 → s1 = 7*16807 = 117649, x = 117649 % 1000 = 649; s2 = 117649*16807, y = s2 % 1000 = 743;
           `(+ x y)` = 1392. One resume per arm activation, so each `C` is spliced once — the LCG step runs
           exactly once per draw (no effect duplication).")
  (input  (do
            (effect Prng (op roll (-> Int64 Int64)))
            (def (main)
              (handle Prng 7 ((roll (k) s (let ((s2 (* s 16807))) (resume (% s2 k) s2))))
                (let ((x (Prng.roll 1000))) (let ((y (Prng.roll 1000))) (+ x y))))) (export main)))
  (output (: 1392 Int64)))

(case "a closure capturing an inner-handled perform result is applied under an OUTER handler of the same effect"
  (doc    "A CLOSURE built inside an INNER handle captures a `let`-bound perform RESULT (`base`), then escapes
           to be applied under an OUTER handler of the SAME effect. The capture must be the inner-handled
           VALUE, NOT a re-perform: `base` is bound to `(Ctr.tick)` under `handle Ctr 50` (a get/set arm),
           so base = 50 and the closure is `(fn (x) (+ x 50))`. Applied under `handle Ctr 5` as `(f 3)`, the
           result must be 3 + 50 = 53. It MISCOMPILED to 8 = 3 + 5 (each apply RE-performed the tick at the
           apply site, re-homed by the OUTER handler) because the capture was compiled as the perform
           EXPRESSION, not its value — the closure-capture-reperform miscompile. Fixed by discharging the
           inner handle when reducing the returned closure (`lambda_of`) AND closing a pure captured binding
           into the closure body before threading detaches it (the let-thread capture-value inline), so the
           closure closes over the value 50, not the perform.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5
                ((tick (u) s (resume s (+ s 1))))
                (let ((f (handle Ctr 50
                           ((tick (u) s (resume s (+ s 1))))
                           (let ((base (Ctr.tick)))
                             (fn ((: x Int64)) (+ x base))))))
                  (f 3))))
            (export main)))
  (output (: 53 Int64)))

(case "a closure captures TWO inner-handled perform results across NESTED lets and escapes under an outer handler"
  (doc    "The nested-`let` sibling of the capture case: the inner handle's body is a `(let ((a (Ctr.tick)))
           (let ((b (Ctr.tick))) (fn (x) (+ x (+ a b)))))` — an OUTER `let` binding `a` referenced by a
           closure buried in the INNER `let`. Both captures must close over their inner-handled VALUES (a =
           50, b = 51 under `handle Ctr 50`, threading state), so the closure is `(fn (x) (+ x 101))` and
           `(f 3)` under `handle Ctr 5` = 3 + 101 = 104. It over-declined CDZ0101 `unbound a` because the
           capture-value inline gated on the let body being DIRECTLY a lambda — here it is another `let`, so
           the outer capture `a` orphaned. Fixed by peeling let-chains (`body_returns_lambda`) so a closure
           reached through nested lets still closes over the outer capture.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5
                ((tick (u) s (resume s (+ s 1))))
                (let ((f (handle Ctr 50
                           ((tick (u) s (resume s (+ s 1))))
                           (let ((a (Ctr.tick)))
                             (let ((b (Ctr.tick)))
                               (fn ((: x Int64)) (+ x (+ a b))))))))
                  (f 3))))
            (export main)))
  (output (: 104 Int64)))

(case "a closure capturing perform results escapes to NO handler at all and applies pure"
  (doc    "The no-outer-handler sibling of the capture cases above (those apply the escapee under an OUTER
           handler of the same effect; here NOTHING handles the effect at the apply sites): the handle's
           RESULT is a closure whose captures are two perform results (x = 5, y = 6 under the advancing
           arm), applied TWICE after the handle fully exits. Both applications must read the captured
           VALUES — (f 10) = 56 and (f 100) = 506 → 562. A capture compiled as the perform expression
           would need a handler at apply and could only reject or re-home; the values must live in the
           closure env, independent of any handler existing.")
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def f (handle St n
                         ((a (u) s (resume s (+ s 1))))
                         (do
                           (def x (St.a))
                           (def y (St.a))
                           (fn ((: k Int64)) (+ (* k x) y)))))
                (+ (f 10) (f 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 562 Int64)))

(case "a handle's RESULT seeds the NEXT handle of the same effect — explicit state handoff between instances"
  (doc    "Sequential same-effect handle instances share nothing implicitly (each seeds fresh); the ONLY
           state transfer is explicit value flow. The first instance's result (its last-read state 8, after
           a +5 advance) becomes the second instance's SEED, whose doubling arm then serves 8 and 16 →
           8 + 16 = 24. Pins the instance-lifecycle boundary: a leak of the first instance's live state
           into the second (rather than the passed value) or a stale-seed re-read would shift both reads.")
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def r1 (handle St n ((a (u) s (resume s (+ s 5)))) (+ (* 0 (St.a)) (St.a))))
                (handle St r1 ((a (u) s (resume s (* s 2)))) (+ (St.a) (St.a)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 24 Int64)))

(case "a CURRIED closure capturing an inner-handled perform result closes over it through partial application"
  (doc    "A curry sibling of the capture case: the inner handle returns `(fn (a) (fn (b) (+ (+ a b) base)))`
           where `base` is the inner-handled `(Ctr.tick)` = 50. Applied `((f 3) 4)` under `handle Ctr 5`, the
           OUTER lambda binds a=3 and returns the residual `(fn (b) (+ (+ 3 b) 50))` (base closed over the
           inner value), then the residual binds b=4 → 3+4+50 = 57. Exercises the closure-capture fix through
           `apply_lambda`'s partial-application/curry path (the reified closure is itself lambda-returning),
           distinct from the direct and nested-let cases. Pins that a captured perform result stays the VALUE
           across currying, never re-performed at either application.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5
                ((tick (u) s (resume s (+ s 1))))
                (let ((f (handle Ctr 50
                           ((tick (u) s (resume s (+ s 1))))
                           (let ((base (Ctr.tick)))
                             (fn ((: a Int64)) (fn ((: b Int64)) (+ (+ a b) base)))))))
                  ((f 3) 4))))
            (export main)))
  (output (: 57 Int64)))

(case "a closure capturing a value computed under a handler may escape the handle applied directly"
  (doc    "The discharge-then-capture idiom (the 'configure a callback from handled state' pattern): the
           perform `(St.get)` runs INSIDE the handle body (a `let` init — the handler is live), and the
           ESCAPING closure captures only the resulting Int64 VALUE `v`. The closure performs nothing, so the
           escape is sound — `((handle St k (arm) (let ((v (St.get))) (fn (x) (+ x v)))) 10)` with k=7 folds
           v=7, closes over it, and applying the escaped closure to 10 yields 17. This was over-rejected
           CDZ0401 (the escape analysis conflated a lexically-inner perform with an escaping one); the
           `lambda_of` handler-discharge fix (the closure-capture-reperform family) folds the in-extent
           `St.get` to its value so the escaped closure is pure. The genuinely-unsound twin — the closure
           BODY performing `(fn (x) (+ x (St.get)))` — correctly STAYS rejected (its perform runs
           out-of-extent on outside-application); this case pins the sound half of that boundary.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: k Int64))
              ((handle St k
                 ((get (u) s (resume s s)))
                 (let ((v (St.get)))
                   (fn ((: x Int64)) (+ x v))))
               10))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 17 Int64)))

(case "a BARE-param escaping closure capturing a handled value declines cleanly (annotated-param twin folds)"
  (doc    "The BARE-parameter twin of the escaping-closure-captures-handled-value case above (v-effects
           self-probe 2026-08-04). The SAME sound shape — a closure capturing a `let`-bound in-extent perform
           result `v`, escaping the handle, applied outside — but the closure's parameter is BARE `(fn (x) (+
           x v))` instead of annotated `(fn ((: x Int64)) …)`. The annotated twin (the case above) FOLDS to
           17; the bare-param version DECLINES with `parameter reference has no local slot` (select.rs:10382,
           the Core::Param no-slot arm) on all 3 backends. A CLEAN decline (compile-time, never a wrong value)
           — the escaping-closure lift (`lambda_of`/env-snapshot) slots an ANNOTATED closure param but not a
           BARE one when the closure captures a handler-computed value, so the bare param's reference reaches
           emit un-slotted. A completeness gap in the closure-capture-escape family (NOT a miscompile): the
           bare-param lift needs the same slot allocation the annotated path gets. Pinned as a decline-witness
           to lock the bare-vs-annotated boundary; flips to 17 PASS when the bare-param escaping-closure lift
           slots the param. Related to the sibling-closures-sharing-outer-capture-scope decline (same no-slot
           arm, different trigger).")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              ((handle St 7 ((get (u) s (resume s s))) (let ((v (St.get))) (fn (x) (+ x v)))) 10))
            (export main)))
  (call   main) (output (: 17 Int64)))

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

(case "a recursive builder PERFORMS per step and a recursive pure fold consumes the built list"
  (doc    "The two recursive helpers above composed, with the effect in the OPPOSITE one: here the
           recursion that performs is the BUILDER — `(grab k acc)` pushes one `(Cnt.bump)` result per
           step for four steps — and the CONSUMER `(suml xs)` is a pure generic match-recursion over the
           result. The counter arm resumes the current count and advances (seed 5 → resumes 5,6,7,8), so
           the built list is [5 6 7 8] and the pure fold sums it to 26. Pins the build-then-fold pipeline
           under ONE handle: an effect-specialized recursion hands a heap list across to an effect-FREE
           recursion, and each `bump`'s resume value must land in its own list slot (a re-served or
           re-ordered perform shifts a slot and breaks the sum).")
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (suml xs)
              (match xs
                ((list) 0)
                ((list h .. t) (+ h (suml t)))))
            (def (grab (: k Int64) (: acc (List Int64)))
              (if (= k 0) acc (grab (- k 1) (List.push acc (Cnt.bump)))))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (suml (grab 4 (list)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 26 Int64)))

(case "MUTUALLY recursive helpers BOTH perform against the same handler"
  (doc    "The recursion pins above are all SINGLE functions; here `evens`/`odds` call each other and BOTH
           perform `(Cnt.tick)`, with different weights per side (×10 vs ×1) so a dispatch that specializes
           only one side of the cycle — or re-serves a tick to the wrong caller — lands off the checksum.
           Ticks walk 5,6,7,8 alternating sides: 10·5 + 6 + 10·7 + 8 = 134. Pins effect-specialization
           across a mutual-recursion CYCLE, not just self-recursion.")
  (input  (do
            (effect Cnt (op tick (-> Unit Int64)))
            (def (evens (: k Int64))
              (if (= k 0) 0 (+ (* 10 (Cnt.tick)) (odds (- k 1)))))
            (def (odds (: k Int64))
              (if (= k 0) 0 (+ (Cnt.tick) (evens (- k 1)))))
            (def (main (: n Int64))
              (handle Cnt n
                ((tick (u) s (resume s (+ s 1))))
                (evens 4)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 134 Int64)))

(case "a mutual-recursion pair where each side performs against its OWN handler (two nested frames)"
  (doc    "The two-frame composition of the mutual-cycle pin above: `pa` performs the OUTER `A`, `pb` the
           INNER `B`, so every hop around the cycle alternates WHICH live frame serves — and each frame
           advances independently (A: 5,6 stepped ×1; B: 100,110 stepped ×10). 10·5 + 100 + 10·6 + 110 =
           320. A cross-frame mixup (either handler serving the other's op, or an advance landing on the
           wrong state) breaks the place-value sum.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (pa (: k Int64))
              (if (= k 0) 0 (+ (* 10 (A.a)) (pb (- k 1)))))
            (def (pb (: k Int64))
              (if (= k 0) 0 (+ (B.b) (pa (- k 1)))))
            (def (main (: n Int64))
              (handle A n
                ((a (u) s (resume s (+ s 1))))
                (handle B 100
                  ((b (u) t (resume t (+ t 10))))
                  (pa 4))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 320 Int64)))

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

; Every handle seed in the cases above is a CONSTANT. A seed may be a RUNTIME value — a caller
; argument flowing into the handler's initial state — and the fold must genuinely START from it (a
; seed baked at compile time, or a let-bound handle whose runtime seed was mishandled by the fold,
; produces a value independent of the argument). Two calls with different seeds witness the
; dependence.

(case "a HEAP handler seed stays readable in the body after performs advance the state"
  (doc    "The ALIAS face of a heap-valued handler seed: `seed` (a let-bound list) is BOTH the handler's
           initial state — which two performs then advance via `List.push` — AND a binding the body
           re-reads AFTER those performs. The state hand-off at the handler boundary must DUP the seed,
           not take it uniquely: a reuse that treated the seed as dead after seeding would let the
           state's pushes clobber the shared payload, and the body's `(List.at seed 0)` would read a
           pushed value instead of the original k. resume values are the PRE-push lengths (1, 2), so
           a = 1, b = 2, and the re-read gives k = 5 → 1 + 2 + 500 = 503. The heap-STATE pins nearby
           thread list/record/set states; the runtime-seed pins use scalars — this is the heap-seed
           aliased-and-re-read composition neither covers.")
  (input  (do
            (effect Acc (op push (-> Int64 Int64)))
            (def (main (: k Int64))
              (let ((seed (list k)))
                (handle Acc seed
                  ((push (v) s (resume (List.len s) (List.push s v))))
                  (let ((a (Acc.push 10)))
                    (let ((b (Acc.push 20)))
                      (+ (+ a b)
                         (* 100 (match (List.at seed 0) ((Some v) v) ((None _u) -1)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 503 Int64)))

(case "a handle seeded from a runtime caller argument advances from that seed"
  (doc    "`(handle Ctr seed …)` where `seed` is main's PARAMETER — the handler's initial state is a
           runtime value, not a compile-time constant. Two ticks encode 100·first + second: seeded 7 →
           7, 8 → 708; seeded 50 → 50, 51 → 5051. The two calls returning seed-dependent values pin
           that the fold starts from the LIVE argument (a compile-time-baked seed, or a state slot
           initialized before the argument arrives, returns the same value for both calls). The
           runtime-seed companion of the constant-seeded counter fold above.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: seed Int64))
              (handle Ctr seed ((tick (u) s (resume s (+ s 1))))
                (+ (* 100 (Ctr.tick)) (Ctr.tick))))
            (export main)))
  (call   main (: 7 Int64))  (output (: 708 Int64))
  (call   main (: 50 Int64)) (output (: 5051 Int64)))

(case "a let-bound handle with a runtime seed composes with arithmetic after the let"
  (doc    "The let-bound face the runtime-seed fold fix targets: `(let ((r (handle Get seed … (+
           (Get.get) 1)))) (* r 2))` — the handle's value is bound, then consumed by later arithmetic.
           Seeded 20 the perform reads 20, the body yields 21, and the doubled result is 42. Pins that
           a let-bound handle whose seed is a caller runtime arg folds cleanly into the enclosing
           computation (the handle is not the def's tail, so its fold must compose, not just
           terminate). Expected: 42.")
  (input  (do
            (effect Get (op get (-> Unit Int64)))
            (def (main (: seed Int64))
              (let ((r (handle Get seed ((get (u) s (resume s s))) (+ (Get.get) 1))))
                (* r 2)))
            (export main)))
  (call   main (: 20 Int64)) (output (: 42 Int64)))

; The counter fold above SEQUENCES its performs with `do` — each perform is a separate statement, so
; the state advance is witnessed only through the last value. These pin the fold where the advancing
; state is observed by ORDER-SENSITIVE operand positions instead: two performs as SIBLING operands of
; one arithmetic expression. The values differ per site (the counter advances between them), so the
; operand evaluation ORDER is observable — an emit that evaluated the right operand first, or batched
; the two performs against one state read, would produce a different value, not just a different trace.

(case "a stateful counter is observed left-to-right by sibling performs in one expression"
  (doc    "`(+ (* 100 (Fresh.next)) (Fresh.next))` under the counter arm seeded 0: the LEFT perform reads
           0 and advances to 1, the RIGHT reads 1 → 0·100 + 1 = 1. The `*100` weighting makes the order
           observable in the VALUE: right-first evaluation would give 1·100 + 0 = 100. The sibling-operand
           companion of the `do`-sequenced counter fold above — same arm, but the state advance is
           witnessed by strict left-to-right operand evaluation inside a single expression, the order
           #Operands Evaluate Left To Right fixes. Expected: 1.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (Fresh.next)) (Fresh.next))))
            (export main)))
  (output (: 1 Int64)))

(case "sibling performs feeding a subtraction witness the advancing state non-commutatively"
  (doc    "`(- (Fresh.next) (Fresh.next))` seeded 5: left reads 5, right reads 6 → 5 − 6 = −1. The
           non-commutative twin of the weighted-add case above — subtraction needs no weighting to expose
           a swapped order (it would flip the sign to +1), so this is the minimal order witness over an
           advancing handler state. Expected: -1.")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle Fresh 5 ((next (u) s (resume s (+ s 1))))
                (- (Fresh.next) (Fresh.next))))
            (export main)))
  (output (: -1 Int64)))

(case "a stateful counter threads through a RECURSIVE callee performing inside the handled region"
  (doc    "`drain` is a self-recursive function performing `(Fresh.next)` once per level, called from the
           handle body with a RUNTIME depth: seeded 10 at n=3 the three activations read 10, 11, 12 →
           10+11+12 = 33; n=0 performs nothing → 0. The stateful-fold companion of the delegation-reaches-
           a-recursive-callee capability case (04-capabilities): there the effect is DELEGATED to the
           entrypoint, here it is DISCHARGED by an in-program handler whose state must thread OUT of one
           recursive activation and INTO the next — across call frames, not just across statements in one
           body. An emit that re-seeded the handler per activation (3×10=30) or read one stale state for
           all levels would miscount. Runtime `n` keeps the recursion out of the fold. Expected: 33 (n=3),
           0 (n=0).")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (def (drain (: n Int64))
              (if (<= n 0) 0 (+ (Fresh.next) (drain (- n 1)))))
            (def (main (: n Int64))
              (handle Fresh 10 ((next (u) s (resume s (+ s 1))))
                (drain n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

(case "a MODULE-exported recursive callee performing per step is homed by the importer's handler"
  (doc    "The MODULE-EXPORT face of the recursive-callee-performing case above: `walk` is a self-recursive
           performer of `Ctr.next`, but it lives INSIDE `(module m …)` and is called through the projection
           `(. m walk)` from the importer's handle body. The handler-context monomorphization must reach the
           module-exported recursive callee — re-homing its per-step perform (and its recursive self-calls)
           under the importer's handler — exactly as it does for a bare-named recursive performer (case
           above). Seeded 1, main(3) reads 1,2,3 as `((10·acc)+next)` → 123. Previously DECLINED (`no
           enclosing handler here`): the effect-reduction's `callee_def_index_of` followed `Ref` but not
           `Resolved::Member`, so a module-qualified recursive callee was never specialized under the handler
           (the module × recursion × effect-context-mono composition gap). Fixed by following the `Member`
           projection there, mirroring `lower::callee_def_index`. (breaker mo1 witness.)")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module m
              (def (walk (: n Int64) (: acc Int64))
                (if (= n 0) acc (walk (- n 1) (+ (* 10 acc) (Ctr.next unit)))))
              (export walk))
            (def (main (: k Int64))
              (handle Ctr 1
                ((next (u) s (resume s (+ s 1))))
                ((. m walk) k 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 123 Int64)))

(case "a MODULE-exported NON-recursive performer is homed by the importer's handler (single perform)"
  (doc    "The base-case sibling of the recursive module-performer above: `once` is a module-exported
           NON-recursive fn performing `Ctr.next` ONCE, called via `(. m once)` from the importer's handle
           body. This single-perform module case ALREADY worked (a non-recursive module callee inlines into
           the handler context at its one call site) — pinning it guards the module-member call → handler-
           homing path that the recursive fix's `callee_def_index_of` Member arm also serves, so a future
           change there can't silently regress the non-recursive module perform. Seeded 5, main(5) reads 5 →
           100+5 = 105. (breaker mo3 bisect witness.)")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module m
              (def (once (: k Int64)) (+ k (Ctr.next unit)))
              (export once))
            (def (main (: n Int64))
              (handle Ctr n
                ((next (u) s (resume s (+ s 1))))
                ((. m once) 100)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))

(case "MUTUALLY-recursive MODULE-exported performers are both homed by the importer's handler"
  (doc    "The mutual-recursion escalation of the module-performer fix: `ping`/`pong` are TWO module-exported
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
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module m
              (def (ping (: n Int64) (: acc Int64))
                (if (= n 0) acc (pong (- n 1) (+ (* 10 acc) (Ctr.next unit)))))
              (def (pong (: n Int64) (: acc Int64))
                (if (= n 0) acc (ping (- n 1) (+ (* 10 acc) (* 2 (Ctr.next unit))))))
              (export ping) (export pong))
            (def (main (: k Int64))
              (handle Ctr 1
                ((next (u) s (resume s (+ s 1))))
                ((. m ping) k 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 143 Int64)))

(case "a module-exported recursive performer called from a HANDLER ARM homes under the outer handler"
  (doc    "Composition escalation of the module-performer fix (breaker mo4): the module recursive performer
           `(. m walk)` is invoked NOT from the handle body directly but from INSIDE another effect's handler
           ARM — `(handle Ask 0 ((get (u) s (resume ((. m walk) k 0) s))) …)` nested under `(handle Ctr …)`.
           The `Ctr` performs inside `walk` must still home under the OUTER `Ctr` handler even though the
           module call originates in the `Ask` arm's resume expression. Confirms the Member-arm reduction
           reaches a module callee through an arm-nested call site, not just a handle-body one. Seeded 10,
           main(3) sums 10+11+12 = 33.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (effect Ask (op get (-> Unit Int64)))
            (module m
              (def (walk (: n Int64) (: acc Int64))
                (if (= n 0) acc (walk (- n 1) (+ acc (Ctr.next unit)))))
              (export walk))
            (def (main (: k Int64))
              (handle Ctr 10 ((next (u) s (resume s (+ s 1))))
                (handle Ask 0 ((get (u) s (resume ((. m walk) k 0) s)))
                  (Ask.get))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33 Int64)))

(case "TWO modules' recursive performers interleave under ONE handler's shared state"
  (doc    "Cross-module state-continuity escalation (breaker mo5): TWO separate modules `ma`/`mb` each export
           a recursive performer of `Ctr.next`, both entered under ONE `Ctr` handler; the handler's per-run
           state must thread continuously ACROSS the module boundary — `wa`'s activations consume the first
           rows, then `wb`'s consume the next (mb scales its tick ×100). The Member-arm reduction must
           specialize BOTH modules' recursive callees under the same handler and keep one shared cursor. At
           k=2, seeded 1: wa reads 1,2 (→3), wb reads 3,4 ×100 (→700), sum 703.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module ma
              (def (wa (: n Int64) (: acc Int64))
                (if (= n 0) acc (wa (- n 1) (+ acc (Ctr.next unit)))))
              (export wa))
            (module mb
              (def (wb (: n Int64) (: acc Int64))
                (if (= n 0) acc (wb (- n 1) (+ acc (* 100 (Ctr.next unit))))))
              (export wb))
            (def (main (: k Int64))
              (handle Ctr 1 ((next (u) s (resume s (+ s 1))))
                (+ ((. ma wa) k 0) ((. mb wb) k 0))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 703 Int64)))

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

; The performed-scrutinee cases dispatch on the effect's result but keep the arm BODIES pure — so a
; state slot corrupted by the match lowering would go unobserved. These thread state through BOTH
; halves: the scrutinee performs (advancing the state), the match dispatches, and the SELECTED arm's
; body performs again and must read the post-scrutinee state.

(case "performs in match-arm bodies fire ONLY for the selected arm — counter witnesses the count"
  (doc    "The untaken-arm face (the neighbors pin the taken arm's state read): three arms carry ZERO,
           ONE, and TWO performs respectively, and a FINAL perform reads the counter — so the result
           encodes exactly how many arm performs fired. n=0 (one-perform arm): 0 + 10·1 = 10. n=1
           (two-perform arm): (0+1) + 10·2 = 21. n=5 (zero-perform arm): 100 + 10·0 = 100. An emit that
           hoisted an arm's perform above the dispatch (or speculatively evaluated an untaken arm)
           drifts the counter at one of the three calls — the differential-count witness a single-arm
           case cannot give.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Ctr 0
                ((next (u) s (resume s (+ s 1))))
                (+ (match n
                     (0 (Ctr.next unit))
                     (1 (+ (Ctr.next unit) (Ctr.next unit)))
                     (_ 100))
                   (* 10 (Ctr.next unit)))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 10 Int64))
  (call   main (: 1 Int64))
  (output (: 21 Int64))
  (call   main (: 5 Int64))
  (output (: 100 Int64)))

(case "a perform in the scrutinee fires exactly ONCE whichever of three arms is selected"
  (doc    "The once-only guarantee at arm-count 3: the scrutinee's `(Ctr.next unit)` advances the state
           exactly once, the dispatch selects among three arms on its VALUE, and a second perform reads
           the post-scrutinee state — seed 0 → 0 dispatches arm-0, then reads 1 (1001); seed 1 → 2002;
           seed 7 → wildcard, 3008. A dispatch that re-evaluated the scrutinee per arm test (three
           probes = three performs) would read a drifted counter in the tail perform.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Ctr n
                ((next (u) s (resume s (+ s 1))))
                (+ (match (Ctr.next unit)
                     (0 1000)
                     (1 2000)
                     (_ 3000))
                   (Ctr.next unit))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 1001 Int64))
  (call   main (: 1 Int64))
  (output (: 2002 Int64))
  (call   main (: 7 Int64))
  (output (: 3008 Int64)))

(case "a matched arm body performs and reads the state the scrutinee advanced"
  (doc    "Seeded 5, the scrutinee `(Ctr.tick)` reads 5 (state → 6) and hits the literal-5 arm, whose
           BODY performs again: the second tick must read 6 — the state the scrutinee's discharge left —
           not the seed re-read (105) or a per-arm re-seed. 100 + 6 = 106. The arm-body companion of the
           performed-scrutinee case above (whose arms are pure). Expected: 106.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 5 ((tick (u) s (resume s (+ s 1))))
                (match (Ctr.tick)
                  (5 (+ 100 (Ctr.tick)))
                  (_ -1))))
            (export main)))
  (output (: 106 Int64)))

(case "a fall-through arm body performs and reads the post-scrutinee state"
  (doc    "The wildcard twin: seeded 9, the scrutinee reads 9 (state → 10) and MISSES the literal-5 arm;
           the fall-through arm's body performs and reads 10. Pins that the state threads to WHICHEVER
           arm is selected — the dispatch (hit or miss) does not fork or reset the handler state slot.
           Expected: 10.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 9 ((tick (u) s (resume s (+ s 1))))
                (match (Ctr.tick)
                  (5 -1)
                  (_ (Ctr.tick)))))
            (export main)))
  (output (: 10 Int64)))

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

(case "a unit MISMATCH in the resume value is rejected — the arm cannot resume a different unit"
  (doc    "The NEGATIVE twin of the Qty-result pin above: the op declares `(Qty Int64 meter)` but the arm
           resumes a SECOND-typed quantity → CDZ0201. The compile-time unit discipline (units-of-measure's
           no-solver contract) must hold through the resume crossing — a marshalling path that erased the
           unit to a raw scalar at the boundary would let the wrong dimension through silently. The reject
           is at the RESULT position (0201), the resume-side twin of the arg-side reject below.")
  (input  (do
            (effect St (op read (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle St n
                ((read (u) s (resume (Qty.of n (Unit.base #"second")) s)))
                (Qty.value (St.read))))
            (export main)))
  (error  CDZ0201))

(case "a unit MISMATCH in the op argument is rejected — the program cannot perform with a different unit"
  (doc    "The op-ARG direction of the unit-safety pair: the op takes `(Qty Int64 meter)` but the program
           performs with a SECOND-typed quantity → CDZ0203 (the ARGUMENT-position code, vs the resume-side
           0201 above — the same result-vs-arg code split as ordinary typing). Neither effect-boundary
           direction erases units: the dimension is part of the op's contract both ways.")
  (input  (do
            (effect St (op put (-> (Qty Int64 (Unit.base #"meter")) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((put (q) s (resume (Qty.value q) s)))
                (St.put (Qty.of n (Unit.base #"second")))))
            (export main)))
  (error  CDZ0203))

(case "a narrow-width overflow in a handler arm resume value under a narrow annotation is rejected"
  (doc    "The EFFECTS face of the width fit-check: the whole `handle` sits under a `UInt8` annotation, so
           the narrow width must propagate through the handle's result — the op's resume site — into the
           arm's resume VALUE, where the runtime-conditional branch literal `10000` overflows (0..=255) →
           CDZ0302. The resume value is the width descent's longest path yet: annotation → handle body's op
           result → arm resume → runtime `if` branch literal. Without it the overflow would slip into the
           resumed value exactly as the plain-`if` gap did. The fitting twin below computes.")
  (input  (do
            (effect Pick (op get (-> Unit Int64)))
            (def (main (: c Bool))
              (: (handle Pick 0
                   ((get (u) s (resume (if c 10000 5) s)))
                   (Pick.get unit))
                 UInt8))
            (export main)))
  (error  CDZ0302))

(case "a fitting handler arm resume value computes under a narrow annotation"
  (doc    "The no-over-reject control for the effects width face: the same handle shape resuming `(if c 100
           5)` — both branch literals fit UInt8 — computes 100/5 per the runtime condition, at UInt8
           end-to-end. Guards the resume-value width descent against rejecting every narrow-annotated
           handle.")
  (input  (do
            (effect Pick (op get (-> Unit Int64)))
            (def (main (: c Bool))
              (: (handle Pick 0
                   ((get (u) s (resume (if c 100 5) s)))
                   (Pick.get unit))
                 UInt8))
            (export main)))
  (call   main (: true Bool)) (output (: 100 UInt8))
  (call   main (: false Bool)) (output (: 5 UInt8)))

(case "a NARROW-width effect op parameter grounds a fitting perform argument"
  (doc    "An op declared over a NARROW parameter — `Send.put : UInt8 -> Int64` — performed with a fitting
           literal `(Send.put 100)`: the op's declared parameter type grounds the argument (the effect-op
           analogue of the narrow function parameter), the perform crosses to the arm, and the arm resumes
           7 → 7. Pins the narrow-width op-argument path on its FITTING side. (The overflowing twin
           `(Send.put 999)` — expected CDZ0302 like every other narrow-parameter position — currently
           DECLINES rather than rejecting, and an arm that READS the binder `v` also declines: the
           effect-op width descent and the narrow-binder arm read are coverage-not-yet; their pins join
           this one when those land.)")
  (input  (do
            (effect Send (op put (-> UInt8 Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (v) s (resume 7 s)))
                (Send.put 100)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7 Int64)))

(case "a runtime Bool host arg crosses the boundary and drives two response bindings"
  (doc    "The Bool scalar-ARG host-boundary face (v-rust-backend #1708 closed the rust arm: a bool
           marshals to i64 via i64::from, matching wasm's i32 rep). Two host calls each take a runtime
           bool computed from a comparison — `io.check (> n 5)` then `io.check (< n 5)` at n=7 → true then
           false — and each response (10, 20) sums to 30. Pins the bool arg crosses AND that two ops
           consume their rows in order. (breaker bh1, verified past its #1708 witness.) wasm + rust pass;
           rust-async todo pending its host-delegated op-arg path.")
  (input  (do
            (effect io (op check (-> Bool Int64)))
            (def (main (: n Int64))
              (host (io) (+ (io.check (> n 5)) (io.check (< n 5)))))
            (export main)))
  (host-responses (respond io.check (: 10 Int64)) (respond io.check (: 20 Int64)))
  (host-calls (call io.check) (call io.check))
  (call   main (: 7 Int64)) (output (: 30 Int64)))

(case "a Bool host arg BESIDE a scalar and a String composes in one mixed-arity op"
  (doc    "The mixed-arity composition face: one op `(-> Bool Int64 String Int64)` takes a bool, a
           scalar, AND a string arg together — the Bool marshal (#1708) composing with the existing
           scalar and String-arg arms in a single arg list (the multi-arg slot-threading the
           host-arg-before-scalar fix hardened). `io.log (= n 3) n \"tag\"` at n=3 → host answers 42.
           (breaker bh2, verified.) wasm + rust pass; rust-async todo.")
  (input  (do
            (effect io (op log (-> Bool Int64 String Int64)))
            (def (main (: n Int64))
              (host (io) (io.log (= n 3) n "tag")))
            (export main)))
  (host-responses (respond io.log (: 42 Int64)))
  (host-calls (call io.log))
  (call   main (: 3 Int64)) (output (: 42 Int64)))

(case "a RECURSIVE-sum value of runtime depth rides a handler resume"
  (doc    "An op whose declared result is a RECURSIVE sum (`Give.get : Unit -> Nat`) resumed with a
           runtime-depth spine `(mk a)`: the resume value is an unbounded heap structure, not a scalar or
           fixed-shape compound, and the body folds it back to its depth — 3 at `a = 3`, 0 at `a = 0`.
           Pins that the resume path carries a recursive sum intact through the handler machinery (the
           unbounded-depth companion of the Qty/Result resume-value cases).")
  (input  (do
            (type Nat (Z) (S Nat))
            (effect Give (op get (-> Unit Nat)))
            (def (mk (: n Int64)) (if (= n 0) (Z) (S (mk (- n 1)))))
            (def (depth (: v Nat))
              (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
            (def (main (: a Int64))
              (depth (handle Give 0
                ((get (u) s (resume (mk a) s)))
                (Give.get unit))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

(case "a handler STATE that is a recursive sum GROWS one constructor per operation"
  (doc    "The recursive-sum face of heap-valued handler state (list/record/set/string states are pinned;
           this state's SHAPE deepens per op): seeded `(Z)`, each `Acc.bump` arm resumes with next-state
           `(S s)` — wrapping the CURRENT state one level deeper — and `Acc.read` folds the accumulated
           spine to its depth. Two bumps then a read → 2. Pins that the threaded state may be a recursive
           sum whose depth is the operation COUNT (state evolution changes the value's structure, not just
           its contents), composing the state-threading discipline with unbounded recursive values.")
  (input  (do
            (type Nat (Z) (S Nat))
            (effect Acc (op bump (-> Unit Int64)) (op read (-> Unit Int64)))
            (def (depth (: v Nat))
              (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
            (def (main (: a Int64))
              (handle Acc (Z)
                ((bump (u) s (resume 0 (S s)))
                 (read (u) s (resume (depth s) s)))
                (do (Acc.bump unit) (Acc.bump unit) (Acc.read unit))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

(case "a LET-bound outer perform under an inner handle threads its state advance"
  (doc    "An OUTER-handled effect performed INSIDE an inner (different-effect) handle, with the perform's
           value LET-BOUND before the next operation: `A.bump` (threads 0→1) let-bound, then `A.get` reads
           1 — the state advance crosses the inner `B` handler level intact. Pins the cross-level state
           threading for the value-consumed sequencing form. (The DO-discarded twin of this shape — `(do
           (A.bump unit) (A.get unit))` under the inner handle — currently DROPS the advance, a filed
           lowering bug; when it is fixed its case joins this one, and this pin guards the form that must
           keep working.)")
  (input  (do
            (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op noop (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((bump (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 100
                  ((noop (u) t (resume t t)))
                  (let ((x (A.bump unit)))
                    (A.get unit)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "a do-sequenced perform train under its OWN inner handle threads state"
  (doc    "The inner-effect control for the cross-level threading: the do-sequenced bump/get train targets
           the INNER handler itself (`B`, seeded 100) while an outer `A` handler wraps it — `B.bump`
           (100→101 discarded) then `B.get` reads 101. Pins that do-sequencing is sound when the performs
           discharge at the NEAREST handler; combined with the let-bound case above it brackets the filed
           do-discarded CROSS-level state-drop precisely (same sequencing one level down: works; same
           crossing with let: works; do + crossing: the bug).")
  (input  (do
            (effect A (op noop (-> Unit Int64)))
            (effect B (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((noop (u) s (resume s s)))
                (handle B 100
                  ((bump (u) t (resume 0 (+ t 1)))
                   (get (u) t (resume t t)))
                  (do (B.bump unit) (B.get unit)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 101 Int64)))

(case "a do-discarded OUTER perform under an inner handle threads its state advance across the level"
  (doc    "The FIXED cross-level case the two cases above bracket: a do-sequenced perform of an OUTER-handled
           effect, its value DISCARDED in a `(do …)` and crossing an INNER handler of a DIFFERENT effect,
           threads its state advance out to the outer handler. `A.bump` (0→1) is do-discarded under the
           inner `B` handle, then `A.get` reads 1 — NOT the stale seed 0. This was a silent wrong-value
           miscompile on all backends (`thread_bounded`'s `do` fold collapsed the sequence to only the last
           item, erasing the non-final FOREIGN perform the inner handler does not discharge); the fix
           preserves a non-final item still reaching a foreign perform. Completes the bracket: sequencing one
           level down works (train case), crossing with a let works (let-bound case), and NOW do + crossing
           works too — the discarded-value form is no longer a state-drop.")
  (input  (do
            (effect A (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op noop (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((bump (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 100
                  ((noop (u) t (resume t t)))
                  (do (A.bump unit) (A.get unit)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "interleaved do-discarded performs at BOTH nest levels thread their own states independently"
  (doc    "The composition the single-effect fix case above doesn't reach: TWO counters at different nest
           levels, both advanced by DO-DISCARDED performs, interleaved in one `(do …)` — outer, inner,
           outer again — then read via a final `(+ outer-get inner-get)`. CountA (outer, seed 0, +1 per
           bump) is bumped twice; CountB (inner, seed 100, +10 per bump) once; expected 2 + 110 = 112.
           Each discarded perform crosses (or doesn't) the inner handler per ITS effect: the A bumps are
           foreign to the inner handle and must survive its do-fold, the B bump is discharged locally, and
           neither may clobber the other's threaded slot. The fixed collapse dropped exactly this class of
           non-final foreign perform; a partial fix that preserved only ONE crossing (or merged the two
           state slots) lands off by a bump-width at one counter.")
  (input  (do
            (effect CountA (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect CountB (op bump (-> Unit Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle CountA 0
                ((bump (u) s (resume 0 (+ s 1)))
                 (get (u) s (resume s s)))
                (handle CountB 100
                  ((bump (u) t (resume 0 (+ t 10)))
                   (get (u) t (resume t t)))
                  (do (CountA.bump unit)
                      (CountB.bump unit)
                      (CountA.bump unit)
                      (+ (CountA.get unit) (CountB.get unit))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 112 Int64)))

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
(case "a discarded pure trapping item in a host do-body is elided beside a host call (dead-init ruling)"
  (doc    "`(host (io) (do (/ 100 d) (io.put 1) 42))` at d = 0: the non-final `(/ 100 d)` is a discarded
           PURE item — its value flows nowhere, so per the dead-init ruling it is unobserved and its
           divide-by-zero trap does NOT fire; the do makes a host call (`io.put 1`) and yields its last
           form, 42. The foreign-perform exception preserves the PERFORM (`io.put` still runs, host-call
           recorded), not the pure discarded sibling. Pins that the Core::Seq emit ELIDES a discarded pure
           non-final statement rather than force-evaluating it (adv-56 rust miscompile — `let _ = <stmt>;`
           ran the trap). The pure-only dead-init twin (02-binding-and-control) elides the same way with no
           host call; this is the host-call face. Rust + wasm pass (the wasm Core::Seq emit elides a
           non-host-reaching statement via the SAME `subtree_reaches_host_call` predicate CDZ0307 warns on);
           rust-async declines this host-delegated shape (todo).")
  (input  (do
            (effect io (op put (-> Int64 Int64)))
            (def (main (: d Int64))
              (host (io)
                (do (/ 100 d)
                    (io.put 1)
                    42)))
            (export main)))
  (host-responses (respond io.put (: 0 Int64)))
  (host-calls (call io.put))
  (call   main (: 0 Int64)) (output (: 42 Int64)))

(case "a value-leaving host-call statement in a do-body runs and its result is dropped (dead-init sibling)"
  (doc    "The DROP face of the dead-init Core::Seq emit: `(do (io.put 1) (io.put 2) 42)` — the two non-final
           statements are VALUE-LEAVING host calls (`io.put : Int64 -> Int64`), not Unit. Each must RUN (both
           host calls fire, recorded in order) but its returned value is DISCARDED — the emit drops the
           leftover so the block stays stack-balanced and yields the tail, 42. Distinct from the pure-elide
           sibling above (which does NOT emit its statement at all): a host-reaching statement is always
           emitted; only a non-Unit RESULT is dropped. Pins the `Lir::Drop` arm the sibling's pure-elide path
           doesn't exercise. Rust + wasm pass; rust-async todo pending its host-delegated Seq emit.")
  (input  (do
            (effect io (op put (-> Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (do (io.put 1)
                    (io.put 2)
                    42)))
            (export main)))
  (host-responses (respond io.put (: 0 Int64)) (respond io.put (: 0 Int64)))
  (host-calls (call io.put) (call io.put))
  (call   main (: 0 Int64)) (output (: 42 Int64)))

(case "a NOMINAL-Unit-typed host-call statement in a do-body is not spuriously dropped"
  (doc    "The nominal-Unit edge of the Seq stmt-DROP arm (the sibling above): `(Done (io.fire k))` is a
           non-final statement that REACHES a host call AND is typed `Done` — a NEWTYPE over `Unit`
           (`(type Done (Done Unit))`). A nominal-Unit leaves NO machine value at the boundary just like a
           bare `Unit` (`valtype_of` is None), so it must NOT be dropped. The drop test must strip nominals
           (`type_of(..).strip_nominal() != Unit`) — WITHOUT that, `Done` ≠ `Ty::Unit` takes the drop branch
           and `Lir::Drop` underflows the empty stack → an invalid module (`wasm-tools: expected a type but
           nothing on stack`). io.fire still fires (recorded), the do yields io.get's response, 9. Mirrors
           the field-proj / tail-drop Unit checks that already strip_nominal. Rust + wasm pass; rust-async
           todo.")
  (input  (do
            (type Done (Done Unit))
            (effect io (op fire (-> Int64 Unit)) (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (io)
                (do (Done (io.fire k))
                    (io.get unit))))
            (export main)))
  (host-responses (respond io.fire (: 0 Int64)) (respond io.get (: 9 Int64)))
  (host-calls (call io.fire) (call io.get))
  (call   main (: 5 Int64)) (output (: 9 Int64)))

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

(case "an ABORTIVE arm whose value is a COMPOUND matches the handle body type and folds"
  (doc    "The compound-valued abortive arm (the tuple companion of the scalar abort cases): an operation
           whose declared RESULT is a `(Tuple Int64 Int64)` is handled by an ABORTIVE arm (no `resume`) that
           yields a tuple — `(bail (n) s (tuple n n))`. The whole handle body IS the perform `(Bail.bail 7)`,
           so the arm value becomes the handle value: `(7, 7)`. This exercises the abortive type-consistency
           guard on the SOUND side — the arm body type `(Tuple Int64 Int64)` equals the op result type AND
           the handle body type, so it folds (a mismatch would decline, guarding the compound-body-abort
           miscompile where a scalar abort value disagreed with a compound position). Pins that a
           compound-valued abort matching its declared type folds rather than over-declining.")
  (input  (do
            (effect Bail (op bail (-> Int64 (Tuple Int64 Int64))))
            (def (main)
              (handle Bail (tuple 0 0) ((bail (n) s (tuple n n)))
                (Bail.bail 7))) (export main)))
  (output (: (tuple 7 7) (Tuple Int64 Int64))))

(case "an abortive arm yields a heap LIST built in the arm as the handle's value"
  (doc    "The heap-collection abort (the tuple abort above is a fixed-shape compound; this arm BUILDS an
           RRB list): `(stop (v) s (list v v v))` never resumes — the 3-element list constructed in the arm
           becomes the handle's value, abandoning the body's continuation (`(list 1)` never evaluates). The
           abort's `br` must carry a live heap HANDLE out of the handler block (not a scalar), and the
           abandoned continuation's pending values must not corrupt it. `List.len` reads 3.")
  (input  (do
            (effect Halt (op stop (-> Int64 (List Int64))))
            (def (main (: n Int64))
              (List.len (handle Halt 0
                ((stop (v) s (list v v v)))
                (do (Halt.stop n) (list 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64)))

(case "an abortive arm that READS a heap-typed (Map) handler state folds — seed-let-lift on the abort path"
  (doc    "breaker heap-abort-state. A HEAP-typed handler state (`Map.empty`) is not a shareable constant, so
           `reduce_handle` let-binds the seed to a fresh `#seed` and threads THAT (each state splice is a
           `#seed` ref). An ABORT arm (no resume) whose expression READS the state binder — `(halt (u) s (*
           1000 (+ (Map.len s) a)))` — carries `#seed` refs in the collapsed abort value. Before the fix the
           abort-collapse return path did NOT re-wrap the value in the `(let ((#seed Map.empty)) …)` (only the
           resumptive return did), so `#seed` read UNBOUND → CDZ0101 on a valid program. Fixed by applying the
           same seed-let-lift on the abortive returns. Seeded `Map.empty`, called `(main 2)`: the abort reads
           `(Map.len Map.empty)` = 0, so `(* 1000 (+ 0 2))` = 2000. Pins that a heap-state read in an abort arm
           folds (scalar-state reads already folded — a scalar seed is a shareable constant with no `#seed`;
           heap-state CONSTANT-answer abort arms already folded — no `#seed` ref survives).")
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St Map.empty
                ((halt (u) s (* 1000 (+ (Map.len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2000 Int64)))

(case "an abortive arm that READS a heap-typed (List) handler state folds — the List face"
  (doc    "The List face of the heap-abort-state fix above (breaker sk2g): same shape with a `(list)` seed and
           `(List.len s)` in the abort arm. `(list)` is a heap seed → `#seed` let-bound → the abort value's
           `#seed` ref is wrapped by the seed-let-lift on the abort path. `(main 2)`: `(List.len (list))` = 0
           → `(* 1000 (+ 0 2))` = 2000. Confirms the fix is state-shape-agnostic (Map + List).")
  (input  (do
            (effect St (op halt (-> Unit Int64)))
            (def (main (: a Int64))
              (handle St (list)
                ((halt (u) s (* 1000 (+ (List.len s) a))))
                (St.halt)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2000 Int64)))

(case "an abortive arm yields a RECURSIVE-SUM spine as the handle's value"
  (doc    "The unbounded-shape abort: the arm yields `(S (S (Z)))` — a recursive-sum spine — and the
           abandoned body would have produced the different-depth `(Z)`. The abort path must carry the
           multi-node heap structure out intact; the fold reads depth 2 (a corrupted or body-value handle
           would read 0). With the list case above, pins that the abortive `br` carries every heap value
           class, completing the abort-value matrix (scalar/runtime-scalar/tuple/list/recursive-sum).")
  (input  (do
            (type Nat (Z) (S Nat))
            (effect Halt (op stop (-> Int64 Nat)))
            (def (depth (: v Nat))
              (match v ((S rest) (+ 1 (depth rest))) ((Z u) 0)))
            (def (main (: n Int64))
              (depth (handle Halt 0
                ((stop (v) s (S (S (Z)))))
                (do (Halt.stop n) (Z)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64)))

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

(case "a BLOCK-wrapped branch perform in a let-init declines cleanly (adv-69 safe floor, not a silent miscompile)"
  (doc    "adv-69 (HIGH, breaker+corpus-bugfix): a branch-performing conditional wrapped in a BLOCK inside a
           `let`-init — `(let ((v (let ((b true)) (if b (St.get) 99)))) (+ (* 10 v) (St.get)))` — DROPPED the
           branch perform's state advance at the block boundary: the trailing `(St.get)` resumed the block-
           ENTRY state, not the branch's out-state. Seeded 3 it produced 33 (= 10*3 + 3) on wasm AND rust,
           where the correct value is 34 (= 10*3 + 4). The hoist's Site 4 lifts a conditional that is DIRECTLY
           a `let`-init to tail position (per-branch threading carries the advance), but a conditional behind
           a `let`/`do` block wrapper is opaque to it. Until the full through-block distribution lands (an
           alpha-safe commuting conversion, a separate increment), reduce_handle DECLINES this residual shape
           → a clean Todo (honest 'not yet reducible'), NEVER the silent 33. This case grades TODO on all
           backends (the safe floor); its 34 becomes a PASS when the full fold lands. Distinct from the FIXED
           direct-init/connective-scrutinee cases above (those still compute — see the control below).")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b true)) (if b (St.get) 99))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a DIRECT-init branch perform in a let-init threads its state advance (adv-69 control, still computes)"
  (doc    "The working control for adv-69's safe-decline: the SAME body but with the branch-performing
           conditional DIRECTLY the `let`-init (no block wrapper) — `(let ((v (if true (St.get) 99))) (+ (* 10
           v) (St.get)))`. Hoist Site 4 lifts it to tail position, so each branch threads the perform's advance
           through the continuation: seeded 3, v=3 (first get, state→4), trailing get reads 4 → 10*3 + 4 = 34.
           Pins that the adv-69 safe-decline floor does NOT over-decline the direct-init path the hoist already
           handles correctly. Computes on all backends.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (let ((v (if true (St.get) 99)))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main) (output (: 34 Int64)))

; Three more faces of the adv-69 safe-decline floor (all grade TODO — decline cleanly, flip to a 34/11 PASS
; when the through-block commuting-conversion fold lands): the floor's block_wrapped_branch_performs guard
; peels DEPTH-N pure let/do wrappers and covers a HEAP-accumulator perform in the same let-init position.
; The arm-resume-value positional sub-face (a3 — a block-wrapped OUTER-effect perform inside an INNER
; handle's resume-value) is now ALSO declined by a targeted `Resume{value}`-keyed guard (its case below).

(case "a DEPTH-2 block-wrapped branch perform in a let-init declines cleanly (adv-69 safe floor, nested wrappers)"
  (doc    "The depth-2 face of the adv-69 safe-decline: the branch-performing conditional sits behind TWO
           nested `let` wrappers in the init — `(let ((v (let ((b true)) (let ((c true)) (if (and b c) (St.get)
           99))))) …)`. The floor's block_wrapped_branch_performs peel recurses through depth-N pure let/do
           wrappers, so this declines cleanly (TODO) exactly as the depth-1 witness does — NOT the silent 33
           state-drop. Pins that the trigger is ANY block nesting ≥1, not a single-wrapper shape. Flips to the
           34 PASS when the through-block fold lands. Declines on all backends (shared lowering).")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b true)) (let ((c true)) (if (and b c) (St.get) 99)))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a HEAP-accumulator block-wrapped branch perform in a let-init declines cleanly (adv-69 safe floor)"
  (doc    "The heap-state face: the block-wrapped branch performs a HEAP-accumulating effect — a `Log.add`
           that `List.push`es onto the handler's list state — in the let-init `(let ((v (let ((b true)) (if b
           (Log.add 5) 99)))) …)`. Under the floor this declines cleanly (TODO) rather than DROPPING the push
           (the silent-miscompile would lose the entry: `count` would read the entry list, e.g. length 0 not
           1) — the data-loss face of the block-boundary out-state drop, so the safe decline matters more here
           than for a stale scalar. Flips to the 11 PASS (10*1 + 1, the pushed entry counted twice) when the
           through-block fold lands. Declines on all backends.")
  (input  (do
            (effect Log (op add (-> Int64 Unit)) (op count (-> Unit Int64)))
            (def (main)
              (handle Log (list)
                ((add (v) s (resume unit (List.push s v)))
                 (count (u) s (resume (List.len s) s)))
                (let ((v (let ((b true)) (if b (Log.add 5) 99))))
                  (+ (* 10 (Log.count)) (Log.count)))))
            (export main)))
  (call   main) (output (: 11 Int64)))

(case "a BLOCK-wrapped branch perform in a NESTED handler-arm resume-value declines cleanly (adv-69 a3 sub-face)"
  (doc    "adv-69 a3 (breaker probe-a3, block-outstate battery): the SAME block-boundary out-state drop as the
           let-init floor above, but at a DIFFERENT position — a block-wrapped branch-performing conditional in
           a NESTED handler's arm RESUME-VALUE, performing the OUTER handler's op. The outer `St` handler threads
           its state through the inner `Up` handle, but the block boundary inside the inner arm's resume-VALUE
           `(resume (let ((b true)) (if b (St.get) 99)) t)` dropped the outer `St.get`'s advance: seeded 3 it ran
           33, correct is 34 (= 10*(St.get resumes 3, state→4 seen by trailing get) ... trailing `(St.get)` reads
           4). The let-init scanner stops at a nested `handle` (an inner handle's lets are its own reduction), so
           this position escaped that floor. A targeted guard keyed PRECISELY on the `Resume{value}` position
           (not a position-agnostic block-wrapped-perform scan, which over-declines working threaded positions)
           declines this residual shape → a clean Todo, never the silent 33. Grades TODO on all backends; its 34
           becomes a PASS when the full through-block fold lands (same deferred commuting conversion as the
           let-init face).")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (effect Up (op ask (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (handle Up 0
                  ((ask (u) t (resume (let ((b true)) (if b (St.get) 99)) t)))
                  (+ (* 10 (Up.ask)) (St.get)))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a DIRECT branch perform in a NESTED handler-arm resume-value declines cleanly (adv-69 a3-direct sub-face)"
  (doc    "The DIRECT-conditional twin of the a3 case: `(resume (if true (St.get) 99) t)` — a branch-performing
           conditional DIRECTLY (no block wrapper) in a nested handler's arm resume-value, performing the OUTER
           op. Unlike the let-init face — where a DIRECT init is lifted by Site 4 and folds — a `resume`-value
           is never hoisted (it lives inside the inner `Up` handle's arm, which the outer `St` reduction does
           not rewrite), so the direct conditional here ALSO drops the outer `St.get`'s advance: seeded 3 it ran
           33, correct is 34. The a3 guard's `Resume{value}` scanner declines this via its direct-conditional
           disjunct (verified: dropping that disjunct makes this miscompile to 33, not fold to 34 — so the
           disjunct is load-bearing, not an over-decline). Pins that the resume-value drop is NOT block-wrapper-
           specific (contrast the let-init face). Grades TODO on all backends; flips to 34 PASS on the
           through-block fold.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (effect Up (op ask (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (handle Up 0
                  ((ask (u) t (resume (if true (St.get) 99) t)))
                  (+ (* 10 (Up.ask)) (St.get)))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a block-wrapped branch perform of the OUTER effect in a let-init INSIDE a nested handler body declines cleanly (adv-69 a4 sub-face)"
  (doc    "adv-69 a4 (v-effects self-probe 2026-08-04): the let-init block-boundary drop, but the miscompiling
           `let` sits inside a NESTED inner handler's BODY and performs the OUTER effect. `(handle A 3 ((ga …))
           (handle B 100 ((gb …)) (let ((v (let ((k true)) (if k (A.ga) 9)))) (+ (* 10 v) (A.ga)))))` — the
           block-wrapped branch perform is of the OUTER `A`, in a `let`-init in the inner `B` handle's body, and
           the continuation RE-READS `A`. Seeded A=3 it ran 33, correct is 34 (A.ga returns 3, advances to 4;
           trailing A.ga must read 4). The single-handle version of this shape declines via the let-init floor,
           but the intervening nested `B` handle made the OUTER `A` reduction's scanner stop at the inner
           `Handle` and MISS the block-wrapped `A`-perform in `B`'s body — a silent miscompile. FIX: the scanner
           now descends into a nested handle's BODY (not its arms — that is a3's territory) keeping the OUTER
           ctx, so `block_wrapped_branch_performs` (ctx-keyed) fires only on an OUTER-effect perform (an inner
           `B`-effect perform never matches → no over-decline of `B`'s own shapes). Grades TODO on all backends;
           flips to 34 PASS on the through-block fold.")
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B 100 ((gb (u) t (resume t t)))
                  (let ((v (let ((k true)) (if k (A.ga) 9))))
                    (+ (* 10 v) (A.ga))))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a block-wrapped OUTER-effect perform in a let-init THREE handlers deep declines cleanly (adv-69 a4-depth3 sub-face)"
  (doc    "adv-69 a4 at DEPTH-3 (breaker nh5 escalation, block-outstate battery): the a4 nested-handle-body
           drop, but the block-wrapped OUTER-effect (`A`) perform sits in a `let`-init THREE handlers deep —
           `(handle A 3 (…) (handle B 100 (…) (handle C 200 (…) (let ((v (let ((k true)) (if k (A.ga) 9))))
           (+ (* 10 v) (A.ga))))))`. Seeded A=3 it ran 33, correct is 34. Pins that the a4 scanner's descent
           into a nested handle's BODY is RECURSIVE, not one-level: the outer `A` reduction descends through
           BOTH the `B` and `C` handle bodies (keeping the outer ctx) to reach the block-wrapped `A`-perform.
           If the descent peeled only one `Handle`, this depth-3 shape would escape and miscompile — so this
           locks in the depth-N property (analogous to the a2 depth-2 witness for the flat let-init floor).
           Grades TODO on all backends; flips to 34 PASS on the through-block fold.")
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (effect C (op gc (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B 100 ((gb (u) t (resume t t)))
                  (handle C 200 ((gc (u) w (resume w w)))
                    (let ((v (let ((k true)) (if k (A.ga) 9))))
                      (+ (* 10 v) (A.ga)))))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a block-wrapped OUTER-effect perform in a nested handle's INIT declines cleanly (adv-69 a4-init sub-face)"
  (doc    "adv-69 a4-init (liaison/Copilot on merged #1933): the a4 nested-handle-escape, but the block-wrapped
           OUTER-effect perform sits in the inner handle's INIT — `(handle A 3 ((ga …)) (handle B (let ((k true))
           (if k (A.ga) 9)) ((gb …)) (+ (* 10 (B.gb)) (A.ga))))`. The inner `B` handle's INIT is evaluated as
           part of the handle expression in the OUTER `A` extent (eval.rs passes `init` to `reduce_handle`
           alongside `body`), so a block-wrapped `A`-perform there drops the outer advance exactly like the a4
           body face: seeded A=3 it ran 33, correct is 34 (B's init A.ga returns 3, A→4; B.gb returns B-state 3;
           trailing A.ga must read 4 → 10*3 + 4). The a4 fix scanned the inner handle's BODY but early-returned
           without the INIT, missing this position. FIX: the nested-Handle scan checks the init node directly
           (`block_wrapped_branch_performs`) AND recurses into both init and body — ctx-keyed, so only an
           OUTER-op perform fires (no over-decline of `B`'s shapes). Grades TODO on all backends; flips to 34
           PASS on the through-block fold.")
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B (let ((k true)) (if k (A.ga) 9))
                  ((gb (u) t (resume t t)))
                  (+ (* 10 (B.gb)) (A.ga)))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a BLOCK-wrapped branch perform in a MATCH-SCRUTINEE declines cleanly (adv-69 g3 sub-face)"
  (doc    "adv-69 g3 (breaker probe-g3, block-outstate battery): the SAME block-boundary out-state drop, at a
           MATCH-SCRUTINEE consuming position. `(match (let ((b true)) (if b (St.get) 99)) (v (+ (* 10 v)
           (St.get))))` — the scrutinee is a block-wrapped branch-performing conditional. Site 5 lifts a
           scrutinee that is DIRECTLY a branch-performing conditional (per-branch threading carries its
           advance), but a block wrapper is opaque to it, so the scrutinee's out-state reverts to entry: seeded
           3 it ran 33, correct is 34 (v=3, state→4, trailing `(St.get)` reads 4). Keyed on the WRAPPED shape
           only (a DIRECT `if`/`match` scrutinee still folds — no over-decline of the Site-5 path). Declines
           cleanly → a clean Todo, never the silent 33; flips to 34 PASS on the through-block fold.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (match (let ((b true)) (if b (St.get) 99))
                  (v (+ (* 10 v) (St.get))))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a BLOCK-wrapped branch perform in a non-tail DO-STATEMENT declines cleanly (adv-69 c3 sub-face)"
  (doc    "adv-69 c3 (breaker probe-c3, block-outstate battery): the SAME block-boundary out-state drop, at a
           non-tail `do`-STATEMENT position. `(do (let ((x true)) (if x (St.put 7) unit)) (+ (* 10 (St.get))
           x))` — a block-wrapped branch perform as a DISCARDED (non-last) `do` item. Site 1 hoists a non-last
           item that is DIRECTLY a branch-performing conditional (distributing the continuation into each
           branch), but a block wrapper defeats its match, so the statement's `St.put 7` advance is dropped:
           seeded 3 the trailing `(St.get)` reads the stale pre-statement state → ran 33, correct is 73 (put
           sets state 7, `(St.get)` resumes 7 → 10*7 + shadowed-outer x=3 = 73). The minimal twins d2/e1 — a
           BARE `if` in the statement, or a def-bound cond — hoist fine and PASS, so this keys on the block
           wrapper. Declines cleanly → a clean Todo, never the silent 33; flips to 73 PASS on the through-block
           fold.")
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (do
                  (let ((x true)) (if x (St.put 7) unit))
                  (+ (* 10 (St.get)) x))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))

(case "a block-wrapped OUTER-effect perform in a non-tail do-statement INSIDE a nested handler body declines cleanly (adv-69 c3-nested sub-face)"
  (doc    "adv-69 c3-nested (v-effects self-probe 2026-08-04): the c3 non-tail do-statement drop, but the
           block-wrapped branch perform is of the OUTER effect and sits in a `do`-statement INSIDE a nested
           inner handler's body. `(handle A x ((ga …)(pa …)) (handle B 100 ((gb …)) (do (let ((k true)) (if k
           (A.pa 7) unit)) (+ (* 10 (A.ga)) x))))` — the discarded statement's `A.pa 7` advance drops at the
           block boundary: seeded 3 it ran 33, correct is 73 (pa sets state 7, `(A.ga)` reads 7 → 10*7 + x=3).
           Same nested-handle-escape class as the a4 let-init face, but for the do-statement scanner: the outer
           `A` reduction's `body_has_block_wrapped_scrutinee_or_statement_branch_perform` scan STOPPED at the
           nested `B` `Handle` and missed the block-wrapped `A`-perform in `B`'s body. FIX: that scanner now
           descends into a nested handle's BODY (not arms) keeping the OUTER ctx — ctx-keyed so only an
           outer-effect perform fires (no over-decline of `B`'s own shapes). Grades TODO on all backends; flips
           to 73 PASS on the through-block fold.")
  (input  (do
            (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
            (effect B (op gb (-> Unit Int64)))
            (def (main (: x Int64))
              (handle A x ((ga (u) s (resume s s)) (pa (v) _s (resume unit v)))
                (handle B 100 ((gb (u) t (resume t t)))
                  (do (let ((k true)) (if k (A.pa 7) unit))
                      (+ (* 10 (A.ga)) x)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))

(case "a block-wrapped OUTER-effect perform in a do-statement TWO nested handlers deep declines cleanly (adv-69 nh7 depth-3)"
  (doc    "adv-69 nh7 (breaker depth escalation of c3-nested): the SAME non-tail do-statement drop, but the
           outer `A`-perform sits inside TWO stacked nested handlers (`B` then `C`) — `(handle A x (…) (handle
           B 100 (…) (handle C 200 (…) (do (let ((k true)) (if k (A.pa 7) unit)) (+ (* 10 (A.ga)) x)))))`. The
           block-wrapped `A.pa 7` advance drops at the block boundary: seeded 3 it ran 33, correct is 73 (pa
           sets state 7, `(A.ga)` reads 7 → 10*7 + x=3). Verifies the c3-nested scanner's nested-handle-body
           descent is RECURSIVE — it re-invokes on EACH nested body, so the depth-2 nesting (A over B over C)
           is covered exactly like depth-1, the depth-N regression guard analogous to a4-depth3 for the let-
           init scanner. Grades TODO on all backends; flips to 73 PASS on the through-block fold.")
  (input  (do
            (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
            (effect B (op gb (-> Unit Int64)))
            (effect C (op gc (-> Unit Int64)))
            (def (main (: x Int64))
              (handle A x ((ga (u) s (resume s (+ s 1))) (pa (v) _s (resume unit v)))
                (handle B 100 ((gb (u) t (resume t t)))
                  (handle C 200 ((gc (u) w (resume w w)))
                    (do (let ((k true)) (if k (A.pa 7) unit))
                        (+ (* 10 (A.ga)) x))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))

(case "a block-wrapped OUTER-effect perform in a match-SCRUTINEE inside a nested handler body declines cleanly (adv-69 g3-nested)"
  (doc    "adv-69 g3-nested (v-effects bonus-probe of the c3-nested fix): the g3 match-scrutinee face of the
           nested-handle-body escape. A block-wrapped OUTER `A`-perform sits in a `match` SCRUTINEE inside a
           nested `B` handler's body — `(handle A 3 ((ga …)) (handle B 100 ((gb …)) (match (let ((k true)) (if
           k (A.ga) 9)) (v (+ (* 10 v) (A.ga))))))`. The block-wrapped branch perform's advance drops at the
           block boundary: seeded 3 it ran 33, correct is 34 (the scrutinee `A.ga` reads 3 and advances state
           to 4, so v = 3; the arm's trailing `(A.ga)` reads the advanced 4 → 10*3 + 4 = 34). The g3/c3 scanner
           (`body_has_block_wrapped_scrutinee_or_statement_branch_perform`) shares
           the do-statement scanner's nested-handle-body descent, so the match-scrutinee position in a nested
           body is covered by the same fix. Grades TODO on all backends; flips to 34 PASS on the through-block
           fold.")
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B 100 ((gb (u) t (resume t t)))
                  (match (let ((k true)) (if k (A.ga) 9))
                    (v (+ (* 10 v) (A.ga)))))))
            (export main)))
  (call   main) (output (: 34 Int64)))

(case "a block-wrapped conditional whose CONDITION performs (not a branch) folds correctly (adv-69 boundary control)"
  (doc    "The passing boundary control for the adv-69 guards: a block-wrapped conditional whose CONDITION
           performs — `(let ((v (let ((b (> (St.get) 0))) (if b 7 99)))) (+ (* 10 v) (St.get)))` — must still
           FOLD, not decline. Unlike the adv-69 faces (where a BRANCH performs, so the advance is branch-local
           and drops at the block boundary), here the perform is a pure `let`-binding on the block's STRICT
           SPINE: `(St.get)` runs unconditionally as `b`'s init, advancing the state once (seeded 3 → 4), and
           the `if`'s branches (7 / 99) perform nothing. So the block's out-state IS the threaded post-perform
           state — no drop. v = 7 (b = 3>0 = true), trailing `(St.get)` reads the advanced 4 → 10*7 + 4 = 74.
           Pins that the adv-69 decline-guards (`block_wrapped_branch_performs` et al.) key on a BRANCH perform,
           NOT any perform inside a block — a condition/spine perform is correctly threaded and folds. Computes
           on all backends.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main)
              (handle St 3 ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b (> (St.get) 0))) (if b 7 99))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main) (output (: 74 Int64)))

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

(case "a CROSS-handler op whose inline ARG performs an OUTER handler's op folds (op-arg let-lift)"
  (doc    "The cross-handler analogue of the same-handler nested-perform-arg case above (Acc.step (Acc.step 1)):
           a NESTED handler's op whose INLINE argument performs an OUTER handler's op — `(B.put (A.get))` under
           `handle A (handle B …)`, where `A.get` homes to the enclosing `A` (foreign to `B`). B's arm uses its
           param `v` TWICE (`(resume (+ s v) (+ s v))`), so substituting the performing `(A.get)` inline would
           duplicate it (effect-duplication guard). Fixed by the op-arg LET-LIFT: bind the foreign-perform arg
           to a fresh `#cv` once, then B's arm reads the pure ref twice — exactly the WORKING let-bound spelling
           `(let ((x (A.get))) (B.put x))`. `A.get`=7 (no advance), `B.put(7)` = `0+7` = 7. Pins that an inline
           cross-handler op-arg-performs-outer folds (was a clean decline — the inline-arg-position completeness
           gap; the let-bound spelling always folded).")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (main)
              (handle A 7 ((get (u) s (resume s s)))
                (handle B 0 ((put (v) s (resume (+ s v) (+ s v))))
                  (B.put (A.get)))))
            (export main)))
  (output (: 7 Int64)))

(case "a cross-handler op-arg performing an outer effect runs EXACTLY ONCE though the arm uses it thrice"
  (doc    "The soundness control for the op-arg let-lift: the foreign-perform arg must run ONCE (an op arg is
           evaluated once, before the call, regardless of how many times the arm reads its param). `(B.put
           (A.tick))` where `A.tick` ADVANCES the outer A-state, and B's arm reads `v` THREE times
           (`(resume (+ (+ v v) v) s)`). If the lift wrongly duplicated the perform, A would advance 3× and the
           reads would differ; correctly it advances ONCE (10→11), all three reads see 10 → `v+v+v` = 30, then
           the outer `(A.get)` reads the once-advanced 11 → `(+ 30 11)` = 41. Pins that the `#cv` let-bind
           runs the foreign perform exactly once and the arm reads the pure ref — no effect duplication.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((put (v) s (resume (+ (+ v v) v) s))) (B.put (A.tick)))))
                  (+ b (A.get)))))
            (export main)))
  (output (: 41 Int64)))

(case "TWO outer op-results as SIBLING args of one inner perform evaluate left-to-right"
  (doc    "The multi-arg face of the op-arg let-lift: BOTH arguments of the inner `(B.put (A.get) (A.get))`
           are foreign performs of the ADVANCING outer op, so their evaluation ORDER is observable — the
           first read returns 7 (state → 8), the second 8 (state → 9), and B's arm sums them (15). A lift
           that reordered the sibling performs, ran one twice, or batched them against the same state would
           break the sum. The two-lift companion of the single-arg pin above.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((put (v w) s (resume (+ v w) s)))
                  (B.put (A.get) (A.get)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 15 Int64)))

(case "a DEPTH-3 op-arg chain threads across two handler layers"
  (doc    "The depth face of the op-arg let-lift: `(C.inn (B.mid (A.get)))` under a 3-deep stack — the
           OUTERMOST perform's argument is itself a perform, cascading inward to `A.get` (whose argument is
           Unit, not a perform), so the lift must fire at two nesting levels of the SAME expression. A.get reads 7, B.mid adds
           its state (7+100 = 107), C.inn doubles (214). A lift that flattened only one level, or evaluated
           the chain against the wrong handler's state, would break a factor. The chain companion of the
           sibling-args pin above.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op mid (-> Int64 Int64)))
            (effect C (op inn (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B 100
                  ((mid (v) s (resume (+ v s) s)))
                  (handle C 0
                    ((inn (v) s (resume (* 2 v) s)))
                    (C.inn (B.mid (A.get)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 214 Int64)))

(case "the cross-handler op-arg lift fires 100 times inside a recursive accumulator loop"
  (doc    "The SCALE face of the op-arg let-lift: `(B.put (A.get))` — the single-shot pin above — placed in a
           100-iteration accumulator loop, with A's arm ADVANCING per read. B's arm reads its param THREE
           times (`(/ (+ (+ v v) v) 3)` = v exactly), so each iteration's lift must bind the foreign perform
           ONCE and serve the pure ref thrice — a lift that re-ran the perform per read would see A advance
           between reads (v, v+1, v+2) and shift the quotient. Every advance must also thread across
           iterations: the sum of A's reads 0..99 = 4950. The recursion companion of the sibling-args and
           depth-3 pins, with the arm shaped to force the lift's duplication handling.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc (loop (- n 1) (+ acc (B.put (A.get))))))
            (def (main (: k Int64))
              (handle A 0
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((put (v) s (resume (/ (+ (+ v v) v) 3) s)))
                  (loop k 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 4950 Int64)))

(case "a cross-handler foreign-perform op-arg into a MATCH-shaped-resume arm declines cleanly (completeness gap)"
  (doc    "The completeness boundary where the op-arg lift meets the match-shaped-resume peel (breaker nv1f).
           The inner `B.cut`'s ARG is a CROSS-HANDLER foreign perform `(A.src)` AND B's arm RESUMES through a
           MATCH on a slice of its param — `(cut (b) t (match (Bytes.slice b 1 2) ((Some w) (resume w t)) ((None
           _x) (resume (Bytes.of (list)) t))))`. Each half folds ALONE: a cross-handler foreign-perform arg with
           a BARE-resume arm folds (nv1c/nv1d: `(cut (b) t (resume (Bytes.len b) t))`), and a match-shaped-resume
           arm with a LITERAL arg folds (nv1e). But their CONJUNCTION declines: `b` is single-use so `(A.src)`
           substitutes directly into the match SCRUTINEE `(Bytes.slice (A.src) 1 2)`, and threading a foreign
           perform embedded in a peeled match-value's scrutinee across the outer `A` fold is not yet composed.
           DECLINE, never a wrong value (an honest not-yet-reducible todo). When the op-arg-lift × match-peel
           composition lands, this FOLDS to 14: `A.src` = bytes[20,30,40]; `Bytes.slice … 1 2` = [30,40] (Some);
           `B.cut` resumes that view; `Bytes.len` = 2; `(+ 2 12)` = 14. The output is already pinned (14); when
           the composition lands, flip this case's baseline entry todo→pass.")
  (input  (do
            (effect A (op src (-> Unit Bytes)))
            (effect B (op cut (-> Bytes Bytes)))
            (def (main (: a Int64))
              (handle A 0
                ((src (u) s (resume (Bytes.of (list 20 30 40)) s)))
                (handle B 0
                  ((cut (b) t
                    (match (Bytes.slice b 1 2)
                      ((Some w) (resume w t))
                      ((None _x) (resume (Bytes.of (list)) t)))))
                  (+ (Bytes.len (B.cut (A.src))) a))))
            (export main)))
  (call   main (: 12 Int64)) (output (: 14 Int64)))

(case "a BRANCHING tree walk performs once per leaf at 200-leaf scale"
  (doc    "Branching self-recursion × per-node performs (the recursive-perform pins are all LINEAR loops):
           `walk` recurses into BOTH children of a user-sum tree (`(+ (walk a) (walk b))`), each LEAF
           performing once in operand position. Over a 200-leaf spine the state must thread through every
           branch junction: the walk sums the leaves (5 + 199·1 = 204) while 200 advances land, and the
           trailing perform reads exactly 200 → 10·204 + 200 = 2240. A state fork or drop at any of the
           199 junctions shifts one of the factors.")
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (effect Cnt (op bump (-> Unit Int64)))
            (def (build (: i Int64) (: e Exp))
              (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
            (def (walk (: e Exp))
              (match e
                ((Exp.Lit v) (+ v (* 0 (Cnt.bump))))
                ((Exp.Add a b) (+ (walk a) (walk b)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1))))
                (+ (* 10 (walk (build n (Exp.Lit 5)))) (Cnt.bump))))
            (export main)))
  (call   main (: 199 Int64)) (output (: 2240 Int64)))

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

(case "a handler threads a TUPLE of two heaps as state with different ops touching different halves"
  (doc    "The SPLIT-state idiom: state = (tuple (list) Map.empty), note touches the LIST half and
           tag the MAP half — each arm PROJECTS its half ((. st 0)/(. st 1)), updates it, rebuilds
           the tuple; the untouched half's handle threads through unchanged, and tag reads List.len
           of the OTHER half so the halves must stay in sync. (Arm bodies use projections rather
           than match — a handler arm whose body is a match trips the ML-printer arm-extent
           ambiguity, filed separately.)")
  (input (do
        (effect S (op note (-> Int64 Int64)) (op tag (-> Int64 Int64)))
        (def (main (: n Int64))
          (handle S (tuple (list) Map.empty)
            ((note (v) st
              (let ((lg2 (List.push (. st 0) v)))
                (resume (List.len lg2) (tuple lg2 (. st 1)))))
             (tag (k) st
              (let ((ix2 (Map.insert (. st 1) k (List.len (. st 0)))))
                (let ((got (match (Map.lookup ix2 k) ((Some x) x) ((None _u) -1))))
                  (resume got (tuple (. st 0) ix2))))))
            (do
              (def r1 (S.note 10))
              (def t1 (S.tag 5))
              (def r2 (S.note n))
              (def t2 (S.tag 5))
              (+ (* r1 1000) (+ (* t1 100) (+ (* r2 10) t2))))))
        (export main)))
  (call main (: 20 Int64)) (output (: 1122 Int64))
  (call main (: 0 Int64)) (output (: 1122 Int64)))
(case "a LIST built in the handle body from perform results crosses the handle exit live"
  (doc    "The collect pin's list exits via STATE; this one is constructed IN the body from perform
           RESULTS interleaved with a runtime param — element evaluation interleaves with
           perform/resume round-trips, and the finished heap value survives handler teardown.")
  (input (do
        (effect Ctr (op tick (-> Unit Int64)))
        (def (sum-l (: l (List Int64)) (: acc Int64))
          (match l
            ((list) acc)
            ((list h .. t) (sum-l t (+ acc h)))))
        (def (main (: n Int64))
          (do
            (def xs (handle Ctr 0
                      ((tick (_u) c (resume c (+ c 1))))
                      (list (Ctr.tick) (Ctr.tick) n)))
            (+ (* (sum-l xs 0) 10) (List.len xs))))
        (export main)))
  (call main (: 5 Int64)) (output (: 63 Int64))
  (call main (: 0 Int64)) (output (: 13 Int64)))

(case "a MAP keyed by perform results in the handle body crosses the exit and looks up by those keys"
  (doc    "The CHAMP composition: the map's KEYS are perform results — insert-arg evaluation
           interleaves with perform/resume, the champ hash runs on resumed values, and the map is
           looked up by those keys post-exit.")
  (input (do
        (effect Ctr (op tick (-> Unit Int64)))
        (def (get (: m (Map Int64 Int64)) (: k Int64))
          (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
        (def (main (: n Int64))
          (do
            (def m (handle Ctr 0
                     ((tick (_u) c (resume c (+ c 1))))
                     (Map.insert (Map.insert Map.empty (Ctr.tick) 10) (Ctr.tick) n)))
            (+ (* (get m 0) 10) (get m 1))))
        (export main)))
  (call main (: 20 Int64)) (output (: 120 Int64))
  (call main (: 0 Int64)) (output (: 100 Int64)))

(case "a rope accumulates across perform/resume boundaries and content-checks at the exit"
  (doc    "The strings member: a recursive builder concats a chunk per perform, each chunk selected
           by the resume value — the accumulating rope survives N suspension boundaries, and the
           handler SEED shifts which letters are picked (content-checked at exit).")
  (input (do
        (effect Ctr (op tick (-> Unit Int64)))
        (def (pick (: k Int64))
          (match (& k 3)
            (0 "a") (1 "b") (2 "c") (_ "d")))
        (def (go (: i Int64) (: acc String))
          (if (= i 0)
              acc
              (go (- i 1) (String.concat acc (pick (Ctr.tick))))))
        (def (main (: n Int64))
          (do
            (def s (handle Ctr n
                     ((tick (_u) c (resume c (+ c 1))))
                     (go 3 "")))
            (+ (* (String.byte-len s) 10)
               (if (= s "abc") 1 0))))
        (export main)))
  (call main (: 0 Int64)) (output (: 31 Int64))
  (call main (: 1 Int64)) (output (: 30 Int64)))

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

(case "an effectful walk over an INPUT list combines each element with a fresh perform"
  (doc    "The input-driven map (the build case above is DEPTH-driven — the recursion count decides the
           output; here an INPUT list drives the walk and each of ITS elements combines with a perform):
           `tag-all` reads `xs[i]` and pushes `100·(Idx.next) + v` — pairing the element with a fresh id —
           recursing until `List.at` misses. Seeded 1: elements 10/20/n pick up ids 1/2/3; the readout
           encodes `10·len + tagged[2]` = 30 + (100·3 + 30) = 360 at n = 30. Pins the tag-each-element
           idiom (a compiler numbering its input nodes): per-element state advances interleave with
           per-element heap reads, and the tagged list escapes the handle.")
  (input  (do
            (effect Idx (op next (-> Unit Int64)))
            (def (tag-all (: xs (List Int64)) (: i Int64) (: acc (List Int64)))
              (match (List.at xs i)
                ((Some v) (tag-all xs (+ i 1) (List.push acc (+ (* 100 (Idx.next unit)) v))))
                ((None u) acc)))
            (def (main (: n Int64))
              (let ((tagged (handle Idx 1
                              ((next (u) s (resume s (+ s 1))))
                              (tag-all (list 10 20 n) 0 (list)))))
                (+ (* 10 (List.len tagged))
                   (match (List.at tagged 2) ((Some v) v) ((None u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 360 Int64)))

(case "a closure capturing a handler-computed VALUE escapes the handle and applies outside"
  (doc    "The escaping-closure acceptance witness (breaker finding, fixed): the perform runs INSIDE the
           handle (`base = (Cfg.get unit)`), the closure captures the resulting plain VALUE — performing
           nothing itself — and escapes as the handle's result, applied OUTSIDE (2+40 = 42). The escape
           analysis must distinguish 'a perform occurred in the body that built this closure' from 'this
           closure performs' (it once rejected CDZ0401 on exactly this shape); the correct-reject twin —
           a closure whose BODY performs escaping — stays rejected elsewhere.")
  (input  (do
            (effect Cfg (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (let ((f (handle Cfg n
                         ((get (u) s (resume s s)))
                         (let ((base (Cfg.get unit)))
                           (fn ((: x Int64)) (+ x base))))))
                (f 40)))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 42 Int64)))

(case "TWO closures escaping one handle carry DISTINCT captured state reads"
  (doc    "The two-capture composition: both closures capture different reads of the SAME advancing
           counter (a = seed, b = seed+1) and escape in one tuple; applied outside, each must see ITS
           read — f(100) = 100+3, g(10) = 10·4 → 143. An environment that shared one capture slot (or
           re-read the final state for both) collapses a and b.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (match (handle Ctr n
                       ((next (u) s (resume s (+ s 1))))
                       (tuple (let ((a (Ctr.next unit))) (fn ((: x Int64)) (+ x a)))
                              (let ((b (Ctr.next unit))) (fn ((: x Int64)) (* x b)))))
                ((tuple f g) (+ (f 100) (g 10)))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 143 Int64)))

(case "a closure built OUTSIDE a handler is applied INSIDE it beside performs"
  (doc    "The inbound direction: a pure closure constructed before the handle is applied within the
           handle body with a PERFORM as its argument — `(f (Ctr.next unit))` reads the seed 4 through
           the ×10 capture, the second perform reads 5 → 45. The closure's environment predates the
           handler frame; application under the handler must not confuse the capture with handler state.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (mk (: k Int64)) (fn ((: x Int64)) (* x k)))
            (def (main (: n Int64))
              (let ((f (mk 10)))
                (handle Ctr n
                  ((next (u) s (resume s (+ s 1))))
                  (+ (f (Ctr.next unit)) (Ctr.next unit)))))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 45 Int64)))

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

(case "a callee whose body LET-BINDS a handle seeded by its param, called with the caller's runtime arg, keeps that arg bound"
  (doc    "The caller-arg-through-a-let-wrapped-handle-seed shape: `(def (f x) (let ((r (handle St x …))) r))`
           binds the result of a `handle` whose SEED is the param `x`, and `main` calls `(f k)` passing its OWN
           runtime param `k`. This spuriously reported CDZ0101 'unbound name k' FROM THE COMPILE BACKEND (`cdz
           check` passed) — inlining `f` substituted the handle seed `x`→the arg node carrying `k`, then the
           tail-resumptive fold's `deep_fresh_copy` spliced that ONE seed node at BOTH state-binder references
           in the arm body (`(resume s (+ s 1))`), and the re-parent to the last site ORPHANED the first, so
           `k` re-resolved unbound. The fix let-binds a non-constant seed ONCE at the fold entry (an orphaned-
           occurrence bug of the same family as an extracted child spliced without a re-parenting copy). Only
           the CONJUNCTION triggers it — a const arg, a handle DIRECTLY in the body (no let), or a let over a
           NON-handle init each compiled. `main 5`: the handler resumes with the state (seeded 5), `(St.tick)`
           yields it → 5. Guards that a let-bound handle init seeded by a param does not drop the caller's
           runtime argument (the exact shape `verify_enforce` injects for `@ensures`/`@requires` over a
           handle-bodied def).")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (f (: x Int64))
              (let ((r (handle St x ((tick (u) s (resume s (+ s 1)))) (St.tick)))) r))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64)))

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

(case "an inner handle's SEED expression performs against the outer handler"
  (doc    "The seed-position perform: `(handle B (+ (A.get unit) 100) …)` — the inner handle's SEED is
           computed by performing the OUTER effect (A.get reads n=5), so the inner handler starts at 105
           and the body's `(B.get unit)` reads it back beside a second outer read (105 + 5 = 110). The
           seed expression evaluates in the OUTER handler's scope BEFORE the inner handler exists; a
           lowering that evaluated the seed under the inner handler (or defaulted it) mis-seeds.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B (+ (A.get unit) 100)
                  ((get (u) t (resume t t)))
                  (+ (B.get unit) (A.get unit)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 110 Int64)))

(case "an inner same-effect handle's RESULT feeds the outer handler's next perform"
  (doc    "Cross-region value flow under shadowing: the inner `handle Ctr 100` discharges `(Ctr.bump 2)`
           with a MULTIPLYING arm (100·2 = 200) and its result becomes the ARGUMENT of the outer region's
           `(Ctr.bump inner)` — discharged by the outer ADDING arm (10 + 200 = 210). The value crosses
           from the inner region's arm through the let into the outer region's perform; the shadow pins
           nearby witness state ISOLATION, this witnesses the VALUE HANDOFF between regions of one
           effect.")
  (input  (do
            (effect Ctr (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Ctr n
                ((bump (v) s (resume (+ s v) (+ s v))))
                (let ((inner (handle Ctr 100
                               ((bump (v) t (resume (* t v) t)))
                               (Ctr.bump 2))))
                  (Ctr.bump inner))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 210 Int64)))

(case "same-effect shadowing with ADVANCING states — the outer state survives the inner handle and resumes advanced"
  (doc    "The STATEFUL upgrade of the lexical-partition case above (there both arms resume `s` unchanged,
           so a shared or re-seeded state slot is invisible): here BOTH handlers ADVANCE a counter. The
           outer `Ctr` seeds 10; its first tick reads 10 (state → 11). The inner `handle Ctr 2000` then
           discharges its own region's two ticks — 2000 and 2001 (its own slot, seeded independently,
           advancing independently) → 4001. The perform AFTER the inner handle exits reaches the OUTER
           handler again and must read 11 — the outer state advanced by the pre-inner tick, UNTOUCHED by
           the inner region's two discharges, resumed exactly where it left off. 10 + 4001 + 11 = 4022. A
           shadow implementation sharing one state slot (inner ticks bleeding the outer to 12/13), or
           re-seeding the outer on inner-exit (reading 10 again → 4021), breaks the value. Expected: 4022.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main)
              (handle Ctr 10 ((tick (u) s (resume s (+ s 1))))
                (+ (Ctr.tick)
                   (+ (handle Ctr 2000 ((tick (u) s (resume s (+ s 1))))
                        (+ (Ctr.tick) (Ctr.tick)))
                      (Ctr.tick)))))
            (export main)))
  (output (: 4022 Int64)))

(case "a nested handle's INIT expression performs against the OUTER handler before installing"
  (doc    "The install boundary itself performing: the inner seed (Out.tick) evaluates in the
           OUTER's scope, its resume value becomes the inner seed, and the outer state advance
           survives to the trailing (Out.tick) — 100+101=201.")
  (input (do
        (effect Out (op tick (-> Unit Int64)))
        (effect In (op get (-> Unit Int64)))
        (def (main (: seed Int64))
          (handle Out seed
            ((tick (_u) c (resume c (+ c 1))))
            (+ (handle In (Out.tick)
                 ((get (_u) s (resume s s)))
                 (In.get))
               (Out.tick))))
        (export main)))
  (call main (: 100 Int64)) (output (: 201 Int64))
  (call main (: 0 Int64)) (output (: 1 Int64)))

(case "outer handler state threads AROUND a completed inner handle of a different effect"
  (doc    "State continuity across an inner lifecycle: outer tick / full inner handle installs+runs+
           tears down / outer tick — the b−a=1 digit proves exactly ONE increment happened across
           the inner (a state reset or double-advance flips it).")
  (input (do
        (effect Out (op tick (-> Unit Int64)))
        (effect In (op get (-> Unit Int64)))
        (def (main (: seed Int64))
          (handle Out seed
            ((tick (_u) c (resume c (+ c 1))))
            (do
              (def a (Out.tick))
              (def inner (handle In 5
                           ((get (_u) s (resume s s)))
                           (In.get)))
              (def b (Out.tick))
              (+ (* a 100) (+ (* inner 10) (- b a))))))
        (export main)))
  (call main (: 3 Int64)) (output (: 351 Int64))
  (call main (: 0 Int64)) (output (: 51 Int64)))

(case "TWO sequential inner handles seed from the outer's ADVANCING state"
  (doc    "Repeated seeding: each inner's performing INIT reads the outer at a different point
           (i1=seed, i2=seed+1) and fin−i1=2 proves both advances stuck across two full inner
           install/teardown cycles.")
  (input (do
        (effect Out (op tick (-> Unit Int64)))
        (effect In (op get (-> Unit Int64)))
        (def (main (: seed Int64))
          (handle Out seed
            ((tick (_u) c (resume c (+ c 1))))
            (do
              (def i1 (handle In (Out.tick)
                        ((get (_u) s (resume s s)))
                        (In.get)))
              (def i2 (handle In (Out.tick)
                        ((get (_u) s (resume s s)))
                        (In.get)))
              (def fin (Out.tick))
              (+ (* i1 100) (+ (* i2 10) (- fin i1))))))
        (export main)))
  (call main (: 3 Int64)) (output (: 342 Int64))
  (call main (: 0 Int64)) (output (: 12 Int64)))

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

(case "a mutually-recursive group with a BRANCH-PERFORM sharing a strict expr with the mutual call declines cleanly (adv-69 rw4 sub-face)"
  (doc    "adv-69 recursive-branch-perform, MUTUAL-SCC face (v-effects self-probe 2026-08-04, breaker rw4).
           CONTRAST the two folding cases above (perform and mutual call in SEPARATE branches — no shared
           strict context, mutually exclusive): here the branch-perform and the mutual call SHARE one strict
           expression — `(def (even-w n) (if (= n 0) 0 (+ (if true (St.get) 0) (odd-w (- n 1)))))` (and the
           odd-w twin). The `(if true (St.get) 0)` branch-perform is a strict operand of `+` ALONGSIDE the
           mutual call `(odd-w …)`. The single-return specialization threads the branch perform against the
           INCOMING state, but the advance is branch-local and the recursion carries the incoming state
           forward, so it drops across the cycle: seeded St=1 it ran 3 (three gets all read seed 1), correct
           is 6 (1+2+3). DECLINE cleanly (safe floor) — a full fold needs the branch-perform lifted before
           specialization. Detected by `branch_perform_coexists_with_reentrant_call` (a branch-performing
           conditional as a strict operand alongside a re-entrant self/mutual call), keyed via
           `contains_recursive_call` so it covers the mutual SCC, not just direct self-recursion. This is the
           MUTUAL-SCC face ONLY; the SELF-recursive faces (bare `(walk n)` with the same `+` shape) are
           rewritten by the load-time accum pass and are tracked SEPARATELY (still open). Grades TODO on all
           backends; flips to 6 PASS when the branch-perform-before-recursion fold lands.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (even-w (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (odd-w (- n 1)))))
            (def (odd-w (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (even-w (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (even-w 3)))
            (export main)))
  (output (: 6 Int64)))

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

(case "a branch-performing conditional in a self-recursive performer threads the advance across recursion (rw1)"
  (doc    "The recursive-branch-perform fix (v-effects self-probe, breaker rw1, operator-prioritized as a HIGH
           miscompile): a discharged perform inside a conditional BRANCH `(if true (St.get) 0)` that is a strict
           operand alongside the self-call `(walk (- n 1))` — `(+ (if true (St.get) 0) (walk (- n 1)))`. The
           branch perform advances the handler state, and the sibling recursion must see that advance. This was
           a SILENT MISCOMPILE — `thread_bounded`'s `If` arm returned the post-CONDITION state as the `if`'s
           out-state (branch advances unmerged), so the walk reseeded from the stale pre-branch state and every
           step read the seed: seeded 1 it ran 3 (1+1+1), correct is 6 (1+2+3). FIXED by MERGING the per-branch
           out-states into a conditional-valued out-state `(if cond then-out else-out)` so the sibling recursion
           threads the branch's advance (gated on a pure condition + `#cv`-free branch out-states to stay
           arena-safe). Now folds to 6 on all backends.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (walk (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
  (output (: 6 Int64)))

(case "a RUNTIME-conditioned branch perform in a self-recursive performer threads the advance (rw3)"
  (doc    "The runtime-condition face of the recursive-branch-perform fix (rw3): the branch conditional's test
           is a RUNTIME value `(> n 0)` rather than a constant, so the fold cannot key on a foldable condition —
           the per-branch out-state merge handles it uniformly. Same shape/values as rw1 (seeded 1 → 6), the
           branch perform's advance threads to the sibling recursion via the conditional-valued out-state.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64)) (if (= n 0) 0 (+ (if (> n 0) (St.get) 0) (walk (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
  (output (: 6 Int64)))

(case "a HEAP-state branch perform in a self-recursive performer threads the pushes across recursion (rw5)"
  (doc    "The heap-state (data-loss) face of the recursive-branch-perform fix (rw5): the handler state is a
           LIST accumulator and the branch perform conditionally pushes onto it; the branch advance is the
           `List.push`, which the recursion must carry forward. Pre-fix the pushes were LOST (the branch
           out-state dropped, so each step pushed against the empty seed → count 0); the per-branch out-state
           merge threads the growing list, so the three conditional pushes accumulate → length 3. The data-loss
           twin of rw1 — a wrong heap value, not just a stale scalar.")
  (input  (do
            (effect Log (op add (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (= n 0) (Log.count) (do (if true (Log.add n) 0) (walk (- n 1)))))
            (def (main)
              (handle Log (list) ((add (v) s (resume v (List.push s v))) (count (u) s (resume (List.len s) s)))
                (walk 3)))
            (export main)))
  (output (: 3 Int64)))

(case "a MATCH-arm perform in a self-recursive performer threads the advance across recursion (rw-match)"
  (doc    "The MATCH-arm face of the recursive-branch-perform fix (the `if`-branch cases rw1/rw3/rw5 are its
           `if` siblings): the discharged perform is in a `match` ARM body — `(+ (match true (_ (St.get)))
           (walk (- n 1)))` — a strict operand alongside the self-call. Same drop as rw1: `thread_bounded`'s
           `Match` arm returned the post-SCRUTINEE state as the match's out-state (arm advances unmerged), so
           the sibling recursion reseeded from the stale pre-arm state — seeded 1 it ran 3, correct is 6. FIXED
           by the `Match` arm analogue of the `if` per-branch out-state merge: the arm out-states merge into a
           `(match scrut (pat arm-out)…)`-valued out-state (gated on a pure scrutinee + `#cv`-free arm
           out-states, same as the `if` arm). Now folds to 6 on all backends. Pins that the merge covers BOTH
           conditional forms (`if` and `match`).")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (walk (: n Int64)) (if (= n 0) 0 (+ (match true (_ (St.get))) (walk (- n 1)))))
            (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
            (export main)))
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

(case "an abortive perform in a recursive callee with a PENDING continuation in the handle body abandons it"
  (doc    "The soundness companion of the tail-walk-bail above (which is the handle body's TAIL, no pending
           work). Here the recursive-abortive callee's result feeds a PENDING continuation in the handle
           body: `(+ (go 2) 999999)` where `(go n)` tail-recurses and bails at zero. The abort MUST abandon
           the `(+ _ 999999)` — bail's arm value 500 becomes the handle's value, +7 outside → 507. It must
           NOT flow 500 INTO the pending `+ 999999` (the adv-52 miscompile: 500+999999+7 = 1000506, a silent
           wrong value that appeared on all backends). Abandoning past a pending continuation at the OUTER
           call site needs the br-out-of-handle non-local-exit convention (a later vertical); until then the
           compiler DECLINES this shape cleanly (a Todo) rather than emit the wrong value — so this case pins
           the DECLINE as the safe floor. A value-recorded case that declines grades Todo, so recording the
           correct 507 guards that the fold, if it ever serves this shape, MUST yield 507, never 1000506.
           (breaker adv-52; the mutual-recursion and pending-inside-the-callee neighbors already decline.)")
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64))
              (if (= n 0) (Mx.bail 5) (go (- n 1))))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 2) 999999)) 7)) (export main)))
  (output (: 507 Int64)))

(case "a RUNTIME branch selects between an abortive perform and a plain value per call"
  (doc    "The per-call abort selection (the branch-abort pins use const conditions): `(if (> n 0)
           (+ (Bail.out n) 999) (- 0 n))` — n=4 takes the abortive path (the arm multiplies, the +999
           continuation is abandoned → 40); n=-6 takes the plain path (6). ONE compiled body must both
           abandon and complete depending on the call — an emit specializing the handle to always-abort
           (or always-resume) breaks the other call.")
  (input  (do
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail 0
                ((out (v) s (* v 10)))
                (if (> n 0)
                  (+ (Bail.out n) 999)
                  (- 0 n))))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 40 Int64))
  (call   main (: -6 Int64))
  (output (: 6 Int64)))

(case "an abortive perform MID-WALK carries the accumulated state out as its argument"
  (doc    "The annotated-walk bail above aborts at the BASE with a const; here the abort fires MID-walk
           at a sentinel (n=2) and its ARGUMENT is the accumulator built so far — walk(5,0) accumulates
           5+4+3=12 before bailing with 12; walk(1,0) never hits the sentinel and returns normally (1).
           The abort value carries live loop state out through the abandoned frames (the early-exit-with-
           partial-result idiom); an abort that read a stale accumulator drifts.")
  (input  (do
            (effect Bail (op out (-> Int64 Int64)))
            (def (walk (: n Int64) (: acc Int64))
              (if (< n 1) acc (if (= n 2) (Bail.out acc) (walk (- n 1) (+ acc n)))))
            (def (main (: n Int64))
              (handle Bail 0
                ((out (v) s v))
                (walk n 0)))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 12 Int64))
  (call   main (: 1 Int64))
  (output (: 1 Int64)))

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

(case "a runtime LIST argument to an effect op is WALKED by a recursive fold inside the arm"
  (doc    "The walked-collection upgrade of the compound-parameter pins (the tuple/record args above are
           const scalar-leaf projections): `(Sink.tally (list a 2 30))` carries a runtime-element list
           into the arm, whose body runs a full RECURSIVE fold over the bound parameter before resuming —
           10+2+30 = 42. The RRB handle must arrive intact and support the head-tail destructure loop
           from inside the handler context (an arm is not a plain function body — the fold runs under the
           handler's dispatch machinery).")
  (input  (do
            (effect Sink (op tally (-> (List Int64) Int64)))
            (def (sum-l (: xs (List Int64)) (: acc Int64))
              (match xs ((list) acc) ((list h .. t) (sum-l t (+ acc h)))))
            (def (main (: a Int64))
              (handle Sink 0
                ((tally (xs) s (resume (sum-l xs 0) s)))
                (Sink.tally (list a 2 30))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 42 Int64)))

(case "a MAP argument to an effect op is looked up inside the arm at the handler's own state"
  (doc    "The CHAMP-descent-in-arm face: the perform carries a 2-entry map, and the arm looks it up at
           the handler's STATE value (`s`, seeded from the boundary parameter) — composing the op
           argument, the state slot, and the CHAMP descent in one arm expression. k=2 hits (20), k=9
           misses (-1). A lowering that rebound the arm's parameter or state wrong feeds the lookup the
           wrong key or trie.")
  (input  (do
            (effect Sink (op pick (-> (Map Int64 Int64) Int64)))
            (def (main (: k Int64))
              (handle Sink k
                ((pick (m) s (resume (match (Map.lookup m s) ((Some v) v) ((None u) -1)) s)))
                (Sink.pick (Map.insert (Map.insert Map.empty 1 10) 2 20))))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 20 Int64))
  (call   main (: 9 Int64))
  (output (: -1 Int64)))

(case "a handler ARM enumerates a 60-key trie op argument and resumes its fold"
  (doc    "The DEEP-trie upgrade of the map-argument arm case above (whose map has 2 entries): the
           perform carries a 60-key MULTI-LEVEL trie, and the arm runs a full `Map.to-list` enumeration
           plus a pair-fold over it before resuming — Σ i for i = 1..60 = 1830. The multi-level
           enumeration walk (node descent, cross-node merge order) runs INSIDE the handler's dispatch
           machinery; an arm context that corrupted a frame slot mid-walk would poison the sum. The
           arm-side companion of the deep-trie enumeration pins.")
  (input  (do
            (effect Sink (op tally (-> (Map Int64 Int64) Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (sum-pairs (: ps (List (Tuple Int64 Int64))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (sum-pairs t (+ acc v)))))))
            (def (main (: n Int64))
              (handle Sink 0
                ((tally (m) s (resume (sum-pairs (Map.to-list m) 0) s)))
                (Sink.tally (fill n Map.empty))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 1830 Int64)))

(case "a handler's heap STATE grows to a 40-key trie across resumes and enumerates at the end"
  (doc    "The state-side companion: the handler's STATE is a map that GROWS by one insert per resume
           across 40 separate `put` discharges (values i·10), then a second op enumerates the
           accumulated trie — Σ i·10 for i = 1..40 = 8200. Composes state threading (each resume hands
           the next state forward), trie growth past the single-node capacity, and the enumeration walk
           over the final accumulated structure. The keyed-store idiom's growth face at scale (the
           Map-state pins put/get single entries).")
  (input  (do
            (effect Acc (op put (-> Int64 Int64)) (op total (-> Unit Int64)))
            (def (sum-pairs (: ps (List (Tuple Int64 Int64))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (sum-pairs t (+ acc v)))))))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Acc.put i) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Acc Map.empty
                ((put (v) s (resume 0 (Map.insert s v (* v 10))))
                 (total (u) s (resume (sum-pairs (Map.to-list s) 0) s)))
                (do
                  (feed 1 (+ n 1))
                  (Acc.total))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 8200 Int64)))

(case "a handler SEEDED with a 40-key trie reads it across resumes"
  (doc    "The deep-trie SEED face (the state-growth case above starts EMPTY and grows; here the heap
           state arrives fully-built at the handle boundary): a 40-key trie built before the handle
           seeds it, and the arm reads `Map.len` across two resumes (80). The seed materializes once
           and threads intact — a seed path that re-evaluated the fill per resume, or that handed the
           arm a stale snapshot, would double-build or misread.")
  (input  (do
            (effect Rd (op keys (-> Unit Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (main (: n Int64))
              (handle Rd (fill n Map.empty)
                ((keys (u) s (resume (Map.len s) s)))
                (+ (Rd.keys) (Rd.keys))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 80 Int64)))

(case "an arm REPLACES the trie state wholesale and the next op reads the replacement"
  (doc    "The state-slot ownership face at scale: the arm's resume hands back a COMPLETELY NEW trie
           (a 60-key rebuild with a different key prefix) in place of the 30-key seed — drop-old /
           adopt-new across one resume. The swap op reports the OLD len as its value (30) while
           installing the replacement; the next op reads the NEW len (60) → 30·1000 + 60 = 30060.
           A state thread that leaked the old trie, or aliased old and new, would corrupt one of the
           two reads. (The wholesale-replacement companion of the per-op insert growth above.)")
  (input  (do
            (effect Sw (op swap (-> Unit Int64)) (op len (-> Unit Int64)))
            (def (fill (: i Int64) (: k Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) k (Map.insert m (+ (* k 1000) i) i))))
            (def (main (: n Int64))
              (handle Sw (fill n 1 Map.empty)
                ((swap (u) s (resume (Map.len s) (fill (* n 2) 2 Map.empty)))
                 (len (u) s (resume (Map.len s) s)))
                (+ (* 1000 (Sw.swap)) (Sw.len))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 30060 Int64)))

(case "a TUPLE op argument with a HEAP leaf destructures inside the arm"
  (doc    "The mixed-representation companion of the scalar tuple-parameter case: `(tuple a \"abc\")`
           carries an i64 AND a rope handle through the perform; the arm destructures both and measures
           the string leaf (39 + 3 = 42). The op-argument boxing must carry the heap handle beside the
           scalar without confusing slots (the effects twin of the mixed-representation generic pins).")
  (input  (do
            (effect Sink (op unpack (-> (Tuple Int64 String) Int64)))
            (def (main (: a Int64))
              (handle Sink 0
                ((unpack (p) s (match p ((tuple n str) (resume (+ n (String.byte-len str)) s)))))
                (Sink.unpack (tuple a "abc"))))
            (export main)))
  (call   main (: 39 Int64))
  (output (: 42 Int64)))

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

(case "a String ENTRY arg rides a rope into an effect-op argument and the arm reads its bytes"
  (doc    "The String-entry-arg family (13-strings — wasm declines the entry marshal, a sound todo; rust
           computes) composed with EFFECTS: the boundary `s` is concatenated into a runtime rope, performed
           as the String ARGUMENT of `Log.emit`, and the handler arm reads the arg's byte length —
           byte-len(\"xy\"+\"abc\") = 5. Pins the full entry→rope→op-arg→arm chain on the targets that
           marshal the entry arg; the op-argument String path itself is already pinned const (the blen
           case above) — this witnesses a RUNTIME-valued op argument flowing from the component boundary.")
  (input  (do
            (effect Log (op emit (-> String Int64)))
            (def (main (: s String))
              (handle Log 0
                ((emit (m) st (resume (String.byte-len m) st)))
                (Log.emit (String.concat s "abc"))))
            (export main)))
  (call   main (: "xy" String))
  (output (: 5 Int64)))

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

(case "a handler state of TWO tries (tuple) updates each side independently across resumes"
  (doc    "The multi-field upgrade of the sum-state case above (whose state wraps ONE counter): the
           handler's state is a TUPLE of two maps, each op destructuring the pair and rebuilding it with
           ITS side updated — two addl grow the left trie, one addr the right, and sizes reads both
           (2·10 + 1 = 21). A state-slot rebuild that clobbered the untouched side (or aliased the two
           tries) would misreport a size. The two-table handler shape (e.g. a symbol table beside a
           diagnostics table) threaded as one compound state.")
  (input  (do
            (effect Tw (op addl (-> Int64 Int64)) (op addr (-> Int64 Int64)) (op sizes (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Tw (tuple Map.empty Map.empty)
                ((addl (v) s (match s ((tuple l r) (resume 0 (tuple (Map.insert l v v) r)))))
                 (addr (v) s (match s ((tuple l r) (resume 0 (tuple l (Map.insert r v v))))))
                 (sizes (u) s (match s ((tuple l r) (resume (+ (* 10 (Map.len l)) (Map.len r)) s)))))
                (do
                  (Tw.addl 1)
                  (Tw.addl 2)
                  (Tw.addr 10)
                  (Tw.sizes))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 21 Int64)))

(case "a RECORD handler state evolves a table and a counter that genuinely DIVERGE"
  (doc    "The record companion, with a divergence witness: the state is `(record (tbl …) (ops …))` and
           each put inserts into the table AND increments the counter — but the table DEDUPES (three
           puts, two distinct keys) while the counter counts every op, so tbl-len 2 ≠ ops 3 (→ 23). The
           divergence proves both fields genuinely evolve per-resume rather than mirroring one count; a
           state rebuild that recomputed one field from the other would collapse them. Field access via
           projection, rebuild via the record constructor — the row machinery inside the arm.")
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op stats (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (record (tbl Map.empty) (ops 0))
                ((put (v) s (resume 0 (record (tbl (Map.insert (. s tbl) v v)) (ops (+ (. s ops) 1)))))
                 (stats (u) s (resume (+ (* 10 (Map.len (. s tbl))) (. s ops)) s)))
                (do
                  (St.put 5)
                  (St.put 6)
                  (St.put 5)
                  (St.stats))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 23 Int64)))

(case "a SET-valued handler state accumulates uniques across resumes (the visited-set idiom)"
  (doc    "The Set face of heap handler state (the map-state rows insert; a set's DEDUP across resumes
           is the distinct contract): 20 marks of `i mod 7` feed a handler whose state is a Set — each
           arm resumes a dup-flag and inserts — and a final count reads 7 uniques. The visited-set a
           graph walk carries: membership decided against the accumulated state at every resume, the
           insert a no-op for repeats (a state thread that re-seeded or double-inserted would inflate
           the count).")
  (input  (do
            (effect Seen (op mark (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Seen.mark (% i 7)) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Seen (Set.of (list))
                ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v)))
                 (count (u) s (resume (Set.len s) s)))
                (do
                  (feed 0 n)
                  (Seen.count))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 7 Int64)))

(case "the mark op's dup-flag RESULT counts repeats while the set state dedupes"
  (doc    "The companion reading the op RESULTS instead of the final state: each mark resumes 1 iff the
           value was already seen, and the caller SUMS the flags — 20 feeds of `i mod 7` produce 13
           repeats (20 − 7 first-sightings). Pins that the per-resume result is computed against the
           state BEFORE that resume's insert (an arm that inserted first and then tested would flag
           every mark as a repeat), composing the membership read, the state advance, and the resumed
           value in one arm.")
  (input  (do
            (effect Seen (op mark (-> Int64 Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Seen.mark (% i 7)) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Seen (Set.of (list))
                ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v))))
                (feed 0 n)))
            (export main)))
  (call   main (: 20 Int64)) (output (: 13 Int64)))

(case "a perform in a match-arm guard is discharged by the enclosing handle"
  (doc    "`(handle Ask 5 ((get () s (resume s (- s 1)))) (match 9 ((guard n (> (Ask.get) 3)) 100) (n 200)))`
           — a perform `(Ask.get)` inside a match-arm GUARD condition, discharged by an intra-program
           `handle`. A perform in the SCRUTINEE, ARM BODY, or an IF CONDITION under the same handle all fold,
           and NOW so does a guard condition — for the SOUND, NARROW shape: a guarded arm whose inner pattern
           is IRREFUTABLE (a bare name / `_`) followed by an irrefutable catch-all. Such a match is selected
           iff the guard holds, so `reduce_handle` desugars it to `(if <guard> <arm-body> <catch-all-body>)`
           (each binder let-bound to the scrutinee), where the guard is an `if` CONDITION — a strict-first
           position the if-condition fold routes through the enclosing handle. The guard reads the seed 5,
           `5 > 3` holds, so the first arm fires → 100. (A REFUTABLE guarded pattern now ALSO folds — via a
           match that keeps the pattern and hoists the guard into an inner `if`, see the cases below. MULTIPLE
           guarded arms — which sequence handler state per arm-test — remain not-this-shape and decline
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

(case "a performing match-arm guard on a REFUTABLE pattern folds (keeps the match, hoists the guard)"
  (doc    "The refutable-pattern face of the performing guard-desugar (breaker bg-family). When the guarded
           arm's inner pattern is REFUTABLE — a literal, ctor, `(bin …)`, or `(tuple …)` — the irrefutable
           rewrite (`(if g b b2)`) would be UNSOUND: it drops the pattern-match, so a scrutinee FAILING the
           pattern would still run the guard `g`. The sound rewrite KEEPS the pattern and hoists the
           performing guard into an `if` INSIDE the matched arm: `(match k ((guard P g) b) (_ b2))` ≡
           `(match k (P (if g b b2)) (_ b2))`. Here the bit-pattern `(bin (u8 tag) (u8 val))` matches the
           two-byte scrutinee (tag=7, val=42), the guard `(> val (St.quota))` reads the seed (n) and holds
           for n<42, so the arm yields `(+ (* 100 tag) val)` = 742; for n≥42 the guard fails and the arm's
           inner `if` falls to the catch-all -1. A scrutinee that FAILS the pattern reaches the catch-all
           WITHOUT running the guard perform (the match, not the guard, gates it). Seeded n=5 → 742.")
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (+ s 1))))
                (match (bin (u8 (UInt8.wrap 7)) (u8 (UInt8.wrap 42)))
                  ((guard (bin (u8 tag) (u8 val)) (> val (St.quota)))
                    (+ (* 100 tag) val))
                  (_other -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 742 Int64)))

(case "a performing guard on a refutable pattern whose guard FAILS falls to the catch-all"
  (doc    "The guard-fails path of the refutable performing-guard fold: same shape as above but seeded so the
           guard is false — `(> val (St.quota))` with `val`=42 and the seed n=50, so `42 > 50` is FALSE. The
           pattern still matches (tag=7, val=42), the hoisted inner `if` evaluates the guard (which reads the
           seed 50) and takes the else branch → the catch-all -1. Pins that a matched pattern with a failing
           performing guard folds to the fall-through, not the guarded body.")
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (+ s 1))))
                (match (bin (u8 (UInt8.wrap 7)) (u8 (UInt8.wrap 42)))
                  ((guard (bin (u8 tag) (u8 val)) (> val (St.quota)))
                    (+ (* 100 tag) val))
                  (_other -1))))
            (export main)))
  (call   main (: 50 Int64)) (output (: -1 Int64)))

(case "a performing guard on a TUPLE-destructuring pattern folds"
  (doc    "The tuple-pattern spelling of the refutable performing-guard fold: `(guard (tuple tag val) (> val
           (St.quota)))` destructures a tuple scrutinee, and the performing guard hoists into the matched
           arm's inner `if` exactly as for the bit-pattern. `(tuple 7 42)` matches (tag=7, val=42), guard
           `42 > 5` holds → `(+ 700 42)` = 742. Confirms the refutable-pattern guard-desugar is
           pattern-shape-agnostic (bit patterns, tuples, and by extension ctor patterns all route).")
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (+ s 1))))
                (match (tuple 7 42)
                  ((guard (tuple tag val) (> val (St.quota)))
                    (+ (* 100 tag) val))
                  (_other -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 742 Int64)))

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

(case "an effectful helper performing UNDER A CONDITIONAL folds in a self-call arg"
  (doc    "The inlined helper's perform sits inside an `if` BRANCH — `turn(x,acc) = if x==1 then acc + B.b x
           else acc`, called in the self-call arg `(run (- fuel 1) (turn fuel acc))`. Threading the arg
           inlines the helper's `if`; each branch gets its own copy of the incoming state-refs. That copy was
           `copy_pure` (`beta_reduce`), whose pinned-name fast path returned the RESOLVE-PINNED `run#eff$s0`
           state ref AS-IS, so both branches SHARED the one node — a single-parent-arena orphan re-parented
           onto a dead node → CDZ0101 leaking `run#eff$s0`. `deep_fresh_copy` per branch (an unpinned fresh
           leaf that re-resolves against the spec sig, which declares `$s0`) folds it. run(4,0): only fuel==1
           performs, B.b 1 → 1 (resume hands the op arg back), so acc = 0 + 1 = 1. A shared/stale pin would
           leak `$s0`; a dropped branch-state advance would give a wrong value.")
  (input  (do
            (effect B (op b (-> Int64 Int64)) (op done (-> Int64 Int64)))
            (def (turn (: x Int64) (: acc Int64)) (if (= x 1) (+ acc (B.b x)) acc))
            (def (run (: fuel Int64) (: acc Int64))
              (if (= fuel 0) (B.done acc) (run (- fuel 1) (turn fuel acc))))
            (def (main)
              (handle B 0 ((b (x) s (resume x x)) (done (x) s (resume x x)))
                (run 4 0)))
            (export main)))
  (output (: 1 Int64)))

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

; An effect operation whose declared RETURN type is a STRUCTURAL RECORD in the ML surface's field
; spelling — each field a `(: name type)` annotation triple, `(op get (-> Unit (Record (: a Int64) (: b
; Int64))))` — must type the PERFORM `(St.get)` at that declared record, so a field of the performed value
; reads. The op's `(meta t)` scheme is read by `type_in_env` (the type-lambda scheme reducer), whose
; `RecordCtor` decode originally accepted only the 2-element `(name type)` pair; the ML `{a: Int64, …}`
; return lowers each field to a 3-element `(: a Int64)` triple, so the record decoded to `None` → the op had
; NO `(-> Unit result)` scheme → the nullary-perform site fell back to the op's META-record and `St.get()`
; typed as `(Record (apply …) (effect-op …) (t …))` instead of `{a, b}` (CDZ0203 "record has no field a" at
; a consumer). Handling `St` with a `get` arm that resumes `(record (a 1) (b 2))` and reading field `a` = 1
; pins that a structural-record effect-op return threads its declared type to the perform site (the
; `type_in_env` companion of the same `(: name type)` decode fix `typeval_of` carries for variant payloads).
(case "an effect op with a structural-record return types the perform at the declared record"
  (input  (do
            (effect St (op get (-> Unit (Record (: a Int64) (: b Int64)))))
            (def (get-a (: r (Record (: a Int64) (: b Int64)))) (. r a))
            (def (main)
              (handle St (record (a 0) (b 0))
                ((get (u) s (resume (record (a 1) (b 2)) s)))
                (get-a (St.get unit))))
            (export main)))
  (output (: 1 Int64)))

(case "one performing closure applied twice observes the handler state stepping between calls"
  (doc    "An effectful closure defined and applied (twice) directly under its handler: the SAME closure
           value performs `Tick.tick` at both applications, and the handler arm resumes with `(+ v st)`
           while stepping its state by the runtime k — so the two calls through ONE closure see
           DIFFERENT states: f(1)=1+100, then f(2)=2+100+k → 213 at k=10, 203 at k=0. Pins that a
           closure's perform re-enters the CURRENT handler state per application (not a state captured
           at closure creation). The HOF spelling of this — the closure passed to a recursive walker
           applied under the CALLER's handle — rejects by the documented per-callee-param homing
           analysis (:531/:549's soundness twin); this inline spelling is the supported one.")
  (input  (do
        (effect Tick (op tick (-> Int64 Int64)))
        (def (main (: k Int64))
          (handle Tick 100
            ((tick (v) st (resume (+ v st) (+ st k))))
            (do
              (def f (fn ((: v Int64)) (Tick.tick v)))
              (+ (f 1) (f 2)))))
        (export main)))
  (call   main (: 10 Int64)) (output (: 213 Int64))
  (call   main (: 0 Int64)) (output (: 203 Int64)))

(case "a closure crosses the perform boundary as an operation ARGUMENT and applies in the arm"
  (doc    "The op's first parameter IS a function — `app : (-> (-> Int64 Int64) Int64 Int64)` — so the
           closure VALUE rides the perform into the handler arm (the :285-family arm applies a
           lexically-visible closure; here it arrives as the operation's PAYLOAD). Two performs hand
           TWO different closures through the same op while the state steps: app(double,1) at st=10 →
           double(11) = 22; app(add-7,1) at st=10+k → 18+k. 2223 at k=5, 2218 at k=0. An op-argument
           marshalling that unified fn payloads by signature (or re-homed the closure to the arm's
           frame losing its identity) answers with the wrong body. Note: an op RESULT typed as a fn
           curried-flattens per arrow right-associativity — `(-> A (-> B C))` reads as a 2-param op,
           so the result-side face is inexpressible today (clean CDZ0201 documents it).")
  (input  (do
        (effect App (op app (-> (-> Int64 Int64) Int64 Int64)))
        (def (main (: k Int64))
          (handle App 10
            ((app (f v) st (resume (f (+ v st)) (+ st k))))
            (+ (* 100 (App.app (fn ((: x Int64)) (* x 2)) 1))
               (App.app (fn ((: x Int64)) (+ x 7)) 1))))
        (export main)))
  (call   main (: 5 Int64)) (output (: 2223 Int64))
  (call   main (: 0 Int64)) (output (: 2218 Int64)))

(case "a LIST of closures rides one perform and the arm picks by state per call"
  (doc    "Effects × collections × fn-values composed: the op payload is a whole `(List (-> Int64
           Int64))`, and the arm indexes it BY THE HANDLER STATE — call 1 (st=0) applies fs[0] =
           (+ x k) at 10 → 10+k, steps the state; call 2 (st=1) applies fs[1] = (* x k) → 10k
           (13030 at k=3; k=0 collapses to 10000 and separates the arms). The heap list of fn
           handles crosses the perform ONCE and is indexed TWICE under different states — a payload
           marshalling that flattened the list to its first element, or re-resolved handles by
           signature, collapses the two calls.")
  (input  (do
        (effect Pick (op pick (-> (List (-> Int64 Int64)) Int64)))
        (def (main (: k Int64))
          (handle Pick 0
            ((pick (fs) st
               (match (List.at fs st)
                 ((Some f) (resume (f 10) (+ st 1)))
                 ((None _u) (resume -1 st)))))
            (do
              (def fs (List.push (List.push (list) (fn ((: x Int64)) (+ x k))) (fn ((: x Int64)) (* x k))))
              (+ (* 1000 (Pick.pick fs)) (Pick.pick fs)))))
        (export main)))
  (call   main (: 3 Int64)) (output (: 13030 Int64))
  (call   main (: 0 Int64)) (output (: 10000 Int64)))

;; A tail-resumptive handler fold MUST keep an arm's local (def x ...) and a perform-site/handle-body
;; (def x ...) in their own scopes. Regression: reduce_handle spliced the arm body at the perform site
;; WITHOUT alpha-renaming, so an arm-local x captured a same-named free x both directions — silent
;; wrong value (F1 arm→body: 10 for 105; F2 body→arm: 14 for 107; both backends, shared reduce_handle).
;; Fixed by v-effects 515d6b57d (alpha-rename the local value binders — let pairs + do-local defs — of
;; both the handle body and the substituted arm body to fresh #-names). op-param + state binders were
;; already hygienic; only arm-internal locals needed the rename. breaker-routed (FINDING #33).
(case "handler-arm bindings and perform-site bindings stay in their own scopes across the fold"
  (input  (do
        (effect E (op get (-> Unit Int64)))
        (def (main (: mode Int64))
          (do
            (def x 100)
            (if (= mode 1)
                (handle E 0
                  ((get (u) s (do (def x 5) (resume (+ x s) s))))
                  (+ x (E.get)))
                (handle E 0
                  ((get (u) s (resume x s)))
                  (do (def x 7) (+ x (E.get)))))))
        (export main)))
  (call   main (: 1 Int64)) (output (: 105 Int64))
  (call   main (: 2 Int64)) (output (: 107 Int64)))

;; A handled effect performed via a closure EXTRACTED from a collection (list + List.at + match, applied
;; lexically under the handle) DECLINES with the HONEST 'not yet reducible by the tail-resumptive fold'
;; message — NOT the misleading 'performed with no enclosing handler here' (there IS one). Reject-don't-
;; miscompile-with-honest-message discipline (27-des:5120 class). Fixed by v-effects 1747c764a: the fold
;; couldn't trace the app through the collection slot (subtree_performs treated the lambda as pure) →
;; standalone lift → no-home arm; now remapped to the honest not-yet-reducible decline. breaker-routed.
(case "an effect performed via a collection-extracted closure declines honestly (not-yet-reducible, not a false no-handler claim)"
  (input (do
        (effect Ask (op ask (-> Int64 Int64)))
        (def (main)
          (handle Ask 5
            ((ask (n) s (resume (* n 2) s)))
            (match (List.at (list (fn (x) (Ask.ask x))) 0)
              ((Some f) (f 3))
              ((None) 0))))
        (export main)))
  (declines))

;; TWO NESTED handlers, each arm a do-def-local x, the body reading the enclosing FN-LOCAL x through a
;; right-nested (+ x (+ (A.geta) (B.getb))) — each binding MUST keep its own scope (no compounding
;; leak). Regression arc: pre-hygiene this MISCOMPILED to 43 (the body's x read through the inlined
;; arms); v-effects' first hygiene fix (515d6b57d) then made it a FALSE-UNBOUND (CDZ0101) because the
;; freshen pass renamed the nested inner arm and orphaned the body's fn-local x; fixed by treating a
;; nested handle as OPAQUE in the freshen walk (v-effects 77ffe55b0) — now computes 1033 (1000+11+22).
;; The deep companion of the single-handle arm-hygiene pin. breaker #33-nested.
(case "nested handlers with colliding arm-local bindings each keep their own scope (no compounding leak)"
  (input  (do
        (effect A (op geta (-> Unit Int64)))
        (effect B (op getb (-> Unit Int64)))
        (def (main (: mode Int64))
          (do
            (def x 1000)
            (handle A 1
              ((geta (u) s (do (def x 10) (resume (+ x s) s))))
              (handle B 2
                ((getb (u) s (do (def x 20) (resume (+ x s) s))))
                (+ x (+ (A.geta) (B.getb)))))))
        (export main)))
  (call   main (: 0 Int64)) (output (: 1033 Int64)))

(case "a performing do-def feeding a bin-construction operand under a handler stays bound (F2)"
  (doc    "The bin ENCODER was the sole construction-operand position that resolved its operand against
           a scope snapshot taken BEFORE the handler fold rewrote the do-defs (tuple/record/list/Set.of
           all re-resolve after) — so a do-def bound INSIDE the handle body (here `a` from a performed
           `Src.next`) read Unbound at the `bin` operand and the case died CDZ0101 `unbound name a`. Fixed
           by re-resolving the bin operand after the capture-avoiding freshen (v-inference, F2, a4da5beb7).
           The reducible arm resumes the seed unchanged, so `a = Src.next = 10`, `frame = bin(u8 10)`,
           `Bytes.at 0 = 10`. Witnesses the handler-body do-def × bin-operand seam.")
  (input  (do
        (effect Src (op next (-> Unit Int64)))
        (def (main)
          (handle Src 10
            ((next (u) s (resume s s)))
            (do
              (def a (Src.next))
              (def frame (bin (u8 (UInt8.wrap a))))
              (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1)))))
        (export main)))
  (call   main) (output (: 10 Int64)))

(case "a perform-free do-def feeding a bin-construction operand under a handler stays bound (F2)"
  (doc    "The perform-irrelevant twin of the F2 seam: performing-ness was never the trigger — ANY do-def
           bound in a handle body and consumed by a `bin` operand hit the pre-freshen scope snapshot. Here
           `a = (+ 5 1) = 6` with no perform in the def, yet the identical CDZ0101 unbound fired pre-fix.
           `frame = bin(u8 6)`, `Bytes.at 0 = 6`. Pins the discriminator: it is the bin operand under the
           handler-fold rewrite, not the effect.")
  (input  (do
        (effect Src (op next (-> Unit Int64)))
        (def (main)
          (handle Src 10
            ((next (u) s (resume s s)))
            (do
              (def a (+ 5 1))
              (def frame (bin (u8 (UInt8.wrap a))))
              (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1)))))
        (export main)))
  (call   main) (output (: 6 Int64)))

(case "two do-def-bound performs whose sum mixes handler-state width with a narrow param declines cleanly (not-yet-reducible, not an invalid module)"
  (doc    "SAFE FLOOR (v-effects, F1, 5cf911aeb). Two do-defs each bound to a performed `Src.next` are
           summed under a handler whose arm threads `(+ s x)` — mixing the i64 handler state `s` with the
           narrow UInt8 param `x`. The fold used to emit an INVALID wasm module (`func[0]`, expected i64
           found i32) while rust computed 25; the invalid module was the bug. reduce_handle now DECLINES
           cleanly (codeless `not yet reducible`) rather than emit a malformed artifact — declines-rather-
           than-miscompiles. Computing 25/20 needs a later widening-coercion fold that widens the narrow
           operand to the i64 state carrier; when that lands this flips to a value pin.")
  (input  (do
        (effect Src (op next (-> Unit Int64)))
        (def (main (: x UInt8))
          (handle Src 10
            ((next (u) s (resume s (+ s x))))
            (do
              (def a (Src.next))
              (def b (Src.next))
              (+ a b))))
        (export main)))
  (declines))

(case "a conditionally-resuming (abortive-or-resume) arm reading the enclosing fn's param declines cleanly (not-yet-reducible, not a false unbound)"
  (doc    "SAFE FLOOR (v-effects, 94581e5f1). A handler arm that CONDITIONALLY resumes — `(if cond -999
           (resume ...))`, one branch aborts with a value, the other resumes — reading the enclosing fn's
           param `k` through the handler seed `(tuple 0 k)`. The E5 reify folds used to mis-handle this
           partially-resuming arm (rewrote only the resuming branch, orphaning a synthesized copy of the
           seed's `k`), relocating a CDZ0101 `unbound name k` at lowering — check passed, emit diverged.
           A new `arm_partially_resumes` gate now makes both reify blocks DECLINE cleanly (codeless
           not-yet-reducible) when the branches disagree on resume-vs-abort, rather than emit through the
           broken fold. Computing the -999/3 value needs a later increment that lowers a conditionally-
           resuming arm; the floor is decline-rather-than-miscompile. Distinct from the straight-line
           do-def and F1 mixed-width seams — this is the abort/resume-branch-disagreement path.")
  (input  (do
        (effect Sim (op step (-> Unit Int64)))
        (def (main (: k Int64))
          (handle Sim (tuple 0 k)
            ((step (u) st (if (>= (. st 0) (. st 1)) -999 (resume (. st 0) (tuple (+ (. st 0) 1) (. st 1))))))
            (+ (Sim.step) (+ (Sim.step) (Sim.step)))))
        (export main)))
  (declines))

(case "handler op-param and state binders stay hygienic when colliding with perform-site names"
  (doc    "The CLEAN half of the arm-inline hygiene finding (arm-internal do-def/let locals leak;
           these two binder kinds do NOT): the arm's op PARAM `v` shadows a body-side v=1000
           (arm v = the operand 3, resume 3+50; body v intact → 1053) and the STATE binder `s`
           shadows a body-side s=1000 (arm s = the seed 50; body s intact → 1050). The fold
           evidently renames op params and the state binder — pinning that so the arm-LOCAL fix
           extends the SAME treatment rather than regressing these.")
  (input  (do
        (effect E (op get (-> Int64 Int64)))
        (def (main (: mode Int64))
          (if (= mode 1)
              (do
                (def v 1000)
                (handle E 50
                  ((get (v) s (resume (+ v s) s)))
                  (+ v (E.get 3))))
              (do
                (def s 1000)
                (handle E 50
                  ((get (u) s (resume s s)))
                  (+ s (E.get 7))))))
        (export main)))
  (call   main (: 1 Int64)) (output (: 1053 Int64))
  (call   main (: 2 Int64)) (output (: 1050 Int64)))

(case "a performing closure duplicated through a generic tuple applies twice with stepping state"
  (doc    "Effects × the generic DATA position: the performing closure rides `dup` (an unannotated
           generic) into a tuple, and BOTH projections apply under the handler — the homing analysis
           must track the perform through the generic construction + projection (the collection-slot
           spelling is the pinned decline; the GENERIC-TUPLE slot computes because dup inlines/
           monomorphizes into the handler scope). Two applications see stepping state: 100 then
           100+k → 210 at k=10, 200 at k=0. A homing that lost the closure through the generic slot
           would false-reject; a projection that shared ONE application's frame would double-count
           the first state.")
  (input  (do
        (effect E (op get (-> Unit Int64)))
        (def (dup x) (tuple x x))
        (def (main (: k Int64))
          (handle E 100
            ((get (u) s (resume s (+ s k))))
            (do
              (def p (dup (fn ((: _y Int64)) (E.get))))
              (+ ((. p 0) 1) ((. p 1) 2)))))
        (export main)))
  (call   main (: 10 Int64)) (output (: 210 Int64))
  (call   main (: 0 Int64)) (output (: 200 Int64)))
(case "a do-def-bound perform inside a recursive fn called under a handle declines cleanly (specializer floor, not a mangled-name CDZ0201)"
  (doc    "SAFE FLOOR (v-effects, 0d2afb083). A recursive function whose body do-def-binds a performed
           operation — `(do (def scaled (Env.scale i)) (check-all (- i 1) …))` — used to fail CDZ0201
           `check-all#eff2 has no body`: the effect specializer RESERVED a body:None spec def and memoized
           the mangled name before threading the body, and on the do-def-bound-perform body the thread
           returned None (unthreadable) leaving the reserved bodyless def + memo, so the recursive self-call
           resolved to it and leaked the internal `#eff` name. The fix declines UNCODED naming the base fn
           ('the recursive function check-all performs a discharged operation in a form the effect
           specializer does not yet handle') — a clean not-yet-reducible floor, not a mangled CDZ0201.
           Computing the value (110) needs a later body-clone specialization increment. The inline-
           expression twin `(check-all (- i 1) (+ bad (Env.scale i)))` already compiles; this is the
           do-def-bound-perform-in-a-recursive-fn seam, distinct from the straight-line do-def and F1 seams.")
  (input  (do
        (effect Env (op scale (-> Int64 Int64)))
        (def (check-all (: i Int64) (: bad Int64))
          (if (= i 0)
              bad
              (do
                (def scaled (Env.scale i))
                (check-all (- i 1) (+ bad scaled)))))
        (def (main (: k Int64))
          (handle Env k
            ((scale (v) s (resume (* v s) s)))
            (check-all 10 0)))
        (export main)))
  (declines))

(case "Qty arithmetic on the handler state binder via an arm-local def threads and runs"
  (doc    "The working perimeter of the Qty-stateful-handler pattern (v-cad/notebook @param): a handler
           whose state is a `Qty` (a unit-carrying scalar) advances its state by arithmetic on the state
           binder `s`. The arm-local-def form — `(do (def t (+ s s)) (resume t s))` — type-checks and runs
           (`s` keeps its `(Qty Int64 meter)` type through the def, so `(+ s s)` is Qty+Qty). `main` performs
           once and reads `Qty.value`: `2·21 = 42`. Pins the semantics an INLINE resume-slot `(+ s s)` must
           match (that inline form currently false-rejects — see the flip-pin held for v-inference).")
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((step (_u) s (do (def t (+ s s)) (resume t s))))
                (Qty.value (Acc.step))))
            (export main)))
  (call   main (: 21 Int64))
  (output (: 42 Int64)))

(case "a Qty handler state advances via Qty.value / re-wrap in the next-state slot"
  (doc    "The value-then-rewrap workaround: the next-state slot advances by unwrapping the Qty to its
           scalar (`Qty.value s`), computing, and re-wrapping (`Qty.of (* … 2) meter`). Two performs read
           `Qty.value` of each and sum: seed 5 → first step advances state to 10, the two performed results
           are 5 and 10 → `Qty.value 5 + Qty.value 10`… (a+2a) reads = 15 at a=5. Pins the re-wrap path as a
           valid Qty-state advance alongside the arm-local-def form.")
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((step (_u) s (resume s (Qty.of (* (Qty.value s) 2) (Unit.base #"meter")))))
                (Qty.value (+ (Acc.step) (Acc.step)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 15 Int64)))

(case "Qty arithmetic INLINE in a handler resume VALUE slot keeps the state binder's Qty type (#44 fix)"
  (doc    "The inline resume-slot companion to the arm-local-def perimeter above: `(resume (+ s s) s)`
           resumes with the doubled state VALUE directly, matching `(do (def t (+ s s)) (resume t s))` — so
           `main` reads `Qty.value` of `2·21 = 42`. This inline form used to FALSE-REJECT CDZ0201: the state
           binder `s` was inferred at type `Any` inside the resume-slot `(+ s s)`, so `(+ Any Any)` missed
           the Qty-aware arith arm and defaulted to Int64, then the slot check reported Int64 vs the
           `(Qty Int64 meter)` state type. The fix (v-inference, 520142726) types the state binder from the
           seed via `handle_arm_state_ty`, so the inline arith sees `s : (Qty Int64 meter)` and threads
           correctly. A genuine seed/next-state type mismatch still rejects CDZ0201 — no soundness weakening.")
  (input  (do
            (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((step (_u) s (resume (+ s s) s)))
                (Qty.value (Acc.step))))
            (export main)))
  (call   main (: 21 Int64))
  (output (: 42 Int64)))

(case "a TWO-site arm over a Qty state gates on the unwrapped magnitude"
  (doc    "The two-site refold face of the Qty-state family above: the arm's branch condition reads the
           op ARGUMENT (`(> v 10)`), the pass path folds the unwrapped state into its answer and
           advances by re-wrap (`(Qty.of (+ (Qty.value s) 1) (Unit.base #\"meter\"))`), the fail path holds. feed 20 →
           20+5 = 25 (state 6m), feed 3 → 0, feed 30 → 30+6 = 36 → 2536. Pins the served multi-site
           family over a UNIT-CARRYING state — the erased-unit representation must survive the
           refold's continuation rebuild on both branches.")
  (input  (do
            (effect Acc (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle Acc (Qty.of a (Unit.base #"meter"))
                ((feed (v) s (if (> v 10) (resume (+ v (Qty.value s)) (Qty.of (+ (Qty.value s) 1) (Unit.base #"meter"))) (resume 0 s))))
                (+ (* 100 (Acc.feed 20)) (+ (* 10 (Acc.feed 3)) (Acc.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2536 Int64)))

(case "a Float64 handler state advances fractionally through a two-site arm"
  (doc    "The f64 face of the state-representation matrix: the state walks 0.5 → 0.75 → 1.0 (+0.25
           per pass — dyadic fractions, no rounding ambiguity) while the arm gates on the integer op
           argument. feed 20 → 20, feed 5 → 0, feed 30 → 30 → 2030. Pins that the refold's state
           threading carries an f64 slot through the continuation rebuild.")
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St 0.5
                ((feed (v) s (if (> v 10) (resume v (+ s 0.25)) (resume 0 s))))
                (+ (* 100 (St.feed 20)) (+ (* 10 (St.feed a)) (St.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2030 Int64)))

(case "a Float64 op RESULT crosses resume and float state arithmetic is observed by comparison"
  (doc    "The f64 op-result face: the arm resumes the CURRENT float state and halves it (`(* s 0.5)`),
           so two reads yield 0.5 and 0.25 — all dyadic, exact — and the body observes their sum via
           `(> … 0.7)` → 1. Pins float values crossing the resume boundary and float next-state
           arithmetic, with a comparison consumer (float equality is not the corpus idiom).")
  (input  (do
            (effect St (op frac (-> Unit Float64)))
            (def (main (: a Int64))
              (handle St 0.5
                ((frac (u) s (resume s (* s 0.5))))
                (if (> (+ (St.frac) (St.frac)) 0.7) 1 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))

(case "a TUPLE handler state — the arm destructures, branches, and rebuilds both slots"
  (doc    "The product-state twin-accumulator: `(tuple lo hi)` where the two-site arm (a match around
           the if) routes fails into `lo` (accumulating) and passes into `hi` (counting +1 per pass,
           resumed with `v + hi`); the trailing `sum` reads both. step 20 → 120 (hi 101), step 3 → 0
           (lo 3), sum → 104 → 120 + 0 + 104000 = 104120. Both slots must survive every rebuild —
           a dropped or swapped slot breaks the place-value sum.")
  (input  (do
            (effect St (op step (-> Int64 Int64)) (op sum (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (tuple 0 100)
                ((step (v) s
                  (match s
                    ((tuple lo hi)
                      (if (> v 10)
                        (resume (+ v hi) (tuple lo (+ hi 1)))
                        (resume lo (tuple (+ lo v) hi))))))
                 (sum (u) s (match s ((tuple lo hi) (resume (+ lo hi) s)))))
                (+ (St.step 20) (+ (St.step n) (* 1000 (St.sum))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 104120 Int64)))

(case "a tuple-of-HEAP state — every dispatch grows BOTH components in one rebuild"
  (doc    "The heap escalation of the tuple state: `(tuple (list) map)` where each `rec` pushes onto
           the List AND inserts into the Map in one rebuild, answering the pre-push length; the
           trailing `stats` reads across both components. rec 7 → 0 ([7], {…,7:14}), rec 5 → 1
           ([7 5], {…,5:10}), stats → 2 + m[7]=14 = 16 → 0 + 1 + 1600 = 1601. The twin-accumulator
           idiom as ONE tuple-valued state (the do-threaded twin-accumulator pins spell it as two
           separate bindings).")
  (input  (do
            (effect St (op rec (-> Int64 Int64)) (op stats (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (tuple (list) (Map.insert Map.empty 0 0))
                ((rec (v) s
                  (match s
                    ((tuple xs m)
                      (resume (List.len xs) (tuple (List.push xs v) (Map.insert m v (* v 2)))))))
                 (stats (u) s
                  (match s
                    ((tuple xs m)
                      (resume (+ (List.len xs) (match (Map.lookup m 7) ((Some x) x) ((None _u) 0))) s)))))
                (+ (St.rec 7) (+ (St.rec n) (* 100 (St.stats))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1601 Int64)))

(case "a compile-time Char comparison folds beside performs (the runtime-Char boundary's served face)"
  (doc    "`(String.scalar-at \\\"hello\\\" 1)` with BOTH operands compile-time constants yields
           `(Some #\\e)` at compile time, and the `(= c #\\e)` comparison folds to 1 beside a live
           perform: 5 + 1 = 6. The RUNTIME face is a by-design boundary: a runtime Char has no
           representation yet, so `String.scalar-at` over a runtime string/index rejects (the
           diagnostic names the alternatives — `String.at` for an `(Option String)` one-scalar read,
           `Bytes.at` over `String.to-bytes` for ASCII scans); an effect crossing inherits that
           boundary unchanged.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1))))
                (+ (St.bump)
                   (match (String.scalar-at "hello" 1)
                     ((Some c) (if (= c #\e) 1 0))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

(case "a TWO-site arm over a BigInt state (heap-scalar state through the refold)"
  (doc    "The heap-scalar sibling of the Qty two-site pin above: the state is a BigInt (`(BigInt.of
           a)`), advanced with BigInt arithmetic (`(+ s 1N)`) on the pass path and read back through
           `Int64.of` in the resume value. Same walk: 25, 0, 36 → 2536. With the Qty face, pins that
           the refold's state threading is representation-agnostic — boxed heap scalars behave as
           machine ints do.")
  (input  (do
            (effect Acc (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle Acc (BigInt.of a)
                ((feed (v) s (if (> v 10) (resume (+ v (Int64.of s)) (+ s 1N)) (resume 0 s))))
                (+ (* 100 (Acc.feed 20)) (+ (* 10 (Acc.feed 3)) (Acc.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2536 Int64)))

(case "a two-site arm over a STRING state (concat on pass, hold on fail) with a trailing length reader"
  (doc    "The string-accumulator idiom through the refold: the pass branch grows the state by
           `String.concat s \\\"x\\\"`, the fail branch holds, and a trailing single-site `len` op reads
           `String.byte-len` (served under the arm-shape rule — trailing single-site after multi-site
           performs). tag 20 → 20 (s \\\"x\\\"), tag 5 → 0, tag 30 → 30 (s \\\"xx\\\"), len → 2 →
           20 + 0 + 30 + 200 = 250. Completes the state-representation matrix's string face.")
  (input  (do
            (effect St (op tag (-> Int64 Int64)) (op len (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St ""
                ((tag (v) s (if (> v 10) (resume v (String.concat s "x")) (resume 0 s)))
                 (len (u) s (resume (String.byte-len s) s)))
                (+ (St.tag 20) (+ (St.tag n) (+ (St.tag 30) (* 100 (St.len)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 250 Int64)))

(case "a two-site arm over a BYTES state (bin-built seed, concat growth) with a trailing size reader"
  (doc    "The binary-accumulator twin of the string face above: the seed is `(bin (u8 0))` (one byte),
           the pass branch appends `(bin (u8 (UInt8.wrap v)))` via `Bytes.concat`, and a trailing
           single-site `size` reads `Bytes.len`. feed 20 → 20 (2 bytes), feed 5 → 0, feed 30 → 30
           (3 bytes), size → 3 → 20 + 0 + 30 + 300 = 350. Composes the bin-construction idiom with
           the refold + the trailing-single-site rule.")
  (input  (do
            (effect St (op feed (-> Int64 Int64)) (op size (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (bin (u8 0))
                ((feed (v) s (if (> v 10) (resume v (Bytes.concat s (bin (u8 (UInt8.wrap v))))) (resume 0 s)))
                 (size (u) s (resume (Bytes.len s) s)))
                (+ (St.feed 20) (+ (St.feed n) (+ (St.feed 30) (* 100 (St.size)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 350 Int64)))

(case "a SET state dedup accumulator — the condition PROBES the state, the pass branch inserts"
  (doc    "Three rule faces in one realistic handler: the branch condition READS the heap state
           (`Set.contains s v` — a membership probe), the pass branch advances it (`Set.insert`), and
           a trailing single-site `card` reads the cardinality. add 7 → new (7, {7}), add 3 → new
           (3, {7 3}), add 7 → DUP (0, held), card → 2 → 7 + 3 + 0 + 200 = 210. The seen-set dedup
           idiom whole; a re-served insert or a stale membership read breaks the checksum.")
  (input  (do
            (effect St (op add (-> Int64 Int64)) (op card (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Set.of (list))
                ((add (v) s (if (Set.contains s v) (resume 0 s) (resume v (Set.insert s v))))
                 (card (u) s (resume (Set.len s) s)))
                (+ (St.add 7) (+ (St.add n) (+ (St.add 7) (* 100 (St.card)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 210 Int64)))

(case "a SYMBOL-keyed Map state routes hits to different keys (route-table accumulator)"
  (doc    "Interned-symbol keys × the refold: the two-site arm routes each hit to a DIFFERENT symbol
           key — passes accumulate under `(Symbol.of \\\"a\\\")`, fails under `(Symbol.of \\\"b\\\")` — and a
           trailing total reads BOTH keys back. hit 20 → a=20, hit 3 → b=3, total → 23 → 20 + 0 +
           2300 = 2320. The symbol lookups must intern to the SAME keys the arm's inserts used across
           dispatches.")
  (input  (do
            (effect St (op hit (-> Int64 Int64)) (op total (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Map.insert (Map.insert Map.empty (Symbol.of "a") 0) (Symbol.of "b") 0)
                ((hit (v) s
                  (if (> v 10)
                    (resume v (Map.insert s (Symbol.of "a") v))
                    (resume 0 (Map.insert s (Symbol.of "b") v))))
                 (total (u) s
                  (resume (+ (match (Map.lookup s (Symbol.of "a")) ((Some x) x) ((None _u) -1))
                            (match (Map.lookup s (Symbol.of "b")) ((Some y) y) ((None _u) -1))) s)))
                (+ (St.hit 20) (+ (St.hit n) (* 100 (St.total))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2320 Int64)))

(case "a NESTED Map-of-Map state — the arm updates the inner map through the outer per dispatch"
  (doc    "Two-level heap-state rebuild through the fold: every `put` reads the inner map through the
           outer (`Map.lookup s 1`), accumulates into it, and rebuilds BOTH levels (`Map.insert s 1
           (Map.insert inner 2 …)`); the trailing `get` traverses the nesting. inner[2] starts 10:
           put 5 → 15, put 7 → 22, get → 22 → 5 + 7 + 2200 = 2212. Two-level CHAMP persistence per
           dispatch — a dropped rebuild level or a stale inner read breaks the accumulation.")
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Map.insert Map.empty 1 (Map.insert Map.empty 2 10))
                ((put (v) s
                  (resume v
                    (match (Map.lookup s 1)
                      ((Some inner) (Map.insert s 1 (Map.insert inner 2 (+ v (match (Map.lookup inner 2) ((Some x) x) ((None _u) 0))))))
                      ((None _u) s))))
                 (get (u) s
                  (resume (match (Map.lookup s 1)
                            ((Some inner) (match (Map.lookup inner 2) ((Some x) x) ((None _u) -1)))
                            ((None _u) -2)) s)))
                (+ (St.put n) (+ (St.put 7) (* 100 (St.get))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2212 Int64)))

(case "triple-nested same-op performs — each argument is the inner perform's result"
  (doc    "Perform-in-ARGUMENT-position chains freely with a single-site arm: `(St.dbl (St.dbl
           (St.dbl n)))` doubles thrice with the state counting dispatches — 5 → 10 → 20 → 40. (A
           MULTI-site perform in another multi-site perform's argument declines: the argument
           dispatch is inherently mid-chain, the arm-shape mixing rule's interleaved case.)")
  (input  (do
            (effect St (op dbl (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((dbl (v) s (resume (* v 2) (+ s 1))))
                (St.dbl (St.dbl (St.dbl n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 40 Int64)))

(case "a multi-site arm's resume value routes through a pure helper call"
  (doc    "The pass branch's resume VALUE is `(triple v)` — a named pure helper call — rather than an
           inline expression: sift 20 → 60 (s 1), sift 5 → 0, sift 30 → 90 (s 2) → 150. Pins that the
           refold's branch-value rebuild tolerates a function call in the value slot (the helper is
           effect-free; its call folds as opaque pure computation inside the arm).")
  (input  (do
            (effect St (op sift (-> Int64 Int64)))
            (def (triple (: x Int64)) (* x 3))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume (triple v) (+ s 1)) (resume 0 s))))
                (+ (St.sift 20) (+ (St.sift n) (St.sift 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64)))

(case "a RECORD handler state — the arm projects one field and rebuilds the record"
  (doc    "Record-typed handler state (the state-family pins cover scalar/sum/collection/closure; this
           adds the product): the arm answers with a projection (`(. s count)`) and advances by
           REBUILDING the record with one field bumped and the other carried (`(record (count …+1)
           (tag (. s tag)))`). hit → 5 (count becomes 6), hit → 6 → 56. A dropped or reordered field
           in the rebuild breaks the checksum.")
  (input  (do
            (effect St (op hit (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (record (count n) (tag 7))
                ((hit (_u) s (resume (. s count) (record (count (+ (. s count) 1)) (tag (. s tag))))))
                (+ (* 10 (St.hit)) (St.hit))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))

(case "an OPEN-ROW helper projects from the record state INSIDE the arm"
  (doc    "Row polymorphism under the fold: `get-count` is typed OPEN over extra fields (`(. r count)`
           only), and the arm calls it on the state — a row-poly instantiation happening inside
           handler machinery. Same walk as the direct-projection pin above (56); the helper must
           instantiate at the state's record shape when the arm body is folded, not resolve against
           a stale row.")
  (input  (do
            (effect St (op hit (-> Unit Int64)))
            (def (get-count r) (. r count))
            (def (main (: n Int64))
              (handle St (record (count n) (tag 7))
                ((hit (_u) s (resume (get-count s) (record (count (+ (get-count s) 1)) (tag 9)))))
                (+ (* 10 (St.hit)) (St.hit))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))

(case "a two-site arm gates on a PROJECTED field of the record state (rate limiter)"
  (doc    "The refold × record-projection composition — the rate-limiter idiom whole: the branch
           condition compares two projected fields (`(< (. s hits) (. s cap))`), the pass path
           rebuilds with hits+1, the fail path answers -1 and holds. cap 2: feed 7 → 7 (hits 1),
           feed 8 → 8 (hits 2), feed 9 → -1 (limit) → 779. The projection in CONDITION position
           and the rebuild across both branches compose with the two-hole refold.")
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (record (hits 0) (cap n))
                ((feed (v) s
                  (if (< (. s hits) (. s cap))
                    (resume v (record (hits (+ (. s hits) 1)) (cap (. s cap))))
                    (resume -1 s))))
                (+ (* 100 (St.feed 7)) (+ (* 10 (St.feed 8)) (St.feed 9)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 779 Int64)))

(case "a two-site arm branches on SYMBOL equality of the state"
  (doc    "The interned-symbol face of the served multi-site family: the condition is `(= s (Symbol.of
           \"loud\"))` — an O(1) symbol identity check against the state binder. Both reads take the
           loud path at seed \"loud\": 500 + 300 = 800. Extends the refold's condition coverage to
           Symbol-typed states (the mode-dispatch handler idiom's read half).")
  (input  (do
            (effect St (op emit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "loud")
                ((emit (v) s (if (= s (Symbol.of "loud")) (resume (* v 100) s) (resume v s))))
                (+ (St.emit n) (St.emit 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 800 Int64)))

(case "a mode-REPLACING arm swaps the Symbol state; a conditional-value arm reads it"
  (doc    "The write half of the mode-dispatch idiom: `flip` REPLACES the Symbol state (`loud` →
           `quiet`) while `emit` answers conditionally on it (single resume site — the branch is in
           the VALUE, not around the resume). emit 5 loud → 500, flip → 0 (mode quiet), emit 3
           quiet → 3 → 503. Pins a symbol-valued state transition observed by a later dispatch.
           (The two-site-branch × mode-replacing composition in ONE handler still declines — the
           open second-op family.)")
  (input  (do
            (effect St (op emit (-> Int64 Int64)) (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "loud")
                ((emit (v) s (resume (if (= s (Symbol.of "loud")) (* v 100) v) s))
                 (flip (u) s (resume 0 (Symbol.of "quiet"))))
                (+ (St.emit n) (+ (St.flip) (St.emit 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 503 Int64)))

; --- Effects micro-family: positional op-arg binding, per-slot ctor-element ordering, the SET
; constructor's dedup-of-results, stateful nested-handler isolation, and a growing Bytes-rope
; state. Each is the ASYMMETRIC/stateful sibling of an existing symmetric/stateless pin.

(case "a 3-arg effect op binds its operands POSITIONALLY — an argument-order swap is caught"
  (doc    "The positional sibling of the commutative add3 pin: the arm encodes 100x+10y+z so ANY operand permutation diverges (add3's a+b+c passes under a swap); runtime a in the second perform + stepping state.")
  (input  (do
            (effect Calc (op mix (-> Int64 Int64 Int64 Int64)))
            (def (main (: a Int64))
              (handle Calc 1000
                ((mix (x y z) s (resume (+ (* 100 x) (+ (* 10 y) (+ z s))) (+ s 1))))
                (+ (Calc.mix 1 2 3) (Calc.mix a 5 6))))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 2580 Int64)))

(case "performs as list-literal elements land at their POSITIONS in perform order"
  (doc    "The per-slot sibling of the sum-read list-ctor pin: xs[0] and xs[2] read INDIVIDUALLY (positional weights) + a post-build tick proves state continuity — ticks k,k+1,k+2 land at slots 0,1,2; a right-to-left fill or shared temp diverges. The handler stays live AROUND the reads.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ctr k
                ((tick (_u) s (resume s (+ s 1))))
                (do
                  (def xs (list (Ctr.tick) (Ctr.tick) (Ctr.tick)))
                  (+ (* 100 (match (List.at xs 0) ((Option.Some v) v) ((Option.None _u) -1)))
                     (+ (* 10 (match (List.at xs 2) ((Option.Some v) v) ((Option.None _u) -1)))
                        (Ctr.tick))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 578 Int64)))

(case "performs as Set.of elements build the set with stepping state and dedup applies to the RESULTS"
  (doc    "SET completes the compound-constructor perform-threading family (tuple/list/record/map) and adds what none have: the ctor DEDUPS its element results — CHAMP hash on resumed values. Stepping arm (+2): 3 distinct {k,k+2,k+4}, len 3 + membership.")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ctr k
                ((tick (_u) s (resume s (+ s 2))))
                (do
                  (def s (Set.of (list (Ctr.tick) (Ctr.tick) (Ctr.tick))))
                  (+ (* 100 (Set.len s))
                     (+ (* 10 (if (Set.contains s k) 1 0))
                        (if (Set.contains s (+ k 4)) 1 0))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 311 Int64)))

(case "a STALLED counter makes all Set.of perform results collide to a singleton"
  (doc    "The collide face: (resume s s) stalls the state so all three performs return k — the set must collapse to a singleton (a builder assuming distinct element slots miscounts).")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ctr k
                ((tick (_u) s (resume s s)))
                (do
                  (def s (Set.of (list (Ctr.tick) (Ctr.tick) (Ctr.tick))))
                  (+ (* 10 (Set.len s))
                     (if (Set.contains s k) 1 0)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 11 Int64)))

(case "nested SAME-effect handlers isolate STATE — inner performs never advance the outer counter"
  (doc    "The STATEFUL sibling of the stateless region-partition pin: outer +1 / inner +2 strides, reads BEFORE/INSIDE/AFTER the inner region. The after-read is load-bearing: outer resumes at its own single advance (101), not advanced by inner performs (103/105) nor reset by inner teardown (100). Runtime inner seed.")
  (input  (do
            (effect E (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (handle E 100
                ((get (_u) s (resume s (+ s 1))))
                (+ (E.get)
                   (+ (handle E (* k 10)
                        ((get (_u) s (resume s (+ s 2))))
                        (+ (E.get) (E.get)))
                      (E.get)))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 303 Int64)))

(case "a Bytes ROPE handler state GROWS by bin-append per perform and each resume reads the prior length"
  (doc    "BYTES joins the handler-state type family (scalar/tuple/record/Map/Set): the wire-accumulator idiom — each put APPENDS (bin (u8 v)) via Bytes.concat (deeper rope per perform), resume value = PRIOR length (1,2,3 -> 123). The state rope must survive perform round-trips with its seam structure intact.")
  (input  (do
            (effect Acc (op put (-> UInt8 Int64)))
            (def (main (: a Int64) (: b Int64))
              (handle Acc (Bytes.of (list 9))
                ((put (v) s (resume (Bytes.len s) (Bytes.concat s (bin (u8 v))))))
                (do
                  (def l1 (Acc.put (UInt8.wrap a)))
                  (def l2 (Acc.put (UInt8.wrap b)))
                  (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
            (export main)))
  (call   main (: 1 Int64) (: 2 Int64))
  (output (: 123 Int64)))

(case "an Ast.List handler STATE accumulates a node per perform and each resume reads the prior length"
  (doc    "AST joins the handler-state type family (scalar/tuple/record/Map/Set/Bytes-rope): the
           template-accumulator idiom — each put pushes `(Ast.Int (BigInt.of v))` onto the `Ast.List`
           state's element list (rebuilt via the Ast.List ctor, matched back open per perform), resume
           value = the PRIOR List.len (0,1,2 -> 12). A recursive-sum state with BigInt-boxed leaves must
           survive the perform round-trips exactly as the flat state shapes do.")
  (input  (do
            (effect Acc (op put (-> Int64 Int64)))
            (def (main (: a Int64) (: b Int64))
              (handle Acc (Ast.List (list))
                ((put (v) s (match s
                              ((Ast.List els)
                                (resume (List.len els)
                                        (Ast.List (List.push els (Ast.Int (BigInt.of v))))))
                              (_ (resume -100 s)))))
                (do
                  (def l1 (Acc.put a))
                  (def l2 (Acc.put b))
                  (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
            (export main)))
  (call   main (: 1 Int64) (: 2 Int64))
  (output (: 12 Int64)))

(case "an OPTION handler state TOGGLES its variant per perform"
  (doc    "A sum-typed state whose VARIANT changes per dispatch (the state-family pins hold their variant
           fixed): the arm matches its own state and flips it — `Some v` resumes v and stores None; `None`
           resumes -1 and stores `Some 99`. Three performs walk Some 7 → None → Some 99, and the place-value
           checksum (100·7 + 10·(−1) + 99 = 789) breaks if any transition writes the wrong variant or a
           stale payload. The state slot must carry a full sum value whose constructor differs call-to-call.")
  (input  (do
            (effect St (op tog (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Option.Some n)
                ((tog (u) s
                  (match s
                    ((Option.Some v) (resume v (Option.None)))
                    ((Option.None)   (resume -1 (Option.Some 99))))))
                (+ (* 100 (St.tog)) (+ (* 10 (St.tog)) (St.tog)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 789 Int64)))

(case "an Option-of-HEAP handler state transitions None to Some and grows the payload"
  (doc    "The heap composition of the variant-transitioning state: `(Option (List Int64))` starts None;
           the first feed creates `Some (list v)`, later feeds push into the existing payload, and each
           resume reports the PRIOR length (0, 1, 2 → 12). The transition allocates the list inside the
           arm on the None path and grows it on the Some path — a sum-wrapped heap payload whose variant
           AND contents both evolve across performs.")
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St (Option.None)
                ((feed (v) s
                  (match s
                    ((Option.None) (resume 0 (Option.Some (list v))))
                    ((Option.Some xs) (resume (List.len xs) (Option.Some (List.push xs v)))))))
                (+ (* 100 (St.feed a)) (+ (* 10 (St.feed (+ a 1))) (St.feed (+ a 2))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 12 Int64)))

(case "a RESULT handler state is matched per variant with one resume per arm (Ok accumulates, Err echoes)"
  (doc    "The Result sibling of the Option variant pins above: the state is `(Result Int64 Int64)` and the
           arm matches it — the Ok path accumulates `(resume (+ acc v) (Ok (+ acc v)))`, the Err path
           echoes its payload unchanged. Each match ARM has exactly ONE resume site, so the shape folds
           (the latching Ok→Err transition, whose if branches on the accumulator READ FROM THE STATE
           binder inside one arm, is the pinned condition-reads-state decline). This run stays on the Ok
           path: 3, 3+4=7, 7+2=9 → 379. Pins per-variant dispatch over a two-payload sum state where both
           constructors carry data (Option's None carries none).")
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Result.Ok 0)
                ((add (v) s
                  (match s
                    ((Result.Ok acc) (resume (+ acc v) (Result.Ok (+ acc v))))
                    ((Result.Err e) (resume e (Result.Err e))))))
                (+ (* 100 (St.add n)) (+ (* 10 (St.add 4)) (St.add 2)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 379 Int64)))

(case "an Ast node as the effect OP ARGUMENT is destructured by the arm"
  (doc    "The op-ARGUMENT direction of the Ast crossing (the resume-value case above is the arm→body
           direction; this is body→arm): the program performs `(Sink.eat (Ast.List …))` and the ARM
           pattern-matches the node, resuming with its element count — a 2-element list then an empty
           one (2 + 0 = 2). The op-arg marshal must carry the recursive sum into the arm intact, the
           analyzer-handler idiom (a handler that inspects syntax it is handed).")
  (input  (do
            (effect Sink (op eat (-> Ast Int64)))
            (def (main (: n Int64))
              (handle Sink 0
                ((eat (a) s (match a
                              ((Ast.List els) (resume (List.len els) s))
                              (_ (resume -1 s)))))
                (+ (Sink.eat (Ast.List (list (Ast.Int (BigInt.of n)) (Ast.Name "x"))))
                   (Sink.eat (Ast.List (list))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 2 Int64)))

(case "a FOUR-arg effect op binds positionally (place-value checksum)"
  (doc    "The arity extension of the 3-arg positional pin: four operands at four place values —
           `(Calc.mix4 5 2 3 4)` → 1000·5 + 100·2 + 10·3 + 4 = 5234. Any operand permutation or
           marshal-slot mixup at arity 4 diverges.")
  (input  (do
            (effect Calc (op mix4 (-> Int64 Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle Calc 0
                ((mix4 (a b c d) s (resume (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d))) s)))
                (Calc.mix4 n 2 3 4)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5234 Int64)))

(case "a HETEROGENEOUS 4-arg op (Int64/String/Bool/Int64) marshals every type to its arm binder"
  (doc    "The mixed-signature face of the op-arg marshal (the positional pins are homogeneous-Int):
           one op carries a scalar id, a heap String, a Bool flag, and a scalar score, and the arm
           consumes each per its type — id scaled (500), name measured (3), flag branched (1000),
           score added (7) → 1510. Real host-effect signatures are exactly this shape.")
  (input  (do
            (effect Rec (op entry (-> Int64 String Bool Int64 Int64)))
            (def (main (: n Int64))
              (handle Rec 0
                ((entry (id name flag score) s
                  (resume (+ (* 100 id) (+ (String.byte-len name) (+ (if flag 1000 0) score))) s)))
                (Rec.entry n "abc" true 7)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1510 Int64)))

(case "a RECORD op result crosses resume; the body projects both fields"
  (doc    "A STRUCTURAL record in the op signature (records-as-STATE are pinned; the crossing was not —
           structural products marshal differently from nominal sums and positional tuples): the arm
           resumes `(record (x (* id 2)) (y (+ id 1)))` and the body projects both fields — 10 + 6 =
           16. The field layout must survive the resume marshal.")
  (input  (do
            (effect St (op fetch (-> Int64 (Record (x Int64) (y Int64)))))
            (def (main (: n Int64))
              (handle St 0
                ((fetch (id) s (resume (record (x (* id 2)) (y (+ id 1))) s)))
                (let ((r (St.fetch n)))
                  (+ (. r x) (. r y)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64)))

(case "a RECORD as op ARGUMENT — the arm projects the fields it is handed"
  (doc    "The argument direction of the record crossing: the body hands `(record (hits n) (misses 3))`
           to the op and the ARM projects both fields — 10·5 − 3 = 47. With the result-direction pin
           above and the record-STATE pins, structural records cover all three effect positions.")
  (input  (do
            (effect St (op score (-> (Record (hits Int64) (misses Int64)) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((score (r) s (resume (- (* (. r hits) 10) (. r misses)) s)))
                (St.score (record (hits n) (misses 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 47 Int64)))

(case "a record is built and consumed inside the arm (structural product per dispatch)"
  (doc    "The arm-internal face: the record never crosses the boundary — the arm builds it from the
           op argument, binds it via a match, and resumes the projected sum (10 + 6 = 16). Pins
           structural-product construction + projection inside folded arm bodies.")
  (input  (do
            (effect St (op fetch (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((fetch (id) s
                  (resume (match (record (x (* id 2)) (y (+ id 1)))
                            (r (+ (. r x) (. r y)))) s)))
                (St.fetch n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64)))

(case "a heterogeneous TUPLE op result (String, Int64) crosses resume and destructures"
  (doc    "The result-direction twin of the heterogeneous-args pin above: the arm resumes a
           `(Tuple String Int64)` — a heap String and a scalar in one payload — and the body
           destructures it: byte-len \\\"row\\\" + 5·10 = 53. Both marshal directions now carry
           mixed-type payloads.")
  (input  (do
            (effect Rec (op fetch (-> Int64 (Tuple String Int64))))
            (def (main (: n Int64))
              (handle Rec 0
                ((fetch (id) s (resume (tuple "row" (* id 10)) (+ s 1))))
                (match (Rec.fetch n)
                  ((tuple name score) (+ (String.byte-len name) score)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 53 Int64)))

(case "a USER-SUM op result (Status) crosses resume; the body matches per variant"
  (doc    "User-DECLARED sums through the effect boundary (Option/Result crossings are pinned; nominal
           sums go through the general type marshal, not the built-in paths): the op's result type is
           `Status` (a payload variant + an empty one), the arm resumes either, and the body matches —
           poll 20 → Active 40, poll 5 → Idle → -1 → 39. The marshal must carry the nominal tag and
           payload across resume.")
  (input  (do
            (effect St (op poll (-> Int64 Status)))
            (type Status (Active Int64) (Idle))
            (def (main (: n Int64))
              (handle St 0
                ((poll (v) s (resume (if (> v 10) (Status.Active (* v 2)) (Status.Idle)) (+ s 1))))
                (+ (match (St.poll 20) ((Status.Active x) x) ((Status.Idle) -1))
                   (match (St.poll n) ((Status.Active x) x) ((Status.Idle) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 39 Int64)))

(case "a user SUM is constructed AND matched inside the arm (per-dispatch classification)"
  (doc    "The arm-internal face: the sum never crosses the boundary — the arm builds a `Status` from
           the op argument, matches it immediately, and resumes the scalar classification (20 pass →
           20, 5 fail → 0 → 20). Pins nominal-sum construction + dispatch working inside folded arm
           bodies.")
  (input  (do
            (effect St (op classify (-> Int64 Int64)))
            (type Status (Active Int64) (Idle))
            (def (main (: n Int64))
              (handle St 0
                ((classify (v) s
                  (resume (match (if (> v 10) (Status.Active v) (Status.Idle))
                            ((Status.Active x) x)
                            ((Status.Idle) 0)) s)))
                (+ (St.classify 20) (St.classify n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20 Int64)))

(case "a GENERIC user sum ((Box Int64)) as op result — nominal tag + instantiated payload cross resume"
  (doc    "The generic extension of the monomorphic Status crossing above: `(Box a)` instantiated at
           Int64 — wrap 20 → Full 60, wrap 5 → Empty → -1 → 59. The instantiated payload slot and the
           nominal tag both survive the resume marshal.")
  (input  (do
            (effect St (op wrap (-> Int64 (Box Int64))))
            (type (Box a) (Full a) (Empty))
            (def (main (: n Int64))
              (handle St 0
                ((wrap (v) s (resume (if (> v 10) (Box.Full (* v 3)) (Box.Empty)) (+ s 1))))
                (+ (match (St.wrap 20) ((Box.Full x) x) ((Box.Empty) -1))
                   (match (St.wrap n) ((Box.Full x) x) ((Box.Empty) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 59 Int64)))

(case "a generic sum instantiated at a HEAP payload ((Box (List Int64))) crosses resume"
  (doc    "The heap-instantiation face: the generic payload slot holds a LIST — grab 20 → Full [20 20
           20] (len 3), grab 5 → Empty → -1 → 2. Instantiation-specific layout (a heap pointer in the
           payload slot) through the resume marshal.")
  (input  (do
            (effect St (op grab (-> Int64 (Box (List Int64)))))
            (type (Box a) (Full a) (Empty))
            (def (main (: n Int64))
              (handle St 0
                ((grab (v) s (resume (if (> v 10) (Box.Full (list v v v)) (Box.Empty)) s)))
                (+ (match (St.grab 20) ((Box.Full xs) (List.len xs)) ((Box.Empty) -1))
                   (match (St.grab n) ((Box.Full xs) (List.len xs)) ((Box.Empty) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64)))

(case "a RECURSIVE user sum (Tree) crosses resume; the body folds it"
  (doc    "A user-declared RECURSIVE sum (payloads contain the sum itself) as the op result: the arm
           builds a 3-leaf tree from the op argument and the body folds it with a recursive helper —
           Node(Leaf 5, Node(Leaf 10, Leaf 1)) → 16. Distinct from the built-in Ast crossings: a user
           recursive type goes through the general nominal marshal.")
  (input  (do
            (effect St (op grow (-> Int64 Tree)))
            (type Tree (Leaf Int64) (Node Tree Tree))
            (def (sum-tree t)
              (match t
                ((Tree.Leaf v) v)
                ((Tree.Node l r) (+ (sum-tree l) (sum-tree r)))))
            (def (main (: n Int64))
              (handle St 0
                ((grow (v) s (resume (Tree.Node (Tree.Leaf v) (Tree.Node (Tree.Leaf (* v 2)) (Tree.Leaf 1))) s)))
                (sum-tree (St.grow n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64)))

(case "a recursive sum as op ARGUMENT — the arm dispatches on its shape"
  (doc    "The argument direction: the body hands trees to the op and the ARM pattern-dispatches on
           the shape it receives — a Leaf answers its payload (5), a Node answers 99 → 104. The op-arg
           marshal carries the recursive structure into the arm intact.")
  (input  (do
            (effect St (op weigh (-> Tree Int64)))
            (type Tree (Leaf Int64) (Node Tree Tree))
            (def (main (: n Int64))
              (handle St 0
                ((weigh (t) s
                  (resume (match t
                            ((Tree.Leaf v) v)
                            ((Tree.Node l r) 99)) s)))
                (+ (St.weigh (Tree.Leaf n)) (St.weigh (Tree.Node (Tree.Leaf 1) (Tree.Leaf 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 104 Int64)))

(case "a USER sum as handler state — a countdown mode machine (Fast k -> Slow)"
  (doc    "The state-slot completion of the user-sum ladder (Option/Result STATE pins exist; a
           user-declared sum state did not): `Mode` starts `Fast n`, the arm decrements the payload
           per dispatch and TRANSITIONS variants at zero — Fast 2 → Fast 1 → Fast 0 → Slow, resuming
           2, 1, 0 → 210. Nominal-sum layout in the state slot, with a variant transition mid-run.")
  (input  (do
            (effect St (op step (-> Unit Int64)))
            (type Mode (Fast Int64) (Slow))
            (def (main (: n Int64))
              (handle St (Mode.Fast n)
                ((step (u) s
                  (match s
                    ((Mode.Fast k) (if (> k 0) (resume k (Mode.Fast (- k 1))) (resume 0 (Mode.Slow))))
                    ((Mode.Slow) (resume -1 (Mode.Slow))))))
                (+ (* 100 (St.step)) (+ (* 10 (St.step)) (St.step)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 210 Int64)))

(case "three DISCARDED performs on a do-spine still advance the state"
  (doc    "Effect-only evaluation: three `(St.bump)` results are discarded on the do-spine — evaluated
           purely for their state effect — and the trailing peek reads the fully-advanced 8 (seed 5,
           three advances). A fold that elided 'unused' performs would skip the advances and read 5.
           The most imperative idiom in the language, pinned standalone.")
  (input  (do
            (effect St (op bump (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1)))
                 (peek (u) s (resume s s)))
                (do
                  (St.bump)
                  (St.bump)
                  (St.bump)
                  (St.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))

(case "NEGATIVE values thread every effect slot — state, argument, and result stay signed"
  (doc    "A sign-extension slip in any marshal (i64 truncation, a wrong-width reload) surfaces only
           on negative values; this case drives negatives through EVERY slot at once so each marshal
           path has a signed witness: seed −100, op arg −5, resume values −105/−107, next-state
           arithmetic −110 → −212. The signed-values face of the effect machinery.")
  (input  (do
            (effect St (op dip (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (- 0 100)
                ((dip (v) s (resume (+ v s) (- s 10))))
                (+ (St.dip (- 0 n)) (St.dip 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -212 Int64)))

(case "Int64 MAX threads the handler state intact (representation at the boundary)"
  (doc    "The state slot must carry a full i64: the seed is Int64 MAX, the first peek reads it back
           EXACTLY, the state decrements, and the second peek reads MAX−1 → 1. Any narrower
           intermediate representation (or a float round-trip) corrupts the boundary value.")
  (input  (do
            (effect St (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 9223372036854775807
                ((peek (u) s (resume s (- s 1))))
                (if (= (St.peek) 9223372036854775807) (if (= (St.peek) 9223372036854775806) 1 2) 3)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))

(case "ZERO threads every effect slot (zero seed, zero args, zero results)"
  (doc    "The degenerate-value face: a zero state seed, a zero LITERAL argument, a zero COMPUTED
           argument (`(- n n)`), and zero resume values — all thread and the +7 tail lands the
           checksum. Zeros matter because a wrong slot read aliases with an uninitialized cell; a
           positive checksum cannot distinguish 0-the-value from 0-the-missing-write.")
  (input  (do
            (effect St (op echo (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((echo (v) s (resume (+ v s) s)))
                (+ (St.echo 0) (+ (St.echo (- n n)) 7))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))

(case "a handle whose body NEVER performs is exactly its body (zero dispatches)"
  (doc    "The zero-dispatch degenerate: the effect is declared, the handler installed with a live
           arm — and the body never performs, so the handle is exactly `(* n 2)` = 10. The fold's
           fully-eliminated path: the handler apparatus must vanish without residue (no stray seed
           evaluation effects, no frame cost observable in the value).")
  (input  (do
            (effect St (op never (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 100
                ((never (u) s (resume s s)))
                (* n 2)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))

(case "a pure closure-driver call beside a perform in one handle body"
  (doc    "A generic driver (`apply-twice`, a lambda-lifted closure call on every backend, incl. the
           async EnvClosure emit) runs BESIDE a perform in one handle body: the driver computes
           10 + 12 = 22 purely, the bump reads 100 → 122. The closure machinery and the effect fold
           coexist in one body on all three targets.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (apply-twice f (: a Int64)) (+ (f a) (f (+ a 1))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (+ (apply-twice (fn ((: x Int64)) (* x 2)) n) (St.bump))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 122 Int64)))

(case "a closure-driver result feeds a perform's ARGUMENT"
  (doc    "The dataflow composition: the driver's computed 22 flows INTO the effect dispatch as the
           op argument, and the arm scales it (220). The closure-call result must be fully reduced
           before the dispatch marshals it.")
  (input  (do
            (effect St (op log (-> Int64 Int64)))
            (def (apply-twice f (: a Int64)) (+ (f a) (f (+ a 1))))
            (def (main (: n Int64))
              (handle St 0
                ((log (v) s (resume (* v 10) (+ s 1))))
                (St.log (apply-twice (fn ((: x Int64)) (* x 2)) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 220 Int64)))

(case "a bin PATTERN destructures a perform-result Bytes (the wire round-trip through a handler)"
  (doc    "binary-matching × effects, the codec round-trip: the ARM constructs framed Bytes from its
           state (`(bin (u16 …) (u8 7))`) and the BODY destructures the perform result with a bin
           PATTERN, recovering both fields — hi = 258 (the seed, big-endian u16), lo = 7 → 258 + 700 =
           958. The protocol-handler idiom: a handler serves wire bytes, the caller parses them; the
           pattern must read exactly the bytes the arm's construction laid down.")
  (input  (do
            (effect St (op fetch (-> Unit Bytes)))
            (def (main (: n Int64))
              (handle St n
                ((fetch (u) s (resume (bin (u16 (UInt16.wrap s)) (u8 7)) (+ s 1))))
                (match (St.fetch)
                  ((bin (u16 hi) (u8 lo)) (+ (Int64.of hi) (* 100 (Int64.of lo))))
                  (_ -1))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 958 Int64)))

(case "a bin-pattern arm binds a parsed byte and PERFORMS again with it (parse-then-act)"
  (doc    "The pipeline composition of the bin-pattern crossing above: the match arm's binder `b` —
           established by the bin PATTERN over the perform result — feeds a SECOND perform
           (`(St.log (Int64.of b))`), whose arm multiplies by 10: fetch serves byte 5, log answers
           50. Pins that a bin-pattern binding flows into a subsequent dispatch correctly — the
           parse-then-act shape every wire-protocol reducer uses.")
  (input  (do
            (effect St (op fetch (-> Unit Bytes)) (op log (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((fetch (u) s (resume (bin (u8 (UInt8.wrap s))) (+ s 1)))
                 (log (v) s (resume (* v 10) s)))
                (match (St.fetch)
                  ((bin (u8 b)) (St.log (Int64.of b)))
                  (_ -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))

(case "eval of a quoted expression folds INSIDE a handle body beside performs"
  (doc    "quote/eval × effects coexistence: `(eval (quote (+ 1 2)))` — a COMPILE-TIME eval of a
           compile-time-visible quote — sits between two performs and folds to its 3 while the
           performs discharge normally: 5 + 3 + 6 = 14. Both features rewrite the handle body (the
           eval reconstructs-and-compiles, the fold discharges performs); this pins that they
           compose. (An arm-built RUNTIME Ast fed to eval is rejected by design with CDZ0101, whose
           message begins: `eval` executes only a COMPILE-TIME-VISIBLE AST construction (a
           `(quote …)` or literal `Ast.*`): it reconstructs the source that AST denotes and compiles
           it. — the compiler builds and analyzes AST but does not run a dynamically-built one.)")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (St.next) (+ (eval (quote (+ 1 2))) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 14 Int64)))

(case "a @requires-guarded def PERFORMS in its body — contract check and effect specialization compose"
  (doc    "@requires × effects: the enforcement rewrite injects `(if (>= x 0) BODY (trap …))` at
           body-entry AND the body performs `(St.bump)`, so the def is both contract-checked and
           effect-specialized. Two satisfying calls observe the advancing state: f 5 → 5+100 = 105
           (s → 101), f 2 → 2+101 = 103 → 208. The two body rewrites (contract if-wrap, effect
           specialization) must not fight over the same def.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (+ (f n) (f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 208 Int64)))

(case "a VIOLATED @requires traps at body-entry BEFORE the body's perform fires"
  (doc    "The enforcement-order guarantee of the pair, made OBSERVABLE by an ABORTIVE arm: `(f -5)`
           violates `(>= x 0)`, and the handler's `bump` arm never resumes — it ABORTS the handle with
           999. So if the rewrite order were wrong (perform first, check second), `(St.bump)` would
           dispatch, the abort would win, and the program would RETURN 999 instead of trapping — this
           case would fail its trap expectation. The trap firing proves the injected check runs at
           body-entry, before the perform. (A resuming arm could not distinguish the two orders —
           both end in the same trap.)")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s 999))
                (f (- 0 n))))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))

(case "a satisfied @ensures on a performing def passes the effectful result through"
  (doc    "The postcondition side of the contract × effects pair (the @requires pins are above): the
           `@ensures (>= ret 100)` wrapper checks the EFFECT-DERIVED result — f 5 = 5 + bump(100) =
           105, satisfying — and passes it through unchanged (105). Single call; the multi-call face
           is the open let-perform × branching-condition fold bug tracked separately.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))

(case "a VIOLATED @ensures on a performing def traps at body-exit"
  (doc    "The violated face: `(>= ret 1000)` fails against the effect-derived 105, so the injected
           body-exit check traps — postcondition enforcement works when the result came through a
           resume rather than pure arithmetic.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (ensures (>= ret 1000)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))

(case "a STACKED @requires + @ensures contract on a performing def threads all three layers"
  (doc    "The full Hoare triple × effects: precondition check (satisfied), effectful body (the bump
           resumes 100), postcondition check (105 >= 100, satisfied) — pre + perform + post all thread
           and the contract-checked effectful result returns (105).")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0))
            (@ (ensures (>= ret 100))
               (def (f (: x Int64)) (+ x (St.bump)))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))

(case "the stacked contract's PRE fires through the @ensures layer BEFORE the perform (abortive observer)"
  (doc    "Composes the descent-through-annotation-layers guarantee (the @requires reaches the def
           through the intervening @ensures wrapper) with OBSERVABLE check-before-perform ordering:
           the bump arm ABORTS with 999, so if the perform ran before the (violated) precondition
           check, the program would return 999 — the trap proves the pre fires first, through the
           stack.")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (requires (>= x 0))
            (@ (ensures (>= ret 100))
               (def (f (: x Int64)) (+ x (St.bump)))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s 999))
                (f (- 0 n))))
            (export main)))
  (call   main (: 5 Int64))
  (trap   "unreachable"))

(case "a @test-tier @ensures on a performing def runs and checks"
  (doc    "The three-layer annotation stack — `@test` → `@ensures` → a performing def — threads: the
           test-tier postcondition checks the effect-derived 105 and passes it through. Completes the
           annotation-tier crossings with effects (plain and @test-tier contracts both compose).")
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ test (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump)))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))

(case "an @invariant newtype is constructed from PERFORM results"
  (doc    "The invariant-type × effects cross: a `Percent` (0..100 @invariant) built from two perform
           results at advancing state — mk(42) and mk(43), both satisfying — unwrapped and summed
           (85). The invariant machinery (the synthesized checker) and the effect fold compose when
           the checked value originates from a handler.")
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (mk (: v Int64)) (Percent.Pct v))
            (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (main (: n Int64))
              (handle St 42
                ((next (u) s (resume s (+ s 1))))
                (+ (unwrap (mk (St.next))) (unwrap (mk (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64)))

(case "an ABORTING arm derives its answer from an Ast op-arg and discards the continuation"
  (doc    "The abort composition of the Ast op-arg: the arm never resumes, so the handle's value IS the
           arm's — `(Int64.of b)` on the node's BigInt payload plus the state — and the continuation's
           pending `(+ 500 …)` is DISCARDED (1000 + 25 + 0 = 1025, not 1525). Composes the Ast crossing
           with the abort shape: the payload extraction must happen on the discard path exactly as on
           the resume path.")
  (input  (do
            (effect Halt (op stop (-> Ast Int64)))
            (def (main (: n Int64))
              (+ 1000
                 (handle Halt 0
                   ((stop (a) s (match a ((Ast.Int b) (+ (Int64.of b) s)) (_ -1))))
                   (+ 500 (Halt.stop (Ast.Int (BigInt.of n)))))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 1025 Int64)))

; --- Effects/try leftovers: the closure-resume factory, the response-transforming adapter
; interposer (wasm-first; rust todo rides the host-effect family), and the try-composition
; faces (const folds through arm/ctor positions; runtime operand + failing fold are pending
; bricks graded todo). ---

(case "a handler arm resumes with a CLOSURE (in a tuple) capturing the op param and state"
  (doc    "The factory-through-effect idiom (the deferred-resume pins wrap `resume` in a thunk; here the RESUME VALUE is a fresh closure over the arm's OWN binders): the body calls the returned fn AFTER the frame resumed, so base/s must live in the closure env, not the dead arm frame. The direct fn-typed op result curried-flattens, so the closure crosses in (Tuple (-> Int64 Int64) Int64) — also pinning a mixed closure+scalar payload through resume.")
  (input  (do
            (effect Mk (op make (-> Int64 (Tuple (-> Int64 Int64) Int64))))
            (def (main (: k Int64))
              (handle Mk 10
                ((make (base) s (resume (tuple (fn ((: x Int64)) (+ x (+ base s))) base) s)))
                (match (Mk.make k)
                  ((tuple f b) (+ (f 1) (+ (f 2) b))))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 38 Int64)))

(case "the handler STATE is a CLOSURE the arm replaces with one capturing the perform-time op argument"
  (doc    "Strategy-as-state: the state slot carries a closure the arm APPLIES for its answer and REPLACES
           per dispatch — and the replacement `(fn (x) (+ x v))` closes over the op argument `v`, so the
           state closes over RUNTIME data from the previous perform. Seed is the identity: eval 4 → 4,
           next state adds 4; eval 3 → 7 → 407. A stale strategy (804→wrong) or a late-bound capture
           breaks the checksum. The closure sits in the STATE slot proper — the closure-in-tuple pin
           above crosses one through a RESUME value instead.")
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (resume (f v) (fn ((: x Int64)) (+ x v)))))
                (+ (* 100 (St.eval n)) (St.eval 3))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 407 Int64)))

(case "a CLOSURE state whose body performs an OUTER effect when the arm applies it"
  (doc    "The cross-frame face of strategy-as-state: the inner handler's closure state has `(+ x
           (Aux.base))` as its body, so APPLYING the state inside the inner arm performs the OUTER
           effect — the application crosses a live handler frame. Aux seeds 50 and advances per read:
           eval 4 → 4+50 = 54 (Aux → 51), eval 3 → 3+51 = 54 → 5454. Pins that a perform fired from a
           closure applied inside another handler's ARM homes against the outer frame and its advance
           is observed by the next application.")
  (input  (do
            (effect Aux (op base (-> Unit Int64)))
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Aux 50
                ((base (u) b (resume b (+ b 1))))
                (handle St (fn ((: x Int64)) (+ x (Aux.base)))
                  ((eval (v) f (resume (f v) f)))
                  (+ (* 100 (St.eval n)) (St.eval 3)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 5454 Int64)))

(case "TWO closures minted by the same op at DIFFERENT states each keep their own snapshot"
  (doc    "The aliasing probe of the closure-factory pin above: `mk` is performed twice with a state
           advance between, so two distinct closures exist whose envs captured DIFFERENT values of the
           same state binder. `f` captures 5, `bump` advances to 15, `g` captures 15; `(f 0)`=5 and
           `(g 0)`=15 → 515. A shared or late-bound environment gives 1515 (both see the advance) or
           15 (both see the seed) — the checksum separates all three worlds. Each resume-crossed
           closure env must be a private snapshot, not a reference into the handler frame.")
  (input  (do
            (effect St (op mk (-> Unit (Tuple (-> Int64 Int64) Int64))) (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (tuple (fn ((: x Int64)) (+ x s)) 0) s))
                 (bump (u) s (resume s (+ s 10))))
                (match (St.mk)
                  ((tuple f _z)
                    (do (St.bump)
                        (match (St.mk)
                          ((tuple g _w) (+ (* 100 (f 0)) (g 0)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 515 Int64)))

(case "a CLOSURE as the op ARGUMENT — the arm applies the caller's strategy to its own state"
  (doc    "The body→arm direction of the closure crossing (the factory pins are arm→body): the op's
           PARAMETER type is `(-> Int64 Int64)` and the body passes a different lambda per perform. The
           arm answers `(f s)` — the caller's strategy applied to the handler's CURRENT state — and
           advances. `(*3)` at s=5 → 15, then `(+7)` at s=6 → 13 → 1513. Unlike the result direction
           (which curried-flattens and needs the tuple crossing), a fn-typed op ARGUMENT is direct.
           Pins the visitor idiom: the handler owns the data, callers send the computation.")
  (input  (do
            (effect Ap (op app (-> (-> Int64 Int64) Int64)))
            (def (main (: n Int64))
              (handle Ap n
                ((app (f) s (resume (f s) (+ s 1))))
                (+ (* 100 (Ap.app (fn ((: x Int64)) (* x 3)))) (Ap.app (fn ((: x Int64)) (+ x 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1513 Int64)))

(case "an ABORTING arm applies the CLOSURE STATE for its final answer"
  (doc    "The abort face of strategy-as-state (the resumptive faces are pinned above): `(fire (v) f
           (f v))` never resumes, so the handle's value IS the strategy applied to the op argument —
           `(*7)` at 6 → 42 — and the pending continuation `(+ 500 …)` is DISCARDED (1000 + 42 = 1042,
           not 1542). The closure state must be applicable on the abort path exactly as on the resume
           path. (A closure IN the abort value itself — minted by the aborting arm and applied after —
           is the not-yet-reducible non-tail-resume boundary; applying the state to produce a SCALAR
           answer folds.)")
  (input  (do
            (effect St (op fire (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ 1000
                (handle St (fn ((: x Int64)) (* x 7))
                  ((fire (v) f (f v)))
                  (+ 500 (St.fire n)))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 1042 Int64)))

(case "a WRAP-composing closure state (the replacement captures the PREVIOUS closure) folds at two dispatches"
  (doc    "The self-referential face of strategy-as-state: each dispatch REPLACES the closure state with a
           lambda that wraps the previous one — `(fn (x) (* (f x) 2))`, so the env chain grows per perform
           (id → ×2). Two performs fold: eval 5 → 5 (state becomes ×2), eval 3 → 6 → 56. At THREE
           dispatches this shape declines (the unboundedly-growing env chain is the honest boundary) —
           this case pins the served depth, its scalar-capturing sibling below pins that the boundary is
           the CLOSURE-chain env specifically.")
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (resume (f v) (fn ((: x Int64)) (* (f x) 2)))))
                (+ (* 10 (St.eval n)) (St.eval 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))

(case "a SCALAR-capturing closure-state replacement folds at three dispatches (no env chain)"
  (doc    "The discriminating sibling of the wrap-composing pin above: here the replacement captures only
           the APPLIED RESULT `(let ((r (f v))) … (fn (x) (+ x r)))` — a scalar — so no closure-chain env
           grows and THREE dispatches fold where the wrap-composing shape declines. id at 5 → r=5 (state
           x+5), f(3)=8 → r=8 (state x+8), f(4)=12 → 592. Together the pair pins the exact boundary:
           what the replacement CAPTURES (prior closure vs scalar) decides the fold, not replacement or
           runtime capture per se.")
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (let ((r (f v))) (resume r (fn ((: x Int64)) (+ x r))))))
                (+ (* 100 (St.eval n)) (+ (* 10 (St.eval 3)) (St.eval 4)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 592 Int64)))

(case "a LIST of strategies is walked recursively, each applied to a fresh perform result"
  (doc    "Closures as COLLECTION elements consumed under a handler: `apply-all` destructures a list of
           three lambdas and applies each to its own `(Cnt.next)` — the counter walks 5,6,7 while the
           strategy changes per slot (×10, +100, id → 50 + 106 + 7 = 163). Each element's application
           and each perform must pair up in order; a re-served perform or a slot skew breaks the sum.
           (The INDEXED lookup route — `List.at`/`Map.lookup` yielding Option-of-closure with a
           perform-computed key — is a separate known wasm-codegen defect; this direct-destructure
           walk is the served face.)")
  (input  (do
            (effect Cnt (op next (-> Unit Int64)))
            (def (apply-all fs)
              (match fs
                ((list) 0)
                ((list f .. r) (+ (f (Cnt.next)) (apply-all r)))))
            (def (main (: n Int64))
              (handle Cnt n
                ((next (u) s (resume s (+ s 1))))
                (apply-all (list (fn ((: x Int64)) (* x 10)) (fn ((: x Int64)) (+ x 100)) (fn ((: x Int64)) x)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 163 Int64)))

(case "an interposing handler TRANSFORMS the host response before resuming (offset adapter)"
  (doc    "The ADAPTER sibling of the observe interposer (:866 counts + forwards unchanged): the arm transforms the host response before resuming (+1000 each; 30+40 → 2070 — a dropped transform gives 70, a double-apply 3070).")
  (input  (do
            (effect ask (op get (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle ask unit
                  ((get (k) s (resume (+ (ask.get k) 1000) s)))
                  (+ (ask.get 3) (ask.get 4)))))
            (export main)))
  (host-responses (respond ask.get (: 30 Int64)) (respond ask.get (: 40 Int64)))
  (host-calls (call ask.get) (call ask.get))
  (output (: 2070 Int64)))

(case "an in-program two-site handler runs INSIDE a host block beside a host call"
  (doc    "The host-frame × in-program-frame MIX (wasm/rust; the rust-async lowering declines this
           composition — its baseline row is a todo, the interposer precedent): the host block's body
           holds a real host call AND a plain in-program handle side by side — `(+ (ask.get 3) (handle
           St 0 …))`. The host response (30) and the served two-site sift (5 pass at s=0, 1 fail → 5)
           sum to 35. Pins that an in-program handler's fold is undisturbed by a sibling host effect
           in the same body — the frames are independent.")
  (input  (do
            (effect ask (op get (-> Int64 Int64)))
            (effect St (op sift (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (+ (ask.get 3)
                   (handle St 0
                     ((sift (v) s (if (> v 1) (resume v (+ s 1)) (resume 0 s))))
                     (+ (St.sift 5) (St.sift 1))))))
            (export main)))
  (host-responses (respond ask.get (: 30 Int64)))
  (host-calls (call ask.get))
  (output (: 35 Int64)))

(case "a constant-try helper folds and its result feeds a handler arm's resume"
  (doc    "try × handler-arm composition (the effect pins keep performs on the spine, try in the body): a CONST succeeding try in a helper folds through and feeds the arm's resume.")
  (input  (do
            (effect Ask (op ask (-> Unit Int64)))
            (def (get)
              (do
                (def v (try (Some 7)))
                (Some (+ v 1))))
            (def (main (: k Int64))
              (handle Ask unit
                ((ask (_u) s (resume (match (get) ((Option.Some v) v) ((Option.None _u) -5)) s)))
                (+ (Ask.ask unit) (* k 100))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 108 Int64)))

(case "a constant-try helper that fails feeds the None arm's fallback into resume"
  (doc    "The failing face: the fold does NOT elide the short-circuit — the boundary block/break emit is its own pending brick, so this grades todo until it lands (oracle 95).")
  (input  (do
            (effect Ask (op ask (-> Unit Int64)))
            (def (get)
              (do
                (def v (try (: (None unit) (Option Int64))))
                (Some (+ v 1))))
            (def (main (: k Int64))
              (handle Ask unit
                ((ask (_u) s (resume (match (get) ((Option.Some v) v) ((Option.None _u) -5)) s)))
                (+ (Ask.ask unit) (* k 100))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 95 Int64)))

(case "constant-succeeding tries as LIST-literal elements unwrap in place and the list builds"
  (doc    "try as a COLLECTION-constructor element (the parse-all idiom): const-succeeding tries unwrap in place and the list builds.")
  (input  (do
            (def (mk)
              (: (do
                (def xs (list (try (Some 1)) (try (Some 2))))
                (Some (List.len xs))) (Option Int64)))
            (def (main (: k Int64))
              (+ (* k 0) (match (mk) ((Option.Some v) v) ((Option.None _u) -1))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 2 Int64)))

(case "a runtime-operand try as a list element declines pending brick 3b"
  (doc    "The runtime face: a runtime operand mid-list hits the documented brick-3b boundary — flips when the runtime-try increment lands (oracle 3 at pick=1).")
  (input  (do
            (def (mk (: pick Int64))
              (: (do
                (def xs (list (try (Some 1)) (try (if (= pick 1) (Some 2) (: (None unit) (Option Int64)))) (try (Some 3))))
                (Some (List.len xs))) (Option Int64)))
            (def (main (: k Int64))
              (match (mk k) ((Option.Some v) v) ((Option.None _u) -1)))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 3 Int64)))

; --- Handler-composition perimeter: per-recursion-level handles, def-bound performs beside a
; recursive performing loop, and closure-handle slot swapping through tail calls. ---

(case "each recursion level installs its own handle around a def-bound perform"
  (doc    "The handle-INSIDE-the-recursion contrast to the specializer finding (handle OUTSIDE a
           recursive fn whose body def-binds a perform is the held CDZ0201): here every level of
           `nest` installs a FRESH handle seeded with its own n, def-binds the perform, and
           recurses NON-TAIL under it — each level reads its own seed and the sum n(n+1)/2 comes
           back through the nested handles (10 at k=4, 0 at k=0). The per-level handle means the
           perform's home is level-local — no cross-fn specialization needed, which is exactly why
           this computes while the outer-handle twin doesn't; pins the shape so the specializer fix
           doesn't disturb it.")
  (input  (do
        (effect E (op get (-> Unit Int64)))
        (def (nest (: n Int64))
          (if (= n 0)
              0
              (handle E n
                ((get (u) s (resume s s)))
                (do
                  (def v (E.get))
                  (+ v (nest (- n 1)))))))
        (def (main (: k Int64)) (nest k))
        (export main)))
  (call   main (: 4 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

(case "a def-bound perform composes with a recursive performing loop in one handle body"
  (doc    "The WORKING perimeter of the recursive-specializer gap (a def-bound perform INSIDE the
           recursive fn is the held finding): the handle body def-binds ONE perform (100k) and then
           runs the recursive loop whose performs sit in EXPRESSION position (6k) — the two shapes
           compose in one body (106k → 212 at k=2, 0 at k=0). Pins that the straight-line def-bound
           fix and the expression-position recursion each keep working while the specializer learns
           the combined shape — a fix that re-specialized the whole body wrongly breaks one addend.")
  (input  (do
        (effect Env (op scale (-> Int64 Int64)))
        (def (check-all (: i Int64) (: bad Int64))
          (if (= i 0) bad (check-all (- i 1) (+ bad (Env.scale i)))))
        (def (main (: k Int64))
          (handle Env k
            ((scale (v) s (resume (* v s) s)))
            (do
              (def first (Env.scale 100))
              (+ first (check-all 3 0)))))
        (export main)))
  (call   main (: 2 Int64)) (output (: 212 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

; --- Effects perimeter remainder: mutual-recursion state threading, argument-swap weaves,
; an inner arm consuming the outer handler's result, and an all-narrow-width handler. ---

(case "a MUTUALLY-recursive performing pair threads one handler state through both fns"
  (doc    "The mutual-recursion face of the effect specializer (self-recursion is pinned; the held
           finding is the def-bound variant): `ev` and `od` alternate, each performing `Cnt.tick`
           in expression position — the specializer must clone BOTH fns of the group coherently
           (ev#eff calling od#eff calling ev#eff) and the single state threads through the
           alternation (4 ticks: 4k+6 → 14 at k=2, 6 at k=0). A specializer that cloned only the
           entry fn (leaving od calling the UN-specialized ev) loses the homing mid-alternation.")
  (input  (do
        (effect Cnt (op tick (-> Unit Int64)))
        (def (ev (: n Int64)) (if (= n 0) 0 (+ (Cnt.tick) (od (- n 1)))))
        (def (od (: n Int64)) (if (= n 0) 0 (+ (Cnt.tick) (ev (- n 1)))))
        (def (main (: k Int64))
          (handle Cnt k
            ((tick (u) s (resume s (+ s 1))))
            (ev 4)))
        (export main)))
  (call   main (: 2 Int64)) (output (: 14 Int64))
  (call   main (: 0 Int64)) (output (: 6 Int64)))

(case "an argument swap weaves with perform results through a recursive handler body"
  (doc    "Permutation × effects: each tail call swaps the accumulators AND folds a fresh perform
           into the outgoing one — `(weave (- n 1) b (+ a (Src.next)))` interleaves the handler's
           stepping state (draws k, k+1, k+2) with the arg permutation ((0,2)→(2,3)→(3,6) → 36 at
           k=2; 12 at k=0). A lowering that sequenced the perform AFTER the slot assignment reads
           the swapped-in value into the wrong sum; one that re-performed per slot double-draws.
           The evaluation-order contract at the tail-call boundary under an active handler.")
  (input  (do
        (effect Src (op next (-> Unit Int64)))
        (def (weave (: n Int64) (: a Int64) (: b Int64))
          (if (= n 0)
              (+ (* 10 a) b)
              (weave (- n 1) b (+ a (Src.next)))))
        (def (main (: k Int64))
          (handle Src k
            ((next (u) s (resume s (+ s 1))))
            (weave 3 0 0)))
        (export main)))
  (call   main (: 2 Int64)) (output (: 36 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))

(case "an inner arm resumes with the OUTER handler's result while both states advance"
  (doc    "The dataflow-coupled cross-level perform (the :890 interpose pin performs the outer as a
           DISCARDED observation): `bump`'s arm performs `Log.put c` and resumes with the OUTER
           handler's RESULT — each bump's value is (inner count + outer accumulator) with BOTH
           states stepping between events: 100, 102, 104 → 306. A homing that discharged the arm's
           put against the wrong level (or a resume that captured the outer's state pre-put) shifts
           an addend; the value chain 100/102/104 encodes the exact interleaving of the two state
           threads.")
  (input  (do
        (effect Log (op put (-> Int64 Int64)))
        (effect Ctr (op bump (-> Unit Int64)))
        (def (main (: _mode Int64))
          (handle Log 100
            ((put (v) s (resume (+ v s) (+ s 1))))
            (handle Ctr 0
              ((bump (u) c (resume (Log.put c) (+ c 1))))
              (+ (Ctr.bump) (+ (Ctr.bump) (Ctr.bump))))))
        (export main)))
  (call   main (: 0 Int64)) (output (: 306 Int64)))

(case "an all-UInt8 handler (state, op result, arm arithmetic) computes at narrow width"
  (doc    "The width-CONSISTENT perimeter of the handler-state widening seam (the MIXED-width
           state/result shape is the interim clean decline, flip-pinned upstream): state seeded
           `(: 10 UInt8)`, op result UInt8, arm arithmetic all-narrow — the fold computes (10)
           with no widening required. Boxes the coming widening fix from the other side: a fix
           that widened EVERY handler state to Int64 would break this narrow-consistent shape's
           typing (the op result must stay UInt8 for the caller's Int64.of).")
  (input  (do
        (effect Src (op next (-> Unit UInt8)))
        (def (main (: x UInt8))
          (handle Src (: 10 UInt8)
            ((next (u) s (resume s (+ s x))))
            (Int64.of (Src.next))))
        (export main)))
  (call   main (: 5 UInt8)) (output (: 10 Int64)))

; --- SET handler state (grow + dedup) and the two-effect recursive loop with independent states. ---

(case "a SET handler state grows per perform and dedupes a repeated event"
  (doc    "The seen-set idiom as HANDLER STATE (scalar/map/record states are pinned; the set kind
           completes the collection-state family): each `note` resumes the PRE-insert len then
           inserts its event — distinct events step 0,1,2 (210 at k=5); a REPEATED event (k=10
           collides with the second note) dedupes so the third len stays 1 (110). A state threading
           that re-materialized the set per arm reads 0,0,0; one that inserted before resuming reads
           1,2,3 — both caught. The dedupe row also pins content-hashing through the threaded state
           (the same CHAMP the standalone set pins cover, here surviving perform/resume cycles).")
  (input  (do
        (effect Seen (op note (-> Int64 Int64)))
        (def (main (: k Int64))
          (handle Seen (Set.of (list))
            ((note (v) st (resume (Set.len st) (Set.insert st v))))
            (do
              (def a (Seen.note k))
              (def b (Seen.note 10))
              (def c (Seen.note k))
              (+ (* 100 c) (+ (* 10 b) a)))))
        (export main)))
  (call   main (: 5 Int64)) (output (: 210 Int64))
  (call   main (: 10 Int64)) (output (: 110 Int64)))

(case "a recursive loop performs to TWO handlers per iteration with independent states"
  (doc    "The two-effect specialization of the working recursion shape: each iteration performs
           `A.geta` AND `B.getb` — two different effects homing to two different handler levels —
           and both states step independently (A: k,k+1,k+2; B: 100,110,120 → 3k+333: 339 at k=2,
           333 at k=0). A specializer that keyed the recursive fn's effect-clone on ONE effect
           (dropping the second's homing) or shared the two state threads breaks an arithmetic
           progression. The multi-effect face of the per-iteration perform family.")
  (input  (do
        (effect A (op geta (-> Unit Int64)))
        (effect B (op getb (-> Unit Int64)))
        (def (loop (: n Int64) (: acc Int64))
          (if (= n 0) acc (loop (- n 1) (+ acc (+ (A.geta) (B.getb))))))
        (def (main (: k Int64))
          (handle A k
            ((geta (u) s (resume s (+ s 1))))
            (handle B 100
              ((getb (u) s (resume s (+ s 10))))
              (loop 3 0))))
        (export main)))
  (call   main (: 2 Int64)) (output (: 339 Int64))
  (call   main (: 0 Int64)) (output (: 333 Int64)))

; --- The heap-valued handler-state/op-result completions: Symbol results, BigInt state+result,
; Rational state with per-step normalization. ---

(case "a SYMBOL-returning effect op interns through the handler and results compare by content"
  (doc    "SYMBOL joins the op-result type family (the interner-service idiom): a (-> String Symbol) op whose arm interns via Symbol.of; results flow back through resume and a rope-arg intern equals a flat-arg intern by content, with the results also ORDERING content-lexicographically.")
  (input  (do
            (effect Reg (op intern (-> String Symbol)))
            (def (main (: k Int64))
              (handle Reg 0
                ((intern (s) c (resume (Symbol.of s) (+ c 1))))
                (do
                  (def s1 (Reg.intern (String.concat "sym" "A")))
                  (def s2 (Reg.intern "symA"))
                  (def s3 (Reg.intern (if (= k 1) "symB" "symA")))
                  (+ (* 100 (if (= s1 s2) 1 0))
                     (+ (* 10 (if (= s1 s3) 1 0))
                        (if (< s1 s3) 1 0))))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 101 Int64)))

(case "a BIGINT handler state multiplies per perform and each resume reads the prior product"
  (doc    "BIGINT joins the handler-state type family with state AND op-result both heap-numeric: the product grows per perform and each resume returns the PRIOR product (a=1, b=7, c=70 at k=7), and ALL THREE resume results are read via a digit encode (a*10000 + b*100 + c = 10770) so every resume is observed, the combined encode narrowed ONCE through checked Int64.of.")
  (input  (do
            (effect Acc (op grow (-> Int64 BigInt)))
            (def (main (: k Int64))
              (handle Acc (BigInt.of 1)
                ((grow (m) s (resume s (* s (BigInt.of m)))))
                (do
                  (def a (Acc.grow k))
                  (def b (Acc.grow 10))
                  (def c (Acc.grow 10))
                  (Int64.of (+ (* a (BigInt.of 10000)) (+ (* b (BigInt.of 100)) c))))))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 10770 Int64)))

(case "a RATIONAL handler state accumulates unit fractions exactly and resumes read the prior sum"
  (doc    "RATIONAL completes the heap-numeric state pair: 1/2+1/3 accumulates with gcd-normalization per perform round-trip (canonical 5/6 — an unnormalized 15/18 breaks the digit encode). Each resume returns the PRIOR sum — r0=0/1, r1=1/2, r2=5/6 at k=1 — and ALL THREE are read via a num/den digit encode (r0n r0d r1n r1d r2n r2d = 0 1 1 2 5 6 -> 11256) so every resume result is observed; the runtime arg defeats folding.")
  (input  (do
            (effect Avg (op sample (-> Int64 Rational)))
            (def (main (: k Int64))
              (handle Avg (Rational.of 0 1)
                ((sample (v) s (resume s (+ s (Rational.of 1 v)))))
                (do
                  (def r0 (Avg.sample 2))
                  (def r1 (Avg.sample 3))
                  (def r2 (Avg.sample (* k 6)))
                  (+ (* 100000 (Int64.of (Rational.numerator r0)))
                     (+ (* 10000 (Int64.of (Rational.denominator r0)))
                        (+ (* 1000 (Int64.of (Rational.numerator r1)))
                           (+ (* 100 (Int64.of (Rational.denominator r1)))
                              (+ (* 10 (Int64.of (Rational.numerator r2)))
                                 (Int64.of (Rational.denominator r2))))))))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 11256 Int64)))

; --- A heap-field record op argument with state accumulation. ---

(case "a record op argument with a HEAP field crosses the perform and its scalar field accumulates state"
  (doc    "The :4626 record-op-arg pin is all-scalar + stateless; this record carries a ROPE field beside the scalar through the perform AND the arm accumulates the scalar into STATE across two performs — the op-arg boxing keeps the heap handle beside the scalar while the state cell threads independently.")
  (input  (do
            (effect Db (op put (-> (Record (name String) (qty Int64)) Int64)))
            (def (main (: k Int64))
              (handle Db 0
                ((put (r) s (resume (+ s (. r qty)) (+ s (. r qty)))))
                (do
                  (def a (Db.put (record (name (String.concat "wid" "get")) (qty k))))
                  (def b (Db.put (record (name "bolt") (qty 10))))
                  (+ (* 100 a) b))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 515 Int64)))

; --- The classic effect idioms as a family: reader (ambient env), writer (pre-order trace),
; and gensym (fresh-symbol allocation) — each a recursive walk performing per node/element. ---

(case "a RECURSIVE tree evaluator resolves variables through a READER effect with a Map-state handler"
  (doc    "The reader-as-effect idiom (the explicit env-threading sibling landed in 05-compound): Var resolves via (Env.read name) from RECURSIVE walk frames at different depths; the handler owns the Map env as STATE; a String op-arg + scalar result cross per perform.")
  (input  (do
            (type Expr (Lit Int64) (Var String) (Add (Tuple Expr Expr)))
            (effect Env (op read (-> String Int64)))
            (def (eval-e (: e Expr))
              (match e
                ((Expr.Lit n) n)
                ((Expr.Var name) (Env.read name))
                ((Expr.Add (tuple a b)) (+ (eval-e a) (eval-e b)))))
            (def (main (: k Int64))
              (handle Env (Map.insert (Map.insert Map.empty "x" k) "y" 3)
                ((read (name) s (resume (Option.expect (Map.lookup s name) "unbound") s)))
                (eval-e (Expr.Add (tuple (Expr.Var "x") (Expr.Add (tuple (Expr.Var "y") (Expr.Lit 1))))))))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 14 Int64)))

(case "a WRITER effect accumulates a PRE-ORDER trace string during a recursive tree walk"
  (doc    "The writer idiom: each Add/Mul node logs its op tag BEFORE recursing (pre-order), the handler concats onto a String state, and dump reads the trace back beside the value ((2+3)*4: trace exactly \"*+\" — order-sensitive; the result triple-encodes value/len/content-eq).")
  (input  (do
            (type Expr (Lit Int64) (Add (Tuple Expr Expr)) (Mul (Tuple Expr Expr)))
            (effect Trace (op log (-> String Unit)) (op dump (-> Unit String)))
            (def (eval-t (: e Expr))
              (match e
                ((Expr.Lit n) n)
                ((Expr.Add (tuple a b)) (do (Trace.log "+") (+ (eval-t a) (eval-t b))))
                ((Expr.Mul (tuple a b)) (do (Trace.log "*") (* (eval-t a) (eval-t b))))))
            (def (main (: k Int64))
              (handle Trace ""
                ((log (tag) s (resume unit (String.concat s tag)))
                 (dump (_u) s (resume s s)))
                (do
                  (def v (eval-t (Expr.Mul (tuple (Expr.Add (tuple (Expr.Lit 2) (Expr.Lit k))) (Expr.Lit 4)))))
                  (def trace (Trace.dump))
                  (+ (* 100 v) (+ (* 10 (String.byte-len trace)) (if (= trace "*+") 1 0))))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 2021 Int64)))

(case "a GENSYM effect derives fresh symbols from a counter state — same base yields distinct symbols"
  (doc    "The allocator idiom at the SYMBOL level (the scalar-id gensym pin sums draws): the arm concats the string op-arg with a counter suffix and interns — the same base twice yields DISTINCT symbols (x_e/x_o); results accumulate into a list and compare against a literal intern, exercising Option<Symbol> slot equality.")
  (input  (do
            (effect Gensym (op fresh (-> String Symbol)))
            (def (rename-all (: xs (List String)) (: i Int64) (: out (List Symbol)))
              (match (List.at xs i)
                ((Option.Some base) (rename-all xs (+ i 1) (List.push out (Gensym.fresh base))))
                ((Option.None _u) out)))
            (def (main (: k Int64))
              (handle Gensym k
                ((fresh (base) n (resume (Symbol.of (String.concat base (if (= (% n 2) 0) "_e" "_o"))) (+ n 1))))
                (do
                  (def syms (rename-all (list "x" "x" "y") 0 (list)))
                  (+ (* 100 (List.len syms))
                     (+ (* 10 (if (= (List.at syms 0) (List.at syms 1)) 1 0))
                        (if (= (Option.expect (List.at syms 0) "s0") (Symbol.of "x_e")) 1 0))))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 301 Int64)))

; --- Uncalled-def faces of the handler validation walk (the resume-value/state pins above are
; CALLED-def shapes; these must reject whether or not the def is reached). Note the op-member
; face is CDZ0201 (closed-set row lookup), not CDZ0101. ---

(case "an unbound name in an uncalled def's handler-ARM resume argument is rejected"
  (doc    "The uncalled-def face of the resume-value scope check (the CALLED-def face is pinned above): the unbound name sits in a handler arm's resume inside a never-called def — a scope walk that descends def bodies but skips handler ARMS (dispatched code, not straight-line body) runs to 42. rcdzc rejects CDZ0101.")
  (input  (do
            (effect E (op get (-> Unit Int64)))
            (def (unused (: k Int64))
              (handle E k
                ((get (_u) s (resume no-such-name s)))
                (E.get)))
            (def (main) 42)
            (export main)))
  (error  CDZ0101))

(case "a HANDLE of an undeclared effect in an uncalled def is rejected"
  (doc    "The effect-NAME face: (handle NoSuchEffect ...) in a never-called def — the handle head must resolve to a declared effect whether or not the def is reached. rcdzc rejects CDZ0101.")
  (input  (do
            (def (unused (: k Int64))
              (handle NoSuchEffect k
                ((op (_u) s (resume 1 s)))
                1))
            (def (main) 42)
            (export main)))
  (error  CDZ0101))

(case "a PERFORM of an undeclared op on a declared effect in an uncalled def is rejected"
  (doc    "The op-MEMBER face: E is declared with op get, but the uncalled def performs (E.no-such-op) — the op lookup on a declared effect's row is a TYPE error (CDZ0201, the closed-set member check), distinct from the CDZ0101 unbound-name faces; it must fire in uncalled defs too. rcdzc rejects CDZ0201.")
  (input  (do
            (effect E (op get (-> Unit Int64)))
            (def (unused (: k Int64))
              (handle E k
                ((get (_u) s (resume 1 s)))
                (E.no-such-op)))
            (def (main) 42)
            (export main)))
  (error  CDZ0201))

(case "a stateful perform inside the arm of a fused match on a call result threads state once"
  (doc    "The fused-clone seam × handler state: the match scrutinee is a CALL result (`mk` — a
           fusion candidate whose arms clone into the callee's branches) and BOTH arms perform to a
           stateful handler, with a final perform reading the count. Exactly ONE arm perform runs
           (the taken arm's — branches are exclusive) and the value encodes the order: k=7 → Hi arm
           reads 0 → 70, final reads 1 → 71; k=2 → Lo arm → 200, final → 201. The hazard is the
           handler-frame threading through the CLONED payload-binder arms — a clone that re-seeded or
           lost the state advance breaks a digit. The fused companion of the arm-perform pins (whose
           scrutinees are scalars or performs, never a fused call-result sum).")
  (input  (do
            (effect Fresh (op next (-> Unit Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (handle Fresh 0 ((next (u) s (resume s (+ s 1))))
                (+ (match (mk k)
                     ((Hi h) (+ (* 10 h) (Fresh.next)))
                     ((Lo w) (+ (* 100 w) (Fresh.next))))
                   (Fresh.next))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 71 Int64))
  (call   main (: 2 Int64)) (output (: 201 Int64)))

(case "host calls issue only from the TAKEN arm of a fused match and in arm order"
  (doc    "Host delegation × the match-fusion seam: a fused match (call-result scrutinee) whose BOTH
           arms perform a host-delegated `io.get` — fusion clones each arm's host perform into the
           callee's branches, and the observable host-call sequence must stay EXACTLY ONE call (the
           taken arm's), consuming the single response: k=7 → Hi arm → 70+3=73 with [io.get] the
           whole trace. A clone that speculated the untaken arm's perform, or emitted the host call
           outside the branch dispatch, would issue TWO calls (the fixture rejects the trace) or
           consume the response in the wrong operand. Computes on ALL targets since the H1 rust
           host-call emit (b362d1414) — the no-arg integer-result shape is exactly H1's slice.")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (host (io)
                (match (mk k)
                  ((Hi h) (+ (* 10 h) (io.get)))
                  ((Lo w) (- (io.get) w)))))
            (export main)))
  (host-responses (respond io.get (: 3 Int64)))
  (host-calls (call io.get))
  (call   main (: 7 Int64)) (output (: 73 Int64)))

(case "an abortive perform in a fused-match arm carries the payload binder out and abandons the rest"
  (doc    "The abortive face of the fused-clone seam: the match scrutinee is a CALL result (fusion
           candidate), one arm's body performs `(Bail.bail (* h 10))` — the abort ARGUMENT reads the
           arm's SumPayload binder — abandoning a PENDING outer addition (+1000), while the other arm
           returns normally through it: k=7 → 70 (the +1000 abandoned); k=2 → 200+1000 = 1200. The
           fused clones must keep the abort's br-out-of-block correct in BOTH branch copies and route
           the payload binder into the abort argument (a clone that resumed the pending add with the
           arm value is the adv-52 class; a mis-bound payload reads garbage into the abort). NOTE this
           is a NON-TAIL abort (the arm feeds a pending +): the match-arm lowering handles what the
           if-branch non-tail abort doc note still defers.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (handle Bail 0 ((bail (n) s n))
                (+ (match (mk k)
                     ((Hi h) (Bail.bail (* h 10)))
                     ((Lo w) (* w 100)))
                   1000)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 70 Int64))
  (call   main (: 2 Int64)) (output (: 1200 Int64)))

(case "a host op returning Bool crosses the boundary and drives a branch"
  (doc    "The BOOL-result host boundary (H2, rcdzc 2ded4a5a9): a `(-> Unit Bool)` delegated host op
           crosses as its i32/i64 truthiness — the host supplies `true`, the guest reads it back and
           drives `(if (Env.flag) 100 200)` → 100. wasm reads i32→bool at the boundary; the rust
           backend emits `(crate::__cdz_host_<key>() != 0)`. The bool companion of the int-result host
           pins. Note the (host-calls …) fixture names the effect LOWERCASE (`env.flag`). Computes on
           wasm + rust; rust-async declines (H2 not yet on that target).")
  (input  (do
            (effect Env (op flag (-> Unit Bool)))
            (def (main)
              (host (Env)
                (if (Env.flag) 100 200)))
            (export main)))
  (host-responses (respond env.flag (: true Bool)))
  (host-calls (call env.flag))
  (output (: 100 Int64)))

(case "a let-bound host result captured by two escaping closures fires the host op once (adv-62)"
  (doc    "adv-62 (breaker, HIGH wasm soundness): a `let`-bound host-call result captured by TWO OR MORE
           ESCAPING closures must fire the host op EXACTLY ONCE — the `let`-bound `v` is shared by both
           closures. The callee `mk` returns `(tuple (fn (x) (+ v x)) (fn (x) (* v x)))` from inside a
           `(host (io) …)`; `main` destructures the tuple and calls both closures. The bug: `mk` β-inlines
           into the match scrutinee, the match folds to a single Leaf, and — because the inlined `io.get`
           copy lost its effect-op meta — the `scrutinee_reaches_host_perform` guard missed it, so the
           bare-body fold RE-EMITTED the whole `(host …)` block once per tuple binder → `io.get` fired
           TWICE → the second call had no recorded response and TRAPPED. FIX (v-effects): the guard now
           treats a `Resolved::Host` block in the scrutinee as reaching a host perform — a CONSERVATIVE
           OVER-APPROXIMATION (not every compiling host block performs: an op-reference-only body like
           `(host (E) (E.get))` compiles without a perform — see the rcdzc regression
           `a_host_with_too_many_operands_is_cdz0201`; over-reporting is safe here because it only keeps
           the wrapper, which merely materializes the scrutinee once), so the `MatchSum` wrapper is
           kept and the scrutinee materializes ONCE;
           and the wasm `Core::Let` emit maps the scalar value node → its slot so the two closures capture
           the SAME slot rather than re-lowering the host call. With io.get=21: `f(10)=21+10=31`,
           `g(100)=21*100=2100`, sum 2131 — and the (host-calls) fixture pins the SINGLE firing. rust
           declines the shape (its closure-in-tuple-through-host emit is a separate frontier).")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (mk)
              (host (io)
                (let ((v (io.get unit)))
                  (tuple (fn ((: x Int64)) (+ v x)) (fn ((: x Int64)) (* v x))))))
            (def (main)
              (match (mk) ((tuple f g) (+ (f 10) (g 100)))))
            (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (output (: 2131 Int64)))

(case "two DISTINCT let-bound host calls each captured by its own escaping closure fire once each in order (adv-62)"
  (doc    "The two-distinct-calls ORDER companion of the adv-62 single-call pin above (breaker escalation):
           `mk` binds `x = io.a` and `y = io.b` inside `(host (io) …)` and returns `(tuple (fn (n) (+ x n))
           (fn (n) (* y n)))`; `main` calls both. Each host op must fire EXACTLY ONCE and IN ORDER (io.a
           then io.b) — the per-closure re-fire bug (fixed #1528) would have fired each per capturing closure
           and/or lost the order. With io.a=3, io.b=5: `f(10)=3+10=13`, `g(100)=5*100=500`, sum 513, and the
           (host-calls io.a io.b) fixture pins BOTH the single firing of each AND their order — coverage the
           single-call pin can't give. rust/rust-async decline the closure-in-tuple-through-host shape (todo),
           as with the single-call case.")
  (input  (do
            (effect io (op a (-> Unit Int64)) (op b (-> Unit Int64)))
            (def (mk)
              (host (io)
                (let ((x (io.a unit)) (y (io.b unit)))
                  (tuple (fn ((: n Int64)) (+ x n)) (fn ((: n Int64)) (* y n))))))
            (def (main)
              (match (mk) ((tuple f g) (+ (f 10) (g 100)))))
            (export main)))
  (host-responses (respond io.a (: 3 Int64)) (respond io.b (: 5 Int64)))
  (host-calls (call io.a) (call io.b))
  (output (: 513 Int64)))

(case "a unit-result host op consumes its response row so the next value op reads its own (adv-65)"
  (doc    "adv-65 (breaker, HIGH wasm differential): a UNIT-result host op must CONSUME its response row,
           in order, so a later value-result op reads ITS OWN row — not the unit op's. `(host (io) (do
           (io.ping k) (+ (io.get k) k)))` with responses [io.ping=0, io.get=7], k=3 → 10 (io.get reads
           its own row 7: 7+3). The wasm host runner previously did NOT advance the response cursor on a
           unit-result op (it returns nothing), so `io.get` read io.ping's row 0 → 3 (silent wrong value);
           rust was correct (per-op response lists). FIX (v-effects, cdz-run): a unit-result op advances
           the cursor IFF the row at the cursor is FOR THIS OP (kebab-normalized match) — consuming a
           supplied row, but NOT skipping a value op's row for a pure observe-only unit op (H8's `log.emit`
           shape, which supplies no row). The response model is in-order consumption of ALL calls.")
  (input  (do
            (effect io (op ping (-> Int64 Unit)) (op get (-> Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (do (io.ping k)
                    (+ (io.get k) k))))
            (export main)))
  (host-responses (respond io.ping (: 0 Int64)) (respond io.get (: 7 Int64)))
  (host-calls (call io.ping) (call io.get))
  (call   main (: 3 Int64)) (output (: 10 Int64)))

(case "the unit-op response-cursor discriminator: a nonzero ping row is not misread by the later get (adv-65)"
  (doc    "adv-65 CURSOR DISCRIMINATOR: the same shape with io.ping's row = 99 (not 0) — if the unit op
           failed to consume its row, io.get would read 99 → 102; consuming it correctly gives io.get its
           own row 7 → 10. Pins that the fix READS THE RIGHT ROW, not merely that a zero happens to be
           harmless.")
  (input  (do
            (effect io (op ping (-> Int64 Unit)) (op get (-> Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (do (io.ping k)
                    (+ (io.get k) k))))
            (export main)))
  (host-responses (respond io.ping (: 99 Int64)) (respond io.get (: 7 Int64)))
  (host-calls (call io.ping) (call io.get))
  (call   main (: 3 Int64)) (output (: 10 Int64)))

(case "a host result captured by two closures stored in a RECORD fires the host op once (adv-62b)"
  (doc    "adv-62b (breaker→v-effects, HIGH wasm soundness): the RECORD-face sibling of adv-62 — a
           `let`-bound host-call result captured by two closures stored in a RECORD must fire the host op
           EXACTLY ONCE. `(def (mk) (host (io) (let ((v (io.get))) (record (f (fn (x) (+ v x))) (g (fn (x)
           (* v x)))))))` + `(def (main) (let ((r (mk))) (+ ((. r f) 10) ((. r g) 100))))` → 2131 (io.get=21
           once: f(10)=31 + g(100)=2100). The bug: `r`'s init `(mk)` reaches a host call THROUGH THE CALL,
           but `subtree_reaches_host_call`'s AST walk stopped at the `(mk)` node and missed the host call in
           mk's body → `r` was copy-propagated → each `(. r •)` re-inlined the `(host …)` block → io.get
           fired PER projection → the 2nd call had no recorded response and TRAPPED. FIX (v-effects): the
           `should_keep_binding` host-force-keep test now ALSO follows a CALL init into its inlined callee
           body (`core_reaches_host_call`, a Core-tree walk, gated to a `Resolved::Apply` init), so `r` is
           force-kept — materialized ONCE, every projection reads the shared record slot via `LocalRef`.
           rust declines the record-of-closures-through-host shape (a separate frontier). The tuple/match
           face is adv-62 (#1528); this is the record face.")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (mk)
              (host (io)
                (let ((v (io.get unit)))
                  (record (f (fn ((: x Int64)) (+ v x))) (g (fn ((: x Int64)) (* v x)))))))
            (def (main)
              (let ((r (mk)))
                (+ ((. r f) 10) ((. r g) 100))))
            (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (output (: 2131 Int64)))

(case "a runtime Bytes value crosses a host op boundary as list<u8> (H-bytes-arg)"
  (doc    "The wasm host-ARG Bytes path: a runtime `Bytes` argument to a host op crosses the component
           boundary as `list<u8>` (the (ptr,len) shared-memory shape, same core marshalling as a String arg
           but a `list<u8>` component type — a DEFINED type referenced by index in the import instance-type,
           vs String's inline `string`). Previously DECLINED on wasm while the rust backend crossed it — a
           reverse-parity coverage gap (v-rust-backend flagged, breaker banked the probe). `main(k)` slices a
           runtime `to-bytes` rope `(Bytes.slice … k 3)` and passes the 3-byte view to `io.sink`; the host
           answers 99. Pins that a Bytes host arg (a) COMPILES on wasm and (b) the emitted component is valid
           (`sink: func(p0: list<u8>) -> s64`, wasm-tools-verified) and (c) RUNS. rust already passed it;
           rust-async declines the multi-def host-do shape (todo). The canon Lower carries Memory(0) (no
           realloc for an argument — the guest allocates, the host reads).")
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (match (Bytes.slice (String.to-bytes (String.concat "abc" "defgh")) k 3)
                  ((Some cut) (io.sink cut))
                  ((None _u) -1))))
            (export main)))
  (host-responses (respond io.sink (: 99 Int64)))
  (host-calls (call io.sink))
  (call   main (: 2 Int64)) (output (: 99 Int64)))

(case "a host op with a Bytes arg AND a scalar arg crosses both parameters (list<u8> beside a scalar)"
  (doc    "Coverage for the wasm Bytes-host-arg increment's MIXED-ARITY face: a host op `sink2 : (-> Bytes
           Int64 Int64)` takes a `list<u8>` param AND a scalar `Int64` param. Pins that the import
           instance-type + core functype handle a `list<u8>` param (a defined-type-index) BESIDE an inline
           scalar — the func type declares `(p0: list<u8>, p1: s64) -> s64`, and the core marshalling pushes
           the Bytes `(ptr,len)` then the scalar. `main(k)` passes a 2-byte `Bytes.of` and the scalar 5; the
           host answers 9. Complements the single-Bytes-arg pin (which has no scalar to prove the mixed
           layout). rust crosses it too (both scalar+list handle-transport); rust-async declines the shape.")
  (input  (do
            (effect io (op sink2 (-> Bytes Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (io.sink2 (Bytes.of (list (UInt8.wrap k) (UInt8.wrap 66))) 5)))
            (export main)))
  (host-responses (respond io.sink2 (: 9 Int64)))
  (host-calls (call io.sink2))
  (call   main (: 65 Int64)) (output (: 9 Int64)))

(case "a host result captured by closures in a NESTED tuple fires the host op once (adv-62 nested face)"
  (doc    "adv-62 family, NESTED-destructure face: the let-bound host result `v` is shared by closures at
           TWO tuple nesting levels — `(tuple f (tuple g h))` — destructured by a nested pattern. All three
           closures capture the ONE `io.get` (fired once at the shared `let`, not re-lowered per projection);
           the fix (should_keep_binding follows the CALL init into mk's host body → force-keep + materialize
           once) threads through the nested `Core::Proj` chain. io.get=10: f(1)=11, g(2)=20, h(3)=7, sum 38.
           rust crosses it; rust-async declines the closure-in-tuple-through-host shape (todo).")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (mk)
              (host (io)
                (let ((v (io.get unit)))
                  (tuple (fn ((: x Int64)) (+ v x))
                         (tuple (fn ((: x Int64)) (* v x)) (fn ((: x Int64)) (- v x)))))))
            (def (main)
              (match (mk) ((tuple f (tuple g h)) (+ (+ (f 1) (g 2)) (h 3)))))
            (export main)))
  (host-responses (respond io.get (: 10 Int64)))
  (host-calls (call io.get))
  (output (: 38 Int64)))

(case "a host-block scrutinee folding to a multi-arm sum switch fires the host op once (adv-62 switch face)"
  (doc    "adv-62 family, SWITCH-path face (vs the Leaf-fold face the base cases pin): the host block `(mk)`
           β-inlines into the MATCH SCRUTINEE and folds to a multi-arm sum SWITCH (not a single-arm Leaf), so
           the scrutinee-reaches-host-perform guard keeps the `MatchSum` wrapper and materializes the host
           call ONCE — each arm reads the one materialized scrutinee, not a re-lowered `(host …)` block. io.get=7
           → (> 7 5) → Big 7 → 7*10 = 70 (the Small arm's +100 discriminates). Pins that the host materialize
           holds on the Switch path, not just the tuple/record Leaf fold. rust crosses it; rust-async declines.")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (type R (Big Int64) (Small Int64))
            (def (mk)
              (host (io)
                (let ((v (io.get unit)))
                  (if (> v 5) (Big v) (Small v)))))
            (def (main)
              (match (mk) ((Big h) (* h 10)) ((Small w) (+ w 100))))
            (export main)))
  (host-responses (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (output (: 70 Int64)))

; --- Host-row consumption under CONTROL FLOW: the (host-responses …) fixture is consumed in the
; ORDER calls are made, and only calls on the taken path consume rows. The pins above fix the
; straight-line order (two calls in one +) and the abandoned-path elision; these pin the
; consumption order when the CALL SEQUENCE is produced by recursion (tail and non-tail) and when
; a runtime branch selects WHICH op fires first. ---

(case "a recursion-driven host-call sequence consumes one response row per iteration in order"
  (doc    "The recursive-walk composition of the two-calls-in-order pin: `walk` performs `(io.get)` once
           per iteration in TAIL position, so n=3 consumes the rows [3,7,5] first-to-last as the digits
           accumulate left-to-right → 375. A runner that re-read row 0 per iteration gives 333; one that
           consumed from the tail gives 573. The host-calls fixture asserts exactly three calls.")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (walk (: n Int64) (: acc Int64))
              (if (> n 0) (walk (- n 1) (+ (* 10 acc) (io.get))) acc))
            (def (main (: n Int64))
              (host (io) (walk n 0)))
            (export main)))
  (host-responses (respond io.get (: 3 Int64))
                  (respond io.get (: 7 Int64))
                  (respond io.get (: 5 Int64)))
  (host-calls (call io.get) (call io.get) (call io.get))
  (call   main (: 3 Int64))
  (output (: 375 Int64)))

(case "a NON-TAIL host call consumes rows on the unwind, deepest frame first"
  (doc    "The unwind-order face: `(+ (* 10 (walk (- n 1))) (io.get))` recurses BEFORE performing, so the
           deepest frame's `(io.get)` fires first — rows [3,7,5] bind deepest-to-shallowest and the digits
           accumulate 3 → 37 → 375. The same rows in a TAIL-position walk (the pin above) yield the same
           375 by a DIFFERENT path (there, row order = iteration order; here, row order = unwind order) —
           a runner that issued calls at frame-ENTRY rather than at the perform's evaluation point would
           flip the two shapes apart (573 here).")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (walk (: n Int64))
              (if (> n 0) (+ (* 10 (walk (- n 1))) (io.get)) 0))
            (def (main (: n Int64))
              (host (io) (walk n)))
            (export main)))
  (host-responses (respond io.get (: 3 Int64))
                  (respond io.get (: 7 Int64))
                  (respond io.get (: 5 Int64)))
  (host-calls (call io.get) (call io.get) (call io.get))
  (call   main (: 3 Int64))
  (output (: 375 Int64)))

(case "a runtime branch selects WHICH host op consumes the first response row"
  (doc    "The branch-selected companion of the abandoned-path elision pin: `(if (> n 5) (io.get) (io.alt))`
           at n=3 takes the alt branch, so the FIRST row consumed is `io.alt`'s 100, and the following
           unconditional `(io.get)` consumes the second row 7 → 107. The host-calls fixture asserts the
           taken-path sequence [alt, get] — a runner that consumed rows by op-declaration order (get
           first) or issued the untaken branch's call would mis-bind both rows.")
  (input  (do
            (effect io (op get (-> Unit Int64)) (op alt (-> Unit Int64)))
            (def (main (: n Int64))
              (host (io)
                (+ (if (> n 5) (io.get) (io.alt))
                   (io.get))))
            (export main)))
  (host-responses (respond io.alt (: 100 Int64))
                  (respond io.get (: 7 Int64)))
  (host-calls (call io.alt) (call io.get))
  (call   main (: 3 Int64))
  (output (: 107 Int64)))

; --- Handler-ARM effect composition beyond the single observation pin above (:1031, whose
; re-performed value is discarded): arms whose OUTER-perform results feed the resume value,
; sibling handlers sharing one outer counter, and a transitive arm-perform cascade. ---

(case "an arm performs the outer effect TWICE and the results feed the resume value"
  (doc    "The value-carrying face of the arm-performs-outer pin: A's arm resumes `(+ (Count.tick)
           (Count.tick))` — the observation IS the resume value, not a discarded side effect. Count
           seeded 10: the two arm ticks read 10 and 11 (arm resumes 21, Count threads to 12), and a
           tick AFTER the inner handle closes reads 12 → 21 + 100·12 = 1221. Pins that an arm's
           under-frame performs advance the outer state exactly like body-level performs (a re-seeded
           or frame-local Count gives 21+100·10=1021; per-arm-entry re-reads give 20).")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect Count (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Count 10 ((tick (u) c (resume c (+ c 1))))
                (+ (handle A 0 ((a (u) s (resume (+ (Count.tick) (Count.tick)) s))) (A.a))
                   (* 100 (Count.tick)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1221 Int64)))

(case "TWO sibling inner handlers observe through ONE outer counter"
  (doc    "Under-frame threading ACROSS sequential handler frames: sibling handles A and B each tick
           the same enclosing Count from their arms. Count seeded 0: A's arm ticks (0→1), B's arm
           ticks (1→2), the body's final tick reads 2 → 7 + 10·3 + 100·2 = 237. Pins that the outer
           state is ONE line threading through both siblings in evaluation order — per-handler counter
           instances (a frame-local clone) would read 0 at the final tick (37).")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect Count (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Count 0 ((tick (u) c (resume c (+ c 1))))
                (+ (handle A 0 ((a (u) s (do (Count.tick) (resume 7 s)))) (A.a))
                   (+ (* 10 (handle B 0 ((b (u) s (do (Count.tick) (resume 3 s)))) (B.b)))
                      (* 100 (Count.tick))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 237 Int64)))

(case "a depth-3 transitive arm-perform cascade threads the innermost state end to end"
  (doc    "C's arm performs B; B's arm performs A — each perform resolving one frame further out
           (the under-frame discipline applied TRANSITIVELY). A seeded 100 resumes s and threads s+1.
           `(C.c)` → C's arm asks B ×10 → B's arm asks A → 100 (A→101) → C resumes 1000. The second
           `(C.c)` walks the same cascade reading 101 (A→102) → 1010. The direct `(A.a)` then reads
           102. 1000+1010+102 = 2112. A cascade that re-entered A at its seed per chain (2102), or
           resolved B's perform against a stale frame, breaks the sum.")
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect C (op c (-> Unit Int64)))
            (def (main (: k Int64))
              (handle A 100 ((a (u) s (resume s (+ s 1))))
                (handle B 0 ((b (u) s (resume (A.a) s)))
                  (handle C 0 ((c (u) s (resume (* 10 (B.b)) s)))
                    (+ (C.c) (+ (C.c) (A.a)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2112 Int64)))

(case "TWO Map-stated handlers stacked route each op to its own Map with no cross-contamination"
  (doc    "Heap-valued handler state × handler stacking: A and B each carry their own `(Map.empty)`-seeded
           state; six interleaved ops (3 to A at one regime, 2-or-3 to B) must route each `put` to ITS
           handler's Map, and both `size` reads at the end see only their own inserts — 3/3 at n=3 (33)
           and 2/3 at n=1 where A's third put duplicates key 1 (23). A state-slot mixup between the
           stacked frames (one Map receiving the other's insert, or a size read against the wrong frame)
           corrupts either count. The heap-state sibling of the scalar two-handler pins; each resume
           dups/drops a CHAMP handle per op.")
  (input  (do
            (effect A (op puta (-> Int64 Unit)) (op sizea (-> Unit Int64)))
            (effect B (op putb (-> Int64 Unit)) (op sizeb (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A (Map.empty)
                ( (puta (k) m (resume unit (Map.insert m k k)))
                  (sizea (u) m (resume (Map.len m) m)) )
                (handle B (Map.empty)
                  ( (putb (k) m (resume unit (Map.insert m k k)))
                    (sizeb (u) m (resume (Map.len m) m)) )
                  (do
                    (A.puta 1) (B.putb 10) (A.puta 2) (B.putb 20) (B.putb 30) (A.puta n)
                    (+ (* 10 (A.sizea)) (B.sizeb))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33 Int64))
  (call   main (: 1 Int64)) (output (: 23 Int64)))

(case "performs in BOTH operands of an or consume state only on the reached paths"
  (doc    "The RESUMPTIVE-perform composition with short-circuit (the abortive pins cover elision of an
           ABORT; this observes handler STATE): `(or (> (Ctr.tick) 10) (> (Ctr.tick) 3))` seeded k, with
           a trailing tick pinning the exact post-connective state. k=20: the lhs tick reads 20 (s→21),
           true short-circuits the rhs → 100 + 21 = 121 (ONE tick). k=4: lhs 4 (s→5) false, rhs 5 (s→6)
           true → 100 + 6 = 106 (TWO ticks). k=0: both false (s→2) → 200 + 2 = 202. A fold treating the
           rhs perform as unconditional double-fires and shifts every digit — the adv-55 rhs-conditionality
           class observed at the STATE tier, where a wrong fold is visible even when the boolean value
           happens to agree. (Core::And is the shared and/or core node — this case correctly references
           it even though the surface operator here is `or`.)")
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ctr k ((tick (u) s (resume s (+ s 1))))
                (+ (if (or (> (Ctr.tick) 10) (> (Ctr.tick) 3)) 100 200)
                   (Ctr.tick))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 121 Int64))
  (call   main (: 4 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: 202 Int64)))

(case "a Map-stated handler threads 50 recursive puts and reads the accumulated size"
  (doc    "Heap-valued handler state × recursion at scale: `fill` performs one `put` per recursive step,
           each resume dup/dropping the CHAMP handle as the Map grows to 50 entries — a Perceus witness
           on the handler path (a per-resume leak or premature free surfaces as memory corruption or a
           fault long before 50). The trailing `size` reads the fully-accumulated state (50). The
           straight-line put pins cover 2-3 ops; this is the recursion-driven volume shape a memoizing
           pass actually produces.")
  (input  (do
            (effect Store (op put (-> Int64 Unit)) (op size (-> Unit Int64)))
            (def (fill (: i Int64))
              (if (= i 0) unit (do (Store.put i) (fill (- i 1)))))
            (def (main (: n Int64))
              (handle Store (Map.empty)
                ( (put (k) m (resume unit (Map.insert m k (* k 2))))
                  (size (u) m (resume (Map.len m) m)) )
                (do
                  (fill n)
                  (Store.size))))
            (export main)))
  (call   main (: 50 Int64)) (output (: 50 Int64)))
(case "a recursive performer of a nested-handler op whose resume performs the outer effect threads the advance"
  (doc    "The recursive-nested-arm-resume fix (v-effects self-probe, concierge-steered pre-spec-lift): a
           recursive `loop` calls a nested `B` handler's op `B.step` whose ARM resume-value performs the OUTER
           `A` effect — `(step (u) t (resume (A.tick) t))`. Each iteration's `A.tick` reads+advances A-state.
           `loop 2` sums the two B.step results (= A.tick's pre-advance values 10 then 11) → 21. Pins that the
           per-iteration outer advance made INSIDE a nested handler's resume-value threads correctly across the
           recursion — the merge specializes `loop` against BOTH A and B, and the pre-spec-lift makes the
           arm-hidden `A.tick` a direct-body perform so it threads via the top-level perform arm. Before the
           fix the merge was skipped (the outer perform hidden in B's arm was invisible to the merge decision)
           and the advance dropped. NO post-loop A read here (that observing sub-case is a separate increment).")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (loop (: n Int64)) (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (handle B 0 ((step (u) t (resume (A.tick) t)))
                  (loop 2))))
            (export main)))
  (output (: 21 Int64)))

(case "a recursive nested-op performer whose per-iteration outer advance is read by a POST-loop observer threads it"
  (doc    "The post-loop-observer companion of the case above — the sub-case that one explicitly deferred. The
           recursive `loop` calls `B.step` whose arm resumes with the outer `(A.tick)`; then a POST-loop
           `(A.get)` reads the A-state the recursion advanced. `loop 1` calls `B.step` once → `A.tick` returns
           the pre-advance A-state 10 and advances A → 11; the loop sums that one value = 10. Then `(A.get)`
           reads the ADVANCED A-state 11, so `(+ (loop 1) (A.get))` = `(+ 10 11)` = 21. Pins that the outer
           advance the recursion made is OBSERVABLE after the loop — the merged specialization returns the
           advanced A out-state (multi-value) and the post-loop `(A.get)` reads it, not the pre-loop seed
           (which would give 20 — the silent miscompile this fix eliminates). Requires: (1) the merged
           specialization target the accum-COPY of the seed-wrapped `loop` (`accum_seed_redirect`, threading
           the accumulator seed as a call-site arg), and (2) the merged nested-handler body drain its pending
           multi-value spec-call temp into a wrapping `let` (else the out-state projection leaks its binder).")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (loop (: n Int64)) (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (handle B 0 ((step (u) t (resume (A.tick) t)))
                  (+ (loop 1) (A.get)))))
            (export main)))
  (output (: 21 Int64)))

(case "a DEPTH-3 nested-op chain whose deepest resume performs the outer effect declines cleanly (no silent drop)"
  (doc    "The depth-3 companion of the post-observer case above — the outer perform hides TWO handler levels
           down. `loop` performs `C.hop`; C's arm resumes `(B.step)`; B's arm resumes `(A.tick)`; then a
           post-loop `(A.get)`. The correct value is 21 (tick returns 10 advancing A→11; loop=10; A.get reads
           11). The depth-2 fix's pre-spec-lift (`lift_inner_op_arm_outer_perform`) rewrites `(C.hop)` into
           C's resume value `(B.step)` in ONE step, but does NOT chase `B.step`'s OWN arm-hidden `(A.tick)` —
           so folding it would specialize against B alone and DROP A's advance → a SILENT 20 (the regression
           this guards against — it briefly shipped that way in #2136 before the depth-3 guard). A correct
           depth-3 fold must lift RECURSIVELY (a later increment); until then this DECLINES cleanly (a decline
           is safe, a wrong value is not). `resume_val_op_arm_also_performs_outer` detects the deeper chain
           (the op the resume value performs has an arm that itself performs YET ANOTHER effect op) and leaves
           it un-lifted → `specialize_recursive` declines. Flips decline→21 when the recursive lift lands.")
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (def (loop (: n Int64)) (if (= n 0) 0 (+ (C.hop) (loop (- n 1)))))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (handle B 0 ((step (u) t (resume (A.tick) t)))
                  (handle C 0 ((hop (u) w (resume (B.step) w)))
                    (+ (loop 1) (A.get))))))
            (export main)))
  (output (: 21 Int64)))

(case "a DEPTH-3 nested-op chain WITHOUT a post-observer folds (the no-observer control for the observer-gated guard)"
  (doc    "The no-observer control (breaker rx6) for the depth-3 decline case above. The SAME recursion ×
           depth-3 chain — `loop` performs `C.hop`; C's arm resumes `(B.step)`; B's arm resumes `(A.tick)` —
           but the body is bare `(loop 2)` with NO post-loop observer and a single-op `A`. Without an observer
           of the recursion's out-state, the accum-redirect never engages, so the between-iteration advance
           carries through the merge and the chain FOLDS: `loop 2` sums the two `A.tick` pre-advance values 10
           then 11 = 21. This pins that the observer-GATED depth-3+ guard does NOT over-decline the working
           no-observer chain — the guard (`caller_observes_outstate && resume_val_op_arm_also_performs_outer`)
           fires ONLY when the out-state is observed (the decline case above), so this twin is unaffected.
           #2179's guard briefly over-declined this (fold→decline); the observer gate is what separates the
           must-decline observer chain from this must-fold no-observer twin.")
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (effect C (op hop (-> Unit Int64)))
            (def (loop (: n Int64)) (if (= n 0) 0 (+ (C.hop) (loop (- n 1)))))
            (def (main)
              (handle A 10 ((tick (u) s (resume s (+ s 1))))
                (handle B 0 ((step (u) t (resume (A.tick) t)))
                  (handle C 0 ((hop (u) w (resume (B.step) w)))
                    (loop 2)))))
            (export main)))
  (output (: 21 Int64)))

(case "an s-around-k ctl arm that ALSO performs an outer effect in the arm body folds — the two E5 fixes compose"
  (doc    "The COMPOSITION guard for the two E5 fixes: the s-around-k lexical-`ctl` pin (`pin_refs_to_binders`)
           and the arm-performs-outer path must compose without re-orphaning the pinned state binder. An inner
           `G` handler's arm reads the state binder `s` AROUND its `(k x)` continuation application AND performs
           the OUTER `A` effect in the SAME arm body — `(y (x) s k (+ (+ s (A.get)) (k x)))`. Seeded n=100
           (runtime param), A seeded 7: `s`=100, `(A.get)`=7, `(k 5)`=5 (the continuation `C = □` returns 5),
           so `(+ (+ 100 7) 5)` = 112. Pins that s-around-k + an arm-body outer perform fold together (a naive
           interaction re-detached the arm body after the pin, re-leaking `s` as CDZ0101 — the pre-fix ek1
           signature; this witness catches that regression). breaker ek8.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (+ (+ s (A.get)) (k x))))
                  (G.y 5))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))

(case "an s-around-k ctl arm whose K-ARGUMENT performs an outer effect folds"
  (doc    "The k-argument face of the composition guard above: the outer perform sits INSIDE the `(k …)`
           argument rather than beside it — `(y (x) s k (+ s (k (+ x (A.get)))))`. Seeded n=100, A seeded 7:
           `(A.get)`=7, the k-arg `(+ x (A.get))` = `(+ 5 7)` = 12, `(k 12)` returns 12 into `C = □`, and `s`
           around it = 100, so `(+ 100 12)` = 112. Pins that the arm-body state binder `s` stays resolved when
           the `(k v)`→`(resume v s)` rewrite's argument itself performs an outer effect. breaker ek8d.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (+ s (k (+ x (A.get))))))
                  (G.y 5))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))

(case "an s-around-k ctl arm whose handle BODY performs an outer effect after the inner perform folds"
  (doc    "The body-perform face: the s-around-k arm has NO perform of its own — `(y (x) s k (+ s (k (+ x
           1))))` — but the inner `G` handle's BODY performs the outer `A` effect AFTER the G-perform: `(+
           (G.y 5) (A.get))`. Seeded n=100, A seeded 7: `(G.y 5)` folds the arm — `(k (+ 5 1))` = `(k 6)` = 6
           into `C = □`, `s`=100 → `(+ 100 6)` = 106; then the body's `(A.get)`=7, so `(+ 106 7)` = 113. Pins
           that the pinned state binder survives when the OUTER perform is in the handle body (region-wrapped
           around the inner handle) rather than in the arm. breaker ek10.")
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect G (op y (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle G n
                  ((y (x) s k (+ s (k (+ x 1)))))
                  (+ (G.y 5) (A.get)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 113 Int64)))

(case "a nested-handler ctl arm whose continuation-consuming body ALSO performs an OUTER effect folds"
  (doc    "The confluence of the lexical-`ctl` surface and the nested-handler outer-perform family: an INNER
           handler `B`'s 5-part arm applies its continuation `k` AND, in the same continuation-consuming
           body, performs an OUTER handler `A`'s op — `(flip () t k (+ (* (k 2) 10) (A.geta)))` under
           `handle A(handle B … (B.flip))`. When `k` is applied lexically `(k 2)` = `(resume 2 t)` returning
           into B's delimited context `C = □` (the whole B body is the flip) = 2, so `(* 2 10)` = 20; then
           `(A.geta)` reads A's state (seeded 100) = 100, giving `(+ 20 100)` = 120. Pins that the within-
           activation `ctl`→`resume` rewrite composes with a sibling OUTER perform in the SAME arm body under
           a nested handler — the lexical-`k` result and the foreign `A.geta` both resolve and thread
           correctly (a miscompile would drop A's read or mis-thread the continuation). Guards the seam
           between the lexical-`ctl` fold and the nested-handler outer-perform threading.")
  (input  (do
            (effect A (op geta (-> Unit Int64)))
            (effect B (op flip (-> Unit Int64)))
            (def (main) (handle A 100 ((geta (u) s (resume s (+ s 1))))
              (handle B 0 ((flip () t k (+ (* (k 2) 10) (A.geta)))) (B.flip)))) (export main)))
  (output (: 120 Int64)))


(case "a closure looked up from a map by a perform result, applied to a perform result, threads through call_indirect"
  (doc    "A Map of CLOSURES indexed by a perform-computed key, the selected closure applied to a
           perform-fed argument, under a resumptive handler: `(match (Map.lookup ops (St.pick)) ((Some f)
           (f (St.feed))) ((None _u) -1))`. This is the closure-from-collection × effects-threaded-operands
           shape — the looked-up closure is the funcref-table callee (`call_indirect`) and BOTH its selector
           key and its applied argument are perform results the handler fold splices in. Pins a wasm-codegen
           miscompile (breaker-found, v-effects-routed, 2026-08-05): the closure operand is a dup-site
           `Core::SumPayload` (the `Some` payload) whose Perceus retain floats its cell into a scratch slot
           typed i32; the perform-threaded i64 argument was materialized into that SAME slot (the closure and
           the arg both emitted at `cell_slot + 1`), and a wasm local has one type function-wide → an i32/i64
           collision → `call_indirect`'s function failed to validate (invalid module, wasmtime rejected at
           compile). The rust backend always ran it correctly (the fold is sound; the defect was purely the
           wasm scratch-slot allocation). ops = {0: x↦x*2, 1: x↦x+1000}; the pick/feed handler threads s=5,6,…
           so pick→5%2=1 selects the +1000 closure, feed→6, giving 6+1000 = 1006.")
  (input  (do
            (effect St (op pick (-> Unit Int64)) (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle St n
                  ((pick (u) s (resume (% s 2) (+ s 1)))
                   (feed (u) s (resume s (+ s 1))))
                  (match (Map.lookup ops (St.pick))
                    ((Some f) (f (St.feed)))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
