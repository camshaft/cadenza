# PR #2151 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — correctness/safety [VERIFIED, MED]

https://github.com/camshaft/cadenza/pull/2151 (HeapHandle slice-2b — value-heap Bytes build/read ops for
the reducer's Option<Bytes> fields). Copilot 1 inline. Same u32-truncation class as my #2109 (decode
graft) — one-side-validated (read_bytes validates, bytes_from doesn't).

## `bytes_from` allocates `data.len() as u32` (lossy truncation), then the loop writes ALL of `data` at `i as u32` indices → for a slice > u32::MAX, under-allocs then writes past the buffer / corrupts the guest value-heap (Copilot, wasm_host.rs:813) — correctness/safety [VERIFIED, MED]
> `bytes_from` truncates `data.len()` to `u32` via `as u32`. For slices larger than `u32::MAX`, this
> would allocate the wrong length and then write past the allocated buffer (or otherwise corrupt the
> guest value-heap). Convert the length with `u32::try_from` and return an error if it doesn't fit.

VERIFIED in the #2151 diff (two lossy conversions, compounding):
  ```
  pub fn bytes_from(&mut self, data: &[u8]) -> Result<u32, ComponentError> {
      let mut buf = self.bytes_alloc(data.len() as u32)?;   // diff:77 — TRUNCATES len
      for (i, &b) in data.iter().enumerate() {
          buf = self.bytes_set(buf, i as u32, b)?;          // diff:79 — i ALSO truncates
      }
  ```
For `data.len() > u32::MAX` (e.g. 2^32 + 5): `bytes_alloc` receives the TRUNCATED length (5) and allocs a
tiny buffer, but the loop iterates the FULL `data` (2^32+5 bytes) writing at `i as u32` — which wraps —
so `bytes_set` writes past the under-allocated buffer at wrapped indices → corrupts the guest value-heap
(and/or the `bytes_set` bounds-check fails late/inconsistently). MED: it requires a >4GiB single byte
slice, which is implausible for a reducer's `Option<Bytes>` field TODAY, but (a) `data` provenance is
host-marshalling of a value that could grow, (b) this is exactly the u32-truncation class the codebase
has been hardening (cf my #2109 decode-graft u32 truncation, #2093), and (c) silent `as u32` on a length
is a latent corruption seam, not a clean error. Note the ASYMMETRY: the read-dual `read_bytes` (diff:87-96)
correctly validates each byte with `u8::try_from` + Traps on a bad value — so the read side is defensive
while the build side truncates silently. Fix per Copilot: `let n = u32::try_from(data.len()).map_err(|_|
ComponentError::Trap("bytes_from: slice too large for u32 value-heap length"))?;` then `bytes_alloc(n)`;
the `i as u32` in the loop is then safe (i < n <= u32::MAX). v-agent-harness owns cdz-kernel/src. PR OPEN
→ foldable pre-merge. (Copilot bot reliable; source-verified.)
