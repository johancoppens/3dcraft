//! Random multi-step designs (docs/testing.md › Random multi-step
//! designs): a seeded generator of feature sequences built from the real
//! command set, with operands on a grid and on the bodies' own faces,
//! edges and vertices; a runner that holds every step to the oracles
//! (a crash, a wrong result, a refusal); and the file format of a design,
//! with references by name, which the shrinker in `cargo xtask fuzz`
//! cuts down to a minimal reproducer.
//!
//! The binary `craft3d-fuzz` runs one seed or one design file per process;
//! `cargo xtask fuzz` runs many of them, each in its own process with a
//! timeout and a memory cap, shrinks what fails and writes the summary the
//! morning health report reads.

pub mod generate;
pub mod rng;
pub mod runner;
pub mod steps;

pub use generate::{Generator, run_seed};
pub use runner::{Deep, Options, Report, Runner, run_design};
pub use steps::{Design, Group, Setup};
