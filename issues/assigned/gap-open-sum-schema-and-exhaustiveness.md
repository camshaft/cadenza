# Gap: open-sum schema decode + open-tail match exhaustiveness (4 standing gate TODOs)

**File:** `spec/semantics/15-rows-and-open-sums.sexp` — 4 cases grade TODO:
- "an open sum's payload decodes against a schema to a typed result"
- "an open sum payload that does not match its schema yields a typed failure, not a trap"
- "a match on an open sum with an open-tail arm is exhaustive"
- "a match on an open sum omitting the open-tail arm is rejected"

**Confirmed:** all 4 are standing todos on current trunk (37 pass / 4 todo / 0 fail in this file).

Implement open-sum schema decoding (payload decodes to a typed result; a schema mismatch is a TYPED
failure, not a trap) and open-tail match exhaustiveness (an open-tail arm makes the match exhaustive;
omitting it is a rejection). Confirm the exact semantics against the SPEC TEXT (rows-and-open-sums /
type-system spec), not the impl gloss. The 4 cases already exist as todos — make them pass.

Area: rcdzc (infer + lowering for open sums). This is a coherent feature slice. Coordinate with
v-patterns (owns match exhaustiveness) for the open-tail-arm exhaustiveness cases.
