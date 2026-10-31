//! `mass_properties` of a sliver: the common part of a cone set ten
//! tolerances deep into a block under a B-spline roof (the corpus case
//! `near/cone-spline/002`). Its face on the roof is a strip 1e-5 wide and
//! 1.5 long, whose sums agree from one doubling to the next to the
//! rounding of its coordinates over its width and no further: judged by
//! the rounding over its length they never settled, and measuring the
//! body took the seven doublings of a boundary of spline edges on a
//! spline surface, more than ten minutes. It lives here because `topo`
//! cannot see the kernel.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::quadrature::gauss_legendre;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

/// What FreeCAD 1.1.0 (OCCT 7.8.1) measures of the common part: the
/// reference of the corpus case, a kernel that shares nothing with ours.
const REFERENCE_VOLUME: f64 = 0.000025896168424455972;
const REFERENCE_AREA: f64 = 5.179299125135435;

/// The common volume integrated directly, independent of both kernels.
/// The sliver is where the cone (base z = 0, apex 1.625 up, radius 1.5
/// about (2, 1.625)) reaches the last 1e-5 of the domed block in z (its
/// wall at z = 1e-5). On a line along y at (x, z) the cone holds
/// |y − 1.625| ≤ √(r(z)² − (x − 2)²), r(z) = 1.5 (1 − z / 1.625), and the
/// block holds 1.0125 ≤ y ≤ 1.0125 + roof(u, v), the roof the bicubic
/// Bernstein polynomial of the test kit's heights at u = (2.75 − x) / 1.5,
/// v = (z + 1.99999) / 2: the length both hold is closed form. It is
/// summed by Gauss–Legendre, 6 nodes over the slab in z and 8 nodes on
/// each of `panels` pieces of x ∈ [1.25, 2.75]. With 2000, 8000 and 32000
/// pieces it gives 2.58961684956e-5, 2.58961685049e-5 and 2.58961685044e-5
/// (FreeCAD: 2.58961684245e-5).
fn integrated_volume(panels: u32) -> f64 {
    let height = |i: usize, j: usize| 1.75 * (1.0 + 0.15 * (((i * 7 + j * 3) % 5) as f64 - 2.0));
    let bernstein = |t: f64| {
        let r = 1.0 - t;
        [r * r * r, 3.0 * r * r * t, 3.0 * r * t * t, t * t * t]
    };
    let roof = |u: f64, v: f64| {
        let (bu, bv) = (bernstein(u), bernstein(v));
        let mut sum = 0.0;
        for (i, wu) in bu.iter().enumerate() {
            for (j, wv) in bv.iter().enumerate() {
                sum += wu * wv * height(i, j);
            }
        }
        sum
    };
    let length = |x: f64, z: f64| {
        let r = 1.5 * (1.0 - z / 1.625);
        let d = r * r - (x - 2.0) * (x - 2.0);
        if d <= 0.0 {
            return 0.0;
        }
        let s = d.sqrt();
        let (u, v) = ((2.75 - x) / 1.5, (z + 1.99999) / 2.0);
        let low = 1.0125_f64.max(1.625 - s);
        let high = (1.0125 + roof(u, v)).min(1.625 + s);
        (high - low).max(0.0)
    };
    let (in_z, in_x) = (gauss_legendre(6), gauss_legendre(8));
    let (z0, z1, x0, x1) = (0.0, 1e-5, 1.25, 2.75);
    let piece = (x1 - x0) / f64::from(panels);
    let mut volume = 0.0;
    for &(tz, wz) in &in_z {
        let z = z0 + (z1 - z0) * 0.5 * (tz + 1.0);
        let mut area = 0.0;
        for k in 0..panels {
            let middle = x0 + piece * (f64::from(k) + 0.5);
            for &(tx, wx) in &in_x {
                area += wx * 0.5 * piece * length(middle + 0.5 * piece * tx, z);
            }
        }
        volume += wz * 0.5 * (z1 - z0) * area;
    }
    volume
}

