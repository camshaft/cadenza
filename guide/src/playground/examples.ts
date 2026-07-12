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
    name: "A type error (see the squiggle)",
    surface: "sexpr",
    source: `(module m
  (def (main) (+ 1 true))
  (export main))`,
  },
];

export const DEFAULT_EXAMPLE = EXAMPLES[0];
