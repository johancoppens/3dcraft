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

use craft3d_topo::{Body, Inertia, MassError, MassProperties, inertia, mass_properties};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Most bodies remembered; the oldest go first.
const MAX_REMEMBERED: usize = 256;

type Measured = Result<MassProperties, MassError>;

static REMEMBERED: Mutex<Vec<(Weak<Body>, Measured)>> = Mutex::new(Vec::new());

/// `mass_properties(body)`, measured at most once per body. The lock is
/// not held while measuring: two threads that ask at once both measure.
pub(crate) fn mass_of(body: &Arc<Body>) -> Measured {
    let key = Arc::downgrade(body);
    {
        let remembered = REMEMBERED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, measured)) = remembered.iter().find(|(seen, _)| seen.ptr_eq(&key)) {
            return measured.clone();
        }
    }
    let measured = mass_properties(body);
    let mut remembered = REMEMBERED.lock().unwrap_or_else(PoisonError::into_inner);
    // Bodies that are gone are forgotten.
    remembered.retain(|(seen, _)| seen.strong_count() > 0);
    if !remembered.iter().any(|(seen, _)| seen.ptr_eq(&key)) {
        if remembered.len() >= MAX_REMEMBERED {
            drop(remembered.remove(0));
        }
        remembered.push((key, measured.clone()));
    }
    measured
}

type Turned = Result<Inertia, MassError>;

static REMEMBERED_INERTIA: Mutex<Vec<(Weak<Body>, Turned)>> = Mutex::new(Vec::new());

/// `inertia(body)`, measured at most once per body, like [`mass_of`]:
/// only bodies with a material are asked for it, and summing the second
/// moments of a body with spline faces takes as long as its volume.
pub(crate) fn inertia_of(body: &Arc<Body>) -> Turned {
    let key = Arc::downgrade(body);
    {
        let remembered = REMEMBERED_INERTIA
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((_, measured)) = remembered.iter().find(|(seen, _)| seen.ptr_eq(&key)) {
            return measured.clone();
        }
    }
    let measured = inertia(body);
    let mut remembered = REMEMBERED_INERTIA
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    remembered.retain(|(seen, _)| seen.strong_count() > 0);
    if !remembered.iter().any(|(seen, _)| seen.ptr_eq(&key)) {
        if remembered.len() >= MAX_REMEMBERED {
            drop(remembered.remove(0));
        }
        remembered.push((key, measured.clone()));
    }
    measured
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
}
