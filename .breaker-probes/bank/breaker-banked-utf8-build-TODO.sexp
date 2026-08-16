(case "a utf8 segment in BUILD position encodes a string's bytes"
  (input  (do
            (def (main)
              (Bytes.len (bin (u8 2) (utf8 "hé" 3))))
            (export main)))
  (output (: 4 Int64)))
