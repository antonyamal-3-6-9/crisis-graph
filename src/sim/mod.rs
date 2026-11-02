//! Simulation helpers for demos (never part of the dispatch path).

pub mod driver;
pub mod surge;

pub use driver::{heartbeat_key, mission_secs, SimDriver, DRIVER_ACTOR};
