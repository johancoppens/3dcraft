//! Dimensions, units and quantities.
//!
//! A quantity is a number in base units (millimetres, radians, kilograms,
//! seconds) plus the exponents of its dimension. Angle is treated as a
//! dimension of its own, although physics calls it dimensionless: that is
//! what lets `10 mm + 30 deg` be rejected.
//!
//! Mass and time make the mechanical quantities: a force is a mass times
//! a length over a time squared (`N` is kg·m/s²), a pressure or a stress a
//! force over an area (`MPa` is N/mm²), a moment a force times a length.
//! They are derived, never stored as dimensions of their own, so
//! `1000 N / (100 mm^2)` is 10 MPa.

use crate::error::UnitMismatch;
use std::fmt;

/// Exponents of the base dimensions. `length: 2` is an area, all zero is a
/// plain number; `length: 1, mass: 1, time: -2` is a force.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Dimension {
    pub length: i32,
    pub angle: i32,
    pub mass: i32,
    pub time: i32,
}

impl Dimension {
    pub const NONE: Dimension = Dimension::new(0, 0);
    pub const LENGTH: Dimension = Dimension::new(1, 0);
    pub const ANGLE: Dimension = Dimension::new(0, 1);
    pub const MASS: Dimension = Dimension::of(0, 0, 1, 0);
    pub const TIME: Dimension = Dimension::of(0, 0, 0, 1);
    /// Mass times length over time squared: newtons.
    pub const FORCE: Dimension = Dimension::of(1, 0, 1, -2);
    /// Force over area: pascals; a pressure and a stress.
    pub const PRESSURE: Dimension = Dimension::of(-1, 0, 1, -2);
    /// Force times length: a moment (a torque) and an energy alike.
    pub const MOMENT: Dimension = Dimension::of(2, 0, 1, -2);
    /// Mass over volume: kg/m³.
    pub const DENSITY: Dimension = Dimension::of(-3, 0, 1, 0);

    /// A dimension of length and angle only.
    pub const fn new(length: i32, angle: i32) -> Dimension {
        Dimension::of(length, angle, 0, 0)
    }

    pub const fn of(length: i32, angle: i32, mass: i32, time: i32) -> Dimension {
        Dimension {
            length,
            angle,
            mass,
            time,
        }
    }

    pub fn is_dimensionless(self) -> bool {
        self == Dimension::NONE
    }

    /// Whether it has a mass or a time in it: a force, a stress, a
    /// moment, a mass, a density.
    pub fn is_mechanical(self) -> bool {
        self.mass != 0 || self.time != 0
    }

    fn exponents(self) -> [i32; 4] {
        [self.length, self.angle, self.mass, self.time]
    }

    fn from_exponents([length, angle, mass, time]: [i32; 4]) -> Dimension {
        Dimension::of(length, angle, mass, time)
    }

    fn zip(self, other: Dimension, f: impl Fn(i32, i32) -> Option<i32>) -> Option<Dimension> {
        let [a, b, c, d] = self.exponents();
        let [e, g, h, k] = other.exponents();
        Some(Dimension::from_exponents([
            f(a, e)?,
            f(b, g)?,
            f(c, h)?,
            f(d, k)?,
        ]))
    }

    /// Dimension of a product. `None` when an exponent overflows.
    pub fn checked_mul(self, other: Dimension) -> Option<Dimension> {
        self.zip(other, i32::checked_add)
    }

    /// Dimension of a quotient. `None` when an exponent overflows.
    pub fn checked_div(self, other: Dimension) -> Option<Dimension> {
        self.zip(other, i32::checked_sub)
    }

    /// Dimension of an integer power. `None` when an exponent overflows.
    pub fn checked_pow(self, exponent: i32) -> Option<Dimension> {
        self.zip(self, |a, _| a.checked_mul(exponent))
    }

    /// Dimension of a square root. `None` when an exponent is odd.
    pub fn checked_sqrt(self) -> Option<Dimension> {
        if self.exponents().iter().any(|e| e % 2 != 0) {
            return None;
        }
        self.zip(self, |a, _| Some(a / 2))
    }

