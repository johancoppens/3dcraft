//! Evidence for the developer to look at: Jansen's leg and a four-bar
//! solved over a crank turn, drawn as SVG (bars at a few frames, the foot
//! path or coupler curve as a thick line) and a strip of the leg at 0°,
//! 90°, 180° and 270°. Written to `target/evidence/asm-solver/` and
//! printed between `EVIDENCE-SVG-BEGIN <name>` / `EVIDENCE-SVG-END`.
//! `cargo test -p craft3d-assembly --test evidence -- --ignored --nocapture`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::*;
use craft3d_assembly::{Solution, Solver};
use craft3d_math::Vec3;
use std::f64::consts::PI;
use std::fmt::Write as _;

/// Jansen's bars as pairs of joints (O A B C D E F G = 0..7).
const BARS: [(usize, usize); 11] = [
    (0, 1),
    (1, 3),
    (1, 5),
    (2, 3),
    (2, 4),
    (3, 4),
    (2, 5),
    (4, 6),
    (5, 6),
    (6, 7),
    (5, 7),
];

/// One Jansen leg on a crank; instance per joint that carries it.
fn leg() -> (craft3d_assembly::Problem, [Vec3; 8]) {
    let mut p = planar(8, 1.0, 100.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "crank");
    p.mates[0].driven[0] = Some(0.0);
    add_leg(&mut p, 1, 2, 0.0, 1.0, 0.0);
    disturb(&mut p, 0.01);
    (p, jansen_joints(0.0, 1.0, 0.0).unwrap())
}

/// The joints of a solved leg: each through a body that carries it.
fn leg_joints(sol: &Solution, r: &[Vec3; 8]) -> [Vec3; 8] {
    // O, B ground; A crank; C bar j; D hip; E bar c; F bar f; G foot.
    let body = [0, 1, 0, 2, 4, 5, 6, 7];
    std::array::from_fn(|k| sol.poses[body[k]].point(r[k]))
}

struct Svg {
    body: String,
    min: (f64, f64),
    max: (f64, f64),
}

impl Svg {
    fn new() -> Svg {
        Svg {
            body: String::new(),
            min: (f64::MAX, f64::MAX),
            max: (f64::MIN, f64::MIN),
        }
    }
    fn grow(&mut self, p: Vec3) {
        self.min = (self.min.0.min(p.x), self.min.1.min(-p.y));
        self.max = (self.max.0.max(p.x), self.max.1.max(-p.y));
    }
    fn line(&mut self, a: Vec3, b: Vec3, colour: &str, width: f64) {
        self.grow(a);
        self.grow(b);
        let _ = writeln!(
            self.body,
            r#"<line x1="{:.3}" y1="{:.3}" x2="{:.3}" y2="{:.3}" stroke="{colour}" stroke-width="{width}" stroke-linecap="round"/>"#,
            a.x, -a.y, b.x, -b.y
        );
    }
    fn polyline(&mut self, pts: &[Vec3], colour: &str, width: f64) {
        let mut d = String::new();
        for p in pts {
            self.grow(*p);
            let _ = write!(d, "{:.3},{:.3} ", p.x, -p.y);
        }
        let _ = writeln!(
            self.body,
            r#"<polyline points="{d}" fill="none" stroke="{colour}" stroke-width="{width}" stroke-linejoin="round"/>"#
        );
    }
    fn dot(&mut self, p: Vec3, colour: &str, r: f64) {
        self.grow(p);
        let _ = writeln!(
            self.body,
            r#"<circle cx="{:.3}" cy="{:.3}" r="{r}" fill="{colour}"/>"#,
            p.x, -p.y
        );
    }
    fn text(&mut self, p: Vec3, s: &str, size: f64) {
        self.grow(p);
        let _ = writeln!(
            self.body,
            r#"<text x="{:.3}" y="{:.3}" font-family="sans-serif" font-size="{size}" fill="dimgray">{s}</text>"#,
            p.x, -p.y
        );
    }
    fn finish(&self, margin: f64) -> String {
        let (x0, y0) = (self.min.0 - margin, self.min.1 - margin);
        let (w, h) = (
            self.max.0 - self.min.0 + 2.0 * margin,
            self.max.1 - self.min.1 + 2.0 * margin,
        );
        let px = 900.0;
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x0:.3} {y0:.3} {w:.3} {h:.3}\" width=\"{px}\" height=\"{:.0}\">\n<rect x=\"{x0:.3}\" y=\"{y0:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" fill=\"white\"/>\n{}</svg>\n",
            px * h / w,
            self.body
        )
    }
}

/// A shade from blue to orange for frame `k` of `n`.
fn shade(k: usize, n: usize) -> String {
    let t = k as f64 / (n.max(2) - 1) as f64;
    let (r, g, b) = (40.0 + 200.0 * t, 90.0 + 60.0 * t, 200.0 - 170.0 * t);
    format!("rgb({r:.0},{g:.0},{b:.0})")
}

