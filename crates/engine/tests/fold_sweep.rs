//! fold-check, step 1 (a probe, printed and not held to anything): the
//! rule the design proposes for the validity check, a face regular at the
//! tolerance (docs/design/verification.md › *A face folded over itself*),
//! asked of every freeform face of every body the repository's documents
//! and scripts build today: which faces it would refuse, where, how near
//! a vertex of the face, and what it costs. The rule here is the probe's
//! form of it: every Bézier span of the face's surface whose offsets by
//! plus and minus the body's working tolerance `geom::offset::regular`
//! proves not to fold (no radius of curvature below the tolerance, a
//! normal everywhere), the place of each span that is not, whether it lies
//! in the face (`topo::face_holds`) and its distance from the nearest
//! vertex of the face. Lines `SWEEP …` (with `-- --nocapture`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

mod common;

use craft3d_doc::Document;
use craft3d_geom::offset::{OffsetError, regular};
use craft3d_geom::{NurbsSurface, RationalBezierPatch, Surface};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, FaceId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Fnv(u64);

impl std::fmt::Write for Fnv {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for b in s.bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Ok(())
    }
}

fn hash(value: &impl std::fmt::Debug) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    write!(h, "{value:?}").unwrap();
    h.0
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One Bézier span as a surface of its own.
fn span_surface(patch: &RationalBezierPatch) -> Option<Surface> {
    let (m, n) = patch.degrees();
    let clamped = |d: usize| {
        let mut k = vec![0.0; d + 1];
        k.extend(vec![1.0; d + 1]);
        k
    };
    NurbsSurface::new(
        m,
        n,
        clamped(m),
        clamped(n),
        m + 1,
        n + 1,
        patch.control_points(),
        patch.weights(),
    )
    .ok()
    .map(Surface::Nurbs)
}

/// A place on a surface the rule does not prove: what, where.
#[derive(Clone, Debug)]
struct Place {
    what: &'static str,
    uv: (f64, f64),
    radius: f64,
}

/// What the rule says of one surface at the tolerance `t`: the spans, the
/// spans proven, and the places it does not prove.
#[derive(Clone, Debug, Default)]
struct Verdict {
    spans: usize,
    proven: usize,
    places: Vec<Place>,
}

fn verdict(surface: &Surface, t: f64) -> Option<Verdict> {
    let nurbs = match surface {
        Surface::Nurbs(n) => n.clone(),
        Surface::Bezier(b) => NurbsSurface::from_bezier(b).ok()?,
        _ => return None,
    };
    let spans = nurbs.bezier_spans().ok()?;
    let mut out = Verdict {
        spans: spans.len(),
        ..Verdict::default()
    };
    for span in &spans {
        let Some(own) = span_surface(&span.patch) else {
            out.places.push(Place {
                what: "no surface",
                uv: (span.u.0, span.v.0),
                radius: f64::NAN,
            });
            continue;
        };
        let global = |at: (f64, f64)| {
            (
                span.u.0 + (span.u.1 - span.u.0) * at.0,
                span.v.0 + (span.v.1 - span.v.0) * at.1,
            )
        };
        let mut found = Ok(());
        for d in [t, -t] {
            found = regular(&own, d);
            if found.is_err() {
                break;
            }
        }
        match found {
            Ok(()) => out.proven += 1,
            Err(OffsetError::NoNormal { at }) => out.places.push(Place {
                what: "no normal",
                uv: global(at),
                radius: 0.0,
            }),
            Err(OffsetError::Folds { at, radius }) => out.places.push(Place {
                what: "fold",
                uv: global(at),
                radius,
            }),
            Err(OffsetError::Unproven { at }) => out.places.push(Place {
                what: "unproven",
                uv: global(at),
                radius: f64::NAN,
            }),
            Err(other) => out.places.push(Place {
                what: if matches!(other, OffsetError::Spent) {
                    "spent"
                } else {
                    "other"
                },
                uv: (span.u.0, span.v.0),
                radius: f64::NAN,
            }),
        }
    }
    Some(out)
}

