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

(case "Blake3.of of the empty byte string is the official empty-input BLAKE3-256 digest"
  (doc    "The load-bearing byte-identity pin: `Blake3.of b\"\"` is the OFFICIAL BLAKE3 empty-input
           vector `af1349b9...e41f3262` (32 bytes). Because the compile-time fold and the runtime op both
           call the one `blake3` crate over the same bytes, this same digest is what a runtime hash of the
           same input produces (design-compiler-primitives.md §9). Executes byte-identical — the value is
           materialized and compared, not merely folded.")
  (input  (Blake3.of b""))
  (output (: b"\xaf\x13\x49\xb9\xf5\xf9\xa1\xa6\xa0\x40\x4d\xea\x36\xdc\xc9\x49\x9b\xcb\x25\xc9\xad\xc1\x12\xb7\xcc\x9a\x93\xca\xe4\x1f\x32\x62" Bytes)))

(case "Blake3.of is a 32-byte digest"
  (doc    "A BLAKE3-256 digest is always 32 bytes, whatever the input length. Pins the output width so a
           downstream `Bytes.eq` against a same-width contract-id is well-formed.")
  (input  (Bytes.len (Blake3.of b"the quick brown fox")))
  (output (: 32 Int64)))

(case "Blake3.of is deterministic — the same bytes hash to the same digest"
  (doc    "Hashing the SAME input twice yields the SAME digest — the property that makes a hash usable as
           a content-address / identity. `Bytes.eq` over the two digests is true.")
  (input  (= (Blake3.of b"abc") (Blake3.of b"abc")))
  (output (: true Bool)))

(case "Blake3.of is sensitive — a one-bit-different input hashes to a different digest"
  (doc    "Two inputs differing by a single byte hash to DIFFERENT digests, so distinct declarations get
           distinct ids (exact-hash routing depends on this). `Bytes.eq` over the two digests is false.")
  (input  (= (Blake3.of b"abc") (Blake3.of b"abd")))
  (output (: false Bool)))

(case "Blake3.of composes with Ast.encode — hashing an encoded AST folds to a 32-byte digest"
  (doc    "The shape of the contract-id path (design §4), with NO contract knowledge in the compiler:
           `Ast.encode` folds a compile-time AST value to its canonical bytes (a compile-time `ConstBytes`),
           and `Blake3.of` folds those bytes to a 32-byte digest at compile time. Here the whole
           `Blake3.of (Ast.encode …)` const-folds; its length is 32. A userspace transform would sit
           between the two, canonicalizing the declaration — but the primitives compose exactly like this.")
  (input  (Bytes.len (Blake3.of (Ast.encode (Ast.Int 7)))))
  (output (: 32 Int64)))

; --- Runtime Blake3.of (heap op 91 `hash-blake3`) + the section-9 byte-identity witness ------------
; Blake3.of over a RUNTIME Bytes (not a compile-time constant) lowers to the value-heap op 91; the digest
; is byte-identical to the compile-time fold (both call the one blake3 crate over the same bytes). A
; runtime `b` arrives through the entry call, so `Blake3.of b` takes the runtime path, not the const fold.

(case "Blake3.of of a runtime Bytes is a 32-byte digest (runtime op 91)"
  (doc    "`(Bytes.of (list (UInt8.wrap k) (UInt8.wrap k) (UInt8.wrap k)))` over a runtime entry param `k` builds a RUNTIME Bytes (not a constant),
           so `Blake3.of` of it lowers to the runtime `hash-blake3` heap op, returning a fresh 32-byte Bytes.
           Pins the runtime lowering executes and yields the 32-byte width.")
  (input  (do
            (def (run (: k Int64)) (Bytes.len (Blake3.of (Bytes.of (list (UInt8.wrap k) (UInt8.wrap k) (UInt8.wrap k))))))
            (export run)))
  (call   run 5)
  (output (: 32 Int64)))

(case "runtime Blake3.of equals the compile-time fold of the same bytes (section-9 byte-identity)"
  (doc    "THE section-9 cross-check: the runtime `hash-blake3` op over a RUNTIME Bytes `(Bytes.of (list
           (UInt8.wrap k)))` (k=5) produces the SAME digest as the COMPILE-TIME fold `Blake3.of b\"\x05\"`
           (a ConstBytes baked at compile time). Both call the one blake3 crate over the same one byte, so
           `Bytes.eq` is true — compile==runtime, witnessed end-to-end. (This is also the P4 dispatch shape:
           `Bytes.eq` of a runtime digest against a compile-time id constant.)")
  (input  (do
            (def (run (: k Int64)) (= (Blake3.of (Bytes.of (list (UInt8.wrap k)))) (Blake3.of b"\x05")))
            (export run)))
  (call   run 5)
  (output (: true Bool)))

; --- The primitives COMPOSE into a contract-id (design §4 — validate the primitives suffice) -----------
; A userspace contract-id is `tag ++ blake3(canonical-declaration-bytes)`. These pin that the three
; primitives compose to a COMPILE-TIME CONSTANT tagged id, entirely in the compiler's contract-agnostic
; surface: a compile-time Ast (here a `quote`; import reflection's `__ast__` is the same value form) ->
; `Ast.encode` -> `Blake3.of` -> `Bytes.concat` with a userspace domain tag. All const-fold; the whole id
; is a baked constant a guest compares against `msg.contract` with `Bytes.eq`. (The concrete contract-id
; SCHEME + the .cdz library live in the platform lane; this only pins that the primitives are sufficient.)

(case "the primitives compose to a compile-time tagged contract-id of the expected length"
  (doc    "`Bytes.concat (userspace 0x01 tag) (Blake3.of (Ast.encode <decl-Ast>))` folds to a 33-byte
           constant (1 tag byte + 32 blake3 bytes) — a content-address of the declaration AST, built with
           NO contract knowledge in the compiler. Pins the design §4 composition end-to-end at compile time.")
  (input  (Bytes.len (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))))
  (output (: 33 Int64)))

(case "the composed contract-id is deterministic — the same declaration folds to the same id"
  (doc    "Hashing the SAME declaration AST twice yields the same tagged id (a content-address). `Bytes.eq`
           over the two folded ids is true.")
  (input  (= (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
             (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))))
  (output (: true Bool)))

(case "distinct declarations fold to distinct contract-ids"
  (doc    "Two DIFFERENT declaration ASTs (differing name) fold to DIFFERENT tagged ids — the property that
           makes exact-hash contract routing sound. `Bytes.eq` over the two ids is false.")
  (input  (= (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
             (Bytes.concat b"\x01" (Blake3.of (Ast.encode (quote (contract temp-fahrenheit Int64 Int64)))))))
  (output (: false Bool)))

(case "the compile-time contract-id fold is byte-identical to the host Contract (golden #3238)"
  (doc    "BYTE-IDENTITY cross-check (design §9): the compile-time primitive fold and the host contract-id
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
  (input  (= (Bytes.concat b"\x01"
               (Blake3.of (Ast.encode
                 (quote (contract "temp.celsius" (types (type Temp (Mk f64))) Temp Temp)))))
             b"\x01\x86\x0c\x7a\xcb\x43\xd2\x6c\xb9\x3a\x8c\xed\xd7\xd6\x97\xb2\x30\x08\x8a\x50\xd5\x0e\xb2\x37\x6c\xcf\xee\x60\xca\xba\x2c\x52\x3a"))
  (output (: true Bool)))

; --- Primitive 2: const execution — a TAKEN trap on the const-folded path is FAIL-LOUD ---------------
; A `trap("msg")` reached while const-EXECUTING a total function (const-demanded arguments) is not a silent
; decline to a runtime value: the evaluator surfaces the trap's MESSAGE as the compile error CDZ0304 (the
; provable-trap code). This is what makes a userspace self-reflection transform's genuine-absence trap (a
; malformed contract module, a missing required pragma) a fail-loud, actionable authoring error at the
; fold — not a corrupt id. A `trap` on an UN-taken branch of a const fold is never executed, so it does not
; decline the fold (the sibling cases above fold through such dead arms); only a TAKEN trap surfaces.

(case "a taken const-fold trap surfaces its message as a compile error (CDZ0304)"
  (doc    "A recursive `const`-param countdown reaches `trap(\"…\")` on the const-executed path when its
           argument is compile-time-known (`f 3` counts down to 0). The general const-evaluator executes the
           trap and surfaces its MESSAGE as the provable-trap compile error CDZ0304 — a const-executed trap
           is fail-loud, not a decline to a runtime trap. Message-matched so the surfaced text is pinned.")
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "const countdown reached zero") (f (- n 1))))
            (def (main) (f 3))
            (export main)))
  (error  CDZ0304 (message "const countdown reached zero")))

(case "a const trap consumed by Ast.encode surfaces its message, not the generic decline"
  (doc    "The self-reflection shape: a trap reached while folding `Ast.encode`'s operand. `Ast.encode`
           DEMANDS a compile-time-constant AST, and the trapping operand comes through a NON-recursive
           `const`-param fn the ordinary inliner reduces to a textless trap (message dropped) — so
           `Ast.encode` const-evaluates its operand and surfaces the trap's MESSAGE as CDZ0304 rather than
           the generic \"Ast.encode of a runtime AST value\" decline. This is the P4 contract-id library's
           genuine-absence trap (a missing required pragma) surfacing an actionable compile error.")
  (input  (do
            (def (f (const (: n Int64)))
              (if (= n 0) (trap "const trap under Ast.encode") n))
            (def (g) (Ast.encode (Ast.Int (BigInt.of (f 0)))))
            (export g)))
  (error  CDZ0304 (message "const trap under Ast.encode")))

; --- Primitive 2: const execution — Option-threaded navigation folds (nullary variant, no sentinel) --
; The operator-directed clean form of a self-reflection transform threads `Option` (not sentinel Name/""
; values) through its AST navigation: a helper returns `Option.None` on a non-match. When that `_ =>
; Option.None` arm is TAKEN on the const-folded path, the const-evaluator must FOLD the nullary variant to
; its empty-payload sum value — NOT decline (a bare `Option.None` resolves through a `(. Option None)`
; member / the `(intrinsic sum-new)` constructor head, which the evaluator must recognize as the value the
; variant denotes). This is what lets the no-sentinel contract-id library const-fold (design D-clean-form).

(case "const execution folds a TAKEN nullary variant (Option.None) in Option-threaded AST navigation"
  (doc    "`name-of` returns `Option.None` for a non-`Ast.Name` form; `label` matches that Option and, on
           the `Option.None` branch, builds `(Ast.Name \"none\")`. Fed a non-Name `(Ast.Int 5)`, the `_ =>
           Option.None` arm is TAKEN, so folding `label` requires const-EVALUATING the nullary `Option.None`
           value. `Ast.encode` DEMANDS a compile-time constant, so if the taken `Option.None` did not fold the
           encode would decline (\"runtime AST value\") and this equality could not be computed. Folds to
           `Ast.encode (Ast.Name \"none\")`, equal to the RHS — witnessing the nullary-variant const-fold that
           the Option-threaded (no-sentinel) navigation depends on.")
  (input  (do
            (def (name-of (const (: form Ast)))
              (match form ((Ast.Name n) (Option.Some n)) (_ Option.None)))
            (def (label (const (: form Ast)))
              (match (name-of form) ((Option.None) (Ast.Name "none")) ((Option.Some n) (Ast.Name n))))
            (def (run)
              (= (Ast.encode (label (Ast.Int (BigInt.of 5)))) (Ast.encode (Ast.Name "none"))))
            (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — HIGHER-ORDER folds (a closure passed to a `const` fn parameter) -----
; A `const f: (T) -> U` parameter carrying a `fn` argument is captured as a first-class closure and APPLIED
; per element, so a user recursive map/filter/fold that threads a closure const-folds. Without first-class
; const closures the fold declined (a lambda is not a `is_const_value`, and a closure parameter has no
; static lambda for the evaluator to reduce). (The stdlib `List.map`/`filter`/`fold` are augmentations that
; expand to iterator pipelines the evaluator does not yet interpret — a separate decline class.)

(case "const execution folds a higher-order recursion — a closure applied per element"
  (doc    "`mymap` threads a `const` closure `f` and applies it to each element of a `const` list. Fed the
           mapper `(fn (x) (Ast.Int (BigInt.of x)))` over `(list 1 2 3)`, the whole higher-order recursion
           const-folds to `(list (Ast.Int 1) (Ast.Int 2) (Ast.Int 3))`. `Ast.encode` DEMANDS a compile-time
           constant, so a non-folding higher-order call would decline the encode; the fold's bytes equal the
           encoding of the literal list, witnessing that the closure was captured + applied at compile time.")
  (input  (do
            (def (mymap (const (: xs (List Int64))) (const (: f (-> Int64 Ast))))
              (match xs
                ((list) (: (list) (List Ast)))
                ((list h .. t) (List.prepend (mymap t f) (f h)))))
            (def (run)
              (= (Ast.encode (Ast.List (mymap (list 1 2 3) (fn ((: x Int64)) (Ast.Int (BigInt.of x))))))
                 (Ast.encode (Ast.List (list (Ast.Int (BigInt.of 1)) (Ast.Int (BigInt.of 2)) (Ast.Int (BigInt.of 3)))))))
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

(case "a recursive const (Option Int64) param reaching a trap folds to CDZ0304, not a runtime hang"
  (doc    "`f (Option.Some 2)` counts the payload down and reaches `trap` at 0. The `(Option Int64)` const
           param now ACTIVATES the recursive const-fold, which executes the countdown and surfaces the taken
           trap's MESSAGE as CDZ0304 — where before the param type failed the activation gate, so a runtime
           recursive call was emitted whose trap base case ran to a non-terminating wasm loop (divergent from
           the rust backend, which trapped). Folding eliminates the runtime artifact, closing the divergence.")
  (input  (do
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (if (= k 0) (trap "adv option reached zero") (f (Option.Some (- k 1)))))
                ((Option.None) 0)))
            (def (main) (f (Option.Some 2)))
            (export main)))
  (error  CDZ0304 (message "adv option reached zero")))

(case "a recursive const (Option Int64) param with a VALUE base case folds to the value"
  (doc    "The trap-free twin: the same `(Option Int64)` const-param countdown returns a value at 0. It
           const-folds to that value (99), witnessed by encoding it and comparing to the encoded literal —
           `Ast.encode` demands a compile-time constant, so a non-folding recursion would decline the encode.")
  (input  (do
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (if (= k 0) 99 (f (Option.Some (- k 1)))))
                ((Option.None) 0)))
            (def (run)
              (= (Ast.encode (Ast.Int (BigInt.of (f (Option.Some 2)))))
                 (Ast.encode (Ast.Int (BigInt.of 99)))))
            (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — the `(const <expr>)` FORCE-EVAL / const-DEMAND block ----------------
; `(const e)` (operator-requested) forces `e` to reduce to a compile-time constant and REJECTS if it cannot.
; It is the explicit const-DEMAND marker: the evaluator runs on `e` DIRECTLY, so a total computation over
; compile-time-known data folds WITHOUT threading `const` params through its callees (dropping that clunk).
; A residual runtime value is an authoring error (CDZ0201), not a silent pass-through to a runtime compute.

(case "a `(const …)` force-eval block folds a helper call over constant data (no const params needed)"
  (doc    "`sq` declares NO `const` parameter, yet `(const (sq 5))` folds to 25 at compile time: the `(const
           …)` block is the demand signal, so the evaluator interprets `sq 5` directly. This is the construct
           the operator wanted — force a computation to const-fold at the use site instead of threading
           `const` params through every helper. The folded value is the ordinary scalar 25.")
  (input  (do
            (def (sq (: x Int64)) (* x x))
            (def (main) (const (sq 5)))
            (export main)))
  (output (: 25 Int64)))

(case "a `(const …)` block whose expression depends on runtime data is rejected (CDZ0201)"
  (doc    "`(const (+ k 1))` over a runtime entry parameter `k` cannot reduce to a compile-time constant, so
           the force-eval block REJECTS with CDZ0201 — the block ASSERTS compile-time evaluability, so a
           residual runtime value is a fail-loud authoring error, not a silent pass-through to a runtime add.")
  (input  (do
            (def (main (: k Int64)) (const (+ k 1)))
            (export main)))
  (error  CDZ0201 (message "const` block requires a compile-time constant")))

; --- Primitive 2: const execution — MUTUAL recursion over a compound (Ast) value folds -----------------
; The general const-evaluator reaches a MUTUALLY-recursive pair of `const` functions over an `Ast` value:
; `leaves` (per-node) ↔ `leaves-list` (per-child-list) count the leaves of a quoted form. Pins that mutual
; recursion + a `(List Ast)` const param + a per-element sum all compose in one const-fold (the P4 self-
; reflection transform shape). Regression guard for the recursive engine's reach over compound compositions.

(case "mutual recursion over an Ast value const-folds a leaf count (under Ast.encode demand)"
  (doc    "`leaves (quote (f 1 2))` = 3 (three leaf nodes: `f`, `1`, `2`) via a mutually-recursive
           `leaves`/`leaves-list` pair, both `const`. `Ast.encode` demands a compile-time constant, so the
           whole mutual recursion must const-fold for the encode to succeed; its bytes equal the encoding of
           the literal count 3 — witnessing the fold reached across the mutual recursion + the `(List Ast)`
           param + the per-element `+`.")
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a ((Ast.List xs) (leaves-list xs)) (_ 1)))
            (def (leaves-list (const (: xs (List Ast))))
              (match xs
                ((list) 0)
                ((list h .. t) (+ (leaves h) (leaves-list t)))))
            (def (run)
              (= (Ast.encode (Ast.Int (BigInt.of (leaves (quote (f 1 2))))))
                 (Ast.encode (Ast.Int (BigInt.of 3)))))
            (export run)))
  (output (: true Bool)))

; --- Primitive 2: const execution — TUPLE destructuring in the recursive evaluator --------------------
; The const-evaluator now DESTRUCTURES a tuple pattern `(tuple a b)` in a `match` (the matcher recognizes
; the tuple pattern; a binder reads its slot via an `Elem` step over the `CVal::Tuple`). This lets a `(const
; …)`-demanded computation over a tuple const value fold — a `(Tuple …)` const param is otherwise excluded
; from the bare recursive-fold activation gate (it is a product/dictionary shape a counter-driven consumer
; passes unchanged), so `(const …)` is the demand marker that forces such a fold.

(case "a `(const …)` block destructures a tuple in a match and folds"
  (doc    "`(const (match (tuple 3 5) ((tuple a b) (+ a b))))` folds to 8: the evaluator matches the tuple
           pattern and reads binders `a`/`b` out of the `CVal::Tuple` (an `Elem`-step projection), which
           previously declined. The `(const …)` block forces the fold (a `(Tuple …)` shape is not activated
           by the bare gate).")
  (input  (do
            (def (main) (const (match (tuple 3 5) ((tuple a b) (+ a b)))))
            (export main)))
  (output (: 8 Int64)))

(case "a `(const …)` block folds a recursion over a tuple const value"
  (doc    "`f` counts the first tuple slot down to 0 while accumulating into the second, threading a fresh
           `(tuple …)` each step. `(const (f (tuple 3 0)))` folds to 3 — the tuple pattern destructure, the
           tuple construction, and the recursion all compose in one const-fold under the force-eval block.")
  (input  (do
            (def (f (const (: t (Tuple Int64 Int64))))
              (match t ((tuple a b) (if (= a 0) b (f (tuple (- a 1) (+ b 1)))))))
            (def (main) (const (f (tuple 3 0))))
            (export main)))
  (output (: 3 Int64)))

; --- Primitive 2: const execution — `(const …)` is never STRICTER than the ordinary fold --------------
; The force-eval block first tries the ORDINARY compile-time fold (`core_of`) and only falls back to the
; general value-interpreter for what that doesn't reach (recursion/composition). So `(const e)` folds
; EVERYTHING the plain compiler already folds — a `Float`/`Bytes`/record/tuple constant expression the
; value-interpreter has no value for still folds through the block, rather than a false CDZ0201 reject.

(case "a `(const …)` block folds a constant Float expression (never stricter than the ordinary fold)"
  (doc    "`(const (+ 1.5 2.0))` folds to 3.5 — the ordinary `core_of` fold already produces the constant
           Float, so the force-eval block returns it rather than rejecting (the general value-interpreter
           carries no Float value, so a const_eval-only block wrongly rejected this perfectly-constant expr).")
  (input  (do
            (def (main) (const (+ 1.5 2.0)))
            (export main)))
  (output (: 3.5 Float64)))

; --- Primitive 2: const execution — RECORD field projection folds ------------------------------------
; A direct field projection off a constant record `(. (record …) field)` const-folds (the evaluator builds
; a `CVal::Record` and projects the field by name). Regression guard for record const-folding — the value
; class the P4 descriptor `contract(m) -> Record(id, …)` reads a field off. A record threaded THROUGH a
; record-param FUNCTION now folds too: a `(record …)` LITERAL resolves to an APPLIED `RecordNew` (not the
; symbol-headed `Resolved::Record`), so the value-interpreter reduces it via `reduce_ctor` to the symbol-
; headed compound and builds a `CVal::Record` — evaluating each field in the current env, so a record built
; from const-param values folds as well.

(case "a direct record field projection const-folds"
  (doc    "`(. (record (x 7) (y 9)) x)` folds to 7 at compile time; witnessed byte-exact through `Ast.encode`
           (which demands a compile-time constant) against the encoding of the literal 7.")
  (input  (do
            (def (main)
              (= (Ast.encode (Ast.Int (BigInt.of (. (record (= x 7) (= y 9)) x))))
                 (Ast.encode (Ast.Int (BigInt.of 7)))))
            (export main)))
  (output (: true Bool)))

(case "a record threaded through a record-param function const-folds"
  (doc    "`(const (get-x (record (x 42) (y 7))))` folds to 42: the record LITERAL resolves to an applied
           `RecordNew`, which the value-interpreter reduces (via `reduce_ctor`) to a `CVal::Record`, binds to
           the `get-x` parameter, and projects `x` off inside the body. Previously declined — `apply_const_prim`
           had no `RecordNew` arm — so a record-param helper under a `(const …)` block rejected CDZ0201. The
           param's record type is recovered by inference from the `(. r x)` projection (no annotation needed).")
  (input  (do
            (def (get-x r) (. r x))
            (def (main) (const (get-x (record (= x 42) (= y 7)))))
            (export main)))
  (output (: 42 Int64)))

(case "a record BUILT from a const param then read through a function const-folds"
  (doc    "`mk n` builds `(record (x n) (y (* n 2)))` from its const param, `sum-r` reads both fields; `(const
           (sum-r (mk 10)))` folds to 30. Exercises the general case: the compound constructor evaluates each
           field in the CURRENT env (the bound `n`), not just a param-free literal — the value-interpreter twin
           of how a `(tuple …)` built from const params already folded.")
  (input  (do
            (def (mk (const (: n Int64))) (record (= x n) (= y (* n 2))))
            (def (sum-r r) (+ (. r x) (. r y)))
            (def (main) (const (sum-r (mk 10))))
            (export main)))
  (output (: 30 Int64)))

; --- Primitive 2: const execution — MAP query folds (lookup / replace) under `(const …)` --------------
; A `(const …)`-demanded MAP QUERY const-folds: `Map.empty`/`Map.insert`/`Map.lookup` evaluate over a
; `CVal::Map` (an assoc list, latest-write-per-key wins). Only order-INDEPENDENT results fold (a looked-up
; value); a const map is NEVER materialized to a `Core` — its runtime CHAMP iteration order is not presumed
; (soundness), so an order-exposing use (encode / to-list) still declines rather than risk a wrong order.

(case "a `(const …)` map lookup folds to the associated value"
  (doc    "`Map.lookup (Map.insert (Map.empty) 2 20) 2` folds to `Option.Some 20`; the match extracts 20.")
  (input  (do
            (def (main)
              (const (match (Map.lookup (Map.insert (Map.empty) 2 20) 2)
                       ((Option.Some v) v)
                       ((Option.None) 0))))
            (export main)))
  (output (: 20 Int64)))

(case "a `(const …)` map insert replaces a key in place (latest write wins)"
  (doc    "Inserting key 1 twice keeps the LATEST value; `Map.lookup … 1` folds to 99, not 10 — the const
           map's `insert` replaces the association in place, matching the runtime persistent-map semantics.")
  (input  (do
            (def (main)
              (const (match (Map.lookup (Map.insert (Map.insert (Map.empty) 1 10) 1 99) 1)
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

(case "a `(const …)` set membership + dedup-len fold"
  (doc    "`Set.of (list 1 2 2 3)` dedups to {1,2,3} (len 3); `Set.contains … 2` is true. The const set is
           queried, never materialized. Here `Set.len` of the deduped set folds to 3.")
  (input  (do
            (def (main) (const (Set.len (Set.of (list 1 2 2 3)))))
            (export main)))
  (output (: 3 Int64)))

(case "a `(const …)` set contains after insert folds"
  (doc    "`Set.contains (Set.insert (Set.of (list 1 2)) 3) 3` folds to true (100 branch) — insert adds a
           new member, contains queries it, all at compile time under the force-eval block.")
  (input  (do
            (def (main) (const (if (Set.contains (Set.insert (Set.of (list 1 2)) 3) 3) 100 200)))
            (export main)))
  (output (: 100 Int64)))

(case "a `(const …)` map lookup MISS folds to Option.None"
  (doc    "Negative path: `Map.lookup … 2` on a map lacking key 2 folds to `Option.None`; the match's None
           arm yields 0. Pins the absent-key branch (the positive lookup is pinned above).")
  (input  (do
            (def (main)
              (const (match (Map.lookup (Map.insert (Map.empty) 1 10) 2)
                       ((Option.Some v) v)
                       ((Option.None) 0))))
            (export main)))
  (output (: 0 Int64)))

(case "a `(const …)` set contains of an ABSENT member folds to false"
  (doc    "Negative path: `Set.contains (Set.of (list 1 2 3)) 9` folds to false → the 200 branch. Pins the
           absent-member result (the present-member case is pinned above).")
  (input  (do
            (def (main) (const (if (Set.contains (Set.of (list 1 2 3)) 9) 100 200)))
            (export main)))
  (output (: 200 Int64)))

(case "a `(const …)` force-eval block preserves its operand's ascribed width"
  (doc    "The force-eval block folds its operand at the ascribed width: `(const (: 5 Int32))` reduces to the
           32-bit constant 5, not a default-width Int64. Pins that the const-demand block is transparent to
           type grounding — it forces the ascribed value and carries its declared width through the fold.")
  (input  (const (: 5 Int32)))
  (output (: 5 Int32)))

(case "a `(const …)` block with the wrong operand count is a malformed reject (arity guard)"
  (doc    "The block is written `(const <expression>)` — EXACTLY one operand. `(const 1 2)` (two) is a coded
           malformed reject at resolve (CDZ0201), a DISTINCT path from the runtime-dependence reject above: it
           fires on the FORM's arity before any force-eval, never a silent pass-through of the first operand.")
  (input  (const 1 2))
  (error  CDZ0201))

; --- Primitive 0/2: Ast.encode folds ANY operand the const-evaluator can reduce (demand-path) -----------
; `Ast.encode` DEMANDS a compile-time-constant AST. When `core_of` alone does not fold its operand (a
; Map/Set query, a composition inside a `const`-param fn), the encode now const-EVALUATES the operand and
; MATERIALIZES a folded Ast value through the SAME canonical codec — so it folds byte-identical, rather
; than the generic "runtime AST value" decline. (A genuinely-runtime operand still declines; a taken trap
; surfaces CDZ0304.)

(case "Ast.encode folds a Map query composed inside a const-param fn (const_eval demand path)"
  (doc    "`label` (a `const`-param fn) does a `Map.lookup` and wraps the value in `Ast.Int`; `core_of` alone
           leaves the map query runtime, but `Ast.encode`'s operand const-EVALUATES it to the constant
           `Ast.Int 9` and encodes it — byte-identical to encoding the literal `(Ast.Int 9)`. Before, this
           declined ('runtime AST value') even though the value is fully compile-time-known.")
  (input  (do
            (def (label (const (: n Int64)))
              (match (Map.lookup (Map.insert (Map.empty) 1 n) 1)
                ((Option.Some v) (Ast.Int (BigInt.of v)))
                ((Option.None) (Ast.Name "none"))))
            (def (main)
              (= (Ast.encode (label 9)) (Ast.encode (Ast.Int (BigInt.of 9)))))
            (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — a compound BUILT IN A RECURSION folds -----------------------------
; The value-interpreter reduces an applied compound constructor (`RecordNew`/`TupleNew`/`ListNew`) in the
; CURRENT env (#3516), so a recursion that constructs a FRESH compound each step — the record analogue of
; the tuple-threading case above — const-folds. These pin that generalization in a recursive setting (the
; #3516 cases were non-recursive): a record built from a recursively-computed field, and Bytes assembled by
; repeated `Bytes.concat`. Regression guard for compound construction composing with the recursive engine.

(case "a recursion that builds a fresh RECORD each step const-folds a field read"
  (doc    "`walk n` returns `(record (v k))` where `k` counts the recursion depth; `(const (. (walk 4) v))`
           folds to 4. Each step builds a fresh `(record (v …))` — an applied `RecordNew` whose field is the
           recursively-computed value — which the interpreter evaluates in the current env, then the outer
           `(. … v)` projects. Before #3516 the record construction declined (`apply_const_prim` had no
           `RecordNew` arm), so the whole recursion did.")
  (input  (do
            (def (walk (const (: n Int64)))
              (if (= n 0) (record (= v 0)) (record (= v (+ 1 (. (walk (- n 1)) v))))))
            (def (main) (const (. (walk 4) v)))
            (export main)))
  (output (: 4 Int64)))

(case "a recursion that builds Bytes via Bytes.concat const-folds byte-exact"
  (doc    "`bcat n` concatenates `n` copies of the one-byte `A` (0x41); `(const (bcat 3))` folds to the 3-byte
           sequence `AAA`, witnessed byte-exact through `Ast.encode` against `Bytes.of (list 65 65 65)`. Pins
           `Bytes.concat` + an empty-`Bytes` base composing across the recursive engine under a force-eval
           block.")
  (input  (do
            (def (bcat (const (: n Int64)))
              (if (= n 0) (Bytes.of (list)) (Bytes.concat (Bytes.of (list 65)) (bcat (- n 1)))))
            (def (main)
              (= (Ast.encode (Ast.Bytes (const (bcat 3))))
                 (Ast.encode (Ast.Bytes (Bytes.of (list 65 65 65))))))
            (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — a CHAR value threads through the recursive engine -----------------
; The value-interpreter carries a `CVal::Char` (materializes to `Core::ConstChar`), so a `Char` const param
; folds through a RECURSION. A NON-recursive Char op already folds via `core_of` (`Char.to-int`/`from-int`);
; the recursive engine runs the interpreter, which had no Char value, so `(const …)` over a recursive
; Char-param fn declined (cb04). Equality/ordering compare by scalar; `Char.to-int` reads it; `Char.from-int`
; builds one fallibly (`Option`, `None` for a surrogate / out-of-range int).

(case "a Char threaded through a recursion const-folds (cb04)"
  (doc    "`walk n c` adds 1 per step down to `Char.to-int c` at the base; `(const (walk 3 #\\a))` folds to
           3 + 97 = 100 (`#\\a` scalar is 97). The Char const param binds a `CVal::Char` through the
           recursion — before, the interpreter had no Char value so the fold declined CDZ0201.")
  (input  (do
            (def (walk (const (: n Int64)) (const (: c Char)))
              (if (= n 0) (Char.to-int c) (+ 1 (walk (- n 1) c))))
            (def (main) (const (walk 3 #\a)))
            (export main)))
  (output (: 100 Int64)))

(case "Char equality in a recursion counts matches under const"
  (doc    "`cnt n c` counts, over `n` steps, how many equal `#\\a` — `(const (cnt 4 #\\a))` folds to 4. Pins
           Char `=` (scalar compare) composing with the recursive engine.")
  (input  (do
            (def (cnt (const (: n Int64)) (const (: c Char)))
              (if (= n 0) 0 (if (= c #\a) (+ 1 (cnt (- n 1) c)) (cnt (- n 1) c))))
            (def (main) (const (cnt 4 #\a)))
            (export main)))
  (output (: 4 Int64)))

(case "Char.from-int then to-int round-trips under const"
  (doc    "`(const (f 97))` folds to 97: `Char.from-int 97` = `(Some #\\a)`, matched and read back with
           `Char.to-int`. Pins the fallible `Int -> (Option Char)` folding through a const-param fn + match.")
  (input  (do
            (def (f (const (: n Int64)))
              (match (Char.from-int n) ((Option.Some c) (Char.to-int c)) ((Option.None) -1)))
            (def (main) (const (f 97)))
            (export main)))
  (output (: 97 Int64)))

(case "Char.from-int of a surrogate folds to None under const"
  (doc    "`(const (f 55296))` folds to -1: 55296 = U+D800 is a UTF-16 surrogate, not a scalar value, so
           `Char.from-int` yields `(None)` and the match takes the absent arm. Pins the fallible path's
           negative direction (out-of-range/surrogate) at compile time.")
  (input  (do
            (def (f (const (: n Int64)))
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

(case "a taken trap over a Char const param on the bare-call path surfaces CDZ0304"
  (doc    "`(f #\\a 2)` counts `n` to 0 then traps; the bare recursive-call fold executes the trap and
           surfaces its message as CDZ0304 — symmetric with the Int64 shape. Before `Core::ConstChar` was a
           recognized const value, the gate skipped the fold and the Char-returning `f` failed to emit.")
  (input  (do
            (def (f (const (: c Char)) (const (: n Int64)))
              (if (= n 0) (trap "char base reached") (f c (- n 1))))
            (def (main) (f #\a 2))
            (export main)))
  (error  CDZ0304 (message "char base reached")))

(case "a Char const param folds a VALUE through a bare recursive call"
  (doc    "`(f #\\a 3)` recurses `n` to 0 then reads `Char.to-int c` = 97 — the value form of the bare-call
           path (no `(const …)` block), folding a Char threaded through the recursion.")
  (input  (do
            (def (f (const (: c Char)) (const (: n Int64)))
              (if (= n 0) (Char.to-int c) (f c (- n 1))))
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

(case "a Float64 accumulator threaded through a recursion const-folds"
  (doc    "`fsum n acc` adds 1.5 per step down to `acc`; `(const (fsum 3 0.0))` folds to 4.5. The Float const
           param binds a `CVal::Float` through the recursion and each `+` folds at the node's Float64 width.")
  (input  (do
            (def (fsum (const (: n Int64)) (const (: acc Float64)))
              (if (= n 0) acc (fsum (- n 1) (+ acc 1.5))))
            (def (main) (const (fsum 3 0.0)))
            (export main)))
  (output (: 4.5 Float64)))

(case "Float64 comparison in a recursion counts matches under const"
  (doc    "`cnt n x` counts, over `n` steps, how many times `x < 2.0` — `(const (cnt 4 1.5))` folds to 4.
           Pins float `<` (IEEE partial order) composing with the recursive engine.")
  (input  (do
            (def (cnt (const (: n Int64)) (const (: x Float64)))
              (if (= n 0) 0 (if (< x 2.0) (+ 1 (cnt (- n 1) x)) (cnt (- n 1) x))))
            (def (main) (const (cnt 4 1.5)))
            (export main)))
  (output (: 4 Int64)))

(case "a taken trap over a Float64 const param on the bare-call path surfaces CDZ0304"
  (doc    "`(f 1.5 2)` counts `n` to 0 then traps; the bare recursive-call fold executes the trap and surfaces
           its message as CDZ0304 — a Float const value is recognized by `is_const_value` (`Core::ConstFloat`),
           so the activation gate fires (symmetric with Int64/Char).")
  (input  (do
            (def (f (const (: x Float64)) (const (: n Int64)))
              (if (= n 0) (trap "float base reached") (f x (- n 1))))
            (def (main) (f 1.5 2))
            (export main)))
  (error  CDZ0304 (message "float base reached")))

(case "Float64 equality by canonical bits folds through a recursion"
  (doc    "`mk n` recurses to the constant `0.0`; `(const (= (mk 2) 0.0))` folds to true — float `=` under the
           canonical byte form, evaluated in the prim block at the operand width.")
  (input  (do
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

(case "a scalar field projected off a nullary const fn's record folds to the field, dropping the record"
  (doc    "`descriptor()` returns `(record (id 42) (extra (quote …)))`; `(const (. (descriptor) id))` folds to
           42, the `extra` sibling never materialized. Before, the nullary call declined in the value-
           interpreter and the record built whole.")
  (input  (do
            (def (descriptor) (record (= id 42) (= extra (quote (a b c)))))
            (def (main) (const (. (descriptor) id)))
            (export main)))
  (output (: 42 Int64)))

(case "a Bytes field projected off a nullary record with Ast siblings folds byte-exact and eliminates the record"
  (doc    "The operator's structured descriptor shape: `descriptor()` returns a record whose `id` is a
           `Blake3.of(Ast.encode …)` Bytes and whose siblings are `Ast` values (no runtime form). Reading
           `(. (descriptor) id)` — NO `(const …)` block, the ordinary member fold — reduces to the id Bytes
           (equal to the direct fold) and drops the Ast siblings, so the record never materializes. Before,
           the whole record built and the component failed to instantiate.")
  (input  (do
            (def (descriptor)
              (record (= id (Blake3.of (Ast.encode (quote (contract c Int64 Int64)))))
                      (= input (quote Int64))
                      (= output (quote Int64))))
            (def (main)
              (= (. (descriptor) id) (Blake3.of (Ast.encode (quote (contract c Int64 Int64))))))
            (export main)))
  (output (: true Bool)))

(case "a field projection off a nullary record whose field came from a RECURSIVE helper folds"
  (doc    "`descriptor()` builds `(record (id (leaves (quote …))) …)` where `leaves` is a RECURSIVE const fn;
           `(const (. (descriptor) id))` folds to 3. `descriptor` itself is not in a cycle (the recursion is
           at the HELPER, not back to `descriptor`), so the nullary-body evaluation fires and the helper folds
           via the param-recursion path — the shape of the operator's `descriptor() = contract(Ast.module)`,
           where `contract` recurses over the module forms. (A genuinely self-recursive nullary `(def (f) (f))`
           is instead declined at the recursion bound, so the fold never overflows the native stack.)")
  (input  (do
            (def (leaves (const (: a Ast)))
              (match a ((Ast.List xs) (List.len xs)) (_ 1)))
            (def (descriptor) (record (= id (leaves (quote (f 1 2)))) (= extra (quote z))))
            (def (main) (const (. (descriptor) id)))
            (export main)))
  (output (: 3 Int64)))

; --- Primitive 2: const execution — a Tuple const param folds on the BARE-CALL path (ca06) ------------
; The recursive-call activation gate accepts a `const` param whose type is a bare NAME or a SHRINKING type-
; constructor, but EXCLUDED `(Tuple …)` (grouped with the product/dictionary forms). A `const (: t (Tuple …))`
; param in a counter-in-tuple recursion genuinely SHRINKS, and the value-interpreter folds tuples — but the
; gate skipped it, so a `const` Tuple-param fn could not be called on the bare path (no `(const …)` block):
; it emitted a runtime fn whose `const` Tuple param has no runtime slot ("function return type has no machine
; representation"). Admitting `(Tuple …)` fires the fold, symmetric with the scalar const params. (`(Record …)`
; stays excluded — that is the ad-hoc-polymorphism dictionary-consumer shape, which inlines+erases at runtime.)

(case "a Tuple const param folds a VALUE through a bare recursive call (ca06)"
  (doc    "`f` counts the first tuple slot to 0 while accumulating into the second; `(f (tuple 3 0))` — NO
           `(const …)` block — folds to 3. The `const` Tuple param binds a `CVal::Tuple` and the recursion
           folds; before, the gate excluded `(Tuple …)` so the `const`-param fn reached the emitter.")
  (input  (do
            (def (f (const (: t (Tuple Int64 Int64))))
              (match t ((tuple a b) (if (= a 0) b (f (tuple (- a 1) (+ b 1)))))))
            (def (main) (f (tuple 3 0)))
            (export main)))
  (output (: 3 Int64)))

(case "a taken trap over a Tuple const param on the bare-call path surfaces CDZ0304 (ca06)"
  (doc    "`(f (tuple 2 0))` counts to 0 then traps; the bare recursive-call fold executes the trap and
           surfaces its message as CDZ0304 — symmetric with the Int64/Char/Float const-param shapes.")
  (input  (do
            (def (f (const (: t (Tuple Int64 Int64))))
              (match t ((tuple a b) (if (= a 0) (trap "tuple base reached") (f (tuple (- a 1) (+ b 1)))))))
            (def (main) (f (tuple 2 0)))
            (export main)))
  (error  CDZ0304 (message "tuple base reached")))

; --- Primitive 2: const execution — the OPERATOR'S structured whole-record descriptor form ------------
; The platform's structured contract descriptor is a 5-field record `(id Bytes, name String, input Ast,
; output Ast, types Ast)` returned by a nullary `descriptor()` (= `contract(Ast.module)`); a guest reads
; ONE scalar field off it (`descriptor().id` for routing, `descriptor().name` for display). #3543 made the
; field projection ELIMINATE the record — the three non-representable `Ast` siblings never materialize — so
; the whole-record form instantiates + runs (not just the `Bytes` forms). These pin that end-to-end: BOTH a
; Bytes and a String field project off the same multi-Ast-sibling record and fold, with NO `(const …)` block
; (the ordinary member fold, exactly as a guest writes it). Regression guard for the #3413 structured form.

(case "the structured descriptor's String name field projects and eliminates the record"
  (doc    "`descriptor()` returns the 5-field `(id name input output types)` record; `(. (descriptor) name)`
           folds to the name String and the `Ast`-typed siblings drop — no `(const …)` block. Before #3543 the
           record materialized (its `Ast` fields have no runtime form) and the component failed to instantiate.")
  (input  (do
            (def (descriptor)
              (record (= id (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
                      (= name "cdz-platform.temp-celsius")
                      (= input (quote Int64)) (= output (quote Int64)) (= types (quote (list)))))
            (def (main) (= (. (descriptor) name) "cdz-platform.temp-celsius"))
            (export main)))
  (output (: true Bool)))

(case "the structured descriptor's Bytes id field projects byte-exact and eliminates the record"
  (doc    "The routing path: `(. (descriptor) id)` off the same 5-field record folds to the `Blake3.of(Ast.encode
           …)` id Bytes (equal to the direct fold), dropping the `Ast` siblings — no `(const …)` block. Pins the
           operator's structured whole-record descriptor form folds + eliminates for the id, same as the name.")
  (input  (do
            (def (descriptor)
              (record (= id (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64)))))
                      (= name "cdz-platform.temp-celsius")
                      (= input (quote Int64)) (= output (quote Int64)) (= types (quote (list)))))
            (def (main) (= (. (descriptor) id) (Blake3.of (Ast.encode (quote (contract temp-celsius Int64 Int64))))))
            (export main)))
  (output (: true Bool)))

; --- Primitive 2: const execution — SET ALGEBRA + Set/Map removal fold (query-only) -------------------
; The value-interpreter folds `Set.union`/`intersection`/`difference` + `Set.remove` + `Map.remove`, the
; companions of the `Set.of`/`insert`/`contains`/`len` + `Map.*` ops it already carried. Membership is by
; `cval_eq`; an undecidable comparison declines the whole op (never a wrong member set). Like every const
; Set/Map these are QUERY-ONLY — the result set/map is never MATERIALIZED (`cval_to_core` declines a
; `CVal::Set`/`CVal::Map`, since the runtime CHAMP iteration order must not be presumed); only order-
; INDEPENDENT results (a size, a membership) leave the fold. A `(const …)` block forces the fold.

(case "Set.union const-folds a membership count"
  (doc "`{1,2} ∪ {2,3}` has 3 distinct members; the union folds and `Set.len` reads 3 (order-independent).")
  (input (do (def (main) (const (Set.len (Set.union (Set.of (list 1 2)) (Set.of (list 2 3)))))) (export main)))
  (output (: 3 Int64)))
(case "Set.intersection const-folds a membership count"
  (doc "`{1,2,3} ∩ {2,3,4}` = `{2,3}`; `Set.len` reads 2.")
  (input (do (def (main) (const (Set.len (Set.intersection (Set.of (list 1 2 3)) (Set.of (list 2 3 4)))))) (export main)))
  (output (: 2 Int64)))
(case "Set.difference const-folds a membership count"
  (doc "`{1,2,3} ∖ {2}` = `{1,3}`; `Set.len` reads 2.")
  (input (do (def (main) (const (Set.len (Set.difference (Set.of (list 1 2 3)) (Set.of (list 2)))))) (export main)))
  (output (: 2 Int64)))
(case "Set.remove then contains const-folds to false"
  (doc "Removing `2` from `{1,2,3}` then testing membership of `2` folds to false.")
  (input (do (def (main) (const (Set.contains (Set.remove (Set.of (list 1 2 3)) 2) 2))) (export main)))
  (output (: false Bool)))
(case "Map.remove then len const-folds"
  (doc "Removing key `1` from a 2-entry map leaves 1 entry; `Map.len` reads 1.")
  (input (do (def (main) (const (Map.len (Map.remove (Map.insert (Map.insert (Map.empty) 1 10) 2 20) 1)))) (export main)))
  (output (: 1 Int64)))
(case "Map.remove then lookup misses"
  (doc "Looking up the removed key `1` folds to `Option.None` (the match's absent arm → -1).")
  (input (do (def (main) (const (match (Map.lookup (Map.remove (Map.insert (Map.empty) 1 10) 1) 1) ((Option.Some v) v) ((Option.None) -1)))) (export main)))
  (output (: -1 Int64)))

; --- Primitive 2: const execution — the CHAMP-ORDER SOUNDNESS negative (never materialize a const Map/Set) --
; A const Map/Set is QUERY-ONLY: `cval_to_core` DECLINES a `CVal::Map`/`CVal::Set`, so an ORDER-EXPOSING use —
; materializing it to a list via `Map.to-list`/`Set.to-list` — does NOT fold. The runtime collection is a CHAMP
; whose iteration order the compiler must not presume, so folding a to-list would bake a PRESUMED order (a
; miscompile if it differs from the runtime's). The query ops (lookup/contains/len/algebra) fold because their
; results are order-INDEPENDENT; a to-list is not. These pin the soundness NEGATIVE (previously only a comment):
; a `(const …)` over a Map/Set to-list REJECTS. A future change that materialized a const Map in some order
; would make these FOLD — and fail the pin, catching the order-presumption regression.

(case "a const Map materialized to a list declines — CHAMP iteration order is not presumed (soundness)"
  (doc    "`Map.to-list` over a constant map is order-EXPOSING; const_eval never materializes a `CVal::Map`
           (`cval_to_core` declines it), so `(const (List.len (Map.to-list …)))` REJECTS rather than baking a
           presumed insertion order. The query ops fold; a to-list does not.")
  (input  (do (def (main) (const (List.len (Map.to-list (Map.insert (Map.insert (Map.empty) 1 10) 2 20))))) (export main)))
  (error  CDZ0201 (message "compile-time constant")))

(case "a const Set materialized to a list declines — CHAMP iteration order is not presumed (soundness)"
  (doc    "The Set twin: `Set.to-list` over a constant set is order-EXPOSING; const_eval never materializes a
           `CVal::Set`, so `(const (List.len (Set.to-list …)))` REJECTS. Only order-independent Set results
           (contains/len/algebra) fold.")
  (input  (do (def (main) (const (List.len (Set.to-list (Set.of (list 1 2 3)))))) (export main)))
  (error  CDZ0201 (message "compile-time constant")))

;; -- closed pure handles fold under (const ...) across state kinds: scalar, String, tuple, record (breaker batch 386; List/Map/Set states = the open collection-state seam) --
(case "chk1 a closed pure handle with SCALAR state folds under (const ...)"
  (input (do
    (effect E (op tick (-> Int64)))
    (def (main) (const (handle E 40 ((tick () s (resume s (+ s 1)))) (+ (E.tick) 2))))
    (export main)))
  (output (: 42 Int64)))

(case "chk2 a closed handle with STRING state folds under (const ...)"
  (input (do
    (effect E (op tick (-> Int64)))
    (def (main)
      (const
        (handle E "x"
          ((tick () s (resume (String.byte-len s) (String.concat s "y"))))
          (+ (* 10 (E.tick)) (E.tick)))))
    (export main)))
  (output (: 12 Int64)))

(case "cms1 closed handle with TUPLE state under (const ...)"
  (input (do
    (effect E (op tick (-> Int64)))
    (def (main)
      (const
        (handle E (tuple 1 10)
          ((tick () s
             (match s
               ((tuple a b) (resume (+ a b) (tuple (+ a 1) (* b 2)))))))
          (+ (E.tick) (E.tick)))))
    (export main)))
  (output (: 33 Int64)))

(case "cms2 closed handle with RECORD state under (const ...)"
  (input (do
    (effect E (op tick (-> Int64)))
    (def (main)
      (const
        (handle E (record (= a 1) (= b 10))
          ((tick () s
             (resume (+ (. s a) (. s b))
                     (record (= a (+ (. s a) 1)) (= b (* (. s b) 2))))))
          (+ (E.tick) (E.tick)))))
    (export main)))
  (output (: 33 Int64)))

;; -- fail-loud TIMING is demand-scoped, by design (v-cp ruling): a NON-recursive const fn taken trap stays a RUNTIME trap on the bare call (it may sit under a runtime branch; unconditional CDZ0304 would over-reject) and surfaces CDZ0304 only under an explicit (const ...) demand; recursive const fns surface on both paths (breaker batch 391) --
(case "cn02a a NON-recursive const fn taken trap is a RUNTIME trap on the bare call (by-design: it may sit under a runtime branch)"
  (input (do
    (def (f (const (: n Int64)))
      (if (= n 0) (trap "cn02a int zero") n))
    (def (main) (f 0))
    (export main)))
  (trap "unreachable"))

(case "cn02b the Option twin — bare-call taken trap stays a runtime trap (by-design)"
  (input (do
    (def (f (const (: o (Option Int64))))
      (match o
        ((Option.Some k) (if (= k 0) (trap "cn02b zero payload") k))
        ((Option.None) 0)))
    (def (main) (f (Option.Some 0)))
    (export main)))
  (trap "unreachable"))

(case "cn02c the tuple twin — bare-call taken trap stays a runtime trap (by-design)"
  (input (do
    (def (f (const (: t (Tuple Int64 Int64))))
      (match t ((tuple a b) (if (= a b) (trap "cn02c fields met") a))))
    (def (main) (f (tuple 3 3)))
    (export main)))
  (trap "unreachable"))

(case "cn02d the SAME shape under a (const ...) demand DOES surface CDZ0304 (the explicit fail-loud opt-in)"
  (input (do
    (def (f (const (: o (Option Int64))))
      (match o
        ((Option.Some k) (if (= k 0) (trap "cn02d zero payload") k))
        ((Option.None) 0)))
    (def (main) (const (f (Option.Some 0))))
    (export main)))
  (error CDZ0304 (message "cn02d zero payload")))

; --- Primitive: RUNTIME Ast.print (heap op 92) — render a runtime Ast to canonical s-expr text ------------
; A COMPILE-TIME-visible Ast folds to a `Core::ConstStr` (`lower_print`); a RUNTIME Ast (built from a runtime
; input) lowers to `Core::AstPrint {operand, discs}` → the value-heap `ast-print` op (heap index 92), which
; walks the heap Ast and renders it BYTE-IDENTICAL to the compile-time `print_ast_value` fold. `discs` is a
; baked descriptor of the 7 Ast variant discs (LEB [int,float,bool,str,name,bytes,list]) the op reads to
; classify variants by name. Runtime print == compile-time print — pinned end-to-end (the op runs).

(case "runtime Ast.print renders a top-level Ast.Int to its decimal text"
  (doc    "`n` is a runtime entry param, so `(Ast.Int (BigInt.of n))` is a RUNTIME Ast → `Ast.print` lowers to
           the `ast-print` heap op (op 92). `(run 42)` renders \"42\" — identical to the compile-time fold.")
  (input  (do (def (run (: n Int64)) (Ast.print (Ast.Int (BigInt.of n)))) (export run)))
  (call   run 42)
  (output (: "42" String)))

(case "runtime Ast.print renders a nested Ast.List byte-identical to the compile-time fold"
  (doc    "The nested case (#3621: the op reads list elements via vec-get): `(Ast.List (list (Ast.Name \"f\")
           (Ast.Int (BigInt.of n))))` at runtime renders `(f 2)` — parens, space-separated, each element
           recursively, identical to `print_ast_value`.")
  (input  (do (def (run (: n Int64)) (Ast.print (Ast.List (list (Ast.Name "f") (Ast.Int (BigInt.of n)))))) (export run)))
  (call   run 2)
  (output (: "(f 2)" String)))

(case "runtime Ast.print renders a doubly-nested Ast.List"
  (doc    "A list-of-list — `((f 2))` — pins the recursive vec-get walk to depth 2.")
  (input  (do (def (run (: n Int64)) (Ast.print (Ast.List (list (Ast.List (list (Ast.Name "f") (Ast.Int (BigInt.of n)))))))) (export run)))
  (call   run 2)
  (output (: "((f 2))" String)))

(case "compile-time Ast.print of the same Ast folds to the identical text"
  (doc    "The compile-time control: a CONSTANT `Ast.Int` folds to the `Core::ConstStr` \"42\" via
           `print_ast_value` — the same text the runtime op produces, witnessing runtime==compile-time.")
  (input  (do (def (main) (Ast.print (Ast.Int (BigInt.of 42)))) (export main)))
  (output (: "42" String)))

; --- Primitive 2: const execution — a (const …) HANDLE with growing collection state folds (cm02) --------
; A closed finite `handle` with a const init, under `(const …)`, folds its answer: const_eval's Handle arm
; DELEGATES to `reduce_handle` (the effect reducer — threads continuations/resumes/state) and const-evaluates
; the resulting PURE AST. `reduce_handle` keeps a GROWING collection state as a re-read `let` binding, which
; `core_of` alone leaves as a `Core::Let` this stage cannot fold; const-evaluating the reduced AST folds the
; query answers via the List/Set/Map arms. Reuses the one reducer (no duplication). (Diagnosis: v-effects.)

(case "a const handle threading a growing List state folds its query answers"
  (doc    "`E.tick` resumes `List.len s` and threads `(List.prepend s 0)` (state GROWS by one each perform).
           `(const (handle E (list 7) … (+ (* 10 (E.tick)) (E.tick))))`: first tick reads len 1 → resumes 1,
           state → `(0 7)`; second reads len 2 → resumes 2; `(+ (* 10 1) 2)` = 12. Before, the growing-list
           state kept a `Core::Let` the const block could not fold (CDZ0201); now the Handle arm reduces +
           const-evaluates it.")
  (input  (do
            (effect E (op tick (-> Unit Int64)))
            (def (main) (const (handle E (list 7) ((tick (u) s (resume (List.len s) (List.prepend s 0)))) (+ (* 10 (E.tick)) (E.tick)))))
            (export main)))
  (output (: 12 Int64)))

(case "a const handle threading a growing Set state folds its query answers"
  (doc    "The Set-state companion: `E.tick` resumes `Set.len s` and threads `(Set.insert s 0)`. Seeded
           `{7}`: len 1 → 1, then `{7,0}` len 2 → 2; `(+ (* 10 1) 2)` = 12. Same Handle-arm reduce+fold path;
           the query answer (a size) is order-independent so it folds soundly.")
  (input  (do
            (effect E (op tick (-> Unit Int64)))
            (def (main) (const (handle E (Set.of (list 7)) ((tick (u) s (resume (Set.len s) (Set.insert s 0)))) (+ (* 10 (E.tick)) (E.tick)))))
            (export main)))
  (output (: 12 Int64)))

;; -- (const ...) handles with COLLECTION states fold: growing List, Map insert+lookup, growing Set (breaker batch 395; the #3636 flip — completes the state-kind matrix begun in batch 386) --
(case "cm02 closed MULTI-dispatch handle with LIST state folds under (const ...)"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main)
              (const (handle E (list 7)
                ((tick () s (resume (List.len s) (List.prepend s 0))))
                (+ (* 10 (E.tick)) (E.tick)))))
            (export main)))
  (output (: 12 Int64)))

(case "cms3 closed handle with MAP state under (const ...)"
  (input (do
    (effect E (op tick (-> Int64)))
    (def (rd (: m (Map Int64 Int64)))
      (match (Map.lookup m 0)
        ((Option.Some v) v)
        ((Option.None) -1)))
    (def (main)
      (const
        (handle E (Map.insert (map) 0 5)
          ((tick () s (resume (rd s) (Map.insert s 0 (+ (rd s) 1)))))
          (+ (E.tick) (E.tick)))))
    (export main)))
  (output (: 11 Int64)))

(case "cms4 closed handle with SET state under (const ...)"
  (input (do
    (effect E (op tick (-> Int64)))
    (def (main)
      (const
        (handle E (Set.of (list 1))
          ((tick () s (resume (Set.len s) (Set.insert s (+ (Set.len s) 1)))))
          (+ (E.tick) (E.tick)))))
    (export main)))
  (output (: 3 Int64)))
