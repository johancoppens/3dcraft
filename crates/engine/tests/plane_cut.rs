//! `feature.planeCut`: a body cut by planes in one feature, one side of
//! each kept (docs/design/plane-cut.md). Through the commands: closed
//! forms of volume and area at three scales, the two sides of a plane
//! that add up to the body, the same cut made the five-feature way, the
//! planes by base plane, datum plane, flat face and point with normal,
//! what each plane reports (a face left, whole parts taken away, nothing
//! changed), one feature against a feature a plane bit for bit, the
//! refusals with the plane named, the edit
//! of the list by ids, switching off, following a name of the target
//! into the cut body, undo and redo, save and reopen. The names are in
//! `plane_cut_names.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, assert_solid, body, built, failed, failure, feature, inspect, names_of,
    new_document, refused, run, session, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// The command, in one place.
const CUT: &str = "feature.planeCut";

const BLOCK_VOLUME: f64 = 24_000.0;
const BLOCK_AREA: f64 = 5_200.0;

/// A block of 40 × 30 × 20 at the origin, `Part` (F1).
fn part(s: &mut Session, k: f64) {
    built(
        s,
        "feature.block",
        json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "name": "Part"}),
    );
}

/// A plane through `at` (times `k`) square to `normal`.
fn through(k: f64, at: [f64; 3], normal: [f64; 3]) -> Value {
    json!({"point": at.map(|x| x * k), "normal": normal})
}

fn bodies(s: &mut Session) -> Vec<String> {
    inspect(s)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect()
}

fn vec3(value: &Value) -> [f64; 3] {
    let v = value.as_array().unwrap_or_else(|| panic!("{value}"));
    [
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    ]
}

/// The names of the edges of the body of `feature`.
fn edge_names(s: &mut Session, feature: &str) -> Vec<String> {
    run(s, "document.edges", json!({"feature": feature}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

/// The corner at (40, 0, 20) cut off through the points 10 along each of
/// its edges, by a point and a normal: the block less a tetrahedron of
/// 1000/6, three right triangles of 50 replaced by an equilateral one of
/// side 10 √2. The cut face is named by the plane's id.
#[test]
fn a_corner_cut_off_a_block_by_a_point_and_a_normal() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let result = built(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": through(k, [30.0, 0.0, 20.0], [1.0, -1.0, 1.0]),
                "name": "Bevel",
            }),
        );
        assert_eq!(
            result["planes"],
            json!([{"id": 1, "cuts": true, "faces": 1}]),
            "{result}"
        );
        assert!(result.get("warnings").is_none(), "{result}");
        let slant = 50.0 * 3f64.sqrt();
        let found = assert_solid(
            &mut s,
            "Bevel",
            BLOCK_VOLUME - 1000.0 / 6.0,
            Some(BLOCK_AREA - 150.0 + slant),
            k,
        );
        assert_eq!(bodies(&mut s), ["Bevel"]);
        let mut faces = names_of(&found);
        faces.sort();
        assert_eq!(
            faces,
            [
                "F2:copy(0)/F1:face(0)",
                "F2:copy(0)/F1:face(1)",
                "F2:copy(0)/F1:face(2)",
                "F2:copy(0)/F1:face(3)",
                "F2:copy(0)/F1:face(4)",
                "F2:copy(0)/F1:face(5)",
                "F2:cut(1)",
            ]
        );
        // Its edges by the faces they lie between; one the block had and
        // the plane left whole also by the copy of its own name.
        let edges = edge_names(&mut s, "Bevel");
        for wanted in [
            "F2:between[F2:copy(0)/F1:face(1),F2:cut(1)]",
            "F2:between[F2:copy(0)/F1:face(2),F2:cut(1)]",
            "F2:between[F2:copy(0)/F1:face(5),F2:cut(1)]",
        ] {
            assert!(edges.iter().any(|e| e == wanted), "{wanted} in {edges:?}");
        }
        assert_eq!(edges.len(), 15, "{edges:?}");
        // The parameters as the document keeps them, and the report.
        let stored = feature(&mut s, "Bevel");
        assert_eq!(stored["params"]["type"], "planeCut", "{stored}");
        assert_eq!(stored["params"]["planes"][0]["id"], 1, "{stored}");
        assert_eq!(stored["params"]["planes"][0]["side"], "below", "{stored}");
        assert_eq!(stored["params"]["next_plane"], 2, "{stored}");
        assert_eq!(stored["planes"][0]["cuts"], true, "{stored}");
        // A sketch on the cut face lies in the plane as given: its origin
        // the point, its normal the plane's, out of the body.
        let sketch = built(&mut s, "sketch.create", json!({"face": "Bevel:cut(1)"}));
        let origin = vec3(&sketch["frame"]["origin"]);
        let normal = vec3(&sketch["normal"]);
        let unit = 1.0 / 3f64.sqrt();
        for (got, want) in origin.into_iter().zip([30.0 * k, 0.0, 20.0 * k]) {
            assert!((got - want).abs() <= 1e-9 * k, "{origin:?}");
        }
        for (got, want) in normal.into_iter().zip([unit, -unit, unit]) {
            assert!((got - want).abs() <= 1e-12, "{normal:?}");
        }
    }
}

/// A gable roof on the block by two datum planes: eaves at 12, the ridge
/// at 20, along the top of the block. The cut follows the datums.
#[test]
fn a_roof_by_two_datum_planes_follows_them() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let at = |p: [f64; 3]| json!(p.map(|x| x * k));
        built(
            &mut s,
            "datum.plane",
            json!({"through": [at([0.0, 0.0, 12.0]), at([40.0, 0.0, 12.0]), at([0.0, 15.0, 20.0])], "name": "South"}),
        );
        built(
            &mut s,
            "datum.plane",
            json!({"through": [at([40.0, 30.0, 12.0]), at([0.0, 30.0, 12.0]), at([40.0, 15.0, 20.0])], "name": "North"}),
        );
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": ["South", "North"], "name": "Roof"}),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 1},
                {"id": 2, "cuts": true, "faces": 1},
            ]),
            "{result}"
        );
        let found = assert_solid(&mut s, "Roof", 19_200.0, Some(4480.0), k);
        assert_eq!(found["faces"], 7, "{found}");
        let names = names_of(&found);
        assert!(names.contains(&"F4:cut(1)".to_string()), "{names:?}");
        assert!(names.contains(&"F4:cut(2)".to_string()), "{names:?}");
        // The top of the block is cut away: the ridge lies in its plane.
        assert!(
            !names.contains(&"F4:copy(0)/F1:face(1)".to_string()),
            "{names:?}"
        );
        let edges = edge_names(&mut s, "Roof");
        assert!(
            edges.iter().any(|e| e == "F4:between[F4:cut(1),F4:cut(2)]"),
            "{edges:?}"
        );
        // A sketch on a cut face has the datum's own origin.
        let sketch = built(
            &mut s,
            "sketch.create",
            json!({"face": "Roof:cut(1)", "name": "OnRoof"}),
        );
        let origin = vec3(&sketch["frame"]["origin"]);
        for (got, want) in origin.into_iter().zip([0.0, 0.0, 12.0 * k]) {
            assert!((got - want).abs() <= 1e-9 * k, "{origin:?}");
        }
        // The eaves lowered to 10: the same names on a lower roof.
        built(
            &mut s,
            "datum.plane",
            json!({"replace": "South", "through": [at([0.0, 0.0, 10.0]), at([40.0, 0.0, 10.0]), at([0.0, 15.0, 20.0])]}),
        );
        built(
            &mut s,
            "datum.plane",
            json!({"replace": "North", "through": [at([40.0, 30.0, 10.0]), at([0.0, 30.0, 10.0]), at([40.0, 15.0, 20.0])]}),
        );
        let lower = assert_solid(&mut s, "Roof", 18_000.0, None, k);
        let mut after = names_of(&lower);
        let mut before = names;
        after.sort();
        before.sort();
        assert_eq!(after, before);
        assert_eq!(feature(&mut s, "OnRoof")["status"], "ok");
    }
}

