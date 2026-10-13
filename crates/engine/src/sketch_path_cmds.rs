//! `sketch.path`: a chain of lines and arcs through shared corners, drawn
//! by one command in one undo step.
//!
//! A driver that does not read responses cannot share a corner between a
//! line and an arc drawn by `sketch.line` and `sketch.arc`: the id of the
//! corner only comes back in the response. Here the corners are implied by
//! the order of the segments, so a slot, a rounded rectangle or a keyhole
//! is written from coordinates alone, and the profile detector sees one
//! closed loop.
//!
//! An arc is stored the way `sketch.arc` stores it: a centre point, and
//! counter-clockwise from its start to its end. A segment that is walked
//! clockwise is therefore stored with its two ends swapped; the corners it
//! shares with its neighbours are the same either way.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::sketch_cmds::{PointArg, change, coordinates, ids, sketch_ref, sketch_result};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{ConstraintId, ConstraintKind, EntityId, Sketch, SketchError};
use serde_json::{Map, Value, json};

const COMMAND: &str = "sketch.path";

/// Most segments one `sketch.path` takes.
const MAX_PATH_SEGMENTS: usize = 1_000;

/// What a segment may hold.
const SEGMENT_KEYS: [&str; 8] = [
    "line",
    "arc",
    "center",
    "through",
    "radius",
    "large",
    "clockwise",
    "tangent",
];

pub(crate) static SPECS: &[CommandSpec] = &[
    // The keys of a segment are spelled out in the summary, not in the
    // params: `panic_hunt` reads every quoted word before a colon in the
    // params as a parameter of the command itself.
    spec!("sketch.path", "Sketch Path", r#"{"sketch": id or name, "start": point id or [x, y], "segments": [segment, ...], "closed": boolean = false, "construction": boolean = false}"#,
        "Adds a chain of lines and arcs in one undo step. Each segment runs from where the one before it ended (the first from start) to its own end, and consecutive segments share that corner point. A segment is {\"line\": end}, {\"arc\": end, \"center\": point id or [x, y], \"clockwise\": boolean = false}, {\"arc\": end, \"through\": [x, y]}, {\"arc\": end, \"radius\": number, \"large\": boolean = false, \"clockwise\": boolean = false} or {\"arc\": end, \"tangent\": true}. An end is a point id, coordinates [x, y], or the word \"start\" for the path's own start point, which closes the loop with that segment; closed adds a straight line back to the start instead. An arc is given by its center (walked counter-clockwise unless clockwise), by a point it passes through, by its radius (the shorter way round unless large; counter-clockwise unless clockwise), or by tangent alone: it then leaves in the direction the segment before it arrives. \"tangent\": true on any segment, a line too, also adds a tangent constraint between it and the segment before it; on the first segment that is the last one, in a path that ends at its start. Returns points (the corners in order, the start first), entities (one per segment, entities[i] from points[i] to points[i+1], then the closing line), centers (the centre point of each entity, null for a line) and constraints (the tangent constraints added).", doc: true, sketch_path),
];

type Xy = (f64, f64);

/// Where a segment ends.
#[derive(Clone, Copy)]
enum End {
    /// At the path's own start point.
    Start,
    Point(PointArg),
}

/// How a segment gets from the corner before it to its end.
#[derive(Clone, Copy)]
enum Shape {
    Line,
    Center {
        center: PointArg,
        clockwise: bool,
    },
    Through(Xy),
    Radius {
        radius: f64,
        large: bool,
        clockwise: bool,
    },
    /// Leaving in the direction the segment before it arrives.
    Tangent,
}

#[derive(Clone, Copy)]
struct Segment {
    end: End,
    shape: Shape,
    /// Add a tangent constraint with the segment before it.
    tangent: bool,
}

struct Path {
    start: PointArg,
    segments: Vec<Segment>,
    closed: bool,
    construction: bool,
    /// The document's linear tolerance: what counts as no length at all.
    tolerance: f64,
}

/// What the command made, in the order of the path.
struct Built {
    points: Vec<EntityId>,
    entities: Vec<EntityId>,
    centers: Vec<Option<EntityId>>,
    constraints: Vec<ConstraintId>,
}

