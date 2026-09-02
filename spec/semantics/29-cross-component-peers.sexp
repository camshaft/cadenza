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
(diagnostic-quality)

(case
  "a peer op arity mismatch is rejected at compose time, not a trap"
  (doc
    "PROVIDER exports `add` taking ONE argument; the CONSUMER binds `Math.add` as taking TWO — an
           arity mismatch. Composition MUST be rejected at compose time (the signature check names the op
           and both arities: 2 vs 1), not run to an opaque runtime trap. Relocated from the in-crate
           rcdzc `a_peer_op_arity_mismatch_is_rejected_at_compose_time_not_a_trap`.")
  (peer "cadenza:math/api" (do (def (add (: x Int64)) (+ x 1)) (export add)))
  (input
    (do
      (effect Math (op add (-> Int64 Int64 Int64)))
      (bind Math "cadenza:math/api")
      (def (main (: x Int64)) (host (Math) (Math.add x x)))
      (export main)))
  (call main (: 5 Int64))
  (trap "signature mismatch"))

(case
  "a peer op type mismatch is rejected at compose time (same arity, different type)"
  (doc
    "PROVIDER exports `neg` over Float64; the CONSUMER binds `M.neg` over Int64 — SAME arity (1→1),
           DIFFERENT boundary type (S64 vs Float64). The signature check rejects it at compose time naming
           the op, the position (argument 0), and both types. Relocated from the in-crate rcdzc
           `a_peer_op_type_mismatch_is_rejected_at_compose_time`.")
  (peer "cadenza:m/api" (do (def (neg (: x Float64)) (+ x x)) (export neg)))
  (input
    (do
      (effect M (op neg (-> Int64 Int64)))
      (bind M "cadenza:m/api")
      (def (main (: x Int64)) (host (M) (M.neg x)))
      (export main)))
  (call main (: 5 Int64))
  (trap "type mismatch"))

(case
  "a peer missing a bound op is rejected naming the op"
  (doc
    "PROVIDER exports only `add`; the CONSUMER binds BOTH `add` and `sub` on the interface — the peer
           is missing `sub`. Composition is rejected naming the missing op (and what the peer DOES offer),
           not an opaque linker instance error. Relocated from the in-crate rcdzc
           `a_peer_missing_a_bound_op_is_rejected_naming_the_op`.")
  (peer "cadenza:m/api" (do (def (add (: x Int64) (: y Int64)) (+ x y)) (export add)))
  (input
    (do
      (effect M (op add (-> Int64 Int64 Int64)) (op sub (-> Int64 Int64 Int64)))
      (bind M "cadenza:m/api")
      (def (main (: x Int64)) (host (M) (+ (M.add x x) (M.sub x x))))
      (export main)))
  (call main (: 5 Int64))
  (trap "does not export op"))

; ── peer ops returning COLLECTIONS cross as handles and are read over the shared runtime ──────────
; (migrated from the in-crate rcdzc run_with_peers list-result tests). A heap-RESULT peer op is
; host-wrapped; the crossed collection is a runtime handle read back (len / at / field projection).
(case
  "pcl1 a peer op returning a list crosses as a handle and its length is read"
  (doc
    "PROVIDER `dup` returns a 3-element list; the consumer binds it and reads List.len of the crossed
           list over the shared runtime: dup(7)=[7,7,7] → 3.")
  (peer "cadenza:l/api" (do (def (dup (: x Int64)) #list(x x x)) (export dup)))
  (input
    (do
      (effect L (op dup (-> Int64 (List Int64))))
      (bind L "cadenza:l/api")
      (def (main (: x Int64)) (host (L) (List.len (L.dup x))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 3 Int64)))

(case
  "pcl2 a peer op returning a tuple with a variable-length list element crosses, both fields read"
  (doc
    "PROVIDER `mk` returns `(tuple (list x (+ x 1) (+ x 2)) (* x 10))` — a runtime-built list paired
           with a scalar; the consumer reads BOTH fields of the crossed tuple: mk(4)=([4,5,6],40),
           List.len(field0)=3 + field1=40 = 43 (the dynamic-depth element survived the crossing).")
  (peer
    "cadenza:p/api"
    (do (def (mk (: x Int64)) #tuple(#list(x (+ x 1) (+ x 2)) (* x 10))) (export mk)))
  (input
    (do
      (effect P (op mk (-> Int64 (Tuple (List Int64) Int64))))
      (bind P "cadenza:p/api")
      (def (main (: x Int64)) (host (P) (+ (List.len (. (P.mk x) 0)) (. (P.mk x) 1))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 43 Int64))
  (live-objects known-leak))

(case
  "pcl3 an element of a peer-returned list is read and used"
  (doc
    "PROVIDER `mklist(x)` = [x+1, x+2]; the consumer reads element 0 with List.at and unwraps the
           Option to the scalar the entrypoint returns: mklist(7)=[8,9], List.at 0 → Some 8 → 8.")
  (peer "cadenza:e/api" (do (def (mklist (: x Int64)) #list((+ x 1) (+ x 2))) (export mklist)))
  (input
    (do
      (effect L (op mklist (-> Int64 (List Int64))))
      (bind L "cadenza:e/api")
      (def (main (: x Int64)) (host (L) (match (List.at (L.mklist x) 0) ((Some v) v) (None 0))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 8 Int64)))

; ── peer ops returning MAP/SET results cross as handles read over the shared runtime ───────────────
; (the map/set analogue of the list-result cases above). A peer op building a Map or Set has its perform
; host-wrapped; the crossed collection is a runtime handle read back (len / lookup / membership).
(case
  "pcm1 a peer op returning a map crosses as a handle and its length is read"
  (doc
    "PROVIDER `mk(x)` builds a 2-entry (Map Int64 Int64); the consumer binds it and reads Map.len of the
           crossed map over the shared runtime: mk(7) = {1:7, 2:8} → 2 (the runtime-built map survived the
           cross-component boundary as a handle).")
  (peer
    "cadenza:mm/api"
    (do (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 x) 2 (+ x 1))) (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 Int64))))
      (bind M "cadenza:mm/api")
      (def (main (: x Int64)) (host (M) (Map.len (M.mk x))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 2 Int64)))

(case
  "pcm2 a value of a peer-returned map is read by key and used"
  (doc
    "PROVIDER `mk(x)` = {1:x+10, 2:x+20}; the consumer looks up key 2 in the crossed map and unwraps
           the Option to the scalar the entrypoint returns: mk(5) = {1:15, 2:25}, Map.lookup 2 → Some 25 → 25
           (a VALUE read off the crossed map, not just its size).")
  (peer
    "cadenza:mv/api"
    (do
      (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 (+ x 10)) 2 (+ x 20)))
      (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 Int64))))
      (bind M "cadenza:mv/api")
      (def (main (: x Int64)) (host (M) (match (Map.lookup (M.mk x) 2) ((Some v) v) (None 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 25 Int64)))

(case
  "pcs1 a peer op returning a set crosses as a handle and its length is read"
  (doc
    "PROVIDER `mk(x)` builds a 2-element (Set Int64) {x, x+1} (distinct); the consumer reads Set.len of
           the crossed set over the shared runtime: mk(7) = {7,8} → 2 (the runtime-built set crossed as a
           handle; the CHAMP survived the boundary).")
  (peer
    "cadenza:ss/api"
    (do (def (mk (: x Int64)) (Set.insert (Set.insert #set() x) (+ x 1))) (export mk)))
  (input
    (do
      (effect S (op mk (-> Int64 (Set Int64))))
      (bind S "cadenza:ss/api")
      (def (main (: x Int64)) (host (S) (Set.len (S.mk x))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 2 Int64)))

(case
  "pcs2 membership of a peer-returned set is queried for a present and an absent element"
  (doc
    "PROVIDER `mk(x)` = {x, x+1}; the consumer queries Set.contains on the crossed set for a PRESENT
           element (x) and an ABSENT one (99): mk(7) = {7,8}, contains 7 → yes (+10), contains 99 → no (+0)
           → 10. Pins that membership reads correctly over the crossed CHAMP on both faces.")
  (peer
    "cadenza:sc/api"
    (do (def (mk (: x Int64)) (Set.insert (Set.insert #set() x) (+ x 1))) (export mk)))
  (input
    (do
      (effect S (op mk (-> Int64 (Set Int64))))
      (bind S "cadenza:sc/api")
      (def
        (main (: x Int64))
        (host (S) (+ (if (Set.contains (S.mk x) x) 10 0) (if (Set.contains (S.mk x) 99) 1 0))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 10 Int64)))

(case
  "pcm3 a peer-returned map enumerates via Map.to-list and its values fold to a sum"
  (doc
    "PROVIDER `mk(x)` = {1:x+10, 2:x+20}; the consumer enumerates the crossed map with Map.to-list and
           folds the entry VALUES to a sum over the shared runtime: mk(5) = {1:15, 2:25}, sum of values = 40.
           Pins that the crossed map is fully ENUMERABLE (not only point-queried) and its entry values cross
           intact — the map analogue of the list-fold peer cases.")
  (peer
    "cadenza:mf/api"
    (do
      (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 (+ x 10)) 2 (+ x 20)))
      (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 Int64))))
      (bind M "cadenza:mf/api")
      (def
        (sumv (: es (List (Tuple Int64 Int64))) (: acc Int64))
        (match es (#list() acc) (#list(e (.. rest)) (sumv rest (+ acc (. e 1))))))
      (def (main (: x Int64)) (host (M) (sumv (Map.to-list (M.mk x)) 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 40 Int64))
  ; reclaims clean since the #8cb61381a runtime flag-day (shared immortal empty-vec singleton +
  ; collection-reclaim tightening reached the peer-returned-Map path); was known-leak 1 pre-flag-day.
  (live-objects 0))

; -- a LIST argument crosses INBOUND to a peer as a handle (migrated from rcdzc
; a_list_argument_crosses_inbound_to_a_peer_as_a_handle): the inbound twin of the list-RESULT crossing —
; a List has a distinct runtime rep (RRB vector) but crosses as a handle the peer dereferences.
(case
  "pla1 a list argument crosses inbound to a peer and is read by the peer op"
  (doc
    "PROVIDER `total : (List Int64) -> Int64` returns the list's length; the consumer builds
           `(list 10 20 30)` on the shared runtime and passes its handle into the peer op:
           total([10,20,30]) = 3.")
  (peer "cadenza:l/api" (do (def (total (: xs (List Int64))) (List.len xs)) (export total)))
  (input
    (do
      (effect L (op total (-> (List Int64) Int64)))
      (bind L "cadenza:l/api")
      (def (main) (host (L) (L.total #list(10 20 30))))
      (export main)))
  (call main)
  (output (: 3 Int64)))

; -- a peer op returning a (List <user-sum>) crosses to a peer executor (migrated from rcdzc
; the_agent_kernel_list_of_hostop_result_runs_through_a_peer_executor): the agent-kernel seam — interpret
; returns a branch-built (List HostOp) (a List of a String-payload user sum) that crosses peer->peer as a
; handle; the executor reads its length. Both sides declare the shared HostOp type.
(case
  "pak1 a peer op returning a (List of a user sum) crosses and its length is read"
  (doc
    "PROVIDER `interpret(kind, turn)` returns a branch-built (List HostOp): kind=1 -> [Append, Exec]
           (2 ops). The consumer binds it and reads List.len of the crossed list: interpret(1,0) -> len 2.")
  (peer
    "cadenza:agent/kernel"
    (do
      (type HostOp (Append String) (Exec String) (Http String) (Noop Int64))
      (def
        (interpret (: kind Int64) (: turn Int64))
        (if (= kind 1) #list((Append "a") (Exec "e")) #list((Noop 0))))
      (export interpret)))
  (input
    (do
      (type HostOp (Append String) (Exec String) (Http String) (Noop Int64))
      (effect K (op interpret (-> Int64 Int64 (List HostOp))))
      (bind K "cadenza:agent/kernel")
      (def (main (: kind Int64)) (host (K) (List.len (K.interpret kind 0))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "two effects bound to the same interface share one peer instance (both ops run)"
  (doc
    "ONE provider component exports `fa` (adds 10) and `fb` (adds 20) on `cadenza:x/y`; the consumer
           declares TWO effects A and B and binds BOTH to `cadenza:x/y` — a legal dedup on the EFFECT name,
           merging both ops onto a SINGLE `cadenza:x/y` instance import (not two colliding imports, which
           would be a silent-invalid component). `main = A.fa(1) + B.fb(2) = 11 + 22 = 33`, proving both
           effects route to the one shared provider instance and each op returns its own result. Relocated
           (RUN half) from the in-crate rcdzc `two_effects_bound_to_the_same_interface_share_one_peer_instance`
           — its white-box single-instance-import structural pin stays in rcdzc.")
  (peer
    "cadenza:x/y"
    (do (def (fa (: x Int64)) (+ x 10)) (def (fb (: x Int64)) (+ x 20)) (export fa) (export fb)))
  (input
    (do
      (effect A (op fa (-> Int64 Int64)))
      (effect B (op fb (-> Int64 Int64)))
      (bind A "cadenza:x/y")
      (bind B "cadenza:x/y")
      (def (main) (host (A B) (+ (A.fa 1) (B.fb 2))))
      (export main)))
  (call main)
  (output (: 33 Int64)))

; ── map/set peer-result DEPTH: empty-collection edge, nested map-of-list, canonical enumeration order ──
(case
  "pcm4 a peer op returning an EMPTY map crosses and reads length zero"
  (doc
    "The size-0 boundary: PROVIDER `mk` returns an empty (Map Int64 Int64); the consumer reads Map.len
           of the crossed empty map → 0. Pins that an empty collection crosses as a VALID handle (not a
           null/absent result) and its length reads zero.")
  (peer "cadenza:me/api" (do (def (mk (: x Int64)) (: (Map.empty) (Map Int64 Int64))) (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 Int64))))
      (bind M "cadenza:me/api")
      (def (main (: x Int64)) (host (M) (Map.len (M.mk x))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "pcs3 a peer op returning an EMPTY set crosses and reads length zero"
  (doc
    "The size-0 boundary for sets: PROVIDER `mk` returns an empty (Set Int64); Set.len of the crossed
           empty set → 0. The set analogue of the empty-map edge — an empty CHAMP crosses as a valid handle.")
  (peer "cadenza:se/api" (do (def (mk (: x Int64)) (: #set() (Set Int64))) (export mk)))
  (input
    (do
      (effect S (op mk (-> Int64 (Set Int64))))
      (bind S "cadenza:se/api")
      (def (main (: x Int64)) (host (S) (Set.len (S.mk x))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "pcm5 a peer op returning a map of lists crosses and a value list's length is read"
  (doc
    "Nested collection: PROVIDER `mk(x)` = {1: [x, x+1]} — a (Map Int64 (List Int64)); the consumer
           looks up key 1 in the crossed map and reads List.len of the value list: mk(5) = {1:[5,6]},
           Map.lookup 1 → Some [5,6], List.len → 2. Pins that a nested map-of-list crosses with the inner
           list intact (the value's dynamic depth survived the boundary).")
  (peer
    "cadenza:ml/api"
    (do (def (mk (: x Int64)) (Map.insert (Map.empty) 1 #list(x (+ x 1)))) (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 (List Int64)))))
      (bind M "cadenza:ml/api")
      (def
        (main (: x Int64))
        (host (M) (match (Map.lookup (M.mk x) 1) ((Some l) (List.len l)) (None 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "pcm6 a peer-returned map enumerates in canonical key order regardless of insert order"
  (doc
    "Ordering across the crossing: PROVIDER `mk(x)` inserts keys 3, 1, 2 (NON-sorted insert order); the
           consumer reads the FIRST entry's key via Map.to-list. Canonical KEY order is 1,2,3, so the head
           key is 1 regardless of insert order. Pins that deterministic canonical enumeration order survives
           the cross-component boundary (the crossed map is not left in insert order).")
  (peer
    "cadenza:mo/api"
    (do
      (def (mk (: x Int64)) (Map.insert (Map.insert (Map.insert (Map.empty) 3 x) 1 x) 2 x))
      (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 Int64))))
      (bind M "cadenza:mo/api")
      (def
        (main (: x Int64))
        (host (M) (match (Map.to-list (M.mk x)) (#list(e (.. rest)) (. e 0)) (#list() -1))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 1 Int64)))

(case
  "a consumer calls a scalar op across a source peer provider"
  (doc
    "The L3 COMPOSITION PROOF (happy path): a separately-compiled PROVIDER exports neg over Int64;
           the CONSUMER binds cadenza:math/api and performs M.neg under a host delegation. The harness
           compiles the provider to its own component and composes via --peer cadenza:math/api=<peer.wasm>
           (run_with_peers), then runs main 5 end-to-end → -5. Relocated from the in-crate rcdzc scalar
           peer round-trip.")
  (peer "cadenza:math/api" (do (def (neg (: x Int64)) (- 0 x)) (export neg)))
  (input
    (do
      (effect M (op neg (-> Int64 Int64)))
      (bind M "cadenza:math/api")
      (def (main (: x Int64)) (host (M) (M.neg x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: -5 Int64)))

(case
  "an effect bound to a peer routes a two-argument op to the boundary"
  (doc
    "The effects-unification RUN: a source consumer declares `(effect Math (op add …))`, DEFAULTS it to
           the peer cadenza:math/api via `(bind Math …)`, and DELEGATES `(host (Math) (Math.add x x))`. The
           escaping two-argument perform reaches the peer envelope exactly as an `(extern …)` op would — a
           peer dependency and a host effect are ONE concept. add(5,5)=10 crosses end-to-end: the passing
           two-arg scalar peer-op RUN witness (the positive complement to the arity/type-mismatch rejects,
           which trap before observing a value). Relocated from the in-crate rcdzc
           u2_an_effect_bound_to_a_peer_routes_to_the_boundary (its hand-built add peer rewritten as source).")
  (peer "cadenza:math/api" (do (def (add (: a Int64) (: b Int64)) (+ a b)) (export add)))
  (input
    (do
      (effect Math (op add (-> Int64 Int64 Int64)))
      (bind Math "cadenza:math/api")
      (def (main (: x Int64)) (host (Math) (Math.add x x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a non-kebab (camelCase) peer op name agrees across both sides and runs"
  (doc
    "PROVIDER exports camelCase `addTwo` on cadenza:math/api (its interface member kebab-normalizes to
           `add-two`); the CONSUMER binds the same interface and performs `Math.addTwo`. Both sides carry the
           kebab boundary name `add-two`, so they link; addTwo(5)=10 crosses end-to-end. Relocated (RUN half)
           from rcdzc a_non_kebab_peer_op_name_agrees_across_both_sides_and_runs — its white-box
           `add-two`-boundary-name pin stays in rcdzc.")
  (peer "cadenza:math/api" (do (def (addTwo (: x Int64)) (+ x x)) (export addTwo)))
  (input
    (do
      (effect Math (op addTwo (-> Int64 Int64)))
      (bind Math "cadenza:math/api")
      (def (main (: x Int64)) (host (Math) (Math.addTwo x)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a versioned interface name agrees across both sides and runs"
  (doc
    "PROVIDER publishes `dbl` on the VERSIONED cadenza:math/api@1.0.0; the CONSUMER binds the exact
           versioned string. The @version is part of the component-boundary extern name emitted verbatim on
           BOTH sides — a mismatch would not link. dbl(6)=12 crosses end-to-end. Relocated (RUN half) from
           rcdzc a_versioned_interface_name_agrees_across_both_sides_and_runs — its white-box versioned-name
           pin stays in rcdzc.")
  (peer "cadenza:math/api@1.0.0" (do (def (dbl (: x Int64)) (+ x x)) (export dbl)))
  (input
    (do
      (effect Math (op dbl (-> Int64 Int64)))
      (bind Math "cadenza:math/api@1.0.0")
      (def (main (: x Int64)) (host (Math) (Math.dbl x)))
      (export main)))
  (call main (: 6 Int64))
  (output (: 12 Int64)))

(case
  "a consumer bound to two scalar peer interfaces runs with no runtime"
  (doc
    "Two SEPARATELY-compiled scalar providers on DISTINCT interfaces (cadenza:math/api exporting neg,
           cadenza:succ/api exporting inc); the consumer binds BOTH and touches NO value-heap runtime, taking
           the peer-ONLY multi-interface envelope (g=2). main(4) = neg(4) + inc(4) = -4 + 5 = 1 — a value from
           EACH of two distinct peer interfaces in one body. Two (peer …) clauses compose in one case.
           Relocated from rcdzc u9c_two_scalar_peer_interfaces_no_runtime.")
  (peer "cadenza:math/api" (do (def (neg (: x Int64)) (- 0 x)) (export neg)))
  (peer "cadenza:succ/api" (do (def (inc (: x Int64)) (+ x 1)) (export inc)))
  (input
    (do
      (effect M (op neg (-> Int64 Int64)))
      (effect S (op inc (-> Int64 Int64)))
      (bind M "cadenza:math/api")
      (bind S "cadenza:succ/api")
      (def (main (: x Int64)) (host (M) (host (S) (+ (M.neg x) (S.inc x)))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 1 Int64)))

; ── map/set peer-result INTEROP: a crossed collection is first-class for further local runtime ops ──
(case
  "pcs4 a peer-returned set unions with a locally-built set over the shared runtime"
  (doc
    "INTEROP: the crossed set is not read-only — it participates in further runtime ops. PROVIDER
           `mk(x)` = {x, x+1}; the consumer unions the crossed set with a LOCALLY-built {x+1, x+2} and reads
           Set.len of the union: mk(7) = {7,8} ∪ {8,9} = {7,8,9} → 3 (the overlapping 8 dedups). Pins that a
           crossed CHAMP handle is a first-class set the local Set.union merges against.")
  (peer
    "cadenza:su/api"
    (do (def (mk (: x Int64)) (Set.insert (Set.insert #set() x) (+ x 1))) (export mk)))
  (input
    (do
      (effect S (op mk (-> Int64 (Set Int64))))
      (bind S "cadenza:su/api")
      (def
        (main (: x Int64))
        (host (S) (Set.len (Set.union (S.mk x) (Set.insert (Set.insert #set() (+ x 1)) (+ x 2))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 3 Int64)))

(case
  "pcm7 a peer-returned map accepts a further local insert over the shared runtime"
  (doc
    "INTEROP: the crossed map is a first-class handle further ops build on. PROVIDER `mk(x)` = {1:x,
           2:x+1}; the consumer inserts a THIRD entry {3:x+2} locally and reads Map.len: mk(7) = {1:7, 2:8},
           + {3:9} → 3. Pins that a crossed map handle accepts a persistent local Map.insert over the shared
           runtime (the map analogue of the set-union interop).")
  (peer
    "cadenza:mi/api"
    (do (def (mk (: x Int64)) (Map.insert (Map.insert (Map.empty) 1 x) 2 (+ x 1))) (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 (Map Int64 Int64))))
      (bind M "cadenza:mi/api")
      (def (main (: x Int64)) (host (M) (Map.len (Map.insert (M.mk x) 3 (+ x 2)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 3 Int64)))

; ── the ENTRYPOINT'S OWN result escapes as a runtime resource while reaching a peer op (the fused
; envelope: the consumer imports BOTH the peer interface AND the value-heap runtime, and main RETURNS
; the raw peer-produced compound rather than reading a scalar off it). Distinct emit paths per result
; shape — flat Tuple (assemble_extern_runtime_resource), non-recursive Option (emit_runtime_sum_resource),
; recursive List (emit_recursive_sum_resource). Migrated from the in-crate rcdzc PL35/PL36/PL37.
(case
  "ptr1 a peer compound (tuple) result escapes the entrypoint via the fused envelope"
  (doc
    "PROVIDER `mkpair(x)` = (x, x+1); the consumer RETURNS the raw tuple the peer produced, so the
           entrypoint's OWN result escapes as a runtime resource while a peer op is reached in the body
           (the component imports both the peer interface and the value-heap runtime). main(9) = (9,10),
           escaping to the host as its canonical value form.")
  (peer "cadenza:p/api" (do (def (mkpair (: x Int64)) #tuple(x (+ x 1))) (export mkpair)))
  (input
    (do
      (effect P (op mkpair (-> Int64 (Tuple Int64 Int64))))
      (bind P "cadenza:p/api")
      (def (main (: x Int64)) (host (P) (P.mkpair x)))
      (export main)))
  (call main (: 9 Int64))
  (output (: (tuple 9 10) (Tuple Int64 Int64)))
  (live-objects known-leak))

(case
  "por1 a peer Option result escapes the entrypoint via the fused envelope"
  (doc
    "The non-recursive SUM-resource escape path (emit_runtime_sum_resource, peer-aware): main RETURNS
           the raw Option a peer-derived list was indexed into (`List.at` IS the whole body), so the sum
           result escapes as a resource while the peer op `mklist` is reached. mklist(7)=[8,9],
           List.at 0 = Some 8; main RETURNS that Option → escapes as its value form.")
  (peer "cadenza:l/api" (do (def (mklist (: x Int64)) #list((+ x 1) (+ x 2))) (export mklist)))
  (input
    (do
      (effect L (op mklist (-> Int64 (List Int64))))
      (bind L "cadenza:l/api")
      (def (main (: x Int64)) (host (L) (List.at (L.mklist x) 0)))
      (export main)))
  (call main (: 7 Int64))
  (output (: (Some 8) (Option Int64)))
  (live-objects known-leak))

(case
  "plr1 a peer LIST result escapes the entrypoint via the fused envelope"
  (doc
    "The recursive-sum / value-encode walker escape path (emit_recursive_sum_resource), distinct from
           ptr1's flat Tuple and por1's non-recursive Option: main RETURNS the raw List the peer produced,
           so a variable-length collection escapes the entrypoint as a resource. mklist(7)=[8,9]; main
           RETURNS it → escapes as the List's value form.")
  (peer "cadenza:l/api" (do (def (mklist (: x Int64)) #list((+ x 1) (+ x 2))) (export mklist)))
  (input
    (do
      (effect L (op mklist (-> Int64 (List Int64))))
      (bind L "cadenza:l/api")
      (def (main (: x Int64)) (host (L) (L.mklist x)))
      (export main)))
  (call main (: 7 Int64))
  (output (: #list(8 9) (List Int64)))
  (live-objects known-leak))

; ── peer RESULT crossings read down to a scalar (no entrypoint escape): a BIGINT handle and a NESTED
; compound. Migrated from the in-crate rcdzc PL25/PL26.
(case
  "pbi1 a peer op returning a BigInt crosses as a handle and value-equality holds"
  (doc
    "PROVIDER `big(x)` = BigInt.of(x) widens the fixed-width int to a bignum handle (is_extern_heap_type,
           crosses as a u32 handle like a compound). The consumer compares the crossed BigInt to a locally
           built BigInt.of(x): big(42) == BigInt.of(42) → equal → 1. Confirms the bignum handle crosses and
           value-equality holds across the boundary.")
  (peer "cadenza:big/api" (do (def (big (: x Int64)) (BigInt.of x)) (export big)))
  (input
    (do
      (effect B (op big (-> Int64 BigInt)))
      (bind B "cadenza:big/api")
      (def (main (: x Int64)) (host (B) (if (= (B.big x) (BigInt.of x)) 1 0)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 1 Int64)))

(case
  "pnc1 a peer op returning a nested compound crosses as one handle and is projected"
  (doc
    "PROVIDER `nest(x)` = (tuple (list x x x) (+ x 1)) — a tuple whose first element is a List; the whole
           nesting crosses as a SINGLE handle. The consumer projects element 0 (the nested List), reads its
           List.len, and adds scalar element 1: nest(5)=([5,5,5],6), len 3 + 6 = 9. Pins that a compound
           CONTAINING a collection crosses intact and the inner list is reachable via projection.")
  (peer "cadenza:n/api" (do (def (nest (: x Int64)) #tuple(#list(x x x) (+ x 1))) (export nest)))
  (input
    (do
      (effect N (op nest (-> Int64 (Tuple (List Int64) Int64))))
      (bind N "cadenza:n/api")
      (def (main (: x Int64)) (host (N) (+ (List.len (. (N.nest x) 0)) (. (N.nest x) 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

; ── map/set/record ARG crossings (INBOUND): the consumer builds a compound and passes it TO a peer op ──
; (the arg-direction twin of the result cases above). The consumer builds a Map/Set/Record locally and hands
; it in as a peer op argument; the provider reads it over the shared runtime.
(case
  "pca1 a map argument crosses INBOUND to a peer and its length is read there"
  (doc
    "The ARG (inbound) direction: the CONSUMER builds a (Map Int64 Int64) and passes it INTO the peer
           op, which reads Map.len over the shared runtime. PROVIDER `msz` takes a map arg and returns its
           length; consumer builds {1:x, 2:x+1} and hands it in: msz({1:7, 2:8}) → 2 (the map crossed as a
           shared handle, read by the provider — the arg twin of the map-result cases).")
  (peer "cadenza:ma/api" (do (def (msz (: m (Map Int64 Int64))) (Map.len m)) (export msz)))
  (input
    (do
      (effect M (op msz (-> (Map Int64 Int64) Int64)))
      (bind M "cadenza:ma/api")
      (def
        (main (: x Int64))
        (host (M) (M.msz (Map.insert (Map.insert (Map.empty) 1 x) 2 (+ x 1)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "pca2 a set argument crosses INBOUND to a peer and its length is read there"
  (doc
    "The set arg twin: the consumer builds a (Set Int64) {x, x+1} and passes it into the peer op, which
           reads Set.len over the shared runtime: ssz({7, 8}) → 2 (the crossed CHAMP is read by the provider).")
  (peer "cadenza:sa/api" (do (def (ssz (: s (Set Int64))) (Set.len s)) (export ssz)))
  (input
    (do
      (effect S (op ssz (-> (Set Int64) Int64)))
      (bind S "cadenza:sa/api")
      (def (main (: x Int64)) (host (S) (S.ssz (Set.insert (Set.insert #set() x) (+ x 1)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 2 Int64))
  (live-objects known-leak))

(case
  "pca3 a record of a string and a scalar crosses INBOUND to a peer, both fields read there"
  (doc
    "A RECORD carrying a HEAP (String) field beside a scalar crosses inbound: the consumer builds
           `(record (msg \"hi\") (n 4))` and passes it to the peer op, which reads BOTH fields —
           String.byte-len of msg (2) plus n (4) = 6. Pins that a record with a heap field crosses as a
           handle and its fields are read by the provider (the record analogue of the compound-arg crossing).")
  (peer
    "cadenza:ra/api"
    (do
      (def (req (: r (Record (: msg String) (: n Int64)))) (+ (String.byte-len r.msg) r.n))
      (export req)))
  (input
    (do
      (effect R (op req (-> (Record (: msg String) (: n Int64)) Int64)))
      (bind R "cadenza:ra/api")
      (def (main) (host (R) (R.req #record((= msg "hi") (= n 4)))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "pca4 a tuple of a string and a scalar crosses INBOUND to a peer, both fields read there"
  (doc
    "The TUPLE analogue of the record-of-(String,scalar) inbound-arg case above: the consumer builds
           `(tuple \"hi\" 4)` — a heap (String) field beside a scalar — and passes it as ONE handle to the
           peer op, which projects BOTH positional fields: String.byte-len of field 0 (2) plus field 1 (4)
           = 6. Pins that a TUPLE with a heap field (not only a record) crosses as a handle and its fields
           are read by the provider.")
  (peer
    "cadenza:req/api"
    (do (def (req (: t (Tuple String Int64))) (+ (String.byte-len (. t 0)) (. t 1))) (export req)))
  (input
    (do
      (effect S (op req (-> (Tuple String Int64) Int64)))
      (bind S "cadenza:req/api")
      (def (main) (host (S) (S.req #tuple("hi" 4))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "a scalar tuple argument crosses INBOUND to a peer and both fields are summed there"
  (doc
    "The pure-scalar analogue of pca4: the consumer builds `(tuple x (+ x 1))` — two Int64 fields, no
           heap — and passes it as ONE handle to the peer op `sum`, which projects both fields and adds them.
           main(9) = S.sum((9,10)) = 9 + 10 = 19. Pins that a scalar-only compound crosses as a handle inbound
           and the provider reads both fields. Relocated from rcdzc u16_a_compound_argument_crosses_to_a_peer.
           Like the Map/Set inbound-arg twins pca1/pca2, the scalar-only compound crosses inbound as ONE
           handle that is not yet reclaimed after the peer call → live-objects 1 (known leak, same class as
           pca1/pca2; the heap-field tuple pca4 reclaims clean). Flips to 0 when peer-inbound-arg compound
           reclaim lands (v-rust-backend arg-side emit / v-memory-safety).")
  (peer
    "cadenza:adder/api"
    (do (def (sum (: t (Tuple Int64 Int64))) (+ (. t 0) (. t 1))) (export sum)))
  (input
    (do
      (effect S (op sum (-> (Tuple Int64 Int64) Int64)))
      (bind S "cadenza:adder/api")
      (def (main (: x Int64)) (host (S) (S.sum #tuple(x (+ x 1)))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 19 Int64))
  (live-objects known-leak))

(case
  "two compound arguments each cross as their own handle in one peer call"
  (doc
    "u16 pins a SINGLE compound arg; this pins that MULTIPLE compound args each cross as their OWN handle
           in ONE call (the multi-handle inbound case). Peer `add4` takes TWO `(Tuple Int64 Int64)` and sums all
           four fields; the consumer passes two freshly-built tuples. main(5) = add4((5,5),(5,5)) = 20 — both
           tuple handles crossed inbound and were read. Relocated from rcdzc
           two_compound_arguments_cross_to_a_peer_in_one_op.
           Same known-leak class as the single scalar-tuple inbound-arg case and pca1/pca2: each scalar-only
           compound crosses inbound as ONE not-yet-reclaimed handle → TWO args leak 2. Flips to 0 when
           peer-inbound-arg compound reclaim lands (v-rust-backend arg-side emit / v-memory-safety).")
  (peer
    "cadenza:s/api"
    (do
      (def
        (add4 (: a (Tuple Int64 Int64)) (: b (Tuple Int64 Int64)))
        (+ (+ (. a 0) (. a 1)) (+ (. b 0) (. b 1))))
      (export add4)))
  (input
    (do
      (effect S (op add4 (-> (Tuple Int64 Int64) (Tuple Int64 Int64) Int64)))
      (bind S "cadenza:s/api")
      (def (main (: x Int64)) (host (S) (S.add4 #tuple(x x) #tuple(x x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64))
  (live-objects known-leak))

(case
  "a String argument handle passed to a peer stays live for a later local read (borrow not move)"
  (doc
    "A refcount/borrow-correctness pin: passing a String handle to a peer op must BORROW it, not consume
           it. The consumer let-binds a runtime rope `s = String.concat \"ab\" \"cde\"`, passes it to peer `blen`,
           THEN reads the SAME rope again locally: main = S.blen(s) + String.byte-len(s) = 5 + 5 = 10. If the
           peer-arg emit treated the handle as OWNED (moved/freed by the call), the second local read would be a
           use-after-free — a trap or a garbage length, NOT 10 (and it would still VALIDATE: byte-valid is not
           refcount-correct). Running it e2e proves the handle stays live across the boundary. Relocated from
           rcdzc a_string_argument_handle_survives_the_peer_call_and_is_reused_locally.")
  (peer "cadenza:reuse/api" (do (def (blen (: s String)) (String.byte-len s)) (export blen)))
  (input
    (do
      (effect S (op blen (-> String Int64)))
      (bind S "cadenza:reuse/api")
      (def
        (main)
        (let ((s (String.concat "ab" "cde"))) (host (S) (+ (S.blen s) (String.byte-len s)))))
      (export main)))
  (call main)
  (output (: 10 Int64)))

(case
  "a string argument crosses to a peer and the doubled result byte-len is read"
  (doc
    "The converse shape (string ARG + string RESULT both cross as rope handles over the shared runtime):
           PROVIDER `converse(prompt)` = String.concat prompt prompt BUILDS a new rope from the crossed arg;
           the CONSUMER passes the literal \"hello\" host-wrapped and reads the doubled completion's byte-len.
           main = String.byte-len(host M (M.converse \"hello\")) = len(\"hellohello\") = 10 — proving the String
           ARG crossed INTO the peer as a rope and was CONSUMED (a broken arg emit would trap or mis-length),
           and the String RESULT crossed back. Relocated (RUN half) from rcdzc u8_a_string_argument_crosses_to_a_peer
           — its white-box value-heap-runtime-import pin stays in rcdzc.")
  (peer
    "cadenza:model/api"
    (do (def (converse (: prompt String)) (String.concat prompt prompt)) (export converse)))
  (input
    (do
      (effect M (op converse (-> String String)))
      (bind M "cadenza:model/api")
      (def (main) (String.byte-len (host (M) (M.converse "hello"))))
      (export main)))
  (call main)
  (output (: 10 Int64)))

(case
  "a symbol argument crosses to and from a peer as a runtime handle like a string"
  (doc
    "A Symbol crosses the peer boundary as a runtime handle exactly like a String — a Symbol is a
           String byte-leaf at run time (`is_extern_heap_type` includes `Ty::Symbol`), so a peer op taking
           AND returning a Symbol crosses its handle both ways with no marshaling. PROVIDER `echo` returns
           the crossed Symbol unchanged; the CONSUMER passes `(Symbol.of \"hi\")` INTO the peer op (arg
           crosses as a handle), reads the returned Symbol handle BACK, and compares it to the original —
           equal → 1. Before, a peer op declaring a Symbol DECLINED at the boundary while the identical
           String op crossed; this pins the transport parity.")
  (peer "cadenza:sym/api" (do (def (echo (: s Symbol)) s) (export echo)))
  (input
    (do
      (effect Sym (op echo (-> Symbol Symbol)))
      (bind Sym "cadenza:sym/api")
      (def (main) (host (Sym) (if (= (Sym.echo (Symbol.of "hi")) (Symbol.of "hi")) 1 0)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a scalar quantity argument crosses to and from a peer as its inner scalar"
  (doc
    "A scalar-inner Quantity crosses the peer boundary as its INNER scalar — the unit is a compile-time
           value ERASED before codegen (`(Qty Int64 u)` has the same runtime rep as Int64), so it crosses BY
           VALUE (not a handle), the guest's static type carrying the unit. PROVIDER `dbl` doubles the
           magnitude (unit preserved); the CONSUMER passes `3 meter` INTO the peer op (crosses as the inner
           Int64 3) and reads the returned Qty's magnitude with `Qty.value` → 6. Both arg and result cross as
           the inner scalar. (The complementary HEAP-inner Qty — e.g. `(Qty Rational …)` — still crosses as a
           handle; this pins the scalar half over a peer.)")
  (peer
    "cadenza:q/api"
    (do
      (def
        (dbl (: q (Qty Int64 (Unit.base #"meter"))))
        (Qty.of (* 2 (Qty.value q)) (Unit.base #"meter")))
      (export dbl)))
  (input
    (do
      (effect Q (op dbl (-> (Qty Int64 (Unit.base #"meter")) (Qty Int64 (Unit.base #"meter")))))
      (bind Q "cadenza:q/api")
      (def (main) (host (Q) (Qty.value (Q.dbl (Qty.of 3 (Unit.base #"meter"))))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "a heap-inner (Rational) quantity crosses to and from a peer as a shared runtime handle"
  (doc
    "The COMPLEMENT of the scalar-inner Quantity case above: a HEAP-inner Quantity — `(Qty Rational m)`,
           whose magnitude is an exact rational (a value-heap bignum pair, NOT a scalar) — crosses the peer
           boundary as its opaque runtime HANDLE, not by value. This is the OTHER arm of the Qty ABI split:
           the by-value arm needs an inner scalar boundary form, which a Rational has none of, so it falls
           through to the heap-handle path (the scalar case pins the by-value arm; BOTH must hold for the
           Quantity ABI to be sound over a peer). PROVIDER `echo` returns the crossed `(Qty Rational m)`
           unchanged; the CONSUMER builds `1/2 meter`, passes its handle INTO the peer op (arg crosses as a
           handle), reads the returned Qty's magnitude back with `Qty.value` (a Rational), and compares it
           EXACTLY to a fresh `1/2` → 1. The exact-rational magnitude crossed both ways as a shared-runtime
           handle with no marshaling; main returns a scalar so the compound crosses only at the peer boundary,
           not as the escaping result. Relocated from rcdzc a_heap_inner_quantity_crosses_to_and_from_a_peer_as_a_runtime_handle.")
  (peer
    "cadenza:qr/api"
    (do (def (echo (: q (Qty Rational (Unit.base #"meter")))) q) (export echo)))
  (input
    (do
      (effect
        Qr
        (op echo (-> (Qty Rational (Unit.base #"meter")) (Qty Rational (Unit.base #"meter")))))
      (bind Qr "cadenza:qr/api")
      (def
        (main)
        (host
          (Qr)
          (if
            (=
              (Qty.value (Qr.echo (Qty.of (Rational.of 1 2) (Unit.base #"meter"))))
              (Rational.of 1 2))
            1
            0)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a runtime-produced String argument crosses to a peer as its handle"
  (doc
    "A String argument that is NOT a literal but a RUNTIME `String.concat` — a value-heap rope handle
           built in-body — crosses to the peer as its handle: `blen(String.concat \"ab\" \"cde\")` reads the
           crossed rope's byte-len = 5. The runtime-built companion of the literal string-arg crossing; pins
           that a DYNAMICALLY-produced rope handle crosses the boundary, not only a compile-time literal.")
  (peer "cadenza:strs2/api" (do (def (blen (: s String)) (String.byte-len s)) (export blen)))
  (input
    (do
      (effect S (op blen (-> String Int64)))
      (bind S "cadenza:strs2/api")
      (def (main) (host (S) (S.blen (String.concat "ab" "cde"))))
      (export main)))
  (call main)
  (output (: 5 Int64)))

(case
  "a mixed String and scalar argument cross to a peer in one op, each in its own ABI lane"
  (doc
    "A single peer op taking `(-> String Int64 …)` crosses a String AND a scalar in ONE call: the
           String lowers to a rope HANDLE (an i32 handle), the Int64 lowers DIRECTLY (an i64), pushed in
           declaration order, and the peer reads BOTH — `blen-plus(\"hello\", 7)` = byte-len(\"hello\") + 7 =
           5 + 7 = 12. Pins that the two DISTINCT arg-emit paths (heap-handle vs direct scalar) INTERLEAVE
           correctly in one call (a mis-order would misalign the peer's params) — the exact call shape a
           `(-> String Int64 …)` model op takes.")
  (peer
    "cadenza:mix/api"
    (do (def (blen-plus (: s String) (: n Int64)) (+ (String.byte-len s) n)) (export blen-plus)))
  (input
    (do
      (effect S (op blen-plus (-> String Int64 Int64)))
      (bind S "cadenza:mix/api")
      (def (main) (host (S) (S.blen-plus "hello" 7)))
      (export main)))
  (call main)
  (output (: 12 Int64)))

; ── breaker batch 542: the two peer-matrix cells the landed coverage misses — the heap-ARG
; direction (consumer→provider lowering; the landed pcl/pcm cases are all RESULT-crossing) and
; the cross-boundary CENSUS (peer-returned heap consumed ×50 must reclaim on the consumer side;
; live-objects clauses verified working on peer cases).
(case
  "pah1 a heap LIST argument crosses INTO the peer and is consumed by the provider (arg-direction lowering)"
  (peer
    "cadenza:a/api"
    (do
      (def (sum (: xs (List Int64))) (match xs (#list() 0) (#list(h (.. t)) (+ h (sum t)))))
      (export sum)))
  (input
    (do
      (effect A (op sum (-> (List Int64) Int64)))
      (bind A "cadenza:a/api")
      (def (bld (: i Int64)) (if (= i 0) #list() (List.push (bld (- i 1)) i)))
      (def (main (: n Int64)) (host (A) (A.sum (bld n))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 10 Int64))
  ; reclaims clean since the #8cb61381a runtime flag-day (collection-reclaim tightening reached the
  ; heap-List inbound-arg path); was known-leak 1 pre-flag-day.
  (live-objects 0))

(case
  "pcc1 fifty peer-returned lists consumed on the consumer side reclaim to zero (cross-boundary census)"
  (peer "cadenza:a/api" (do (def (dup (: x Int64)) #list(x x x)) (export dup)))
  (input
    (do
      (effect A (op dup (-> Int64 (List Int64))))
      (bind A "cadenza:a/api")
      (def (frames (: k Int64)) (if (= k 0) 0 (host (A) (+ (List.len (A.dup k)) (frames (- k 1))))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 150 Int64))
  (live-objects 0))

(case
  "a consumer bound to two distinct peer interfaces combines their results"
  (doc
    "Two SEPARATELY-compiled providers on DISTINCT interfaces: cadenza:math/api exports a scalar neg,
           cadenza:pairs/api exports a COMPOUND-returning pair. The consumer binds BOTH and computes a value
           from EACH in one body: pairs.pair(9) = (9,9) crosses as a runtime handle, project element 0 → 9,
           then math.neg(9) → -9 — over the multi-interface extern+runtime envelope. Relocated (RUN half) from
           rcdzc u9_a_consumer_binds_two_distinct_peer_interfaces — its white-box both-interfaces-imported +
           both-providers-publish pins stay in rcdzc.")
  (peer "cadenza:math/api" (do (def (neg (: x Int64)) (- 0 x)) (export neg)))
  (peer "cadenza:pairs/api" (do (def (pair (: x Int64)) #tuple(x x)) (export pair)))
  (input
    (do
      (effect M (op neg (-> Int64 Int64)))
      (effect P (op pair (-> Int64 (Tuple Int64 Int64))))
      (bind M "cadenza:math/api")
      (bind P "cadenza:pairs/api")
      (def (main (: x Int64)) (host (M) (host (P) (M.neg (. (P.pair x) 0)))))
      (export main)))
  (call main (: 9 Int64))
  (output (: -9 Int64)))

; ── the WITH-METHODS fused envelope: a String / Bytes RESULT (a byte-leaf heap rep carrying
; len/is-empty/to-bytes) escapes the entrypoint while reaching a peer op. This is the live model-call
; boundary (Bedrock-as-peer): a prompt crosses IN, a completion RETURNS OUT and escapes. Uses the
; methods-carrying fused assembler (emit_runtime_bytes_resource), distinct from the plain compound
; escape (ptr1/plr1). Migrated from the in-crate rcdzc PL39/PL40/PL41/PL42.
(case
  "pse1 the full (-> String String) converse crosses a peer and its String result escapes main"
  (doc
    "THE FULL MODEL-CALL SHAPE: peer op `converse : String -> String` gets a prompt IN and the
           entrypoint RETURNS the completion (escapes as a resource-WITH-METHODS via the fused envelope,
           the String ARG emit composed with the String-RESULT escape). converse(\"hi\") = \"hihi\";
           main RETURNS it → escapes as its String value form.")
  (peer
    "cadenza:model/api"
    (do (def (converse (: prompt String)) (String.concat prompt prompt)) (export converse)))
  (input
    (do
      (effect M (op converse (-> String String)))
      (bind M "cadenza:model/api")
      (def (main) (host (M) (M.converse "hi")))
      (export main)))
  (call main)
  (output (: "hihi" String))
  (live-objects known-leak))

(case
  "psc1 chained peer String ops flow a result into an arg then the second result escapes"
  (doc
    "The agentic-pipeline shape `tag(converse(prompt))`: a String is BOTH a peer result (handle out
           of converse) AND a peer argument (handle into tag) in one body, then the second op's result
           escapes. Two ops on ONE interface. converse(\"hi\")=\"hihi\"; tag(\"hihi\")=\"T:hihi\"; main
           RETURNS it → escapes.")
  (peer
    "cadenza:model/api"
    (do
      (def (converse (: p String)) (String.concat p p))
      (def (tag (: s String)) (String.concat "T:" s))
      (export converse)
      (export tag)))
  (input
    (do
      (effect M (op converse (-> String String)) (op tag (-> String String)))
      (bind M "cadenza:model/api")
      (def (main) (host (M) (M.tag (M.converse "hi"))))
      (export main)))
  (call main)
  (output (: "T:hihi" String))
  (live-objects known-leak))

(case
  "pbk1 a request-struct {prompt,max-tokens} crosses to a peer and its String completion escapes"
  (doc
    "THE REALISTIC BEDROCK SHAPE: peer op `converse : (Tuple String Int64) -> String` — a request
           record {prompt, max-tokens} crosses IN as ONE handle, the peer projects the prompt field (. r 0)
           and doubles it, and the entrypoint RETURNS the String completion (escapes). The production
           model-call signature, not the toy String->String. converse((\"hi\",64)) reads \"hi\" → \"hihi\"
           → escapes.")
  (peer
    "cadenza:model/api"
    (do
      (def (converse (: r (Tuple String Int64))) (String.concat (. r 0) (. r 0)))
      (export converse)))
  (input
    (do
      (effect M (op converse (-> (Tuple String Int64) String)))
      (bind M "cadenza:model/api")
      (def (main) (host (M) (M.converse #tuple("hi" 64))))
      (export main)))
  (call main)
  (output (: "hihi" String))
  (live-objects known-leak))

(case
  "pby1 a peer BYTES result escapes the entrypoint via the with-methods fused envelope"
  (doc
    "The binary-result sibling of pse1: a Bytes result crosses via the SAME with-methods fused
           envelope as a String (both are the byte-leaf heap rep) but decodes to the Bytes value form
           `(: b\"…\" Bytes)`, NOT the String form. The shape a model op returning a binary blob (an
           embedding, an image) takes. mk = String.to-bytes \"hi\"; main RETURNS it → escapes as b\"hi\".")
  (peer "cadenza:blob/api" (do (def (mk (: _x Int64)) (String.to-bytes "hi")) (export mk)))
  (input
    (do
      (effect M (op mk (-> Int64 Bytes)))
      (bind M "cadenza:blob/api")
      (def (main (: x Int64)) (host (M) (M.mk x)))
      (export main)))
  (call main (: 1 Int64))
  (output (: b"hi" Bytes)))

(case
  "a middle peer is both a consumer and a provider (A to B to C chain)"
  (doc
    "B is a MIDDLE component: it BINDS cadenza:pairs/api (consuming A's compound pair) AND publishes its
           own cadenza:mid/api (providing to C). Two (peer …) clauses ship A (pairs/api) and B (mid/api); the
           top consumer C binds only cadenza:mid/api. A value flows A→B→C: C.main(9) → B.mid(9) →
           (A.pair(9)=(9,9)).0 + 1 = 10 — B both consumes A and provides to C over the fused envelope, and
           the harness wires A into B's linker (transitive peer dependency). Relocated from rcdzc
           u11_a_middle_component_is_both_consumer_and_provider — its white-box mid-publishes+imports pin stays.")
  (peer "cadenza:pairs/api" (do (def (pair (: x Int64)) #tuple(x x)) (export pair)))
  (peer
    "cadenza:mid/api"
    (do
      (effect P (op pair (-> Int64 (Tuple Int64 Int64))))
      (bind P "cadenza:pairs/api")
      (def (mid (: x Int64)) (host (P) (+ (. (P.pair x) 0) 1)))
      (export mid)))
  (input
    (do
      (effect M (op mid (-> Int64 Int64)))
      (bind M "cadenza:mid/api")
      (def (main (: x Int64)) (host (M) (M.mid x)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 10 Int64)))

; ── breaker batch 554: the peer-leak taxonomy contrast cell. #4610 known-leaked the compound
; results that ESCAPE to the entrypoint (por1/plr1/pcl 1-2 cells — the boundary-envelope family);
; this pins the other side: a let-bound result MULTI-consumed in-frame (len + element read),
; fifty frames, reclaims to ZERO — the leak keys to ESCAPE position, not consumption count.
(case
  "pkc1 fifty let-bound multi-consumed peer results reclaim to zero (the leak keys to escape, not consumption)"
  (peer "cadenza:a/api" (do (def (dup (: x Int64)) #list(x x x)) (export dup)))
  (input
    (do
      (effect A (op dup (-> Int64 (List Int64))))
      (bind A "cadenza:a/api")
      (def
        (frames (: k Int64))
        (if
          (= k 0)
          0
          (host
            (A)
            (let
              ((r (A.dup k)))
              (+
                (+ (List.len r) (match (List.at r 0) ((Option.Some v) v) ((Option.None) -1)))
                (frames (- k 1)))))))
      (def (main (: n Int64)) (frames n))
      (export main)))
  (call main (: 50 Int64))
  (output (: 1425 Int64))
  (live-objects 0))

(case
  "a peer op returning a tuple result is projected to a scalar over the shared runtime"
  (doc
    "Both sides from source: a provider `(def (pair (: x Int64)) (tuple x x))` published as
           cadenza:pairs/api returns a RUNTIME tuple; the consumer binds it, performs `P.pair`, and projects
           element 0. The tuple crosses as a handle over the shared value-heap runtime and `(. (P.pair x) 0)`
           reads it: pair(9) = (9,9), element 0 = 9. The single-interface compound-result-projection pin.")
  (peer "cadenza:pairs/api" (do (def (pair (: x Int64)) #tuple(x x)) (export pair)))
  (input
    (do
      (effect P (op pair (-> Int64 (Tuple Int64 Int64))))
      (bind P "cadenza:pairs/api")
      (def (main (: x Int64)) (host (P) (. (P.pair x) 0)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 9 Int64)))

(case
  "a peer string result is consumed in-program via equality and byte-len"
  (doc
    "PROVIDER `reply(seed)` returns the String \"ok\" @cadenza:model/api; the CONSUMER performs M.reply,
           EQUALITY-dispatches on the crossed completion (`= \"ok\"`), and on match returns its byte-len — the
           coarse tool-call dispatch the native agent loop uses. main(1): reply → \"ok\", `(= … \"ok\")` holds
           → String.byte-len = 2. Pins that a peer String RESULT is CONSUMED in-program (`=` over a crossed
           runtime rope + byte-len), distinct from a String result that merely escapes (pse1). Relocated from
           the in-crate rcdzc u7_a_string_result_crosses_a_peer_and_is_consumed_in_program.")
  (peer "cadenza:model/api" (do (def (reply (: seed Int64)) "ok") (export reply)))
  (input
    (do
      (effect M (op reply (-> Int64 String)))
      (bind M "cadenza:model/api")
      (def
        (main (: seed Int64))
        (if (= (host (M) (M.reply seed)) "ok") (String.byte-len (host (M) (M.reply seed))) 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "a fallible Result peer op result crosses and its Ok arm matches"
  (doc
    "PROVIDER `safediv(x) = if (= x 0) (Err 1) (Ok (/ 100 x))` returns a (Result Int64 Int64)
           @cadenza:d/api; the CONSUMER matches the crossed Result — the Ok payload passes through, the Err
           payload is negated so the arms are distinguishable in the scalar result. safediv(5) = Ok(100/5) =
           Ok(20) → the Ok arm yields 20 (a peer op's SUM/Result result crosses as a handle and is matched).")
  (peer
    "cadenza:d/api"
    (do (def (safediv (: x Int64)) (if (= x 0) (Err 1) (Ok (/ 100 x)))) (export safediv)))
  (input
    (do
      (effect D (op safediv (-> Int64 (Result Int64 Int64))))
      (bind D "cadenza:d/api")
      (def (main (: x Int64)) (host (D) (match (D.safediv x) ((Ok q) q) ((Err e) (- 0 e)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64)))

(case
  "a fallible Result peer op result crosses and its Err arm matches"
  (doc
    "The Err path of the fallible peer op: safediv(0) hits the zero guard → Err(1); the consumer's Err
           arm negates the payload → -1 (distinct from any Ok result), proving BOTH variants of the crossed
           Result are matched over the shared runtime.")
  (peer
    "cadenza:d/api"
    (do (def (safediv (: x Int64)) (if (= x 0) (Err 1) (Ok (/ 100 x)))) (export safediv)))
  (input
    (do
      (effect D (op safediv (-> Int64 (Result Int64 Int64))))
      (bind D "cadenza:d/api")
      (def (main (: x Int64)) (host (D) (match (D.safediv x) ((Ok q) q) ((Err e) (- 0 e)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: -1 Int64)))

(case
  "a signature mismatch in a peer-into-peer (transitive chain) binding is rejected at compose time"
  (doc
    "An A→B→C chain where the MIDDLE peer B (cadenza:mid/api) binds A's `base` as 2-arg, but A
           (cadenza:base/api) exports `base` as 1-arg. When the harness wires A into B (the transitive
           dependency), the compose-time signature check catches B's mismatched binding of `base` — a
           signature mismatch naming the base op — rather than an opaque runtime trap. (The valid chain is
           covered by the A→B→C middle-peer case; this is its reject face across the transitive edge.)")
  (peer "cadenza:base/api" (do (def (base (: x Int64)) (* x 2)) (export base)))
  (peer
    "cadenza:mid/api"
    (do
      (effect Base (op base (-> Int64 Int64 Int64)))
      (bind Base "cadenza:base/api")
      (def (mid (: x Int64)) (host (Base) (Base.base x x)))
      (export mid)))
  (input
    (do
      (effect Mid (op mid (-> Int64 Int64)))
      (bind Mid "cadenza:mid/api")
      (def (main (: x Int64)) (host (Mid) (Mid.mid x)))
      (export main)))
  (call main (: 5 Int64))
  (trap "signature mismatch"))

(case
  "a signature mismatch in ONE of several bound peers is rejected at compose time"
  (doc
    "A consumer binds TWO peers: M.add (cadenza:math/api, correct) + P.neg (cadenza:p/api). P
           mismatches — it exports neg over Float64 while the consumer binds neg over Int64 — while M is
           fine. The compose-time signature check ITERATES all bound peers and rejects on the offending one
           (a type mismatch on P's neg), not an opaque runtime trap. (The both-peers-match positive is
           covered by the two-scalar-peer-interfaces case; this is the one-of-several-mismatches reject.)")
  (peer "cadenza:math/api" (do (def (add (: x Int64) (: y Int64)) (+ x y)) (export add)))
  (peer "cadenza:p/api" (do (def (neg (: x Float64)) (+ x x)) (export neg)))
  (input
    (do
      (effect M (op add (-> Int64 Int64 Int64)))
      (effect P (op neg (-> Int64 Int64)))
      (bind M "cadenza:math/api")
      (bind P "cadenza:p/api")
      (def (main (: x Int64)) (host (M P) (+ (M.add x x) (P.neg x))))
      (export main)))
  (call main (: 5 Int64))
  (trap "type mismatch"))

(case
  "a peer result handle passes straight through to another peer's arg without local inspection"
  (doc
    "A (cadenza:pairs/api) mints a tuple `pair(x) = (tuple x (+ x 1))`; that tuple handle flows STRAIGHT
           into B (cadenza:adder/api) `sum(t) = (. t 0) + (. t 1)` as its argument — the consumer never
           inspects it locally: main(x) = S.sum(P.pair x). main(9) = sum((9,10)) = 19. A peer RESULT handle
           crosses directly into ANOTHER peer's ARG over the shared runtime (peer→peer passthrough).")
  (peer "cadenza:pairs/api" (do (def (pair (: x Int64)) #tuple(x (+ x 1))) (export pair)))
  (peer
    "cadenza:adder/api"
    (do (def (sum (: t (Tuple Int64 Int64))) (+ (. t 0) (. t 1))) (export sum)))
  (input
    (do
      (effect P (op pair (-> Int64 (Tuple Int64 Int64))))
      (effect S (op sum (-> (Tuple Int64 Int64) Int64)))
      (bind P "cadenza:pairs/api")
      (bind S "cadenza:adder/api")
      (def (main (: x Int64)) (host (P) (host (S) (S.sum (P.pair x)))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 19 Int64))
  ; the crossed scalar-tuple handle is not yet reclaimed after the peer boundary (same class as pca1/pca2 and
  ; the scalar-tuple inbound-arg case); flips to 0 when peer-boundary compound reclaim lands (v-rust-backend / v-memory-safety).
  (live-objects known-leak))

(case
  "a let-bound peer compound is read twice (both live) then reclaimed"
  (doc
    "A peer op `pair(x) = (tuple x x)` @cadenza:pairs/api returns a runtime tuple; the consumer LET-binds
           the crossed tuple and reads it TWICE — `(+ (. t 0) (. t 1))` — both reads must see a LIVE tuple (no
           premature drop between them), then the dead binding reclaims at scope end. pair(9) = (9,9) →
           9 + 9 = 18.")
  (peer "cadenza:pairs/api" (do (def (pair (: x Int64)) #tuple(x x)) (export pair)))
  (input
    (do
      (effect P (op pair (-> Int64 (Tuple Int64 Int64))))
      (bind P "cadenza:pairs/api")
      (def (main (: x Int64)) (host (P) (let ((t (P.pair x))) (+ (. t 0) (. t 1)))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 18 Int64)))

(case
  "a compound built from two distinct peers escapes the entrypoint as a resource"
  (doc
    "main RETURNS a tuple built from ops on TWO distinct peer interfaces — A.pa(x)=2x @cadenza:a/api and
           B.pb(x)=3x @cadenza:b/api — so the fresh `(A.pa x, B.pb x)` tuple ESCAPES the entrypoint as a
           resource reaching both interfaces. main(5) = (tuple 10 15). Exercises the multi-interface (g>1)
           fused resource-escape assembler.")
  (peer "cadenza:a/api" (do (def (pa (: x Int64)) (+ x x)) (export pa)))
  (peer "cadenza:b/api" (do (def (pb (: x Int64)) (+ (+ x x) x)) (export pb)))
  (input
    (do
      (effect A (op pa (-> Int64 Int64)))
      (bind A "cadenza:a/api")
      (effect B (op pb (-> Int64 Int64)))
      (bind B "cadenza:b/api")
      (def (main (: x Int64)) (host (A B) #tuple((A.pa x) (B.pb x))))
      (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 10 15) (Tuple Int64 Int64)))
  ; the fused escaping compound built across two peers leaves one handle unreclaimed at the boundary;
  ; flips to 0 when peer-boundary compound reclaim lands (v-rust-backend / v-memory-safety).
  (live-objects known-leak))

(case
  "a string result chained through two peers escapes the entrypoint via the methods envelope"
  (doc
    "Two peers with a chained String result: A (cadenza:a/api) `ga(_x) = String.concat \"h\" \"i\"` = \"hi\";
           B (cadenza:b/api) `gb(s) = String.concat s s` (doubles). main chains A.ga into B.gb and RETURNS the
           result: gb(ga(x)) = gb(\"hi\") = \"hihi\" → escapes the entrypoint as a String resource. Exercises
           the WITH-METHODS multi-interface (g>1) fused resource-escape assembler (a String result lifts
           make+encode+len methods, with the component-type indices shifted by g for two interfaces).")
  (peer "cadenza:a/api" (do (def (ga (: _x Int64)) (String.concat "h" "i")) (export ga)))
  (peer "cadenza:b/api" (do (def (gb (: s String)) (String.concat s s)) (export gb)))
  (input
    (do
      (effect A (op ga (-> Int64 String)))
      (bind A "cadenza:a/api")
      (effect B (op gb (-> String String)))
      (bind B "cadenza:b/api")
      (def (main (: x Int64)) (host (A B) (B.gb (A.ga x))))
      (export main)))
  (call main (: 0 Int64))
  (output (: "hihi" String))
  ; the String result chained through two peers escapes with one boundary handle unreclaimed;
  ; flips to 0 when peer-boundary result reclaim lands (v-rust-backend / v-memory-safety).
  (live-objects known-leak))

(case
  "a peer compound is projected and rebuilt into a fresh escaping compound (mixed ownership reclaims)"
  (doc
    "MIXED-OWNERSHIP reclaim: a peer op `mk(x) = (tuple x x)` @cadenza:p/api mints a runtime tuple; the
           consumer LET-binds it, reads BOTH fields (borrowing projections), and rebuilds a FRESH tuple
           `(t.0, t.1 + 100)` that ESCAPES as a resource — while the BORROWED peer handle drops ONCE at scope
           end (not double-counted against the escaping resource). main(5) = (tuple 5 105). A miscount would
           double-free (trap) or read a freed field (wrong result).")
  (peer "cadenza:p/api" (do (def (mk (: x Int64)) #tuple(x x)) (export mk)))
  (input
    (do
      (effect P (op mk (-> Int64 (Tuple Int64 Int64))))
      (bind P "cadenza:p/api")
      (def (main (: x Int64)) (host (P) (let ((t (P.mk x))) #tuple((. t 0) (+ (. t 1) 100)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: (tuple 5 105) (Tuple Int64 Int64)))
  ; the borrowed peer handle / rebuilt escaping compound leaves one boundary handle unreclaimed;
  ; flips to 0 when peer-boundary compound reclaim lands (v-rust-backend / v-memory-safety).
  (live-objects known-leak))

(case
  "the agent-return shape: three peers with a Cedar-gated escaping string result"
  (doc
    "The native agent harness's reported shape — THREE distinct peers: Cedar (cadenza:cedar/api)
           authorize(_s)=1, Inbox (cadenza:inbox/api) next()=\"hi\" (a nullary Unit-arg op), Model
           (cadenza:model/api) converse(p)=\"R:\"++p. main gates on Cedar.authorize: if authorized (=1) it
           returns Model.converse(Inbox.next()) = converse(\"hi\") = \"R:hi\", else \"denied\". The authorized
           path → \"R:hi\" escapes as a String resource reaching all three peer interfaces in one entrypoint.")
  (peer "cadenza:cedar/api" (do (def (authorize (: _s String)) 1) (export authorize)))
  (peer "cadenza:inbox/api" (do (def (next) (String.concat "h" "i")) (export next)))
  (peer
    "cadenza:model/api"
    (do (def (converse (: p String)) (String.concat "R:" p)) (export converse)))
  (input
    (do
      (effect Inbox (op next (-> Unit String)))
      (effect Model (op converse (-> String String)))
      (effect Cedar (op authorize (-> String Int64)))
      (bind Inbox "cadenza:inbox/api")
      (bind Model "cadenza:model/api")
      (bind Cedar "cadenza:cedar/api")
      (def
        (main)
        (if
          (= (host (Cedar) (Cedar.authorize "tool:chat")) 1)
          (host (Model) (Model.converse (host (Inbox) (Inbox.next))))
          "denied"))
      (export main)))
  (call main)
  (output (: "R:hi" String))
  ; the escaping String result reaching three peers leaves two boundary handles unreclaimed (Inbox.next + Model.converse);
  ; flips to 0 when peer-boundary result reclaim lands (v-rust-backend / v-memory-safety).
  (live-objects known-leak))

(case
  "a diamond peer graph shares one provider across two middle peers"
  (doc
    "DIAMOND: a shared provider A (cadenza:base/api) base(x)=2x is consumed by TWO middle peers — B
           (cadenza:b/api) bee(x)=base(x)+1 and C (cadenza:c/api) cee(x)=base(x)+10, each binding A — and the
           top consumer D binds BOTH B and C: main(x)=bee(x)+cee(x). The harness wires A into BOTH B's and C's
           linkers (a SHARED transitive dependency) and both middles into D. main(5): base(5)=10, bee=11,
           cee=20 → 31.")
  (peer "cadenza:base/api" (do (def (base (: x Int64)) (* x 2)) (export base)))
  (peer
    "cadenza:b/api"
    (do
      (effect BA (op base (-> Int64 Int64)))
      (bind BA "cadenza:base/api")
      (def (bee (: x Int64)) (host (BA) (+ (BA.base x) 1)))
      (export bee)))
  (peer
    "cadenza:c/api"
    (do
      (effect CA (op base (-> Int64 Int64)))
      (bind CA "cadenza:base/api")
      (def (cee (: x Int64)) (host (CA) (+ (CA.base x) 10)))
      (export cee)))
  (input
    (do
      (effect DB (op bee (-> Int64 Int64)))
      (effect DC (op cee (-> Int64 Int64)))
      (bind DB "cadenza:b/api")
      (bind DC "cadenza:c/api")
      (def (main (: x Int64)) (host (DB) (host (DC) (+ (DB.bee x) (DC.cee x)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 31 Int64)))

(case
  "reclaim across a nested-compound projection from a peer result"
  (doc
    "A peer op `nest(x) = (tuple (tuple x (+ x 1)) (+ x 2))` @cadenza:nest/api returns a NESTED tuple
           ((x, x+1), x+2); the consumer reads the OUTER tuple (an owned peer result), projects element 0 —
           its INNER tuple, a nested compound, so the inner is dup'd and the outer dropped — then projects
           the inner element 1: `(. (. (P.nest x) 0) 1)`. main(9) = ((9,10),11).0.1 = (9,10).1 = 10.
           Exercises reclaim across a nested-compound projection of a crossed peer handle.")
  (peer
    "cadenza:nest/api"
    (do (def (nest (: x Int64)) #tuple(#tuple(x (+ x 1)) (+ x 2))) (export nest)))
  (input
    (do
      (effect P (op nest (-> Int64 (Tuple (Tuple Int64 Int64) Int64))))
      (bind P "cadenza:nest/api")
      (def (main (: x Int64)) (host (P) (. (. (P.nest x) 0) 1)))
      (export main)))
  (call main (: 9 Int64))
  (output (: 10 Int64))
  ; the nested-compound projection off a crossed peer handle leaves one boundary handle unreclaimed;
  ; flips to 0 when peer-boundary compound reclaim lands (v-rust-backend / v-memory-safety).
  (live-objects known-leak))
