//! `cdz-smith` — the fuzzer CLI.
//!
//! Subcommands:
//!   fuzz   [--iterations N] [--seed S] [--timeout SECS] [--findings DIR]
//!            run the generate→compile→file loop (the continuous / cron mode). N omitted = forever.
//!   once   <SEED>            generate + compile one seed; print the verdict (no filing).
//!   gen    <SEED>            print the generated program source for a seed.
//!   verify <FILE|SEED>       recompile a filed `.sexp` (or a seed's program); print the verdict.
//!
//! Deliberately dependency-light arg parsing (no clap) so the fuzzer binary stays small and its
//! panic surface is just the compiler's.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cdz_smith::driver::{self, Config};
use cdz_smith::oracle::{Verdict, compile_catching};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("fuzz");
    match cmd {
        "fuzz" => cmd_fuzz(&args[1..]),
        "once" => cmd_once(&args[1..]),
        "gen" => cmd_gen(&args[1..]),
        "verify" => cmd_verify(&args[1..]),
        "-h" | "--help" | "help" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("cdz-smith: unknown subcommand `{other}`\n");
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "cdz-smith — fuzz the reference compiler\n\
         \n\
         USAGE:\n\
         \x20 cdz-smith fuzz   [--iterations N] [--seed S] [--timeout SECS] [--findings DIR]\n\
         \x20 cdz-smith once   <SEED>\n\
         \x20 cdz-smith gen    <SEED>\n\
         \x20 cdz-smith verify <FILE.sexp | SEED>\n"
    );
}

fn cmd_fuzz(args: &[String]) -> ExitCode {
    let mut iterations: Option<u64> = None;
    let mut seed: Option<u64> = None;
    let mut timeout_secs: u64 = 10;
    let mut findings: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--iterations" | "-n" => iterations = it.next().and_then(|s| s.parse().ok()),
            "--seed" => seed = it.next().and_then(|s| parse_seed(s)),
            "--timeout" => timeout_secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            "--findings" => findings = it.next().map(PathBuf::from),
            other => {
                eprintln!("cdz-smith fuzz: unexpected arg `{other}`");
                return ExitCode::from(2);
            }
        }
    }

    // Resolve the findings dir: explicit flag, else discover spec/semantics/failures from cwd.
    let findings_dir = match findings {
        Some(d) => d,
        None => match cdz_smith::finding::FindingStore::discover(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
            Ok(store) => store.dir().to_path_buf(),
            Err(e) => {
                eprintln!("cdz-smith: could not locate spec/semantics/failures: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    let cfg = Config {
        iterations,
        run_seed: seed.unwrap_or_else(default_run_seed),
        timeout: Duration::from_secs(timeout_secs),
        findings_dir: findings_dir.clone(),
        commit: driver::detect_commit(),
        progress_every: 1000,
    };

    eprintln!(
        "[cdz-smith] fuzzing @{} | seed {} | timeout {}s | findings → {}",
        cfg.commit,
        cfg.run_seed,
        timeout_secs,
        findings_dir.display()
    );
    match driver::run(&cfg) {
        Ok(stats) => {
            eprintln!(
                "[cdz-smith] done: {} programs | {} crashes ({} new buckets, {} dup hits) | {} timeouts",
                stats.total(),
                stats.crashes,
                stats.new_buckets,
                stats.duplicate_hits,
                stats.timeouts
            );
            // A batch run that surfaced a NEW bucket exits non-zero so a CI/cron wrapper can notice.
            if stats.new_buckets > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("cdz-smith: run failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_once(args: &[String]) -> ExitCode {
    let Some(seed) = args.first().and_then(|s| parse_seed(s)) else {
        eprintln!("cdz-smith once: expected a SEED");
        return ExitCode::from(2);
    };
    let src = driver::program_for_seed(seed);
    println!("--- program (seed {seed}) ---\n{src}\n--- verdict ---");
    report(&driver::once(seed))
}

fn cmd_gen(args: &[String]) -> ExitCode {
    let Some(seed) = args.first().and_then(|s| parse_seed(s)) else {
        eprintln!("cdz-smith gen: expected a SEED");
        return ExitCode::from(2);
    };
    print!("{}", driver::program_for_seed(seed));
    ExitCode::SUCCESS
}

fn cmd_verify(args: &[String]) -> ExitCode {
    let Some(arg) = args.first() else {
        eprintln!("cdz-smith verify: expected a FILE.sexp or a SEED");
        return ExitCode::from(2);
    };
    // A path to a reproducer, or a bare seed.
    let source = if std::path::Path::new(arg).exists() {
        match std::fs::read_to_string(arg) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cdz-smith verify: cannot read {arg}: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else if let Some(seed) = parse_seed(arg) {
        driver::program_for_seed(seed)
    } else {
        // Try resolving relative to a discovered failures dir (so `verify foo.smith.sexp` works
        // from anywhere in the repo, matching the note's suggested command).
        match cdz_smith::finding::FindingStore::discover(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
            Ok(store) => {
                let p = store.dir().join(arg);
                match std::fs::read_to_string(&p) {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!(
                            "cdz-smith verify: `{arg}` is neither a file, a seed, nor a finding in {}",
                            store.dir().display()
                        );
                        return ExitCode::from(2);
                    }
                }
            }
            Err(_) => {
                eprintln!("cdz-smith verify: `{arg}` is neither a readable file nor a seed");
                return ExitCode::from(2);
            }
        }
    };
    report(&compile_catching(&source))
}

/// Print a verdict; exit non-zero iff it is a finding (a crash), so `verify` doubles as a check.
fn report(v: &Verdict) -> ExitCode {
    match v {
        Verdict::Compiled { component_len } => {
            println!("COMPILED ({component_len} bytes) — not a bug");
            ExitCode::SUCCESS
        }
        Verdict::Declined { code, message } => {
            println!(
                "DECLINED [{}] — not a bug: {message}",
                code.as_deref().unwrap_or("uncoded")
            );
            ExitCode::SUCCESS
        }
        Verdict::ParseError(e) => {
            println!("PARSE ERROR — not a compiler finding: {e}");
            ExitCode::from(3)
        }
        Verdict::Crash(info) => {
            println!(
                "CRASH — a bug\n  site:    {}\n  message: {}",
                info.site.as_deref().unwrap_or("<unknown>"),
                info.message.lines().next().unwrap_or("")
            );
            ExitCode::from(1)
        }
    }
}

/// Accept a seed as decimal or `0x`-prefixed hex.
fn parse_seed(s: &str) -> Option<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn default_run_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xC0FFEE)
}
