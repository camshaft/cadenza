(do
  (decline WasmHostPeerResourceFusion
    (code UnsupportedConstruct)
    (reason "a host effect and a peer effect composed with a resource-escaping entrypoint")
    (doc "A host effect and a peer effect both composed with a resource-escaping entrypoint (#6163).")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the combined host-and-peer import-space emit alongside the resource escape")
      (ref pr 6163)))
  (decline WasmClosureTransformer
    (code UnsupportedConstruct)
    (reason "an export that both receives and returns a closure")
    (doc "An export that both receives and returns a closure — a closure transformer (#6216).")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the combined receive-and-return closure boundary emit (DESIGN-closure-host-resource-rcdzc.md)")
      (ref pr 6216)))
  (decline WasmCompoundResultWithClosureExport
    (code UnsupportedConstruct)
    (reason "a compound result alongside a closure export")
    (doc "A compound result alongside a closure (plain or round-trip) export (#6216).")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the compound-result host-boundary emit alongside a closure/round-trip-closure export")
      (ref pr 6216)))
  (decline WasmValueFormWalkerRecursive
    (code UnsupportedConstruct)
    (reason "a recursive-sum or runtime-collection value as a host-boundary result")
    (doc "A recursive-sum / runtime-collection value rendered as a host-boundary result (#6216).")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "a value-form walker that loops to a runtime-determined depth (folding to a scalar works)")
      (ref pr 6216)))
  (decline WasmBytesCrossingHostOpNoBoundaryForm
    (code UnsupportedConstruct)
    (reason "a host op whose signature has no host-boundary form")
    (doc "A bytes-crossing host op whose signature has no host-boundary form (#6216).")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the host compound-result ABI (e.g. option<list<u8>>)")
      (ref pr 6216)))
  (decline WasmMapPatternRuntimeMap
    (code UnsupportedConstruct)
    (reason "matching a map-pattern payload against a runtime map")
    (doc "Matching a map-pattern payload against a runtime map (#6216, select/dispatch).")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the per-binder runtime keyed-read for a map-pattern over a runtime map")
      (ref pr 6216)))
  (decline PrimAsValueNeedsClosure
    (code UnsupportedConstruct)
    (reason "a built-in operation used as a runtime value")
    (doc "A built-in operation used as a runtime value (would need a synthesized runtime closure). Coded CDZ0900 at its emit site (lower/compute.rs) by #6349.")
    (blocked-on
      (status blocked)
      (owner v-compiler-primitives)
      (needs "runtime-closure synthesis for a built-in used as a value")
      (ref pr 6349)))
  (decline TailResumptiveFoldUnhandledForm
    (code UnsupportedConstruct)
    (reason "an effect handler in a form the tail-resumptive fold does not specialize")
    (doc "An effect handler in a form the tail-resumptive fold does not specialize (cross-function / non-tail resume) — v-effects #6219.")
    (blocked-on
      (status blocked)
      (owner v-effects)
      (needs "the tail-resumptive fold to specialize a cross-function or non-tail resume")
      (ref pr 6219)))
  (decline MatchOverHeapCollectionScrutinee
    (code UnsupportedConstruct)
    (reason "matching over a heap-backed Set or Map scrutinee")
    (doc "Matching over a heap-backed Set/Map scrutinee — even a whole-value binder — needs a heap walk the compiler does not yet emit; surfaced by the dedup self-suppression fix (#6417).")
    (blocked-on
      (status blocked)
      (owner v-inference)
      (needs "match lowering that emits a heap walk over a Set/Map scrutinee (even a whole-value binder)")
      (ref pr 6417)))
  (decline RecursiveFunctionRuntimeSpecialization
    (code UnsupportedConstruct)
    (reason "a recursive function needs runtime specialization")
    (doc "A recursive function applied where it would need runtime specialization (the eval beta-reduction recursion guard, eval.rs); reworded off deferral wording to \"which is not supported\" by v-core-opt #6565.")
    (blocked-on
      (status blocked)
      (owner v-core-opt)
      (needs "runtime specialization of a recursive function")
      (ref pr 6565))))
