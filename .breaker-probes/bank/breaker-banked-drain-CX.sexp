(case "a map pattern over a fused CALL-result scrutinee tests key presence per-branch"
  (doc    "Map patterns × the match-fusion seam: the scrutinee is a CALL result (`mk` — an if of two
           differently-KEYED map builds), and the arms are key-presence QUERIES, not structural
           shapes — {1,2} present → v+w (30); miss falls through to the {3} query (3000); a
           key-presence test is refutable on presence, so the dispatch is a presence-probe CHAIN the
           fusion must clone coherently per branch (a clone that evaluated arm 1's probes against the
           OTHER branch's map, or cached a probe result across branches, mixes the arms). The
           fused-scrutinee companion of the runtime map-pattern pins (:375 — direct if-built value,
           single arm; here the sum-free two-branch join feeds a multi-arm presence dispatch).")
  (input  (do
            (def (mk (: b Bool))
              (if b (Map.insert (Map.insert Map.empty 1 10) 2 20)
                    (Map.insert Map.empty 3 30)))
            (def (main (: b Bool))
              (match (mk b)
                ((map (1 v) (2 w)) (+ v w))
                ((map (3 z)) (* z 100))
                (_ -1)))
            (export main)))
  (call   main (: true Bool)) (output (: 30 Int64))
  (call   main (: false Bool)) (output (: 3000 Int64)))
