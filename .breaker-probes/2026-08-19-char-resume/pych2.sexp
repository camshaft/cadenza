(case "pych2 probe: an op RESUMES an (Option Char) computed from the handler state via Char.from-int — the alphabet position tracks the state, the body matches Some/None and reads the char back via Char.to-int; a Char (inside an Option) round-trips through the resume seam and the threaded state advances the code point per dispatch"
  (input (do
  (effect E (op letter (-> (Option Char))))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((letter () s (resume (Char.from-int (+ (: 97 Int64) s)) (+ s 1))))
      (+ (* 1000 (match (E.letter) ((Some c) (Char.to-int c)) ((None) (: -1 Int64))))
         (match (E.letter) ((Some c) (Char.to-int c)) ((None) (: -1 Int64))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 98099 Int64))
  (call   main (: 0 Int64)) (output (: 97098 Int64)))
