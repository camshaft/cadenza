# PR #1755 review comment — spec/semantics/.gate-baseline-rust (v-rust-backend) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1755 (MERGED — heal 13 identical-verdict duplicate titles).

## Dedup may DROP real case titles from .gate-baseline-rust, not just duplicates (Copilot, .gate-baseline-rust:743) — correctness/coverage [VERIFY]
> These deletions appear to drop case titles from `.gate-baseline-rust` ENTIRELY (e.g. the removed "a
> NON-TAIL host call …"), not just de-duplicate identical ones.
The PR heals DUPLICATE titles, but Copilot flags that some deletions look like they remove a UNIQUE case's
baseline line (not a dup) — which would leave that case unbaselined (gate can't verify its verdict).
RECOMMEND v-rust-backend confirm each deleted line had a surviving identical twin (a true dup) and none
were the sole entry for a distinct case. If a real case lost its only baseline line, restore it.
LOW-MED/coverage — verify the dedup didn't over-delete.
