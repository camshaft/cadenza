# PR#781 review comment — print_bin (b[...]) missing the has_nonlast_comment_after guard; non-last comment-after breaks round-trip

Mirrored from GitHub PR review comment (Copilot), id `3629859027`.
PR: https://github.com/camshaft/cadenza/pull/781 (batch-staging; fix belongs on trunk)
Location: `implementation/seed/crates/cadenza-syntax/src/printer.rs:2213` (`print_bin`), dispatch at ~694.

## Comment (verbatim)

> `print_bin` now uses `bracketed_comment_aware`, which unwraps `(comment-after ...)` to `seg // text`.
> If a `(comment-after ...)` appears on a non-last bin segment (possible for decoded /
> metaprogramming-built ASTs), the next segment's leading `,` gets swallowed by the `//` comment,
> producing invalid output. Other literal printers guard this via `has_nonlast_comment_after(...)`;
> `print_bin` should do the same and fall back to the generic `bin(...)` call form in that case to
> preserve round-tripping.

## Liaison verification (CONFIRMED on trunk — a real gap the PR#763 fix MISSED)

The PR#763 printer-totality fix (`15b4a5072`, IS on trunk) added `has_nonlast_comment_after` guards at
the record/map/list/tuple/set DISPATCH sites (printer.rs lines ~352 `inline_ok`, ~733, ~3260, ~3487).
But `print_bin` was NOT covered:
- Dispatch (printer.rs ~694): `"bin" => return self.print_bin(args)` — UNGUARDED (no `inline_ok` /
  `has_nonlast_comment_after` check, unlike the `list`/`tuple` dispatch at ~350).
- `print_bin` (~2208-2213) calls `self.bracketed_comment_aware("b[", "]", false, segs)` directly.
- `bracketed_comment_aware` (~2307) only checks `last_has_trailing` (the LAST element); it does NOT
  call `has_nonlast_comment_after`, so a non-last `comment-after` segment still sugars → the `,`
  separator lands after the `//` → swallowed → invalid re-parse.

So `print_bin`'s own doc-comment claim ("A non-last `comment-after` (decoded AST only) declines the
sugar → generic call form") is currently FALSE — nothing declines it. Same PR#758/#763 class
(printer must be total over decoded/metaprogramming ASTs).

Also worth checking: the OTHER `bracketed_comment_aware` callers — `#(` set (~428) and list (~2297) /
tuple (~2381) — ARE reached through guarded dispatch (the ~350 `inline_ok` gate), so they're covered;
`print_bin` is the one that dispatches UNguarded. (v-syntax: confirm the set `#(` path at 428 is also
guarded at its dispatch, since it shares bracketed_comment_aware.)

Fix (per Copilot): guard the `bin` dispatch (or inside `print_bin`) on `has_nonlast_comment_after(segs)`
→ fall back to the generic `bin(...)` call form; OR (cleaner, fixes all callers at once) make
`bracketed_comment_aware` itself decline to the generic form when `has_nonlast_comment_after(elems)`.
Add a decoded-AST regression test: `(bin (comment-after "x" seg0) seg1)` must round-trip via the call
form. Owner: v-syntax (`printer.rs`; PR#763 comment-preservation series). Routed as a note.