/// With `keep` the target keeps its body: cut once on each side of one
/// plane, the two parts add up to the block, in volume and in first
/// moments, and their areas to the block's and twice the section.
#[test]
fn the_two_sides_of_a_plane_add_up_to_the_body() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let plane = through(k, [20.0, 0.0, 0.0], [2.0, 1.0, 3.0]);
        let mut above = plane.clone();
        above["side"] = json!("above");
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [plane], "keep": true, "name": "Below"}),
        );
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [above], "name": "Above"}),
        );
        assert_eq!(bodies(&mut s), ["Below", "Above"]);
        let (a, b) = (body(&mut s, "Below"), body(&mut s, "Above"));
        let volume = a["volume"].as_f64().unwrap() + b["volume"].as_f64().unwrap();
        assert!(
            (volume - BLOCK_VOLUME * k * k * k).abs() <= 1e-9 * BLOCK_VOLUME * k * k * k,
            "{volume}"
        );
        let (ca, cb) = (vec3(&a["centroid"]), vec3(&b["centroid"]));
        let (va, vb) = (a["volume"].as_f64().unwrap(), b["volume"].as_f64().unwrap());
        for (axis, want) in [20.0, 15.0, 10.0].into_iter().enumerate() {
            let moment = va * ca[axis] + vb * cb[axis];
            let whole = BLOCK_VOLUME * k * k * k * want * k;
            assert!((moment - whole).abs() <= 1e-9 * whole, "axis {axis}");
        }
        // Each has the cut face: the areas add up to the block's and
        // twice the section, and the two cut faces have one name each.
        let section =
            (a["area"].as_f64().unwrap() + b["area"].as_f64().unwrap() - BLOCK_AREA * k * k) / 2.0;
        assert!(section > 100.0 * k * k, "{section}");
        assert!(names_of(&a).contains(&"F2:cut(1)".to_string()), "{a}");
        assert!(names_of(&b).contains(&"F3:cut(1)".to_string()), "{b}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

/// The planes a cut takes besides a point and a normal: a base plane, a
/// flat face of an earlier body by its name (its normal out of that
/// body), each moved by an offset; and `side: above`.
#[test]
fn planes_by_base_plane_and_flat_face_with_offset_and_side() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        // 5 under the top face of the block itself, keeping what lies
        // behind it.
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [{"plane": "Part:face(1)", "offset": -5.0 * k}], "name": "Lower"}),
        );
        assert_solid(&mut s, "Lower", 40.0 * 30.0 * 15.0, None, k);
        // What lies above z = 5.
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Lower", "planes": {"plane": "xy", "offset": 5.0 * k, "side": "above"}, "name": "Slab"}),
        );
        assert_eq!(result["planes"][0]["id"], 1, "{result}");
        let slab = assert_solid(&mut s, "Slab", 40.0 * 30.0 * 10.0, Some(3800.0), k);
        let names = names_of(&slab);
        assert!(names.contains(&"F3:cut(1)".to_string()), "{names:?}");
        assert!(
            names.contains(&"F3:copy(0)/F2:cut(1)".to_string()),
            "{names:?}"
        );
        // Seen from the kept side: the cut face of `side: above` looks
        // down, out of the body.
        let sketch = built(&mut s, "sketch.create", json!({"face": "Slab:cut(1)"}));
        let normal = vec3(&sketch["normal"]);
        assert!((normal[2] + 1.0).abs() <= 1e-12, "{normal:?}");
        // A base plane alone, as a string. zx has its normal along +y:
        // below keeps y <= 0, where the slab only has its front face.
        let reason = failed(
            &mut s,
            CUT,
            json!({"target": "Slab", "planes": "zx", "name": "Gone"}),
        );
        assert!(
            reason.starts_with("planes.0: plane 1 leaves nothing: `Slab` lies wholly above it"),
            "{reason}"
        );
    }
}

/// A plane that leaves nothing fails the feature, naming the plane; one
/// that cuts nothing changes nothing and is listed.
#[test]
fn a_plane_that_misses_is_listed_and_one_that_leaves_nothing_fails() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let result = built(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": [
                    through(k, [0.0, 0.0, 25.0], [0.0, 0.0, 1.0]),
                    through(k, [0.0, 0.0, 20.0], [0.0, 0.0, 1.0]),
                    through(k, [40.0, 0.0, 20.0], [1.0, 0.0, 1.0]),
                    through(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
                ],
                "keep": true,
                "name": "Half",
            }),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": false, "faces": 0},
                {"id": 2, "cuts": false, "faces": 0},
                {"id": 3, "cuts": false, "faces": 0},
                {"id": 4, "cuts": true, "faces": 1},
            ]),
            "{result}"
        );
        let warnings = result["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 3, "{result}");
        assert!(
            warnings[0]
                .as_str()
                .unwrap()
                .starts_with("plane 1 changes nothing"),
            "{result}"
        );
        assert_solid(&mut s, "Half", BLOCK_VOLUME / 2.0, None, k);
        // Nothing left: above the plane in the top face, and above a
        // plane over the body, after a plane that cuts.
        let reason = failed(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": [{"point": [0.0, 0.0, 20.0 * k], "normal": [0, 0, 1], "side": "above"}],
                "keep": true,
                "name": "Gone",
            }),
        );
        assert!(
            reason.starts_with("planes.0: plane 1 leaves nothing: `Part` lies wholly below it"),
            "{reason}"
        );
        let reason = failed(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": [
                    through(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
                    {"point": [0.0, 0.0, 12.0 * k], "normal": [0, 0, -1], "id": 7},
                ],
                "keep": true,
                "name": "GoneToo",
            }),
        );
        assert!(
            reason.starts_with(
                "planes.1: plane 7 leaves nothing: what the planes before it leave of `Part` lies wholly above it"
            ),
            "{reason}"
        );
        // Everything else is still built.
        assert_eq!(body(&mut s, "Half")["valid"], true);
    }
}

