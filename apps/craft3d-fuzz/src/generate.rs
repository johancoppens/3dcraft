//! The generator: from a seed, one group of commands at a time, each
//! chosen from what the document holds at that moment, so that operands
//! land on a grid and on the bodies' own vertices, faces and edges.
//! Coincident faces, shared edges, tangent and touching solids are then
//! the common case, not the rare one.
//!
//! Every length is a multiple of a quarter of the base size (`q`), or a
//! coordinate of a vertex of a body; directions are mostly the axes.
//! Nothing here knows how the kernel builds anything: it reads the
//! document through `document.inspect` and `document.edges`, as an agent
//! would.

use crate::rng::Rng;
use crate::runner::{Applied, Options, Runner};
use crate::steps::{Cmd, Design, Group, Setup, map_strings, symbolic_text};
use serde_json::{Value, json};

/// A body of the document, as `document.inspect` shows it.
#[derive(Clone, Debug)]
struct Body {
    /// The name of the feature that holds it.
    name: String,
    faces: Vec<String>,
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Body {
    /// The centre of its box.
    fn mid(&self) -> [f64; 3] {
        let mut c = self.lo;
        for (slot, hi) in c.iter_mut().zip(self.hi) {
            *slot = (*slot + hi) / 2.0;
        }
        c
    }
}

/// The edges (as the two faces each lies between) and vertices of a body.
#[derive(Clone, Debug, Default)]
struct Edges {
    pairs: Vec<(String, String)>,
    vertices: Vec<[f64; 3]>,
}

fn point(value: &Value) -> Option<[f64; 3]> {
    let a = value.as_array()?;
    let mut p = [0.0; 3];
    for (slot, v) in p.iter_mut().zip(a) {
        *slot = v.as_f64().filter(|x| x.is_finite())?;
    }
    (a.len() == 3).then_some(p)
}

fn bodies(runner: &mut Runner) -> Vec<Body> {
    let Some(inspect) = runner.query("document.inspect", &json!({})) else {
        return Vec::new();
    };
    inspect
        .get("bodies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|b| {
            let name = b.get("name")?.as_str()?.to_string();
            let faces = b
                .get("faceNames")?
                .as_array()?
                .iter()
                .filter_map(|f| f.as_str().map(str::to_string))
                .collect();
            let bbox = b.get("bbox")?.as_array()?;
            Some(Body {
                name,
                faces,
                lo: point(bbox.first()?)?,
                hi: point(bbox.get(1)?)?,
            })
        })
        .collect()
}

fn edges(runner: &mut Runner, body: &str) -> Edges {
    let Some(result) = runner.query("document.edges", &json!({ "feature": body })) else {
        return Edges::default();
    };
    let pairs = result
        .get("edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let faces = e.get("faces")?.as_array()?;
            let a = faces.first()?.as_str()?.to_string();
            let b = faces.get(1)?.as_str()?.to_string();
            (a != b).then_some((a, b))
        })
        .collect();
    let vertices = result
        .get("vertices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| point(v.get("point")?))
        .collect();
    Edges { pairs, vertices }
}

/// Whether the boxes of `a` and `b`, grown by `gap`, overlap.
fn boxes_meet(a: &Body, b: &Body, gap: f64) -> bool {
    (0..3).all(|i| {
        let (Some(alo), Some(ahi), Some(blo), Some(bhi)) =
            (a.lo.get(i), a.hi.get(i), b.lo.get(i), b.hi.get(i))
        else {
            return false;
        };
        alo - gap <= *bhi && blo - gap <= *ahi
    })
}

const AXES: [[f64; 3]; 6] = [
    [0.0, 0.0, 1.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, -1.0],
    [-1.0, 0.0, 0.0],
    [0.0, -1.0, 0.0],
];

pub struct Generator {
    rng: Rng,
    /// A quarter of the base size: the grid step.
    q: f64,
}

fn cmd(command: &str, params: Value) -> Cmd {
    Cmd {
        command: command.to_string(),
        params,
    }
}

impl Generator {
    pub fn new(setup: Setup) -> Generator {
        Generator {
            rng: Rng::new(setup.seed),
            q: setup.base() / 4.0,
        }
    }

