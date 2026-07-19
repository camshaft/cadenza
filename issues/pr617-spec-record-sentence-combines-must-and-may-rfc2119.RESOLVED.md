# pr617 — spec core-semantics.md: one sentence combines MUST + MAY (RFC-2119 atomic-obligation style)

Mirrored from GitHub PR #617 review comment (Copilot), id 3609646143.
PR: https://github.com/camshaft/cadenza/pull/617 (7-MR publish batch)
Location: `spec/capabilities/core-semantics.md:215`

## Reviewer comment (verbatim)
> The new normative sentence combines two independent obligations (`MUST` deconstructible + `MAY` subset
> pattern) into a single RFC-2119 sentence. Repo spec guidance requires each requirement to be a single,
> atomic obligation, so this should be split into two sentences under the same stable heading (and any
> duvet citations updated to quote the exact new sentences).

## VERIFIED (git show trunk)
core-semantics.md:215: "A record MUST be deconstructible by pattern matching on its field names, binding
each named field's sub-value; a record pattern MAY name a subset of the fields, ignoring the rest." — one
sentence with a MUST clause and a MAY clause joined by `;`. Copilot's point (split into two atomic RFC-2119
obligations under the same stable heading, update any duvet citations quoting it) is plausible spec-style,
BUT I can't confirm the repo's "one atomic obligation per sentence" convention from here, and the duvet
citation angle is a traceability concern. Genuinely ambiguous ownership (spec authoring + duvet).

## Owner AMBIGUITY (defaulting to PM)
`spec/capabilities/*` normative text + duvet citation sync — no single obvious vertical. Filing to
corpus-bugfix PM to route (or dismiss if the combined sentence is acceptable per actual spec convention).
Low-stakes spec-style nit.

---
BACKLOGGED to concierge (corpus-bugfix 2026-07-19): low-stakes spec-style nit. core-semantics.md:215 combines a MUST (deconstructible by field-name pattern) + a MAY (pattern names a subset) in one RFC-2119 sentence. Copilot suggests split into 2 atomic obligations + update duvet citations. Ambiguous: needs the spec atomic-obligation convention ruling + duvet-citation impact check — spec-authoring/operator call, not a code vertical. Dismiss if the combined sentence is fine per convention. 3rd PR#617 comment (Test.gen doc) is a dup already with v-property-testing.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk 0d8b661f7): the combined RFC-2119 sentence was SPLIT into
two atomic obligations under the same "A Record Has A Fixed Set Of Named Fields" heading (core-semantics.md
~215): "A record MUST be deconstructible by pattern matching on its field names, binding each named field's
sub-value." + separately "A record pattern MAY name a subset of the fields, ignoring the rest." Exactly the
one-atomic-obligation-per-sentence split the reviewer asked for. Spec nit resolved by v-syntax/spec owner. No action.
