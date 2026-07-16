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
//! Output format is chosen by the `-o` file EXTENSION: `.stl` → binary STL (printer convenience), `.glb` →
//! binary glTF (design-primary, watertight-preserving). 3MF (a ZIP+XML format needing extra deps) is later.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use cdz_cad::{gltf, mesh_with_segments, parse_solid, stl};

/// The mesh file format, resolved from the output extension.
enum Format {
    Stl,
    Glb,
}

impl Format {
    /// Pick a format from the output path's extension (case-insensitive). Unknown/absent → an error listing
    /// the supported set (so a typo is a clear message, not a silently-wrong file).
    fn from_path(p: &std::path::Path) -> Result<Format, String> {
        match p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        {
            Some(e) if e == "stl" => Ok(Format::Stl),
            Some(e) if e == "glb" => Ok(Format::Glb),
            Some(e) => Err(format!(
                "unsupported output extension `.{e}` (use .stl or .glb)"
            )),
            None => Err("output has no extension (use .stl or .glb)".to_string()),
        }
    }
}

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
                "cdz-cad: {msg}\n\nusage: cdz-cad <input.sexp|-> -o <out.stl|out.glb> [--segments N]"
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

    // Resolve the output format from the extension BEFORE meshing (fail fast on a bad -o).
    let format = match Format::from_path(&args.output) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cdz-cad: {e}");
            return ExitCode::FAILURE;
        }
    };

    // 3. Mesh it with manifold, then 4. serialize to the chosen format and write.
    let m = mesh_with_segments(&solid, args.segments);
    let bytes = match format {
        Format::Stl => stl::to_binary_stl(&m),
        Format::Glb => gltf::to_glb(&m),
    };
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

    // Report the model's bounding box (extents / size / center) — the "how big is it / does it fit the bed?"
    // answer a printer workflow wants. Computed by manifold from the evaluated geometry (rotations + booleans
    // included), so it needs no language-side fold.
    match cdz_cad::bounds_with_segments(&solid, args.segments) {
        Some(b) => {
            let d = b.dimensions();
            let c = b.center();
            eprintln!(
                "cdz-cad: bounds min [{:.3}, {:.3}, {:.3}] max [{:.3}, {:.3}, {:.3}] size [{:.3}, {:.3}, {:.3}] center [{:.3}, {:.3}, {:.3}]",
                b.min[0], b.min[1], b.min[2],
                b.max[0], b.max[1], b.max[2],
                d[0], d[1], d[2],
                c[0], c[1], c[2],
            );
        }
        None => eprintln!("cdz-cad: bounds: (empty — no geometry)"),
    }
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
