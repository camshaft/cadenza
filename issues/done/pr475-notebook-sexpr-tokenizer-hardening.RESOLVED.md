# PR #475 (merged, batch 104) — notebook sexpr.ts: escaped-quote, unterminated-string, and recursion-vs-doc

Mirrored from Copilot inline review on merged PR #475 (4 clustered comments). Confirmed on trunk.
Owner: **v-notebook** (`guide/src/notebook/sexpr.ts` + `sexpr.test.ts`).

## 1. Escaped-quote check is wrong for even backslash runs (comment 3595530810, line 44)
> Inside quoted strings, the tokenizer treats any quote preceded by `\` as escaped. This is incorrect
> when the quote is preceded by an even number of backslashes (e.g. a string ending in an escaped
> backslash `\\`), where the quote actually terminates the string.

Trunk: `if (c === '"' && text[i - 1] !== "\\")` — only looks one char back, so `"\\"` (an escaped
backslash then closing quote) is mis-read as an escaped quote and the string never closes. Real.

## 2. Unterminated string literal is silently accepted (comment 3595530856, line 61)
> If the input ends while still inside a quoted string, `tokenize` currently just flushes the partial
> string atom and parsing succeeds. Unterminated string literals are accepted rather than rejected.

Trunk: end-of-input `flush()` with no `inStr` check. Should throw (the caller renders a fallback on
throw). Real.

## 3. Doc claims "explicit stack, can't overflow" but parser is recursive descent (comment 3595530877, line 66)
> The doc comment says parsing is done via an explicit stack and cannot overflow, but `parseSexpr`
> uses recursive descent (`parseNode` calls itself). This can still overflow the JS call stack.

Trunk lines 31-32 doc: "Bounded recursion via an explicit stack — a deeply nested value can't
overflow." Line 37 `const parseNode = (): Node =>` is recursive descent. Either convert to a real
explicit stack or fix the doc to admit the recursion + add a depth guard. Real.

## 4. Test coverage gap (comment 3595530955, sexpr.test.ts line 55)
> Tests don't cover (1) unterminated string literals, (2) a quoted string ending with an escaped
> backslash (`\\`), and the deep-nesting path.

Add cases for #1/#2/#3 once fixed.

## Suggested fix
One tokenizer-hardening pass: count trailing backslashes for the escape decision, reject `inStr` at
EOF, and reconcile the recursion doc (explicit stack or documented depth cap) — with the three test
cases from #4.

PR: https://github.com/camshaft/cadenza/pull/475
