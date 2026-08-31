; Capabilities — witnesses the mandatory capability floor of capabilities-and-effects.md (no ambient
; authority) and host-interface-binding.md. An effect declaration is a ROUTING-AGNOSTIC CONTRACT —
; (effect <name> (op <op> (-> <param>... <result>))) — that says nothing about where the effect is
; discharged. Routing is decided at the ENTRYPOINT: a (host (<effect>...) <body>) delegation grants a
; set of effects boundary access, and the host is their TERMINAL handler (the boundary counterpart of
; (handle …)). An effect an entrypoint delegates and no nearer handler discharges enters the manifest;
; an effect neither handled nor delegated that is nonetheless reached is CDZ0401 (the single "no home"
; rejection, merging the former undeclared-host and undischarged-intra checks); a delegation naming an
; effect never reached is CDZ0404 (latent authority). An operation is performed as <name>.<op>. The
; optional effect-row TYPING layer is NOT witnessed here (it is a later capability).
;
; These exercise the effect surface, realized when the seed's reader learns the (effect …)
; declaration and the entrypoint (host …) delegation. Until then the seed declines them; it still enforces
; the capability floor itself once it lowers the new surface.
(case
  "an entrypoint delegation lets a program reach its host function"
  (doc
    "Witnesses capabilities-and-effects.md #An Entrypoint Delegates The Capabilities It Grants To
           The Host and #Host-Binding Is A Routing Decision Made At The Entrypoint: `log` is declared as
           a routing-agnostic effect, and main DELEGATES it to the host with a (host (log) …) form, so
           `log.emit` is bound at the boundary (host-interface-binding.md #A Host Import Is A WIT-Typed
           Function The Manifest Enumerates) — the delegation IS the manifest grant — and the run makes
           the host call, then terminates normally with the unit value (the operation's WIT result is
           Unit). The (output …) clause pins the terminal condition and (host-calls …) pins the ordered
           host-call observation.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main) (host (log) (log.emit "ready")))
      (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "ready" String))))

(case
  "a host-op argument COMPUTED by guest arithmetic crosses the boundary evaluated"
  (doc
    "The computed-argument face of the host boundary (the arg-witnessing pins above pass const
           string literals): the delegated op's argument is a guest arithmetic expression over a runtime
           parameter — `(out.put (* (+ n 1) 10))` at n = 4 — and the host-calls record witnesses the
           EVALUATED value 50 crossing the boundary (not the expression, not a lazy thunk). The host's
           response (99) then becomes the perform's value. Pins that guest computation completes BEFORE
           the boundary crossing and the recorded call carries the result — the log-a-derived-metric
           idiom.")
  (input
    (do
      (effect out (op put (-> Int64 Int64)))
      (def (main (: n Int64)) (host (out) (out.put (* (+ n 1) 10))))
      (export main)))
  (call main (: 4 Int64))
  (host-responses (respond out.put (: 99 Int64)))
  (host-calls (call out.put (: 50 Int64)))
  (output (: 99 Int64)))

; An entrypoint's delegation reaches an effect performed anywhere in the operations REACHABLE from its
; body — including inside a RECURSIVE function it calls. capabilities-and-effects.md #An Entrypoint
; Delegates The Capabilities It Grants To The Host: "The compiler MUST determine a program's required
; capabilities from the operations its entrypoints actually REACH and delegate", and #The Authority An
; Entrypoint Reaches: "determined by the operations reachable from its own body under its own delegations"
; — reachability follows the CALL GRAPH. So `main`, delegating `log` with `(host (log) …)` and calling a
; recursive `go` that performs `log.emit`, reaches `log.emit` under its delegation: the program is granted
; and MUST run, emitting one host call per performance. The non-recursive case already works — `(host
; (log) (go))` for a non-recursive `go` performing `log.emit` runs, as does a two-level non-recursive
; chain — and the intra-program-handler analog works through recursion too (a recursive `go` performing an
; effect discharged by an enclosing `handle` runs). A compiler whose host-delegation REACHABILITY analysis
; does not traverse into a recursive function wrongly concludes the effect is ungranted and rejects the
; program (CDZ0401) — a FALSE rejection of a valid, granted program, and the recursion-of-the-performing-
; function is the sole trigger (the same effect performed in a non-recursive callee, or discharged by an
; intra-program handler through the same recursion, is accepted). A generation that does not yet follow a
; recursive call in delegation reachability must not reject a program the delegation grants.
(case
  "an entrypoint delegation reaches an effect performed in a recursive callee"
  (doc
    "`main` delegates `log` with `(host (log) …)` and calls a recursive `go` that performs
           `log.emit` on each step — so `log.emit` is reachable from `main`'s body under its delegation
           and IS granted (capabilities-and-effects.md #An Entrypoint Delegates The Capabilities It Grants
           To The Host: capabilities are the operations the entrypoint actually REACHES, reachability
           following the call graph). The program MUST run, terminating in `unit` and making one
           `log.emit` host call per performance (here one, `go 1`). Pins that delegation reachability
           traverses into a recursive function: the non-recursive callee case already runs (the case
           above, and a two-level chain), and the intra-program-handler analog runs through recursion, so
           a compiler that rejects this as ungranted (CDZ0401) falsely rejects a valid program because the
           performing function is recursive. A generation that does not yet follow a recursive call in
           delegation reachability must not reject a program the delegation grants.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (go n) (if (= n 0) unit (do (log.emit "x") (go (- n 1)))))
      (def (main) (host (log) (go 1)))
      (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "x" String))))

