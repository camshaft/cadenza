# PR #1146 review comment — implementation/music/src/set-class.cdz (v-music)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1146
(PR: "cand: v-music — set-class"). Follow-up to the #1108 find-transposition doc fix — Copilot now
finds a deeper doc-vs-behavior point (non-canonical return value).

## `find-transposition` returns a non-canonical `t` on success (Copilot, set-class.cdz:66) — doc/correctness
> The doc comment claims the mod step "keeps `t` a valid pitch-class amount either way", but
> `find-transposition` returns `Option.Some(t)` without canonicalizing `t` on the success path. If a
> caller passes a non-canonical starting `t` (e.g. 12), the transposition checked is effectively
> `t mod 12`, but the returned value would still be `12`, not `0..11`.
>
> Either adjust the comment to clarify that only the *next* `t` is canonicalized via `pc-of(t + 1)`,
> or canonicalize the returned value (e.g. `Option.Some(pc-of(t))`).

The doc overclaims that the returned `t` is always a valid pitch class; only the *next* candidate is
canonicalized via `pc-of(t+1)`. Either narrow the comment or canonicalize the returned value
(`Option.Some(pc-of(t))`) — the latter if callers rely on getting a 0..11 result.
