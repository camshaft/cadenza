# Linearity is surgical, not core: immutability+RC already covers memory; graded types are the aim

*2026-07-04*

**What happened.** The question "do we adopt linear/affine types?" is resolved: **no** as a mandatory
core discipline, **yes** in three surgical, opt-in places. The reasoning turns on separating two axes
that "linear + liquid" bundles together:

- **Liquid/refinement types** constrain *which values* inhabit a type (`{v : Int | v > 0}`) — the
  verification story, handled separately ([[2026-07-04-refinements-are-liquid-verification-is-extrinsic]]).
- **Linear/affine types** constrain *how many times a value is used* — mostly a *memory/resource*
  story, and only secondarily a verification one.

**Why linearity is NOT mandatory core.** The usual reason to reach for linear/affine types is
**memory** — safe in-place update, no aliasing hazards. That gap is already closed:
**immutability makes the heap acyclic, which makes reference counting complete, and Perceus-style reuse
gives in-place update automatically when a reference is unique**
([[2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete]]). That reuse analysis *is*
affine reasoning — but it runs **in the compiler, invisibly**, exactly as in Koka and Lean 4. Surfacing
full linear/affine types as mandatory core would therefore:
- **Fight the chosen surface.** Persistent, structurally-shared immutable data structures are
  *deliberately aliased* — sharing is the point. Linearity (use-once, no sharing) pulls the opposite
  way.
- **Re-add the tax being avoided.** Rust's ownership is affine-types-plus-borrowing — the heavyweight,
  high-ceremony model the language leaned *away* from — to buy memory safety immutability + RC already
  provides.

**Where affine reasoning DOES earn its keep (surgical, opt-in):**
1. **One-shot continuations.** Effect continuations are affine — resumable at most once — which is what
   keeps fuel accounting sound and RC deterministic
   ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]). A localized linear
   constraint the language already commits to; it must not be generalized to all values.
2. **Linear capability / resource handles.** A capability token or external resource that must be used
   *exactly once* — opened-then-closed, spent-once — is a perfect narrow use of affinity. This is
   **typestate**, and it is high-value in the smart-contract domain (a coin that cannot be
   double-spent). It touches the core domain, so it is the one place linearity reaches toward the
   capability model rather than staying purely in a layer.
3. **An opt-in usage-verification layer, later.** Linearity as an *optional* verification layer — assert
   "this value is used linearly," compiler checks it — fits the progressive-verification frame (`VIII`)
   exactly. It is the *usage/protocol* complement to liquid types' *value-constraint* checking, in the
   same tier as contracts and refinements. Not core; a layer.

**The frame to aim at (course-setting, not build-now).** The modern synthesis of these two axes is
**graded / quantitative types**: multiplicities (`0 / 1 / ω` — erased / linear / unrestricted) as
annotations over a resource algebra (**Idris 2's Quantitative Type Theory**, **Granule**). Worth
*aiming* at even if never built, because **QTT's `0` multiplicity is proof-irrelevant / erased** — the
same mechanism that expresses linearity is the one that cleanly reconciles a verification layer with the
standing requirement that **types are erased from the component**
([[2026-07-04-static-typing-is-mandatory-post-pivot]]). The spec should be structured so a graded/QTT
treatment could land later without a rewrite, not built now.

**Net.**
| Question | Verdict | Where |
|---|---|---|
| Liquid / refinement types | **Yes** | the refinement verification layer ([[2026-07-04-refinements-are-liquid-verification-is-extrinsic]]) |
| Linear/affine as mandatory core | **No** | immutability + RC already covers memory |
| Affine, surgically | **Yes** | one-shot continuations; linear capability handles; optional usage layer |
| Graded / quantitative types | **Aim at, don't build** | keep the erasure / `0`-multiplicity door open |

**The requirements it drives.** No new mandatory-core requirement — the point is that linearity is
*absent* from the core. `spec/capabilities/memory-and-resource-model.md` §Aliasing is annotated that the
compiler's uniqueness/reuse analysis is internal affine reasoning, not a surfaced linear-type discipline.
`spec/capabilities/capabilities-and-effects.md` records that a capability/resource handle MAY be
constrained to affine (exactly/at-most-once) use, and that effect continuations are one-shot (shared
with [[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]). `verification-layers.md`
notes an optional usage-verification (linearity) layer as future work alongside contracts and
refinements, and that the eventual unifying frame is graded/quantitative types with an erased `0`
multiplicity. This is course-setting: it constrains what later generations may add so the door stays
open, per the operator's aim to "start the language in the right direction even if it's not implemented
right now."