#[derive(Default)]
struct Tally {
    documents: usize,
    bodies: usize,
    faces: usize,
    freeform: usize,
    spans: usize,
    proven: usize,
    places: BTreeMap<String, usize>,
    nanos: u128,
}

/// The rule asked of every freeform face of `body`, each surface once
/// (`seen`, by its debug text and the tolerance).
fn sweep_body(
    name: &str,
    feature: &str,
    body: &Body,
    tol: &Tolerance,
    seen: &mut BTreeMap<u64, Verdict>,
    tally: &mut Tally,
) {
    tally.bodies += 1;
    let t = body.working_tolerance(tol.linear);
    for (index, face) in body.faces.iter().enumerate() {
        tally.faces += 1;
        if !matches!(face.surface, Surface::Nurbs(_) | Surface::Bezier(_)) {
            continue;
        }
        tally.freeform += 1;
        let key = hash(&(&face.surface, t.to_bits()));
        let start = Instant::now();
        let found = if let Some(v) = seen.get(&key) {
            v.clone()
        } else {
            let Some(v) = verdict(&face.surface, t) else {
                continue;
            };
            seen.insert(key, v.clone());
            tally.nanos += start.elapsed().as_nanos();
            tally.spans += v.spans;
            tally.proven += v.proven;
            v
        };
        if found.places.is_empty() {
            continue;
        }
        // Where each place is: in the face or not, how far from the
        // nearest vertex of the face.
        let mut corners: Vec<Vec3> = Vec::new();
        for lp in &face.loops {
            if let Some(lp) = body.loop_(*lp) {
                for coedge in &lp.coedges {
                    if let Some(edge) = body.edge(coedge.edge) {
                        for v in [edge.start, edge.end] {
                            if let Some(vertex) = body.vertex(v) {
                                corners.push(vertex.point);
                            }
                        }
                    }
                }
            }
        }
        let points: Vec<Vec3> = found
            .places
            .iter()
            .map(|p| face.surface.eval(p.uv.0, p.uv.1))
            .collect();
        let inside = craft3d_topo::face_holds(body, face, tol, &points);
        for (k, (place, p)) in found.places.iter().zip(&points).enumerate() {
            let near = corners
                .iter()
                .map(|c| c.distance(*p))
                .fold(f64::INFINITY, f64::min);
            let held = match &inside {
                Some(list) => match list.get(k) {
                    Some(true) => "in",
                    Some(false) => "out",
                    None => "?",
                },
                None => "?",
            };
            let class = format!(
                "{} {} {}",
                place.what,
                held,
                if near <= 32.0 * t {
                    "near-vertex"
                } else {
                    "away"
                }
            );
            *tally.places.entry(class.clone()).or_default() += 1;
            println!(
                "SWEEP {name} | {feature} | face {} | {class} | at ({:.6}, {:.6}, {:.6}) uv ({:.5}, {:.5}) radius {:.3e} ({:.3} t) | nearest vertex {:.3e} ({:.1} t) | t {:e}",
                FaceId(index as u32).0,
                p.x,
                p.y,
                p.z,
                place.uv.0,
                place.uv.1,
                place.radius,
                place.radius / t,
                near,
                near / t,
                t
            );
        }
    }
}

fn sweep_document(name: &str, doc: &Document, tally: &mut Tally) {
    tally.documents += 1;
    let Some(tol) = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular) else {
        println!("SWEEP {name} | no tolerance");
        return;
    };
    let regen = craft3d_regen::regenerate(doc);
    let mut seen: BTreeMap<u64, Verdict> = BTreeMap::new();
    let mut bodies: BTreeMap<usize, ()> = BTreeMap::new();
    for (feature, result) in doc.features.iter().zip(&regen.features) {
        let Some(body) = &result.body else {
            continue;
        };
        let address = std::sync::Arc::as_ptr(body) as usize;
        if bodies.insert(address, ()).is_some() {
            continue;
        }
        sweep_body(name, &feature.name, body, &tol, &mut seen, tally);
    }
}

