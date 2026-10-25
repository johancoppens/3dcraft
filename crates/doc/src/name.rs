//! Persistent names: how a feature points at a face, edge or vertex that
//! another feature made.
//!
//! A name says where the entity came from, never where it sits in the
//! body: the feature that made it, the part it plays, and the entities of
//! that feature's input it was made from (for an extrusion, the ids of the
//! sketch entities). It is written `F2:side(e7)`. An upstream edit either
//! leaves the name pointing at the same thing, or leaves it pointing at
//! nothing, which regeneration reports; it never makes it point at
//! something else (docs/design/names.md).
//!
//! A feature that makes its body out of another body (a transform, a
//! mirror, a pattern, a boolean, a blend) names what it makes after what it
//! was made from: `F5:copy(2)/F2:side(e7)` is the image, in instance 2 of
//! F5, of the face F2 calls `side(e7)`. The part after `/` is a name of its
//! own.
//!
//! A copy of a copy does not grow a link per feature. A face that a row of
//! features each kept as instance 0 (the target of every cut of a plate,
//! the body every blend rounds) is one link, named after the first
//! feature of the row: `F2001:copy(0)/F1:face(1)` is the top of a plate a
//! thousand cuts went through, the way going on through instance 0 of
//! each body until it reaches F1. Steps in a row that are not instance 0
//! (the tool of each union, an instance of a pattern of a pattern) are one
//! link too, their instances listed: `F9:copy(1,3)/F2:side(e7)` is in
//! instance 1 of F9, which is instance 3 of the body F9's tool was made
//! from, which is F2's. So a name has a link per change between instance 0
//! and other instances along the way, not per feature, and the features in
//! between are the ones the history says each instance was made from.
//! Every name of the old long form (one link per feature) has exactly one
//! short form and the other way round: [`PersistentName::canonical`] makes
//! it, and reading a name (text or file) does so (docs/design/names.md).

use crate::FeatureId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

/// Most sources a name carries (a cap is named after every curve of its
/// outer loop).
pub const MAX_NAME_SOURCES: usize = 4096;

/// Most face names an edge or a vertex named by its faces lists, those
/// after `|` included.
pub const MAX_NAME_FACES: usize = 256;

/// Most links one name is made of (`a/b/c` is three). A name regeneration
/// makes has three at most (a copy, a piece, the name it was first given):
/// a copy of a copy is one link ([`PersistentName::copy`]). The cap is for
/// a name read from text or a file, in its long form or hostile.
pub const MAX_NAME_DEPTH: usize = 32;

/// The largest sketch entity id a name can carry: ids are below 2^32, so
/// that the bits above can say which instance of a sketch pattern a curve
/// is ([`CurveRef`]).
pub const MAX_CURVE_ENTITY: u64 = u32::MAX as u64;

/// The largest instance index a name can carry in `e7[k]`.
pub const MAX_CURVE_INDEX: u32 = 0xFFFD;

/// A curve of a sketch as the source of a name: the seed entity itself
/// (`e7`), an instance of a sketch pattern of it (`e7[3]`, `e7[2,5]` in
/// a grid), or every instance of it at once (`e7[*]`, in the name of a cap
/// whose loop holds a whole run). Kept in a name as one number
/// ([`CurveRef::encode`]): the entity id in the low 32 bits and the
/// instance above it, so a name without patterns has the number it always
/// had, and the kernel, which takes curve ids as numbers, carries
/// instances without knowing about them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurveRef {
    pub entity: u64,
    pub index: CurveIndex,
}

/// Which instance of its seed a curve is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CurveIndex {
    /// The seed entity itself (instance 0, or 0,0 in a grid): `e7`.
    Seed,
    /// Instance `k` of a pattern in one direction: `e7[3]`.
    At(u32),
    /// Instance `i, j` of a grid: `e7[2,5]`.
    Grid(u32, u32),
    /// Every instance: `e7[*]`.
    All,
}

const INDEX_SHIFT: u32 = 32;
const SECOND_SHIFT: u32 = 48;
const FIELD: u64 = 0xFFFF;

impl CurveRef {
    /// The seed entity `entity`.
    pub const fn seed(entity: u64) -> Self {
        Self {
            entity,
            index: CurveIndex::Seed,
        }
    }

    /// The number a name keeps: `entity`, plus `i + 1` from bit 32 and
    /// `j + 1` from bit 48 (`0xFFFF` from bit 32 for every instance).
    /// `None` when the entity or an index is beyond what a name holds.
    pub fn encode(self) -> Option<u64> {
        if self.entity > MAX_CURVE_ENTITY {
            return None;
        }
        let field = |i: u32| (i <= MAX_CURVE_INDEX).then(|| u64::from(i) + 1);
        let (first, second) = match self.index {
            CurveIndex::Seed => (0, 0),
            CurveIndex::At(i) => (field(i)?, 0),
            CurveIndex::Grid(i, j) => (field(i)?, field(j)?),
            CurveIndex::All => (FIELD, 0),
        };
        Some(self.entity | (first << INDEX_SHIFT) | (second << SECOND_SHIFT))
    }

    /// The curve a number of a name stands for; `None` for a number no
    /// curve encodes to.
    pub fn decode(source: u64) -> Option<Self> {
        let entity = source & MAX_CURVE_ENTITY;
        let first = (source >> INDEX_SHIFT) & FIELD;
        let second = (source >> SECOND_SHIFT) & FIELD;
        let index = |field: u64| u32::try_from(field.checked_sub(1)?).ok();
        let index = match (first, second) {
            (0, 0) => CurveIndex::Seed,
            (FIELD, 0) => CurveIndex::All,
            (0 | FIELD, _) => return None,
            (i, 0) => CurveIndex::At(index(i)?),
            (i, j) if j != FIELD => CurveIndex::Grid(index(i)?, index(j)?),
            _ => return None,
        };
        Some(Self { entity, index })
    }

    /// Reads `e7`, `e7[3]`, `e7[2,5]` or `e7[*]`.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = |t: &str| -> Option<u64> {
            (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
                .then(|| t.parse::<u64>().ok())
                .flatten()
        };
        let index = |t: &str| -> Option<u32> {
            digits(t.trim())
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n <= MAX_CURVE_INDEX)
        };
        let rest = text.strip_prefix('e')?;
        let (entity, index) = match rest.split_once('[') {
            None => (rest, CurveIndex::Seed),
            Some((entity, inner)) => {
                let inner = inner.strip_suffix(']')?;
                let index = match inner.split(',').collect::<Vec<_>>().as_slice() {
                    ["*"] => CurveIndex::All,
                    [k] => CurveIndex::At(index(k)?),
                    [i, j] => CurveIndex::Grid(index(i)?, index(j)?),
                    _ => return None,
                };
                (entity, index)
            }
        };
        let entity = digits(entity).filter(|&e| e <= MAX_CURVE_ENTITY)?;
        Some(Self { entity, index })
    }
}

impl fmt::Display for CurveRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.entity)?;
        match self.index {
            CurveIndex::Seed => Ok(()),
            CurveIndex::At(k) => write!(f, "[{k}]"),
            CurveIndex::Grid(i, j) => write!(f, "[{i},{j}]"),
            CurveIndex::All => f.write_str("[*]"),
        }
    }
}

/// Most levels the instance path of a curve has: one per pattern of a
/// tree of sketch patterns, the innermost first
/// ([`crate::sketch::MAX_PATTERN_DEPTH`]).
pub const MAX_CURVE_LEVELS: usize = 3;

/// The tag (bits 48 to 63) of a continuation word for an outer level in
/// one direction, and for a grid. The tags of [`OffsetHead`] are 1 to 4;
/// bits 32 to 47 are 0 in both, a combination no [`CurveRef`] encodes to.
const CONTINUATION_AT: u64 = 5;
const CONTINUATION_GRID: u64 = 6;
/// The low bits of a continuation word for `*` (every instance).
const CONTINUATION_ALL: u64 = 0xFFFF;

/// The instance path of a curve made by nested sketch patterns: the
/// curve with its index in the innermost pattern (`e3[2]`), and its
/// index in each pattern around that one, the innermost first
/// (`e3[2][1]` is instance 2 of the inner pattern in instance 1 of the
/// outer one; `e5[1][2,4]` in cell 2,4 of a grid). A trailing level whose
/// index is 0 is left out, so wrapping a pattern in another renames none
/// of its curves: `e3[2]` is the same curve in cell 0. Kept in a name as
/// the curve's number followed by one continuation word per outer level
/// (`encode`): bits 32 to 47 are 0, the tag 5 (one direction) or 6 (a
/// grid) from bit 48, and `i + 1` (`0xFFFF` for `*`), or `i + 1` and
/// `j + 1` from bit 16, in the low 32 bits. A name without nesting has
/// the numbers it always had.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurvePath {
    pub curve: CurveRef,
    /// The index in each outer pattern, the innermost first: `At`,
    /// `Grid` or `All` (never `Seed`).
    pub outer: Vec<CurveIndex>,
}

/// One continuation word: an outer level's index.
fn continuation(index: CurveIndex) -> Option<u64> {
    let field = |i: u32| (i <= MAX_CURVE_INDEX).then(|| u64::from(i) + 1);
    Some(match index {
        CurveIndex::At(i) => (CONTINUATION_AT << SECOND_SHIFT) | field(i)?,
        CurveIndex::All => (CONTINUATION_AT << SECOND_SHIFT) | CONTINUATION_ALL,
        CurveIndex::Grid(i, j) => {
            (CONTINUATION_GRID << SECOND_SHIFT) | field(i)? | (field(j)? << 16)
        }
        CurveIndex::Seed => return None,
    })
}

/// The outer level a continuation word stands for; `None` for every
/// other number.
pub fn decode_continuation(word: u64) -> Option<CurveIndex> {
    let first = (word >> INDEX_SHIFT) & FIELD;
    let tag = (word >> SECOND_SHIFT) & FIELD;
    if first != 0 {
        return None;
    }
    let low = word & MAX_CURVE_ENTITY;
    let index = |field: u64| {
        u32::try_from(field.checked_sub(1)?)
            .ok()
            .filter(|&i| i <= MAX_CURVE_INDEX)
    };
    match tag {
        CONTINUATION_AT if low == CONTINUATION_ALL => Some(CurveIndex::All),
        CONTINUATION_AT => Some(CurveIndex::At(index(low)?)),
        CONTINUATION_GRID => Some(CurveIndex::Grid(index(low & FIELD)?, index(low >> 16)?)),
        _ => None,
    }
}

/// True for an index that is instance 0 of its level.
fn is_zero(index: CurveIndex) -> bool {
    matches!(
        index,
        CurveIndex::Seed | CurveIndex::At(0) | CurveIndex::Grid(0, 0)
    )
}

impl CurvePath {
    /// A curve that is not nested: `e7`, `e7[3]`.
    pub fn plain(curve: CurveRef) -> Self {
        Self {
            curve,
            outer: Vec::new(),
        }
    }

    /// The path of curve `entity` with an index on every level, the
    /// innermost first (`At(0)` or `Grid(0, 0)` for instance 0, `All` for
    /// every instance), in the one form a name keeps: trailing levels at
    /// instance 0 left out, and `*` on an outer level after `*` on every
    /// level inside it left out too (every instance of every level is
    /// `e3[*]`); with no level left the seed, `e3`.
    pub fn of_levels(entity: u64, levels: &[CurveIndex]) -> Self {
        let mut levels: Vec<CurveIndex> = levels.to_vec();
        loop {
            match levels.as_slice() {
                [.., last] if is_zero(*last) => {
                    levels.pop();
                }
                [.., CurveIndex::All, CurveIndex::All] => {
                    levels.pop();
                }
                _ => break,
            }
        }
        let mut levels = levels.into_iter();
        let first = levels.next().unwrap_or(CurveIndex::Seed);
        Self {
            curve: CurveRef {
                entity,
                index: first,
            },
            outer: levels.collect(),
        }
    }

    /// The index on level `level` (0 the innermost), with the levels a
    /// name leaves out: instance 0, or `*` after a `*`.
    pub fn level(&self, level: usize) -> CurveIndex {
        if level == 0 {
            return self.curve.index;
        }
        match self.outer.get(level - 1) {
            Some(index) => *index,
            None => {
                let all_inside = self.curve.index == CurveIndex::All
                    && self.outer.iter().all(|i| *i == CurveIndex::All);
                if all_inside {
                    CurveIndex::All
                } else {
                    CurveIndex::At(0)
                }
            }
        }
    }

