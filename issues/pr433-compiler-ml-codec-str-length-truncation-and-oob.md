# PR review comments — mirrored from GitHub PR #433 (Copilot inline) — compiler-ml codec

- **PR:** #433 "fleet: fifty-seventh batch (…, compiler-ml codec Name/Str, …)" (MERGED)
- **Files:** `implementation/compiler-ml/src/encode.cdz:20/41`, `implementation/compiler-ml/src/decode.cdz:32`
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592305584, 3592305662, 3592305747
- **Links:** https://github.com/camshaft/cadenza/pull/433#discussion_r3592305584 (+ r3592305662, r3592305747)

## Comments (verbatim, condensed)
> [encode.cdz:20] `str-payload` encodes the string length into a single byte via `UInt8.wrap`, but doesn't guard against `String.byte-len(s) > 255`. For longer strings this silently truncates [the length → corrupt decode].
> [encode.cdz:41 / decode.cdz:32] `read-str` returns "" when the length-prefixed payload would slice out of bounds. That masks malformed encodings / turns corrupted/partial buffers into an empty `Name`/`Str` silently; prefer trapping.

## Liaison triage — CONFIRMED against trunk
- `str-payload(s) = Bytes.concat(b1(String.byte-len(s)), …)` where `b1(x) = UInt8.wrap(x)` — a symbol
  longer than 255 bytes has its length byte WRAP mod 256, so the decoder reads the wrong length →
  silent corruption. The doc comment claims "255-byte cap is ample" but there is NO guard enforcing it.
  FIX: guard/trap when `byte-len > 255` (or use a multi-byte length).
- `read-str` returning "" on an out-of-bounds slice silently converts malformed/partial buffers into an
  empty `Name`/`Str` (both encode.cdz + decode.cdz). FIX: trap (or return an explicit error) on OOB
  rather than silently emptying.
All in the compiler-ml codec (v-compiler-ml). The str-payload truncation is the higher-severity one
(data corruption for a long symbol). Fixes on `trunk`. Quotes + links in queue file.
