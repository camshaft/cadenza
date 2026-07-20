/// The `/music` route — a live browser run of a Cadenza music-theory model: edit a program that builds a
/// chord progression from exact rational intervals and schedules it, Run it, and see the resulting MIDI
/// event stream as a table with a "no stuck keys" (balanced) verdict. Mirrors /cad's shape (an editable
/// program over the real language, executed in-browser via a preloaded library), but the result is an event
/// structure rendered as a TABLE + badge instead of geometry. Part of the operator's "demos into the guide".
///
/// THE SPLIT (confirmed with v-guide + v-music): this vertical owns the route + shell + the page mechanism
/// (compile-against-preloaded-libs, run, parse+render the event table). v-music owns the music/*.cdz libs
/// (the feature semantics); v-guide authors the showcase MODELS (examples.ts) + narrative. v1 renders the
/// EVENT-STRUCTURE correctness story (schedule()→balanced() no-stuck-keys), NOT Web Audio synthesis.
///
/// PRELOADED LIBRARY: the reader's buffer holds ONLY the model — the music vocabulary (interval-ratio/chord/
/// pitch/piece/schedule/…) is link-merged at compile via `compileWithPreloaded`. The host AUTO-INJECTS the
/// import clauses (musicPreload.injectImport) before compiling, so the buffer stays clean.
///
/// SURFACE: /music respects the global surface toggle for EDITING (a per-surface starter), but the compiled
/// value is always RUN + rendered in s-expr (`run(component, "sexpr")`) — the MidiEvent parser reads the
/// canonical machine form, not the display surface (same as /cad's driver).

import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { compileWithPreloaded } from "../compiler/client.ts";
import { run as runComponent } from "../runner/client.ts";
import { useSyntax } from "../syntax/SyntaxContext.tsx";
import { SyntaxToggle } from "../syntax/SyntaxToggle.tsx";
import { LazyCodeEditor } from "../editor/LazyCodeEditor.tsx";
import { wrapPrefixOf } from "../components/wrapModule.ts";
import { injectImport, MUSIC_PRELOAD_NAMES, MUSIC_LIB_FORMAT } from "./musicPreload.ts";
import { parseMidiEvents, isBalanced, type MidiEventRow } from "./midiEvents.ts";
import { EXAMPLES, DEFAULT_EXAMPLE } from "./examples.ts";
import { resolveExampleParam, writeExampleParam } from "../components/exampleParam.ts";
import type { Surface } from "../compiler/client.ts";

// The staged music library sources (?raw text), preloaded so a bare buffer's imports link-merge. Regenerated
// by `cargo xtask guide-wasm` (stage-wasm.mjs), gitignored — same pattern as /cad's EXACT_CDZ etc. Kept in
// lockstep with MUSIC_PRELOAD_NAMES: one raw import per preloaded module, in the same order.
import SCHEDULE_CDZ from "../wasm/music/schedule.cdz?raw";
import PITCH_CDZ from "../wasm/music/pitch.cdz?raw";
import INTERVAL_RATIO_CDZ from "../wasm/music/interval-ratio.cdz?raw";
import SCALE_RATIO_CDZ from "../wasm/music/scale-ratio.cdz?raw";
import SCALE_CDZ from "../wasm/music/scale.cdz?raw";
import CHORD_RATIO_CDZ from "../wasm/music/chord-ratio.cdz?raw";
import CHORD_CDZ from "../wasm/music/chord.cdz?raw";
import RHYTHM_CDZ from "../wasm/music/rhythm.cdz?raw";
import RHYTHM_RATIO_CDZ from "../wasm/music/rhythm-ratio.cdz?raw";
import COMPOSE_CDZ from "../wasm/music/compose.cdz?raw";
import PIECE_CDZ from "../wasm/music/piece.cdz?raw";

