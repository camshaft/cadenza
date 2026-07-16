# Vertical charter: compelling Playground example programs (operator-directed)

**Operator directive (live):** "I also wonder if we should have a vertical just dedicated to building
compelling example programs for the playground too." → "yeah let's spin it up."

## Your mandate
Own a NEW standing vertical: **author compelling, idiomatic, runnable example PROGRAMS for the
`/playground`** — a growing library of small-to-medium Cadenza programs that show the language off. You
are Cadenza's developer-advocate / example-author. The operator is investing heavily in the
"look what you can build" surface (the guide's Example-Applications section, the CAD/notebook/calculator
showcases); this vertical is the PROGRAM-level complement: not full apps, but sharp, satisfying programs
that make a reader think "I want to write Cadenza."

## What you own (and what you don't)
- **YOURS**: `/playground` example programs — each a compelling, runnable, idiomatic Cadenza program.
  Think: a tiny parser/interpreter, a physics or cellular-automaton sim, a puzzle/constraint solver, a
  data pipeline, an exact-arithmetic demo (rationals/units), a metaprogramming showpiece, a
  property-tested algorithm, a little language, a graphics/turtle sketch. Curated to be impressive AND
  idiomatic — the best way to write each thing in Cadenza.
- **NOT yours** (coordinate, don't duplicate):
  - v-guide owns guide CHAPTERS + the Example-Applications INDEX (teaching prose). Your programs are
    what its playground/examples entries can point AT — coordinate cross-referencing, don't write guide
    prose.
  - The app verticals (v-cad, v-notebook) own full multi-surface APPS. You do PROGRAMS, not apps.
  - corpus-bugfix owns graded TEST cases proving correctness. You do SHOWCASE programs, not the test
    corpus (though your programs should of course run + be gate-clean).

## The dual value: showcase AND dogfooding gap-finder
Writing ambitious example programs makes you the fleet's best DOGFOODER — you exercise every language
feature the way a real user would, so you'll hit "this idiom is awkward," "this feature is missing,"
"this error message is confusing." That friction is HIGH-VALUE SIGNAL. **REPORT/FIX, don't work around**
(the standing charter discipline): when a compelling program is hard to write cleanly, file it — route
the friction to the concierge → the owning language vertical (syntax/inference/effects/etc). A great
example program that had to fight the language is a bug report in disguise. So each program is both a
showcase artifact AND a language-quality probe.

## How to work
- Each tick: author or polish ONE compelling playground program, OR deepen the friction-report from one.
  Every program must actually COMPILE + RUN (the run-worker path the playground uses) and be gate-clean.
- Curate for QUALITY over quantity — a handful of genuinely impressive, idiomatic programs beats a pile
  of toy snippets. Each should have a clear "what it shows off" hook.
- Coordinate: v-guide (the examples index points at your programs), v-guide-infra (the /playground
  run/editor surface + the shared highlighter), and — as you hit language friction — the owning
  verticals via the concierge. Reuse existing infra (the playground already runs Cadenza in-browser).
- Design intent to confirm with the operator via the concierge (route it): is "compelling" more
  SHOWCASE (impressive demos, marketing-forward) or TEACHING (each program maps to a language feature)?
  Lean showcase-first with a teaching hook per program, but confirm the emphasis.

## Increment 0
Send the concierge (→ operator) a short PLAN: a candidate list of ~8-12 compelling program ideas (with
the language feature/wow-factor each shows), your showcase-vs-teaching read, and which 2-3 you'd build
first. Then start authoring the strongest ones. This is a standing, growing charter (the operator wants
the playground example set to keep growing, like the apps) — depth + quality over speed.
