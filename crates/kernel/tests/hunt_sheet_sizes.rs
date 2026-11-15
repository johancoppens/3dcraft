//! "Builds at one value, fails a billionth away": plates and sheet-metal
//! parts with one dimension stepped through values 1e-9 apart (and some
//! through a wide range), each built part held to its closed-form volume
//! (and flat area) and `topo::check`; where the outcome changes along a
//! sweep the values are printed with the reason, and the change is held
//! to where a real limit lies (a piece shorter than the tolerance, a web
//! of no length, a flat of no length). The gated sweeps take 200 values
//! each; the 2000-value ones are `#[ignore]`d.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_sheet_common;

use craft3d_kernel::extrude::fill;
use craft3d_kernel::plate::{Side, plate};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Direction, FlangeSpec, JogSpec, LengthFrom, On, Spec, build, flat_area,
};
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{Body, FaceId, check};
use hunt_sheet_common::*;
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// The sweeps known to build wrong or invalid bodies at some of their
/// values, by name: each a finding, reduced in
/// crates/kernel/tests/hunt_sheet_repro.rs; a fix has to take it off.
/// None since 2026-11-15: the bend down at 140 degrees about
/// `R = 0.1763 T`, where the traced run of its bottom skin's arcs was
/// four whole turns, and the bends down below `R = 0.02 T`, refused, are
/// their closed forms (`a_bend_down_at_one_radius_is_its_closed_form`).
const KNOWN_WRONG: &[&str] = &[];

/// A body built (with its flat area and the closed form's, for a sheet-
/// metal part) or why not, or a panic's message.
type Made = Result<Result<(Body, Option<(f64, f64)>), String>, String>;

/// What one value gave: built right (`B`), built wrong (`W`), invalid
/// (`I`), refused (`R`, with the reason), a panic (`P`).
struct Sweep {
    name: String,
    values: Vec<f64>,
    outcomes: Vec<char>,
    reasons: Vec<String>,
    wrong: Vec<String>,
}

impl Sweep {
    fn new(name: &str) -> Self {
        Sweep {
            name: name.into(),
            values: Vec::new(),
            outcomes: Vec::new(),
            reasons: Vec::new(),
            wrong: Vec::new(),
        }
    }

    /// Records one value: the body built (or why not), the closed-form
    /// volume and flat area (if any) and the kernel's flat area.
    fn record(&mut self, value: f64, made: Made, volume: f64, tol: &Tolerance, mesh_it: bool) {
        self.values.push(value);
        let (c, why) = match made {
            Err(p) => ('P', p),
            Ok(Err(e)) => ('R', e),
            Ok(Ok((body, flat))) => {
                let mut found = Found::default();
                let m = if mesh_it {
                    sound(&body, tol, &mut found)
                } else if check(&body, tol).is_empty() {
                    let m = measured(&body);
                    if m.is_none() {
                        found
                            .problems
                            .push("passes the checker but cannot be measured".into());
                    }
                    m
                } else {
                    found.invalid = true;
                    None
                };
                if let Some(m) = m {
                    let slack = m.bound
                        + 1e-9 * volume
                        + if any_freeform(&body) {
                            0.5 * tol.linear * m.area
                        } else {
                            0.0
                        };
                    if (m.volume - volume).abs() > slack {
                        found.problems.push(format!(
                            "volume {:.17e} against {volume:.17e} ({:.3e} of it)",
                            m.volume,
                            m.volume / volume - 1.0
                        ));
                    }
                }
                if let Some((got, want)) = flat
                    && (got - want).abs() > 1e-9 * want
                {
                    found
                        .problems
                        .push(format!("flat area {got:.17e} against {want:.17e}"));
                }
                if found.invalid {
                    ('I', format!("{:?}", found.problems))
                } else if !found.problems.is_empty() {
                    ('W', format!("{:?}", found.problems))
                } else {
                    ('B', String::new())
                }
            }
        };
        if matches!(c, 'W' | 'I' | 'P') {
            self.wrong
                .push(format!("{} at {value:.17e}: {c}: {why}", self.name));
        }
        self.outcomes.push(c);
        self.reasons.push(why);
    }

