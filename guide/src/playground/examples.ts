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
    name: "A type error (see the squiggle)",
    surface: "sexpr",
    source: `(module m
  (def (main) (+ 1 true))
  (export main))`,
  },
];

export const DEFAULT_EXAMPLE = EXAMPLES[0];
