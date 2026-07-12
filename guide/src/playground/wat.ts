/// Convert an emitted WebAssembly component to its text form (WAT), for the playground's "WAT" output
/// view. Uses the `wasm-tools` WASM that ships inside `@bytecodealliance/jco-transpile` (the same
/// toolkit that transpiles components to run them), via its exported `wasm-tools` subpath — `print`
/// is the in-browser `wasm-tools print`, `$init`-guarded internally.

import { print } from "@bytecodealliance/jco-transpile/wasm-tools";

/// Print component bytes as WAT text. Returns a friendly message rather than throwing on failure.
export async function toWat(component: Uint8Array): Promise<string> {
  try {
    return await print(component);
  } catch (e) {
    return `; could not render WAT: ${e instanceof Error ? e.message : String(e)}`;
  }
}
