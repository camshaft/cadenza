//! rcdzc-wasm — the Cadenza compiler as a WASM artifact (agent-runtime minimal-kernel, K1-dep wasm-swap).
//!
//! Operator directive #54: the agent-kernel runs a WASM build of the compiler via its embedded wasmtime, not
//! native-linked rcdzc — keeping the kernel minimal (wasm-host + log + broad primitives) and the compiler a
//! swappable wasm artifact updatable without a kernel redeploy. This crate is that artifact.
//!
//! It wraps [`rcdzc::compile_component`] (the wasm-portable compile path — feasibility proven) in a wasm
//! entrypoint: **AST bytes in → program-component wasm bytes out**. It takes AST bytes (not surface text) so
//! the front-end reader (`cadenza-syntax`) need NOT be in the compiler wasm — the log stores AST bytes anyway
//! (the routed source-vs-AST sub-fork; AST-in is the lean). The compile GLUE ([`compile_ast`]) is pure + host-
//! testable; the wasm EXPORT ([`compile`]) wraps it over a minimal linear-memory ABI.

/// Compile a Cadenza program's AST bytes into a program-component wasm (the ABI-independent glue — pure, and
/// unit-testable on the host). Returns `Ok(component_bytes)` or `Err(diagnostic_message)` — a bad program is a
/// loud error string the kernel surfaces, not a silent empty. Wraps [`rcdzc::compile_component`], the same
/// entrypoint the native K1 kernel used, now reachable from the wasm build.
pub fn compile_ast(ast_bytes: &[u8]) -> Result<Vec<u8>, String> {
    rcdzc::compile_component(ast_bytes).map_err(|d| format!("{} [{:?}]", d.message, d.code))
}

/// Compile a Cadenza program's AST bytes into a PROVIDER component published under `iface` (e.g.
/// `cadenza:agent/kernel`) — the named counterpart of [`compile_ast`], the foundation for wasm-swapping the
/// agent-kernel's PROVIDER compile (its `interpret` is a named provider peer, not a plain component). Mirrors
/// the kernel's native `compile_interpret_provider` recipe: compile with a `component-name` artifact under the
/// compiler stack, extract the wasm artifact. Pure + host-testable (like `compile_ast`); a future wasm export
/// wraps it once the linear-memory ABI carries the interface string. Returns `Ok(component_bytes)` or the
/// diagnostic string. (Plain `compile_ast` = `compile_component` = NO component name — this adds the name so a
/// peer executor can bind the export, which the plain path can't produce.)
pub fn compile_ast_named(ast_bytes: &[u8], iface: &str) -> Result<Vec<u8>, String> {
    use rcdzc::abi::Artifact;
    use rcdzc::backend::Target;
    let out = rcdzc::host::run_with_compiler_stack(|| {
        rcdzc::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "interpret", ast_bytes.to_vec()),
                rcdzc::cli::component_name_artifact(iface),
            ],
            &[Target::Wasm],
        )
    });
    out.artifact(Target::Wasm.artifact_kind())
        .map(|b| b.to_vec())
        .ok_or_else(|| format!("provider did not compile: {:?}", out.diagnostics))
}

/// The minimal linear-memory ABI the kernel's wasmtime invokes (wasm target only). The kernel writes the AST
/// bytes into this module's memory at `ptr` (len `len`), calls `compile`, and reads the RESULT the same way:
/// the return value packs the result `(ptr << 32) | len` into a u64, where the result region is
/// `[status: 1 byte][payload...]` — status 0 = ok (payload = component wasm), status 1 = error (payload = the
/// UTF-8 diagnostic). A packed return of 0 means "could not even allocate" (out of memory). This is a first,
/// deliberately-simple core ABI; a richer component-model `compile: (list u8) -> result<list u8, string>` is a
/// routed sub-fork (the crate compiles as a cdylib either way; the glue [`compile_ast`] is unchanged).
///
/// # Safety
/// `ptr`/`len` must describe a valid, initialized region in this module's linear memory (the kernel wrote it).
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn compile(ptr: u32, len: u32) -> u64 {
    let ast = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    let (status, payload) = match compile_ast(ast) {
        Ok(bytes) => (0u8, bytes),
        Err(msg) => (1u8, msg.into_bytes()),
    };
    pack_result(status, &payload)
}

