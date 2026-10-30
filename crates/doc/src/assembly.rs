//! Assemblies: a document that places instances of other documents and
//! mates them (docs/design/assemblies.md; decided by the developer,
//! 2026-10-29).
//!
//! An assembly is a [`Document`] whose [`Document::assembly`] is set. It
//! keeps the document's variables, tables, assertions, outputs, named sets
//! and info as a part does, and has no features of its own: what it shows
//! is its instances' bodies, placed. It is saved as a `.3dcraftasm` file;
//! a `.3dcraft` file never holds one.
//!
//! - **An instance is a reference, not a copy.** Its [`Source`] names a
//!   file by its path, relative to the assembly's file, and the SHA-256 of
//!   its bytes, or a document packed into the assembly's file. The
//!   assembly never edits a source's history.
//! - **Configurations.** An instance may set the source's public
//!   parameters ([`ParamOverride`]), as expressions of the assembly's
//!   variables; the source is regenerated with those values.
//! - **Rule 4 across files.** A mate names geometry through an instance
//!   ([`MateRef`]): `Bar[2]/F3:face(5)`. A name that is gone fails the
//!   mate by that name; it never retargets.
//! - **Rule 2.** Bodies, frames and poses are derived and may be thrown
//!   away. A packed source carries the source document itself.

use crate::{Blob, DocError, Document, Expr, MAX_NAME_LEN};
use serde::{Deserialize, Serialize};

/// Most instances an assembly holds.
pub const MAX_INSTANCES: usize = 10_000;
/// Most mates an assembly holds.
pub const MAX_MATES: usize = 20_000;
/// Most couplings an assembly holds.
pub const MAX_COUPLINGS: usize = 10_000;
/// Most parameter overrides one instance holds.
pub const MAX_OVERRIDES: usize = 1_000;
/// The name of the reserved instance that stands for the world: a mate to
/// it grounds an instance.
pub const WORLD: &str = "world";
/// What a mate names of the world (`world/xy`, …): the planes `xy`, `yz`
/// and `zx` (normal z, x and y; x axis x, y and z), the axes `x`, `y` and
/// `z` (as the plane across them) and the `origin` (as `xy`).
pub const WORLD_FRAMES: [&str; 7] = ["xy", "yz", "zx", "x", "y", "z", "origin"];
/// Most terms a linear coupling holds.
pub const MAX_COUPLING_TERMS: usize = 64;
/// The free motions a mate's variables are named after (`<mate>_angle`, …).
pub const MOTIONS: [&str; 4] = ["angle", "distance", "x", "y"];

/// What an assembly holds beside the document's variables and checks.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Assembly {
    /// In the order they were added; names unique.
    pub instances: Vec<Instance>,
    /// In the order they were added; names unique.
    pub mates: Vec<Mate>,
    /// In the order they were added; names unique.
    pub couplings: Vec<Coupling>,
    /// The source documents carried in the file (`assembly.pack`), each
    /// the bytes of a `.3dcraft` or `.3dcraftasm` file, in ascending order
    /// of their SHA-256, each once.
    pub packed: Vec<Blob>,
}

/// One placed use of a source document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// Unique among the instances: `Bar[3]` or a plain name, not
    /// [`WORLD`], and without `/` (which separates it from the name of
    /// what it holds in a reference).
    pub name: String,
    pub source: Source,
    /// One body of a multi-body source, by the name or the id (`F3`) of
    /// the feature that made it; `None` takes all of the source's bodies.
    pub body: Option<String>,
    /// Public parameters of the source set by this instance, by name, in
    /// the order they were given; names unique.
    pub params: Vec<ParamOverride>,
    /// The SHA-256 the source is pinned to: a source whose bytes have
    /// changed fails the instance instead of being followed.
    pub pinned: Option<String>,
    /// For a source that is an assembly: whether it moves as one block or
    /// keeps its own free motions.
    pub mode: InstanceMode,
    pub visible: bool,
    /// The colour it is drawn in, over the source's own.
    pub colour: Option<[u8; 3]>,
    /// The pose it starts from: the solver's start, and where it stays
    /// when nothing constrains it.
    pub placement: Placement,
}