/// What regeneration refuses, with the parameter and the plane: a plane a
/// tolerance from a face, a sheet as the target, a face that is not flat,
/// a normal of no length, a plane placed by something that is no plane.
#[test]
fn refusals_name_the_plane_and_the_reason() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let reason = failed(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": [
                    through(k, [30.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
                    through(k, [0.0, 0.0, 20.0 - 1e-6], [0.0, 0.0, 1.0]),
                ],
                "keep": true,
                "name": "Near",
            }),
        );
        assert!(reason.starts_with("planes.1: plane 2: "), "{reason}");
        assert!(reason.contains("at the edge of tolerance"), "{reason}");
        assert!(reason.contains("F2:copy(0)/F1:face(1)"), "{reason}");
        assert!(reason.contains("F2:cut(2)"), "{reason}");
        assert!(reason.contains("nothing is snapped"), "{reason}");

        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": 5.0 * k, "height": 10.0 * k, "position": [60.0 * k, 0.0, 0.0], "name": "Rod"}),
        );
        let reason = failed(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": ["Rod:face(0)"], "keep": true, "name": "Round"}),
        );
        assert!(reason.starts_with("planes.0: "), "{reason}");
        assert!(reason.contains("flat"), "{reason}");

        let reason = failed(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [{"normal": [0, 0, 0]}], "keep": true, "name": "Flat"}),
        );
        assert!(
            reason.starts_with("planes.0.normal must be a direction"),
            "{reason}"
        );
        let reason = failed(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [{"normal": [0, 0, 1], "offset": "30 deg"}], "keep": true, "name": "Turned"}),
        );
        assert!(reason.starts_with("planes.0.offset: "), "{reason}");

        // A sheet has no inside.
        built(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Outline"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Outline", "from": [100.0 * k, 0.0], "to": [120.0 * k, 10.0 * k]}),
        );
        built(
            &mut s,
            "feature.fill",
            json!({"sketch": "Outline", "name": "Sheet"}),
        );
        let reason = failed(
            &mut s,
            CUT,
            json!({"target": "Sheet", "planes": ["yz"], "name": "Trimmed"}),
        );
        assert!(reason.starts_with("target: `Sheet` is a sheet"), "{reason}");
        // A target whose body went into another feature says where.
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [through(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0])], "name": "Taken"}),
        );
        let reason = failed(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": ["xy"], "name": "Late"}),
        );
        assert!(reason.contains("already went into `Taken`"), "{reason}");
    }
}

/// What the command refuses before anything is stored.
#[test]
fn bad_parameters_are_refused() {
    let mut s = session();
    new_document(&mut s, 1.0);
    part(&mut s, 1.0);
    let plane = json!({"normal": [0, 0, 1], "point": [0, 0, 10]});
    for (params, says) in [
        (json!({"planes": [plane]}), "missing parameter `target`"),
        (json!({"target": "Part"}), "missing parameter `planes`"),
        (
            json!({"target": "Nope", "planes": [plane]}),
            "no feature named `Nope`",
        ),
        (json!({"target": "Part", "planes": []}), "`planes` is empty"),
        (
            json!({"target": "Part", "planes": [[0, 0, 1]]}),
            "`planes.0` must be a plane",
        ),
        (
            json!({"target": "Part", "planes": [true]}),
            "`planes.0` must be a plane",
        ),
        (json!({"target": "Part", "planes": [{}]}), "needs `plane`"),
        (
            json!({"target": "Part", "planes": [{"plane": "xy", "normal": [0, 0, 1]}]}),
            "not both",
        ),
        (
            json!({"target": "Part", "planes": [{"plane": "xy", "point": [0, 0, 1]}]}),
            "`point` goes with `normal`",
        ),
        (
            json!({"target": "Part", "planes": [{"normal": [0, 0, 1], "up": true}]}),
            "unknown key `up`",
        ),
        (
            json!({"target": "Part", "planes": [{"normal": [0, 1]}]}),
            "three expressions",
        ),
        (
            json!({"target": "Part", "planes": [{"normal": [0, 0, 1], "side": "up"}]}),
            "`planes.0.side` must be",
        ),
        (
            json!({"target": "Part", "planes": [{"normal": [0, 0, 1], "id": 0}]}),
            "`planes.0.id` is a plane's id",
        ),
        (
            json!({"target": "Part", "planes": [{"normal": [0, 0, 1], "id": 4294967295u64}]}),
            "`planes.0.id` is a plane's id",
        ),
        (
            json!({"target": "Part", "planes": [{"plane": "xy", "id": 3}, {"plane": "yz", "id": 3}]}),
            "given to two planes",
        ),
        (
            json!({"target": "Part", "planes": ["nowhere"]}),
            "there is no plane `nowhere`",
        ),
        (
            json!({"target": "Part", "planes": ["xy"], "keep": 1}),
            "`keep`",
        ),
        (
            json!({"target": "Part", "planes": ["xy"], "side": "above"}),
            "unknown parameter `side`",
        ),
    ] {
        let said = refused(&mut s, CUT, params.clone());
        assert!(said.contains(says), "{params}: {said}");
    }
    let many: Vec<Value> = (0..257).map(|_| json!("xy")).collect();
    let said = refused(&mut s, CUT, json!({"target": "Part", "planes": many}));
    assert!(said.contains("the most is 256"), "{said}");
    // Nothing was stored.
    assert_eq!(inspect(&mut s)["features"].as_array().unwrap().len(), 1);
    // 256 planes that all cut nothing are a cut that changes nothing.
    let many: Vec<Value> = (0..256)
        .map(|i| json!({"normal": [0, 0, 1], "point": [0, 0, 30 + i]}))
        .collect();
    let result = built(
        &mut s,
        CUT,
        json!({"target": "Part", "planes": many, "name": "Idle"}),
    );
    assert_eq!(result["planes"].as_array().unwrap().len(), 256);
    assert_eq!(result["warnings"].as_array().unwrap().len(), 256);
    assert_solid(&mut s, "Idle", BLOCK_VOLUME, Some(BLOCK_AREA), 1.0);
}

