//! `cdz test` / `cdz watch` — the test runner, the shared-closure precompile, the `--emit-shred`
//! producer, and the property-testing engine (value generation, bounds narrowing, and shrinking).
//! Extracted verbatim from `main.rs` (file-shrink, behavior + surface unchanged). The whole in-process
//! runner is `#[cfg(feature = "standalone")]` (it needs the bundled `rcdzc`); the `!standalone` build
//! keeps only the honest-error `run_test` stub. The command entry points `run_test` / `run_watch` /
//! `run_emit_shred` are the items `main` dispatches to.

use crate::*;

// ── cdz test ─────────────────────────────────────────────────────────────────────────────────────

/// `cdz test FILE` — compile a SEPARATE test component from the file's `@test` NULLARY definitions and
/// run each, reporting pass/fail. The flow, all in this one process for the compile half:
///  1. Parse the source (`load_program`), encode the `ast` artifact.
///  2. Enumerate the `@test` definitions' SOURCE names from a `Db` (`db.test_defs`) — the tests to run,
///     in declaration order; filtered by `--filter` if given.
///  3. Compile with an `EmitTests` sidecar request → the wasm component whose exports ARE the tests
///     (`layout::compute_tests`). A test that TRAPS on failure crosses as a nullary no-result entry.
///  4. Run each test IN-PROCESS via the `cdz-run` LIBRARY (`run_capturing` — no sibling binary), calling
///     the test's kebab export. The export RETURNING = PASS; it TRAPPING = FAIL. A failure's message rides
///     an OBSERVED host-op entry (the assertion text the test emitted via its report host effect before
///     trapping); `run_capturing`'s observed-op list also yields the `Test.gen-int` count that distinguishes a
///     property test from a plain unit test — no subprocess, no stderr parsing.
///
/// Exits non-zero if ANY test fails (or if a file's compile declines / no `@test` is present) — the CI
/// shape. FILE may be a DIRECTORY: every source file under it (recursively, `.cdz`/`.ml`/`.sexp`) is run
/// and the pass/fail totals are aggregated, so `cdz test <dir>` runs a whole package's suite in one call.
///
/// The precompiled test components for a `cdz test <dir>` run, from ONE shared-arena `EmitTestsComposed`
/// compile: each file's `@test` CONSUMER component (keyed by file-link name), plus the ONE shared-closure
/// PROVIDER component every consumer imports + its interface name. `run_test_file` looks up its consumer and
/// runs it linked against the provider peer (`run_with_peers`) over one shared runtime — so the shared
/// closure is LOWERED and EMITTED once (a provider component) instead of re-embedded in every file's
/// component (the >98% per-file emit/JIT cost). BEST-EFFORT: on any hiccup (compile declines, provider or a
/// file's consumer absent, single file, multi-dir stem-collision) `run_test_file` FALLS BACK to its exact
/// per-file `EmitTests` compile — behavior is never worse than before.
/// A shared-closure PROVIDER peer: its component bytes, the interface name it exports (the consumer imports
/// under this exact name), and the closure's CONTENT HASH (the same `Query::ClosureHash` value the
/// `.provider.wasm` is keyed by, when available). The content hash — NOT the group key (which is the import
/// NAME-set, stable across content edits) — is what a JIT-artifact (cwasm) cache must key on: a content change
/// with an unchanged import set must invalidate the cwasm, else a stale compiled provider would be reused.
#[cfg(feature = "standalone")]
pub(crate) type ProviderPeer = (Vec<u8>, String, Option<String>);

#[derive(Default)]
#[cfg(feature = "standalone")]
pub(crate) struct Precompiled {
    /// file-link-name → (that file's `@test` CONSUMER component bytes, the PROVIDER-GROUP key it links
    /// against). The group key indexes `providers`. A file absent here fell back (self-contained, decline,
    /// or its group produced no provider) and `run_test_file` re-emits it standalone.
    components: std::collections::HashMap<String, (Vec<u8>, String)>,
    /// provider-group key → the group's [`ProviderPeer`]. ONE entry per GENUINE shared closure (Option-A
    /// grouping — a `cdz test <dir>` over a HETEROGENEOUS tree emits one provider per closure, NOT one
    /// whole-compiler union over every file). A consumer links against `providers[its group key]`; a missing
    /// entry ⇒ that group declined ⇒ its files fall back per-file.
    providers: std::collections::HashMap<String, ProviderPeer>,
    /// For a SINGLE-file `cdz test <file>` run ONLY: the file's import closure, loaded once and SHARED with
    /// `run_test_file` so it isn't parsed twice (PR#907 — dropping the `files.len() < 2` blanket-skip meant a
    /// single file's closure was loaded here for the cache decision AND again in `run_test_file`). `Rc` so the
    /// share is a refcount bump, not a deep clone of the arenas. `None` for a multi-file run (each file loads
    /// its own closure once in `run_test_file`, as before — stashing all N would raise peak memory for no gain).
    single_file_closure: Option<std::rc::Rc<Vec<closure::LoadedFile>>>,
}

/// Compile ONE closure-group's shared provider + its `@test` consumers, with the cross-invocation provider
/// cache — the per-group unit [`precompile_tests_per_file`] runs after partitioning the target files by
/// shared closure. `ast_inputs` is the UNION of this group's closure ASTs (the group's target `@test` files +
/// the shared libs they import, deduped by link name); `entry` is any closure file name (drives linking, does
/// not restrict which files' @tests emit). Returns the provider peer (bytes + interface name) when the
/// composed emit produced one, plus the per-file consumer components (named by file-link). SINGLE-MONO flow:
/// ONE `EmitTestsComposed` (one monomorphize+layout) yields the closure-hash sidecar plus the composed
/// provider plus every file's consumer; the cache decision is then made from that emitted hash, so a HIT
/// reuses the persisted `.provider.wasm` (discarding the emitted provider) while a MISS atomic-persists the
/// emitted provider. This replaces an earlier two-drive flow (`Query::ClosureHash`, then a HIT
/// `EmitTestsConsumerOnly` or a MISS `EmitTestsComposed`) that paid the closure monomorphize+layout TWICE on a
/// HIT; folding to one emit pays it once. Best-effort throughout: a decline yields `(None, [])` and every file
/// in the group falls back to its own `EmitTests`.
#[cfg(feature = "standalone")]
pub(crate) fn precompile_group(
    ast_inputs: Vec<cadenza_compile_abi::Artifact>,
    entry: &str,
    cache_dir: Option<&std::path::Path>,
) -> (Option<ProviderPeer>, Vec<(String, Vec<u8>)>) {
    let entry_marker = cadenza_compile_abi::abi::entry_artifact(entry);
    let drive = |req: cadenza_compile_abi::Request| -> cadenza_compile_abi::CompileOutput {
        let mut inputs = ast_inputs.clone();
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&[req]),
        ));
        inputs.push(entry_marker.clone());
        rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]))
    };

    // CROSS-INVOCATION PROVIDER CACHE, codegen-skip-on-HIT flow: drive `EmitTestsConsumerOnly` FIRST — one
    // monomorphize+layout that emits the closure CONTENT-HASH sidecar (`KIND_CLOSURE_HASH`, hoisted onto this
    // path by rcdzc #1502) + every file's CONSUMER component, but NO provider (`emit_provider=false`). We
    // decide HIT/MISS from that hash:
    //   • HIT (a validated `.provider.wasm` exists for the hash): pair the CACHED provider with the
    //     ConsumerOnly consumers — DONE, and we NEVER emit the provider. This SKIPS the ~215s provider CODEGEN
    //     (the ~570-def self-host closure's emit) that dominates the warm-once cost — the whole point of the
    //     cache. (Measured by v-compiler-perf, #1502: the ~231s HIT precompile was "~all of it the provider
    //     emit that gets thrown away.")
    //   • MISS: drive `EmitTestsComposed` — which emits the provider — and PERSIST it by the hash so the next
    //     run HITs. We pay the provider codegen ONLY when there's no cached provider to reuse.
    // The KEY is v-rust-backend's canonical `closure_content_hash`; #1502's `consumer_only_emits_the_closure_
    // hash_sidecar` locks that ConsumerOnly's hash EQUALS Composed's, so the HIT decision (from ConsumerOnly)
    // and the persisted key (from Composed) agree by construction.
    //
    // Why ConsumerOnly-first (not the prior single-`EmitTestsComposed`): Composed ALWAYS emits the provider —
    // paying its ~215s codegen even on a HIT, then discarding it. ConsumerOnly's mono+layout is the ~15s floor
    // (measured: `cdz func-layout`), with only thin consumer codegen — so a HIT collapses ~230s→~20s. The
    // trade is a MISS now pays TWO monos (ConsumerOnly ~15s + Composed ~230s ≈ 245s vs single-Composed's
    // ~230s) — a negligible regression on the RARE miss (only the first warm, or when the closure content
    // changes) for a large win on the COMMON hit (every re-gate against a stable closure). NOTE the hash is
    // over THIS GROUP's closure only — grouping shrinks each provider AND scopes each cache entry to one
    // closure (a lib change busts only the groups whose closure includes it).
    let consumer_out = drive(cadenza_compile_abi::Request::EmitTestsConsumerOnly);
    let closure_hash = consumer_out
        .artifact(cadenza_compile_abi::sidecar::KIND_CLOSURE_HASH)
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .filter(|h| !h.is_empty());

    // OBSERVABILITY (`CDZ_PROVIDER_CACHE_TRACE` = any non-empty value): emit ONE line to stderr PER GROUP
    // recording the cache decision + closure key, so a caller can VERIFY a run warmed/hit the cache (which
    // group's provider persisted vs was reused) and a test can distinguish a HIT from the standalone fallback.
    // Off by default → zero output on the normal path; peer of `CDZ_WASM_BACKTRACE` / `CDZ_DUMP_TEST_WASM`.
    let trace = |ev: &str| {
        if std::env::var("CDZ_PROVIDER_CACHE_TRACE").is_ok_and(|v| !v.trim().is_empty()) {
            let key = closure_hash.as_deref().unwrap_or("<no-hash>");
            let dir = cache_dir
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "<no-cache-dir>".into());
            eprintln!("[provider-cache] {ev} key={key} dir={dir}");
        }
    };

    // A VALIDATED cached provider for this key, if one exists. VALIDATE the bytes compile BEFORE trusting the
    // hit path: a truncated / corrupt / stale-format cache file must NOT break `cdz test` (it would surface
    // later as an opaque per-file "invalid peer component" compile error). If it doesn't compile, discard it →
    // treat as a MISS (emit + re-persist via the Composed drive below), which self-heals the bad entry.
    let cached_provider = closure_hash
        .as_ref()
        .and_then(|h| cache_dir.map(|d| d.join(format!("{h}.provider.wasm"))))
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::read(&p).ok())
        .filter(|bytes| cdz_run::compile_component(bytes).is_ok());

    // Decide the peer provider AND which drive's output supplies the consumers/iface:
    //   HIT   → cached provider + the ConsumerOnly consumers (no Composed drive: skips the provider codegen).
    //   DECLINE (no hash) → ConsumerOnly emitted no shared-closure hash; a Composed drive would re-mono and
    //           re-decline, so DON'T drive it — fall back per-file (peer stays None below).
    //   MISS  → drive Composed to emit the provider, persist it, and use ITS output (provider + consumers are
    //           generated together, guaranteed consistent).
    let (provider, out) = if let Some(cached) = cached_provider {
        trace("hit");
        (Some(cached), consumer_out)
    } else if closure_hash.is_none() {
        trace("decline no-shared-closure");
        (None, consumer_out)
    } else {
        // MISS: emit the provider via Composed (the only drive that emits it), then persist by the hash.
        let composed_out = drive(cadenza_compile_abi::Request::EmitTestsComposed);
        let emitted_provider = composed_out
            .artifacts
            .iter()
            .find(|a| a.kind == "component-provider")
            .map(|p| p.bytes.clone());
        if let (Some(bytes), Some(dir), Some(key)) = (&emitted_provider, cache_dir, &closure_hash) {
            // Best-effort ATOMIC persist: write a pid-stamped temp in the SAME dir, then rename onto the
            // content-addressed key — rename is atomic on POSIX, so a reader (incl. a CONCURRENT `cdz test`)
            // never sees a partial file at the key, and a crash mid-write leaves only the temp (never a
            // truncated file at the key that a later run would HIT as corrupt). A write/rename FAILURE
            // (full/RO FS) just means the next run re-emits — no correctness impact.
            let _ = std::fs::create_dir_all(dir);
            let final_path = dir.join(format!("{key}.provider.wasm"));
            let tmp = dir.join(format!(".{key}.provider.wasm.{}.tmp", std::process::id()));
            if std::fs::write(&tmp, bytes).is_err() {
                let _ = std::fs::remove_file(&tmp);
                trace("miss no-persist(write-failed)");
            } else if std::fs::rename(&tmp, &final_path).is_err() {
                // Rename failed. On POSIX rare (a real FS error). On WINDOWS `rename` fails when the dest
                // EXISTS — the self-heal case (a corrupt {key} present) where the corrupt file must NOT
                // survive: best-effort remove the dest + retry once; if that also fails, drop the temp.
                let _ = std::fs::remove_file(&final_path);
                if std::fs::rename(&tmp, &final_path).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                    trace("miss no-persist(rename-failed)");
                } else {
                    trace("miss persisted");
                }
            } else {
                trace("miss persisted");
            }
        } else {
            // MISS but nothing to persist (no cache dir, no emitted provider, or no key) — still use whatever
            // provider was emitted (may be `None` → the group falls back per-file below).
            trace(if emitted_provider.is_none() {
                "miss no-persist(no-provider)"
            } else {
                "miss no-persist(no-key-or-dir)"
            });
        }
        (emitted_provider, composed_out)
    };

    // Demux: the `component-name` sidecar carries the provider's interface string; the N `component` artifacts
    // are the per-file consumers (named by file-link). A DECLINE (ill-typed @test, or an un-representable
    // higher-order cross-edge in THIS group's union) yields no provider/consumers → this group's files fall
    // back to their own per-file `EmitTests` (which re-surfaces any fault located; we do NOT report here).
    let iface = out
        .artifacts
        .iter()
        .find(|a| a.kind == "component-name")
        .map(|a| String::from_utf8_lossy(&a.bytes).into_owned());
    let consumers = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component")
        .map(|a| (a.name.clone(), a.bytes.clone()))
        .collect();
    // Pair the provider with its interface name only when BOTH are present — a consumer can only be linked
    // against a peer we can name; else the group's files fall back per-file (safe degrade). Carry the closure
    // CONTENT HASH so a JIT-artifact (cwasm) cache can key on it (content-addressed, not the import-name group
    // key) — the cwasm must invalidate when the closure content changes even if the import set doesn't.
    let peer = provider
        .zip(iface)
        .map(|(bytes, iface)| (bytes, iface, closure_hash.clone()));
    (peer, consumers)
}

#[cfg(feature = "standalone")]
pub(crate) fn precompile_tests_per_file(files: &[String]) -> Precompiled {
    use std::collections::HashMap;
    if files.is_empty() {
        return Precompiled::default();
    }
    // NOTE (was `files.len() < 2`): the composed path serves TWO wins, and a single target file benefits from
    // ONE of them. (i) BATCH amortization — lower the shared closure once across N files in this invocation —
    // is genuinely N/A for one file. (ii) The CROSS-INVOCATION PROVIDER CACHE — persist the shared-closure
    // provider so a LATER `cdz test <that file>` is a consumer-only HIT that skips the ~381s closure lower — is
    // exactly the single-file-local-verify win, and it applies whenever ONE file imports a big shared closure
    // (v-compiler-ml verifying a witness against the ~1360-def self-host closure). So we no longer blanket-skip
    // on a single file; the real "nothing to do here" test is whether the closure UNION has a cross-file member
    // — checked below (`asts.len() < 2`) AFTER we gather it, since a lone SELF-CONTAINED file (no imports) has
    // no provider to hoist or cache and must stay on its byte-identical per-file compile.
    // CORRECTNESS GATE (PR#881): a closure file's link name is its dir-BLIND STEM (`program_name` =
    // file_stem). The union below dedups by that stem AND `run_test_file` looks its component up by the same
    // stem — so two DIFFERENT-directory target files with the SAME stem (e.g. two `t.cdz`, or a `lib.cdz` in
    // each of two subdirs) would collapse to one AST and a lookup could fetch the WRONG dir's component,
    // MISATTRIBUTING pass/fail (the best-effort fallback only fires on an ABSENT component, not a
    // present-but-wrong one). So only take the shared-precompile fast path when every target file shares ONE
    // parent directory — then a shared stem means genuinely the same file, and the stem key is unambiguous.
    // Otherwise return empty ⇒ every file falls back to its own per-file compile (correct, just not amortized).
    // `cdz test <dir>` (recursive) is the multi-dir case this guards; a flat single-dir suite keeps the win.
    let parent_of = |p: &str| {
        std::path::Path::new(p)
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_default()
    };
    let first_dir = parent_of(&files[0]);
    if files.iter().any(|f| parent_of(f) != first_dir) {
        return Precompiled::default();
    }
    // GROUP the target files by their genuine SHARED CLOSURE, then compose ONE provider per group (Option-A).
    // WHY NOT one union over all files: a `cdz test <dir>` over a HETEROGENEOUS tree (e.g. compiler-ml/src, 44
    // files importing ~20 distinct libs: parse-db, db, sread-eval, infer-db, …) would fold every file's
    // cross-edges into ONE provider ≈ the whole compiler — the heaviest possible emit, and one un-representable
    // higher-order cross-edge ANYWHERE in that union declines the WHOLE dir to per-file. Grouping by shared
    // closure keeps each provider SMALL + homogeneous (the 9 `sread-eval-*` files → one sread-eval provider; a
    // `conformance-db` file → another) and DECLINE-ISOLATED (a decline drops only its group). It also scopes
    // each cache entry to one closure (a lib change busts only the groups whose closure includes it).
    //
    // The GROUP KEY is the file's IMPORTED-closure name-set (its transitive-closure link names MINUS itself) —
    // computed free from the closure we already load. Keyed by SET EQUALITY, NOT overlap: equality does not
    // re-collapse on a near-universal base (`db` is in almost every closure, so overlap-grouping would merge
    // everything back into one union — the exact defect we're fixing), while equality groups genuinely-identical
    // closures (the homogeneous families the composed path handles). A file with an EMPTY imported set (a
    // self-contained file, no shared closure to hoist) is dropped from grouping → it falls back to standalone.
    let cache_dir = provider_cache_dir();
    // group key (sorted, `\0`-joined imported-closure names) → (union ASTs by link name, an entry name, the
    // TARGET file stems bucketed into this group).
    struct Group {
        asts: HashMap<String, cadenza_compile_abi::Artifact>,
        entry: String,
        // The stems of the TARGET `@test` files that fell into THIS group (their `closure[0].name`). A group's
        // composed emit produces a consumer for EVERY closure member that has `@test`s — but an
        // imported-with-tests member (e.g. `parse-db`: imported into ~10 groups' closures AND a target of its
        // OWN group) is a target of only ONE group. We store its consumer ONLY from its own group (below), so a
        // stem's consumer is never overwritten by a group where it's merely an imported member linked against
        // the WRONG provider (PR#914 correctness — the grouping-era cousin of the PR#881 stem collision).
        targets: std::collections::HashSet<String>,
    }
    // For a SINGLE-file run, keep the loaded closure to SHARE with `run_test_file` (it would otherwise re-load
    // + re-parse the same file — PR#907). Only the single-file case: stashing all N of a dir run would raise
    // peak memory (holding every file's closure at once) for no gain — a dir file loads its closure once in
    // `run_test_file` regardless (it never reuses a sibling's).
    let single = files.len() == 1;
    let mut single_file_closure: Option<std::rc::Rc<Vec<closure::LoadedFile>>> = None;
    let mut groups: HashMap<String, Group> = HashMap::new();
    for f in files {
        let Ok(closure) = load_import_closure_with(f, &|_| None) else {
            continue; // a file that fails to load falls back to its own compile (reports the error located)
        };
        // The entry (element 0) is the target file; the rest are its imported closure. A file with NO imported
        // siblings is self-contained — no provider to hoist/cache, no per-file emit to amortize — so it keeps
        // its byte-identical standalone `EmitTests` path (not grouped). We still stash it (single-file case)
        // so `run_test_file` reuses the parse.
        if closure.len() >= 2 {
            let mut imported: Vec<String> = closure[1..].iter().map(|cf| cf.name.clone()).collect();
            imported.sort();
            imported.dedup();
            let key = imported.join("\0");
            let group = groups.entry(key).or_insert_with(|| Group {
                asts: HashMap::new(),
                entry: closure[0].name.clone(),
                targets: std::collections::HashSet::new(),
            });
            // This file is a TARGET of this group (its @tests are what we run here). Record its stem so we keep
            // only ITS-group consumer, not a same-stem consumer emitted as an imported member of another group.
            group.targets.insert(closure[0].name.clone());
            for cf in &closure {
                group.asts.entry(cf.name.clone()).or_insert_with(|| {
                    cadenza_compile_abi::Artifact::new(
                        cadenza_compile_abi::Artifact::KIND_AST,
                        cf.name.clone(),
                        cadenza_syntax::codec::encode(&cf.arenas),
                    )
                });
            }
        }
        // Stash the single-file closure for reuse (after building any group above — the closure is still owned
        // here; the group only borrowed it). A self-contained single file (`closure.len() < 2`) is stashed too:
        // it skips grouping but `run_test_file` still reuses the parse.
        if single {
            single_file_closure = Some(std::rc::Rc::new(closure));
        }
    }
    if groups.is_empty() {
        // Nothing to compose — but a single-file run still hands its stashed closure to `run_test_file` so the
        // parse isn't repeated (a self-contained single file, or a single importing file whose group declined).
        return Precompiled {
            single_file_closure,
            ..Precompiled::default()
        };
    }

    // Compose each group independently. A group whose composed emit declines contributes no provider/consumers
    // (its files fall back per-file); the others still get their shared provider. Each consumer records WHICH
    // group provider it links against, so `run_test_file` binds it to the right peer.
    let mut precompiled = Precompiled {
        single_file_closure,
        ..Precompiled::default()
    };
    for (key, group) in groups {
        let targets = group.targets;
        let ast_inputs: Vec<cadenza_compile_abi::Artifact> = group.asts.into_values().collect();
        let (provider, consumers) =
            precompile_group(ast_inputs, &group.entry, cache_dir.as_deref());
        let Some(provider) = provider else {
            continue; // group declined / no nameable provider → its files fall back standalone
        };
        precompiled.providers.insert(key.clone(), provider);
        for (name, bytes) in consumers {
            // Keep ONLY consumers for files that are TARGETS of this group. The composed emit produces a
            // consumer for every closure member that has `@test`s, but an imported-with-tests member (e.g.
            // `parse-db`) is a target of just ONE group; storing its consumer from a group where it's only an
            // imported member would OVERWRITE (last-group-wins) its own-group consumer → `run_test_file` links
            // it against the wrong group's provider (PR#914). Filtering by target keeps each stem's consumer
            // from its own group, keyed to the provider whose closure it was actually emitted against.
            if targets.contains(&name) {
                precompiled.components.insert(name, (bytes, key.clone()));
            }
        }
    }
    precompiled
}

