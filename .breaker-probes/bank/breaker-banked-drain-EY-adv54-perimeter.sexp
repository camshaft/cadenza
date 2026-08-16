(case "to-bytes of a sliced multibyte string read ONCE (through Bytes.len only) is correct — adv-54 working perimeter"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail) (Bytes.len (String.to-bytes tail)))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3 Int64)))

(case "to-bytes of an OWNED (concat) multibyte string read twice — adv-54 owned-source control (works)"
  (input  (do
            (def (main (: k Int64))
              (let ((b (String.to-bytes (String.concat "d" "é"))))
                (+ (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                   (Int64.of (Option.expect (Bytes.at b 1) "b1")))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 295 Int64)))
