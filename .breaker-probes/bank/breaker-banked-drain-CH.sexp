(case "Rational projections trap on a value outside Int64 range via the checked narrowing"
  (doc    "The OVERFLOW face of the four exact integer projections (their sign/tie semantics are pinned
           at :212-:320): each is a DERIVATION ending in the checked `Int64.of` narrowing, so a
           rational whose integer part exceeds Int64 range must TRAP there — 2·Int64.max (built by
           exact Rational multiply, no intermediate overflow) traps through truncate, floor, ceil, AND
           round alike, while Int64.max/1 itself narrows cleanly (the in-range control). A projection
           that narrowed by bit-truncation would wrap to a small/negative integer instead; a derivation
           that lost the checked narrow on one of the four (e.g. round's tie-adjust path re-deriving
           differently) would split the quartet. Runtime k defeats folding.")
  (input  (do
            (def (big (: k Int64))
              (* (Rational.of 9223372036854775807 1) (Rational.of k 1)))
            (def (main (: k Int64) (: which Int64))
              (match which
                (0 (Rational.truncate (big k)))
                (1 (Rational.floor (big k)))
                (2 (Rational.ceil (big k)))
                (_ (Rational.round (big k)))))
            (export main)))
  (call   main (: 1 Int64) (: 0 Int64)) (output (: 9223372036854775807 Int64))
  (call   main (: 2 Int64) (: 0 Int64)) (trap "unreachable")
  (call   main (: 2 Int64) (: 1 Int64)) (trap "unreachable")
  (call   main (: 2 Int64) (: 2 Int64)) (trap "unreachable")
  (call   main (: 2 Int64) (: 3 Int64)) (trap "unreachable"))
