; BREAKER REPRODUCER 2026-07-17 — rust-backend Todo→Fail regression (landed in batch 160, ffdeda5ff).
; The effects case "a state-advancing helper called before a later read threads its write through the
; continuation" (14-effects-and-handlers.sexp:3648) PASSES on wasm (value 50) but the RUST backend now
; FAILS to build: error[E0282]: type annotations needed for `BTreeMap<_, _>`.
; TRUNK BASELINE: rust verdict was `todo` (clean decline); FRESH BINARY: `fail` (artifact does not build).
; A Todo→Fail transition = a real regression (MEMORY.md gate rule). wasm computes 50 correctly, so this is a
; RUST-BACKEND codegen gap, not a semantic miscompile: the handler state `(Map.empty)` lowers to a Rust
; `BTreeMap` whose key/value types are not fixed by usage in the generated code → rustc E0282. Likely exposed
; by the effects inline/hoist fix e59f2c80f (wasm-focused) not covering the rust emit path for an empty-Map
; handler state. Owner: v-rust-backend (rust emit must annotate the BTreeMap element types, or the handler-
; state lowering must carry the effect op's Tuple/Int64 types to the empty-map construction).
; rust-async fails identically. Same case is PASS on wasm and PASS on the trunk wasm baseline.
(case "a state-advancing helper called before a later read threads its write through the continuation"
  (doc "repro of the rust-backend E0282 — see 14-effects-and-handlers.sexp:3648 for the graded original")
  (input  (do
            (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
            (def (demand (: k Int64) (: compute Int64))
              (match (Db.get k)
                (((. Option Some) v) v)
                (((. Option None) u) (do (Db.put (tuple k compute)) compute))))
            (def (run-then-get)
              (handle Db (Map.empty)
                ((get (k) s (resume (Map.lookup s k) s))
                 (put (kv) s (match kv ((tuple k v) (resume unit (Map.insert s k v))))))
                (let ((a (demand 5 25)))
                  (match (Db.get 5) (((. Option Some) v) (+ a v)) (((. Option None) u) 99)))))
            (export run-then-get)))
  (output (: 50 Int64)))

; ---
; RESOLVED (corpus-bugfix 2026-07-17, from v-rust-backend note): already LANDED in batch 164
; (v-rust-backend 1b528bcc5, empty-Map handler-state annotation fix). Green on current trunk.
