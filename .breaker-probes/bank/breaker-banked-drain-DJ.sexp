(case "a map pattern with SYMBOL keys dispatches on a runtime symbol-keyed map"
  (doc    "The SYMBOL-key face of the map pattern (every pinned map-pattern key is an Int or String
           literal): `(map (#\"width\" w) (#\"height\" h))` queries a runtime symbol-keyed map —
           the config-record idiom (#\"width\"↦800 style tables) — with a two-key arm, a one-key
           fall-through arm, and interned-content key probes (1400 / 5000). A presence probe that
           compared symbol keys by handle rather than interned content, or a pattern-key literal
           that failed to intern to the same symbol the builder did, breaks an arm.")
  (input  (do
            (def (mk (: b Bool))
              (if b (Map.insert (Map.insert Map.empty #"width" 800) #"height" 600)
                    (Map.insert Map.empty #"depth" 5)))
            (def (main (: b Bool))
              (match (mk b)
                ((map (#"width" w) (#"height" h)) (+ w h))
                ((map (#"depth" d)) (* d 1000))
                (_ -1)))
            (export main)))
  (call   main (: true Bool)) (output (: 1400 Int64))
  (call   main (: false Bool)) (output (: 5000 Int64)))
