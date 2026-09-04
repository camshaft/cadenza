; Effects and handlers (part 2 of 3) — continuation of 14-effects-and-handlers.sexp, split 2026-08-11
; for parallel-append throughput (glob-enumerated spec/semantics/*.sexp; baselines key on description). Same genre.
(diagnostic-quality)

(case
  "a handler whose STATE is a sum destructures it in the arm"
  (doc
    "The handler's threaded STATE is a SUM (`Option Int64`), and the arm DESTRUCTURES it with a `match`
           to decide the resume value — the state-as-sum analogue of the scalar-countdown handlers. Seeded
           `(Some 5)`, the `get` arm matches its state `s`: `(Some n)` resumes with the payload `n`, `None`
           resumes `0` (a total handler over the state's variants). The body `(+ 1 (St.get))` performs once,
           reads `5` from the `(Some 5)` state, and yields `(+ 1 5)` = 6. Pins that a handler's state slot
           carries a compound sum through the fold and the arm may pattern-match it — the shape of a pass
           threading an optional/typed piece of context (a `Maybe`-valued accumulator) across performs.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          (Some 5)
          ((get (u) s (match s ((Some n) (resume n s)) (None (resume 0 s)))))
          (+ 1 (St.get))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a handler whose STATE is a TUPLE reads both fields and rebuilds the pair per performance"
  (doc
    "The handler's threaded state is a TUPLE packing TWO independent slots — a running accumulator and
           a fixed base — and the arm READS BOTH components (via projection) and REBUILDS the pair to thread
           a modified state, a read-modify-write on a compound state slot. `Acc.step : Int64 -> Int64`, arm
           `(step (v) p (resume (+ (. p 0) (. p 1)) (tuple (+ (. p 0) v) (. p 1))))`: it resumes with the sum
           of the two fields and threads a new tuple advancing only field 0 by `v` (field 1 held). Seeded
           `(0, 100)`: `(Acc.step 1)` reads `(0, 100)` → resumes `0 + 100` = 100, state → `(1, 100)`; `(Acc.step
           2)` reads `(1, 100)` → resumes `1 + 100` = 101, state → `(3, 100)`; so `(+ 100 101)` = 201. Pins
           that a handler state slot carries a TUPLE through the fold — the arm projects its fields and
           reconstructs it — the compound-scalar-pair companion of the sum-state and list-state cases (two
           independent scalar sub-states threaded in one tuple slot, not one shared counter).")
  (input
    (do
      (effect Acc (op step (-> Int64 Int64)))
      (def
        (main)
        (handle
          Acc
          #tuple(0 100)
          ((step (v) p (resume (+ (. p 0) (. p 1)) #tuple((+ (. p 0) v) (. p 1)))))
          (+ (Acc.step 1) (Acc.step 2))))
      (export main)))
  (output (: 201 Int64)))

(case
  "a handler whose STATE is a RECORD combining a scalar counter and a heap LIST field"
  (doc
    "The handler state is a RECORD with a scalar field AND a HEAP field (a list) — the AST-node
           accumulator shape (a record of results one of whose fields is a heap value). Each `push` arm READS
           both fields and REBUILDS the record: it increments the scalar `n` and conses the value onto the
           list `xs`, threading the new record; the `count` arm reads back the scalar `n`. `Acc.push : Int64
           -> Int64`, `Acc.count : Unit -> Int64`, seeded `{n: 0, xs: []}`: `(Acc.push 10)` → `{n: 1, xs:
           [10]}`, `(Acc.push 20)` → `{n: 2, xs: [20, 10]}`, `(Acc.count)` reads `n` = 2. Pins that a handler
           state slot carries a RECORD with a nested HEAP field through the fold — the arm projects its
           fields (scalar and heap) and reconstructs the record, so the value-heap field is correctly
           threaded read-modify-write across performs (the compound-with-heap-field companion of the
           scalar-pair tuple-state and the Set-state cases). Both backends agree (the readout is the scalar
           field).")
  (input
    (do
      (effect Acc (op push (-> Int64 Int64)) (op count (-> Unit Int64)))
      (def
        (main)
        (handle
          Acc
          #record((= n 0) (= xs #list()))
          ((push (v) st (resume v #record((= n (+ st.n 1)) (= xs (List.push st.xs v)))))
            (count (u) st (resume st.n st)))
          (let ((a (Acc.push 10))) (let ((b (Acc.push 20))) (Acc.count)))))
      (export main)))
  (output (: 2 Int64)))

(case
  "an arm chooses its resume value by an if on the handler state"
  (doc
    "A handler arm whose body is NOT a bare `(resume …)` but an `if` on the STATE that resumes a
           different value per branch — a CONDITIONAL resume. `(get (u) s (if (> s 5) (resume 100 s) (resume
           200 s)))`: the arm inspects its state `s` and resumes 100 when `s > 5`, else 200. Seeded 7,
           `7 > 5` holds, so `(Ask.get)` resumes 100 and the body `(+ 1 (Ask.get))` = `(+ 1 100)` = 101.
           Pins that the fold serves an arm that branches on its state to pick the resume value (each branch
           a tail resume) — the scalar-`if` companion of the sum-state `match` arm above, the shape of a
           handler that answers differently depending on the accumulated context.")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (def
        (main)
        (handle Ask 7 ((get (u) s (if (> s 5) (resume 100 s) (resume 200 s)))) (+ 1 (Ask.get))))
      (export main)))
  (output (: 101 Int64)))

(case
  "a performed operation composes under a projection and a negation"
  (doc
    "Witnesses that an effect operation composes under the STRICT one-operand forms — a tuple
           projection and a boolean negation — exactly as under arithmetic. `(. (tuple (Fresh.next)
           (Fresh.next)) 1)` builds a pair from two successive reads (seeded 0 → 0 and 1) and projects the
           second, 1; `(not (= … 0))` negates a comparison of a performed value. Both operands are evaluated
           left to right, threading the counter, before the enclosing op applies. This pins that the fold
           threads through projection/negation, not only conditionals and arithmetic — a performed value is
           an ordinary sub-expression everywhere it appears.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (. #tuple((Fresh.next) (Fresh.next)) 1)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a perform composes as the SOURCE of a pipeline"
  (doc
    "An effect operation composes as the LHS value of a `|>` pipeline — the common surface form for
           `f(perform())`. `(|> (Fresh.next) (+ 100))` desugars to the application `(+ (Fresh.next) 100)`
           (the pipeline splices its value as the first argument of the rhs application), so the perform is
           an ordinary strict operand the fold threads. `Fresh.next` seeded 5 resumes 5, and `5 + 100` = 105.
           Pins that the pipeline desugar preserves the perform's strict-operand position — a performed value
           flows through `|>` exactly as through a direct application, the way an effectful pass reads
           `input |> transform`.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (main) (handle Fresh 5 ((next () s (resume s (+ s 1)))) (|> (Fresh.next) (+ 100))))
      (export main)))
  (output (: 105 Int64)))

(case
  "performs in the ELEMENTS of a tuple / list CONSTRUCTOR thread left-to-right"
  (doc
    "A perform in a tuple or list CONSTRUCTOR element is a strict, ordered position — each element is
           evaluated exactly once, left to right, before the compound is built — so the fold threads it like
           an arithmetic operand or a call argument. This pins the STRING-HEADED constructor primitive
           `(\"tuple\" …)` / `(\"list\" …)`, which is what the ML surface's tuple/list literal `(a, b)` /
           `[a, b]` lowers to (a bare `tuple` NAME reduces via `(meta apply)` and threads through the call
           path; the string-head ctor is the primitive and reaches the compound-constructor fold arm). Two
           `Fresh.next` reads in a tuple, projected and summed: seeded 0, the elements read 0 then 1, so `(+
           (. p 0) (. p 1))` = `0 + 1` = 1. Before this, a perform in a tuple/list/record element declined
           ('not yet reducible by the tail-resumptive fold') — the ML surface (which always emits the
           string-head ctor) could not build a tuple/list from performed values without a manual prefetch;
           now the fold hoists the perform out of the element position like the operand/arg/sum-payload
           cases it already handled. (Record fields — a `(label value)` pair structure — are a follow-up.)")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next () s (resume s (+ s 1))))
          (let ((p #tuple((Fresh.next) (Fresh.next)))) (+ (. p 0) (. p 1)))))
      (export main)))
  (output (: 1 Int64)))

(case
  "performs in the FIELD VALUES of a RECORD constructor thread in written order"
  (doc
    "The record companion of the tuple/list-element case: a perform in a RECORD field VALUE is a
           strict, ordered position — each field value is evaluated in WRITTEN order before the record is
           built — so the fold threads it and rebuilds the `(\"record\" (label rvalue)…)` form, keeping the
           labels. This pins the STRING-HEADED record ctor primitive `(\"record\" …)`, what the ML record
           literal `{ a = …, b = … }` lowers to (its `(label value)` pair args). The fields are WRITTEN `b`
           then `a` (reverse of sorted order) to pin that the VALUES evaluate in written order, not the
           record's canonical sorted order: seeded 0, `b`'s value reads 0 and `a`'s reads 1, so `(- (. r a)
           (. r b))` = `1 - 0` = 1 (had it evaluated `a` first — sorted order — it would be `0 - 1` = -1).
           Before this the record-field perform declined; the fold now hoists it like the tuple/list element
           and the operand/arg/sum-payload cases. Completes the compound-constructor element threading
           (tuple / list / record).")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next () s (resume s (+ s 1))))
          (let ((r #record((= b (Fresh.next)) (= a (Fresh.next))))) (- r.a r.b))))
      (export main)))
  (output (: 1 Int64)))

(case
  "performs in the VALUES of a MAP constructor thread and the entry reads back correctly"
  (doc
    "The map completion of the compound-constructor element threading (tuple/list/record above). A
           perform in a map entry's VALUE is a strict, ordered position — each entry is evaluated in written
           order, and within an entry the key then the value — so the fold threads it and rebuilds the
           `(\"map\" (key rvalue)…)` string-headed ctor. Two `Fresh.next` VALUES under keys 10 and 20: seeded
           0, the first entry's value reads 0 (under key 10), the second reads 1 (under key 20); looking up
           key 20 returns `(Some 1)`, matched to 1. Pins that a map built from performed values threads the
           reads in entry order and stores each under its key correctly (a lookup confirms key 20 holds the
           second read, 1, not the first). Completes tuple / list / record / MAP — an effectful program can
           build any compound from performed values directly. (wasm: rust declines — value-heap/Map emission
           parity gap, not the effects fold.)")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next () s (resume s (+ s 1))))
          (let
            ((m #map((= 10 (Fresh.next)) (= 20 (Fresh.next)))))
            (match (Map.lookup m 20) ((Some v) v) (None 99)))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a SUM constructor payload that is a compound built from performs threads and destructures"
  (doc
    "The composition of the sum-constructor payload path with the compound-constructor element
           threading: the payload of `Some` is a TUPLE built from two performs — `(Some (\"tuple\"
           (Fresh.next) (Fresh.next)))` — using the STRING-HEADED tuple ctor the ML surface emits. The tuple
           threads its two performs (reads 0 then 1), the sum ctor wraps the threaded `(0, 1)`, and the
           enclosing match destructures it: `(Some p)` → `(+ (. p 0) (. p 1))` = `0 + 1` = 1. Pins that a
           compound built from performs composes INSIDE a sum constructor payload (a scalar sum payload
           `W.Mk(Fresh.next())` already worked; this is the compound-payload companion) — the fold threads
           the nested compound-ctor element positions and the sum ctor is a transparent wrapper over the
           threaded value. The shape a real pass builds when it returns `Some((id, node))` from an effectful
           walk. Both backends agree.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next () s (resume s (+ s 1))))
          (match (Some #tuple((Fresh.next) (Fresh.next))) ((Some p) (+ (. p 0) (. p 1))) (None 99))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a performed operation composes as a RECORD field value that is then projected"
  (doc
    "The record-constructor companion of the tuple/projection case: a perform in a RECORD FIELD VALUE
           is a strict, unconditional position, so it composes and the surrounding projection is a pure
           one-hole context. `(. (record (x (Ask.get)) (y 3)) x)` builds a record whose `x` field is the
           performed value, then projects `x` — `C = (. (record (x []) (y 3)) x)`, a strongly-pure context
           around the single perform. `Ask.get` resumes 7, so the record is `{x: 7, y: 3}` and the projection
           yields 7. Pins that the fold threads through a record field the same as a tuple element — a
           performed value is an ordinary sub-expression in a compound constructor.")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (def (main) (handle Ask 0 ((get (u) s (resume 7 s))) (. #record((= x (Ask.get)) (= y 3)) x)))
      (export main)))
  (output (: 7 Int64)))

(case
  "a FLOAT-result effect op threads a float state and its resume folds under a float-dispatched operator"
  (doc
    "The value and state columns of the effect fold are TYPE-AGNOSTIC: an operation whose result and
           whose handler state are both Float64 thread through the same machinery as the Int64 cases, and the
           `+` in the continuation — which now dispatches on operand TYPE (float `+`, no separate `+.`) —
           resolves to the float add inside the folded body. `Rng.next : Unit -> Float64`, arm `(next (u) s
           (resume s (+ s 2.0)))`, seeded 1.5: `(Rng.next)` reads 1.5 (state → 3.5), the second reads 3.5
           (state → 5.5), so `(+ 1.5 3.5)` = 5.0. Pins that (i) a non-Int64 SCALAR result/state slot threads
           correctly (the fold copies values by identity, indifferent to their type) and (ii) the unified
           numeric `+` picks the Float64 add within a folded continuation — the float companion of the
           two-lets / operator-operand Int64 sequencing cases.")
  (input
    (do
      (effect Rng (op next (-> Unit Float64)))
      (def (main) (handle Rng 1.5 ((next (u) s (resume s (+ s 2.0)))) (+ (Rng.next) (Rng.next))))
      (export main)))
  (output (: 5.0 Float64)))

(case
  "a BOOL-result effect op threads state across two performs on a boolean connective's operands"
  (doc
    "The Bool companion of the float/Int64 sequencing cases: a `Unit -> Bool` operation whose resume
           value is derived from the handler state, performed on BOTH operands of an `and` (the left operand
           is true, so the connective does NOT short-circuit and the right also runs). `Coin.flip : Unit ->
           Bool`, arm `(flip (u) s (resume (= s 0) (+ s 1)))`, seeded 0: the first `(Coin.flip)` reads `(= 0
           0)` = true (state → 1), the second reads `(= 1 0)` = false (state → 2), so `(and true (not
           false))` = `(and true true)` = true. Pins that (i) a Bool result/state column threads through the
           fold like any scalar and (ii) when the connective's LEFT operand is true the RIGHT-operand perform
           genuinely runs and reads the ADVANCED state (had it not threaded, the second would read `(= 0 0)` =
           true too and `(not true)` = false → the whole `and` false). Distinct from the abortive-connective
           and pure-one-hole-in-an-and-lhs cases: here BOTH operands perform and thread tail-resumptively.")
  (input
    (do
      (effect Coin (op flip (-> Unit Bool)))
      (def
        (main)
        (handle Coin 0 ((flip (u) s (resume (= s 0) (+ s 1)))) (and (Coin.flip) (not (Coin.flip)))))
      (export main)))
  (output (: true Bool)))

(case
  "a connective-wrapped perform in an if condition threads its state advance to the taken branch"
  (doc
    "A short-circuit connective `(and b (> (St.tick) 0))` sitting DIRECTLY in an `if` CONDITION — the
           condition's `tick` advances the handler state, and the taken branch's `tick` must READ that advance.
           Seeded 0, arm `(tick (u) s (resume (+ s 1) (+ s 1)))`; with `b = true` the condition's `tick` resumes
           1 (state → 1), so the then-branch `(St.tick)` resumes 2. Had the condition's advance been dropped (the
           connective → `if`-desugar's out-state is the post-CONDITION state, which the `If` thread arm does not
           observe per-branch), the then-branch would read the seed and resume 1 — the silent miscompile this
           pins. FIXED by hoist Site 5: a conditional whose CONDITION/SCRUTINEE itself performs in a branch is
           bound to a `let` (`(if C t e)` ≡ `(let ((#cv C)) (if #cv t e))`), turning C into a `let`-init that
           Site 4 distributes so each branch threads under C's advanced state. Controls that already threaded
           (bare effectful compare in the cond, a LET-bound connective) are unaffected; `not` is not part of the
           broken desugar. b=true → 2.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (main (: b Bool))
        (handle
          St
          0
          ((tick (u) s (resume (+ s 1) (+ s 1))))
          (if (and b (> (St.tick) 0)) (St.tick) -99)))
      (export main)))
  (call main (: true Bool))
  (output (: 2 Int64)))

(case
  "a BLOCK-wrapped branch perform in a let-init folds through the block (adv-69 Site-6 commuting conversion)"
  (doc
    "adv-69 through-block fold (v-effects Site 6, the alpha-safe commuting conversion). A branch-
           performing conditional wrapped in a BLOCK inside a `let`-init — `(let ((v (let ((b true)) (if b
           (St.get) 99)))) (+ (* 10 v) (St.get)))`. The hoist's Site 4 lifts a conditional that is DIRECTLY a
           `let`-init to tail position (per-branch threading carries the advance), but a conditional behind a
           `let` block wrapper was opaque to it — historically this DECLINED as a safe floor (the alternative
           was DROPPING the branch's state advance at the block boundary: the trailing `(St.get)` would read
           the block-ENTRY state → 33, not the branch out-state → 34). Site 6 now FLOATS the pure wrapper
           binding `b` OUT into the enclosing `let` (`(let ((b true) (v (if b (St.get) 99))) …)`) — a commuting
           conversion sound because `b` is pure, so hoisting it earlier in the same sequential binding list
           changes no effect order — exposing the conditional as a DIRECT init that Site 4 distributes. The
           branch advance now threads → the trailing `(St.get)` reads 4 → 10*3 + 4 = 34. Folds on all backends
           (shared lowering). Same 34 as the direct-init control below, now reached through the block.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (let ((v (let ((b true)) (if b (St.get) 99)))) (+ (* 10 v) (St.get)))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a DIRECT-init branch perform in a let-init threads its state advance (adv-69 control, still computes)"
  (doc
    "The working control for adv-69's safe-decline: the SAME body but with the branch-performing
           conditional DIRECTLY the `let`-init (no block wrapper) — `(let ((v (if true (St.get) 99))) (+ (* 10
           v) (St.get)))`. Hoist Site 4 lifts it to tail position, so each branch threads the perform's advance
           through the continuation: seeded 3, v=3 (first get, state→4), trailing get reads 4 → 10*3 + 4 = 34.
           Pins that the adv-69 safe-decline floor does NOT over-decline the direct-init path the hoist already
           handles correctly. Computes on all backends.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (let ((v (if true (St.get) 99))) (+ (* 10 v) (St.get)))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

; Three more faces of the adv-69 safe-decline floor (all grade TODO — decline cleanly, flip to a 34/11 PASS
; when the through-block commuting-conversion fold lands): the floor's block_wrapped_branch_performs guard
; peels DEPTH-N pure let/do wrappers and covers a HEAP-accumulator perform in the same let-init position.
; The arm-resume-value positional sub-face (a3 — a block-wrapped OUTER-effect perform inside an INNER
; handle's resume-value) is now ALSO declined by a targeted `Resume{value}`-keyed guard (its case below).
(case
  "a DEPTH-2 block-wrapped branch perform in a let-init folds through the block (adv-69 Site-6, nested wrappers)"
  (doc
    "The depth-2 face of the adv-69 safe-decline: the branch-performing conditional sits behind TWO
           nested `let` wrappers in the init — `(let ((v (let ((b true)) (let ((c true)) (if (and b c) (St.get)
           99))))) …)`. The floor's block_wrapped_branch_performs peel recurses through depth-N pure let/do
           wrappers, so this FOLDS via Site 6 exactly as the depth-1 witness does (float the pure wrappers out, then Site 4) — NOT the silent 33
           state-drop. Pins that the trigger is ANY block nesting ≥1, not a single-wrapper shape. Flips to the
           34 (Site 6 floats the pure wrapper bindings out; Site 4 then distributes). Folds on all backends (shared lowering).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (let
            ((v (let ((b true)) (let ((c true)) (if (and b c) (St.get) 99)))))
            (+ (* 10 v) (St.get)))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a BLOCK-wrapped branch perform in a MATCH-SCRUTINEE folds through the block (adv-69 g3)"
  (doc
    "The match-scrutinee face of the through-block fold: `(match (let ((b true)) (if b (St.get) 99)) (v
           (+ (* 10 v) (St.get))))`. Site 6 floats the pure wrapper `b` out, exposing the direct conditional
           scrutinee Site 5 lifts, so the branch advance threads → the trailing `(St.get)` reads 4 → 10*3 + 4
           = 34 (was the safe-floor decline / silent 33).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (match (let ((b true)) (if b (St.get) 99)) (v (+ (* 10 v) (St.get))))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a BLOCK-wrapped branch perform in a non-tail DO-STATEMENT folds to 73 (adv-69 c3)"
  (doc
    "A block-wrapped branch-performing conditional as a non-tail `do`-statement: `(do (let ((x true))
           (if x (St.put 7) unit)) (+ (* 10 (St.get)) x))`. Site 1 hoists a DIRECT non-last branch-performing
           item; the block (`let`) wrapper once hid it (the `put` advance dropped → safe-decline), but Site 1's
           THROUGH-BLOCK extension now FOLDS it: FRESHEN the wrapper's local binders (alpha-rename so `rest`'s
           `x` = the enclosing fn param is not captured by the block's `x`), PEEL the pure `let` wrapper,
           distribute `rest` into the conditional branches, and RE-WRAP in the freshened `let`. `x`=true → the
           `if` runs `(St.put 7)` (state 3→7), then `(+ (* 10 (St.get)) x)` = `10·7 + 3` = 73.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def
        (main (: x Int64))
        (handle
          St
          x
          ((get (u) s (resume s s)) (put (v) _s (resume unit v)))
          (do (let ((x true)) (if x (St.put 7) unit)) (+ (* 10 (St.get)) x))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 73 Int64)))

(case
  "a BLOCK-wrapped OUTER perform in a do-statement inside a nested handle body folds to 73 (adv-69 c3-nested)"
  (doc
    "The c3 do-statement drop where the block-wrapped perform is of the OUTER effect and sits in a
           `do`-statement INSIDE a nested inner handler's body: the outer reduction's statement scanner would
           miss it past the nested `Handle`, so the fold declines cleanly rather than drop the advance.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
      (effect B (op gb (-> Unit Int64)))
      (def
        (main (: x Int64))
        (handle
          A
          x
          ((ga (u) s (resume s s)) (pa (v) _s (resume unit v)))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (do (let ((k true)) (if k (A.pa 7) unit)) (+ (* 10 (A.ga)) x)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 73 Int64)))

(case
  "a HEAP-accumulator block-wrapped branch perform in a let-init declines cleanly (adv-69 safe floor)"
  (doc
    "The heap-state face: the block-wrapped branch performs a HEAP-accumulating effect — a `Log.add`
           that `List.push`es onto the handler's list state — in the let-init `(let ((v (let ((b true)) (if b
           (Log.add 5) 99)))) …)`. Under the floor this declines cleanly (TODO) rather than DROPPING the push
           (the silent-miscompile would lose the entry: `count` would read the entry list, e.g. length 0 not
           1) — the data-loss face of the block-boundary out-state drop, so the safe decline matters more here
           than for a stale scalar. Flips to the 11 PASS (10*1 + 1, the pushed entry counted twice) when the
           through-block fold lands. Declines on all backends.")
  (input
    (do
      (effect Log (op add (-> Int64 Unit)) (op count (-> Unit Int64)))
      (def
        (main)
        (handle
          Log
          #list()
          ((add (v) s (resume unit (List.push s v))) (count (u) s (resume (List.len s) s)))
          (let ((v (let ((b true)) (if b (Log.add 5) 99)))) (+ (* 10 (Log.count)) (Log.count)))))
      (export main)))
  (call main)
  (output (: 11 Int64)))

(case
  "a BLOCK-wrapped branch perform in a NESTED handler-arm resume-value declines cleanly (adv-69 a3 sub-face)"
  (doc
    "adv-69 a3 (breaker probe-a3, block-outstate battery): the SAME block-boundary out-state drop as the
           let-init floor above, but at a DIFFERENT position — a block-wrapped branch-performing conditional in
           a NESTED handler's arm RESUME-VALUE, performing the OUTER handler's op. The outer `St` handler threads
           its state through the inner `Up` handle, but the block boundary inside the inner arm's resume-VALUE
           `(resume (let ((b true)) (if b (St.get) 99)) t)` dropped the outer `St.get`'s advance: seeded 3 it ran
           33, correct is 34 (= 10*(St.get resumes 3, state→4 seen by trailing get) ... trailing `(St.get)` reads
           4). The let-init scanner stops at a nested `handle` (an inner handle's lets are its own reduction), so
           this position escaped that floor. A targeted guard keyed PRECISELY on the `Resume{value}` position
           (not a position-agnostic block-wrapped-perform scan, which over-declines working threaded positions)
           declines this residual shape → a clean Todo, never the silent 33. Grades TODO on all backends; its 34
           becomes a PASS when the full through-block fold lands (same deferred commuting conversion as the
           let-init face).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (effect Up (op ask (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (handle
            Up
            0
            ((ask (u) t (resume (let ((b true)) (if b (St.get) 99)) t)))
            (+ (* 10 (Up.ask)) (St.get)))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a DIRECT branch perform in a NESTED handler-arm resume-value declines cleanly (adv-69 a3-direct sub-face)"
  (doc
    "The DIRECT-conditional twin of the a3 case: `(resume (if true (St.get) 99) t)` — a branch-performing
           conditional DIRECTLY (no block wrapper) in a nested handler's arm resume-value, performing the OUTER
           op. Unlike the let-init face — where a DIRECT init is lifted by Site 4 and folds — a `resume`-value
           is never hoisted (it lives inside the inner `Up` handle's arm, which the outer `St` reduction does
           not rewrite), so the direct conditional here ALSO drops the outer `St.get`'s advance: seeded 3 it ran
           33, correct is 34. The a3 guard's `Resume{value}` scanner declines this via its direct-conditional
           disjunct (verified: dropping that disjunct makes this miscompile to 33, not fold to 34 — so the
           disjunct is load-bearing, not an over-decline). Pins that the resume-value drop is NOT block-wrapper-
           specific (contrast the let-init face). Grades TODO on all backends; flips to 34 PASS on the
           through-block fold.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (effect Up (op ask (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (handle Up 0 ((ask (u) t (resume (if true (St.get) 99) t))) (+ (* 10 (Up.ask)) (St.get)))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a block-wrapped branch perform of the OUTER effect in a let-init INSIDE a nested handler body folds through the block (adv-69 a4)"
  (doc
    "adv-69 a4 (v-effects self-probe 2026-08-04): the let-init block-boundary drop, but the miscompiling
           `let` sits inside a NESTED inner handler's BODY and performs the OUTER effect. `(handle A 3 ((ga …))
           (handle B 100 ((gb …)) (let ((v (let ((k true)) (if k (A.ga) 9)))) (+ (* 10 v) (A.ga)))))` — the
           block-wrapped branch perform is of the OUTER `A`, in a `let`-init in the inner `B` handle's body, and
           the continuation RE-READS `A`. Seeded A=3 it ran 33, correct is 34 (A.ga returns 3, advances to 4;
           trailing A.ga must read 4). The single-handle version of this shape folds via the let-init Site 6,
           but the intervening nested `B` handle made the OUTER `A` reduction's scanner stop at the inner
           `Handle` and MISS the block-wrapped `A`-perform in `B`'s body — a silent miscompile. FIX: the scanner
           now descends into a nested handle's BODY (not its arms — that is a3's territory) keeping the OUTER
           ctx, so `block_wrapped_branch_performs` (ctx-keyed) fires only on an OUTER-effect perform (an inner
           `B`-effect perform never matches → no over-decline of `B`'s own shapes). Grades TODO on all backends;
           flips to 34 PASS on the through-block fold.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)))
      (effect B (op gb (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          3
          ((ga (u) s (resume s (+ s 1))))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (let ((v (let ((k true)) (if k (A.ga) 9)))) (+ (* 10 v) (A.ga))))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a DIRECT-conditional OUTER-effect perform in a let-init inside a nested handler body folds (adv-69 a4 control)"
  (doc
    "The direct-conditional twin of the a4 through-block fold (no block wrapper) at the same position:
           Site 4 lifts a direct `let`-init even through the nested B handle, so the outer-A branch advance
           threads and the trailing `(A.ga)` reads the advanced state → 10*3 + 4 = 34. Pins that the a4
           through-block fix does not over-decline the already-working direct-init path.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)))
      (effect B (op gb (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          3
          ((ga (u) s (resume s (+ s 1))))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (let ((v (if true (A.ga) 9))) (+ (* 10 v) (A.ga))))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a block-wrapped OUTER-effect perform in a let-init THREE handlers deep folds through the block (adv-69 a4-depth3)"
  (doc
    "adv-69 a4 at DEPTH-3 (breaker nh5 escalation, block-outstate battery): the a4 nested-handle-body
           drop, but the block-wrapped OUTER-effect (`A`) perform sits in a `let`-init THREE handlers deep —
           `(handle A 3 (…) (handle B 100 (…) (handle C 200 (…) (let ((v (let ((k true)) (if k (A.ga) 9))))
           (+ (* 10 v) (A.ga))))))`. Seeded A=3 it ran 33, correct is 34. Pins that the a4 scanner's descent
           into a nested handle's BODY is RECURSIVE, not one-level: the outer `A` reduction descends through
           BOTH the `B` and `C` handle bodies (keeping the outer ctx) to reach the block-wrapped `A`-perform.
           If the descent peeled only one `Handle`, this depth-3 shape would escape and miscompile — so this
           locks in the depth-N property (analogous to the a2 depth-2 witness for the flat let-init floor).
           Folds to 34 on all backends via the Site-6 through-block commuting conversion (floats the pure wrapper out, then Site 4 distributes).")
  (input
    (do
      (effect A (op ga (-> Unit Int64)))
      (effect B (op gb (-> Unit Int64)))
      (effect C (op gc (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          3
          ((ga (u) s (resume s (+ s 1))))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (handle
              C
              200
              ((gc (u) w (resume w w)))
              (let ((v (let ((k true)) (if k (A.ga) 9)))) (+ (* 10 v) (A.ga)))))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a block-wrapped OUTER-effect perform in a nested handle's INIT folds to 34 (adv-69 a4-init sub-face)"
  (doc
    "adv-69 a4-init (liaison/Copilot on merged #1933): the a4 nested-handle-escape, but the block-wrapped
           OUTER-effect perform sits in the inner handle's INIT — `(handle A 3 ((ga …)) (handle B (let ((k true))
           (if k (A.ga) 9)) ((gb …)) (+ (* 10 (B.gb)) (A.ga))))`. The inner `B` handle's INIT is evaluated as
           part of the handle expression in the OUTER `A` extent (eval.rs passes `init` to `reduce_handle`
           alongside `body`), so a block-wrapped `A`-perform there drops the outer advance exactly like the a4
           body face: seeded A=3 it ran 33, correct is 34 (B's init A.ga returns 3, A→4; B.gb returns B-state 3;
           trailing A.ga must read 4 → 10*3 + 4). The a4 fix scanned the inner handle's BODY but early-returned
           without the INIT, missing this position. FIX: the nested-Handle scan checks the init node directly
           (`block_wrapped_branch_performs`) AND recurses into both init and body — ctx-keyed, so only an
           OUTER-op perform fires (no over-decline of `B`'s shapes). Now FOLDS to 34 (Site 7 nested-handle-init through-block float, reduce.rs): the pure `let`
           wrapper is floated OUTSIDE the inner handle so the conditional is a DIRECT seed the outer fold threads.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)))
      (effect B (op gb (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          3
          ((ga (u) s (resume s (+ s 1))))
          (handle
            B
            (let ((k true)) (if k (A.ga) 9))
            ((gb (u) t (resume t t)))
            (+ (* 10 (B.gb)) (A.ga)))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a BLOCK-wrapped branch perform in a MATCH-SCRUTINEE folds through the block (adv-69 g3)"
  (doc
    "adv-69 g3 (breaker probe-g3, block-outstate battery): the SAME block-boundary out-state drop, at a
           MATCH-SCRUTINEE consuming position. `(match (let ((b true)) (if b (St.get) 99)) (v (+ (* 10 v)
           (St.get))))` — the scrutinee is a block-wrapped branch-performing conditional. Site 5 lifts a
           scrutinee that is DIRECTLY a branch-performing conditional (per-branch threading carries its
           advance), but a block wrapper is opaque to it, so the scrutinee's out-state reverts to entry: seeded
           3 it ran 33, correct is 34 (v=3, state→4, trailing `(St.get)` reads 4). Keyed on the WRAPPED shape
           only (a DIRECT `if`/`match` scrutinee still folds — no over-decline of the Site-5 path). Folds
           to 34 via Site 6 (floats the pure wrapper out, exposing the direct scrutinee Site 5 handles), never the silent 33.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (match (let ((b true)) (if b (St.get) 99)) (v (+ (* 10 v) (St.get))))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a BLOCK-wrapped branch perform in a non-tail DO-STATEMENT folds to 73 (adv-69 c3 sub-face)"
  (doc
    "adv-69 c3 (breaker probe-c3, block-outstate battery): the SAME block-boundary out-state drop, at a
           non-tail `do`-STATEMENT position. `(do (let ((x true)) (if x (St.put 7) unit)) (+ (* 10 (St.get))
           x))` — a block-wrapped branch perform as a DISCARDED (non-last) `do` item. Site 1 hoists a non-last
           item that is DIRECTLY a branch-performing conditional (distributing the continuation into each
           branch), but a block wrapper defeats its match, so the statement's `St.put 7` advance is dropped:
           seeded 3 the trailing `(St.get)` reads the stale pre-statement state → ran 33, correct is 73 (put
           sets state 7, `(St.get)` resumes 7 → 10*7 + shadowed-outer x=3 = 73). The minimal twins d2/e1 — a
           BARE `if` in the statement, or a def-bound cond — hoist fine and PASS, so this keys on the block
           wrapper. Declines cleanly → a clean Todo, never the silent 33; flips to 73 PASS on the through-block
           fold.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def
        (main (: x Int64))
        (handle
          St
          x
          ((get (u) s (resume s s)) (put (v) _s (resume unit v)))
          (do (let ((x true)) (if x (St.put 7) unit)) (+ (* 10 (St.get)) x))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 73 Int64)))

(case
  "a block-wrapped OUTER-effect perform in a non-tail do-statement INSIDE a nested handler body folds to 73 (adv-69 c3-nested sub-face)"
  (doc
    "adv-69 c3-nested (v-effects self-probe 2026-08-04): the c3 non-tail do-statement drop, but the
           block-wrapped branch perform is of the OUTER effect and sits in a `do`-statement INSIDE a nested
           inner handler's body. `(handle A x ((ga …)(pa …)) (handle B 100 ((gb …)) (do (let ((k true)) (if k
           (A.pa 7) unit)) (+ (* 10 (A.ga)) x))))` — the discarded statement's `A.pa 7` advance drops at the
           block boundary: seeded 3 it ran 33, correct is 73 (pa sets state 7, `(A.ga)` reads 7 → 10*7 + x=3).
           Same nested-handle-escape class as the a4 let-init face, but for the do-statement scanner: the outer
           `A` reduction's `body_has_block_wrapped_scrutinee_or_statement_branch_perform` scan STOPPED at the
           nested `B` `Handle` and missed the block-wrapped `A`-perform in `B`'s body. FIX: that scanner now
           descends into a nested handle's BODY (not arms) keeping the OUTER ctx — ctx-keyed so only an
           outer-effect perform fires (no over-decline of `B`'s own shapes). Grades TODO on all backends; flips
           to 73 PASS on the through-block fold.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
      (effect B (op gb (-> Unit Int64)))
      (def
        (main (: x Int64))
        (handle
          A
          x
          ((ga (u) s (resume s s)) (pa (v) _s (resume unit v)))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (do (let ((k true)) (if k (A.pa 7) unit)) (+ (* 10 (A.ga)) x)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 73 Int64)))

(case
  "a block-wrapped OUTER-effect perform in a do-statement TWO nested handlers deep folds to 73 (adv-69 nh7 depth-3)"
  (doc
    "adv-69 nh7 (breaker depth escalation of c3-nested): the SAME non-tail do-statement drop, but the
           outer `A`-perform sits inside TWO stacked nested handlers (`B` then `C`) — `(handle A x (…) (handle
           B 100 (…) (handle C 200 (…) (do (let ((k true)) (if k (A.pa 7) unit)) (+ (* 10 (A.ga)) x)))))`. The
           block-wrapped `A.pa 7` advance drops at the block boundary: seeded 3 it ran 33, correct is 73 (pa
           sets state 7, `(A.ga)` reads 7 → 10*7 + x=3). Verifies the c3-nested scanner's nested-handle-body
           descent is RECURSIVE — it re-invokes on EACH nested body, so the depth-2 nesting (A over B over C)
           is covered exactly like depth-1, the depth-N regression guard analogous to a4-depth3 for the let-
           init scanner. Grades TODO on all backends; flips to 73 PASS on the through-block fold.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)) (op pa (-> Int64 Unit)))
      (effect B (op gb (-> Unit Int64)))
      (effect C (op gc (-> Unit Int64)))
      (def
        (main (: x Int64))
        (handle
          A
          x
          ((ga (u) s (resume s (+ s 1))) (pa (v) _s (resume unit v)))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (handle
              C
              200
              ((gc (u) w (resume w w)))
              (do (let ((k true)) (if k (A.pa 7) unit)) (+ (* 10 (A.ga)) x))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 73 Int64)))

(case
  "a block-wrapped OUTER-effect perform in a match-SCRUTINEE inside a nested handler body folds through the block (adv-69 g3-nested)"
  (doc
    "adv-69 g3-nested (v-effects bonus-probe of the c3-nested fix): the g3 match-scrutinee face of the
           nested-handle-body escape. A block-wrapped OUTER `A`-perform sits in a `match` SCRUTINEE inside a
           nested `B` handler's body — `(handle A 3 ((ga …)) (handle B 100 ((gb …)) (match (let ((k true)) (if
           k (A.ga) 9)) (v (+ (* 10 v) (A.ga))))))`. The block-wrapped branch perform's advance drops at the
           block boundary: seeded 3 it ran 33, correct is 34 (the scrutinee `A.ga` reads 3 and advances state
           to 4, so v = 3; the arm's trailing `(A.ga)` reads the advanced 4 → 10*3 + 4 = 34). The g3/c3 scanner
           (`body_has_block_wrapped_scrutinee_or_statement_branch_perform`) shares
           the do-statement scanner's nested-handle-body descent, so the match-scrutinee position in a nested
           body is covered by the same fix. Folds to 34 on all backends via the Site-6 through-block
           fold.")
  (input
    (do
      (effect A (op ga (-> Unit Int64)))
      (effect B (op gb (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          3
          ((ga (u) s (resume s (+ s 1))))
          (handle
            B
            100
            ((gb (u) t (resume t t)))
            (match (let ((k true)) (if k (A.ga) 9)) (v (+ (* 10 v) (A.ga)))))))
      (export main)))
  (call main)
  (output (: 34 Int64)))

(case
  "a block-wrapped conditional whose CONDITION performs (not a branch) folds correctly (adv-69 boundary control)"
  (doc
    "The passing boundary control for the adv-69 guards: a block-wrapped conditional whose CONDITION
           performs — `(let ((v (let ((b (> (St.get) 0))) (if b 7 99)))) (+ (* 10 v) (St.get)))` — must still
           FOLD, not decline. Unlike the adv-69 faces (where a BRANCH performs, so the advance is branch-local
           and drops at the block boundary), here the perform is a pure `let`-binding on the block's STRICT
           SPINE: `(St.get)` runs unconditionally as `b`'s init, advancing the state once (seeded 3 → 4), and
           the `if`'s branches (7 / 99) perform nothing. So the block's out-state IS the threaded post-perform
           state — no drop. v = 7 (b = 3>0 = true), trailing `(St.get)` reads the advanced 4 → 10*7 + 4 = 74.
           Pins that the adv-69 decline-guards (`block_wrapped_branch_performs` et al.) key on a BRANCH perform,
           NOT any perform inside a block — a condition/spine perform is correctly threaded and folds. Computes
           on all backends.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          St
          3
          ((get (u) s (resume s (+ s 1))))
          (let ((v (let ((b (> (St.get) 0))) (if b 7 99)))) (+ (* 10 v) (St.get)))))
      (export main)))
  (call main)
  (output (: 74 Int64)))

(case
  "two performs bound by nested lets thread the handler state in order"
  (doc
    "Two performs on the strict spine, each BOUND by its own `let`, thread the handler state in
           evaluation order across the binds. `(let ((a (Ask.get))) (let ((b (Ask.get))) (+ a b)))` under a
           counter that hands back `s` and threads `s + 10` (seeded 0): the first `Ask.get` binds `a = 0`
           (state → 10), the second binds `b = 10` (state → 20), so `(+ a b)` = `(+ 0 10)` = 10. The `let`
           inits run unconditionally in sequence — a strict spine the threading fold walks left to right —
           so each perform sees the state the previous one advanced, not the seed. Pins sequential
           state-threading through a chain of let bindings (had the state not threaded, both reads would be
           0 and the sum 0), the essential shape of a pass pulling several fresh values in a row.")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          Ask
          0
          ((get (u) s (resume s (+ s 10))))
          (let ((a (Ask.get))) (let ((b (Ask.get))) (+ a b)))))
      (export main)))
  (output (: 10 Int64)))

(case
  "two performs of a MULTI-parameter op each combine both args with the state advancing between them"
  (doc
    "A two-scalar-parameter operation whose arm combines BOTH arguments with the threaded state, called
           twice on one strict spine so each perform reads the state the previous one advanced. `Acc.add2 :
           Int64 -> Int64 -> Int64`, arm `(add2 (a b) s (resume (+ (+ a b) s) (+ s 1)))` — sums its two args
           plus the current state, threading `s + 1`. Seeded 100: `(Acc.add2 1 2)` = `1 + 2 + 100` = 103
           (state → 101), then `(Acc.add2 10 20)` = `10 + 20 + 101` = 131 (state → 102), so `(+ 103 131)` =
           234. Pins that a multi-parameter op's arm binds ALL its parameters AND the state, and that the
           state advances between successive performs on the spine (had it not threaded, the second would
           read 100 too, giving 233) — the multi-arg companion of the sequential-state-threading case above.")
  (input
    (do
      (effect Acc (op add2 (-> Int64 Int64 Int64)))
      (def
        (main)
        (handle
          Acc
          100
          ((add2 (a b) s (resume (+ (+ a b) s) (+ s 1))))
          (+ (Acc.add2 1 2) (Acc.add2 10 20))))
      (export main)))
  (output (: 234 Int64)))

(case
  "a THREE-parameter op arm binds all three parameters and the state"
  (doc
    "The arity extension of the two-parameter case: an operation with THREE scalar parameters whose
           arm binds all three plus the state. `Acc.add3 : Int64 -> Int64 -> Int64 -> Int64`, arm `(add3 (a
           b c) s (resume (+ (+ (+ a b) c) s) (+ s 1)))` — sums its three args plus the current state,
           threading `s + 1`. Seeded 1000: `(Acc.add3 1 2 3)` = `1 + 2 + 3 + 1000` = 1006 (state → 1001),
           then `(Acc.add3 10 20 30)` = `10 + 20 + 30 + 1001` = 1061 (state → 1002), so `(+ 1006 1061)` =
           2067. Pins that arm-parameter binding scales past two — all three op parameters AND the state
           binder resolve in the arm body, and the state still threads between successive performs on the
           spine.")
  (input
    (do
      (effect Acc (op add3 (-> Int64 Int64 Int64 Int64)))
      (def
        (main)
        (handle
          Acc
          1000
          ((add3 (a b c) s (resume (+ (+ (+ a b) c) s) (+ s 1))))
          (+ (Acc.add3 1 2 3) (Acc.add3 10 20 30))))
      (export main)))
  (output (: 2067 Int64)))

(case
  "a perform's result flowing as the ARGUMENT of an enclosing perform threads state inner-to-outer"
  (doc
    "The data dependency runs THROUGH the argument position rather than through a let: the inner
           perform's result is the very argument the outer perform consumes — `(Acc.step (Acc.step 1))`.
           Because an argument is evaluated before its call, the INNER perform runs first and advances the
           state the OUTER one then reads, so the two are still sequenced left-of-the-arrow / inner-first.
           `Acc.step : Int64 -> Int64`, arm `(step (a) s (resume (+ a s) (+ s 1)))`, seeded 100: inner
           `(Acc.step 1)` = `1 + 100` = 101 (state → 101), outer `(Acc.step 101)` = `101 + 101` = 202 (state
           → 102), so the result is 202. Pins that state threads through nested-perform ARGUMENT evaluation
           in inner-to-outer order (had the outer read the seed 100 instead of the inner's advanced 101 it
           would be 201) — the argument-position companion of the two-lets and multi-param cases above, with
           the added twist that one perform's OUTPUT is the other's INPUT.")
  (input
    (do
      (effect Acc (op step (-> Int64 Int64)))
      (def (main) (handle Acc 100 ((step (a) s (resume (+ a s) (+ s 1)))) (Acc.step (Acc.step 1))))
      (export main)))
  (output (: 202 Int64)))

(case
  "a CROSS-handler op whose inline ARG performs an OUTER handler's op folds (op-arg let-lift)"
  (doc
    "The cross-handler analogue of the same-handler nested-perform-arg case above (Acc.step (Acc.step 1)):
           a NESTED handler's op whose INLINE argument performs an OUTER handler's op — `(B.put (A.get))` under
           `handle A (handle B …)`, where `A.get` homes to the enclosing `A` (foreign to `B`). B's arm uses its
           param `v` TWICE (`(resume (+ s v) (+ s v))`), so substituting the performing `(A.get)` inline would
           duplicate it (effect-duplication guard). Fixed by the op-arg LET-LIFT: bind the foreign-perform arg
           to a fresh `#cv` once, then B's arm reads the pure ref twice — exactly the WORKING let-bound spelling
           `(let ((x (A.get))) (B.put x))`. `A.get`=7 (no advance), `B.put(7)` = `0+7` = 7. Pins that an inline
           cross-handler op-arg-performs-outer folds (was a clean decline — the inline-arg-position completeness
           gap; the let-bound spelling always folded).")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op put (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          7
          ((get (u) s (resume s s)))
          (handle B 0 ((put (v) s (resume (+ s v) (+ s v)))) (B.put (A.get)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a cross-handler op-arg performing an outer effect runs EXACTLY ONCE though the arm uses it thrice"
  (doc
    "The soundness control for the op-arg let-lift: the foreign-perform arg must run ONCE (an op arg is
           evaluated once, before the call, regardless of how many times the arm reads its param). `(B.put
           (A.tick))` where `A.tick` ADVANCES the outer A-state, and B's arm reads `v` THREE times
           (`(resume (+ (+ v v) v) s)`). If the lift wrongly duplicated the perform, A would advance 3× and the
           reads would differ; correctly it advances ONCE (10→11), all three reads see 10 → `v+v+v` = 30, then
           the outer `(A.get)` reads the once-advanced 11 → `(+ 30 11)` = 41. Pins that the `#cv` let-bind
           runs the foreign perform exactly once and the arm reads the pure ref — no effect duplication.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op put (-> Int64 Int64)))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (let
            ((b (handle B 0 ((put (v) s (resume (+ (+ v v) v) s))) (B.put (A.tick)))))
            (+ b (A.get)))))
      (export main)))
  (output (: 41 Int64)))

(case
  "TWO outer op-results as SIBLING args of one inner perform evaluate left-to-right"
  (doc
    "The multi-arg face of the op-arg let-lift: BOTH arguments of the inner `(B.put (A.get) (A.get))`
           are foreign performs of the ADVANCING outer op, so their evaluation ORDER is observable — the
           first read returns 7 (state → 8), the second 8 (state → 9), and B's arm sums them (15). A lift
           that reordered the sibling performs, ran one twice, or batched them against the same state would
           break the sum. The two-lift companion of the single-arg pin above.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op put (-> Int64 Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((get (u) s (resume s (+ s 1))))
          (handle B 0 ((put (v w) s (resume (+ v w) s))) (B.put (A.get) (A.get)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 15 Int64)))

(case
  "a DEPTH-3 op-arg chain threads across two handler layers"
  (doc
    "The depth face of the op-arg let-lift: `(C.inn (B.mid (A.get)))` under a 3-deep stack — the
           OUTERMOST perform's argument is itself a perform, cascading inward to `A.get` (whose argument is
           Unit, not a perform), so the lift must fire at two nesting levels of the SAME expression. A.get reads 7, B.mid adds
           its state (7+100 = 107), C.inn doubles (214). A lift that flattened only one level, or evaluated
           the chain against the wrong handler's state, would break a factor. The chain companion of the
           sibling-args pin above.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op mid (-> Int64 Int64)))
      (effect C (op inn (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((get (u) s (resume s s)))
          (handle
            B
            100
            ((mid (v) s (resume (+ v s) s)))
            (handle C 0 ((inn (v) s (resume (* 2 v) s))) (C.inn (B.mid (A.get)))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 214 Int64)))

(case
  "the cross-handler op-arg lift fires 100 times inside a recursive accumulator loop"
  (doc
    "The SCALE face of the op-arg let-lift: `(B.put (A.get))` — the single-shot pin above — placed in a
           100-iteration accumulator loop, with A's arm ADVANCING per read. B's arm reads its param THREE
           times (`(/ (+ (+ v v) v) 3)` = v exactly), so each iteration's lift must bind the foreign perform
           ONCE and serve the pure ref thrice — a lift that re-ran the perform per read would see A advance
           between reads (v, v+1, v+2) and shift the quotient. Every advance must also thread across
           iterations: the sum of A's reads 0..99 = 4950. The recursion companion of the sibling-args and
           depth-3 pins, with the arm shaped to force the lift's duplication handling.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op put (-> Int64 Int64)))
      (def (loop (: n Int64) (: acc Int64)) (if (= n 0) acc (loop (- n 1) (+ acc (B.put (A.get))))))
      (def
        (main (: k Int64))
        (handle
          A
          0
          ((get (u) s (resume s (+ s 1))))
          (handle B 0 ((put (v) s (resume (/ (+ (+ v v) v) 3) s))) (loop k 0))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 4950 Int64)))

(case
  "a cross-handler foreign-perform op-arg into a MATCH-shaped-resume arm folds under nested handlers (nv1f)"
  (doc
    "A nested-handle fold where the inner `B.cut`'s ARG is a CROSS-HANDLER foreign perform `(A.src)` AND
           B's arm RESUMES through a MATCH on a slice of its param — `(cut (b) t (match (Bytes.slice b 1 2)
           ((Some w) (resume w t)) ((None _x) (resume (Bytes.of (list)) t))))`. Each half already folded ALONE
           (nv1c/nv1d cross-handler bare-resume; nv1e match-arm literal-arg); their conjunction previously
           DECLINED. ROOT (breaker nv1f, confirmed by instrumentation): the OUTER `A` handler's
           `hoist_resumptive_conditional` recursed INTO the inner `B` handle-internal's ARM and Site-2
           distributed B's arm-op `(. B cut)` into the match branches, inverting the arm so the `match` sat in
           the op-slot (`effect_op_of` → None → B's op-map empty → the nested fold declined). Fix: a nested
           handle's arm list is opaque to the outer hoist WHEN no arm reaches an op the OUTER handler discharges
           (those arms fold under their own handler's ctx via the inside-out `reduce_handle`); an arm that DOES
           re-perform the outer effect stays in the hoist. B's arm performs only B → skipped → folds. `A.src` =
           bytes[20,30,40]; `Bytes.slice … 1 2` = [30,40] (Some); `B.cut` resumes that view; `Bytes.len` = 2;
           `(+ 2 12)` = 14.")
  (input
    (do
      (effect A (op src (-> Unit Bytes)))
      (effect B (op cut (-> Bytes Bytes)))
      (def
        (main (: a Int64))
        (handle
          A
          0
          ((src (u) s (resume (Bytes.of #list(20 30 40)) s)))
          (handle
            B
            0
            ((cut
                (b)
                t
                (match
                  (Bytes.slice b 1 2)
                  ((Some w) (resume w t))
                  ((None _x) (resume (Bytes.of #list()) t)))))
            (+ (Bytes.len (B.cut (A.src))) a))))
      (export main)))
  (call main (: 12 Int64))
  (output (: 14 Int64)))

(case
  "a BRANCHING tree walk performs once per leaf at 200-leaf scale"
  (doc
    "Branching self-recursion × per-node performs (the recursive-perform pins are all LINEAR loops):
           `walk` recurses into BOTH children of a user-sum tree (`(+ (walk a) (walk b))`), each LEAF
           performing once in operand position. Over a 200-leaf spine the state must thread through every
           branch junction: the walk sums the leaves (5 + 199·1 = 204) while 200 advances land, and the
           trailing perform reads exactly 200 → 10·204 + 200 = 2240. A state fork or drop at any of the
           199 junctions shifts one of the factors.")
  (input
    (do
      (type Exp (Lit Int64) (Add Exp Exp))
      (effect Cnt (op bump (-> Unit Int64)))
      (def (build (: i Int64) (: e Exp)) (if (= i 0) e (build (- i 1) (Exp.Add e (Exp.Lit 1)))))
      (def
        (walk (: e Exp))
        (match e ((Exp.Lit v) (+ v (* 0 (Cnt.bump)))) ((Exp.Add a b) (+ (walk a) (walk b)))))
      (def
        (main (: n Int64))
        (handle
          Cnt
          0
          ((bump (u) s (resume s (+ s 1))))
          (+ (* 10 (walk (build n (Exp.Lit 5)))) (Cnt.bump))))
      (export main)))
  (call main (: 199 Int64))
  (output (: 2240 Int64))
  (live-objects known-leak))

(case
  "two performs as the two ARGUMENTS of a pure USER function thread the state left-to-right"
  (doc
    "The performs sit in the argument list of a non-primitive, effect-free USER function, whose call
           evaluates its arguments left-to-right before applying — so the two reads are sequenced by the
           call's own argument evaluation, not by an operator or a let. `sub a b = a - b`, `Acc.get : Unit
           -> Int64`, arm `(get (u) s (resume s (+ s 5)))`, seeded 10: `(sub (Acc.get) (Acc.get))` reads the
           first arg as 10 (state → 15) and the second as 15 (state → 20), so `(sub 10 15)` = -5. Pins that
           the fold sequences performs across a user call's ARGUMENT list identically to operator operands
           (had the args not threaded, both would read 10 → 0) — the user-call companion of the operator-
           operand and nested-perform cases, and distinct from the arms that call an effect-free helper on a
           resume RESULT (the performs here are the call's inputs, sequenced at the call site).")
  (input
    (do
      (effect Acc (op get (-> Unit Int64)))
      (def (sub a b) (- a b))
      (def (main) (handle Acc 10 ((get (u) s (resume s (+ s 5)))) (sub (Acc.get) (Acc.get))))
      (export main)))
  (output (: -5 Int64)))

(case
  "a do-sequence of unit-returning performs runs each for effect, then yields the tail value"
  (input
    (do
      (effect Log (op w (-> Int64 Unit)))
      (def (main) (handle Log 0 ((w (n) s (resume unit (+ s n)))) (do (Log.w 3) (Log.w 4) 99)))
      (export main)))
  (doc
    "The side-effect-only sequencing shape: a `do` of two UNIT-returning performs run purely for
           effect, then a tail value. `Log.w : Int64 -> Unit` accumulates its argument into the handler
           state (seeded 0, threads `s + n`); `(do (Log.w 3) (Log.w 4) 99)` performs `Log.w 3` (state 0 → 3)
           then `Log.w 4` (state 3 → 7) — each yields unit, discarded — and the sequence's value is the tail
           `99`. Pins that a chain of unit-op performs threads state in order while their unit results are
           dropped, the essential shape of a compiler pass that EMITS several diagnostics (each advancing an
           accumulator) then returns its result. The handler's threaded total is observed only through the
           state; the handle's value is the body's tail.")
  (output (: 99 Int64)))

(case
  "textually-identical performs are DISTINCT state-advancing reads, not a common subexpression"
  (doc
    "A soundness pin against the backend optimizer's CSE/value-numbering: four TEXTUALLY-IDENTICAL
           `(C.t)` performs are FOUR DISTINCT reads that each advance the handler state, NOT a common
           subexpression to dedup. `(+ (* (C.t) (C.t)) (* (C.t) (C.t)))` seeded 0, arm `(resume s (+ s 1))`:
           evaluated left-to-right, the four reads are 0, 1, 2, 3, so it is `(+ (* 0 1) (* 2 3))` = `(+ 0 6)`
           = 6. Were the compiler to treat the identical `(C.t)` as a common subexpression and compute it
           ONCE (a CSE that ignores effect ordering), the answer would be wrong (e.g. `(* 0 0) + (* 0 0)` =
           0). The effect fold discharges each perform to its own distinct state-advancing read BEFORE the
           optimizer runs, so straight-line CSE never sees a shared effectful node — pinned here at 6.")
  (input
    (do
      (effect C (op t (-> Unit Int64)))
      (def (main) (handle C 0 ((t (u) s (resume s (+ s 1)))) (+ (* (C.t) (C.t)) (* (C.t) (C.t)))))
      (export main)))
  (output (: 6 Int64)))

(case
  "DOMINATOR CSE does not reuse a condition's perform-product in the taken branch"
  (doc
    "The conditional companion of the straight-line CSE pin above, against the backend's DOMINATOR CSE
           (which hoists a subexpression computed in an `if` CONDITION into a branch it dominates). The
           condition and the taken branch each contain the TEXTUALLY-IDENTICAL product `(* (C.t) (C.t))`, but
           they are DISTINCT state-advancing reads — the branch must recompute, NOT reuse the condition's
           value. `C` seeded 1, arm `(resume s (+ s 1))`: the condition `(* (C.t) (C.t))` reads 1 then 2 = 2,
           and `2 > 0` is true; the taken then-branch `(* (C.t) (C.t))` reads 3 then 4 = 12. So the result is
           12. Were dominator CSE to hoist the condition's product and reuse it in the branch (ignoring that
           each `(C.t)` is a distinct effectful read), the branch would wrongly yield 2. Sound because the
           fold discharges every perform to its own distinct read BEFORE the optimizer runs, so no effectful
           node is ever shared for CSE to hoist — pinned at 12 across an `if` this time, not just a
           straight-line spine.")
  (input
    (do
      (effect C (op t (-> Unit Int64)))
      (def
        (main)
        (handle C 1 ((t (u) s (resume s (+ s 1)))) (if (> (* (C.t) (C.t)) 0) (* (C.t) (C.t)) 99)))
      (export main)))
  (output (: 12 Int64)))

(case
  "an if→SELECT-eligible conditional with a performed condition and pure branches stays sound"
  (doc
    "A soundness pin against the backend's if→SELECT conversion (which turns a small trap-free `if`
           into a branchless `select` that evaluates BOTH arms eagerly). The condition performs and the two
           branches are pure scalar values — exactly the shape the conversion targets. `C` seeded 3, arm
           `(resume s (+ s 1))`: the condition `(C.t)` reads 3 (state → 4), `3 < 5` is true, so the pure
           then-branch `10` is the value. Sound because the perform is discharged to a single sequenced read
           in the CONDITION before the optimizer runs — the branches carry no effectful node, so converting
           the `if` to a branchless `select` (eager on both scalar arms) cannot duplicate, reorder, or drop
           the perform. Pinned at 10 — the perform runs exactly once, in the condition, regardless of the
           if/select lowering. (Distinct from the CSE pins: here the concern is the branchless-select
           transform evaluating both arms, not a shared subexpression being hoisted.)")
  (input
    (do
      (effect C (op t (-> Unit Int64)))
      (def (main) (handle C 3 ((t (u) s (resume s (+ s 1)))) (if (< (C.t) 5) 10 20)))
      (export main)))
  (output (: 10 Int64)))

(case
  "a 2-arm match→SELECT with a performed scrutinee and pure arms stays sound"
  (doc
    "The match companion of the if→SELECT pin, against the backend's 2-arm match→SELECT conversion
           (which lowers a small trap-free two-arm `match` to a branchless `select` evaluating both arm
           values eagerly). The SCRUTINEE performs and the two arm bodies are pure scalar values — the shape
           the conversion targets. `C` seeded 7, arm `(resume s (+ s 1))`: the scrutinee `(C.t)` reads 7
           (state → 8), which does not match the `0` arm, so the `_` arm's `200` is the value. Sound because
           the perform is discharged to a single sequenced read in the SCRUTINEE before the optimizer runs —
           the arm bodies carry no effectful node, so converting the match to a branchless `select` (eager on
           both scalar arms) cannot duplicate, reorder, or drop the perform. Pinned at 200 — the perform runs
           exactly once, in the scrutinee, regardless of the match/select lowering. The control (seed 0 so
           the scrutinee reads 0) selects the `0` arm → 100.")
  (input
    (do
      (effect C (op t (-> Unit Int64)))
      (def (main) (handle C 7 ((t (u) s (resume s (+ s 1)))) (match (C.t) (0 100) (_ 200))))
      (export main)))
  (output (: 200 Int64)))

; --- A perform inside an if/match BRANCH threads its state OUT to the continuation after the conditional.
; A branch's state advance is not local to the branch: the code following the conditional must run against
; the branch's POST-state, not the pre-branch state. Because only one branch runs, the state after the
; conditional is a runtime PHI of the branches — realized by distributing the continuation into each
; branch (`(do (if c t e) k)` ≡ `(if c (do t k) (do e k))`), so the conditional ends up in tail position
; where the fold threads correctly. The condition/scrutinee is evaluated exactly once (never duplicated);
; a short-circuit connective is the same shape via its if-desugar. Contrast: a conditional in TAIL position
; (no continuation) and a perform in the CONDITION both already threaded — the gap was specifically a
; branch perform whose advance must flow OUT to a continuation.
(case
  "a perform in a taken if-branch threads its state to the continuation after the if"
  (doc
    "The then-branch performs `Fresh.next` (reads 0, threads 0->1); the continuation `(Fresh.next)`
           after the `if` reads 1. The branch's state advance is NOT lost — it flows out to the code after
           the conditional. `if` in tail position and a perform in the condition both thread already; this
           pins the branch-then-continuation case, realized by lifting the `if` to tail position and
           distributing the continuation into each branch.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (if true (Fresh.next) 99) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a perform in a taken match-arm threads its state to the continuation after the match"
  (doc
    "Same threading via `match`: the `0` arm performs `Fresh.next` (reads 0, threads 0->1); the
           continuation `(Fresh.next)` reads 1. Confirms the phi-out-of-branch threading is in the shared
           conditional fold, not `if`-specific — a `match` arm body is a branch position exactly like an
           `if` branch.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (match 0 (0 (Fresh.next)) (_ 99)) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a performing match SCRUTINEE threads its state into a performing arm body"
  (doc
    "Both the match SCRUTINEE and the selected ARM BODY perform, and the arm's perform reads the state
           the SCRUTINEE's perform advanced — the two-hole shape through a performing `match`. `Ask` seeded
           3, `get` hands back `s` and threads `s - 1`: the scrutinee `(Ask.get)` reads 3 (state -> 2) and
           binds the `n` arm (`3 != 0`); the arm body `(+ n (Ask.get))` performs again, reading the advanced
           state 2, so it is `(+ 3 2)` = 5. Pins that state threaded THROUGH a performing scrutinee reaches a
           performing arm body — the scrutinee is a strict-first position whose effect is sequenced before
           the arm runs, exactly as an operator operand's is. Distinct from the constant-scrutinee arm-thread
           case above (there the scrutinee is the literal `0`; here it performs).")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          Ask
          3
          ((get (u) s (resume s (- s 1))))
          (match (Ask.get) (0 (Ask.get)) (n (+ n (Ask.get))))))
      (export main)))
  (output (: 5 Int64)))

(case
  "a match arm's DESTRUCTURED payload is the argument to a perform in that arm's body"
  (doc
    "A match arm destructures a sum constructor, binding its payload, and that BOUND VALUE is the
           argument to a perform in the arm body — the binder-into-perform-argument shape. The scrutinee
           `(Some 5)` is a pure literal, so the `(Some n)` arm binds `n = 5` and its body `(Ctr.tick n)`
           performs `Ctr.tick` with that bound payload. `Ctr.tick : Int64 -> Int64`, arm `(tick (d) s (resume
           (+ d s) (+ s 1)))`, seeded 100: `(Ctr.tick 5)` = `5 + 100` = 105. Pins that a value bound by a
           constructor pattern in a match arm flows correctly as a perform's argument (the arm binder is in
           scope for the perform, and the fold threads the handler state through it) — distinct from the
           performing-scrutinee case above (there the scrutinee performs and the arm reads STATE; here the
           scrutinee is pure and the arm feeds its BOUND payload into the op).")
  (input
    (do
      (effect Ctr (op tick (-> Int64 Int64)))
      (def
        (main)
        (handle
          Ctr
          100
          ((tick (d) s (resume (+ d s) (+ s 1))))
          (match (Some 5) ((Some n) (Ctr.tick n)) (None 0))))
      (export main)))
  (output (: 105 Int64)))

(case
  "the else-branch of an if threads a performed state to the continuation"
  (doc
    "The else-branch (taken, cond false) performs once (reads 0, threads 0->1); the continuation reads
           1. Pins that BOTH arms thread out, not just the then-arm — the distribution wraps the continuation
           into each branch.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (if false 99 (Fresh.next)) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a short-circuit connective threads a branch perform to the continuation"
  (doc
    "`(and (= (Fresh.next) 0) (= (Fresh.next) 1))` desugars to `(if (= (Fresh.next) 0) (= (Fresh.next)
           1) false)`; both reads happen (0, then 1), threading 0->2. The continuation `(Fresh.next)` reads
           2. The connective's rhs is a branch (runs only on the taken path), so its perform's advance must
           flow out to the continuation exactly as an explicit `if` branch's does — even though the condition
           itself performs (the condition threads, then the branch, then the distributed continuation).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (and (= (Fresh.next) 0) (= (Fresh.next) 1)) (Fresh.next))))
      (export main)))
  (output (: 2 Int64)))

(case
  "branches performing DIFFERENT counts each thread their own post-state to the continuation"
  (doc
    "The two branches advance the state by different amounts — the then-branch reads once (0->1), the
           else-branch reads twice (0->1->2). With cond true the then-branch runs, so the continuation reads
           1; the continuation is threaded independently through each branch's own post-state, not a single
           merged one. Pins the phi is per-branch: the distributed continuation sees whichever branch ran.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (do (if true (Fresh.next) (do (Fresh.next) (Fresh.next))) (Fresh.next))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a branch perform under two nested handlers threads the inner state to the continuation"
  (doc
    "The branch performs the INNER effect `A` (reads 0, threads 0->1); the continuation `(A.an)` reads
           1. The outer handler `B` is present but unperformed. Pins that the branch-to-continuation
           threading composes with nested handlers — the distribution preserves each effect's own state
           slot, so the inner effect's branch advance still reaches the continuation under the outer fold.")
  (input
    (do
      (effect A (op an (-> Unit Int64)))
      (effect B (op bn (-> Unit Int64)))
      (def
        (main)
        (handle
          B
          100
          ((bn (u) t (resume t (+ t 1))))
          (handle A 0 ((an (u) s (resume s (+ s 1)))) (do (if true (A.an) 0) (A.an)))))
      (export main)))
  (output (: 1 Int64)))

(case
  "a handler accumulates into a list and a read-out operation reads it back"
  (doc
    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges and #A Handler Evaluates To The Value Of Its Body: `Diag` declares two operations —
           `emit : Int64 -> Unit` (record a diagnostic) and `collect : Unit -> (List Int64)` (read the
           accumulated diagnostics). The handler is seeded with the empty list; each `(Diag.emit code)`
           resumes with `unit` and threads `(List.push s code)` forward, accumulating `(list 201 210)`;
           then `(Diag.collect)` reads the accumulated list back — its arm `(resume s s)` hands the state
           out as the operation's value and threads it unchanged. Because the read-out is an ORDINARY
           OPERATION, the handler needs no separate return clause: the body pulls the accumulator into its
           own value by performing `collect`, and the handle evaluates to that body value `(list 201 210)`.
           This is the compiler's diagnostics idiom as a real accumulator (the earlier record-and-continue
           `Diag.emit` that resumed unit and discarded the code was the stateless placeholder for it), and
           it needs the list-growth capability to build the accumulator.")
  (input
    (do
      (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64))))
      (def
        (main)
        (handle
          Diag
          #list()
          ((emit (code) s (resume unit (List.push s code))) (collect (u) s (resume s s)))
          (do (Diag.emit 201) (Diag.emit 210) (Diag.collect))))
      (export main)))
  (output (: #list(201 210) (List Int64)))
  (live-objects known-leak))

(case
  "a handler threads a SET as its state — the seen-set idiom, deduping across performs"
  (doc
    "The Set analogue of the list-accumulator handler: the threaded state is a SET (a `seen`/`visited`
           set), and an `add` operation inserts into it while a `count` operation reads its size. Because a
           Set DEDUPES, two `(Seen.add 2)` performs of the same key leave the set unchanged after the first.
           `Seen.add : Int64 -> Int64`, arm `(add (k) m (resume k (Set.insert m k)))`; `Seen.count : Unit ->
           Int64`, arm `(count (u) m (resume (Set.len m) m))`. Seeded `{1}`: `(Seen.add 2)` → `{1, 2}` (2
           elements), `(Seen.add 2)` inserts the duplicate 2 → still `{1, 2}`, and `(Seen.count)` reads
           `Set.len {1,2}` = 2. Pins that a handler state slot carries a persistent SET through the fold —
           the arm reads it (`Set.len`) and rebuilds it (`Set.insert`) per performance, and the set's
           set-semantics (dedup) hold across the threaded reads — the visited-set idiom a graph/AST walk
           needs. (wasm: the rust target declines — it lacks the value-heap/Set emission the component-model
           backend has, the same backend-parity gap as the list-state cases, not an effects-fold limitation.)")
  (input
    (do
      (effect Seen (op add (-> Int64 Int64)) (op count (-> Unit Int64)))
      (def
        (main)
        (handle
          Seen
          #set(1)
          ((add (k) m (resume k (Set.insert m k))) (count (u) m (resume (Set.len m) m)))
          (let ((a (Seen.add 2))) (let ((b (Seen.add 2))) (Seen.count)))))
      (export main)))
  (output (: 2 Int64)))

(case
  "a handler threads a TUPLE of two heaps as state with different ops touching different halves"
  (doc
    "The SPLIT-state idiom: state = (tuple (list) Map.empty), note touches the LIST half and
           tag the MAP half — each arm PROJECTS its half ((. st 0)/(. st 1)), updates it, rebuilds
           the tuple; the untouched half's handle threads through unchanged, and tag reads List.len
           of the OTHER half so the halves must stay in sync. (Arm bodies use projections rather
           than match — a handler arm whose body is a match trips the ML-printer arm-extent
           ambiguity, filed separately.)")
  (input
    (do
      (effect S (op note (-> Int64 Int64)) (op tag (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          #tuple(#list() Map.empty)
          ((note
              (v)
              st
              (let ((lg2 (List.push (. st 0) v))) (resume (List.len lg2) #tuple(lg2 (. st 1)))))
            (tag
              (k)
              st
              (let
                ((ix2 (Map.insert (. st 1) k (List.len (. st 0)))))
                (let
                  ((got (match (Map.lookup ix2 k) ((Some x) x) ((None _u) -1))))
                  (resume got #tuple((. st 0) ix2))))))
          (do
            (def r1 (S.note 10))
            (def t1 (S.tag 5))
            (def r2 (S.note n))
            (def t2 (S.tag 5))
            (+ (* r1 1000) (+ (* t1 100) (+ (* r2 10) t2))))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 1122 Int64))
  (call main (: 0 Int64))
  (output (: 1122 Int64)))

(case
  "a LIST built in the handle body from perform results crosses the handle exit live"
  (doc
    "The collect pin's list exits via STATE; this one is constructed IN the body from perform
           RESULTS interleaved with a runtime param — element evaluation interleaves with
           perform/resume round-trips, and the finished heap value survives handler teardown.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (sum-l (: l (List Int64)) (: acc Int64))
        (match l (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: n Int64))
        (do
          (def xs (handle Ctr 0 ((tick (_u) c (resume c (+ c 1)))) #list((Ctr.tick) (Ctr.tick) n)))
          (+ (* (sum-l xs 0) 10) (List.len xs))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 63 Int64))
  (call main (: 0 Int64))
  (output (: 13 Int64))
  (live-objects 0))

(case
  "a MAP keyed by perform results in the handle body crosses the exit and looks up by those keys"
  (doc
    "The CHAMP composition: the map's KEYS are perform results — insert-arg evaluation
           interleaves with perform/resume, the champ hash runs on resumed values, and the map is
           looked up by those keys post-exit.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (get (: m (Map Int64 Int64)) (: k Int64))
        (match (Map.lookup m k) ((Some v) v) ((None _u) -1)))
      (def
        (main (: n Int64))
        (do
          (def
            m
            (handle
              Ctr
              0
              ((tick (_u) c (resume c (+ c 1))))
              (Map.insert (Map.insert Map.empty (Ctr.tick) 10) (Ctr.tick) n)))
          (+ (* (get m 0) 10) (get m 1))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 120 Int64))
  (call main (: 0 Int64))
  (output (: 100 Int64)))

(case
  "a rope accumulates across perform/resume boundaries and content-checks at the exit"
  (doc
    "The strings member: a recursive builder concats a chunk per perform, each chunk selected
           by the resume value — the accumulating rope survives N suspension boundaries, and the
           handler SEED shifts which letters are picked (content-checked at exit).")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (pick (: k Int64)) (match (& k 3) (0 "a") (1 "b") (2 "c") (_ "d")))
      (def
        (go (: i Int64) (: acc String))
        (if (= i 0) acc (go (- i 1) (String.concat acc (pick (Ctr.tick))))))
      (def
        (main (: n Int64))
        (do
          (def s (handle Ctr n ((tick (_u) c (resume c (+ c 1)))) (go 3 "")))
          (+ (* (String.byte-len s) 10) (if (= s "abc") 1 0))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 31 Int64))
  (call main (: 1 Int64))
  (output (: 30 Int64)))

(case
  "a handler threads a MAP as its state — a key-value store deduping keys across performs"
  (doc
    "The Map analogue of the Set-state seen-set: the threaded state is a MAP (a key→value store), and a
           `put` operation inserts a key while a `count` operation reads its size — exercising the CHAMP
           key→value path through the effect fold, distinct from the key-only Set case. Because a Map keys
           uniquely, `put`ting the same key twice leaves one entry. `Store.put : Int64 -> Unit`, arm `(put (k)
           m (resume unit (Map.insert m k k)))`; `Store.count : Unit -> Int64`, arm `(count (u) m (resume
           (Map.len m) m))`. Seeded empty: `(Store.put 1)` → `{1:1}`, `(Store.put 2)` → `{1:1, 2:2}`,
           `(Store.put 1)` re-inserts key 1 → still two keys, and `(Store.count)` reads `Map.len` = 2. Pins
           that a handler state slot carries a persistent MAP through the fold across MULTIPLE performs — the
           arm reads it (`Map.len`) and rebuilds it (`Map.insert`) per performance, and the map's key-dedup
           holds across the threaded reads (the keyed-store idiom, and a guard that a Map.lookup/insert CSE
           change cannot regress a Map threaded as effect state). (wasm: the rust target declines — it lacks
           the value-heap/Map emission the component-model backend has, the same backend-parity gap as the
           list-state and Set-state cases, not an effects-fold limitation.)")
  (input
    (do
      (effect Store (op put (-> Int64 Unit)) (op count (-> Unit Int64)))
      (def
        (main)
        (handle
          Store
          (Map.empty)
          ((put (k) m (resume unit (Map.insert m k k))) (count (u) m (resume (Map.len m) m)))
          (do (Store.put 1) (Store.put 2) (Store.put 1) (Store.count))))
      (export main)))
  (output (: 2 Int64)))

(case
  "sequenced memoize helpers with a local let thread the Map-state out-state (the memo-spine shape)"
  (doc
    "The real memoized-query-DB spine (the shape compiler-ml's #4 hardening needs): a cross-function
           helper `store(k)` that BINDS A LOCAL `let vv = k*10` and performs `Db.put((k, vv))` returning `vv`
           — the memoize combinator's on-miss arm — called TWICE in a `do` SEQUENCE before a final read. The
           first `(store 3)` is a NON-FINAL do item: `put`'s next-state (`Map.insert m k vv`) threads FORWARD
           to `(store 5)` and the trailing `(Db.tot)`, and it references `vv`, which the helper's `let` binds
           LOCAL to the first item. Without the `do`-arm LET-LIFT this leaked `CDZ0101 unbound vv` (the
           out-state spliced past the `let` scope); the fix lifts `(let ((vv …)) lbody)` to wrap the whole
           continuation so `vv` stays in scope. Both stores insert their key → `Db.tot` reads `Map.len` = 2.
           Pins that a memoize helper (local let + get/put) composes in a sequence — the substrate for an
           effect-based salsa-style Db (a FINAL-position such call always worked; the sequenced case is the fix).")
  (input
    (do
      (effect Db (op put (-> (Tuple Int64 Int64) Unit)) (op tot (-> Unit Int64)))
      (def (store (: k Int64)) (let ((vv (* k 10))) (do (Db.put #tuple(k vv)) vv)))
      (def
        (main)
        (handle
          Db
          (Map.empty)
          ((tot (u) s (resume (Map.len s) s))
            (put (kv) s (match kv (#tuple(k v) (resume unit (Map.insert s k v))))))
          (do (store 3) (store 5) (Db.tot))))
      (export main)))
  (output (: 2 Int64)))

; The memoize COMBINATOR spine `demand`: a cross-function helper whose on-MISS arm performs `Db.put` (a
; state-advancing write) and whose on-HIT arm returns the cached value — `(match (Db.get k) ((Some v) v)
; ((None u) (do (Db.put (k, compute)) compute)))` — threaded over a `Map`-state handler. The helper's
; branch `put` must thread its Map-state advance to the CALLER's continuation (a later `Db.get` of the same
; key, or a sibling `demand`), or the later read misses and re-computes. These pin the three facets: a
; let-bound demand then a re-read; the pre-populated HIT branch; and two sibling demands of one key.
(case
  "a memoize helper's on-miss put threads to a later read of the same key"
  (doc
    "`(let ((a (demand 5 25))) (match (Db.get 5) ((Some v) (+ a v)) ((None u) 99)))` — `demand 5 25`
           misses, performs `Db.put (5, 25)` and returns 25; the later `(Db.get 5)` must observe that write
           and HIT (Some 25) → 25 + 25 = 50. A single-return path that dropped the branch `put`'s out-state
           at the call boundary would leave the later get missing → the 99 arm (a silent wrong value).")
  (input
    (do
      (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
      (def
        (demand (: k Int64) (: compute Int64))
        (match
          (Db.get k)
          ((Option.Some v) v)
          ((Option.None u) (do (Db.put #tuple(k compute)) compute))))
      (def
        (run-then-get)
        (handle
          Db
          (Map.empty)
          ((get (k) s (resume (Map.lookup s k) s))
            (put (kv) s (match kv (#tuple(k v) (resume unit (Map.insert s k v))))))
          (let
            ((a (demand 5 25)))
            (match (Db.get 5) ((Option.Some v) (+ a v)) ((Option.None u) 99)))))
      (export run-then-get)))
  (call run-then-get)
  (output (: 50 Int64)))

(case
  "a memoize helper on a pre-populated key takes the hit branch and does not re-put"
  (doc
    "The HIT-branch facet: the body pre-`put`s (5, 25), so `demand 5 99`'s inner `(Db.get 5)` HITS
           (Some 25, the first-built arm) and returns 25 WITHOUT re-putting 99; the distributed continuation
           `(match (Db.get 5) ((Some w) (+ a w)) …)` reads a=25, w=25 → 50. Pins the taken-hit branch of the
           let-init conditional distribution (the miss facet is the case above).")
  (input
    (do
      (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
      (def
        (demand (: k Int64) (: compute Int64))
        (match
          (Db.get k)
          ((Option.Some v) v)
          ((Option.None u) (do (Db.put #tuple(k compute)) compute))))
      (def
        (run-hit)
        (handle
          Db
          (Map.empty)
          ((get (k) s (resume (Map.lookup s k) s))
            (put (kv) s (match kv (#tuple(k v) (resume unit (Map.insert s k v))))))
          (do
            (Db.put #tuple(5 25))
            (let
              ((a (demand 5 99)))
              (match (Db.get 5) ((Option.Some w) (+ a w)) ((Option.None u) 99))))))
      (export run-hit)))
  (call run-hit)
  (output (: 50 Int64)))

(case
  "two sibling memoize demands of one key: the first's put threads to the second"
  (doc
    "The sibling-call facet: `(let ((a (demand 5 25)) (b (demand 5 999))) (+ a b))` — the first demand
           fills (5, 25) and the SECOND, of the same key, must HIT the first's write (get → Some 25 → 25)
           rather than re-compute 999 → 25 + 25 = 50. Pins that a helper's branch `put` out-state threads to
           a LATER SIBLING binding's inlined demand (a single-return drop would give 25 + 999 = 1024).")
  (input
    (do
      (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
      (def
        (demand (: k Int64) (: compute Int64))
        (match
          (Db.get k)
          ((Option.Some v) v)
          ((Option.None u) (do (Db.put #tuple(k compute)) compute))))
      (def
        (run-twice)
        (handle
          Db
          (Map.empty)
          ((get (k) s (resume (Map.lookup s k) s))
            (put (kv) s (match kv (#tuple(k v) (resume unit (Map.insert s k v))))))
          (let ((a (demand 5 25)) (b (demand 5 999))) (+ a b))))
      (export run-twice)))
  (call run-twice)
  (output (: 50 Int64)))

(case
  "a RECURSIVE effectful walk accumulates into a list-state handler"
  (doc
    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges with the state on the VALUE HEAP and the performer RECURSIVE — the compiler's real
           diagnostics shape: a recursive program walk emitting into a `list<diagnostic>` threaded as
           handler state. `walk` performs `(Diag.emit n)` at each step of a recursive descent and reads
           the accumulator back with `(Diag.collect)` at the base; the handler seeds `(list)` and threads
           `(List.push s v)`, so `(walk 3)` accumulates `(list 3 2 1)`, whose length is 3. This is the
           combination — recursion AND a runtime-compound handler state — that the effect-context
           monomorphization must lower as a real specialized function (its state lives on the value heap,
           threaded as trailing params/returns), not only the self-contained scalar case. `List.len`
           makes `main` return a scalar so the whole program is the runtime-scalar path.")
  (input
    (do
      (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64))))
      (def (walk n) (if (< n 1) (Diag.collect unit) (do (Diag.emit n) (walk (- n 1)))))
      (def
        (main)
        (handle
          Diag
          #list()
          ((emit (v) s (resume unit (List.push s v))) (collect (u) s (resume s s)))
          (List.len (walk 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a recursive effectful walk accumulates into a STRING-state handler"
  (doc
    "The rope-STRING analogue of the list-state accumulator above: the handler's threaded state is a
           heap STRING built with `String.concat` across a recursive descent, exercising the value-heap
           runtime's rope path (canonicalized at each construction site) rather than a list. `Log` declares
           `emit : String -> Unit` (append a piece) and `dump : Unit -> String` (read the accumulator);
           the handler seeds `\"\"` and each `(Log.emit \"x\")` resumes `unit` and threads `(String.concat
           s m)`, so a recursive `walk` performing three emits builds `\"xxx\"`, whose byte length is 3.
           `String.byte-len` makes `main` a runtime-scalar so the whole program stays on the scalar path.
           Pins that a handler's threaded state may be a heap STRING carried through the recursive-effectful
           specialization — the String-STATE companion of the list-state accumulator and the String-RESULT
           resume-value case, guarding the effect-mechanism × rope-runtime seam. (wasm: the rust target
           declines — it lacks the value-heap/String emission the component-model backend has, the same
           backend-parity gap as the list-state and String-result cases, not an effects-fold limitation.)")
  (input
    (do
      (effect Log (op emit (-> String Unit)) (op dump (-> Unit String)))
      (def (walk (: n Int64)) (if (= n 0) (Log.dump) (do (Log.emit "x") (walk (- n 1)))))
      (def
        (main)
        (handle
          Log
          ""
          ((emit (m) s (resume unit (String.concat s m))) (dump (u) s (resume s s)))
          (String.byte-len (walk 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "a recursive effectful walk BUILDS a list as its return value, one fresh element per step"
  (doc
    "The list is the recursion's RETURN VALUE (not handler state, unlike the accumulator case above):
           a recursive `build` reads a fresh index and CONSES it onto the list the rest of the walk returns.
           The perform is bound in a `let` BEFORE the self-call — `(let ((v (Idx.next))) ((. List push)
           (build (- n 1)) v))` — so `v` reads PRE-recursion state (the sound ordering; a perform AFTER the
           self-call would read the recursion's out-state, which the single-return specialization cannot
           carry and correctly declines). `Idx` seeded 1 threads `s + 1`, so the three steps read 1, 2, 3 —
           three fresh elements — and `(List.len (build 3))` = 3. Pins that effect-context specialization
           lowers a list-BUILDING recursive walk (the shape of a compiler pass collecting fresh names into a
           list as it descends), with the built list crossing to a `List.len` readout via the value heap.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def
        (build (: n Int64))
        (if (= n 0) #list() (let ((v (Idx.next))) (List.push (build (- n 1)) v))))
      (def (main) (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (List.len (build 3))))
      (export main)))
  (output (: 3 Int64)))

(case
  "an effectful walk over an INPUT list combines each element with a fresh perform"
  (doc
    "The input-driven map (the build case above is DEPTH-driven — the recursion count decides the
           output; here an INPUT list drives the walk and each of ITS elements combines with a perform):
           `tag-all` reads `xs[i]` and pushes `100·(Idx.next) + v` — pairing the element with a fresh id —
           recursing until `List.at` misses. Seeded 1: elements 10/20/n pick up ids 1/2/3; the readout
           encodes `10·len + tagged[2]` = 30 + (100·3 + 30) = 360 at n = 30. Pins the tag-each-element
           idiom (a compiler numbering its input nodes): per-element state advances interleave with
           per-element heap reads, and the tagged list escapes the handle.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def
        (tag-all (: xs (List Int64)) (: i Int64) (: acc (List Int64)))
        (match
          (List.at xs i)
          ((Some v) (tag-all xs (+ i 1) (List.push acc (+ (* 100 (Idx.next unit)) v))))
          ((None u) acc)))
      (def
        (main (: n Int64))
        (let
          ((tagged
              (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (tag-all #list(10 20 n) 0 #list()))))
          (+ (* 10 (List.len tagged)) (match (List.at tagged 2) ((Some v) v) ((None u) -1)))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 360 Int64))
  (live-objects known-leak))

(case
  "a closure capturing a handler-computed VALUE escapes the handle and applies outside"
  (doc
    "The escaping-closure acceptance witness (breaker finding, fixed): the perform runs INSIDE the
           handle (`base = (Cfg.get unit)`), the closure captures the resulting plain VALUE — performing
           nothing itself — and escapes as the handle's result, applied OUTSIDE (2+40 = 42). The escape
           analysis must distinguish 'a perform occurred in the body that built this closure' from 'this
           closure performs' (it once rejected CDZ0401 on exactly this shape); the correct-reject twin —
           a closure whose BODY performs escaping — stays rejected elsewhere.")
  (input
    (do
      (effect Cfg (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (let
          ((f
              (handle
                Cfg
                n
                ((get (u) s (resume s s)))
                (let ((base (Cfg.get unit))) (fn ((: x Int64)) (+ x base))))))
          (f 40)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 42 Int64)))

(case
  "TWO closures escaping one handle carry DISTINCT captured state reads"
  (doc
    "The two-capture composition: both closures capture different reads of the SAME advancing
           counter (a = seed, b = seed+1) and escape in one tuple; applied outside, each must see ITS
           read — f(100) = 100+3, g(10) = 10·4 → 143. An environment that shared one capture slot (or
           re-read the final state for both) collapses a and b.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (match
          (handle
            Ctr
            n
            ((next (u) s (resume s (+ s 1))))
            #tuple((let ((a (Ctr.next unit))) (fn ((: x Int64)) (+ x a)))
              (let ((b (Ctr.next unit))) (fn ((: x Int64)) (* x b)))))
          (#tuple(f g) (+ (f 100) (g 10)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 143 Int64)))

(case
  "a closure built OUTSIDE a handler is applied INSIDE it beside performs"
  (doc
    "The inbound direction: a pure closure constructed before the handle is applied within the
           handle body with a PERFORM as its argument — `(f (Ctr.next unit))` reads the seed 4 through
           the ×10 capture, the second perform reads 5 → 45. The closure's environment predates the
           handler frame; application under the handler must not confuse the capture with handler state.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (def (mk (: k Int64)) (fn ((: x Int64)) (* x k)))
      (def
        (main (: n Int64))
        (let
          ((f (mk 10)))
          (handle Ctr n ((next (u) s (resume s (+ s 1)))) (+ (f (Ctr.next unit)) (Ctr.next unit)))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 45 Int64)))

; A closure returned from a handle that captures a perform result RESOLVED BY THAT INNER HANDLE closes over
; the VALUE, not the perform expression — applying it later must reuse the captured value, never re-perform
; (which the enclosing handler would re-home, giving a wrong answer). `base` is bound to `(Ctr.tick)` under
; an inner `handle Ctr 50`, so `base = 50`; the returned `(fn (x) (+ x base))` applied as `(f 3)` is 53 —
; whether or not an outer `handle Ctr 5` wraps the application. These pin capture-the-value across the four
; shapes: a direct closure under an outer handler, the no-outer-handler twin, two captures across nested
; lets, and a curried closure.
(case
  "a closure capturing an inner-handled perform result closes over the value under an outer handler"
  (doc
    "`base` = `(Ctr.tick)` under the INNER `handle Ctr 50` is 50, captured by the returned `(fn (x)
           (+ x base))`. Applied `(f 3)` under an OUTER `handle Ctr 5`, the result is 3 + 50 = 53 — the
           capture is the inner-handled VALUE 50, not the perform expression (which the outer handler would
           re-home to 5, giving a wrong 8). Pins that a closure over an inner-handled result carries the
           value across the handle boundary.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (let
            ((f
                (handle
                  Ctr
                  50
                  ((tick (u) s (resume s (+ s 1))))
                  (let ((base (Ctr.tick))) (fn ((: x Int64)) (+ x base))))))
            (f 3))))
      (export main)))
  (call main)
  (output (: 53 Int64)))

(case
  "a closure capturing an inner-handled perform result needs no outer handler"
  (doc
    "The no-outer-handler twin: the same inner-handled captured closure applied with NO enclosing
           handler. Discharging the inner handle closes the closure over `base = 50`, so `(f 3)` = 53 with
           no outer handler needed (an unhomed inner perform would over-decline CDZ0401). Pins that the
           inner handle fully resolves the captured result.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (let
          ((f
              (handle
                Ctr
                50
                ((tick (u) s (resume s (+ s 1))))
                (let ((base (Ctr.tick))) (fn ((: x Int64)) (+ x base))))))
          (f 3)))
      (export main)))
  (call main)
  (output (: 53 Int64)))

(case
  "a closure captures two inner-handled results across nested lets"
  (doc
    "The nested-let sibling: a closure buried at the end of NESTED lets captures TWO inner-handled
           perform results — `(let ((a (Ctr.tick))) (let ((b (Ctr.tick))) (fn (x) (+ x (+ a b)))))`. Under
           the inner `handle Ctr 50`, a = 50 and b = 51 (threaded), so the closure is `(fn (x) (+ x 101))`;
           applied `(f 3)` under an outer `handle Ctr 5` = 104. Pins that captures across a let-chain (the
           outer capture referenced by a closure inside the inner let) close over their values.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (let
            ((f
                (handle
                  Ctr
                  50
                  ((tick (u) s (resume s (+ s 1))))
                  (let ((a (Ctr.tick))) (let ((b (Ctr.tick))) (fn ((: x Int64)) (+ x (+ a b))))))))
            (f 3))))
      (export main)))
  (call main)
  (output (: 104 Int64)))

(case
  "a curried closure capturing an inner-handled result closes over the value across both applications"
  (doc
    "The curry sibling: the inner handle returns `(fn (a) (fn (b) (+ (+ a b) base)))` capturing the
           inner-handled `base = (Ctr.tick) = 50`. Applied `((f 3) 4)` under an outer `handle Ctr 5`, the
           capture stays the value 50 across BOTH the partial application and the residual, never
           re-performed: 3 + 4 + 50 = 57. Pins the value-capture composes with currying.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          5
          ((tick (u) s (resume s (+ s 1))))
          (let
            ((f
                (handle
                  Ctr
                  50
                  ((tick (u) s (resume s (+ s 1))))
                  (let ((base (Ctr.tick))) (fn ((: a Int64)) (fn ((: b Int64)) (+ (+ a b) base)))))))
            ((f 3) 4))))
      (export main)))
  (call main)
  (output (: 57 Int64)))

(case
  "the heap list a handle BUILDS escapes the handle and is consumed outside it"
  (doc
    "The handle's VALUE is a heap list, and it flows OUT of the handle into the enclosing scope. Unlike
           the case above (which reads `List.len` INSIDE the handle body), here the `handle` expression is
           bound to `xs` in an enclosing `let` and consumed AFTER the handle: `(let ((xs (handle Idx 1 …
           (build 3)))) ((. List len) xs))`. So the effect-built list is the handle's result value, lives on
           the value heap, and is a first-class value the surrounding computation reads — the essential shape
           of a compiler PHASE that runs an effectful walk and hands its collected result (a list of fresh
           names / diagnostics) to the next phase. `Idx` seeded 1 threads `s + 1`, the walk collects three
           elements, and the outside `List.len xs` = 3. Pins that a handle's heap-value result crosses the
           handle boundary intact.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def
        (build (: n Int64))
        (if (= n 0) #list() (let ((v (Idx.next))) (List.push (build (- n 1)) v))))
      (def
        (main)
        (let ((xs (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (build 3)))) (List.len xs)))
      (export main)))
  (output (: 3 Int64)))

(case
  "an effect-built heap list bound in a let is USED TWICE and retained across both uses"
  (doc
    "The DUP / retain shape for an effect-built heap value: the list a handle builds is bound to `xs`
           and consumed MORE THAN ONCE (`(+ ((. List len) xs) ((. List len) xs))`), so the binding is a
           shared owner the first use must NOT free out from under the second. Unlike the escapes-and-
           consumed-ONCE case above, this exercises the Perceus dup — a multiply-used heap binding must be
           RETAINED, not consumed by its first reader. `Idx` seeded 1, `build 3` collects three elements, and
           `len xs + len xs` = `3 + 3` = 6 (a use-after-free from the first `List.len` consuming `xs` would
           read a freed handle / wrong length). Pins that an effect-built heap value bound and used twice is
           reference-managed correctly across the uses — the effects × dup-retain composition. (wasm: rust
           declines — value-heap/List emission parity gap, not the effects fold.)")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def
        (build (: n Int64))
        (if (= n 0) #list() (let ((v (Idx.next))) (List.push (build (- n 1)) v))))
      (def
        (main)
        (handle
          Idx
          1
          ((next (u) s (resume s (+ s 1))))
          (let ((xs (build 3))) (+ (List.len xs) (List.len xs)))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a STRING-result effect op resumes with a string that folds through a concat"
  (doc
    "The effect fold's value column carries a heap STRING: an operation returning `String` is resumed
           with a string literal, and that performed value flows into `String.concat` in the continuation.
           `Env.name : Unit -> String`, arm `(name (u) s (resume \"cdz\" s))`, so `(Env.name)` yields the heap
           string `\"cdz\"`; the body `(String.concat (Env.name) \"!\")` appends `\"!\"`, giving `\"cdz!\"`.
           Pins that a performed String resume value threads through the fold like any scalar and composes
           under a heap string operation — the String companion of the tuple/list value-column cases,
           exercising the value-heap runtime for the resume value rather than an immediate. (wasm: the rust
           target declines — it lacks the value-heap/String emission the component-model backend has, the
           same backend-parity gap as the list-building cases, not an effects-fold limitation.)")
  (input
    (do
      (effect Env (op name (-> Unit String)))
      (def (main) (handle Env 0 ((name (u) s (resume "cdz" s))) (String.concat (Env.name) "!")))
      (export main)))
  (output (: "cdz!" String)))

(case
  "a recursive walk threads TWO effects at once — a fresh-index counter and a running total"
  (doc
    "The full compiler-pass shape: ONE recursive walk that reads a fresh index from `Idx` AND folds a
           running total through `Tot`, under TWO nested handlers, each threading its own state independently.
           `walk` at each step reads `v = (Idx.next)` (a fresh index) then `(Tot.add v)` (accumulate it), then
           recurses; at the base it reads back the total `(Tot.total)`. `Idx` seeded 1 threads `s + 1` so the
           three indices are 1, 2, 3; `Tot` seeded 0 threads `t + d` so the total is `1 + 2 + 3` = 6. Both
           states are live on the recursion stack simultaneously and thread through DISTINCT slots (the walk
           specializes once against the merged two-effect context — a single shared slot per effect would
           clobber on re-entry). Pins that effect-context monomorphization threads more than one effect
           through one recursive walk — a fresh-name counter AND a diagnostics/total accumulator — the exact
           combination a self-hosting compiler pass needs.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (effect Tot (op add (-> Int64 Int64)) (op total (-> Unit Int64)))
      (def
        (walk (: n Int64))
        (if (= n 0) (Tot.total unit) (let ((v (Idx.next))) (let ((u (Tot.add v))) (walk (- n 1))))))
      (def
        (main)
        (handle
          Tot
          0
          ((add (d) t (resume t (+ t d))) (total (uu) t (resume t t)))
          (handle Idx 1 ((next (u) s (resume s (+ s 1)))) (walk 3))))
      (export main)))
  (output (: 6 Int64)))

(case
  "one effect's result flows as the ARGUMENT to a DIFFERENT effect's op under nested handlers"
  (doc
    "The cross-effect, non-recursive companion of the two-effects-in-one-walk case: the result of an
           INNER-handled effect's perform is the very argument an OUTER-handled effect's perform consumes —
           `(Dst.put (Src.get))`. The argument `(Src.get)` is discharged by the inner `Src` handler first
           (advancing the Src state), and its result feeds `Dst.put`, discharged by the outer `Dst` handler
           (advancing the Dst state independently). `Src.get : Unit -> Int64` seeded 5, arm `(get (u) s
           (resume s (+ s 1)))` → reads 5; `Dst.put : Int64 -> Int64` seeded 100, arm `(put (v) t (resume (+
           v t) (+ t 10)))` → `(Dst.put 5)` = `5 + 100` = 105. Pins that a value produced by discharging one
           effect crosses into a DIFFERENT effect's operation as its argument, each threading its own handler
           state through a distinct slot — the two folds compose along the data dependency without sharing or
           clobbering state (distinct from the SAME-effect nested-perform-argument case, where one handler's
           single state slot threads both reads).")
  (input
    (do
      (effect Src (op get (-> Unit Int64)))
      (effect Dst (op put (-> Int64 Int64)))
      (def
        (main)
        (handle
          Src
          5
          ((get (u) s (resume s (+ s 1))))
          (handle Dst 100 ((put (v) t (resume (+ v t) (+ t 10)))) (Dst.put (Src.get)))))
      (export main)))
  (output (: 105 Int64)))

(case
  "a handle's TUPLE value pairing a scalar with a built list escapes and is destructured"
  (doc
    "The handle's VALUE is a COMPOUND — a tuple pairing a scalar with an effect-built heap list — and
           the whole tuple escapes the handle to be destructured outside. `(handle Idx 1 … (tuple 42 (build
           2)))` evaluates to `(42, [2,1])`; bound to `r` in an enclosing `let`, `(+ (. r 0) ((. List len)
           (. r 1)))` reads the scalar 42 and the built list's length 2 → 44. Pins that a handle can return a
           MIXED compound (a scalar beside a heap value) as its result and hand it whole to the enclosing
           computation — a phase returning both a summary count and its collected list.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def
        (build (: n Int64))
        (if (= n 0) #list() (let ((v (Idx.next))) (List.push (build (- n 1)) v))))
      (def
        (main)
        (let
          ((r (handle Idx 1 ((next (u) s (resume s (+ s 1)))) #tuple(42 (build 2)))))
          (+ (. r 0) (List.len (. r 1)))))
      (export main)))
  (output (: 44 Int64)))

(case
  "an effect-built NESTED-compound value escapes the handle and a nested projection reads through it"
  (doc
    "The nested-compound escape: the handle's VALUE is a TUPLE OF TUPLES built from performed reads,
           it escapes to an enclosing `let`, and a NESTED projection `(. (. r 0) 0/1)` reads through the
           outer then inner aggregate — the effect-produced companion of the plain nested-projection escape,
           and a memory-safety pin for the aggregate-projection-that-escapes path. `Idx` seeded 10, arm
           `(resume s (+ s 1))`: the inner tuple `(tuple (Idx.next) (Idx.next))` reads 10 then 11 = `(10,
           11)`, the outer third read is 12, so `r = ((10, 11), 12)`; the nested projection `(. (. r 0) 1)`
           reads the inner tuple's second field, 11. Pins that a nested compound the handle builds from
           performed values escapes intact AND a projection reaching THROUGH the outer aggregate into a
           nested one is correctly reference-managed (no use-after-free / double-free when the nested field
           outlives its parent aggregate) — the effects × nested-projection-escape composition.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def
        (main)
        (let
          ((r
              (handle
                Idx
                10
                ((next (u) s (resume s (+ s 1))))
                #tuple(#tuple((Idx.next) (Idx.next)) (Idx.next)))))
          (. (. r 0) 1)))
      (export main)))
  (output (: 11 Int64)))

; A RECURSIVE effectful walk whose handler arm resumes WITH THE STATE ITSELF and threads a CHANGED state
; `(resume s (+ s 1))` — the exact combination (recursion × a state-threading arm whose resume VALUE is
; the state) that leaked a compiler-internal specialization name. The recursive-def specialization
; synthesizes a state-threading copy with a trailing `$s{k}` state param; the arm's resume value (`s`,
; substituted with a reference to that state param) was extracted straight off the discarded `resume`
; node, so its parent chain did not reach the specialized def — the reference resolved UNBOUND, surfacing
; the internal `walk#eff2$s0` name as a CDZ0101. Copying the extracted resume value/next-state (a
; re-parenting copy) attaches them to the threaded body, so the state-param reference resolves. Each
; factor alone already worked (the list-accumulator case above threads `(resume unit …)`; a non-recursive
; `(+ (Tick.tick) (Tick.tick))` threads fine; a recursive walk with a CONSTANT resume state compiles), so
; this pins their intersection.
(case
  "a recursive effectful walk under a state-threading handler compiles without leaking an internal name"
  (doc
    "`(def (walk (: n Int64)) (if (< n 1) 0 (do (Tick.tick) (walk (- n 1)))))` performs `Tick.tick`
           at each of n recursive steps, under a handler that resumes with the state and threads a changed
           one `(resume s (+ s 1))`. The walk returns the base `0` (the ticks thread state but the value is
           the base). This must compile and run to 0 — the recursive counterpart of the non-recursive
           state-threading case and of the recursive constant-state case, which both work. The E3/E4
           specialization must not leak its internal `walk#eff{n}$s{k}` state-param name as an unbound-name
           error: the recursive self-call's threaded state, and the arm's resume value that references the
           state param, must resolve against the synthesized specialization, not dangle.")
  (input
    (do
      (effect Tick (op tick (-> Unit Int64)))
      (def (walk (: n Int64)) (if (< n 1) 0 (do (Tick.tick) (walk (- n 1)))))
      (def (main) (handle Tick 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a BARE-param recursive state-threading walk compiles without leaking the specialization name"
  (doc
    "The bare-name-parameter companion of the annotated case above: `(def (walk n) …)` (no `(: n T)`)
           exercises the SAME specialization path — the synthesized trailing `$s{k}` state param must
           resolve against the specialized copy, not dangle as an unbound `walk#eff{n}$s{k}`. `walk 3`
           ticks the state 0→1→2→3 but returns the base 0.")
  (input
    (do
      (effect Tick (op tick (-> Unit Int64)))
      (def (walk n) (if (< n 1) 0 (do (Tick.tick) (walk (- n 1)))))
      (def (main) (handle Tick 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (output (: 0 Int64)))

(case
  "a recursive fn under a two-arm single-effect handler specializes with one state param"
  (doc
    "A handler with SEVERAL arms of ONE effect threads ONE logical state, so a recursive fn under it
           specializes with a single trailing state param — each perform substitutes its OWN arm's state
           binder. `St` has two ops: `get` (reads the counter, resumes it unchanged) and `tick` (returns 1,
           threads `s-1`). `loop` recurses summing a `tick` per non-zero `get`; seeded 3, `get` reads
           3,2,1,0 and `tick` returns 1 three times → 1+1+1+0 = 3. Pins that the arm-count does not gate
           specialization — only the single-EFFECT property does.")
  (input
    (do
      (effect St (op get (-> Unit Int64)) (op tick (-> Unit Int64)))
      (def (loop) (if (= (St.get) 0) 0 (+ (St.tick) (loop))))
      (def (main) (handle St 3 ((get (u) s (resume s s)) (tick (u) s (resume 1 (- s 1)))) (loop)))
      (export main)))
  (output (: 3 Int64)))

(case
  "two effects each declaring a same-named operation do not collide"
  (doc
    "Witnesses capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
           Operations (2nd sentence): `Unify` and `Scope` each declare a `resolve` operation, reached as
           `Unify.resolve` and `Scope.resolve`; the qualified names disambiguate. A `handle` discharges
           exactly ONE effect — its head names that effect and every arm is one of that effect's
           operations — so discharging both effects is two NESTED handles: an outer `(handle Scope …)`
           and an inner `(handle Unify …)`, each binding its own effect's `resolve`. The body performs
           `Unify.resolve`, discharged by the inner handler and resumed with 5; `Scope` is installed but
           never performed. Both handlers are stateless (seed `unit`). Pins that an operation is reached
           through its declaring effect and a shared operation name is collision-free — the two `resolve`
           arms live under distinct handlers keyed to distinct effects.")
  (input
    (do
      (effect Unify (op resolve (-> Int64 Int64)))
      (effect Scope (op resolve (-> Int64 Int64)))
      (def
        (main)
        (handle
          Scope
          unit
          ((resolve (x) s (resume x s)))
          (handle Unify unit ((resolve (x) s (resume (+ x 1) s))) (Unify.resolve 4))))
      (export main)))
  (output (: 5 Int64))
  (host-calls))

(case
  "an effect operation may be named `bind` — the interop directive keyword is not reserved for op names"
  (doc
    "`bind` is the head of the top-level peer-binding DIRECTIVE `(bind Effect \"cadenza:pkg/iface\")`,
           but that keyword is reserved only at the top level — an effect operation, like any member, may
           be named `bind`. `(effect Scope (op bind (-> Int64 Int64)) (op depth (-> Unit Int64)))` declares
           a `bind` operation whose handler arm is the NESTED list `(bind (v) d (resume (+ v d) (+ d 1)))`.
           Seeded 0: `(Scope.bind 10)` reads d=0 → `10 + 0` = 10 (state → 1), `(Scope.bind 20)` reads d=1 →
           `20 + 1` = 21 (state → 2), `(Scope.depth)` reads 2, so `(+ 10 (+ 21 2))` = 33. Pins that the
           malformed-`(bind …)` diagnostic scopes to TOP-LEVEL directives only: an arena-wide scan misreads
           the arity-3 handler arm as a malformed peer-binding (wrong arity) and rejects the program with a
           spurious CDZ0201 — a false positive on a legal operation name, fixed by scoping the scan to
           top-level `(bind …)` forms.")
  (input
    (do
      (effect Scope (op bind (-> Int64 Int64)) (op depth (-> Unit Int64)))
      (def
        (main)
        (handle
          Scope
          0
          ((bind (v) d (resume (+ v d) (+ d 1))) (depth (u) d (resume d d)))
          (let ((a (Scope.bind 10))) (let ((b (Scope.bind 20))) (+ a (+ b (Scope.depth)))))))
      (export main)))
  (output (: 33 Int64)))

; The dual of the collision-free cross-effect case: WITHIN one effect, an operation name declared TWICE
; is ill-formed. capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
; Operations: an effect declaration "binds each of its operations to an operation type, so that the set of
; operations an effect offers is a CLOSED, statically-known SET rather than an open collection of ad-hoc
; names." Two `(op f …)` in one effect bind the name `f` twice — the set is then not well-defined (which
; operation type governs a performance of `E.f`?), the same ill-formedness a record with a duplicate field
; (`(record (a 1) (a 2))`) and a module with a duplicate definition (`(module … (def (f) 1) (def (f) 2))`)
; are rejected for (CDZ0201): a fixed/closed set cannot name the same member twice. The effect MUST be
; rejected, not resolved by keeping one `f` and silently discarding the other. A compiler that registers
; each operation into the effect's table without checking for a name already bound silently keeps one and
; accepts the declaration — the effect-declaration sibling of the record-field and module-definition
; duplicate gaps. (Distinct from the cross-effect case above, where `Unify.resolve` and `Scope.resolve` are
; two operations of two effects, disambiguated by their effect — collision-free per the spec's 2nd
; sentence. Here it is one effect naming one operation twice.) A generation that does not yet check for a
; duplicate operation name declines rather than silently choosing one.
(case
  "an effect that declares an operation name twice is rejected"
  (doc
    "`(effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64)))` declares the operation `f` twice —
           but an effect's operations are a CLOSED, statically-known SET, each name bound to one operation
           type (capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
           Operations). Binding `f` twice makes the set ill-defined, the same ill-formedness a record with a
           duplicate field name or a module with a duplicate definition is rejected for (CDZ0201) — a fixed
           set cannot name the same member twice. The effect MUST be rejected, not resolved by keeping one
           `f` and discarding the other. Pins that the duplicate-member check reaches an effect's operation
           set, the effect-declaration sibling of the record-field (`(record (a 1) (a 2))`) and module-
           definition (`(module … (def (f) 1) (def (f) 2))`) duplicate cases; distinct from the collision-
           free cross-effect case above (`Unify.resolve` / `Scope.resolve`), which is two effects' distinct
           operations. A generation that does not yet detect a duplicate operation name declines rather
           than silently choosing one.")
  (input
    (do (effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64))) (def (main) 1) (export main)))
  (error CDZ0201))

; An effect declaration's clauses are `(op <name> <type>)` operations (and an optional leading `(doc …)`). A
; clause that is NOT an operation form — a bare literal `(effect E 5)`, a non-`op` list `(effect E (foo …))`,
; or an empty `(op)` — is malformed (CDZ0201, "an effect clause must be an operation"). A well-formed `(op …)`
; clause whose NAME is a non-name (`(op 5 …)`) is still an op clause, so it keeps the more-specific "an effect
; operation must be named" reject, not the generic clause one. A `(doc …)` clause is legitimate. (Migrated
; from rcdzc a_malformed_effect_clause_is_cdz0201.)
(case
  "a bare literal effect clause is a malformed effect declaration"
  (input (do (effect E 5) (def (main) 1) (export main)))
  (error CDZ0201 (message "an effect clause must be an operation")))

(case
  "a non-op list effect clause is a malformed effect declaration"
  (input (do (effect E (foo (-> Unit Int64))) (def (main) 1) (export main)))
  (error CDZ0201 (message "an effect clause must be an operation")))

(case
  "an empty op effect clause is a malformed effect declaration"
  (input (do (effect E (op)) (def (main) 1) (export main)))
  (error CDZ0201 (message "an effect clause must be an operation")))

(case
  "an op clause with a non-name keeps the specific must-be-named reject, not the generic clause one"
  (input (do (effect E (op 5 (-> Unit Int64))) (def (main) 1) (export main)))
  (error
    CDZ0201
    (message "an effect operation must be named")
    (not "an effect clause must be an operation")))

(case
  "an effect declaration with a leading doc clause is well-formed and its op handles + runs"
  (input
    (do
      (effect E (doc "the effect") (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5 s))) (E.get)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

; The DIAGNOSTIC-QUALITY half of the (spec-sanctioned, NOT rejected) two-same-named-effects situation: an
; effect's identity is its DECLARATION, not its name (14-effects:3129), so two `(effect E …)` are DISTINCT
; and a bare `E` resolves the FIRST. Naming an op declared only on a LATER same-named `E` fails "no operation
; `b`", but the ordinary "closest matches" list is baffling (the author sees `b`'s declaration a few lines
; up), so the message EXPLAINS the shadowing (`b` is on a later `(effect E …)`; a bare `E` resolves/discharges
; the first) instead of a typo list. A GENUINE typo (no later `E` declares it) keeps the ordinary did-you-mean.
; The hint fires at BOTH loci: the call site (CDZ0201) and a handler arm (CDZ0403). No confident-typo fix (the
; op is real, just in another declaration). (Migrated from rcdzc
; an_op_on_a_later_same_named_effect_gets_the_shadowed_declaration_hint.)
(case
  "an op declared only on a LATER same-named effect explains the shadowing, not a typo list"
  (input
    (do
      (effect E (op a (-> Int64 Int64)))
      (effect E (op b (-> Int64 Int64)))
      (def (main) (host (E) (E.b 5)))
      (export main)))
  (error
    CDZ0201
    (message "effect `E` has no operation `b`")
    (message "declared on a LATER")
    (message "resolves the FIRST")
    (not "closest matches")
    (not "did you mean")))

(case
  "a genuine effect-op typo (no shadowing) keeps the ordinary did-you-mean"
  (input (do (effect E (op emit (-> Int64 Int64))) (def (main) (host (E) (E.emt 5))) (export main)))
  (error
    CDZ0201
    (message "effect `E` has no operation `emt`")
    (message "did you mean `emit`?")
    (not "declared on a LATER")))

(case
  "a handler arm naming a later-effect op explains the shadowing on its CDZ0403"
  (input
    (do
      (effect E (op a (-> Int64 Int64)))
      (effect E (op b (-> Int64 Int64)))
      (def (main) (handle E 0 ((b (n) s (resume n s))) (E.a 5)))
      (export main)))
  (error
    CDZ0403
    (message "this handler arm names an operation its effect does not declare")
    (message "declared on a LATER")
    (message "discharges the FIRST")))

; CONTEXT-AWARE suggestion for a typo'd HANDLE effect name (diagnostics.md §A Diagnostic Carries A Route To A
; Fix — a fix must be one an agent applies and it WORKS, the one-shot rule): a `handle`'s effect name is
; rewritten to `(. Name op)` arm ops, so a typo there is a MEMBER OPERAND. Its candidate pool DROPS names with
; no members (variant constructors), because suggesting one would fail the one-shot rule (`(. Log op)` →
; "record has no field `op`"). So a name equidistant from a VARIANT and a member-accessible EFFECT prefers the
; effect; and when no member-accessible name is close, the diagnostic stays the honest plain unbound rather
; than offering a variant ctor a fix could not resolve (no fix beats a wrong one). (Migrated from rcdzc
; a_member_operand_typo_prefers_a_member_accessible_name_over_a_nearer_variant +
; a_member_operand_does_not_suggest_a_prelude_variant_constructor.)
(case
  "a typo'd handle effect name prefers a member-accessible effect over an equidistant variant"
  (input
    (do
      (effect Logr (op op (-> Unit Unit)))
      (type T (Log Int64))
      (def (main) (handle Logg 0 ((op (u) s (resume s s))) 42))
      (export main)))
  (error
    CDZ0101
    (message "did you mean `Logr`?")
    (fix (kind replace) (replacement "Logr") (unverified))))

(case
  "a typo'd handle effect name does not suggest a prelude variant constructor with no members"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle Nope 0 ((get (u) s (resume s s))) 42))
      (export main)))
  (error CDZ0101 (message "unbound") (not "None") (not "did you mean")))

; --- Handler resolution is dynamic in extent, across function boundaries ------------------------
; capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically Determined. The cases
; above perform and handle inside one `main`, where dynamic and lexical resolution coincide. These cases
; SEPARATE them: the perform is in a callee and the handler is in a caller, so resolution MUST follow the
; call chain, not the performing function's definition site. Each of these would be an ungranted-effect
; rejection (CDZ0401) under definition-site (lexical) resolution — the performing function is defined at
; top level with no handler in scope — so a defined output is itself the witness that resolution is dynamic.
; Which handler discharges each performance is nonetheless fixed statically (by monomorphizing the handler
; context), preserving determinism (constitution III).
(case
  "an effect performed in a callee is discharged by the caller's handler"
  (doc
    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined: `gen` performs `(Bump.by 41)` but installs no handler; `main` handles `Bump` around
           its CALL to `gen`. Resolution follows the call chain, so the perform in `gen` is discharged by
           `main`'s handler and the run computes 42. Under definition-site (lexical) resolution `gen` has no
           `Bump` handler in scope and the effect would be ungranted (CDZ0401) — the defined output 42 is
           the witness that a function may perform an operation its CALLER discharges. The handler is
           stateless (seed `unit`).")
  (input
    (do
      (effect Bump (op by (-> Int64 Int64)))
      (def (gen) (Bump.by 41))
      (def (main) (handle Bump unit ((by (n) s (resume (+ n 1) s))) (gen)))
      (export main)))
  (output (: 42 Int64))
  (host-calls))

(case
  "a callee whose body LET-BINDS a handle seeded by its param, called with the caller's runtime arg, keeps that arg bound"
  (doc
    "The caller-arg-through-a-let-wrapped-handle-seed shape: `(def (f x) (let ((r (handle St x …))) r))`
           binds the result of a `handle` whose SEED is the param `x`, and `main` calls `(f k)` passing its OWN
           runtime param `k`. This spuriously reported CDZ0101 'unbound name k' FROM THE COMPILE BACKEND (`cdz
           check` passed) — inlining `f` substituted the handle seed `x`→the arg node carrying `k`, then the
           tail-resumptive fold's `deep_fresh_copy` spliced that ONE seed node at BOTH state-binder references
           in the arm body (`(resume s (+ s 1))`), and the re-parent to the last site ORPHANED the first, so
           `k` re-resolved unbound. The fix let-binds a non-constant seed ONCE at the fold entry (an orphaned-
           occurrence bug of the same family as an extracted child spliced without a re-parenting copy). Only
           the CONJUNCTION triggers it — a const arg, a handle DIRECTLY in the body (no let), or a let over a
           NON-handle init each compiled. `main 5`: the handler resumes with the state (seeded 5), `(St.tick)`
           yields it → 5. Guards that a let-bound handle init seeded by a param does not drop the caller's
           runtime argument (the exact shape `verify_enforce` injects for `@ensures`/`@requires` over a
           handle-bodied def).")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def (f (: x Int64)) (let ((r (handle St x ((tick (u) s (resume s (+ s 1)))) (St.tick)))) r))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a let-bound handle whose seed is an EXPRESSION over the caller's runtime arg folds"
  (doc
    "Edge of the seed let-lift fix (the caller-runtime-arg-seed case above): the seed is not a bare arg
           but an EXPRESSION `(+ x 1)`, so the let-lift binds the whole expression once at the fold entry.
           `tick` returns the seed = k+1. main(5) = 6.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (f (: x Int64))
        (let ((r (handle St (+ x 1) ((tick (u) s (resume s (+ s 1)))) (St.tick)))) r))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a let-bound runtime-arg handle seed with the state used three times in the arm folds"
  (doc
    "Edge of the seed let-lift fix: the state binder is spliced at THREE sites in the arm body
           `(resume s (+ s (+ s 1)))`, so the once-bound seed must reach every splice without orphaning.
           `tick` returns the seed unchanged. main(5) = 5.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (f (: x Int64))
        (let ((r (handle St x ((tick (u) s (resume s (+ s (+ s 1))))) (St.tick)))) r))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a let-bound runtime-arg handle seed with two performs threads and advances the seed"
  (doc
    "Edge of the seed let-lift fix: the body performs TWICE `(+ (St.tick) (St.tick))`, so the seed is
           threaded AND advanced — first tick reads the seed (5) and advances to 6, second tick reads 6, and
           5 + 6 = 11. main(5) = 11.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def
        (f (: x Int64))
        (let ((r (handle St x ((tick (u) s (resume s (+ s 1)))) (+ (St.tick) (St.tick))))) r))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "a let-bound CONSTANT handle seed stays byte-identical and folds to the constant"
  (doc
    "Edge of the seed let-lift fix: a CONSTANT seed `0` takes the byte-identical path (the let-lift wrap
           is skipped for a shareable constant), so `tick` returns the constant. The caller's arg is ignored.
           main(5) = 0.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (def (f (: x Int64)) (let ((r (handle St 0 ((tick (u) s (resume s (+ s 1)))) (St.tick)))) r))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "an effect resolves past an intermediate frame that installs no handler"
  (doc
    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined: the call chain is `main` (handles `Ping`) -> `mid` (no handler) -> `leaf`
           (performs `Ping.ping`). The perform in `leaf` searches OUTWARD along the call chain, past `mid`
           which installs no handler, to `main`'s handler, which resumes with 5; `mid` then computes
           `(+ 5 100)` = 105. An intermediate function that installs no handler is transparent to
           resolution — it is merely a frame on the chain. The handler is stateless.")
  (input
    (do
      (effect Ping (op ping (-> Unit Int64)))
      (def (leaf) (Ping.ping))
      (def (mid) (+ (leaf) 100))
      (def (main) (handle Ping unit ((ping () s (resume 5 s))) (mid)))
      (export main)))
  (output (: 105 Int64))
  (host-calls))

(case
  "a nearer handler on the call chain shadows an outer one"
  (doc
    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined and #A Handler May Interpose On An Effect (a handler nearer the perform wins): the
           call chain is `main` (handler *100) -> `mid` (handler *10) -> `leaf` (performs `Mul.by 1`). The
           NEAREST active handler on the chain is `mid`'s, so `(Mul.by 1)` resolves to `(* 1 10)` = 10 and
           `main`'s outer *100 handler is never reached (the inner arm does not re-perform, so it does not
           forward). The result is 10, not 1000 — pinning that the nearest DYNAMIC handler discharges the
           operation and shadows the outer one. Both handlers are stateless.")
  (input
    (do
      (effect Mul (op by (-> Int64 Int64)))
      (def (leaf) (Mul.by 1))
      (def (mid) (handle Mul unit ((by (x) s (resume (* x 10) s))) (leaf)))
      (def (main) (handle Mul unit ((by (x) s (resume (* x 100) s))) (mid)))
      (export main)))
  (output (: 10 Int64))
  (host-calls))

(case
  "two LEXICALLY-NESTED handlers of the same effect partition the performs by region"
  (doc
    "The lexical-nesting companion of the call-chain shadow above: two handlers of the SAME effect `E`
           nest in ONE expression, and TWO performs are partitioned by which handler's region they sit in. `(+
           (handle E 5 … (E.get)) (E.get))`: the FIRST `(E.get)` is inside the inner `handle E 5`, so it
           resolves to the inner seed 5; the SECOND `(E.get)` is OUTSIDE the inner handle (a sibling operand
           of the `+`), so it escapes the inner region and reaches the OUTER `handle E 100`, resolving to 100.
           Both arms resume with the state unchanged (`(get (u) s (resume s s))`), so `(+ 5 100)` = 105. Pins
           that lexical handler nesting of the same effect partitions performs by REGION — the inner handle
           discharges only the performs textually within its body, and a perform outside it reaches the next
           enclosing handler (distinct from the call-chain case, where the whole callee runs under the nearer
           handler). Both backends agree.")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          100
          ((get (u) s (resume s s)))
          (+ (handle E 5 ((get (u) s (resume s s))) (E.get)) (E.get))))
      (export main)))
  (output (: 105 Int64)))

(case
  "an inner handle's SEED expression performs against the outer handler"
  (doc
    "The seed-position perform: `(handle B (+ (A.get unit) 100) …)` — the inner handle's SEED is
           computed by performing the OUTER effect (A.get reads n=5), so the inner handler starts at 105
           and the body's `(B.get unit)` reads it back beside a second outer read (105 + 5 = 110). The
           seed expression evaluates in the OUTER handler's scope BEFORE the inner handler exists; a
           lowering that evaluated the seed under the inner handler (or defaulted it) mis-seeds.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect B (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((get (u) s (resume s s)))
          (handle B (+ (A.get unit) 100) ((get (u) t (resume t t))) (+ (B.get unit) (A.get unit)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 110 Int64)))

(case
  "an inner same-effect handle's RESULT feeds the outer handler's next perform"
  (doc
    "Cross-region value flow under shadowing: the inner `handle Ctr 100` discharges `(Ctr.bump 2)`
           with a MULTIPLYING arm (100·2 = 200) and its result becomes the ARGUMENT of the outer region's
           `(Ctr.bump inner)` — discharged by the outer ADDING arm (10 + 200 = 210). The value crosses
           from the inner region's arm through the let into the outer region's perform; the shadow pins
           nearby witness state ISOLATION, this witnesses the VALUE HANDOFF between regions of one
           effect.")
  (input
    (do
      (effect Ctr (op bump (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Ctr
          n
          ((bump (v) s (resume (+ s v) (+ s v))))
          (let
            ((inner (handle Ctr 100 ((bump (v) t (resume (* t v) t))) (Ctr.bump 2))))
            (Ctr.bump inner))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 210 Int64)))

(case
  "same-effect shadowing with ADVANCING states — the outer state survives the inner handle and resumes advanced"
  (doc
    "The STATEFUL upgrade of the lexical-partition case above (there both arms resume `s` unchanged,
           so a shared or re-seeded state slot is invisible): here BOTH handlers ADVANCE a counter. The
           outer `Ctr` seeds 10; its first tick reads 10 (state → 11). The inner `handle Ctr 2000` then
           discharges its own region's two ticks — 2000 and 2001 (its own slot, seeded independently,
           advancing independently) → 4001. The perform AFTER the inner handle exits reaches the OUTER
           handler again and must read 11 — the outer state advanced by the pre-inner tick, UNTOUCHED by
           the inner region's two discharges, resumed exactly where it left off. 10 + 4001 + 11 = 4022. A
           shadow implementation sharing one state slot (inner ticks bleeding the outer to 12/13), or
           re-seeding the outer on inner-exit (reading 10 again → 4021), breaks the value. Expected: 4022.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          10
          ((tick (u) s (resume s (+ s 1))))
          (+
            (Ctr.tick)
            (+
              (handle Ctr 2000 ((tick (u) s (resume s (+ s 1)))) (+ (Ctr.tick) (Ctr.tick)))
              (Ctr.tick)))))
      (export main)))
  (output (: 4022 Int64)))

(case
  "same-effect shadowing with ADVANCING states, HEAP (tuple) state — the outer heap seed survives the inner handle and threads correctly across the straddle"
  (doc
    "The HEAP-state (`#seed`) twin of the scalar `same-effect shadowing with ADVANCING states` case
           above. Both handlers of `M` carry a TUPLE state read via `(. s 0)` / `(. s 1)` projection (a heap
           seed, threaded via the fold's `#seed` let-lift — unlike the scalar counter, which is a shareable
           constant with no `#seed`). The OUTER handler is dispatched at `(M.step 1)` BEFORE and `(M.step 2)`
           AFTER the inner `(handle M (tuple 100 0) …)`, so the outer arm is re-applied at a site straddling
           the nested same-effect handle — the exact shape whose sum-state variant-matched cousin DECLINES
           (`su6d`, an unbound-`#seed` scoping gap). Here the state is READ by projection (not variant-matched),
           so it COMPILES, and pins that the outer heap seed threads correctly across the straddle: outer
           `(M.step 1)` reads `(. (tuple n 0) 0)`=n (state→`(tuple n 1)`); the inner region discharges its own
           `(tuple 100 0)` seed independently (`(M.step 4)`→100 state→`(tuple 101 0)`, `(M.step 0)`→101) = 201;
           the post-inner outer `(M.step 2)` must read the OUTER state advanced by step 1 (`(. (tuple n 1) 0)`=n)
           — UNTOUCHED by the inner region. Total 2·n+201 (n=3→207, n=10→221, n=0→201). A shadow sharing one
           heap-seed slot, or re-seeding the outer on inner-exit, breaks the value; an unbound-`#seed` regression
           would decline. Guards the heap-`#seed` threading the su6d fold-gap probing (v-effects a45) exercised.")
  (input
    (do
      (effect M (op step (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          #tuple(n 0)
          ((step (v) s (resume (. s 0) #tuple((. s 0) (+ (. s 1) v)))))
          (+
            (M.step 1)
            (+
              (handle
                M
                #tuple(100 0)
                ((step (v) s (resume (. s 0) #tuple((+ (. s 0) 1) (. s 1)))))
                (+ (M.step 4) (M.step 0)))
              (M.step 2)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 207 Int64))
  (call main (: 10 Int64))
  (output (: 221 Int64))
  (call main (: 0 Int64))
  (output (: 201 Int64)))

(case
  "a nested handle's INIT expression performs against the OUTER handler before installing"
  (doc
    "The install boundary itself performing: the inner seed (Out.tick) evaluates in the
           OUTER's scope, its resume value becomes the inner seed, and the outer state advance
           survives to the trailing (Out.tick) — 100+101=201.")
  (input
    (do
      (effect Out (op tick (-> Unit Int64)))
      (effect In (op get (-> Unit Int64)))
      (def
        (main (: seed Int64))
        (handle
          Out
          seed
          ((tick (_u) c (resume c (+ c 1))))
          (+ (handle In (Out.tick) ((get (_u) s (resume s s))) (In.get)) (Out.tick))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 201 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "outer handler state threads AROUND a completed inner handle of a different effect"
  (doc
    "State continuity across an inner lifecycle: outer tick / full inner handle installs+runs+
           tears down / outer tick — the b−a=1 digit proves exactly ONE increment happened across
           the inner (a state reset or double-advance flips it).")
  (input
    (do
      (effect Out (op tick (-> Unit Int64)))
      (effect In (op get (-> Unit Int64)))
      (def
        (main (: seed Int64))
        (handle
          Out
          seed
          ((tick (_u) c (resume c (+ c 1))))
          (do
            (def a (Out.tick))
            (def inner (handle In 5 ((get (_u) s (resume s s))) (In.get)))
            (def b (Out.tick))
            (+ (* a 100) (+ (* inner 10) (- b a))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 351 Int64))
  (call main (: 0 Int64))
  (output (: 51 Int64)))

(case
  "TWO sequential inner handles seed from the outer's ADVANCING state"
  (doc
    "Repeated seeding: each inner's performing INIT reads the outer at a different point
           (i1=seed, i2=seed+1) and fin−i1=2 proves both advances stuck across two full inner
           install/teardown cycles.")
  (input
    (do
      (effect Out (op tick (-> Unit Int64)))
      (effect In (op get (-> Unit Int64)))
      (def
        (main (: seed Int64))
        (handle
          Out
          seed
          ((tick (_u) c (resume c (+ c 1))))
          (do
            (def i1 (handle In (Out.tick) ((get (_u) s (resume s s))) (In.get)))
            (def i2 (handle In (Out.tick) ((get (_u) s (resume s s))) (In.get)))
            (def fin (Out.tick))
            (+ (* i1 100) (+ (* i2 10) (- fin i1))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 342 Int64))
  (call main (: 0 Int64))
  (output (: 12 Int64)))

(case
  "the same function called under two handlers is discharged by each in turn"
  (doc
    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined (the monomorphization property): a single function `ask` = `(+ (Get.get) 1)` is
           called under two DIFFERENT `Get` handlers — one resuming 10, one resuming 20. The first call
           yields `(+ 10 1)` = 11, the second `(+ 20 1)` = 21, and `main` sums them to 32. The same
           definition is discharged by whichever handler is active on the call chain at each call site, so
           a self-hosting compiler specializes (monomorphizes) `ask` once per handler context it is called
           under — the effect is an implicit parameter threaded from the caller that installed the handler.
           Under definition-site resolution `ask` has no `Get` handler in scope and both calls would be
           ungranted (CDZ0401); the defined output 32 is the witness for dynamic resolution. Both handlers
           are stateless.")
  (input
    (do
      (effect Get (op get (-> Unit Int64)))
      (def (ask) (+ (Get.get) 1))
      (def
        (main)
        (+
          (handle Get unit ((get () s (resume 10 s))) (ask))
          (handle Get unit ((get () s (resume 20 s))) (ask))))
      (export main)))
  (output (: 32 Int64))
  (host-calls))

(case
  "an effect resolves through a deep chain of intermediate functions"
  (doc
    "Witnesses capabilities-and-effects.md #Handler Resolution Is Dynamic In Extent And Statically
           Determined at depth: the chain is `main` (handles `Ask`) -> `a` -> `b` -> `c` -> `d`
           (performs `Ask.ask`), each of `a`/`b`/`c` adding 1 to its callee's result and installing no
           handler. The perform in `d` resolves past three intermediate frames to `main`'s handler, which
           resumes with 7; the +1s then compose back up the chain: d=7, c=8, b=9, a=10.
           Pins that dynamic resolution reaches an arbitrarily deep enclosing handler and that the
           intermediate frames are transparent. The handler is stateless.")
  (input
    (do
      (effect Ask (op ask (-> Unit Int64)))
      (def (d) (Ask.ask))
      (def (c) (+ (d) 1))
      (def (b) (+ (c) 1))
      (def (a) (+ (b) 1))
      (def (main) (handle Ask unit ((ask () s (resume 7 s))) (a)))
      (export main)))
  (output (: 10 Int64))
  (host-calls))

(case
  "a stateful handler threads its counter across a function boundary"
  (doc
    "Witnesses capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges composed with #Handler Resolution Is Dynamic In Extent: the `Fresh` counter, seeded
           0 in `main`, is folded across performs that happen in a CALLEE. `label` performs `(Fresh.next)`;
           `pair-of` calls `label` twice to build `(tuple (label) (label))`. The handler discharges both
           performs — reached dynamically through `pair-of` and `label` — threading the counter across the
           function boundary: the first `label` sees 0, the second sees 1, giving `(tuple 0 1)`. Pins that
           the folded state is not a lexical-scope construct but a dynamic-extent one that persists across
           calls, exactly as the compiler's fresh-name supply must. The handle evaluates to the body's
           tuple; the final counter 2 is discarded.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def (label) (Fresh.next))
      (def (pair-of) #tuple((label) (label)))
      (def (main) (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (pair-of)))
      (export main)))
  (output (: #tuple(0 1) (Tuple Int64 Int64))))

; --- A recursive function drives an effect (the state-machine idiom) --------------------------
; capabilities-and-effects.md #A Handler Threads State Across The Operations It Discharges composed
; with #Handler Resolution Is Dynamic In Extent, at the point a function RECURSES while performing.
; These are the shape a self-hosting compiler actually has — a recursive walk (over an AST, a token
; stream) that performs an effect (fresh name, diagnostic, unification) on each step. A recursive
; effectful function CANNOT be discharged by inlining it into the handled region (its body would
; inline without bound); it needs effect-context monomorphization — the function emitted once as a
; real wasm function that reads the discharging handler as an implicit evidence parameter
; (options/effects-model/lowering-to-wasm.md §Effect-context monomorphization, §Stage 3). A
; generation that resolves cross-function effects only by inlining DECLINES these (an honest todo,
; never a hang or a miscompile — reject-don't-miscompile); the recorded output is the semantics a
; monomorphizing generation realizes.
(case
  "a recursive function counts down through a stateful effect and bails at zero"
  (doc
    "Witnesses the recursive-effect idiom: `loop` performs `(Countdown.tick)` and recurses
           until the tick reads 0. The handler is seeded with 3 and its arm
           `(Countdown.tick (u) s (resume s (- s 1)))` hands back the current counter and threads
           `s - 1` forward, so successive ticks read 3, 2, 1, 0. `loop` adds 1 for each non-zero
           tick and returns 0 at the zero tick: the four ticks (3,2,1,0) yield `1 + 1 + 1 + 0` = 3.
           The counter is folded across a RECURSIVE call chain (dynamic extent), exactly as a
           compiler's fresh-name/position counter is folded across a recursive AST walk. `loop`
           recurses while performing, so it cannot be inlined into the handle (non-terminating);
           discharging it needs effect-context monomorphization — until a generation realizes that,
           the compiler declines rather than inlines (reject-don't-miscompile). The recorded output
           3 is the semantics a monomorphizing generation produces.")
  (input
    (do
      (effect Countdown (op tick (-> Unit Int64)))
      (def (loop) (if (= (Countdown.tick) 0) 0 (+ 1 (loop))))
      (def (main) (handle Countdown 3 ((tick (u) s (resume s (- s 1)))) (loop)))
      (export main)))
  (output (: 3 Int64)))

(case
  "a self-recursive effectful loop sums a fresh-id draw per step — the gensym idiom"
  (doc
    "The compiler-ml port's fresh-id generator shape (`implementation/compiler-ml/src/fresh.cdz`, the
           self-host's first use of the effect system): `id-sum n = if n = 0 then 0 else (Fresh.next) +
           id-sum(n - 1)` draws one fresh id per recursion and sums them. The perform `(Fresh.next)` is the
           LEFT operand of the `+` and the self-call the RIGHT — a strict spine where the perform is
           evaluated BEFORE the self-call, so it reads the PRE-recursion (incoming) state, which the
           single-return effect-context specialization threads correctly. Seeded 0, the ids drawn are 0, 1,
           2, so `id-sum 3` = `0 + (1 + (2 + 0))` = 3. Pins the gensym idiom the self-hosted compiler uses to
           thread unique type-variable / name ids without a hand-plumbed counter. (Contrast the SELF-CALL-
           before-perform shape — two sibling recursive calls whose second reads the first's OUT-state —
           which the single-return spec cannot thread and declines cleanly, pending the multi-value-return
           increment.)")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (id-sum (: n Int64)) (if (= n 0) 0 (+ (Fresh.next) (id-sum (- n 1)))))
      (def (main) (handle Fresh 0 ((next () s (resume s (+ s 1)))) (id-sum 3)))
      (export main)))
  (output (: 3 Int64)))

; NESTED match-scrutinee destructuring of a TUPLE-returning performing helper (breaker eg1). `stamp`
; returns a tuple with a fresh `(C.tick)` embedded (`stamp p = match p (#tuple(a b) -> #tuple(a b
; (C.tick)))`); `main` calls it as the scrutinee of an outer match, then AGAIN as the scrutinee of an
; inner match in the outer arm — two levels of tuple-destructure over a performing helper. This FOLDS.
; It was a CDZ0900 decline until the resumptive-conditional hoist was made match-aware: after each stamp
; call inlines to `(match tuple-literal (#tuple(a b) #tuple(a b (C.tick))))` and Site 5 lifts the
; performing scrutinee to a `#cv` let, the fold is left with a well-formed `(match #cv (P (match … (Q b))))`.
; The hoist's generic child-recursion then descended into the OUTER arm PAIR `(P body)` and — because a
; two-element `(pattern body)` pair resolves as `Apply { head: pattern, args: [body] }` — Site 2 (strict-
; application distribution) misread the PATTERN as a function head and distributed it into the inner
; match's branches, corrupting the arm into a bare `(match … (P …))` (an arm slot holding a raw match →
; resolve_match Poisons it → CDZ0900). The fix recurses into match arm BODIES only, never the arm pair as
; an expression, so the pattern is preserved. NARROWED (verified with controls): the trigger was this
; NESTED match-scrutinee-over-a-tuple-returning-performing-helper shape, NOT "two calls" and NOT
; genericity. Two cross-function performing-helper calls in OPERATOR / LET / DO position also fold (even
; monomorphic: `(+ (bump 10) (bump 20))`, `(let ((a (bump 10))) (let ((b (bump 20))) …))`, `(do (bump 10)
; (bump 20))`); a SINGLE tuple-destructure call folds (eg2); nested match-scrutinee performs over a
; BARE-BINDER (non-destructured) result fold. Handler seeds n, `tick` returns the current state and
; threads s+1, so the first stamp's tick reads t1=n and the second t2=n+1; the result is
; byte-len("hi")=2 + 7 + 100*t1 + 10000*t2 = 10009 + 10100n (n=3 -> 40309, n=0 -> 10009, n=5 -> 60509).
(case
  "NESTED match-scrutinee destructure of a tuple-returning performing helper folds (single call + operator/let/do-position calls fold; the nested tuple-destructure over the helper now folds too — the resumptive-conditional hoist recurses into match arm BODIES, not the arm pair)"
  (doc
    "breaker eg1, the effects x cross-function-inline frontier. `stamp` is a performing helper
           `stamp p = match p (#tuple(a b) -> #tuple(a b (C.tick)))` returning a tuple with a fresh `(C.tick)`
           embedded; `main` calls it as the scrutinee of an OUTER match, then AGAIN as the scrutinee of an
           INNER match in the outer arm (here generic, used at (String,Bool) then (Int64,String)). This FOLDS.
           It was a CDZ0900 decline until the resumptive-conditional hoist was made match-aware: its generic
           child-recursion descended into a match arm PAIR `(pattern body)`, which resolves as an
           `Apply { head: pattern, args: [body] }`, so Site 2 (strict-application distribution) misread the
           PATTERN as a function head and distributed it into the arm body's branches — malforming the arm
           into a bare `(match …)` (arm slot holding a raw match) that resolve_match Poisoned into CDZ0900.
           The fix recurses into arm BODIES only, keeping the pattern intact. NARROWED with controls: the
           trigger was this NESTED match-scrutinee-over-a-tuple-returning-performing-helper shape, NOT the
           number of calls and NOT genericity. Two cross-function performing-helper calls in OPERATOR / LET /
           DO position fold (even monomorphic); a single tuple-destructure call folds (eg2); nested
           match-scrutinee performs over a BARE-BINDER result fold. Handler seeds n, `tick` returns the
           current state and threads s+1 -> first stamp reads t1=n, second reads t2=n+1; result =
           byte-len(\"hi\")=2 + 7 + 100*t1 + 10000*t2 = 10009 + 10100n. The recorded outputs are VERIFIED via
           the both-performs-inlined control.")
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (stamp p) (match p (#tuple(a b) #tuple(a b (C.tick)))))
      (def (main (: n Int64))
        (handle C n ((tick () s (resume s (+ s 1))))
          (match (stamp #tuple("hi" true))
            (#tuple(s _b t1)
              (match (stamp #tuple(7 "yo"))
                (#tuple(v _s2 t2)
                  (+ (String.byte-len s) (+ v (+ (* 100 t1) (* 10000 t2))))))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 40309 Int64))
  (call main (: 0 Int64))
  (output (: 10009 Int64))
  (call main (: 5 Int64))
  (output (: 60509 Int64)))

; NAME-COLLISION variant of eg1 (the binder-hygiene frontier). Identical nested shape, but the caller's
; OUTER destructure binder is named `a` — the SAME spelling as `stamp`'s INTERNAL match-arm binder
; (`stamp p = match p (#tuple(a b) …)`). The idealistic value is unchanged by the rename: `a` is the first
; element of the OUTER stamp's result (1), `c` the inner's (2), `t1`/`t2` the two ticks (n, n+1). So
; `a + 1000*c + 10*t1 + 100*t2` = 1 + 2000 + 10n + 100(n+1) = 2101 + 110n (n=0 -> 2101, n=3 -> 2431).
; FOLDS CORRECTLY: the resumptive-conditional commute nests the continuation (which references the outer
; `a`) lexically UNDER the inlined `stamp`'s own `(#tuple(a b))` match-arm binder — which WOULD capture the
; outer `a`, except `reduce_applied_lambdas` now α-renames each inlined helper body's match-arm pattern
; binders to fresh names (`freshen_match_arm_binders`) at the inline point, so `stamp`'s internal `a`
; becomes `#a{n}` and cannot capture the caller's `a`. A pure α-conversion (inert for distinct-name cases),
; so the outer `a`=1 stays 1. (Formerly SAFE-DECLINED; the freshening supersedes that floor.)
(case
  "eg1 name-collision variant — caller destructure binder shares the helper's internal match-arm binder name folds (inlined-helper match-binder freshening)"
  (doc
    "The eg1 nested shape with the caller's OUTER destructure binder named `a`, colliding with `stamp`'s
           INTERNAL binder `a`. The idealistic semantics are identical to eg1 (the rename is inert): outer
           `a`=1, inner `c`=2, ticks t1=n and t2=n+1, so `a + 1000*c + 10*t1 + 100*t2` = 2101 + 110n. Folds
           correctly because `reduce_applied_lambdas` α-renames the inlined helper's `(#tuple(a b))` arm
           binders to fresh `#a{n}`/`#b{n}` at the inline point (`freshen_match_arm_binders`), so the commute
           can no longer capture the outer `a` — a capture-avoiding hygiene fix superseding the former
           safe-decline. Distinct-name eg1 above is unaffected (freshening is a no-op there).")
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (stamp p) (match p (#tuple(a b) #tuple(a b (C.tick)))))
      (def (main (: n Int64))
        (handle C n ((tick () s (resume s (+ s 1))))
          (match (stamp #tuple(1 true))
            (#tuple(a _b t1)
              (match (stamp #tuple(2 true))
                (#tuple(c _d t2)
                  (+ a (+ (* 1000 c) (+ (* 10 t1) (* 100 t2))))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2101 Int64))
  (call main (: 3 Int64))
  (output (: 2431 Int64)))

; eg1 collision through a SHADOWING helper-arm let: the same name-collision family, but the inlined helper's
; match arm has a nested `(let ((a (+ a 100))) …)` that REBINDS the colliding binder `a`. The inlined-helper
; freshening must rename BOTH binder classes — the match-arm pattern binder AND the let binder — else the
; continuation captures through the un-freshened LET binder (probe: folded 102202 not 102201). `stamp`'s arm
; is `#tuple(a b) -> (let ((a (+ a 100))) #tuple(a b (C.tick)))`, so `stamp #tuple(1 _)` = `#tuple(101 _ n)`
; and `stamp #tuple(2 _)` = `#tuple(102 _ (n+1))`; outer a=101, c=102, t1=n, t2=n+1 ->
; a + 1000*c + 10*t1 + 100*t2 = 101 + 102000 + 10n + 100(n+1) = 102201 + 110n (n=0 -> 102201, n=3 -> 102531).
; Pins that `reduce_applied_lambdas` runs BOTH `freshen_local_binders` (let/do/fn) and
; `freshen_match_arm_binders` (match-arm) on the inlined body. (v-effects hardening probe of the #8222 fix.)
(case
  "eg1 collision through a SHADOWING helper-arm let folds (inlined-helper freshening covers let AND match-arm binders)"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (stamp p) (match p (#tuple(a b) (let ((a (+ a 100))) #tuple(a b (C.tick))))))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (match (stamp #tuple(1 true))
            (#tuple(a _b t1)
              (match (stamp #tuple(2 true))
                (#tuple(c _d t2)
                  (+ a (+ (* 1000 c) (+ (* 10 t1) (* 100 t2))))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 102201 Int64))
  (call main (: 3 Int64))
  (output (: 102531 Int64)))

(case
  "a MUTUALLY-recursive effectful group is specialized under a state handler"
  (doc
    "Effect-context monomorphization extends past a SINGLE self-recursive function to a MUTUALLY-
           recursive group. `ev` and `od` call each other, and the effect `Ctr.tick` is reached by `ev`
           only THROUGH its partner `od` — so detecting that `ev` reaches the effect requires following the
           RECURSIVE partner call, and specializing it requires tying the two specializations' knot (each
           partner's recursive call resolves to the other's specialized copy). Seeded 7, `tick` hands back
           the counter and threads `s - 1`: `ev(4)`→`od(3)` reads 7, `ev(2)`→`od(1)` reads 6, `ev(0)`=0, so
           the sum is `7 + (6 + 0)` = 13. Recursive-while-performing across a MUTUAL cycle — the same
           dynamic-extent state fold as the single-recursion countdown, over a call graph rather than a
           single self-call.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (od (- n 1))))
      (def (od (: n Int64)) (+ (Ctr.tick) (ev (- n 1))))
      (def (main) (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) (ev 4)))
      (export main)))
  (output (: 13 Int64)))

(case
  "a STRUCTURALLY-IDENTICAL mutual-performer SCC round-trips through the cadenza backend"
  (doc
    "The content-addressed-spec-dedup face of the mutual-performer SCC (v-effects, dispatched by
           v-core-opt). `ev` and `od` are STRUCTURALLY IDENTICAL — each is `(if (= n 0) 0 (+ (Cnt.tick)
           (partner (- n 1))))` — so effect-threading specializes both to the SAME shape and the layout's
           congruence dedup (`effect_spec_merge_map`) COLLAPSES `od#eff` into its representative `ev#eff`
           (dropped from `layout.order`, never emitted; callers redirected). The wasm backend redirects the
           merged spec's func-index and the rust backend canonicalizes its name; the CADENZA backend emits by
           NAME too, so it must likewise canonicalize a `Core::Call` to the merged partner to its
           representative — else the emitted `(def (ev#eff …) … (od#eff …))` names a def with no `(def …)` and
           the round-trip fails `unbound name od#eff` (CDZ0101). Seeded 0, `tick` hands back the counter and
           threads `s + 1`: four ticks over `ev(4)→od(3)→ev(2)→od(1)→ev(0)=0` read 0,1,2,3, so the sum is
           `0 + 1 + 2 + 3` = 6. Value-equivalent on the direct path AND the cadenza round-trip — the merged
           partner's self-consistent representative computes the identical mutual recursion.")
  (input
    (do
      (effect Cnt (op tick (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (+ (Cnt.tick) (od (- n 1)))))
      (def (od (: n Int64)) (if (= n 0) 0 (+ (Cnt.tick) (ev (- n 1)))))
      (def (main) (handle Cnt 0 ((tick (u) s (resume s (+ s 1)))) (ev 4)))
      (export main)))
  (output (: 6 Int64)))

(case
  "a mutually-recursive group performs in one branch and recurses in the OTHER"
  (doc
    "The SPLIT-BRANCH mutual-recursion shape: unlike the case above (where the perform `(Ctr.tick)`
           and the mutual call `(ev …)` sit in the SAME strict expression `(+ (Ctr.tick) (ev …))`), here
           the perform is in a cycle def's BASE-CASE branch and the mutual call is in its RECURSIVE branch —
           `(def (ev n) (if (= n 0) (Fresh.next) (od (- n 1))))` with `(od n) = (if (= n 0) 0 (ev (- n 1)))`.
           Detecting that `ev` reaches `Fresh` still requires following the recursive partner, and the two
           specializations' knot must tie even though each def's perform and mutual call are in DIFFERENT
           branches (the branch-distributed state threading + cross-def memo knot). Seeded 0, `next` resumes
           `s + 1`: `(ev 2)` chains `ev2→od1→ev0`, and `ev0` hits its BASE branch `(Fresh.next)` which
           resumes the seed `0 + 1` = 1 — so the result is 1, a NON-ZERO value that witnesses the perform
           in the separate base-case branch actually fired (an odd start `(ev 3)`→`ev3→od2→ev1→od0` = 0
           never reaches it). This is the fresh-name / gensym shape an effectful AST-walking compiler pass
           needs (`relabel(node)` ↔ `relabel-list(children)`, the counter threaded as a `Fresh` effect
           rather than an explicit parameter). Pins that the mutual specialization ties the knot across the
           separate-branch case, not only the same-branch one. (This shape was previously a clean decline
           pending the fold work; it now specializes correctly.)")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
      (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
      (def (main) (handle Fresh 0 ((next () s (resume (+ s 1) s))) (ev 2)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a THREE-way mutually-recursive effectful cycle specializes and runs"
  (doc
    "The mutual-recursion specialization generalizes to any cycle length: `ev`/`od`/`tw` form a
           three-def cycle where the effect `Ctr.tick` is reached only through `tw`. The visited-set-bounded
           `body_reaches_discharged` follows the cycle and ties all three specializations' knot. Seeded 9,
           `tick` hands back `s` and threads `s-1`.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (od (- n 1))))
      (def (od (: n Int64)) (tw (- n 1)))
      (def (tw (: n Int64)) (+ (Ctr.tick) (ev (- n 1))))
      (def (main) (handle Ctr 9 ((tick (u) s (resume s (- s 1)))) (ev 6)))
      (export main)))
  (call main)
  (output (: 17 Int64)))

(case
  "an abortive handler over a non-tail MUTUAL recursion folds to the abort value (99)"
  (doc
    "An ABORTIVE handler over a MUTUALLY-recursive callee with a non-tail cross-call: the partner's
           pending `+ 1` frames (which an abort must abandon) must NOT flow the abort value back through them
           (`(+ 1 (od …))` with `od = (+ 1 (ev …))` → 103, not 99). The non-local-exit TAGGED-RETURN CC folds
           it over the whole SCC: `ev#eff` and `od#eff` each return `#tuple(tag value)`, and the tagged
           threader treats a call to ANY `mutual_scc_of` member as a recursive tag-check-short-circuit
           (`(let ((r (od#eff …))) (if (= (. r 0) 1) r #tuple(0 (+ 1 (. r 1)))))`), so `ev`'s base abort
           `#tuple(1 99)` propagates its tag up through BOTH partners' pending frames, abandoning them → 99.
           (The gate relaxes the mutual guard for the tagged shape; a mutual case the threader cannot model —
           e.g. a branch-perform sharing a strict expr with the mutual call, rw4 — still declines cleanly.)")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (ev (: n Int64)) (if (= n 0) (Bail.bail 99) (+ 1 (od (- n 1)))))
      (def (od (: n Int64)) (+ 1 (ev (- n 1))))
      (def (main) (handle Bail 0 ((bail (n) s n)) (ev 4)))
      (export main)))
  (call main)
  (output (: 99 Int64)))

(case
  "a mutual group performing in an IF base-case branch and recursing in the other threads and runs"
  (doc
    "The if-branch companion of the match-dispatched separate-branch case: `ev` performs `Fresh.next`
           in its base-case branch and calls the partner `od` in its recursive branch, over a state-threading
           handler seeded 42. `ev 2 -> od 1 -> ev 0` fires `Fresh.next`; the `next` arm resumes `s+1` threading
           `s`, so the single base draw at seed 42 answers 43. The per-branch state-ref copy makes the
           branch-distributed state thread correctly and the memo knot tie.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
      (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
      (def (main) (handle Fresh 42 ((next () s (resume (+ s 1) s))) (ev 2)))
      (export main)))
  (call main)
  (output (: 43 Int64)))

(case
  "a split-branch mutual-effect group whose recursion never reaches the base-case perform runs to 0"
  (doc
    "The split-branch mutual group specializes even when the effect is never actually performed at
           runtime: `ev` performs `Fresh.next` in its base branch and calls `od` in its recursive branch, but
           `ev 3 -> od 2 -> ev 1 -> od 0 -> 0` bottoms out in `od`'s base (0), never reaching `ev`'s
           `Fresh.next`. The per-branch state-ref copy makes the branch-distributed state thread correctly and
           the memo knot tie; the run yields 0 (no draw).")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
      (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
      (def (main) (handle Fresh 0 ((next () s (resume s (+ s 1)))) (ev 3)))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a mutually-recursive group performs through a shared non-recursive helper"
  (doc
    "Composes the two cross-function triggers: a mutually-recursive group (`ev`/`od`) where the
           effect is performed inside a NON-recursive helper `h` that `od` calls, rather than syntactically
           in `od`'s own body. The helper INLINES (the non-recursive inline trigger) and the mutual pair
           SPECIALIZES (the recursive trigger), and they compose — `od`'s `(h)` is inlined to `(Ctr.tick)`
           within the specialized `od#ctx`. Seeded 7, threading `s - 1`, the ticks read 7 then 6, so `ev(4)`
           = `7 + (6 + 0)` = 13. Pins that specialization detecting the effect through a mutual partner and
           inlining a performing helper cooperate in one recursive group.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (h) (Ctr.tick))
      (def (ev (: n Int64)) (if (= n 0) 0 (od (- n 1))))
      (def (od (: n Int64)) (+ (h) (ev (- n 1))))
      (def (main) (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) (ev 4)))
      (export main)))
  (output (: 13 Int64)))

(case
  "a mutually-recursive group performs in its entry def while its partner only dispatches"
  (doc
    "The MIRROR of the case above, and the one that pins the mutual-group scheme fixpoint. Here the
           ENTRY def `ev` (the one the handle body calls, so its scheme is demanded FIRST) is the one that
           PERFORMS — it recurses through its partner `od`, which is a PURE DISPATCHER whose body is
           ENTIRELY the sibling call `(ev (- n 1))`. Computing `ev`'s scheme demands `od`'s mid-flight,
           while `ev`'s own signature is still provisional; `od`'s body — being only `(ev …)` — then reads
           that provisional `ev` and would type as an undetermined `Any`. The mutual-group scheme solve
           must NOT cache that provisional `None` for `od` (else the dispatcher is poisoned permanently and
           the whole group declines); once `ev` resolves via its base case, a re-demand computes `od`'s
           true `Int64 -> Int64`. Seeded 7, threading `s - 1`, the ticks read 7 then 6, so `ev(4)` =
           `(Ctr.tick) + od(3)` = `7 + ev(2)` = `7 + ((Ctr.tick) + od(1))` = `7 + (6 + ev(0))` =
           `7 + 6 + 0` = 13. Recursive-while-performing, so it needs effect-context specialization
           (`DESIGN-effects-rcdzc.md` §4.2, §4.3).")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (+ (Ctr.tick) (od (- n 1)))))
      (def (od (: n Int64)) (ev (- n 1)))
      (def (main) (handle Ctr 7 ((tick (u) s (resume s (- s 1)))) (ev 4)))
      (export main)))
  (output (: 13 Int64)))

(case
  "a mutually-recursive group with the perform in a DIFFERENT branch from the mutual call folds"
  (doc
    "The mutual-group shape where the perform and the mutual call sit in SEPARATE branches of a
           conditional — distinct from the cases above where they share one strict expression `(+ (Ctr.tick)
           (od …))`. `ev`'s base-case branch performs `(Fresh.next)` while its recursive branch calls the
           partner `(od …)`: `(def (ev n) (if (= n 0) (Fresh.next) (od (- n 1))))`, `(def (od n) (if (= n 0)
           0 (ev (- n 1))))`. Under the state-threading handler this recurses `ev 2 -> od 1 -> ev 0`, where
           the base case fires `Fresh.next`: seeded 42, the arm resumes `s + 1` = 43. Pins that effect-context
           specialization ties the `ev#ctx`/`od#ctx` memo knot even when each branch of a cycle def EMBEDS
           the threaded state independently (the performing branch substitutes it into the resume value, the
           mutual-call branch appends it as a trailing state argument) — each branch needs its own copy of
           the state reference, not a shared one. Was a compile-time leak of the internal `ev#eff…$s0`
           specialization name (a `cdz check`-clean / `compile`-fail gap); now folds to 43.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (ev (: n Int64)) (if (= n 0) (Fresh.next) (od (- n 1))))
      (def (od (: n Int64)) (if (= n 0) 0 (ev (- n 1))))
      (def (main) (handle Fresh 42 ((next () s (resume (+ s 1) s))) (ev 2)))
      (export main)))
  (output (: 43 Int64)))

(case
  "a mutually-recursive group with a BRANCH-PERFORM sharing a strict expr with the mutual call declines cleanly (adv-69 rw4 sub-face)"
  (doc
    "adv-69 recursive-branch-perform, MUTUAL-SCC face (v-effects self-probe 2026-08-04, breaker rw4).
           CONTRAST the two folding cases above (perform and mutual call in SEPARATE branches — no shared
           strict context, mutually exclusive): here the branch-perform and the mutual call SHARE one strict
           expression — `(def (even-w n) (if (= n 0) 0 (+ (if true (St.get) 0) (odd-w (- n 1)))))` (and the
           odd-w twin). The `(if true (St.get) 0)` branch-perform is a strict operand of `+` ALONGSIDE the
           mutual call `(odd-w …)`. The single-return specialization threads the branch perform against the
           INCOMING state, but the advance is branch-local and the recursion carries the incoming state
           forward, so it drops across the cycle: seeded St=1 it ran 3 (three gets all read seed 1), correct
           is 6 (1+2+3). DECLINE cleanly (safe floor) — a full fold needs the branch-perform lifted before
           specialization. Detected by `branch_perform_coexists_with_reentrant_call` (a branch-performing
           conditional as a strict operand alongside a re-entrant self/mutual call), keyed via
           `contains_recursive_call` so it covers the mutual SCC, not just direct self-recursion. This is the
           MUTUAL-SCC face ONLY; the SELF-recursive faces (bare `(walk n)` with the same `+` shape) are
           rewritten by the load-time accum pass and are tracked SEPARATELY (still open). Grades TODO on all
           backends; flips to 6 PASS when the branch-perform-before-recursion fold lands.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (even-w (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (odd-w (- n 1)))))
      (def (odd-w (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (even-w (- n 1)))))
      (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (even-w 3)))
      (export main)))
  (output (: 6 Int64)))

(case
  "a MATCH-dispatched mutual group with the perform in one arm and the mutual call in another folds"
  (doc
    "The `match` companion of the separate-branch mutual case above — the cycle dispatches on a
           `match` rather than an `if`, with the perform in one arm and the mutual call in another. `(def (ev
           n) (match n (0 (Fresh.next)) (_ (od (- n 1)))))`, `(def (od n) (match n (0 0) (_ (ev (- n 1)))))`.
           Same recursion `ev 2 -> od 1 -> ev 0`, the `0` arm fires `Fresh.next`: seeded 42, the arm resumes
           `s + 1` = 43. Pins that each MATCH ARM (like each `if` branch) gets its own copy of the threaded
           state reference — the performing arm substitutes it into the resume value while the mutual-call arm
           appends it as a trailing state argument, and a single-parent arena would otherwise orphan a shared
           state-ref node, leaking the internal `ev#eff…$s0` name. Was the match-dispatch analogue of the
           if-branch leak; now folds to 43.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (ev (: n Int64)) (match n (0 (Fresh.next)) (_ (od (- n 1)))))
      (def (od (: n Int64)) (match n (0 0) (_ (ev (- n 1)))))
      (def (main) (handle Fresh 42 ((next () s (resume (+ s 1) s))) (ev 2)))
      (export main)))
  (output (: 43 Int64)))

(case
  "a mutually-recursive fresh-id walk assigns a fresh id at each node and sums them"
  (doc
    "The essential compiler-PASS idiom the mutual-effect fixes unblock: a `Fresh` gensym threaded
           through a MUTUALLY-recursive walk over a tree — `node` visits a node (assigns it `(Fresh.next)`)
           and recurses into its `children`, which recurse back into `node`. This is exactly the shape an
           AST-relabelling pass takes (`relabel(node)` ↔ `relabel-list(children)`), with the fresh-id counter
           threaded by the handler rather than passed as an explicit parameter. `Fresh` seeded 0, arm `(next
           () s (resume s (+ s 1)))` hands back `s` and threads `s + 1`. `(node 5)` visits the node chain
           `node 5 -> children 4 -> node 3 -> children 2 -> node 1 -> children 0`, firing `Fresh.next` at
           each `node` step (n = 5, 3, 1) — reading 0, 1, 2 — and summing them along the way: `node 1` =
           `2 + 0` = 2, `node 3` = `1 + 2` = 3, `node 5` = `0 + 3` = 3. Pins that effect-context
           specialization threads a fresh-name generator through a mutual tree walk end to end — the pass a
           self-hosting compiler runs over its own AST (each perform reads the PRE-recursion state, the
           sound pre-order shape). Both backends agree.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def (node (: n Int64)) (if (= n 0) (Fresh.next) (+ (Fresh.next) (children (- n 1)))))
      (def (children (: n Int64)) (if (= n 0) 0 (node (- n 1))))
      (def (main) (handle Fresh 0 ((next () s (resume s (+ s 1)))) (node 5)))
      (export main)))
  (output (: 3 Int64)))

(case
  "a mutual walk where BOTH partners perform threads one shared counter across the cycle"
  (doc
    "The fresh-id-walk case above reaches the effect through ONE partner (`node` performs, `children`
           only dispatches). Here BOTH cycle defs perform the same effect — each reads a fresh id before
           recursing into the other — so the shared handler counter is advanced by BOTH specializations
           (`node#ctx` AND `children#ctx`), and the reads interleave along the cycle. `Fresh` seeded 0, arm
           `(next () s (resume s (+ s 1)))`. `(node 3)`: `node` reads id 0 then `+ children 2`; `children 2`
           reads id 1 then `+ node 1`; `node 1` reads id 2 then `+ children 0`; `children 0` = 0. So `node 1`
           = `2 + 0` = 2, `children 2` = `1 + 2` = 3, `node 3` = `0 + 3` = 3. Pins that effect-context
           specialization threads ONE shared state slot correctly when BOTH members of a mutual group
           perform (not only when the effect is reached through a single partner) — each partner's
           specialization carries the threaded counter and the interleaved reads advance it in cycle order.
           Both backends agree.")
  (input
    (do
      (effect Fresh (op next (-> Int64)))
      (def
        (node (: n Int64))
        (if (= n 0) (Fresh.next) (let ((v (Fresh.next))) (+ v (children (- n 1))))))
      (def (children (: n Int64)) (if (= n 0) 0 (let ((w (Fresh.next))) (+ w (node (- n 1))))))
      (def (main) (handle Fresh 0 ((next () s (resume s (+ s 1)))) (node 3)))
      (export main)))
  (output (: 3 Int64)))

(case
  "a recursive function sums a range it walks by performing a fresh-index effect"
  (doc
    "Witnesses the recursive-effect idiom folding a real accumulator across a self-recursive
           walk: `Idx` supplies a descending index (seeded 3, each `next` hands back `s` and threads
           `s - 1`), and `sum-down` recurses — performing `(Idx.next)` once per step and adding it
           to the sum of the rest — until the index reaches 0. The performs read 3, 2, 1, 0, so the
           walk computes `3 + 2 + 1 + 0` = 6. This is a self-recursive consumer driven entirely by a
           stateful effect (the counter is not a parameter — it is threaded by the handler across the
           recursion), the essential shape of a compiler pass that walks a structure while pulling
           fresh state. Being recursive-while-performing, it declines under inlining-only resolution
           and needs effect-context monomorphization; the recorded output 6 is the realized
           semantics.")
  (input
    (do
      (effect Idx (op next (-> Unit Int64)))
      (def (sum-down) (let ((i (Idx.next))) (if (= i 0) 0 (+ i (sum-down)))))
      (def (main) (handle Idx 3 ((next (u) s (resume s (- s 1)))) (sum-down)))
      (export main)))
  (output (: 6 Int64)))

(case
  "a branch-performing conditional in a self-recursive performer threads the advance across recursion (rw1)"
  (doc
    "The recursive-branch-perform fix (v-effects self-probe, breaker rw1, operator-prioritized as a HIGH
           miscompile): a discharged perform inside a conditional BRANCH `(if true (St.get) 0)` that is a strict
           operand alongside the self-call `(walk (- n 1))` — `(+ (if true (St.get) 0) (walk (- n 1)))`. The
           branch perform advances the handler state, and the sibling recursion must see that advance. This was
           a SILENT MISCOMPILE — `thread_bounded`'s `If` arm returned the post-CONDITION state as the `if`'s
           out-state (branch advances unmerged), so the walk reseeded from the stale pre-branch state and every
           step read the seed: seeded 1 it ran 3 (1+1+1), correct is 6 (1+2+3). FIXED by MERGING the per-branch
           out-states into a conditional-valued out-state `(if cond then-out else-out)` so the sibling recursion
           threads the branch's advance (gated on a pure condition + `#cv`-free branch out-states to stay
           arena-safe). Now folds to 6 on all backends.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (+ (if true (St.get) 0) (walk (- n 1)))))
      (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (output (: 6 Int64)))

(case
  "a RUNTIME-conditioned branch perform in a self-recursive performer threads the advance (rw3)"
  (doc
    "The runtime-condition face of the recursive-branch-perform fix (rw3): the branch conditional's test
           is a RUNTIME value `(> n 0)` rather than a constant, so the fold cannot key on a foldable condition —
           the per-branch out-state merge handles it uniformly. Same shape/values as rw1 (seeded 1 → 6), the
           branch perform's advance threads to the sibling recursion via the conditional-valued out-state.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (+ (if (> n 0) (St.get) 0) (walk (- n 1)))))
      (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (output (: 6 Int64)))

(case
  "a HEAP-state branch perform in a self-recursive performer threads the pushes across recursion (rw5)"
  (doc
    "The heap-state (data-loss) face of the recursive-branch-perform fix (rw5): the handler state is a
           LIST accumulator and the branch perform conditionally pushes onto it; the branch advance is the
           `List.push`, which the recursion must carry forward. Pre-fix the pushes were LOST (the branch
           out-state dropped, so each step pushed against the empty seed → count 0); the per-branch out-state
           merge threads the growing list, so the three conditional pushes accumulate → length 3. The data-loss
           twin of rw1 — a wrong heap value, not just a stale scalar.")
  (input
    (do
      (effect Log (op add (-> Int64 Int64)) (op count (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) (Log.count) (do (if true (Log.add n) 0) (walk (- n 1)))))
      (def
        (main)
        (handle
          Log
          #list()
          ((add (v) s (resume v (List.push s v))) (count (u) s (resume (List.len s) s)))
          (walk 3)))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a MATCH-arm perform in a self-recursive performer threads the advance across recursion (rw-match)"
  (doc
    "The MATCH-arm face of the recursive-branch-perform fix (the `if`-branch cases rw1/rw3/rw5 are its
           `if` siblings): the discharged perform is in a `match` ARM body — `(+ (match true (_ (St.get)))
           (walk (- n 1)))` — a strict operand alongside the self-call. Same drop as rw1: `thread_bounded`'s
           `Match` arm returned the post-SCRUTINEE state as the match's out-state (arm advances unmerged), so
           the sibling recursion reseeded from the stale pre-arm state — seeded 1 it ran 3, correct is 6. FIXED
           by the `Match` arm analogue of the `if` per-branch out-state merge: the arm out-states merge into a
           `(match scrut (pat arm-out)…)`-valued out-state (gated on a pure scrutinee + `#cv`-free arm
           out-states, same as the `if` arm). Now folds to 6 on all backends. Pins that the merge covers BOTH
           conditional forms (`if` and `match`).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (+ (match true (_ (St.get))) (walk (- n 1)))))
      (def (main) (handle St 1 ((get (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (output (: 6 Int64)))

(case
  "a recursive function with an annotated parameter walks and bails through an abortive handler"
  (doc
    "The recursive-effect idiom with an ANNOTATED parameter and an ABORTIVE discharge. `walk` takes
           `(: n Int64)` and tail-recurses, counting `n` down; at zero it performs `(Bail.bail 99)`, whose
           handler arm never resumes — so the abort at the base ABANDONS the walk and its value 99 becomes
           the handle's value (propagating up the tail calls, no state threaded). Witnesses that recursive
           effect-context specialization handles an annotated parameter (not only a bare name) — the
           synthesized specialized function re-annotates the parameter with its solved type. `(walk 3)`
           ticks 3→2→1→0 then bails → 99 (`DESIGN-effects-rcdzc.md` §4.2, §4.3).")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (walk (: n Int64)) (if (= n 0) (Bail.bail 99) (walk (- n 1))))
      (def (main) (handle Bail 0 ((bail (n) s n)) (walk 3)))
      (export main)))
  (output (: 99 Int64)))

(case
  "an abortive perform in a recursive callee with a PENDING continuation in the handle body abandons it"
  (doc
    "The soundness companion of the tail-walk-bail above (which is the handle body's TAIL, no pending
           work). Here the recursive-abortive callee's result feeds a PENDING continuation in the handle
           body: `(+ (go 2) 999999)` where `(go n)` tail-recurses and bails at zero. The abort MUST abandon
           the `(+ _ 999999)` — bail's arm value 500 becomes the handle's value, +7 outside → 507. It must
           NOT flow 500 INTO the pending `+ 999999` (the adv-52 miscompile: 500+999999+7 = 1000506, a silent
           wrong value that appeared on all backends). Abandoning past a pending continuation at the OUTER
           call site is folded by the non-local-exit TAGGED-RETURN CC: `reduce_handle` forces `go` into
           tagged mode (`go#eff` returns `#tuple(tag value)`) and short-circuits the pending op on the abort
           tag — `(let ((r (go#eff 2 0))) (if (= (. r 0) 1) (. r 1) (+ (. r 1) 999999)))` — so the arm value
           500 becomes the handle value and `+ 7` → 507, never 1000506. (breaker adv-52; the mutual-recursion
           neighbor still declines — its cross-def SCC tagged threading is a later increment.)")
  (input
    (do
      (effect Mx (op bail (-> Int64 Int64)))
      (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
      (def (main) (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 2) 999999)) 7))
      (export main)))
  (output (: 507 Int64)))

(case
  "a RUNTIME branch selects between an abortive perform and a plain value per call"
  (doc
    "The per-call abort selection (the branch-abort pins use const conditions): `(if (> n 0)
           (+ (Bail.out n) 999) (- 0 n))` — n=4 takes the abortive path (the arm multiplies, the +999
           continuation is abandoned → 40); n=-6 takes the plain path (6). ONE compiled body must both
           abandon and complete depending on the call — an emit specializing the handle to always-abort
           (or always-resume) breaks the other call.")
  (input
    (do
      (effect Bail (op out (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle Bail 0 ((out (v) s (* v 10))) (if (> n 0) (+ (Bail.out n) 999) (- 0 n))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 40 Int64))
  (call main (: -6 Int64))
  (output (: 6 Int64)))

(case
  "an abortive perform MID-WALK carries the accumulated state out as its argument"
  (doc
    "The annotated-walk bail above aborts at the BASE with a const; here the abort fires MID-walk
           at a sentinel (n=2) and its ARGUMENT is the accumulator built so far — walk(5,0) accumulates
           5+4+3=12 before bailing with 12; walk(1,0) never hits the sentinel and returns normally (1).
           The abort value carries live loop state out through the abandoned frames (the early-exit-with-
           partial-result idiom); an abort that read a stale accumulator drifts.")
  (input
    (do
      (effect Bail (op out (-> Int64 Int64)))
      (def
        (walk (: n Int64) (: acc Int64))
        (if (< n 1) acc (if (= n 2) (Bail.out acc) (walk (- n 1) (+ acc n)))))
      (def (main (: n Int64)) (handle Bail 0 ((out (v) s v)) (walk n 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "a recursive function threads two nested handlers' states at once"
  (doc
    "Witnesses that effect-context resolution threads EACH enclosing handler's state
           independently across a recursion (capabilities-and-effects.md #A Handler Threads State
           Across The Operations It Discharges composed with #Handler Resolution Is Dynamic In
           Extent): `loop` recurses under TWO nested stateful handlers — `A` (a countdown seeded 3,
           `tick` hands back `s` and threads `s - 1`) governs the recursion depth, and `B` (an
           accumulator seeded 0, `bump` hands back `s` and threads `s + 10`) is folded across the
           steps. Each non-zero tick performs `B.bump` and adds it to the recursion's tail: the ticks
           read 3, 2, 1, 0 (three non-zero), and the bumps read 0, 10, 20, so the sum is
           `0 + 10 + 20 + 0` = 30. Both states are live on the call stack SIMULTANEOUSLY — the
           mechanism must give each handler context its own threaded state (a single shared slot per
           effect would clobber when the recursion re-enters), which is exactly what threading each
           context as a distinct hidden parameter/return provides. This is the essential shape of a
           self-hosting compiler pass that walks a structure while folding more than one piece of
           state (a fresh-name counter AND a diagnostics list). Recursive-while-performing, so it
           needs effect-context monomorphization; the recorded output 30 is the realized semantics.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)))
      (effect B (op bump (-> Unit Int64)))
      (def (loop) (if (= (A.tick) 0) 0 (+ (B.bump) (loop))))
      (def
        (main)
        (handle
          B
          0
          ((bump (u) s (resume s (+ s 10))))
          (handle A 3 ((tick (u) s (resume s (- s 1)))) (loop))))
      (export main)))
  (output (: 30 Int64)))

(case
  "a recursive walk threads THREE nested handlers' states at once"
  (doc
    "Generalizes the two-nested-handler case to THREE: one recursive `walk` performs `A.a`, `B.b`, and
           `C.c` at each step, under three nested stateful handlers, and each handler's state threads
           INDEPENDENTLY — the merged effect context carries THREE distinct slots (a shared per-effect slot
           would clobber on re-entry). Each handler hands back `s` and threads `s + 1`: seeded A=100, B=200,
           C=300, over `(walk 2)`, the `A.a` reads are 100, 101 (sum 201), the `B.b` reads 200, 201 (401),
           the `C.c` reads 300, 301 (601), so the total is `201 + 401 + 601` = 1203. Pins that effect-context
           monomorphization scales past two effects — N handlers over one recursive walk thread N distinct
           states — the shape of a self-hosting pass folding several pieces of context (a name counter, a
           diagnostics list, a symbol table) at once. Identical on both backends.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect C (op c (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (+ (A.a) (+ (B.b) (+ (C.c) (walk (- n 1)))))))
      (def
        (main)
        (handle
          A
          100
          ((a (u) s (resume s (+ s 1))))
          (handle
            B
            200
            ((b (u) s (resume s (+ s 1))))
            (handle C 300 ((c (u) s (resume s (+ s 1)))) (walk 2)))))
      (export main)))
  (output (: 1203 Int64)))

(case
  "a handler arm capturing an enclosing fn param folds under a multi-arm nested handler"
  (doc
    "A handler arm may reference a name bound by an ENCLOSING function — here `converse`'s arm
           `(resume p 0)` captures `run-with`'s parameter `p`, NOT the arm's own params/state. When the
           recursive driver `run` performs BOTH effects (so the fold takes the two-nested-states MERGE path)
           AND the inner handler is MULTI-ARM (`Tools` declares `dispatch`+`done`), the captured `p` used to
           be LOST — the synthesized `run#ctx` carried the driver's params and the threaded states but not
           `p`, so the spliced free `p` re-resolved against `run#ctx`'s signature (which lacked it) and the
           whole program declined `CDZ0101 unbound name p` (a valid program falsely refused; found by the
           agent-harness dogfood). The fix threads a captured enclosing-fn param as an EXTRA specialized
           parameter (after the originals, before the trailing states), passed UNCHANGED at every call since
           it is constant across the recursion. `run-with(3)` seeds `run(3,0)`; each step adds
           `converse→p (=3)` and `dispatch→1`, over three steps: `(0+3+1)+(3+1)+(3+1)` threaded = 12. This is
           the shape of a self-hosting pass whose handler closes over a config parameter (a routing table, a
           fuel budget) while walking a structure under more than one effect.")
  (input
    (do
      (effect Model (op converse (-> Int64 Int64)))
      (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) acc (run (- fuel 1) (+ acc (+ (Model.converse fuel) (Tools.dispatch fuel))))))
      (def
        (run-with (: p Int64))
        (handle
          Model
          0
          ((converse (q) s (resume p 0)))
          (handle Tools 0 ((dispatch (a) s (resume 1 0)) (done (a) s (resume a 0))) (run 3 0))))
      (def (main) (run-with 3))
      (export main)))
  (output (: 12 Int64)))

(case
  "an inner handler's INIT state is computed by performing an enclosing effect"
  (doc
    "The seed of an inner handler is itself a PERFORM of an OUTER effect — the two handlers compose
           through the init position, not just the body. `(handle Seed 0 ((s (u) t (resume 50 t))) (handle
           Ask (Seed.s) …))`: the inner `Ask` handler's INIT is `(Seed.s)`, discharged by the enclosing
           `Seed` handler to 50. So `Ask` is seeded 50, and `(Ask.get)` (its arm resumes the state) reads 50.
           Pins that a handler init is an ordinary strict expression the outer handler's fold threads — the
           inner handler's starting state can be COMPUTED by an effect, the shape of a pass whose scratch
           state is initialized from a queried piece of outer context.")
  (input
    (do
      (effect Seed (op s (-> Unit Int64)))
      (effect Ask (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          Seed
          0
          ((s (u) t (resume 50 t)))
          (handle Ask (Seed.s) ((get (u) st (resume st st))) (Ask.get))))
      (export main)))
  (output (: 50 Int64)))

(case
  "a mutually-recursive group threads two nested handlers' states at once"
  (doc
    "The two-nested-handler state-threading of the case above, but over a MUTUALLY-RECURSIVE group
           rather than a single self-recursive `loop` — composing merge (two effects, two handler contexts)
           WITH mutual specialization (`ev`/`od`, each performing a DIFFERENT effect). `ev` performs `A.tick`
           and recurses through `od`; `od` performs `B.bump` and recurses through `ev`; both handler
           contexts must thread INDEPENDENTLY across the alternation. `A` is a countdown seeded 3 (`tick`
           hands back `s`, threads `s - 1`), `B` an accumulator seeded 0 (`bump` hands back `s`, threads
           `s + 10`). Along `ev(4) → od(3) → ev(2) → od(1) → ev(0)=0`, the A-ticks read 3 then 2 (in `ev`)
           and the B-bumps read 0 then 10 (in `od`), so the strict-spine sum is `3 + 0 + 2 + 10 + 0` = 15.
           Each specialized function (`ev#ctx`/`od#ctx`) must carry BOTH threaded states as distinct hidden
           slots — a shared per-effect slot would clobber when the mutual recursion re-enters — pinning
           that merge (`merged_nested_ctx`) and mutual-group specialization cooperate. Recursive-while-
           performing across two effects, so it needs effect-context monomorphization
           (`DESIGN-effects-rcdzc.md` §4.2, §4.3).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)))
      (effect B (op bump (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (+ (A.tick) (od (- n 1)))))
      (def (od (: n Int64)) (+ (B.bump) (ev (- n 1))))
      (def
        (main)
        (handle
          A
          3
          ((tick (u) s (resume s (- s 1))))
          (handle B 0 ((bump (u) s (resume s (+ s 10)))) (ev 4))))
      (export main)))
  (output (: 15 Int64)))

(case
  "a non-tail-resumptive outer handler reduces a reducible inner handle before its own fold"
  (doc
    "Nested handlers of DISTINCT effects where the OUTER handler's arm resumes NON-tail. The
           inside-out reduction reduces the inner handle only while THREADING the outer body — which
           requires the outer arm to be tail-resumptive. When the outer arm is non-tail, its delimited
           continuation is the E5 pure one-hole fold, which sees the whole inner `handle` as an opaque
           non-uniform continuation and would decline. Reducing the inner (tail-resumptive) handler FIRST
           discharges `B`: `(handle B 0 ((b (u) t (resume 20 t))) (+ (A.a) (B.b)))` folds `B.b` to its
           resume value 20 (B threads no observable effect and A.a is left untouched as B does not discharge
           it), leaving `(+ (A.a) 20)`. That is a single `A`-perform in a pure one-hole context `C = (+ □
           20)`, so the outer arm `(+ 1 (resume 10 s))` folds to `(+ 1 (+ 10 20))` = 31. Sound and
           frame-free: reducing the inner handler is the same reduction the threading path performs, only
           sequenced before the outer fold rather than during it.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          0
          ((a (u) s (+ 1 (resume 10 s))))
          (handle B 0 ((b (u) t (resume 20 t))) (+ (A.a) (B.b)))))
      (export main)))
  (output (: 31 Int64)))

(case
  "a recursive function that installs a fresh handler on each call grows its context without bound"
  (doc
    "Witnesses the LIMIT of effect-context monomorphization: `loop` wraps its own recursive
           call in a FRESH `(handle …)`, so each recursive call runs under a handler context one
           frame deeper than the last. The performed `Fresh.next` at the base resolves to the
           INNERMOST (most recent) handler — a shadowing that is well-defined operationally (the run
           counts a fresh supply seeded 100, reads 100 once, so the result is 100) — but there is no
           FINITE set of handler contexts to specialize the function against: the context grows by
           one frame per recursion. A compiler that discharges recursive effects by
           effect-context monomorphization (emitting the function once per handler context) cannot
           cover an unbounded family of contexts, so it DECLINES rather than looping forever building
           specializations (reject-don't-miscompile; the seed bounds the handler-context depth and
           declines past the bound — it must never overflow the compiler, on any target). A generation
           that reifies
           continuations as data (a general one-shot / scheduler tier) discharges this; the recorded
           output 100 is that semantics. This case guards against the compiler crashing on
           unbounded context growth.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (def
        (loop n)
        (handle
          Fresh
          100
          ((next (u) s (resume s (+ s 1))))
          (if (= n 0) (Fresh.next) (loop (- n 1)))))
      (def (main) (loop 2))
      (export main)))
  (output (: 100 Int64)))

; --- Rejections the routing model introduces ----------------------------------------------------
; An effect declaration is the CLOSED set of an effect's operations, so a handler arm for an operation
; the effect does not declare is rejected (CDZ0403), and an operation reached with neither an enclosing
; handler nor an enclosing entrypoint delegation — so it would escape ungranted — is rejected (CDZ0401,
; the single "no home" check that merges the former undischarged-intra and undeclared-host rejections).
; These are the compile-time checks that keep "no ambient authority" a property of the source
; (capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time Error, #A Handler Arm Names An
; Operation Its Effect Declares).
; Performing an operation is TYPED exactly as an ordinary function application: its arguments are checked
; against the operation's declared parameter types (capabilities-and-effects.md #Performing An Operation Is
; Typed And Contributes To The Row: "Performing an operation MUST check its arguments against the operation's
; declared parameter types … so that an effect operation is typed exactly as an ordinary function
; application is"). So performing `E.op` — declared `(-> Int64 Int64)` — on a Bool argument is a type
; mismatch, rejected (CDZ0203) exactly as an ordinary `(f true)` on an Int64-parameter `f` is. A compiler
; that lowers the perform without checking the argument against the declared parameter type MISCOMPILES: it
; feeds the Bool (or worse, a String) through the op's Int64 slot and produces a garbage value rather than
; rejecting — `(E.op "str")` returns a nonsense integer. A generation that does not yet type-check a perform's
; arguments declines rather than emitting the mistyped operation.
(case
  "performing an operation with an argument of the wrong type is a type error"
  (doc
    "`E.op` is declared `(-> Int64 Int64)`, so performing `(E.op true)` supplies a Bool where the
           operation's parameter type is Int64 — a type mismatch the compiler MUST reject (CDZ0203,
           capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row: a
           perform's arguments are checked against the declared parameter types, exactly as an ordinary
           function application's are — `(f true)` on an Int64-parameter `f` is rejected the same way,
           CDZ0203). Pins that an effect operation's arguments are type-checked: a compiler that lowers the
           perform without checking feeds the Bool through the op's Int64 slot and produces a wrong value
           (and a String argument yields a garbage integer). A generation that does not yet check a
           perform's arguments declines rather than emitting the mistyped operation.")
  (input
    (do
      (effect E (op op (-> Int64 Int64)))
      (def (main) (handle E unit ((op (n) s (resume n s))) (E.op true)))
      (export main)))
  (error CDZ0203 (message "operation `op`") (message "Int64") (message "Bool")))

; The perform-argument message NAMES the operation + its expected/actual types (the perform-site analogue of
; the member-op wrong-type-arg message), not the generic "Int64 and Bool must be the same type here" internal
; clash. When the performed value and the declared argument are SAME-KIND compounds that differ structurally
; (a record field-set diff), it appends the minimal-conflict DELTA — WHICH field is wrong — the annotation /
; operator-arg / peer-join sites already carry. A SCALAR mismatch (Int64 vs Bool, no structural delta) keeps
; the bare message with no delta tail. (Migrated from rcdzc a_perform_arg_structural_mismatch_names_the_delta_
; not_just_the_types + the message half of performing_an_operation_with_a_wrong_type_argument_is_rejected.)
(case
  "a structurally-mismatched perform argument names the operation and the field-level delta"
  (input
    (do
      (effect Log (op put (-> (Record (: x Int64)) Unit)))
      (def (main) (handle Log unit ((put (r) s (resume unit s))) (Log.put #record((= y 2)))))
      (export main)))
  (error CDZ0203 (message "operation `put`") (message "was performed") (message "field `x`")))

(case
  "a scalar-mismatched perform argument names the operation with no structural-delta tail"
  (input
    (do
      (effect Log (op put (-> Int64 Unit)))
      (def (main) (handle Log unit ((put (r) s (resume unit s))) (Log.put true)))
      (export main)))
  (error CDZ0203 (message "operation `put`") (not " — ")))

; A match-arm GUARD condition must be SIDE-EFFECT-FREE (operator directive): a guard is a boolean predicate
; the pattern engine may evaluate speculatively or repeatedly, so a PERFORM in a guard cond has no
; well-defined evaluation count/order and is a compile error (CDZ0407), regardless of the guarded pattern's
; shape (irrefutable name or refutable literal). A NON-performing guard is fine, and a perform in the ARM
; BODY (or scrutinee) is fine — only the GUARD position is forbidden. (Migrated from rcdzc
; a_perform_in_a_match_guard_is_cdz0407_guards_must_be_side_effect_free.)
(case
  "a performing guard on an irrefutable inner pattern is CDZ0407"
  (input
    (do
      (effect Ask (op get (-> Int64)))
      (def
        (main)
        (handle
          Ask
          5
          ((get () s (resume s (- s 1))))
          (match 9 ((guard n (> (Ask.get) 3)) 100) (n 200))))
      (export main)))
  (error CDZ0407))

(case
  "a performing guard on a refutable inner pattern is CDZ0407 too"
  (input
    (do
      (effect Ask (op get (-> Int64)))
      (def
        (main)
        (handle
          Ask
          5
          ((get () s (resume s (- s 1))))
          (match 9 ((guard 9 (> (Ask.get) 3)) 100) (n 200))))
      (export main)))
  (error CDZ0407))

(case
  "a non-performing guard under a handle compiles and selects"
  (input
    (do
      (effect Ask (op get (-> Int64)))
      (def
        (main)
        (handle Ask 5 ((get () s (resume s (- s 1)))) (match 9 ((guard n (> n 3)) 100) (n 200))))
      (export main)))
  (call main)
  (output (: 100 Int64)))

(case
  "a perform in an arm BODY (not the guard) under a handle folds and runs"
  (input
    (do
      (effect Ask (op get (-> Int64)))
      (def (main) (handle Ask 5 ((get () s (resume s (- s 1)))) (match 9 (n (Ask.get)))))
      (export main)))
  (call main)
  (output (: 5 Int64)))

; The retired effect-name-less handle shape `(handle <seed> (arm…) body)` — no effect in the head, the arm
; op written dotted — is NOT the canonical handler form, so it is rejected CDZ0201 ("this handle is not in
; canonical form"), pointing at the canonical shape. The rejected handle never resolves AS a handler, so its
; body-perform would ALSO trip the entrypoint no-home CDZ0401 — a CONSEQUENCE, deduped so the author sees the
; ONE "make it canonical" error, not a misdirecting "you have no handler" (they do — it is just the old shape).
; (Migrated from rcdzc a_noncanonical_handle_is_rejected_as_one_cdz0201.)
(case
  "a non-canonical effect-name-less handle is rejected, with no consequent no-home error"
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def (main) (handle 0 ((Bail.bail (n) s n)) (+ (Bail.bail 7) 100)))
      (export main)))
  (error CDZ0201 (message "this handle is not in canonical form") (no-other-errors)))

; A `resume` value whose type mismatches the operation's RESULT type is CDZ0201. When the mismatch is a
; numeric COERCION (`(resume x s)` with x:Int8, op result Int64), it carries the `(Int64.of …)` wrap fix —
; the resume position joins the argument / annotation / let-binder / ctor-payload sites that offer the same
; of-conversion. A NON-coercible mismatch (Bool where Int64) carries no fix (no conversion applies). (Migrated
; from rcdzc a_mistyped_resume_reports_one_error_with_a_coercion_fix_when_applicable — the reject + fix facets;
; the "exactly ONE error" dedup of the UNCODED not-yet-reducible decline stays a rust residual.)
(case
  "a coercible mistyped resume value carries an of-conversion wrap fix"
  (input
    (do
      (effect E (op a (-> Int64 Int64)))
      (def (main (: x Int8)) (handle E unit ((a (n) s (resume x s))) (E.a 1)))
      (export main)))
  (error CDZ0201 (fix (kind wrap) (replacement-contains "(Int64.of "))))

(case
  "a non-coercible mistyped resume value carries no coercion fix"
  (input
    (do
      (effect E (op a (-> Int64 Int64)))
      (def (main) (handle E unit ((a (n) s (resume true s))) (E.a 1)))
      (export main)))
  (error CDZ0201 (no-fix)))

; The perform-argument check must fire for EVERY declared parameter type, not only Int64. An operation
; declared `(-> String Unit)` performed on an Int64 argument — `(E.emit 42)` — is the same type mismatch
; as the Int64-parameter case above and MUST be rejected (CDZ0203). This is the STRING-parameter sibling:
; a compiler whose perform lowering dispatches on the DECLARED parameter type (routing a String-parameter
; op to a string-argument path) before checking the ARGUMENT's actual type skips the check when the
; declared parameter is String, and feeds the Int through the op's String slot — the handler arm binds `s`
; to `42` typed as a String, so `(E.emit 42)` runs to `unit` (and a downstream `(String.byte-len s)` in
; the arm reads a non-String value). The Int64-parameter op catches its bad argument (`(E.op true)` above);
; the String-parameter op must catch its bad argument identically, or the argument check is not "exactly as
; an ordinary function application" for every parameter type. A generation that does not yet check a
; String-parameter op's argument declines rather than binding the mistyped value into the handler arm.
(case
  "performing a string-parameter operation with a non-string argument is a type error"
  (doc
    "`E.emit` is declared `(-> String Unit)`, so performing `(E.emit 42)` supplies an Int64 where the
           operation's parameter type is String — a type mismatch the compiler MUST reject (CDZ0203,
           capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row),
           exactly as the Int64-parameter case `(E.op true)` above is. Pins that the perform-argument check
           fires for a STRING-declared parameter too, not only Int64: a compiler that dispatches a perform
           on the declared parameter type — routing a String-parameter op to a string-argument path —
           before checking the argument's actual type skips the check for a String parameter and binds the
           Int `42` into the handler arm as a String, so `(E.emit 42)` runs to `unit` instead of being
           rejected. The argument check must be uniform across parameter types. A generation that does not
           yet check a String-parameter op's argument declines rather than binding the mistyped value.")
  (input
    (do
      (effect E (op emit (-> String Unit)))
      (def (main) (handle E unit ((emit (s) st (resume unit st))) (E.emit 42)))
      (export main)))
  (error CDZ0203))

; The perform-argument check must also fire for a COMPOUND declared parameter type, not only the scalar
; types Int64 (above) and String (above). An operation declared `(-> (List Int64) Unit)` performed on an
; Int64 argument — `(E.put 42)` — is the same type mismatch and MUST be rejected (CDZ0203). This is the
; COMPOUND-parameter sibling of the two scalar-parameter cases: a compiler whose perform check compares the
; argument only against a scalar Kind skips the check when the declared parameter is a compound, binds the
; Int `42` into the handler arm typed as a `List Int64`, and `(E.put 42)` runs to `unit` (a downstream
; `(List.len xs)` in the arm then reads a non-list value). A tuple argument where a list is declared, or a
; wrong element type, slips through the same way. The argument check must be uniform across ALL parameter
; type shapes — scalar and compound alike. A generation that does not yet check a compound-parameter op's
; argument declines rather than binding the mistyped value into the handler arm.
(case
  "performing an operation with a wrong-type argument for a compound parameter is a type error"
  (doc
    "`E.put` is declared `(-> (List Int64) Unit)`, so performing `(E.put 42)` supplies an Int64 where
           the operation's parameter type is the compound `List Int64` — a type mismatch the compiler MUST
           reject (CDZ0203, capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To
           The Row), exactly as the Int64-parameter (`(E.op true)`) and String-parameter (`(E.emit 42)`)
           cases above are. Pins that the perform-argument check fires for a COMPOUND declared parameter
           too, not only scalars: a compiler that compares the argument only against a scalar Kind skips the
           check for a compound parameter and binds the Int `42` into the handler arm typed as a `List
           Int64`, so `(E.put 42)` runs to `unit` (a downstream `(List.len xs)` reads a non-list value). The
           argument check must be uniform across all parameter type shapes. A generation that does not yet
           check a compound-parameter op's argument declines rather than binding the mistyped value.")
  (input
    (do
      (effect E (op put (-> (List Int64) Unit)))
      (def (main) (handle E unit ((put (xs) s (resume unit s))) (E.put 42)))
      (export main)))
  (error CDZ0203))

(case
  "an operation with a TUPLE parameter binds the compound and the arm projects it"
  (doc
    "The positive companion: an operation whose declared PARAMETER is a compound `(Tuple Int64
           Int64)` binds the whole tuple to the arm's parameter, which the arm projects. `Add.sum : (->
           (Tuple Int64 Int64) Int64)`; performed as `(Add.sum (tuple 3 4))`, the arm binds `p` to the pair
           and resumes with `(+ (. p 0) (. p 1))` = 7. Pins that a compound OP parameter threads through the
           fold and is projectable in the arm (the type-position spelling is capital `Tuple`, the type
           constructor — lowercase `tuple` is the value constructor). NOTE: the arm here projects `p` from a
           pure `(tuple 3 4)` argument; when the tuple argument itself PERFORMS and the arm uses `p` more
           than once, the fold declines rather than duplicate the perform (see the effect-duplication guard
           — a substituted performing argument copied per param-use would re-issue its effect).")
  (input
    (do
      (effect Add (op sum (-> (Tuple Int64 Int64) Int64)))
      (def (main) (handle Add 0 ((sum (p) s (resume (+ (. p 0) (. p 1)) s))) (Add.sum #tuple(3 4))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a runtime LIST argument to an effect op is WALKED by a recursive fold inside the arm"
  (doc
    "The walked-collection upgrade of the compound-parameter pins (the tuple/record args above are
           const scalar-leaf projections): `(Sink.tally (list a 2 30))` carries a runtime-element list
           into the arm, whose body runs a full RECURSIVE fold over the bound parameter before resuming —
           10+2+30 = 42. The RRB handle must arrive intact and support the head-tail destructure loop
           from inside the handler context (an arm is not a plain function body — the fold runs under the
           handler's dispatch machinery).")
  (input
    (do
      (effect Sink (op tally (-> (List Int64) Int64)))
      (def
        (sum-l (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def
        (main (: a Int64))
        (handle Sink 0 ((tally (xs) s (resume (sum-l xs 0) s))) (Sink.tally #list(a 2 30))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 42 Int64))
  (live-objects 0))

(case
  "a MAP argument to an effect op is looked up inside the arm at the handler's own state"
  (doc
    "The CHAMP-descent-in-arm face: the perform carries a 2-entry map, and the arm looks it up at
           the handler's STATE value (`s`, seeded from the boundary parameter) — composing the op
           argument, the state slot, and the CHAMP descent in one arm expression. k=2 hits (20), k=9
           misses (-1). A lowering that rebound the arm's parameter or state wrong feeds the lookup the
           wrong key or trie.")
  (input
    (do
      (effect Sink (op pick (-> (Map Int64 Int64) Int64)))
      (def
        (main (: k Int64))
        (handle
          Sink
          k
          ((pick (m) s (resume (match (Map.lookup m s) ((Some v) v) ((None u) -1)) s)))
          (Sink.pick (Map.insert (Map.insert Map.empty 1 10) 2 20))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 20 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64)))

(case
  "a handler ARM enumerates a 60-key trie op argument and resumes its fold"
  (doc
    "The DEEP-trie upgrade of the map-argument arm case above (whose map has 2 entries): the
           perform carries a 60-key MULTI-LEVEL trie, and the arm runs a full `Map.to-list` enumeration
           plus a pair-fold over it before resuming — Σ i for i = 1..60 = 1830. The multi-level
           enumeration walk (node descent, cross-node merge order) runs INSIDE the handler's dispatch
           machinery; an arm context that corrupted a frame slot mid-walk would poison the sum. The
           arm-side companion of the deep-trie enumeration pins.")
  (input
    (do
      (effect Sink (op tally (-> (Map Int64 Int64) Int64)))
      (def
        (fill (: i Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
      (def
        (sum-pairs (: ps (List (Tuple Int64 Int64))) (: acc Int64))
        (match ps (#list() acc) (#list(h (.. t)) (match h (#tuple(_k v) (sum-pairs t (+ acc v)))))))
      (def
        (main (: n Int64))
        (handle
          Sink
          0
          ((tally (m) s (resume (sum-pairs (Map.to-list m) 0) s)))
          (Sink.tally (fill n Map.empty))))
      (export main)))
  (call main (: 60 Int64))
  (output (: 1830 Int64))
  (live-objects 0))

(case
  "a handler's heap STATE grows to a 40-key trie across resumes and enumerates at the end"
  (doc
    "The state-side companion: the handler's STATE is a map that GROWS by one insert per resume
           across 40 separate `put` discharges (values i·10), then a second op enumerates the
           accumulated trie — Σ i·10 for i = 1..40 = 8200. Composes state threading (each resume hands
           the next state forward), trie growth past the single-node capacity, and the enumeration walk
           over the final accumulated structure. The keyed-store idiom's growth face at scale (the
           Map-state pins put/get single entries).")
  (input
    (do
      (effect Acc (op put (-> Int64 Int64)) (op total (-> Unit Int64)))
      (def
        (sum-pairs (: ps (List (Tuple Int64 Int64))) (: acc Int64))
        (match ps (#list() acc) (#list(h (.. t)) (match h (#tuple(_k v) (sum-pairs t (+ acc v)))))))
      (def (feed (: i Int64) (: n Int64)) (if (= i n) 0 (+ (Acc.put i) (feed (+ i 1) n))))
      (def
        (main (: n Int64))
        (handle
          Acc
          Map.empty
          ((put (v) s (resume 0 (Map.insert s v (* v 10))))
            (total (u) s (resume (sum-pairs (Map.to-list s) 0) s)))
          (do (feed 1 (+ n 1)) (Acc.total))))
      (export main)))
  (call main (: 40 Int64))
  (output (: 8200 Int64))
  (live-objects known-leak))

(case
  "a handler SEEDED with a 40-key trie reads it across resumes"
  (doc
    "The deep-trie SEED face (the state-growth case above starts EMPTY and grows; here the heap
           state arrives fully-built at the handle boundary): a 40-key trie built before the handle
           seeds it, and the arm reads `Map.len` across two resumes (80). The seed materializes once
           and threads intact — a seed path that re-evaluated the fill per resume, or that handed the
           arm a stale snapshot, would double-build or misread.")
  (input
    (do
      (effect Rd (op keys (-> Unit Int64)))
      (def
        (fill (: i Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
      (def
        (main (: n Int64))
        (handle Rd (fill n Map.empty) ((keys (u) s (resume (Map.len s) s))) (+ (Rd.keys) (Rd.keys))))
      (export main)))
  (call main (: 40 Int64))
  (output (: 80 Int64)))

(case
  "an arm REPLACES the trie state wholesale and the next op reads the replacement"
  (doc
    "The state-slot ownership face at scale: the arm's resume hands back a COMPLETELY NEW trie
           (a 60-key rebuild with a different key prefix) in place of the 30-key seed — drop-old /
           adopt-new across one resume. The swap op reports the OLD len as its value (30) while
           installing the replacement; the next op reads the NEW len (60) → 30·1000 + 60 = 30060.
           A state thread that leaked the old trie, or aliased old and new, would corrupt one of the
           two reads. (The wholesale-replacement companion of the per-op insert growth above.)")
  (input
    (do
      (effect Sw (op swap (-> Unit Int64)) (op len (-> Unit Int64)))
      (def
        (fill (: i Int64) (: k Int64) (: m (Map Int64 Int64)))
        (if (= i 0) m (fill (- i 1) k (Map.insert m (+ (* k 1000) i) i))))
      (def
        (main (: n Int64))
        (handle
          Sw
          (fill n 1 Map.empty)
          ((swap (u) s (resume (Map.len s) (fill (* n 2) 2 Map.empty)))
            (len (u) s (resume (Map.len s) s)))
          (+ (* 1000 (Sw.swap)) (Sw.len))))
      (export main)))
  (call main (: 30 Int64))
  (output (: 30060 Int64)))

(case
  "a TUPLE op argument with a HEAP leaf destructures inside the arm"
  (doc
    "The mixed-representation companion of the scalar tuple-parameter case: `(tuple a \"abc\")`
           carries an i64 AND a rope handle through the perform; the arm destructures both and measures
           the string leaf (39 + 3 = 42). The op-argument boxing must carry the heap handle beside the
           scalar without confusing slots (the effects twin of the mixed-representation generic pins).")
  (input
    (do
      (effect Sink (op unpack (-> (Tuple Int64 String) Int64)))
      (def
        (main (: a Int64))
        (handle
          Sink
          0
          ((unpack (p) s (match p (#tuple(n str) (resume (+ n (String.byte-len str)) s)))))
          (Sink.unpack #tuple(a "abc"))))
      (export main)))
  (call main (: 39 Int64))
  (output (: 42 Int64)))

(case
  "an operation with a RECORD parameter binds the compound and the arm reads its fields"
  (doc
    "The record companion of the tuple-parameter case: an operation whose declared PARAMETER is a
           `(Record (: a Int64) (: b Int64))` binds the whole record to the arm's parameter, whose fields the arm
           reads by member access. `Add.sum : (-> (Record (: a Int64) (: b Int64)) Int64)`; performed as
           `(Add.sum (record (a 3) (b 4)))`, the arm binds `p` and resumes with `(+ (. p a) (. p b))` = 7.
           The arm references `p` TWICE (once per field), but the argument is a PURE record — it reaches no
           perform — so substituting it into both uses duplicates no effect and the fold serves it (the
           effect-duplication guard only declines a param whose argument REACHES A PERFORM, not a pure
           compound; the precise perform-detector does not misread a record's field pairs as a call). Pins
           that a record OP parameter threads and is field-readable, matching the tuple parameter.")
  (input
    (do
      (effect Add (op sum (-> (Record (: a Int64) (: b Int64)) Int64)))
      (def
        (main)
        (handle Add 0 ((sum (p) s (resume (+ p.a p.b) s))) (Add.sum #record((= a 3) (= b 4)))))
      (export main)))
  (output (: 7 Int64)))

(case
  "a NON-tail-resumptive arm projects a tuple parameter twice in its pure one-hole context"
  (doc
    "The compound-parameter case through the NON-tail-resumptive (pure one-hole) fold path rather than
           the tail-resumptive threading path. The arm `(+ 1 (resume (+ (* (. p 0) 100) (. p 1)) s))` resumes
           NON-tail (the resume is inside `+ 1`), so its delimited continuation is folded as a pure one-hole
           context: the perform IS the whole body (`C = []`), so `(resume v s)` yields `v` and the arm value
           is `(+ 1 v)`. The resume value projects the bound tuple parameter `p` TWICE — `(* (. p 0) 100)`
           and `(. p 1)` — over the PURE argument `(tuple 3 4)`, so `v = 304` and the handle yields
           `(+ 1 304)` = 305. Pins that the pure-one-hole substitution binds a compound op parameter and
           tolerates projecting it multiple times when the argument is pure (a pure argument copied per
           projection duplicates no effect — the same soundness the tail-path duplication guard enforces,
           here satisfied because the argument reaches no perform).")
  (input
    (do
      (effect Add (op sum (-> (Tuple Int64 Int64) Int64)))
      (def
        (main)
        (handle
          Add
          0
          ((sum (p) s (+ 1 (resume (+ (* (. p 0) 100) (. p 1)) s))))
          (Add.sum #tuple(3 4))))
      (export main)))
  (output (: 305 Int64)))

; The SAME spec sentence has a second half: performing an operation must "YIELD the operation's declared
; RESULT type" (capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row).
; A handler arm resumes the continuation with the value the operation yields — `(resume <value> <state>)`
; "returns <value> to the point that performed the operation" (this file's header) — so the resume VALUE
; is what the operation yields, and it MUST have the operation's declared result type. For `E.op` declared
; `(-> Int64 Int64)`, `(resume true s)` resumes with a Bool where the declared result is Int64 — a type
; mismatch the compiler MUST reject (CDZ0201), exactly as feeding a Bool argument to the perform is (the
; case above) and exactly as an ordinary function whose body returns the wrong type is. A compiler that
; checks a perform's ARGUMENTS but not the resume value against the result type feeds the Bool back through
; the op's Int64-typed result slot and yields the wrong value — `(E.op 1)` returns `true` (and `(resume 99
; s)` for a Bool-result op returns the integer `99`) rather than rejecting. This is the result-type half of
; the perform-argument case above: the two halves of one spec sentence must both hold. A generation that
; does not yet check the resume value against the declared result type declines rather than yielding it.
(case
  "resuming with a value of the wrong type for the operation's result is a type error"
  (doc
    "`E.op` is declared `(-> Int64 Int64)`, so its result type is Int64 and the value a handler
           resumes with — `(resume <value> <state>)`, the value returned to the perform site — MUST be an
           Int64. `(resume true s)` resumes with a Bool, a mismatch against the declared result type the
           compiler MUST reject (CDZ0201, capabilities-and-effects.md #Performing An Operation Is Typed And
           Contributes To The Row: a perform must 'yield the operation's declared result type', so an
           effect operation is typed exactly as an ordinary function application — whose body returning the
           wrong type is rejected the same way). This is the result-type companion of the argument-type
           case above (`(E.op true)`): the same spec sentence checks arguments against parameter types AND
           yields the declared result type. A compiler that checks the arguments but not the resume value
           feeds the Bool through the op's Int64 result slot and yields `true` from `(E.op 1)` rather than
           rejecting. A generation that does not yet check the resume value against the result type
           declines rather than yielding it.")
  (input
    (do
      (effect E (op op (-> Int64 Int64)))
      (def (main) (handle E unit ((op (n) s (resume true s))) (E.op 1)))
      (export main)))
  (error CDZ0201))

; The resume-value result-type check must hold when the declared result type is a COMPOUND, not only a
; scalar. `E.get` declared `(-> (List Int64))` has result type `List Int64`, so a handler resuming with an
; Int64 — `(resume 42 s)` — or a Bool, or a tuple, is the same result-type mismatch the scalar case above
; is, and MUST be rejected (CDZ0201). A compiler that checks the resume value against a SCALAR result type
; but not a compound one yields the mistyped value: `(E.get)` returns `42` for `(resume 42 s)`, and — worse
; — `(resume (tuple 7 8) s)` yields `(list)`, a TUPLE reinterpreted through the op's List result slot and
; rendered as an (empty) list, a type-confusion wrong value. This is the compound-result sibling of the
; scalar-result case above: the "yield the declared result type" check must be uniform across result types,
; not gated by whether the declared result is scalar. A generation that does not yet check a compound
; result type declines rather than yielding the mistyped value.
(case
  "resuming with a wrong-type value for a compound result type is a type error"
  (doc
    "`E.get` is declared `(-> (List Int64))`, so its result type is the compound `List Int64` and the
           value a handler resumes with MUST be a `List Int64`. `(resume 42 s)` resumes with an Int64 — a
           mismatch against the declared compound result type the compiler MUST reject (CDZ0201,
           capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To The Row: a
           perform must 'yield the operation's declared result type'). This is the compound-result-type
           companion of the scalar-result case above (`(resume true s)` for an Int64 result): the check
           must be uniform across result types. A compiler that checks a scalar result type but not a
           compound one yields the mistyped value — `(E.get)` returns `42`, and resuming with a tuple where
           a list is declared renders `(list)`, a type-confusion wrong value. A generation that does not yet
           check a compound result type declines rather than yielding the mistyped value.")
  (input
    (do
      (effect E (op get (-> (List Int64))))
      (def (main) (handle E unit ((get () s (resume 42 s))) (E.get)))
      (export main)))
  (error CDZ0201))

; A resume carries two values — `(resume <value> <state>)` — and BOTH are ordinary expressions subject to
; #Binding Is Lexical (core-semantics.md, unconditional): a reference to an unbound name in either is a
; compile-time error (CDZ0101). The resume VALUE position already rejects an unbound name (`(resume
; undefined-xyz s)` is caught), but the STATE position does not: `(resume unit undefined-xyz)` runs to the
; handler's result instead of rejecting the unbound `undefined-xyz`. A compiler that scope-checks only the
; resume value and not the resume state lets an unbound reference in the state slip through — the same
; unbound-name gap the unselected-conditional-branch and short-circuited-connective-operand cases closed,
; here in a resume's second argument. A generation that does not yet scope-check the resume state declines.
(case
  "an unbound name in a resume's state position is rejected"
  (doc
    "`(resume unit undefined-xyz)` references the unbound name `undefined-xyz` in the resume's STATE
           position (its second argument); a resume's state is an ordinary expression, so an unbound name
           in it is a compile-time error (CDZ0101, core-semantics.md #Binding Is Lexical — unconditional),
           exactly as an unbound name in the resume VALUE position (`(resume undefined-xyz s)`) already is.
           Pins that scope resolution reaches the resume STATE, not only the resume value. A compiler that
           scope-checks the value but not the state runs to the handler's result instead of rejecting. A
           generation that does not yet scope-check the resume state declines rather than emitting a
           component.")
  (input
    (do
      (effect E (op put (-> Int64 Unit)))
      (def (main) (handle E unit ((put (p) s (resume unit undefined-xyz))) (E.put 1)))
      (export main)))
  (error CDZ0101))

(case
  "a handler arm for an operation the effect does not declare is rejected"
  (doc
    "`Choose` declares only `pick`; a handler arm naming `Choose.guess` names an operation the
           effect does not declare, rejected at compile time (CDZ0403) because the declaration is the
           closed set of an effect's operations (capabilities-and-effects.md #A Handler Arm Names An
           Operation Its Effect Declares). A generation that does not yet check arm membership declines
           rather than running the program (reject-don't-miscompile).")
  (input
    (do
      (effect Choose (op pick (-> Unit Int64)))
      (def (main) (handle Choose unit ((guess () s (resume 5 s))) (Choose.pick)))
      (export main)))
  (error
    CDZ0403
    (message "closest matches: `pick`")
    (not "did you mean")
    (fix (kind delete) (unverified))))

; The TIER-1 (close-typo) companion of the far-miss above: a handler-arm op ONE edit from a declared op
; (`picks` for `pick`) gets a CONFIDENT "did you mean `pick`?" + a REPLACE fix on the mistyped op key, rather
; than the far-miss's delete-the-arm. (Migrated from rcdzc a_handler_arm_for_an_undeclared_operation_is_cdz0403
; — the tier-1 close-typo path.)
(case
  "a close-typo handler arm op names the declared op with a replace fix"
  (input
    (do
      (effect Choose (op pick (-> Unit Int64)))
      (def (main) (handle Choose unit ((picks () s (resume 5 s))) (Choose.pick)))
      (export main)))
  (error
    CDZ0403
    (message "did you mean `pick`?")
    (fix (kind replace) (replacement "pick") (unverified))))

; A handler arm binding the WRONG NUMBER of parameter binders is CDZ0201 — the arm analogue of a function at
; the wrong arity, naming the operation and the expected/actual counts. Was either silently accepted (too few)
; or surfaced only the leaky "not reducible by the tail-resumptive fold" decline (too many); the coded reject
; now names the count and the consequent fold-decline is dropped (ONE primary). The ELIDED-UNIT convention
; holds: a `(-> Unit R)` op accepts BOTH a 0-binder and a 1-binder arm, so only a count outside `{0,1}` for a
; unit op is a mismatch; a genuine N-param op requires exactly N. (Migrated from rcdzc
; a_handler_arm_with_the_wrong_parameter_count_is_cdz0201.)
(case
  "a handler arm binding too many parameters for a unit op is rejected"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u v) s (resume 5 s))) (+ (E.get) 1)))
      (export main)))
  (error
    CDZ0201
    (message "handler arm for operation `get` binds 2 parameters")
    (message "declares 0 or 1")
    (not "not reducible by the tail-resumptive fold")))

(case
  "a handler arm binding too few parameters for a genuine one-param op is rejected"
  (input
    (do
      (effect E (op set (-> Int64 Unit)))
      (def (main) (handle E 0 ((set () s (resume unit s))) (E.set 3)))
      (export main)))
  (error
    CDZ0201
    (message "handler arm for operation `set` binds 0 parameters")
    (message "declares 1")))

(case
  "the elided-unit zero-binder handler arm for a unit op compiles and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5 s))) (+ (E.get) 1)))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "the elided-unit one-binder handler arm for a unit op compiles and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u) s (resume 5 s))) (+ (E.get) 1)))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "a handler mixing arms of two different effects is rejected"
  (doc
    "A handler discharges EXACTLY ONE effect — every arm names an operation of the handle head's
           declaring effect (capabilities-and-effects.md #A Handler Discharges Exactly One Effect).
           `(handle A … ((a …) (b …)) …)` mixes an arm for `A.a` with an arm for `b`, an operation of a
           DIFFERENT effect `B`; since `b` is not one of `A`'s declared operations, the arm is rejected
           CDZ0403 (the same closed-set check that rejects an undeclared operation name). Discharging two
           effects over one sub-computation is expressed by NESTING a handler per effect, not by enumerating
           two effects' operations in one handler's arms.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (def (main) (handle A 0 ((a (u) s (resume 1 s)) (b (u) s (resume 2 s))) (A.a)))
      (export main)))
  (error CDZ0403))

(case
  "a handler that does not discharge every operation of its effect is rejected"
  (doc
    "`Diag` declares two operations, `emit` and `collect`; a `handle Diag` binding only `emit`
           leaves `collect` undischarged. A handle names ONE effect and its arms ARE that effect's
           operations, and an effect's operations are a CLOSED, statically-known SET (capabilities-and-
           effects.md #An Effect Declaration Names The Effect And Types Its Operations), so a handler must
           discharge the WHOLE set — the effect analogue of match exhaustiveness over a sum's variants. A
           handler missing an operation is rejected at compile time (CDZ0405): it would claim to discharge
           `Diag` while leaving `Diag.collect` without a home. Discharging a subset across LAYERS is nested
           handles, each exhaustive for its own effect (see the collision-free cross-effect case, which is
           two nested single-operation handlers). A generation that does not yet check handler
           exhaustiveness declines rather than running the partial handler (reject-don't-miscompile).")
  (input
    (do
      (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64))))
      (def
        (main)
        (handle
          Diag
          #list()
          ((emit (code) s (resume unit (List.push s code))))
          (do (Diag.emit 1) 0)))
      (export main)))
  (error
    CDZ0405
    (message "`collect`")
    (message "add (collect () s (resume")
    (fix (replacement-contains "(collect ()") (unverified))))

(case
  "a resume outside any handler arm is rejected"
  (doc
    "A `resume` hands a value back to the point that performed a handler arm's operation, so it is
           meaningful ONLY inside a handler arm's body (capabilities-and-effects.md #A Handler Arm May
           Resume). A `resume` in a plain definition body — with no enclosing handler arm to return into —
           is a malformed use of the control form, rejected at compile time (CDZ0201) rather than silently
           accepted and declined only at lowering. Pins that `cdz check` surfaces a stray resume (a
           well-formedness fault visible without emitting), not just the backend.")
  (input (do (effect Amb (op flip (-> Unit Int64))) (def (main) (resume 1 0)) (export main)))
  (error CDZ0201))

(case
  "a host delegating a value definition rather than an effect is rejected"
  (doc
    "A `host` delegates EFFECTS to the boundary — it grants exactly the effects its body reaches
           (capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative). `(host (foo) …)`
           where `foo` is a value definition names a VALUE, not an effect: there is nothing to delegate, so
           it is a malformed grant, rejected at compile time (CDZ0201) rather than silently accepted as a
           no-op that computes an empty manifest. Pins that a delegation names a declared effect.")
  (input (do (def foo 5) (def (main) (host (foo) 5)) (export main)))
  (error CDZ0201 (message "foo") (message "effect")))

(case
  "a bind directive naming a value definition rather than an effect is rejected"
  (doc
    "The `(bind …)` peer-binding analogue of the host-delegates-a-value reject above (the U-pivot
           unifies a peer dependency with an effect, so binding a peer names a declared EFFECT). `(bind foo
           \"cadenza:x/y\")` where `foo` is a value definition names a VALUE, not an effect — there is
           nothing to route to a peer, so it is a malformed binding rejected at compile time (CDZ0201)
           rather than SILENTLY DROPPED (the `bind` scan used to ignore a non-effect/malformed directive, so
           a typo'd binding quietly did nothing). Pins that a peer binding names a declared effect, the same
           bar the host delegation and the `(extern …)` interface hold.")
  (input (do (def (foo) 5) (bind foo "cadenza:x/y") (def (main) 0) (export main)))
  (error CDZ0201))

(case
  "a host delegating the same effect twice is rejected"
  (doc
    "A `host`'s effect list is a SET — the manifest is the union of the effects that escape to the
           boundary (capabilities-and-effects.md #Host Delegation Is An Entrypoint's Prerogative). `(host (A
           A) …)` names the effect `A` twice: the same fixed-set-no-duplicates ill-formedness a duplicate
           operation in an effect declaration and a duplicate arm in a handler are rejected for (CDZ0201) —
           a closed set cannot name the same member twice. Rejected at compile time rather than
           double-imported at the boundary (which traps at run time).")
  (input (do (effect A (op a (-> Unit Int64))) (def (main) (host (A A) (A.a))) (export main)))
  (error CDZ0201 (message "more than once")))

(case
  "binding the same effect to a peer twice is rejected"
  (doc
    "The `(bind …)` peer-routing analogue of the duplicate-host-delegation reject above (the U-pivot
           unifies a peer dependency with an effect). A `(bind E \"iface\")` route is a SET — one peer per
           effect — so `(bind E \"cadenza:a/x\") (bind E \"cadenza:b/y\")` binds `E` twice: the same
           fixed-set-no-duplicates ill-formedness (`scan_effect_bindings` silently keeps only the FIRST, so
           the second is a dead, ambiguous line — the author wrote two routes and only one takes). Rejected
           at compile time (CDZ0201) rather than silently dropped. (A compile-request `--bind` REBIND is a
           separate layer — merged after load — and is unaffected; this flags two SOURCE `(bind …)` for one
           effect.)")
  (input
    (do
      (effect E (op e (-> Int64 Int64)))
      (bind E "cadenza:a/x")
      (bind E "cadenza:b/y")
      (def (main) (handle E 0 ((e (n) s (resume n s))) (E.e 1)))
      (export main)))
  (error CDZ0201))

(case
  "a bind directive with a malformed peer interface name is rejected"
  (doc
    "A `(bind Effect \"iface\")` INTERFACE STRING is a component-boundary name — it is emitted
           VERBATIM as the extern name the peer-instance import binds under, so it must be a valid
           component-model interface name `namespace:package/interface` in kebab-case (lowercase package),
           the same shape the runtime heap import and every provider export use. `\"Math/API\"` is not: an
           uppercase package segment. Without a compile-time check the string would `kebab_extern_name`-
           mangle to the INVALID extern name `math/-a-p-i` and produce a component wasmtime rejects at LOAD
           with NO diagnostic — a silent invalid-component miscompile. Rejected at compile time (CDZ0201)
           naming the offending string, the peer-binding analogue of the other bind rejects. A bare package
           name with no `/interface` projection (`cadenza:math`) is malformed the same way.")
  (input
    (do
      (effect Math (op add (-> Int64 Int64 Int64)))
      (bind Math "Math/API")
      (def (main (: x Int64)) (host (Math) (Math.add x x)))
      (export main)))
  (error CDZ0201))

(case
  "a bind directive to a package name with no interface projection is rejected"
  (doc
    "The other common face of a malformed peer interface name (the sibling of the uppercase-package
           case above): a bare PACKAGE name with NO `/interface` projection. A peer binding imports an
           interface INSTANCE, whose component extern name is `namespace:package/interface` — the `/iface`
           projection is REQUIRED (component-abi.md; the same shape the runtime heap import
           `cadenza:runtime/heap` uses). `\"cadenza:math\"` names only the package, so it is not a valid
           interface name and the emitted component's import would fail to load. Rejected at compile time
           (CDZ0201) rather than a silent invalid-component miscompile — exercising the projection-required
           branch of the interface-name check, distinct from the kebab/lowercase branch the `Math/API` case
           covers. The likeliest author typo (forgetting the `/api`), so worth its own witness.")
  (input
    (do
      (effect Math (op add (-> Int64 Int64)))
      (bind Math "cadenza:math")
      (def (main (: x Int64)) (host (Math) (Math.add x)))
      (export main)))
  (error CDZ0201))

(case
  "a peer-bound operation cannot take or return a closure"
  (doc
    "Peers exchange VALUE-HEAP HANDLES (a tuple/record/sum/list/map/string/…); a closure is not a
           value-heap value, so it has no peer-boundary form (a closure crosses the HOST boundary as a
           component-model resource, per closures-across-host, NOT a peer). Without a compile-time check a
           peer-bound op whose signature involves a function type — `(op mk (-> Int64 (-> Int64 Int64)))`
           bound to a peer — type-checks, then APPLYING the peer-returned closure declines deep in lowering
           with an opaque `value is not applyable`. Reject it at the binding (CDZ0201) with the real reason
           — the `(-> …)` in the operation's signature is the tell. Detected SYNTACTICALLY: a boundary
           position of the op's `(-> …)` arrow that is ITSELF a `(-> …)` list. Fires only for a peer-BOUND
           effect (a closure crossing the HOST boundary via `(host …)` is unaffected).")
  (input
    (do
      (effect F (op mk (-> Int64 (-> Int64 Int64))))
      (bind F "cadenza:f/api")
      (def (main) 0)
      (export main)))
  (error CDZ0201))

(case
  "a peer-bound operation takes a String argument (it crosses as a runtime handle)"
  (doc
    "A String/Bytes ARGUMENT to a peer-bound op crosses the boundary as a runtime rope HANDLE, just
           like a compound (tuple/record) argument — both peers share one value-heap runtime, so the arg is
           an opaque u32 handle into it, never a marshaled component `string`. (This once DECLINED CDZ0201:
           the arg lowered as a component `string` needing a `mem` canonical option the runtime-only peer
           envelope never supplied, producing an invalid consumer component; the inbound-rope-handle emit is
           now wired — `collect_used_ops`/`collect_host_arg_strings` are peer-aware, so a peer String arg
           builds a rope while a HOST String arg still marshals as `(ptr,len)`.) This case pins that
           DECLARING and PERFORMING such an op now COMPILES + runs: an in-program handler overrides the peer
           binding (the free test-mock) and answers `blen(s) = 100` regardless of `s`, so `(S.blen \"hi\")`
           = 100 — proving the String-arg op type-checks and its argument flows without a live peer. The e2e
           crossing to a real peer (byte-len read there) is pinned by the `a_string_argument_crosses_to_a_
           peer_*` backend tests. Only the ARGUMENT direction changed; a String/Bytes RESULT already worked.")
  (input
    (do
      (effect S (op blen (-> String Int64)))
      (bind S "cadenza:str/api")
      (def (main) (handle S 0 ((blen (s) k (resume 100 k))) (S.blen "hi")))
      (export main)))
  (output (: 100 Int64))
  (host-calls))

(case
  "a String ENTRY arg rides a rope into an effect-op argument and the arm reads its bytes"
  (doc
    "The String-entry-arg family (13-strings — wasm declines the entry marshal, a sound todo; rust
           computes) composed with EFFECTS: the boundary `s` is concatenated into a runtime rope, performed
           as the String ARGUMENT of `Log.emit`, and the handler arm reads the arg's byte length —
           byte-len(\"xy\"+\"abc\") = 5. Pins the full entry→rope→op-arg→arm chain on the targets that
           marshal the entry arg; the op-argument String path itself is already pinned const (the blen
           case above) — this witnesses a RUNTIME-valued op argument flowing from the component boundary.")
  (input
    (do
      (effect Log (op emit (-> String Int64)))
      (def
        (main (: s String))
        (handle
          Log
          0
          ((emit (m) st (resume (String.byte-len m) st)))
          (Log.emit (String.concat s "abc"))))
      (export main)))
  (call main (: "xy" String))
  (output (: 5 Int64)))

; (The two "peer op whose compound/SUM RESULT escapes the entrypoint declines" corpus cases were REMOVED
;  once the resource-escape × peer-extern envelope FUSION landed — the shapes they witnessed as declines
;  now EMIT + run. The corpus gate cannot compose a live peer, so a peer-crossing RUN can't be a graded
;  case here; the crossings are pinned e2e by the backend `run_with_peers` tests
;  a_peer_{compound,option,list}_result_escapes_the_entrypoint_via_the_fused_envelope.)
(case
  "a handle whose head names a value rather than an effect is rejected"
  (doc
    "A `handle`'s HEAD names the effect the handler discharges, and its arms ARE that effect's
           operations (capabilities-and-effects.md #A Handler Arm Names An Operation Its Effect Declares).
           `(handle foo 0 …)` where `foo` is a value definition names a VALUE, not an effect — a malformed
           handle. Rejected at compile time (CDZ0201) with a message naming the head, rather than surfacing
           as a leaky desugar artifact (the head folds into each arm's member-access projection). Pins that
           a handle head names a declared effect.")
  (input (do (def foo 5) (def (main) (handle foo 0 ((x (u) s (resume 1 s))) 5)) (export main)))
  (error CDZ0201))

(case
  "an effect operation declared with no name is rejected"
  (doc
    "An operation clause is `(op <name> <type>)` — the name is a bare identifier, the type its arrow.
           `(op (-> Unit Int64))` puts the TYPE where the name belongs, declaring a NAMELESS operation:
           there is no `E.op` to project, so the operation is unreachable. An operation must be named, like a
           definition or a sum variant (an effect's operations are a closed, named set,
           capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its Operations), so
           this is rejected at compile time (CDZ0201) rather than silently registered with an empty name.")
  (input (do (effect E (op (-> Unit Int64))) (def (main) 5) (export main)))
  (error CDZ0201))

(case
  "an effect operation reached with neither a handler nor a delegation is rejected"
  (doc
    "`Ask` is a routing-agnostic effect; `main` performs `(Ask.ask)` with no enclosing handler and
           no enclosing entrypoint `host` delegation, so the effect would escape ungranted — rejected at
           compile time (CDZ0401, capabilities-and-effects.md #An Ungranted Effect Is A Compile-Time
           Error). This is the single 'no home for a reached effect' check: since host-binding is now an
           entrypoint routing decision rather than a declaration-time marker, the former CDZ0402
           (undischarged intra-program effect) and the former undeclared-host CDZ0401 are one condition.
           Contrast the interpose case above, where an enclosing `host (ask)` delegation gives the effect
           a home.")
  (input (do (effect Ask (op ask (-> Unit Int64))) (def (main) (+ (Ask.ask) 1)) (export main)))
  (error CDZ0401 (fix (kind wrap) (replacement-contains "(host (Ask) ") (unverified))))

; A `host` is exactly `(host (<effect>…) <body>)`. A host with TOO MANY operands — `(host (E) <body> extra)` —
; was silently accepted (the surplus dropped, a silent miscompile). It is now CDZ0201 "too many operands"
; with a delete-the-surplus fix, and the consequent CDZ0401 (the malformed host's body-perform looks
; un-delegated to the no-home walk) is deduped so it is the SOLE error. (Migrated from rcdzc
; a_host_with_too_many_operands_is_cdz0201; the no-home CDZ0401 control is the case above, a valid host is
; exercised throughout this chapter.)
(case
  "a host with too many operands is rejected with a delete-the-surplus fix, no consequent no-home error"
  (input (do (effect E (op get (-> Unit Int64))) (def (main) (host (E) (E.get) 99)) (export main)))
  (error CDZ0201 (message "too many operands") (fix (kind delete)) (no-other-errors)))

; A `handle`'s HEAD must name an EFFECT (the arms ARE that effect's operations). A head that is a VALUE def
; (`(handle foo …)`, foo a value) or a TYPE (`(handle C …)`, C a sum) is CDZ0201 naming the head as a value /
; a type ("head must name an EFFECT"), not the leaky member-access / fold-decline cascade the desugared
; `(. head op)` once produced. An UNBOUND head keeps its own CDZ0101 (the resolver's primary). (Migrated from
; rcdzc a_handle_head_naming_a_value_reports_one_clear_diagnostic — the head-naming facets; the "exactly ONE
; diagnostic" dedup of the UNCODED member-access/fold cascades stays a rust residual.)
(case
  "a handle head that is a value def names the head must be an effect"
  (input (do (def foo 5) (def (main) (handle foo 0 ((x (u) s (resume 1 s))) 5)) (export main)))
  (error CDZ0201 (message "head must name an EFFECT") (message "foo")))

(case
  "a handle head that is a type names the head must be an effect"
  (input (do (type C (Red)) (def (main) (handle C 0 ((a (u) s (resume 1 s))) 5)) (export main)))
  (error CDZ0201 (message "head must name an EFFECT") (message "a type")))

(case
  "an unbound handle head keeps its own unbound-name error"
  (input (do (def (main) (handle Nonesuch 0 ((x (u) s (resume 1 s))) 5)) (export main)))
  (error CDZ0101))

(case
  "a handle head that is a prelude scalar type names the head must be an effect"
  (input (do (def (main) (handle Int64 0 ((x (u) s (resume 1 s))) 5)) (export main)))
  (error CDZ0201 (message "head must name an EFFECT") (message "is a type")))

(case
  "a handle head that is a prelude sum type names the head must be an effect"
  (input (do (def (main) (handle Option 0 ((x (u) s (resume 1 s))) 5)) (export main)))
  (error CDZ0201 (message "head must name an EFFECT") (message "is a type")))

; A `resume` inside a handler arm is exactly `(resume <value> <next-state>)`. TOO MANY operands
; (`(resume 5 s 9)`) was silently accepted — now CDZ0201 "too many operands" with a delete-the-surplus fix;
; TOO FEW (`(resume 5)`, no next-state) is CDZ0201 "no next-state". Neither also reports the spurious
; stray-resume "no enclosing handler arm" secondary (the resume IS in an arm, just malformed). A GENUINE
; top-level `resume` outside any handler arm is the placement error CDZ0201 "no enclosing handler arm".
; (Migrated from rcdzc a_resume_with_the_wrong_number_of_operands_is_cdz0201 — the arity + placement rejects;
; the stray+malformed cross-diagnostic suppression stays a rust residual.)
(case
  "a resume with too many operands is rejected with a delete-the-surplus fix"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5 s 9))) (+ (E.get) 1)))
      (export main)))
  (error CDZ0201 (message "too many operands") (fix (kind delete)) (not "no enclosing handler arm")))

(case
  "a resume missing its next-state is rejected"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5))) (+ (E.get) 1)))
      (export main)))
  (error CDZ0201 (message "no next-state") (not "no enclosing handler arm")))

(case
  "a genuine top-level stray resume with no enclosing handler arm is rejected"
  (input (do (def (main) (resume 5 6)) (export main)))
  (error CDZ0201 (message "no enclosing handler arm")))

; The third effect-boundary type check (beside the resume ANSWER and NEXT-STATE checks): an op's ARGUMENT
; must match the operation's PARAMETER type. `(op put (-> Int64 Unit))` takes Int64, so performing `(E.put
; "x")` / `(E.put true)` is CDZ0203 — "operation `put` expects an argument of type Int64, but a value of type
; <T> was performed" (the argument-position code, vs the resume-side CDZ0201; the same result-vs-arg split as
; ordinary typing). Base-type twin of the units-mismatch op-arg case in chapter 14. (Edge-probed by
; v-wasmtime-migration.)
(case
  "performing an op with a String argument under an Int64-parameter operation is a type error"
  (input
    (do
      (effect E (op put (-> Int64 Unit)) (op tot (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          0
          ((put (v) s (resume unit (+ s v))) (tot (u) s (resume s s)))
          (do (E.put "x") (E.tot))))
      (export main)))
  (error
    CDZ0203
    (message "operation `put` expects an argument of type Int64")
    (message "String")
    (message "was performed")))

(case
  "performing an op with a Bool argument under an Int64-parameter operation is a type error"
  (input
    (do
      (effect E (op put (-> Int64 Unit)) (op tot (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          0
          ((put (v) s (resume unit (+ s v))) (tot (u) s (resume s s)))
          (do (E.put true) (E.tot))))
      (export main)))
  (error
    CDZ0203
    (message "operation `put` expects an argument of type Int64")
    (message "Bool")
    (message "was performed")))

; A `resume`'s ANSWER (first operand) must match the OPERATION'S RESULT type: `(op get (-> Unit Int64))`
; yields Int64, so resuming with a String or Bool answer is CDZ0201 naming the answer type + "the operation's
; result type is Int64". This is the answer-slot twin of the next-state-slot type check below. (Edge-probed by
; v-wasmtime-migration.)
(case
  "resuming with a String answer under an Int64-result operation is a type error"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u) s (resume "x" s))) (E.get)))
      (export main)))
  (error
    CDZ0201
    (message "resumes with a value of type")
    (message "String")
    (message "result type is Int64")))

(case
  "resuming with a Bool answer under an Int64-result operation is a type error"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u) s (resume true s))) (E.get)))
      (export main)))
  (error
    CDZ0201
    (message "resumes with a value of type")
    (message "Bool")
    (message "result type is Int64")))

; A `resume`'s NEXT-STATE (second operand) must match the handler's SEED type: `(handle E 0 …)` seeds an
; Int64 state, so resuming with a Bool or String next-state is CDZ0201 naming the next-state type + "state
; type is Int64". A next-state that matches the seed (arithmetic on `s`, `s` itself, a same-shape tuple seed)
; is well-typed and the handler folds + runs. (Migrated from rcdzc
; resuming_with_a_wrong_type_next_state_is_cdz0201.)
(case
  "resuming with a Bool next-state under an Int64-seeded handler is a type error"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5 true))) (+ (E.get) 1)))
      (export main)))
  (error CDZ0201 (message "next-state of type") (message "Bool") (message "state type is Int64")))

(case
  "resuming with a String next-state under an Int64-seeded handler is a type error"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5 "x"))) (+ (E.get) 1)))
      (export main)))
  (error CDZ0201 (message "next-state of type") (message "String") (message "state type is Int64")))

(case
  "resuming with a matching arithmetic next-state under an Int64 seed folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get () s (resume 5 (+ s 1)))) (+ (E.get) 1)))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "resuming with a same-shape tuple next-state under a tuple seed folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E #tuple(0 0) ((get () s (resume 5 #tuple(1 2)))) (E.get)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

; A `resume` in the TAIL of a `let` body is tail-resumptive (the let's value IS its body's value), so the
; tail-resume peel must keep the `let` around both the value and the next-state rather than declining — the
; shape a discrete-event `sleep` arm uses. `(do (Sim.sleep 3) 42)` under a clock handler resumes and yields
; the continuation 42. A `ctl`-form arm that BINDS `k` but never references it (resuming via its own
; `resume`) is an ordinary tail-resumptive arm — the vacuous `k` binder is dropped, not declined; same
; program with the extra `(sleep (d) s k …)` binder still folds → 42. (Migrated from rcdzc
; a_let_wrapped_tail_resume_folds / a_ctl_arm_with_an_unused_k_binder_is_an_ordinary_resumptive_arm.)
(case
  "a let-wrapped tail resume in a sleep arm folds and runs"
  (input
    (do
      (effect Sim (op sleep (-> Int64 Unit)) (op now (-> Unit Int64)))
      (def
        (main)
        (handle
          Sim
          0
          ((now (u) s (resume s s)) (sleep (d) s (let ((wake (+ s d))) (resume unit wake))))
          (do (Sim.sleep 3) 42)))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a ctl arm with an unused k binder is an ordinary resumptive arm and runs"
  (input
    (do
      (effect Sim (op sleep (-> Int64 Unit)) (op now (-> Unit Int64)))
      (def
        (main)
        (handle
          Sim
          0
          ((now (u) s (resume s s)) (sleep (d) s k (let ((wake (+ s d))) (resume unit wake))))
          (do (Sim.sleep 3) 42)))
      (export main)))
  (call main)
  (output (: 42 Int64)))

; The tail-resume peel COMPOSES through NESTED/combination wrappings, not just a single wrap: a `resume` in
; the tail of a `let` around a `do`, of nested `let`s, of a `match` inside a `let`, and of a `do` inside a
; `match` arm each fold to a tail-resumptive arm and run. Seed 0, one `(E.get)` perform, so the program value
; is the resumed value. These pin that the peel recurses through combinations — a peel that handled only one
; wrapping level would regress them. (Edge-probed by v-wasmtime-migration.)
(case
  "a tail resume under a let wrapping a do folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u) s (let ((x 1)) (do (+ x 1) (resume (+ s x) s))))) (E.get)))
      (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a tail resume under nested lets folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main)
        (handle E 0 ((get (u) s (let ((a 1)) (let ((b 2)) (resume (+ s (+ a b)) s))))) (E.get)))
      (export main)))
  (call main)
  (output (: 3 Int64)))

(case
  "a tail resume under a match inside a let folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          0
          ((get (u) s (let ((x s)) (match (> x -1) (true (resume (+ x 7) s)) (false (resume 0 s))))))
          (E.get)))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "a tail resume under a do inside a match arm folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          0
          ((get (u) s (match (> s -1) (true (do 99 (resume (+ s 5) s))) (false (resume 0 s)))))
          (E.get)))
      (export main)))
  (call main)
  (output (: 5 Int64)))

; The composition also holds at DEPTH THREE (a let around a match around a do around the resume), and the
; NEXT-STATE slot may itself be a `match` over the state that threads a transition across performs: seeded 0,
; arm `(resume s (match (> s 0) (true (- s 1)) (false (+ s 1))))` steps state 0 -> 1 (then 1 -> 0), so
; `(+ (E.get) (E.get))` reads 0 then 1 = 1. (Edge-probed by v-wasmtime-migration; extends the nested-wrapping
; cases above.)
(case
  "a tail resume under a three-level let-match-do nesting folds and runs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          0
          ((get
              (u)
              s
              (let ((x s)) (match (> x -1) (true (do 7 (resume (+ x 4) s))) (false (resume 0 s))))))
          (E.get)))
      (export main)))
  (call main)
  (output (: 4 Int64)))

(case
  "a next-state computed by a match over the state threads a transition across performs"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main)
        (handle
          E
          0
          ((get (u) s (resume s (match (> s 0) (true (- s 1)) (false (+ s 1))))))
          (+ (E.get) (E.get))))
      (export main)))
  (call main)
  (output (: 1 Int64)))

; A NON-TAIL resume and an ABORTING (non-resumptive) arm each have a well-defined handle value (v-effects
; rulings). In `(do (resume s s) 99)` the `resume` runs as a STATEMENT — its continuation fires for effect and
; its value is discarded — then the arm's tail `99` is the handle value (arm-tail-value-wins, the two-hole/
; non-tail semantics; cf the 14c pyre6 capability). An arm that NEVER resumes ABORTS: the continuation (here
; the `(+ [] 1)` around the perform) is DROPPED and the handle yields the arm's own value. (Edge-probed by
; v-wasmtime-migration; minimal non-tail + abort coverage beside the complex pyre6/Bail cases.)
(case
  "a non-tail resume runs the continuation as a statement and the arm tail is the handle value"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u) s (do (resume s s) 99))) (E.get)))
      (export main)))
  (call main)
  (output (: 99 Int64)))

(case
  "a handler arm that never resumes aborts and the handle yields the arm's own value"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 0 ((get (u) s 42)) (+ (E.get) 1)))
      (export main)))
  (call main)
  (output (: 42 Int64)))

(case
  "a host delegation of an effect the body never reaches is latent authority and is rejected"
  (doc
    "The delegation twin of the no-home reject above: `main` `host`-delegates `log` but its body is
           `42` and never performs `log.emit`, so the entrypoint would carry a granted-but-unexercised
           capability — latent authority, rejected at compile time (CDZ0404, capabilities-and-effects.md
           #Host Delegation Is An Entrypoint's Prerogative). Contrast the no-home CDZ0401 above (an effect
           PERFORMED with no home); here the effect is DELEGATED but never reached. (migrated from rcdzc
           a_delegation_of_an_unreached_effect_is_cdz0404.)")
  (input (do (effect log (op emit (-> String Unit))) (def (main) (host (log) 42)) (export main)))
  (error CDZ0404))

(case
  "an intra-program handler whose body never performs its effect is inert and runs to the body value"
  (doc
    "The INTRA-PROGRAM contrast to the host-delegation latent-authority reject above: a `host`-delegated
           effect the body never reaches is CDZ0404 (a granted-but-unexercised capability), but an in-program
           `(handle E …)` whose body never performs `E` is NOT rejected — the handler is simply inert and the
           expression runs to its body's value. Latent authority is a HOST-BOUNDARY property (a capability that
           would cross to the boundary unexercised); an in-program handler installs no capability, so an
           unexercised one is fine. Here `(handle E 99 ((get (u) s (resume s s))) 7)` never performs `E.get`, so
           the arm never fires and the body `7` is the value. (Edge-probed by v-wasmtime-migration.)")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (main) (handle E 99 ((get (u) s (resume s s))) 7))
      (export main)))
  (call main)
  (output (: 7 Int64)))

(case
  "the same declared effect is handled in-program by one entrypoint and delegated by another"
  (doc
    "Host-binding is a ROUTING decision made at the entrypoint, not a declaration-time property
           (capabilities-and-effects.md #Host-Binding Is A Routing Decision Made At The Entrypoint): an
           effect declaration is a routing-agnostic contract, so ONE `(effect E …)` may be handled entirely
           in-program by one entrypoint AND delegated to the host by another, in the SAME program. Here
           `handled` wraps `(E.ask)` in a `(handle E …)` that resumes 42 — E is discharged in-program and
           does NOT enter the manifest for this entrypoint; `delegated` performs `(E.ask)` under `(host (E)
           …)` — E escapes to the boundary and IS a capability there. `handled()` = 42, deterministically,
           with no host response needed; the routing is decided by the enclosing handler/delegation, never by
           `E`'s declaration.")
  (input
    (do
      (effect E (op ask (-> Unit Int64)))
      (def (handled) (handle E 0 ((ask (u) s (resume 42 s))) (E.ask)))
      (def (delegated) (host (E) (E.ask)))
      (export handled)
      (export delegated)))
  (call handled)
  (output (: 42 Int64)))

(case
  "a program that delegates no effect is pure and never suspends"
  (doc
    "Witnesses capabilities-and-effects.md #Purity Is The Empty Effect Row: a program that reaches
           no effect it must route runs straight to normal termination, makes no host call, and has an
           empty manifest. This is the same property the compiler component itself has.")
  (input (do (def (main) (+ 20 22)) (export main)))
  (output (: 42 Int64))
  (host-calls))

(case
  "two effects declared with the same name are distinct, not one merged effect"
  (doc
    "Two `(effect Log …)` declarations name the SAME bare `Log` but declare DIFFERENT operation
           sets — the first only `emit`, the second only `record`. They are two DISTINCT effects
           (capabilities-and-effects.md #An Effect's Operations Are A Closed Set: an effect's identity is
           its declaration, not its name), NOT one effect merging both operation sets. A bare `Log`
           reference resolves the first-declared, whose closed operation set is `{emit}`; so a handler arm
           naming `record` — the SECOND Log's operation — names an operation the first Log does not
           declare, rejected CDZ0403. Pins that a same-name second declaration never leaks its operations
           into the first (were the two conflated into one effect declaring `{emit, record}`, the `record`
           arm would be accepted). This is the effect twin of the duplicate-definition rule (11-modules):
           a name resolves to one declaration, never a silent union across same-named declarations.")
  (input
    (do
      (effect Log (op emit (-> Int64 Int64)))
      (effect Log (op record (-> Int64 Int64)))
      (def (main) (handle Log 0 ((record (n) s (resume n s))) 0))
      (export main)))
  (error CDZ0403))

(case
  "an effect operation returning a SUM is resumed with a sum value and matched"
  (doc
    "The effect/sum intersection: `Ask`'s operation `query` is typed `(-> Int64 Resp)` where `Resp`
           is a user sum `(Yes Int64) | No`. An in-program handler discharges it by RESUMING with a
           constructed sum value `(Resp.Yes n)`, and the body MATCHES the operation's result on `Resp`'s
           variants. `(handle Ask unit ((query (n) s (resume (Resp.Yes n) s))) (match (Ask.query 5) …))`
           resumes with `(Yes 5)`, the match binds `v = 5` → 5. Pins that a sum flows through an effect
           operation's result — constructed in the handler arm, resumed, and deconstructed at the perform
           site — the sum companion of the Int/Unit-resuming handler cases (none of which resume a sum).")
  (input
    (do
      (type Resp (Yes Int64) (No))
      (effect Ask (op query (-> Int64 Resp)))
      (def
        (main (: k Int64))
        (handle
          Ask
          unit
          ((query (n) s (resume (Resp.Yes n) s)))
          (match (Ask.query k) ((Resp.Yes v) v) ((Resp.No) -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case
  "a performed sum carrying a TUPLE payload is matched and the arm reads MULTIPLE payload elements"
  (doc
    "The effect × sum-with-compound-payload intersection, and a soundness pin against the backend's
           per-arm-body CSE (which shares the sum-payload prefix when an arm reads more than one payload
           element). The performed op returns `(Option (Tuple Int64 Int64))`, the handler resumes a `(Some
           (k, k+1))`, and the matching arm reads BOTH tuple elements. `Look.find : Int64 -> (Option (Tuple
           Int64 Int64))`, arm `(find (k) s (resume (Some (tuple k (+ k 1))) s))`; `(Look.find 5)` resumes
           `(Some (5, 6))`, so the `(Some p)` arm computes `(+ (. p 0) (. p 1))` = `5 + 6` = 11. Pins that a
           sum carrying a TUPLE payload flows through an effect op's result and the arm's two payload
           projections (`.0`, `.1`) — which the per-arm-body CSE folds to a shared payload load — stay sound
           over the effect-produced value, because the fold discharges the perform to a concrete resumed
           value before the optimizer runs. Both backends → 11. The compound-payload companion of the
           scalar-payload sum-resume case above.")
  (input
    (do
      (effect Look (op find (-> Int64 (Option (Tuple Int64 Int64)))))
      (def
        (main)
        (handle
          Look
          0
          ((find (k) s (resume (Some #tuple(k (+ k 1))) s)))
          (match (Look.find 5) ((Some p) (+ (. p 0) (. p 1))) (None 0))))
      (export main)))
  (output (: 11 Int64)))

(case
  "an effect operation taking a SUM parameter matches it in the handler arm"
  (doc
    "The mirror of the sum-RESULT case: `Exec.run` is typed `(-> Cmd Int64)` where `Cmd` is a user
           sum `(Add Int64) | (Mul Int64)`. The PERFORM passes a runtime-built sum `(Exec.run (Cmd.Mul k))`,
           and the handler arm MATCHES the operation's `Cmd` parameter to dispatch — `Cmd.Mul n` resumes
           `(* n 2)`, `Cmd.Add n` resumes `(+ n 1)`. `(Exec.run (Cmd.Mul 5))` → `2*5` = 10. Pins that a sum
           flows INTO an effect operation as its argument, built at the perform site and deconstructed by
           the handler — the operand companion of the sum-result case above.")
  (input
    (do
      (type Cmd (Add Int64) (Mul Int64))
      (effect Exec (op run (-> Cmd Int64)))
      (def
        (main (: k Int64))
        (handle
          Exec
          unit
          ((run (c) s (match c ((Cmd.Add n) (resume (+ n 1) s)) ((Cmd.Mul n) (resume (* n 2) s)))))
          (Exec.run (Cmd.Mul k))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a SUM is threaded as a handler's folded state across operations"
  (doc
    "A handler's STATE is a user sum `St = (Cnt Int64)` — the value threaded across the operations it
           discharges (capabilities-and-effects.md #A Handler Threads State Across The Operations It
           Discharges). Seeded `(St.Cnt 0)`, each `bump` arm reads the current count out of the sum state
           (`cur`), resumes with it, and threads the incremented sum (`nxt`) as the new state — so two
           `(Tick.bump)`s see 0 then 1. `(+ (bump) (bump))` = 0 + 1 = 1. Pins that a sum is a valid handler
           state value, deconstructed and rebuilt across resumes (the sum companion of the Int-state cases).")
  (input
    (do
      (type St (Cnt Int64))
      (effect Tick (op bump (-> Unit Int64)))
      (def (cur (: s St)) (match s ((St.Cnt c) c)))
      (def (nxt (: s St)) (match s ((St.Cnt c) (St.Cnt (+ c 1)))))
      (def
        (main (: k Int64))
        (handle Tick (St.Cnt 0) ((bump (u) s (resume (cur s) (nxt s)))) (+ (Tick.bump) (Tick.bump))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "a handler state of TWO tries (tuple) updates each side independently across resumes"
  (doc
    "The multi-field upgrade of the sum-state case above (whose state wraps ONE counter): the
           handler's state is a TUPLE of two maps, each op destructuring the pair and rebuilding it with
           ITS side updated — two addl grow the left trie, one addr the right, and sizes reads both
           (2·10 + 1 = 21). A state-slot rebuild that clobbered the untouched side (or aliased the two
           tries) would misreport a size. The two-table handler shape (e.g. a symbol table beside a
           diagnostics table) threaded as one compound state.")
  (input
    (do
      (effect Tw (op addl (-> Int64 Int64)) (op addr (-> Int64 Int64)) (op sizes (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          Tw
          #tuple(Map.empty Map.empty)
          ((addl (v) s (match s (#tuple(l r) (resume 0 #tuple((Map.insert l v v) r)))))
            (addr (v) s (match s (#tuple(l r) (resume 0 #tuple(l (Map.insert r v v))))))
            (sizes (u) s (match s (#tuple(l r) (resume (+ (* 10 (Map.len l)) (Map.len r)) s)))))
          (do (Tw.addl 1) (Tw.addl 2) (Tw.addr 10) (Tw.sizes))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 21 Int64)))

(case
  "a RECORD handler state evolves a table and a counter that genuinely DIVERGE"
  (doc
    "The record companion, with a divergence witness: the state is `(record (tbl …) (ops …))` and
           each put inserts into the table AND increments the counter — but the table DEDUPES (three
           puts, two distinct keys) while the counter counts every op, so tbl-len 2 ≠ ops 3 (→ 23). The
           divergence proves both fields genuinely evolve per-resume rather than mirroring one count; a
           state rebuild that recomputed one field from the other would collapse them. Field access via
           projection, rebuild via the record constructor — the row machinery inside the arm.")
  (input
    (do
      (effect St (op put (-> Int64 Int64)) (op stats (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #record((= tbl Map.empty) (= ops 0))
          ((put (v) s (resume 0 #record((= tbl (Map.insert s.tbl v v)) (= ops (+ s.ops 1)))))
            (stats (u) s (resume (+ (* 10 (Map.len s.tbl)) s.ops) s)))
          (do (St.put 5) (St.put 6) (St.put 5) (St.stats))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 23 Int64)))

(case
  "a SET-valued handler state accumulates uniques across resumes (the visited-set idiom)"
  (doc
    "The Set face of heap handler state (the map-state rows insert; a set's DEDUP across resumes
           is the distinct contract): 20 marks of `i mod 7` feed a handler whose state is a Set — each
           arm resumes a dup-flag and inserts — and a final count reads 7 uniques. The visited-set a
           graph walk carries: membership decided against the accumulated state at every resume, the
           insert a no-op for repeats (a state thread that re-seeded or double-inserted would inflate
           the count).")
  (input
    (do
      (effect Seen (op mark (-> Int64 Int64)) (op count (-> Unit Int64)))
      (def (feed (: i Int64) (: n Int64)) (if (= i n) 0 (+ (Seen.mark (% i 7)) (feed (+ i 1) n))))
      (def
        (main (: n Int64))
        (handle
          Seen
          #set()
          ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v)))
            (count (u) s (resume (Set.len s) s)))
          (do (feed 0 n) (Seen.count))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 7 Int64))
  (live-objects known-leak))

(case
  "the mark op's dup-flag RESULT counts repeats while the set state dedupes"
  (doc
    "The companion reading the op RESULTS instead of the final state: each mark resumes 1 iff the
           value was already seen, and the caller SUMS the flags — 20 feeds of `i mod 7` produce 13
           repeats (20 − 7 first-sightings). Pins that the per-resume result is computed against the
           state BEFORE that resume's insert (an arm that inserted first and then tested would flag
           every mark as a repeat), composing the membership read, the state advance, and the resumed
           value in one arm.")
  (input
    (do
      (effect Seen (op mark (-> Int64 Int64)))
      (def (feed (: i Int64) (: n Int64)) (if (= i n) 0 (+ (Seen.mark (% i 7)) (feed (+ i 1) n))))
      (def
        (main (: n Int64))
        (handle
          Seen
          #set()
          ((mark (v) s (resume (if (Set.contains s v) 1 0) (Set.insert s v))))
          (feed 0 n)))
      (export main)))
  (call main (: 20 Int64))
  (output (: 13 Int64))
  (live-objects known-leak))

(case
  "a perform in a match-arm guard is discharged by the enclosing handle"
  (doc
    "`(handle Ask 5 ((get () s (resume s (- s 1)))) (match 9 ((guard n (> (Ask.get) 3)) 100) (n 200)))`
           — a perform `(Ask.get)` inside a match-arm GUARD condition, discharged by an intra-program
           `handle`. A perform in the SCRUTINEE, ARM BODY, or an IF CONDITION under the same handle all fold,
           and NOW so does a guard condition — for the SOUND, NARROW shape: a guarded arm whose inner pattern
           is IRREFUTABLE (a bare name / `_`) followed by an irrefutable catch-all. Such a match is selected
           iff the guard holds, so `reduce_handle` desugars it to `(if <guard> <arm-body> <catch-all-body>)`
           (each binder let-bound to the scrutinee), where the guard is an `if` CONDITION — a strict-first
           position the if-condition fold routes through the enclosing handle. The guard reads the seed 5,
           `5 > 3` holds, so the first arm fires → 100. (A REFUTABLE guarded pattern now ALSO folds — via a
           match that keeps the pattern and hoists the guard into an inner `if`, see the cases below. MULTIPLE
           guarded arms — which sequence handler state per arm-test — remain not-this-shape and decline
           cleanly, an honest 'not yet reducible' todo, never the misleading 'no enclosing handler'.)
           UPDATE (guards-side-effect-free, operator directive PR #2543, CDZ0407): a perform in a guard is NOW
           a COMPILE ERROR — the fold this once pinned is removed. `(Ask.get)` in the guard cond → CDZ0407.
           The historical fold rationale above is retained for context; the workaround is to lift the perform
           to a `let` before the match and guard on the bound value.")
  (input
    (do
      (effect Ask (op get (-> Int64)))
      (def
        (main)
        (handle
          Ask
          5
          ((get () s (resume s (- s 1))))
          (match 9 ((guard n (> (Ask.get) 3)) 100) (n 200))))
      (export main)))
  (error CDZ0407))

(case
  "a performing match-arm guard folds with WILDCARD patterns (no binder to let-bind)"
  (doc
    "The wildcard spelling of the guard-desugar above: both the guarded arm's inner pattern and the
           catch-all are `_` (bind nothing), so the desugar to `(if <guard> <arm-body> <catch-all-body>)`
           needs NO enclosing `let` — the bare `if` suffices (the `binders.is_empty()` path). `Ask` seeded 5,
           `(> (Ask.get) 3)` reads 5 → true, so the first arm fires → 100. Pins that the guard-routing
           desugar handles a wildcard-patterned guarded arm (no scrutinee binder) as well as a named one.
           UPDATE (guards-side-effect-free, CDZ0407): `(Ask.get)` in the guard cond is NOW a COMPILE ERROR.")
  (input
    (do
      (effect Ask (op get (-> Int64)))
      (def
        (main)
        (handle
          Ask
          5
          ((get () s (resume s (- s 1))))
          (match 9 ((guard _ (> (Ask.get) 3)) 100) (_ 200))))
      (export main)))
  (error CDZ0407))

(case
  "a performing match-arm guard on a REFUTABLE pattern folds (keeps the match, hoists the guard)"
  (doc
    "The refutable-pattern face of the performing guard-desugar (breaker bg-family). When the guarded
           arm's inner pattern is REFUTABLE — a literal, ctor, `(bin …)`, or `(tuple …)` — the irrefutable
           rewrite (`(if g b b2)`) would be UNSOUND: it drops the pattern-match, so a scrutinee FAILING the
           pattern would still run the guard `g`. The sound rewrite KEEPS the pattern and hoists the
           performing guard into an `if` INSIDE the matched arm: `(match k ((guard P g) b) (_ b2))` ≡
           `(match k (P (if g b b2)) (_ b2))`. Here the bit-pattern `(bin (u8 tag) (u8 val))` matches the
           two-byte scrutinee (tag=7, val=42), the guard `(> val (St.quota))` reads the seed (n) and holds
           for n<42, so the arm yields `(+ (* 100 tag) val)` = 742; for n≥42 the guard fails and the arm's
           inner `if` falls to the catch-all -1. A scrutinee that FAILS the pattern reaches the catch-all
           WITHOUT running the guard perform (the match, not the guard, gates it). Seeded n=5 → 742.
           UPDATE (guards-side-effect-free, CDZ0407): `(St.quota)` in the guard cond is NOW a COMPILE ERROR.")
  (input
    (do
      (effect St (op quota (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((quota (u) s (resume s (+ s 1))))
          (match
            (bin (u8 (UInt8.wrap 7)) (u8 (UInt8.wrap 42)))
            ((guard (bin (u8 tag) (u8 val)) (> val (St.quota))) (+ (* 100 tag) val))
            (_other -1))))
      (export main)))
  (error CDZ0407))

(case
  "a performing guard on a refutable pattern whose guard FAILS falls to the catch-all"
  (doc
    "The guard-fails path of the refutable performing-guard fold: same shape as above but seeded so the
           guard is false — `(> val (St.quota))` with `val`=42 and the seed n=50, so `42 > 50` is FALSE. The
           pattern still matches (tag=7, val=42), the hoisted inner `if` evaluates the guard (which reads the
           seed 50) and takes the else branch → the catch-all -1. Pins that a matched pattern with a failing
           performing guard folds to the fall-through, not the guarded body.
           UPDATE (guards-side-effect-free, CDZ0407): `(St.quota)` in the guard cond is NOW a COMPILE ERROR.")
  (input
    (do
      (effect St (op quota (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((quota (u) s (resume s (+ s 1))))
          (match
            (bin (u8 (UInt8.wrap 7)) (u8 (UInt8.wrap 42)))
            ((guard (bin (u8 tag) (u8 val)) (> val (St.quota))) (+ (* 100 tag) val))
            (_other -1))))
      (export main)))
  (error CDZ0407))

(case
  "a performing guard on a TUPLE-destructuring pattern folds"
  (doc
    "The tuple-pattern spelling of the refutable performing-guard fold: `(guard (tuple tag val) (> val
           (St.quota)))` destructures a tuple scrutinee, and the performing guard hoists into the matched
           arm's inner `if` exactly as for the bit-pattern. `(tuple 7 42)` matches (tag=7, val=42), guard
           `42 > 5` holds → `(+ 700 42)` = 742. Confirms the refutable-pattern guard-desugar is
           pattern-shape-agnostic (bit patterns, tuples, and by extension ctor patterns all route).
           UPDATE (guards-side-effect-free, CDZ0407): `(St.quota)` in the guard cond is NOW a COMPILE ERROR.")
  (input
    (do
      (effect St (op quota (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((quota (u) s (resume s (+ s 1))))
          (match
            #tuple(7 42)
            ((guard #tuple(tag val) (> val (St.quota))) (+ (* 100 tag) val))
            (_other -1))))
      (export main)))
  (error CDZ0407))

(case
  "an effectful condition of a same-constructor if is performed exactly once"
  (doc
    "The evaluate-ONCE pin for the common-constructor if-arm hoist, observable through handler
           state: `(if (< (Ctr.tick) 1) (tuple 1 2) (tuple 3 4))` — both arms build a same-arity tuple,
           so the hoist rewrites to per-element selections over ONE condition value. The counter arm
           `(tick (_) s (resume s (+ s 1)))` returns the current count and threads +1. First perform
           returns 0 → the condition is TRUE → t = (1, 2); the trailing `(Ctr.tick)` then returns 1 (the
           state advanced exactly once by the condition). So 100·1 + 10·2 + 1 = 121. A hoist that
           DUPLICATED the condition per payload slot would perform tick twice (returns 0 then 1 — the
           two element selections disagree, t = (1, 4), and the trailing tick returns 2 → 142); one that
           re-evaluated it once more for the second element still skews the trailing read. Pins that the
           rewrite binds the condition to ONE evaluation whose value feeds every payload selection.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (_) s (resume s (+ s 1))))
          (let
            ((t (if (< (Ctr.tick unit) 1) #tuple(1 2) #tuple(3 4))))
            (+ (+ (* 100 (. t 0)) (* 10 (. t 1))) (Ctr.tick unit)))))
      (export main)))
  (call main)
  (output (: 121 Int64)))

; --- An abort abandons frames holding LIVE HEAP operands (the Perceus face of the abortive class) --
; The abortive cases above pin CONTROL (which value wins, what unwinds); these pin MEMORY: a pending
; frame abandoned by an abort may hold heap operands — a consuming op's result, a borrowed lookup —
; whose owners are still live OUTSIDE the handle. The abandoned operands must be reclaimed exactly
; once and the owners left intact: an unwind that double-frees (or skips a retain) corrupts the
; owner's later read; one that leaks is invisible here but the owner-read pins the correctness half.
(case
  "an abort abandons a pending consuming op and the shared binding survives"
  (doc
    "`(+ (List.len (List.push xs 9)) (Bail.bail 3))` under `handle Bail` — the LEFT operand has
           already run when the abort fires: `(List.push xs 9)` consumed the still-live `xs` (retain →
           path-copy) and its result sits in the abandoned frame. The abort discards the pending `+`
           and yields 3; the outer `(List.len xs)` then reads the ORIGINAL `xs` → 1, so 3 + 1 = 4. A
           lowering that unwinds without dropping the abandoned push-result leaks it (unobservable
           here), but one that double-drops — or that skipped the retain because the consume 'would be
           abandoned' — corrupts `xs` and misses 4. Pins the retain-then-abandon interaction.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main (: d Int64))
        (let
          ((xs (List.push #list() d)))
          (+
            (handle Bail 0 ((bail (n) s n)) (+ (List.len (List.push xs 9)) (Bail.bail 3)))
            (List.len xs))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 4 Int64)))

(case
  "heap-valued handler state above an inner abort keeps threading"
  (doc
    "Nested handles where the OUTER handler's state is a HEAP value (a list accumulator) and the
           INNER handle aborts: `(+ (handle Bail … (Bail.bail 10)) (Acc.add 5))` under `handle Acc
           (list) ((add (n) s (resume (List.len s) (List.push s n))))`. The inner abort yields 10 and
           unwinds ONLY its own handle — the outer Acc handler's list state must survive the unwind
           untouched, so the subsequent `(Acc.add 5)` reads len [] = 0 and 10 + 0 = 10. An unwind that
           reclaimed the outer handler's state cell (or reset its threading) corrupts the later perform.
           The heap-state companion of the scalar three-nested-handlers abort case above.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (effect Acc (op add (-> Int64 Int64)))
      (def
        (main (: d Int64))
        (handle
          Acc
          #list()
          ((add (n) s (resume (List.len s) (List.push s n))))
          (+ (handle Bail 0 ((bail (n) s2 n)) (Bail.bail 10)) (Acc.add 5))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 10 Int64)))

(case
  "an abort abandons a pending borrowed map lookup and the map survives"
  (doc
    "The borrowed-operand face: `(+ (Option.expect (Map.lookup m \"k\") \"v\") (Bail.bail 20))` —
           the lookup's extracted value (from the still-live `m`) is pending in the abandoned frame when
           the abort fires. The handle yields 20; the outer `(Map.len m)` must still see the intact map
           → 1, so 21. An unwind that dropped the abandoned lookup result as if OWNED would free the
           value `m` still holds — the abort-path twin of the borrowed-key ownership discipline the
           lookup/contains emits observe on the normal path.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (def
        (main (: d Int64))
        (let
          ((m (Map.insert Map.empty "k" 1)))
          (+
            (handle
              Bail
              0
              ((bail (n) s n))
              (+ (Option.expect (Map.lookup m "k") "v") (Bail.bail 20)))
            (Map.len m))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 21 Int64)))

(case
  "a fresh-id supply threads state across two sibling recursive calls in one arm"
  (doc
    "The natural effectful TREE WALK — `relabel(Node l r) = relabel(l) + relabel(r)` with the
           `Fresh.next` gensym at the leaf. Two SIBLING self-recursive calls in one `match` arm: the
           handler state the FIRST sibling advances must be visible to the SECOND (each leaf draws the
           next id). Under a 0-based counter a 3-leaf tree draws 0, 1, 2 → 3. The single-return
           specialization threaded only the INCOMING state to each self-call, so both siblings drew the
           same id (a state-reset miscompile) and the shape was DECLINED; the multi-value-return
           specialization (`f#ctx` yields `(value, out-state)`, each self-call let-bound and its out-state
           threaded to the next sibling) folds it correctly. The canonical compiler-pass gensym over a
           tree (node numbering, SSA names, type-variable ids).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Tree (Leaf) (Node Tree Tree))
      (def
        (relabel (: t Tree))
        (match t ((Leaf) (Fresh.next)) ((Node l r) (+ (relabel l) (relabel r)))))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (relabel (Node (Node (Leaf) (Leaf)) (Leaf)))))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "sibling-recursive effect threading is left-to-right (order-observing)"
  (doc
    "The same tree walk but with a NON-COMMUTATIVE combiner `(- (relabel l) (relabel r))`, so the
           result witnesses the EVALUATION ORDER of the two siblings: the LEFT sibling draws first (the
           smaller id). `(Node (Leaf) (Leaf))` → left id 0, right id 1 → 0 - 1 = -1. A right-first or
           state-reset threading would give 0 - 0 = 0 or 1 - 0 = 1; -1 pins strict left-to-right
           out-state threading between the siblings.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Tree (Leaf) (Node Tree Tree))
      (def
        (relabel (: t Tree))
        (match t ((Leaf) (Fresh.next)) ((Node l r) (- (relabel l) (relabel r)))))
      (def (main) (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (relabel (Node (Leaf) (Leaf)))))
      (export main)))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a perform BETWEEN two sibling recursive calls threads the intervening state"
  (doc
    "`relabel(Node l r) = (relabel l) + Fresh.next() + (relabel r)` — a discharged perform sits
           BETWEEN the two sibling self-calls on the strict spine, so it draws the id the LEFT sibling
           left and hands the advanced state to the RIGHT sibling. `(Node (Leaf) (Leaf))`: left draws 0,
           the middle perform draws 1, right draws 2 → 0 + 1 + 2 = 3. Exercises the multi-value out-state
           threading interleaved with an ordinary perform in one arm.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Tree (Leaf) (Node Tree Tree))
      (def
        (relabel (: t Tree))
        (match t ((Leaf) (Fresh.next)) ((Node l r) (+ (+ (relabel l) (Fresh.next)) (relabel r)))))
      (def (main) (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (relabel (Node (Leaf) (Leaf)))))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "sibling recursive calls sequenced through let bindings thread state"
  (doc
    "The `let`-sequenced form of the sibling walk — `(Node l r) => let a = relabel l in let b =
           relabel r in a - b` — the shape a hand-written SSA linearizer uses (bind the left result, then
           the right, threading the id counter through the RESULT). The second binding's init must thread
           against the state the first advanced. `(Node (Leaf) (Leaf))` → a = 0, b = 1 → -1. Confirms the
           multi-value out-state threads through `let` inits, not only bare operator operands.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Tree (Leaf) (Node Tree Tree))
      (def
        (relabel (: t Tree))
        (match
          t
          ((Leaf) (Fresh.next))
          ((Node l r) (let ((a (relabel l))) (let ((b (relabel r))) (- a b))))))
      (def (main) (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (relabel (Node (Leaf) (Leaf)))))
      (export main)))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "a sibling-recursive walk threads a HEAP list accumulator across the siblings"
  (doc
    "The ssa/collect face of the multi-value-return walk: each leaf draws a fresh id into a
           singleton list and a Node CONCATENATES its two sibling walks' lists — `collect(Node l r) =
           List.concat (collect l) (collect r)`. The out-state a self-call advances is threaded to its
           sibling, and the VALUE carried back through the tuple return is now a HEAP value (a List), not
           a scalar — so this pins that the multi-value return threads a heap-allocated result across the
           siblings correctly (a `.0` projection off the runtime tuple, not just an Int64). A 3-leaf tree
           draws ids 0,1,2 into a length-3 list. Regression guard for the real SSA-linearizer shape,
           where the accumulated instruction list is the threaded heap value.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Tree (Leaf) (Node Tree Tree))
      (def
        (collect (: t Tree))
        (match
          t
          ((Leaf) (List.push #list() (Fresh.next)))
          ((Node l r) (List.concat (collect l) (collect r)))))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (List.len (collect (Node (Node (Leaf) (Leaf)) (Leaf))))))
      (export main)))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "a post-order effectful walk draws each node's id AFTER both children (SSA reg-alloc shape)"
  (doc
    "The exact SSA register-allocation shape: a node's own id is drawn AFTER lowering both children
           — `lower(Bin l r) = let a = lower l in let b = lower r in Fresh.next()`, so the parent register
           number follows its subtrees'. The two sibling self-calls (`lower l`, `lower r`) each advance
           the id supply, then the node itself draws the NEXT id — the multi-value return must thread the
           counter through BOTH children and leave the parent's draw last. `Bin (Lit) (Bin (Lit) (Lit))`
           over a 0-based counter: left Lit=0, right subtree (Lit=1, Lit=2, its Bin=3), root Bin=4 → the
           root's result register is 4. Pins the natural post-order gensym the compiler-ml SSA linearizer
           writes (its hand-threaded counter can become this effectful walk once repro-1 landed).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Expr (Lit Int64) (Bin Expr Expr))
      (def
        (lower (: e Expr))
        (match
          e
          ((Lit v) (Fresh.next))
          ((Bin l r) (let ((a (lower l))) (let ((b (lower r))) (Fresh.next))))))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (lower (Bin (Lit 1) (Bin (Lit 2) (Lit 3))))))
      (export main)))
  (output (: 4 Int64))
  (live-objects 0))

(case
  "a cross-function recursive fold's out-state threads to a later perform in the caller's continuation"
  (doc
    "The CALLER-observed out-state face of the multi-value return (the recursive analogue of the
           inlined helper-call out-state threading). `run-ops` recursively performs `Prim.run` per list
           element, advancing the handler state s -> s+1 each time; the handle body is `(do (run-ops [1 2
           3]) (Prim.run 0))`, so a TRAILING perform in the caller's `do` — AFTER the recursive fold
           returns — must observe the state the recursion accumulated. Three performs advance 0 -> 3, and
           the trailing `(Prim.run 0)` resumes with s = 3. The single-return specialization drops the
           recursion's final out-state (returns the incoming state unchanged), silently miscompiling the
           trailing perform to the PRE-recursion 0; forcing MULTI-VALUE specialization when the caller's
           spine observes the out-state threads the advance through. Regression guard for task #15.")
  (input
    (do
      (effect Prim (op run (-> Int64 Int64)))
      (def
        (run-ops (: ops (List Int64)))
        (match ops (#list(h (.. rest)) (do (Prim.run h) (run-ops rest))) (_ 0)))
      (def
        (main)
        (handle Prim 0 ((run (tag) s (resume s (+ s 1)))) (do (run-ops #list(1 2 3)) (Prim.run 0))))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a LET-BOUND perform after a cross-function recursive helper observes the helper's out-state"
  (doc
    "The LET-INIT face of the caller-observed out-state (breaker tk3d; the bare-do-item face is pinned
           above). A cross-fn recursive helper `pump` emits 3 bytes onto the inner Sink handler's Bytes
           state; a following do-item `(let ((out (Sink.flush))) (Bytes.len out))` binds a perform that must
           observe the 3-byte accumulation. The bug: `mark_caller_observed_outstate`'s perform-detector
           (`reaches_any_perform`) mis-treated the `let`'s raw bindings sublist `((out (Sink.flush)))` as an
           APPLICATION and never descended into the init, so the let-bound flush was NOT seen as observing
           pump's out-state → pump stayed single-return → its Sink-slot advance was dropped → flush read the
           SEED (len 0, a 3-backend silent miscompile). Fixed by routing a `let` through its real init/body
           positions in the perform walk. Now the flush observes 3 → 3.")
  (input
    (do
      (effect Src (op read (-> Unit Int64)))
      (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
      (def (pump (: k Int64)) (if (= k 0) unit (do (Sink.emit (Src.read)) (pump (- k 1)))))
      (def
        (main (: n Int64))
        (handle
          Src
          n
          ((read (u) s (resume s (+ s 1))))
          (handle
            Sink
            (bin)
            ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
              (flush (u) b (resume b b)))
            (do (pump 3) (let ((out (Sink.flush))) (Bytes.len out))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

(case
  "the let-bound-after-helper observation is state-type-general (scalar counter)"
  (doc
    "The scalar face of tk3d (breaker tk3h): the SAME [cross-fn helper advances a slot] x [let-bound
           perform observes it] shape but the state is a plain Int64 counter, not heap Bytes. `pump` bumps
           the Ctr slot 3x cross-function, then `(let ((v (Ctr.get))) (+ v 100))` must read 3 → 103. Before
           the reaches_any_perform let-init fix it read the seed 0 → 100. Pins that the caller-observed
           out-state fix is state-type-general (not Bytes-specific).")
  (input
    (do
      (effect Src (op read (-> Unit Int64)))
      (effect Ctr (op bump (-> Int64 Unit)) (op get (-> Unit Int64)))
      (def (pump (: k Int64)) (if (= k 0) unit (do (Ctr.bump (Src.read)) (pump (- k 1)))))
      (def
        (main (: n Int64))
        (handle
          Src
          n
          ((read (u) s (resume s (+ s 1))))
          (handle
            Ctr
            0
            ((bump (v) c (resume unit (+ c 1))) (get (u) c (resume c c)))
            (do (pump 3) (let ((v (Ctr.get))) (+ v 100))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 103 Int64)))

; The task-#15 caller-observed-out-state fix (the bare-do-item core case above) has adversarial companions
; over the SAME `run-ops` cross-function recursive list fold, each pinning a distinct facet: the out-state
; is observed by a READ-OUT op (not only a state-advancing one), TWO successive folds each thread their
; out-state to the next spine item, and — the negative control — a fold whose out-state is NOT observed
; stays single-return (the caller-side mark must not over-fire). A fourth pins the observer INSIDE the
; fold's base case (a per-element counter read back at the end).
(case
  "a cross-function fold's out-state is observed by a read-out op in the continuation"
  (doc
    "The read-out companion of the caller-observed out-state: `(do (run-ops [1 2 3]) (Prim.total))` —
           `total` resumes with the state UNCHANGED (`resume s s`), so it READS the 3 the cross-fn recursion
           accumulated. Any perform observes the out-state, so multi-value specialization fires and the
           read-out op sees 3 (a single-return path would read the pre-recursion 0).")
  (input
    (do
      (effect Prim (op run (-> Int64 Int64)) (op total (-> Int64)))
      (def
        (run-ops (: ops (List Int64)))
        (match ops (#list(h (.. rest)) (do (Prim.run h) (run-ops rest))) (_ 0)))
      (def
        (main)
        (handle
          Prim
          0
          ((run (tag) s (resume s (+ s 1))) (total () s (resume s s)))
          (do (run-ops #list(1 2 3)) (Prim.total))))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "two successive cross-function folds thread out-state left-to-right in the continuation"
  (doc
    "Two `run-ops` cross-fn folds in the caller's `do` must EACH thread their out-state to the next
           spine item: `(do (run-ops [1 2 3]) (run-ops [1 2]) (Prim.run 0))` — the first advances 0->3, the
           second continues 3->5, and the trailing `(Prim.run 0)` reads 5. A state RESET on the second fold
           would give 2; pins the caller-side mark records BOTH callees and the `do` carries state between
           them.")
  (input
    (do
      (effect Prim (op run (-> Int64 Int64)))
      (def
        (run-ops (: ops (List Int64)))
        (match ops (#list(h (.. rest)) (do (Prim.run h) (run-ops rest))) (_ 0)))
      (def
        (main)
        (handle
          Prim
          0
          ((run (tag) s (resume s (+ s 1))))
          (do (run-ops #list(1 2 3)) (run-ops #list(1 2)) (Prim.run 0))))
      (export main)))
  (output (: 5 Int64))
  (live-objects 0))

(case
  "a cross-function fold whose out-state is NOT observed stays single-return"
  (doc
    "The negative control: `(handle Prim 0 (...) (run-ops [1 2 3]))` — the handle value is the fold's
           VALUE (the base-case 0) and no later spine item performs, so the out-state is UNOBSERVED.
           Multi-value must NOT be forced (the caller-side mark must not over-fire); the single-return path
           folds it correctly to 0. Pins the mark's precision alongside the positive companions.")
  (input
    (do
      (effect Prim (op run (-> Int64 Int64)))
      (def
        (run-ops (: ops (List Int64)))
        (match ops (#list(h (.. rest)) (do (Prim.run h) (run-ops rest))) (_ 0)))
      (def (main) (handle Prim 0 ((run (tag) s (resume s (+ s 1)))) (run-ops #list(1 2 3))))
      (export main)))
  (output (: 0 Int64))
  (live-objects 0))

(case
  "a per-element perform in a cross-function fold threads to a read-out in the base case"
  (doc
    "The observer sits INSIDE the fold's base case: `run-ops` performs `Prim.run` per element then its
           `(_ (Prim.total 0))` base case reads the accumulated state. All three per-element performs advance
           0->3 and the base-case `total` reads it → 3 (proving the performs both ran AND threaded through the
           cross-function recursion, not silently dropped).")
  (input
    (do
      (effect Prim (op run (-> Int64 Int64)) (op total (-> Int64 Int64)))
      (def
        (run-ops (: ops (List Int64)))
        (match ops (#list(head (.. rest)) (do (Prim.run head) (run-ops rest))) (_ (Prim.total 0))))
      (def
        (main)
        (handle
          Prim
          0
          ((run (tag) s (resume s (+ s 1))) (total (u) s (resume s s)))
          (run-ops #list(1 2 3))))
      (export main)))
  (output (: 3 Int64))
  (live-objects 0))

(case
  "a nested inner handler that re-threads its own state folds (merged-context seed from init)"
  (doc
    "Two NESTED handlers over a cross-function recursive loop that performs BOTH effects — the
           merged-context signature. The INNER handler `Tools` re-threads its OWN bound state in the arm
           `(step (a) s (resume a s))` — the resume's next-state is the state BINDER `s`, not a fresh
           value. `type_of` of a bare state binder alone is `Any` (its type is the seed's), so deriving
           the merged inner slot's type from the arms' next-states ALONE yielded `Any` and DECLINED the
           merge — while the SAME handler standalone folded (single-handler `reduce_handle` seeds the slot
           type from the init). The merged path now seeds identically from the inner `init` (`Tools 0` →
           Int64), so a stateful inner handler re-threading `s` folds. `loop 3 0` draws step ids handing
           back the accumulator each turn: 3, then 2, then 1 → stop(6) → 6. (Reported by v-agent-harness
           Inc-2; the fix mirrors the single-handler init-seeded state-type derivation.)")
  (input
    (do
      (effect Model (op ask (-> Int64 Int64)))
      (effect Tools (op step (-> Int64 Int64)) (op stop (-> Int64 Int64)))
      (def
        (loop (: i Int64) (: acc Int64))
        (if (= (Model.ask i) 0) (Tools.stop acc) (loop (- i 1) (Tools.step (+ acc i)))))
      (def
        (main)
        (handle
          Model
          0
          ((ask (q) s (resume q q)))
          (handle Tools 0 ((step (a) s (resume a s)) (stop (a) s (resume a s))) (loop 3 0))))
      (export main)))
  (output (: 6 Int64)))

(case
  "a post-order labeling walk returns a labeled tree (heap result, id drawn after children)"
  (doc
    "The canonical compiler NODE-NUMBERING pass: walk a `Tree`, draw a fresh `Fresh.next` id per node,
           and RETURN a new labeled `Ann` tree (a HEAP result, not a scalar sum). Post-order — each node's
           own id is drawn AFTER labeling both children via `let`-bound sibling recursion, so the two
           sibling self-calls thread the id supply and the parent's id follows its subtrees'. Exercises the
           multi-value return carrying a heap-constructed result across siblings PLUS the parent draw last.
           Tree `(Node (Node Leaf Leaf) Leaf)`: inner-left Leaf=0, inner-right Leaf=1, inner Node=2, outer
           Leaf=3, root Node=4 → the root's label is 4. The real 'label every node, return the labeled
           tree' shape the compiler-ml port's numbering pass writes.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Tree (Leaf) (Node Tree Tree))
      (type Ann (ALeaf Int64) (ANode Int64 Ann Ann))
      (def
        (relabel (: t Tree))
        (match
          t
          ((Leaf) (ALeaf (Fresh.next)))
          ((Node l r) (let ((la (relabel l))) (let ((ra (relabel r))) (ANode (Fresh.next) la ra))))))
      (def (root-id (: a Ann)) (match a ((ALeaf i) i) ((ANode i l r) i)))
      (def
        (main)
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (root-id (relabel (Node (Node (Leaf) (Leaf)) (Leaf))))))
      (export main)))
  (output (: 4 Int64))
  (live-objects known-leak))

(case
  "a fresh-id walk over a THREE-constructor sum threads state across mixed-arity arms"
  (doc
    "The gensym walk generalized to a real `Expr` sum with THREE constructors of DIFFERENT arities —
           `Lit` (nullary, one id), `Neg` (one child + its own id), `Add` (two children + its own id). Each
           arm performs `Fresh.next` and recurses on its children with the id supply threaded left-to-right
           across the (0, 1, or 2) sibling self-calls. Confirms the multi-value sibling-threading is not
           special to a 2-constructor Leaf/Node tree — a match arm with a perform-then-N-siblings folds for
           any arity. `Add (Neg Lit) Lit`: Add=0, Neg=1, Lit-under-Neg=2, right-Lit=3 → sum 6.")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Expr (Lit) (Add Expr Expr) (Neg Expr))
      (def
        (count-ids (: e Expr))
        (match
          e
          ((Lit) (Fresh.next))
          ((Neg x) (+ (Fresh.next) (count-ids x)))
          ((Add l r) (+ (Fresh.next) (+ (count-ids l) (count-ids r))))))
      (def
        (main)
        (handle Fresh 0 ((next (u) s (resume s (+ s 1)))) (count-ids (Add (Neg (Lit)) (Lit)))))
      (export main)))
  (output (: 6 Int64))
  (live-objects 0))

(case
  "an effect-performing helper called inside a recursive self-call's argument folds"
  (doc
    "A recursive driver `run` whose SELF-CALL argument contains a call to a separate effect-performing
           HELPER `turn` — `(run (- fuel 1) (+ acc (turn fuel)))`, where `turn a = Tools.dispatch a`. Threading
           the self-call's arg inlines `turn` (β-reduces + threads its performing body); the inlined
           `Tools.dispatch` resumes `(a a)` (hands its arg back AND as the next state). The resume VALUE and
           NEXT-STATE are the SAME substituted-arg node, and it is RESOLVE-PINNED (a bare param occurrence),
           so the ordinary copy SHARED one node across the two splice positions — a single-parent-arena
           orphan that surfaced the driver's own params as CDZ0101 `unbound name fuel`/`acc` (reported by
           v-agent-harness Inc-3). A DEEP-FRESH copy of the resume value/next-state gives each splice its own
           subtree, re-resolving against the specialized def's sig. `run 4 0` accumulates dispatch(4..1) =
           4+3+2+1 → done(10) → 10. Pins the effectful-helper-in-a-self-call-arg shape a self-hosted pass
           writes (a per-node effectful helper threaded through a recursive walk).")
  (input
    (do
      (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: a Int64)) (Tools.dispatch a))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (+ acc (turn fuel)))))
      (def
        (main)
        (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a))) (run 4 0)))
      (export main)))
  (output (: 10 Int64)))

(case
  "an effectful helper that also reads an outer/driver parameter folds in a self-call arg"
  (doc
    "The follow-up to the single-param effectful-helper-in-a-self-call-arg case: here the helper
           `turn` performs AND references a DRIVER parameter in its own body — `turn(a, acc) = acc +
           Tools.dispatch a`, called as `(run (- fuel 1) (turn fuel acc))`, where `acc` is also `run`'s
           param. Inlining `turn` β-substitutes the driver's `acc` into the helper body by returning the arg
           node AS-IS (the pinned-name fast path), so that `acc` kept a pin to `run`'s now-dead scope; when
           the inline happens INSIDE the recursive self-call's arg, the reduced body lands in the synthesized
           `f#ctx` def where the pinned `acc` no longer resolves → CDZ0101 `unbound name acc`. Deep-fresh-
           copying the reduced inline body drops the stale pins so every name re-resolves against the
           specialized def's sig (carrying the driver's params). `run 4 0` = 4+3+2+1 = 10. (v-agent-harness
           Inc-3 follow-up.)")
  (input
    (do
      (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: a Int64) (: acc Int64)) (+ acc (Tools.dispatch a)))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn fuel acc))))
      (def
        (main)
        (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a))) (run 4 0)))
      (export main)))
  (output (: 10 Int64)))

; NEIGHBORS of the effectful-helper-in-a-self-call-arg deep-fresh-copy fix (breaker): the case above pins
; ONE driver param (acc) read by the inlined helper. These push the same deep-fresh-copy path: TWO driver
; params read at once, the helper called TWICE (nested) in the self-call arg (each inline must get fresh
; pins), and a helper whose OWN param NAME shadows a driver param. All fold cleanly and re-resolve against
; the specialized def's sig — a stale pin from any of these shapes would surface as CDZ0101 unbound name.
(case
  "an effectful helper reading TWO driver parameters folds in a self-call arg"
  (doc
    "The two-driver-param extension: `turn(a, acc, fuel) = acc + fuel + Tools.dispatch a` reads BOTH
           `acc` and `fuel` (both `run`'s params), called `(run (- fuel 1) (turn fuel acc fuel))`. Inlining
           β-substitutes two driver pins into the helper body inside the self-call arg; the deep-fresh-copy
           must drop BOTH stale pins so each re-resolves against the specialized def's sig. With dispatch a →
           a, turn = acc + 2*fuel; run 3 0 = 6, 10, 12 → 12. A copy that missed one pin → CDZ0101.")
  (input
    (do
      (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: a Int64) (: acc Int64) (: fuel Int64)) (+ (+ acc fuel) (Tools.dispatch a)))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn fuel acc fuel))))
      (def
        (main)
        (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a))) (run 3 0)))
      (export main)))
  (output (: 12 Int64)))

(case
  "an effectful helper called twice (nested) in a self-call arg folds each inline independently"
  (doc
    "The helper appears TWICE in the self-call arg — `(turn fuel (turn fuel acc))` — so the inliner
           reduces two copies of the effectful body into the same self-call arg. Each inline must be
           deep-fresh-copied independently; a shared or stale pin across the two copies would collide or fail
           to resolve. With turn(a,acc) = acc + a: run 2 0 → inner turn(2,0)=2, outer turn(2,2)=4; then
           inner turn(1,4)=5, outer turn(1,5)=6 → done 6. Pins that repeated inlining in one arg is sound.")
  (input
    (do
      (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: a Int64) (: acc Int64)) (+ acc (Tools.dispatch a)))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn fuel (turn fuel acc)))))
      (def
        (main)
        (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a))) (run 2 0)))
      (export main)))
  (output (: 6 Int64)))

(case
  "an effectful helper performing UNDER A CONDITIONAL folds in a self-call arg"
  (doc
    "The inlined helper's perform sits inside an `if` BRANCH — `turn(x,acc) = if x==1 then acc + B.b x
           else acc`, called in the self-call arg `(run (- fuel 1) (turn fuel acc))`. Threading the arg
           inlines the helper's `if`; each branch gets its own copy of the incoming state-refs. That copy was
           `copy_pure` (`beta_reduce`), whose pinned-name fast path returned the RESOLVE-PINNED `run#eff$s0`
           state ref AS-IS, so both branches SHARED the one node — a single-parent-arena orphan re-parented
           onto a dead node → CDZ0101 leaking `run#eff$s0`. `deep_fresh_copy` per branch (an unpinned fresh
           leaf that re-resolves against the spec sig, which declares `$s0`) folds it. run(4,0): only fuel==1
           performs, B.b 1 → 1 (resume hands the op arg back), so acc = 0 + 1 = 1. A shared/stale pin would
           leak `$s0`; a dropped branch-state advance would give a wrong value.")
  (input
    (do
      (effect B (op b (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: x Int64) (: acc Int64)) (if (= x 1) (+ acc (B.b x)) acc))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (B.done acc) (run (- fuel 1) (turn fuel acc))))
      (def (main) (handle B 0 ((b (x) s (resume x x)) (done (x) s (resume x x))) (run 4 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "an effectful helper performing in an if CONDITION folds in a self-call arg"
  (doc
    "The condition-position sibling of the branch case above: the inlined helper's perform sits in the
           `if` CONDITION rather than a branch — `(if (= (B.b x) 1) (+ acc 1) acc)`. When `turn` inlines into
           the self-call arg, the condition perform must fold and re-resolve against the specialized sig, not
           leak an internal `$s0`. run(4,0): only fuel==1's `B.b 1` (resume hands the arg back → 1) makes the
           condition true, so acc = 0 + 1 = 1.")
  (input
    (do
      (effect B (op b (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: x Int64) (: acc Int64)) (if (= (B.b x) 1) (+ acc 1) acc))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (B.done acc) (run (- fuel 1) (turn fuel acc))))
      (def (main) (handle B 0 ((b (x) s (resume x x)) (done (x) s (resume x x))) (run 4 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "an effectful helper performing in a NESTED if branch folds in a self-call arg"
  (doc
    "The nested-conditional sibling: the perform sits in a branch of a NESTED `if` inside the inlined
           helper — `(if (= x 1) (if (= x 1) (+ acc (B.b x)) acc) acc)`. The per-branch fresh-copy must reach
           through the nesting so the inner branch's perform re-resolves cleanly. run(4,0): only fuel==1
           reaches the inner `(B.b 1)` → 1, so acc = 0 + 1 = 1.")
  (input
    (do
      (effect B (op b (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: x Int64) (: acc Int64)) (if (= x 1) (if (= x 1) (+ acc (B.b x)) acc) acc))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (B.done acc) (run (- fuel 1) (turn fuel acc))))
      (def (main) (handle B 0 ((b (x) s (resume x x)) (done (x) s (resume x x))) (run 4 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "an effectful helper whose own parameter name shadows a driver parameter folds in a self-call arg"
  (doc
    "Name-collision edge: the helper's OWN param is named `acc` — the same name as `run`'s driver
           param — and the helper performs on it: `turn(acc) = acc + Tools.dispatch acc`, called `(turn acc)`
           in the self-call arg. The deep-fresh-copy + re-resolve must bind the inlined body's `acc` to the
           helper's param, not leave a stale pin to `run`'s `acc`. With dispatch acc → acc, turn doubles:
           run 3 1 = 2, 4, 8 → done 8. A mis-resolution to the driver's `acc` (or a stale pin) would give a
           wrong value or CDZ0101.")
  (input
    (do
      (effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))
      (def (turn (: acc Int64)) (+ acc (Tools.dispatch acc)))
      (def
        (run (: fuel Int64) (: acc Int64))
        (if (= fuel 0) (Tools.done acc) (run (- fuel 1) (turn acc))))
      (def
        (main)
        (handle Tools 0 ((dispatch (a) s (resume a a)) (done (a) s (resume a a))) (run 3 1)))
      (export main)))
  (output (: 8 Int64)))

(case
  "a state-advancing helper called before a later read threads its write through the continuation"
  (doc
    "A memoized-DB shape (a self-hosting compiler's `demand`): the helper `demand` reads state, and on a
           MISS writes it (`(do (Db.put …) compute)`) before returning; a LATER read in the caller's
           continuation must SEE that write. `demand` inlines into the handle body, and its `None` arm's
           effectful `(Db.put …)` sits inside a `do` under a `match`. Two composed loci made this a silent
           miscompile (→ 99): (1) inlining `demand` collapsed its `(do (Db.put …) compute)` to bare `compute`
           (a `do` resolves to its last form — dropping the effectful intermediate on the substituting inline
           path); (2) even preserved, a branch's state advance was dropped as the conditional's out-state. The
           fix preserves the `do` on inline and re-hoists the exposed conditional to tail position so the
           branch `put` threads. `demand 5 25` misses → writes 5→25 → returns 25; the later `Db.get 5` now
           HITS (Some 25) → 25 + 25 = 50. A drop of the write takes the None arm → 99.")
  (input
    (do
      (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
      (def
        (demand (: k Int64) (: compute Int64))
        (match
          (Db.get k)
          ((Option.Some v) v)
          ((Option.None u) (do (Db.put #tuple(k compute)) compute))))
      (def
        (run-then-get)
        (handle
          Db
          (Map.empty)
          ((get (k) s (resume (Map.lookup s k) s))
            (put (kv) s (match kv (#tuple(k v) (resume unit (Map.insert s k v))))))
          (let
            ((a (demand 5 25)))
            (match (Db.get 5) ((Option.Some v) (+ a v)) ((Option.None u) 99)))))
      (export run-then-get)))
  (output (: 50 Int64)))

(case
  "a RECURSIVE memoizing demand with let-wrapped resumes folds through the group over a state handler"
  (doc
    "The recursive companion of the memoized-DB pins above — a self-hosting compiler's `demand` that
           RECURSES into a node's child, with the on-miss `compute` binding the recursive result in a
           `let` (`(let ((a (demand child))) (let ((b (demand child))) (- (+ a b) b)))`). The get/put/kids
           group threads the state slot through the recursion, and each arm resumes through a `let`-wrapped
           or bare value. `run 3`: `demand 3` misses → `compute 3` → `kids 3` = Some 2 → `a = demand 2`,
           `b = demand 2`, value `(+ a b) - b` = `a`; `demand 2` → Some 1 → likewise = `demand 1`; `demand 1`
           → Some 0 → = `demand 0`; `demand 0` → `kids 0` = None → 0. So the whole chain returns 0 (the
           `+ b - b` cancels at every level, and the leaf is 0). A drift in the group's state-slot threading
           or the `let`-wrapped-resume peel across the recursion would shift the leaf/cancellation.
           STANDALONE (no imports) — exercises the recursive memoize-demand + let-binding fold path the
           existing non-recursive memoize pins don't, and which slices touching `Core::Let.bindings` stress
           (breaker dpd-control-A).")
  (input
    (do
      (effect
        St
        (op get (-> Int64 (Option Int64)))
        (op put (-> (Tuple Int64 Int64) Unit))
        (op kids (-> Int64 (Option Int64))))
      (def
        (demand (: id Int64))
        (match (St.get id) ((Option.Some v) v) ((Option.None _) (cache id (compute id)))))
      (def (cache (: id Int64) (: v Int64)) (match (St.put #tuple(id v)) (_ v)))
      (def
        (compute (: id Int64))
        (match
          (St.kids id)
          ((Option.Some childId)
            (let ((a (demand childId))) (let ((b (demand childId))) (- (+ a b) b))))
          ((Option.None _) id)))
      (def
        (main (: root Int64))
        (handle
          St
          root
          ((get (id) s (resume (Option.None unit) s))
            (put (pair) s (match pair (#tuple(_ _) (resume unit s))))
            (kids (id) s (resume (if (<= id 0) (Option.None unit) (Option.Some (- id 1))) s)))
          (demand root)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 0 Int64)))

(case
  "a helper called TWICE threads its first write so the second call HITS the memoized value"
  (doc
    "The cumulative-loss companion of the single-demand case above — the WIDER witness that a
           state-advancing helper's write survives across MULTIPLE later calls, not just one. `demand`
           is a memoizing `demand`: on a MISS it `(Db.put …)` then returns; on a HIT it returns the
           stored value. `demand 5 25` misses → writes 5↦25 → returns 25. Then `demand 5 999` must HIT
           the FIRST call's put (`Db.get 5` → Some 25) and return 25 — NOT re-miss and recompute 999. So
           `a + b` = 25 + 25 = 50. A drop of the FIRST call's out-state across the SECOND call (the
           cumulative-loss bug the single-demand case cannot catch — it only reads state once) would make
           the second demand miss → recompute 999 → 1024. Pins that the handler state threads through a
           CHAIN of helper calls, each seeing every prior call's writes.")
  (input
    (do
      (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)))
      (def
        (demand (: k Int64) (: compute Int64))
        (match
          (Db.get k)
          ((Option.Some v) v)
          ((Option.None u) (do (Db.put #tuple(k compute)) compute))))
      (def
        (run-twice)
        (handle
          Db
          (Map.empty)
          ((get (k) s (resume (Map.lookup s k) s))
            (put (kv) s (match kv (#tuple(k v) (resume unit (Map.insert s k v))))))
          (let ((a (demand 5 25)) (b (demand 5 999))) (+ a b))))
      (export run-twice)))
  (output (: 50 Int64)))

; An effect operation whose declared RETURN type is a STRUCTURAL RECORD in the ML surface's field
; spelling — each field a `(: name type)` annotation triple, `(op get (-> Unit (Record (: a Int64) (: b
; Int64))))` — must type the PERFORM `(St.get)` at that declared record, so a field of the performed value
; reads. The op's `(meta t)` scheme is read by `type_in_env` (the type-lambda scheme reducer), whose
; `RecordCtor` decode originally accepted only the 2-element `(name type)` pair; the ML `{a: Int64, …}`
; return lowers each field to a 3-element `(: a Int64)` triple, so the record decoded to `None` → the op had
; NO `(-> Unit result)` scheme → the nullary-perform site fell back to the op's META-record and `St.get()`
; typed as `(Record (: apply …) (effect-op …) (t …))` instead of `{a, b}` (CDZ0203 "record has no field a" at
; a consumer). Handling `St` with a `get` arm that resumes `(record (a 1) (b 2))` and reading field `a` = 1
; pins that a structural-record effect-op return threads its declared type to the perform site (the
; `type_in_env` companion of the same `(: name type)` decode fix `typeval_of` carries for variant payloads).
(case
  "an effect op with a structural-record return types the perform at the declared record"
  (input
    (do
      (effect St (op get (-> Unit (Record (: a Int64) (: b Int64)))))
      (def (get-a (: r (Record (: a Int64) (: b Int64)))) r.a)
      (def
        (main)
        (handle
          St
          #record((= a 0) (= b 0))
          ((get (u) s (resume #record((= a 1) (= b 2)) s)))
          (get-a (St.get unit))))
      (export main)))
  (output (: 1 Int64)))

(case
  "one performing closure applied twice observes the handler state stepping between calls"
  (doc
    "An effectful closure defined and applied (twice) directly under its handler: the SAME closure
           value performs `Tick.tick` at both applications, and the handler arm resumes with `(+ v st)`
           while stepping its state by the runtime k — so the two calls through ONE closure see
           DIFFERENT states: f(1)=1+100, then f(2)=2+100+k → 213 at k=10, 203 at k=0. Pins that a
           closure's perform re-enters the CURRENT handler state per application (not a state captured
           at closure creation). The HOF spelling of this — the closure passed to a recursive walker
           applied under the CALLER's handle — rejects by the documented per-callee-param homing
           analysis (:531/:549's soundness twin); this inline spelling is the supported one.")
  (input
    (do
      (effect Tick (op tick (-> Int64 Int64)))
      (def
        (main (: k Int64))
        (handle
          Tick
          100
          ((tick (v) st (resume (+ v st) (+ st k))))
          (do (def f (fn ((: v Int64)) (Tick.tick v))) (+ (f 1) (f 2)))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 213 Int64))
  (call main (: 0 Int64))
  (output (: 203 Int64)))

(case
  "a closure crosses the perform boundary as an operation ARGUMENT and applies in the arm"
  (doc
    "The op's first parameter IS a function — `app : (-> (-> Int64 Int64) Int64 Int64)` — so the
           closure VALUE rides the perform into the handler arm (the :285-family arm applies a
           lexically-visible closure; here it arrives as the operation's PAYLOAD). Two performs hand
           TWO different closures through the same op while the state steps: app(double,1) at st=10 →
           double(11) = 22; app(add-7,1) at st=10+k → 18+k. 2223 at k=5, 2218 at k=0. An op-argument
           marshalling that unified fn payloads by signature (or re-homed the closure to the arm's
           frame losing its identity) answers with the wrong body. Note: an op RESULT typed as a fn
           curried-flattens per arrow right-associativity — `(-> A (-> B C))` reads as a 2-param op,
           so the result-side face is inexpressible today (clean CDZ0201 documents it).")
  (input
    (do
      (effect App (op app (-> (-> Int64 Int64) Int64 Int64)))
      (def
        (main (: k Int64))
        (handle
          App
          10
          ((app (f v) st (resume (f (+ v st)) (+ st k))))
          (+ (* 100 (App.app (fn ((: x Int64)) (* x 2)) 1)) (App.app (fn ((: x Int64)) (+ x 7)) 1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2223 Int64))
  (call main (: 0 Int64))
  (output (: 2218 Int64)))

(case
  "a LIST of closures rides one perform and the arm picks by state per call"
  (doc
    "Effects × collections × fn-values composed: the op payload is a whole `(List (-> Int64
           Int64))`, and the arm indexes it BY THE HANDLER STATE — call 1 (st=0) applies fs[0] =
           (+ x k) at 10 → 10+k, steps the state; call 2 (st=1) applies fs[1] = (* x k) → 10k
           (13030 at k=3; k=0 collapses to 10000 and separates the arms). The heap list of fn
           handles crosses the perform ONCE and is indexed TWICE under different states — a payload
           marshalling that flattened the list to its first element, or re-resolved handles by
           signature, collapses the two calls.")
  (input
    (do
      (effect Pick (op pick (-> (List (-> Int64 Int64)) Int64)))
      (def
        (main (: k Int64))
        (handle
          Pick
          0
          ((pick
              (fs)
              st
              (match (List.at fs st) ((Some f) (resume (f 10) (+ st 1))) ((None _u) (resume -1 st)))))
          (do
            (def
              fs
              (List.push (List.push #list() (fn ((: x Int64)) (+ x k))) (fn ((: x Int64)) (* x k))))
            (+ (* 1000 (Pick.pick fs)) (Pick.pick fs)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 13030 Int64))
  (call main (: 0 Int64))
  (output (: 10000 Int64)))

; A tail-resumptive handler fold MUST keep an arm's local (def x ...) and a perform-site/handle-body
; (def x ...) in their own scopes. Regression: reduce_handle spliced the arm body at the perform site
; WITHOUT alpha-renaming, so an arm-local x captured a same-named free x both directions — silent
; wrong value (F1 arm→body: 10 for 105; F2 body→arm: 14 for 107; both backends, shared reduce_handle).
; Fixed by v-effects 515d6b57d (alpha-rename the local value binders — let pairs + do-local defs — of
; both the handle body and the substituted arm body to fresh #-names). op-param + state binders were
; already hygienic; only arm-internal locals needed the rename. breaker-routed (FINDING #33).
(case
  "handler-arm bindings and perform-site bindings stay in their own scopes across the fold"
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main (: mode Int64))
        (do
          (def x 100)
          (if
            (= mode 1)
            (handle E 0 ((get (u) s (do (def x 5) (resume (+ x s) s)))) (+ x (E.get)))
            (handle E 0 ((get (u) s (resume x s))) (do (def x 7) (+ x (E.get)))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 105 Int64))
  (call main (: 2 Int64))
  (output (: 107 Int64)))

; A handled effect performed via a closure EXTRACTED from a collection (list + List.at + match, applied
; lexically under the handle) DECLINES with the HONEST 'not yet reducible by the tail-resumptive fold'
; message — NOT the misleading 'performed with no enclosing handler here' (there IS one). Reject-don't-
; miscompile-with-honest-message discipline (27-des:5120 class). Fixed by v-effects 1747c764a: the fold
; couldn't trace the app through the collection slot (subtree_performs treated the lambda as pure) →
; standalone lift → no-home arm; now remapped to the honest not-yet-reducible decline. breaker-routed.
(case
  "an effect performed via a collection-extracted closure declines honestly (not-yet-reducible, not a false no-handler claim)"
  (input
    (do
      (effect Ask (op ask (-> Int64 Int64)))
      (def
        (main)
        (handle
          Ask
          5
          ((ask (n) s (resume (* n 2) s)))
          (match (List.at #list((fn (x) (Ask.ask x))) 0) ((Some f) (f 3)) ((None) 0))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

; TWO NESTED handlers, each arm a do-def-local x, the body reading the enclosing FN-LOCAL x through a
; right-nested (+ x (+ (A.geta) (B.getb))) — each binding MUST keep its own scope (no compounding
; leak). Regression arc: pre-hygiene this MISCOMPILED to 43 (the body's x read through the inlined
; arms); v-effects' first hygiene fix (515d6b57d) then made it a FALSE-UNBOUND (CDZ0101) because the
; freshen pass renamed the nested inner arm and orphaned the body's fn-local x; fixed by treating a
; nested handle as OPAQUE in the freshen walk (v-effects 77ffe55b0) — now computes 1033 (1000+11+22).
; The deep companion of the single-handle arm-hygiene pin. breaker #33-nested.
(case
  "nested handlers with colliding arm-local bindings each keep their own scope (no compounding leak)"
  (input
    (do
      (effect A (op geta (-> Unit Int64)))
      (effect B (op getb (-> Unit Int64)))
      (def
        (main (: mode Int64))
        (do
          (def x 1000)
          (handle
            A
            1
            ((geta (u) s (do (def x 10) (resume (+ x s) s))))
            (handle
              B
              2
              ((getb (u) s (do (def x 20) (resume (+ x s) s))))
              (+ x (+ (A.geta) (B.getb)))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1033 Int64)))

(case
  "a performing do-def feeding a bin-construction operand under a handler stays bound (F2)"
  (doc
    "The bin ENCODER was the sole construction-operand position that resolved its operand against
           a scope snapshot taken BEFORE the handler fold rewrote the do-defs (tuple/record/list/Set.of
           all re-resolve after) — so a do-def bound INSIDE the handle body (here `a` from a performed
           `Src.next`) read Unbound at the `bin` operand and the case died CDZ0101 `unbound name a`. Fixed
           by re-resolving the bin operand after the capture-avoiding freshen (v-inference, F2, a4da5beb7).
           The reducible arm resumes the seed unchanged, so `a = Src.next = 10`, `frame = bin(u8 10)`,
           `Bytes.at 0 = 10`. Witnesses the handler-body do-def × bin-operand seam.")
  (input
    (do
      (effect Src (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Src
          10
          ((next (u) s (resume s s)))
          (do
            (def a (Src.next))
            (def frame (bin (u8 (UInt8.wrap a))))
            (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main)
  (output (: 10 Int64)))

(case
  "a perform-free do-def feeding a bin-construction operand under a handler stays bound (F2)"
  (doc
    "The perform-irrelevant twin of the F2 seam: performing-ness was never the trigger — ANY do-def
           bound in a handle body and consumed by a `bin` operand hit the pre-freshen scope snapshot. Here
           `a = (+ 5 1) = 6` with no perform in the def, yet the identical CDZ0101 unbound fired pre-fix.
           `frame = bin(u8 6)`, `Bytes.at 0 = 6`. Pins the discriminator: it is the bin operand under the
           handler-fold rewrite, not the effect.")
  (input
    (do
      (effect Src (op next (-> Unit Int64)))
      (def
        (main)
        (handle
          Src
          10
          ((next (u) s (resume s s)))
          (do
            (def a (+ 5 1))
            (def frame (bin (u8 (UInt8.wrap a))))
            (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1)))))
      (export main)))
  (call main)
  (output (: 6 Int64)))

(case
  "two do-def-bound performs whose sum mixes handler-state width with a narrow param declines cleanly (not-yet-reducible, not an invalid module)"
  (doc
    "SAFE FLOOR (v-effects, F1, 5cf911aeb). Two do-defs each bound to a performed `Src.next` are
           summed under a handler whose arm threads `(+ s x)` — mixing the i64 handler state `s` with the
           narrow UInt8 param `x`. The fold used to emit an INVALID wasm module (`func[0]`, expected i64
           found i32) while rust computed 25; the invalid module was the bug. reduce_handle now DECLINES
           cleanly (codeless `not yet reducible`) rather than emit a malformed artifact — declines-rather-
           than-miscompiles. Computing 25/20 needs a later widening-coercion fold that widens the narrow
           operand to the i64 state carrier; when that lands this flips to a value pin.")
  (input
    (do
      (effect Src (op next (-> Unit Int64)))
      (def
        (main (: x UInt8))
        (handle
          Src
          10
          ((next (u) s (resume s (+ s x))))
          (do (def a (Src.next)) (def b (Src.next)) (+ a b))))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 25 Int64))
  (call main (: 0 UInt8))
  (output (: 20 Int64)))

(case
  "a conditionally-resuming (abortive-or-resume) arm reading the enclosing fn's param folds (conditional-abort/resume-branch lowering)"
  (doc
    "A handler arm that CONDITIONALLY resumes — `(if cond -999 (resume ...))`, one branch aborts with a
           value, the other resumes — reading the enclosing fn's param `k` through the handler seed
           `(tuple 0 k)`. This FOLDS: the E5 reify handles the partially-resuming arm correctly (the abort
           branch yields its value + abandons, the resume branch threads state), and the seed's `k` resolves
           against the grafted chain — no orphaned copy. (Formerly an over-conservative `arm_partially_resumes`
           gate DECLINED this, from an era when the reify rewrote only the resuming branch and orphaned `k` →
           a relocated CDZ0101 at lowering; the reify machinery now lowers a conditional-abort/resume arm, so
           the guard was removed.) n=3: each step reads state via `(. st 0)`/`(. st 1)`; at k=3 the first step
           has `(. st 0)=0 >= (. st 1)=3`? no (0<3) — resumes 0, then 1, then 2 aborts at `2>=3`? no... the
           three steps resume 0,1,2 → `(+ 0 (+ 1 2))=3`; at k=0 the first step `0>=0` aborts -999, abandoning
           the pending `+`. Reclaims clean (live-objects 0 under --guarded-all). Verified reject-safe: were the
           reify ever incomplete, an orphaned free name resolves UNBOUND → Poison → an honest reject, not a
           silent wrong value.")
  (input
    (do
      (effect Sim (op step (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Sim
          #tuple(0 k)
          ((step
              (u)
              st
              (if (>= (. st 0) (. st 1)) -999 (resume (. st 0) #tuple((+ (. st 0) 1) (. st 1))))))
          (+ (Sim.step) (+ (Sim.step) (Sim.step)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (output (: -999 Int64))
  (live-objects 0))

(case
  "handler op-param and state binders stay hygienic when colliding with perform-site names"
  (doc
    "The CLEAN half of the arm-inline hygiene finding (arm-internal do-def/let locals leak;
           these two binder kinds do NOT): the arm's op PARAM `v` shadows a body-side v=1000
           (arm v = the operand 3, resume 3+50; body v intact → 1053) and the STATE binder `s`
           shadows a body-side s=1000 (arm s = the seed 50; body s intact → 1050). The fold
           evidently renames op params and the state binder — pinning that so the arm-LOCAL fix
           extends the SAME treatment rather than regressing these.")
  (input
    (do
      (effect E (op get (-> Int64 Int64)))
      (def
        (main (: mode Int64))
        (if
          (= mode 1)
          (do (def v 1000) (handle E 50 ((get (v) s (resume (+ v s) s))) (+ v (E.get 3))))
          (do (def s 1000) (handle E 50 ((get (u) s (resume s s))) (+ s (E.get 7))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1053 Int64))
  (call main (: 2 Int64))
  (output (: 1050 Int64)))

(case
  "a performing closure duplicated through a generic tuple applies twice with stepping state"
  (doc
    "Effects × the generic DATA position: the performing closure rides `dup` (an unannotated
           generic) into a tuple, and BOTH projections apply under the handler — the homing analysis
           must track the perform through the generic construction + projection (the collection-slot
           spelling is the pinned decline; the GENERIC-TUPLE slot computes because dup inlines/
           monomorphizes into the handler scope). Two applications see stepping state: 100 then
           100+k → 210 at k=10, 200 at k=0. A homing that lost the closure through the generic slot
           would false-reject; a projection that shared ONE application's frame would double-count
           the first state.")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (dup x) #tuple(x x))
      (def
        (main (: k Int64))
        (handle
          E
          100
          ((get (u) s (resume s (+ s k))))
          (do (def p (dup (fn ((: _y Int64)) (E.get)))) (+ ((. p 0) 1) ((. p 1) 2)))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 210 Int64))
  (call main (: 0 Int64))
  (output (: 200 Int64)))

(case
  "a do-def-bound perform inside a recursive fn called under a handle declines cleanly (specializer floor, not a mangled-name CDZ0201)"
  (doc
    "SAFE FLOOR (v-effects, 0d2afb083). A recursive function whose body do-def-binds a performed
           operation — `(do (def scaled (Env.scale i)) (check-all (- i 1) …))` — used to fail CDZ0201
           `check-all#eff2 has no body`: the effect specializer RESERVED a body:None spec def and memoized
           the mangled name before threading the body, and on the do-def-bound-perform body the thread
           returned None (unthreadable) leaving the reserved bodyless def + memo, so the recursive self-call
           resolved to it and leaked the internal `#eff` name. The fix declines UNCODED naming the base fn
           ('the recursive function check-all performs a discharged operation in a form the effect
           specializer does not yet handle') — a clean not-yet-reducible floor, not a mangled CDZ0201.
           Computing the value (110) needs a later body-clone specialization increment. The inline-
           expression twin `(check-all (- i 1) (+ bad (Env.scale i)))` already compiles; this is the
           do-def-bound-perform-in-a-recursive-fn seam, distinct from the straight-line do-def and F1 seams.")
  (input
    (do
      (effect Env (op scale (-> Int64 Int64)))
      (def
        (check-all (: i Int64) (: bad Int64))
        (if (= i 0) bad (do (def scaled (Env.scale i)) (check-all (- i 1) (+ bad scaled)))))
      (def (main (: k Int64)) (handle Env k ((scale (v) s (resume (* v s) s))) (check-all 10 0)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 110 Int64)))

(case
  "Qty arithmetic on the handler state binder via an arm-local def threads and runs"
  (doc
    "The working perimeter of the Qty-stateful-handler pattern (v-cad/notebook @param): a handler
           whose state is a `Qty` (a unit-carrying scalar) advances its state by arithmetic on the state
           binder `s`. The arm-local-def form — `(do (def t (+ s s)) (resume t s))` — type-checks and runs
           (`s` keeps its `(Qty Int64 meter)` type through the def, so `(+ s s)` is Qty+Qty). `main` performs
           once and reads `Qty.value`: `2·21 = 42`. Pins the semantics an INLINE resume-slot `(+ s s)` must
           match (that inline form currently false-rejects — see the flip-pin held for v-inference).")
  (input
    (do
      (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: a Int64))
        (handle
          Acc
          (Qty.of a (Unit.base #"meter"))
          ((step (_u) s (do (def t (+ s s)) (resume t s))))
          (Qty.value (Acc.step))))
      (export main)))
  (call main (: 21 Int64))
  (output (: 42 Int64)))

(case
  "a Qty handler state advances via Qty.value / re-wrap in the next-state slot"
  (doc
    "The value-then-rewrap workaround: the next-state slot advances by unwrapping the Qty to its
           scalar (`Qty.value s`), computing, and re-wrapping (`Qty.of (* … 2) meter`). Two performs read
           `Qty.value` of each and sum: seed 5 → first step advances state to 10, the two performed results
           are 5 and 10 → `Qty.value 5 + Qty.value 10`… (a+2a) reads = 15 at a=5. Pins the re-wrap path as a
           valid Qty-state advance alongside the arm-local-def form.")
  (input
    (do
      (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: a Int64))
        (handle
          Acc
          (Qty.of a (Unit.base #"meter"))
          ((step (_u) s (resume s (Qty.of (* (Qty.value s) 2) (Unit.base #"meter")))))
          (Qty.value (+ (Acc.step) (Acc.step)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 15 Int64)))

(case
  "Qty arithmetic INLINE in a handler resume VALUE slot keeps the state binder's Qty type (#44 fix)"
  (doc
    "The inline resume-slot companion to the arm-local-def perimeter above: `(resume (+ s s) s)`
           resumes with the doubled state VALUE directly, matching `(do (def t (+ s s)) (resume t s))` — so
           `main` reads `Qty.value` of `2·21 = 42`. This inline form used to FALSE-REJECT CDZ0201: the state
           binder `s` was inferred at type `Any` inside the resume-slot `(+ s s)`, so `(+ Any Any)` missed
           the Qty-aware arith arm and defaulted to Int64, then the slot check reported Int64 vs the
           `(Qty Int64 meter)` state type. The fix (v-inference, 520142726) types the state binder from the
           seed via `handle_arm_state_ty`, so the inline arith sees `s : (Qty Int64 meter)` and threads
           correctly. A genuine seed/next-state type mismatch still rejects CDZ0201 — no soundness weakening.")
  (input
    (do
      (effect Acc (op step (-> Unit (Qty Int64 (Unit.base #"meter")))))
      (def
        (main (: a Int64))
        (handle
          Acc
          (Qty.of a (Unit.base #"meter"))
          ((step (_u) s (resume (+ s s) s)))
          (Qty.value (Acc.step))))
      (export main)))
  (call main (: 21 Int64))
  (output (: 42 Int64)))

(case
  "a TWO-site arm over a Qty state gates on the unwrapped magnitude"
  (doc
    "The two-site refold face of the Qty-state family above: the arm's branch condition reads the
           op ARGUMENT (`(> v 10)`), the pass path folds the unwrapped state into its answer and
           advances by re-wrap (`(Qty.of (+ (Qty.value s) 1) (Unit.base #\"meter\"))`), the fail path holds. feed 20 →
           20+5 = 25 (state 6m), feed 3 → 0, feed 30 → 30+6 = 36 → 2536. Pins the served multi-site
           family over a UNIT-CARRYING state — the erased-unit representation must survive the
           refold's continuation rebuild on both branches.")
  (input
    (do
      (effect Acc (op feed (-> Int64 Int64)))
      (def
        (main (: a Int64))
        (handle
          Acc
          (Qty.of a (Unit.base #"meter"))
          ((feed
              (v)
              s
              (if
                (> v 10)
                (resume (+ v (Qty.value s)) (Qty.of (+ (Qty.value s) 1) (Unit.base #"meter")))
                (resume 0 s))))
          (+ (* 100 (Acc.feed 20)) (+ (* 10 (Acc.feed 3)) (Acc.feed 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2536 Int64)))

(case
  "a Float64 handler state advances fractionally through a two-site arm"
  (doc
    "The f64 face of the state-representation matrix: the state walks 0.5 → 0.75 → 1.0 (+0.25
           per pass — dyadic fractions, no rounding ambiguity) while the arm gates on the integer op
           argument. feed 20 → 20, feed 5 → 0, feed 30 → 30 → 2030. Pins that the refold's state
           threading carries an f64 slot through the continuation rebuild.")
  (input
    (do
      (effect St (op feed (-> Int64 Int64)))
      (def
        (main (: a Int64))
        (handle
          St
          0.5
          ((feed (v) s (if (> v 10) (resume v (+ s 0.25)) (resume 0 s))))
          (+ (* 100 (St.feed 20)) (+ (* 10 (St.feed a)) (St.feed 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2030 Int64)))

(case
  "a Float64 op RESULT crosses resume and float state arithmetic is observed by comparison"
  (doc
    "The f64 op-result face: the arm resumes the CURRENT float state and halves it (`(* s 0.5)`),
           so two reads yield 0.5 and 0.25 — all dyadic, exact — and the body observes their sum via
           `(> … 0.7)` → 1. Pins float values crossing the resume boundary and float next-state
           arithmetic, with a comparison consumer (float equality is not the corpus idiom).")
  (input
    (do
      (effect St (op frac (-> Unit Float64)))
      (def
        (main (: a Int64))
        (handle St 0.5 ((frac (u) s (resume s (* s 0.5)))) (if (> (+ (St.frac) (St.frac)) 0.7) 1 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a TUPLE handler state — the arm destructures, branches, and rebuilds both slots"
  (doc
    "The product-state twin-accumulator: `(tuple lo hi)` where the two-site arm (a match around
           the if) routes fails into `lo` (accumulating) and passes into `hi` (counting +1 per pass,
           resumed with `v + hi`); the trailing `sum` reads both. step 20 → 120 (hi 101), step 3 → 0
           (lo 3), sum → 104 → 120 + 0 + 104000 = 104120. Both slots must survive every rebuild —
           a dropped or swapped slot breaks the place-value sum.")
  (input
    (do
      (effect St (op step (-> Int64 Int64)) (op sum (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #tuple(0 100)
          ((step
              (v)
              s
              (match
                s
                (#tuple(lo hi)
                  (if
                    (> v 10)
                    (resume (+ v hi) #tuple(lo (+ hi 1)))
                    (resume lo #tuple((+ lo v) hi))))))
            (sum (u) s (match s (#tuple(lo hi) (resume (+ lo hi) s)))))
          (+ (St.step 20) (+ (St.step n) (* 1000 (St.sum))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 104120 Int64)))

(case
  "a tuple-of-HEAP state — every dispatch grows BOTH components in one rebuild"
  (doc
    "The heap escalation of the tuple state: `(tuple (list) map)` where each `rec` pushes onto
           the List AND inserts into the Map in one rebuild, answering the pre-push length; the
           trailing `stats` reads across both components. rec 7 → 0 ([7], {…,7:14}), rec 5 → 1
           ([7 5], {…,5:10}), stats → 2 + m[7]=14 = 16 → 0 + 1 + 1600 = 1601. The twin-accumulator
           idiom as ONE tuple-valued state (the do-threaded twin-accumulator pins spell it as two
           separate bindings).")
  (input
    (do
      (effect St (op rec (-> Int64 Int64)) (op stats (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #tuple(#list() (Map.insert Map.empty 0 0))
          ((rec
              (v)
              s
              (match
                s
                (#tuple(xs m)
                  (resume (List.len xs) #tuple((List.push xs v) (Map.insert m v (* v 2)))))))
            (stats
              (u)
              s
              (match
                s
                (#tuple(xs m)
                  (resume (+ (List.len xs) (match (Map.lookup m 7) ((Some x) x) ((None _u) 0))) s)))))
          (+ (St.rec 7) (+ (St.rec n) (* 100 (St.stats))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1601 Int64)))

(case
  "a compile-time Char comparison folds beside performs (the runtime-Char boundary's served face)"
  (doc
    "`(String.scalar-at \\\"hello\\\" 1)` with BOTH operands compile-time constants yields
           `(Some #\\e)` at compile time, and the `(= c #\\e)` comparison folds to 1 beside a live
           perform: 5 + 1 = 6. The RUNTIME face is a by-design boundary: a runtime Char has no
           representation yet, so `String.scalar-at` over a runtime string/index rejects (the
           diagnostic names the alternatives — `String.at` for an `(Option String)` one-scalar read,
           `Bytes.at` over `String.to-bytes` for ASCII scans); an effect crossing inherits that
           boundary unchanged.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((bump (u) s (resume s (+ s 1))))
          (+
            (St.bump)
            (match (String.scalar-at "hello" 1) ((Some c) (if (= c #\e) 1 0)) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a TWO-site arm over a BigInt state (heap-scalar state through the refold)"
  (doc
    "The heap-scalar sibling of the Qty two-site pin above: the state is a BigInt (`(BigInt.of
           a)`), advanced with BigInt arithmetic (`(+ s 1N)`) on the pass path and read back through
           `Int64.of` in the resume value. Same walk: 25, 0, 36 → 2536. With the Qty face, pins that
           the refold's state threading is representation-agnostic — boxed heap scalars behave as
           machine ints do.")
  (input
    (do
      (effect Acc (op feed (-> Int64 Int64)))
      (def
        (main (: a Int64))
        (handle
          Acc
          (BigInt.of a)
          ((feed (v) s (if (> v 10) (resume (+ v (Int64.of s)) (+ s 1N)) (resume 0 s))))
          (+ (* 100 (Acc.feed 20)) (+ (* 10 (Acc.feed 3)) (Acc.feed 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2536 Int64)))

(case
  "a two-site arm over a STRING state (concat on pass, hold on fail) with a trailing length reader"
  (doc
    "The string-accumulator idiom through the refold: the pass branch grows the state by
           `String.concat s \\\"x\\\"`, the fail branch holds, and a trailing single-site `len` op reads
           `String.byte-len` (served under the arm-shape rule — trailing single-site after multi-site
           performs). tag 20 → 20 (s \\\"x\\\"), tag 5 → 0, tag 30 → 30 (s \\\"xx\\\"), len → 2 →
           20 + 0 + 30 + 200 = 250. Completes the state-representation matrix's string face.")
  (input
    (do
      (effect St (op tag (-> Int64 Int64)) (op len (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          ""
          ((tag (v) s (if (> v 10) (resume v (String.concat s "x")) (resume 0 s)))
            (len (u) s (resume (String.byte-len s) s)))
          (+ (St.tag 20) (+ (St.tag n) (+ (St.tag 30) (* 100 (St.len)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 250 Int64)))

(case
  "a two-site arm over a BYTES state (bin-built seed, concat growth) with a trailing size reader"
  (doc
    "The binary-accumulator twin of the string face above: the seed is `(bin (u8 0))` (one byte),
           the pass branch appends `(bin (u8 (UInt8.wrap v)))` via `Bytes.concat`, and a trailing
           single-site `size` reads `Bytes.len`. feed 20 → 20 (2 bytes), feed 5 → 0, feed 30 → 30
           (3 bytes), size → 3 → 20 + 0 + 30 + 300 = 350. Composes the bin-construction idiom with
           the refold + the trailing-single-site rule.")
  (input
    (do
      (effect St (op feed (-> Int64 Int64)) (op size (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (bin (u8 0))
          ((feed
              (v)
              s
              (if (> v 10) (resume v (Bytes.concat s (bin (u8 (UInt8.wrap v))))) (resume 0 s)))
            (size (u) s (resume (Bytes.len s) s)))
          (+ (St.feed 20) (+ (St.feed n) (+ (St.feed 30) (* 100 (St.size)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 350 Int64)))

(case
  "a SET state dedup accumulator — the condition PROBES the state, the pass branch inserts"
  (doc
    "Three rule faces in one realistic handler: the branch condition READS the heap state
           (`Set.contains s v` — a membership probe), the pass branch advances it (`Set.insert`), and
           a trailing single-site `card` reads the cardinality. add 7 → new (7, {7}), add 3 → new
           (3, {7 3}), add 7 → DUP (0, held), card → 2 → 7 + 3 + 0 + 200 = 210. The seen-set dedup
           idiom whole; a re-served insert or a stale membership read breaks the checksum.")
  (input
    (do
      (effect St (op add (-> Int64 Int64)) (op card (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #set()
          ((add (v) s (if (Set.contains s v) (resume 0 s) (resume v (Set.insert s v))))
            (card (u) s (resume (Set.len s) s)))
          (+ (St.add 7) (+ (St.add n) (+ (St.add 7) (* 100 (St.card)))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 210 Int64)))

(case
  "a SYMBOL-keyed Map state routes hits to different keys (route-table accumulator)"
  (doc
    "Interned-symbol keys × the refold: the two-site arm routes each hit to a DIFFERENT symbol
           key — passes accumulate under `(Symbol.of \\\"a\\\")`, fails under `(Symbol.of \\\"b\\\")` — and a
           trailing total reads BOTH keys back. hit 20 → a=20, hit 3 → b=3, total → 23 → 20 + 0 +
           2300 = 2320. The symbol lookups must intern to the SAME keys the arm's inserts used across
           dispatches.")
  (input
    (do
      (effect St (op hit (-> Int64 Int64)) (op total (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Map.insert (Map.insert Map.empty (Symbol.of "a") 0) (Symbol.of "b") 0)
          ((hit
              (v)
              s
              (if
                (> v 10)
                (resume v (Map.insert s (Symbol.of "a") v))
                (resume 0 (Map.insert s (Symbol.of "b") v))))
            (total
              (u)
              s
              (resume
                (+
                  (match (Map.lookup s (Symbol.of "a")) ((Some x) x) ((None _u) -1))
                  (match (Map.lookup s (Symbol.of "b")) ((Some y) y) ((None _u) -1)))
                s)))
          (+ (St.hit 20) (+ (St.hit n) (* 100 (St.total))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 2320 Int64)))

(case
  "a NESTED Map-of-Map state — the arm updates the inner map through the outer per dispatch"
  (doc
    "Two-level heap-state rebuild through the fold: every `put` reads the inner map through the
           outer (`Map.lookup s 1`), accumulates into it, and rebuilds BOTH levels (`Map.insert s 1
           (Map.insert inner 2 …)`); the trailing `get` traverses the nesting. inner[2] starts 10:
           put 5 → 15, put 7 → 22, get → 22 → 5 + 7 + 2200 = 2212. Two-level CHAMP persistence per
           dispatch — a dropped rebuild level or a stale inner read breaks the accumulation.")
  (input
    (do
      (effect St (op put (-> Int64 Int64)) (op get (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Map.insert Map.empty 1 (Map.insert Map.empty 2 10))
          ((put
              (v)
              s
              (resume
                v
                (match
                  (Map.lookup s 1)
                  ((Some inner)
                    (Map.insert
                      s
                      1
                      (Map.insert
                        inner
                        2
                        (+ v (match (Map.lookup inner 2) ((Some x) x) ((None _u) 0))))))
                  ((None _u) s))))
            (get
              (u)
              s
              (resume
                (match
                  (Map.lookup s 1)
                  ((Some inner) (match (Map.lookup inner 2) ((Some x) x) ((None _u) -1)))
                  ((None _u) -2))
                s)))
          (+ (St.put n) (+ (St.put 7) (* 100 (St.get))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2212 Int64))
  (live-objects known-leak))

(case
  "triple-nested same-op performs — each argument is the inner perform's result"
  (doc
    "Perform-in-ARGUMENT-position chains freely with a single-site arm: `(St.dbl (St.dbl
           (St.dbl n)))` doubles thrice with the state counting dispatches — 5 → 10 → 20 → 40. (A
           MULTI-site perform in another multi-site perform's argument declines: the argument
           dispatch is inherently mid-chain, the arm-shape mixing rule's interleaved case.)")
  (input
    (do
      (effect St (op dbl (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle St 0 ((dbl (v) s (resume (* v 2) (+ s 1)))) (St.dbl (St.dbl (St.dbl n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 40 Int64)))

(case
  "a multi-site arm's resume value routes through a pure helper call"
  (doc
    "The pass branch's resume VALUE is `(triple v)` — a named pure helper call — rather than an
           inline expression: sift 20 → 60 (s 1), sift 5 → 0, sift 30 → 90 (s 2) → 150. Pins that the
           refold's branch-value rebuild tolerates a function call in the value slot (the helper is
           effect-free; its call folds as opaque pure computation inside the arm).")
  (input
    (do
      (effect St (op sift (-> Int64 Int64)))
      (def (triple (: x Int64)) (* x 3))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((sift (v) s (if (> v 10) (resume (triple v) (+ s 1)) (resume 0 s))))
          (+ (St.sift 20) (+ (St.sift n) (St.sift 30)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 150 Int64)))

(case
  "a RECORD handler state — the arm projects one field and rebuilds the record"
  (doc
    "Record-typed handler state (the state-family pins cover scalar/sum/collection/closure; this
           adds the product): the arm answers with a projection (`(. s count)`) and advances by
           REBUILDING the record with one field bumped and the other carried (`(record (count …+1)
           (tag (. s tag)))`). hit → 5 (count becomes 6), hit → 6 → 56. A dropped or reordered field
           in the rebuild breaks the checksum.")
  (input
    (do
      (effect St (op hit (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #record((= count n) (= tag 7))
          ((hit (_u) s (resume s.count #record((= count (+ s.count 1)) (= tag s.tag)))))
          (+ (* 10 (St.hit)) (St.hit))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "an OPEN-ROW helper projects from the record state INSIDE the arm"
  (doc
    "Row polymorphism under the fold: `get-count` is typed OPEN over extra fields (`(. r count)`
           only), and the arm calls it on the state — a row-poly instantiation happening inside
           handler machinery. Same walk as the direct-projection pin above (56); the helper must
           instantiate at the state's record shape when the arm body is folded, not resolve against
           a stale row.")
  (input
    (do
      (effect St (op hit (-> Unit Int64)))
      (def (get-count r) r.count)
      (def
        (main (: n Int64))
        (handle
          St
          #record((= count n) (= tag 7))
          ((hit (_u) s (resume (get-count s) #record((= count (+ (get-count s) 1)) (= tag 9)))))
          (+ (* 10 (St.hit)) (St.hit))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "a two-site arm gates on a PROJECTED field of the record state (rate limiter)"
  (doc
    "The refold × record-projection composition — the rate-limiter idiom whole: the branch
           condition compares two projected fields (`(< (. s hits) (. s cap))`), the pass path
           rebuilds with hits+1, the fail path answers -1 and holds. cap 2: feed 7 → 7 (hits 1),
           feed 8 → 8 (hits 2), feed 9 → -1 (limit) → 779. The projection in CONDITION position
           and the rebuild across both branches compose with the two-hole refold.")
  (input
    (do
      (effect St (op feed (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #record((= hits 0) (= cap n))
          ((feed
              (v)
              s
              (if
                (< s.hits s.cap)
                (resume v #record((= hits (+ s.hits 1)) (= cap s.cap)))
                (resume -1 s))))
          (+ (* 100 (St.feed 7)) (+ (* 10 (St.feed 8)) (St.feed 9)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 779 Int64))
  (live-objects known-leak))

(case
  "a two-site arm branches on SYMBOL equality of the state"
  (doc
    "The interned-symbol face of the served multi-site family: the condition is `(= s (Symbol.of
           \"loud\"))` — an O(1) symbol identity check against the state binder. Both reads take the
           loud path at seed \"loud\": 500 + 300 = 800. Extends the refold's condition coverage to
           Symbol-typed states (the mode-dispatch handler idiom's read half).")
  (input
    (do
      (effect St (op emit (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Symbol.of "loud")
          ((emit (v) s (if (= s (Symbol.of "loud")) (resume (* v 100) s) (resume v s))))
          (+ (St.emit n) (St.emit 3))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 800 Int64)))

(case
  "a mode-REPLACING arm swaps the Symbol state; a conditional-value arm reads it"
  (doc
    "The write half of the mode-dispatch idiom: `flip` REPLACES the Symbol state (`loud` →
           `quiet`) while `emit` answers conditionally on it (single resume site — the branch is in
           the VALUE, not around the resume). emit 5 loud → 500, flip → 0 (mode quiet), emit 3
           quiet → 3 → 503. Pins a symbol-valued state transition observed by a later dispatch.
           (The two-site-branch × mode-replacing composition in ONE handler still declines — the
           open second-op family.)")
  (input
    (do
      (effect St (op emit (-> Int64 Int64)) (op flip (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Symbol.of "loud")
          ((emit (v) s (resume (if (= s (Symbol.of "loud")) (* v 100) v) s))
            (flip (u) s (resume 0 (Symbol.of "quiet"))))
          (+ (St.emit n) (+ (St.flip) (St.emit 3)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 503 Int64)))

; --- Effects micro-family: positional op-arg binding, per-slot ctor-element ordering, the SET
; constructor's dedup-of-results, stateful nested-handler isolation, and a growing Bytes-rope
; state. Each is the ASYMMETRIC/stateful sibling of an existing symmetric/stateless pin.
(case
  "a 3-arg effect op binds its operands POSITIONALLY — an argument-order swap is caught"
  (doc
    "The positional sibling of the commutative add3 pin: the arm encodes 100x+10y+z so ANY operand permutation diverges (add3's a+b+c passes under a swap); runtime a in the second perform + stepping state.")
  (input
    (do
      (effect Calc (op mix (-> Int64 Int64 Int64 Int64)))
      (def
        (main (: a Int64))
        (handle
          Calc
          1000
          ((mix (x y z) s (resume (+ (* 100 x) (+ (* 10 y) (+ z s))) (+ s 1))))
          (+ (Calc.mix 1 2 3) (Calc.mix a 5 6))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 2580 Int64)))

(case
  "performs as list-literal elements land at their POSITIONS in perform order"
  (doc
    "The per-slot sibling of the sum-read list-ctor pin: xs[0] and xs[2] read INDIVIDUALLY (positional weights) + a post-build tick proves state continuity — ticks k,k+1,k+2 land at slots 0,1,2; a right-to-left fill or shared temp diverges. The handler stays live AROUND the reads.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Ctr
          k
          ((tick (_u) s (resume s (+ s 1))))
          (do
            (def xs #list((Ctr.tick) (Ctr.tick) (Ctr.tick)))
            (+
              (* 100 (match (List.at xs 0) ((Option.Some v) v) ((Option.None _u) -1)))
              (+ (* 10 (match (List.at xs 2) ((Option.Some v) v) ((Option.None _u) -1))) (Ctr.tick))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 578 Int64)))

(case
  "performs as Set.of elements build the set with stepping state and dedup applies to the RESULTS"
  (doc
    "SET completes the compound-constructor perform-threading family (tuple/list/record/map) and adds what none have: the ctor DEDUPS its element results — CHAMP hash on resumed values. Stepping arm (+2): 3 distinct {k,k+2,k+4}, len 3 + membership.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Ctr
          k
          ((tick (_u) s (resume s (+ s 2))))
          (do
            (def s #set((Ctr.tick) (Ctr.tick) (Ctr.tick)))
            (+
              (* 100 (Set.len s))
              (+ (* 10 (if (Set.contains s k) 1 0)) (if (Set.contains s (+ k 4)) 1 0))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 311 Int64)))

(case
  "a STALLED counter makes all Set.of perform results collide to a singleton"
  (doc
    "The collide face: (resume s s) stalls the state so all three performs return k — the set must collapse to a singleton (a builder assuming distinct element slots miscounts).")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Ctr
          k
          ((tick (_u) s (resume s s)))
          (do
            (def s #set((Ctr.tick) (Ctr.tick) (Ctr.tick)))
            (+ (* 10 (Set.len s)) (if (Set.contains s k) 1 0)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

(case
  "nested SAME-effect handlers isolate STATE — inner performs never advance the outer counter"
  (doc
    "The STATEFUL sibling of the stateless region-partition pin: outer +1 / inner +2 strides, reads BEFORE/INSIDE/AFTER the inner region. The after-read is load-bearing: outer resumes at its own single advance (101), not advanced by inner performs (103/105) nor reset by inner teardown (100). Runtime inner seed.")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          E
          100
          ((get (_u) s (resume s (+ s 1))))
          (+
            (E.get)
            (+ (handle E (* k 10) ((get (_u) s (resume s (+ s 2)))) (+ (E.get) (E.get))) (E.get)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 303 Int64)))

(case
  "a Bytes ROPE handler state GROWS by bin-append per perform and each resume reads the prior length"
  (doc
    "BYTES joins the handler-state type family (scalar/tuple/record/Map/Set): the wire-accumulator idiom — each put APPENDS (bin (u8 v)) via Bytes.concat (deeper rope per perform), resume value = PRIOR length (1,2,3 -> 123). The state rope must survive perform round-trips with its seam structure intact.")
  (input
    (do
      (effect Acc (op put (-> UInt8 Int64)))
      (def
        (main (: a Int64) (: b Int64))
        (handle
          Acc
          (Bytes.of #list(9))
          ((put (v) s (resume (Bytes.len s) (Bytes.concat s (bin (u8 v))))))
          (do
            (def l1 (Acc.put (UInt8.wrap a)))
            (def l2 (Acc.put (UInt8.wrap b)))
            (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 123 Int64)))

(case
  "an Ast.List handler STATE accumulates a node per perform and each resume reads the prior length"
  (doc
    "AST joins the handler-state type family (scalar/tuple/record/Map/Set/Bytes-rope): the
           template-accumulator idiom — each put pushes `(Ast.Int (BigInt.of v))` onto the `Ast.List`
           state's element list (rebuilt via the Ast.List ctor, matched back open per perform), resume
           value = the PRIOR List.len (0,1,2 -> 12). A recursive-sum state with BigInt-boxed leaves must
           survive the perform round-trips exactly as the flat state shapes do.")
  (input
    (do
      (effect Acc (op put (-> Int64 Int64)))
      (def
        (main (: a Int64) (: b Int64))
        (handle
          Acc
          (Ast.List #list())
          ((put
              (v)
              s
              (match
                s
                ((Ast.List els)
                  (resume (List.len els) (Ast.List (List.push els (Ast.Int (BigInt.of v))))))
                (_ (resume -100 s)))))
          (do (def l1 (Acc.put a)) (def l2 (Acc.put b)) (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
      (export main)))
  (call main (: 1 Int64) (: 2 Int64))
  (output (: 12 Int64)))

(case
  "an OPTION handler state TOGGLES its variant per perform"
  (doc
    "A sum-typed state whose VARIANT changes per dispatch (the state-family pins hold their variant
           fixed): the arm matches its own state and flips it — `Some v` resumes v and stores None; `None`
           resumes -1 and stores `Some 99`. Three performs walk Some 7 → None → Some 99, and the place-value
           checksum (100·7 + 10·(−1) + 99 = 789) breaks if any transition writes the wrong variant or a
           stale payload. The state slot must carry a full sum value whose constructor differs call-to-call.")
  (input
    (do
      (effect St (op tog (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Option.Some n)
          ((tog
              (u)
              s
              (match
                s
                ((Option.Some v) (resume v (Option.None)))
                ((Option.None) (resume -1 (Option.Some 99))))))
          (+ (* 100 (St.tog)) (+ (* 10 (St.tog)) (St.tog)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 789 Int64)))

(case
  "an Option-of-HEAP handler state transitions None to Some and grows the payload"
  (doc
    "The heap composition of the variant-transitioning state: `(Option (List Int64))` starts None;
           the first feed creates `Some (list v)`, later feeds push into the existing payload, and each
           resume reports the PRIOR length (0, 1, 2 → 12). The transition allocates the list inside the
           arm on the None path and grows it on the Some path — a sum-wrapped heap payload whose variant
           AND contents both evolve across performs.")
  (input
    (do
      (effect St (op feed (-> Int64 Int64)))
      (def
        (main (: a Int64))
        (handle
          St
          (Option.None)
          ((feed
              (v)
              s
              (match
                s
                ((Option.None) (resume 0 (Option.Some #list(v))))
                ((Option.Some xs) (resume (List.len xs) (Option.Some (List.push xs v)))))))
          (+ (* 100 (St.feed a)) (+ (* 10 (St.feed (+ a 1))) (St.feed (+ a 2))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 12 Int64)))

(case
  "a RESULT handler state is matched per variant with one resume per arm (Ok accumulates, Err echoes)"
  (doc
    "The Result sibling of the Option variant pins above: the state is `(Result Int64 Int64)` and the
           arm matches it — the Ok path accumulates `(resume (+ acc v) (Ok (+ acc v)))`, the Err path
           echoes its payload unchanged. Each match ARM has exactly ONE resume site, so the shape folds
           (the latching Ok→Err transition, whose if branches on the accumulator READ FROM THE STATE
           binder inside one arm, is the pinned condition-reads-state decline). This run stays on the Ok
           path: 3, 3+4=7, 7+2=9 → 379. Pins per-variant dispatch over a two-payload sum state where both
           constructors carry data (Option's None carries none).")
  (input
    (do
      (effect St (op add (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (Result.Ok 0)
          ((add
              (v)
              s
              (match
                s
                ((Result.Ok acc) (resume (+ acc v) (Result.Ok (+ acc v))))
                ((Result.Err e) (resume e (Result.Err e))))))
          (+ (* 100 (St.add n)) (+ (* 10 (St.add 4)) (St.add 2)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 379 Int64)))

(case
  "an Ast node as the effect OP ARGUMENT is destructured by the arm"
  (doc
    "The op-ARGUMENT direction of the Ast crossing (the resume-value case above is the arm→body
           direction; this is body→arm): the program performs `(Sink.eat (Ast.List …))` and the ARM
           pattern-matches the node, resuming with its element count — a 2-element list then an empty
           one (2 + 0 = 2). The op-arg marshal must carry the recursive sum into the arm intact, the
           analyzer-handler idiom (a handler that inspects syntax it is handed).")
  (input
    (do
      (effect Sink (op eat (-> Ast Int64)))
      (def
        (main (: n Int64))
        (handle
          Sink
          0
          ((eat (a) s (match a ((Ast.List els) (resume (List.len els) s)) (_ (resume -1 s)))))
          (+
            (Sink.eat (Ast.List #list((Ast.Int (BigInt.of n)) (Ast.Name "x"))))
            (Sink.eat (Ast.List #list())))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 2 Int64)))

(case
  "a FOUR-arg effect op binds positionally (place-value checksum)"
  (doc
    "The arity extension of the 3-arg positional pin: four operands at four place values —
           `(Calc.mix4 5 2 3 4)` → 1000·5 + 100·2 + 10·3 + 4 = 5234. Any operand permutation or
           marshal-slot mixup at arity 4 diverges.")
  (input
    (do
      (effect Calc (op mix4 (-> Int64 Int64 Int64 Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Calc
          0
          ((mix4 (a b c d) s (resume (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d))) s)))
          (Calc.mix4 n 2 3 4)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5234 Int64)))

(case
  "a HETEROGENEOUS 4-arg op (Int64/String/Bool/Int64) marshals every type to its arm binder"
  (doc
    "The mixed-signature face of the op-arg marshal (the positional pins are homogeneous-Int):
           one op carries a scalar id, a heap String, a Bool flag, and a scalar score, and the arm
           consumes each per its type — id scaled (500), name measured (3), flag branched (1000),
           score added (7) → 1510. Real host-effect signatures are exactly this shape.")
  (input
    (do
      (effect Rec (op entry (-> Int64 String Bool Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Rec
          0
          ((entry
              (id name flag score)
              s
              (resume (+ (* 100 id) (+ (String.byte-len name) (+ (if flag 1000 0) score))) s)))
          (Rec.entry n "abc" true 7)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1510 Int64)))

(case
  "a RECORD op result crosses resume; the body projects both fields"
  (doc
    "A STRUCTURAL record in the op signature (records-as-STATE are pinned; the crossing was not —
           structural products marshal differently from nominal sums and positional tuples): the arm
           resumes `(record (x (* id 2)) (y (+ id 1)))` and the body projects both fields — 10 + 6 =
           16. The field layout must survive the resume marshal.")
  (input
    (do
      (effect St (op fetch (-> Int64 (Record (: x Int64) (: y Int64)))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((fetch (id) s (resume #record((= x (* id 2)) (= y (+ id 1))) s)))
          (let ((r (St.fetch n))) (+ r.x r.y))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64)))

(case
  "a RECORD as op ARGUMENT — the arm projects the fields it is handed"
  (doc
    "The argument direction of the record crossing: the body hands `(record (hits n) (misses 3))`
           to the op and the ARM projects both fields — 10·5 − 3 = 47. With the result-direction pin
           above and the record-STATE pins, structural records cover all three effect positions.")
  (input
    (do
      (effect St (op score (-> (Record (: hits Int64) (: misses Int64)) Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((score (r) s (resume (- (* r.hits 10) r.misses) s)))
          (St.score #record((= hits n) (= misses 3)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 47 Int64)))

(case
  "a record is built and consumed inside the arm (structural product per dispatch)"
  (doc
    "The arm-internal face: the record never crosses the boundary — the arm builds it from the
           op argument, binds it via a match, and resumes the projected sum (10 + 6 = 16). Pins
           structural-product construction + projection inside folded arm bodies.")
  (input
    (do
      (effect St (op fetch (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((fetch (id) s (resume (match #record((= x (* id 2)) (= y (+ id 1))) (r (+ r.x r.y))) s)))
          (St.fetch n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64)))

(case
  "a heterogeneous TUPLE op result (String, Int64) crosses resume and destructures"
  (doc
    "The result-direction twin of the heterogeneous-args pin above: the arm resumes a
           `(Tuple String Int64)` — a heap String and a scalar in one payload — and the body
           destructures it: byte-len \\\"row\\\" + 5·10 = 53. Both marshal directions now carry
           mixed-type payloads.")
  (input
    (do
      (effect Rec (op fetch (-> Int64 (Tuple String Int64))))
      (def
        (main (: n Int64))
        (handle
          Rec
          0
          ((fetch (id) s (resume #tuple("row" (* id 10)) (+ s 1))))
          (match (Rec.fetch n) (#tuple(name score) (+ (String.byte-len name) score)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 53 Int64)))

(case
  "a USER-SUM op result (Status) crosses resume; the body matches per variant"
  (doc
    "User-DECLARED sums through the effect boundary (Option/Result crossings are pinned; nominal
           sums go through the general type marshal, not the built-in paths): the op's result type is
           `Status` (a payload variant + an empty one), the arm resumes either, and the body matches —
           poll 20 → Active 40, poll 5 → Idle → -1 → 39. The marshal must carry the nominal tag and
           payload across resume.")
  (input
    (do
      (effect St (op poll (-> Int64 Status)))
      (type Status (Active Int64) (Idle))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((poll (v) s (resume (if (> v 10) (Status.Active (* v 2)) (Status.Idle)) (+ s 1))))
          (+
            (match (St.poll 20) ((Status.Active x) x) ((Status.Idle) -1))
            (match (St.poll n) ((Status.Active x) x) ((Status.Idle) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 39 Int64)))

(case
  "a user SUM is constructed AND matched inside the arm (per-dispatch classification)"
  (doc
    "The arm-internal face: the sum never crosses the boundary — the arm builds a `Status` from
           the op argument, matches it immediately, and resumes the scalar classification (20 pass →
           20, 5 fail → 0 → 20). Pins nominal-sum construction + dispatch working inside folded arm
           bodies.")
  (input
    (do
      (effect St (op classify (-> Int64 Int64)))
      (type Status (Active Int64) (Idle))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((classify
              (v)
              s
              (resume
                (match
                  (if (> v 10) (Status.Active v) (Status.Idle))
                  ((Status.Active x) x)
                  ((Status.Idle) 0))
                s)))
          (+ (St.classify 20) (St.classify n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64)))

(case
  "a GENERIC user sum ((Box Int64)) as op result — nominal tag + instantiated payload cross resume"
  (doc
    "The generic extension of the monomorphic Status crossing above: `(Box a)` instantiated at
           Int64 — wrap 20 → Full 60, wrap 5 → Empty → -1 → 59. The instantiated payload slot and the
           nominal tag both survive the resume marshal.")
  (input
    (do
      (effect St (op wrap (-> Int64 (Box Int64))))
      (type (Box a) (Full a) (Empty))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((wrap (v) s (resume (if (> v 10) (Box.Full (* v 3)) (Box.Empty)) (+ s 1))))
          (+
            (match (St.wrap 20) ((Box.Full x) x) ((Box.Empty) -1))
            (match (St.wrap n) ((Box.Full x) x) ((Box.Empty) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 59 Int64)))

(case
  "a generic sum instantiated at a HEAP payload ((Box (List Int64))) crosses resume"
  (doc
    "The heap-instantiation face: the generic payload slot holds a LIST — grab 20 → Full [20 20
           20] (len 3), grab 5 → Empty → -1 → 2. Instantiation-specific layout (a heap pointer in the
           payload slot) through the resume marshal.")
  (input
    (do
      (effect St (op grab (-> Int64 (Box (List Int64)))))
      (type (Box a) (Full a) (Empty))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grab (v) s (resume (if (> v 10) (Box.Full #list(v v v)) (Box.Empty)) s)))
          (+
            (match (St.grab 20) ((Box.Full xs) (List.len xs)) ((Box.Empty) -1))
            (match (St.grab n) ((Box.Full xs) (List.len xs)) ((Box.Empty) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64)))

(case
  "a RECURSIVE user sum (Tree) crosses resume; the body folds it"
  (doc
    "A user-declared RECURSIVE sum (payloads contain the sum itself) as the op result: the arm
           builds a 3-leaf tree from the op argument and the body folds it with a recursive helper —
           Node(Leaf 5, Node(Leaf 10, Leaf 1)) → 16. Distinct from the built-in Ast crossings: a user
           recursive type goes through the general nominal marshal.")
  (input
    (do
      (effect St (op grow (-> Int64 Tree)))
      (type Tree (Leaf Int64) (Node Tree Tree))
      (def (sum-tree t) (match t ((Tree.Leaf v) v) ((Tree.Node l r) (+ (sum-tree l) (sum-tree r)))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((grow
              (v)
              s
              (resume (Tree.Node (Tree.Leaf v) (Tree.Node (Tree.Leaf (* v 2)) (Tree.Leaf 1))) s)))
          (sum-tree (St.grow n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64))
  (live-objects known-leak))

(case
  "a recursive sum as op ARGUMENT — the arm dispatches on its shape"
  (doc
    "The argument direction: the body hands trees to the op and the ARM pattern-dispatches on
           the shape it receives — a Leaf answers its payload (5), a Node answers 99 → 104. The op-arg
           marshal carries the recursive structure into the arm intact.")
  (input
    (do
      (effect St (op weigh (-> Tree Int64)))
      (type Tree (Leaf Int64) (Node Tree Tree))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((weigh (t) s (resume (match t ((Tree.Leaf v) v) ((Tree.Node l r) 99)) s)))
          (+ (St.weigh (Tree.Leaf n)) (St.weigh (Tree.Node (Tree.Leaf 1) (Tree.Leaf 2))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 104 Int64)))

(case
  "a USER sum as handler state — a countdown mode machine (Fast k -> Slow)"
  (doc
    "The state-slot completion of the user-sum ladder (Option/Result STATE pins exist; a
           user-declared sum state did not): `Mode` starts `Fast n`, the arm decrements the payload
           per dispatch and TRANSITIONS variants at zero — Fast 2 → Fast 1 → Fast 0 → Slow, resuming
           2, 1, 0 → 210. Nominal-sum layout in the state slot, with a variant transition mid-run.")
  (input
    (do
      (effect St (op step (-> Unit Int64)))
      (type Mode (Fast Int64) (Slow))
      (def
        (main (: n Int64))
        (handle
          St
          (Mode.Fast n)
          ((step
              (u)
              s
              (match
                s
                ((Mode.Fast k) (if (> k 0) (resume k (Mode.Fast (- k 1))) (resume 0 (Mode.Slow))))
                ((Mode.Slow) (resume -1 (Mode.Slow))))))
          (+ (* 100 (St.step)) (+ (* 10 (St.step)) (St.step)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 210 Int64)))

(case
  "three DISCARDED performs on a do-spine still advance the state"
  (doc
    "Effect-only evaluation: three `(St.bump)` results are discarded on the do-spine — evaluated
           purely for their state effect — and the trailing peek reads the fully-advanced 8 (seed 5,
           three advances). A fold that elided 'unused' performs would skip the advances and read 5.
           The most imperative idiom in the language, pinned standalone.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)) (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((bump (u) s (resume s (+ s 1))) (peek (u) s (resume s s)))
          (do (St.bump) (St.bump) (St.bump) (St.peek))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 8 Int64)))

(case
  "NEGATIVE values thread every effect slot — state, argument, and result stay signed"
  (doc
    "A sign-extension slip in any marshal (i64 truncation, a wrong-width reload) surfaces only
           on negative values; this case drives negatives through EVERY slot at once so each marshal
           path has a signed witness: seed −100, op arg −5, resume values −105/−107, next-state
           arithmetic −110 → −212. The signed-values face of the effect machinery.")
  (input
    (do
      (effect St (op dip (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle St -100 ((dip (v) s (resume (+ v s) (- s 10)))) (+ (St.dip (- 0 n)) (St.dip 3))))
      (export main)))
  (call main (: 5 Int64))
  (output (: -212 Int64)))

(case
  "Int64 MAX threads the handler state intact (representation at the boundary)"
  (doc
    "The state slot must carry a full i64: the seed is Int64 MAX, the first peek reads it back
           EXACTLY, the state decrements, and the second peek reads MAX−1 → 1. Any narrower
           intermediate representation (or a float round-trip) corrupts the boundary value.")
  (input
    (do
      (effect St (op peek (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          9223372036854775807
          ((peek (u) s (resume s (- s 1))))
          (if (= (St.peek) 9223372036854775807) (if (= (St.peek) 9223372036854775806) 1 2) 3)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "ZERO threads every effect slot (zero seed, zero args, zero results)"
  (doc
    "The degenerate-value face: a zero state seed, a zero LITERAL argument, a zero COMPUTED
           argument (`(- n n)`), and zero resume values — all thread and the +7 tail lands the
           checksum. Zeros matter because a wrong slot read aliases with an uninitialized cell; a
           positive checksum cannot distinguish 0-the-value from 0-the-missing-write.")
  (input
    (do
      (effect St (op echo (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle St 0 ((echo (v) s (resume (+ v s) s))) (+ (St.echo 0) (+ (St.echo (- n n)) 7))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64)))

(case
  "a handle whose body NEVER performs is exactly its body (zero dispatches)"
  (doc
    "The zero-dispatch degenerate: the effect is declared, the handler installed with a live
           arm — and the body never performs, so the handle is exactly `(* n 2)` = 10. The fold's
           fully-eliminated path: the handler apparatus must vanish without residue (no stray seed
           evaluation effects, no frame cost observable in the value).")
  (input
    (do
      (effect St (op never (-> Unit Int64)))
      (def (main (: n Int64)) (handle St 100 ((never (u) s (resume s s))) (* n 2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a pure closure-driver call beside a perform in one handle body"
  (doc
    "A generic driver (`apply-twice`, a lambda-lifted closure call on every backend, incl. the
           async EnvClosure emit) runs BESIDE a perform in one handle body: the driver computes
           10 + 12 = 22 purely, the bump reads 100 → 122. The closure machinery and the effect fold
           coexist in one body on all three targets.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (def (apply-twice f (: a Int64)) (+ (f a) (f (+ a 1))))
      (def
        (main (: n Int64))
        (handle
          St
          100
          ((bump (u) s (resume s (+ s 1))))
          (+ (apply-twice (fn ((: x Int64)) (* x 2)) n) (St.bump))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 122 Int64)))

(case
  "a closure-driver result feeds a perform's ARGUMENT"
  (doc
    "The dataflow composition: the driver's computed 22 flows INTO the effect dispatch as the
           op argument, and the arm scales it (220). The closure-call result must be fully reduced
           before the dispatch marshals it.")
  (input
    (do
      (effect St (op log (-> Int64 Int64)))
      (def (apply-twice f (: a Int64)) (+ (f a) (f (+ a 1))))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((log (v) s (resume (* v 10) (+ s 1))))
          (St.log (apply-twice (fn ((: x Int64)) (* x 2)) n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 220 Int64)))

(case
  "a bin PATTERN destructures a perform-result Bytes (the wire round-trip through a handler)"
  (doc
    "binary-matching × effects, the codec round-trip: the ARM constructs framed Bytes from its
           state (`(bin (u16 …) (u8 7))`) and the BODY destructures the perform result with a bin
           PATTERN, recovering both fields — hi = 258 (the seed, big-endian u16), lo = 7 → 258 + 700 =
           958. The protocol-handler idiom: a handler serves wire bytes, the caller parses them; the
           pattern must read exactly the bytes the arm's construction laid down.")
  (input
    (do
      (effect St (op fetch (-> Unit Bytes)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((fetch (u) s (resume (bin (u16 (UInt16.wrap s)) (u8 7)) (+ s 1))))
          (match (St.fetch) ((bin (u16 hi) (u8 lo)) (+ (Int64.of hi) (* 100 (Int64.of lo)))) (_ -1))))
      (export main)))
  (call main (: 258 Int64))
  (output (: 958 Int64)))

(case
  "a bin-pattern arm binds a parsed byte and PERFORMS again with it (parse-then-act)"
  (doc
    "The pipeline composition of the bin-pattern crossing above: the match arm's binder `b` —
           established by the bin PATTERN over the perform result — feeds a SECOND perform
           (`(St.log (Int64.of b))`), whose arm multiplies by 10: fetch serves byte 5, log answers
           50. Pins that a bin-pattern binding flows into a subsequent dispatch correctly — the
           parse-then-act shape every wire-protocol reducer uses.")
  (input
    (do
      (effect St (op fetch (-> Unit Bytes)) (op log (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((fetch (u) s (resume (bin (u8 (UInt8.wrap s))) (+ s 1))) (log (v) s (resume (* v 10) s)))
          (match (St.fetch) ((bin (u8 b)) (St.log (Int64.of b))) (_ -1))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64)))

(case
  "eval of a quoted expression folds INSIDE a handle body beside performs"
  (doc
    "quote/eval × effects coexistence: `(eval (quote (+ 1 2)))` — a COMPILE-TIME eval of a
           compile-time-visible quote — sits between two performs and folds to its 3 while the
           performs discharge normally: 5 + 3 + 6 = 14. Both features rewrite the handle body (the
           eval reconstructs-and-compiles, the fold discharges performs); this pins that they
           compose. (An arm-built RUNTIME Ast fed to eval is rejected by design with CDZ0101, whose
           message begins: `eval` executes only a COMPILE-TIME-VISIBLE AST construction (a
           `(quote …)` or literal `Ast.*`): it reconstructs the source that AST denotes and compiles
           it. — the compiler builds and analyzes AST but does not run a dynamically-built one.)")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (+ (St.next) (+ (eval (quote (+ 1 2))) (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 14 Int64)))

(case
  "a @requires-guarded def PERFORMS in its body — contract check and effect specialization compose"
  (doc
    "@requires × effects: the enforcement rewrite injects `(if (>= x 0) BODY (trap …))` at
           body-entry AND the body performs `(St.bump)`, so the def is both contract-checked and
           effect-specialized. Two satisfying calls observe the advancing state: f 5 → 5+100 = 105
           (s → 101), f 2 → 2+101 = 103 → 208. The two body rewrites (contract if-wrap, effect
           specialization) must not fight over the same def.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.bump))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (+ (f n) (f 2))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 208 Int64)))

(case
  "a VIOLATED @requires traps at body-entry BEFORE the body's perform fires"
  (doc
    "The enforcement-order guarantee of the pair, made OBSERVABLE by an ABORTIVE arm: `(f -5)`
           violates `(>= x 0)`, and the handler's `bump` arm never resumes — it ABORTS the handle with
           999. So if the rewrite order were wrong (perform first, check second), `(St.bump)` would
           dispatch, the abort would win, and the program would RETURN 999 instead of trapping — this
           case would fail its trap expectation. The trap firing proves the injected check runs at
           body-entry, before the perform. (A resuming arm could not distinguish the two orders —
           both end in the same trap.)")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.bump))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s 999)) (f (- 0 n))))
      (export main)))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "a satisfied @ensures on a performing def passes the effectful result through"
  (doc
    "The postcondition side of the contract × effects pair (the @requires pins are above): the
           `@ensures (>= ret 100)` wrapper checks the EFFECT-DERIVED result — f 5 = 5 + bump(100) =
           105, satisfying — and passes it through unchanged (105). Single call; the multi-call face
           is the open let-perform × branching-condition fold bug tracked separately.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (f n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "a VIOLATED @ensures on a performing def traps at body-exit"
  (doc
    "The violated face: `(>= ret 1000)` fails against the effect-derived 105, so the injected
           body-exit check traps — postcondition enforcement works when the result came through a
           resume rather than pure arithmetic.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (ensures (>= ret 1000)) (def (f (: x Int64)) (+ x (St.bump))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (f n)))
      (export main)))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "a STACKED @requires + @ensures contract on a performing def threads all three layers"
  (doc
    "The full Hoare triple × effects: precondition check (satisfied), effectful body (the bump
           resumes 100), postcondition check (105 >= 100, satisfied) — pre + perform + post all thread
           and the contract-checked effectful result returns (105).")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (requires (>= x 0)) (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump)))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (f n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "the stacked contract's PRE fires through the @ensures layer BEFORE the perform (abortive observer)"
  (doc
    "Composes the descent-through-annotation-layers guarantee (the @requires reaches the def
           through the intervening @ensures wrapper) with OBSERVABLE check-before-perform ordering:
           the bump arm ABORTS with 999, so if the perform ran before the (violated) precondition
           check, the program would return 999 — the trap proves the pre fires first, through the
           stack.")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ (requires (>= x 0)) (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump)))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s 999)) (f (- 0 n))))
      (export main)))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "a @test-tier @ensures on a performing def runs and checks"
  (doc
    "The three-layer annotation stack — `@test` → `@ensures` → a performing def — threads: the
           test-tier postcondition checks the effect-derived 105 and passes it through. Completes the
           annotation-tier crossings with effects (plain and @test-tier contracts both compose).")
  (input
    (do
      (effect St (op bump (-> Unit Int64)))
      (@ test (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump)))))
      (def (main (: n Int64)) (handle St 100 ((bump (u) s (resume s (+ s 1)))) (f n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 105 Int64)))

(case
  "an @invariant newtype is constructed from PERFORM results"
  (doc
    "The invariant-type × effects cross: a `Percent` (0..100 @invariant) built from two perform
           results at advancing state — mk(42) and mk(43), both satisfying — unwrapped and summed
           (85). The invariant machinery (the synthesized checker) and the effect fold compose when
           the checked value originates from a handler.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
      (def (mk (: v Int64)) (Percent.Pct v))
      (def (unwrap (: p Percent)) (match p ((Percent.Pct n) n)))
      (def
        (main (: n Int64))
        (handle
          St
          42
          ((next (u) s (resume s (+ s 1))))
          (+ (unwrap (mk (St.next))) (unwrap (mk (St.next))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 85 Int64)))

(case
  "an ABORTING arm derives its answer from an Ast op-arg and discards the continuation"
  (doc
    "The abort composition of the Ast op-arg: the arm never resumes, so the handle's value IS the
           arm's — `(Int64.of b)` on the node's BigInt payload plus the state — and the continuation's
           pending `(+ 500 …)` is DISCARDED (1000 + 25 + 0 = 1025, not 1525). Composes the Ast crossing
           with the abort shape: the payload extraction must happen on the discard path exactly as on
           the resume path.")
  (input
    (do
      (effect Halt (op stop (-> Ast Int64)))
      (def
        (main (: n Int64))
        (+
          1000
          (handle
            Halt
            0
            ((stop (a) s (match a ((Ast.Int b) (+ (Int64.of b) s)) (_ -1))))
            (+ 500 (Halt.stop (Ast.Int (BigInt.of n)))))))
      (export main)))
  (call main (: 25 Int64))
  (output (: 1025 Int64)))

; --- Effects/try leftovers: the closure-resume factory, the response-transforming adapter
; interposer (wasm-first; rust todo rides the host-effect family), and the try-composition
; faces (const folds through arm/ctor positions; runtime operand + failing fold are pending
; bricks graded todo). ---
(case
  "a handler arm resumes with a CLOSURE (in a tuple) capturing the op param and state"
  (doc
    "The factory-through-effect idiom (the deferred-resume pins wrap `resume` in a thunk; here the RESUME VALUE is a fresh closure over the arm's OWN binders): the body calls the returned fn AFTER the frame resumed, so base/s must live in the closure env, not the dead arm frame. The direct fn-typed op result curried-flattens, so the closure crosses in (Tuple (-> Int64 Int64) Int64) — also pinning a mixed closure+scalar payload through resume.")
  (input
    (do
      (effect Mk (op make (-> Int64 (Tuple (-> Int64 Int64) Int64))))
      (def
        (main (: k Int64))
        (handle
          Mk
          10
          ((make (base) s (resume #tuple((fn ((: x Int64)) (+ x (+ base s))) base) s)))
          (match (Mk.make k) (#tuple(f b) (+ (f 1) (+ (f 2) b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 38 Int64)))

(case
  "the handler STATE is a CLOSURE the arm replaces with one capturing the perform-time op argument"
  (doc
    "Strategy-as-state: the state slot carries a closure the arm APPLIES for its answer and REPLACES
           per dispatch — and the replacement `(fn (x) (+ x v))` closes over the op argument `v`, so the
           state closes over RUNTIME data from the previous perform. Seed is the identity: eval 4 → 4,
           next state adds 4; eval 3 → 7 → 407. A stale strategy (804→wrong) or a late-bound capture
           breaks the checksum. The closure sits in the STATE slot proper — the closure-in-tuple pin
           above crosses one through a RESUME value instead.")
  (input
    (do
      (effect St (op eval (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (fn ((: x Int64)) x)
          ((eval (v) f (resume (f v) (fn ((: x Int64)) (+ x v)))))
          (+ (* 100 (St.eval n)) (St.eval 3))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 407 Int64)))

(case
  "a CLOSURE state whose body performs an OUTER effect when the arm applies it"
  (doc
    "The cross-frame face of strategy-as-state: the inner handler's closure state has `(+ x
           (Aux.base))` as its body, so APPLYING the state inside the inner arm performs the OUTER
           effect — the application crosses a live handler frame. Aux seeds 50 and advances per read:
           eval 4 → 4+50 = 54 (Aux → 51), eval 3 → 3+51 = 54 → 5454. Pins that a perform fired from a
           closure applied inside another handler's ARM homes against the outer frame and its advance
           is observed by the next application.")
  (input
    (do
      (effect Aux (op base (-> Unit Int64)))
      (effect St (op eval (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Aux
          50
          ((base (u) b (resume b (+ b 1))))
          (handle
            St
            (fn ((: x Int64)) (+ x (Aux.base)))
            ((eval (v) f (resume (f v) f)))
            (+ (* 100 (St.eval n)) (St.eval 3)))))
      (export main)))
  (call main (: 4 Int64))
  (output (: 5454 Int64)))

(case
  "TWO closures minted by the same op at DIFFERENT states each keep their own snapshot"
  (doc
    "The aliasing probe of the closure-factory pin above: `mk` is performed twice with a state
           advance between, so two distinct closures exist whose envs captured DIFFERENT values of the
           same state binder. `f` captures 5, `bump` advances to 15, `g` captures 15; `(f 0)`=5 and
           `(g 0)`=15 → 515. A shared or late-bound environment gives 1515 (both see the advance) or
           15 (both see the seed) — the checksum separates all three worlds. Each resume-crossed
           closure env must be a private snapshot, not a reference into the handler frame.")
  (input
    (do
      (effect St (op mk (-> Unit (Tuple (-> Int64 Int64) Int64))) (op bump (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((mk (u) s (resume #tuple((fn ((: x Int64)) (+ x s)) 0) s))
            (bump (u) s (resume s (+ s 10))))
          (match
            (St.mk)
            (#tuple(f _z) (do (St.bump) (match (St.mk) (#tuple(g _w) (+ (* 100 (f 0)) (g 0)))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 515 Int64)))

(case
  "a CLOSURE as the op ARGUMENT — the arm applies the caller's strategy to its own state"
  (doc
    "The body→arm direction of the closure crossing (the factory pins are arm→body): the op's
           PARAMETER type is `(-> Int64 Int64)` and the body passes a different lambda per perform. The
           arm answers `(f s)` — the caller's strategy applied to the handler's CURRENT state — and
           advances. `(*3)` at s=5 → 15, then `(+7)` at s=6 → 13 → 1513. Unlike the result direction
           (which curried-flattens and needs the tuple crossing), a fn-typed op ARGUMENT is direct.
           Pins the visitor idiom: the handler owns the data, callers send the computation.")
  (input
    (do
      (effect Ap (op app (-> (-> Int64 Int64) Int64)))
      (def
        (main (: n Int64))
        (handle
          Ap
          n
          ((app (f) s (resume (f s) (+ s 1))))
          (+ (* 100 (Ap.app (fn ((: x Int64)) (* x 3)))) (Ap.app (fn ((: x Int64)) (+ x 7))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1513 Int64)))

(case
  "an ABORTING arm applies the CLOSURE STATE for its final answer"
  (doc
    "The abort face of strategy-as-state (the resumptive faces are pinned above): `(fire (v) f
           (f v))` never resumes, so the handle's value IS the strategy applied to the op argument —
           `(*7)` at 6 → 42 — and the pending continuation `(+ 500 …)` is DISCARDED (1000 + 42 = 1042,
           not 1542). The closure state must be applicable on the abort path exactly as on the resume
           path. (A closure IN the abort value itself — minted by the aborting arm and applied after —
           is the not-yet-reducible non-tail-resume boundary; applying the state to produce a SCALAR
           answer folds.)")
  (input
    (do
      (effect St (op fire (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (+ 1000 (handle St (fn ((: x Int64)) (* x 7)) ((fire (v) f (f v))) (+ 500 (St.fire n)))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 1042 Int64)))

(case
  "a WRAP-composing closure state (the replacement captures the PREVIOUS closure) folds at two dispatches"
  (doc
    "The self-referential face of strategy-as-state: each dispatch REPLACES the closure state with a
           lambda that wraps the previous one — `(fn (x) (* (f x) 2))`, so the env chain grows per perform
           (id → ×2). Two performs fold: eval 5 → 5 (state becomes ×2), eval 3 → 6 → 56. At THREE
           dispatches this shape declines (the unboundedly-growing env chain is the honest boundary) —
           this case pins the served depth, its scalar-capturing sibling below pins that the boundary is
           the CLOSURE-chain env specifically.")
  (input
    (do
      (effect St (op eval (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (fn ((: x Int64)) x)
          ((eval (v) f (resume (f v) (fn ((: x Int64)) (* (f x) 2)))))
          (+ (* 10 (St.eval n)) (St.eval 3))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

(case
  "a SCALAR-capturing closure-state replacement folds at three dispatches (no env chain)"
  (doc
    "The discriminating sibling of the wrap-composing pin above: here the replacement captures only
           the APPLIED RESULT `(let ((r (f v))) … (fn (x) (+ x r)))` — a scalar — so no closure-chain env
           grows and THREE dispatches fold where the wrap-composing shape declines. id at 5 → r=5 (state
           x+5), f(3)=8 → r=8 (state x+8), f(4)=12 → 592. Together the pair pins the exact boundary:
           what the replacement CAPTURES (prior closure vs scalar) decides the fold, not replacement or
           runtime capture per se.")
  (input
    (do
      (effect St (op eval (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          (fn ((: x Int64)) x)
          ((eval (v) f (let ((r (f v))) (resume r (fn ((: x Int64)) (+ x r))))))
          (+ (* 100 (St.eval n)) (+ (* 10 (St.eval 3)) (St.eval 4)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 592 Int64)))

(case
  "a LIST of strategies is walked recursively, each applied to a fresh perform result"
  (doc
    "Closures as COLLECTION elements consumed under a handler: `apply-all` destructures a list of
           three lambdas and applies each to its own `(Cnt.next)` — the counter walks 5,6,7 while the
           strategy changes per slot (×10, +100, id → 50 + 106 + 7 = 163). Each element's application
           and each perform must pair up in order; a re-served perform or a slot skew breaks the sum.
           (The INDEXED lookup route — `List.at`/`Map.lookup` yielding Option-of-closure with a
           perform-computed key — is a separate known wasm-codegen defect; this direct-destructure
           walk is the served face.)")
  (input
    (do
      (effect Cnt (op next (-> Unit Int64)))
      (def (apply-all fs) (match fs (#list() 0) (#list(f (.. r)) (+ (f (Cnt.next)) (apply-all r)))))
      (def
        (main (: n Int64))
        (handle
          Cnt
          n
          ((next (u) s (resume s (+ s 1))))
          (apply-all
            #list((fn ((: x Int64)) (* x 10)) (fn ((: x Int64)) (+ x 100)) (fn ((: x Int64)) x)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 163 Int64))
  (live-objects known-leak))

(case
  "an interposing handler TRANSFORMS the host response before resuming (offset adapter)"
  (doc
    "The ADAPTER sibling of the observe interposer (:866 counts + forwards unchanged): the arm transforms the host response before resuming (+1000 each; 30+40 → 2070 — a dropped transform gives 70, a double-apply 3070).")
  (input
    (do
      (effect ask (op get (-> Int64 Int64)))
      (def
        (main)
        (host
          (ask)
          (handle
            ask
            unit
            ((get (k) s (resume (+ (ask.get k) 1000) s)))
            (+ (ask.get 3) (ask.get 4)))))
      (export main)))
  (host-responses (respond ask.get (: 30 Int64)) (respond ask.get (: 40 Int64)))
  (host-calls (call ask.get) (call ask.get))
  (output (: 2070 Int64)))

(case
  "an in-program two-site handler runs INSIDE a host block beside a host call"
  (doc
    "The host-frame × in-program-frame MIX (wasm/rust; the rust-async lowering declines this
           composition — its baseline row is a todo, the interposer precedent): the host block's body
           holds a real host call AND a plain in-program handle side by side — `(+ (ask.get 3) (handle
           St 0 …))`. The host response (30) and the served two-site sift (5 pass at s=0, 1 fail → 5)
           sum to 35. Pins that an in-program handler's fold is undisturbed by a sibling host effect
           in the same body — the frames are independent.")
  (input
    (do
      (effect ask (op get (-> Int64 Int64)))
      (effect St (op sift (-> Int64 Int64)))
      (def
        (main)
        (host
          (ask)
          (+
            (ask.get 3)
            (handle
              St
              0
              ((sift (v) s (if (> v 1) (resume v (+ s 1)) (resume 0 s))))
              (+ (St.sift 5) (St.sift 1))))))
      (export main)))
  (host-responses (respond ask.get (: 30 Int64)))
  (host-calls (call ask.get))
  (output (: 35 Int64)))

(case
  "a constant-try helper folds and its result feeds a handler arm's resume"
  (doc
    "try × handler-arm composition (the effect pins keep performs on the spine, try in the body): a CONST succeeding try in a helper folds through and feeds the arm's resume.")
  (input
    (do
      (effect Ask (op ask (-> Unit Int64)))
      (def (get) (do (def v (try (Some 7))) (Some (+ v 1))))
      (def
        (main (: k Int64))
        (handle
          Ask
          unit
          ((ask (_u) s (resume (match (get) ((Option.Some v) v) ((Option.None _u) -5)) s)))
          (+ (Ask.ask unit) (* k 100))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 108 Int64)))

(case
  "a constant-try helper that fails feeds the None arm's fallback into resume"
  (doc
    "The failing face: the fold does NOT elide the short-circuit — the boundary block/break emit is its own pending brick, so this grades todo until it lands (oracle 95).")
  (input
    (do
      (effect Ask (op ask (-> Unit Int64)))
      (def (get) (do (def v (try (: (None unit) (Option Int64)))) (Some (+ v 1))))
      (def
        (main (: k Int64))
        (handle
          Ask
          unit
          ((ask (_u) s (resume (match (get) ((Option.Some v) v) ((Option.None _u) -5)) s)))
          (+ (Ask.ask unit) (* k 100))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 95 Int64)))

(case
  "constant-succeeding tries as LIST-literal elements unwrap in place and the list builds"
  (doc
    "try as a COLLECTION-constructor element (the parse-all idiom): const-succeeding tries unwrap in place and the list builds.")
  (input
    (do
      (def
        (mk)
        (: (do (def xs #list((try (Some 1)) (try (Some 2)))) (Some (List.len xs))) (Option Int64)))
      (def (main (: k Int64)) (+ (* k 0) (match (mk) ((Option.Some v) v) ((Option.None _u) -1))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2 Int64)))

(case
  "a runtime-operand try as a list element declines pending brick 3b"
  (doc
    "The runtime face: a runtime operand mid-list hits the documented brick-3b boundary — flips when the runtime-try increment lands (oracle 3 at pick=1).")
  (input
    (do
      (def
        (mk (: pick Int64))
        (:
          (do
            (def
              xs
              #list((try (Some 1))
                (try (if (= pick 1) (Some 2) (: (None unit) (Option Int64))))
                (try (Some 3))))
            (Some (List.len xs)))
          (Option Int64)))
      (def (main (: k Int64)) (match (mk k) ((Option.Some v) v) ((Option.None _u) -1)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64)))

; --- Handler-composition perimeter: per-recursion-level handles, def-bound performs beside a
; recursive performing loop, and closure-handle slot swapping through tail calls. ---
(case
  "each recursion level installs its own handle around a def-bound perform"
  (doc
    "The handle-INSIDE-the-recursion contrast to the specializer finding (handle OUTSIDE a
           recursive fn whose body def-binds a perform is the held CDZ0201): here every level of
           `nest` installs a FRESH handle seeded with its own n, def-binds the perform, and
           recurses NON-TAIL under it — each level reads its own seed and the sum n(n+1)/2 comes
           back through the nested handles (10 at k=4, 0 at k=0). The per-level handle means the
           perform's home is level-local — no cross-fn specialization needed, which is exactly why
           this computes while the outer-handle twin doesn't; pins the shape so the specializer fix
           doesn't disturb it.")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def
        (nest (: n Int64))
        (if
          (= n 0)
          0
          (handle E n ((get (u) s (resume s s))) (do (def v (E.get)) (+ v (nest (- n 1)))))))
      (def (main (: k Int64)) (nest k))
      (export main)))
  (call main (: 4 Int64))
  (output (: 10 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a def-bound perform composes with a recursive performing loop in one handle body"
  (doc
    "The WORKING perimeter of the recursive-specializer gap (a def-bound perform INSIDE the
           recursive fn is the held finding): the handle body def-binds ONE perform (100k) and then
           runs the recursive loop whose performs sit in EXPRESSION position (6k) — the two shapes
           compose in one body (106k → 212 at k=2, 0 at k=0). Pins that the straight-line def-bound
           fix and the expression-position recursion each keep working while the specializer learns
           the combined shape — a fix that re-specialized the whole body wrongly breaks one addend.")
  (input
    (do
      (effect Env (op scale (-> Int64 Int64)))
      (def
        (check-all (: i Int64) (: bad Int64))
        (if (= i 0) bad (check-all (- i 1) (+ bad (Env.scale i)))))
      (def
        (main (: k Int64))
        (handle
          Env
          k
          ((scale (v) s (resume (* v s) s)))
          (do (def first (Env.scale 100)) (+ first (check-all 3 0)))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 212 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

; --- Effects perimeter remainder: mutual-recursion state threading, argument-swap weaves,
; an inner arm consuming the outer handler's result, and an all-narrow-width handler. ---
(case
  "a MUTUALLY-recursive performing pair threads one handler state through both fns"
  (doc
    "The mutual-recursion face of the effect specializer (self-recursion is pinned; the held
           finding is the def-bound variant): `ev` and `od` alternate, each performing `Cnt.tick`
           in expression position — the specializer must clone BOTH fns of the group coherently
           (ev#eff calling od#eff calling ev#eff) and the single state threads through the
           alternation (4 ticks: 4k+6 → 14 at k=2, 6 at k=0). A specializer that cloned only the
           entry fn (leaving od calling the UN-specialized ev) loses the homing mid-alternation.")
  (input
    (do
      (effect Cnt (op tick (-> Unit Int64)))
      (def (ev (: n Int64)) (if (= n 0) 0 (+ (Cnt.tick) (od (- n 1)))))
      (def (od (: n Int64)) (if (= n 0) 0 (+ (Cnt.tick) (ev (- n 1)))))
      (def (main (: k Int64)) (handle Cnt k ((tick (u) s (resume s (+ s 1)))) (ev 4)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 14 Int64))
  (call main (: 0 Int64))
  (output (: 6 Int64)))

(case
  "an argument swap weaves with perform results through a recursive handler body"
  (doc
    "Permutation × effects: each tail call swaps the accumulators AND folds a fresh perform
           into the outgoing one — `(weave (- n 1) b (+ a (Src.next)))` interleaves the handler's
           stepping state (draws k, k+1, k+2) with the arg permutation ((0,2)→(2,3)→(3,6) → 36 at
           k=2; 12 at k=0). A lowering that sequenced the perform AFTER the slot assignment reads
           the swapped-in value into the wrong sum; one that re-performed per slot double-draws.
           The evaluation-order contract at the tail-call boundary under an active handler.")
  (input
    (do
      (effect Src (op next (-> Unit Int64)))
      (def
        (weave (: n Int64) (: a Int64) (: b Int64))
        (if (= n 0) (+ (* 10 a) b) (weave (- n 1) b (+ a (Src.next)))))
      (def (main (: k Int64)) (handle Src k ((next (u) s (resume s (+ s 1)))) (weave 3 0 0)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 36 Int64))
  (call main (: 0 Int64))
  (output (: 12 Int64)))

(case
  "an inner arm resumes with the OUTER handler's result while both states advance"
  (doc
    "The dataflow-coupled cross-level perform (the :890 interpose pin performs the outer as a
           DISCARDED observation): `bump`'s arm performs `Log.put c` and resumes with the OUTER
           handler's RESULT — each bump's value is (inner count + outer accumulator) with BOTH
           states stepping between events: 100, 102, 104 → 306. A homing that discharged the arm's
           put against the wrong level (or a resume that captured the outer's state pre-put) shifts
           an addend; the value chain 100/102/104 encodes the exact interleaving of the two state
           threads.")
  (input
    (do
      (effect Log (op put (-> Int64 Int64)))
      (effect Ctr (op bump (-> Unit Int64)))
      (def
        (main (: _mode Int64))
        (handle
          Log
          100
          ((put (v) s (resume (+ v s) (+ s 1))))
          (handle
            Ctr
            0
            ((bump (u) c (resume (Log.put c) (+ c 1))))
            (+ (Ctr.bump) (+ (Ctr.bump) (Ctr.bump))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 306 Int64)))

(case
  "an all-UInt8 handler (state, op result, arm arithmetic) computes at narrow width"
  (doc
    "The width-CONSISTENT perimeter of the handler-state widening seam (the MIXED-width
           state/result shape is the interim clean decline, flip-pinned upstream): state seeded
           `(: 10 UInt8)`, op result UInt8, arm arithmetic all-narrow — the fold computes (10)
           with no widening required. Boxes the coming widening fix from the other side: a fix
           that widened EVERY handler state to Int64 would break this narrow-consistent shape's
           typing (the op result must stay UInt8 for the caller's Int64.of).")
  (input
    (do
      (effect Src (op next (-> Unit UInt8)))
      (def
        (main (: x UInt8))
        (handle Src (: 10 UInt8) ((next (u) s (resume s (+ s x)))) (Int64.of (Src.next))))
      (export main)))
  (call main (: 5 UInt8))
  (output (: 10 Int64)))

; --- SET handler state (grow + dedup) and the two-effect recursive loop with independent states. ---
(case
  "a SET handler state grows per perform and dedupes a repeated event"
  (doc
    "The seen-set idiom as HANDLER STATE (scalar/map/record states are pinned; the set kind
           completes the collection-state family): each `note` resumes the PRE-insert len then
           inserts its event — distinct events step 0,1,2 (210 at k=5); a REPEATED event (k=10
           collides with the second note) dedupes so the third len stays 1 (110). A state threading
           that re-materialized the set per arm reads 0,0,0; one that inserted before resuming reads
           1,2,3 — both caught. The dedupe row also pins content-hashing through the threaded state
           (the same CHAMP the standalone set pins cover, here surviving perform/resume cycles).")
  (input
    (do
      (effect Seen (op note (-> Int64 Int64)))
      (def
        (main (: k Int64))
        (handle
          Seen
          #set()
          ((note (v) st (resume (Set.len st) (Set.insert st v))))
          (do
            (def a (Seen.note k))
            (def b (Seen.note 10))
            (def c (Seen.note k))
            (+ (* 100 c) (+ (* 10 b) a)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 210 Int64))
  (call main (: 10 Int64))
  (output (: 110 Int64)))

(case
  "a recursive loop performs to TWO handlers per iteration with independent states"
  (doc
    "The two-effect specialization of the working recursion shape: each iteration performs
           `A.geta` AND `B.getb` — two different effects homing to two different handler levels —
           and both states step independently (A: k,k+1,k+2; B: 100,110,120 → 3k+333: 339 at k=2,
           333 at k=0). A specializer that keyed the recursive fn's effect-clone on ONE effect
           (dropping the second's homing) or shared the two state threads breaks an arithmetic
           progression. The multi-effect face of the per-iteration perform family.")
  (input
    (do
      (effect A (op geta (-> Unit Int64)))
      (effect B (op getb (-> Unit Int64)))
      (def
        (loop (: n Int64) (: acc Int64))
        (if (= n 0) acc (loop (- n 1) (+ acc (+ (A.geta) (B.getb))))))
      (def
        (main (: k Int64))
        (handle
          A
          k
          ((geta (u) s (resume s (+ s 1))))
          (handle B 100 ((getb (u) s (resume s (+ s 10)))) (loop 3 0))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 339 Int64))
  (call main (: 0 Int64))
  (output (: 333 Int64)))

; --- The heap-valued handler-state/op-result completions: Symbol results, BigInt state+result,
; Rational state with per-step normalization. ---
(case
  "a SYMBOL-returning effect op interns through the handler and results compare by content"
  (doc
    "SYMBOL joins the op-result type family (the interner-service idiom): a (-> String Symbol) op whose arm interns via Symbol.of; results flow back through resume and a rope-arg intern equals a flat-arg intern by content, with the results also ORDERING content-lexicographically.")
  (input
    (do
      (effect Reg (op intern (-> String Symbol)))
      (def
        (main (: k Int64))
        (handle
          Reg
          0
          ((intern (s) c (resume (Symbol.of s) (+ c 1))))
          (do
            (def s1 (Reg.intern (String.concat "sym" "A")))
            (def s2 (Reg.intern "symA"))
            (def s3 (Reg.intern (if (= k 1) "symB" "symA")))
            (+ (* 100 (if (= s1 s2) 1 0)) (+ (* 10 (if (= s1 s3) 1 0)) (if (< s1 s3) 1 0))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 101 Int64)))

(case
  "a BIGINT handler state multiplies per perform and each resume reads the prior product"
  (doc
    "BIGINT joins the handler-state type family with state AND op-result both heap-numeric: the product grows per perform and each resume returns the PRIOR product (a=1, b=7, c=70 at k=7), and ALL THREE resume results are read via a digit encode (a*10000 + b*100 + c = 10770) so every resume is observed, the combined encode narrowed ONCE through checked Int64.of.")
  (input
    (do
      (effect Acc (op grow (-> Int64 BigInt)))
      (def
        (main (: k Int64))
        (handle
          Acc
          (BigInt.of 1)
          ((grow (m) s (resume s (* s (BigInt.of m)))))
          (do
            (def a (Acc.grow k))
            (def b (Acc.grow 10))
            (def c (Acc.grow 10))
            (Int64.of (+ (* a (BigInt.of 10000)) (+ (* b (BigInt.of 100)) c))))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 10770 Int64)))

(case
  "a RATIONAL handler state accumulates unit fractions exactly and resumes read the prior sum"
  (doc
    "RATIONAL completes the heap-numeric state pair: 1/2+1/3 accumulates with gcd-normalization per perform round-trip (canonical 5/6 — an unnormalized 15/18 breaks the digit encode). Each resume returns the PRIOR sum — r0=0/1, r1=1/2, r2=5/6 at k=1 — and ALL THREE are read via a num/den digit encode (r0n r0d r1n r1d r2n r2d = 0 1 1 2 5 6 -> 11256) so every resume result is observed; the runtime arg defeats folding.")
  (input
    (do
      (effect Avg (op sample (-> Int64 Rational)))
      (def
        (main (: k Int64))
        (handle
          Avg
          (Rational.of 0 1)
          ((sample (v) s (resume s (+ s (Rational.of 1 v)))))
          (do
            (def r0 (Avg.sample 2))
            (def r1 (Avg.sample 3))
            (def r2 (Avg.sample (* k 6)))
            (+
              (* 100000 (Int64.of (Rational.numerator r0)))
              (+
                (* 10000 (Int64.of (Rational.denominator r0)))
                (+
                  (* 1000 (Int64.of (Rational.numerator r1)))
                  (+
                    (* 100 (Int64.of (Rational.denominator r1)))
                    (+
                      (* 10 (Int64.of (Rational.numerator r2)))
                      (Int64.of (Rational.denominator r2))))))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11256 Int64)))

; --- A heap-field record op argument with state accumulation. ---
(case
  "a record op argument with a HEAP field crosses the perform and its scalar field accumulates state"
  (doc
    "The :4626 record-op-arg pin is all-scalar + stateless; this record carries a ROPE field beside the scalar through the perform AND the arm accumulates the scalar into STATE across two performs — the op-arg boxing keeps the heap handle beside the scalar while the state cell threads independently.")
  (input
    (do
      (effect Db (op put (-> (Record (: name String) (: qty Int64)) Int64)))
      (def
        (main (: k Int64))
        (handle
          Db
          0
          ((put (r) s (resume (+ s r.qty) (+ s r.qty))))
          (do
            (def a (Db.put #record((= name (String.concat "wid" "get")) (= qty k))))
            (def b (Db.put #record((= name "bolt") (= qty 10))))
            (+ (* 100 a) b))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 515 Int64)))

; --- The classic effect idioms as a family: reader (ambient env), writer (pre-order trace),
; and gensym (fresh-symbol allocation) — each a recursive walk performing per node/element. ---
(case
  "a RECURSIVE tree evaluator resolves variables through a READER effect with a Map-state handler"
  (doc
    "The reader-as-effect idiom (the explicit env-threading sibling landed in 05-compound): Var resolves via (Env.read name) from RECURSIVE walk frames at different depths; the handler owns the Map env as STATE; a String op-arg + scalar result cross per perform.")
  (input
    (do
      (type Expr (Lit Int64) (Var String) (Add (Tuple Expr Expr)))
      (effect Env (op read (-> String Int64)))
      (def
        (eval-e (: e Expr))
        (match
          e
          ((Expr.Lit n) n)
          ((Expr.Var name) (Env.read name))
          ((Expr.Add #tuple(a b)) (+ (eval-e a) (eval-e b)))))
      (def
        (main (: k Int64))
        (handle
          Env
          (Map.insert (Map.insert Map.empty "x" k) "y" 3)
          ((read (name) s (resume (Option.expect (Map.lookup s name) "unbound") s)))
          (eval-e (Expr.Add #tuple((Expr.Var "x") (Expr.Add #tuple((Expr.Var "y") (Expr.Lit 1))))))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 14 Int64))
  (live-objects known-leak))

(case
  "a WRITER effect accumulates a PRE-ORDER trace string during a recursive tree walk"
  (doc
    "The writer idiom: each Add/Mul node logs its op tag BEFORE recursing (pre-order), the handler concats onto a String state, and dump reads the trace back beside the value ((2+3)*4: trace exactly \"*+\" — order-sensitive; the result triple-encodes value/len/content-eq).")
  (input
    (do
      (type Expr (Lit Int64) (Add (Tuple Expr Expr)) (Mul (Tuple Expr Expr)))
      (effect Trace (op log (-> String Unit)) (op dump (-> Unit String)))
      (def
        (eval-t (: e Expr))
        (match
          e
          ((Expr.Lit n) n)
          ((Expr.Add #tuple(a b)) (do (Trace.log "+") (+ (eval-t a) (eval-t b))))
          ((Expr.Mul #tuple(a b)) (do (Trace.log "*") (* (eval-t a) (eval-t b))))))
      (def
        (main (: k Int64))
        (handle
          Trace
          ""
          ((log (tag) s (resume unit (String.concat s tag))) (dump (_u) s (resume s s)))
          (do
            (def
              v
              (eval-t (Expr.Mul #tuple((Expr.Add #tuple((Expr.Lit 2) (Expr.Lit k))) (Expr.Lit 4)))))
            (def trace (Trace.dump))
            (+ (* 100 v) (+ (* 10 (String.byte-len trace)) (if (= trace "*+") 1 0))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 2021 Int64))
  (live-objects known-leak))

(case
  "a GENSYM effect derives fresh symbols from a counter state — same base yields distinct symbols"
  (doc
    "The allocator idiom at the SYMBOL level (the scalar-id gensym pin sums draws): the arm concats the string op-arg with a counter suffix and interns — the same base twice yields DISTINCT symbols (x_e/x_o); results accumulate into a list and compare against a literal intern, exercising Option<Symbol> slot equality.")
  (input
    (do
      (effect Gensym (op fresh (-> String Symbol)))
      (def
        (rename-all (: xs (List String)) (: i Int64) (: out (List Symbol)))
        (match
          (List.at xs i)
          ((Option.Some base) (rename-all xs (+ i 1) (List.push out (Gensym.fresh base))))
          ((Option.None _u) out)))
      (def
        (main (: k Int64))
        (handle
          Gensym
          k
          ((fresh
              (base)
              n
              (resume (Symbol.of (String.concat base (if (= (% n 2) 0) "_e" "_o"))) (+ n 1))))
          (do
            (def syms (rename-all #list("x" "x" "y") 0 #list()))
            (+
              (* 100 (List.len syms))
              (+
                (* 10 (if (= (List.at syms 0) (List.at syms 1)) 1 0))
                (if (= (Option.expect (List.at syms 0) "s0") (Symbol.of "x_e")) 1 0))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 301 Int64))
  (live-objects known-leak))

; --- Uncalled-def faces of the handler validation walk (the resume-value/state pins above are
; CALLED-def shapes; these must reject whether or not the def is reached). Note the op-member
; face is CDZ0201 (closed-set row lookup), not CDZ0101. ---
(case
  "an unbound name in an uncalled def's handler-ARM resume argument is rejected"
  (doc
    "The uncalled-def face of the resume-value scope check (the CALLED-def face is pinned above): the unbound name sits in a handler arm's resume inside a never-called def — a scope walk that descends def bodies but skips handler ARMS (dispatched code, not straight-line body) runs to 42. rcdzc rejects CDZ0101.")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (unused (: k Int64)) (handle E k ((get (_u) s (resume no-such-name s))) (E.get)))
      (def (main) 42)
      (export main)))
  (error CDZ0101))

(case
  "a HANDLE of an undeclared effect in an uncalled def is rejected"
  (doc
    "The effect-NAME face: (handle NoSuchEffect ...) in a never-called def — the handle head must resolve to a declared effect whether or not the def is reached. rcdzc rejects CDZ0101.")
  (input
    (do
      (def (unused (: k Int64)) (handle NoSuchEffect k ((op (_u) s (resume 1 s))) 1))
      (def (main) 42)
      (export main)))
  (error CDZ0101))

(case
  "a PERFORM of an undeclared op on a declared effect in an uncalled def is rejected"
  (doc
    "The op-MEMBER face: E is declared with op get, but the uncalled def performs (E.no-such-op) — the op lookup on a declared effect's row is a TYPE error (CDZ0201, the closed-set member check), distinct from the CDZ0101 unbound-name faces; it must fire in uncalled defs too. rcdzc rejects CDZ0201.")
  (input
    (do
      (effect E (op get (-> Unit Int64)))
      (def (unused (: k Int64)) (handle E k ((get (_u) s (resume 1 s))) (E.no-such-op)))
      (def (main) 42)
      (export main)))
  (error CDZ0201))

(case
  "a stateful perform inside the arm of a fused match on a call result threads state once"
  (doc
    "The fused-clone seam × handler state: the match scrutinee is a CALL result (`mk` — a
           fusion candidate whose arms clone into the callee's branches) and BOTH arms perform to a
           stateful handler, with a final perform reading the count. Exactly ONE arm perform runs
           (the taken arm's — branches are exclusive) and the value encodes the order: k=7 → Hi arm
           reads 0 → 70, final reads 1 → 71; k=2 → Lo arm → 200, final → 201. The hazard is the
           handler-frame threading through the CLONED payload-binder arms — a clone that re-seeded or
           lost the state advance breaks a digit. The fused companion of the arm-perform pins (whose
           scrutinees are scalars or performs, never a fused call-result sum).")
  (input
    (do
      (effect Fresh (op next (-> Unit Int64)))
      (type Sz (Hi Int64) (Lo Int64))
      (def (mk x) (if (> x 5) (Hi x) (Lo x)))
      (def
        (main (: k Int64))
        (handle
          Fresh
          0
          ((next (u) s (resume s (+ s 1))))
          (+
            (match (mk k) ((Hi h) (+ (* 10 h) (Fresh.next))) ((Lo w) (+ (* 100 w) (Fresh.next))))
            (Fresh.next))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 71 Int64))
  (call main (: 2 Int64))
  (output (: 201 Int64)))

(case
  "host calls issue only from the TAKEN arm of a fused match and in arm order"
  (doc
    "Host delegation × the match-fusion seam: a fused match (call-result scrutinee) whose BOTH
           arms perform a host-delegated `io.get` — fusion clones each arm's host perform into the
           callee's branches, and the observable host-call sequence must stay EXACTLY ONE call (the
           taken arm's), consuming the single response: k=7 → Hi arm → 70+3=73 with [io.get] the
           whole trace. A clone that speculated the untaken arm's perform, or emitted the host call
           outside the branch dispatch, would issue TWO calls (the fixture rejects the trace) or
           consume the response in the wrong operand. Computes on ALL targets since the H1 rust
           host-call emit (b362d1414) — the no-arg integer-result shape is exactly H1's slice.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (type Sz (Hi Int64) (Lo Int64))
      (def (mk x) (if (> x 5) (Hi x) (Lo x)))
      (def
        (main (: k Int64))
        (host (io) (match (mk k) ((Hi h) (+ (* 10 h) (io.get))) ((Lo w) (- (io.get) w)))))
      (export main)))
  (host-responses (respond io.get (: 3 Int64)))
  (host-calls (call io.get))
  (call main (: 7 Int64))
  (output (: 73 Int64)))

(case
  "an abortive perform in a fused-match arm carries the payload binder out and abandons the rest"
  (doc
    "The abortive face of the fused-clone seam: the match scrutinee is a CALL result (fusion
           candidate), one arm's body performs `(Bail.bail (* h 10))` — the abort ARGUMENT reads the
           arm's SumPayload binder — abandoning a PENDING outer addition (+1000), while the other arm
           returns normally through it: k=7 → 70 (the +1000 abandoned); k=2 → 200+1000 = 1200. The
           fused clones must keep the abort's br-out-of-block correct in BOTH branch copies and route
           the payload binder into the abort argument (a clone that resumed the pending add with the
           arm value is the adv-52 class; a mis-bound payload reads garbage into the abort). NOTE this
           is a NON-TAIL abort (the arm feeds a pending +): the match-arm lowering handles what the
           if-branch non-tail abort doc note still defers.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (type Sz (Hi Int64) (Lo Int64))
      (def (mk x) (if (> x 5) (Hi x) (Lo x)))
      (def
        (main (: k Int64))
        (handle
          Bail
          0
          ((bail (n) s n))
          (+ (match (mk k) ((Hi h) (Bail.bail (* h 10))) ((Lo w) (* w 100))) 1000)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 70 Int64))
  (call main (: 2 Int64))
  (output (: 1200 Int64)))

(case
  "a host op returning Bool crosses the boundary and drives a branch"
  (doc
    "The BOOL-result host boundary (H2, rcdzc 2ded4a5a9): a `(-> Unit Bool)` delegated host op
           crosses as its i32/i64 truthiness — the host supplies `true`, the guest reads it back and
           drives `(if (Env.flag) 100 200)` → 100. wasm reads i32→bool at the boundary; the rust
           backend emits `(crate::__cdz_host_<key>() != 0)`. The bool companion of the int-result host
           pins. Note the (host-calls …) fixture names the effect LOWERCASE (`env.flag`). Computes on
           wasm + rust; rust-async declines (H2 not yet on that target).")
  (input
    (do
      (effect Env (op flag (-> Unit Bool)))
      (def (main) (host (Env) (if (Env.flag) 100 200)))
      (export main)))
  (host-responses (respond env.flag (: true Bool)))
  (host-calls (call env.flag))
  (output (: 100 Int64)))

(case
  "a let-bound host result captured by two escaping closures fires the host op once (adv-62)"
  (doc
    "adv-62 (breaker, HIGH wasm soundness): a `let`-bound host-call result captured by TWO OR MORE
           ESCAPING closures must fire the host op EXACTLY ONCE — the `let`-bound `v` is shared by both
           closures. The callee `mk` returns `(tuple (fn (x) (+ v x)) (fn (x) (* v x)))` from inside a
           `(host (io) …)`; `main` destructures the tuple and calls both closures. The bug: `mk` β-inlines
           into the match scrutinee, the match folds to a single Leaf, and — because the inlined `io.get`
           copy lost its effect-op meta — the `scrutinee_reaches_host_perform` guard missed it, so the
           bare-body fold RE-EMITTED the whole `(host …)` block once per tuple binder → `io.get` fired
           TWICE → the second call had no recorded response and TRAPPED. FIX (v-effects): the guard now
           treats a `Resolved::Host` block in the scrutinee as reaching a host perform — a CONSERVATIVE
           OVER-APPROXIMATION (not every compiling host block performs: an op-reference-only body like
           `(host (E) (E.get))` compiles without a perform — see the rcdzc regression
           `a_host_with_too_many_operands_is_cdz0201`; over-reporting is safe here because it only keeps
           the wrapper, which merely materializes the scrutinee once), so the `MatchSum` wrapper is
           kept and the scrutinee materializes ONCE;
           and the wasm `Core::Let` emit maps the scalar value node → its slot so the two closures capture
           the SAME slot rather than re-lowering the host call. With io.get=21: `f(10)=21+10=31`,
           `g(100)=21*100=2100`, sum 2131 — and the (host-calls) fixture pins the SINGLE firing. rust
           declines the shape (its closure-in-tuple-through-host emit is a separate frontier).")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (mk)
        (host
          (io)
          (let ((v (io.get unit))) #tuple((fn ((: x Int64)) (+ v x)) (fn ((: x Int64)) (* v x))))))
      (def (main) (match (mk) (#tuple(f g) (+ (f 10) (g 100)))))
      (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (output (: 2131 Int64))
  (live-objects 0))

(case
  "a host block WRAPS a match whose let-bound host result is captured by two escaping closures — fires once (adv-62)"
  (doc
    "adv-62 host-WRAPS-match face: the host block is on the OUTSIDE of the match — `(host (io) (match (let
           ((v (io.get))) #tuple(closures…)) arms))` — instead of INSIDE the scrutinee (the `(match (host …
           (let …) …) arms)` shape the sibling case above pins). This is exactly the shape the `--target
           cadenza` re-emit produces when it faithfully HOISTS a helper's `(host …)` block to wrap the whole
           match (inlining the `mk` call), and it TRAPPED on a fresh wasm compile too: the match-scrutinee
           materialization guard (`scrutinee_reaches_host_perform`) walked the `let`-scrutinee's bindings
           SUBLIST `((v (io.get …)))`, which mis-resolves as an arg-less `Resolved::Apply` and bailed WITHOUT
           descending into the `(io.get …)` init — so the guard MISSED the perform, DROPPED the `MatchSum`
           wrapper, and the bare-body fold re-emitted the whole host block once per tuple binder → `io.get`
           fired TWICE and the second call had no recorded response (trap). FIX (v-effects): the guard gained
           a `Resolved::Let` arm that descends into each binding init + the body (a strict WIDENING — detects
           more performs → keeps more materialization wrappers, always safe), so the `let`-bound host result
           is materialized ONCE and both closures capture the same slot. With io.get=21: `f(10)=21+10=31`,
           `g(100)=21*100=2100`, sum 2131, and the (host-calls) fixture pins the SINGLE firing on every
           backend.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (main)
        (host
          (io)
          (match
            (let ((v (io.get unit))) #tuple((fn ((: x Int64)) (+ v x)) (fn ((: x Int64)) (* v x))))
            (#tuple(f g) (+ (f 10) (g 100))))))
      (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (output (: 2131 Int64))
  (live-objects 0))

(case
  "a host block WRAPS a match binding TWO distinct host calls each captured by its own closure — fires once each in order (adv-62)"
  (doc
    "The two-distinct-calls ORDER companion of the host-WRAPS-match single-call pin above — coverage the
           single-op case can't give. The host block WRAPS the whole match (the host-wraps-match shape the
           adv-62 fix targets); the `let` binds `x = io.a` and `y = io.b` and returns `(tuple (fn (n) (+ x n))
           (fn (n) (* y n)))`; `main` calls both closures. Each host op must fire EXACTLY ONCE and IN ORDER
           (io.a then io.b) — the pre-fix host-wraps-match double-fire would have fired each per capturing
           closure and/or lost the order. This pins that the `Resolved::Let` scrutinee-materialization arm
           (the adv-62 host-wraps-match fix) materializes a MULTI-binding let-scrutinee once, preserving both
           the single firing of each op AND their order, not just the single-op count. With io.a=3, io.b=5:
           `f(10)=3+10=13`, `g(100)=5*100=500`, sum 513, and the (host-calls io.a io.b) fixture pins both
           firings and their order.")
  (input
    (do
      (effect io (op a (-> Unit Int64)) (op b (-> Unit Int64)))
      (def
        (main)
        (host
          (io)
          (match
            (let ((x (io.a unit)) (y (io.b unit))) #tuple((fn ((: n Int64)) (+ x n)) (fn ((: n Int64)) (* y n))))
            (#tuple(f g) (+ (f 10) (g 100))))))
      (export main)))
  (host-responses (respond io.a (: 3 Int64)) (respond io.b (: 5 Int64)))
  (host-calls (call io.a) (call io.b))
  (output (: 513 Int64))
  (live-objects 0))

(case
  "two DISTINCT let-bound host calls each captured by its own escaping closure fire once each in order (adv-62)"
  (doc
    "The two-distinct-calls ORDER companion of the adv-62 single-call pin above (breaker escalation):
           `mk` binds `x = io.a` and `y = io.b` inside `(host (io) …)` and returns `(tuple (fn (n) (+ x n))
           (fn (n) (* y n)))`; `main` calls both. Each host op must fire EXACTLY ONCE and IN ORDER (io.a
           then io.b) — the per-closure re-fire bug (fixed #1528) would have fired each per capturing closure
           and/or lost the order. With io.a=3, io.b=5: `f(10)=3+10=13`, `g(100)=5*100=500`, sum 513, and the
           (host-calls io.a io.b) fixture pins BOTH the single firing of each AND their order — coverage the
           single-call pin can't give. rust/rust-async decline the closure-in-tuple-through-host shape (todo),
           as with the single-call case.")
  (input
    (do
      (effect io (op a (-> Unit Int64)) (op b (-> Unit Int64)))
      (def
        (mk)
        (host
          (io)
          (let
            ((x (io.a unit)) (y (io.b unit)))
            #tuple((fn ((: n Int64)) (+ x n)) (fn ((: n Int64)) (* y n))))))
      (def (main) (match (mk) (#tuple(f g) (+ (f 10) (g 100)))))
      (export main)))
  (host-responses (respond io.a (: 3 Int64)) (respond io.b (: 5 Int64)))
  (host-calls (call io.a) (call io.b))
  (output (: 513 Int64))
  (live-objects 0))

(case
  "a unit-result host op consumes its response row so the next value op reads its own (adv-65)"
  (doc
    "adv-65 (breaker, HIGH wasm differential): a UNIT-result host op must CONSUME its response row,
           in order, so a later value-result op reads ITS OWN row — not the unit op's. `(host (io) (do
           (io.ping k) (+ (io.get k) k)))` with responses [io.ping=0, io.get=7], k=3 → 10 (io.get reads
           its own row 7: 7+3). The wasm host runner previously did NOT advance the response cursor on a
           unit-result op (it returns nothing), so `io.get` read io.ping's row 0 → 3 (silent wrong value);
           rust was correct (per-op response lists). FIX (v-effects, cdz-run): a unit-result op advances
           the cursor IFF the row at the cursor is FOR THIS OP (kebab-normalized match) — consuming a
           supplied row, but NOT skipping a value op's row for a pure observe-only unit op (H8's `log.emit`
           shape, which supplies no row). The response model is in-order consumption of ALL calls.")
  (input
    (do
      (effect io (op ping (-> Int64 Unit)) (op get (-> Int64 Int64)))
      (def (main (: k Int64)) (host (io) (do (io.ping k) (+ (io.get k) k))))
      (export main)))
  (host-responses (respond io.ping (: 0 Int64)) (respond io.get (: 7 Int64)))
  (host-calls (call io.ping) (call io.get))
  (call main (: 3 Int64))
  (output (: 10 Int64)))

(case
  "the unit-op response-cursor discriminator: a nonzero ping row is not misread by the later get (adv-65)"
  (doc
    "adv-65 CURSOR DISCRIMINATOR: the same shape with io.ping's row = 99 (not 0) — if the unit op
           failed to consume its row, io.get would read 99 → 102; consuming it correctly gives io.get its
           own row 7 → 10. Pins that the fix READS THE RIGHT ROW, not merely that a zero happens to be
           harmless.")
  (input
    (do
      (effect io (op ping (-> Int64 Unit)) (op get (-> Int64 Int64)))
      (def (main (: k Int64)) (host (io) (do (io.ping k) (+ (io.get k) k))))
      (export main)))
  (host-responses (respond io.ping (: 99 Int64)) (respond io.get (: 7 Int64)))
  (host-calls (call io.ping) (call io.get))
  (call main (: 3 Int64))
  (output (: 10 Int64)))

(case
  "a host result captured by two closures stored in a RECORD fires the host op once (adv-62b)"
  (doc
    "adv-62b (breaker→v-effects, HIGH wasm soundness): the RECORD-face sibling of adv-62 — a
           `let`-bound host-call result captured by two closures stored in a RECORD must fire the host op
           EXACTLY ONCE. `(def (mk) (host (io) (let ((v (io.get))) (record (f (fn (x) (+ v x))) (g (fn (x)
           (* v x)))))))` + `(def (main) (let ((r (mk))) (+ ((. r f) 10) ((. r g) 100))))` → 2131 (io.get=21
           once: f(10)=31 + g(100)=2100). The bug: `r`'s init `(mk)` reaches a host call THROUGH THE CALL,
           but `subtree_reaches_host_call`'s AST walk stopped at the `(mk)` node and missed the host call in
           mk's body → `r` was copy-propagated → each `(. r •)` re-inlined the `(host …)` block → io.get
           fired PER projection → the 2nd call had no recorded response and TRAPPED. FIX (v-effects): the
           `should_keep_binding` host-force-keep test now ALSO follows a CALL init into its inlined callee
           body (`core_reaches_host_call`, a Core-tree walk, gated to a `Resolved::Apply` init), so `r` is
           force-kept — materialized ONCE, every projection reads the shared record slot via `LocalRef`.
           rust declines the record-of-closures-through-host shape (a separate frontier). The tuple/match
           face is adv-62 (#1528); this is the record face.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (mk)
        (host
          (io)
          (let
            ((v (io.get unit)))
            #record((= f (fn ((: x Int64)) (+ v x))) (= g (fn ((: x Int64)) (* v x)))))))
      (def (main) (let ((r (mk))) (+ (r.f 10) (r.g 100))))
      (export main)))
  (host-responses (respond io.get (: 21 Int64)))
  (host-calls (call io.get))
  (output (: 2131 Int64))
  (live-objects known-leak))

(case
  "a runtime Bytes value crosses a host op boundary as list<u8> (H-bytes-arg)"
  (doc
    "The wasm host-ARG Bytes path: a runtime `Bytes` argument to a host op crosses the component
           boundary as `list<u8>` (the (ptr,len) shared-memory shape, same core marshalling as a String arg
           but a `list<u8>` component type — a DEFINED type referenced by index in the import instance-type,
           vs String's inline `string`). Previously DECLINED on wasm while the rust backend crossed it — a
           reverse-parity coverage gap (v-rust-backend flagged, breaker banked the probe). `main(k)` slices a
           runtime `to-bytes` rope `(Bytes.slice … k 3)` and passes the 3-byte view to `io.sink`; the host
           answers 99. Pins that a Bytes host arg (a) COMPILES on wasm and (b) the emitted component is valid
           (`sink: func(p0: list<u8>) -> s64`, wasm-tools-verified) and (c) RUNS. rust already passed it;
           rust-async declines the multi-def host-do shape (todo). The canon Lower carries Memory(0) (no
           realloc for an argument — the guest allocates, the host reads).")
  (input
    (do
      (effect io (op sink (-> Bytes Int64)))
      (def
        (main (: k Int64))
        (host
          (io)
          (match
            (Bytes.slice (String.to-bytes (String.concat "abc" "defgh")) k 3)
            ((Some cut) (io.sink cut))
            ((None _u) -1))))
      (export main)))
  (host-responses (respond io.sink (: 99 Int64)))
  (host-calls (call io.sink))
  (call main (: 2 Int64))
  (output (: 99 Int64))
  (live-objects known-leak))

(case
  "a host op with a Bytes arg AND a scalar arg crosses both parameters (list<u8> beside a scalar)"
  (doc
    "Coverage for the wasm Bytes-host-arg increment's MIXED-ARITY face: a host op `sink2 : (-> Bytes
           Int64 Int64)` takes a `list<u8>` param AND a scalar `Int64` param. Pins that the import
           instance-type + core functype handle a `list<u8>` param (a defined-type-index) BESIDE an inline
           scalar — the func type declares `(p0: list<u8>, p1: s64) -> s64`, and the core marshalling pushes
           the Bytes `(ptr,len)` then the scalar. `main(k)` passes a 2-byte `Bytes.of` and the scalar 5; the
           host answers 9. Complements the single-Bytes-arg pin (which has no scalar to prove the mixed
           layout). rust crosses it too (both scalar+list handle-transport); rust-async declines the shape.")
  (input
    (do
      (effect io (op sink2 (-> Bytes Int64 Int64)))
      (def
        (main (: k Int64))
        (host (io) (io.sink2 (Bytes.of #list((UInt8.wrap k) (UInt8.wrap 66))) 5)))
      (export main)))
  (host-responses (respond io.sink2 (: 9 Int64)))
  (host-calls (call io.sink2))
  (call main (: 65 Int64))
  (output (: 9 Int64))
  (live-objects known-leak))

(case
  "a host result captured by closures in a NESTED tuple fires the host op once (adv-62 nested face)"
  (doc
    "adv-62 family, NESTED-destructure face: the let-bound host result `v` is shared by closures at
           TWO tuple nesting levels — `(tuple f (tuple g h))` — destructured by a nested pattern. All three
           closures capture the ONE `io.get` (fired once at the shared `let`, not re-lowered per projection);
           the fix (should_keep_binding follows the CALL init into mk's host body → force-keep + materialize
           once) threads through the nested `Core::Proj` chain. io.get=10: f(1)=11, g(2)=20, h(3)=7, sum 38.
           rust crosses it; rust-async declines the closure-in-tuple-through-host shape (todo).")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def
        (mk)
        (host
          (io)
          (let
            ((v (io.get unit)))
            #tuple((fn ((: x Int64)) (+ v x))
              #tuple((fn ((: x Int64)) (* v x)) (fn ((: x Int64)) (- v x)))))))
      (def (main) (match (mk) (#tuple(f #tuple(g h)) (+ (+ (f 1) (g 2)) (h 3)))))
      (export main)))
  (host-responses (respond io.get (: 10 Int64)))
  (host-calls (call io.get))
  (output (: 38 Int64))
  (live-objects 0))

(case
  "a host-block scrutinee folding to a multi-arm sum switch fires the host op once (adv-62 switch face)"
  (doc
    "adv-62 family, SWITCH-path face (vs the Leaf-fold face the base cases pin): the host block `(mk)`
           β-inlines into the MATCH SCRUTINEE and folds to a multi-arm sum SWITCH (not a single-arm Leaf), so
           the scrutinee-reaches-host-perform guard keeps the `MatchSum` wrapper and materializes the host
           call ONCE — each arm reads the one materialized scrutinee, not a re-lowered `(host …)` block. io.get=7
           → (> 7 5) → Big 7 → 7*10 = 70 (the Small arm's +100 discriminates). Pins that the host materialize
           holds on the Switch path, not just the tuple/record Leaf fold. rust crosses it; rust-async declines.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (type R (Big Int64) (Small Int64))
      (def (mk) (host (io) (let ((v (io.get unit))) (if (> v 5) (Big v) (Small v)))))
      (def (main) (match (mk) ((Big h) (* h 10)) ((Small w) (+ w 100))))
      (export main)))
  (host-responses (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (output (: 70 Int64)))

; --- Host-row consumption under CONTROL FLOW: the (host-responses …) fixture is consumed in the
; ORDER calls are made, and only calls on the taken path consume rows. The pins above fix the
; straight-line order (two calls in one +) and the abandoned-path elision; these pin the
; consumption order when the CALL SEQUENCE is produced by recursion (tail and non-tail) and when
; a runtime branch selects WHICH op fires first. ---
(case
  "a recursion-driven host-call sequence consumes one response row per iteration in order"
  (doc
    "The recursive-walk composition of the two-calls-in-order pin: `walk` performs `(io.get)` once
           per iteration in TAIL position, so n=3 consumes the rows [3,7,5] first-to-last as the digits
           accumulate left-to-right → 375. A runner that re-read row 0 per iteration gives 333; one that
           consumed from the tail gives 573. The host-calls fixture asserts exactly three calls.")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def (walk (: n Int64) (: acc Int64)) (if (> n 0) (walk (- n 1) (+ (* 10 acc) (io.get))) acc))
      (def (main (: n Int64)) (host (io) (walk n 0)))
      (export main)))
  (host-responses
    (respond io.get (: 3 Int64))
    (respond io.get (: 7 Int64))
    (respond io.get (: 5 Int64)))
  (host-calls (call io.get) (call io.get) (call io.get))
  (call main (: 3 Int64))
  (output (: 375 Int64)))

(case
  "a NON-TAIL host call consumes rows on the unwind, deepest frame first"
  (doc
    "The unwind-order face: `(+ (* 10 (walk (- n 1))) (io.get))` recurses BEFORE performing, so the
           deepest frame's `(io.get)` fires first — rows [3,7,5] bind deepest-to-shallowest and the digits
           accumulate 3 → 37 → 375. The same rows in a TAIL-position walk (the pin above) yield the same
           375 by a DIFFERENT path (there, row order = iteration order; here, row order = unwind order) —
           a runner that issued calls at frame-ENTRY rather than at the perform's evaluation point would
           flip the two shapes apart (573 here).")
  (input
    (do
      (effect io (op get (-> Unit Int64)))
      (def (walk (: n Int64)) (if (> n 0) (+ (* 10 (walk (- n 1))) (io.get)) 0))
      (def (main (: n Int64)) (host (io) (walk n)))
      (export main)))
  (host-responses
    (respond io.get (: 3 Int64))
    (respond io.get (: 7 Int64))
    (respond io.get (: 5 Int64)))
  (host-calls (call io.get) (call io.get) (call io.get))
  (call main (: 3 Int64))
  (output (: 375 Int64)))

(case
  "a runtime branch selects WHICH host op consumes the first response row"
  (doc
    "The branch-selected companion of the abandoned-path elision pin: `(if (> n 5) (io.get) (io.alt))`
           at n=3 takes the alt branch, so the FIRST row consumed is `io.alt`'s 100, and the following
           unconditional `(io.get)` consumes the second row 7 → 107. The host-calls fixture asserts the
           taken-path sequence [alt, get] — a runner that consumed rows by op-declaration order (get
           first) or issued the untaken branch's call would mis-bind both rows.")
  (input
    (do
      (effect io (op get (-> Unit Int64)) (op alt (-> Unit Int64)))
      (def (main (: n Int64)) (host (io) (+ (if (> n 5) (io.get) (io.alt)) (io.get))))
      (export main)))
  (host-responses (respond io.alt (: 100 Int64)) (respond io.get (: 7 Int64)))
  (host-calls (call io.alt) (call io.get))
  (call main (: 3 Int64))
  (output (: 107 Int64)))

; --- Handler-ARM effect composition beyond the single observation pin above (:1031, whose
; re-performed value is discarded): arms whose OUTER-perform results feed the resume value,
; sibling handlers sharing one outer counter, and a transitive arm-perform cascade. ---
(case
  "an arm performs the outer effect TWICE and the results feed the resume value"
  (doc
    "The value-carrying face of the arm-performs-outer pin: A's arm resumes `(+ (Count.tick)
           (Count.tick))` — the observation IS the resume value, not a discarded side effect. Count
           seeded 10: the two arm ticks read 10 and 11 (arm resumes 21, Count threads to 12), and a
           tick AFTER the inner handle closes reads 12 → 21 + 100·12 = 1221. Pins that an arm's
           under-frame performs advance the outer state exactly like body-level performs (a re-seeded
           or frame-local Count gives 21+100·10=1021; per-arm-entry re-reads give 20).")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect Count (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Count
          10
          ((tick (u) c (resume c (+ c 1))))
          (+
            (handle A 0 ((a (u) s (resume (+ (Count.tick) (Count.tick)) s))) (A.a))
            (* 100 (Count.tick)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1221 Int64)))

(case
  "TWO sibling inner handlers observe through ONE outer counter"
  (doc
    "Under-frame threading ACROSS sequential handler frames: sibling handles A and B each tick
           the same enclosing Count from their arms. Count seeded 0: A's arm ticks (0→1), B's arm
           ticks (1→2), the body's final tick reads 2 → 7 + 10·3 + 100·2 = 237. Pins that the outer
           state is ONE line threading through both siblings in evaluation order — per-handler counter
           instances (a frame-local clone) would read 0 at the final tick (37).")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect Count (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Count
          0
          ((tick (u) c (resume c (+ c 1))))
          (+
            (handle A 0 ((a (u) s (do (Count.tick) (resume 7 s)))) (A.a))
            (+
              (* 10 (handle B 0 ((b (u) s (do (Count.tick) (resume 3 s)))) (B.b)))
              (* 100 (Count.tick))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 237 Int64)))

(case
  "a depth-3 transitive arm-perform cascade threads the innermost state end to end"
  (doc
    "C's arm performs B; B's arm performs A — each perform resolving one frame further out
           (the under-frame discipline applied TRANSITIVELY). A seeded 100 resumes s and threads s+1.
           `(C.c)` → C's arm asks B ×10 → B's arm asks A → 100 (A→101) → C resumes 1000. The second
           `(C.c)` walks the same cascade reading 101 (A→102) → 1010. The direct `(A.a)` then reads
           102. 1000+1010+102 = 2112. A cascade that re-entered A at its seed per chain (2102), or
           resolved B's perform against a stale frame, breaks the sum.")
  (input
    (do
      (effect A (op a (-> Unit Int64)))
      (effect B (op b (-> Unit Int64)))
      (effect C (op c (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          A
          100
          ((a (u) s (resume s (+ s 1))))
          (handle
            B
            0
            ((b (u) s (resume (A.a) s)))
            (handle C 0 ((c (u) s (resume (* 10 (B.b)) s))) (+ (C.c) (+ (C.c) (A.a)))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2112 Int64)))

(case
  "TWO Map-stated handlers stacked route each op to its own Map with no cross-contamination"
  (doc
    "Heap-valued handler state × handler stacking: A and B each carry their own `(Map.empty)`-seeded
           state; six interleaved ops (3 to A at one regime, 2-or-3 to B) must route each `put` to ITS
           handler's Map, and both `size` reads at the end see only their own inserts — 3/3 at n=3 (33)
           and 2/3 at n=1 where A's third put duplicates key 1 (23). A state-slot mixup between the
           stacked frames (one Map receiving the other's insert, or a size read against the wrong frame)
           corrupts either count. The heap-state sibling of the scalar two-handler pins; each resume
           dups/drops a CHAMP handle per op.")
  (input
    (do
      (effect A (op puta (-> Int64 Unit)) (op sizea (-> Unit Int64)))
      (effect B (op putb (-> Int64 Unit)) (op sizeb (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          (Map.empty)
          ((puta (k) m (resume unit (Map.insert m k k))) (sizea (u) m (resume (Map.len m) m)))
          (handle
            B
            (Map.empty)
            ((putb (k) m (resume unit (Map.insert m k k))) (sizeb (u) m (resume (Map.len m) m)))
            (do
              (A.puta 1)
              (B.putb 10)
              (A.puta 2)
              (B.putb 20)
              (B.putb 30)
              (A.puta n)
              (+ (* 10 (A.sizea)) (B.sizeb))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 33 Int64))
  (call main (: 1 Int64))
  (output (: 23 Int64)))

(case
  "performs in BOTH operands of an or consume state only on the reached paths"
  (doc
    "The RESUMPTIVE-perform composition with short-circuit (the abortive pins cover elision of an
           ABORT; this observes handler STATE): `(or (> (Ctr.tick) 10) (> (Ctr.tick) 3))` seeded k, with
           a trailing tick pinning the exact post-connective state. k=20: the lhs tick reads 20 (s→21),
           true short-circuits the rhs → 100 + 21 = 121 (ONE tick). k=4: lhs 4 (s→5) false, rhs 5 (s→6)
           true → 100 + 6 = 106 (TWO ticks). k=0: both false (s→2) → 200 + 2 = 202. A fold treating the
           rhs perform as unconditional double-fires and shifts every digit — the adv-55 rhs-conditionality
           class observed at the STATE tier, where a wrong fold is visible even when the boolean value
           happens to agree. (Core::And is the shared and/or core node — this case correctly references
           it even though the surface operator here is `or`.)")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main (: k Int64))
        (handle
          Ctr
          k
          ((tick (u) s (resume s (+ s 1))))
          (+ (if (or (> (Ctr.tick) 10) (> (Ctr.tick) 3)) 100 200) (Ctr.tick))))
      (export main)))
  (call main (: 20 Int64))
  (output (: 121 Int64))
  (call main (: 4 Int64))
  (output (: 106 Int64))
  (call main (: 0 Int64))
  (output (: 202 Int64)))

(case
  "a Map-stated handler threads 50 recursive puts and reads the accumulated size"
  (doc
    "Heap-valued handler state × recursion at scale: `fill` performs one `put` per recursive step,
           each resume dup/dropping the CHAMP handle as the Map grows to 50 entries — a Perceus witness
           on the handler path (a per-resume leak or premature free surfaces as memory corruption or a
           fault long before 50). The trailing `size` reads the fully-accumulated state (50). The
           straight-line put pins cover 2-3 ops; this is the recursion-driven volume shape a memoizing
           pass actually produces.")
  (input
    (do
      (effect Store (op put (-> Int64 Unit)) (op size (-> Unit Int64)))
      (def (fill (: i Int64)) (if (= i 0) unit (do (Store.put i) (fill (- i 1)))))
      (def
        (main (: n Int64))
        (handle
          Store
          (Map.empty)
          ((put (k) m (resume unit (Map.insert m k (* k 2)))) (size (u) m (resume (Map.len m) m)))
          (do (fill n) (Store.size))))
      (export main)))
  (call main (: 50 Int64))
  (output (: 50 Int64))
  (live-objects known-leak))

(case
  "a recursive performer of a nested-handler op whose resume performs the outer effect threads the advance"
  (doc
    "The recursive-nested-arm-resume fix (v-effects self-probe, concierge-steered pre-spec-lift): a
           recursive `loop` calls a nested `B` handler's op `B.step` whose ARM resume-value performs the OUTER
           `A` effect — `(step (u) t (resume (A.tick) t))`. Each iteration's `A.tick` reads+advances A-state.
           `loop 2` sums the two B.step results (= A.tick's pre-advance values 10 then 11) → 21. Pins that the
           per-iteration outer advance made INSIDE a nested handler's resume-value threads correctly across the
           recursion — the merge specializes `loop` against BOTH A and B, and the pre-spec-lift makes the
           arm-hidden `A.tick` a direct-body perform so it threads via the top-level perform arm. Before the
           fix the merge was skipped (the outer perform hidden in B's arm was invisible to the merge decision)
           and the advance dropped. NO post-loop A read here (that observing sub-case is a separate increment).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def (loop (: n Int64)) (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (handle B 0 ((step (u) t (resume (A.tick) t))) (loop 2))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a recursive nested-op performer whose per-iteration outer advance is read by a POST-loop observer threads it"
  (doc
    "The post-loop-observer companion of the case above — the sub-case that one explicitly deferred. The
           recursive `loop` calls `B.step` whose arm resumes with the outer `(A.tick)`; then a POST-loop
           `(A.get)` reads the A-state the recursion advanced. `loop 1` calls `B.step` once → `A.tick` returns
           the pre-advance A-state 10 and advances A → 11; the loop sums that one value = 10. Then `(A.get)`
           reads the ADVANCED A-state 11, so `(+ (loop 1) (A.get))` = `(+ 10 11)` = 21. Pins that the outer
           advance the recursion made is OBSERVABLE after the loop — the merged specialization returns the
           advanced A out-state (multi-value) and the post-loop `(A.get)` reads it, not the pre-loop seed
           (which would give 20 — the silent miscompile this fix eliminates). Requires: (1) the merged
           specialization target the accum-COPY of the seed-wrapped `loop` (`accum_seed_redirect`, threading
           the accumulator seed as a call-site arg), and (2) the merged nested-handler body drain its pending
           multi-value spec-call temp into a wrapping `let` (else the out-state projection leaks its binder).")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (def (loop (: n Int64)) (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (handle B 0 ((step (u) t (resume (A.tick) t))) (+ (loop 1) (A.get)))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a DEPTH-3 nested-op chain whose deepest resume performs the outer effect declines cleanly (no silent drop)"
  (doc
    "The depth-3 companion of the post-observer case above — the outer perform hides TWO handler levels
           down. `loop` performs `C.hop`; C's arm resumes `(B.step)`; B's arm resumes `(A.tick)`; then a
           post-loop `(A.get)`. The correct value is 21 (tick returns 10 advancing A→11; loop=10; A.get reads
           11). The depth-2 fix's pre-spec-lift (`lift_inner_op_arm_outer_perform`) rewrites `(C.hop)` into
           C's resume value `(B.step)` in ONE step, but does NOT chase `B.step`'s OWN arm-hidden `(A.tick)` —
           so folding it would specialize against B alone and DROP A's advance → a SILENT 20 (the regression
           this guards against — it briefly shipped that way in #2136 before the depth-3 guard). A correct
           depth-3 fold must lift RECURSIVELY (a later increment); until then this DECLINES cleanly (a decline
           is safe, a wrong value is not). `resume_val_op_arm_also_performs_outer` detects the deeper chain
           (the op the resume value performs has an arm that itself performs YET ANOTHER effect op) and leaves
           it un-lifted → `specialize_recursive` declines. Flips decline→21 when the recursive lift lands.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (effect C (op hop (-> Unit Int64)))
      (def (loop (: n Int64)) (if (= n 0) 0 (+ (C.hop) (loop (- n 1)))))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
          (handle
            B
            0
            ((step (u) t (resume (A.tick) t)))
            (handle C 0 ((hop (u) w (resume (B.step) w))) (+ (loop 1) (A.get))))))
      (export main)))
  (output (: 21 Int64)))

(case
  "a DEPTH-3 nested-op chain WITHOUT a post-observer folds (the no-observer control for the observer-gated guard)"
  (doc
    "The no-observer control (breaker rx6) for the depth-3 decline case above. The SAME recursion ×
           depth-3 chain — `loop` performs `C.hop`; C's arm resumes `(B.step)`; B's arm resumes `(A.tick)` —
           but the body is bare `(loop 2)` with NO post-loop observer and a single-op `A`. Without an observer
           of the recursion's out-state, the accum-redirect never engages, so the between-iteration advance
           carries through the merge and the chain FOLDS: `loop 2` sums the two `A.tick` pre-advance values 10
           then 11 = 21. This pins that the observer-GATED depth-3+ guard does NOT over-decline the working
           no-observer chain — the guard (`caller_observes_outstate && resume_val_op_arm_also_performs_outer`)
           fires ONLY when the out-state is observed (the decline case above), so this twin is unaffected.
           #2179's guard briefly over-declined this (fold→decline); the observer gate is what separates the
           must-decline observer chain from this must-fold no-observer twin.")
  (input
    (do
      (effect A (op tick (-> Unit Int64)))
      (effect B (op step (-> Unit Int64)))
      (effect C (op hop (-> Unit Int64)))
      (def (loop (: n Int64)) (if (= n 0) 0 (+ (C.hop) (loop (- n 1)))))
      (def
        (main)
        (handle
          A
          10
          ((tick (u) s (resume s (+ s 1))))
          (handle
            B
            0
            ((step (u) t (resume (A.tick) t)))
            (handle C 0 ((hop (u) w (resume (B.step) w))) (loop 2)))))
      (export main)))
  (output (: 21 Int64)))

(case
  "an s-around-k ctl arm that ALSO performs an outer effect in the arm body folds — the two E5 fixes compose"
  (doc
    "The COMPOSITION guard for the two E5 fixes: the s-around-k lexical-`ctl` pin (`pin_refs_to_binders`)
           and the arm-performs-outer path must compose without re-orphaning the pinned state binder. An inner
           `G` handler's arm reads the state binder `s` AROUND its `(k x)` continuation application AND performs
           the OUTER `A` effect in the SAME arm body — `(y (x) s k (+ (+ s (A.get)) (k x)))`. Seeded n=100
           (runtime param), A seeded 7: `s`=100, `(A.get)`=7, `(k 5)`=5 (the continuation `C = □` returns 5),
           so `(+ (+ 100 7) 5)` = 112. Pins that s-around-k + an arm-body outer perform fold together (a naive
           interaction re-detached the arm body after the pin, re-leaking `s` as CDZ0101 — the pre-fix ek1
           signature; this witness catches that regression). breaker ek8.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect G (op y (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          7
          ((get (u) s (resume s s)))
          (handle G n ((y (x) s k (+ (+ s (A.get)) (k x)))) (G.y 5))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 112 Int64)))

(case
  "an s-around-k ctl arm whose K-ARGUMENT performs an outer effect folds"
  (doc
    "The k-argument face of the composition guard above: the outer perform sits INSIDE the `(k …)`
           argument rather than beside it — `(y (x) s k (+ s (k (+ x (A.get)))))`. Seeded n=100, A seeded 7:
           `(A.get)`=7, the k-arg `(+ x (A.get))` = `(+ 5 7)` = 12, `(k 12)` returns 12 into `C = □`, and `s`
           around it = 100, so `(+ 100 12)` = 112. Pins that the arm-body state binder `s` stays resolved when
           the `(k v)`→`(resume v s)` rewrite's argument itself performs an outer effect. breaker ek8d.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect G (op y (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          7
          ((get (u) s (resume s s)))
          (handle G n ((y (x) s k (+ s (k (+ x (A.get)))))) (G.y 5))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 112 Int64)))

(case
  "an s-around-k ctl arm whose handle BODY performs an outer effect after the inner perform folds"
  (doc
    "The body-perform face: the s-around-k arm has NO perform of its own — `(y (x) s k (+ s (k (+ x
           1))))` — but the inner `G` handle's BODY performs the outer `A` effect AFTER the G-perform: `(+
           (G.y 5) (A.get))`. Seeded n=100, A seeded 7: `(G.y 5)` folds the arm — `(k (+ 5 1))` = `(k 6)` = 6
           into `C = □`, `s`=100 → `(+ 100 6)` = 106; then the body's `(A.get)`=7, so `(+ 106 7)` = 113. Pins
           that the pinned state binder survives when the OUTER perform is in the handle body (region-wrapped
           around the inner handle) rather than in the arm. breaker ek10.")
  (input
    (do
      (effect A (op get (-> Unit Int64)))
      (effect G (op y (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          7
          ((get (u) s (resume s s)))
          (handle G n ((y (x) s k (+ s (k (+ x 1))))) (+ (G.y 5) (A.get)))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 113 Int64)))

(case
  "a nested-handler ctl arm whose continuation-consuming body ALSO performs an OUTER effect folds"
  (doc
    "The confluence of the lexical-`ctl` surface and the nested-handler outer-perform family: an INNER
           handler `B`'s 5-part arm applies its continuation `k` AND, in the same continuation-consuming
           body, performs an OUTER handler `A`'s op — `(flip () t k (+ (* (k 2) 10) (A.geta)))` under
           `handle A(handle B … (B.flip))`. When `k` is applied lexically `(k 2)` = `(resume 2 t)` returning
           into B's delimited context `C = □` (the whole B body is the flip) = 2, so `(* 2 10)` = 20; then
           `(A.geta)` reads A's state (seeded 100) = 100, giving `(+ 20 100)` = 120. Pins that the within-
           activation `ctl`→`resume` rewrite composes with a sibling OUTER perform in the SAME arm body under
           a nested handler — the lexical-`k` result and the foreign `A.geta` both resolve and thread
           correctly (a miscompile would drop A's read or mis-thread the continuation). Guards the seam
           between the lexical-`ctl` fold and the nested-handler outer-perform threading.")
  (input
    (do
      (effect A (op geta (-> Unit Int64)))
      (effect B (op flip (-> Unit Int64)))
      (def
        (main)
        (handle
          A
          100
          ((geta (u) s (resume s (+ s 1))))
          (handle B 0 ((flip () t k (+ (* (k 2) 10) (A.geta)))) (B.flip))))
      (export main)))
  (output (: 120 Int64)))

(case
  "a closure looked up from a map by a perform result, applied to a perform result, threads through call_indirect"
  (doc
    "A Map of CLOSURES indexed by a perform-computed key, the selected closure applied to a
           perform-fed argument, under a resumptive handler: `(match (Map.lookup ops (St.pick)) ((Some f)
           (f (St.feed))) ((None _u) -1))`. This is the closure-from-collection × effects-threaded-operands
           shape — the looked-up closure is the funcref-table callee (`call_indirect`) and BOTH its selector
           key and its applied argument are perform results the handler fold splices in. Pins a wasm-codegen
           miscompile (breaker-found, v-effects-routed, 2026-08-05): the closure operand is a dup-site
           `Core::SumPayload` (the `Some` payload) whose Perceus retain floats its cell into a scratch slot
           typed i32; the perform-threaded i64 argument was materialized into that SAME slot (the closure and
           the arg both emitted at `cell_slot + 1`), and a wasm local has one type function-wide → an i32/i64
           collision → `call_indirect`'s function failed to validate (invalid module, wasmtime rejected at
           compile). The rust backend always ran it correctly (the fold is sound; the defect was purely the
           wasm scratch-slot allocation). ops = {0: x↦x*2, 1: x↦x+1000}; the pick/feed handler threads s=5,6,…
           so pick→5%2=1 selects the +1000 closure, feed→6, giving 6+1000 = 1006.")
  (input
    (do
      (effect St (op pick (-> Unit Int64)) (op feed (-> Unit Int64)))
      (def
        (main (: n Int64))
        (do
          (def
            ops
            (Map.insert
              (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2)))
              1
              (fn ((: x Int64)) (+ x 1000))))
          (handle
            St
            n
            ((pick (u) s (resume (% s 2) (+ s 1))) (feed (u) s (resume s (+ s 1))))
            (match (Map.lookup ops (St.pick)) ((Some f) (f (St.feed))) ((None _u) -1)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1006 Int64))
  (live-objects 0))

(case
  "a record-LITERAL scrutinee whose fields perform, destructured by a `(record …)` arm, DECLINES (breaker finding #8)"
  (doc
    "REJECT-DON'T-MISCOMPILE (v-effects finding #8, breaker 3-backend-agree-wrong). A record-pattern
           field binder resolves to `Resolved::Member { operand: scrutinee }` (resolve.rs Case 6rec), whose
           member-FOLD re-lowers the SOURCE record literal's field init at EACH projection. When the scrutinee
           is a record LITERAL with PERFORMING fields, each field-binder read re-evaluates the literal, so
           `(St.next)` fires once per binder: this shape silently returned 80 (want 56) on ALL 3 backends —
           the draws fired 2× the operations (wrong binder values) AND the re-eval advances were not all
           committed to the outer state (a DOUBLE defect). The `MatchSum` wrapper materializes the scrutinee
           into ONE slot, but a record binder reads BY NAME through the fold, BYPASSING that slot — unlike a
           tuple/sum binder, which reads the slot via `Elem`/`SumPayload`, so the TUPLE-literal twin below is
           CORRECT. The correct fold-fix (fold the record binder onto the materialized slot) is a deeper
           member-fold rewire, itself blocked behind a coupled scope bug (a let-BOUND record MATCHED under a
           handle declines CDZ0101), so until both land this DECLINES (CDZ0201) rather than miscompile. The
           workaround the diagnostic names — `let`-bind then read by `(. r field)` projection — folds
           correctly (the positive control below). Flips decline→56 when the slot-fold lands.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (match #record((= a (St.next)) (= b (St.next))) (#record((= a x) (= b y)) (+ (* 10 x) y)))))
      (export main)))
  (error CDZ0201))

(case
  "the record-perform workaround — `let`-bind then read by `(. r field)` projection — folds once each (finding #8 control)"
  (doc
    "The POSITIVE control for the record-literal-perform decline above (breaker rw2). Binding the record
           with `let` first, then reading its fields by `(. r a)` / `(. r b)` PROJECTION, evaluates each
           performing field EXACTLY ONCE: the `let` binder materializes the record once and each projection
           reads the materialized value — no re-eval, no re-perform. `next` returns s then advances (5→6, 6→7),
           so a=5, b=6, giving (* 10 5) + 6 = 56. This pins that the decline is SCOPED to the record-LITERAL ×
           record-DESTRUCTURE shape and that the recommended workaround is genuinely correct on all backends.")
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((r #record((= a (St.next)) (= b (St.next))))) (+ (* 10 r.a) r.b))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 56 Int64)))

; ── literal-arm dispatch on effectful draws (breaker za) ─────────────────────────────────────────
; A match whose LITERAL arms are selected by a state draw, where the SELECTED arm performs again.
; Three escalations: dispatch on a raw draw (za1), on a scrutinee COMPUTED from two draws — the
; derived-key dispatch idiom (za2), and NESTED two-level literal dispatch where each level matches
; a let-bound draw and only the innermost arm performs a third time (za3). Pins that arm selection
; sees the PRE-arm state while the arm body sees the POST-selection state — the draw that chose the
; arm and the draw the arm reads are distinct dispatches of the same op against an advancing state.
(case
  "za1 literal-arm dispatch on a draw — the MATCHED arm performs again, both calls exercised"
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((k (St.next))) (match k (5 (+ 100 (St.next))) (6 200) (_o 300)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 106 Int64))
  (call main (: 6 Int64))
  (output (: 200 Int64))
  (call main (: 9 Int64))
  (output (: 300 Int64)))

(case
  "za2 a COMPUTED scrutinee (difference of two draws) selects the performing arm at one input only"
  (input
    (do
      (effect St (op next (-> Unit Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next (u) s (resume s (* s 2))))
          (let
            ((a (St.next)))
            (let ((b (St.next))) (match (- b a) (5 (+ 1000 (St.next))) (_o (- 0 (- b a))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1020 Int64))
  (call main (: 3 Int64))
  (output (: -3 Int64)))

(case
  "za3 NESTED literal-arm dispatch — each level matches a let-bound draw, the innermost arm reads a third"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 3))))
          (let
            ((d1 (St.next)))
            (match
              d1
              (5 (let ((d2 (St.next))) (match d2 (8 (+ 100 (St.next))) (_i (- 0 _i)))))
              (_o (* 10 _o))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 111 Int64))
  (call main (: 3 Int64))
  (output (: 30 Int64)))

; ── same-effect shadowing + the let-crossing-nested-handle scope gap (breaker sh) ────────────────
; sh1 pins nested SAME-effect handler shadowing: inner draws hit the inner state/arm, the first
; draw after the inner region hits the outer — two independent state threads, innermost-wins
; dispatch. sh2d/sh2n pinned the DECLINE face and now FOLD (the SEED sub-face is fixed): a let
; bound inside a handle body read by a nested handle's SEED orphaned (freshen_walk left the nested
; handle opaque, so the freshened outer binder cfg→#cfgN was not rewritten in the seed reference).
; FIX: freshen the nested handle-internal's SEED child under the enclosing renames (effects.rs
; freshen_walk). sh2d main(10)=32 (seed=11, inner seed 22, one inner draw 22, trailing outer draw
; 10), sh2n main(5)=11. The remaining sub-face — the outer binder read in the inner handle's ARM or
; BODY (sh2g/sh2m) — still declines CDZ0101 (freshening the arm/body risks the fn-local-ref orphan;
; a separate careful increment). Params cross fine; lets bound OUTSIDE the outer handle cross fine.
(case
  "sh1 a NESTED handler for the SAME effect shadows the outer — inner draws hit the inner state, the draw after hits the outer"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (+
            (handle
              St
              5
              ((next () s (resume s (* s 10))))
              (let ((a (St.next))) (let ((b (St.next))) (+ a b))))
            (St.next))))
      (export main)))
  (call main (: 100 Int64))
  (output (: 155 Int64))
  (call main (: 7 Int64))
  (output (: 62 Int64)))

(case
  "sh2d a let bound in a handle body, read by a NESTED handle's seed — folds (let-crossing seed freshened)"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((seed (+ n 1)))
            (+ (handle St (* seed 2) ((next () s (resume s (+ s 100)))) (St.next)) (St.next)))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 32 Int64)))

(case
  "sh2n the config-fetch idiom with its perform LET-LIFTED — folds (let-crossing seed freshened)"
  (input
    (do
      (effect A (op base (-> Int64)))
      (effect B (op step (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((base () s (resume s s)))
          (let
            ((seed (A.base)))
            (handle B seed ((step () t (resume t (+ t 1)))) (+ (B.step) (B.step))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 11 Int64)))

; ── multi-op, deep, and heap-state same-effect shadowing (breaker mo) ────────────────────────────
; Escalations of the sh1 shadowing pin: mo1 shadows a TWO-op effect (the inner handler
; re-interprets BOTH ops with different arms — doubling get, striding bump); mo2 stacks the same
; effect THREE deep with draws interleaved before/inside/after each region (three independent
; state threads); mo4 pins the shadow REGION boundary — a same-effect draw in the inner handle's
; SEED homes to the OUTER handler (the shadow starts at the body, not the seed); mo5 threads
; independent heap LISTS as the two states (interleaved pushes, each arm measuring its own list).
(case
  "mo1 a TWO-op effect fully shadowed — the inner handler re-interprets BOTH ops with different arms"
  (input
    (do
      (effect St (op get (-> Int64)) (op bump (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((get () s (resume s s)) (bump () s (resume s (+ s 1))))
          (+
            (handle
              St
              50
              ((get () s (resume (* s 2) s)) (bump () s (resume s (+ s 10))))
              (+ (St.bump) (St.get)))
            (St.get))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 177 Int64))
  (call main (: 100 Int64))
  (output (: 270 Int64)))

(case
  "mo2 THREE-deep same-effect shadowing — each depth's draws thread its own state, interleaved before/inside/after"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (+
            (St.next)
            (+
              (handle
                St
                100
                ((next () s (resume s (+ s 20))))
                (+
                  (St.next)
                  (+
                    (handle St 7 ((next () s (resume s (* s 3)))) (+ (St.next) (St.next)))
                    (St.next))))
              (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 259 Int64))
  (call main (: 0 Int64))
  (output (: 249 Int64)))

(case
  "mo4 a SAME-effect draw in the inner handle's SEED homes to the OUTER handler — the shadow starts at the body, not the seed"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (+
            (handle St (* (St.next) 10) ((next () s (resume s (+ s 100)))) (+ (St.next) (St.next)))
            (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 206 Int64))
  (call main (: 2 Int64))
  (output (: 143 Int64)))

(case
  "mo5 LIST-state shadowing — inner and outer thread independent heap lists, growth interleaved"
  (input
    (do
      (effect St (op push (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          #list(n)
          ((push (v) s (resume (List.len s) (List.push s v))))
          (+
            (St.push 10)
            (+
              (handle
                St
                #list(7 8 9)
                ((push (v) s (resume (+ (List.len s) 100) (List.push s v))))
                (+ (St.push 1) (St.push 2)))
              (St.push 20)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 210 Int64))
  (call main (: 0 Int64))
  (output (: 210 Int64)))

; ── same-effect shadows installed from INSIDE handler machinery (breaker is) ─────────────────────
; The mo chapter shadows in the handle BODY; these install the shadow from handler-adjacent
; positions. is1: a draw-SELECTED match arm installs a branch-local shadow — the inner region
; exists on one dispatch path only, and the outer thread resumes after it. is2: the ARM's
; RESUME-VALUE is a nested same-effect handle instantiated per dispatch from the live state s.
; is3: the ARM's NEXT-STATE slot is computed by a nested same-effect handle — the shadow's
; result feeds the outer state thread itself (s' = 3·(s+100) per dispatch).
(case
  "is1 a draw-SELECTED match arm installs a nested same-effect shadow — outer state resumes after the branch-local region"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((k (St.next)))
            (+
              (match
                k
                (5 (handle St 70 ((next () s (resume s (+ s 7)))) (+ (St.next) (St.next))))
                (_o (* 2 _o)))
              (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 153 Int64))
  (call main (: 3 Int64))
  (output (: 10 Int64)))

(case
  "is2 the ARM's resume-value is a nested SAME-effect handle — a fresh shadow instantiated per dispatch from the live state"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next
              ()
              s
              (resume
                (handle St (* s 10) ((next () t (resume t (+ t 1)))) (+ (St.next) (St.next)))
                (+ s 1))))
          (+ (St.next) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 222 Int64))
  (call main (: 2 Int64))
  (output (: 102 Int64)))

(case
  "is3 the ARM's NEXT-STATE is computed by a nested SAME-effect handle — the shadow feeds the outer state thread"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next
              ()
              s
              (resume
                s
                (handle St (+ s 100) ((next () t (resume t (* t 2)))) (+ (St.next) (St.next))))))
          (+ (St.next) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 320 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

; ── a fresh same-effect handler per RECURSION frame (breaker hp) ─────────────────────────────────
; A recursive fn that installs a handler around its own recursive call — a dynamic handler STACK
; grown per frame. This folds (the mutual-recursion ≥2-call-site decline does not bite: one
; recursive call site, handle wrapped around the recursion). hp1 draws PRE-order (each frame
; draws its own seed, the base case draws from the DEEPEST frame); hp2 draws POST-order (after
; the recursive return — each frame's state must survive its entire subtree); hp3 puts the
; recursive call in the SEED position (each frame's handler seeded by the whole subtree below,
; the seed evaluating in the CALLER's ambient handler).
(case
  "hp1 a RECURSIVE fn installs a fresh same-effect handler per frame — the base case draws from the deepest, each frame from its own"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (level (: d Int64))
        (if
          (<= d 0)
          (St.next)
          (handle St (* d 100) ((next () s (resume s (+ s 1)))) (+ (St.next) (level (- d 1))))))
      (def (main (: n Int64)) (handle St 7 ((next () s (resume s (+ s 1)))) (level n)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 401 Int64))
  (call main (: 0 Int64))
  (output (: 7 Int64))
  (call main (: 3 Int64))
  (output (: 701 Int64)))

(case
  "hp2 per-frame handlers with a POST-ORDER draw — each frame draws its own state AFTER the recursive call returns"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (level (: d Int64))
        (if
          (<= d 0)
          (St.next)
          (handle
            St
            (* d 100)
            ((next () s (resume s (+ s 1))))
            (+ (level (- d 1)) (* 1000 (St.next))))))
      (def (main (: n Int64)) (handle St 7 ((next () s (resume s (+ s 1)))) (level n)))
      (export main)))
  (call main (: 2 Int64))
  (output (: 301100 Int64))
  (call main (: 1 Int64))
  (output (: 101100 Int64)))

(case
  "hp3 the recursive call sits in the SEED — each frame's handler is seeded by the whole subtree below it"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (level (: d Int64))
        (if
          (<= d 0)
          (St.next)
          (handle St (level (- d 1)) ((next () s (resume s (+ s 1)))) (+ (St.next) (St.next)))))
      (def (main (: n Int64)) (handle St n ((next () s (resume s (* s 2)))) (level 2)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 23 Int64))
  (call main (: 3 Int64))
  (output (: 15 Int64)))

; ── SAME-effect performs as op ARGUMENTS (breaker pa) ────────────────────────────────────────────
; The delegated-effect-as-argument pin covers a HOST effect feeding an intra-program op; these pin
; the same-effect face: an op's argument is itself a perform against the SAME handler, so the arg
; dispatch advances the very state the outer dispatch's arm reads. pa1 = one draw feeding a
; stateful scale. pa2 = TWO draws as the two arguments of one 2-ary op — pins left-to-right
; argument evaluation and that the arm sees the post-args state. pa3 = a THREE-deep nested
; arg-feed scale(scale(next)) where scale itself advances state per dispatch.
(case
  "pa1 a SAME-effect perform as another op's ARGUMENT — the arg dispatch advances the state the outer dispatch reads"
  (input
    (do
      (effect St (op next (-> Int64)) (op scale (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))) (scale (v) s (resume (* v s) s)))
          (+ (St.scale (St.next)) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 36 Int64))
  (call main (: 3 Int64))
  (output (: 16 Int64)))

(case
  "pa2 TWO same-effect draws as the TWO arguments of one op — left-to-right arg order, the arm reads the post-args state"
  (input
    (do
      (effect St (op next (-> Int64)) (op mix (-> Int64 Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))) (mix (a b) s (resume (+ (* 100 a) (+ (* 10 b) s)) s)))
          (St.mix (St.next) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 567 Int64))
  (call main (: 0 Int64))
  (output (: 12 Int64)))

(case
  "pa3 THREE-deep nested same-effect arg-feed — scale(scale(next)) with a state-advancing scale arm"
  (input
    (do
      (effect St (op next (-> Int64)) (op scale (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))) (scale (v) s (resume (+ (* 10 v) s) (+ s 1))))
          (St.scale (St.scale (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 567 Int64))
  (call main (: 2 Int64))
  (output (: 234 Int64)))

; ── PERFORMING match guards (breaker gp) ─────────────────────────────────────────────────────────
; Guards whose CONDITION performs. gp1: one performing guard over a LET-BOUND draw — the guard's
; own dispatch advances state, and BOTH the hit and miss arm bodies read the guard-advanced state.
; (The let-bound scrutinee is load-bearing: an INLINE performing scrutinee with a performing guard
; is breaker finding #9's guard-miss re-eval, filed to v-effects.) gp2e: TWO pure guards cascade
; over a draw with a re-performing fallback. gp2-decline: >=2 guard ARMS where ANY guard performs
; is an honest not-yet-reducible decline (the multi-guard arm-copy cascade lands the performing
; condition non-tail); flip values banked (main 6=248, 30=111, 1=-8) for when that fold lands.
(case
  "gp1 a PERFORMING guard on a wildcard pattern is a COMPILE ERROR (guards-side-effect-free, CDZ0407)"
  (doc
    "Was a fold pin (breaker gp1); guards-side-effect-free (operator directive PR #2543) makes a perform
           in a guard cond a COMPILE ERROR. `(St.next)` in `(> (St.next) 6)` → CDZ0407. breaker re-adds the
           dispatch-semantics coverage as a let-lifted pure-guard equivalent in a follow-up batch (bind the
           guard's draw to a `let` before the match, guard on the bound value).")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((k (St.next)))
            (match k ((guard _x (> (St.next) 6)) (+ 100 (St.next))) (_o (* 10 (St.next)))))))
      (export main)))
  (error CDZ0407))

(case
  "gp2e TWO pure guards cascade over a draw, the fallback re-performs — guard misses leave dispatch serviceable"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (let
            ((k (St.next)))
            (match k ((guard _a (> _a 50)) 111) ((guard _b (> _b 10)) 222) (_o (- 0 (St.next)))))))
      (export main)))
  (call main (: 60 Int64))
  (output (: 111 Int64))
  (call main (: 20 Int64))
  (output (: 222 Int64))
  (call main (: 3 Int64))
  (output (: -6 Int64)))

(case
  "gp2 TWO guard arms whose conditions PERFORM (`(> (St.next) …)`) — a COMPILE ERROR (CDZ0407), guards-side-effect-free"
  (doc
    "Reclassified from a `(declines)` fold-gap to `(error CDZ0407)` (v-effects, 2026-08-29): a perform in
           a guard condition is a COMPILE ERROR under the guards-side-effect-free directive (operator PR #2543),
           the SAME rule the finding-#9 cases below pin. gp2 is the multi-guard face — a PURE let-bound scrutinee
           `k` matched by TWO arms whose guard conditions each perform `St.next` — and the compiler emits CDZ0407
           at the performing guard exactly as it does for the finding-#9 performing-scrutinee shape. It is NOT a
           fold gap: folding a performing guard is precisely the re-eval MISCOMPILE #2543 banned (a guard MISS
           would re-draw), so this stays rejected forever. Pinning the CODE (not a bare `(declines)`, which any
           refusal satisfies) guards that a future change cannot silently start FOLDING it. Workaround: lift each
           `(St.next)` to a `let` before the match and guard on the bound value.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (let
            ((k (St.next)))
            (match
              k
              ((guard _a (> (St.next) 50)) 111)
              ((guard _b (> (St.next) 10)) (+ 200 (St.next)))
              (_o (- 0 (St.next)))))))
      (export main)))
  (error CDZ0407))

(case
  "a performing scrutinee matched by a PERFORMING GUARD is a COMPILE ERROR (breaker finding #9, now CDZ0407)"
  (doc
    "HISTORY: v-effects finding #9 was a silent 3-backend re-eval MISCOMPILE — `(match (St.next) ((guard x
           (> x (St.next))) …) (_o _o))` where the performing guard's desugar copied the performing scrutinee
           per named binder, so a guard MISS re-drew (f1: 6 not 3). Interim fix reject-don't-miscompiled it (a
           todo decline). SUPERSEDED by guards-side-effect-free (operator directive PR #2543): a perform in a
           guard cond is now a COMPILE ERROR (CDZ0407) — `(> x (St.next))` has a perform, so the whole
           finding-#9 shape errors at the guard BEFORE the fold ever sees it. The re-eval class is eliminated
           at the source for the guard arm. Workaround: lift the guard's draw to a `let` before the match and
           guard on the bound value (breaker re-adds pure-guard dispatch coverage in a follow-up).")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (match (St.next) ((guard x (> x (St.next))) (* 100 x)) (_o _o))))
      (export main)))
  (error CDZ0407))

(case
  "a performing guard with a WILDCARD fallback is ALSO a COMPILE ERROR (finding #9 sibling, CDZ0407)"
  (doc
    "The wildcard-fallback sibling of the finding-#9 shape above: same performing scrutinee + performing
           guard `(> x (St.next))` but a `_` fallback. Under the interim fold-decline this folded (single named
           binder = one copy); under guards-side-effect-free it is a COMPILE ERROR like every performing guard —
           the perform in the guard cond is CDZ0407 regardless of the fallback shape. Pins that the guard-reject
           is fallback-shape-agnostic (named or wildcard): a performing guard is illegal, period.")
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (match (St.next) ((guard x (> x (St.next))) (* 100 x)) (_ 99))))
      (export main)))
  (error CDZ0407))

; ── an inner handle's RESULT feeds outer control flow (breaker hs) ───────────────────────────────
; The inner same-effect handle runs to completion and its VALUE drives the enclosing region's
; control: hs1 = the result is a match SCRUTINEE (a literal arm selects on it; the arm body and
; the trailing draw hit the OUTER state); hs2 = the inner handle is OUTER-seeded and builds a
; TUPLE scrutinee whose destructured arm re-performs against the outer; hs3 = the result is an
; IF-condition operand compared against an outer draw, both branches re-performing outer.
(case
  "hs1 an inner SAME-effect handle's result is the match SCRUTINEE — the selected arm and the trailing draw hit the OUTER state"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (+
            (match
              (handle St 4 ((next () t (resume t (* t 3)))) (+ (St.next) (St.next)))
              (16 (+ 1000 (St.next)))
              (_o (- 0 _o)))
            (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1011 Int64))
  (call main (: 0 Int64))
  (output (: 1001 Int64)))

(case
  "hs2 an OUTER-seeded inner handle builds a tuple scrutinee — the destructured arm re-performs against the outer"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (match
            (handle St (St.next) ((next () t (resume t (* t 2)))) #tuple((St.next) (St.next)))
            (#tuple(a b) (+ (* 100 a) (+ (* 10 b) (St.next)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 606 Int64))
  (call main (: 2 Int64))
  (output (: 243 Int64)))

(case
  "hs3 an inner SAME-effect handle's result is the IF condition — the taken branch re-performs against the outer"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (if
            (> (handle St 10 ((next () t (resume t (+ t 5)))) (+ (St.next) (St.next))) (St.next))
            (+ 100 (St.next))
            (- 0 (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 106 Int64))
  (call main (: 50 Int64))
  (output (: -51 Int64)))

; ── two-effect interleaving under nested handlers (breaker ti) ───────────────────────────────────
; Distinct effects A/B(/C) with both handlers live, states advancing in one expression. ti1 =
; A-B-A-B draws interleaved in ONE sum, each effect threading its own state. ti2b = B's arm
; performs A in its resume-VALUE on EVERY dispatch (three dispatches — deepens the single-dispatch
; resume-value pin). ti3 = arm cross-feed PLUS direct body interleave in the same program. ti4 =
; THREE-effect chained cross-feed (C's arm performs B, B's arm performs A; two C dispatches walk
; the whole chain twice). ti5 = the region face: a FOREIGN outer perform in the inner handle's
; BODY (not its arm) — A advances inside B's region and the post-region draw sees it. (The
; cross-effect perform in a NEXT-STATE slot stays the corpus-pinned honest decline.)
(case
  "ti1 A-B-A-B interleaved draws in ONE expression — each effect threads its own state independently"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (+ s 1))))
          (handle B (* n 10) ((b () t (resume t (+ t 100)))) (+ (A.a) (+ (B.b) (+ (A.a) (B.b)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 211 Int64))
  (call main (: 1 Int64))
  (output (: 123 Int64)))

(case
  "ti1h the HEAP-state twin of ti1 — two DIFFERENT effects, BOTH threading a TUPLE (heap) state, interleaved A-B-A draws with values crossing"
  (doc
    "ti1 threads SCALAR state per effect; this pins the HEAP (#seed/#st) dimension of the same
           two-different-effect interleave — the merged_nested_ctx multi-slot path where each effect carries
           its own TUPLE state, read+advanced by projection. A.ta reads s, resumes it, advances s[1]+=v;
           B.tb reads s, resumes it, advances s[0]+=v; body draws (A.ta 1),(B.tb 4),(A.ta 2) and sums their
           projections. main = 2n+101 (3->107, 10->121, 0->101). Guards the two-slot heap-state THREADING +
           per-slot reclaim (the multi-slot resume-seam OccTable path, design note §2/§5.1): each effect's
           heap state must thread on its OWN slot without the interleave bleeding one into the other, and
           neither is over-freed across the crossing draws. WITNESSED trap-CLEAN (v-effects a52) on both the
           release runtime (rc-underflow OOB) and the debug #4635 getter-guard (read-through). Complements
           ti1 (scalar) + the same-effect straddle heap twin (#5883).")
  (input
    (do
      (effect A (op ta (-> Int64 (Tuple Int64 Int64))))
      (effect B (op tb (-> Int64 (Tuple Int64 Int64))))
      (def
        (main (: n Int64))
        (handle
          A
          #tuple(n 0)
          ((ta (v) s (resume s #tuple((. s 0) (+ (. s 1) v)))))
          (handle
            B
            #tuple(100 0)
            ((tb (v) s (resume s #tuple((+ (. s 0) v) (. s 1)))))
            (let
              ((p (A.ta 1)) (q (B.tb 4)) (r (A.ta 2)))
              (+ (. p 0) (+ (. q 0) (+ (. r 0) (. r 1))))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 107 Int64))
  (call main (: 10 Int64))
  (output (: 121 Int64))
  (call main (: 0 Int64))
  (output (: 101 Int64)))

(case
  "ti2b the inner arm's resume-VALUE performs the outer effect on EVERY dispatch — three dispatches, both states advancing"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (+ s 1))))
          (handle B 0 ((b () t (resume (+ t (A.a)) (+ t 1)))) (+ (B.b) (+ (B.b) (B.b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 21 Int64))
  (call main (: 2 Int64))
  (output (: 12 Int64)))

(case
  "ti3 cross-effect resume-value feed PLUS direct body interleave — B's arm and the body both advance A"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (* s 2))))
          (handle B 1000 ((b () t (resume (+ t (A.a)) (- t 1)))) (+ (B.b) (+ (A.a) (B.b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2034 Int64))
  (call main (: 1 Int64))
  (output (: 2006 Int64)))

(case
  "ti4 THREE-effect chained cross-feed — C's arm performs B, B's arm performs A, two C dispatches walk the whole chain twice"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (effect C (op c (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (+ s 1))))
          (handle
            B
            100
            ((b () t (resume (+ t (A.a)) (+ t 1))))
            (handle C 10000 ((c () u (resume (+ u (B.b)) (+ u 1)))) (+ (C.c) (C.c))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20213 Int64))
  (call main (: 0 Int64))
  (output (: 20203 Int64)))

(case
  "ti5 a FOREIGN outer perform inside the inner region — A advances inside B's body, the post-region draw sees it"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (+ s 1))))
          (+ (handle B 50 ((b () t (resume t (* t 2)))) (let ((x (B.b))) (+ x (A.a)))) (A.a))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 61 Int64))
  (call main (: 0 Int64))
  (output (: 51 Int64)))

; ── STRING handler state (breaker ss) ────────────────────────────────────────────────────────────
; A heap STRING as the threaded handler state: ss1 grows it per dispatch with the growth chunk
; branching on the op ARGUMENT (the arm returns the pre-growth byte-len); ss2 grows it across a
; DO sequence where discarded draws still advance the rope; ss3 nests SAME-effect handlers with
; independent string states — the inner self-DOUBLES (a rope-of-ropes) while the outer appends.
(case
  "ss1 a STRING handler state grows per dispatch — each arm returns the pre-growth length, growth is op-arg-branchy"
  (input
    (do
      (effect Log (op emit (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Log
          "x"
          ((emit (v) s (resume (String.byte-len s) (String.concat s (if (> v 0) "ab" "c")))))
          (+ (Log.emit n) (+ (* 10 (Log.emit n)) (* 100 (Log.emit 0))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 531 Int64))
  (call main (: 0 Int64))
  (output (: 321 Int64)))

(case
  "ss2 string state grows across a DO sequence — discarded draws still advance the rope"
  (input
    (do
      (effect Log (op emit (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          Log
          "s"
          ((emit () s (resume (String.byte-len s) (String.concat s "yz"))))
          (do (Log.emit) (Log.emit) (+ (* 10 (Log.emit)) n))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 55 Int64))
  (call main (: 0 Int64))
  (output (: 50 Int64)))

(case
  "ss3 nested SAME-effect handlers with independent STRING states — inner self-doubles, outer appends"
  (input
    (do
      (effect Log (op emit (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          Log
          "aa"
          ((emit () s (resume (String.byte-len s) (String.concat s "b"))))
          (+
            (Log.emit)
            (+
              (*
                10
                (handle
                  Log
                  "wxyz"
                  ((emit () t (resume (String.byte-len t) (String.concat t t))))
                  (+ (Log.emit) (Log.emit))))
              (* 1000 (Log.emit))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3122 Int64)))

; ── let-lifted pure-guard dispatch coverage (breaker gl; restores the gp shapes post-CDZ0407) ────
; guards-side-effect-free made a perform in a guard cond CDZ0407 (the gp1/gp2 pins above became
; reject witnesses). These restore the DISPATCH-semantics coverage in the sanctioned form the
; CDZ0407 text teaches: bind the guard's draw(s) to lets evaluated once BEFORE the match, guard
; on the bound values. gl2 also makes the formerly-declined multi-guard cascade expressible.
(case
  "gl1 the let-lifted pure-guard equivalent of the gp1 dispatch shape — the pre-bound guard draw advances the state the arms read"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (let
            ((k (St.next)))
            (let
              ((g (St.next)))
              (match k ((guard _x (> g 6)) (+ 100 (St.next))) (_o (* 10 (St.next))))))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 108 Int64))
  (call main (: 2 Int64))
  (output (: 40 Int64)))

(case
  "gl2 the let-lifted pure-guard equivalent of the gp2 cascade — both guard draws pre-bound, all three arms reachable"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (let
            ((k (St.next)))
            (let
              ((g1 (St.next)))
              (let
                ((g2 (St.next)))
                (match
                  k
                  ((guard _a (> g1 50)) 111)
                  ((guard _b (> g2 10)) (+ 200 (St.next)))
                  (_o (- 0 (St.next)))))))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 248 Int64))
  (call main (: 30 Int64))
  (output (: 111 Int64))
  (call main (: 1 Int64))
  (output (: -8 Int64)))

; ── MAP handler state keyed by op ARGUMENTS (breaker mk) ─────────────────────────────────────────
; The map-state pins above enumerate/measure; these pin KEYED dynamics: mk1 = per-key counters
; (lookup-or-default then insert, key = the op arg); mk2 = keys DERIVED from prior draws with a
; deliberate collision path (n=1 makes the derived key hit the first insert); mk3 = the arm
; SHRINKS the state via remove (idempotent re-remove, missing-key no-op).
(case
  "mk1 a MAP handler state keyed by the op ARGUMENT — per-key counters, lookup-or-default then insert per dispatch"
  (input
    (do
      (effect Reg (op touch (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Reg
          #map((= 1 10))
          ((touch
              (k)
              s
              (resume
                (match (Map.lookup s k) ((Some v) v) ((None) 0))
                (Map.insert s k (+ (match (Map.lookup s k) ((Some v) v) ((None) 0)) 1)))))
          (+ (Reg.touch n) (+ (* 10 (Reg.touch n)) (* 100 (Reg.touch 1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1010 Int64))
  (call main (: 1 Int64))
  (output (: 1320 Int64)))

(case
  "mk2 map keys DERIVED from prior draws — n=1 collides the derived key with the first insert, shrinking the map"
  (input
    (do
      (effect Reg (op touch (-> Int64 Int64)) (op size (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          Reg
          #map()
          ((touch (k) s (resume (Map.len s) (Map.insert s k k))) (size () s (resume (Map.len s) s)))
          (let
            ((a (Reg.touch n)))
            (let ((b (Reg.touch (+ a 1)))) (+ (* 100 (Reg.size)) (+ (* 10 b) (Reg.touch n)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 212 Int64))
  (call main (: 1 Int64))
  (output (: 111 Int64)))

(case
  "mk3 the arm SHRINKS the map state via remove — re-removing the same key is idempotent, a missing key is a no-op"
  (input
    (do
      (effect Reg (op drop (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Reg
          #map((= 1 11) (= 2 22) (= 3 33))
          ((drop (k) s (resume (Map.len (Map.remove s k)) (Map.remove s k))))
          (+ (Reg.drop n) (+ (* 10 (Reg.drop n)) (* 100 (Reg.drop 3))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 122 Int64))
  (call main (: 9 Int64))
  (output (: 233 Int64)))

; ── tuple-state SLOT dynamics (breaker tp) ───────────────────────────────────────────────────────
; The tuple-state pin above advances one slot with the other held; these pin slot DYNAMICS:
; tp1 = two ops each OWN a slot (lo +1s field 0, hi doubles field 1); tp2 = a SWAP op exchanges
; the slots and interleaved readers observe it; tp3 = a NESTED tuple ((a b) c) whose arm rebuilds
; the inner Fibonacci pair while bumping the outer counter.
(case
  "tp1 TWO ops each own a tuple-state SLOT — lo advances field 0, hi doubles field 1, interleaved"
  (input
    (do
      (effect Tw (op lo (-> Int64)) (op hi (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          Tw
          #tuple(n (* n 10))
          ((lo () s (resume (. s 0) #tuple((+ (. s 0) 1) (. s 1))))
            (hi () s (resume (. s 1) #tuple((. s 0) (* (. s 1) 2)))))
          (+ (Tw.lo) (+ (Tw.hi) (+ (Tw.lo) (Tw.hi))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 161 Int64))
  (call main (: 1 Int64))
  (output (: 33 Int64)))

(case
  "tp2 a SWAP op exchanges the tuple-state slots — interleaved readers observe the exchange"
  (input
    (do
      (effect Tw (op rd (-> Int64)) (op swap (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          Tw
          #tuple(n 100)
          ((rd () s (resume (. s 0) #tuple((+ (. s 0) 1) (. s 1))))
            (swap () s (resume (. s 1) #tuple((. s 1) (. s 0)))))
          (+ (Tw.rd) (+ (* 10 (Tw.swap)) (+ (Tw.rd) (* 1000 (Tw.swap)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7105 Int64))
  (call main (: 0 Int64))
  (output (: 2100 Int64)))

(case
  "tp3 a NESTED tuple state ((a b) c) — the arm rebuilds the inner Fibonacci pair and bumps the outer counter per dispatch"
  (input
    (do
      (effect Tw (op step (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          Tw
          #tuple(#tuple(n 1) 100)
          ((step
              ()
              s
              (resume
                (+ (. (. s 0) 0) (. s 1))
                #tuple(#tuple((+ (. (. s 0) 0) (. (. s 0) 1)) (. (. s 0) 0)) (+ (. s 1) 1)))))
          (+ (Tw.step) (+ (Tw.step) (Tw.step)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 325 Int64))
  (call main (: 0 Int64))
  (output (: 305 Int64)))

; ── SUM-typed handler states (breaker su) ────────────────────────────────────────────────────────
; The handler state is a SUM whose VARIANT (not just payload) changes across dispatches. su1 =
; Idle→Run transition then payload accumulation. su2 = a THREE-variant cyclic machine, the op arg
; selecting the transition (both Mid exits exercised). su3 = a RECURSIVE sum (Peano tower grown
; by two per dispatch, measured by a recursive fn). su4 = a variant carrying a HEAP list payload.
; su5 = the sum STATE escapes as the handle's value via a dump op, matched OUTSIDE. su6f/su6g =
; MIXED state kinds across a same-effect shadow boundary (scalar/sum each way). su6d pins the
; decline boundary: BOTH handlers sum-state + same effect + >=2 inner dispatches routes the 2nd
; arm copy's variant match through the scalar-probe path (lower.rs) — honest decline, flip
; values 8085/3080 banked for the arm-copy fold fix.
(case
  "su1 a SUM-typed state machine — Idle transitions to Run on first dispatch, Run accumulates thereafter"
  (input
    (do
      (type Mode (Idle) (Run Int64))
      (effect M (op step (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          (Idle)
          ((step (v) s (match s ((Idle) (resume 0 (Run v))) ((Run k) (resume k (Run (+ k v)))))))
          (+ (M.step n) (+ (* 10 (M.step 3)) (* 100 (M.step 1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 850 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

(case
  "su2 a THREE-variant cyclic machine — the op arg selects the transition, both Mid exits exercised"
  (input
    (do
      (type Gear (Lo) (Mid Int64) (HiG Int64))
      (effect G (op shift (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          G
          (Lo)
          ((shift
              (v)
              s
              (match
                s
                ((Lo) (resume 1 (Mid v)))
                ((Mid k) (if (> v k) (resume (* 10 k) (HiG (+ k v))) (resume (- 0 k) (Lo))))
                ((HiG k) (resume (* 100 k) (Lo))))))
          (+ (G.shift n) (+ (G.shift 4) (+ (G.shift 2) (G.shift 9))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 17 Int64))
  (call main (: 1 Int64))
  (output (: 512 Int64)))

(case
  "su3 a RECURSIVE sum state — the arm grows a Peano tower by two per dispatch, a recursive fn measures it"
  (input
    (do
      (type Nat (Z) (S Nat))
      (def (depth (: m Nat)) (match m ((Z) 0) ((S p) (+ 1 (depth p)))))
      (effect T (op grow (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          T
          (Z)
          ((grow () s (resume (depth s) (S (S s)))))
          (+ (T.grow) (+ (* 10 (T.grow)) (* 100 (T.grow))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 420 Int64))
  (live-objects 0))

(case
  "su4 a sum variant carrying a HEAP list — Empty seeds on first put, Full grows the payload thereafter"
  (input
    (do
      (type Buf (Empty) (Full (List Int64)))
      (effect B (op put (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          (Empty)
          ((put
              (v)
              s
              (match
                s
                ((Empty) (resume 0 (Full #list(v))))
                ((Full xs) (resume (List.len xs) (Full (List.push xs v)))))))
          (+ (B.put n) (+ (* 10 (B.put 7)) (* 100 (B.put 8))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 210 Int64)))

(case
  "su5 the sum STATE escapes as the handle's value via a dump op — matched OUTSIDE the handler"
  (input
    (do
      (type Mode (Idle) (Run Int64))
      (effect M (op step (-> Int64 Int64)) (op dump (-> Mode)))
      (def
        (main (: n Int64))
        (match
          (handle
            M
            (Idle)
            ((step (v) s (match s ((Idle) (resume 0 (Run v))) ((Run k) (resume k (Run (+ k v))))))
              (dump () s (resume s s)))
            (do (M.step n) (M.step 3) (M.dump)))
          ((Idle) -1)
          ((Run k) (* 2 k))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 16 Int64))
  (call main (: 0 Int64))
  (output (: 6 Int64)))

(case
  "su6f a SCALAR-state outer shadowed by a SUM-state inner cycler — the inner machine transitions twice"
  (input
    (do
      (type Mode (Idle) (Run Int64))
      (effect M (op step (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          n
          ((step (v) s (resume s (+ s v))))
          (+
            (M.step 1)
            (*
              10
              (handle
                M
                (Idle)
                ((step
                    (v)
                    s
                    (match s ((Idle) (resume 100 (Run (* v 2)))) ((Run k) (resume k (Idle))))))
                (+ (M.step 4) (M.step 0)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1085 Int64))
  (call main (: 0 Int64))
  (output (: 1080 Int64)))

(case
  "su6g a SUM-state outer shadowed by a SCALAR-state inner — mixed state kinds across the shadow boundary"
  (input
    (do
      (type Mode (Idle) (Run Int64))
      (effect M (op step (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          (Run n)
          ((step (v) s (match s ((Idle) (resume 0 (Run v))) ((Run k) (resume k (Run (+ k v)))))))
          (+
            (M.step 1)
            (* 10 (handle M 0 ((step (v) t (resume (+ t v) (+ t 1)))) (+ (M.step 4) (M.step 0)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 55 Int64))
  (call main (: 0 Int64))
  (output (: 50 Int64)))

(case
  "su6d BOTH handlers sum-state, same effect, TWO inner dispatches — declines (2nd arm copy's variant match hits the scalar-probe path)"
  (doc
    "ROOT-CAUSED (v-effects a44, 2026-08-30). Bisected to a minimal reproducer: sum-state handlers
           whose arm matches the state via VARIANT patterns fold FINE in isolation — single handler 1/2
           dispatches, and a NESTED same-effect handler with 1 dispatch each, all compile. The decline needs
           BOTH (a) a nested inner same-effect handle AND (b) the OUTER handler dispatched at a site AFTER
           that inner handle (here `(M.step 2)` follows the inner `(handle M (Idle) …)`). At that post-nested
           outer dispatch the fold re-applies the outer arm, and the outer handler's heap-state SEED let-lift
           (`(let ((#seedNN (Run n))) …)`, `apply_seed_wrap`) does NOT scope over the re-applied arm — so the
           arm's `(match s …)` scrutinee resolves to an UNBOUND `#seedNN` (CDZ0101), whose `type_of` is `Any`,
           which misses the sum-route (`lower.rs` Ty::Sum→`lower_match_sum`) and falls to the scalar-probe
           path, declining `a match pattern that is not a scalar literal or _ is not yet supported`
           (`lower.rs:5747`). So it is NOT a general variant-pattern gap and NOT a type-annotation loss — it is
           the `#seed`/`#st` seed-let-lift SCOPING across a nested same-effect handle (the guarded seam the
           `apply_seed_wrap` forget/graft comments describe). Fix is fold-side (extend the outer seed-let scope
           to cover post-nested-handle outer dispatches) — deep + soundness-critical, deferred to a focused
           session. lower_match_sum genuinely needs the concrete sum type, so there is no lower.rs routing
           shortcut. Workaround: lift the state read to a `let` outside the handle, or avoid straddling a
           nested same-effect handle with outer dispatches.")
  (input
    (do
      (type Mode (Idle) (Run Int64))
      (effect M (op step (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          (Run n)
          ((step (v) s (match s ((Idle) (resume 0 (Run v))) ((Run k) (resume k (Run (+ k v)))))))
          (+
            (M.step 1)
            (+
              (*
                10
                (handle
                  M
                  (Idle)
                  ((step
                      (v)
                      s
                      (match s ((Idle) (resume 100 (Run (* v 2)))) ((Run k) (resume k (Idle))))))
                  (+ (M.step 4) (M.step 0))))
              (* 1000 (M.step 2))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2080 Int64))
  (call main (: 1 Int64))
  (output (: 3081 Int64)))

; ── SET-state gated arms (breaker se) ────────────────────────────────────────────────────────────
; The insert+measure Set-state pin above never GATES on membership; these pin gated dynamics:
; se1 = dedup (re-adding an existing element leaves the size fixed); se2 = a membership-GATED
; arm, the visited-set idiom (first visit admits+records, a revisit answers negated without
; growing); se3 = a DRAIN-style arm (remove on hit — the second take of the same key routes to
; the miss path).
(case
  "se1 a SET handler state with dedup dynamics — re-adding an existing element leaves the size fixed"
  (input
    (do
      (effect Sx (op add (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Sx
          #set(1 2)
          ((add (v) s (resume (Set.len s) (Set.insert s v))))
          (+ (Sx.add n) (+ (* 10 (Sx.add 2)) (* 100 (Sx.add n))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 332 Int64))
  (call main (: 1 Int64))
  (output (: 222 Int64)))

(case
  "se2 a membership-GATED arm — first visit admits and records, a revisit answers negated without growing"
  (input
    (do
      (effect Sx (op visit (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Sx
          #set(3)
          ((visit (v) s (if (Set.contains s v) (resume (- 0 v) s) (resume v (Set.insert s v)))))
          (+ (Sx.visit n) (+ (* 10 (Sx.visit 3)) (* 100 (Sx.visit n))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: -525 Int64))
  (call main (: 3 Int64))
  (output (: -333 Int64)))

(case
  "se3 a DRAIN-style arm removes on hit — the second take of the same key routes to the miss path"
  (input
    (do
      (effect Sx (op take (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          Sx
          #set(1 2 3)
          ((take
              (v)
              s
              (if
                (Set.contains s v)
                (resume (Set.len (Set.remove s v)) (Set.remove s v))
                (resume (* 100 (Set.len s)) s))))
          (+ (Sx.take n) (+ (* 10 (Sx.take n)) (Sx.take 2)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 2003 Int64))
  (call main (: 9 Int64))
  (output (: 3302 Int64)))

; ── short-circuit connectives whose OPERANDS perform (breaker bc) ────────────────────────────────
; The untaken-BRANCH pins forbid a speculative branch perform; these pin the skipped-OPERAND face:
; a short-circuited connective operand must NOT dispatch, and the skip is observable through the
; state the NEXT draw reads. bc1 = AND (false-first skips), bc2 = OR (true-first skips), bc3 = a
; nested and-of-or tree where each call row exercises a distinct skip pattern.
(case
  "bc1 short-circuit AND over two draws — the false-first row skips the second draw, observed by the branch draw"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (if (and (> (St.next) 2) (> (St.next) 4)) (St.next) (- 0 (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7 Int64))
  (call main (: 3 Int64))
  (output (: -5 Int64))
  (call main (: 0 Int64))
  (output (: -1 Int64)))

(case
  "bc2 short-circuit OR over two draws — the true-first row skips the second draw"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (if (or (> (St.next) 4) (> (St.next) 1)) (St.next) (- 0 (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: 2 Int64))
  (output (: 4 Int64))
  (call main (: 0 Int64))
  (output (: -2 Int64)))

(case
  "bc3 a nested and-of-or short-circuit tree over draws — each row exercises a distinct skip pattern"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (if (and (or (> (St.next) 8) (> (St.next) 2)) (> (St.next) 5)) (St.next) (- 0 (St.next)))))
      (export main)))
  (call main (: 9 Int64))
  (output (: 11 Int64))
  (call main (: 4 Int64))
  (output (: 7 Int64))
  (call main (: 1 Int64))
  (output (: -3 Int64))
  (call main (: 0 Int64))
  (output (: -2 Int64)))

; ── draws in literal-ELEMENT and mixed-signature positions (breaker da/sa/rs/mx) ─────────────────
; da = performs as collection-LITERAL elements (list positions, map keys+values, set elements with
; input-selected collisions). sa = STRING boundary crossings (string op args incl. the empty
; string, a draw-branch-derived string arg, string RESUME values concatenated). rs = RECORD-state
; dynamics (per-op field ownership, Record.with functional update in the arm, NESTED records).
; mx = mixed-arity/mixed-TYPE op signatures (Int64+String+Bool in one op, both-args-draw-derived,
; a tuple-arg op chained through its tuple result).
(case
  "da1 THREE draws inside one list literal passed to a helper — left-to-right element order, the post-call draw sees all three"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (sum3 (: xs (List Int64)))
        (match
          (List.at xs 0)
          ((Some a)
            (match
              (List.at xs 1)
              ((Some b) (match (List.at xs 2) ((Some c) (+ a (+ b c))) ((None) 0)))
              ((None) 0)))
          ((None) 0)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (+ (sum3 #list((St.next) (St.next) (St.next))) (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 75 Int64))
  (call main (: 1 Int64))
  (output (: 15 Int64)))

(case
  "da2 draws as MAP-literal keys and values — two entries built from three sequential draws"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (+ (Map.len #map((= (St.next) (St.next)) (= (St.next) 100))) (* 10 (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 82 Int64))
  (call main (: 0 Int64))
  (output (: 32 Int64)))

(case
  "da3 draws as SET-literal elements — n=7 collides the first draw with the fixed element, n=6 collides the second"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (+ s 1))))
          (+ (Set.len #set(7 (St.next) (St.next))) (* 100 (St.next)))))
      (export main)))
  (call main (: 7 Int64))
  (output (: 902 Int64))
  (call main (: 6 Int64))
  (output (: 802 Int64))
  (call main (: 0 Int64))
  (output (: 203 Int64)))

(case
  "sa1 STRING op arguments measured into a scalar state — the empty string is a real zero-length argument"
  (input
    (do
      (effect Log (op tag (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          Log
          n
          ((tag (w) s (resume (+ (String.byte-len w) s) (+ s (String.byte-len w)))))
          (+ (Log.tag "ab") (+ (* 10 (Log.tag "xyz")) (* 100 (Log.tag ""))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1107 Int64))
  (call main (: 0 Int64))
  (output (: 552 Int64)))

(case
  "sa2 a string BUILT from a prior draw's branch becomes the next op's argument — the tag arm reads the post-pick state"
  (input
    (do
      (effect Log (op pick (-> Int64)) (op tag (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          Log
          n
          ((pick () s (resume s (+ s 1))) (tag (w) s (resume (* (String.byte-len w) s) s)))
          (Log.tag (String.concat "id-" (if (> (Log.pick) 3) "long" "s")))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 42 Int64))
  (call main (: 1 Int64))
  (output (: 8 Int64)))

(case
  "sa3 STRING resume values — the arm builds a rope per dispatch branching op-arg vs live state, body concatenates two"
  (input
    (do
      (effect Log (op name (-> Int64 String)))
      (def
        (main (: n Int64))
        (handle
          Log
          n
          ((name (k) s (resume (String.concat "u" (if (> k s) "-big" "-sm")) (+ s 1))))
          (+
            (String.byte-len (Log.name 3))
            (* 10 (String.byte-len (String.concat (Log.name 99) (Log.name 0)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 94 Int64))
  (call main (: 2 Int64))
  (output (: 95 Int64)))

(case
  "rs1 a RECORD state where each op owns a FIELD — bump advances a, scale multiplies b, interleaved"
  (input
    (do
      (effect R (op bump (-> Int64)) (op scale (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          R
          #record((= a n) (= b 3))
          ((bump () s (resume s.a #record((= a (+ s.a 1)) (= b s.b))))
            (scale () s (resume s.b #record((= a s.a) (= b (* s.b 10))))))
          (+ (R.bump) (+ (R.scale) (+ (R.bump) (R.scale))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 44 Int64))
  (call main (: 0 Int64))
  (output (: 34 Int64)))

(case
  "rs2 the arm updates the record state via Record.with — field b is held by the functional update across dispatches"
  (input
    (do
      (effect R (op bump (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          R
          #record((= a n) (= b 100))
          ((bump () s (resume (+ s.a s.b) (Record.with s #"a" (+ s.a 1)))))
          (+ (R.bump) (+ (R.bump) (R.bump)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 318 Int64))
  (call main (: 0 Int64))
  (output (: 303 Int64)))

(case
  "rs3 a NESTED record state — the arm functionally updates the inner record's x by y and bumps the outer counter"
  (input
    (do
      (effect R (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          R
          #record((= inner #record((= x n) (= y 2))) (= cnt 0))
          ((tick
              ()
              s
              (resume
                (+ s.inner.x (* 100 s.cnt))
                #record((= inner (Record.with s.inner #"x" (+ s.inner.x s.inner.y)))
                  (= cnt (+ s.cnt 1))))))
          (+ (R.tick) (+ (R.tick) (R.tick)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 321 Int64))
  (call main (: 0 Int64))
  (output (: 306 Int64)))

(case
  "mx1 a THREE-arg op mixing Int64/String/Bool — one arm consumes all three kinds beside the live state"
  (input
    (do
      (effect E (op mix (-> Int64 String Bool Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((mix (k w f) s (resume (+ (* (if f 10 1) k) (+ (String.byte-len w) s)) (+ s 1))))
          (+ (E.mix 3 "ab" true) (E.mix 4 "xyz" false))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 50 Int64))
  (call main (: 0 Int64))
  (output (: 40 Int64)))

(case
  "mx2 mixed-arg op with BOTH args draw-derived — the int arg is a draw, the string arg branches on a second draw"
  (input
    (do
      (effect E (op pick (-> Int64)) (op mix (-> Int64 String Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((pick () s (resume s (+ s 2))) (mix (k w) s (resume (+ (* k (String.byte-len w)) s) s)))
          (E.mix (E.pick) (if (> (E.pick) 6) "wide" "nn"))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 29 Int64))
  (call main (: 1 Int64))
  (output (: 7 Int64)))

(case
  "mx3 a TUPLE-arg op chained through a tuple RESULT — the second dispatch consumes the first's destructured output"
  (input
    (do
      (effect E (op quo (-> (Tuple Int64 Int64) (Tuple Int64 Int64))))
      (def
        (main (: n Int64))
        (handle
          E
          n
          ((quo (p) s (match p (#tuple(q r) (resume #tuple((+ q s) (* r 2)) (+ s 10))))))
          (match
            (E.quo #tuple(3 4))
            (#tuple(x y) (match (E.quo #tuple(x y)) (#tuple(u v) (+ (* 100 u) v)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2316 Int64))
  (call main (: 0 Int64))
  (output (: 1316 Int64)))

; ── helper-chain depth, extreme values, multi-effect helpers, list transforms (breaker hc/nx/mf/st/lt) ──
; hc = performing HELPER CHAINS (three-deep leaf perform, two depths performing, the same helper
; under two sequential handlers, self-composition, a tuple-returning helper). nx = value EXTREMES
; through the state thread (zero-crossing subtraction, an exact 2^62 seed, sign-branching arms,
; alternating-sign geometric stride, i64::MIN-adjacent op args). mf = helpers performing TWO
; effects (both nesting orders commute; a helper called from an arm's resume-value AND the body).
; st = handle expressions as OPERANDS of enclosing pure sums (one and two sibling regions).
; lt = list-state TRANSFORMS in the arm (end-swap and element-wise doubling via List.update).
(case
  "hc1 a THREE-deep pure helper chain whose LEAF performs — two top-level calls thread the state through the depth"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (leaf (: k Int64)) (+ (St.next) k))
      (def (mid (: k Int64)) (* (leaf k) 2))
      (def (top (: k Int64)) (+ (mid k) 1))
      (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (+ (top 10) (top 100))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 244 Int64))
  (call main (: 0 Int64))
  (output (: 224 Int64)))

(case
  "hc2 helpers at TWO depths both perform — the call-site draw, mid's draw, and leaf's draw arrive in order"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (leaf (: k Int64)) (+ (St.next) k))
      (def (mid (: k Int64)) (+ (* (St.next) 100) (leaf k)))
      (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (mid (St.next))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 612 Int64))
  (call main (: 0 Int64))
  (output (: 102 Int64)))

(case
  "hc3 the SAME performing helper under two SEQUENTIAL handlers — each handle interprets its draws with its own arm and seed"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (twice) (+ (St.next) (St.next)))
      (def
        (main (: n Int64))
        (+
          (handle St n ((next () s (resume s (+ s 1)))) (twice))
          (* 100 (handle St 7 ((next () s (resume s (* s 3)))) (twice)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2811 Int64))
  (call main (: 0 Int64))
  (output (: 2801 Int64)))

(case
  "hc4 a performing helper COMPOSED with itself — probe(probe(draw)), three dispatches through two call frames"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (probe (: k Int64)) (+ (St.next) (* k 10)))
      (def
        (main (: n Int64))
        (handle St n ((next () s (resume s (+ s 1)))) (probe (probe (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 567 Int64))
  (call main (: 0 Int64))
  (output (: 12 Int64)))

(case
  "hc5 a helper RETURNS a tuple of two draws — the caller destructures it and re-performs"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def (pair2) #tuple((St.next) (St.next)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s 2))))
          (match (pair2) (#tuple(a b) (+ (* 100 a) (+ (* 10 b) (St.next)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 620 Int64))
  (call main (: 1 Int64))
  (output (: 124 Int64)))

(case
  "nx1 a SUBTRACTING stride crosses zero — negative states thread and sum correctly"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (- s 7))))
          (+ (St.next) (+ (St.next) (+ (St.next) (St.next))))))
      (export main)))
  (call main (: 10 Int64))
  (output (: -2 Int64))
  (call main (: 0 Int64))
  (output (: -42 Int64))
  (call main (: -5 Int64))
  (output (: -62 Int64)))

(case
  "nx2 a 2^62 seed threads exactly — the difference of consecutive draws recovers the stride"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle St 4611686018427387904 ((next () s (resume s (- s n)))) (- (St.next) (St.next))))
      (export main)))
  (call main (: 1000000007 Int64))
  (output (: 1000000007 Int64))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

(case
  "nx3 the arm branches on the op arg's SIGN — negation on the negative path, n and -n both exercised"
  (input
    (do
      (effect St (op push (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((push (v) s (if (< v 0) (resume (- 0 v) (- s 1)) (resume v (+ s 1)))))
          (+ (St.push n) (+ (* 10 (St.push (- 0 n))) (* 100 (St.push -3))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 355 Int64))
  (call main (: -2 Int64))
  (output (: 322 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

(case
  "nx4 an alternating-sign GEOMETRIC stride (*-2) — the sign flips per dispatch and the sum telescopes"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((next () s (resume s (* s -2))))
          (+ (St.next) (+ (St.next) (+ (St.next) (St.next))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: -15 Int64))
  (call main (: -1 Int64))
  (output (: 5 Int64)))

(case
  "nx5 i64::MIN-adjacent op arguments — the arm's subtraction stays in range and the two dispatches differ by exactly the state stride"
  (input
    (do
      (effect St (op keep (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          0
          ((keep (v) s (resume (- v s) (+ s 1))))
          (- (St.keep -9223372036854775800) (St.keep -9223372036854775800))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "mf1 a helper performing TWO different effects — both handlers discharge it, both states advance per call"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def (both) (+ (A.a) (* 10 (B.b))))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (+ s 1))))
          (handle B 100 ((b () t (resume t (* t 2)))) (+ (both) (both)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3011 Int64))
  (call main (: 0 Int64))
  (output (: 3001 Int64)))

(case
  "mf2 the SAME two-effect helper under the OPPOSITE handler nesting order — distinct effects commute"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def (both) (+ (A.a) (* 10 (B.b))))
      (def
        (main (: n Int64))
        (handle
          B
          100
          ((b () t (resume t (* t 2))))
          (handle A n ((a () s (resume s (+ s 1)))) (+ (both) (both)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3011 Int64))
  (call main (: 0 Int64))
  (output (: 3001 Int64)))

(case
  "mf3 one performing helper called from an ARM's resume-value AND the body — three calls, one advancing thread"
  (input
    (do
      (effect A (op a (-> Int64)))
      (effect B (op b (-> Int64)))
      (def (draw+) (+ (A.a) 1000))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((a () s (resume s (+ s 1))))
          (handle B 0 ((b () t (resume (draw+) t))) (+ (B.b) (+ (draw+) (B.b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3018 Int64))
  (call main (: 0 Int64))
  (output (: 3003 Int64)))

(case
  "st1 a handle expression as ONE operand of an enclosing pure sum — the pure operand and the handled region compose"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (+ (* 1000 n) (handle St n ((next () s (resume s (+ s 1)))) (+ (St.next) (St.next)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5011 Int64))
  (call main (: 0 Int64))
  (output (: 1 Int64)))

(case
  "st2 TWO sibling handle expressions as operands of one sum — independent regions with different arms and seeds"
  (input
    (do
      (effect St (op next (-> Int64)))
      (def
        (main (: n Int64))
        (+
          (handle St n ((next () s (resume s (+ s 1)))) (+ (St.next) (St.next)))
          (* 100 (handle St (* n 10) ((next () s (resume s (- s 2)))) (+ (St.next) (St.next))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 9811 Int64))
  (call main (: 1 Int64))
  (output (: 1803 Int64)))

(case
  "lt1 the arm SWAPS its list state's ends per dispatch — the head alternates between the original ends"
  (input
    (do
      (effect L (op spin (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          L
          #list(n 2 9)
          ((spin
              ()
              s
              (resume
                (match (List.at s 0) ((Some h) h) ((None) -1))
                (match
                  (List.at s 0)
                  ((Some h)
                    (match
                      (List.at s 2)
                      ((Some t) (List.update (List.update s 0 t) 2 h))
                      ((None) s)))
                  ((None) s)))))
          (+ (L.spin) (+ (* 10 (L.spin)) (* 100 (L.spin))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 595 Int64))
  (call main (: 0 Int64))
  (output (: 90 Int64)))

(case
  "lt2 the arm DOUBLES every list element per dispatch (via a projection helper) — the sum geometrically grows"
  (input
    (do
      (effect L (op amp (-> Int64)))
      (def (el (: s (List Int64)) (: i Int64)) (match (List.at s i) ((Some v) v) ((None) 0)))
      (def
        (main (: n Int64))
        (handle
          L
          #list(n 3)
          ((amp
              ()
              s
              (resume
                (+ (el s 0) (el s 1))
                (List.update (List.update s 0 (* (el s 0) 2)) 1 (* (el s 1) 2)))))
          (+ (L.amp) (+ (* 10 (L.amp)) (* 100 (L.amp))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3368 Int64))
  (call main (: 0 Int64))
  (output (: 1263 Int64)))

; ── BYTES handler states + std-Option state/resume coverage (breaker by/op) ──────────────────────
; by = a BYTES buffer as the threaded state: growth per dispatch, a SLICE view (start,LEN) read
; with in-range and out-of-range rows, op-arg values WRAPPED to bytes at runtime, and nested
; SAME-effect handlers with independent buffers (outer self-doubles, inner appends) — completing
; the shadow state-kind matrix (scalar/list/string/sum/bytes). op = the STD Option: op1 threads
; it as the STATE (None seeds to Some, payload accumulates — the std twin of the custom-sum
; machine); op2 resumes an OPTION value from a single-site arm (Some and None rows both
; exercised; the two-site-resume x Option-value conjunction stays not-yet-reducible).
(case
  "by1 a BYTES handler state grows two bytes per dispatch — each arm returns the pre-growth length"
  (input
    (do
      (effect B (op put (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          (Bytes.of #list((UInt8.wrap 65)))
          ((put
              (v)
              s
              (resume
                (Bytes.len s)
                (Bytes.concat s (Bytes.of #list((UInt8.wrap 66) (UInt8.wrap 67)))))))
          (+ (B.put n) (+ (* 10 (B.put n)) (* 100 (B.put n))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 531 Int64)))

(case
  "by2 a SLICE view (start,LEN) of the Bytes state read per dispatch — in-range bytes returned, out-of-range answers -1"
  (input
    (do
      (effect B (op peek (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          (Bytes.of #list((UInt8.wrap 10) (UInt8.wrap 20) (UInt8.wrap 30) (UInt8.wrap 40)))
          ((peek
              (i)
              s
              (resume
                (match
                  (Bytes.slice s 1 2)
                  ((Some sl) (match (Bytes.at sl i) ((Some b) (Int64.of b)) ((None) -1)))
                  ((None) -99))
                s)))
          (+ (B.peek n) (+ (* 10 (B.peek 1)) (* 100 (B.peek 2))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 220 Int64))
  (call main (: 1 Int64))
  (output (: 230 Int64)))

(case
  "by3 op-arg values WRAPPED to bytes at runtime accumulate in the state — the fourth emit sees three prior"
  (input
    (do
      (effect B (op emit (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          (Bytes.of #list())
          ((emit (v) s (resume (Bytes.len s) (Bytes.concat s (Bytes.of #list((UInt8.wrap v)))))))
          (do (B.emit n) (B.emit 77) (B.emit 200) (B.emit 0))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (call main (: 255 Int64))
  (output (: 3 Int64)))

(case
  "by4 nested SAME-effect handlers with independent BYTES states — the outer buffer self-doubles, the inner appends"
  (input
    (do
      (effect B (op put (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          (Bytes.of #list((UInt8.wrap 1)))
          ((put () s (resume (Bytes.len s) (Bytes.concat s s))))
          (+
            (B.put)
            (+
              (*
                10
                (handle
                  B
                  (Bytes.of #list((UInt8.wrap 9) (UInt8.wrap 8) (UInt8.wrap 7)))
                  ((put
                      ()
                      t
                      (resume (Bytes.len t) (Bytes.concat t (Bytes.of #list((UInt8.wrap 0)))))))
                  (+ (B.put) (B.put))))
              (* 1000 (B.put))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2071 Int64)))

(case
  "op1 the STD Option as handler state — None seeds to Some on first feed, the payload accumulates thereafter"
  (input
    (do
      (effect O (op feed (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          O
          (None)
          ((feed (v) s (match s ((None) (resume 0 (Some v))) ((Some k) (resume k (Some (+ k v)))))))
          (+ (O.feed n) (+ (* 10 (O.feed 3)) (* 100 (O.feed 1))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 850 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

(case
  "a match-discard chain sequencing 3 dispatches over LIST-OF-LISTS state declines cleanly (let-chain twin folds)"
  (doc
    "SEQUENCING-FORM FENCE (breaker list-of-lists-state, 2026-08-11): the same program that FOLDS when the
           three `Rows.add` dispatches are sequenced with a `let`-chain (the ll1 green pin in 14c) DECLINES
           (honest `not yet reducible` todo) when they are sequenced with a wildcard `(match (Rows.add ..) (_
           ...))` discard chain at depth 3. The tail-resumptive fold tracks the wildcard-match discard
           SEQUENCING form only to depth 2 (two chained discards fold); depth 3+ falls to a clean decline.
           `let`-binding is the general sequencing form and folds at ANY depth — so this is a pure coverage gap
           in the match-discard tracking, not a soundness issue. This case is a REJECT-DON'T-MISCOMPILE sentinel:
           the pinned value (identical to the ll1 let-chain twin, 400800 / 100200) is what the fold MUST produce
           if it ever folds this shape; until the match-discard tracking extends past depth 2 it declines
           cleanly. If a future fold change turns this into a SILENT WRONG VALUE (dropping a dispatch's
           out-state across a discard) rather than a decline-or-correct, this case goes red. Flips todo->pass
           when the wildcard-match discard-chain sequencing is tracked past depth 2.")
  (input
    (do
      (effect Rows (op add (-> Int64 Int64)) (op pick (-> Int64 Int64 Int64)))
      (def
        (row-at (: xss (List (List Int64))) (: i Int64) (: j Int64))
        (match
          (List.at xss i)
          ((Some xs) (match (List.at xs j) ((Some v) v) ((None _u) -1)))
          ((None _u) -2)))
      (def
        (main (: n Int64))
        (handle
          Rows
          #list()
          ((add (v) s (resume (List.len s) (List.push s #list(v (* v 10) (* v 100)))))
            (pick (i j) s (resume (row-at s i j) s)))
          (match
            (Rows.add n)
            (_
              (match
                (Rows.add (+ n 1))
                (_
                  (match
                    (Rows.add (+ n 2))
                    (_ (+ (* 10000 (Rows.pick 1 1)) (+ (* 100 (Rows.pick 2 0)) (Rows.pick 0 2)))))))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 400800 Int64))
  (call main (: 0 Int64))
  (output (: 100200 Int64)))

(case
  "a MIXED mutual SCC where only ONE partner performs — the non-performing arithmetic leg threads through the group fold"
  (doc
    "The mixed-SCC face of the group multi-value fold: `pa` and `pb` mutually recurse, but only `pa`
           performs (the base case `(S.tick)`); `pb` is pure arithmetic wrapping the cycle
           `(let ((child (pa (- n 1)))) (+ child (* 2 n)))`. The landed mutual-group pins have BOTH partners
           performing (14b `a mutual walk where BOTH partners perform ...`); this pins that the group fold
           also threads the value correctly when the effect is reached through only ONE partner and the OTHER
           leg is non-performing arithmetic that consumes the recursion result. `main(3)`: pa(0) draws the
           seed (tick resumes s->s, seed 0) = 0; pb(1) = 0 + 2*1 = 2; pb(2) = 2 + 2*2 = 6; pb(3) = 6 + 2*3 =
           12. Uniform across all 3 backends and stable across O0..O3 (opt-sweep 0-divergence). Breaker
           mixed-scc probe mx1 (2026-08-11). FENCE (documented, not pinned here): if a TRAILING draw observes
           the SCC's post-recursion out-state advance, it stays behind the fold frontier and declines cleanly
           — the same mutual-SCC-out-state completeness gap pinned at 14c `a mutually-recursive performer pair
           whose out-state a trailing caller draw observes declines cleanly`.")
  (input
    (do
      (effect S (op tick (-> Unit Int64)) (op put (-> Int64 Unit)))
      (def (pa (: n Int64)) (if (= n 0) (S.tick) (pb n)))
      (def (pb (: n Int64)) (let ((child (pa (- n 1)))) (+ child (* 2 n))))
      (def
        (main (: k Int64))
        (handle S 0 ((tick (u) s (resume s s)) (put (v) s (resume unit (+ s v)))) (pa k)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 12 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "a CONDITIONALLY-recursive helper reaching a performer in one branch folds where no outer recursion needs the boundary guard"
  (doc
    "The precision companion of finding #19: `inner` is a self-recursive performer (`S.tick`/hop);
           `maybe` CONDITIONALLY reaches it — `(if (= go 1) (inner 2 0) 55)`, one branch recurses into the
           performer, the other is a constant. Called from body position (NOT under an outer recursion), both
           branches exercised across two calls `(+ (maybe 1) (* 1000 (maybe 0)))`. The finding-#19
           recursion-boundary guard flags a helper by its POTENTIAL transitive reach (a sound
           over-approximation — s19h under an OUTER recursion declines conservatively even when the taken
           branch is pure); this pins that WITHOUT an outer-recursion boundary the conditional helper folds
           precisely. `main(1)`: maybe(1)=inner(2,0): tick@s=1->1 (s->2), tick@s=2->2 (s->3), inner=0+1+2=3;
           maybe(0)=55; 3 + 1000*55 = 55003. `main(0)`: seed 0 → inner=0+0+1=1; 1 + 55000 = 55001. Uniform on
           all 3 backends, stable across O0..O3 (opt-sweep 0-divergence). Breaker conditional-reach probe ci1
           (2026-08-11).")
  (input
    (do
      (effect S (op tick (-> Int64)))
      (def (inner (: k Int64) (: acc Int64)) (if (< k 1) acc (inner (- k 1) (+ acc (S.tick)))))
      (def (maybe (: go Int64)) (if (= go 1) (inner 2 0) 55))
      (def
        (main (: n Int64))
        (handle S n ((tick () s (resume s (+ s 1)))) (+ (maybe 1) (* 1000 (maybe 0)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 55003 Int64))
  (call main (: 0 Int64))
  (output (: 55001 Int64)))

(case
  "an inner handler arm performs the OUTER effect mid-transition — both threads advance in lockstep per inner dispatch"
  (doc
    "Cross-arm perform where the inner handler B's arm computes its answer by performing the OUTER
           effect A `(gb () t (resume (+ t (A.ga)) (+ t 10)))`. The landed 51-forward pins A.ga in an inner
           arm's resume-VALUE via a helper; this pins the mid-TRANSITION face — the outer perform is inside the
           arm's answer computation, so EACH inner B.gb dispatch advances BOTH handler threads (A's s and B's
           t) in lockstep. Arm bodies resolve under the handlers enclosing THEIR handle, so B's arm (nested
           inside A) can see A. `main(3)`: A seed 3, B seed 100. 1st B.gb: t=100, answer 100 + A.ga(s=3->3,
           s->4) = 103, t->110; 2nd B.gb: t=110, answer 110 + A.ga(s=4->4, s->5) = 114, t->120; body
           `(+ (B.gb) (* 1000 (B.gb)))` = 103 + 1000*114 = 114103. Uniform x3, opt-sweep 0-divergence. Breaker
           nesting-order probe no1 (2026-08-11); its order-flipped twin below pins the scoping rule this relies
           on.")
  (input
    (do
      (effect A (op ga (-> Int64)))
      (effect B (op gb (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          A
          n
          ((ga () s (resume s (+ s 1))))
          (handle B 100 ((gb () t (resume (+ t (A.ga)) (+ t 10)))) (+ (B.gb) (* 1000 (B.gb))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 114103 Int64))
  (call main (: 0 Int64))
  (output (: 111100 Int64)))

(case
  "a cross-arm perform with the handler nesting FLIPPED has no home — arm bodies resolve under the handlers enclosing their handle"
  (doc
    "The order-flipped REJECT twin of the cross-arm-perform pin above. The SAME program with the nesting
           FLIPPED (B OUTSIDE, A inside) is CDZ0401: B's arm body `(resume (+ t (A.ga)) ...)` performs A, but
           an arm body resolves under the handlers enclosing ITS OWN handle — B's handle encloses nothing that
           handles A (A's handle is INSIDE B's body, not around B's arm), so A.ga in B's arm has no home. Pins
           the scoping rule the green mid-transition case relies on: a cross-arm perform of an outer effect is
           legal only when the performing arm's handle is nested INSIDE the target effect's handler. Breaker
           nesting-order probe no2 (2026-08-11).")
  (input
    (do
      (effect A (op ga (-> Int64)))
      (effect B (op gb (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          B
          100
          ((gb () t (resume (+ t (A.ga)) (+ t 10))))
          (handle A n ((ga () s (resume s (+ s 1)))) (+ (B.gb) (* 1000 (B.gb))))))
      (export main)))
  (error CDZ0401))

(case
  "THREE performs nested in one expression — each op's result is the next op's argument, three distinct arms and strides"
  (doc
    "Deeper than the landed 2-deep tuple-projection pin: three distinct ops nested in ONE expression
           `(S.c (S.b (S.a 5)))`, each op's result flowing as the next op's argument, plus a fourth trailing
           dispatch — three arms with different state strides (a: +1, b: +10, c: +100) threaded through one
           spine. Argument-position evaluation order must be exact (innermost perform first). S seeded n, arms
           `a (v) s -> resume (+ v s) (+ s 1)`, `b (v) s -> resume (* v 2) (+ s 10)`, `c (v) s -> resume (- v s)
           (+ s 100)`. `main(3)`: S.a 5 @s=3 -> 5+3=8, s->4; S.b 8 @s=4 -> 16, s->14; S.c 16 @s=14 -> 16-14=2,
           s->114; S.a 1 @s=114 -> 1+114=115, s->115; body `(+ (S.c (S.b (S.a 5))) (* 10000 (S.a 1)))` =
           2 + 10000*115 = 1150002. Uniform on all 3 backends, opt-sweep 0-divergence. Breaker
           nested-perform-compose probe nc1 (2026-08-11). (The middle-op-aborts face nc2 declines, banked — the
           1-dispatch-before-abort fence.)")
  (input
    (do
      (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((a (v) s (resume (+ v s) (+ s 1)))
            (b (v) s (resume (* v 2) (+ s 10)))
            (c (v) s (resume (- v s) (+ s 100))))
          (+ (S.c (S.b (S.a 5))) (* 10000 (S.a 1)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 1150002 Int64))
  (call main (: 0 Int64))
  (output (: 1119999 Int64)))

(case
  "the recursion DEPTH itself is a draw — the walk runs mod-4-of-state iterations, including the zero-depth face"
  (doc
    "The recursion bound is DATA from a perform, not a literal: `(let ((d (S.depth))) (+ (* 100000 d)
           (walk d 0)))` — `S.depth` resumes `(% s 4)` so the walk's iteration count is drawn from the handler
           state, and the depth draw ALSO advances the state that the subsequent `walk`'s per-hop `S.tick`
           reads. Covers the zero-depth face (when the draw is 0 the walk never performs and the fold must
           still thread cleanly). depth arm `(resume (% s 4) (+ s 1))`, tick arm `(resume s (+ s 1))`.
           `main(6)`: d = 6%4 = 2 (s->7); walk(2,0): tick@7->7 acc=7 (s->8), tick@8->8 acc=78 (s->9); 100000*2
           + 78 = 200078. `main(4)`: d = 0 (zero-depth, walk never ticks) → 0. `main(5)`: d = 1 (s->6);
           walk(1,0): tick@6->6 acc=6; 100000*1 + 6 = 100006. Uniform on all 3 backends, opt-sweep
           0-divergence. Breaker draw-determined-depth probe sd2 (2026-08-11).")
  (input
    (do
      (effect S (op depth (-> Int64)) (op tick (-> Int64)))
      (def (walk (: k Int64) (: acc Int64)) (if (< k 1) acc (walk (- k 1) (+ (* 10 acc) (S.tick)))))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((depth () s (resume (% s 4) (+ s 1))) (tick () s (resume s (+ s 1))))
          (let ((d (S.depth))) (+ (* 100000 d) (walk d 0)))))
      (export main)))
  (call main (: 6 Int64))
  (output (: 200078 Int64))
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 100006 Int64)))

(case
  "a SEVEN-op handler — the widest arm table in the corpus, each op a distinct answer shape and stride, order scrambled"
  (doc
    "Op-COUNT scaling of one handler's dispatch table: the landed handlers top out around 3-4 ops; this
           pins SEVEN ops with distinct answer shapes (+1 / *2 / -3 / square / mod / div / negate) and distinct
           state strides (+1..+7), performed in SCRAMBLED order (o1,o3,o5,o7,o2,o4,o6) at seven positional
           weights. A mis-wired dispatch index or a shared arm slot would perturb the positional sum. Each arm
           advances the shared state by its own stride. `main(2)`: o1@s=2 -> 3 (s->3); o3@s=3 -> 0 (s->6);
           o5@s=6 -> 1 (s->11); o7@s=11 -> -11 (s->18); o2@s=18 -> 36 (s->20); o4@s=20 -> 400 (s->24); o6@s=24
           -> 12 (s->30); body sums 3 + 10*0 + 100*1 + 1000*(-11) + 10000*36 + 100000*400 + 10000000*12 =
           160349103. Uniform on all 3 backends, opt-sweep 0-divergence. Breaker wide-arm-table probe w7
           (2026-08-11).")
  (input
    (do
      (effect
        W
        (op o1 (-> Int64))
        (op o2 (-> Int64))
        (op o3 (-> Int64))
        (op o4 (-> Int64))
        (op o5 (-> Int64))
        (op o6 (-> Int64))
        (op o7 (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          W
          n
          ((o1 () s (resume (+ s 1) (+ s 1)))
            (o2 () s (resume (* s 2) (+ s 2)))
            (o3 () s (resume (- s 3) (+ s 3)))
            (o4 () s (resume (* s s) (+ s 4)))
            (o5 () s (resume (% s 5) (+ s 5)))
            (o6 () s (resume (/ s 2) (+ s 6)))
            (o7 () s (resume (- 0 s) (+ s 7))))
          (+
            (W.o1)
            (+
              (* 10 (W.o3))
              (+
                (* 100 (W.o5))
                (+ (* 1000 (W.o7)) (+ (* 10000 (W.o2)) (+ (* 100000 (W.o4)) (* 10000000 (W.o6))))))))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 160349103 Int64))
  (call main (: 0 Int64))
  (output (: 142711381 Int64)))

(case
  "a TUPLE-keyed Map STATE grown across dispatches — compound-key inserts and lookups thread, the flipped key misses"
  (doc
    "The STATE face of tuple-keyed Maps: the landed tuple-key pin crosses a tuple-keyed Map as an op
           RESULT in one dispatch; here the Map IS the threaded handler state, grown by `mark` (insert
           `(tuple x y) -> (+ x y)`) and read by `check` (lookup `(tuple x y)`) across dispatches. Structural
           tuple-key equality must thread through the state so a later lookup hits an earlier insert, and a
           FLIPPED key `(tuple 2 1)` vs the inserted `(tuple 1 2)` correctly MISSES (distinct compound keys).
           `mark` resumes `(Map.len s)` (pre-insert size). `main(3)`: mark(1,2) inserts (1,2)->3; mark(3,4)
           inserts (3,4)->7; check(1,2)=3, check(3,4)=7, check(2,1)=MISS=-1; body
           `(+ (* 10000 3) (+ (* 100 7) -1))` = 30000 + 700 - 1 = 30699. Uniform on all 3 backends, opt-sweep
           0-divergence. Breaker tuple-key-state probe tk1 (2026-08-11).")
  (input
    (do
      (effect S (op mark (-> Int64 Int64 Int64)) (op check (-> Int64 Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          Map.empty
          ((mark (x y) s (resume (Map.len s) (Map.insert s #tuple(x y) (+ x y))))
            (check
              (x y)
              s
              (resume (match (Map.lookup s #tuple(x y)) ((Some v) v) ((None _u) -1)) s)))
          (let
            ((_a (S.mark 1 2)))
            (let
              ((_b (S.mark n 4)))
              (+ (* 10000 (S.check 1 2)) (+ (* 100 (S.check n 4)) (S.check 2 1)))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 30699 Int64))
  (call main (: 1 Int64))
  (output (: 30499 Int64)))

(case
  "a DEPTH-3 same-effect shadow ladder — each shadow's seed draws from the ENCLOSING handler, strides 1 2 3 stay separate"
  (doc
    "Cross-seeded same-effect shadowing: three nested handlers for the SAME effect St, but unlike the
           literal-seed towers (dn1), each inner shadow's SEED is COMPUTED by drawing from the ENCLOSING
           handler — `(handle St (* (St.get) 10) ...)` seeds the middle from the outer's first draw, and
           `(handle St (+ (St.get) 5) ...)` seeds the inner from the middle's first draw. Each level's arm has
           its own stride (outer +1, middle +2, inner +3) and its own state slot; the body's two draws both
           home to the innermost. `main(3)`: outer seed 3, its first draw (for the middle seed) = 3 (s->4),
           middle seed = 30; middle's first draw (for the inner seed) = 30 (s->32), inner seed = 35; body
           `(+ (St.get) (* 100 (St.get)))` on the inner: 35 (s->38) + 100*38 = 35 + 3800 = 3835. The strides
           stay separate — no slot bleeds across the ladder. Uniform on all 3 backends, opt-sweep 0-divergence.
           Breaker shadow-ladder probe sl1 (2026-08-11).")
  (input
    (do
      (effect St (op get (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          St
          n
          ((get () s (resume s (+ s 1))))
          (handle
            St
            (* (St.get) 10)
            ((get () t (resume t (+ t 2))))
            (handle St (+ (St.get) 5) ((get () u (resume u (+ u 3)))) (+ (St.get) (* 100 (St.get)))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 3835 Int64))
  (call main (: 0 Int64))
  (output (: 805 Int64)))

(case
  "an Option built by one dispatch is STORED into the tuple state and drained by a later dispatch — Some and None both persist"
  (doc
    "Option-valued handler STATE, not just Option-valued resume: the threaded state is a
           `(tuple counter (Option Int64))`. `stash` bin-matches the state, computes a state-dependent
           `Some`/`None`, and installs it into the tuple's Option slot (resuming a dummy); a LATER `read`
           dispatch pulls that Option back out of the state and unwraps it. The Option must survive the state
           thread intact across the intervening dispatches — Some and None both persist. `find` is the direct
           Option-resume companion (verdict from the state comparison). `main(5)`: state (5, None); stash(2):
           2<5 -> store Some(102); read -> unwrap Some(102) = 102; find(9): 9<5 false -> None -> unwrap-or -3;
           body `(+ (* 100 102) -3)` = 10197. `main(1)`: state (1, None); stash(2): 2<1 false -> store None;
           read -> unwrap-or None -7; find(9): 9<1 false -> None -> -3; `(+ (* 100 -7) -3)` = -703. Uniform on
           all 3 backends, opt-sweep 0-divergence. Breaker option-result-branch probe op2 (2026-08-11).")
  (input
    (do
      (effect
        S
        (op find (-> Int64 (Option Int64)))
        (op stash (-> Int64 Int64))
        (op read (-> Int64)))
      (def (unwrap-or (: o (Option Int64)) (: d Int64)) (match o ((Some v) v) ((None _u) d)))
      (def
        (main (: n Int64))
        (handle
          S
          #tuple(n (: (None unit) (Option Int64)))
          ((find
              (k)
              st
              (match
                st
                (#tuple(s o)
                  (resume (if (< k s) (Some (+ k 100)) (: (None unit) (Option Int64))) st))))
            (stash
              (k)
              st
              (match
                st
                (#tuple(s _o)
                  (resume 0 #tuple(s (if (< k s) (Some (+ k 100)) (: (None unit) (Option Int64))))))))
            (read () st (match st (#tuple(_s o) (resume (unwrap-or o -7) st)))))
          (let ((_a (S.stash 2))) (+ (* 100 (S.read)) (unwrap-or (S.find 9) -3)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10197 Int64))
  (call main (: 1 Int64))
  (output (: -703 Int64)))

(case
  "NEGATIVE and HUGE indices cross the dispatch — List.at in the arm answers None-fallback for both out-of-range directions"
  (doc
    "An out-of-range index crossing the dispatch into an arm-side `List.at` must answer the None
           fallback in BOTH directions. The op `at` takes an index that flows into `(List.at s i)` where the
           handler state `s` is a 3-element list; a negative index and a huge positive index both miss. This
           guards the index marshal across the effect boundary: a truncating or sign-confused marshal would
           fold an out-of-range index into a valid slot. `main(1)`: at(1)=20, at(-1)=None=-7, at(99)=None=-7;
           `(+ (* 10000 20) (+ (* 100 -7) -7))` = 200000 - 700 - 7 = 199293. `main(-5)`: at(-5)=-7, at(-1)=-7,
           at(99)=-7 = -70707. Uniform on all 3 backends, opt-sweep 0-divergence. Breaker index-edge-faces
           probe nx1 (2026-08-11); its i64-extreme twin below pins the MAX/MIN faces.")
  (input
    (do
      (effect S (op at (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          #list(10 20 30)
          ((at (i) s (resume (match (List.at s i) ((Some v) v) ((None _u) -7)) s)))
          (+ (* 10000 (S.at n)) (+ (* 100 (S.at -1)) (S.at 99)))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 199293 Int64))
  (call main (: -5 Int64))
  (output (: -70707 Int64)))

(case
  "i64-EXTREME indices — MAX and MIN as List.at arguments both answer the None fallback; a truncating index marshal would wrap into range"
  (doc
    "The sharp face of the index-marshal guard: i64 MAX (9223372036854775807) and i64 MIN
           (-9223372036854775808) as `List.at` indices through the dispatch. A truncating i64->i32 index
           marshal would wrap MAX to -1 and MIN to 0 — folding both into (or adjacent to) the valid range and
           silently returning an in-range element instead of the None fallback. Both must answer None (-7).
           `main(0)`: at(MAX)=None=-7, at(MIN)=None=-7; `(+ (* 1000 -7) -7)` = -7007. Uniform on all 3 backends
           AND stable across O0..O3 (opt-sweep 0-divergence) — the sweep is the real guard, since a wrap could
           be introduced by a specific opt level. Breaker index-edge-faces probe nx2 (2026-08-11).")
  (input
    (do
      (effect S (op at (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          #list(10 20 30)
          ((at (i) s (resume (match (List.at s i) ((Some v) v) ((None _u) -7)) s)))
          (+ (* 1000 (S.at 9223372036854775807)) (S.at -9223372036854775808))))
      (export main)))
  (call main (: 0 Int64))
  (output (: -7007 Int64)))

(case
  "the STATE is the divisor — a descending thread crosses zero and the exact dispatch that hits it traps"
  (doc
    "Dividing the op VALUE by the THREADED STATE, with the state descending toward zero so the exact
           dispatch that lands on divisor 0 traps divide-by-zero. The landed div pins divide by op args or
           constants; dividing BY the threaded state (which the arm advances) is the uncovered direction, and
           this lands the trap ON the state thread. `div` arm resumes `(/ v s)` and steps `s -> s-1`.
           `main(5)`: div(100)@s=5 = 20 (s->4); div(100)@s=4 = 25 (s->3); `(+ 20 (* 1000 25))` = 25020.
           `main(2)`: div(100)@s=2 = 50 (s->1); div(100)@s=1 = 100 (s->0); `(+ 50 (* 1000 100))` = 100050.
           `main(1)`: div(100)@s=1 = 100 (s->0); div(100)@s=0 -> DIVIDE BY ZERO trap. Uniform on all 3 backends
           incl. the trap, opt-sweep 0-divergence. Breaker state-divisor probe dv1 (2026-08-11).")
  (input
    (do
      (effect S (op div (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((div (v) s (resume (/ v s) (- s 1))))
          (let ((a (S.div 100))) (let ((b (S.div 100))) (+ a (* 1000 b))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 25020 Int64))
  (call main (: 2 Int64))
  (output (: 100050 Int64))
  (call main (: 1 Int64))
  (trap "divide by zero"))

(case
  "INT_MIN divided by the state — the minus-one seed overflows, other signs give exact halves"
  (doc
    "The OTHER i64 division trap through the state thread: INT_MIN / state. When the state is -1,
           `INT_MIN / -1` overflows i64 (the quotient +2^63 is unrepresentable) and traps integer overflow;
           other divisors give exact halves. `div` arm resumes `(/ v s)`, steps `s -> s+1`, dividing the fixed
           INT_MIN dividend by the seed. `main(2)`: INT_MIN / 2 = -4611686018427387904. `main(-2)`: INT_MIN /
           -2 = 4611686018427387904 (exact, no overflow — |result| < 2^63). `main(-1)`: INT_MIN / -1 -> INTEGER
           OVERFLOW trap. Uniform on all 3 backends incl. the trap, opt-sweep 0-divergence. Breaker
           state-divisor probe dv2 (2026-08-11); the divide-by-zero twin is above.")
  (input
    (do
      (effect S (op div (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle S n ((div (v) s (resume (/ v s) (+ s 1)))) (S.div -9223372036854775808)))
      (export main)))
  (call main (: 2 Int64))
  (output (: -4611686018427387904 Int64))
  (call main (: -2 Int64))
  (output (: 4611686018427387904 Int64))
  (call main (: -1 Int64))
  (trap "integer overflow"))

(case
  "BINARY SEARCH against a hidden state target — the body bisects on the arm's ordering verdicts, eight probes find any target in 0..100"
  (doc
    "An oracle-driven control-flow shape over effect state: the handler state `n` is the hidden search
           TARGET, and the `cmp` arm answers a 3-way ordering verdict against it — `(if (< v s) 1 (if (> v s)
           -1 0))`. The body `bisect` recurses, halving `[lo,hi]` on each verdict (8 probes bound the search over
           0..100), so the RECURSION PATH is data-dependent on the hidden target — every seed drives a distinct
           sequence of dispatches through the fold. Verifies the tail-resumptive fold serves a real
           oracle-search program where the answer flows back and steers control. Four seeds exercise distinct
           paths incl. the 0 and 100 BOUNDARY faces: `main(37)`=37, `main(0)`=0, `main(100)`=100, `main(63)`=63.
           Uniform on all 3 backends, opt-sweep 0-divergence. Breaker ordering-verdicts probe cp2 (2026-08-11).")
  (input
    (do
      (effect S (op cmp (-> Int64 Int64)))
      (def
        (bisect (: lo Int64) (: hi Int64) (: k Int64))
        (if
          (< k 1)
          -1
          (let
            ((mid (/ (+ lo hi) 2)))
            (let
              ((c (S.cmp mid)))
              (if
                (= c 0)
                mid
                (if (< c 0) (bisect lo (- mid 1) (- k 1)) (bisect (+ mid 1) hi (- k 1))))))))
      (def
        (main (: n Int64))
        (handle S n ((cmp (v) s (resume (if (< v s) 1 (if (> v s) -1 0)) s))) (bisect 0 100 8)))
      (export main)))
  (call main (: 37 Int64))
  (output (: 37 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 100 Int64))
  (output (: 100 Int64))
  (call main (: 63 Int64))
  (output (: 63 Int64)))

(case
  "remainder BY the state — truncated dividend-sign semantics hold as the state walks through negative and positive divisors"
  (doc
    "The remainder companion of the state-divisor pins: `(% v s)` where the DIVISOR is the threaded
           state `s` (which the arm advances `s -> s+3`, so it crosses from negative through positive across
           dispatches). i64 `%` truncates toward zero, taking the SIGN OF THE DIVIDEND regardless of the
           divisor's sign — this must hold as the state-divisor changes sign. `rem` arm resumes `(% v s)`.
           `main(3)`: a = (-7 % 3) = -1 (s->6); b = (7 % 6) = 1; `(+ -1 (* 100 1))` = 99. `main(-5)`: a =
           (-7 % -5) = -2 (negative divisor, dividend-sign result, s->-2); b = (7 % -2) = 1; `(+ -2 (* 100 1))`
           = 98. `main(2)`: a = (-7 % 2) = -1 (s->5); b = (7 % 5) = 2; `(+ -1 (* 100 2))` = 199. Distinct from
           the fixed-divisor negative-remainder pin (nm1): here the DIVISOR is the walking state. Uniform on
           all 3 backends, opt-sweep 0-divergence. Breaker state-remainder probe rm1 (2026-08-11).")
  (input
    (do
      (effect S (op rem (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((rem (v) s (resume (% v s) (+ s 3))))
          (let ((a (S.rem -7))) (let ((b (S.rem 7))) (+ a (* 100 b))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 99 Int64))
  (call main (: -5 Int64))
  (output (: 98 Int64))
  (call main (: 2 Int64))
  (output (: 199 Int64)))

(case
  "a state byte built into a Bytes and decoded by a from-bytes match in a tail-resumptive arm folds"
  (doc
    "The inline (no-let) face of finding #20 (breaker via corpus-bugfix): the handler state byte is wrapped
           into a one-element `Bytes`, decoded through a `String.from-bytes` MATCH in the resume value, and the
           two arms (valid Some / invalid None) both cross the dispatch. `dec` arm resumes `(match
           (String.from-bytes (Bytes.of (list (UInt8.wrap s)))) ((Some t) (String.byte-len t)) ((None _u) -1))`.
           `main(65)`: 65 = ASCII 'A', a valid 1-byte UTF-8 string → byte-len 1. `main(200)`: 200 = 0xC8, a lone
           continuation byte → invalid UTF-8 → None → -1. Uniform on all 3 backends, opt-sweep 0-divergence.
           (The LET-BOUND twin — let-binding the Bytes first — DECLINES cleanly on current trunk, a
           reject-don't-miscompile fold-frontier todo, NOT an ICE; this inline form is the passing sentinel.)
           Breaker finding-20 inline twin (2026-08-12).")
  (input
    (do
      (effect S (op dec (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((dec
              ()
              s
              (resume
                (match
                  (String.from-bytes (Bytes.of #list((UInt8.wrap s))))
                  ((Some t) (String.byte-len t))
                  ((None _u) -1))
                s)))
          (S.dec)))
      (export main)))
  (call main (: 65 Int64))
  (output (: 1 Int64))
  (call main (: 200 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "the LET-BOUND twin of finding #20 — a state byte LET-bound as a Bytes then decoded by a from-bytes match in a tail-resumptive arm folds (flatten+pin fix landed; regression-guard)"
  (doc
    "adv-20 (breaker via corpus-bugfix, v-effects lane): the LET-BOUND face of the inline sentinel directly
           above. The handler state byte is FIRST let-bound into a one-element `Bytes` — `(let ((b (Bytes.of
           (list (UInt8.wrap s))))) …)` — then that binder `b` is consumed by a `String.from-bytes` decode-MATCH
           in the resume value. Same two arms (valid Some / invalid None), same expected results as the inline
           twin: `main(65)` → 'A' valid 1-byte → byte-len 1; `main(200)` → 0xC8 lone continuation → invalid UTF-8
           → None → -1.
           WHY IT'S A TODO (not the inline pass): the CONJUNCTION `let`-binder (state-derived init) CONSUMED BY a
           decode-match is a clean-decline on current trunk — `parameter reference has no local slot`
           (select.rs). Bisection pins the trigger to the conjunction: let+`Bytes.len` alone COMPILES, inline+
           decode-match alone COMPILES (the sentinel above), only let+decode-match declines. Root (v-inference
           diagnosed): in `reduce_handle`'s tail-resumptive `beta_reduce`, `copy_structural` copies the let-init
           `s` FRESH while still an unresolved name; by the time it re-resolves, `freshen_local_binders` + the
           resume-rewrite have DETACHED the arm from its handle form, so `handle_arm_binds` fails on the copy and
           it falls back to the ORIGINAL slot-less op-param binder — while the trailing resume-arg `s` (no
           intervening let scope) resolves straight to the arm state and substitutes fine. The correct fix is a
           pin-predicate in eval.rs that pins the arm's own op-param uses sitting inside a let-INIT so the
           Ref-subst branch fires on the shared occurrence (v-inference's resolve machinery; miscompile risk if
           it over-pins the capture/mv classes). It is a REJECT-DON'T-MISCOMPILE decline (safe, not an ICE, not a
           wrong answer). This case now FOLDS to the 1 / -1 PASS — the flatten+pin fix (2687ed3ae, one-hole
           path flattens the let-wrapped resume + pins arm-state uses so they survive the copy) landed; it
           stands as a regression-guard for the invariant. adv-20 let-bound twin (2026-08-15).")
  (input
    (do
      (effect S (op dec (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((dec
              ()
              s
              (let
                ((b (Bytes.of #list((UInt8.wrap s)))))
                (resume
                  (match (String.from-bytes b) ((Some t) (String.byte-len t)) ((None _u) -1))
                  s))))
          (S.dec)))
      (export main)))
  (call main (: 65 Int64))
  (output (: 1 Int64))
  (call main (: 200 Int64))
  (output (: -1 Int64))
  (live-objects 0))

(case
  "the PASSING FLOOR of adv-20's bisection — a state byte LET-bound as a Bytes and consumed by Bytes.len (NO decode-match) in a tail-resumptive arm folds correctly"
  (doc
    "adv-20 bisection floor (v-effects lane): the PASSING control that isolates the finding-#20 decline
           above to the `let`+decode-match CONJUNCTION. SAME let-bound state-derived `Bytes` — `(let ((b (Bytes.of
           (list (UInt8.wrap s))))) …)` — but the binder `b` is consumed by `Bytes.len` (a plain byte-count), NOT
           a `String.from-bytes` decode-MATCH. The resume value `(+ (Bytes.len b) s)` reads both the let-bound
           `b`'s length AND the arm state `s` directly, so the output varies with state and the let-bound
           state-Bytes coexisting with state-arithmetic in the resume is exercised end to end. `main(65)`: the
           one-element Bytes has length 1, state 65 → resume `1 + 65` = 66. `main(200)`: length 1, state 200 →
           `1 + 200` = 201. This COMPILES + folds correct on all backends (the LET-BOUND twin directly above
           DECLINES only because its consumer is a decode-match, whose specialization copies the let-init `s`
           fresh and detaches it from the arm form; a plain `Bytes.len` consumer never triggers that copy). Pins
           the passing side of the bisection so a future change can't quietly break let-binding a state-derived
           Bytes even in the non-decode shape. Uniform on all 3 backends, opt-sweep 0-divergence. adv-20 passing
           floor (2026-08-15).")
  (input
    (do
      (effect S (op dec (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((dec () s (let ((b (Bytes.of #list((UInt8.wrap s))))) (resume (+ (Bytes.len b) s) s))))
          (S.dec)))
      (export main)))
  (call main (: 65 Int64))
  (output (: 66 Int64))
  (call main (: 200 Int64))
  (output (: 201 Int64)))

(case
  "adv-20 class is BROADER than the Bytes decode — a state-derived OPTION LET-bound then consumed by a plain Some/None match also folds (flatten+pin fix landed; regression-guard)"
  (doc
    "adv-20 class-boundary witness (v-effects lane): proves finding-#20's decline is NOT specific to the
           `String.from-bytes` Bytes decode-match — the trigger is the GENERAL shape `let`-bound state-derived
           value CONSUMED BY A MATCH in a tail-resumptive resume. Here the let-bound value is an OPTION built
           from the handler state — `(let ((o (if (> s 100) ((. Option Some) (+ s 1)) ((. Option None) unit))))
           …)` — consumed by a plain `(match o ((Some x) x) ((None _u) -1))`. NO Bytes, NO decode; just a
           state-derived Option matched. PRE-FIX this DECLINED with the IDENTICAL root as the let-bound Bytes
           twin above (`cdz compile` → `parameter reference has no local slot`, a clean Reject::decline, NOT an
           ICE — confirmed by direct standalone compile 2026-08-15). CONTRAST the PASSING FLOOR two cases up
           (`Bytes.len b`, no match) which COMPILES — so the discriminant is the MATCH consumer, not the value
           type: any match on a let-bound state-derived value re-materializes the let-init's state occurrence
           under the arm-body specialization (`copy_structural` copies the init `s` fresh, `freshen_local_binders`
           + the resume-rewrite detach it from the handle form, `handle_arm_binds` fails on the copy → slot-less
           original op-param binder). This BROADENS the fix surface: v-inference's eval.rs let-init resolve
           pin-predicate must cover ANY match-consumed let-bound state value, not just the Bytes decode. This
           now FOLDS to the -1 / 201 PASS (the flatten+pin fix 2687ed3ae landed); it stands as a regression-guard
           for the broadened class. `main(65)`: s=65 not >100 → None → -1. `main(200)`: s=200 >100 → Some 201 →
           201. adv-20 broadened-class regression-guard (2026-08-15).")
  (input
    (do
      (effect S (op dec (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((dec
              ()
              s
              (let
                ((o (if (> s 100) (Option.Some (+ s 1)) (Option.None unit))))
                (resume (match o ((Some x) x) ((None _u) -1)) s))))
          (S.dec)))
      (export main)))
  (call main (: 65 Int64))
  (output (: -1 Int64))
  (call main (: 200 Int64))
  (output (: 201 Int64)))

(case
  "adv-20 nested-let — a NESTED let inside the let-init folds (recursive flatten; regression-guard)"
  (doc
    "adv-20 nested-let regression-guard (v-effects; was a TODO witness, now PASSES — the recursive flatten
           landed). The one-hole flatten handles a let-wrapped resume whose let-init reads the arm state; the
           recursive `flatten_nested_pure_let` (be0f729b3) extends it to a NESTED `let` INSIDE the let-init.
           Here `o`'s init is itself `(let ((c (+ s 1))) ((. Option Some) c))` — a nested let reading `s` —
           consumed by the resume-value match; without the recursion the inner let-init's `s` would survive to
           emit as a slotless `Core::Param{arm.state}` (the original finding-#20 class, one `let` deeper).
           `main(65)`: inner `c` = 66, `o` = Some 66, match → 66. `main(200)`: `c` = 201 → 201. Guards the
           recursive-flatten fix against a regression to one-level-only. adv-20 nested-let (2026-08-15).")
  (input
    (do
      (effect S (op dec (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((dec
              ()
              s
              (let
                ((o (let ((c (+ s 1))) (Option.Some c))))
                (resume (match o ((Some x) x) ((None _u) -1)) s))))
          (S.dec)))
      (export main)))
  (call main (: 65 Int64))
  (output (: 66 Int64))
  (call main (: 200 Int64))
  (output (: 201 Int64)))

(case
  "adv-20 DEEP nest — a THREE-level nested let in the let-init folds, guarding arbitrary recursion depth"
  (doc
    "adv-20 depth regression-guard (v-effects): the deeper twin of the nested-let case above — `o`'s init
           is a THREE-level `(let ((a (+ s 1))) (let ((b (+ a 1))) ((. Option Some) b)))`, each level reading a
           binder that ultimately traces to the arm state `s`. The recursive `flatten_nested_pure_let`
           (be0f729b3) must flatten to arbitrary depth (not just one nested level), so every level's `s`-derived
           init is substituted and none reaches emit as a slotless `Core::Param`. Guards against a regression
           that handles only a single nested level. `main(65)`: `a` = 66, `b` = 67, `o` = Some 67 → 67.
           `main(200)`: `a` = 201, `b` = 202 → 202. Uniform on all backends. adv-20 deep-nest (2026-08-15).")
  (input
    (do
      (effect S (op dec (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          n
          ((dec
              ()
              s
              (let
                ((o (let ((a (+ s 1))) (let ((b (+ a 1))) (Option.Some b)))))
                (resume (match o ((Some x) x) ((None _u) -1)) s))))
          (S.dec)))
      (export main)))
  (call main (: 65 Int64))
  (output (: 67 Int64))
  (call main (: 200 Int64))
  (output (: 202 Int64)))

(case
  "PREFIX-order edges — the state string compares against crossed op-arg strings: equal, longer-prefix, and shorter-prefix faces"
  (doc
    "3-way LEXICOGRAPHIC String ordering (< / = / >) against a threaded String handler state — distinct
           from the string-EQUALITY pins (sg3, one-shot lock): the `vs` arm answers `(if (< s probe) -1 (if
           (= s probe) 0 1))`, comparing the state string `s` to each crossed op-arg probe. Covers the three
           prefix-order edges: EQUAL (`s`='mm' vs 'mm' → 0), STATE-LONGER-prefix (`s`='mm' vs 'm', 'm' is a
           proper prefix so 'mm' > 'm' → 1), and PROBE-LONGER/lex-greater (`s`='mm' vs 'mzz', 'mm' < 'mzz' →
           -1). Seed parity picks the state string 'mm' (even) or 'mz' (odd). `main(2)`: s='mm'; vs('mm')=0,
           vs('m')=1, vs('mzz')=-1; `(+ (* 100 0) (+ (* 10 1) -1))` = 9. `main(3)`: s='mz'; vs('mm')=1 (mz>mm),
           vs('m')=1, vs('mzz')=-1 (mz<mzz); `(+ 100 (+ 10 -1))` = 109. Uniform on all 3 backends, opt-sweep
           0-divergence. Breaker rope-lex-order probe lx2 (2026-08-11).")
  (input
    (do
      (effect S (op vs (-> String Int64)))
      (def
        (main (: n Int64))
        (handle
          S
          (if (= (% n 2) 0) "mm" "mz")
          ((vs (probe) s (resume (if (< s probe) -1 (if (= s probe) 0 1)) s)))
          (+ (* 100 (S.vs "mm")) (+ (* 10 (S.vs "m")) (S.vs "mzz")))))
      (export main)))
  (call main (: 2 Int64))
  (output (: 9 Int64))
  (call main (: 3 Int64))
  (output (: 109 Int64)))

(case
  "a THREE-phase state machine — Idle to Running(count,peak) to Done(total), the sentinel input drives the final transition"
  (doc
    "A user 3-variant SUM as the handler state — `(type Phase (Idle) (Running Int64 Int64) (Done Int64))`
           — driven through its lifecycle by dispatches: the landed variant-transition pins are Option/Result
           TWO-phase; this pins a THREE-phase machine with per-phase payload arithmetic and an absorbing
           terminal. The `step` arm matches the current phase and transitions: Idle -> Running(v,v); Running(c,p)
           accumulates `count += v` and tracks `peak = max`, or on a NEGATIVE sentinel input transitions to
           Done(count+peak); Done is absorbing (threads unchanged). `query` decodes whichever phase holds.
           `main(3)`: step(3) Idle->Running(3,3); step(7) Running->Running(10,7); step(-1) sentinel ->
           Done(10+7=17); query Done -> 10000+17 = 10017. `main(9)`: Running(9,9)->Running(16,9)->Done(25) ->
           10025. Uniform on all 3 backends, opt-sweep 0-divergence. Breaker phase-machine probe ph1
           (2026-08-12).")
  (input
    (do
      (type Phase (Idle) (Running Int64 Int64) (Done Int64))
      (effect M (op step (-> Int64 Int64)) (op query (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          M
          (Idle)
          ((step
              (v)
              st
              (match
                st
                ((Idle) (resume 0 (Running v v)))
                ((Running c p)
                  (resume c (if (< v 0) (Done (+ c p)) (Running (+ c v) (if (> v p) v p)))))
                ((Done t) (resume t st))))
            (query
              ()
              st
              (resume
                (match st ((Idle) -1) ((Running c p) (+ (* 100 c) p)) ((Done t) (+ 10000 t)))
                st)))
          (let ((_a (M.step n))) (let ((_b (M.step 7))) (let ((_c (M.step -1))) (M.query))))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 10017 Int64))
  (call main (: 9 Int64))
  (output (: 10025 Int64)))

(case
  "a resume value that MATCHES a COMPOUND scrutinee over the growing state binds it once — finding-24 match-scrutinee face"
  (doc
    "Breaker sft1 (min-heap-sift) SHRUNK to its mechanism: a tail-resumptive arm whose resume value is a
           `(match <compound> (h2 <body>))` — a SINGLE bare-name binder over a COMPOUND scrutinee that references
           the growing List state. The resumptive fold threads a match by copying its SCRUTINEE into every
           continuation copy (one per dispatch), so a compound scrutinee over handler state duplicated per
           dispatch makes emit grow SUPER-LINEARLY — the finding-24 continuation-duplication class, exposed
           through a match-scrutinee instead of a `let`-init (sft1: exponential, 47015 locals in one function ->
           INVALID wasm too-many-locals). The fix canonicalizes the irrefutable single-binder match to a `let`
           (`(match k (h2 b))` = `(let ((h2 k)) b)`), routing the scrutinee through the per-dispatch `#st`
           state-bind so it binds ONCE (sft1: 1.47MB -> 38820 bytes valid). This case pins the shape at 3
           dispatches: `push v` appends `v+n` to the state and resumes the LAST element read back through the
           match binder. `main(1)`: pushes 4,5,6; resumes 4,5,6; `(+ (* 100 4) (+ (* 10 5) 6))` = 456.
           `main(2)`: pushes 5,6,7 -> 567. Uniform on all 3 backends. v-effects finding-24 match-scrutinee
           coverage-gap fix (2026-08-14).")
  (input
    (do
      (effect H (op push (-> Int64 Int64)))
      (def (getat (: xs (List Int64)) (: i Int64)) (match (List.at xs i) ((Some v) v) ((None u) 0)))
      (def
        (main (: n Int64))
        (handle
          H
          (: #list() (List Int64))
          ((push
              (v)
              st
              (match (List.push st (+ v n)) (h2 (resume (getat h2 (- (List.len h2) 1)) h2)))))
          (let
            ((a (H.push 3)))
            (let ((b (H.push 4))) (let ((c (H.push 5))) (+ (* 100 a) (+ (* 10 b) c)))))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 456 Int64))
  (call main (: 2 Int64))
  (output (: 567 Int64)))

(case
  "a min-heap in a list — push sifts UP, popmin sifts DOWN — the finding-24 match-scrutinee hoist at integration scale"
  (doc
    "The breaker sft1 min-heap in FULL, the integration witness for the finding-24 match-scrutinee
           coverage-gap fix that the sibling case above pins minimally. TWO compound match-scrutinees over the
           growing List state feed resume in a single arm each: the `push` arm matches `(siftup (List.push st v) …)`
           and the `popmin` arm matches `(siftdn (List.update (dropl …) …) 0)`. Before the hoist, the resumptive
           fold copied each compound scrutinee into every continuation copy — one per dispatch — and the recursive
           `siftup`/`siftdn`/`smallest`/`getat`/`dropl` heap machinery materialized inside each copy, so a 6-perform
           drive (push,push,push,popmin,push,popmin) grew SUPER-LINEARLY: 47015 locals in one function, 1.47MB
           INVALID wasm (`too many locals`), while rust and rust-async — which don't cap locals — passed. The fix
           canonicalizes each irrefutable single-binder match over a compound scrutinee to a `let`, routing the
           scrutinee through the per-dispatch `#st` state-bind so it binds ONCE: 38820 bytes VALID, uniform on all
           three backends. This is the sibling shrink's shape at real scale — two hoist sites, recursion, and live
           heap ops — catching an integration-scale regression the 3-dispatch shrink cannot. `push v` appends `v`
           and sifts up answering the new root; `popmin` drops the last element onto the root and sifts down
           answering the OLD root. `main(10)`: push 12,4,7 -> popmin -> push 7 -> popmin = 120404040707.
           `main(0)`: push 2,4,7 -> popmin -> push -3 -> popmin = 20202019697. Breaker bank
           .breaker-probes/2026-08-14-minheap-sift; v-effects finding-24 match-scrutinee fix (2026-08-14).")
  (input
    (do
      (effect H (op push (-> Int64 Int64)) (op popmin (-> Int64)))
      (def (getat (: xs (List Int64)) (: i Int64)) (match (List.at xs i) ((Some v) v) ((None u) 0)))
      (def
        (siftup (: xs (List Int64)) (: i Int64))
        (if
          (= i 0)
          xs
          (if
            (< (getat xs i) (getat xs (/ (- i 1) 2)))
            (siftup
              (List.update (List.update xs (/ (- i 1) 2) (getat xs i)) i (getat xs (/ (- i 1) 2)))
              (/ (- i 1) 2))
            xs)))
      (def
        (smallest (: xs (List Int64)) (: i Int64))
        (if
          (< (+ (* 2 i) 1) (List.len xs))
          (if
            (< (getat xs (+ (* 2 i) 1)) (getat xs i))
            (if
              (< (+ (* 2 i) 2) (List.len xs))
              (if (< (getat xs (+ (* 2 i) 2)) (getat xs (+ (* 2 i) 1))) (+ (* 2 i) 2) (+ (* 2 i) 1))
              (+ (* 2 i) 1))
            (if
              (< (+ (* 2 i) 2) (List.len xs))
              (if (< (getat xs (+ (* 2 i) 2)) (getat xs i)) (+ (* 2 i) 2) i)
              i))
          i))
      (def
        (siftdn (: xs (List Int64)) (: i Int64))
        (if
          (= (smallest xs i) i)
          xs
          (siftdn
            (List.update (List.update xs (smallest xs i) (getat xs i)) i (getat xs (smallest xs i)))
            (smallest xs i))))
      (def
        (dropl (: xs (List Int64)) (: i Int64) (: keep Int64) (: acc (List Int64)))
        (if (< i keep) (dropl xs (+ i 1) keep (List.push acc (getat xs i))) acc))
      (def
        (main (: n Int64))
        (handle
          H
          (: #list() (List Int64))
          ((push
              (v)
              st
              (match
                (siftup (List.push st v) (- (List.len (List.push st v)) 1))
                (h2 (resume (getat h2 0) h2))))
            (popmin
              ()
              st
              (if
                (= (List.len st) 0)
                (resume -99 st)
                (if
                  (= (List.len st) 1)
                  (resume (getat st 0) (: #list() (List Int64)))
                  (match
                    (siftdn
                      (List.update
                        (dropl st 0 (- (List.len st) 1) (: #list() (List Int64)))
                        0
                        (getat st (- (List.len st) 1)))
                      0)
                    (h2 (resume (getat st 0) h2)))))))
          (let
            ((a (H.push (+ n 2))))
            (let
              ((b (H.push 4)))
              (let
                ((c (H.push 7)))
                (let
                  ((d (H.popmin)))
                  (let
                    ((e (H.push (- n 3))))
                    (let
                      ((f (H.popmin)))
                      (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
      (export main)))
  (call main (: 10 Int64))
  (output (: 120404040707 Int64))
  (call main (: 0 Int64))
  (output (: 20202019697 Int64))
  (live-objects known-leak))

; ── multi-shot continuation safe-rejects (migrated from rcdzc tests/mod.rs, delanguaging handoff from
;    v-rcdzc-test-shrink 2026-08-30). A multi-shot arm `(+ (resume 1 s) (resume 2 s))` splices its
;    delimited continuation C TWICE; that is sound ONLY when every op C performs is discharged BY THIS
;    handler (folded to pure code). When C spans a HOST call or reaches an OUTER handler's effect, a second
;    splice would re-issue that boundary/outer op per resume — so the fold declines cleanly rather than
;    double it (host-composition invariant, §4.4). The decline is CLEAN: it never leaks an internal `#eff`
;    specialization name or a `$s` state-param name (the message-pin below witnesses the clean text).
(case
  "a multi-shot continuation spanning a host call is rejected, never doubling the host call"
  (doc
    "The leading `Amb.flip` gets continuation `C = (+ [] (Ask.ask))`; the multi-shot arm `(+ (resume 1
           s) (resume 2 s))` would splice `C` twice, running the host-delegated `Ask.ask` once per resume — a
           re-deriving host cannot reconstruct a chain of run-local heap handles (§4.4: a reified continuation
           must not span a host call). So it stays a clean decline, never runs to a value (a doubled host
           call). Migrated from rcdzc a_multishot_continuation_spanning_a_host_call_declines_not_doubles_the_call.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (effect Ask (op ask (-> Unit Int64)))
      (def
        (main)
        (host
          (Ask)
          (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Amb.flip) (Ask.ask)))))
      (export main)))
  (error CDZ0408))

(case
  "a multi-shot continuation reaching an outer handler's effect is rejected, never doubling it"
  (doc
    "An inner multi-shot `Amb` handler nested in an outer `Ctr` handler; the body `(+ (Ctr.tick)
           (Amb.flip))`. The `Amb.flip` continuation is spliced twice by `(+ (resume 1 s) (resume 2 s))`,
           which would re-issue the OUTER-handler-discharged `Ctr.tick` per resume — advancing the outer
           state more than once for a single perform. The re-reducing fold is sound only when every performed
           op in the continuation is discharged BY THIS handler; an op that escapes to an outer handler stays
           a clean decline. Migrated from rcdzc a_multishot_continuation_reaching_an_outer_handler_effect_declines_not_doubles_it.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (u) s (resume s (+ s 1))))
          (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (+ (Ctr.tick) (Amb.flip)))))
      (export main)))
  (error CDZ0408))

(case
  "a multi-shot continuation reaching an outer effect via an if branch is rejected"
  (doc
    "The conditional-path companion: a multi-shot arm whose re-run continuation reaches an outer-handler
           effect through an `if` BRANCH (not a strict operand). `(if (< (Amb.flip) 5) (Ctr.tick) 99)` — re-
           running the continuation per `Amb.flip` resume would re-issue the outer `Ctr.tick` in the taken
           branch, advancing the outer state more than once. Exercises the `if`/branch threading fold path,
           distinct from the strict-operand shape. Migrated from rcdzc a_multishot_continuation_reaching_an_outer_effect_via_an_if_branch_declines.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (main)
        (handle
          Ctr
          0
          ((tick (u) c (resume c (+ c 1))))
          (handle
            Amb
            0
            ((flip (u) s (+ (resume 1 s) (resume 2 s))))
            (if (< (Amb.flip) 5) (Ctr.tick) 99))))
      (export main)))
  (error CDZ0408))

(case
  "a multi-shot continuation spanning a host call inside a do is rejected"
  (doc
    "The do-sequence companion: a multi-shot arm whose continuation spans a HOST call inside a `(do …)`.
           `(do (Log.emit (Amb.flip)) 7)` under a multi-shot `flip` arm — re-running the continuation per
           resume would emit the delegated `Log.emit` host call more than once (§4.4 host-composition
           invariant). Exercises the `Core::Seq`/do path, distinct from the strict-operand host case. Migrated
           from rcdzc a_multishot_continuation_spanning_a_host_call_in_a_do_declines.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (effect Log (op emit (-> Int64 Unit)))
      (def
        (main)
        (host
          (Log)
          (handle Amb 0 ((flip (u) s (+ (resume 1 s) (resume 2 s)))) (do (Log.emit (Amb.flip)) 7))))
      (export main)))
  (error CDZ0408))

; ── self-call gated behind a nested conditional in a cond/scrutinee — the multi-value hoist safe-reject
;    (migrated from rcdzc tests/mod.rs a_selfcall_gated_in_an_if_condition_declines_not_hoisted, delanguaging
;    handoff from v-rcdzc-test-shrink 2026-08-30). `thread_returning_tuple` threads the condition and lifts a
;    condition-level self-call pending temp AROUND the whole `if` — sound ONLY for a self-call on the cond's
;    UNCONDITIONAL strict spine. A self-call under a NESTED `if`/`and`/`or` in the cond/scrutinee runs on only
;    some paths, so hoisting its temp would make it run UNCONDITIONALLY (an eval-order MISCOMPILE). The mode
;    gate `multivalue_leaves_threadable` rejects it → a clean CDZ0900 decline, never a miscompile or a leaked
;    internal `#eff`/bodyless-spec name (the message-pin witnesses the clean text). The DIRECT-in-condition
;    shape `(< (walk …) 100)` (no nested gate) still folds — so this is a precise decline, not a blanket one.
(case
  "a self-call gated behind a nested if IN an if-condition declines cleanly, never hoisted"
  (doc
    "`walk` self-calls inside a nested `(if (> n 5) (walk (- n 1)) 0)` sitting in the OUTER `if`'s
           CONDITION. The self-call runs on only the `> n 5` path; hoisting its multi-value pending temp
           around the whole `if` would run it unconditionally + thread state as if always taken. Declines
           cleanly rather than miscompile.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def (walk (: n Int64)) (if (= n 0) 0 (+ (if (> n 5) (walk (- n 1)) 0) (Ctr.tick))))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a self-call gated behind a nested if IN a match-scrutinee declines cleanly, never hoisted"
  (doc
    "The match-scrutinee position of the nested-conditional self-call: the self-call sits under
           `(match (if (> n 5) (walk (- n 1)) 0) (_ 0))` in the scrutinee. Same eval-order hazard as the
           if-condition face, a distinct fold path — declines cleanly.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (walk (: n Int64))
        (if (= n 0) 0 (+ (match (if (> n 5) (walk (- n 1)) 0) (_ 0)) (Ctr.tick))))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 0 Int64)))

(case
  "a self-call gated behind an and short-circuit in an if-condition declines cleanly, never hoisted"
  (doc
    "The `and` short-circuit face: the self-call sits under `(and (> n 5) (< (walk (- n 1)) 100))` in
           the condition, so it runs only when the left conjunct holds. Hoisting its temp would run it
           unconditionally — declines cleanly rather than miscompile the short-circuit.")
  (input
    (do
      (effect Ctr (op tick (-> Unit Int64)))
      (def
        (walk (: n Int64))
        (if (= n 0) 0 (+ (if (and (> n 5) (< (walk (- n 1)) 100)) 1 0) (Ctr.tick))))
      (def (main) (handle Ctr 0 ((tick (u) s (resume s (+ s 1)))) (walk 3)))
      (export main)))
  (call main)
  (output (: 0 Int64)))

; ── effect safe-rejects: escaping / captured-continuation / partial-application (migrated from rcdzc
;    tests/mod.rs, delanguaging handoff from v-rcdzc-test-shrink 2026-08-30). A performing closure or a
;    reified continuation that CROSSES its handler's extent has no home for its perform / no machine
;    representation at the boundary, so the compiler refuses it up front rather than emit a trapping or
;    wrong-valued artifact. Pinned by exact diagnostic CODE (a hard rejection) where one is assigned.
(case
  "an escaping closure whose body performs is rejected (perform out of the handler's extent)"
  (doc
    "The handler's body value is `(fn (x) (+ x (St.get)))` — a closure that ESCAPES the `handle St`
           (it is returned, then applied `(… 10)` AFTER the handle closes). Its `St.get` would run
           out-of-extent, with no handler live, so it is a soundness rejection (CDZ0401 effect-escape), not
           a value. Migrated from rcdzc an_escaping_closure_whose_body_performs_still_declines.")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (def
        (main (: k Int64))
        ((handle St k ((get (u) s (resume s s))) (fn ((: x Int64)) (+ x (St.get)))) 10))
      (export main)))
  (error CDZ0401))

(case
  "an escaping captured continuation is refused, not miscompiled"
  (doc
    "A handler arm that RETURNS its continuation as an escaping value — `(flip (u) s (fn (x) (resume x
           s)))` yields the resume as a lambda; the handle value IS that lambda, applied `(k 5)` after the
           handle closes. This is the genuine captured-`k` frontier (§4.4): it needs a reified `Ty::Cont`
           heap value the seed does not build, so it is refused up front (a handle value that is a function
           has no machine representation at the boundary) — never compiled to a trapping / wrong-valued
           artifact. Currently a codeless decline (a latent seq-286 code gap, tracked separately). Migrated
           from rcdzc an_escaping_captured_continuation_is_refused_not_miscompiled.")
  (input
    (do
      (effect Amb (op flip (-> Unit Int64)))
      (def
        (main)
        (let ((k (handle Amb 0 ((flip (u) s (fn (x) (resume x s)))) (+ 100 (Amb.flip))))) (k 5)))
      (export main)))
  (call main)
  (output (: 105 Int64)))

(case
  "a partial application of a performing closure that escapes its handler is rejected"
  (doc
    "`mk` boxes a curried closure `(fn a (fn b (+ a (+ b (E.tick)))))`; `main` handles `E` and applies
           the projected closure to ONE of its two args (`(f 3)` — a genuine 1-of-2 partial), RETURNING the
           residual closure as the handle body's value. The residual closure PERFORMS `E.tick` and ESCAPES
           its handler (returned out of the `handle`), so lifting it to a standalone function hits the
           effect-escape / unrepresentable-closure rejection (CDZ0201) — a clean reject, never a mis-emit /
           invalid wasm. Migrated from rcdzc a_partial_application_of_a_performing_closure_under_a_handler_declines_cleanly.")
  (input
    (module m
      (effect E (op tick (-> Unit Int64)))

      (type Box (C (-> Int64 (-> Int64 Int64))))

      (def (mk) (Box.C (fn ((: a Int64)) (fn ((: b Int64)) (+ a (+ b (E.tick)))))))

      (def (main) (handle E 0 ((tick (u) s (resume s (+ s 1)))) (match (mk) ((Box.C f) (f 3)))))

      (export main)))
  (error CDZ0201))

(case
  "hst1 a TUPLE handler state rebuilt per tail-resume threads exactly (the state-accumulator face)"
  (doc
    "Breaker scaling-leak fence (2026-08-31, v-memory-safety-commissioned pin): a handle whose
     tuple state is REBUILT on every resume (`(resume (* v 2) #tuple((+ cnt 1) (+ sum v)))`)
     threads the state exactly — result = sum of 2k for k=n..1 = n(n+1): 30 at n=5, 0 at n=0,
     10100 at n=100 (rust-agreed at filing). The KNOWN LEAK this pin tracks: one SUPERSEDED state
     tuple per perform survives (live 5@n=5, 100@n=100 — the iteration-SCALING class; triage =
     drop-old-on-rebind in the discharge's state self-loop, v-memory-safety reclaim lane after
     05:18721, with the v-effects desugar dependency). #5766 tolerate-fewer auto-passes the
     collapse; the value pins hold either way.")
  (input
    (do
      (effect St (op bump (-> Int64 Int64)))
      (def (loop (: k Int64) (: acc Int64)) (if (> k 0) (loop (- k 1) (+ acc (St.bump k))) acc))
      (def
        (main (: n Int64))
        (handle
          St
          #tuple(0 0)
          ((bump
              (v)
              s
              (match
                s
                (#tuple(cnt sum) (resume (* v 2) #tuple((+ cnt 1) (+ sum v))))
                (_ (resume 0 s)))))
          (loop n 0)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 30 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: 100 Int64))
  (output (: 10100 Int64))
  (live-objects known-leak))

(case
  "an abortive perform in a non-tail accumulator-introduced recursion declines cleanly (safe floor; flips to the abort value under the non-local-exit vertical)"
  (doc
    "breaker's non-local-exit face: a non-tail ASSOCIATIVE abortive recursion `(def (loop k) (if (> k 0)
           (+ (loop (- k 1)) (if (= k 2) (E.bail) k)) 0))` under `(handle E 0 ((bail (u) s 99)) (loop n))`.
           accumulator-introduction rewrites `(+ (loop …) …)` into a TAIL self-call whose ACCUMULATOR ARGUMENT
           carries the abort, so a self-call-tail-only check is fooled and single-return specialization would
           fold the abort as tail-RESUMPTIVE (thread 99 into the accumulator → the silent-miscompile
           main(3)=103). Under the safe floor (`abortive_perform_off_tail` in `specialize_recursive`) this
           DECLINES cleanly (CDZ0900) on all backends rather than miscompiling. The ABORT must ABANDON the
           pending `+` frames and yield the arm value: idealistically main(1)=1 (k never hits the bail) and
           main(3)=99 (the k==2 bail abandons everything). Flips to those PASSes when the non-local-exit
           calling convention (tagged-return) lands. Fix #7361; breaker-verified no over-decline (a
           TAIL-position abortive recursion still folds).")
  (input
    (do
      (effect E (op bail (-> Unit Int64)))
      (def (loop (: k Int64)) (if (> k 0) (+ (loop (- k 1)) (if (= k 2) (E.bail unit) k)) 0))
      (def (main (: n Int64)) (handle E 0 ((bail (u) s 99)) (loop n)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 99 Int64)))

(case
  "an INDIRECT (helper-hidden) abortive perform in a non-tail accumulator recursion declines cleanly (safe floor; was a miscompile pre-fix)"
  (doc
    "The INTERPROC sibling of the abortive accum case above: the perform is hidden in a HELPER the
           per-step term calls — `(+ (loop (- k 1)) (helper k))` with `(def (helper k) (if (= k 2) (E.bail 99)
           k))`. Accumulator introduction reassociates `(helper k)` onto the accumulator arg; naively the abort
           would ride buried in a callee and the non-local-exit CC could not see it to short-circuit — a SILENT
           MISCOMPILE (main(3) ran 103; the pending `+` frames were summed instead of abandoned). FOLDED by
           INLINING the simple performing helper into the per-step term BEFORE reassociation (accum's
           `plan_helper_inline` / `build_inlined_term`): the whole-term helper call `(helper k)` is replaced by
           the helper's body with its params bound to the call args — `(if (= k 2) (E.bail 99) k)` — so the
           perform becomes DIRECT in the reassociated combine `(+ acc (if (= k 2) (E.bail 99) k))`, the exact
           shape the DIRECT-term abortive-accumulator fold of the case above already handles. The inline is
           narrow + capture/duplication-safe (simple atom args, binder-free helper body, a single direct
           perform); anything outside that shape keeps the original safe decline (`term_calls_performing_def`
           → a plain non-tail form the effects safe-floor rejects, CDZ0900) — never a wrong value. main(1)=1
           (k never hits the bail) and main(3)=99 (the k==2 bail abandons the pending `+` frames, homing the
           perform arg 99 to the `(bail (v) s v)` arm) on all three backends.")
  (input
    (do
      (effect E (op bail (-> Int64 Int64)))
      (def (helper (: k Int64)) (if (= k 2) (E.bail 99) k))
      (def (loop (: k Int64)) (if (> k 0) (+ (loop (- k 1)) (helper k)) 0))
      (def (main (: n Int64)) (handle E 0 ((bail (v) s v)) (loop n)))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 3 Int64))
  (output (: 99 Int64)))

; dcx1: the GREEN complement of "an escaping closure whose body performs is rejected" — the SAME
; escaping-performer closure is FINE when its call site sits under a live handler, and dispatch is
; DYNAMIC: the closure created under handle-111 but applied under handle-222 draws 222 (the call-time
; handler), not 111 (the creation-time one). Pins call-time (dynamic-extent) dispatch through an
; escaped closure; the unhandled twin above stays CDZ0401. n+222.
(case
  "an escaped performing closure dispatches to the CALL-site handler, not its creation-site one"
  (input
    (do
      (effect E (op get (-> Int64)))
      (def
        (main (: n Int64))
        (let
          ((f (handle E 111 ((get () s (resume s s))) (fn ((: k Int64)) (+ k (E.get))))))
          (handle E 222 ((get () s (resume s s))) (f n))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 222 Int64))
  (call main (: 5 Int64))
  (output (: 227 Int64)))

; esx1: a SAME-effect perform in the inner shadow's ARM RESUME-VALUE homes to the OUTER handler — the
; arm-position companion of mo4 (which pins the SEED homing outward). The inner handler's own arm is
; OUTSIDE its handled region, so the bare `(E.get)` inside `(resume (+ (E.get) s) s)` dispatches to
; the enclosing E handler (seed 7), never to itself: body draw = 7 + 50 = 57 (a self-dispatch would
; double the inner state to 100 or loop forever). Distinct from the adv-69 a3 sub-faces, whose
; BRANCH-wrapped arm performs decline; this plain perform FOLDS. (breaker probe hsx2, verified
; tri-target exact + byte-idempotent; fully scalar, composes no value-heap runtime.)
(case
  "a same-effect perform in the inner arm's resume-value homes to the outer handler"
  (input
    (do
      (effect E (op get (-> Int64)))
      (def
        (main (: n Int64))
        (handle
          E
          7
          ((get () s (resume s s)))
          (handle E 50 ((get () s (resume (+ (E.get) s) s))) (+ (E.get) (* 100 n)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 57 Int64))
  (call main (: 3 Int64))
  (output (: 357 Int64)))

; csx1: the HYGIENIC margin around the eg1 name-collision decline — the identical nested shape with
; the caller's outer destructure binder named `s`, the handler's own STATE binder spelling. This
; collision axis is a DIFFERENT pair than the declined one (caller binder vs the inlined helper's
; internal arm binder): the helper's binders are a/b, so the capture detector must NOT fire, and the
; state binder must not capture the continuation's `s` reads. Folds correctly to the same idealistic
; value as the collision todo above (2101 + 110n) — one spelling swap flips decline<->fold, pinning
; the detector's precision from the hygienic side. Adjacent margins breaker-verified the same tick:
; try-binder = state binder (15/-1), op-param = continuation binder (201100 + 1103n... exact), and
; the helper's INTERNAL binder spelled `s` with distinct outer binders (2101 + 110n) — all fold
; correct; the sole live gap is the let-init value-flow shape (filed, fence held). Guards the coming
; match-binder freshening fix against over-declining or mis-freshening state-binder spellings.
; (breaker probe cs2, verified tri-target exact + byte-idempotent.)
(case
  "eg1 shape with the outer destructure binder spelled like the handler state binder folds"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (stamp p) (match p (#tuple(a b) #tuple(a b (C.tick)))))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (match
            (stamp #tuple(1 true))
            (#tuple(s _b t1)
              (match
                (stamp #tuple(2 true))
                (#tuple(c _d t2) (+ s (+ (* 1000 c) (+ (* 10 t1) (* 100 t2))))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2101 Int64))
  (call main (: 3 Int64))
  (output (: 2431 Int64)))

; mtx1: the DEPTH face of the mutually-recursive effectful specialization — the existing mutual-group
; case pins the specialization KNOT at toy depth; this pins that the specialized pair stays
; LOOP-CONVERTED (O(1) stack) with a perform on every other step: depth 200,000 through the ev/od
; cycle under a state handler computes exactly (od adds 1, ev adds the tick draw k = 0..99,999;
; 100000 + 4999950000 = 5000050000). A specialization that breaks the mutual-loop conversion (each
; partner's recursive call must resolve to the other's specialized copy AS A LOOP EDGE, not a call)
; traps on stack exhaustion around 1e4-1e5 frames instead. Also a regression smoke for the
; tail-call/mutual-loop analysis cluster (extracted to select/tailcall.rs in #8063). (breaker probe
; mt3, verified tri-target exact + byte-idempotent; pure mutual pairs and a TRIPLE a->b->c cycle
; verified to 1e7 same tick.)
(case
  "a specialized mutual effectful pair stays loop-converted at depth 200000"
  (input
    (do
      (effect Ctr (op tick (-> Int64)))
      (def (ev (: n Int64) (: acc Int64)) (if (= n 0) acc (od (- n 1) (+ acc (Ctr.tick)))))
      (def (od (: n Int64) (: acc Int64)) (if (= n 0) acc (ev (- n 1) (+ acc 1))))
      (def (main (: n Int64)) (handle Ctr 0 ((tick () s (resume s (+ s 1)))) (ev n 0)))
      (export main)))
  (call main (: 6 Int64))
  (output (: 6 Int64))
  (call main (: 200000 Int64))
  (output (: 5000050000 Int64)))

; clx1: the LET-BOUND VALUE-FLOW face of the eg1 name-collision — each stamp call is let-bound
; (`q`/`r`) and the bound value THEN matched with the colliding destructure binder `a`. The capture
; is identical to the direct-scrutinee collision todo above (the commute nests the continuation
; under the inlined helper's own `a`), but the collision arrives through the let's VALUE FLOW, not
; the syntactic scrutinee position — the shape #8052's first detector deliberately excluded and
; which then folded WRONG (2102: the outer `a` read the inner helper's binder) on BOTH backends
; until #8057 peeled the scrutinee through let-binding refs (Resolved::Ref) before the collision
; test. Now FOLDS correctly: the match-arm-binder freshening (`reduce_applied_lambdas` α-renames the
; inlined helper's `(#tuple(a b))` arm binders to fresh names) makes the let-bound value-flow collision
; impossible, exactly as for the direct twin. Idealistic value identical to the direct twin, 2101 + 110n,
; hand-derivation confirmed by the effect-free control (stamp2 with t as a parameter: 2101/2431 exact).
; (breaker probe col4, the tick-1476 P0; matrix: direct + 2nd-binder + this shape now all FOLD via
; freshening; colliding-but-unused, state-binder-spelling, and distinct-name shapes fold too.)
(case
  "eg1 collision through a let-bound value flow folds (same inlined-helper match-binder freshening)"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def (stamp p) (match p (#tuple(a b) #tuple(a b (C.tick)))))
      (def
        (main (: n Int64))
        (handle
          C
          n
          ((tick () s (resume s (+ s 1))))
          (let
            ((q (stamp #tuple(1 true))))
            (match
              q
              (#tuple(a _b t1)
                (let
                  ((r (stamp #tuple(2 true))))
                  (match r (#tuple(c _d t2) (+ a (+ (* 1000 c) (+ (* 10 t1) (* 100 t2))))))))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2101 Int64))
  (call main (: 3 Int64))
  (output (: 2431 Int64)))

; mhx1: the PRECISION face of the CDZ0408 multi-shot boundary reject — the host call is LET-BOUND in
; a strict prefix BEFORE the multi-shot perform, so the flip's delimited continuation
; `(+ h (* [] 10))` is PURE over the already-bound response: re-running it per resume re-issues NO
; boundary op (the host fires exactly once; §4.4's 'must not span a host call' is satisfied). Today
; it still rejects CDZ0408 (v-effects ruling 2026-09-03: PRECISION GAP — the detector
; one_handle_multishot_reaches_foreign scans the WHOLE handle body for a foreign perform, not
; whether the perform sits inside the reified continuation). Idealistic value: log answers 100, the
; two resumes see (100 + 1*10) + (100 + 2*10) = 230 at any n. Flips to PASS when the detector tests
; the actual continuation; the four reject pins above (host IN the continuation) are unaffected and
; must stay rejects. (breaker probe mh3.)
(case
  "a multi-shot whose host call completed before the perform is admitted once the detector tests the continuation"
  (input
    (do
      (effect A (op flip (-> Int64)))
      (effect H (op log (-> Int64 Int64)))
      (def
        (main (: n Int64))
        (host
          (H)
          (handle
            A
            0
            ((flip () s (+ (resume 1 s) (resume 2 s))))
            (let ((h (H.log (+ n 1)))) (+ h (* (A.flip) 10))))))
      (export main)))
  (call main (: 7 Int64))
  (host-responses (respond h.log (: 100 Int64)))
  (host-calls (call h.log (: 8 Int64)))
  (output (: 230 Int64)))

; cpx1: the COLLECTION-mediated face of the escaped-performing-closure dispatch (dcx1 above pins the
; direct let flow: creation-site 111, call-site 222, the CALL-site handler wins). Here the closure is
; created under handler 111 inside a LIST, the list escapes the handle, and the element is retrieved
; and applied TWICE under a second handler with advancing state. Today the collection-mediated flow
; DECLINES CDZ0900 (the specializer's escaped-closure fold tracks direct value flow, not flow through
; a collection element) — a safe reject, never a wrong-home dispatch. Idealistic values from dcx1's
; call-site-dispatch semantics: applications read the SECOND handler's advancing state 10n then
; 10n+1, so (1 + 10n) + (2 + 10n + 1) = 20n + 4. Flips when the escaped-closure fold (or a runtime
; dispatch representation) covers collection-element flow. (breaker probe cp2; the no-creation-
; handler variant cp1 is a separate bare-error reject, queued to diagnostics.)
(case
  "a performing closure retrieved from a list dispatches to the call-site handler once collection flow is covered"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (let
          ((fs (handle C 111 ((tick () s (resume s s))) #list((fn ((: x Int64)) (+ x (C.tick)))))))
          (handle
            C
            (* n 10)
            ((tick () s (resume s (+ s 1))))
            (match (List.at fs 0) ((Some f) (+ (f 1) (f 2))) ((None) -1)))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 4 Int64))
  (call main (: 2 Int64))
  (output (: 44 Int64)))

; sqx1: the escaped performing CLOSURE under two SEQUENTIAL call-site handlers — composes dcx1
; (escape + call-site homing, one application) with hc3 (a performing DEF helper under sequential
; handles). The closure escapes its creation-site handler (111, never observed), is applied TWICE
; under the first call-site handle (advancing state: reads n then n+1 -> (1+n)+(2+n+1) = 2n+4) and
; once under a second handle with a DIFFERENT arm semantics (decrementing, seed 500 -> 503). Pins
; per-application re-homing of a first-class closure across handles for DIRECT value flow — the
; boundary the collection-mediated cpx1 todo sits just past (list flow declines; this direct flow
; must keep folding while that flip is built). (breaker probe sq1, verified tri-target exact +
; byte-idempotent; fully scalar.)
(case
  "an escaped performing closure re-homes per application across two sequential handlers"
  (input
    (do
      (effect C (op tick (-> Int64)))
      (def
        (main (: n Int64))
        (let
          ((f (handle C 111 ((tick () s (resume s s))) (fn ((: x Int64)) (+ x (C.tick))))))
          (+
            (handle C n ((tick () s (resume s (+ s 1)))) (+ (f 1) (f 2)))
            (* 100000 (handle C 500 ((tick () s (resume s (- s 1)))) (f 3))))))
      (export main)))
  (call main (: 0 Int64))
  (output (: 50300004 Int64))
  (call main (: 3 Int64))
  (output (: 50300010 Int64)))

; hfx1/hfx2: the adv-62 host-wraps-match fix (Resolved::Let guard descent, #8109) WITHOUT the
; escaping-closure machinery the pinned adv-62 case couples in — isolating the guard's let-init
; descent on the plain shape. hfx1: a host block wraps a match over a let-bound host result in a
; PLAIN tuple scrutinee `(host (E) (match (let ((v (E.op))) #tuple(v (+ v n))) (#tuple(a b) …)))` —
; the let-init `(E.op)` must be materialized ONCE (the guard descends into the binding init), so the
; op fires exactly once: 7 + 100*(7+n) = 1207 at n=5. hfx2: TWO let-bound host results in the
; scrutinee tuple — each init materialized once, exactly two firings in order (no double, no miss):
; 7 + 100*9 = 907. A guard that missed the let-init (the pre-#8109 bug) re-emitted the host block
; per binder and trapped on the missing second response. (breaker probes hf1/hf2; wasm exact with
; single/double host-call traces, cadenza hop benign-stabilizing drift with identical values,
; census 0.)
(case
  "a host block wrapping a match over a plain-tuple let-bound host result fires the op once"
  (input
    (do
      (effect E (op op (-> Int64)))
      (def
        (main (: n Int64))
        (host (E) (match (let ((v (E.op))) #tuple(v (+ v n))) (#tuple(a b) (+ a (* 100 b))))))
      (export main)))
  (call main (: 5 Int64))
  (host-responses (respond e.op (: 7 Int64)))
  (host-calls (call e.op))
  (output (: 1207 Int64)))

(case
  "a host block wrapping a match over two let-bound host results fires each once in order"
  (input
    (do
      (effect E (op op (-> Int64)))
      (def
        (main (: n Int64))
        (host
          (E)
          (match (let ((v (E.op))) (let ((w (E.op))) #tuple(v w))) (#tuple(a b) (+ a (* 100 b))))))
      (export main)))
  (call main (: 5 Int64))
  (host-responses (respond e.op (: 7 Int64)) (respond e.op (: 9 Int64)))
  (host-calls (call e.op) (call e.op))
  (output (: 907 Int64)))

; ixx1: an INTEGRATION witness — a handler op that BIN-MATCHES its Bytes argument in the arm and
; accumulates the extracted byte into a LIST handler-state, exercising the effect fold + a bin-match
; in an op arm + list-state threading + the list-state reclaim (#8107 family) together. `Acc.feed`
; takes a Bytes, the arm matches `(bin (u8 x) (u8 y))`, resumes with x+y, and threads `List.push s x`.
; feed(#list(n 10)) then feed(#list(3 4)): (n+10) + 1000*(3+4). Value-correct AND the accumulated
; list-state reclaims to 0 (implicit corpus assertion holds — a composition of mechanisms each fenced
; alone, pinned together as a regression witness). n=5 -> 15 + 7000 = 7015; n=0 -> 7010. (breaker
; probe ix1, verified tri-target exact + byte-idempotent + live-objects 0 in-gate.)
(case
  "a handler op bin-matches its Bytes argument and accumulates the byte in list-state"
  (input
    (do
      (effect Acc (op feed (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          Acc
          #list()
          ((feed
              (b)
              s
              (match
                b
                ((bin (u8 x) (u8 y))
                  (resume (+ (Int64.of x) (Int64.of y)) (List.push s (Int64.of x))))
                (_ (resume -1 s)))))
          (+ (Acc.feed (Bytes.of #list((UInt8.of n) 10))) (* 1000 (Acc.feed (Bytes.of #list(3 4)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 7015 Int64))
  (call main (: 0 Int64))
  (output (: 7010 Int64)))

; mkx1: a handler that GROWS a Map handler-state and holds it to completion leaks 1 (value-correct). A
; handler op bin-matches its Bytes arg, uses the extracted byte as a Map key, does Map.lookup then
; Map.insert (a counter), threading the Map handler-state. VALUE exact — two bumps of the same key =>
; count reaches 2, so 1 + 10*2 = 21. But it leaks 1 object. ROOT CAUSE (v-memory-safety rc-trace, superseding
; my original "Map-key-slot" reading): the leaked node#6 is the FINAL threaded Map state at HANDLER
; COMPLETION, not reclaimed — the trx1 family (handler-completion final-state), NOT a key-slot/CHAMP-key-half
; issue. DECISIVE control: a NON-handler with the SAME bin-extracted key into Map.lookup+insert reclaims 0
; (CHAMP/key reclaim is fine outside a handler); mkx1 leaks ONLY because the handler's grown final Map isn't
; dropped at completion. The bin-KEY merely REVEALS the husk (a non-immortal entry makes the un-reclaimed
; final-map husk COUNT; the const-key mvx1 below has an immortal key so the same husk isn't counted → looked
; like 0 — a visibility artifact, not a key-vs-value reclaim difference). v-mem's #8105 compound-shell closed
; the match-scrutinee + drain-to-empty (md2/md3) shapes; this GROW-and-hold final-state shape rides the
; handler-completion reclaim (co-designed with v-effects reduce_handle). Gate counts it accurately (nix-real:
; flipping to (live-objects 0) reds nix corpus-14b, got 1). Pinned known-leak + filed to v-memory-safety.
(case
  "a bin-extracted Map key in a lookup-insert handler-state cycle counts correctly (leaks pending handler-final-state reclaim)"
  (input
    (do
      (effect T (op bump (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          T
          Map.empty
          ((bump
              (b)
              s
              (match
                b
                ((bin (u8 k) (u8 _r))
                  (let
                    ((key (Int64.of k)))
                    (let
                      ((cur (match (Map.lookup s key) ((Some v) v) ((None) 0))))
                      (resume (+ cur 1) (Map.insert s key (+ cur 1))))))
                (_ (resume -1 s)))))
          (+
            (T.bump (Bytes.of #list((UInt8.of n) 0)))
            (* 10 (T.bump (Bytes.of #list((UInt8.of n) 0)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 21 Int64))
  (live-objects known-leak))

; skx1: the SET counterpart of mkx1 — a bin-match-extracted value used as a SET ELEMENT in a
; contains+insert handler-state cycle reads 0 husks in-gate (where mkx1's Map version leaks 1).
; A handler op bin-matches its Bytes arg, uses the extracted byte as a Set element, does
; Set.contains then Set.insert (a seen-set), threading the Set handler-state. Value exact (first mark
; 0/new, repeat 1/seen): mark(n) mark(n) mark(9) -> 0 + 10*1 + 100*(0 if n!=9 else 1). n=5 -> 10;
; n=9 -> 110 (all three mark 9: 0,1,1). NOTE (superseding my original "the leak is specific to Map's
; KEY-VALUE pairing" reading): v-memory-safety's rc-trace localized mkx1 to HANDLER-COMPLETION final-state
; reclaim (trx1 family), not a Map key-slot. So this skx1 0-reading is NOT proof of a key-vs-value/Set
; distinction; it is only an in-gate 0 for the Set handler-final-state shape — whether the Set final-state
; genuinely reclaims or is a husk-visibility artifact (like mvx1's immortal-const-key 0) is for v-mem to
; confirm on nix. Kept as a value + in-gate-balance witness, not a root-cause contrast. (breaker probe sk1,
; verified tri-target exact + byte-idempotent + live-objects 0 in-gate.)
(case
  "a bin-extracted Set element in a contains-insert handler-state cycle reclaims cleanly"
  (input
    (do
      (effect Seen (op mark (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          Seen
          (Set.of #list())
          ((mark
              (b)
              s
              (match
                b
                ((bin (u8 k) (u8 _r))
                  (let
                    ((key (Int64.of k)))
                    (if (Set.contains s key) (resume 1 s) (resume 0 (Set.insert s key)))))
                (_ (resume -1 s)))))
          (+
            (Seen.mark (Bytes.of #list((UInt8.of n) 0)))
            (+
              (* 10 (Seen.mark (Bytes.of #list((UInt8.of n) 0))))
              (* 100 (Seen.mark (Bytes.of #list(9 0))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (call main (: 9 Int64))
  (output (: 110 Int64)))

; mvx1: a bin-match-extracted value stored as a Map VALUE under a CONSTANT key in handler-state reads 0
; husks in-gate (where mkx1's bin-extracted-KEY version reads 1). A handler op bin-matches its Bytes
; arg, inserts (7, extracted-byte) into the Map state, reads it back. put(n) stores n@7 then put(3)
; overwrites: n + 100*3. n=5 -> 305; n=9 -> 309. CORRECTION (v-memory-safety rc-trace, superseding my
; original "the leak is specific to the KEY slot" triangulation): mkx1's leak is the un-reclaimed
; HANDLER-COMPLETION final Map state (trx1 family), NOT a key-slot. This mvx1 0-reading is a husk-VISIBILITY
; artifact: the CONSTANT key 7 is immortal, so the same un-reclaimed final-map husk is NOT counted here —
; the key/value difference is about what makes the husk observable, not about which slot reclaims. So the
; mkx1/skx1/mvx1 trio is a value + in-gate-balance witness set, NOT a key-vs-value reclaim localization.
; (breaker probe mv1, verified tri-target exact + byte-idempotent + live-objects 0 in-gate.)
(case
  "a bin-extracted value stored as a Map value under a constant key reclaims cleanly"
  (input
    (do
      (effect T (op put (-> Bytes Int64)))
      (def
        (main (: n Int64))
        (handle
          T
          Map.empty
          ((put
              (b)
              s
              (match
                b
                ((bin (u8 k) (u8 _r))
                  (let
                    ((val (Int64.of k)))
                    (let
                      ((s2 (Map.insert s 7 val)))
                      (resume (match (Map.lookup s2 7) ((Some v) v) ((None) -1)) s2))))
                (_ (resume -1 s)))))
          (+ (T.put (Bytes.of #list((UInt8.of n) 0))) (* 100 (T.put (Bytes.of #list(3 0)))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 305 Int64))
  (call main (: 9 Int64))
  (output (: 309 Int64)))
