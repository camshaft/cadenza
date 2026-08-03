# PR #1348 review comments — cdz/tests/{normalize,fmt_project}_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1348 (PR: "cand: v-cdz-tooling — 23b890840").
Natural follow-on to the #1327 BrokenPipe-tolerance sweep — the tolerant-write logic is now duplicated.

## BrokenPipe-tolerant stdin write duplicated across CLI tests — extract a shared helper (Copilot, normalize_cli.rs:145 + fmt_project_cli.rs:306) — maintainability
> [normalize_cli.rs:145] The BrokenPipe-tolerant stdin write logic is now duplicated across multiple
> integration tests (here, fmt_project_cli.rs, and query_cli.rs uses a similar pattern). This
> duplication risks the tests drifting (different tolerated error kinds / messages) and makes future
> adjustments harder. Consider extracting a shared helper (e.g. tests/common.rs).
> [fmt_project_cli.rs:306] ...duplicated with normalize_cli.rs and overlaps with the helper added in
> convert_cli.rs. Move it into a shared integration-test helper module reused from all CLI tests that
> pipe stdin.

The #1327 sweep applied BrokenPipe-tolerance at each stdin-write site by copy — now it's duplicated
across normalize/fmt_project/convert/query CLI tests, which will drift (different tolerated kinds /
messages). Extract one shared helper (tests/common.rs — `write_stdin_tolerating_broken_pipe(child,
bytes)`) and call it everywhere so the behavior stays single-sourced.