; Reachability is a STATIC (call-graph) property: an effect performed only inside an `if`-branch is REACHED
; (it could run), so the delegation grants it and the program compiles — and then the RUNTIME branch decides
; whether the host call actually fires. These pin both sides: with the branch taken, the delegated effect
; runs (one host call); with it not taken, no host call, same terminal unit. (TODO on the rust backend like
; the other host-delegation cases — a known rust host-boundary gap; the wasm path pins the semantics.)
(case
  "a delegated effect reached only through an if-branch is granted and fires when the branch is taken"
  (doc
    "The effect `log.emit` is performed ONLY in the then-branch of `(if b … unit)`, so it is REACHABLE
           from main's body — the static reachability analysis grants it under the `(host (log) …)`
           delegation and the program compiles. Called with `b = true` the branch is taken, so exactly one
           host call fires. Pins that a conditionally-reached effect is granted on the STATIC could-it-run,
           not on whether a particular run reaches it — reachability follows the call graph, not the value.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main (: b Bool)) (host (log) (if b (log.emit "yes") unit)))
      (export main)))
  (call main (: true Bool))
  (output (: unit Unit))
  (host-calls (call log.emit (: "yes" String))))

(case
  "the same conditional delegation makes no host call when the branch is not taken"
  (doc
    "The runtime complement: the identical program called with `b = false` takes the `unit` branch, so
           `log.emit` — though granted (reachable) at compile time — does NOT fire; main terminates at unit
           with NO host call. Pins that the static GRANT (the effect is in the manifest because it is
           reachable) is independent of the runtime OCCURRENCE (whether a given run performs it) — the grant
           is conservative, the performance is exact.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main (: b Bool)) (host (log) (if b (log.emit "yes") unit)))
      (export main)))
  (call main (: false Bool))
  (output (: unit Unit)))

; Reachability follows into a HIGHER-ORDER call, not only direct/recursive/conditional ones: an effect
; performed inside a CLOSURE passed to a HOF is reached through the closure's call site. Both directions
; are pinned — granted (delegated → runs) and the soundness-critical UNGRANTED (an effect hidden in a
; closure-passed-to-a-HOF is NOT a loophole; the analysis finds it and rejects CDZ0401). The granted case is
; TODO on rust (host-boundary gap); the reject grades on BOTH backends (a compile-time check, no emit).
(case
  "a delegated effect performed in a closure passed to a HOF is reached and fires"
  (doc
    "`log.emit` is performed inside `(fn (u) (log.emit \"hi\"))` passed to `apply-fn`, which calls it.
           Reachability follows the closure through the HOF's call site, so the `(host (log) …)` delegation
           grants it and the program runs, firing one host call. Pins that host-delegation reachability
           traverses a higher-order/indirect call — an effect reached only via a closure argument is granted,
           the HOF companion of the recursive-callee reachability case.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (apply-fn (: f (-> Unit Unit))) (f unit))
      (def (main) (host (log) (apply-fn (fn (u) (log.emit "hi")))))
      (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "hi" String))))

