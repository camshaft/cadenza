/// Pure test-classification helpers for the in-browser `@test` runner — NO worker/DOM imports, so
/// `node --test` covers them. `client.ts` (which DOES import Worker) composes these; extracting them
/// here makes two correctness-critical partitions testable in isolation:
///   - `classifyParamTests`: split a compiler `param_test_signatures` list into the SCALAR props (arg-
///     driven), COMPOUND props (`-gen` pool-driven), and DEFERRED names (a parameterized `@test` the
///     compiler couldn't synthesize a generator for). A bug here silently DROPS or mis-buckets a property
///     test — it would vanish from the report or run in the wrong driver.
///   - `allTestNames`: the union of nullary + scalar-prop + compound-prop names, used to fan a whole-suite
///     timeout/error out against EVERY test. A bug here means a driven property that timed out silently
///     vanishes from the report (the failure mode the client comment calls out).

/// A `param_test_signature` row as the compiler reports it: `compound:false` = a scalar param test
/// (driven live over generated call-args), `compound:true` = a `-gen` wrapper (driven over a seeded
/// `Test.gen-int` pool with shrinking).
export interface ParamTestSig {
  name: string;
  compound: boolean;
  paramTypes: string[];
}

export interface ScalarProp {
  name: string;
  paramTypes: string[];
}
export interface CompoundProp {
  name: string;
}

export interface ParamTestClassification {
  scalarProps: ScalarProp[];
  compoundProps: CompoundProp[];
  /// Parameterized `@test`s that are NEITHER scalar NOR compound — no synthesized generator for the
  /// parameter shape yet, so the UI shows them pending (deferred) rather than dropping or failing them.
  deferredNames: string[];
}

/// Partition the parameterized `@test`s: scalar (arg-driven) vs compound (`-gen` pool-driven) from the
/// signatures, and DEFERRED = every `paramTestNames` entry the signatures did NOT classify as either
/// (a defensive union — e.g. a parameter shape the compiler couldn't synthesize a generator for). Pure.
export function classifyParamTests(
  sigs: ParamTestSig[],
  paramTestNames: string[],
): ParamTestClassification {
  const scalarProps = sigs.filter((s) => !s.compound).map((s) => ({ name: s.name, paramTypes: s.paramTypes }));
  const compoundProps = sigs.filter((s) => s.compound).map((s) => ({ name: s.name }));
  const driven = new Set([...scalarProps, ...compoundProps].map((p) => p.name));
  const deferredNames = paramTestNames.filter((n) => !driven.has(n));
  return { scalarProps, compoundProps, deferredNames };
}

/// The union of every runnable test name — nullary `@test`s plus scalar and compound property names. A
/// whole-suite timeout/error is fanned out against THIS set so no driven test vanishes from the report.
export function allTestNames(
  testNames: string[],
  scalarProps: { name: string }[],
  compoundProps: { name: string }[],
): string[] {
  return [...testNames, ...scalarProps.map((p) => p.name), ...compoundProps.map((p) => p.name)];
}
