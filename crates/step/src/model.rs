//! The file model: records with parameters, entity instances, a file.

use std::collections::BTreeMap;

/// One parameter of a record.
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    /// `$`: an optional attribute that is not given.
    Omitted,
    /// `*`: an attribute a subtype derives.
    Derived,
    Integer(i64),
    Real(f64),
    String(String),
    /// `.NAME.`, without the dots. Booleans and logicals are the
    /// enumerations `T`, `F` and `U`.
    Enum(String),
    /// `"…"`: the text between the quotes, hexadecimal digits of which the
    /// first says how many bits of the second are unused.
    Binary(String),
    /// `#id`: a reference to an entity instance.
    Ref(u64),
    List(Vec<Param>),
    /// `KEYWORD(parameter)`: a value with the name of its type, as select
    /// types need.
    Typed(String, Box<Param>),
}

impl Param {
    /// `.T.` or `.F.`.
    pub fn boolean(value: bool) -> Param {
        Param::Enum(if value { "T" } else { "F" }.to_string())
    }

    pub fn enumeration(name: &str) -> Param {
        Param::Enum(name.to_string())
    }

    pub fn string(text: &str) -> Param {
        Param::String(text.to_string())
    }

    pub fn typed(keyword: &str, value: Param) -> Param {
        Param::Typed(keyword.to_string(), Box::new(value))
    }

    pub fn refs(ids: impl IntoIterator<Item = u64>) -> Param {
        Param::List(ids.into_iter().map(Param::Ref).collect())
    }

    pub fn reals(values: impl IntoIterator<Item = f64>) -> Param {
        Param::List(values.into_iter().map(Param::Real).collect())
    }

