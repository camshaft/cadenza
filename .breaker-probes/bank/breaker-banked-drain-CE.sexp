(case "a runtime UInt8 param encodes into a u8 segment in-range by construction"
  (doc    "The runtime-OPERAND encode face at the type-guaranteed boundary: a `UInt8` PARAM feeds a
           `(bin (u8 x) (u8 7))` build — in-range BY CONSTRUCTION (the type has no out-of-range
           value), so the encode is total with no runtime range check needed. Read back through a
           u16 stitch: x=255 → 0xFF07 = 65287 (the max byte rides the high position intact — a
           signed-i8 mis-carrier would sign-extend and corrupt the stitch); x=0 → 7. The runtime
           companion of the const `(bin (u8 256))`→CDZ0304 reject: the type system makes the runtime
           face UNREACHABLE for out-of-range, and this pins the in-range encode running end-to-end.")
  (input  (do
            (def (main (: x UInt8))
              (match (bin (u8 x) (u8 7))
                ((bin (u16 n)) (Int64.of n))
                (_ -1)))
            (export main)))
  (call   main (: 255 UInt8)) (output (: 65287 Int64))
  (call   main (: 0 UInt8)) (output (: 7 Int64)))

(case "a runtime Int64 into a u8 segment is a type mismatch not a truncation"
  (doc    "The header's promised runtime face (:690's doc: 'a non-constant value of the wrong type is
           a CDZ0203 type error — see the runtime section' — promised but never pinned): a runtime
           `Int64` param in a `u8` segment position rejects CDZ0203 at compile time. It does NOT
           truncate to the low byte, and there is NO 'binary value does not fit' runtime trap on this
           path — the segment's operand type is `UInt8`, so the mismatch is total and static. Closes
           the loop with the in-range UInt8-param encode above: wrong TYPE rejects, right type is
           in-range by construction.")
  (input  (do
            (def (main (: x Int64)) (Bytes.len (bin (u8 x))))
            (export main)))
  (call   main (: 5 Int64))
  (error  CDZ0203))
