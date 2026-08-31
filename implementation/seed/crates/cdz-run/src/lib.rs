//! The generic wasm-component runner.
//!
//! One job: instantiate a finished component, compose the value-heap runtime when the component
//! imports it, invoke a chosen export with typed arguments, and render the result to canonical text.
//! Everything wasmtime lives here; callers hand in bytes and get back a [`Outcome`].
//!
//! The compiler is never in this crate's dependency graph — running a component needs no compiler —
//! so `cdz-run` stays a pure consumer of finished artifacts (component-abi.md).

use anyhow::{Result, anyhow};
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Linker, Type, Val};
use wasmtime::{Config, Engine, Store};
// `OptLevel` is a cranelift-only Config knob — only in scope for a compiler-enabled build.
#[cfg(feature = "cranelift")]
use wasmtime::OptLevel;

/// The command surface (`RunArgs` + `run`), embeddable so the unified `cdz` binary can mount `cdz run`.
pub mod cli;

/// The wasmtime engine for a run. `cdz-run` is a ONE-SHOT tool: it JIT-compiles the component, invokes
/// an export ONCE, and exits — so Cranelift's optimizing backend (the `Engine::default()` `OptLevel::
/// Speed`) spends compile time that the single execution never repays. `OptLevel::None` skips the
/// optimization passes: the generated code is slower per-instruction, but the compile is much faster,
/// and for a tiny gate program (which runs once) the total `Component::new`→run time drops. This is the
/// dominant per-invocation cost across the gate's ~1000 spawns (cdz-run was the slowest pipeline stage).
fn engine() -> Engine {
    // ONE shared engine per process, returned as a cheap `Arc`-backed clone. This is a CORRECTNESS
    // requirement, not just an optimization: the epoch-ticker (`arm_epoch_ticker`) is spawned ONCE (a
    // `Once`) and increments the epoch of the engine it was handed. If `engine()` minted a FRESH engine
    // per call, the ticker would advance only the FIRST engine's epoch — every later run's `Store` deadline
    // would never fire, so a runaway loop on a second-or-later engine spins a core FOREVER (the deadline
    // trap silently never trips). That regression wedged pr-sync's `cargo test --workspace` gate (the loop
    // test hung at 99% CPU) because another run had already armed the ticker on a different engine first.
    // Sharing the engine means the ticker and every `Store` refer to the SAME epoch clock.
    static ENGINE: std::sync::OnceLock<Engine> = std::sync::OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let mut cfg = Config::new();
            // Cranelift-only Config knob (the JIT's optimizer). Skipped in a compiler-free
            // (`--no-default-features`) build, whose engine only ever `deserialize`s precompiled
            // artifacts — there is no cranelift to configure. `epoch_interruption` below is engine-level
            // (backend-independent), so the runaway-loop trap safety net holds for both builds.
            #[cfg(feature = "cranelift")]
            cfg.cranelift_opt_level(OptLevel::None);
            // EPOCH INTERRUPTION: cap an in-process run's wall-clock so a MISCOMPILED runtime-looping
            // program (e.g. an emitted body that loops forever) TRAPS at a deadline instead of spinning a
            // CPU core indefinitely. This is the durable fleet-health safety net: a runaway corpus case that
            // ran in-process with no cap previously starved pr-sync + flooded the host (64 orphaned
            // CPU-spinning `cdz run` procs wedged the integrator ~2h in one session). Each `Store` gets an
            // epoch deadline (see `new_store`), and a single background thread advances the engine's epoch on
            // a fixed tick, so the deadline is wall-clock-bounded. `epoch_interruption(true)` is what makes
            // `set_epoch_deadline` effective.
            cfg.epoch_interruption(true);
            // A fresh `Config` can only fail to build an `Engine` on an unsupported target/feature
            // combination, which this host supports; fall back to the default engine if that ever changes
            // rather than panic.
            Engine::new(&cfg).unwrap_or_default()
        })
        .clone()
}

/// JIT-compile a finished component to a runnable [`Component`] — the compile choke-point (seq-250). Mirrors
/// wasmtime's own `#[cfg(any(cranelift, winch))]` gate on `Component::new`: with the `cranelift` feature ON
/// (default) it JIT-compiles; with it OFF (`--no-default-features`, the AOT corpus-exec build) no compiler
/// is linked, so it errors — that build reaches components only via `Component::deserialize` of a
/// precompiled `.cwasm`. Routing every compile through here is what lets the crate build in BOTH configs.
/// (Named `jit_*` to avoid colliding with the higher-level `compile_component` → [`CompiledComponent`].)
#[cfg(feature = "cranelift")]
fn jit_component(engine: &Engine, bytes: &[u8]) -> Result<Component> {
    Component::new(engine, bytes)
}
#[cfg(not(feature = "cranelift"))]
fn jit_component(_engine: &Engine, _bytes: &[u8]) -> Result<Component> {
    anyhow::bail!(
        "cdz-run was built without the `cranelift` feature and cannot JIT-compile a component; \
         this build runs only precompiled `.cwasm` artifacts via deserialize (the AOT corpus-exec path)"
    )
}

/// Load a runnable component from `bytes` honoring [`RunOpts::precompiled`] (seq-250): in precompiled mode
/// the bytes are a serialized `.cwasm` and are loaded with `Component::deserialize` (no compiler needed —
/// the cranelift-FREE exec path); otherwise they are a component `.wasm` JIT-compiled via [`jit_component`].
/// Composition + instantiation downstream are identical either way. Every guest/consumer/provider load on a
/// run path routes through here so `--precompiled` is honored uniformly.
fn load_guest(engine: &Engine, bytes: &[u8], opts: &RunOpts) -> Result<Component> {
    if opts.precompiled {
        // SAFETY: `bytes` is a `.cwasm` produced by our own `cdz-run --precompile-out` (the cranelift-ON
        // precompile tool) for a compatible engine. `deserialize` re-validates the artifact's embedded
        // compatibility header (wasmtime version + target + Config-compat set) and returns `Err` on any
        // mismatch, so a foreign/stale/tampered artifact is rejected here, never mis-executed.
        // A guest `.cwasm` may be SELF-FRAMED with its `cdz-result-type` section (see `frame_precompiled`);
        // strip the frame here so `deserialize` sees the raw serialized artifact. Every precompiled load
        // routes through here, so unframing at this ONE choke point keeps deserialize working for all
        // callers (run/grade/live-objects/peers); a RAW `.cwasm` (no magic — runtime/store/legacy) is
        // returned whole. The result-Ty MAP is read separately by the render paths (`result_types_of`).
        let (_rtypes, cwasm) = unframe_precompiled(bytes);
        unsafe { Component::deserialize(engine, cwasm) }
            .map_err(|e| anyhow!("deserialize precompiled component (.cwasm): {e}"))
    } else {
        jit_component(engine, bytes)
    }
}

/// Core-module compile choke-point — the `Module::new` companion of [`jit_component`], gated the same way
/// (wasmtime's `Module::new` is `#[cfg(any(cranelift, winch))]`).
#[cfg(feature = "cranelift")]
fn jit_module(engine: &Engine, bytes: &[u8]) -> Result<wasmtime::Module> {
    wasmtime::Module::new(engine, bytes)
}
#[cfg(not(feature = "cranelift"))]
fn jit_module(_engine: &Engine, _bytes: &[u8]) -> Result<wasmtime::Module> {
    anyhow::bail!(
        "cdz-run was built without the `cranelift` feature and cannot JIT-compile a core module; \
         this build runs only precompiled artifacts via deserialize"
    )
}

/// Precompile a finished component to a serialized AOT artifact (the `.cwasm` the corpus-exec `deserialize`s)
/// — seq-250's compile-once half. Requires a compiler, so it is `cranelift`-gated: v-nix's `cdz-precompile`
/// nix phase-bin (cranelift-ON, built once + cached) invokes this to emit per-case artifacts, which the
/// cranelift-free exec then runs. `Engine::precompile_component` validates + compiles the bytes; the output
/// loads only under an engine with a matching compatibility hash (see [`engine_fp`]).
#[cfg(feature = "cranelift")]
pub fn precompile_component_bytes(component_bytes: &[u8]) -> Result<Vec<u8>> {
    engine()
        .precompile_component(component_bytes)
        .map_err(|e| anyhow!("precompile component: {e}"))
}

/// Framing magic for a self-describing precompiled GUEST `.cwasm` that carries its `cdz-result-type` map
/// (corpus-28 nested-Bytes render fix). A serialized wasmtime `.cwasm` DROPS the component's custom
/// sections, so the cranelift-free AOT deserialize path (`opts.precompiled`) would render TYPE-BLIND — a
/// WIT-erased leaf (`list<u8>` as `Bytes`, `string` as `Symbol`) as `#list(…)`/raw instead of `b"…"`/`#"…"`,
/// diverging from the JIT path which scans the section (`compile_component`). To keep the AOT render
/// byte-identical, [`frame_precompiled`] PREPENDS the guest's `cdz-result-type` section here and
/// [`unframe_precompiled`] splits it back off before `Component::deserialize`. GUEST-ONLY by construction:
/// only the guest component wasm carries the section (it holds the guest's EXPORT result types), so the
/// runtime/store `.cwasm` — which have no such section — stay RAW automatically (frame iff the scan is
/// non-empty). A raw `.cwasm` (no magic — every runtime/store/pre-existing artifact) → empty map = the
/// prior type-blind behavior, so this is fully back-compatible.
const CDZ_CWASM_RTYPES_MAGIC: &[u8; 8] = b"CDZRTYP1";

/// Frame a serialized guest `.cwasm` with its `cdz-result-type` section: `MAGIC ‖ len(u32-le) ‖ rtypes ‖
/// cwasm`. `None` rtypes (a component with no section — the runtime/store precompiles) → the raw `cwasm`
/// unframed, so those artifacts are byte-identical to before. See [`CDZ_CWASM_RTYPES_MAGIC`].
///
/// `cranelift`-gated: the ONLY caller is `--precompile-out` (`cli::precompile_to`, itself
/// `#[cfg(feature = "cranelift")]` — precompilation needs the compiler). The cranelift-FREE build reaches
/// artifacts only via `unframe_precompiled` (deserialize side, ungated), so without this cfg `frame_precompiled`
/// is dead code in the `--no-default-features` config (delegate-compile / syntax-roundtrip) and trips `-D warnings`.
#[cfg(feature = "cranelift")]
pub(crate) fn frame_precompiled(cwasm: Vec<u8>, rtypes: Option<Vec<u8>>) -> Vec<u8> {
    match rtypes {
        None => cwasm,
        Some(rt) => {
            let mut out =
                Vec::with_capacity(CDZ_CWASM_RTYPES_MAGIC.len() + 4 + rt.len() + cwasm.len());
            out.extend_from_slice(CDZ_CWASM_RTYPES_MAGIC);
            out.extend_from_slice(&(rt.len() as u32).to_le_bytes());
            out.extend_from_slice(&rt);
            out.extend_from_slice(&cwasm);
            out
        }
    }
}

/// Split a possibly-framed precompiled `.cwasm` into `(result_type_section?, cwasm)` — the inverse of
/// [`frame_precompiled`]. A raw `.cwasm` (no magic: runtime/store/legacy) → `(None, whole)`. TOTAL: a
/// truncated/malformed frame falls back to raw (never a panic; `Component::deserialize` rejects genuinely
/// bad bytes). The returned `cwasm` slice is what `Component::deserialize` receives.
pub(crate) fn unframe_precompiled(bytes: &[u8]) -> (Option<&[u8]>, &[u8]) {
    let hdr = CDZ_CWASM_RTYPES_MAGIC.len();
    if bytes.len() >= hdr + 4 && &bytes[..hdr] == CDZ_CWASM_RTYPES_MAGIC.as_slice() {
        let len = u32::from_le_bytes([bytes[hdr], bytes[hdr + 1], bytes[hdr + 2], bytes[hdr + 3]])
            as usize;
        let rt_start = hdr + 4;
        if let Some(rt_end) = rt_start.checked_add(len)
            && rt_end <= bytes.len()
        {
            return (Some(&bytes[rt_start..rt_end]), &bytes[rt_end..]);
        }
    }
    (None, bytes)
}

/// The guest export result-Ty map for a component about to run — from EITHER source, so every render path
/// is typed regardless of JIT-vs-AOT: on the JIT path (`!precompiled`, `component_bytes` is a `.wasm`) the
/// `cdz-result-type` custom section is byte-scanned; on the cranelift-free AOT path (`precompiled`,
/// `component_bytes` is a `.cwasm` whose serialize DROPPED the section) it is read from the self-frame
/// `--precompile-out` prepended (see [`frame_precompiled`]). Empty (no section / raw `.cwasm`) → type-blind.
fn result_types_of(
    component_bytes: &[u8],
    opts: &RunOpts,
) -> std::collections::HashMap<String, cadenza_syntax::ast::Arenas> {
    if opts.precompiled {
        parse_result_types(unframe_precompiled(component_bytes).0)
    } else {
        parse_result_types(scan_result_type_section(component_bytes).as_deref())
    }
}

/// The wall-clock a single in-process run may take before the epoch deadline TRAPS it. Generous — a
/// legitimate heap program is milliseconds; this only fires on a genuine runaway loop. `CDZ_RUN_TIMEOUT_SECS`
/// overrides (0 disables — for a debugger). The epoch ticker below advances one epoch per `EPOCH_TICK`, so
/// the deadline is `RUN_TIMEOUT / EPOCH_TICK` ticks.
fn run_timeout_secs() -> u64 {
    std::env::var("CDZ_RUN_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
}

/// The epoch-tick interval — the background thread bumps the engine's epoch this often, so a run's epoch
/// deadline resolves to a wall-clock bound with this granularity.
const EPOCH_TICK: std::time::Duration = std::time::Duration::from_millis(100);

/// The ceiling on the load-scale factor (see [`scale_from_load`]). A genuine runaway loop is CPU-bound and
/// always runnable, so it burns its epoch deadline in ~real time and still TRAPS within `MAX_LOAD_SCALE ×`
/// the base timeout — bounding the worst-case trap latency (e.g. the default 30s → at most 480s under a
/// pathological load spike) so the safety net can never be defeated, only stretched.
const MAX_LOAD_SCALE: u64 = 16;

/// The pure oversubscription-factor computation for the epoch deadline, split out for a load-independent
/// unit test. `load1` is the 1-minute run-queue length, `ncpu` the core count; their ratio is roughly how
/// many runnable threads share each core, i.e. how much longer than its CPU time a correct run takes in
/// WALL clock. Returns that ratio (rounded UP, so a hair over full utilization already grants headroom),
/// clamped to `[1, MAX_LOAD_SCALE]`: at or below full utilization → 1 (no stretch, identical prior
/// behavior), and any degenerate input (non-finite, `ncpu < 1`) → 1 (fail safe to the unscaled deadline).
fn scale_from_load(load1: f64, ncpu: f64) -> u64 {
    if !load1.is_finite() || !ncpu.is_finite() || ncpu < 1.0 {
        return 1;
    }
    let factor = (load1 / ncpu).ceil();
    if !factor.is_finite() || factor < 1.0 {
        1
    } else {
        (factor as u64).min(MAX_LOAD_SCALE)
    }
}

/// The live epoch-deadline stretch for THIS host's current load. The deadline is WALL-CLOCK (the ticker
/// bumps the epoch every `EPOCH_TICK` regardless of whether the guest is scheduled on a core), so under CPU
/// oversubscription — a herd of `cdz-run` processes on one box, `loadavg` well above `ncpu` — a CORRECT
/// trivial run is descheduled off-core and can trip a deadline it would never reach on an idle box. That is
/// a load-induced FALSE `interrupt` trap, and it corrupted pr-sync's merge-gate verdicts on trivial cases
/// every herd (routed by v-fleet-tooling). Scaling the budget by `loadavg/ncpu` restores a starved run's
/// true CPU budget. Linux-only signal (`/proc/loadavg`); anywhere it can't be read/parsed → factor 1
/// (unchanged behavior, so non-Linux and a missing procfs both fall back to the plain wall-clock deadline).
fn load_scale_factor() -> u64 {
    let ncpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f64;
    match std::fs::read_to_string("/proc/loadavg") {
        Ok(s) => match s
            .split_whitespace()
            .next()
            .and_then(|f| f.parse::<f64>().ok())
        {
            Some(load1) => scale_from_load(load1, ncpu),
            None => 1,
        },
        Err(_) => 1,
    }
}

/// Create a `Store` with the run's epoch deadline armed (trap on deadline), and ensure the background
/// epoch-ticker for `engine` is running. Every in-process run goes through here so a runaway loop can't
/// escape the wall-clock cap. A `run_timeout_secs()` of 0 disables the deadline (unbounded — for a
/// debugger); otherwise the deadline is `ceil(timeout / EPOCH_TICK)` ticks and the store traps past it.
fn new_store(engine: &Engine) -> Store<()> {
    let mut store = Store::new(engine, ());
    // Past the deadline: TRAP (surfaces as Outcome::Trap — "a runaway loop"), not yield/callback. Set on
    // EVERY store because the shared engine has `epoch_interruption(true)`, under which a store's DEFAULT
    // epoch deadline is 0 — and since the ticker starts the engine epoch at 0, `epoch(0) >= deadline(0)`
    // trips at the FIRST function entry. So a store with no explicit deadline traps INSTANTLY, which is
    // why the `secs == 0` "unbounded" path must set an effectively-infinite deadline rather than skip it.
    store.epoch_deadline_trap();
    let secs = run_timeout_secs();
    if secs > 0 {
        arm_epoch_ticker(engine);
        // `secs.saturating_mul(1000)`, NOT `secs * 1000`: an absurdly large `CDZ_RUN_TIMEOUT_SECS` (the
        // user reaching for "effectively unbounded") would otherwise overflow the `u64` millis product —
        // a PANIC in a debug build, and in release it WRAPS to a tiny value, so a huge timeout inverts into
        // a near-instant trap (the same class of inversion as the `secs == 0` bug). Saturating means a
        // giant timeout clamps to a giant tick count (≈ never reached), which is the intended "unbounded".
        // The `load_scale_factor()` (≥1) STRETCHES the wall-clock budget under CPU oversubscription so a
        // correct-but-off-core run isn't false-trapped as a runaway (see `load_scale_factor`); it composes
        // as another `saturating_mul` so the overflow-safety above is preserved.
        let ticks = secs
            .saturating_mul(1000)
            .saturating_mul(load_scale_factor())
            .div_ceil(EPOCH_TICK.as_millis() as u64)
            .max(1);
        store.set_epoch_deadline(ticks);
    } else {
        // `CDZ_RUN_TIMEOUT_SECS=0` = the documented UNBOUNDED escape hatch (for a debugger / a legitimately
        // long run). Set an effectively-infinite deadline (`u64::MAX` ticks) rather than SKIP the call: with
        // `epoch_interruption(true)` a store's default deadline is 0, so skipping would trap immediately
        // (the exact inversion breaker found — every program, even `(def (main) 7)`, hit `trap: interrupt`).
        // No ticker is armed, so the epoch never advances and `u64::MAX` is never reached: truly unbounded.
        store.set_epoch_deadline(u64::MAX);
    }
    store
}

/// Start (once per engine, idempotent) a detached background thread that advances `engine`'s epoch every
/// `EPOCH_TICK`. It holds a `Weak<Engine>`-style clone: `Engine` is refcounted + cheap to clone, and the
/// thread simply keeps ticking for the process lifetime (a one-shot tool exits soon after). Keyed on the
/// engine's identity so repeated `new_store` calls in one process don't spawn a thread each.
fn arm_epoch_ticker(engine: &Engine) {
    use std::sync::Once;
    // `cdz-run` builds ONE engine per process in practice (the callers each call `engine()` once per run,
    // and a batch process reuses the same run path) — a single ticker suffices. A `Once` guards the spawn.
    static TICKER: Once = Once::new();
    let engine = engine.clone();
    TICKER.call_once(move || {
        std::thread::Builder::new()
            .name("cdz-run-epoch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(EPOCH_TICK);
                    engine.increment_epoch();
                }
            })
            .ok();
    });
}

/// Load the value-heap runtime as a `Component`, reusing a CACHED compiled artifact when possible.
///
/// JIT-compiling the ~67KB runtime component is ~75ms, and it is BYTE-IDENTICAL for every heap program
/// (fixed by its content hash), yet `cdz-run` spawns fresh per program — so the gate recompiled the
/// SAME runtime hundreds of times. With `opts.runtime_cache_dir` set, the first run compiles + writes
/// `<dir>/<hash>-wt<engine-fingerprint>.cwasm` (an engine-compatibility fingerprint in the name), and
/// every later run `deserialize`s that (~0.25ms — a ~300× drop on runtime composition).
///
/// Safety: `Component::deserialize` is `unsafe` because arbitrary bytes could be malformed, but the
/// deserialize path (`load_code` → `serialization::check_compatible`) VALIDATES the artifact's embedded
/// header — the exact wasmtime version (`env!("CARGO_PKG_VERSION")`, string-equal), the native-host ISA
/// flags, and the compiler `Metadata` — and returns `Err` on any mismatch rather than misbehaving. So a
/// stale/incompatible `.cwasm` is REJECTED, not misread, EVEN IF the filename didn't distinguish it.
///
/// The filename fingerprint is therefore NOT the soundness net — that's the header check — it is what
/// keeps DIFFERENT engine versions/configs from THRASHING one shared path (each deserialize-failing then
/// overwriting the other's file). It comes from `Engine::precompile_compatibility_hash()`, wasmtime's
/// purpose-built AOT-cache fingerprint: its doc guarantees that if the hash matches between two engines,
/// an artifact from one deserializes in the other. So it captures the wasmtime version AND every `Config`
/// input that affects the compiled bytes — strictly stronger, and self-maintaining across a wasmtime
/// bump, than the old `CARGO_PKG_VERSION_MAJOR` of `cdz-run` (which is perma-`0`, since cdz-run is
/// `0.0.0` — so the version component NEVER changed across a wasmtime upgrade, and every artifact shared
/// the one `wt0` path). We only ever read a file THIS binary itself wrote, and any `deserialize` error
/// falls straight through to a fresh `Component::new`. So the cache can only make a run faster, never
/// change what it does.
///
/// A short hex fingerprint of the engine's AOT compatibility, for use in the `.cwasm` filename. Derived
/// from `Engine::precompile_compatibility_hash()` (wasmtime's guarantee: equal hash ⇒ artifacts
/// interchange), so it changes whenever a wasmtime bump or a `Config` change would make old artifacts
/// deserialize-fail — giving those artifacts a distinct path instead of thrashing a shared one. Fed
/// through `DefaultHasher` only to render the opaque `impl Hash` as a compact stable hex token.
#[cfg(feature = "cranelift")]
fn engine_fp(engine: &Engine) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut h);
    format!("{:016x}", h.finish())
}
/// Compiler-free (`--no-default-features`) build: `Engine::precompile_compatibility_hash` is cranelift-gated
/// in wasmtime, and this build never WRITES an fp-named cache artifact (it cannot compile) — the fingerprint
/// is only ever consumed to NAME a `.cwasm` lookup path. A fixed sentinel keeps the cache-path code compiling
/// unchanged; the AOT corpus-exec is driven by explicit artifact paths (v-nix's precompile pipeline), never
/// by fp-named cache probing, and `Component::deserialize` still self-validates artifact compatibility.
#[cfg(not(feature = "cranelift"))]
fn engine_fp(_engine: &Engine) -> String {
    "no-cranelift".to_string()
}

fn load_runtime_component(
    engine: &Engine,
    runtime_bytes: &[u8],
    opts: &RunOpts,
) -> Result<Component> {
    // PRECOMPILED (seq-250): `runtime_bytes` is already a serialized `.cwasm` (the shared runtime artifact,
    // precompiled ONCE by the cranelift-ON tool and passed via `--runtime`). Deserialize it directly —
    // skip the fp-named JIT cache dance entirely (the cranelift-free exec cannot compile a cache miss, and
    // its `engine_fp` is a sentinel that would never match the tool's artifact name anyway).
    if opts.precompiled {
        // SAFETY: same contract as `load_guest` — a `.cwasm` from our own precompile tool; `deserialize`
        // re-validates the compatibility header and errs on mismatch.
        return unsafe { Component::deserialize(engine, runtime_bytes) }
            .map_err(|e| anyhow!("deserialize precompiled value-heap runtime (.cwasm): {e}"));
    }
    let Some(dir) = opts.runtime_cache_dir.as_deref() else {
        // No cache configured — compile directly.
        return jit_component(engine, runtime_bytes)
            .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"));
    };
    // `<hash>-wt<engine-fingerprint>.cwasm`: the runtime's content address pins the SOURCE, the engine
    // fingerprint pins the COMPILER (wasmtime version + every `Config` input affecting the artifact), so a
    // cache file is only ever consulted for the exact runtime+engine it was made for.
    //
    // Key by the content address of the ACTUAL `runtime_bytes` we are compiling — NOT the component's
    // recorded requirement (`req.hash`). In the normal store-resolved path these are equal (`resolve_runtime`
    // content-verifies the stored bytes against `req.hash`), so the release cwasm keeps the SAME key and is
    // still reused. But with an explicit `--runtime <path>` override the override bytes have a DIFFERENT
    // address than `req.hash`; keying by `req.hash` there served the ALREADY-cached RELEASE cwasm and
    // silently ignored the override bytes — the bug that made a `--runtime <debug> --store <store>` run
    // execute the RELEASE runtime, so `--report-live-objects` read the shipped counter-less build and printed
    // a vacuous 0. Hashing the bytes we actually compile makes the cwasm a true content-addressed cache: the
    // debug runtime gets its own key and can never collide with the release one.
    let hash = crate::cli::content_address(runtime_bytes);
    let cache_path = dir.join(format!("{hash}-wt{}.cwasm", engine_fp(engine)));
    let cache_path = Some(cache_path);

    // Fast path: a cached artifact that deserializes cleanly.
    if let Some(path) = &cache_path
        && let Ok(bytes) = std::fs::read(path)
    {
        // SAFETY: bytes were produced by THIS binary's `Component::serialize` (below) for this exact
        // engine config + wasmtime version; `deserialize` re-checks that header and errs on mismatch,
        // so a corrupt/foreign file is rejected here rather than trusted.
        match unsafe { Component::deserialize(engine, &bytes) } {
            Ok(c) => return Ok(c),
            Err(_) => { /* stale/incompatible — fall through to recompile + rewrite */ }
        }
    }

    // Slow path: compile once, then persist the compiled artifact for next time (best-effort — a write
    // failure just means the next run recompiles, never an error).
    let component = jit_component(engine, runtime_bytes)
        .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"))?;
    if let Some(path) = &cache_path
        && let Ok(serialized) = component.serialize()
    {
        // Write to a temp sibling then rename, so a concurrent reader never sees a half-written file
        // (the gate runs cdz-run in parallel). A collision on the temp name is harmless — last writer
        // wins and the content is identical.
        let tmp = path.with_extension(format!("cwasm.tmp.{}", std::process::id()));
        if std::fs::write(&tmp, &serialized).is_ok() {
            let _ = std::fs::rename(&tmp, path); // ignore: a lost race just recompiles next time
        }
    }
    Ok(component)
}

mod grade;
mod render;
pub use render::render_val;

/// The fixed identity of the value-heap runtime interface — the same for every program a generation
/// emits (component-abi.md §The Value-Heap Runtime Crosses By A Well-Known Import: the interface
/// identity is fixed at the declared-default location). A program imports it under this name plus a
/// content-address suffix (below), so this is the PREFIX the runtime import is recognized by.
const RUNTIME_IFACE: &str = "cadenza:runtime/heap";

/// The required runtime a component records: the exact import name it declares, and the content
/// address (hash) of the runtime that satisfies it. Per component-abi.md §The Emitted Component
/// Records Its Required Runtime, a program's runtime import name is `cadenza:runtime/heap@0.0.0+<hash>`
/// — the fixed interface plus the runtime's content address as semver build-metadata. The host reads
/// the hash back to resolve the exact runtime (§The Host Resolves The Runtime By Content Address).
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeReq {
    /// The verbatim import name the component declares — the linker MUST bind under exactly this.
    pub import_name: String,
    /// The content address (lowercase hex BLAKE3) the component requires, extracted from the name.
    pub hash: String,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The export returned; its result rendered to canonical text (`unit` for a no-result export).
    Value(String),
    /// The export trapped at run time (message).
    Trap(String),
}

