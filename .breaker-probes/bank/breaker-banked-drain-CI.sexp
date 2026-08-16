(case "an UNSIZED bytes segment mid-form is rejected as ill-formed"
  (doc    "A build-position `(bytes b)` with NO size occurrence is the open-tail splice — well-formed
           only as the FINAL segment. Mid-form — `(bin (u8 1) (bytes mid) (u8 9))` — the following
           `u8` has no static offset (the spliced length is runtime data), so the form is rejected
           CDZ0220 (ill-formed binary form) at compile time on all targets, uniformly. Pins the
           mid-form face of the unsized-bytes rule the header describes (:605 pins bit-misalignment;
           the unsized-mid-splice face was unpinned) — a lowering that accepted it would need a
           runtime offset chain the vocabulary deliberately avoids.")
  (input  (do
            (def (main (: x UInt8))
              (Bytes.len (bin (u8 1) (bytes (Bytes.of (list x 20))) (u8 9))))
            (export main)))
  (call   main (: 7 UInt8))
  (error  CDZ0220))