    /// Prints where the outcome changes, and returns the runs as
    /// (outcome, first value, last value, count, reason of the first).
    fn runs(&self) -> Vec<(char, f64, f64, usize, String)> {
        let mut out: Vec<(char, f64, f64, usize, String)> = Vec::new();
        for i in 0..self.values.len() {
            let (c, v) = (self.outcomes[i], self.values[i]);
            let short = shortened(&self.reasons[i], 150);
            match out.last_mut() {
                Some(run) if run.0 == c && (c != 'R' || run.4 == short) => {
                    run.2 = v;
                    run.3 += 1;
                }
                _ => out.push((c, v, v, 1, short)),
            }
        }
        let built = self.outcomes.iter().filter(|c| **c == 'B').count();
        let refused = self.outcomes.iter().filter(|c| **c == 'R').count();
        println!(
            "{}: {} values, built right {built}, refused {refused}, wrong {}, in {} runs",
            self.name,
            self.values.len(),
            self.wrong.len(),
            out.len()
        );
        for (c, a, b, n, why) in out.iter().take(40) {
            println!("  {c} x{n} from {a:.17e} to {b:.17e} {why}");
        }
        for w in &self.wrong {
            println!("  WRONG {w}");
        }
        out
    }

    /// Holds the sweep: nothing wrong, invalid or a panic (but the known
    /// values), and at most `changes` changes of outcome.
    fn hold(&self, changes: usize) {
        let runs = self.runs();
        if KNOWN_WRONG.contains(&self.name.as_str()) {
            // Known to build wrong bodies at some of its values (a finding:
            // crates/kernel/tests/hunt_sheet_repro.rs has it reduced): not
            // held here; the gated test holds the sweep about the value to
            // still being wrong.
            return;
        }
        assert!(self.wrong.is_empty(), "{}: {:#?}", self.name, self.wrong);
        assert!(
            runs.len() <= changes + 1,
            "{}: the outcome changes {} times along the sweep",
            self.name,
            runs.len() - 1
        );
    }
}

fn line(id: u64, start: P2, end: P2) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

/// A plate of `shape` at scale 1, 1 thick, its body.
fn plated(shape: &Shape, tol: &Tolerance) -> Result<Body, String> {
    let placing = Placing {
        turn: 0.3,
        k: 1.0,
        at: [0.0, 0.0],
        flip_lines: false,
    };
    let curves = sketch(shape, &placing, 1);
    let profiles = detect_profiles(&curves, tol).map_err(|e| format!("profile: {e}"))?;
    let sheet = fill(&profiles, &Frame::WORLD, tol)
        .map_err(|e| format!("fill: {e}"))?
        .body;
    let faces: Vec<FaceId> = (0..sheet.faces.len() as u32).map(FaceId).collect();
    plate(&sheet, &faces, 1.0, Side::Out, &BTreeMap::new(), tol)
        .map(|p| p.body)
        .map_err(|e| e.to_string())
}

fn plate_sweep(name: &str, values: &[f64], shape: impl Fn(f64) -> Shape) -> Sweep {
    let tol = tolerance(1.0);
    let mut sweep = Sweep::new(name);
    for (i, &v) in values.iter().enumerate() {
        let s = shape(v);
        let volume = s.area();
        let made = guarded(|| plated(&s, &tol).map(|b| (b, None)));
        sweep.record(v, made, volume, &tol, i % 10 == 0);
    }
    sweep
}

fn steps(centre: f64, step: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| centre + step * (i as f64 - (n / 2) as f64))
        .collect()
}

fn simple(what: String, outer: Vec<Seg>, inner: (P2, f64)) -> Shape {
    Shape {
        what,
        outer,
        holes: Vec::new(),
        inner,
        hole_discs: Vec::new(),
        feature: inner.1,
    }
}

