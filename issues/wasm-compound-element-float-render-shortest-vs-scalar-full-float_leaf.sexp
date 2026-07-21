; LIVE wasm float-render inconsistency (corpus-bugfix, trunk 0eb020e6d, 2026-07-21; fuzzer-isolated scalar-vs-compound).
; ROUTED to v-runtime (KIND_FLOAT heap-node encoding is their lane).
; RULING (pr-sync/concierge, 2026-07-21): (a) FULL-EXPANSION is canonical (converging the deviating
; wasm compound float_leaf path TO the established canon = bugfix, no operator needed). v-runtime owns the
; convergence + the large-significand KIND_FLOAT doc-codec fix (u128 significand overflow >16 bytes). corpus-
; bugfix HOLDS the compound pins until that codec fix lands, then pins compound top-of-exponent (tuple/list/
; Option) + a large-significand (f64::MAX-ish) round-trip case across all 3 backends.
;
; BLAST RADIUS (sharpened 2026-07-21, trunk 0eb020e6d): the divergence hits EVERY compound element
; position — (tuple x 1.0), (list x), nested (tuple (tuple x)), (Some x) sum payload — anything boxed as a
; KIND_FLOAT heap node; only the bare-scalar top-level result agrees (it uses cdz-run display_float, not the
; heap path). TRIGGER MAGNITUDE: only when shortest-round-tripping decimal != full binary64 expansion; short
; values (0.1, 3.14) AND whole powers (1e19, 1e21, whose shortest already == full) AGREE. Top-of-exponent
; floats (3.4028235e38) are the clean witnesses. One box/encode fix (float_leaf) corrects all positions.
;
; THE BUG: a Float64 renders DIFFERENTLY depending on whether it is a bare scalar or a COMPOUND ELEMENT,
; and the compound form also DIVERGES from rust.
;   SCALAR   (do (def (main) 3.4028235e38) ...)            => wasm 340282349999999991754788743781432688640.0
;                                                              rust 340282349999999991754788743781432688640.0   AGREE (full expansion)
;   COMPOUND (do (def (main) (tuple 3.4028235e38 1.0)) ...) => wasm (tuple 340282350000000000000000000000000000000.0 1.0)   SHORTEST
;                                                              rust (tuple 340282349999999991754788743781432688640.0 1.0)  FULL     DIVERGE
;
; ROOT CAUSE (traced): two distinct wasm float renderers.
;   - SCALAR path: cdz-run/src/render.rs::display_float -> format!("{f:.0}.0") = FULL expansion.
;   - COMPOUND ELEMENT: a heap value rendered via cdz-runtime/src/lib.rs::float_leaf (line ~1919) ->
;     format!("{f:e}") = SHORTEST round-tripping, stored in the KIND_FLOAT node.
;   Rust renders FULL everywhere. Canonical = FULL (scalar + rust + the landed 01-literals top-of-exponent
;   pin + the 1e19 precedent all emit full expansion), so float_leaf's shortest-form is the OUTLIER.
;
; FIX (v-runtime): make float_leaf's KIND_FLOAT decimal encoding match the scalar/display_float FULL-expansion
; form for whole-magnitude floats (or, if shortest IS intended canonical, converge BOTH paths + rust + re-pin
; the scalar case — but that contradicts the already-landed 01-literals pin, so full is the settled canonical).
; Once converged, corpus-bugfix will pin a COMPOUND top-of-exponent case (tuple/list element) across all 3 backends.
;
; This case is written to EXPECT the canonical full-expansion compound render; it will grade `todo`/`fail`
; on wasm until float_leaf is fixed (rust already renders full).
(case "a top-of-exponent float renders its full expansion as a TUPLE element (compound render matches scalar)"
  (doc  "The compound-element companion of the scalar top-of-exponent pin (01-literals). A Float64 at the top
         of the binary64 exponent range, as a tuple element, must render its FULL decimal expansion — the SAME
         form the scalar path and rust emit — not the shortest round-tripping form. Guards against the wasm
         KIND_FLOAT (float_leaf) renderer diverging from the scalar display_float path.")
  (input  (do (def (main) (tuple 3.4028235e38 1.0)) (export main)))
  (output (: (tuple 340282349999999991754788743781432688640.0 1.0) (Tuple Float64 Float64))))
