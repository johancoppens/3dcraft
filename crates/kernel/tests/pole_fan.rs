//! Meshes of faces that close in a point of their surface (a pole), and of
//! seams that run into one: every mesh of a solid is a closed manifold, or
//! an error that names the face; never a mesh that is neither.
//!
//! A mesh is held to that as a program that reads it would: its vertices
//! welded where they are the same bits, every side used once each way,
//! the triangles round every vertex one fan, and no two vertices nearer
//! than a trillionth of the body's size (points that are one place to the
//! eye, and two to the mesh). And its volume within the chord bound of the
//! body's (area times tolerance), the normal at a smooth tip that of the
//! tip's plane, and the same bits on one, two and all threads.
//!
//! The families: domes over thin ellipses closed across a seam, their tip
//! anywhere over the base (as STEP files hold them; no command builds
//! one), the faces of which met the fault (docs/design/tessellator.md ›
//! *Sideways at a seam*); spindles closed across a seam at both poles;
//! seeded spline profiles turned onto their axis (one pole or two, square,
//! slanted and needle ends, whole and part turns: what `feature.revolve`
//! builds); the reference solids with poles (sphere, hemisphere, cone,
//! spline cone); and hostile faces (all pole, not finite).

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Plane, Surface};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{Mesh, TessParams, tessellate_body, tessellate_body_with_threads};
use craft3d_testkit::solids;
use craft3d_topo::{Body, BodyKind, Coedge, check, mass_bound};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The model's frame, one turned, and that one thirty sizes away.
fn frames(k: f64) -> [(&'static str, Frame); 3] {
    let turned = Frame::new(
        Vec3::ZERO,
        Vec3::new(0.31, -0.52, 0.79),
        Vec3::new(1.0, 0.27, -0.11),
    )
    .unwrap();
    let far = turned.at(Vec3::new(30.0 * k, -21.0 * k, 9.0 * k)).unwrap();
    [("world", Frame::WORLD), ("turned", turned), ("far", far)]
}

fn forward(edge: craft3d_topo::EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: false,
    }
}

fn backward(edge: craft3d_topo::EdgeId) -> Coedge {
    Coedge {
        edge,
        reversed: true,
    }
}

/// The rational quadratic circle in nine points (four quarters), as
/// shares of the half axes, and its weights and knots.
fn ring() -> ([(f64, f64); 9], [f64; 9], Vec<f64>) {
    let points = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ];
    let weights = [1.0, FRAC_1_SQRT_2].repeat(5);
    let mut w = [1.0; 9];
    w.copy_from_slice(&weights[..9]);
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    (points, w, knots)
}

/// A dome over the ellipse with the half axes `a` and `b`, closed across
/// a seam (on `+x`, or `−x` with `flip`) in a tip `length` over `foot`:
/// one rational face of degree 2 × 2 whose border `v = 1` is the tip, the
/// seam an edge its loop uses twice, and a flat base. Its volume is
/// eleven fifteenths of `π a b length` wherever the tip stands.
fn seamed_dome(frame: &Frame, (a, b): (f64, f64), flip: bool, foot: [f64; 2], length: f64) -> Body {
    let s = if flip { -1.0 } else { 1.0 };
    let (points, w, knots) = ring();
    let top = frame.point(foot[0], foot[1], length);
    let (mut net, mut weights, mut rim) = (vec![], vec![], vec![]);
    for (&(x, y), &w) in points.iter().zip(&w) {
        let low = frame.point(s * a * x, s * b * y, 0.0);
        net.extend([low, frame.point(s * a * x, s * b * y, length), top]);
        weights.extend([w; 3]);
        rim.push(low);
    }
    let ends = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let surface = NurbsSurface::new(
        2,
        2,
        knots.clone(),
        ends.clone(),
        9,
        3,
        net.clone(),
        weights,
    )
    .unwrap();
    let rim = NurbsCurve::new(2, knots, rim, w.to_vec()).unwrap();
    let seam = NurbsCurve::new(2, ends, net[..3].to_vec(), vec![1.0; 3]).unwrap();
    let mut body = Body::new(BodyKind::Solid);
    let (start, apex) = (body.add_vertex(net[0]), body.add_vertex(top));
    let rim = body.add_edge(Curve3::Nurbs(rim), (0.0, 1.0), start, start);
    let seam = body.add_edge(Curve3::Nurbs(seam), (0.0, 1.0), start, apex);
    let outline = body.add_loop(vec![forward(rim), forward(seam), backward(seam)]);
    let dome = body.add_face(Surface::Nurbs(surface), false, vec![outline]);
    let down = frame.reversed_at(frame.origin()).unwrap();
    let outline = body.add_loop(vec![backward(rim)]);
    let base = body.add_face(Surface::Plane(Plane::new(down)), false, vec![outline]);
    body.add_shell(vec![dome, base]);
    body
}

