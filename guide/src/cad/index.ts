/// The CAD mesh driver for the /cad browser route — v-cad's module (G3b). A TS port of the Rust cdz-cad
/// (implementation/seed/crates/cdz-cad): parse a rendered EXACT `Solid` s-expr (Rational `n/d` coords) into
/// a CSG tree, walk it into a manifold-3d solid, triangulate, and return the flat triangle buffers the /cad
/// route feeds to three.js. v-guide-infra owns the route/canvas/deps; this module is v-cad's (per the split).
///
/// The run worker produces the model's rendered value like:
///   (: (Difference (Cube (: (tuple 50/1 30/1 5/1) Vec3)) (Sphere 127/20)) Solid)
/// The exact `Solid` model (implementation/cad) uses Rational coordinates (`n/d`); this parser reads the
/// numerator/denominator and evaluates to a JS number (n/d) at the mesh leaf — the MODEL is exact, the
/// geometry kernel (manifold) is float. Full-size Cube/Cylinder. Rotate carries an exact Rational
/// Euler-degree triple (the trig runs at the manifold leaf, like Revolve); Mirror a plane normal.

import Module from "manifold-3d";
import type { ManifoldToplevel } from "manifold-3d";

/// The result of meshing a Solid: triangle buffers ready for a three.js BufferGeometry, or a typed error
/// to render (never throws). Flat shape — matches the /cad route's contract exactly.
///   - `positions`: flat vertex XYZ, 3 floats per vertex.
///   - `indices`: triangle indices into `positions` (3 per triangle).
///   - `normals`: optional flat per-vertex normals; the route computes them if omitted.
export type MeshResult =
  | { ok: true; positions: Float32Array; indices: Uint32Array; normals?: Float32Array }
  | { ok: false; error: string };

/// Default tessellation of curved primitives (sphere/cylinder/circle) + the revolve/Bézier sweep, matching
/// the Rust driver's `DEFAULT_SEGMENTS`. This is the fallback resolution when `meshFromSolid` is called
/// without an explicit segment count (the /cad preview passes its quality-slider value; the STL/3MF export
/// uses the same threaded value so what you see is what you download). A mesh hint only — the exact Rational
/// MODEL is never touched, so the same model at any resolution is the same geometry, just tessellated finer.
export const DEFAULT_SEGMENTS = 32;

/// Floor on the segment count (matches cdz-cad's `--segments` min): fewer than 3 can't close a curved loop
/// (manifold rejects it), so a slider/argument below this is clamped up rather than erroring.
export const MIN_SEGMENTS = 3;

/// The maximum Solid nesting depth (matches cdz-cad's MAX_DEPTH) — an adversarially deep input Errs cleanly
/// instead of overflowing the recursive descent.
const MAX_DEPTH = 256;

type Vec3 = [number, number, number];
type Vec2 = [number, number];

/// A 2-D path segment (mirror of cdz-cad's `PathSeg`) — absolute (`*Abs`) or relative (`*Rel`) to the
/// running cursor. A cubic Bézier carries end + two control points.
type PathSeg =
  | { k: "moveAbs"; p: Vec2 }
  | { k: "moveRel"; d: Vec2 }
  | { k: "lineAbs"; p: Vec2 }
  | { k: "lineRel"; d: Vec2 }
  | { k: "cubicAbs"; e: Vec2; c0: Vec2; c1: Vec2 }
  | { k: "cubicRel"; e: Vec2; c0: Vec2; c1: Vec2 };

/// A 2-D cross-section (mirror of cdz-cad's `Profile`) — the input an extrude/revolve lifts.
type Profile =
  | { p: "rect"; w: number; h: number }
  | { p: "circle"; r: number }
  | { p: "path"; segs: PathSeg[] };

