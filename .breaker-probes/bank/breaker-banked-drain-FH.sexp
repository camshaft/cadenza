(case "a fold of Bytes.concat builds a growing byte-rope whose length and bytes read back exactly"
  (doc    "The Bytes analogue of the String concat-loop scale pin: a tail-recursive fold appends one
           fresh byte (UInt8.wrap i) per iteration via Bytes.concat, growing a deep byte-rope — read
           back len n, first byte 0, last byte n-1 (n=5 → 504; n=1 → 100, the single-chunk floor).
           The rope-3-leaf compact pins are shallow; this exercises the per-iteration concat +
           positional Bytes.at addressing across n rope seams (a fold that lost a chunk or double-
           counted a seam drifts the len; a mis-addressed at reads the wrong byte). The byte-builder
           idiom (assembling a wire buffer incrementally).")
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc Bytes))
              (if (= i n) acc (build (+ i 1) n (Bytes.concat acc (Bytes.of (list (UInt8.wrap i)))))))
            (def (main (: n Int64))
              (let ((b (build 0 n (Bytes.of (list)))))
                (+ (* 100 (Bytes.len b))
                   (+ (* 10 (Int64.of (Option.expect (Bytes.at b 0) "b0")))
                      (Int64.of (Option.expect (Bytes.at b (- n 1)) "blast"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 504 Int64))
  (call   main (: 1 Int64)) (output (: 100 Int64)))
