# PR review comment — mirrored from GitHub PR #413 (Copilot inline)

- **PR:** #413 "fleet: thirty-eighth batch (lsp, core-opt, rust-backend, open-sums, broad features)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/cdz/src/main.rs:513`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591451209
- **Link:** https://github.com/camshaft/cadenza/pull/413#discussion_r3591451209

## Comment (verbatim)
> `cdz build` allows `entry` to be a glob (per the comment), but the entry name passed to the compiler is derived from the *pattern* (`entry_spec`) rather than the resolved entry file. If `entry` is a glob (e.g. `src/*.cdz`), this can produce an invalid entry name (like `*`) and fail package linking. Also, if the entry glob matches multiple files, the build should fail with a clear error rather than implicitly compiling multiple entry candidates.
>
> Derive `entry_name` from the single resolved entry file path, and require that `entry` expands to exactly one file before adding `modules`.

## Liaison triage
`cdz build`: the entry NAME handed to the compiler is derived from the glob PATTERN (`entry_spec`), not
the resolved file, so a glob entry (`src/*.cdz`) can yield an invalid entry name (e.g. `*`) → package
linking fails; and a multi-match glob has no clear error. Real cdz-tooling correctness/UX bug. FIX (as
reviewer says): derive `entry_name` from the single resolved entry file path, and require the glob
expands to exactly one file (else fail with a clear error). cdz-tooling territory (v-cdz-tooling owns
the cdz CLI). Fix on `trunk`. Quote + link in queue file.
