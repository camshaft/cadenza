# pr589 — printer.rs test comment self-contradicts on `(A)` nullary-list rendering

Mirrored from GitHub PR #589 review comment (Copilot), id 3608597313.
PR: https://github.com/camshaft/cadenza/pull/589 (13-MR publish batch)
Location: `implementation/seed/crates/cadenza-syntax/src/printer.rs:4112`

## Reviewer comment (verbatim)
> Test comment contradicts the expected behavior: this case preserves the nullary list variant `(A)`
> by rendering it as `A()` (not as bare `A`), so the comment should match what the assertions and
> printer now do.

## VERIFIED (git show trunk)
The test `nullary_variant_as_a_one_element_list_renders_as_a_type_decl_not_a_backtick_application`
has a two-paragraph doc comment that contradicts itself: the FIRST paragraph ends "Fix: accept a
1-elem list nullary + render `(A)` as the canonical bare `A`." — but the SECOND paragraph + the
assertion say the `()` is PRESERVED (renders `A()`, NOT bare `A`, because `(A)` 1-elem-list and `A`
atom are DISTINCT arenas and corpus_roundtrip requires EXACT round-trip). So the "render as bare `A`"
line is stale (describes an earlier/rejected design); the actual behavior is `(A)` → `A()`. Fix =
correct the first-paragraph line to match the preserved-`A()` behavior. Minor doc-comment accuracy,
no behavior change.

## Owner
`cadenza-syntax/src/printer.rs` = v-syntax (owns the ML printer/round-trip).
