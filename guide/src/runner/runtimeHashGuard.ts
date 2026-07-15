/// Pure helpers for the runtime/compiler hash guard — NO worker/DOM imports, so `node --test` can
/// unit-test them. `client.ts` composes these with SubtleCrypto + the compiler's `runtimeHash()`.
///
/// WHY THIS GUARD EXISTS: jco strips the version+hash off the `cadenza:runtime/heap` import, so a
/// runtime.wasm whose ABI DOESN'T match the compiler links anyway (bare interface name) and corrupts
/// memory — surfacing as a cryptic "memory access out of bounds" / "unreachable" trap. That's the
/// stale-deployment failure mode (a Pages bundle or local pkg whose runtime predates a compiler change).
/// Comparing sha-256(bundled runtime) to `required_runtime_hash()` lets us report it as "stale, refresh".

/// Lowercase hex sha-256 of a digest (a Web Crypto `ArrayBuffer` result), matching the format
/// `required_runtime_hash()` returns (plain hex, no prefix).
export function hexDigest(digest: ArrayBuffer): string {
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

/// Rewrite a trap/error message when the runtime is KNOWN not to match the compiler (`matches === false`
/// — a real mismatch). When the runtime is verified-good (`true`) or the check was inconclusive (`null`,
/// e.g. no SubtleCrypto), return the original message unchanged — never cry wolf on a genuine user-code
/// trap. The mismatch text keeps the underlying error for debugging and tells the reader to hard-reload.
export function explainIfStaleRuntime(message: string, matches: boolean | null): string {
  if (matches === false) {
    return (
      "This looks like a stale build: the bundled Cadenza runtime doesn't match the compiler, so " +
      "running a compound value corrupts memory. Refresh the page (hard-reload) to pick up the current " +
      `build. (underlying error: ${message})`
    );
  }
  return message;
}