/// Where an instance's source document is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// A file, by its path relative to the assembly's file (`/` between
    /// the parts), and the SHA-256 of its bytes when it was added or last
    /// followed (lowercase hex).
    File { path: String, hash: String },
    /// A document packed into the assembly's file, by the SHA-256 of its
    /// bytes (one of [`Assembly::packed`]).
    Packed { hash: String },
}

impl Source {
    /// The SHA-256 the source had when it was last read.
    pub fn hash(&self) -> &str {
        match self {
            Source::File { hash, .. } | Source::Packed { hash } => hash,
        }
    }
}

/// How a subassembly moves in its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InstanceMode {
    /// As one block.
    #[default]
    Rigid,
    /// Its own free motions stay free in the parent.
    Flexible,
}

impl InstanceMode {
    pub fn name(self) -> &'static str {
        match self {
            InstanceMode::Rigid => "rigid",
            InstanceMode::Flexible => "flexible",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "rigid" => Some(InstanceMode::Rigid),
            "flexible" => Some(InstanceMode::Flexible),
            _ => None,
        }
    }
}

/// A public parameter of the source set by an instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamOverride {
    pub name: String,
    /// In the assembly's variables, with a unit where the parameter has
    /// one (`2 * len`, `80 mm`).
    pub expr: Expr,
}

/// A rigid pose: a rotation, then a translation.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Placement {
    /// In the assembly's length unit.
    pub position: [f64; 3],
    /// A rotation vector: the axis scaled by the angle, in degrees.
    pub rotation: [f64; 3],
}

impl Placement {
    fn is_finite(&self) -> bool {
        self.position
            .iter()
            .chain(self.rotation.iter())
            .all(|v| v.is_finite())
    }
}

/// How a mate joins two frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MateKind {
    Fixed,
    Revolute,
    Slider,
    Cylindrical,
    Planar,
    Ball,
}

impl MateKind {
    pub const ALL: [MateKind; 6] = [
        MateKind::Fixed,
        MateKind::Revolute,
        MateKind::Slider,
        MateKind::Cylindrical,
        MateKind::Planar,
        MateKind::Ball,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MateKind::Fixed => "fixed",
            MateKind::Revolute => "revolute",
            MateKind::Slider => "slider",
            MateKind::Cylindrical => "cylindrical",
            MateKind::Planar => "planar",
            MateKind::Ball => "ball",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The free motions a mate of this kind leaves, by the name its
    /// variable has after the mate's name (`Hinge_angle`): angles first.
    pub fn free(self) -> &'static [&'static str] {
        match self {
            MateKind::Fixed | MateKind::Ball => &[],
            MateKind::Revolute => &["angle"],
            MateKind::Slider => &["distance"],
            MateKind::Cylindrical => &["angle", "distance"],
            MateKind::Planar => &["x", "y", "angle"],
        }
    }
}

/// Geometry named through an instance: `Bar[2]/F3:face(5)`, or the world
/// with one of its datums (`world/xy`, `world/z`, `world/origin`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MateRef {
    pub instance: String,
    /// A persistent name of a face, edge or vertex of the instance's
    /// bodies, or the name or id of a datum feature of its source, as
    /// text.
    pub name: String,
}

impl MateRef {
    /// Reads `instance/name`: the instance is everything before the first
    /// `/`.
    pub fn parse(text: &str) -> Result<Self, DocError> {
        let (instance, name) = text.split_once('/').ok_or_else(|| {
            DocError::Invalid(format!(
                "`{text}` is not a reference through an instance: write `Instance/Feature:face(n)`"
            ))
        })?;
        let reference = Self {
            instance: instance.to_string(),
            name: name.to_string(),
        };
        reference.check()?;
        Ok(reference)
    }

