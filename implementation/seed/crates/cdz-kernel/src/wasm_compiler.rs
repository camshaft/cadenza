//! Run the compiler AS A WASM (minimal-kernel re-charter, K1-dep wasm-swap; operator #54).
//!
//! The deploy-once kernel does NOT native-link rcdzc — it loads `rcdzc.wasm` (the wasm32-wasip1 build of the
//! compiler, from the `rcdzc-wasm` crate) and invokes it via the wasmtime it already hosts. So the compiler is
//! a swappable wasm artifact, updatable without a kernel redeploy. This module is the host glue: instantiate
//! the compiler-wasm as a core module with a WASI preview1 ctx, and drive its alloc→compile→read→dealloc ABI
//! to turn AST bytes into a program-component wasm.
//!
//! The compiler-wasm's ABI (see `rcdzc-wasm/src/lib.rs`): `alloc(len)->ptr`, `compile(ptr,len)->packed`
//! (`(rptr<<32)|rlen`; result region = `[status:1][payload…]`, status 0=ok/component, 1=err/UTF-8 diagnostic),
//! `dealloc(ptr,len)`. This is plan (a) — a direct core-module+WASI path; plan (b) re-authors rcdzc-wasm as a
//! component to run uniformly through cdz-run's Component API (then this module folds away). Feature-gated
//! (`wasm-compiler`) so the default build stays light (native rcdzc compile path).

use anyhow::{anyhow, Context, Result};
use wasmtime::{Engine, Linker, Memory, Module, Store, TypedFunc};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::WasiCtxBuilder;

/// Compile a Cadenza program's AST bytes into a program-component wasm by RUNNING the compiler-wasm
/// (`rcdzc_wasm`). Instantiates the core module with a WASI preview1 ctx, then drives its host-alloc ABI:
/// `alloc(n)` → write the AST at `ptr` → `compile(ptr,n)` → read the packed result region → `dealloc` both.
/// Returns the component bytes on success, or the compiler's diagnostic string on a compile error (status 1).
/// This is the wasm-swap: the same `(AST bytes) -> (component wasm)` contract as the native
/// [`crate::kernel::compile_interpret_provider`], but executed inside wasmtime instead of native rcdzc.
pub fn compile_via_wasm(rcdzc_wasm: &[u8], ast_bytes: &[u8]) -> Result<Vec<u8>> {
    let engine = Engine::default();
    let module = Module::new(&engine, rcdzc_wasm).context("load rcdzc.wasm module")?;

    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |t| t)
        .context("add WASI preview1 to the linker")?;
    let wasi = WasiCtxBuilder::new().build_p1();
    let mut store = Store::new(&engine, wasi);
    let instance = linker
        .instantiate(&mut store, &module)
        .context("instantiate rcdzc.wasm")?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("rcdzc.wasm exports no `memory`"))?;
    let alloc: TypedFunc<u32, u32> = instance
        .get_typed_func(&mut store, "alloc")
        .context("rcdzc.wasm `alloc` export")?;
    let compile: TypedFunc<(u32, u32), u64> = instance
        .get_typed_func(&mut store, "compile")
        .context("rcdzc.wasm `compile` export")?;
    let dealloc: TypedFunc<(u32, u32), ()> = instance
        .get_typed_func(&mut store, "dealloc")
        .context("rcdzc.wasm `dealloc` export")?;

    let ast_len =
        u32::try_from(ast_bytes.len()).map_err(|_| anyhow!("AST too large for the wasm ABI"))?;

    // 1) alloc a guest region + write the AST bytes into it.
    let in_ptr = alloc
        .call(&mut store, ast_len)
        .context("alloc AST region")?;
    if in_ptr == 0 {
        return Err(anyhow!("rcdzc.wasm alloc returned null (out of memory)"));
    }
    write_mem(&memory, &mut store, in_ptr, ast_bytes)?;

    // 2) compile → the packed (ptr<<32)|len result region.
    let packed = compile
        .call(&mut store, (in_ptr, ast_len))
        .context("call rcdzc.wasm `compile`")?;
    // Free the input region now (the result is independent).
    dealloc.call(&mut store, (in_ptr, ast_len)).ok();

    if packed == 0 {
        return Err(anyhow!(
            "rcdzc.wasm compile returned null (allocation failure)"
        ));
    }
    let rptr = (packed >> 32) as u32;
    let rlen = (packed & 0xffff_ffff) as u32;

    // 3) read the result region `[status:1][payload…]`.
    let region = read_mem(&memory, &mut store, rptr, rlen)?;
    // Free the result region (before returning — the bytes are copied into `region`).
    dealloc.call(&mut store, (rptr, rlen)).ok();

    let (&status, payload) = region
        .split_first()
        .ok_or_else(|| anyhow!("empty result region from rcdzc.wasm"))?;
    match status {
        0 => Ok(payload.to_vec()),
        1 => Err(anyhow!(
            "rcdzc.wasm compile error: {}",
            String::from_utf8_lossy(payload)
        )),
        other => Err(anyhow!("rcdzc.wasm returned unknown status byte {other}")),
    }
}

/// Write `data` into the guest linear `memory` at `ptr`, bounds-checked.
fn write_mem(memory: &Memory, store: &mut Store<WasiP1Ctx>, ptr: u32, data: &[u8]) -> Result<()> {
    memory
        .write(store, ptr as usize, data)
        .map_err(|e| anyhow!("write {} bytes into guest memory at {ptr}: {e}", data.len()))
}

