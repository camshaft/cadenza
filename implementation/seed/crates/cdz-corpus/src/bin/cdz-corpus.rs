//! `cdz-corpus` — read the executable-semantics corpus.
//!
//! ```text
//! cdz-corpus records FILE…             # parse corpus cases → one normalized record per case
//! ```
//!
//! This bin is a thin shim: the command surface lives in `cdz_corpus::cli` (an embeddable clap `Args`
//! group + a `run` entry), so the unified `cdz` binary can mount the SAME code as `cdz corpus …`
//! without a second binary on the PATH. xtask (which shells out to this standalone bin) keeps working;
//! both entry points share one implementation and one `--help`.

use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(name = "cdz-corpus", about = "Read the executable-semantics corpus.")]
struct Cli {
    #[command(flatten)]
    corpus: cdz_corpus::cli::CorpusArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    cdz_corpus::cli::run(&cli.corpus, "cdz-corpus")
}
