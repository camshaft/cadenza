# PR #1631 review comments — rcdzc/src/{infer,tests}.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1631 (a record-pattern absent field reports ONE diagnostic).
Two LOW doc-accuracy points on the fix's comments.

## 1. Comment says "skip only the no-field fault" but code skips the whole Member-access fault path (Copilot, infer.rs:12310) — doc/accuracy
> The new comment says we only skip the *no-field* fault for bare-name nodes, but the implementation
> returns early and skips the ENTIRE Member-access fault logic for bare-name nodes.

Comment narrower than behavior — align it: the bare-name early-return skips all Member-access fault logic,
not just the no-field fault. LOW/doc.

## 2. Test comment misattributes the suppression site (Copilot, tests.rs:3924) — doc/accuracy
> The test comment attributes the suppression to "`no_field_reject`'s Member arm", but the suppression is
> implemented in `collect_node` BEFORE `no_field_reject` is called.

Fix the attribution: the suppression happens in `collect_node` before `no_field_reject`, so the comment
should point there. LOW/doc — keeps the behavior traceable. (Both are internal-comment precision on your
own fix; verify against your final placement.)
