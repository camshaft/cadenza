# DESIGN — Configurable Integer Overflow Behavior

> **STATUS: DRAFT PROPOSAL — not for build.** Authored by `v-spec-oracle` for operator review (relayed
> via `concierge`). Layers on the 2026-08-29 operator ruling *"trap on any overflow unless the caller
> explicitly asks for wrapping arithmetic"* (numeric-model §Overflow). The trap-default backend fix
> (routed to v-rust-backend/v-core-opt) proceeds independently of this configurability layer.

## 1. Motivation

The ruling fixes the LANGUAGE DEFAULT: an unqualified `+`/`-`/`*` traps on overflow for every integer
type (signed and unsigned); the named forms `Int64.wrapping-add` … are the explicit opt-in. The operator
then floated making that default **configurable**, verbatim: *"a compiler setting that is able to define
what the overflow behavior should be, for both signed and unsigned. Might even be nice to override it per
operator? But we should also have a module level pragma that … takes precedence over global settings."*

This proposal turns the fixed default into a **resolved-per-operation policy** selected by a precedence
stack, without weakening the local-reasoning guarantee that a named form (`wrapping-add`) always means
what it says.

## 2. Selectable behaviors (VALUES)

The overflow behavior of an unqualified arithmetic operator resolves to one of:

- **trap** — overflow raises a trap (the language default / the ruling).
- **wrap** — two's-complement modular result (what `Int64.wrapping-add` produces explicitly today).
- **saturate** *(proposed, operator to confirm)* — clamp to the type's min/max. Natural third mode; the
  codegen machinery is uniform. If adopted it also needs a named per-op form (`Int64.saturating-add`) for
  symmetry with wrap.

Selectable **separately for signed and unsigned** (operator explicit). NOTE: the **overflow-fallible**
form (Option-returning, `checked-add`) is NOT a selectable *mode* — it changes the RESULT TYPE, so it can
never be a silent ambient default; it stays a named per-op form only. Only trap/wrap/(saturate) — which
preserve the result type — can be ambient modes.

## 3. GLOBAL setting

Two mechanisms, and a hard constraint:

- **Recommended: a project-manifest field** (`Project.cdz`), e.g.
  `[numeric] overflow-signed = "trap"`, `overflow-unsigned = "trap"` (defaults trap/trap = the ruling).
- **Optional: a compiler flag** (`--overflow-signed=…`) for one-off builds, overriding the manifest.

**⚠ DETERMINISM CONSTRAINT (hard).** A setting that changes arithmetic *semantics* is a reproducibility
hazard: the same source under different settings computes different results (or traps vs wraps). Cadenza's
determinism/reproducibility principle requires a program's meaning to be fixed by its source + its
reproducible build inputs, NOT by an ambient invocation flag. Therefore the effective overflow policy MUST
be part of the **reproducible build envelope** — captured in the manifest and folded into the build's
content hash — so a compiled artifact's behavior is determined by source+manifest, not the command line.
A bare `--overflow=` flag that silently changes semantics without entering the hash is disallowed. (Route
to the hash/manifest owner — see §8.)

## 4. MODULE-LEVEL PRAGMA

- **Syntax (proposed):** a module-top declaration, e.g.
  `(pragma overflow (signed wrap) (unsigned trap))`, or split `(pragma overflow-signed wrap)`.
- **Precedence:** OVERRIDES the global setting (operator explicit).
- **Scoping — RESOLVED RECOMMENDATION (definition-site / lexical):** the pragma fixes the overflow mode of
  the operators *written in that module*, at their definition site — NOT the call site. So a function's
  arithmetic behaves per ITS OWN module's pragma regardless of who calls it. This preserves modularity: a
  library function's meaning does not change because a caller's module set a different pragma. (Alternative
  — dynamic/call-site resolution — is rejected: it makes a function's semantics caller-dependent, breaking
  local reasoning and the determinism of a compiled function.)

## 5. PER-OPERATOR OVERRIDE — resolving the operator's ambiguity