/// Compile AST bytes into a NAMED PROVIDER component (published under the interface string), returning the
/// same packed `(rptr << 32) | rlen` result region as [`compile`] — the named counterpart wrapping
/// [`compile_ast_named`]. The kernel writes the AST at `ast_ptr` (len `ast_len`) AND the interface string
/// (UTF-8, e.g. `cadenza:agent/kernel`) at `iface_ptr` (len `iface_len`), then calls this. So the kernel can
/// wasm-swap its PROVIDER compile (a peer-bindable named export), not just a plain component. Result region +
/// OOM-sentinel (packed 0) semantics are identical to [`compile`]. A non-UTF-8 interface string is a status-1
/// error (a bad name is a loud diagnostic, not a trap).
///
/// # Safety
/// `ast_ptr`/`ast_len` and `iface_ptr`/`iface_len` must each describe a valid, initialized region in this
/// module's linear memory (the kernel wrote them).
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn compile_named(
    ast_ptr: u32,
    ast_len: u32,
    iface_ptr: u32,
    iface_len: u32,
) -> u64 {
    let ast = std::slice::from_raw_parts(ast_ptr as *const u8, ast_len as usize);
    let iface_bytes = std::slice::from_raw_parts(iface_ptr as *const u8, iface_len as usize);
    let (status, payload) = match std::str::from_utf8(iface_bytes) {
        Ok(iface) => match compile_ast_named(ast, iface) {
            Ok(bytes) => (0u8, bytes),
            Err(msg) => (1u8, msg.into_bytes()),
        },
        Err(_) => (1u8, b"interface name is not valid UTF-8".to_vec()),
    };
    pack_result(status, &payload)
}

/// Build the packed result region `(rptr << 32) | rlen` for a `[status: 1 byte][payload...]` region, leaked
/// so the host can read it before freeing (via [`dealloc`]). Returns the packed-0 OOM sentinel if the region
/// can't be allocated. Shared by [`compile`] and [`compile_named`] so the two exports pack results identically.
/// `try_reserve_exact` (not `with_capacity`) keeps an oversized payload from aborting the whole instance (
/// `with_capacity` calls `handle_alloc_error` -> the wasm `unreachable` trap on OOM).
#[cfg(target_arch = "wasm32")]
fn pack_result(status: u8, payload: &[u8]) -> u64 {
    let mut out = Vec::<u8>::new();
    if out.try_reserve_exact(1 + payload.len()).is_err() {
        return 0; // "could not even allocate" — honor the packed-0 OOM sentinel, don't trap the instance.
    }
    out.push(status);
    out.extend_from_slice(payload);
    let boxed = out.into_boxed_slice();
    let rlen = boxed.len() as u64;
    let rptr = Box::into_raw(boxed) as *mut u8 as u64;
    (rptr << 32) | rlen
}

/// Allocate `len` bytes in this module's linear memory and return the pointer — the host calls this FIRST to
/// get a region to write the AST bytes into (the host can't allocate guest memory itself). The classic wasm
/// host-alloc protocol: host `alloc(n)` → ptr, host writes n bytes at ptr, host `compile(ptr, n)`. The block
/// is leaked (forgotten) so it stays live across the host's write + the `compile` call; the host frees it with
/// [`dealloc`] after. Returns 0 on allocation failure.
///
/// # Safety
/// The returned pointer is valid for `len` bytes until [`dealloc`] is called with the same `ptr`/`len`.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn alloc(len: u32) -> u32 {
    // try_reserve_exact (not with_capacity) so an oversized/hostile `len` returns the documented 0 sentinel
    // rather than aborting the whole instance: with_capacity calls handle_alloc_error -> the `unreachable`
    // trap on OOM, which would kill the long-lived kernel instead of letting the host handle it gracefully.
    let mut buf = Vec::<u8>::new();
    if buf.try_reserve_exact(len as usize).is_err() {
        return 0; // out of memory — the host sees a null ptr and errors, instance stays alive.
    }
    let ptr = buf.as_mut_ptr() as u32;
    std::mem::forget(buf); // leak: the host owns it until dealloc
    ptr
}

