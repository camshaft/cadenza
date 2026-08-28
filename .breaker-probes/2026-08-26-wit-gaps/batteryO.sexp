; breaker WIT battery O — flatten-arity spill boundaries + deep non-string shapes (wasm-first).

(case "co01 a 20-field s64 record export PARAM (flatten spill) sums across the boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (a1 (s64)) (a2 (s64)) (a3 (s64)) (a4 (s64)) (a5 (s64)) (a6 (s64)) (a7 (s64)) (a8 (s64)) (a9 (s64)) (a10 (s64)) (a11 (s64)) (a12 (s64)) (a13 (s64)) (a14 (s64)) (a15 (s64)) (a16 (s64)) (a17 (s64)) (a18 (s64)) (a19 (s64)) (a20 (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: m (Record (: a1 Int64) (: a2 Int64) (: a3 Int64) (: a4 Int64) (: a5 Int64) (: a6 Int64) (: a7 Int64) (: a8 Int64) (: a9 Int64) (: a10 Int64) (: a11 Int64) (: a12 Int64) (: a13 Int64) (: a14 Int64) (: a15 Int64) (: a16 Int64) (: a17 Int64) (: a18 Int64) (: a19 Int64) (: a20 Int64))))
      (+ (. m a1) (+ (. m a10) (. m a20))))
    (export f)))
  (call f (: (record (= a1 1) (= a2 2) (= a3 3) (= a4 4) (= a5 5) (= a6 6) (= a7 7) (= a8 8) (= a9 9) (= a10 10) (= a11 11) (= a12 12) (= a13 13) (= a14 14) (= a15 15) (= a16 16) (= a17 17) (= a18 18) (= a19 19) (= a20 20)) (Record (: a1 Int64) (: a2 Int64) (: a3 Int64) (: a4 Int64) (: a5 Int64) (: a6 Int64) (: a7 Int64) (: a8 Int64) (: a9 Int64) (: a10 Int64) (: a11 Int64) (: a12 Int64) (: a13 Int64) (: a14 Int64) (: a15 Int64) (: a16 Int64) (: a17 Int64) (: a18 Int64) (: a19 Int64) (: a20 Int64))))
  (output (: 31 Int64)))

(case "co02 a 20-field s64 record export RESULT (spilled result) crosses back"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result ("record" (b1 (s64)) (b2 (s64)) (b3 (s64)) (b4 (s64)) (b5 (s64)) (b6 (s64)) (b7 (s64)) (b8 (s64)) (b9 (s64)) (b10 (s64)) (b11 (s64)) (b12 (s64)) (b13 (s64)) (b14 (s64)) (b15 (s64)) (b16 (s64)) (b17 (s64)) (b18 (s64)) (b19 (s64)) (b20 (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: x Int64))
      (record (= b1 x) (= b2 2) (= b3 3) (= b4 4) (= b5 5) (= b6 6) (= b7 7) (= b8 8) (= b9 9) (= b10 10) (= b11 11) (= b12 12) (= b13 13) (= b14 14) (= b15 15) (= b16 16) (= b17 17) (= b18 18) (= b19 19) (= b20 (* x 2))))
    (export f)))
  (call f (: 9 Int64))
  (output (: (record (= b1 9) (= b2 2) (= b3 3) (= b4 4) (= b5 5) (= b6 6) (= b7 7) (= b8 8) (= b9 9) (= b10 10) (= b11 11) (= b12 12) (= b13 13) (= b14 14) (= b15 15) (= b16 16) (= b17 17) (= b18 18) (= b19 19) (= b20 18)) (record (b1 Int64) (b2 Int64) (b3 Int64) (b4 Int64) (b5 Int64) (b6 Int64) (b7 Int64) (b8 Int64) (b9 Int64) (b10 Int64) (b11 Int64) (b12 Int64) (b13 Int64) (b14 Int64) (b15 Int64) (b16 Int64) (b17 Int64) (b18 Int64) (b19 Int64) (b20 Int64)))))

(case "co03 list<option<record>> result crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("list" ("option" ("record" (v (s64)))))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: m (Record (: x Int64))))
      (list (Option.Some (record (= v (. m x)))) Option.None (Option.Some (record (= v 7)))))
    (export f)))
  (call f (: (record (= x 5)) (Record (: x Int64))))
  (output (: ((Some (record (= v 5))) (None unit) (Some (record (= v 7)))) (List (Option (record (v Int64)))))))

(case "co04 option<variant-with-payload> field crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (o ("option" ("variant" (keep) (drop (s64))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (type Act (Keep) (Drop Int64))
    (def (f (: m (Record (: x Int64))))
      (record (= o (if (= (. m x) 0) (Option.Some Act.Keep) (Option.Some (Act.Drop (. m x)))))))
    (export f)))
  (call f (: (record (= x 4)) (Record (: x Int64))))
  (output (: (record (= o (Some (drop 4)))) (record (o (Option act))))))

(case "co05 depth-4 nested record result crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (l1 ("record" (l2 ("record" (l3 ("record" (v (s64)))))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do
    (def (f (: m (Record (: x Int64))))
      (record (= l1 (record (= l2 (record (= l3 (record (= v (* (. m x) 3))))))))))
    (export f)))
  (call f (: (record (= x 5)) (Record (: x Int64))))
  (output (: (record (= l1 (record (= l2 (record (= l3 (record (= v 15)))))))) (record (l1 (record (l2 (record (l3 (record (v Int64)))))))))))