/// A spindle closed across a seam at both poles: one face whose
/// meridians run from a point to a point `height` above it through the
/// ellipse with the half axes `a` and `b` at half the height, the seam
/// (on `+x`, or `−x` with `flip`) an edge from pole to pole that its one
/// loop walks there and back, as a STEP file has a sphere. Every section
/// is the ellipse scaled by `2 v (1 − v)` at the height `v height`, so its
/// volume is `2 π a b height / 15`.
fn seamed_spindle(frame: &Frame, (a, b): (f64, f64), flip: bool, height: f64) -> Body {
    let s = if flip { -1.0 } else { 1.0 };
    let (points, w, knots) = ring();
    let (low, high) = (frame.point(0.0, 0.0, 0.0), frame.point(0.0, 0.0, height));
    let (mut net, mut weights) = (vec![], vec![]);
    for (&(x, y), &w) in points.iter().zip(&w) {
        net.extend([low, frame.point(s * a * x, s * b * y, 0.5 * height), high]);
        weights.extend([w; 3]);
    }
    let ends = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let surface = NurbsSurface::new(2, 2, knots, ends.clone(), 9, 3, net.clone(), weights).unwrap();
    let seam = NurbsCurve::new(2, ends, net[..3].to_vec(), vec![1.0; 3]).unwrap();
    let mut body = Body::new(BodyKind::Solid);
    let (bottom, top) = (body.add_vertex(low), body.add_vertex(high));
    let seam = body.add_edge(Curve3::Nurbs(seam), (0.0, 1.0), bottom, top);
    let outline = body.add_loop(vec![forward(seam), backward(seam)]);
    let face = body.add_face(Surface::Nurbs(surface), false, vec![outline]);
    body.add_shell(vec![face]);
    body
}

/// What keeps `mesh` from being the surface of a solid, if anything: its
/// vertices welded by their bits, a side not used once each way, a vertex
/// whose triangles make more than one fan, or two vertices within
/// `1e-12 size` of each other.
fn not_a_closed_manifold(mesh: &Mesh, size: f64) -> Option<String> {
    let welded = match mesh.weld() {
        Ok(welded) => welded,
        Err(t) => return Some(format!("triangle {t} has a corner that does not exist")),
    };
    let at = |k: u32| {
        let [x, y, z] = welded.positions[k as usize];
        Vec3::new(x, y, z)
    };
    if welded.triangles.is_empty() {
        return Some("no triangles".to_string());
    }
    let mut sides: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    // Per vertex, the corner that follows each corner round it.
    let mut round: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for &[a, b, c] in &welded.triangles {
        for (from, to, other) in [(a, b, c), (b, c, a), (c, a, b)] {
            *sides.entry((from, to)).or_insert(0) += 1;
            round.insert((from, to), other);
        }
    }
    for (&(a, b), &n) in &sides {
        let back = sides.get(&(b, a)).copied().unwrap_or(0);
        if n != 1 || back != 1 {
            return Some(format!(
                "the side from {:?} to {:?} is used {n} times that way and {back} the other",
                at(a),
                at(b)
            ));
        }
    }
    // Round each vertex: from one neighbour to the next, as many steps as
    // it has triangles, back where it started.
    let mut fans: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for (&(vertex, from), &other) in &round {
        fans.entry(vertex).or_default().push((from, other));
    }
    for (vertex, steps) in &fans {
        let next: BTreeMap<u32, u32> = steps.iter().copied().collect();
        let start = steps[0].0;
        let (mut here, mut count) = (start, 0);
        loop {
            here = next[&here];
            count += 1;
            if here == start || count > steps.len() {
                break;
            }
        }
        if count != steps.len() {
            return Some(format!(
                "the triangles round {:?} make more than one fan ({count} of {})",
                at(*vertex),
                steps.len()
            ));
        }
    }
    let near = 1e-12 * size;
    let mut order: Vec<u32> = (0..welded.positions.len() as u32).collect();
    order.sort_by(|&i, &j| at(i).x.total_cmp(&at(j).x));
    for (n, &i) in order.iter().enumerate() {
        for &j in &order[n + 1..] {
            if at(j).x - at(i).x > near {
                break;
            }
            if at(i).distance(at(j)) <= near {
                return Some(format!(
                    "two vertices {:e} apart at {:?}",
                    at(i).distance(at(j)),
                    at(i)
                ));
            }
        }
    }
    None
}