    /// The name of a mechanical dimension that has one.
    fn mechanical_name(self) -> Option<&'static str> {
        Some(match self {
            Dimension::MASS => "mass",
            Dimension::TIME => "time",
            Dimension::FORCE => "force",
            Dimension::PRESSURE => "pressure",
            Dimension::MOMENT => "moment",
            Dimension::DENSITY => "density",
            _ => return None,
        })
    }

    /// The unit a value of this dimension is shown in when nothing else
    /// says which, for a document whose lengths are in `length`: a
    /// force in N, a pressure or a stress in MPa, a moment (or an energy)
    /// in N m, a mass in kg, a time in s, a density in kg/m^3, whatever
    /// the length unit;
    /// anything else as the product of the document's length unit, rad,
    /// kg and s to their powers (`mm^2`, `mm*rad`, `kg*mm^-3`). None for
    /// a plain number.
    pub fn default_unit(self, length: Unit) -> ShownUnit {
        let named = |unit: Unit| ShownUnit {
            text: unit.symbol().to_string(),
            factor: unit.factor(),
            dimension: self,
            length_power: 0,
        };
        match self {
            Dimension::MASS => return named(Unit::Kilogram),
            Dimension::TIME => return named(Unit::Second),
            Dimension::FORCE => return named(Unit::Newton),
            Dimension::PRESSURE => return named(Unit::Megapascal),
            Dimension::MOMENT => {
                return ShownUnit {
                    text: "N m".to_string(),
                    factor: Unit::Newton.factor() * Unit::Metre.factor(),
                    dimension: self,
                    length_power: 0,
                };
            }
            // Per cubic metre, as a material's density is kept.
            Dimension::DENSITY => {
                return ShownUnit {
                    text: "kg/m^3".to_string(),
                    factor: Unit::Metre.factor().powi(-3),
                    dimension: self,
                    length_power: 0,
                };
            }
            _ => {}
        }
        let mut parts = Vec::new();
        for (symbol, exponent) in [
            (length.symbol(), self.length),
            ("rad", self.angle),
            ("kg", self.mass),
            ("s", self.time),
        ] {
            match exponent {
                0 => {}
                1 => parts.push(symbol.to_string()),
                n => parts.push(format!("{symbol}^{n}")),
            }
        }
        ShownUnit {
            text: parts.join("*"),
            factor: length.factor().powi(self.length),
            dimension: self,
            length_power: self.length,
        }
    }
}

impl Dimension {
    /// The dimension with its article, for messages: "a length",
    /// "an angle", "a length^-1"; "a plain number" for none.
    pub fn described(self) -> String {
        if self.is_dimensionless() {
            return "a plain number".to_string();
        }
        let name = self.to_string();
        let article = if name.starts_with(['a', 'e', 'i', 'o', 'u']) {
            "an"
        } else {
            "a"
        };
        format!("{article} {name}")
    }

    /// For the value of `source` with this dimension, when it fell into
    /// the trap of a unit read as belonging to a divisor: a negative
    /// exponent, and `source` divides by a number written with a unit right
    /// after the `/`, without parentheses (`1/16 in` is 1 / (16 in)). A
    /// sentence that says so and how to write what was meant, starting
    /// with "; ". Empty otherwise: `10 mm / (360 deg)` and `1 / (2 mm)`
    /// mean what they say.
    pub fn unit_hint(self, source: &str) -> &'static str {
        if (self.length < 0 || self.angle < 0) && divides_by_a_unit(source) {
            "; a unit belongs to the number just before it, so `1/16 in` is 1 / (16 in) and \
             `pi / 4 rad` is pi / (4 rad): write `1 in / 16` and `pi / 4 * 1 rad`"
        } else {
            ""
        }
    }
}

/// Whether `source` has a `/` followed directly by a number and a unit
/// (`1/16 in`, `pi / 4 rad`), the divisor not in parentheses.
fn divides_by_a_unit(source: &str) -> bool {
    use crate::lexer::{TokenKind, tokenize};
    let Ok(tokens) = tokenize(source) else {
        return false;
    };
    tokens.windows(3).any(|three| match three {
        [slash, number, unit] => {
            slash.kind == TokenKind::Slash
                && matches!(number.kind, TokenKind::Number(_))
                && unit.kind == TokenKind::Ident
                && source
                    .get(unit.pos..unit.end)
                    .and_then(Unit::from_symbol)
                    .is_some()
        }
        _ => false,
    })
}

