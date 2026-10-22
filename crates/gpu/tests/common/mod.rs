//! Shared by the integration tests: adapters, scenes, and the comparison of
//! a GPU frame with the CPU renderer's (oracle 7 in docs/testing.md).
//!
//! # The tolerances, and where they come from
//!
//! The two renderers run the same algorithm on the same samples; they differ
//! in arithmetic. The CPU renderer works in `f64` on exact vertex positions.
//! The GPU works in `f32` and its rasteriser first snaps every projected
//! vertex to a grid of `2^-8` pixel (eight sub-pixel bits: what every desktop
//! driver and lavapipe have; Vulkan's `subPixelPrecisionBits`). Everything
//! below follows from those two facts.
//!
//! **The band.** Snapping moves a triangle's edge by at most one grid step,
//! so the two renderers can give a sample to different triangles only when
//! it lies within `2^-8` pixel of an edge. Between two triangles of one
//! smooth face that changes nothing visible. It matters where the surface
//! under the sample changes: at *feature edges* (the border of a face, an
//! edge without a second triangle, a silhouette edge whose two triangles
//! face opposite ways on screen) and where two surfaces pass through each
//! other, which shows as a change of face id in the CPU frame. The band is
//! every pixel within one pixel (its 3 × 3 neighbourhood) of a projected
//! feature edge or of a change of id in the CPU frame. It is computed from
//! the mesh, the camera and the CPU frame only, never from the GPU frame.
//!
//! **Face ids** outside the band: equal, no tolerance.
//!
//! **Inside the band** the renderers may disagree, but only on samples within
//! `2^-8` pixel of an edge. A line that crosses a pixel passes within `2^-8`
//! pixel of its centre with probability `2 · 2^-8 = 1/128`. That is the
//! limit, [`BAND_MISMATCH`], as a fraction of the band's pixels: the rate if
//! an edge ran through every pixel of the band, where in fact the band is
//! three pixels wide around each edge (so about a third of it is expected
//! from a single edge, and the rest is room for the edges that crowd
//! together at a pole or a corner). A systematic error is far above it: a
//! picture shifted by one pixel changes about a third of the band.
//!
//! **Distance** outside the band: `|gpu - cpu| <= 2^-20 · reach + 2^-7 · G`.
//! `reach` is the largest distance the vertex shader subtracts (eye to mesh
//! centre plus the mesh diagonal): one `f32` rounding is `2^-24` relative,
//! and the path from position to stored distance (subtract, three products,
//! two sums, the rasteriser's interpolation, each on numbers up to `reach`)
//! is bounded by sixteen of them. `G` is the largest difference between the
//! pixel's CPU distance and its eight neighbours', which bounds the gradient
//! of the distance per pixel; a vertex moved by one grid step (`2^-8` pixel)
//! changes the interpolated value by at most the gradient times that step,
//! doubled because the triangle's vertices move independently.
//!
//! **Colour** outside the band: at most [`COLOR_TOLERANCE`] = 1 of 255 per
//! channel, no exceptions. Both renderers round the shaded value to an
//! integer per sample; a value that lies within the `f32` error of a
//! half-integer rounds differently, which is one level. The box filter of
//! supersampling averages such differences and cannot exceed one level.
//! Where the picture itself changes quickly (the specular highlight on a
//! small feature) the snapped vertices add `2^-7 · Gc`, with `Gc` the
//! largest difference to the eight neighbours in the CPU picture, by the
//! argument used for the distance: one more level only where neighbouring
//! pixels differ by 128 levels or more.
//!
//! **How many** colours outside the band may differ by that one level: a
//! sample rounds differently only when its exact value lies within the
//! GPU's error `e` of a half-integer, which has probability `2e`. The error
//! is the `f32` error of the shading (`SHADING_ERROR` levels) plus what a
//! vertex moved by one grid step does to the interpolated colour
//! (`2^-7 · Gc`, as above). The sum of `min(1, 2 · (2^-7 · Gc + 0.004))`
//! over the pixels outside the band is the largest expected number of
//! differing pixels; more than that is a systematic error (a mean that is
//! truncated instead of rounded differs in half of the shaded pixels).
//!
//! # See-through pictures
//!
//! With `RenderParams::opacity` below 1 a sample's colour is a function of
//! two shaded surfaces: the front one and the nearest one strictly behind
//! it. The GPU blends the two with a table of the CPU renderer's own blend
//! (`blend.wgsl`), so the blend adds no error of its own: a sample whose
//! two layers equal the CPU's has the CPU's colour. What changes is that
//! two layers can be wrong instead of one, and both are known from the CPU
//! renderer alone: the *front picture* is the CPU frame at opacity 1, the
//! *picture behind* the CPU frame at opacity 0 (which is the layer behind
//! wherever the front is seen through). The tolerances above then read:
//!
//! - **The band** also holds every pixel within one pixel of a change of
//!   the face *behind* ([`behind_ids`]): where two surfaces behind the
//!   front pass through each other no mesh edge and no id of the front
//!   says so. Hidden feature edges were in the band already (it is made
//!   from all edges of the mesh, seen or not).
//! - **Colour** outside the band: each layer is within its own tolerance
//!   `1 + floor(2^-7 · G)`, with `G` its picture's gradient, so the exact
//!   blends differ by at most `o · e_front + (1 - o) · e_behind <=
//!   max(e_front, e_behind)`, an integer, and rounding is monotone and
//!   commutes with adding an integer: the blended levels differ by at most
//!   `1 + floor(2^-7 · max(G_front, G_behind))`.
//! - **How many** differ at all: a blended sample can differ only when one
//!   of its layers does, so the probabilities of the two layers add.
//!
//! Face ids and distances are the front's and keep their tolerances.
//!
//! # Cut-open pictures
//!
//! With `RenderParams::section` two things are added to a picture, and the
//! tolerances above cover both without being loosened.
//!
//! **The cut.** A fragment is left out where its distance from the plane
//! is positive. The GPU computes that distance in `f32` from positions
//! relative to the mesh centre: an error of at most `F32_PATH` times the
//! mesh's size, a few 10⁻⁷ of it. The two renderers can therefore decide
//! differently only for a sample whose surface point is that near to the
//! plane, which on screen is a strip along the line where the plane
//! crosses the surface, far narrower than a pixel unless the surface runs
//! along the plane (and a triangle that lies *in* the plane is decided as
//! a whole, by the same rule in both: `Cut::clips`). That line is known
//! from the mesh and the plane alone (`Cut::outline`), so **the band**
//! also holds every pixel within one pixel of the projected outline of
//! the section. Where the cut shows another surface behind it, the id
//! changes there too and the pixel was in the band already; the outline
//! adds the places where the surface behind has the same id (the inside
//! of a ball that is one face).
//!
//! **The cap.** It is where the number of fragments left out at a sample
//! is odd. The renderers count the same fragments except within `2^-8`
//! pixel of a triangle's edge, and a miscount shows only where it changes
//! the parity: not at an edge two triangles of the cut-away part share (a
//! sample is in exactly one of them in either renderer) and not at a
//! silhouette of the cut-away part (a sample is in both of its triangles
//! or in neither: the count changes by two), but only at the outline of
//! the section and at an edge with one triangle, both of which are in the
//! band. Outside the band the cap's id is therefore equal, its **colour**
//! is equal exactly (it is not shaded: one byte triple in both, and the
//! tolerance of one level is not used), and its **distance** is the
//! distance of a point of a plane, within the tolerance of any other
//! surface. Inside the band the same 1/128 holds: an edge passes within a
//! grid step of a pixel centre with that probability whether the edge is
//! a triangle's or the plane's.
//!
//! A see-through cut-open picture has both arguments at once: its two
//! layers are the CPU frames at opacity 1 and 0 with the section, and the
//! face behind the front is found with the section too ([`behind_ids`]).

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_gpu::{
    ADAPTER_HINT, Camera, Frame, Mesh, NO_FACE, Offscreen, Projection, RenderParams, SECTION_FACE,
    Section,
};
use craft3d_math::Vec3;
use craft3d_render::{Cut, section_bounds};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Largest colour difference per channel outside the band, in 1/255.
pub const COLOR_TOLERANCE: f64 = 1.0;
/// Largest fraction of the band's pixels whose face id, or whose colour by
/// more than the tolerance, may differ.
pub const BAND_MISMATCH: f64 = 1.0 / 128.0;
/// One sub-pixel grid step of the rasteriser, in pixels.
const SNAP: f64 = 1.0 / 256.0;
/// Bound on the `f32` error of a shaded value, in levels of 255: the
/// specular term `255 · 0.35 · x^48` multiplies the relative error of `x`
/// (a normalised dot product: about eight roundings of `2^-24`) by 48,
/// which is `89 · 48 · 8 · 2^-24 ≈ 0.002`; doubled.
const SHADING_ERROR: f64 = 0.004;
/// Bound on the accumulated `f32` rounding of a distance, relative to the
/// largest number involved: sixteen roundings of `2^-24`.
const F32_PATH: f64 = 16.0 / 16_777_216.0;