/// `feature.edit`: the whole list of planes, each kept by its id or found
/// again by what it is; the target; keep. Undo and redo.
#[test]
fn an_edit_keeps_planes_by_their_ids() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    part(&mut s, k);
    let south = json!({"point": [0, 0, 12], "normal": [0, -8, 15]});
    let north = json!({"point": [40, 30, 12], "normal": [0, 8, 15]});
    built(
        &mut s,
        CUT,
        json!({"target": "Part", "planes": [south, north], "name": "Roof"}),
    );
    assert_solid(&mut s, "Roof", 19_200.0, Some(4480.0), k);
    let ids = |s: &mut Session| -> Vec<u64> {
        feature(s, "Roof")["params"]["planes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(ids(&mut s), [1, 2]);
    // A plane put between them, the others as they were given: they are
    // found again by what they are and keep their ids.
    let hip = json!({"point": [35, 0, 12], "normal": [8, 0, 15]});
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Roof", "planes": [south, hip, north]}),
    );
    assert_eq!(ids(&mut s), [1, 3, 2]);
    let hipped = body(&mut s, "Roof");
    let names = names_of(&hipped);
    for wanted in ["F2:cut(1)", "F2:cut(2)", "F2:cut(3)"] {
        assert!(names.contains(&wanted.to_string()), "{names:?}");
    }
    assert!(hipped["volume"].as_f64().unwrap() < 19_200.0);
    // The order turned round, by ids: the same body.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Roof", "planes": [
            {"point": [40, 30, 12], "normal": [0, 8, 15], "id": 2},
            {"point": [35, 0, 12], "normal": [8, 0, 15], "id": 3},
            {"point": [0, 0, 12], "normal": [0, -8, 15], "id": 1},
        ]}),
    );
    assert_eq!(ids(&mut s), [2, 3, 1]);
    let turned = body(&mut s, "Roof");
    assert_close(
        &turned["volume"],
        hipped["volume"].as_f64().unwrap(),
        "volume",
    );
    assert_close(&turned["area"], hipped["area"].as_f64().unwrap(), "area");
    let (mut a, mut b) = (names_of(&turned), names);
    a.sort();
    b.sort();
    assert_eq!(a, b);
    // Plane 3 moved, by its id: it stays plane 3. Plane 2 deleted and a
    // plane like none before added: it is plane 4, never 2 again.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Roof", "planes": [
            {"point": [30, 0, 12], "normal": [8, 0, 15], "id": 3},
            {"point": [0, 0, 12], "normal": [0, -8, 15], "id": 1},
            {"point": [40, 30, 14], "normal": [0, 8, 15]},
        ]}),
    );
    assert_eq!(ids(&mut s), [3, 1, 4]);
    assert_eq!(feature(&mut s, "Roof")["params"]["next_plane"], 5);
    let names = names_of(&body(&mut s, "Roof"));
    assert!(!names.contains(&"F2:cut(2)".to_string()), "{names:?}");
    assert!(names.contains(&"F2:cut(4)".to_string()), "{names:?}");
    // An id the cut has no plane of, an id twice, a parameter it has not.
    for (params, says) in [
        (
            json!({"feature": "Roof", "planes": [{"plane": "xy", "id": 2}]}),
            "the cut has no plane 2; its planes are 3, 1, 4",
        ),
        (
            json!({"feature": "Roof", "planes": [{"plane": "xy", "id": 9}]}),
            "the cut has no plane 9",
        ),
        (
            json!({"feature": "Roof", "planes": [{"plane": "xy", "id": 3}, {"plane": "yz", "id": 3}]}),
            "given to two planes",
        ),
        (
            json!({"feature": "Roof", "planes": []}),
            "`planes` is empty",
        ),
        (
            json!({"feature": "Roof", "tool": "Part"}),
            "`tool` is not a parameter of a plane cut",
        ),
        (
            json!({"feature": "Roof", "target": "Roof"}),
            "the feature itself",
        ),
    ] {
        let said = refused(&mut s, "feature.edit", params.clone());
        assert!(said.contains(says), "{params}: {said}");
    }
    assert_eq!(ids(&mut s), [3, 1, 4]);
    // Undo gives the planes back as they were; redo the edit.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(ids(&mut s), [2, 3, 1]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(ids(&mut s), [3, 1, 4]);
    // Another target (a block like the first, in its place), kept.
    built(
        &mut s,
        "feature.block",
        json!({"size": [40, 30, 20], "name": "Other"}),
    );
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Other", "position": 1}),
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Roof", "target": "Other", "keep": true}),
    );
    let mut listed = bodies(&mut s);
    listed.sort();
    assert_eq!(listed, ["Other", "Part", "Roof"]);
}

/// Switched off, the cut passes its target's body on under its own name,
/// the faces copies as when it is built; with `keep` it passes nothing.
#[test]
fn switched_off_it_passes_the_target_on() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [through(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0])], "name": "Half"}),
        );
        assert_solid(&mut s, "Half", BLOCK_VOLUME / 2.0, None, k);
        run(&mut s, "feature.suppress", json!({"feature": "Half"}));
        let whole = assert_solid(&mut s, "Half", BLOCK_VOLUME, Some(BLOCK_AREA), k);
        let mut names = names_of(&whole);
        names.sort();
        assert_eq!(names[0], "F2:copy(0)/F1:face(0)");
        assert_eq!(names.len(), 6);
        assert_eq!(feature(&mut s, "Half")["passesOn"], "F1");
        run(
            &mut s,
            "feature.suppress",
            json!({"feature": "Half", "suppressed": false}),
        );
        assert_solid(&mut s, "Half", BLOCK_VOLUME / 2.0, None, k);
        // A condition, and a kept copy that is off: nothing of its own.
        run(&mut s, "variable.set", json!({"name": "open", "expr": "0"}));
        let off = run(
            &mut s,
            CUT,
            json!({"target": "Half", "planes": ["yz"], "keep": true, "when": "open", "name": "Opened"}),
        );
        assert_eq!(off["status"], "inactive", "{off}");
        assert_eq!(bodies(&mut s), ["Half"]);
        assert!(feature(&mut s, "Opened").get("passesNothing").is_some());
    }
}