    pub fn as_ref(&self) -> Option<u64> {
        match self {
            Param::Ref(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_real(&self) -> Option<f64> {
        match self {
            Param::Real(x) => Some(*x),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Param::Integer(x) => Some(*x),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Param::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<&str> {
        match self {
            Param::Enum(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Param]> {
        match self {
            Param::List(items) => Some(items),
            _ => None,
        }
    }

    /// Calls `visit` with every instance this parameter refers to, however
    /// deeply nested. Iterative: the depth of the parameter does not
    /// matter.
    pub fn for_each_ref(&self, visit: &mut impl FnMut(u64)) {
        let mut stack = vec![self];
        while let Some(param) = stack.pop() {
            match param {
                Param::Ref(id) => visit(*id),
                Param::List(items) => stack.extend(items.iter().rev()),
                Param::Typed(_, inner) => stack.push(inner),
                _ => {}
            }
        }
    }
}

/// A keyword with its parameters: `PLANE('', #12)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub keyword: String,
    pub params: Vec<Param>,
}

impl Record {
    pub fn new(keyword: &str, params: Vec<Param>) -> Self {
        Self {
            keyword: keyword.to_string(),
            params,
        }
    }
}

/// What an instance is: one record, or the records of a complex instance
/// (an instance of several leaf types at once), in the order they stand in
/// the file. The standard asks for alphabetical order; neither [`crate::write`]
/// nor [`crate::read`] reorders them.
#[derive(Clone, Debug, PartialEq)]
pub enum Entity {
    Simple(Record),
    Complex(Vec<Record>),
}

impl Entity {
    /// The records: one for a simple instance.
    pub fn records(&self) -> &[Record] {
        match self {
            Entity::Simple(record) => std::slice::from_ref(record),
            Entity::Complex(records) => records,
        }
    }

    /// The record with this keyword, if the instance has one.
    pub fn record(&self, keyword: &str) -> Option<&Record> {
        self.records().iter().find(|r| r.keyword == keyword)
    }

    /// Whether the instance is (also) of this type.
    pub fn is(&self, keyword: &str) -> bool {
        self.record(keyword).is_some()
    }

    /// Calls `visit` with every instance this one refers to.
    pub fn for_each_ref(&self, visit: &mut impl FnMut(u64)) {
        for record in self.records() {
            for param in &record.params {
                param.for_each_ref(visit);
            }
        }
    }
}

/// `#id = …;`
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub id: u64,
    pub entity: Entity,
}

/// The three header entities every file has (clause 8.2), as values.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Header {
    /// `FILE_DESCRIPTION.description`.
    pub description: Vec<String>,
    /// `FILE_DESCRIPTION.implementation_level`; `2;1` for a file that
    /// follows edition 2 without the optional sections.
    pub implementation_level: String,
    /// `FILE_NAME.name`.
    pub name: String,
    /// `FILE_NAME.time_stamp`: ISO 8601, or empty.
    pub time_stamp: String,
    pub author: Vec<String>,
    pub organization: Vec<String>,
    pub preprocessor_version: String,
    pub originating_system: String,
    pub authorization: String,
    /// `FILE_SCHEMA`: the schemas the data section is an instance of.
    pub schemas: Vec<String>,
}

fn strings(items: &[String]) -> Param {
    // An empty list of strings is not allowed for these attributes: the
    // convention is one empty string.
    if items.is_empty() {
        Param::List(vec![Param::String(String::new())])
    } else {
        Param::List(items.iter().map(|s| Param::String(s.clone())).collect())
    }
}

fn strings_of(param: Option<&Param>) -> Option<Vec<String>> {
    param?
        .as_list()?
        .iter()
        .map(|p| p.as_str().map(str::to_string))
        .collect()
}

impl Header {
    /// The header as the records `FILE_DESCRIPTION`, `FILE_NAME` and
    /// `FILE_SCHEMA`, in that order.
    pub fn records(&self) -> Vec<Record> {
        vec![
            Record::new(
                "FILE_DESCRIPTION",
                vec![
                    strings(&self.description),
                    Param::string(&self.implementation_level),
                ],
            ),
            Record::new(
                "FILE_NAME",
                vec![
                    Param::string(&self.name),
                    Param::string(&self.time_stamp),
                    strings(&self.author),
                    strings(&self.organization),
                    Param::string(&self.preprocessor_version),
                    Param::string(&self.originating_system),
                    Param::string(&self.authorization),
                ],
            ),
            Record::new("FILE_SCHEMA", vec![strings(&self.schemas)]),
        ]
    }

    /// The header values of a file's header records; `None` when one of
    /// the three records is missing or does not have the parameters the
    /// standard gives it.
    pub fn from_records(records: &[Record]) -> Option<Header> {
        let find = |keyword: &str| records.iter().find(|r| r.keyword == keyword);
        let description = find("FILE_DESCRIPTION")?;
        let name = find("FILE_NAME")?;
        let schema = find("FILE_SCHEMA")?;
        let text = |record: &Record, at: usize| {
            record
                .params
                .get(at)
                .and_then(Param::as_str)
                .map(str::to_string)
        };
        Some(Header {
            description: strings_of(description.params.first())?,
            implementation_level: text(description, 1)?,
            name: text(name, 0)?,
            time_stamp: text(name, 1)?,
            author: strings_of(name.params.get(2))?,
            organization: strings_of(name.params.get(3))?,
            preprocessor_version: text(name, 4)?,
            originating_system: text(name, 5)?,
            authorization: text(name, 6)?,
            schemas: strings_of(schema.params.first())?,
        })
    }
}

/// A whole exchange file: the header records and the instances of the one
/// data section, in file order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StepFile {
    pub header: Vec<Record>,
    pub data: Vec<Instance>,
}

impl StepFile {
    pub fn new(header: &Header) -> Self {
        Self {
            header: header.records(),
            data: Vec::new(),
        }
    }

    /// The id the next added instance gets: one more than the largest so
    /// far.
    fn next_id(&self) -> u64 {
        self.data
            .iter()
            .map(|i| i.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Adds a simple instance and returns its id. Ids count up from 1 in
    /// the order of adding.
    pub fn add(&mut self, keyword: &str, params: Vec<Param>) -> u64 {
        self.add_entity(Entity::Simple(Record::new(keyword, params)))
    }

    /// Adds a complex instance and returns its id.
    pub fn add_complex(&mut self, records: Vec<Record>) -> u64 {
        self.add_entity(Entity::Complex(records))
    }

    fn add_entity(&mut self, entity: Entity) -> u64 {
        // Appending keeps `data` sorted when it was built by `add` alone,
        // so the last id is the largest and this is O(1) then.
        let id = match self.data.last() {
            Some(last) if self.data.len() as u64 == last.id => last.id.saturating_add(1),
            _ => self.next_id(),
        };
        self.data.push(Instance { id, entity });
        id
    }

    /// The header as values, when it has the three standard records.
    pub fn header_values(&self) -> Option<Header> {
        Header::from_records(&self.header)
    }

    /// A lookup from id to instance. When an id occurs twice (which
    /// [`crate::read`] refuses) the first one wins.
    pub fn index(&self) -> BTreeMap<u64, &Instance> {
        let mut index = BTreeMap::new();
        for instance in &self.data {
            index.entry(instance.id).or_insert(instance);
        }
        index
    }

    /// The first reference to an instance that does not exist, as
    /// `(from, to)`.
    pub fn unresolved_reference(&self) -> Option<(u64, u64)> {
        let index = self.index();
        for instance in &self.data {
            let mut missing = None;
            instance.entity.for_each_ref(&mut |to| {
                if missing.is_none() && !index.contains_key(&to) {
                    missing = Some(to);
                }
            });
            if let Some(to) = missing {
                return Some((instance.id, to));
            }
        }
        None
    }

    /// A cycle of references, if there is one: ids of instances each of
    /// which refers to the next, and the last to the first.
    ///
    /// Part 21 itself does not forbid cycles, but the instances of a B-rep
    /// form a tree-like graph (a solid refers to shells, a shell to faces,
    /// …), and a reader that follows references recursively must know
    /// there is no cycle before it starts. Iterative, so a long chain of
    /// references is not a deep recursion. References to instances that do
    /// not exist are ignored here.
    pub fn reference_cycle(&self) -> Option<Vec<u64>> {
        let index = self.index();
        // 0 = not visited, 1 = on the current path, 2 = done.
        let mut state: BTreeMap<u64, u8> = BTreeMap::new();
        for start in &self.data {
            if state.get(&start.id).copied().unwrap_or(0) != 0 {
                continue;
            }
            // The path, each with the references still to follow.
            let mut path: Vec<(u64, Vec<u64>)> = Vec::new();
            let targets = |id: u64| {
                let mut out = Vec::new();
                if let Some(instance) = index.get(&id) {
                    instance.entity.for_each_ref(&mut |to| out.push(to));
                }
                out.reverse();
                out
            };
            state.insert(start.id, 1);
            path.push((start.id, targets(start.id)));
            while let Some((id, pending)) = path.last_mut() {
                let id = *id;
                let Some(next) = pending.pop() else {
                    state.insert(id, 2);
                    path.pop();
                    continue;
                };
                if !index.contains_key(&next) {
                    continue;
                }
                match state.get(&next).copied().unwrap_or(0) {
                    0 => {
                        state.insert(next, 1);
                        path.push((next, targets(next)));
                    }
                    1 => {
                        let from = path.iter().position(|(p, _)| *p == next).unwrap_or(0);
                        return Some(path.iter().skip(from).map(|(p, _)| *p).collect());
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// How many instances are (also) of each type, by keyword.
    pub fn counts(&self) -> BTreeMap<&str, usize> {
        let mut counts = BTreeMap::new();
        for instance in &self.data {
            for record in instance.entity.records() {
                *counts.entry(record.keyword.as_str()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// How many instances are (also) of this type.
    pub fn count(&self, keyword: &str) -> usize {
        self.data.iter().filter(|i| i.entity.is(keyword)).count()
    }
}