/// One renderer per adapter on this machine. Fails, loudly, when there is
/// none: a parity test that silently skips proves nothing.
pub fn offscreens() -> Vec<Offscreen> {
    let adapters = craft3d_gpu::adapters();
    assert!(
        !adapters.is_empty(),
        "no GPU adapter found, so the GPU renderer cannot be tested: {ADAPTER_HINT}"
    );
    adapters
        .iter()
        .map(|adapter| {
            Offscreen::on_adapter(adapter)
                .unwrap_or_else(|e| panic!("cannot use adapter {:?}: {e}", adapter.get_info().name))
        })
        .collect()
}

/// Where the developer looks (docs/testing.md, evidence).
pub fn evidence(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/gpu");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

pub fn params(width: u32, height: u32, ssaa: u32) -> RenderParams {
    RenderParams {
        width,
        height,
        ssaa,
        ..RenderParams::default()
    }
}

/// A perspective camera that frames `mesh` from `toward_eye`.
pub fn perspective(mesh: &Mesh, toward_eye: Vec3, fov_y_deg: f64, p: &RenderParams) -> Camera {
    let aspect = f64::from(p.width) / f64::from(p.height);
    Camera::framing(&mesh.bbox(), toward_eye, Vec3::Z, fov_y_deg, aspect).unwrap()
}

/// An orthographic camera that shows all of `mesh` from `toward_eye`.
pub fn orthographic(mesh: &Mesh, toward_eye: Vec3, p: &RenderParams) -> Camera {
    let bbox = mesh.bbox();
    let (center, diagonal) = (bbox.center().unwrap(), bbox.diagonal());
    let aspect = f64::from(p.width) / f64::from(p.height);
    Camera {
        eye: center + toward_eye.normalized().unwrap() * (diagonal * 2.0),
        target: center,
        up: Vec3::Z,
        projection: Projection::Orthographic {
            half_height: diagonal * 0.55 / aspect.min(1.0),
        },
    }
}

/// Pixel coordinates (x right, y down) and view distance of `p`, from the
/// definition in `craft3d_render::ViewAxes`. `None` at or behind the eye.
fn project(camera: &Camera, p: Vec3, width: f64, height: f64) -> Option<(f64, f64, f64)> {
    let axes = camera.axes().ok()?;
    let rel = p - axes.eye;
    let dist = rel.dot(axes.forward);
    let scale = match axes.projection {
        Projection::Perspective { .. } if dist > 0.0 => 1.0 / (dist * axes.half),
        Projection::Perspective { .. } => return None,
        Projection::Orthographic { .. } => 1.0 / axes.half,
    };
    let aspect = width / height;
    let x = (rel.dot(axes.right) * scale / aspect * 0.5 + 0.5) * width;
    let y = (0.5 - rel.dot(axes.up) * scale * 0.5) * height;
    (x.is_finite() && y.is_finite()).then_some((x, y, dist))
}

/// The coordinates of an edge's two ends, as bits, smaller first.
type EdgeKey = ([u64; 3], [u64; 3]);
/// An edge's two ends and, per triangle that uses it, the face id and the
/// sign of the triangle's area on screen.
type EdgeUse = (Vec3, Vec3, Vec<(u32, i8)>);

/// The band (see the module docs): `true` for a pixel within one pixel of a
/// feature edge of the mesh or of a change of face id in the CPU frame.
pub fn band(mesh: &Mesh, camera: &Camera, cpu: &Frame) -> Vec<bool> {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let mut near_edge = vec![false; w * h];
    let mut mark = |x: f64, y: f64| {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    near_edge[py as usize * w + px as usize] = true;
                }
            }
        }
    };

    // Changes of face id in the CPU frame: face borders that are visible,
    // and the curves where surfaces pass through each other.
    for y in 0..h {
        for x in 0..w {
            let id = cpu.ids[y * w + x];
            let right = x + 1 < w && cpu.ids[y * w + x + 1] != id;
            let below = y + 1 < h && cpu.ids[(y + 1) * w + x] != id;
            if right || below {
                mark(x as f64 + 0.5, y as f64 + 0.5);
                if right {
                    mark(x as f64 + 1.5, y as f64 + 0.5);
                }
                if below {
                    mark(x as f64 + 0.5, y as f64 + 1.5);
                }
            }
        }
    }

    // Feature edges of the mesh. Triangles meet where their corners have
    // the same coordinates, whichever vertex index carries them.
    let key = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let mut edges: BTreeMap<EdgeKey, EdgeUse> = BTreeMap::new();
    for (t, &face) in mesh.face_ids.iter().enumerate() {
        let corners = mesh.corners(t).unwrap();
        let screen = corners.map(|p| project(camera, p, w as f64, h as f64));
        let side = match screen {
            [Some(a), Some(b), Some(c)] => {
                let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
                if area > 0.0 {
                    1
                } else if area < 0.0 {
                    -1
                } else {
                    0
                }
            }
            _ => 0,
        };
        for (p, q) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (corners[p], corners[q]);
            let (a, b) = if key(p) <= key(q) { (p, q) } else { (q, p) };
            edges
                .entry((key(a), key(b)))
                .or_insert_with(|| (a, b, Vec::new()))
                .2
                .push((face, side));
        }
    }
    for (a, b, users) in edges.values() {
        let smooth = match users.as_slice() {
            [(face_a, side_a), (face_b, side_b)] => {
                face_a == face_b && side_a == side_b && *side_a != 0
            }
            _ => false,
        };
        if smooth {
            continue;
        }
        let (Some(a), Some(b)) = (
            project(camera, *a, w as f64, h as f64),
            project(camera, *b, w as f64, h as f64),
        ) else {
            continue;
        };
        let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        // Half-pixel steps; an edge that long is mostly off screen anyway.
        let steps = ((length * 2.0).ceil() as usize).clamp(1, 8 * (w + h));
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            mark(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        }
    }
    near_edge
}

