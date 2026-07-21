(do (def (main) (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple 10.41 (tuple true 3.4028235e38)))))) (export main))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk af0a646f7) — SPEC-SETTLED, routed v-rust-backend =====
; VERIFIED reproducing: wasm renders 3.4028235e38 as 340282350000000000000000000000000000000.0 (shortest
; round-tripping); rust run-rust as 340282349999999991754788743781432688640.0 (full binary expansion). Both
; parse to the SAME f64. CANONICAL = shortest round-tripping (SPEC: 12-metaprogramming:907/1100/1121/1155
; "shortest round-tripping decimal ... top of the exponent range"; reference-compiler.md:698/735 round-trip).
; So WASM is correct; the RUST value-renderer is the bug (emit shortest form via Grisu/Ryu, not exact
; expansion). NO concierge ask (spec settles it). Routed to v-rust-backend. Pin a top-of-exponent render case
; once fixed. Not a fix agent (their run-rust printer lane).