/// The plates: a gear's root radius about 19 (A-019's), a rounded
/// rectangle whose radius runs to half its height (its sides to no
/// length), a disc of three arcs one of whose cuts moves, a slot whose
/// straight sides run to no length.
fn plate_sweeps(n: usize) -> Vec<Sweep> {
    let mut out = Vec::new();
    out.push(plate_sweep(
        "gear root radius",
        &steps(19.0, 1e-9, n),
        |rr| {
            simple(
                format!("gear rr {rr}"),
                gear(16, rr, rr + 3.0, 0.45, 0.1),
                ([0.0, 0.0], 0.9 * rr),
            )
        },
    ));
    out.push(plate_sweep(
        "rounded rectangle to half its height",
        &(0..n).map(|i| 10.0 - 1e-8 * i as f64).collect::<Vec<_>>(),
        |r| {
            simple(
                format!("rounded rectangle 50 x 20 r {r}"),
                rounded_rect(50.0, 20.0, r),
                ([0.0, 0.0], 9.0),
            )
        },
    ));
    out.push(plate_sweep(
        "disc of three arcs, a cut moved",
        &steps(2.0, 1e-9, n),
        |a| {
            simple(
                format!("disc cut at 0.5, {a}, 4.4"),
                split_circle([0.0, 0.0], 19.0, &[0.5, a, 4.4]),
                ([0.0, 0.0], 18.0),
            )
        },
    ));
    out.push(plate_sweep(
        "slot to a circle",
        &(0..n).map(|i| 1e-8 * i as f64).collect::<Vec<_>>(),
        |l| simple(format!("slot l {l}"), slot(l, 5.0, 2), ([0.0, 0.0], 4.5)),
    ));
    out
}