// ---------------------------------------------------------------- parsing

fn shown(text: &str) -> String {
    text.chars().take(40).collect()
}

fn segment_bool(
    at: &str,
    map: &Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, CommandError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => fail(format!("{at}: `{key}` must be true or false")),
    }
}

fn parse_segment(index: usize, value: &Value) -> Result<Segment, CommandError> {
    let at = format!("{COMMAND}: segments[{index}]");
    let Value::Object(map) = value else {
        return fail(format!(
            "{at} must be an object such as {{\"line\": [x, y]}} or {{\"arc\": [x, y], \"center\": [x, y]}}"
        ));
    };
    if let Some(unknown) = map.keys().find(|k| !SEGMENT_KEYS.contains(&k.as_str())) {
        return fail(format!(
            "{at}: unknown key `{}`; a segment holds: {}",
            shown(unknown),
            SEGMENT_KEYS.join(", ")
        ));
    }
    // As in the parameters themselves, null counts as absent.
    let get = |key: &str| map.get(key).filter(|v| !v.is_null());
    let (key, is_arc, end) = match (get("line"), get("arc")) {
        (Some(end), None) => ("line", false, end),
        (None, Some(end)) => ("arc", true, end),
        (Some(_), Some(_)) => {
            return fail(format!(
                "{at} has both `line` and `arc`; a segment is one of them"
            ));
        }
        (None, None) => {
            return fail(format!(
                "{at} needs `line` or `arc` with the point it ends at"
            ));
        }
    };
    let end = match end {
        Value::String(word) if word == "start" => End::Start,
        Value::Array(_) | Value::Number(_) | Value::String(_) => End::Point(PointArg::parse(
            COMMAND,
            &format!("segments[{index}].{key}"),
            end,
        )?),
        _ => {
            return fail(format!(
                "{at}: `{key}` must be the point the segment ends at: a point id, coordinates [x, y] or \"start\""
            ));
        }
    };
    let tangent = segment_bool(&at, map, "tangent")?.unwrap_or(false);
    let large = segment_bool(&at, map, "large")?;
    let clockwise = segment_bool(&at, map, "clockwise")?;
    let (center, through, radius) = (get("center"), get("through"), get("radius"));

    if !is_arc {
        for (key, given) in [
            ("center", center.is_some()),
            ("through", through.is_some()),
            ("radius", radius.is_some()),
            ("large", large.is_some()),
            ("clockwise", clockwise.is_some()),
        ] {
            if given {
                return fail(format!("{at}: `{key}` belongs to an arc, not to a line"));
            }
        }
        return Ok(Segment {
            end,
            shape: Shape::Line,
            tangent,
        });
    }

    let shape = match (center, through, radius) {
        (Some(center), None, None) => Shape::Center {
            center: PointArg::parse(COMMAND, &format!("segments[{index}].center"), center)?,
            clockwise: clockwise.unwrap_or(false),
        },
        (None, Some(through), None) => match coordinates(through) {
            Some(point) => Shape::Through(point),
            None => {
                return fail(format!("{at}: `through` must be two finite numbers [x, y]"));
            }
        },
        (None, None, Some(radius)) => match radius.as_f64().filter(|r| r.is_finite() && *r > 0.0) {
            Some(radius) => Shape::Radius {
                radius,
                large: large.unwrap_or(false),
                clockwise: clockwise.unwrap_or(false),
            },
            None => {
                return fail(format!("{at}: `radius` must be a positive number"));
            }
        },
        (None, None, None) if tangent => Shape::Tangent,
        (None, None, None) => {
            return fail(format!(
                "{at}: an arc needs `center`, `through`, `radius` or `tangent`: true"
            ));
        }
        _ => {
            return fail(format!(
                "{at}: give one of `center`, `through` and `radius`, not several"
            ));
        }
    };
    if large.is_some() && !matches!(shape, Shape::Radius { .. }) {
        return fail(format!(
            "{at}: `large` chooses between the two arcs of a `radius`; this arc has none"
        ));
    }
    if clockwise.is_some() && !matches!(shape, Shape::Center { .. } | Shape::Radius { .. }) {
        return fail(format!(
            "{at}: `clockwise` goes with `center` or `radius`; a `through` point or a tangent already fixes the way round"
        ));
    }
    Ok(Segment {
        end,
        shape,
        tangent,
    })
}

