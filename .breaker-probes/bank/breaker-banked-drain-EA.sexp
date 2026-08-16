(case "TWO independent fallible lookups matched JOINTLY through one tuple scrutinee"
  (doc    "The heterogeneous joint-match: a List.at (RRB index) and a Map.lookup (CHAMP probe) —
           two DIFFERENT fallible collection ops — feed ONE tuple scrutinee, and the four-quadrant
           dispatch reads both tags: both-hit 20+5=25, list-only 10, map-only 500, neither -1. The
           pinned quadrant matches build both Options from the SAME producer shape (branch-built ifs,
           :6141); here the two Options materialize from different collection walks whose Some
           payloads carry different provenance (an element vs an entry value) — a joint dispatch that
           read one op's tag twice, or unified the two Some payload slots, breaks a quadrant.")
  (input  (do
            (def (main (: i Int64) (: j Int64))
              (let ((xs (list 10 20)) (m (Map.insert Map.empty 1 5)))
                (match (tuple (List.at xs i) (Map.lookup m j))
                  ((tuple (Some a) (Some b)) (+ a b))
                  ((tuple (Some a) (None u)) a)
                  ((tuple (None u) (Some b)) (* b 100))
                  (_ -1))))
            (export main)))
  (call   main (: 1 Int64) (: 1 Int64)) (output (: 25 Int64))
  (call   main (: 0 Int64) (: 9 Int64)) (output (: 10 Int64))
  (call   main (: 5 Int64) (: 1 Int64)) (output (: 500 Int64))
  (call   main (: 5 Int64) (: 9 Int64)) (output (: -1 Int64)))
