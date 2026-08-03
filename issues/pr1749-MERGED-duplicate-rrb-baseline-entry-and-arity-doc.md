# PR #1749 review comments — spec/semantics (corpus-bugfix) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1749 (MERGED).

## 1. Duplicate baseline entry: "concatenating two large runtime RRB vectors…" now at 462 AND 4320 (Copilot, all 3 baselines) — corpus-hygiene [VERIFIED]
> This case already appears earlier (line 461/462), so adding it again introduces a duplicate baseline
> entry. The gate baseline should list each case name exactly once.
VERIFIED on trunk: the case appears TWICE in all 3 baselines (.gate-baseline / -rust / -rust-async, at
:462 and :4320). Content-keyed matching union-dedups so it's benign for the gate, but it's corpus-hygiene
noise + the duplicated .sexp case (if any) is wasted. Remove the second copy. LOW.

## 2. `arity->1` reads like a function arrow (Copilot, 07-type-system.sexp:225) — doc
> `arity->1` reads like a function arrow rather than "arity greater than 1". Reword (e.g. "arity > 1").
LOW/doc. Fold both into the next corpus edit per the no-standalone-polish steer.