impl fmt::Display for Dimension {
    /// `dimensionless`, `length`, `length^2`, `length * angle^-1`; a
    /// force, a pressure, a moment, a mass and a time by their names,
    /// another mechanical one as `length^-3 * mass`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "dimensionless");
        }
        if let Some(name) = self.mechanical_name() {
            return f.write_str(name);
        }
        let mut first = true;
        for (name, exponent) in [
            ("length", self.length),
            ("angle", self.angle),
            ("mass", self.mass),
            ("time", self.time),
        ] {
            if exponent == 0 {
                continue;
            }
            if !first {
                write!(f, " * ")?;
            }
            first = false;
            if exponent == 1 {
                write!(f, "{name}")?;
            } else {
                write!(f, "{name}^{exponent}")?;
            }
        }
        Ok(())
    }
}

/// The units an expression may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Micrometre,
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
    Degree,
    Radian,
    Gram,
    Kilogram,
    Second,
    Newton,
    Kilonewton,
    Pascal,
    Kilopascal,
    Megapascal,
    Gigapascal,
    Joule,
}

impl Unit {
    /// The units of length and angle. Their symbols are reserved: no
    /// variable may be named `mm` or `deg`.
    pub const ALL: [Unit; 8] = [
        Unit::Micrometre,
        Unit::Millimetre,
        Unit::Centimetre,
        Unit::Metre,
        Unit::Inch,
        Unit::Foot,
        Unit::Degree,
        Unit::Radian,
    ];

    /// The units of mass, time, force, pressure (and stress) and energy.
    /// Their symbols are not reserved, because designs name variables
    /// `N`, `s`, `g` and `t`: they are units only right after a number
    /// (`20 N`), a parenthesis or `pi`, and a variable everywhere else.
    /// A force unit followed by a length unit is a moment: `20 N m`.
    pub const MECHANICAL: [Unit; 10] = [
        Unit::Gram,
        Unit::Kilogram,
        Unit::Second,
        Unit::Newton,
        Unit::Kilonewton,
        Unit::Pascal,
        Unit::Kilopascal,
        Unit::Megapascal,
        Unit::Gigapascal,
        Unit::Joule,
    ];

    /// Every unit: [`Unit::ALL`], then [`Unit::MECHANICAL`].
    pub fn every() -> impl Iterator<Item = Unit> {
        Unit::ALL.into_iter().chain(Unit::MECHANICAL)
    }

    /// Whether the symbol is a reserved name (a length or an angle unit).
    pub fn is_reserved(self) -> bool {
        Unit::ALL.contains(&self)
    }