/// The directory the shared-closure PROVIDER components are cached in, content-addressed by the closure hash
/// — `$CDZ_PROVIDER_CACHE` if set (and non-empty), else `<default-store>/providers` (the store is already the
/// per-checkout content-addressed artifact dir). Reusing the store dir keeps the cache co-located with the
/// runtime it pairs with + swept by the same tooling. Returns `Option` (the call site degrades to "no cache"
/// on `None`) so a future store-resolution failure can opt out cleanly; today it always resolves to `Some`
/// (`default_store` is infallible), so caching is always available — a write failure is the actual degrade path.
#[cfg(feature = "standalone")]
pub(crate) fn provider_cache_dir() -> Option<std::path::PathBuf> {
    if let Ok(d) = std::env::var("CDZ_PROVIDER_CACHE") {
        let d = d.trim();
        if !d.is_empty() {
            return Some(std::path::PathBuf::from(d));
        }
    }
    Some(default_store().join("providers"))
}

/// Enumerate the resolved suite's `@test` definitions as a CADENZA-AST-BINARY value and return — the body
/// of `cdz test --list`. WASMTIME-FREE by construction: it loads each file's import closure, builds the
/// compiler `Db`, and reads `db.test_defs()` — the same front-half `cdz test` runs BEFORE any wasm emit/JIT
/// (`run_test_file`'s enumeration head), stopping there. It compiles nothing and links no runtime, so a
/// `--no-default-features` `cdz` (no `cdz-run`) still produces it.
///
/// Output: the SAME `(test-list (test <name> <is-property> <file>)…)` cadenza-ast value the DELEGATE path
/// (`rcdzc::sidecar` `Query::TestList`, `KIND_TEST_LIST`) emits — one `(test …)` child per test, POSITIONAL:
/// `name` (`Str`), `is-property` (`Bool`), `file` (`Str`) — `codec::encode`d and written verbatim to stdout.
/// This is the operator cadenza-ast-binary-everywhere directive (NO JSON) and keeps `--list` FORMAT-IDENTICAL
/// across the `standalone` (this in-process path) and delegate builds, so v-nix's dynamic-derivations
/// discovery decodes ONE format with the shared `codec` regardless of which `cdz` it invokes. The names come
/// from the `Db`, NOT a regex (the compiler's own source carries `@test` as a parsed token — a regex would
/// massively over-count, per v-test-shred). `is-property` is `!def.params.is_empty() || name.ends_with("-gen")`
/// — a `@test` taking parameters (or the `Test.gen` property wrapper) is a property test; a nullary one is a
/// plain unit test (matches the delegate path's `compile_tests` classification exactly).
///
/// Enumeration mirrors `run_test_file` exactly: a PACKAGE (a file that declares imports) links its whole
/// closure and keeps only the ENTRY file's own `@test`s (an imported library's tests belong to THAT file,
/// counted when it is itself the entry — a directory run visits each); a lone file decodes directly. Dedup
/// is PER FILE (`seen`), matching the run. Order is the resolved-`files` order (path-sorted / manifest
/// order) then declaration order — deterministic, so a drift-guard comparing a fresh `--list` to a
/// committed one is stable. Ignores `--filter`/`--tag`: a manifest must enumerate the WHOLE suite.
#[cfg(feature = "standalone")]
pub(crate) fn list_tests(files: &[String], format: ListFormat) -> ExitCode {
    match format {
        // DEFAULT: the canonical cadenza-ast-BINARY `(test-list …)` value, written VERBATIM to stdout (the
        // delegate path's `Query::TestList` bytes are likewise raw; consumers decode with the shared `codec`).
        ListFormat::Binary => match list_test_bytes(files) {
            Ok(bytes) => {
                use std::io::Write as _;
                match std::io::stdout().write_all(&bytes) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("{PROG}: --list: could not write the test-list: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(code) => code,
        },
        // `--format nix`: the eval-readable nix attrset list (v-nix's scoped-cached-IFD discovery source),
        // printed to stdout (the discovery drv redirects to `$out`, a single `import`-able file).
        ListFormat::Nix => match collect_test_entries(files) {
            Ok(entries) => {
                print!("{}", list_test_nix(entries));
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
    }
}

/// Enumerate the resolved suite's `@test`s and return the `codec::encode`d `(test-list (test <name>
/// <is-property> <file>)…)` cadenza-ast value (the enumeration half of [`list_tests`], factored out so it
/// is unit-testable without capturing stdout). `Err(ExitCode::FAILURE)` on a load/decode/link fault (a
/// broken project cannot be honestly enumerated — failing red is what the drift-guard wants).
#[cfg(feature = "standalone")]
pub(crate) fn list_test_bytes(files: &[String]) -> Result<Vec<u8>, ExitCode> {
    // Both `--list` projections (cadenza-ast-binary + `--format nix`) share ONE enumeration; the binary form
    // encodes each collected `(name, is_property, file)` as a `(test …)` child of `(test-list …)`.
    let entries = collect_test_entries(files)?;
    let mut b = cadenza_syntax::Builder::new();
    let mut children: Vec<cadenza_syntax::StructId> = Vec::with_capacity(entries.len() + 1);
    children.push(b.name("test-list"));
    for (name, is_property, file) in &entries {
        let head = b.name("test");
        let name_n = b.atom_leaf(cadenza_syntax::Leaf::Str(name.as_str().into()));
        let isprop_n = b.atom_leaf(cadenza_syntax::Leaf::Bool(*is_property));
        let file_n = b.atom_leaf(cadenza_syntax::Leaf::Str(file.as_str().into()));
        children.push(b.list(vec![head, name_n, isprop_n, file_n]));
    }
    let root = b.list(children);
    Ok(cadenza_syntax::codec::encode(&b.finish(root)))
}

/// A nix STRING literal for `s` — quotes + escapes `"`, `\`, a `${` antiquotation opener, and newlines, so
/// a `@test` name or source path with a special char can't break the emitted (and `import`-ed) nix.
#[cfg(feature = "standalone")]
pub(crate) fn nix_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `--list --format nix`: a PURE, IFD-cache-stable nix attrset list — `[ { name = "…"; is_property = …;
/// file = "…"; } … ]` — the eval-readable projection v-nix's scoped-cached-IFD discovery derivation writes to
/// `$out` and the flake `import`s. SORTED by `(file, name)` so an identical `@test` set yields BYTE-IDENTICAL
/// output (the discovery drv is then content-stable — eval re-reads only on a real test add/remove, not
/// ordering noise). Attr names (`name`/`is_property`/`file`) match the emit-shred manifest so the fan-out's
/// `(file-stem, name)` join is clean. Pure: no timestamps/hashed paths, only the enumerated fields.
#[cfg(feature = "standalone")]
pub(crate) fn list_test_nix(mut entries: Vec<(String, bool, String)>) -> String {
    entries.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)));
    let mut s = String::from("[\n");
    for (name, is_property, file) in &entries {
        s.push_str(&format!(
            "  {{ name = {}; is_property = {is_property}; file = {}; }}\n",
            nix_str(name),
            nix_str(file),
        ));
    }
    s.push_str("]\n");
    s
}

/// Enumerate the resolved suite's `@test`s as owned `(name, is_property, file)` tuples — the walk shared by
/// [`list_test_bytes`] (cadenza-ast-binary) and [`list_test_nix`]. Same semantics as [`list_tests`]: follow
/// each file's import closure, build the compiler `Db`, keep only the ENTRY file's own `@test`s in a package
/// (byte-for-byte `run_test_file`'s filter), dedup per file. `is_property` = `!params.is_empty() ||
/// name.ends_with("-gen")` (the delegate `compile_tests` classification). Wasmtime-free.
#[cfg(feature = "standalone")]
pub(crate) fn collect_test_entries(
    files: &[String],
) -> Result<Vec<(String, bool, String)>, ExitCode> {
    let mut entries: Vec<(String, bool, String)> = Vec::new();
    for file in files {
        // Follow the file's import closure — the SAME linked program `cdz test`/`cdz check` sees, so a test
        // in a module that imports a sibling enumerates against the same package. A load error is FATAL for
        // `--list` (a broken project cannot be honestly enumerated; failing red is what the drift-guard wants).
        let closure = match load_import_closure_with(file, &|_| None) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return Err(ExitCode::FAILURE);
            }
        };
        let is_package = !declared_import_paths(&closure[0].arenas).is_empty();
        // Encode each closure file's AST to the canonical binary form (the front-end↔compiler bridge), then
        // build the `Db` the enumeration reads — a package links every file into one arena + loads it WITH
        // its linkage (so `file_of` can scope tests to the entry); a lone file decodes directly.
        let ast_arts: Vec<cadenza_compile_abi::Artifact> = closure
            .iter()
            .map(|f| {
                cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        let (db, entry_filter) = if is_package {
            let mut rcdzc_files = Vec::with_capacity(ast_arts.len());
            for art in &ast_arts {
                let Some(a) = cadenza_syntax::codec::decode(&art.bytes) else {
                    eprintln!("{PROG}: {file}: could not decode `{}`'s AST", art.name);
                    return Err(ExitCode::FAILURE);
                };
                rcdzc_files.push((art.name.clone(), a));
            }
            let program = match rcdzc::link::link(&rcdzc_files, &closure[0].name) {
                Ok(p) => p,
                Err(r) => {
                    eprintln!("{PROG}: {file}: {}", r.message);
                    return Err(ExitCode::FAILURE);
                }
            };
            let linkage = program.linkage();
            let entry_ix = program.entry;
            let db = rcdzc::db::Db::load_linked(program.arenas, Some(linkage.clone()));
            (db, Some((linkage, entry_ix)))
        } else {
            let Some(rcdzc_arenas) = cadenza_syntax::codec::decode(&ast_arts[0].bytes) else {
                eprintln!("{PROG}: {file}: could not decode the program's AST");
                return Err(ExitCode::FAILURE);
            };
            (rcdzc::db::Db::load(rcdzc_arenas), None)
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for i in db.test_defs() {
            // In a PACKAGE, `test_defs()` sees every linked file's `@test`s — keep only the ENTRY file's own
            // (an imported library's tests are enumerated when it is itself the entry), byte-for-byte the
            // `run_test_file` filter, so `--list` and the run enumerate the identical set.
            if let Some((linkage, entry_ix)) = &entry_filter {
                match linkage.file_of(db.defs[i].sig_occ) {
                    Some(fi) if fi == *entry_ix => {}
                    _ => continue,
                }
            }
            let name = db.defs[i].name.clone();
            if !seen.insert(name.clone()) {
                continue;
            }
            // A `@test` taking parameters (or the `Test.gen` `-gen` property wrapper) is a PROPERTY test (run
            // over generated inputs); a nullary one is a plain unit test. This matches the delegate path's
            // `compile_tests` classification EXACTLY, so `--list` agrees across both builds.
            let is_property = !db.defs[i].params.is_empty() || name.ends_with("-gen");
            entries.push((name, is_property, file.clone()));
        }
    }
    Ok(entries)
}

/// `cdz test --emit-shred` — the compiler-driven test SHRED (the operator model), the body behind the flag.
/// Drives the `EmitTestsShred` sidecar IN-PROCESS (linked `rcdzc`, the same in-process compile the `cdz test`
/// runner uses — no wasmtime, no cdz-run) PER PROJECT FILE (each its own shared-closure GROUP: a multi-file
/// project is NOT one linkable program — independent files don't share an entry, and packages are DAGs), and
/// writes a single FLAT `out_dir/`: `main-<group>.wasm` (each group's emitted library, when it has one) +
/// `test-<name>.wasm` (the per-`@test` components, flat) + ONE `manifest.cdzb` (the merged cadenza-ast-binary
/// manifest). Each group's per-program manifest carries `main-file` = "main.wasm" (has-lib) or "" (standalone);
/// here we REWRITE it to this group's real `main-<group>.wasm` (or keep "" for standalone) and MERGE all
/// groups' entries into the one manifest a runner reads (`cdz-run <target> --call <export> [--peer
/// <main-iface>=<main-file>] --store S`). Compile-only; exits non-zero if any file fails to compile.
#[cfg(feature = "standalone")]
pub(crate) fn run_emit_shred(
    files: &[String],
    out_dir: &std::path::Path,
    standalone: bool,
    two_stage: bool,
) -> ExitCode {
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        eprintln!(
            "{PROG}: --emit-shred: cannot create {}: {e}",
            out_dir.display()
        );
        return ExitCode::FAILURE;
    }
    // Mode selection (§S6b): TWO-STAGE (`--two-stage`) emits cadenza-ast FRAGMENTS — one shared-closure
    // `closure-<i>.cdzb` + one per-`@test` `test-<name>.cdzb` — spliced+compiled LATER by the fan-out
    // (`rcdzc closure.cdzb test.cdzb --export <name>`), for standalone-everywhere heavy suites without the
    // O(tests×closure) blowup. STANDALONE (`--standalone`) emits each `@test` as a self-contained WASM
    // component (NO main). Else the shared-main peer WASM shred. `--two-stage` wins if both are set.
    let shred_req = if two_stage {
        cadenza_compile_abi::Request::EmitTestsShredTwoStage
    } else if standalone {
        cadenza_compile_abi::Request::EmitTestsShredStandalone
    } else {
        cadenza_compile_abi::Request::EmitTestsShred
    };
    // The shared-artifact file EXTENSION + per-test target extension: two-stage writes cadenza-ast fragments
    // (`.cdzb`), the wasm modes write components (`.wasm`).
    let ext = if two_stage { "cdzb" } else { "wasm" };
    // The merged manifest's entries, collected across groups as owned fields (each group's arena is dropped
    // before the next): (name, is_property, file, export, target, main-iface, main-file).
    let mut all_entries: Vec<(String, bool, String, String, String, String, String)> = Vec::new();
    // Target FILE basenames already written — so a `@test` name that repeats across files (e.g. choreography's
    // ~3) gets a UNIQUE target file (disambiguated `-<group>`), never overwriting a sibling (the flat layout's
    // one requirement, v-test-shred). The manifest `target` field is rewritten to the unique name it reads.
    let mut written_targets: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut any_fail = false;
    for (i, file) in files.iter().enumerate() {
        // GROUP = one project file + its import closure. Load it, encode each closure file's AST, drive
        // `EmitTestsShred` in-process (link + emit over this group's linked program). A file's closure is its
        // own group; a standalone file (no imports) is a lone-file group (→ possibly no main).
        let closure = match load_import_closure_with(file, &|_| None) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{PROG}: {e}");
                any_fail = true;
                continue;
            }
        };
        let mut inputs: Vec<cadenza_compile_abi::Artifact> = closure
            .iter()
            .map(|f| {
                cadenza_compile_abi::Artifact::new(
                    cadenza_compile_abi::Artifact::KIND_AST,
                    f.name.clone(),
                    cadenza_syntax::codec::encode(&f.arenas),
                )
            })
            .collect();
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(std::slice::from_ref(&shred_req)),
        ));
        inputs.push(cadenza_compile_abi::abi::entry_artifact(&closure[0].name));
        let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
        // A per-`@test` DECLINE (a compound/closure-param test that can't cross the peer boundary — the
        // deferred #4031 limit) is error-severity, but it is INFORMATIONAL for the shred, NOT a failure: the
        // compile still emits the SHREDDABLE tests + a manifest listing them (the runner runs what shredded +
        // skips the rest). So report the diagnostics (so a decline is visible) but do NOT fail the run or SKIP
        // the file — proceed to take its shreddable output (a file with 3 ok + 2 declined tests still
        // contributes its 3, rather than being dropped whole). `--emit-shred` exits 0 whenever it writes a
        // manifest; only a HARD I/O failure (below) fails it. (`--standalone` has no peer boundary → no
        // declines → this is a clean full shred.)
        if out.has_error() {
            report_errors(&out);
        }
        // ENTRY-SCOPE by the manifest `file` field. A PACKAGE's linked program enumerates EVERY linked file's
        // `@test`s (not just the entry's) — so without this, each file re-emits the WHOLE package's tests
        // (cad: 996 entries for 138 real tests). Each `@test` belongs to its OWN source file's group (emitted
        // when THAT file is the entry), so keep only entries whose `file` == this entry file's stem, and write
        // only those tests' components. An independent-file suite (iterators) has file == entry_stem for all
        // (its closure is just itself), so nothing is dropped there.
        let entry_stem = closure[0].name.clone();
        // The group's SHARED artifact: two-stage → the `closure` ast fragment (→ `closure-<i>.cdzb`); the
        // wasm modes → the `component-provider` main (→ `main-<i>.wasm`). Empty when the group has none (a
        // standalone wasm shred, or a two-stage suite whose closure declined).
        let has_main = if two_stage {
            out.artifacts
                .iter()
                .any(|a| a.kind == cadenza_compile_abi::Artifact::KIND_AST && a.name == "closure")
        } else {
            out.artifacts.iter().any(|a| a.kind == "component-provider")
        };
        let group_main_file = if !has_main {
            String::new()
        } else if two_stage {
            format!("closure-{i}.cdzb")
        } else {
            format!("main-{i}.wasm")
        };
        // Decode the group's manifest → the OWN `@test`s (this file's own, by the `file` field). For each, pick
        // a UNIQUE target FILE name (disambiguate a cross-file name collision with `-<group>`), map the
        // rcdzc consumer artifact name (`test-<name>`) → that unique file, and push the entry with `target`
        // rewritten to it (+ `main-file` → this group's real main / "" standalone). `own` drives the writes.
        let mut own: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(m) = out
            .artifacts
            .iter()
            .find(|a| a.kind == cadenza_compile_abi::sidecar::KIND_SHRED_MANIFEST)
        {
            let Some(arenas) = cadenza_syntax::codec::decode(&m.bytes) else {
                eprintln!("{PROG}: --emit-shred: could not decode {file}'s shred manifest");
                any_fail = true;
                continue;
            };
            if let Some(entries) = arenas.as_form(arenas.root, "shred-manifest") {
                for &e in entries {
                    let Some(f) = arenas.as_form(e, "entry") else {
                        continue;
                    };
                    if f.len() != 7 {
                        continue;
                    }
                    let name = arenas.as_str(f[0]).unwrap_or("").to_string();
                    let test_file = arenas.as_str(f[2]).unwrap_or("");
                    if test_file != entry_stem {
                        continue; // an imported file's @test — its OWN group emits it (no cross-file dup)
                    }
                    // Unique target file: `test-<name>.<ext>`, else `test-<name>-<group>.<ext>` on a
                    // cross-file name collision (group index is unique, and within a group `@test` names are
                    // unique). `<ext>` = `cdzb` (two-stage fragment) or `wasm` (compiled component).
                    let mut target = format!("test-{name}.{ext}");
                    if written_targets.contains(&target) {
                        target = format!("test-{name}-{i}.{ext}");
                    }
                    written_targets.insert(target.clone());
                    own.insert(format!("test-{name}"), target.clone());
                    all_entries.push((
                        name,
                        arenas.as_bool(f[1]).unwrap_or(false),
                        test_file.to_string(),
                        arenas.as_str(f[3]).unwrap_or("").to_string(),
                        target,
                        arenas.as_str(f[5]).unwrap_or("").to_string(),
                        group_main_file.clone(),
                    ));
                }
            }
        }
        // Write the group's SHARED artifact (the closure fragment / main provider — only when this file HAS
        // own tests that link it, else it is an orphan) + the OWN per-`@test` artifacts, each to its UNIQUE
        // target file (from `own`). Two-stage artifacts are kind `ast` (`closure` + `test-<name>` fragments);
        // the wasm modes are `component-provider` (main) + `component` (per-test consumer).
        let write_to = |rel: &str, bytes: &[u8], any_fail: &mut bool| {
            let p = out_dir.join(rel);
            if let Err(e) = std::fs::write(&p, bytes) {
                eprintln!("{PROG}: --emit-shred: cannot write {}: {e}", p.display());
                *any_fail = true;
            }
        };
        for a in &out.artifacts {
            if two_stage {
                if a.kind != cadenza_compile_abi::Artifact::KIND_AST {
                    continue;
                }
                if a.name == "closure" {
                    if !own.is_empty() {
                        write_to(&group_main_file, &a.bytes, &mut any_fail);
                    }
                } else if let Some(target) = own.get(&a.name) {
                    write_to(target, &a.bytes, &mut any_fail);
                }
            } else {
                match a.kind.as_str() {
                    "component-provider" if !own.is_empty() => {
                        write_to(&group_main_file, &a.bytes, &mut any_fail)
                    }
                    "component" => {
                        if let Some(target) = own.get(&a.name) {
                            write_to(target, &a.bytes, &mut any_fail)
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // The MERGED manifest — ONE `(shred-manifest (entry name is-property file export target main-iface
    // main-file)…)` across all groups, `codec::encode`d (the cadenza-ast-binary tooling format).
    let mut b = cadenza_syntax::Builder::new();
    let mut children: Vec<cadenza_syntax::StructId> = Vec::with_capacity(all_entries.len() + 1);
    children.push(b.name("shred-manifest"));
    for (name, is_prop, file, export, target, iface, main_file) in &all_entries {
        let head = b.name("entry");
        let name_n = b.atom_leaf(cadenza_syntax::Leaf::Str(name.as_str().into()));
        let isprop_n = b.atom_leaf(cadenza_syntax::Leaf::Bool(*is_prop));
        let file_n = b.atom_leaf(cadenza_syntax::Leaf::Str(file.as_str().into()));
        let export_n = b.atom_leaf(cadenza_syntax::Leaf::Str(export.as_str().into()));
        let target_n = b.atom_leaf(cadenza_syntax::Leaf::Str(target.as_str().into()));
        let iface_n = b.atom_leaf(cadenza_syntax::Leaf::Str(iface.as_str().into()));
        let mainfile_n = b.atom_leaf(cadenza_syntax::Leaf::Str(main_file.as_str().into()));
        children.push(b.list(vec![
            head, name_n, isprop_n, file_n, export_n, target_n, iface_n, mainfile_n,
        ]));
    }
    let root = b.list(children);
    let manifest_path = out_dir.join("manifest.cdzb");
    if let Err(e) = std::fs::write(
        &manifest_path,
        cadenza_syntax::codec::encode(&b.finish(root)),
    ) {
        eprintln!(
            "{PROG}: --emit-shred: cannot write {}: {e}",
            manifest_path.display()
        );
        any_fail = true;
    }
    if any_fail {
        ExitCode::FAILURE
    } else {
        eprintln!(
            "cdz: shredded {} test(s) into {}",
            all_entries.len(),
            out_dir.display()
        );
        ExitCode::SUCCESS
    }
}

// `cdz test` runs the compiler + property-generator IN-PROCESS (it needs `rcdzc`: type-directed input
// gen over a live `Db`, emit-shred compiles), so the whole test runner is `standalone`-only. A
// `!standalone` (thin-dispatcher) build has no in-process runner — CI runs tests via the nix per-@test
// shred matrix on the default-features seedCompiler, and devs run `cdz test` on a default-features build
// (v-test-shred confirmed). The stub errors honestly (NON-ZERO) rather than exiting 0. A future external
// `cdz-test` bin would delegate here.
#[cfg(not(feature = "standalone"))]
pub(crate) fn run_test(_args: &TestArgs) -> ExitCode {
    eprintln!(
        "{PROG}: `cdz test` requires the bundled compiler (rcdzc); this --no-default-features build has \
         no in-process test runner. Run tests via the nix per-@test shred matrix (CI) or a \
         default-features `cdz test` build (dev)."
    );
    ExitCode::FAILURE
}

#[cfg(feature = "standalone")]
pub(crate) fn run_test(args: &TestArgs) -> ExitCode {
    // Resolve WHICH files to run. Cases:
    //  - NO arg → search UP from the current directory for the nearest `Project.cdz` (like `cargo test`
    //    finding `Cargo.toml`) and run its suite;
    //  - a `Project.cdz` (or a directory holding one): run the manifest's `tests` list — the project
    //    TELLS us its suite (the Cadenza-authored manifest, no per-run flags);
    //  - a directory with NO manifest: run every source file's `@test`s (path-sorted walk);
    //  - a single file: the one-file case.
    let target: String = match &args.file {
        Some(f) => f.clone(),
        None => match find_manifest_upward() {
            Some(p) => p.to_string_lossy().into_owned(),
            None => {
                eprintln!(
                    "{PROG}: no `{MANIFEST_NAME}` found in the current directory or any ancestor \
                     (name a file/dir to test, or add a `{MANIFEST_NAME}`)"
                );
                return ExitCode::FAILURE;
            }
        },
    };
    let path = std::path::Path::new(&target);
    let is_manifest_arg = path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME);
    let manifest_dir: Option<std::path::PathBuf> = if is_manifest_arg {
        path.parent().map(|p| {
            if p.as_os_str().is_empty() {
                std::path::Path::new(".").to_path_buf()
            } else {
                p.to_path_buf()
            }
        })
    } else if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        None
    };
    let files: Vec<String> = if let Some(dir) = &manifest_dir {
        match load_manifest(dir) {
            Err(e) => {
                eprintln!("{PROG}: {e}");
                return ExitCode::FAILURE;
            }
            // A manifest is present: run its declared `tests`, resolved relative to the manifest's dir.
            // A `tests` entry may be a literal file OR a GLOB (`*.cdz`, `tests/*.cdz`, `**/x.cdz`),
            // expanded against the dir (path-sorted, deduped) — so a project can say `tests = ["*.cdz"]`.
            Ok(Some((mpath, m))) => {
                if m.tests.is_empty() {
                    eprintln!(
                        "{PROG}: {}: the manifest declares no `tests` (add `def tests = [\"…\"]`)",
                        mpath.display()
                    );
                    return ExitCode::SUCCESS;
                }
                let expanded = expand_manifest_globs(dir, &m.tests, &m.exclude);
                if expanded.is_empty() {
                    eprintln!(
                        "{PROG}: {}: the manifest's `tests` matched no files",
                        mpath.display()
                    );
                    return ExitCode::SUCCESS;
                }
                expanded
            }
            // No manifest in the directory: fall back to walking every source file (path-sorted).
            Ok(None) if is_manifest_arg => {
                eprintln!("{PROG}: {target}: no such file");
                return ExitCode::FAILURE;
            }
            Ok(None) => {
                let mut out = Vec::new();
                if let Err(e) = collect_source_dir(dir, &mut out) {
                    eprintln!("{PROG}: {e}");
                    return ExitCode::FAILURE;
                }
                if out.is_empty() {
                    eprintln!(
                        "{PROG}: {target}: no source files (.cdz/.ml/.sexp) found in directory"
                    );
                    return ExitCode::SUCCESS; // an empty tree is vacuously green
                }
                out
            }
        }
    } else {
        // A single-file target. If it's a COMPILED artifact (`.wasm`) rather than a source file, guide the
        // user instead of the misleading "0 tests found — add `@test`" (a `.wasm` has no source to scan):
        // `cdz test` runs a SOURCE file's `@test`s, the inverse of `cdz run`, which runs the `.wasm`.
        if !is_source_file(&target) && path.extension().and_then(|e| e.to_str()) == Some("wasm") {
            eprintln!(
                "{PROG} test: `{target}` is a COMPILED component, but `cdz test` runs a SOURCE file's \
                 `@test` definitions. Pass the source (`.cdz`/`.ml`/`.sexp`) instead — e.g. `cdz test \
                 src.cdz`; `cdz run {target}` is how you run a compiled component."
            );
            return ExitCode::FAILURE;
        }
        vec![target.clone()]
    };

    // `--list`: ENUMERATE the resolved suite's `@test` names as a cadenza-ast-binary `(test-list …)` value
    // and EXIT — no check-gate, no emit, no JIT, no wasmtime. This is the compiler-informed discovery source
    // v-nix's dynamic-derivations fan-out reads (no committed index, no IFD); it must be cheap and touch NONE
    // of the run machinery below. Short-circuit here, right after resolving `files`, so it shares the exact
    // file-resolution `cdz test` uses (manifest / dir walk / one file) but nothing after it.
    if args.list {
        return list_tests(&files, args.format);
    }
    // `--emit-shred`: shred the suite into per-@test wasm + a manifest (compile-only), then EXIT. Shares the
    // exact file-resolution above; the per-group emit + write is `run_emit_shred`.
    if args.emit_shred {
        let Some(out_dir) = args.out_dir.as_deref() else {
            eprintln!("{PROG} test: --emit-shred requires --out-dir <DIR>");
            return ExitCode::FAILURE;
        };
        return run_emit_shred(&files, out_dir, args.standalone, args.two_stage);
    }

    // GATE ON `cdz check` CLEAN FIRST — before running any `@test`. A source file that fails to PARSE (an
    // unclosed paren, a truncated form) is RECOVERED by the reader (it prints the errors, then hands back a
    // truncated arena of `<error>` placeholders), so the defs that DID parse still compile + run and the
    // suite reports "N passed, 0 failed" while the parse-broken sibling def is SILENTLY ABSENT. That is
    // precisely how a paren-imbalance regression landed GREEN through the fleet-gate `cdz test` step and then
    // blocked the pr-sync queue at the fresh full check (v-syntax's 76-min post-mortem, routed by concierge).
    // `cdz check` already exits non-zero on any error-severity fault (parse OR type), following each file's
    // import closure; run it over the SAME resolved files here and FAIL RED if any has an error, rather than
    // run a suite whose green is a lie. Dedup by canonical path (mirror `run_check`): `check_one` checks a
    // file's whole closure, so a module pulled into an earlier target's closure needn't be re-checked.
    //
    // SKIP the check-gate in `--warm-only` mode: a warm pass runs NO `@test` (it emits+JITs the shared-closure
    // provider into the cache, then exits), so the "green suite is a lie" risk this gate guards against cannot
    // arise — there is no suite. The check itself is expensive (each `check_one` type-checks the file's WHOLE
    // import closure — for a large self-host suite that's the ~570-def closure re-checked, the dominant residual
    // of a warm-once now that the emit is cached), so re-checking here just to immediately exit is pure waste.
    // The ACTUAL per-file `cdz test` sweep that later CONSUMES this warm cache runs its OWN check-gate (this
    // same block, `warm_only=false`), so the false-green protection is preserved exactly where a suite runs.
    if !args.warm_only {
        let canon = |p: &str| {
            std::fs::canonicalize(p)
                .map(|c| c.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string())
        };
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut check_failed = false;
        for f in &files {
            let canon_f = canon(f);
            if covered.contains(&canon_f) {
                continue;
            }
            let (had_error, closure_paths) = check_one(f, false, false, false);
            check_failed |= had_error;
            covered.insert(canon_f);
            for path in &closure_paths {
                covered.insert(canon(path));
            }
        }
        if check_failed {
            eprintln!(
                "{PROG} test: the project has errors (above) — NOT running the suite. Any of: a def that \
                 fails to PARSE (silently absent), a def that fails to RESOLVE/TYPE-CHECK, or a file that \
                 fails to READ leaves a suite whose green would be a lie; fix the errors (or run `cdz \
                 check` to see them) first."
            );
            return ExitCode::FAILURE;
        }
    }

    // The runtime store (shared across files). `cdz` runs each test IN-PROCESS — wasmtime + the runner
    // are linked in via the `cdz-run` LIBRARY, not shelled out to a sibling `cdz-run` BINARY — so the
    // one-binary guarantee holds for `cdz test` exactly as it does for `cdz run`: a single `cdz` on the
    // PATH both compiles AND runs the tests, with no second executable to install.
    let store = args.store.clone().unwrap_or_else(default_store);
    let multi = files.len() > 1;

    // Shared-arena lower-once: compile all target files in ONE EmitTestsPerFile pass (lowers the shared
    // closure once, emits one component per file). `run_test_file` looks its component up by name instead of
    // re-lowering the whole closure per file. Best-effort — an empty map (single file, or a union hiccup)
    // just means every file falls back to its own per-file compile, byte-identical to before. This ALSO
    // persists each closure group's provider to the cross-invocation cache — which is exactly the warm a
    // subsequent per-file sweep reuses.
    let precompile_start = std::time::Instant::now();
    let precompiled = precompile_tests_per_file(&files);
    // `--report-time`: the PRECOMPILE phase (per-closure emit — `EmitTestsComposed` on a `.provider.wasm` MISS
    // is the heavy ~270s+ closure LOWER; `EmitTestsConsumerOnly` on a HIT is cheap — plus the `Query::
    // ClosureHash` layout pass). Distinct from the provider JIT below: this pins whether the warm-once cost is
    // the EMIT (provider-cache miss) or the JIT (cwasm miss), the exact split pr-sync needs.
    if args.report_time {
        println!(
            "⏱ precompile: {} shared-closure provider(s) emitted/loaded in {}ms",
            precompiled.providers.len(),
            precompile_start.elapsed().as_millis()
        );
    }

    // JIT each shared-closure PROVIDER ONCE for the whole project, up front — then every file's composition
    // reuses the JIT'd provider `Component` instead of re-JITing it from bytes per file. `Component::new` (the
    // wasmtime JIT) of the heavy closure (the ~1360-def self-host provider) is the DOMINANT per-file startup
    // cost — the "sits there for a bit when each file's tests start" stall — so hoisting it out of the per-file
    // loop makes the project JIT the closure 1×, not N× (the rust-test-harness model: compile the shared code
    // once, then run every test against it). Each file still gets its own thin consumer + its own
    // per-file/per-test PASS/FAIL run below, so localization is untouched — we collapse the JIT, not the
    // reporting. A provider that fails to JIT here is simply omitted → that group's files fall back to their
    // standalone per-file compile in `run_test_file` (best-effort, no worse than before).
    // DESERIALIZE from a persisted cwasm when possible: the group `key` is the closure's content hash, so with
    // a cache dir we use `compile_provider_cached` — it persists the JIT'd artifact content-addressed by
    // (closure-hash ‖ engine fingerprint) and DESERIALIZES it (fast, ~seconds) on a later gate with an
    // unchanged closure, skipping the ~270s cold re-JIT of the heavy self-host closure. This runs BEFORE the
    // `--warm-only` early-return too: `--warm-only` (the gate's serial warm pass) must persist the CWASM, not
    // just the `.provider.wasm` emit — else the per-file sweep workers each cwasm-MISS and re-JIT (the 270s
    // stall stays). So warming = emit-persist (precompile above) + JIT-persist (here). Without a cache dir,
    // fall back to a plain in-process JIT.
    let provider_jit_start = std::time::Instant::now();
    let provider_cwasm_dir = provider_cache_dir();
    let jit_providers: std::collections::HashMap<String, cdz_run::CompiledProvider> = precompiled
        .providers
        .iter()
        .filter_map(|(key, (bytes, iface, content_hash))| {
            // Key the cwasm by the closure CONTENT HASH (not the import-name group `key`), so a content edit
            // invalidates it. Only cache when we HAVE a content hash + a cache dir; else plain in-process JIT.
            let compiled = match (&provider_cwasm_dir, content_hash) {
                (Some(dir), Some(hash)) => {
                    cdz_run::compile_provider_cached(bytes, iface.clone(), dir, hash)
                }
                _ => cdz_run::compile_provider(bytes, iface.clone()),
            };
            compiled.ok().map(|p| (key.clone(), p))
        })
        .collect();

    // `--report-time`: the PROJECT-WIDE provider JIT/deserialize — the dominant cost, paid ONCE here (the
    // provider-JIT-once fix) rather than per file. On a cwasm HIT this is a fast deserialize (~seconds); on a
    // MISS it's the full ~270s JIT. Printed BEFORE the `--warm-only` return so a warming run ALSO shows it —
    // the gate warms via `--warm-only`, so this line is how pr-sync/the operator see whether the warm step
    // itself HIT the cwasm (fast) or had to re-JIT (slow) it.
    if args.report_time && !jit_providers.is_empty() {
        println!(
            "⏱ provider JIT: {} shared closure(s) JIT'd/loaded once in {}ms",
            jit_providers.len(),
            provider_jit_start.elapsed().as_millis()
        );
    }
    // `--warm-only`: the emit cache (`.provider.wasm`, precompile above) AND the JIT cache (`.cwasm`, the
    // provider-JIT just above) are now both persisted. Stop here WITHOUT running the tests — a subsequent
    // per-file sweep HITS both (skips the closure emit AND the ~270s re-JIT). Report what warmed.
    if args.warm_only {
        let groups = precompiled.providers.len();
        let jitted = jit_providers.len();
        println!(
            "warmed {groups} shared-closure provider(s) — {jitted} JIT-cached (cwasm) — into the cache \
             ({} target file(s) across the suite); a per-file `cdz test` sweep will now reuse both",
            files.len()
        );
        return ExitCode::SUCCESS;
    }
    let pre = PrecompiledRun {
        precompiled: &precompiled,
        jit_providers: &jit_providers,
        report_time: args.report_time,
    };

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut any_error = false; // a file whose compile DECLINED (distinct from a test that failed)
    for (i, file) in files.iter().enumerate() {
        // In multi-file mode, head each file's block with its path so the output stays legible.
        if multi {
            if i > 0 {
                println!();
            }
            println!("── {file} ──");
        }
        match run_test_file(
            file,
            args.filter.as_deref(),
            args.tag.as_deref(),
            &store,
            args.trials,
            args.seed,
            &pre,
        ) {
            Ok((p, f)) => {
                total_pass += p;
                total_fail += f;
            }
            Err(()) => any_error = true, // the compile declined; errors already printed to stderr
        }
    }

    // A combined total across a package (a single file already printed its own "N passed, M failed").
    if multi {
        println!(
            "\n═══ TOTAL: {total_pass} passed, {total_fail} failed (across {} files) ═══",
            files.len()
        );
    }
    // A SINGLE explicit `cdz test <file>` that found ZERO tests is almost always a mistake — the user meant
    // to test something (e.g. wrote an UNKNOWN test-ish annotation like `@property`, which is silently
    // stripped so its def is not a test, leaving the file with no `@test`). Without a note this exits 0 with
    // NO output — a whole file can be dead + "green" by omission (breaker's silent-no-op finding). Print a
    // hint (still exit 0 — an empty file is not a failure, and this must not red the storeless library case).
    // Only for a single explicit file: a DIRECTORY/package run legitimately has test-free library modules,
    // and per-file "0 tests" there would be noise (each already headed by its path). `@test` is the property
    // spelling (a parameterized `@test`); `@property` is NOT a supported annotation (operator ruling).
    if !multi
        && total_pass == 0
        && total_fail == 0
        && !any_error
        && let Some(file) = files.first()
    {
        // Distinguish "no @test at all" from "a --tag/--filter EXCLUDED every test". Blaming a missing
        // `@test` when the real cause is an over-narrow selector (e.g. a typo'd `--tag`) points the user at
        // the wrong fix — the file may be full of tests the filter skipped. Only the unfiltered case is a
        // genuine "add a `@test`" situation.
        match (args.tag.as_deref(), args.filter.as_deref()) {
            // BOTH selectors present: they AND-compose, so either (or their intersection) could be empty.
            // Don't falsely blame one (a matching `--tag` with a missing `--filter` would be mis-reported) —
            // name both and point at their empty intersection.
            (Some(t), Some(f)) => println!(
                "0 tests matched `--tag {t}` AND `--filter {f}` in {file} — no `@test` both carries \
                 `@tag(\"{t}\")` and has a name containing `{f}` (loosen or drop a selector)."
            ),
            (Some(t), None) => println!(
                "0 tests matched `--tag {t}` in {file} — no `@test` carries that `@tag(\"{t}\")` (check for a \
                 typo, or drop `--tag` to run every test)."
            ),
            (None, Some(f)) => println!(
                "0 tests matched `--filter {f}` in {file} — no `@test` name contains that substring (check \
                 for a typo, or drop `--filter` to run every test)."
            ),
            (None, None) => println!(
                "0 tests found in {file} — a test needs the `@test` annotation (a parameterized `@test` is a \
                 property test); an unrecognized annotation is silently ignored."
            ),
        }
    }
    if total_fail == 0 && !any_error {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Run one file's `@test` definitions, printing `PASS`/`FAIL` per test and a per-file `N passed, M
/// failed` summary. Returns `(passed, failed)` on success, or `Err(())` when the test compile DECLINED
/// (its errors are printed to stderr) — distinct from a clean run where some tests failed. A file with no
/// matching `@test` prints nothing and returns `(0, 0)` (vacuously green), so a directory of mixed
/// modules — some without tests — aggregates cleanly.
/// The project-wide precompiled state a `cdz test` run threads into each file's [`run_test_file`]: the
/// per-closure grouping's components/providers ([`Precompiled`]) PLUS the providers JIT'd ONCE up front
/// (shared across every file so the heavy closure isn't re-JIT'd per file). Bundled so `run_test_file` takes
/// one context arg instead of two parallel maps.
#[cfg(feature = "standalone")]
pub(crate) struct PrecompiledRun<'a> {
    precompiled: &'a Precompiled,
    jit_providers: &'a std::collections::HashMap<String, cdz_run::CompiledProvider>,
    /// `--report-time`: emit per-phase (compose/run) + per-test durations (like `cargo test --report-time`).
    report_time: bool,
}

#[cfg(feature = "standalone")]
pub(crate) fn run_test_file(
    file: &str,
    filter: Option<&str>,
    tag: Option<&str>,
    store: &std::path::Path,
    trials: u64,
    seed: u64,
    pre: &PrecompiledRun<'_>,
) -> Result<(usize, usize), ()> {
    let precompiled = pre.precompiled;
    let jit_providers = pre.jit_providers;
    // Follow the entry file's IMPORT CLOSURE so a test in a module that imports a sibling (e.g. a pass
    // that reuses another module's type) resolves + runs — `cdz test FILE` sees the SAME linked program
    // `cdz check FILE` does. A file that imports nothing loads as a lone file, byte-identical to a
    // standalone single-file test compile; only a file carrying an `(import …)` pulls its siblings in.
    //
    // REUSE the closure `precompile_tests_per_file` already loaded for a SINGLE-file run (PR#907 — avoid
    // re-parsing the same file's whole closure twice). The stash is `Some` only for a single-file `cdz test
    // <file>` (a dir run loads each file's closure once here, never a sibling's); `Rc` so this is a refcount
    // bump. A multi-file run (or a defensive `None`) loads fresh, byte-identical to before.
    let loaded;
    let closure: &[closure::LoadedFile] = match &precompiled.single_file_closure {
        Some(rc) => rc.as_slice(),
        None => {
            loaded = match load_import_closure_with(file, &|_| None) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{PROG}: {e}");
                    return Err(());
                }
            };
            &loaded
        }
    };
    let is_package = !declared_import_paths(&closure[0].arenas).is_empty();

    // Encode each closure file's `ast` ONCE — the per-file artifacts feed BOTH the `Db` that enumerates
    // the ENTRY file's `@test` names and the package emit compile below. The front-end (`cadenza_syntax`)
    // and compiler (`rcdzc`) have DISTINCT arena types; the canonical binary form is the bridge.
    let ast_arts: Vec<cadenza_compile_abi::Artifact> = closure
        .iter()
        .map(|f| {
            cadenza_compile_abi::Artifact::new(
                cadenza_compile_abi::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();

    // Build the compiler `Db` used to enumerate test names + solve property-test param types. A single
    // file decodes directly (`Db::load`, byte-identical to before); a PACKAGE links every closure file
    // into one arena and loads it WITH its linkage (`Db::load_linked`), so a cross-file name resolves. On
    // a package, `linkage` also maps a test def back to its file so we run ONLY the ENTRY file's own
    // tests — an imported library's tests run when THAT file is itself the entry (a directory run visits
    // each), never double-counted through an importer.
    let (mut db, entry_filter) = if is_package {
        let mut rcdzc_files = Vec::with_capacity(ast_arts.len());
        for art in &ast_arts {
            let Some(a) = cadenza_syntax::codec::decode(&art.bytes) else {
                eprintln!("{PROG}: {file}: could not decode `{}`'s AST", art.name);
                return Err(());
            };
            rcdzc_files.push((art.name.clone(), a));
        }
        let program = match rcdzc::link::link(&rcdzc_files, &closure[0].name) {
            Ok(p) => p,
            Err(r) => {
                eprintln!("{PROG}: {file}: {}", r.message);
                return Err(());
            }
        };
        let linkage = program.linkage();
        let entry_ix = program.entry;
        let db = rcdzc::db::Db::load_linked(program.arenas, Some(linkage.clone()));
        (db, Some((linkage, entry_ix)))
    } else {
        let Some(rcdzc_arenas) = cadenza_syntax::codec::decode(&ast_arts[0].bytes) else {
            eprintln!("{PROG}: {file}: could not decode the program's AST");
            return Err(());
        };
        (rcdzc::db::Db::load(rcdzc_arenas), None)
    };
    // Each test's name PLUS the generators for its parameters (empty = a plain nullary test, run once;
    // non-empty = a PROPERTY test, run `trials` times with generated inputs). A param whose type is not a
    // generatable scalar makes `param_generators` return `None` — reported per test, not aborting the run.
    let mut tests: Vec<TestSpec> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for i in db.test_defs() {
        // In a PACKAGE, `test_defs()` sees every linked file's `@test`s — keep only the ENTRY file's own,
        // so an imported library's tests aren't run through its importer (they run when that library is
        // itself the entry). A def's file is the file whose id-range holds its signature node.
        if let Some((linkage, entry_ix)) = &entry_filter {
            match linkage.file_of(db.defs[i].sig_occ) {
                Some(fi) if fi == *entry_ix => {}
                _ => continue,
            }
        }
        let name = db.defs[i].name.clone();
        if filter.is_some_and(|needle| !name.contains(needle)) {
            continue;
        }
        // `--tag <t>`: keep only a test whose def carries the `@tag("t")` string tag. AND-composed with
        // `--filter` (both are additive selectors; an absent one imposes no constraint). A test with no
        // `@tag` is skipped under `--tag`, and every test runs when `--tag` is absent.
        if tag.is_some_and(|want| !db.tags_of(i).iter().any(|t| t == want)) {
            continue;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        // `@exhaustive`: the test is driven over its ENTIRE finite input domain, not by random sampling.
        // Captured per test (before `db` is re-borrowed by the next `param_generators`).
        let exhaustive = db.is_exhaustive(i);
        // `@requires` bounds for constrained generation — captured before `param_generators` re-borrows `db`
        // mutably. Only meaningful when the test is a boundary-arg (scalar-param) property; the -gen wrapper
        // path (compound params) draws internally and isn't clamped here (a later increment).
        let (bounds, relations) = param_bounds(&db, i);
        let gens = param_generators(&mut db, i);
        // For a `-gen` wrapper, capture its parameter `GenTy` now (db is in scope here, not in the run loop).
        // Only meaningful for the exhaustive-newtype enumeration below; a non-wrapper test yields `None`.
        let gen_ty = name
            .ends_with("-gen")
            .then(|| rcdzc::proptest_gen::gen_ty_of_wrapper_param(&db, &name))
            .flatten();
        tests.push(TestSpec {
            name,
            gens,
            exhaustive,
            bounds,
            relations,
            gen_ty,
        });
    }
    if tests.is_empty() {
        // No matching `@test` here. A file with no tests (e.g. a pure library module in a package dir, or
        // a `--filter` that selects nothing) is vacuously green — return (0, 0) and print nothing, so a
        // directory run aggregates without a spurious error line per test-free file.
        return Ok((0, 0));
    }

    // The test component. FAST PATH (Option-C composed): if the shared-arena precompile produced this file's
    // CONSUMER component (keyed by its link name) AND a shared-closure PROVIDER peer, use them — the consumer
    // imports the closure from the provider, so the whole closure was emitted ONCE (in the provider) instead
    // of re-embedded here. SLOW PATH (miss — single file, decline, multi-dir stem-collision, or the file
    // wasn't in the composed set): compile this file alone with an `EmitTests` request, exactly as before
    // (`layout::compute_tests`; a package's `entry` marker drives linking). A per-file DECLINE is reported
    // located here (the fallback owns error reporting — the precompile does not).
    // The composed consumer + shared provider for this file, if the precompile produced them. The composition
    // is JIT-compiled ONCE below (`compile_composition`) and reused across every trial, so a multi-trial
    // property test does NOT re-JIT per trial (PR#892 (a) — the earlier `has_multi_trial` fall-back guard is
    // obsolete now that the composed path reuses the JIT like the standalone path does).
    // Look up this file's consumer + the GROUP provider it links against (Option-A per-closure grouping — a
    // consumer records its group key, indexing `providers`). A consumer present but whose group provider is
    // absent (shouldn't happen — they're inserted together — but degrade safely) falls back per-file.
    let composed =
        precompiled
            .components
            .get(&closure[0].name)
            .and_then(|(consumer, group_key)| {
                precompiled.providers.get(group_key).map(
                    |(provider_bytes, iface, _content_hash)| {
                        (consumer.clone(), provider_bytes, iface, group_key.as_str())
                    },
                )
            });
    let component: Vec<u8> = if let Some((consumer, _, _, _)) = &composed {
        consumer.clone()
    } else {
        let mut inputs = ast_arts;
        inputs.push(cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::EmitTests]),
        ));
        if is_package {
            inputs.push(cadenza_compile_abi::abi::entry_artifact(&closure[0].name));
        }
        let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
        let Some(component) = out.artifact("component") else {
            // The test compile declined — report its errors (a parameterized `@test`, an ill-typed test body,
            // an invalid-kebab `@test` name, …). We HOLD the closure files (source + spans), so render each
            // fault at its `file:line:col` (the located reporter), not the bare `cdz: error …` — an anchored
            // decline (e.g. CDZ0201 on a bad `@test` name) then points at the name occurrence like `cdz check`.
            report_errors_located(&out, closure);
            return Err(());
        };
        component.to_vec()
    };

    // DEBUG (CDZ_DUMP_TEST_WASM): write the emitted test component to that path, for a WAT-diff of the
    // instantiation-set-dependent emit (bug#4). Throwaway.
    if let Ok(path) = std::env::var("CDZ_DUMP_TEST_WASM") {
        // Report the write outcome honestly — don't print "wrote …" when the write FAILED (a swallowed
        // permission/path error made this debug dump claim false success; PR#584 nit).
        match std::fs::write(&path, &component) {
            Ok(()) => eprintln!(
                "[dump] wrote test component ({} bytes) to {path}",
                component.len()
            ),
            Err(e) => eprintln!("[dump] FAILED to write test component to {path}: {e}"),
        }
    }

    // Resolve the value-heap runtime ONCE for this file's test component (reused across every test + trial):
    // the component records the exact runtime hash it was emitted against, and we read `<store>/<hash>.wasm`
    // BY CONTENT ADDRESS — the same resolution `cdz run` uses. A scalar/const test component imports no
    // runtime, so `required_runtime` returns `None` and we run with no runtime (no store needed). A missing
    // store entry is reported here, once, rather than as a trap inside each test.
    //
    // COMPOSED path: the consumer is CROSS-EDGE-EXCLUDING — the heap-using shared closure was hoisted into the
    // PROVIDER — so a consumer can import NO runtime while its provider peer DOES (e.g. a cad test whose heap
    // ops all live in the shared closure). `run_composition` composes ONE runtime for whichever of consumer OR
    // peer declares it (they pin the SAME runtime by content hash), reading the bytes from `opts.runtime`. So
    // we must resolve the runtime from EITHER component: try the consumer first, then fall back to the
    // provider. Resolving from only the consumer (as the standalone path does) left `opts.runtime = None` for a
    // consumer that imports no runtime but whose provider requires it → "requires the value-heap runtime …
    // but none was provided" for every grouped cad test (the reject). A shared runtime is a single instance, so
    // either source's identical bytes serve both.
    let runtime = {
        let consumer_rt = match resolve_test_runtime(&component, store) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{PROG}: {file}: {e}");
                return Err(());
            }
        };
        match (consumer_rt, composed.as_ref()) {
            // Consumer already pins the runtime — use it (same bytes the provider would resolve to).
            (Some(rt), _) => Some(rt),
            // Consumer imports no runtime but a provider peer may: resolve from the provider bytes.
            (None, Some((_, provider_bytes, _, _))) => {
                match resolve_test_runtime(provider_bytes, store) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("{PROG}: {file}: {e}");
                        return Err(());
                    }
                }
            }
            // Standalone (or composed with a runtime-free provider): no runtime needed.
            (None, _) => None,
        }
    };

    // Build the per-trial RUN TARGET, JIT-compiling ONCE + reusing across every test + trial — `Component::new`
    // (the wasmtime JIT) is the DOMINANT per-run cost (~8s for the self-host component vs ~0.1s to run it), so
    // compiling once instead of once-per-trial is the whole point. STANDALONE (common): the self-contained
    // component. COMPOSED (Option-C): the consumer + its shared-closure provider peer, JIT'd into ONE
    // `CompiledComposition` (consumer + peer Components) reused per trial — so a multi-trial property test
    // does NOT re-JIT the composition per trial (PR#892 materialize-once fix). Both reuse across trials.
    // `--report-time`: time the COMPOSE phase (this file's consumer JIT — the provider was JIT'd once up front).
    let compose_start = std::time::Instant::now();
    let target = if let Some((consumer, provider_bytes, iface, group_key)) = composed {
        // Prefer the PROJECT-WIDE pre-JIT'd provider (JIT'd ONCE in `run_test`, shared across every file) so
        // the heavy shared closure is not re-JIT'd per file — the per-file startup-stall fix. Only the thin
        // consumer is JIT'd here. Fall back to JITing the provider from bytes if it's somehow absent from the
        // map (a provider that failed to pre-JIT, or a caller that didn't pre-JIT) — behavior-identical, just
        // without the reuse. Both paths produce the SAME `CompiledComposition`, so the run is unchanged.
        let composition = match jit_providers.get(group_key) {
            Some(jit_provider) => cdz_run::compile_composition_with_providers(
                &consumer,
                std::slice::from_ref(jit_provider),
            ),
            None => cdz_run::compile_composition(
                &consumer,
                &[cdz_run::Peer {
                    bytes: provider_bytes.clone(),
                    interface: iface.clone(),
                }],
            ),
        };
        match composition {
            Ok(c) => RunTarget::Composed(c),
            Err(e) => {
                eprintln!("{PROG}: {file}: could not compile the composed test component: {e:#}");
                return Err(());
            }
        }
    } else {
        match cdz_run::compile_component(&component) {
            Ok(c) => RunTarget::Standalone(c),
            Err(e) => {
                eprintln!("{PROG}: {file}: could not compile the test component: {e:#}");
                return Err(());
            }
        }
    };

    // Run each test IN-PROCESS (via the `cdz-run` library — no sibling binary), in declaration order. A
    // NULLARY test runs ONCE — PASS = the export returned, FAIL = it trapped. A PROPERTY test (parameters)
    // runs `trials` times with generated inputs; it PASSES only if every trial returns, and FAILS on the
    // first trapping trial — reported with the failing inputs (shrunk toward a minimal counterexample) + the
    // seed to replay. The runtime cache dir is the store, so the JIT-compiled runtime is reused per trial.
    let compose_ms = compose_start.elapsed().as_millis();
    let run_start = std::time::Instant::now();

    let mut passed = 0usize;
    let mut failed = 0usize;
    for TestSpec {
        name,
        gens,
        exhaustive,
        bounds,
        relations,
        gen_ty,
    } in &tests
    {
        let kebab = cadenza_syntax::extern_name::kebab_extern_name(name);
        let run_one = |arg_vals: &[String]| -> TrialOutcome {
            run_one_trial(&target, runtime.as_deref(), &kebab, store, arg_vals)
        };
        // `--report-time`: per-TEST duration. Snapshot the fail-counter + a timer around this test's run;
        // after its `match` arm prints PASS/FAIL, emit a ` ⏱ {name} {ms}ms` line (like `cargo --report-time`).
        let test_start = std::time::Instant::now();
        let fail_before = failed;
        match gens {
            // A parameter whose type is not a generatable scalar — cannot property-test it. Report + fail.
            None => {
                failed += 1;
                println!(
                    "FAIL {name}: cannot generate inputs — a parameter's type is not a scalar this \
                     runner generates (Int/Bool/Float/Char); annotate it with a scalar type"
                );
            }
            // An `@exhaustive` test whose (original) parameter was COMPOUND: the compiler synthesized a
            // gen-driven wrapper that builds the value from the runner's random int POOL, which offers no
            // way to ENUMERATE a domain (it samples). So exhaustive checking is not (yet) supported for a
            // compound-parameter test — regardless of whether that domain is unbounded (a `List`) or
            // finite (a small user-sum enum). Decline cleanly, rather than sampling under an `@exhaustive`
            // label (which would falsely imply a proof) or aborting the file at the compound export
            // boundary. (Exhaustive enumeration works for a BOUNDED SCALAR signature — the boundary-arg
            // route above — where the domain is enumerated directly, not drawn from the pool.)
            // An `@exhaustive` over a BOUNDED `@invariant` NEWTYPE (`Percent = Pct(Int64)` with
            // `@invariant [0,100]`) CAN be enumerated: its `-gen` wrapper param is a single-variant `Sum`
            // whose payload is an `IntRange{lo,hi}`, and the `IntRange` decode map `v = lo + (pool & MAX) %
            // span` is INVERTIBLE — feeding pool int `v-lo` drives the wrapper over the exact value `v`. So
            // run it once per `v in lo..=hi` (a PROOF over the in-domain set), if `span` fits the cap. Any
            // other compound shape (a List/Tuple/multi-variant sum, or a too-wide range) falls through to the
            // clean decline below.
            Some(gens)
                if gens.is_empty()
                    && *exhaustive
                    && exhaustive_newtype_range(gen_ty.as_ref()).is_some() =>
            {
                let (lo, hi) = exhaustive_newtype_range(gen_ty.as_ref()).unwrap();
                // Render via the WHOLE Sum GenTy (it consumes selector then payload, matching the pool below),
                // so a failing case decodes to `S(v)`, the full nominal value.
                let full_gt = gen_ty.as_ref().unwrap();
                let span = (hi - lo + 1) as usize;
                let run_pool = |pool: &[i64]| -> TrialOutcome {
                    run_one_trial_with_pool(&target, runtime.as_deref(), &kebab, store, &[], pool).0
                };
                // The `-gen` wrapper for a single-variant `Sum` newtype draws a variant SELECTOR first
                // (`sel = gen % k`, here k=1 → any int selects the sole variant), THEN the `IntRange` payload
                // (`v = lo + (gen & MAX) % span`). So the pool for value `v` is `[selector=0, v-lo]` — mirror
                // the decode order exactly (a 1-element pool would run dry on the payload draw → a spurious
                // body trap). `pool_for(v)` builds it; `render_pool_value` decodes the SAME pool to `S(v)`.
                let pool_for = |v: i64| -> [i64; 2] { [0, v.wrapping_sub(lo)] };
                let failing =
                    (lo..=hi).find(|&v| matches!(run_pool(&pool_for(v)), TrialOutcome::Fail(_)));
                match failing {
                    None => {
                        passed += 1;
                        println!("PASS {name} (exhaustive, {span} cases)");
                    }
                    Some(v) => {
                        failed += 1;
                        // Render the failing case as the wrapper's decoded VALUE (`S(2)`), not a raw pool int.
                        let rendered = render_pool_value(full_gt, &pool_for(v))
                            .unwrap_or_else(|| v.to_string());
                        let msg = match run_pool(&pool_for(v)) {
                            TrialOutcome::Fail(Some(m)) => format!(": {m}"),
                            _ => String::new(),
                        };
                        println!(
                            "FAIL {name}{msg}\n  counterexample: {name}({rendered})  (exhaustive — the \
                             domain contains a failing case)"
                        );
                    }
                }
            }
            // An `@exhaustive` test whose (original) parameter was COMPOUND: the compiler synthesized a
            // gen-driven wrapper that builds the value from the runner's random int POOL, which offers no
            // way to ENUMERATE a domain (it samples). So exhaustive checking is not (yet) supported for a
            // compound-parameter test — regardless of whether that domain is unbounded (a `List`) or
            // finite (a small user-sum enum). Decline cleanly, rather than sampling under an `@exhaustive`
            // label (which would falsely imply a proof) or aborting the file at the compound export
            // boundary. (Exhaustive enumeration works for a BOUNDED SCALAR signature — the boundary-arg
            // route above — where the domain is enumerated directly, not drawn from the pool.)
            Some(gens) if gens.is_empty() && *exhaustive => {
                failed += 1;
                println!(
                    "FAIL {name}: @exhaustive is not supported for a compound parameter (a \
                     collection/tuple/record/sum) — its generator samples the random pool and cannot \
                     enumerate a domain; use a sampled `@test` for it, or make the signature bounded \
                     SCALAR parameters (Bool / a narrow integer) for exhaustive checking"
                );
            }
            // Nullary SOURCE signature — but this splits at runtime into two cases by whether the body
            // performs `Test.gen-int`: a GENERATOR-DRIVEN property test (a nullary wrapper that pulls random
            // ints from the runner to build its own inputs — the compound/int-stream route) vs a plain
            // unit test (pulls no generated int). Decide it by RUNNING once under a seeded int pool and
            // counting the `Test.gen-int` calls the guest made.
            Some(gens) if gens.is_empty() => {
                match run_gen_driven(
                    &target,
                    runtime.as_deref(),
                    &kebab,
                    store,
                    trials,
                    seed,
                    gen_ty.as_ref(),
                ) {
                    // The test consumed NO generated int → a plain unit test; report its single run.
                    GenDrivenOutcome::Plain(TrialOutcome::Pass) => {
                        passed += 1;
                        println!("PASS {name}");
                    }
                    GenDrivenOutcome::Plain(TrialOutcome::Fail(msg)) => {
                        failed += 1;
                        match msg {
                            Some(m) => println!("FAIL {name}: {m}"),
                            None => println!("FAIL {name}"),
                        }
                    }
                    // A generator-driven property test that passed every trial.
                    GenDrivenOutcome::Property(None) => {
                        passed += 1;
                        println!("PASS {name} ({trials} trials)");
                    }
                    // A generator-driven property test with a counterexample (the shrunk failing int pool).
                    GenDrivenOutcome::Property(Some(fail)) => {
                        failed += 1;
                        let msg = fail.message.map(|m| format!(": {m}")).unwrap_or_default();
                        // Prefer the CONCRETE VALUE the shrunk pool decodes to (e.g. `never-three([0,0,0])`)
                        // over the raw driver ints: recover the wrapper's original compound parameter type
                        // (the pre-synthesis def `<name-without-gen>` survives, `@test`-stripped, with its
                        // param type intact) and re-run the SAME generator derivation over the shrunk pool.
                        // Falls back to the raw-int line when the type can't be recovered/decoded (a shape
                        // the decoder doesn't yet render) — never a wrong value.
                        let pool_ints: Vec<i64> =
                            fail.inputs.iter().filter_map(|s| s.parse().ok()).collect();
                        let rendered = rcdzc::proptest_gen::gen_ty_of_wrapper_param(&db, name)
                            .and_then(|gty| render_pool_value(&gty, &pool_ints));
                        match rendered {
                            // Render the counterexample as a call to the ORIGINAL test (the `-gen` suffix is
                            // the synthesized-wrapper detail; `never_three([0,0,0])` reads as the user wrote
                            // it), while the `FAIL` line keeps the wrapper name the runner reports throughout.
                            Some(value) => {
                                let orig = name.strip_suffix("-gen").unwrap_or(name);
                                println!(
                                    "FAIL {name}{msg}\n  counterexample: {orig}({value})  (seed {seed}; \
                                     replay with `--seed {seed}`)"
                                )
                            }
                            None => {
                                let pool = fail
                                    .inputs
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                println!(
                                    "FAIL {name}{msg}\n  counterexample: generated ints [{pool}]  \
                                     (seed {seed}; replay with `--seed {seed}`)"
                                );
                            }
                        }
                    }
                }
            }
            // An `@exhaustive` PROPERTY test: drive the ENTIRE finite input domain (every combination of
            // the scalar parameters) rather than random sampling — a pass is a PROOF over the domain. Only
            // a BOUNDED domain can be enumerated; an unbounded parameter (a 32/64-bit int or a float, whose
            // domain is astronomically/infinitely large) makes `exhaustive_domain` return `None` → report
            // (the property must narrow its types, e.g. to `Bool`/`UInt8`, to be exhaustively provable).
            Some(gens) if *exhaustive => match exhaustive_domain(gens) {
                // An unbounded domain (a wide integer / float) is DECLINED with a diagnostic, never
                // silently sampled — so an exhaustive result is never reported for a domain not fully
                // covered.
                //= spec/capabilities/property-based-testing.md#an-unbounded-domain-declines-exhaustive-checking
                //# A property requested to be checked exhaustively over an unbounded input domain MUST be declined with a diagnostic rather than silently sampled, so that an exhaustive result is never reported for a domain that was not fully covered.
                None => {
                    failed += 1;
                    println!(
                        "FAIL {name}: @exhaustive needs a BOUNDED input domain — a parameter's type \
                         (a wide integer or a float) has too large a domain to enumerate; narrow it \
                         (e.g. Bool or UInt8)"
                    );
                }
                Some(domain) => {
                    let total = domain.len();
                    match domain
                        .into_iter()
                        .find(|inputs| matches!(run_one(inputs), TrialOutcome::Fail(_)))
                    {
                        // No failing case in the WHOLE enumerated domain → a proof over the domain, not a
                        // sample.
                        //= spec/capabilities/property-based-testing.md#exhaustive-coverage-is-a-proof-over-a-bounded-domain
                        //# When a property is checked by enumerating its entire bounded finite domain, a run that finds no failing input MUST be treated as a proof of the property over the domain rather than as a sample.
                        None => {
                            passed += 1;
                            println!("PASS {name} (exhaustive, {total} cases)");
                        }
                        Some(inputs) => {
                            failed += 1;
                            // Re-run the failing case to recover its reported message.
                            let msg = match run_one(&inputs) {
                                TrialOutcome::Fail(Some(m)) => format!(": {m}"),
                                _ => String::new(),
                            };
                            let args_str = inputs.join(", ");
                            println!(
                                "FAIL {name}{msg}\n  counterexample: {name}({args_str})  (exhaustive \
                                 — the domain contains a failing case)"
                            );
                        }
                    }
                }
            },
            // A sampled PROPERTY test: run `trials` trials with generated inputs.
            Some(gens) => match run_property(gens, bounds, relations, trials, seed, &run_one) {
                None => {
                    passed += 1;
                    println!("PASS {name} ({trials} trials)");
                }
                Some(PropertyFailure { inputs, message }) => {
                    failed += 1;
                    let args_str = inputs.join(", ");
                    let msg = message.map(|m| format!(": {m}")).unwrap_or_default();
                    // A reported property failure records BOTH the input that produced it (the shrunk
                    // counterexample args) AND the seed to replay — so the failing run is reproducible.
                    //= spec/capabilities/property-based-testing.md#generation-is-seeded-and-reproducible
                    //# A reported property failure MUST record the seed and the input that produced it.
                    println!(
                        "FAIL {name}{msg}\n  counterexample: {name}({args_str})  (seed {seed}; replay \
                         with `--seed {seed}`)"
                    );
                }
            },
        }
        // Per-test duration (like `cargo test --report-time`) — a compact line under the test's PASS/FAIL,
        // emitted only under `--report-time` so the default output is unchanged. Label the outcome so a slow
        // PASS and a slow FAIL are both attributable at a glance.
        if pre.report_time {
            let outcome = if failed > fail_before { "FAIL" } else { "PASS" };
            println!(
                "  ⏱ {outcome} {name} {}ms",
                test_start.elapsed().as_millis()
            );
        }
    }

    // Per-STEP timing for this file (compose = this file's consumer JIT; run = all its tests) — the "where the
    // time goes" breakdown the operator asked for. The heavy shared-closure provider JIT is NOT here — it's
    // paid ONCE up front in `run_test` (reported there), which is the whole point of the provider-JIT-once fix.
    if pre.report_time {
        println!(
            "  ⏱ {file}: compose {compose_ms}ms · run {}ms",
            run_start.elapsed().as_millis()
        );
    }

    println!("\n{passed} passed, {failed} failed");
    Ok((passed, failed))
}

