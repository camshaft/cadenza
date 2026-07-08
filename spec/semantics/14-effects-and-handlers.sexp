; Effects and handlers — witnesses capabilities-and-effects.md. An effect is declared with (effect
; <name> (op <op> <type>)…): a ROUTING-AGNOSTIC CONTRACT that names the effect and types its operations
; and says NOTHING about where it is discharged. Routing is decided by the nearest enclosing router: a
; (handle <init> ((<name>.<op> (params…) <state> body)…) body) discharges the effect IN-PROGRAM (it does
; NOT appear in the manifest), while an entrypoint (host (<effect>…) body) DELEGATES it to the component
; boundary as a plain imported-function call the host resolves (the host is its terminal handler; it enters
; the manifest as the escaping row; the delegation is the grant). The SAME declared effect may be handled in
; one program and delegated in another — there is no (host) marker on the declaration and no separate import
; form. An operation is performed and handled as <name>.<op>.
;
; A HANDLER FOLDS STATE (capabilities-and-effects.md #A Handler Threads State Across The Operations It
; Discharges). Every handle SEEDS an initial state — `(handle <init> (arms…) body)` — fixed where the
; handler is installed, so nothing is ambient. Every arm binds the CURRENT state after its operation's
; parameters — `(<name>.<op> (params…) <state> body)` — and resume carries BOTH outputs:
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
; These are (needs effects) cases a later generation realizes; the seed realizes the mandatory capability
; floor but not the effect surface or the algebraic-handler layer. A response-returning delegated call fixes
; its response with (host-responses …) so the run is a deterministic function of input and that response.

(case "a run's result is a deterministic function of a host call's recorded response"
  (doc    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: `ask` is a routing-agnostic effect the entrypoint delegates to the host, so
           `ask.ask` is a plain imported-function call returning its response at the boundary. The
           (host-responses …) fixture supplies the response in call order; given that response the run
           deterministically computes 100. How the host produces the response — inline, fiber-suspend, or
           re-derive from the recorded responses — is host policy the program does not observe.")
  (needs  effects)
  (input  (module m
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (* (ask.ask) 10)))))
  (host-responses (respond ask.ask (: 10 Int64)))
  (host-calls (call ask.ask))
  (output (: 100 Int64)))

(case "two host calls consume their responses in order"
  (doc    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: two host calls consume two responses in the order made; the sum is a deterministic
           function of input and the ordered response sequence.")
  (needs  effects)
  (input  (module m
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (+ (ask.ask) (ask.ask))))))
  (host-responses (respond ask.ask (: 3 Int64))
                  (respond ask.ask (: 4 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 7 Int64)))

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
  (needs  effects)
  (input  (module m
            (effect ask (op ask (-> Unit Int64)))
            (effect Scale (op by (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle unit ((Scale.by (n) s (resume (* n 2) s)))
                  (Scale.by (ask.ask)))))))
  (host-responses (respond ask.ask (: 21 Int64)))
  (host-calls (call ask.ask))
  (output (: 42 Int64)))

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
  (needs  effects)
  (input  (module m
            (effect ask (op ask (-> Unit Int64)))
            (effect Count (op tick (-> Unit Unit)))
            (def (main)
              (host (ask)
                (handle unit ((Count.tick (u) s (resume unit s)))
                  (handle unit ((ask.ask () s (do (Count.tick) (resume (ask.ask) s))))
                    (+ (ask.ask) (ask.ask))))))))
  (host-responses (respond ask.ask (: 3 Int64))
                  (respond ask.ask (: 4 Int64)))
  (host-calls (call ask.ask) (call ask.ask))
  (output (: 7 Int64)))

