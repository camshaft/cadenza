(do
  (effect St (op feed (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle St (list)
      ((feed (v) s (resume (List.len s) (List.push s v))))
      (+ (St.feed 20) (+ (St.feed n) (St.feed 30)))))
  (export main))
