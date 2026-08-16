(case "uc1 String equality across the effect boundary with MULTIBYTE content (rope vs flat, arm-built)"
  (input  (do
            (effect St (op mk (-> Unit String)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (String.concat "é" "∀") s)))
                (if (= (St.mk) "é∀") 1 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
