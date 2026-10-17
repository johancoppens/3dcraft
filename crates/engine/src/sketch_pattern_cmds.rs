//! Patterns inside a sketch: `sketch.pattern` makes or edits one, and the
//! JSON `sketch.inspect` lists them with.
//!
//! A pattern repeats seed entities as rigid images of their solved shape:
//! in a row or a grid (`step`, `count2`, `step2`), as a chained run whose
//! instances join end to start (`chain`), or round a centre (`angle`,
//! `center`, and `chain` for a ring of runs). The count and the steps are
//! expressions; the instances are made at regeneration and not stored
//! (docs/design/document.md, *Patterns inside a sketch*).

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::sketch_cmds::{EntityArg, change, entity_id, ids, local_id, sketch_ref, sketch_result};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{
    CenterRef, CurveIndex, CurvePath, EntityId, Expr, Geometry, InstanceIndex,
    MAX_PATTERN_INSTANCES, PatternId, SecondDirection, Sketch, SketchPatternLayout,
};
use craft3d_regen::{ConstraintState, Placed, SketchResult};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "sketch.pattern",
    "Sketch Pattern",
    r#"{"sketch": id or name, "entities": [entity id, ...], "patterns": [pattern id, ...], "count": expr, "step": [expr, expr], "count2": expr, "step2": [expr, expr], "chain": [point id, point id], "angle": expr, "center": point id or [expr, expr], "skip": [instance, ...], "replace": pattern id}"#,
    "Repeats seed entities of a sketch (lines, arcs, circles, splines and points; the points of curves come along) as rigid images of their solved shape: count instances, the seed's included (a whole number of at least 1, an expression such as \"floor(span / pitch)\"). step makes a row, instance k moved by k * step in the sketch's axes; count2 and step2 add rows: a grid. chain alone makes a chained run: the seed runs from the first point to the second, and each instance starts where the one before ends (the step is to - from, as solved), so the joints are shared exactly and a profile closes at any count. angle with center (a point of the sketch, or fixed coordinates) turns instance k by k * angle counter-clockwise; with chain as well it is a ring of runs, whose end has to be its start turned by angle. The instances are made at every regeneration from the solved seed and named after it: e7[3] is instance 3 of the seed curve e7 (e7[2,4] in a grid), so F2:side(e7[3]) is the side of instance 3, and a reference to an instance that a smaller count no longer has fails rather than moving to another. A point of an instance can be constrained or ended on where a point is taken: \"e5[3]\", or \"e5[last]\" for the last instance whatever the count (coincident with a fixed point, it lets the solver find the step of a run). patterns puts whole patterns in the seed (a pattern of a pattern, at most three levels deep; each pattern in one seed): the run of tabs p1 repeated cell by cell. Their instances are named innermost first, e3[2][1] being instance 2 of the inner pattern in instance 1 of the outer one, and e3[2] in instance 0 (so wrapping renames nothing); a point of them is \"e8[last][last]\", and a seed curve may start on an image point of a pattern in its seed (\"e8[last]\"), as may a chain. skip leaves out instances of this pattern's own level, by number (3, \"3\", or [2, 4] / \"2,4\" in a grid): never instance 0; one past the count is kept and back when the count grows; not in a chained run or a ring yet. replace edits pattern \"p1\": what is not given stays. Returns the pattern's id and its instances.",
    doc: true,
    sketch_pattern
)];

/// A pattern id written `"p3"` or `3`.
pub(crate) fn pattern_id(
    command: &str,
    key: &str,
    value: &Value,
) -> Result<PatternId, CommandError> {
    local_id(command, key, 'p', value).map(PatternId)
}

/// Two expressions `[x, y]`.
fn expr2(p: &Params, key: &str) -> Result<Option<[Expr; 2]>, CommandError> {
    let Some(value) = p.get(key) else {
        return Ok(None);
    };
    let wrong = || {
        let shown: String = value.to_string().chars().take(40).collect();
        CommandError::new(format!(
            "{}: `{key}` must be two expressions [x, y], got {shown}",
            p.command()
        ))
    };
    let Value::Array(items) = value else {
        return Err(wrong());
    };
    let [x, y] = items.as_slice() else {
        return Err(wrong());
    };
    Ok(Some([
        Expr::new(p.expr_of(key, x)?),
        Expr::new(p.expr_of(key, y)?),
    ]))
}

