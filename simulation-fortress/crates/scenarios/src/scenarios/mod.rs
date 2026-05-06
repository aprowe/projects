//! Catalog of playable scenarios. Each child module exports a
//! single `Scenario`-implementing struct that the launcher (CLI
//! flags or TUI scenario picker) can instantiate.

pub mod airport;
pub mod bank;
pub mod cabin;
pub mod cafeteria;
pub mod family_home;
pub mod farming;
pub mod home_invasion;
pub mod mansion;
pub mod office;
