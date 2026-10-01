/// CLI argument parsing and command adapter modules.
pub mod args;
/// CLI-to-use-case execution adapter and command rendering boundary.
pub mod execute;
/// Admissibility of the global keys, by leaf of the command tree.
pub mod global_flags;
/// CLI-facing output helpers.
pub mod output;
/// CLI signal routing helpers.
pub mod signal;
