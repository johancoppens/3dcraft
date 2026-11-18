//! One way to write a number (the UX round of 2026-11-18, V5): every
//! length, angle, area and volume the section, the measuring and the
//! notes show (the plate's distance, the dimension's label, the card's
//! number and rows, X · Y · Z, a box, the hint of a clamp, a wall's
//! thickness) is written here, so the view and the card say one text for
//! one value.
//!
//! - A length: at most [`LENGTH_PLACES_MM`] decimals in millimetres (as
//!   many more or fewer in another unit as it is larger or smaller: 4 in
//!   inches, 6 in metres), never more than the document's tolerance gives,
//!   no trailing zeros; below a hundredth of a millimetre two significant
//!   digits (`0.0057 mm`).
//! - An angle: at most [`ANGLE_PLACES`] decimals of a degree.
//! - An area or a volume: [`SIGNIFICANT`] significant digits, no trailing
//!   zeros (`1450 mm²`, `0.0123 mm²`, `39.9 cm³`).
//! - A range: each end by its rule (`0.045 – 0.1 mm`).
//!
//! What is shortened keeps its whole value ([`Shown::whole`]: to the
//! document's tolerance) for the hover; a click copies what is shown.
//! *The developer may overrule* the places: they are the constants below.

use craft3d_math::Portable;

/// The decimals of a length in millimetres: a person reads thousandths;
/// the exact value is in the hover and in `select.get`.
pub const LENGTH_PLACES_MM: usize = 3;
/// The decimals of an angle in degrees.
pub const ANGLE_PLACES: usize = 2;
/// The significant digits of an area or a volume.
pub const SIGNIFICANT: usize = 4;
/// Significant digits of a length too small for its places.
const SMALL_SIGNIFICANT: i32 = 2;
/// Most decimals anything is written with.
const MOST_PLACES: usize = 12;

/// A number as shown, and as a whole where it was shortened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    /// What the window shows and a click copies: `25.981 mm`.
    pub text: String,
    /// The value to the document's tolerance: `25.980762 mm`; the same as
    /// `text` where nothing was shortened.
    pub whole: String,
}

impl Shown {
    /// Whether the text shows less than the whole value: then the hover
    /// gives it.
    pub fn shortened(&self) -> bool {
        self.text != self.whole
    }
}

/// How many millimetres one `unit` is, for the units a document's
/// lengths come in; `None` for another.
fn unit_in_mm(unit: &str) -> Option<f64> {
    match unit {
        "um" | "µm" => Some(1e-3),
        "mm" => Some(1.0),
        "cm" => Some(10.0),
        "m" => Some(1000.0),
        "in" | "inch" => Some(25.4),
        "ft" => Some(304.8),
        _ => None,
    }
}

/// The places a length in `unit` is written with at most: as many as a
/// thousandth of a millimetre takes in it (`mm` 3, `in` 4, `m` 6, `um`
/// 0); [`LENGTH_PLACES_MM`] for a unit not known.
pub fn length_places(unit: &str) -> usize {
    let base = LENGTH_PLACES_MM as f64;
    let places = unit_in_mm(unit).map_or(base, |mm| base + mm.plog10());
    if places.is_finite() {
        places.round().clamp(0.0, MOST_PLACES as f64) as usize
    } else {
        LENGTH_PLACES_MM
    }
}

/// The places `tolerance` holds (1e-6: 6); `None` for no tolerance.
fn tolerance_places(tolerance: f64) -> Option<usize> {
    (tolerance.is_finite() && tolerance > 0.0)
        .then(|| (-tolerance.plog10()).ceil().clamp(0.0, MOST_PLACES as f64) as usize)
}

/// `v` to `places` decimals without trailing zeros, never `-0`.
fn trimmed(v: f64, places: usize) -> String {
    if !v.is_finite() {
        return "?".to_string();
    }
    let places = places.min(MOST_PLACES);
    let text = format!("{v:.places$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    match text.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_string(),
        _ => text,
    }
}

