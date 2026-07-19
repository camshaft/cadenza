# PR review comment — mirrored from GitHub PR #414 (Copilot inline)

- **PR:** #414 "fleet: thirty-ninth batch (v-cad, duvet-check WAL, diagnostics, iterators, broad features)" (MERGED)
- **File:** `implementation/cad/src/examples.cdz:52` (`radial-pattern` / `radial-from`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591489059
- **Link:** https://github.com/camshaft/cadenza/pull/414#discussion_r3591489059

## Comment (verbatim)
> `radial-pattern` claims it returns `empty` for `n <= 0`, but for negative `n` it calls `radial-from` with `i` starting at 0 and a termination check `i == n`, which will never be reached (0,1,2,… will not equal a negative n) → non-termination.

## Liaison triage — CONFIRMED against trunk
Confirmed: `radial-pattern(n,…) = radial-from(n, 0, …)` and `radial-from` terminates on `if i == n`,
with `i` starting at 0 and incrementing (`i + 1`). For negative `n`, `i` (0,1,2,…) never equals `n` →
unbounded recursion. The docstring claims "empty for `n <= 0`" but the guard only stops at exact
equality from below. This is the SAME class as the pr412 `cube-row` bug (guards exact-0/equality, doc
says `<= 0`) — the CAD examples have a recurring negative-`n` non-termination pattern. FIX: guard
`i >= n` (or reject/clamp `n <= 0` up front). CAD territory (v-cad). Fix on `trunk`. Quote + link in
queue file.

<!-- RESOLVED 2026-07-15 (trunk 98d9414e1, PR #412/#414): cube-row + radial-pattern both guard n<=0→empty; regression tests pin (-1) terminate as empty. v-cad swept all G-series examples: no other count-toward-bound recursion (solid.cdz recursions are structural, immune). -->