    /// The unit as written in an expression.
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Micrometre => "um",
            Unit::Millimetre => "mm",
            Unit::Centimetre => "cm",
            Unit::Metre => "m",
            Unit::Inch => "in",
            Unit::Foot => "ft",
            Unit::Degree => "deg",
            Unit::Radian => "rad",
            Unit::Gram => "g",
            Unit::Kilogram => "kg",
            Unit::Second => "s",
            Unit::Newton => "N",
            Unit::Kilonewton => "kN",
            Unit::Pascal => "Pa",
            Unit::Kilopascal => "kPa",
            Unit::Megapascal => "MPa",
            Unit::Gigapascal => "GPa",
            Unit::Joule => "J",
        }
    }

    /// The unit spelled out: `millimetre`, `newton`.
    pub fn spelled(self) -> &'static str {
        match self {
            Unit::Micrometre => "micrometre",
            Unit::Millimetre => "millimetre",
            Unit::Centimetre => "centimetre",
            Unit::Metre => "metre",
            Unit::Inch => "inch",
            Unit::Foot => "foot",
            Unit::Degree => "degree",
            Unit::Radian => "radian",
            Unit::Gram => "gram",
            Unit::Kilogram => "kilogram",
            Unit::Second => "second",
            Unit::Newton => "newton",
            Unit::Kilonewton => "kilonewton",
            Unit::Pascal => "pascal",
            Unit::Kilopascal => "kilopascal",
            Unit::Megapascal => "megapascal",
            Unit::Gigapascal => "gigapascal",
            Unit::Joule => "joule",
        }
    }

    /// Inverse of [`Unit::symbol`]. Case-sensitive.
    pub fn from_symbol(symbol: &str) -> Option<Unit> {
        Unit::every().find(|unit| unit.symbol() == symbol)
    }

    pub fn dimension(self) -> Dimension {
        match self {
            Unit::Micrometre
            | Unit::Millimetre
            | Unit::Centimetre
            | Unit::Metre
            | Unit::Inch
            | Unit::Foot => Dimension::LENGTH,
            Unit::Degree | Unit::Radian => Dimension::ANGLE,
            Unit::Gram | Unit::Kilogram => Dimension::MASS,
            Unit::Second => Dimension::TIME,
            Unit::Newton | Unit::Kilonewton => Dimension::FORCE,
            Unit::Pascal | Unit::Kilopascal | Unit::Megapascal | Unit::Gigapascal => {
                Dimension::PRESSURE
            }
            Unit::Joule => Dimension::MOMENT,
        }
    }

    /// How many base units (millimetres, radians, kilograms, seconds and
    /// their products) one of this unit is. The inch is exactly 25.4 mm by
    /// definition (1959), the foot 12 inches. A newton is 1 kg·m/s², so
    /// 1000 kg·mm/s²; a pascal 1 N/m², so 10⁻³ kg/(mm·s²), and a
    /// megapascal (1 N/mm²) 1000 of them; a joule 1 N·m.
    pub fn factor(self) -> f64 {
        match self {
            Unit::Micrometre => 1e-3,
            Unit::Millimetre => 1.0,
            Unit::Centimetre => 10.0,
            Unit::Metre => 1000.0,
            Unit::Inch => 25.4,
            Unit::Foot => 304.8,
            Unit::Degree => std::f64::consts::PI / 180.0,
            Unit::Radian => 1.0,
            Unit::Gram => 1e-3,
            Unit::Kilogram => 1.0,
            Unit::Second => 1.0,
            Unit::Newton => 1e3,
            Unit::Kilonewton => 1e6,
            Unit::Pascal => 1e-3,
            Unit::Kilopascal => 1.0,
            Unit::Megapascal => 1e3,
            Unit::Gigapascal => 1e6,
            Unit::Joule => 1e6,
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// A unit a value is shown in: one unit or a product of units to powers,
/// as written (`MPa`, `N m`, `N/mm²`, `kg/m^3`), with how many base units
/// one of it is and its dimension.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownUnit {
    /// As written, trimmed; empty for a plain number.
    pub text: String,
    pub factor: f64,
    pub dimension: Dimension,
    /// The power of the document's length unit it is made with: a
    /// document's default for an area is `mm^2` (2), a chosen unit and a
    /// named one (`MPa`) do not change with the document (0).
    pub length_power: i32,
}

/// Longest unit text [`ShownUnit::parse`] reads, in bytes.
pub const MAX_UNIT_LEN: usize = 64;

impl ShownUnit {
    /// Reads a unit: units (their symbols, as in expressions) to powers
    /// (`mm^2`, `mm²`, `s^-1`), multiplied by a space, `*` or `·`, with at
    /// most one `/` before the units that divide: `MPa`, `N m`, `kN*m`,
    /// `N/mm²`, `kg/m^3`, `mm/m`. The error says what is wrong.
    pub fn parse(text: &str) -> Result<ShownUnit, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err("a unit is empty".into());
        }
        if trimmed.len() > MAX_UNIT_LEN {
            return Err(format!("a unit is at most {MAX_UNIT_LEN} bytes"));
        }
        let mut halves = trimmed.split('/');
        let top = halves.next().unwrap_or("");
        let bottom = halves.next();
        if halves.next().is_some() {
            return Err(format!("`{trimmed}` has more than one `/`"));
        }
        let mut factor = 1.0_f64;
        let mut dimension = Dimension::NONE;
        let mut any = false;
        for (part, sign) in [(Some(top), 1), (bottom, -1)] {
            let Some(part) = part else { continue };
            let terms: Vec<&str> = part
                .split(|c: char| c.is_whitespace() || c == '*' || c == '·' || c == '⋅')
                .filter(|t| !t.is_empty())
                .collect();
            if terms.is_empty() {
                return Err(format!(
                    "`{trimmed}` has no unit {}",
                    if sign == 1 { "before `/`" } else { "after `/`" }
                ));
            }
            for term in terms {
                if term == "1" && sign == 1 {
                    continue;
                }
                let (symbol, power) = power_of(term).ok_or_else(|| {
                    format!(
                        "`{term}` in `{trimmed}` is not a unit to a whole power (`mm^2`, `mm²`)"
                    )
                })?;
                let unit = Unit::from_symbol(symbol).ok_or_else(|| {
                    format!(
                        "`{symbol}` in `{trimmed}` is not a unit; the units are {}",
                        unit_list()
                    )
                })?;
                let power = power.checked_mul(sign).ok_or("a power is too large")?;
                any = true;
                factor *= unit.factor().powi(power);
                dimension = unit
                    .dimension()
                    .checked_pow(power)
                    .and_then(|d| dimension.checked_mul(d))
                    .ok_or("a power is too large")?;
            }
        }
        if !any {
            return Err(format!("`{trimmed}` has no unit"));
        }
        if !(factor.is_finite() && factor > 0.0) {
            return Err(format!("`{trimmed}` is too large or too small a unit"));
        }
        Ok(ShownUnit {
            text: trimmed.to_string(),
            factor,
            dimension,
            length_power: 0,
        })
    }

    /// `quantity` as a number of this unit; an error that names both
    /// dimensions when they differ.
    pub fn value_of(&self, quantity: Quantity) -> Result<f64, String> {
        if quantity.dimension != self.dimension {
            return Err(format!(
                "the unit `{}` is {}, but the value is {}",
                self.text,
                self.dimension.described(),
                quantity.dimension.described()
            ));
        }
        Ok(quantity.value / self.factor)
    }
}

