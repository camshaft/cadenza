(case "nu2 TWO newtype wrappers over the same inner keep separate tries type-distinct"
  (input  (do
            (type UserId (Mk Int64))
            (type GroupId (Mk Int64))
            (def (fillu (: i Int64) (: m (Map UserId Int64)))
              (if (= i 0) m (fillu (- i 1) (Map.insert m (UserId.Mk i) (* i 2)))))
            (def (fillg (: i Int64) (: m (Map GroupId Int64)))
              (if (= i 0) m (fillg (- i 1) (Map.insert m (GroupId.Mk i) (* i 3)))))
            (def (main (: n Int64))
              (do
                (def mu (fillu n Map.empty))
                (def mg (fillg n Map.empty))
                (+ (* 100 (match (Map.lookup mu (UserId.Mk 10)) ((Some v) v) ((None _u) -1)))
                   (match (Map.lookup mg (GroupId.Mk 10)) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 2030 Int64)))
