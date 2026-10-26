//! Hydrostatics of any closed bodies (`measure.hydrostatics`, and the
//! assertion measure `hydrostatic`): the displaced volume below a
//! waterplane, the centre of buoyancy, the waterplane's area and second
//! moments, the waterline for a weight, BM, KB, KM, KG, GM and the
//! righting arm GZ at a heel. The geometry is `craft3d_kernel::hydro`
//! (exact on the B-rep); the design note is docs/design/hydrostatics.md.

use crate::commands::{Run, length_unit, spec, vec3};
use crate::params::Params;
use crate::verify_cmds::{
    Ctx, Dim, Outcome, all_bodies, direction, nonempty, subject, whole_bodies,
};
use crate::{CommandError, CommandSpec, Session};
use craft3d_kernel::hydro::{Floating, Immersion, Water, float, immersion};
use craft3d_math::Vec3;
use craft3d_topo::Body;
use serde_json::{Map, Value, json};

/// The parameters both take.
pub(crate) const KEYS: [&str; 11] = [
    "of",
    "up",
    "along",
    "heel",
    "level",
    "mass",
    "density",
    "centerOfGravity",
    "kg",
    "keel",
    "loads",
];

/// Most bodies floated together, and most loads.
const MAX_BODIES: usize = 200;
const MAX_LOADS: usize = 1_000;

/// Sea water, kg/m³, when no density is given.
pub(crate) const SEA_WATER: f64 = 1025.0;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("measure.hydrostatics", "Hydrostatics", r#"{"of": subject = all bodies, "up": direction = "z", "along": direction = "x", "heel": angle expr = 0, "level": length expr = solved from the weight, "mass": kg expr = the bodies' mass from their materials, "density": kg/m³ or "1.000 g/cm3" or expr = 1025, "centerOfGravity": [expr, expr, expr] = the centre of mass of the bodies and loads, "kg": length expr, "keel": length expr = 0, "loads": [{mass: kg expr, at: [expr, expr, expr]}] = none}"#,
        "Hydrostatics of any closed bodies floating in water of a density (default sea water, 1025 kg/m³): the water's plane is square to up (default z), turned by heel (default 0; a plain number is degrees) about along (default x, the length; square to up), and lies at level (a height along the water's normal from the origin) when that is given, or else where the bodies displace their weight: mass (kg, default the bodies' mass from their materials) plus the loads, found by the kernel by Newton's method on the waterplane area within a bracket (at most 64 steps; not converging is an error, as are bodies that sink and a weight of zero). Everything is exact on the B-rep: the part below the water is the boolean intersection with a half-space and its volume and centroid are integrated, the waterplane's area and second moments are Green's theorem along the curves in which the plane cuts the bodies. Returns volume (∇, the document's unit cubed), displacement (ρ∇, kg), mass, level, draft (from the keel: K is the point keel along up from the origin, default the origin), centerOfBuoyancy, waterplane (area, centroid, iTransverse about the axis along `along` through its centroid, iLongitudinal about the one across, iProduct), wettedArea, KB, BM = iTransverse/∇, BMl, KM, KMl, and with a centre of gravity (centerOfGravity, else the centre of mass of the bodies with materials and the loads; kg sets its height above K, on the line along up through the origin when no centre is known) KG, GM = KM − KG, GMl and GZ, the righting arm (B − G) · (water's up × along), positive when it rights. Heights are along the water's normal. Bodies that overlap are counted once each. Read-only.", doc: true, names: ["of"], measure_hydrostatics),
];

/// What the assertion measure `hydrostatic` can assert: the key of the
/// result, and its dimension.
pub(crate) const QUANTITIES: [(&str, Dim); 22] = [
    ("volume", Dim::Length(3)),
    ("displacement", Dim::Mass),
    ("mass", Dim::Mass),
    ("level", Dim::Length(1)),
    ("draft", Dim::Length(1)),
    ("KB", Dim::Length(1)),
    ("BM", Dim::Length(1)),
    ("BMl", Dim::Length(1)),
    ("KM", Dim::Length(1)),
    ("KMl", Dim::Length(1)),
    ("KG", Dim::Length(1)),
    ("GM", Dim::Length(1)),
    ("GMl", Dim::Length(1)),
    ("GZ", Dim::Length(1)),
    ("waterplaneArea", Dim::Length(2)),
    ("iTransverse", Dim::Length(4)),
    ("iLongitudinal", Dim::Length(4)),
    ("wettedArea", Dim::Length(2)),
    ("buoyancyX", Dim::Length(1)),
    ("buoyancyY", Dim::Length(1)),
    ("buoyancyZ", Dim::Length(1)),
    ("steps", Dim::Count),
];

