# PR review comments — mirrored from GitHub PR #404 (Copilot inline)

- **PR:** #404 (MERGED)
- **File:** `implementation/seed/crates/cdz-runtime/src/lib.rs` (two doc comments, @3145 and @3226)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591033034, 3591033066
- **Links:** https://github.com/camshaft/cadenza/pull/404#discussion_r3591033034 , #discussion_r3591033066

## Comments (verbatim)
> The doc comment says `op_str_from_bytes` is WIT-exported at index 83, but the WIT interface documents `str-from-bytes` at index 85. Keeping the index correct matters because the runtime WIT order is a frozen contract.
>
> This comment says `op_str_from_bytes` is WIT-exported at index 83, but `runtime.wit` documents `str-from-bytes` as index 85. This should be kept in sync with the frozen runtime ABI numbering.

## Liaison triage — CONFIRMED against trunk
Confirmed: `cdz-runtime/wit/runtime.wit:398` declares `str-from-bytes: func(buf: u32) -> u32; // 85`
(and the section header at :386 says "index 85"), but TWO doc comments in `cdz-runtime/src/lib.rs`
(~3145 and ~3226) say "WIT-EXPORTED at index 83". The comments are simply WRONG about the index (the
real index is 85; 83 was bigint-of-bytes). Doc-only mismatch, but in the frozen-ABI area.

⚠️ IMPLEMENTATION NOTE for the owner: `cdz-runtime`'s `//` comments are INSIDE the frozen
`REQUIRED_RUNTIME_HASH` — editing them changes the hash, so this fix requires a runtime rebuild
(`cargo xtask build`) + `codegen --check`, not a bare comment edit. v-runtime owns cdz-runtime. Fix the
two comments to say index 85. Fix on `trunk`. Quotes + links in queue file.