/// FNV-1a over every bit of the mesh.
fn digest(mesh: &Mesh) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |x: u64| {
        for byte in x.to_le_bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for p in mesh.positions.iter().chain(&mesh.normals) {
        for x in [p.x, p.y, p.z] {
            eat(x.to_bits());
        }
    }
    for t in &mesh.triangles {
        for &v in t {
            eat(u64::from(v));
        }
    }
    for &f in &mesh.face_ids {
        eat(u64::from(f));
    }
    h
}

/// What a body is held to when meshed.
struct Held {
    /// Its volume and area.
    volume: f64,
    area: f64,
    /// Its size, for the distance at which two vertices are one place.
    size: f64,
    /// A smooth tip and the normal of its plane: every normal the mesh has
    /// there within 0.99 of it.
    tip: Option<(Vec3, Vec3)>,
}

/// `body` meshed at `chord` on one thread, and on two and all when
/// `threads`: `Ok` with its triangles where it is a closed manifold within
/// the chord bound of its volume, with its tip's normal and the same bits
/// on every thread count; `Err` with what is wrong otherwise, and with the
/// tessellator's reason where it refuses (`refused` true).
fn meshed(
    body: &Body,
    (chord, angle): (f64, Option<f64>),
    held: &Held,
    threads: bool,
) -> Result<usize, (bool, String)> {
    let params = TessParams {
        chordal_tolerance: chord,
        max_angle: angle,
        ..TessParams::default()
    };
    let mesh = tessellate_body_with_threads(body, &params, 1).map_err(|e| (true, e.to_string()))?;
    if let Some(what) = not_a_closed_manifold(&mesh, held.size) {
        return Err((false, what));
    }
    let welded = mesh.weld().unwrap();
    let off = (welded.volume() - held.volume).abs();
    if off > held.area * chord {
        return Err((
            false,
            format!(
                "the volume {} is {off:e} off {}, past the chord bound {:e}",
                welded.volume(),
                held.volume,
                held.area * chord
            ),
        ));
    }
    if let Some((tip, normal)) = held.tip {
        let mut at_tip = 0;
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            if p.distance(tip) <= 1e-9 * held.size && n.length() > 0.0 {
                at_tip += 1;
                let cos = n.normalized().unwrap().dot(normal);
                // (A NaN fails too.)
                if cos.partial_cmp(&0.99) == Some(std::cmp::Ordering::Less) || cos.is_nan() {
                    return Err((
                        false,
                        format!("a normal at the tip {cos:.4} against its plane"),
                    ));
                }
            }
        }
        if at_tip == 0 {
            return Err((false, "no normal at the tip".to_string()));
        }
    }
    if threads {
        let one = digest(&mesh);
        for count in [2, 0] {
            let other = tessellate_body_with_threads(body, &params, count)
                .map_err(|e| (false, format!("on {count} threads: refused: {e}")))?;
            if digest(&other) != one {
                return Err((false, format!("another mesh on {count} threads")));
            }
        }
    }
    Ok(mesh.triangles.len())
}

/// Every case meshed: none refused, none wrong; the triangles of all.
fn all_made(cases: &[(String, Body, Held)], chords: &[f64], threads: bool) -> usize {
    let (triangles, refused) = made_or_refused(cases, chords, None, threads);
    assert!(refused.is_empty(), "{refused:#?}");
    triangles
}

