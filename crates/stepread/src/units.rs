//! The units of a representation context (ISO 10303-41, the measure
//! schema: `si_unit`, `conversion_based_unit`, `length_unit`,
//! `plane_angle_unit`, `uncertainty_measure_with_unit`).

use crate::{ImportError, Parsed, err, real};
use craft3d_step::Param;

/// How deep a conversion-based unit may be defined through others.
const MAX_UNIT_DEPTH: usize = 4;

pub(crate) struct Units {
    pub metres_per_length: f64,
    pub length_name: String,
    pub radians_per_angle: f64,
    pub uncertainty_metres: Option<f64>,
}

fn prefix(name: &str) -> Option<f64> {
    Some(match name {
        "EXA" => 1e18,
        "PETA" => 1e15,
        "TERA" => 1e12,
        "GIGA" => 1e9,
        "MEGA" => 1e6,
        "KILO" => 1e3,
        "HECTO" => 1e2,
        "DECA" => 1e1,
        "DECI" => 1e-1,
        "CENTI" => 1e-2,
        "MILLI" => 1e-3,
        "MICRO" => 1e-6,
        "NANO" => 1e-9,
        "PICO" => 1e-12,
        "FEMTO" => 1e-15,
        "ATTO" => 1e-18,
        _ => return None,
    })
}

/// What one of unit `id` is in SI (metres, radians), and its name.
/// `quantity` is `LENGTH_UNIT` or `PLANE_ANGLE_UNIT`, `si` the SI unit
/// expected (`METRE`, `RADIAN`), `measure` the measure-with-unit entity
/// of a conversion (`LENGTH_MEASURE_WITH_UNIT`).
fn unit(
    parsed: &Parsed,
    id: u64,
    si: &str,
    measure: &str,
    depth: usize,
) -> Result<(f64, String), ImportError> {
    if depth > MAX_UNIT_DEPTH {
        return err(format!(
            "{}: a unit defined through more than {MAX_UNIT_DEPTH} others",
            parsed.describe(id)
        ));
    }
    let instance = parsed.instance(id)?;
    if let Some(record) = instance.entity.record("SI_UNIT") {
        let name = record.params.get(1).and_then(Param::as_enum).unwrap_or("");
        if name != si {
            return err(format!(
                "{}: the SI unit {name} where {si} is needed",
                parsed.describe(id)
            ));
        }
        return match record.params.first() {
            Some(Param::Enum(p)) => match prefix(p) {
                Some(factor) => Ok((factor, format!("{p} {si}"))),
                None => err(format!("{}: unknown SI prefix {p}", parsed.describe(id))),
            },
            _ => Ok((1.0, si.to_string())),
        };
    }
    if let Some(record) = instance.entity.record("CONVERSION_BASED_UNIT") {
        let name = record
            .params
            .first()
            .and_then(Param::as_str)
            .unwrap_or("")
            .to_string();
        let Some(of) = record.params.get(1).and_then(Param::as_ref) else {
            return err(format!(
                "{}: a conversion without its measure",
                parsed.describe(id)
            ));
        };
        let with = parsed.record(of, measure)?;
        let value = with.params.first().and_then(real);
        let base = with.params.get(1).and_then(Param::as_ref);
        let (Some(value), Some(base)) = (value, base) else {
            return err(format!(
                "{}: not a measure with a unit",
                parsed.describe(of)
            ));
        };
        let (factor, _) = unit(parsed, base, si, measure, depth + 1)?;
        let total = value * factor;
        if !(total.is_finite() && total > 0.0) {
            return err(format!(
                "{}: the unit {name} is {total} {si}",
                parsed.describe(id)
            ));
        }
        return Ok((total, name));
    }
    err(format!(
        "{}: a unit that is neither an SI unit nor converted from one",
        parsed.describe(id)
    ))
}

/// The units and uncertainty of context `context`.
pub(crate) fn read(parsed: &Parsed, context: u64) -> Result<Units, ImportError> {
    let instance = parsed.instance(context)?;
    let Some(assigned) = instance.entity.record("GLOBAL_UNIT_ASSIGNED_CONTEXT") else {
        return err(format!(
            "{}: the representation context assigns no units",
            parsed.describe(context)
        ));
    };
    let list = assigned
        .params
        .first()
        .and_then(Param::as_list)
        .unwrap_or(&[]);
    let mut length = None;
    let mut angle = None;
    for id in list.iter().filter_map(Param::as_ref) {
        let unit_instance = parsed.instance(id)?;
        if length.is_none() && unit_instance.entity.is("LENGTH_UNIT") {
            length = Some(unit(parsed, id, "METRE", "LENGTH_MEASURE_WITH_UNIT", 0)?);
        } else if angle.is_none() && unit_instance.entity.is("PLANE_ANGLE_UNIT") {
            angle = Some(unit(
                parsed,
                id,
                "RADIAN",
                "PLANE_ANGLE_MEASURE_WITH_UNIT",
                0,
            )?);
        }
    }
    let Some((metres_per_length, length_name)) = length else {
        return err(format!(
            "{}: the representation context has no length unit",
            parsed.describe(context)
        ));
    };
    // Angles default to radians: only the semi-angle of a cone is an
    // angle in a B-rep, and a file without an angle unit has radians if
    // it has anything (ISO 10303-41's SI default).
    let radians_per_angle = angle.map_or(1.0, |(factor, _)| factor);
    let mut uncertainty_metres: Option<f64> = None;
    if let Some(record) = instance
        .entity
        .record("GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT")
    {
        for id in record
            .params
            .first()
            .and_then(Param::as_list)
            .unwrap_or(&[])
            .iter()
            .filter_map(Param::as_ref)
        {
            let Ok(measure) = parsed.record(id, "UNCERTAINTY_MEASURE_WITH_UNIT") else {
                continue;
            };
            let value = measure.params.first().and_then(real);
            let of = measure.params.get(1).and_then(Param::as_ref);
            let (Some(value), Some(of)) = (value, of) else {
                continue;
            };
            // Only length uncertainties; an angle's is not a distance.
            if !parsed.instance(of)?.entity.is("LENGTH_UNIT") {
                continue;
            }
            let (factor, _) = unit(parsed, of, "METRE", "LENGTH_MEASURE_WITH_UNIT", 0)?;
            let metres = value * factor;
            if metres.is_finite() && metres > 0.0 {
                uncertainty_metres = Some(uncertainty_metres.map_or(metres, |u| u.min(metres)));
            }
        }
    }
    Ok(Units {
        metres_per_length,
        length_name,
        radians_per_angle,
        uncertainty_metres,
    })
}