/// A feature after the cut that names a face as the target had it is
/// followed into the cut body, as through every feature that takes a
/// body; one cut away fails with its name.
#[test]
fn a_name_of_the_target_is_followed_into_the_cut_body() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": [through(k, [0.0, 0.0, 10.0], [0.0, 0.0, 1.0])], "name": "Half"}),
        );
        // The bottom of the block, by the name the block gave it, moved
        // out by 2: the half block grows by 40 × 30 × 2.
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["Part:face(0)"], "distance": 2.0 * k, "name": "Thicker"}),
        );
        assert_solid(&mut s, "Thicker", BLOCK_VOLUME / 2.0 + 2400.0, None, k);
        // The cut face, moved out by 1.
        built(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["Half:cut(1)"], "distance": k, "name": "Higher"}),
        );
        assert_solid(&mut s, "Higher", BLOCK_VOLUME / 2.0 + 3600.0, None, k);
        // The top of the block is cut away: named, it fails, by name.
        let reason = failed(
            &mut s,
            "feature.offsetFace",
            json!({"faces": ["Part:face(1)"], "distance": k, "name": "Gone"}),
        );
        assert!(reason.contains("F1:face(1)"), "{reason}");
        assert!(reason.contains("Half"), "{reason}");
    }
}

/// The same body the five-feature way: a datum plane, a sketch, a
/// rectangle, an extrusion and a boolean per plane. One feature gives the
/// same volume, area, centroid and counts.
#[test]
fn one_feature_cuts_what_five_features_per_plane_cut() {
    for k in SCALES {
        let planes: [([f64; 3], [f64; 3], [f64; 3]); 3] = [
            // A point, a direction in the plane, the normal.
            ([30.0, 0.0, 20.0], [1.0, 1.0, 0.0], [1.0, -1.0, 1.0]),
            ([0.0, 0.0, 12.0], [1.0, 0.0, 0.0], [0.0, -8.0, 15.0]),
            ([40.0, 30.0, 14.0], [1.0, 0.0, 0.0], [0.0, 8.0, 15.0]),
        ];
        // One feature.
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let listed: Vec<Value> = planes.iter().map(|(p, _, n)| through(k, *p, *n)).collect();
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": listed, "name": "Cut"}),
        );
        let one = body(&mut s, "Cut");
        run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(inspect(&mut s)["features"].as_array().unwrap().len(), 2);

        // Five features a plane.
        let mut t = session();
        new_document(&mut t, k);
        part(&mut t, k);
        let mut target = "Part".to_string();
        for (index, (p, u, n)) in planes.iter().enumerate() {
            // Three points counter-clockwise about the normal: p, p + u,
            // p + n × u.
            let v = [
                n[1] * u[2] - n[2] * u[1],
                n[2] * u[0] - n[0] * u[2],
                n[0] * u[1] - n[1] * u[0],
            ];
            let at = |q: [f64; 3]| json!(q.map(|x| x * k));
            let second = [p[0] + u[0], p[1] + u[1], p[2] + u[2]];
            let third = [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
            built(
                &mut t,
                "datum.plane",
                json!({"through": [at(*p), at(second), at(third)], "name": format!("P{index}")}),
            );
            built(
                &mut t,
                "sketch.create",
                json!({"plane": format!("P{index}"), "name": format!("S{index}")}),
            );
            run(
                &mut t,
                "sketch.rect",
                json!({"sketch": format!("S{index}"), "from": [-200.0 * k, -200.0 * k], "to": [200.0 * k, 200.0 * k]}),
            );
            let name = format!("C{index}");
            built(
                &mut t,
                "feature.extrude",
                json!({"sketch": format!("S{index}"), "distance": 200.0 * k, "cut": target, "name": name}),
            );
            target = name;
        }
        let five = body(&mut t, &target);
        assert_eq!(inspect(&mut t)["features"].as_array().unwrap().len(), 13);
        for key in ["faces", "edges", "vertices", "shells"] {
            assert_eq!(one[key], five[key], "{key} at scale {k}");
        }
        assert_close(&one["volume"], five["volume"].as_f64().unwrap(), "volume");
        assert_close(&one["area"], five["area"].as_f64().unwrap(), "area");
        let (a, b) = (vec3(&one["centroid"]), vec3(&five["centroid"]));
        for axis in 0..3 {
            assert!((a[axis] - b[axis]).abs() <= 1e-9 * 40.0 * k, "{a:?} {b:?}");
        }
    }
}

