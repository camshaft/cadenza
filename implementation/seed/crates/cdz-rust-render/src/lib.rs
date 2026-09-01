//! Rust-backend boundary-value RENDER + descriptor-note PARSE logic, shared by the corpus gate
//! (`cargo xtask gate --target rust`) and the `cdz run-rust` subcommand.
//!
//! Both compile a Cadenza program to `--target rust`, run it, and must render the boundary value to the
//! SAME canonical s-expr the wasm oracle (`cdz-run`) prints — so wasm-vs-rust outcomes compare
//! like-for-like. This module is the PURE half: given the emitted module TEXT (whose `// cdz-*` comment
//! NOTES the Rust backend writes describe the boundary types) it parses those descriptors and builds the
//! Rust render/reconstruction expression as a source STRING. No I/O, no process spawning — the caller
//! (the gate, or `cdz run-rust`) owns the emit→rustc→run harness. Extracted verbatim from `xtask` so the
//! gate and the subcommand can't drift; `cdz-rust-render` is the single source of truth for the render.
//!
//! Dep-free: the produced strings reference `cdz_num::Big` / `cdz-rt` types, but this crate only PRODUCES
//! that text — it does not link them.

/// Make a Cadenza name a Rust identifier the SAME way the Rust backend does (`sanitize_ident`): a `-`
/// (and any non-ident char) becomes `_`. Kept in lockstep so the driver's call names match the emitted
/// `pub fn` names. (A small copy rather than a dependency on the compiler crate, per the xtask/tools
/// process boundary — the tools are separate binaries.)
pub fn rust_ident(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()) {
            s.push(c);
        } else if c.is_ascii_digit() {
            s.push('_');
            s.push(c);
        } else {
            s.push('_');
        }
    }
    if s.is_empty() {
        s.push('_');
    }
    s
}

/// Translate a corpus CALL ARGUMENT (a canonical sexp VALUE) into the Rust expression that reconstructs
/// it, so a compound argument crosses into the emitted `pub fn` the way the Rust backend represents it.
///
/// The gate passes each arg as its canonical value text. A bare SCALAR (`20`, `-1`, `true`) is already a
/// valid Rust literal whose type the fn signature fixes, so it passes through verbatim. A COMPOUND value
/// must be rebuilt to match the backend's representation (mirroring `cdz_render_at`, the result side):
///  - `(tuple v0 v1 …)` → the Rust tuple `(e0, e1, …)`; a ONE-element tuple gets the trailing-comma form
///    `(e0,)` (Rust would otherwise read `(e0)` as a parenthesized scalar, not a 1-tuple).
///  - `(record (name val) …)` → a Rust tuple of the field values in SORTED-KEY order — the same canonical
///    order the backend lowers a record to (`(Record (x _) (y _))` → `(i64, i64)` with `x` first).
///
/// Anything else (a `list`/sum/`Some`/`Ok` arg, or a value shape not yet needed) passes through verbatim —
/// no regression: those constructs decline at the BACKEND today (list/sum results have no native Rust form),
/// so no trial reaches here relying on them, and a genuinely unhandled shape fails rustc exactly as before.
/// A `cdz_num::Big` constructor expression for an `i128` BigInt entry-arg value — mirrors the backend's
/// `const_big_expr`: in-i64 range → `Big::from_i64`, else `from_sign_magnitude_bytes(&[sign, LE-bytes…])`
/// (the runtime's canonical sign-magnitude leaf, little-endian). Keeps the driver's BigInt arg byte-identical
/// to the value the library body constructs, so the two compare equal.
pub fn big_arg_expr(n: i128) -> String {
    if let Ok(n64) = i64::try_from(n) {
        return format!("cdz_num::Big::from_i64({n64})");
    }
    let sign = if n < 0 { 1u8 } else { 0u8 };
    let mag = n.unsigned_abs();
    // Little-endian magnitude bytes, trimmed of trailing zeros (a canonical minimal form).
    let mut bytes = vec![sign];
    let le = mag.to_le_bytes();
    let last = le.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);
    bytes.extend_from_slice(&le[..last]);
    let elems = bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("cdz_num::Big::from_sign_magnitude_bytes(&[{elems}])")
}

