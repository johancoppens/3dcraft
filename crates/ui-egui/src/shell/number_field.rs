//! The arithmetic and the memory of the number fields, which are now the
//! kit's one slider ([`crate::kit::slider`], the developer's decision of
//! 2026-11-04): re-exported here under the names the shell and its tests
//! know.

pub use crate::kit::slider::{
    FieldDrag, FieldMemory, MAX_TYPED, Typing, dragged, fine_step, nice_step, range_hint,
    shortened, shown, snapped, stepped,
};
