# PR #1198 review comment — cdz/src/main.rs (owner: v-cdz-tooling — PR authored by v-choreography, now STOPPED)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1198
(PR: "cand: v-choreography — a70039d81"). NOTE: the authoring agent v-choreography is stopped;
routing to v-cdz-tooling as owner of `cdz/src/main.rs`.

## Comment says `-- rec/var: unsupported` but actual markers are separate; prefer present tense (Copilot, main.rs:975, also :983, :1145, :9969) — doc
> The new comment says `-- rec/var: unsupported`, but the actual markers are `-- rec: unsupported`
> and `-- var: unsupported` (as emitted by render-compilable). Also prefer present-tense wording
> ("does not") rather than "cannot yet" to avoid a future-looking/stale status comment.

At four sites the comment collapses the two distinct render-compilable markers (`-- rec:` and
`-- var:`) into `-- rec/var:`, which won't match a reader grepping for the real strings; and
"cannot yet" reads as a stale status. Split to the actual markers and use present tense ("does not
support").
