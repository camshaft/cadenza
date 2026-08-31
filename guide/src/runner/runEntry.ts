/// How the RUN worker picks (and describes) a compiled component's runnable entry — extracted PURE (no
/// jco/worker imports) so it is unit-testable under `node --test` against a plain fake `root` object,
/// the way `genPool.ts` / `classifyTests.ts` are node-tested twins of worker logic.
///
/// A compiled program surfaces its entry one of two ways:
///   - COMPOUND result → the resource-escape interface `cadenza:run/run` with `make()` + `encode()`.
///   - SCALAR/unit result → a bare exported function (the entry itself).
/// In BOTH shapes the maker/function mirrors the entry point's ARITY: a nullary `def main() = …` yields a
/// nullary `make()` / `main()`, but a PARAMETERIZED `def main(a: Int64) = …` yields `make(a)` / `main(a)`.
/// Run supplies NO input, so it can only invoke a nullary entry — invoking an arity-N one would lower the
/// missing argument from `undefined` and throw a cryptic "Cannot convert undefined to a BigInt" (this was
/// the operator-reported "any program with an argument fails / result coerced to a BigInt" playground bug:
/// the compound `make(a)` maker was called with no args). `selectRunEntry` detects the parameterized shape
/// up front and returns a `parameterized` plan the worker turns into a helpful message instead.

/// The component's exported FUNCTIONS, each with its name and the JS function (whose `.length` is the
/// declared parameter count). Used to pick a nullary entry to run, or to explain a parameterized one.
export function exportedFunctions(
  root: Record<string, unknown>,
): { name: string; fn: (...a: unknown[]) => unknown }[] {
  return Object.entries(root)
    .filter(([, v]) => typeof v === "function")
    .map(([name, v]) => ({ name, fn: v as (...a: unknown[]) => unknown }));
}

/// The resource-escape interface a compound-returning program exports.
interface RunIface {
  make: (...a: unknown[]) => unknown;
  encode: (h: unknown) => Uint8Array;
}

/// What the worker should do to produce this component's value. `parameterized` carries the entry's arity
/// (and its name when known — the bare-function shape exposes it; the `make()` maker does not) so the
/// worker can render a precise "give it no parameters / call it in the REPL" message.
export type RunPlan =
  | { kind: "compound"; iface: RunIface }
  | { kind: "scalar"; fn: (...a: unknown[]) => unknown }
  | { kind: "parameterized"; name: string | null; arity: number }
  | { kind: "none" };

/// Decide how to invoke `root`'s runnable entry. Prefers the compound resource-escape path; falls back to a
/// nullary bare function; classifies a parameterized entry (compound OR bare) as `parameterized` so the
/// worker never blindly calls an arity-N maker/function with no argument (the BigInt-coercion trap above).
export function selectRunEntry(root: Record<string, unknown>): RunPlan {
  const iface = (root["cadenza:run/run"] ?? root["run"]) as RunIface | undefined;
  if (iface && typeof iface.make === "function") {
    // A parameterized entry compiles to an arity-N `make(...)`; Run has no argument to give it.
    if (iface.make.length > 0) return { kind: "parameterized", name: null, arity: iface.make.length };
    return { kind: "compound", iface };
  }
  const fns = exportedFunctions(root);
  // Prefer a NULLARY entry (the runnable `main` shape) — Run produces a value with no input.
  const nullary = fns.find((f) => f.fn.length === 0);
  if (nullary) return { kind: "scalar", fn: nullary.fn };
  // The only runnable export takes arguments (e.g. `export { inc }` where `inc(x: Int64)`).
  const param = fns[0];
  if (param) return { kind: "parameterized", name: param.name, arity: param.fn.length };
  return { kind: "none" };
}

/// The message Run shows for a program whose entry point takes parameters — Run supplies no input, so it can
/// only invoke a nullary entry. `name` is the entry's export name when known (bare-function shape), or `null`
/// when only its arity is known (the compound `make()` maker doesn't carry the name).
export function parameterizedEntryMessage(name: string | null, arity: number): string {
  const args = arity === 1 ? "an argument" : `${arity} arguments`;
  const holes = Array.from({ length: arity }, () => "…").join(", ");
  if (name) {
    return (
      `\`${name}\` takes ${args}, so Run can't produce a value on its own. ` +
      `Call it in the REPL (e.g. \`${name}(${holes})\`), or add \`def main() = ${name}(…)\` and export \`main\`.`
    );
  }
  return (
    `The program's entry point takes ${args}, so Run can't produce a value on its own. ` +
    `Call it from the REPL (e.g. \`main(${holes})\` with your values), or give the entry point no parameters so Run can invoke it.`
  );
}