/// The probability, at most, that a pixel outside the band differs by a
/// level, given the largest difference to its neighbours in the CPU picture
/// (see the module docs).
pub fn differ_probability(color_gradient: f64) -> f64 {
    (2.0 * (2.0 * SNAP * color_gradient + SHADING_ERROR)).min(1.0)
}

/// The colour tolerance of a pixel outside the band, in levels, given the
/// largest difference to its neighbours in the CPU picture.
pub fn color_tolerance(color_gradient: f64) -> f64 {
    COLOR_TOLERANCE + (2.0 * SNAP * color_gradient).floor()
}

/// What the comparison of one scene found.
#[derive(Debug, Default)]
pub struct Parity {
    pub pixels: usize,
    pub covered: usize,
    pub band: usize,
    /// Outside the band: pixels with different face ids.
    pub id_outside: usize,
    /// Outside the band: pixels whose distance differs by more than the
    /// tolerance, and the largest difference as a fraction of it.
    pub depth_outside: usize,
    pub depth_ratio: f64,
    /// Outside the band: pixels by their largest channel difference
    /// (0, 1, 2, 3 or more), and those over their tolerance.
    pub color_histogram: [usize; 4],
    pub color_outside: usize,
    /// Outside the band: the number of pixels expected to differ at all, at
    /// most (the sum of [`differ_probability`]).
    pub color_differ_allowed: f64,
    /// Inside the band: pixels with different ids, and with a colour
    /// difference over the tolerance.
    pub id_band: usize,
    pub color_band: usize,
}

fn neighbours(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    (-1_i64..=1)
        .flat_map(|dy| (-1_i64..=1).map(move |dx| (dx, dy)))
        .filter_map(move |(dx, dy)| {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            (nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && (dx, dy) != (0, 0))
                .then(|| ny as usize * w + nx as usize)
        })
}

/// The face behind the front one at every pixel's first sample (the
/// sample the frame's ids come from), or [`NO_FACE`]: the second of the
/// two surfaces a see-through picture is made of. It is found here from
/// the definition, with a rasteriser of its own, because the CPU frame
/// holds the front's ids only; it is used for the band and nothing else,
/// so surfaces closer together than a billionth of the scene's depth
/// count as one.
pub fn behind_ids(mesh: &Mesh, camera: &Camera, width: u32, height: u32, ssaa: u32) -> Vec<u32> {
    behind_ids_cut(mesh, camera, width, height, ssaa, None)
}

