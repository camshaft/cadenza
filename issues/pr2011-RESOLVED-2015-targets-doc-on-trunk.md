# PR #2011 review — cdz-agent-host/src/config.rs (v-agent-harness-host) — MERGED — doc-clarity [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2011 ([observability] model target kinds on real backend). Copilot
(id 3711665297) flags the `targets` doc conflates "supports multiple" with "requires >1".

## `[observability]` doc says "define more than one target backend" (>1) but the code validates only non-empty (>=1) (Copilot, config.rs:67) — doc-clarity [VERIFIED]
> The doc comment mixes "one-or-more configured targets" with "operator requirement: define more than one
> target backend". The code validates only non-empty (>=1), and the real requirement here seems to be that
> the schema SUPPORTS multiple backends, not that configs must include >1. Rewording would avoid confusing
> operators and future readers.

VERIFIED on trunk: config.rs:63-64 doc: "metrics FAN OUT to one-or-more configured `targets` (operator
requirement: 'define more than one target backend')." But the validation (config.rs:286) is `if
self.observability.targets.is_empty() { …error… }` — i.e. it requires `>= 1`, NOT `> 1`. So the
parenthetical "more than one" (>1) contradicts BOTH the "one-or-more" (>=1) phrasing right beside it AND
the code. The actual design is "the schema SUPPORTS a LIST of backends (fan-out); at least one is required
when enabled." An operator reading "define more than one" might think a single target is rejected (it
isn't) — or a future reader might add a `> 1` validation to match the doc. LOW/doc-clarity. Fix per
Copilot: reword to "metrics fan out to the configured `targets` — a LIST so multiple backends are
supported; at least one is required when enabled." Drops the misleading ">1" requirement. v-agent-harness
-host owns cdz-agent-host/src.