    /// Whether this is the one form [`CurvePath::of_levels`] gives: no
    /// trailing level at instance 0, at most [`MAX_CURVE_LEVELS`] levels,
    /// `*` on an outer level only inside-out, and an inner index written
    /// out when there are outer ones.
    pub fn is_canonical(&self) -> bool {
        if self.outer.is_empty() {
            return true;
        }
        if self.outer.len() >= MAX_CURVE_LEVELS || self.curve.index == CurveIndex::Seed {
            return false;
        }
        let mut all = self.curve.index == CurveIndex::All;
        for index in &self.outer {
            match index {
                CurveIndex::Seed => return false,
                CurveIndex::All if !all => return false,
                CurveIndex::All => {}
                _ => all = false,
            }
        }
        let last = self.outer.last().copied().unwrap_or(CurveIndex::Seed);
        let before = match self.outer.len() {
            1 => self.curve.index,
            n => self.outer.get(n - 2).copied().unwrap_or(CurveIndex::Seed),
        };
        !is_zero(last) && !(last == CurveIndex::All && before == CurveIndex::All)
    }

    /// The numbers a name keeps: the curve's, then a continuation word per
    /// outer level. `None` beyond what a name holds or not canonical.
    pub fn encode(&self) -> Option<Vec<u64>> {
        if !self.is_canonical() {
            return None;
        }
        let mut out = Vec::with_capacity(1 + self.outer.len());
        out.push(self.curve.encode()?);
        for index in &self.outer {
            out.push(continuation(*index)?);
        }
        Some(out)
    }

    /// Reads one curve with its continuation words from the start of
    /// `words`: the path and how many words it took. `None` when the
    /// first word is no curve, or what follows it is not canonical.
    pub fn read(words: &[u64]) -> Option<(Self, usize)> {
        let (&first, rest) = words.split_first()?;
        let curve = CurveRef::decode(first)?;
        let outer: Vec<CurveIndex> = rest
            .iter()
            .map_while(|&w| decode_continuation(w))
            .take(MAX_CURVE_LEVELS)
            .collect();
        let path = Self { curve, outer };
        let taken = 1 + path.outer.len();
        path.is_canonical().then_some((path, taken))
    }

    /// Reads `e3`, `e3[2]`, `e3[2][1]`, `e3[*][1]`, `e5[1][2,4]`.
    pub fn parse(text: &str) -> Option<Self> {
        let (first, rest) = match text.find(']') {
            Some(at) => (text.get(..=at)?, text.get(at + 1..)?),
            None => (text, ""),
        };
        let curve = CurveRef::parse(first)?;
        let mut outer = Vec::new();
        let mut rest = rest;
        while !rest.is_empty() {
            if outer.len() >= MAX_CURVE_LEVELS {
                return None;
            }
            let inner = rest.strip_prefix('[')?;
            let at = inner.find(']')?;
            let (level, after) = (inner.get(..at)?, inner.get(at + 1..)?);
            // An outer level reads as the inner one does.
            let index = CurveRef::parse(&format!("e0[{level}]"))?.index;
            outer.push(index);
            rest = after;
        }
        let path = Self { curve, outer };
        path.is_canonical().then_some(path)
    }
}

impl fmt::Display for CurvePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.curve)?;
        for index in &self.outer {
            match index {
                CurveIndex::Seed => f.write_str("[0]")?,
                CurveIndex::At(k) => write!(f, "[{k}]")?,
                CurveIndex::Grid(i, j) => write!(f, "[{i},{j}]")?,
                CurveIndex::All => f.write_str("[*]")?,
            }
        }
        Ok(())
    }
}

/// The numbers of a name's sources cut into curves: each curve with the
/// continuation words of its outer levels, every other number alone. An
/// error for a continuation word that follows no curve, or a path that is
/// not in the form a name keeps.
pub fn curve_groups(sources: &[u64]) -> Result<Vec<&[u64]>, &'static str> {
    let mut groups = Vec::new();
    let mut rest = sources;
    while !rest.is_empty() {
        let first = rest.first().copied().unwrap_or_default();
        let followed = rest
            .get(1..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&w| decode_continuation(w).is_some())
            .count();
        if decode_continuation(first).is_some() {
            return Err("a continuation word of a nested pattern follows no curve");
        }
        if followed > 0 && CurvePath::read(rest).is_none_or(|(_, n)| n != followed + 1) {
            return Err("the instance path of a curve is not in the form a name keeps");
        }
        let (group, after) = rest.split_at(followed + 1);
        groups.push(group);
        rest = after;
    }
    Ok(groups)
}

/// Sorts the curves of a cap's sources (each with its continuation
/// words) word by word and drops repeats: the order names keep, which for
/// curves that are not nested is the order of their numbers.
#[cfg(test)]
pub(crate) fn sort_curves(sources: &[u64]) -> Vec<u64> {
    let mut groups: Vec<&[u64]> =
        curve_groups(sources).unwrap_or_else(|_| sources.chunks(1).collect());
    groups.sort_unstable();
    groups.dedup();
    groups.concat()
}

/// Writes one source group: a curve (nested or not), or a number no curve
/// encodes to as it is.
fn write_group(f: &mut fmt::Formatter<'_>, group: &[u64]) -> fmt::Result {
    match CurvePath::read(group) {
        Some((path, n)) if n == group.len() => write!(f, "{path}"),
        _ => {
            for (i, word) in group.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "e{word}")?;
            }
            Ok(())
        }
    }
}

/// What a curve an offset feature made is, as the head of its group in
/// the sources of a sweep's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OffsetKind {
    /// `F9:offset(e7)`: the offset of curve `e7`; in the name of a cap
    /// the curves of the loop, `F9:offset(e1,e2,e7[*])`.
    Offset,
    /// `F9:offsetCorner(e7,e8)`: the arc or mitre the offset put at the
    /// corner where `e7` runs into `e8` (in the direction the loop runs).
    Corner,
    /// `F9:offsetExtension(e7,e8)`: the straight piece a sharp corner
    /// carries the offset of the arc `e7` on by, towards its mitre with
    /// `e8`.
    Extension,
    /// `F9:loop(e1,e2,e7)/F9:offset(e7)`: not a curve but what tells
    /// apart the pieces of one curve of an offset (a region that split):
    /// the loop the piece closes, by the curves whose offsets it holds. It
    /// qualifies the group after it.
    Loop,
}

impl OffsetKind {
    pub fn name(self) -> &'static str {
        match self {
            OffsetKind::Offset => "offset",
            OffsetKind::Corner => "offsetCorner",
            OffsetKind::Extension => "offsetExtension",
            OffsetKind::Loop => "loop",
        }
    }

    const ALL: [OffsetKind; 4] = [
        OffsetKind::Offset,
        OffsetKind::Corner,
        OffsetKind::Extension,
        OffsetKind::Loop,
    ];
}

/// The head of a group of an offset in the sources of a sweep's name:
/// `F10:side(F9:offset(e7))`. In `sources` it is one number, followed by
/// the curves of the sketch it names (one or more for
/// [`OffsetKind::Offset`] and [`OffsetKind::Loop`], two for the others).
/// The number has the feature in the low 32 bits, 0 in bits 32 to 47 and
/// the kind from bit 48, which is a number no [`CurveRef`] encodes to, so
/// a name without offsets has the numbers it always had and a head is
/// never read as a curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OffsetHead {
    pub feature: FeatureId,
    pub kind: OffsetKind,
}

impl OffsetHead {
    /// The number of the head; `None` past what a name holds.
    pub fn encode(self) -> Option<u64> {
        if self.feature.0 > MAX_CURVE_ENTITY {
            return None;
        }
        let tag = 1 + OffsetKind::ALL.iter().position(|k| *k == self.kind)? as u64;
        Some(self.feature.0 | (tag << SECOND_SHIFT))
    }

    /// The head a number stands for; `None` for every other number.
    pub fn decode(source: u64) -> Option<Self> {
        let first = (source >> INDEX_SHIFT) & FIELD;
        let tag = (source >> SECOND_SHIFT) & FIELD;
        if first != 0 || tag == 0 {
            return None;
        }
        let kind = *OffsetKind::ALL.get(usize::try_from(tag - 1).ok()?)?;
        Some(Self {
            feature: FeatureId(source & MAX_CURVE_ENTITY),
            kind,
        })
    }
}

/// One curve an offset made, in the sources of a name: its head, the
/// curves of the sketch it names, and the loop that tells it from other
/// pieces of the same curve, when it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffsetGroup<'a> {
    pub head: OffsetHead,
    pub curves: &'a [u64],
    pub within: Option<&'a [u64]>,
}

/// The sources of a sweep's name cut into the curves an offset feature
/// made. `Ok(None)` when there is no head (the sources are curves of a
/// sketch, as always); an error when heads and curves do not go together.
pub fn offset_groups(sources: &[u64]) -> Result<Option<Vec<OffsetGroup<'_>>>, &'static str> {
    if !sources.iter().any(|&s| OffsetHead::decode(s).is_some()) {
        return Ok(None);
    }
    let mut groups = Vec::new();
    let mut within: Option<(FeatureId, &[u64])> = None;
    let mut rest = sources;
    while let Some((&first, after)) = rest.split_first() {
        let head = OffsetHead::decode(first)
            .ok_or("a name with offset curves has an offset head before every curve")?;
        let count = after
            .iter()
            .position(|&s| OffsetHead::decode(s).is_some())
            .unwrap_or(after.len());
        let (curves, next) = after.split_at(count);
        let named = curve_groups(curves)?;
        if named
            .iter()
            .any(|g| CurvePath::read(g).is_none_or(|(_, n)| n != g.len()))
        {
            return Err("an offset names curves of a sketch");
        }
        match head.kind {
            OffsetKind::Offset | OffsetKind::Loop if named.is_empty() => {
                return Err("an offset or a loop names at least one curve");
            }
            OffsetKind::Corner | OffsetKind::Extension if named.len() != 2 => {
                return Err("an offset corner or extension names two curves");
            }
            _ => {}
        }
        if head.kind == OffsetKind::Loop {
            if within.is_some() {
                return Err("a loop qualifies one offset curve, not another loop");
            }
            within = Some((head.feature, curves));
        } else {
            let qualifier = within.take();
            if qualifier.is_some_and(|(feature, _)| feature != head.feature) {
                return Err("a loop qualifies an offset curve of its own feature");
            }
            groups.push(OffsetGroup {
                head,
                curves,
                within: qualifier.map(|(_, curves)| curves),
            });
        }
        rest = next;
    }
    if within.is_some() {
        return Err("a loop qualifies the offset curve after it");
    }
    Ok(Some(groups))
}

fn write_curves(f: &mut fmt::Formatter<'_>, curves: &[u64]) -> fmt::Result {
    let groups = curve_groups(curves).unwrap_or_else(|_| curves.chunks(1).collect());
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            f.write_str(",")?;
        }
        write_group(f, group)?;
    }
    Ok(())
}

/// Writes one offset curve: `F9:offset(e7)`,
/// `F9:loop(e1,e2,e7)/F9:offsetCorner(e7,e8)`.
fn write_offset(f: &mut fmt::Formatter<'_>, group: &OffsetGroup<'_>) -> fmt::Result {
    let feature = group.head.feature;
    if let Some(within) = group.within {
        write!(f, "{feature}:loop(")?;
        write_curves(f, within)?;
        f.write_str(")/")?;
    }
    write!(f, "{feature}:{}(", group.head.kind.name())?;
    write_curves(f, group.curves)?;
    f.write_str(")")
}

/// Reads one offset curve as [`write_offset`] writes it: its heads and
/// curves, as numbers.
fn parse_offset(text: &str, reading: &Reading<'_>) -> Option<Vec<u64>> {
    let link = |t: &str| -> Option<(u64, OffsetKind, Vec<u64>)> {
        let (feature, rest) = split_feature(t)?;
        let feature = feature_of(feature, reading).ok()?.0;
        let at = rest.find('(')?;
        let (role, inner) = rest.split_at(at);
        let kind = *OffsetKind::ALL.iter().find(|k| k.name() == role)?;
        let inner = inner.strip_prefix('(')?.strip_suffix(')')?;
        let curves = split_outside(inner, ',')
            .into_iter()
            .map(|part| CurvePath::parse(part.trim()).and_then(|c| c.encode()))
            .collect::<Option<Vec<Vec<u64>>>>()?
            .concat();
        Some((feature, kind, curves))
    };
    let mut out = Vec::new();
    let parts = split_outside(text, '/');
    let links = match parts.as_slice() {
        [one] => vec![link(one)?],
        [loop_, one] => {
            let first = link(loop_)?;
            let second = link(one)?;
            if first.1 != OffsetKind::Loop || second.1 == OffsetKind::Loop {
                return None;
            }
            vec![first, second]
        }
        _ => return None,
    };
    for (feature, kind, curves) in links {
        out.push(
            OffsetHead {
                feature: FeatureId(feature),
                kind,
            }
            .encode()?,
        );
        out.extend(curves);
    }
    let groups = offset_groups(&out).ok().flatten()?;
    (groups.len() == 1).then_some(out)
}

