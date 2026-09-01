//! Parse a WIT world declaration and rebuild each world as the canonical `cadenza_ast` `world_schema_tree`
//! binary-AST artifact — the `KIND_WIT_WORLD` input a Cadenza reducer guest consumes via
//! `cdz compile guest.cdz wit-world:<world>=<world>.bin`.
//!
//! The artifact bytes are exactly `codec::encode` of a `world_schema_tree(name, interfaces)` root — the node
//! rcdzc's `wit_world::parse_target_world` walks back (`(world <name> <iface>…)`), whose per-member func and
//! type descriptors are built through the shared `wit_func_sig` / `wit_type_*` builders so the world means
//! the same tree regardless of source (external artifact, in-source `(world …)`, or compiler-ml emit).
//!
//! WIT-text parsing lives here (the `wit-parser` crate `cargo component` also uses); the `cadenza_ast`
//! builders do the binary-AST construction; rcdzc consumes the artifact. WIT resources and named
//! type-aliases are a later slice (`world_schema_tree` v0); the reducer worlds use only
//! records/variants/enums/flags/tuples/lists/options/results/primitives, all inlined structurally here.

use cadenza_ast::ast::{Builder, StructId, WitDir};
use cadenza_ast::codec;
use wit_parser::{Resolve, Type, TypeDefKind, TypeId, WorldItem};

/// A parsed WIT document. Parse once, then list its worlds and emit each world's artifact bytes.
pub struct Worlds {
    resolve: Resolve,
}

impl Worlds {
    /// Parse a `.wit` document from its source text. `label` is the filename used in parse diagnostics.
    pub fn parse(label: &str, wit_src: &str) -> Result<Self, String> {
        Self::parse_with_deps(&[], label, wit_src)
    }

    /// Parse a `.wit` document that may IMPORT another WIT package. Each `(label, src)` in `deps` is pushed
    /// into the SAME `Resolve` FIRST (dependency order), so the main document's cross-package import (e.g. a
    /// test world's `import cadenza:platform/reducer.{…}`, whose package lives in a SIBLING `world.wit`)
    /// resolves — `wit-parser` binds a package reference against packages already in the resolve. `push_str`
    /// per file (not `push_dir`) because the dep + main are DISTINCT packages, not one package's dir tree.
    /// The resolve then holds worlds from every pushed package; name the wanted world(s) on the CLI so only
    /// the main document's world is emitted (not the deps').
    pub fn parse_with_deps(
        deps: &[(String, String)],
        label: &str,
        wit_src: &str,
    ) -> Result<Self, String> {
        let mut resolve = Resolve::default();
        for (dlabel, dsrc) in deps {
            resolve
                .push_str(dlabel, dsrc)
                .map_err(|e| format!("parse dep {dlabel}: {e}"))?;
        }
        resolve
            .push_str(label, wit_src)
            .map_err(|e| format!("parse {label}: {e}"))?;
        Ok(Self { resolve })
    }

    /// The names of every world the document declares, in declaration order.
    pub fn names(&self) -> Vec<String> {
        self.resolve
            .worlds
            .iter()
            .map(|(_, w)| w.name.clone())
            .collect()
    }

    /// Build the `codec::encode`d `world_schema_tree` artifact for the world named `world_name`.
    pub fn artifact(&self, world_name: &str) -> Result<Vec<u8>, String> {
        let (_, world) = self
            .resolve
            .worlds
            .iter()
            .find(|(_, w)| w.name == world_name)
            .ok_or_else(|| format!("no world `{world_name}` declared"))?;

        let mut b = Builder::new();
        let mut interfaces = Vec::new();
        // Imports first (host-provided), then the exports (guest-provided) — caller order is preserved in
        // the tree; the direction is structural (`import`/`export` sub-head), so the two are distinguishable.
        // A type-ONLY interface (e.g. `types`, holding only `type` aliases a peer `use`s) carries no
        // functions: wit-parser lists it among the world's imports because other interfaces `use` it, but it
        // is a type namespace, not a capability — it contributes no importable op, and the shared world tree
        // carries only interfaces with members (matching the in-source `(world …)` form v-syntax lowers). So
        // skip it.
        push_interfaces(
            &mut b,
            &self.resolve,
            world.imports.values(),
            WitDir::Import,
            world_name,
            &mut interfaces,
        )?;
        push_interfaces(
            &mut b,
            &self.resolve,
            world.exports.values(),
            WitDir::Export,
            world_name,
            &mut interfaces,
        )?;
        let root = b.world_schema_tree(world_name, &interfaces);
        let arenas = b.finish(root);
        Ok(codec::encode(&arenas))
    }
}