/// How far a volume measured on `body` can be from the volume its faces
/// stand for, given that its edges are only within the tolerance of the
/// faces they bound. `mass_properties` sums `(p − c)·n / 3` over each
/// face as that face's own boundary (the feet of its edges on its
/// surface) bounds it, `c` the middle of the body's box. Where an edge is
/// `e` off the surfaces of its two faces, their two boundaries along it
/// are up to `2e` apart, and the faces leave a ribbon that wide and as
/// long as the edge open (or covered twice); the sum misses that ribbon,
/// at most `(1/3) · |p − c| · 2e · L` with `|p − c|` at most half the box's
/// diagonal `D`. Summed over the edges: `Σ e L D / 3`. (The body's own
/// geometry, faces trimmed up to `e` off where they should end, moves
/// the volume by `e L` times the slab's thickness, a hundred thousand
/// times less here.) `e` and `L` are measured on 64 places of each edge.
fn closure_bound(body: &craft3d_topo::Body) -> f64 {
    let corners = body.bbox().ok().and_then(|b| b.corners());
    let diagonal = corners.map_or(0.0, |(lo, hi)| lo.distance(hi));
    open_ribbons(body) * diagonal / 6.0
}

/// `Σ 2 e L` over the edges of `body`: the area of the ribbons its faces
/// can leave open where an edge is `e` off the surfaces of its two faces
/// (each face's boundary is the edge's foot on its own surface), `L` the
/// edge's length; how far its measured area can be from the area its
/// faces stand for. `e` and `L` are measured on 64 places of each edge.
fn open_ribbons(body: &craft3d_topo::Body) -> f64 {
    let mut ribbons = 0.0;
    for (index, edge) in body.edges.iter().enumerate() {
        let surfaces: Vec<&craft3d_geom::Surface> = body
            .faces
            .iter()
            .filter(|face| {
                face.loops.iter().any(|&l| {
                    body.loop_(l)
                        .is_some_and(|lp| lp.coedges.iter().any(|c| c.edge.0 as usize == index))
                })
            })
            .map(|face| &face.surface)
            .collect();
        let (t0, t1) = edge.range;
        let points: Vec<Vec3> = (0..=64)
            .map(|i| edge.curve.eval(t0 + (t1 - t0) * f64::from(i) / 64.0))
            .collect();
        let length: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        let mut off = 0.0_f64;
        for &p in &points {
            for surface in &surfaces {
                let (u, v) = surface.project(p).unwrap();
                off = off.max(surface.eval(u, v).distance(p));
            }
        }
        ribbons += 2.0 * off * length;
    }
    ribbons
}

