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
    (reason "a variant sub-pattern below a record field")
    (doc "A refutable VARIANT sub-pattern below a record field (#record((= x (Some c)))), in a match OR a binding pattern — the sole remaining residual. v-ast-compound BUILT everything else: the POSITIONAL Elem-reachable case (#6890 match + #6911 binding) AND the RECORD/tuple/list-below-a-field case (#6944, name-keyed RecordSubStep::Field, both faces) — those all BIND now. What STILL declines (this id): only a VARIANT below a record field, which is REFUTABLE and needs the match-arm switch-lowering path (a separate increment). Emit sites: binding — resolve.rs last_binder_named (has_variant→declined) + lower/match_tree.rs check_binding_pattern; match — resolve.rs match_arm_record_binds Unwireable + Case-6rec-nested skips Payload → Case-6rec-nested-decline.")
    (blocked-on
      (status blocked)
      (owner v-ast-compound)
      (needs "the match-arm switch-lowering path for a refutable variant sub-pattern below a record field (the positional + record/tuple/list-below-field cases are built — #6890/#6911/#6944)")
      (ref pr 6944)))
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
      (ref pr 6624)))
  (decline WasmParameterizedHeapReturnNoValueEncode
    (code UnsupportedConstruct)
    (reason "a parameterized (non-nullary) export cannot return this heap type; its value form is emitted only for a nullary constant export and it has no runtime value-encode render")
    (doc "A parameterized (non-nullary) export whose scalar params are fine but whose heap RESULT (List/Option/Result/Tuple/Symbol/...) reached the boundary: the value form is emitted only for a nullary constant-bake export, and a runtime value-encode render for it is not available. The dominant reachable heap-return decline (v-cdz-smith #9183 reachability census; the ~18 return-type variants are one logical gap). Classified feature-gap by v-rust-backend. Emit site backend/wasm/mod.rs:1205.")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "a runtime value-encode render for a heap return type on the parameterized-export path (the nullary constant-bake path has it)")
      (ref pr 9183)))
  (decline WasmHostOpNoBoundaryFormOnBareEffect
    (code UnsupportedConstruct)
    (reason "a host operation whose argument or result type has no component boundary form on a bare effect")
    (doc "A bare (effect ...) host operation whose argument or result type has no component boundary form. On a bare effect, host results cross as a scalar/unit and arguments as a scalar/unit/string or a Bytes (list<u8>); a Bytes/option<list<u8>> RESULT crosses only on the WORLD-DRIVEN (wit-world) path, and a record/compound argument or a list<list<u8>>/list<tuple> result is not supported on a bare effect. THE dominant reachable decline (v-cdz-smith #9183 census: ~17.7k hits across the result and argument faces = one emit site backend/wasm/mod.rs:211, via host::first_unrepresentable_host_op). Distinct from WasmBytesCrossingHostOpNoBoundaryForm (the narrower bytes-crossing-member site). Classified feature-gap by v-rust-backend.")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "a host-boundary form for a compound/record argument or a compound-list result on a bare effect, or route it through the wit-world path")
      (ref pr 9183)))
  (decline WasmMultiHostEffectDelegation
    (code UnsupportedConstruct)
    (reason "delegating more than one host effect from one entrypoint (one interface per envelope)")
    (doc "An entrypoint delegating more than one distinct host effect. The host-delegation emit binds one component interface per envelope, so two distinct effect names cannot both be delegated. One logical gap over 6 emit sites in backend/wasm/mod.rs: the bare host-delegating path, four resource-escaping-entrypoint variants, and the closure-export variant. Reachable via the bare path (v-cdz-smith #9183 census, ~216 hits). Classified feature-gap by v-rust-backend.")
    (blocked-on
      (status blocked)
      (owner v-rust-backend)
      (needs "multi-interface host delegation (more than one host effect per envelope)")
      (ref pr 9183))))
