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
; class the P4 descriptor `contract(m) -> Record(id, …)` reads a field off. (A record projected THROUGH a
; record-param FUNCTION does not yet fold — the reducer does not inline a record-param fn and the general
; evaluator does not reach its body; tracked separately, deeper than a value-domain arm.)

(case "a direct record field projection const-folds"
  (doc    "`(. (record (x 7) (y 9)) x)` folds to 7 at compile time; witnessed byte-exact through `Ast.encode`
           (which demands a compile-time constant) against the encoding of the literal 7.")
  (input  (do
            (def (main)
              (= (Ast.encode (Ast.Int (BigInt.of (. (record (x 7) (y 9)) x))))
                 (Ast.encode (Ast.Int (BigInt.of 7)))))
            (export main)))
  (output (: true Bool)))

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
