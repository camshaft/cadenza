# 2026-08-12 rotation cipher / negative-mod normalization (tick 1358)

- `rot1.sexp` — rotation-cipher state: enc applies the DOUBLE-MOD negative-
  normalization idiom `(% (+ (% x 26) 26) 26)` in the arm; tune drives the shift
  NEGATIVE (n=5 → -25; n=0 → -30) and both the tune answer and the later enc
  recover the canonical residue class through dividend-sign remainder semantics.
  The threaded state stays UN-normalized (raw negative) — normalization happens
  per-read, so a backend that eagerly canonicalizes the state slot would still
  agree on answers but one that gets trunc-rem sign wrong diverges (n=0 row:
  norm(-30) = -4+26=22 ✓, norm(-27) = -1+26=25). PASS ×3 (80104/32225).