/// The part an entity plays in the feature that made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameRole {
    /// The flat face where an extrusion starts.
    StartCap,
    /// The flat face where it ends.
    EndCap,
    /// The face swept by one curve.
    Side,
    /// The edge of the start cap made from one curve.
    StartEdge,
    EndEdge,
    /// The straight edge swept by a corner, between two curves.
    LateralEdge,
    StartVertex,
    EndVertex,
    /// The image of an entity of another body, in a feature that moves,
    /// mirrors or repeats that body, or combines it with others. Its
    /// sources are the instances along the way: either the one number 0,
    /// when the entity is in instance 0 (for a boolean the target) of this
    /// feature, and of the body that was made from, and so on until the
    /// way reaches the feature of the name after `/`; or numbers none of
    /// which is 0: the instance in this feature (for a boolean 1 is the
    /// tool), then the instance in the body that instance was made from,
    /// and so on, the last of them made from a body that goes on through
    /// instance 0 (none, one or several features) to the body the name
    /// after `/` is of, when that name is no copy (2026-10-21: so every
    /// instance of a pattern names the faces of its seed alike, as
    /// instance 0 does, whatever took the seed's body), and directly from
    /// that body when it is a copy. The name after `/` is the name of the
    /// entity it is the image of there.
    Copy,
    /// One of several entities that the name after `/` would otherwise
    /// name all at once: a curve that sweeps more than one face (a spline
    /// cut at its corners, a spline that closes on itself, a spline turned
    /// a full turn, which is made in two halves) calls the first
    /// `F2:side(e7)` and the others `F2:piece(1)/F2:side(e7)`, and so on;
    /// their edges and vertices likewise. Its one source is the number,
    /// from 1; the feature is the same before and after `/`.
    Piece,
    /// A face of a primitive, by its number: the faces of a primitive are
    /// made in a fixed order (for a block: 0 bottom, 1 top, 2 front (-y),
    /// 3 back (+y), 4 left (-x), 5 right (+x), in its own axes; for a
    /// cylinder: 0 the wall, 1 the bottom, 2 the top). Its one source is
    /// that number.
    Face,
    /// An edge of a primitive, by its number.
    Edge,
    /// A vertex of a primitive, by its number.
    Vertex,
    /// An edge, by the faces of its body it lies between (twice the same
    /// face for a seam). The faces are in [`PersistentName::faces`].
    Between,
    /// A vertex, by the faces of its body that meet in it.
    Corner,
    /// The face a chamfer puts in the place of an edge. Its sources are
    /// plain numbers: the place of the edge's reference in the chamfer's
    /// list of edges, and, when that reference gave several edges, which
    /// of them, counted in the order of where they lie in space.
    Chamfer,
    /// The flat face in the corner where three chamfered edges meet. Its
    /// sources are plain numbers: the places of the three references in
    /// the chamfer's list, ascending.
    ChamferCorner,
    /// The face a fillet puts in the place of an edge: a piece of a
    /// cylinder or of a torus. Its sources are as a chamfer face's.
    Fillet,
    /// The piece of a ball in the corner where three filleted edges meet.
    /// Its sources are as a chamfer corner's.
    FilletCorner,
    /// A face of the groove a thread cuts. Its one source is a plain
    /// number: 0 the root, 1 the flank that faces the start of the
    /// thread, 2 the flank that faces away from it; the faces of the
    /// groove are one per half turn, numbered as pieces
    /// (`F9:piece(3)/F9:thread(1)`).
    Thread,
}

impl NameRole {
    pub const ALL: [NameRole; 20] = [
        NameRole::StartCap,
        NameRole::EndCap,
        NameRole::Side,
        NameRole::StartEdge,
        NameRole::EndEdge,
        NameRole::LateralEdge,
        NameRole::StartVertex,
        NameRole::EndVertex,
        NameRole::Copy,
        NameRole::Piece,
        NameRole::Face,
        NameRole::Edge,
        NameRole::Vertex,
        NameRole::Between,
        NameRole::Corner,
        NameRole::Chamfer,
        NameRole::ChamferCorner,
        NameRole::Fillet,
        NameRole::FilletCorner,
        NameRole::Thread,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NameRole::StartCap => "startCap",
            NameRole::EndCap => "endCap",
            NameRole::Side => "side",
            NameRole::StartEdge => "startEdge",
            NameRole::EndEdge => "endEdge",
            NameRole::LateralEdge => "lateralEdge",
            NameRole::StartVertex => "startVertex",
            NameRole::EndVertex => "endVertex",
            NameRole::Copy => "copy",
            NameRole::Piece => "piece",
            NameRole::Face => "face",
            NameRole::Edge => "edge",
            NameRole::Vertex => "vertex",
            NameRole::Between => "between",
            NameRole::Corner => "corner",
            NameRole::Chamfer => "chamfer",
            NameRole::ChamferCorner => "chamferCorner",
            NameRole::Fillet => "fillet",
            NameRole::FilletCorner => "filletCorner",
            NameRole::Thread => "thread",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    /// True for the roles that are faces. A copy is whatever it is a copy
    /// of: ask [`PersistentName::is_face`].
    pub fn is_face(self) -> bool {
        matches!(
            self,
            NameRole::StartCap
                | NameRole::EndCap
                | NameRole::Side
                | NameRole::Face
                | NameRole::Chamfer
                | NameRole::ChamferCorner
                | NameRole::Fillet
                | NameRole::FilletCorner
                | NameRole::Thread
        )
    }

    /// True for the roles that are vertices. A copy is whatever it is a
    /// copy of: ask [`PersistentName::is_vertex`].
    pub fn is_vertex(self) -> bool {
        matches!(
            self,
            NameRole::StartVertex | NameRole::EndVertex | NameRole::Vertex | NameRole::Corner
        )
    }

    /// True for the roles that are edges. A copy is whatever it is a copy
    /// of: ask [`PersistentName::is_edge`].
    pub fn is_edge(self) -> bool {
        matches!(
            self,
            NameRole::StartEdge
                | NameRole::EndEdge
                | NameRole::LateralEdge
                | NameRole::Edge
                | NameRole::Between
        )
    }

    /// True for a role that names an entity by the faces it lies on, which
    /// are written between `[` and `]`.
    pub fn is_by_faces(self) -> bool {
        matches!(self, NameRole::Between | NameRole::Corner)
    }

    /// True for a role whose name continues after `/` with the name of the
    /// entity it was made from.
    pub fn is_derived(self) -> bool {
        matches!(self, NameRole::Copy | NameRole::Piece)
    }

    /// True for a role whose sources are plain numbers, one or more,
    /// written without the `e` of a sketch entity: `chamfer(0,1)`,
    /// `copy(0,1)`.
    pub fn is_counted(self) -> bool {
        matches!(
            self,
            NameRole::Copy
                | NameRole::Chamfer
                | NameRole::ChamferCorner
                | NameRole::Fillet
                | NameRole::FilletCorner
        )
    }

    /// True for a role with exactly one source that is a plain number (a
    /// piece, or the number of an entity of a primitive) and is written
    /// without the `e` of a sketch entity.
    pub fn is_numbered(self) -> bool {
        matches!(
            self,
            NameRole::Piece | NameRole::Face | NameRole::Edge | NameRole::Vertex | NameRole::Thread
        )
    }
}

/// The name of one face, edge or vertex. As text: `F2:side(e7)`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PersistentName {
    /// The feature that made the entity.
    pub feature: FeatureId,
    pub role: NameRole,
    /// The ids of the input entities it was made from, in the order the
    /// role defines. For a copy: the number of the instance. For an edge
    /// or a vertex named by its faces: empty, or the one number that tells
    /// it from the others on the same faces.
    pub sources: Vec<u64>,
    /// For a derived role (a copy, a piece): the name of the entity this
    /// one is the image of, in the body the way of the copy ends in (see
    /// [`NameRole::Copy`]). `None` for every other role.
    pub of: Option<Arc<PersistentName>>,
    /// For an edge or a vertex named by its faces (`between`, `corner`)
    /// that lists any: the faces. `None` for every other role. Behind a
    /// pointer, because most names are of faces and there are many.
    pub by: Option<Box<FaceLists>>,
}

/// The faces an edge or a vertex named by its faces lists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FaceLists {
    /// The names of the faces of the feature's body the entity lies on,
    /// in ascending order (see [`PersistentName::with_faces`]). A seam
    /// has its one face twice.
    pub on: Vec<PersistentName>,
    /// When several entities lie on the same faces: the names of the
    /// other faces near it (at the ends of the edge, at the other ends
    /// of the vertex's edges, or further away), in ascending order.
    pub near: Vec<PersistentName>,
}

/// The name an `Arc` holds, taken out of it when nothing else holds it
/// and cloned otherwise.
fn unshared(name: Arc<PersistentName>) -> PersistentName {
    Arc::try_unwrap(name).unwrap_or_else(|shared| (*shared).clone())
}

impl PersistentName {
    /// A name that is not derived from another: `F2:side(e7)`.
    pub fn new(feature: FeatureId, role: NameRole, sources: Vec<u64>) -> Self {
        Self {
            feature,
            role,
            sources,
            of: None,
            by: None,
        }
    }

    /// The image of `of` in instance `instance` of `feature`:
    /// `F5:copy(2)/F2:side(e7)`. `of` is a name of the body that instance
    /// was made from. When `of` is a copy that goes the same way (instance
    /// 0 and instance 0, or two that are not 0) the two are one link:
    /// `F9:copy(0)` of `F7:copy(0)/F2:side(e7)` is `F9:copy(0)/F2:side(e7)`,
    /// `F9:copy(1)` of `F7:copy(3)/F2:side(e7)` is `F9:copy(1,3)/F2:side(e7)`
    /// (see [`NameRole::Copy`]). So a name grows by a link only where the
    /// way changes between instance 0 and others, not per feature.
    pub fn copy(feature: FeatureId, instance: u64, of: PersistentName) -> Self {
        Self::copy_along(feature, vec![instance], of)
    }

    /// The copy in `feature` whose way starts with the instances `path`,
    /// directly followed by `of` (a name of the body the last of them was
    /// made from): one link with `of` when they go the same way.
    fn copy_along(feature: FeatureId, path: Vec<u64>, of: PersistentName) -> Self {
        let zero = |p: &[u64]| p == [0];
        let none_zero = |p: &[u64]| !p.is_empty() && p.iter().all(|&i| i != 0);
        let (sources, of) = match of {
            PersistentName {
                role: NameRole::Copy,
                sources: inner,
                of: Some(next),
                by: None,
                feature: inner_feature,
            } => {
                if zero(&path) && zero(&inner) {
                    // Instance 0 on and on: the way through instance 0
                    // starts here now.
                    (path, next)
                } else if none_zero(&path)
                    && zero(&inner)
                    && !(next.role == NameRole::Copy && next.by.is_none())
                {
                    // Other instances, then instance 0 on and on down to a
                    // name that is no copy: the run of instance 0 is left
                    // out as it is in instance 0 itself, so every instance
                    // of a pattern names a face of its seed alike
                    // (`F4:copy(1)/F2:face(2)` as `F4:copy(0)/F2:face(2)`,
                    // whatever took the seed's body between F2 and F4).
                    // Before a copy that is not instance 0 the run stays
                    // a link: the instances would otherwise read as one
                    // way.
                    (path, next)
                } else if none_zero(&path) && none_zero(&inner) {
                    let mut sources = path;
                    sources.extend(inner);
                    (sources, next)
                } else {
                    let inner = PersistentName {
                        role: NameRole::Copy,
                        sources: inner,
                        of: Some(next),
                        by: None,
                        feature: inner_feature,
                    };
                    (path, Arc::new(inner))
                }
            }
            of => (path, Arc::new(of)),
        };
        Self {
            of: Some(of),
            ..Self::new(feature, NameRole::Copy, sources)
        }
    }

    /// The same name in its short form: copies of copies that go the same
    /// way one link (see [`PersistentName::copy`]), and the faces an edge
    /// or a vertex lists the same way, in the order names are compared in.
    /// A name regeneration makes is in this form already; one read in the
    /// long form (`F9:copy(0)/F7:copy(0)/F2:side(e7)`, as files and names
    /// written before format version 15 have it) means the same entity in
    /// it (`F9:copy(0)/F2:side(e7)`). In a short name two links in a row
    /// never go the same way, so a name in either form reads as one
    /// entity.
    pub fn canonical(self) -> Self {
        self.canonical_with(true)
    }

