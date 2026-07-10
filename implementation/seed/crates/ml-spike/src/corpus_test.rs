//! Corpus round-trip harness. Uses the REAL s-expr reader (`cdz_compiler::ast::read_all`) to
//! parse every `spec/semantics/*.sexp` file into `(case …)` nodes, then extracts each case's
//! `(input <program>)` argument as the Ast to round-trip through the ML surface.

use crate::{print_ml, read_ml};
use cdz_compiler::ast::{self, Node};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Pull the `(input <program>)` argument out of every `(case …)` form in a corpus file, using the
/// real reader so multi-line inputs, comments, quasiquote sugar, dotted names, etc. are handled
/// exactly as the compiler front door handles them.
pub fn extract_inputs(corpus_file: &Path) -> Vec<Node> {
    let content = fs::read_to_string(corpus_file).expect("read corpus file");
    let forms = match ast::read_all(&content) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut inputs = Vec::new();
    for form in &forms {
        if let Some(args) = form.as_form("case") {
            // A case is `(case "name" (doc …)? (input …) (output …|error …|trap …))`.
            for arg in args {
                if let Some(input_args) = arg.as_form("input") {
                    if input_args.len() == 1 {
                        inputs.push(input_args[0].clone());
                    }
                }
            }
        }
    }
    inputs
}

/// The "kind" of a node, for bucketing round-trip failures by the construct that broke.
fn node_kind(n: &Node) -> String {
    match n {
        Node::Int(_) => "int".into(),
        Node::Float(_) => "float".into(),
        Node::Str(_) => "string".into(),
        Node::Bool(_) => "bool".into(),
        Node::Name(_) => "name".into(),
        Node::List(items) => match items.first() {
            Some(Node::Name(h)) => format!("({})", h),
            Some(_) => "(apply)".into(),
            None => "(empty)".into(),
        },
    }
}

pub struct CorpusResult {
    pub passed: usize,
    pub failed: usize,
    /// construct-head -> (fail count, sample s-expr, sample reason)
    pub fail_buckets: BTreeMap<String, (usize, String, String)>,
}

pub fn test_corpus_file(corpus_file: &Path) -> CorpusResult {
    let inputs = extract_inputs(corpus_file);
    let mut r = CorpusResult {
        passed: 0,
        failed: 0,
        fail_buckets: BTreeMap::new(),
    };

    for original in &inputs {
        let ml = print_ml(original);
        match read_ml(&ml) {
            Ok(round) => {
                if &round == original {
                    r.passed += 1;
                } else {
                    r.failed += 1;
                    let kind = node_kind(original);
                    let entry = r.fail_buckets.entry(kind).or_insert((
                        0,
                        format!("{}  =ml=>  {}", short(original), ml),
                        "AST mismatch".into(),
                    ));
                    entry.0 += 1;
                }
            }
            Err(e) => {
                r.failed += 1;
                let kind = node_kind(original);
                let entry = r.fail_buckets.entry(kind).or_insert((
                    0,
                    format!("{}  =ml=>  {}", short(original), ml),
                    format!("parse: {}", e),
                ));
                entry.0 += 1;
            }
        }
    }
    r
}

fn short(n: &Node) -> String {
    let s = format!("{:?}", n);
    if s.len() > 90 {
        format!("{}…", &s[..90])
    } else {
        s
    }
}

/// Survey the head symbols used across all inputs (grammar-scope discovery).
pub fn survey_heads(dir: &Path) -> BTreeMap<String, usize> {
    let mut heads: BTreeMap<String, usize> = BTreeMap::new();
    let mut files: Vec<_> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sexp"))
        .collect();
    files.sort();
    for f in &files {
        for input in extract_inputs(f) {
            collect_heads(&input, &mut heads);
        }
    }
    heads
}

fn collect_heads(n: &Node, heads: &mut BTreeMap<String, usize>) {
    if let Node::List(items) = n {
        if let Some(Node::Name(h)) = items.first() {
            *heads.entry(h.clone()).or_insert(0) += 1;
        }
        for it in items {
            collect_heads(it, heads);
        }
    }
}
