(case "Map.to-list over Bytes KEYS enumerates in the blessed unsigned-lexicographic key order"
  (doc    "The MAP-key companion of the Bytes-Set enumeration pin: keys {[200]↦1, [x=3]↦2, [3,1]↦3}
           must enumerate [3], [3,1], [200] (values 2,3,1 → 231) — the #1120 blessed order driving
           Map.to-list's KEY ordering with the prefix rule and the unsigned face in one shot (a
           signed byte order enumerates [200] first → 123). The guide's Bytes-as-key rationale
           (equality finds, order ENUMERATES) is exactly this observable; the Set face is pinned
           beside it, the Map-key face closes the pair.")
  (input  (do
            (def (main (: x UInt8))
              (let ((m (Map.insert (Map.insert (Map.insert Map.empty
                          (Bytes.of (list 200)) 1)
                          (Bytes.of (list x)) 2)
                          (Bytes.of (list 3 1)) 3)))
                (+ (* 100 (. (Option.expect (List.at (Map.to-list m) 0) "a") 1))
                   (+ (* 10 (. (Option.expect (List.at (Map.to-list m) 1) "b") 1))
                      (. (Option.expect (List.at (Map.to-list m) 2) "c") 1)))))
            (export main)))
  (call   main (: 3 UInt8)) (output (: 231 Int64)))