fn emit(name: &str, svg: &str) {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/asm-solver");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(name), svg);
    println!("EVIDENCE-SVG-BEGIN {name}");
    print!("{svg}");
    println!("EVIDENCE-SVG-END");
}

fn solve_turn(p: &craft3d_assembly::Problem, frames: usize) -> Vec<Solution> {
    let mut s = Solver::new(p).unwrap();
    let mut out = vec![s.solve().unwrap()];
    for k in 1..=frames {
        out.push(s.advance(&[2.0 * PI * k as f64 / frames as f64]).unwrap());
    }
    out
}

#[test]
#[ignore]
fn evidence_jansen_leg_and_four_bar() {
    // (a, b) Jansen's leg: bars at 8 frames, the foot path over the turn.
    let (p, r) = leg();
    let sols = solve_turn(&p, 72);
    let mut svg = Svg::new();
    let path: Vec<Vec3> = sols.iter().map(|s| leg_joints(s, &r)[7]).collect();
    for (n, k) in (0..72).step_by(9).enumerate() {
        let j = leg_joints(&sols[k], &r);
        for (a, b) in BARS {
            svg.line(j[a], j[b], &shade(n, 8), 0.6);
        }
    }
    svg.polyline(&path, "firebrick", 2.2);
    for k in [0, 2] {
        svg.dot(r[k], "black", 1.5);
    }
    svg.text(
        Vec3::new(-105.0, -102.0, 0.0),
        "Jansen leg, 72 frames: bars every 45 deg, foot path in red",
        4.0,
    );
    emit("jansen-leg.svg", &svg.finish(8.0));

    // The strip: the leg at 0, 90, 180 and 270 degrees, side by side.
    let mut strip = Svg::new();
    for (n, k) in [0usize, 18, 36, 54].iter().enumerate() {
        let shift = Vec3::new(130.0 * n as f64, 0.0, 0.0);
        let j = leg_joints(&sols[*k], &r);
        strip.polyline(
            &path.iter().map(|q| *q + shift).collect::<Vec<_>>(),
            "mistyrose",
            1.0,
        );
        for (a, b) in BARS {
            strip.line(j[a] + shift, j[b] + shift, "darkslategray", 1.2);
        }
        for q in j {
            strip.dot(q + shift, "darkslategray", 1.2);
        }
        strip.dot(j[7] + shift, "firebrick", 2.2);
        strip.text(
            Vec3::new(-60.0, -104.0, 0.0) + shift,
            &format!("crank {} deg", 90 * n),
            6.0,
        );
    }
    emit("jansen-strip.svg", &strip.finish(8.0));

    // (c) A crank-rocker four-bar (a 1, b 4, c 3, d 3.5): the coupler curve.
    let (a, b, c, d) = (1.0, 4.0, 3.0, 3.5);
    let th0 = 60f64.to_radians();
    let a0 = v2(a * th0.pcos(), a * th0.psin());
    let b0 = circ(a0, b, v2(d, 0.0), c, 1.0).unwrap();
    let u = (b0 - a0) * (1.0 / b);
    let p0 = a0 + u * (0.5 * b) + Vec3::new(-u.y, u.x, 0.0) * (0.4 * b);
    let mut fp = planar(4, 1.0, 4.0);
    pin(&mut fp, 0, 1, Vec3::ZERO, "crank");
    pin(&mut fp, 1, 2, a0, "A");
    pin(&mut fp, 2, 3, b0, "B");
    pin(&mut fp, 3, 0, v2(d, 0.0), "D");
    fp.mates[0].driven[0] = Some(0.0);
    let sols = solve_turn(&fp, 72);
    let mut svg = Svg::new();
    let curve: Vec<Vec3> = sols.iter().map(|s| s.poses[2].point(p0)).collect();
    for (n, k) in (0..72).step_by(9).enumerate() {
        let s = &sols[k];
        let (pa, pb, pp) = (
            s.poses[1].point(a0),
            s.poses[3].point(b0),
            s.poses[2].point(p0),
        );
        let col = shade(n, 8);
        svg.line(Vec3::ZERO, pa, &col, 0.03);
        svg.line(pa, pb, &col, 0.03);
        svg.line(pb, v2(d, 0.0), &col, 0.03);
        svg.line(pa, pp, &col, 0.015);
        svg.line(pb, pp, &col, 0.015);
    }
    svg.polyline(&curve, "firebrick", 0.08);
    svg.dot(Vec3::ZERO, "black", 0.07);
    svg.dot(v2(d, 0.0), "black", 0.07);
    svg.text(
        Vec3::new(-1.0, -1.2, 0.0),
        "Crank-rocker four-bar 1 / 4 / 3 / 3.5: coupler curve in red",
        0.18,
    );
    emit("four-bar-coupler.svg", &svg.finish(0.4));
}
