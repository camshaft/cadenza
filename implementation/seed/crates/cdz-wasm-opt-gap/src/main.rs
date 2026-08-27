//! wasm-opt-gap — parse `wasm-opt --metrics` output + module sizes into ONE
//! `(gap ...)` s-expression record (or `(optimal ...)`), the per-case report of the
//! wasm-opt optimality-gap analysis. Std-only, zero-dependency, no wasm-opt call of
//! its own: the per-case Nix derivation runs `wasm-opt --all-features -O3/-Oz` +
//! `--metrics`, then invokes this to PARSE + FORMAT that output the way we want. The
//! aggregator derivation later collects every per-case record into the top-level
//! `wasm-opt-gaps.sexp`. See implementation/design/DESIGN-wasm-opt-gap-analysis-rcdzc.md.
//!
//! Usage:
//!   wasm-opt-gap --case NAME [--module N] --orig N --o3 N --oz N \
//!                --metrics-ours FILE --metrics-opt FILE
//!
//! The signal is SIZE + `--metrics` delta, NEVER byte identity (wasm-opt re-encodes).

use std::collections::BTreeMap;
use std::process::ExitCode;

/// Parse binaryen `--metrics` text into `category -> count`. Skips the `Metrics`
/// header, the bare `total` section label, and the `[total]` aggregate (it
/// double-counts the categories). Keeps BOTH the bracketed structural summaries
/// (`[funcs]`, `[vars]`, `[imports]`, ...) and the instruction categories
/// (`LocalGet`, `Call`, ...): the category is the first token with any surrounding
/// `[]` stripped, the value is the last integer on the line.
fn parse_metrics(text: &str) -> BTreeMap<String, i64> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(first) = line.split_whitespace().next() else {
            continue;
        };
        let cat = first.trim_matches(|c| c == '[' || c == ']');
        if cat.is_empty() || cat == "Metrics" || cat == "total" {
            continue;
        }
        // value = the last whitespace-separated token that parses as an integer
        // (skips the `:` separator binaryen prints between name and count).
        let Some(val) = line
            .split_whitespace()
            .rev()
            .find_map(|t| t.parse::<i64>().ok())
        else {
            continue;
        };
        m.insert(cat.to_string(), val);
    }
    m
}

struct Sizes {
    orig: i64,
    o3: i64,
    oz: i64,
}

/// Build the per-case sexpr record. Returns an `(optimal ...)` marker when `-O3`
/// finds no size reduction (delta <= 0) — the aggregator drops those; otherwise a
/// `(gap ...)` record with the changed metrics, the dominant dropped category, and
/// the routing `owner-lane`.
fn format_record(
    case: &str,
    module: u32,
    sizes: &Sizes,
    ours: &BTreeMap<String, i64>,
    opt: &BTreeMap<String, i64>,
) -> String {
    let d3 = sizes.orig - sizes.o3;
    let dz = sizes.orig - sizes.oz;
    if d3 <= 0 {
        return format!(
            "(optimal (case {:?}) (module {}) (size (orig {})))",
            case, module, sizes.orig
        );
    }

    // Changed categories over the union of keys, `ours -> opt`, only where they differ.
    let mut keys: Vec<&String> = ours.keys().chain(opt.keys()).collect();
    keys.sort();
    keys.dedup();
    let changed: Vec<(String, i64, i64)> = keys
        .into_iter()
        .filter_map(|k| {
            let a = *ours.get(k).unwrap_or(&0);
            let b = *opt.get(k).unwrap_or(&0);
            (a != b).then(|| (k.clone(), a, b))
        })
        .collect();

    // dominant = category with the largest DROP (a > b); ties broken by name asc
    // (deterministic). Selects the gap KIND per the taxonomy.
    let dominant = changed
        .iter()
        .filter(|(_, a, b)| a > b)
        .max_by_key(|(k, a, b)| (a - b, std::cmp::Reverse(k.clone())))
        .map(|(k, _, _)| k.as_str())
        .unwrap_or("none");

    // owner-lane: a `funcs` DROP is function-elimination (inlining a forwarding
    // wrapper) → the inliner lane (v-compiler-ml / v-core-opt); otherwise a
    // local/instruction gap → this vertical (wasm-opt).
    let funcs_dropped =
        ours.get("funcs").copied().unwrap_or(0) > opt.get("funcs").copied().unwrap_or(0);
    let lane = if funcs_dropped { "inliner" } else { "wasm-opt" };

    let metrics = changed
        .iter()
        .map(|(k, a, b)| format!("({k} {a} {b})"))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "(gap\n  (case {case:?})\n  (module {module})\n  (size (orig {}) (o3 {}) (oz {}))\n  (delta (o3 {d3}) (oz {dz}))\n  (metrics {metrics})\n  (dominant {dominant})\n  (owner-lane {lane}))",
        sizes.orig, sizes.o3, sizes.oz
    )
}