    fn check(&self) -> Result<(), DocError> {
        if self.instance.is_empty() || self.name.trim().is_empty() {
            return Err(DocError::Invalid(format!(
                "`{self}` needs an instance and a name"
            )));
        }
        if self.name.len() > MAX_NAME_LEN * 4 || self.name.chars().any(char::is_control) {
            return Err(DocError::Invalid(format!(
                "the name in `{}` is too long or holds control characters",
                self.instance
            )));
        }
        // The world's frames are known without reading anything: one it
        // does not have is refused here, not left to fail the solve.
        if self.instance == WORLD && !WORLD_FRAMES.contains(&self.name.as_str()) {
            return Err(DocError::Invalid(format!(
                "the world has no `{}`: write {}",
                self.name.chars().take(60).collect::<String>(),
                WORLD_FRAMES
                    .iter()
                    .map(|f| format!("{WORLD}/{f}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        Ok(())
    }
}

impl std::fmt::Display for MateRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.instance, self.name)
    }
}

/// A joint between two instances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mate {
    /// Unique among the mates, and a valid variable name: its free
    /// motions are the variables `<name>_angle`, `<name>_distance`, ….
    pub name: String,
    pub kind: MateKind,
    pub a: MateRef,
    pub b: MateRef,
    /// The distance between the two frames along `a`'s z, when given.
    pub offset: Option<Expr>,
    /// The angle between the two frames about `a`'s z, when given.
    pub angle: Option<Expr>,
    /// The range the main free motion stays in.
    pub limits: Option<(Expr, Expr)>,
    /// `b`'s frame turned over (its z reversed).
    pub flip: bool,
}

/// A coupling of two mates' motions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CouplingKind {
    /// `θa = −ratio · θb`.
    Gear,
    /// `d = radius · θ`.
    Rack,
    /// `d = lead · θ / 360°`.
    Screw,
    /// `Σ cᵢ · qᵢ = value` over the coupling's [`Coupling::terms`]: any
    /// linear tie of free motions, of any number of mates (a planetary
    /// stage, two sliders, a slider and a revolute).
    Linear,
}

impl CouplingKind {
    pub const ALL: [CouplingKind; 4] = [
        CouplingKind::Gear,
        CouplingKind::Rack,
        CouplingKind::Screw,
        CouplingKind::Linear,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CouplingKind::Gear => "gear",
            CouplingKind::Rack => "rack",
            CouplingKind::Screw => "screw",
            CouplingKind::Linear => "linear",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Mates whose motions are tied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coupling {
    /// Unique among the couplings.
    pub name: String,
    pub kind: CouplingKind,
    /// The two mates, by name; both empty for a linear coupling, which
    /// names its mates in its terms.
    pub mates: (String, String),
    /// The ratio, the radius or the lead; a linear coupling's right-hand
    /// side.
    pub value: Expr,
    /// A linear coupling's terms (none for the other kinds).
    #[serde(default)]
    pub terms: Vec<CouplingTerm>,
}

/// One term `coefficient · <mate>_<motion>` of a linear coupling.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CouplingTerm {
    pub mate: String,
    /// The free motion: `angle`, `distance`, `x` or `y` ([`MOTIONS`]).
    pub motion: String,
    /// Its coefficient, an expression whose product with the motion has
    /// the dimension of the coupling's value.
    pub coefficient: Expr,
}

impl Coupling {
    /// The mates it names, in order, each once.
    pub fn mate_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let all = [self.mates.0.as_str(), self.mates.1.as_str()]
            .into_iter()
            .chain(self.terms.iter().map(|t| t.mate.as_str()));
        for m in all {
            if !m.is_empty() && !out.contains(&m) {
                out.push(m);
            }
        }
        out
    }