/// The depth key and the point of the plane of `cut` under the position
/// (`px`, `py`) of a picture of `w` × `h` pixels, from the definition of
/// the camera: where the ray through the position meets the plane.
fn cap_key(camera: &Camera, cut: &Cut, px: f64, py: f64, w: f64, h: f64) -> Option<f64> {
    let axes = camera.axes().ok()?;
    let across = axes.right * ((px / w * 2.0 - 1.0) * (w / h) * axes.half)
        + axes.up * ((1.0 - py / h * 2.0) * axes.half);
    let perspective = matches!(axes.projection, Projection::Perspective { .. });
    let (start, direction) = if perspective {
        (axes.eye, axes.forward + across)
    } else {
        (axes.eye + across, axes.forward)
    };
    let dist = (cut.origin - start).dot(cut.normal) / direction.dot(cut.normal);
    let key = if perspective { -1.0 / dist } else { dist };
    let visible = !perspective || dist > 0.0;
    (key.is_finite() && visible && cut.holds_cap(start + direction * dist)).then_some(key)
}

/// [`behind_ids`] of a picture cut open by `section`: fragments in the
/// half that is cut away are left out and counted, and where their number
/// is odd the cap is one more surface, with [`SECTION_FACE`] as its face.
pub fn behind_ids_cut(
    mesh: &Mesh,
    camera: &Camera,
    width: u32,
    height: u32,
    ssaa: u32,
    section: Option<&Section>,
) -> Vec<u32> {
    let cut = section.and_then(|section| Cut::new(section, &section_bounds(mesh)));
    let (w, h) = (width as usize, height as usize);
    let perspective = matches!(camera.projection, Projection::Perspective { .. });
    let offset = 0.5 / f64::from(ssaa.max(1));
    let projected: Vec<Option<(f64, f64, f64)>> = mesh
        .positions
        .iter()
        .map(|&p| {
            project(camera, p, w as f64, h as f64).map(|(x, y, dist)| {
                // The CPU renderer's depth key: affine in screen space.
                (x, y, if perspective { -1.0 / dist } else { dist })
            })
        })
        .collect();
    let keys = || projected.iter().flatten().map(|p| p.2);
    let span = keys().fold(f64::NEG_INFINITY, f64::max) - keys().fold(f64::INFINITY, f64::min);
    let eps = if span.is_finite() { span * 1e-9 } else { 0.0 };
    // Per pixel: the key and the face of the front and of the one behind.
    let nothing = (f64::INFINITY, NO_FACE);
    let mut layers = vec![(nothing, nothing); w * h];
    let mut odd = vec![false; w * h];
    let put = |layer: &mut ((f64, u32), (f64, u32)), key: f64, face: u32| {
        let (front, behind) = layer;
        if key < front.0 - eps {
            // A new front: the old one is behind it now.
            if front.0.is_finite() {
                *behind = *front;
            }
            *front = (key, face);
        } else if key > front.0 + eps && key < behind.0 {
            *behind = (key, face);
        }
    };
    for (tri, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
        let [Some(a), Some(b), Some(c)] = tri.map(|k| projected[k as usize]) else {
            continue;
        };
        // The corners' distances from the plane, and which way the
        // triangle faces it.
        let cut_triangle = cut.map(|cut| {
            let [pa, pb, pc] = tri.map(|k| mesh.positions[k as usize]);
            let facing = (pb - pa)
                .cross(pc - pa)
                .normalized()
                .map_or(0.0, |n| n.dot(cut.normal));
            ([cut.side(pa), cut.side(pb), cut.side(pc)], facing)
        });
        let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if area == 0.0 || !area.is_finite() {
            continue;
        }
        let range = |lo: f64, hi: f64, len: usize| {
            let first = (lo - offset).ceil().max(0.0) as i64;
            let last = ((hi - offset).floor().max(-1.0) as i64).min(len as i64 - 1);
            first..=last
        };
        for y in range(a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1), h) {
            for x in range(a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0), w) {
                let (px, py) = (x as f64 + offset, y as f64 + offset);
                let edge = |p: (f64, f64, f64), q: (f64, f64, f64)| {
                    ((q.0 - p.0) * (py - p.1) - (q.1 - p.1) * (px - p.0)) / area
                };
                let (wa, wb, wc) = (edge(b, c), edge(c, a), edge(a, b));
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let key = (wa * a.2 + wb * b.2 + wc * c.2) / (wa + wb + wc);
                let i = y as usize * w + x as usize;
                if let (Some(cut), Some(([sa, sb, sc], facing))) = (cut, cut_triangle) {
                    // Perspective-correct: weighted by one over the
                    // distance, which the key holds in perspective.
                    let inv = |p: (f64, f64, f64)| if perspective { -p.2 } else { 1.0 };
                    let (qa, qb, qc) = (wa * inv(a), wb * inv(b), wc * inv(c));
                    if cut.clips((qa * sa + qb * sb + qc * sc) / (qa + qb + qc), facing) {
                        odd[i] = !odd[i];
                        continue;
                    }
                }
                put(&mut layers[i], key, face);
            }
        }
    }
    if let Some(cut) = cut.filter(|cut| cut.cap) {
        for i in (0..w * h).filter(|&i| odd[i]) {
            let (px, py) = ((i % w) as f64 + offset, (i / w) as f64 + offset);
            if let Some(key) = cap_key(camera, &cut, px, py, w as f64, h as f64) {
                put(&mut layers[i], key, SECTION_FACE);
            }
        }
    }
    layers.into_iter().map(|(_, behind)| behind.1).collect()
}

