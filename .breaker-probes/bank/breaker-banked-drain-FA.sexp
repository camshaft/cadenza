(case "a handle whose value is a RECORD assembled from resume results is intact after discharge"
  (doc    "The RECORD-result face of the handle-yields-collection family (drain DU pinned a Map; this
           is a labeled product whose THREE fields each come from a resume): the handler seeds a
           counter, and the record literal's fields `a`/`b`/`c` evaluate LEFT-TO-RIGHT — a reads 3
           (state→4), b reads 4, c reads 5 → record {a:3,b:4,c:5}, projected to 345. The record
           escapes the discharged handle and each field must hold the state value at ITS evaluation
           point (a field-order reordering, or a re-materialize that re-read the seed, drifts a
           digit). The construct-order-under-a-handler + escaping-record composition.")
  (input  (do
            (effect Src (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (let ((r (handle Src n
                         ((next (u) s (resume s (+ s 1))))
                         (record (a (Src.next)) (b (Src.next)) (c (Src.next))))))
                (+ (* 100 (. r a)) (+ (* 10 (. r b)) (. r c)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64)))
