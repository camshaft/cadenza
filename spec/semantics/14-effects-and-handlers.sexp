; Effects and handlers — witnesses capabilities-and-effects.md. A host import is a suspending boundary
; effect; the manifest is the escaping effect row; purity is the empty row. An intra-program effect is
; discharged by a handler and does NOT appear in the manifest. These are (needs effects) cases a later
; generation realizes; the seed realizes the mandatory capability floor (witnessed in 04-capabilities)
; but not the algebraic-handler layer. A response-returning host call fixes its response with
; (host-responses …) so the run is a deterministic function of input and that response.

(case "a run resumes across a host call by replaying the recorded response"
  (doc    "Witnesses capabilities-and-effects.md #Suspension Is Replay From The Host's Log: `ask` is a
           response-returning host import. The host owns the log; re-invoking the run with the recorded
           response advances it. The program holds no resume state — it is re-invokable from its entry.
           The (host-responses …) fixture supplies the response in call order; the run then computes 100.")
  (needs  effects)
  (input  (module m
            (import (host ask (func () Int64)))
            (use (capability ask))
            (def (main)
              (* (ask) 10))))
  (host-responses (respond ask (: 10 Int64)))
  (host-calls (call ask))
  (output (: 100 Int64)))

(case "two host calls replay their responses in order"
  (doc    "Witnesses capabilities-and-effects.md #A Run's Observable Behavior Is A Deterministic
           Function Of Its Input And Responses: two host calls consume two logged responses in the
           order made; the sum is a deterministic function of input and the ordered log.")
  (needs  effects)
  (input  (module m
            (import (host ask (func () Int64)))
            (use (capability ask))
            (def (main)
              (+ (ask) (ask)))))
  (host-responses (respond ask (: 3 Int64))
                  (respond ask (: 4 Int64)))
  (host-calls (call ask) (call ask))
  (output (: 7 Int64)))

(case "an effect discharged by a handler does not escape to the manifest"
  (doc    "Witnesses capabilities-and-effects.md #An Effect That Does Not Escape Is Discharged By A
           Handler and #An Effect Discharged By An In-Program Handler Does Not Appear In The Manifest:
           the `choose` effect is raised in the body and discharged by an enclosing handler that resumes
           it with 5, so the effect never reaches a host function. The program imports no host function,
           so its manifest is empty (host-calls asserts none), yet it uses an effect internally.")
  (needs  effects)
  (input  (module m
            (def (main)
              (handle ((choose () (resume 5)))
                (+ (choose) 1)))))
  (output (: 6 Int64))
  (host-calls))

(case "a handler resumes its continuation at most once by default"
  (doc    "Witnesses capabilities-and-effects.md #A Continuation Is One-Shot By Default: the handler
           resumes the continuation exactly once, so the affine discipline holds and the result is a
           single value (the resumed computation is not duplicated).")
  (needs  effects)
  (input  (module m
            (def (main)
              (handle ((get () (resume 41)))
                (+ (get) 1)))))
  (output (: 42 Int64))
  (host-calls))

(case "a program that imports no host function is pure and never suspends"
  (doc    "Witnesses capabilities-and-effects.md #Purity Is The Empty Effect Row: a program that reaches
           no host function runs straight to normal termination, makes no host call, and has an empty
           manifest. This is the same property the compiler component itself has.")
  (needs  effects)
  (input  (module m
            (def (main) (+ 20 22))))
  (output (: 42 Int64))
  (host-calls))