(case
  "an ungranted effect hidden in a closure passed to a HOF is still rejected"
  (doc
    "The soundness-critical direction: the SAME closure `(fn (u) (log.emit \"hi\"))` passed to a HOF,
           but with NO `(host (log) …)` delegation and no handler. The effect is still REACHED through the
           closure's call site, so it is ungranted and the program is rejected CDZ0401 — a closure passed to
           a HOF is NOT a loophole to smuggle an effect past the grant check. Pins that reachability finds an
           effect through a higher-order call for the REJECTION too, not only when granting.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (apply-fn (: f (-> Unit Unit))) (f unit))
      (def (main) (apply-fn (fn (u) (log.emit "hi"))))
      (export main)))
  (error CDZ0401))

(case
  "reaching an effect neither handled nor delegated is rejected at compile time"
  (doc
    "Witnesses capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time Error: main
           performs `log.emit` for an effect no enclosing handler discharges and the entrypoint does not
           delegate to the host, so the effect would escape ungranted and the program is rejected
           (CDZ0401). This is the single 'no home for a reached effect' check — it subsumes both the
           former reached-but-undeclared host operation and the former undischarged intra-program effect
           (CDZ0402, now merged), since host-binding is an entrypoint routing decision rather than a
           declaration-time property.")
  (input (do (effect log (op emit (-> String Unit))) (def (main) (log.emit "ready")) (export main)))
  (error CDZ0401 (message "add a handler or delegate")))

(case
  "a delegation naming an effect that is never reached is rejected as latent authority"
  (doc
    "Witnesses capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative: main
           delegates `log` to the host but its body never performs a `log` operation, so the manifest
           would carry latent authority — a granted capability that is never exercised — and the program
           is rejected (CDZ0404). The manifest must be exactly the effects that escape, no more and no
           fewer.")
  (input (do (effect log (op emit (-> String Unit))) (def (main) (host (log) 42)) (export main)))
  (error CDZ0404))

; The latent case above (CDZ0404) has NO syntactic effect op in its body (`(host (log) 42)`); the runtime-
; conditional case (`(if b …)`) is granted because a runtime branch is reachable. These pin the boundary
; between them: an effect in a STATICALLY-DEAD branch (`(if false …)`). Reachability is SYNTACTIC — a dead-
; branch effect still COUNTS as reached: with a delegation it exercises the grant (NOT latent → not CDZ0404,
; unlike the no-op-at-all latent case), and without one it is still an ungranted effect (CDZ0401, exactly as
; a live one). So the compiler does NOT const-fold `if false` away before the reachability/manifest analysis
; — a precise-reachability change that elided the dead branch would flip both (the grant → latent CDZ0404,
; the ungranted → compiles), so these lock the syntactic-reachability contract.
(case
  "an effect in a statically-dead branch is still syntactically reached, so its delegation is not latent"
  (doc
    "`(host (log) (if false (log.emit \"x\") unit))` delegates `log` and its body SYNTACTICALLY
           contains `log.emit`, even though the `if false` then-branch is never taken. Reachability is
           syntactic, so the effect COUNTS as reached and the delegation is exercised — NOT latent authority
           (contrast the CDZ0404 case above, whose body has no `log` op at all). The program compiles and runs
           to unit (the dead branch makes no host call). Pins that `if false` is NOT const-folded away before
           the manifest/latent analysis — a delegation is latent only when NO syntactic occurrence exists,
           not when the sole occurrence is in dead code.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main) (host (log) (if false (log.emit "x") unit)))
      (export main)))
  (output (: unit Unit)))

(case
  "an effect in a statically-dead branch still requires a grant — ungranted is rejected"
  (doc
    "The dual: the SAME dead-branch `(if false (log.emit \"x\") unit)` with NO delegation is rejected
           CDZ0401 (ungranted effect), exactly as a live `log.emit` is. Syntactic reachability again — the
           dead-branch effect still needs a home, so its absence is a fault; the compiler does not excuse it
           by folding the branch. Together with the case above, pins that a dead-branch effect is treated
           identically to a live one on BOTH sides of the grant boundary (needs a grant; exercises one).")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main) (if false (log.emit "x") unit))
      (export main)))
  (error CDZ0401))

