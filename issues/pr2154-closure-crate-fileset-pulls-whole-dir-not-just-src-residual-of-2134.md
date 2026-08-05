# PR #2154 review — flake.nix (v-nix) — OPEN — build-isolation residual [VERIFIED, LOW-MED] (residual of MY #2134)

https://github.com/camshaft/cadenza/pull/2154 (seq-126 Part B.1 — TRUE src-isolation for per-crate checks;
THE fix for MY #2134 src-isolation finding). Copilot 1 inline — the isolation is much better (real
src-isolation delivered) but a residual remains: closure crates are included as WHOLE DIRECTORIES, not
scoped to src/.

## the fileset maps each closure crate to its whole directory (`./${rootWorkspaceCrates.${c}}`), pulling in dependency crates' `tests/` + other top-level files → editing a dependency crate's tests (which C doesn't run) still invalidates C's check, undermining the "ONLY C's tests/" goal (Copilot, flake.nix:328) — build-isolation [VERIFIED, LOW-MED]
> `fileset` currently includes each closure crate as a directory (`./${rootWorkspaceCrates.${c}}`), which
> pulls in more than just `src/` (e.g. dependency crates' `tests/` and any other top-level files). That
> undermines the stated goal of "ONLY C's tests/" and can cause unrelated edits (like changing tests in a
> dependency crate that isn't being tested) to invalidate this crate's check. Consider scoping closure
> inputs to `Cargo.toml` + `src/` (and optionally `build.rs`) for each closure member, and keep `tests/`
> only for the checked crate.

VERIFIED in the #2154 diff: `fileset = unions ((map (c: ./. + "/${rootWorkspaceCrates.${c}}") closure)
++ nonClosureManifests closure ++ optional (…/tests exists) …/tests …)` (diff:57-61). The closure map
takes each closure crate as its WHOLE directory `./${rootWorkspaceCrates.${c}}` — so a dependency crate in
C's closure contributes its ENTIRE tree: `src/` (needed, correct) PLUS its `tests/`, `benches/`, any
top-level files (NOT needed to compile C, and NOT run by C's check). The comment two lines up (diff:23-24)
states the goal as "FULL src/ for C's dep-CLOSURE … + ONLY C's tests/". But because closure members come
in as directories, a DEPENDENCY's `tests/` is in the fileset too → editing dep-crate D's tests invalidates
C's check even though C never runs them. So the fix delivers real SRC-isolation (the #2134 headline — huge
improvement over allMemberSrc) but not TESTS-isolation across the closure. LOW-MED (much narrower than the
#2134 whole-workspace cross-trigger, but the "ONLY C's tests/" claim is still overstated). Fix per Copilot
+ matching the non-closure treatment: scope each CLOSURE member to `Cargo.toml` + `src/` (+ optional
`build.rs`), and add `tests/` ONLY for the checked crate `C` (already done via the `optional …/tests`
union). i.e. don't take closure members as whole dirs — take their `src/`+manifest, mirroring how
non-closure members already get manifest-only. v-nix owns flake.nix / the CI pipeline. PR OPEN → foldable.
(Owning the chain: my #2134 drove this fix; this is the one-layer-deeper residual on it — the closure-dir
granularity over-includes. The core src-isolation win stands.)
