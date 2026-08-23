//! Content-address a component's BARE external imports.
//!
//! [`add_import_versions`] takes a component (a cargo-component build) and a map from a BARE import name
//! to a version suffix, and rewrites each top-level external import whose name is a key in the map so it
//! carries that version (`name` -> `name@<version>`). Every other section is copied verbatim by
//! wasm-encoder's round-trip component reencoder; only the import-name emit is overridden.
//!
//! The one caller is `cargo xtask build`: it maps `cadenza:nfc/normalize` -> `0.0.0+<nfc-hash>` so the
//! value-heap runtime's transitive NFC dependency becomes content-addressed like every other import, and
//! the runtime composes with no name->hash manifest. Keeping this the crate's ONLY responsibility (and its
//! ONLY dependencies wasmparser + wasm-encoder) keeps the re-addressing isolated from the platform host,
//! rcdzc, and cdz-run.
//!
//! The version suffix must be a valid component-import semver build metadata (`[0-9A-Za-z-]`, no `_`), so
//! the content hash it carries must be hex or base62 — never base64url (which uses `_`).

use std::collections::BTreeMap;

use wasm_encoder::reencode::{Error, Reencode, ReencodeComponent};
use wasm_encoder::{Component, ComponentImportSection};
use wasmparser::Parser;

/// Rewrite each top-level external import of `component` whose (bare) name is a key in `versions` to carry
/// the mapped version: `name` becomes `name@<version>`. Imports not in the map, and every other section,
/// are reproduced verbatim. Returns the rewritten component bytes and the number of imports rewritten (so
/// a caller can assert it actually found the import it meant to re-address).
///
/// `versions` maps a bare import name (e.g. `"cadenza:nfc/normalize"`) to a version WITHOUT the leading
/// `@` (e.g. `"0.0.0+9a57...".`). The value must be valid semver-with-build-metadata; a base64url hash is
/// rejected by the component encoder because build metadata forbids `_`.
pub fn add_import_versions(
    component: &[u8],
    versions: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, usize), String> {
    let mut rewriter = Rewriter {
        versions,
        rewrote: 0,
    };
    let mut out = Component::new();
    rewriter
        .parse_component(&mut out, Parser::new(0), component)
        .map_err(|e| format!("cdz-component-rewrite: reencoding the component failed: {e}"))?;
    Ok((out.finish(), rewriter.rewrote))
}

/// A round-trip component reencoder that only diverges from a verbatim copy in the import-name emit.
struct Rewriter<'a> {
    versions: &'a BTreeMap<String, String>,
    rewrote: usize,
}

// All reencoder behaviour is the identity round-trip (`RoundtripReencoder`'s defaults) — we never remap an
// index or type — so the impls are empty except for the one overridden hook below.
impl Reencode for Rewriter<'_> {
    type Error = std::convert::Infallible;
}

impl ReencodeComponent for Rewriter<'_> {
    fn parse_component_import_section(
        &mut self,
        imports: &mut ComponentImportSection,
        section: wasmparser::ComponentImportSectionReader<'_>,
    ) -> Result<(), Error<<Self as Reencode>::Error>> {
        for import in section {
            let import = import?;
            let ty = self.component_type_ref(import.ty)?;
            match self.versions.get(import.name.0) {
                Some(version) => {
                    self.rewrote += 1;
                    imports.import(&format!("{}@{version}", import.name.0), ty);
                }
                None => {
                    imports.import(import.name.0, ty);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_encoder::{Component, ComponentImportSection, ComponentTypeRef, ComponentTypeSection};

    /// A minimal valid component that imports one interface (an empty instance type) under `name`.
    fn component_importing(name: &str) -> Vec<u8> {
        let mut c = Component::new();
        let mut types = ComponentTypeSection::new();
        types.ty().instance(&wasm_encoder::InstanceType::new()); // type 0: empty instance
        c.section(&types);
        let mut imports = ComponentImportSection::new();
        imports.import(name, ComponentTypeRef::Instance(0));
        c.section(&imports);
        c.finish()
    }

    /// Read back the (single) top-level component import name.
    fn import_name(component: &[u8]) -> String {
        for payload in Parser::new(0).parse_all(component) {
            if let wasmparser::Payload::ComponentImportSection(reader) = payload.unwrap() {
                let import = reader.into_iter().next().unwrap().unwrap();
                return import.name.0.to_string();
            }
        }
        panic!("no component import section");
    }

    #[test]
    fn adds_the_version_to_a_matching_bare_import() {
        let original = component_importing("cadenza:nfc/normalize");
        assert_eq!(import_name(&original), "cadenza:nfc/normalize");

        let mut versions = BTreeMap::new();
        // hex here (base62 in production); either is semver-safe.
        versions.insert(
            "cadenza:nfc/normalize".to_string(),
            "0.0.0+b2a4957895809e29".to_string(),
        );
        let (rewritten, n) = add_import_versions(&original, &versions).unwrap();

        assert_eq!(n, 1, "exactly one import rewritten");
        assert_eq!(
            import_name(&rewritten),
            "cadenza:nfc/normalize@0.0.0+b2a4957895809e29"
        );
    }

    #[test]
    fn leaves_unmatched_imports_untouched() {
        let original = component_importing("cadenza:other/iface");
        let mut versions = BTreeMap::new();
        versions.insert("cadenza:nfc/normalize".to_string(), "0.0.0+abc".to_string());
        let (rewritten, n) = add_import_versions(&original, &versions).unwrap();
        assert_eq!(n, 0, "no import matched");
        assert_eq!(import_name(&rewritten), "cadenza:other/iface");
    }
}
