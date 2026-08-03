(case "a 200-element runtime Set of BYTES resolves every member through a multi-level CHAMP over the blessed byte order"
  (doc    "Combines the just-landed Bytes total order with a SCALE CHAMP set — a gap the small-Bytes-set cases
           and the Int64 large-set case each miss individually. `build` inserts, for i in 0..n, a distinct
           2-byte Bytes value `[i/256, i%256]` (i in 0..200 → high byte 0, low byte i, all distinct) into a
           `Set Bytes`. At n=200 the CHAMP grows multi-level, and its hashing/eq/canonical-order run over the
           BYTES leaves (the new blessed unsigned-lex order the champ descent uses). `present` then re-derives
           each `[i/256, i%256]` and checks membership (+1 each); `absent` checks n..n+50 (values 200..249 →
           `[0, i]` for i>=200 needs 2 bytes low>255 — instead use high byte 1 so they are genuinely-absent
           distinct keys `[1, j]`) score a 1000 penalty if wrongly found. checksum = 200 (all present, none of
           the 50 absent found). A Bytes key lost/misrouted in the multi-level CHAMP, or an unsigned-order
           mismatch between insert and probe, breaks it. The Bytes-element companion of the Int64 large-set +
           the small-Bytes-set membership cases.")
  (input  (do
            (def (bk (: i Int64)) (Bytes.of (list (UInt8.wrap (/ i 256)) (UInt8.wrap (% i 256)))))
            (def (build (: i Int64) (: n Int64) (: s (Set Bytes)))
              (if (< i n) (build (+ i 1) n (Set.insert s (bk i))) s))
            (def (present (: i Int64) (: n Int64) (: s (Set Bytes)) (: acc Int64))
              (if (< i n)
                (present (+ i 1) n s (+ acc (if (Set.contains s (bk i)) 1 0)))
                acc))
            (def (absent (: j Int64) (: hi Int64) (: s (Set Bytes)) (: acc Int64))
              (if (< j hi)
                (absent (+ j 1) hi s (+ acc (if (Set.contains s (Bytes.of (list (UInt8.wrap 1) (UInt8.wrap (% j 256))))) 1000 0)))
                acc))
            (def (main (: n Int64))
              (let ((s (build 0 n (Set.of (list)))))
                (- (present 0 n s 0) (absent 0 50 s 0))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 200 Int64)))