/// Push each interface-WITH-members from `items` as a `dir` interface node into `out`, skipping the
/// type-only interfaces (no functions — a `use`d type namespace, not a capability). Shared by the world's
/// import and export passes, which differ only in the item set and the [`WitDir`].
fn push_interfaces<'a>(
    b: &mut Builder,
    resolve: &Resolve,
    items: impl IntoIterator<Item = &'a WorldItem>,
    dir: WitDir,
    world_name: &str,
    out: &mut Vec<StructId>,
) -> Result<(), String> {
    for item in items {
        if let WorldItem::Interface { id, .. } = item {
            if resolve.interfaces[*id].functions.is_empty() {
                continue;
            }
            out.push(build_interface(b, resolve, *id, dir, world_name)?);
        }
    }
    Ok(())
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
            // `PackageName`'s Display is `ns:name` (plus `@version` when versioned); an unversioned
            // `package cadenza:platform;` is `cadenza:platform`.
            Ok(format!("{pkg_name}/{short}"))
        }
        None => {
            // No owning package — the short name is the only name available. Degrade gracefully rather than
            // fail the whole artifact.
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
                "unsupported WIT type-def `{what}` ({other:?}) — WIT resources/handles and \
                 futures/streams are a later slice (world_schema_tree v0)"
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A self-contained WIT fixture exercising the mapping: an imported capability interface with a func
    /// taking a record and returning an `option<list<u8>>`, and an exported guest interface returning a
    /// named variant. Decoupled from any sibling crate's `world.wit` (a clean crate boundary — the point of
    /// this standalone utility), so the mapping is tested without reaching outside the crate.
    const FIXTURE: &str = r#"
        package cadenza:fixture;

        interface store {
            record key { bytes: list<u8> }
            get: func(k: key) -> option<list<u8>>;
        }

        interface guest {
            variant outcome { continue, close(string) }
            step: func(msg: list<u8>) -> outcome;
        }

        world ordinary {
            import store;
            export guest;
        }
    "#;

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

    #[test]
    fn names_lists_every_declared_world() {
        let worlds = Worlds::parse("fixture.wit", FIXTURE).expect("parse fixture");
        assert_eq!(worlds.names(), vec!["ordinary".to_string()]);
    }

    #[test]
    fn artifact_is_a_well_formed_world_tree_with_fq_import_and_export() {
        let worlds = Worlds::parse("fixture.wit", FIXTURE).expect("parse fixture");
        let bytes = worlds.artifact("ordinary").expect("build ordinary");
        let arenas = codec::decode(&bytes).expect("decode the artifact");
        // The root is `(world <name> <iface>…)` — `as_form` returns the children after the `world` head.
        let items = arenas
            .as_form(arenas.root, "world")
            .expect("root is a `world` form");
        assert_eq!(
            arenas.as_name(items[0]),
            Some("ordinary"),
            "first child is the world name"
        );
        let (names, dirs) = interface_names_and_dirs(&arenas, &items[1..]);
        // Interface names are FULLY QUALIFIED (`cadenza:fixture/<iface>`) so the export self-describes the
        // component name and imports match the host's component-model names.
        assert!(
            names.contains(&"cadenza:fixture/store"),
            "artifact is missing the imported interface (found {names:?})"
        );
        assert!(
            names.contains(&"cadenza:fixture/guest"),
            "artifact is missing the exported interface (found {names:?})"
        );
        assert!(dirs.contains(&"import"));
        assert!(dirs.contains(&"export"));
    }

    #[test]
    fn unknown_world_is_an_error() {
        let worlds = Worlds::parse("fixture.wit", FIXTURE).expect("parse fixture");
        let err = worlds.artifact("nope").expect_err("no such world");
        assert!(err.contains("no world `nope`"), "got: {err}");
    }
}
