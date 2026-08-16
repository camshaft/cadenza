(case "ss1 a STRING handler state grows per dispatch — each arm returns the pre-growth length, growth is op-arg-branchy"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Log "x"
                ((emit (v) s (resume (String.byte-len s) (String.concat s (if (> v 0) "ab" "c")))))
                (+ (Log.emit n) (+ (* 10 (Log.emit n)) (* 100 (Log.emit 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 531 Int64))
  (call   main (: 0 Int64)) (output (: 321 Int64)))
