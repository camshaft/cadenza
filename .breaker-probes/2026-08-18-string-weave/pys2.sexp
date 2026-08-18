(case "pys2 a STRING STATE THREAD GROWING THROUGH RESUMES — weave answers the current rope and appends a marker into the state thread, the seed picks ropes of DIFFERENT LENGTHS so the two draws' concatenated byte length separates the runs, and the rope value must flow through the tail resume's answer and state positions intact"
  (input  (do
            (effect E (op weave (-> String)))
            (def (main (: n Int64))
              (handle E (if (= (% n 3) 0) "a" "bb")
                ((weave () s (resume s (String.concat s "x"))))
                (let ((p (E.weave)))
                  (let ((q (E.weave)))
                    (String.byte-len (String.concat p q))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
