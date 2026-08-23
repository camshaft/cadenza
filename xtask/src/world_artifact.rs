//! Produce the platform's `KIND_WIT_WORLD` binary-AST artifacts from `cdz-platform/wit/world.wit`.
//!
//! A Cadenza reducer guest targets the platform's typed reducer world by consuming a preparsed binary-AST
//! world artifact: `cdz compile guest.cdz wit-world:reducer-world=reducer-world.bin` (the `wit-world` =
//! [`KIND_WIT_WORLD`] input kind). The compiler never parses WIT text — by design (`wit_world.rs`), the
//! structured world reaches rcdzc only as this binary tree. This tool is the bridge: it parses the ONE
//! source-of-truth `world.wit` with `wit-parser` (the same crate `cargo component` uses for the Rust
//! reducer-echo) and rebuilds each world as the canonical `cadenza_ast` `world_schema_tree` node, so a guest
//! can target the shared `reducer-world` / `event-reducer-world` without re-declaring it inline.
//!
//! The artifact bytes are exactly `codec::encode` of a `world_schema_tree(name, interfaces)` root — the node
//! rcdzc's `wit_world::parse_target_world` walks back (`(world <name> <iface>…)`), whose per-member func and
//! type descriptors are built through the shared `wit_func_sig` / `wit_type_*` builders so the world means
//! the same tree regardless of source (external artifact, in-source `(world …)`, or compiler-ml emit).
//!
//! It regenerates from `world.wit` on every run (no committed binary, so no staleness to drift): the nix
//! reducer-guest derivation calls this to produce the artifact it feeds `cdz compile`. WIT-text parsing
//! lives here in the build tool; the `cadenza_ast` builders do the binary-AST construction; rcdzc consumes
//! the artifact. WIT resources and named type-aliases are a later slice (`world_schema_tree` v0); the
//! current `world.wit` reducer worlds use only records/variants/enums/lists/options/results/primitives, all
//! inlined structurally here.

use crate::Paths;
use cadenza_ast::ast::{Builder, StructId, WitDir};
use cadenza_ast::codec;
use std::path::Path;
use wit_parser::{Resolve, Type, TypeDefKind, TypeId, WorldItem};

/// The worlds `world.wit` declares that a reducer guest may target — the ordinary floor and the privileged
/// superset (design/cadenza-platform.md §3). Each is emitted as its own artifact file `<name>.bin`.
const WORLDS: &[&str] = &["reducer-world", "event-reducer-world"];

/// Regenerate the `KIND_WIT_WORLD` artifacts. Writes `<out>/<world>.bin` per [`WORLDS`]; `out` defaults to
/// `<repo>/target/wit-worlds`. Exits the process non-zero (after printing) on any parse/build/write error,
/// matching `codegen::run`'s style — a build step, not a library call.
pub fn run(
    paths: &Paths,
    out: Option<std::path::PathBuf>,
    wit: Option<std::path::PathBuf>,
    world: Option<String>,
) {
    let wit_path = wit.unwrap_or_else(|| paths.seed.join("crates/cdz-platform/wit/world.wit"));
    let out_dir = out.unwrap_or_else(|| paths.repo.join("target/wit-worlds"));
    let worlds: Vec<String> = match world {
        Some(w) => vec![w],
        None => WORLDS.iter().map(|s| s.to_string()).collect(),
    };

    if let Err(e) = generate(&wit_path, &out_dir, &worlds) {
        eprintln!("xtask world-artifact: {e}");
        std::process::exit(1);
    }
}

