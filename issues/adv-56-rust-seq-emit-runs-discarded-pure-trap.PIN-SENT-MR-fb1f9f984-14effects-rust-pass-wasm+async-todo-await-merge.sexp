; adv-56 (breaker, 2026-08-02): the rust backend's new Core::Seq emit (H5, #1451) evaluates a
; DISCARDED PURE trapping item — `{ let _ = <stmt>; … }` runs it — so a do that mixes a dead
; pure computation with a host call spuriously traps, violating the dead-init ruling.
;
; observed:  rust backend traps "division by zero" at main(0).
; expected:  42 + host-calls [io.put] — the discarded `(/ 100 d)` reaches NO perform, so per the
;            §283 dead-init ruling its trap is unobserved and elided (pinned pure-only twin
;            "a discarded trapping item in a do sequence is elided" PASSES on rust); only the
;            io.put item is preserved (the foreign-perform exception covers performs, not pure items).
; brackets:  s4 pure-only do (no host call) → rust PASS (elides). s2 same shape safe divisor →
;            rust PASS. s3 order-swapped (host call first) → rust FAIL identically. s5 handled
;            (non-host) perform + discarded trap → both backends PASS. So the delta is precisely
;            HOST-call-in-do + discarded pure trapping sibling on the rust emit path.
; wasm:      this host-delegated shape currently DECLINES on wasm (todo) — no cross-backend value
;            disagreement, but rust contradicts the corpus-pinned ruling.
; fix shape: the Seq emit (backend/rust/expr.rs, #1451) — or the lower-tier do-fold — must DROP a
;            non-final item that reaches no perform/host call instead of emitting `let _ = <it>;`.
(case "adv-56 a discarded pure trapping item in a do that also makes a host call is elided (rust Seq emit)"
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
