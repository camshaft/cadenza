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
      (ref pr 6565)))
  (decline NestedRecordFieldPatternDescent
    (code UnsupportedConstruct)
    (reason "a nested compound sub-pattern below a record field")
    (doc "A deeper compound sub-pattern below a record field — a record match pattern (resolve.rs match_arm_record_binds), a tuple/list/constructor-nested record match (resolve.rs, reclassified CDZ0201->CDZ0900 by #6850), or a record binding pattern (resolve.rs + lower/match_tree.rs, #6800/#6838). v-ast-compound is building PathStep::Field (name-keyed record-field descent) to make most such cases BIND; whatever still declines is tagged declined(id) in that build PR.")
    (blocked-on
      (status blocked)
      (owner v-ast-compound)
      (needs "PathStep::Field name-keyed record-field descent so a nested compound sub-pattern under a record field binds")
      (ref pr 6850)))
  (decline WasmClosureBoundaryNoRepr
    (code UnsupportedConstruct)
    (reason "a closure's param, result, or capture type has no machine representation")
    (doc "A closure crossing the host boundary whose parameter, result, or capture type has no machine representation — one family over the 3 sibling diag.rs declines CLOSURE_PARAM/RESULT/CAPTURE_NO_REPR. A fully-typed closure crosses via the host-closure resource; these are the un-built frontier (e.g. a bare `(fn (v1) v1)` in a list whose v1 solves to Any — infer recovers the param type from an enclosing higher-order arrow when it can, this face cannot). Fuzzer-surfaced (v-cdz-smith reachability sweep #6878, faces #1/#6); classified feature-gap by v-rust-backend. NUANCE: the pure-Any param subcase borders the CDZ0203 annotate-it undetermined-type reject — kept as a closure-boundary family tag; the operator may reclassify the pure-Any face to a coded CDZ0203 reject later.")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "the host-closure-resource boundary emit for a closure whose param/result/capture type has no scalar machine representation")
      (ref pr 6878)))
  (decline WasmHeapReturnParamNoBoundaryRep
    (code UnsupportedConstruct)
    (reason "a parameterized heap-return export forwards scalar params only; this param type has no boundary representation")
    (doc "A parameterized heap-return export (the make(a…)->own<t> resource-escape path) forwards scalar and fixed-shape scalar tuple/record params only; a String/Bytes/list param has no boundary representation on this path. The mem-leaf param lift that would forward it exists for the typed-interface member route (#6624/#6639) but is not wired on the bare resource-escape path. Fuzzer-surfaced (#6878 face #3); classified feature-gap (buildable-next) by v-rust-backend. Emit site backend/wasm/mod.rs:9277.")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "wire the mem-leaf param lift on the bare resource-escape heap-return export path (exists for the typed-interface member route #6624/#6639)")
      (ref pr 6624))))
