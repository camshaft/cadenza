; breaker unpinned-decline sweep — higher-order values + runtime pattern-match surfaces (wasm-first).

(case "ch01 a bare prim passed as a function value applies"
  (input  (do
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: a Int64) (: b Int64)) ((g a) b))
            (def (main (: n Int64)) (ap + n 2))
            (export main)))
  (call   main (: 40 Int64))
  (output (: 42 Int64)))

(case "ch02 a closure stored in a record field projects and applies"
  (input  (do
            (def (main (: n Int64))
              (let ((r (record (= f (fn (x) (+ x n))))))
                ((. r f) 2)))
            (export main)))
  (call   main (: 40 Int64))
  (output (: 42 Int64)))

(case "ch03 a closure stored in a LIST is fetched and applied"
  (input  (do
            (def (main (: n Int64))
              (match (List.at (list (fn (x) (+ x n)) (fn (x) (* x n))) 1)
                ((Option.Some g) (g 3))
                ((Option.None) -1)))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 30 Int64)))

(case "ch04 a runtime-branch-selected closure applies"
  (input  (do
            (def (main (: n Int64))
              (let ((g (if (> n 5) (fn (x) (+ x 1)) (fn (x) (* x 2)))))
                (g n)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 8 Int64)))

(case "ch05 literal STRING patterns match a runtime String"
  (input  (do
            (def (main (: s String))
              (match s
                ("alpha" 1)
                ("beta" 2)
                (_ 0)))
            (export main)))
  (call   main (: "beta" String))
  (output (: 2 Int64)))

(case "ch06 a literal BYTES pattern matches runtime Bytes"
  (input  (do
            (def (main (: b Bytes))
              (match b
                (b"\x01\x02" 1)
                (_ 0)))
            (export main)))
  (call   main (: (list 1 2) Bytes))
  (output (: 1 Int64)))

(case "ch07 Ast.read of a RUNTIME string parses"
  (input  (do
            (def (main (: s String))
              (match (Ast.read s)
                ((Ok (Ast.Int b)) 1)
                ((Ok _) 2)
                ((Err _) 0)))
            (export main)))
  (call   main (: "7" String))
  (output (: 1 Int64)))

(case "ch08 String.from-bytes of RUNTIME-built bytes validates"
  (input  (do
            (def (main (: k Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap k) (UInt8.wrap 98))))
                ((Option.Some s) (String.byte-len s))
                ((Option.None) -1)))
            (export main)))
  (call   main (: 97 Int64))
  (output (: 2 Int64)))
