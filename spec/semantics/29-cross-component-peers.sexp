; ── Cross-component PEER composition (the `(peer <iface> <provider>)` corpus clause) ──────────────
;
; A `(peer "IFACE" PROVIDER)` clause ships a STANDALONE provider program compiled with
; `--component-name IFACE`; the `(input …)` consumer binds that interface with the imposed-world
; form `(effect E (op …)) (bind E "IFACE") (host (E) (E.op …))`, and the harness composes them
; (`run_with_peers` / nix `cdz-run --peer <iface>=<peer.wasm>`) into one execution. These pin the
; cross-component boundary that used to live only as in-crate rcdzc `run_with_peers` tests.
;
; AUTHORING RULES (verified against the compose path + breaker's differential):
;   • bind-ONLY — the consumer declares `(effect E …)` + `(bind E "IFACE")`; do NOT add an explicit
;     `(wit-world …)` clause (the wit-world+peer combination yields an unparseable component).
;   • the PROVIDER is a bare `(do (def (op …) …) (export op))` — no effect/bind/wit-world inside it.
;   • a heap-RESULT peer op (String/Bytes/collection) must have its perform HOST-WRAPPED
;     `(host (E) (… (E.op …)))` — an un-wrapped perform is CDZ0401 (EffectNoHome), not a frontier.
;   • a compose-time signature reject surfaces as a TRAP graded by canonical kind: arity/type →
;     `(trap "signature mismatch")` or `(trap "type mismatch")` (both = PeerSignatureMismatch);
;     a missing bound op → `(trap "does not export op")` (PeerMissingInterface).
;
; The rust / rust-async backends have no peer-compose path yet, so these grade PASS on wasm and
; TODO on rust/rust-async (a legitimate not-yet, not a regression).

(case "a peer op arity mismatch is rejected at compose time, not a trap"
  (doc    "PROVIDER exports `add` taking ONE argument; the CONSUMER binds `Math.add` as taking TWO — an
           arity mismatch. Composition MUST be rejected at compose time (the signature check names the op
           and both arities: 2 vs 1), not run to an opaque runtime trap. Relocated from the in-crate
           rcdzc `a_peer_op_arity_mismatch_is_rejected_at_compose_time_not_a_trap`.")
  (peer   "cadenza:math/api" (do (def (add (: x Int64)) (+ x 1)) (export add)))
  (input  (do (effect Math (op add (-> Int64 Int64 Int64))) (bind Math "cadenza:math/api")
              (def (main (: x Int64)) (host (Math) (Math.add x x))) (export main)))
  (call   main (: 5 Int64))
  (trap   "signature mismatch"))

(case "a peer op type mismatch is rejected at compose time (same arity, different type)"
  (doc    "PROVIDER exports `neg` over Float64; the CONSUMER binds `M.neg` over Int64 — SAME arity (1→1),
           DIFFERENT boundary type (S64 vs Float64). The signature check rejects it at compose time naming
           the op, the position (argument 0), and both types. Relocated from the in-crate rcdzc
           `a_peer_op_type_mismatch_is_rejected_at_compose_time`.")
  (peer   "cadenza:m/api" (do (def (neg (: x Float64)) (+ x x)) (export neg)))
  (input  (do (effect M (op neg (-> Int64 Int64))) (bind M "cadenza:m/api")
              (def (main (: x Int64)) (host (M) (M.neg x))) (export main)))
  (call   main (: 5 Int64))
  (trap   "type mismatch"))

(case "a peer missing a bound op is rejected naming the op"
  (doc    "PROVIDER exports only `add`; the CONSUMER binds BOTH `add` and `sub` on the interface — the peer
           is missing `sub`. Composition is rejected naming the missing op (and what the peer DOES offer),
           not an opaque linker instance error. Relocated from the in-crate rcdzc
           `a_peer_missing_a_bound_op_is_rejected_naming_the_op`.")
  (peer   "cadenza:m/api" (do (def (add (: x Int64) (: y Int64)) (+ x y)) (export add)))
  (input  (do (effect M (op add (-> Int64 Int64 Int64)) (op sub (-> Int64 Int64 Int64)))
              (bind M "cadenza:m/api")
              (def (main (: x Int64)) (host (M) (+ (M.add x x) (M.sub x x)))) (export main)))
  (call   main (: 5 Int64))
  (trap   "does not export op"))
