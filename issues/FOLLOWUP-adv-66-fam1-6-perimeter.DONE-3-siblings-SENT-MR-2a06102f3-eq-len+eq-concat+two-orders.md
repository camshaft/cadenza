# FOLLOW-UP: adv-66 fam1-6 perimeter pins (v-wasm-opt requested)

v-wasm-opt explicitly asked for breaker's fam1-6 perimeter siblings as a follow-up pin after the base
adv-66 pin (aef86b1b9) lands — to guard the OVER-ROTATION direction (a future dup-pass change that
over-consumes a borrow, or mis-widens the consume set, would flip one of the 5 currently-passing
siblings). "Pin an edge even if it already passes" discipline; cheap durable coverage for the
BytesCompact borrow→consume seam boundary.

The 5 passing siblings (breaker fam1-6, all currently PASS on trunk — they must STAY pass):
- concat-result double-read (→ 11)
- slice-VIEW double-read (→ 11)
- List.concat 3-way read (→ 1073)
- compact + TWO order-compares, no eq (→ 11)
- compact eq-then-LEN (→ 101)
- compact eq-then-CONCAT+len WITHOUT the rope-on-left order-compare (→ 111)
Source: breaker probes fam1-6; the .breaker-probes/2026-08-03-h8 / bc-family scratch. RE-DERIVE + gate
each on all 3 backends before pinning (some may decline on rust/rust-async — record per-backend verdict,
todo where declined). ACTION when base MR lands + capacity: add these to 10-bytes near the base adv-66
pin as an "over-rotation perimeter" block, own small MR. No rush (v-wasm-opt: base first, perimeter follow-up).