The operator asked *"might even be nice to override it per operator?"* — three readings:

- **(i) per operator KIND** (`+`/`-`/`*` independently) — e.g. `+` wraps but `*` traps, set globally or
  per module. A coarse knob.
- **(ii) per call-SITE / expression** — a single operator occurrence overridden. **This is exactly what
  the existing named forms already provide** (`Int64.wrapping-add` is a per-call-site wrap override).
- **(iii) per operand TYPE / width** — e.g. all `Int32` wrap, `Int64` traps (finer than signed/unsigned).

**RECOMMENDATION:** read "per operator" as **(ii) per call-site, already satisfied by the named forms** —
so *no new per-call surface is needed*; the named forms ARE the per-operation override. The genuinely new
work is the **global + module-pragma default** that the *unqualified* operator selects. Offer **(i)
per-operator-kind** as an optional extra knob if the operator wants it (small addition: the pragma/global
gains per-kind keys). **(iii) per-type** is likely subsumed by signed/unsigned + width being part of the
type; present only if the operator wants per-width control.

## 6. PRECEDENCE STACK (explicit, most-specific wins)

1. **Named per-operation form** (`Int64.wrapping-add` / `saturating-*` / `checked-*`) — ALWAYS wins.
2. *(if adopted)* per-operator-kind override.
3. **Module pragma.**
4. **Global setting** (manifest; flag override of manifest for one-off, subject to §3 determinism rule).
5. **Language default: TRAP** (the ruling).

## 7. How it layers on the named-wrapping opt-in

The named forms stay **distinct and authoritative** — top of the precedence stack, **immune to any ambient
mode**. An author who writes `Int64.wrapping-add` MEANS wrap, and no global/module setting can silently
turn it into trap or saturate. They are the escape hatch that preserves *local* reasoning under any ambient
policy. They are NOT sugar for a settings-flippable per-op override. (Recommend keeping them exactly as
today; the configurability layer only changes what the *unqualified* operator resolves to.)

## 8. Interaction with oracle + backend + mechanism owners

**Central mechanism:** the resolved overflow mode MUST become a property of **each arithmetic AST/IR node**
(resolved at compile time from the precedence stack), so that const-fold, the backend codegen, and the
oracle all read the same per-operation mode. Leaving it implicit/ambient would let the three drift.

- **Backend codegen** (v-rust-backend / v-core-opt): each arithmetic op emits trap-check / wrap / saturate
  per its resolved node mode. The signed-overflow-check emit I just routed IS the "trap mode" codegen; wrap
  = unchecked op; saturate = clamp.
- **Oracle** (v-lean-oracle): models each arithmetic node per its resolved mode (not a single global
  assumption). Requires the mode to be encoded on the node.
- **Manifest + flag mechanism:** coordinate with the `Project.cdz` manifest / compiler-flag owner
  (v-cdz-crate-split / xtask, or v-compiler-primitives) — flag concierge to route.
- **Module-pragma parsing + resolution:** coordinate with the reader/parser + module-resolution owner
  (v-inference owns resolution/module surface) — flag concierge to route.
- **Reproducibility envelope:** coordinate with the hash/manifest owner (v-platform) — the effective policy
  must enter the build's content hash (§3).

## 9. Open questions for the operator

1. **Modes:** trap/wrap only, or also **saturate**?
2. **"Per operator":** confirm it means per-call-site (**already = named forms**, no new surface) vs adding
   per-operator-kind (`+`/`-`/`*`) knobs vs per-width. Recommendation: named forms already cover it.
3. **Named forms immune to ambient mode?** Recommended YES (local reasoning); confirm.
4. **Global setting as manifest field (reproducible) vs flag** — the determinism rule (§3) requires the
   effective policy to enter the build hash. Confirm manifest-primary.
5. **Pragma scoping** definition-site (recommended, modular) vs call-site — confirm.
6. **Default stays TRAP/TRAP** for signed/unsigned (the ruling) when nothing is set — confirm.