// --------------------------------------------------------------- geometry

fn invalid<T>(what: impl Into<String>) -> Result<T, SketchError> {
    Err(SketchError::Invalid(what.into()))
}

fn sub(a: Xy, b: Xy) -> Xy {
    (a.0 - b.0, a.1 - b.1)
}

fn cross(a: Xy, b: Xy) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn finite(point: Xy) -> Option<Xy> {
    (point.0.is_finite() && point.1.is_finite()).then_some(point)
}

/// The unit vector along `v`; `None` when it has no length or overflows.
fn unit(v: Xy) -> Option<Xy> {
    let length = v.0.hypot(v.1);
    if !(length.is_finite() && length > 0.0) {
        return None;
    }
    finite((v.0 / length, v.1 / length))
}

/// The direction in which an arc about `center` passes `point`.
fn arc_direction(center: Xy, point: Xy, ccw: bool) -> Option<Xy> {
    let (x, y) = unit(sub(point, center))?;
    Some(if ccw { (-y, x) } else { (y, -x) })
}

/// The centre of the circle through `from`, `through` and `to`, and whether
/// that order goes counter-clockwise. The circumcentre is computed from
/// `from`, so the coordinates that are squared are differences.
fn arc_through(from: Xy, through: Xy, to: Xy, tolerance: f64) -> Result<(Xy, bool), SketchError> {
    let (b, c) = (sub(to, from), sub(through, from));
    let chord = b.0.hypot(b.1);
    let turn = cross(b, c);
    // How far the through point is from the chord: with nothing in
    // between, the three points are on one line and the circle is not
    // defined (or immeasurably large).
    let sagitta = turn.abs() / chord;
    if !(sagitta.is_finite() && sagitta > tolerance) {
        return invalid(
            "the `through` point is on the line through the arc's ends (or on one of them), so no arc passes through all three",
        );
    }
    let (bb, cc) = (b.0 * b.0 + b.1 * b.1, c.0 * c.0 + c.1 * c.1);
    let d = 2.0 * turn;
    let center = (
        from.0 + (c.1 * bb - b.1 * cc) / d,
        from.1 + (b.0 * cc - c.0 * bb) / d,
    );
    match finite(center) {
        // A through point to the left of the chord is passed on the way
        // round to the right: clockwise.
        Some(center) => Ok((center, turn < 0.0)),
        None => invalid("the arc through these three points is too large to compute"),
    }
}

/// The centre of an arc of `radius` from `from` to `to`.
fn arc_by_radius(
    from: Xy,
    to: Xy,
    radius: f64,
    large: bool,
    clockwise: bool,
    tolerance: f64,
) -> Result<Xy, SketchError> {
    let chord = sub(to, from);
    let length = chord.0.hypot(chord.1);
    let half = length / 2.0;
    if radius < half - tolerance {
        return invalid(format!(
            "a radius of {radius} cannot span the {length} between the arc's ends: it must be at least half of that, {half}"
        ));
    }
    // Within the tolerance of half the chord it is a half circle.
    let height = (radius * radius - half * half).max(0.0).sqrt();
    let Some((ux, uy)) = unit(chord) else {
        return invalid("the arc's ends are too far apart to compute");
    };
    // Walking counter-clockwise the centre is on the left; the short way
    // round it is then on the left of the chord too, the long way round on
    // its right. Clockwise is the mirror image.
    let side = if clockwise == large { 1.0 } else { -1.0 };
    let center = (
        (from.0 + to.0) / 2.0 - uy * height * side,
        (from.1 + to.1) / 2.0 + ux * height * side,
    );
    finite(center).ok_or_else(|| SketchError::Invalid("the arc is too large to compute".into()))
}