    /// [`PersistentName::canonical`]; the faces a link lists only when
    /// `lists` (they list none themselves, so this goes one list deep).
    fn canonical_with(self, lists: bool) -> Self {
        // In a loop, from the innermost link out: a hostile chain cannot
        // overflow the stack.
        let mut links = Vec::new();
        let mut next = Some(self);
        while let Some(mut link) = next {
            next = link.of.take().map(unshared);
            links.push(link);
        }
        let mut built: Option<PersistentName> = None;
        while let Some(mut link) = links.pop() {
            if lists && let Some(by) = link.by.take() {
                let FaceLists { on, near } = *by;
                let short = |names: Vec<PersistentName>| -> Vec<PersistentName> {
                    names.into_iter().map(|n| n.canonical_with(false)).collect()
                };
                link = link.with_faces(short(on), short(near));
            }
            link = match built.take() {
                Some(of) if link.role == NameRole::Copy && link.by.is_none() => {
                    Self::copy_along(link.feature, link.sources, of)
                }
                of => {
                    link.of = of.map(Arc::new);
                    link
                }
            };
            built = Some(link);
        }
        // There is always at least one link; the fallback is never used.
        built.unwrap_or_else(|| Self::new(FeatureId(0), NameRole::Side, Vec::new()))
    }

    /// Entity number `piece` of those `of` would name in its own feature
    /// (see [`NameRole::Piece`]); `of` itself for number 0.
    pub fn piece(piece: u32, of: PersistentName) -> Self {
        if piece == 0 {
            return of;
        }
        let feature = of.feature;
        Self {
            of: Some(Arc::new(of)),
            ..Self::new(feature, NameRole::Piece, vec![u64::from(piece)])
        }
    }

    /// The edge of `feature`'s body between `faces` (two names; a seam has
    /// its face twice): `F7:between[a,b]`. `near` and `index` are what
    /// tells it from other edges between the same faces, when there are.
    pub fn between(
        feature: FeatureId,
        faces: Vec<PersistentName>,
        near: Vec<PersistentName>,
        index: Option<u64>,
    ) -> Self {
        Self::new(feature, NameRole::Between, index.into_iter().collect()).with_faces(faces, near)
    }

    /// The vertex of `feature`'s body where `faces` meet:
    /// `F7:corner[a,b,c]`. `near` and `index` are what tells it from
    /// other vertices on the same faces, when there are.
    pub fn corner(
        feature: FeatureId,
        faces: Vec<PersistentName>,
        near: Vec<PersistentName>,
        index: Option<u64>,
    ) -> Self {
        Self::new(feature, NameRole::Corner, index.into_iter().collect()).with_faces(faces, near)
    }

    /// This link listing `on` and `near`, each put in the order names
    /// are compared in (by feature, then role, then sources, then what a
    /// copy is a copy of), which is the order regeneration lists them in:
    /// a name typed or stored with its faces in another order is the
    /// same name. Listing nothing is listing no lists.
    pub fn with_faces(
        mut self,
        mut on: Vec<PersistentName>,
        mut near: Vec<PersistentName>,
    ) -> Self {
        on.sort();
        near.sort();
        self.by = if on.is_empty() && near.is_empty() {
            None
        } else {
            Some(Box::new(FaceLists { on, near }))
        };
        self
    }

    /// The faces this link lies on, when it is an edge or a vertex named
    /// by its faces.
    pub fn faces(&self) -> &[PersistentName] {
        self.by.as_ref().map_or(&[], |by| by.on.as_slice())
    }

    /// The faces near this link that tell it from others on the same
    /// faces.
    pub fn near(&self) -> &[PersistentName] {
        self.by.as_ref().map_or(&[], |by| by.near.as_slice())
    }

    /// True for the short form of a cap: a start or end cap named without
    /// the outline it was made from, `F2:startCap()`, or a copy of one
    /// (`F7:copy(0)/F2:endCap()`). No feature names a face so (a cap is
    /// named after the curves of its loop), so the form is free to mean
    /// "the cap of that role of that feature" ([`Self::shortens`]).
    pub fn is_short_cap(&self) -> bool {
        let root = self.root();
        matches!(root.role, NameRole::StartCap | NameRole::EndCap) && root.sources.is_empty()
    }

    /// True when this name is the short form of a cap ([`Self::is_short_cap`])
    /// and `full` is a cap of the same role of the same feature, by way of
    /// the same copies: `F2:startCap()` shortens `F2:startCap(e5,e6,e7)`,
    /// `F7:copy(0)/F2:endCap()` shortens `F7:copy(0)/F2:endCap(e1,e2,e3)`.
    /// The one place that says what the short form means: every lookup of
    /// a face by its name goes through it.
    pub fn shortens(&self, full: &PersistentName) -> bool {
        if !self.is_short_cap() {
            return false;
        }
        let (mut short, mut long) = (self, full);
        for _ in 0..MAX_NAME_DEPTH {
            if (short.feature, short.role) != (long.feature, long.role)
                || short.by.is_some()
                || long.by.is_some()
            {
                return false;
            }
            match (&short.of, &long.of) {
                (Some(next_short), Some(next_long)) => {
                    if short.sources != long.sources {
                        return false;
                    }
                    (short, long) = (next_short, next_long);
                }
                // The caps: the short one has no outline, the full one
                // has one.
                (None, None) => return !long.sources.is_empty(),
                _ => return false,
            }
        }
        false
    }

    /// The name at the end of the chain: what the entity was when it was
    /// first made.
    pub fn root(&self) -> &PersistentName {
        let mut name = self;
        // The chain is finite; the cap is for a name built by hand.
        for _ in 0..MAX_NAME_DEPTH {
            match &name.of {
                Some(next) => name = next,
                None => break,
            }
        }
        name
    }

    /// How many names follow each other in this one, without the faces a
    /// link lists.
    fn chain(&self) -> usize {
        let mut depth = 1;
        let mut name = self;
        while let Some(next) = &name.of {
            depth += 1;
            name = next;
            if depth > MAX_NAME_DEPTH {
                break;
            }
        }
        depth
    }

    /// How many names this one is made of: how many features in a row
    /// each made their body from the one before. An edge or a vertex
    /// named by its faces is as deep as the deepest of them: it is of the
    /// same body.
    pub fn depth(&self) -> usize {
        let listed = self
            .root()
            .listed()
            .map(PersistentName::chain)
            .max()
            .unwrap_or(1);
        self.chain().saturating_sub(1).saturating_add(listed.max(1))
    }

    /// The faces this link lists: those it lies on, then those near it.
    fn listed(&self) -> impl Iterator<Item = &PersistentName> {
        self.faces().iter().chain(self.near())
    }

    /// True when the entity is a face.
    pub fn is_face(&self) -> bool {
        self.root().role.is_face()
    }

    /// True when the entity is an edge.
    pub fn is_edge(&self) -> bool {
        self.root().role.is_edge()
    }

    /// True when the entity is a vertex.
    pub fn is_vertex(&self) -> bool {
        self.root().role.is_vertex()
    }

    /// Whether the name is one that can be written and read back: a copy
    /// has one or more instance numbers and the name it is a copy of, a
    /// piece one number and the name it is a piece of, nothing else
    /// is followed by another name, only an edge or a vertex named by its
    /// faces lists faces (plain face names, which list none themselves),
    /// and the caps on depth, sources and faces hold. A name read from
    /// text always is; one built from parts (a file) may not be.
    pub fn check(&self) -> Result<(), &'static str> {
        let mut sources = 0usize;
        let root = self.check_chain(&mut sources, true)?;
        if root.listed().count() > MAX_NAME_FACES {
            return Err("it lists too many faces");
        }
        for face in root.listed() {
            let end = face.check_chain(&mut sources, false)?;
            if !end.role.is_face() {
                return Err("an edge or a vertex lists faces, and this is not one");
            }
        }
        if self.depth() > MAX_NAME_DEPTH {
            return Err("it is made of too many names");
        }
        Ok(())
    }

    /// Checks the links of one chain and returns its last. `sources` is
    /// what the whole name has used of [`MAX_NAME_SOURCES`]; `lists` says
    /// whether the last link may list faces.
    fn check_chain(
        &self,
        sources: &mut usize,
        lists: bool,
    ) -> Result<&PersistentName, &'static str> {
        let mut name = self;
        for _ in 0..MAX_NAME_DEPTH {
            *sources = sources.saturating_add(name.sources.len());
            if *sources > MAX_NAME_SOURCES {
                return Err("it has too many sources");
            }
            if name.role.is_by_faces() {
                if !lists {
                    return Err("a face listed by an edge or a vertex lists no faces itself");
                }
                if name.sources.len() > 1 {
                    return Err("an edge or a vertex named by its faces has at most one number");
                }
            } else {
                if name.by.is_some() {
                    return Err("only a between and a corner list faces");
                }
                if name.role.is_numbered() && name.sources.len() != 1 {
                    return Err("a piece, face, edge or vertex has one number");
                }
                if name.role == NameRole::Copy
                    && name.sources.len() > 1
                    && name.sources.contains(&0)
                {
                    return Err("a copy's numbers are one 0, or numbers none of which is 0");
                }
                if name.role.is_counted() && name.sources.is_empty() {
                    return Err("a copy, a chamfer or a fillet face has at least one number");
                }
                if !(name.role.is_counted() || name.role.is_numbered())
                    && offset_groups(&name.sources)?.is_none()
                {
                    curve_groups(&name.sources)?;
                }
            }
            match (name.role.is_derived(), &name.of) {
                (true, Some(next)) => name = next,
                (true, None) => return Err("a copy without the name it is a copy of"),
                (false, Some(_)) => return Err("only a copy is followed by another name"),
                (false, None) => return Ok(name),
            }
        }
        Err("it is made of too many names")
    }

    /// Writes the chain; a link that lists faces writes them when `lists`
    /// (the faces themselves are written without, so this never goes
    /// deeper than one list).
    fn write(&self, f: &mut fmt::Formatter<'_>, lists: bool) -> fmt::Result {
        let mut name = self;
        for depth in 0..=MAX_NAME_DEPTH {
            if depth > 0 {
                f.write_str("/")?;
            }
            write!(f, "{}:{}", name.feature, name.role.name())?;
            if name.role.is_by_faces() {
                f.write_str("[")?;
                if lists {
                    for (i, face) in name.faces().iter().enumerate() {
                        if i > 0 {
                            f.write_str(",")?;
                        }
                        face.write(f, false)?;
                    }
                    for (i, face) in name.near().iter().enumerate() {
                        f.write_str(if i > 0 { "," } else { "|" })?;
                        face.write(f, false)?;
                    }
                }
                f.write_str("]")?;
            }
            if !(name.role.is_by_faces() && name.sources.is_empty()) {
                f.write_str("(")?;
                let prefix =
                    if name.role.is_numbered() || name.role.is_by_faces() || name.role.is_counted()
                    {
                        ""
                    } else {
                        "e"
                    };
                let groups = if prefix.is_empty() {
                    None
                } else {
                    offset_groups(&name.sources).ok().flatten()
                };
                if let Some(groups) = groups {
                    for (i, group) in groups.iter().enumerate() {
                        if i > 0 {
                            f.write_str(",")?;
                        }
                        write_offset(f, group)?;
                    }
                    f.write_str(")")?;
                    match &name.of {
                        Some(next) => name = next,
                        None => break,
                    }
                    continue;
                }
                if prefix.is_empty() {
                    for (i, source) in name.sources.iter().enumerate() {
                        if i > 0 {
                            f.write_str(",")?;
                        }
                        write!(f, "{source}")?;
                    }
                } else {
                    // A curve of a sketch, possibly an instance of a
                    // pattern of it (nested patterns: with the indices of
                    // the outer ones); a number no curve encodes to (only
                    // a name built by hand has one) as it is.
                    write_curves(f, &name.sources)?;
                }
                f.write_str(")")?;
            }
            match &name.of {
                Some(next) => name = next,
                None => break,
            }
        }
        Ok(())
    }
}

impl fmt::Display for PersistentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, true)
    }
}

impl From<PersistentName> for String {
    fn from(name: PersistentName) -> String {
        name.to_string()
    }
}

/// Cuts `text` at every `separator` that is outside brackets of both
/// kinds. Brackets that do not match leave the rest in one piece, which
/// then fails to read as a name.
fn split_outside(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut from) = (0usize, 0usize);
    for (at, c) in text.char_indices() {
        match c {
            '(' | '[' => depth = depth.saturating_add(1),
            ')' | ']' => depth = depth.saturating_sub(1),
            c if c == separator && depth == 0 => {
                parts.push(text.get(from..at).unwrap_or_default());
                from = at.saturating_add(c.len_utf8());
            }
            _ => {}
        }
    }
    parts.push(text.get(from..).unwrap_or_default());
    parts
}

