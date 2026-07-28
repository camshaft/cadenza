;; PIN-ON-LAND: add to spec/semantics/15-rows-and-open-sums.sexp (beside the l6 Record.with runtime case)
;; once v-inference's materialize-once fix (MR 13fd27095, stacks on 5cdc957da) lands. The reviewer found
;; that runtime_record_fields re-emitted the operand once PER preserved field (no backend CSE); the fix
;; let-binds the operand once via a self-keyed Core::Let { (record, record) } so every synth projection
;; reads a shared LocalRef. Value was already correct for pure operands (this case = 12), so it PASSES
;; both pre- and post-fix — but pinning it LOCKS the multi-preserved-field runtime-Record.with path (the
;; landed l6 pin is only a SINGLE-preserved-field borrowed-projection shape, which never re-emitted).
;;
;; NOTE: this is the VALUE-correctness half. The EFFECT-COUNT witness (operand performs an effect; must
;; fire ONCE not N times) is the true miscompile witness — v-inference is trying to hand a compiling
;; effectful-operand shape (the effectful-do-returning-record declines for an unrelated reason in both our
;; attempts). v-inference's rcdzc lib test already pins emit-once STRUCTURALLY (constant-appears-once)
;; meanwhile. ON a compiling effectful shape: add a perform-count row too.
;;
;; ON LAND (13fd27095 on trunk): gate this case PASS on wasm+rust+rust-async, insert beside the l6 runtime
;; Record.with case in 15-rows, baseline (1 pass) x3, verify titles-agree/0-dup/0-omission + gate --check
;; all 3 + roundtrip, commit + MR, notify v-inference (+ request the effectful shape for the perform-count row).

(case "a Record.with over a runtime record with MULTIPLE preserved fields evaluates the operand once"
  (doc    "The materialize-once discipline for a runtime-record row-op (v-inference 13fd27095, after the
           reviewer's re-emit finding on 49d6eec14). `runtime_record_fields` builds a fresh record from a
           `(. record field)` projection for EVERY unchanged field; before the fix the raw operand was
           re-emitted once per preserved field (the backend has no CSE — each Core::Proj re-calls
           emit(operand)), an N-fold redundant eval (perf cliff for a pure operand, an observable MISCOMPILE
           for an effectful one). The fix let-binds the runtime operand ONCE (self-keyed Core::Let) so every
           projection reads a shared LocalRef. Here `(mk v)` is a 3-field runtime record; `Record.with … a 99`
           updates `a` and leaves TWO preserved fields (`b`, `c`), so the operand `(mk v)` would re-emit
           twice without the fix. Reading the preserved `c` → v+2 = 12 at v=10 (value correct either way;
           the pin LOCKS the multi-preserved-field path the single-field l6 case does not exercise). Both
           backends. The effect-count face — the operand performs an effect exactly once, not per preserved
           field — is pinned structurally in rcdzc's lib test and follows here once a compiling
           effectful-operand shape is available.")
  (input  (do
            (def (mk (: n Int64)) (record (a n) (b (+ n 1)) (c (+ n 2))))
            (def (main (: v Int64)) (. (Record.with (mk v) a 99) c))
            (export main)))
  (call   main (: 10 Int64)) (output (: 12 Int64)))
