//! threemf.rs — serialize a [`crate::Mesh`] to 3MF, the native twin of the browser 3MF writer
//! (`guide/src/cad/threemf.ts`) and the printer-world companion to STL/glTF (GH #400, export parity).
//!
//! 3MF is the modern manifold interchange (a watertight mesh + a declared unit, unlike STL's unitless
//! triangle soup). It is an OPC package: a ZIP holding three parts —
//!   * `[Content_Types].xml` — the OPC content-type map
//!   * `_rels/.rels`         — the package relationship to the model part
//!   * `3D/3dmodel.model`     — the mesh, as 3MF-core XML
//!
//! DEPENDENCY NOTE: `gltf.rs` deliberately chose GLB over 3MF to avoid "a zip + xml dep". We keep that
//! spirit: this writer is **dependency-free**. The XML is written by hand (as the browser writer does),
//! and the ZIP uses the STORE method (no compression) — a stored entry needs only a CRC-32 (a ~15-line
//! table-free implementation below) and the fixed local/central-directory records, no `flate2`/`zip`
//! crate. So native 3MF parity costs zero new dependencies, honoring cdz-cad's dependency-light design.
//!
//! The emitted 3MF declares its UNIT. CAD is exact-Rational METERS internally (the operator's model), so
//! the default is "meter"; `to_3mf_with_unit` overrides it (millimeter is the printer-world common case).

use crate::Mesh;

/// A 3MF-declarable length unit (the 3MF-core allowed set).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreeMfUnit {
    Micron,
    Millimeter,
    Centimeter,
    Inch,
    Foot,
    Meter,
}

impl ThreeMfUnit {
    fn as_str(self) -> &'static str {
        match self {
            ThreeMfUnit::Micron => "micron",
            ThreeMfUnit::Millimeter => "millimeter",
            ThreeMfUnit::Centimeter => "centimeter",
            ThreeMfUnit::Inch => "inch",
            ThreeMfUnit::Foot => "foot",
            ThreeMfUnit::Meter => "meter",
        }
    }
}

/// Serialize `mesh` to a `.3mf` container (bytes), unit = meter (matches CAD's exact-Rational-meter model).
pub fn to_3mf(mesh: &Mesh) -> Vec<u8> {
    to_3mf_with_unit(mesh, ThreeMfUnit::Meter)
}

/// Serialize `mesh` to a `.3mf` container with an explicit declared `unit`.
pub fn to_3mf_with_unit(mesh: &Mesh, unit: ThreeMfUnit) -> Vec<u8> {
    let model = model_xml(mesh, unit);
    let mut zip = ZipStore::new();
    zip.add("[Content_Types].xml", CONTENT_TYPES.as_bytes());
    zip.add("_rels/.rels", DOT_RELS.as_bytes());
    zip.add("3D/3dmodel.model", model.as_bytes());
    zip.finish()
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml" />
<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml" />
</Types>"#;

const DOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" />
</Relationships>"#;

/// The 3MF-core model XML: one object (a mesh of vertices + triangles) and one build item. `unit` is
/// declared on the `<model>` element (3MF carries its own unit — lossless with our meter model).
fn model_xml(mesh: &Mesh, unit: ThreeMfUnit) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(&format!(
        "<model unit=\"{}\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n",
        unit.as_str()
    ));
    s.push_str(" <resources>\n");
    s.push_str("  <object id=\"1\" type=\"model\">\n");
    s.push_str("   <mesh>\n");
    s.push_str("    <vertices>\n");
    for i in 0..mesh.vertex_count() {
        let b = i * 3;
        s.push_str(&format!(
            "     <vertex x=\"{}\" y=\"{}\" z=\"{}\" />\n",
            mesh.positions[b],
            mesh.positions[b + 1],
            mesh.positions[b + 2]
        ));
    }
    s.push_str("    </vertices>\n");
    s.push_str("    <triangles>\n");
    for t in 0..mesh.triangle_count() {
        s.push_str(&format!(
            "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" />\n",
            mesh.indices[t * 3],
            mesh.indices[t * 3 + 1],
            mesh.indices[t * 3 + 2]
        ));
    }
    s.push_str("    </triangles>\n");
    s.push_str("   </mesh>\n");
    s.push_str("  </object>\n");
    s.push_str(" </resources>\n");
    s.push_str(" <build>\n");
    s.push_str("  <item objectid=\"1\" />\n");
    s.push_str(" </build>\n");
    s.push_str("</model>\n");
    s
}

// ── A minimal STORE-method (uncompressed) ZIP writer — dependency-free ──────────────────────────────
// 3MF is a ZIP; a STORED entry is the raw bytes plus a CRC-32 and fixed-layout local + central-directory
// records. No compression → no `flate2`; the CRC-32 is computed inline. This is all the ZIP a valid 3MF
// package needs.

struct ZipStore {
    /// The concatenated local file records (header + name + data), in add order.
    body: Vec<u8>,
    /// One central-directory entry description per added file, for the trailing directory.
    entries: Vec<CdEntry>,
}

struct CdEntry {
    name: String,
    crc: u32,
    size: u32,
    /// Byte offset of this entry's local header within `body`.
    offset: u32,
}

