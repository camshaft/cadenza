; BREAKER FINDING (2026-07-17): SPEC-vs-IMPLEMENTATION discrepancy, NOT a miscompile.
; spec/semantics/17-symbols.sexp:31 states "Symbols are EQUALITY-ONLY in this version: = and use as a
; map key, no ordering (< > …). A content-lexicographic order MAY be added additively by a later
; decision ... it is deliberately NOT pinned here." BUT the implementation ACCEPTS and evaluates all
; four ordering ops on Symbol-vs-Symbol, consistently on wasm + rust + rust-async (rust's String Ord =
; UTF-8 byte order matches wasm's byte-leaf order, incl. multibyte/emoji). No graded case pins
; Symbol<Symbol either way (only Symbol-vs-NUMBER < is pinned CDZ0201 at line 375). infer.rs:8018
; treats Symbol as a TEXT-like atom "comparable/equatable" so Symbol-vs-Symbol passes the comparability
; gate. Backends AGREE (sound, no miscompile) — the gap is the spec PROSE vs the accepted behavior.
; Disposition (corpus-bugfix / v-inference): either (a) prose is stale -> PIN Symbol ordering as
; content-lexicographic graded cases, or (b) frontend should REJECT Symbol<Symbol (CDZ0203) per prose.
; These probes all PASS today (documenting the accepted behavior):
(case "symbol lt is accepted and content-lexicographic (vs spec 'equality-only')"
  (doc "a<b true, b<a false — accepted despite 17-symbols.sexp:31 'no ordering'")
  (input (do (def (main) (< (Symbol.of "a") (Symbol.of "b"))) (export main)))
  (output (: true Bool)))
(case "symbol ordering agrees across backends on a multibyte pair"
  (doc "z (0x7A) < é (0xC3 0xA9) by byte order on all backends")
  (input (do (def (main) (< (Symbol.of "z") (Symbol.of "é"))) (export main)))
  (output (: true Bool)))
