; FUTURE INCREMENT (v-effects, 2026-07-17). NOT a miscompile — a CLEAN DECLINE (honest "not yet reducible"
; todo). The task #15 fix (MR fa88f66cf: caller-observed cross-fn recursive fold out-state now threads via
; forced multi-value specialization) threads a SELF-recursive callee's out-state to a caller's continuation.
; But a MUTUALLY-recursive callee (ea<->eb SCC, each performing a discharged op), whose out-state a later
; caller-spine item observes, is DECLINED cleanly (guard callee_calls_other_recursive_def) rather than folded:
; the multi-value tuple machinery (thread_returning_tuple + the multi-value self-call arm) threads a SELF-call's
; out-state but NOT a mutual-SCC SIBLING's, so forcing multi-value there LEAKS the internal $s0/$t0 state-param
; names (a confusing CDZ0101). Declining is correct + safe (pinned by
; a_caller_observed_mutually_recursive_fold_declines_cleanly_no_leak, asserts no $s/$t/#eff leak).
;
; TO BUILD (when a consumer forces it — none today): group-wide multi-value specialization over the whole
; mutual-recursive SCC, so each SCC member returns (value, out-state) and the sibling call threads the partner's
; out-state (the mutual analogue of the self-call tuple arm). Miscompile-prone (state threading across an SCC);
; gate the sharp bisection: mutual pair each performing, followed by a trailing perform / a second fold /
; sibling calls. Breaker witness: (def (ea n) (if (= n 0) 0 (+ (eb (- n 1)) (Counter.bump)))) (def (eb n) ...
; symmetric) ; (+ (* 1000 (ea 3)) (Counter.bump)) currently DECLINES; a group-wide fix would fold it.
;
; SEVERITY: none (clean decline). Promote only when a self-hosted / consumer pass needs a mutual-recursive
; effectful walk with an observed handler-state accumulator. Tracked in v-effects memory
; queued-branch-outstate-lost-to-later-perform.

(do
  (effect Counter (op bump (-> Int64)))
  (def (ea (: n Int64)) (if (= n 0) 0 (+ (eb (- n 1)) (Counter.bump))))
  (def (eb (: n Int64)) (if (= n 0) 0 (+ (ea (- n 1)) (Counter.bump))))
  (def (main)
    (handle Counter 0 ((bump () s (resume s (+ s 1))))
      (+ (* 1000 (ea 3)) (Counter.bump))))
  (export main))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk 7a065bbf7/995fa4134) =====
; NOT A BUG — CONFIRMED clean intentional decline on trunk: the exact repro program declines with the honest
; "this handler is not yet reducible by the tail-resumptive fold (cross-function or non-tail resume arrives in
; a later increment)" — NO $s0/$t0 leak, NO confusing CDZ0101. Severity NONE. This is a v-effects FUTURE
; INCREMENT (group-wide multi-value specialization over a mutual-recursive SCC), to build only when a consumer
; forces it (none today). Owned + tracked in v-effects memory (queued-branch-outstate-lost-to-later-perform).
; Renamed .FUTURE-INCREMENT-v-effects so it stops surfacing as untriaged. No route/agent needed.
