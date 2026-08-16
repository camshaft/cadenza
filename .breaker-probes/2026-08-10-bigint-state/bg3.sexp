(case "bg3 a BIGINT handler state — tripling per dispatch crosses the i64 boundary mid-thread, the exact multi-limb value survives"
  (input  (do
            (effect E (op triple (-> Int64)) (op report (-> BigInt)))
            (def (main (: n Int64))
              (handle E (+ (BigInt.of 1000000000000000000) (BigInt.of n))
                ((triple () s (resume 1 (* s (BigInt.of 3))))
                 (report () s (resume s s)))
                (do (E.triple) (E.triple) (E.triple) (E.report))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 27000000000000000027 BigInt))
  (call   main (: 0 Int64)) (output (: 27000000000000000000 BigInt)))