/// Parse `world.wit` and write each world's artifact into `out_dir`, returning the paths written.
fn generate(wit_path: &Path, out_dir: &Path, worlds: &[String]) -> Result<(), String> {
    let mut resolve = Resolve::default();
    resolve
        .push_file(wit_path)
        .map_err(|e| format!("parse {}: {e}", wit_path.display()))?;

    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    for world_name in worlds {
        let bytes = build_world(&resolve, world_name)?;
        let out = out_dir.join(format!("{world_name}.bin"));
        std::fs::write(&out, &bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
        println!(
            "xtask world-artifact: wrote {} ({} bytes) from {}",
            out.display(),
            bytes.len(),
            wit_path.display()
        );
    }
    Ok(())
}

/// Build the `codec::encode`d `world_schema_tree` artifact for the world named `world_name`.
fn build_world(resolve: &Resolve, world_name: &str) -> Result<Vec<u8>, String> {
    let (_, world) = resolve
        .worlds
        .iter()
        .find(|(_, w)| w.name == world_name)
        .ok_or_else(|| format!("world.wit declares no world `{world_name}`"))?;

    let mut b = Builder::new();
    let mut interfaces = Vec::new();
    // Imports first (host-provided), then the exports (guest-provided) — caller order is preserved in the
    // tree; the direction is structural (`import`/`export` sub-head), so the two are distinguishable.
    // A type-ONLY interface (e.g. `types`, holding only `type` aliases a peer `use`s) carries no functions:
    // wit-parser lists it among the world's imports because other interfaces `use` it, but it is a type
    // namespace, not a capability — it contributes no importable op, and the shared world tree carries only
    // interfaces with members (matching the in-source `(world …)` form v-syntax lowers). So skip it.
    for (_, item) in &world.imports {
        if let WorldItem::Interface { id, .. } = item {
            if resolve.interfaces[*id].functions.is_empty() {
                continue;
            }
            interfaces.push(build_interface(
                &mut b,
                resolve,
                *id,
                WitDir::Import,
                world_name,
            )?);
        }
    }
    for (_, item) in &world.exports {
        if let WorldItem::Interface { id, .. } = item {
            if resolve.interfaces[*id].functions.is_empty() {
                continue;
            }
            interfaces.push(build_interface(
                &mut b,
                resolve,
                *id,
                WitDir::Export,
                world_name,
            )?);
        }
    }
    let root = b.world_schema_tree(world_name, &interfaces);
    let arenas = b.finish(root);
    Ok(codec::encode(&arenas))
}

/// Build one interface node `(<dir> Name (member MName FuncSig)…)`, its members sorted by name (rcdzc
/// resolves by name, so a stable order is deterministic).
fn build_interface(
    b: &mut Builder,
    resolve: &Resolve,
    id: wit_parser::InterfaceId,
    dir: WitDir,
    world_name: &str,
) -> Result<StructId, String> {
    let iface = &resolve.interfaces[id];
    let short = iface
        .name
        .as_deref()
        .ok_or_else(|| format!("{world_name}: an interface in the world has no name"))?;
    // Emit the FULLY-QUALIFIED WIT name `ns:pkg/iface` (e.g. `cadenza:platform/guest`), not the bare short
    // name. The host composes an import and publishes an export under the FQ component-model name, and rcdzc
    // derives a self-describing reducer's component-name from its FQ export interface (so it compiles with no
    // `--component-name` flag). Import-effect synthesis still binds by the short (last-`/`-segment) name, so
    // a performed `identity.id` resolves against `cadenza:platform/identity` all the same.
    let name = fq_name(resolve, iface, short, world_name)?;

    // Build each member's func sig first (owning the param-name Strings), then borrow them for `wit_interface`.
    let mut funcs: Vec<(String, StructId)> = Vec::with_capacity(iface.functions.len());
    for f in iface.functions.values() {
        let mut params: Vec<(String, StructId)> = Vec::with_capacity(f.params.len());
        for (pname, ty) in &f.params {
            params.push((pname.clone(), map_type(b, resolve, *ty)?));
        }
        let result = match f.result {
            Some(ty) => map_type(b, resolve, ty)?,
            None => b.wit_type_unit(),
        };
        let param_refs: Vec<(&str, StructId)> =
            params.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        funcs.push((f.name.clone(), b.wit_func_sig(&param_refs, result)));
    }
    funcs.sort_by(|a, c| a.0.cmp(&c.0));

    let member_refs: Vec<(&str, StructId)> = funcs.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    Ok(b.wit_interface(dir, &name, &member_refs))
}

/// The fully-qualified WIT name `ns:pkg/iface` for an interface — its owning package name plus the short
/// interface name (e.g. `cadenza:platform` + `guest` → `cadenza:platform/guest`). Falls back to the short
/// name if the interface has no owning package (a standalone interface not in a package).
fn fq_name(
    resolve: &Resolve,
    iface: &wit_parser::Interface,
    short: &str,
    world_name: &str,
) -> Result<String, String> {
    match iface.package {
        Some(pkg) => {
            let pkg_name = &resolve.packages[pkg].name;
            // `PackageName`'s Display is `ns:name` (plus `@version` when versioned); `world.wit`'s
            // `package cadenza:platform;` is unversioned, so this is `cadenza:platform`.
            Ok(format!("{pkg_name}/{short}"))
        }
        None => {
            // No owning package — the short name is the only name available. Not expected for the platform
            // worlds, but degrade gracefully rather than fail the whole artifact.
            let _ = world_name;
            Ok(short.to_string())
        }
    }
}

/// Map a `wit-parser` type to its canonical `cadenza_ast` WIT type descriptor, inlining named types /
/// aliases structurally (`world_schema_tree` v0 carries no named-type nodes).
fn map_type(b: &mut Builder, resolve: &Resolve, ty: Type) -> Result<StructId, String> {
    Ok(match ty {
        Type::Bool => b.wit_type_prim("bool"),
        Type::U8 => b.wit_type_prim("u8"),
        Type::U16 => b.wit_type_prim("u16"),
        Type::U32 => b.wit_type_prim("u32"),
        Type::U64 => b.wit_type_prim("u64"),
        Type::S8 => b.wit_type_prim("s8"),
        Type::S16 => b.wit_type_prim("s16"),
        Type::S32 => b.wit_type_prim("s32"),
        Type::S64 => b.wit_type_prim("s64"),
        Type::F32 => b.wit_type_prim("f32"),
        Type::F64 => b.wit_type_prim("f64"),
        Type::Char => b.wit_type_prim("char"),
        Type::String => b.wit_type_prim("string"),
        Type::Id(id) => map_typedef(b, resolve, id)?,
        // `ErrorContext` and any future primitive are not used by the reducer worlds; fail loud rather than
        // silently emit a wrong descriptor.
        other => return Err(format!("unsupported WIT type {other:?} in a reducer world")),
    })
}

/// Map a named type-def to a descriptor. An alias (`type hash = list<u8>`) inlines to its target; a record /
/// variant / enum / flags / tuple / option / result / list build the corresponding compound descriptor.
fn map_typedef(b: &mut Builder, resolve: &Resolve, id: TypeId) -> Result<StructId, String> {
    let td = &resolve.types[id];
    Ok(match &td.kind {
        TypeDefKind::Type(inner) => map_type(b, resolve, *inner)?,
        TypeDefKind::List(elem) => {
            let e = map_type(b, resolve, *elem)?;
            b.wit_type_list(e)
        }
        TypeDefKind::Option(inner) => {
            let i = map_type(b, resolve, *inner)?;
            b.wit_type_option(i)
        }
        TypeDefKind::Result(r) => {
            let ok = r.ok.map(|t| map_type(b, resolve, t)).transpose()?;
            let err = r.err.map(|t| map_type(b, resolve, t)).transpose()?;
            b.wit_type_result(ok, err)
        }
        TypeDefKind::Tuple(t) => {
            let elems = t
                .types
                .iter()
                .map(|&e| map_type(b, resolve, e))
                .collect::<Result<Vec<_>, _>>()?;
            b.wit_type_tuple(&elems)
        }
        TypeDefKind::Record(rec) => {
            let mut fields: Vec<(String, StructId)> = Vec::with_capacity(rec.fields.len());
            for f in &rec.fields {
                fields.push((f.name.clone(), map_type(b, resolve, f.ty)?));
            }
            let field_refs: Vec<(&str, StructId)> =
                fields.iter().map(|(n, d)| (n.as_str(), *d)).collect();
            b.wit_type_record(&field_refs)
        }
        TypeDefKind::Variant(v) => {
            let mut cases: Vec<(String, Option<StructId>)> = Vec::with_capacity(v.cases.len());
            for c in &v.cases {
                let payload = c.ty.map(|t| map_type(b, resolve, t)).transpose()?;
                cases.push((c.name.clone(), payload));
            }
            let case_refs: Vec<(&str, Option<StructId>)> =
                cases.iter().map(|(n, d)| (n.as_str(), *d)).collect();
            b.wit_type_variant(&case_refs)
        }
        TypeDefKind::Enum(e) => {
            let names: Vec<&str> = e.cases.iter().map(|c| c.name.as_str()).collect();
            b.wit_type_enum(&names)
        }
        TypeDefKind::Flags(f) => {
            let names: Vec<&str> = f.flags.iter().map(|fl| fl.name.as_str()).collect();
            b.wit_type_flags(&names)
        }
        other => {
            let what = td.name.as_deref().unwrap_or("<anonymous>");
            return Err(format!(
                "unsupported WIT type-def `{what}` ({other:?}) in a reducer world — WIT resources/handles \
                 and futures/streams are a later slice (world_schema_tree v0)"
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `reducer-world` artifact round-trips: it decodes to a `(world reducer-world …)` tree whose
    /// interfaces include the four ordinary imports and the `guest` export, each carrying `member` funcs —
    /// the exact node `wit_world::parse_target_world` walks. Guards the builder mapping against drift.
    #[test]
    fn reducer_world_artifact_is_a_well_formed_world_tree() {
        let mut resolve = Resolve::default();
        resolve
            .push_str(
                "world.wit",
                include_str!("../../implementation/seed/crates/cdz-platform/wit/world.wit"),
            )
            .expect("parse world.wit");
        let bytes = build_world(&resolve, "reducer-world").expect("build reducer-world");

        let arenas = codec::decode(&bytes).expect("decode the artifact");
        // The root is `(world <name> <iface>…)` — `as_form` returns the children after the `world` head.
        let items = arenas
            .as_form(arenas.root, "world")
            .expect("root is a `world` form");
        assert_eq!(
            arenas.as_name(items[0]),
            Some("reducer-world"),
            "first child is the world name"
        );
        let (names, dirs) = interface_names_and_dirs(&arenas, &items[1..]);
        // Interface names are FULLY QUALIFIED (`cadenza:platform/<iface>`) so the export self-describes the
        // component name and imports match the host's component-model names.
        for expected in [
            "cadenza:platform/state",
            "cadenza:platform/blobs",
            "cadenza:platform/identity",
            "cadenza:platform/run",
            "cadenza:platform/guest",
        ] {
            assert!(
                names.contains(&expected),
                "reducer-world artifact is missing interface `{expected}` (found {names:?})"
            );
        }
        assert!(dirs.contains(&"import"));
        assert!(dirs.contains(&"export"));
    }

    /// The privileged superset adds the graph / deliver / provenance imports.
    #[test]
    fn event_reducer_world_adds_the_privileged_imports() {
        let mut resolve = Resolve::default();
        resolve
            .push_str(
                "world.wit",
                include_str!("../../implementation/seed/crates/cdz-platform/wit/world.wit"),
            )
            .expect("parse world.wit");
        let bytes =
            build_world(&resolve, "event-reducer-world").expect("build event-reducer-world");
        let arenas = codec::decode(&bytes).expect("decode the artifact");
        let items = arenas
            .as_form(arenas.root, "world")
            .expect("root is a `world` form");
        let (names, _dirs) = interface_names_and_dirs(&arenas, &items[1..]);
        for expected in [
            "cadenza:platform/graph",
            "cadenza:platform/deliver",
            "cadenza:platform/provenance",
        ] {
            assert!(
                names.contains(&expected),
                "event-reducer-world artifact is missing privileged interface `{expected}` (found {names:?})"
            );
        }
    }

    /// Read each interface node `(import|export <Name> <member>…)` in `ifaces`, returning the interface
    /// names and the set of directions seen — via the public `head_name`/`as_form`/`as_name` accessors.
    fn interface_names_and_dirs<'a>(
        arenas: &'a cadenza_ast::ast::Arenas,
        ifaces: &[StructId],
    ) -> (Vec<&'a str>, Vec<&'a str>) {
        let mut names = Vec::new();
        let mut dirs = Vec::new();
        for &iface in ifaces {
            let Some(dir) = arenas.head_name(iface) else {
                continue;
            };
            dirs.push(dir);
            if let Some(children) = arenas.as_form(iface, dir)
                && let Some(name) = children.first().and_then(|&n| arenas.as_name(n))
            {
                names.push(name);
            }
        }
        (names, dirs)
    }
}
