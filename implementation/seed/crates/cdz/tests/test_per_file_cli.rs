//! `cdz test <dir>` — the shared-arena `EmitTestsPerFile` precompile path is BEHAVIOR-IDENTICAL to the
//! per-file compile.
//!
//! `cdz test <dir>` now compiles all target files in ONE `EmitTestsPerFile` pass (lower the shared closure
//! once, one component per file) and each file reuses its component instead of re-lowering the whole closure.
//! rcdzc's own test proves each file's shared-arena component is byte-identical to its standalone `EmitTests`
//! compile; this pins the CLI-visible contract: a directory run reports the SAME per-file PASS/FAIL, the same
//! per-file `N passed, M failed`, the same TOTAL, and a test-free file stays vacuously green — regardless of
//! whether a file's tests came from the precompile fast path or the per-file fallback.

use std::process::Command;

fn cdz() -> &'static str {
    env!("CARGO_BIN_EXE_cdz")
}

/// Whether the value-heap runtime STORE is present. CI's bare `test` job runs `cargo test` with NO
/// `cargo xtask build`, so there is no store — and a composed/heap-value `cdz test` run resolves the
/// content-addressed runtime BEFORE it can report PASS/FAIL, failing "no runtime of content address …
/// in the store … build it" + exiting non-zero storeless. Skip such a test when the store is absent;
/// the store-having `gate` + `@test suites` jobs exercise it fully. Resolve the store dir the SAME way
/// the runtime resolver does (`CADENZA_STORE` env first, else `<target>/cadenza-store`), so this guard
/// AGREES with the storeless-rerun mechanism (which sets `CADENZA_STORE` to an empty temp dir).
/// Mirrors the `store_present()` guard in `run_emitted_cli.rs` / `peer_list_handle_cli.rs`.
fn store_present() -> bool {
    if let Ok(dir) = std::env::var("CADENZA_STORE") {
        return std::path::Path::new(&dir).is_dir()
            && std::fs::read_dir(&dir)
                .map(|mut e| e.next().is_some())
                .unwrap_or(false);
    }
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"))
        .parent()
        .and_then(|d| d.parent())
        .map(|t| t.join("cadenza-store").exists())
        .unwrap_or(false)
}

/// Write a multi-file package: `a` (1 pass + 1 fail), `b` (1 pass), `lib` (no @test). Returns the dir.
fn package(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-perfile-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("a.cdz"),
        "@test\ndef t_a_pass() =\n  if 1 + 1 == 2 then unit else trap(\"a: math\")\n\n\
         @test\ndef t_a_fail() =\n  trap(\"a: intentional fail\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.cdz"),
        "@test\ndef t_b_pass() =\n  if 2 * 3 == 6 then unit else trap(\"b: math\")\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib.cdz"), "def helper(n: Int64) =\n  n + 1\n").unwrap();
    dir
}

