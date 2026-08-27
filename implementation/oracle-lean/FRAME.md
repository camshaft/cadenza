# The `cdz-oracle` wire frame

`cdz-oracle` reads a **request frame** on stdin and writes a **verdict response frame** on stdout.
This realizes OQ-A of `implementation/design/DESIGN-lean-differential-oracle.md` with the doc's
chosen default: a length-prefixed frame that carries nothing but the two frozen byte formats
(`spec/contracts/ast-encoding.md` module bytes and `spec/contracts/deterministic-value-form.md`
value bytes). The frame's own envelope — counts and blob lengths — uses **unsigned LEB128** varints,
matching the varint discipline of the AST encoding contract.

Notation: `uleb` is an unsigned LEB128 varint; `blob = uleb len · <len> raw bytes`;
`str = blob` interpreted as UTF-8; `array<T> = uleb count · T×count`.

## Request (stdin)

```
Request := array<blob>            -- modules: each blob is raw ast-encoding.md bytes
           array<Trial>           -- trials
Trial   := str                    -- entry export symbol
           array<blob>            -- args: each blob is deterministic-value-form.md bytes
           array<blob>            -- hostResponses: fed in call order (deterministic-value-form.md bytes)
```

A malformed request is reported on stderr with a non-zero exit; it never produces a verdict.

## Response (stdout)

```
Response := array<Verdict>
Verdict  := u8 tag
            <payload by tag>
            array<blob>           -- hostCalls: ordered host-call records made by this trial
```

Outcome tags and payloads:

| tag | outcome       | payload                                            |
|-----|---------------|----------------------------------------------------|
| 0   | `Value`       | `blob` — canonical value-form bytes                |
| 1   | `Trap`        | `str` — canonical trap kind                        |
| 2   | `Error`       | `str` — diagnostic code (Phase L4)                 |
| 3   | `Diverges`    | none — fuel budget exhausted                       |
| 4   | `Unsupported` | `str` — reason the oracle declines this trial      |

`Unsupported` and `Diverges` are coverage-gaps the conformance harness **skips**, never a
differential mismatch (design §1.2). The response has exactly one verdict per request trial, in
order.

## Status

L0.1 models the envelope end-to-end but interprets neither modules nor values: every trial yields
`Unsupported`. The format is versioned by being additive-only; module/value interpretation arrives in
L0.2 (AST decode) and L1.1 (evaluation).
