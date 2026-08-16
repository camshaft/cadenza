(case "an @ensures predicate projecting a MAP lookup out of the returned collection enforces at exit"
  (doc    "The COLLECTION-OP predicate face of @ensures (the projection family pins accessor chains —
           tuple components, record fields, and 941a5148d's nested `(. (. ret 0) 1)`; this predicate
           runs a Map.lookup + Option match over the returned CHAMP): `mk` must return a map whose
           key-1 value is positive — a=5 satisfies (main reads len 1), a=0 violates (the looked-up 0
           fails `> v 0` → the ensures trap). The rewrite binds `ret` to a HEAP collection and the
           predicate exercises a real collection operation against it at body-exit, not just an
           accessor walk; the None arm's `false` also pins that a predicate may DENY a shape outright.")
  (input  (do
            (@ (ensures (match (Map.lookup ret 1) ((Some v) (> v 0)) ((None u) false)))
               (def (mk (: a Int64)) (Map.insert Map.empty 1 a)))
            (def (main (: a Int64))
              (Map.len (mk a)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (trap "unreachable"))