/// The sliver is measured, in seconds, and to what the modelling
/// tolerance leaves of it. Its spline edges lie within the tolerance of
/// the faces they bound and not on them, so its faces close only up to
/// that, and its volume, a slab 1e-5 thick, is held to the volume
/// integrated directly ([`integrated_volume`]) within what those edges
/// leave open ([`closure_bound`], derived there and measured on the
/// result), and so to FreeCAD's. Measured: 2.2e-9 from both, of a bound
/// of 7.3e-7. (Until 2026-10-16 it was held to FreeCAD's within
/// 2e-9, a measured 6.6e-10 with the curves the box of the two bodies
/// gave; marched in the box of each pair of faces they are other fits,
/// within the same tolerance, and the slab moved by 2.2e-9: an
/// expectation changed, as `docs/testing.md` › *Changing an expectation*
/// asks, with the direct integral as the independent pin.) The volume is
/// also held to what the union (which has no thin face) and the two
/// solids (whose volumes are closed forms) imply, volume and area being
/// additive over a union and what it has in common.
#[test]
fn a_sliver_ten_tolerances_thick_is_measured() {
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let (radius, height) = (1.5, 1.625);
    let cone = primitives::cone(
        &Frame::new(Vec3::new(2.0, 1.625, 0.0), Vec3::Z, Vec3::X).unwrap(),
        radius,
        height,
    )
    .unwrap();
    let frame = Frame::new(
        Vec3::new(2.75, 1.0125000000000002, -1.99999),
        Vec3::Y,
        -Vec3::X,
    )
    .unwrap();
    let dome = domed_block(&frame, 1.5, 2.0, 1.75, true).unwrap();
    let cone_volume = PI * radius * radius * height / 3.0;
    let cone_area = PI * radius * (radius + radius.phypot(height));

    let made = |op: BooleanOp| {
        let body = boolean(&cone, &dome.body, op, &tol).unwrap().body;
        let defects = check(&body, &tol);
        assert!(defects.is_empty(), "{op:?}: {defects:?}");
        body
    };
    let measure =
        |body: &craft3d_topo::Body| mass_properties(body).unwrap_or_else(|error| panic!("{error}"));
    let union_body = made(BooleanOp::Union);
    let union = measure(&union_body);
    let sliver = made(BooleanOp::Intersect);
    let common = measure(&sliver);
    let rest = measure(&made(BooleanOp::Subtract));

    // The direct integral agrees with itself at a quarter of the pieces
    // (to 1e-14) and with FreeCAD (to 1e-13).
    let integrated = integrated_volume(8000);
    assert!((integrated - integrated_volume(2000)).abs() < 1e-14);
    assert!((integrated - REFERENCE_VOLUME).abs() < 1e-13);
    let bound = closure_bound(&sliver);
    assert!(bound > 0.0 && bound < 1e-5, "bound {bound:e}");
    assert!(
        (common.volume - integrated).abs() <= bound,
        "volume {:e}, integrated {integrated:e}, bound {bound:e}",
        common.volume
    );
    assert!(
        (common.volume - REFERENCE_VOLUME).abs() <= bound,
        "volume {:e}, FreeCAD {REFERENCE_VOLUME:e}, bound {bound:e}",
        common.volume
    );
    // Measured: 9.4e-8 from the reference.
    assert!(
        (common.area - REFERENCE_AREA).abs() < 2e-7,
        "area {}",
        common.area
    );
    // Measured: 7.0e-8 and 6.8e-8, the tolerance of the curves of the
    // union, which are long.
    let implied = cone_volume + dome.volume - union.volume;
    assert!(
        (common.volume - implied).abs() < 2e-7,
        "volume {:e}, the union implies {implied:e}",
        common.volume
    );
    // (Until 2026-10-16 within 2e-7, measured 6.8e-8; with the fits of
    // the box of each pair of faces 2.3e-7: both are what the edges of the
    // union and of the sliver leave open, `open_ribbons`, measured on
    // them. An expectation changed as for the volume above.)
    let implied = cone_area + dome.area - union.area;
    let ribbons = open_ribbons(&union_body) + open_ribbons(&sliver);
    assert!(ribbons > 0.0 && ribbons < 1e-4, "ribbons {ribbons:e}");
    assert!(
        (common.area - implied).abs() <= ribbons,
        "area {}, the union implies {implied}",
        common.area
    );
    // The cone less the sliver has the sliver's thin faces the other way
    // around. The two differ by 1.4e-10, measured: each volume is taken
    // about the middle of its own body, and faces that close only to the
    // modelling tolerance give a volume that depends on that point. (Both
    // taken about the origin they differed by 4.5e-15, having the same
    // error; that was agreement with itself. The sliver's volume was 2.7e-9
    // from the reference then and is 6.6e-10 from it now.)
    let implied = cone_volume - rest.volume;
    assert!(
        (common.volume - implied).abs() < 1e-9,
        "volume {:e}, the rest implies {implied:e}",
        common.volume
    );
    // A slab lies halfway up: its centre is at half its thickness.
    let centroid = common.centroid.unwrap();
    assert!((centroid.z - 5e-6).abs() < 1e-8, "centroid {centroid:?}");
}

