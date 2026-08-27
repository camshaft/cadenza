//! Shared drivers for the relocated wasmtime behavior tests — the counterparts of the helpers that
//! lived in `rcdzc/src/tests.rs`. Import paths are the only change: `crate::` -> `rcdzc::`.

#![allow(dead_code)] // helpers are consumed incrementally as test groups relocate in.

/// Read a test program from the s-expression surface into rcdzc's `Arenas`, through the REAL front-end
/// reader (`cadenza-syntax::sexpr`) via a byte round-trip — exactly as rcdzc's `testkit::parse` did. The
/// bridge between the two crates' distinct `Arenas` is BYTES: `sexpr::read` -> cadenza-syntax `encode`
/// -> rcdzc `codec::decode`, which also exercises that rcdzc's copied `codec.rs` stays byte-compatible.
pub fn parse(src: &str) -> rcdzc::ast::Arenas {
    let arenas = cadenza_syntax::sexpr::read(src)
        .unwrap_or_else(|e| panic!("test s-expr failed to read: {e:?}\n  src: {src}"));
    let bytes = cadenza_syntax::codec::encode(&arenas);
    rcdzc::codec::decode(&bytes)
        .unwrap_or_else(|| panic!("cadenza-syntax bytes failed to decode with rcdzc codec: {src}"))
}

/// Compile a test program source string to a component, via rcdzc's public compile surface: parse ->
/// rcdzc `codec::encode` -> `compile_component`. Panics with the diagnostic on a compile error.
pub fn compile_component(src: &str) -> Vec<u8> {
    rcdzc::compile_component(&rcdzc::codec::encode(&parse(src)))
        .unwrap_or_else(|d| panic!("compile failed: {d:?}\n  src: {src}"))
}

/// Decode a boundary result value, panicking with a type-named message on a mismatch.
pub trait FromVal: Sized {
    fn from_val(v: &wasmtime::component::Val) -> Self;
}

macro_rules! from_val_scalar {
    ($t:ty, $variant:ident) => {
        impl FromVal for $t {
            fn from_val(v: &wasmtime::component::Val) -> $t {
                match v {
                    wasmtime::component::Val::$variant(n) => *n,
                    other => panic!(
                        concat!("expected ", stringify!($variant), " result, got {:?}"),
                        other
                    ),
                }
            }
        }
    };
}

from_val_scalar!(i64, S64);
from_val_scalar!(bool, Bool);
from_val_scalar!(u64, U64);
from_val_scalar!(u32, U32);
from_val_scalar!(i32, S32);
// Narrow component primitives — an aliased <=16-bit width crosses as its faithful s8/u8/s16/u16.
from_val_scalar!(i8, S8);
from_val_scalar!(u8, U8);
from_val_scalar!(i16, S16);
from_val_scalar!(u16, U16);
// Floats cross as the component f64/f32 primitives (bits-compared in tests: -0.0 != 0.0, NaN exact).
from_val_scalar!(f64, Float64);
from_val_scalar!(f32, Float32);

/// Instantiate `component_bytes` under wasmtime, call its nullary export `name`, and return the single
/// result decoded to `T` — the "run the artifact" behavior check, generic over the boundary type.
pub fn run_returns<T: FromVal>(component_bytes: &[u8], name: &str) -> T {
    run_returns_with(component_bytes, name, &[])
}

/// Instantiate `component_bytes` and call export `name` WITH the given argument values, decoding the
/// single result to `T` — the behavior check for a parameterized exported function.
pub fn run_returns_with<T: FromVal>(
    component_bytes: &[u8],
    name: &str,
    args: &[wasmtime::component::Val],
) -> T {
    use wasmtime::component::{Component, Linker, Val};
    use wasmtime::{Engine, Store};

    let engine = Engine::default();
    let component = Component::from_binary(&engine, component_bytes).expect("valid component");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker
        .instantiate(&mut store, &component)
        .expect("instantiate");
    let func = instance.get_func(&mut store, name).expect("export present");
    // A one-slot result buffer; the initial value is overwritten by the call (its variant is
    // irrelevant — `call` writes the actual result), then decoded to `T`.
    let mut results = [Val::Bool(false)];
    func.call(&mut store, args, &mut results).expect("call");
    func.post_return(&mut store).expect("post_return");
    T::from_val(&results[0])
}