/// The centre of the arc that leaves `from` in `direction` (a unit vector)
/// and ends at `to`, and whether it goes counter-clockwise.
fn arc_tangent(from: Xy, to: Xy, direction: Xy, tolerance: f64) -> Result<(Xy, bool), SketchError> {
    let chord = sub(to, from);
    let normal = (-direction.1, direction.0);
    // How far the end is from the tangent line; the centre is on the same
    // side of it.
    let offset = normal.0 * chord.0 + normal.1 * chord.1;
    if !(offset.is_finite() && offset.abs() > tolerance) {
        return invalid(
            "the end is on the line along which the segment before it arrives, so the tangent arc to it is a straight line (or a full turn back); use a line, or give the arc a `center`, `through` or `radius`",
        );
    }
    let radius = (chord.0 * chord.0 + chord.1 * chord.1) / (2.0 * offset);
    let center = (from.0 + normal.0 * radius, from.1 + normal.1 * radius);
    match finite(center) {
        Some(center) => Ok((center, offset > 0.0)),
        None => invalid("the tangent arc is too large to compute"),
    }
}

// ---------------------------------------------------------------- drawing

/// The segment before the one being drawn.
#[derive(Clone, Copy)]
struct Drawn {
    entity: EntityId,
    /// The direction in which it arrives at its end.
    arrives: Xy,
}

/// One segment added to the sketch.
struct Step {
    end: EntityId,
    end_at: Xy,
    center: Option<EntityId>,
    drawn: Drawn,
}

fn draw_segment(
    s: &mut Sketch,
    path: &Path,
    segment: &Segment,
    start: EntityId,
    from: (EntityId, Xy),
    before: Option<Drawn>,
) -> Result<Step, SketchError> {
    let (from, from_at) = from;
    let tolerance = path.tolerance;
    let end = match segment.end {
        End::Start => start,
        End::Point(point) => point.place(s)?,
    };
    let end_at = s.point(end)?;
    let chord = sub(end_at, from_at);
    let length = chord.0.hypot(chord.1);
    if end == from || !(length.is_finite() && length > tolerance) {
        return invalid(if length.is_finite() {
            "it ends where it starts; a segment needs two different points"
        } else {
            "its ends are too far apart to compute"
        });
    }
    let (center, ccw) = match segment.shape {
        Shape::Line => {
            let entity = s.add_line(from, end, path.construction)?;
            let Some(arrives) = unit(chord) else {
                return invalid("its ends are too far apart to compute");
            };
            return Ok(Step {
                end,
                end_at,
                center: None,
                drawn: Drawn { entity, arrives },
            });
        }
        Shape::Center { center, clockwise } => (center.place(s)?, !clockwise),
        Shape::Through(through) => {
            let (center, ccw) = arc_through(from_at, through, end_at, tolerance)?;
            (s.add_point(center.0, center.1)?, ccw)
        }
        Shape::Radius {
            radius,
            large,
            clockwise,
        } => {
            let center = arc_by_radius(from_at, end_at, radius, large, clockwise, tolerance)?;
            (s.add_point(center.0, center.1)?, !clockwise)
        }
        Shape::Tangent => {
            let Some(before) = before else {
                return invalid(
                    "an arc given by `tangent` alone needs a segment before it to leave from; give the first segment a `center`, `through` or `radius`",
                );
            };
            let (center, ccw) = arc_tangent(from_at, end_at, before.arrives, tolerance)?;
            (s.add_point(center.0, center.1)?, ccw)
        }
    };
    let center_at = s.point(center)?;
    for (name, corner) in [("start", from_at), ("end", end_at)] {
        let radius = sub(corner, center_at);
        let radius = radius.0.hypot(radius.1);
        if !(radius.is_finite() && radius > tolerance) {
            return invalid(format!(
                "the arc's centre is on its {name} (or immeasurably far from it): there is no radius"
            ));
        }
    }
    // Stored counter-clockwise from start to end, as `sketch.arc` does.
    let entity = if ccw {
        s.add_arc(center, from, end, path.construction)?
    } else {
        s.add_arc(center, end, from, path.construction)?
    };
    let Some(arrives) = arc_direction(center_at, end_at, ccw) else {
        return invalid("the arc's centre is on its end: there is no radius");
    };
    Ok(Step {
        end,
        end_at,
        center: Some(center),
        drawn: Drawn { entity, arrives },
    })
}