/// Every case meshed with the angle criterion `angle`: none wrong; the
/// triangles of all, and what was refused.
fn made_or_refused(
    cases: &[(String, Body, Held)],
    chords: &[f64],
    angle: Option<f64>,
    threads: bool,
) -> (usize, Vec<String>) {
    let (mut faults, mut refusals) = (Vec::new(), Vec::new());
    let mut triangles = 0;
    for (what, body, held) in cases {
        for &chord in chords {
            match meshed(body, (chord * held.size, angle), held, threads) {
                Ok(count) => triangles += count,
                Err((true, why)) => {
                    refusals.push(format!("{what} at a chord of {chord} of its size: {why}"));
                }
                Err((false, why)) => {
                    faults.push(format!("{what} at a chord of {chord} of its size: {why}"));
                }
            }
        }
    }
    println!(
        "{} bodies at {} chords, angle {angle:?}: {triangles} triangles, {} faults, {} refused",
        cases.len(),
        chords.len(),
        faults.len(),
        refusals.len()
    );
    assert!(faults.is_empty(), "{faults:#?}");
    (triangles, refusals)
}

/// The domes of the reviews' families at `k` in `frame`.
fn domes(k: f64, frame_name: &str, frame: &Frame) -> Vec<(String, Body, Held)> {
    let mut cases = Vec::new();
    for (a, b) in [(1.0, 1.0), (1.0, 0.1), (1.0, 0.01), (0.3, 0.003)] {
        for x in [0.0, 0.5, 0.9, 0.99] {
            for flip in [false, true] {
                let length = 0.5;
                let (a, b, length) = (a * k, b * k, length * k);
                let foot = [x * a, 0.0];
                let body = seamed_dome(frame, (a, b), flip, foot, length);
                let volume = 11.0 / 15.0 * PI * a * b * length;
                // Its area as measured in the model's frame (the measure
                // refuses some of these far from the origin).
                let (mass, _) =
                    mass_bound(&seamed_dome(&Frame::WORLD, (a, b), flip, foot, length)).unwrap();
                assert!(
                    (mass.volume - volume).abs() <= 1e-12 * volume,
                    "the dome {a} x {b} over {x}: {} against {volume}",
                    mass.volume
                );
                cases.push((
                    format!(
                        "the dome {a:e} x {b:e} with its tip over {x} of its half axis, its seam on \
                         {}x, in the frame {frame_name}",
                        if flip { "-" } else { "+" }
                    ),
                    body,
                    Held {
                        volume,
                        area: mass.area,
                        size: 2.0 * a,
                        tip: Some((frame.point(foot[0], foot[1], length), frame.z())),
                    },
                ));
            }
        }
    }
    cases
}

/// The coarsest to the finest chord, as shares of the size.
const CHORDS: [f64; 5] = [1e-1, 2e-2, 1e-2, 2e-3, 1e-3];

#[test]
fn domes_closed_across_a_seam_mesh_as_closed_manifolds_in_the_model_s_frame() {
    // Until 2026-11-15, 27 of these 160 meshes (28 of the 160 in the
    // turned frame, 21 of the 96 at 1e-3) had points within 1e-12 of each
    // other along the seam, sides used other than once each way, or
    // vertices where two fans meet, all without an error.
    let cases = domes(1.0, "world", &Frame::WORLD);
    all_made(&cases, &CHORDS, true);
}

#[test]
fn domes_closed_across_a_seam_mesh_as_closed_manifolds_turned_and_far() {
    for (name, frame) in frames(1.0).into_iter().skip(1) {
        all_made(&domes(1.0, name, &frame), &CHORDS, false);
    }
}

#[test]
fn domes_closed_across_a_seam_mesh_as_closed_manifolds_small_and_large() {
    for k in [1e-3, 1e6] {
        for (name, frame) in frames(k) {
            all_made(&domes(k, name, &frame), &[1e-1, 1e-2, 2e-3], false);
        }
    }
}

