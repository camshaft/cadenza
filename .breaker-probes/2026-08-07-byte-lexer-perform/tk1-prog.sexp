(do
  (effect Tok (op take (-> Unit Int64)))
  (def (lex (: b Bytes) (: i Int64) (: acc Int64))
    (match (Bytes.at b i)
      ((Some c) (lex b (+ i 1) (+ (* acc 10) (+ c (Tok.take)))))
      ((None _u) acc)))
  (def (main (: n Int64))
    (handle Tok 0
      ((take (u) s (resume s (+ s 1))))
      (lex (bin (u8 (UInt8.wrap 1)) (u8 (UInt8.wrap 2)) (u8 (UInt8.wrap 3))) 0 0)))
  (export main))
