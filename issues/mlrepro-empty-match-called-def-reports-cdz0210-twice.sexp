;; DIAGNOSTIC DUP (v-inference, low severity) — a CALLED def with a zero-arm `(match x)` on an inhabited
;; scrutinee reports CDZ0210 "a zero-arm match is exhaustive only on an uninhabited (Never) scrutinee…"
;; TWICE: once at the def body, once at the CALL SITE (the reduced body re-runs the exhaustiveness check).
;; An UNCALLED def reports it ONCE. Same class as the _w47 member-access / tuple-index bare+rich dup and
;; the tuple-by-name call-site cascade: the SAME coded fault at DIFFERENT nodes (def vs call-site), so the
;; node-keyed `coded_nodes` dedup misses the pair. FIX: fold into `dedup_faults` (compile.rs) — drop the
;; call-site CDZ0210 when the def-body CDZ0210 with the same message is present (a `has_zero_arm_match_reject`
;; flag + message-match drop, exactly like the tuple-by-name `MEMBER_NOT_RECORD_DECLINE` drop). DEFERRED:
;; batch with the next dedup_faults change AFTER _w47 lands (avoid stacking on the unlanded _w47, same file).
;; LOW severity (both reports name the real fault; just noisy). The uncalled single-report is correct.
(module m (def (f (: x Int64)) (match x)) (def (main) (f 1)) (export main))
