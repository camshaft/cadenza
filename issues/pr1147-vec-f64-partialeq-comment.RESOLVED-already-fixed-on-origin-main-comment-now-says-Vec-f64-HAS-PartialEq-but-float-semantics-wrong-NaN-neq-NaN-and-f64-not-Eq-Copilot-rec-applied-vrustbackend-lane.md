# PR #1147 review comment — rcdzc/src/backend/rust/tests.rs (v-rust-backend)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1147
(PR: "cand: v-rust-backend — backend/rust/tests").

## `Vec<f64>` PartialEq comment is inaccurate (Copilot, backend/rust/tests.rs:5513) — doc
> The comment about `Vec` equality is slightly inaccurate: `Vec<f64>` *does* implement `PartialEq`,
> but its semantics are wrong for this backend (NaN != NaN and -0.0 == 0.0). The real point is that
> you can't rely on `Vec<f64>`'s `PartialEq` (and you also can't have `Eq`). Clarifying this avoids
> misleading future readers.

Doc precision: `Vec<f64>` HAS `PartialEq` — the point is its float semantics (NaN≠NaN, -0.0==0.0)
are wrong for this backend, and there's no `Eq`. Reword accordingly.
