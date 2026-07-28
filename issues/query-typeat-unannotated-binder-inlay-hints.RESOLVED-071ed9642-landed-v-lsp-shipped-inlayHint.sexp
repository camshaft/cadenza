;; QUERY GAP (v-inference-owned, flagged by v-lsp) — the `TypeAt{node}` sidecar query returns the DECLARED
;; type at a binder (absent → "unknown"), NOT the INFERRED/solved type. It resolves for an ANNOTATED param
;; binder and for USES of a resolved name, but an UN-ANNOTATED binder node returns "unknown" even when the
;; type is locally inferable. This blocks LSP inlayHint (inline inferred `param: <inferred>` hints — whose
;; whole point is annotating what the author did NOT write). v-lsp has binder enumeration/placement/range-
;; filtering done and parked on this one query.
;;
;; PROBES (v-lsp, on trunk via parse_surface + TypeAt per node):
;;   (def (f (: a Int64)) …)         TypeAt(a-binder) = Int64   ✓   (annotated → declared type)
;;   (def (f x) (+ x 1))             TypeAt(x-binder) = unknown ✗   (should infer a numeric type)
;;   (def (id x) x) with (id 5)      TypeAt(x-binder) = unknown ✗   (should be the solved param type)
;;   a let-bound name binder          TypeAt = unknown           ✗
;;
;; FIX DIRECTION (v-inference): TypeAt (or a new TypeOfBinder/InferredTypeAt) should return the SOLVED type
;; at an un-annotated binder — the monomorphic type where the def is used at exactly one instantiation, or
;; the generalized/constrained scheme var otherwise. Locus: the sidecar TypeAt handler (query.rs / the
;; type-query entry) — it currently reads the DECLARED annotation; make it fall through to `type_of` /
;; the binder's inferred slot when no annotation is present. Care: a GENERIC binder (`id`'s `x`) has no
;; single monomorphic type — decide the render (a scheme var `a`, or the sole-use instantiation). Co-design
;; the query shape with v-lsp. NOT urgent (inlayHint is the last LSP feature; rest of editor set complete).
(module m (def (f x) (+ x 1)) (def (main) (f 5)) (export main))