/// Render a run error to a trap MESSAGE that surfaces the wasm trap REASON, not just the outer
/// "error while executing at wasm backtrace:" wrapper anyhow prints. wasmtime attaches a
/// [`wasmtime::Trap`] code to a trapping error's chain; its `Display` is the canonical reason
/// (`integer divide by zero`, `integer overflow`, `wasm 'unreachable' instruction executed`, `out of
/// bounds memory access`, …). Surface that reason FIRST so a reason-matching consumer (the behavior
/// gate) can recognize the trap, then the full error for a human. A non-trap error (no `Trap` in the
/// chain) renders its whole anyhow CAUSE CHAIN inline (`{e:#}`), not just the outer message — so a HOST
/// func error (e.g. an exhausted `--host-response` list) surfaces its actionable reason, which wasmtime
/// otherwise buries under an "error while executing …" wrapper (see the `None` arm).
fn trap_message(e: &anyhow::Error) -> String {
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(trap) => format!("{trap}: {e:?}"),
        // A non-`Trap` error renders its whole CAUSE CHAIN inline (`{e:#}`), not just the outer message:
        // when a HOST func returns an error (e.g. an exhausted `--host-response` list) wasmtime wraps it as
        // "error while executing at wasm backtrace: …" and the actionable cause (`host call `E.op` has no
        // recorded response …`) is a chain LINK — the bare `{e}` printed only the wrapper (a fleet breaker
        // flagged this). `{e:#}` joins the chain (`outer: cause: root`) so the real reason is visible.
        None => format!("{e:#}"),
    }
}

/// How to run a component: which export, what arguments, and the value-heap runtime to compose.
#[derive(Debug, Default, Clone)]
pub struct RunOpts {
    /// The export to invoke. `None` selects the sole function export (by signature) — the common
    /// case for a scalar entry, whose ABI is `() -> scalar` and whose name the compiler emits verbatim.
    pub export: Option<String>,
    /// Raw, still-untyped argument strings from the CLI; coerced to the export's declared param types.
    pub args: Vec<String>,
    /// The value-heap runtime component bytes the caller resolved BY CONTENT ADDRESS. Required only
    /// when the component records a required runtime (see [`required_runtime`]); the caller is
    /// responsible for having fetched the runtime whose content address matches, and for binding it
    /// under the component's exact import name.
    pub runtime: Option<Vec<u8>>,
    /// Directory to cache the COMPILED runtime artifact in (normally the content-addressed store). JIT-
    /// compiling the 67KB runtime component is ~75ms and it is BYTE-IDENTICAL across every heap program
    /// — so, when set, `compose_runtime` writes `<dir>/<hash>.cwasm` on the first compile and
    /// `deserialize`s it (~0.25ms) on every later run. `None` disables the cache (always JIT). The
    /// cache is keyed by the runtime's content hash AND a wasmtime-version fingerprint in the filename,
    /// and a `deserialize` failure falls back to a fresh compile — so a version/config mismatch can
    /// never load an incompatible artifact.
    pub runtime_cache_dir: Option<std::path::PathBuf>,
    // NOTE: there is deliberately NO `nfc` field. The value-heap runtime imports
    // `cadenza:nfc/normalize@0.0.0+<hash>` (its NFC dependency, self-describing — the hash is stamped inline
    // into the import at build time), and the host resolves that NFC component from the store BY THAT INLINE
    // HASH at compose time (`resolve_nfc_by_hash`, keyed off `runtime_cache_dir`/`CDZ_STORE`/the default store
    // — a pure CAS lookup, NO `runtime.toml`/mapping) rather than the caller threading it through a field.
    // This is intentional: a required `nfc` field on this struct — constructed by ~190 test literals across
    // rcdzc/cdz/cdz-calc/cdz-smith — was a livelock magnet (every new literal that omitted it failed to
    // compile, and a merge-window couldn't stop an in-flight peer from adding one). Self-resolution removes the
    // field so no literal ever mentions NFC → future RunOpts field-adds can't reintroduce that race.
    /// The HOST-CALL RESPONSES (E2h) — the values the host returns to a program's delegated host calls,
    /// in call order. A program that delegates an effect to the host (`(host (E…) …)`) imports each
    /// operation as a boundary func; when it performs one, the bound host func returns the next response
    /// here (`capabilities-and-effects.md` §A Run Is A Deterministic Function Of Its Input And
    /// Responses). Empty for a program that makes no host call. Coerced to each call's declared result
    /// type at binding. The corpus `(host-responses …)` fixture supplies these.
    pub host_responses: Vec<HostResponse>,

    /// PRECOMPILED mode (seq-250 AOT corpus-exec): the component bytes handed to the run functions — the
    /// guest AND the `runtime` — are serialized `.cwasm` AOT artifacts (from `cdz-run --precompile-out`),
    /// to be loaded with `Component::deserialize` instead of JIT-compiled with `Component::new`. This is
    /// what lets the cranelift-FREE exec (`--no-default-features`) run a corpus program: it cannot compile,
    /// so every component reaches it pre-compiled. `false` (default) = the historical JIT path, unchanged.
    /// Composition/instantiation is IDENTICAL either way — only how the `Component`s are obtained differs.
    pub precompiled: bool,
}

/// One recorded host-call RESPONSE — the operation it answers and the value the host returns. The
/// operation name (`E.op`, dotted) pairs a response with its call for the ordered-consume model; the
/// value is a raw text form (`(: 10 Int64)`) coerced to the op's boundary result type at binding.
#[derive(Debug, Clone)]
pub struct HostResponse {
    /// The dotted operation name the response answers (e.g. `ask.ask`) — for the ordered model + a
    /// mismatch diagnostic. This increment consumes responses purely in ORDER (the op name is recorded
    /// for the diagnostic, not yet matched).
    pub op: String,
    /// The response value in canonical text form (`(: 10 Int64)` or a bare `10`) — coerced to the op's
    /// declared boundary result type when the host func is bound.
    pub value: String,
}

/// Validate `component_bytes` as a well-formed component — the cheap structural check before a run.
pub fn validate(component_bytes: &[u8]) -> Result<()> {
    let engine = engine();
    jit_component(&engine, component_bytes)
        .map(|_| ())
        .map_err(|e| anyhow!("invalid component: {e}"))
}

/// Instantiate `component_bytes`, compose the value-heap runtime if imported, invoke the chosen
/// export with the (coerced) arguments, and return the rendered outcome. The OBSERVED host calls are
/// discarded; use [`run_capturing`] to also get the ordered list of host operations the run performed.
pub fn run(component_bytes: &[u8], opts: &RunOpts) -> Result<Outcome> {
    run_capturing(component_bytes, opts, None, false, None).map(|(o, _calls)| o)
}

/// Run a RAW CORE wasm MODULE (not a component): instantiate `module_bytes` with NO imports, invoke the
/// exported nullary `export` returning a single `i64`, and return `Outcome::Value(<i64>)` / `Outcome::Trap`.
///
/// This is the `cdz run-emitted` seam — the compiler-ml wasm-emit backend produces a core `(module (func
/// (result i64)) (export "main"))` that imports nothing (an integer module needs no value-heap runtime), so
/// it runs standalone via `wasmtime::Module`, distinct from [`run`]'s component + runtime-compose path. A
/// trap (div0/mod0/`i64::MIN / -1`) surfaces as `Outcome::Trap` — the caller maps it to the differential's
/// `declined`. An invalid module / missing-or-wrong-typed export is an `Err` (a harness/build break).
pub fn run_core_module(module_bytes: &[u8], export: &str) -> Result<Outcome> {
    let engine = engine();
    let module =
        jit_module(&engine, module_bytes).map_err(|e| anyhow!("invalid core module: {e}"))?;
    let mut store: Store<()> = new_store(&engine);
    // No imports: an integer module is self-contained. An unexpected import → a clear error.
    let instance = wasmtime::Instance::new(&mut store, &module, &[])
        .map_err(|e| anyhow!("instantiating core module: {e}"))?;
    let func = instance
        .get_func(&mut store, export)
        .ok_or_else(|| anyhow!("core module exports no function `{export}`"))?;
    // The emit backend's `main` is `() -> i64`. Bind to that typed signature; a shape mismatch is an Err.
    let typed = func
        .typed::<(), i64>(&store)
        .map_err(|e| anyhow!("export `{export}` is not `() -> i64`: {e}"))?;
    match typed.call(&mut store, ()) {
        Ok(v) => Ok(Outcome::Value(v.to_string())),
        Err(e) => Ok(Outcome::Trap(trap_message(&e))),
    }
}

