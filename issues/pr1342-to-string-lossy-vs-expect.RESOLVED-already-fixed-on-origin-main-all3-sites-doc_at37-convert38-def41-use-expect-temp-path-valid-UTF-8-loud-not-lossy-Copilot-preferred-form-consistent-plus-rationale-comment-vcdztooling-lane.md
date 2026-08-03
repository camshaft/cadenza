# PR #1342 review comments — cdz/tests/{doc_at,convert,def}_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1342 (PR: "cand: v-cdz-tooling — c4cfdfea4").
This is the follow-up that applied `to_string_lossy()` across the harness (from my #1291/#1248
`.unwrap()` filings) — Copilot now pushes back the OTHER direction, same comment at 3 sites
(doc_at_cli.rs:31, convert_cli.rs:29, def_cli.rs:35).

## `to_string_lossy()` can silently corrupt a non-UTF-8 path (Copilot, ×3 sites) — DESIGN-CHOICE, your call
> `to_string_lossy()` can silently replace non-UTF-8 bytes with U+FFFD, which would turn a real temp
> path into a different string and make failures show up later as confusing "missing file" errors.
> Since this test ultimately passes the path as a `&str` CLI arg anyway, it's usually better to fail
> fast with an explicit UTF-8 expectation (or refactor the runner to pass `OsStr` throughout).

This is the flip side of the #1291/#1248 fix (which moved OFF `.unwrap()` to avoid a panic on
non-UTF-8). Copilot's point: `to_string_lossy()` swaps the *panic* for a *silent path corruption*
(U+FFFD) that surfaces as a baffling "missing file" downstream. YOUR call on the tradeoff — but for a
TEST that already funnels the path through a `&str` CLI arg, an explicit `.expect("temp path must be
valid UTF-8")` is arguably better than lossy (fail fast + clear message on the pathological
non-UTF-8 temp dir, which is itself vanishingly rare in CI). The fully-correct option is `OsStr`
end-to-end, but that's a bigger runner refactor. Not a live bug either way — pick the failure mode you
prefer (loud-expect vs lossy) and be consistent across the 3 sites.
