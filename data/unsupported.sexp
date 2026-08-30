; data/unsupported.sexp — the auto-generated registry of every construct rcdzc declines to compile.
; GENERATED from the DeclineId catalog (rcdzc/src/diag.rs) by `cargo run -p xtask-codegen-unsupported`.
; The (code …) and (reason …) fields are DERIVED — do NOT hand-edit them (a `codegen --check` gate reds
; on drift). The (blocked-on …) block IS hand-authored (status/owner/needs/ref) and is PRESERVED across
; regenerations — that is where triage + routing-to-owning-lanes lives. Status: blocked | in-flight |
; permanent | design-gated | unowned. (Unsupported-error tracker, operator seq-286-broad.)
(do
  (unsupported wasm-host-peer-resource-fusion
    (code CDZ0900)
    (reason "a host effect and a peer effect composed with a resource-escaping entrypoint")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the combined host-and-peer import-space emit alongside the resource escape")
      (ref pr 6163)))
  (unsupported wasm-closure-transformer
    (code CDZ0900)
    (reason "an export that both receives and returns a closure")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the combined receive-and-return closure boundary emit (DESIGN-closure-host-resource-rcdzc.md)")
      (ref pr 6216)))
  (unsupported wasm-compound-result-with-closure-export
    (code CDZ0900)
    (reason "a compound result alongside a closure export")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the compound-result host-boundary emit alongside a closure/round-trip-closure export")
      (ref pr 6216)))
  (unsupported wasm-value-form-walker-recursive
    (code CDZ0900)
    (reason "a recursive-sum or runtime-collection value as a host-boundary result")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "a value-form walker that loops to a runtime-determined depth (folding to a scalar works)")
      (ref pr 6216)))
  (unsupported wasm-bytes-crossing-host-op-no-boundary-form
    (code CDZ0900)
    (reason "a host op whose signature has no host-boundary form")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the host compound-result ABI (e.g. option<list<u8>>)")
      (ref pr 6216)))
  (unsupported wasm-map-pattern-runtime-map
    (code CDZ0900)
    (reason "matching a map-pattern payload against a runtime map")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the per-binder runtime keyed-read for a map-pattern over a runtime map")
      (ref pr 6216)))
  (unsupported prim-as-value-needs-closure
    (code none)
    (reason "a built-in operation used as a runtime value")
    (blocked-on
      (status blocked)
      (owner v-compiler-primitives)
      (needs "runtime-closure synthesis for a built-in used as a value; the CDZ0900 coding flip at lower/compute.rs is gated on v-inference's dedup_faults fix")))
  (unsupported tail-resumptive-fold-unhandled-form
    (code CDZ0900)
    (reason "an effect handler in a form the tail-resumptive fold does not specialize")
    (blocked-on
      (status blocked)
      (owner v-effects)
      (needs "the tail-resumptive fold to specialize a cross-function or non-tail resume")
      (ref pr 6219)))
)