/// The decimals that give `v` `digits` significant digits (none for a
/// number with that many before the point).
fn significant_places(v: f64, digits: i32) -> usize {
    if v == 0.0 || !v.is_finite() {
        return 0;
    }
    let magnitude = v.abs().plog10().floor();
    if !magnitude.is_finite() {
        return 0;
    }
    (f64::from(digits) - 1.0 - magnitude).clamp(0.0, MOST_PLACES as f64) as usize
}

/// `n` with `unit` after it, or alone without one.
fn with_unit(n: String, unit: &str) -> String {
    if unit.is_empty() {
        n
    } else {
        format!("{n} {unit}")
    }
}

/// A length's number alone, in `unit`, at the document's `tolerance`.
pub fn length_number(v: f64, tolerance: f64, unit: &str) -> String {
    let most = length_places(unit);
    let held = tolerance_places(tolerance).unwrap_or(MOST_PLACES);
    // Below the last place shown: two significant digits.
    let small = 10f64.ppowi(1 - most as i32);
    let places = if v != 0.0 && v.abs() < small {
        significant_places(v, SMALL_SIGNIFICANT).max(most)
    } else {
        most
    };
    trimmed(v, places.min(held))
}

/// A length's number to the document's `tolerance`: the whole value.
pub fn length_whole(v: f64, tolerance: f64) -> String {
    trimmed(v, tolerance_places(tolerance).unwrap_or(6))
}

/// A length in `unit` (`30 mm`, `25.981 mm`), and its whole value.
pub fn length(v: f64, tolerance: f64, unit: &str) -> Shown {
    Shown {
        text: with_unit(length_number(v, tolerance, unit), unit),
        whole: with_unit(length_whole(v, tolerance), unit),
    }
}

/// A length's text alone: [`length`]'s `text`.
pub fn length_text(v: f64, tolerance: f64, unit: &str) -> String {
    length(v, tolerance, unit).text
}

/// A range of lengths, each end by the rule: `0.045 – 0.1 mm`.
pub fn length_range(lo: f64, hi: f64, tolerance: f64, unit: &str) -> Shown {
    Shown {
        text: with_unit(
            format!(
                "{} – {}",
                length_number(lo, tolerance, unit),
                length_number(hi, tolerance, unit)
            ),
            unit,
        ),
        whole: with_unit(
            format!(
                "{} – {}",
                length_whole(lo, tolerance),
                length_whole(hi, tolerance)
            ),
            unit,
        ),
    }
}

/// Three lengths along X, Y and Z: `X 15 · Y 25.981 · Z 0 mm`.
pub fn length_parts(parts: [f64; 3], tolerance: f64, unit: &str) -> Shown {
    let [x, y, z] = parts.map(|c| length_number(c, tolerance, unit));
    let [wx, wy, wz] = parts.map(|c| length_whole(c, tolerance));
    Shown {
        text: with_unit(format!("X {x} · Y {y} · Z {z}"), unit),
        whole: with_unit(format!("X {wx} · Y {wy} · Z {wz}"), unit),
    }
}

/// An angle in degrees at the document's angular `tolerance` (in
/// degrees; 0 for none): `135°`, `22.5°`.
pub fn angle(degrees: f64, tolerance: f64) -> Shown {
    let held = tolerance_places(tolerance).unwrap_or(MOST_PLACES);
    Shown {
        text: format!("{}°", trimmed(degrees, ANGLE_PLACES.min(held))),
        whole: format!(
            "{}°",
            trimmed(degrees, tolerance_places(tolerance).unwrap_or(6))
        ),
    }
}

