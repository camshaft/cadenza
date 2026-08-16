(case "m11 an OPTION-of-closure built DIRECTLY (no collection) + perform-conditioned + perform-fed"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((feed (u) s (resume s (+ s 1))))
                (let ((o (if (= (% (St.feed) 2) 1) (Option.Some (fn ((: x Int64)) (+ x 1000))) (Option.None))))
                  (match o
                    ((Option.Some f) (f (St.feed)))
                    ((Option.None) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
