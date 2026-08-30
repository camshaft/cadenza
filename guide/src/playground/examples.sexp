(playground
  (example
    (id "hello-arithmetic")
    (name "Hello, arithmetic")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (def (main) (+ 2 3))
  (export main)))
    (expected "5"))
  (example
    (id "recursive-sum")
    (name "A recursive sum")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (def (sm n)
    (if (= n 0) 0 (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main)))
    (expected "15"))
  (example
    (id "pattern-matching")
    (name "Pattern matching")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (def (main)
    (match 2
      (1 10)
      (2 20)
      (_ 0)))
  (export main)))
    (expected "20"))
  (example
    (id "option-sum-types")
    (name "Option & sum types")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (type Opt (Some Int64) (None unit))
  (def (main)
    (match (Some 7)
      ((Some x) x)
      ((None _) 0)))
  (export main)))
    (expected "7"))
  (example
    (id "records")
    (name "Records")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (def (area r) (* (. r w) (. r h)))
  (def (main) (area #record((= w 4) (= h 5))))
  (export main)))
    (expected "20"))
  (example
    (id "record-patterns")
    (name "Record patterns (match on fields)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; Match destructures the record's fields; a literal field value guards the origin case.
  (def (score p)
    (match p
      (#record((= x 0) (= y 0)) 0)
      (#record((= x xv) (= y yv)) (+ (* xv xv) (* yv yv)))))
  (def (main) (score #record((= x 3) (= y 4))))
  (export main)))
    (expected "25"))
  (example
    (id "nested-record-patterns")
    (name "Nested record patterns (destructure in place)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; A match arm destructures a record nested inside a compound pattern — the tuple's first
  ; slot is a point, and (x px) (y py) bind its fields directly; w binds the weight.
  (def (weigh pair)
    (match pair
      (#tuple(#record((= x px) (= y py)) w) (* w (+ (* px px) (* py py))))))
  (def (main) (weigh #tuple(#record((= x 3) (= y 4)) 2)))
  (export main)))
    (expected "50"))
  (example
    (id "variant-nested-record-patterns")
    (name "Variant-nested record patterns (match through a variant)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; A match arm reaches through a variant into its record payload: (Some (record ...)) binds
  ; the point's fields directly; the None arm handles the empty case.
  (def (dist opt)
    (match opt
      ((Some #record((= x px) (= y py))) (+ (* px px) (* py py)))
      ((None) 0)))
  (def (main) (dist (Some #record((= x 3) (= y 4)))))
  (export main)))
    (expected "25"))
  (example
    (id "tuple")
    (name "A tuple")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (def (main) #tuple(1 2 3))
  (export main)))
    (expected "(: #tuple(1 2 3) (Tuple Int64 Int64 Int64))"))
  (example
    (id "tuple-split-concat")
    (name "Splitting & joining tuples")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; Tuples have STRUCTURAL, compile-time-known arity. Tuple.split-at cuts a tuple
  ; into a (head, tail) pair of tuples at a fixed index; Tuple.concat is its inverse,
  ; welding two tuples back into one. Split (1 2 3 4 5) at 2, then rebuild it.
  (def (main)
    (let ((parts (Tuple.split-at #tuple(1 2 3 4 5) 2)))
      (match parts
        (#tuple(head tail)
         #tuple(head tail (Tuple.concat head tail))))))
  (export main)))
    (expected "(: #tuple(#tuple(1 2) #tuple(3 4 5) #tuple(1 2 3 4 5)) (Tuple (Tuple Int64 Int64) (Tuple Int64 Int64 Int64) (Tuple Int64 Int64 Int64 Int64 Int64)))"))
  (example
    (id "lists")
    (name "Lists")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; Concatenate two lists and return the RESULT — you see the whole list, not just its length.
  (def (main)
    (List.concat #list(1 2) #list(3 4 5)))
  (export main)))
    (expected "(: #list(1 2 3 4 5) (List Int64))"))
  (example
    (id "list-update-functional")
    (name "Functional list update")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; List.update xs i v : return a NEW list with index i replaced by v (a functional
  ; update — the original list is unchanged). Indices must be in [0, len).
  (def (set-at xs i v) (List.update xs i v))
  (def (main)
    (let ((xs #list(10 20 30 40 50)))
      ; Update index 0 and index 4 of xs; then show xs ITSELF is untouched.
      #tuple((set-at xs 0 99) (set-at xs 4 99) xs)))
  (export main)))
    (expected "(: #tuple(#list(99 20 30 40 50) #list(10 20 30 40 99) #list(10 20 30 40 50)) (Tuple (List Int64) (List Int64) (List Int64)))"))
  (example
    (id "expression-interpreter")
    (name "Expression interpreter")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "-20"))
  (example
    (id "collatz-orbit")
    (name "Collatz orbit length")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
  ; Count the steps for n to reach 1 under the Collatz map
  ; (n/2 if even, 3n+1 if odd). 27 is the famously long orbit.
  (def (collatz n steps)
    (if (= n 1)
        steps
        (if (= (% n 2) 0)
            (collatz (/ n 2) (+ steps 1))
            (collatz (+ (* 3 n) 1) (+ steps 1)))))
  (def (main) (collatz 27 0))
  (export main)))
    (expected "111"))
  (example
    (id "function-composition")
    (name "Function composition")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; compose builds a NEW function that runs g then f — a closure over both.
  (def (compose f g) (fn (x) (f (g x))))
  (def (double x) (* x 2))
  (def (inc x) (+ x 1))
  (def (main) ((compose double inc) 20))
  (export main)))
    (expected "42"))
  (example
    (id "higher-order-apply-n")
    (name "Higher-order functions (apply n times)")
    (theme "basics")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: (tuple 22 32) (Tuple Int64 Int64))"))
  (example
    (id "memoized-fibonacci")
    (name "Memoized Fibonacci (Map cache)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "832040"))
  (example
    (id "frequency-count")
    (name "Frequency count (fold into a Map)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #list(#tuple(1 2) #tuple(2 1) #tuple(3 3)) (List (Tuple Int64 Int64)))"))
  (example
    (id "map-swap-take-inventory")
    (name "Map swap & take (inventory)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #tuple((Some 5) (Some 2) #list(#tuple(1 9))) (Tuple (Option Int64) (Option Int64) (List (Tuple Int64 Int64))))"))
  (example
    (id "bytes-map-key")
    (name "Bytes as a Map key")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #tuple((Some 3) (Some 1)) (Tuple (Option Int64) (Option Int64)))"))
  (example
    (id "symbol-keyed-map")
    (name "Symbols as Map keys")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #tuple((Some 65280) (None unit) true) (Tuple (Option Int64) (Option Int64) Bool))"))
  (example
    (id "byte-string-literal")
    (name "Byte-string literal")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
  ; b"…" builds a Bytes value straight from the source text — no String.to-bytes step.
  ; "GIF89a" is the GIF magic-number header: 6 bytes, first byte 'G' (ASCII 71).
  (def (main)
    (let ((magic b"GIF89a"))
      (match (Bytes.at magic 0)
        ((Some first) #tuple((Bytes.len magic) first))
        ((None) (trap "byte-literal: unexpectedly empty")))))
  (export main)))
    (expected "(: #tuple(6 71) (Tuple Int64 Int64))"))
  (example
    (id "value-encode-canonical-bytes")
    (name "Canonical binary encoding (Value.encode)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
  ; Value.encode : forall a. a -> Bytes — the single canonical binary form of ANY
  ; value (total: every value has one). One generic helper encodes different shapes;
  ; we report the byte length of each value's canonical encoding.
  (def (size v) (Bytes.len (Value.encode v)))
  (def (main)
    #tuple(
      (size #record((= x 3) (= y 4)))
      (size (Some 7))
      (size #list(1 2 3))))
  (export main)))
    (expected "(: (tuple 102 61 66) (Tuple Int64 Int64 Int64))"))
  (example
    (id "value-encode-determinism")
    (name "Structural encoding is deterministic (Value.encode)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
  ; Value.encode is deterministic + structural: two structurally-equal values encode to
  ; byte-identical output (a content address). Bytes has a total order, so we compare the
  ; encodings directly with =. Equal records -> identical bytes; a differing field -> not.
  (def (main)
    (let ((ba (Value.encode #record((= x 3) (= y 4))))
          (bb (Value.encode #record((= x 3) (= y 4))))
          (bc (Value.encode #record((= x 3) (= y 5)))))
      #tuple((Bytes.len ba) (= ba bb) (= ba bc))))
  (export main)))
    (expected "(: (tuple 102 true false) (Tuple Int64 Bool Bool))"))
  (example
    (id "value-encode-roundtrip")
    (name "Encode / decode round-trip (Value)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: (tuple 73 7) (Tuple Int64 Int64))"))
  (example
    (id "exact-rational-arithmetic")
    (name "Exact rational arithmetic")
    (theme "numbers")
    (surface "sexpr")
    (source (do
  (pragma default-fraction Rational)
  ; Bare literals here are EXACT fractions, so this sums to exactly 1 — no float drift.
  (def (sum) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 6)))
  (def (main) (sum))
  (export main)))
    (expected "(: 1/1 Rational)"))
  (example
    (id "rational-parts")
    (name "Rational numerator & denominator")
    (theme "numbers")
    (surface "sexpr")
    (source (do
  (pragma default-fraction Rational)
  ; Exact sum: 1/2 + 1/3 + 1/12 = 11/12 (already in lowest terms).
  (def (total) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 12)))
  ; Pull the reduced fraction apart into its two BigInt components.
  (def (main)
    #tuple((Rational.numerator (total)) (Rational.denominator (total))))
  (export main)))
    (expected "(: #tuple(11 12) (Tuple BigInt BigInt))"))
  (example
    (id "rational-floor-ceil")
    (name "Rational floor & ceil")
    (theme "numbers")
    (surface "sexpr")
    (source (do
  (pragma default-fraction Rational)
  ; floor/ceil project an EXACT fraction to an Int64, rounding toward -inf / +inf.
  ; 7/2 = 3.5 -> floor 3, ceil 4.   -7/2 = -3.5 -> floor -4, ceil -3.
  (def (main)
    #tuple(
      (Rational.floor (/ 7 2))
      (Rational.ceil (/ 7 2))
      (Rational.floor (/ -7 2))
      (Rational.ceil (/ -7 2))))
  (export main)))
    (expected "(: #tuple(3 4 -4 -3) (Tuple Int64 Int64 Int64 Int64))"))
  (example
    (id "float-rounding-drift")
    (name "Float rounding drift")
    (theme "numbers")
    (surface "sexpr")
    (source (do
  ; Add 0.1 to itself ten times. Exact math says 1.0, but Float64 rounding accumulates.
  (def (add-tenths (: n Int64) (: acc Float64))
    (if (= n 0) acc (add-tenths (- n 1) (+ acc 0.1))))
  (def (main) (add-tenths 10 0.0))
  (export main)))
    (expected "0.9999999999999999"))
  (example
    (id "gcd-lcm-euclid")
    (name "GCD and LCM (Euclid)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
  ; Euclid's algorithm: gcd(a, b) = gcd(b, a mod b), until b is 0.
  (def (gcd a b) (if (= b 0) a (gcd b (% a b))))
  ; LCM falls out of GCD: lcm(a, b) = a * b / gcd(a, b).
  (def (lcm a b) (/ (* a b) (gcd a b)))
  (def (main) (lcm 12 18))
  (export main)))
    (expected "36"))
  (example
    (id "set-algebra")
    (name "Set algebra (symmetric difference)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
  ; The symmetric difference of two sets: elements in exactly one of them.
  ; (A minus B) union (B minus A). Sets also give you dedup for free.
  (def (sym-diff a b)
    (Set.union (Set.difference a b) (Set.difference b a)))
  (def (main)
    (let ((a (Set.of #list(1 2 3 4)))
          (b (Set.of #list(3 4 5 6))))
      (Set.to-list (sym-diff a b))))
  (export main)))
    (expected "(: #list(1 2 5 6) (List Int64))"))
  (example
    (id "set-intersection-mutual")
    (name "Mutual friends (set intersection)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
  ; Model each person's friends as a set; the mutual friends are the members
  ; BOTH sets share. Set.of dedups, Set.intersection keeps the shared members,
  ; Set.to-list renders them in sorted order.
  (def (mutual a b)
    (Set.to-list (Set.intersection (Set.of a) (Set.of b))))
  (def (main)
    (mutual #list(1 2 3 4 5) #list(3 4 5 6 7)))
  (export main)))
    (expected "(: #list(3 4 5) (List Int64))"))
  (example
    (id "rpn-calculator")
    (name "RPN calculator (stack machine)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "35"))
  (example
    (id "count-primes")
    (name "Count primes (trial division)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "25"))
  (example
    (id "rule-110")
    (name "Rule 110 cellular automaton")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #list(0 0 0 1 1 1 1 1) (List Int64))"))
  (example
    (id "data-pipeline")
    (name "Data pipeline over records")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #tuple(#list(36 41 40) 39) (Tuple (List Int64) Int64))"))
  (example
    (id "matrix-transpose")
    (name "Matrix transpose")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #list(#list(1 4) #list(2 5) #list(3 6)) (List (List Int64)))"))
  (example
    (id "tower-of-hanoi")
    (name "Tower of Hanoi (move count)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
  ; Moves to solve Tower of Hanoi: solve n-1, move the largest disk, solve n-1 again.
  ; This is exactly 2^n - 1.
  (def (hanoi n)
    (if (= n 0)
        0
        (+ (+ (hanoi (- n 1)) 1) (hanoi (- n 1)))))
  (def (main) (hanoi 10))
  (export main)))
    (expected "1023"))
  (example
    (id "population-count")
    (name "Population count (bits)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
  ; Count the 1-bits of n: the low bit is n mod 2; shift right by dividing by 2.
  (def (popcount n acc)
    (if (= n 0)
        acc
        (popcount (/ n 2) (+ acc (% n 2)))))
  (def (main) (popcount 2730 0))
  (export main)))
    (expected "6"))
  (example
    (id "run-length-encoding")
    (name "Run-length encoding")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #list(#tuple(1 3) #tuple(2 1) #tuple(3 2)) (List (Tuple Int64 Int64)))"))
  (example
    (id "quicksort")
    (name "Quicksort")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #list(1 2 3 4 5 6 7 8 9) (List Int64))"))
  (example
    (id "binary-search")
    (name "Binary search")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #tuple((Some 5) (None unit)) (Tuple (Option Int64) (Option Int64)))"))
  (example
    (id "binary-search-tree")
    (name "Binary search tree")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #list(1 2 3 4 5 6 7 8 9) (List Int64))"))
  (example
    (id "palindrome-check")
    (name "Palindrome check")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "true"))
  (example
    (id "count-vowels")
    (name "Count vowels")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "5"))
  (example
    (id "caesar-cipher-char")
    (name "Caesar cipher (char arithmetic)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: #tuple(#\\D #\\B) (Tuple Char Char))"))
  (example
    (id "unicode-nfc-normalization")
    (name "Unicode NFC normalization")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "(: (tuple 1 2) (Tuple Int64 Int64))"))
  (example
    (id "string-slice-date-fields")
    (name "Substring slicing (ISO date fields)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
  ; Pull the fixed-width fields out of an ISO date "YYYY-MM-DD" by scalar range.
  ; String.slice takes a half-open [start, end) scalar range and returns Option
  ; String (None if the range is out of bounds) — so we default a bad slice to "".
  (def (field s lo hi)
    (match (String.slice s lo hi) ((Some part) part) ((None) "")))
  (def (main)
    (let ((d "2026-08-14"))
      #tuple((field d 0 4) (field d 5 7) (field d 8 10))))
  (export main)))
    (expected "(: #tuple(\"2026\" \"08\" \"14\") (Tuple String String String))"))
  (example
    (id "integer-square-root")
    (name "Integer square root")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
  ; The largest g such that g*g <= n: search up until g*g overshoots, then back off one.
  (def (isqrt-from n g)
    (if (> (* g g) n) (- g 1) (isqrt-from n (+ g 1))))
  (def (isqrt n) (isqrt-from n 1))
  (def (main) (isqrt 144))
  (export main)))
    (expected "12"))
  (example
    (id "mutual-recursion")
    (name "Mutual recursion (even & odd)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "5"))
  (example
    (id "ackermann")
    (name "Ackermann function")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
  ; ack(m,n): the classic total-but-not-primitive-recursive function. The inner call
  ; ack(m, n-1) is itself an argument to the outer ack — recursion nested in recursion.
  (def (ack m n)
    (if (= m 0)
        (+ n 1)
        (if (= n 0)
            (ack (- m 1) 1)
            (ack (- m 1) (ack m (- n 1))))))
  (def (main) (ack 3 3))
  (export main)))
    (expected "61"))
  (example
    (id "n-queens")
    (name "N-queens (count solutions)")
    (theme "algorithms")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "92"))
  (example
    (id "reverse-a-list")
    (name "Reverse a list")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; rev only reads indices in [0, len), so a miss is an invariant violation — trap, don't invent an element.
  (def (at (: xs (List Int64)) (: i Int64))
    (match (List.at xs i) ((Some v) v) ((None) (trap "reverse: index out of range"))))
  ; Walk from the last index down, pushing each element onto a fresh list.
  (def (rev xs i acc)
    (if (< i 0) acc (rev xs (- i 1) (List.push acc (at xs i)))))
  (def (main)
    (let ((xs #list(1 2 3 4 5)))
      (rev xs (- (List.len xs) 1) (: #list() (List Int64)))))
  (export main)))
    (expected "(: #list(5 4 3 2 1) (List Int64))"))
  (example
    (id "stack-via-prepend")
    (name "Stack (push via prepend)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; A stack keeps its TOP at the FRONT. `List.prepend` puts an element on the
  ; head in O(1) — `(List.prepend stack x)` — so a push is just a prepend.
  ; (Contrast `List.push`, which appends to the END; see "Reverse a list".)
  (def (push stack x)
    (List.prepend stack x))
  (def (main)
    ; Push 10, then 20, then 30 — so 30 ends up on top of the stack.
    (push (push (push (: #list() (List Int64)) 10) 20) 30))
  (export main)))
    (expected "(: #list(30 20 10) (List Int64))"))
  (example
    (id "metaprogramming-quote-eval")
    (name "Metaprogramming (quote & eval)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; `build` constructs the expression (base*base + offset) as DATA — a syntax
  ; tree, without running it. `unquote` splices live values into that tree.
  (def (build base offset)
    (quasiquote (+ (* (unquote base) (unquote base)) (unquote offset))))
  (def (main)
    ; Show BOTH: the generated syntax tree, AND the value eval computes from it.
    ; (eval runs a compile-time-visible quasiquote, so the runnable copy is inline.)
    #tuple(
      (build 6 5)
      (eval (quasiquote (+ (* (unquote 6) (unquote 6)) (unquote 5))))))
  (export main)))
    (expected "(: #tuple(((. Ast List) #list(((. Ast Name) \"+\") ((. Ast List) #list(((. Ast Name) \"*\") ((. Ast Int) 6) ((. Ast Int) 6))) ((. Ast Int) 5))) 41) (Tuple Ast Int64))"))
  (example
    (id "effects-handlers-stateful")
    (name "Effects & handlers (stateful)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  ; Declare an effect, then discharge it with a handler that threads STATE.
  (effect Tick (op tick (-> Int64 Int64)))
  (def (main)
    ; handler seeds state 0; each tick adds n to the state and returns the new total.
    (handle Tick 0
      ((tick (n) s (resume (+ s n) (+ s n))))
      (+ (Tick.tick 10) (Tick.tick 5))))
  (export main)))
    (expected "25"))
  (example
    (id "iterator-pipeline")
    (name "Iterator pipeline (filter → map → fold)")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "36"))
  (example
    (id "iterator-zip-dot-product")
    (name "Iterator zip: dot product")
    (theme "data-and-collections")
    (surface "sexpr")
    (source (do
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
  (export main)))
    (expected "32"))
  (example
    (id "a-type-error")
    (name "A type error (see the squiggle)")
    (theme "basics")
    (surface "sexpr")
    (source (do
  (def (main) (+ 1 true))
  (export main)))
    (expect-error "true"))
)
