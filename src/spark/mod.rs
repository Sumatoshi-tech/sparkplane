//! DGX Spark workstation client and feature-gated appliance bootstrap.

pub mod cli;
pub mod client;
#[cfg(feature = "appliance")]
pub mod engine;
pub mod install;
pub mod launch;
pub mod qualification;
#[cfg(feature = "appliance")]
pub mod reconcile;
pub mod wire;

#[cfg(feature = "appliance")]
pub mod agent;
#[cfg(all(feature = "appliance", test))]
pub mod bench;
#[cfg(feature = "appliance")]
pub mod executor;
#[cfg(feature = "appliance")]
pub mod gateway;
#[cfg(feature = "appliance")]
pub mod model;
#[cfg(feature = "appliance")]
pub mod model_catalog;
#[cfg(all(feature = "appliance", test))]
pub mod recipe;
pub mod resources;
#[cfg(feature = "appliance")]
pub mod state;
#[cfg(feature = "appliance")]
pub mod upstream;

pub const EXIT_INTERNAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_REJECTED: i32 = 3;
pub const EXIT_UNREACHABLE: i32 = 4;
pub const EXIT_OPERATION_FAILED: i32 = 5;
#[cfg(feature = "appliance")]
pub const MAX_ENGINE_STARTUP_DEADLINE_SECONDS: u64 = 1_800;