    /// The scale a seed is built at: mostly 1, a quarter each at 1e-3 and
    /// 1e3.
    pub fn scale_of(seed: u64) -> f64 {
        match Rng::new(seed ^ 0x5ca1e).below(4) {
            0 => 1e-3,
            1 => 1e3,
            _ => 1.0,
        }
    }

    /// How many groups a seed's design has: 3 to 12.
    pub fn length_of(seed: u64) -> usize {
        3 + Rng::new(seed ^ 0x1e9).below(10)
    }

    /// `n` grid steps.
    fn g(&self, n: i64) -> f64 {
        n as f64 * self.q
    }

    fn grid(&mut self, lo: i64, hi: i64) -> f64 {
        let n = self.rng.range(lo, hi);
        self.g(n)
    }

    fn grid_point(&mut self) -> [f64; 3] {
        [self.grid(-4, 4), self.grid(-4, 4), self.grid(-4, 4)]
    }

    /// A point to put something at: a grid point, a vertex of a body, a
    /// corner or the centre of a body's box.
    fn anchor(&mut self, runner: &mut Runner, bodies: &[Body]) -> [f64; 3] {
        if bodies.is_empty() || self.rng.chance(35) {
            return self.grid_point();
        }
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return self.grid_point();
        };
        match self.rng.weighted(&[50, 30, 20]) {
            0 => {
                let e = edges(runner, &body.name);
                match self.rng.pick(&e.vertices) {
                    Some(v) => *v,
                    None => self.corner(&body),
                }
            }
            1 => self.corner(&body),
            _ => body.mid(),
        }
    }

    fn corner(&mut self, body: &Body) -> [f64; 3] {
        let bits = self.rng.below(8);
        let mut c = body.lo;
        for (i, (slot, hi)) in c.iter_mut().zip(body.hi).enumerate() {
            if bits >> i & 1 == 1 {
                *slot = hi;
            }
        }
        c
    }

    fn axis(&mut self) -> [f64; 3] {
        match self.rng.weighted(&[50, 15, 15, 5, 5, 5, 3, 2]) {
            i @ 0..=5 => AXES.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
            6 => [1.0, 1.0, 0.0],
            _ => [1.0, 1.0, 1.0],
        }
    }

    fn angle(&mut self) -> &'static str {
        match self.rng.weighted(&[75, 10, 10, 5]) {
            0 => "0 deg",
            1 => "90 deg",
            2 => "45 deg",
            _ => "30 deg",
        }
    }

    /// A small length for a blend, a wall or an offset: an eighth of a
    /// grid step to two.
    fn small(&mut self) -> f64 {
        let k = [0.25, 0.5, 1.0, 2.0];
        let i = self.rng.below(k.len());
        k.get(i).copied().unwrap_or(1.0) * self.q
    }

    /// The next group, chosen from what `runner`'s document holds.
    pub fn group(&mut self, runner: &mut Runner, k: usize) -> Group {
        let bodies = bodies(runner);
        let n = bodies.len();
        let weights: [u64; 12] = if n == 0 {
            [70, 20, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        } else {
            let two = if n >= 2 { 30 } else { 0 };
            [18, 14, 5, two, 14, 4, 6, 2, 5, 5, 6, 8]
        };
        let name = format!("N{k}");
        let sketch = format!("S{k}");
        let (step, commands) = match self.rng.weighted(&weights) {
            0 => ("primitive", self.primitive(runner, &bodies, &name)),
            1 => ("extrude", self.extrude(&bodies, &name, &sketch)),
            2 => ("revolve", self.revolve(&bodies, &name, &sketch)),
            3 => ("boolean", self.boolean(&bodies, &name)),
            4 => ("blend", self.blend(runner, &bodies, &name)),
            5 => ("shell", self.shell(&bodies, &name)),
            6 => ("offsetFace", self.offset(&bodies, &name)),
            7 => ("deleteFace", self.delete_face(&bodies, &name)),
            8 => ("pattern", self.pattern(runner, &bodies, &name)),
            9 => ("mirror", self.mirror(runner, &bodies, &name)),
            10 => ("transform", self.transform(runner, &bodies, &name)),
            _ => ("edit", self.edit(runner, &bodies, &name)),
        };
        let names = runner.names().clone();
        let commands = commands
            .into_iter()
            .map(|c| Cmd {
                params: map_strings(&c.params, &|s| symbolic_text(s, &names), 0),
                command: c.command,
            })
            .collect();
        Group {
            step: step.to_string(),
            commands,
        }
    }

    fn primitive(&mut self, runner: &mut Runner, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let kind = self.rng.weighted(&[35, 25, 12, 13, 15]);
        // Tangent to a face of a body's box, from outside or inside.
        if (kind == 1 || kind == 2)
            && let (true, Some(body)) = (self.rng.chance(35), self.rng.pick(bodies).cloned())
        {
            let r = self.grid(1, 4);
            // A cylinder stands along z, so it touches a side in x or y.
            let axis = self.rng.below(if kind == 1 { 2 } else { 3 });
            let toward_hi = self.rng.chance(50);
            let inside = self.rng.chance(30);
            let (lo, hi) = (body.lo.get(axis), body.hi.get(axis));
            let plane = if toward_hi { hi } else { lo }.copied().unwrap_or(0.0);
            // Outside: beyond the face by r; inside: short of it by r.
            let away = if toward_hi != inside { r } else { -r };
            let mut c = self.anchor(runner, std::slice::from_ref(&body));
            if let Some(slot) = c.get_mut(axis) {
                *slot = plane + away;
            }
            if kind == 1 {
                c[2] = body.lo[2];
                let h = body.hi[2] - body.lo[2];
                let height = if h > 0.0 && self.rng.chance(60) {
                    h
                } else {
                    self.grid(1, 8)
                };
                return vec![cmd(
                    "feature.cylinder",
                    json!({"radius": r, "height": height, "position": c, "name": name}),
                )];
            }
            return vec![cmd(
                "feature.sphere",
                json!({"radius": r, "position": c, "name": name}),
            )];
        }
        let mut p = self.anchor(runner, bodies);
        if self.rng.chance(30) {
            let d = self.grid_point();
            p = [p[0] + d[0] / 2.0, p[1] + d[1] / 2.0, p[2] + d[2] / 2.0];
        }
        let axis = self.axis();
        let angle = self.angle();
        match kind {
            0 => {
                let size = [self.grid(1, 8), self.grid(1, 8), self.grid(1, 8)];
                vec![cmd(
                    "feature.block",
                    json!({"size": size, "position": p, "axis": axis, "angle": angle, "name": name}),
                )]
            }
            1 => {
                let (r, h) = (self.grid(1, 4), self.grid(1, 8));
                vec![cmd(
                    "feature.cylinder",
                    json!({"radius": r, "height": h, "position": p, "axis": axis, "angle": angle, "name": name}),
                )]
            }
            2 => {
                let r = self.grid(1, 4);
                vec![cmd(
                    "feature.sphere",
                    json!({"radius": r, "position": p, "name": name}),
                )]
            }
            3 => {
                let (r, h) = (self.grid(1, 4), self.grid(1, 8));
                vec![cmd(
                    "feature.cone",
                    json!({"radius": r, "height": h, "position": p, "axis": axis, "angle": angle, "name": name}),
                )]
            }
            _ => {
                let major = self.rng.range(2, 6);
                let minor = self.rng.range(1, major - 1);
                vec![cmd(
                    "feature.torus",
                    json!({"major": self.g(major), "minor": self.g(minor), "position": p, "axis": axis, "angle": angle, "name": name}),
                )]
            }
        }
    }

    /// The commands that make a closed profile in `sketch`.
    fn profile(&mut self, sketch: &str, near_axis: bool) -> Cmd {
        match self.rng.weighted(&[55, 30, 15]) {
            0 => {
                let (x0, x1) = if near_axis {
                    let a = self.rng.range(0, 3);
                    (a, a + self.rng.range(1, 4))
                } else {
                    let a = self.rng.range(-4, 3);
                    (a, self.rng.range(a + 1, 4))
                };
                let y0 = self.rng.range(-4, 3);
                let y1 = self.rng.range(y0 + 1, 4);
                let pts =
                    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| [self.g(x), self.g(y)]);
                cmd(
                    "sketch.polyline",
                    json!({"sketch": sketch, "points": pts, "closed": true}),
                )
            }
            1 => {
                let r = self.rng.range(1, 3);
                let cx = if near_axis {
                    // At the radius from the axis it touches it.
                    r + self.rng.range(0, 2)
                } else {
                    self.rng.range(-3, 3)
                };
                let cy = self.rng.range(-3, 3);
                cmd(
                    "sketch.circle",
                    json!({"sketch": sketch, "center": [self.g(cx), self.g(cy)], "radius": self.g(r)}),
                )
            }
            _ => {
                // An L: a rectangle with a corner taken out.
                let x0 = if near_axis { 0 } else { self.rng.range(-4, 0) };
                let (x1, x2) = (x0 + self.rng.range(1, 2), x0 + 4);
                let y0 = self.rng.range(-4, 0);
                let (y1, y2) = (y0 + self.rng.range(1, 2), y0 + 4);
                let pts = [(x0, y0), (x2, y0), (x2, y1), (x1, y1), (x1, y2), (x0, y2)]
                    .map(|(x, y)| [self.g(x), self.g(y)]);
                cmd(
                    "sketch.polyline",
                    json!({"sketch": sketch, "points": pts, "closed": true}),
                )
            }
        }
    }

    /// `sketch.create` on a face of a body or on a world plane.
    fn sketch_plane(
        &mut self,
        bodies: &[Body],
        sketch: &str,
        world_only: bool,
    ) -> (Cmd, Option<String>) {
        if !world_only
            && self.rng.chance(60)
            && let Some(body) = self.rng.pick(bodies).cloned()
            && let Some(face) = self.rng.pick(&body.faces).cloned()
        {
            return (
                cmd("sketch.create", json!({"face": face, "name": sketch})),
                Some(body.name),
            );
        }
        let plane = ["xy", "yz", "zx"]
            .get(self.rng.below(3))
            .copied()
            .unwrap_or("xy");
        let offset = if self.rng.chance(50) {
            0.0
        } else {
            self.grid(-4, 4)
        };
        (
            cmd(
                "sketch.create",
                json!({"plane": plane, "offset": offset, "name": sketch}),
            ),
            None,
        )
    }

    /// `join`, `cut` or `intersect` with a body, or nothing.
    fn combine(&mut self, bodies: &[Body], prefer: Option<&str>, params: &mut Value) {
        let which = self.rng.weighted(&[45, 30, 18, 7]);
        if which == 0 || bodies.is_empty() {
            return;
        }
        let body = match prefer {
            Some(name) if self.rng.chance(75) => name.to_string(),
            _ => match self.rng.pick(bodies) {
                Some(b) => b.name.clone(),
                None => return,
            },
        };
        let key = ["", "cut", "join", "intersect"]
            .get(which)
            .copied()
            .unwrap_or("cut");
        if let Some(map) = params.as_object_mut() {
            map.insert(key.into(), body.into());
        }
    }

    fn extrude(&mut self, bodies: &[Body], name: &str, sketch: &str) -> Vec<Cmd> {
        let (create, on) = self.sketch_plane(bodies, sketch, false);
        let profile = self.profile(sketch, false);
        let mut distance = self.grid(1, 8);
        if self.rng.chance(20) {
            distance = -distance;
        }
        let mut params = json!({"sketch": sketch, "distance": distance, "name": name});
        self.combine(bodies, on.as_deref(), &mut params);
        if let Some(map) = params.as_object_mut() {
            if self.rng.chance(10) {
                map.insert("symmetric".into(), true.into());
            }
            if map.contains_key("cut") && self.rng.chance(30) {
                map.remove("distance");
                map.insert("throughAll".into(), true.into());
                map.insert("reversed".into(), true.into());
            }
        }
        vec![create, profile, cmd("feature.extrude", params)]
    }

    fn revolve(&mut self, bodies: &[Body], name: &str, sketch: &str) -> Vec<Cmd> {
        let (create, _) = self.sketch_plane(bodies, sketch, true);
        let profile = self.profile(sketch, true);
        let angle = match self.rng.weighted(&[60, 15, 15, 10]) {
            0 => "360 deg",
            1 => "90 deg",
            2 => "180 deg",
            _ => "270 deg",
        };
        let mut params = json!({"sketch": sketch, "axis": "y", "angle": angle, "name": name});
        self.combine(bodies, None, &mut params);
        vec![create, profile, cmd("feature.revolve", params)]
    }

    fn boolean(&mut self, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let a = self.rng.below(bodies.len());
        let Some(target) = bodies.get(a) else {
            return Vec::new();
        };
        // Mostly a tool whose box meets the target's (within a grid step):
        // apart, most booleans only say the result is empty.
        let near: Vec<&Body> = bodies
            .iter()
            .enumerate()
            .filter(|(i, b)| *i != a && boxes_meet(target, b, self.q))
            .map(|(_, b)| b)
            .collect();
        let tool = if !near.is_empty() && self.rng.chance(85) {
            self.rng.pick(&near).copied()
        } else {
            let mut b = self.rng.below(bodies.len().saturating_sub(1));
            if b >= a {
                b += 1;
            }
            bodies.get(b)
        };
        let Some(tool) = tool else {
            return Vec::new();
        };
        let op = ["union", "subtract", "intersect"]
            .get(self.rng.weighted(&[35, 40, 25]))
            .copied()
            .unwrap_or("union");
        let keep = self.rng.chance(20);
        vec![cmd(
            "feature.boolean",
            json!({"op": op, "target": target.name, "tool": tool.name, "keepTool": keep, "name": name}),
        )]
    }

    fn blend(&mut self, runner: &mut Runner, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        let e = edges(runner, &body.name);
        if e.pairs.is_empty() {
            // A sphere or a torus: nothing to blend; something else.
            return self.primitive(runner, bodies, name);
        }
        let count = 1 + self.rng.below(3);
        let mut chosen: Vec<[String; 2]> = Vec::new();
        for _ in 0..count {
            if let Some((a, b)) = self.rng.pick(&e.pairs).cloned() {
                let pair = [a, b];
                if !chosen.contains(&pair) {
                    chosen.push(pair);
                }
            }
        }
        let size = self.small();
        let chain = self.rng.chance(15);
        if self.rng.chance(55) {
            vec![cmd(
                "feature.fillet",
                json!({"edges": chosen, "radius": size, "chain": chain, "name": name}),
            )]
        } else {
            let mut params =
                json!({"edges": chosen, "distance": size, "chain": chain, "name": name});
            if let Some(map) = params.as_object_mut() {
                match self.rng.weighted(&[70, 20, 10]) {
                    1 => {
                        let second = self.small();
                        map.insert("second".into(), second.into());
                    }
                    2 => {
                        map.insert("angle".into(), "30 deg".into());
                    }
                    _ => {}
                }
            }
            vec![cmd("feature.chamfer", params)]
        }
    }

    fn faces_of(&mut self, body: &Body, most: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for _ in 0..most {
            if let Some(f) = self.rng.pick(&body.faces).cloned()
                && !out.contains(&f)
            {
                out.push(f);
            }
        }
        out
    }

    fn shell(&mut self, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        let open = match self.rng.weighted(&[30, 55, 15]) {
            0 => Vec::new(),
            1 => self.faces_of(&body, 1),
            _ => self.faces_of(&body, 2),
        };
        let thickness = self.small() / 2.0;
        vec![cmd(
            "feature.shell",
            json!({"source": body.name, "thickness": thickness, "open": open, "name": name}),
        )]
    }

    fn offset(&mut self, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        let most = if self.rng.chance(85) { 1 } else { 2 };
        let faces = self.faces_of(&body, most);
        let mut distance = self.small();
        if self.rng.chance(45) {
            distance = -distance;
        }
        vec![cmd(
            "feature.offsetFace",
            json!({"faces": faces, "distance": distance, "name": name}),
        )]
    }

    fn delete_face(&mut self, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        let faces = self.faces_of(&body, 1);
        vec![cmd(
            "feature.deleteFace",
            json!({"faces": faces, "name": name}),
        )]
    }

    /// A step along an axis: the body's own extent (instances touch), half
    /// of it (they overlap) or one and a half (a gap), on the grid.
    fn step_along(&mut self, body: &Body, axis: usize) -> f64 {
        let extent = body
            .hi
            .get(axis)
            .zip(body.lo.get(axis))
            .map_or(0.0, |(h, l)| h - l);
        let k = [1.0, 0.5, 1.5]
            .get(self.rng.weighted(&[50, 30, 20]))
            .copied()
            .unwrap_or(1.0);
        let quarter = self.q / 4.0;
        let snapped = (extent * k / quarter).round() * quarter;
        if snapped.is_finite() && snapped > 0.0 {
            snapped
        } else {
            self.q
        }
    }

    fn pattern(&mut self, runner: &mut Runner, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        if self.rng.chance(60) {
            let axis = self.rng.below(3);
            let length = self.step_along(&body, axis);
            let mut step = [0.0; 3];
            if let Some(slot) = step.get_mut(axis) {
                *slot = length;
            }
            let count = self.rng.range(2, 4);
            return vec![cmd(
                "feature.pattern",
                json!({"source": body.name, "count": count, "step": step, "name": name}),
            )];
        }
        let count = self.rng.range(2, 6);
        let angle = match self.rng.weighted(&[70, 15, 15]) {
            0 => format!("360 deg / {count}"),
            1 => "90 deg".to_string(),
            _ => "45 deg".to_string(),
        };
        let axis = if self.rng.chance(75) {
            [0.0, 0.0, 1.0]
        } else {
            self.axis()
        };
        let p = self.anchor(runner, std::slice::from_ref(&body));
        vec![cmd(
            "feature.pattern",
            json!({"source": body.name, "count": count, "angle": angle, "axis": axis, "point": p, "name": name}),
        )]
    }

    fn mirror(&mut self, runner: &mut Runner, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        let normal = AXES
            .get(self.rng.below(3))
            .copied()
            .unwrap_or([0.0, 0.0, 1.0]);
        let p = match self.rng.weighted(&[60, 20, 20]) {
            0 => self.anchor(runner, std::slice::from_ref(&body)),
            1 => body.mid(),
            _ => self.grid_point(),
        };
        let keep = self.rng.chance(75);
        vec![cmd(
            "feature.mirror",
            json!({"source": body.name, "normal": normal, "point": p, "keep": keep, "name": name}),
        )]
    }

    fn transform(&mut self, runner: &mut Runner, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(body) = self.rng.pick(bodies).cloned() else {
            return Vec::new();
        };
        let translate = if self.rng.chance(50) {
            let axis = self.rng.below(3);
            let mut t = [0.0; 3];
            let length = self.step_along(&body, axis);
            if let Some(slot) = t.get_mut(axis) {
                *slot = if self.rng.chance(50) { length } else { -length };
            }
            t
        } else {
            let p = self.grid_point();
            [p[0] / 2.0, p[1] / 2.0, p[2] / 2.0]
        };
        let mut params = json!({"source": body.name, "translate": translate, "keep": self.rng.chance(60), "name": name});
        if self.rng.chance(30) {
            let angle = ["90 deg", "45 deg", "180 deg"]
                .get(self.rng.below(3))
                .copied()
                .unwrap_or("90 deg");
            let axis = AXES
                .get(self.rng.below(3))
                .copied()
                .unwrap_or([0.0, 0.0, 1.0]);
            let pivot = self.anchor(runner, std::slice::from_ref(&body));
            if let Some(map) = params.as_object_mut() {
                map.insert("angle".into(), angle.into());
                map.insert("axis".into(), json!(axis));
                map.insert("pivot".into(), json!(pivot));
            }
        }
        vec![cmd("feature.transform", params)]
    }
}

