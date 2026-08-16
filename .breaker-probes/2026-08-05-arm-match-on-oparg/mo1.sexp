(case "mo1 an arm that MATCHES its op-arg sum and resumes differently per variant (SINGLE resume-site via value-hoist)"
  (input  (do
            (effect St (op eat (-> (Option Int64) Int64)))
            (def (main (: n Int64))
              (handle St n
                ((eat (o) s (resume (match o ((Option.Some v) (* v 10)) ((Option.None) -1)) s)))
                (+ (St.eat (Option.Some n)) (St.eat (Option.None)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 49 Int64)))
