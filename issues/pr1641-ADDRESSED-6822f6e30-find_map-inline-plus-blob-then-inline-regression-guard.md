# PR #1641 review comment — cdz-agent-host/src/host.rs (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1641 (MERGED).

## `control/summary` selection: first-by-family then inline-check misses a later inline summary (Copilot, host.rs:172) — correctness [VERIFIED]
> Current logic selects the FIRST `control/summary` control effect by family and only then checks whether
> it has an inline payload. If multiple `control/summary` effects are present and the first uses a
> non-inline payload, this returns `None` even if a later `control/summary` carries inline bytes. Scanning
> for the first matching *inline* summary also avoids duplicating the family-compare via
> `ContentType::matches_family`.

VERIFIED against the merged code: host.rs:167-172 does `.find(|ce| ce.request.content_type.family.as_ref()
== effect_ct::SUMMARY).and_then(|ce| match ce.request.payload { Some(Inline(bytes)) => …, _ => None })`.
So the FIRST family-matching summary is picked, and if its payload isn't `Inline`, the whole thing returns
`None` — even if a later `control/summary` IS inline. Fix: fold the inline check INTO the find predicate
(`.find_map` scanning for the first summary whose payload is `Inline`), and use
`ContentType::matches_family` to avoid duplicating the family compare. LOW-MED (multiple `control/summary`
in one fork is unlikely today, but it's a real silent-drop edge). Fix-forward.