/// Marks every pixel within one pixel of the projected outline of the
/// section in `band` (see the module docs, *Cut-open pictures*).
fn mark_outline(
    mesh: &Mesh,
    camera: &Camera,
    section: &Section,
    w: usize,
    h: usize,
    band: &mut [bool],
) {
    let Some(cut) = Cut::new(section, &section_bounds(mesh)) else {
        return;
    };
    let mut mark = |x: f64, y: f64| {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    band[py as usize * w + px as usize] = true;
                }
            }
        }
    };
    for [a, b] in cut.outline(mesh) {
        let (Some(a), Some(b)) = (
            project(camera, a, w as f64, h as f64),
            project(camera, b, w as f64, h as f64),
        ) else {
            continue;
        };
        let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = ((length * 2.0).ceil() as usize).clamp(1, 8 * (w + h));
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            mark(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        }
    }
}

/// The band of a picture with `params`: [`band`], the outline of its
/// section, and for a see-through picture the changes of the face behind.
pub fn band_of(mesh: &Mesh, camera: &Camera, params: &RenderParams, cpu: &Frame) -> Vec<bool> {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let mut band = band(mesh, camera, cpu);
    if let Some(section) = &params.section {
        mark_outline(mesh, camera, section, w, h, &mut band);
    }
    if params.opacity() < 1.0 {
        let behind = behind_ids_cut(
            mesh,
            camera,
            cpu.width,
            cpu.height,
            params.ssaa,
            params.section.as_ref(),
        );
        mark_changes(&behind, w, h, &mut band);
    }
    band
}

/// Compares a GPU frame with the CPU frame of the same scene, cut open or
/// not, see-through or not (see the module docs).
pub fn compare_with(
    mesh: &Mesh,
    camera: &Camera,
    params: &RenderParams,
    cpu: &Frame,
    gpu: &Frame,
) -> (Parity, Vec<bool>) {
    let band = band_of(mesh, camera, params, cpu);
    if params.opacity() < 1.0 {
        let [front, back] = layers(mesh, camera, params);
        compare_layered(mesh, camera, cpu, gpu, band, &[&front, &back])
    } else {
        compare_layered(mesh, camera, cpu, gpu, band, &[cpu])
    }
}

/// The band of a see-through picture: [`band`], and every pixel within one
/// pixel of a change of the face behind the front one ([`behind_ids`]).
pub fn band_see_through(mesh: &Mesh, camera: &Camera, cpu: &Frame, ssaa: u32) -> Vec<bool> {
    let mut band = band(mesh, camera, cpu);
    let behind = behind_ids(mesh, camera, cpu.width, cpu.height, ssaa);
    mark_changes(&behind, cpu.width as usize, cpu.height as usize, &mut band);
    band
}

/// Marks every pixel within one pixel of a change of `ids` in `band`.
fn mark_changes(ids: &[u32], w: usize, h: usize, band: &mut [bool]) {
    let mut mark = |x: usize, y: usize| {
        for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                band[ny * w + nx] = true;
            }
        }
    };
    for y in 0..h {
        for x in 0..w {
            let id = ids[y * w + x];
            if x + 1 < w && ids[y * w + x + 1] != id {
                mark(x, y);
                mark(x + 1, y);
            }
            if y + 1 < h && ids[(y + 1) * w + x] != id {
                mark(x, y);
                mark(x, y + 1);
            }
        }
    }
}

/// The CPU renderer's two layers of a see-through picture, as pictures:
/// the frame at opacity 1 (the front) and at opacity 0 (the one behind,
/// wherever the front is seen through).
pub fn layers(mesh: &Mesh, camera: &Camera, params: &RenderParams) -> [Frame; 2] {
    [1.0, 0.0].map(|opacity| {
        let layer = RenderParams { opacity, ..*params };
        craft3d_render::render(mesh, camera, &layer).unwrap()
    })
}

/// Compares a GPU frame with the CPU frame of the same scene.
pub fn compare(mesh: &Mesh, camera: &Camera, cpu: &Frame, gpu: &Frame) -> (Parity, Vec<bool>) {
    let band = band(mesh, camera, cpu);
    compare_layered(mesh, camera, cpu, gpu, band, &[cpu])
}

/// Compares a see-through GPU frame with the CPU frame of the same scene
/// (see the module docs, *See-through pictures*).
pub fn compare_see_through(
    mesh: &Mesh,
    camera: &Camera,
    params: &RenderParams,
    cpu: &Frame,
    gpu: &Frame,
) -> (Parity, Vec<bool>) {
    let band = band_see_through(mesh, camera, cpu, params.ssaa);
    let [front, back] = layers(mesh, camera, params);
    compare_layered(mesh, camera, cpu, gpu, band, &[&front, &back])
}