/// The dimension of quantity `name` of a `hydrostatic` assertion
/// (lengths for a name it does not know, which the measure refuses).
pub(crate) fn dim_of_quantity(name: Option<&str>) -> Dim {
    QUANTITIES
        .iter()
        .find(|(q, _)| Some(*q) == name)
        .map_or(Dim::Length(1), |(_, d)| *d)
}

fn measure_hydrostatics(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "measure.hydrostatics";
    Params::new(ID, params, &KEYS)?;
    let map = match params {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let ctx = Ctx::new(doc, &regen).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let out = hydrostatics(&ctx, &map).map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    Ok(Value::Object(out))
}

/// The assertion measure: one quantity of [`hydrostatics`].
pub(crate) fn measure(ctx: &Ctx<'_>, p: &Map<String, Value>) -> Result<Outcome, String> {
    let mut rest = p.clone();
    let quantity = match rest.remove("quantity") {
        Some(Value::String(q)) => q,
        _ => {
            return Err(format!(
                "`quantity` is required: one of {}",
                names().join(", ")
            ));
        }
    };
    let Some((name, dim)) = QUANTITIES.iter().find(|(q, _)| *q == quantity) else {
        return Err(format!(
            "unknown quantity `{}`; one of {}",
            quantity.chars().take(40).collect::<String>(),
            names().join(", ")
        ));
    };
    let detail = hydrostatics(ctx, &rest)?;
    let value = detail.get(*name).and_then(Value::as_f64).ok_or_else(|| {
        let why = detail
            .get("why")
            .and_then(Value::as_str)
            .unwrap_or("it is not defined here");
        format!("{name} has no value: {why}")
    })?;
    let mut out = Outcome::number(*dim, value);
    out.detail = detail;
    Ok(out)
}

fn names() -> Vec<&'static str> {
    QUANTITIES.iter().map(|(q, _)| *q).collect()
}

/// An angle: a plain number in degrees or an expression with an angle
/// unit (or none, then degrees).
fn angle(ctx: &Ctx<'_>, value: &Value, key: &str) -> Result<f64, String> {
    let radians = match value {
        Value::Number(n) => n.as_f64().map(f64::to_radians),
        Value::String(source) => {
            let q = evaluate(ctx, source, key)?;
            if q.dimension == craft3d_expr::Dimension::ANGLE {
                Some(q.value)
            } else if q.dimension.is_dimensionless() {
                Some(q.value.to_radians())
            } else {
                return Err(format!(
                    "`{key}` is {}, but an angle is needed",
                    q.dimension.described()
                ));
            }
        }
        _ => None,
    };
    radians
        .filter(|r| r.is_finite())
        .ok_or_else(|| format!("`{key}` must be an angle: a number of degrees or an expression"))
}

fn evaluate(ctx: &Ctx<'_>, source: &str, key: &str) -> Result<craft3d_expr::Quantity, String> {
    let evaluation = crate::appearance::evaluation(ctx.doc);
    match &evaluation {
        Some(evaluation) => evaluation.evaluate(source),
        None => craft3d_expr::evaluate(source, |_| None),
    }
    .map_err(|e| format!("`{key}`: {e}"))
}

/// kg per unit of mass that can follow the number.
const MASS_UNITS: [(&str, f64); 4] = [("kg", 1.0), ("g", 1e-3), ("t", 1e3), ("lb", 0.453_592_37)];