impl Generator {
    /// A new value for one parameter of an earlier feature, so that every
    /// feature after it is rebuilt from a changed input: a size, a
    /// position, a distance, a radius, a thickness, moved by grid steps.
    fn edit(&mut self, runner: &mut Runner, bodies: &[Body], name: &str) -> Vec<Cmd> {
        let Some(inspect) = runner.query("document.inspect", &json!({})) else {
            return self.primitive(runner, bodies, name);
        };
        let editable: Vec<(String, String, Value)> = inspect
            .get("features")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let feature = f.get("name")?.as_str()?.to_string();
                let params = f.get("params")?;
                let kind = params.get("type")?.as_str()?.to_string();
                Some((feature, kind, params.clone()))
            })
            .filter(|(_, kind, _)| {
                matches!(
                    kind.as_str(),
                    "block"
                        | "cylinder"
                        | "sphere"
                        | "cone"
                        | "torus"
                        | "extrude"
                        | "fillet"
                        | "chamfer"
                        | "shell"
                        | "offsetFace"
                        | "transform"
                )
            })
            .collect();
        let Some((feature, kind, params)) = self.rng.pick(&editable).cloned() else {
            return self.primitive(runner, bodies, name);
        };
        let number = |v: Option<&Value>| -> Option<f64> {
            let v = v?;
            v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
        };
        let shift = |g: &mut Generator, at: Option<&Value>| -> Option<Value> {
            let a = at?.as_array()?;
            let mut p = Vec::with_capacity(3);
            for v in a {
                p.push(number(Some(v))? + g.grid(-2, 2) / 2.0);
            }
            Some(json!(p))
        };
        let mut change = serde_json::Map::new();
        match kind.as_str() {
            "block" => {
                if self.rng.chance(50) {
                    let size = [self.grid(1, 8), self.grid(1, 8), self.grid(1, 8)];
                    change.insert("size".into(), json!(size));
                } else if let Some(p) = shift(self, params.get("position")) {
                    change.insert("position".into(), p);
                }
            }
            "cylinder" | "cone" | "sphere" => {
                match self.rng.below(if kind == "sphere" { 2 } else { 3 }) {
                    0 => {
                        let r = self.grid(1, 4);
                        change.insert("radius".into(), r.into());
                    }
                    1 => {
                        if let Some(p) = shift(self, params.get("position")) {
                            change.insert("position".into(), p);
                        }
                    }
                    _ => {
                        let h = self.grid(1, 8);
                        change.insert("height".into(), h.into());
                    }
                }
            }
            "torus" => {
                let r = self.grid(1, 2);
                change.insert("minor".into(), r.into());
            }
            "extrude" => {
                let d = self.grid(1, 8);
                change.insert("distance".into(), d.into());
            }
            "fillet" => {
                let r = self.small();
                change.insert("radius".into(), r.into());
            }
            "chamfer" | "offsetFace" => {
                let mut d = self.small();
                if kind == "offsetFace" && self.rng.chance(45) {
                    d = -d;
                }
                change.insert("distance".into(), d.into());
            }
            "shell" => {
                let t = self.small() / 2.0;
                change.insert("thickness".into(), t.into());
            }
            _ => {
                if let Some(p) = shift(self, params.get("translate")) {
                    change.insert("translate".into(), p);
                }
            }
        }
        if change.is_empty() {
            return self.primitive(runner, bodies, name);
        }
        change.insert("feature".into(), feature.into());
        vec![cmd("feature.edit", Value::Object(change))]
    }
}

/// Generates and runs the design of a seed, group by group; `before` is
/// told every group before it runs (the runner outside keeps them, so that
/// a process that dies still leaves the design that killed it).
pub fn run_seed(
    setup: Setup,
    options: Options,
    before: &mut dyn FnMut(&Design),
) -> Result<(Design, Runner), String> {
    let mut runner = Runner::new(setup, options)?;
    let mut generator = Generator::new(setup);
    let mut design = Design {
        setup,
        groups: Vec::new(),
    };
    for k in 0..Generator::length_of(setup.seed) {
        let group = generator.group(&mut runner, k);
        design.groups.push(group.clone());
        before(&design);
        if runner.apply(&group) == Applied::Found {
            break;
        }
    }
    runner.finish();
    Ok((design, runner))
}
