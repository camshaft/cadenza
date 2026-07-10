# The paren problem is a decoding problem, and the AI-native win is semantic context at the edit point — external research validates syntax-as-projection

*2026-07-09*

**What happened.** A two-pass deep-research investigation asked what an AI-native programming
language — one optimized for LLM/agent authorship and reading rather than for humans — should look
like, and whether Cadenza's central bet (concrete syntax as a *projection* of a homoiconic binary
AST, currently rendered as s-expressions) is the right one. The motivating worry was concrete:
agents, like humans, get stuck balancing parentheses, so an unaided Lisp surface looked like a poor
fit. The investigation (a fanned-out web search with adversarial verification of every load-bearing
quantitative claim, followed by four design agents grounded in this repo) returned findings strong
enough to name here, which is the reason this note exists in the one artifact allowed to cite prior
art by name.

The findings, external prior art named because a learning is historical reference:

1. **The paren-balancing failure is real but is a *decoding* problem, not a syntax problem.**
   Grammar-guided constrained decoding (SynCode, COLM 2024; GCD, EMNLP 2023; PICARD, EMNLP-Findings
   2021; CRANE, ICML 2025) is proven sound and complete with respect to a context-free grammar and
   empirically eliminates 100% of JSON syntax errors and ~96% of Python/Go syntax errors (the
   residual is exactly the context-sensitive off-side rule — indentation). The corollary is decisive
   for Cadenza: **the more regular and CFG-capturable the grammar, the closer to 100% validity comes
   for free**, and s-expressions are maximally CFG-friendly. So the paren worry is an argument *for*
   owning the decoder, not against the surface. There is a documented tool (`agent-lisp-paren-aid`)
   whose own instructions tell agents not to count parens themselves — the failure is real when the
   decoder is unconstrained, and disappears when it is constrained.

2. **The largest measured authorship win is semantic context injected at the edit point, not a
   better surface.** Hazel's "Statically Contextualizing LLMs with Typed Holes" (OOPSLA 2024) — whose
   slogan is *"AIs need IDEs, too!"* — feeds the model the type and binding context of the hole being
   filled, available even in the presence of errors because a totality metatheorem keeps a well-typed
   program sketch always reachable. Measured gains: ~3× from type-definition context, up to 4× from
   error-correction rounds, an order of magnitude for one 15B model from type information. This is the
   same affordance Cadenza already mandates as the queryable oracle
   ([[2026-07-04-the-compiler-is-a-queryable-oracle]]) and solve-once/read-downstream typing
   ([[2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive]]).

3. **Feeding the model a serialized AST *underperforms* fluent surface code** (arXiv 2312.00413;
   GraphCodeBERT chose data-flow over the AST for the same reason). A projection meant to be *read*
   should render to readable code, not a tree dump — pretrained models are fluent in surface syntax,
   not in serialized trees. This qualifies the projection bet: the AST is the right *store* and the
   right *edit target*, but not the right *prompt representation*.

4. **Flattened, linear syntax generates better than deep nesting** (MoonBit's AI-native design;
   the KV-cache/autoregressive argument). Deep s-expression nesting is the one point of tension with a
   pure homoiconic surface — the reading projection an agent sees benefits from an
   A-normal-form/SSA-style linearization even though the store stays the nested tree.

5. **Executable/scripted edits beat text patching, and the win is the validation loop the AST
   enables.** Meta's "Don't Transform the Code, Code the Transforms" (arXiv 2410.08806) is a direct
   head-to-head: an LLM emitting an AST→AST *transform* beat direct rewriting, precision 0.95 vs 0.60,
   F1 0.97 vs 0.75 (on small synthetic transforms — precision-only, N=16). SWE-agent's peer-reviewed
   Agent-Computer Interface ablation (NeurIPS 2024) shows an edit command that *rejects broken edits
   and re-prompts* raises resolution 15.0%→18.0%, and removing the structured editor entirely costs
   7.7 points; edits are the dominant failure surface (a single failed edit drops eventual success
   90.5%→57.2%). Conversely, **wrapping edits in JSON made every model worse** in Aider's tests
   (escaping overhead), and hard grammar constraints degrade *reasoning* (CRANE, up to 10 points) even
   though they fix parsing — so the edit medium must stay close to ordinary code and must leave room
   for reasoning. "Run a program to rewrite code" is otherwise decades-proven at scale (Coccinelle,
   6000+ Linux commits; ClangMR, 35k call sites across 100M LOC; OpenRewrite's typed, format-preserving
   trees; jscodeshift/recast reprinting only changed nodes).

