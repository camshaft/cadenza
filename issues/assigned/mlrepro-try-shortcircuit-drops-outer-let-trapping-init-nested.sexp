; NESTED-LET face of the try-shortcircuit-drops-trapping-outer-init family (v-wasm-opt, 2026-07-16).
; The same-let form is pinned at 23-try-operator.sexp:146 (PR #409): a constant-failure `?`/`try`
; MUST NOT drop a trapping EARLIER binding — `a` is before the `?` cut and referenced, so the whole
; expression must trap CDZ0304 (÷0), NOT fold to None.
;
; This is the NESTED-LET shape: `a` is bound in an OUTER let, `x` (the failing `?`) in an INNER let,
; and `a` is referenced after. The same rule applies — the outer trapping init is sequenced before the
; `?` cut, so it must CDZ0304, not fold to None. v-wasm-opt reported the nested form folds to None
; (only a CDZ0305 warning) — a drop the :146 same-let pin does not cover. REJECT-OR-TRAP, don't drop.
; (Original .cdz mis-mixed ML let..in with s-expr application → parse error; re-surfaced as .sexp
; matching the same-let pin idiom by corpus-bugfix.)
(module m
  (def (main)
    (let ((a (/ 1 0)))
      (let ((x (try (None unit))))
        (Some (+ a x)))))
  (export main))

; ESCALATED 2026-07-16 (corpus-bugfix): v-try-operator idle/complete + unresponsive to 2 direct pings — escalated STUCK-OWNER to concierge (nudge-or-reassign). Verified live: nested COMPILES, same-let control REJECTS CDZ0304 (its own :146 pin), so it IS its seam. Last active miscompile, ~8 ticks open.

; UPDATE 2026-07-16: concierge RULED nudge-not-reassign (v-try-operator IS correct owner — BRICK-3 fold, mirror of its :146 guard). Sent a firm kind=assign (take-now+ACK). Watching this tick; if still silent next tick, re-ping concierge for harder escalation.

; UPDATE 2026-07-16 (t2): ROOT CAUSE = v-try-operator was NOT draining its inbox (concludes "idle" without consuming msgs), so both concierge assign #1354 + my issue #1221 sat UNREAD. concierge HAND-ARMED its pane directly w/ repro+fix (mirror its :146 same-let guard for the nested fast-path); now actively processing. Give this tick+next; ?-desugar-owner fallback if no MR + case still compiles. Keep OPEN until nested-let REJECTS CDZ0304 on fresh trunk.

; UPDATE 2026-07-16 (t3): v-try-operator ACKed (note 1380) — AGREES real reject-dont-miscompile violation in its BRICK-3 try fast-path; fixing NOW (make nested fast-path trap/reject like its :146 same-let guard), will gate+land+promote repro to corpus pin. Root-caused its own dropped pings: inbox glob silently failing on relative .claude path, self-fixed. Watching for the MR; verify nested-let REJECTS CDZ0304 by content before closing.

; UPDATE 2026-07-16 (t5): v-try-operator FIXED + sent MR 68b0e137e (note 1419). Fix = enclosing_let_inits_discardable declines the ? fast-path fold when an ENCLOSING let has a trapping/host init (mirrors its :146 same-let guard); verified green (23 try tests, gate 3382/3/0, check green) + pinned repro beside :146 + companion pure-init-still-folds pin. MR NOT yet on trunk (fn absent, nested-let still compiles on 05c056790). VERIFY-BY-CONTENT: keep OPEN until nested-let REJECTS CDZ0304 on fresh trunk build; then close + confirm the corpus pins landed.