/// Run `cdz test <path>`; return (exit_ok, stdout).
fn run_test(path: &std::path::Path) -> (bool, String) {
    let out = Command::new(cdz())
        .args(["test", path.to_str().unwrap()])
        .output()
        .expect("spawn cdz test");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

#[test]
fn a_directory_run_reports_the_same_per_file_and_total_results() {
    let dir = package("dir");
    let (ok, out) = run_test(&dir);
    // The suite has a failing test, so the run exits non-zero — expected.
    assert!(!ok, "a suite with a failing test exits non-zero:\n{out}");

    // Per-file results (fast path for a/b that HAVE @tests; lib is vacuously green with no output block body).
    assert!(out.contains("PASS t_a_pass"), "a's passing test:\n{out}");
    assert!(out.contains("FAIL t_a_fail"), "a's failing test:\n{out}");
    assert!(out.contains("PASS t_b_pass"), "b's passing test:\n{out}");
    // a: 1 passed, 1 failed; b: 1 passed, 0 failed — each file's own summary line.
    assert!(
        out.contains("1 passed, 1 failed"),
        "a's per-file summary:\n{out}"
    );
    assert!(
        out.contains("1 passed, 0 failed"),
        "b's per-file summary:\n{out}"
    );
    // The aggregate across the package.
    assert!(
        out.contains("TOTAL: 2 passed, 1 failed (across 3 files)"),
        "the package TOTAL aggregates all files:\n{out}"
    );
}

#[test]
fn a_single_file_run_matches_that_file_within_the_directory_run() {
    // The single-file path FALLS BACK to the per-file compile (no closure to share) — its result must match
    // what the same file produces inside the directory (fast) run: same PASS/FAIL lines + per-file summary.
    let dir = package("single");
    let (_ok, single) = run_test(&dir.join("a.cdz"));
    assert!(
        single.contains("PASS t_a_pass")
            && single.contains("FAIL t_a_fail")
            && single.contains("1 passed, 1 failed"),
        "single-file a.cdz reports the same as within the dir run:\n{single}"
    );
}

#[test]
fn same_stem_files_in_different_subdirs_do_not_cross_contaminate() {
    // PR#881 regression: a closure file's link name is its dir-BLIND stem, and the shared precompile keys the
    // union + lookup by that stem. Two same-stem files in DIFFERENT subdirs (`d1/t.cdz`, `d2/t.cdz`) must NOT
    // collapse — else a lookup fetches the WRONG dir's component and MISATTRIBUTES pass/fail. The fix gates the
    // shared precompile on a single parent dir; a multi-dir tree falls back per-file. Assert each file runs
    // its OWN test: d1/t passes (t_one), d2/t fails via its OWN trap (t_two) — not a wrong-component error.
    let root = std::env::temp_dir().join(format!("cdz-collide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("d1")).unwrap();
    std::fs::create_dir_all(root.join("d2")).unwrap();
    std::fs::write(
        root.join("d1/t.cdz"),
        "@test\ndef t_one() =\n  if 1 == 1 then unit else trap(\"d1\")\n",
    )
    .unwrap();
    std::fs::write(
        root.join("d2/t.cdz"),
        "@test\ndef t_two() =\n  trap(\"d2 fails on its own\")\n",
    )
    .unwrap();
    let (_ok, out) = run_test(&root);
    assert!(
        out.contains("PASS t_one"),
        "d1/t runs its own passing test:\n{out}"
    );
    assert!(
        out.contains("FAIL t_two"),
        "d2/t runs its OWN failing test (not a wrong-component lookup):\n{out}"
    );
    // The tell of the bug was a "component exports no function `t-two`" error (fetched d1's component). Its
    // ABSENCE confirms d2 ran its own component.
    assert!(
        !out.contains("exports no function"),
        "no wrong-component reuse (the stem-collision failure mode):\n{out}"
    );
}

#[test]
fn a_shared_import_closure_runs_via_the_composed_provider_path() {
    // Option-C composed path: when a dir's @test files IMPORT a shared (non-inlined) def, EmitTestsComposed
    // hoists that closure into ONE provider component + emits each file as a cross-edge-EXCLUDING consumer
    // importing it; run_test links consumer→provider via run_with_peers over one shared runtime. Assert the
    // per-file results are correct through that path: a RECURSIVE shared `sumto` (stays standalone → a real
    // cross-edge, not inlined) is called by two test files — one asserts the right value (PASS), one a wrong
    // value (FAIL via its own trap). Correct results here mean the shared closure crossed the peer edge fine.
    let dir = std::env::temp_dir().join(format!("cdz-composed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("shared.cdz"),
        "def sumto(n: Int64) =\n  if n == 0 then 0 else n + sumto(n - 1)\nexport { sumto }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ta.cdz"),
        "import { sumto } from \"shared\"\n@test\ndef t_sumto_pass() =\n  if sumto(5) == 15 then unit else trap(\"ta\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("tb.cdz"),
        "import { sumto } from \"shared\"\n@test\ndef t_sumto_fail() =\n  if sumto(3) == 999 then unit else trap(\"tb intentional\")\n",
    )
    .unwrap();
    let (_ok, out) = run_test(&dir);
    assert!(
        out.contains("PASS t_sumto_pass"),
        "ta's test passes through the composed provider (sumto(5)==15):\n{out}"
    );
    assert!(
        out.contains("FAIL t_sumto_fail"),
        "tb's test fails on its own trap through the composed provider:\n{out}"
    );
    // The shared-closure fn crossed the peer edge correctly — NOT a link/exports mismatch (which would show
    // as "exports no function" or an invalid-module error rather than a clean per-file PASS/FAIL).
    assert!(
        !out.contains("exports no function") && !out.contains("could not run test"),
        "the consumer linked against the provider cleanly (no cross-component link error):\n{out}"
    );
}

#[test]
fn a_property_test_in_a_shared_closure_dir_still_runs_correctly() {
    // A MULTI-TRIAL test (property test with a scalar param) in a shared-closure dir. Since the (a) fix
    // (CompiledComposition JIT'd once + reused per trial, PR#892), the composed path handles multi-trial
    // tests without a per-trial re-JIT — so this now runs via composed (no forced standalone fall-back). This
    // pins that such a test runs + passes correctly over all its trials; the run path is invisible to behavior
    // — only the result is asserted here.
    let dir = std::env::temp_dir().join(format!("cdz-composed-prop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("shared.cdz"),
        "def sumto(n: Int64) =\n  if n == 0 then 0 else n + sumto(n - 1)\nexport { sumto }\n",
    )
    .unwrap();
    // A PROPERTY test (scalar param `k`) that imports the shared closure: `sumto(0) == 0` holds for every k,
    // so it PASSES over all trials. Its presence forces this file off the composed path onto standalone.
    std::fs::write(
        dir.join("tp.cdz"),
        "import { sumto } from \"shared\"\n@test\ndef t_prop(k: Int64) =\n  if sumto(0) == 0 then unit else trap(\"prop\")\n",
    )
    .unwrap();
    let (ok, out) = run_test(&dir);
    assert!(
        ok && out.contains("PASS t_prop"),
        "a property test in a shared-closure dir runs + passes (via the standalone fallback, no per-trial \
         re-JIT):\n{out}"
    );
    assert!(
        !out.contains("could not run test") && !out.contains("exports no function"),
        "no link/run error — the fallback compiled the file standalone cleanly:\n{out}"
    );
}

#[test]
fn the_provider_cache_persists_reuses_and_self_heals() {
    // Cross-invocation provider-cache (single-file-local-verify win): the FIRST `cdz test <dir>` over a
    // shared-closure dir is a cache MISS → emits + PERSISTS the shared-closure provider component (keyed by
    // the canonical closure hash) under CDZ_PROVIDER_CACHE; a SUBSEQUENT run is a HIT → reuses the cached
    // provider (skipping the provider emit). A CORRUPT cache entry must SELF-HEAL (re-emit), never break the
    // run. Uses an isolated CDZ_PROVIDER_CACHE so it doesn't touch the shared default store/providers.
    let dir = std::env::temp_dir().join(format!("cdz-pcache-src-{}", std::process::id()));
    let cache = std::env::temp_dir().join(format!("cdz-pcache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("shared.cdz"),
        "def sumto(n: Int64) =\n  if n == 0 then 0 else n + sumto(n - 1)\nexport { sumto }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ta.cdz"),
        "import { sumto } from \"shared\"\n@test\ndef t_c() =\n  if sumto(5) == 15 then unit else trap(\"ta\")\n",
    )
    .unwrap();

    let run = || -> (bool, String) {
        let out = Command::new(cdz())
            .args(["test", dir.to_str().unwrap()])
            .env("CDZ_PROVIDER_CACHE", &cache)
            .output()
            .expect("spawn cdz test");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
        )
    };
    // A helper: the sole persisted provider file, if any.
    let cached_provider = || -> Option<std::path::PathBuf> {
        std::fs::read_dir(&cache).ok()?.flatten().find_map(|e| {
            let p = e.path();
            // Match the PRODUCTION cache filename suffix (`<hash>.provider.wasm`), not just any `.wasm` — a
            // stray/multi-entry `.wasm` in the dir must not be mistaken for the provider we persisted.
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".provider.wasm"))
                .then_some(p)
        })
    };

    // RUN 1 — MISS: runs correctly + PERSISTS a provider.
    let (ok1, out1) = run();
    assert!(
        ok1 && out1.contains("PASS t_c"),
        "run 1 (miss) passes:\n{out1}"
    );
    let provider = cached_provider().expect("run 1 (miss) persisted a provider .wasm to the cache");

    // RUN 2 — HIT: still correct, reusing the cached provider (same file, not re-emitted differently).
    let (ok2, out2) = run();
    assert!(
        ok2 && out2.contains("PASS t_c"),
        "run 2 (hit) passes:\n{out2}"
    );

    // CORRUPT the cached provider → RUN 3 must SELF-HEAL (validation rejects it → miss → re-emit), NOT error.
    std::fs::write(&provider, b"GARBAGE").unwrap();
    let (ok3, out3) = run();
    assert!(
        ok3 && out3.contains("PASS t_c"),
        "run 3 (corrupt cache) self-heals + passes (not an 'invalid peer component' error):\n{out3}"
    );
    assert!(
        !out3.contains("invalid peer component") && !out3.contains("could not compile"),
        "a corrupt cache entry must not surface as a compile error:\n{out3}"
    );
}

#[test]
fn a_single_file_with_imports_reuses_a_warmed_provider_cache() {
    // The single-file-local-verify win (v-compiler-ml's need): `cdz test <one-file>` where that file IMPORTS a
    // shared closure must REUSE a warmed provider cache — skipping the (expensive) shared-closure lower — the
    // same consumer-only HIT the dir path takes. Regression guard: the composed path once blanket-skipped on
    // `files.len() < 2`, so a single-file run NEVER touched the cache (it always re-embedded the whole closure).
    // Now the skip is keyed on the closure UNION (`asts.len() < 2`) — a single SELF-CONTAINED file still skips,
    // but a single file WITH imports takes the cache path. Assert: (1) warming via a run persists a provider;
    // (2) a SINGLE-FILE run of the importing file, with the SAME cache, still passes (it's a HIT — the shared
    // `sumto` crossed the peer edge from the cached provider, not re-embedded). A self-contained single file is
    // covered separately by `a_single_file_run_matches_that_file_within_the_directory_run` (must stay standalone).
    let dir = std::env::temp_dir().join(format!("cdz-1file-hit-src-{}", std::process::id()));
    let cache = std::env::temp_dir().join(format!("cdz-1file-hit-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&dir).unwrap();
    // A RECURSIVE shared closure (stays standalone → a real cross-edge, not inlined).
    std::fs::write(
        dir.join("shared.cdz"),
        "def sumto(n: Int64) =\n  if n == 0 then 0 else n + sumto(n - 1)\nexport { sumto }\n",
    )
    .unwrap();
    // TWO importing @test files so the WARM run (the dir) takes the composed+persist path.
    std::fs::write(
        dir.join("ta.cdz"),
        "import { sumto } from \"shared\"\n@test\ndef t_ha() =\n  if sumto(5) == 15 then unit else trap(\"ta\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("tb.cdz"),
        "import { sumto } from \"shared\"\n@test\ndef t_hb() =\n  if sumto(4) == 10 then unit else trap(\"tb\")\n",
    )
    .unwrap();

    let cached_provider = || -> Option<std::path::PathBuf> {
        std::fs::read_dir(&cache).ok()?.flatten().find_map(|e| {
            let p = e.path();
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".provider.wasm"))
                .then_some(p)
        })
    };

    // WARM: a dir run (>=2 files) persists the shared-closure provider. `CDZ_PROVIDER_CACHE_TRACE` makes the
    // cache decision OBSERVABLE on stderr, so we can PROVE the single-file run below is a genuine HIT (reusing
    // the cached provider) and not the standalone fallback — the exact regression this test guards.
    let warm = Command::new(cdz())
        .args(["test", dir.to_str().unwrap()])
        .env("CDZ_PROVIDER_CACHE", &cache)
        .env("CDZ_PROVIDER_CACHE_TRACE", "1")
        .output()
        .expect("spawn cdz test (warm)");
    let warm_out = String::from_utf8_lossy(&warm.stdout);
    let warm_err = String::from_utf8_lossy(&warm.stderr);
    assert!(
        warm.status.success() && warm_out.contains("PASS t_ha") && warm_out.contains("PASS t_hb"),
        "warm dir run passes:\n{warm_out}\n{warm_err}"
    );
    assert!(
        warm_err.contains("[provider-cache] miss persisted"),
        "the warm run was a MISS that PERSISTED the provider (trace):\n{warm_err}"
    );
    cached_provider().expect("the warm run persisted a provider .wasm to the cache");

    // HIT via a SINGLE-FILE run of the importing file: must pass AND take the cache path (trace = `hit`) — the
    // regression this guards is the single-file path NEVER reaching the cache (it always re-embedded). We assert
    // both the correct result (the shared `sumto` crossed the peer edge from the cached provider) and the HIT
    // marker (proving the closure was NOT re-lowered) — a link/exports mismatch would show instead.
    let single = Command::new(cdz())
        .args(["test", dir.join("ta.cdz").to_str().unwrap()])
        .env("CDZ_PROVIDER_CACHE", &cache)
        .env("CDZ_PROVIDER_CACHE_TRACE", "1")
        .output()
        .expect("spawn cdz test (single-file hit)");
    let single_out = String::from_utf8_lossy(&single.stdout);
    let single_err = String::from_utf8_lossy(&single.stderr);
    assert!(
        single.status.success() && single_out.contains("PASS t_ha"),
        "a single-file run of the importing file passes via the warmed cache:\n{single_out}\n{single_err}"
    );
    assert!(
        single_err.contains("[provider-cache] hit"),
        "the single-file run was a cache HIT (reused the warmed provider, did NOT re-lower the closure):\n{single_err}"
    );
    assert!(
        !single_out.contains("exports no function")
            && !single_out.contains("could not run test")
            && !single_out.contains("invalid peer component"),
        "the single-file run linked cleanly against the cached provider (no cross-component error):\n{single_out}"
    );
}

#[test]
fn a_heterogeneous_dir_composes_one_provider_per_shared_closure() {
    // Option-A per-closure grouping: a `cdz test <dir>` over a HETEROGENEOUS tree — files importing DIFFERENT
    // shared closures — must emit ONE provider PER genuine closure, NOT one whole-dir union. Regression guard:
    // the composed path once folded EVERY file's cross-edges into a single union provider; on a real dir with
    // ~20 distinct libs that provider ≈ the whole compiler, and one un-representable edge anywhere declined the
    // WHOLE dir to per-file. Here two independent shared libs (`liba` with `fa`, `libb` with `fb`) each back
    // their own consumer group. Assert: (1) all files pass through their respective composed providers; (2) the
    // trace shows TWO distinct `miss persisted` keys (two providers, not one union); (3) two provider files
    // persisted. This is the exact heterogeneous case the whole-dir union mis-scoped.
    let dir = std::env::temp_dir().join(format!("cdz-hetero-{}", std::process::id()));
    let cache = std::env::temp_dir().join(format!("cdz-hetero-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&dir).unwrap();
    // Closure A: recursive `fa` (stays standalone → a real cross-edge). Closure B: recursive `fb`. DISJOINT —
    // no file imports both, so grouping by imported-closure-set yields TWO groups.
    std::fs::write(
        dir.join("liba.cdz"),
        "def fa(n: Int64) =\n  if n == 0 then 0 else n + fa(n - 1)\nexport { fa }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("libb.cdz"),
        "def fb(n: Int64) =\n  if n == 0 then 1 else n * fb(n - 1)\nexport { fb }\n",
    )
    .unwrap();
    // Two @test files import liba, two import libb — four consumers across two closure groups.
    std::fs::write(
        dir.join("ta1.cdz"),
        "import { fa } from \"liba\"\n@test\ndef t_a1() =\n  if fa(5) == 15 then unit else trap(\"a1\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ta2.cdz"),
        "import { fa } from \"liba\"\n@test\ndef t_a2() =\n  if fa(4) == 10 then unit else trap(\"a2\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("tb1.cdz"),
        "import { fb } from \"libb\"\n@test\ndef t_b1() =\n  if fb(4) == 24 then unit else trap(\"b1\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("tb2.cdz"),
        "import { fb } from \"libb\"\n@test\ndef t_b2() =\n  if fb(3) == 6 then unit else trap(\"b2\")\n",
    )
    .unwrap();

    let out = Command::new(cdz())
        .args(["test", dir.to_str().unwrap()])
        .env("CDZ_PROVIDER_CACHE", &cache)
        .env("CDZ_PROVIDER_CACHE_TRACE", "1")
        .output()
        .expect("spawn cdz test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // All four tests pass through their respective composed providers.
    for t in ["PASS t_a1", "PASS t_a2", "PASS t_b1", "PASS t_b2"] {
        assert!(
            out.status.success() && stdout.contains(t),
            "{t} through its group provider:\n{stdout}\n{stderr}"
        );
    }
    assert!(
        !stdout.contains("exports no function") && !stdout.contains("could not run test"),
        "each consumer linked cleanly against ITS group's provider (no cross-group misattribution):\n{stdout}"
    );
    // TWO providers persisted (one per closure), with TWO DISTINCT keys — not one union. Parse the trace keys.
    let keys: std::collections::HashSet<&str> = stderr
        .lines()
        .filter(|l| l.contains("[provider-cache] miss persisted"))
        .filter_map(|l| l.split("key=").nth(1))
        .map(|s| s.split_whitespace().next().unwrap_or(""))
        .collect();
    assert!(
        keys.len() == 2,
        "exactly TWO distinct provider closures persisted (one per shared lib), not a single whole-dir union:\n{stderr}"
    );
    let persisted = std::fs::read_dir(&cache)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".provider.wasm"))
                })
                .count()
        })
        .unwrap_or(0);
    assert!(
        persisted == 2,
        "two provider .wasm files persisted (one per closure group), got {persisted}"
    );
}