/// A plain number in `units` (the first is what a bare number is in), a
/// number or an expression of the variables followed by one of them.
fn per_unit(ctx: &Ctx<'_>, value: &Value, key: &str, units: &[(&str, f64)]) -> Result<f64, String> {
    let wrong = || {
        let listed: Vec<&str> = units.iter().map(|(u, _)| *u).collect();
        format!(
            "`{key}` must be a number or an expression of the variables that gives a plain number, with or without a unit ({})",
            listed.join(", ")
        )
    };
    let number = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => {
            let text = text.trim();
            // The longest unit that ends the text after a space or a
            // digit, so that `t` does not take the end of a name.
            let unit = units
                .iter()
                .filter(|(u, _)| {
                    text.strip_suffix(u).is_some_and(|before| {
                        before
                            .chars()
                            .last()
                            .is_some_and(|c| c == ' ' || c.is_ascii_digit() || c == ')')
                    })
                })
                .max_by_key(|(u, _)| u.len());
            let (source, per) = match unit {
                Some((u, per)) => (text.get(..text.len() - u.len()).unwrap_or(""), *per),
                None => (text, 1.0),
            };
            let source = source.trim();
            if source.is_empty() {
                return Err(wrong());
            }
            let q = evaluate(ctx, source, key)?;
            if !q.dimension.is_dimensionless() {
                return Err(format!(
                    "`{key}` is {}; {}",
                    q.dimension.described(),
                    wrong()
                ));
            }
            Some(q.value * per)
        }
        _ => None,
    };
    number.filter(|n| n.is_finite()).ok_or_else(wrong)
}

/// kg/m³ per unit of density that can follow the number.
const DENSITY_UNITS: [(&str, f64); 8] = [
    ("kg/m3", 1.0),
    ("kg/m^3", 1.0),
    ("g/cm3", 1000.0),
    ("g/cm^3", 1000.0),
    ("g/mm3", 1e6),
    ("g/mm^3", 1e6),
    ("lb/in3", 27_679.904_710_203_12),
    ("lb/ft3", 16.018_463_373_960_14),
];

fn given<'v>(p: &'v Map<String, Value>, key: &str) -> Option<&'v Value> {
    p.get(key).filter(|v| !v.is_null())
}

/// The weight: its mass in kg and, when known, its centre.
struct Weight {
    mass: f64,
    center: Option<Vec3>,
}

