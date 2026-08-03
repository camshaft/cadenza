; FINDING (v-patterns self-probe, 2026-08-02, trunk e60402ef9) — CDZ0101 unbound `v` +
; invalid wasm on a GUARDED MAP arm whose looked-up key's value is a COMPILE-TIME CONSTANT
; while the map as a whole is RUNTIME (another key holds a dynamic value), when BOTH the guard
; cond AND the arm body reference the value binder `v`.
;
; NOT a decline — the compiler ACCEPTS + emits garbage (CDZ0101 unbound `v` at the body node,
; then "invalid component: failed to parse WebAssembly module"). Reject-don't-miscompile violated
; at the artifact level.
;
; BOUNDARY (all minimized on trunk e60402ef9):
;   FAILS:  (Map.insert (Map.insert Map.empty 1 5) 2 n)  — key 1 = CONST 5 (the looked-up key),
;           key 2 = RUNTIME n; arm = (guard (map (1 v) .. r) (> v 3)) with BODY = v.  → CDZ0101 unbound v
;   PASSES: key 1 runtime ((1 n)(2 5))                          — looked-up value is runtime
;   PASSES: both keys runtime ((1 n)(2 n))
;   PASSES: const-only map (Map.insert Map.empty 1 5)           — whole map folds
;   PASSES: guard reads v but BODY returns 100 (not v)          — only the body-v read leaks
;   PASSES: BODY returns v but NO guard                         — only with the guard present
; So the trigger is the INTERSECTION: a partial-const map (looked-up key const, another key runtime)
; + a guard cond reading v + the body ALSO reading v. The guard-vs-body binder resolution disagrees
; on whether `v` is a const-folded value or a runtime MapField (Case 6mg / guard_cond_map_binds vs
; the body's match_arm_map_binds), so one path const-folds `v` away and the other emits an unbound
; reference. Suspected: partial-const map value folding makes the looked-up value a ConstInt on one
; resolution path but a MapField read on the other; the guard desugar (extract cond+body) severs them.
;
; Likely v-patterns lane (guarded-map-arm binder resolution, resolve.rs Case 6mg + the guard-scalar/
; map desugar in lower.rs) — same "guard desugar severs a binder" family as Finding #46 (scalar) and
; the do_local_binds re-parent class, but here gated on PARTIAL-CONST map value folding.
;
; Expected (hand-derived): main n=9 → map {1:5, 2:9}; key 1 value v=5; guard (> 5 3) holds → body v = 5.

(case "a guarded map arm whose looked-up key is const but another key is runtime resolves its value binder in both guard and body"
  (input  (do
            (def (main (: n Int64))
              (match (Map.insert (Map.insert Map.empty 1 5) 2 n)
                ((guard (map (1 v) .. r) (> v 3)) v)
                (_ -1)))
            (export main)))
  (call   main (: 9 Int64)) (output (: 5 Int64)))

; --- ROOT-CAUSE PROGRESS (v-patterns, Inc-456 trace on trunk 5aac6e8ce) ---
; RESOLUTION SUCCEEDS: node 22 (guard-cond `v`) AND node 26 (body `v`) both resolve to
;   MapField{scrutinee:12, key:15} cleanly (traced rcdzc::resolve). So it is NOT a resolve-stage
;   binder-ascent failure.
; The `unbound v` comes from the EMIT/DESUGAR stage: `desugar_runtime_map_match` fires (map is
;   runtime: key 2 = `n`), and a CLONE of the arm (node 6119, a fresh `v` in the 6000+ range with a
;   re-parented scrutinee 6109) lowers to Poison(unbound `v`) — traced rcdzc::lower:134. Two "runtime
;   map match → nested presence-test if-chain" traces fire (scrutinee 6109 then 12), i.e. the desugar's
;   `rewritten` match is re-lowered / re-entered and the cloned `v` in it never got a resolution.
; So the desugar re-parents/clones the guarded arm (guard wrap at lower.rs ~7357 `(if guard body else)`
;   + the presence-if chain), and on that clone the `v` (whose MapField scrutinee is the ORIGINAL map)
;   is severed → unbound. Only trips when the LOOKED-UP key's value is a CONST (5) while the map is
;   runtime — the partial-const shape must be what routes `v`'s MapField through a clone path that the
;   all-runtime and all-const shapes avoid (all-const → const fold path; all-runtime → body reused
;   verbatim + memo wins per the 7236 comment). NEXT: find where the 6119 clone is minted (guard wrap
;   vs a value-fold re-lower) and either forget+reresolve `v` against the clone or reuse the arm verbatim
;   (memo) as the all-runtime path does. Same "guard-desugar severs a binder" family, partial-const face.