#[test]
fn plates_stepped_by_billionths_change_only_at_real_limits() {
    for s in plate_sweeps(200) {
        // The rounded rectangle's and the slot's sides reach no length
        // within the tolerance: one change each is a real limit.
        s.hold(2);
    }
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn plates_stepped_by_billionths_wide() {
    for s in plate_sweeps(2000) {
        s.hold(4);
    }
}

/// A rectangle 50 × 40 (times `k`) with flanges, `t` thick.
fn rect(k: f64, t: f64, flanges: Vec<FlangeSpec>, jogs: Vec<JogSpec>) -> Spec {
    let c = [
        [0.0, 0.0],
        [50.0 * k, 0.0],
        [50.0 * k, 40.0 * k],
        [0.0, 40.0 * k],
    ];
    let curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    Spec {
        base: detect_profiles(&curves, &tolerance(k)).unwrap().remove(0),
        plane: Frame::WORLD,
        thickness: t,
        flanges,
        relief: None,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts: Vec::new(),
        jogs,
        ids: Vec::new(),
    }
}

fn flange(on: u64, l: f64, th: f64, dir: Direction, r: f64, kk: f64) -> FlangeSpec {
    FlangeSpec {
        on: On::Base(on),
        length: l,
        angle: th,
        direction: dir,
        radius: r,
        k: kk,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

/// Builds `spec`, the solid and the flat area.
fn sheet(spec: &Spec, tol: &Tolerance, flat: f64) -> Made {
    guarded(|| {
        build(spec, tol)
            .map(|p| {
                let a = flat_area(&p);
                (p.solid.body, Some((a, flat)))
            })
            .map_err(|e| e.to_string())
    })
}

/// The sheet-metal sweeps, at scale 1, T = 1 unless said.
fn sheet_sweeps(n: usize) -> Vec<Sweep> {
    let tol = tolerance(1.0);
    let t = 1.0;
    let mut out = Vec::new();
    // An L whose radius is the thickness, give or take billionths.
    let mut s = Sweep::new("L, radius about the thickness");
    for r in steps(t, 1e-9, n) {
        let th = 1.2;
        let kk = 0.4;
        let spec = rect(
            1.0,
            t,
            vec![flange(2, 20.0, th, Direction::Up, r, kk)],
            Vec::new(),
        );
        let v = 50.0 * 40.0 * t + 40.0 * (th * t * (r + 0.5 * t) + 20.0 * t);
        let a = 2000.0 + 40.0 * (th * (r + kk * t) + 20.0);
        s.record(r, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    // An L whose radius runs from 1e-5 to 1 times the thickness: where the
    // plate's trace stops refusing it.
    for (deg, dir) in [
        (69.0f64, Direction::Up),
        (179.0, Direction::Up),
        (90.0, Direction::Down),
        (140.0, Direction::Down),
    ] {
        let mut s = Sweep::new(&format!(
            "L at {deg} degrees {dir:?}, radius 1e-5 to 1 times the thickness"
        ));
        for i in 0..n {
            let r = t * 10f64.ppowf(-5.0 + 5.0 * i as f64 / (n - 1) as f64);
            let th = deg.to_radians();
            let kk = 0.4;
            let spec = rect(1.0, t, vec![flange(2, 20.0, th, dir, r, kk)], Vec::new());
            let v = 50.0 * 40.0 * t + 40.0 * (th * t * (r + 0.5 * t) + 20.0 * t);
            let a = 2000.0 + 40.0 * (th * (r + kk * t) + 20.0);
            s.record(r, sheet(&spec, &tol, a), v, &tol, false);
        }
        out.push(s);
    }
    // A bend down at 140 degrees whose radius is 0.1763 times the
    // thickness, give or take a hundredth of a billionth (the value of
    // the radius sweep above that built a wrong body), and the radius
    // through 0.1 to 0.3 times the thickness.
    for (name, values) in [
        (
            "L at 140 degrees down, radius about 0.1763 T by 1e-11",
            steps(0.1762914118095948, 1e-11, n),
        ),
        (
            "L at 140 degrees down, radius 0.1 to 0.3 T",
            (0..n)
                .map(|i| 0.1 + 0.2 * i as f64 / (n - 1) as f64)
                .collect::<Vec<f64>>(),
        ),
    ] {
        let mut s = Sweep::new(name);
        let th = 140f64.to_radians();
        for r in values {
            let spec = rect(
                1.0,
                t,
                vec![flange(2, 20.0, th, Direction::Down, r * t, 0.4)],
                Vec::new(),
            );
            let v = 2000.0 * t + 40.0 * (th * t * (r * t + 0.5 * t) + 20.0 * t);
            let a = 2000.0 + 40.0 * (th * (r * t + 0.4 * t) + 20.0);
            s.record(r, sheet(&spec, &tol, a), v, &tol, false);
        }
        out.push(s);
    }
    // A jog whose offset runs up from where its web has no length.
    let mut s = Sweep::new("jog, web from nothing");
    let (alpha, r, kk) = (FRAC_PI_2, 1.0, 0.5);
    let min_h = (2.0 * r + t) * (1.0 - alpha.pcos());
    for i in 0..n {
        let h = min_h + 2e-8 * i as f64;
        let jog = JogSpec {
            line: [[25.0, -5.0], [25.0, 45.0]],
            offset: h,
            angle: alpha,
            direction: Direction::Up,
            radius: r,
            k: kk,
            ids: [1, 2],
        };
        let spec = rect(1.0, t, Vec::new(), vec![jog]);
        let web = (h - min_h) / alpha.psin();
        let ba = alpha * (r + kk * t);
        let v = t * 40.0 * (50.0 + web) + 2.0 * alpha * 40.0 * t * (r + 0.5 * t);
        let a = 2000.0 + 40.0 * (2.0 * ba + web);
        s.record(h - min_h, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    // A flange measured to the mould line whose length runs up from its
    // setback (its flat from nothing).
    let mut s = Sweep::new("mould length from the setback");
    let (th, r, kk) = (1.1, 2.0, 0.33);
    let setback = (r + t) * (0.5 * th).ptan();
    for i in 0..n {
        let l = setback + 2e-8 * i as f64;
        let mut f = flange(2, l, th, Direction::Down, r, kk);
        f.from = LengthFrom::Mould;
        let spec = rect(1.0, t, vec![f], Vec::new());
        let flat = l - setback;
        let v = 2000.0 * t + 40.0 * (th * t * (r + 0.5 * t) + flat * t);
        let a = 2000.0 + 40.0 * (th * (r + kk * t) + flat);
        s.record(flat, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    // Two walls at a corner, both turned 90 degrees give or take
    // billionths of a radian.
    let mut s = Sweep::new("two walls at a corner about square");
    for th in steps(FRAC_PI_2, 1e-9, n) {
        let (r, kk) = (1.5, 0.5);
        let spec = rect(
            1.0,
            t,
            vec![
                flange(1, 15.0, th, Direction::Up, r, kk),
                flange(2, 15.0, th, Direction::Up, r, kk),
            ],
            Vec::new(),
        );
        let bends = (50.0 + 40.0) * (th * t * (r + 0.5 * t) + 15.0 * t);
        let v = 2000.0 * t + bends;
        let a = 2000.0 + 90.0 * (th * (r + kk * t) + 15.0);
        s.record(th - FRAC_PI_2, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    // A hem: folded back by 180 degrees less billionths.
    let mut s = Sweep::new("hem about folded back");
    for i in 0..n {
        let th = PI - 1e-9 * i as f64;
        let (r, kk) = (0.5, 0.5);
        let spec = rect(
            1.0,
            t,
            vec![flange(2, 10.0, th, Direction::Up, r, kk)],
            Vec::new(),
        );
        let v = 2000.0 * t + 40.0 * (th * t * (r + 0.5 * t) + 10.0 * t);
        let a = 2000.0 + 40.0 * (th * (r + kk * t) + 10.0);
        s.record(PI - th, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    // A tiny angle: 1e-9 to 1e-1 radians.
    let mut s = Sweep::new("a flange turned by a tiny angle");
    for i in 0..n {
        let th = 10f64.ppowf(-9.0 + 8.0 * i as f64 / (n - 1) as f64);
        let (r, kk) = (2.0, 0.5);
        let spec = rect(
            1.0,
            t,
            vec![flange(2, 10.0, th, Direction::Up, r, kk)],
            Vec::new(),
        );
        let v = 2000.0 * t + 40.0 * (th * t * (r + 0.5 * t) + 10.0 * t);
        let a = 2000.0 + 40.0 * (th * (r + kk * t) + 10.0);
        s.record(th, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    // An inset about the sliver limit (4 tolerances).
    let mut s = Sweep::new("an inset about four tolerances");
    for i in 0..n {
        let inset = 4e-6 + 1e-9 * (i as f64 - (n / 2) as f64) * 10.0;
        let (th, r, kk) = (FRAC_PI_2, 1.0, 0.5);
        let mut f = flange(2, 10.0, th, Direction::Up, r, kk);
        f.inset = [inset, 0.0];
        let spec = rect(1.0, t, vec![f], Vec::new());
        // An inset within the tolerance of 0 is 0 (docs/design/sheetmetal.md
        // › *Insets*).
        let w = 40.0 - if inset <= tol.linear { 0.0 } else { inset };
        let v = 2000.0 * t + w * (th * t * (r + 0.5 * t) + 10.0 * t);
        let a = 2000.0 + w * (th * (r + kk * t) + 10.0);
        s.record(inset, sheet(&spec, &tol, a), v, &tol, false);
    }
    out.push(s);
    let _ = TAU;
    out
}

#[test]
fn sheet_metal_stepped_by_billionths_changes_only_at_real_limits() {
    let sweeps = sheet_sweeps(200);
    for s in &sweeps {
        s.hold(3);
    }
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn sheet_metal_stepped_by_billionths_wide() {
    for s in sheet_sweeps(2000) {
        s.hold(6);
    }
}
