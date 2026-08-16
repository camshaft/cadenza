(case "sb1 an op ARG drawn at a shadow boundary — both the arg draw and the consuming dispatch home to the INNER handler" (input (do
  (effect St (op add (-> Int64 Int64)) (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((add (v) s (resume (+ v s) s))
       (next () s (resume s (+ s 1))))
      (handle St 100
        ((add (v) s (resume (* v s) s))
         (next () s (resume s (+ s 10))))
        (St.add (St.next)))))
  (export main)))
  (call main (: 5 Int64)) (output (: 11000 Int64))
  (call main (: 0 Int64)) (output (: 11000 Int64)))