// The preloaded sources, aligned 1:1 with MUSIC_PRELOAD_NAMES (schedule, pitch, interval-ratio, scale-ratio,
// scale, chord-ratio, chord, rhythm, rhythm-ratio, compose, piece).
const PRELOAD_SOURCES = [
  SCHEDULE_CDZ, PITCH_CDZ, INTERVAL_RATIO_CDZ, SCALE_RATIO_CDZ, SCALE_CDZ,
  CHORD_RATIO_CDZ, CHORD_CDZ, RHYTHM_CDZ, RHYTHM_RATIO_CDZ, COMPOSE_CDZ, PIECE_CDZ,
];
const PRELOAD_NAMES = [...MUSIC_PRELOAD_NAMES];
const PRELOAD_FORMATS = PRELOAD_NAMES.map(() => MUSIC_LIB_FORMAT);

/// The page's run outcome. `events` = a MidiEvent stream (render the table + badge); `scalar` = any other
/// value (a Bool verdict, an Int64 list — render the raw text); `error` = a compile/run failure.
type Status =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "events"; rows: MidiEventRow[]; balanced: boolean }
  | { phase: "scalar"; text: string }
  | { phase: "error"; message: string };

export default function MusicPage() {
  const { surface } = useSyntax();
  const initialSlug = resolveExampleParam(EXAMPLES.map((e) => e.slug), DEFAULT_EXAMPLE.slug);
  const [exampleSlug, setExampleSlug] = useState(initialSlug);
  const example = EXAMPLES.find((e) => e.slug === exampleSlug) ?? DEFAULT_EXAMPLE;
  const [source, setSource] = useState(() => example.source[surface] ?? example.source.ml);
  const [status, setStatus] = useState<Status>({ phase: "idle" });
  const runningRef = useRef(false);

  // The IDE linter config — diagnose the buffer in its live edit surface against the preloaded music vocab
  // (else every imported name faults as unbound). `prepare` auto-injects the imports (same as `runModel`);
  // `wrapPrefixBytes` maps a squiggle back onto the reader's buffer. surface via ref so the extension array
  // isn't rebuilt on toggle. Mirrors /cad's cadIde.
  const surfaceRef = useRef(surface);
  surfaceRef.current = surface;
  const musicIde = useRef({
    surface: () => surfaceRef.current,
    prepare: (editorText: string, from: Surface) => {
      const compiled = injectImport(editorText, from);
      return { compiled, wrapPrefixBytes: wrapPrefixOf(editorText, compiled) };
    },
    preload: () => ({ names: PRELOAD_NAMES, sources: PRELOAD_SOURCES, formats: PRELOAD_FORMATS }),
  }).current;

  // THE run path: inject imports + export (buffer stays clean), compile against the preloaded music libs, run,
  // and RENDER IN S-EXPR (the parser reads the canonical machine form). A MidiEvent list → the event table +
  // balanced badge; any other value → its raw text (a Bool verdict / an Int64 list). Serialized via runningRef.
  const runModel = useCallback(async (src: string, from: Surface) => {
    if (runningRef.current) return;
    runningRef.current = true;
    setStatus({ phase: "running" });
    try {
      const program = injectImport(src, from);
      const out = await compileWithPreloaded(program, from, PRELOAD_NAMES, PRELOAD_SOURCES, PRELOAD_FORMATS);
      if (!out.component) {
        const d = out.diagnostics.find((x) => x.error) ?? out.diagnostics[0];
        setStatus({ phase: "error", message: d ? `${d.code} ${d.message}` : "compile declined" });
        return;
      }
      const result = await runComponent(out.component, "sexpr");
      if (result.kind !== "value") {
        const msg = result.kind === "trap" ? `trap: ${result.message}` : result.kind === "timeout" ? "timed out" : `error: ${result.message}`;
        setStatus({ phase: "error", message: msg });
        return;
      }
      const parsed = parseMidiEvents(result.text);
      if (parsed.ok) setStatus({ phase: "events", rows: parsed.rows, balanced: isBalanced(parsed.rows) });
      else setStatus({ phase: "scalar", text: result.text });
    } catch (e) {
      setStatus({ phase: "error", message: e instanceof Error ? e.message : String(e) });
    } finally {
      runningRef.current = false;
    }
  }, []);

  // Auto-run the current example on mount + whenever the example or surface changes (re-seed the editor first).
  useEffect(() => {
    const seeded = example.source[surface] ?? example.source.ml;
    setSource(seeded);
    void runModel(seeded, surface);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [exampleSlug, surface]);

  const onRun = () => void runModel(source, surface);
  const onPickExample = (slug: string) => { setExampleSlug(slug); writeExampleParam(slug); };

  return (
    <article className="mx-auto flex max-w-5xl flex-col gap-4 px-4 py-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-2xl font-bold">Music theory, live</h1>
        <div className="flex items-center gap-2">
          <SyntaxToggle />
          <Link to="/" className="text-sm text-cadenza-400 hover:underline">← guide</Link>
        </div>
      </div>
      <p className="text-sm text-slate-400">
        Build a chord progression from intervals measured as exact fractions of an octave, schedule it into
        timed MIDI events, and check that every note that switches on switches off again, all computed with exact
        arithmetic and run in your browser.
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <label className="text-xs text-slate-500" htmlFor="music-example">Example</label>
        <select
          id="music-example"
          data-testid="music-example-picker"
          className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-sm"
          value={exampleSlug}
          onChange={(e) => onPickExample(e.target.value)}
        >
          {EXAMPLES.map((e) => <option key={e.slug} value={e.slug}>{e.title}</option>)}
        </select>
        <span className="text-xs text-slate-500">{example.description}</span>
      </div>

      <div className="flex flex-col gap-4 md:flex-row">
        <div className="flex flex-col gap-2 md:min-w-0 md:flex-[2]">
          <LazyCodeEditor value={source} onChange={setSource} ide={musicIde} minHeight="8rem" />
          <div className="flex items-center justify-between">
            <span className="font-mono text-xs text-slate-500" data-testid="music-status">
              {status.phase === "error" ? <span className="text-rose-300">{status.message}</span>
                : status.phase === "running" ? "running…"
                : status.phase === "events" ? `${status.rows.length} MIDI events`
                : status.phase === "scalar" ? "a value"
                : "Run to see the result"}
            </span>
            <button
              onClick={onRun}
              disabled={status.phase === "running"}
              className="min-h-11 rounded bg-cadenza-600 px-3 text-xs font-semibold text-white enabled:hover:bg-cadenza-500 disabled:opacity-40 sm:min-h-0 sm:py-1"
            >▶ Run</button>
          </div>
        </div>

        {/* Result pane — the event-stream TABLE + balanced badge, or a scalar value. */}
        <div data-testid="music-result" className="rounded-lg border border-slate-800 bg-slate-950 p-3 md:min-w-[18rem] md:flex-[3]">
          {status.phase === "events" ? (
            <>
              <div
                data-testid="music-balanced-badge"
                data-balanced={status.balanced}
                className={`mb-3 inline-block rounded px-2 py-1 text-xs font-semibold ${status.balanced ? "bg-emerald-900/60 text-emerald-200" : "bg-rose-900/60 text-rose-200"}`}
              >
                {status.balanced ? "balanced: no stuck keys" : "UNBALANCED: a key never releases"}
              </div>
              <div className="max-h-[24rem] overflow-auto">
                <table className="w-full border-collapse font-mono text-xs" data-testid="music-event-table" data-event-count={status.rows.length}>
                  <thead className="text-slate-500">
                    <tr><th className="border-b border-slate-800 py-1 pr-4 text-left">tick</th><th className="border-b border-slate-800 py-1 pr-4 text-left">note</th><th className="border-b border-slate-800 py-1 text-left">on/off</th></tr>
                  </thead>
                  <tbody>
                    {status.rows.map((r, i) => (
                      <tr key={i} className="text-slate-300">
                        <td className="py-0.5 pr-4">{r.tick}</td>
                        <td className="py-0.5 pr-4">{r.note}</td>
                        <td className={`py-0.5 ${r.on ? "text-emerald-400" : "text-slate-500"}`}>{r.on ? "on" : "off"}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          ) : status.phase === "scalar" ? (
            <pre className="overflow-auto whitespace-pre-wrap font-mono text-xs text-slate-300">{status.text}</pre>
          ) : status.phase === "error" ? (
            <div className="text-xs text-rose-300">{status.message}</div>
          ) : (
            <div className="text-sm text-slate-600">{status.phase === "running" ? "running…" : "Run to see the result"}</div>
          )}
        </div>
      </div>
    </article>
  );
}
