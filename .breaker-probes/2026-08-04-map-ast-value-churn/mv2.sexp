(case "mv2 the SNAPSHOT face: an earlier Map version keeps its Ast payload after later overwrites"
  (input  (do
            (def (main (: n Int64))
              (do
                (def m1 (Map.insert Map.empty 7 (Ast.Int (BigInt.of n))))
                (def m2 (Map.insert m1 7 (Ast.Name "later")))
                (+ (* 10 (match (Map.lookup m1 7) ((Some (Ast.Int b)) (Int64.of b)) (_ -1)))
                   (match (Map.lookup m2 7) ((Some (Ast.Name _s)) 1) (_ 0)))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 251 Int64)))