#[test]
fn domes_closed_across_a_seam_mesh_as_closed_manifolds_with_the_export_s_angles() {
    // The angles of `file.export`'s `coarse` and `normal` qualities (20
    // and 10 degrees) at their tolerances (2 and 1 thousandths of the side
    // of a square with the body's area, 1.3 for these domes), on the thin
    // domes. Refusals are counted, not failed: the angle is a wish, which
    // a face along so sharp a nose cannot always meet (0.1.x refused
    // `coarse` for half of such domes too). In the model's frame only: in
    // a turned one such a dome takes 15 to 100 s at 20 degrees, before
    // 2026-11-15 as after, and its STL is known to fold in single
    // precision (`crates/engine/tests/pole_fan.rs`).
    let mut refused = Vec::new();
    for (name, frame) in frames(1.0).into_iter().take(1) {
        let cases: Vec<_> = domes(1.0, name, &frame)
            .into_iter()
            .filter(|(what, ..)| what.contains("x 1e-2") || what.contains("x 3e-3"))
            .filter(|(what, ..)| what.contains("over 0.5 ") || what.contains("over 0.9 "))
            .collect();
        for (angle, chord) in [(20.0_f64, 1.3e-3), (10.0, 6.5e-4)] {
            let (_, out) = made_or_refused(&cases, &[chord], Some(angle.to_radians()), false);
            refused.extend(out);
        }
    }
    println!("refused: {refused:#?}");
}

#[test]
fn the_dome_found_without_a_manifold_is_one() {
    // The dome of the finding (crates/kernel/tests/pole_measure.rs): at
    // 0.002, 12,004 triangles with three vertices within 1e-14 of each
    // other 4e-3 from the tip whose sides were used twice each way; at
    // 0.02, 1,378 with one side shared by four triangles.
    let frame = Frame::new(
        Vec3::new(3.0, -2.0, 5.0),
        Vec3::new(2.0, -3.0, 6.0),
        Vec3::new(1.0, 0.3, 0.0),
    )
    .unwrap();
    for (flip, chord) in [(false, 0.002), (true, 0.02)] {
        let body = seamed_dome(&frame, (1.0, 0.01), flip, [0.9, 0.0], 0.5);
        let held = Held {
            volume: 11.0 / 15.0 * PI * 0.01 * 0.5,
            area: mass_bound(&body).unwrap().0.area,
            size: 2.0,
            tip: Some((frame.point(0.9, 0.0, 0.5), frame.z())),
        };
        let made = meshed(&body, (chord, None), &held, true)
            .unwrap_or_else(|(_, e)| panic!("flip {flip}, at {chord}: {e}"));
        println!("the dome, its seam flipped {flip}, at {chord}: {made} triangles");
    }
}

#[test]
fn spindles_closed_across_a_seam_at_both_poles_mesh_as_closed_manifolds() {
    let mut cases = Vec::new();
    for k in SCALES {
        for (name, frame) in frames(k) {
            for (a, b) in [(1.0, 1.0), (1.0, 0.1), (1.0, 0.01)] {
                for flip in [false, true] {
                    let (a, b, height) = (a * k, b * k, 2.0 * k);
                    let body = seamed_spindle(&frame, (a, b), flip, height);
                    let tolerance = Tolerance::new(1e-7 * k, 1e-9).unwrap();
                    assert_eq!(check(&body, &tolerance), [], "the spindle {a} x {b}");
                    let volume = 2.0 * PI * a * b * height / 15.0;
                    let (mass, _) = mass_bound(&body).unwrap();
                    assert!(
                        (mass.volume - volume).abs() <= 1e-12 * volume,
                        "the spindle {a} x {b}: {} against {volume}",
                        mass.volume
                    );
                    cases.push((
                        format!("the spindle {a:e} x {b:e}, flipped {flip}, in {name}"),
                        body,
                        Held {
                            volume,
                            area: mass.area,
                            size: height,
                            // Pointed at both poles: each ruling its own
                            // normal there.
                            tip: None,
                        },
                    ));
                }
            }
        }
    }
    all_made(&cases, &[1e-1, 1e-2, 2e-3], false);
}