/// The symbol and the power of a term of a unit: `mm^2`, `mm²`, `s^-1`,
/// `N` (1). `None` for a power that is not a whole number from -9 to 9.
fn power_of(term: &str) -> Option<(&str, i32)> {
    if let Some((symbol, power)) = term.split_once('^') {
        let power: i32 = power.parse().ok()?;
        return ((-9..=9).contains(&power) && power != 0 && !symbol.is_empty())
            .then_some((symbol, power));
    }
    for (mark, power) in [('²', 2), ('³', 3)] {
        if let Some(symbol) = term.strip_suffix(mark) {
            return (!symbol.is_empty()).then_some((symbol, power));
        }
    }
    Some((term, 1))
}

/// Every unit's symbol, for messages: `um, mm, ..., GPa and J`.
pub(crate) fn unit_list() -> String {
    let symbols: Vec<&str> = Unit::every().map(Unit::symbol).collect();
    match symbols.split_last() {
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// A value with a dimension. `value` is in base units: millimetres for
/// length, radians for angle, kilograms for mass and seconds for time (so
/// an area is in mm² and a force in kg·mm/s², a thousandth of a newton).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub dimension: Dimension,
}

impl Quantity {
    pub const fn new(value: f64, dimension: Dimension) -> Quantity {
        Quantity { value, dimension }
    }

    pub const fn dimensionless(value: f64) -> Quantity {
        Quantity::new(value, Dimension::NONE)
    }

    /// `value` of `unit`, converted to base units.
    pub fn from_unit(value: f64, unit: Unit) -> Quantity {
        Quantity::new(value * unit.factor(), unit.dimension())
    }

    /// The quantity as a number of `unit`. Fails when `unit` measures
    /// another dimension (an area cannot be given in `mm`).
    pub fn to_unit(self, unit: Unit) -> Result<f64, UnitMismatch> {
        if self.dimension != unit.dimension() {
            return Err(UnitMismatch {
                unit,
                dimension: self.dimension,
            });
        }
        Ok(self.value / unit.factor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative bound for one multiplication and one division in `f64`.
    const EPS: f64 = 1e-15;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= EPS * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn length_units_convert_to_millimetres() {
        let cases = [
            (Unit::Micrometre, 0.001),
            (Unit::Millimetre, 1.0),
            (Unit::Centimetre, 10.0),
            (Unit::Metre, 1000.0),
            (Unit::Inch, 25.4),
            (Unit::Foot, 304.8),
        ];
        for (unit, mm) in cases {
            let q = Quantity::from_unit(1.0, unit);
            assert_eq!(q.dimension, Dimension::LENGTH, "{unit}");
            assert!(close(q.value, mm), "{unit}: {}", q.value);
        }
    }

    #[test]
    fn angle_units_convert_to_radians() {
        let half_turn = Quantity::from_unit(180.0, Unit::Degree);
        assert_eq!(half_turn.dimension, Dimension::ANGLE);
        assert!(close(half_turn.value, std::f64::consts::PI));
        assert_eq!(Quantity::from_unit(2.0, Unit::Radian).value, 2.0);
    }

    #[test]
    fn to_unit_inverts_from_unit() {
        for unit in Unit::ALL {
            let back = Quantity::from_unit(7.25, unit).to_unit(unit).unwrap();
            assert!(close(back, 7.25), "{unit}: {back}");
        }
        let inch = Quantity::from_unit(1.0, Unit::Inch);
        assert!(close(inch.to_unit(Unit::Centimetre).unwrap(), 2.54));
        assert!(close(inch.to_unit(Unit::Foot).unwrap(), 1.0 / 12.0));
    }

    #[test]
    fn to_unit_rejects_another_dimension() {
        let length = Quantity::from_unit(1.0, Unit::Millimetre);
        assert_eq!(
            length.to_unit(Unit::Degree),
            Err(UnitMismatch {
                unit: Unit::Degree,
                dimension: Dimension::LENGTH
            })
        );
        let area = Quantity::new(4.0, Dimension::new(2, 0));
        assert!(area.to_unit(Unit::Millimetre).is_err());
        assert!(Quantity::dimensionless(1.0).to_unit(Unit::Radian).is_err());
        let message = area.to_unit(Unit::Millimetre).unwrap_err().to_string();
        assert!(message.contains("length^2") && message.contains("`mm`"));
    }

    #[test]
    fn symbols_round_trip_and_are_case_sensitive() {
        for unit in Unit::ALL {
            assert_eq!(Unit::from_symbol(unit.symbol()), Some(unit));
        }
        assert_eq!(Unit::from_symbol("MM"), None);
        assert_eq!(Unit::from_symbol(""), None);
        assert_eq!(Unit::from_symbol("mmm"), None);
    }

    #[test]
    fn dimensions_are_described_with_their_article_and_an_inverse_says_why() {
        assert_eq!(Dimension::LENGTH.described(), "a length");
        assert_eq!(Dimension::ANGLE.described(), "an angle");
        assert_eq!(Dimension::NONE.described(), "a plain number");
        assert_eq!(Dimension::new(-1, 0).described(), "a length^-1");
        assert_eq!(Dimension::new(0, -1).described(), "an angle^-1");
        // `1/16 in` and `pi / 4 rad` fell into the trap: the hint says how
        // to write what was meant.
        let inverse_length = Dimension::new(-1, 0);
        let inverse_angle = Dimension::new(0, -1);
        assert!(inverse_length.unit_hint("1/16 in").contains("`1 in / 16`"));
        assert!(
            inverse_length
                .unit_hint("w * 2 / 3 mm")
                .contains("`1 in / 16`")
        );
        assert!(
            inverse_angle
                .unit_hint("pi / 4 rad")
                .contains("`pi / 4 * 1 rad`")
        );
        // Inverses written as meant: nothing.
        for (d, source) in [
            (inverse_length, "1 / (2 mm)"),
            (inverse_length, "n / width"),
            (Dimension::new(1, -1), "10 mm / (360 deg)"),
            (inverse_angle, "1 / (4 rad)"),
        ] {
            assert_eq!(d.unit_hint(source), "", "{source}");
        }
        // Not an inverse, whatever the text: nothing.
        for d in [Dimension::NONE, Dimension::LENGTH, Dimension::new(2, 0)] {
            assert_eq!(d.unit_hint("1/16 in"), "");
        }
    }

    #[test]
    fn dimension_arithmetic() {
        let l = Dimension::LENGTH;
        assert_eq!(l.checked_mul(l), Some(Dimension::new(2, 0)));
        assert_eq!(l.checked_div(l), Some(Dimension::NONE));
        assert_eq!(l.checked_pow(-3), Some(Dimension::new(-3, 0)));
        assert_eq!(
            Dimension::new(2, -4).checked_sqrt(),
            Some(Dimension::new(1, -2))
        );
        assert_eq!(l.checked_sqrt(), None);
        assert_eq!(Dimension::new(2, 1).checked_sqrt(), None);
        assert_eq!(Dimension::new(-1, 0).checked_sqrt(), None);
        assert_eq!(Dimension::new(i32::MAX, 0).checked_mul(l), None);
        assert_eq!(Dimension::new(i32::MIN, 0).checked_div(l), None);
        assert_eq!(Dimension::new(i32::MAX, 0).checked_pow(2), None);
    }

    #[test]
    fn dimension_display() {
        assert_eq!(Dimension::NONE.to_string(), "dimensionless");
        assert_eq!(Dimension::LENGTH.to_string(), "length");
        assert_eq!(Dimension::ANGLE.to_string(), "angle");
        assert_eq!(Dimension::new(2, 0).to_string(), "length^2");
        assert_eq!(Dimension::new(1, -1).to_string(), "length * angle^-1");
    }

    fn eval(source: &str) -> Quantity {
        crate::evaluate(source, |_| None).unwrap()
    }

    #[test]
    fn mechanical_dimensions_are_derived() {
        let kg_m_per_s2 = Dimension::MASS
            .checked_mul(Dimension::LENGTH)
            .and_then(|d| d.checked_div(Dimension::TIME.checked_pow(2)?))
            .unwrap();
        assert_eq!(kg_m_per_s2, Dimension::FORCE);
        assert_eq!(
            Dimension::FORCE.checked_div(Dimension::new(2, 0)),
            Some(Dimension::PRESSURE)
        );
        assert_eq!(
            Dimension::FORCE.checked_mul(Dimension::LENGTH),
            Some(Dimension::MOMENT)
        );
        assert_eq!(
            Dimension::of(2, 0, 2, -4).checked_sqrt(),
            Some(Dimension::of(1, 0, 1, -2))
        );
        assert_eq!(Dimension::FORCE.checked_sqrt(), None);
        assert_eq!(
            Dimension::of(0, 0, i32::MAX, 0).checked_mul(Dimension::MASS),
            None
        );
        assert!(Dimension::FORCE.is_mechanical() && !Dimension::LENGTH.is_mechanical());
        // Named where they have a name, spelled out otherwise.
        assert_eq!(Dimension::FORCE.to_string(), "force");
        assert_eq!(Dimension::PRESSURE.described(), "a pressure");
        assert_eq!(Dimension::MOMENT.described(), "a moment");
        assert_eq!(Dimension::MASS.described(), "a mass");
        assert_eq!(Dimension::DENSITY.to_string(), "density");
        assert_eq!(Dimension::of(-2, 0, 1, 0).to_string(), "length^-2 * mass");
    }

    #[test]
    fn mechanical_units_convert_and_combine() {
        // load / area is a stress: 1000 N on 100 mm² is 10 MPa.
        let stress = eval("1000 N / (100 * (1 mm)^2)");
        assert_eq!(stress.dimension, Dimension::PRESSURE);
        assert!(close(stress.to_unit(Unit::Megapascal).unwrap(), 10.0));
        assert!(close(stress.to_unit(Unit::Pascal).unwrap(), 1e7));
        // N/mm² is MPa, and kg·m/s² is N.
        assert!(close(eval("1 MPa").value, eval("1 N / (1 mm)^2").value));
        let newton = eval("1 kg * 1 m / (1 s)^2");
        assert_eq!(newton.dimension, Dimension::FORCE);
        assert!(close(newton.to_unit(Unit::Newton).unwrap(), 1.0));
        assert!(close(eval("2 kN").to_unit(Unit::Newton).unwrap(), 2000.0));
        assert!(close(
            eval("0.21 GPa").to_unit(Unit::Megapascal).unwrap(),
            210.0
        ));
        assert!(close(eval("500 g").to_unit(Unit::Kilogram).unwrap(), 0.5));
        assert!(close(eval("3 kPa").to_unit(Unit::Pascal).unwrap(), 3000.0));
        // A moment: a force unit then a length unit.
        let moment = eval("20 N m");
        assert_eq!(moment.dimension, Dimension::MOMENT);
        assert!(close(moment.to_unit(Unit::Joule).unwrap(), 20.0));
        assert!(close(eval("5 kN mm").value, eval("5 N m").value));
        assert!(close(eval("200 N * 0.1 m").value, eval("20 J").value));
        // A stress and a length do not add.
        assert!(crate::evaluate("1 MPa + 1 mm", |_| None).is_err());
        for unit in Unit::MECHANICAL {
            assert_eq!(Unit::from_symbol(unit.symbol()), Some(unit));
            let back = Quantity::from_unit(7.25, unit).to_unit(unit).unwrap();
            assert!(close(back, 7.25), "{unit}: {back}");
            assert!(!unit.is_reserved());
        }
    }

    #[test]
    fn mechanical_symbols_stay_free_as_variable_names() {
        // `N` a count, `t` a thickness, `s` a spacing: variables still, and
        // a unit right after a number.
        for name in ["N", "s", "g", "kg", "J", "MPa"] {
            assert!(!crate::is_reserved_name(name), "{name}");
            assert!(crate::check_variable_name(name).is_ok(), "{name}");
        }
        let n = Quantity::dimensionless(3.0);
        let q = crate::evaluate("N * 2 N", |name| (name == "N").then_some(n)).unwrap();
        assert_eq!(q.dimension, Dimension::FORCE);
        assert!(close(q.to_unit(Unit::Newton).unwrap(), 6.0));
        // A rename of the variable leaves the unit alone.
        assert_eq!(
            crate::rename_variable("N * 2 N + (N) * 1 N", "N", "teeth").unwrap(),
            "teeth * 2 N + (teeth) * 1 N"
        );
        assert!(!crate::refers_to_variable("2 N", "N"));
        assert_eq!(crate::names_in("N * 2 N"), ["N"]);
        // Alone, an undefined `N` says how to write the unit.
        let alone = crate::evaluate("2 * N", |_| None).unwrap_err().to_string();
        assert!(alone.contains("write `1 N`"), "{alone}");
        // A stress written `1000 N / 100 mm^2` divides by (100 mm)²: the
        // hint says a unit belongs to the number before it.
        assert!(
            !Dimension::PRESSURE
                .unit_hint("1000 N / 100 mm^2")
                .is_empty()
        );
        // A spelled unit is pointed to its symbol.
        let error = crate::evaluate("2 Mpa", |_| None).unwrap_err().to_string();
        assert!(error.contains("`MPa`"), "{error}");
    }

    #[test]
    fn shown_units_parse_as_products_and_quotients() {
        let cases = [
            ("MPa", 1e3, Dimension::PRESSURE),
            ("N/mm²", 1e3, Dimension::PRESSURE),
            ("N/mm^2", 1e3, Dimension::PRESSURE),
            (" N m ", 1e6, Dimension::MOMENT),
            ("N*mm", 1e3, Dimension::MOMENT),
            ("kN·m", 1e9, Dimension::MOMENT),
            ("kg/m^3", 1e-9, Dimension::of(-3, 0, 1, 0)),
            ("mm/m", 1e-3, Dimension::NONE),
            ("1/s", 1.0, Dimension::of(0, 0, 0, -1)),
            ("mm³", 1.0, Dimension::new(3, 0)),
        ];
        for (text, factor, dimension) in cases {
            let unit = ShownUnit::parse(text).unwrap();
            assert_eq!(unit.text, text.trim());
            assert!(close(unit.factor, factor), "{text}: {}", unit.factor);
            assert_eq!(unit.dimension, dimension, "{text}");
        }
        let mpa = ShownUnit::parse("MPa").unwrap();
        assert!(close(
            mpa.value_of(eval("1000 N / (100 * (1 mm)^2)")).unwrap(),
            10.0
        ));
        let wrong = mpa.value_of(eval("3 mm")).unwrap_err();
        assert!(
            wrong.contains("a pressure") && wrong.contains("a length"),
            "{wrong}"
        );
        for bad in [
            "", "  ", "Nm", "N/", "/mm", "N/mm/s", "mm^x", "mm^0", "mm^99", "MPA", "2 mm", "^2",
        ] {
            assert!(ShownUnit::parse(bad).is_err(), "{bad}");
        }
        assert!(ShownUnit::parse(&"mm ".repeat(40)).is_err());
    }

    #[test]
    fn default_units_name_the_mechanical_dimensions() {
        let shown = |d: Dimension, length: Unit| {
            let u = d.default_unit(length);
            (u.text, u.factor, u.length_power)
        };
        assert_eq!(shown(Dimension::FORCE, Unit::Metre), ("N".into(), 1e3, 0));
        assert_eq!(
            shown(Dimension::PRESSURE, Unit::Millimetre),
            ("MPa".into(), 1e3, 0)
        );
        assert_eq!(shown(Dimension::MOMENT, Unit::Inch), ("N m".into(), 1e6, 0));
        assert_eq!(
            shown(Dimension::MASS, Unit::Millimetre),
            ("kg".into(), 1.0, 0)
        );
        // The others as before: the document's length unit to its power.
        assert_eq!(
            shown(Dimension::new(2, 0), Unit::Centimetre),
            ("cm^2".into(), 100.0, 2)
        );
        assert_eq!(
            shown(Dimension::new(1, 1), Unit::Millimetre),
            ("mm*rad".into(), 1.0, 1)
        );
        assert_eq!(
            shown(Dimension::NONE, Unit::Millimetre),
            (String::new(), 1.0, 0)
        );
        assert_eq!(
            shown(Dimension::DENSITY, Unit::Millimetre),
            ("kg/m^3".into(), 1e-9, 0)
        );
        assert_eq!(
            shown(Dimension::of(-2, 0, 1, 0), Unit::Millimetre),
            ("mm^-2*kg".into(), 1.0, -2)
        );
    }
}
