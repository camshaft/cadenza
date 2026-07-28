; BREAKER FINDING — a Symbol ENTRY PARAMETER on the rust targets emits the CADENZA literal syntax
; (#"read") verbatim into the generated Rust driver source, which fails to build:
;   error: expected one of `!` or `[`, found `"read"`
; The check side accepts; wasm declines the Symbol entry marshal (sound todo). Same class as the
; BigInt entry-param E0308 (filed earlier, adv-bigint-entry-param-*): the driver's argument marshal
; lacks a Symbol arm and falls through to emitting the literal's Cadenza text.
;
; Grades: wasm todo(decline) / rust FAIL(no-build) / rust-async FAIL(no-build).
; Expected after fix: rust computes true (then pin per-target like the String-entry family).

(case "a Symbol entry parameter compares to an interned constant"
  (input (do (def (main (: s Symbol)) (= s (Symbol.of "read"))) (export main)))
  (call main (: #"read" Symbol))
  (output (: true Bool)))
