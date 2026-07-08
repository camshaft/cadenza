# Authoring the compiler in aspirational Cadenza surfaces the language's real gaps

*2026-07-05*

**What happened.** With the language far enough along to attempt it, the compiler's vertical slice was
authored **in Cadenza itself** — `(module m (def (main) (+ 20 22)))` taken all the way through
`decode → resolve → type-check → lower → serialize → frame` — written **as if every documented
capability were fully realized** (generics, algebraic effects in the language, width-indexed integers,
rows, typed sum-type IR). This was a deliberate design spike, not a buildable artifact: wherever the
current seed would decline, the source carried an inline `DECLINE(<capability>)` marker. The point was
to let the flagship program tell us what to build next, rather than guessing from the roadmap.

The spike lives in the disposable, gitignored `implementation/` tree (the compiler is a regenerable
projection of the specs — [[2026-07-02-compiler-core-restarted-four-times]]), so **its durable output
is not the source but this learning, the sibling design learnings it drove, and the corpus cases it
spawned.**

**Why it worked.** Two things fell out that a roadmap reading had not made obvious:

- **Marker frequency is a prioritization signal.** Counting the `DECLINE` markers across one honest
  end-to-end slice, the distribution was **effects (10) ≫ numeric-model (5) > sum-type-declaration (3)
  > collections (2) > fallible-access (1)**. Effects — the ambient state a compiler carries
  (diagnostics, a fresh-name supply, a unification store) — is the single largest thing standing between
  the language and a compiler authored in it, ahead of the numeric model. That is a genuine tension with
  the operator's M0–M9 ladder, which schedules effects at M6, after the numeric model (M4) and traits
  (M5); it is recorded for the operator to resolve (pull effects earlier, or keep the ladder and let the
  Cadenza-authored compiler wait on M6). It composes with — and sharpens — the gap analysis in
  [[2026-07-05-self-hosting-is-gated-on-generics-the-rest-is-libraries-and-scale]], which had generics as
  the linchpin; the two are not in conflict (generics gates the *type* machinery, effects gates the
  *state* machinery), and a real compiler needs both.

- **Where the language fights the author is a design signal, not a defect.** The spike surfaced four
  findings that each drove a durable change rather than a workaround:
  1. The intermediate representation wants to be a **typed sum**, not the spec-mandated string-tagged
     `Ast` nodes — the finding that drove
     [[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]].
  2. Modeling the lexical environment as an effect forced manual save/restore bookkeeping that an
     immutable map gives for free — the finding that drove
     [[2026-07-05-dynamic-extent-is-an-effect-lexical-extent-is-a-parameter]].
  3. "Record a diagnostic and continue" is genuinely elegant as an effect that resumes with unit — the
     strongest argument *for* effects, and a concrete witness that the effect model earns its keep.
  4. There was **no spec surface for *declaring* an intra-program effect and its typed operations** (the
     corpus only ever *handled* ad-hoc operations) — the gap that drove
     [[2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant]].

  The framing generalizes the standing discipline that Cadenza source is written as a well-typed static
  program even while the seed is permissive ([[2026-07-03-author-cadenza-as-static-even-though-the-seed-is-dynamic]]):
  here the whole flagship program is written against the *finished* language, and the delta from what the
  seed realizes is the backlog.

**The requirement it drove.** No capability requirement changed from the spike *itself* — its value was
as a driver. It directly produced three sibling learnings (the typed-IR, dynamic-vs-lexical-extent, and
effect-declaration entries dated the same day) and a family of `(needs …)`-tagged corpus cases (the
compiler's own idioms: a declared effect with a typed operation, record-and-continue diagnostics,
LEB128, an exhaustive typed-IR serializer). It also confirmed the method itself as reusable: **author
the next-hardest program against the finished language, and read the declines as the prioritized
worklist.**