/// The parsed CSG tree — mirror of cdz-cad's `Solid` enum, coordinates already evaluated (n/d → number).
type Solid =
  | { t: "empty" }
  | { t: "cube"; s: Vec3 }
  | { t: "sphere"; r: number }
  | { t: "cylinder"; h: number; r: number }
  | { t: "union"; a: Solid; b: Solid }
  | { t: "difference"; a: Solid; b: Solid }
  | { t: "intersection"; a: Solid; b: Solid }
  | { t: "translate"; v: Vec3; of: Solid }
  | { t: "scale"; v: Vec3; of: Solid }
  | { t: "rotate"; v: Vec3; of: Solid }
  | { t: "mirror"; v: Vec3; of: Solid }
  | { t: "extrudeLinear"; profile: Profile; height: number }
  | { t: "revolve"; profile: Profile; degrees: number }
  // An OpenSCAD-`$fn`-style tessellation-resolution OVERRIDE for a subtree (the model's `Detail(n, child)`):
  // `segments` overrides the inherited/ambient resolution when meshing `of`. A mesh HINT only — no geometry
  // changes (twin of the native cdz-cad `Solid::Detail`).
  | { t: "detail"; segments: number; of: Solid };

// ── s-expr tokenizer + recursive-descent parser (twin of cdz-cad's lib.rs) ──────────────────────────

type Tok = "(" | ")" | { atom: string };

