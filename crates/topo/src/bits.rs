//! A body written out as words, every bit of every number of it
//! ([`craft3d_math::Bits`]): two bodies give the same words only when they
//! are the same body, bit for bit.

use crate::{Body, BodyKind, Coedge, Edge, Face, Loop, Shell, Vertex};
use craft3d_math::Bits;

impl Bits for Vertex {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Vertex { point } = self;
        point.push_bits(out);
    }
}

impl Bits for Edge {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Edge {
            curve,
            range,
            start,
            end,
        } = self;
        curve.push_bits(out);
        range.push_bits(out);
        start.0.push_bits(out);
        end.0.push_bits(out);
    }
}

impl Bits for Coedge {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Coedge { edge, reversed } = self;
        edge.0.push_bits(out);
        reversed.push_bits(out);
    }
}

impl Bits for Loop {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Loop { coedges } = self;
        coedges.push_bits(out);
    }
}

impl Bits for Face {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Face {
            surface,
            reversed,
            loops,
        } = self;
        surface.push_bits(out);
        reversed.push_bits(out);
        loops.len().push_bits(out);
        for l in loops {
            l.0.push_bits(out);
        }
    }
}

impl Bits for Shell {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Shell { faces } = self;
        faces.len().push_bits(out);
        for f in faces {
            f.0.push_bits(out);
        }
    }
}

impl Bits for BodyKind {
    fn push_bits(&self, out: &mut Vec<u64>) {
        out.push(match self {
            BodyKind::Solid => 0,
            BodyKind::Sheet => 1,
        });
    }
}

impl Bits for Body {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let Body {
            kind,
            vertices,
            edges,
            loops,
            faces,
            shells,
            tolerances,
        } = self;
        kind.push_bits(out);
        vertices.push_bits(out);
        edges.push_bits(out);
        loops.push_bits(out);
        faces.push_bits(out);
        shells.push_bits(out);
        tolerances.push_bits(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::{Vec3, bits_of};

    #[test]
    fn bodies_differing_in_the_sign_of_a_zero_differ() {
        let mut a = Body::new(BodyKind::Solid);
        a.add_vertex(Vec3::new(0.0, 1.0, 2.0));
        let mut b = Body::new(BodyKind::Solid);
        b.add_vertex(Vec3::new(-0.0, 1.0, 2.0));
        // Equal as numbers, not bit for bit.
        assert_eq!(a, b);
        assert_ne!(bits_of(&a), bits_of(&b));
        assert_eq!(bits_of(&a), bits_of(&a.clone()));
        let sheet = Body {
            kind: BodyKind::Sheet,
            ..a.clone()
        };
        assert_ne!(bits_of(&a), bits_of(&sheet));
    }
}
