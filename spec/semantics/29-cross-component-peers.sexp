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

; ── peer ops returning COLLECTIONS cross as handles and are read over the shared runtime ──────────
; (migrated from the in-crate rcdzc run_with_peers list-result tests). A heap-RESULT peer op is
; host-wrapped; the crossed collection is a runtime handle read back (len / at / field projection).
(case "pcl1 a peer op returning a list crosses as a handle and its length is read"
  (doc    "PROVIDER `dup` returns a 3-element list; the consumer binds it and reads List.len of the crossed
           list over the shared runtime: dup(7)=[7,7,7] → 3.")
  (peer   "cadenza:l/api" (do (def (dup (: x Int64)) (list x x x)) (export dup)))
  (input  (do (effect L (op dup (-> Int64 (List Int64)))) (bind L "cadenza:l/api")
              (def (main (: x Int64)) (host (L) (List.len (L.dup x)))) (export main)))
  (call   main (: 7 Int64))
  (output (: 3 Int64)))

(case "pcl2 a peer op returning a tuple with a variable-length list element crosses, both fields read"
  (doc    "PROVIDER `mk` returns `(tuple (list x (+ x 1) (+ x 2)) (* x 10))` — a runtime-built list paired
           with a scalar; the consumer reads BOTH fields of the crossed tuple: mk(4)=([4,5,6],40),
           List.len(field0)=3 + field1=40 = 43 (the dynamic-depth element survived the crossing).")
  (peer   "cadenza:p/api" (do (def (mk (: x Int64)) (tuple (list x (+ x 1) (+ x 2)) (* x 10))) (export mk)))
  (input  (do (effect P (op mk (-> Int64 (Tuple (List Int64) Int64)))) (bind P "cadenza:p/api")
              (def (main (: x Int64)) (host (P) (+ (List.len (. (P.mk x) 0)) (. (P.mk x) 1)))) (export main)))
  (call   main (: 4 Int64))
  (output (: 43 Int64)))

(case "pcl3 an element of a peer-returned list is read and used"
  (doc    "PROVIDER `mklist(x)` = [x+1, x+2]; the consumer reads element 0 with List.at and unwraps the
           Option to the scalar the entrypoint returns: mklist(7)=[8,9], List.at 0 → Some 8 → 8.")
  (peer   "cadenza:e/api" (do (def (mklist (: x Int64)) (list (+ x 1) (+ x 2))) (export mklist)))
  (input  (do (effect L (op mklist (-> Int64 (List Int64)))) (bind L "cadenza:e/api")
              (def (main (: x Int64)) (host (L) (match (List.at (L.mklist x) 0) ((Some v) v) (None 0)))) (export main)))
  (call   main (: 7 Int64))
  (output (: 8 Int64)))

; ── peer ops returning MAP/SET results cross as handles read over the shared runtime ───────────────
; (the map/set analogue of the list-result cases above). A peer op building a Map or Set has its perform
; host-wrapped; the crossed collection is a runtime handle read back (len / lookup / membership).
(case "pcm1 a peer op returning a map crosses as a handle and its length is read"
  (doc    "PROVIDER `mk(x)` builds a 2-entry (Map Int64 Int64); the consumer binds it and reads Map.len of the
           crossed map over the shared runtime: mk(7) = {1:7, 2:8} → 2 (the runtime-built map survived the
           cross-component boundary as a handle).")
  (peer   "cadenza:mm/api" (do (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 x) 2 (+ x 1))) (export mk)))
  (input  (do (effect M (op mk (-> Int64 (Map Int64 Int64)))) (bind M "cadenza:mm/api")
              (def (main (: x Int64)) (host (M) (Map.len (M.mk x)))) (export main)))
  (call   main (: 7 Int64))
  (output (: 2 Int64)))

(case "pcm2 a value of a peer-returned map is read by key and used"
  (doc    "PROVIDER `mk(x)` = {1:x+10, 2:x+20}; the consumer looks up key 2 in the crossed map and unwraps
           the Option to the scalar the entrypoint returns: mk(5) = {1:15, 2:25}, Map.lookup 2 → Some 25 → 25
           (a VALUE read off the crossed map, not just its size).")
  (peer   "cadenza:mv/api" (do (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 (+ x 10)) 2 (+ x 20))) (export mk)))
  (input  (do (effect M (op mk (-> Int64 (Map Int64 Int64)))) (bind M "cadenza:mv/api")
              (def (main (: x Int64)) (host (M) (match (Map.lookup (M.mk x) 2) ((Some v) v) (None 0)))) (export main)))
  (call   main (: 5 Int64))
  (output (: 25 Int64)))

(case "pcs1 a peer op returning a set crosses as a handle and its length is read"
  (doc    "PROVIDER `mk(x)` builds a 2-element (Set Int64) {x, x+1} (distinct); the consumer reads Set.len of
           the crossed set over the shared runtime: mk(7) = {7,8} → 2 (the runtime-built set crossed as a
           handle; the CHAMP survived the boundary).")
  (peer   "cadenza:ss/api" (do (def (mk (: x Int64)) (Set.insert (Set.insert (Set.of (list)) x) (+ x 1))) (export mk)))
  (input  (do (effect S (op mk (-> Int64 (Set Int64)))) (bind S "cadenza:ss/api")
              (def (main (: x Int64)) (host (S) (Set.len (S.mk x)))) (export main)))
  (call   main (: 7 Int64))
  (output (: 2 Int64)))

(case "pcs2 membership of a peer-returned set is queried for a present and an absent element"
  (doc    "PROVIDER `mk(x)` = {x, x+1}; the consumer queries Set.contains on the crossed set for a PRESENT
           element (x) and an ABSENT one (99): mk(7) = {7,8}, contains 7 → yes (+10), contains 99 → no (+0)
           → 10. Pins that membership reads correctly over the crossed CHAMP on both faces.")
  (peer   "cadenza:sc/api" (do (def (mk (: x Int64)) (Set.insert (Set.insert (Set.of (list)) x) (+ x 1))) (export mk)))
  (input  (do (effect S (op mk (-> Int64 (Set Int64)))) (bind S "cadenza:sc/api")
              (def (main (: x Int64))
                (host (S) (+ (if (Set.contains (S.mk x) x) 10 0) (if (Set.contains (S.mk x) 99) 1 0))))
              (export main)))
  (call   main (: 7 Int64))
  (output (: 10 Int64)))

(case "pcm3 a peer-returned map enumerates via Map.to-list and its values fold to a sum"
  (doc    "PROVIDER `mk(x)` = {1:x+10, 2:x+20}; the consumer enumerates the crossed map with Map.to-list and
           folds the entry VALUES to a sum over the shared runtime: mk(5) = {1:15, 2:25}, sum of values = 40.
           Pins that the crossed map is fully ENUMERABLE (not only point-queried) and its entry values cross
           intact — the map analogue of the list-fold peer cases.")
  (peer   "cadenza:mf/api" (do (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 (+ x 10)) 2 (+ x 20))) (export mk)))
  (input  (do (effect M (op mk (-> Int64 (Map Int64 Int64)))) (bind M "cadenza:mf/api")
              (def (sumv (: es (List (Tuple Int64 Int64))) (: acc Int64))
                (match es ((list) acc) ((list e .. rest) (sumv rest (+ acc (. e 1))))))
              (def (main (: x Int64)) (host (M) (sumv (Map.to-list (M.mk x)) 0)))
              (export main)))
  (call   main (: 5 Int64))
  (output (: 40 Int64)))

; -- a LIST argument crosses INBOUND to a peer as a handle (migrated from rcdzc
; a_list_argument_crosses_inbound_to_a_peer_as_a_handle): the inbound twin of the list-RESULT crossing —
; a List has a distinct runtime rep (RRB vector) but crosses as a handle the peer dereferences.
(case "pla1 a list argument crosses inbound to a peer and is read by the peer op"
  (doc    "PROVIDER `total : (List Int64) -> Int64` returns the list's length; the consumer builds
           `(list 10 20 30)` on the shared runtime and passes its handle into the peer op:
           total([10,20,30]) = 3.")
  (peer   "cadenza:l/api" (do (def (total (: xs (List Int64))) (List.len xs)) (export total)))
  (input  (do (effect L (op total (-> (List Int64) Int64))) (bind L "cadenza:l/api")
              (def (main) (host (L) (L.total (list 10 20 30)))) (export main)))
  (call   main)
  (output (: 3 Int64)))

; -- a peer op returning a (List <user-sum>) crosses to a peer executor (migrated from rcdzc
; the_agent_kernel_list_of_hostop_result_runs_through_a_peer_executor): the agent-kernel seam — interpret
; returns a branch-built (List HostOp) (a List of a String-payload user sum) that crosses peer->peer as a
; handle; the executor reads its length. Both sides declare the shared HostOp type.
(case "pak1 a peer op returning a (List of a user sum) crosses and its length is read"
  (doc    "PROVIDER `interpret(kind, turn)` returns a branch-built (List HostOp): kind=1 -> [Append, Exec]
           (2 ops). The consumer binds it and reads List.len of the crossed list: interpret(1,0) -> len 2.")
  (peer   "cadenza:agent/kernel"
          (do (type HostOp (Append String) (Exec String) (Http String) (Noop Int64))
              (def (interpret (: kind Int64) (: turn Int64)) (if (= kind 1) (list (Append "a") (Exec "e")) (list (Noop 0))))
              (export interpret)))
  (input  (do (type HostOp (Append String) (Exec String) (Http String) (Noop Int64))
              (effect K (op interpret (-> Int64 Int64 (List HostOp)))) (bind K "cadenza:agent/kernel")
              (def (main (: kind Int64)) (host (K) (List.len (K.interpret kind 0)))) (export main)))
  (call   main (: 1 Int64))
  (output (: 2 Int64)))
(case "two effects bound to the same interface share one peer instance (both ops run)"
  (doc    "ONE provider component exports `fa` (adds 10) and `fb` (adds 20) on `cadenza:x/y`; the consumer
           declares TWO effects A and B and binds BOTH to `cadenza:x/y` — a legal dedup on the EFFECT name,
           merging both ops onto a SINGLE `cadenza:x/y` instance import (not two colliding imports, which
           would be a silent-invalid component). `main = A.fa(1) + B.fb(2) = 11 + 22 = 33`, proving both
           effects route to the one shared provider instance and each op returns its own result. Relocated
           (RUN half) from the in-crate rcdzc `two_effects_bound_to_the_same_interface_share_one_peer_instance`
           — its white-box single-instance-import structural pin stays in rcdzc.")
  (peer   "cadenza:x/y" (do (def (fa (: x Int64)) (+ x 10)) (def (fb (: x Int64)) (+ x 20)) (export fa) (export fb)))
  (input  (do (effect A (op fa (-> Int64 Int64))) (effect B (op fb (-> Int64 Int64)))
              (bind A "cadenza:x/y") (bind B "cadenza:x/y")
              (def (main) (host (A B) (+ (A.fa 1) (B.fb 2)))) (export main)))
  (call   main)
  (output (: 33 Int64)))

; ── map/set peer-result DEPTH: empty-collection edge, nested map-of-list, canonical enumeration order ──
(case "pcm4 a peer op returning an EMPTY map crosses and reads length zero"
  (doc    "The size-0 boundary: PROVIDER `mk` returns an empty (Map Int64 Int64); the consumer reads Map.len
           of the crossed empty map → 0. Pins that an empty collection crosses as a VALID handle (not a
           null/absent result) and its length reads zero.")
  (peer   "cadenza:me/api" (do (def (mk (: x Int64)) (: (Map.empty) (Map Int64 Int64))) (export mk)))
  (input  (do (effect M (op mk (-> Int64 (Map Int64 Int64)))) (bind M "cadenza:me/api")
              (def (main (: x Int64)) (host (M) (Map.len (M.mk x)))) (export main)))
  (call   main (: 0 Int64))
  (output (: 0 Int64)))

(case "pcs3 a peer op returning an EMPTY set crosses and reads length zero"
  (doc    "The size-0 boundary for sets: PROVIDER `mk` returns an empty (Set Int64); Set.len of the crossed
           empty set → 0. The set analogue of the empty-map edge — an empty CHAMP crosses as a valid handle.")
  (peer   "cadenza:se/api" (do (def (mk (: x Int64)) (: (Set.of (list)) (Set Int64))) (export mk)))
  (input  (do (effect S (op mk (-> Int64 (Set Int64)))) (bind S "cadenza:se/api")
              (def (main (: x Int64)) (host (S) (Set.len (S.mk x)))) (export main)))
  (call   main (: 0 Int64))
  (output (: 0 Int64)))

(case "pcm5 a peer op returning a map of lists crosses and a value list's length is read"
  (doc    "Nested collection: PROVIDER `mk(x)` = {1: [x, x+1]} — a (Map Int64 (List Int64)); the consumer
           looks up key 1 in the crossed map and reads List.len of the value list: mk(5) = {1:[5,6]},
           Map.lookup 1 → Some [5,6], List.len → 2. Pins that a nested map-of-list crosses with the inner
           list intact (the value's dynamic depth survived the boundary).")
  (peer   "cadenza:ml/api" (do (def (mk (: x Int64)) (Map.insert (Map.empty) 1 (list x (+ x 1)))) (export mk)))
  (input  (do (effect M (op mk (-> Int64 (Map Int64 (List Int64))))) (bind M "cadenza:ml/api")
              (def (main (: x Int64)) (host (M) (match (Map.lookup (M.mk x) 1) ((Some l) (List.len l)) (None 0)))) (export main)))
  (call   main (: 5 Int64))
  (output (: 2 Int64)))

(case "pcm6 a peer-returned map enumerates in canonical key order regardless of insert order"
  (doc    "Ordering across the crossing: PROVIDER `mk(x)` inserts keys 3, 1, 2 (NON-sorted insert order); the
           consumer reads the FIRST entry's key via Map.to-list. Canonical KEY order is 1,2,3, so the head
           key is 1 regardless of insert order. Pins that deterministic canonical enumeration order survives
           the cross-component boundary (the crossed map is not left in insert order).")
  (peer   "cadenza:mo/api" (do (def (mk (: x Int64)) (Map.insert (Map.insert (Map.insert (Map.empty) 3 x) 1 x) 2 x)) (export mk)))
  (input  (do (effect M (op mk (-> Int64 (Map Int64 Int64)))) (bind M "cadenza:mo/api")
              (def (main (: x Int64)) (host (M) (match (Map.to-list (M.mk x)) ((list e .. rest) (. e 0)) ((list) -1)))) (export main)))
  (call   main (: 9 Int64))
  (output (: 1 Int64)))
