(case "the empty symbol as a compound-key LEAF hashes distinctly and is found by its runtime-interned twin"
  (doc    "The EMPTY-symbol boundary inside a compound key: `(tuple #\"\" 1)` and `(tuple #\"x\" 1)`
           are DISTINCT CHAMP keys (len 2 — a hash that treated the zero-length name as a null/skip
           collapses them), and the lookup probe builds its key with `(Symbol.of \"\")` — the
           RUNTIME-interned empty symbol must land on the same interned identity as the literal
           `#\"\"` (10s digit reads 10... encoded 10·10 + 2 = 102). Composes the pinned empty-symbol
           self-equality (:182) with interning-through-a-compound-hash — the degenerate-name face of
           the symbol-keyed-map family.")
  (input  (do
            (def (main (: k Int64))
              (let ((m (Map.insert (Map.insert Map.empty (tuple #"" 1) 10) (tuple #"x" 1) 20)))
                (+ (* 10 (match (Map.lookup m (tuple (Symbol.of "") 1)) ((Some v) v) ((None u) -1)))
                   (Map.len m))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 102 Int64)))