fn build(s: &mut Sketch, path: &Path) -> Result<Built, SketchError> {
    let within = |index: usize| {
        move |e: SketchError| SketchError::Invalid(format!("segments[{index}]: {e}"))
    };
    let start = path.start.place(s)?;
    let mut at = (start, s.point(start)?);
    let mut built = Built {
        points: vec![start],
        entities: Vec::with_capacity(path.segments.len().saturating_add(1)),
        centers: Vec::with_capacity(path.segments.len().saturating_add(1)),
        constraints: Vec::new(),
    };
    let mut first: Option<EntityId> = None;
    let mut before: Option<Drawn> = None;
    for (index, segment) in path.segments.iter().enumerate() {
        let step = draw_segment(s, path, segment, start, at, before).map_err(within(index))?;
        // The first segment's tangency is with the last one, which is not
        // drawn yet.
        if let (true, Some(before)) = (segment.tangent, before) {
            let tangent = ConstraintKind::Tangent {
                a: before.entity,
                b: step.drawn.entity,
            };
            built
                .constraints
                .push(s.add_constraint(tangent).map_err(within(index))?);
        }
        first.get_or_insert(step.drawn.entity);
        built.points.push(step.end);
        built.entities.push(step.drawn.entity);
        built.centers.push(step.center);
        at = (step.end, step.end_at);
        before = Some(step.drawn);
    }
    if path.closed {
        if at.0 == start {
            return invalid(
                "the path already ends at its start, so `closed` has no line to add; leave it out",
            );
        }
        if let [only] = path.segments.as_slice()
            && matches!(only.shape, Shape::Line)
        {
            return invalid(
                "`closed` would add a second line on top of the only one; a closed path needs an arc or at least two lines",
            );
        }
        let (dx, dy) = sub(s.point(start)?, at.1);
        let length = dx.hypot(dy);
        if !(length.is_finite() && length > path.tolerance) {
            return invalid(
                "`closed` would add a line of no length: the last segment ends at the coordinates of the start; end it at \"start\" instead, so both are one point",
            );
        }
        let line = s.add_line(at.0, start, path.construction)?;
        built.entities.push(line);
        built.centers.push(None);
        at.0 = start;
        if let (Some(arrives), Some(last)) = (unit((dx, dy)), before.as_mut()) {
            *last = Drawn {
                entity: line,
                arrives,
            };
        }
    }
    let first_tangent = path.segments.first().is_some_and(|segment| segment.tangent);
    if let (true, Some(first), Some(last)) = (first_tangent, first, before) {
        if at.0 != start {
            return invalid(
                "segments[0]: `tangent` on the first segment is with the last one, so the path must end at its start: end the last segment at \"start\", or set `closed`",
            );
        }
        let tangent = ConstraintKind::Tangent {
            a: last.entity,
            b: first,
        };
        built
            .constraints
            .push(s.add_constraint(tangent).map_err(within(0))?);
    }
    Ok(built)
}

// ---------------------------------------------------------------- command

