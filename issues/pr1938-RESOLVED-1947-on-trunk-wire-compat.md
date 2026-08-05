# PR #1938 review — cdz-kernel event.rs / event_ast.rs (v-agent-harness) — correctness/wire-compat [VERIFIED]

https://github.com/camshaft/cadenza/pull/1938 (§6 supervision slice-1 — structured `CloseOutcome`
Success-vs-Failure). Copilot (copilot-pull-request-reviewer overview "🟡 not ready" + 4 inline) flags a
frozen-log wire-format break the doc claims not to introduce. VERIFIED against the diff + trunk.

## `encode_close_outcome` prepends a tag byte for `Success`, changing the frozen `Closed` binary encoding → old persisted streams misdecode (Copilot, event.rs:701 & :703) — correctness/wire-compat [VERIFIED]
> `CloseOutcome` is documented as additive/no-wire-break, but the new `encode_close_outcome`/
> `decode_close_outcome` introduces an extra discriminant byte for `Success`, changing the previously
> frozen `Closed` encoding (and therefore event hashes/cause edges) without a versioned migration. …
> encode `Success` identically to the legacy `Payload` encoding and reserve a new non-payload tag for
> `Failure` (e.g. `2`).

VERIFIED. Trunk (origin/main event.rs:583) encodes `Closed{outcome: Payload}` as `out.push(7);
encode_payload(outcome, out)` — where `encode_payload` writes its OWN tag (0=Inline/1=Blob) then the
body. New diff: `Closed` (tag 7) → `encode_close_outcome`, which for `Success(p)` pushes `0` THEN
`encode_payload(p)`. So an OLD stream `[7][0=inline][len][bytes…]` is now read as: tag 7 →
`decode_close_outcome` reads first byte `0` → `Success(decode_payload(c))` → `decode_payload` reads the
NEXT byte (the OLD payload's `len`) as a payload tag. For any non-empty inline payload (`len>=2`) that's
an unknown-tag DecodeError; for a blob it silently mis-parses. Event hashes/cause edges over `Closed`
frames also shift. MED-HIGH (gated on whether durable logs already exist — owner's call), asymmetric-cost
direction. Fix per Copilot: encode `Success` byte-identically to the legacy `Payload` form (no extra tag),
reserve a fresh non-payload tag for `Failure`.

## textual (`event_ast`) `read_body` requires `(closed (success …))|(closed (failure …))`, rejecting legacy `(closed <payload>)` as corruption (Copilot, event_ast.rs:914) — correctness/wire-compat [VERIFIED]
> `read_body` now requires the new `(closed (success ...)) | (closed (failure ...))` shape, so any durable
> logs encoded with the previous `(closed <payload>)` form will start failing to decode as corruption.
> … accept legacy payload heads (`inline`/`blob`) here and interpret them as `CloseOutcome::Success`.

VERIFIED in the diff (event_ast.rs read_body `"closed"` arm rewritten from `[pf]`→`[of]`, now dispatching
on `success`/`failure` heads). Same wire-break in the textual codec. Fix per Copilot: in the `closed` arm,
accept a legacy `inline`/`blob` payload head and map it to `CloseOutcome::Success`.

## `CloseOutcome::Failure` not in the FROZEN `every_variant_round_trips` harness (Copilot, event.rs:1044) — test-precision [VERIFIED, PARTIALLY-COVERED]
> The new `CloseOutcome::Failure` arm isn't exercised by `encode_decode_round_trips_every_variant` … add a
> `Closed { outcome: CloseOutcome::Failure(...) }` case to `all_variants()`.

VERIFIED-with-nuance: the PR DOES add a dedicated `closed_round_trips_both_close_outcome_arms_through_the_
shared_codec` test that pins BOTH Success and Failure (diff :200-225), so the Failure codec is NOT
untested. Copilot's literal ask stands only for the FROZEN `all_variants()`/`every_variant_round_trips`
harness — adding the Failure arm there would guard it under the same frozen-codec net as every other
variant (protects against future codec drift the dedicated test might be updated alongside). LOW —
belt-and-suspenders, since a dedicated test already covers it. Relay as "already covered; also add to
all_variants for the frozen net".

## doc comment claims "no sentinel / no wire break / tolerant decoder" but codec is tag-based and rejects unknown tags (Copilot, event.rs:217) — clarity [VERIFIED]
> The `CloseOutcome` doc comment claims "no sentinel"/"no wire break"/"tolerant decoder", but the codec …
> is tag-based and (by design) rejects unknown tags. … update to describe the actual wire-compat contract.

VERIFIED — the doc's compat claim is falsified by the codec above. Once the Success-legacy /
Failure-new-tag compat approach lands, reword to state what's preserved (Success byte-identical to legacy
Payload) vs what requires a tag reservation (Failure). clarity. v-agent-harness owns cdz-kernel/src.
