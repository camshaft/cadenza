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

; The scan+generate is TYPE-AGNOSTIC across the scalar leaves: the accessor's result type is whatever the
; @param annotation declares (`(op name (-> Unit <Type>))`), so a Float64/Bool/… param generates a
; correctly-typed accessor with no per-type code. These pin the non-Int scalar leaves + the multi-param
; case (two @param sites → two accessor ops under ONE generated `Param` effect).

(case "an @param of a Float64 type generates a Float64-typed accessor"
  (doc    "The type-agnostic generate: `@param(widget: slider) ratio : Float64` makes the sidecar generate
           `(op ratio (-> Unit Float64))`, so the host value crosses as a Float64. With a host response of
           2.5, `main` returns 2.5. Pins that the accessor's result type follows the annotation for a
           non-Int scalar (Float64), not just Int64.")
  (input  (do
            (: (@ (param (: widget slider)) ratio) Float64)
            (def (main) (host (Param) (Param.ratio)))
            (export main)))
  (call   main)
  (host-responses (respond Param.ratio (: 2.5 Float64)))
  (output (: 2.5 Float64)))

(case "an @param of a Bool type generates a Bool-typed accessor"
  (doc    "The Bool leaf: `@param(widget: toggle) mirror : Bool` generates `(op mirror (-> Unit Bool))`, so
           the host supplies a Bool. With a host response of true, `main` returns true. Pins the Bool arm
           of the type-agnostic accessor generation.")
  (input  (do
            (: (@ (param (: widget toggle)) mirror) Bool)
            (def (main) (host (Param) (Param.mirror)))
            (export main)))
  (call   main)
  (host-responses (respond Param.mirror (: true Bool)))
  (output (: true Bool)))

(case "two @param sites generate two accessors under one Param effect"
  (doc    "The MULTI-param case: two `@param` sites (`w`, `h`) generate one `Param` effect with TWO
           accessor ops (`(op w …) (op h …)`), each host-bound independently. `(+ (Param.w) (Param.h))`
           with host responses 3 and 4 is 7. Pins that the sidecar collects ALL sites into a single
           generated effect (one effect, one op per param), not one effect per site.")
  (input  (do
            (: (@ (param (: widget slider)) w) Int64)
            (: (@ (param (: widget slider)) h) Int64)
            (def (main) (host (Param) (+ (Param.w) (Param.h))))
            (export main)))
  (call   main)
  (host-responses (respond Param.w (: 3 Int64))
                  (respond Param.h (: 4 Int64)))
  (output (: 7 Int64)))

; SCAN ROBUSTNESS: the config kv (widget/range/…) is OPTIONAL to the SCAN — the sidecar reads the param
; NAME + declared TYPE (which drive the generated accessor) and does not require any widget metadata to
; generate. A bare `(param)` (no config) still yields a typed accessor; the config only feeds the widget
; MANIFEST (a later brick), not the effect interface. Pins that a config-less @param is not rejected and
; still generates its accessor — the type is the load-bearing metadata, the widget is presentational.

(case "an @param with no widget config still generates its typed accessor"
  (doc    "The config kv is optional to the accessor generation: `(: (@ (param) width) Int64)` — a bare
           `(param)` with NO widget/range — still makes the sidecar generate `(op width (-> Unit Int64))`,
           so `(Param.width)` resolves + reads the host value (→ 5). Pins that the SCAN keys on the param
           name + declared type, not on the widget metadata (which only drives the later manifest).")
  (input  (do
            (: (@ (param) width) Int64)
            (def (main) (host (Param) (Param.width)))
            (export main)))
  (call   main)
  (host-responses (respond Param.width (: 5 Int64)))
  (output (: 5 Int64)))

; The realistic parametric shape: SEVERAL @param sites of DIFFERENT scalar types, all under the one
; generated `Param` effect, used together in real control flow (a CAD/notebook model reads a bool toggle,
; an int count, a float ratio). Pins that the sidecar generates a heterogeneous effect (ops of distinct
; result types) and each accessor host-binds independently within one `(host (Param) …)` delegation.

(case "mixed-type @param sites share one Param effect and drive control flow"
  (doc    "Three `@param`s of DIFFERENT types — `count : Int64`, `ratio : Float64`, `on : Bool` — generate
           one `Param` effect with three distinctly-typed accessor ops. The guest branches on `(Param.on)`
           and returns `(Param.count)`: with the host responding on=true, count=42, `main` returns 42. Pins
           the realistic parametric-model shape (several heterogeneous params under one delegation, used in
           control flow), beyond the same-type two-site case — each accessor is host-bound at its own type.")
  (input  (do
            (: (@ (param (: widget slider)) count) Int64)
            (: (@ (param (: widget slider)) ratio) Float64)
            (: (@ (param (: widget toggle)) on) Bool)
            (def (main) (host (Param) (if (Param.on) (Param.count) 0)))
            (export main)))
  (call   main)
  (host-responses (respond Param.on (: true Bool))
                  (respond Param.count (: 42 Int64)))
  (output (: 42 Int64)))