    /// Renames the mate `old` to `new` wherever it names it.
    pub fn rename_mate(&mut self, old: &str, new: &str) {
        for m in [&mut self.mates.0, &mut self.mates.1] {
            if m == old {
                *m = new.to_string();
            }
        }
        for t in &mut self.terms {
            if t.mate == old {
                t.mate = new.to_string();
            }
        }
    }
}

/// Checks an instance's name: not empty, not the world, no `/`, no
/// control characters, not too long.
pub fn check_instance_name(name: &str) -> Result<(), DocError> {
    if name.is_empty() || name.trim() != name {
        return Err(DocError::Invalid(
            "an instance name must not be empty or start or end with a space".to_string(),
        ));
    }
    if name == WORLD {
        return Err(DocError::Invalid(format!(
            "`{WORLD}` is reserved for the world"
        )));
    }
    if name.contains('/') || name.chars().any(char::is_control) || name.len() > MAX_NAME_LEN {
        return Err(DocError::Invalid(format!(
            "an instance name must not contain `/` or control characters, nor be longer than {MAX_NAME_LEN} bytes"
        )));
    }
    Ok(())
}

/// Checks a mate's or a coupling's name: a valid variable name, so that
/// `<name>_angle` is one too.
pub fn check_mate_name(name: &str) -> Result<(), DocError> {
    craft3d_expr::check_variable_name(name).map_err(|e| {
        DocError::Invalid(format!("`{name}` cannot name a mate or a coupling: {e}"))
    })?;
    if name.len() > MAX_NAME_LEN.saturating_sub(16) {
        return Err(DocError::Invalid(format!(
            "the mate name `{name}` is too long"
        )));
    }
    Ok(())
}

fn check_hash(hash: &str) -> Result<(), DocError> {
    if hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(DocError::Invalid(format!(
            "`{}` is not a SHA-256 in lowercase hex",
            hash.chars().take(80).collect::<String>()
        )))
    }
}

/// Checks a source path: relative, `/` between the parts, no empty part.
/// `..` is allowed (a part beside the assembly's folder); the engine
/// checks the result against the files it may read.
pub fn check_source_path(path: &str) -> Result<(), DocError> {
    let bad = |why: &str| Err(DocError::Invalid(format!("source path `{path}`: {why}")));
    if path.is_empty() || path.len() > 4096 {
        return bad("it must not be empty or longer than 4096 bytes");
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return bad("it must be relative to the assembly's file, with `/` between the parts");
    }
    if path.split('/').any(|part| part.is_empty() || part == ".") {
        return bad("it has an empty or `.` part");
    }
    if path.chars().any(char::is_control) {
        return bad("it holds control characters");
    }
    Ok(())
}