/// Free a region previously returned by [`alloc`] (or a result region returned by [`compile`], whose length
/// is `(packed & 0xffff_ffff)`). The host calls this to release the AST-input region after `compile` and the
/// result region after reading it — so the compiler wasm doesn't leak across many compiles in a long-lived
/// kernel.
///
/// # Safety
/// `ptr`/`len` must be a region returned by [`alloc`]/[`compile`] and not already freed.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: u32, len: u32) {
    // Reconstitute the Vec with the SAME capacity `alloc` used (it allocated `len` capacity) and drop it.
    let _ = Vec::from_raw_parts(ptr as *mut u8, 0, len as usize);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build AST bytes for a source the same way the kernel would feed them (via cadenza-syntax's codec — a
    /// dev-dependency here, exactly as rcdzc's own testkit bridges the two crates' Arenas by bytes).
    fn ast_of(src: &str) -> Vec<u8> {
        let arenas = cadenza_syntax::sexpr::read(src).expect("test source parses");
        cadenza_syntax::codec::encode(&arenas)
    }

    #[test]
    fn compile_ast_compiles_a_valid_program_to_a_component() {
        // The glue compiles a real program's AST to a wasm component (non-empty bytes). This is exactly what
        // the wasm export wraps — proving the compiler-as-wasm path produces a program the kernel can run.
        let ast = ast_of("(do (def (main) 42) (export main))");
        let out = compile_ast(&ast).expect("a valid program compiles");
        assert!(!out.is_empty(), "the compiled component has bytes");
        // A wasm component starts with the wasm magic `\0asm`.
        assert_eq!(&out[..4], b"\0asm", "the output is a wasm module/component");
    }

    #[test]
    fn compile_ast_named_compiles_an_interpret_provider() {
        // The named counterpart: compile an `interpret` program AS A PROVIDER published under an interface —
        // what the agent-kernel needs (its interpret is a named provider peer). Produces a wasm component; the
        // component-name artifact makes the export bindable by a peer executor (the plain compile_ast can't).
        let ast =
            ast_of("(do (def (interpret (: kind Int64) (: turn Int64)) kind) (export interpret))");
        let out = compile_ast_named(&ast, "cadenza:agent/kernel")
            .expect("a valid interpret provider compiles");
        assert!(!out.is_empty(), "the compiled provider has bytes");
        assert_eq!(&out[..4], b"\0asm", "the output is a wasm module/component");
    }

    #[test]
    fn compile_ast_named_reports_a_bad_program_as_a_loud_error() {
        let ast = ast_of("(do (def (interpret) undefined-name) (export interpret))");
        let err = compile_ast_named(&ast, "cadenza:agent/kernel")
            .expect_err("a provider with an unbound name must not compile");
        assert!(!err.is_empty(), "the error carries the diagnostic: {err}");
    }

    #[test]
    fn compile_ast_reports_a_bad_program_as_a_loud_error() {
        // An unbound name is a loud Err string (the kernel surfaces it), not a silent empty component.
        let ast = ast_of("(do (def (main) undefined-name) (export main))");
        let err = compile_ast(&ast).expect_err("a program with an unbound name must not compile");
        assert!(
            err.contains("CDZ") || !err.is_empty(),
            "the error carries the diagnostic: {err}"
        );
    }
}
