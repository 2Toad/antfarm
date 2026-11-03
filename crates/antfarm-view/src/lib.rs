//! View models shared by antfarm's front ends: what each screen shows, built as plain
//! serializable data from a snapshot and a `now`. Nothing here draws or reads the clock.

pub mod fmt;
pub mod lanes;
pub mod view;

pub use view::*;
