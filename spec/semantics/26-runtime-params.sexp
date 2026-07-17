; Runtime parameters via `@param` annotation-driven codegen — DESIGN-runtime-parameter-host-effect.md
; (operator direction). A function/value marked `@param(widget: …, …) name : Type` is a RUNTIME INPUT the
; host supplies; a build-time SIDECAR (v-metaprogramming) scans every `@param` site and GENERATES a single
; strongly-typed effect `Param` with one accessor op per param (`Param.width : Int64`, …), and the host
; binds each accessor at run time (v-effects' host-effect mechanism). The `@param` annotation surface is
; v-syntax's; the scan + generate is v-metaprogramming's; the run-time bind is v-effects'.
;
; CANONICAL SHAPE (v-syntax): `@param(widget: slider, …) width : Type` parses to
;   (: (@ (param (: widget slider) …) width) Type)
; — the OUTER colon carries the explicit type, its inner is the `@`-annotation over the param name, and
; the `(param …)` application's tail is the config kv pairs.
;
; B-INVARIANT: `@param` MUST carry an explicit type — the generated accessor's result type IS the
; annotation type, so an un-typed `@param` has no accessor type (and would reintroduce a generate-order
; circularity, since the accessor is generated before resolve). An untyped `@param(…) name` is rejected.
;
; FIRST BRICK: a single SCALAR `@param` generates one `(op name (-> Unit Type))` accessor. The widget
; MANIFEST + the Quantity (num/den) host ABI are later increments; these cases pin the core scan+generate
; contract — a `@param` site makes `Param.<name>` a host-delegated accessor of the annotated type.

(case "an @param site generates a Param accessor a host delegation reads at run time"
  (doc    "The core contract: `@param(widget: slider) width : Int64` — parsed to `(: (@ (param (: widget
           slider)) width) Int64)` — makes the sidecar GENERATE `(effect Param (op width (-> Unit
           Int64)))`. So a guest `(host (Param) (Param.width))` resolves `Param.width` to the generated
           accessor, performs it as a host call, and reads the host-supplied value. With the host
           responding 7, `main` returns 7. Pins that a @param site alone (no hand-written effect) makes
           its accessor a typed host-delegated op — the scan+generate the sidecar performs.")
  (input  (do
            (: (@ (param (: widget slider)) width) Int64)
            (def (main) (host (Param) (Param.width)))
            (export main)))
  (call   main)
  (host-responses (respond Param.width (: 7 Int64)))
  (output (: 7 Int64)))

(case "the generated Param accessor carries the @param's declared type into arithmetic"
  (doc    "The accessor is STRONGLY TYPED by the annotation, not `get(String) -> T`: `Param.width : Int64`,
           so the host value flows into ordinary Int64 arithmetic. `(+ (Param.width) 1)` with a host
           response of 41 is 42 — the accessor's result is an Int64 the `+` accepts. Pins that the
           generated op's result type is the @param's declared type (the accessor is monomorphic in the
           right type, so no runtime type check / no stringly-typed get).")
  (input  (do
            (: (@ (param (: widget number)) base) Int64)
            (def (main) (host (Param) (+ (Param.base) 1)))
            (export main)))
  (call   main)
  (host-responses (respond Param.base (: 41 Int64)))
  (output (: 42 Int64)))