/// `text` cut at the first `close` outside brackets: what is before it,
/// and what is after it.
fn closing(text: &str, close: char) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (at, c) in text.char_indices() {
        match c {
            c if c == close && depth == 0 => {
                return Some((
                    text.get(..at)?,
                    text.get(at.saturating_add(c.len_utf8())..)?,
                ));
            }
            '(' | '[' => depth = depth.saturating_add(1),
            ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// What a name is read with: how to word an error, and how many sources
/// and listed faces the whole name may still have.
struct Reading<'a> {
    wrong: &'a dyn Fn(&str) -> String,
    sources: usize,
    faces: usize,
    /// How a feature written by its name (`Plate:face(1)`) is found: the
    /// command's document resolves it to the id. `None` for a name read
    /// from a file, which has ids only.
    features: Option<&'a dyn Fn(&str) -> Result<FeatureId, String>>,
}

/// The feature at the start of a name: `F3`, or with a resolver a
/// feature's name.
fn feature_of(text: &str, reading: &Reading<'_>) -> Result<FeatureId, String> {
    let wrong = reading.wrong;
    if let Some(digits) = text.strip_prefix('F')
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        return digits
            .parse::<u64>()
            .map(FeatureId)
            .map_err(|_| wrong("the feature must be written F and a number"));
    }
    match reading.features {
        Some(resolve) if !text.is_empty() => resolve(text).map_err(|reason| wrong(&reason)),
        _ => Err(wrong("the feature must be written F and a number")),
    }
}

/// `text` cut at the `:` between the feature and the role: the last `:`
/// before the first bracket, so that a feature's name may hold a `:`.
fn split_feature(text: &str) -> Option<(&str, &str)> {
    let head = text.find(['(', '[']).unwrap_or(text.len());
    let at = text.get(..head)?.rfind(':')?;
    Some((text.get(..at)?, text.get(at.saturating_add(1)..)?))
}

/// The numbers between the brackets of `F2:side(e7,e8)`.
fn parse_sources(
    inner: &str,
    numbered: bool,
    reading: &mut Reading<'_>,
) -> Result<Vec<u64>, String> {
    let wrong = reading.wrong;
    let mut sources = Vec::new();
    if inner.is_empty() {
        return Ok(sources);
    }
    let (mut offsets, mut parts) = (0usize, 0usize);
    for part in split_outside(inner, ',') {
        if reading.sources == 0 {
            return Err(wrong("it has too many sources"));
        }
        reading.sources -= 1;
        parts += 1;
        let part = part.trim();
        if !numbered && (part.starts_with('F') || part.contains(':')) {
            offsets += 1;
            let numbers = parse_offset(part, reading).ok_or_else(|| {
                wrong(
                    "an offset curve is written F9:offset(e7), F9:offsetCorner(e7,e8), F9:offsetExtension(e7,e8) or F9:loop(e1,e7)/F9:offset(e7)",
                )
            })?;
            reading.sources = reading
                .sources
                .checked_sub(numbers.len().saturating_sub(1))
                .ok_or_else(|| wrong("it has too many sources"))?;
            sources.extend(numbers);
            continue;
        }
        let id = if numbered {
            part.parse::<u64>()
                .ok()
                .filter(|_| part.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| wrong("this role takes a plain number, as in copy(0)"))?
        } else {
            let words = CurvePath::parse(part)
                .and_then(|c| c.encode())
                .ok_or_else(|| {
                    wrong(
                        "a source must be written e and a number below 4294967296, with an instance of a pattern after it as in e7[3], e7[2,5] or e7[*], and of each pattern around that one after it, the innermost first, as in e7[3][1]",
                    )
                })?;
            reading.sources = reading
                .sources
                .checked_sub(words.len().saturating_sub(1))
                .ok_or_else(|| wrong("it has too many sources"))?;
            sources.extend(words);
            continue;
        };
        sources.push(id);
    }
    if !numbered && offsets > 0 && (offsets < parts || offset_groups(&sources).is_err()) {
        return Err(wrong(
            "the sources are all curves of a sketch or all offset curves, not both",
        ));
    }
    Ok(sources)
}

/// The face names of one list of `F7:corner[a,b|c]`.
fn parse_faces(inner: &str, reading: &mut Reading<'_>) -> Result<Vec<PersistentName>, String> {
    let wrong = reading.wrong;
    let mut faces = Vec::new();
    if inner.trim().is_empty() {
        return Ok(faces);
    }
    for part in split_outside(inner, ',') {
        if reading.faces == 0 {
            return Err(wrong("it lists too many faces"));
        }
        reading.faces -= 1;
        let face = parse_chain(part.trim(), reading, false)?;
        if !face.is_face() {
            return Err(wrong(
                "an edge or a vertex lists the faces it lies on, and this is not a face",
            ));
        }
        faces.push(face);
    }
    Ok(faces)
}

/// One name of a chain: `F2:side(e7)`, or with `lists`
/// `F7:corner[a,b|c](1)`.
fn parse_link(
    text: &str,
    reading: &mut Reading<'_>,
    lists: bool,
) -> Result<PersistentName, String> {
    let wrong = reading.wrong;
    let (feature, rest) =
        split_feature(text).ok_or_else(|| wrong("there is no `:` after the feature"))?;
    let FeatureId(feature) = feature_of(feature, reading)?;
    // A cap without brackets is its short form: `F2:startCap` is
    // `F2:startCap()`, the one cap of that role.
    if let Some(role @ (NameRole::StartCap | NameRole::EndCap)) = NameRole::from_name(rest) {
        return Ok(PersistentName::new(FeatureId(feature), role, Vec::new()));
    }
    let at = rest
        .find(['(', '['])
        .ok_or_else(|| wrong("there is no `(` after the role"))?;
    let (role, rest) = rest.split_at(at);
    let role = NameRole::from_name(role).ok_or_else(|| {
        let roles: Vec<&str> = NameRole::ALL.iter().map(|r| r.name()).collect();
        wrong(&format!("the role must be one of {}", roles.join(", ")))
    })?;
    if role.is_by_faces() {
        if !lists {
            return Err(wrong(
                "an edge or a vertex lists faces, which list no faces themselves",
            ));
        }
        let inner = rest
            .strip_prefix('[')
            .ok_or_else(|| wrong("a between or a corner is followed by `[` and its faces"))?;
        // The `]` that closes the list: the faces may have brackets of
        // their own (`side(e7[3])`).
        let (inner, rest) =
            closing(inner, ']').ok_or_else(|| wrong("there is no `]` after the faces"))?;
        let (on, near) = match split_outside(inner, '|').as_slice() {
            [on] => (*on, ""),
            [on, near] => (*on, *near),
            _ => return Err(wrong("there is one `|` at most between the faces")),
        };
        let faces = parse_faces(on, reading)?;
        let near = parse_faces(near, reading)?;
        let sources = match rest {
            "" => Vec::new(),
            _ => {
                let inner = rest
                    .strip_prefix('(')
                    .and_then(|rest| rest.strip_suffix(')'))
                    .ok_or_else(|| wrong("after `]` comes nothing, or one number as in (1)"))?;
                let sources = parse_sources(inner, true, reading)?;
                if sources.len() != 1 {
                    return Err(wrong("after `]` comes nothing, or one number as in (1)"));
                }
                sources
            }
        };
        return Ok(PersistentName::new(FeatureId(feature), role, sources).with_faces(faces, near));
    }
    let rest = rest
        .strip_prefix('(')
        .ok_or_else(|| wrong("there is no `(` after the role"))?;
    let inner = rest
        .strip_suffix(')')
        .ok_or_else(|| wrong("it does not end with `)`"))?;
    let sources = parse_sources(inner, role.is_numbered() || role.is_counted(), reading)?;
    if role.is_counted() && sources.is_empty() {
        return Err(wrong(
            "this role takes at least one number, as in copy(0) or chamfer(0)",
        ));
    }
    if role.is_numbered() && sources.len() != 1 {
        return Err(wrong("this role takes one number, as in face(2)"));
    }
    Ok(PersistentName::new(FeatureId(feature), role, sources))
}

/// A chain of names: `F5:copy(2)/F2:side(e7)`. With `lists` its last may
/// be an edge or a vertex that lists faces.
fn parse_chain(
    text: &str,
    reading: &mut Reading<'_>,
    lists: bool,
) -> Result<PersistentName, String> {
    let wrong = reading.wrong;
    let mut links = Vec::new();
    for part in split_outside(text, '/') {
        if links.len() >= MAX_NAME_DEPTH {
            return Err(wrong("it is made of too many names"));
        }
        links.push(parse_link(part, reading, lists)?);
    }
    // Built from the end: every name but the last is derived from the
    // one after it, and the last is derived from nothing.
    let mut name: Option<PersistentName> = None;
    while let Some(mut link) = links.pop() {
        match (link.role.is_derived(), name.take()) {
            (true, Some(of)) => link.of = Some(Arc::new(of)),
            (true, None) => {
                return Err(wrong(
                    "a copy is followed by `/` and the name it is a copy of",
                ));
            }
            (false, Some(_)) => {
                return Err(wrong("only a copy is followed by `/` and another name"));
            }
            (false, None) => {}
        }
        name = Some(link);
    }
    name.ok_or_else(|| wrong("it is empty"))
}

impl PersistentName {
    /// Reads a name in which a feature may be written by its name as well
    /// as by its id, wherever a feature starts a name (`Plate:face(1)`,
    /// `F7:copy(1)/Pin:side(e2)`, `Off:offset(e7)` inside a cap's
    /// sources): `features` gives the id of a name, or why there is none
    /// (unknown, or several features of that name). The name read has
    /// ids only, as every stored name. A feature's name that holds a `/`,
    /// a bracket, a `,` or a `|` cannot be read so: write its id.
    pub fn parse_with_features(
        text: &str,
        features: &dyn Fn(&str) -> Result<FeatureId, String>,
    ) -> Result<Self, String> {
        Self::read(text, Some(features))
    }

    fn read(
        text: &str,
        features: Option<&dyn Fn(&str) -> Result<FeatureId, String>>,
    ) -> Result<Self, String> {
        let shown: String = text.chars().take(60).collect();
        let wrong =
            |what: &str| format!("`{shown}` is not a name: {what}; a name looks like F2:side(e7)");
        let mut reading = Reading {
            wrong: &wrong,
            sources: MAX_NAME_SOURCES,
            faces: MAX_NAME_FACES,
            features,
        };
        // Read in the long form or the short one, it is the same name.
        let name = parse_chain(text, &mut reading, true)?.canonical();
        name.check().map_err(wrong)?;
        Ok(name)
    }
}

impl FromStr for PersistentName {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Self::read(text, None)
    }
}

impl TryFrom<String> for PersistentName {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_cap_stands_for_the_caps_of_its_role() {
        let name = |text: &str| text.parse::<PersistentName>().unwrap();
        let short = name("F2:startCap()");
        assert!(short.is_short_cap());
        assert!(short.shortens(&name("F2:startCap(e5,e6,e7)")));
        assert!(!short.shortens(&name("F2:endCap(e5,e6,e7)")));
        assert!(!short.shortens(&name("F3:startCap(e5,e6,e7)")));
        assert!(!short.shortens(&short));
        assert!(!short.shortens(&name("F7:copy(0)/F2:startCap(e5,e6,e7)")));
        let copied = name("F7:copy(0)/F2:endCap()");
        assert!(copied.is_short_cap());
        assert!(copied.shortens(&name("F7:copy(0)/F2:endCap(e1,e2)")));
        assert!(!copied.shortens(&name("F7:copy(1)/F2:endCap(e1,e2)")));
        assert!(!copied.shortens(&name("F2:endCap(e1,e2)")));
        // A full name is no short form of anything.
        let full = name("F2:startCap(e5,e6,e7)");
        assert!(!full.is_short_cap());
        assert!(!full.shortens(&full));
        assert!(!name("F2:side(e5)").is_short_cap());
    }

    #[test]
    fn a_cap_without_brackets_is_its_short_form() {
        let name = |text: &str| text.parse::<PersistentName>().unwrap();
        assert_eq!(name("F2:startCap"), name("F2:startCap()"));
        assert_eq!(name("F2:endCap"), name("F2:endCap()"));
        assert_eq!(name("F7:copy(0)/F2:endCap"), name("F7:copy(0)/F2:endCap()"));
        assert!(name("F2:endCap").is_short_cap());
        assert_eq!(name("F2:startCap").to_string(), "F2:startCap()");
        // Only a cap has a short form.
        for text in ["F2:side", "F3:face", "F2:startEdge", "F2:startcap"] {
            assert!(text.parse::<PersistentName>().is_err(), "{text}");
        }
    }

