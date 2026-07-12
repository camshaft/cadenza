// Browser stub for `oxc-minify` (a native Node addon). jco-transpile imports `minify` at module
// top-level but only CALLS it under `opts.minify`, which the guide never sets — so this stub only
// needs to resolve the import, never actually run. If it is ever called, that's a misconfiguration.
export function minify(): never {
  throw new Error(
    "oxc-minify is stubbed in the browser build — transpile without `minify` (the guide never minifies user output).",
  );
}
