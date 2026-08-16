(case "sa2 a string BUILT from a prior draw's branch becomes the next op's argument — the tag arm reads the post-pick state"
  (input  (do
            (effect Log (op pick (-> Int64)) (op tag (-> String Int64)))
            (def (main (: n Int64))
              (handle Log n
                ((pick () s (resume s (+ s 1)))
                 (tag (w) s (resume (* (String.byte-len w) s) s)))
                (Log.tag (String.concat "id-" (if (> (Log.pick) 3) "long" "s")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64))
  (call   main (: 1 Int64)) (output (: 8 Int64)))