    #[test]
    fn a_feature_may_be_written_by_its_name_when_a_document_resolves_it() {
        let features = |text: &str| -> Result<FeatureId, String> {
            match text {
                "Plate" => Ok(FeatureId(2)),
                "Off" => Ok(FeatureId(3)),
                "Joined" => Ok(FeatureId(7)),
                "a:b" => Ok(FeatureId(9)),
                "Twice" => Err("several features are called `Twice`".to_string()),
                other => Err(format!("no feature is called `{other}`")),
            }
        };
        let read = |text: &str| PersistentName::parse_with_features(text, &features);
        let id = |text: &str| text.parse::<PersistentName>().unwrap();
        for (by_name, by_id) in [
            ("Plate:face(1)", "F2:face(1)"),
            ("Plate:side(e7)", "F2:side(e7)"),
            ("Plate:startCap", "F2:startCap()"),
            ("Plate:endCap()", "F2:endCap()"),
            ("Joined:copy(1)/Plate:side(e7)", "F7:copy(1)/F2:side(e7)"),
            ("F7:copy(1)/Plate:side(e7)", "F7:copy(1)/F2:side(e7)"),
            (
                "Joined:between[Plate:side(e7),F2:endCap(e1)]",
                "F7:between[F2:side(e7),F2:endCap(e1)]",
            ),
            (
                "Plate:endCap(Off:offset(e1,e2))",
                "F2:endCap(F3:offset(e1,e2))",
            ),
            (
                "Plate:side(Off:loop(e1,e7)/Off:offset(e7))",
                "F2:side(F3:loop(e1,e7)/F3:offset(e7))",
            ),
            ("a:b:face(0)", "F9:face(0)"),
            // An id stays an id.
            ("F2:face(1)", "F2:face(1)"),
        ] {
            assert_eq!(read(by_name).unwrap(), id(by_id), "{by_name}");
            // What is stored has ids only.
            assert_eq!(read(by_name).unwrap().to_string(), id(by_id).to_string());
        }
        let unknown = read("Plat:face(1)").unwrap_err();
        assert!(unknown.contains("no feature is called `Plat`"), "{unknown}");
        let nested = read("F7:copy(1)/Nope:side(e7)").unwrap_err();
        assert!(nested.contains("no feature is called `Nope`"), "{nested}");
        let twice = read("Twice:face(0)").unwrap_err();
        assert!(twice.contains("several features"), "{twice}");
        // Without a document a name has ids only.
        assert!("Plate:face(1)".parse::<PersistentName>().is_err());
        assert!(read(":face(1)").is_err());
    }

    #[test]
    fn a_piece_is_named_after_the_first_with_its_number() {
        let side = PersistentName::new(FeatureId(2), NameRole::Side, vec![7]);
        assert_eq!(PersistentName::piece(0, side.clone()), side);
        let second = PersistentName::piece(1, side.clone());
        assert_eq!(second.to_string(), "F2:piece(1)/F2:side(e7)");
        assert!(second.is_face() && !second.is_edge());
        assert_eq!(second.check(), Ok(()));
        assert_eq!(second.root(), &side);
        assert_ne!(second, PersistentName::piece(2, side));
    }

    #[test]
    fn a_name_is_written_and_read_back() {
        let cases = [
            ("F2:side(e7)", NameRole::Side, vec![7]),
            ("F2:endCap(e7,e8,e9)", NameRole::EndCap, vec![7, 8, 9]),
            ("F2:lateralEdge(e9,e7)", NameRole::LateralEdge, vec![9, 7]),
            (
                "F18446744073709551615:startVertex()",
                NameRole::StartVertex,
                vec![],
            ),
            ("F5:copy(2)/F2:side(e7)", NameRole::Copy, vec![2]),
            ("F3:face(5)", NameRole::Face, vec![5]),
            ("F3:edge(11)", NameRole::Edge, vec![11]),
            ("F3:vertex(0)", NameRole::Vertex, vec![0]),
            ("F4:copy(1)/F3:face(5)", NameRole::Copy, vec![1]),
            ("F2:piece(1)/F2:side(e7)", NameRole::Piece, vec![1]),
            (
                "F5:copy(0)/F2:piece(3)/F2:lateralEdge(e7,e7)",
                NameRole::Copy,
                vec![0],
            ),
            (
                "F9:copy(0)/F5:copy(11)/F2:endCap(e7,e8)",
                NameRole::Copy,
                vec![0],
            ),
            ("F9:copy(1,3,2)/F2:side(e7)", NameRole::Copy, vec![1, 3, 2]),
        ];
        for (text, role, sources) in cases {
            let name: PersistentName = text.parse().unwrap();
            assert_eq!((name.role, &name.sources), (role, &sources), "{text}");
            assert_eq!(name.to_string(), text);
            // In JSON a name is its text.
            let json = serde_json::to_value(&name).unwrap();
            assert_eq!(json, serde_json::json!(text));
            assert_eq!(
                serde_json::from_value::<PersistentName>(json).unwrap(),
                name
            );
        }
        assert_eq!(
            "F3:side( e4, e5 )"
                .parse::<PersistentName>()
                .unwrap()
                .sources,
            [4, 5],
            "spaces after commas are accepted"
        );
        for role in NameRole::ALL {
            assert_eq!(NameRole::from_name(role.name()), Some(role));
            assert_eq!(
                serde_json::to_value(role).unwrap(),
                serde_json::json!(role.name())
            );
        }
        assert!(NameRole::Side.is_face() && !NameRole::LateralEdge.is_face());
        assert!(NameRole::Face.is_face() && !NameRole::Edge.is_face());
        assert!(!NameRole::Copy.is_face(), "a copy is what it is a copy of");
    }

    #[test]
    fn a_copy_is_what_it_is_a_copy_of() {
        let side: PersistentName = "F2:side(e7)".parse().unwrap();
        let copy = PersistentName::copy(FeatureId(5), 2, side.clone());
        assert_eq!(copy.to_string(), "F5:copy(2)/F2:side(e7)");
        assert_eq!(copy.of.as_deref(), Some(&side));
        assert_eq!((copy.depth(), copy.root()), (2, &side));
        assert!(copy.is_face() && !copy.role.is_face());
        let edge: PersistentName = "F7:copy(0)/F2:lateralEdge(e7,e8)".parse().unwrap();
        assert!(!edge.is_face());
        // Built from parts, a name can be one that cannot be written.
        let mut bare = copy.clone();
        bare.of = None;
        assert!(bare.check().is_err());
        let mut none = copy.clone();
        none.sources.clear();
        assert!(none.check().is_err());
        let mut tail = side.clone();
        tail.of = Some(Arc::new(side.clone()));
        assert!(tail.check().is_err());
        let mut wide = side.clone();
        wide.sources = vec![1; MAX_NAME_SOURCES + 1];
        assert!(wide.check().is_err());
        // Too many links: pieces of pieces, which nothing makes.
        let mut deep = side.clone();
        for _ in 0..MAX_NAME_DEPTH {
            deep = PersistentName {
                of: Some(Arc::new(deep)),
                ..PersistentName::new(FeatureId(2), NameRole::Piece, vec![1])
            };
        }
        assert_eq!(deep.check(), Err("it is made of too many names"));
    }

    /// Every instance of a pattern names a face of its seed alike, whether
    /// or not something took the seed's body between (a chamfer of the
    /// seed): the run of instance 0 down to a name that is no copy is left
    /// out in instance 0 and in the others (the Perseverance's finding,
    /// 2026-10-21).
    #[test]
    fn every_instance_names_a_face_of_its_seed_alike() {
        let face: PersistentName = "F2:face(2)".parse().unwrap();
        let chamfered = PersistentName::copy(FeatureId(3), 0, face.clone());
        for k in 0..8 {
            let plain = PersistentName::copy(FeatureId(4), k, face.clone());
            let through = PersistentName::copy(FeatureId(4), k, chamfered.clone());
            assert_eq!(through, plain, "instance {k}");
            assert_eq!(through.to_string(), format!("F4:copy({k})/F2:face(2)"));
        }
        // Before a copy that is not instance 0 the run stays a link, so
        // the two ways through a pattern of a pattern stay apart.
        let inner = PersistentName::copy(FeatureId(7), 2, face.clone());
        let a = PersistentName::copy(
            FeatureId(9),
            1,
            PersistentName::copy(FeatureId(8), 0, inner.clone()),
        );
        let b = PersistentName::copy(
            FeatureId(9),
            1,
            PersistentName::copy(FeatureId(8), 2, PersistentName::copy(FeatureId(7), 0, face)),
        );
        assert_ne!(a, b);
        assert_eq!(a.to_string(), "F9:copy(1)/F8:copy(0)/F7:copy(2)/F2:face(2)");
        assert_eq!(b.to_string(), "F9:copy(1,2)/F2:face(2)");
        for name in [a, b] {
            assert_eq!(name.to_string().parse::<PersistentName>().unwrap(), name);
            assert_eq!(name.clone().canonical(), name);
        }
    }

    #[test]
    fn a_copy_of_a_copy_that_goes_the_same_way_is_one_link() {
        let side: PersistentName = "F2:side(e7)".parse().unwrap();
        let copy = PersistentName::copy(FeatureId(5), 2, side.clone());
        // Instance 1 of F9 of instance 2 of F5: one link.
        let twice = PersistentName::copy(FeatureId(9), 1, copy.clone());
        assert_eq!(twice.to_string(), "F9:copy(1,2)/F2:side(e7)");
        assert_eq!((twice.depth(), twice.root()), (2, &side));
        assert_eq!(twice.to_string().parse::<PersistentName>().unwrap(), twice);
        // Instance 0 of F9 of instance 2 of F5: the way changes, two links.
        let target = PersistentName::copy(FeatureId(9), 0, copy.clone());
        assert_eq!(target.to_string(), "F9:copy(0)/F5:copy(2)/F2:side(e7)");
        // A face a row of features each kept as instance 0 has one name of
        // two links, after the first feature of the row.
        let mut kept = PersistentName::copy(FeatureId(3), 0, side.clone());
        for feature in 4..2000 {
            kept = PersistentName::copy(FeatureId(feature), 0, kept);
        }
        assert_eq!(kept.to_string(), "F1999:copy(0)/F2:side(e7)");
        assert_eq!(kept.check(), Ok(()));
        // The wall of the first of a thousand holes: the tool of the first
        // cut, then the target of every later one.
        let mut wall = PersistentName::copy(FeatureId(4), 1, side.clone());
        for feature in 5..2000 {
            wall = PersistentName::copy(FeatureId(feature), 0, wall);
        }
        assert_eq!(wall.to_string(), "F1999:copy(0)/F4:copy(1)/F2:side(e7)");
        let through = PersistentName::copy(FeatureId(2001), 1, wall);
        assert_eq!(
            through.to_string(),
            "F2001:copy(1)/F1999:copy(0)/F4:copy(1)/F2:side(e7)"
        );
        // A tool of a tool of a tool: one link, an instance per step.
        let mut tool = side.clone();
        for feature in 3..10 {
            tool = PersistentName::copy(FeatureId(feature), 1, tool);
        }
        assert_eq!(tool.to_string(), "F9:copy(1,1,1,1,1,1,1)/F2:side(e7)");
        // The long form is read as the short one: the same name.
        for (long, short) in [
            (
                "F9:copy(0)/F5:copy(2)/F2:side(e7)",
                "F9:copy(0)/F5:copy(2)/F2:side(e7)",
            ),
            (
                "F9:copy(1)/F5:copy(2)/F2:side(e7)",
                "F9:copy(1,2)/F2:side(e7)",
            ),
            (
                "F11:copy(0)/F10:copy(0)/F9:copy(0)/F3:copy(1)/F2:face(0)",
                "F11:copy(0)/F3:copy(1)/F2:face(0)",
            ),
            (
                "F11:copy(0)/F10:copy(0)/F9:copy(0)/F6:copy(0)/F3:copy(0)/F1:face(0)",
                "F11:copy(0)/F1:face(0)",
            ),
            (
                "F7:copy(1)/F5:copy(0)/F2:side(e7)",
                "F7:copy(1)/F2:side(e7)",
            ),
            (
                "F7:copy(1)/F5:copy(0)/F4:copy(0)/F2:side(e7)",
                "F7:copy(1)/F2:side(e7)",
            ),
            (
                "F9:copy(1)/F8:copy(2)/F7:copy(0)/F2:side(e7)",
                "F9:copy(1,2)/F2:side(e7)",
            ),
            (
                "F9:copy(1)/F8:copy(0)/F7:copy(2)/F2:side(e7)",
                "F9:copy(1)/F8:copy(0)/F7:copy(2)/F2:side(e7)",
            ),
            (
                "F9:copy(1)/F8:copy(3)/F7:copy(0)/F6:copy(0)/F3:copy(2)/F2:piece(1)/F2:side(e7)",
                "F9:copy(1,3)/F7:copy(0)/F3:copy(2)/F2:piece(1)/F2:side(e7)",
            ),
            (
                "F9:copy(0)/F8:copy(1)/F7:between[F7:copy(0)/F3:copy(0)/F1:face(1),F7:copy(1)/F2:side(e1)]",
                "F9:copy(0)/F8:copy(1)/F7:between[F7:copy(0)/F1:face(1),F7:copy(1)/F2:side(e1)]",
            ),
        ] {
            let read: PersistentName = long.parse().unwrap();
            assert_eq!(read.to_string(), short, "{long}");
            assert_eq!(read, short.parse::<PersistentName>().unwrap(), "{long}");
            assert_eq!(read.clone().canonical(), read, "{long}");
        }
        // A file has the long form link by link; built so, it is the same.
        let built = PersistentName {
            of: Some(Arc::new(PersistentName {
                of: Some(Arc::new(side.clone())),
                ..PersistentName::new(FeatureId(5), NameRole::Copy, vec![1])
            })),
            ..PersistentName::new(FeatureId(9), NameRole::Copy, vec![2])
        };
        assert_eq!(built.to_string(), "F9:copy(2)/F5:copy(1)/F2:side(e7)");
        assert_eq!(built.canonical().to_string(), "F9:copy(2,1)/F2:side(e7)");
        // A way with a 0 among other numbers is not a name: it is two
        // links.
        for text in ["F9:copy(1,0)/F2:side(e7)", "F9:copy(0,1)/F2:side(e7)"] {
            let error = text.parse::<PersistentName>().unwrap_err();
            assert!(error.contains("none of which is 0"), "{text}: {error}");
        }
        // The faces an edge lists are put in order again: their short
        // forms can compare otherwise than their long ones did.
        let a = "F7:copy(0)/F5:copy(0)/F1:face(1)";
        let b = "F7:copy(0)/F3:side(e1)";
        let edge: PersistentName = format!("F7:between[{a},{b}]").parse().unwrap();
        assert_eq!(
            edge.to_string(),
            "F7:between[F7:copy(0)/F1:face(1),F7:copy(0)/F3:side(e1)]"
        );
        // Hostile: a long form beyond the cap is refused before it is made
        // short, and a deep chain built by hand is made short in a loop.
        assert!(
            ("F1:copy(0)/".repeat(MAX_NAME_DEPTH) + "F2:side(e7)")
                .parse::<PersistentName>()
                .is_err()
        );
        let mut deep = side.clone();
        for _ in 0..100_000 {
            deep = PersistentName {
                of: Some(Arc::new(deep)),
                ..PersistentName::new(FeatureId(1), NameRole::Copy, vec![0])
            };
        }
        assert_eq!(deep.canonical().to_string(), "F1:copy(0)/F2:side(e7)");
    }

