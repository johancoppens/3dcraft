//! The area, volume and centroid of a body, measured once.
//!
//! Measuring a body with spline faces exactly takes seconds
//! (`craft3d_topo::mass_properties`), and the same body is measured by
//! several things in a row: the scene (the model's size picks the display
//! tolerance), `document.inspect`, `document.validate`, an export's
//! automatic tolerance. A body is immutable once regeneration made it and
//! shared by `Arc` from then on (also by later regenerations that reuse
//! it), so its measure is remembered by the body itself: the `Arc`'s
//! address, held by a `Weak` so the address cannot be taken by another
//! body while the entry exists. The answer is the very value a fresh
//! measure gives; only the time differs.

use craft3d_math::Vec3;
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{
    Body, Inertia, MassError, MassProperties, ShellVolume, inertia, mass_and_inertia,
    mass_with_shells, mass_with_shells_each_while,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Most bodies remembered; past it the bodies that are gone are
/// forgotten, and then the oldest. Before 2026-10-21 it was 256, and a
/// document of more bodies than that (the Garabit pier has 307) measured
/// every body again for each `document.inspect`: the store forgot the
/// first bodies while it measured the last ones. Fewer in a browser.
#[cfg(not(target_arch = "wasm32"))]
const MAX_REMEMBERED: usize = 1 << 14;
#[cfg(target_arch = "wasm32")]
const MAX_REMEMBERED: usize = 1 << 10;

/// Every this many bodies remembered, the bodies that are gone are
/// forgotten: their entries (and the `Arc` headers their `Weak`s keep)
/// do not wait for the store to be full.
const SWEEP_EVERY: u64 = 64;

type Measured = Result<MassProperties, MassError>;
type Turned = Result<Inertia, MassError>;

/// Remembered answers, by the address of the body's `Arc` (which the
/// `Weak` keeps from being taken by another body while the entry
/// exists), with when each was remembered.
struct Store<T> {
    by_body: BTreeMap<usize, (Weak<Body>, u64, T)>,
    next: u64,
}

impl<T> Store<T> {
    const fn new() -> Self {
        Store {
            by_body: BTreeMap::new(),
            next: 0,
        }
    }
}

static REMEMBERED: Mutex<Store<Measured>> = Mutex::new(Store::new());
static REMEMBERED_INERTIA: Mutex<Store<Turned>> = Mutex::new(Store::new());
/// What each shell of a body of several shells encloses, measured with
/// the body ([`shells_of`]).
static REMEMBERED_SHELLS: Mutex<Store<Shells>> = Mutex::new(Store::new());

/// The key of `body` in a [`Store`].
fn address(body: &Arc<Body>) -> usize {
    Arc::as_ptr(body) as usize
}

type Shells = Vec<Result<ShellVolume, MassError>>;

thread_local! {
    /// Bodies this thread measured afresh ([`fresh_measures`]).
    static FRESH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread integrated a body (for the mass, the
/// inertia or both at once) rather than finding the answer remembered:
/// for tests that hold a command to measuring nothing.
#[cfg(test)]
pub(crate) fn fresh_measures() -> usize {
    FRESH.with(std::cell::Cell::get)
}

fn counted() {
    FRESH.with(|fresh| fresh.set(fresh.get().saturating_add(1)));
}

/// What `store` remembers of `body`.
fn recall<T: Clone>(store: &Mutex<Store<T>>, body: &Arc<Body>) -> Option<T> {
    let key = Arc::downgrade(body);
    let remembered = store.lock().unwrap_or_else(PoisonError::into_inner);
    remembered
        .by_body
        .get(&address(body))
        .filter(|(seen, _, _)| seen.ptr_eq(&key))
        .map(|(_, _, value)| value.clone())
}

/// Remembers `value` for `body` in `store`, unless it is there already.
fn remember<T>(store: &Mutex<Store<T>>, body: &Arc<Body>, value: T) {
    let key = Arc::downgrade(body);
    let mut remembered = store.lock().unwrap_or_else(PoisonError::into_inner);
    let at = address(body);
    if remembered
        .by_body
        .get(&at)
        .is_some_and(|(seen, _, _)| seen.ptr_eq(&key))
    {
        return;
    }
    if remembered.next.is_multiple_of(SWEEP_EVERY) || remembered.by_body.len() >= MAX_REMEMBERED {
        // Bodies that are gone are forgotten.
        remembered
            .by_body
            .retain(|_, (seen, _, _)| seen.strong_count() > 0);
    }
    if remembered.by_body.len() >= MAX_REMEMBERED {
        // Then the oldest quarter at once, so a full store is not scanned
        // for every body it takes.
        let mut whens: Vec<u64> = remembered
            .by_body
            .values()
            .map(|(_, when, _)| *when)
            .collect();
        whens.sort_unstable();
        let cut = whens.get(MAX_REMEMBERED / 4).copied().unwrap_or(u64::MAX);
        remembered.by_body.retain(|_, (_, when, _)| *when >= cut);
    }
    let when = remembered.next;
    remembered.next = when.saturating_add(1);
    remembered.by_body.insert(at, (key, when, value));
}

/// The corners of the box around a body's mesh, with the chordal
/// tolerance it was meshed at (its bits).
type Boxed = (u64, Option<(Vec3, Vec3)>);

static REMEMBERED_BOXES: Mutex<Store<Boxed>> = Mutex::new(Store::new());

/// The corners of the box around `body` meshed at `params` (what
/// `document.inspect` lists as `bbox`; `None` when it cannot be meshed),
/// worked out once per body and chordal tolerance: the last tolerance is
/// remembered, which stays the same while no body's area changes.
pub(crate) fn mesh_box_of(body: &Arc<Body>, params: &TessParams) -> Option<(Vec3, Vec3)> {
    let tolerance = params.chordal_tolerance.to_bits();
    if let Some((at, corners)) = recall(&REMEMBERED_BOXES, body)
        && at == tolerance
    {
        return corners;
    }
    let corners = tessellate_body(body, params)
        .ok()
        .and_then(|mesh| mesh.bbox().corners());
    forget(&REMEMBERED_BOXES, body);
    remember(&REMEMBERED_BOXES, body, (tolerance, corners));
    corners
}

/// Forgets what `store` remembers of `body`.
fn forget<T>(store: &Mutex<Store<T>>, body: &Arc<Body>) {
    let mut remembered = store.lock().unwrap_or_else(PoisonError::into_inner);
    drop(remembered.by_body.remove(&address(body)));
}

/// `mass_properties(body)`, measured at most once per body. The lock is
/// not held while measuring: two threads that ask at once both measure.
pub(crate) fn mass_of(body: &Arc<Body>) -> Measured {
    if let Some(measured) = recall(&REMEMBERED, body) {
        return measured;
    }
    counted();
    let (measured, shells) = mass_with_shells(body);
    remember(&REMEMBERED, body, measured.clone());
    if body.shells.len() > 1 {
        remember(&REMEMBERED_SHELLS, body, shells);
    }
    measured
}

/// What each shell of `body` encloses (`craft3d_topo::mass_with_shells`),
/// remembered with the body's measure: the faces are integrated once for
/// both, when the body was first measured.
pub(crate) fn shells_of(body: &Arc<Body>) -> Shells {
    if let Some(shells) = recall(&REMEMBERED_SHELLS, body) {
        return shells;
    }
    counted();
    let (measured, shells) = mass_with_shells(body);
    remember(&REMEMBERED, body, measured);
    remember(&REMEMBERED_SHELLS, body, shells.clone());
    shells
}

/// [`mass_of`] of each of `bodies`, ahead of asking for them one by one:
/// the bodies not measured yet are measured at once, the faces of a run of
/// them shared out over the threads (`craft3d_topo::mass_properties_each`,
/// each the value `mass_properties` gives), so that a body's slowest face
/// does not leave the other threads waiting while the next body is not
/// started. Each is remembered as [`mass_of`] would remember it. Nothing
/// after the first body that cannot be measured (measured now or before)
/// is measured, as a command that stops there would leave it: `false`
/// then.
pub(crate) fn measure_ahead(bodies: &[&Arc<Body>]) -> bool {
    let mut fresh: Vec<&Arc<Body>> = Vec::new();
    let mut fine = true;
    for &body in bodies {
        match recall(&REMEMBERED, body) {
            Some(Err(_)) => {
                fine = false;
                break;
            }
            Some(Ok(_)) => {}
            None => {
                if !fresh.iter().any(|known| Arc::ptr_eq(known, body)) {
                    fresh.push(body);
                }
            }
        }
    }
    // Room left in the store for what the bodies are asked for next.
    fresh.truncate(MAX_REMEMBERED / 2);
    if fresh.len() < 2 {
        return fine;
    }
    let plain: Vec<&Body> = fresh.iter().map(|body| body.as_ref()).collect();
    for (body, measured) in fresh
        .iter()
        .zip(mass_with_shells_each_while(&plain, |_| true))
    {
        let Some((measured, shells)) = measured else {
            return false;
        };
        fine &= measured.is_ok();
        counted();
        remember(&REMEMBERED, body, measured);
        if body.shells.len() > 1 {
            remember(&REMEMBERED_SHELLS, body, shells);
        }
    }
    fine
}

/// `inertia(body)` and `mass_properties(body)`, each measured at most once
/// per body like [`mass_of`]: only bodies with a material are asked for
/// the inertia, and summing the second moments of a body with spline
/// faces takes as long as its volume. Asked first, the two are measured
/// at once (`craft3d_topo::mass_and_inertia`, each the value it is on its
/// own), in about the time of the inertia alone.
pub(crate) fn mass_and_inertia_of(body: &Arc<Body>) -> (Measured, Turned) {
    if let Some(turned) = recall(&REMEMBERED_INERTIA, body) {
        return (mass_of(body), turned);
    }
    counted();
    let (measured, turned) = match recall(&REMEMBERED, body) {
        Some(measured) => (measured, inertia(body)),
        None => mass_and_inertia(body),
    };
    remember(&REMEMBERED, body, measured.clone());
    remember(&REMEMBERED_INERTIA, body, turned.clone());
    (measured, turned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Frame;
    use craft3d_topo::mass_properties;

    fn block(dx: f64) -> Arc<Body> {
        Arc::new(
            craft3d_testkit::solids::block(&Frame::WORLD, dx, 2.0, 3.0)
                .unwrap()
                .body,
        )
    }

    #[test]
    fn a_body_is_measured_once_and_gives_what_a_fresh_measure_gives() {
        let body = block(1.0);
        let first = mass_of(&body);
        assert_eq!(first, mass_properties(&body));
        assert_eq!(mass_of(&body), first);
        // Another body of another size is not taken for it.
        let other = block(2.0);
        assert_ne!(mass_of(&other), first);
        assert_eq!(mass_of(&other), mass_properties(&other));
    }

    /// A document of more bodies than the store held before 2026-10-21
    /// (256): each body is measured once, however often all of them are
    /// asked for, and a body that is gone makes room.
    #[test]
    fn many_bodies_are_each_measured_once() {
        let bodies: Vec<Arc<Body>> = (0..300).map(|k| block(1.0 + f64::from(k))).collect();
        let before = fresh_measures();
        for _ in 0..3 {
            for body in &bodies {
                assert!(mass_of(body).is_ok());
            }
        }
        assert_eq!(fresh_measures() - before, 300);
        let refs: Vec<&Arc<Body>> = bodies.iter().collect();
        assert!(measure_ahead(&refs));
        assert_eq!(fresh_measures() - before, 300);
        // Another body at the address of one that is gone is not taken
        // for it: the store's `Weak` keeps that address from being reused
        // while it remembers it.
        let gone = block(0.5);
        let its = mass_of(&gone);
        drop(gone);
        let fresh = block(0.25);
        assert_eq!(mass_of(&fresh), mass_properties(&fresh));
        assert_ne!(mass_of(&fresh), its);
    }

    /// `document.inspect {measures: false}` measures nothing and leaves
    /// out what it would take measuring; the full inspect after it has
    /// them, measured once, and lists the same rest.
    #[test]
    fn inspect_without_measures_measures_nothing() {
        use crate::{Session, Workspace};
        use serde_json::json;
        let mut s = Session::new(Workspace::none());
        let mut run = |id: &str, params: serde_json::Value| {
            s.execute(id, &params)
                .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
        };
        run("file.new", json!({"units": "mm"}));
        run(
            "feature.block",
            json!({"size": [40, 30, 10], "name": "Plate"}),
        );
        run(
            "body.material",
            json!({"body": "Plate", "name": "Steel", "density": 7850}),
        );
        run("feature.sphere", json!({"radius": 4, "name": "Ball"}));
        let before = fresh_measures();
        let quick = run("document.inspect", json!({"measures": false}));
        assert_eq!(fresh_measures() - before, 0);
        let full = run("document.inspect", json!({}));
        assert_eq!(fresh_measures() - before, 2, "each body once");
        let again = run("document.inspect", json!({"measures": true}));
        assert_eq!(fresh_measures() - before, 2);
        assert_eq!(again, full);
        let measured = [
            "volume",
            "area",
            "centroid",
            "mass",
            "massUnit",
            "inertia",
            "inertiaUnit",
            "bbox",
            "bboxTolerance",
        ];
        let (Some(quick_bodies), Some(full_bodies)) =
            (quick["bodies"].as_array(), full["bodies"].as_array())
        else {
            panic!("{quick} {full}");
        };
        assert_eq!(quick_bodies.len(), 2);
        assert!(full_bodies[0]["mass"].as_f64().is_some_and(|m| m > 0.0));
        for (quick, full) in quick_bodies.iter().zip(full_bodies) {
            let mut stripped = full.clone();
            for key in measured {
                assert!(quick.get(key).is_none(), "{key}: {quick}");
                if let Some(map) = stripped.as_object_mut() {
                    map.remove(key);
                }
            }
            assert!(full.get("volume").is_some() && full.get("bbox").is_some());
            assert_eq!(*quick, stripped);
        }
        assert_eq!(quick["features"], full["features"]);
        // Not a boolean: refused, nothing measured.
        assert!(
            s.execute("document.inspect", &json!({"measures": 1}))
                .is_err()
        );
    }

    #[test]
    fn mass_and_inertia_together_are_what_each_is_alone_and_measured_once() {
        let body = block(1.5);
        let before = fresh_measures();
        let (mass, turned) = mass_and_inertia_of(&body);
        assert_eq!(mass, mass_properties(&body));
        assert_eq!(turned, inertia(&body));
        assert_eq!(mass_of(&body), mass);
        assert_eq!(mass_and_inertia_of(&body), (mass, turned));
        assert_eq!(fresh_measures() - before, 1);
    }

    /// A colour or a material measures nothing: the mass and inertia of a
    /// body with a material are measured when document.measure or
    /// document.inspect asks for them, once (both at once), and not again.
    #[test]
    fn colours_and_materials_measure_nothing() {
        use crate::{Access, Session, Workspace};
        use serde_json::json;
        let mut s = Session::new(Workspace {
            read: Access::None,
            write: Access::None,
        });
        let mut run = |id: &str, params: serde_json::Value| {
            s.execute(id, &params)
                .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
        };
        run("file.new", json!({"units": "mm"}));
        run(
            "feature.block",
            json!({"size": [40, 30, 10], "name": "Plate"}),
        );
        run(
            "feature.cylinder",
            json!({"position": [20, 15, -5], "radius": 5, "height": 20, "name": "Pin"}),
        );
        let before = fresh_measures();
        run(
            "body.appearance",
            json!({"body": "Plate", "color": "#c8102e"}),
        );
        run(
            "body.material",
            json!({"body": "Plate", "name": "Steel", "density": 7850}),
        );
        run(
            "body.material",
            json!({"body": "Pin", "name": "Brass", "density": "8.5 g/cm3"}),
        );
        assert_eq!(fresh_measures() - before, 0);
        let measured = run("document.measure", json!({}));
        assert!(measured["mass"].as_f64().is_some_and(|m| m > 0.0));
        assert_eq!(
            fresh_measures() - before,
            2,
            "one per body, mass and inertia at once"
        );
        run("document.inspect", json!({}));
        run("document.measure", json!({}));
        assert_eq!(fresh_measures() - before, 2);
    }
}
