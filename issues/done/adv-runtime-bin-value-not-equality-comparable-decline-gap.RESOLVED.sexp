; BREAKER FINDING — DECLINE GAP (reject-don't-miscompile, NOT a miscompile; both backends): a RUNTIME-
; constructed `(bin <segment>...)` value — which the spec states IS a Bytes value (16-binary-matching.sexp:4
; "expression position `(bin …)` CONSTRUCTS a Bytes value", :43 "builds a Bytes value") — cannot be `=`-
; compared. It DECLINES ("compiler can't compile it yet"), even with an explicit `Bytes` annotation, though
; a runtime `Bytes.of` value of the same content compares fine and a CONSTANT `(bin …)` compares fine.
;
; This is a completeness gap, not a soundness bug — the compiler DECLINES rather than producing a wrong
; value, and both backends are consistent on the core case. Filing so `=` over a runtime `bin` result can be
; closed; a runtime `bin` should be `=`-comparable exactly as any other Bytes value is.
;
; ISOLATED (recompute-before-crying-bug, one axis at a time):
;   d1  runtime (bin (u8 (UInt8.wrap v))) = (Bytes.of (list 5))            -> DECLINES (both backends)
;   d2  same but ANNOTATED (: (bin …) Bytes) = (Bytes.of …)               -> DECLINES (both) — annotation
;                                                                             does not help; not a type-infer gap
;   c1  CONTROL runtime (Bytes.of (list (UInt8.wrap v))) = (Bytes.of …)   -> WORKS (both) — runtime Bytes `=` fine
;   c2  CONTROL runtime bin through (Bytes.concat (bin …) (Bytes.of ())) = -> WORKS on wasm; DECLINES on rust
;   c3  CONTROL constant (bin (u8 5)) = (Bytes.of (list 5))               -> WORKS (both) — constant bin folds+cmp
;   c4  CONTROL runtime (Bytes.len (bin …))                                -> WORKS on wasm; DECLINES on rust
; So: the runtime `bin` RESULT is spec'd Bytes and behaves as Bytes under wasm's Bytes.len/Bytes.concat, but
; the `=` lowering declines on it directly (wasm), and on RUST a runtime `bin` value is more broadly
; unsupported (even Bytes.len/Bytes.concat decline). The common backend-agnostic gap is `=` over a runtime
; bin (d1/d2). No path produces a WRONG value — every failure is a clean decline.
;
; SUGGESTED FIX (v-patterns / bin-construction owner + v-inference): a runtime `(bin …)` construction result
; must ground to the SAME Bytes type/representation a `Bytes.of` produces, so `=` (and on rust, Bytes.len/
; Bytes.concat) accept it. The constant path (c3) already folds to a comparable Bytes; the runtime emit
; leaves the value in a form `=` does not recognize as Bytes. Route rust's broader Bytes.len/concat decline
; (c2/c4) to v-rust-backend. Confirm RESOLVED when d1/d2 compare true on both backends.
;
; The cases below assert the CORRECT results (all compare true / len 1). They DECLINE today; the controls
; c1/c3 pass. Flip to pass when a runtime bin result is `=`-comparable.

(case "adv runtime-bin-eq: a runtime-constructed bin value compares equal to the Bytes it builds"
  (doc "`(= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))` with v=5: the runtime bin builds the one-byte
        Bytes 0x05, equal to `(Bytes.of (list 5))` -> true. The spec says `(bin …)` CONSTRUCTS a Bytes
        value, so it must be `=`-comparable like any Bytes. Today it DECLINES on both backends (the runtime
        bin result is not recognized as a comparable Bytes), though a runtime `Bytes.of` of the same content
        compares fine and a constant bin compares fine. Should return true.")
  (input (do (def (main (: v Int64)) (= (bin (u8 (UInt8.wrap v))) (Bytes.of (list 5)))) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case "adv runtime-bin-eq: an explicit Bytes annotation does not make it comparable (still declines)"
  (doc "The annotated companion: `(= (: (bin (u8 (UInt8.wrap v))) Bytes) (Bytes.of (list 5)))` — asserting
        the runtime bin's Bytes type explicitly — STILL declines. Pins the gap is not type inference failing
        to see it as Bytes (the annotation confirms Bytes) but the `=` lowering declining on the runtime bin
        representation. Should return true.")
  (input (do (def (main (: v Int64)) (= (: (bin (u8 (UInt8.wrap v))) Bytes) (Bytes.of (list 5)))) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case "adv runtime-bin-eq CONTROL: a runtime Bytes.of value IS equality-comparable"
  (doc "The control that PASSES: `(= (Bytes.of (list (UInt8.wrap v))) (Bytes.of (list 5)))` with v=5 -> true.
        Runtime Bytes `=` works; pins the gap is SPECIFIC to the runtime `bin` construction result, not to
        runtime Bytes equality in general.")
  (input (do (def (main (: v Int64)) (= (Bytes.of (list (UInt8.wrap v))) (Bytes.of (list 5)))) (export main)))
  (call main (: 5 Int64))
  (output (: true Bool)))

(case "adv runtime-bin-eq CONTROL: a CONSTANT bin value IS equality-comparable"
  (doc "The constant control that PASSES: `(= (bin (u8 5)) (Bytes.of (list 5)))` -> true. A constant bin
        folds to a comparable Bytes; pins the gap is the RUNTIME bin emit specifically, not bin `=` in
        general — the runtime path leaves the value in a form `=` does not accept as Bytes.")
  (input (= (bin (u8 5)) (Bytes.of (list 5))))
  (output (: true Bool)))