pub fn rust_call_arg(val: &str) -> String {
    let v = val.trim();
    // A FLOAT SPECIAL-VALUE literal (`nan`/`inf`/`-inf`) is not a Rust value token — map it to the `f64`
    // associated constant so it crosses as the right float. (The corpus writes these as bare words; a
    // finite float literal like `1.5` is already valid Rust, so it passes through below.) Only `f64` forms
    // appear as args today; a `Float32` NaN arg would need `f32::NAN` + a cast, but none occur.
    match v {
        "nan" | "NaN" => return "f64::NAN".to_string(),
        "inf" | "+inf" => return "f64::INFINITY".to_string(),
        "-inf" => return "f64::NEG_INFINITY".to_string(),
        _ => {}
    }
    // A SYMBOL literal (`#"read"`) — the canonical value form of a `Symbol` arg. A `Symbol` param emits as
    // an owned Rust `String` in the rust backend (a symbol erases to its interned text — the export is
    // `s: String`), so marshal it exactly like a String ENTRY arg: strip the `#` sigil and cross the quoted
    // text as `"read".to_string()`. Without this the `#"read"` fell through to the String arm's `starts_with
    // ('"')` check (it starts with `#`, not `"`) and was emitted VERBATIM into the driver's Rust source →
    // `error: expected one of ! or [, found "read"` (a no-build, not a decline — breaker/corpus-bugfix, the
    // Symbol twin of the FIXED String/BigInt entry-arg marshals). Checked BEFORE the String arm since the
    // inner `"read"` is itself a valid String literal we reuse.
    if let Some(inner) = v.strip_prefix('#')
        && inner.starts_with('"')
        && inner.ends_with('"')
        && inner.len() >= 2
    {
        return format!("{inner}.to_string()");
    }
    // A STRING literal (`"abc"`) — the canonical value form of a `String` arg — must cross as an OWNED
    // `String`, because the emitted export's parameter is `s: String` (owned), NOT `&str`. A bare `"abc"`
    // Rust literal is a `&'static str`, so passing it directly is a type error (E0308) — the exported-entry
    // String-arg surface (breaker-found, corpus-bugfix; no corpus case passed a String ENTRY arg before, so
    // it was untested). Wrap it `"abc".to_string()`. The quoted form is unambiguously a String value (a
    // Cadenza string renders `"..."`); the inner escapes are already Rust-valid (cdz-run's string form and
    // Rust's share `\n`/`\t`/`\"`/`\\`), so the literal is emitted verbatim inside the `.to_string()`.
    if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
        return format!("{v}.to_string()");
    }
    // A NON-SCALAR entry arg must cross via the SAME construction form the emitted LIBRARY body uses for
    // that type — the corpus writes the RAW Cadenza literal/expr text (`100N`, `1R`, `((. Bytes of) …)`),
    // none of which is valid Rust, so the rust gate DRIVER's arg-emit must lower it (breaker-found CLUSTER;
    // the exported-entry non-scalar-arg surface was wholly untested — every String/BigInt/… input case built
    // the value INSIDE the program). Each form below mirrors `cdz_num`/the Vec builder the backend emits.
    //
    // A BIGINT literal is `<digits>N` (the `N` suffix). Cross as `cdz_num::Big` — `from_i64` for an in-i64
    // magnitude, else `from_sign_magnitude_bytes(&[sign, LE-bytes…])` (the same two-way split `const_big_expr`
    // uses). The digits are parsed to an `i128` first (covers well beyond i64); a magnitude past i128 would
    // need the byte form from the raw digits — no corpus arg reaches that, so parse-to-i128 suffices.
    if let Some(digits) = v.strip_suffix('N')
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        && let Ok(n) = digits.parse::<i128>()
    {
        return big_arg_expr(n);
    }
    // A RATIONAL literal is `<int>R` (an integer rational `n/1`) or `<n>/<d>` (a fraction). Cross as
    // `cdz_num::Rational::new(Big::from_i64(n), Big::from_i64(d))` — the same form the body emits for
    // `Rational.of`. `Rational::new` normalizes, so any equivalent spelling is fine.
    if let Some(int) = v.strip_suffix('R')
        && !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        && let Ok(n) = int.parse::<i64>()
    {
        return format!(
            "cdz_num::Rational::new(cdz_num::Big::from_i64({n}), cdz_num::Big::from_i64(1))"
        );
    }
    if let Some((ns, ds)) = v.split_once('/')
        && let (Ok(n), Ok(d)) = (ns.trim().parse::<i64>(), ds.trim().parse::<i64>())
    {
        return format!(
            "cdz_num::Rational::new(cdz_num::Big::from_i64({n}), cdz_num::Big::from_i64({d}))"
        );
    }
    // A NATIVE M2 compound literal `#tuple(…)` / `#list(…)` / `#record(…)` (the M3-nativized corpus arg
    // form: the `#head` is FUSED to its paren group with no separating space, unlike the legacy
    // `(tuple …)` / `(list …)` / `(record …)` form). Normalize `#head(inner)` → `(head inner)` and reuse the
    // arms below — the inner text is byte-identical, so a NESTED native element (`#tuple(100 #tuple(10 3))`)
    // is handled by the same recursive `rust_call_arg`. Without this the `#`-led form fell through to the
    // pass-through-verbatim arm and leaked `#tuple(…)` into the driver's Rust source → `error: expected one
    // of ! or [, found tuple` (rustc reading `#` as an attribute start) — an M3-native-compound arg no-build
    // (v-gha-green nightly rust-gate-full, all 24 shards; the compound twin of the FIXED `#"sym"` Symbol-arg
    // marshal above). Restricted to the heads the arms below rebuild (`tuple`/`list`/`record`); a `#set(…)` /
    // `#map(…)` arg has no rust-backend construction form and falls through to verbatim (declines, as before).
    if let Some(after_hash) = v.strip_prefix('#')
        && let Some(lp) = after_hash.find('(')
        && after_hash.ends_with(')')
    {
        let head = &after_hash[..lp];
        if matches!(head, "tuple" | "list" | "record") {
            let inner = &after_hash[lp + 1..after_hash.len() - 1];
            return rust_call_arg(&format!("({head} {inner})"));
        }
    }
    // A compound is a parenthesized head form; a bare token is a scalar literal → verbatim.
    let inner = match v.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        Some(inner) => inner.trim(),
        None => return v.to_string(),
    };
    // A BYTES value crosses as `((. Bytes of) (list <byte>…))` — the dotted `Bytes.of` member-access head.
    // Emit the same `Vec<u8>` the library body builds: `vec![<b>u8, …]`. Detect the dotted head before the
    // whitespace split (which would mis-tokenize the `(. Bytes of)` sub-form). ONLY marshal when the inner
    // `(list …)` shape is genuinely present — if it is absent (a malformed `((. Bytes of) …)`), FALL THROUGH
    // to pass-through-verbatim below so rustc/the backend REJECTS it, rather than silently defaulting to an
    // empty `vec![]` (which would compile a program with the WRONG argument value — a silent harness
    // miscompile, Copilot PR#507). `and_then` (not `map(...).unwrap_or_default()`) keeps the `None` path
    // falling through instead of collapsing to empty.
    if let Some(after) = inner.strip_prefix("(. Bytes of)")
        && let Some(elems) = after
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .and_then(|s| s.trim().strip_prefix("list"))
            .map(|s| {
                split_top_level(s.trim())
                    .iter()
                    .map(|b| format!("{}u8", b.trim()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
    {
        return format!("vec![{elems}]");
    }
    let (head, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    match head {
        // A LIST value `(list e0 e1 …)` → Rust's `vec![<e0>, <e1>, …]` (the `Vec<T>` the library uses). Each
        // element is marshalled recursively (a list of tuples/sums composes). An empty `(list)` → `vec![]`.
        "list" => {
            let elems: Vec<String> = split_top_level(rest)
                .iter()
                .map(|e| rust_call_arg(e))
                .collect();
            format!("vec![{}]", elems.join(", "))
        }
        // A built-in Option/Result variant → Rust's own `Option`/`Result` (what the library emits). A
        // payload is marshalled recursively; `None` is nullary. Matches the backend's `SumNew` rendering.
        "Some" => format!("Some({})", rust_call_arg(rest)),
        "None" => "None".to_string(),
        "Ok" => format!("Ok({})", rust_call_arg(rest)),
        "Err" => format!("Err({})", rust_call_arg(rest)),
        "tuple" => {
            let elems: Vec<String> = split_top_level(rest)
                .iter()
                .map(|e| rust_call_arg(e))
                .collect();
            if elems.len() == 1 {
                format!("({},)", elems[0]) // 1-tuple: trailing comma so it isn't a paren-scalar.
            } else {
                format!("({})", elems.join(", "))
            }
        }
        "record" => {
            // A record value crosses in the SAME positional Rust tuple the backend emits (fields in canonical
            // SORTED-key order). The corpus writes a record arg in one of TWO surface forms:
            //  - NAMED-field form `(record (= a 3) (= b 4))` — each element is the canonical `(= name value)`
            //    ascription triple (DESIGN-record-type-syntax Phase B); sort by NAME so the positional tuple
            //    matches the backend's sorted-key field order. (A legacy `(name value)` pair is tolerated.)
            //  - POSITIONAL value form `(record 3 4)` — bare values ALREADY in field order (the `record` head
            //    is dropped by cdz-run's tuple-literal parser; several DIRECT-CALL record-arg cases use this).
            // Disambiguate by whether EVERY element is parenthesized: a named-field element is `(= a 3)`, a
            // positional scalar element is a bare `3`. (A positional element that is itself a compound would
            // also be parenthesized; the record-arg corpus is scalar-field, so this split is exact there.)
            let raw = split_top_level(rest);
            let all_pairs = !raw.is_empty() && raw.iter().all(|f| f.trim().starts_with('('));
            let elems: Vec<String> = if all_pairs {
                let mut fields: Vec<(String, String)> = raw
                    .iter()
                    .filter_map(|f| {
                        let f = f.trim();
                        let body = f.strip_prefix('(')?.strip_suffix(')')?.trim();
                        // Canonical `= name value` triple → strip the `=` head; else a legacy `name value`.
                        let body = body.strip_prefix("= ").map(str::trim_start).unwrap_or(body);
                        let (name, fval) = body.split_once(char::is_whitespace)?;
                        Some((name.trim().to_string(), rust_call_arg(fval)))
                    })
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                fields.into_iter().map(|(_, v)| v).collect()
            } else {
                // Positional value form — the bare values are already in field order.
                raw.iter().map(|e| rust_call_arg(e)).collect()
            };
            if elems.len() == 1 {
                format!("({},)", elems[0])
            } else {
                format!("({})", elems.join(", "))
            }
        }
        // Not a compound the harness rebuilds — pass through verbatim (declines at the backend if unsupported).
        _ => v.to_string(),
    }
}

/// Make a Cadenza SUM / VARIANT name a Rust identifier the SAME way the Rust backend's `types::sum_ident`
/// does — kept in lockstep so a sum VALUE that escapes renders through the enum the backend actually
/// emitted. A clean ident (valid Rust ident chars, not the mangle marker) passes through, except a Rust
/// PRIMITIVE type name (`i64`, `bool`, …) which is prefixed `cdz_ty_`; any lossy / leading-digit /
/// marker-prefixed name is hex-mangled `cdzsum_<hex-utf8>` (injective, so two distinct sum names never
/// collide into one emitted enum). Mirrors `types::sum_ident` + `sanitize_ident`'s keyword handling.
pub fn sum_rust_ident(name: &str) -> String {
    const MARKER: &str = "cdzsum_";
    let is_clean = {
        let mut chars = name.chars();
        matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
            && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
    };
    if is_clean && !name.starts_with(MARKER) {
        // A clean ident may be a Rust keyword (→ `r#kw`, or `cdz_kw_…` for the raw-ident exceptions) or a
        // primitive type name (→ `cdz_ty_…`); mirror the backend's escapes.
        let s = rust_ident(name);
        if matches!(s.as_str(), "crate" | "self" | "Self" | "super" | "_") {
            format!("cdz_kw_{s}")
        } else if is_rust_keyword_driver(&s) {
            format!("r#{s}")
        } else if is_rust_primitive_type_driver(&s) {
            format!("cdz_ty_{s}")
        } else {
            s
        }
    } else {
        let mut hex = String::with_capacity(name.len() * 2 + MARKER.len());
        hex.push_str(MARKER);
        for b in name.bytes() {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }
}

/// Rust reserved words — the driver's copy of the backend's `is_rust_keyword`, for `sum_rust_ident`.
pub fn is_rust_keyword_driver(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "gen"
    )
}

/// Rust primitive type names — the driver's copy of the backend's `is_rust_primitive_type`.
pub fn is_rust_primitive_type_driver(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

/// The export `name`'s CADENZA result type, read off the `// cdz-return[<ident>]: <type>` note the
/// backend emits before each fn (the type's `render_name`, e.g. `Int64`, `(Tuple Int64 Bool)`, `(Record
/// (a Int64) (b Int64))`). `None` if no matching note is found. The gate renders the result to cdz-run's
/// text form from THIS (the Cadenza type keeps field names + the Tuple/Record distinction the Rust type
/// erases). `name` is the export's SANITIZED ident, matching the note's `[<ident>]` tag.
pub fn cdz_return_type(module: &str, name: &str) -> Option<String> {
    let prefix = format!("// cdz-return[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// The export `name`'s per-closure-param CADENZA arrow SHAPES, read off the `// cdz-param-shapes[<ident>]:
/// <arrow> | <arrow> | …` note the backend emits for a consumer (an export with ≥1 fn-typed param). Each
/// entry is a closure param's `render_name` arrow type IN PARAMETER ORDER (`(-> (Tuple Int64 Int64) Int64)`
/// vs `(-> (Record (a Int64) (b Int64)) Int64)`) — the pre-erasure shape the Rust `Rc<dyn Fn((i64,i64))>`
/// loses. Used to disambiguate producer↔consumer pairing when a Tuple-arg and a Record-arg closure erase to
/// the same type. Empty `Vec` when no note (a non-consumer, or a consumer with no ambiguity-relevant note).
/// Split on ` | ` — a separator that cannot occur inside a `render_name` (which uses `(-> …)`/spaces).
pub fn cdz_param_shapes(module: &str, name: &str) -> Vec<String> {
    let prefix = format!("// cdz-param-shapes[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            return rest
                .trim()
                .split(" | ")
                .map(|s| s.trim().to_string())
                .collect();
        }
    }
    Vec::new()
}

/// The `// cdz-produces-closure[<name>]: <arrow>` note for a PEELED producer — the Cadenza arrow it supplies
/// (`(-> <arg-shapes> <result>)` via `render_name`). A peeled producer's `cdz-return` is its SCALAR result,
/// not the closure shape, so this carries the pre-erasure arrow for producer↔consumer shape disambiguation
/// (the async FACTORY producer instead carries the arrow in its own `cdz-return`). `None` when absent.
pub fn cdz_produces_closure(module: &str, name: &str) -> Option<String> {
    let prefix = format!("// cdz-produces-closure[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// The `// cdz-unit[<name>]: <value-form>` note for a QUANTITY export — the unit's canonical VALUE-form
/// s-expr (`Unit::render_value_form`, byte-identical to what cdz-run prints inside `((. Qty of) …)`). Only
/// a Qty-returning fn emits one; `None` for any other result. Spliced verbatim by the top-level Qty render.
pub fn cdz_unit_form(module: &str, name: &str) -> Option<String> {
    let prefix = format!("// cdz-unit[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// The native Rust scalar type for a Cadenza scalar type-NAME (`render_name` form) — `Int64`→`i64`,
/// `UInt32`→`u32`, `Float64`→`f64`, `Float32`→`f32`, etc. Used by the Qty display-scale render to spell the
/// inner width the scale multiply runs in. Falls back to `i64` for an unrecognized name (only the aliased
/// integer/float widths reach a scaled Qty magnitude — the backend declines a non-aliased inner upstream).
pub fn rust_scalar_type_name(ty: &str) -> &'static str {
    match ty.trim() {
        "Int8" => "i8",
        "Int16" => "i16",
        "Int32" => "i32",
        "Int64" => "i64",
        "UInt8" => "u8",
        "UInt16" => "u16",
        "UInt32" => "u32",
        "UInt64" => "u64",
        "Float32" => "f32",
        "Float64" => "f64",
        _ => "i64",
    }
}

/// The `// cdz-scale[<name>]: <num>/<den>` note for a NON-scale-1 QUANTITY export — the unit's scale to its
/// dimension's reference, which the harness applies to the boundary magnitude so `5 kilometer` displays as
/// `5000 meter`. Only a Qty result at a non-reference unit emits one; `None` for a scale-1 (or non-Qty)
/// result (the magnitude is displayed as stored). Returns the `(num, den)` machine-integer ratio.
pub fn cdz_scale(module: &str, name: &str) -> Option<(i128, i128)> {
    let prefix = format!("// cdz-scale[{name}]:");
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            let (n, d) = rest.trim().split_once('/')?;
            return Some((n.trim().parse().ok()?, d.trim().parse().ok()?));
        }
    }
    None
}

/// The PER-ELEMENT quantity display-scale notes (`// cdz-qty-at[<name>]: <path> <num>/<den>`) for a COMPOUND
/// result carrying non-scale-1 Qty leaves — one per leaf, keyed by the render's LOGICAL descent PATH: a
/// tuple/record field is a positional `.i` segment (`0`, `1`, `0.1`), an Option/Result payload a `?N`
/// segment (`?0` payload, `?0`/`?1` Ok/Err; nested composes, e.g. `0?0`), and a USER-DEFINED sum variant
/// payload a LOCAL `<variant>?<idx>` segment. Returns a map `path → (num, den)`; the Qty arm looks up the
/// CURRENT descent path and applies that leaf's scale (the compound twin of the top-level `cdz_scale`), so a
/// tuple/record/sum of quantities at different units scales each element independently. Empty for a result
/// with no compound non-scale-1 Qty leaf.
pub fn cdz_qty_at(module: &str, name: &str) -> std::collections::HashMap<String, (i128, i128)> {
    let prefix = format!("// cdz-qty-at[{name}]:");
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&prefix) {
            // `<path> <num>/<den>`
            let rest = rest.trim();
            if let Some((path, scale)) = rest.split_once(char::is_whitespace)
                && let Some((n, d)) = scale.trim().split_once('/')
                && let (Ok(num), Ok(den)) = (n.trim().parse(), d.trim().parse())
            {
                map.insert(path.trim().to_string(), (num, den));
            }
        }
    }
    map
}

/// Whether `ty` (a `render_name`-form type) is a scalar the flag-gated VALUE-DOC path handles today. The
/// value-doc path (`CDZ_VALUE_DOC`) emits the result as a self-describing `(: value type)` codec doc rendered
/// by the ONE canonical printer (`render_binary`) — the operator-directed parser-elimination / op-seq-283
/// convergence. Restricted to a bare `Int64` for now (its runtime value is already an `i64`, so
/// `IntValue::from_i64` is exact); other widths (unsigned needs `from_u64`) and COMPOUND shapes (a Ty-guided
/// walk in rcdzc) are follow-ups. Default path keeps `cdz_render_expr` (byte-identical) when this is false.
pub fn is_value_doc_scalar(ty: &str) -> bool {
    ty.trim() == "Int64"
}

/// A Rust EXPRESSION producing the `"CDZDOC:<hex>"` marker string for a scalar `Int64` result binding
/// `val_expr` (`__r`): build the `(: <value> Int64)` codec doc via `cadenza_ast::Builder` + `codec::encode`
/// (the SAME wire cdz-run's `value_codec` emits, decoded by the harness's `render_binary`), hex-encode it, and
/// prefix `CDZDOC:` (the marker the harness's [`crate` consumer]/`value_doc::interpret_run_stdout` detects).
/// Built by explicit concatenation (NOT `format!`) so the generated code's own `{`/`}` need no brace-escaping;
/// only `val_expr` + `type_name` are interpolated. The emitted program links `cadenza_ast` (the rust exec
/// layer `--extern`s it — run.rs `RlibDirs.cadenza_ast`), so `cadenza_ast::…` resolves at compile time.
pub fn value_doc_render_scalar(val_expr: &str, type_name: &str) -> String {
    let mut s = String::new();
    s.push_str("{ let mut __vb = cadenza_ast::ast::Builder::new(); ");
    s.push_str("let __vc = __vb.name(\":\"); ");
    s.push_str(
        "let __vv = __vb.atom_leaf(cadenza_ast::ast::Leaf::Int { \
         value: cadenza_ast::ast::IntValue::from_i64((",
    );
    s.push_str(val_expr);
    s.push_str(") as i64), radix: cadenza_ast::ast::Radix::Dec }); ");
    s.push_str("let __vt = __vb.name(\"");
    s.push_str(type_name);
    s.push_str("\"); ");
    s.push_str("let __vr = __vb.list(vec![__vc, __vv, __vt]); ");
    s.push_str("let __vbytes = cadenza_ast::codec::encode(&__vb.finish(__vr)); ");
    s.push_str("const __VHEX: &[u8] = b\"0123456789abcdef\"; ");
    s.push_str("let mut __vs = String::from(\"CDZDOC:\"); ");
    s.push_str(
        "for __vbyte in &__vbytes { __vs.push(__VHEX[(__vbyte >> 4) as usize] as char); \
         __vs.push(__VHEX[(__vbyte & 15) as usize] as char); } ",
    );
    s.push_str("__vs }");
    s
}

/// A Rust EXPRESSION that renders the driver's result binding `__r` (whose CADENZA type is `ty`, in
/// `render_name` form) to cdz-run's canonical text form — the value the gate grades against. Type-
/// directed and recursive over the Cadenza type:
///  - `Unit` → the token `unit`;
///  - `(Tuple T0 T1 …)` → `(tuple <r.0> <r.1> …)`;
///  - `(Record (a T0) (b T1) …)` → `(record (a <r.0>) (b <r.1>) …)` — the fields are already in sorted
///    order (both the type's render and the emitted Rust tuple order them the same), so element `i`
///    reads `.i`;
///  - any scalar (`Int64`, `Bool`, …) → `{}` (an integer/bool `Display`s exactly as cdz-run prints it).
#[allow(clippy::too_many_arguments)]
pub fn cdz_render_expr(
    ty: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
    newtypes: &std::collections::HashMap<String, String>,
    sum_params: &std::collections::HashMap<String, usize>,
    unit_form: Option<&str>,
    unit_scale: Option<(i128, i128)>,
    // PER-ELEMENT Qty display-scale map (`cdz_qty_at`): logical-path → `(num, den)`. Consulted by the Qty arm
    // for a Qty NESTED in a Tuple/Record (its scale is not the top-level `unit_scale`). Empty for a result
    // with no compound non-scale-1 Qty leaf; the top-level bare Qty keeps using `unit_scale`.
    qty_at: &std::collections::HashMap<String, (i128, i128)>,
    // The set of sum idents whose variant heads render QUALIFIED (see [`cdz_render_at`] / the
    // `// cdz-sum-qualified-heads[…]` notes). Parsed from the module by [`cdz_sum_qualified_heads`].
    qualified_heads: &std::collections::HashSet<String>,
) -> String {
    let mut helpers = Vec::new();
    let mut on_path = Vec::new();
    let expr = cdz_render_at(
        ty,
        "__r",
        sums,
        newtypes,
        sum_params,
        unit_form,
        unit_scale,
        "",
        qty_at,
        qualified_heads,
        &mut helpers,
        &mut on_path,
    );
    // The recursive-sum render helpers (if any) are hoisted ahead of the expression, then the expression
    // is a block that defines them and evaluates. Each helper is a `fn`, so mutual/self recursion works.
    if helpers.is_empty() {
        expr
    } else {
        format!("{{ {} {expr} }}", helpers.join(" "))
    }
}

/// The recursive worker for [`cdz_render_expr`]: `path` is the Rust access path to the value being
/// rendered (starts at `__r`, descends `.0`/`.1`… into tuple/record elements — a record IS a positional
/// tuple in sorted-field order, so its `i`th field is `.i`).
///
/// `helpers` collects generated recursive render `fn`s (for a RECURSIVE user sum, whose TYPE unfolds
/// infinitely — `IntList = Cons(Tuple Int64 IntList) | Nil` — so it CANNOT be inlined without the codegen
/// itself never terminating). `on_path` is the set of user-sum idents currently being unfolded on the
/// recursion path: re-entering one means a cycle, so emit a CALL to its (runtime-recursive) helper fn
/// instead of inlining, moving the recursion from gate-codegen-time (over the infinite type) to Rust
/// runtime (over the FINITE value — a `Nil` leaf terminates it). Mirrors the wasm value-encode, which walks
/// the value, not the type.
/// The render PATH for a Set element / Map key: a Float element/key is stored in the total-order wrapper
/// `__CdzF32`/`__CdzF64` (a bare `f32`/`f64` is not `Ord`), and the collection `.iter()` binds it BY
/// REFERENCE (`&__CdzF{N}`). The float render (`(path).clone() as f64`) needs the RAW float, and
/// `(&__CdzF32) as f64` is a non-primitive cast (rustc E0605) — so unwrap the wrapper via its `.get()`
/// (`__CdzF{N}: Copy`, so `(*bind).get()` moves a copy out of the `&`-binding, yielding the `f32`/`f64`).
/// A non-float key/element is its own render path (byte-identical). Scalar Float only — a nested float leaf
/// in a TUPLE key/element keeps the per-leaf wrapper and is a separate (rarer) render gap.
fn ord_unwrap_render_path(ty: &str, bind: &str) -> String {
    if ty == "Float32" || ty == "Float64" {
        format!("(*{bind}).get()")
    } else {
        bind.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cdz_render_at(
    ty: &str,
    path: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
    newtypes: &std::collections::HashMap<String, String>,
    sum_params: &std::collections::HashMap<String, usize>,
    // The QUANTITY unit VALUE-form (`Unit::render_value_form`), present only when THIS `path` is a Qty and
    // its unit note reached here. Threaded ONLY to the TOP-LEVEL result (`__r`): the corpus has no Qty
    // nested inside a compound result, so a descent into a tuple/record/sum element passes `None`, and a
    // Qty reached with no form falls back to converting the type-string unit. Consumed by the Qty arm.
    unit_form: Option<&str>,
    // The NON-scale-1 quantity's `num/den` scale — present only for the TOP-LEVEL Qty result at a
    // non-reference unit; the Qty arm multiplies the boundary magnitude by it (Float `× num/den`, Int
    // `× num / den`). `None` for a scale-1 Qty (display = stored) and every non-top-level descent.
    unit_scale: Option<(i128, i128)>,
    // The LOGICAL descent path (`""` top-level; a tuple/record field is `.i`; an Option/Result payload is
    // `?N`; a user-sum variant payload is the LOCAL `<variant>?<idx>`), keying `qty_at` for a nested Qty leaf.
    // Distinct from the Rust access `path` (`(__r).0`): the logical path is the corpus/type descent, stable
    // across the Rust binding shape. EXTENDED by the Tuple/Record/Option/Result/user-sum arms; forwarded
    // unchanged by a newtype (transparent) and a List (its per-iteration binder is a per-element-scale
    // follow-up, so a Qty inside a list still renders raw).
    logical_path: &str,
    // PER-ELEMENT Qty display-scale map — logical-path → `(num, den)`; the Qty arm looks up `logical_path`.
    qty_at: &std::collections::HashMap<String, (i128, i128)>,
    // The set of sum idents whose variant heads render QUALIFIED (`((. Ast Str) …)`) rather than bare — the
    // backend's per-sum `sum_needs_qualified_heads` decision, parsed from the `// cdz-sum-qualified-heads[…]`
    // notes. The user-sum arm's `disp_head` consults it (replacing the old `ty == "Ast"` name hack).
    qualified_heads: &std::collections::HashSet<String>,
    helpers: &mut Vec<String>,
    on_path: &mut Vec<String>,
) -> String {
    let ty = ty.trim();
    if ty == "Unit" {
        return "\"unit\".to_string()".to_string();
    }
    // An erased NEWTYPE (`// cdz-newtype[Pt]: (Tuple Int64 Int64)`) — its runtime value IS the inner type
    // (the tag erased, `type-system.md §156`), and `Ty::Nominal`'s render_name is the bare name `Pt`. Render
    // by its INNER type so a `Pt`-typed boundary value renders structurally as `(tuple 5 5)` — NOT falling
    // to the scalar `Display` of the erased Rust tuple `(i64, i64)` (rustc E0277). Checked before the user-
    // sum arm (a newtype has no `cdz-sum` descriptor) and the scalar fallthrough.
    if let Some(inner) = newtypes.get(ty) {
        // Same value/path — a newtype-over-Qty forwards the unit form (the tag is erased, so a `Pt = Mk Qty`
        // renders as its inner quantity).
        return cdz_render_at(
            inner,
            path,
            sums,
            newtypes,
            sum_params,
            unit_form,
            unit_scale,
            logical_path,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
    }
    // `(Tuple T0 T1 …)` → the M2 native ctor `#tuple(…)`. The EMPTY tuple `(Tuple)` (a variant's explicit
    // empty-tuple payload, distinct from `Unit`) renders the literal `#tuple()` — no elements, no `path` read.
    // (M2 native-compound-data flag-day: the canonical value spelling is `#ctor(…)`, matching cdz-run's
    // in-process render + the migrated corpus/wasm `.gate-baseline`; the head has NO trailing space —
    // `#tuple(e0 e1)`, not `#tuple( e0 e1)`.)
    if let Some(elems) = parse_head_type(ty, "Tuple") {
        if elems.is_empty() {
            return "\"#tuple()\".to_string()".to_string();
        }
        let placeholders = vec!["{}"; elems.len()].join(" ");
        let args: Vec<String> = elems
            .iter()
            .enumerate()
            .map(|(i, e)| {
                // Extend the LOGICAL path (`i` at top-level, `{lp}.{i}` nested) so a Qty element looks up its
                // per-element scale in `qty_at`. A non-Qty element ignores it. `unit_form`/`unit_scale` stay
                // `None` for the descent (top-level-only); the Qty arm falls back to `qty_at[logical_path]`.
                let child_lp = if logical_path.is_empty() {
                    i.to_string()
                } else {
                    format!("{logical_path}.{i}")
                };
                cdz_render_at(
                    e,
                    &format!("({path}).{i}"),
                    sums,
                    newtypes,
                    sum_params,
                    None,
                    None,
                    &child_lp,
                    qty_at,
                    qualified_heads,
                    helpers,
                    on_path,
                )
            })
            .collect();
        return format!("format!(\"#tuple({placeholders})\", {})", args.join(", "));
    }
    // A record TYPE `(Record (: a T0) (: b T1) …)` → the VALUE form `(record (a …) (b …) …)`. Each element
    // is a `(: name Type)` ASCRIPTION node (RT3, DESIGN-record-type-syntax — the canonical record-TYPE
    // field is the shared `(: name T)` binder node, matching a param binder / `e: T`), in sorted order
    // (matching the emitted tuple), so field `i` reads `.i`. The head is matched CAPITALIZED —
    // `Ty::render_name` writes a record type `(Record …)`, distinct from the lowercase value constructor
    // `(record …)`; the EMITTED value form stays lowercase `(record …)`, cdz-run's canonical value
    // spelling. (Was matched lowercase `record`, which stopped matching after a1c9bc09 → a record return
    // type fell through to the scalar `Display` path and the emitted `(i64, i64)` tuple failed rustc E0277
    // "doesn't implement Display", failing every record-escape case on the rust gate.)
    if let Some(fields) = parse_head_type(ty, "Record") {
        // Each field renders as `(<name> <value>)` — the name is a literal, the value is `.i` rendered
        // as its own type. The `format!` gets one `({} {})` group per field, args = name, value, ….
        let mut args = Vec::with_capacity(fields.len() * 2);
        for (i, field) in fields.iter().enumerate() {
            // `field` is the ascription `(: name Type)` — strip its OUTER parens (exactly one each;
            // `trim_end_matches(')')` would wrongly eat a nested type's close paren, e.g.
            // `(: y (Tuple Int64 Int64))`), then read the `:` head, the name, and the rest (the type).
            let f = field.trim();
            let inner = f
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(f)
                .trim();
            // Ascription `: name Type`: drop the `:` head, then split name from type. Tolerate the legacy
            // bare `name Type` pair (no leading `:`) too, so a stray head-app form still reads its name.
            let after_colon = inner.strip_prefix(':').map(str::trim).unwrap_or(inner);
            let (fname, fty) = after_colon
                .split_once(char::is_whitespace)
                .unwrap_or((after_colon, ""));
            args.push(format!("\"{}\"", fname.trim()));
            // Extend the logical path by the sorted-field index `i` (== the emitted tuple `.i`), so a Qty
            // field looks up its per-element scale in `qty_at`.
            let child_lp = if logical_path.is_empty() {
                i.to_string()
            } else {
                format!("{logical_path}.{i}")
            };
            args.push(cdz_render_at(
                fty.trim(),
                &format!("({path}).{i}"),
                sums,
                newtypes,
                sum_params,
                None,
                None,
                &child_lp,
                qty_at,
                qualified_heads,
                helpers,
                on_path,
            ));
        }
        // Each field renders as the canonical `(= name value)` ascription triple (DESIGN-record-type-
        // syntax Phase B) under the M2 native `#record(…)` ctor head, matching the wasm runtime's
        // value-output render + the migrated corpus — so the -rust / -rust-async baselines agree with the
        // wasm `.gate-baseline`. `args` still interleaves name + value per field; the group wrapper is `(= _ _)`
        // and the head gained the M2 `#` prefix (`#record((= a 1) (= b 2))`, no trailing space after the head).
        let groups = vec!["(= {} {})"; fields.len()].join(" ");
        return format!("format!(\"#record({groups})\", {})", args.join(", "));
    }
    // A `(List T)` value is the Rust `Vec<T>` the backend emits — render it as cdz-run's canonical
    // `(list e0 e1 …)` (empty → `(list)`), each element rendered as its own type `T`. Emit a Rust block
    // that folds the elements into a `String`: iterate `&<path>` (borrow — the value may be read only
    // here), render each element via a FRESH binder, and join under the `(list …)` wrapper. The element
    // binder `__e{depth}` (keyed on path length) avoids a shadow-capture when the element is itself a
    // list/sum. `.iter()` yields `&T`; the recursive render reads the binder, and default ref binding
    // makes a `&i64`/`&(…)` `Display`/index fine, exactly as the Option/Result payload render relies on.
    let ebind = format!("__e{}", path.len());
    if let Some(args) = parse_head_type(ty, "List") {
        let elem_ty = args.first().map(String::as_str).unwrap_or("");
        // Extend the logical path with `.*` (a list element) so a Qty element looks up its per-element scale
        // in `qty_at` — the scale is uniform across elements, keyed once, applied to each per-iteration bind.
        let elem_lp = if logical_path.is_empty() {
            "*".to_string()
        } else {
            format!("{logical_path}.*")
        };
        let inner = cdz_render_at(
            elem_ty,
            &ebind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &elem_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        // Build the M2 native `#list(<e0> <e1> …)` (empty → `#list()`): seed with "#list(", then each
        // element's render SPACE-SEPARATED — a leading space only BEFORE elements after the first (via
        // `enumerate`), so the head `#list(` has no trailing space and elements are `e0 e1`, not ` e0 e1`.
        return format!(
            "{{ let mut __s = String::from(\"#list(\"); for (__i, {ebind}) in ({path}).iter().enumerate() {{ if __i > 0 {{ __s.push(' '); }} __s.push_str(&({inner})); }} __s.push(')'); __s }}"
        );
    }
    // A `(Set E)` value is the Rust `BTreeSet<E>` the backend emits — render it as cdz-run's canonical
    // M2 native `#set(e0 e1 …)` (empty → `#set()`), each element rendered as its type. A `BTreeSet`
    // iterates in SORTED order, which IS the canonical element-value order the runtime uses.
    if let Some(args) = parse_head_type(ty, "Set") {
        let elem_ty = args.first().map(String::as_str).unwrap_or("");
        // `.*` element segment — same uniform-per-element scale as a list (see the List arm).
        let elem_lp = if logical_path.is_empty() {
            "*".to_string()
        } else {
            format!("{logical_path}.*")
        };
        // A Float Set element is the `__CdzF{N}` Ord wrapper — unwrap it for the float render (E0605 else).
        let elem_path = ord_unwrap_render_path(elem_ty, &ebind);
        let inner = cdz_render_at(
            elem_ty,
            &elem_path,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &elem_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        return format!(
            "{{ let mut __s = String::from(\"#set(\"); for (__i, {ebind}) in ({path}).iter().enumerate() {{ if __i > 0 {{ __s.push(' '); }} __s.push_str(&({inner})); }} __s.push(')'); __s }}"
        );
    }
    // A `(Map K V)` value is the Rust `BTreeMap<K, V>` the backend emits — render it as cdz-run's canonical
    // M2 native `#map((= k0 v0) (= k1 v1) …)` (empty → `#map()`), each entry the `(= <key> <value>)` ascription
    // triple with key and value rendered as their own types. A `BTreeMap` iterates in SORTED KEY order — the
    // canonical key order.
    if let Some(args) = parse_head_type(ty, "Map") {
        let key_ty = args.first().map(String::as_str).unwrap_or("");
        let val_ty = args.get(1).map(String::as_str).unwrap_or("");
        let kbind = format!("__mk{}", path.len());
        let vbind = format!("__mv{}", path.len());
        // Extend the logical path with `!k` (map key) / `!v` (map value) so a Qty in either slot looks up its
        // uniform-per-entry scale in `qty_at` (matching `collect_qty_scale_paths`'s Map arm).
        let key_lp = if logical_path.is_empty() {
            "!k".to_string()
        } else {
            format!("{logical_path}!k")
        };
        let val_lp = if logical_path.is_empty() {
            "!v".to_string()
        } else {
            format!("{logical_path}!v")
        };
        // A Float Map KEY is the `__CdzF{N}` Ord wrapper — unwrap it for the float render (E0605 else).
        let key_path = ord_unwrap_render_path(key_ty, &kbind);
        let kr = cdz_render_at(
            key_ty,
            &key_path,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &key_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        let vr = cdz_render_at(
            val_ty,
            &vbind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &val_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        return format!(
            "{{ let mut __s = String::from(\"#map(\"); for (__i, ({kbind}, {vbind})) in ({path}).iter().enumerate() {{ if __i > 0 {{ __s.push(' '); }} __s.push_str(&format!(\"(= {{}} {{}})\", {kr}, {vr})); }} __s.push(')'); __s }}"
        );
    }
    // A QUANTITY result `(Qty <inner> <unit>)` — the rust backend maps a `Ty::Qty { inner }` at a scale-1
    // unit to its INNER magnitude's type (the wrapper erases), so `{path}` is the magnitude. cdz-run renders
    // it `((. Qty of) <magnitude> <unit-value-form>)`: render the magnitude by its inner type, then splice
    // the unit's VALUE-form s-expr. The canonical value form (`((. Unit base) …)`, a `Unit./` quotient for a
    // derived unit) comes from the backend's `// cdz-unit` note (`unit_form`) — byte-identical to cdz-run.
    // Scale-1 only reaches here, so the stored magnitude IS the displayed one (no scaling in the render).
    if let Some(args) = parse_head_type(ty, "Qty") {
        let inner_ty = args.first().map(String::as_str).unwrap_or("");
        // Prefer the backend's value-form note (handles EVERY unit shape — base, power, product, quotient).
        // Fall back to converting the type-string unit (a Qty reached WITHOUT a note — only the simple
        // base/dimensionless/single-positive-power shapes the type-string convert covers; the corpus has no
        // note-less Qty result, so the fallback is belt-and-suspenders for a nested Qty a future case adds).
        let unit = match unit_form {
            Some(v) => v.to_string(),
            None => match args.get(1).map(String::as_str).unwrap_or("") {
                "Unit.one" => "(. Unit one)".to_string(),
                u if u.starts_with("(Unit.base ") => {
                    format!("((. Unit base) {}", &u["(Unit.base ".len()..])
                }
                u if u.starts_with("(Unit.^ (Unit.base ") => {
                    u.replacen("(Unit.base ", "((. Unit base) ", 1)
                }
                other => other.to_string(),
            },
        };
        let unit_lit = unit.replace('\\', "\\\\").replace('"', "\\\"");
        // The scale for THIS Qty: the top-level `unit_scale` note if present (a bare Qty result), ELSE the
        // per-element `qty_at` entry keyed by the current logical descent path (a Qty NESTED in a tuple/
        // record — the compound scale-fold this slice adds). `None` when neither → a scale-1 Qty renders its
        // stored magnitude unchanged. This is what fixes the rust-red: a `(Tuple (Qty km) (Qty mile))` now
        // scales each element by its own `qty_at[0]`/`qty_at[1]` instead of rendering the raw magnitude.
        let unit_scale = unit_scale.or_else(|| qty_at.get(logical_path).copied());
        // A NON-scale-1 unit DISPLAY-SCALES the stored magnitude to its reference (`5 km` → `5000 m`): the
        // backend crosses the RAW magnitude + a `// cdz-scale` note (`unit_scale`), and the display multiply
        // happens HERE, in the inner numeric type, mirroring the wasm boundary value-encode
        // (`const_value_ast_scaled`): a Float multiplies as f64 (`v * num as f64 / den as f64`, IEEE rounds);
        // an Int multiplies then integer-divides (`v * num / den`, truncates toward zero). Only Float/Int
        // inners reach here with a scale (the backend's `qty_scale_supported` gate declines a Rational/BigInt
        // non-scale-1, which needs exact rational scaling). A scale-1 Qty has `unit_scale == None` → the raw
        // path renders unchanged. The scaled value binds to a fresh local `__q` of the inner type so the
        // recursive render reads a plain scalar (not the arithmetic expression). Wrap the scaled expr in its
        // inner-typed `let` so the `* / ` runs in the right width (an i64 magnitude scales as i64, etc.).
        let scaled_path = match unit_scale {
            Some((num, den)) => {
                let bind = format!("__q{}", path.len());
                let (letbind_ty, scaled_expr) = if inner_ty.trim() == "Rational" {
                    // A RATIONAL magnitude scales EXACTLY (no rounding): multiply by the scale as a Rational
                    // `num/den` — `Rational::mul` normalizes to lowest terms, so `5 mile` = `5/1 · 201168/125`
                    // = `201168/25 meter` exactly. `Big::from_i64` builds the ratio's limbs (a real
                    // prefix/family scale fits i64). The exact twin of the Float(rounds)/Int(truncates) cases.
                    (
                        "cdz_num::Rational".to_string(),
                        format!(
                            "({path}).mul(&cdz_num::Rational::new(cdz_num::Big::from_i64({num}i64), cdz_num::Big::from_i64({den}i64)))"
                        ),
                    )
                } else if inner_ty.trim() == "BigInt" {
                    // A BIGINT magnitude scales in the bignum path: `Big.mul(num) then quotient by den`
                    // (`divmod(…).0`, truncating toward zero like the fixed-Int case). For a WHOLE-ratio scale
                    // (a prefix like `kilo` = `×1000/1`) this is EXACT (`5 km` → `5000 m`, no truncation — the
                    // "exact, no truncation" corpus case); a non-whole ratio truncates, the BigInt twin of the
                    // fixed-Int branch. `Big::from_i64` builds the scale limbs (a real prefix/family scale fits
                    // i64). `divmod` returns an `Option<(Big, Big)>` — a `den` from a unit scale is never 0, so
                    // `.expect` never fires (belt: a 0 would be a malformed unit).
                    (
                        "cdz_num::Big".to_string(),
                        format!(
                            "({path}).mul(&cdz_num::Big::from_i64({num}i64)).divmod(&cdz_num::Big::from_i64({den}i64)).expect(\"unit scale denominator is non-zero\").0"
                        ),
                    )
                } else {
                    // The magnitude's Rust scalar type (`Float64`→`f64`, `Int64`→`i64`, `UInt32`→`u32`, …).
                    let inner_rust = rust_scalar_type_name(inner_ty);
                    // Float multiplies in f64/f32 (num/den cast to the float type, IEEE rounds); an integer
                    // multiplies then truncating-divides in its own width (a `{n}{ty}` literal fixes width).
                    let expr = if inner_rust == "f64" || inner_rust == "f32" {
                        format!("(({path}) * ({num} as {inner_rust}) / ({den} as {inner_rust}))")
                    } else {
                        format!("(({path}) * ({num}{inner_rust}) / ({den}{inner_rust}))")
                    };
                    (inner_rust.to_string(), expr)
                };
                let letbind = format!("let {bind}: {letbind_ty} = {scaled_expr};");
                Some((bind, letbind))
            }
            None => None,
        };
        // The magnitude renders by its inner type; a Qty nested in the magnitude position is impossible (an
        // inner is a numeric scalar), so pass `None` for the descent's unit form + scale.
        let render_path = scaled_path
            .as_ref()
            .map(|(b, _)| b.as_str())
            .unwrap_or(path);
        let inner = cdz_render_at(
            inner_ty,
            render_path,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            logical_path,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        let body = format!("format!(\"((. Qty of) {{}} {unit_lit})\", {inner})");
        return match scaled_path {
            Some((_, letbind)) => format!("{{ {letbind} {body} }}"),
            None => body,
        };
    }
    // A `BigInt` value is the Rust `cdz_num::Big` the backend emits — render it as its exact decimal, the
    // BARE integer text cdz-run prints for a BigInt (`42`, `-58`), via `Big::to_decimal_string`. `{path}`
    // is a `Big`/`&Big`; the method takes `&self`, so a reference works. Matches the runtime's BigInt
    // value-encode (a sign-magnitude leaf rendered as its decimal).
    if ty == "BigInt" {
        return format!("({path}).to_decimal_string()");
    }
    // A `Rational` value is the Rust `cdz_num::Rational` the backend emits — render it as cdz-run's `n/d`
    // form (`1/2`, `5/1`, `-3/4`), via `Rational::to_display_string`. It is kept in lowest terms with the
    // sign on the numerator + a positive denominator, so the string matches the oracle (an integer-valued
    // rational still shows the explicit `/1`).
    if ty == "Rational" {
        return format!("({path}).to_display_string()");
    }
    // A `String` value is the Rust `String` the backend emits — render it as cdz-run's canonical
    // `"<content>"` form, ESCAPING the content with the SAME rules as `cadenza_syntax::literal::escape_string`
    // (the wasm renderer's `Leaf::Str` path applies it): `\n`/`\t`/`\r` named, `\` → `\\`, `"` → `\"`, every
    // other char verbatim. A raw passthrough (the old behavior) left a backslash or an embedded double-quote
    // UNescaped — diverging from the wasm canonical render and (for a `"`) emitting a malformed, non-reparseable
    // literal (v-cdz-smith wasm-vs-rust differential, witnesses `"\\"` and `"i\""`). The emitted block folds
    // the chars into the `"…"` string, mirroring the `Bytes` arm's `b"…"` fold. `{path}` is a `String`/`&String`.
    if ty == "String" {
        return format!(
            "{{ let mut __s = String::from(\"\\\"\"); for __c in ({path}).chars() {{ match __c {{ \
             '\\n' => __s.push_str(\"\\\\n\"), \
             '\\t' => __s.push_str(\"\\\\t\"), \
             '\\r' => __s.push_str(\"\\\\r\"), \
             '\\\\' => __s.push_str(\"\\\\\\\\\"), \
             '\\\"' => __s.push_str(\"\\\\\\\"\"), \
             __c => __s.push(__c), \
             }} }} __s.push('\\\"'); __s }}"
        );
    }
    // A `Symbol` value is the Rust `String` the backend emits (a Symbol IS its canonical text) — render it
    // as cdz-run's canonical CONSTRUCTION form `((. Symbol of) "<content>")` (`lower::const_value_ast`'s
    // Symbol surface; the gate accepts the bare value or the `(: value type)` form, so the value form
    // suffices). The inner content is ESCAPED with the SAME `escape_string` rules as the `String` arm (the
    // wasm renderer applies `escape_string` to a `Leaf::Sym` too — render.rs), so a Symbol containing a
    // backslash or quote is a well-formed, canonical literal rather than the old raw passthrough. `{path}` is
    // a `String`/`&String`.
    if ty == "Symbol" {
        return format!(
            "{{ let mut __s = String::from(\"((. Symbol of) \\\"\"); for __c in ({path}).chars() {{ match __c {{ \
             '\\n' => __s.push_str(\"\\\\n\"), \
             '\\t' => __s.push_str(\"\\\\t\"), \
             '\\r' => __s.push_str(\"\\\\r\"), \
             '\\\\' => __s.push_str(\"\\\\\\\\\"), \
             '\\\"' => __s.push_str(\"\\\\\\\"\"), \
             __c => __s.push(__c), \
             }} }} __s.push('\\\"'); __s.push(')'); __s }}"
        );
    }
    // A `Char` value is the Rust `char` the backend emits — render it as cdz-run's canonical `#\<…>` form,
    // matching `cadenza-syntax`'s `literal::render_char`: the named specials (`#\space`/`#\newline`/`#\tab`/
    // `#\return`/`#\null`), a control scalar → `#\u+HHHH` (uppercase hex, ≥4 digits), else `#\<char>`. The
    // emitted Rust block matches `char` against those cases. `{path}` is a `char`/`&char`; the block binds
    // an owned `char` via `.clone()` (a `&char` clones to `char`; `char` is Copy+Clone).
    if ty == "Char" {
        // `.clone()` (not a bare bind): `{path}` may be a `char` value OR a `&char` payload binder; `char`
        // is Copy+Clone, so `.clone()` yields an owned `char` from either (a bare `let __c: char = &char`
        // would fail).
        return format!(
            "{{ let __c: char = ({path}).clone(); match __c {{ \
             ' ' => \"#\\\\space\".to_string(), \
             '\\n' => \"#\\\\newline\".to_string(), \
             '\\t' => \"#\\\\tab\".to_string(), \
             '\\r' => \"#\\\\return\".to_string(), \
             '\\0' => \"#\\\\null\".to_string(), \
             __c if __c.is_control() => format!(\"#\\\\u+{{:04X}}\", __c as u32), \
             __c => format!(\"#\\\\{{}}\", __c), \
             }} }}"
        );
    }
    // A `Bytes` value is the Rust `Vec<u8>` the backend emits — render it as cdz-run's canonical `b"…"`
    // form, escaping each byte with the SAME rules as the runtime's `escape_byte`: `\n`/`\r`/`\t`/`\\`/`\"`
    // named, `\0`, printable ASCII `0x20..=0x7e` passthrough, else `\xHH` (lowercase hex). The emitted Rust
    // block folds the bytes into the `b"…"` string. (`{path}` is a `Vec<u8>`/`&Vec<u8>`; `.iter()` yields
    // `&u8`.)
    if ty == "Bytes" {
        return format!(
            "{{ let mut __s = String::from(\"b\\\"\"); for &__byte in ({path}).iter() {{ match __byte {{ \
             b'\\n' => __s.push_str(\"\\\\n\"), \
             b'\\r' => __s.push_str(\"\\\\r\"), \
             b'\\t' => __s.push_str(\"\\\\t\"), \
             b'\\\\' => __s.push_str(\"\\\\\\\\\"), \
             b'\\\"' => __s.push_str(\"\\\\\\\"\"), \
             0 => __s.push_str(\"\\\\0\"), \
             0x20..=0x7e => __s.push(__byte as char), \
             b => __s.push_str(&format!(\"\\\\x{{:02x}}\", b)), \
             }} }} __s.push('\\\"'); __s }}"
        );
    }
    // The BUILT-IN `Option`/`Result` map to Rust's OWN `Option`/`Result`, so a value of one is rendered by
    // MATCHING it — the driver knows both variant shapes (`Some`/`None`, `Ok`/`Err`) and cdz-run's canonical
    // BARE form for a built-in variant (`(Some <p>)`, `(None unit)`, `(Ok <p>)`, `(Err <p>)`). The payload
    // types come from the head's type ARGS (`(Option A)` → the `Some` payload is `A`; `(Result A B)` → `Ok`
    // is `A`, `Err` is `B`), each rendered recursively, so a nested `(Option (Option Int64))` or an
    // `(Option (Tuple …))` composes. Matching `&<path>` borrows (the value may be used only here) and relies
    // on default binding modes (the payload binder is a reference, which `Display`s / indexes fine). A
    // FRESH binder per match depth (`__v{depth}`, derived from the path length) avoids a shadow-capture when
    // a payload is itself a sum.
    let vbind = format!("__v{}", path.len());
    if let Some(args) = parse_head_type(ty, "Option") {
        let payload = args.first().map(String::as_str).unwrap_or("");
        // Extend the logical path with `?0` (Option's single payload) so a Qty payload looks up its
        // per-element scale in `qty_at` (the emit walk keys an Option payload as `<path>?0`).
        let child_lp = if logical_path.is_empty() {
            "?0".to_string()
        } else {
            format!("{logical_path}?0")
        };
        let inner = cdz_render_at(
            payload,
            &vbind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &child_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        return format!(
            "match &{path} {{ Some({vbind}) => format!(\"(Some {{}})\", {inner}), None => \"(None unit)\".to_string() }}"
        );
    }
    if let Some(args) = parse_head_type(ty, "Result") {
        // Ok payload → `?0`, Err payload → `?1` (the type-arg indices the emit walk keys a Result on).
        let ok_lp = if logical_path.is_empty() {
            "?0".to_string()
        } else {
            format!("{logical_path}?0")
        };
        let err_lp = if logical_path.is_empty() {
            "?1".to_string()
        } else {
            format!("{logical_path}?1")
        };
        let ok = cdz_render_at(
            args.first().map(String::as_str).unwrap_or(""),
            &vbind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &ok_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        let err = cdz_render_at(
            args.get(1).map(String::as_str).unwrap_or(""),
            &vbind,
            sums,
            newtypes,
            sum_params,
            None,
            None,
            &err_lp,
            qty_at,
            qualified_heads,
            helpers,
            on_path,
        );
        return format!(
            "match &{path} {{ Ok({vbind}) => format!(\"(Ok {{}})\", {ok}), Err({vbind}) => format!(\"(Err {{}})\", {err}) }}"
        );
    }
    // A GENERIC user sum at a concrete instantiation — `(Box Int64)`, a HEAD-APPLIED type whose head names
    // a sum with a `// cdz-sum-params[Head]: N` note. Its descriptor's payload tokens carry `T{k}`
    // placeholders (`(W T0) (E)`); substitute the instantiation's args (`[Int64]`) for the placeholders,
    // then render INLINE via a `match` (no helper `fn`, so no Rust generic type signature to spell — Rust
    // infers `Box<i64>` from the matched value). This is what lets a generic-sum boundary value render on
    // the rust gate the way it does on wasm; without it a generic-sum escape fell to the scalar path and
    // failed rustc E0277 ("doesn't implement Display").
    if let Some((head, args)) = parse_applied_type(ty)
        && let Some(variants) = sums.get(&head)
        && sum_params.get(&head).copied().unwrap_or(0) == args.len()
        && !args.is_empty()
    {
        let vbind = format!("__g{}", path.len());
        // The emitted enum ident (escaped/mangled the SAME way the backend's `sum_ident` does), used in the
        // `prog::<Enum>::` path; the PRINTED name stays the Cadenza `head`/`vname`.
        let head_ident = sum_rust_ident(&head);
        let mut arms = Vec::with_capacity(variants.len());
        for (vname, payloads) in variants {
            let vident = sum_rust_ident(vname);
            // Substitute the instantiation args for the `T{k}` placeholders in each payload token.
            let subst: Vec<String> = payloads
                .iter()
                .map(|p| subst_type_params(p, &args))
                .collect();
            match subst.len() {
                0 => arms.push(format!(
                    "prog::{head_ident}::{vident} => \"({vname} unit)\".to_string()"
                )),
                1 => {
                    let inner = cdz_render_at(
                        &subst[0],
                        &vbind,
                        sums,
                        newtypes,
                        sum_params,
                        None,
                        None,
                        logical_path,
                        qty_at,
                        qualified_heads,
                        helpers,
                        on_path,
                    );
                    arms.push(format!(
                        "prog::{head_ident}::{vident}({vbind}) => format!(\"({vname} {{}})\", {inner})"
                    ));
                }
                n => {
                    let placeholders = vec!["{}"; n].join(" ");
                    let parts: Vec<String> = subst
                        .iter()
                        .enumerate()
                        .map(|(i, pty)| {
                            cdz_render_at(
                                pty,
                                &format!("({vbind}).{i}"),
                                sums,
                                newtypes,
                                sum_params,
                                None,
                                None,
                                logical_path,
                                qty_at,
                                qualified_heads,
                                helpers,
                                on_path,
                            )
                        })
                        .collect();
                    arms.push(format!(
                        "prog::{head_ident}::{vident}({vbind}) => format!(\"({vname} {placeholders})\", {})",
                        parts.join(", ")
                    ));
                }
            }
        }
        return format!("match &{path} {{ {} }}", arms.join(", "));
    }
    // A USER sum — a bare type name (`Opt`, `P`, `E`) with an emitted `// cdz-sum[…]` descriptor giving its
    // variants (name + payload type) in discriminant order. Render by MATCHING into cdz-run's BARE form,
    // uniform with a built-in sum: a payload variant → `(<Variant> <payload>)` (payload rendered
    // recursively from its type); a nullary variant → `(<Variant> unit)`. The Rust variant identifier is
    // the SANITIZED name (matching the emitted enum); the printed name is the CADENZA variant name (the
    // descriptor's first token). A MONOMORPHIC user sum is a bare name here; a GENERIC one is handled by the
    // head-applied arm above.
    if let Some(variants) = sums.get(ty) {
        // The enum is defined INSIDE `mod prog { … }` (the driver wraps the emitted module), so the
        // driver's `fn main` names it qualified: `prog::<Enum>::<Variant>`. (A built-in Option/Result is
        // std's, unqualified — handled above.)
        //
        // A user sum is rendered through a generated recursive helper `fn __render_<Ident>(v: &prog::Ident)
        // -> String`, NOT inlined. This is what makes a RECURSIVE sum terminate: `IntList = Cons(Tuple Int64
        // IntList) | Nil` unfolds infinitely as a TYPE, so inlining `cdz_render_at` for each payload never
        // returns (the codegen itself diverges → stack overflow building the render expression). Routing
        // through a helper moves the recursion to Rust RUNTIME over the finite value: the helper matches the
        // variants, and a self-referential payload position emits a CALL to the same helper (because the sum
        // is on `on_path` when its payloads are rendered), so a `Nil` leaf terminates the runtime walk.
        // The emitted enum ident (escaped/mangled the SAME way the backend's `sum_ident` does), used in the
        // `prog::<Enum>` path + the helper name; the PRINTED name stays the Cadenza `ty`/`vname`.
        let ty_ident = sum_rust_ident(ty);
        let fn_name = format!("__render_{ty_ident}");
        if !on_path.iter().any(|s| s == ty) {
            // First time this sum is unfolded on the path — generate its helper (once; a later occurrence
            // reuses it). Push the name onto the path so a self-reference inside a payload emits a call.
            if !helpers
                .iter()
                .any(|h| h.contains(&format!("fn {fn_name}(")))
            {
                on_path.push(ty.to_string());
                // A variant's DISPLAY HEAD (including the opening paren) in the canonical value form. Most
                // user/prelude sums render a variant BARE — `(Cons …`, `(Pos …`. But a sum in the
                // `qualified_heads` set renders QUALIFIED — `((. Ast Int) …`, `((. Ast Name) …` — via member
                // access. This is a PER-SUM property the backend computes (`lower::sum_needs_qualified_heads`,
                // emitted as `// cdz-sum-qualified-heads[…]`): true iff ANY variant name is bound in the
                // prelude to a NON-variant-ctor (a type ctor / module / value), so a bare head would resolve
                // to that OTHER binding. The built-in reflection `Ast` qualifies (its `Int`/`Float`/`Bool`
                // are type ctors, `List` the list module → whole sum qualifies, so `Str`/`Name` do too);
                // `Sign`/`Ordering`/a user sum with no prelude-colliding variant name stay bare. Replaces
                // the old `ty == "Ast"` NAME hack, which wrongly qualified any sum literally named `Ast`
                // regardless of its variants (the boundary-render divergence corpus-bugfix filed). Every arm
                // then emits `{head} <payload…>)` uniformly (nullary → `{head} unit)`).
                let qualified = qualified_heads.contains(ty);
                let disp_head = |vname: &str| -> String {
                    if qualified {
                        format!("((. {ty} {vname})")
                    } else {
                        format!("({vname}")
                    }
                };
                let mut arms = Vec::with_capacity(variants.len());
                for (vname, payloads) in variants {
                    let vident = sum_rust_ident(vname);
                    let head = disp_head(vname);
                    match payloads.len() {
                        // A nullary variant → `{head} unit)`.
                        0 => arms.push(format!(
                            "prog::{ty_ident}::{vident} => \"{head} unit)\".to_string()"
                        )),
                        // A single-payload variant → `(Name <payload>)`, the payload rendered from `__p`
                        // (its own type — a scalar, tuple, record, or nested sum; kept nested if a tuple).
                        1 => {
                            // Key a Qty payload by the LOCAL `<variant>?0` (the helper is reused across
                            // call-sites, so no outer path prefix — matches the emit walk's user-sum key).
                            let child_lp = format!("{vname}?0");
                            let inner = cdz_render_at(
                                &payloads[0],
                                "__p",
                                sums,
                                newtypes,
                                sum_params,
                                None,
                                None,
                                &child_lp,
                                qty_at,
                                qualified_heads,
                                helpers,
                                on_path,
                            );
                            arms.push(format!(
                                "prog::{ty_ident}::{vident}(__p) => format!(\"{head} {{}})\", {inner})"
                            ));
                        }
                        // A MULTI-payload variant → `(Name e0 e1 …)` SPREAD FLAT. Its N payloads box as ONE
                        // Rust tuple field (`P((i64, Option<i64>))`), so bind that tuple `__p` and render
                        // each element `(__p).i` by its own payload type — the flat form the wasm value-
                        // encode produces (`(P 5 (Some 5))`), NOT the nested `(P (tuple 5 (Some 5)))`.
                        n => {
                            let placeholders = vec!["{}"; n].join(" ");
                            let parts: Vec<String> = payloads
                                .iter()
                                .enumerate()
                                .map(|(i, pty)| {
                                    // Local key `<variant>?<i>` per payload slot (helper reused; no prefix).
                                    let child_lp = format!("{vname}?{i}");
                                    cdz_render_at(
                                        pty,
                                        &format!("(__p).{i}"),
                                        sums,
                                        newtypes,
                                        sum_params,
                                        None,
                                        None,
                                        &child_lp,
                                        qty_at,
                                        qualified_heads,
                                        helpers,
                                        on_path,
                                    )
                                })
                                .collect();
                            arms.push(format!(
                                "prog::{ty_ident}::{vident}(__p) => format!(\"{head} {placeholders})\", {})",
                                parts.join(", ")
                            ));
                        }
                    }
                }
                on_path.pop();
                // `#[allow(unused)]` — a mutually-referenced helper may be defined but only reached via
                // another; the block-hoisting emits every generated fn, some unused on a given path.
                helpers.push(format!(
                    "#[allow(unused)] fn {fn_name}(__v: &prog::{ty_ident}) -> String {{ match __v {{ {} }} }}",
                    arms.join(", ")
                ));
            }
        }
        // A borrow — the value may be used only here; the helper takes `&prog::Ident`.
        return format!("{fn_name}(&{path})");
    }
    // A FLOAT (`Float32`/`Float64`) renders to cdz-run's canonical VALUE form, NOT Rust's `{}`: a whole
    // float is `N.0` (Rust's `{}` prints `42`, the corpus wants `42.0`), `-0.0` keeps its sign, and NaN
    // is the canonical `nan` (the round-trippable form the binary-AST printer emits; seq-287 routed
    // cdz-run's render_val through it — `Leaf::FloatNan` → `nan` — and this Rust-gate render matches it,
    // retiring the old `NaN` spelling). Inline it (widening a Float32 to f64 first) so both gates agree.
    if ty == "Float64" || ty == "Float32" {
        // `.clone() as f64` (not a bare `as f64`): the path may be a VALUE (`.0`, top-level `__r`) OR a
        // `&f64` reference (a payload binder in a sum-render helper `match &v { Enum::Float(__p) => … }`
        // binds `__p: &f64`), and `(&f64) as f64` is an invalid reference cast (E0606). `f64: Clone` +
        // autoref makes `.clone()` yield an owned `f64` from either, then `as f64` is a no-op / a
        // Float32→f64 widen. (Surfaced when `Ast` — a sum with a `Float` payload — became renderable once
        // its `String` payload got a rep; the helper then hit the `&f64` cast.)
        return format!(
            "{{ let __f = ({path}).clone() as f64; \
             if __f == 0.0 && __f.is_sign_negative() {{ \"-0.0\".to_string() }} \
             else if __f.is_nan() {{ \"nan\".to_string() }} \
             else if __f.fract() == 0.0 && __f.is_finite() {{ format!(\"{{:.0}}.0\", __f) }} \
             else {{ format!(\"{{}}\", __f) }} }}"
        );
    }
    // A scalar: Display it.
    format!("format!(\"{{}}\", {path})")
}

/// Parse the `// cdz-sum[<Ident>]: (<Variant> <payload-render>) (<Nullary>) …` descriptor notes the Rust
/// backend emits into a map `Ident → [(variant-name, Some(payload-type) | None)]`, variants in
/// discriminant order. The gate driver reads this to `match` a USER-sum boundary value into its canonical
/// bare form (the enum decl gives rustc the type; this gives the driver the variant structure the plain
/// return-type name erases). Only monomorphic user sums have a descriptor (see `emit_sum_descriptors`).
pub fn cdz_sum_descriptors(
    module: &str,
) -> std::collections::HashMap<String, Vec<(String, Vec<String>)>> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-sum[") else {
            continue;
        };
        let Some((ident, groups)) = rest.split_once("]:") else {
            continue;
        };
        // Each top-level `(…)` group is one variant: its first token is the Cadenza variant name, and the
        // remaining top-level tokens are its payload type render_names — ZERO (nullary), ONE (single), or N
        // (a MULTI-payload variant, whose N payloads the harness renders SPREAD FLAT). `split_top_level`
        // respects nesting, so a payload that is itself a `(Tuple …)`/`(record …)`/`(Option …)` stays one
        // token; the token COUNT is the variant's arity (a single `(Tuple …)` token = one tuple payload,
        // kept nested; N tokens = a multi-payload variant, spread).
        let variants: Vec<(String, Vec<String>)> = split_top_level(groups.trim())
            .iter()
            .filter_map(|g| {
                let inner = g.strip_prefix('(')?.strip_suffix(')')?.trim();
                let toks = split_top_level(inner);
                let (name, payloads) = toks.split_first()?;
                Some((name.trim().to_string(), payloads.to_vec()))
            })
            .collect();
        map.insert(ident.trim().to_string(), variants);
    }
    map
}

/// Parse the `// cdz-sum-params[<Ident>]: N` notes into a map `Ident → parameter count`. A GENERIC user
/// sum emits one so the driver knows how many `T{k}` placeholders its descriptor's payloads carry (hence
/// how many concrete args to bind from the result type). A monomorphic sum emits none (count 0, absent).
pub fn cdz_sum_params(module: &str) -> std::collections::HashMap<String, usize> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-sum-params[") else {
            continue;
        };
        if let Some((ident, n)) = rest.split_once("]:")
            && let Ok(count) = n.trim().parse::<usize>()
        {
            map.insert(ident.trim().to_string(), count);
        }
    }
    map
}

/// Parse the `// cdz-sum-qualified-heads[<Ident>]` marker notes into a set of sum idents whose variant
/// heads render QUALIFIED (`((. Ast Str) …)`) rather than bare (`(Str …)`) at the value boundary. A sum
/// gets this marker iff any of its variant names is bound in the prelude to a NON-variant-ctor (a type
/// ctor / module / value) — the backend's `lower::sum_needs_qualified_heads` per-sum decision, emitted so
/// the driver renders each user-sum ctor exactly as the wasm backend does. A sum absent from this set
/// renders its heads bare (the common case — `Some`/`None`/`Cons`/a user sum with no prelude-colliding
/// variant name). This REPLACES the old hard-coded `ty == "Ast"` name check, which wrongly qualified any
/// sum literally NAMED `Ast` regardless of its variants (the boundary-render divergence).
pub fn cdz_sum_qualified_heads(module: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for line in module.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("// cdz-sum-qualified-heads[")
            && let Some(ident) = rest.strip_suffix(']')
        {
            set.insert(ident.trim().to_string());
        }
    }
    set
}

/// Split a HEAD-APPLIED type `(<Head> <Arg>…)` into `(head, args)` — `"(Box Int64)"` → `("Box", ["Int64"])`,
/// `"(M Int64 Bool)"` → `("M", ["Int64", "Bool"])`. `None` if `ty` is not a parenthesized head-applied form
/// (a bare name like `Box`, or a scalar). Respects nesting via `split_top_level` (an arg that is itself a
/// `(Option …)` stays one arg). Used to recognize a generic-sum instantiation at the render site.
pub fn parse_applied_type(ty: &str) -> Option<(String, Vec<String>)> {
    let inner = ty.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    let toks = split_top_level(inner);
    let (head, args) = toks.split_first()?;
    Some((head.trim().to_string(), args.to_vec()))
}

/// Substitute the type-parameter placeholders `T0`, `T1`, … in a descriptor payload token with the concrete
/// instantiation `args` — `"T0"` with `["Int64"]` → `"Int64"`; `"(Option T0)"` → `"(Option Int64)"`. A
/// placeholder is a WHOLE token `T{k}` (a nested one inside `(Option T0)` is replaced by a bounded scan
/// over `Tk` word-boundaries). Only `k < args.len()` is substituted; a `T{k}` out of range is left as-is
/// (should not occur — the param count matches).
pub fn subst_type_params(payload: &str, args: &[String]) -> String {
    let mut s = payload.to_string();
    // Replace longest index first (T10 before T1) so a prefix match doesn't corrupt a two-digit index.
    for k in (0..args.len()).rev() {
        let placeholder = format!("T{k}");
        // Replace `Tk` only at token boundaries — surrounded by start/end, whitespace, or parens — so a
        // type name that merely CONTAINS "T0" is not mangled. Rebuild by scanning tokens split on the
        // structural chars; simplest robust form: replace `(Tk)`, ` Tk`, `Tk `, and a bare whole `Tk`.
        if s == placeholder {
            s = args[k].clone();
            continue;
        }
        s = s
            .replace(&format!("({placeholder} "), &format!("({} ", args[k]))
            .replace(&format!(" {placeholder})"), &format!(" {})", args[k]))
            .replace(&format!(" {placeholder} "), &format!(" {} ", args[k]))
            .replace(&format!("({placeholder})"), &format!("({})", args[k]));
    }
    s
}

/// Parse the `// cdz-newtype[<Ident>]: <inner-render-name>` descriptor notes into a map `Ident → inner
/// type`. An erased newtype's runtime value IS its inner type (the tag adds nothing), so the gate renders a
/// newtype-typed boundary value by its inner type — see [`cdz_render_at`]'s newtype arm.
pub fn cdz_newtype_descriptors(module: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in module.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("// cdz-newtype[") else {
            continue;
        };
        if let Some((ident, inner)) = rest.split_once("]:") {
            map.insert(ident.trim().to_string(), inner.trim().to_string());
        }
    }
    map
}

/// Split a `render_name` head-applied type `(<Head> A B …)` into its argument strings, or `None` if `ty`
/// is not `(<Head> …)`. Respects nesting — a space or paren inside a nested `(…)` group does not split.
/// Used to destructure `(Tuple T0 T1)` and `(Record (a T0) (b T1))`.
pub fn parse_head_type(ty: &str, head: &str) -> Option<Vec<String>> {
    let inner = ty.strip_prefix('(')?.strip_suffix(')')?.trim();
    let rest = inner.strip_prefix(head)?;
    // `head` must be a WHOLE token: either it is the entire content — `(Tuple)`, the empty tuple, zero args
    // — or it is followed by whitespace before its args (`(Tuple T0 …)`). The whitespace check alone would
    // reject the exact-match empty case `(Tuple)` (rest is ""), so a `(Tuple)` return type fell through to a
    // scalar `Display` of the erased Rust `()` → E0277; and it must still reject a hypothetical `(TupleX …)`
    // (rest starts with `X`, neither empty nor whitespace-led).
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(split_top_level(rest.trim()))
}

/// Split a string into top-level whitespace-separated groups, treating a balanced `(…)` as one group.
/// `"a (Tuple Int64 Bool) c"` → `["a", "(Tuple Int64 Bool)", "c"]`.
pub fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => {
                if depth == 0 && start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b')' => depth -= 1,
            _ if b.is_ascii_whitespace() && depth == 0 => {
                if let Some(st) = start.take() {
                    out.push(s[st..i].to_string());
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(st) = start {
        out.push(s[st..].to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_top_level_respects_nesting() {
        assert_eq!(
            split_top_level("a (Tuple Int64 Bool) c"),
            vec!["a", "(Tuple Int64 Bool)", "c"]
        );
        assert_eq!(split_top_level("Int64"), vec!["Int64"]);
        assert!(split_top_level("").is_empty());
    }

    #[test]
    fn rust_call_arg_marshals_native_m2_compound_forms() {
        // M3-nativized compound ARGS (`#head(…)`, head fused to its parens) marshal to the SAME Rust as the
        // legacy `(head …)` form — the fix for the nightly rust-gate-full leak (`#tuple(…)` reaching rustc as
        // an attribute-start → "expected one of ! or [, found tuple"). Byte-identical inner ⇒ same output.
        assert_eq!(rust_call_arg("#tuple(10 3)"), "(10, 3)");
        assert_eq!(rust_call_arg("(tuple 10 3)"), "(10, 3)"); // legacy form unchanged
        assert_eq!(rust_call_arg("#list(4 8)"), "vec![4, 8]");
        // Named-field record → sorted-by-name positional tuple (matches the backend's field order).
        assert_eq!(rust_call_arg("#record((= x 10) (= y 3))"), "(10, 3)");
        // Positional record value form.
        assert_eq!(rust_call_arg("#record(3 4)"), "(3, 4)");
        // A NESTED native element recurses through the same path.
        assert_eq!(rust_call_arg("#tuple(100 #tuple(10 3))"), "(100, (10, 3))");
        assert_eq!(rust_call_arg("#list(4 #tuple(1 2))"), "vec![4, (1, 2)]");
        // A `#"sym"` Symbol arg is still the symbol marshal (not caught by the compound branch — no `(`).
        assert_eq!(rust_call_arg("#\"read\""), "\"read\".to_string()");
    }

    #[test]
    fn parse_head_type_destructures_a_head_applied_type() {
        assert_eq!(
            parse_head_type("(Tuple Int64 Bool)", "Tuple"),
            Some(vec!["Int64".to_string(), "Bool".to_string()])
        );
        // The empty tuple `(Tuple)` is a whole-token head match with zero args (not a fall-through).
        assert_eq!(parse_head_type("(Tuple)", "Tuple"), Some(vec![]));
        // A different head, or a bare name, does not match.
        assert_eq!(parse_head_type("(Record (x Int64))", "Tuple"), None);
        assert_eq!(parse_head_type("Int64", "Tuple"), None);
        // A head that is merely a PREFIX of the token must not match (`TupleX` ≠ `Tuple`).
        assert_eq!(parse_head_type("(TupleX Int64)", "Tuple"), None);
    }

    #[test]
    fn cdz_sum_descriptors_parses_variant_structure() {
        // The backend emits `// cdz-sum[Ident]: (Variant payload) (Nullary) …`.
        let module = "// cdz-sum[Opt]: (Some Int64) (None)\npub fn main() -> i64 { 0 }\n";
        let sums = cdz_sum_descriptors(module);
        let opt = sums.get("Opt").expect("Opt descriptor parsed");
        assert_eq!(opt.len(), 2);
        assert_eq!(opt[0], ("Some".to_string(), vec!["Int64".to_string()]));
        assert_eq!(opt[1], ("None".to_string(), Vec::<String>::new()));
    }

    #[test]
    fn cdz_newtype_and_return_and_scale_read_their_notes() {
        let module = concat!(
            "// cdz-return[main]: (Tuple Int64 Int64)\n",
            "// cdz-newtype[Pt]: (Tuple Int64 Int64)\n",
            "pub fn main() -> (i64, i64) { (5, 5) }\n"
        );
        assert_eq!(
            cdz_return_type(module, "main").as_deref(),
            Some("(Tuple Int64 Int64)")
        );
        assert_eq!(
            cdz_newtype_descriptors(module)
                .get("Pt")
                .map(String::as_str),
            Some("(Tuple Int64 Int64)")
        );
    }

    #[test]
    fn rust_ident_sanitizes_like_the_backend() {
        assert_eq!(rust_ident("foo-bar"), "foo_bar");
        assert_eq!(rust_ident("main"), "main");
        // A leading digit is prefixed with `_` (a Rust ident can't start with a digit).
        assert_eq!(rust_ident("1x"), "_1x");
    }

    #[test]
    fn cdz_render_expr_renders_a_scalar_and_a_tuple() {
        let sums = std::collections::HashMap::new();
        let newtypes = std::collections::HashMap::new();
        let params = std::collections::HashMap::new();
        // A scalar result renders via Display of the access path.
        let scalar = cdz_render_expr(
            "Int64",
            &sums,
            &newtypes,
            &params,
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            scalar.contains("__r"),
            "scalar render references the result path: {scalar}"
        );
        // A tuple result descends into `.0`/`.1` and rebuilds the `(tuple …)` s-expr.
        let tup = cdz_render_expr(
            "(Tuple Int64 Int64)",
            &sums,
            &newtypes,
            &params,
            None,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        );
        assert!(
            tup.contains(".0") && tup.contains(".1"),
            "tuple render descends into fields: {tup}"
        );
        assert!(
            tup.contains("tuple"),
            "tuple render emits the (tuple …) head: {tup}"
        );
    }

    #[test]
    fn a_sum_renders_heads_qualified_iff_it_is_in_the_qualified_heads_set_not_by_name() {
        // Boundary-render divergence fix: the old `disp_head` qualified ANY sum literally NAMED `Ast`; now it
        // consults the per-sum `qualified_heads` set (the backend's `sum_needs_qualified_heads` decision).
        // A user sum named `Ast` with NON-colliding variants (Lit/Node) is NOT in the set → renders BARE
        // `(Lit …)`, matching wasm + the `Tree` control (the divergence corpus-bugfix filed). The SAME sum,
        // when in the set (the built-in reflection `Ast`, or a user sum with a prelude-colliding variant like
        // `Int`), renders QUALIFIED `((. Ast Lit) …)`. So qualification is a per-sum property, not the name.
        let mut sums = std::collections::HashMap::new();
        sums.insert(
            "Ast".to_string(),
            vec![
                ("Lit".to_string(), vec!["Int64".to_string()]),
                ("Node".to_string(), vec![]),
            ],
        );
        let nt = std::collections::HashMap::new();
        let params = std::collections::HashMap::new();
        let qty = std::collections::HashMap::new();
        // NOT qualified → bare.
        let empty = std::collections::HashSet::new();
        let bare = cdz_render_expr("Ast", &sums, &nt, &params, None, None, &qty, &empty);
        assert!(
            bare.contains("(Lit ") && !bare.contains("(. Ast Lit)"),
            "a sum absent from qualified_heads renders BARE (Lit …), not qualified by name:\n{bare}"
        );
        // In qualified_heads → qualified member-access heads.
        let mut q = std::collections::HashSet::new();
        q.insert("Ast".to_string());
        let qual = cdz_render_expr("Ast", &sums, &nt, &params, None, None, &qty, &q);
        assert!(
            qual.contains("(. Ast Lit)"),
            "a sum in qualified_heads renders its heads QUALIFIED ((. Ast Lit) …):\n{qual}"
        );
    }

    #[test]
    fn cdz_sum_qualified_heads_parses_the_marker_notes() {
        let module = "// cdz-sum[Ast]: (Int Int64) (Str String)\n// cdz-sum-qualified-heads[Ast]\n// cdz-sum[Foo]: (A) (B)\n";
        let set = cdz_sum_qualified_heads(module);
        assert!(set.contains("Ast"), "Ast is marked qualified-heads");
        assert!(!set.contains("Foo"), "Foo has no marker → not qualified");
    }
}
