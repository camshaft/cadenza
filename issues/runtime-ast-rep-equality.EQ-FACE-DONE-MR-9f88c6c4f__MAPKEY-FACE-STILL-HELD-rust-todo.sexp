;; ROOT ISOLATED (breaker #36): the bug is READ's reify SPECIFICALLY. Four-variant perimeter:
;; (a) runtime Ast.Int payload trips it identically to Ast.Name → payload KIND irrelevant;
;; (b) const-BUILT Ast.List vs partially-runtime list WORKS (1/0);
;; (c) (read ...) vs partially-runtime list DIRECTLY = invalid module (NO rebuild needed);
;; (d) (quote ...) vs the same partially-runtime list WORKS (1/0).
;; ⇒ quote's reify produces a rep the mixed equality bridges, but READ's reify (lower_read's
;; reify_read_ast) emits a DIFFERENT Ast rep → fix seam = make read's reify match quote/Ast.* rep.

;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until the mixed const/runtime Ast rep boundary
;; in equality/reify is fixed. Origin: breaker FINDING (issue 000000017484). CONFIRMED trunk 31a5f4f32:
;; [main 1] expected 1 — wasm 'invalid component: func[10] failed to validate', rust E0308 mismatched
;; types (BOTH backends, differently). A runtime-selected (Ast.Name <String>) payload in a rebuilt list,
;; compared (=) against a const-folded (read ...) → the equality lowering/reify commits to MISMATCHED
;; REPS (const-Ast vs runtime-heap-Ast). Either ingredient alone is fine (runtime-payload Name matched
;; directly 4/4; const-payload rebuild+compare 1/0); only the partially-runtime-vs-fully-const = composition
;; breaks. Cannot ride todo (rust FAILS to build). OWNER: Ast reify/equality lowering (v-metaprogramming;
;; rust-emit half may need v-rust-backend). Oracle 1/0. ON FIX: rebuild cdz; gate x3 → 1/0; pin into
;; 12-metaprogramming.sexp beside the quote/read/reify pins; baseline x3; roundtrip + --check; MR.

(case "a runtime-selected Name payload inside a rebuilt Ast compares against a read result"
  (input  (do
        (def (main (: mode Int64))
          (match (read "(defn add 1)")
            ((Ast.List parts)
              (match parts
                ((list (Ast.Name _kw) rest .. more)
                  (if (= (Ast.List (List.prepend (List.prepend more rest)
                                                 (Ast.Name (if (= mode 1) "defx" "defy"))))
                         (read "(defx add 1)"))
                      1 0))
                (_ -2)))
            (_ -3)))
        (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 0 Int64)))

;; MAP-KEY CONSUMER FACE (breaker #36 widen): a read result WITH an Int payload used as a CHAMP map
;; key breaks ALONE (no mixed equality) — reify_read_ast's Int rep (Ty::int64 vs boxed-BigInt) is
;; unusable by champ_hash/eq too. Verified on trunk 31a5f4f32: insert (read "(a 1)")→42, lookup by
;; (read "(a 1)") → 17 (MISS, wasm func[17] invalid before the fix); the (a b)-no-Int twin works (42);
;; the quote-key twin works (42). Same root as the = face — 191e65164's Ty::BigInt retype fixes both.
;; This face guards that the fix reaches ALL structural consumers, not just equality.
(case "a read result with an Int payload used as a map key round-trips (read-reify rep usable by champ hash)"
  (input  (do
        (def (main)
          (match (Map.lookup (Map.insert (Map.empty) (read "(a 1)") 42) (read "(a 1)"))
            ((Some v) v)
            ((None _u) -1)))
        (export main)))
  (output (: 42 Int64)))
