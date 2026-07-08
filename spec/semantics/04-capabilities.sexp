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
  (input  (module m
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "ready")))))
  (output (: unit Unit))
  (host-calls (call log.emit (: "ready" String))))

(case "reaching an effect neither handled nor delegated is rejected at compile time"
  (doc    "Witnesses capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time Error: main
           performs `log.emit` for an effect no enclosing handler discharges and the entrypoint does not
           delegate to the host, so the effect would escape ungranted and the program is rejected
           (CDZ0401). This is the single 'no home for a reached effect' check — it subsumes both the
           former reached-but-undeclared host operation and the former undischarged intra-program effect
           (CDZ0402, now merged), since host-binding is an entrypoint routing decision rather than a
           declaration-time property.")
  (needs  effects)
  (input  (module m
            (effect log (op emit (-> String Unit)))
            (def (main)
              (log.emit "ready"))))
  (error  CDZ0401))

(case "a delegation naming an effect that is never reached is rejected as latent authority"
  (doc    "Witnesses capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative: main
           delegates `log` to the host but its body never performs a `log` operation, so the manifest
           would carry latent authority — a granted capability that is never exercised — and the program
           is rejected (CDZ0404). The manifest must be exactly the effects that escape, no more and no
           fewer.")
  (needs  effects)
  (input  (module m
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                42))))
  (error  CDZ0404))

(case "the program manifest is the union of its entrypoints' delegations"
  (doc    "Witnesses capabilities-and-effects.md #The Program Manifest Is The Union Of Its Entrypoints'
           Delegations: main delegates `log` to the host and performs `log.emit`, so the manifest grants
           it and the run makes the host call — the manifest is the union of what the entrypoints
           delegate — then terminates normally with the unit value. The (output …) clause pins the
           terminal condition.")
  (needs  effects)
  (input  (module m
            (effect log (op emit (-> String Unit)))
            (def (main)
              (host (log)
                (log.emit "1")))))
  (output (: unit Unit))
  (host-calls (call log.emit (: "1" String))))

(case "an entrypoint that delegates no effect is pure and makes no host call"
  (doc    "Witnesses capabilities-and-effects.md #A Host Import Is A Boundary Effect And The Manifest
           Is Its Row: an entrypoint that delegates no effect to the host has the empty effect row, runs
           straight to normal termination with no suspension, and its manifest is empty. (host-calls)
           asserts none was made. This is realized by the seed today (no effect surface needed).")
  (input  (module m
            (def (main)
              42)))
  (output (: 42 Int64))
  (host-calls))

(case "a program uses a response-returning delegated host function's return value"
  (doc    "Witnesses capabilities-and-effects.md #A Run Is A Deterministic Function Of Its Input And
           Responses: `ask` is a routing-agnostic effect the entrypoint delegates to the host, and its
           operation's return value is used. The (host-responses …) fixture supplies the response the host
           returns in call order, so the run's result is a deterministic function of input and that
           response; (host-calls …) pins the call.")
  (needs  effects)
  (input  (module m
            (effect ask (op ask (-> Unit Int64)))
            (def (main)
              (host (ask)
                (+ 1 (ask.ask))))))
  (host-responses (respond ask.ask (: 41 Int64)))
  (host-calls (call ask.ask))
  (output (: 42 Int64)))
