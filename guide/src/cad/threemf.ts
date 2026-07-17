/// threemf.ts — serialize a browser CAD mesh (the flat MeshResult buffers from index.ts) to 3MF, the
/// modern manifold/printer interchange format (P3, req #5: "export to STL AND 3mf in the browser").
///
/// Like stl.ts this is the mesh→bytes SERIALIZATION half only (v-cad owns it); v-guide-infra owns the
/// /cad download UI. They meet at `meshTo3mf(mesh, unit?)`, which returns a `Uint8Array` of the `.3mf`
/// container a download handler saves as `<name>.3mf`.
///
/// 3MF is a ZIP container (OPC package). We assemble the three required parts and zip them with `fflate`:
///   * `[Content_Types].xml` — from @jscadui/3mf-export's staticFiles
///   * `_rels/.rels`         — the package relationship to the model part
///   * `3D/3dmodel.model`    — the mesh XML, from @jscadui/3mf-export's `to3dmodel(...)`
/// The model XML carries the UNIT (3MF declares its own unit), so we thread the model's unit through — CAD
/// is exact Rational METERS internally (operator's Q1 ruling), so the default here is "meter": a 3MF we
/// emit is unit-declared and lossless, matching the model. `to3dmodel` takes meshes of {id, vertices,
/// indices} — the SAME flat buffers stl.ts uses, so no glTF-Transform round-trip is needed.

import { zipSync, strToU8 } from "fflate";
// @ts-expect-error — @jscadui/3mf-export ships JS with JSDoc types, no .d.ts; the shapes are documented.
import { to3dmodel, fileForContentTypes } from "@jscadui/3mf-export";

/// The flat mesh buffers a 3MF write needs — the success shape of index.ts's `MeshResult` (structural, so
/// this module does not depend on the parser).
export interface ThreeMfMesh {
  positions: Float32Array;
  indices: Uint32Array;
}

/// A 3MF-declarable length unit. CAD is exact meters internally, so "meter" is the default; the /cad UI may
/// pass another (millimeter is the printer-world common case) — the model math is unit-agnostic, this only
/// labels the exported part.
export type ThreeMfUnit = "micron" | "millimeter" | "centimeter" | "inch" | "foot" | "meter";

/// The standard OPC package relationships part pointing at the model — a fixed file for our single-model
/// package (the 3MF core relationship type + the required `/3D/3dmodel.model` target).
const DOT_RELS = `<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" />
</Relationships>`;

/// Serialize `mesh` to a `.3mf` container (a `Uint8Array`). `unit` labels the exported model part (default
/// "meter", matching CAD's exact-Rational-meter internal model).
export function meshTo3mf(mesh: ThreeMfMesh, unit: ThreeMfUnit = "meter"): Uint8Array {
  // The single build object: our mesh, with a build item so slicers see it (an empty build errors in the
  // exporter). `to3dmodel` returns the model XML string.
  const model = to3dmodel({
    meshes: [{ id: "1", vertices: mesh.positions, indices: mesh.indices, name: "cad-model" }],
    items: [{ objectID: "1" }],
    header: { unit, title: "cdz-cad model", application: "Cadenza CAD" },
  });

  // Assemble the OPC package: three parts at their exact required paths, zipped.
  const zipped = zipSync({
    "[Content_Types].xml": strToU8(fileForContentTypes.content),
    "_rels/.rels": strToU8(DOT_RELS),
    "3D/3dmodel.model": strToU8(model),
  });
  return zipped;
}
