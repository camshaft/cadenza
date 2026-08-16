(case "a host op returning Bool crosses the boundary and drives a branch"
  (input  (do
            (effect Env (op flag (-> Unit Bool)))
            (def (main)
              (host (Env)
                (if (Env.flag) 100 200)))
            (export main)))
  (host-responses (respond env.flag (: true Bool)))
  (host-calls (call env.flag))
  (output (: 100 Int64)))