/// The two points of `chain`: ids, or image points of a pattern in the
/// seed (`"e8[last]"`, made in the same edit).
fn chain_param(p: &Params) -> Result<Option<[EntityArg; 2]>, CommandError> {
    let Some(value) = p.get("chain") else {
        return Ok(None);
    };
    let command = p.command();
    match value.as_array().map(Vec::as_slice) {
        Some([from, to]) => Ok(Some([
            EntityArg::parse(command, "chain", from)?,
            EntityArg::parse(command, "chain", to)?,
        ])),
        _ => fail(format!(
            "{command}: `chain` must be two point ids [from, to], the ends of the seed's run"
        )),
    }
}

/// The id an entity argument stands for before its image point is made:
/// the seed point of an image, which [`with_chain`] replaces.
fn placeholder(arg: EntityArg) -> EntityId {
    match arg {
        EntityArg::Id(id) => id,
        EntityArg::Image(of, _) => of,
    }
}

/// The layout with the ends of its chain as they are in the sketch, the
/// image points among them made.
fn with_chain(
    sketch: &mut Sketch,
    mut layout: SketchPatternLayout,
    chain: Option<[EntityArg; 2]>,
) -> Result<SketchPatternLayout, craft3d_doc::SketchError> {
    let Some([from, to]) = chain else {
        return Ok(layout);
    };
    let (a, b) = (from.place(sketch)?, to.place(sketch)?);
    match &mut layout {
        SketchPatternLayout::Chain { from, to } => {
            (*from, *to) = (a, b);
        }
        SketchPatternLayout::Circular {
            chain: Some(ends), ..
        } => *ends = [a, b],
        _ => {}
    }
    Ok(layout)
}

fn center_param(p: &Params) -> Result<Option<CenterRef>, CommandError> {
    let Some(value) = p.get("center") else {
        return Ok(None);
    };
    match value {
        Value::Array(_) => Ok(expr2(p, "center")?.map(|at| CenterRef::At { at })),
        other => Ok(Some(CenterRef::Point {
            point: entity_id(p.command(), "center", other)?,
        })),
    }
}

/// The layout the params give, if they give one; the ends of a chain that
/// are image points stand for their seed points until [`with_chain`].
fn layout_param(
    p: &Params,
    chain: Option<[EntityArg; 2]>,
) -> Result<Option<SketchPatternLayout>, CommandError> {
    let command = p.command();
    let step = expr2(p, "step")?;
    let step2 = expr2(p, "step2")?;
    let count2 = p.expr("count2")?.map(Expr::new);
    let chain = chain.map(|[a, b]| [placeholder(a), placeholder(b)]);
    let angle = p.expr("angle")?.map(Expr::new);
    let center = center_param(p)?;
    if step2.is_some() != count2.is_some() {
        return fail(format!(
            "{command}: `count2` and `step2` go together: the rows of a grid and the step between them"
        ));
    }
    if step2.is_some() && step.is_none() {
        return fail(format!(
            "{command}: a grid has a `step` along its rows as well as `step2` between them"
        ));
    }
    match (step, angle, center) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => fail(format!(
            "{command}: give `step` for a row or a grid, or `angle` and `center` to go round, not both"
        )),
        (Some(_), None, None) if chain.is_some() => fail(format!(
            "{command}: a chained run takes its step from its seed (to - from); give `chain` without `step`"
        )),
        (Some(step), None, None) => Ok(Some(SketchPatternLayout::Linear {
            step,
            second: match (step2, count2) {
                (Some(step), Some(count)) => Some(SecondDirection { step, count }),
                _ => None,
            },
        })),
        (None, Some(angle), Some(center)) => Ok(Some(SketchPatternLayout::Circular {
            center,
            angle,
            chain,
        })),
        (None, Some(_), None) => fail(format!(
            "{command}: `angle` needs a `center` to turn about: a point id or coordinates [x, y]"
        )),
        (None, None, Some(_)) => fail(format!(
            "{command}: `center` needs an `angle`: the turn from one instance to the next"
        )),
        (None, None, None) => match chain {
            Some([from, to]) => Ok(Some(SketchPatternLayout::Chain { from, to })),
            None => Ok(None),
        },
    }
}

