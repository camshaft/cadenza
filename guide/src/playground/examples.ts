/// Curated starter programs for the playground's Examples dropdown. Each is a full program (the
/// playground buffer is compiled verbatim) authored in the s-expression surface as flat top-level
/// definitions grouped in a `(do …)` (no `module` wrapper — that's boilerplate unless an example's
/// point IS modules); the surface toggle re-renders them into idiomatic top-level ML `def`s. All are
/// verified to compile + run against the current compiler.

import type { Surface } from "../compiler/client.ts";

export interface Example {
  /// Stable kebab-case identifier — the deep-link target (`/playground?example=<id>`) and the nav key.
  /// Distinct from `name` (the human label) so a rename of the label never breaks a deep-link.
  id: string;
  name: string;
  /// Which themed nav bucket this example sits under in the sidebar's "Examples" section. Drives the
  /// grouping v-guide-infra renders from data (no hardcoded nav list). Kept coarse (4 buckets) so 36
  /// examples stay scannable: basics (language fundamentals), algorithms (classic computations),
  /// data-and-collections (lists/maps/sets/records at work), numbers (numeric-model showcases).
  theme: "basics" | "algorithms" | "data-and-collections" | "numbers";
  surface: Surface;
  source: string;
  /// Optional pinned result, in the S-EXPR canonical render. When set, the check-examples gate asserts the
  /// program runs to exactly this value — turning the example into a regression test rather than a mere
  /// "it runs" check. BOTH scalars (bare number/bool) AND compound values are pinnable: a playground
  /// example has no in-browser Check (it's loaded and Run), so an s-expr-canonical compound like
  /// `(: (list 1 2 3) (List Int64))` is stable to pin. The value is compared on the s-expr pass, and the
  /// gate runs both surfaces — so a pin IS checked either way, but for an ML-authored example it's only
  /// asserted against the RENDERED s-expr toggle output (brittle: depends on a byte-stable ML→s-expr
  /// render, and the pin reads in a different surface than it's maintained in). So pins must be
  /// `surface: "sexpr"` — all playground examples are, and check-examples FAILS LOUDLY on a non-sexpr pin.
  /// (A graded chapter EXERCISE differs: it's compared in the reader's LIVE surface, where a compound
  /// renders differently in ML vs s-expr, so exercise pins stay scalar-only — see check-examples.)
  expected?: string;
  /// When `true`, this example is a teaching case authored to NOT compile (the "see the squiggle" type
  /// error). The check-examples gate then asserts it DECLINES rather than runs — an intentional negative
  /// case. Declared explicitly here (not sniffed from `source`) so re-authoring the example's body to a
  /// different type error can't silently flip it back to a value-example that then fails the sweep.
  expectError?: boolean;
}

