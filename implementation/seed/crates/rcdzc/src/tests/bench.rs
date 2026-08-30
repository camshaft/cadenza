use crate::compile::compile_component;
use crate::testkit::parse;
use std::time::Instant;

/// A `(module m (def (main) (let ((a0 0)(a1 a0)…(a{n-1} a{n-2})) a{n-1})) (export main))` program:
/// a chain of `n` bindings each aliasing the previous, all constant so the whole thing folds and
/// compiles end-to-end (the resolver is fully exercised — `n` distinct reference nodes, each
/// ascending into the growing bindings-list — with no decline short-circuiting the run).
fn deep_let_chain(n: usize) -> String {
    let mut binds = String::from("(a0 0)");
    for i in 1..n {
        binds.push_str(&format!("(a{i} a{})", i - 1));
    }
    format!(
        "(module m (def (main) (let ({binds}) a{})) (export main))",
        n - 1
    )
}

#[test]
#[ignore = "benchmark, not a correctness gate — run with --ignored --nocapture"]
fn scope_resolution_deep_let_chain() {
    // A few reads per size, keep the best (min) to shed scheduler noise. The absolute numbers are
    // machine-dependent; the SHAPE across N is the signal — a per-binding time that stays flat is
    // O(N) resolution, one that climbs with N is the old O(N²).
    // Sizes stay under the compiler's recursive-descent depth bound (a deeper aliasing chain
    // DECLINES at the fold/inference depth backstop — orthogonal to scope resolution); a size that
    // declines is SKIPPED, not counted, so the bench never conflates that ceiling with the timing.
    println!("\n  deep sequential let-chain — full compile_component wall time");
    println!(
        "  {:>7}  {:>12}  {:>14}",
        "N", "total (ms)", "per-bind (µs)"
    );
    for &n in &[100usize, 250, 500, 750, 1000] {
        let src = deep_let_chain(n);
        let bytes = crate::codec::encode(&parse(&src));
        // Warm once (build the arena caches); skip this size if the depth bound declines it.
        if compile_component(&bytes).is_err() {
            println!("  {n:>7}  {:>12}  {:>14}", "(declined)", "-");
            continue;
        }
        let mut best = f64::INFINITY;
        for _ in 0..5 {
            let t0 = Instant::now();
            let out = compile_component(&bytes);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            out.expect("bench program compiles");
            best = best.min(ms);
        }
        println!(
            "  {:>7}  {:>12.3}  {:>14.3}",
            n,
            best,
            best * 1e3 / n as f64
        );
    }
    println!();
}