/// The instances `skip` leaves out: numbers (`3`, `"3"`) or, in a grid,
/// pairs (`[2, 4]`, `"2,4"`), in ascending order without repeats.
fn skip_param(p: &Params) -> Result<Option<Vec<InstanceIndex>>, CommandError> {
    let command = p.command();
    let Some(value) = p.get("skip") else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail(format!(
            "{command}: `skip` must be an array of instances, such as [3, 5] or [[2, 4]] in a grid"
        ));
    };
    if items.len() > MAX_PATTERN_INSTANCES {
        return fail(format!(
            "{command}: a pattern skips at most {MAX_PATTERN_INSTANCES} instances"
        ));
    }
    let wrong = |item: &Value| {
        let shown: String = item.to_string().chars().take(40).collect();
        CommandError::new(format!(
            "{command}: `skip`: {shown} is not an instance; give a whole number from 1 (3 or \"3\"), or two in a grid ([2, 4] or \"2,4\")"
        ))
    };
    let number = |item: &Value| -> Result<u32, CommandError> {
        item.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| wrong(item))
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let index = match item {
            Value::Number(_) => InstanceIndex::At(number(item)?),
            Value::String(text) => match InstanceIndex::parse(text) {
                Some(index @ (InstanceIndex::At(_) | InstanceIndex::Grid(_))) => index,
                _ => return Err(wrong(item)),
            },
            Value::Array(pair) => match pair.as_slice() {
                [i, j] => InstanceIndex::Grid([number(i)?, number(j)?]),
                _ => return Err(wrong(item)),
            },
            _ => return Err(wrong(item)),
        };
        out.push(index);
    }
    let key = |index: &InstanceIndex| match *index {
        InstanceIndex::At(k) => (0, k, 0),
        InstanceIndex::Grid([i, j]) => (1, i, j),
        InstanceIndex::Last => (2, 0, 0),
    };
    out.sort_by_key(key);
    out.dedup();
    Ok(Some(out))
}

