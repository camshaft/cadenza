(case "a 61-digit BigInt literal parses exactly and round-trips through arithmetic"
  (doc    "The DEEP-parse exactness witness (the pinned huge literal :460 is 21 digits ≈ 4 limbs;
           this one is 61 digits = 2^200, ~10 limbs of decimal→binary carry chain): the literal
           1606…375N (2^200 − 1) plus a runtime k must equal the PRODUCT (2^100)²  — computed by
           exact multiply of two 31-digit literals — exactly when k=1 (1/0 by k). Any drift in the
           decimal parse's digit-table/carry path (the 14fc82ed7 OnceLock-cached table) or in the
           multi-limb multiply makes the equality miss at exactly one k. The arithmetic cross-check
           means the oracle is internal — two independent construction routes to 2^200 must collide.")
  (input  (do
            (def (main (: k Int64))
              (if (= (+ 1606938044258990275541962092341162602522202993782792835301375N (BigInt.of k))
                     (* 1267650600228229401496703205376N 1267650600228229401496703205376N))
                1 0))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
