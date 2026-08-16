(case "uc2 a multibyte String as a Map KEY inserted in-arm and probed by its flat twin in-body"
  (input  (do
            (effect St (op stash (-> Unit Int64)) (op grab (-> Unit (Map String Int64))))
            (def (main (: n Int64))
              (handle St Map.empty
                ((stash (u) s (resume 0 (Map.insert s (String.concat "é" "∀") 42)))
                 (grab (u) s (resume s s)))
                (do
                  (def _x (St.stash))
                  (match (Map.lookup (St.grab) "é∀") ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
