# Platform-conformance case grammar — the STRAWMAN for corpus-bugfix co-design (O1)

Co-design artifact for the `(platform-case …)` genre reader. corpus-bugfix OWNS all `cdz-corpus`
reader edits (single-writer); v-platform-conformance owns the `xtask` grade path + the
`spec/platform/.gate-baseline`. This file is the concrete sexp + proposed record-line encoding
corpus-bugfix asked for so they can map it to the reader Struct/Arena model. Semantics live in
`DESIGN-platform-conformance-suite.md` (D1–D5, MD1, I1–I5); this pins the exact bytes.

## How it slots into the existing reader

Today `read()` picks up only top-level `(case …)` forms and `parse_case` walks its clauses into a
`Record`. The platform genre is a SIBLING top-level form `(platform-case …)` producing a SEPARATE
record type (a `PlatformRecord`), emitted with its own record-line vocabulary and a genre marker line
so the xtask grade path dispatches on genre. `(case …)` behavior is unchanged.

- `read()` (lib.rs ~140): additionally match `head_name == Some("platform-case")` → `parse_platform_case`.
- New `PlatformRecord` struct + `parse_platform_case` mirroring `parse_case`'s clause-walk style.
- `render` (lib.rs ~170): emit a `platform-case` marker line first, then the session/kickoff/expect/
  end-state lines below, `---`-terminated like today.

## I1 concrete case (single session, single kickoff, drive-to-fixpoint)

```
(platform-case "a counter session folds its kickoff and records count=1"
  (doc "one session, no effects; kickoff -> worker; worker folds the start event into kv[count]=1")
  (session "worker" (reducer
    (do
      (def (fold (: ev Event) (: kv Kv))
        (kv-set kv "count" (+ (kv-get-or kv "count" 0) 1)))
      (export fold))))
  (kickoff "worker" (inbound "start" (: unit Unit)))
  (end-state "worker" (kv "count" (: 1 Int64)) (status quiescent))
  (events-processed "worker" 1))
```

(The reducer program body is illustrative — the real fold signature/prelude is the xtask grade path's
concern, NOT the reader's. The reader treats `(reducer <prog>)` EXACTLY like `(case (input <prog>))`:
normalize the inner program to one-line text via the SAME `normalize_program`, store the text. The
reader does not compile or understand the reducer.)

## I2 case adds handler sessions + expect-effects

```
(platform-case "worker performs `now` (served by clock session) then logs it"
  (session "worker" (reducer <worker-prog>))
  (session "clock"  (reducer <clock-prog>)  (serves "now"))
  (session "logger" (reducer <logger-prog>) (serves "log"))
  (kickoff "worker" (inbound "start" (: unit Unit)))
  (expect-effects
    (effect (from "worker") (family "now"))
    (effect (from "worker") (family "log") (: "t=0" String)))
  (end-state "worker" (status quiescent)))
```

## I3 case adds cross-session messaging

```
(platform-case "worker messages reporter; reporter records seen=1"
  (session "worker"   (reducer <worker-prog>))
  (session "reporter" (reducer <reporter-prog>))
  (kickoff "worker" (inbound "start" (: unit Unit)))
  (expect-messages
    (message (from "worker") (to "reporter") (family "message") (: "done" String)))
  (end-state "reporter" (kv "seen" (: 1 Int64)) (status quiescent)))
```

Negative delivery case: `(expect-delivery-failure (from "worker") (to "ghost"))`.

## AGREED record-line encoding (tab-delimited, one PlatformRecord, `---`-terminated)

Converged with corpus-bugfix (they own the reader). KEY PRINCIPLE (their refinement): every line is
FIXED-ARITY; a list is expressed as REPEATED lines (like `host-call`), never a variable-length
trailing list on one line — the reader parses fixed-column lines far more robustly. So `serves`,
`end-kv`, and the ordered effect/message lists are each one line per element. Values keep the corpus
`(: value Type)` canonical text so the grader reuses value-comparison; ordered lists reuse the
`host-call` stream-order = list-order lowering.

```
platform-case\t<title>                                  (genre marker; grade path dispatches on it)
doc\t<one-line doc>                                     (0 or 1)
session\t<alias>\t<normalized reducer program one-line> (1+, in declaration order)
serves\t<alias>\t<family>                               (0+, one per served family; ties to a session)
kickoff\t<alias>\t<inbound-name>\t<value-form>          (exactly 1)
expect-effect\t<from-alias>\t<family>[\t<value-form>]   (0+, in order; value col omitted when no payload)
expect-message\t<from-alias>\t<to-alias>\t<family>\t<value-form> (0+, in order)
expect-delivery-failure\t<from-alias>\t<to-alias>       (0+)
end-kv\t<alias>\t<key>\t<value-form>                    (0+)
end-status\t<alias>\t<state>                            (0+)  state in {active,quiescent,stalled,closed}
events-processed\t<alias>\t<n>                           (0+)
---
```

Rationale for the flat shape (vs nested): keeps the record stream line-oriented + `\t`-split like
today, so the xtask driver parses it with the same split-on-tab loop and needs no s-expr parser (the
corpus reader's founding constraint: "a thin driver can run each without a parser of its own").

## Open O1 spelling forks (corpus-bugfix's call where it touches the reader shape)

- `kickoff` vs `start`; `serves` vs `handles`; `expect-effects` group vs flat `expect-effect` lines.
  STRAWMAN default: the spelling above. The record LINES are flat regardless; the SEXP grouping
  (`(expect-effects (effect …)…)`) is a reader-side convenience — decide whether the sexp groups or
  is flat `(expect-effect …)` siblings.
- Whether `serves` is a child clause of `(session …)` or a sibling top-level clause keyed by alias.
  STRAWMAN default: child of `(session …)` — reads naturally, and the reader already walks a clause's
  own tail (cf. `message_clause`).
- Markdown-literate twin (O4): add `session`/`kickoff`/`expect-effects`/`expect-messages`/`end-state`
  fence kinds to `markdown.rs`. Defer to a later increment; I1 is `.sexp`-only.
