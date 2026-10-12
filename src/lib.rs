//! nexus-menu — find installed applications that have no menu entry and
//! generate freedesktop `.desktop` files for them.
//!
//! A maintained, desktop-agnostic reimplementation of KDE's long-dead
//! KAppFinder. Like the original it is catalog-driven: it ships templates for
//! programs that commonly arrive without a menu entry, checks which of them
//! are actually installed, and offers to add only the ones missing.
//!
//! This crate holds everything that is not argument parsing or printing, so
//! the command line (`src/main.rs`) and the desktop app (`gui/`) run exactly
//! the same code. [`engine`] is the entry point for callers that want data
//! back instead of text on a terminal.

pub mod catalog;
pub mod desktop;
pub mod engine;
pub mod lnk;
pub mod prefixes;
pub mod registry;
pub mod scanner;
pub mod select;
#[cfg(test)]
mod testutil;
pub mod winapps;
