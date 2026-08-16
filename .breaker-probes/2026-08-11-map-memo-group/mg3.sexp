(case "mg3 a REAL Map-keyed memo group — three defs thread the CHAMP cache, a hit short-circuits the recompute"
  (input  (do
            (effect St (op get (-> Int64 Int64)) (op put (-> Int64 Int64 Int64)))
            (def (type-of (: id Int64))
              (let ((cur (St.get id)))
                (if (= cur 0) (cache-type id (compute-type id)) cur)))
            (def (cache-type (: id Int64) (: t Int64))
              (let ((_w (St.put id t))) t))
            (def (compute-type (: id Int64))
              (if (= id 0) 5
                (let ((_v (type-of (- id 1))))
                  (let ((b (type-of (- id 1)))) b))))
            (def (main (: k Int64))
              (handle St Map.empty
                ((get (id) s (resume (match (Map.lookup s id) ((Some v) v) ((None _u) 0)) s))
                 (put (id v) s (resume v (Map.insert s id v))))
                (type-of k)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64)))
