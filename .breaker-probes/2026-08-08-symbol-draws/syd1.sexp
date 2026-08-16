(case "syd1 an op returns a SYMBOL picked by state parity — symbol equality gates a branch that draws again"
  (input  (do
            (effect E (op tag (-> Symbol)) (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((tag () s (resume (if (= (% s 2) 0) #"alpha" #"beta") (+ s 1)))
                 (next () s (resume s (+ s 1))))
                (if (= (E.tag) #"alpha")
                    (+ 100 (E.next))
                    (+ 200 (E.next)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 105 Int64))
  (call   main (: 7 Int64)) (output (: 208 Int64))
  (call   main (: -2 Int64)) (output (: 99 Int64)))
