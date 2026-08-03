# PR #1088 review comments — implementation/music/src/interval-vector.cdz (v-music)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1088
(PR: "cand: v-music — interval-vector").

## 1. Module doc misdescribes `Icv` representation (Copilot, interval-vector.cdz:15, also :19) — doc
> The module-level doc claims the vector is returned as an `Icv` *record of named fields*, but `Icv`
> is a single-constructor type with positional components (and only one named accessor
> `ic-tritone`). This is misleading for callers and for future maintenance; reword to describe the
> actual representation (a dedicated `Icv` value + accessor helpers).

## 2. Opaque out-of-range trap message in `icv-count` (Copilot, interval-vector.cdz:92) — minor
> The out-of-range trap message in `icv-count` is a bit opaque when it fires (it doesn't mention the
> function/parameter), which makes debugging harder. Consider making the message self-identifying.

Point 1 is the doc-accuracy one; point 2 is a nice-to-have (self-identifying trap message).