(case "an effect discharged by a handler does not escape to the manifest"
  (doc    "Witnesses capabilities-and-effects.md #An Effect That Does Not Escape Is Discharged By A
           Handler and #An Effect Discharged By An In-Program Handler Does Not Appear In The Manifest:
           the `Choose` effect is declared with a nullary operation `pick`, raised in the body as
           `(Choose.pick)`, and discharged by an enclosing handler that resumes it with 5, so the effect
           never reaches a host function. The handler is stateless (seed `unit`, thread `s` unchanged). The
           program imports no host function, so its manifest is empty (host-calls asserts none), yet it uses
           an effect internally. Operations are qualified by their declaring effect (#An Effect Declaration
           Names The Effect And Types Its Operations).")
  (needs  effects)
  (input  (module m
            (effect Choose (op pick (-> Unit Int64)))
            (def (main)
              (handle unit ((Choose.pick () s (resume 5 s)))
                (+ (Choose.pick) 1)))))
  (output (: 6 Int64))
  (host-calls))

(case "a handler resumes its continuation at most once by default"
  (doc    "Witnesses capabilities-and-effects.md #A Continuation Is One-Shot By Default: the handler
           resumes the continuation exactly once, so the affine discipline holds and the result is a
           single value (the resumed computation is not duplicated). `Get` is declared with a nullary
           operation `get` returning Int64, performed as `(Get.get)`; the handler is stateless.")
  (needs  effects)
  (input  (module m
            (effect Get (op get (-> Unit Int64)))
            (def (main)
              (handle unit ((Get.get () s (resume 41 s)))
                (+ (Get.get) 1)))))
  (output (: 42 Int64))
  (host-calls))

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
  (needs  effects)
  (input  (module m
            (effect Fresh (op next (-> Unit Int64)))
            (def (main)
              (handle 0 ((Fresh.next (u) s (resume s (+ s 1))))
                (do (Fresh.next)
                    (Fresh.next)
                    (Fresh.next))))))
  (output (: 2 Int64)))

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
  (needs  effects)
  (needs  list-growth)
  (input  (module m
            (effect Diag (op emit (-> Int64 Unit))
                         (op collect (-> Unit (List Int64))))
            (def (main)
              (handle (list) ((Diag.emit (code) s (resume unit (List.push s code)))
                              (Diag.collect (u) s (resume s s)))
                (do (Diag.emit 201)
                    (Diag.emit 210)
                    (Diag.collect))))))
  (output (: (list 201 210) (List Int64))))

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
  (needs  effects)
  (needs  list-growth)
  (input  (module m
            (effect Diag (op emit (-> Int64 Unit))
                         (op collect (-> Unit (List Int64))))
            (def (walk n)
              (if (< n 1)
                  (Diag.collect unit)
                  (do (Diag.emit n) (walk (- n 1)))))
            (def (main)
              (handle (list) ((Diag.emit (v) s (resume unit (List.push s v)))
                              (Diag.collect (u) s (resume s s)))
                (List.len (walk 3))))))
  (output (: 3 Int64)))

(case "two effects each declaring a same-named operation do not collide"
  (doc    "Witnesses capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
           Operations (2nd sentence): `Unify` and `Scope` each declare a `resolve` operation, reached as
           `Unify.resolve` and `Scope.resolve`; the qualified names disambiguate, so the two handler arms
           discharge distinct operations. The body performs `Unify.resolve`, resumed with 5. The handler is
           stateless (seed `unit`). Pins that an operation is reached through its declaring effect and a
           shared operation name is collision-free.")
  (needs  effects)
  (input  (module m
            (effect Unify (op resolve (-> Int64 Int64)))
            (effect Scope (op resolve (-> Int64 Int64)))
            (def (main)
              (handle unit ((Unify.resolve (x) s (resume (+ x 1) s))
                            (Scope.resolve (x) s (resume x s)))
                (Unify.resolve 4)))))
  (output (: 5 Int64))
  (host-calls))

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
  (needs  effects)
  (input  (module m
            (effect Bump (op by (-> Int64 Int64)))
            (def (gen) (Bump.by 41))
            (def (main)
              (handle unit ((Bump.by (n) s (resume (+ n 1) s)))
                (gen)))))
  (output (: 42 Int64))
  (host-calls))