fn steps(path: &Path) -> Vec<(String, Value)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let list = match &value {
        Value::Array(list) => list.clone(),
        Value::Object(map) => map
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    list.iter()
        .filter_map(|step| match step {
            Value::Array(pair) => Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.clone())),
            Value::Object(map) => Some((
                map.get("command")?.as_str()?.to_string(),
                map.get("params").cloned().unwrap_or(json!({})),
            )),
            _ => None,
        })
        .collect()
}

fn listed(folder: &str, suffix: &str) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(root().join(folder)) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(suffix))
        .collect();
    found.sort();
    found
}

fn files(folders: &[&str], tag: &str) {
    let mut tally = Tally::default();
    for folder in folders {
        for path in listed(folder, ".3dcraft") {
            let name = format!("{folder}/{}", path.file_name().unwrap().to_string_lossy());
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            match craft3d_format::from_bytes(&bytes) {
                Ok(doc) => sweep_document(&name, &doc, &mut tally),
                Err(e) => println!("SWEEP {name} | unreadable {e}"),
            }
        }
    }
    said(tag, &tally);
}

fn scripts(folders: &[(&str, &str)], tag: &str) {
    let mut tally = Tally::default();
    let dir = common::workdir("fold-sweep", tag);
    for (folder, suffix) in folders {
        for path in listed(folder, suffix) {
            let name = format!("{folder}/{}", path.file_name().unwrap().to_string_lossy());
            let list = steps(&path);
            if list.is_empty() {
                continue;
            }
            let mut s = common::session_in(&dir);
            if list.first().is_none_or(|(id, _)| id != "file.new") {
                let _ = s.execute("file.new", &json!({"units": "mm"}));
            }
            for (id, params) in list {
                if (id.starts_with("file.")
                    || id.starts_with("view.")
                    || id.starts_with("ui.")
                    || id.starts_with("assembly.")
                    || id == "document.check")
                    && id != "file.new"
                {
                    continue;
                }
                let _ = s.execute(&id, &params);
            }
            match s.document_bytes().map(|b| craft3d_format::from_bytes(&b)) {
                Some(Ok(doc)) => sweep_document(&name, &doc, &mut tally),
                Some(Err(e)) => println!("SWEEP {name} | unreadable {e}"),
                None => println!("SWEEP {name} | no document"),
            }
        }
    }
    said(tag, &tally);
}

fn said(tag: &str, tally: &Tally) {
    println!(
        "SWEEP tally {tag}: {} documents, {} bodies, {} faces, {} freeform, {} spans asked, {} proven, places {:?}, {:.1} ms in the rule",
        tally.documents,
        tally.bodies,
        tally.faces,
        tally.freeform,
        tally.spans,
        tally.proven,
        tally.places,
        tally.nanos as f64 / 1e6
    );
}

#[test]
fn published_and_showcase() {
    files(
        &[
            "docs/examples/published",
            "docs/showcase",
            "crates/engine/tests/fillet_fade",
            "crates/engine/tests/stale_sketches",
        ],
        "files",
    );
}

#[test]
fn format_fixtures() {
    files(
        &[
            "crates/format/tests/fixtures/versions",
            "crates/format/tests/fixtures/migrations",
        ],
        "fixtures",
    );
}

#[test]
fn examples_tasks_and_benchmarks() {
    scripts(
        &[
            ("docs/examples", ".json"),
            ("docs/tasks", ".json"),
            ("crates/engine/tests/example_models", ".json"),
            ("crates/engine/tests/bench_models", ".steps.json"),
            ("crates/engine/tests/edit_path", ".steps.json"),
            ("crates/engine/tests/self_check", ".json"),
        ],
        "scripts",
    );
}

#[test]
fn findings_scripts() {
    scripts(
        &[
            ("crates/engine/tests/blend_findings", ".json"),
            ("crates/engine/tests/fuzz_findings", ".json"),
            ("crates/engine/tests/kayak_findings", ".json"),
            ("crates/engine/tests/boolean_findings", ".json"),
        ],
        "findings",
    );
}