// ── cdz watch ──────────────────────────────────────────────────────────────────────────────────

/// Is `path` a Cadenza SOURCE file (or the project manifest) — the only changes `cdz watch` re-runs on?
/// A POSITIVE filter (source extensions + `Project.cdz`) is what keeps `watch` from self-triggering: a
/// `watch build`/`watch test` writes `.wasm`/`.rs`/`.dwarf`/`link-map.txt`/`.cdz-run-*` artifacts INTO the
/// watched dir, and an editor churns swap/temp files — none of those are source, so none re-fire the run.
#[cfg(feature = "watch")]
pub(crate) fn is_watch_trigger(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME) {
        return true;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("cdz" | "ml" | "sexp" | "sexpr")
    )
}

/// `cdz watch [target] --exec <check|test|build>` — the `cargo watch` analogue. Resolve the project's
/// manifest directory (the same `Project.cdz` `cdz build`/`test` use, searching upward when omitted),
/// watch that directory recursively, and RE-RUN the chosen command whenever a SOURCE file (or the
/// manifest) changes. The loop runs the command once up front (initial feedback, like `cargo watch`),
/// then blocks on the filesystem event channel; on a source-file event it DEBOUNCES (keeps draining
/// events for `debounce_ms` so a burst of saves — or an editor's write-then-rename — coalesces into ONE
/// run), re-runs SYNCHRONOUSLY (so runs never overlap — the concierge guard), then inspects the events
/// that arrived DURING the run: artifact-only churn (the run's own outputs) is discarded, but a SOURCE
/// edit made mid-run is NOT reflected in the run that just finished, so it triggers one more re-run
/// rather than being dropped until the next event. Ctrl-C exits. The re-run itself is the ordinary
/// in-process `run_check`/`run_test`/`run_build`/`run_project`.
#[cfg(feature = "watch")]
pub(crate) fn run_watch(args: &WatchArgs) -> ExitCode {
    use notify::{RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::Duration;

    // Resolve the project dir to watch (no entry requirement — `check` needs none; `build`/`test` report
    // their own missing-entry error on the re-run). This validates the target up front so `cdz watch` on a
    // manifest-less dir fails immediately rather than watching nothing.
    let (dir, _mpath, m) = match resolve_project_manifest(args.target.as_deref(), "cdz watch") {
        Ok(v) => v,
        Err(code) => return code,
    };

    // The re-run closure: construct the chosen command's Args targeted at the resolved manifest dir, and
    // invoke the ordinary handler in-process. `--store` threads through to the commands that resolve the
    // value-heap runtime (`test`/`run`); `check`/`build` don't take a store. Returns the command's code.
    let store = args.store.clone();
    let call = args.call.clone();
    let run_args = args.args.clone();
    let filter = args.filter.clone();
    let tag = args.tag.clone();
    let trials = args.trials;
    let seed = args.seed;
    // Clear the terminal before each run (`--clear`, like `cargo watch -c`). A `Copy` bool captured
    // separately from `args` (which is moved into the `rerun` closure below). The clear is emitted BEFORE
    // the run's banner so each run's output starts on a fresh screen; a no-op when `--clear` is unset.
    let clear = args.clear;
    let clear_screen = move || {
        if clear {
            use std::io::Write;
            print!("\x1b[2J\x1b[H"); // ANSI: erase display + move cursor home
            let _ = std::io::stdout().flush();
        }
    };
    let dir_str = dir.to_string_lossy().into_owned();
    let rerun = move || -> ExitCode {
        match args.exec {
            WatchCmd::Check => run_check(&CheckArgs {
                file: Some(dir_str.clone()),
                json: false,
                verify_fixes: false,
                diagnostics_wire: false, // watch is an interactive re-check; the raw grader wire is a one-shot mode
            }),
            WatchCmd::Test => run_test(&TestArgs {
                file: Some(dir_str.clone()),
                filter: filter.clone(),
                tag: tag.clone(),
                store: store.clone(),
                trials,
                seed,
                warm_only: false, // watch RUNS the tests on each change, never a warm-only pass
                report_time: false, // watch is an interactive re-run; timing is an opt-in of a direct run
                list: false, // watch RE-RUNS the suite; enumeration-and-exit is a one-shot direct-run mode
                format: ListFormat::Binary, // moot when list=false
                emit_shred: false, // watch RE-RUNS; the shred build-output is a one-shot direct-run mode
                out_dir: None,
                standalone: false,
                two_stage: false,
            }),
            WatchCmd::Build => run_build(&BuildArgs {
                dir: Some(dir_str.clone()),
                out: None,
                release: false,
                opt_level: None,
                target: BuildTargetArg::Wasm,
            }),
            // `run` in PROJECT mode: `component = the dir` routes through `run_project` (build the entry,
            // then run it), the same path `cdz run <dir>` takes. `store` threads through for a heap run.
            // A watch `run` sets only the entry + its interactive call/args/store; every other flag takes
            // its default (no grade, no leak-ceiling, no verdict/diagnostics wire, sexp render). The spread
            // keeps this site compiling when a new `RunArgs` field is added (the cross-crate E0063 class).
            WatchCmd::Run => run_project(&cdz_run::cli::RunArgs {
                component: Some(std::path::PathBuf::from(&dir_str)),
                call: call.clone(),
                args: run_args.clone(),
                store: store.clone(),
                ..Default::default()
            }),
        }
    };

    let label = match args.exec {
        WatchCmd::Check => "check",
        WatchCmd::Test => "test",
        WatchCmd::Build => "build",
        WatchCmd::Run => "run",
    };

    // Set up the recursive watch on the manifest directory. `notify`'s recommended watcher is the
    // platform-native backend (inotify/FSEvents/kqueue). Events flow over an mpsc channel.
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res| {
        // A send failure only means the receiver was dropped (we're exiting) — nothing to do.
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{PROG}: cannot create a filesystem watcher: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = watcher.watch(&dir, RecursiveMode::Recursive) {
        eprintln!("{PROG}: cannot watch {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    // ALSO watch each PATH DEPENDENCY's directory, so editing a dep's source re-triggers the run — the
    // multi-project edit loop (`cdz watch --exec run` on a project with `def deps`). The dep dir is
    // resolved relative to this manifest's dir (same as `build_path_deps`). A dep whose dir can't be
    // watched (e.g. it doesn't exist yet) is noted but NOT fatal — the run itself reports an unresolvable
    // dep; here we just skip watching it. The source-file filter still gates re-runs, so a dep's own
    // `.wasm` build artifacts don't self-trigger.
    for dep in &m.deps {
        #[allow(clippy::infallible_destructuring_match)]
        let dep_path = match dep {
            DepSource::Path(p) => p,
        };
        let dep_dir = dir.join(dep_path);
        if let Err(e) = watcher.watch(&dep_dir, RecursiveMode::Recursive) {
            eprintln!(
                "{PROG}: note: not watching dependency `{dep_path}` ({}): {e}",
                dep_dir.display()
            );
        }
    }

    let debounce = Duration::from_millis(args.debounce_ms);
    eprintln!(
        "{PROG}: watching {} — re-running `cdz {label}` on change (Ctrl-C to stop)",
        dir.display()
    );

    // Whether a batch of filesystem events touched a SOURCE file / the manifest — the only changes that
    // warrant a re-run. Artifact writes (a `build`/`run`'s own `.wasm`/`link-map.txt`) and editor temp
    // churn are ignored, so they never self-trigger.
    let batch_touches_source = |batch: &[notify::Result<notify::Event>]| -> bool {
        batch.iter().any(|res| {
            res.as_ref()
                .map(|ev| ev.paths.iter().any(|p| is_watch_trigger(p)))
                .unwrap_or(false)
        })
    };

    // 1. Initial run (once — the initial feedback, like `cargo watch`).
    clear_screen();
    let _ = rerun();

    // Drain the STARTUP event burst before arming the change loop. macOS FSEvents delivers a spurious
    // create/coalesced event for the pre-existing watched directory right after the watch begins (Linux
    // inotify does not) — without this drain, that event would trip the loop's change path and fire a
    // SPURIOUS extra run on startup (a real double-build on macOS). Give FSEvents a brief moment to emit
    // that burst, then discard everything queued. The settle is SHORT + FIXED (not per-event) and bounded
    // to the startup window, so it can't swallow a user's later edit — a real change after this returns to
    // the normal blocking `recv` below. (The startup burst lands within a few ms of arming the watch; a
    // 150ms settle covers it well under the test/debounce windows without adding meaningful startup lag.)
    std::thread::sleep(Duration::from_millis(150));
    while rx.try_recv().is_ok() {}

    // 2/3. Event loop: block for a change, debounce-coalesce, re-run, then check whether MORE source
    // edits arrived DURING the run — if so, run again (a mid-run save must not be lost).
    let mut pending = false; // a source edit seen while a run was in flight, not yet acted on
    loop {
        if pending {
            // A source edit landed DURING the last run — it is real + unreflected, so re-run NOW without
            // blocking or re-checking (we already confirmed it touched source when we set `pending`).
            // Still coalesce whatever else is immediately queued so a mid-run burst folds into this run.
            pending = false;
            while rx.try_recv().is_ok() {}
        } else {
            // Block until some event arrives (or the channel closes → exit), then coalesce the debounce
            // window and re-run only if the batch touched a SOURCE file / the manifest.
            let first = match rx.recv() {
                Ok(ev) => ev,
                Err(_) => return ExitCode::SUCCESS, // watcher dropped
            };
            let mut batch = vec![first];
            while let Ok(ev) = rx.recv_timeout(debounce) {
                batch.push(ev);
            }
            if !batch_touches_source(&batch) {
                continue; // artifact/temp churn only — nothing to re-run
            }
        }
        clear_screen();
        eprintln!("{PROG}: ⟳ change detected — re-running `cdz {label}`");
        let _ = rerun();
        // Inspect events that arrived DURING the re-run. Artifact-only churn (the run's own outputs) is
        // discarded — already reflected. But a SOURCE edit made mid-run is NOT reflected in the run we
        // just finished, so flag it (`pending`) to re-run once more rather than silently dropping that
        // save. (`pending` re-runs UNCONDITIONALLY next iteration — the source check happens HERE, so an
        // artifact-only follow-on batch can't cancel a real mid-run edit.)
        let mut during = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            during.push(ev);
        }
        if batch_touches_source(&during) {
            pending = true;
        }
    }
}

/// The scalar KIND of a property-test parameter — what the runner generates a value of and renders to a
/// `cdz-run --arg` string. Restricted to the scalars `cdz-run`'s `coerce_one` parses from `--arg` text:
/// the boundary-crossable scalars this first property-testing increment supports (a compound param —
/// tuple/sum/list — is a later increment via a guest-side `Gen` effect).
#[derive(Clone, Copy)]
#[cfg(feature = "standalone")]
pub(crate) enum GenKind {
    /// A fixed-width integer: `(signed, width)`. Generated as a random value in the width's range.
    Int {
        signed: bool,
        width: u32,
    },
    Bool,
    /// A float (32 or 64). Generated as a random finite decimal — the text parses as either width, so the
    /// width need not be tracked here (`cdz-run`'s `coerce_one` parses `f32`/`f64` from the same decimal).
    Float,
    Char,
}

/// One `@test`/`@exhaustive` def selected to run, with everything the runner needs to drive it: its `name`,
/// the per-parameter generators (`None` = a param type the runner can't generate; empty = a nullary def run
/// once), whether it is `@exhaustive`, and the per-parameter `@requires` integer `bounds` for constrained
/// generation (empty ⇒ unconstrained). Distilled once per test in the collection loop before the run loop.
#[cfg(feature = "standalone")]
pub(crate) struct TestSpec {
    name: String,
    gens: Option<Vec<GenKind>>,
    exhaustive: bool,
    bounds: Vec<ParamBound>,
    /// Recognized ORDER relations between two integer params from `@requires` (e.g. `(< a b)`), enforced by
    /// rejection sampling in the generator. Empty for the common single-param / unconstrained case.
    relations: Vec<Relation>,
    /// The synthesized `-gen` wrapper's parameter `GenTy` (via `gen_ty_of_wrapper_param`), captured at BUILD
    /// time because `db` is not in scope in the run loop. `Some` only for a `-gen` wrapper; `None` for a plain
    /// or scalar test. Used by the `@exhaustive` path to ENUMERATE a bounded newtype domain (a single-variant
    /// `Sum` whose payload is an `IntRange` — feed pool values `0..span` to drive the wrapper over `lo..=hi`).
    gen_ty: Option<rcdzc::proptest_gen::GenTy>,
}

/// The generators for definition `def`'s parameters, or `None` if ANY parameter's solved type is not a
/// generatable scalar (so the test cannot be property-run). An EMPTY vec means a nullary def (run once).
/// Each param's type is solved with `infer::type_of` on its binder (seeing through a `(: n T)` annotation,
/// the shape a boundary parameter needs) — the same type `layout::export_params` crossed it as.
#[cfg(feature = "standalone")]
pub(crate) fn param_generators(db: &mut rcdzc::db::Db, def: usize) -> Option<Vec<GenKind>> {
    let params = db.defs[def].params.clone();
    let mut kinds = Vec::with_capacity(params.len());
    for p in params {
        // See through a `(: name T)` binder to the name occurrence `type_of` types (bare param → itself).
        let binder = db
            .ast
            .as_form(p, ":")
            .and_then(|t| t.first().copied())
            .unwrap_or(p);
        let ty = rcdzc::infer::type_of(db, binder);
        let kind = match ty {
            rcdzc::ty::Ty::Int(it) => GenKind::Int {
                signed: it.ground_signed(),
                width: it.ground_width(),
            },
            rcdzc::ty::Ty::Bool => GenKind::Bool,
            rcdzc::ty::Ty::Float(_) => GenKind::Float,
            rcdzc::ty::Ty::Char => GenKind::Char,
            _ => return None, // a non-scalar (or unresolved) param — cannot generate it here
        };
        kinds.push(kind);
    }
    Some(kinds)
}

/// The constraint a `@requires` precondition imposes on one scalar parameter, so the generator draws only
/// IN-DOMAIN values and never trips the (D) body-entry precondition trap. For an INTEGER param it is an
/// inclusive range `[lo, hi]` (`i128` so a full `i64`/`u64` range plus a `±1` strict-to-inclusive adjustment
/// never overflows). For a BOOL param, `bool_force` pins it: a bare-Bool precondition `@requires(b)` requires
/// `b` true, so the generator must draw `true` (a random `false` would trip the pre-trap). `None` = no Bool
/// constraint. The two are independent (a param is one or the other kind); an int param never sets
/// `bool_force`, a bool param never narrows `[lo, hi]`.
#[derive(Clone, Copy)]
#[cfg(feature = "standalone")]
pub(crate) struct ParamBound {
    lo: i128,
    hi: i128,
    bool_force: Option<bool>,
}

#[cfg(feature = "standalone")]
impl ParamBound {
    /// The widest bound — no constraint. Narrowed by each recognized `@requires` comparison.
    fn unbounded() -> Self {
        ParamBound {
            lo: i128::MIN,
            hi: i128::MAX,
            bool_force: None,
        }
    }
    /// Clamp a drawn value into `[lo, hi]`. An empty range (lo > hi, from contradictory requires) leaves the
    /// value unchanged — the precondition is unsatisfiable and the trap is the correct outcome, not our job
    /// to hide.
    fn clamp(&self, v: i128) -> i128 {
        if self.lo > self.hi {
            v
        } else {
            v.clamp(self.lo, self.hi)
        }
    }
    /// Whether this bound narrows anything (worth applying).
    fn is_constrained(&self) -> bool {
        self.lo != i128::MIN || self.hi != i128::MAX
    }
}

/// A recognized relation between two integer parameters from a `@requires` — e.g. `(< a b)` or `(= a b)`,
/// where both sides are param names (not a param-vs-literal, which a `ParamBound` already covers). Unlike a
/// per-param range clamp, a relation COUPLES two params, so it cannot be satisfied by clamping one in
/// isolation. Two enforcement strategies by operator. An ORDER op (`< <= > >=`) is enforced by REJECTION
/// SAMPLING: re-draw (advancing the seed deterministically) until every relation holds, bounded by fuel. An
/// EQUALITY (`=`) is enforced by PROPAGATION: the right param's value is copied FROM the left, so `a = b`
/// holds by construction with ZERO rejection (two independent draws are ~never equal, so rejection would only
/// exhaust fuel — propagation is the reject-free analogue of clamping for a range bound). Any unrecognized
/// shape stays unconstrained exactly as before. `op` is one of the recognized operator strings; `left`/`right`
/// are parameter POSITIONS (matching the `GenKind` vec order).
#[derive(Clone, Copy)]
#[cfg(feature = "standalone")]
pub(crate) struct Relation {
    pub(crate) left: usize,
    pub(crate) op: &'static str,
    pub(crate) right: usize,
}

/// Whether `l OP r` holds for the recognized operators (an unrecognized op vacuously holds — it was never
/// recorded, so this is only reached for `< <= > >= =`). After `propagate_equalities` runs, an `=` relation
/// always holds; it is still checked here so the rejection loop's `relations_hold` guard is total.
#[cfg(feature = "standalone")]
pub(crate) fn relation_holds(op: &str, l: i64, r: i64) -> bool {
    match op {
        "<" => l < r,
        "<=" => l <= r,
        ">" => l > r,
        ">=" => l >= r,
        "=" => l == r,
        _ => true,
    }
}

/// Enforce each EQUALITY relation (`(= a b)`) by copying the LEFT param's value onto the RIGHT — so `a = b`
/// holds BY CONSTRUCTION, no rejection. Iterated to a fixpoint (bounded by the equality count) so a chain
/// `a = b and b = c` fully propagates (all become `a`) regardless of the order the relations were recorded.
/// Order relations are untouched here (they go through rejection sampling). Applied after each draw and after
/// building each shrink trial.
#[cfg(feature = "standalone")]
pub(crate) fn propagate_equalities(relations: &[Relation], inputs: &mut [String]) {
    let eq_count = relations.iter().filter(|r| r.op == "=").count();
    for _ in 0..eq_count {
        for rel in relations {
            if rel.op == "=" && rel.left < inputs.len() && rel.right < inputs.len() {
                inputs[rel.right] = inputs[rel.left].clone();
            }
        }
    }
}

/// Whether EVERY recorded relation holds over the rendered `inputs`. A param whose rendered value does not
/// parse as an `i64` (e.g. a Bool `"true"`) makes its relation vacuously hold — relations are only recorded
/// between integer params, so this is a defensive skip, never the common path.
#[cfg(feature = "standalone")]
pub(crate) fn relations_hold(relations: &[Relation], inputs: &[String]) -> bool {
    relations.iter().all(|rel| {
        let (Some(l), Some(r)) = (inputs.get(rel.left), inputs.get(rel.right)) else {
            return true;
        };
        match (l.parse::<i64>(), r.parse::<i64>()) {
            (Ok(l), Ok(r)) => relation_holds(rel.op, l, r),
            _ => true,
        }
    })
}

/// Per-parameter integer bounds distilled from a def's `@requires` preconditions (empty ⇒ no bound). The
/// generator applies these so a `@test` over a `@requires`-constrained def draws only inputs SATISFYING the
/// precondition — the (D) enforcement traps a violated precondition at body entry (a HARD contract for every
/// caller, `verify_enforce.rs`), so feeding an out-of-domain draw would spuriously FAIL the test rather than
/// exercise the property. This is the runner-side half of "constrained generation": recognize simple
/// range/comparison predicates over a single scalar param (`(>= x LO)`, `(< x HI)`, `(= x K)`, and their
/// mirrors), and constrain that param's draw. An UNRECOGNIZED predicate shape leaves the param unbounded —
/// the draw is unconstrained, exactly as before (never wrong: an over-broad draw that happens to satisfy the
/// pre still passes; only a pre-violating draw was the bug, and a recognized bound removes those).
///
/// Only INTEGER params are bounded here (the boundary-arg route generates Int/Bool/Float/Char; a comparison
/// bound is meaningful for integers — Bool/Float/Char preconditions fall through unrecognized). Keyed by
/// parameter POSITION (matching the `GenKind` vec order).
#[cfg(feature = "standalone")]
pub(crate) fn param_bounds(db: &rcdzc::db::Db, def: usize) -> (Vec<ParamBound>, Vec<Relation>) {
    let params = &db.defs[def].params;
    // Map each param NAME to its position, so a `(>= name lit)` predicate targets the right slot.
    let pos_of: std::collections::HashMap<&str, usize> = params
        .iter()
        .enumerate()
        .filter_map(|(i, &p)| {
            // A param is a bare name atom or an annotated `(: name T)` binder — the name is the head child.
            let name_node = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            db.ast.as_name(name_node).map(|n| (n, i))
        })
        .collect();
    let mut bounds = vec![ParamBound::unbounded(); params.len()];
    let mut relations = Vec::new();
    for &pred in db.requires_of(def) {
        narrow_from_predicate(db, pred, &pos_of, &mut bounds, &mut relations);
    }
    (bounds, relations)
}

/// Narrow `bounds` (and collect `relations`) from ONE `@requires` predicate AST node. Recognizes a
/// comparison `(OP a b)` for OP in `>= > <= < =`: a (param, literal) pairing in either order narrows that
/// param's `ParamBound`; a (param, param) pairing for an ORDER op (`< <= > >=`) or `=` records a `Relation`
/// between the two params (a coupled constraint a single-param clamp cannot express — an order relation is
/// satisfied by rejection sampling, an equality by propagation). It also descends a conjunction
/// `(and p q …)` / `(& p q …)` so `(and (>= x 0) (< x 100))` bounds `x` to `[0, 99]`. Anything else (a call,
/// a non-linear predicate) is left unrecognized — no change, exactly as before.
#[cfg(feature = "standalone")]
pub(crate) fn narrow_from_predicate(
    db: &rcdzc::db::Db,
    pred: cadenza_syntax::ast::StructId,
    pos_of: &std::collections::HashMap<&str, usize>,
    bounds: &mut [ParamBound],
    relations: &mut Vec<Relation>,
) {
    // Descend a conjunction: every conjunct constrains independently.
    for head in ["and", "&"] {
        if let Some(tail) = db.ast.as_form(pred, head) {
            for &conj in tail {
                narrow_from_predicate(db, conj, pos_of, bounds, relations);
            }
            return;
        }
    }
    // A BARE PARAM NAME predicate — `@requires(b)` where `b` is a Bool param — requires that param TRUE. A
    // random `false` draw would trip the (D) pre-trap, so pin the generated value to `true` (the Bool analogue
    // of pinning an int to a constant). A bare name in a `@requires` can only be a Bool param (the enforcement
    // wraps `(if PRE BODY (trap))`, which type-checks only for a Bool `PRE`); a name that is not a param
    // (a prelude/global) simply isn't in `pos_of` and is left unconstrained.
    if let Some(name) = db.ast.as_name(pred)
        && let Some(&i) = pos_of.get(name)
    {
        bounds[i].bool_force = Some(true);
        return;
    }
    // A comparison `(OP lhs rhs)`. Identify OP, then the (param, literal) pairing in either order.
    for op in [">=", ">", "<=", "<", "="] {
        let Some(t) = db.ast.as_form(pred, op) else {
            continue;
        };
        if t.len() != 2 {
            return;
        }
        let (lhs, rhs) = (t[0], t[1]);
        let as_i128 = |v: &cadenza_syntax::ast::IntValue| v.to_i128();
        // `(OP param lit)` — the common spelling.
        if let (Some(name), Some(lit)) = (db.ast.as_name(lhs), db.ast.as_int(rhs).and_then(as_i128))
        {
            if let Some(&i) = pos_of.get(name) {
                apply_cmp(op, lit, false, &mut bounds[i]);
            }
            return;
        }
        // `(OP lit param)` — the mirrored spelling; the operator flips (lit < x ⇒ x > lit).
        if let (Some(lit), Some(name)) = (db.ast.as_int(lhs).and_then(as_i128), db.ast.as_name(rhs))
        {
            if let Some(&i) = pos_of.get(name) {
                apply_cmp(op, lit, true, &mut bounds[i]);
            }
            return;
        }
        // `(OP param param)` — a RELATION between two params. An ORDER op (`< <= > >=`) is satisfied by
        // rejection sampling; an EQUALITY `=` is satisfied by propagation (copy left→right). A param compared
        // to itself is skipped: `(< a a)` is unsatisfiable (leave it to trap, not our job to mask), and
        // `(= a a)` is trivially true (no constraint).
        if let (Some(ln), Some(rn)) = (db.ast.as_name(lhs), db.ast.as_name(rhs))
            && let (Some(&li), Some(&ri)) = (pos_of.get(ln), pos_of.get(rn))
            && li != ri
            && matches!(op, "<" | "<=" | ">" | ">=" | "=")
        {
            relations.push(Relation {
                left: li,
                op,
                right: ri,
            });
        }
        return; // recognized OP; a (param, param) relation was recorded above if applicable
    }
}

/// Narrow one `ParamBound` by `param OP lit` (or, when `mirrored`, `lit OP param`). A strict `<`/`>` becomes
/// an inclusive bound via `±1` (integers). `=` pins both ends.
#[cfg(feature = "standalone")]
pub(crate) fn apply_cmp(op: &str, lit: i128, mirrored: bool, b: &mut ParamBound) {
    // Normalize the mirrored form `lit OP param` to `param OP' lit`: `lit < x` ⇔ `x > lit`, etc.
    let op = if mirrored {
        match op {
            "<" => ">",
            ">" => "<",
            "<=" => ">=",
            ">=" => "<=",
            other => other, // `=` is symmetric
        }
    } else {
        op
    };
    match op {
        ">=" => b.lo = b.lo.max(lit),
        ">" => b.lo = b.lo.max(lit.saturating_add(1)),
        "<=" => b.hi = b.hi.min(lit),
        "<" => b.hi = b.hi.min(lit.saturating_sub(1)),
        "=" => {
            b.lo = b.lo.max(lit);
            b.hi = b.hi.min(lit);
        }
        _ => {}
    }
}

/// The number of `(list …)` elements a synthesized variable-length list generator produces candidates for
/// (mirrors `proptest_gen::G1_LIST_LEN`). The wrapper draws a count `c = gen % (LEN+1)`, then LEN candidate
/// elements, keeping the first `c`. The decoder MUST use the same LEN to consume the pool in lockstep.
#[cfg(feature = "standalone")]
pub(crate) const RUNNER_LIST_LEN: usize = 3;

/// Render the concrete value a shrunk driver `pool` decodes to for the wrapper's parameter generator shape
/// `gty`, mirroring the derivation `proptest_gen::build_gen` synthesized into the `-gen` wrapper — so the
/// reported counterexample is the actual value that failed (`[0, 0, 0]` / `(1, false)` / `Err(3)`) rather
/// than the raw driver ints. `gty` is `proptest_gen`'s OWN `GenTy` (via `gen_ty_of_wrapper_param`), so the
/// decode vocabulary is the SAME one the generator was built from — it can never drift out of sync, and a
/// `Sum`/nested shape is covered identically to the wrapper. The pool is consumed via a shared cursor in the
/// SAME order the wrapper pulls `Test.gen-int`. `None` only if the pool runs dry (a malformed shrink).
#[cfg(feature = "standalone")]
pub(crate) fn render_pool_value(gty: &rcdzc::proptest_gen::GenTy, pool: &[i64]) -> Option<String> {
    let mut cursor = 0usize;
    decode_value(gty, pool, &mut cursor)
}

/// If `gen_ty` is a `-gen` wrapper param that CAN be exhaustively enumerated — a single-variant `Sum` newtype
/// whose sole payload is an `IntRange{lo,hi}` (a bounded `@invariant` newtype like `Percent = Pct(Int64)` with
/// `@invariant [0,100]`) AND whose domain size `hi-lo+1` fits [`MAX_EXHAUSTIVE_CASES`] — return `(lo, hi)`.
/// `None` for any other shape (a List/Tuple/multi-variant sum, a non-IntRange payload, or a range too large to
/// enumerate) — the caller then declines the `@exhaustive` cleanly. This is what lets `@exhaustive` PROVE a
/// property over a small refined newtype's whole domain (drive the wrapper over each `v in lo..=hi`) instead
/// of sampling / declining.
#[cfg(feature = "standalone")]
pub(crate) fn exhaustive_newtype_range(
    gen_ty: Option<&rcdzc::proptest_gen::GenTy>,
) -> Option<(i64, i64)> {
    use rcdzc::proptest_gen::GenTy;
    let GenTy::Sum { variants, .. } = gen_ty? else {
        return None;
    };
    // A single-variant newtype whose one payload is an IntRange.
    let [(_, Some(GenTy::IntRange { lo, hi }))] = variants.as_slice() else {
        return None;
    };
    let (lo, hi) = (*lo, *hi);
    // A valid range (lo<=hi) whose size fits the enumeration cap — else decline (too large to prove).
    let span = (hi as i128) - (lo as i128) + 1;
    (span >= 1 && span <= MAX_EXHAUSTIVE_CASES as i128).then_some((lo, hi))
}

/// One step of the pool→value decode (see [`render_pool_value`]). `cursor` advances by exactly the number of
/// `Test.gen-int` ints the corresponding `build_gen` arm consumes, in the same order.
#[cfg(feature = "standalone")]
pub(crate) fn decode_value(
    gty: &rcdzc::proptest_gen::GenTy,
    pool: &[i64],
    cursor: &mut usize,
) -> Option<String> {
    use rcdzc::proptest_gen::GenTy;
    // Pull the next driver int (the wrapper's `Test.gen-int`); `None` if the shrunk pool is exhausted.
    let next = |cursor: &mut usize| -> Option<i64> {
        let v = pool.get(*cursor).copied()?;
        *cursor += 1;
        Some(v)
    };
    match gty {
        // A scalar Int consumes one int: the value IS that int.
        GenTy::Int => Some(next(cursor)?.to_string()),
        // A range-constrained int consumes one int, mapped into `[lo, hi]` EXACTLY as the generator's
        // `build_gen` IntRange arm does: mask to non-negative (`& i64::MAX`), `% SPAN`, `+ lo`. Mirroring the
        // derivation keeps the decoded counterexample equal to the value that actually ran.
        GenTy::IntRange { lo, hi } => {
            let span = hi.wrapping_sub(*lo).wrapping_add(1);
            let v = lo.wrapping_add((next(cursor)? & i64::MAX).rem_euclid(span));
            Some(v.to_string())
        }
        // A Bool consumes one int, taken as its parity (`gen % 2 == 0`) — the `build_gen` Bool derivation.
        GenTy::Bool => Some((next(cursor)?.rem_euclid(2) == 0).to_string()),
        // A Float consumes one int, converted to an integer-valued float (`FloatN.of-int`).
        GenTy::Float(_) => Some(format!("{}.0", next(cursor)?)),
        // A variable-length list: a count `c = MIN + (gen % (LEN+1-MIN))` then LEN candidate elements (all
        // drawn), value = the first `c`. `min_len` (a min-length refinement floor, clamped to LEN) mirrors
        // the generator's `build_var_list_gen` count formula EXACTLY so the decode stays in lockstep with the
        // run. The decoder draws all LEN elements regardless of `c`, same as the wrapper.
        GenTy::List(elem, min_len) => {
            let min = (*min_len).min(RUNNER_LIST_LEN);
            let span = (RUNNER_LIST_LEN + 1 - min) as i64;
            // Mirror the generator EXACTLY: `(gen & i64::MAX) % span` (mask non-negative, then mod), NOT
            // rem_euclid — the wrapper masks the sign bit, which differs from rem_euclid for a negative gen.
            let c = min + ((next(cursor)? & i64::MAX) % span) as usize;
            let mut elems = Vec::with_capacity(RUNNER_LIST_LEN);
            for _ in 0..RUNNER_LIST_LEN {
                elems.push(decode_value(elem, pool, cursor)?);
            }
            elems.truncate(c);
            Some(format!("[{}]", elems.join(", ")))
        }
        // A tuple draws one value per slot, in order.
        GenTy::Tuple(slots) => {
            let mut vals = Vec::with_capacity(slots.len());
            for slot in slots.iter() {
                vals.push(decode_value(slot, pool, cursor)?);
            }
            Some(format!("({})", vals.join(", ")))
        }
        // A record draws one value per field, in the field order `build_gen` used.
        GenTy::Record(fields) => {
            let mut parts = Vec::with_capacity(fields.len());
            for (fname, fty) in fields.iter() {
                let v = decode_value(fty, pool, cursor)?;
                parts.push(format!("{fname}: {v}"));
            }
            Some(format!("{{{}}}", parts.join(", ")))
        }
        // A user SUM: the wrapper draws a selector `sel = gen % k` FIRST, then EVERY variant's payload
        // unconditionally (in order), and keeps variant `sel`. The decoder mirrors that EXACTLY — draw the
        // selector, then decode each variant's payload advancing the cursor over ALL of them, keeping only
        // the selected variant's rendering (`Err(3)`, or a bare `None` for a nullary variant). Draining every
        // payload keeps the cursor correct even when the sum is NESTED inside an enclosing compound.
        GenTy::Sum { variants, .. } => {
            if variants.is_empty() {
                return None;
            }
            let k = variants.len();
            let sel = (next(cursor)?.rem_euclid(k as i64)) as usize;
            let mut selected: Option<String> = None;
            for (i, (vname, payload)) in variants.iter().enumerate() {
                let rendered = match payload {
                    None => vname.clone(),
                    Some(pty) => format!("{vname}({})", decode_value(pty, pool, cursor)?),
                };
                if i == sel {
                    selected = Some(rendered);
                }
            }
            selected
        }
        // A Set: the generator draws a count `c = (gen & i64::MAX) % (LEN+1)` then folds `c` `Set.insert`s of
        // the first `c` of `RUNNER_LIST_LEN` candidate elements over the empty set (a VARIABLE-cardinality set,
        // so the empty/singleton sets are reachable — see `build_var_set_gen`). Mirror it EXACTLY: draw the
        // count, decode all `RUNNER_LIST_LEN` candidates (cursor advances over every one, so a NESTED Set stays
        // in lockstep), keep the length-`c` prefix, then DEDUP by rendered value (a collision yields a smaller
        // set, as `Set.insert` does). `{}` for c=0. A refined-newtype element renders in-domain via its GenTy.
        GenTy::Set(elem) => {
            let span = (RUNNER_LIST_LEN + 1) as i64;
            let c = ((next(cursor)? & i64::MAX) % span) as usize;
            let mut drawn = Vec::with_capacity(RUNNER_LIST_LEN);
            for _ in 0..RUNNER_LIST_LEN {
                drawn.push(decode_value(elem, pool, cursor)?);
            }
            drawn.truncate(c);
            let mut seen: Vec<String> = Vec::with_capacity(c);
            for e in drawn {
                if !seen.contains(&e) {
                    seen.push(e);
                }
            }
            Some(format!("{{{}}}", seen.join(", ")))
        }
        // A Map: the generator draws a count `c = (gen & i64::MAX) % (LEN+1)` then folds `c` `Map.insert`s of
        // the first `c` of `RUNNER_LIST_LEN` candidate key/value pairs over `(Map.empty)` (a VARIABLE-size map,
        // so the empty/small maps are reachable — see `build_var_map_gen`). Mirror it EXACTLY: draw the count,
        // decode all `RUNNER_LIST_LEN` (key, value) candidate pairs (cursor advances over every one, so a
        // NESTED Map stays in lockstep), keep the length-`c` prefix, then apply LAST-WRITE-WINS by rendered key
        // (preserving first-insertion order, as the insert fold does). `{}` for c=0. Refined-newtype key/value
        // decodes in-domain via its own GenTy.
        GenTy::Map(kty, vty) => {
            let span = (RUNNER_LIST_LEN + 1) as i64;
            let c = ((next(cursor)? & i64::MAX) % span) as usize;
            let mut drawn: Vec<(String, String)> = Vec::with_capacity(RUNNER_LIST_LEN);
            for _ in 0..RUNNER_LIST_LEN {
                let k = decode_value(kty, pool, cursor)?;
                let v = decode_value(vty, pool, cursor)?;
                drawn.push((k, v));
            }
            drawn.truncate(c);
            let mut entries: Vec<(String, String)> = Vec::with_capacity(c);
            for (k, v) in drawn {
                // Last-write-wins: update an existing key's value in place (keeping its position), else append.
                if let Some(slot) = entries.iter_mut().find(|(ek, _)| ek == &k) {
                    slot.1 = v;
                } else {
                    entries.push((k, v));
                }
            }
            let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            Some(format!("{{{}}}", parts.join(", ")))
        }
    }
}

/// The outcome of one trial: PASS (the export returned) or FAIL (it trapped) with the failure message the
/// test reported (via its `Test`/report host effect), if any.
#[cfg(feature = "standalone")]
pub(crate) enum TrialOutcome {
    Pass,
    Fail(Option<String>),
}

/// A property-test failure: the (rendered) inputs that reproduced it, and the reported message.
#[cfg(feature = "standalone")]
pub(crate) struct PropertyFailure {
    inputs: Vec<String>,
    message: Option<String>,
}

/// How a file's `@test` component is run per trial. STANDALONE: a self-contained component, JIT-compiled ONCE
/// and reused across every trial (the common path — `run_capturing_compiled`). COMPOSED (Option-C): a
/// cross-edge-EXCLUDING consumer + its shared-closure provider peer, JIT-compiled ONCE into a
/// `CompiledComposition` and reused across every trial (`run_composition_capturing`) — so a multi-trial
/// property test no longer re-JITs consumer+peer per trial (PR#892). Both paths yield the SAME
/// `(Outcome, observed-op-list)` shape, so the trial logic (gen-int count, failure message) is identical.
#[cfg(feature = "standalone")]
pub(crate) enum RunTarget {
    Standalone(cdz_run::CompiledComponent),
    Composed(cdz_run::CompiledComposition),
}

/// Run the test component IN-PROCESS once, calling `kebab` with `arg_vals` (rendered arg text). PASS = the
/// export returned; FAIL carries the failure message the test reported (via its `Test`/report host effect)
/// if any. `runtime` is the value-heap runtime bytes the component was resolved against (or `None` for a
/// scalar/const test component that imports no runtime).
#[cfg(feature = "standalone")]
pub(crate) fn run_one_trial(
    target: &RunTarget,
    runtime: Option<&[u8]>,
    kebab: &str,
    store: &std::path::Path,
    arg_vals: &[String],
) -> TrialOutcome {
    run_one_trial_with_pool(target, runtime, kebab, store, arg_vals, &[]).0
}

/// Whether to include the full wasm BACKTRACE (the `<wasm function N>` frames wasmtime captures on a trap)
/// in a trapping test's FAIL message, rather than trimming to the reason's first line. Off by default (a
/// one-line counterexample stays legible); enabled by setting `CDZ_WASM_BACKTRACE` to any non-empty value
/// other than `0`/`false`. A debug lever for localizing a COMPILED trap — the frame indices are the only
/// locus for a self-host trap, where the usual isolate-the-case repro doesn't reproduce it.
#[cfg(feature = "standalone")]
pub(crate) fn wasm_backtrace_enabled() -> bool {
    match std::env::var("CDZ_WASM_BACKTRACE") {
        Ok(v) => {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

/// The well-known GENERATOR effect operation a property test performs to pull one random `Int64` from the
/// runner's driver: `Test.gen-int : Unit -> Int64` (the "well-known `Test` effect extends" convention — the
/// same `Test` effect that carries `fail`). `cdz test` answers a `Test.gen-int` performance with the next int
/// from a seeded pool, so a generator built on this ONE op — bolero's Driver model, one int source that
/// type-directed generation decodes — needs no per-shape host coordination.
#[cfg(feature = "standalone")]
pub(crate) const GEN_OP_LABEL: &str = "test.gen-int";

/// Run the test component IN-PROCESS (via the `cdz-run` LIBRARY — `run_capturing`, no sibling binary),
/// ALSO supplying a seeded int `pool` as ordered `Test.gen-int=<n>` host responses (consumed IN ORDER by each
/// `Test.gen-int` performance — a result-bearing op; a unit op like `Test.fail` consumes none). Returns the
/// trial outcome AND how many `Test.gen-int` calls the guest actually made (counted from the OBSERVED host-op
/// list `run_capturing` returns) — the signal that distinguishes a PROPERTY test (pulls ≥1 generated int)
/// from a plain unit test (pulls none). An unconsumed pool response is harmless (ignored).
#[cfg(feature = "standalone")]
pub(crate) fn run_one_trial_with_pool(
    target: &RunTarget,
    runtime: Option<&[u8]>,
    kebab: &str,
    store: &std::path::Path,
    arg_vals: &[String],
    pool: &[i64],
) -> (TrialOutcome, usize) {
    // Each pool int becomes a `Test.gen-int` host response, consumed in order. The op label pairs it with the
    // call for the ordered-consume model (the value is coerced to the op's `Int64` result at binding).
    let host_responses: Vec<cdz_run::HostResponse> = pool
        .iter()
        .map(|n| cdz_run::HostResponse {
            op: "Test.gen-int".to_string(),
            value: n.to_string(),
        })
        .collect();
    // FINDING#23: the runtime imports `cadenza:nfc/normalize`, but cdz-run now SELF-RESOLVES that NFC
    // component from the store (keyed off `runtime_cache_dir`, set below) inside its compose step — no `nfc`
    // field to thread here.
    let opts = cdz_run::RunOpts {
        export: Some(kebab.to_string()),
        args: arg_vals.to_vec(),
        runtime: runtime.map(<[u8]>::to_vec),
        runtime_cache_dir: Some(store.to_path_buf()),
        host_responses,
        // `cdz run` JIT-compiles the freshly-built project (it HAS the compiler); precompiled/deserialize
        // mode is the cranelift-free corpus-exec path, not this front-end.
        precompiled: false,
    };
    // Both targets were JIT-compiled ONCE by the caller + are reused across every trial (`Component::new` is
    // ~99% of a run's cost). STANDALONE runs the compiled component; COMPOSED links the compiled consumer
    // against the compiled provider peer over one runtime. Both return the SAME `(Outcome, observed)` shape,
    // so the trial logic below is identical.
    let run_result = match target {
        RunTarget::Standalone(compiled) => {
            cdz_run::run_capturing_compiled(compiled, &opts, None, false, None)
        }
        RunTarget::Composed(composition) => cdz_run::run_composition_capturing(composition, &opts),
    };
    match run_result {
        Ok((outcome, observed)) => {
            let gens = count_gen_calls(&observed);
            let trial = match outcome {
                cdz_run::Outcome::Value(_) => TrialOutcome::Pass,
                // A trapping test FAILS. Prefer the assertion message the test emitted (via its report host
                // effect, e.g. `Test.fail("…")`) — it rides an OBSERVED op entry as `<op>\t<message>`. But if
                // there is NO such op, the body TRAPPED for another reason (an arithmetic OVERFLOW `+ traps:
                // overflows Int64`, a div-by-zero, an explicit `trap("…")`) — and that reason is exactly what
                // distinguishes "the property BODY TRAPPED" from "the property RETURNED FALSE". The runtime's
                // `Trap(reason)` carries that reason, so fall back to it (prefixed so the author sees the body
                // trapped rather than the property being false — a very different diagnosis, e.g. a full-domain
                // Int64 generator whose unguarded `+` overflows on two large samples is NOT a real violation).
                cdz_run::Outcome::Trap(reason) => {
                    TrialOutcome::Fail(observed_failure_message(&observed).or_else(|| {
                        // A wasmtime trap renders as `wasm trap: <reason>` followed by a multi-line wasm
                        // BACKTRACE (`0: 0x… - <wasm function N>` frames). By default trim to the FIRST line
                        // — the actionable reason — so the one-line counterexample report stays legible.
                        // With `CDZ_WASM_BACKTRACE` set, keep the WHOLE reason (frames included): a compiled
                        // trap (esp. self-host, where the isolated-repro trick fails) is hard to localize
                        // without the `<wasm function N>` frame indices, and there is no other way to see them
                        // in `cdz test` (v-wasm-opt's diagnostic-quality gap — the backtrace IS captured, it
                        // was just being discarded here).
                        let trimmed = reason.trim();
                        (!trimmed.is_empty()).then(|| {
                            if wasm_backtrace_enabled() {
                                format!("body trapped: {trimmed}")
                            } else {
                                format!(
                                    "body trapped: {}",
                                    reason.lines().next().unwrap_or("").trim()
                                )
                            }
                        })
                    }))
                }
            };
            (trial, gens)
        }
        // A run-level error (an invalid component, an unresolvable runtime the pre-check missed) — surface
        // it as a failure so the test is reported, not silently skipped.
        Err(e) => (
            TrialOutcome::Fail(Some(format!("could not run test: {e:#}"))),
            0,
        ),
    }
}

/// How many `Test.gen-int` performances the guest made, from the OBSERVED host-op list `run_capturing` returns
/// (each entry is a dotted `E.op`, optionally `\t<str-args>`). `> 0` ⇒ the test is a PROPERTY test driven
/// by the int pool. Matches the op field (before any tab) case-insensitively against the `Test.gen-int` label.
#[cfg(feature = "standalone")]
pub(crate) fn count_gen_calls(observed: &[String]) -> usize {
    observed
        .iter()
        .filter(|entry| {
            let op = entry.split('\t').next().unwrap_or(entry);
            op.eq_ignore_ascii_case(GEN_OP_LABEL)
        })
        .count()
}

/// The assertion message a trapping test reported, from the OBSERVED host-op list. `run_capturing`
/// records each string-carrying host call as `<op>\t<message>`, but that is EVERY string-arg op — a
/// `log.emit("…")` a test performs before it fails carries a message too. So match ONLY a REPORTING op
/// (one whose dotted name ends in `.fail` — `test.fail`/`report.fail`, the ops a failing assertion
/// performs), not just the first tab-carrying entry, or a benign log line would be misreported as the
/// failure message. The LAST such `.fail` wins (the one closest to the trap). `None` if no reporting op
/// carried a message (a bare trap with no assertion text).
#[cfg(feature = "standalone")]
pub(crate) fn observed_failure_message(observed: &[String]) -> Option<String> {
    observed.iter().rev().find_map(|entry| {
        let (op, msg) = entry.split_once('\t')?;
        // The op field is a dotted `E.op`; a reporting op ends in `.fail` (case-insensitive, since the
        // observed op label preserves the boundary spelling — `Test.fail`/`test.fail`).
        op.to_ascii_lowercase()
            .ends_with(".fail")
            .then(|| msg.to_string())
    })
}

/// Resolve the value-heap runtime bytes the test `component` requires, BY CONTENT ADDRESS from `store` —
/// the same content-addressed resolution `cdz run` performs. Returns `Ok(None)` for a scalar/const
/// component that imports no runtime (no store needed), `Ok(Some(bytes))` when the store holds the exact
/// required hash, and `Err` (a clear, once-per-file message) when the component needs a runtime the store
/// does not hold — reported before running rather than as an opaque trap inside each test.
#[cfg(feature = "standalone")]
pub(crate) fn resolve_test_runtime(
    component: &[u8],
    store: &std::path::Path,
) -> Result<Option<Vec<u8>>, String> {
    let req = match cdz_run::required_runtime(component) {
        Ok(Some(req)) => req,
        Ok(None) => return Ok(None), // scalar/const test component — no runtime import
        Err(e) => return Err(format!("could not inspect the test component: {e:#}")),
    };
    if req.hash.is_empty() {
        return Err(
            "the test component imports the value-heap runtime but records no content address to \
             resolve it by (an unpinned runtime import)"
                .to_string(),
        );
    }
    let path = store.join(format!("{}.wasm", req.hash));
    if !path.is_file() {
        return Err(format!(
            "no runtime of content address {} in the store at {} — build it (`cargo xtask build`) so \
             `cdz test` can run a heap-value test",
            req.hash,
            store.display()
        ));
    }
    std::fs::read(&path)
        .map(Some)
        .map_err(|e| format!("reading the stored runtime {}: {e}", path.display()))
}

/// Run a PROPERTY test `trials` times with generated inputs, returning `None` if every trial passed or the
/// first counterexample (SHRUNK toward a minimal failing input). Generation is seeded (`seed`) so a run is
/// reproducible; each trial advances the seed deterministically (`seed + trial`), so the failing trial's
/// inputs re-generate identically on replay. On the first failing trial, `shrink` searches for a smaller
/// still-failing input before reporting.
#[cfg(feature = "standalone")]
pub(crate) fn run_property(
    gens: &[GenKind],
    bounds: &[ParamBound],
    relations: &[Relation],
    trials: u64,
    seed: u64,
    run_one: &dyn Fn(&[String]) -> TrialOutcome,
) -> Option<PropertyFailure> {
    for trial in 0..trials {
        let inputs = generate_inputs(gens, bounds, relations, seed.wrapping_add(trial));
        if let TrialOutcome::Fail(message) = run_one(&inputs) {
            let (inputs, message) = shrink(gens, bounds, relations, &inputs, message, run_one);
            return Some(PropertyFailure { inputs, message });
        }
    }
    None
}

/// What a nullary-signature test turned out to be at runtime: a PLAIN unit test (consumed no generated
/// int — its single-run outcome), or a generator-driven PROPERTY test (`None` = every trial passed, or
/// the shrunk failing int pool).
#[cfg(feature = "standalone")]
pub(crate) enum GenDrivenOutcome {
    Plain(TrialOutcome),
    Property(Option<PropertyFailure>),
}

/// The number of random ints a property test's generator is offered per trial — the driver POOL size. A
/// generator pulls as many as its shape needs (a scalar 1, an `(Int64, Bool)` 2, a small list a few); a
/// pool larger than any reasonable shape means the guest never runs dry, and unconsumed responses are
/// ignored. (When compound generators land and can pull unboundedly, this becomes a per-trial budget.)
#[cfg(feature = "standalone")]
pub(crate) const GEN_POOL_SIZE: usize = 64;

/// Run a nullary-signature test, deciding PLAIN vs generator-driven PROPERTY by whether it pulls any
/// `Test.gen-int` int. The FIRST run uses a seeded pool (`seed`); if the guest consumed ZERO generated ints
/// it is a plain unit test — return its outcome directly (one run, today's semantics, unaffected by the
/// unconsumed pool). If it consumed ≥1, it is a property test: run `trials` trials each with a FRESH
/// seeded pool (`seed + trial`, reproducible), failing on the first trapping trial with the SHRUNK pool.
#[cfg(feature = "standalone")]
pub(crate) fn run_gen_driven(
    target: &RunTarget,
    runtime: Option<&[u8]>,
    kebab: &str,
    store: &std::path::Path,
    trials: u64,
    seed: u64,
    gen_ty: Option<&rcdzc::proptest_gen::GenTy>,
) -> GenDrivenOutcome {
    let run_pool = |pool: &[i64]| -> (TrialOutcome, usize) {
        run_one_trial_with_pool(target, runtime, kebab, store, &[], pool)
    };
    // First trial (trial 0) doubles as the PLAIN-vs-property probe.
    let pool0 = gen_pool(seed, GEN_POOL_SIZE);
    let (outcome0, gens0) = run_pool(&pool0);
    if gens0 == 0 {
        // No generated int consumed → a plain unit test. Its outcome is the single run.
        return GenDrivenOutcome::Plain(outcome0);
    }
    // A property test. Trial 0's result counts; if it already failed, shrink + report.
    if let TrialOutcome::Fail(message) = outcome0 {
        return GenDrivenOutcome::Property(Some(shrink_pool(
            &pool0, gens0, message, gen_ty, &run_pool,
        )));
    }
    // Remaining trials, each a fresh seeded pool.
    for trial in 1..trials {
        let pool = gen_pool(seed.wrapping_add(trial), GEN_POOL_SIZE);
        let (outcome, gens) = run_pool(&pool);
        if let TrialOutcome::Fail(message) = outcome {
            return GenDrivenOutcome::Property(Some(shrink_pool(
                &pool, gens, message, gen_ty, &run_pool,
            )));
        }
    }
    GenDrivenOutcome::Property(None)
}

/// A seeded pool of `size` random `Int64`s — the driver stream a property test's generator pulls from.
/// Reproducible from `seed` (bolero's `driver::Rng` over a seeded `Xoshiro256PlusPlus`), so a reported
/// failing seed replays the exact pool.
#[cfg(feature = "standalone")]
pub(crate) fn gen_pool(seed: u64, size: usize) -> Vec<i64> {
    use bolero_generator::driver::{self, Rng};
    use bolero_generator::{ValueGenerator, produce};
    let rng = rand_from_seed(seed);
    let mut d = Rng::new(rng, &driver::Options::default());
    (0..size)
        .map(|_| produce::<i64>().generate(&mut d).unwrap_or(0))
        .collect()
}

/// SHRINK a failing int pool toward a minimal counterexample: reduce the CONSUMED prefix (`gens` ints —
/// the ones the generator actually pulled; trailing pool entries never affected the run) toward 0, one
/// position at a time by halving, keeping any reduction that STILL fails. Reports the consumed prefix
/// (rendered) — the ints that reproduce the failure. Greedy + bounded, like the scalar `shrink`.
///
/// This IS the harness's shrinking search: on a failing property it searches for a SMALLER input that
/// still fails (halving each consumed position, keeping only reductions that still `Fail`); it TERMINATES
/// (each position halves toward 0, `while n != 0`, and a non-failing candidate breaks that position — no
/// unbounded search); and it REPORTS the minimal `best` prefix it converged to as the counterexample.
//= spec/capabilities/property-based-testing.md#shrinking-converges-to-a-minimal-failing-input
//# When a property fails, the harness MUST search for a smaller input that still fails.
//= spec/capabilities/property-based-testing.md#shrinking-converges-to-a-minimal-failing-input
//# The shrinking search MUST terminate rather than search unboundedly.
//= spec/capabilities/property-based-testing.md#shrinking-converges-to-a-minimal-failing-input
//# The shrinking search MUST report a minimal failing input.
#[cfg(feature = "standalone")]
pub(crate) fn shrink_pool(
    pool: &[i64],
    gens: usize,
    message: Option<String>,
    gen_ty: Option<&rcdzc::proptest_gen::GenTy>,
    run_pool: &dyn Fn(&[i64]) -> (TrialOutcome, usize),
) -> PropertyFailure {
    // Only the CONSUMED prefix matters — the generator pulled `gens` ints; the rest of the pool is inert.
    let mut best: Vec<i64> = pool.iter().take(gens).copied().collect();
    let mut best_msg = message;
    // DECODED-SPACE shrink for a single-IntRange newtype (`Percent = Pct(Int64)` with `@invariant [0,100]`):
    // the wrapper pool is `[selector, payload]` and the payload decodes `v = lo + (payload & MAX) % span`,
    // which is NOT monotonic in the raw payload int — so the generic raw-int halving below cannot converge to
    // the domain-minimal (it reported e.g. Pct(67), not the true boundary). Here we bisect the DECODED value
    // toward `lo` directly (candidate value `c` ⇒ pool payload `c - lo`, the invertible map), keeping any `c`
    // that still fails, so the counterexample shrinks to the smallest in-domain failing value. Only this
    // single-IntRange-newtype shape is handled (its pool layout is known: selector at 0, payload at 1);
    // compound/multi-leaf shapes fall through to the generic pass unchanged.
    if let Some((lo, hi)) = exhaustive_newtype_range(gen_ty)
        && best.len() >= 2
    {
        // DECODED-SPACE shrink toward the boundary. The generic raw-int halving below cannot converge for an
        // IntRange leaf (decode `v = lo + (payload & MAX) % span` is non-monotonic in the raw int), so bisect
        // the DECODED value: find the LEAST `v in [lo, hi]` that still fails, via the invertible map (pool
        // payload = v - lo). This assumes the common upward-closed fail-set (`v >= threshold`), the shape a
        // refined-newtype property almost always has; a fail-set that isn't upward-closed still yields a
        // VALID failing value (never a wrong one — every kept candidate is RE-RUN and confirmed to fail),
        // just not necessarily the global minimum. `hi_fail` = a known-failing upper bound (the current
        // counterexample's value); `lo_pass` = the greatest value known to PASS (or lo-1 if lo itself fails).
        let decoded = |payload: i64| lo.wrapping_add((payload & i64::MAX).rem_euclid(hi - lo + 1));
        let mut hi_fail = decoded(best[1]); // the current failing value
        let mut lo_pass = lo - 1; // exclusive lower fence: everything <= lo_pass is presumed passing
        while lo_pass + 1 < hi_fail {
            let mid = lo_pass + (hi_fail - lo_pass) / 2;
            // Run the property at decoded value `mid` (pool payload = mid - lo), without holding a borrow of
            // `best` across the mutation below.
            let outcome = {
                let mut trial = best.clone();
                trial[1] = mid.wrapping_sub(lo);
                run_pool(&trial).0
            };
            match outcome {
                TrialOutcome::Fail(m) => {
                    hi_fail = mid; // mid fails → the boundary is at or below mid
                    best[1] = mid.wrapping_sub(lo);
                    best_msg = m;
                }
                _ => lo_pass = mid, // mid passes → the boundary is above mid
            }
        }
        return PropertyFailure {
            inputs: best.iter().map(|n| n.to_string()).collect(),
            message: best_msg,
        };
    }
    for i in 0..best.len() {
        let mut n = best[i];
        while n != 0 {
            n /= 2;
            let mut trial = best.clone();
            trial[i] = n;
            // Re-run with the candidate prefix (the runner pads with the untouched trailing pool via the
            // original size is unnecessary — the consumed prefix is what the generator reads in order).
            let (outcome, _) = run_pool(&trial);
            if matches!(outcome, TrialOutcome::Fail(_)) {
                best[i] = n;
                if let (TrialOutcome::Fail(m), _) = run_pool(&best) {
                    best_msg = m;
                }
            } else {
                break; // this position can't shrink further while still failing
            }
        }
    }
    PropertyFailure {
        inputs: best.iter().map(|n| n.to_string()).collect(),
        message: best_msg,
    }
}

/// Generate one `--arg` string per generator, from a driver seeded at `seed` — bolero's `driver::Rng`
/// (a seeded, reproducible driver) feeding each type's `ValueGenerator`. The rendered forms are exactly
/// what `cdz-run`'s `coerce_one` parses (`5`, `-3`, `true`, `1.5`, a single char).
///
/// The generation is a pure function of `seed`: the same seed re-produces the same inputs on every run, so
/// a property run is reproducible from its recorded seed (`run_property` seeds trial `t` at `seed + t`, and
/// `--seed` replays the exact pool). This is what lets a reported failure be replayed deterministically.
//= spec/capabilities/property-based-testing.md#generation-is-seeded-and-reproducible
//# A property run MUST be reproducible from its recorded seed, producing the same inputs on every conforming run.
/// Generate one input tuple, SATISFYING the `@requires` constraints: per-param `bounds` are applied by
/// clamping in `draw_inputs`, and cross-param `relations` (e.g. `(< a b)`) are satisfied by REJECTION
/// SAMPLING — re-draw from a fresh derived seed until every relation holds, bounded by `RELATION_FUEL`
/// re-draws. Clamping keeps generation a pure function of the seed for the common (no-relation) case, so
/// reproducibility is unchanged; when relations ARE present, the returned tuple is still a deterministic
/// function of `seed` (the same seed re-derives the same accepted draw). If fuel is exhausted (a relation
/// too tight to hit by sampling), the last draw is returned unchanged — the (D) precondition trap then
/// fires and the property reports honestly rather than looping forever.
#[cfg(feature = "standalone")]
pub(crate) fn generate_inputs(
    gens: &[GenKind],
    bounds: &[ParamBound],
    relations: &[Relation],
    seed: u64,
) -> Vec<String> {
    // Draw, then PROPAGATE equalities (copy left→right so `(= a b)` holds by construction), then check the
    // remaining ORDER relations. Propagation is applied to every attempt so the order check sees the
    // post-propagation values.
    let mut first = draw_inputs(gens, bounds, seed);
    propagate_equalities(relations, &mut first);
    if relations.is_empty() || relations_hold(relations, &first) {
        return first;
    }
    // An ORDER relation is still violated — re-draw from a distinct derived seed until all hold, bounded by
    // fuel. The derived seed `seed ^ (k * ODD)` keeps every attempt a deterministic function of the original
    // seed. (Equalities always hold post-propagation, so only an unsatisfiable order relation exhausts fuel.)
    const RELATION_FUEL: u64 = 256;
    for k in 1..=RELATION_FUEL {
        let mut candidate = draw_inputs(gens, bounds, seed ^ k.wrapping_mul(0x9E3779B97F4A7C15));
        propagate_equalities(relations, &mut candidate);
        if relations_hold(relations, &candidate) {
            return candidate;
        }
    }
    first // fuel exhausted: return the first draw; the precondition trap reports honestly
}

/// Draw one input tuple from `seed`, applying only the per-param `bounds` clamps (no relation handling —
/// that is `generate_inputs`'s rejection loop). Split out so the rejection loop can re-draw cheaply.
#[cfg(feature = "standalone")]
pub(crate) fn draw_inputs(gens: &[GenKind], bounds: &[ParamBound], seed: u64) -> Vec<String> {
    use bolero_generator::driver::{self, Rng};
    use bolero_generator::{ValueGenerator, produce};
    let rng = rand_from_seed(seed);
    let mut d = Rng::new(rng, &driver::Options::default());
    gens.iter()
        .enumerate()
        .map(|(i, g)| match g {
            GenKind::Bool => {
                // A `@requires(b)` bare-Bool precondition pins this param (`bool_force`); otherwise draw
                // randomly. Pinning (not re-draw) keeps generation a pure function of the seed.
                match bounds.get(i).and_then(|b| b.bool_force) {
                    Some(forced) => forced.to_string(),
                    None => produce::<bool>()
                        .generate(&mut d)
                        .unwrap_or(false)
                        .to_string(),
                }
            }
            GenKind::Char => produce::<char>()
                .generate(&mut d)
                .unwrap_or('a')
                .to_string(),
            GenKind::Float => {
                let v = produce::<f64>().generate(&mut d).unwrap_or(0.0);
                // Render a finite decimal `coerce_one` (`parse::<f64>`) accepts; a non-finite generated
                // value falls back to 0 (NaN/inf have no re-parseable decimal here).
                if v.is_finite() { v } else { 0.0 }.to_string()
            }
            GenKind::Int { signed, width } => {
                let raw = produce::<i64>().generate(&mut d).unwrap_or(0);
                // `@requires`-constrained generation: if this param carries a recognized integer bound,
                // CLAMP the drawn value into it so the drawn input SATISFIES the precondition (the (D)
                // body-entry enforcement traps a violated pre — an out-of-domain draw would spuriously fail
                // the test). Clamp (not re-draw) keeps generation a pure function of the seed, so the
                // reproducibility contract (a replayed seed reproduces the inputs) still holds. An
                // unconstrained param is unchanged. The clamp is in the value's own signed i128 space, then
                // `render_int` re-narrows to the width.
                let bounded = match bounds.get(i) {
                    Some(b) if b.is_constrained() => b.clamp(raw as i128),
                    _ => raw as i128,
                };
                render_int(bounded as i64, *signed, *width)
            }
        })
        .collect()
}

/// The maximum number of cases `@exhaustive` will enumerate. A domain larger than this is treated as
/// unbounded (`exhaustive_domain` returns `None`) — enumerating millions of cases would be a denial of
/// service, not a proof. `Bool`×`Bool` = 4, a `UInt8` = 256, `UInt8`×`Bool` = 512 all fit comfortably;
/// a 16-bit int (65 536) fits; a 32/64-bit int or a float does not (narrow the type to prove exhaustively).
#[cfg(feature = "standalone")]
pub(crate) const MAX_EXHAUSTIVE_CASES: usize = 100_000;

/// The COMPLETE input domain of a property whose parameters are all bounded scalars — every combination of
/// each parameter's full value set, as rendered `--arg` strings (the Cartesian product). `None` if the
/// domain is unbounded/too large (any `Float`, or an integer width whose range times the running product
/// would exceed [`MAX_EXHAUSTIVE_CASES`]) — such a property cannot be exhaustively proven and must narrow
/// its types. An empty `gens` (a nullary signature) yields one case (the empty argument list), though the
/// exhaustive path is only taken for a parameterized boundary-arg test.
#[cfg(feature = "standalone")]
pub(crate) fn exhaustive_domain(gens: &[GenKind]) -> Option<Vec<Vec<String>>> {
    // Build the per-parameter value sets (each the full rendered domain of that scalar), bailing if any is
    // unbounded, while tracking the running product so we stop before building an enormous set.
    let mut per_param: Vec<Vec<String>> = Vec::with_capacity(gens.len());
    let mut product: usize = 1;
    for g in gens {
        let values = scalar_domain(g)?;
        product = product.checked_mul(values.len())?;
        if product > MAX_EXHAUSTIVE_CASES {
            return None;
        }
        per_param.push(values);
    }
    // Cartesian product of the per-parameter value sets, in row-major order (last parameter varies
    // fastest), seeded with one empty tuple.
    let mut domain: Vec<Vec<String>> = vec![Vec::new()];
    for values in &per_param {
        let mut next = Vec::with_capacity(domain.len() * values.len());
        for prefix in &domain {
            for v in values {
                let mut row = prefix.clone();
                row.push(v.clone());
                next.push(row);
            }
        }
        domain = next;
    }
    Some(domain)
}

/// The full rendered value domain of ONE bounded scalar generator, or `None` if it is unbounded/too large.
/// `Bool` = {false, true}; `Char` is bounded but astronomically large (all Unicode scalars) so it is not
/// enumerated here; an integer is enumerable only for narrow widths (≤16 bits) whose range fits within
/// [`MAX_EXHAUSTIVE_CASES`]; a `Float` is never enumerable. Each value is rendered exactly as
/// `generate_inputs` renders it (so `cdz-run`'s `coerce_one` accepts it).
#[cfg(feature = "standalone")]
pub(crate) fn scalar_domain(g: &GenKind) -> Option<Vec<String>> {
    match g {
        GenKind::Bool => Some(vec!["false".to_string(), "true".to_string()]),
        // A `Char`'s domain is every Unicode scalar (~1.1M) — far past the cap; a float is infinite. Not
        // exhaustively enumerable (narrow to a bounded integer/Bool instead).
        GenKind::Char | GenKind::Float => None,
        GenKind::Int { signed, width } => {
            // Only widths whose FULL range fits the cap are enumerable (8/16-bit); 32/64-bit are unbounded
            // for this purpose. Enumerate the whole range, rendered via `render_int` (same as sampling).
            let range: Vec<i64> = match (signed, width) {
                (false, 8) => (0..=u8::MAX as i64).collect(),
                (true, 8) => (i8::MIN as i64..=i8::MAX as i64).collect(),
                (false, 16) => (0..=u16::MAX as i64).collect(),
                (true, 16) => (i16::MIN as i64..=i16::MAX as i64).collect(),
                _ => return None, // 32/64-bit (or a deferred width) — too large to enumerate
            };
            Some(
                range
                    .into_iter()
                    .map(|v| render_int(v, *signed, *width))
                    .collect(),
            )
        }
    }
}

/// Render a generated `i64` as the decimal text for an integer parameter of the given signedness/width,
/// truncated into that width's range so `cdz-run`'s `parse::<iN/uN>` accepts it (a wider raw value would
/// fail to parse as the narrower type). The `as` truncation into range keeps the full value spread.
#[cfg(feature = "standalone")]
pub(crate) fn render_int(raw: i64, signed: bool, width: u32) -> String {
    match (signed, width) {
        (true, 8) => (raw as i8).to_string(),
        (false, 8) => (raw as u8).to_string(),
        (true, 16) => (raw as i16).to_string(),
        (false, 16) => (raw as u16).to_string(),
        (true, 32) => (raw as i32).to_string(),
        (false, 32) => (raw as u32).to_string(),
        (false, 64) => (raw as u64).to_string(),
        // signed 64 (and any deferred/other width defaults to i64 at the boundary).
        _ => raw.to_string(),
    }
}

/// A reproducible RNG from a `u64` seed — `Xoshiro256PlusPlus` (bolero's own generator rng), whose
/// `seed_from_u64` SplitMix64-expands the seed to the full state, so `cdz test --seed N` is deterministic
/// without depending on OS entropy.
#[cfg(feature = "standalone")]
pub(crate) fn rand_from_seed(seed: u64) -> rand_xoshiro::Xoshiro256PlusPlus {
    use rand_core::SeedableRng;
    use rand_xoshiro::Xoshiro256PlusPlus;
    Xoshiro256PlusPlus::seed_from_u64(seed)
}

/// SHRINK a failing property input toward a minimal counterexample: for each argument position, try
/// replacing it with progressively "smaller" values (an integer toward 0 by halving, a bool toward
/// `false`, a float toward 0, a char toward `a`) and keep any replacement that STILL fails. Greedy +
/// bounded — one left-to-right pass per position, each position bisected — so it terminates quickly and
/// reports a smaller, more legible witness than the raw random input. Returns the shrunk inputs + the
/// (possibly updated) failure message from the last failing run.
#[cfg(feature = "standalone")]
pub(crate) fn shrink(
    gens: &[GenKind],
    bounds: &[ParamBound],
    relations: &[Relation],
    inputs: &[String],
    message: Option<String>,
    run_one: &dyn Fn(&[String]) -> TrialOutcome,
) -> (Vec<String>, Option<String>) {
    let mut best = inputs.to_vec();
    let mut best_msg = message;
    for (i, g) in gens.iter().enumerate() {
        for candidate in shrink_candidates(g, &best[i]) {
            // A shrink candidate must still SATISFY this param's `@requires` bound — otherwise shrinking an
            // integer toward 0 could push it out of the precondition domain, and the (D) body-entry trap
            // would be mistaken for "still fails", yielding an out-of-domain (spurious) counterexample.
            // Skip a candidate the bound rejects (an unconstrained param admits every candidate).
            if let Some(b) = bounds.get(i)
                && b.is_constrained()
                && let Ok(n) = candidate.parse::<i64>()
                && b.clamp(n as i128) != n as i128
            {
                continue;
            }
            // Likewise a BOOL param pinned by `@requires(b)` must not shrink off its forced value — shrinking
            // `true`→`false` would break the precondition and trip the (D) pre-trap (a spurious "still fails").
            if let Some(b) = bounds.get(i)
                && let Some(forced) = b.bool_force
                && candidate != forced.to_string()
            {
                continue;
            }
            let mut trial = best.clone();
            trial[i] = candidate;
            // PROPAGATE equalities first: shrinking the LEFT param of `(= a b)` must carry to the right so the
            // pair stays equal (a shrink of the right param is a copy, harmlessly overwritten — the right is
            // slaved to the left). Then a shrink must not break a cross-param ORDER RELATION (`(< a b)`) —
            // shrinking `b` toward 0 could make `a < b` false, and the (D) trap would masquerade as "still
            // fails". Skip a trial that violates any relation (no relations ⇒ admits every candidate).
            propagate_equalities(relations, &mut trial);
            if !relations_hold(relations, &trial) {
                continue;
            }
            if let TrialOutcome::Fail(m) = run_one(&trial) {
                best = trial;
                best_msg = m;
            }
        }
    }
    (best, best_msg)
}

/// The ordered shrink candidates for one argument (largest-reduction first), by kind: an integer halves
/// toward 0 (then 0); a bool toward `false`; a float toward 0; a char toward `a`. Each is a value that,
/// if it still fails, is a smaller witness than the current one.
#[cfg(feature = "standalone")]
pub(crate) fn shrink_candidates(g: &GenKind, current: &str) -> Vec<String> {
    match g {
        GenKind::Int { .. } => {
            let Ok(mut n) = current.parse::<i64>() else {
                return Vec::new();
            };
            let mut out = Vec::new();
            // Halve toward 0 (a geometric descent), ending at 0 — a bounded sequence.
            while n != 0 {
                n /= 2;
                out.push(n.to_string());
            }
            out
        }
        GenKind::Bool => {
            if current == "true" {
                vec!["false".to_string()]
            } else {
                Vec::new()
            }
        }
        GenKind::Float => {
            if current != "0" {
                vec!["0".to_string()]
            } else {
                Vec::new()
            }
        }
        GenKind::Char => {
            if current != "a" {
                vec!["a".to_string()]
            } else {
                Vec::new()
            }
        }
    }
}
