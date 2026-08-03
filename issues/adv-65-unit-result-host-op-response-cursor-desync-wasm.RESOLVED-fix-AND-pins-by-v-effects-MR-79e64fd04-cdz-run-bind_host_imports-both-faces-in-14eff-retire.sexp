; adv-65 (breaker, 2026-08-03, HIGH differential wrong-value — response-cursor desync, follow-on
; to H8 #7037ef2c9): a UNIT-RESULT host op does not advance the wasm runner's response cursor,
; so the NEXT value-bearing op reads the unit op's response row. Rust (H8's own lane) is correct.
;
; observed (trunk b641a1724): (do (io.ping k) (+ (io.get k) k)) with responses [ping:99, get:7],
;   k=3 -> wasm 102 (get read the PING row 99: 99+3), rust 10 (get correctly read 7).
;   With ping-row 0 the same shape gives wasm 3 (0+3) — the cursor-probe pair nails the row read.
; brackets: value-op-FIRST order (get then ping) passes BOTH backends (10) — the desync needs a
;   unit-result op BEFORE a value op. A unit-result op ALONE passes both (h8b, 42). Interleaved
;   ping/get/ping (h8c) fails wasm 3 / passes rust 10. opt-sweep clean per face (level-independent).
; expected: per the corpus host-response model (responses consumed IN ORDER of the calls made),
;   the unit-result op consumes its row on BOTH backends — rust's behavior. Either wasm must
;   consume-and-discard the row, or the gate driver shouldn't require a row for unit ops on
;   either backend — but the two MUST agree; today the same case can't be green on both.
; note: h8b (the H8 landing's shape) passes BOTH backends only because a LONE unit op leaves no
;   later value op to expose the cursor — the corpus's H8 cases are all blind to this by shape.
(case "adv-65 a unit-result host op consumes its response row — the next value op reads ITS OWN row"
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