fn sketch_pattern(session: &mut Session, params: &Value) -> Run {
    const COMMAND: &str = "sketch.pattern";
    let p = Params::new(
        COMMAND,
        params,
        &[
            "sketch", "entities", "patterns", "count", "step", "count2", "step2", "chain", "angle",
            "center", "skip", "replace",
        ],
    )?;
    let id = sketch_ref(session, &p, COMMAND)?;
    let seed = match p.get("entities") {
        None => None,
        Some(Value::Array(items)) => {
            if items.len() > craft3d_doc::sketch::MAX_SKETCH_ENTITIES {
                return fail(format!(
                    "{COMMAND}: a pattern repeats at most {} entities",
                    craft3d_doc::sketch::MAX_SKETCH_ENTITIES
                ));
            }
            let mut seed = Vec::with_capacity(items.len());
            for item in items {
                seed.push(entity_id(COMMAND, "entities", item)?);
            }
            Some(seed)
        }
        Some(_) => {
            return fail(format!(
                "{COMMAND}: `entities` must be an array of entity ids, such as [\"e5\", \"e6\"]"
            ));
        }
    };
    let inner = match p.get("patterns") {
        None => None,
        Some(Value::Array(items)) => {
            if items.len() > craft3d_doc::MAX_SKETCH_PATTERNS {
                return fail(format!(
                    "{COMMAND}: a pattern repeats at most {} patterns",
                    craft3d_doc::MAX_SKETCH_PATTERNS
                ));
            }
            let mut inner = Vec::with_capacity(items.len());
            for item in items {
                inner.push(pattern_id(COMMAND, "patterns", item)?);
            }
            Some(inner)
        }
        Some(_) => {
            return fail(format!(
                "{COMMAND}: `patterns` must be an array of pattern ids, such as [\"p1\"]"
            ));
        }
    };
    let skip = skip_param(&p)?;
    let count = p.expr("count")?.map(Expr::new);
    let chain = chain_param(&p)?;
    let layout = layout_param(&p, chain)?;
    let replace = p
        .get("replace")
        .map(|v| pattern_id(COMMAND, "replace", v))
        .transpose()?;

    let (pattern, removal) = match replace {
        None => {
            let seed = match (seed, &inner) {
                (Some(seed), _) => seed,
                (None, Some(inner)) if !inner.is_empty() => Vec::new(),
                (None, _) => return Err(p.missing("entities")),
            };
            let Some(count) = count else {
                return Err(p.missing("count"));
            };
            let Some(layout) = layout else {
                return fail(format!(
                    "{COMMAND}: give `step` (a row; with `count2` and `step2` a grid), `chain` (a chained run), or `angle` and `center` (round a centre)"
                ));
            };
            let made = change(session, COMMAND, "Add sketch pattern", id, |s| {
                let layout = with_chain(s, layout, chain)?;
                s.add_nested_pattern(
                    seed,
                    inner.unwrap_or_default(),
                    count,
                    layout,
                    skip.unwrap_or_default(),
                )
            })?;
            (made, None)
        }
        Some(pattern) => {
            let removal = change(session, COMMAND, "Edit sketch pattern", id, |s| {
                let Some(old) = s.pattern(pattern).cloned() else {
                    return Err(craft3d_doc::SketchError::Invalid(format!(
                        "the sketch has no pattern {pattern}"
                    )));
                };
                let layout = match layout {
                    Some(layout) => with_chain(s, layout, chain)?,
                    None => old.layout,
                };
                s.replace_nested_pattern(
                    pattern,
                    seed.unwrap_or(old.seed),
                    inner.unwrap_or(old.patterns),
                    count.unwrap_or(old.count),
                    layout,
                    skip.unwrap_or(old.skip),
                )
            })?;
            (pattern, Some(removal))
        }
    };
    let instances = {
        let open = session.document_mut()?;
        let regen = open.regen();
        regen
            .feature(id)
            .and_then(|f| f.sketch.as_ref())
            .and_then(|s| s.patterns.iter().find(|r| r.id == pattern))
            .map(|r| r.instances())
    };
    let mut own = json!({ "pattern": pattern.to_string() });
    if let Some(map) = own.as_object_mut() {
        map.insert(
            "instances".into(),
            instances.map_or(Value::Null, Value::from),
        );
        if let Some(removal) = removal {
            if !removal.images.is_empty() {
                map.insert("imagesDeleted".into(), ids(&removal.images).into());
            }
            if !removal.constraints.is_empty() {
                let gone: Vec<String> = removal
                    .constraints
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                map.insert("constraintsDeleted".into(), gone.into());
            }
            if !removal.patterns.is_empty() {
                let gone: Vec<String> = removal.patterns.iter().map(ToString::to_string).collect();
                map.insert("patternsDeleted".into(), gone.into());
            }
        }
    }
    sketch_result(session, id, own)
}

fn state_name(state: ConstraintState) -> &'static str {
    match state {
        ConstraintState::Satisfied => "satisfied",
        ConstraintState::Redundant => "redundant",
        ConstraintState::Conflicting => "conflicting",
        ConstraintState::Unsolved => "unsolved",
        ConstraintState::Missing => "missing",
    }
}