/// Count the core-module instructions in `component_bytes` matching `pred` — an emission-strategy probe
/// (e.g. `i64.mul` count for inline-vs-emit-once). Walks every code-section entry with `wasmparser`.
pub fn count_opcode(component_bytes: &[u8], pred: impl Fn(&wasmparser::Operator) -> bool) -> usize {
    use wasmparser::{Parser, Payload};
    let mut n = 0;
    for payload in Parser::new(0).parse_all(component_bytes) {
        if let Ok(Payload::CodeSectionEntry(body)) = payload {
            let mut ops = body.get_operators_reader().expect("ops");
            while let Ok(op) = ops.read() {
                if pred(&op) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Instantiate `component_bytes`, call export `name` with `args`, and report whether the call TRAPPED
/// — the trap-assertion behavior check (empty linker, so for programs that import no runtime).
pub fn call_traps(component_bytes: &[u8], name: &str, args: &[wasmtime::component::Val]) -> bool {
    use wasmtime::component::{Component, Linker, Val};
    use wasmtime::{Engine, Store};

    let engine = Engine::default();
    let component = Component::from_binary(&engine, component_bytes).expect("valid component");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker
        .instantiate(&mut store, &component)
        .expect("instantiate");
    let func = instance.get_func(&mut store, name).expect("export present");
    let mut results = [Val::S64(0)];
    func.call(&mut store, args, &mut results).is_err()
}

/// Locate the built value-heap runtime component `.wasm` whose content hash matches the compiler's
/// `REQUIRED_RUNTIME_HASH`, or `None` if it is not present (so a heap test SKIPS rather than fails when
/// the runtime has not been built — `cargo xtask build`/`codegen` produces it). When `CADENZA_STORE` is
/// set it is AUTHORITATIVE (resolve only there, no fallback), so a storeless rerun makes this `None`.
pub fn find_runtime_wasm() -> Option<Vec<u8>> {
    use rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH;
    let candidates: Vec<std::path::PathBuf> = if let Ok(dir) = std::env::var("CADENZA_STORE") {
        vec![std::path::PathBuf::from(dir).join(format!("{REQUIRED_RUNTIME_HASH}.wasm"))]
    } else {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let seed = manifest.join("../..").canonicalize().ok()?; // .../implementation/seed
        let repo = seed.join("../..").canonicalize().ok()?; // repo root
        vec![
            repo.join(format!("target/cadenza-store/{REQUIRED_RUNTIME_HASH}.wasm")),
            seed.join("crates/cdz-runtime/target/wasm32-unknown-unknown/release/cdz_runtime.wasm"),
        ]
    };
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path)
            && cdz_run::cli::content_address(&bytes) == REQUIRED_RUNTIME_HASH
        {
            return Some(bytes);
        }
    }
    None
}

/// The NFC component the value-heap runtime depends on, resolved by content address exactly like
/// [`find_runtime_wasm`]. Returns `None` when absent (caller skips the run).
pub fn find_nfc_wasm() -> Option<Vec<u8>> {
    use rcdzc::backend::wasm::runtime_abi::REQUIRED_NFC_HASH;
    let candidates: Vec<std::path::PathBuf> = if let Ok(dir) = std::env::var("CADENZA_STORE") {
        vec![std::path::PathBuf::from(dir).join(format!("{REQUIRED_NFC_HASH}.wasm"))]
    } else {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let seed = manifest.join("../..").canonicalize().ok()?;
        let repo = seed.join("../..").canonicalize().ok()?;
        vec![repo.join(format!("target/cadenza-store/{REQUIRED_NFC_HASH}.wasm"))]
    };
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path)
            && cdz_run::cli::content_address(&bytes) == REQUIRED_NFC_HASH
        {
            return Some(bytes);
        }
    }
    None
}

/// Locate the DEBUG-counters value-heap runtime (`DEBUG_RUNTIME_HASH`) in the content-addressed store,
/// verifying its hash — the runtime the `live-objects` reclaim probes need. `None` when absent.
pub fn find_debug_runtime_wasm() -> Option<Vec<u8>> {
    use rcdzc::backend::wasm::runtime_abi::DEBUG_RUNTIME_HASH;
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let seed = manifest.join("../..").canonicalize().ok()?;
    let repo = seed.join("../..").canonicalize().ok()?;
    let path = repo.join(format!("target/cadenza-store/{DEBUG_RUNTIME_HASH}.wasm"));
    let bytes = std::fs::read(&path).ok()?;
    (cdz_run::cli::content_address(&bytes) == DEBUG_RUNTIME_HASH).then_some(bytes)
}

/// Run a component that MAY import the value-heap runtime (e.g. one returning a heap tuple), linking the
/// runtime via `cdz_run` and returning its rendered result string, or `None` when the runtime wasm is
/// absent (so the caller skips the run — the established heap-test pattern). Panics on a trap.
pub fn run_linked(component_bytes: &[u8], export: &str) -> Option<String> {
    let runtime = find_runtime_wasm()?;
    let opts = cdz_run::RunOpts {
        export: Some(export.to_string()),
        args: vec![],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(component_bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => Some(s),
        cdz_run::Outcome::Trap(t) => panic!("linked run trapped: {t}"),
    }
}