/// The comparison, given the band and the pictures whose gradients bound
/// the colour: the CPU frame itself for an opaque picture, its two layers
/// for a see-through one.
fn compare_layered(
    mesh: &Mesh,
    camera: &Camera,
    cpu: &Frame,
    gpu: &Frame,
    band: Vec<bool>,
    layers: &[&Frame],
) -> (Parity, Vec<bool>) {
    assert_eq!((cpu.width, cpu.height), (gpu.width, gpu.height));
    assert_eq!(cpu.rgb.len(), gpu.rgb.len());
    assert_eq!(cpu.ids.len(), gpu.ids.len());
    assert_eq!(cpu.depth.len(), gpu.depth.len());
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let bbox = mesh.bbox();
    let reach = bbox
        .center()
        .map_or(0.0, |c| (camera.eye - c).length() + bbox.diagonal());
    let mut parity = Parity {
        pixels: w * h,
        ..Parity::default()
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (id_c, id_g) = (cpu.ids[i], gpu.ids[i]);
            parity.covered += usize::from(id_c != NO_FACE);
            let channel = |frame: &Frame, at: usize, c: usize| f64::from(frame.rgb[at * 3 + c]);
            let color_diff = (0..3)
                .map(|c| (channel(cpu, i, c) - channel(gpu, i, c)).abs())
                .fold(0.0, f64::max);
            if band[i] {
                parity.band += 1;
                parity.id_band += usize::from(id_c != id_g);
                parity.color_band += usize::from(color_diff > COLOR_TOLERANCE);
                continue;
            }
            parity.id_outside += usize::from(id_c != id_g);
            // Per layer, the largest difference to the eight neighbours.
            let gradients = layers.iter().map(|layer| {
                neighbours(x, y, w, h)
                    .flat_map(|n| (0..3).map(move |c| (n, c)))
                    .map(|(n, c)| (channel(layer, i, c) - channel(layer, n, c)).abs())
                    .fold(0.0, f64::max)
            });
            let color_gradient = gradients.clone().fold(0.0, f64::max);
            let color_tolerance = COLOR_TOLERANCE + (2.0 * SNAP * color_gradient).floor();
            parity.color_histogram[(color_diff as usize).min(3)] += 1;
            parity.color_differ_allowed += gradients.map(differ_probability).sum::<f64>().min(1.0);
            parity.color_outside += usize::from(color_diff > color_tolerance);

            let (d_c, d_g) = (cpu.depth[i], gpu.depth[i]);
            if d_c.is_infinite() || d_g.is_infinite() {
                // Nothing drawn: both must say so.
                parity.depth_outside += usize::from(d_c != d_g);
                continue;
            }
            let gradient = neighbours(x, y, w, h)
                .map(|n| (cpu.depth[n] - d_c).abs())
                .filter(|g| g.is_finite())
                .fold(0.0, f64::max);
            let tolerance = F32_PATH * reach + 2.0 * SNAP * gradient;
            let error = (d_c - d_g).abs();
            // NaN on the GPU side must fail too.
            if error.is_nan() || error > tolerance {
                parity.depth_outside += 1;
            }
            parity.depth_ratio = parity.depth_ratio.max(error / tolerance);
        }
    }
    (parity, band)
}

impl Parity {
    /// The assertions of oracle 7.
    pub fn check(&self, scene: &str) {
        let report = format!("{scene}: {self:?}");
        // The comparison must not be hollow: most of the picture, and most
        // of the model in it, is compared strictly.
        assert!(self.covered > self.pixels / 50, "nothing to see: {report}");
        assert!(
            self.band < self.pixels / 2,
            "the band covers half the picture: {report}"
        );
        assert_eq!(
            self.id_outside, 0,
            "face ids differ outside the band: {report}"
        );
        assert_eq!(self.depth_outside, 0, "distances differ: {report}");
        assert_eq!(self.color_outside, 0, "colours differ: {report}");
        let differing: usize = self.color_histogram.iter().skip(1).sum();
        assert!(
            differing as f64 <= self.color_differ_allowed.ceil(),
            "{differing} colours differ by a level where rounding explains at most {}: {report}",
            self.color_differ_allowed.ceil()
        );
        let allowed = (self.band as f64 * BAND_MISMATCH).ceil() as usize;
        assert!(
            self.id_band <= allowed,
            "more than {allowed} face ids differ inside the band: {report}"
        );
        assert!(
            self.color_band <= allowed,
            "more than {allowed} colours differ inside the band: {report}"
        );
    }
}

/// CPU picture, GPU picture and their differences side by side, as a PNG:
/// in the third panel the band is dark blue, a colour difference is green
/// (brighter is larger; one level is already visible), a different face id
/// is red.
pub fn write_evidence(name: &str, cpu: &Frame, gpu: &Frame, band: &[bool]) -> PathBuf {
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let mut rgb = Vec::with_capacity(w * h * 9);
    for y in 0..h {
        for frame in [cpu, gpu] {
            rgb.extend_from_slice(&frame.rgb[y * w * 3..(y + 1) * w * 3]);
        }
        for x in 0..w {
            let i = y * w + x;
            let diff = (0..3)
                .map(|c| cpu.rgb[i * 3 + c].abs_diff(gpu.rgb[i * 3 + c]))
                .max()
                .unwrap();
            let red = if cpu.ids[i] != gpu.ids[i] { 255 } else { 0 };
            let green = if diff == 0 {
                0
            } else {
                (96 + u32::from(diff) * 16).min(255) as u8
            };
            let blue = if band[i] { 90 } else { 0 };
            rgb.extend([red, green, blue]);
        }
    }
    let path = evidence(name);
    let png = craft3d_render::encode_png(cpu.width * 3, cpu.height, &rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    path
}

// ---------------------------------------------------------------------
// The multisampled picture of the viewport renderer (`Onscreen`).
//
// Multisampling shades once per pixel, at its centre, exactly where the
// CPU renderer takes its one sample for `ssaa = 1`; the samples only
// decide coverage. So a pixel whose samples all lie in one triangle has
// the very colour of the readback renderer, within the tolerance above.
// Two kinds of pixel are different by design:
//
// - **In the band**, at a feature edge, the samples lie on both sides and
//   the resolved colour is a blend: that is the anti-aliasing. Its colour
//   is not compared with the CPU's hard edge; the number of pixels that
//   differ is only measured.
// - **On a seam**, within a pixel of an interior triangle edge of one
//   face, the samples may lie in two triangles, each shaded at the pixel
//   centre by extrapolating its own interpolated normal, and the resolved
//   colour is their coverage-weighted mean. Each fragment is within one
//   pixel of gradient of the centre value, so the pixel is allowed
//   `COLOR_TOLERANCE + Gc` levels, with `Gc` the largest difference to its
//   eight neighbours in the CPU picture.
//
// Face ids and distances come from the renderer's capture pass at one
// sample per pixel and keep the tolerance above everywhere.

/// `true` for a pixel within one pixel of any projected triangle edge:
/// where a multisampled pixel may take its colour from two triangles.
pub fn seams(mesh: &Mesh, camera: &Camera, width: u32, height: u32) -> Vec<bool> {
    let (w, h) = (width as usize, height as usize);
    let mut near = vec![false; w * h];
    let mut mark = |x: f64, y: f64| {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (px, py) = (cx + dx, cy + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    near[py as usize * w + px as usize] = true;
                }
            }
        }
    };
    for t in 0..mesh.triangles.len() {
        let corners = mesh.corners(t).unwrap();
        let screen = corners.map(|p| project(camera, p, w as f64, h as f64));
        for (p, q) in [(0, 1), (1, 2), (2, 0)] {
            let (Some(a), Some(b)) = (screen[p], screen[q]) else {
                continue;
            };
            let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let steps = ((length * 2.0).ceil() as usize).clamp(1, 8 * (w + h));
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                mark(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            }
        }
    }
    near
}

