;; MISCOMPILE — SILENT WRONG VALUE (2026-07-14, seed rcdzc). `cdz check` CLEAN; `cdz compile -t wasm`
;; SUCCEEDS; runs and returns the WRONG value. A persistent `Map.insert(env, …)` MUTATES its operand map
;; when that map is a PARAMETER shared across the SIBLING recursive calls of a self-recursive function —
;; violating `collections-and-text.md` §"A Map Is Built By Functional Construction" ("Each MUST produce a
;; NEW map value and LEAVE ITS OPERAND MAP UNCHANGED").
;;
;; `ev` is a tree-walking interpreter. `(Add (Bind Var) Var)` under `env = {x:1}` should evaluate to
;; 2 + 1 = 3: the LEFT operand `(Bind Var)` evaluates `Var` under `env + {x:2}` (→ 2), and the RIGHT
;; operand `Var` evaluates under the ORIGINAL `env` (→ 1). It returns 4 (= 2 + 2): the left operand's
;; `Map.insert(env, "x", 2)` corrupted the shared `env`, so the right operand's lookup reads 2.
;;
;; SHARP BISECTION (2026-07-14):
;;   - The SAME operand mutation with `let`-bound maps (not a recursive param) is CORRECT (3): `(let ((m
;;     (Map.insert (map) x 1))) (+ (lookup (insert m x 2) x) (lookup m x)))` → 3.
;;   - Two NON-recursive HELPER fns (`via-insert env` + `via-plain env`) summed → CORRECT (3).
;;   - A SINGLE non-recursive fn inserting into its env param in one operand and reading it in the other →
;;     CORRECT (3).
;;   - Only a SELF-RECURSIVE `ev` — where the `Map.insert(env,…)` (in one arm) and the sibling read of the
;;     SAME `env` param are both reached through recursive `ev` calls — MISCOMPILES.
;; So the trigger is a heap MAP PARAMETER consumed-in-place by `map-insert` in one recursive sub-call
;; while a sibling recursive sub-call still reads it — the borrow/Perceus family (a shared param needs a
;; dup before a consuming `map-insert`, exactly like the earlier String/slot-alias findings). It DIRECTLY
;; breaks a scope-threading interpreter/type-checker: `src/interp.cdz`'s `interp-shadow-restores` @test
;; hits it (env not preserved for the sibling after a nested Let inserts).
(do
  (type E (Var) (Add E E) (Bind E))
  (def (ev (: e E) (: env (Map String Int64)))
    (match e
      (((. E Var)) (Option.expect (Map.lookup env "x") "v"))
      (((. E Bind) body) (ev body (Map.insert env "x" 2)))
      (((. E Add) a b) (+ (ev a env) (ev b env)))))
  (def (main (: d Int64))
    (ev ((. E Add) ((. E Bind) ((. E Var))) ((. E Var))) (Map.insert (map) "x" 1)))
  (export main))
