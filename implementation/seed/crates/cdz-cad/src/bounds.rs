//! bounds.rs — the axis-aligned bounding box of a [`crate::Solid`] (GH #400).
//!
//! The design DEFERRED a `bounding-box` fold in the Cadenza `Solid` library (a recursive Float64 min/max
//! fold trips an unimplemented-scalar-float-`==` backend gap — see the vertical's findings). But the NATIVE
//! driver doesn't need the language-side fold: `manifold` already computes an exact, transform-and-boolean-
//! aware AABB from the evaluated mesh (rotations included — no trig needed on our side). So the driver
//! reports extents directly from the geometry kernel. This is genuinely useful: "does my print fit the bed?"
//! / "how big is this model?" — a `cdz-cad --bounds` answer a printer workflow wants.

use crate::{to_manifold_with_segments, Solid, DEFAULT_SEGMENTS};

/// An axis-aligned bounding box: the min and max corners (world coordinates). `None` for an empty solid
/// (no geometry to bound).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    /// The per-axis size `[dx, dy, dz]` (max − min).
    pub fn dimensions(&self) -> [f64; 3] {
        [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ]
    }

    /// The center point `[x, y, z]`.
    pub fn center(&self) -> [f64; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }

    /// Whether this box fits inside an axis-aligned build volume of size `[x, y, z]` (a print-bed check).
    /// Compares the box's DIMENSIONS (orientation-as-modeled) against the volume, with a small tolerance.
    pub fn fits_within(&self, volume: [f64; 3]) -> bool {
        let d = self.dimensions();
        let eps = 1e-9;
        d[0] <= volume[0] + eps && d[1] <= volume[1] + eps && d[2] <= volume[2] + eps
    }
}

/// Compute the bounding box of `solid` at the default tessellation. `None` if the solid is empty (manifold
/// reports no bounds for empty geometry).
pub fn bounds(solid: &Solid) -> Option<Bounds> {
    bounds_with_segments(solid, DEFAULT_SEGMENTS)
}

/// Compute the bounding box at a given tessellation. (Tessellation barely affects a bound — a coarser
/// sphere's hull is slightly inside the true sphere — but we thread it through for consistency with meshing.)
pub fn bounds_with_segments(solid: &Solid, segments: i32) -> Option<Bounds> {
    let m = to_manifold_with_segments(solid, segments);
    m.bounding_box().map(|bb| Bounds {
        min: bb.min(),
        max: bb.max(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_solid;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_centered_cube_has_symmetric_bounds() {
        // a 2×2×2 cube centred at the origin spans [-1, 1] on each axis.
        let b =
            bounds(&parse_solid("(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)").unwrap()).unwrap();
        assert!(
            approx(b.min[0], -1.0) && approx(b.max[0], 1.0),
            "x spans [-1,1]"
        );
        let d = b.dimensions();
        assert!(
            approx(d[0], 2.0) && approx(d[1], 2.0) && approx(d[2], 2.0),
            "2×2×2"
        );
        let c = b.center();
        assert!(
            approx(c[0], 0.0) && approx(c[1], 0.0) && approx(c[2], 0.0),
            "centred at origin"
        );
    }

    #[test]
    fn a_translate_shifts_the_bounds() {
        // translate a unit-ish sphere out along +x → its center x is the translation.
        let b = bounds(
            &parse_solid("(: (Translate (: (tuple 5.0 0.0 0.0) Vec3) (Sphere 1.0)) Solid)")
                .unwrap(),
        )
        .unwrap();
        assert!(approx(b.center()[0], 5.0), "center x follows the translate");
        // radius-1 sphere → x spans roughly [4, 6].
        assert!(
            b.min[0] > 3.9 && b.max[0] < 6.1,
            "sphere-1 at x=5 spans ~[4,6]"
        );
    }

    #[test]
    fn a_union_bounds_encloses_both() {
        // two unit cubes 10 apart on x → the union's x-extent spans both.
        let b = bounds(
            &parse_solid(
                "(: (Union (Cube (: (tuple 1.0 1.0 1.0) Vec3)) (Translate (: (tuple 10.0 0.0 0.0) Vec3) (Cube (: (tuple 1.0 1.0 1.0) Vec3)))) Solid)",
            )
            .unwrap(),
        )
        .unwrap();
        // leftmost cube min x = -0.5, rightmost cube max x = 10.5.
        assert!(approx(b.min[0], -0.5), "encloses the left cube");
        assert!(approx(b.max[0], 10.5), "encloses the right cube");
    }

    #[test]
    fn empty_has_no_bounds() {
        assert_eq!(
            bounds(&parse_solid("(: (Empty unit) Solid)").unwrap()),
            None
        );
    }

    #[test]
    fn a_rotated_cubes_tight_mesh_bounds_stay_inside_the_models_conservative_box() {
        // CROSS-IMPLEMENTATION SOUNDNESS: the model's `bounding-box` (exact.cdz) returns a CONSERVATIVE
        // symmetric box for a Rotate — the L1 corner-radius R = mx+my+mz — because an exact tight rotated box
        // needs trig. The driver, by contrast, reports the TIGHT AABB from the evaluated mesh. The contract
        // that makes the model's box safe (e.g. for a print-bed check) is: the tight mesh bounds must ALWAYS
        // be ENCLOSED by the conservative model box — never escape it. If the model box ever under-approximated
        // (the reviewer-flagged `max(mx,my,mz)` bug fixed by the L1 radius), a rotated part would exceed its
        // reported bounds. Pin it end-to-end: a 2×2×2 cube ([-1,1]^3, per-axis |coord|=1 → R=1+1+1=3, so the
        // model box is [-3,3]^3) rotated 45° about z. The tight mesh footprint is the 45° diamond: x/y half-
        // extent = √2 ≈ 1.4142 (the corner (1,1) rotates onto an axis at distance √2), z unchanged at ±1.
        let b = bounds(
            &parse_solid(
                "(: (Rotate (: (tuple 0/1 0/1 45/1) Vec3) (Cube (: (tuple 2/1 2/1 2/1) Vec3))) Solid)",
            )
            .unwrap(),
        )
        .unwrap();
        // (a) the tight bounds are the expected 45° diamond: ±√2 in x/y, ±1 in z.
        let root2 = 2.0_f64.sqrt();
        assert!(
            approx(b.max[0], root2) && approx(b.min[0], -root2),
            "tight x half-extent is √2 (the rotated corner)"
        );
        assert!(
            approx(b.max[1], root2) && approx(b.min[1], -root2),
            "tight y half-extent is √2"
        );
        assert!(
            approx(b.max[2], 1.0) && approx(b.min[2], -1.0),
            "z unchanged at ±1"
        );
        // (b) the SOUNDNESS contract: the tight mesh bounds are strictly inside the model's conservative
        // [-3,3]^3 box (R = 3). A margin here confirms the L1 radius over-approximates, never under.
        let conservative = 3.0;
        assert!(
            b.min.iter().all(|&c| c >= -conservative) && b.max.iter().all(|&c| c <= conservative),
            "the tight rotated-cube bounds must stay inside the model's conservative [-3,3]^3 box"
        );
    }

    #[test]
    fn fits_within_a_build_volume() {
        let b =
            bounds(&parse_solid("(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)").unwrap()).unwrap();
        assert!(b.fits_within([3.0, 3.0, 3.0]), "a 2-cube fits a 3-bed");
        assert!(!b.fits_within([1.0, 3.0, 3.0]), "…but not a 1-wide bed");
    }
}
