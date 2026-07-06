# A recursive consumer of a runtime heap value must be typed Heap, or the compiler diverges

*2026-07-05*

**What happened.** The seed compiler monomorphizes: when a function is applied to an argument whose
compile-time kind is a runtime **heap handle** (`Kind::Heap` — a tuple, sum, list, or bytes value)
and the callee has no runtime representation for that argument by value, `gen_call` **inlines** the
callee at the call site (binds each parameter to its argument node and re-emits the body). This is
how a lambda flows into a higher-order function and how per-call specialization works. It is sound
for a *terminating* inline expansion. It is a trap for a **recursive** function: if a recursive
consumer's parameter is (mis)typed as a scalar rather than Heap, but it is applied to a Heap value,
every recursive call re-triggers the inline — the expansion never bottoms out and the **compiler
itself diverges** (a stack overflow, or a wall-clock hang), rather than emitting a program.

This was hit **twice**, from opposite directions, on the same underlying cause:

- First with **runtime sum-match**: a recursive `sm` folding a runtime linked list
  (`(def (sm xs) (match xs ((Cons (tuple h t)) (+ h (sm t))) ((Nil _) 0)))`). Its parameter `xs`
  defaulted to `Int64` (unconstrained → the inference default), so the `Heap` argument at
  `(sm t)` forced inline recursion and the compile stack-overflowed.

- Then with a recursive **Bytes consumer**: `(def (sumb b i acc) (if (< i (Bytes.len b)) (sumb b (+ i 1) …) acc))`
  reading a runtime byte buffer. Its parameter `b` defaulted to `Int64` for exactly the same reason,
  and `(Bytes.len b)` / `(Bytes.at b i)` did nothing to constrain it, so a runtime buffer argument
  forced inline recursion and the compiler hung for minutes before it was killed.

The fix, in both cases, is the same shape and lives entirely in **inference, not codegen**: the site
that *consumes* a heap value must constrain the consumed operand to `Kind::Heap`, so the parameter
is inferred `Heap`, so the recursive call emits a real runtime `call` (the recursion happens at run
time, bounded by data) instead of an unbounded compile-time inline. Concretely: a **constructor-
pattern match arm** (`(Cons (tuple h t))`) forces its scrutinee to `Heap`; a **Bytes consumer**
(`Bytes.len`/`Bytes.at`/`Bytes.slice`) forces its buffer argument to `Heap`. With the constraint in
place, both programs compile — the sum fold runs and returns its scalar; the Bytes consumer either
runs or *declines cleanly* on a not-yet-emitted step, never hangs.

**Why.** The compiler mixes two lowering strategies for a callee — inline (compile-time β-reduction,
for lambdas and heap-by-value arguments) and call (a real runtime `call`, for scalars) — and chooses
between them by the argument's inferred `Kind`. That makes **the kind lattice load-bearing for
termination of the compiler**, not merely for the correctness of the emitted program. An
under-constrained parameter does not fail safe: the inference default (`Int64`) is precisely the kind
that, combined with a Heap argument, selects the inline path, so a *missing* constraint doesn't
merely produce a worse type — it flips a recursive function onto a non-terminating compilation path.
The general rule this exposes: **every operation that consumes a runtime heap value must publish that
constraint to inference**, because the consumer's parameter kind is what decides inline-vs-call, and
inline-on-a-recursive-Heap-consumer does not terminate. A consumer that reads a heap value but leaves
its operand's kind unconstrained is a latent compiler hang waiting for a recursive caller.

**The requirement it drove.** This sharpens **decline-don't-miscompile**
([Decline, do not miscompile](./2026-07-03-decline-do-not-miscompile.md)) with a corollary that was
implicit but never stated: *a compiler that hangs (or overflows its own stack) on a construct it
cannot yet compile has miscompiled — a non-terminating compilation is not a decline.* "Cannot yet"
must be observably distinct from "does wrong," and an infinite loop is neither observable nor
distinct; it is the worst failure mode because it produces no diagnostic at all. The enforcing
discipline is in the seed's `InferCtx`: a heap-value **consumer** (a constructor-pattern match arm;
the `Bytes.len`/`at`/`slice` intrinsics) constrains its consumed operand to the heap kind, so a
recursive consumer is typed for a runtime `call` and the compiler terminates. This composes with
[type inference is Hindley-Milner](./2026-07-04-inference-is-hindley-milner.md) (a parameter's kind
is the solution derived from *all* its uses, so a consumer use must contribute its constraint) and
with the value-heap runtime work
([the runtime is tag-free](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md)),
where these heap consumers live. It is descriptive of a seed engineering invariant, not a new
language-level requirement — but it is the reason two otherwise-unrelated features (runtime sum-match,
runtime bytes) needed the identical inference change, and it will recur for every future heap
consumer added.
