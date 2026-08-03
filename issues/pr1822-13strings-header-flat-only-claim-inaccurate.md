# PR #1822 review comment — spec/semantics/13-strings.sexp (breaker) — OPEN

https://github.com/camshaft/cadenza/pull/1822 (2-pin cross-leaf UTF-8 validation; author breaker via gh).

## New header claims the earlier from-bytes pins "validate FLAT buffers" — but a cross-leaf seam pin already exists above (Copilot, 13-strings.sexp:4356) — doc/accuracy [VERIFIED]
> This header says the earlier `from-bytes` pins above only validate FLAT buffers, but the file already
> contains a cross-leaf `Bytes.concat` seam-split UTF-8 pin (e.g. the two-byte é case). [So the "only
> flat" framing is inaccurate.]
VERIFIED on the PR head: the new header (~:4353) reads "the from-bytes pins ABOVE validate FLAT buffers;
these pin the CROSS-LEAF walk" — but an existing case at :2684 is "String.from-bytes validates a
multi-byte scalar straddling a runtime byte-rope's SEAM" (a cross-leaf/seam case, NOT flat). So the "pins
above validate FLAT buffers" claim over-generalizes — a cross-leaf pin already exists above. Reword the
header to distinguish what these NEW pins uniquely cover (e.g. 3-byte scalar split across BOTH seams of a
3-leaf rope + torn-continuation-in-next-leaf) vs "flat only". LOW/doc — fold into the next 13-strings edit
per the no-standalone-polish steer.
