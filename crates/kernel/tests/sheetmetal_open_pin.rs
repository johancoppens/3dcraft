//! Open corners are built bit for bit as before closed corners existed:
//! six open parts (a box with each relief and a hole, a Z, a flange with a
//! cut-out) pinned by the bits of their volume, area and flat area, their
//! counts and a hash of every vertex of the solid and the sheet, as main
//! built them (docs/design/sheetmetal.md › *Closed corners*).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::profile::{Curve2, Profile, SketchCurve, detect_profiles};
use craft3d_kernel::sheetmetal::{
    Closing, Cutout, Direction, FlangeSpec, LengthFrom, On, Part, Relief, ReliefKind, Spec, build,
    flat_area,
};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::mass_properties;
use std::f64::consts::FRAC_PI_2;

fn tol() -> Tolerance {
    Tolerance::new(1e-6, 1e-9).unwrap()
}

fn profiles(curves: &[SketchCurve]) -> Vec<Profile> {
    detect_profiles(curves, &tol()).unwrap()
}

fn line(id: u64, start: [f64; 2], end: [f64; 2]) -> SketchCurve {
    SketchCurve {
        id,
        curve: Curve2::Line { start, end },
    }
}

fn rect(w: f64, d: f64, hole: bool) -> Profile {
    let c = [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]];
    let mut curves: Vec<SketchCurve> = (0..4)
        .map(|i| line(i as u64 + 1, c[i], c[(i + 1) % 4]))
        .collect();
    if hole {
        curves.push(SketchCurve {
            id: 9,
            curve: Curve2::Circle {
                center: [20.0, 12.0],
                radius: 3.0,
            },
        });
    }
    profiles(&curves).remove(0)
}

fn flange(on: On, direction: Direction) -> FlangeSpec {
    FlangeSpec {
        on,
        length: 10.0,
        angle: FRAC_PI_2,
        direction,
        radius: 1.0,
        k: 0.4,
        from: LengthFrom::Tangent,
        inset: [0.0; 2],
        outline: None,
    }
}

/// The parts every open-corner build is pinned on.
pub fn parts() -> Vec<(String, Part)> {
    let mut out = Vec::new();
    let four: Vec<FlangeSpec> = (1..=4)
        .map(|c| flange(On::Base(c), Direction::Up))
        .collect();
    for relief in [
        None,
        Some(ReliefKind::Rect),
        Some(ReliefKind::Round),
        Some(ReliefKind::Tear),
    ] {
        let name = format!("box-{}", relief.map_or("none", |r| r.name()));
        let part = build(
            &spec(
                rect(60.0, 40.0, true),
                four.clone(),
                relief.map(|kind| Relief { kind, size: 3.0 }),
                Vec::new(),
            ),
            &tol(),
        )
        .unwrap();
        out.push((name, part));
    }
    let z = vec![
        flange(On::Base(1), Direction::Up),
        flange(On::Tip(0), Direction::Down),
    ];
    out.push((
        "z".to_string(),
        build(&spec(rect(40.0, 30.0, false), z, None, Vec::new()), &tol()).unwrap(),
    ));
    let vent = profiles(&[SketchCurve {
        id: 20,
        curve: Curve2::Circle {
            center: [30.0, -8.0],
            radius: 2.0,
        },
    }])
    .remove(0);
    out.push((
        "cutout".to_string(),
        build(
            &spec(
                rect(60.0, 40.0, false),
                vec![
                    flange(On::Base(1), Direction::Up),
                    flange(On::Base(2), Direction::Up),
                ],
                Some(Relief {
                    kind: ReliefKind::Round,
                    size: 3.0,
                }),
                vec![Cutout {
                    profile: vent,
                    from: "vent".to_string(),
                }],
            ),
            &tol(),
        )
        .unwrap(),
    ));
    out
}

/// Volume, area and flat area bits, faces, edges, vertices, vertex hash.
type Pin = (u64, u64, u64, usize, usize, usize, u64);

/// volume, area and flat area bits, counts, and an FNV hash over every
/// vertex's coordinates' bits of the solid and of the sheet.
pub fn pin(part: &Part) -> Pin {
    let m = mass_properties(&part.solid.body).unwrap();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for body in [&part.solid.body, &part.sheet] {
        for v in &body.vertices {
            for c in [v.point.x, v.point.y, v.point.z] {
                h = (h ^ c.to_bits()).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    let b = &part.solid.body;
    (
        m.volume.to_bits(),
        m.area.to_bits(),
        flat_area(part).to_bits(),
        b.faces.len(),
        b.edges.len(),
        b.vertices.len(),
        h,
    )
}

fn spec(
    base: Profile,
    flanges: Vec<FlangeSpec>,
    relief: Option<Relief>,
    cutouts: Vec<Cutout>,
) -> Spec {
    Spec {
        base,
        plane: Frame::WORLD,
        thickness: 1.0,
        flanges,
        relief,
        bend_relief: None,
        corner: Closing::OPEN,
        corners: Vec::new(),
        cutouts,
        ids: Vec::new(),
    }
}

/// What main built before closed corners existed (76e4d0f5 and 8ee335b4,
/// Check runs 40609618286 and 40608178864, this file's parts printed
/// there): open parts are built bit for bit as they were.
const PINS: &[(&str, Pin)] = &[
    (
        "box-none",
        (
            0x40b2eaf6edad319d,
            0x40c389d06a2c64bd,
            0x40b2cb8c7383dc63,
            39,
            79,
            42,
            0xb645a3d6dc522ee,
        ),
    ),
    (
        "box-rect",
        (
            0x40b19e6a782f982c,
            0x40c23d43f4aecb55,
            0x40b182c516ec806a,
            47,
            103,
            58,
            0xf376c9112c666e45,
        ),
    ),
    (
        "box-round",
        (
            0x40b1a6243d70cb73,
            0x40c2426a782f9832,
            0x40b18a7edc2db3b8,
            43,
            91,
            50,
            0x37a9a030a0a0b8d2,
        ),
    ),
    (
        "box-tear",
        (
            0x40b14997c7f3322e,
            0x40c1f8b77f3131f8,
            0x40b12df266b01a52,
            43,
            91,
            50,
            0x96495c5ef99d6e3b,
        ),
    ),
    (
        "z",
        (
            0x40a118fdb9effea7,
            0x40b1d66a782f9838,
            0x40a0ffdbbe9bba77,
            22,
            44,
            24,
            0xbfd02f4c22107336,
        ),
    ),
    (
        "cutout",
        (
            0x40ababb1d4f9870f,
            0x40bcb065f0030f89,
            0x40ab8e29e7435092,
            24,
            50,
            28,
            0xa5007de87ed7f0af,
        ),
    ),
];

#[test]
fn open_parts_are_built_bit_for_bit_as_before_closed_corners() {
    for (name, part) in parts() {
        let p = pin(&part);
        println!(
            "PIN {name} {:#x} {:#x} {:#x} {} {} {} {:#x}",
            p.0, p.1, p.2, p.3, p.4, p.5, p.6
        );
        let (_, want) = PINS.iter().find(|(n, _)| *n == name).unwrap();
        assert_eq!(p, *want, "{name}");
    }
    assert_eq!(PINS.len(), parts().len());
}
