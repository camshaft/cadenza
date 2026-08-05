# PR #2148 review — cadenza-ast/src/codec.rs (v-syntax) — OPEN — maintainability [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2148 (tighten the per-dict-skip test — parse the import section,
not a blob scan; folds #2131 review). Copilot 1 inline (repeated at 2 sites).

## `parse_transport_imports` hardcodes `[u8; 32]` in its return type while using the `HASH_LEN` constant internally → a future `HASH_LEN` change won't compile (array-length mismatch) (Copilot, codec.rs:1955 & :2478) — maintainability [VERIFIED, LOW]
> `parse_transport_imports` hardcodes `[u8; 32]` in its return type while using the `HASH_LEN` constant
> internally. If `HASH_LEN` ever changes, this helper will fail to compile due to the array-length
> mismatch; tying the signature to `HASH_LEN` keeps the helper consistent with the transport format
> constant.

VERIFIED: the test helper is `fn parse_transport_imports(bytes: &[u8]) -> Vec<[u8; 32]>` (#2148 diff:12)
but internally uses `r.take(HASH_LEN)` (diff:18) and `let mut h = [0u8; HASH_LEN]` (diff:19). `HASH_LEN`
is `const HASH_LEN: usize = 32` (codec.rs:181). So today `[u8; 32]` == `[u8; HASH_LEN]` and it compiles;
but if `HASH_LEN` ever changed (a wider transport hash), the `[0u8; HASH_LEN]` body wouldn't fit the
`Vec<[u8; 32]>` return → compile error, and the "single source of truth for the transport hash width" that
`HASH_LEN` exists to be is silently broken at this boundary. LOW/maintainability (test-only helper, no
runtime bug; a hash-width change is unlikely but the constant exists precisely so sites don't hardcode
32). Fix per Copilot: return `Vec<[u8; HASH_LEN]>` to tie the signature to the constant. v-syntax owns the
codec.rs wire (per the dict.rs ownership split — codec/wire = v-syntax). PR OPEN → foldable pre-merge.