/// Planes through variables follow them; a saved document reopens to the
/// same body, names and report.
#[test]
fn planes_follow_variables_and_the_document_saves_and_reopens() {
    for k in SCALES {
        let dir = workdir("plane_cut", &format!("round-trip-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        part(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "eaves", "expr": format!("{} mm", 12.0 * k)}),
        );
        run(&mut s, "variable.set", json!({"name": "rise", "expr": "8"}));
        built(
            &mut s,
            CUT,
            json!({
                "target": "Part",
                "planes": [
                    {"point": [0, 0, "eaves"], "normal": [0, "-rise", 15]},
                    {"point": [40.0 * k, 30.0 * k, "eaves"], "normal": [0, "rise", 15], "id": 5},
                    {"plane": "Part:face(4)", "offset": -2.0 * k},
                ],
                "name": "Roof",
            }),
        );
        // The roof of 19200 less a slice of 2 off its left end.
        assert_solid(&mut s, "Roof", 19_200.0 * 38.0 / 40.0, None, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "eaves", "expr": format!("{} mm", 10.0 * k)}),
        );
        let before = body(&mut s, "Roof");
        assert_close(
            &before["volume"],
            // The ridge stays 8 above the eaves: 38 × (30 × 10 + 120).
            15_960.0 * k * k * k,
            "volume",
        );
        let report = feature(&mut s, "Roof")["planes"].clone();
        assert_eq!(
            report,
            json!([
                {"id": 6, "cuts": true, "faces": 1},
                {"id": 5, "cuts": true, "faces": 1},
                {"id": 7, "cuts": true, "faces": 1},
            ])
        );
        run(
            &mut s,
            "file.save",
            json!({"path": "roof.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "roof.3dcraft"}));
        let after = body(&mut s, "Roof");
        for key in [
            "faces",
            "edges",
            "vertices",
            "shells",
            "volume",
            "area",
            "faceNames",
        ] {
            assert_eq!(after[key], before[key], "{key} after a round trip");
        }
        let reopened = feature(&mut s, "Roof");
        assert_eq!(reopened["planes"], report);
        assert_eq!(reopened["params"]["next_plane"], 8);
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

/// The review's (2026-11-20): a plane through the gap between two lumps
/// of a body meets neither and takes the lump beyond it away. A block
/// with a slot through its top, cut above 12: two legs of 15 × 30 × 8;
/// the plane x = 20, in the slot, keeps the left one, whether the legs
/// come from another cut or from a plane before it in the same list.
#[test]
fn a_plane_between_two_lumps_takes_the_one_beyond_it_away() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [10.0 * k, 32.0 * k, 12.0 * k], "position": [15.0 * k, -k, 10.0 * k], "name": "Slot"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Part", "tool": "Slot", "name": "Legs"}),
        );
        let level = json!({"point": [0.0, 0.0, 12.0 * k], "normal": [0, 0, 1], "side": "above"});
        let gap = through(k, [20.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        built(
            &mut s,
            CUT,
            json!({"target": "Legs", "planes": [level], "keep": true, "name": "TwoLegs"}),
        );
        let two = assert_solid(&mut s, "TwoLegs", 7200.0, Some(3240.0), k);
        assert_eq!(two["shells"], 2, "{two}");
        built(
            &mut s,
            CUT,
            json!({"target": "TwoLegs", "planes": [gap], "keep": true, "name": "LeftLeg"}),
        );
        let one = assert_solid(&mut s, "LeftLeg", 3600.0, Some(1620.0), k);
        assert_eq!(one["shells"], 1, "{one}");
        built(
            &mut s,
            CUT,
            json!({"target": "Legs", "planes": [level, gap], "name": "InOne"}),
        );
        let one = assert_solid(&mut s, "InOne", 3600.0, Some(1620.0), k);
        assert_eq!(one["shells"], 1, "{one}");
    }
}

/// The review's (2026-11-20): a lofted body, whose side is a spline
/// surface, cut on the slant: the two sides add up to the body, each is
/// valid, a sketch on the cut face lies in the plane, and three planes
/// leave a valid wedge. At three scales (the review's at 1; the others
/// since its second round).
#[test]
fn a_lofted_body_cut_on_the_slant_adds_up() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        for (name, z, center, radius) in [
            ("Base", 0.0, [0.0, 0.0], 10.0),
            ("Middle", 30.0, [3.0, 0.0], 14.0),
            ("End", 60.0, [0.0, 2.0], 8.0),
        ] {
            built(
                &mut s,
                "sketch.create",
                json!({"name": name, "plane": "xy", "offset": z * k}),
            );
            run(
                &mut s,
                "sketch.circle",
                json!({"sketch": name, "center": center.map(|c| c * k), "radius": radius * k}),
            );
        }
        built(
            &mut s,
            "feature.loft",
            json!({"sections": ["Base", "Middle", "End"], "name": "Vase"}),
        );
        let slant = json!({"point": [0.0, 0.0, 25.0 * k], "normal": [0.3, 0.2, 1]});
        let mut other = slant.clone();
        other["side"] = json!("above");
        built(
            &mut s,
            CUT,
            json!({"target": "Vase", "planes": [slant], "keep": true, "name": "Low"}),
        );
        built(
            &mut s,
            CUT,
            json!({"target": "Vase", "planes": [other], "keep": true, "name": "High"}),
        );
        built(
            &mut s,
            CUT,
            json!({"target": "Vase", "planes": [slant, {"normal": [1, 0, 0]}, {"point": [0.0, 4.0 * k, 0.0], "normal": [0, 1, 0], "side": "above"}], "keep": true, "name": "Wedge"}),
        );
        let (whole, low, high) = (
            body(&mut s, "Vase"),
            body(&mut s, "Low"),
            body(&mut s, "High"),
        );
        let wedge = body(&mut s, "Wedge");
        for part in [&low, &high, &wedge] {
            assert_eq!(part["valid"], true, "scale {k}: {part}");
        }
        let v = |b: &Value| b["volume"].as_f64().unwrap();
        let sum = v(&low) + v(&high);
        assert!(
            (sum - v(&whole)).abs() <= 1e-8 * v(&whole),
            "scale {k}: {} + {} is not {}",
            v(&low),
            v(&high),
            v(&whole)
        );
        assert!(v(&low) > 0.25 * v(&whole) && v(&high) > 0.25 * v(&whole));
        let a = |b: &Value| b["area"].as_f64().unwrap();
        assert!(
            a(&low) + a(&high) > a(&whole) + 1000.0 * k * k,
            "scale {k}: twice the section"
        );
        let sketch = built(&mut s, "sketch.create", json!({"face": "Low:cut(1)"}));
        let normal = vec3(&sketch["normal"]);
        let length = (0.09f64 + 0.04 + 1.0).sqrt();
        for (got, want) in normal.into_iter().zip([0.3, 0.2, 1.0]) {
            assert!((got - want / length).abs() <= 1e-12, "{normal:?}");
        }
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "scale {k}: {check}");
    }
}

/// The review's (2026-11-20): a part of 1 mm, 100 m from the origin, cut
/// by planes given through points near the origin (so a plane's own
/// origin is 100 m from what it cuts): through the cube's centre it halves
/// it; through a corner's three points half way along its edges it takes
/// off a tetrahedron of 1/48; two planes square to the axes leave a slab.
#[test]
fn a_part_far_from_the_origin_is_cut_to_its_closed_forms() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [100_000, -50_000, -50_000], "name": "Part"}),
    );
    let diagonal =
        |at: f64, side: &str| json!({"point": [0, 0, at], "normal": [1, 1, 1], "side": side});
    for (name, planes, volume) in [
        ("Half", json!([diagonal(1.5, "below")]), 0.5),
        ("Corner", json!([diagonal(0.5, "below")]), 1.0 / 48.0),
        ("Rest", json!([diagonal(0.5, "above")]), 47.0 / 48.0),
        (
            "Slab",
            json!([
                {"point": [100_000.5, 0, 0], "normal": [1, 0, 0]},
                {"point": [0, -49_999.75, 0], "normal": [0, -1, 0]}
            ]),
            0.375,
        ),
    ] {
        built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": planes, "keep": true, "name": name}),
        );
        assert_solid(&mut s, name, volume, None, 1.0);
    }
}

/// The review's (2026-11-20): a plane placed by a feature after the cut,
/// a plane placed by the cut's own face, and a target after the cut: each
/// fails the cut with the parameter and what to do, and builds nothing.
#[test]
fn planes_and_a_target_from_later_or_from_the_cut_itself_fail_by_name() {
    let mut s = session();
    new_document(&mut s, 1.0);
    part(&mut s, 1.0);
    built(
        &mut s,
        CUT,
        json!({"target": "Part", "planes": {"plane": "xy", "offset": 10}, "name": "Half"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [100, 0, 0], "name": "Later"}),
    );
    let first = json!({"plane": "xy", "offset": 10, "id": 1});
    for (params, says) in [
        (
            json!({"feature": "Half", "planes": [first, "Later:face(1)"]}),
            "planes.1: `Later`, which makes the face F3:face(1), comes after this feature",
        ),
        (
            json!({"feature": "Half", "planes": [first, {"plane": "Half:cut(1)", "offset": -2}]}),
            "planes.1: `Half`, which makes the face F2:cut(1), is this feature itself",
        ),
        (
            json!({"feature": "Half", "target": "Later"}),
            "target: `Later` comes after this feature",
        ),
    ] {
        let result = run(&mut s, "feature.edit", params.clone());
        assert_eq!(result["status"], "failed", "{params}: {result}");
        let reason = result["error"].as_str().unwrap();
        assert!(reason.starts_with(says), "{params}: {reason}");
        assert_eq!(bodies(&mut s), ["Part", "Later"], "{params}");
        run(&mut s, "edit.undo", json!({}));
        assert_solid(&mut s, "Half", BLOCK_VOLUME / 2.0, None, 1.0);
    }
}

