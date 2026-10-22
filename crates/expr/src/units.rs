//! Dimensions, units and quantities.
//!
//! A quantity is a number in base units (millimetres, radians) plus the
//! exponents of its dimension. Angle is treated as a dimension of its own,
//! although physics calls it dimensionless: that is what lets
//! `10 mm + 30 deg` be rejected.

use crate::error::UnitMismatch;
use std::fmt;

/// Exponents of the base dimensions. `length: 2` is an area, both zero is a
/// plain number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Dimension {
    pub length: i32,
    pub angle: i32,
}

impl Dimension {
    pub const NONE: Dimension = Dimension::new(0, 0);
    pub const LENGTH: Dimension = Dimension::new(1, 0);
    pub const ANGLE: Dimension = Dimension::new(0, 1);

    pub const fn new(length: i32, angle: i32) -> Dimension {
        Dimension { length, angle }
    }

    pub fn is_dimensionless(self) -> bool {
        self == Dimension::NONE
    }

    /// Dimension of a product. `None` when an exponent overflows.
    pub fn checked_mul(self, other: Dimension) -> Option<Dimension> {
        Some(Dimension::new(
            self.length.checked_add(other.length)?,
            self.angle.checked_add(other.angle)?,
        ))
    }

    /// Dimension of a quotient. `None` when an exponent overflows.
    pub fn checked_div(self, other: Dimension) -> Option<Dimension> {
        Some(Dimension::new(
            self.length.checked_sub(other.length)?,
            self.angle.checked_sub(other.angle)?,
        ))
    }

    /// Dimension of an integer power. `None` when an exponent overflows.
    pub fn checked_pow(self, exponent: i32) -> Option<Dimension> {
        Some(Dimension::new(
            self.length.checked_mul(exponent)?,
            self.angle.checked_mul(exponent)?,
        ))
    }

    /// Dimension of a square root. `None` when an exponent is odd.
    pub fn checked_sqrt(self) -> Option<Dimension> {
        if self.length % 2 != 0 || self.angle % 2 != 0 {
            return None;
        }
        Some(Dimension::new(self.length / 2, self.angle / 2))
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

    /// For a dimension with a negative exponent, which is nearly always a
    /// unit read as belonging to a divisor (`1/16 in` is 1 / (16 in)): a
    /// sentence that says so and how to write what was meant, starting with
    /// "; ". Empty for every other dimension.
    pub fn unit_hint(self) -> &'static str {
        if self.length < 0 || self.angle < 0 {
            "; a unit belongs to the number just before it, so `1/16 in` is 1 / (16 in) and \
             `pi / 4 rad` is pi / (4 rad): write `1 in / 16` and `pi / 4 * 1 rad`"
        } else {
            ""
        }
    }
}

impl fmt::Display for Dimension {
    /// `dimensionless`, `length`, `length^2`, `length * angle^-1`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "dimensionless");
        }
        let mut first = true;
        for (name, exponent) in [("length", self.length), ("angle", self.angle)] {
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
}

impl Unit {
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
        }
    }

    /// Inverse of [`Unit::symbol`]. Case-sensitive.
    pub fn from_symbol(symbol: &str) -> Option<Unit> {
        Unit::ALL.into_iter().find(|unit| unit.symbol() == symbol)
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
        }
    }

    /// How many base units (millimetres or radians) one of this unit is.
    /// The inch is exactly 25.4 mm by definition (1959), the foot 12 inches.
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
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// A value with a dimension. `value` is in base units: millimetres for
/// length, radians for angle (so an area is in mm²).
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
        // `1/16 in` and `pi / 4 rad` are inverses: the hint says how to
        // write what was meant. Nothing for the others.
        assert!(Dimension::new(-1, 0).unit_hint().contains("`1 in / 16`"));
        assert!(
            Dimension::new(0, -1)
                .unit_hint()
                .contains("`pi / 4 * 1 rad`")
        );
        for d in [Dimension::NONE, Dimension::LENGTH, Dimension::new(2, 0)] {
            assert_eq!(d.unit_hint(), "");
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
}
