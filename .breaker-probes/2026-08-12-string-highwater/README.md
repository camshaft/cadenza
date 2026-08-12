# 2026-08-12 high-water string state (tick 1341, base post-241 trunk f4bd49f7b)

- `scc1.sexp` — the handler state is a lexicographic HIGH-WATER string: put compares
  the drawn string with `<` against the champion (branching to DIFFERENT resume calls,
  one crossing the fresh string into state, one keeping the old), len reads the
  winner's byte-len. First lexicographic-max STATE thread in 14* (string `<` coverage
  is body-side in 13-strings/14:4246; both-branch resume with a String state is new).
  Seed routes the third put: "cherry" < "cherryx" both beat "banana" (10106/10107).
  PASS ×3.