/// A block with a slot through its top (`Legs`): cut above 12 it is two
/// legs of 15 × 30 × 8, and a plane in the slot takes one of them away.
fn legs(s: &mut Session, k: f64) {
    part(s, k);
    built(
        s,
        "feature.block",
        json!({"size": [10.0 * k, 32.0 * k, 12.0 * k], "position": [15.0 * k, -k, 10.0 * k], "name": "Slot"}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Part", "tool": "Slot", "name": "Legs"}),
    );
}

/// What a plane reports, for the three things it can do (the review's
/// second round, 2026-11-20). `cuts` says whether it took anything away
/// when it was applied, the planes in the order of the list; `faces` how
/// many faces of the cut body are pieces of it.
///
/// - It leaves a face: `cuts`, one face or more.
/// - It takes material away and leaves no face: whole parts beyond it
///   that it does not pass through (the plane in the slot, after the legs
///   were cut loose), or a piece whose cut face the planes after it cut
///   away (the plane in the slot first: it cuts the base under the slot,
///   which the next plane takes away). `cuts`, no face, and no warning:
///   the body is another for it.
/// - It changes nothing (a second plane in the slot, the one leg left
///   wholly on its kept side): not `cuts`, no face, and a warning.
#[test]
fn a_plane_reports_what_it_did_and_only_one_that_changes_nothing_is_warned_of() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        legs(&mut s, k);
        let level = json!({"point": [0.0, 0.0, 12.0 * k], "normal": [0, 0, 1], "side": "above"});
        let gap = through(k, [20.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let idle = through(k, [22.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Legs", "planes": [level, gap, idle], "keep": true, "name": "Left"}),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 1},
                {"id": 2, "cuts": true, "faces": 0},
                {"id": 3, "cuts": false, "faces": 0},
            ]),
            "scale {k}: {result}"
        );
        let warnings = result["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "scale {k}: {result}");
        assert!(
            warnings[0]
                .as_str()
                .unwrap()
                .starts_with("plane 3 changes nothing"),
            "{result}"
        );
        // The same in the document's own account of the feature.
        assert_eq!(feature(&mut s, "Left")["planes"], result["planes"]);
        let left = assert_solid(&mut s, "Left", 3600.0, Some(1620.0), k);
        assert_eq!(left["shells"], 1, "{left}");
        // The plane in the slot first: it cuts the base, and the level
        // takes the base away with the face it cut.
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Legs", "planes": [gap, level], "keep": true, "name": "LeftToo"}),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 0},
                {"id": 2, "cuts": true, "faces": 1},
            ]),
            "scale {k}: {result}"
        );
        assert!(result.get("warnings").is_none(), "scale {k}: {result}");
        let left = assert_solid(&mut s, "LeftToo", 3600.0, Some(1620.0), k);
        assert_eq!(left["shells"], 1, "{left}");
        // Two level planes on a block of its own, the lower one last: the
        // upper one cut and its face is gone; the lower one first: the
        // upper one changes nothing.
        built(
            &mut s,
            "feature.block",
            json!({"size": [40.0 * k, 30.0 * k, 20.0 * k], "position": [100.0 * k, 0.0, 0.0], "name": "Slab"}),
        );
        let at = |z: f64| through(k, [0.0, 0.0, z], [0.0, 0.0, 1.0]);
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Slab", "planes": [at(15.0), at(10.0)], "keep": true, "name": "Lower"}),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 0},
                {"id": 2, "cuts": true, "faces": 1},
            ]),
            "scale {k}: {result}"
        );
        assert!(result.get("warnings").is_none(), "scale {k}: {result}");
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Slab", "planes": [at(10.0), at(15.0)], "keep": true, "name": "LowerToo"}),
        );
        assert_eq!(
            result["planes"],
            json!([
                {"id": 1, "cuts": true, "faces": 1},
                {"id": 2, "cuts": false, "faces": 0},
            ]),
            "scale {k}: {result}"
        );
        let warnings = result["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "scale {k}: {result}");
        assert!(
            warnings[0]
                .as_str()
                .unwrap()
                .starts_with("plane 2 changes nothing"),
            "{result}"
        );
        for name in ["Lower", "LowerToo"] {
            assert_solid(&mut s, name, BLOCK_VOLUME / 2.0, None, k);
        }
    }
}

/// What the body of `feature` is, without its names: its edges (kind,
/// ends, length) and its vertices' points, each list in an order of its
/// own, and the body's measures and counts. Two bodies that are the same
/// bit for bit give the same.
fn shape(s: &mut Session, feature: &str) -> Value {
    let listed = run(s, "document.edges", json!({"feature": feature}));
    assert_eq!(listed["truncated"], false, "{listed}");
    let mut edges: Vec<String> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| json!([e["curve"], e["start"], e["end"], e["length"], e["closed"]]).to_string())
        .collect();
    edges.sort();
    let mut vertices: Vec<String> = listed["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["point"].to_string())
        .collect();
    vertices.sort();
    let found = body(s, feature);
    json!({
        "edges": edges,
        "vertices": vertices,
        "volume": found["volume"],
        "area": found["area"],
        "faces": found["faces"],
        "shells": found["shells"],
        "valid": found["valid"],
    })
}