export const EXAMPLES: Example[] = [
  {
    id: "hello-arithmetic",
    name: "Hello, arithmetic",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (def (main) (+ 2 3))
  (export main))`,
    expected: "5",
  },
  {
    id: "recursive-sum",
    name: "A recursive sum",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (def (sm n)
    (if (= n 0) 0 (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main))`,
    expected: "15",
  },
  {
    id: "pattern-matching",
    name: "Pattern matching",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (def (main)
    (match 2
      (1 10)
      (2 20)
      (_ 0)))
  (export main))`,
    expected: "20",
  },
  {
    id: "option-sum-types",
    name: "Option & sum types",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (type Opt (Some Int64) (None unit))
  (def (main)
    (match (Some 7)
      ((Some x) x)
      ((None _) 0)))
  (export main))`,
    expected: "7",
  },
  {
    id: "records",
    name: "Records",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (def (area r) (* (. r w) (. r h)))
  (def (main) (area #record((= w 4) (= h 5))))
  (export main))`,
    expected: "20",
  },
  {
    // Shows off: MATCHING on a record — destructure its fields right in the match arm, and use a
    // LITERAL field (x 0)/(y 0) to single out a special case. Here the origin scores 0; any other
    // point scores x² + y². (3,4) => 25. In ML the arms read `{ x = 0, y = 0 }` and `{ x = xv, y = yv }`.
    id: "record-patterns",
    name: "Record patterns (match on fields)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; Match destructures the record's fields; a literal field value guards the origin case.
  (def (score p)
    (match p
      ((record (x 0) (y 0)) 0)
      ((record (x xv) (y yv)) (+ (* xv xv) (* yv yv)))))
  (def (main) (score (record (x 3) (y 4))))
  (export main))`,
    expected: "25",
  },
  {
    // Shows off: a record NESTED inside a compound pattern. The tuple's first slot is a point
    // record, and (x px) (y py) bind its fields in place — no intermediate binding, no new type.
    // The second slot binds the weight alongside. weigh((3,4), 2) => 2·(9+16) = 50. In ML the arm
    // reads `({ x = px, y = py }, w)`.
    id: "nested-record-patterns",
    name: "Nested record patterns (destructure in place)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; A match arm destructures a record nested inside a compound pattern — the tuple's first
  ; slot is a point, and (x px) (y py) bind its fields directly; w binds the weight.
  (def (weigh pair)
    (match pair
      ((tuple (record (x px) (y py)) w) (* w (+ (* px px) (* py py))))))
  (def (main) (weigh (tuple (record (x 3) (y 4)) 2)))
  (export main))`,
    expected: "50",
  },
  {
    // Shows off: a match arm reaching THROUGH a variant into the record it carries. `(Some (record
    // (x px) (y py)))` binds the point's fields directly from inside the Option — the variant×record
    // corner that completes {tuple,list,variant}×record nested patterns. dist(Some(3,4)) => 9+16 = 25,
    // None => 0. In ML the arm reads `Some({ x = px, y = py })`.
    id: "variant-nested-record-patterns",
    name: "Variant-nested record patterns (match through a variant)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; A match arm reaches through a variant into its record payload: (Some (record ...)) binds
  ; the point's fields directly; the None arm handles the empty case.
  (def (dist opt)
    (match opt
      ((Some (record (x px) (y py))) (+ (* px px) (* py py)))
      ((None) 0)))
  (def (main) (dist (Some (record (x 3) (y 4)))))
  (export main))`,
    expected: "25",
  },
  {
    id: "tuple",
    name: "A tuple",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (def (main) #tuple(1 2 3))
  (export main))`,
    expected: "(: #tuple(1 2 3) (Tuple Int64 Int64 Int64))",
  },
  {
    // Shows off: `Tuple.split-at` and `Tuple.concat` — tuples have STRUCTURAL, compile-time-known arity,
    // so these cut a tuple into a (head, tail) pair at a fixed index and weld two tuples back into one.
    // Split (1 2 3 4 5) at 2 into (1 2) and (3 4 5), then Tuple.concat rebuilds the original — concat is
    // split's inverse. The result shows the head, the tail, and the rebuilt tuple.
    id: "tuple-split-concat",
    name: "Splitting & joining tuples",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; Tuples have STRUCTURAL, compile-time-known arity. Tuple.split-at cuts a tuple
  ; into a (head, tail) pair of tuples at a fixed index; Tuple.concat is its inverse,
  ; welding two tuples back into one. Split (1 2 3 4 5) at 2, then rebuild it.
  (def (main)
    (let ((parts (Tuple.split-at #tuple(1 2 3 4 5) 2)))
      (match parts
        (#tuple(head tail)
         #tuple(head tail (Tuple.concat head tail))))))
  (export main))`,
    expected: "(: #tuple(#tuple(1 2) #tuple(3 4 5) #tuple(1 2 3 4 5)) (Tuple (Tuple Int64 Int64) (Tuple Int64 Int64 Int64) (Tuple Int64 Int64 Int64 Int64 Int64)))",
  },
  {
    id: "lists",
    name: "Lists",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; Concatenate two lists and return the RESULT — you see the whole list, not just its length.
  (def (main)
    (List.concat #list(1 2) #list(3 4 5)))
  (export main))`,
    expected: "(: #list(1 2 3 4 5) (List Int64))",
  },
  {
    // Shows off: `List.update` — a FUNCTIONAL element update: `(List.update xs i v)` returns a NEW list
    // with index i replaced by v, leaving the original untouched (indices must be in [0, len)). We update
    // index 0 and index 4 of the same list, then show the ORIGINAL is unchanged — lists are persistent.
    id: "list-update-functional",
    name: "Functional list update",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; List.update xs i v : return a NEW list with index i replaced by v (a functional
  ; update — the original list is unchanged). Indices must be in [0, len).
  (def (set-at xs i v) (List.update xs i v))
  (def (main)
    (let ((xs #list(10 20 30 40 50)))
      ; Update index 0 and index 4 of xs; then show xs ITSELF is untouched.
      #tuple((set-at xs 0 99) (set-at xs 4 99) xs)))
  (export main))`,
    expected: "(: #tuple(#list(99 20 30 40 50) #list(10 20 30 40 99) #list(10 20 30 40 50)) (Tuple (List Int64) (List Int64) (List Int64)))",
  },
  {
    // Shows off: a recursive sum type + structural pattern matching = a real (if tiny) interpreter.
    // Each `match` arm destructures one Expr shape; `eval` recurses into the subtrees. This is the
    // "look what you can build" hook — an AST evaluator in a dozen lines. Computes (2 + 3) * -(4) = -20.
    id: "expression-interpreter",
    name: "Expression interpreter",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; A tiny arithmetic language, evaluated by structural recursion over its AST.
  (type Expr
    (Lit Int64)
    (Add (Tuple Expr Expr))
    (Mul (Tuple Expr Expr))
    (Neg Expr))
  (def (eval e)
    (match e
      ((Lit n) n)
      ((Add p) (+ (eval (. p 0)) (eval (. p 1))))
      ((Mul p) (* (eval (. p 0)) (eval (. p 1))))
      ((Neg x) (- 0 (eval x)))))
  ; (2 + 3) * -(4)  ==>  -20
  (def (main)
    (eval (Mul #tuple((Add #tuple((Lit 2) (Lit 3)))
                      (Neg (Lit 4))))))
  (export main))`,
    expected: "-20",
  },
  {
    // Shows off: tail-style recursion + integer arithmetic driving a classic number-theory routine.
    // The Collatz orbit of 27 famously climbs to 9232 before falling — this counts its 111 steps.
    id: "collatz-orbit",
    name: "Collatz orbit length",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Count the steps for n to reach 1 under the Collatz map
  ; (n/2 if even, 3n+1 if odd). 27 is the famously long orbit.
  (def (collatz n steps)
    (if (= n 1)
        steps
        (if (= (% n 2) 0)
            (collatz (/ n 2) (+ steps 1))
            (collatz (+ (* 3 n) 1) (+ steps 1)))))
  (def (main) (collatz 27 0))
  (export main))`,
    expected: "111",
  },
  {
    // Shows off: first-class functions — a function that RETURNS a function (a closure over f and g).
    // `compose` builds a new function; applying it to 20 runs inc then double ==> 42.
    id: "function-composition",
    name: "Function composition",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; compose builds a NEW function that runs g then f — a closure over both.
  (def (compose f g) (fn (x) (f (g x))))
  (def (double x) (* x 2))
  (def (inc x) (+ x 1))
  (def (main) ((compose double inc) 20))
  (export main))`,
    expected: "42",
  },
  {
    // Shows off: higher-order functions — `apply-n-times` takes a FUNCTION as an argument and applies it
    // n times; `adder` RETURNS a closure that captures k. So (apply-n-times (adder 3) 4 10) adds 3 four
    // times (10→13→16→19→22), and an inline lambda doubling 1 five times gives 32. Functions are values.
    id: "higher-order-apply-n",
    name: "Higher-order functions (apply n times)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; apply-n-times takes a FUNCTION f and applies it n times to x: f(f(...f(x))).
  ; f is an ordinary value — passed in, then called in the recursion.
  (def (apply-n-times f n x)
    (if (= n 0) x (apply-n-times f (- n 1) (f x))))
  ; adder RETURNS a closure that adds a fixed k to its argument.
  (def (adder k) (fn (x) (+ x k)))
  (def (main)
    #tuple(
      ; add 3, four times: 10 -> 13 -> 16 -> 19 -> 22
      (apply-n-times (adder 3) 4 10)
      ; double, five times: 1 -> 2 -> 4 -> 8 -> 16 -> 32
      (apply-n-times (fn (x) (* x 2)) 5 1)))
  (export main))`,
    expected: "(: (tuple 22 32) (Tuple Int64 Int64))",
  },
  {
    // Shows off: a Map used as a MEMO CACHE, threaded functionally through the recursion. Each call
    // returns (value, updated-map); the cache turns exponential fib into linear. fib(30) = 832040 —
    // it returns instantly, where naive fib(30) would make >2.7M calls.
    id: "memoized-fibonacci",
    name: "Memoized Fibonacci (Map cache)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A persistent Map threaded as a cache: fib returns (value, updated-map).
  ; Looking a result up before recomputing turns exponential fib into linear.
  (def (fib n mp)
    (match (Map.lookup mp n)
      ((Some v) #tuple(v mp))
      ((None)
       (if (< n 2)
           #tuple(n (Map.insert mp n n))
           (let ((a (fib (- n 1) mp)))
             (let ((b (fib (- n 2) (. a 1))))
               (let ((r (+ (. a 0) (. b 0))))
                 #tuple(r (Map.insert (. b 1) n r)))))))))
  (def (main) (. (fib 30 (Map.empty)) 0))
  (export main))`,
    expected: "832040",
  },
  {
    // Shows off: building a frequency table by folding a list into a Map (add-or-bump each key).
    // Uses the List.at/List.len index-recursion idiom (the prelude List has no fold). The result is a
    // (List (Tuple Int64 Int64)) of (value, count) pairs — the shape a notebook `table` cell renders.
    id: "frequency-count",
    name: "Frequency count (fold into a Map)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Count occurrences of each element, accumulating into a Map.
  (def (bump mp k)
    (match (Map.lookup mp k)
      ((Some c) (Map.insert mp k (+ c 1)))
      ((None) (Map.insert mp k 1))))
  ; List has no fold, so walk by index with List.at / List.len.
  (def (tally xs i n mp)
    (if (= i n)
        mp
        (match (List.at xs i)
          ((Some x) (tally xs (+ i 1) n (bump mp x)))
          ((None) mp))))
  (def (main)
    (let ((xs #list(3 1 3 3 1 2)))
      (Map.to-list (tally xs 0 (List.len xs) (Map.empty)))))
  (export main))`,
    expected: "(: #list(#tuple(1 2) #tuple(2 1) #tuple(3 3)) (List (Tuple Int64 Int64)))",
  },
  {
    // Shows off: `Map.swap` and `Map.take` — both return a (value, new-map) PAIR, a functional update
    // that hands you BOTH the old reading and the updated map with no separate lookup. swap sets k->v and
    // returns the previous value; take removes k and returns its value. Stock {apple:5, pear:2}: restock
    // apples to 9 (learning the old 5), then discontinue pears (taking out the 2). Result surfaces both.
    id: "map-swap-take-inventory",
    name: "Map swap & take (inventory)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Both Map.swap and Map.take return a (value, new-map) PAIR — a functional update
  ; that hands you BOTH the old value and the updated map, no separate lookup needed.
  ;   Map.swap m k v : set k -> v, return (previous value, new map)
  ;   Map.take m k   : remove k, return (its value, map without k)
  ; Stock starts {apple: 5, pear: 2} (keys 1, 2). Restock apples to 9 (learn the old
  ; 5), then discontinue pears (take out the 2), surfacing both old readings.
  (def (main)
    (let ((stock (Map.insert (Map.insert (Map.empty) 1 5) 2 2)))
      (match (Map.swap stock 1 9)
        (#tuple(old-apples restocked)
         (match (Map.take restocked 2)
           (#tuple(gone-pears final)
            #tuple(old-apples gone-pears (Map.to-list final))))))))
  (export main))`,
    expected: "(: #tuple((Some 5) (Some 2) #list(#tuple(1 9))) (Tuple (Option Int64) (Option Int64) (List (Tuple Int64 Int64))))",
  },
  {
    // Shows off: Bytes as a Map KEY. Bytes carries a total order (lexicographic over unsigned bytes),
    // so a byte string can index a Map directly — no hashing it to an Int first. Here we tally how
    // often each word (as UTF-8 bytes) occurs: "red" 3x, "blue" 1x => ((Some 3), (Some 1)).
    id: "bytes-map-key",
    name: "Bytes as a Map key",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Bump the count for a Bytes key (0 if absent), then re-insert. Bytes has a total order,
  ; so a byte string can be a Map KEY directly — no need to hash it to an Int first.
  (def (bump (: m (Map Bytes Int64)) (: k Bytes))
    (match (Map.lookup m k)
      ((Some c) (Map.insert m k (+ c 1)))
      ((None) (Map.insert m k 1))))
  (def (main)
    ; Tally how often each word (as UTF-8 bytes) occurs, keyed by the bytes themselves.
    (let ((red (String.to-bytes "red"))
          (blue (String.to-bytes "blue")))
      (let ((m (bump (bump (bump (bump (Map.empty) red) blue) red) red)))
        #tuple((Map.lookup m red) (Map.lookup m blue)))))
  (export main))`,
    expected: "(: #tuple((Some 3) (Some 1)) (Tuple (Option Int64) (Option Int64)))",
  },
  {
    // Shows off: the `Symbol` type — an INTERNED name. Two symbols built from the same text are the SAME
    // value, so equality is a cheap identity check (not a string compare), which makes symbols good tags
    // and Map keys. Here a Symbol-keyed palette maps color names to hex codes: green resolves, teal is
    // absent (None), and `Symbol.of "red" == Symbol.of "red"` is true (interning).
    id: "symbol-keyed-map",
    name: "Symbols as Map keys",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A Symbol is an INTERNED name: two symbols built from the same text are the SAME
  ; value, so equality is a cheap identity check (no string compare). That makes
  ; symbols good keys/tags. Key a Map by symbol to look up a color's hex code.
  (def (main)
    (let ((palette
            (Map.insert
              (Map.insert
                (Map.insert (Map.empty) (Symbol.of "red") 16711680)
                (Symbol.of "green") 65280)
              (Symbol.of "blue") 255)))
      #tuple(
        (Map.lookup palette (Symbol.of "green"))
        (Map.lookup palette (Symbol.of "teal"))
        (= (Symbol.of "red") (Symbol.of "red")))))
  (export main))`,
    expected: "(: #tuple((Some 65280) (None unit) true) (Tuple (Option Int64) (Option Int64) Bool))",
  },
  {
    // Shows off: a byte-string literal — b"…" is a Bytes value directly, no String.to-bytes needed.
    // "GIF89a" is the classic GIF magic-number header; its length is 6 and its first byte is 'G' (71).
    id: "byte-string-literal",
    name: "Byte-string literal",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; b"…" builds a Bytes value straight from the source text — no String.to-bytes step.
  ; "GIF89a" is the GIF magic-number header: 6 bytes, first byte 'G' (ASCII 71).
  (def (main)
    (let ((magic b"GIF89a"))
      (match (Bytes.at magic 0)
        ((Some first) #tuple((Bytes.len magic) first))
        ((None) (trap "byte-literal: unexpectedly empty")))))
  (export main))`,
    expected: "(: #tuple(6 71) (Tuple Int64 Int64))",
  },
  {
    // Shows off: `Value.encode` — the single canonical binary form of ANY value. Its type is
    // `forall a. a -> Bytes` (TOTAL: every value has a value-form), so one generic `size` helper works
    // across a record, an Option, and a list. Here we report the byte length of each canonical encoding.
    id: "value-encode-canonical-bytes",
    name: "Canonical binary encoding (Value.encode)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Value.encode : forall a. a -> Bytes — the single canonical binary form of ANY
  ; value (total: every value has one). One generic helper encodes different shapes;
  ; we report the byte length of each value's canonical encoding.
  (def (size v) (Bytes.len (Value.encode v)))
  (def (main)
    #tuple(
      (size #record((= x 3) (= y 4)))
      (size (Some 7))
      (size #list(1 2 3))))
  (export main))`,
    expected: "(: (tuple 102 61 66) (Tuple Int64 Int64 Int64))",
  },
  {
    // Shows off: `Value.encode` is DETERMINISTIC and STRUCTURAL — two structurally-equal values encode to
    // byte-identical output, so the encoding is a content address. Bytes has a total order, so we compare
    // the encodings with `=`. Two equal records -> identical bytes (true); one differing field -> different
    // bytes (false). This is what makes Value.encode usable as a canonical key/digest.
    id: "value-encode-determinism",
    name: "Structural encoding is deterministic (Value.encode)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Value.encode is deterministic + structural: two structurally-equal values encode to
  ; byte-identical output (a content address). Bytes has a total order, so we compare the
  ; encodings directly with =. Equal records -> identical bytes; a differing field -> not.
  (def (main)
    (let ((ba (Value.encode #record((= x 3) (= y 4))))
          (bb (Value.encode #record((= x 3) (= y 4))))
          (bc (Value.encode #record((= x 3) (= y 5)))))
      #tuple((Bytes.len ba) (= ba bb) (= ba bc))))
  (export main))`,
    expected: "(: (tuple 102 true false) (Tuple Int64 Bool Bool))",
  },
  {
    // Shows off: a full `Value.encode` -> `Value.decode` ROUND-TRIP. encode : forall a. a -> Bytes (total);
    // decode : forall a. Bytes -> Option a, where the result annotation grounds `a` (here `Option Point`).
    // We encode a Point, decode it back, and read the restored fields (x+y). decode is partial (Option), so
    // we match — None would mean the bytes didn't fit the type. The target is a named type (Point).
    id: "value-encode-roundtrip",
    name: "Encode / decode round-trip (Value)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A value survives a trip through its canonical bytes. Value.encode : a -> Bytes
  ; (total); Value.decode : Bytes -> Option a, with the result annotation grounding a.
  ; Decode is partial (None if the bytes don't fit the type), so we match on the Option.
  (type Point (Mk (Record (: x Int64) (: y Int64))))
  (def (main)
    (let ((p (Point.Mk #record((= x 3) (= y 4)))))
      (let ((bytes (Value.encode p)))
        (match (: (Value.decode bytes) (Option Point))
          ((Some (Point.Mk r)) #tuple((Bytes.len bytes) (+ (. r x) (. r y))))
          ((None) #tuple((Bytes.len bytes) 0))))))
  (export main))`,
    expected: "(: (tuple 73 7) (Tuple Int64 Int64))",
  },
  {
    // Shows off: EXACT rational arithmetic — 1/2 + 1/3 + 1/6 is EXACTLY 1, with no floating-point
    // drift. The `(pragma default-fraction Rational)` directive makes every bare literal in scope an
    // exact fraction; compare with Float64, where 0.1 + 0.2 famously isn't 0.3.
    id: "exact-rational-arithmetic",
    name: "Exact rational arithmetic",
    theme: "numbers",
    surface: "sexpr",
    source: `(do
  (pragma default-fraction Rational)
  ; Bare literals here are EXACT fractions, so this sums to exactly 1 — no float drift.
  (def (sum) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 6)))
  (def (main) (sum))
  (export main))`,
    expected: "(: 1/1 Rational)",
  },
  {
    // Shows off: taking a rational APART — sum exact fractions to a reduced fraction, then read its
    // numerator and denominator with Rational.numerator / Rational.denominator (each a BigInt).
    // 1/2 + 1/3 + 1/12 = 11/12 exactly, so this returns (11, 12).
    id: "rational-parts",
    name: "Rational numerator & denominator",
    theme: "numbers",
    surface: "sexpr",
    source: `(do
  (pragma default-fraction Rational)
  ; Exact sum: 1/2 + 1/3 + 1/12 = 11/12 (already in lowest terms).
  (def (total) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 12)))
  ; Pull the reduced fraction apart into its two BigInt components.
  (def (main)
    #tuple((Rational.numerator (total)) (Rational.denominator (total))))
  (export main))`,
    expected: "(: #tuple(11 12) (Tuple BigInt BigInt))",
  },
  {
    // Shows off: projecting an EXACT rational back to an integer. `Rational.floor` rounds toward -inf and
    // `Rational.ceil` toward +inf, each narrowing to Int64 — so the direction (not just truncation) is the
    // point: 7/2 = 3.5 -> floor 3 / ceil 4, and -7/2 = -3.5 -> floor -4 / ceil -3 (floor keeps going DOWN
    // for a negative, ceil UP). The result (3, 4, -4, -3) shows both signs so the toward-±inf rule is visible.
    id: "rational-floor-ceil",
    name: "Rational floor & ceil",
    theme: "numbers",
    surface: "sexpr",
    source: `(do
  (pragma default-fraction Rational)
  ; floor/ceil project an EXACT fraction to an Int64, rounding toward -inf / +inf.
  ; 7/2 = 3.5 -> floor 3, ceil 4.   -7/2 = -3.5 -> floor -4, ceil -3.
  (def (main)
    #tuple(
      (Rational.floor (/ 7 2))
      (Rational.ceil (/ 7 2))
      (Rational.floor (/ -7 2))
      (Rational.ceil (/ -7 2))))
  (export main))`,
    expected: "(: #tuple(3 4 -4 -3) (Tuple Int64 Int64 Int64 Int64))",
  },
  {
    // Shows off: Float64 rounding drift — the flip side of the exact-rational example. Adding 0.1 ten
    // times should give 1.0, but IEEE 754 can't represent 0.1 exactly, so the errors accumulate to
    // 0.9999999999999999. Also shows explicit parameter type annotations (n: Int64, acc: Float64) —
    // Cadenza never silently promotes between numeric types.
    id: "float-rounding-drift",
    name: "Float rounding drift",
    theme: "numbers",
    surface: "sexpr",
    source: `(do
  ; Add 0.1 to itself ten times. Exact math says 1.0, but Float64 rounding accumulates.
  (def (add-tenths (: n Int64) (: acc Float64))
    (if (= n 0) acc (add-tenths (- n 1) (+ acc 0.1))))
  (def (main) (add-tenths 10 0.0))
  (export main))`,
    expected: "0.9999999999999999",
  },
  {
    // Shows off: the classic Euclidean algorithm — recursion + modulo — and deriving LCM from it.
    // gcd(a,b) = gcd(b, a mod b); lcm(a,b) = a*b/gcd. lcm(12,18) = 36.
    id: "gcd-lcm-euclid",
    name: "GCD and LCM (Euclid)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Euclid's algorithm: gcd(a, b) = gcd(b, a mod b), until b is 0.
  (def (gcd a b) (if (= b 0) a (gcd b (% a b))))
  ; LCM falls out of GCD: lcm(a, b) = a * b / gcd(a, b).
  (def (lcm a b) (/ (* a b) (gcd a b)))
  (def (main) (lcm 12 18))
  (export main))`,
    expected: "36",
  },
  {
    // Shows off: Set operations as first-class values. Symmetric difference (elements in exactly one
    // of two sets) is built from union + difference: (A\\B) ∪ (B\\A). Result: {1, 2, 5, 6}.
    id: "set-algebra",
    name: "Set algebra (symmetric difference)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; The symmetric difference of two sets: elements in exactly one of them.
  ; (A minus B) union (B minus A). Sets also give you dedup for free.
  (def (sym-diff a b)
    (Set.union (Set.difference a b) (Set.difference b a)))
  (def (main)
    (let ((a (Set.of #list(1 2 3 4)))
          (b (Set.of #list(3 4 5 6))))
      (Set.to-list (sym-diff a b))))
  (export main))`,
    expected: "(: #list(1 2 5 6) (List Int64))",
  },
  {
    // Shows off: `Set.intersection` — the members two sets share. Modeling each person's friends as a
    // set, the mutual friends are exactly the intersection. `Set.of` dedups the input lists, and
    // `Set.to-list` renders the shared members in sorted order. Complements set-algebra (union/difference).
    id: "set-intersection-mutual",
    name: "Mutual friends (set intersection)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Model each person's friends as a set; the mutual friends are the members
  ; BOTH sets share. Set.of dedups, Set.intersection keeps the shared members,
  ; Set.to-list renders them in sorted order.
  (def (mutual a b)
    (Set.to-list (Set.intersection (Set.of a) (Set.of b))))
  (def (main)
    (mutual #list(1 2 3 4 5) #list(3 4 5 6 7)))
  (export main))`,
    expected: "(: #list(3 4 5) (List Int64))",
  },
  {
    // Shows off: a sum type as a functional STACK (cons list) with real push/pop via match, driving a
    // stack machine. Evaluates RPN (postfix): numbers push; an operator pops two and pushes the result.
    // "3 4 + 5 *" = (3+4)*5 = 35.
    id: "rpn-calculator",
    name: "RPN calculator (stack machine)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; A stack is a cons list — push and pop are just constructors and match.
  (type Stack (Empty unit) (Cons (Tuple Int64 Stack)))
  (def (push s x) (Cons #tuple(x s)))
  ; A postfix token: a number, or a binary operator.
  (type Tok (Num Int64) (Plus unit) (Times unit))
  ; An operator pops the top two operands and pushes the result. A well-formed postfix expression always
  ; has two operands on the stack for a binary op, so underflow is an invariant violation — trap, not a no-op.
  (def (apply2 s f)
    (match s
      ((Cons top1)
       (match (. top1 1)
         ((Cons top2) (push (. top2 1) (f (. top2 0) (. top1 0))))
         ((Empty _) (trap "rpn: operator underflow (need two operands)"))))
      ((Empty _) (trap "rpn: operator underflow (empty stack)"))))
  (def (step s tok)
    (match tok
      ((Num n) (push s n))
      ((Plus _) (apply2 s +))
      ((Times _) (apply2 s *))))
  (def (run toks i n s)
    (if (= i n)
        s
        (match (List.at toks i)
          ((Some t) (run toks (+ i 1) n (step s t)))
          ((None) s))))
  ; A completed RPN run leaves exactly one result on the stack; an empty stack here means a malformed
  ; expression, so trap rather than fabricate a 0.
  (def (top s) (match s ((Cons t) (. t 0)) ((Empty _) (trap "rpn: empty result stack"))))
  ; 3 4 + 5 *  ==>  (3 + 4) * 5 = 35
  (def (main)
    (let ((toks #list((Num 3) (Num 4) (Plus unit) (Num 5) (Times unit))))
      (top (run toks 0 (List.len toks) (Empty unit)))))
  (export main))`,
    expected: "35",
  },
  {
    // Shows off: nested recursion for a real algorithm — count primes ≤ N by trial division.
    // isprime tests divisibility up to √n; count folds over the range. Primes ≤ 100 = 25.
    id: "count-primes",
    name: "Count primes (trial division)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Trial division: n is prime if no d with d*d <= n divides it.
  (def (isprime-from d n)
    (if (> (* d d) n)
        true
        (if (= (% n d) 0) false (isprime-from (+ d 1) n))))
  (def (isprime n) (if (< n 2) false (isprime-from 2 n)))
  ; Count the primes in 2..=n.
  (def (count k n acc)
    (if (> k n)
        acc
        (count (+ k 1) n (if (isprime k) (+ acc 1) acc))))
  (def (main) (count 2 100 0))
  (export main))`,
    expected: "25",
  },
  {
    // Shows off: a cellular automaton — Rule 110, one of the simplest Turing-complete systems — as a
    // list transformation. Each generation maps a row of 0/1 cells to the next via a neighbourhood
    // rule. Returns the row after 4 generations of a single seed cell.
    id: "rule-110",
    name: "Rule 110 cellular automaton",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Elementary cellular automaton Rule 110 — a row of 0/1 cells, each new cell a
  ; function of its left/center/right neighbours. Famously Turing-complete.
  (def (cell xs i) (match (List.at xs i) ((Some v) v) ((None) 0)))
  (def (rule l c r)
    ; The output bit for each neighbourhood, indexed by (l*4 + c*2 + r).
    (match (+ (+ (* l 4) (* c 2)) r)
      (0 0) (1 1) (2 1) (3 1) (4 0) (5 1) (6 1) (7 0) (_ 0)))
  (def (step-from xs i n acc)
    (if (= i n)
        acc
        (step-from xs (+ i 1) n
          (List.push acc (rule (cell xs (- i 1)) (cell xs i) (cell xs (+ i 1)))))))
  (def (step xs) (step-from xs 0 (List.len xs) #list()))
  (def (gens xs k) (if (= k 0) xs (gens (step xs) (- k 1))))
  (def (main) (gens #list(0 0 0 0 0 0 0 1) 4))
  (export main))`,
    expected: "(: #list(0 0 0 1 1 1 1 1) (List Int64))",
  },
  {
    // Shows off: records + a list of them = a little data pipeline, returning BOTH the data and the
    // result. Project the age field of each (name, age) record into a list, and compute their average,
    // returning the pair so you SEE both the extracted data [36,41,40] AND the aggregate 39.
    id: "data-pipeline",
    name: "Data pipeline over records",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A list of records — access a field with (. r age).
  (def (age r) (. r age))
  ; Project every record's age into a list.
  (def (ages xs i n acc)
    (if (= i n)
        acc
        (match (List.at xs i)
          ((Some r) (ages xs (+ i 1) n (List.push acc (age r))))
          ((None) acc))))
  ; Sum the ages (for the average).
  (def (sum-age xs i n acc)
    (if (= i n)
        acc
        (match (List.at xs i)
          ((Some r) (sum-age xs (+ i 1) n (+ acc (age r))))
          ((None) acc))))
  (def (main)
    (let ((people #list(#record((= name "Ada") (= age 36))
                        #record((= name "Alan") (= age 41))
                        #record((= name "Grace") (= age 40)))))
      ; Return (the projected ages, their average) — both the data and the result.
      #tuple((ages people 0 (List.len people) (: #list() (List Int64)))
             (/ (sum-age people 0 (List.len people) 0) (List.len people)))))
  (export main))`,
    expected: "(: #tuple(#list(36 41 40) 39) (Tuple (List Int64) Int64))",
  },
  {
    // Shows off: 2D data — a matrix as a list of rows (list of lists), and building a NEW nested
    // collection at runtime. Transpose turns rows into columns: [[1 2 3] [4 5 6]] becomes
    // [[1 4] [2 5] [3 6]]. The result is a runtime-built (List (List Int64)) that renders in full.
    id: "matrix-transpose",
    name: "Matrix transpose",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; m is a list of rows; each row is a list of Int64. elem reads m[i][j]; i and j are always in range
  ; (0..rows, 0..cols), so a miss is an invariant violation — trap, never a magic 0 masquerading as data.
  (def (elem (: m (List (List Int64))) (: i Int64) (: j Int64))
    (match (List.at m i)
      ((Some r) (match (List.at r j) ((Some v) v) ((None) (trap "transpose: column index out of range"))))
      ((None) (trap "transpose: row index out of range"))))
  ; Build column j by reading m[0][j], m[1][j], ... down the rows.
  (def (col m j i rows acc)
    (if (= i rows) acc (col m j (+ i 1) rows (List.push acc (elem m i j)))))
  ; For each column index, push the built column onto the result — that's the transpose.
  (def (go m j cols rows acc)
    (if (= j cols)
        acc
        (go m (+ j 1) cols rows
          (List.push acc (col m j 0 rows (: #list() (List Int64)))))))
  (def (main)
    (let ((m #list(#list(1 2 3) #list(4 5 6))))
      (go m 0 3 2 (: #list() (List (List Int64))))))
  (export main))`,
    expected: "(: #list(#list(1 4) #list(2 5) #list(3 6)) (List (List Int64)))",
  },
  {
    // Shows off: exponential recursion made concrete — the minimum moves to solve Tower of Hanoi with
    // n disks is 2^n - 1 (move n-1, move the big disk, move n-1 back). hanoi(10) = 1023.
    id: "tower-of-hanoi",
    name: "Tower of Hanoi (move count)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Moves to solve Tower of Hanoi: solve n-1, move the largest disk, solve n-1 again.
  ; This is exactly 2^n - 1.
  (def (hanoi n)
    (if (= n 0)
        0
        (+ (+ (hanoi (- n 1)) 1) (hanoi (- n 1)))))
  (def (main) (hanoi 10))
  (export main))`,
    expected: "1023",
  },
  {
    // Shows off: bit-twiddling with plain integer arithmetic — count the 1-bits (population count) of a
    // number by repeatedly taking n mod 2 and dividing by 2. 2730 = 0b101010101010 has 6 one-bits.
    id: "population-count",
    name: "Population count (bits)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Count the 1-bits of n: the low bit is n mod 2; shift right by dividing by 2.
  (def (popcount n acc)
    (if (= n 0)
        acc
        (popcount (/ n 2) (+ acc (% n 2)))))
  (def (main) (popcount 2730 0))
  (export main))`,
    expected: "6",
  },
  {
    // Shows off: a classic list transform — run-length encoding. Walk the list tracking the current
    // run; emit a (value, count) tuple when it ends. [1 1 1 2 3 3] => [(1,3) (2,1) (3,2)]. The empty
    // accumulator is annotated so its element type is fixed before any element is pushed.
    id: "run-length-encoding",
    name: "Run-length encoding",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; The walk only reads in-range indices, so a miss is an invariant violation, not a real element.
  (def (at (: xs (List Int64)) (: i Int64))
    (match (List.at xs i) ((Some v) v) ((None) (trap "rle: index out of range"))))
  ; Walk the list carrying the current value + its running count; emit on a change.
  (def (go xs i n cur cnt acc)
    (if (= i n)
        (List.push acc #tuple(cur cnt))
        (let ((x (at xs i)))
          (if (= x cur)
              (go xs (+ i 1) n cur (+ cnt 1) acc)
              (go xs (+ i 1) n x 1 (List.push acc #tuple(cur cnt)))))))
  (def (main)
    (let ((xs #list(1 1 1 2 3 3)))
      ; Annotate the empty seed so its element type is known before the first push.
      (go xs 1 (List.len xs) (at xs 0) 1 (: #list() (List (Tuple Int64 Int64))))))
  (export main))`,
    expected: "(: #list(#tuple(1 3) #tuple(2 1) #tuple(3 2)) (List (Tuple Int64 Int64)))",
  },
  {
    // Shows off: quicksort as a divide-and-conquer recursion. Pivot on the head, partition the rest
    // into elements below and at-or-above the pivot, sort each side, and concat lows ++ [pivot] ++ highs.
    // [5 3 8 1 9 2 7 4 6] => [1 2 3 4 5 6 7 8 9]. Returns the sorted LIST, not just a count.
    id: "quicksort",
    name: "Quicksort",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Bounds-checked read: qsort only reads in-range indices, so a miss is an invariant violation.
  (def (at (: xs (List Int64)) (: i Int64))
    (match (List.at xs i) ((Some v) v) ((None) (trap "qsort: index out of range"))))
  ; Partition xs[i..n) into (lows, highs) relative to pivot: < pivot goes low, >= goes high.
  (def (part xs i n pivot lows highs)
    (if (= i n)
        #tuple(lows highs)
        (let ((x (at xs i)))
          (if (< x pivot)
              (part xs (+ i 1) n pivot (List.push lows x) highs)
              (part xs (+ i 1) n pivot lows (List.push highs x))))))
  ; Empty/singleton is already sorted; else pivot on the head and recurse on each side.
  (def (qsort (: xs (List Int64)))
    (if (< (List.len xs) 2)
        xs
        (let ((pivot (at xs 0)))
          (match (part xs 1 (List.len xs) pivot (: #list() (List Int64)) (: #list() (List Int64)))
            (#tuple(lows highs)
             (List.concat (List.concat (qsort lows) #list(pivot)) (qsort highs)))))))
  (def (main)
    (qsort #list(5 3 8 1 9 2 7 4 6)))
  (export main))`,
    expected: "(: #list(1 2 3 4 5 6 7 8 9) (List Int64))",
  },
  {
    // Shows off: binary search over a SORTED list, halving the range each step. The result is an honest
    // (Some index) when found and (None) when absent — never a -1 "not found" sentinel flowing in-band.
    // Find 11 => (Some 5); find 8 (absent) => (None). The pair shows both outcomes side by side.
    id: "binary-search",
    name: "Binary search",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Bounds-checked read: bsearch only probes in-range indices, so a miss is an invariant violation.
  (def (at (: xs (List Int64)) (: i Int64))
    (match (List.at xs i) ((Some v) v) ((None) (trap "bsearch: index out of range"))))
  ; Binary search over a SORTED list. Returns (Some index) if found, (None) if absent —
  ; an honest "not found", never a -1 sentinel flowing in-band.
  (def (go xs target lo hi)
    (if (> lo hi)
        (: (None) (Option Int64))
        (let ((mid (/ (+ lo hi) 2))
              (v (at xs mid)))
          (if (= v target)
              (Some mid)
              (if (< v target)
                  (go xs target (+ mid 1) hi)
                  (go xs target lo (- mid 1)))))))
  (def (bsearch (: xs (List Int64)) (: target Int64))
    (go xs target 0 (- (List.len xs) 1)))
  (def (main)
    ; A sorted list; find 11 -> (Some 5), then confirm 8 is absent -> (None).
    (let ((xs #list(1 3 5 7 9 11 13 15 17 19)))
      #tuple((bsearch xs 11) (bsearch xs 8))))
  (export main))`,
    expected: "(: #tuple((Some 5) (None unit)) (Tuple (Option Int64) (Option Int64)))",
  },
  {
    // Shows off: a binary search TREE — a recursive, branching sum type (distinct from the cons-list
    // shape used elsewhere). Insert keeps the ordering invariant; an in-order traversal then reads the
    // values back in sorted order, so the tree sorts for free. [5 3 8 1 4 7 9 2 6] => [1..9].
    id: "binary-search-tree",
    name: "Binary search tree",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A binary search tree: a recursive, branching sum type (distinct from the cons-LIST shape).
  ; Leaf is the empty tree; Node carries a value plus its left and right subtrees.
  (type Tree (Leaf Unit) (Node (Tuple Int64 Tree Tree)))
  ; Insert keeps the ordering invariant: smaller keys go left, larger go right, dup ignored.
  (def (insert (: t Tree) (: x Int64))
    (match t
      ((Leaf _u) (Node #tuple(x (Leaf unit) (Leaf unit))))
      ((Node nd)
       (match nd (#tuple(v l r)
         (if (< x v)
             (Node #tuple(v (insert l x) r))
             (if (> x v)
                 (Node #tuple(v l (insert r x)))
                 (Node #tuple(v l r)))))))))
  ; In-order traversal yields the values in SORTED order — the tree sorts for free.
  (def (inorder (: t Tree))
    (match t
      ((Leaf _u) (: #list() (List Int64)))
      ((Node nd)
       (match nd (#tuple(v l r)
         (List.concat (List.push (inorder l) v) (inorder r)))))))
  (def (build (: xs (List Int64)) (: i Int64) (: t Tree))
    (if (= i (List.len xs))
        t
        (match (List.at xs i)
          ((Some x) (build xs (+ i 1) (insert t x)))
          ((None) (trap "build: index out of range")))))
  (def (main)
    (let ((xs #list(5 3 8 1 4 7 9 2 6)))
      (inorder (build xs 0 (Leaf unit)))))
  (export main))`,
    expected: "(: #list(1 2 3 4 5 6 7 8 9) (List Int64))",
  },
  {
    // Shows off: string work via bytes — check a word reads the same forwards and backwards by comparing
    // the i-th and (n-1-i)-th UTF-8 bytes moving inward. "racecar" -> 1 (true).
    id: "palindrome-check",
    name: "Palindrome check",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Compare bytes from both ends moving inward; a mismatch means not a palindrome.
  ; i and j stay within [0, len), so a miss is an invariant violation — trap, don't invent a byte.
  (def (at bs i) (match (Bytes.at bs i) ((Some b) b) ((None) (trap "palindrome: byte index out of range"))))
  (def (pal bs i j)
    (if (>= i j)
        true
        (if (= (at bs i) (at bs j)) (pal bs (+ i 1) (- j 1)) false)))
  (def (main)
    (let ((bs (String.to-bytes "racecar")))
      (pal bs 0 (- (Bytes.len bs) 1))))
  (export main))`,
    expected: "true",
  },
  {
    // Shows off: scanning a string's UTF-8 bytes in a loop — walk each byte and count the ASCII vowels.
    // `Bytes.at` returns an Option (past-the-end is None), so we match it. "education" -> 5.
    id: "count-vowels",
    name: "Count vowels",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Is this byte one of a e i o u (lowercase ASCII)?
  (def (is-vowel b)
    (if (= b 97) true
    (if (= b 101) true
    (if (= b 105) true
    (if (= b 111) true
    (if (= b 117) true false))))))
  ; Bytes.at returns (Option Int64); i stays within [0, len), so a miss is an invariant violation — trap.
  (def (byte-at bs i) (match (Bytes.at bs i) ((Some b) b) ((None) (trap "count-vowels: byte index out of range"))))
  ; Walk every byte, counting vowels.
  (def (go bs i n acc)
    (if (= i n)
        acc
        (go bs (+ i 1) n (if (is-vowel (byte-at bs i)) (+ acc 1) acc))))
  (def (count-vowels s)
    (let ((bs (String.to-bytes s)))
      (go bs 0 (Bytes.len bs) 0)))
  (def (main) (count-vowels "education"))
  (export main))`,
    expected: "5",
  },
  {
    // Shows off: `Char.to-int` / `Char.from-int` — a char IS its Unicode scalar. A Caesar cipher shifts a
    // letter by n within A..Z: read the code point, shift modulo 26 off 'A' (65), rebuild the letter
    // (Char.from-int returns Option Char — None on an invalid code point). 'A'+3='D'; 'Y'+3 wraps to 'B'.
    id: "caesar-cipher-char",
    name: "Caesar cipher (char arithmetic)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; A char IS its Unicode scalar. Char.to-int reads the code point; Char.from-int
  ; rebuilds a char (Option Char — None on an invalid code point). Shift an uppercase
  ; letter by n, wrapping A..Z: work in [0,26) off 'A' (65), then add 65 back.
  (def (letter s) (match (String.scalar-at s 0) ((Some c) c) ((None) (trap "empty string"))))
  (def (shift c n)
    (let ((code (Char.to-int c)))
      (match (Char.from-int (+ 65 (% (+ (- code 65) n) 26)))
        ((Some r) r)
        ((None) (trap "caesar: bad code point")))))
  (def (main)
    ; 'A' shifted by 3 -> 'D'; 'Y' shifted by 3 wraps around -> 'B'.
    #tuple((shift (letter "A") 3) (shift (letter "Y") 3)))
  (export main))`,
    expected: "(: #tuple(#\\D #\\B) (Tuple Char Char))",
  },
  {
    // Shows off: Unicode NFC normalization at string CONSTRUCTION. Concatenating a plain "e" with a lone
    // U+0301 COMBINING ACUTE ACCENT composes them into the single precomposed scalar "é" (U+00E9), so the
    // result reads as 1 scalar / 2 UTF-8 bytes — not 2 scalars / 3 bytes. Strings are normalized as they're built.
    id: "unicode-nfc-normalization",
    name: "Unicode NFC normalization",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A lone U+0301 COMBINING ACUTE ACCENT, decoded from its two UTF-8 bytes.
  (def (accent) (String.from-bytes (Bytes.of #list(204 129))))
  (def (main)
    (match (accent)
      ((Some acc)
       ; Concatenation constructs a new string, so the join is normalized to NFC:
       ; "e" + combining-accent composes into the single precomposed scalar "é".
       (let ((composed (String.concat "e" acc)))
         ; (scalar count, byte count) of the composed "é" — 1 scalar, 2 bytes.
         #tuple((String.scalar-len composed) (String.byte-len composed))))
      ((None) (trap "nfc: invalid accent bytes"))))
  (export main))`,
    expected: "(: (tuple 1 2) (Tuple Int64 Int64))",
  },
  {
    // Shows off: `String.slice` — a half-open [start, end) substring by SCALAR index, returning
    // `Option String` (None when the range is out of bounds). Here we pull the fixed-width fields out
    // of an ISO date "YYYY-MM-DD", defaulting a bad slice to "" — a tiny, real parsing task.
    id: "string-slice-date-fields",
    name: "Substring slicing (ISO date fields)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; Pull the fixed-width fields out of an ISO date "YYYY-MM-DD" by scalar range.
  ; String.slice takes a half-open [start, end) scalar range and returns Option
  ; String (None if the range is out of bounds) — so we default a bad slice to "".
  (def (field s lo hi)
    (match (String.slice s lo hi) ((Some part) part) ((None) "")))
  (def (main)
    (let ((d "2026-08-14"))
      #tuple((field d 0 4) (field d 5 7) (field d 8 10))))
  (export main))`,
    expected: "(: #tuple(\"2026\" \"08\" \"14\") (Tuple String String String))",
  },
  {
    // Shows off: computing a result without a built-in — integer square root by searching upward for the
    // largest g with g*g <= n. isqrt(144) = 12.
    id: "integer-square-root",
    name: "Integer square root",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; The largest g such that g*g <= n: search up until g*g overshoots, then back off one.
  (def (isqrt-from n g)
    (if (> (* g g) n) (- g 1) (isqrt-from n (+ g 1))))
  (def (isqrt n) (isqrt-from n 1))
  (def (main) (isqrt 144))
  (export main))`,
    expected: "12",
  },
  {
    // Shows off: MUTUAL recursion — two top-level defs each call the other, and Cadenza resolves the
    // forward reference across definitions (a whole recursion group, not just self-recursion). Classic
    // even/odd parity, then count how many of 0..<10 are even -> 5.
    id: "mutual-recursion",
    name: "Mutual recursion (even & odd)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; Each predicate is defined in terms of the OTHER — a mutually recursive group.
  ; Cadenza resolves the forward reference to is-odd before it is defined below.
  (def (is-even n) (if (= n 0) true (is-odd (- n 1))))
  (def (is-odd n) (if (= n 0) false (is-even (- n 1))))
  ; Count how many of 0..<n are even, using is-even.
  (def (count-evens n i acc)
    (if (= i n)
        acc
        (count-evens n (+ i 1) (if (is-even i) (+ acc 1) acc))))
  (def (main) (count-evens 10 0 0))
  (export main))`,
    expected: "5",
  },
  {
    // Shows off: NON-primitive recursion — the Ackermann–Péter function, whose recursion nests inside
    // its own argument (ack(m-1, ack(m, n-1))). It's total but not primitive-recursive, and grows
    // explosively, so keep the inputs tiny: ack(3, 3) = 61.
    id: "ackermann",
    name: "Ackermann function",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; ack(m,n): the classic total-but-not-primitive-recursive function. The inner call
  ; ack(m, n-1) is itself an argument to the outer ack — recursion nested in recursion.
  (def (ack m n)
    (if (= m 0)
        (+ n 1)
        (if (= n 0)
            (ack (- m 1) 1)
            (ack (- m 1) (ack m (- n 1))))))
  (def (main) (ack 3 3))
  (export main))`,
    expected: "61",
  },
  {
    // Shows off: backtracking search — count the solutions to the N-queens puzzle. A partial placement
    // is a list of column positions (one per row placed so far); `safe` rejects a column that shares a
    // column or diagonal with an earlier queen; solve/try-cols recurse row by row, summing the counts.
    // The classic 8x8 board has 92 solutions.
    id: "n-queens",
    name: "N-queens (count solutions)",
    theme: "algorithms",
    surface: "sexpr",
    source: `(do
  ; placed positions are always in range (i < row <= length), so a miss is an invariant violation, not data.
  (def (at (: xs (List Int64)) (: i Int64))
    (match (List.at xs i) ((Some v) v) ((None) (trap "queens: placed[i] out of range"))))
  (def (adiff a b) (if (> a b) (- a b) (- b a)))
  ; Is this column safe in the current row, given the columns already placed above?
  (def (safe placed col row i)
    (if (= i row)
        true
        (let ((pc (at placed i)))
          (if (= pc col) false
          (if (= (adiff pc col) (adiff i row)) false
          (safe placed col row (+ i 1)))))))
  ; Try every column in the current row; sum the solution counts.
  (def (try-cols n placed row col acc)
    (if (= col n)
        acc
        (try-cols n placed row (+ col 1)
          (if (safe placed col row 0)
              (+ acc (solve n (List.push placed col) (+ row 1)))
              acc))))
  (def (solve n placed row)
    (if (= row n) 1 (try-cols n placed row 0 0)))
  (def (main) (solve 8 (: #list() (List Int64)) 0))
  (export main))`,
    expected: "92",
  },
  {
    // Shows off: rebuilding a list in reverse by pushing elements from the end onto a fresh list.
    // The prelude List has no reverse, so we walk indices downward. [1 2 3 4 5] -> [5 4 3 2 1].
    id: "reverse-a-list",
    name: "Reverse a list",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; rev only reads indices in [0, len), so a miss is an invariant violation — trap, don't invent an element.
  (def (at (: xs (List Int64)) (: i Int64))
    (match (List.at xs i) ((Some v) v) ((None) (trap "reverse: index out of range"))))
  ; Walk from the last index down, pushing each element onto a fresh list.
  (def (rev xs i acc)
    (if (< i 0) acc (rev xs (- i 1) (List.push acc (at xs i)))))
  (def (main)
    (let ((xs #list(1 2 3 4 5)))
      (rev xs (- (List.len xs) 1) (: #list() (List Int64)))))
  (export main))`,
    expected: "(: #list(5 4 3 2 1) (List Int64))",
  },
  {
    // Shows off: `List.prepend` — the O(1) front insert (receiver-first: `(List.prepend xs x)` puts x on
    // the head). A stack keeps its TOP at the front, so a push IS a prepend — the natural counterpart to
    // `reverse-a-list`'s `List.push`, which appends to the END. Here we push 10, 20, 30, so 30 lands on top.
    id: "stack-via-prepend",
    name: "Stack (push via prepend)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; A stack keeps its TOP at the FRONT. \`List.prepend\` puts an element on the
  ; head in O(1) — \`(List.prepend stack x)\` — so a push is just a prepend.
  ; (Contrast \`List.push\`, which appends to the END; see "Reverse a list".)
  (def (push stack x)
    (List.prepend stack x))
  (def (main)
    ; Push 10, then 20, then 30 — so 30 ends up on top of the stack.
    (push (push (push (: #list() (List Int64)) 10) 20) 30))
  (export main))`,
    expected: "(: #list(30 20 10) (List Int64))",
  },
  {
    // Shows off: metaprogramming — code is DATA. `quasiquote` builds an AST value without running it,
    // `unquote` splices a computed value into that AST, and `eval` runs a constructed program. The
    // result SHOWS both halves: the generated syntax tree for (base*base + offset), and the value
    // `eval` computes from it — (Ast, 41) — so you can see the code-as-data, not just trust the number.
    id: "metaprogramming-quote-eval",
    name: "Metaprogramming (quote & eval)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; \`build\` constructs the expression (base*base + offset) as DATA — a syntax
  ; tree, without running it. \`unquote\` splices live values into that tree.
  (def (build base offset)
    (quasiquote (+ (* (unquote base) (unquote base)) (unquote offset))))
  (def (main)
    ; Show BOTH: the generated syntax tree, AND the value eval computes from it.
    ; (eval runs a compile-time-visible quasiquote, so the runnable copy is inline.)
    #tuple(
      (build 6 5)
      (eval (quasiquote (+ (* (unquote 6) (unquote 6)) (unquote 5))))))
  (export main))`,
    expected: "(: #tuple(((. Ast List) #list(((. Ast Name) \"+\") ((. Ast List) #list(((. Ast Name) \"*\") ((. Ast Int) 6) ((. Ast Int) 6))) ((. Ast Int) 5))) 41) (Tuple Ast Int64))",
  },
  {
    // Shows off: algebraic effects & handlers — a signature Cadenza feature — discharged entirely
    // in-program. `Tick.tick n` performs an operation; the handler threads a running total as its
    // STATE, hands back the new total, and resumes. tick 10 -> 10, tick 5 -> 15, so 10 + 15 = 25.
    id: "effects-handlers-stateful",
    name: "Effects & handlers (stateful)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  ; Declare an effect, then discharge it with a handler that threads STATE.
  (effect Tick (op tick (-> Int64 Int64)))
  (def (main)
    ; handler seeds state 0; each tick adds n to the state and returns the new total.
    (handle Tick 0
      ((tick (n) s (resume (+ s n) (+ s n))))
      (+ (Tick.tick 10) (Tick.tick 5))))
  (export main))`,
    expected: "25",
  },
  {
    // Shows off: composing lazy-style transformers into a pipeline. A hand-rolled linked sequence
    // (Iter) with filter/map/fold; each stage recurses down the sequence and rebuilds it. The pipeline
    // keeps the evens of 1..6 -> [2,4,6], triples them -> [6,12,18], and sums -> 36.
    // (Authored with v-iterators, who own the iterator family.)
    id: "iterator-pipeline",
    name: "Iterator pipeline (filter → map → fold)",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A linked sequence of Int64: empty (Nil) or a head paired with the rest.
  (type Iter (Nil unit) (Cons (Tuple Int64 Iter)))
  (def (from-list xs)
    (match xs
      (#list() (Nil unit))
      (#list(h .. t) (Cons #tuple(h (from-list t))))))
  ; Keep elements satisfying p.
  (def (ifilter it p)
    (match it
      ((Nil _) (Nil unit))
      ((Cons c) (if (p (. c 0))
                    (Cons #tuple((. c 0) (ifilter (. c 1) p)))
                    (ifilter (. c 1) p)))))
  ; Apply f to every element.
  (def (imap it f)
    (match it
      ((Nil _) (Nil unit))
      ((Cons c) (Cons #tuple((f (. c 0)) (imap (. c 1) f))))))
  ; Reduce left-to-right with f, starting from acc.
  (def (ifold it acc f)
    (match it
      ((Nil _) acc)
      ((Cons c) (ifold (. c 1) (f acc (. c 0)) f))))
  ; evens of 1..6 -> triple -> sum = 36
  (def (main)
    (ifold (imap (ifilter (from-list #list(1 2 3 4 5 6)) (fn (x) (= 0 (% x 2))))
                 (fn (x) (* x 3)))
           0
           (fn (a x) (+ a x))))
  (export main))`,
    expected: "36",
  },
  {
    // Shows off: consuming TWO sequences in lockstep — a zip. Walk both Iters together, multiply the
    // paired heads, and sum, giving the dot product of two vectors. [1,2,3]·[4,5,6] = 4+10+18 = 32.
    // (Authored with v-iterators; monomorphic Int64 Iter, clean on both surfaces.)
    id: "iterator-zip-dot-product",
    name: "Iterator zip: dot product",
    theme: "data-and-collections",
    surface: "sexpr",
    source: `(do
  ; A linked sequence of Int64: empty (Nil) or a head paired with the rest.
  (type Iter (Nil unit) (Cons (Tuple Int64 Iter)))
  (def (from-list xs)
    (match xs
      (#list() (Nil unit))
      (#list(h .. t) (Cons #tuple(h (from-list t))))))
  ; Walk a and b in lockstep: sum head-a * head-b, recurse on both tails.
  ; Stops at the shorter (either Nil ends it).
  (def (dot a b)
    (match a
      ((Nil _) 0)
      ((Cons pa)
       (match pa
         (#tuple(ha ra)
          (match b
            ((Nil _) 0)
            ((Cons pb)
             (match pb
               (#tuple(hb rb) (+ (* ha hb) (dot ra rb)))))))))))
  ; [1,2,3] · [4,5,6] = 1*4 + 2*5 + 3*6 = 32
  (def (main) (dot (from-list #list(1 2 3)) (from-list #list(4 5 6))))
  (export main))`,
    expected: "32",
  },
  {
    id: "a-type-error",
    name: "A type error (see the squiggle)",
    theme: "basics",
    surface: "sexpr",
    source: `(do
  (def (main) (+ 1 true))
  (export main))`,
    expectError: true,
  },
];

export const DEFAULT_EXAMPLE = EXAMPLES[0];