impl Assembly {
    pub fn instance(&self, name: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.name == name)
    }

    pub fn mate(&self, name: &str) -> Option<&Mate> {
        self.mates.iter().find(|m| m.name == name)
    }

    pub fn coupling(&self, name: &str) -> Option<&Coupling> {
        self.couplings.iter().find(|c| c.name == name)
    }

    /// True when it holds no instance, mate or coupling: nothing worth
    /// keeping was added to it (its packed documents follow its instances).
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty() && self.mates.is_empty() && self.couplings.is_empty()
    }

    /// The packed document with this SHA-256.
    pub fn packed(&self, hash: &str) -> Option<&Blob> {
        self.packed.iter().find(|b| b.sha256() == hash)
    }

    /// Keeps `blob` among the packed documents, in order, once.
    pub fn pack(&mut self, blob: Blob) {
        let hash = blob.sha256();
        if self.packed(&hash).is_none() {
            let at = self.packed.partition_point(|b| b.sha256() < hash);
            self.packed.insert(at, blob);
        }
    }

    /// Drops the packed documents no instance uses: neither as its packed
    /// source nor as the bytes it is pinned to (a pinned instance keeps
    /// them, so it shows them whatever its file holds later).
    pub fn drop_unused_packed(&mut self) {
        let used: std::collections::BTreeSet<String> = self
            .instances
            .iter()
            .flat_map(|i| {
                let packed = match &i.source {
                    Source::Packed { hash } => Some(hash.clone()),
                    Source::File { .. } => None,
                };
                packed.into_iter().chain(i.pinned.clone())
            })
            .collect();
        self.packed.retain(|b| used.contains(&b.sha256()));
    }

    /// The rules of [`Document::validate`] for the assembly part.
    pub fn validate(&self) -> Result<(), DocError> {
        if self.instances.len() > MAX_INSTANCES
            || self.mates.len() > MAX_MATES
            || self.couplings.len() > MAX_COUPLINGS
        {
            return Err(DocError::Invalid(format!(
                "an assembly holds at most {MAX_INSTANCES} instances, {MAX_MATES} mates and {MAX_COUPLINGS} couplings"
            )));
        }
        let mut names = std::collections::BTreeSet::new();
        for instance in &self.instances {
            check_instance_name(&instance.name)?;
            if !names.insert(instance.name.as_str()) {
                return Err(DocError::Invalid(format!(
                    "two instances are named `{}`",
                    instance.name
                )));
            }
            match &instance.source {
                Source::File { path, hash } => {
                    check_source_path(path)?;
                    check_hash(hash)?;
                }
                Source::Packed { hash } => {
                    check_hash(hash)?;
                    if self.packed(hash).is_none() {
                        return Err(DocError::Invalid(format!(
                            "instance `{}` uses the packed document {hash}, which the file does not hold",
                            instance.name
                        )));
                    }
                }
            }
            if let Some(pinned) = &instance.pinned {
                check_hash(pinned)?;
            }
            if let Some(body) = &instance.body
                && (body.is_empty() || body.len() > MAX_NAME_LEN)
            {
                return Err(DocError::Invalid(format!(
                    "instance `{}` names its body by an empty or too long name",
                    instance.name
                )));
            }
            if instance.params.len() > MAX_OVERRIDES {
                return Err(DocError::Invalid(format!(
                    "instance `{}` sets more than {MAX_OVERRIDES} parameters",
                    instance.name
                )));
            }
            let mut params = std::collections::BTreeSet::new();
            for p in &instance.params {
                if p.name.is_empty() || p.name.len() > MAX_NAME_LEN || !params.insert(&p.name) {
                    return Err(DocError::Invalid(format!(
                        "instance `{}` sets the parameter `{}` twice or by an invalid name",
                        instance.name, p.name
                    )));
                }
            }
            if !instance.placement.is_finite() {
                return Err(DocError::Invalid(format!(
                    "instance `{}` has a placement that is not finite",
                    instance.name
                )));
            }
        }
        let mut mates = std::collections::BTreeSet::new();
        for mate in &self.mates {
            check_mate_name(&mate.name)?;
            if !mates.insert(mate.name.as_str()) {
                return Err(DocError::Invalid(format!(
                    "two mates are named `{}`",
                    mate.name
                )));
            }
            for r in [&mate.a, &mate.b] {
                r.check()?;
                if r.instance != WORLD && self.instance(&r.instance).is_none() {
                    return Err(DocError::Invalid(format!(
                        "mate `{}` names the instance `{}`, which the assembly does not have",
                        mate.name, r.instance
                    )));
                }
            }
            if mate.a.instance == mate.b.instance {
                return Err(DocError::Invalid(format!(
                    "mate `{}` joins `{}` to itself",
                    mate.name, mate.a.instance
                )));
            }
        }
        let mut couplings = std::collections::BTreeSet::new();
        for coupling in &self.couplings {
            check_mate_name(&coupling.name)?;
            if mates.contains(coupling.name.as_str()) || !couplings.insert(coupling.name.as_str()) {
                return Err(DocError::Invalid(format!(
                    "the coupling name `{}` is taken",
                    coupling.name
                )));
            }
            if coupling.kind == CouplingKind::Linear {
                if !coupling.mates.0.is_empty() || !coupling.mates.1.is_empty() {
                    return Err(DocError::Invalid(format!(
                        "linear coupling `{}` names its mates in its terms, not in mates",
                        coupling.name
                    )));
                }
                if coupling.terms.is_empty() || coupling.terms.len() > MAX_COUPLING_TERMS {
                    return Err(DocError::Invalid(format!(
                        "linear coupling `{}` needs 1 to {MAX_COUPLING_TERMS} terms",
                        coupling.name
                    )));
                }
                let mut seen = std::collections::BTreeSet::new();
                for term in &coupling.terms {
                    if !MOTIONS.contains(&term.motion.as_str()) {
                        return Err(DocError::Invalid(format!(
                            "linear coupling `{}`: `{}` is not a free motion; write angle, distance, x or y",
                            coupling.name,
                            term.motion.chars().take(40).collect::<String>()
                        )));
                    }
                    if !seen.insert((term.mate.as_str(), term.motion.as_str())) {
                        return Err(DocError::Invalid(format!(
                            "linear coupling `{}` names `{}_{}` twice",
                            coupling.name, term.mate, term.motion
                        )));
                    }
                }
            } else {
                if !coupling.terms.is_empty() {
                    return Err(DocError::Invalid(format!(
                        "coupling `{}`: only a linear coupling has terms",
                        coupling.name
                    )));
                }
                if coupling.mates.0 == coupling.mates.1 {
                    return Err(DocError::Invalid(format!(
                        "coupling `{}` couples a mate to itself",
                        coupling.name
                    )));
                }
            }
            for m in coupling.mate_names() {
                if self.mate(m).is_none() {
                    return Err(DocError::Invalid(format!(
                        "coupling `{}` names the mate `{m}`, which the assembly does not have",
                        coupling.name
                    )));
                }
            }
            if coupling.kind != CouplingKind::Linear
                && (coupling.mates.0.is_empty() || coupling.mates.1.is_empty())
            {
                return Err(DocError::Invalid(format!(
                    "coupling `{}` needs two mates",
                    coupling.name
                )));
            }
        }
        let mut last: Option<String> = None;
        for blob in &self.packed {
            let hash = blob.sha256();
            if last.as_ref().is_some_and(|l| *l >= hash) {
                return Err(DocError::Invalid(
                    "the packed documents are not in ascending order of their SHA-256, each once"
                        .to_string(),
                ));
            }
            last = Some(hash);
        }
        Ok(())
    }
}

