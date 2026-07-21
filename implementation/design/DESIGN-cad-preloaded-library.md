# DESIGN — CAD preloaded library: user /cad output is ONLY the user's model (P5)

*2026-07-16. Operator directive (v-cad reopening, req #4, verbatim): "I don't want the cad dsl to have
to embed the types in the output. It should include the whole library PRE-LOADED so the output is ONLY
the user's model."*

> **STATUS: ✅ SHIPPED (P5 delivered, Option A + ruling A). This doc is now a RECORD, not a proposal.**
> The reconciliation box just below records what shipped; the investigation below is retained as the design
> record (it was run against trunk `2d143ba3b` at design time).

## ✅ What shipped (reconciliation — read this first)

P5 landed as **Option A (preloaded-modules parameter on the browser compile entry)** — the recommended
option, operator ruling A. The reader's /cad buffer + emitted artifact is now ONLY the user's model.

- **Compiler entry** — `implementation/seed/crates/cdz-wasm/src/lib.rs` gained a preloaded-modules compile
  path (`compile_with_preloaded` / `compileWithPreloaded`): the preloaded library modules arrive as
  parallel `names`/`sources`/`formats` arrays, and the user's buffer resolves its `import`s against them.
  The native project-mode resolution mirrored into the browser entry, exactly as Option A proposed.
- **The preloaded library** — `exact.cdz` (+ `helpers.cdz`), authored in ML, passed as the preload set
  (`guide/src/cad/preloadModel.ts`, `CAD_LIB_NAME = "exact"`).
- **The /cad starter** — no longer inlines `type Vec3r/Solidr` + the constructor boilerplate (verified:
  none in `CadPage.tsx`). `CadPage`'s `injectImport` prepends only the pragma + the exact import; the
  reader edits **only the model** (`def main() = lower(...)`), and every picker example is a bare
  single-buffer model.
- **Seam split (as designed)** — the compile-entry/resolver-scope change was compiler/inference +
  cdz-wasm territory; v-cad supplied the preloadable CAD library + the single-buffer example contract;
  v-guide-infra wired the /cad route to pass the preload set. Option C (string-prepend) stayed rejected.
- **FORK RESOLUTIONS:** Q1 → Option A; Q2 (preload form) → parsed `.cdz` sources (ml format string); Q3
  (priority) → shipped alongside/after P1.

The original investigation + options analysis (below) is retained verbatim as the design record.

---


## The problem, located

A /cad program today must **re-declare the entire CAD type vocabulary inline**. The /cad starter
(`guide/src/cad/CadPage.tsx`) opens with:

```
type Vec3r = | V3r(Rational, Rational, Rational)
type Solidr = | Cuber(Vec3r) | Spherer(Rational) | Differencer(Solidr, Solidr) …
def r(n: Int64) = Rational.of(n, 1)
def main() = Solidr.Differencer( … )
```

— and its own comment says why: *"the CAD library modules aren't resolvable in the browser compiler,
so each program defines its own `Vec3r`/`Solidr`."* So every model carries the library's `type`/`def`
declarations as boilerplate, and the compiled output embeds them. That is exactly the "embedded types in
output" the operator wants gone: the user's file (and the emitted artifact) should be **only their
model** — `def main() = difference(cube(...), sphere(...))` — with `Vec3r`/`Solidr`/the constructors
already in scope.

## Root cause

The browser compiler entry is single-text with no module resolution:
- `guide/src/compiler/worker.ts` calls `wasmCompile(text, from)` — one source string.
- `implementation/seed/crates/cdz-wasm/src/lib.rs:250` — `pub fn compile(text: &str, from: &str)`. No
  parameter for imports or a preloaded module set.
- So a browser program cannot `import { Solidr } from "cad"` (the native project-mode resolution that
  `cdz test implementation/cad` uses does not exist in the browser wasm entry).

There IS a preload mechanism in the compiler — the **prelude** (`rcdzc/src/prelude.rs`: "the ONE map of
built-in bindings the resolver consults by name … installed as real AST nodes"). The CAD library is not
part of it (nor should the *language* prelude carry an app library). What's missing is a way to make an
**app-specific library** ambient for a given compile.

## Options

**Option A — a preloaded-modules parameter on the browser compile entry.** Add
`compile(text, from, preload?)` where `preload` is a set of already-parsed library modules (the CAD
`.cdz` sources, or their compiled form) whose exports are in scope for `text`. The /cad route passes the
CAD library; the user's `text` is just their model and references `Solidr`/`cube`/… directly. Mirrors
native project-mode resolution, scoped to the browser entry. Compiler-side (cdz-wasm + the resolver's
module-scope), likely with **v-peer-linking** (cross-module binding) and the compiler-ml/inference owner
(resolver scope).

**Option B — app-prelude injection.** Generalize the prelude so a caller can supply an *additional*
ambient binding map (the CAD library) layered under the language prelude. The resolver already consults
the prelude by name; an app-prelude is a second such map. Cleaner conceptually (reuses the exact
mechanism the operator invoked — "pre-loaded"), but touches the prelude's single-map assumption.

**Option C — string prepend (REJECTED).** Prepend the library source to the user's text before compile.
This is what the starter effectively does inline; it does NOT satisfy the operator (the types still end
up in the compiled output / the user's editable text). Rejected — it's the status quo in disguise.

## Recommendation

**Option A** (preloaded-modules parameter), because it (a) keeps the user's text + emitted artifact to
ONLY their model, (b) mirrors the native project resolution that already works, and (c) is the same
"preloaded library" the operator described. Option B is a cleaner long-term shape if the prelude is being
generalized anyway (coordinate with whoever owns `prelude.rs`).

## Cross-vertical seam (this is NOT solely v-cad's to build)

The compile-entry + resolver-scope change is **compiler/inference + peer-linking territory**, not the
CAD library. v-cad is the **motivating consumer** (and owns the /cad library + the starter that gets
simplified once this lands). Concretely:
- **compiler-ml / v-inference** — resolver module-scope + the `compile` entry signature.
- **v-peer-linking** — cross-module binding of the preloaded library's exports into the user program.
- **v-guide-infra** — the /cad route passes the preloaded CAD library to `compile`; the starter drops the
  inline `type`/`def` boilerplate and becomes just `def main() = …`.
- **v-cad (me)** — supply the CAD library in a preloadable form; rewrite the starter to the minimal
  model once the entry exists; verify the emitted output no longer embeds the type defs.

## Open questions for the operator / seam owners

1. **Option A vs B** — a per-compile preload parameter, or a generalized app-prelude?
2. **Preload form** — parsed `.cdz` sources, or a pre-compiled library component the user program links
   against (the peer-linking path)?
3. Is this worth doing now, or after the higher-priority units work (P1) fully lands? (P5 is req #4; the
   operator listed it without a priority order.)

No code from me until the seam owner + option are chosen. On a ruling, I supply the preloadable CAD
library + simplify the starter.