/// Read `len` bytes out of the guest linear `memory` at `ptr`, bounds-checked.
fn read_mem(memory: &Memory, store: &mut Store<WasiP1Ctx>, ptr: u32, len: u32) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    memory
        .read(store, ptr as usize, &mut buf)
        .map_err(|e| anyhow!("read {len} bytes from guest memory at {ptr}: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate the built `rcdzc.wasm` (the wasm32-wasip1 debug/release artifact of the rcdzc-wasm crate). A
    /// test SKIPS if it is not built (like the runtime-store gating) — `cargo build --target wasm32-wasip1 -p
    /// rcdzc-wasm` produces it. Walks up from the crate dir to the seed root, then into rcdzc-wasm's target.
    fn find_rcdzc_wasm() -> Option<Vec<u8>> {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()? // crates/
            .join("rcdzc-wasm/target/wasm32-wasip1");
        for profile in ["debug", "release"] {
            let p = base.join(profile).join("rcdzc_wasm.wasm");
            if let Ok(bytes) = std::fs::read(&p) {
                return Some(bytes);
            }
        }
        None
    }

    fn ast_of(src: &str) -> Vec<u8> {
        let arenas = cadenza_syntax::sexpr::read(src).expect("test source parses");
        cadenza_syntax::codec::encode(&arenas)
    }

    #[test]
    fn compile_via_wasm_matches_the_native_compiler_differentially() {
        // THE wasm-swap proof: running rcdzc.wasm on an AST produces the SAME component the native
        // compile_interpret_provider's underlying compile does — i.e. the compiler-as-wasm is faithful.
        let Some(rcdzc_wasm) = find_rcdzc_wasm() else {
            eprintln!("[wasm_compiler] rcdzc.wasm not built (cargo build --target wasm32-wasip1 -p rcdzc-wasm); skipping");
            return;
        };
        let ast = ast_of("(do (def (main) 42) (export main))");

        let via_wasm =
            compile_via_wasm(&rcdzc_wasm, &ast).expect("compile a valid program via wasm");
        assert!(
            !via_wasm.is_empty() && &via_wasm[..4] == b"\0asm",
            "wasm-compiled output is a wasm component"
        );

        // Differential: the native compile of the SAME AST produces byte-identical component bytes (both call
        // the same rcdzc::compile_component; the wasm build is deterministic).
        let native = rcdzc::compile_component(&ast).expect("native compile of the same AST");
        assert_eq!(
            via_wasm, native,
            "the compiler-wasm produces the SAME component as native rcdzc (faithful wasm-swap)"
        );
    }

    #[test]
    fn an_impossible_alloc_returns_the_zero_sentinel_not_a_trap() {
        // Reviewer-flagged robustness pin (rcdzc-wasm alloc/compile): an oversized/hostile allocation request
        // must return the documented 0 sentinel — NOT abort the whole instance. rcdzc-wasm's alloc uses
        // try_reserve_exact (not with_capacity, which calls handle_alloc_error -> the `unreachable` trap on
        // OOM). We drive `alloc(u32::MAX)` (a ~4 GiB request wasm32 can't satisfy) directly and assert the
        // call RETURNS a null ptr, with the instance still alive (a subsequent small alloc succeeds). If a
        // future change reverts to with_capacity, this alloc TRAPS and the call errors — flipping the test.
        let Some(rcdzc_wasm) = find_rcdzc_wasm() else {
            eprintln!("[wasm_compiler] rcdzc.wasm not built; skipping");
            return;
        };
        let engine = Engine::default();
        let module = Module::new(&engine, &rcdzc_wasm).expect("load rcdzc.wasm");
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |t| t).expect("wasi");
        let mut store = Store::new(&engine, WasiCtxBuilder::new().build_p1());
        let instance = linker
            .instantiate(&mut store, &module)
            .expect("instantiate");
        let alloc: TypedFunc<u32, u32> = instance
            .get_typed_func(&mut store, "alloc")
            .expect("alloc export");

        let ptr = alloc
            .call(&mut store, u32::MAX)
            .expect("an impossible alloc RETURNS gracefully (does not trap the instance)");
        assert_eq!(
            ptr, 0,
            "an unsatisfiable alloc returns the 0 (null) OOM sentinel"
        );
        // The instance survived the failed alloc — a normal-sized alloc still works afterward.
        let ok = alloc
            .call(&mut store, 16)
            .expect("the instance is still alive after a failed alloc");
        assert_ne!(
            ok, 0,
            "a reasonable alloc succeeds after the failed one (instance not trapped)"
        );
    }

    #[test]
    fn compile_via_wasm_surfaces_a_compile_error() {
        let Some(rcdzc_wasm) = find_rcdzc_wasm() else {
            eprintln!("[wasm_compiler] rcdzc.wasm not built; skipping");
            return;
        };
        let ast = ast_of("(do (def (main) undefined-name) (export main))");
        let err = compile_via_wasm(&rcdzc_wasm, &ast)
            .expect_err("a program with an unbound name must not compile");
        assert!(
            format!("{err:#}").contains("compile error"),
            "the wasm compiler surfaces the diagnostic: {err:#}"
        );
    }
}
