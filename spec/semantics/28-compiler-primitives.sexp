; Compiler primitives for userspace contract-building (design/DESIGN-compiler-primitives.md).
; The compiler exposes three CONTRACT-AGNOSTIC primitives a userspace Cadenza program composes into a
; contract-id, with the compiler never modelling "contract": (1) IMPORT REFLECTION, (2) CONST EXECUTION,
; (3) blake3 HASHING. This file pins the primitives themselves — never a contract shape (that lives in a
; userspace .cdz library). Landed incrementally; a generation that does not realize a primitive declines
; its cases.
;
; --- Primitive 3: blake3 hashing (`Blake3.of : Bytes -> Bytes`) -------------------------------------
; `Blake3.of` is a plain, unkeyed BLAKE3-256 content hash — 32 raw bytes, NO key / derive-key / domain
; tag (all domain separation is userspace's job, design D7). It NAMES THE ALGORITHM (design D5): a future
; digest would be a DIFFERENT named function, never a silent change to a generic `Hash`. Over a
; compile-time-visible `Bytes` it FOLDS to a compile-time constant via `blake3::hash`; that constant is
; BYTE-IDENTICAL to the runtime `hash-blake3` heap op (design §9 — same crate, same bytes, both places).
(diagnostic-quality)

(case
  "Blake3.of of the empty byte string is the official empty-input BLAKE3-256 digest"
  (doc
    "The load-bearing byte-identity pin: `Blake3.of b\"\"` is the OFFICIAL BLAKE3 empty-input
           vector `af1349b9...e41f3262` (32 bytes). Because the compile-time fold and the runtime op both
           call the one `blake3` crate over the same bytes, this same digest is what a runtime hash of the
           same input produces (design-compiler-primitives.md §9). Executes byte-identical — the value is
           materialized and compared, not merely folded.")
  (input (Blake3.of b""))
  (output
    (:
      b"\xaf\x13I\xb9\xf5\xf9\xa1\xa6\xa0@M\xea6\xdc\xc9I\x9b\xcb%\xc9\xad\xc1\x12\xb7\xcc\x9a\x93\xca\xe4\x1f2b"
      Bytes))
  (live-objects known-leak))

(case
  "Blake3.of is a 32-byte digest"
  (doc
    "A BLAKE3-256 digest is always 32 bytes, whatever the input length. Pins the output width so a
           downstream `Bytes.eq` against a same-width contract-id is well-formed.")
  (input (Bytes.len (Blake3.of b"the quick brown fox")))
  (output (: 32 Int64)))

(case
  "Blake3.of is deterministic — the same bytes hash to the same digest"
  (doc
    "Hashing the SAME input twice yields the SAME digest — the property that makes a hash usable as
           a content-address / identity. `Bytes.eq` over the two digests is true.")
  (input (= (Blake3.of b"abc") (Blake3.of b"abc")))
  (output (: true Bool)))

(case
  "Blake3.of is sensitive — a one-bit-different input hashes to a different digest"
  (doc
    "Two inputs differing by a single byte hash to DIFFERENT digests, so distinct declarations get
           distinct ids (exact-hash routing depends on this). `Bytes.eq` over the two digests is false.")
  (input (= (Blake3.of b"abc") (Blake3.of b"abd")))
  (output (: false Bool)))

(case
  "Blake3.of composes with Ast.encode — hashing an encoded AST folds to a 32-byte digest"
  (doc
    "The shape of the contract-id path (design §4), with NO contract knowledge in the compiler:
           `Ast.encode` folds a compile-time AST value to its canonical bytes (a compile-time `ConstBytes`),
           and `Blake3.of` folds those bytes to a 32-byte digest at compile time. Here the whole
           `Blake3.of (Ast.encode …)` const-folds; its length is 32. A userspace transform would sit
           between the two, canonicalizing the declaration — but the primitives compose exactly like this.")
  (input (Bytes.len (Blake3.of (Ast.encode (Ast.Int 7)))))
  (output (: 32 Int64)))

; --- Runtime Blake3.of (heap op 91 `hash-blake3`) + the section-9 byte-identity witness ------------
; Blake3.of over a RUNTIME Bytes (not a compile-time constant) lowers to the value-heap op 91; the digest
; is byte-identical to the compile-time fold (both call the one blake3 crate over the same bytes). A
; runtime `b` arrives through the entry call, so `Blake3.of b` takes the runtime path, not the const fold.
(case
  "Blake3.of of a runtime Bytes is a 32-byte digest (runtime op 91)"
  (doc
    "`(Bytes.of (list (UInt8.wrap k) (UInt8.wrap k) (UInt8.wrap k)))` over a runtime entry param `k` builds a RUNTIME Bytes (not a constant),
           so `Blake3.of` of it lowers to the runtime `hash-blake3` heap op, returning a fresh 32-byte Bytes.
           Pins the runtime lowering executes and yields the 32-byte width.")
  (input
    (do
      (def
        (run (: k Int64))
        (Bytes.len (Blake3.of (Bytes.of #list((UInt8.wrap k) (UInt8.wrap k) (UInt8.wrap k))))))
      (export run)))
  (call run 5)
  (output (: 32 Int64)))

(case
  "runtime Blake3.of equals the compile-time fold of the same bytes (section-9 byte-identity)"
  (doc "\x")
  (input
    (do
      (def (run (: k Int64)) (= (Blake3.of (Bytes.of #list((UInt8.wrap k)))) (Blake3.of b"\x05")))
      (export run)))
  (call run 5)
  (output (: true Bool)))

; --- The primitives COMPOSE into a contract-id (design §4 — validate the primitives suffice) -----------
; A userspace contract-id is `tag ++ blake3(canonical-declaration-bytes)`. These pin that the three
; primitives compose to a COMPILE-TIME CONSTANT tagged id, entirely in the compiler's contract-agnostic
; surface: a compile-time Ast (here a `quote`; import reflection's `__ast__` is the same value form) ->
; `Ast.encode` -> `Blake3.of` -> `Bytes.concat` with a userspace domain tag. All const-fold; the whole id
; is a baked constant a guest compares against `msg.contract` with `Bytes.eq`. (The concrete contract-id
; SCHEME + the .cdz library live in the platform lane; this only pins that the primitives are sufficient.)
(case
  "the primitives compose to a compile-time tagged contract-id of the expected length"
  (doc
    "`Bytes.concat (userspace 0x01 tag) (Blake3.of (Ast.encode <decl-Ast>))` folds to a 33-byte
           constant (1 tag byte + 32 blake3 bytes) — a content-address of the declaration AST, built with
           NO contract knowledge in the compiler. Pins the design §4 composition end-to-end at compile time.")
  (input
    (Bytes.len
      (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))))
  (output (: 33 Int64)))

(case
  "the composed contract-id is deterministic — the same declaration folds to the same id"
  (doc
    "Hashing the SAME declaration AST twice yields the same tagged id (a content-address). `Bytes.eq`
           over the two folded ids is true.")
  (input
    (=
      (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
      (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))))
  (output (: true Bool)))

(case
  "distinct declarations fold to distinct contract-ids"
  (doc
    "Two DIFFERENT declaration ASTs (differing name) fold to DIFFERENT tagged ids — the property that
           makes exact-hash contract routing sound. `Bytes.eq` over the two ids is false.")
  (input
    (=
      (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
      (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-fahrenheit Int64 Int64)))))))
  (output (: false Bool)))

(case
  "the compile-time contract-id fold is byte-identical to the host Contract (golden #3238)"
  (doc
    "BYTE-IDENTITY cross-check (design §9): the compile-time primitive fold and the host contract-id
           constructor (`cdz_platform::Contract::new` → `cdz_contract::contract_declaration` → `Hash::of`)
           must produce the SAME 33-byte id, else a userspace P4 contract-id would not match the platform's
           routing key. The host hashes the ASSEMBLED semantic-interface form
           `(contract <str-name> (types <type-decl>…) <input-name> <output-name>)`, canonicalized then
           `codec::encode`d, prefixed with the 0x01 Contract tag. This reproduces that exact form for the
           `temp.celsius` fixture — note the name is a STRING leaf (not a Name), and `(types (type Temp
           (Mk f64)))` mirrors the host's declaration builder — and folds `Bytes.concat 0x01 (Blake3.of
           (Ast.encode …))` at compile time. The result equals the golden id pinned by
           `cdz-platform` test `id_is_byte_stable_…` (#3238, base62 01UUXRcMG63Ct66Z4TP7l6QfY7pvktdISpoHyTdJVtS70).
           Because `Ast.encode` canonicalizes with the same `cadenza-ast` codec the host uses, an assembled
           quote of the same structure encodes byte-for-byte identically. If this drifts, the compile-time
           primitives and the host contract-id have diverged — a P4 flag-day, not a papering-over.")
  (input
    (=
      (Bytes.concat
        b"\x01"
        (Blake3.of
          (Ast.encode (quote (contract "temp.celsius" (types (type Temp (Mk f64))) Temp Temp)))))
      b"\x01\x86\x0cz\xcbC\xd2l\xb9:\x8c\xed\xd7\xd6\x97\xb20\x08\x8aP\xd5\x0e\xb27l\xcf\xee`\xca\xba,R:"))
  (output (: true Bool)))

; --- Primitive 2: const execution — a TAKEN trap on the const-folded path is FAIL-LOUD ---------------
; A `trap("msg")` reached while const-EXECUTING a total function (const-demanded arguments) is not a silent
; decline to a runtime value: the evaluator surfaces the trap's MESSAGE as the compile error CDZ0304 (the
; provable-trap code). This is what makes a userspace self-reflection transform's genuine-absence trap (a
; malformed contract module, a missing required pragma) a fail-loud, actionable authoring error at the
; fold — not a corrupt id. A `trap` on an UN-taken branch of a const fold is never executed, so it does not
; decline the fold (the sibling cases above fold through such dead arms); only a TAKEN trap surfaces.
(case
  "a taken const-fold trap surfaces its message as a compile error (CDZ0304)"
  (doc
    "A recursive `const`-param countdown reaches `trap(\"…\")` on the const-executed path when its
           argument is compile-time-known (`f 3` counts down to 0). The general const-evaluator executes the
           trap and surfaces its MESSAGE as the provable-trap compile error CDZ0304 — a const-executed trap
           is fail-loud, not a decline to a runtime trap. Message-matched so the surfaced text is pinned.")
  (input
    (do
      (def (f (const (: n Int64))) (if (= n 0) (trap "const countdown reached zero") (f (- n 1))))
      (def (main) (f 3))
      (export main)))
  (error CDZ0304 (message "const countdown reached zero")))

(case
  "a const trap consumed by Ast.encode surfaces its message, not the generic decline"
  (doc
    "The self-reflection shape: a trap reached while folding `Ast.encode`'s operand. `Ast.encode`
           DEMANDS a compile-time-constant AST, and the trapping operand comes through a NON-recursive
           `const`-param fn the ordinary inliner reduces to a textless trap (message dropped) — so
           `Ast.encode` const-evaluates its operand and surfaces the trap's MESSAGE as CDZ0304 rather than
           the generic \"Ast.encode of a runtime AST value\" decline. This is the P4 contract-id library's
           genuine-absence trap (a missing required pragma) surfacing an actionable compile error.")
  (input
    (do
      (def (f (const (: n Int64))) (if (= n 0) (trap "const trap under Ast.encode") n))
      (def (g) (Ast.encode (Ast.Int (BigInt.of (f 0)))))
      (export g)))
  (error CDZ0304 (message "const trap under Ast.encode")))

; --- Primitive 2: const execution — Option-threaded navigation folds (nullary variant, no sentinel) --
; The operator-directed clean form of a self-reflection transform threads `Option` (not sentinel Name/""
; values) through its AST navigation: a helper returns `Option.None` on a non-match. When that `_ =>
; Option.None` arm is TAKEN on the const-folded path, the const-evaluator must FOLD the nullary variant to
; its empty-payload sum value — NOT decline (a bare `Option.None` resolves through a `(. Option None)`
; member / the `(intrinsic sum-new)` constructor head, which the evaluator must recognize as the value the
; variant denotes). This is what lets the no-sentinel contract-id library const-fold (design D-clean-form).
(case
  "const execution folds a TAKEN nullary variant (Option.None) in Option-threaded AST navigation"
  (doc
    "`name-of` returns `Option.None` for a non-`Ast.Name` form; `label` matches that Option and, on
           the `Option.None` branch, builds `(Ast.Name \"none\")`. Fed a non-Name `(Ast.Int 5)`, the `_ =>
           Option.None` arm is TAKEN, so folding `label` requires const-EVALUATING the nullary `Option.None`
           value. `Ast.encode` DEMANDS a compile-time constant, so if the taken `Option.None` did not fold the
           encode would decline (\"runtime AST value\") and this equality could not be computed. Folds to
           `Ast.encode (Ast.Name \"none\")`, equal to the RHS — witnessing the nullary-variant const-fold that
           the Option-threaded (no-sentinel) navigation depends on.")
  (input
    (do
      (def
        (name-of (const (: form Ast)))
        (match form ((Ast.Name n) (Option.Some n)) (_ Option.None)))
      (def
        (label (const (: form Ast)))
        (match (name-of form) ((Option.None) (Ast.Name "none")) ((Option.Some n) (Ast.Name n))))
      (def (run) (= (Ast.encode (label (Ast.Int (BigInt.of 5)))) (Ast.encode (Ast.Name "none"))))
      (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — HIGHER-ORDER folds (a closure passed to a `const` fn parameter) -----
; A `const f: (T) -> U` parameter carrying a `fn` argument is captured as a first-class closure and APPLIED
; per element, so a user recursive map/filter/fold that threads a closure const-folds. Without first-class
; const closures the fold declined (a lambda is not a `is_const_value`, and a closure parameter has no
; static lambda for the evaluator to reduce). (The stdlib `List.map`/`filter`/`fold` are augmentations that
; expand to iterator pipelines the evaluator does not yet interpret — a separate decline class.)
(case
  "const execution folds a higher-order recursion — a closure applied per element"
  (doc
    "`mymap` threads a `const` closure `f` and applies it to each element of a `const` list. Fed the
           mapper `(fn (x) (Ast.Int (BigInt.of x)))` over `(list 1 2 3)`, the whole higher-order recursion
           const-folds to `(list (Ast.Int 1) (Ast.Int 2) (Ast.Int 3))`. `Ast.encode` DEMANDS a compile-time
           constant, so a non-folding higher-order call would decline the encode; the fold's bytes equal the
           encoding of the literal list, witnessing that the closure was captured + applied at compile time.")
  (input
    (do
      (def
        (mymap (const (: xs (List Int64))) (const (: f (-> Int64 Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t)) (List.prepend (mymap t f) (f h)))))
      (def
        (run)
        (=
          (Ast.encode (Ast.List (mymap #list(1 2 3) (fn ((: x Int64)) (Ast.Int (BigInt.of x))))))
          (Ast.encode
            (Ast.List
              #list((Ast.Int (BigInt.of 1)) (Ast.Int (BigInt.of 2)) (Ast.Int (BigInt.of 3)))))))
      (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — a COMPOUND-typed const param activates the recursive fold ----------
; The const-fold activation gate accepts a `const` parameter of a SHRINKING type-constructor shape (`(Option
; T)`, `(Result …)`, a user sum), not only `(List …)`/bare-name — the evaluator carries those values, so a
; total recursion over such a const param folds. Before, an `(Option Int64)` const param NEVER activated the
; fold: the recursion emitted a RUNTIME call whose trap base case then ran to a NON-TERMINATING wasm loop
; (a wasm/rust divergence — breaker adv-const-option-param-recursive-trap-wasm-hang). Folding it removes the
; runtime artifact entirely (a taken trap surfaces CDZ0304; a value base case folds to the value). (Product/
; dictionary forms — `(Record …)`/`(Tuple …)`/`(Map …)`/`(Set …)` — stay EXCLUDED: a counter-driven dict
; consumer passes them unchanged, so the fold would not terminate on the shape.)
(case
  "a recursive const (Option Int64) param reaching a trap folds to CDZ0304, not a runtime hang"
  (doc
    "`f (Option.Some 2)` counts the payload down and reaches `trap` at 0. The `(Option Int64)` const
           param now ACTIVATES the recursive const-fold, which executes the countdown and surfaces the taken
           trap's MESSAGE as CDZ0304 — where before the param type failed the activation gate, so a runtime
           recursive call was emitted whose trap base case ran to a non-terminating wasm loop (divergent from
           the rust backend, which trapped). Folding eliminates the runtime artifact, closing the divergence.")
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match
          o
          ((Option.Some k) (if (= k 0) (trap "adv option reached zero") (f (Option.Some (- k 1)))))
          ((Option.None) 0)))
      (def (main) (f (Option.Some 2)))
      (export main)))
  (error CDZ0304 (message "adv option reached zero")))

(case
  "a recursive const (Option Int64) param with a VALUE base case folds to the value"
  (doc
    "The trap-free twin: the same `(Option Int64)` const-param countdown returns a value at 0. It
           const-folds to that value (99), witnessed by encoding it and comparing to the encoded literal —
           `Ast.encode` demands a compile-time constant, so a non-folding recursion would decline the encode.")
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match o ((Option.Some k) (if (= k 0) 99 (f (Option.Some (- k 1))))) ((Option.None) 0)))
      (def
        (run)
        (=
          (Ast.encode (Ast.Int (BigInt.of (f (Option.Some 2)))))
          (Ast.encode (Ast.Int (BigInt.of 99)))))
      (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — the `(const <expr>)` FORCE-EVAL / const-DEMAND block ----------------
; `(const e)` (operator-requested) forces `e` to reduce to a compile-time constant and REJECTS if it cannot.
; It is the explicit const-DEMAND marker: the evaluator runs on `e` DIRECTLY, so a total computation over
; compile-time-known data folds WITHOUT threading `const` params through its callees (dropping that clunk).
; A residual runtime value is an authoring error (CDZ0201), not a silent pass-through to a runtime compute.
(case
  "a `(const …)` force-eval block folds a helper call over constant data (no const params needed)"
  (doc
    "`sq` declares NO `const` parameter, yet `(const (sq 5))` folds to 25 at compile time: the `(const
           …)` block is the demand signal, so the evaluator interprets `sq 5` directly. This is the construct
           the operator wanted — force a computation to const-fold at the use site instead of threading
           `const` params through every helper. The folded value is the ordinary scalar 25.")
  (input (do (def (sq (: x Int64)) (* x x)) (def (main) (const (sq 5))) (export main)))
  (output (: 25 Int64)))

(case
  "a `(const …)` block whose expression depends on runtime data is rejected (CDZ0201)"
  (doc
    "`(const (+ k 1))` over a runtime entry parameter `k` cannot reduce to a compile-time constant, so
           the force-eval block REJECTS with CDZ0201 — the block ASSERTS compile-time evaluability, so a
           residual runtime value is a fail-loud authoring error, not a silent pass-through to a runtime add.")
  (input (do (def (main (: k Int64)) (const (+ k 1))) (export main)))
  (error CDZ0201 (message "const` block requires a compile-time constant")))

; --- Primitive 2: const execution — MUTUAL recursion over a compound (Ast) value folds -----------------
; The general const-evaluator reaches a MUTUALLY-recursive pair of `const` functions over an `Ast` value:
; `leaves` (per-node) ↔ `leaves-list` (per-child-list) count the leaves of a quoted form. Pins that mutual
; recursion + a `(List Ast)` const param + a per-element sum all compose in one const-fold (the P4 self-
; reflection transform shape). Regression guard for the recursive engine's reach over compound compositions.
(case
  "mutual recursion over an Ast value const-folds a leaf count (under Ast.encode demand)"
  (doc
    "`leaves (quote (f 1 2))` = 3 (three leaf nodes: `f`, `1`, `2`) via a mutually-recursive
           `leaves`/`leaves-list` pair, both `const`. `Ast.encode` demands a compile-time constant, so the
           whole mutual recursion must const-fold for the encode to succeed; its bytes equal the encoding of
           the literal count 3 — witnessing the fold reached across the mutual recursion + the `(List Ast)`
           param + the per-element `+`.")
  (input
    (do
      (def (leaves (const (: a Ast))) (match a ((Ast.List xs) (leaves-list xs)) (_ 1)))
      (def
        (leaves-list (const (: xs (List Ast))))
        (match xs (#list() 0) (#list(h (.. t)) (+ (leaves h) (leaves-list t)))))
      (def
        (run)
        (=
          (Ast.encode (Ast.Int (BigInt.of (leaves (quote (f 1 2))))))
          (Ast.encode (Ast.Int (BigInt.of 3)))))
      (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — TUPLE destructuring in the recursive evaluator --------------------
; The const-evaluator now DESTRUCTURES a tuple pattern `(tuple a b)` in a `match` (the matcher recognizes
; the tuple pattern; a binder reads its slot via an `Elem` step over the `CVal::Tuple`). This lets a `(const
; …)`-demanded computation over a tuple const value fold — a `(Tuple …)` const param is otherwise excluded
; from the bare recursive-fold activation gate (it is a product/dictionary shape a counter-driven consumer
; passes unchanged), so `(const …)` is the demand marker that forces such a fold.
(case
  "a `(const …)` block destructures a tuple in a match and folds"
  (doc
    "`(const (match (tuple 3 5) ((tuple a b) (+ a b))))` folds to 8: the evaluator matches the tuple
           pattern and reads binders `a`/`b` out of the `CVal::Tuple` (an `Elem`-step projection), which
           previously declined. The `(const …)` block forces the fold (a `(Tuple …)` shape is not activated
           by the bare gate).")
  (input (do (def (main) (const (match #tuple(3 5) (#tuple(a b) (+ a b))))) (export main)))
  (output (: 8 Int64)))

(case
  "a `(const …)` block folds a recursion over a tuple const value"
  (doc
    "`f` counts the first tuple slot down to 0 while accumulating into the second, threading a fresh
           `(tuple …)` each step. `(const (f (tuple 3 0)))` folds to 3 — the tuple pattern destructure, the
           tuple construction, and the recursion all compose in one const-fold under the force-eval block.")
  (input
    (do
      (def
        (f (const (: t (Tuple Int64 Int64))))
        (match t (#tuple(a b) (if (= a 0) b (f #tuple((- a 1) (+ b 1)))))))
      (def (main) (const (f #tuple(3 0))))
      (export main)))
  (output (: 3 Int64)))

; --- Primitive 2: const execution — `(const …)` is never STRICTER than the ordinary fold --------------
; The force-eval block first tries the ORDINARY compile-time fold (`core_of`) and only falls back to the
; general value-interpreter for what that doesn't reach (recursion/composition). So `(const e)` folds
; EVERYTHING the plain compiler already folds — a `Float`/`Bytes`/record/tuple constant expression the
; value-interpreter has no value for still folds through the block, rather than a false CDZ0201 reject.
(case
  "a `(const …)` block folds a constant Float expression (never stricter than the ordinary fold)"
  (doc
    "`(const (+ 1.5 2.0))` folds to 3.5 — the ordinary `core_of` fold already produces the constant
           Float, so the force-eval block returns it rather than rejecting (the general value-interpreter
           carries no Float value, so a const_eval-only block wrongly rejected this perfectly-constant expr).")
  (input (do (def (main) (const (+ 1.5 2.0))) (export main)))
  (output (: 3.5 Float64)))

; --- Primitive 2: const execution — RECORD field projection folds ------------------------------------
; A direct field projection off a constant record `(. (record …) field)` const-folds (the evaluator builds
; a `CVal::Record` and projects the field by name). Regression guard for record const-folding — the value
; class the P4 descriptor `contract(m) -> Record(id, …)` reads a field off. A record threaded THROUGH a
; record-param FUNCTION now folds too: a `(record …)` LITERAL resolves to an APPLIED `RecordNew` (not the
; symbol-headed `Resolved::Record`), so the value-interpreter reduces it via `reduce_ctor` to the symbol-
; headed compound and builds a `CVal::Record` — evaluating each field in the current env, so a record built
; from const-param values folds as well.
(case
  "a direct record field projection const-folds"
  (doc
    "`(. (record (x 7) (y 9)) x)` folds to 7 at compile time; witnessed byte-exact through `Ast.encode`
           (which demands a compile-time constant) against the encoding of the literal 7.")
  (input
    (do
      (def
        (main)
        (=
          (Ast.encode (Ast.Int (BigInt.of (. #record((= x 7) (= y 9)) x))))
          (Ast.encode (Ast.Int (BigInt.of 7)))))
      (export main)))
  (output (: true Bool)))

(case
  "a record threaded through a record-param function const-folds"
  (doc
    "`(const (get-x (record (x 42) (y 7))))` folds to 42: the record LITERAL resolves to an applied
           `RecordNew`, which the value-interpreter reduces (via `reduce_ctor`) to a `CVal::Record`, binds to
           the `get-x` parameter, and projects `x` off inside the body. Previously declined — `apply_const_prim`
           had no `RecordNew` arm — so a record-param helper under a `(const …)` block rejected CDZ0201. The
           param's record type is recovered by inference from the `(. r x)` projection (no annotation needed).")
  (input
    (do (def (get-x r) r.x) (def (main) (const (get-x #record((= x 42) (= y 7))))) (export main)))
  (output (: 42 Int64)))

(case
  "a record BUILT from a const param then read through a function const-folds"
  (doc
    "`mk n` builds `(record (x n) (y (* n 2)))` from its const param, `sum-r` reads both fields; `(const
           (sum-r (mk 10)))` folds to 30. Exercises the general case: the compound constructor evaluates each
           field in the CURRENT env (the bound `n`), not just a param-free literal — the value-interpreter twin
           of how a `(tuple …)` built from const params already folded.")
  (input
    (do
      (def (mk (const (: n Int64))) #record((= x n) (= y (* n 2))))
      (def (sum-r r) (+ r.x r.y))
      (def (main) (const (sum-r (mk 10))))
      (export main)))
  (output (: 30 Int64)))

; --- Primitive 2: const execution — MAP query folds (lookup / replace) under `(const …)` --------------
; A `(const …)`-demanded MAP QUERY const-folds: `Map.empty`/`Map.insert`/`Map.lookup` evaluate over a
; `CVal::Map` (an assoc list, latest-write-per-key wins). Only order-INDEPENDENT results fold (a looked-up
; value); a const map is NEVER materialized to a `Core` — its runtime CHAMP iteration order is not presumed
; (soundness), so an order-exposing use (encode / to-list) still declines rather than risk a wrong order.
(case
  "a `(const …)` map lookup folds to the associated value"
  (doc
    "`Map.lookup (Map.insert (Map.empty) 2 20) 2` folds to `Option.Some 20`; the match extracts 20.")
  (input
    (do
      (def
        (main)
        (const
          (match (Map.lookup (Map.insert (Map.empty) 2 20) 2) ((Option.Some v) v) ((Option.None) 0))))
      (export main)))
  (output (: 20 Int64)))

(case
  "a `(const …)` map insert replaces a key in place (latest write wins)"
  (doc
    "Inserting key 1 twice keeps the LATEST value; `Map.lookup … 1` folds to 99, not 10 — the const
           map's `insert` replaces the association in place, matching the runtime persistent-map semantics.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Map.lookup (Map.insert (Map.insert (Map.empty) 1 10) 1 99) 1)
            ((Option.Some v) v)
            ((Option.None) 0))))
      (export main)))
  (output (: 99 Int64)))

; --- Primitive 2: const execution — SET query folds (of / insert / contains / len) under `(const …)` --
; Parallel to Map: a `(const …)`-demanded SET QUERY const-folds over a `CVal::Set` (distinct members).
; `Set.of` dedups a constant list; `Set.insert` adds if absent; `Set.contains` → Bool; `Set.len` → count.
; Same soundness guard: a const set is only QUERIED, never MATERIALIZED (its runtime CHAMP iteration order
; is not presumed). (Bare `Set.empty` as a value is a minor residual — no `SetEmpty` prim to fold; use
; `Set.of`.)
(case
  "a `(const …)` set membership + dedup-len fold"
  (doc
    "`Set.of (list 1 2 2 3)` dedups to {1,2,3} (len 3); `Set.contains … 2` is true. The const set is
           queried, never materialized. Here `Set.len` of the deduped set folds to 3.")
  (input (do (def (main) (const (Set.len #set(1 2 2 3)))) (export main)))
  (output (: 3 Int64)))

(case
  "a `(const …)` set contains after insert folds"
  (doc
    "`Set.contains (Set.insert (Set.of (list 1 2)) 3) 3` folds to true (100 branch) — insert adds a
           new member, contains queries it, all at compile time under the force-eval block.")
  (input
    (do (def (main) (const (if (Set.contains (Set.insert #set(1 2) 3) 3) 100 200))) (export main)))
  (output (: 100 Int64)))

(case
  "a `(const …)` map lookup MISS folds to Option.None"
  (doc
    "Negative path: `Map.lookup … 2` on a map lacking key 2 folds to `Option.None`; the match's None
           arm yields 0. Pins the absent-key branch (the positive lookup is pinned above).")
  (input
    (do
      (def
        (main)
        (const
          (match (Map.lookup (Map.insert (Map.empty) 1 10) 2) ((Option.Some v) v) ((Option.None) 0))))
      (export main)))
  (output (: 0 Int64)))

(case
  "a `(const …)` set contains of an ABSENT member folds to false"
  (doc
    "Negative path: `Set.contains (Set.of (list 1 2 3)) 9` folds to false → the 200 branch. Pins the
           absent-member result (the present-member case is pinned above).")
  (input (do (def (main) (const (if (Set.contains #set(1 2 3) 9) 100 200))) (export main)))
  (output (: 200 Int64)))

(case
  "a `(const …)` force-eval block preserves its operand's ascribed width"
  (doc
    "The force-eval block folds its operand at the ascribed width: `(const (: 5 Int32))` reduces to the
           32-bit constant 5, not a default-width Int64. Pins that the const-demand block is transparent to
           type grounding — it forces the ascribed value and carries its declared width through the fold.")
  (input (const (: 5 Int32)))
  (output (: 5 Int32)))

(case
  "a `(const …)` block with the wrong operand count is a malformed reject (arity guard)"
  (doc
    "The block is written `(const <expression>)` — EXACTLY one operand. `(const 1 2)` (two) is a coded
           malformed reject at resolve (CDZ0201), a DISTINCT path from the runtime-dependence reject above: it
           fires on the FORM's arity before any force-eval, never a silent pass-through of the first operand.")
  (input (const 1 2))
  (error CDZ0201))

; --- Primitive 0/2: Ast.encode folds ANY operand the const-evaluator can reduce (demand-path) -----------
; `Ast.encode` DEMANDS a compile-time-constant AST. When `core_of` alone does not fold its operand (a
; Map/Set query, a composition inside a `const`-param fn), the encode now const-EVALUATES the operand and
; MATERIALIZES a folded Ast value through the SAME canonical codec — so it folds byte-identical, rather
; than the generic "runtime AST value" decline. (A genuinely-runtime operand still declines; a taken trap
; surfaces CDZ0304.)
(case
  "Ast.encode folds a Map query composed inside a const-param fn (const_eval demand path)"
  (doc
    "`label` (a `const`-param fn) does a `Map.lookup` and wraps the value in `Ast.Int`; `core_of` alone
           leaves the map query runtime, but `Ast.encode`'s operand const-EVALUATES it to the constant
           `Ast.Int 9` and encodes it — byte-identical to encoding the literal `(Ast.Int 9)`. Before, this
           declined ('runtime AST value') even though the value is fully compile-time-known.")
  (input
    (do
      (def
        (label (const (: n Int64)))
        (match
          (Map.lookup (Map.insert (Map.empty) 1 n) 1)
          ((Option.Some v) (Ast.Int (BigInt.of v)))
          ((Option.None) (Ast.Name "none"))))
      (def (main) (= (Ast.encode (label 9)) (Ast.encode (Ast.Int (BigInt.of 9)))))
      (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — a BARE nullary `Map.empty` value folds a Map query ----------------
; The const-evaluator's `Member` arm (`(. Map empty)`) used to const-evaluate its MODULE operand, get None,
; and hard-decline — so a BARE `Map.empty` (the nullary collection-empty VALUE, not the parenthesized
; application `(Map.empty)`) never folded, and a `Map.insert`/`Map.lookup`/`Map.len` chain starting from it
; declined even though it is fully compile-time-known. The arm now falls back to `core_of` + `core_to_cval`
; (the same bridge the catch-all uses) for a member the module-operand can't project, folding the bare
; `Map.empty` to a constant `Core::MapNew` → `CVal::Map`. The `Map.insert`/`lookup`/`len` query arms already
; existed in `apply_const_prim`; this connects the bare-empty base to them. (`Set` has no `.empty` member —
; you build an empty set as `Set.of (list)` — so this is Map-specific; the Set path folds via `Set.of`.)
(case
  "a bare Map.empty const-folds its length to zero"
  (doc
    "`(const (Map.len Map.empty))` folds to 0. Pins the bare nullary `Map.empty` value flowing into the
           const-evaluator as an empty `CVal::Map` (the Member-arm `core_of` fallback), then `Map.len` → 0.")
  (input (const (Map.len Map.empty)))
  (output (: 0 Int64)))

(case
  "a bare Map.empty threaded through Map.insert folds its length"
  (doc
    "`(const (Map.len (Map.insert Map.empty 7 100)))` folds to 1 — the bare `Map.empty` folds to a
           constant empty map, `Map.insert` appends `7 ↦ 100` (a `CVal::Map` query, key compared by
           `cval_eq`), and `Map.len` counts one entry. This is the standalone shape behind the Map-state
           const handle (v-effects cms3): before, the bare `Map.empty` base declined the whole query.")
  (input (const (Map.len (Map.insert Map.empty (: 7 Int64) (: 100 Int64)))))
  (output (: 1 Int64)))

(case
  "a bare Map.empty threaded through two distinct Map.inserts folds its length to two"
  (doc
    "Two inserts at DISTINCT keys grow the constant map to two entries — `cval_eq` decides the keys are
           unequal, so both append. `(const (Map.len (Map.insert (Map.insert Map.empty 7 100) 0 5)))` → 2.")
  (input
    (const
      (Map.len
        (Map.insert (Map.insert Map.empty (: 7 Int64) (: 100 Int64)) (: 0 Int64) (: 5 Int64)))))
  (output (: 2 Int64)))

(case
  "a Map.insert replacing an existing key does not grow the constant map"
  (doc
    "Re-inserting the SAME key REPLACES (latest-write-wins) rather than appends — `cval_eq` finds the
           existing `7` entry. `(const (Map.len (Map.insert (Map.insert Map.empty 7 1) 7 2)))` stays 1.")
  (input
    (const
      (Map.len (Map.insert (Map.insert Map.empty (: 7 Int64) (: 1 Int64)) (: 7 Int64) (: 2 Int64)))))
  (output (: 1 Int64)))

(case
  "a bare Map.empty threaded through Map.insert then Map.lookup folds the found value"
  (doc
    "`(const (Map.lookup (Map.insert Map.empty 7 100) 7))` folds to `(Some 100)` — the const `Map.lookup`
           arm finds the key (`cval_eq`) and returns its value under the result type's Some disc.")
  (input (const (Map.lookup (Map.insert Map.empty (: 7 Int64) (: 100 Int64)) (: 7 Int64))))
  (output (: (Some 100) (Option Int64))))

(case
  "a Map.lookup of an absent key over a constant map folds to None"
  (doc
    "`(const (Map.lookup (Map.insert Map.empty 7 100) 9))` folds to `(None unit)` — `cval_eq` decides
           `9 ≠ 7` for the one entry, so the const lookup returns None under the result type's None disc.")
  (input (const (Map.lookup (Map.insert Map.empty (: 7 Int64) (: 100 Int64)) (: 9 Int64))))
  (output (: (None unit) (Option Int64))))

; --- Primitive 2: const execution — a compound BUILT IN A RECURSION folds -----------------------------
; The value-interpreter reduces an applied compound constructor (`RecordNew`/`TupleNew`/`ListNew`) in the
; CURRENT env (#3516), so a recursion that constructs a FRESH compound each step — the record analogue of
; the tuple-threading case above — const-folds. These pin that generalization in a recursive setting (the
; #3516 cases were non-recursive): a record built from a recursively-computed field, and Bytes assembled by
; repeated `Bytes.concat`. Regression guard for compound construction composing with the recursive engine.
(case
  "a recursion that builds a fresh RECORD each step const-folds a field read"
  (doc
    "`walk n` returns `(record (v k))` where `k` counts the recursion depth; `(const (. (walk 4) v))`
           folds to 4. Each step builds a fresh `(record (v …))` — an applied `RecordNew` whose field is the
           recursively-computed value — which the interpreter evaluates in the current env, then the outer
           `(. … v)` projects. Before #3516 the record construction declined (`apply_const_prim` had no
           `RecordNew` arm), so the whole recursion did.")
  (input
    (do
      (def
        (walk (const (: n Int64)))
        (if (= n 0) #record((= v 0)) #record((= v (+ 1 (. (walk (- n 1)) v))))))
      (def (main) (const (. (walk 4) v)))
      (export main)))
  (output (: 4 Int64)))

(case
  "a recursion that builds Bytes via Bytes.concat const-folds byte-exact"
  (doc
    "`bcat n` concatenates `n` copies of the one-byte `A` (0x41); `(const (bcat 3))` folds to the 3-byte
           sequence `AAA`, witnessed byte-exact through `Ast.encode` against `Bytes.of (list 65 65 65)`. Pins
           `Bytes.concat` + an empty-`Bytes` base composing across the recursive engine under a force-eval
           block.")
  (input
    (do
      (def
        (bcat (const (: n Int64)))
        (if (= n 0) (Bytes.of #list()) (Bytes.concat (Bytes.of #list(65)) (bcat (- n 1)))))
      (def
        (main)
        (=
          (Ast.encode (Ast.Bytes (const (bcat 3))))
          (Ast.encode (Ast.Bytes (Bytes.of #list(65 65 65))))))
      (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — a Map/Set COLLECTION STATE built in a RECURSION folds a query -------
; The record/Bytes cases above build a fresh COMPOUND per step; a Map/Set is different — the interpreter
; carries it as a QUERY-ONLY `CVal::Map`/`CVal::Set` (never re-materialized; `cval_to_core` declines it), and
; the growing-state ops fold through `apply_const_prim`'s `cval_eq`-gated arms. These pin that the recursive
; engine composes with that query-only collection model: a `Map.insert`/`Set.insert` chain ACCUMULATED across
; recursion depth, then a size/lookup query folding to a scalar the block can materialize. (`Map.len` is the
; current spelling — `Map.size` was renamed; the bare `Map.empty` base folds since #3670.)
(case
  "a recursion that builds a Map collection state const-folds its length"
  (doc
    "`build n` inserts `n ↦ n*10` onto `(build (n-1))`, bottoming at `Map.empty`; `(const (Map.len (build
           3)))` folds to 3. Pins a Map collection state ACCUMULATED across the recursive const engine — each
           step is a `Map.insert` onto the recursively-built `CVal::Map`, then `Map.len` counts the entries.")
  (input
    (do
      (def
        (build (const (: n Int64)))
        (if (= n 0) (Map.empty) (Map.insert (build (- n 1)) n (* n 10))))
      (def (main) (const (Map.len (build 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a recursion that builds a Map collection state const-folds a key lookup"
  (doc
    "The query companion: after building the same recursive map, `(const (Map.lookup (build 3) 2))` folds
           the `Some 20` payload (`cval_eq` finds key 2 ↦ 20). Pins that a `Map.lookup` over a recursively-
           accumulated `CVal::Map` folds its found value, not just a count.")
  (input
    (do
      (def
        (build (const (: n Int64)))
        (if (= n 0) (Map.empty) (Map.insert (build (- n 1)) n (* n 10))))
      (def (main) (const (match (Map.lookup (build 3) 2) ((Option.Some v) v) ((Option.None) 0))))
      (export main)))
  (output (: 20 Int64)))

(case
  "a recursion that builds a Set collection state const-folds its length"
  (doc
    "The Set twin: `acc n` inserts `n` onto `(acc (n-1))`, bottoming at `(Set.of (list))`; `(const (Set.len
           (acc 4)))` folds to 4 — a Set collection state accumulated across recursion, deduped by `cval_eq`,
           then counted. (Set has no `.empty`; the empty base is `Set.of (list)`.)")
  (input
    (do
      (def (acc (const (: n Int64))) (if (= n 0) #set() (Set.insert (acc (- n 1)) n)))
      (def (main) (const (Set.len (acc 4))))
      (export main)))
  (output (: 4 Int64)))

; --- Primitive 2: const execution — a CHAR value threads through the recursive engine -----------------
; The value-interpreter carries a `CVal::Char` (materializes to `Core::ConstChar`), so a `Char` const param
; folds through a RECURSION. A NON-recursive Char op already folds via `core_of` (`Char.to-int`/`from-int`);
; the recursive engine runs the interpreter, which had no Char value, so `(const …)` over a recursive
; Char-param fn declined (cb04). Equality/ordering compare by scalar; `Char.to-int` reads it; `Char.from-int`
; builds one fallibly (`Option`, `None` for a surrogate / out-of-range int).
(case
  "a Char threaded through a recursion const-folds (cb04)"
  (doc
    "`walk n c` adds 1 per step down to `Char.to-int c` at the base; `(const (walk 3 #\\a))` folds to
           3 + 97 = 100 (`#\\a` scalar is 97). The Char const param binds a `CVal::Char` through the
           recursion — before, the interpreter had no Char value so the fold declined CDZ0201.")
  (input
    (do
      (def
        (walk (const (: n Int64)) (const (: c Char)))
        (if (= n 0) (Char.to-int c) (+ 1 (walk (- n 1) c))))
      (def (main) (const (walk 3 #\a)))
      (export main)))
  (output (: 100 Int64)))

(case
  "Char equality in a recursion counts matches under const"
  (doc
    "`cnt n c` counts, over `n` steps, how many equal `#\\a` — `(const (cnt 4 #\\a))` folds to 4. Pins
           Char `=` (scalar compare) composing with the recursive engine.")
  (input
    (do
      (def
        (cnt (const (: n Int64)) (const (: c Char)))
        (if (= n 0) 0 (if (= c #\a) (+ 1 (cnt (- n 1) c)) (cnt (- n 1) c))))
      (def (main) (const (cnt 4 #\a)))
      (export main)))
  (output (: 4 Int64)))

(case
  "Char.from-int then to-int round-trips under const"
  (doc
    "`(const (f 97))` folds to 97: `Char.from-int 97` = `(Some #\\a)`, matched and read back with
           `Char.to-int`. Pins the fallible `Int -> (Option Char)` folding through a const-param fn + match.")
  (input
    (do
      (def
        (f (const (: n Int64)))
        (match (Char.from-int n) ((Option.Some c) (Char.to-int c)) ((Option.None) -1)))
      (def (main) (const (f 97)))
      (export main)))
  (output (: 97 Int64)))

(case
  "Char.from-int of a surrogate folds to None under const"
  (doc
    "`(const (f 55296))` folds to -1: 55296 = U+D800 is a UTF-16 surrogate, not a scalar value, so
           `Char.from-int` yields `(None)` and the match takes the absent arm. Pins the fallible path's
           negative direction (out-of-range/surrogate) at compile time.")
  (input
    (do
      (def
        (f (const (: n Int64)))
        (match (Char.from-int n) ((Option.Some c) (Char.to-int c)) ((Option.None) -1)))
      (def (main) (const (f 55296)))
      (export main)))
  (output (: -1 Int64)))

; --- Primitive 2: const execution — a Char const param folds on the BARE-CALL path too ----------------
; The recursive-call ACTIVATION gate fires the interpreter for a bare `(f #\c …)` call (not just a `(const
; …)` block) when a `const` param is const-foldable and all args are const. A Char literal must be RECOGNIZED
; as a const value (`is_const_value`/`core_is_const_value` carry `Core::ConstChar`) for the gate's all-args-
; const check to pass — else the fold is skipped and a Char-returning recursive `f` reaches the emitter,
; which has no runtime Char representation. With that, the bare-call path is symmetric with Int64: a taken
; trap surfaces CDZ0304 (not a decline), and a value folds.
(case
  "a taken trap over a Char const param on the bare-call path surfaces CDZ0304"
  (doc
    "`(f #\\a 2)` counts `n` to 0 then traps; the bare recursive-call fold executes the trap and
           surfaces its message as CDZ0304 — symmetric with the Int64 shape. Before `Core::ConstChar` was a
           recognized const value, the gate skipped the fold and the Char-returning `f` failed to emit.")
  (input
    (do
      (def
        (f (const (: c Char)) (const (: n Int64)))
        (if (= n 0) (trap "char base reached") (f c (- n 1))))
      (def (main) (f #\a 2))
      (export main)))
  (error CDZ0304 (message "char base reached")))

(case
  "a Char const param folds a VALUE through a bare recursive call"
  (doc
    "`(f #\\a 3)` recurses `n` to 0 then reads `Char.to-int c` = 97 — the value form of the bare-call
           path (no `(const …)` block), folding a Char threaded through the recursion.")
  (input
    (do
      (def (f (const (: c Char)) (const (: n Int64))) (if (= n 0) (Char.to-int c) (f c (- n 1))))
      (def (main) (f #\a 3))
      (export main)))
  (output (: 97 Int64)))

; --- Primitive 2: const execution — a FLOAT value threads through the recursive engine (ca03) ---------
; The value-interpreter carries a `CVal::Float` (the exact `Decimal`, matching `Core::ConstFloat`), so a
; `Float64`/`Float32` const param folds through a RECURSION. Arithmetic (`+`/`-`/`*`/`/`) folds at the
; operation node's solved WIDTH exactly like `lower_float_arith` (Float32 rounds through binary32; a
; non-finite result declines); `=` is by canonical bits at the operand width (`-0.0` differs from `0.0`),
; `< <= > >=` by the IEEE partial order — both handled in the prim block, which has the node for the width.
; A non-recursive float op already folds via `core_of`; the recursive engine had no Float value (ca03).
(case
  "a Float64 accumulator threaded through a recursion const-folds"
  (doc
    "`fsum n acc` adds 1.5 per step down to `acc`; `(const (fsum 3 0.0))` folds to 4.5. The Float const
           param binds a `CVal::Float` through the recursion and each `+` folds at the node's Float64 width.")
  (input
    (do
      (def
        (fsum (const (: n Int64)) (const (: acc Float64)))
        (if (= n 0) acc (fsum (- n 1) (+ acc 1.5))))
      (def (main) (const (fsum 3 0.0)))
      (export main)))
  (output (: 4.5 Float64)))

(case
  "Float64 comparison in a recursion counts matches under const"
  (doc
    "`cnt n x` counts, over `n` steps, how many times `x < 2.0` — `(const (cnt 4 1.5))` folds to 4.
           Pins float `<` (IEEE partial order) composing with the recursive engine.")
  (input
    (do
      (def
        (cnt (const (: n Int64)) (const (: x Float64)))
        (if (= n 0) 0 (if (< x 2.0) (+ 1 (cnt (- n 1) x)) (cnt (- n 1) x))))
      (def (main) (const (cnt 4 1.5)))
      (export main)))
  (output (: 4 Int64)))

(case
  "a taken trap over a Float64 const param on the bare-call path surfaces CDZ0304"
  (doc
    "`(f 1.5 2)` counts `n` to 0 then traps; the bare recursive-call fold executes the trap and surfaces
           its message as CDZ0304 — a Float const value is recognized by `is_const_value` (`Core::ConstFloat`),
           so the activation gate fires (symmetric with Int64/Char).")
  (input
    (do
      (def
        (f (const (: x Float64)) (const (: n Int64)))
        (if (= n 0) (trap "float base reached") (f x (- n 1))))
      (def (main) (f 1.5 2))
      (export main)))
  (error CDZ0304 (message "float base reached")))

(case
  "Float64 equality by canonical bits folds through a recursion"
  (doc
    "`mk n` recurses to the constant `0.0`; `(const (= (mk 2) 0.0))` folds to true — float `=` under the
           canonical byte form, evaluated in the prim block at the operand width.")
  (input
    (do
      (def (mk (const (: n Int64))) (if (= n 0) 0.0 (mk (- n 1))))
      (def (main) (const (= (mk 2) 0.0)))
      (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — a field projection off a NULLARY const fn's record eliminates it ---
; A nullary def resolves its NAME to a `Ref` straight at its body (no `Lambda` wrapper), so the value-
; interpreter's `const_eval_apply` — which bound params via `lambda_params_of` (None for a nullary) — could
; not call it. So `descriptor().id`, where `descriptor()` returns a `contract(Ast.module)`-style record with
; `Ast`-typed sibling fields, did NOT fold: the whole record MATERIALIZED (its `Ast` fields have no runtime
; representation) and the component failed to instantiate. Evaluating a nullary def's body directly lets the
; field read reduce to that field's constant and DROP the unused siblings — so the operator's structured
; whole-record descriptor form (not just the `Bytes` form) instantiates. (Reported by v-platform.)
(case
  "a scalar field projected off a nullary const fn's record folds to the field, dropping the record"
  (doc
    "`descriptor()` returns `(record (id 42) (extra (quote …)))`; `(const (. (descriptor) id))` folds to
           42, the `extra` sibling never materialized. Before, the nullary call declined in the value-
           interpreter and the record built whole.")
  (input
    (do
      (def (descriptor) #record((= id 42) (= extra (quote (a b c)))))
      (def (main) (const (. (descriptor) id)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a Bytes field projected off a nullary record with Ast siblings folds byte-exact and eliminates the record"
  (doc
    "The operator's structured descriptor shape: `descriptor()` returns a record whose `id` is a
           `Blake3.of(Ast.encode …)` Bytes and whose siblings are `Ast` values (no runtime form). Reading
           `(. (descriptor) id)` — NO `(const …)` block, the ordinary member fold — reduces to the id Bytes
           (equal to the direct fold) and drops the Ast siblings, so the record never materializes. Before,
           the whole record built and the component failed to instantiate.")
  (input
    (do
      (def
        (descriptor)
        #record((= id (Blake3.of (Ast.encode (quote (contract c Int64 Int64)))))
          (= input (quote Int64))
          (= output (quote Int64))))
      (def (main) (= (. (descriptor) id) (Blake3.of (Ast.encode (quote (contract c Int64 Int64))))))
      (export main)))
  (output (: true Bool)))

(case
  "a field projection off a nullary record whose field came from a RECURSIVE helper folds"
  (doc
    "`descriptor()` builds `(record (id (leaves (quote …))) …)` where `leaves` is a RECURSIVE const fn;
           `(const (. (descriptor) id))` folds to 3. `descriptor` itself is not in a cycle (the recursion is
           at the HELPER, not back to `descriptor`), so the nullary-body evaluation fires and the helper folds
           via the param-recursion path — the shape of the operator's `descriptor() = contract(Ast.module)`,
           where `contract` recurses over the module forms. (A genuinely self-recursive nullary `(def (f) (f))`
           is instead declined at the recursion bound, so the fold never overflows the native stack.)")
  (input
    (do
      (def (leaves (const (: a Ast))) (match a ((Ast.List xs) (List.len xs)) (_ 1)))
      (def (descriptor) #record((= id (leaves (quote (f 1 2)))) (= extra (quote z))))
      (def (main) (const (. (descriptor) id)))
      (export main)))
  (output (: 3 Int64)))

; A WRAPPER-typed (Rational) field projected off a nullary record + fed to a rational op (#3543 regression, breaker-
; bisected). #3543 folds `(. (rect) w)` so the record is eliminated; a SCALAR field folds to a bare `ConstInt`, and
; an INTEGER-VALUED Rational field ALSO folds to a bare `ConstInt` (its numerator). The rational op over those
; operands did NOT recognize a `ConstInt` as the rational `n/1`, so it fell through to the RUNTIME `RationalBinOp`
; whose bare-`ConstInt` operand has no heap ownership class at emit → DECLINE ("borrowing op operand has an ownership
; this backend cannot yet prove"). This was the notebook Rational-field regression. `const_rational_of` now reads a
; `ConstInt(n)` as `n/1`, so the constant pair folds to a `Core::ConstRational` — no runtime op, no borrow
; classification. `(pragma default-fraction Rational)` makes the record fields heap Rationals (the notebook shape).
(case
  "a wrapper (Rational) field projected off a nullary record folds through a rational op — no borrow-ownership decline"
  (doc
    "`(rect)` returns `(record (= w (width)) (= h 3))` with Rational fields (default-fraction Rational). `(* (.
           (rect) w) (. (rect) h))` folds each projection to its integer-valued Rational (a `ConstInt`) and the
           multiply folds them as `4/1 * 3/1 = 12/1` — a `Core::ConstRational`, so the record is eliminated and NO
           runtime rational op (hence no borrow-ownership classification) is reached. Before, the bare-`ConstInt`
           operand of the runtime `RationalBinOp` had no ownership class and DECLINED.")
  (input
    (do
      (pragma default-fraction Rational)
      (def (width) 4)
      (def (rect) #record((= w (width)) (= h 3)))
      (def (main) (* (. (rect) w) (. (rect) h)))
      (export main)))
  (output (: 12/1 Rational)))

; --- Primitive 2: const execution — a Tuple const param folds on the BARE-CALL path (ca06) ------------
; The recursive-call activation gate accepts a `const` param whose type is a bare NAME or a SHRINKING type-
; constructor, but EXCLUDED `(Tuple …)` (grouped with the product/dictionary forms). A `const (: t (Tuple …))`
; param in a counter-in-tuple recursion genuinely SHRINKS, and the value-interpreter folds tuples — but the
; gate skipped it, so a `const` Tuple-param fn could not be called on the bare path (no `(const …)` block):
; it emitted a runtime fn whose `const` Tuple param has no runtime slot ("function return type has no machine
; representation"). Admitting `(Tuple …)` fires the fold, symmetric with the scalar const params. (`(Record …)`
; stays excluded — that is the ad-hoc-polymorphism dictionary-consumer shape, which inlines+erases at runtime.)
(case
  "a Tuple const param folds a VALUE through a bare recursive call (ca06)"
  (doc
    "`f` counts the first tuple slot to 0 while accumulating into the second; `(f (tuple 3 0))` — NO
           `(const …)` block — folds to 3. The `const` Tuple param binds a `CVal::Tuple` and the recursion
           folds; before, the gate excluded `(Tuple …)` so the `const`-param fn reached the emitter.")
  (input
    (do
      (def
        (f (const (: t (Tuple Int64 Int64))))
        (match t (#tuple(a b) (if (= a 0) b (f #tuple((- a 1) (+ b 1)))))))
      (def (main) (f #tuple(3 0)))
      (export main)))
  (output (: 3 Int64)))

(case
  "a taken trap over a Tuple const param on the bare-call path surfaces CDZ0304 (ca06)"
  (doc
    "`(f (tuple 2 0))` counts to 0 then traps; the bare recursive-call fold executes the trap and
           surfaces its message as CDZ0304 — symmetric with the Int64/Char/Float const-param shapes.")
  (input
    (do
      (def
        (f (const (: t (Tuple Int64 Int64))))
        (match t (#tuple(a b) (if (= a 0) (trap "tuple base reached") (f #tuple((- a 1) (+ b 1)))))))
      (def (main) (f #tuple(2 0)))
      (export main)))
  (error CDZ0304 (message "tuple base reached")))

; --- Primitive 2: const execution — the OPERATOR'S structured whole-record descriptor form ------------
; The platform's structured contract descriptor is a 5-field record `(id Bytes, name String, input Ast,
; output Ast, types Ast)` returned by a nullary `descriptor()` (= `contract(Ast.module)`); a guest reads
; ONE scalar field off it (`descriptor().id` for routing, `descriptor().name` for display). #3543 made the
; field projection ELIMINATE the record — the three non-representable `Ast` siblings never materialize — so
; the whole-record form instantiates + runs (not just the `Bytes` forms). These pin that end-to-end: BOTH a
; Bytes and a String field project off the same multi-Ast-sibling record and fold, with NO `(const …)` block
; (the ordinary member fold, exactly as a guest writes it). Regression guard for the #3413 structured form.
(case
  "the structured descriptor's String name field projects and eliminates the record"
  (doc
    "`descriptor()` returns the 5-field `(id name input output types)` record; `(. (descriptor) name)`
           folds to the name String and the `Ast`-typed siblings drop — no `(const …)` block. Before #3543 the
           record materialized (its `Ast` fields have no runtime form) and the component failed to instantiate.")
  (input
    (do
      (def
        (descriptor)
        #record((= id (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
          (= name "cdz-platform.temp-celsius")
          (= input (quote Int64))
          (= output (quote Int64))
          (= types (quote #list()))))
      (def (main) (= (. (descriptor) name) "cdz-platform.temp-celsius"))
      (export main)))
  (output (: true Bool)))

(case
  "the structured descriptor's Bytes id field projects byte-exact and eliminates the record"
  (doc
    "The routing path: `(. (descriptor) id)` off the same 5-field record folds to the `Blake3.of(Ast.encode
           …)` id Bytes (equal to the direct fold), dropping the `Ast` siblings — no `(const …)` block. Pins the
           operator's structured whole-record descriptor form folds + eliminates for the id, same as the name.")
  (input
    (do
      (def
        (descriptor)
        #record((= id (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
          (= name "cdz-platform.temp-celsius")
          (= input (quote Int64))
          (= output (quote Int64))
          (= types (quote #list()))))
      (def
        (main)
        (= (. (descriptor) id) (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64))))))
      (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — SET ALGEBRA + Set/Map removal fold (query-only) -------------------
; The value-interpreter folds `Set.union`/`intersection`/`difference` + `Set.remove` + `Map.remove`, the
; companions of the `Set.of`/`insert`/`contains`/`len` + `Map.*` ops it already carried. Membership is by
; `cval_eq`; an undecidable comparison declines the whole op (never a wrong member set). Like every const
; Set/Map these are QUERY-ONLY — the result set/map is never MATERIALIZED (`cval_to_core` declines a
; `CVal::Set`/`CVal::Map`, since the runtime CHAMP iteration order must not be presumed); only order-
; INDEPENDENT results (a size, a membership) leave the fold. A `(const …)` block forces the fold.
(case
  "Set.union const-folds a membership count"
  (doc
    "`{1,2} ∪ {2,3}` has 3 distinct members; the union folds and `Set.len` reads 3 (order-independent).")
  (input (do (def (main) (const (Set.len (Set.union #set(1 2) #set(2 3))))) (export main)))
  (output (: 3 Int64)))

(case
  "Set.intersection const-folds a membership count"
  (doc "`{1,2,3} ∩ {2,3,4}` = `{2,3}`; `Set.len` reads 2.")
  (input
    (do (def (main) (const (Set.len (Set.intersection #set(1 2 3) #set(2 3 4))))) (export main)))
  (output (: 2 Int64)))

(case
  "Set.difference const-folds a membership count"
  (doc "`{1,2,3} ∖ {2}` = `{1,3}`; `Set.len` reads 2.")
  (input (do (def (main) (const (Set.len (Set.difference #set(1 2 3) #set(2))))) (export main)))
  (output (: 2 Int64)))

(case
  "Set.remove then contains const-folds to false"
  (doc "Removing `2` from `{1,2,3}` then testing membership of `2` folds to false.")
  (input (do (def (main) (const (Set.contains (Set.remove #set(1 2 3) 2) 2))) (export main)))
  (output (: false Bool)))

(case
  "Map.remove then len const-folds"
  (doc "Removing key `1` from a 2-entry map leaves 1 entry; `Map.len` reads 1.")
  (input
    (do
      (def (main) (const (Map.len (Map.remove (Map.insert (Map.insert (Map.empty) 1 10) 2 20) 1))))
      (export main)))
  (output (: 1 Int64)))

(case
  "Map.remove then lookup misses"
  (doc "Looking up the removed key `1` folds to `Option.None` (the match's absent arm → -1).")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Map.lookup (Map.remove (Map.insert (Map.empty) 1 10) 1) 1)
            ((Option.Some v) v)
            ((Option.None) -1))))
      (export main)))
  (output (: -1 Int64)))

; --- Primitive 2: const execution — COMPOUND keys/elements in a const Map/Set fold via structural cval_eq --
; The const Map/Set cases above key on SCALARS; membership there is a scalar `cval_eq`. A KEY / element can
; also be a COMPOUND — a tuple, record, or sum — and `apply_const_prim`'s `cval_eq` compares those STRUCTURALLY
; (element-/field-/payload-wise; the sums/records/tuples arm from the Option-nav fold work). These pin that a
; const Map keyed on a compound folds its `Map.lookup`, that a compound key the map lacks folds to `None`, and
; that a const Set of compounds DEDUPS by structural equality — the subtle surface where a `cval_eq` regression
; would silently MISMATCH keys (wrong value, or a spurious dedup). A key comparison the stage cannot decide
; declines the whole op (never a wrong verdict), so these witness the DECIDABLE compound-key path folds.
(case
  "a const Map keyed on a TUPLE folds a lookup by structural key equality"
  (doc
    "`Map.lookup` over `{(1,2)↦10, (3,4)↦20}` at key `(3,4)` folds to `Some 20` — `cval_eq` compares the
        tuple keys element-wise, matching the second entry. Pins compound-tuple-key const Map folding.")
  (input
    (const
      (match
        (Map.lookup (Map.insert (Map.insert Map.empty #tuple(1 2) 10) #tuple(3 4) 20) #tuple(3 4))
        ((Option.Some v) v)
        ((Option.None) 0))))
  (output (: 20 Int64)))

(case
  "a const Map lookup of an absent TUPLE key folds to None"
  (doc
    "At key `(9,9)`, absent from `{(1,2)↦10}`, the lookup folds to `None` (absent arm → -1) — `cval_eq`
        DECIDES the tuple keys unequal element-wise. The negative companion of the found case above.")
  (input
    (const
      (match
        (Map.lookup (Map.insert Map.empty #tuple(1 2) 10) #tuple(9 9))
        ((Option.Some v) v)
        ((Option.None) -1))))
  (output (: -1 Int64)))

(case
  "a const Set of TUPLES dedups by structural equality"
  (doc
    "`Set.of (list (1,2) (1,2) (3,4))` folds to a 2-member set — `cval_eq` finds the two `(1,2)` tuples
        equal element-wise and dedups them, leaving `{(1,2),(3,4)}`; `Set.len` reads 2.")
  (input (const (Set.len #set(#tuple(1 2) #tuple(1 2) #tuple(3 4)))))
  (output (: 2 Int64)))

(case
  "a const Set of RECORDS dedups by structural field equality"
  (doc
    "`Set.of (list (record (a 1)) (record (a 1)) (record (a 2)))` folds to 2 — `cval_eq` compares records
        field-wise, deduping the two `{a:1}` records. Pins the record arm of structural const-Set dedup.")
  (input (const (Set.len #set(#record((= a 1)) #record((= a 1)) #record((= a 2))))))
  (output (: 2 Int64)))

(case
  "a const Map keyed on a SUM variant folds a lookup by structural key equality"
  (doc
    "`Map.lookup` over `{(Option.Some 5)↦42}` at `(Option.Some 5)` folds to `Some 42` — `cval_eq` compares
        the sum keys by discriminant then payload. Pins the sum arm of compound-key const Map folding.")
  (input
    (const
      (match
        (Map.lookup (Map.insert Map.empty (Option.Some 5) 42) (Option.Some 5))
        ((Option.Some v) v)
        ((Option.None) 0))))
  (output (: 42 Int64)))

; --- Primitive 2: const execution — a const Set.to-list AND Map.to-list FOLD in CANONICAL VALUE ORDER ---------
; A to-list is ORDER-EXPOSING, so folding it is sound ONLY if the compiler bakes the SAME order the runtime op
; produces. That order is NOT a presumed CHAMP layout: `collections-and-text.md` §Set/Map Iteration Is
; Deterministic pins it MUST-level as the canonical VALUE total order (the visit order MUST agree with the
; canonical byte form), and the runtime `set-to-list`/`map-to-list` ops explicitly re-sort by that order
; (`value_cmp_shaped`), which the compiler's `const_key_order` is the same order (v-runtime confirmed it a
; CONTRACT, not an implementation detail; runtime witnesses pinned in 19-sets by breaker #3749). So a NON-empty
; CONSTANT `Set.to-list` / `Map.to-list` now FOLD to a canonically-ordered list — byte-matching the runtime op —
; turning a `(const … to-list …)` DEMAND from a REJECT into a fold. `Map.to-list` yields a list of `(key value)`
; TUPLES in canonical KEY order (the op's `(List (Tuple K V))` shape). A key/element the canonical order cannot
; rank as a constant (float / bytes / nested-collection / a runtime value) keeps the runtime op (`const_key_order`
; declines EXACTLY those, matching the runtime op's own non-orderable decline).
(case
  "a const Map.to-list folds its length under a const demand"
  (doc
    "`Map.to-list` over a constant map folds to a key-sorted list of `(k v)` tuples, so `(const (List.len
           (Map.to-list …)))` folds to 2 — no longer a REJECT. The Map twin of the Set.to-list fold.")
  (input
    (do
      (def (main) (const (List.len (Map.to-list (Map.insert (Map.insert #map() 2 20) 1 10)))))
      (export main)))
  (output (: 2 Int64)))

(case
  "a const Map.to-list enumerates key-sorted — the head entry is the smallest key"
  (doc
    "The entries materialize as `(tuple key value)` in canonical KEY order, so the head of a 3-entry map
           `{3↦30,1↦10,2↦20}` is `(tuple 1 10)`: `(+ (* 100 k) v)` = 110. Pins the (k v) tuple element shape +
           the key-sorted order.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (List.at (Map.to-list (Map.insert (Map.insert (Map.insert #map() 3 30) 1 10) 2 20)) 0)
            ((Option.Some #tuple(k v)) (+ (* 100 k) v))
            ((Option.None) -1))))
      (export main)))
  (output (: 110 Int64)))

(case
  "a const Map.to-list carries the value at each index"
  (doc
    "The value rides with its key at every position: index 1 of `{5↦50,2↦20}` (key-sorted) is
           `(tuple 5 50)`, `(+ k v)` = 55.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (List.at (Map.to-list (Map.insert (Map.insert #map() 5 50) 2 20)) 1)
            ((Option.Some #tuple(k v)) (+ k v))
            ((Option.None) -1))))
      (export main)))
  (output (: 55 Int64)))

(case
  "a const Map.to-list sorts STRING keys lexicographically"
  (doc
    "String keys sort lexicographically (`\"ab\" < \"bb\"`), so the head of `{\"bb\"↦2,\"ab\"↦1}` is
           `(tuple \"ab\" 1)` — value 1. Pins the String-key canonical order for the Map fold.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (List.at (Map.to-list (Map.insert (Map.insert #map() "bb" 2) "ab" 1)) 0)
            ((Option.Some #tuple(k v)) v)
            ((Option.None) -1))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a const Map.to-list order byte-matches the RUNTIME map-to-list op (cross-check)"
  (doc
    "The belt-and-suspenders soundness cross-check (the #3765 shape for Map): a RUNTIME `Map.to-list` (the
           map built through a runtime `Map.insert n`, forcing the heap `map-to-list` op) yields the SAME list of
           `(k v)` tuples as the COMPILE-TIME fold — both key-sorted. Pins that `const_key_order` (compile) and
           `value_cmp_shaped` (runtime) agree for Map keys, catching any drift between the two impls.")
  (input
    (do
      (def
        (run (: n Int64))
        (=
          #tuple(1 (Map.to-list (Map.insert (Map.insert #map() n 10) (+ n 1) 20)))
          #tuple(1 (const (Map.to-list (Map.insert (Map.insert #map() 3 10) 4 20))))))
      (export run)))
  (call run 3)
  (output (: true Bool)))

(case
  "a const Set.to-list folds to a canonically-sorted list under a const demand"
  (doc
    "`Set.to-list` over a CONSTANT set folds to its elements in canonical VALUE order (spec-pinned, ==
           the runtime `set-to-list` op's order), so `(const (List.len (Set.to-list (Set.of (list 1 2 3)))))`
           folds to 3 — no longer a REJECT. Was the CHAMP-order soundness negative; now a fold, sound because
           `const_key_order` byte-matches the runtime op (v-runtime contract + breaker #3749 witnesses).")
  (input (do (def (main) (const (List.len (Set.to-list #set(1 2 3))))) (export main)))
  (output (: 3 Int64)))

(case
  "a const Set.to-list sorts ints in canonical (numeric-ascending) value order, dedup'd"
  (doc
    "`(Set.of (list 3 1 2 3))` folds to the 3-member set `{1,2,3}`; `Set.to-list` materializes them in
           canonical value order `(1 2 3)` — insertion order and the duplicate `3` are both erased. Pins that
           the const fold's element ORDER is the canonical numeric-ascending order.")
  (input (const (= (Set.to-list #set(3 1 2 3)) #list(1 2 3))))
  (output (: true Bool)))

(case
  "a const Set.to-list orders negative ints numerically, not by encoding"
  (doc
    "Negatives sort by NUMERIC value (`-3 < 0 < 2 < 5`), not a two's-complement byte order — the canonical
           value order is numeric. `(Set.to-list (Set.of (list 5 -3 0 -3 2)))` folds to `(-3 0 2 5)`.")
  (input (const (= (Set.to-list #set(5 -3 0 -3 2)) #list(-3 0 2 5))))
  (output (: true Bool)))

(case
  "a const Set.to-list orders strings lexicographically"
  (doc
    "String elements sort lexicographically (the canonical String value order). `(Set.to-list (Set.of
           (list \"banana\" \"apple\" \"cherry\")))` folds to `(\"apple\" \"banana\" \"cherry\")`.")
  (input (const (= (Set.to-list #set("banana" "apple" "cherry")) #list("apple" "banana" "cherry"))))
  (output (: true Bool)))

(case
  "a const Set.to-list order byte-matches the RUNTIME set-to-list op (cross-check)"
  (doc
    "The belt-and-suspenders soundness cross-check: a RUNTIME `Set.to-list` (the set built through a
           runtime `Set.insert n`, forcing the heap `set-to-list` op) yields the SAME order as the COMPILE-TIME
           fold of `{1,2,3}` — both `(1 2 3)`. Pins that `const_key_order` (compile) and `value_cmp_shaped`
           (runtime) agree, catching any implementation drift between the two impls of the one spec'd order.")
  (input
    (do
      (def (run (: n Int64)) (= (Set.to-list (Set.insert #set(3 1) n)) (Set.to-list #set(1 2 3))))
      (export run)))
  (call run 2)
  (output (: true Bool)))

; --- Primitive 2: const Set/Map.to-list folds TUPLE elements/keys by element-wise lexicographic order --------
; A tuple orders ELEMENT-WISE lexicographically (position 0, then 1, …), recursing through the canonical value
; order — the runtime to-list tuple order (19-sets). `const_key_order`/`cval_key_order` now rank tuples, so a
; const Set/Map of tuples materializes its to-list byte-matching the runtime (the operator wants full generality
; across ALL shapes). A tuple whose element the canonical order cannot rank (a float / nested collection) still
; declines. (Sum + record element ordering land in the following sections — every runtime-orderable shape now folds.)
(case
  "a const Set.to-list of TUPLE elements folds, dedup'd, in element-wise lexicographic order"
  (doc
    "`{(3,30),(1,99),(1,10)}` sorts lexicographically — first by element 0, ties broken by element 1 — so
           the head is `(1,10)` (not `(1,99)`): `(+ (* 100 k) v)` = 110. Dedups a repeated tuple. Pins the
           element-wise recursive tuple order matching the runtime (19-sets).")
  (input
    (do
      (def
        (main)
        (const
          (+
            (*
              1000
              (List.len (Set.to-list #set(#tuple(3 30) #tuple(1 99) #tuple(1 10) #tuple(1 10)))))
            (match
              (List.at (Set.to-list #set(#tuple(3 30) #tuple(1 99) #tuple(1 10))) 0)
              ((Option.Some #tuple(k v)) (+ (* 100 k) v))
              ((Option.None) -1)))))
      (export main)))
  (output (: 3110 Int64)))

(case
  "a const Set.to-list of TUPLE elements byte-matches the RUNTIME set-to-list (cross-check)"
  (doc
    "The soundness cross-check: a RUNTIME tuple Set.to-list (built via a runtime Set.insert of `(tuple 2
           n)`) equals the COMPILE-TIME fold of `{(1,10),(2,20),(3,30)}` — both lexicographically ordered.
           Pins that `const_key_order`'s recursive Tuple arm agrees with the runtime tuple order.")
  (input
    (do
      (def
        (run (: n Int64))
        (=
          (Set.to-list (Set.insert #set(#tuple(1 10) #tuple(3 30)) #tuple(2 n)))
          (const (Set.to-list #set(#tuple(1 10) #tuple(2 20) #tuple(3 30))))))
      (export run)))
  (call run 20)
  (output (: true Bool)))

; --- Primitive 2: const Set/Map.to-list folds SUM elements/keys by discriminant-then-payload order -----------
; A sum (Option / Result / user variant) orders by DISCRIMINANT first (the variant index), then by PAYLOAD
; within the same variant — the runtime `value_cmp_shaped` Sum order. `const_key_order`/`cval_key_order` now
; rank sums, so a const Set/Map of sums materializes its to-list byte-matching the runtime (operator: full
; generality across ALL shapes). A payload the canonical order cannot rank still declines.
(case
  "a const Set.to-list of SUM (Option) elements folds by discriminant-then-payload, dedup'd"
  (doc
    "`{Some 5, None, Some 1}` folds to 3 members; `Set.to-list` orders the `Some` variant (by payload:
           1 before 5) before/after `None` by discriminant, and the head here is `Some 1` (the Some disc sorts
           first, smallest payload). Pins disc-first-then-payload + dedup of a repeated `Some`.")
  (input
    (do
      (def
        (main)
        (const
          (+
            (*
              1000
              (List.len
                (Set.to-list #set((Option.Some 5) (Option.None) (Option.Some 1) (Option.Some 1)))))
            (match
              (List.at (Set.to-list #set((Option.Some 5) (Option.None) (Option.Some 1))) 0)
              ((Option.Some (Option.Some v)) v)
              ((Option.Some (Option.None)) -100)
              ((Option.None) -1)))))
      (export main)))
  (output (: 3001 Int64)))

(case
  "a const Set.to-list of SUM elements byte-matches the RUNTIME set-to-list (cross-check)"
  (doc
    "The soundness cross-check: a RUNTIME `Option` Set.to-list (built via a runtime `Set.insert (Option.Some
           n)`) equals the COMPILE-TIME fold of `{Some 1, Some 5, None}` — both disc-then-payload ordered. Pins
           that `const_key_order`'s Sum arm agrees with the runtime `value_cmp_shaped` Sum order.")
  (input
    (do
      (def
        (run (: n Int64))
        (=
          (Set.to-list (Set.insert #set((Option.Some 1) (Option.None)) (Option.Some n)))
          (const (Set.to-list #set((Option.Some 1) (Option.Some 5) (Option.None))))))
      (export run)))
  (call run 5)
  (output (: true Bool)))

; --- Primitive 2: const Set/Map.to-list folds RECORD elements/keys in canonical (name-lexicographic) field order
; A record orders FIELD-WISE in its CANONICAL field order — name-lexicographic (`Symbol` orders by name), which is
; the descriptor field order the runtime `value_cmp_shaped` Record walk uses ("the same canonical order equality/
; encode use") AND the iteration order of the compiler's `BTreeMap<Symbol, _>` record rep. `const_key_order`/
; `cval_key_order` now rank records by that order, so a const Set/Map of records materializes its to-list
; byte-matching the runtime. This is the LAST shape — the fold now covers every shape the runtime orders (operator:
; full generality). These pin it DISCRIMINATED against source/decl order: the record is written `lo`-first but
; orders `hi`-first (h < l), so the head is the `hi`-smallest, not the `lo`-smallest.
(case
  "a const Set.to-list of RECORD elements folds in canonical (name-lexicographic) field order, dedup'd"
  (doc
    "`{(lo 9, hi 1), (lo 0, hi 2), (lo 9, hi 1)}` — a 2-field record written `lo`-FIRST in source, but records
           order by the CANONICAL name-lexicographic field order (`hi` before `lo`), so the compare reads `hi` first:
           `hi 1` < `hi 2` makes `{hi:1,lo:9}` the head (NOT the `lo`-smaller `{hi:2,lo:0}` a source/decl order would
           pick). Reads the head's `lo` (9) and confirms len 2 (the repeat deduped): `2*1000 + 9`. Pins record
           field-wise order in the name-canonical order matching the runtime, discriminated against source order.")
  (input
    (do
      (def
        (main)
        (const
          (+
            (*
              1000
              (List.len
                (Set.to-list
                  #set(#record((= lo 9) (= hi 1))
                    #record((= lo 0) (= hi 2))
                    #record((= lo 9) (= hi 1))))))
            (match
              (List.at (Set.to-list #set(#record((= lo 9) (= hi 1)) #record((= lo 0) (= hi 2)))) 0)
              ((Option.Some r) r.lo)
              ((Option.None) -1)))))
      (export main)))
  (output (: 2009 Int64)))

(case
  "a const Set.to-list of RECORD elements byte-matches the RUNTIME set-to-list (cross-check)"
  (doc
    "The soundness cross-check: a RUNTIME record Set.to-list (built via a runtime `Set.insert` of a record
           whose `hi` field is a runtime `n`) equals the COMPILE-TIME fold of the same 2-record set — both order the
           records field-wise in the canonical (name-lexicographic) field order (`hi` before `lo`). Pins that
           `const_key_order`'s Record arm agrees with the runtime `value_cmp_shaped` Record order (descriptor field
           order == name-canonical == the compiler's `BTreeMap<Symbol>` iteration order).")
  (input
    (do
      (def
        (run (: n Int64))
        (=
          (Set.to-list (Set.insert #set(#record((= lo 9) (= hi 1))) #record((= lo 0) (= hi n))))
          (const (Set.to-list #set(#record((= lo 9) (= hi 1)) #record((= lo 0) (= hi 2)))))))
      (export run)))
  (call run 2)
  (output (: true Bool)))

; --- Primitive 2: const three-way (Ordering.of) folds a constant COMPOUND pair to an Ordering -----------------
; `Ordering.of a b` (the namespaced `compare`) is the three-way comparison yielding `Ordering` (Less/Equal/Greater).
; Its scalar fold (Int/Bool/String/Char/Float) already lands; a constant COMPOUND pair (tuple/record/sum of constant
; leaves) now folds too, through the SAME `const_key_order` canonical value order `Set.to-list`/equality use — which
; mirrors the runtime `value_cmp_shaped` the compound `value-cmp` walk uses, so the fold and the runtime walk report
; the SAME Ordering (core-semantics §331). Pinned under a `(const ...)` demand: without the compound fold the compare
; lowers to a runtime `value-cmp` (NOT a constant) and the general evaluator has no `compare` arm, so the const demand
; would reject — these cases PROVE the fold. Float/bytes-less/set/map leaf still declines (no total order), as it must.
(case
  "a const (Ordering.of) of two constant TUPLES folds to an Ordering under the const demand"
  (doc
    "`Ordering.of (tuple 1 2) (tuple 1 3)`: tuples order element-wise lexicographically — position 0 is equal
           (1=1), so position 1 decides 2 < 3 → Less. The `(const ...)` demand forces a compile-time Ordering (a nullary
           Less/Equal/Greater sum) via `const_key_order`, else it rejects — so this pins the compound compare fold.
           Matches the folded Ordering to an Int (Less → 1).")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Ordering.of #tuple(1 2) #tuple(1 3))
            ((Ordering.Less _) 1)
            ((Ordering.Equal _) 2)
            ((Ordering.Greater _) 3))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a const (Ordering.of) of two constant RECORDS folds in canonical (name-lexicographic) field order"
  (doc
    "Discriminated against source/decl order: both records are written `lo`-FIRST, but records order FIELD-WISE
           in the canonical name-lexicographic field order (`hi` before `lo`). `Ordering.of {lo 9, hi 1} {lo 0, hi 2}`
           reads `hi` first: `hi 1` < `hi 2` → Less, IGNORING that `lo 9` > `lo 0` (which a source/decl order would let
           decide → Greater). The const demand forces the fold via `const_key_order`'s Record arm; Less → 1 (a wrong
           field order would yield Greater → 3).")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Ordering.of #record((= lo 9) (= hi 1)) #record((= lo 0) (= hi 2)))
            ((Ordering.Less _) 1)
            ((Ordering.Equal _) 2)
            ((Ordering.Greater _) 3))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a const (Ordering.of) of two same-discriminant SUMS folds by payload"
  (doc
    "Sums order by DISCRIMINANT first, then payload. `Ordering.of (Option.Some 5) (Option.Some 3)`: same
           discriminant (Some), so the payload decides 5 > 3 → Greater. The const demand forces the compound sum fold
           via `const_key_order`; Greater → 3.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Ordering.of (Option.Some 5) (Option.Some 3))
            ((Ordering.Less _) 1)
            ((Ordering.Equal _) 2)
            ((Ordering.Greater _) 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a const (Ordering.of) of two constant tuples byte-matches the RUNTIME value-cmp (cross-check)"
  (doc
    "Soundness cross-check: a RUNTIME three-way `Ordering.of` over a tuple with a runtime element `n` equals the
           COMPILE-TIME fold of the same constant tuple pair — both order the tuples element-wise lexicographically and
           map the result to the same Ordering (core-semantics §331). Pins that `const_key_order`'s compound arm agrees
           with the runtime `value_cmp_shaped` the `value-cmp` walk emits.")
  (input
    (do
      (def
        (rank (: o Ordering))
        (match o ((Ordering.Less _) 1) ((Ordering.Equal _) 2) ((Ordering.Greater _) 3)))
      (def
        (run (: n Int64))
        (=
          (rank (Ordering.of #tuple(1 n) #tuple(1 3)))
          (const (rank (Ordering.of #tuple(1 2) #tuple(1 3))))))
      (export run)))
  (call run 2)
  (output (: true Bool)))

(case
  "coc1 a compound (Ordering.of) does NOT elide a runtime operand's construction effect (fold effect-safety guard)"
  (doc
    "The compound compare fold DISCARDS its operands (it returns a bare Ordering variant), so it fires ONLY when
           BOTH are fully-constant values — never when an operand carries a runtime subterm whose construction can trap or
           perform. Here the order of `(Option.Some <payload>)` vs `(Option.None)` is decided by the differing
           DISCRIMINANT alone, so a fold reading only the discriminant would drop the payload and SKIP its effect. The
           fully-const `is_const_value` guard keeps this runtime (the payload depends on `k`): `go 0` constructs the Some
           payload, whose trapping `(/ 1 k)` fires BEFORE the compare — proving the operand's effect is preserved,
           not elided (a division by zero at `k = 0`).")
  (input
    (do
      (def
        (go (: k Int64))
        (match
          (Ordering.of (Option.Some (/ 1 k)) (: (Option.None unit) (Option Int64)))
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (export go)))
  (call go 0)
  (trap "division by zero"))

(case
  "coc2 an (Ordering.of) of a Char-leaf compound orders by codepoint (Char is a blessed leaf)"
  (doc
    "The three-way companion of 03-equality's `<` Char-leaf order (one total order, two ways — §331): a Char
           leaf is orderable in a compound for BOTH the boolean `<` and the three-way `Ordering.of`, because a
           Char has a total order by codepoint (scalar `Ordering.of #\\a #\\b` is blessed and folds).
           `Ordering.of (tuple 1 #\\a) (tuple 1 #\\b)` → the first components tie (1 = 1), the Char leaf decides:
           #\\a (U+0061) < #\\b (U+0062) → `Ordering.Less` → the arm yields 1. Pins that the compound-ordering
           fold + the runtime walk share ONE blessed-leaf vocabulary (`is_orderable_compound`:
           Int/Bool/String/Symbol/Bytes/Char + nested, NOT Float/Set/Map), so const and runtime agree (§331) —
           Char was blessed into the walk like Bytes/PR#1120 (compiler-only: the runtime `value_cmp_shaped`
           already orders a Char-in-compound as its codepoint `Shape::Int`).")
  (input
    (do
      (def
        (main)
        (match
          (Ordering.of #tuple(1 #\a) #tuple(1 #\b))
          ((Ordering.Less _) 1)
          ((Ordering.Equal _) 2)
          ((Ordering.Greater _) 3)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

; --- Primitive 2: const boolean ORDERING (`<`/`<=`/`>`/`>=`) folds a constant COMPOUND pair --------------------
; The boolean ordering operators are the §331 companion of the three-way `Ordering.of`: a type surfaces ONE total
; order two ways that cannot disagree. The compound `=` fold already lands; a constant COMPOUND `<`/`<=`/`>`/`>=`
; now folds too, through the SAME `const_key_order` canonical value order (mirroring the runtime `value_cmp_shaped`
; the `value-cmp` walk uses), guarded on both operands being fully constant (the fold discards them, so a runtime
; trapping subterm must not be elided). Pinned under a `(const ...)` demand — without the fold the `<` lowers to a
; runtime `value-cmp` (not a constant) and the demand rejects — plus a §331 agreement check vs `Ordering.of`.
(case
  "a const (<) of two constant TUPLES folds to a Bool under the const demand"
  (doc
    "`(< (tuple 1 2) (tuple 1 3))`: tuples order element-wise — position 0 equal, position 1 decides 2 < 3 →
           true. The `(const ...)` demand forces the compile-time Bool via `const_key_order`, else it rejects, so this
           pins the compound ordering fold (the boolean-operator twin of the `Ordering.of` tuple case above).")
  (input (do (def (main) (const (if (< #tuple(1 2) #tuple(1 3)) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "a const (>=) of two constant RECORDS folds in canonical (name-lexicographic) field order"
  (doc
    "Discriminated against source/decl order: both records are written `lo`-FIRST but order by the canonical
           name-lexicographic field order (`hi` before `lo`). `(>= {lo 9, hi 1} {lo 0, hi 2})` reads `hi` first:
           `hi 1` < `hi 2` → the record is LESS, so `>=` is false → 0, IGNORING that `lo 9` > `lo 0` (a source/decl
           order would make `>=` true → 1). Pins record field-wise ordering in the boolean operator.")
  (input
    (do
      (def (main) (const (if (>= #record((= lo 9) (= hi 1)) #record((= lo 0) (= hi 2))) 1 0)))
      (export main)))
  (output (: 0 Int64)))

(case
  "a const (>) of two same-discriminant SUMS folds by payload"
  (doc
    "Sums order by discriminant then payload. `(> (Option.Some 5) (Option.Some 3))`: same discriminant (Some),
           payload 5 > 3 → true → 1. The const demand forces the compound sum ordering fold via `const_key_order`.")
  (input (do (def (main) (const (if (> (Option.Some 5) (Option.Some 3)) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "cbo1 a const compound (<) agrees with the three-way (Ordering.of) on the same operands (§331)"
  (doc
    "The one-total-order-two-ways invariant: `(< a b)` is true EXACTLY when `Ordering.of a b` is Less, for the
           same constant compound operands. Compares the boolean fold against the three-way fold's Less arm over
           `(tuple 2 (Option.Some 1))` vs `(tuple 2 (Option.Some 9))` (position 0 equal, then Some 1 < Some 9 → Less);
           both must fold to agree → true. Pins that the two compound folds surface the SAME order (cannot diverge).")
  (input
    (do
      (def
        (main)
        (const
          (=
            (< #tuple(2 (Option.Some 1)) #tuple(2 (Option.Some 9)))
            (match
              (Ordering.of #tuple(2 (Option.Some 1)) #tuple(2 (Option.Some 9)))
              ((Ordering.Less _) true)
              ((Ordering.Equal _) false)
              ((Ordering.Greater _) false)))))
      (export main)))
  (output (: true Bool)))

(case
  "cbo2 a compound (<) does NOT elide a runtime operand's construction effect (fold effect-safety guard)"
  (doc
    "The boolean-operator twin of coc1: the compound `<` fold discards its operands, so it fires only when both
           are fully constant. Here the order of `(Option.Some <payload>)` vs `(Option.None)` is decided by the
           discriminant alone, but the fully-const guard keeps this runtime (the payload depends on `k`): `go 0`
           constructs the Some payload, whose `(/ 1 0)` traps 'division by zero' before the compare — the effect is
           preserved, not elided.")
  (input
    (do
      (def
        (go (: k Int64))
        (if (< (Option.Some (/ 1 k)) (: (Option.None unit) (Option Int64))) 1 0))
      (export go)))
  (call go 0)
  (trap "division by zero"))

; --- Primitive 2: const LIST ordering folds — element-wise lexicographic, prefix-then-length tiebreak ---------
; A List orders LEXICOGRAPHICALLY (element-wise, first non-equal decides; a proper prefix is less — shorter wins on
; a common prefix), the runtime `value_cmp_shaped` List order (03-equality "a runtime list orders lexicographically"
; + "a proper prefix orders less"; Rust `[T]: Ord`). `const_key_order`/`cval_key_order` gained a List arm, so a
; fully-constant list comparison (`<`/`<=`/`>`/`>=`/`Ordering.of`) folds — bare lists and list-leaf compounds alike,
; gated (like the other compound-ordering folds) on `is_orderable_compound` so a Char/Float-in-list still declines.
(case
  "a const (<) of two constant LISTS folds element-wise under the const demand"
  (doc
    "`(< (list 1 2) (list 1 3))`: element 0 equal (1=1), element 1 decides 2 < 3 → true. The `(const ...)`
           demand forces the compile-time Bool via `const_key_order`'s List arm, else it rejects. The const face of
           03-equality:568 (which runs the runtime walk on inlined-const lists → same answer, now folded).")
  (input (do (def (main) (const (if (< #list(1 2) #list(1 3)) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "a const list PREFIX orders less than its extension (shorter-is-less tiebreak) under the const demand"
  (doc
    "The prefix/length tiebreak: `[1]` is a proper prefix of `[1,2]`, so `(< (list 1) (list 1 2))` → true (a
           shorter list on a common prefix is less — UNLIKE a fixed-arity tuple, where a length mismatch is a
           different type). Pins the List arm's `len().cmp()` fallthrough. The const face of 03-equality:580.")
  (input (do (def (main) (const (if (< #list(1) #list(1 2)) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "a const (Ordering.of) of two constant LISTS folds to an Ordering (longer-with-greater-tail)"
  (doc
    "`Ordering.of (list 3) (list 1)`: element 0 decides 3 > 1 → Greater (the length never matters once an
           element differs). Folds via `const_key_order`'s List arm under the const demand → Greater → 3.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Ordering.of #list(3) #list(1))
            ((Ordering.Less _) 1)
            ((Ordering.Equal _) 2)
            ((Ordering.Greater _) 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a const (<) of a LIST-leaf compound folds (nested list inside a tuple)"
  (doc
    "A list nested as a tuple element still folds: `(< (tuple 0 (list 1 2)) (tuple 0 (list 1 3)))` — tuple
           field 0 equal (0=0), field 1 is a List deciding `[1,2] < [1,3]` → true. Pins that `const_key_order`
           recurses tuple → list, and `is_orderable_compound` blesses a `(Tuple Int64 (List Int64))`.")
  (input
    (do (def (main) (const (if (< #tuple(0 #list(1 2)) #tuple(0 #list(1 3))) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "cll1 a const list ordering agrees with the RUNTIME list walk (cross-check, §331)"
  (doc
    "Soundness cross-check: a RUNTIME list `<` (over a list with a runtime element `n`) equals the COMPILE-TIME
           fold of the same constant lists — both order lexicographically. `go 2` builds `[1,2]` at runtime; `(< [1,n]
           [1,3])` = true, and the const `(< [1,2] [1,3])` = true → equal. Pins the List arm matches the runtime
           `value_cmp_shaped` order.")
  (input
    (do
      (def (run (: n Int64)) (= (< #list(1 n) #list(1 3)) (const (< #list(1 2) #list(1 3)))))
      (export run)))
  (call run 2)
  (output (: true Bool)))

(case
  "cll2 a const BARE empty list orders against a populated sibling under the const demand (element unified from the sibling)"
  (doc
    "The fold-side face of breaker olf4 (wrong-diagnostic #11): `(< (list) (list 1))` — the bare empty list's
           element type is `Int64` by UNIFICATION with the sibling, but `type_of` renders an unresolved element var
           for it, so an orderability check on that side ALONE mis-declined it as a float/set/map no-total-order case.
           `comparison_compound_ty` prefers the sibling's resolved `(List Int64)`, so the fold fires: `[]` is a proper
           prefix of `[1]`, so `[] < [1]` → true, folded under the `(const ...)` demand. No ascription needed.")
  (input (do (def (main) (const (if (< #list() #list(1)) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "cll3 a const (Ordering.of) of a bare empty vs populated list folds to Less (prefix), sibling-resolved"
  (doc
    "The three-way twin: `Ordering.of (list) (list 5)` — the empty list is a proper prefix, so it orders Less
           (shorter-is-less). The bare empty list's element type resolves from the sibling `(List Int64)` via
           `comparison_compound_ty`, so the fold fires under the const demand → Less → 1.")
  (input
    (do
      (def
        (main)
        (const
          (match
            (Ordering.of #list() #list(5))
            ((Ordering.Less _) 1)
            ((Ordering.Equal _) 2)
            ((Ordering.Greater _) 3))))
      (export main)))
  (output (: 1 Int64)))

; --- Primitive 2: const Set/Map.to-list folds BYTES elements/keys by unsigned byte-lexicographic order -------
; A `Bytes` element/key has a runtime canonical order pinned in 19-sets: UNSIGNED byte-lexicographic (0x80 sorts
; as 128, not signed −128). `const_key_order`/`cval_key_order` now rank `Bytes` by that same order (Rust
; `[u8]: Ord`), so a const Set/Map of Bytes materializes its to-list byte-matching the runtime op.
(case
  "a const Set.to-list of BYTES elements folds, dedup'd, in unsigned byte order"
  (doc
    "`{0x80, 0x05, 0x7f}` (single-byte Bytes) enumerates UNSIGNED as [5, 127, 128]: the head byte is 5 and
           the last is 128 (0x80 is 128 unsigned, NOT signed −128). Pins Bytes const-fold + the unsigned order
           matching 19-sets' runtime pin; also dedups (the len is 3 distinct).")
  (input
    (do
      (def
        (main)
        (const
          (+
            (*
              1000
              (List.len
                (Set.to-list #set((Bytes.of #list(128)) (Bytes.of #list(5)) (Bytes.of #list(127))))))
            (match
              (List.at
                (Set.to-list #set((Bytes.of #list(128)) (Bytes.of #list(5)) (Bytes.of #list(127))))
                2)
              ((Option.Some h) (match (Bytes.at h 0) ((Option.Some v) v) ((Option.None) -1)))
              ((Option.None) -2)))))
      (export main)))
  (output (: 3128 Int64)))

(case
  "a const Set.to-list of BYTES elements byte-matches the RUNTIME set-to-list (cross-check)"
  (doc
    "The soundness cross-check: a RUNTIME Bytes Set.to-list (built via a runtime Set.insert) equals the
           COMPILE-TIME fold of the same set — both order the single-byte Bytes {5, 9} identically. Pins that
           `const_key_order`'s Bytes arm (Rust [u8] cmp) agrees with the runtime's unsigned-byte order.")
  (input
    (do
      (def
        (run (: n Int64))
        (=
          (Set.to-list (Set.insert #set((Bytes.of #list(5))) (Bytes.of #list((UInt8.wrap n)))))
          (const (Set.to-list #set((Bytes.of #list(5)) (Bytes.of #list(9)))))))
      (export run)))
  (call run 9)
  (output (: true Bool)))

; --- Primitive 2: const Set.to-list folds CHAR elements by Unicode-scalar order (wasm-emit gap is orthogonal) -
; A `Char` element orders by Unicode SCALAR value; the RUNTIME to-list of a Char set sorts by that order on the
; rust targets (19-sets ckr1). The runtime WASM to-list op declines a Char element (a wasm-EMIT gap, v-rb's
; lane), but a const-fold BAKES the sorted list and never invokes that op — so the const fold works on ALL
; backends (a compile-time constant), sound because `const_key_order`'s Char arm (`char: Ord` = the scalar
; order) matches the rust runtime. (This is why the const fold was un-blockable independent of the wasm-emit gap.)
(case
  "a const Set.to-list of CHAR elements folds sorted by Unicode scalar, dedup'd"
  (doc
    "`{#\\c, #\\a, #\\b, #\\a}` folds to the 3-member set `{#\\a,#\\b,#\\c}`; `Set.to-list` materializes
           them in Unicode-scalar order, so the head is `#\\a` (scalar 97) and the length is 3 (the duplicate
           `#\\a` deduped). Pins the Char const-fold + scalar order (matching the rust runtime, 19-sets ckr1).")
  (input
    (do
      (def
        (main)
        (const
          (+
            (* 1000 (List.len (Set.to-list #set(#\c #\a #\b #\a))))
            (match
              (List.at (Set.to-list #set(#\c #\a #\b #\a)) 0)
              ((Option.Some h) (Char.to-int h))
              ((Option.None) -1)))))
      (export main)))
  (output (: 3097 Int64)))

; --- Primitive 2: const Set.to-list folds through the RECURSIVE engine + a const-param consumer -------------
; #3765 folds `Set.to-list` on a syntactic `Core::SetOf` (the `core_of` path). This extends the SAME
; canonical-order materialization to the const-EVALUATOR path (`apply_const_prim`'s `SetToList` arm over a
; `CVal::Set`), so a set the recursive engine BUILT (never a syntactic `Set.of`) also materializes, and the
; folded list flows into a const-param helper. (breaker batch 420 coverage gaps m7/m4.) Same non-orderable
; decline set (Char/Bytes/nested keep the runtime op).
(case
  "a const Set.to-list over a RECURSION-built set folds to the sorted list"
  (doc
    "`acc n` inserts `n` onto `(acc (n-1))` from a `Set.of (list)` base — the set is built by the
           recursive const engine (a `CVal::Set`, never a syntactic `Set.of`), so #3765's `core_of` fold does
           not see it. The const_eval `SetToList` arm materializes it: `(const (Set.to-list (acc 3)))` folds to
           `(1 2 3)` (canonical order), and `Set.len` reads 3. Pins the recursion-built materialization.")
  (input
    (do
      (def (acc (const (: n Int64))) (if (= n 0) #set() (Set.insert (acc (- n 1)) n)))
      (def (main) (const (= (Set.to-list (acc 3)) #list(1 2 3))))
      (export main)))
  (output (: true Bool)))

(case
  "a const Set.to-list result flows into a const-param helper"
  (doc
    "The folded list is a first-class `CVal::List`, so it feeds a `const`-param helper: `(f (Set.to-list
           (Set.of (list 3 1 2))))` where `f` reads `(List.len xs)` folds to 3. Pins that the materialized list
           composes with the const-execution engine (breaker gap m4's direct-consumer face).")
  (input
    (do
      (def (f (const (: xs (List Int64)))) (List.len xs))
      (def (main) (const (f (Set.to-list #set(3 1 2)))))
      (export main)))
  (output (: 3 Int64)))

; --- Primitive 2: a Set/Map ACCUMULATOR threaded through a const recursion + then QUERIED at the call site ---
; A recursion that threads a `Set`/`Map` as a `const`-param ACCUMULATOR and RETURNS it (then queried by
; Set.len/to-list/Map.lookup at the call site) used to REJECT: core_of's piecewise lowering of the recursive
; call const-evaluates the accumulator to a `CVal::Set`/`CVal::Map` that `cval_to_core` won't MATERIALIZE (the
; query-only soundness guard), so the recursive call declines — and the `(const …)` block surfaced that decline
; before trying whole-expression `const_eval`. Now the block tries `const_eval` on a non-trap decline, which
; folds the whole thing (the collection flows through the query as a `CVal`, never materialized). A provable
; ConstTrap still surfaces fail-loud (surfaced before the const_eval attempt). (breaker gaps m7/m7b.)
(case
  "a Set ACCUMULATOR threaded through a const recursion then Set.len folds"
  (doc
    "`grow` threads a `(Set Int64)` accumulator + an Int64 counter; `(const (Set.len (grow (Set.of (list))
           3)))` folds to 3 — the recursion builds `{7,14,21}` and `Set.len` reads it, via whole-expression
           const_eval (core_of's piecewise recursive-call fold declines on the un-materializable CVal::Set).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if (= k 0) s (grow (Set.insert s (* k 7)) (- k 1))))
      (def (main) (const (Set.len (grow #set() 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a Set ACCUMULATOR threaded through a const recursion then to-list folds (m7)"
  (doc
    "The to-list face: the same recursion-threaded set, materialized. `(const (List.len (Set.to-list
           (grow (Set.of (list)) 3))))` folds to 3 — const_eval folds the recursion to a CVal::Set, then
           Set.to-list (the const_eval arm) sorts+materializes it. Was a REJECT (piecewise decline surfaced).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if (= k 0) s (grow (Set.insert s (* k 7)) (- k 1))))
      (def (main) (const (List.len (Set.to-list (grow #set() 3)))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a Map ACCUMULATOR threaded through a const recursion then Map.lookup folds"
  (doc
    "The Map twin: `grow` threads a `(Map Int64 Int64)` accumulator; after building `{1↦10,2↦20,3↦30}`,
           `(const (match (Map.lookup (grow (map) 3) 2) …))` folds the value 20 — the accumulator flows through
           the lookup query as a CVal::Map, never materialized.")
  (input
    (do
      (def
        (grow (const (: m (Map Int64 Int64))) (const (: k Int64)))
        (if (= k 0) m (grow (Map.insert m k (* k 10)) (- k 1))))
      (def
        (main)
        (const (match (Map.lookup (grow #map() 3) 2) ((Option.Some v) v) ((Option.None) -1))))
      (export main)))
  (output (: 20 Int64)))

(case
  "csc1 a STRING comparison threaded through a const recursion folds (const_eval scalar-ordering arm)"
  (doc
    "The recursive-engine (const_eval) twin of the scalar String `<` fold: `core_of` folds a
           direct `(< \"b\" \"m\")`, but a String comparison INSIDE a recursion routes through the
           general evaluator's `apply_const_prim`, which handled Lt/Gt only for Int/Char — a String
           operand declined, so the whole `(const ...)` recursion rejected. Now it folds. `grow` threads
           a `(Set Int64)` accumulator (un-materializable → forces whole-expression const_eval) and gates
           each insert on `(< \"b\" hi)`: with `hi = \"m\"` the compare is TRUE each step, so it inserts
           {3,2,1} and `Set.len` reads 3. Was a REJECT (String Lt returned None → the fold declined).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)) (const (: hi String)))
        (if (= k 0) s (grow (Set.insert s (if (< "b" hi) k (- 0 k))) (- k 1) hi)))
      (def (main) (const (Set.len (grow #set() 3 "m"))))
      (export main)))
  (output (: 3 Int64)))

(case
  "csc2 a BOOL comparison threaded through a const recursion folds (const_eval scalar-ordering arm)"
  (doc
    "The Bool twin of csc1: `false < true`, so `(< low hi)` with `low = false`, `hi = true` is TRUE
           each step. The recursion threads a `(Set Int64)` accumulator (forces const_eval) and gates each
           insert on the Bool compare; folds `Set.len` to 3. Pins `apply_const_prim`'s new Bool ordering arm.")
  (input
    (do
      (def
        (grow
          (const (: s (Set Int64)))
          (const (: k Int64))
          (const (: low Bool))
          (const (: hi Bool)))
        (if (= k 0) s (grow (Set.insert s (if (< low hi) k (- 0 k))) (- k 1) low hi)))
      (def (main) (const (Set.len (grow #set() 3 false true))))
      (export main)))
  (output (: 3 Int64)))

(case
  "cso1 a three-way (Ordering.of) MATCHED in a const recursion folds (const_eval Compare arm + nullary-Ordering match)"
  (doc
    "`apply_const_prim` had no Compare arm, so a three-way `Ordering.of` threaded through a recursion declined
           — even though `core_of` folds it directly. `grow` gates each Set.insert on `(match (Ordering.of k 2) …)`;
           the un-materializable Set forces whole-expression const_eval. Two fixes land it: the const_eval Compare arm
           builds the Ordering variant at `ordering_discs`, and `const_pattern_matches` matches the payload-less
           Ordering variant against the corpus-conventional `(Ordering.Greater _)` placeholder. k∈{3,2,1} → Greater/
           Equal/Less → inserts {300,200,100} → Set.len 3. Was a REJECT (Compare unhandled in const_eval).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if
          (= k 0)
          s
          (grow
            (Set.insert
              s
              (match
                (Ordering.of k 2)
                ((Ordering.Less _) 100)
                ((Ordering.Equal _) 200)
                ((Ordering.Greater _) 300)))
            (- k 1))))
      (def (main) (const (Set.len (grow #set() 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "cso2 a COMPOUND three-way (Ordering.of) in a const recursion folds (const_eval Compare arm, compound path)"
  (doc
    "The compound face of cso1: `Ordering.of` on two TUPLES threaded through a const recursion. `(tuple k 0)`
           vs `(tuple 2 0)` orders by the first element (k vs 2), so k=3→Greater, k=2→Equal, k=1→Less → {300,200,100}
           → Set.len 3. Pins the const_eval Compare arm's compound path (via `cval_key_order`, gated on
           `is_orderable_compound`).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if
          (= k 0)
          s
          (grow
            (Set.insert
              s
              (match
                (Ordering.of #tuple(k 0) #tuple(2 0))
                ((Ordering.Less _) 100)
                ((Ordering.Equal _) 200)
                ((Ordering.Greater _) 300)))
            (- k 1))))
      (def (main) (const (Set.len (grow #set() 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "cbw1 a SHIFT (<<) threaded through a const recursion folds (const_eval delegation to core_of's fold_arith)"
  (doc
    "Bitwise/shift ops carry no hand-written `apply_const_prim` arm, but — unlike the member-headed
           `Ordering.of` — `<<` is a plain operator the const_eval DELEGATION can rebuild + refold through
           `core_of`'s `fold_arith`. `grow` threads `(<< k 1)` into a `(Set Int64)` accumulator (forces
           whole-expression const_eval): k∈{3,2,1} → {6,4,2} → `Set.len` 3. Pins that the delegation covers
           bitwise/shift in the recursive-fold path (no native arm needed).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if (= k 0) s (grow (Set.insert s (<< k 1)) (- k 1))))
      (def (main) (const (Set.len (grow #set() 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "cbw2 a bitwise AND (&) threaded through a const recursion folds (const_eval delegation)"
  (doc
    "The bitwise-AND twin of cbw1: `(& k 1)` is the low bit — k∈{3,2,1} → {1,0,1} → the SET dedups to
           {1,0} → `Set.len` 2. Confirms `&` folds in const_eval via the delegation (plain-operator refold).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if (= k 0) s (grow (Set.insert s (& k 1)) (- k 1))))
      (def (main) (const (Set.len (grow #set() 3))))
      (export main)))
  (output (: 2 Int64)))

(case
  "cdv1 integer DIV and REM threaded through a const recursion fold (const_eval arithmetic arm)"
  (doc
    "`apply_const_prim` folded Add/Sub/Mul but not `/`/`%`, so a recursion doing integer division declined
           and the `(const ...)` rejected — though `core_of` folds a direct `(/ n 10)`. `grow` extracts the base-10
           DIGITS of `n` by threading `(% n 10)` into a `(Set Int64)` accumulator and recursing on `(/ n 10)`;
           the un-materializable Set forces whole-expression const_eval, so `/`/`%` route through
           `apply_const_prim`. 12345 → digits {5,4,3,2,1}, `Set.len` = 5. Was a REJECT (Div/Rem returned None).")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: n Int64)))
        (if (< n 1) s (grow (Set.insert s (% n 10)) (/ n 10))))
      (def (main) (const (Set.len (grow #set() 12345))))
      (export main)))
  (output (: 5 Int64)))

(case
  "cdv2 a const-recursion divide-by-ZERO is fail-loud CDZ0304, not a silent decline"
  (doc
    "The soundness face: a `/` by a computed 0 inside a const recursion must TRAP fail-loud (CDZ0304
           'division by zero'), matching `core_of` + the runtime — NOT return None (which would DECLINE the fold and
           surface a generic 'cannot reduce' reject that masks the fault). `grow` computes `(/ 100 (- n n))` — the
           divisor `(- n n)` is 0 — so `apply_const_prim`'s Div folds to a `CVal::Trap` that propagates out of the
           recursion + Set.insert + Set.len to the const demand.")
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: n Int64)))
        (if (< n 1) s (grow (Set.insert s (/ 100 (- n n))) (- n 1))))
      (def (main) (const (Set.len (grow #set() 3))))
      (export main)))
  (error CDZ0304 (message "division by zero")))

; -- closed pure handles fold under (const ...) across state kinds: scalar, String, tuple, record (breaker batch 386; List/Map/Set states = the open collection-state seam) --
(case
  "chk1 a closed pure handle with SCALAR state folds under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def (main) (const (handle E 40 ((tick () s (resume s (+ s 1)))) (+ (E.tick) 2))))
      (export main)))
  (output (: 42 Int64)))

(case
  "chk2 a closed handle with STRING state folds under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main)
        (const
          (handle
            E
            "x"
            ((tick () s (resume (String.byte-len s) (String.concat s "y"))))
            (+ (* 10 (E.tick)) (E.tick)))))
      (export main)))
  (output (: 12 Int64)))

(case
  "cms1 closed handle with TUPLE state under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main)
        (const
          (handle
            E
            #tuple(1 10)
            ((tick () s (match s (#tuple(a b) (resume (+ a b) #tuple((+ a 1) (* b 2)))))))
            (+ (E.tick) (E.tick)))))
      (export main)))
  (output (: 33 Int64)))

(case
  "cms2 closed handle with RECORD state under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main)
        (const
          (handle
            E
            #record((= a 1) (= b 10))
            ((tick () s (resume (+ s.a s.b) #record((= a (+ s.a 1)) (= b (* s.b 2))))))
            (+ (E.tick) (E.tick)))))
      (export main)))
  (output (: 33 Int64)))

; -- fail-loud TIMING is demand-scoped, by design (v-cp ruling): a NON-recursive const fn taken trap stays a RUNTIME trap on the bare call (it may sit under a runtime branch; unconditional CDZ0304 would over-reject) and surfaces CDZ0304 only under an explicit (const ...) demand; recursive const fns surface on both paths (breaker batch 391) --
(case
  "cn02a a NON-recursive const fn taken trap is a RUNTIME trap on the bare call (by-design: it may sit under a runtime branch)"
  (input
    (do
      (def (f (const (: n Int64))) (if (= n 0) (trap "cn02a int zero") n))
      (def (main) (f 0))
      (export main)))
  (trap "unreachable"))

(case
  "cn02b the Option twin — bare-call taken trap stays a runtime trap (by-design)"
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match o ((Option.Some k) (if (= k 0) (trap "cn02b zero payload") k)) ((Option.None) 0)))
      (def (main) (f (Option.Some 0)))
      (export main)))
  (trap "unreachable"))

(case
  "cn02c the tuple twin — bare-call taken trap stays a runtime trap (by-design)"
  (input
    (do
      (def
        (f (const (: t (Tuple Int64 Int64))))
        (match t (#tuple(a b) (if (= a b) (trap "cn02c fields met") a))))
      (def (main) (f #tuple(3 3)))
      (export main)))
  (trap "unreachable"))

(case
  "cn02d the SAME shape under a (const ...) demand DOES surface CDZ0304 (the explicit fail-loud opt-in)"
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match o ((Option.Some k) (if (= k 0) (trap "cn02d zero payload") k)) ((Option.None) 0)))
      (def (main) (const (f (Option.Some 0))))
      (export main)))
  (error CDZ0304 (message "cn02d zero payload")))

; --- Primitive: RUNTIME Ast.print (heap op 92) — render a runtime Ast to canonical s-expr text ------------
; A COMPILE-TIME-visible Ast folds to a `Core::ConstStr` (`lower_print`); a RUNTIME Ast (built from a runtime
; input) lowers to `Core::AstPrint {operand, discs}` → the value-heap `ast-print` op (heap index 92), which
; walks the heap Ast and renders it BYTE-IDENTICAL to the compile-time `print_ast_value` fold. `discs` is a
; baked descriptor of the 7 Ast variant discs (LEB [int,float,bool,str,name,bytes,list]) the op reads to
; classify variants by name. Runtime print == compile-time print — pinned end-to-end (the op runs).
(case
  "runtime Ast.print renders a top-level Ast.Int to its decimal text"
  (doc
    "`n` is a runtime entry param, so `(Ast.Int (BigInt.of n))` is a RUNTIME Ast → `Ast.print` lowers to
           the `ast-print` heap op (op 92). `(run 42)` renders \"42\" — identical to the compile-time fold.")
  (input (do (def (run (: n Int64)) (Ast.print (Ast.Int (BigInt.of n)))) (export run)))
  (call run 42)
  (output (: "42" String))
  (live-objects known-leak))

(case
  "runtime Ast.print renders a nested Ast.List byte-identical to the compile-time fold"
  (doc
    "The nested case (#3621: the op reads list elements via vec-get): `(Ast.List (list (Ast.Name \"f\")
           (Ast.Int (BigInt.of n))))` at runtime renders `(f 2)` — parens, space-separated, each element
           recursively, identical to `print_ast_value`.")
  (input
    (do
      (def (run (: n Int64)) (Ast.print (Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of n))))))
      (export run)))
  (call run 2)
  (output (: "(f 2)" String))
  (live-objects known-leak))

(case
  "runtime Ast.print renders a doubly-nested Ast.List"
  (doc "A list-of-list — `((f 2))` — pins the recursive vec-get walk to depth 2.")
  (input
    (do
      (def
        (run (: n Int64))
        (Ast.print (Ast.List #list((Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of n))))))))
      (export run)))
  (call run 2)
  (output (: "((f 2))" String))
  (live-objects known-leak))

(case
  "compile-time Ast.print of the same Ast folds to the identical text"
  (doc
    "The compile-time control: a CONSTANT `Ast.Int` folds to the `Core::ConstStr` \"42\" via
           `print_ast_value` — the same text the runtime op produces, witnessing runtime==compile-time.")
  (input (do (def (main) (Ast.print (Ast.Int (BigInt.of 42)))) (export main)))
  (output (: "42" String)))

; --- Primitive: RUNTIME Ast.encode (heap op 93) — serialize a runtime Ast to canonical cdzast bytes --------
; A COMPILE-TIME-visible Ast folds to a `Core::ConstBytes` (`lower_ast_encode`); a RUNTIME Ast (built from a
; runtime input) lowers to `Core::AstEncode {operand, discs}` -> the value-heap `ast-encode` op (heap index
; 93), which walks the heap Ast into `ast::Arenas` and serializes it via the SAME shared `cadenza-ast` codec
; the compile-time fold uses -> BYTE-IDENTICAL output. `discs` is a baked 9-disc descriptor (LEB
; [int,float,bool,str,name,list,bytes,char,symbol]) the op reads to classify variants by name. The runtime op
; produces the same canonical document as the compile-time fold; witnessed here by `Bytes.len` (the file's
; established Ast.encode idiom, e.g. the `Blake3.of (Ast.encode …)` cases above), value-sensitive to `n`.
; (Was a decline "Ast.encode of a runtime AST value is not yet computed"; v-runtime landed op 93 in #3634.)
(case
  "runtime Ast.encode of an Ast.Int serializes via the ast-encode heap op, canonical length"
  (doc
    "`n` is a runtime entry param, so `(Ast.Int (BigInt.of n))` is a RUNTIME Ast -> `Ast.encode` lowers
           to the `ast-encode` heap op (op 93), returning a fresh Bytes leaf serialized by the shared codec.
           `(run 7)` -> a 16-byte canonical document (identical LENGTH to the compile-time fold, pinned by the
           control case below); `(run 100000000000)` -> 20 bytes (the larger BigInt widens the varint payload,
           proving the runtime value actually flows through the op rather than a baked constant).")
  (input (do (def (run (: n Int64)) (Bytes.len (Ast.encode (Ast.Int (BigInt.of n))))) (export run)))
  (call run 7)
  (output (: 16 Int64))
  (call run (: 100000000000 Int64))
  (output (: 20 Int64)))

(case
  "compile-time Ast.encode of the same Ast.Int folds to the identical length"
  (doc
    "The compile-time control: a CONSTANT `Ast.Int` folds to a `Core::ConstBytes` whose length is 16 —
           the SAME length the runtime op produces above, witnessing runtime==compile-time for the Int leaf.")
  (input (do (def (main) (Bytes.len (Ast.encode (Ast.Int (BigInt.of 7))))) (export main)))
  (output (: 16 Int64)))

(case
  "runtime Ast.encode of a nested Ast.List serializes via the recursive walk"
  (doc
    "The nested case: `(Ast.List (list (Ast.Name \"f\") (Ast.Int (BigInt.of n))))` at runtime serializes
           via the op's recursive walk (vec-get over the list, each element encoded by variant) into a 25-byte
           canonical document — pinning the recursive encode path end-to-end through the heap op.")
  (input
    (do
      (def
        (run (: n Int64))
        (Bytes.len (Ast.encode (Ast.List #list((Ast.Name "f") (Ast.Int (BigInt.of n)))))))
      (export run)))
  (call run 2)
  (output (: 25 Int64)))

; --- Primitive 2: const execution — a (const …) HANDLE with growing collection state folds (cm02) --------
; A closed finite `handle` with a const init, under `(const …)`, folds its answer: const_eval's Handle arm
; DELEGATES to `reduce_handle` (the effect reducer — threads continuations/resumes/state) and const-evaluates
; the resulting PURE AST. `reduce_handle` keeps a GROWING collection state as a re-read `let` binding, which
; `core_of` alone leaves as a `Core::Let` this stage cannot fold; const-evaluating the reduced AST folds the
; query answers via the List/Set/Map arms. Reuses the one reducer (no duplication). (Diagnosis: v-effects.)
(case
  "a const handle threading a growing List state folds its query answers"
  (doc
    "`E.tick` resumes `List.len s` and threads `(List.prepend s 0)` (state GROWS by one each perform).
           `(const (handle E (list 7) … (+ (* 10 (E.tick)) (E.tick))))`: first tick reads len 1 → resumes 1,
           state → `(0 7)`; second reads len 2 → resumes 2; `(+ (* 10 1) 2)` = 12. Before, the growing-list
           state kept a `Core::Let` the const block could not fold (CDZ0201); now the Handle arm reduces +
           const-evaluates it.")
  (input
    (do
      (effect E (op tick (-> Unit Int64)))
      (def
        (main)
        (const
          (handle
            E
            #list(7)
            ((tick (u) s (resume (List.len s) (List.prepend s 0))))
            (+ (* 10 (E.tick)) (E.tick)))))
      (export main)))
  (output (: 12 Int64)))

(case
  "a const handle threading a growing Set state folds its query answers"
  (doc
    "The Set-state companion: `E.tick` resumes `Set.len s` and threads `(Set.insert s 0)`. Seeded
           `{7}`: len 1 → 1, then `{7,0}` len 2 → 2; `(+ (* 10 1) 2)` = 12. Same Handle-arm reduce+fold path;
           the query answer (a size) is order-independent so it folds soundly.")
  (input
    (do
      (effect E (op tick (-> Unit Int64)))
      (def
        (main)
        (const
          (handle
            E
            #set(7)
            ((tick (u) s (resume (Set.len s) (Set.insert s 0))))
            (+ (* 10 (E.tick)) (E.tick)))))
      (export main)))
  (output (: 12 Int64)))

; -- (const ...) handles with COLLECTION states fold: growing List, Map insert+lookup, growing Set (breaker batch 395; the #3636 flip — completes the state-kind matrix begun in batch 386) --
(case
  "cm02 closed MULTI-dispatch handle with LIST state folds under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main)
        (const
          (handle
            E
            #list(7)
            ((tick () s (resume (List.len s) (List.prepend s 0))))
            (+ (* 10 (E.tick)) (E.tick)))))
      (export main)))
  (output (: 12 Int64)))

(case
  "cms3 closed handle with MAP state under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (rd (: m (Map Int64 Int64)))
        (match (Map.lookup m 0) ((Option.Some v) v) ((Option.None) -1)))
      (def
        (main)
        (const
          (handle
            E
            (Map.insert #map() 0 5)
            ((tick () s (resume (rd s) (Map.insert s 0 (+ (rd s) 1)))))
            (+ (E.tick) (E.tick)))))
      (export main)))
  (output (: 11 Int64)))

(case
  "cms4 closed handle with SET state under (const ...)"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main)
        (const
          (handle
            E
            #set(1)
            ((tick () s (resume (Set.len s) (Set.insert s (+ (Set.len s) 1)))))
            (+ (E.tick) (E.tick)))))
      (export main)))
  (output (: 3 Int64)))

; -- breaker batch 401 (2026-08-26): const-eval decline-class FLIP pins. These 12 probes were filed
; as declines (sweeps 1-2 + the recursive-const-Option wasm HANG, a non-terminating artifact from
; terminating source); all flipped to pass with the P2 const-execution work (#3670 Map.empty fold,
; #3695/#3697/#3698 collection-state + structural cval_eq). rop1 pins the miscompile fix; dc/s2
; pin the closed decline classes: Float/Char/tuple/record/Option const params in recursion,
; inline compound construct+eliminate in recursive position, and Map ops folding under Ast.encode.
(case
  "rop1 recursive const (Option Int64) param countdown reaching a trap folds to CDZ0304 (was a wasm hang)"
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match
          o
          ((Option.Some k) (if (= k 0) (trap "adv option reached zero") (f (Option.Some (- k 1)))))
          ((Option.None) 0)))
      (def (main) (f (Option.Some 2)))
      (export main)))
  (error CDZ0304 (message "adv option reached zero")))

(case
  "rop2 recursive const (Option Int64) param recursion on the value path folds to 99"
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match o ((Option.Some k) (if (= k 0) 99 (f (Option.Some (- k 1))))) ((Option.None) 0)))
      (def
        (run)
        (=
          (Ast.encode (Ast.Int (BigInt.of (f (Option.Some 2)))))
          (Ast.encode (Ast.Int (BigInt.of 99)))))
      (export run)))
  (output (: true Bool)))

(case
  "rop3 payload extracted to a bare Int64 const param before recursing folds (CDZ0304)"
  (input
    (do
      (def (g (const (: k Int64))) (if (= k 0) (trap "adv extracted reached zero") (g (- k 1))))
      (def (f (const (: o (Option Int64)))) (match o ((Option.Some k) (g k)) ((Option.None) 0)))
      (def (main) (f (Option.Some 2)))
      (export main)))
  (error CDZ0304 (message "adv extracted reached zero")))

(case
  "dc01 Float64 const-param countdown trap surfaces CDZ0304"
  (input
    (do
      (def (f (const (: x Float64))) (if (= x 0.0) (trap "dc01 float reached zero") (f (- x 1.0))))
      (def (main) (f 3.0))
      (export main)))
  (error CDZ0304 (message "dc01 float reached zero")))

(case
  "dc02 tuple const-param countdown trap surfaces CDZ0304"
  (input
    (do
      (def
        (f (const (: t (Tuple Int64 Int64))))
        (match t (#tuple(a b) (if (= a b) (trap "dc02 tuple fields met") (f #tuple((- a 1) b))))))
      (def (main) (f #tuple(3 1)))
      (export main)))
  (error CDZ0304 (message "dc02 tuple fields met")))

(case
  "dc03 record const-param countdown trap surfaces CDZ0304"
  (input
    (do
      (def
        (f (const (: r (Record (: n Int64)))))
        (if (= r.n 0) (trap "dc03 record field reached zero") (f #record((= n (- r.n 1))))))
      (def (main) (f #record((= n 2))))
      (export main)))
  (error CDZ0304 (message "dc03 record field reached zero")))

(case
  "dc04 (Option Int64) const-param recursion on the VALUE path folds to 99 under Ast.encode"
  (input
    (do
      (def
        (f (const (: o (Option Int64))))
        (match o ((Option.Some k) (if (= k 0) 99 (f (Option.Some (- k 1))))) ((Option.None) 0)))
      (def
        (run)
        (=
          (Ast.encode (Ast.Int (BigInt.of (f (Option.Some 2)))))
          (Ast.encode (Ast.Int (BigInt.of 99)))))
      (export run)))
  (output (: true Bool)))

(case
  "dc06 Map insert+lookup inside a const fn folds under Ast.encode"
  (input
    (do
      (def
        (f (const (: n Int64)))
        (match
          (Map.lookup (Map.insert #map() n "found") n)
          ((Option.Some s) s)
          ((Option.None) "absent")))
      (def (run) (= (Ast.encode (Ast.Name (f 4))) (Ast.encode (Ast.Name "found"))))
      (export run)))
  (output (: true Bool)))

(case
  "s2a Char const-param equality in const recursion trap surfaces CDZ0304"
  (input
    (do
      (def
        (f (const (: c Char)) (const (: n Int64)))
        (if (= n 0) (if (= c #\a) (trap "s2a char was a") (trap "s2a char other")) (f c (- n 1))))
      (def (main) (f #\a 2))
      (export main)))
  (error CDZ0304 (message "s2a char was a")))

(case
  "s2b INLINE record projection in the recursive argument trap surfaces CDZ0304"
  (input
    (do
      (def
        (f (const (: n Int64)))
        (if (= n 0) (trap "s2b inline record zero") (f (. #record((= lo (- n 1)) (= hi 9)) lo))))
      (def (main) (f 3))
      (export main)))
  (error CDZ0304 (message "s2b inline record zero")))

(case
  "s2c INLINE tuple destructure in the recursive body trap surfaces CDZ0304"
  (input
    (do
      (def
        (f (const (: n Int64)))
        (if (= n 0) (trap "s2c inline tuple zero") (match #tuple((- n 1) 9) (#tuple(a b) (f a)))))
      (def (main) (f 3))
      (export main)))
  (error CDZ0304 (message "s2c inline tuple zero")))

(case
  "s2d record-returning const helper projected in the recursive argument trap surfaces CDZ0304"
  (input
    (do
      (def (mk (const (: n Int64))) #record((= lo n) (= hi (* n 2))))
      (def
        (f (const (: n Int64)))
        (if (= n 0) (trap "s2d projected to zero") (f (. (mk (- n 1)) lo))))
      (def (main) (f 3))
      (export main)))
  (error CDZ0304 (message "s2d projected to zero")))

; -- breaker batch 402 (2026-08-26): const-eval INNER-POSITION and VALUE-CLASS fold pins — the
; non-redundant residue of the P2 flip sweep after overlap-check vs the existing const pins.
; cm01/cm03 pin (const ...) demanded INSIDE a handler (resume answer position, seed position);
; cm02c the multi-dispatch STRING-state closed handle; cn-c1/c3/c4/c5 pin the value classes
; Symbol / beyond-i64 BigInt / exact Rational / Qty unit-algebra folding under (const ...);
; cn-c2c the Set query under Ast.encode's const-param demand path (Map twin pinned above).
(case
  "cm01 (const ...) in a resume ANSWER position folds"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (handle E (% n 3) ((tick () s (resume (const (* 6 7)) (+ s 1)))) (+ (E.tick) (E.tick))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 84 Int64)))

(case
  "cm03 (const ...) as a handler SEED folds"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (handle E (const (+ 20 20)) ((tick () s (resume s (+ s 1)))) (+ (E.tick) (E.tick))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 81 Int64)))

(case
  "cm02c (const ...) closed handle with STRING state"
  (input
    (do
      (effect E (op tick (-> Int64)))
      (def
        (main)
        (const
          (handle
            E
            "x"
            ((tick () s (resume (String.byte-len s) (String.concat s "y"))))
            (+ (* 10 (E.tick)) (E.tick)))))
      (export main)))
  (output (: 12 Int64)))

(case
  "cn-c1 Symbol equality under (const ...)"
  (input (do (def (main) (const (if (= (Symbol.of "hot") (Symbol.of "hot")) 1 0))) (export main)))
  (output (: 1 Int64)))

(case
  "cn-c3 BigInt beyond-i64 multiply under (const ...)"
  (input
    (do
      (def (main) (const (= (* 111111111111N 111111111111N) 12345679012320987654321N)))
      (export main)))
  (output (: true Bool)))

(case
  "cn-c4 Rational exact division under (const ...)"
  (input
    (do
      (def (main) (const (= (+ (Rational.of 1 3) (Rational.of 1 6)) (Rational.of 1 2))))
      (export main)))
  (output (: true Bool)))

(case
  "cn-c5 Qty unit algebra under (const ...)"
  (input
    (do
      (def
        (main)
        (const (Qty.value (+ (Qty.of 3 (Unit.of #"meter")) (Qty.of 4 (Unit.of #"meter"))))))
      (export main)))
  (output (: 7 Int64)))

(case
  "cn-c2c a Set query composed inside a const-param fn folds under Ast.encode demand"
  (input
    (do
      (def (f (const (: n Int64))) (if (Set.contains #set(1 2 3) n) "in" "out"))
      (def (run) (= (Ast.encode (Ast.Name (f 2))) (Ast.encode (Ast.Name "in"))))
      (export run)))
  (output (: true Bool)))

; -- breaker batch 412 (2026-08-26): composition-depth folds under EXPLICIT (const ...) demand —
; the sweep-4 residue dissolved. The old trap-in-MAIN detectors graded todo by INTENDED demand
; semantics (dc05/cn02 family: no demand -> no fold -> runtime trap correct); under a (const ...)
; demand every composed shape folds and surfaces the taken trap: recursive AST leaf-count via an
; Option-threaded indexed walk (cdf1), decode-of-encode navigated through Result+Option+Ast matches
; (cdf2b trap + cdf2v value twin — the COMPILE-TIME Ast.decode folds; only the runtime op94 emit
; remains open), and a function-typed const param applied inside const recursion (cdf3). The cd06
; import face was a module-body grammar artifact (cd06b, with (do ...), already passes).
(case
  "cdf1 recursive AST leaf-count via Option-threaded walk under an explicit (const ...) demand surfaces the taken trap"
  (input
    (do
      (def (leaves (const (: a Ast))) (match a ((Ast.List xs) (leaves-of xs 0)) (_ 1)))
      (def
        (leaves-of (const (: xs (List Ast))) (const (: i Int64)))
        (match
          (List.at xs i)
          ((Option.Some c) (+ (leaves c) (leaves-of xs (+ i 1))))
          ((Option.None) 0)))
      (def
        (main)
        (const
          (if
            (= (leaves (quote (f 1 2))) 3)
            (trap "cdf1 three leaves")
            (trap "cdf1 WRONG leaf count"))))
      (export main)))
  (error CDZ0304 (message "cdf1 three leaves")))

(case
  "cdf2b decode-of-encode navigation (Result-correct) under (const ...) surfaces the taken trap"
  (input
    (do
      (def
        (second-int (const (: a Ast)))
        (match
          (Ast.decode (Ast.encode a))
          ((Ok d)
            (match
              d
              ((Ast.List xs)
                (match
                  (List.at xs 1)
                  ((Option.Some c) (match c ((Ast.Int b) b) (_ (BigInt.of -1))))
                  ((Option.None) (BigInt.of -1))))
              (_ (BigInt.of -2))))
          ((Err _) (BigInt.of -3))))
      (def
        (main)
        (const
          (if
            (= (second-int (quote (g 7))) 7N)
            (trap "cdf2b roundtrip navigated")
            (trap "cdf2b WRONG"))))
      (export main)))
  (error CDZ0304 (message "cdf2b roundtrip navigated")))

(case
  "cdf2v decode-of-encode navigation VALUE twin under Ast.encode demand"
  (input
    (do
      (def
        (second-int (const (: a Ast)))
        (match
          (Ast.decode (Ast.encode a))
          ((Ok d)
            (match
              d
              ((Ast.List xs)
                (match
                  (List.at xs 1)
                  ((Option.Some c) (match c ((Ast.Int b) b) (_ (BigInt.of -1))))
                  ((Option.None) (BigInt.of -1))))
              (_ (BigInt.of -2))))
          ((Err _) (BigInt.of -3))))
      (def (run) (= (Ast.encode (Ast.Int (second-int (quote (g 7))))) (Ast.encode (Ast.Int 7N))))
      (export run)))
  (output (: true Bool)))

(case
  "cdf3 function-typed const param applied in const recursion under (const ...) surfaces the taken trap"
  (input
    (do
      (def
        (ap (const (: g (-> Int64 Int64))) (const (: n Int64)))
        (if (= n 0) (g 5) (ap g (- n 1))))
      (def
        (main)
        (const
          (if
            (= (ap (fn (x) (* x 2)) 2) 10)
            (trap "cdf3 lambda applied in fold")
            (trap "cdf3 WRONG"))))
      (export main)))
  (error CDZ0304 (message "cdf3 lambda applied in fold")))

; -- breaker batch 413 (2026-08-26): the demand-asymmetry bank dissolved — every composed const
; shape folds under a FORCED (const ...) demand and surfaces the taken trap: list-REST-pattern
; recursion, indexed walk with a NON-const index param, self-recursive explicit-stack walk,
; Ast.module reflection count, and a corpus-style linear collect with List.prepend. The unforced
; twins grade todo by INTENDED demand semantics (dc05/cn02: trap-in-main without a demand = runtime
; trap), not by any fold gap. Companion to batch 412's sweep-4 dissolution.
(case
  "cgf01 leaf count via list-REST-pattern recursion folds (CDZ0304 detector) under FORCED (const ...) demand"
  (input
    (do
      (def (leaves (const (: a Ast))) (match a ((Ast.List xs) (leaves-list xs)) (_ 1)))
      (def
        (leaves-list (const (: xs (List Ast))))
        (match xs (#list() 0) (#list(h (.. t)) (+ (leaves h) (leaves-list t)))))
      (def
        (main)
        (const (if (= (leaves (quote (f 1 2))) 3) (trap "cgf01 folded three") (trap "cgf01 WRONG"))))
      (export main)))
  (error CDZ0304 (message "cgf01 folded three")))

(case
  "cgf02 leaf count via indexed walk with NON-const index folds (CDZ0304 detector) under FORCED (const ...) demand"
  (input
    (do
      (def (leaves (const (: a Ast))) (match a ((Ast.List xs) (leaves-of xs 0)) (_ 1)))
      (def
        (leaves-of (const (: xs (List Ast))) (: i Int64))
        (match
          (List.at xs i)
          ((Option.Some c) (+ (leaves c) (leaves-of xs (+ i 1))))
          ((Option.None) 0)))
      (def
        (main)
        (const (if (= (leaves (quote (f 1 2))) 3) (trap "cgf02 folded three") (trap "cgf02 WRONG"))))
      (export main)))
  (error CDZ0304 (message "cgf02 folded three")))

(case
  "cgf05 leaf count via SELF-recursive explicit-stack walk folds (CDZ0304 detector) under FORCED (const ...) demand"
  (input
    (do
      (def
        (count (const (: stack (List Ast))))
        (match
          stack
          (#list() 0)
          (#list(h (.. t)) (match h ((Ast.List es) (count (List.concat es t))) (_ (+ 1 (count t)))))))
      (def
        (main)
        (const
          (if (= (count #list((quote (f 1 2)))) 3) (trap "cgf05 folded three") (trap "cgf05 WRONG"))))
      (export main)))
  (error CDZ0304 (message "cgf05 folded three")))

(case
  "cgf04t leaf count over Ast.module — trap detector distinguishes fold from runtime walk under FORCED (const ...) demand"
  (input
    (do
      (def
        (leaves-list (const (: xs (List Ast))))
        (match
          xs
          (#list() 0)
          (#list(h (.. t))
            (match h ((Ast.List es) (+ (leaves-list es) (leaves-list t))) (_ (+ 1 (leaves-list t)))))))
      (def (forms-of (const (: mm Ast))) (match mm ((Ast.List fs) fs) (_ (: #list() (List Ast)))))
      (def
        (main)
        (const
          (if
            (> (leaves-list (forms-of Ast.module)) 0)
            (trap "cgf04t folded positive")
            (trap "cgf04t WRONG"))))
      (export main)))
  (error CDZ0304 (message "cgf04t folded positive")))

(case
  "cgf06 corpus-style linear collect under TRAP demand (H2 test) under FORCED (const ...) demand"
  (input
    (do
      (def
        (child (const (: form Ast)) (: i Int64))
        (match
          form
          ((Ast.List es) (match (List.at es i) ((Option.Some v) v) ((Option.None) (Ast.Name "?"))))
          (_ (Ast.Name "?"))))
      (def (name-of (const (: form Ast))) (match form ((Ast.Name n) n) (_ "")))
      (def (head-name (const (: form Ast))) (name-of (child form 0)))
      (def
        (keep-f (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t)) (if (= (head-name h) "f") (List.prepend (keep-f t) h) (keep-f t)))))
      (def
        (main)
        (const
          (if
            (= (List.len (keep-f #list((quote (f 1)) (quote (g 2))))) 1)
            (trap "cgf06 folded one")
            (trap "cgf06 WRONG"))))
      (export main)))
  (error CDZ0304 (message "cgf06 folded one")))

; -- breaker batch 421 (2026-08-26): the #3774 <=1-element orderability edges — a single-element
; and an EMPTY (typed) const Set.to-list fold, and a single-entry const Map.to-list reads its lone
; (k v). Same-hour pins of the bonus fix that rode the Map.to-list fold.
(case
  "le1 a SINGLE-element const Set.to-list folds"
  (input (do (def (main) (const (List.len (Set.to-list #set(42))))) (export main)))
  (output (: 1 Int64)))

(case
  "le2 an EMPTY const Set.to-list folds to the empty list"
  (input (do (def (main) (const (List.len (Set.to-list (: #set() (Set Int64)))))) (export main)))
  (output (: 0 Int64)))

(case
  "le3 a single-entry const Map.to-list reads its lone (k v)"
  (input
    (do
      (def
        (main)
        (const
          (match
            (List.at (Map.to-list (Map.insert #map() 7 70)) 0)
            ((Option.Some #tuple(k v)) (+ k v))
            ((Option.None) -1))))
      (export main)))
  (output (: 77 Int64)))

; -- breaker batch 422 (2026-08-26): the #3774 sort-by-skip SOUNDNESS faces — a LONE non-orderable
; element (Set) / KEY (Map) still declines under the const to-list fold (pre-fix, sort_by over 0/1
; elements never called the comparator, so a lone compound would have MATERIALIZED while the runtime
; op declines — a compile/runtime divergence). The EMPTY set of a non-orderable element type is
; order-trivial and folds to the empty list (the correct boundary of the per-element pre-check).
; #7234 refined the DIAGNOSTIC: a non-orderable element/key type has NO TOTAL ORDER, so its to-list
; enumeration is undefined — the shared front-end now declines CDZ0203 (the no-total-order carve-out
; family: float leaf = IEEE partial order, set/map leaf = no blessed order; ordering #7143 + compare
; #7210 + to-list #7234), the DEEPER reason than the former generic CDZ0201 "not a compile-time
; constant". Still a decline (the const value cannot be materialized), just coded for the real cause.
(case
  "lnr1 a const Set.to-list of a LONE non-orderable element still declines (the sort-by-skip soundness face)"
  (doc
    "A LONE element (a `sort_by` over one element never calls the comparator) must still be probed for
        orderability. A tuple carrying a FLOAT is genuinely non-orderable (a float offers only the IEEE partial
        order), so its to-list has no total order → CDZ0203 at the shared front-end (the deeper reason than the
        former const-demand CDZ0201). A tuple of orderable scalars still folds — see the tuple-order cases above;
        this pins the LONE-non-orderable soundness face still declines, now with the no-total-order code.")
  (input (do (def (main) (const (List.len (Set.to-list #set(#tuple(1.5 2)))))) (export main)))
  (error CDZ0203 (message "no total order")))

(case
  "lnr2 a const Map.to-list with a LONE non-orderable KEY still declines"
  (doc
    "The Map-key twin: a lone tuple KEY carrying a FLOAT is non-orderable, so its to-list has no total order
        over the keys → CDZ0203 at the shared front-end (the deeper reason than the former const-demand CDZ0201).")
  (input
    (do
      (def (main) (const (List.len (Map.to-list (Map.insert #map() #tuple(1.5 2) 10)))))
      (export main)))
  (error CDZ0203 (message "no total order")))

(case
  "lnr3 an EMPTY set of a non-orderable element type IS order-trivial — const to-list folds to 0"
  (input
    (do
      (def (main) (const (List.len (Set.to-list (: #set() (Set (Tuple Int64 Int64)))))))
      (export main)))
  (output (: 0 Int64)))

; -- breaker batch 424 (2026-08-26): #3783 same-hour edge pins (the ConstBlock fall-through fix) —
; a SET-accumulator recursion's to-list reads its sorted head and surfaces the taken trap as CDZ0304
; (fail-loud through the whole fold), and a recursive const-param walk SUMS the folded list. Both
; rejected the CDZ0201 catch-all pre-#3783. Residue: Char (blocked on v-runtime) / Bytes elements.
(case
  "sar1 a recursion-built set accumulator to-list reads its sorted head under the trap detector (CDZ0304)"
  (input
    (do
      (def
        (grow (const (: s (Set Int64))) (const (: k Int64)))
        (if (= k 0) s (grow (Set.insert s (* k 7)) (- k 1))))
      (def
        (main)
        (const
          (match
            (List.at (Set.to-list (grow #set() 4)) 0)
            ((Option.Some v) (if (= v 7) (trap "cst1 sorted head seven") (trap "cst1 WRONG")))
            ((Option.None) (trap "cst1 EMPTY")))))
      (export main)))
  (error CDZ0304 (message "cst1 sorted head seven")))

(case
  "sar2 a recursive const walk SUMS a folded Set.to-list"
  (input
    (do
      (def
        (suml (const (: xs (List Int64))) (const (: i Int64)))
        (match (List.at xs i) ((Option.Some v) (+ v (suml xs (+ i 1)))) ((Option.None) 0)))
      (def (main) (const (suml (Set.to-list #set(5 1 3)) 0)))
      (export main)))
  (output (: 9 Int64)))

; -- breaker batch 426 (2026-08-26): RUNTIME Blake3 digests as compare operands and CHAMP keys —
; freshly admitted by the #3786 Owned classification: bare = twins/sensitivity, set-member dedup by
; content, Map key findable by a fresh recompute, and the Blake3-of-encode composition. OUTPUT-ONLY
; pins (the borrowing-op owned-operand reclaim is v-runtime's Blake3Of-class follow-up; live-objects
; clauses arrive with it). wasm pass / rust todo (the runtime Blake3/encode paths pend on rust).
(case
  "bk1 two runtime Blake3 digests of the same bytes bare-compare equal"
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (=
            (Blake3.of (Bytes.of #list((UInt8.wrap n) 2)))
            (Blake3.of (Bytes.of #list((UInt8.wrap n) 2))))
          1
          0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "bk2 one-byte-different inputs bare-compare unequal digests"
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (=
            (Blake3.of (Bytes.of #list((UInt8.wrap n))))
            (Blake3.of (Bytes.of #list((UInt8.wrap (+ n 1))))))
          1
          0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "bk3 runtime digests dedup as set members by content"
  (input
    (do
      (def
        (main (: n Int64))
        (Set.len
          #set((Blake3.of (Bytes.of #list((UInt8.wrap n))))
            (Blake3.of (Bytes.of #list((UInt8.wrap n))))
            (Blake3.of (Bytes.of #list(9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "bk4 a Map keyed by a runtime digest is findable by a fresh recompute"
  (input
    (do
      (def
        (main (: n Int64))
        (match
          (Map.lookup
            (Map.insert #map() (Blake3.of (Bytes.of #list((UInt8.wrap n)))) 42)
            (Blake3.of (Bytes.of #list((UInt8.wrap n)))))
          ((Option.Some v) v)
          ((Option.None) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "bk5 Blake3-of-encode runtime composition compares equal across recomputation"
  (input
    (do
      (def
        (main (: n Int64))
        (if
          (=
            (Blake3.of (Ast.encode (Ast.Int (BigInt.of n))))
            (Blake3.of (Ast.encode (Ast.Int (BigInt.of n)))))
          1
          0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64))
  (live-objects 0))

; --- Primitive 2: a compile-provable arith TRAP under a RUNTIME branch traps at RUNTIME, not at compile ------
; Operator ruling 2026-08-27 (per cn02): "if branch reachability depends on a runtime value we should absolutely
; not error out and should trap at runtime." A `(/ 1 0)` (or any provable overflow/OOB) in an `if` branch / `match`
; arm is CONDITIONALLY reached, so it DEMOTES to a runtime trap (fires only when taken) rather than hard-erroring at
; compile. CDZ0304 is kept ONLY under a `(const ...)` demand (cn02d / cdv2) or a STATICALLY-UNCONDITIONAL trap.
(case
  "dzb1 a const divide-by-zero in an UNTAKEN if-branch does NOT compile-error; the taken branch returns its value"
  (doc
    "`(if (> n 0) 7 (/ 1 0))` — the `(/ 1 0)` is a compile-provable trap, but it sits in the else-branch under a
           RUNTIME condition. At n>0 the else is never taken, so the program returns 7 (it must NOT hard-error at
           compile, per the operator ruling). Pins the demote: the ConstTrap branch became a runtime trap, so the `if`
           compiles and the taken branch computes. Also emits the CDZ0309 WARNING (operator follow-on): the
           fold-synthesized trap could fire along the reachable else-path — flagged, but not an error.")
  (input (do (def (main (: n Int64)) (if (> n 0) 7 (/ 1 0))) (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (warns CDZ0309 (message "potentially reachable trap")))

(case
  "dzb2 the SAME divide-by-zero branch traps at RUNTIME when the condition takes it"
  (doc
    "The runtime face of dzb1: at n≤0 the else-branch IS taken, so the demoted trap fires at RUNTIME with the
           PRESERVED divide-by-zero KIND (operator ruling 2026-08-27, Lean-oracle finding): the demote target is
           `Core::TrapDivZero`, so the const `(/ 1 0)` reads identically to a runtime `(/ n 0)` at the trap site
           (`divide by zero`, not the bare `unreachable` a plain `Core::Trap` would report). Pins that the demote
           PRESERVES both the trap AND its kind — deferred to runtime, not dropped, not kind-erased.")
  (input (do (def (main (: n Int64)) (if (> n 0) 7 (/ 1 0))) (export main)))
  (call main (: 0 Int64))
  (trap "CDZ0701")
  (warns CDZ0309 (message "potentially reachable trap")))

(case
  "dzb3 a const divide-by-zero in a MATCH arm traps at runtime, not at compile (the match twin)"
  (doc
    "`(match n (0 (/ 1 0)) (_ 7))` — the `(/ 1 0)` arm is conditionally reached (only at n=0), so it demotes to
           a runtime trap: the match COMPILES (no CDZ0304), and taking the trapping arm at n=0 traps at runtime with
           the PRESERVED `divide by zero` kind (`Core::TrapDivZero`, the match twin of dzb2's `if`-branch demote).")
  (input (do (def (main (: n Int64)) (match n (0 (/ 1 0)) (_ 7))) (export main)))
  (call main (: 0 Int64))
  (trap "CDZ0701")
  (warns CDZ0309 (message "potentially reachable trap")))

(case
  "dzw1 an EXPLICIT user (trap …) in a runtime branch does NOT warn CDZ0309 (only const-fold-origin traps warn)"
  (doc
    "The discrimination the operator asked for: CDZ0309 flags a fold-SYNTHESIZED reachable trap, NOT an
           intentional user trap. `(if (> n 0) 7 (trap \"chosen\"))` — the else is an explicit `(trap …)` (lowers to a
           plain `Core::Trap`, not a provable-trap poison), so `demote_conditional_trap` never touches it and NO
           CDZ0309 is emitted. At n>0 it returns 7; the author's trap fires only if the else is taken (a bare
           `unreachable`). No `(warns …)` clause — the case pins that this path builds clean (a spurious CDZ0309 here
           would be the discrimination breaking).")
  (input (do (def (main (: n Int64)) (if (> n 0) 7 (trap "chosen"))) (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))

(case
  "dzb4 a STATICALLY-UNCONDITIONAL const divide-by-zero is STILL fail-loud CDZ0304 (the carve-out holds)"
  (doc
    "The other half of the ruling: a provable trap NOT guarded by a runtime branch is statically-unconditional,
           so it stays a compile error. `(def (main) (/ 1 0))` always traps on every run → CDZ0304, unchanged. Pins
           that the demote is scoped to conditionally-reached (guarded) positions, not the unconditional spine.")
  (input (do (def (main) (/ 1 0)) (export main)))
  (error CDZ0304 (message "divide by zero")))

(case
  "dzb5 a const REMAINDER-by-zero in a conditional branch demotes to the SAME div-by-zero kind (the % twin)"
  (doc
    "`%`-by-zero shares the div-by-zero cause with `/`-by-zero (`const_trap_cause`'s `Div | Rem if y==0`), so a
           conditionally-reached `(% 1 0)` demotes to `Core::TrapDivZero` exactly like dzb2's `(/ 1 0)` — pins the Rem
           arm of the kind-preserving demote, not just Div. At n=0 the else is taken → traps 'divide by zero'.")
  (input (do (def (main (: n Int64)) (if (> n 0) 7 (% 1 0))) (export main)))
  (call main (: 0 Int64))
  (trap "CDZ0701")
  (warns CDZ0309 (message "potentially reachable trap")))

; -- overflow-demote family (operator 2026-08-27: "add a dedicated overflow core op as well … we should
; really be better about tagging traps that we've inserted so it's clear what happened"). The overflow
; twin of dzb: a const arithmetic OVERFLOW in a conditionally-reached branch demotes to a KIND-PRESERVING
; runtime trap (`Core::TrapOverflow`) that surfaces the "integer overflow" kind — NOT the bare
; "unreachable" a plain `Core::Trap` reports — so a fold-provable const overflow reads identically to its
; runtime counterpart at the trap site. ovb3 pins the discrimination: a shift-COUNT-out-of-range (which
; wasm masks — no native overflow trap) still demotes to the kind-less `unreachable`.
(case
  "ovb1 a const OVERFLOW branch not taken computes the taken branch (the value face, like dzb1)"
  (doc
    "`(if (> n 0) 7 (* MAX MAX))` — the else multiplies Int64.max by itself (a compile-provable OVERFLOW),
           but it is CONDITIONALLY reached, so it demotes to a runtime trap rather than failing the build (CDZ0304).
           At n>0 the else is untaken → returns 7. Warns CDZ0309 (the fold-synthesized trap could fire on the else path).")
  (input
    (do
      (def (main (: n Int64)) (if (> n 0) 7 (* 9223372036854775807 9223372036854775807)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (warns CDZ0309 (message "potentially reachable trap")))

(case
  "ovb2 the SAME overflow branch traps 'integer overflow' at RUNTIME when the condition takes it"
  (doc
    "The runtime face of ovb1: at n≤0 the else IS taken, so the demoted trap fires at RUNTIME with the
           PRESERVED overflow KIND (operator ruling 2026-08-27) — `Core::TrapOverflow`, so the const `(* MAX MAX)`
           reads identically to a runtime checked-multiply overflow at the trap site ('integer overflow', not the
           bare 'unreachable' a plain `Core::Trap` would report). Pins the overflow twin of dzb2's div-by-zero demote.")
  (input
    (do
      (def (main (: n Int64)) (if (> n 0) 7 (* 9223372036854775807 9223372036854775807)))
      (export main)))
  (call main (: 0 Int64))
  (trap "CDZ0703")
  (warns CDZ0309 (message "potentially reachable trap")))

(case
  "ovb3 a shift-COUNT-out-of-range branch stays the kind-less 'unreachable' (the discrimination)"
  (doc
    "The kind IS a deterministic function of the operation: a shift whose COUNT is out of range 0..64 is NOT
           surfaced as an overflow — wasm MASKS the shift count (no native trap), so the compiler's guard is a bare
           `unreachable`, and the cause names an out-of-range count, not 'overflow'. So `(<< 1 100)` demotes to the
           kind-less `Core::Trap`, unlike ovb2's true arithmetic overflow. Pins that only genuine overflow gets the
           overflow kind. (Still a fold-synthesized reachable trap → CDZ0309.)")
  (input (do (def (main (: n Int64)) (if (> n 0) 7 (<< 1 100))) (export main)))
  (call main (: 0 Int64))
  (trap "CDZ0704")
  (warns CDZ0309 (message "potentially reachable trap")))

(case
  "ovb4 the OTHER Div overflow — Int64.min / -1 — demotes to the overflow kind (not divide-by-zero)"
  (doc
    "`(/ Int64.min -1)` has no Int64 quotient (2^63 overflows) — the sole non-zero-divisor Div trap, cause 'the
           quotient overflows Int64'. In a conditional branch it demotes to `Core::TrapOverflow`, NOT `TrapDivZero`
           (the divisor is -1, not 0) — pins that `is_overflow_trap` catches the division-overflow message AND that the
           two Div traps are discriminated by kind at the demote. At n=0 the else is taken → traps 'integer overflow'.")
  (input (do (def (main (: n Int64)) (if (> n 0) 7 (/ -9223372036854775808 -1))) (export main)))
  (call main (: 0 Int64))
  (trap "CDZ0703")
  (warns CDZ0309 (message "potentially reachable trap")))

; -- breaker batch 431 (2026-08-26): #3799 complement pins — a const Map with CHAR keys to-lists
; key-sorted (the Set face is owner-pinned), and the sorted Char head surfaces a taken trap as
; CDZ0304 under the const demand (the fail-loud discipline through the Char fold). Both targets
; (the fold bakes at compile time — the runtime wasm Char op gap, ckr1, is untouched).
(case
  "chm1 a const Map with CHAR keys to-lists key-sorted"
  (input
    (do
      (def
        (main)
        (const
          (match
            (List.at (Map.to-list (Map.insert (Map.insert #map() #\c 3) #\a 1)) 0)
            ((Option.Some #tuple(k v)) (if (= k #\a) v -2))
            ((Option.None) -1))))
      (export main)))
  (output (: 1 Int64)))

(case
  "chm2 the sorted Char head surfaces a taken trap under the const demand (CDZ0304)"
  (input
    (do
      (def
        (main)
        (const
          (match
            (List.at (Set.to-list #set(#\q #\b #\z)) 0)
            ((Option.Some ch) (if (= ch #\b) (trap "chm2 head is b") (trap "chm2 WRONG")))
            ((Option.None) (trap "chm2 EMPTY")))))
      (export main)))
  (error CDZ0304 (message "chm2 head is b")))

; -- List.len fold trap-preservation (migrated from rcdzc list_len_fold_preserves_a_trapping_element_construction,
; 2026-08-27): List.len folds to the constant arity ONLY when every element construction is provably
; trap-free. An element with a RUNTIME-computed value that can trap must NOT be dropped by the fold —
; the runtime length must evaluate the construction so the trap still surfaces. The trap-free constant
; twin still folds. Before the guard this ran to the arity on all backends and swallowed the trap.
(case
  "List.len does NOT fold away a trapping element construction (runtime denominator)"
  (doc
    "List.len folds to the constant arity only when every element construction is provably
           trap-free. A `(Rational.of 3 d)` element with a RUNTIME denominator is not trap-free — at
           d=0 it is a zero-denominator trap — so List.len must emit the runtime length (evaluating the
           construction) rather than fold to the constant 2 and DROP the trap.")
  (input
    (do
      (def (main (: d Int64)) (List.len #list((Rational.of 1 2) (Rational.of 3 d))))
      (export main)))
  (call main (: 0 Int64))
  (trap "unreachable")
  (call main (: 2 Int64))
  (output (: 2 Int64)))

(case
  "List.len of a trap-free constant list still folds to its arity"
  (doc
    "The trap-free twin of the guard above: every element of `(list 1 2 3)` is a provably
           trap-free literal, so List.len folds to the constant arity 3 — the trap-preservation guard
           did not over-decline on constant lists.")
  (input (do (def (main) (List.len #list(1 2 3))) (export main)))
  (output (: 3 Int64)))

; -- select-ification trap parity (behavioral half migrated from rcdzc a_possibly_trapping_if_arm_is_not_select_ified,
; 2026-08-27; the white-box Lir If-vs-Select inspection stays a wasmtime-free rcdzc unit test): a wasm
; `select` evaluates BOTH arms, so an `if` with a possibly-trapping arm must NOT select-ify — it must
; evaluate only the taken arm. This is the observable half: the untaken trapping arm does not trap.
(case
  "a possibly-trapping if arm evaluates only the taken branch (no eager trap on the untaken arm)"
  (doc
    "A wasm `select` evaluates BOTH arms unconditionally, so an `if` whose arm contains a trapping
           op — a checked `(/ a b)` — must stay an `if` that evaluates only the taken arm. c=true,b=4 →
           20/4=5; c=false,b=0 → 20 (the untaken `(/ a 0)` is NOT evaluated, so no trap); c=true,b=0 →
           the TAKEN `(/ a 0)` divides by zero and traps.")
  (input (do (def (main (: c Bool) (: a Int64) (: b Int64)) (if c (/ a b) a)) (export main)))
  (call main (: true Bool) (: 20 Int64) (: 4 Int64))
  (output (: 5 Int64))
  (call main (: false Bool) (: 20 Int64) (: 0 Int64))
  (output (: 20 Int64))
  (call main (: true Bool) (: 20 Int64) (: 0 Int64))
  (trap "divide by zero"))

; -- nested strength-reduction overflow parity (behavioral half migrated from rcdzc
; a_nested_multiply_by_a_power_of_two_also_strength_reduces, 2026-08-27; the white-box Lir shl-count /
; no-mul-div_s inspection stays a wasmtime-free rcdzc unit test): a nested `(* (* x 2) 4)` reduces both
; multiplies to shifts, and the shift chain must still trap on the SAME overflow the checked multiply
; would — value parity (x*8) and overflow-trap parity, at Int64 and at a narrow Int8 (inner and outer).
(case
  "a nested (* (* x 2) 4) strength-reduces to shifts and preserves value + Int64 overflow-trap parity"
  (doc
    "The inner `(* x 2)` is an operand of the outer `* 4`; both reduce to shifts (`x << 1`,
           `<< 2`). Value parity: x*8. Overflow parity: 2^61 * 8 = 2^64 overflows Int64, so the shift
           chain must still trap exactly as the checked multiply would.")
  (input (do (def (main (: x Int64)) (* (: (* x 2) Int64) 4)) (export main)))
  (call main (: 5 Int64))
  (output (: 40 Int64))
  (call main (: -3 Int64))
  (output (: -24 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 2305843009213693952 Int64))
  (trap "overflow"))

(case
  "the narrow Int8 nested (* (* x 2) 4) traps on BOTH inner and outer overflow"
  (doc
    "The narrow-width twin: Int8 `(* (* x 2) 4)` = x*8. 15*8=120 fits; x=20 overflows the OUTER
           multiply (160 > 127); x=100 overflows the INNER `* 2` already (200 > 127). Both narrow
           overflows must trap — the strength-reduced shift chain range-checks the narrow bound.")
  (input (do (def (main (: x Int8)) (* (: (* x 2) Int8) 4)) (export main)))
  (call main (: 15 Int8))
  (output (: 120 Int8))
  (call main (: 20 Int8))
  (trap "overflow")
  (call main (: 100 Int8))
  (trap "overflow"))

; -- flow-sensitive equal-branch collapse parity (behavioral half migrated from rcdzc
; an_if_whose_branches_refine_to_the_same_constant_collapses, 2026-08-27; the white-box Lir collapse /
; no-compare inspection stays a wasmtime-free rcdzc unit test): when both branches reduce to the SAME
; constant under their branch refinements and the condition is trap-free, the whole `if` is that
; constant; a trapping condition blocks the collapse and still traps.
(case
  "an if whose branches refine to the same constant collapses to that constant (value parity)"
  (doc
    "Flow-sensitive collapse: under x>10 the inner `(> x 5)` is decided true so the then-branch is
           7, matching the else 7 — so `(if (> x 10) (if (> x 5) 7 8) 7)` is 7 for every x.")
  (input (do (def (main (: x Int64)) (if (> x 10) (if (> x 5) 7 8) 7)) (export main)))
  (call main (: -100 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: 6 Int64))
  (output (: 7 Int64))
  (call main (: 11 Int64))
  (output (: 7 Int64))
  (call main (: 50 Int64))
  (output (: 7 Int64)))

(case
  "an if whose branches do NOT refine to the same constant keeps both values"
  (doc
    "The negative twin: `(if (> x 10) (if (> x 5) 7 8) 9)` does not collapse — x=20 → 7 (x>10,
           inner x>5 true), x=0 → 9 (else).")
  (input (do (def (main (: x Int64)) (if (> x 10) (if (> x 5) 7 8) 9)) (export main)))
  (call main (: 20 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (output (: 9 Int64)))

(case
  "a trapping condition blocks the equal-branch collapse and still traps"
  (doc
    "The collapse requires a trap-free condition. `(> (/ 10 n) 0)` has a trapping `/`, so the `if`
           is kept and traps at n=0 (the trapping condition is not dropped); n=5 → 7.")
  (input (do (def (main (: n Int64)) (if (> (/ 10 n) 0) (if (> (/ 10 n) 0) 7 8) 7)) (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

; -- kept let-binding carries its initializer's range (behavioral half migrated from rcdzc
; a_kept_let_binding_carries_its_initializers_range, 2026-08-27; the white-box Lir guard-elision
; inspection — the `(+ y y)`/`(* y y)` over a masked binding sheds its overflow guard, a full-range
; binding keeps it — stays a wasmtime-free rcdzc unit test): a multi-use `let`-binding's `LocalRef`
; carries the range of its INITIALIZER, so `(& x 255)` bound to `y` propagates [0,255] through `y` and
; the guard-elided `(+ y y)`/`(* y y)` computes the same value the guarded inlined form would.
(case
  "a kept let-binding carries its masked initializer's range and the guard-elided doubling computes correctly (value parity)"
  (doc
    "`(let ((y (& x 255))) (+ y y))` — `y` is `x & 255` ∈ [0,255], so `y + y` ∈ [0,510] sheds its
           overflow guard yet is exactly 2·(x&255): x=200 → 200&255=200 → 400; x=-1 → 255 → 510; x=0 → 0.")
  (input (do (def (main (: x Int64)) (let ((y (& x 255))) (+ y y))) (export main)))
  (call main (: 200 Int64))
  (output (: 400 Int64))
  (call main (: -1 Int64))
  (output (: 510 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a kept let-binding's masked range lets a guard-elided multiply compute correctly (value parity)"
  (doc
    "`(let ((y (& x 255))) (* y y))` — the same masked binding, squared: x=20 → 20&255=20 → 400;
           x=0 → 0; x=16 → 256, in [0,255²] and guard-elided but exact.")
  (input (do (def (main (: x Int64)) (let ((y (& x 255))) (* y y))) (export main)))
  (call main (: 20 Int64))
  (output (: 400 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 16 Int64))
  (output (: 256 Int64)))

; --- let-binding reuse / CSE: a bound value is computed once and reused, value-transparently ---------
; A `let`-bound runtime value used more than once is value-numbered to one computation (the backend's
; dominator CSE); the OBSERVABLE invariant these pin is value-transparency — reusing a binding yields the
; same result as recomputing it. (The "computed once" is an internal optimization the run cannot witness;
; the run witnesses the correct value.)
(case
  "a multi-use runtime let binding is computed once and reused (value-transparent)"
  (doc
    "`(let ((s (+ a b))) (+ s s))` binds the sum once and adds it to itself: a=10,b=20 -> s=30 ->
           30+30 = 60. Reusing `s` is value-transparent (same result as recomputing `(+ a b)` twice).")
  (input (do (def (main (: a Int64) (: b Int64)) (let ((s (+ a b))) (+ s s))) (export main)))
  (call main (: 10 Int64) (: 20 Int64))
  (output (: 60 Int64)))

(case
  "a named runtime binding computes its value exactly once"
  (doc
    "The subtraction companion: `(let ((s (- a b))) (+ s s))` with a=7,b=4 -> s=3 -> 3+3 = 6. Pins
           that a named runtime binding used twice is value-transparent over a non-commutative operand.")
  (input (do (def (main (: a Int64) (: b Int64)) (let ((s (- a b))) (+ s s))) (export main)))
  (call main (: 7 Int64) (: 4 Int64))
  (output (: 6 Int64)))

(case
  "a sequential let binding names an earlier binding in a later initializer"
  (doc
    "A sequential (let*-style) binding group: `(let ((s (+ a b)) (t (+ s s))) (+ t 1))` — `t`'s
           initializer reads the earlier `s`. a=3,b=4 -> s=7 -> t=14 -> 14+1 = 15. Pins that a later
           binding's initializer sees the earlier bindings in the same group.")
  (input
    (do (def (main (: a Int64) (: b Int64)) (let ((s (+ a b)) (t (+ s s))) (+ t 1))) (export main)))
  (call main (: 3 Int64) (: 4 Int64))
  (output (: 15 Int64)))

(case
  "a multi-use runtime bool binding is reused in a nested condition"
  (doc
    "A `let`-bound Bool used as the condition of two nested `if`s: `(let ((p (< a b))) (if p (if p 1
           2) 3))`. a=1,b=9 -> p=true -> inner p=true -> 1; a=9,b=1 -> p=false -> 3. Pins that a bound
           comparison result is reused value-transparently across both branch tests.")
  (input
    (do (def (main (: a Int64) (: b Int64)) (let ((p (< a b))) (if p (if p 1 2) 3))) (export main)))
  (call main (: 1 Int64) (: 9 Int64))
  (output (: 1 Int64))
  (call main (: 9 Int64) (: 1 Int64))
  (output (: 3 Int64)))

(case
  "a nested-if tower sharing an arm flattens and preserves the truth table"
  (doc
    "(if c1 x (if c2 x y)) = (if (or c1 c2) x y) [shared THEN]; (if c1 (if c2 x y) y) =
           (if (and c1 c2) x y) [shared ELSE]. main = 100*A1 + A2 with x=10,y=20: (T,T)→1010, (T,F)→1020,
           (F,T)→1020, (F,F)→2020.")
  (input
    (do
      (def
        (main (: c1 Bool) (: c2 Bool))
        (+ (* 100 (if c1 10 (if c2 10 20))) (if c1 (if c2 10 20) 20)))
      (export main)))
  (call main (: true Bool) (: true Bool))
  (output (: 1010 Int64))
  (call main (: true Bool) (: false Bool))
  (output (: 1020 Int64))
  (call main (: false Bool) (: true Bool))
  (output (: 1020 Int64))
  (call main (: false Bool) (: false Bool))
  (output (: 2020 Int64)))

(case
  "a flattened nested-if shared arm preserves trap shielding (reached only when a condition selects it)"
  (doc
    "(if c1 (/ 10 n) (if c2 (/ 10 n) 20)): the shared `/` is reached exactly when c1||c2. Neither
           selects (F,F) at n=0 → 20 (shielded); reached via c1 or c2 at n=0 → traps.")
  (input
    (do
      (def (main (: c1 Bool) (: c2 Bool) (: n Int64)) (if c1 (/ 10 n) (if c2 (/ 10 n) 20)))
      (export main)))
  (call main (: false Bool) (: false Bool) (: 0 Int64))
  (output (: 20 Int64))
  (call main (: true Bool) (: false Bool) (: 0 Int64))
  (trap "divide by zero")
  (call main (: false Bool) (: true Bool) (: 0 Int64))
  (trap "divide by zero"))

(case
  "a flattened nested-if combined condition short-circuits a trapping second condition"
  (doc
    "(if c1 10 (if (> (/ 10 n) 0) 10 20)): the trapping c2 `/` is evaluated only when c1 is false
           (short-circuit or). c1=true,n=0 → 10 (shielded); c1=false,n=0 → traps.")
  (input
    (do (def (main (: c1 Bool) (: n Int64)) (if c1 10 (if (> (/ 10 n) 0) 10 20))) (export main)))
  (call main (: true Bool) (: 0 Int64))
  (output (: 10 Int64))
  (call main (: false Bool) (: 0 Int64))
  (trap "divide by zero"))

(case
  "a repeated condition in a nested if collapses to the outer condition (value parity)"
  (doc
    "Within a branch the enclosing condition is known, so a directly-nested if on the SAME condition
           collapses: `(if c (if c 1 2) 3)` = `(if c 1 3)` — c=true → 1 (inner c true), c=false → 3.")
  (input (do (def (main (: c Bool)) (if c (if c 1 2) 3)) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 3 Int64)))

(case
  "a negated repeated condition in a nested if collapses (value parity)"
  (doc
    "In the else-branch c is known false, so `(if (not c) A B)` is A: `(if c 1 (if (not c) 2 3))` =
           `(if c 1 2)` — c=true → 1, c=false → (not c true) → 2.")
  (input (do (def (main (: c Bool)) (if c 1 (if (not c) 2 3))) (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 2 Int64)))

; A CONSTANT wrong-typed operand to a collection / conversion op is a TYPE error (CDZ0203), NOT a
; misleading/uncoded const-fold decline. The wrong-typed operand reaches the lowering decline before the
; type-check surfaces; the lowering now DEFERS to infer's authoritative CDZ0203 ("expects an argument of
; type (Set _)/(Map _ _)/Int64, got Int64/String") via the neutral "(see the type error above)" decline
; that dedup_faults drops. The runtime/unsolved forms keep their honest declines (this only fires for a
; DEFINITE non-matching kind). Sibling of the String-op faces (#6870/#6875); reported by v-deferral-declines.
(case
  "a constant non-Set operand to Set.contains is a coded type mismatch, not an uncoded decline"
  (input (do (def (main) (Set.contains 5 0)) (export main)))
  (error CDZ0203))
(case
  "a constant non-Set operand to Set.insert is a coded type mismatch, not an uncoded decline"
  (input (do (def (main) (Set.insert 5 0)) (export main)))
  (error CDZ0203))
(case
  "a constant non-Map operand to Map.lookup is a coded type mismatch, not an uncoded decline"
  (input (do (def (main) (Map.lookup 5 0)) (export main)))
  (error CDZ0203))
(case
  "a constant non-integer operand to Int64.of is a coded type mismatch, not a misleading conversion decline"
  (input (do (def (main) (Int64.of "x")) (export main)))
  (error CDZ0203))
