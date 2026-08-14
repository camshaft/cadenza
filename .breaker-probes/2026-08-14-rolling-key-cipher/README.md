# vig1 — Vigenere-style rolling-key cipher (2026-08-14, tick 1477)

Two ops share ONE key stream: `encb` adds and `decb` subtracts the key byte
selected by the advancing key index (mod-3 cycle through a keyat def whose
middle byte is seed-shaped: (n%4)+1), both mod 26. Interleaved enc/dec draws
each consume a key position — the state is the INDEX, the key is recomputed
per dispatch through a literal-pattern match def (0/1/_ arms).

First draft used n%5+1 — seed-INVARIANT for n∈{10,0} (both ≡0 mod 5); caught
in modeling, re-keyed to n%4+1 (10%4=2 → key [3,3,7]; 0%4=0 → [3,1,7]).

The dec answer uses (+ (- b k) 26) % 26 to keep the dividend non-negative —
avoids relying on truncating-division sign semantics in the pin.

PASS ×3 wasm. **Pool (batch-275).**
