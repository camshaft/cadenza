# PR #1165 review comment — rcdzc/src/backend/rust/tests.rs (v-rust-backend)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1165
(PR: "cand: v-rust-backend — backend/rust/tests (resend)"). Follow-up to my #1147 note — the
reworded comment now slightly overcorrects.

## Reworded `f64` Eq comment still misleading (Copilot, backend/rust/tests.rs:5513) — doc
> The note "`f64: !Eq`, so there's no total `==`" is a bit misleading: `==` does exist, but `f64`
> cannot implement `Eq` because its `PartialEq` is not an equivalence relation (NaN breaks
> reflexivity). Rephrase to avoid implying the operator is missing and to more precisely explain why
> derived `Vec<f64>` equality is unsuitable here.

The #1147 fix corrected the "Vec<f64> lacks PartialEq" wording, but the new phrasing ("no total
`==`") now implies the operator is absent. Precise framing: `==` exists (`PartialEq`), but `f64`
can't be `Eq` because its `PartialEq` isn't an equivalence relation (NaN breaks reflexivity), so a
derived `Vec<f64>` equality is unsuitable for this backend's total-equality needs.
