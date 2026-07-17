/// Curated starter programs for the playground's Examples dropdown. Each is a full module (the
/// playground buffer is compiled verbatim) authored in the s-expression surface; the surface toggle
/// re-renders them. All are verified to compile + run against the current compiler.

import type { Surface } from "../compiler/client.ts";

export interface Example {
  name: string;
  surface: Surface;
  source: string;
}

export const EXAMPLES: Example[] = [
  {
    name: "Hello, arithmetic",
    surface: "sexpr",
    source: `(module m
  (def (main) (+ 2 3))
  (export main))`,
  },
  {
    name: "A recursive sum",
    surface: "sexpr",
    source: `(module m
  (def (sm n)
    (if (= n 0) 0 (+ n (sm (- n 1)))))
  (def (main) (sm 5))
  (export main))`,
  },
  {
    name: "Pattern matching",
    surface: "sexpr",
    source: `(module m
  (def (main)
    (match 2
      (1 10)
      (2 20)
      (_ 0)))
  (export main))`,
  },
  {
    name: "Option & sum types",
    surface: "sexpr",
    source: `(module m
  (type Opt (Some Int64) (None unit))
  (def (main)
    (match (Some 7)
      ((Some x) x)
      ((None _) 0)))
  (export main))`,
  },
  {
    name: "Records",
    surface: "sexpr",
    source: `(module m
  (def (area r) (* (. r w) (. r h)))
  (def (main) (area (record (w 4) (h 5))))
  (export main))`,
  },
  {
    name: "A tuple",
    surface: "sexpr",
    source: `(module m
  (def (main) (tuple 1 2 3))
  (export main))`,
  },
  {
    name: "Lists",
    surface: "sexpr",
    source: `(module m
  (def (main)
    (List.len (List.concat (list 1 2) (list 3 4 5))))
  (export main))`,
  },
  {
    // Shows off: a recursive sum type + structural pattern matching = a real (if tiny) interpreter.
    // Each `match` arm destructures one Expr shape; `eval` recurses into the subtrees. This is the
    // "look what you can build" hook — an AST evaluator in a dozen lines. Computes (2 + 3) * -(4) = -20.
    name: "Expression interpreter",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: tail-style recursion + integer arithmetic driving a classic number-theory routine.
    // The Collatz orbit of 27 famously climbs to 9232 before falling — this counts its 111 steps.
    name: "Collatz orbit length",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: first-class functions — a function that RETURNS a function (a closure over f and g).
    // `compose` builds a new function; applying it to 20 runs inc then double ==> 42.
    name: "Function composition",
    surface: "sexpr",
    source: `(module m
  ; compose builds a NEW function that runs g then f — a closure over both.
  (def (compose f g) (fn (x) (f (g x))))
  (def (double x) (* x 2))
  (def (inc x) (+ x 1))
  (def (main) ((compose double inc) 20))
  (export main))`,
  },
  {
    // Shows off: a Map used as a MEMO CACHE, threaded functionally through the recursion. Each call
    // returns (value, updated-map); the cache turns exponential fib into linear. fib(30) = 832040 —
    // it returns instantly, where naive fib(30) would make >2.7M calls.
    name: "Memoized Fibonacci (Map cache)",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: building a frequency table by folding a list into a Map (add-or-bump each key).
    // Uses the List.at/List.len index-recursion idiom (the prelude List has no fold). The result is a
    // (List (Tuple Int64 Int64)) of (value, count) pairs — the shape a notebook `table` cell renders.
    name: "Frequency count (fold into a Map)",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: EXACT rational arithmetic — 1/2 + 1/3 + 1/6 is EXACTLY 1, with no floating-point
    // drift. The (pragma default-fraction Rational) makes bare literals exact fractions; compare with
    // Float64, where 0.1 + 0.2 famously isn't 0.3. The pragma lives in a nested module whose function
    // the outer main calls (the way a module sets its own numeric default).
    name: "Exact rational arithmetic",
    surface: "sexpr",
    source: `(do
  (module m
    (pragma default-fraction Rational)
    ; Bare literals here are EXACT fractions, so this sums to exactly 1 — no float drift.
    (def (sum) (+ (+ (/ 1 2) (/ 1 3)) (/ 1 6))))
  (def (main) ((. m sum) unit))
  (export main))`,
  },
  {
    // Shows off: the classic Euclidean algorithm — recursion + modulo — and deriving LCM from it.
    // gcd(a,b) = gcd(b, a mod b); lcm(a,b) = a*b/gcd. lcm(12,18) = 36.
    name: "GCD and LCM (Euclid)",
    surface: "sexpr",
    source: `(module m
  ; Euclid's algorithm: gcd(a, b) = gcd(b, a mod b), until b is 0.
  (def (gcd a b) (if (= b 0) a (gcd b (% a b))))
  ; LCM falls out of GCD: lcm(a, b) = a * b / gcd(a, b).
  (def (lcm a b) (/ (* a b) (gcd a b)))
  (def (main) (lcm 12 18))
  (export main))`,
  },
  {
    // Shows off: Set operations as first-class values. Symmetric difference (elements in exactly one
    // of two sets) is built from union + difference: (A\\B) ∪ (B\\A). Result: {1, 2, 5, 6}.
    name: "Set algebra (symmetric difference)",
    surface: "sexpr",
    source: `(module m
  ; The symmetric difference of two sets: elements in exactly one of them.
  ; (A minus B) union (B minus A). Sets also give you dedup for free.
  (def (sym-diff a b)
    (Set.union (Set.difference a b) (Set.difference b a)))
  (def (main)
    (let ((a (Set.of (list 1 2 3 4)))
          (b (Set.of (list 3 4 5 6))))
      (Set.to-list (sym-diff a b))))
  (export main))`,
  },
  {
    // Shows off: a sum type as a functional STACK (cons list) with real push/pop via match, driving a
    // stack machine. Evaluates RPN (postfix): numbers push; an operator pops two and pushes the result.
    // "3 4 + 5 *" = (3+4)*5 = 35.
    name: "RPN calculator (stack machine)",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: nested recursion for a real algorithm — count primes ≤ N by trial division.
    // isprime tests divisibility up to √n; count folds over the range. Primes ≤ 100 = 25.
    name: "Count primes (trial division)",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: a cellular automaton — Rule 110, one of the simplest Turing-complete systems — as a
    // list transformation. Each generation maps a row of 0/1 cells to the next via a neighbourhood
    // rule. Returns the row after 4 generations of a single seed cell.
    name: "Rule 110 cellular automaton",
    surface: "sexpr",
    source: `(module m
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
  },
  {
    // Shows off: records + a list of them = a little data pipeline. Aggregate a field (average age)
    // over a list of (name, age) records by index-recursion. (36 + 41 + 40) / 3 = 39.
    name: "Data pipeline over records",
    surface: "sexpr",
    source: `(module m
  ; A list of records — access a field with (. r age) — aggregated into an average.
  (def (sum-age xs i n acc)
    (if (= i n)
        acc
        (match (List.at xs i)
          ((Some r) (sum-age xs (+ i 1) n (+ acc (. r age))))
          ((None) acc))))
  (def (main)
    (let ((people (list (record (name "Ada") (age 36))
                        (record (name "Alan") (age 41))
                        (record (name "Grace") (age 40)))))
      (/ (sum-age people 0 (List.len people) 0) (List.len people))))
  (export main))`,
  },
  {
    name: "A type error (see the squiggle)",
    surface: "sexpr",
    source: `(module m
  (def (main) (+ 1 true))
  (export main))`,
  },
];

export const DEFAULT_EXAMPLE = EXAMPLES[0];
