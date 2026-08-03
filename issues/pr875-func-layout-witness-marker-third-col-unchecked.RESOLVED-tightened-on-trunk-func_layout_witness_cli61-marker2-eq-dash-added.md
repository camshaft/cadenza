# PR#875 review comment — func_layout_witness_cli marker validation skips 3rd column (v-cdz-tooling)

Mirrored from GitHub PR#875 review comment (Copilot), id `3663428341`.
File: `implementation/seed/crates/cdz/tests/func_layout_witness_cli.rs:60` — `cdz` crate test →
v-cdz-tooling's lane (per the PR#867 routing precedent: they own this exact compile-reuse witness file).

## Comment (verbatim)

- (id 3663428341, func_layout_witness_cli.rs:60) "The marker validation claims the first line must be
  `defs-begin<TAB><import-base><TAB>-`, but the assertion only checks `defs-begin` and that the second
  column parses as u32. This would allow unexpected third-column values to slip through while still
  passing validation, contradicting the intended format check."

## Liaison verification (confirmed on trunk f63ad16b8)

Line 57-59:
```
assert!(
    marker.len() == 3 && marker[0] == "defs-begin" && marker[1].parse::<u32>().is_ok(),
    "first line must be the `defs-begin<TAB><import-base><TAB>-` marker, got {first:?}\nfull:\n{text}"
);
```
The assert enforces exactly 3 cols, col0 == "defs-begin", col1 parses as u32 — but never checks
`marker[2] == "-"`. The error message asserts the format is `defs-begin<TAB><import-base><TAB>-`, so a
row like `defs-begin\t5\tXYZ` would pass while contradicting the stated format. Add `&& marker[2] == "-"`
to the assert. Test-strictness only, no runtime defect (behavior-neutral; the CLI does emit "-" today).

Owner: **v-cdz-tooling** (`cdz/tests/*` — the func-layout compile-reuse witness; they own this file per
PR#867). One-line assert tightening.
