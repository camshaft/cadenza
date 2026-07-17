/// Curated starter programs for the playground's Examples dropdown. Each is a full program (the
/// playground buffer is compiled verbatim) authored in the s-expression surface as flat top-level
/// definitions grouped in a `(do …)` (no `module` wrapper — that's boilerplate unless an example's
/// point IS modules); the surface toggle re-renders them into idiomatic top-level ML `def`s. All are
/// verified to compile + run against the current compiler.

import type { Surface } from "../compiler/client.ts";

export interface Example {
  name: string;
  surface: Surface;
  source: string;
  /// Optional pinned SCALAR result (bare number or bool, as the runner renders it). When set, the
  /// check-examples gate asserts the program runs to exactly this value — turning the example into a
  /// regression test rather than a mere "it runs" check. Only scalars are pinned (a compound value
  /// renders differently across the s-expr/ML surfaces); compound-returning examples leave it unset.
  expected?: string;
}

export const EXAMPLES: Example[] = [
  {
    name: "Hello, arithmetic",
    surface: "sexpr",
    source: `(do
  (def (main) (+ 2 3))
  (export main))`,
    expected: "5",
  },
  {
    name: "A recursive sum",
    surface: "sexpr",
    source: `(do
  (def (sm n)
    (if (= n 0) 0 (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main))`,
    expected: "15",
  },
  {
    name: "Pattern matching",
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
    name: "Option & sum types",
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
    name: "Records",
    surface: "sexpr",
    source: `(do
  (def (area r) (* (. r w) (. r h)))
  (def (main) (area (record (w 4) (h 5))))
  (export main))`,
    expected: "20",
  },
  {
    name: "A tuple",
    surface: "sexpr",
    source: `(do
  (def (main) (tuple 1 2 3))
  (export main))`,
    expected: "(: (tuple 1 2 3) (Tuple Int64 Int64 Int64))",
  },
  {
    name: "Lists",
    surface: "sexpr",
    source: `(do
  ; Concatenate two lists and return the RESULT — you see the whole list, not just its length.
  (def (main)
    (List.concat (list 1 2) (list 3 4 5)))
  (export main))`,
    expected: "(: (list 1 2 3 4 5) (List Int64))",
  },
  {
    // Shows off: a recursive sum type + structural pattern matching = a real (if tiny) interpreter.
    // Each `match` arm destructures one Expr shape; `eval` recurses into the subtrees. This is the
    // "look what you can build" hook — an AST evaluator in a dozen lines. Computes (2 + 3) * -(4) = -20.
    name: "Expression interpreter",
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
    (eval (Mul (tuple (Add (tuple (Lit 2) (Lit 3)))
                      (Neg (Lit 4))))))
  (export main))`,
    expected: "-20",
  },
  {
    // Shows off: tail-style recursion + integer arithmetic driving a classic number-theory routine.
    // The Collatz orbit of 27 famously climbs to 9232 before falling — this counts its 111 steps.
    name: "Collatz orbit length",
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
    name: "Function composition",
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
    // Shows off: a Map used as a MEMO CACHE, threaded functionally through the recursion. Each call
    // returns (value, updated-map); the cache turns exponential fib into linear. fib(30) = 832040 —
    // it returns instantly, where naive fib(30) would make >2.7M calls.
    name: "Memoized Fibonacci (Map cache)",
    surface: "sexpr",
    source: `(do
  ; A persistent Map threaded as a cache: fib returns (value, updated-map).
  ; Looking a result up before recomputing turns exponential fib into linear.
  (def (fib n mp)
    (match (Map.lookup mp n)
      ((Some v) (tuple v mp))
      ((None)
       (if (< n 2)
           (tuple n (Map.insert mp n n))
           (let ((a (fib (- n 1) mp)))
             (let ((b (fib (- n 2) (. a 1))))
               (let ((r (+ (. a 0) (. b 0))))
                 (tuple r (Map.insert (. b 1) n r)))))))))
  (def (main) (. (fib 30 (Map.empty)) 0))
  (export main))`,
    expected: "832040",
  },
  {
    // Shows off: building a frequency table by folding a list into a Map (add-or-bump each key).
    // Uses the List.at/List.len index-recursion idiom (the prelude List has no fold). The result is a
    // (List (Tuple Int64 Int64)) of (value, count) pairs — the shape a notebook `table` cell renders.
    name: "Frequency count (fold into a Map)",
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
    (let ((xs (list 3 1 3 3 1 2)))
      (Map.to-list (tally xs 0 (List.len xs) (Map.empty)))))
  (export main))`,
    expected: "(: (list (tuple 1 2) (tuple 2 1) (tuple 3 3)) (List (Tuple Int64 Int64)))",
  },
  {
    // Shows off: EXACT rational arithmetic — 1/2 + 1/3 + 1/6 is EXACTLY 1, with no floating-point
    // drift. The `(pragma default-fraction Rational)` directive makes every bare literal in scope an
    // exact fraction; compare with Float64, where 0.1 + 0.2 famously isn't 0.3.
    name: "Exact rational arithmetic",
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
    // Shows off: the classic Euclidean algorithm — recursion + modulo — and deriving LCM from it.
    // gcd(a,b) = gcd(b, a mod b); lcm(a,b) = a*b/gcd. lcm(12,18) = 36.
    name: "GCD and LCM (Euclid)",
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
    name: "Set algebra (symmetric difference)",
    surface: "sexpr",
    source: `(do
  ; The symmetric difference of two sets: elements in exactly one of them.
  ; (A minus B) union (B minus A). Sets also give you dedup for free.
  (def (sym-diff a b)
    (Set.union (Set.difference a b) (Set.difference b a)))
  (def (main)
    (let ((a (Set.of (list 1 2 3 4)))
          (b (Set.of (list 3 4 5 6))))
      (Set.to-list (sym-diff a b))))
  (export main))`,
    expected: "(: (list 1 2 5 6) (List Int64))",
  },
  {
    // Shows off: a sum type as a functional STACK (cons list) with real push/pop via match, driving a
    // stack machine. Evaluates RPN (postfix): numbers push; an operator pops two and pushes the result.
    // "3 4 + 5 *" = (3+4)*5 = 35.
    name: "RPN calculator (stack machine)",
    surface: "sexpr",
    source: `(do
  ; A stack is a cons list — push and pop are just constructors and match.
  (type Stack (Empty unit) (Cons (Tuple Int64 Stack)))
  (def (push s x) (Cons (tuple x s)))
  ; A postfix token: a number, or a binary operator.
  (type Tok (Num Int64) (Plus unit) (Times unit))
  ; An operator pops the top two operands and pushes the result.
  (def (apply2 s f)
    (match s
      ((Cons top1)
       (match (. top1 1)
         ((Cons top2) (push (. top2 1) (f (. top2 0) (. top1 0))))
         ((Empty _) s)))
      ((Empty _) s)))
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
  (def (top s) (match s ((Cons t) (. t 0)) ((Empty _) 0)))
  ; 3 4 + 5 *  ==>  (3 + 4) * 5 = 35
  (def (main)
    (let ((toks (list (Num 3) (Num 4) (Plus unit) (Num 5) (Times unit))))
      (top (run toks 0 (List.len toks) (Empty unit)))))
  (export main))`,
    expected: "35",
  },
  {
    // Shows off: nested recursion for a real algorithm — count primes ≤ N by trial division.
    // isprime tests divisibility up to √n; count folds over the range. Primes ≤ 100 = 25.
    name: "Count primes (trial division)",
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
    name: "Rule 110 cellular automaton",
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
  (def (step xs) (step-from xs 0 (List.len xs) (list)))
  (def (gens xs k) (if (= k 0) xs (gens (step xs) (- k 1))))
  (def (main) (gens (list 0 0 0 0 0 0 0 1) 4))
  (export main))`,
    expected: "(: (list 0 0 0 1 1 1 1 1) (List Int64))",
  },
  {
    // Shows off: records + a list of them = a little data pipeline, returning BOTH the data and the
    // result. Project the age field of each (name, age) record into a list, and compute their average,
    // returning the pair so you SEE both the extracted data [36,41,40] AND the aggregate 39.
    name: "Data pipeline over records",
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
    (let ((people (list (record (name "Ada") (age 36))
                        (record (name "Alan") (age 41))
                        (record (name "Grace") (age 40)))))
      ; Return (the projected ages, their average) — both the data and the result.
      (tuple (ages people 0 (List.len people) (: (list) (List Int64)))
             (/ (sum-age people 0 (List.len people) 0) (List.len people)))))
  (export main))`,
    expected: "(: (tuple (list 36 41 40) 39) (Tuple (List Int64) Int64))",
  },
  {
    // Shows off: exponential recursion made concrete — the minimum moves to solve Tower of Hanoi with
    // n disks is 2^n - 1 (move n-1, move the big disk, move n-1 back). hanoi(10) = 1023.
    name: "Tower of Hanoi (move count)",
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
    name: "Population count (bits)",
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
    name: "Run-length encoding",
    surface: "sexpr",
    source: `(do
  (def (at xs i) (match (List.at xs i) ((Some v) v) ((None) 0)))
  ; Walk the list carrying the current value + its running count; emit on a change.
  (def (go xs i n cur cnt acc)
    (if (= i n)
        (List.push acc (tuple cur cnt))
        (let ((x (at xs i)))
          (if (= x cur)
              (go xs (+ i 1) n cur (+ cnt 1) acc)
              (go xs (+ i 1) n x 1 (List.push acc (tuple cur cnt)))))))
  (def (main)
    (let ((xs (list 1 1 1 2 3 3)))
      ; Annotate the empty seed so its element type is known before the first push.
      (go xs 1 (List.len xs) (at xs 0) 1 (: (list) (List (Tuple Int64 Int64))))))
  (export main))`,
    expected: "(: (list (tuple 1 3) (tuple 2 1) (tuple 3 2)) (List (Tuple Int64 Int64)))",
  },
  {
    // Shows off: string work via bytes — check a word reads the same forwards and backwards by comparing
    // the i-th and (n-1-i)-th UTF-8 bytes moving inward. "racecar" -> 1 (true).
    name: "Palindrome check",
    surface: "sexpr",
    source: `(do
  ; Compare bytes from both ends moving inward; a mismatch means not a palindrome.
  (def (at bs i) (match (Bytes.at bs i) ((Some b) b) ((None) 0)))
  (def (pal bs i j)
    (if (>= i j)
        true
        (if (= (at bs i) (at bs j)) (pal bs (+ i 1) (- j 1)) false)))
  (def (main)
    (let ((bs (String.to-bytes "racecar")))
      (if (pal bs 0 (- (Bytes.len bs) 1)) 1 0)))
  (export main))`,
    expected: "1",
  },
  {
    // Shows off: scanning a string's UTF-8 bytes in a loop — walk each byte and count the ASCII vowels.
    // `Bytes.at` returns an Option (past-the-end is None), so we match it. "education" -> 5.
    name: "Count vowels",
    surface: "sexpr",
    source: `(do
  ; Is this byte one of a e i o u (lowercase ASCII)?
  (def (is-vowel b)
    (if (= b 97) true
    (if (= b 101) true
    (if (= b 105) true
    (if (= b 111) true
    (if (= b 117) true false))))))
  ; Bytes.at returns (Option Int64); treat a missing byte as a non-vowel 0.
  (def (byte-at bs i) (match (Bytes.at bs i) ((Some b) b) ((None) 0)))
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
    // Shows off: computing a result without a built-in — integer square root by searching upward for the
    // largest g with g*g <= n. isqrt(144) = 12.
    name: "Integer square root",
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
    name: "Mutual recursion (even & odd)",
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
    // Shows off: rebuilding a list in reverse by pushing elements from the end onto a fresh list.
    // The prelude List has no reverse, so we walk indices downward. [1 2 3 4 5] -> [5 4 3 2 1].
    name: "Reverse a list",
    surface: "sexpr",
    source: `(do
  (def (at xs i) (match (List.at xs i) ((Some v) v) ((None) 0)))
  ; Walk from the last index down, pushing each element onto a fresh list.
  (def (rev xs i acc)
    (if (< i 0) acc (rev xs (- i 1) (List.push acc (at xs i)))))
  (def (main)
    (let ((xs (list 1 2 3 4 5)))
      (rev xs (- (List.len xs) 1) (: (list) (List Int64)))))
  (export main))`,
    expected: "(: (list 5 4 3 2 1) (List Int64))",
  },
  {
    // Shows off: metaprogramming — code is DATA. `quasiquote` builds an AST value without running it,
    // `unquote` splices a computed value into that AST, and `eval` runs the constructed code. Here we
    // generate the expression (base*base + offset) for base=6, offset=5 and evaluate it -> 41.
    name: "Metaprogramming (quote & eval)",
    surface: "sexpr",
    source: `(do
  ; quasiquote builds code as data; unquote splices a value in; eval runs it.
  ; This generates and evaluates  (base * base) + offset.
  (def (gen base offset)
    (eval (quasiquote (+ (* (unquote base) (unquote base)) (unquote offset)))))
  (def (main) (gen 6 5))
  (export main))`,
    expected: "41",
  },
  {
    // Shows off: algebraic effects & handlers — a signature Cadenza feature — discharged entirely
    // in-program. `Tick.tick n` performs an operation; the handler threads a running total as its
    // STATE, hands back the new total, and resumes. tick 10 -> 10, tick 5 -> 15, so 10 + 15 = 25.
    name: "Effects & handlers (stateful)",
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
    name: "Iterator pipeline (filter → map → fold)",
    surface: "sexpr",
    source: `(do
  ; A linked sequence of Int64: empty (Nil) or a head paired with the rest.
  (type Iter (Nil unit) (Cons (Tuple Int64 Iter)))
  (def (from-list xs)
    (match xs
      ((list) (Nil unit))
      ((list h .. t) (Cons (tuple h (from-list t))))))
  ; Keep elements satisfying p.
  (def (ifilter it p)
    (match it
      ((Nil _) (Nil unit))
      ((Cons c) (if (p (. c 0))
                    (Cons (tuple (. c 0) (ifilter (. c 1) p)))
                    (ifilter (. c 1) p)))))
  ; Apply f to every element.
  (def (imap it f)
    (match it
      ((Nil _) (Nil unit))
      ((Cons c) (Cons (tuple (f (. c 0)) (imap (. c 1) f))))))
  ; Reduce left-to-right with f, starting from acc.
  (def (ifold it acc f)
    (match it
      ((Nil _) acc)
      ((Cons c) (ifold (. c 1) (f acc (. c 0)) f))))
  ; evens of 1..6 -> triple -> sum = 36
  (def (main)
    (ifold (imap (ifilter (from-list (list 1 2 3 4 5 6)) (fn (x) (= 0 (% x 2))))
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
    name: "Iterator zip: dot product",
    surface: "sexpr",
    source: `(do
  ; A linked sequence of Int64: empty (Nil) or a head paired with the rest.
  (type Iter (Nil unit) (Cons (Tuple Int64 Iter)))
  (def (from-list xs)
    (match xs
      ((list) (Nil unit))
      ((list h .. t) (Cons (tuple h (from-list t))))))
  ; Walk a and b in lockstep: sum head-a * head-b, recurse on both tails.
  ; Stops at the shorter (either Nil ends it).
  (def (dot a b)
    (match a
      ((Nil _) 0)
      ((Cons pa)
       (match pa
         ((tuple ha ra)
          (match b
            ((Nil _) 0)
            ((Cons pb)
             (match pb
               ((tuple hb rb) (+ (* ha hb) (dot ra rb)))))))))))
  ; [1,2,3] · [4,5,6] = 1*4 + 2*5 + 3*6 = 32
  (def (main) (dot (from-list (list 1 2 3)) (from-list (list 4 5 6))))
  (export main))`,
    expected: "32",
  },
  {
    name: "A type error (see the squiggle)",
    surface: "sexpr",
    source: `(do
  (def (main) (+ 1 true))
  (export main))`,
  },
];

export const DEFAULT_EXAMPLE = EXAMPLES[0];