    #[test]
    fn text_that_is_not_a_name_is_refused_with_a_reason() {
        for (text, reason) in [
            ("", "no `:`"),
            ("side(e7)", "no `:`"),
            ("2:side(e7)", "F and a number"),
            ("F:side(e7)", "F and a number"),
            ("F-1:side(e7)", "F and a number"),
            ("F99999999999999999999:side(e7)", "F and a number"),
            ("F2:side", "no `(`"),
            ("F2:roof(e7)", "the role must be one of"),
            ("F2:side(e7", "does not end with `)`"),
            ("F2:side(7)", "e and a number"),
            ("F2:side(e7,)", "e and a number"),
            ("F2:side(e7)x", "does not end with `)`"),
            ("F2:side(e7;e8)", "e and a number"),
            ("F5:copy(2)", "followed by `/`"),
            ("F5:copy(2)/", "no `:`"),
            ("F5:copy()/F2:side(e7)", "one number"),
            ("F5:copy(1,e2)/F2:side(e7)", "plain number"),
            ("F5:copy(1,)/F2:side(e7)", "plain number"),
            ("F3:face()", "one number"),
            ("F3:face(e1)", "plain number"),
            ("F3:edge(1,2)", "one number"),
            ("F3:face(1)/F2:side(e7)", "only a copy"),
            ("F5:copy(e2)/F2:side(e7)", "plain number"),
            ("F5:copy(+2)/F2:side(e7)", "plain number"),
            ("F2:side(e7)/F1:side(e1)", "only a copy"),
            ("/F2:side(e7)", "no `:`"),
        ] {
            let error = text.parse::<PersistentName>().unwrap_err();
            assert!(error.contains(reason), "`{text}`: {error}");
        }
        // Hostile lengths: an error or a name, never a panic or a huge
        // allocation.
        assert!(
            ("F2:side(".to_string() + &"e1,".repeat(100_000) + "e1)")
                .parse::<PersistentName>()
                .is_err()
        );
        assert!("💥".repeat(10_000).parse::<PersistentName>().is_err());
        assert!(
            "F1:copy(0)/"
                .repeat(100_000)
                .parse::<PersistentName>()
                .is_err()
        );
        assert!("/".repeat(100_000).parse::<PersistentName>().is_err());
        // The sources of all links together are capped.
        let many = "e1,".repeat(MAX_NAME_SOURCES - 1) + "e1";
        assert!(format!("F2:side({many})").parse::<PersistentName>().is_ok());
        assert!(
            format!("F5:copy(0)/F2:side({many})")
                .parse::<PersistentName>()
                .is_err()
        );
        assert!(serde_json::from_value::<PersistentName>(serde_json::json!(7)).is_err());
    }

    #[test]
    fn an_edge_and_a_vertex_are_named_by_their_faces() {
        let a = "F7:copy(0)/F1:face(1)";
        let b = "F7:copy(1)/F2:endCap(e7,e8,e9)";
        let c = "F7:copy(1)/F2:side(e7)";
        for (text, role, faces, near, index) in [
            (
                format!("F7:between[{a},{b}]"),
                NameRole::Between,
                2,
                0,
                None,
            ),
            (
                format!("F7:between[{a},{a}]"),
                NameRole::Between,
                2,
                0,
                None,
            ),
            (
                format!("F7:between[{a},{b}|{c}]"),
                NameRole::Between,
                2,
                1,
                None,
            ),
            (
                format!("F7:between[{a},{b}|{c}](1)"),
                NameRole::Between,
                2,
                1,
                Some(1),
            ),
            (
                format!("F7:between[{a},{b}](0)"),
                NameRole::Between,
                2,
                0,
                Some(0),
            ),
            (
                format!("F7:corner[{a},{b},{c}]"),
                NameRole::Corner,
                3,
                0,
                None,
            ),
            (
                format!("F7:corner[{a}|{b},{c}](2)"),
                NameRole::Corner,
                1,
                2,
                Some(2),
            ),
            ("F7:corner[]".to_string(), NameRole::Corner, 0, 0, None),
            (
                "F7:corner[](3)".to_string(),
                NameRole::Corner,
                0,
                0,
                Some(3),
            ),
            (
                format!("F9:copy(1)/F7:corner[{a},{b},{c}]"),
                NameRole::Copy,
                3,
                0,
                None,
            ),
        ] {
            let name: PersistentName = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            let root = name.root();
            assert_eq!(name.role, role, "{text}");
            assert_eq!(
                (
                    root.faces().len(),
                    root.near().len(),
                    root.sources.first().copied()
                ),
                (faces, near, index),
                "{text}"
            );
            assert_eq!(name.to_string(), text);
            assert_eq!(name.check(), Ok(()), "{text}");
            assert!(!name.is_face(), "{text}");
            assert_eq!(name.is_vertex(), text.contains("corner"), "{text}");
            assert_eq!(name.is_edge(), text.contains("between"), "{text}");
            let json = serde_json::to_value(&name).unwrap();
            assert_eq!(json, serde_json::json!(text));
            assert_eq!(
                serde_json::from_value::<PersistentName>(json).unwrap(),
                name
            );
        }
        // Built from parts, it is the same name.
        let face = |text: &str| text.parse::<PersistentName>().unwrap();
        let built =
            PersistentName::corner(FeatureId(7), vec![face(a), face(b)], vec![face(c)], Some(1));
        assert_eq!(built.to_string(), format!("F7:corner[{a},{b}|{c}](1)"));
        assert_eq!(built.to_string().parse::<PersistentName>().unwrap(), built);
        let edge = PersistentName::between(FeatureId(7), vec![face(a), face(b)], Vec::new(), None);
        assert_eq!(edge.to_string(), format!("F7:between[{a},{b}]"));
        // The faces in any order are the same name, written in the one
        // order.
        let typed: PersistentName = format!("F7:corner[{c},{b},{a}|{c},{a}](1)")
            .parse()
            .unwrap();
        assert_eq!(
            typed.to_string(),
            format!("F7:corner[{a},{b},{c}|{a},{c}](1)")
        );
        assert_eq!(
            format!("F9:copy(0)/F7:between[{b},{a}]")
                .parse::<PersistentName>()
                .unwrap(),
            format!("F9:copy(0)/F7:between[{a},{b}]")
                .parse::<PersistentName>()
                .unwrap()
        );
        // It is as deep as its faces: it is of the same body as they are.
        assert_eq!((face(a).depth(), built.depth()), (2, 2));
        assert_eq!(
            PersistentName::copy(FeatureId(8), 0, built.clone()).depth(),
            3
        );
        assert!(face("F3:vertex(1)").is_vertex() && face("F3:startVertex(e1)").is_vertex());
        assert!(face("F3:edge(1)").is_edge() && face("F3:lateralEdge(e1,e2)").is_edge());
        assert!(face("F4:copy(0)/F3:endVertex(e1)").is_vertex());
    }

    #[test]
    fn text_that_is_not_a_name_by_faces_is_refused_with_a_reason() {
        let a = "F7:copy(0)/F1:face(1)";
        for (text, reason) in [
            ("F7:corner".to_string(), "no `(`"),
            ("F7:corner(1)".to_string(), "followed by `[`"),
            (format!("F7:corner[{a}"), "no `]`"),
            (format!("F7:corner[{a}]x"), "after `]`"),
            (format!("F7:corner[{a}]()"), "after `]`"),
            (format!("F7:corner[{a}](1,2)"), "after `]`"),
            (format!("F7:corner[{a}](e1)"), "plain number"),
            (format!("F7:corner[{a}|{a}|{a}]"), "one `|`"),
            (format!("F7:corner[{a},]"), "no `:`"),
            ("F7:corner[F1:vertex(1)]".to_string(), "not a face"),
            ("F7:corner[F7:copy(0)/F1:edge(1)]".to_string(), "not a face"),
            (
                format!("F7:corner[F7:corner[{a}]]"),
                "list no faces themselves",
            ),
            (format!("F7:corner[{a}]/F2:side(e7)"), "only a copy"),
            (format!("F7:side[{a}]"), "no `(`"),
            (format!("F7:face[{a}](1)"), "no `(`"),
            (format!("F7:corner[{a}"), "no `]`"),
            ("F7:corner[[[[".to_string(), "no `]`"),
            ("F7:corner]".to_string(), "no `(`"),
        ] {
            let error = text.parse::<PersistentName>().unwrap_err();
            assert!(error.contains(reason), "`{text}`: {error}");
        }
        // The caps: faces listed, sources of all of them, depth.
        let many = |n: usize| format!("F7:corner[{}]", vec![a; n].join(","));
        assert!(many(MAX_NAME_FACES).parse::<PersistentName>().is_ok());
        assert!(
            many(MAX_NAME_FACES + 1)
                .parse::<PersistentName>()
                .unwrap_err()
                .contains("too many faces")
        );
        let wide = format!(
            "F2:side({})",
            vec!["e1"; MAX_NAME_SOURCES / 2 + 1].join(",")
        );
        assert!(
            format!("F2:corner[{wide}]")
                .parse::<PersistentName>()
                .is_ok()
        );
        assert!(
            format!("F2:corner[{wide},{wide}]")
                .parse::<PersistentName>()
                .unwrap_err()
                .contains("too many sources")
        );
        // A long form within the cap is read as its short form.
        let deep = "F1:copy(0)/".repeat(MAX_NAME_DEPTH - 1) + "F2:side(e7)";
        let corner = format!("F1:corner[{deep}]");
        assert_eq!(
            corner.parse::<PersistentName>().unwrap().to_string(),
            "F1:corner[F1:copy(0)/F2:side(e7)]"
        );
        assert!(
            format!("F1:corner[F1:copy(0)/{deep}]")
                .parse::<PersistentName>()
                .is_err()
        );
        // Hostile lengths and nesting: an error, never a panic.
        assert!(
            ("F7:corner[".repeat(10_000) + &"]".repeat(10_000))
                .parse::<PersistentName>()
                .is_err()
        );
        assert!(
            format!("F7:corner[{}", "(".repeat(100_000))
                .parse::<PersistentName>()
                .is_err()
        );
        assert!("|".repeat(100_000).parse::<PersistentName>().is_err());
        // Built from parts, a name can be one that cannot be written.
        let face: PersistentName = a.parse().unwrap();
        let good = PersistentName::corner(FeatureId(7), vec![face.clone()], Vec::new(), None);
        assert_eq!(good.check(), Ok(()));
        let nested = PersistentName::corner(FeatureId(7), vec![good.clone()], Vec::new(), None);
        assert!(nested.check().is_err());
        let listing = face.clone().with_faces(Vec::new(), vec![face.clone()]);
        assert!(listing.check().is_err(), "a face lists no faces");
        let mut two = good.clone();
        two.sources = vec![1, 2];
        assert!(two.check().is_err());
        let vertex = PersistentName::new(FeatureId(1), NameRole::Vertex, vec![1]);
        assert!(
            PersistentName::corner(FeatureId(7), vec![vertex], Vec::new(), None)
                .check()
                .is_err()
        );
        // Written, a name never goes deeper than one list, whatever it is.
        assert_eq!(nested.to_string(), "F7:corner[F7:corner[]]");
    }