/// The patterns of a sketch as `sketch.inspect` lists them.
pub(crate) fn patterns_json(sketch: &Sketch, solved: Option<&SketchResult>) -> Vec<Value> {
    sketch
        .patterns
        .iter()
        .map(|pattern| {
            let mut entry = json!({
                "id": pattern.id.to_string(),
                "seed": ids(&pattern.seed),
                "count": pattern.count.source(),
                "layout": pattern.layout.type_name(),
            });
            let Some(map) = entry.as_object_mut() else {
                return entry;
            };
            // A tree of patterns: the patterns in its seed, the one it is
            // in, and the levels of the names of its curves.
            let levels = sketch.pattern_levels(pattern.id);
            if !pattern.patterns.is_empty() {
                let inner: Vec<String> = pattern.patterns.iter().map(ToString::to_string).collect();
                map.insert("patterns".into(), inner.into());
            }
            if let Some(parent) = sketch.parent_pattern(pattern.id) {
                map.insert("in".into(), parent.to_string().into());
            }
            if levels.len() > 1 || !pattern.patterns.is_empty() {
                let shown: Vec<String> = levels.iter().map(ToString::to_string).collect();
                map.insert("levels".into(), shown.into());
            }
            if !pattern.skip.is_empty() {
                let shown: Vec<String> = pattern.skip.iter().map(ToString::to_string).collect();
                map.insert("skip".into(), shown.into());
            }
            let pair = |[x, y]: &[Expr; 2]| json!([x.source(), y.source()]);
            match &pattern.layout {
                SketchPatternLayout::Linear { step, second } => {
                    map.insert("step".into(), pair(step));
                    if let Some(second) = second {
                        map.insert("step2".into(), pair(&second.step));
                        map.insert("count2".into(), second.count.source().into());
                    }
                }
                SketchPatternLayout::Chain { from, to } => {
                    map.insert("chain".into(), json!([from.to_string(), to.to_string()]));
                }
                SketchPatternLayout::Circular {
                    center,
                    angle,
                    chain,
                } => {
                    map.insert("angle".into(), angle.source().into());
                    map.insert(
                        "center".into(),
                        match center {
                            CenterRef::Point { point } => point.to_string().into(),
                            CenterRef::At { at } => pair(at),
                        },
                    );
                    if let Some([from, to]) = chain {
                        map.insert("chain".into(), json!([from.to_string(), to.to_string()]));
                    }
                }
            }
            // The curves it makes and how a name calls them.
            let curves: Vec<&craft3d_doc::Entity> = pattern
                .seed
                .iter()
                .filter_map(|id| sketch.entity(*id).ok())
                .filter(|e| !matches!(e.geometry, Geometry::Point { .. } | Geometry::Image { .. }))
                .collect();
            if let Some(result) =
                solved.and_then(|s| s.patterns.iter().find(|r| r.id == pattern.id))
            {
                let [a, b] = result.counts;
                map.insert("instances".into(), result.instances().into());
                map.insert("counts".into(), json!([a, b]));
                if !result.inactive.is_empty() {
                    let shown: Vec<String> =
                        result.inactive.iter().map(ToString::to_string).collect();
                    map.insert("inactive".into(), shown.into());
                }
                // Every level around a seed curve of this pattern: the
                // instances of each, and of the whole tree.
                let around: Vec<&craft3d_regen::PatternResult> = levels
                    .iter()
                    .filter_map(|id| solved.and_then(|s| s.patterns.iter().find(|r| r.id == *id)))
                    .collect();
                if around.len() > 1 {
                    let total = around
                        .iter()
                        .try_fold(1usize, |n, r| n.checked_mul(r.instances()));
                    map.insert("allInstances".into(), total.map_or(Value::Null, Value::from));
                }
                if let Placed::Turn { .. } = result.placed {
                    map.insert("closes".into(), result.closes.into());
                }
                if let Some(chain) = result.chain {
                    map.insert("chainState".into(), state_name(chain).into());
                }
                // The last instance on every level, the innermost first.
                let last: Vec<CurveIndex> = around
                    .iter()
                    .map(|r| {
                        let (i, j) = r
                            .resolve(craft3d_doc::InstanceIndex::Last)
                            .unwrap_or((0, 0));
                        if r.grid {
                            CurveIndex::Grid(i, j)
                        } else {
                            CurveIndex::At(i)
                        }
                    })
                    .collect();
                let names: Vec<Value> = curves
                    .iter()
                    .map(|e| {
                        json!({
                            "seed": e.id.to_string(),
                            // One level: the index as it is (`e7[0]`
                            // at one instance), as before nesting.
                            "last": match last.as_slice() {
                                [index] => craft3d_doc::CurveRef { entity: e.id.0, index: *index }.to_string(),
                                _ => CurvePath::of_levels(e.id.0, &last).to_string(),
                            },
                            "run": CurvePath::of_levels(e.id.0, &[CurveIndex::All]).to_string(),
                        })
                    })
                    .collect();
                map.insert("curves".into(), names.into());
            }
            entry
        })
        .collect()
}
