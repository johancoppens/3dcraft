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

use craft3d_topo::{
    Body, Inertia, MassError, MassProperties, inertia, mass_and_inertia, mass_properties,
    mass_properties_each,
};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Most bodies remembered; the oldest go first.
const MAX_REMEMBERED: usize = 256;

type Measured = Result<MassProperties, MassError>;
type Turned = Result<Inertia, MassError>;

/// Remembered answers, by body.
type Store<T> = Mutex<Vec<(Weak<Body>, T)>>;

static REMEMBERED: Store<Measured> = Mutex::new(Vec::new());
static REMEMBERED_INERTIA: Store<Turned> = Mutex::new(Vec::new());

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
fn recall<T: Clone>(store: &Store<T>, body: &Arc<Body>) -> Option<T> {
    let key = Arc::downgrade(body);
    let remembered = store.lock().unwrap_or_else(PoisonError::into_inner);
    remembered
        .iter()
        .find(|(seen, _)| seen.ptr_eq(&key))
        .map(|(_, value)| value.clone())
}

/// Remembers `value` for `body` in `store`, unless it is there already.
fn remember<T>(store: &Store<T>, body: &Arc<Body>, value: T) {
    let key = Arc::downgrade(body);
    let mut remembered = store.lock().unwrap_or_else(PoisonError::into_inner);
    // Bodies that are gone are forgotten.
    remembered.retain(|(seen, _)| seen.strong_count() > 0);
    if !remembered.iter().any(|(seen, _)| seen.ptr_eq(&key)) {
        if remembered.len() >= MAX_REMEMBERED {
            drop(remembered.remove(0));
        }
        remembered.push((key, value));
    }
}

/// `mass_properties(body)`, measured at most once per body. The lock is
/// not held while measuring: two threads that ask at once both measure.
pub(crate) fn mass_of(body: &Arc<Body>) -> Measured {
    if let Some(measured) = recall(&REMEMBERED, body) {
        return measured;
    }
    counted();
    let measured = mass_properties(body);
    remember(&REMEMBERED, body, measured.clone());
    measured
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
    for (body, measured) in fresh.iter().zip(mass_properties_each(&plain)) {
        let Some(measured) = measured else {
            return false;
        };
        fine &= measured.is_ok();
        counted();
        remember(&REMEMBERED, body, measured);
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
