(case "a string-literal match pattern matches a runtime ROPE scrutinee by content"
  (doc    "The rope-SCRUTINEE face of string-literal patterns: the pinned match≡chain family (:1553ff)
           selects between FLAT literals with an if; here each scrutinee is a 2-chunk CONCAT rope
           whose seam falls MID-pattern (\"hel\"+\"lo\" vs the pattern \"hello\", \"h\"+\"i\" vs
           \"hi\") — the literal-pattern content test must canonicalize across the chunk boundary
           (a per-chunk or pointer compare misses both) while the non-matching rope falls through
           (100·1 + 10·2 + 0 = 120). The pattern-position companion of the rope==flat equality pin.")
  (input  (do
            (def (classify (: s String))
              (match s ("hello" 1) ("hi" 2) (_ 0)))
            (def (main (: k Int64))
              (+ (* 100 (classify (String.concat "hel" "lo")))
                 (+ (* 10 (classify (String.concat "h" "i")))
                    (classify (String.concat "no" "pe")))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 120 Int64)))
