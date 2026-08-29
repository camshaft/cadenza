(do (def (main) (const (match #list(1 2 3) (#list(a b c) (* a (* b c))) (_ -1)))) (export main))
