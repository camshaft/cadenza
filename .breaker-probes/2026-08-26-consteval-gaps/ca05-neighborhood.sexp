; ca05 isolation: (Option Int64) const-param recursive countdown compiled to a HANG on wasm
; (interrupt) and a bare trap on rust, where source semantics trap with a message after 3 calls.

(case "cn01 CONTROL no-const: Option param countdown traps with message at runtime"
  (input  (do
            (def (f (: o (Option Int64)))
              (match o
                ((Option.Some k) (if (= k 0) (trap "cn01 option reached zero") (f (Option.Some (- k 1)))))
                ((Option.None) 0)))
            (def (main) (f (Option.Some 2)))
            (export main)))
  (trap   "cn01 option reached zero"))

(case "cn02 const Option param NON-recursive taken trap surfaces CDZ0304"
  (input  (do
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (if (= k 0) (trap "cn02 zero payload") k))
                ((Option.None) 0)))
            (def (main) (f (Option.Some 0)))
            (export main)))
  (error  CDZ0304 (message "cn02 zero payload")))

(case "cn03 const Option param NON-recursive value path folds"
  (input  (do
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (+ k 10))
                ((Option.None) 0)))
            (def (run) (= (Ast.encode (Ast.Int (BigInt.of (f (Option.Some 5))))) (Ast.encode (Ast.Int (BigInt.of 15)))))
            (export run)))
  (output (: true Bool)))

(case "cn04 const Option recursive countdown RETURNING value (no trap) folds"
  (input  (do
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (if (= k 0) 99 (f (Option.Some (- k 1)))))
                ((Option.None) 0)))
            (def (run) (= (Ast.encode (Ast.Int (BigInt.of (f (Option.Some 2))))) (Ast.encode (Ast.Int (BigInt.of 99)))))
            (export run)))
  (output (: true Bool)))

(case "cn05 recursive countdown on BARE Int64 payload extracted before recursion (const)"
  (input  (do
            (def (g (const (: k Int64)))
              (if (= k 0) (trap "cn05 reached zero") (g (- k 1))))
            (def (f (const (: o (Option Int64))))
              (match o
                ((Option.Some k) (g k))
                ((Option.None) 0)))
            (def (main) (f (Option.Some 2)))
            (export main)))
  (error  CDZ0304 (message "cn05 reached zero")))
