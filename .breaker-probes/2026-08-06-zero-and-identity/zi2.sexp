(case "zi2 a handle whose body NEVER performs is exactly its body (zero dispatches)"
  (input  (do
            (effect St (op never (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 100
                ((never (u) s (resume s s)))
                (* n 2)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))
