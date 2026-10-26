//! Rust port of the Python `maze` weave-maze generator.

pub mod maze;
pub mod random;
pub mod renderers;

pub use crate::maze::{create_maze, find_solution, Grid};
