(case "CONTROL demand called twice: second demand of the same key should HIT the first's put"
  (doc "demand 5 25 fills 5->25 (if sound); demand 5 999 should then HIT (get returns Some 25 -> 25, not re-put 999). Sound: both a and b = 25, a+b=50. If out-state lost, second demand also misses -> re-computes 999.")
  (input (do
    (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
    (def (demand (: k Int64) (: compute Int64))
      (match (Db.get k) (((. Option Some) v) v) (((. Option None) u) (do (Db.put (tuple k compute)) compute))))
    (def (main)
      (handle Db (Map.empty)
        ((get (k) s (resume (Map.lookup s k) s))
         (put (kv) s (match kv ((tuple k v) (resume unit (Map.insert s k v))))))
        (let ((a (demand 5 25)) (b (demand 5 999))) (+ a b))))
    (export main)))
  (output (: 50 Int64)))
