# PR #1114 review comments — guide/src/calculator/mixed.ts + mixed.test.ts (v-guide-editor)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1114
(PR: "cand: v-guide-editor — calculator mixed").

## `toMixed` throws on "1/0" (divide-by-zero) — should be total (Copilot, mixed.ts:39 + test at mixed.test.ts:43) — correctness
> [mixed.ts:39] `toMixed` will throw on inputs like "1/0" because `BARE_RATIONAL` matches it and
> `n / d` divides by zero. Even if the compiler should never emit a zero denominator, this function
> is exported and is also imported from scripts/tests, so it's worth making it total and consistent
> with the comment that the denominator is positive.
> [mixed.test.ts:43] Add a unit test case to ensure invalid-but-matching inputs like "1/0" don't
> crash (division by zero) and instead pass through unchanged.

Real robustness point: `BARE_RATIONAL` matches `"1/0"`, so `n / d` divides by zero and throws.
Since `toMixed` is exported + imported from scripts/tests, make it total — guard `d === 0` and pass
the input through unchanged (matching the "denominator is positive" comment) — and add the `"1/0"`
regression test.
