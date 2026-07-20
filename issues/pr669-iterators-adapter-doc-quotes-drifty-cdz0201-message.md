# pr669 — iterators adapter.cdz doc hard-quotes a CDZ0201 message that may drift / not match (Copilot, dup on #669+#670)

Mirrored from GitHub PR #669 review comment (Copilot) id 3613577735 (= PR#670 dup id 3613607681, same locus).
PR: https://github.com/camshaft/cadenza/pull/669 (iterators adapter consumers)
Location: `implementation/iterators/src/adapter.cdz:128`

## Reviewer comment (verbatim, #669)
> The comment quotes CDZ0201 as "const arg not compile-time-known", but the compiler's diagnostic text for
> this case is different (it mentions a `const` parameter and runtime data). Since this is a doc comment
> meant to explain the workaround, it should either match the real wording or avoid quoting an exact message.
(The #670 dup adds: the message is "likely to drift" — describe the condition instead of quoting it.)

## VERIFIED (git show trunk)
adapter.cdz:126-127: the `sum` def comment explains why it drives `drive` directly: "...the `sum(from-list(…))`
call site would be rejected CDZ0201 `const arg not compile-time-known`." That hard-quotes a CDZ0201 message
in a doc comment — brittle if the diagnostic wording drifts (and Copilot says it doesn't match the real text,
which mentions a const parameter + runtime data). Fix (Copilot): describe the CONDITION ("rejected because a
const parameter argument must be compile-time-known"), don't quote an exact message. Doc-only durability nit.

## Owner
`implementation/iterators/*` = v-iterators. (One finding; also flagged on PR#670 as id 3613607681 — same locus.)