/// Planes in one feature are the cuts a feature a plane makes, bit for
/// bit: four planes on the block (two slopes, a corner, the base plane yz
/// moved in with its other side kept) in one feature, and each alone in a
/// feature that takes the one before. Every vertex, every edge and both
/// measures are the same numbers.
#[test]
fn planes_in_one_feature_are_a_feature_a_plane_bit_for_bit() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        part(&mut s, k);
        let planes = [
            through(k, [0.0, 0.0, 12.0], [0.0, -8.0, 15.0]),
            through(k, [40.0, 30.0, 12.0], [0.0, 8.0, 15.0]),
            through(k, [30.0, 0.0, 12.0], [1.0, -1.0, 1.0]),
            json!({"plane": "yz", "offset": 3.0 * k, "side": "above"}),
        ];
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Part", "planes": planes, "keep": true, "name": "One"}),
        );
        for plane in result["planes"].as_array().unwrap() {
            assert_eq!(plane["cuts"], true, "scale {k}: {result}");
            assert_eq!(plane["faces"], 1, "scale {k}: {result}");
        }
        let mut target = "Part".to_string();
        for (index, plane) in planes.iter().enumerate() {
            let name = format!("Step{index}");
            built(
                &mut s,
                CUT,
                json!({"target": target, "planes": [plane], "name": name}),
            );
            target = name;
        }
        let (one, stepped) = (shape(&mut s, "One"), shape(&mut s, "Step3"));
        assert_eq!(one["valid"], true, "scale {k}: {one}");
        assert_eq!(one, stepped, "scale {k}");
    }
}

/// A stated limit of the first slice: a plane placed by a datum or a face
/// of a feature that is switched off fails the whole cut, naming the
/// plane and that feature; nothing is left out silently. Switched on
/// again, the cut is built as before. (A condition per plane, by which
/// such a plane could be left out, is not built yet:
/// docs/design/plane-cut.md › *Not built yet*.)
#[test]
fn a_plane_placed_by_a_feature_that_is_switched_off_fails_the_cut_by_name() {
    let mut s = session();
    new_document(&mut s, 1.0);
    part(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 10], "position": [100, 0, 0], "name": "Gauge"}),
    );
    built(
        &mut s,
        CUT,
        json!({"target": "Part", "planes": ["Gauge:face(1)", through(1.0, [35.0, 0.0, 0.0], [1.0, 0.0, 0.0])], "name": "Half"}),
    );
    assert_solid(&mut s, "Half", 35.0 * 30.0 * 10.0, None, 1.0);
    run(&mut s, "feature.suppress", json!({"feature": "Gauge"}));
    let reason = failure(&mut s, "Half");
    assert!(
        reason.starts_with("planes.0: `Gauge`, which makes the face F2:face(1), is suppressed"),
        "{reason}"
    );
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Gauge", "suppressed": false}),
    );
    assert_solid(&mut s, "Half", 35.0 * 30.0 * 10.0, None, 1.0);
}

/// The outline of a roof of 24 sides (the coordinator's, 2026-11-20).
const EAVES_OUTLINE: [[f64; 2]; 24] = [
    [50.4, 0.0],
    [50.464, 9.658],
    [43.648, 18.0],
    [31.798, 22.713],
    [18.9, 23.383],
    [8.219, 21.91],
    [0.0, 21.0],
    [-8.219, 21.91],
    [-18.9, 23.383],
    [-31.798, 22.713],
    [-43.648, 18.0],
    [-50.464, 9.658],
    [-50.4, 0.0],
    [-44.727, -8.56],
    [-36.373, -15.0],
    [-27.598, -19.713],
    [-18.9, -23.383],
    [-9.756, -26.008],
    [0.0, -27.0],
    [9.756, -26.008],
    [18.9, -23.383],
    [27.598, -19.713],
    [36.373, -15.0],
    [44.727, -8.56],
];

/// The guide's advice for planes worked out from an outline (*Cut by
/// planes* › planes from a computation): a hipped roof over an outline of
/// 24 sides, a plane through every side at the eaves, each given through
/// its side's first corner with a normal square to that side from the
/// side's own numbers, `[dy, -dx, L / tan(a)]`, nothing rounded. Then two
/// neighbours meet on the corner between them to rounding: the roof is
/// built at three scales, has no short edge, and where a corner's upright
/// edge reaches the eaves it ends in one vertex, the corner itself. (With the normals rounded to
/// six decimals a plane passes its neighbour's corner by half a
/// tolerance, and the cut is refused there: docs/design/booleans.md.)
#[test]
fn a_roof_from_the_outlines_own_numbers_meets_on_the_corners() {
    // 1 / tan(38 degrees).
    let run_over_rise = 1.279_941_632_193_078_8;
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let outline: Vec<[f64; 2]> = EAVES_OUTLINE.iter().map(|p| p.map(|c| c * k)).collect();
        built(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Outline"}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Outline", "points": outline, "closed": true}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Outline", "distance": 40.0 * k, "name": "Prism"}),
        );
        let planes: Vec<Value> = (0..outline.len())
            .map(|i| {
                let (from, to) = (outline[i], outline[(i + 1) % outline.len()]);
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                let length = (dx * dx + dy * dy).sqrt();
                json!({
                    "point": [from[0], from[1], 14.0 * k],
                    "normal": [dy, -dx, length * run_over_rise],
                })
            })
            .collect();
        let result = built(
            &mut s,
            CUT,
            json!({"target": "Prism", "planes": planes, "name": "Roof"}),
        );
        let roof = body(&mut s, "Roof");
        assert_eq!(roof["valid"], true, "scale {k}: {roof}");
        assert_eq!(roof["shells"], 1, "scale {k}: {roof}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "scale {k}: {check}");
        // Every plane that leaves a face cut; none of them is a twin of
        // its neighbour's.
        let with_faces = result["planes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["faces"] == 1)
            .count();
        assert!(with_faces >= 12, "scale {k}: {result}");
        let listed = run(&mut s, "document.edges", json!({"feature": "Roof"}));
        assert_eq!(listed["truncated"], false, "{listed}");
        for edge in listed["edges"].as_array().unwrap() {
            let length = edge["length"].as_f64().unwrap();
            assert!(length >= k, "scale {k}: an edge of {length}: {edge}");
        }
        // Where a corner's upright edge reaches the eaves it ends in one
        // vertex, which is the corner at the eaves' height to rounding:
        // nothing else lies within a thousandth of it.
        let points: Vec<[f64; 3]> = listed["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|vertex| vec3(&vertex["point"]))
            .collect();
        let mut at_the_eaves = 0;
        for corner in &outline {
            let near: Vec<f64> = points
                .iter()
                .map(|p| {
                    let (dx, dy, dz) = (p[0] - corner[0], p[1] - corner[1], p[2] - 14.0 * k);
                    (dx * dx + dy * dy + dz * dz).sqrt()
                })
                .filter(|&distance| distance <= 1e-3 * k)
                .collect();
            assert!(near.len() <= 1, "scale {k}: {near:?} at {corner:?}");
            if let Some(&distance) = near.first() {
                at_the_eaves += 1;
                assert!(
                    distance <= 1e-9 * k,
                    "scale {k}: {distance} from {corner:?}"
                );
            }
        }
        assert!(
            at_the_eaves >= 8,
            "scale {k}: {at_the_eaves} corners at the eaves"
        );
    }
}