/// What the comparison of a multisampled picture found.
#[derive(Debug, Default)]
pub struct MsaaParity {
    pub pixels: usize,
    pub covered: usize,
    pub band: usize,
    /// Pixels outside the band: on a seam, and not.
    pub seam: usize,
    pub plain: usize,
    /// Outside the band: different face ids, distances over the tolerance
    /// and the largest distance error as a fraction of it.
    pub id_outside: usize,
    pub depth_outside: usize,
    pub depth_ratio: f64,
    /// Plain pixels over the colour tolerance of §7.2, their histogram of
    /// the largest channel difference (0, 1, 2, 3 or more), and how many
    /// of them rounding explains at most.
    pub color_plain_over: usize,
    pub color_plain_histogram: [usize; 4],
    pub color_plain_allowed: f64,
    /// Seam pixels over `COLOR_TOLERANCE + Gc`, and seam pixels that differ
    /// by more than one level (measured, not bounded).
    pub color_seam_over: usize,
    pub color_seam_differ: usize,
    /// Inside the band: different ids (bounded), and colours that differ
    /// by more than one level (the anti-aliasing: measured only).
    pub id_band: usize,
    pub color_band: usize,
}

/// Compares the multisampled picture `gpu` (with ids and distances from
/// the capture pass) with the CPU frame of the same scene at `ssaa = 1`.
pub fn compare_msaa(
    mesh: &Mesh,
    camera: &Camera,
    cpu: &Frame,
    gpu: &Frame,
) -> (MsaaParity, Vec<bool>) {
    let band = band(mesh, camera, cpu);
    compare_msaa_layered(mesh, camera, cpu, gpu, band, &[cpu])
}

/// [`compare_msaa`] for a see-through picture: the band and the gradients
/// of *See-through pictures* in the module docs. A seam is a pixel near a
/// triangle edge of either layer (the seams are made from every triangle
/// of the mesh, seen or not).
pub fn compare_msaa_see_through(
    mesh: &Mesh,
    camera: &Camera,
    params: &RenderParams,
    cpu: &Frame,
    gpu: &Frame,
) -> (MsaaParity, Vec<bool>) {
    let band = band_see_through(mesh, camera, cpu, params.ssaa);
    let [front, back] = layers(mesh, camera, params);
    compare_msaa_layered(mesh, camera, cpu, gpu, band, &[&front, &back])
}

/// [`compare_msaa`] for a picture with `params`: cut open or not,
/// see-through or not, with the band of [`band_of`].
pub fn compare_msaa_with(
    mesh: &Mesh,
    camera: &Camera,
    params: &RenderParams,
    cpu: &Frame,
    gpu: &Frame,
) -> (MsaaParity, Vec<bool>) {
    let band = band_of(mesh, camera, params, cpu);
    if params.opacity() < 1.0 {
        let [front, back] = layers(mesh, camera, params);
        compare_msaa_layered(mesh, camera, cpu, gpu, band, &[&front, &back])
    } else {
        compare_msaa_layered(mesh, camera, cpu, gpu, band, &[cpu])
    }
}

fn compare_msaa_layered(
    mesh: &Mesh,
    camera: &Camera,
    cpu: &Frame,
    gpu: &Frame,
    band: Vec<bool>,
    layers: &[&Frame],
) -> (MsaaParity, Vec<bool>) {
    assert_eq!((cpu.width, cpu.height), (gpu.width, gpu.height));
    assert_eq!(cpu.rgb.len(), gpu.rgb.len());
    assert_eq!(cpu.ids.len(), gpu.ids.len());
    assert_eq!(cpu.depth.len(), gpu.depth.len());
    let (w, h) = (cpu.width as usize, cpu.height as usize);
    let seams = seams(mesh, camera, cpu.width, cpu.height);
    let bbox = mesh.bbox();
    let reach = bbox
        .center()
        .map_or(0.0, |c| (camera.eye - c).length() + bbox.diagonal());
    let mut out = MsaaParity {
        pixels: w * h,
        ..MsaaParity::default()
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let (id_c, id_g) = (cpu.ids[i], gpu.ids[i]);
            out.covered += usize::from(id_c != NO_FACE);
            let channel = |frame: &Frame, at: usize, c: usize| f64::from(frame.rgb[at * 3 + c]);
            let color_diff = (0..3)
                .map(|c| (channel(cpu, i, c) - channel(gpu, i, c)).abs())
                .fold(0.0, f64::max);
            if band[i] {
                out.band += 1;
                out.id_band += usize::from(id_c != id_g);
                out.color_band += usize::from(color_diff > COLOR_TOLERANCE);
                continue;
            }
            out.id_outside += usize::from(id_c != id_g);
            let gradients = layers.iter().map(|layer| {
                neighbours(x, y, w, h)
                    .flat_map(|n| (0..3).map(move |c| (n, c)))
                    .map(|(n, c)| (channel(layer, i, c) - channel(layer, n, c)).abs())
                    .fold(0.0, f64::max)
            });
            let color_gradient = gradients.clone().fold(0.0, f64::max);
            if seams[i] {
                out.seam += 1;
                out.color_seam_over += usize::from(color_diff > COLOR_TOLERANCE + color_gradient);
                out.color_seam_differ += usize::from(color_diff > COLOR_TOLERANCE);
            } else {
                out.plain += 1;
                let tolerance = COLOR_TOLERANCE + (2.0 * SNAP * color_gradient).floor();
                out.color_plain_histogram[(color_diff as usize).min(3)] += 1;
                out.color_plain_allowed += gradients.map(differ_probability).sum::<f64>().min(1.0);
                out.color_plain_over += usize::from(color_diff > tolerance);
            }
            let (d_c, d_g) = (cpu.depth[i], gpu.depth[i]);
            if d_c.is_infinite() || d_g.is_infinite() {
                out.depth_outside += usize::from(d_c != d_g);
                continue;
            }
            let gradient = neighbours(x, y, w, h)
                .map(|n| (cpu.depth[n] - d_c).abs())
                .filter(|g| g.is_finite())
                .fold(0.0, f64::max);
            let tolerance = F32_PATH * reach + 2.0 * SNAP * gradient;
            let error = (d_c - d_g).abs();
            if error.is_nan() || error > tolerance {
                out.depth_outside += 1;
            }
            out.depth_ratio = out.depth_ratio.max(error / tolerance);
        }
    }
    (out, band)
}

