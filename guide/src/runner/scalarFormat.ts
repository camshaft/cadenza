/// Format a SCALAR run result using its static result type — NO worker/DOM imports, so `node --test`
/// covers it. The browser run path calls a scalar export and does `String(fn())`, but jco lowers a
/// whole-number Float32/Float64 to a JS integer-valued `number`, so `String(5)` drops the `.0` and a
/// float prints indistinguishably from an Int64 (the reference runner shows `5.0`). The JS value type
/// alone can't disambiguate — sized ints and `Qty.value` are `number` too — only the STATIC result type
/// can, which `cdz-wasm::export_types` now exposes. This applies that type to the stringified value.

/// Whether a rendered type string names a floating-point type (Float32/Float64). We match the type
/// HEAD so a parameterized/annotated form (`Float64`, `(: … Float64)`, `Float32`) still counts, while
/// `Int*`, `UInt*`, `Qty`, `Bool`, etc. do not.
function isFloatType(resultType: string): boolean {
  return /\bFloat(32|64)\b/.test(resultType);
}

/// Format the stringified scalar `value` for display, given its static `resultType` (or null/unknown).
/// A Float-typed value that stringified WITHOUT a decimal point (a whole number, e.g. `5`) gets a
/// forced `.0` so it reads as a float (`5.0`), matching the reference runner. A value that already has
/// a `.`/`e` (e.g. `4.5`, `1e-9`) is left alone, as is any non-float type (Int/UInt/Qty/Bool) — so a
/// sized int `2` or a `Qty.value` `3000` is never wrongly decorated. Unknown type → value unchanged.
export function formatScalarByType(value: string, resultType: string | null | undefined): string {
  if (resultType && isFloatType(resultType) && /^-?\d+$/.test(value.trim())) {
    return `${value}.0`;
  }
  return value;
}

/// The type of the export the run path will invoke, from `export_types`' `name<TAB>type` lines. The
/// guide wraps a snippet as `def main() = <expr>` + `export main`, so `main`'s type is the result type;
/// fall back to the SOLE export's type when there's exactly one and it isn't named `main`. Null when
/// there's no unambiguous single result type (multiple non-main exports, or none).
export function resultTypeOf(exportTypesText: string): string | null {
  const rows = exportTypesText
    .split("\n")
    .map((l) => l.split("\t"))
    .filter((c) => c.length >= 2 && c[0].trim().length > 0)
    .map((c) => [c[0].trim(), c[1].trim()] as const);
  const main = rows.find((r) => r[0] === "main");
  if (main) return main[1];
  if (rows.length === 1) return rows[0][1];
  return null;
}
