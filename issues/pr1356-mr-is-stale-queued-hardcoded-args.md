# PR #1356 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1356 (PR: "cand: v-fleet-tooling — 16fa5ebed").
Follow-up to the #1325/#1330 stale-queue short-circuit work.

## `mr_is_stale_queued` called with hard-coded `false` args + "single subprocess" comment overstates (Copilot, fleet.rs:2360) — maintainability/doc
> The call to `mr_is_stale_queued` is now passing hard-coded `false` values for `in_flight`/`landed`,
> which makes the call site misleading and easier to accidentally break if the surrounding
> early-`continue` logic changes. Also, the comment implies the landed check is a single subprocess,
> but `ref_landed_on_trunk` can run both `git merge-base --is-ancestor` and (if needed) `git cherry`
> (see `ref_landed_on_trunk` at fleet.rs:2224-2237), so it's more accurate to describe this as
> skipping the *additional* `git show` used for file collision.
> Consider storing the `landed` result and passing the actual variables into the predicate for
> self-documentation.

Two points from the #1325/#1330 short-circuit refactor: (1) the `false, false` literals at the call
site are only correct because the early-`continue`s already excluded in-flight/landed — but that's
implicit and break-prone; store `landed` and pass the real variables so the call self-documents. (2)
the comment says "single subprocess" for the landed check, but `ref_landed_on_trunk` can run
merge-base + git cherry — reword to "skips the additional `git show` for file collision".
