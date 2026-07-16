# PR review comment — mirrored from GitHub PR #412 (Copilot inline)

- **PR:** #412 "fleet: thirty-seventh batch (Ast.Float leaf recovery, watchdog fix, broad features + orphan reconcile)" (MERGED)
- **File:** `implementation/cad/src/examples.cdz:43` (`cube-row`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591416002
- **Link:** https://github.com/camshaft/cadenza/pull/412#discussion_r3591416002

## Comment (verbatim)
> `cube-row` claims it returns `empty` for `n <= 0`, but the implementation only checks `n == 0`. For negative `n` this will recurse forever (`n - 1` moves farther from 0).

## Liaison triage — CONFIRMED against trunk
Confirmed in cad/src/examples.cdz: the docstring says "Returns `empty` for `n <= 0`", but the body is
`if n == 0 then empty() else union(cube-uniform(1.0), translate(…, cube-row(n - 1, step)))`. For a
negative `n`, `n == 0` is false and `n - 1` moves FARTHER from 0 → unbounded recursion (stack overflow /
non-termination). Real bug in a CAD example (part of the rsolid-port vertical). FIX: guard with
`n <= 0` (matching the doc) instead of `n == 0`. CAD territory (v-cad owns implementation/cad). Fix on
`trunk`. Quote + link in queue file.

<!-- RESOLVED 2026-07-15 (trunk 98d9414e1, PR #412/#414): cube-row + radial-pattern both guard n<=0→empty; regression tests pin (-1) terminate as empty. v-cad swept all G-series examples: no other count-toward-bound recursion (solid.cdz recursions are structural, immune). -->
