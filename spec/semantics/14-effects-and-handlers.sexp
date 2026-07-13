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
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (* (ask.ask) 10))) (export main)))
  (host-responses (respond ask.ask (: 10 Int64)))
  (host-calls (call ask.ask))
  (output (: 100 Int64)))

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
              (handle 0 ((Bail.bail (n) s n))
                (+ 1 (Bail.bail 7)))) (export main)))
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
              (handle 0 ((Bail.bail (n) s n))
                (if true (Bail.bail 7) 99))) (export main)))
  (output (: 7 Int64)))

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
              (handle 0 ((Bail.bail (n) s n))
                (let ((k 5)) (if true (Bail.bail 7) k)))) (export main)))
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
              (handle 0 ((Get.get (n) s (resume 5 s)))
                (+ x (Get.get 0)))) (export main)))
  (call   main (: 10 Int64))
  (output (: 15 Int64)))

(case "a runtime condition selects an abortive branch reading an enclosing parameter"
  (doc    "The branch-tail abort with a RUNTIME condition over an enclosing parameter — the shape a
           validation routine takes: `(handle 0 ((Bail.bail (n) s n)) (if (< x 5) (Bail.bail 7) x))`. The
           `if` is the handle's value, so an abort in a branch tail is local to that branch (yields the arm
           value); the other branch reads the parameter `x` and falls through. Called with `x = 9` (not <
           5), the false branch yields `x` = 9 — no abort. This composes the branch-tail abortive fold with
           a free parameter reference and a runtime condition (`DESIGN-effects-rcdzc.md` §4.2).")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: x Int64))
              (handle 0 ((Bail.bail (n) s n))
                (if (< x 5) (Bail.bail 7) x))) (export main)))
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
              (handle 0 ((Bail.bail (n) s n))
                (+ 100 (if (< x 5) (Bail.bail 7) 50)))) (export main)))
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
              (handle 0 ((Bail.bail (n) s n))
                (if (< (Bail.bail 7) 5) 1 2))) (export main)))
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
              (handle false ((Bail.bail (n) s (< n 100)))
                (and (< x 5) (Bail.bail 7)))) (export main)))
  (call   main (: 3 Int64))
  (output (: true Bool)))

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
              (handle 0 ((Bail.bail (n) s n))
                (let ((k (if (< x 5) (Bail.bail 7) 0))) (+ 1 k)))) (export main)))
  (call   main (: 9 Int64))
  (output (: 1 Int64)))

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
              (handle 0 ((Tick.tick (u) s (resume s (+ s 1))))
                (walk 3))) (export main)))
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

(case "a program that delegates no effect is pure and never suspends"
  (doc    "Witnesses capabilities-and-effects.md #Purity Is The Empty Effect Row: a program that reaches
           no effect it must route runs straight to normal termination, makes no host call, and has an
           empty manifest. This is the same property the compiler component itself has.")
  (input  (do
            (def (main) (+ 20 22)) (export main)))
  (output (: 42 Int64))
  (host-calls))