/// A small solid far from the origin of the model is measured as well as
/// the same solid at the origin. The volume is a sum over the faces of
/// their area times their distance from a point, of which only the solid
/// itself is left; taken about the origin, a solid of 1e-5 at a distance
/// of 6 leaves a part in 6e5 of every term, and a volume good to 1e-9 at
/// the origin was good to 1e-5 there. (Until 2026-10-13. It is taken
/// about the middle of the body now.)
#[test]
fn a_small_solid_far_from_the_origin_is_measured_as_well_as_at_it() {
    let place = Frame::new(
        Vec3::new(2.0, 3.0, 5.0),
        Vec3::new(0.3, -0.5, 0.8),
        Vec3::new(0.9, 0.4, -0.1),
    )
    .unwrap();
    for size in [1e-5, 1e-3, 1.0] {
        for solid in craft3d_testkit::solids::all(&place, size).unwrap() {
            let m = mass_properties(&solid.body).unwrap();
            let off = (m.volume / solid.volume - 1.0).abs();
            assert!(
                off < 1e-9,
                "{} of size {size}: volume off by {off:e}",
                solid.name
            );
            let off = (m.area / solid.area - 1.0).abs();
            assert!(
                off < 1e-9,
                "{} of size {size}: area off by {off:e}",
                solid.name
            );
            let off = m.centroid.unwrap().distance(solid.centroid) / size;
            assert!(
                off < 1e-8,
                "{} of size {size}: centroid off by {off:e} of its size",
                solid.name
            );
        }
    }
}

/// The sliver and the cone it was cut from are meshed at any tolerance,
/// also one ten thousand times the sliver's thickness. Its face on the
/// roof is a strip 1e-5 wide along a border of the patch: straight and
/// 5e-6 apart in the patch's parameters, where nothing crosses, and
/// curved in space, where a chord of one side passes the other side by
/// 2.6e-4 at a tolerance of 1e-2. The triangles between the two then
/// faced backwards, cutting them cured nothing, and the tessellator gave
/// up as "needs about 2050 segments" at every tolerance but 1e-5 (until
/// 2026-10-13). A chord now also asks to be cut when another part of the
/// boundary is nearer to it in space than four times its own bulge.
#[test]
fn a_sliver_is_meshed_at_tolerances_far_above_its_thickness() {
    use craft3d_tess::{TessParams, tessellate_body};
    let tol = Tolerance::new(1e-6, 1e-9).unwrap();
    let cone = primitives::cone(
        &Frame::new(Vec3::new(2.0, 1.625, 0.0), Vec3::Z, Vec3::X).unwrap(),
        1.5,
        1.625,
    )
    .unwrap();
    let frame = Frame::new(
        Vec3::new(2.75, 1.0125000000000002, -1.99999),
        Vec3::Y,
        -Vec3::X,
    )
    .unwrap();
    for nurbs in [false, true] {
        let dome = domed_block(&frame, 1.5, 2.0, 1.75, nurbs).unwrap();
        for op in [BooleanOp::Intersect, BooleanOp::Subtract] {
            let body = boolean(&cone, &dome.body, op, &tol).unwrap().body;
            let mass = mass_properties(&body).unwrap();
            for chord in [1e-1, 1e-2, 1e-3, 1e-4, 1e-5] {
                let params = TessParams {
                    chordal_tolerance: chord,
                    ..TessParams::default()
                };
                let mesh = tessellate_body(&body, &params)
                    .unwrap_or_else(|e| panic!("{op:?} nurbs {nurbs} at {chord:e}: {e}"));
                let welded = mesh.weld().unwrap();
                assert!(welded.is_closed(), "{op:?} nurbs {nurbs} at {chord:e}");
                // A mesh within `chord` of the faces encloses their
                // volume to within their area times that.
                let off = (welded.volume() - mass.volume).abs();
                assert!(
                    off <= chord * mass.area,
                    "{op:?} nurbs {nurbs} at {chord:e}: volume off by {off:e}"
                );
            }
        }
    }
}