impl Document {
    /// True for an assembly.
    pub fn is_assembly(&self) -> bool {
        self.assembly.is_some()
    }

    /// The document's assembly, or an error that says the document is a
    /// part.
    pub fn assembly(&self) -> Result<&Assembly, DocError> {
        self.assembly.as_ref().ok_or_else(|| {
            DocError::Invalid(
                "the document is a part, not an assembly (start one with assembly.new)".to_string(),
            )
        })
    }

    /// [`Document::assembly`], to change.
    pub fn assembly_mut(&mut self) -> Result<&mut Assembly, DocError> {
        self.assembly.as_mut().ok_or_else(|| {
            DocError::Invalid(
                "the document is a part, not an assembly (start one with assembly.new)".to_string(),
            )
        })
    }

    /// The rules an assembly document keeps: it has no features of its
    /// own, and its assembly part is valid.
    pub(crate) fn validate_assembly(&self) -> Result<(), DocError> {
        let Some(assembly) = &self.assembly else {
            return Ok(());
        };
        if !self.features.is_empty() {
            return Err(DocError::Invalid(
                "an assembly has no features of its own: its bodies are its instances'".to_string(),
            ));
        }
        assembly.validate()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::Units;

    fn hash() -> String {
        "ab".repeat(32)
    }

    fn instance(name: &str) -> Instance {
        Instance {
            name: name.to_string(),
            source: Source::File {
                path: "bar.3dcraft".to_string(),
                hash: hash(),
            },
            body: None,
            params: Vec::new(),
            pinned: None,
            mode: InstanceMode::Rigid,
            visible: true,
            colour: None,
            placement: Placement::default(),
        }
    }

    #[test]
    fn a_valid_assembly_and_its_rules() {
        let mut doc = Document::new(Units::default());
        let mut assembly = Assembly::default();
        assembly.instances.push(instance("Bar[1]"));
        assembly.instances.push(instance("Bar[2]"));
        assembly.mates.push(Mate {
            name: "Hinge".to_string(),
            kind: MateKind::Revolute,
            a: MateRef::parse("Bar[1]/F1:face(1)").unwrap(),
            b: MateRef::parse("Bar[2]/F1:face(2)").unwrap(),
            offset: None,
            angle: None,
            limits: None,
            flip: false,
        });
        doc.assembly = Some(assembly);
        doc.validate().unwrap();

        let mut twice = doc.clone();
        twice
            .assembly_mut()
            .unwrap()
            .instances
            .push(instance("Bar[1]"));
        assert!(twice.validate().is_err());

        let mut world = doc.clone();
        world
            .assembly_mut()
            .unwrap()
            .instances
            .push(instance(WORLD));
        assert!(world.validate().is_err());

        let mut dangling = doc.clone();
        dangling.assembly_mut().unwrap().mates[0].b = MateRef::parse("Gone/F1:face(1)").unwrap();
        assert!(dangling.validate().is_err());

        let mut packed = doc.clone();
        packed.assembly_mut().unwrap().instances[0].source = Source::Packed { hash: hash() };
        assert!(packed.validate().is_err());

        let mut bad_path = doc.clone();
        bad_path.assembly_mut().unwrap().instances[0].source = Source::File {
            path: "/etc/passwd".to_string(),
            hash: hash(),
        };
        assert!(bad_path.validate().is_err());
    }

    #[test]
    fn references_read_and_write_as_instance_slash_name() {
        let r = MateRef::parse("Wheel[2]/F3:face(5)").unwrap();
        assert_eq!(r.instance, "Wheel[2]");
        assert_eq!(r.name, "F3:face(5)");
        assert_eq!(r.to_string(), "Wheel[2]/F3:face(5)");
        assert!(MateRef::parse("no-slash").is_err());
        assert!(MateRef::parse("/F1:face(1)").is_err());
        assert!(MateRef::parse("A/").is_err());
    }

    #[test]
    fn packing_keeps_each_document_once_in_order() {
        let mut a = Assembly::default();
        a.pack(Blob::new(b"two".to_vec()));
        a.pack(Blob::new(b"one".to_vec()));
        a.pack(Blob::new(b"two".to_vec()));
        assert_eq!(a.packed.len(), 2);
        assert!(a.packed[0].sha256() < a.packed[1].sha256());
        a.drop_unused_packed();
        assert!(a.packed.is_empty());
        a.validate().unwrap();
    }

    #[test]
    fn the_bytes_an_instance_is_pinned_to_stay_packed() {
        let mut a = Assembly::default();
        let pinned = Blob::new(b"pinned".to_vec());
        a.pack(pinned.clone());
        a.pack(Blob::new(b"unused".to_vec()));
        a.instances.push(Instance {
            source: Source::File {
                path: "a.3dcraft".into(),
                hash: pinned.sha256(),
            },
            pinned: Some(pinned.sha256()),
            ..instance("A")
        });
        a.drop_unused_packed();
        assert_eq!(a.packed.len(), 1);
        assert!(a.packed(&pinned.sha256()).is_some());
        a.validate().unwrap();
    }
}
