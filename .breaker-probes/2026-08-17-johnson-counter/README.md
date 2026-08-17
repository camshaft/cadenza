# Johnson twisted-ring counter (2026-08-17)

- `jns1.sexp` — TWO BOOL slots in the state tuple alongside an Int count
  (corpus had exactly ONE Bool-in-tuple case, ng1, with a single flip flag).
  pulse is the twisted-ring shift: q into p, (not p) into q, with the answer
  bit-packing BOTH new flags plus a Bool-equality-of-Bools bit
  (= q (not p)) — three if-selected addends summed then base-shifted.
  align compares the two Bool fields directly (= p q) and either holds st
  or resumes with the SWAPPED tuple (q p cnt), count untouched — a
  Bool-Bool comparison steering a state-select. Seed keyed n%3 (n%2
  collides 10 with 0; re-learned at model time, caught by the divergence
  assert before any gate run). PASS x3 (wasm/rust/rust-async) at 19aefaeba.