/// `v` to [`SIGNIFICANT`] digits, without trailing zeros, thousands
/// apart by a thin space from five digits on (`12 340`).
fn significant(v: f64) -> String {
    if !v.is_finite() {
        return "?".to_string();
    }
    let places = significant_places(v, SIGNIFICANT as i32);
    // Round the digits before the point too: 123 456 to 123 500.
    let magnitude = if v == 0.0 {
        0.0
    } else {
        v.abs().plog10().floor()
    };
    let drop = (magnitude - (SIGNIFICANT as f64 - 1.0)).max(0.0);
    let v = if drop > 0.0 && drop.is_finite() {
        let unit = 10f64.ppowi(drop as i32);
        (v / unit).round() * unit
    } else {
        v
    };
    let text = trimmed(v, places);
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => ("−", rest),
        None => ("", text.as_str()),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (digits, None),
    };
    let mut out = String::from(sign);
    let count = whole.chars().count();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && count > 4 && (count - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    if let Some(f) = fraction {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// An area in the unit squared: `1450 mm²`, `0.0123 mm²`.
pub fn area(v: f64, unit: &str) -> String {
    if unit.is_empty() {
        significant(v)
    } else {
        format!("{} {unit}²", significant(v))
    }
}

/// A volume: cm³ in a millimetre document (`39.9 cm³`), else the unit
/// cubed.
pub fn volume(v: f64, unit: &str) -> String {
    match unit {
        "mm" => format!("{} cm³", significant(v / 1000.0)),
        "" => significant(v),
        _ => format!("{} {unit}³", significant(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_has_at_most_three_decimals_and_no_trailing_zeros() {
        let mm = |v: f64| length(v, 1e-6, "mm").text;
        assert_eq!(mm(30.0), "30 mm");
        assert_eq!(mm(25.980_762), "25.981 mm");
        assert_eq!(mm(0.0057), "0.0057 mm");
        assert_eq!(mm(0.045_396), "0.045 mm");
        assert_eq!(mm(5.0), "5 mm");
        assert_eq!(mm(-0.0001), "-0.0001 mm");
        assert_eq!(mm(-0.000_000_01), "0 mm");
        assert_eq!(mm(0.0), "0 mm");
        assert_eq!(mm(f64::NAN), "? mm");
        // The whole value, for the hover.
        let shown = length(25.980_762, 1e-6, "mm");
        assert_eq!(shown.whole, "25.980762 mm");
        assert!(shown.shortened());
        assert!(!length(30.0, 1e-6, "mm").shortened());
    }

    #[test]
    fn a_length_never_shows_more_than_the_tolerance_gives() {
        assert_eq!(length_number(25.980_762, 0.01, "mm"), "25.98");
        assert_eq!(length_number(0.0057, 0.001, "mm"), "0.006");
        assert_eq!(length_number(12.7, 1.0, "mm"), "13");
    }

    #[test]
    fn other_units_take_the_places_of_a_thousandth_of_a_millimetre() {
        assert_eq!(length_places("mm"), 3);
        assert_eq!(length_places("in"), 4);
        assert_eq!(length_places("cm"), 4);
        assert_eq!(length_places("m"), 6);
        assert_eq!(length_places("um"), 0);
        assert_eq!(length_places("parsec"), 3);
        assert_eq!(length(1.234_567, 1e-9, "in").text, "1.2346 in");
    }

    #[test]
    fn a_range_writes_each_end_by_the_rule() {
        let r = length_range(0.045_396, 0.1, 1e-6, "mm");
        assert_eq!(r.text, "0.045 – 0.1 mm");
        assert_eq!(r.whole, "0.045396 – 0.1 mm");
    }

    #[test]
    fn parts_along_the_axes() {
        let p = length_parts([15.0, 25.980_762, 0.0], 1e-6, "mm");
        assert_eq!(p.text, "X 15 · Y 25.981 · Z 0 mm");
    }

    #[test]
    fn an_angle_has_at_most_two_decimals() {
        assert_eq!(angle(135.0, 1e-9).text, "135°");
        assert_eq!(angle(22.4567, 1e-9).text, "22.46°");
        assert_eq!(angle(22.4567, 1e-9).whole, "22.4567°");
    }

    #[test]
    fn an_area_or_a_volume_has_four_significant_digits() {
        assert_eq!(area(1450.0, "mm"), "1450 mm²");
        assert_eq!(area(1450.123, "mm"), "1450 mm²");
        assert_eq!(area(0.012_34, "mm"), "0.01234 mm²");
        assert_eq!(area(0.0123, "mm"), "0.0123 mm²");
        assert_eq!(area(123_456.0, "mm"), "123\u{2009}500 mm²");
        assert_eq!(area(0.0, "mm"), "0 mm²");
        assert_eq!(volume(39_900.0, "mm"), "39.9 cm³");
        assert_eq!(volume(2.5, "m"), "2.5 m³");
    }
}