/// A seeded stream (xorshift: the same on every machine).
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn between(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A seeded spline profile from the axis at (0, 0), rising all the way:
/// to a second point of the axis (two poles) or to a flat top (one pole),
/// leaving the axis square (a smooth pole), at a slant (a point) or nearly
/// along it (a needle). Degree, control points, a name. (The second
/// review's generator, 2026-11-15.)
fn profile(rng: &mut Seeded) -> (usize, Vec<[f64; 2]>, String) {
    let degree = 2 + rng.below(4) as usize;
    let count = degree + 1 + rng.below(5) as usize;
    let spindle = rng.below(2) == 0;
    let height = rng.between(1.0, 4.0);
    let ends = ["square", "slant", "needle"];
    let (low, high) = (rng.below(3) as usize, rng.below(3) as usize);
    let mut control = vec![[0.0, 0.0]];
    let inner = count - 2;
    for i in 0..inner {
        let z = height * (i as f64 + rng.between(0.3, 0.7)) / inner as f64;
        control.push([rng.between(0.4, 1.6), z]);
    }
    let first = control[1];
    control[1] = match ends[low] {
        "square" => [first[0], 0.0],
        "slant" => [first[0], first[1].max(0.3 * first[0])],
        _ => [0.02 * first[0].max(0.5), first[1].max(0.3)],
    };
    if spindle {
        let last = control[inner];
        control[inner] = match ends[high] {
            "square" => [last[0], height],
            "slant" => [
                last[0],
                last[1]
                    .min(height - 0.3 * last[0])
                    .max(control[inner - 1][1]),
            ],
            _ => [
                0.02 * last[0].max(0.5),
                last[1].min(height - 0.3).max(control[inner - 1][1]),
            ],
        };
        control.push([0.0, height]);
    } else {
        control.push([rng.between(0.3, 1.2), height]);
    }
    for i in 1..control.len() {
        if control[i][1] < control[i - 1][1] {
            control[i][1] = control[i - 1][1];
        }
    }
    let name = format!(
        "degree {degree}, {count} points, {} from a {} end to a {} one",
        if spindle { "a spindle" } else { "a drop" },
        ends[low],
        if spindle { ends[high] } else { "flat" }
    );
    (degree, control, name)
}

/// The solid the profile turns from `from` to `to` about the y axis of
/// `plane`, at scale `k`.
fn turned(
    (degree, control): (usize, &[[f64; 2]]),
    k: f64,
    plane: &Frame,
    (from, to): (f64, f64),
) -> Body {
    let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
    let control: Vec<[f64; 2]> = control.iter().map(|&[r, z]| [r * k, z * k]).collect();
    let spans = control.len() - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    let (first, last) = (control[0], *control.last().unwrap());
    let mut curves = vec![SketchCurve {
        id: 3,
        curve: Curve2::Spline(Spline2 {
            degree,
            knots,
            control,
        }),
    }];
    let mut id = 5;
    let mut line = |start: [f64; 2], end: [f64; 2]| {
        curves.push(SketchCurve {
            id,
            curve: Curve2::Line { start, end },
        });
        id += 2;
    };
    if last[0] != 0.0 {
        line(last, [0.0, last[1]]);
    }
    line([0.0, last[1]], first);
    let profiles = detect_profiles(&curves, &tolerance).unwrap();
    revolve(
        &profiles,
        plane,
        [0.0, 0.0],
        [0.0, 1.0],
        from,
        to,
        &tolerance,
    )
    .unwrap()
    .body
}

#[test]
fn spline_profiles_turned_onto_their_axis_mesh_as_closed_manifolds() {
    let mut cases = Vec::new();
    for seed in 0..12_u64 {
        let mut rng =
            Seeded(0x9e37_79b9_7f4a_7c15 ^ (seed + 1).wrapping_mul(0x2545_f491_4f6c_dd1d));
        let (degree, control, name) = profile(&mut rng);
        let height = control.last().unwrap()[1];
        for (from, to) in [(0.0, TAU), (0.0, 1.3 * PI), (-0.4, 0.9)] {
            for k in SCALES {
                for (frame_name, frame) in frames(k) {
                    if k != 1.0 && frame_name == "world" {
                        continue;
                    }
                    let body = turned((degree, &control), k, &frame, (from, to));
                    let (mass, _) = mass_bound(&body).unwrap();
                    cases.push((
                        format!(
                            "seed {seed}: {name}, turned {from}..{to:.4}, at {k} in {frame_name}"
                        ),
                        body,
                        Held {
                            volume: mass.volume,
                            area: mass.area,
                            size: height.max(2.0) * k,
                            tip: None,
                        },
                    ));
                }
            }
        }
    }
    all_made(&cases, &[1e-2, 1e-3], false);
}

#[test]
fn the_reference_solids_with_poles_mesh_as_closed_manifolds() {
    let mut cases = Vec::new();
    for k in SCALES {
        for (frame_name, frame) in frames(k) {
            for reference in [
                solids::sphere(&frame, 2.5 * k).unwrap(),
                solids::hemisphere(&frame, 2.0 * k).unwrap(),
                solids::cone(&frame, 2.0 * k, 3.0 * k).unwrap(),
                solids::spline_cone(&frame, 2.0 * k, 3.0 * k).unwrap(),
            ] {
                cases.push((
                    format!("the {} at {k} in {frame_name}", reference.name),
                    reference.body,
                    Held {
                        volume: reference.volume,
                        area: reference.area,
                        size: 5.0 * k,
                        tip: None,
                    },
                ));
            }
        }
    }
    all_made(&cases, &[1e-1, 1e-2, 1e-3], true);
}

/// `body` meshed at chords from coarse to fine: an error or a closed
/// manifold, never a panic, never a mesh that is neither.
fn an_error_or_a_closed_manifold(body: &Body, size: f64, chords: &[f64], what: &str) {
    for &chord in chords {
        let params = TessParams {
            chordal_tolerance: chord * size,
            ..TessParams::default()
        };
        let started = std::time::Instant::now();
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tessellate_body(body, &params)
        }));
        let took = started.elapsed().as_secs_f64();
        match made {
            Err(_) => panic!("{what}: a panic at {chord}"),
            Ok(Err(error)) => println!("{what} at {chord}: refused in {took:.2} s: {error}"),
            Ok(Ok(mesh)) => {
                if let Some(fault) = not_a_closed_manifold(&mesh, size) {
                    panic!("{what} at {chord}: meshed, and {fault}");
                }
                println!(
                    "{what} at {chord}: {} triangles in {took:.2} s",
                    mesh.triangles.len()
                );
            }
        }
    }
}

