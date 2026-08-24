# reducer-multifile-cdz — multi-file `mkCadenzaGuest` gate fixture

A minimal Cadenza reducer guest that spans **two source files**: the entry `reducer.cdz` imports
`stamp` from the sibling library module `helper.cdz` (listed in `libs`). Its sole purpose is to be a
focused, permanent witness that the flake's multi-file `mkCadenzaGuest` path works — the `libs`
manifest + `cp`-to-clean-basename + import-by-stem resolution introduced in PR #3158.

Without this fixture the multi-file path has no passing exerciser on `main`: the real multi-file guest
(the §9 checker, `reducer-check-cdz`) is complex and separately gated, so a regression in
`mkCadenzaGuest`'s multi-file branch would not be caught here. This fixture isolates that mechanism —
if the multi-file compile breaks (e.g. an unresolved import from a store-path stem), the build fails.

Auto-enumerated by the flake like every other guest: a `libs` file beside `reducer.cdz` (one
repo-relative source path per line) flips it to the multi-file compile; absent, a guest stays
single-file. Driven by `harness-runs/reducer-multifile-cdz-echo.ml`.

It may retire once the checker guest is a stable, always-green on-`main` multi-file exerciser — until
then this is the guard.
