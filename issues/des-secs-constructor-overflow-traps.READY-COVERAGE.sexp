;; READY-TO-LAND DES coverage pin — append to spec/semantics/27-discrete-event-simulation.sexp
;; the moment MR d7ace7b04 (since-underflow) lands + a clean fleet sync. Third of the
;; time-arithmetic safety-edge family: clock add-overflow (at) + span sub-underflow (since)
;; are pinned; this pins constructor MUL-overflow (secs = n * 1e9). Verified PASS (probe):
;; secs(18446744074) overflows the *1e9 and TRAPS; control secs(5)=5e9 returns.
;; Baseline anchor for surgical +1 insert (alphabetical): description starts "a Duration
;; constructor scaling a count past…" → sorts in the "a D…" block near the existing
;; "a Duration constructor `secs` scales…" line.

(case "a Duration constructor scaling a count past the UInt64 nanosecond range traps rather than wrapping"
  (doc    "The constructor-boundary safety property, completing the time-arithmetic overflow family with
           the clock-overflow (`at`) and span-underflow (`since`) cases. `secs` scales its `UInt64` count
           by 1e9 (§3.2), so a count large enough that `n * 1_000_000_000` exceeds `UInt64.max` (~1.8e10
           seconds ≈ 584 years) OVERFLOWS the multiply and TRAPS 'integer overflow' — it does NOT silently
           wrap to a tiny `Duration`. This is load-bearing for the same reason as the clock/span cases: a
           silent wrap would hand the scheduler a `Duration` far smaller than intended, firing a
           long-delay event almost immediately and misordering the event queue. Trapping turns an
           over-long duration into a clean failure. Graded via a runtime `(call …)` arg so the multiply is
           a real instruction: `secs(18446744074)` (just past `UInt64.max / 1e9`) traps; the control
           `secs(5)` = 5_000_000_000 returns normally.")
  (input  (do
            (type Duration (Duration UInt64))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (dur-ns (: d Duration)) (match d ((Duration.Duration v) v)))
            (def (main (: n UInt64)) (dur-ns (secs n)))
            (export main)))
  (call   main (: 18446744074 UInt64))
  (trap   "integer overflow")
  (call   main (: 5 UInt64))
  (output (: 5000000000 UInt64)))