Four design agents then grounded these against the repo and independently converged on one system,
keyed on the two primitives the spec already fixes — content-addressed AST nodes
(`options/structural-interface/content-addressed-nodes.md`) and the compiler as a queryable oracle
(`tooling-and-lsp.md` §"The Compiler Is A Queryable Oracle"): a read-only *flattened, type- and
diagnostic-annotated* projection with progressive disclosure (reading); an edit expressed as a
validated, atomic transaction over semantic selectors, type-checked before it lands (writing); and a
context economy in which an agent holds signatures and node handles rather than source text
(delivery), for a measured ~85–96% context reduction versus loading whole files.

**Why.** The homoiconic-decoupled-display bet
(`options/code-shape/homoiconic-decoupled-display.md`) is *more* right than it first appeared, but for
a reason the original framing understated: because display is decoupled from the one hashable store,
the language is free to offer a projection tuned for a machine reader that it would never accept as a
*stored* syntax — flattened past the tree's nesting, annotated with inferred types and diagnostics
inline, and folded for context economy — precisely because that projection is *read-only* and never
has to round-trip to the canonical bytes. The round-trip obligation
(`agent-authoring.md` §"Textual Syntaxes Round-Trip Through The Canonical Form") is a constraint on a
*syntax* (a stored/edited form), not on a *view* (a read/query rendering); conflating the two would
have foreclosed the highest-value reading affordance the research identified. That distinction —
round-tripping syntax versus non-round-tripping view — is the load-bearing clarification.

Everything else the research prizes, Cadenza had already decided for independently-arrived reasons:
edit the tree not the text (structural interface); a rejection is a machine-readable route to a
compliant program, not an opaque failure ([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]);
transformation is an ordinary `Ast → Ast` program on the same seam as compilation
([[2026-07-04-program-transformation-is-a-program]]); the host is value-agnostic and the compiler owns
the one reader/printer ([[2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer]]); the type
is solved once and read downstream. The external evidence is therefore mostly *vindication* of the
existing normative surface, which is the most useful thing a research pass can return: it says the
spec's agent-facing decisions are the ones a from-first-principles AI-native design lands on, and it
localizes the few genuine gaps rather than proposing a redesign. The one durable *tension* it surfaces
is nesting depth (finding 4): the store stays nested, but the agent-facing reading view should
linearize, so the projection layer — not the AST — carries the flattening.

The honest gaps the pass could *not* close, recorded so they are not mistaken for settled: no rigorous
tokens-per-semantic-unit comparison across syntaxes survived verification (the token-efficiency half
of the question is unmeasured, and the tokenizer the language controls may matter more than the
surface); and there is no non-vendor head-to-head of "execute a transform" versus "emit a diff" for
*general* editing (the Meta result is small and synthetic). Both are cheaply answerable against
Cadenza's own corpus and are the experiment this work motivates rather than concludes.

**The requirement it drove.** No new *behavioral* requirement — the language's runtime meaning is
untouched — and, strikingly, most of the "design" is already required: the structural read/rewrite
interface, deterministic addressing, edit-yields-well-formed-or-rejection, machine-readable diagnostics
with a verified/applicability-tagged fix (`agent-authoring.md` §"Structural Editing", §"Machine-Readable
Output"), the queryable oracle and incremental-equals-batch (`tooling-and-lsp.md`), and decoupled
displays over one binary AST (`code-shape`). This learning drives a fold whose content is the *deltas*
those docs do not yet carry:

1. **A view is not a syntax.** `code-shape` / `agent-authoring.md` should distinguish a round-tripping
   *textual syntax* (a stored/edited form, bound by the round-trip obligation) from a read-only
   *projection/view* (a rendering for reading and query, explicitly exempt from round-trip), admitting a
   flattened, type- and diagnostic-annotated agent-reading view as a first-class projection without
   weakening the canonical-form contract.
2. **An edit is a validated transaction.** `agent-authoring.md` §"Structural Edits Preserve
   Well-Formedness Or Report" should tighten from per-edit well-formedness to an *atomic transaction*
   that lands only a well-typed program or reports diagnostics computed against the rejected candidate —
   the missing normative form of the reject-don't-miscompile discipline on the *write* path.
3. **Context economy is a tooling affordance.** `tooling-and-lsp.md` should add that an agent can
   retrieve a signature/skeleton view and a reachability-scoped slice, so it can hold a large program in
   a bounded context by asking for structure rather than loading whole bodies — the one affordance the
   oracle requirements imply but do not state.
4. **Design choices**, non-normative, touching no frozen contract: a flattened/typed reading-view choice
   under `code-shape`, and a semantic-selector + named-refactoring + transaction extension to the
   `structural-interface` choice (the edit-as-a-program realization of
   [[2026-07-04-program-transformation-is-a-program]], with the edit script itself in the s-expression
   surface so it inherits grammar-constrained validity). The no-proper-names discipline keeps every
   external name in finding 1–5 in this learning alone, never in the capability or choice text.
