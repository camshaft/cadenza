//! cdz-cad — the native CAD mesh driver CLI (GH #400, increment G2c).
//!
//! Turns a Cadenza program's rendered `Solid` value into a mesh file. The seam is B1 "render-tree-as-data":
//! a program built on `implementation/cad` is compiled + run (via `cdz`/`cdz-run`) and its single export
//! crosses as canonical s-expr text; this tool READS that text, parses it into a CSG tree, meshes it with
//! manifold, and writes the mesh to a file.
//!
//! Usage:
//!   cdz-cad <input.sexp> -o <out.stl>     read a rendered Solid from a file
//!   cdz-run … | cdz-cad - -o out.stl      read it from stdin (the pipe from the run surface)
//!   cdz-cad in.sexp -o out.stl --segments 64
//!
//! It is deliberately a SEPARATE binary (in the workspace-excluded cdz-cad crate) rather than a subcommand
//! of the seed `cdz`: that keeps the C++/cmake `manifold-csg` build out of the seed workspace + gate. A
//! future `cdz cad` can shell out to this bin (the `cdz calc` precedent), or the pipeline can be driven as
//! `cdz run … | cdz-cad - -o out.stl` directly.
//!
//! Only STL is written in this sub-slice (the universal printer format, zero-dependency); 3MF/glTF follow.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use cdz_cad::{mesh_with_segments, parse_solid, stl};

struct Args {
    input: String, // a path, or "-" for stdin
    output: PathBuf,
    segments: i32,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!(
                "cdz-cad: {msg}\n\nusage: cdz-cad <input.sexp|-> -o <out.stl> [--segments N]"
            );
            return ExitCode::FAILURE;
        }
    };

    // 1. Read the rendered Solid text (from a file or stdin).
    let text = match read_input(&args.input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cdz-cad: reading `{}`: {e}", args.input);
            return ExitCode::FAILURE;
        }
    };

    // 2. Parse it into a CSG tree. cdz-run wraps the value as `(: <solid> Solid)`; parse_solid unwraps it.
    let solid = match parse_solid(text.trim()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cdz-cad: {e}");
            return ExitCode::FAILURE;
        }
    };

    // 3. Mesh it with manifold, then 4. serialize to binary STL and write.
    let m = mesh_with_segments(&solid, args.segments);
    let bytes = stl::to_binary_stl(&m);
    if let Err(e) = std::fs::write(&args.output, &bytes) {
        eprintln!("cdz-cad: writing `{}`: {e}", args.output.display());
        return ExitCode::FAILURE;
    }

    eprintln!(
        "cdz-cad: wrote {} ({} triangles, {} vertices) to {}",
        human_bytes(bytes.len()),
        m.triangle_count(),
        m.vertex_count(),
        args.output.display()
    );
    ExitCode::SUCCESS
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<PathBuf> = None;
    let mut segments: i32 = cdz_cad::DEFAULT_SEGMENTS;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => {
                output = Some(PathBuf::from(it.next().ok_or("`-o` needs a file path")?));
            }
            "--segments" => {
                let s = it.next().ok_or("`--segments` needs a number")?;
                segments = s
                    .parse::<i32>()
                    .map_err(|_| format!("`--segments` expects an integer, got `{s}`"))?;
                if segments < 3 {
                    return Err("`--segments` must be at least 3".to_string());
                }
            }
            "-h" | "--help" => return Err("help".to_string()),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown flag `{other}`"));
            }
            other => {
                if input.is_some() {
                    return Err(format!("unexpected extra argument `{other}`"));
                }
                input = Some(other.to_string());
            }
        }
    }

    Ok(Args {
        input: input.ok_or("no input (a file path, or `-` for stdin)")?,
        output: output.ok_or("no output (`-o <out.stl>`)")?,
        segments,
    })
}

fn read_input(input: &str) -> std::io::Result<String> {
    if input == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(input)
    }
}

fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
    }
}
