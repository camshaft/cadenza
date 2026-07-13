/// Convert emitted WebAssembly bytes to their text form (WAT), for the playground's "WAT" output
/// view. Uses the `wasm-tools` WASM that ships inside `@bytecodealliance/jco-transpile` (the same
/// toolkit that transpiles components to run them), via its exported `wasm-tools` subpath — `print`
/// is the in-browser `wasm-tools print`, `$init`-guarded internally.
///
/// The playground feeds this the program's embedded CORE MODULE (unwrapped from the component and
/// compiled without DWARF), so the reader sees just the executed `(module …)` rather than the
/// component-model wrapper. `print` handles either shape, so this stays byte-agnostic.

import { print } from "@bytecodealliance/jco-transpile/wasm-tools";

/// Print wasm bytes (a core module or a component) as WAT text. Returns a friendly message rather than
/// throwing on failure.
export async function toWat(bytes: Uint8Array): Promise<string> {
  try {
    return await print(bytes);
  } catch (e) {
    return `; could not render WAT: ${e instanceof Error ? e.message : String(e)}`;
  }
}