(case
  "the program manifest is the union of its entrypoints' delegations"
  (doc
    "Witnesses capabilities-and-effects.md #The Program Manifest Is The Union Of Its Entrypoints'
           Delegations: main delegates `log` to the host and performs `log.emit`, so the manifest grants
           it and the run makes the host call — the manifest is the union of what the entrypoints
           delegate — then terminates normally with the unit value. The (output …) clause pins the
           terminal condition.")
  (input
    (do
      (effect log (op emit (-> String Unit)))
      (def (main) (host (log) (log.emit "1")))
      (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "1" String))))

(case
  "a component's manifest is the union of two entrypoints' distinct rows"
  (doc
    "Witnesses capabilities-and-effects.md #A Component Is Bound Against The Union Of Its Entrypoints'
           Rows: a component's import surface MUST be the UNION of the escaping rows its entrypoints
           acknowledge — one import surface serving every entrypoint. Two exports delegate DIFFERENT host
           effects: `a` delegates `log` (`(host (log) …)`), `b` delegates `trace` (`(host (trace) …)`).
           Neither reaches the other's effect, yet the component is bound against BOTH: the manifest
           `(. m (meta capabilities))` is `(list \"log\" \"trace\")` — the union, in definition order — not
           either entrypoint's row alone. This is distinct from the single-entrypoint union case (one export,
           one effect): here the union spans two entrypoints with disjoint rows, pinning that provisioning is
           per-COMPONENT even though acknowledgment is per-entrypoint.")
  (input
    (do
      (module m
        (effect log (op emit (-> String Unit)))

        (effect trace (op mark (-> Int64 Unit)))

        (def (a) (host (log) (log.emit "hi")))

        (def (b) (host (trace) (trace.mark 1))))
      (= (. m (meta capabilities)) #list("log" "trace"))))
  (output (: true Bool)))

(case
  "the manifest is DERIVED from what entrypoints delegate, not from effect declarations"
  (doc
    "Witnesses capabilities-and-effects.md #Undeclared Capability Is A Compile-Time Error (2nd
           sentence): the compiler MUST determine a program's required capabilities from the operations its
           entrypoints ACTUALLY reach and delegate, NOT from a separately-asserted list. The module declares
           TWO effects — `Used` and `Unused` — but `main` only delegates `Used` (`(host (Used) …)`); `Unused`
           is declared yet never delegated or performed. So the manifest `(. m (meta capabilities))` is
           exactly `(list \"Used\")` — the declared-but-unreached `Unused` does NOT inflate it. Pins that the
           capability row is DERIVED from actual delegation (a declaration alone grants nothing), so an
           idle declaration can never overstate the component's authority.")
  (input
    (do
      (module m
        (effect Used (op u (-> Unit Int64)))

        (effect Unused (op x (-> Unit Int64)))

        (def (main) (host (Used) (Used.u))))
      (= (. m (meta capabilities)) #list("Used"))))
  (output (: true Bool)))

(case
  "one entrypoint's host authority is not reachable by another that does not delegate it"
  (doc
    "Witnesses capabilities-and-effects.md #Authority Availability Is Not Authority: authority is
           per-entrypoint, not per-component. Entrypoint `a` delegates `ask` to the host (`(host (ask) …)`),
           so the `ask` import is present in the instance for `a`'s sake. Entrypoint `b` performs `(ask.ask)`
           WITHOUT its own enclosing handler or delegation — so even though the `ask` import is AVAILABLE in
           the shared instance, `b` has no authority to reach it: `b` is rejected at compile time (CDZ0401,
           the no-home check, which is scoped to EACH export's body). Availability in the instance is not
           authority in the call graph — an import present for one export is inert for an export whose body
           does not itself grant it, keeping 'no ambient authority' transitive per entrypoint.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (a) (host (ask) (ask.ask)))
      (def (b) (+ (ask.ask) 1))
      (export a)
      (export b)))
  (error CDZ0401))

(case
  "an entrypoint that delegates no effect is pure and makes no host call"
  (doc
    "Witnesses capabilities-and-effects.md #A Host Import Is A Boundary Effect And The Manifest
           Is Its Row: an entrypoint that delegates no effect to the host has the empty effect row, runs
           straight to normal termination with no suspension, and its manifest is empty. (host-calls)
           asserts none was made. This is realized by the seed today (no effect surface needed).")
  (input (do (def (main) 42) (export main)))
  (output (: 42 Int64))
  (host-calls))

(case
  "a program uses a response-returning delegated host function's return value"
  (doc
    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: `ask` is a routing-agnostic effect the entrypoint delegates to the host, and its
           operation's return value is used. The (host-responses …) fixture supplies the response the host
           returns in call order, so the run's result is a deterministic function of input and that
           response; (host-calls …) pins the call.")
  (input
    (do
      (effect ask (op ask (-> Unit Int64)))
      (def (main) (host (ask) (+ 1 (ask.ask))))
      (export main)))
  (host-responses (respond ask.ask (: 41 Int64)))
  (host-calls (call ask.ask))
  (output (: 42 Int64)))

(case
  "a host response SIZES a guest-built collection which is then folded and keyed"
  (doc
    "The response-to-COLLECTION composition (the chain pins above stay scalar): one crossing
           scalar becomes the iteration BOUND of a list build, the built list is folded, and the
           SAME response keys a Map holding the fold — three uses of one boundary value through
           the collection machinery. (rust: host-delegation cases todo per this file's convention.)")
  (input
    (do
      (effect ask (op size (-> Unit Int64)))
      (def
        (build (: i Int64) (: n Int64) (: acc (List Int64)))
        (if (> i n) acc (build (+ i 1) n (List.push acc i))))
      (def
        (sum-l (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main)
        (host
          (ask)
          (do
            (def n (ask.size))
            (def xs (build 1 n #list()))
            (def m (Map.insert Map.empty n (sum-l xs 0)))
            (match (Map.lookup m n) ((Some v) (+ (* v 10) n)) ((None _u) -1)))))
      (export main)))
  (host-responses (respond ask.size (: 3 Int64)))
  (host-calls (call ask.size))
  (output (: 63 Int64))
  (live-objects 0))

(case
  "a host call's response is an ordinary value that feeds a LATER host call"
  (doc
    "Witnesses capabilities-and-effects.md #A Host Call Returns A Response: a host call is a plain
           function call that returns its response to the program, so the response is an ordinary value the
           program computes with — including as the input to a SUBSEQUENT host call. `io.read : Unit ->
           Int64` and `io.scale : Int64 -> Int64` are both delegated; the body is `(io.scale (+ (io.read)
           1))`. The ordered fixture answers `io.read` with 20; the program adds 1 → 21 and passes that as
           `io.scale`'s argument, which the host answers with 42 — so the run yields 42 and makes the two
           host calls IN ORDER (`io.read` then `io.scale`). Pins that a host response is a first-class
           returned value threaded through a normal computation into a later host call's argument (a data
           dependency ACROSS the host boundary), not a side effect the program cannot observe — the
           chained-call companion of the two-independent-calls cases (each response feeds the next step
           rather than combining two independent responses). (wasm: the rust target declines — it lacks the
           host-envelope emission the component-model backend has, the host-boundary parity gap, not an
           effects-fold limitation.)")
  (input
    (do
      (effect io (op read (-> Unit Int64)) (op scale (-> Int64 Int64)))
      (def (main) (host (io) (io.scale (+ (io.read) 1))))
      (export main)))
  (host-responses (respond io.read (: 20 Int64)) (respond io.scale (: 42 Int64)))
  (host-calls (call io.read) (call io.scale))
  (output (: 42 Int64)))

; --- A non-kebab effect / operation name crosses under a normalized component extern name --------------
; A host effect crosses the component boundary at two name-minting sites the value-export path does not
; touch: the effect NAME is the imported WIT interface's extern name, and each operation NAME is a func the
; interface exports. Both must be KEBAB-CASE (the component-model extern-name rule), but a valid Cadenza
; identifier may be uppercase/underscore/camelCase (`Log`, `Ask`, `my_eff`, `askUser`). Emitting such a name
; verbatim yields an INVALID, unloadable component ("import name `Log` is not a valid extern name"). The
; name is NORMALIZED to kebab-case at both effect-boundary sites (the same `kebab_extern_name` the value
; exports use), so a non-kebab effect/op name produces a loadable component; the CORE host import/export
; names (which the program's core module binds against) stay verbatim. An already-lowercase effect+op is
; the identity — byte-identical to before.
(case
  "a host effect with a non-kebab NAME crosses under a normalized interface extern name"
  (doc
    "`(effect Log (op msg (-> Unit Int64)))` delegated via `(host (Log) …)` — `Log` is a valid
           identifier but not a valid component import extern name. The effect name is normalized to the
           kebab interface name `log`; the program still names the effect `Log` in source and the host
           responds to `Log.msg`. Produces a LOADABLE component (was an invalid artifact with no
           diagnostic). The value-export kebab fix (eacfb5f8) did not reach the effect host-import site;
           this pins it.")
  (input
    (do (effect Log (op msg (-> Unit Int64))) (def (main) (host (Log) (Log.msg))) (export main)))
  (host-responses (respond Log.msg (: 0 Int64)))
  (output (: 0 Int64)))

(case
  "a host effect with a non-kebab OPERATION name crosses under a normalized func extern name"
  (doc
    "The operation-name site: `(effect e (op Ask (-> Unit Int64)))` — the op `Ask` is a func the
           imported interface exports, so its extern name must be kebab. It is normalized to `ask` (the
           instance-type export decl and the alias that reads it agree on the kebab name); the source
           performs `e.Ask` and the host responds to `e.Ask`. A loadable component, not the invalid
           artifact `export name Ask is not a valid extern name`.")
  (input (do (effect e (op Ask (-> Unit Int64))) (def (main) (host (e) (e.Ask))) (export main)))
  (host-responses (respond e.Ask (: 0 Int64)))
  (output (: 0 Int64)))

(case
  "a host op with a String parameter composes with the value-heap runtime"
  (doc
    "The shared-memory host shape and the value-heap runtime import compose in ONE component
           (`envelope::assemble_host_runtime_mem`). `main` builds a runtime `List` (the value-heap runtime
           import) AND delegates a host effect `Note` whose op takes a `String` parameter (the shared-memory
           host shape — the `(ptr,len)` a `string` lowers to is read from a memory both the program and the
           op's canon-lower bind). Previously this COMBINATION declined ('a host op with a string parameter
           composed with the value-heap runtime is not yet emitted') while each half alone emitted; the
           envelope now threads the shared-memory core module through the two-interface (host + heap) fusion.
           `build 3` makes a length-3 list; the host `Note.note` fires with its String arg; the terminal
           value is `(List.len xs)` = 3. Unblocks a property test asserting-with-MESSAGE over a heap
           collection (v-property-testing). The (host-calls …) pins the String arg crossing shared memory.")
  (input
    (do
      (effect Note (op note (-> String Unit)))
      (def (build (: n Int64)) (if (= n 0) #list() (List.push (build (- n 1)) n)))
      (def (main) (host (Note) (let ((xs (build 3))) (do (Note.note "built") (List.len xs)))))
      (export main)))
  (output (: 3 Int64))
  (host-calls (call note.note (: "built" String))))

(case
  "an ungranted effect hidden in a COLLECTION-stored closure is still rejected"
  (doc
    "The collection route of the closure-smuggling family (:148 pins the HOF-param route): the
           performing closure is stored in a LIST, extracted by `List.at` through an Option match, and
           applied — the effect is reached through a collection slot + projection rather than a direct
           param, and the grant check must find it there too (CDZ0401). A reachability analysis that
           tracked fn values only through call arguments (not through collection stores/loads) would
           let a list smuggle an ungranted effect past the boundary.")
  (input
    (do
      (effect Net (op fetch (-> Int64 Int64)))
      (def
        (main (: k Int64))
        (do
          (def fs #list((fn ((: x Int64)) (Net.fetch x))))
          (match (List.at fs 0) ((Some f) (f k)) ((None _u) -1))))
      (export main)))
  (error CDZ0401))

; -- host-op runtime bytes/string argument marshaling (migration from rcdzc host-mem arg tests, 2026-08-27;
; per v-platform-itest: no new drive clause — the runtime-compound arg marshaling is transitively asserted by
; the host response value, since broken marshaling traps or returns the wrong value).
(case
  "a runtime Bytes host arg crosses as list<u8> and the host call returns its response"
  (input
    (do
      (effect hb (op h (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (host (hb) (hb.h (Bytes.of #list(((. (UInt 8) wrap) n) ((. (UInt 8) wrap) 66))))))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond hb.h (: 7 Int64)))
  (host-calls (call hb.h))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "a runtime String host arg is marshaled into shared memory and the host call returns its response"
  (input
    (do
      (effect hs (op h (-> String Int64)))
      (def
        (main (: n Int64))
        (match
          (String.from-bytes (Bytes.of #list(((. (UInt 8) wrap) n))))
          ((Some s) (host (hs) (hs.h s)))
          (None 0)))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond hs.h (: 42 Int64)))
  (host-calls (call hs.h))
  (output (: 42 Int64))
  (live-objects known-leak))

(case
  "a host op with two runtime Bytes args marshals each to a disjoint region"
  (input
    (do
      (effect io (op sink2 (-> Bytes Bytes Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (io.sink2
            (Bytes.of #list(((. (UInt 8) wrap) k)))
            (Bytes.of #list(((. (UInt 8) wrap) (+ k 1)))))))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond io.sink2 (: 9 Int64)))
  (host-calls (call io.sink2))
  (output (: 9 Int64))
  (live-objects known-leak))

(case
  "a host op with three runtime Bytes args marshals each to a disjoint region"
  (input
    (do
      (effect io (op sink3 (-> Bytes Bytes Bytes Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (io.sink3
            (Bytes.of #list(((. (UInt 8) wrap) k)))
            (Bytes.of #list(((. (UInt 8) wrap) (+ k 1))))
            (Bytes.of #list(((. (UInt 8) wrap) (+ k 2)))))))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond io.sink3 (: 11 Int64)))
  (host-calls (call io.sink3))
  (output (: 11 Int64))
  (live-objects known-leak))

(case
  "a host op interleaving runtime Bytes and scalar args keeps regions and slots distinct"
  (input
    (do
      (effect io (op mix (-> Bytes Int64 Bytes Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (io.mix
            (Bytes.of #list(((. (UInt 8) wrap) k)))
            (+ k 7)
            (Bytes.of #list(((. (UInt 8) wrap) (+ k 1)))))))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond io.mix (: 3 Int64)))
  (host-calls (call io.mix))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a host op mixing a const String and a runtime Bytes arg routes each to its own path"
  (input
    (do
      (effect io (op mix2 (-> String Bytes Int64)))
      (def
        (main (: k Int64))
        (host (io) (io.mix2 "const-key" (Bytes.of #list(((. (UInt 8) wrap) k))))))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond io.mix2 (: 4 Int64)))
  (host-calls (call io.mix2))
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "an empty runtime String host arg marshals as a zero-length buffer"
  (input
    (do
      (effect hs (op h (-> String Int64)))
      (def
        (main)
        (match (String.from-bytes (Bytes.of #list())) ((Some s) (host (hs) (hs.h s))) (None 0)))
      (export main)))
  (call main)
  (host-responses (respond hs.h (: 99 Int64)))
  (host-calls (call hs.h))
  (output (: 99 Int64)))

(case
  "a multibyte-length runtime String host arg copies the full length"
  (input
    (do
      (effect hs (op h (-> String Int64)))
      (def
        (main (: a Int64))
        (match
          (String.from-bytes
            (Bytes.of #list(((. (UInt 8) wrap) a) ((. (UInt 8) wrap) a) ((. (UInt 8) wrap) a))))
          ((Some s) (host (hs) (hs.h s)))
          (None 0)))
      (export main)))
  (call main (: 65 Int64))
  (host-responses (respond hs.h (: 99 Int64)))
  (host-calls (call hs.h))
  (output (: 99 Int64))
  (live-objects known-leak))

; -- a host-op RESULT flowing into a Bytes-resource escape (migration from rcdzc a_scalar_host_op_result_
; escaping_as_a_bytes_resource_runs_e2e, 2026-08-27): the host response value reaches the escaped Bytes
; resource payload; no new drive clause (existing host-responses + value-escape).
(case
  "a host op result escapes as a single-byte Bytes resource"
  (input
    (do
      (effect hr (op h (-> Int64 UInt8)))
      (def (main (: x Int64)) (host (hr) (Bytes.of #list((hr.h x)))))
      (export main)))
  (call main (: 9 Int64))
  (host-responses (respond hr.h (: 7 UInt8)))
  (host-calls (call hr.h))
  (output (: b"\x07" Bytes))
  (live-objects known-leak))