fn sketch_path(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        COMMAND,
        params,
        &["sketch", "start", "segments", "closed", "construction"],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let start = PointArg::required(&p, COMMAND, "start")?;
    let Some(Value::Array(items)) = p.get("segments") else {
        return match p.get("segments") {
            None => Err(p.missing("segments")),
            Some(_) => fail(format!(
                "{COMMAND}: `segments` must be an array of segments such as {{\"line\": [x, y]}}"
            )),
        };
    };
    if items.is_empty() {
        return fail(format!("{COMMAND}: a path needs at least one segment"));
    }
    if items.len() > MAX_PATH_SEGMENTS {
        return fail(format!(
            "{COMMAND}: at most {MAX_PATH_SEGMENTS} segments at a time, got {}",
            items.len()
        ));
    }
    let mut segments = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        segments.push(parse_segment(index, item)?);
    }
    let tolerance = session.document()?.history.current().tolerance.linear;
    let path = Path {
        start,
        segments,
        closed: p.bool("closed")?.unwrap_or(false),
        construction: p.bool("construction")?.unwrap_or(false),
        // A document's tolerance is a positive number; were it not, only a
        // segment of exactly no length would be refused.
        tolerance: if tolerance.is_finite() && tolerance > 0.0 {
            tolerance
        } else {
            0.0
        },
    };
    let built = change(session, COMMAND, "Add path", id, |s| build(s, &path))?;
    let centers: Vec<Value> = built
        .centers
        .iter()
        .map(|center| center.map_or(Value::Null, |id| id.to_string().into()))
        .collect();
    let constraints: Vec<String> = built.constraints.iter().map(|c| c.to_string()).collect();
    sketch_result(
        session,
        id,
        json!({
            "points": ids(&built.points),
            "entities": ids(&built.entities),
            "centers": centers,
            "constraints": constraints,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-9;

    fn near(a: Xy, b: Xy) -> bool {
        (a.0 - b.0).abs() <= 1e-12 && (a.1 - b.1).abs() <= 1e-12
    }

    #[test]
    fn three_points_give_their_circle_and_the_way_round() {
        // The unit circle about (3, -2), through its east, north and west
        // points: counter-clockwise in that order, clockwise in reverse.
        let (e, n, w) = ((4.0, -2.0), (3.0, -1.0), (2.0, -2.0));
        let (center, ccw) = arc_through(e, n, w, TOL).unwrap();
        assert!(near(center, (3.0, -2.0)) && ccw, "{center:?} {ccw}");
        let (center, ccw) = arc_through(w, n, e, TOL).unwrap();
        assert!(near(center, (3.0, -2.0)) && !ccw, "{center:?} {ccw}");
        // Collinear, and a through point on an end.
        assert!(arc_through(e, (3.0, -2.0), w, TOL).is_err());
        assert!(arc_through(e, e, w, TOL).is_err());
        assert!(arc_through(e, (f64::MAX, -f64::MAX), w, TOL).is_err());
    }

    #[test]
    fn a_radius_gives_one_of_four_centres() {
        // A chord of 6 along x and a radius of 5: the centres are 4 above
        // and below its middle.
        let (a, b) = ((0.0, 0.0), (6.0, 0.0));
        for (large, clockwise, expected) in [
            (false, false, (3.0, 4.0)),
            (true, false, (3.0, -4.0)),
            (false, true, (3.0, -4.0)),
            (true, true, (3.0, 4.0)),
        ] {
            let center = arc_by_radius(a, b, 5.0, large, clockwise, TOL).unwrap();
            assert!(near(center, expected), "{large} {clockwise}: {center:?}");
        }
        // Exactly half the chord is a half circle; less is no arc.
        let center = arc_by_radius(a, b, 3.0, false, false, TOL).unwrap();
        assert!(near(center, (3.0, 0.0)), "{center:?}");
        assert!(arc_by_radius(a, b, 2.999, false, false, TOL).is_err());
        assert!(arc_by_radius(a, (f64::MAX, 0.0), f64::MAX, false, false, TOL).is_ok());
        assert!(arc_by_radius((-f64::MAX, 0.0), (f64::MAX, 0.0), 1.0, false, false, TOL).is_err());
    }

    #[test]
    fn a_tangent_arc_turns_towards_its_end() {
        // Leaving the origin along +x: an end above is reached turning
        // left (counter-clockwise), about a centre on the y axis.
        let (center, ccw) = arc_tangent((0.0, 0.0), (2.0, 2.0), (1.0, 0.0), TOL).unwrap();
        assert!(near(center, (0.0, 2.0)) && ccw, "{center:?} {ccw}");
        let (center, ccw) = arc_tangent((0.0, 0.0), (0.0, -4.0), (1.0, 0.0), TOL).unwrap();
        assert!(near(center, (0.0, -2.0)) && !ccw, "{center:?} {ccw}");
        // Straight ahead and straight back are not arcs.
        assert!(arc_tangent((0.0, 0.0), (5.0, 0.0), (1.0, 0.0), TOL).is_err());
        assert!(arc_tangent((0.0, 0.0), (-5.0, 0.0), (1.0, 0.0), TOL).is_err());
    }
}
