# PR #2034 review — flake.nix (v-nix) — MERGED — robustness + a typo-churn note

https://github.com/camshaft/cadenza/pull/2034 (full-CI-in-nix increment 6c — gate). Copilot 2 inline: a
real indentation deviation + a typo suggestion that CONFLICTS with an earlier Copilot suggestion.

## `runtime.toml` written via `<<EOF` (indent-preserving) heredoc → the `runtime =` lines carry leading whitespace, deviating from `cargo xtask build`'s format; a consumer anchoring `^runtime =` would fail hash extraction (Copilot, flake.nix:829) — robustness [VERIFIED cause; consumer-break = Copilot's claim, not fully confirmed]
> `runtime.toml` is written via a here-doc with leading indentation, which will become part of the file
> contents. Some consumers parse this file with `^runtime =` (e.g. `.github/workflows/build.yml`), so
> leading whitespace can cause runtime hash extraction to fail. Write the manifest without leading
> whitespace to match `cargo xtask build`'s output format.

VERIFIED the CAUSE: the heredoc is `cat > "$out/runtime.toml" <<EOF` (plain `<<EOF`, NOT `<<-EOF`), and the
content lines are indented ~10 spaces in flake.nix (`cat -A` confirms `          runtime = "$rt"$`). Plain
`<<EOF` preserves leading whitespace, so the emitted `runtime.toml` contains `          runtime = "…"` —
which does NOT match `cargo xtask build`'s (presumably column-0) output format. That deviation is real.
The CONSUMER-BREAKAGE half (a `^runtime =` anchored grep failing) is Copilot's claim — I could not locate
the exact `.github/workflows/build.yml` parse from this worktree to confirm it anchors `^`, so I'm relaying
it as plausible-not-confirmed (per the native-build calibration lesson: assert only what I verified). But
the fix is cheap + strictly-safer regardless: either use `<<-EOF` with TAB indentation (strips leading
tabs) or, cleaner, dedent the heredoc body to column 0 so the file matches `cargo xtask build`'s format
exactly. MED-ish (a hash-extraction failure would red the build/store gate; scoped to whoever reads this
nix-emitted runtime.toml). Fix-forward (merged). v-nix — please confirm the consumer parse (anchored vs
`trim`-tolerant): if anchored, this is a real break; if it trims, it's format-hygiene. Either way, dedent.

## `redding` → Copilot now says `reddening` (flake.nix:458) — typo-churn, v-nix's call [NOTE, not re-filing]
> The phrasing "redding this check" reads like a typo… "reddening" is the standard form…
NOTE: this CONFLICTS with an earlier Copilot comment (#2025 id 3712364833) that asked to change "reding" →
"redding". Now a third form ("reddening") is proposed. This is cosmetic typo-churn on the same word across
review rounds — NOT re-filing as a separate item; v-nix already has the #2025 typo fix folding into inc6d
and can pick the final spelling ("reddening" or just reword to "turning this check red" to end the churn).
Flagging only so v-nix knows the two Copilot suggestions disagree; no action needed beyond the inc6d fold.
