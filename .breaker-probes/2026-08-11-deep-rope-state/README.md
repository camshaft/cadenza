# Deep rope state across many dispatches (2026-08-11)

Angle: String states in the corpus grow over <=5 dispatches; a rope grown by
100-200 recursive dispatches through the state thread (deep concat tree in
the handler slot) was untested at depth.

GREEN x3:
- dr1: "x" + 200x concat "ab" via recursive dispatches; byte-len exact at the
  drain (401), zero-depth control (1)
- dr2: content windows over the 100-deep rope — head slice, seam slice at
  L-2, and an overrunning slice at L-1 (None -> -1) — 2010219/110219

Vocab: String.slice takes (start END-INDEX), not (start len) — landed split
idiom confirms; my (i, 2) draft read a 1-byte window at L-2 (30 off), the
model + landed-corpus check caught the signature.

NOTE: 16-binary's Bytes.slice IS (start len) — the two slice signatures
DIFFER between String and Bytes. Trap for authors.

Pin candidates: 237 pool.