impl ZipStore {
    fn new() -> Self {
        ZipStore {
            body: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Add a STORED (uncompressed) file entry.
    fn add(&mut self, name: &str, data: &[u8]) {
        let offset = self.body.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        // Local file header (signature 0x04034b50), STORE method (0), zeroed time/date.
        self.body.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.body.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.body.extend_from_slice(&0u16.to_le_bytes()); // flags
        self.body.extend_from_slice(&0u16.to_le_bytes()); // method = STORE
        self.body.extend_from_slice(&0u16.to_le_bytes()); // mod time
        self.body.extend_from_slice(&0u16.to_le_bytes()); // mod date
        self.body.extend_from_slice(&crc.to_le_bytes());
        self.body.extend_from_slice(&size.to_le_bytes()); // compressed size = size (STORE)
        self.body.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        self.body
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.body.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.body.extend_from_slice(name.as_bytes());
        self.body.extend_from_slice(data);
        self.entries.push(CdEntry {
            name: name.to_string(),
            crc,
            size,
            offset,
        });
    }

    /// Emit the full ZIP: the local records, then the central directory, then the end-of-directory record.
    fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.body.len() as u32;
        let mut cd = Vec::new();
        for e in &self.entries {
            cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central dir signature
            cd.extend_from_slice(&20u16.to_le_bytes()); // version made by
            cd.extend_from_slice(&20u16.to_le_bytes()); // version needed
            cd.extend_from_slice(&0u16.to_le_bytes()); // flags
            cd.extend_from_slice(&0u16.to_le_bytes()); // method = STORE
            cd.extend_from_slice(&0u16.to_le_bytes()); // mod time
            cd.extend_from_slice(&0u16.to_le_bytes()); // mod date
            cd.extend_from_slice(&e.crc.to_le_bytes());
            cd.extend_from_slice(&e.size.to_le_bytes()); // compressed size
            cd.extend_from_slice(&e.size.to_le_bytes()); // uncompressed size
            cd.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes()); // extra len
            cd.extend_from_slice(&0u16.to_le_bytes()); // comment len
            cd.extend_from_slice(&0u16.to_le_bytes()); // disk number
            cd.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            cd.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            cd.extend_from_slice(&e.offset.to_le_bytes());
            cd.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = cd.len() as u32;
        let n = self.entries.len() as u16;
        self.body.extend_from_slice(&cd);
        // End of central directory record (signature 0x06054b50).
        self.body.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.body.extend_from_slice(&0u16.to_le_bytes()); // this disk
        self.body.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        self.body.extend_from_slice(&n.to_le_bytes()); // entries this disk
        self.body.extend_from_slice(&n.to_le_bytes()); // entries total
        self.body.extend_from_slice(&cd_size.to_le_bytes());
        self.body.extend_from_slice(&cd_offset.to_le_bytes());
        self.body.extend_from_slice(&0u16.to_le_bytes()); // comment len
        self.body
    }
}

/// CRC-32 (IEEE 802.3, the ZIP polynomial 0xEDB88320), computed bitwise — no lookup table, no dependency.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, parse_solid};

    fn cube_mesh() -> Mesh {
        mesh(&parse_solid("(: (Cube (: (tuple 2/1 2/1 2/1) Vec3)) Solid)").unwrap())
    }

    #[test]
    fn crc32_matches_known_vector() {
        // The canonical CRC-32 of "123456789" is 0xCBF43926 — pins the bitwise implementation.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_cube_serializes_to_a_nonempty_3mf_with_zip_magic() {
        let bytes = to_3mf(&cube_mesh());
        assert!(bytes.len() > 100, "a cube 3MF has real content");
        // A ZIP starts with the local file header signature "PK\x03\x04".
        assert_eq!(
            &bytes[0..4],
            &[0x50, 0x4B, 0x03, 0x04],
            "3MF is a zip (PK header)"
        );
        // The end-of-central-directory signature "PK\x05\x06" is present.
        assert!(
            bytes.windows(4).any(|w| w == [0x50, 0x4B, 0x05, 0x06]),
            "the zip has an end-of-central-directory record"
        );
    }

    #[test]
    fn the_model_xml_declares_the_unit_and_geometry() {
        // The model part (embedded as raw bytes in the STORE zip) is findable as a UTF-8 substring.
        let bytes = to_3mf(&cube_mesh());
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("unit=\"meter\""),
            "declares unit=meter by default"
        );
        assert!(text.contains("<mesh>"), "carries a mesh");
        assert!(text.contains("<triangle "), "carries triangles");
        let triangles = text.matches("<triangle ").count();
        assert_eq!(triangles, 12, "a cube is 12 triangles");
    }

    #[test]
    fn a_chosen_unit_is_declared() {
        let bytes = to_3mf_with_unit(&cube_mesh(), ThreeMfUnit::Millimeter);
        assert!(String::from_utf8_lossy(&bytes).contains("unit=\"millimeter\""));
    }

    #[test]
    fn content_type_and_rels_parts_are_present() {
        let bytes = to_3mf(&cube_mesh());
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("[Content_Types].xml"),
            "content-types part named in the zip"
        );
        assert!(text.contains("_rels/.rels"), "rels part named in the zip");
        assert!(
            text.contains("3D/3dmodel.model"),
            "model part at the required path"
        );
    }
}
