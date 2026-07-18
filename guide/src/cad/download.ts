/// Browser download helpers for /cad's mesh exports — trigger a file download of the current meshed model
/// as STL or 3MF (operator ask: "STL/3MF download from the browser /cad, for real CAD use"). v-cad owns the
/// SERIALIZERS (`stl.ts` / `threemf.ts` — flat positions+indices → bytes); this owns the download UI glue:
/// wrap the bytes in a Blob and trigger a same-page `<a download>` click. Kept out of the React component so
/// the byte-shape + filename logic is unit-testable (`download.test.ts`) without a DOM click.

import { meshToBinaryStl } from "./stl.ts";
import { meshTo3mf, type ThreeMfUnit } from "./threemf.ts";

/// The mesh shape the serializers need — flat vertex positions + triangle indices. /cad's successful
/// `MeshResult` (`{ ok: true, positions, indices, normals? }`) is a structural superset, so it satisfies
/// this directly (v-cad confirmed: no adapter needed). Declared locally so this module doesn't depend on
/// the mesh-driver's result type.
export interface DownloadableMesh {
  positions: Float32Array;
  indices: Uint32Array;
}

/// Serialize `mesh` to STL or 3MF bytes + a suggested filename. Pure — returns the bytes + MIME + name so a
/// caller (or a test) can inspect them without touching the DOM. 3MF carries a unit (default millimeter — the
/// printer-world common case for a downloaded CAD model; STL is unitless). Returns the `.stl`/`.3mf` payload.
export function encodeMesh(
  mesh: DownloadableMesh,
  format: "stl" | "3mf",
  unit: ThreeMfUnit = "millimeter",
): { bytes: Uint8Array; mime: string; filename: string } {
  if (format === "stl") {
    return { bytes: meshToBinaryStl(mesh), mime: "model/stl", filename: "cad-model.stl" };
  }
  return { bytes: meshTo3mf(mesh, unit), mime: "model/3mf", filename: "cad-model.3mf" };
}

/// Trigger a browser download of `mesh` as `format`. Builds the bytes (via `encodeMesh`), wraps them in a
/// Blob, and clicks a transient object-URL `<a download>` — the standard client-side "save file" path (no
/// server round-trip; the mesh already lives in the browser). Revokes the object URL after the click so the
/// blob is GC-able. A no-op guard on a non-browser env (SSR / test without a document) keeps it safe to call.
export function downloadMesh(
  mesh: DownloadableMesh,
  format: "stl" | "3mf",
  unit: ThreeMfUnit = "millimeter",
): void {
  if (typeof document === "undefined") return;
  const { bytes, mime, filename } = encodeMesh(mesh, format, unit);
  // Copy into a fresh ArrayBuffer so the Blob owns a standalone buffer (a Uint8Array view over wasm memory
  // could otherwise be a partial/over-long view of a shared buffer).
  const blob = new Blob([bytes.slice()], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
