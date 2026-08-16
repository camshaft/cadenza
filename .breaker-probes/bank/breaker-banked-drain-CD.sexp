(case "a subtree replacement over a SHARED user-sum tree leaves the sibling reference intact"
  (doc    "Persistence under ALIASING for the Ast→Ast transformation shape: `shared` = (Add k 2) is
           bound ONCE and used as BOTH children of `t` (a genuinely aliased subtree, the sharing every
           content-addressed structural editor produces), then `repl` rewrites Lit-2 → Lit-99. The
           rewrite must build a NEW tree (t2 evaluates 99+3 twice → 204) while the ORIGINAL t still
           evaluates 5+5 → 10 (encoded 204010 at k=3) — an in-place rewrite through one alias corrupts
           the other child AND the original; an over-drop of the replaced shared node breaks the
           second ev walk. The aliased-input companion of the CORE replacement pins (whose inputs are
           fresh trees).")
  (input  (do
            (type Exp (Lit Int64) (Add Exp Exp))
            (def (ev e) (match e ((Lit n) n) ((Add a b) (+ (ev a) (ev b)))))
            (def (repl e) (match e
              ((Lit n) (if (= n 2) (Lit 99) (Lit n)))
              ((Add a b) (Add (repl a) (repl b)))))
            (def (main (: k Int64))
              (let ((shared (Add (Lit k) (Lit 2))))
                (let ((t (Add shared shared)))
                  (let ((t2 (repl t)))
                    (+ (* 1000 (ev t2)) (ev t))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 204010 Int64)))