/// The mass and centre of mass of `bodies` from their materials; an error
/// naming a body without one.
fn of_materials(
    ctx: &Ctx<'_>,
    bodies: &[(craft3d_doc::FeatureId, std::sync::Arc<Body>)],
) -> Result<Weight, String> {
    let m = crate::appearance_cmds::metres_per_unit(ctx.doc.units.length);
    let (mut mass, mut moment) = (0.0, Vec3::ZERO);
    for (id, body) in bodies {
        let name = ctx
            .doc
            .feature(*id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let Some((_, material)) = crate::appearance::body_material(ctx.doc, ctx.regen, *id) else {
            return Err(format!(
                "`{name}` has no material, so no weight: give it one with body.material, or give mass"
            ));
        };
        let props =
            crate::mass::mass_of(body).map_err(|e| format!("`{name}` cannot be measured: {e}"))?;
        let centroid = props
            .centroid
            .ok_or_else(|| format!("`{name}` has no volume, so no centre of mass"))?;
        let density = crate::appearance::density(ctx.doc, &material)
            .map_err(|why| format!("`{name}`: {why}"))?;
        let body_mass = props.volume * density * m * m * m;
        if !body_mass.is_finite() {
            return Err(format!("the mass of `{name}` is not a finite number"));
        }
        mass += body_mass;
        moment += centroid * body_mass;
    }
    let center = (mass > 0.0).then(|| moment * (1.0 / mass));
    Ok(Weight { mass, center })
}

/// The weight of the bodies (or `mass`, with `centerOfGravity`) and the
/// loads.
fn weight(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
    bodies: &[(craft3d_doc::FeatureId, std::sync::Arc<Body>)],
) -> Result<Weight, String> {
    let center_given = given(p, "centerOfGravity")
        .map(|v| ctx.point(Some(v), "centerOfGravity"))
        .transpose()?;
    let mut own = match given(p, "mass") {
        Some(v) => {
            let mass = per_unit(ctx, v, "mass", &MASS_UNITS)?;
            if mass < 0.0 {
                return Err(format!("`mass` is {mass} kg, less than nothing"));
            }
            // The centre of the materials when every body has one.
            let center = of_materials(ctx, bodies).ok().and_then(|w| w.center);
            Weight { mass, center }
        }
        None => of_materials(ctx, bodies)?,
    };
    if center_given.is_some() {
        own.center = center_given;
    }
    let mut mass = own.mass;
    let mut moment = own.center.map(|c| c * own.mass);
    match given(p, "loads") {
        None => {}
        Some(Value::Array(loads)) => {
            if loads.len() > MAX_LOADS {
                return Err(format!("at most {MAX_LOADS} loads"));
            }
            for (i, load) in loads.iter().enumerate() {
                let Value::Object(load) = load else {
                    return Err("each load is {mass, at}".into());
                };
                if let Some(key) = load.keys().find(|k| *k != "mass" && *k != "at") {
                    let key: String = key.chars().take(40).collect();
                    return Err(format!("a load has mass and at, not `{key}`"));
                }
                let key = format!("loads[{i}]");
                let m = per_unit(
                    ctx,
                    load.get("mass")
                        .ok_or_else(|| format!("`{key}` needs mass"))?,
                    &key,
                    &MASS_UNITS,
                )?;
                if m < 0.0 {
                    return Err(format!("`{key}`: a mass of {m} kg is less than nothing"));
                }
                let at = ctx.point(load.get("at"), &key)?;
                mass += m;
                moment = moment.map(|sum| sum + at * m);
            }
        }
        Some(_) => return Err("`loads` must be a list of {mass, at}".into()),
    }
    if !mass.is_finite() {
        return Err("the weight is not a finite number".into());
    }
    let center = moment.filter(|_| mass > 0.0).map(|sum| sum * (1.0 / mass));
    Ok(Weight { mass, center })
}

/// Every quantity as the command returns it (flat keys for each of
/// [`QUANTITIES`], `null` where it is not defined, with `why`).
pub(crate) fn hydrostatics(
    ctx: &Ctx<'_>,
    p: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    crate::verify_cmds::only(p, &KEYS)?;
    let found = match given(p, "of") {
        None => all_bodies(ctx),
        Some(v) => nonempty(subject(ctx, v, &["body"])?, "of")?,
    };
    let bodies = whole_bodies(ctx, &found, "hydrostatics")?;
    if bodies.is_empty() {
        return Err("there are no bodies to float".into());
    }
    if bodies.len() > MAX_BODIES {
        return Err(format!(
            "{} bodies; hydrostatics floats at most {MAX_BODIES}",
            bodies.len()
        ));
    }
    let up = match given(p, "up") {
        None => Vec3::new(0.0, 0.0, 1.0),
        Some(v) => direction(Some(v), "up")?,
    };
    let along = match given(p, "along") {
        None => Vec3::new(1.0, 0.0, 0.0),
        Some(v) => direction(Some(v), "along")?,
    };
    let upright = Water::new(up, along).map_err(|e| e.to_string())?;
    let heel = given(p, "heel")
        .map(|v| angle(ctx, v, "heel"))
        .transpose()?
        .unwrap_or(0.0);
    let water = upright.heeled(heel).map_err(|e| e.to_string())?;
    let keel = given(p, "keel")
        .map(|v| ctx.quantity(v, "keel", 1))
        .transpose()?
        .unwrap_or(0.0);
    let k_point = upright.up * keel;
    let density = match given(p, "density") {
        None => SEA_WATER,
        Some(v) => {
            let d = per_unit(ctx, v, "density", &DENSITY_UNITS)?;
            if d <= 0.0 {
                return Err(format!(
                    "`density` is {d} kg/m³; water is denser than nothing"
                ));
            }
            d
        }
    };
    let m = crate::appearance_cmds::metres_per_unit(ctx.doc.units.length);
    let cubic = m * m * m;
    let plain: Vec<&Body> = bodies.iter().map(|(_, b)| b.as_ref()).collect();
    let level_given = given(p, "level")
        .map(|v| ctx.quantity(v, "level", 1))
        .transpose()?;
    // The weight is needed to find the waterline, and for G; with a level
    // given, a weight that cannot be had leaves G unknown.
    let weight = match level_given {
        None => Some(weight(ctx, p, &bodies)?),
        Some(_) => {
            let asked = ["mass", "centerOfGravity", "loads"]
                .iter()
                .any(|k| given(p, k).is_some());
            if asked {
                Some(weight(ctx, p, &bodies)?)
            } else {
                weight(ctx, p, &bodies).ok()
            }
        }
    };
    let (found, steps, residual): (Immersion, Option<usize>, Option<f64>) = match level_given {
        Some(level) => (
            immersion(&plain, &water, level, &ctx.tolerance).map_err(|e| e.to_string())?,
            None,
            None,
        ),
        None => {
            let mass = weight.as_ref().map_or(0.0, |w| w.mass);
            let volume = mass / (density * cubic);
            let Floating {
                immersion,
                steps,
                residual,
            } = float(&plain, &water, volume, &ctx.tolerance).map_err(|e| e.to_string())?;
            (immersion, Some(steps), Some(residual))
        }
    };
    // G: the centre of the weight, its height set by kg.
    let kg_given = given(p, "kg")
        .map(|v| ctx.quantity(v, "kg", 1))
        .transpose()?;
    let center = weight.as_ref().and_then(|w| w.center);
    let g = match (center, kg_given) {
        (Some(c), Some(kg)) => Some(c + water.up * (kg - (c - k_point).dot(water.up))),
        (None, Some(kg)) => Some(k_point + water.up * kg),
        (c, None) => c,
    };

    let mut out = Map::new();
    let unit = length_unit(ctx.doc);
    let num = |v: Option<f64>| v.filter(|x| x.is_finite()).map_or(Value::Null, Value::from);
    let volume = found.volume;
    out.insert("volume".into(), num(Some(volume)));
    out.insert("displacement".into(), num(Some(volume * density * cubic)));
    out.insert("density".into(), density.into());
    out.insert("mass".into(), num(weight.as_ref().map(|w| w.mass)));
    out.insert("level".into(), num(Some(found.level)));
    out.insert(
        "draft".into(),
        num(Some(found.level - k_point.dot(water.up))),
    );
    out.insert("heel".into(), heel.to_degrees().into());
    if let Some(steps) = steps {
        out.insert("steps".into(), steps.into());
    }
    if let Some(residual) = residual {
        out.insert("residual".into(), num(Some(residual)));
    }
    let wp = found.waterplane;
    out.insert(
        "waterplane".into(),
        json!({
            "area": wp.area,
            "centroid": wp.centroid.map(vec3),
            "iTransverse": wp.i_along,
            "iLongitudinal": wp.i_across,
            "iProduct": wp.product,
        }),
    );
    out.insert("waterplaneArea".into(), num(Some(wp.area)));
    out.insert("iTransverse".into(), num(Some(wp.i_along)));
    out.insert("iLongitudinal".into(), num(Some(wp.i_across)));
    out.insert("wettedArea".into(), num(Some(found.wetted)));
    let b = found.buoyancy;
    out.insert("centerOfBuoyancy".into(), b.map_or(Value::Null, vec3));
    out.insert("buoyancyX".into(), num(b.map(|b| b.x)));
    out.insert("buoyancyY".into(), num(b.map(|b| b.y)));
    out.insert("buoyancyZ".into(), num(b.map(|b| b.z)));
    let floating = volume > 0.0;
    let kb = b.map(|b| (b - k_point).dot(water.up));
    let bm = floating.then(|| wp.i_along / volume);
    let bml = floating.then(|| wp.i_across / volume);
    let km = kb.zip(bm).map(|(a, b)| a + b);
    let kml = kb.zip(bml).map(|(a, b)| a + b);
    let kg = g.map(|g| (g - k_point).dot(water.up));
    out.insert("KB".into(), num(kb));
    out.insert("BM".into(), num(bm));
    out.insert("BMl".into(), num(bml));
    out.insert("KM".into(), num(km));
    out.insert("KMl".into(), num(kml));
    out.insert("KG".into(), num(kg));
    out.insert("GM".into(), num(km.zip(kg).map(|(m, g)| m - g)));
    out.insert("GMl".into(), num(kml.zip(kg).map(|(m, g)| m - g)));
    let righting = water.up.cross(water.along);
    out.insert(
        "GZ".into(),
        num(b.zip(g).map(|(b, g)| (b - g).dot(righting))),
    );
    out.insert("centerOfGravity".into(), g.map_or(Value::Null, vec3));
    let why = if !floating {
        Some("nothing lies below the water")
    } else if g.is_none() {
        Some("no centre of gravity: give the bodies materials, or give centerOfGravity or kg")
    } else {
        None
    };
    if let Some(why) = why {
        out.insert("why".into(), why.into());
    }
    let listed: Vec<Value> = bodies
        .iter()
        .map(|(id, _)| {
            ctx.doc
                .feature(*id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
                .into()
        })
        .collect();
    out.insert("bodies".into(), listed.into());
    out.insert("lengthUnit".into(), unit.into());
    out.insert("massUnit".into(), "kg".into());
    Ok(out)
}
