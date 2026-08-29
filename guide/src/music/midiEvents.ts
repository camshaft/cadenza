/// Pure, dep-free parser for a rendered `List(MidiEvent)` value → the row data the /music page's event-stream
/// TABLE renders (tick | note | on/off). NO React, NO compiler imports — unit-testable under `node --test`.
/// This is /music's analog of /cad's `index.ts` (turn a rendered machine value into what the page draws), but
/// far simpler: no geometry kernel, just a tuple-list parse.
///
/// A showcase returning `schedule(progression)` renders (in the canonical s-expr the run path emits) as a LIST
/// of MidiEvent tuples, each: `(: (tuple <on?> <chan> <note> <vel> <tick>) MidiEvent)` — field order
/// (on:bool, chan:Int, note:Int, vel:Int, tick:Int). e.g. `(: (tuple true 0 60 90 0) MidiEvent)` is note-on
/// chan0 note60(middle C) vel90 tick0; `(: (tuple false 0 60 0 960) MidiEvent)` its note-off at tick 960.
/// v1 renders the EVENT-STRUCTURE correctness story: every "on" pairs with an "off" (what `balanced()` proves).

/// One parsed MIDI event row (the columns the table shows + the extras the correctness story leans on).
export interface MidiEventRow {
  on: boolean; // true = note-on, false = note-off (the on/off column)
  chan: number;
  note: number; // MIDI pitch (the note column) — 60 = middle C
  vel: number;
  tick: number; // scheduler tick (the tick column) — the stream is sorted/grouped by this
}

/// The result of parsing a rendered value: the event rows (tick-sorted) + whether it's a MidiEvent list at
/// all. `ok: false` with `error` when the value isn't a MidiEvent list (e.g. a showcase that returns a Bool
/// like R1/R3-balanced, or an Int64 list like R2) — the page then falls back to rendering the scalar/list
/// value directly rather than a MIDI table. Never throws.
export type MidiParse =
  | { ok: true; rows: MidiEventRow[] }
  | { ok: false; error: string };

/// Tokenize a rendered s-expr value into parens + atoms (atoms are maximal non-paren/whitespace runs).
function tokenize(s: string): string[] {
  // M2 native-compound render (#5112): render_value emits compounds head-first as `#list(…)`/`#tuple(…)`/
  // `#record(…)`; this parser was written for the legacy `(list …)`/`(tuple …)` form, so normalize the
  // M2 `#head(` spelling back to `(head ` before tokenizing (nested + balanced by construction). Guarded to
  // a name+`(` so it never touches a `#"hashword"` / `#\c` literal. Fixes both this gate + the live /music page.
  s = s.replace(/#([A-Za-z][\w-]*)\(/g, "($1 ");
  const toks: string[] = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === "(" || c === ")") { toks.push(c); i++; }
    else if (c === " " || c === "\n" || c === "\t" || c === "\r") { i++; }
    else {
      let j = i;
      while (j < s.length && !/[()\s]/.test(s[j])) j++;
      toks.push(s.slice(i, j));
      i = j;
    }
  }
  return toks;
}

/// Parse tokens into a nested s-expr tree (arrays of strings/subtrees). Returns null on a paren mismatch.
type Sexp = string | Sexp[];
function parseSexp(toks: string[]): Sexp | null {
  let i = 0;
  function node(): Sexp | null {
    if (i >= toks.length) return null;
    const t = toks[i++];
    if (t === "(") {
      const list: Sexp[] = [];
      while (i < toks.length && toks[i] !== ")") {
        const child = node();
        if (child === null) return null;
        list.push(child);
      }
      if (toks[i] !== ")") return null; // unbalanced
      i++; // consume ")"
      return list;
    }
    if (t === ")") return null;
    return t;
  }
  const root = node();
  return root;
}

/// A MidiEvent form is `(: (tuple <on> <chan> <note> <vel> <tick>) MidiEvent)` — a 3-element list
/// [":", (tuple …), "MidiEvent"] whose middle child is a 6-element list ["tuple", on, chan, note, vel, tick].
function eventOf(form: Sexp): MidiEventRow | null {
  if (!Array.isArray(form) || form.length !== 3 || form[0] !== ":" || form[2] !== "MidiEvent") return null;
  const tup = form[1];
  if (!Array.isArray(tup) || tup.length !== 6 || tup[0] !== "tuple") return null;
  const [, on, chan, note, vel, tick] = tup as string[];
  if (on !== "true" && on !== "false") return null;
  const n = (x: string) => Number(x);
  return { on: on === "true", chan: n(chan), note: n(note), vel: n(vel), tick: n(tick) };
}

/// Parse a rendered value into MIDI event rows, tick-sorted (a stable play-order for the table). Accepts the
/// value with or without an outer `(: (list …) (List MidiEvent))` type wrapper — it finds the MidiEvent forms
/// wherever they nest. Returns `ok:false` when the value contains no MidiEvent form (not a MIDI-list showcase).
export function parseMidiEvents(rendered: string): MidiParse {
  const tree = parseSexp(tokenize(rendered));
  if (tree === null) return { ok: false, error: "unparseable value (paren mismatch)" };
  const rows: MidiEventRow[] = [];
  // Walk the tree; every subtree that IS a MidiEvent form contributes a row (the list may be wrapped in a
  // `(: (list …) …)` type annotation, so we don't assume a fixed nesting — just collect all event forms).
  const visit = (node: Sexp): void => {
    const ev = eventOf(node);
    if (ev) { rows.push(ev); return; }
    if (Array.isArray(node)) for (const child of node) visit(child);
  };
  visit(tree);
  if (rows.length === 0) return { ok: false, error: "value is not a MidiEvent list" };
  // Stable tick sort (the table's play-order); equal ticks keep source order (note-off before a later on).
  rows.sort((a, b) => a.tick - b.tick);
  return { ok: true, rows };
}

/// A MidiEvent stream is BALANCED iff every note-on has a matching note-off (net outstanding returns to 0 and
/// never goes negative) — the "no stuck keys" property `schedule`/`balanced()` proves. The page shows this as
/// a badge; computing it here from the parsed rows lets the table stand alone even when a showcase returns the
/// event LIST (rather than the Bool), and cross-checks the guest `balanced()` verdict. Keyed by (chan, note).
export function isBalanced(rows: MidiEventRow[]): boolean {
  const outstanding = new Map<string, number>();
  for (const r of rows) {
    const key = `${r.chan}:${r.note}`;
    const cur = outstanding.get(key) ?? 0;
    const next = r.on ? cur + 1 : cur - 1;
    if (next < 0) return false; // an off with no matching on
    outstanding.set(key, next);
  }
  for (const v of outstanding.values()) if (v !== 0) return false; // an on with no matching off (stuck key)
  return true;
}