/// Chords from coarse to fine, as shares of the size.
const ALL: [f64; 3] = [1e-1, 1e-2, 1e-3];

#[test]
fn hostile_poles_are_an_error_or_a_closed_manifold() {
    // A dome whose every row is the tip (the face all pole).
    let frame = Frame::WORLD;
    let mut body = seamed_dome(&frame, (1.0, 0.01), false, [0.9, 0.0], 0.5);
    if let Surface::Nurbs(surface) = &body.faces[0].surface {
        let top = frame.point(0.9, 0.0, 0.5);
        let all = NurbsSurface::new(
            2,
            2,
            surface.knots_u().to_vec(),
            surface.knots_v().to_vec(),
            9,
            3,
            vec![top; 27],
            vec![1.0; 27],
        );
        if let Ok(all) = all {
            body.faces[0].surface = Surface::Nurbs(all);
        }
    }
    an_error_or_a_closed_manifold(&body, 2.0, &ALL, "a dome all pole");
    // The tip moved off the face's surface, and to no place at all.
    for tip in [
        Vec3::new(0.9, 0.0, 0.7),
        Vec3::new(f64::NAN, 0.0, 0.5),
        Vec3::new(f64::INFINITY, 0.0, 0.5),
    ] {
        let mut body = seamed_dome(&frame, (1.0, 0.01), false, [0.9, 0.0], 0.5);
        body.vertices[1].point = tip;
        an_error_or_a_closed_manifold(&body, 2.0, &ALL, &format!("a dome with its tip at {tip:?}"));
    }
    // The seam's curve not finite.
    let mut body = seamed_spindle(&frame, (1.0, 0.01), true, 2.0);
    body.vertices[0].point = Vec3::new(0.0, f64::NAN, 0.0);
    an_error_or_a_closed_manifold(&body, 2.0, &ALL, "a spindle with a pole not finite");
    // Spindles a millionth and a billionth as wide as they are long, at
    // the coarsest chord only: at a hundredth of its size the one a
    // billionth wide takes 10 s to mesh before 2026-11-15 and 13 s after,
    // and 39 s to be refused at a thousandth, either way.
    let body = seamed_spindle(&frame, (1.0, 1e-6), false, 2.0);
    an_error_or_a_closed_manifold(&body, 2.0, &[1e-1], "a spindle 1e-6 wide");
    let body = seamed_spindle(&frame, (1.0, 1e-9), false, 2.0);
    an_error_or_a_closed_manifold(&body, 2.0, &[1e-1], "a spindle 1e-9 wide");
}