fn arg_err(msg: &str) -> ExitCode {
    eprintln!("wasm-opt-gap: {msg}");
    eprintln!(
        "usage: wasm-opt-gap --case NAME [--module N] --orig N --o3 N --oz N \\\n       --metrics-ours FILE --metrics-opt FILE"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut case: Option<String> = None;
    let mut module: u32 = 0;
    let (mut orig, mut o3, mut oz): (Option<i64>, Option<i64>, Option<i64>) = (None, None, None);
    let (mut m_ours, mut m_opt): (Option<String>, Option<String>) = (None, None);

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut take = || args.next();
        match a.as_str() {
            "--case" => case = take(),
            "--module" => {
                module = match take().and_then(|v| v.parse().ok()) {
                    Some(v) => v,
                    None => return arg_err("--module needs an integer"),
                }
            }
            "--orig" => orig = take().and_then(|v| v.parse().ok()),
            "--o3" => o3 = take().and_then(|v| v.parse().ok()),
            "--oz" => oz = take().and_then(|v| v.parse().ok()),
            "--metrics-ours" => m_ours = take(),
            "--metrics-opt" => m_opt = take(),
            other => return arg_err(&format!("unknown argument `{other}`")),
        }
    }

    let (Some(case), Some(orig), Some(o3), Some(oz), Some(m_ours), Some(m_opt)) =
        (case, orig, o3, oz, m_ours, m_opt)
    else {
        return arg_err("missing a required argument");
    };

    let read = |p: &str| std::fs::read_to_string(p);
    let (ours_text, opt_text) = match (read(&m_ours), read(&m_opt)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) => return arg_err(&format!("cannot read {m_ours}: {e}")),
        (_, Err(e)) => return arg_err(&format!("cannot read {m_opt}: {e}")),
    };

    let ours = parse_metrics(&ours_text);
    let opt = parse_metrics(&opt_text);
    println!(
        "{}",
        format_record(&case, module, &Sizes { orig, o3, oz }, &ours, &opt)
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `wasm-opt --all-features --metrics` output for the recursive-numeric
    // probe (`sum`), ours vs -O3 (see the design doc's first grounded finding).
    const OURS: &str = "Metrics\ntotal\n [exports]      : 1       \n [funcs]        : 2       \n [total]        : 45      \n [vars]         : 3       \n Block          : 2       \n Call           : 1       \n Const          : 7       \n LocalGet       : 13      \n LocalSet       : 5       \n";
    const O3: &str = "Metrics\ntotal\n [exports]      : 1       \n [funcs]        : 1       \n [total]        : 37      \n [vars]         : 2       \n Block          : 1       \n Call           : 1       \n Const          : 6       \n LocalGet       : 10      \n LocalSet       : 3       \n";

    #[test]
    fn parse_skips_aggregate_and_keeps_categories() {
        let m = parse_metrics(OURS);
        assert_eq!(m.get("funcs"), Some(&2));
        assert_eq!(m.get("LocalGet"), Some(&13));
        assert_eq!(m.get("vars"), Some(&3));
        // the bare `total` label and the `[total]` aggregate are BOTH excluded
        assert_eq!(m.get("total"), None);
        // `Call` is unchanged across ours/opt but still parsed
        assert_eq!(m.get("Call"), Some(&1));
    }

    #[test]
    fn gap_record_has_delta_metrics_and_inliner_lane() {
        let ours = parse_metrics(OURS);
        let opt = parse_metrics(O3);
        let rec = format_record(
            "sum",
            0,
            &Sizes {
                orig: 131,
                o3: 111,
                oz: 111,
            },
            &ours,
            &opt,
        );
        assert!(rec.starts_with("(gap"));
        assert!(rec.contains("(delta (o3 20) (oz 20))"));
        // funcs dropped 2->1 => inliner lane (a forwarding wrapper was inlined away)
        assert!(rec.contains("(owner-lane inliner)"), "record was: {rec}");
        // changed categories present; UNCHANGED `Call` is omitted
        assert!(rec.contains("(funcs 2 1)"));
        assert!(rec.contains("(LocalGet 13 10)"));
        assert!(!rec.contains("(Call"));
        // dominant = largest drop = LocalGet (13->10, drop 3)
        assert!(rec.contains("(dominant LocalGet)"), "record was: {rec}");
    }

    #[test]
    fn zero_size_reduction_is_optimal_marker() {
        let ours = parse_metrics(OURS);
        let rec = format_record(
            "arith",
            0,
            &Sizes {
                orig: 208,
                o3: 208,
                oz: 208,
            },
            &ours,
            &ours,
        );
        assert!(rec.starts_with("(optimal"), "record was: {rec}");
        assert!(rec.contains("(case \"arith\")"));
    }
}
