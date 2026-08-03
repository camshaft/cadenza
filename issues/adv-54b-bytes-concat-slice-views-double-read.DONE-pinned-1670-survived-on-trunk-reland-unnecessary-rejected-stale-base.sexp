; adv-54b (v-runtime probe, 2026-08-02) — NEW WASM-ONLY SOUNDNESS MISCOMPILE, same family as adv-54.
; A `let`-bound `Bytes.concat` of two `String.to-bytes(String.slice …)` VIEWS, read MORE THAN ONCE via
; `Bytes.at`, traps OUT-OF-BOUNDS on wasm — rust/rust-async compute the correct value.
;
; ROOT (same mechanism as adv-54): `Core::BytesConcat` is NOT in `is_runtime_computation` (lower.rs
; ~13564), so the let-bound `b` is COPY-PROPAGATED — the concat (and its sliced-view to-bytes operands)
; is RECOMPUTED at each `Bytes.at` site rather than named once. Each recompute CONSUMES the borrowed
; slice-view sources, so the 2nd read walks a freed/advanced buffer → OOB trap. adv-54 fixed exactly this
; for StrSlice/StrToBytes; BytesConcat is the next op in the same family.
;
; SHRINK (v-runtime):
;   ONE read  of Bytes.concat(view, view)         -> OK both backends (100)
;   TWO reads of Bytes.concat(view, view)         -> wasm OOB TRAP; rust 200  <- BUG (wasm)
;   concat of OWNED (non-view) to-bytes, 2 reads   -> OK both backends (the recompute of an owned
;                                                     source is idempotent; only a VIEW source leaks)
; Trigger = Bytes.concat whose OPERANDS are slice-view to-bytes + the concat RESULT read > once.
;
; ⚠ FIX IS NOT the naive "add BytesConcat to is_runtime_computation" — v-runtime's adv-54 scoping found
; that forcing the wider Bytes/List/Map/Set/BigInt heap-op family kept REGRESSED 3 cases (bin-parse
; invalid-component + rope-keyed-map bytes-compact): their KEPT-binding EMIT has its own latent bug that
; must be fixed FIRST (a non-Copy heap value read >1× must clone-on-read, matching the Symbol/String arm).
; v-rust-backend OWNS this wider kept-binding-family follow-up (acked to v-runtime 2026-08-02). This
; reproducer is the wasm-side witness for that work.
;
; Severity HIGH: silent OOB trap on the DEFAULT (wasm) backend for an ordinary "concat two substrings and
; read the result more than once" shape. Graded against the CORRECT (rust) value; red on wasm until fixed.

(case "Bytes.concat of two sliced-view to-bytes is read twice without an OOB trap (wasm-only miscompile)"
  (doc    "adv-54b: a let-bound Bytes.concat of two String.to-bytes(String.slice …) VIEWS, read more than
           once, must see the concatenated bytes on every read. `s` = 'ab'++'cdé' (runtime, opaque to the
           fold); `tail` = slice(s,3,5) = 'dé' = bytes [100,0xC3,0xA9]; `b` = concat(to-bytes tail,
           to-bytes tail) = [100,0xC3,0xA9,100,0xC3,0xA9]; `b[0]+b[3]` = 100+100 = 200. Wasm TRAPS OOB
           (the concat is copy-propagated + recomputed, consuming the borrowed view sources on the 1st
           read); rust/rust-async compute 200. Same family as adv-54; BytesConcat needs the kept-binding
           treatment (v-rust-backend owns the wider fix).")
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail)
                    (let ((b (Bytes.concat (String.to-bytes tail) (String.to-bytes tail))))
                      (+ (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                         (Int64.of (Option.expect (Bytes.at b 3) "b3")))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 200 Int64)))