(case "an effect resolves past an intermediate frame that installs no handler"
  (doc    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined: the call chain is `main` (handles `Ping`) -> `mid` (no handler) -> `leaf`
           (performs `Ping.ping`). The perform in `leaf` searches OUTWARD along the call chain, past `mid`
           which installs no handler, to `main`'s handler, which resumes with 5; `mid` then computes
           `(+ 5 100)` = 105. An intermediate function that installs no handler is transparent to
           resolution — it is merely a frame on the chain. The handler is stateless.")
  (needs  effects)
  (input  (module m
            (effect Ping (op ping (-> Unit Int64)))
            (def (leaf) (Ping.ping))
            (def (mid)  (+ (leaf) 100))
            (def (main)
              (handle unit ((Ping.ping () s (resume 5 s)))
                (mid)))))
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
  (needs  effects)
  (input  (module m
            (effect Mul (op by (-> Int64 Int64)))
            (def (leaf) (Mul.by 1))
            (def (mid)  (handle unit ((Mul.by (x) s (resume (* x 10) s))) (leaf)))
            (def (main) (handle unit ((Mul.by (x) s (resume (* x 100) s))) (mid)))))
  (output (: 10 Int64))
  (host-calls))

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
  (needs  effects)
  (input  (module m
            (effect Get (op get (-> Unit Int64)))
            (def (ask) (+ (Get.get) 1))
            (def (main)
              (+ (handle unit ((Get.get () s (resume 10 s))) (ask))
                 (handle unit ((Get.get () s (resume 20 s))) (ask))))))
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
  (needs  effects)
  (input  (module m
            (effect Ask (op ask (-> Unit Int64)))
            (def (d) (Ask.ask))
            (def (c) (+ (d) 1))
            (def (b) (+ (c) 1))
            (def (a) (+ (b) 1))
            (def (main)
              (handle unit ((Ask.ask () s (resume 7 s)))
                (a)))))
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
  (needs  effects)
  (input  (module m
            (effect Fresh (op next (-> Unit Int64)))
            (def (label)   (Fresh.next))
            (def (pair-of) (tuple (label) (label)))
            (def (main)
              (handle 0 ((Fresh.next (u) s (resume s (+ s 1))))
                (pair-of)))))
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
  (needs  effects)
  (input  (module m
            (effect Countdown (op tick (-> Unit Int64)))
            (def (loop)
              (if (= (Countdown.tick) 0)
                  0
                  (+ 1 (loop))))
            (def (main)
              (handle 3 ((Countdown.tick (u) s (resume s (- s 1))))
                (loop)))))
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
  (needs  effects)
  (input  (module m
            (effect Idx (op next (-> Unit Int64)))
            (def (sum-down)
              (let ((i (Idx.next)))
                (if (= i 0)
                    0
                    (+ i (sum-down)))))
            (def (main)
              (handle 3 ((Idx.next (u) s (resume s (- s 1))))
                (sum-down)))))
  (output (: 6 Int64)))

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
  (needs  effects)
  (input  (module m
            (effect A (op tick (-> Unit Int64)))
            (effect B (op bump (-> Unit Int64)))
            (def (loop)
              (if (= (A.tick) 0)
                  0
                  (+ (B.bump) (loop))))
            (def (main)
              (handle 0 ((B.bump (u) s (resume s (+ s 10))))
                (handle 3 ((A.tick (u) s (resume s (- s 1))))
                  (loop))))))
  (output (: 30 Int64)))

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
  (needs  effects)
  (input  (module m
            (effect Fresh (op next (-> Unit Int64)))
            (def (loop n)
              (handle 100 ((Fresh.next (u) s (resume s (+ s 1))))
                (if (= n 0)
                    (Fresh.next)
                    (loop (- n 1)))))
            (def (main)
              (loop 2))))
  (output (: 100 Int64)))

; --- Rejections the routing model introduces ----------------------------------------------------
; An effect declaration is the CLOSED set of an effect's operations, so a handler arm for an operation
; the effect does not declare is rejected (CDZ0403), and an operation reached with neither an enclosing
; handler nor an enclosing entrypoint delegation — so it would escape ungranted — is rejected (CDZ0401,
; the single "no home" check that merges the former undischarged-intra and undeclared-host rejections).
; These are the compile-time checks that keep "no ambient authority" a property of the source
; (capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time Error, #A Handler Arm Names An
; Operation Its Effect Declares).

(case "a handler arm for an operation the effect does not declare is rejected"
  (doc    "`Choose` declares only `pick`; a handler arm naming `Choose.guess` names an operation the
           effect does not declare, rejected at compile time (CDZ0403) because the declaration is the
           closed set of an effect's operations (capabilities-and-effects.md #A Handler Arm Names An
           Operation Its Effect Declares). A generation that does not yet check arm membership declines
           rather than running the program (reject-don't-miscompile).")
  (needs  effects)
  (input  (module m
            (effect Choose (op pick (-> Unit Int64)))
            (def (main)
              (handle unit ((Choose.guess () s (resume 5 s)))
                (Choose.pick)))))
  (error  CDZ0403))

(case "an effect operation reached with neither a handler nor a delegation is rejected"
  (doc    "`Ask` is a routing-agnostic effect; `main` performs `(Ask.ask)` with no enclosing handler and
           no enclosing entrypoint `host` delegation, so the effect would escape ungranted — rejected at
           compile time (CDZ0401, capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time
           Error). This is the single 'no home for a reached effect' check: since host-binding is now an
           entrypoint routing decision rather than a declaration-time marker, the former CDZ0402
           (undischarged intra-program effect) and the former undeclared-host CDZ0401 are one condition.
           Contrast the interpose case above, where an enclosing `host (ask)` delegation gives the effect
           a home.")
  (needs  effects)
  (input  (module m
            (effect Ask (op ask (-> Unit Int64)))
            (def (main)
              (+ (Ask.ask) 1))))
  (error  CDZ0401))

(case "a program that delegates no effect is pure and never suspends"
  (doc    "Witnesses capabilities-and-effects.md #Purity Is The Empty Effect Row: a program that reaches
           no effect it must route runs straight to normal termination, makes no host call, and has an
           empty manifest. This is the same property the compiler component itself has.")
  (needs  effects)
  (input  (module m
            (def (main) (+ 20 22))))
  (output (: 42 Int64))
  (host-calls))