function tokenize(text: string): Tok[] {
  // M2 native-compound render (#5112): render_value emits compound values head-first as `#tuple(…)`,
  // `#list(…)`, `#record(…)`, etc. This twin-of-cdz-cad parser was written for the legacy `(tuple …)`
  // form (a Vec3 coord renders `(tuple 50/1 30/1 5/1)` → now `#tuple(50/1 30/1 5/1)`), so normalize the
  // M2 `#head(` spelling back to `(head ` before tokenizing — nested + balanced by construction. Guarded
  // to a name+`(` so it never touches a `#"hashword"` string or `#\c` char literal.
  text = text.replace(/#([A-Za-z][\w-]*)\(/g, "($1 ");
  const toks: Tok[] = [];
  let cur = "";
  const flush = () => {
    if (cur) {
      toks.push({ atom: cur });
      cur = "";
    }
  };
  for (const c of text) {
    if (c === "(") {
      flush();
      toks.push("(");
    } else if (c === ")") {
      flush();
      toks.push(")");
    } else if (/\s/.test(c)) {
      flush();
    } else {
      cur += c;
    }
  }
  flush();
  return toks;
}

class Parser {
  pos = 0;
  depth = 0;
  toks: Tok[];
  constructor(toks: Tok[]) {
    this.toks = toks;
  }

  private bump(): Tok | undefined {
    return this.toks[this.pos++];
  }
  private isAtom(t: Tok | undefined, a?: string): t is { atom: string } {
    return typeof t === "object" && (a === undefined || t.atom === a);
  }
  private expectOpen(): void {
    if (this.bump() !== "(") throw new Error("expected `(`");
  }
  private expectClose(): void {
    if (this.bump() !== ")") throw new Error("expected `)`");
  }
  private expectAtom(): string {
    const t = this.bump();
    if (!this.isAtom(t)) throw new Error("expected an atom");
    return t.atom;
  }
  /// Is the upcoming form a `(: … …)` type annotation?
  private annotationAhead(): boolean {
    return this.toks[this.pos] === "(" && this.isAtom(this.toks[this.pos + 1], ":");
  }

  /// Parse a value at a Solid position, transparently unwrapping a `(: <value> Type>)` annotation.
  parseSolid(): Solid {
    if (++this.depth > MAX_DEPTH) {
      throw new Error(`solid nests deeper than the limit (${MAX_DEPTH})`);
    }
    let r: Solid;
    if (this.annotationAhead()) {
      this.expectOpen(); // (
      this.expectAtom(); // :
      r = this.parseSolid(); // the annotated value
      this.expectAtom(); // Type name (Solid)
      this.expectClose(); // )
    } else {
      r = this.parseNode();
    }
    this.depth--;
    return r;
  }

  private parseNode(): Solid {
    this.expectOpen();
    const head = this.expectAtom();
    let node: Solid;
    switch (head) {
      case "Empty":
        this.expectAtom(); // `unit` payload
        node = { t: "empty" };
        break;
      case "Cube":
        node = { t: "cube", s: this.parseVec3() };
        break;
      case "Sphere":
        node = { t: "sphere", r: this.parseRational() };
        break;
      case "Cylinder":
        node = { t: "cylinder", h: this.parseRational(), r: this.parseRational() };
        break;
      case "Union":
        node = { t: "union", a: this.parseSolid(), b: this.parseSolid() };
        break;
      case "Difference":
        node = { t: "difference", a: this.parseSolid(), b: this.parseSolid() };
        break;
      case "Intersection":
        node = { t: "intersection", a: this.parseSolid(), b: this.parseSolid() };
        break;
      case "Translate":
        node = { t: "translate", v: this.parseVec3(), of: this.parseSolid() };
        break;
      case "Scale":
        node = { t: "scale", v: this.parseVec3(), of: this.parseSolid() };
        break;
      case "Rotate":
        // Exact Rational Euler-degree triple; the trig runs at the manifold leaf (see toManifold).
        node = { t: "rotate", v: this.parseVec3(), of: this.parseSolid() };
        break;
      case "Mirror":
        node = { t: "mirror", v: this.parseVec3(), of: this.parseSolid() };
        break;
      case "ExtrudeLinear":
        node = { t: "extrudeLinear", profile: this.parseProfile(), height: this.parseRational() };
        break;
      case "Revolve":
        node = { t: "revolve", profile: this.parseProfile(), degrees: this.parseRational() };
        break;
      case "Detail":
        // `(Detail <count> <child>)` — a tessellation-resolution override. The count is an integer segment
        // count (floored); the child is a Solid meshed at that resolution (see toManifold).
        node = { t: "detail", segments: this.parseSegmentCount(), of: this.parseSolid() };
        break;
      default:
        throw new Error(`unknown Solid constructor \`${head}\``);
    }
    this.expectClose();
    return node;
  }

  private parseVec3(): Vec3 {
    if (this.annotationAhead()) {
      this.expectOpen(); // (
      this.expectAtom(); // :
      const v = this.parseVec3(); // inner (tuple …)
      this.expectAtom(); // Vec3
      this.expectClose(); // )
      return v;
    }
    this.expectOpen();
    if (this.expectAtom() !== "tuple") throw new Error("expected a Vec3 `(tuple …)`");
    const v: Vec3 = [this.parseRational(), this.parseRational(), this.parseRational()];
    this.expectClose();
    return v;
  }

  /// A `Vec2` — `(: (tuple x y) Vec2R)` annotation or bare `(tuple x y)` (type-name atom discarded).
  private parseVec2(): Vec2 {
    if (this.annotationAhead()) {
      this.expectOpen();
      this.expectAtom(); // :
      const v = this.parseVec2();
      this.expectAtom(); // Vec2R
      this.expectClose();
      return v;
    }
    this.expectOpen();
    if (this.expectAtom() !== "tuple") throw new Error("expected a Vec2 `(tuple …)`");
    const v: Vec2 = [this.parseRational(), this.parseRational()];
    this.expectClose();
    return v;
  }

  /// A `Profile` — `(Rect <Vec2>)` / `(Circle <r>)` / `(PathProfile <Path>)` (outer annotation discarded).
  private parseProfile(): Profile {
    if (this.annotationAhead()) {
      this.expectOpen();
      this.expectAtom(); // :
      const p = this.parseProfile();
      this.expectAtom(); // ProfileR
      this.expectClose();
      return p;
    }
    this.expectOpen();
    const head = this.expectAtom();
    let prof: Profile;
    switch (head) {
      case "Rect": {
        const [w, h] = this.parseVec2();
        prof = { p: "rect", w, h };
        break;
      }
      case "Circle":
        prof = { p: "circle", r: this.parseRational() };
        break;
      case "PathProfile":
        prof = { p: "path", segs: this.parsePath() };
        break;
      default:
        throw new Error(`unknown Profile constructor \`${head}\``);
    }
    this.expectClose();
    return prof;
  }

  /// A `Path` — `(: (list <seg…>) PathR)` or bare `(list <seg…>)` (type-name atom discarded).
  private parsePath(): PathSeg[] {
    if (this.annotationAhead()) {
      this.expectOpen();
      this.expectAtom(); // :
      const segs = this.parsePath();
      this.expectAtom(); // PathR
      this.expectClose();
      return segs;
    }
    this.expectOpen();
    if (this.expectAtom() !== "list") throw new Error("expected a Path `(list …)`");
    const segs: PathSeg[] = [];
    while (this.toks[this.pos] === "(") segs.push(this.parsePathSeg());
    this.expectClose();
    return segs;
  }

  /// One `PathSeg` — `(MoveToAbs <Vec2>)` / `LineToRel` / `(CubicToAbs <Vec2> <Vec2> <Vec2>)` etc.
  private parsePathSeg(): PathSeg {
    this.expectOpen();
    const head = this.expectAtom();
    let seg: PathSeg;
    switch (head) {
      case "MoveToAbs":
        seg = { k: "moveAbs", p: this.parseVec2() };
        break;
      case "MoveToRel":
        seg = { k: "moveRel", d: this.parseVec2() };
        break;
      case "LineToAbs":
        seg = { k: "lineAbs", p: this.parseVec2() };
        break;
      case "LineToRel":
        seg = { k: "lineRel", d: this.parseVec2() };
        break;
      case "CubicToAbs":
        seg = { k: "cubicAbs", e: this.parseVec2(), c0: this.parseVec2(), c1: this.parseVec2() };
        break;
      case "CubicToRel":
        seg = { k: "cubicRel", e: this.parseVec2(), c0: this.parseVec2(), c1: this.parseVec2() };
        break;
      default:
        throw new Error(`unknown PathSeg constructor \`${head}\``);
    }
    this.expectClose();
    return seg;
  }

  /// A `Detail` SEGMENT-COUNT leaf — the tessellation resolution the override node carries. The model renders
  /// it as an integer (`Int`), so it arrives as a bare integer atom; a rational/float is accepted defensively
  /// and FLOORED (a count has no fractional meaning). A NaN/±inf count is malformed here (a resolution is never
  /// NaN, unlike a coordinate) → a throw the caller turns into a typed error. The value is clamped to a
  /// closable loop at mesh time (`MIN_SEGMENTS`), not here — the tree records what the model asked for.
  private parseSegmentCount(): number {
    const n = this.parseRational();
    if (!Number.isFinite(n)) throw new Error("Detail segment count must be a finite integer");
    return Math.floor(n);
  }

  /// A RATIONAL number leaf `n/d` → the JS number n/d (division at the leaf; the model stays exact). A bare
  /// integer or float is accepted defensively; a `nan` atom maps to NaN; a zero denominator errs.
  private parseRational(): number {
    const a = this.expectAtom();
    // Cadenza renders a NaN float as the LOWERCASE atom `nan` (the compiler's `Prim::FloatNan`), so match
    // case-insensitively — an uppercase-only check missed a real `nan` value and threw. This MUST stay before
    // the `Number(a)` fallthrough below: `Number("nan")` is NaN, which `Number.isFinite` then rejects → throw.
    if (a.toLowerCase() === "nan") return NaN;
    const slash = a.indexOf("/");
    if (slash >= 0) {
      const n = Number(a.slice(0, slash));
      const d = Number(a.slice(slash + 1));
      if (!Number.isFinite(n) || !Number.isFinite(d)) throw new Error(`bad rational \`${a}\``);
      if (d === 0) throw new Error(`rational \`${a}\` has a zero denominator`);
      return n / d;
    }
    const x = Number(a);
    if (!Number.isFinite(x)) throw new Error(`expected a rational \`n/d\`, found \`${a}\``);
    return x;
  }
}

/// Parse a rendered `Solid` s-expression into a CSG tree (throws on a malformed form).
function parseSolid(text: string): Solid {
  const p = new Parser(tokenize(text.trim()));
  const s = p.parseSolid();
  if (p.pos !== p.toks.length) throw new Error("trailing tokens after the solid");
  return s;
}

// ── mesh walk: Solid → manifold-3d Manifold (twin of cdz-cad's mesh.rs) ─────────────────────────────

type ManifoldStatic = ManifoldToplevel["Manifold"];
type ManifoldObj = InstanceType<ManifoldStatic>;
type CrossSectionStatic = ManifoldToplevel["CrossSection"];
type CrossSectionObj = InstanceType<CrossSectionStatic>;

/// Build a 2-D `CrossSection` from a [`Profile`] — the twin of cdz-cad's `profile_to_cross_section`. `rect`/
/// `circle` map onto manifold's centred `square`/`circle`; a `path` is sampled to a polygon (`samplePath`)
/// then built as a single-contour CrossSection.
function profileToCrossSection(CS: CrossSectionStatic, p: Profile, seg: number): CrossSectionObj {
  switch (p.p) {
    case "rect":
      return CS.square([p.w, p.h], true);
    case "circle":
      return CS.circle(p.r, seg);
    case "path":
      return new CS([samplePath(p.segs, seg)]);
  }
}

/// Sample a path's segments to a flat polygon (`[x,y]` points), walking the cursor — the twin of cdz-cad's
/// `sample_path`. A line/move contributes its (absolute) endpoint; a cubic Bézier is sampled at `SEGMENTS`
/// points via the Bernstein form; relative segments offset the running cursor.
function samplePath(segs: PathSeg[], seg: number): Vec2[] {
  const pts: Vec2[] = [];
  let cur: Vec2 = [0, 0];
  const add = (a: Vec2, b: Vec2): Vec2 => [a[0] + b[0], a[1] + b[1]];
  for (const s of segs) {
    switch (s.k) {
      case "moveAbs":
      case "lineAbs":
        cur = s.p;
        pts.push(cur);
        break;
      case "moveRel":
      case "lineRel":
        cur = add(cur, s.d);
        pts.push(cur);
        break;
      case "cubicAbs":
        sampleCubic(pts, cur, s.c0, s.c1, s.e, seg);
        cur = s.e;
        break;
      case "cubicRel": {
        const e = add(cur, s.e);
        sampleCubic(pts, cur, add(cur, s.c0), add(cur, s.c1), e, seg);
        cur = e;
        break;
      }
    }
  }
  return pts;
}

/// Push `seg` sample points of the cubic Bézier `p0→p3` (controls `p1`,`p2`) for `t` in (0,1] — `p0` is
/// assumed already emitted by the prior segment, so we sample the interior + endpoint (Bernstein form).
function sampleCubic(pts: Vec2[], p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, seg: number): void {
  const n = Math.max(1, seg);
  for (let i = 1; i <= n; i++) {
    const t = i / n;
    const u = 1 - t;
    const b0 = u * u * u;
    const b1 = 3 * u * u * t;
    const b2 = 3 * u * t * t;
    const b3 = t * t * t;
    pts.push([
      b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
      b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
    ]);
  }
}

function toManifold(M: ManifoldStatic, CS: CrossSectionStatic, s: Solid, seg: number): ManifoldObj {
  switch (s.t) {
    case "empty":
      // An empty solid → manifold's canonical EMPTY (no geometry), matching the Rust cdz-cad driver's
      // `Manifold::empty()`. 🪤 A zero-size cube (`M.cube([0,0,0])`) is NOT a safe empty here: it reports
      // isEmpty/0-tri in isolation but ANNIHILATES a boolean — `cube([0,0,0]).add(realCube)` yields 0 tris,
      // zeroing the whole model (this blanked the snowflake, whose 6-fold fold starts from an Empty base:
      // `Union(Empty, body)` → nothing). The encapsulated API has no bare empty ctor, but `M.union([])`
      // (union of no manifolds) IS a proper empty that COMPOSES: `M.union([]).add(realCube)` === the cube.
      return M.union([]);
    case "cube":
      return M.cube(s.s, true); // FULL size, centred (matches Cube semantics)
    case "sphere":
      return M.sphere(s.r, seg);
    case "cylinder":
      return M.cylinder(s.h, s.r, s.r, seg, true); // constant-radius, FULL height, centred
    case "union":
      return toManifold(M, CS, s.a, seg).add(toManifold(M, CS, s.b, seg));
    case "difference":
      return toManifold(M, CS, s.a, seg).subtract(toManifold(M, CS, s.b, seg));
    case "intersection":
      return toManifold(M, CS, s.a, seg).intersect(toManifold(M, CS, s.b, seg));
    case "translate":
      return toManifold(M, CS, s.of, seg).translate(s.v);
    case "scale":
      return toManifold(M, CS, s.of, seg).scale(s.v);
    case "rotate":
      // manifold's rotate takes DEGREES per axis (Vec3) — matching the Rotate(euler-degrees, …) model +
      // the native cdz-cad driver. The exact Rational degrees were evaluated to number at parse.
      return toManifold(M, CS, s.of, seg).rotate(s.v);
    case "mirror":
      // reflect across the plane through the origin with normal `s.v` — matching Mirror(normal, …) + native.
      return toManifold(M, CS, s.of, seg).mirror(s.v);
    case "extrudeLinear":
      // Lift the profile straight up +z by `height`, then centre it in z (extrude runs 0..height, shift down
      // height/2 — matches the origin-centred primitives + the native cdz-cad driver). 🪤 NOT the
      // extrude(h, …, center=true) flag: manifold-3d's built-in centering INVERTS the winding of some faces
      // (verified: center=true → 8 outward + 4 INWARD-winding tris on a square prism, so those faces render
      // dark/one-sided — the operator's "extrudes one side, leaves others flat"). extrude(h) + translate is
      // consistently outward-wound (12 outward, 0 inward), like a cube.
      return profileToCrossSection(CS, s.profile, seg)
        .extrude(s.height)
        .translate(0, 0, -s.height / 2);
    case "revolve":
      // sweep the profile about the y-axis by `degrees` (`seg` = sweep tessellation).
      return M.revolve(profileToCrossSection(CS, s.profile, seg), seg, s.degrees);
    case "detail":
      // An OpenSCAD-`$fn`-style resolution override: mesh the child with the node's LOCAL segment count
      // instead of the inherited `seg`, clamped to a closable loop. A deeper `detail` overrides again
      // (dynamic scoping — innermost wins); geometry outside any `detail` keeps the ambient `seg`. A mesh
      // hint only, so the child's shape is unchanged. Twin of the native driver's `Solid::Detail`.
      return toManifold(M, CS, s.of, Math.max(MIN_SEGMENTS, s.segments));
  }
}

// The manifold-3d WASM module is initialized once (async) and memoized — v-cad owns this init; the whole
// module is lazy-loaded behind the /cad route so the wasm never touches the guide's critical path.
let toplevelPromise: Promise<ManifoldToplevel> | null = null;
async function manifoldStatics(): Promise<{ M: ManifoldStatic; CS: CrossSectionStatic }> {
  if (!toplevelPromise) {
    toplevelPromise = Module().then((tl) => {
      tl.setup();
      return tl;
    });
  }
  const tl = await toplevelPromise;
  return { M: tl.Manifold, CS: tl.CrossSection };
}

/// Mesh a rendered `Solid` value into triangle buffers (never throws — a typed error on parse/mesh failure).
/// `segments` is the tessellation resolution for every curved leaf (sphere/cylinder/circle) + the revolve /
/// cubic-Bézier sweep — the OpenSCAD-`$fn`-style quality knob the /cad slider drives. It CASCADES: one value
/// threads through the whole mesh walk to every curved primitive, so raising it refines the entire model at
/// once. It is a MESH hint only — the exact Rational model is unchanged, so the same model at 8 vs 128
/// segments is the same geometry, only tessellated coarser/finer. Defaults to `DEFAULT_SEGMENTS` (32, the
/// native driver's default) and is clamped up to `MIN_SEGMENTS` (a curved loop needs ≥3 sides to close).
export async function meshFromSolid(
  solidText: string,
  segments: number = DEFAULT_SEGMENTS,
): Promise<MeshResult> {
  // Clamp to a closable loop and an integer count; a non-finite/NaN slider value falls back to the default
  // rather than poisoning manifold's tessellation.
  const seg = Number.isFinite(segments) ? Math.max(MIN_SEGMENTS, Math.floor(segments)) : DEFAULT_SEGMENTS;
  let tree: Solid;
  try {
    tree = parseSolid(solidText);
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  try {
    const { M, CS } = await manifoldStatics();
    const gl = toManifold(M, CS, tree, seg).getMesh();
    // MeshGL packs vertex properties as [x, y, z, …extra] per vertex, stride = numProp. With numProp === 3
    // it IS the flat position array; otherwise strip out the first 3 (position) props per vertex.
    const numProp = gl.numProp;
    const positions =
      numProp === 3
        ? gl.vertProperties
        : Float32Array.from(
            { length: (gl.vertProperties.length / numProp) * 3 },
            (_, i) => gl.vertProperties[Math.floor(i / 3) * numProp + (i % 3)],
          );
    return { ok: true, positions, indices: gl.triVerts };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}
