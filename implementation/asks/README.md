# Asks — the spec/seed decision queue, by lifecycle

**What this is.** Every "ask" the compiler-in-Cadenza conformance loop surfaces — something that needs a
*specification* decision, a *seed* fix, or a *tooling* change — lives here as one file, filed by its
lifecycle state. This replaces the single flat `SPEC-BACKLOG.md`; the content is the same (each ask states
the finding, why it matters, a proposed resolution, and evidence), but now an ask **moves through directories
as it progresses**, so at a glance you can see what needs a decision, what's been implemented but not yet
checked, and what's confirmed landed.

## The three states (directories)

```
asks/
  open/                 needs a decision or an implementation — NOT started
  pending-validation/   implemented (seed/spec/tooling changed) — awaiting the loop's re-probe
  done/                 the loop re-probed and CONFIRMED the fix (or the operator accepted the decision)
```

An ask flows **open → pending-validation → done**, and only ever forward. Nothing reaches `done` on a
claim alone — `done` means *verified against the running artifact* (a re-probe, a corpus case that now
agrees, a byte-identical `component-check`), not "someone said they fixed it."

## Filenames — stable ID + priority

- **`done/` and `pending-validation/`**: `ask-NN-<slug>.md`, where `NN` is the ask's **stable ID** (never
  reused, never renumbered — learnings and commits reference `ask-NN` / `#NN`).
- **`open/`**: `PNNN-ask-MM-<slug>.md` — a `PNNN` **priority prefix** (lower = more urgent) precedes the
  stable `ask-MM` ID, so `ls open/` sorts in priority order. Re-prioritizing = renaming the `PNNN` prefix;
  the `ask-MM` ID is untouched. Priority reflects self-hosting critical path first, then correctness, then
  ergonomics/deferred.

## How the loop maintains this each cycle

1. **New finding** → new `open/PNNN-ask-MM-<slug>.md` (next free `MM`), inserted at its priority rank.
2. **Loop observes an ask was implemented** (seed rebuilt, `compiler.cdz` changed, spec edited) → move its
   file `open/ → pending-validation/`, strip the `PNNN` prefix, and note what changed + what to re-probe.
3. **Loop re-probes and confirms** (the whole discipline of this loop: probe the running artifact, don't
   trust the claim) → move `pending-validation/ → done/`, record the verifying evidence (corpus case,
   byte-identity, reproducer now compiling). If the re-probe *fails*, move it back to `open/` with the
   contradicting evidence — a fix that didn't hold is an open ask again.
4. The loop **reports the same findings to the compiler agent** via the `📡 FROM THE CONFORMANCE LOOP`
   section atop `SEED-GAPS-FOR-SELF-HOSTING.md`. `asks/` is the operator's review queue; that banner is the
   agent's work feed. They stay in sync but serve different readers.

## Status glyphs inside a file

The old legend still appears in each ask's title line as a quick signal, now largely redundant with the
directory:
🔴 open, needs operator decision · 🟠 open, measurement/tooling · 🟡 proposed edit awaiting approval ·
⚪ deferred · 🟢 done. **The directory is authoritative**; the glyph is a hint. (A 🟢 file lives in `done/`.)

## Index

`INDEX.md` lists every ask by ID with its current state and one-line summary — the single view across all
three directories. Regenerate it after moving files.

## Related channels (not asks)

- `SEED-GAPS-FOR-SELF-HOSTING.md` — the compiler agent's work feed (seed implementation gaps + the loop's
  `📡` banner). An ask about seed work cross-references its gap number.
- `RUNTIME-REQUESTS.md` — WIT/runtime component requests.
- `spec/learnings/` — dated post-mortems (the durable "why"); an ask often links the learning that drove it.