/// Compose the value-heap runtime (if the component imports it) and instantiate `component_bytes`,
/// returning the live `(store, instance)` for a caller that drives a chosen export directly (the
/// bytes-boundary reducer path below). A component with NO runtime import instantiates against a bare
/// linker (the runtime compose is skipped) — so this serves both the reducer provider (imports the heap)
/// and a bare value-form escape program. The runtime bytes come from `opts.runtime` (as `instantiate_
/// runtime` requires); a component that imports the runtime with no `opts.runtime` supplied errors there.
fn compose_and_instantiate(
    component_bytes: &[u8],
    opts: &RunOpts,
) -> Result<(Store<()>, wasmtime::component::Instance)> {
    let engine = engine();
    let component = load_guest(&engine, component_bytes, opts)
        .map_err(|e| anyhow!("invalid component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);
    if let Some(req) = find_runtime_req(&engine, &component) {
        let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
        bind_runtime_into(
            &engine,
            &mut store,
            &mut linker,
            &req.import_name,
            &rt_instance,
            &heap_names,
        )?;
    }
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate component: {e}"))?;
    Ok((store, instance))
}

/// Run `component_bytes`'s export (like [`run_capturing`]) AND, when the component imports the value-heap
/// runtime, read the runtime's live-cell count (`live-objects`) after the run — the heap-balance
/// observable the corpus opt-out grade asserts (a heap case must end at its expected count, default 0 = no
/// leak / no double-free). Returns the run outcome, the ordered observed host-op list (identical to
/// [`run_capturing`], so a heap case that ALSO delegates host effects still has its `(host-calls …)`
/// verified), and the post-run live-object count as an `Option`:
///   - `Some(n)` — the component imports the value-heap runtime (a HEAP case); `n` is the count read from
///     the composed runtime. The caller supplies the runtime bytes in `opts.runtime`, which MUST be the
///     DEBUG-COUNTERS runtime for the count to be meaningful (the shipped runtime's `live-objects` export
///     returns 0 unconditionally).
///   - `None` — the component imports NO runtime (a scalar/const program has no heap to balance). The
///     opt-out grade SKIPS the balance check for such a case (never a false fail). The program still runs
///     and its value/trap outcome is returned.
///
/// Reuses the same host-capturing linker + [`run_export`] drive as [`run_capturing_compiled`] (so every
/// export shape — named, sole, kebab-normalized, resource/closure escape — is handled identically), then
/// additionally reads the runtime instance's counter when a heap import is present.
pub fn run_with_live_objects(
    component_bytes: &[u8],
    opts: &RunOpts,
    second_call: Option<&[String]>,
    drop_handle: bool,
    call_member: Option<&str>,
) -> Result<(Outcome, Vec<String>, Option<u32>)> {
    use std::sync::{Arc, Mutex};
    let engine = engine();
    let component = load_guest(&engine, component_bytes, opts)
        .map_err(|e| anyhow!("invalid component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);

    // Compose the value-heap runtime IF the component imports it, keeping the runtime instance so its
    // `live-objects` counter can be read after the run. A component with no runtime import composes none
    // (no heap to balance) — the balance check is then skipped (`None` returned below).
    let rt_instance = match find_runtime_req(&engine, &component) {
        Some(req) => {
            let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
            bind_runtime_into(
                &engine,
                &mut store,
                &mut linker,
                &req.import_name,
                &rt_instance,
                &heap_names,
            )?;
            Some(rt_instance)
        }
        None => None,
    };

    // Bind every HOST import so a delegated effect's operations are satisfied by the recorded responses,
    // capturing the observed op sequence (inert for a program that makes no host call).
    let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    bind_host_imports(&engine, &component, &mut linker, opts, &observed, &[])?;

    // bytes-second run-wiring: resolve the running export's guest result-Ty, so a WIT-erased leaf renders
    // its value-form via `render_val_typed` (Bytes `b"…"` vs `list<u8>` `#list`, Symbol `#"…"`). The map
    // comes from the wasm `cdz-result-type` section (JIT) OR the self-framed `.cwasm` (AOT/precompiled) via
    // `result_types_of` — the AOT case is what corpus-28 0008/0026 needed (a serialized `.cwasm` drops the
    // section, so the nix corpus-exec rendered type-blind `#list`). Absent/no-match → `None` = type-blind.
    let result_types = result_types_of(component_bytes, opts);
    let result_ty = lookup_result_ty(&result_types, opts.export.as_deref());

    let outcome = run_export(
        &engine,
        &component,
        &mut store,
        &linker,
        opts,
        second_call,
        drop_handle,
        call_member,
        result_ty,
    )?;
    let calls = observed.lock().expect("observed calls mutex").clone();
    // Read the heap balance ONLY on a clean VALUE return: a trapping run aborted mid-computation, so its
    // heap balance is ill-defined AND the runtime instance may be unusable after the guest trap (calling
    // its `live-objects` export could itself error and mask the real trap). A trap case therefore reports
    // no count — the opt-out grade skips the balance check for it (the trap is the outcome).
    let live = match (&outcome, &rt_instance) {
        (Outcome::Value(_), Some(rt)) => Some(read_live_objects(&mut store, rt)?),
        _ => None,
    };
    Ok((outcome, calls, live))
}

/// Read the runtime heap's `live-objects` export (a nullary `-> u32`) — the count of live heap cells. On
/// the debug-counters runtime this is the real balance; on the shipped runtime it is always 0.
fn read_live_objects(
    store: &mut Store<()>,
    rt_instance: &wasmtime::component::Instance,
) -> Result<u32> {
    let heap_idx = rt_instance
        .get_export_index(&mut *store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;
    let lo_idx = rt_instance
        .get_export_index(&mut *store, Some(&heap_idx), "live-objects")
        .ok_or_else(|| {
            anyhow!(
                "runtime heap has no `live-objects` export (build the debug-counters runtime with \
                 `cargo xtask build`)"
            )
        })?;
    let lo_func = rt_instance
        .get_func(&mut *store, lo_idx)
        .ok_or_else(|| anyhow!("`live-objects` is not a func"))?;
    let mut r = [Val::U32(0)];
    lo_func
        .call(&mut *store, &[], &mut r)
        .map_err(|e| anyhow!("calling live-objects: {e}"))?;
    let _ = lo_func.post_return(&mut *store);
    match r[0] {
        Val::U32(n) => Ok(n),
        ref other => Err(anyhow!("live-objects returned a non-u32: {other:?}")),
    }
}

/// A component-model `list<u8>` argument value from raw bytes (each byte a `Val::U8` element).
fn list_u8_val(bytes: &[u8]) -> Val {
    Val::List(bytes.iter().map(|b| Val::U8(*b)).collect())
}

/// Extract the raw bytes of a component-model `list<u8>` result value (errors if it is not a `list<u8>`).
fn val_list_u8(v: &Val) -> Result<Vec<u8>> {
    match v {
        Val::List(items) => items
            .iter()
            .map(|e| match e {
                Val::U8(b) => Ok(*b),
                other => Err(anyhow!("list element is not a u8: {other:?}")),
            })
            .collect(),
        other => Err(anyhow!("value is not a list<u8>: {other:?}")),
    }
}

/// Invoke a REDUCER provider's `list<u8>`-in / `list<u8>`-out member — the §3c full-A bytes boundary. The
/// provider exports interface INSTANCE `iface` (e.g. `cadenza:reducer/api`) with a member `member` (e.g.
/// `apply`) typed `(list<u8>) -> list<u8>`; this composes the value-heap runtime the provider imports,
/// resolves the interface member, calls it with `input` (the canonical value-form Event document), and
/// returns the result bytes (the value-form of the reducer's effect-list). This is the reducer-run entry
/// a host (the agent-harness) uses to drive a compiled reducer over a value-form document.
pub fn run_reducer_bytes(
    provider_bytes: &[u8],
    iface: &str,
    member: &str,
    input: &[u8],
    opts: &RunOpts,
) -> Result<Vec<u8>> {
    let (mut store, instance) = compose_and_instantiate(provider_bytes, opts)?;
    let iface_idx = instance
        .get_export_index(&mut store, None, iface)
        .ok_or_else(|| anyhow!("reducer provider does not export interface `{iface}`"))?;
    let member_idx = instance
        .get_export_index(&mut store, Some(&iface_idx), member)
        .ok_or_else(|| anyhow!("reducer interface `{iface}` has no member `{member}`"))?;
    let func = instance
        .get_func(&mut store, member_idx)
        .ok_or_else(|| anyhow!("reducer member `{member}` is not a func"))?;
    let mut results = [Val::Bool(false)];
    func.call(&mut store, &[list_u8_val(input)], &mut results)
        .map_err(|e| anyhow!("reducer `{member}` call failed: {e:#}"))?;
    func.post_return(&mut store)
        .map_err(|e| anyhow!("reducer `{member}` post_return failed: {e:#}"))?;
    val_list_u8(&results[0])
}

/// Invoke a reducer provider's TYPED interface member — the typed analog of [`run_reducer_bytes`] (which is
/// specialized to `list<u8>`-in/out). The provider exports interface INSTANCE `iface` with a member `member`
/// whose params/result are arbitrary WIT types (e.g. the reducer world's `on-message(message) -> step`, a
/// record in and out); this composes the value-heap runtime the provider imports, resolves the interface
/// member, calls it with `args`, and returns its single result `Val`. Used to drive a compiled TYPED reducer
/// guest over component-model values (a `Val::Record`, …) — the reducer-run entry a host drives a typed
/// reducer through, and the local validation loop for the compiler's typed lift/lower emit.
pub fn run_reducer_typed(
    provider_bytes: &[u8],
    iface: &str,
    member: &str,
    args: &[Val],
    opts: &RunOpts,
) -> Result<Val> {
    let (mut store, instance) = compose_and_instantiate(provider_bytes, opts)?;
    let iface_idx = instance
        .get_export_index(&mut store, None, iface)
        .ok_or_else(|| anyhow!("reducer provider does not export interface `{iface}`"))?;
    let member_idx = instance
        .get_export_index(&mut store, Some(&iface_idx), member)
        .ok_or_else(|| anyhow!("reducer interface `{iface}` has no member `{member}`"))?;
    let func = instance
        .get_func(&mut store, member_idx)
        .ok_or_else(|| anyhow!("reducer member `{member}` is not a func"))?;
    let mut results = [Val::Bool(false)];
    func.call(&mut store, args, &mut results)
        .map_err(|e| anyhow!("reducer `{member}` call failed: {e:#}"))?;
    func.post_return(&mut store)
        .map_err(|e| anyhow!("reducer `{member}` post_return failed: {e:#}"))?;
    Ok(results
        .into_iter()
        .next()
        .expect("a one-result reducer member yields one value"))
}

/// The ordered `(key, value)` byte-pairs a reducer's host `put` op performed during an invoke — the
/// recording sink [`run_reducer_bytes_with_puts`] returns (and threads through its `Arc<Mutex<_>>` sink).
pub type PutLog = Vec<(Vec<u8>, Vec<u8>)>;

/// Invoke a HOST-FUSED reducer's bytes member (§3c GAP B) while BINDING its host `put`-style op to a
/// RECORDING closure — the deeper behavioral proof that the reducer actually EXECUTES its host effect on
/// the real runtime, not merely that its component loads. Composes the value-heap runtime (as
/// [`run_reducer_bytes`]), then binds `host_iface`.`host_op` (two `list<u8>` params, unit result — the kv
/// `put` shape) to a closure that records each (arg0, arg1) byte-pair, invokes `iface`.`member` with
/// `input`, and returns the member's result document PLUS the ordered puts the reducer performed. Generic:
/// the interface/op names come from the caller (the compiled world), never hard-coded — this is the entry
/// a host (the agent-harness) uses to drive a compiled reducer whose effects it satisfies.
pub fn run_reducer_bytes_with_puts(
    provider_bytes: &[u8],
    iface: &str,
    member: &str,
    input: &[u8],
    host_iface: &str,
    host_op: &str,
    opts: &RunOpts,
) -> Result<(Vec<u8>, PutLog)> {
    use std::sync::{Arc, Mutex};
    let engine = engine();
    let component =
        jit_component(&engine, provider_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);
    if let Some(req) = find_runtime_req(&engine, &component) {
        let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
        bind_runtime_into(
            &engine,
            &mut store,
            &mut linker,
            &req.import_name,
            &rt_instance,
            &heap_names,
        )?;
    }
    // The recording sink: each performed put appends its (key, value) byte-pair. `Arc<Mutex<_>>` because the
    // `func_new` closure must be `Send + Sync + 'static` (wasmtime holds it inside the linker/instance).
    let puts: Arc<Mutex<PutLog>> = Arc::new(Mutex::new(Vec::new()));
    let sink = puts.clone();
    let op_label = host_op.to_string();
    {
        let mut iface_linker = linker
            .instance(host_iface)
            .map_err(|e| anyhow!("linker instance {host_iface}: {e}"))?;
        // The host `put` op crosses two `list<u8>` args (lifted to `Val::List(Val::U8 …)`) and returns unit
        // (a zero-result component functype), so the closure reads both args and writes nothing to `results`.
        iface_linker.func_new(host_op, move |_ctx, params, _results| {
            let key = val_list_u8(
                params
                    .first()
                    .ok_or_else(|| anyhow!("{op_label}: missing arg 0"))?,
            )?;
            let value = val_list_u8(
                params
                    .get(1)
                    .ok_or_else(|| anyhow!("{op_label}: missing arg 1"))?,
            )?;
            sink.lock().unwrap().push((key, value));
            Ok(())
        })?;
    }
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate component: {e}"))?;
    let iface_idx = instance
        .get_export_index(&mut store, None, iface)
        .ok_or_else(|| anyhow!("reducer provider does not export interface `{iface}`"))?;
    let member_idx = instance
        .get_export_index(&mut store, Some(&iface_idx), member)
        .ok_or_else(|| anyhow!("reducer interface `{iface}` has no member `{member}`"))?;
    let func = instance
        .get_func(&mut store, member_idx)
        .ok_or_else(|| anyhow!("reducer member `{member}` is not a func"))?;
    let mut results = [Val::Bool(false)];
    func.call(&mut store, &[list_u8_val(input)], &mut results)
        .map_err(|e| anyhow!("reducer `{member}` call failed: {e:#}"))?;
    func.post_return(&mut store)
        .map_err(|e| anyhow!("reducer `{member}` post_return failed: {e:#}"))?;
    let out = val_list_u8(&results[0])?;
    let recorded = puts.lock().unwrap().clone();
    Ok((out, recorded))
}

/// Invoke a host-fused reducer's bytes member while binding its host `prefix-scan`-style op (one `list<u8>`
/// param, `list<tuple<list<u8>,list<u8>>>` result) to a closure returning a FIXED list of key/value byte
/// PAIRS — the deep behavioral proof of the kv.prefix-scan LIST-OF-BYTE-PAIRS LIFT (§3c). The reducer's
/// `apply` decodes the Event, calls `prefix-scan`, and the guest lift reconstructs a value-heap
/// `List<Tuple<Bytes,Bytes>>` from the host's spilled `(ptr, count)` result + each 16-byte element; this
/// drives it on the real runtime so a test can assert the pairs actually round-trip through the lift (e.g.
/// the reducer branching on `List.len` of the scan). Generic — the interface/op names come from the
/// compiled world. Companion to [`run_reducer_bytes_with_get`] (the option-result lift).
#[allow(clippy::too_many_arguments)]
pub fn run_reducer_bytes_with_scan(
    provider_bytes: &[u8],
    iface: &str,
    member: &str,
    input: &[u8],
    host_iface: &str,
    host_op: &str,
    pairs: Vec<(Vec<u8>, Vec<u8>)>,
    opts: &RunOpts,
) -> Result<Vec<u8>> {
    let engine = engine();
    let component =
        jit_component(&engine, provider_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);
    if let Some(req) = find_runtime_req(&engine, &component) {
        let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
        bind_runtime_into(
            &engine,
            &mut store,
            &mut linker,
            &req.import_name,
            &rt_instance,
            &heap_names,
        )?;
    }
    {
        let mut iface_linker = linker
            .instance(host_iface)
            .map_err(|e| anyhow!("linker instance {host_iface}: {e}"))?;
        // `prefix-scan(prefix: list<u8>) -> list<tuple<list<u8>,list<u8>>>`: the closure ignores the prefix
        // and returns the fixed `pairs` as a `Val::List` of 2-element `Val::Tuple`s (each `(list<u8>, list<u8>)`).
        iface_linker.func_new(host_op, move |_ctx, _params, results| {
            let items = pairs
                .iter()
                .map(|(k, v)| Val::Tuple(vec![list_u8_val(k), list_u8_val(v)]))
                .collect();
            results[0] = Val::List(items);
            Ok(())
        })?;
    }
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate component: {e}"))?;
    let iface_idx = instance
        .get_export_index(&mut store, None, iface)
        .ok_or_else(|| anyhow!("reducer provider does not export interface `{iface}`"))?;
    let member_idx = instance
        .get_export_index(&mut store, Some(&iface_idx), member)
        .ok_or_else(|| anyhow!("reducer interface `{iface}` has no member `{member}`"))?;
    let func = instance
        .get_func(&mut store, member_idx)
        .ok_or_else(|| anyhow!("reducer member `{member}` is not a func"))?;
    let mut results = [Val::Bool(false)];
    func.call(&mut store, &[list_u8_val(input)], &mut results)
        .map_err(|e| anyhow!("reducer `{member}` call failed: {e:#}"))?;
    func.post_return(&mut store)
        .map_err(|e| anyhow!("reducer `{member}` post_return failed: {e:#}"))?;
    val_list_u8(&results[0])
}

/// Invoke a host-fused reducer's bytes member while binding its host `get`-style op (one `list<u8>` param,
/// `option<list<u8>>` result) to a closure that returns a FIXED `reply` (`Some(bytes)` or `None`) — the
/// deep behavioral proof of the kv.get OPTION-RESULT LIFT (§3c GAP C). The reducer's `apply` decodes the
/// Event, calls `get`, and the guest lift reconstructs a value-heap `Option<Bytes>` from the host's spilled
/// `(disc, ptr, len)` result; this drives it on the real runtime and returns the effect-list document, so a
/// test can assert the `Some` inner bytes actually round-trip through the lift. Generic — the interface/op
/// names come from the compiled world.
#[allow(clippy::too_many_arguments)]
pub fn run_reducer_bytes_with_get(
    provider_bytes: &[u8],
    iface: &str,
    member: &str,
    input: &[u8],
    host_iface: &str,
    host_op: &str,
    reply: Option<Vec<u8>>,
    opts: &RunOpts,
) -> Result<Vec<u8>> {
    let engine = engine();
    let component =
        jit_component(&engine, provider_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);
    if let Some(req) = find_runtime_req(&engine, &component) {
        let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
        bind_runtime_into(
            &engine,
            &mut store,
            &mut linker,
            &req.import_name,
            &rt_instance,
            &heap_names,
        )?;
    }
    {
        let mut iface_linker = linker
            .instance(host_iface)
            .map_err(|e| anyhow!("linker instance {host_iface}: {e}"))?;
        // `get(key: list<u8>) -> option<list<u8>>`: the closure ignores the key and returns the fixed `reply`
        // (Some(bytes) → `Val::Option(Some(list<u8>))`, None → `Val::Option(None)`).
        iface_linker.func_new(host_op, move |_ctx, _params, results| {
            let v = reply.clone().map(|b| Box::new(list_u8_val(&b)));
            results[0] = Val::Option(v);
            Ok(())
        })?;
    }
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate component: {e}"))?;
    let iface_idx = instance
        .get_export_index(&mut store, None, iface)
        .ok_or_else(|| anyhow!("reducer provider does not export interface `{iface}`"))?;
    let member_idx = instance
        .get_export_index(&mut store, Some(&iface_idx), member)
        .ok_or_else(|| anyhow!("reducer interface `{iface}` has no member `{member}`"))?;
    let func = instance
        .get_func(&mut store, member_idx)
        .ok_or_else(|| anyhow!("reducer member `{member}` is not a func"))?;
    let mut results = [Val::Bool(false)];
    func.call(&mut store, &[list_u8_val(input)], &mut results)
        .map_err(|e| anyhow!("reducer `{member}` call failed: {e:#}"))?;
    func.post_return(&mut store)
        .map_err(|e| anyhow!("reducer `{member}` post_return failed: {e:#}"))?;
    val_list_u8(&results[0])
}

/// CAPTURE a program's escaping compound result as its RAW canonical value-form `list<u8>` document (the
/// bytes, NOT the decoded text [`run`] renders). A program whose top-level export returns a runtime
/// compound (record/tuple/sum/collection) publishes it through the `cadenza:run/run` instance as a
/// resource: `make() -> own<t>` then `encode(handle) -> list<u8>` (the value-encode walker's output). This
/// runs that make+encode dance and returns the raw bytes — the same wire a reducer's value-DECODE
/// reconstructs, so it feeds [`run_reducer_bytes`] as a real input document without hand-encoding
/// (value-encode and value-decode are inverses). `args` coerce to the escaping export's params (empty for
/// a nullary `main`). Mirrors the raw-bytes half of [`run_resource_escape`], before its decode+print.
pub fn capture_escaped_value_doc(
    component_bytes: &[u8],
    args: &[String],
    opts: &RunOpts,
) -> Result<Vec<u8>> {
    let (mut store, instance) = compose_and_instantiate(component_bytes, opts)?;
    let iface = instance
        .get_export_index(&mut store, None, RUN_INTERFACE)
        .ok_or_else(|| {
            anyhow!("capture: component publishes no `{RUN_INTERFACE}` instance (its result does not escape as a value-form document)")
        })?;
    let make_idx = instance
        .get_export_index(&mut store, Some(&iface), "make")
        .ok_or_else(|| anyhow!("capture: `{RUN_INTERFACE}` exports no `make`"))?;
    let encode_idx = instance
        .get_export_index(&mut store, Some(&iface), "encode")
        .ok_or_else(|| anyhow!("capture: `{RUN_INTERFACE}` exports no `encode`"))?;
    let make = instance
        .get_func(&mut store, make_idx)
        .ok_or_else(|| anyhow!("capture: `make` is not a function"))?;
    let encode = instance
        .get_func(&mut store, encode_idx)
        .ok_or_else(|| anyhow!("capture: `encode` is not a function"))?;

    let make_param_types: Vec<Type> = make.params(&store).iter().map(|(_, t)| t.clone()).collect();
    let make_args = coerce_args(args, &make_param_types)?;
    let mut handle = [Val::Bool(false)];
    make.call(&mut store, &make_args, &mut handle)
        .map_err(|e| anyhow!("capture: `make` call failed: {e:#}"))?;
    make.post_return(&mut store)
        .map_err(|e| anyhow!("capture: `make` post_return failed: {e:#}"))?;
    let mut out = [Val::Bool(false)];
    encode
        .call(&mut store, &handle, &mut out)
        .map_err(|e| anyhow!("capture: `encode` call failed: {e:#}"))?;
    encode
        .post_return(&mut store)
        .map_err(|e| anyhow!("capture: `encode` post_return failed: {e:#}"))?;
    val_list_u8(&out[0])
}

/// [`run`], additionally returning the ordered list of HOST OPERATIONS the run performed (each a dotted
/// `E.op`, in call order) — so a caller (the corpus gate) can verify the observed host-call sequence
/// against a case's recorded `(host-calls …)`. Empty for a program that makes no host call.
pub fn run_capturing(
    component_bytes: &[u8],
    opts: &RunOpts,
    second_call: Option<&[String]>,
    drop_handle: bool,
    call_member: Option<&str>,
) -> Result<(Outcome, Vec<String>)> {
    // The one-shot path: JIT-compile the bytes, then run once. A caller that runs the SAME component many
    // times (the `cdz test` per-@test loop) should instead `compile_component` ONCE and call
    // `run_capturing_compiled` per run — `Component::new` is the dominant cost (measured ~8s for the
    // self-host test component vs ~0.1s to run it), so re-JITing identical bytes per test is the multiplier
    // to avoid. This wrapper keeps the existing single-run API (the corpus/oracle callers) byte-identical.
    let compiled = if opts.precompiled {
        // PRECOMPILED (seq-250): `component_bytes` is a `.cwasm` — deserialize it (the cranelift-free path)
        // instead of JIT-compiling via `compile_component`. Downstream `run_capturing_compiled` is unchanged.
        let engine = engine();
        // A serialized `.cwasm` DROPS the component's custom sections, so a bare `.cwasm` can't carry the
        // `cdz-result-type` map that disambiguates a WIT-erased leaf (`list<u8>`→`Bytes` `b"…"` vs `List`
        // `#list(…)`) — which made the AOT corpus-exec render TYPE-BLIND while the JIT path rendered typed
        // (corpus-28 0008/0026: a nested Bytes leaf printed `#list` not `b"…"`). So `--precompile-out`
        // SELF-FRAMES the guest `.cwasm` with its section (see `frame_precompiled`); split it back off here
        // and populate the map, so the AOT render is byte-identical to the JIT path. A RAW `.cwasm` (no
        // magic — the runtime/store precompiles, any legacy artifact) → `(None, whole)` → empty map = the
        // prior type-blind behavior (unchanged).
        CompiledComponent {
            component: load_guest(&engine, component_bytes, opts)?,
            result_types: result_types_of(component_bytes, opts),
        }
    } else {
        compile_component(component_bytes)?
    };
    run_capturing_compiled(&compiled, opts, second_call, drop_handle, call_member)
}

/// A wasmtime-JIT-COMPILED component, ready to run — the reusable half of a run, split from the per-run
/// state (linker/store/host-bindings). `Component::new` (the JIT) is the dominant cost of a run (measured
/// ~8s for the self-host test component, vs ~0.1s to actually run it); compiling ONCE with
/// [`compile_component`] and running many times with [`run_capturing_compiled`] turns an N-`@test` file's
/// N JITs into one. Cheap to hold + pass by reference (`Component` is `Arc`-backed, `Send + Sync`). Compiled
/// against the process-shared [`engine`], so it runs on that same engine (the epoch ticker + `Store`
/// deadlines all refer to it).
pub struct CompiledComponent {
    component: Component,
    /// The GUEST export result-Ty map (bytes-second run-wiring), byte-scanned from the component's
    /// `cdz-result-type` custom section at compile time (rides IN the component, so it reaches every run).
    /// Consulted by [`run_capturing_compiled`] via [`Self::result_ty_for`] so a WIT-erased leaf renders its
    /// value-form (`render::render_val_typed`). Empty when absent → the type-blind render.
    result_types: std::collections::HashMap<String, cadenza_syntax::ast::Arenas>,
}

impl CompiledComponent {
    /// The result-Ty arena (the structured `Ty` payload, decoded from the `cdz-result-type` section) for
    /// the export being run — `[lookup_result_ty]` over the scanned map. `None` (empty map / no match) →
    /// the type-blind render.
    fn result_ty_for(&self, export: Option<&str>) -> Option<&cadenza_syntax::ast::Arenas> {
        lookup_result_ty(&self.result_types, export)
    }
}

/// JIT-compile `component_bytes` into a reusable [`CompiledComponent`] — the expensive step (see the type
/// docs) done ONCE for a component run repeatedly. Equivalent to the compile half of [`run_capturing`].
pub fn compile_component(component_bytes: &[u8]) -> Result<CompiledComponent> {
    let engine = engine();
    let component =
        jit_component(&engine, component_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    // Byte-scan the component's own `cdz-result-type` custom section (bytes-second): the guest export
    // result-Ty map rides IN the component, so it reaches EVERY invocation (this in-process API AND the
    // spawned corpus-gate binary that pipes the raw component). Absent → empty map (type-blind).
    let result_types = parse_result_types(scan_result_type_section(component_bytes).as_deref());
    Ok(CompiledComponent {
        component,
        result_types,
    })
}

/// Run an already-[`compile_component`]d component — [`run_capturing`] minus the per-call JIT. Every call
/// builds a FRESH linker + store + host-binding set (per-run state must not leak across runs), but reuses
/// the one JIT-compiled `Component`. Returns the outcome + the ordered observed host-op list, identical to
/// `run_capturing`.
pub fn run_capturing_compiled(
    compiled: &CompiledComponent,
    opts: &RunOpts,
    second_call: Option<&[String]>,
    drop_handle: bool,
    call_member: Option<&str>,
) -> Result<(Outcome, Vec<String>)> {
    use std::sync::{Arc, Mutex};
    let engine = engine();
    let component = &compiled.component;

    let mut linker: Linker<()> = Linker::new(&engine);

    // If the component records a required runtime, satisfy that import by forwarding every function
    // the runtime's heap interface exports. The linker binds under the component's EXACT import name
    // (the hashed one), while the function set is DISCOVERED from the runtime component's own type —
    // never a hard-coded list — so it can never drift from the runtime the caller supplied.
    let mut store = new_store(&engine);
    if let Some(req) = find_runtime_req(&engine, component) {
        compose_runtime(&engine, &mut store, &mut linker, &req, opts)?;
    }

    // Bind every HOST import (a delegated effect's operations, E2h) so a program's host calls are
    // satisfied by the recorded responses, consumed in call order. Each performed call APPENDS its
    // dotted `E.op` to `observed`, so the caller can compare the observed sequence against the case's
    // recorded `(host-calls …)`. Inert for a program with no host import (the common case).
    let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    bind_host_imports(&engine, component, &mut linker, opts, &observed, &[])?;

    // bytes-second run-wiring (same as `run_with_live_objects`): the guest result-Ty rides on the compiled
    // component (scanned from its `cdz-result-type` section at `compile_component`), so `cdz run` /
    // `run_capturing_compiled` also disambiguates a WIT-erased leaf via `render_val_typed`.
    let result_ty = compiled.result_ty_for(opts.export.as_deref());

    let outcome = run_export(
        &engine,
        component,
        &mut store,
        &linker,
        opts,
        second_call,
        drop_handle,
        call_member,
        result_ty,
    )?;
    let calls = observed.lock().expect("observed calls mutex").clone();
    Ok((outcome, calls))
}

/// One PEER component a consumer binds across the component boundary (X4,
/// `DESIGN-cross-component-interop-rcdzc.md`): its finished bytes and the INTERFACE it exports that the
/// consumer imports under the same name (`cadenza:<pkg>/<iface>`). The peer is a SEPARATELY-compiled
/// artifact — not merged into the consumer — so `run_with_peers` instantiates it and forwards its
/// exported interface funcs into the consumer's like-named import (component-abi.md §Cross-Component
/// Value Exchange; cross-component-interop.md).
#[derive(Debug, Clone)]
pub struct Peer {
    /// The peer component's bytes.
    pub bytes: Vec<u8>,
    /// The interface the peer EXPORTS and the consumer IMPORTS under this exact name.
    pub interface: String,
}

/// Run a CONSUMER component composed with a set of PEER components across the live component boundary.
/// All components share ONE `wasmtime` store (so a value one produces is meaningful to another — the
/// prerequisite for the shared-runtime handle transport X5 adds), and — when the consumer imports the
/// value-heap runtime — ONE runtime instance (component-abi.md §A Cross-Component Handle Is Meaningful
/// Only In The Shared Runtime Instance).
///
/// Each peer is instantiated first; the consumer's import of `peer.interface` is then bound by
/// forwarding every function the peer's exported interface offers (discovered off the peer instance's
/// type, never a hard-coded list — the same discipline `compose_runtime` uses for the runtime). When the
/// consumer OR any peer imports the value-heap runtime, ONE runtime instance is composed and bound into
/// EVERY component that imports it (X5), so a `value` handle one produces is meaningful to another (they
/// index the same heap — component-abi.md §A Cross-Component Handle Is Meaningful Only In The Shared
/// Runtime Instance). SCOPE: scalar peer ops today; a `value`-handle op rides this shared instance.
///
/// This is the host binding every composed component's value-heap runtime import to the ONE shared
/// instance: the consumer and each peer all pin the same runtime (same content hash → same import name),
/// so their handles index one heap and none is handed a handle into a heap it does not share. (The
/// `value`-handle crossing that USES this shared heap is X5b; X5a establishes the shared instance.)
//= spec/contracts/component-abi.md#a-cross-component-handle-is-meaningful-only-in-the-shared-runtime-instance
//# A host that composes Cadenza components which exchange values by handle MUST bind every such component's value-heap runtime import to the one shared runtime instance, so that the components' handles index one heap and a component cannot be handed a handle into a heap it does not share.
pub fn run_with_peers(consumer_bytes: &[u8], peers: &[Peer], opts: &RunOpts) -> Result<Outcome> {
    // The no-host-bindings special case of [`run_with_peers_hosted`] (like `run_agent` is of the hosted
    // form) — a composed run whose only non-Cadenza surface is `opts.host_responses`, no live closures.
    if opts.precompiled {
        // Peer composition (`compile_composition`) JIT-compiles and takes no `RunOpts`, so it cannot yet
        // deserialize `.cwasm` artifacts — see `run_with_peers_live_objects`. Fail clearly.
        anyhow::bail!(
            "precompiled mode does not yet support peer composition (`--peer`); run peer cases with a \
             cranelift-enabled build, or await precompiled peer-composition support"
        );
    }
    run_with_peers_hosted(consumer_bytes, peers, opts, Vec::new())
}

/// Like [`run_with_peers`], but ALSO binds each [`HostOpBinding`]'s live Rust closure into the consumer —
/// composing the peer providers AND answering host ops with real closures in ONE run. This is the runner
/// the agent-kernel needs: its `interpret` stays a separately-compiled, self-modifiable PROVIDER peer,
/// while its `Prim` effect (exec/http/log) is answered by a host closure — neither [`run_with_peers`]
/// (no host bindings) nor [`run_agent_hosted`] (no peers) does both. The bound ops' interfaces are added
/// to the host-effect skip list so a bound op is never ALSO auto-bound from `opts.host_responses`.
pub fn run_with_peers_hosted(
    consumer_bytes: &[u8],
    peers: &[Peer],
    opts: &RunOpts,
    bindings: Vec<HostOpBinding>,
) -> Result<Outcome> {
    let compiled = compile_composition(consumer_bytes, peers)?;
    run_composition_hosted_capturing(&compiled, opts, bindings, None, false, None)
        .map(|(o, _, _)| o)
}

/// Like [`run_with_peers`], but returns the ordered OBSERVED host-op list alongside the outcome — the
/// composed-run analogue of [`run_capturing`]. A caller that composes a consumer against peers AND needs the
/// observed sequence (e.g. `cdz test` over an Option-C shared-closure `component-provider` peer: it counts
/// `Test.gen-int` to tell a property test from a unit test, and reads a failing test's assertion message off
/// the observed list) uses this instead of the `Outcome`-only [`run_with_peers`]. The observed list is the
/// SAME one the peer path already builds internally; this just surfaces it. No host closures (that is the
/// hosted+capturing form, not needed by `cdz test`).
pub fn run_with_peers_capturing(
    consumer_bytes: &[u8],
    peers: &[Peer],
    opts: &RunOpts,
) -> Result<(Outcome, Vec<String>)> {
    let compiled = compile_composition(consumer_bytes, peers)?;
    run_composition_capturing(&compiled, opts)
}

/// A JIT-compiled consumer + its peer components, reusable across many runs — the composed analogue of
/// [`CompiledComponent`]. `Component::new` (the wasmtime JIT) is ~99% of a run's cost, so a caller that runs
/// the SAME composition repeatedly (e.g. `cdz test` running a PROPERTY test's many trials against one
/// shared-closure `component-provider` peer) compiles ONCE via [`compile_composition`] then runs each trial
/// via [`run_composition_capturing`] — instead of re-JITing consumer+peer per trial (the materialize-once
/// fix; without it a multi-trial composed test pays the dominant JIT per-trial, PR#892).
pub struct CompiledComposition {
    consumer: Component,
    /// Each peer's JIT'd component paired with the interface it exports (needed to bind it into the consumer).
    peers: Vec<(Component, String)>,
}

/// JIT-compile a consumer + its peers into a reusable [`CompiledComposition`] — the expensive step done ONCE
/// for a composition run repeatedly. Equivalent to the compile half of [`run_with_peers_capturing`].
pub fn compile_composition(consumer_bytes: &[u8], peers: &[Peer]) -> Result<CompiledComposition> {
    let engine = engine();
    let consumer = jit_component(&engine, consumer_bytes)
        .map_err(|e| anyhow!("invalid consumer component: {e}"))?;
    let peer_components = peers
        .iter()
        .map(|p| {
            jit_component(&engine, &p.bytes)
                .map(|c| (c, p.interface.clone()))
                .map_err(|e| anyhow!("invalid peer component `{}`: {e}", p.interface))
        })
        .collect::<Result<_>>()?;
    Ok(CompiledComposition {
        consumer,
        peers: peer_components,
    })
}

/// A shared-closure PROVIDER peer JIT-compiled ONCE, reusable across MANY compositions — the interface name it
/// exports + its JIT'd [`Component`] (`Component` is `Clone`/`Arc`-backed, so sharing it is a refcount bump).
/// The whole point: `cdz test <dir>` composes N per-file consumers against ONE shared provider, and the
/// provider (the whole import-closure — the ~1360-def self-host closure) is the DOMINANT JIT cost. Compiling
/// it ONCE here and reusing the `Component` across every file's [`compile_composition_with_providers`] means
/// the heavy provider is JIT'd 1×, not N× (the per-file "sits there for a bit" startup stall) — while each
/// file keeps its own thin consumer + its own per-file/per-test PASS/FAIL run (localization preserved).
#[derive(Clone)]
pub struct CompiledProvider {
    /// The JIT'd provider component (reused across compositions). PRIVATE: a `wasmtime::component::Component`
    /// is an internal implementation detail — exposing it publicly would leak the wasmtime dependency into
    /// cdz-run's public API. Callers only ever obtain a `CompiledProvider` from [`compile_provider`] and pass
    /// it back into [`compile_composition_with_providers`] (both opaque), so the fields need not be public.
    component: Component,
    /// The interface it exports (bound into each consumer's like-named import).
    interface: String,
}

/// JIT-compile a provider's bytes into a reusable [`CompiledProvider`] — do this ONCE per shared closure, then
/// hand the result to [`compile_composition_with_providers`] for each consumer that imports it. Splits the
/// expensive provider JIT out of the per-consumer [`compile_composition`] (which re-JITs the provider from
/// bytes every call).
pub fn compile_provider(
    provider_bytes: &[u8],
    interface: impl Into<String>,
) -> Result<CompiledProvider> {
    let engine = engine();
    let component = jit_component(&engine, provider_bytes)
        .map_err(|e| anyhow!("invalid provider component: {e}"))?;
    Ok(CompiledProvider {
        component,
        interface: interface.into(),
    })
}

/// Like [`compile_provider`], but PERSISTS the JIT'd artifact (the wasmtime "cwasm") to `cache_dir`,
/// content-addressed by (`closure_hash` ‖ engine-compat fingerprint), and REUSES it across process/gate
/// invocations. The provider JIT (`Component::new` of the whole import-closure — the ~1360-def self-host
/// closure) is ~270s cold; deserializing a persisted cwasm is a load+relocate (~seconds), so an UNCHANGED
/// shared closure skips the re-JIT entirely on the next gate — the way the value-heap runtime store already
/// content-addresses wasm (pr-sync's ask, operator-flagged gate slowness).
///
/// FLOW: look for `<cache_dir>/<closure_hash>.<engine_fp>.cwasm` → `Component::deserialize` (validated against
/// this engine; a stale/mismatched artifact returns Err → treated as a MISS, never a miscompile). MISS →
/// `Component::new` (JIT) then `serialize` + best-effort persist. The engine fingerprint in the key means a
/// wasmtime/`Config`/target change auto-invalidates (the old key just isn't found). SAFETY: `deserialize` is
/// `unsafe` only in the "trust the bytes came from wasmtime" sense — satisfied because WE wrote them to our own
/// content-addressed cache and `deserialize` re-validates compatibility. Best-effort throughout: any cache I/O
/// failure falls back to a plain JIT (no correctness impact — "no cache").
pub fn compile_provider_cached(
    provider_bytes: &[u8],
    interface: impl Into<String>,
    cache_dir: &std::path::Path,
    closure_hash: &str,
) -> Result<CompiledProvider> {
    let engine = engine();
    let interface = interface.into();
    let cwasm_path = cache_dir.join(format!("{closure_hash}.{}.cwasm", engine_fp(&engine)));

    // HIT: a persisted cwasm for this exact closure + engine — deserialize (fast) instead of re-JIT. Any
    // failure (missing / truncated / engine-incompatible) drops through to the JIT+persist miss path.
    if let Ok(bytes) = std::fs::read(&cwasm_path) {
        // SAFETY: `bytes` were produced by `Component::serialize` below and written to our own
        // content-addressed cache; `deserialize` re-validates them against this engine (version/Config/target)
        // and returns Err on any mismatch, which we treat as a miss.
        if let Ok(component) = unsafe { Component::deserialize(&engine, &bytes) } {
            return Ok(CompiledProvider {
                component,
                interface,
            });
        }
    }

    // MISS: JIT the provider, then best-effort persist its serialized cwasm (atomic temp+rename) so the next
    // gate hits. A persist failure is silently ignored — the run still has its JIT'd Component.
    let component = jit_component(&engine, provider_bytes)
        .map_err(|e| anyhow!("invalid provider component: {e}"))?;
    if let Ok(serialized) = component.serialize() {
        let _ = std::fs::create_dir_all(cache_dir);
        let tmp = cache_dir.join(format!(
            ".{closure_hash}.{}.cwasm.{}.tmp",
            engine_fp(&engine),
            std::process::id()
        ));
        if std::fs::write(&tmp, &serialized).is_ok() && std::fs::rename(&tmp, &cwasm_path).is_err()
        {
            // Windows/rename-over-existing or a race — best-effort remove-dest + retry, else drop the temp.
            let _ = std::fs::remove_file(&cwasm_path);
            if std::fs::rename(&tmp, &cwasm_path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }
    Ok(CompiledProvider {
        component,
        interface,
    })
}

/// Like [`compile_composition`], but the peers are ALREADY-JIT'd [`CompiledProvider`]s (shared across
/// compositions) — so only the (thin) consumer is JIT'd here. This is the per-project-JIT-once path: JIT each
/// shared provider ONCE ([`compile_provider`]), then compose every file's consumer against the shared
/// provider Component(s) without re-JITing the heavy closure. Behavior-identical to `compile_composition` +
/// `run_composition_*` — same instantiation, same shared-runtime binding — only the provider JIT is hoisted
/// out of the per-file loop.
pub fn compile_composition_with_providers(
    consumer_bytes: &[u8],
    providers: &[CompiledProvider],
) -> Result<CompiledComposition> {
    let engine = engine();
    let consumer = jit_component(&engine, consumer_bytes)
        .map_err(|e| anyhow!("invalid consumer component: {e}"))?;
    let peers = providers
        .iter()
        .map(|p| (p.component.clone(), p.interface.clone()))
        .collect();
    Ok(CompiledComposition { consumer, peers })
}

/// Run a pre-compiled [`CompiledComposition`] once, returning `(Outcome, observed-host-op-list)`. Builds a
/// FRESH store + linker + shared-runtime instance per call (per-run state), reusing the JIT'd consumer/peer
/// Components — so N trials cost N cheap runs + ONE JIT, not N JITs. No host closures (that is the hosted
/// form, not needed by `cdz test`).
pub fn run_composition_capturing(
    compiled: &CompiledComposition,
    opts: &RunOpts,
) -> Result<(Outcome, Vec<String>)> {
    run_composition_hosted_capturing(compiled, opts, Vec::new(), None, false, None)
        .map(|(o, c, _)| (o, c))
}

/// The capturing core, over PRE-COMPILED components — returns `(Outcome, observed-host-op-list)`. The
/// bytes-taking public forms ([`run_with_peers`]/[`run_with_peers_hosted`]/[`run_with_peers_capturing`])
/// compile-then-run via this; a caller that reuses a composition across trials calls [`compile_composition`]
/// once + [`run_composition_capturing`] per trial. The observed list is built + populated by
/// `bind_host_imports` exactly as [`run_capturing`] does, so a composed run's observed sequence is identical
/// in shape to a standalone `run_capturing` one.
fn run_composition_hosted_capturing(
    compiled: &CompiledComposition,
    opts: &RunOpts,
    bindings: Vec<HostOpBinding>,
    // The trial's call shape (a `(then …)` two-call continuation, a `(drop)`, a `(call-method …)`) — the
    // SAME knobs the single-component [`run_with_live_objects`] threads, so a PEER case grades identically
    // to a plain one on the nix grade path. `(None, false, None)` for a plain composed run.
    second_call: Option<&[String]>,
    drop_handle: bool,
    call_member: Option<&str>,
) -> Result<(Outcome, Vec<String>, Option<u32>)> {
    use std::sync::{Arc, Mutex};
    let engine = engine();
    let consumer = &compiled.consumer;
    let peer_components: Vec<&Component> = compiled.peers.iter().map(|(c, _)| c).collect();
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);

    // Shape-check the host bindings up-front (clear error, not an opaque trap in the closure).
    check_host_op_binding_shapes(&engine, consumer, &bindings)?;

    // The runtime import each component may declare — the consumer and each peer. They all pin the SAME
    // runtime (same content hash → same import name), so ONE runtime instance serves them all. Instantiate
    // it once here (if anyone needs it), then bind it into every importing component's linker below. (Peer
    // components are already JIT'd in `compiled.peers` — no re-compile here.)
    let consumer_req = find_runtime_req(&engine, consumer);
    let any_req = consumer_req.clone().or_else(|| {
        peer_components
            .iter()
            .find_map(|c| find_runtime_req(&engine, c))
    });
    let shared_runtime = match &any_req {
        Some(req) => Some(instantiate_runtime(&engine, &mut store, req, opts)?),
        None => None,
    };

    // Bind the shared runtime into the CONSUMER's import (if it declares one).
    if let (Some(req), Some((rt_instance, names))) = (&consumer_req, &shared_runtime) {
        bind_runtime_into(
            &engine,
            &mut store,
            &mut linker,
            &req.import_name,
            rt_instance,
            names,
        )?;
    }

    // Instantiate each peer and forward its exported interface funcs into the consumer's like-named import.
    // A peer that imports the runtime gets the SAME shared instance bound into its linker (so its handles
    // index the one shared heap); its funcs live in the SHARED store. A peer may ALSO import ANOTHER peer's
    // interface (an A→B→C chain, where B binds A and publishes its own for C, U11): peers are given in
    // DEPENDENCY order, so each peer's linker is pre-bound with the interfaces of every EARLIER-instantiated
    // peer. The extracted interface funcs (`(iface, [(fname, Func)])`) are collected as we go, so a later
    // peer's linker and finally the consumer's linker bind against them.
    let mut peer_ifaces: Vec<(String, Vec<(String, wasmtime::component::Func)>)> = Vec::new();
    for (peer_component, peer_iface) in compiled.peers.iter().map(|(c, i)| (c, i)) {
        let mut peer_linker: Linker<()> = Linker::new(&engine);
        if let (Some(req), Some((rt_instance, names))) =
            (find_runtime_req(&engine, peer_component), &shared_runtime)
        {
            bind_runtime_into(
                &engine,
                &mut store,
                &mut peer_linker,
                &req.import_name,
                rt_instance,
                names,
            )?;
        }
        // Bind every EARLIER peer's interface into this peer's linker (dependency order): a peer that
        // imports `cadenza:pairs/api` sees it because the peer providing it was given first.
        bind_peer_ifaces_into(&mut peer_linker, &peer_ifaces)?;
        let peer_instance = peer_linker
            .instantiate(&mut store, peer_component)
            .map_err(|e| anyhow!("instantiate peer `{}`: {e}", peer_iface))?;
        let iface_idx = peer_instance
            .get_export_index(&mut store, None, peer_iface)
            .ok_or_else(|| anyhow!("peer does not export the interface `{}`", peer_iface))?;
        // The interface's function names, read off the peer instance's exported interface type.
        let func_names: Vec<String> = peer_component
            .component_type()
            .exports(&engine)
            .find(|(n, _)| *n == peer_iface)
            .and_then(|(_, item)| match item {
                ComponentItem::ComponentInstance(inst) => Some(
                    inst.exports(&engine)
                        .filter_map(|(fname, i)| {
                            matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                        })
                        .collect(),
                ),
                _ => None,
            })
            .ok_or_else(|| anyhow!("peer export `{}` is not an interface instance", peer_iface))?;
        let mut funcs = Vec::new();
        for fname in &func_names {
            let fidx = peer_instance
                .get_export_index(&mut store, Some(&iface_idx), fname)
                .ok_or_else(|| anyhow!("peer `{}` missing `{fname}`", peer_iface))?;
            let f = peer_instance
                .get_func(&mut store, fidx)
                .ok_or_else(|| anyhow!("peer export `{fname}` is not a func"))?;
            funcs.push((fname.clone(), f));
        }
        peer_ifaces.push((peer_iface.clone(), funcs));
    }

    // COMPOSE-TIME SIGNATURE CHECK: a peer's exported interface func must MATCH the arity the consumer's
    // like-named import declares. `bind_peer_ifaces_into` wires each peer func via a raw dynamic closure
    // (`func_new`) that forwards the caller's args verbatim, so a mismatch (consumer expects `add :
    // (s64,s64)->s64`, peer exports `add : (s64)->s64`) is NOT caught by the linker and surfaces only as an
    // OPAQUE runtime TRAP deep in the callee. Catch it here with a diagnostic naming the interface, op, and
    // the two arities — the [[rcdzc-kebab-extern-name-gotcha]]-adjacent "a boundary shape mismatches with no
    // clear error" class, at the composition edge rather than the emit edge.
    check_peer_iface_signatures(&engine, consumer, &compiled.peers)?;

    // Bind every peer's exported interface into the CONSUMER's linker (the top of the chain imports them).
    bind_peer_ifaces_into(&mut linker, &peer_ifaces)?;

    // Bind the consumer's HOST-effect imports (if any), skipping the peer interfaces already bound above
    // AND the interfaces we bind explicitly below (a live closure), so neither is double-bound as an
    // auto `opts.host_responses` effect (a double-bind is a linker error).
    let mut skip: Vec<String> = compiled.peers.iter().map(|(_, i)| i.clone()).collect();
    skip.extend(bindings.iter().map(|b| b.iface.clone()));
    let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    bind_host_imports(&engine, consumer, &mut linker, opts, &observed, &skip)?;

    // Bind each explicit host-op closure into the consumer, marshalling through the shared runtime's ropes.
    // Requires the shared runtime to exist (String host-op args/results cross as rope handles into it).
    if !bindings.is_empty() {
        let (rt_instance, _) = shared_runtime.as_ref().ok_or_else(|| {
            anyhow!(
                "run_with_peers_hosted requires the value-heap runtime (host-op String args/results cross \
                 as rope handles), but neither the consumer nor any peer imports it"
            )
        })?;
        bind_host_op_bindings(&mut store, &mut linker, rt_instance, bindings)?;
    }

    let outcome = run_export(
        &engine,
        consumer,
        &mut store,
        &linker,
        opts,
        second_call,
        drop_handle,
        call_member,
        // Composition (multi-component) result-Ty threading is a follow-up — the single-component gate
        // paths (run_with_live_objects / run_capturing) carry it; a composed run stays type-blind for now.
        None,
    )?;
    // Take (not clone) the observed list — nothing reads `observed` after this, so move it out (avoids an
    // O(n) copy of the op list). The mutex guard is dropped immediately.
    let calls = std::mem::take(&mut *observed.lock().expect("observed calls mutex"));
    // Heap balance over the SHARED runtime instance — the composed analogue of [`run_with_live_objects`]:
    // a heap-importing peer case must end at its expected live-cell count on the DEBUG-COUNTERS runtime the
    // grade path composes (`--runtime runtimeDebug`). Read ONLY on a clean VALUE return (a trap aborted
    // mid-computation, so the balance is ill-defined AND the instance may be unusable — mirrors
    // `run_with_live_objects`). `None` when no component imported the runtime (a scalar peer case) → the
    // grade skips the balance check.
    let live = match (&outcome, &shared_runtime) {
        (Outcome::Value(_), Some((rt, _))) => Some(read_live_objects(&mut store, rt)?),
        _ => None,
    };
    Ok((outcome, calls, live))
}

/// The PEER-COMPOSING analogue of [`run_with_live_objects`]: compose `consumer_bytes` against its `peers`
/// (the shared-runtime instance + compose-time signature check `run_with_peers` establishes), run the
/// chosen export with the trial's call shape, and read the shared runtime's live-cell count. This is what
/// the corpus GRADE path uses for a `(peer …)` case — the plain [`run_with_live_objects`] runs the consumer
/// ALONE (no peers), so a peer case's imported interface would fall through to an unbound host-call. Returns
/// the outcome, the observed host-op list, and the post-run live count (`None` for a no-heap case).
pub fn run_with_peers_live_objects(
    consumer_bytes: &[u8],
    peers: &[Peer],
    opts: &RunOpts,
    second_call: Option<&[String]>,
    drop_handle: bool,
    call_member: Option<&str>,
) -> Result<(Outcome, Vec<String>, Option<u32>)> {
    // PRECOMPILED (seq-250): the peer-composition path (`compile_composition`) JIT-compiles the consumer +
    // each peer and takes no `RunOpts`, so it cannot yet load precompiled `.cwasm` artifacts. A cranelift-
    // free exec therefore cannot run a `(peer …)` corpus case yet — fail with a clear signal rather than the
    // opaque "cannot JIT-compile" from `jit_component`. Threading `precompiled` through `compile_composition`
    // (+ its providers variant) is the follow-up that lifts this.
    if opts.precompiled {
        anyhow::bail!(
            "precompiled mode does not yet support peer composition (`--peer`); the consumer+peers are \
             still JIT-compiled — run peer cases with a cranelift-enabled build, or await precompiled \
             peer-composition support"
        );
    }
    let compiled = compile_composition(consumer_bytes, peers)?;
    run_composition_hosted_capturing(
        &compiled,
        opts,
        Vec::new(),
        second_call,
        drop_handle,
        call_member,
    )
}

/// Verify a consumer's imported model op `model_iface`.`op_name` has the `(u32) -> u32` boundary shape
/// [`run_agent`] binds (a String prompt/completion each cross as ONE runtime rope HANDLE). Checked BEFORE
/// binding so a mis-shaped op fails with a clear message naming the op + the required shape, rather than
/// panicking or trapping opaquely inside the host closure. If the consumer does not import the interface
/// or the op at all, this returns Ok — the linker's own "unknown import" error is already clear (and a
/// consumer that never performs the op needs no binding). Only a PRESENT-but-mis-shaped op is rejected.
fn check_model_op_shape(
    engine: &Engine,
    consumer: &Component,
    model_iface: &str,
    op_name: &str,
) -> Result<()> {
    // The consumer's declared signatures for `model_iface` (None → it doesn't import the interface).
    let Some(sigs) = consumer
        .component_type()
        .imports(engine)
        .find(|(n, _)| *n == model_iface)
        .and_then(|(_, item)| iface_func_sigs(engine, &item))
    else {
        return Ok(());
    };
    // The op within it (None → the interface is imported but not this op → leave to the linker).
    let Some((_, params, results)) = sigs.iter().find(|(n, _, _)| n == op_name) else {
        return Ok(());
    };
    let is_u32 = |t: &Type| matches!(t, Type::U32);
    let ok = params.len() == 1 && is_u32(&params[0]) && results.len() == 1 && is_u32(&results[0]);
    if !ok {
        let shape = |ts: &[Type]| {
            ts.iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(anyhow!(
            "the model op `{model_iface}`.`{op_name}` must be `(u32) -> u32` (a String prompt and \
             completion each cross as one runtime rope handle), but the consumer imports it as \
             `({}) -> ({})` — a `String -> String` model op lowers to exactly one u32 arg and one u32 \
             result",
            shape(params),
            shape(results),
        ));
    }
    Ok(())
}

/// Run a CONSUMER component whose one peer interface `model_iface` (e.g. `cadenza:model/api`) is answered
/// not by a peer COMPONENT but by a HOST closure — the embedder pattern the native agent-harness uses to
/// wire a `String -> String` model call (Bedrock) into a pure-Cadenza agent loop without a Cadenza peer
/// (which would need TLS/SigV4 Cadenza lacks) and without the host-boundary String-result ABI (unbuilt).
///
/// `model_iface` must export exactly one op `op_name` of type `(String) -> String`, which crosses the
/// component boundary as `converse(u32) -> u32` — the arg/result are opaque handles into the SHARED
/// value-heap runtime (component-abi.md §A Cross-Component Handle Is Meaningful Only In The Shared Runtime
/// Instance). This runner instantiates that ONE shared runtime, captures its `str-get`/`str-new` funcs,
/// and binds the consumer's import of `model_iface`.`op_name` to a closure that: reads the prompt handle
/// to a `String` (`str-get`), calls the caller's `converse`, and returns the completion as a fresh handle
/// (`str-new`). The agent LOOP stays pure Cadenza; the only non-Cadenza surface is `converse` itself
/// (the crate that supplies a Bedrock-backed `converse` keeps the aws-sdk out of this runner's deps).
///
/// SCOPE: the model op is monomorphic `(String) -> String` (the model-call shape); a richer host-backed
/// interface is a later widening. The consumer must import the value-heap runtime (a String is a rope).
pub fn run_agent<F>(
    consumer_bytes: &[u8],
    model_iface: &str,
    op_name: &str,
    opts: &RunOpts,
    converse: F,
) -> Result<Outcome>
where
    F: Fn(String) -> String + Send + Sync + 'static,
{
    use std::sync::Arc;
    let engine = engine();
    let consumer = jit_component(&engine, consumer_bytes)
        .map_err(|e| anyhow!("invalid consumer component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);

    // Verify the consumer's imported model op is the `(u32) -> u32` shape the binding assumes (a String
    // prompt/completion each cross as ONE runtime rope HANDLE) BEFORE binding — a differently-shaped op
    // (wrong arity, a non-u32 param/result, or no result) would otherwise surface as an opaque panic or a
    // confusing trap deep in the closure (the unchecked `results[0]` write). Fail up front with a message
    // naming the op and the shape we require. `None` (the op/interface is not imported at all) is left to
    // the linker's own "unknown import" error, which is already clear.
    check_model_op_shape(&engine, &consumer, model_iface, op_name)?;

    // The consumer's String model call rides the shared value-heap runtime (the prompt/completion are
    // rope handles), so the runtime is REQUIRED here (unlike the scalar peer path where it is optional).
    let req = find_runtime_req(&engine, &consumer).ok_or_else(|| {
        anyhow!(
            "run_agent requires the consumer to import the value-heap runtime (a String model call \
             crosses as a rope handle), but it declares no runtime import"
        )
    })?;
    let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
    bind_runtime_into(
        &engine,
        &mut store,
        &mut linker,
        &req.import_name,
        &rt_instance,
        &heap_names,
    )?;

    // Capture the runtime's `str-get`/`str-new` funcs (the rope<->host-String bridge, runtime.wit #18/#17)
    // so the `converse` closure can read the prompt handle and mint the completion handle.
    let heap_idx = rt_instance
        .get_export_index(&mut store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;
    let get_func_named =
        |store: &mut Store<()>, fname: &str| -> Result<wasmtime::component::Func> {
            let fidx = rt_instance
                .get_export_index(&mut *store, Some(&heap_idx), fname)
                .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
            rt_instance
                .get_func(&mut *store, fidx)
                .ok_or_else(|| anyhow!("runtime export `{fname}` is not a func"))
        };
    let str_get = get_func_named(&mut store, "str-get")?;
    let str_new = get_func_named(&mut store, "str-new")?;
    let converse = Arc::new(converse);

    // Bind the consumer's import `model_iface`.`op_name` to the host closure. The op crosses as
    // `converse(u32) -> u32`: read the prompt handle to a String, call the user's `converse`, mint the
    // completion handle. Calling a captured runtime `Func` inside a `func_new` closure via the passed
    // `ctx` is the same pattern `bind_runtime_into`/`bind_peer_ifaces_into` use.
    let mut iface = linker
        .instance(model_iface)
        .map_err(|e| anyhow!("linker instance {model_iface}: {e}"))?;
    let converse_cl = Arc::clone(&converse);
    let op_label = op_name.to_string();
    iface.func_new(op_name, move |mut ctx, params, results| {
        let prompt_handle = match params.first() {
            Some(Val::U32(h)) => *h,
            other => {
                return Err(anyhow!(
                    "model op `{op_label}` expected a u32 prompt handle, got {other:?}"
                ));
            }
        };
        // str-get(handle) -> string: read the prompt rope out of the shared heap.
        let mut got = [Val::Bool(false)];
        str_get.call(&mut ctx, &[Val::U32(prompt_handle)], &mut got)?;
        str_get.post_return(&mut ctx)?;
        let prompt = match &got[0] {
            Val::String(s) => s.to_string(),
            other => return Err(anyhow!("str-get returned a non-string: {other:?}")),
        };
        // The one non-Cadenza edge: the caller's model call (a Bedrock invoke lives behind this).
        let completion = converse_cl(prompt);
        // str-new(string) -> handle: mint the completion rope; return its handle to the guest.
        let mut made = [Val::Bool(false)];
        str_new.call(&mut ctx, &[Val::String(completion)], &mut made)?;
        str_new.post_return(&mut ctx)?;
        // Write the completion handle into the result slot. `check_model_op_shape` already proved the op
        // has exactly one (u32) result, so `results` is non-empty here; guard anyway rather than index
        // blindly (a defense-in-depth that turns any future shape drift into an error, not a panic).
        let slot = results
            .first_mut()
            .ok_or_else(|| anyhow!("model op `{op_label}` returned no result slot to write"))?;
        *slot = made[0].clone();
        Ok(())
    })?;

    run_export(
        &engine, &consumer, &mut store, &linker, opts, None, false, None, None,
    )
}

/// Like [`run_agent`], but ALSO binds an AUTHORIZATION op — the full agent-harness shape where the
/// Cadenza loop performs `Cedar.authorize(action) -> Int64` (1 allow / 0 deny) before every tool
/// dispatch and dispatches only on allow (the "no ambient authority" property). The model op is
/// `(String) -> String` (`converse`, u32→u32 handles); the authz op is `(String) -> Int64`
/// (`authorize`, a u32 action-rope handle → an s64 decision — a String ARG like converse, but a SCALAR
/// result, so it reads the action rope with `str-get` and writes `Val::S64` directly, no `str-new`).
/// Both are answered by HOST CLOSURES over the ONE shared runtime — `converse` by a Bedrock-backed
/// closure, `authorize` by a `cedar-policy`-backed one — keeping the agent loop pure Cadenza.
#[allow(clippy::too_many_arguments)]
pub fn run_agent_authorized<F, A>(
    consumer_bytes: &[u8],
    model_iface: &str,
    model_op: &str,
    authz_iface: &str,
    authz_op: &str,
    opts: &RunOpts,
    converse: F,
    authorize: A,
) -> Result<Outcome>
where
    F: Fn(String) -> String + Send + Sync + 'static,
    A: Fn(String) -> i64 + Send + Sync + 'static,
{
    use std::sync::Arc;
    let engine = engine();
    let consumer = jit_component(&engine, consumer_bytes)
        .map_err(|e| anyhow!("invalid consumer component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);

    // Shape-check both ops up front (clear message on a mis-shaped op, not an opaque trap in the closure).
    check_model_op_shape(&engine, &consumer, model_iface, model_op)?;
    check_authz_op_shape(&engine, &consumer, authz_iface, authz_op)?;

    // One shared value-heap runtime serves the consumer + both host-op closures (the action/prompt cross
    // as rope handles into it).
    let req = find_runtime_req(&engine, &consumer).ok_or_else(|| {
        anyhow!(
            "run_agent_authorized requires the consumer to import the value-heap runtime (String \
             prompt/action cross as rope handles), but it declares no runtime import"
        )
    })?;
    let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
    bind_runtime_into(
        &engine,
        &mut store,
        &mut linker,
        &req.import_name,
        &rt_instance,
        &heap_names,
    )?;

    let heap_idx = rt_instance
        .get_export_index(&mut store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;
    let get_func_named =
        |store: &mut Store<()>, fname: &str| -> Result<wasmtime::component::Func> {
            let fidx = rt_instance
                .get_export_index(&mut *store, Some(&heap_idx), fname)
                .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
            rt_instance
                .get_func(&mut *store, fidx)
                .ok_or_else(|| anyhow!("runtime export `{fname}` is not a func"))
        };
    let str_get = get_func_named(&mut store, "str-get")?;
    let str_new = get_func_named(&mut store, "str-new")?;

    // Bind the model op `(u32) -> u32`: read the prompt rope, call converse, mint the completion rope.
    {
        let converse = Arc::new(converse);
        let mut iface = linker
            .instance(model_iface)
            .map_err(|e| anyhow!("linker instance {model_iface}: {e}"))?;
        let converse_cl = Arc::clone(&converse);
        let op_label = model_op.to_string();
        let (sg, sn) = (str_get, str_new);
        iface.func_new(model_op, move |mut ctx, params, results| {
            let h = match params.first() {
                Some(Val::U32(h)) => *h,
                other => {
                    return Err(anyhow!(
                        "model op `{op_label}` expected a u32 prompt handle, got {other:?}"
                    ));
                }
            };
            let mut got = [Val::Bool(false)];
            sg.call(&mut ctx, &[Val::U32(h)], &mut got)?;
            sg.post_return(&mut ctx)?;
            let prompt = match &got[0] {
                Val::String(s) => s.to_string(),
                other => return Err(anyhow!("str-get returned a non-string: {other:?}")),
            };
            let completion = converse_cl(prompt);
            let mut made = [Val::Bool(false)];
            sn.call(&mut ctx, &[Val::String(completion)], &mut made)?;
            sn.post_return(&mut ctx)?;
            let slot = results
                .first_mut()
                .ok_or_else(|| anyhow!("model op `{op_label}` returned no result slot"))?;
            *slot = made[0].clone();
            Ok(())
        })?;
    }

    // Bind the authz op `(u32) -> s64`: read the action rope, call authorize, write the s64 decision
    // (a SCALAR result — no str-new). Deny (the safe default) if the runner ever sees a mis-typed slot.
    {
        let authorize = Arc::new(authorize);
        let mut iface = linker
            .instance(authz_iface)
            .map_err(|e| anyhow!("linker instance {authz_iface}: {e}"))?;
        let authorize_cl = Arc::clone(&authorize);
        let op_label = authz_op.to_string();
        let sg = str_get;
        iface.func_new(authz_op, move |mut ctx, params, results| {
            let h = match params.first() {
                Some(Val::U32(h)) => *h,
                other => {
                    return Err(anyhow!(
                        "authz op `{op_label}` expected a u32 action handle, got {other:?}"
                    ));
                }
            };
            let mut got = [Val::Bool(false)];
            sg.call(&mut ctx, &[Val::U32(h)], &mut got)?;
            sg.post_return(&mut ctx)?;
            let action = match &got[0] {
                Val::String(s) => s.to_string(),
                other => return Err(anyhow!("str-get returned a non-string: {other:?}")),
            };
            let decision = authorize_cl(action);
            let slot = results
                .first_mut()
                .ok_or_else(|| anyhow!("authz op `{op_label}` returned no result slot"))?;
            *slot = Val::S64(decision);
            Ok(())
        })?;
    }

    run_export(
        &engine, &consumer, &mut store, &linker, opts, None, false, None, None,
    )
}

/// Verify a consumer's imported authz op `authz_iface`.`authz_op` has the `(u32) -> s64` boundary shape
/// [`run_agent_authorized`] binds (a String action crosses as a rope HANDLE `u32`; the decision is a
/// scalar `Int64` = `s64`). Same up-front-clear-error discipline as [`check_model_op_shape`]; Ok if the
/// op/interface isn't imported (the linker reports an unknown import clearly).
fn check_authz_op_shape(
    engine: &Engine,
    consumer: &Component,
    authz_iface: &str,
    authz_op: &str,
) -> Result<()> {
    let Some(sigs) = consumer
        .component_type()
        .imports(engine)
        .find(|(n, _)| *n == authz_iface)
        .and_then(|(_, item)| iface_func_sigs(engine, &item))
    else {
        return Ok(());
    };
    let Some((_, params, results)) = sigs.iter().find(|(n, _, _)| n == authz_op) else {
        return Ok(());
    };
    let ok = params.len() == 1
        && matches!(params[0], Type::U32)
        && results.len() == 1
        && matches!(results[0], Type::S64);
    if !ok {
        let shape = |ts: &[Type]| {
            ts.iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(anyhow!(
            "the authz op `{authz_iface}`.`{authz_op}` must be `(u32) -> s64` (a String action crosses \
             as a rope handle; the decision is an Int64), but the consumer imports it as `({}) -> ({})` \
             — a `String -> Int64` authz op lowers to one u32 arg and one s64 result",
            shape(params),
            shape(results),
        ));
    }
    Ok(())
}

/// One HOST OP an agent loop performs, answered by a Rust closure over the shared value-heap runtime.
/// The variant fixes the boundary shape — how the closure's String/scalar values marshal to/from the
/// runtime rope handles (`str-get`/`str-new`) — so the runner binds each op correctly. This is the
/// general form the fixed [`run_agent`]/[`run_agent_authorized`] runners are special cases of.
pub enum HostOp {
    /// `(String) -> String` (crosses `(u32) -> u32`): read the arg rope, call the closure, mint the
    /// result rope. E.g. the model call `converse(prompt) -> completion`.
    StringToString(Box<dyn Fn(String) -> String + Send + Sync>),
    /// `(String) -> Int64` (crosses `(u32) -> s64`): read the arg rope, call the closure, write the
    /// scalar. E.g. `authorize(action) -> decision`.
    StringToScalar(Box<dyn Fn(String) -> i64 + Send + Sync>),
    /// `() -> String` (crosses `() -> u32`): call the closure, mint the result rope. E.g. `next() ->
    /// message` (read the next inbox message).
    UnitToString(Box<dyn Fn() -> String + Send + Sync>),
}

/// A host op binding: the interface + op the consumer imports, and the closure (with its shape) that
/// answers it. Passed to [`run_agent_hosted`].
pub struct HostOpBinding {
    pub iface: String,
    pub op: String,
    pub host: HostOp,
}

/// Run a CONSUMER agent-loop component, answering EACH of its imported host ops with a Rust closure —
/// the general embedder runner. Every op's String args/results cross as runtime rope handles into the
/// ONE shared value-heap runtime (the runner captures its `str-get`/`str-new` and marshals per each
/// [`HostOp`] shape). The agent loop stays pure Cadenza; the closures are the only non-Cadenza surface
/// (a Bedrock model call, a Cedar decision, an inbox read). [`run_agent`]/[`run_agent_authorized`] are
/// fixed-shape special cases; this is the form that also binds e.g. `next() -> message` so the loop can
/// read its input.
pub fn run_agent_hosted(
    consumer_bytes: &[u8],
    opts: &RunOpts,
    bindings: Vec<HostOpBinding>,
) -> Result<Outcome> {
    let engine = engine();
    let consumer = jit_component(&engine, consumer_bytes)
        .map_err(|e| anyhow!("invalid consumer component: {e}"))?;
    let mut store = new_store(&engine);
    let mut linker: Linker<()> = Linker::new(&engine);

    check_host_op_binding_shapes(&engine, &consumer, &bindings)?;

    // One shared value-heap runtime serves the consumer + every closure (String args/results are ropes).
    let req = find_runtime_req(&engine, &consumer).ok_or_else(|| {
        anyhow!(
            "run_agent_hosted requires the consumer to import the value-heap runtime (String host-op \
             args/results cross as rope handles), but it declares no runtime import"
        )
    })?;
    let (rt_instance, heap_names) = instantiate_runtime(&engine, &mut store, &req, opts)?;
    bind_runtime_into(
        &engine,
        &mut store,
        &mut linker,
        &req.import_name,
        &rt_instance,
        &heap_names,
    )?;

    bind_host_op_bindings(&mut store, &mut linker, &rt_instance, bindings)?;

    run_export(
        &engine, &consumer, &mut store, &linker, opts, None, false, None, None,
    )
}

/// Shape-check each [`HostOpBinding`] against the consumer's declared import (a clear up-front error,
/// not an opaque trap deep in the closure). A binding the consumer doesn't import is skipped (the linker
/// reports that). Shared by [`run_agent_hosted`] and [`run_with_peers_hosted`].
fn check_host_op_binding_shapes(
    engine: &Engine,
    consumer: &Component,
    bindings: &[HostOpBinding],
) -> Result<()> {
    for b in bindings {
        let want = match &b.host {
            HostOp::StringToString(_) => (&[Type::U32][..], &[Type::U32][..]),
            HostOp::StringToScalar(_) => (&[Type::U32][..], &[Type::S64][..]),
            HostOp::UnitToString(_) => (&[][..], &[Type::U32][..]),
        };
        check_host_op_shape(engine, consumer, &b.iface, &b.op, want.0, want.1)?;
    }
    Ok(())
}

/// Bind each [`HostOpBinding`] into `linker` as a `func_new` closure of the right shape, marshalling
/// String args/results through the shared runtime's `str-get`/`str-new` ropes. The `rt_instance` must
/// already be bound into `linker` (String host-op args/results cross as rope handles into it). Shared by
/// [`run_agent_hosted`] and [`run_with_peers_hosted`] so the two runners bind host ops identically.
fn bind_host_op_bindings(
    store: &mut Store<()>,
    linker: &mut Linker<()>,
    rt_instance: &wasmtime::component::Instance,
    bindings: Vec<HostOpBinding>,
) -> Result<()> {
    use std::sync::Arc;
    let heap_idx = rt_instance
        .get_export_index(&mut *store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;
    let get_func_named =
        |store: &mut Store<()>, fname: &str| -> Result<wasmtime::component::Func> {
            let fidx = rt_instance
                .get_export_index(&mut *store, Some(&heap_idx), fname)
                .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
            rt_instance
                .get_func(&mut *store, fidx)
                .ok_or_else(|| anyhow!("runtime export `{fname}` is not a func"))
        };
    let str_get = get_func_named(store, "str-get")?;
    let str_new = get_func_named(store, "str-new")?;

    // Group the bindings by interface: `Linker::instance(iface)` may be called only ONCE per interface
    // name, so several ops sharing one interface (e.g. `Prim.exec`/`Prim.http`/`Prim.append`) must be
    // bound through a SINGLE `iface_linker`. Preserve order within an interface (first-seen order).
    let mut by_iface: Vec<(String, Vec<(String, HostOp)>)> = Vec::new();
    for b in bindings {
        let HostOpBinding { iface, op, host } = b;
        match by_iface.iter_mut().find(|(i, _)| *i == iface) {
            Some((_, ops)) => ops.push((op, host)),
            None => by_iface.push((iface, vec![(op, host)])),
        }
    }

    // Bind each op to a `func_new` closure of the right shape. Reading the arg rope (`str-get`) / minting
    // the result rope (`str-new`) inside the closure via the passed `ctx` is the `bind_runtime_into`
    // pattern. A helper reads a u32 arg handle to its String; each arm marshals per its HostOp variant.
    for (iface, ops) in by_iface {
        let mut iface_linker = linker
            .instance(&iface)
            .map_err(|e| anyhow!("linker instance {iface}: {e}"))?;
        for (op, host) in ops {
            let op_label = op.clone();
            match host {
                HostOp::StringToString(f) => {
                    let f = Arc::new(f);
                    let (sg, sn) = (str_get, str_new);
                    iface_linker.func_new(&op, move |mut ctx, params, results| {
                        let arg = read_arg_string(&mut ctx, &sg, params, &op_label)?;
                        let out = f(arg);
                        let mut made = [Val::Bool(false)];
                        sn.call(&mut ctx, &[Val::String(out)], &mut made)?;
                        sn.post_return(&mut ctx)?;
                        write_result(results, made[0].clone(), &op_label)?;
                        Ok(())
                    })?;
                }
                HostOp::StringToScalar(f) => {
                    let f = Arc::new(f);
                    let sg = str_get;
                    iface_linker.func_new(&op, move |mut ctx, params, results| {
                        let arg = read_arg_string(&mut ctx, &sg, params, &op_label)?;
                        write_result(results, Val::S64(f(arg)), &op_label)?;
                        Ok(())
                    })?;
                }
                HostOp::UnitToString(f) => {
                    let f = Arc::new(f);
                    let sn = str_new;
                    iface_linker.func_new(&op, move |mut ctx, _params, results| {
                        let out = f();
                        let mut made = [Val::Bool(false)];
                        sn.call(&mut ctx, &[Val::String(out)], &mut made)?;
                        sn.post_return(&mut ctx)?;
                        write_result(results, made[0].clone(), &op_label)?;
                        Ok(())
                    })?;
                }
            }
        }
    }
    Ok(())
}

/// Read a host op's single `u32` arg handle to its `String` via the runtime's `str-get`. Shared by the
/// String-arg [`HostOp`] arms.
fn read_arg_string(
    ctx: &mut wasmtime::StoreContextMut<()>,
    str_get: &wasmtime::component::Func,
    params: &[Val],
    op_label: &str,
) -> Result<String> {
    let h = match params.first() {
        Some(Val::U32(h)) => *h,
        other => {
            return Err(anyhow!(
                "op `{op_label}` expected a u32 arg handle, got {other:?}"
            ));
        }
    };
    let mut got = [Val::Bool(false)];
    str_get.call(&mut *ctx, &[Val::U32(h)], &mut got)?;
    str_get.post_return(&mut *ctx)?;
    match &got[0] {
        Val::String(s) => Ok(s.to_string()),
        other => Err(anyhow!("str-get returned a non-string: {other:?}")),
    }
}

/// Write a host op's single result into the first result slot (guarded — a shape mismatch is an error,
/// not a panic; the up-front shape check already proved the slot exists).
fn write_result(results: &mut [Val], value: Val, op_label: &str) -> Result<()> {
    let slot = results
        .first_mut()
        .ok_or_else(|| anyhow!("op `{op_label}` returned no result slot to write"))?;
    *slot = value;
    Ok(())
}

/// Verify a consumer's imported op `iface`.`op` has the expected `(want_params) -> (want_results)`
/// boundary shape. Generalizes [`check_model_op_shape`]/[`check_authz_op_shape`]; Ok if the op/iface
/// isn't imported (the linker reports that clearly).
fn check_host_op_shape(
    engine: &Engine,
    consumer: &Component,
    iface: &str,
    op: &str,
    want_params: &[Type],
    want_results: &[Type],
) -> Result<()> {
    let Some(sigs) = consumer
        .component_type()
        .imports(engine)
        .find(|(n, _)| *n == iface)
        .and_then(|(_, item)| iface_func_sigs(engine, &item))
    else {
        return Ok(());
    };
    let Some((_, params, results)) = sigs.iter().find(|(n, _, _)| n == op) else {
        return Ok(());
    };
    let matches_ty = |got: &[Type], want: &[Type]| {
        got.len() == want.len()
            && got
                .iter()
                .zip(want)
                .all(|(g, w)| std::mem::discriminant(g) == std::mem::discriminant(w))
    };
    if !matches_ty(params, want_params) || !matches_ty(results, want_results) {
        let shape = |ts: &[Type]| {
            ts.iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(anyhow!(
            "the host op `{iface}`.`{op}` must be `({}) -> ({})`, but the consumer imports it as \
             `({}) -> ({})`",
            shape(want_params),
            shape(want_results),
            shape(params),
            shape(results),
        ));
    }
    Ok(())
}

/// Instantiate the linked component and invoke its chosen export (or the resource-escape path), returning
/// the rendered outcome. Split out of [`run_capturing`] so the host-call observation wraps it.
// The run context (engine/component/store/linker/opts) plus the resource-drive knobs (second_call/
// drop_handle/call_member) it forwards to the closure/escape dispatch — genuinely 8 arguments.
#[allow(clippy::too_many_arguments)]
/// Byte-scan a COMPONENT's top-level sections for the `cdz-result-type` custom section (bytes-second
/// run-wiring): the guest export result-Ty map rides IN the component (rcdzc appends it), so it reaches
/// EVERY invocation incl. the spawned corpus-gate binary that pipes the raw component. No `wasmparser` dep
/// (INTERP-1): a hand walk. Skips the 8-byte preamble; a top-level id-0 custom whose name is
/// `cdz-result-type` yields its payload. Nested core modules are opaque section blobs (skipped whole), so
/// their own id-0 customs never false-match. `None` when absent/malformed -> the type-blind render.
pub(crate) fn scan_result_type_section(bytes: &[u8]) -> Option<Vec<u8>> {
    // magic (4) + version (2) + layer (2) — a component's layer differs from a core module, but we only skip.
    let mut pos = 8usize;
    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;
        let (size, adv) = read_uleb(bytes, pos)?;
        pos += adv;
        let section_end = pos.checked_add(size as usize)?;
        if section_end > bytes.len() {
            return None; // a mis-read size would mis-locate every later section — bail rather than guess.
        }
        if id == 0 {
            // custom section: <name-len:uleb><name><payload>.
            let (name_len, nadv) = read_uleb(bytes, pos)?;
            let name_start = pos + nadv;
            let name_end = name_start.checked_add(name_len as usize)?;
            if name_end <= section_end && &bytes[name_start..name_end] == b"cdz-result-type" {
                return Some(bytes[name_end..section_end].to_vec());
            }
        }
        pos = section_end; // skip the section (nested modules are opaque blobs).
    }
    None
}

/// Decode an unsigned LEB128 at `bytes[pos..]`, returning `(value, bytes_consumed)`. `None` on truncation
/// or an over-long (> u32) encoding (a section size never needs more).
fn read_uleb(bytes: &[u8], pos: usize) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift = 0u32;
    let mut i = 0usize;
    loop {
        let byte = *bytes.get(pos + i)?;
        result |= u32::from(byte & 0x7f).checked_shl(shift)?;
        i += 1;
        if byte & 0x80 == 0 {
            return Some((result, i));
        }
        shift += 7;
        if shift >= 32 {
            return None;
        }
    }
}

/// Parse rcdzc's `KIND_RESULT_TYPES` payload — newline-separated `<export-name>\t<Ty::render_name>` lines —
/// into the export->result-Ty map. A missing/empty/non-UTF-8 payload yields an empty map (type-blind).
fn parse_result_types(
    map_bytes: Option<&[u8]>,
) -> std::collections::HashMap<String, cadenza_syntax::ast::Arenas> {
    let mut map = std::collections::HashMap::new();
    if let Some(bytes) = map_bytes {
        // seq-284 binary-AST wire: the `cdz-result-type` section is ONE canonical binary AST value; the
        // codec decodes each boundary export's structured `Ty` payload into a standalone `Arenas` (rooted
        // at the type). render.rs WALKS it. TOTAL decode — a malformed section yields no entries and the
        // render falls back to type-blind.
        for (name, ty_arena) in cadenza_compile_abi::decode_result_types(bytes) {
            map.insert(name, ty_arena);
        }
    }
    map
}

/// Look up the result-Ty arena for the export being run. Keyed by the requested export name (or its kebab-
/// normalized extern form, or an interface-qualified `iface#member`'s member tail); a nullary run (no
/// `--call`) with a SOLE export uses that one entry. `None` -> the type-blind render.
fn lookup_result_ty<'a>(
    map: &'a std::collections::HashMap<String, cadenza_syntax::ast::Arenas>,
    export: Option<&str>,
) -> Option<&'a cadenza_syntax::ast::Arenas> {
    if map.is_empty() {
        return None;
    }
    if let Some(name) = export {
        let member = name.rsplit('#').next().unwrap_or(name);
        return map.get(name).or_else(|| map.get(member)).or_else(|| {
            let kebab = cadenza_syntax::extern_name::kebab_extern_name(member);
            map.get(&kebab)
        });
    }
    if map.len() == 1 {
        return map.values().next();
    }
    None
}

// The runner threads the fixed per-run set (engine/component/store/linker/opts + the closure-escape
// second_call/drop_handle/call_member + the bytes-second result_ty) — a cohesive arg set, not worth a
// bundle struct for one internal helper.
#[allow(clippy::too_many_arguments)]
fn run_export(
    engine: &Engine,
    component: &Component,
    store: &mut Store<()>,
    linker: &Linker<()>,
    opts: &RunOpts,
    // A `(then …)` two-call-on-one-handle continuation for a CLOSURE export (see `run_closure_resource`);
    // `None` on every non-closure / one-call path. Only the closure-escape dispatch below consults it.
    second_call: Option<&[String]>,
    drop_handle: bool,
    call_member: Option<&str>,
    // The GUEST result-Ty arena (the structured `Ty` payload, decoded from the component's
    // `cdz-result-type` section — seq-284 binary-AST wire). `Some` → the render sites disambiguate a
    // WIT-erased leaf via `render::render_val_typed` (Bytes `b"…"` vs `list<u8>` `#list`, Symbol `#"…"`);
    // `None` → the type-blind `render_val` (unchanged behavior).
    result_ty: Option<&cadenza_syntax::ast::Arenas>,
) -> Result<Outcome> {
    let instance = linker
        .instantiate(&mut *store, component)
        .map_err(|e| anyhow!("instantiate: {e}"))?;

    // Whether `name` (or its kebab-normalized form) resolves to a TOP-LEVEL bare component func. Shared by
    // the escape/closure dispatch below: a `--call <name>` that names a real bare func takes the plain path;
    // a name that does NOT (a compound/closure result carries no bare func under that name) routes to the
    // resource/closure escape instead.
    let names_a_top_level_func = |store: &mut Store<()>, name: &str| -> bool {
        instance.get_func(&mut *store, name).is_some() || {
            let kebab = cadenza_syntax::extern_name::kebab_extern_name(name);
            kebab != name && instance.get_func(&mut *store, &kebab).is_some()
        }
    };

    // An INTERFACE-QUALIFIED export `<iface>#<member>` (e.g. `cadenza:demo/iface#f`): a guest that exports
    // its function THROUGH a named interface INSTANCE (a `--component-name` provider, or the explicit WIT
    // world a corpus `(wit-world …)` case imposes) rather than as a top-level bare func. Resolve the member
    // inside the instance, coerce the args to its declared param types, call, and render — the interface
    // analog of the plain top-level path below. Handled FIRST so a `#`-qualified name never falls into the
    // resource-escape / closure dispatch (which key off a NON-top-level bare name).
    if let Some((iface, member)) = opts.export.as_deref().and_then(|n| n.split_once('#')) {
        let iface_idx = instance
            .get_export_index(&mut *store, None, iface)
            .ok_or_else(|| anyhow!("component exports no interface `{iface}`"))?;
        let member_idx = instance
            .get_export_index(&mut *store, Some(&iface_idx), member)
            .ok_or_else(|| anyhow!("interface `{iface}` has no member `{member}`"))?;
        let func = instance
            .get_func(&mut *store, member_idx)
            .ok_or_else(|| anyhow!("interface member `{iface}#{member}` is not a func"))?;
        let param_types: Vec<Type> = func
            .params(&*store)
            .iter()
            .map(|(_, t)| t.clone())
            .collect();
        let args = coerce_args(&opts.args, &param_types)?;
        let mut results = vec![Val::Bool(false); func.results(&*store).len()];
        return match func.call(&mut *store, &args, &mut results) {
            Ok(()) => {
                let rendered = match results.first() {
                    None => "unit".to_string(),
                    Some(Val::String(s)) => s.clone(),
                    Some(other) => match result_ty {
                        Some(t) => render::render_val_typed(other, t),
                        None => render_val(other),
                    },
                };
                let _ = func.post_return(&mut *store);
                Ok(Outcome::Value(rendered))
            }
            Err(e) => Ok(Outcome::Trap(trap_message(&e))),
        };
    }

    // The RESOURCE ESCAPE (`DESIGN-value-heap-rcdzc.md` §3a): a program whose result is a COMPOUND
    // exports no bare function — it publishes a `cadenza:run/run` instance carrying `make : () -> own<t>`
    // + `encode : (own<t>) -> list<u8>`. Call `make` then `encode`, DECODE the canonical binary value
    // form with the shared codec, and pretty-print `(: value type)` — the value crossing the boundary as
    // a strongly-typed resource, rendered by the host (not spelled out in wasm). Taken when the run instance
    // is present AND the named export (if any) is NOT a top-level bare func — a compound export carries no
    // bare func under its name, so `(call greet)` on a `String`-returning `greet` routes here (the escape
    // has ONE compound result; its make/encode take no export name). No `--call` (the corpus's nullary
    // `main`) also routes here.
    if has_run_instance(engine, component)
        && sole_func_export(engine, component).is_none()
        && opts
            .export
            .as_deref()
            .map(|name| !names_a_top_level_func(&mut *store, name))
            .unwrap_or(true)
    {
        return run_resource_escape(
            &mut *store,
            &instance,
            &opts.args,
            drop_handle,
            call_member,
            second_call,
        );
    }

    // The CLOSURE ESCAPE (`DESIGN-closure-host-resource-rcdzc.md`, C-HOST-1): a program whose result is a
    // closure exports the `cadenza:closure/exports` instance (`make`/`call`), not a bare function. Call
    // `make()` → the closure handle, then `call(handle, args…)` with the caller's arguments, rendering the
    // result. Taken when the closure interface is present AND the named export is NOT a TOP-LEVEL bare func
    // (so the args are the closure's arguments). A MIXED program (a closure export ALONGSIDE a plain export)
    // has BOTH the closure interface and top-level funcs — `--call <plain>` resolves as a bare func and
    // falls through to the plain path below; `--call <closure>` (or no `--call`, the corpus's `main`) has no
    // top-level func and routes here.
    if has_closure_instance(engine, component)
        && opts
            .export
            .as_deref()
            .map(|name| !names_a_top_level_func(&mut *store, name))
            .unwrap_or(true)
    {
        return run_closure_resource(
            engine,
            component,
            &mut *store,
            &instance,
            opts.export.as_deref(),
            &opts.args,
            second_call,
            drop_handle,
            call_member,
        );
    }

    // Resolve the export to call: the named one, or the sole function export found by signature.
    let export_name = match &opts.export {
        Some(name) => name.clone(),
        None => sole_func_export(engine, component).ok_or_else(|| {
            anyhow!(
                "no --call given and the component has no single function export to default to{}",
                callable_exports_hint(engine, component)
            )
        })?,
    };
    // The component's extern name is KEBAB-CASE, but a caller names the export by its SOURCE identifier
    // (`--call fA`), which may not be kebab (`fA`, `my_func`). The compiler normalized the extern name at
    // emit (`kebab_extern_name`); resolve the SOURCE name through the SAME deterministic rule so a caller
    // still uses the source name. Try the verbatim name first (already-kebab / core-level exports match
    // it unchanged), then the normalized form.
    let func = instance
        .get_func(&mut *store, &export_name)
        .or_else(|| {
            let kebab = cadenza_syntax::extern_name::kebab_extern_name(&export_name);
            (kebab != export_name)
                .then(|| instance.get_func(&mut *store, &kebab))
                .flatten()
        })
        .ok_or_else(|| {
            anyhow!(
                "component exports no function `{export_name}`{}",
                callable_exports_hint(engine, component)
            )
        })?;

    // Coerce the raw argument strings to the export's declared parameter types.
    let param_types: Vec<Type> = func
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let args = coerce_args(&opts.args, &param_types)?;

    let result_count = func.results(&*store).len();
    let mut results = vec![Val::Bool(false); result_count];
    match func.call(&mut *store, &args, &mut results) {
        Ok(()) => {
            let rendered = match results.first() {
                None => "unit".to_string(),
                // A compound program's entry returns its result ALREADY rendered to canonical text
                // (the program walked its value through the runtime and assembled the string); take a
                // returned string verbatim rather than re-quoting it. A scalar result renders directly.
                Some(Val::String(s)) => s.clone(),
                Some(other) => match result_ty {
                    Some(t) => render::render_val_typed(other, t),
                    None => render_val(other),
                },
            };
            let _ = func.post_return(&mut *store);
            Ok(Outcome::Value(rendered))
        }
        Err(e) => Ok(Outcome::Trap(trap_message(&e))),
    }
}

/// The required runtime a `component` records, if any: its runtime import (recognized by the fixed
/// `cadenza:runtime/heap` interface prefix) and the content address carried in that import name.
/// A component with no such import produces `None` (a scalar/const program needs no runtime).
pub fn required_runtime(component_bytes: &[u8]) -> Result<Option<RuntimeReq>> {
    let engine = engine();
    let component =
        jit_component(&engine, component_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    Ok(find_runtime_req(&engine, &component))
}

/// Find the runtime import on `component` and parse its content-address suffix into a [`RuntimeReq`].
fn find_runtime_req(engine: &Engine, component: &Component) -> Option<RuntimeReq> {
    component
        .component_type()
        .imports(engine)
        .map(|(name, _)| name.to_string())
        .find(|name| import_is_runtime(name))
        .map(|import_name| {
            let hash = hash_from_import(&import_name);
            RuntimeReq { import_name, hash }
        })
}

/// Is `name` the value-heap runtime import? It is `cadenza:runtime/heap` optionally followed by a
/// version/build-metadata suffix (`@…`) — so match the interface up to the version boundary.
fn import_is_runtime(name: &str) -> bool {
    name == RUNTIME_IFACE || name.starts_with(&format!("{RUNTIME_IFACE}@"))
}

/// The content address recorded in a runtime import name. The name is
/// `cadenza:runtime/heap@<semver>+<hash>`; the hash is the semver build-metadata (after `+`). An
/// import with no `+<hash>` (an unpinned interface) yields an empty string — no content address recorded.
fn hash_from_import(name: &str) -> String {
    name.rsplit_once('+')
        .map(|(_, h)| h.to_string())
        .unwrap_or_default()
}

/// Compose the value-heap runtime: instantiate the runtime component, then forward each function its
/// heap interface exports into the program's import — bound under the program's EXACT import name
/// (`req.import_name`, which carries the content-address suffix). The function names are read off the
/// runtime's own instance type, so the composition always matches the supplied runtime.
fn compose_runtime(
    engine: &Engine,
    store: &mut Store<()>,
    linker: &mut Linker<()>,
    req: &RuntimeReq,
    opts: &RunOpts,
) -> Result<()> {
    let (rt_instance, heap_func_names) = instantiate_runtime(engine, store, req, opts)?;
    bind_runtime_into(
        engine,
        store,
        linker,
        &req.import_name,
        &rt_instance,
        &heap_func_names,
    )
}

/// Instantiate the value-heap runtime component ONCE in `store`, returning its instance + the heap-op
/// function names (read off its own type). Split out of [`compose_runtime`] so a SHARED runtime instance
/// can be bound into SEVERAL components' imports (X5: consumer + peers share one heap so a `value` handle
/// one produces is meaningful to another — component-abi.md §A Cross-Component Handle Is Meaningful Only
/// In The Shared Runtime Instance).
fn instantiate_runtime(
    engine: &Engine,
    store: &mut Store<()>,
    req: &RuntimeReq,
    opts: &RunOpts,
) -> Result<(wasmtime::component::Instance, Vec<String>)> {
    let runtime_bytes = opts.runtime.as_deref().ok_or_else(|| {
        anyhow!(
            "component requires the value-heap runtime {} but none was provided (the host resolves \
             it by content address from the store; build it with `cargo xtask build`)",
            req.hash
        )
    })?;
    let runtime = load_runtime_component(engine, runtime_bytes, opts)?;
    let heap_func_names = heap_interface_funcs(engine, &runtime)?;
    let mut rt_linker: Linker<()> = Linker::new(engine);
    // TRANSITIVE COMPOSE (FINDING#23, leaves-first): the runtime is NOT a leaf — its world imports
    // `cadenza:nfc/normalize` (NFC is the runtime's dependency). So compose the NFC component INTO the
    // runtime's linker BEFORE instantiating the runtime, exactly as we later compose the runtime into the
    // program's linker (nfc → runtime → program). This mirrors the recursive dep-compose v-agent-harness's
    // kernel host uses; here it is one known level (the runtime imports one dep), but the shape is the
    // general leaves-first walk. A runtime that imports nothing (older runtime) skips this.
    compose_nfc_into_runtime_linker(engine, store, &mut rt_linker, &runtime, opts)?;
    let rt_instance = rt_linker
        .instantiate(&mut *store, &runtime)
        .map_err(|e| anyhow!("instantiate runtime: {e}"))?;
    Ok((rt_instance, heap_func_names))
}

/// The NFC interface the runtime imports (FINDING#23). The runtime's world declares `import
/// cadenza:nfc/normalize`; the host composes the stored NFC component (resolved by content hash) into the
/// runtime's linker under this exact interface name.
const NFC_IFACE: &str = "cadenza:nfc/normalize";

/// If `runtime` imports `cadenza:nfc/normalize`, instantiate the NFC component (`opts.nfc`) and forward its
/// `normalize` interface's funcs into `rt_linker` under the import name — the leaves-first transitive
/// compose. The NFC component is itself a LEAF (imports nothing), so it instantiates against a fresh empty
/// linker. If the runtime declares no such import (an older runtime), this is a no-op. Mirrors
/// `bind_runtime_into` (extract the composed instance's interface exports, forward via `func_new`).
fn compose_nfc_into_runtime_linker(
    engine: &Engine,
    store: &mut Store<()>,
    rt_linker: &mut Linker<()>,
    runtime: &Component,
    opts: &RunOpts,
) -> Result<()> {
    // Does the runtime import the NFC interface, and under exactly what name? The NFC import is
    // SELF-DESCRIBING (operator directive 2026-08-23): its name carries the NFC component's content address
    // as a semver build-metadata suffix — `cadenza:nfc/normalize@0.0.0+<hash>`, stamped into the heap at
    // build time (`cdz-component-rewrite`), exactly like a program's `cadenza:runtime/heap@0.0.0+<hash>`
    // import. So match on the interface PREFIX (the version suffix varies) and keep the FULL import name —
    // the linker must satisfy the import under the name the runtime actually declares.
    let nfc_import = runtime
        .component_type()
        .imports(engine)
        .map(|(name, _)| name.to_string())
        .find(|name| name == NFC_IFACE || name.starts_with(&format!("{NFC_IFACE}@")));
    let Some(nfc_import_name) = nfc_import else {
        return Ok(()); // leaf runtime — nothing to compose
    };
    // The content address to resolve the NFC component by = the substring after `+` in the import name
    // (string split, encoding-agnostic — same as the heap runtime import, `find_runtime_req`). No
    // `runtime.toml` / mapping file is consulted: the import itself says how to resolve the dependency
    // (zero runtime indirection — operator directive). A bare import with no `+<hash>` is an unstamped heap
    // (a build bug), reported here rather than silently falling back to a mapping.
    let nfc_hash = nfc_import_name.rsplit_once('+').map(|(_, h)| h);
    let nfc_bytes = nfc_hash.and_then(|h| resolve_nfc_by_hash(opts, h)).ok_or_else(|| {
        anyhow!(
            "the value-heap runtime imports `{nfc_import_name}` (its NFC dependency) but the NFC component \
             could not be resolved by its inline content address from the store — the import must carry a \
             `+<hash>` suffix naming `<store>/<hash>.wasm` (built + stamped by `cargo xtask build`); set \
             CDZ_STORE to the store dir if it is elsewhere"
        )
    })?;
    let nfc =
        load_guest(engine, &nfc_bytes, opts).map_err(|e| anyhow!("load NFC component: {e}"))?;
    // NFC is a leaf (imports nothing) → instantiate against a fresh empty linker.
    let nfc_linker: Linker<()> = Linker::new(engine);
    let nfc_instance = nfc_linker
        .instantiate(&mut *store, &nfc)
        .map_err(|e| anyhow!("instantiate NFC component: {e}"))?;
    // Forward the NFC interface's funcs into the runtime's linker under the import name (verbatim).
    let nfc_idx = nfc_instance
        .get_export_index(&mut *store, None, NFC_IFACE)
        .ok_or_else(|| anyhow!("NFC component does not export {NFC_IFACE}"))?;
    let func_names = interface_func_names(engine, &nfc, NFC_IFACE)?;
    // Bind under the runtime's ACTUAL import name (the versioned `cadenza:nfc/normalize@0.0.0+<hash>`),
    // not the bare interface — the linker satisfies an import by its declared name. The NFC component
    // itself still EXPORTS the bare `NFC_IFACE` (only the heap's IMPORT is stamped), so its export lookups
    // below stay on `NFC_IFACE`.
    let mut iface = rt_linker
        .instance(&nfc_import_name)
        .map_err(|e| anyhow!("runtime linker instance {nfc_import_name}: {e}"))?;
    for fname in &func_names {
        let fidx = nfc_instance
            .get_export_index(&mut *store, Some(&nfc_idx), fname)
            .ok_or_else(|| anyhow!("NFC component missing `{fname}`"))?;
        let f = nfc_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| anyhow!("NFC export `{fname}` is not a func"))?;
        iface.func_new(fname, move |mut ctx, params, results| {
            f.call(&mut ctx, params, results)?;
            f.post_return(&mut ctx)?;
            Ok(())
        })?;
    }
    Ok(())
}

/// Resolve the NFC component's bytes from the value-heap store BY ITS CONTENT ADDRESS (`hash`), the address
/// read inline from the runtime's self-describing NFC import (`cadenza:nfc/normalize@0.0.0+<hash>`). NO
/// `runtime.toml` / mapping file is consulted — the import names the dependency, and the store is a pure CAS
/// (`<store>/<hash>.wasm`); this is content addressing, not indirection (operator directive 2026-08-23: no
/// mapping file passed to any executable). The store dir is, in precedence: `opts.runtime_cache_dir` (the
/// CLI/`cdz` path sets this to the resolved store), else `CDZ_STORE`, else the compiled default
/// (`<repo>/target/cadenza-store`). Loads `<store>/<hash>.wasm` and VERIFIES its content address matches
/// `hash` (so a corrupt/substituted entry can't compose silently — the same integrity check the runtime
/// resolution does). Returns `None` (→ a clear compose error) if any step fails. cdz-run needs no
/// `REQUIRED_NFC_HASH` from the compiler — the hash rides in the import — so it stays free of `rcdzc`.
fn resolve_nfc_by_hash(opts: &RunOpts, hash: &str) -> Option<Vec<u8>> {
    let store = opts
        .runtime_cache_dir
        .clone()
        .or_else(|| std::env::var_os("CDZ_STORE").map(std::path::PathBuf::from))
        .unwrap_or_else(nfc_default_store);
    if opts.precompiled {
        // PRECOMPILED (seq-250 AOT corpus-exec): the value-heap runtime imports the NFC component and the
        // Linker composes it from the store at instantiate — but the cranelift-free exec cannot JIT a raw
        // `.wasm`. So load the store's PRECOMPILED sibling `<hash>.cwasm` (produced once by the cranelift-ON
        // tool alongside the runtime: `cdz-run <store>/<hash>.wasm --precompile-out <store>/<hash>.cwasm`);
        // `load_guest` then `deserialize`s it. No SOURCE content-address check here — a `.cwasm` hashes
        // differently than its `.wasm` source (the address `hash` names the source); `Component::deserialize`
        // validates the artifact's OWN integrity + engine-compatibility instead. This generalizes to any
        // store-composed dependency the runtime pulls, not just NFC — precompile each store component once.
        return std::fs::read(store.join(format!("{hash}.cwasm"))).ok();
    }
    let bytes = std::fs::read(store.join(format!("{hash}.wasm"))).ok()?;
    (crate::cli::content_address(&bytes) == hash).then_some(bytes)
}

/// The compiled default value-heap store (`<repo>/target/cadenza-store`), resolved from this crate's
/// manifest location — the fallback when neither `opts.runtime_cache_dir` nor `CDZ_STORE` pins one.
fn nfc_default_store() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest
        .ancestors()
        .nth(4)
        .unwrap_or(&manifest)
        .to_path_buf();
    repo.join("target/cadenza-store")
}

/// Forward each already-extracted PEER interface into `linker`, so a component importing `cadenza:pkg/iface`
/// resolves it to the like-named peer's exported funcs (U11 chain support). Each entry is
/// `(interface, [(fname, Func)])` — the funcs pulled off an earlier-instantiated peer instance. Shared by
/// each peer's linker (dependency order) and the consumer's linker. A `func_new` closure calls the peer
/// func then its `post_return`, exactly as the inline single-pass binding did.
/// One interface func's signature: its name, its param types (in order), and its result types.
type IfaceFuncSig = (String, Vec<Type>, Vec<Type>);

/// The full SIGNATURE (param types, result types) of every func an interface INSTANCE type exports, by
/// func name. Reads a `ComponentInstance` item's exports (the funcs an interface offers). Used to
/// compare a consumer's IMPORTED interface against a peer's EXPORTED interface at compose time — both
/// the arity (param/result count) AND the position-by-position component types must agree.
fn iface_func_sigs(engine: &Engine, item: &ComponentItem) -> Option<Vec<IfaceFuncSig>> {
    match item {
        ComponentItem::ComponentInstance(inst) => Some(
            inst.exports(engine)
                .filter_map(|(fname, i)| match i {
                    ComponentItem::ComponentFunc(f) => Some((
                        fname.to_string(),
                        f.params().map(|(_, t)| t).collect(),
                        f.results().collect(),
                    )),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

/// Validate that a PROVIDER component's EXPORTED interface func matches the SIGNATURE the IMPORTER
/// declares for the interface `iface` — the op is PRESENT, and both its arity (param/result count) and
/// its position-by-position component types agree. `who` labels the importer in the error (`the
/// consumer` / `peer \`cadenza:mid/api\``). A raw-closure forward (`bind_peer_ifaces_into`) does no
/// subtype check, so a mismatch (a MISSING op, a differing arity, OR a differing type) would surface
/// only as an opaque error: a missing op as wasmtime's instance-level "a matching implementation was
/// not found in the linker" (never naming the op), an arity/type mismatch as a runtime trap. Returns
/// `Ok` when the importer does not import `iface` at all (nothing to check).
fn check_one_iface_binding(
    engine: &Engine,
    importer_type: &wasmtime::component::types::Component,
    provider: &Component,
    iface: &str,
    who: &str,
) -> Result<()> {
    // The importer's declared signature for this interface (if it imports it at all).
    let Some(want_iface) = importer_type
        .imports(engine)
        .find(|(n, _)| *n == iface)
        .and_then(|(_, item)| iface_func_sigs(engine, &item))
    else {
        return Ok(());
    };
    let got_iface: Vec<IfaceFuncSig> = provider
        .component_type()
        .exports(engine)
        .find(|(n, _)| *n == iface)
        .and_then(|(_, item)| iface_func_sigs(engine, &item))
        .unwrap_or_default();
    for (fname, want_params, want_results) in &want_iface {
        let Some((_, got_params, got_results)) = got_iface.iter().find(|(n, _, _)| n == fname)
        else {
            let offered: Vec<&str> = got_iface.iter().map(|(n, _, _)| n.as_str()).collect();
            return Err(anyhow!(
                "peer `{iface}` does not export op `{fname}`, which {who} binds — the peer's \
                 interface offers {} — the peer must export every op the binding names",
                if offered.is_empty() {
                    "no ops".to_string()
                } else {
                    format!("[{}]", offered.join(", "))
                },
            ));
        };
        // Arity first (the count disagreement is the clearest message), then position-by-position types
        // (matching arity but a differing type, e.g. s64 vs f64, is the subtler face).
        if got_params.len() != want_params.len() || got_results.len() != want_results.len() {
            return Err(anyhow!(
                "peer `{iface}` op `{fname}` signature mismatch: {who} imports it taking {} \
                 argument(s) and returning {} result(s), but the peer exports it taking {} \
                 argument(s) and returning {} result(s) — the peer's interface must match the binding",
                want_params.len(),
                want_results.len(),
                got_params.len(),
                got_results.len(),
            ));
        }
        let param_mismatch = want_params
            .iter()
            .zip(got_params.iter())
            .position(|(w, g)| w != g)
            .map(|i| ("argument", i, &want_params[i], &got_params[i]));
        let result_mismatch = want_results
            .iter()
            .zip(got_results.iter())
            .position(|(w, g)| w != g)
            .map(|i| ("result", i, &want_results[i], &got_results[i]));
        if let Some((role, idx, want, got)) = param_mismatch.or(result_mismatch) {
            return Err(anyhow!(
                "peer `{iface}` op `{fname}` type mismatch at {role} {idx}: {who} imports it with \
                 type `{want:?}` there, but the peer exports it with type `{got:?}` — the peer's \
                 interface must match the binding"
            ));
        }
    }
    Ok(())
}

/// Validate every consumer↔peer AND peer↔peer interface binding at compose time (see
/// [`check_one_iface_binding`] for the per-binding check + why the raw-closure forward needs it). Two
/// layers: the CONSUMER's imports against each peer that provides them, AND — for a MULTI-HOP chain
/// (U11: A→B→C, where a middle peer B imports an earlier peer A) — each peer's imports against the
/// EARLIER peers (dependency order, exactly how `run_with_peers` binds them). Without the second layer
/// a mismatch in a chain peer's binding (B's binding of A) traps opaquely instead of naming the op.
fn check_peer_iface_signatures(
    engine: &Engine,
    consumer: &Component,
    peers: &[(Component, String)],
) -> Result<()> {
    // Layer 1: the top consumer against each peer that provides an interface it imports.
    let ctype = consumer.component_type();
    for (peer_component, iface) in peers {
        check_one_iface_binding(engine, &ctype, peer_component, iface, "the consumer")?;
    }
    // Layer 2: each peer against every EARLIER peer (the chain — a middle component imports an earlier
    // peer's interface). `run_with_peers` binds earlier peers into a later peer's linker in dependency
    // order, so a peer at index `i` can import any peer at `j < i`.
    for (i, (peer_component, iface)) in peers.iter().enumerate() {
        let ptype = peer_component.component_type();
        let who = format!("peer `{iface}`");
        for (earlier_component, earlier_iface) in &peers[..i] {
            check_one_iface_binding(engine, &ptype, earlier_component, earlier_iface, &who)?;
        }
    }
    Ok(())
}

fn bind_peer_ifaces_into(
    linker: &mut Linker<()>,
    peer_ifaces: &[(String, Vec<(String, wasmtime::component::Func)>)],
) -> Result<()> {
    for (interface, funcs) in peer_ifaces {
        let mut iface = linker
            .instance(interface)
            .map_err(|e| anyhow!("linker instance {interface}: {e}"))?;
        for (fname, f) in funcs {
            let f = *f;
            iface.func_new(fname, move |mut ctx, params, results| {
                f.call(&mut ctx, params, results)?;
                f.post_return(&mut ctx)?;
                Ok(())
            })?;
        }
    }
    Ok(())
}

/// Forward each heap-op function of an already-instantiated runtime instance into `linker` under
/// `import_name` (the exact hashed name the importing component declared). Reused to bind ONE runtime
/// instance into multiple components' imports (X5).
fn bind_runtime_into(
    engine: &Engine,
    store: &mut Store<()>,
    linker: &mut Linker<()>,
    import_name: &str,
    rt_instance: &wasmtime::component::Instance,
    heap_func_names: &[String],
) -> Result<()> {
    let _ = engine;
    let heap_idx = rt_instance
        .get_export_index(&mut *store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;
    // Bind under the program's exact (hashed) import name, not the bare interface — that is the name
    // the program declared, and the linker matches names verbatim.
    let mut iface = linker
        .instance(import_name)
        .map_err(|e| anyhow!("linker instance {import_name}: {e}"))?;
    for fname in heap_func_names {
        let fidx = rt_instance
            .get_export_index(&mut *store, Some(&heap_idx), fname)
            .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
        let f = rt_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| anyhow!("runtime export `{fname}` is not a func"))?;
        iface.func_new(fname, move |mut ctx, params, results| {
            f.call(&mut ctx, params, results)?;
            f.post_return(&mut ctx)?;
            Ok(())
        })?;
    }
    Ok(())
}

/// Bind every HOST-effect import the component declares (E2h) so its delegated operations resolve to the
/// recorded responses, consumed in call order. A host effect is imported as an INSTANCE (the interface);
/// each function in it is a delegated operation. We enumerate the imported instances OFF THE COMPONENT
/// TYPE (never a hard-coded list — `host-interface-binding.md` §Which Host Functions Exist Is The
/// Target's Concern), skipping the value-heap runtime instance (bound by `compose_runtime`), and bind
/// each func via a dynamic closure that pops the next response and coerces it to the func's declared
/// result type. Responses are shared through an `Rc<RefCell<_>>` cursor (a one-shot single-threaded run).
fn bind_host_imports(
    engine: &Engine,
    component: &Component,
    linker: &mut Linker<()>,
    opts: &RunOpts,
    observed: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    // Interface names ALREADY bound (as cross-component PEERS, X4) — skip them here so a peer interface
    // is not also bound as a host effect (a double-bind is a linker error). Empty for a plain run.
    skip: &[String],
) -> Result<()> {
    use std::sync::{Arc, Mutex};
    // The shared response cursor — every bound host func pops the next response in order. `Arc<Mutex>`
    // (not `Rc`) because wasmtime requires the host closure be `Send + Sync`; a run is single-threaded,
    // so the mutex is uncontended.
    let cursor = Arc::new(Mutex::new(0usize));
    let responses = Arc::new(opts.host_responses.clone());

    // Enumerate the imported instances (host effect interfaces) off the component type. The runtime
    // interface (if imported) is bound elsewhere — skip it here. EVERY func is bound (including a
    // unit-result op like `log.emit`, which returns nothing) so a delegated call is always satisfied.
    // One entry per imported instance: its interface name + its ops (each `(op-name, result-type?)`).
    type HostIface = (String, Vec<(String, Option<Type>)>);
    let imports: Vec<HostIface> = component
        .component_type()
        .imports(engine)
        .filter_map(|(name, item)| {
            if is_runtime_import_name(name) || skip.iter().any(|s| s == name) {
                return None;
            }
            if let ComponentItem::ComponentInstance(inst) = item {
                let funcs: Vec<(String, Option<Type>)> = inst
                    .exports(engine)
                    .filter_map(|(fname, i)| match i {
                        // The op's declared result type, if any — a unit-result op (`func()`) has none,
                        // and consumes NO response (it is still bound + observed).
                        ComponentItem::ComponentFunc(f) => {
                            Some((fname.to_string(), f.results().next()))
                        }
                        _ => None,
                    })
                    .collect();
                Some((name.to_string(), funcs))
            } else {
                None
            }
        })
        .collect();

    for (iface_name, funcs) in imports {
        let mut iface = linker
            .instance(&iface_name)
            .map_err(|e| anyhow!("linker instance {iface_name}: {e}"))?;
        for (fname, ret_ty) in funcs {
            let cursor = Arc::clone(&cursor);
            let responses = Arc::clone(&responses);
            let observed = Arc::clone(observed);
            let op_label = format!("{iface_name}.{fname}");
            iface.func_new(&fname, move |_ctx, params, results| {
                // OBSERVE the call — append its dotted `E.op` in call order (so the gate can verify the
                // sequence against `(host-calls …)`). When the call carries STRING arguments (a
                // `report.fail("msg")` / `log.emit("…")`), append them after a TAB so a consumer that
                // wants the message (`cdz test`, whose failure path emits the assertion text) can read it —
                // WITHOUT polluting the op field: `main.rs` splits the entry on the first tab, so the
                // `host-call\t<op>` line the gate parses keeps a clean `<op>`, and the message rides a
                // separate `host-arg` line. A non-string arg (a scalar) is not captured (nothing reads it).
                let str_args: Vec<String> = params
                    .iter()
                    .filter_map(|v| match v {
                        Val::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                let entry = if str_args.is_empty() {
                    op_label.clone()
                } else {
                    format!("{op_label}\t{}", str_args.join(" "))
                };
                observed.lock().expect("observed calls mutex").push(entry);
                // adv-65 (HIGH differential): a UNIT-result op returns nothing, but it STILL CONSUMES its
                // response row when the fixture supplied one for it — the corpus model is "responses are
                // consumed IN ORDER of the calls made", so a `(host (io) (do (io.ping k) (+ (io.get k) k)))`
                // with responses [io.ping, io.get] must have `io.ping` advance the cursor past ITS row, so
                // the later `io.get` reads its OWN row — not `io.ping`'s (the wasm-vs-rust wrong-value:
                // io.get was reading ping's row → 0+3=3 instead of 7+3=10). Previously a unit op advanced
                // NOTHING, desyncing every unit-op-then-value-op sequence. But a PURE observe-only unit op
                // (H8's `log.emit` — no `(host-response …)` row at all) must NOT consume, or it would skip a
                // later value op's row. So consume IFF the row at the cursor is FOR THIS OP: match the
                // current response's recorded op against this op (both KEBAB-normalized, since the fixture's
                // op may be source-cased while `op_label` is the WIT export name — the same two-sided
                // normalization cdz-run uses elsewhere). Row-present-and-matching → consume-and-discard;
                // row-absent-or-other-op (a pure observe-only unit op) → leave the cursor for the value op.
                if results.is_empty() {
                    let mut idx = cursor.lock().expect("host response cursor mutex");
                    if let Some(resp) = responses.get(*idx) {
                        let want = cadenza_syntax::extern_name::kebab_extern_name(&op_label);
                        let have = cadenza_syntax::extern_name::kebab_extern_name(&resp.op);
                        if want == have {
                            *idx += 1;
                        }
                    }
                }
                // A scalar-result op pops the next recorded response, coerces it to the result type, returns it.
                if let Some(slot) = results.get_mut(0) {
                    let ret_ty = ret_ty
                        .clone()
                        .expect("a result slot implies a declared result type");
                    let mut idx = cursor.lock().expect("host response cursor mutex");
                    let resp = responses.get(*idx).ok_or_else(|| {
                        // Name the OP, the CALL NUMBER (1-based — `*idx` is the 0-based cursor), and how many
                        // responses were supplied, so an exhausted `--host-response` list points straight at
                        // the culprit (`cdz run` was surfacing only the bare wasmtime "error while executing"
                        // wrapper — the actionable cause was buried; a fleet breaker flagged this).
                        anyhow!(
                            "host call `{op_label}` has no recorded response \
                             (call {} of the run; {} response(s) supplied via --host-response)",
                            *idx + 1,
                            responses.len()
                        )
                    })?;
                    *idx += 1;
                    *slot = coerce_one(&scalar_of_value_form(&resp.value), &ret_ty)?;
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

/// Extract the bare scalar text from a response value form: `(: 10 Int64)` → `10`, or a bare `10` → `10`.
/// The corpus records a response as a typed `(: value Type)` form; the runner coerces the value to the
/// op's declared boundary result type, so only the value text is needed here.
fn scalar_of_value_form(form: &str) -> String {
    let t = form.trim();
    if let Some(inner) = t.strip_prefix("(:").and_then(|s| s.strip_suffix(')')) {
        // `(: <value> <Type>)` — the value is the first whitespace-delimited token after `:`.
        let mut it = inner.split_whitespace();
        if let Some(v) = it.next() {
            return v.to_string();
        }
    }
    t.to_string()
}

/// Whether `name` is the value-heap runtime import (recognized by the fixed interface prefix) — bound by
/// `compose_runtime`, so `bind_host_imports` skips it.
fn is_runtime_import_name(name: &str) -> bool {
    name.starts_with(RUNTIME_IFACE)
}

/// The names of the functions the runtime's `cadenza:runtime/heap` interface exports, read off the
/// component type — the source of truth for what to forward, so nothing is hard-coded.
fn heap_interface_funcs(engine: &Engine, runtime: &Component) -> Result<Vec<String>> {
    interface_func_names(engine, runtime, RUNTIME_IFACE)
}

/// The function names a component exports under `iface_name` (a component-instance export). Used to bind
/// the runtime's `heap` funcs into a program's linker AND to bind the NFC component's `normalize` funcs into
/// the runtime's linker (the transitive compose) — same shape, different interface.
fn interface_func_names(
    engine: &Engine,
    component: &Component,
    iface_name: &str,
) -> Result<Vec<String>> {
    for (name, item) in component.component_type().exports(engine) {
        if name != iface_name {
            continue;
        }
        if let ComponentItem::ComponentInstance(inst) = item {
            return Ok(inst
                .exports(engine)
                .filter_map(|(fname, i)| {
                    matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                })
                .collect());
        }
    }
    Err(anyhow!(
        "component does not export the {iface_name} interface"
    ))
}

/// The name of the component's sole top-level FUNCTION export, if there is exactly one — the default
/// entry when `--call` is omitted. Interface/instance exports are ignored; only bare functions count.
fn sole_func_export(engine: &Engine, component: &Component) -> Option<String> {
    let mut only = None;
    for (name, item) in component.component_type().exports(engine) {
        if let ComponentItem::ComponentFunc(_) = item {
            if only.is_some() {
                return None; // more than one — ambiguous, require --call
            }
            only = Some(name.to_string());
        }
    }
    only
}

/// Every top-level FUNCTION export name of `component`, in declaration order — the set a `--call` can
/// name. Used to enrich an export-selection diagnostic ("no function `addd`" / "no single default")
/// with the actual choices, so a caller who typoed or forgot the name is told what IS callable rather
/// than left to guess (the rustc/cargo bar: name the alternatives). Empty when the component is not a
/// plain-function component (e.g. a resource-escape/closure program exports through an instance).
fn func_export_names(engine: &Engine, component: &Component) -> Vec<String> {
    component
        .component_type()
        .exports(engine)
        .filter_map(|(name, item)| match item {
            ComponentItem::ComponentFunc(_) => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

/// Render the callable-export list as a `--help`-style suffix for an export-selection error: `; the
/// component's function exports are: add, sub` — or a clear note when there are none to name.
fn callable_exports_hint(engine: &Engine, component: &Component) -> String {
    let names = func_export_names(engine, component);
    if names.is_empty() {
        "; the component has no plain function exports to call (it may export through an instance)"
            .to_string()
    } else {
        format!(
            "; the component's function exports are: {}",
            names.join(", ")
        )
    }
}

/// The well-known instance a resource-escape program exports its result through (`make`/`encode` live
/// inside it). `cdz-run` recognizes this instance to take the resource-decode path.
const RUN_INTERFACE: &str = "cadenza:run/run";

/// Whether `component` exports `iface_name` as a component-INSTANCE (not a bare function or a type). The
/// shared marker-detection behind `has_run_instance` / `has_closure_instance`.
fn exports_instance(engine: &Engine, component: &Component, iface_name: &str) -> bool {
    component
        .component_type()
        .exports(engine)
        .any(|(name, item)| {
            name == iface_name && matches!(item, ComponentItem::ComponentInstance(_))
        })
}

/// Whether `component` exports a `cadenza:run/run` INSTANCE — the marker of a resource-escape program
/// (its compound result crosses as a resource with a `make`/`encode` pair, not a bare function).
fn has_run_instance(engine: &Engine, component: &Component) -> bool {
    exports_instance(engine, component, RUN_INTERFACE)
}

/// The interface a CLOSURE-resource export publishes under (`make`/`call` live inside it) —
/// `DESIGN-closure-host-resource-rcdzc.md`, C-HOST-1. A closure crossing the boundary becomes a resource
/// the host holds + invokes; `cdz-run` recognizes this instance to take the closure-call path.
const CLOSURE_INTERFACE: &str = "cadenza:closure/exports";

/// Whether `component` exports a `cadenza:closure/exports` INSTANCE — the marker of a closure-resource
/// program (its result is a closure crossing as a resource with a `make`/`call` pair).
fn has_closure_instance(engine: &Engine, component: &Component) -> bool {
    exports_instance(engine, component, CLOSURE_INTERFACE)
}

/// The FUNCTION names the `cadenza:closure/exports` instance exports — used to distinguish a round-trip
/// component (named producer + consumer funcs, NO `call` method) from the single/multi-export shape
/// (which has a `call`). The same instance-func read as [`interface_func_names`], but a missing interface
/// is a plain empty list here (not an error), so a non-closure component simply has no closure funcs.
fn closure_interface_funcs(engine: &Engine, component: &Component) -> Vec<String> {
    interface_func_names(engine, component, CLOSURE_INTERFACE).unwrap_or_default()
}

/// Run a ROUND-TRIP closure program (C-HOST-4): the host produces a closure handle from a PRODUCER export,
/// then threads it BACK into a CONSUMER export that applies it. Recognized when the closure interface has
/// NO `call` method (the single/multi-export shape) but a named CONSUMER (a func whose FIRST param is the
/// resource handle). The corpus names the CONSUMER in `(call <consumer> args…)`; the driver finds the sole
/// PRODUCER (the other func, whose result is the resource — every non-consumer func), calls it with the
/// LEADING args (its own params), then the consumer with the produced handle + the REMAINING args. So
/// `(call apply-it 10 5)` → `make-adder(10)` → handle → `apply-it(handle, 5)`.
fn run_roundtrip_closure(
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    iface: &wasmtime::component::ComponentExportIndex,
    consumer_name: &str,
    iface_funcs: &[String],
    arg_strs: &[String],
) -> Result<Outcome> {
    let get = |store: &mut Store<()>, name: &str| -> Result<wasmtime::component::Func> {
        let idx = instance
            .get_export_index(&mut *store, Some(iface), name)
            .ok_or_else(|| {
                anyhow!("round-trip closure: `{CLOSURE_INTERFACE}` exports no `{name}`")
            })?;
        instance
            .get_func(&mut *store, idx)
            .ok_or_else(|| anyhow!("round-trip closure: `{name}` is not a function"))
    };
    let consumer = get(&mut *store, consumer_name)?;
    // The consumer's params, in SOURCE ORDER — each is either a CLOSURE the host threads a produced handle
    // into (`Type::Own`/`Type::Borrow` — a resource) or a SCALAR taken from the arg strings. A closure param
    // may sit anywhere and there may be several; each gets its OWN fresh handle from the PRODUCER whose
    // RESULT resource type MATCHES that param (a distinct-sig round trip has several producers, one per
    // resource type — the first non-consumer func with a matching own<t> result).
    let cons_params: Vec<Type> = consumer
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    // The producer func matching a given resource type — a func (≠ the consumer) whose sole result is
    // `own<rt>`/`borrow<rt>`. Returns (func, its param types).
    let find_producer = |store: &mut Store<()>,
                         want: &wasmtime::component::ResourceType|
     -> Result<(wasmtime::component::Func, Vec<Type>)> {
        for name in iface_funcs {
            if name == consumer_name {
                continue;
            }
            let f = get(&mut *store, name)?;
            let matches_res = matches!(
                f.results(&*store).first(),
                Some(Type::Own(rt)) | Some(Type::Borrow(rt)) if rt == want
            );
            if matches_res {
                let params = f.params(&*store).iter().map(|(_, t)| t.clone()).collect();
                return Ok((f, params));
            }
        }
        Err(anyhow!(
            "round-trip closure: no producer mints the resource `{consumer_name}` expects"
        ))
    };
    // The corpus supplies the producer args for EACH closure param (in param order), then the consumer's
    // scalar args. Walk the consumer params: a closure param consumes its producer's arity from the front;
    // scalars come after all producer args.
    let n_closure_params = cons_params
        .iter()
        .filter(|t| matches!(t, Type::Own(_) | Type::Borrow(_)))
        .count();
    // Compute total producer-arg count (sum over each closure param's matching producer arity).
    let mut prod_specs: Vec<(wasmtime::component::Func, Vec<Type>)> = Vec::new();
    for t in &cons_params {
        if let Type::Own(rt) | Type::Borrow(rt) = t {
            prod_specs.push(find_producer(&mut *store, rt)?);
        }
    }
    let n_prod_args_total: usize = prod_specs.iter().map(|(_, p)| p.len()).sum();
    if arg_strs.len() < n_prod_args_total {
        return Err(anyhow!(
            "round-trip closure: producing {n_closure_params} closure(s) needs {n_prod_args_total} \
             producer argument(s) but only {} supplied",
            arg_strs.len()
        ));
    }
    // Produce one handle per closure param, each from the next slice of producer args.
    let mut handles: Vec<Val> = Vec::new();
    let mut arg_off = 0usize;
    for (producer, prod_params) in &prod_specs {
        let prod_args = coerce_args(&arg_strs[arg_off..arg_off + prod_params.len()], prod_params)?;
        arg_off += prod_params.len();
        let mut handle = [Val::Bool(false)];
        if let Err(e) = producer.call(&mut *store, &prod_args, &mut handle) {
            return Ok(Outcome::Trap(trap_message(&e)));
        }
        let _ = producer.post_return(&mut *store);
        handles.push(handle[0].clone());
    }
    // Build the consumer's args IN ORDER: a closure param → the next produced handle; a scalar → the next
    // scalar arg string.
    let scalar_strs = &arg_strs[n_prod_args_total..];
    let mut cons_args: Vec<Val> = Vec::new();
    let mut next_handle = 0usize;
    let mut next_scalar = 0usize;
    for t in &cons_params {
        if matches!(t, Type::Own(_) | Type::Borrow(_)) {
            cons_args.push(handles[next_handle].clone());
            next_handle += 1;
        } else {
            let s = scalar_strs.get(next_scalar).ok_or_else(|| {
                anyhow!(
                    "round-trip closure: consumer `{consumer_name}` needs more scalar arguments"
                )
            })?;
            cons_args.push(coerce_one(s, t)?);
            next_scalar += 1;
        }
    }
    let mut out = [Val::Bool(false)];
    match consumer.call(&mut *store, &cons_args, &mut out) {
        Ok(()) => {
            let _ = consumer.post_return(&mut *store);
            Ok(Outcome::Value(render_closure_call_result(out.first())))
        }
        Err(e) => Ok(Outcome::Trap(trap_message(&e))),
    }
}

/// Run a CLOSURE-resource program: reach `make`/`call` inside the `cadenza:closure/exports` instance,
/// call `make(make-args…)` → the closure resource handle, then `call(handle, call-args…)` → the closure's
/// result, rendered. The host acts as the closure's custodian: it holds the opaque handle and invokes the
/// guest's `call` method (which dispatches the closure via the guest's own `call_indirect`). `own<t>`
/// consumes the handle, so this is one `make`+`call` per case.
///
/// The caller supplies ONE flat arg list (`(call name a b c …)`); it is SPLIT by `make`'s declared arity —
/// the first N go to `make` (the EXPORT's parameters, e.g. `adder`'s `k`), the rest to `call` (the
/// CLOSURE's own arguments, e.g. `x`). A nullary export (N=0) sends all args to `call`. So
/// `(call adder (: 10 Int64) (: 5 Int64))` → `make(10)` then `call(5)` = 15.
///
/// TWO-CALL-ON-ONE-HANDLE (`second_call = Some(args2)`, from a corpus `(then …)` clause): a `borrow<t>`
/// closure does NOT consume its handle, so it is REPEATABLE — after `make` + the first `call`, the SAME
/// handle serves a SECOND `call(handle, args2…)`, and the two results render as a tuple `(tuple r1 r2)`
/// (see [`render_two_call_result`]). This pins that a borrowed closure handle stays live across calls (an
/// `own<t>` closure would trap "unknown handle index" on the second call). Applies to the bare/multi-export
/// `call` and the distinct-signature `call-g<n>`; the round-trip path (no `call`/`call-g`) ignores it.
///
/// DROP (`drop_handle`, from a corpus `(drop)` clause): `call` BORROWS the handle, so the minted closure
/// cell stays live after the call — a `--report-live-objects` run reports the leak of 1. When `drop_handle`
/// is set, the host RESOURCE-DROPS the handle after the call(s), firing its `t-dtor` to reclaim the cell, so
/// a `(live-objects 0)` case pins release. Applies to the bare/multi-export path.
// The closure driver needs the full wasmtime run context (engine/component/store/instance) plus the
// three drive knobs (export, args, and the second-call/drop options), so it genuinely takes 8 arguments.
#[allow(clippy::too_many_arguments)]
fn run_closure_resource(
    engine: &Engine,
    component: &Component,
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    export: Option<&str>,
    arg_strs: &[String],
    second_call: Option<&[String]>,
    drop_handle: bool,
    // Unused here — a closure's member is the fixed `call`; `call_member` only steers the value-resource
    // ESCAPE path (`run_resource_escape`). Threaded uniformly so `run_export` passes one arg set to both.
    _call_member: Option<&str>,
) -> Result<Outcome> {
    let iface = instance
        .get_export_index(&mut *store, None, CLOSURE_INTERFACE)
        .ok_or_else(|| anyhow!("closure escape: no `{CLOSURE_INTERFACE}` instance export"))?;
    let iface_funcs = closure_interface_funcs(engine, component);
    // ROUND-TRIP (C-HOST-4): producer + consumer exports, NO `call` method AND NO per-signature `call-g<n>`
    // (a distinct-sig program also lacks a bare `call` but has `call-g0` — handled below). The corpus
    // `(call <consumer> args…)` names the consumer; the sole PRODUCER (the other func) mints the closure.
    let has_call_g = iface_funcs.iter().any(|f| f == "call-g0");
    if !iface_funcs.iter().any(|f| f == "call") && !has_call_g {
        let consumer = export.ok_or_else(|| {
            anyhow!("round-trip closure: no --call given (name the CONSUMER export)")
        })?;
        // The public export name is KEBAB (the compiler normalized it at emit); a caller names the
        // consumer by its SOURCE identifier (`appA`, `my_func`). Resolve through the SAME rule so both
        // sides agree — `iface_funcs` are the actual (kebab) export names, so the comparison inside
        // `run_roundtrip_closure` (a func ≠ the consumer is a producer) must see the kebab consumer name.
        let consumer = cadenza_syntax::extern_name::kebab_extern_name(consumer);
        return run_roundtrip_closure(
            &mut *store,
            instance,
            &iface,
            &consumer,
            &iface_funcs,
            arg_strs,
        );
    }
    // DISTINCT-SIGNATURE multi-export: no bare `call`, but per-signature `call-g<n>` functions (each bound
    // to its own resource type). The corpus `(call <name> …)` names a closure export → `make-<name>`; the
    // matching call is the `call-g<n>` whose `self` param resource type equals `make-<name>`'s RESULT
    // resource type.
    if has_call_g {
        let name = export.ok_or_else(|| {
            anyhow!("distinct-sig closure: no --call given (name a closure export)")
        })?;
        // Public export names are KEBAB; a caller names the closure by its source identifier. Normalize
        // `make-<src>` the same way emit did so the lookup matches (`make-mkA` → `make-mk-a`).
        let make_name = cadenza_syntax::extern_name::kebab_extern_name(&format!("make-{name}"));
        let make_idx = instance
            .get_export_index(&mut *store, Some(&iface), &make_name)
            .ok_or_else(|| anyhow!("distinct-sig closure: no `{make_name}`"))?;
        let make = instance
            .get_func(&mut *store, make_idx)
            .ok_or_else(|| anyhow!("distinct-sig closure: `{make_name}` is not a function"))?;
        // `make`'s result resource type — pair it with the `call-g<n>` whose first param is that same type.
        let make_result = make.results(&*store).first().cloned();
        let want_res = match &make_result {
            Some(Type::Own(rt)) | Some(Type::Borrow(rt)) => *rt,
            other => {
                return Err(anyhow!(
                    "distinct-sig closure: `{make_name}` does not return a resource ({other:?})"
                ));
            }
        };
        // Find the matching call among `call-g<n>` funcs.
        let call_name = iface_funcs
            .iter()
            .filter(|f| f.starts_with("call-g"))
            .find(|cn| {
                let Some(idx) = instance.get_export_index(&mut *store, Some(&iface), cn) else {
                    return false;
                };
                let Some(cf) = instance.get_func(&mut *store, idx) else {
                    return false;
                };
                matches!(cf.params(&*store).first().map(|(_, t)| t.clone()),
                    Some(Type::Own(rt)) | Some(Type::Borrow(rt)) if rt == want_res)
            })
            .cloned()
            .ok_or_else(|| {
                anyhow!("distinct-sig closure: no `call-g<n>` matches `{make_name}`'s resource")
            })?;
        let call_idx = instance
            .get_export_index(&mut *store, Some(&iface), &call_name)
            .ok_or_else(|| anyhow!("distinct-sig closure: no `{call_name}`"))?;
        let call = instance
            .get_func(&mut *store, call_idx)
            .ok_or_else(|| anyhow!("distinct-sig closure: `{call_name}` is not a function"))?;
        // Split args by make's arity (as the multi-export path does).
        let make_param_types: Vec<Type> = make
            .params(&*store)
            .iter()
            .map(|(_, t)| t.clone())
            .collect();
        let n_make = make_param_types.len();
        if arg_strs.len() < n_make {
            return Err(anyhow!(
                "distinct-sig closure: `{make_name}` needs {n_make} arg(s)"
            ));
        }
        let make_args = coerce_args(&arg_strs[..n_make], &make_param_types)?;
        let mut handle = [Val::Bool(false)];
        if let Err(e) = make.call(&mut *store, &make_args, &mut handle) {
            return Ok(Outcome::Trap(trap_message(&e)));
        }
        let _ = make.post_return(&mut *store);
        let param_types: Vec<Type> = call
            .params(&*store)
            .iter()
            .map(|(_, t)| t.clone())
            .collect();
        let arg_types = param_types.get(1..).unwrap_or(&[]);
        let coerced = coerce_args(&arg_strs[n_make..], arg_types)?;
        let mut call_args = vec![handle[0].clone()];
        call_args.extend(coerced);
        let mut out = [Val::Bool(false)];
        if let Err(e) = call.call(&mut *store, &call_args, &mut out) {
            return Ok(Outcome::Trap(trap_message(&e)));
        }
        let _ = call.post_return(&mut *store);
        // A `(then …)` continuation: call `call-g<n>` a SECOND time on the SAME handle (repeatable under
        // `borrow<t>`), rendering the pair as a tuple.
        if let Some(args2) = second_call {
            let coerced2 = coerce_args(args2, arg_types)?;
            let mut call_args2 = vec![handle[0].clone()];
            call_args2.extend(coerced2);
            let mut out2 = [Val::Bool(false)];
            return match call.call(&mut *store, &call_args2, &mut out2) {
                Ok(()) => {
                    let _ = call.post_return(&mut *store);
                    Ok(Outcome::Value(render_two_call_result(
                        out.first(),
                        out2.first(),
                    )))
                }
                Err(e) => Ok(Outcome::Trap(trap_message(&e))),
            };
        }
        return Ok(Outcome::Value(render_closure_call_result(out.first())));
    }
    // The make function to call: a single-export program publishes a bare `make`; a MULTI-EXPORT program
    // publishes `make-<name>` per closure export, and the corpus `(call <name> …)` picks which. Try
    // `make-<export>` first (multi), then the bare `make` (single) — so a single-export case with a `--call
    // main` still resolves `make`.
    // Public export names are KEBAB; normalize `make-<src>` the same way emit did (`make-mkAdder` →
    // `make-mk-adder`) so a multi-export lookup by source name matches.
    let make_name = match export {
        Some(name)
            if {
                let mk = cadenza_syntax::extern_name::kebab_extern_name(&format!("make-{name}"));
                instance
                    .get_export_index(&mut *store, Some(&iface), &mk)
                    .is_some()
            } =>
        {
            cadenza_syntax::extern_name::kebab_extern_name(&format!("make-{name}"))
        }
        _ => "make".to_string(),
    };
    let make_idx = instance
        .get_export_index(&mut *store, Some(&iface), &make_name)
        .ok_or_else(|| anyhow!("closure escape: `{CLOSURE_INTERFACE}` exports no `{make_name}`"))?;
    let call_idx = instance
        .get_export_index(&mut *store, Some(&iface), "call")
        .ok_or_else(|| anyhow!("closure escape: `{CLOSURE_INTERFACE}` exports no `call`"))?;
    let make = instance
        .get_func(&mut *store, make_idx)
        .ok_or_else(|| anyhow!("closure escape: `make` is not a function"))?;
    let call = instance
        .get_func(&mut *store, call_idx)
        .ok_or_else(|| anyhow!("closure escape: `call` is not a function"))?;

    // SPLIT the flat arg list by `make`'s arity: the first `make.params().len()` go to `make` (the export
    // params), the rest to `call` (after its leading `self`).
    let make_param_types: Vec<Type> = make
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let n_make = make_param_types.len();
    if arg_strs.len() < n_make {
        return Err(anyhow!(
            "closure escape: `make` needs {n_make} argument(s) but only {} supplied",
            arg_strs.len()
        ));
    }
    let make_args = coerce_args(&arg_strs[..n_make], &make_param_types)?;
    let mut handle = [Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &make_args, &mut handle) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = make.post_return(&mut *store);
    // `call`'s params are `(self, args…)`; coerce the REMAINING arg strings to the DECLARED arg types
    // (skipping the leading `self` handle param).
    let param_types: Vec<Type> = call
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let arg_types = param_types.get(1..).unwrap_or(&[]);
    let coerced = coerce_args(&arg_strs[n_make..], arg_types)?;
    let mut call_args = vec![handle[0].clone()];
    call_args.extend(coerced);
    let mut out = [Val::Bool(false)];
    if let Err(e) = call.call(&mut *store, &call_args, &mut out) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = call.post_return(&mut *store);
    // A `(then …)` continuation: call `call` a SECOND time on the SAME handle (a `borrow<t>` closure keeps
    // it live across calls — an `own<t>` one would trap "unknown handle index" here), rendering the pair
    // as a tuple. Covers both the bare single-export `make`/`call` and the multi-export `make-<name>`/`call`.
    if let Some(args2) = second_call {
        let coerced2 = coerce_args(args2, arg_types)?;
        let mut call_args2 = vec![handle[0].clone()];
        call_args2.extend(coerced2);
        let mut out2 = [Val::Bool(false)];
        return match call.call(&mut *store, &call_args2, &mut out2) {
            Ok(()) => {
                let _ = call.post_return(&mut *store);
                let rendered = render_two_call_result(out.first(), out2.first());
                drop_handle_if(&mut *store, &handle[0], drop_handle)?;
                Ok(Outcome::Value(rendered))
            }
            Err(e) => Ok(Outcome::Trap(trap_message(&e))),
        };
    }
    let rendered = render_closure_call_result(out.first());
    // A `(drop)` clause: resource-drop the borrowed handle now (its `t-dtor` reclaims the cell) so a
    // `(live-objects 0)` case reads a released heap. No-op unless `drop_handle`.
    drop_handle_if(&mut *store, &handle[0], drop_handle)?;
    Ok(Outcome::Value(rendered))
}

/// Resource-drop a closure/resource handle `Val` when `drop` is set (the `(drop)` clause), firing its
/// `t-dtor` so the guest cell is reclaimed before the run reads the heap balance. A no-op when `drop` is
/// false or the value is not a resource handle.
fn drop_handle_if(store: &mut Store<()>, handle: &Val, drop: bool) -> Result<()> {
    if drop && let Val::Resource(r) = handle.clone() {
        r.resource_drop(&mut *store)
            .map_err(|e| anyhow!("drop closure handle: {e}"))?;
    }
    Ok(())
}

/// Run a resource-escape program: reach `make`/`encode` inside the `cadenza:run/run` instance, call
/// `make()` → a resource handle, `encode(handle)` → the canonical binary value form as `list<u8>`,
/// then DECODE those bytes to `Arenas` and print `(: value type)`. The type travels WITH the value (the
/// encoded s-expression is `(: <value> <type>)`), so the host spells no type name — it decodes and
/// prints. Mirrors the compiler's `constant_value_form`/resource-envelope emission
/// ([[rcdzc-r1-resource-encode-linking-findings]]).
#[allow(clippy::too_many_arguments)] // the escape driver needs the run context plus the make/member/repeat/drop knobs
fn run_resource_escape(
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    args: &[String],
    // A `(drop)` clause: `encode`/a value-resource member BORROWS the handle (reads without consuming), so
    // the escaped resource's cell stays live after. When set, resource-drop the handle after the member
    // call(s) so a `(live-objects 0)` case pins that the escaped resource is reclaimed. No-op by default.
    drop_handle: bool,
    // A `(call-method <member>)` clause: reach this NAMED member on the run-instance instead of the default
    // `encode` (e.g. a `Bytes` value's `len`/`is-empty`/`to-bytes`). `None` = the historical encode escape.
    call_member: Option<&str>,
    // A `(then …)` continuation: call the member a SECOND time on the SAME handle (a borrow method is
    // repeatable), rendering the pair as a tuple. Only meaningful with `call_member`.
    second_call: Option<&[String]>,
) -> Result<Outcome> {
    let iface = instance
        .get_export_index(&mut *store, None, RUN_INTERFACE)
        .ok_or_else(|| anyhow!("resource escape: no `{RUN_INTERFACE}` instance export"))?;
    let make_idx = instance
        .get_export_index(&mut *store, Some(&iface), "make")
        .ok_or_else(|| anyhow!("resource escape: `{RUN_INTERFACE}` exports no `make`"))?;
    // The member to reach on the produced resource: the named one (a `(call-method …)` case) or the default
    // `encode` (the historical value-escape). Both are instance members whose first param is the handle.
    let member_name = call_member.unwrap_or("encode");
    let member_idx = instance
        .get_export_index(&mut *store, Some(&iface), member_name)
        .ok_or_else(|| {
            anyhow!("resource escape: `{RUN_INTERFACE}` exports no member `{member_name}`")
        })?;
    let make = instance
        .get_func(&mut *store, make_idx)
        .ok_or_else(|| anyhow!("resource escape: `make` is not a function"))?;
    let member = instance
        .get_func(&mut *store, member_idx)
        .ok_or_else(|| anyhow!("resource escape: `{member_name}` is not a function"))?;

    // `make` forwards the escaping export's parameters: a NULLARY export takes no args (`make()`); a
    // PARAMETERIZED export (`(def (main (: a Int64)) …)`) takes the leading args. The REMAINING args (past
    // make's arity) are the member's own arguments (past its leading `self` handle). For the default
    // `encode` escape and a nullary member both slices are empty → byte-identical to before.
    let make_param_types: Vec<Type> = make
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let n_make = make_param_types.len().min(args.len());
    let make_args = coerce_args(&args[..n_make], &make_param_types)?;
    let mut handle = [Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &make_args, &mut handle) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = make.post_return(&mut *store);

    // The member's args (its params past the leading `self` handle), coerced from the args past make's.
    let member_param_types: Vec<Type> = member
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let member_arg_types = member_param_types.get(1..).unwrap_or(&[]);
    let coerced = coerce_args(&args[n_make..], member_arg_types)?;
    let mut call_args = vec![handle[0].clone()];
    call_args.extend(coerced);
    let mut out = [Val::Bool(false)];
    if let Err(e) = member.call(&mut *store, &call_args, &mut out) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = member.post_return(&mut *store);

    // A NAMED member's result renders directly (a scalar/bool via `render_val`, a value-form `list<u8>`
    // decoded) — the same disambiguation `encode`'s result uses. A `(then …)` repeats the member on the
    // SAME handle (a borrow method is repeatable) and renders the pair as a tuple.
    let rendered = if let Some(args2) = second_call {
        let coerced2 = coerce_args(args2, member_arg_types)?;
        let mut call_args2 = vec![handle[0].clone()];
        call_args2.extend(coerced2);
        let mut out2 = [Val::Bool(false)];
        if let Err(e) = member.call(&mut *store, &call_args2, &mut out2) {
            return Ok(Outcome::Trap(trap_message(&e)));
        }
        let _ = member.post_return(&mut *store);
        render_two_call_result(out.first(), out2.first())
    } else {
        render_closure_call_result(out.first())
    };
    // A `(drop)` clause: reclaim the escaped resource's cell after the member call(s) (which only borrowed
    // it), so a `(live-objects 0)` case reads a released heap.
    drop_handle_if(&mut *store, &handle[0], drop_handle)?;
    Ok(Outcome::Value(rendered))
}

/// Render a closure `call`'s result value. A scalar/String comes back directly; a `list<u8>` may be EITHER
/// a raw byte-rope result (a `Bytes`/`String` closure — render the bare byte sequence `(5 6)`) OR the
/// canonical VALUE FORM of a compound result (tuple/record/sum — decode + pretty-print `(: value T)`). The
/// two are disambiguated by TRYING to decode: `codec::decode` is total and refuses any bytes whose 8-byte
/// schema header it does not recognize, so a raw byte-rope (which lacks that header) declines and falls
/// through to the raw-list render — no ambiguity, no flag needed.
fn render_closure_call_result(v: Option<&Val>) -> String {
    match v {
        None => "unit".to_string(),
        Some(Val::String(s)) => s.clone(),
        Some(Val::List(items)) => {
            // Try the value-form decode first (a compound result); fall back to the raw byte-rope render.
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|e| match e {
                    Val::U8(b) => Some(*b),
                    _ => None,
                })
                .collect();
            if let Some(bytes) = bytes {
                match cadenza_syntax::codec::decode_detailed(&bytes) {
                    Ok(arenas) => {
                        return cadenza_syntax::sexpr::print(&arenas).trim().to_string();
                    }
                    // DIAGNOSTIC (value-encode<->codec skew hunt): a buffer carrying the `cdzast` header
                    // that DECLINES to decode means a compound VALUE doc the codec can't parse — we then
                    // fall through to the raw byte-rope render (the "renders as raw bytes" bug). `decode`
                    // is total, so surface WHY: the `DecodeError` class + a hex dump so the offending
                    // kind/offset is inspectable in a composed-runtime gate log. Gated on the header so a
                    // GENUINE byte-rope value (not a value doc — the intended fall-through) stays quiet.
                    Err(e) => {
                        if bytes.len() >= 8 && &bytes[..6] == b"cdzast" {
                            eprintln!(
                                "cdz-run: a cdzast value-encode doc was DECLINED by codec::decode \
                                 ({e:?}) — falling back to the raw byte render (value-encode<->codec \
                                 skew). buffer[{}] = {:02x?}",
                                bytes.len(),
                                bytes
                            );
                        }
                    }
                }
            }
            render_val(v.unwrap())
        }
        Some(other) => render_val(other),
    }
}

/// Render a two-call-on-one-handle result (the `(then …)` drive): the pair of `call` results as a
/// tuple value-form `(tuple <v1> <v2>)`. Each result is rendered by [`render_closure_call_result`] and
/// then UNWRAPPED to its bare value — a compound result comes back as the annotated `(: <value> <type>)`
/// form, so we strip the `(: … )` envelope to embed only `<value>` (a scalar renders bare already). The
/// corpus grades this against `(output (: (tuple <v1> <v2>) (Tuple T T)))`, whose `expected_value` is the
/// same `(tuple <v1> <v2>)`. Both calls share ONE handle (a `borrow<t>` closure keeps it live), so a
/// matching tuple proves repeatability + the two results' relationship.
fn render_two_call_result(out1: Option<&Val>, out2: Option<&Val>) -> String {
    let v1 = bare_value_form(&render_closure_call_result(out1));
    let v2 = bare_value_form(&render_closure_call_result(out2));
    format!("(tuple {v1} {v2})")
}

/// Strip an outer `(: <value> <type>)` value-form annotation to its bare `<value>` (the same balanced-token
/// extraction the gate's `expected_value` does), leaving a scalar / already-bare form unchanged. Used to
/// embed a call result inside a `(tuple …)` without a nested type annotation.
fn bare_value_form(rendered: &str) -> String {
    let s = rendered.trim();
    let Some(rest) = s.strip_prefix("(:") else {
        return s.to_string();
    };
    let rest = rest.trim();
    let bytes = rest.as_bytes();
    if bytes.first() == Some(&b'(') {
        let mut depth = 0i32;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return rest[..=i].to_string();
                    }
                }
                _ => {}
            }
        }
        rest.to_string()
    } else {
        match rest.find(char::is_whitespace) {
            Some(idx) => rest[..idx].to_string(),
            None => rest.trim_end_matches(')').to_string(),
        }
    }
}

/// Coerce each raw CLI argument string to the corresponding declared parameter type. The arity must
/// match; each scalar type parses from its natural text form. Compound param types are not yet
/// supported (no export takes them today) and are an explicit error rather than a silent guess.
fn coerce_args(raw: &[String], types: &[Type]) -> Result<Vec<Val>> {
    if raw.len() != types.len() {
        return Err(anyhow!(
            "argument count mismatch: the export takes {} argument(s), {} given",
            types.len(),
            raw.len()
        ));
    }
    raw.iter()
        .zip(types)
        .map(|(s, t)| coerce_one(s, t))
        .collect()
}

/// The CADENZA surface name for a boundary parameter type, for user-facing diagnostics. wasmtime's
/// `Type` Debug prints the component-model spelling (`S64`, `U8`, `Float64`), but the user wrote the
/// Cadenza type (`Int64`, `UInt8`, `Float64`) in their `(: p Int64)` annotation — an arg-coercion error
/// must name THAT, not the internal ABI name, so "cannot parse `hello` as Int64" points at the source.
/// Compound types fall back to the component spelling (no scalar CLI arg reaches them with a parse error).
fn cadenza_type_name(t: &Type) -> String {
    match t {
        Type::Bool => "Bool".into(),
        Type::S8 => "Int8".into(),
        Type::S16 => "Int16".into(),
        Type::S32 => "Int32".into(),
        Type::S64 => "Int64".into(),
        Type::U8 => "UInt8".into(),
        Type::U16 => "UInt16".into(),
        Type::U32 => "UInt32".into(),
        Type::U64 => "UInt64".into(),
        Type::Float32 => "Float32".into(),
        Type::Float64 => "Float64".into(),
        Type::Char => "Char".into(),
        Type::String => "String".into(),
        other => format!("{other:?}"),
    }
}

fn coerce_one(s: &str, t: &Type) -> Result<Val> {
    let parse = |ok: Option<Val>| {
        ok.ok_or_else(|| anyhow!("cannot parse `{s}` as {}", cadenza_type_name(t)))
    };
    Ok(match t {
        Type::Bool => parse(s.parse::<bool>().ok().map(Val::Bool))?,
        Type::S8 => parse(s.parse::<i8>().ok().map(Val::S8))?,
        Type::U8 => parse(s.parse::<u8>().ok().map(Val::U8))?,
        Type::S16 => parse(s.parse::<i16>().ok().map(Val::S16))?,
        Type::U16 => parse(s.parse::<u16>().ok().map(Val::U16))?,
        Type::S32 => parse(s.parse::<i32>().ok().map(Val::S32))?,
        Type::U32 => parse(s.parse::<u32>().ok().map(Val::U32))?,
        Type::S64 => parse(s.parse::<i64>().ok().map(Val::S64))?,
        Type::U64 => parse(s.parse::<u64>().ok().map(Val::U64))?,
        Type::Float32 => parse(s.parse::<f32>().ok().map(Val::Float32))?,
        Type::Float64 => parse(s.parse::<f64>().ok().map(Val::Float64))?,
        Type::Char => parse(
            s.chars()
                .next()
                .filter(|_| s.chars().count() == 1)
                .map(Val::Char),
        )?,
        // The corpus writes a string argument as a QUOTED literal (`(: "abc" String)` → `s` = `"abc"`,
        // quotes included). Parse the literal — strip the delimiters, apply the closed escape set, and
        // NFC-normalize — exactly as the front-end reads a source string, so the marshalled value is the
        // string's CONTENT (`abc`), not the 5-char token `"abc"`. An unquoted `s` (a bare CLI arg) is taken
        // verbatim. (The Rust backend path needs no equivalent: it emits the corpus literal as Rust source,
        // where `"abc"` already denotes the 3-char string.)
        Type::String => {
            let val = match s
                .strip_prefix('"')
                .and_then(|inner| inner.strip_suffix('"'))
            {
                Some(inner) => cadenza_syntax::literal::unescape_string(inner)
                    .map_err(|c| anyhow!("argument `{s}`: unknown string escape `\\{c}`"))?,
                None => s.to_string(),
            };
            Val::String(val)
        }
        // A FIXED-SHAPE tuple argument (the direct-call compound-arg path): the host supplies it as a
        // component `tuple<…>` value, which the canonical ABI flattens into the guest's core params. The
        // corpus writes it as `(tuple <f0> <f1> …)` (an optional leading `tuple` head, else a bare
        // `(<f0> <f1> …)`); parse the paren-wrapped, whitespace-separated fields and coerce each against
        // the tuple's element types. Fields must be scalars (this increment supports a fixed-shape SCALAR
        // tuple; a nested compound field would recurse, a later widening).
        Type::Tuple(tt) => {
            let elem_types: Vec<Type> = tt.types().collect();
            let mut fields = parse_tuple_fields(s).ok_or_else(|| {
                anyhow!("argument `{s}`: expected a tuple literal like `(tuple 3 4)` or `(3 4)`")
            })?;
            if fields.len() != elem_types.len() {
                return Err(anyhow!(
                    "argument `{s}`: tuple has {} field(s), the parameter type expects {}",
                    fields.len(),
                    elem_types.len()
                ));
            }
            // A RECORD closure argument erases to a `tuple<…>` whose fields are laid in canonical SORTED-name
            // order (`tuple_field_abi` / `Core::Record`: a `BTreeMap` over field names). The corpus writes the
            // record value `(record (z 100) (a 3))` in SOURCE order, so when the VALUE is record-headed, sort
            // its `(name value)` fields by name to match the boundary tuple's positions, then unwrap each to its
            // value before coercing. This name-handling is gated on the `record` head: a GENUINE positional
            // tuple — `(a b)` / `(tuple a b)`, e.g. a `tuple<list<u8>, list<u8>>` written `((list 7) (list 9))`
            // — is coerced POSITIONALLY, never name-unwrapped (its `(list 7)` element is a value, not a
            // `name=value` field; the legacy `(name value)` heuristic would otherwise mistake the `list` head
            // for a field name and strip it to a bare `7`, which then fails to coerce as a list).
            let is_record_erased = is_record_headed(s);
            if is_record_erased && fields.iter().all(|f| named_field(f).is_some()) {
                fields.sort_by(|a, b| named_field(a).unwrap().0.cmp(&named_field(b).unwrap().0));
            }
            let vals: Result<Vec<Val>> = fields
                .iter()
                .zip(&elem_types)
                .map(|(f, ft)| {
                    let text = if is_record_erased {
                        unwrap_named_field(f)
                    } else {
                        f.clone()
                    };
                    coerce_one(&text, ft)
                })
                .collect();
            Val::Tuple(vals?)
        }
        // A FIXED-PAYLOAD `(Option scalar)` argument (the direct-call SUM-arg path): the host supplies it as a
        // component `option<T>` value, which the canonical ABI flattens into the guest's `(disc, payload)` core
        // params. The corpus writes `(Some <value>)` for the present case and `None` (a bare atom) for absent;
        // coerce the payload against `option.ty()`.
        Type::Option(ot) => {
            let t = s.trim();
            // The parenthesized body, if any: `(None unit)` → "None unit", `(Some 42)` → "Some 42".
            let inner_paren = t
                .strip_prefix('(')
                .and_then(|x| x.strip_suffix(')'))
                .map(str::trim);
            // NONE accepts the canonical rendered form `(None unit)` (see `render_val`) AND the bare atom
            // `None` — the renderer emits `(None unit)`, so coercion must round-trip it (the arg-marshal twin
            // of the Some path). The head token is `None`.
            let is_none = t == "None"
                || inner_paren.is_some_and(|x| x.split_whitespace().next() == Some("None"));
            if is_none {
                Val::Option(None)
            } else {
                // `(Some <value>)` — strip the parens + the `Some` head, coerce the remaining value.
                let inner = inner_paren
                    .and_then(|x| x.strip_prefix("Some"))
                    .map(str::trim)
                    .ok_or_else(|| {
                        anyhow!(
                            "argument `{s}`: expected an option literal `(Some <value>)`, `(None unit)`, or `None`"
                        )
                    })?;
                Val::Option(Some(Box::new(coerce_one(inner, &ot.ty())?)))
            }
        }
        // A `(Result ok-scalar err-scalar)` argument (the direct-call SUM-arg path): crosses as a component
        // `result<ok,err>` the ABI flattens to `(disc, payload)`. The corpus writes `(Ok <value>)` / `(Err
        // <value>)`; coerce the payload against `result.ok()` / `result.err()`.
        Type::Result(rt) => {
            let t = s.trim();
            let body = t
                .strip_prefix('(')
                .and_then(|x| x.strip_suffix(')'))
                .map(str::trim)
                .ok_or_else(|| {
                    anyhow!("argument `{s}`: expected a result literal `(Ok <value>)` or `(Err <value>)`")
                })?;
            if let Some(v) = body.strip_prefix("Ok").map(str::trim) {
                let ty = rt
                    .ok()
                    .ok_or_else(|| anyhow!("argument `{s}`: result has no ok payload type"))?;
                Val::Result(Ok(Some(Box::new(coerce_one(v, &ty)?))))
            } else if let Some(v) = body.strip_prefix("Err").map(str::trim) {
                let ty = rt
                    .err()
                    .ok_or_else(|| anyhow!("argument `{s}`: result has no err payload type"))?;
                Val::Result(Err(Some(Box::new(coerce_one(v, &ty)?))))
            } else {
                return Err(anyhow!(
                    "argument `{s}`: expected a result literal `(Ok <value>)` or `(Err <value>)`"
                ));
            }
        }
        // A RECORD argument (a world-imposed / interface-nested export's record param): the host supplies it
        // as a component `record{…}` value. The corpus writes `(record (= name value) …)` (canonical field
        // groups); parse the groups, match each DECLARED field by name, and coerce its value against the
        // field's type (a nested record/option/list/tuple field recurses through `coerce_one`). Built in the
        // record type's DECLARED field order so it matches the component-model layout regardless of source order.
        Type::Record(rt) => {
            let mut parts = parse_tuple_fields(s).ok_or_else(|| {
                anyhow!("argument `{s}`: expected a record literal like `(record (= x 1))`")
            })?;
            if parts.first().map(String::as_str) == Some("record") {
                parts.remove(0);
            }
            let mut given: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for f in &parts {
                let (n, v) = named_field(f).ok_or_else(|| {
                    anyhow!("argument `{s}`: record field `{f}` is not a `(= name value)` group")
                })?;
                given.insert(n, v);
            }
            let mut fields: Vec<(String, Val)> = Vec::new();
            for field in rt.fields() {
                let vtext = given.get(field.name).ok_or_else(|| {
                    anyhow!("argument `{s}`: record is missing field `{}`", field.name)
                })?;
                fields.push((field.name.to_string(), coerce_one(vtext, &field.ty)?));
            }
            Val::Record(fields)
        }
        // A LIST argument: the corpus writes `(list e0 e1 …)`; coerce each element against the element type
        // (elements may themselves be compound and recurse).
        Type::List(lt) => {
            let mut parts = parse_tuple_fields(s).ok_or_else(|| {
                anyhow!("argument `{s}`: expected a list literal like `(list 1 2)`")
            })?;
            if parts.first().map(String::as_str) == Some("list") {
                parts.remove(0);
            }
            let et = lt.ty();
            let vals: Result<Vec<Val>> = parts.iter().map(|e| coerce_one(e, &et)).collect();
            Val::List(vals?)
        }
        // An ENUM argument (a payload-less sum): the corpus writes the `render_val` form `(<case> unit)` (a
        // bare `<case>` is also accepted). Extract the case name and validate it against the enum's declared
        // cases; `Val::Enum` carries just the name and the boundary lifts it to the discriminant. This is the
        // read side of the reducer response's `error` enum (a `result<_, enum>` param arm).
        Type::Enum(et) => {
            let case = parse_tuple_fields(s)
                .and_then(|p| p.into_iter().next())
                .unwrap_or_else(|| s.trim().to_string());
            if !et.names().any(|n| n == case) {
                return Err(anyhow!(
                    "argument `{s}`: `{case}` is not a case of the enum (declared cases: {})",
                    et.names().collect::<Vec<_>>().join(", ")
                ));
            }
            Val::Enum(case)
        }
        // A VARIANT argument/response (a sum with scalar-payload cases): the corpus writes the `render_val`
        // form `(<case> <payload>)` for a payload case, or `(<case> unit)` / bare `<case>` for a nullary case.
        // Extract the case name (kebab, the component spelling), validate it, and coerce the payload against
        // the case's declared type. The read side of a variant-with-payload host RESULT response (the twin of
        // the `Type::Enum` arm, with a payload).
        Type::Variant(vt) => {
            let parts = parse_tuple_fields(s).unwrap_or_else(|| vec![s.trim().to_string()]);
            let case = parts
                .first()
                .cloned()
                .unwrap_or_else(|| s.trim().to_string());
            let cd = vt.cases().find(|c| c.name == case).ok_or_else(|| {
                anyhow!(
                    "argument `{s}`: `{case}` is not a case of the variant (declared cases: {})",
                    vt.cases()
                        .map(|c| c.name.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            let payload = match cd.ty {
                Some(pty) => {
                    let pv = parts
                        .get(1)
                        .filter(|v| v.as_str() != "unit")
                        .ok_or_else(|| {
                            anyhow!("argument `{s}`: variant case `{case}` needs a payload value")
                        })?;
                    Some(Box::new(coerce_one(pv, &pty)?))
                }
                None => None,
            };
            Val::Variant(case, payload)
        }
        other => {
            return Err(anyhow!(
                "argument `{s}`: compound parameter type {other:?} is not supported by cdz-run yet"
            ));
        }
    })
}

/// If `field` is a RECORD-field group — the canonical `(= name value)` ascription triple (DESIGN-record-
/// type-syntax Phase B) or a legacy `(name value)` pair — whose NAME element is a bare field name (an
/// identifier, not a number/bool) — return `(name, value)`. A record closure argument erases to a
/// component `tuple<…>` at the boundary, so the corpus's record VALUE `(record (= x 10) (= y 3))` presents
/// each field this way; the driver reorders + unwraps them. Returns `None` for a plain scalar or a
/// positional tuple field (so those stay untouched).
/// Whether a value literal is RECORD-headed — either the paren spelling `(record …)` or the native ctor
/// `#record(…)` (what M3 input-nativization writes). Used by the tuple-erased coercion path to decide
/// whether to name-sort + unwrap the `(= name value)` field groups (a record value crossing as a `tuple<…>`)
/// vs coerce positionally (a genuine tuple). Recognizing ONLY the paren form let a nativized `#record(…)`
/// arg skip the unwrap and mis-coerce its `(= name value)` group as a scalar (the #5718 follow-on gap).
fn is_record_headed(s: &str) -> bool {
    let st = s.trim();
    st.strip_prefix('(')
        .map(str::trim_start)
        .and_then(|r| r.split_whitespace().next())
        == Some("record")
        || st.starts_with("#record(")
}

fn named_field(field: &str) -> Option<(String, String)> {
    let parts = parse_tuple_fields(field)?;
    // Canonical triple `(= name value)` → drop the `=` head; else a legacy pair `(name value)`.
    let parts: &[String] = match parts.split_first() {
        Some((head, rest)) if head == "=" && rest.len() == 2 => rest,
        _ => &parts,
    };
    if parts.len() == 2
        && !parts[0].is_empty()
        && parts[0]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && parts[0].parse::<i128>().is_err()
        && parts[0].parse::<f64>().is_err()
        && parts[0] != "true"
        && parts[0] != "false"
    {
        Some((parts[0].clone(), parts[1].clone()))
    } else {
        None
    }
}

/// Unwrap a record-field group `(name value)` to its VALUE (`(x 10)` → `10`); leave a plain scalar or a
/// positional/nested-compound field unchanged so nested tuples still coerce recursively.
fn unwrap_named_field(field: &str) -> String {
    named_field(field)
        .map(|(_, v)| v)
        .unwrap_or_else(|| field.to_string())
}

/// Parse a corpus tuple argument literal into its field texts. Accepts `(tuple f0 f1 …)` (the canonical
/// value-form spelling the corpus renders) or a bare `(f0 f1 …)`; the outer parens are required. Fields are
/// split on whitespace at the TOP level (a nested `(…)` field stays one token so a nested compound can be
/// coerced recursively later). Returns `None` if `s` is not a paren-wrapped group. This is a minimal
/// scalar-field splitter — sufficient for a fixed-shape SCALAR tuple, where every field is a bare token.
fn parse_tuple_fields(s: &str) -> Option<Vec<String>> {
    // Accept the NATIVE ctor form `#list(…)` / `#tuple(…)` / `#record(…)` / `#map(…)` / `#set(…)` — what the
    // M3 corpus input-nativization writes — as equivalent to the paren form `(list …)`: strip the leading
    // `#` so `#head(` becomes `(head `, the EXACT inverse of the nativization. This is a spelling alias on the
    // OUTERMOST head only (nested #forms in the fields recurse through `coerce_one` → back here), so there is
    // ONE splitter and no second #-form value parser to drift. Everything below is the existing paren logic.
    let trimmed = s.trim();
    let normalized: String;
    let src: &str = match trimmed.strip_prefix('#') {
        Some(rest) => match rest.find('(') {
            Some(p)
                if !rest[..p].is_empty()
                    && rest[..p]
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_') =>
            {
                normalized = format!("({} {}", &rest[..p], &rest[p + 1..]);
                &normalized
            }
            _ => trimmed,
        },
        None => trimmed,
    };
    let inner = src.strip_prefix('(')?.strip_suffix(')')?.trim();
    // Split on whitespace, respecting nested parens (a nested `(…)` field is one token).
    let mut fields = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    fields.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        fields.push(cur);
    }
    // Drop an optional leading `tuple`/`record` head token (the canonical value-form spelling).
    if let Some(first) = fields.first()
        && (first == "tuple" || first == "record")
    {
        fields.remove(0);
    }
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bytes-second run-wiring: the byte-scanner walks a component's top-level sections + returns the
    /// `cdz-result-type` custom section's payload; `parse_result_types` + `lookup_result_ty` resolve the
    /// running export's Ty. A component with no such section → None (type-blind).
    #[test]
    fn scan_and_lookup_result_type_section() {
        use cadenza_syntax::ast::{Builder, IntValue, Leaf, Radix};
        use wasmtime::component::Val;
        // A minimal component-shaped blob: 8-byte preamble, then ONE custom section (id 0) named
        // `cdz-result-type` with the binary-AST result-types payload. `custom_section` framing mirrors emit.
        fn uleb(mut v: u32, out: &mut Vec<u8>) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
        }
        fn custom(name: &str, payload: &[u8]) -> Vec<u8> {
            let mut contents = Vec::new();
            uleb(name.len() as u32, &mut contents);
            contents.extend_from_slice(name.as_bytes());
            contents.extend_from_slice(payload);
            let mut sec = vec![0u8]; // id 0 = custom
            uleb(contents.len() as u32, &mut sec);
            sec.extend_from_slice(&contents);
            sec
        }
        // Build the seq-284 binary-AST payload via the shared codec: g : Bytes (leaf), f : (List (Int 64)).
        // Assert BEHAVIORALLY through the real consumer (render_val_typed): g disambiguates a list<u8> to
        // b"…"; f (List, not Bytes) renders #list. Decode correctness itself is covered by
        // cadenza_compile_abi's round-trip tests; here we prove the scan + parse + lookup + render wiring.
        let g_arena = {
            let mut b = Builder::new();
            let r = b.name("Bytes");
            b.finish(r)
        };
        let f_arena = {
            let mut b = Builder::new();
            let head = b.name("List");
            let ih = b.name("Int");
            let w = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(64),
                radix: Radix::Dec,
            });
            let elem = b.list(vec![ih, w]);
            let r = b.list(vec![head, elem]);
            b.finish(r)
        };
        let payload = cadenza_compile_abi::encode_result_types(&[
            ("g".to_string(), g_arena),
            ("f".to_string(), f_arena),
        ]);
        let mut comp = vec![0, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00]; // preamble
        comp.extend_from_slice(&custom("some-other", b"ignored"));
        comp.extend_from_slice(&custom("cdz-result-type", &payload));

        let scanned = scan_result_type_section(&comp).expect("finds cdz-result-type");
        assert_eq!(scanned, payload);
        let map = parse_result_types(Some(&scanned));
        let u8s = vec![Val::U8(1), Val::U8(2)];
        assert_eq!(
            crate::render::render_val_typed(
                &Val::List(u8s.clone()),
                lookup_result_ty(&map, Some("g")).expect("g present")
            ),
            format!("b\"{}\"", cadenza_syntax::literal::escape_bytes(&[1, 2]))
        );
        assert_eq!(
            crate::render::render_val_typed(
                &Val::List(u8s),
                lookup_result_ty(&map, Some("f")).expect("f present")
            ),
            "#list(1 2)"
        );
        assert!(lookup_result_ty(&map, Some("absent")).is_none());
        // No section → None (type-blind).
        let bare = vec![0, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];
        assert!(scan_result_type_section(&bare).is_none());
    }

    #[test]
    fn empty_bytes_is_invalid() {
        assert!(validate(&[]).is_err());
    }

    #[test]
    fn scale_from_load_is_one_at_or_below_full_utilization() {
        // The whole point of the clamp-to-1 floor: an idle or normally-loaded box gets the EXACT prior
        // wall-clock deadline (no stretch), so this change is a no-op except under real oversubscription.
        assert_eq!(scale_from_load(0.0, 8.0), 1, "idle box → no stretch");
        assert_eq!(scale_from_load(4.0, 8.0), 1, "half-loaded → no stretch");
        assert_eq!(scale_from_load(8.0, 8.0), 1, "exactly full → no stretch");
        // A hair over full utilization already grants one step of headroom (ceil), since even mild
        // oversubscription deschedules a correct run off-core long enough to risk a wall-clock false-trap.
        assert_eq!(scale_from_load(8.1, 8.0), 2, "just over full → ceil to 2");
    }

    #[test]
    fn scale_from_load_tracks_oversubscription_and_clamps() {
        // The oversubscription factor is loadavg/ncpu rounded up: ~how many runnable threads share a core,
        // i.e. how much longer than its CPU time a correct run takes in wall clock. This is the exact
        // stretch that keeps a load-starved trivial case (the pr-sync false-RED) from tripping the deadline.
        assert_eq!(scale_from_load(16.0, 8.0), 2, "2x oversubscribed");
        assert_eq!(scale_from_load(40.0, 8.0), 5, "5x oversubscribed");
        assert_eq!(
            scale_from_load(532.0, 64.0),
            9,
            "the reported peak (~532 on 64 cores)"
        );
        // Clamp: a pathological spike can't grant an unbounded budget — a genuine CPU-bound runaway loop
        // still traps within MAX_LOAD_SCALE× the base timeout (the safety net is stretched, never defeated).
        assert_eq!(
            scale_from_load(100_000.0, 1.0),
            MAX_LOAD_SCALE,
            "clamped at the ceiling"
        );
    }

    #[test]
    fn scale_from_load_fails_safe_on_degenerate_input() {
        // Any unreadable/garbage signal falls back to the unscaled deadline (factor 1) rather than a wild
        // stretch that would blunt the runaway-loop trap — fail SAFE toward the stricter prior behavior.
        assert_eq!(scale_from_load(f64::NAN, 8.0), 1, "NaN load → 1");
        assert_eq!(scale_from_load(f64::INFINITY, 8.0), 1, "inf load → 1");
        assert_eq!(scale_from_load(8.0, 0.0), 1, "zero cpus → 1");
        assert_eq!(scale_from_load(8.0, f64::NAN), 1, "NaN cpus → 1");
        assert_eq!(scale_from_load(-5.0, 8.0), 1, "negative load → 1");
    }

    #[test]
    fn run_core_module_traps_a_runaway_loop_at_the_epoch_deadline() {
        // The durable fleet-health safety net: a miscompiled runtime-looping program must TRAP at the
        // wall-clock deadline, not spin a core forever (which starved pr-sync + flooded the host earlier
        // this session). Assemble a module whose `main` is an infinite `(loop br 0)`, run it with a SHORT
        // timeout, and assert it comes back as a Trap in well under the untimed-forever case (seconds, not
        // never). Uses the epoch interruption armed in `engine()` + `new_store`.
        // SAFETY of the timing: CDZ_RUN_TIMEOUT_SECS=1 → the epoch ticker (100ms) fires the deadline within
        // ~1s; the worker-join below bounds the whole thing at 15s so a REGRESSION (no deadline → hang)
        // fails the test by our own wall-clock rather than hanging CI forever.
        // SAFETY: env is process-global; this test sets it for the short run. Other cdz-run tests don't run
        // a core module, so there's no cross-test interference on this var.
        unsafe {
            std::env::set_var("CDZ_RUN_TIMEOUT_SECS", "1");
        }
        // Run the (would-be-infinite) loop on a WORKER THREAD and join with a HARD wall-clock bound, so a
        // REGRESSION where the deadline never fires FAILS this test (and doesn't wedge the suite) instead of
        // spinning a core forever. A previous regression — `engine()` minting a fresh engine per call so the
        // ticker only advanced the first one's epoch — hung exactly here at 99% CPU and wedged pr-sync's
        // `cargo test --workspace` gate (no per-test timeout there); the earlier "assert elapsed < 15s AFTER
        // the call" could never fire because the call never returned. This join-with-timeout is the guard.
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            // FIRST do a harmless run — this arms the epoch-ticker (a `Once`) against whatever engine
            // `engine()` returns. THEN run the infinite loop. This ordering RELIABLY reproduces the
            // regression: when `engine()` minted a fresh engine per call, the ticker bound to THIS first
            // run's engine and the loop run below got a DIFFERENT, un-ticked engine whose deadline never
            // fired → hang. With the shared-engine fix both runs share one ticked engine, so the loop traps.
            // (Without this priming run, a per-call-engine regression could still pass by luck if the loop
            // happened to hit the first/armed engine — so the priming run is what makes this a true pin.)
            let ok_mod =
                wat::parse_str("(module (func (export \"main\") (result i64) (i64.const 1)))")
                    .expect("assemble the priming module");
            let _ = run_core_module(&ok_mod, "main");
            let wasm = wat::parse_str(
                "(module (func (export \"main\") (result i64) (loop br 0) (i64.const 0)))",
            )
            .expect("assemble the loop module");
            let outcome = run_core_module(&wasm, "main");
            let _ = tx.send(outcome);
        });
        let outcome = rx
            .recv_timeout(std::time::Duration::from_secs(15))
            .expect("the epoch deadline must TRAP the loop within 15s — a regression hangs here");
        let _ = worker.join();
        let outcome = outcome.expect("run returns (trap outcome), not Err");
        assert!(
            matches!(outcome, Outcome::Trap(_)),
            "a runaway loop must TRAP at the deadline, got {outcome:?}"
        );
        unsafe {
            std::env::remove_var("CDZ_RUN_TIMEOUT_SECS");
        }
    }

    #[test]
    fn run_core_module_returns_an_i64_mains_value() {
        // The `cdz run-emitted` HAPPY PATH: the compiler-ml emit backend produces a core `(module (func
        // (result i64)) (export "main"))` and run_core_module invokes it and returns the value. This pins
        // the ordinary success case — without it the ONLY coverage of this seam was the trap/epoch-deadline
        // test, so a regression that broke plain value-return (e.g. a wrong typed<> binding) would pass the
        // suite. A constant `main` needs no runtime import, so this is hermetic (no store).
        let wasm = wat::parse_str("(module (func (export \"main\") (result i64) (i64.const 42)))")
            .expect("assemble the i64 module");
        let outcome = run_core_module(&wasm, "main").expect("an i64 main runs, not Err");
        assert!(
            matches!(&outcome, Outcome::Value(v) if v == "42"),
            "an i64 main returns its value as text, got {outcome:?}"
        );
    }

    #[test]
    fn run_core_module_rejects_a_wrong_typed_main() {
        // A `main` whose result is NOT `() -> i64` (here `() -> f64`) is a real shape mismatch — a bad
        // artifact / emit bug — and must be an `Err` naming the expected signature, not a silent misread or
        // a panic. Pins the guard so a future emit change that ships the wrong signature is caught loudly.
        let wasm = wat::parse_str("(module (func (export \"main\") (result f64) (f64.const 1)))")
            .expect("assemble the f64 module");
        let err = run_core_module(&wasm, "main").expect_err("a non-i64 main is an Err");
        assert!(
            format!("{err}").contains("() -> i64"),
            "the mismatch names the expected signature: {err}"
        );
    }

    #[test]
    fn engine_fingerprint_is_stable_nonempty_and_versioned() {
        // The `.cwasm` cache filename fingerprint (`engine_fp`) must be deterministic for a fixed engine
        // config (else every run writes a fresh path and the cache never hits) and non-trivial (the OLD
        // `CARGO_PKG_VERSION_MAJOR` was perma-"0" — a constant that never distinguished wasmtime versions,
        // so cross-version artifacts thrashed one `wt0` path). Two fingerprints of the SAME shared engine
        // must be equal; the token must be the fixed-width hex we format into the name.
        let e = engine();
        let a = engine_fp(&e);
        let b = engine_fp(&e);
        assert_eq!(a, b, "fingerprint must be stable for a fixed engine config");
        assert_eq!(a.len(), 16, "rendered as fixed-width 16-hex ({a:?})");
        assert!(
            a.chars().all(|c| c.is_ascii_hexdigit()),
            "fingerprint is hex ({a:?})"
        );
        assert_ne!(a, "0", "not the old perma-wt0 constant");
    }

    // `cranelift`-gated: exercises `frame_precompiled`, which is `#[cfg(feature = "cranelift")]` (its only
    // caller is `--precompile-out`). `unframe_precompiled` (ungated) is covered by the run-path tests.
    #[cfg(feature = "cranelift")]
    #[test]
    fn precompiled_cwasm_framing_round_trips_and_raw_is_passthrough() {
        // The self-framed guest `.cwasm` (`frame_precompiled`) carries the `cdz-result-type` section THROUGH
        // the AOT split so the cranelift-free deserialize render is TYPED (corpus-28 nested-Bytes `#list`→
        // `b"…"` fix). Pin: a framed artifact splits back to (section, cwasm) EXACTLY; a RAW `.cwasm` (no
        // magic — the runtime/store precompiles + every legacy artifact) is passthrough `(None, whole)` =
        // back-compat, so an unframed artifact deserializes byte-for-byte as before.
        let cwasm = b"\x00serialized-cranelift-artifact\xff".to_vec();
        let rtypes = b"cdz-result-type binary-AST payload".to_vec();
        // Framed: unframe recovers the section + the EXACT cwasm tail.
        let framed = frame_precompiled(cwasm.clone(), Some(rtypes.clone()));
        assert_ne!(framed, cwasm, "framing must change the bytes");
        assert_eq!(
            unframe_precompiled(&framed),
            (Some(rtypes.as_slice()), cwasm.as_slice()),
            "framed .cwasm round-trips to (section, cwasm) exactly"
        );
        // No section (a runtime/store precompile) → RAW cwasm, byte-identical (those artifacts unaffected).
        let raw = frame_precompiled(cwasm.clone(), None);
        assert_eq!(
            raw, cwasm,
            "no section → raw cwasm (runtime/store stay byte-identical)"
        );
        assert_eq!(
            unframe_precompiled(&raw),
            (None, cwasm.as_slice()),
            "a raw .cwasm has no framed section → type-blind (prior behavior)"
        );
        // A too-short / legacy artifact that cannot hold a frame header → passthrough, no panic (totality).
        let tiny = vec![1u8, 2, 3];
        assert_eq!(unframe_precompiled(&tiny), (None, tiny.as_slice()));
    }

    #[test]
    fn runtime_cache_roundtrips_through_the_fingerprinted_path() {
        // End-to-end: `load_runtime_component` must WRITE a `<hash>-wt<fp>.cwasm` on the first call and
        // DESERIALIZE it on the second (the ~300× speedup that keeps the corpus gate fast). Pins that the
        // fingerprinted filename is actually produced + re-read — a regression that broke the name (e.g.
        // reverting to a non-varying token, or a write/read path mismatch) fails here. An empty `(component)`
        // needs no runtime import, so this is hermetic (no value-heap store).
        let e = engine();
        let bytes = wat::parse_str("(component)").expect("assemble an empty component");
        let dir = std::env::temp_dir().join(format!("cdz-run-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp cache dir");
        let opts = RunOpts {
            runtime_cache_dir: Some(dir.clone()),
            ..Default::default()
        };
        // The cache file is keyed by the CONTENT ADDRESS of the bytes actually compiled (not a
        // component's recorded requirement), so the debug-override path can never collide with a
        // release cwasm — see `load_runtime_component`.
        let hash = crate::cli::content_address(&bytes);

        // First call: cache miss → compile + write the fingerprinted artifact.
        load_runtime_component(&e, &bytes, &opts).expect("first load compiles");
        let expected = dir.join(format!("{hash}-wt{}.cwasm", engine_fp(&e)));
        assert!(
            expected.exists(),
            "first load must write the fingerprinted cache file at {expected:?}"
        );

        // Second call: the file exists → it must DESERIALIZE cleanly (the fast path), not error.
        load_runtime_component(&e, &bytes, &opts)
            .expect("second load deserializes the cached artifact");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_precompiled_artifact_deserializes_into_the_cranelift_free_execs_engine_config() {
        // seq-250 CROSS-CONFIG COMPAT self-check (v-nix's acceptance concern, verified in-PR). The AOT
        // corpus-exec deserializes `.cwasm` artifacts produced by the cranelift-ON precompile tool into a
        // cranelift-FREE engine. The ONLY `Config` delta between the two engines is `cranelift_opt_level`
        // (set in `engine()`, absent in the compiler-free build); everything else — `epoch_interruption`,
        // target, tunables, wasmtime version — is identical. This pins that `opt_level` is NOT a
        // deserialize-compatibility bit, so a tool-produced artifact loads in the exec.
        //
        // A single build cannot hold BOTH a compiler-on and a compiler-off engine, so we mimic the
        // cranelift-free exec's `engine()` with a second cranelift-ON engine that OMITS `cranelift_opt_level`
        // — its deserialize-compat metadata is exactly the cranelift-free engine's (whose only removed knob
        // is that opt_level). If wasmtime ever DID record opt_level as a compat bit, this deserialize would
        // fail here, flagging that `engine()`'s Config needs aligning before the exec can load tool artifacts.
        let component_bytes = wat::parse_str("(component)").expect("assemble empty component");
        let artifact =
            precompile_component_bytes(&component_bytes).expect("precompile via engine()");

        let mut cfg = Config::new();
        cfg.epoch_interruption(true); // matches engine(); opt_level deliberately NOT set (the exec's delta)
        let exec_like = Engine::new(&cfg).expect("build an exec-like (no-opt-level) engine");
        // SAFETY: our own freshly-produced artifact; deserialize re-validates the compat header.
        let loaded = unsafe { Component::deserialize(&exec_like, &artifact) };
        assert!(
            loaded.is_ok(),
            "a cranelift-ON-precompiled .cwasm must deserialize into the cranelift-free exec's engine \
             config (opt_level is not a compat bit); got: {:?}",
            loaded.err()
        );
    }

    #[test]
    fn hash_extracted_from_pinned_import() {
        let name = "cadenza:runtime/heap@0.0.0+abc123";
        assert!(import_is_runtime(name));
        assert_eq!(hash_from_import(name), "abc123");
    }

    #[test]
    fn bare_interface_import_has_no_hash() {
        assert!(import_is_runtime(RUNTIME_IFACE));
        assert_eq!(hash_from_import(RUNTIME_IFACE), "");
    }

    #[test]
    fn non_runtime_import_is_not_matched() {
        assert!(!import_is_runtime("cadenza:host/emit-event"));
    }

    #[test]
    fn host_response_scalar_extracted_from_value_form() {
        // A `(: v T)` form yields the bare value; a bare value passes through; whitespace is tolerated.
        assert_eq!(scalar_of_value_form("(: 10 Int64)"), "10");
        assert_eq!(scalar_of_value_form("(: 42 Int64)"), "42");
        assert_eq!(scalar_of_value_form("7"), "7");
        assert_eq!(scalar_of_value_form("  3  "), "3");
    }

    #[test]
    fn trap_message_surfaces_the_host_error_cause_not_just_the_wrapper() {
        // A HOST func error (e.g. an exhausted `--host-response` list) is propagated through wasm as a
        // wrapped anyhow chain: the OUTER message is a generic "error while executing …" and the actionable
        // reason is a CAUSE. `trap_message` must render the whole chain (`{e:#}`) so the reason is visible —
        // the bare `{e}` (outer only) buried it (a fleet breaker flagged the bare-wrapper output).
        let root = anyhow!(
            "host call `ask.ask` has no recorded response (call 2 of the run; 1 response(s) supplied via --host-response)"
        );
        let wrapped = root.context("error while executing at wasm backtrace: 0: ask.ask");
        let msg = trap_message(&wrapped);
        assert!(
            msg.contains("has no recorded response") && msg.contains("call 2 of the run"),
            "the host-call cause (op + call index) must be surfaced, not just the wrapper: {msg:?}"
        );
    }

    #[test]
    fn runtime_import_name_recognized() {
        // The host-import binder skips the value-heap runtime instance (bound elsewhere).
        assert!(is_runtime_import_name("cadenza:runtime/heap@0.0.0+abc"));
        assert!(!is_runtime_import_name("ask"));
    }

    #[test]
    fn tuple_fields_split_at_top_level() {
        // Bare and `tuple`-headed spellings both split into their scalar fields; a nested group stays whole.
        assert_eq!(parse_tuple_fields("(10 3)").unwrap(), vec!["10", "3"]);
        assert_eq!(parse_tuple_fields("(tuple 10 3)").unwrap(), vec!["10", "3"]);
        assert_eq!(
            parse_tuple_fields("(record (x 10) (y 3))").unwrap(),
            vec!["(x 10)", "(y 3)"]
        );
        assert!(parse_tuple_fields("10").is_none()); // not a paren group
    }

    #[test]
    fn tuple_fields_accept_the_native_ctor_head() {
        // M3 completion: the native #head(…) forms coerce identically to the paren form (the exact inverse
        // of the corpus input-nativization). Head-only normalization; fields (incl nested #forms) recurse.
        assert_eq!(parse_tuple_fields("#tuple(10 3)").unwrap(), vec!["10", "3"]);
        assert_eq!(
            parse_tuple_fields("#list(1 2 3)").unwrap(),
            vec!["list", "1", "2", "3"] // the List arm drops the leading `list`, as for `(list …)`
        );
        assert_eq!(
            parse_tuple_fields("#record((= x 42))").unwrap(),
            vec!["(= x 42)"]
        );
        // A nested #form field stays whole (its own coerce_one recursion re-enters here).
        assert_eq!(
            parse_tuple_fields("#list(#record((= x 1)) #record((= x 2)))").unwrap(),
            vec!["list", "#record((= x 1))", "#record((= x 2))"]
        );
        assert!(parse_tuple_fields("#nope").is_none()); // `#head` without a paren group is not a ctor form
    }

    #[test]
    fn record_head_detected_for_both_paren_and_native_ctor() {
        // A record value crossing as a tuple<…> must be name-sorted + unwrapped; detect the record head in
        // BOTH spellings (the #record(…) native form is what M3 nativization writes — the #5718 follow-on).
        assert!(is_record_headed("(record (= x 42))"));
        assert!(is_record_headed("#record((= x 42))"));
        assert!(is_record_headed("  #record((= a 1) (= b 2))  "));
        // A genuine positional tuple is NOT record-headed (must coerce positionally, never name-unwrap).
        assert!(!is_record_headed("(tuple 3 4)"));
        assert!(!is_record_headed("(3 4)"));
        assert!(!is_record_headed("#tuple(3 4)"));
        assert!(!is_record_headed("#list(1 2 3)"));
    }

    #[test]
    fn named_record_field_detected_and_unwrapped() {
        // A `(name value)` group is recognized as a record field and unwraps to its value.
        assert_eq!(
            named_field("(x 10)"),
            Some(("x".to_string(), "10".to_string()))
        );
        // A named field with a Bool value (the name is a real identifier).
        assert_eq!(
            named_field("(flag true)"),
            Some(("flag".to_string(), "true".to_string()))
        );
        assert_eq!(named_field("(10 3)"), None); // numeric head → a positional tuple, not a record field
        assert_eq!(named_field("(true false)"), None); // a positional Bool tuple, not a named field
        assert_eq!(named_field("10"), None); // a bare scalar
        assert_eq!(unwrap_named_field("(x 10)"), "10");
        assert_eq!(unwrap_named_field("10"), "10");
    }
}
