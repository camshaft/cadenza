;; HELD-PIN-ON-LAND (corpus-bugfix, 2026-07-29): PR#894 leak (Copilot 3674022652), v-rust-backend.
;; A Set/Map with a COMPOUND-OPTION KEY — element/key type (Tuple (Option Int64) Int64) — emitted
;; BTreeSet<(__CdzOpt<i64>, i64)> (ord_key_type recurses into Option) with a BARE (Option,i64) VALUE
;; (wrap_ord_key's Tuple arm gates float-only, skips Option) -> rustc E0308 + missed __CdzOpt injection.
;; FIX (v-rust-backend bbcd80a0d, HELD behind queued EmitTestsConsumerOnly 4c8a8dc51): ty_is_ord_key now
;; DECLINES a compound key CONTAINING a built-in Option (threading __CdzOpt through compounds is a later
;; increment; decline > E0308). A BARE Option key still works (#42 w2, pinned). rust-lib witness:
;; a_compound_key_containing_a_nested_option_declines_cleanly_not_e0308 (rcdzc/backend/rust/tests.rs).
;; ON LAND (bbcd80a0d): derive a corpus-reachable Set/Map-with-(Tuple (Option Int64) Int64)-KEY shape
;; that reaches the rust emit (my Set.of-list probe hits CDZ0201 upstream; Map.insert compound-Option-key
;; PASSES both — need the exact key-typed shape v-rust-backend's rust-lib test uses). Gate -> rust DECLINES
;; (todo/declines), wasm may compute; pin the decline into 19-sets/05-compound. If no clean corpus-reachable
;; shape exists (the leak is emit-internal, covered by their rust-lib test), SKIP the corpus pin and note it.

;; UPDATE (2026-07-29): fix re-shaed to 329096617 (MR queued, base e4b75e912; gates rust 5120/111/0 +
;; rust-async 5117/114/0 + wasm 5208/23/0 0-regress). CORPUS-REACHABILITY DETERMINATION (corpus-bugfix
;; probed trunk fb75237da): NO easily-corpus-reachable shape reaches the rust ord-key emit —
;;   • (Set.of (list (tuple (Some n) 1) ...)) -> CDZ0201 (Set.of-list reducibility limit) BEFORE emit
;;   • Map.insert (tuple (Some n) 1) key -> PASSES both (doesn't hit the wrap gap)
;;   • param-typed (Set (Tuple (Option Int64) Int64)) + Set.contains -> ALSO CDZ0201 (todo) on wasm
;; So the E0308 leak is EMIT-INTERNAL, only exercised by v-rust-backend's rust-lib test
;; (a_compound_key_containing_a_nested_option_declines_cleanly_not_e0308). LIKELY OUTCOME on 329096617
;; land: SKIP the corpus pin (no clean reachable witness) and note the rust-lib test as the guard. RE-CHECK
;; on land in case the decline changes the CDZ0201-blocked shapes' disposition (if a shape then reaches the
;; decline cleanly, pin it; else skip + note).
