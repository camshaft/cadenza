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
; These carry (needs effects): the surface is realized when the seed's reader learns the (effect …)
; declaration and the entrypoint (host …) delegation. Until then the seed skips them; it still enforces
; the capability floor itself once it lowers the new surface.

(case "an entrypoint delegation lets a program reach its host function"
  (doc    "Witnesses capabilities-and-effects.md #An Entrypoint Delegates The Capabilities It Grants To
           The Host and #Host-Binding Is A Routing Decision Made At The Entrypoint: `log` is declared as
           a routing-agnostic effect, and main DELEGATES it to the host with a (host (log) …) form, so
           `log.emit` is bound at the boundary (host-interface-binding.md #A Host Import Is A WIT-Typed
           Function The Manifest Enumerates) — the delegation IS the manifest grant — and the run makes
           the host call, then terminates normally with the unit value (the operation's WIT result is
           Unit). The (output …) clause pins the terminal condition and (host-calls …) pins the ordered
           host-call observation.")
  (needs  effects)
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "ready"))) (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "ready" String))))

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

(case "an entrypoint delegation reaches an effect performed in a recursive callee"
  (doc    "`main` delegates `log` with `(host (log) …)` and calls a recursive `go` that performs
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
  (needs  effects)
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (go n)
              (if (= n 0)
                  unit
                  (do (log.emit "x") (go (- n 1)))))
            (def (main)
              (host (log)
                (go 1))) (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "x" String))))

(case "reaching an effect neither handled nor delegated is rejected at compile time"
  (doc    "Witnesses capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time Error: main
           performs `log.emit` for an effect no enclosing handler discharges and the entrypoint does not
           delegate to the host, so the effect would escape ungranted and the program is rejected
           (CDZ0401). This is the single 'no home for a reached effect' check — it subsumes both the
           former reached-but-undeclared host operation and the former undischarged intra-program effect
           (CDZ0402, now merged), since host-binding is an entrypoint routing decision rather than a
           declaration-time property.")
  (needs  effects)
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (log.emit "ready")) (export main)))
  (error  CDZ0401))

(case "a delegation naming an effect that is never reached is rejected as latent authority"
  (doc    "Witnesses capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative: main
           delegates `log` to the host but its body never performs a `log` operation, so the manifest
           would carry latent authority — a granted capability that is never exercised — and the program
           is rejected (CDZ0404). The manifest must be exactly the effects that escape, no more and no
           fewer.")
  (needs  effects)
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                42)) (export main)))
  (error  CDZ0404))

(case "the program manifest is the union of its entrypoints' delegations"
  (doc    "Witnesses capabilities-and-effects.md #The Program Manifest Is The Union Of Its Entrypoints'
           Delegations: main delegates `log` to the host and performs `log.emit`, so the manifest grants
           it and the run makes the host call — the manifest is the union of what the entrypoints
           delegate — then terminates normally with the unit value. The (output …) clause pins the
           terminal condition.")
  (needs  effects)
  (input  (do
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "1"))) (export main)))
  (output (: unit Unit))
  (host-calls (call log.emit (: "1" String))))

(case "an entrypoint that delegates no effect is pure and makes no host call"
  (doc    "Witnesses capabilities-and-effects.md #A Host Import Is A Boundary Effect And The Manifest
           Is Its Row: an entrypoint that delegates no effect to the host has the empty effect row, runs
           straight to normal termination with no suspension, and its manifest is empty. (host-calls)
           asserts none was made. This is realized by the seed today (no effect surface needed).")
  (input  (do
            (def (main)
              42) (export main)))
  (output (: 42 Int64))
  (host-calls))

(case "a program uses a response-returning delegated host function's return value"
  (doc    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: `ask` is a routing-agnostic effect the entrypoint delegates to the host, and its
           operation's return value is used. The (host-responses …) fixture supplies the response the host
           returns in call order, so the run's result is a deterministic function of input and that
           response; (host-calls …) pins the call.")
  (needs  effects)
  (input  (do
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (+ 1 (ask.ask)))) (export main)))
  (host-responses (respond ask.ask (: 41 Int64)))
  (host-calls (call ask.ask))
  (output (: 42 Int64)))
