; Runtime parameters via `@!param` module-directive-driven codegen — DESIGN-runtime-parameter-host-effect.md
; (operator direction). A module marked `@!param(widget: …, …) name : Type` declares a RUNTIME INPUT the
; host supplies; a build-time SIDECAR (v-metaprogramming) scans every `@!param` site and GENERATES a single
; strongly-typed effect `Param` with one accessor op per param (`Param.width : Int64`, …), and the host
; binds each accessor at run time (v-effects' host-effect mechanism). The `@!param` directive surface is
; v-syntax's; the scan + generate is v-metaprogramming's; the run-time bind is v-effects'.
;
; SIGIL (operator ruling 2026-07-18): `@!param` uses the `@!` MODULE-directive sigil (like `@!default-
; fraction`), NOT the following-form `@` — a runtime parameter parameterizes the whole MODULE, not one form.
; CANONICAL SHAPE (v-syntax): `@!param(widget: slider, …) width : Type` parses to
;   (pragma param (param (: widget slider) …) (: width Type))
; — a `pragma` head (module-attached), a `(param <kv>…)` config group, and a `(: name Type)` binder.
;
; B-INVARIANT: `@!param` MUST carry an explicit type — the generated accessor's result type IS the declared
; type, so an un-typed `@!param` has no accessor type (and would reintroduce a generate-order circularity,
; since the accessor is generated before resolve). An untyped `@!param(…) name` (a bare-name binder) is
; rejected by the pragma-registry `param` arm (CDZ0602 — a malformed module directive).
;
; FIRST BRICK: a single SCALAR `@!param` generates one `(op name (-> Unit Type))` accessor. The widget
; MANIFEST + the Quantity (num/den) host ABI are later increments; these cases pin the core scan+generate
; contract — a `@!param` site makes `Param.<name>` a host-delegated accessor of the annotated type.
(diagnostic-quality)

(case
  "an @param site generates a Param accessor a host delegation reads at run time"
  (doc
    "The core contract: `@param(widget: slider) width : Int64` — parsed to `(: (@ (param (: widget
           slider)) width) Int64)` — makes the sidecar GENERATE `(effect Param (op width (-> Unit
           Int64)))`. So a guest `(host (Param) (Param.width))` resolves `Param.width` to the generated
           accessor, performs it as a host call, and reads the host-supplied value. With the host
           responding 7, `main` returns 7. Pins that a @param site alone (no hand-written effect) makes
           its accessor a typed host-delegated op — the scan+generate the sidecar performs.")
  (input
    (do
      (pragma param (param (: widget slider)) (: width Int64))
      (def (main) (host (Param) (Param.width)))
      (export main)))
  (call main)
  (host-responses (respond Param.width (: 7 Int64)))
  (output (: 7 Int64)))

(case
  "the generated Param accessor carries the @param's declared type into arithmetic"
  (doc
    "The accessor is STRONGLY TYPED by the annotation, not `get(String) -> T`: `Param.width : Int64`,
           so the host value flows into ordinary Int64 arithmetic. `(+ (Param.width) 1)` with a host
           response of 41 is 42 — the accessor's result is an Int64 the `+` accepts. Pins that the
           generated op's result type is the @param's declared type (the accessor is monomorphic in the
           right type, so no runtime type check / no stringly-typed get).")
  (input
    (do
      (pragma param (param (: widget number)) (: base Int64))
      (def (main) (host (Param) (+ (Param.base) 1)))
      (export main)))
  (call main)
  (host-responses (respond Param.base (: 41 Int64)))
  (output (: 42 Int64)))

; The scan+generate is TYPE-AGNOSTIC across the scalar leaves: the accessor's result type is whatever the
; @param annotation declares (`(op name (-> Unit <Type>))`), so a Float64/Bool/… param generates a
; correctly-typed accessor with no per-type code. These pin the non-Int scalar leaves + the multi-param
; case (two @param sites → two accessor ops under ONE generated `Param` effect).
(case
  "an @param of a Float64 type generates a Float64-typed accessor"
  (doc
    "The type-agnostic generate: `@param(widget: slider) ratio : Float64` makes the sidecar generate
           `(op ratio (-> Unit Float64))`, so the host value crosses as a Float64. With a host response of
           2.5, `main` returns 2.5. Pins that the accessor's result type follows the annotation for a
           non-Int scalar (Float64), not just Int64.")
  (input
    (do
      (pragma param (param (: widget slider)) (: ratio Float64))
      (def (main) (host (Param) (Param.ratio)))
      (export main)))
  (call main)
  (host-responses (respond Param.ratio (: 2.5 Float64)))
  (output (: 2.5 Float64)))

(case
  "an @param of a Bool type generates a Bool-typed accessor"
  (doc
    "The Bool leaf: `@param(widget: toggle) mirror : Bool` generates `(op mirror (-> Unit Bool))`, so
           the host supplies a Bool. With a host response of true, `main` returns true. Pins the Bool arm
           of the type-agnostic accessor generation.")
  (input
    (do
      (pragma param (param (: widget toggle)) (: mirror Bool))
      (def (main) (host (Param) (Param.mirror)))
      (export main)))
  (call main)
  (host-responses (respond Param.mirror (: true Bool)))
  (output (: true Bool)))

(case
  "two @param sites generate two accessors under one Param effect"
  (doc
    "The MULTI-param case: two `@param` sites (`w`, `h`) generate one `Param` effect with TWO
           accessor ops (`(op w …) (op h …)`), each host-bound independently. `(+ (Param.w) (Param.h))`
           with host responses 3 and 4 is 7. Pins that the sidecar collects ALL sites into a single
           generated effect (one effect, one op per param), not one effect per site.")
  (input
    (do
      (pragma param (param (: widget slider)) (: w Int64))
      (pragma param (param (: widget slider)) (: h Int64))
      (def (main) (host (Param) (+ (Param.w) (Param.h))))
      (export main)))
  (call main)
  (host-responses (respond Param.w (: 3 Int64)) (respond Param.h (: 4 Int64)))
  (output (: 7 Int64)))

; DUPLICATE-NAME SOUNDNESS: two `@param` sites with the SAME name both generate an `(op width …)` under the
; ONE generated `Param` effect — i.e. a duplicate operation name in an effect. That is rejected by the
; ordinary front-end effect check (an effect has a fixed set of operation names) as CDZ0201, exactly as a
; hand-written effect with two same-named ops is. This PINS that the sidecar does NOT silently dedup / last-
; wins a collision (which would drop a param the host expects to bind, or emit an invalid module) — a
; duplicate @param name is a clean compile error, never a silent miscompile. The generate is a plain splice
; into an ordinary effect, so it composes with the effect-declaration check for free; this case guards that
; composition against a future `generate` change (e.g. one that deduped names) reintroducing the hazard.
(case
  "two @param sites with the same name are rejected as a duplicate effect operation"
  (doc
    "Two `@param` sites both named `width` each generate an `(op width …)` under the single generated
           `Param` effect — a duplicate operation name. The front-end effect check rejects it as CDZ0201
           (an effect has a fixed set of operation names), the SAME reject a hand-written effect with two
           same-named ops gets. Pins that the sidecar surfaces a name collision as a clean compile error
           rather than silently deduping (dropping a param) or emitting an invalid module.")
  (input
    (do
      (pragma param (param (: widget slider)) (: width Int64))
      (pragma param (param (: widget stepper)) (: width Int64))
      (def (main) (host (Param) (Param.width)))
      (export main)))
  (error CDZ0201))

; SCAN ROBUSTNESS: the config kv (widget/range/…) is OPTIONAL to the SCAN — the sidecar reads the param
; NAME + declared TYPE (which drive the generated accessor) and does not require any widget metadata to
; generate. A bare `(param)` (no config) still yields a typed accessor; the config only feeds the widget
; MANIFEST (a later brick), not the effect interface. Pins that a config-less @param is not rejected and
; still generates its accessor — the type is the load-bearing metadata, the widget is presentational.
(case
  "an @param with no widget config still generates its typed accessor"
  (doc
    "The config kv is optional to the accessor generation: `(pragma param (param) (: width Int64))` — a bare
           `(param)` with NO widget/range — still makes the sidecar generate `(op width (-> Unit Int64))`,
           so `(Param.width)` resolves + reads the host value (→ 5). Pins that the SCAN keys on the param
           name + declared type, not on the widget metadata (which only drives the later manifest).")
  (input
    (do
      (pragma param (param) (: width Int64))
      (def (main) (host (Param) (Param.width)))
      (export main)))
  (call main)
  (host-responses (respond Param.width (: 5 Int64)))
  (output (: 5 Int64)))

; The realistic parametric shape: SEVERAL @param sites of DIFFERENT scalar types, all under the one
; generated `Param` effect, used together in real control flow (a CAD/notebook model reads a bool toggle,
; an int count, a float ratio). Pins that the sidecar generates a heterogeneous effect (ops of distinct
; result types) and each accessor host-binds independently within one `(host (Param) …)` delegation.
(case
  "mixed-type @param sites share one Param effect and drive control flow"
  (doc
    "Three `@param`s of DIFFERENT types — `count : Int64`, `ratio : Float64`, `on : Bool` — generate
           one `Param` effect with three distinctly-typed accessor ops. The guest branches on `(Param.on)`
           and returns `(Param.count)`: with the host responding on=true, count=42, `main` returns 42. Pins
           the realistic parametric-model shape (several heterogeneous params under one delegation, used in
           control flow), beyond the same-type two-site case — each accessor is host-bound at its own type.")
  (input
    (do
      (pragma param (param (: widget slider)) (: count Int64))
      (pragma param (param (: widget slider)) (: ratio Float64))
      (pragma param (param (: widget toggle)) (: on Bool))
      (def (main) (host (Param) (if (Param.on) (Param.count) 0)))
      (export main)))
  (call main)
  (host-responses (respond Param.on (: true Bool)) (respond Param.count (: 42 Int64)))
  (output (: 42 Int64)))

; The QUANTITY @param (the v-cad/v-notebook driving case) — now unblocked by v-effects' Quantity-host-op ABI
; (layers 1+2: op-result-type resolution for a Qty + a scalar-inner Qty crossing as its inner scalar, unit
; erased). A `@param(...) width : (Qty Int64 <unit>)` generates a Qty-result accessor; the host supplies the
; magnitude as the inner scalar, the guest's declared (Qty …) type carries the unit, and `Qty.value` reads
; the magnitude back. This is the runtime-parameter path a parametric CAD dimension / notebook length widget
; drives — the sidecar's type-agnostic generate + v-effects' Qty boundary compose end-to-end.
(case
  "a Quantity @param generates a Qty-result accessor the host supplies as a scalar magnitude"
  (doc
    "The Quantity runtime parameter: `@param(widget: slider) width : (Qty Int64 (Unit.base #\"meter\"))`
           makes the sidecar generate `(op width (-> Unit (Qty Int64 meter)))`. The unit is a compile-time
           value erased before codegen (Ty::Qty has its inner's runtime rep), so the host supplies the
           magnitude as the inner Int64; the guest's declared Qty type carries the unit, and `Qty.value`
           reads the magnitude back — with the host responding 42, `main` returns 42. Pins the sidecar's
           type-agnostic generate composing with v-effects' Quantity-host-op ABI end-to-end (the v-cad
           parametric-dimension / v-notebook length-widget driving case).")
  (input
    (do
      (pragma param (param (: widget slider)) (: width (Qty Int64 (Unit.base #"meter"))))
      (def (main) (host (Param) (Qty.value (Param.width))))
      (export main)))
  (call main)
  (host-responses (respond Param.width (: 42 Int64)))
  (output (: 42 Int64)))

; PLACEMENT: a `@!param` is a TOP-LEVEL module directive (like `@!default-fraction`) — it parameterizes the
; whole MODULE, so it is well-placed ONLY as a direct top-level member of the program root. A nested
; `(pragma param …)` (inside a def body / a `(do …)` value position) is MISPLACED: v-syntax confirmed the
; parser does no placement enforcement (it parses a pragma identically at any depth), so the judgment is a
; compile-time semantic one owned by rcdzc's pragma pass (the same pass that owns the `param` registry arm).
; The guard reports it as CDZ0602 (a misplaced module directive) — the PRIMARY fault — and the sidecar scans
; (generate + manifest) skip a nested pragma so it declares no accessor and surfaces no widget. (The OLD
; `@param`-annotation placement reject — CDZ0201 on a misplaced `(@ …)` — no longer applies now that
; `@!param` is a pragma, not a following-form annotation.)
(case
  "a nested @!param is a misplaced module directive (CDZ0602), not a module parameter"
  (doc
    "`@!param` is a MODULE directive — well-placed only as a direct top-level member of the program
           root. A `(pragma param …)` nested inside a definition's body (a `do`-block value position) is
           misplaced: it is not a module directive there. rcdzc's pragma pass reports the placement as
           CDZ0602 (the primary fault), rather than letting the nested pragma's config names raise only a
           confusing CDZ0101 unbound cascade; and the sidecar scans (generate + manifest) skip it so a buried
           pragma declares no accessor / surfaces no widget. Pins the placement guard v-syntax routed to this
           crate (the parser parses a pragma identically at any depth; placement is a compile-time judgment).")
  (input
    (do
      (def (helper) (do (pragma param (param (: widget slider)) (: width Int64)) 5))
      (def (main) 0)
      (export main)))
  (error CDZ0602))

(case
  "a @param accessor splices into a quasiquote and the eval computes with the host value"
  (doc
    "Composition of the @param sidecar with quasiquote metaprogramming: the generated `Param.width`
           accessor is unquoted into a template `(+ ,(Param.width) 1)`, reified to an AST, and `eval`
           executes it. With the host responding 41, the spliced value is 41 and the eval computes 42 —
           pins that a host-delegated @param accessor composes as an ordinary runtime Int64 inside a
           quasiquote/eval, the metaprog×runtime-param interaction.")
  (input
    (do
      (pragma param (param (: widget slider)) (: width Int64))
      (def (main) (host (Param) (eval (quasiquote (+ (unquote (Param.width)) 1)))))
      (export main)))
  (call main)
  (host-responses (respond Param.width (: 41 Int64)))
  (output (: 42 Int64)))

(case
  "a @param value drives a CHAMP map lookup as the KEY"
  (doc
    "Composition of the @param sidecar with the collection machinery: the host-supplied `Param.k`
           value is used as a MAP KEY — `(Map.lookup {1↦10, 2↦20} (Param.k))` with the host responding 2
           finds 20. The accessor's runtime Int64 must flow into the CHAMP hash/lookup path exactly as a
           boundary parameter does (the param×collection interaction — the sidecar's accessors are ordinary
           values, not a special class). The collection companion of the quasiquote/eval composition above.")
  (input
    (do
      (pragma param (param (: widget slider)) (: k Int64))
      (def
        (main)
        (host
          (Param)
          (match
            (Map.lookup (Map.insert (Map.insert Map.empty 1 10) 2 20) (Param.k))
            ((Some v) v)
            ((None u) -1))))
      (export main)))
  (call main)
  (host-responses (respond Param.k (: 2 Int64)))
  (output (: 20 Int64)))

(case
  "one @param read bound ONCE fans out to a map key, a set probe, and arithmetic"
  (doc
    "The read-once-use-many discipline: ONE (Param.k) accessor perform bound in a do-def, then
           fanned to a map KEY, a Set membership probe, and arithmetic. host-responses carries exactly
           ONE respond, so a lowering that re-performed the accessor per use (double-billing the host)
           diverges rather than silently passing. k=2: map hit 20 + in-set 100 + 2000 = 2120.")
  (input
    (do
      (pragma param (param (: widget slider)) (: k Int64))
      (def
        (main)
        (host
          (Param)
          (do
            (def k (Param.k))
            (def
              v
              (match
                (Map.lookup (Map.insert (Map.insert Map.empty 1 10) 2 20) k)
                ((Some x) x)
                ((None _u) -1)))
            (def inn (if (Set.contains #set(2 5) k) 1 0))
            (+ v (+ (* inn 100) (* k 1000))))))
      (export main)))
  (call main)
  (host-responses (respond Param.k (: 2 Int64)))
  (output (: 2120 Int64)))

(case
  "a @param of type Rational desugars to two scalar num/den accessors the guest recombines (#13)"
  (doc
    "A heap `Rational` has no host boundary form (only scalar/unit results cross), so a
           `@param(...) rate : Rational` cannot generate one `(op rate (-> Unit Rational))` host accessor.
           The sidecar desugars it to v-effects' num/den ABI (#13, 14-effects): GENERATE two scalar
           `Int64` accessors `rate-num`/`rate-den`, and REWRITE each `(Param.rate)` use to `(Rational.of
           (Param.rate-num) (Param.rate-den))` so the guest recombines the exact rational from the two
           host-supplied scalars. With the host responding num=7, den=2, `main` builds 7/2 (normalized).
           Pins that a Rational-typed @param is expressible over the fully-supported scalar host path — the
           heap-typed @param frontier, closed by desugaring to the operator-ruled minimal #13 boundary.")
  (input
    (do
      (pragma param (param (: widget slider)) (: rate Rational))
      (def (main) (host (Param) (Param.rate)))
      (export main)))
  (call main)
  (host-responses (respond Param.rate-num (: 7 Int64)) (respond Param.rate-den (: 2 Int64)))
  (output (: 7/2 Rational))
  (live-objects known-leak))

(case
  "a @param of type (Qty Rational unit) desugars to num/den scalars + a guest Qty.of with the unit (#13 B2)"
  (doc
    "The Length layer: a `@param(...) len : (Qty Rational (Unit.base #\"meter\"))` is a Rational-
           MAGNITUDE quantity — the actual `@param : Length` shape (v-effects #13 B2). A heap Rational has
           no host boundary form, so the magnitude crosses as the SAME two scalar `Int64` num/den accessors
           as a bare Rational (`len-num`/`len-den`); the guest recombines them with `Rational.of` and the
           sidecar RE-ATTACHES the unit guest-side via `Qty.of(…, (Unit.base #\"meter\"))` — the unit is a
           compile-time value erased at the boundary, taken verbatim from the annotation. So `(Param.len)`
           rewrites to `(Qty.of (Rational.of (Param.len-num) (Param.len-den)) (Unit.base #\"meter\"))`, a
           `(Qty Rational meter)`. `Qty.value` reads back the exact rational magnitude; with host num=7,
           den=2 that is 7/2. Pins the Rational-magnitude Quantity @param over the scalar host path — the
           layer a parametric-CAD `@param Length` (v-cad) desugars to, closing the heap-typed @param frontier
           for quantities as well as bare rationals.")
  (input
    (do
      (pragma param (param (: widget slider)) (: len (Qty Rational (Unit.base #"meter"))))
      (def (main) (host (Param) (Qty.value (Param.len))))
      (export main)))
  (call main)
  (host-responses (respond Param.len-num (: 7 Int64)) (respond Param.len-den (: 2 Int64)))
  (output (: 7/2 Rational))
  (live-objects known-leak))

; --- Param values as ORDINARY runtime values through the language's control machinery -------------
; The compositions above pin @param x quasiquote and @param x CHAMP-key; these drive a param value
; through the three remaining control shapes — a recursion bound, a match dispatch feeding a SECOND
; param in the selected arm, and a closure capture — so an accessor result is witnessed as a plain
; runtime value everywhere a boundary parameter can go.
(case
  "a @param drives a recursive fold's iteration count"
  (doc
    "The accessor's value bounds a RECURSION: `(build (Param.n) 0)` sums n..1, so the host's 4
           yields 10. The loop's depth is decided by the host response at run time — a sidecar value is a
           first-class loop bound exactly as a boundary parameter is (nothing folds; the recursion emits
           against a genuinely-runtime count).")
  (input
    (do
      (pragma param (param (: widget slider)) (: n Int64))
      (def (build (: i Int64) (: acc Int64)) (if (< i 1) acc (build (- i 1) (+ acc i))))
      (def (main) (host (Param) (build (Param.n) 0)))
      (export main)))
  (call main)
  (host-responses (respond Param.n (: 4 Int64)))
  (output (: 10 Int64)))

(case
  "a @param selects a match arm and a SECOND param feeds the selected body"
  (doc
    "Two accessors in one delegation with a CONTROL dependence between them: `(Param.mode)` is the
           match SCRUTINEE and the selected arm reads `(Param.x)` — mode=1 picks the doubling arm, x=21 →
           42. Pins accessor-in-scrutinee dispatch plus a second accessor performed only on the taken arm
           (the untaken arm's read must not be hoisted into an unconditional host call).")
  (input
    (do
      (pragma param (param (: widget slider)) (: mode Int64))
      (pragma param (param (: widget slider)) (: x Int64))
      (def
        (main)
        (host (Param) (match (Param.mode) (0 (+ (Param.x) 1)) (1 (* (Param.x) 2)) (_ -1))))
      (export main)))
  (call main)
  (host-responses (respond Param.mode (: 1 Int64)) (respond Param.x (: 21 Int64)))
  (output (: 42 Int64)))

(case
  "a @param value crosses into a closure capture and applies"
  (doc
    "The capture face: `(mk (Param.k))` builds a closure OVER the accessor's result, applied to 40
           — host k=2 → 42. The param value must ride the closure environment (allocated after the host
           response arrives) exactly as a boundary parameter would; a capture snapshotting before the
           delegation or aliasing the accessor call itself would misread.")
  (input
    (do
      (pragma param (param (: widget slider)) (: k Int64))
      (def (mk (: a Int64)) (fn ((: v Int64)) (+ v a)))
      (def (main) (host (Param) ((mk (Param.k)) 40)))
      (export main)))
  (call main)
  (host-responses (respond Param.k (: 2 Int64)))
  (output (: 42 Int64)))

(case
  "a @param value SEEDS an in-program handler's state"
  (doc
    "The host→handler-state composition: the host-supplied `seed` param initializes `handle Ctr
           (Param.seed)` — a host-delegated read flowing into the seed position of an IN-PROGRAM
           handler, whose arms then thread it (next reads 10,11,12 → 33 with the host responding 10;
           7,8,9 → 24 at 7). The param pins cover map keys, closures and fold counts; the handler-SEED
           position is the remaining consumer — a seed evaluation that ran before the host delegation
           was wired (or re-performed the param per arm) breaks the sum.")
  (input
    (do
      (pragma param (param (: widget slider)) (: seed Int64))
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (main)
        (host
          (Param)
          (handle
            Ctr
            (Param.seed)
            ((next (u) s (resume s (+ s 1))))
            (+ (Ctr.next) (+ (Ctr.next) (Ctr.next))))))
      (export main)))
  (call main)
  (host-responses (respond Param.seed (: 10 Int64)))
  (output (: 33 Int64)))

(case
  "two @params seed two NESTED handlers independently"
  (doc
    "The multi-param × nested-handle composition: host values a and b each seed their OWN
           handler level — outer `handle CA (Param.a)`, inner `handle CB (Param.b)` — and the body
           draws one value from each (a + b = 30 with hosts 10/20; the asymmetric 5/2 row separates
           the seeds → 7). A seed wiring that read the params in the wrong order (or seeded both
           levels from one accessor) collapses the sum symmetrically and only the asymmetric row
           catches it.")
  (input
    (do
      (pragma param (param (: widget slider)) (: a Int64))
      (pragma param (param (: widget slider)) (: b Int64))
      (effect CA (op geta (-> Unit Int64)))
      (effect CB (op getb (-> Unit Int64)))
      (def
        (main)
        (host
          (Param)
          (handle
            CA
            (Param.a)
            ((geta (u) s (resume s s)))
            (handle CB (Param.b) ((getb (u) s (resume s s))) (+ (CA.geta) (CB.getb))))))
      (export main)))
  (call main)
  (host-responses (respond Param.a (: 10 Int64)) (respond Param.b (: 20 Int64)))
  (output (: 30 Int64)))

(case
  "two @params seeding nested handlers in the WRONG order is caught by an asymmetric row"
  (doc
    "The asymmetric companion to the symmetric 10/20 row above: because `+` is commutative, a
           10/20 host row cannot distinguish the correct wiring (outer=a, inner=b) from a swapped one
           (outer=b, inner=a) — both give 30. The asymmetric 5/2 row SEPARATES them: correct wiring
           reads a=5 (outer CA) + b=2 (inner CB) = 7, while a swapped seeding (or seeding both levels
           from one accessor) would give 4 or 10. Pins the failure mode the symmetric row's doc claims
           but cannot itself catch.")
  (input
    (do
      (pragma param (param (: widget slider)) (: a Int64))
      (pragma param (param (: widget slider)) (: b Int64))
      (effect CA (op geta (-> Unit Int64)))
      (effect CB (op getb (-> Unit Int64)))
      (def
        (main)
        (host
          (Param)
          (handle
            CA
            (Param.a)
            ((geta (u) s (resume s s)))
            (handle CB (Param.b) ((getb (u) s (resume s s))) (+ (CA.geta) (CB.getb))))))
      (export main)))
  (call main)
  (host-responses (respond Param.a (: 5 Int64)) (respond Param.b (: 2 Int64)))
  (output (: 7 Int64)))

(case
  "a @param positions a slice window over a guest-built byte rope"
  (doc
    "Host value × the window family: `(Bytes.slice b (Param.cut) 2)` — the host-supplied cut
           positions a 2-byte window over the guest's rope [1,2]++[3,4,5], read back via Bytes.at
           (host 1 → window [2,3] straddles the SEAM → 23; host 3 → [4,5] → 45; host 4 → only 1
           byte remains → None → -2). The param pins feed arithmetic/keys/counts; a slice BOUND is
           the remaining scalar consumer, and the seam-straddling row composes it with the rope
           window machinery.")
  (input
    (do
      (pragma param (param (: widget slider)) (: cut Int64))
      (def
        (main)
        (host
          (Param)
          (do
            (def b (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4 5))))
            (match
              (Bytes.slice b (Param.cut) 2)
              ((Some w)
                (+
                  (* 10 (match (Bytes.at w 0) ((Some v) v) ((None _u) -1)))
                  (match (Bytes.at w 1) ((Some v) v) ((None _u) -1))))
              ((None _u) -2)))))
      (export main)))
  (call main)
  (host-responses (respond Param.cut (: 1 Int64)))
  (output (: 23 Int64))
  (live-objects 0))

(case
  "a @param slice window at an interior cut reads the non-straddling bytes"
  (doc
    "The companion rows to the seam-straddling slice case above (which pins cut=1 -> [2,3] -> 23):
           an INTERIOR cut past the seam reads a window wholly in the second rope segment — cut=3 -> the
           2-byte window [4,5] -> 45. Pins the doc's second claimed row (only cut=1 was previously tested),
           so the slice-over-rope window is exercised off the seam as well as on it.")
  (input
    (do
      (pragma param (param (: widget slider)) (: cut Int64))
      (def
        (main)
        (host
          (Param)
          (do
            (def b (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4 5))))
            (match
              (Bytes.slice b (Param.cut) 2)
              ((Some w)
                (+
                  (* 10 (match (Bytes.at w 0) ((Some v) v) ((None _u) -1)))
                  (match (Bytes.at w 1) ((Some v) v) ((None _u) -1))))
              ((None _u) -2)))))
      (export main)))
  (call main)
  (host-responses (respond Param.cut (: 3 Int64)))
  (output (: 45 Int64))
  (live-objects 0))

(case
  "a @param slice window whose length exceeds the remaining bytes yields None"
  (doc
    "The doc's third claimed row: a cut leaving fewer than the window length yields None. cut=4 over
           the 5-byte rope leaves only 1 byte ([5]) for a 2-byte window -> Bytes.slice returns None -> the
           match's None arm -> -2. Pins the out-of-range boundary the seam/interior rows do not, completing
           the three rows the slice case's doc describes.")
  (input
    (do
      (pragma param (param (: widget slider)) (: cut Int64))
      (def
        (main)
        (host
          (Param)
          (do
            (def b (Bytes.concat (Bytes.of #list(1 2)) (Bytes.of #list(3 4 5))))
            (match
              (Bytes.slice b (Param.cut) 2)
              ((Some w)
                (+
                  (* 10 (match (Bytes.at w 0) ((Some v) v) ((None _u) -1)))
                  (match (Bytes.at w 1) ((Some v) v) ((None _u) -1))))
              ((None _u) -2)))))
      (export main)))
  (call main)
  (host-responses (respond Param.cut (: 4 Int64)))
  (output (: -2 Int64))
  (live-objects 0))

; --- Response-consumption ORDER: one accessor performed twice, and two accessors interleaved. ---
(case
  "one @param accessor performed TWICE consumes two host responses in order"
  (input
    (do
      (pragma param (param (: widget slider)) (: step Int64))
      (def (main) (host (Param) (- (Param.step) (Param.step))))
      (export main)))
  (call main)
  (host-responses (respond Param.step (: 50 Int64)) (respond Param.step (: 8 Int64)))
  (output (: 42 Int64)))

(case
  "two @param accessors INTERLEAVED (a b a) consume per-op response queues in perform order"
  (input
    (do
      (pragma param (param (: widget slider)) (: a Int64))
      (pragma param (param (: widget slider)) (: b Int64))
      (def (main) (host (Param) (+ (* 100 (Param.a)) (+ (* 10 (Param.b)) (Param.a)))))
      (export main)))
  (call main)
  (host-responses
    (respond Param.a (: 7 Int64))
    (respond Param.b (: 5 Int64))
    (respond Param.a (: 3 Int64)))
  (output (: 753 Int64)))

(case
  "a @param accessor read inside a RECURSIVE walk consumes one response per iteration"
  (doc
    "The consumption-order rows above (TWICE, interleaved a-b-a) are straight-line; this one moves
           the repeated read INSIDE a recursion: `(walk 3 0)` performs `(Param.gain)` once per iteration
           and folds each response into a positional digit — responses 3,7,5 -> 375. Pins that the per-op
           response queue is threaded through the recursive frames in iteration order (a wrong hoist of
           the accessor out of the loop would read one response thrice -> 333; a reversed queue -> 573).
           The fold's-iteration-count row above uses the param as the loop BOUND read once; here the
           accessor is the loop BODY read n times.")
  (input
    (do
      (pragma param (param (: widget slider)) (: gain Int64))
      (def
        (walk (: n Int64) (: acc Int64))
        (if (= n 0) acc (walk (- n 1) (+ (* 10 acc) (Param.gain)))))
      (def (main) (host (Param) (walk 3 0)))
      (export main)))
  (call main)
  (host-responses
    (respond Param.gain (: 3 Int64))
    (respond Param.gain (: 7 Int64))
    (respond Param.gain (: 5 Int64)))
  (output (: 375 Int64)))

(case
  "a @param value sizes a guest-built HEAP structure"
  (doc
    "The recursive-fold row uses a param as a scalar loop bound; here the host response decides a
           HEAP allocation: `(fill (Param.size) (list))` pushes size..1 onto a list, then the guest
           interrogates the structure it built — `(* 10 (List.len xs))` + element 0. size=6 -> a 6-element
           list [6,5,4,3,2,1] -> 60 + 6 = 66. Pins that a sidecar value flows into collection construction
           (the persistent-vector growth path runs against a genuinely-runtime count) and that the
           resulting structure is ordinary — len and positional access agree with the host's number.")
  (input
    (do
      (pragma param (param (: widget slider)) (: size Int64))
      (def
        (fill (: i Int64) (: acc (List Int64)))
        (if (= i 0) acc (fill (- i 1) (List.push acc i))))
      (def
        (main)
        (host
          (Param)
          (do
            (def n (Param.size))
            (def xs (fill n #list()))
            (+ (* 10 (List.len xs)) (match (List.at xs 0) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (call main)
  (host-responses (respond Param.size (: 6 Int64)))
  (output (: 66 Int64)))

; prx1: the generated Param effect is a FIRST-CLASS effect — a guest may handle it IN-PROGRAM (no
; host delegation at all), overriding the runtime parameter the way a test harness would. The
; sidecar's `(effect Param (op width (-> Unit Int64)))` splice is a plain effect declaration, so
; `(handle Param 0 ((width (u) s (resume (+ 40 n) s))) …)` discharges `Param.width` intra-program:
; 42 + n, no host call recorded. Pins that @!param codegen does not privilege the host — the
; accessor participates in the ordinary handler discipline (interceptable, shadowable), and the
; host bind is just the outermost handler. (breaker probe pr1, verified tri-target exact +
; byte-idempotent; the duplicate-name reject is the fenced case above.)
(case
  "a guest handles the generated Param effect in-program overriding the runtime parameter"
  (input
    (do
      (pragma param (param (: widget slider)) (: width Int64))
      (def
        (main (: n Int64))
        (handle Param 0 ((width (u) s (resume (+ 40 n) s))) (+ (Param.width) 2)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64))
  (call main (: 5 Int64))
  (output (: 47 Int64)))

; prx2: the host bind SHADOWED regionally — composes prx1 (the generated effect is first-class) with
; the host delegation: main delegates Param to the host, reads it once at the boundary, and reads it
; again INSIDE an in-program handle that overrides the parameter for that region. The innermost-wins
; discipline holds ACROSS the host/intra boundary: exactly ONE host call is recorded (the outer
; read), the inner read resolves to the override (40 + n), and the sum is host + 100*(40+n). Pins
; that the host bind is literally the outermost handler in the ordinary discipline — a regional
; override never leaks a second boundary call. (breaker probe pr3, verified wasm + hop exact and
; byte-idempotent; the rust host-shim gap keeps the rust row on the documented boundary todo.)
(case
  "an in-program handle shadows the host-bound Param regionally with one boundary call"
  (input
    (do
      (pragma param (param (: widget slider)) (: width Int64))
      (def
        (main (: n Int64))
        (host
          (Param)
          (+
            (Param.width)
            (* 100 (handle Param 0 ((width (u) s (resume (+ 40 n) s))) (Param.width))))))
      (export main)))
  (call main (: 0 Int64))
  (host-responses (respond param.width (: 7 Int64)))
  (host-calls (call param.width))
  (output (: 4007 Int64)))
