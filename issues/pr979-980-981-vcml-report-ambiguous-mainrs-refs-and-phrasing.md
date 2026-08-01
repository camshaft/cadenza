# PR#979/#980/#981 review comments — vcml conformance-db report: ambiguous `main.rs:NNN` refs + truncated phrasing (v-compiler-ml)

Three Copilot review comments, all on the SAME file
`issues/vcml-REPORT-conformance-db-vs-differential-rcdzc.md` — a v-compiler-ml authored REPORT (conformance-db
vs differential-rcdzc analysis), mirrored into `issues/` by the fleet-archive commit (`b1c05ecfa`; the
CONTENT owner is v-compiler-ml, not the mirror). ids `3694721524` (:7, +11, +20), `3694737150` (:66),
`3694750772` (:83).

## Comments (verbatim)

- (id 3694721524, :7, also :11/:20) "The code reference `GateTarget::CadenzaMl` is in `xtask/src/main.rs`,
  but this line cites it as `main.rs:920`, which is ambiguous (there are multiple `main.rs` files in the
  repo). Use the full path to keep the pointer unambiguous."
- (id 3694737150, :66) "The last sentence uses unclear phrasing: 'Held pending operator go on (a).' It
  reads like a truncated 'go-ahead' and makes the status hard to understand when skimming the report."
- (id 3694750772, :83) "The reference to where `report_ml_conformance` 'warns but never reds' is
  ambiguous as written (`main.rs:3976`); the code lives in `xtask/src/main.rs`, and the relevant line is
  in that file. Using the full path keeps this note accurate and searchable."

## Liaison verification (confirmed on trunk b1c05ecfa)

The report cites `report_ml_conformance` at `xtask/src/main.rs:3977` (full, once) but then uses BARE
`main.rs:920`, `main.rs:4066`, `main.rs:3976` for `GateTarget::CadenzaMl`, `ml_agrees_with_oracle`, and the
"warns but never reds" step. `GateTarget::CadenzaMl` grep confirms it's in `xtask/src/main.rs` — but the
repo has multiple `main.rs` (cdz, cdz-run, xtask, …), so a bare `main.rs:NNN` is ambiguous for a reader/
searcher. Copilot right on :7/:11/:20/:83 — prefix all with `xtask/src/main.rs`. And :66 "Held pending
operator go on (a)." IS awkward — reads like "operator go-[ahead] on (a)" truncated; reword to e.g. "Held
pending operator sign-off on step (a)". All DOC/report-precision, behavior-neutral.

Owner: **v-compiler-ml** (they authored the `vcml-REPORT-...` conformance analysis; it's mirrored into
`issues/` but the source/content is theirs). Fix: full `xtask/src/main.rs` paths on the bare `main.rs:NNN`
refs (:7/:11/:20/:83) + reword the truncated ":66" status line. NOTE: if this report lives only as a
mirrored archive snapshot (not a v-compiler-ml working file they can edit), it re-mirrors from their source
— fix at the source note; if they don't own the source either, bounce to the archive/concierge owner.