impl MsaaParity {
    /// The assertions for a multisampled picture (see above).
    pub fn check(&self, scene: &str) {
        let report = format!("{scene}: {self:?}");
        assert!(self.covered > self.pixels / 50, "nothing to see: {report}");
        assert!(
            self.band < self.pixels / 2,
            "the band covers half the picture: {report}"
        );
        assert!(
            self.plain > self.pixels / 4,
            "too little of the picture is compared strictly: {report}"
        );
        assert_eq!(
            self.id_outside, 0,
            "face ids differ outside the band: {report}"
        );
        assert_eq!(self.depth_outside, 0, "distances differ: {report}");
        assert_eq!(
            self.color_plain_over, 0,
            "colours differ away from every edge: {report}"
        );
        let differing: usize = self.color_plain_histogram.iter().skip(1).sum();
        assert!(
            differing as f64 <= self.color_plain_allowed.ceil(),
            "{differing} colours differ by a level where rounding explains at most {}: {report}",
            self.color_plain_allowed.ceil()
        );
        assert_eq!(
            self.color_seam_over, 0,
            "colours on a seam differ by more than a pixel of gradient: {report}"
        );
        let allowed = (self.band as f64 * BAND_MISMATCH).ceil() as usize;
        assert!(
            self.id_band <= allowed,
            "more than {allowed} face ids differ inside the band: {report}"
        );
    }
}

// ---------------------------------------------------------------------
// Rule 5: the same input gives the same frame, bit for bit.

/// `None` when `got` is `expected` bit for bit (colours, face ids and the
/// bits of the distances). Otherwise a short report, and the evidence for
/// the developer under `target/evidence/gpu/`: `<name>.png` with the
/// expected frame, the frame got and their difference side by side (in
/// the third panel a pixel that differs is red over the expected picture
/// darkened), and `<name>.txt` with the pixels that differ. A rare
/// difference cannot be brought back on demand, so the run that sees it
/// has to leave what it saw.
pub fn frame_difference(name: &str, expected: &Frame, got: &Frame) -> Option<String> {
    let same_size = (expected.width, expected.height) == (got.width, got.height)
        && expected.rgb.len() == got.rgb.len()
        && expected.ids.len() == got.ids.len()
        && expected.depth.len() == got.depth.len()
        && expected.rgb.len() == expected.ids.len() * 3;
    if !same_size {
        return Some(format!(
            "{}x{} against {}x{}",
            expected.width, expected.height, got.width, got.height
        ));
    }
    let pixels = expected.ids.len();
    let differs: Vec<usize> = (0..pixels)
        .filter(|&i| {
            expected.rgb[i * 3..i * 3 + 3] != got.rgb[i * 3..i * 3 + 3]
                || expected.ids[i] != got.ids[i]
                || expected.depth[i].to_bits() != got.depth[i].to_bits()
        })
        .collect();
    if differs.is_empty() && expected.sample.to_bits() == got.sample.to_bits() {
        return None;
    }
    let w = expected.width as usize;
    let mut listing = format!(
        "{} pixels differ ({}x{}, sample {} / {}); x y: rgb expected / got, \
         id expected / got, distance expected / got\n",
        differs.len(),
        expected.width,
        expected.height,
        expected.sample,
        got.sample
    );
    for &i in differs.iter().take(1000) {
        listing.push_str(&format!(
            "{} {}: {:?} / {:?}, {} / {}, {} / {}\n",
            i % w,
            i / w,
            &expected.rgb[i * 3..i * 3 + 3],
            &got.rgb[i * 3..i * 3 + 3],
            expected.ids[i],
            got.ids[i],
            expected.depth[i],
            got.depth[i]
        ));
    }
    std::fs::write(evidence(&format!("{name}.txt")), &listing).unwrap();
    let mut rgb = Vec::with_capacity(pixels * 9);
    for y in 0..expected.height as usize {
        for frame in [expected, got] {
            rgb.extend_from_slice(&frame.rgb[y * w * 3..(y + 1) * w * 3]);
        }
        for x in 0..w {
            let i = y * w + x;
            if differs.binary_search(&i).is_ok() {
                rgb.extend([255, 0, 0]);
            } else {
                rgb.extend(expected.rgb[i * 3..i * 3 + 3].iter().map(|c| c / 3));
            }
        }
    }
    let path = evidence(&format!("{name}.png"));
    let png = craft3d_render::encode_png(expected.width * 3, expected.height, &rgb).unwrap();
    std::fs::write(&path, png).unwrap();
    let first: Vec<&str> = listing.lines().skip(1).take(4).collect();
    Some(format!(
        "{} pixels differ, first: {}; evidence: {}",
        differs.len(),
        first.join("; "),
        path.display()
    ))
}
