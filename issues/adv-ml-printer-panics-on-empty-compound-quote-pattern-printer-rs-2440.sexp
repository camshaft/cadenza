; NEVER-PANIC INVARIANT BROKEN (breaker found, trunk 76ed1b0eb; corpus-bugfix verified 3b9fbe9ee):
; `cdz convert --to ml` PANICS 'entered unreachable code' at cadenza-syntax/src/printer.rs:2440 on an
; EMPTY-COMPOUND quote in PATTERN position. The program COMPILES + RUNS (wasm -> 1); only the ML PRINTER
; (pattern-position) panics. Isolation (breaker): expression-position (quote ()) prints fine as quote(#[]);
; a NON-empty quote pattern prints fine; only EMPTY + PATTERN hits the pattern printer's catch-all Atom
; assumption (a non-Atom empty node -> unreachable!). FIX: the pattern printer needs the empty-compound arm
; the EXPRESSION printer already has (v-syntax lane). ⚠ ORDERING HAZARD: the queued corpus pin 8091554bf
; ("rcdzc: pin the empty-compound quote pattern") adds EXACTLY this shape — if it lands BEFORE v-syntax's
; printer fix, xtask roundtrip + corpus_roundtrip go RED fleet-wide (breaker observed it on a discarded
; lineage: 2 cases 'round-trip via ml errored'). So the printer fix must land BEFORE or WITH 8091554bf.
; ROUTED to v-syntax (printer) by corpus-bugfix; pr-sync warned to hold 8091554bf ordering.
(do (def (main) (match (quote ()) ((quote ()) 1) (_ 0))) (export main))
