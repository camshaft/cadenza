(case "cch1 a ONE-SLOT memo cache with an OPTION state — get answers the cached value tagged as a hit when the stored key matches, else computes the square plus the seed bias caching and counting the miss, stats packs hits misses and occupancy, and the bias decides whether the THIRD lookup aliases the hot key (n zero re-hits three) or probes a cold one (n ten misses on four)"
  (input  (do
            (effect C
              (op get (-> Int64 Int64))
              (op stats (-> Int64)))
            (def (main (: n Int64))
              (handle C (tuple (: None (Option (Tuple Int64 Int64))) (: 0 Int64) (: 0 Int64))
                ((get (k) st
                  (match st
                    ((tuple slot hits misses)
                      (match slot
                        ((Some kv)
                          (match kv
                            ((tuple k0 v0)
                              (if (= k0 k)
                                  (resume (+ (* v0 10) 1) (tuple slot (+ hits 1) misses))
                                  (resume (* (+ (* k k) (% n 3)) 10)
                                          (tuple (Some (tuple k (+ (* k k) (% n 3)))) hits (+ misses 1)))))))
                        ((None)
                          (resume (* (+ (* k k) (% n 3)) 10)
                                  (tuple (Some (tuple k (+ (* k k) (% n 3)))) hits (+ misses 1))))))))
                 (stats () st
                  (match st
                    ((tuple slot hits misses)
                      (match slot
                        ((Some kv) (resume (+ (* hits 100) (+ (* misses 10) 1)) st))
                        ((None) (resume (+ (* hits 100) (* misses 10)) st)))))))
                (let ((a (C.get (: 3 Int64))))
                  (let ((b (C.get (: 3 Int64))))
                    (let ((c (C.get (+ (: 3 Int64) (% n 3)))))
                      (let ((d (C.get (: 3 Int64))))
                        (let ((f (C.stats)))
                          (+ (* 10000 (+ (* 10000 (+ (* 10000 (+ (* 10000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1000101017001000131 Int64))
  (call   main (: 0 Int64)) (output (: 900091009100910311 Int64)))
