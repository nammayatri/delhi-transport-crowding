//! Crowding model for city buses, built from ticket records and a static GTFS feed.
//!
//! The pipeline is:
//! 1. [`tickets`]: read raw ticket records.
//! 2. [`assign`]: match each ticket's route and stops to a GTFS pattern, impute missing
//!    alighting stops under the fare constraint, and attach the nearest scheduled trip.
//! 3. [`profile`]: keep a running onboard count per stop-to-stop segment, pooled over all
//!    scheduled trips of a route in each day type and time bin, then shrink sparse bins.
//! 4. [`levels`] and [`capacity`]: expand ticketed riders to all riders, compare with the
//!    bus class capacity, and turn that into a crowding level with a probability.
//! 5. [`comfort`]: per-trip cumulative crowding cost, used by the journey planner.
//! 6. [`nowcast`]: adjust the historical profile with live tickets.
//! 7. [`rt`]: live bus positions; calibrate bus supply and place buses on their routes.

// Dense [position][day type][bin] arrays read most clearly with index loops.
#![allow(clippy::needless_range_loop)]

pub mod assign;
pub mod build;
pub mod capacity;
pub mod comfort;
pub mod config;
pub mod daytype;
pub mod geo;
pub mod groups;
pub mod gtfs;
pub mod levels;
pub mod names;
pub mod nowcast;
pub mod planner;
pub mod profile;
pub mod rt;
pub mod snapshot;
pub mod stats;
pub mod testkit;
pub mod tickets;
pub mod trips;

pub use config::ModelConfig;
pub use daytype::DayType;
