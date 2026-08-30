(example
  (id "metaprogramming-quote-eval")
  (name "Metaprogramming (quote & eval)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (build base offset) (quasiquote (+ (* (unquote base) (unquote base)) (unquote offset))))

  (def (main) #tuple((build 6 5) (eval (quasiquote (+ (* (unquote 6) (unquote 6)) (unquote 5))))))

  (export main)))
  (expected (: #tuple(((. Ast List) #list(((. Ast Name) "+") ((. Ast List) #list(((. Ast Name) "*") ((. Ast Int) 6) ((. Ast Int) 6))) ((. Ast Int) 5))) 41) (Tuple Ast Int64))))