    #[test]
    fn an_instance_of_a_sketch_pattern_is_a_source_of_its_own() {
        // The curve and its number, both ways.
        for (text, curve) in [
            ("e7", CurveRef::seed(7)),
            (
                "e7[3]",
                CurveRef {
                    entity: 7,
                    index: CurveIndex::At(3),
                },
            ),
            (
                "e7[2,5]",
                CurveRef {
                    entity: 7,
                    index: CurveIndex::Grid(2, 5),
                },
            ),
            (
                "e7[*]",
                CurveRef {
                    entity: 7,
                    index: CurveIndex::All,
                },
            ),
            (
                "e4294967295[65533,0]",
                CurveRef {
                    entity: MAX_CURVE_ENTITY,
                    index: CurveIndex::Grid(MAX_CURVE_INDEX, 0),
                },
            ),
        ] {
            assert_eq!(CurveRef::parse(text), Some(curve), "{text}");
            assert_eq!(curve.to_string(), text);
            let number = curve.encode().unwrap();
            assert_eq!(CurveRef::decode(number), Some(curve), "{text}");
        }
        // A seed's number is its id: names without patterns are unchanged.
        assert_eq!(CurveRef::seed(7).encode(), Some(7));
        for bad in [
            "e7[",
            "e7[]",
            "e7[-1]",
            "e7[1,2,3]",
            "e7[last]",
            "e7[65534]",
            "e4294967296",
            "7",
            "e",
            "e7[*,1]",
            "e7[1]x",
        ] {
            assert_eq!(CurveRef::parse(bad), None, "{bad}");
        }
        // Numbers no curve encodes to are shown as they are.
        assert_eq!(CurveRef::decode((1 << 48) | 7), None);

        // In names: a side, a cap with a whole run, an edge between faces
        // that have brackets of their own.
        for text in [
            "F2:side(e7[3])",
            "F2:lateralEdge(e7[3],e8[3])",
            "F2:endCap(e1,e2,e7[*])",
            "F2:side(e7[2,5])",
            "F9:copy(1)/F2:side(e7[4])",
            "F7:between[F7:copy(0)/F2:side(e7[3]),F7:copy(1)/F3:face(1)|F7:copy(0)/F2:side(e8[3])]",
        ] {
            let name: PersistentName = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(name.to_string(), text);
            assert_eq!(name.check(), Ok(()));
        }
        let side = |text: &str| text.parse::<PersistentName>().unwrap();
        assert!(side("F2:side(e7)") < side("F2:side(e7[1])"));
        // An id beyond 32 bits is not a sketch entity's.
        let error = "F8:side(e18446744073709551615)"
            .parse::<PersistentName>()
            .unwrap_err();
        assert!(error.contains("below 4294967296"), "{error}");
        assert!("F8:side(e7[1)".parse::<PersistentName>().is_err());
        assert!(
            "F8:between[F8:side(e7[1]"
                .parse::<PersistentName>()
                .is_err()
        );
    }

    #[test]
    fn a_curve_of_nested_patterns_names_each_level_innermost_first() {
        let at = CurveIndex::At;
        // The path and its words, both ways.
        for (text, levels, words) in [
            ("e3[2][1]", vec![at(2), at(1)], 2),
            ("e3[0][1]", vec![at(0), at(1)], 2),
            ("e3[*][1]", vec![CurveIndex::All, at(1)], 2),
            ("e5[1][2,4]", vec![at(1), CurveIndex::Grid(2, 4)], 2),
            ("e5[1,0][2]", vec![CurveIndex::Grid(1, 0), at(2)], 2),
            ("e3[2][1][3]", vec![at(2), at(1), at(3)], 3),
            (
                "e3[*][*][3]",
                vec![CurveIndex::All, CurveIndex::All, at(3)],
                3,
            ),
            ("e3[2][0][3]", vec![at(2), at(0), at(3)], 3),
            ("e3[2]", vec![at(2)], 1),
        ] {
            let path = CurvePath::parse(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(
                path,
                CurvePath::of_levels(3.max(path.curve.entity), &levels)
            );
            assert_eq!(path.to_string(), text);
            let encoded = path.encode().unwrap();
            assert_eq!(encoded.len(), words, "{text}");
            assert_eq!(CurvePath::read(&encoded), Some((path.clone(), words)));
            // The first word is the curve's own: a curve in cell 0 has the
            // number it had before it was wrapped.
            assert_eq!(CurveRef::decode(encoded[0]), Some(path.curve));
            for word in &encoded[1..] {
                assert_eq!(CurveRef::decode(*word), None, "{text}");
                assert_eq!(OffsetHead::decode(*word), None, "{text}");
            }
            for (level, index) in levels.iter().enumerate() {
                assert_eq!(path.level(level), *index, "{text} {level}");
            }
        }
        // Trailing zero levels, and `*` after `*`, are left out.
        let of = |levels: &[CurveIndex]| CurvePath::of_levels(3, levels).to_string();
        assert_eq!(of(&[at(2), at(0)]), "e3[2]");
        assert_eq!(of(&[at(0), at(0), at(0)]), "e3");
        assert_eq!(of(&[CurveIndex::All, CurveIndex::All]), "e3[*]");
        assert_eq!(of(&[CurveIndex::All, at(0)]), "e3[*]");
        assert_eq!(
            of(&[CurveIndex::All, CurveIndex::All, CurveIndex::All]),
            "e3[*]"
        );
        assert_eq!(of(&[at(2), at(1), at(0)]), "e3[2][1]");
        assert_eq!(CurvePath::parse("e3[*]").unwrap().level(1), CurveIndex::All);
        assert_eq!(CurvePath::parse("e3[2]").unwrap().level(1), at(0));
        // Not in the one form a name keeps, or not a path.
        for bad in [
            "e3[2][0]",
            "e3[*][*]",
            "e3[*][0]",
            "e3[1][*]",
            "e3[2][1][1][1]",
            "e3[][1]",
            "e3[2][",
            "e3[1,2,3][0]",
            "e3[2]x",
            "e3[2][last]",
            "e3[2][-1]",
            "e3[2][65534]",
        ] {
            assert_eq!(CurvePath::parse(bad), None, "{bad}");
        }
        // In names, and in the order names keep (word by word).
        for text in [
            "F2:side(e3[2][1])",
            "F2:endCap(e1,e3[*],e12[*])",
            "F2:endCap(e1,e3[*][1],e12[1])",
            "F2:lateralEdge(e3[2][1],e5[2][1])",
            "F2:startVertex(e5[0][4],e3[1][4])",
            "F10:side(F9:offset(e3[2][1]))",
            "F10:endCap(F9:offset(e1,e3[*][1]))",
            "F10:side(F9:offsetCorner(e3[2][1],e5[0][2]))",
            "F10:side(F9:loop(e1,e3[*][1])/F9:offset(e3[2][1]))",
            "F7:between[F7:copy(0)/F2:side(e3[2][1]),F7:copy(1)/F3:face(1)]",
        ] {
            let name: PersistentName = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(name.to_string(), text);
            assert_eq!(name.check(), Ok(()));
        }
        let side = |text: &str| text.parse::<PersistentName>().unwrap();
        assert!(side("F2:side(e3[0][1])") < side("F2:side(e3[1])"));
        assert!(side("F2:side(e3[1])") < side("F2:side(e3[1][1])"));
        let cap = side("F2:endCap(e3[*][1],e1)");
        let words = cap.sources.clone();
        assert_eq!(cap.to_string(), "F2:endCap(e3[*][1],e1)");
        assert_eq!(
            PersistentName::new(FeatureId(2), NameRole::EndCap, sort_curves(&words)).to_string(),
            "F2:endCap(e1,e3[*][1])"
        );
        // A continuation word that follows no curve, or one too many.
        let word = |text: &str| CurvePath::parse(text).unwrap().encode().unwrap();
        let cont = word("e3[2][1]")[1];
        let head = OffsetHead {
            feature: FeatureId(9),
            kind: OffsetKind::Offset,
        }
        .encode()
        .unwrap();
        for sources in [
            vec![cont],
            vec![cont, 3],
            vec![head, cont],
            vec![3, cont, cont, cont],
            vec![word("e3")[0], cont],
        ] {
            let name = PersistentName::new(FeatureId(2), NameRole::Side, sources.clone());
            assert!(name.check().is_err(), "{sources:?}");
        }
        assert_eq!(decode_continuation(word("e3[2]")[0]), None);
        assert_eq!(decode_continuation(head), None);
        assert_eq!(decode_continuation(cont), Some(at(1)));
    }

    #[test]
    fn offset_curves_are_written_and_read_back() {
        for text in [
            "F10:side(F9:offset(e7))",
            "F10:side(F9:offset(e7[3]))",
            "F10:side(F9:offsetCorner(e7,e8))",
            "F10:side(F9:loop(e1,e2,e7[2,5])/F9:offset(e7[2,5]))",
            "F10:side(F9:offsetExtension(e7,e8))",
            "F10:lateralEdge(F9:loop(e1,e7)/F9:offset(e7),F9:loop(e1,e7)/F9:offsetCorner(e7,e1))",
            "F10:endCap(F9:offset(e1,e2,e7[*]))",
            "F10:lateralEdge(F9:offset(e7),F9:offsetCorner(e7,e8[4]))",
            "F10:piece(1)/F10:side(F9:offset(e7))",
            "F12:between[F10:side(F9:offset(e7)),F11:face(1)]",
        ] {
            let name: PersistentName = text.parse().unwrap();
            assert_eq!(name.to_string(), text);
            assert_eq!(name.check(), Ok(()));
        }
        let name: PersistentName = "F10:side(F9:offset(e7))".parse().unwrap();
        let head = OffsetHead::decode(name.sources[0]).unwrap();
        assert_eq!(
            (head.feature, head.kind),
            (FeatureId(9), OffsetKind::Offset)
        );
        // A head is no curve, and no curve is a head.
        assert_eq!(CurveRef::decode(name.sources[0]), None);
        for curve in ["e7", "e7[3]", "e7[2,5]", "e7[*]", "e4294967295[65533]"] {
            let number = CurveRef::parse(curve).unwrap().encode().unwrap();
            assert_eq!(OffsetHead::decode(number), None, "{curve}");
        }
    }

    #[test]
    fn malformed_offset_curves_are_refused() {
        for text in [
            "F10:side(F9:offsetCorner(e7))",
            "F10:side(F9:offsetCorner(e7,e8,e9))",
            "F10:side(F9:offset())",
            "F10:side(F9:offset(e7),e8)",
            "F10:side(F9:piece(1)/F9:offset(e7))",
            "F10:side(F8:loop(e7)/F9:offset(e7))",
            "F10:side(F9:loop(e7))",
            "F10:side(F9:loop(e7)/F9:loop(e7))",
            "F10:side(F9:loop()/F9:offset(e7))",
            "F10:side(F9:offsetExtension(e7))",
            "F10:side(F9:side(e7))",
            "F10:side(F4294967296:offset(e7))",
            "F10:side(F9:offset(e7)",
            "F10:side(F9:offset(F8:offset(e7)))",
        ] {
            assert!(text.parse::<PersistentName>().is_err(), "{text}");
        }
        // Built by hand: a curve before any head, a head alone.
        let head = OffsetHead {
            feature: FeatureId(9),
            kind: OffsetKind::Offset,
        }
        .encode()
        .unwrap();
        for sources in [vec![7, head, 8], vec![head]] {
            assert!(
                PersistentName::new(FeatureId(10), NameRole::Side, sources)
                    .check()
                    .is_err()
            );
        }
    }
}
