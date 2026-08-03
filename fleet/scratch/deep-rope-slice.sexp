(case "a 2000-deep runtime Bytes.concat rope flattens iteratively and reads its content stack-safe"
  (doc    "The existing recursively-built-bytes case is depth 4; this drives the concat ROPE to depth 2000 so
           the runtime's ITERATIVE `bytes_flatten` (a rope-tree walk) is exercised at a depth that would
           overflow a naive RECURSIVE flatten's native stack. `rep` builds a left-growing rope by prepending a
           1-byte leaf `[i%256]` at each of n steps (each `Bytes.concat` is a rope node over the running rope),
           so after n steps the value is a 2000-node-deep rope of length 2000 whose byte at index i is
           `(2000-1-i)%256` (the first-prepended byte ends up LAST). Reading forces the flatten: `sum` reads
           EVERY index with `Bytes.at` and totals the bytes mod a large prime-ish poison. Since bytes cycle
           0..255, Sigma over one full 0..255 cycle = 32640; 2000 = 7*256 + 208, so total = 7*32640 + Sigma
           0..207 = 228480 + 21528 = 250008. A flatten that overflowed, or a rope walk that mis-ordered/lost a
           node, changes the sum (a None on any read poisons by -1000000). Runtime n keeps it out of the
           const-fold, exercising the real heap rope + iterative flatten. The DEEP companion of the depth-4
           recursively-built-bytes case.")
  (input  (do
            (def (rep (: i Int64) (: n Int64) (: acc Bytes))
              (if (< i n) (rep (+ i 1) n (Bytes.concat (Bytes.of (list (UInt8.wrap (% i 256)))) acc)) acc))
            (def (sum (: i Int64) (: b Bytes) (: acc Int64))
              (if (< i 0) acc
                (sum (- i 1) b (+ acc (match (Bytes.at b i) ((Some v) v) ((None u) -1000000))))))
            (def (main (: n Int64))
              (let ((r (rep 0 n (Bytes.of (list)))))
                (sum (- (Bytes.len r) 1) r 0)))
            (export main)))
  (call   main (: 2000 Int64)) (output (: 250008 Int64)))