#[test]
fn a_composed_test_whose_heap_runtime_lives_only_in_the_provider_still_resolves_it() {
    // SKIP (don't fail) storeless: this composed run builds a recursive `Nat` HEAP value, so `cdz test`
    // resolves the content-addressed value-heap runtime before it can report PASS/FAIL. With no store
    // (CI's bare `test` job) it exits non-zero "no runtime of content address … build it". The
    // store-having gate/@test jobs witness the run fully. (Checked up front so the guard can't miss the
    // resolver's error string.)
    if !store_present() {
        eprintln!(
            "[provider-rt] skipping: no cadenza-store (storeless test job) — a composed heap-value \
             `cdz test` needs the runtime"
        );
        return;
    }

    // REGRESSION (the closure-grouping reject): a composed test's CONSUMER is cross-edge-EXCLUDING — the
    // heap-using shared closure was hoisted into the PROVIDER. So a consumer can import NO value-heap runtime
    // while its provider peer DOES (the shared fn constructs/consumes a heap value; the consumer only holds the
    // handle across two shared calls). `run_test_file` resolved the runtime from the CONSUMER only, leaving
    // `opts.runtime = None` for such a consumer → every test FAILED "component requires the value-heap runtime
    // … but none was provided" (this hit every cad/units.cdz test once grouping made it take the composed path
    // instead of the old whole-dir-union DECLINE-to-standalone). The fix resolves the runtime from EITHER the
    // consumer OR the provider (they pin the same runtime by content hash). Fixture: a RECURSIVE sum type built
    // in the shared lib (recursion keeps it a real cross-edge, not inlined; the sum value is a heap handle that
    // crosses the peer edge), with the consumer only comparing a scalar returned from a shared accessor — so
    // the consumer imports no runtime but the provider requires it. Pre-fix this FAILED with the runtime error;
    // post-fix it PASSES (the provider's runtime is resolved + composed for the shared instance).
    let dir = std::env::temp_dir().join(format!("cdz-provider-rt-{}", std::process::id()));
    let cache = std::env::temp_dir().join(format!("cdz-provider-rt-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&dir).unwrap();
    // A recursive Peano-style sum: `mk` builds an N-deep heap value, `count` folds it back to a scalar. Both
    // recurse (stay standalone → real cross-edges), and the `Nat` handle is the heap value crossing the edge.
    std::fs::write(
        dir.join("nlib.cdz"),
        "type Nat =\n  | Z\n  | S(Nat)\ndef mk(n: Int64) =\n  if n == 0 then Nat.Z else Nat.S(mk(n - 1))\n\
         def count(x: Nat) =\n  match x with | Nat.Z => 0 | Nat.S(r) => 1 + count(r)\nexport { mk, count }\n",
    )
    .unwrap();
    // Two consumers: each builds a Nat via `mk` (in the provider), holds the handle, folds it via `count` (in
    // the provider), and compares the resulting SCALAR — so the consumer itself performs no heap op.
    std::fs::write(
        dir.join("na.cdz"),
        "import { mk, count } from \"nlib\"\n@test\ndef t_n1() =\n  let x = mk(3) in if count(x) == 3 then unit else trap(\"n1\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("nb.cdz"),
        "import { mk, count } from \"nlib\"\n@test\ndef t_n2() =\n  let x = mk(5) in if count(x) == 5 then unit else trap(\"n2\")\n",
    )
    .unwrap();

    let out = Command::new(cdz())
        .args(["test", dir.to_str().unwrap()])
        .env("CDZ_PROVIDER_CACHE", &cache)
        .env("CDZ_PROVIDER_CACHE_TRACE", "1")
        .output()
        .expect("spawn cdz test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The composed path must fire (a provider was persisted) — else this doesn't exercise the reject path.
    assert!(
        stderr.contains("[provider-cache] miss persisted"),
        "the shared Nat closure composed into a provider (else the composed runtime path isn't exercised):\n{stderr}"
    );
    // Both tests pass — the provider's value-heap runtime was resolved + composed even though the CONSUMER
    // imports none. Pre-fix, both FAILED "requires the value-heap runtime … but none was provided".
    assert!(
        out.status.success() && stdout.contains("PASS t_n1") && stdout.contains("PASS t_n2"),
        "a composed test whose runtime lives only in the provider still resolves it:\n{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("but none was provided")
            && !stdout.contains("requires the value-heap runtime"),
        "the provider's runtime must be resolved for a runtime-free consumer (the grouping reject):\n{stdout}"
    );
}

#[test]
fn an_imported_with_tests_file_runs_its_own_group_not_another_groups_overwrite() {
    // PR#914 correctness (grouping-era cousin of the PR#881 stem collision): the composed emit produces a
    // consumer for EVERY closure member that has `@test`s — so a file that is a TARGET of its own group AND an
    // imported-with-tests MEMBER of another group (e.g. `mid` below: target of group {base}, imported member of
    // group {base,mid,extra}) gets a `mid` consumer emitted in BOTH groups. Keyed by bare stem, the second
    // insert would OVERWRITE the first (last-group-wins), so `mid`'s tests could run via the wrong group. The
    // fix stores a consumer ONLY for files that are TARGETS of that group. Assert every file's OWN tests run +
    // pass — `mid` (target of {base}) and `hi` (target of {base,mid,extra}, where mid is an imported member).
    let dir = std::env::temp_dir().join(format!("cdz-pr914-{}", std::process::id()));
    let cache = std::env::temp_dir().join(format!("cdz-pr914-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&dir).unwrap();
    // Two independent recursive shared libs (stay standalone → real cross-edges).
    std::fs::write(
        dir.join("base.cdz"),
        "def base(n: Int64) =\n  if n == 0 then 0 else n + base(n - 1)\nexport { base }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("extra.cdz"),
        "def extra(n: Int64) =\n  if n == 0 then 1 else n * extra(n - 1)\nexport { extra }\n",
    )
    .unwrap();
    // `mid`: imports ONLY base (group key {base}), HAS its own @test, AND exports `mid`. So it's both a target
    // of group {base} and (via `hi`) an imported-with-tests member of group {base,mid,extra}.
    std::fs::write(
        dir.join("mid.cdz"),
        "import { base } from \"base\"\ndef mid(n: Int64) =\n  base(n) + 1\nexport { mid }\n\
         @test\ndef t_mid() =\n  if base(3) == 6 then unit else trap(\"mid\")\n",
    )
    .unwrap();
    // `hi`: imports base + mid + extra → a DIFFERENT group {base,mid,extra} (distinct provider closure). `mid`
    // is an imported-with-tests member of THIS group — the cross-group `mid`-consumer that must not overwrite.
    std::fs::write(
        dir.join("hi.cdz"),
        "import { base } from \"base\"\nimport { mid } from \"mid\"\nimport { extra } from \"extra\"\n\
         @test\ndef t_hi() =\n  if mid(4) + extra(3) == 11 + 6 then unit else trap(\"hi\")\n",
    )
    .unwrap();

    let out = Command::new(cdz())
        .args(["test", dir.to_str().unwrap()])
        .env("CDZ_PROVIDER_CACHE", &cache)
        // PR#918: assert the COMPOSED path actually ran — `cdz test` is best-effort, so if grouping ever
        // DECLINES it silently falls back to per-file `EmitTests` where t_mid/t_hi still PASS standalone,
        // leaving this green WITHOUT exercising the grouping path the PR#914 regression lives in (false guard).
        // The trace lets us require the composed providers were emitted, so a silent fallback FAILS the test.
        .env("CDZ_PROVIDER_CACHE_TRACE", "1")
        .output()
        .expect("spawn cdz test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The COMPOSED path must have run — the two distinct groups ({base} for mid, {base,mid,extra} for hi) each
    // emit + persist a provider. Require BOTH `miss persisted` keys (2 distinct) so this can't green via the
    // per-file fallback (which emits NO provider and prints NO trace) — the PR#918 false-guard fix.
    let keys: std::collections::HashSet<&str> = stderr
        .lines()
        .filter(|l| l.contains("[provider-cache] miss persisted"))
        .filter_map(|l| l.split("key=").nth(1))
        .map(|s| s.split_whitespace().next().unwrap_or(""))
        .collect();
    assert!(
        keys.len() >= 2,
        "the composed grouping path ran (≥2 distinct providers persisted, one per group) — NOT a silent \
         per-file fallback that would green without exercising the cross-group consumer path:\n{stderr}"
    );
    // Both targets pass through their OWN group's provider — mid isn't misattributed to hi's group (or lost).
    assert!(
        out.status.success() && stdout.contains("PASS t_mid") && stdout.contains("PASS t_hi"),
        "an imported-with-tests file runs its own group, not a cross-group overwrite:\n{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("exports no function") && !stdout.contains("could not run test"),
        "no wrong-group link (the PR#914 consumer-overwrite failure mode):\n{stdout}"
    );
}

#[test]
fn warm_only_populates_the_cache_so_a_later_per_file_sweep_hits() {
    // `cdz test --warm-only <dir>` warms each closure group's provider into the cache + EXITS without running
    // tests, so a SUBSEQUENT per-file sweep (each file its own process — the gate's runaway-localization mode)
    // HITS instead of every file cold-emitting the shared closure in parallel (the N×-redundant-emit race:
    // without a warm, N concurrent single-file runs of files sharing ONE closure each MISS + re-emit it). This
    // is the operator's gate-wall-clock lever: warm-once-then-sweep collapses N cliff emits to 1.
    let dir = std::env::temp_dir().join(format!("cdz-warm-{}", std::process::id()));
    let cache = std::env::temp_dir().join(format!("cdz-warm-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&dir).unwrap();
    // A recursive shared closure (stays standalone → a real cross-edge) imported by three @test files.
    std::fs::write(
        dir.join("shared.cdz"),
        "def sumto(n: Int64) =\n  if n == 0 then 0 else n + sumto(n - 1)\nexport { sumto }\n",
    )
    .unwrap();
    for (f, k) in [("ta", 5), ("tb", 4), ("tc", 3)] {
        let want = (0..=k).sum::<i64>();
        std::fs::write(
            dir.join(format!("{f}.cdz")),
            format!(
                "import {{ sumto }} from \"shared\"\n@test\ndef t_{f}() =\n  if sumto({k}) == {want} then unit else trap(\"{f}\")\n"
            ),
        )
        .unwrap();
    }

    // WARM-ONLY: emits the provider once (a single `miss persisted`), runs NO tests (no PASS lines), exits 0.
    let warm = Command::new(cdz())
        .args(["test", "--warm-only", dir.to_str().unwrap()])
        .env("CDZ_PROVIDER_CACHE", &cache)
        .env("CDZ_PROVIDER_CACHE_TRACE", "1")
        .output()
        .expect("spawn cdz test --warm-only");
    let warm_out = String::from_utf8_lossy(&warm.stdout);
    let warm_err = String::from_utf8_lossy(&warm.stderr);
    assert!(
        warm.status.success(),
        "warm-only exits 0:\n{warm_out}\n{warm_err}"
    );
    assert!(
        warm_out.contains("warmed") && !warm_out.contains("PASS "),
        "warm-only warms the cache WITHOUT running tests (no PASS lines):\n{warm_out}"
    );
    assert!(
        warm_err.matches("[provider-cache] miss persisted").count() == 1,
        "warm-only emits the shared provider exactly ONCE:\n{warm_err}"
    );

    // THEN a per-file sweep (each file its own process, as the gate runs them) — every file HITS the warm.
    let run = |f: &str| -> (bool, String) {
        let out = Command::new(cdz())
            .args(["test", dir.join(f).to_str().unwrap()])
            .env("CDZ_PROVIDER_CACHE", &cache)
            .env("CDZ_PROVIDER_CACHE_TRACE", "1")
            .output()
            .expect("spawn cdz test <file>");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    for (f, t) in [("ta.cdz", "t_ta"), ("tb.cdz", "t_tb"), ("tc.cdz", "t_tc")] {
        let (ok, err) = run(f);
        let _ = t;
        assert!(ok, "per-file run of {f} passes after warm:\n{err}");
        assert!(
            err.contains("[provider-cache] hit") && !err.contains("miss persisted"),
            "after --warm-only, the per-file run of {f} HITS the cache (no re-emit):\n{err}"
        );
    }
}
