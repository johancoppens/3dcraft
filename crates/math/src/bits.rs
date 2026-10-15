//! Values written out as words, every bit of every number: two values
//! give the same words only when they are the same value bit for bit (the
//! sign of a zero and the payload of a NaN included). A key for memos that
//! must never take one value for another, at the cost of copying the
//! numbers rather than formatting them; not a file format.
//!
//! Every implementation lists all fields of its type by destructuring, so
//! a field added later does not compile until it is written out too; a
//! list starts with its length and an enum with its variant, so the words
//! of two different values never coincide.

use crate::{Tolerance, Vec3};

/// See the module documentation.
pub trait Bits {
    /// Appends the words of `self` to `out`.
    fn push_bits(&self, out: &mut Vec<u64>);
}

/// The words of `value`.
pub fn bits_of<T: Bits + ?Sized>(value: &T) -> Vec<u64> {
    let mut out = Vec::new();
    value.push_bits(&mut out);
    out
}

impl Bits for f64 {
    fn push_bits(&self, out: &mut Vec<u64>) {
        out.push(self.to_bits());
    }
}

impl Bits for bool {
    fn push_bits(&self, out: &mut Vec<u64>) {
        out.push(u64::from(*self));
    }
}

impl Bits for u32 {
    fn push_bits(&self, out: &mut Vec<u64>) {
        out.push(u64::from(*self));
    }
}

impl Bits for usize {
    fn push_bits(&self, out: &mut Vec<u64>) {
        // A usize is at most 64 bits on every target 3DCraft builds for;
        // saturating keeps this total all the same.
        out.push(u64::try_from(*self).unwrap_or(u64::MAX));
    }
}

impl<T: Bits> Bits for [T] {
    fn push_bits(&self, out: &mut Vec<u64>) {
        self.len().push_bits(out);
        for item in self {
            item.push_bits(out);
        }
    }
}

impl<T: Bits> Bits for Vec<T> {
    fn push_bits(&self, out: &mut Vec<u64>) {
        self.as_slice().push_bits(out);
    }
}

impl<A: Bits, B: Bits> Bits for (A, B) {
    fn push_bits(&self, out: &mut Vec<u64>) {
        self.0.push_bits(out);
        self.1.push_bits(out);
    }
}

impl Bits for Vec3 {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Vec3 { x, y, z } = self;
        out.extend([x.to_bits(), y.to_bits(), z.to_bits()]);
    }
}

impl Bits for Tolerance {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Tolerance { linear, angular } = self;
        out.extend([linear.to_bits(), angular.to_bits()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_bit_for_bit() {
        assert_ne!(bits_of(&0.0f64), bits_of(&-0.0f64));
        assert_eq!(bits_of(&1.5f64), bits_of(&1.5f64));
        assert_ne!(bits_of(&f64::NAN), bits_of(&-f64::NAN));
        assert_ne!(
            bits_of(&1.0f64),
            bits_of(&f64::from_bits(1.0f64.to_bits() + 1))
        );
    }

    #[test]
    fn lists_start_with_their_length() {
        // [[1], []] and [[], [1]] hold the same numbers.
        let a: Vec<Vec<f64>> = vec![vec![1.0], vec![]];
        let b: Vec<Vec<f64>> = vec![vec![], vec![1.0]];
        assert_ne!(bits_of(&a), bits_of(&b));
    }

    #[test]
    fn a_frame_is_all_its_axes() {
        use crate::Frame;
        let moved = Frame::WORLD.at(Vec3::new(0.0, 0.0, 1e-300)).unwrap();
        assert_ne!(bits_of(&Frame::WORLD), bits_of(&moved));
        assert_eq!(bits_of(&Frame::WORLD), bits_of(&Frame::WORLD));
    }
}
