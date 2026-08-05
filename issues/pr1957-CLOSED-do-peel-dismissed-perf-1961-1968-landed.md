# PR #1957 review — rcdzc/src/effects.rs (v-effects) — MERGED — 1 FALSE-POSITIVE (dismiss) + 1 LOW perf

https://github.com/camshaft/cadenza/pull/1957 — MERGED (recursive-branch-perform decline — the HIGH
self-probed miscompile). Copilot 2 inline: the CORRECTNESS one is a verified FALSE POSITIVE; the perf one
is real-but-LOW.

## `operand_is_branch_performing_conditional` "only peels let, not do" (Copilot id 3709615241) — DISMISS, FALSE POSITIVE [VERIFIED]
> `operand_is_branch_performing_conditional` claims to peel `let`/`do` block wrappers … but it currently
> only peels `let` (`Resolved::Let`) and `Ref`. If an operand is `(do … (if …))`, the wrapper prevents
> the conditional from being recognized and the decline guard can miss the intended block-wrapped variant.

FALSE POSITIVE — verified against the resolver. `resolve_do` (resolve.rs:4537) returns
`Resolved::Ref { value: last }` — a `do` block RESOLVES TO a `Ref` to its last form. So the function's
`Resolved::Ref { value } => …recurse…` arm ALREADY peels `do` blocks; there is no separate `do` node to
match. Copilot assumed `do` is its own `Resolved` variant needing a dedicated arm, but `do`→`Ref` means
`(do … (if …))` as an operand resolves to `Ref{value = the (if …)}` and the `Ref` arm recurses straight
into the conditional. The sibling `block_wrapped_branch_performs` documents this exact fact ("A `do` block
resolves to a `Ref` at its last form"). The doc comment's "let/do" wording is accurate — the `Ref` arm IS
the `do` peel. No gap. DISMISS. (Same class as the #1948 Set.of miscount: a bot claim about a syntactic
form that resolves differently than assumed — verified against the resolver, not accepted at face value.)

## `branch_perform_coexists_with_reentrant_call` allocates a `Vec` per `Apply` node (Copilot id 3709615206) — efficiency [VERIFIED, LOW]
> `branch_perform_coexists_with_reentrant_call` allocates a new `Vec` for every `Apply` node
> (`once(head).chain(args)…collect()`), even though the checks can be done with iterators over `head` +
> `args`. This is on a compile-time AST walk and can become a noticeable hotspot for large bodies.

VERIFIED (effects.rs:5877): `let all: Vec<StructId> = std::iter::once(head).chain(args.iter().copied())
.collect();` then two `all.iter().any(...)` passes. The `Vec` is avoidable — each `.any()` can iterate
`std::iter::once(head).chain(args.iter().copied())` directly (the iterator is cheap to rebuild; `args` is
already a slice). Minor compile-time alloc per `Apply` on the effects AST walk. LOW/efficiency — not hot
unless bodies are large + this guard runs per-node. Fix: drop the `collect()`, inline the iterator in both
`.any()` calls (or bind a closure returning the iterator). v-effects owns effects.rs.
