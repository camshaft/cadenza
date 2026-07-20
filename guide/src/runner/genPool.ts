/// Pure property-test GENERATOR core for the in-browser `@test` driver — NO jco/worker imports, so
/// `node --test` covers it. `runWorker.ts` (which imports jco-transpile + uses worker globals) composes
/// these. This is the browser-side TWIN of the native `cdz test` generator/shrinker (rcdzc's
/// `proptest_gen.rs` `build_gen` / `shrink_pool`): the pool→value mapping must match on both sides, so if
/// the compiler ever changes a gen shape (e.g. the int-range fold or the LCG constants), this needs the
/// twin update. Extracting it here lets the mapping be pinned by unit tests instead of only exercised
/// live through the worker. (See memory `jco-resource-drop-oob…` §DURABILITY INVARIANT.)

/// A seeded 64-bit LCG (the same MMIX constants the corpus generators use), stepping a `bigint` state and
/// yielding the low 64 bits. Deterministic from the seed → a failing trial is replayable.
export function lcgStep(state: bigint): bigint {
  const M = 0xffffffffffffffffn;
  return (state * 6364136223846793005n + 1442695040888963407n) & M;
}

/// The inclusive value range for each scalar `paramType` enum (from `param_test_signatures`). Signed widths
/// are [-2^(n-1), 2^(n-1)-1]; unsigned are [0, 2^n-1]. A type not here (`"other"`, a compound, a float,
/// `bool`) returns null — those aren't range-generated scalar ints.
export function intRange(t: string): { min: bigint; max: bigint } | null {
  switch (t) {
    case "int8": return { min: -128n, max: 127n };
    case "int16": return { min: -32768n, max: 32767n };
    case "int32": return { min: -2147483648n, max: 2147483647n };
    case "int64": return { min: -9223372036854775808n, max: 9223372036854775807n };
    case "uint8": return { min: 0n, max: 255n };
    case "uint16": return { min: 0n, max: 65535n };
    case "uint32": return { min: 0n, max: 4294967295n };
    case "uint64": return { min: 0n, max: 18446744073709551615n };
    default: return null;
  }
}

/// Generate one JS argument for a scalar `paramType` from the pool state, returning the arg and the advanced
/// state. jco lowers every Cadenza int width to a JS `bigint`, `Bool` to `boolean`, and a float to `number`
/// — so an int arg is a `bigint` folded into its width range (modulo, always in-range even for a huge draw),
/// a bool is the state's low bit, and a float is an integer-valued `number` (never NaN, matching the
/// compiler's `float-of-int` generator).
export function genArg(type: string, state: bigint): { arg: unknown; state: bigint } {
  const next = lcgStep(state);
  if (type === "bool") return { arg: (next & 1n) === 0n, state: next };
  if (type === "float32" || type === "float64") {
    // An integer-valued float in a modest range (matches `Float64.of-int` — total, never NaN).
    return { arg: Number(next % 2048n) - 1024, state: next };
  }
  const range = intRange(type);
  if (!range) return { arg: 0n, state: next }; // unreachable for a scalar prop (client filters "other")
  const span = range.max - range.min + 1n;
  return { arg: range.min + ((next % span) + span) % span, state: next };
}

/// Build one trial's argument vector for a scalar property test from a base pool state (one arg per param
/// type, threading the LCG state).
export function genArgs(paramTypes: string[], seed: bigint): unknown[] {
  let state = seed;
  const args: unknown[] = [];
  for (const t of paramTypes) {
    const { arg, state: s } = genArg(t, state);
    args.push(arg);
    state = s;
  }
  return args;
}

/// Render a trial's args for a counterexample message (a `bigint` prints without the JS `n` suffix so it
/// reads like a Cadenza literal).
export function renderArgs(name: string, args: unknown[]): string {
  return `${name}(${args.map((a) => (typeof a === "bigint" ? a.toString() : String(a))).join(", ")})`;
}

/// Normalize an identifier for cross-naming-convention matching: a Cadenza source name (`one_plus_one`)
/// crosses the component boundary as a kebab WIT name (`one-plus-one`) that jco then binds in JS as
/// camelCase (`onePlusOne`) — so strip `-`/`_` and lowercase to compare source names to actual exports.
export function normalizeName(n: string): string {
  return n.replace(/[-_]/g, "").toLowerCase();
}

/// A seeded int POOL for the compound `Test.gen-int` driver. Two modes, because the wrapper calls `gen-int`
/// an a-priori-unknown number of times (a `List` gens its length, then each element):
///   - GENERATIVE (initial trial, no `preset`): lazily EXTENDS from the LCG on each draw, capturing the
///     concrete sequence in `values` — every draw deterministic from `seed`, and `values` records exactly
///     what was consumed so the shrinker can replay it.
///   - REPLAY (a `preset` pool, from the shrinker): serves the preset draws, then pads EXHAUSTED draws with
///     `0n` — it does NOT LCG-extend. This is what makes truncation a faithful shrink: a shorter pool means
///     the wrapper genuinely sees fewer/zero draws (→ a shorter collection / zero-valued tail), not a
///     different LCG-seeded tail. (A `0` gen-int typically drives a length/element toward its minimum.)
export class GenPool {
  private state: bigint;
  readonly values: bigint[] = [];
  private i = 0;
  private readonly replay: boolean;
  constructor(seed: bigint, preset?: bigint[]) {
    this.state = seed;
    this.replay = preset !== undefined;
    if (preset) this.values = preset.slice();
  }
  /// The `Test.gen-int` host op: yield the next i64 (jco lowers i64 ↔ JS bigint).
  next = (): bigint => {
    if (this.i >= this.values.length) {
      if (this.replay) return 0n; // exhausted a preset pool → pad with 0 (faithful truncation-shrink)
      this.state = lcgStep(this.state);
      this.values.push(this.state & 0xffffffffffffffffn);
    }
    return this.values[this.i++];
  };
}
