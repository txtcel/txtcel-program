//! On-chain account state and the shared helpers operating on it.
//!
//! This module was decomposed out of a single large `state.rs` into cohesive
//! submodules (constants, account structs, generic asserts, PDA derivation,
//! low-level account utilities, fee/shard helpers, and account loaders). Every
//! item is re-exported here so existing `use crate::state::*` /
//! `use crate::state::Foo;` consumers keep resolving unchanged.

mod account_utils;
mod accounts;
mod asserts;
mod constants;
mod fees;
mod loaders;
mod pda;

pub use account_utils::*;
pub use accounts::*;
pub use asserts::*;
pub use constants::*;
pub use fees::*;
pub use loaders::*;
pub use pda::*;

// Content node structure lives in its own module (`crate::content`) for
// decomposition. Re-exported here so existing `use crate::state::*` consumers
// keep resolving `ContentNode`, `load_content`, `derive_content_pda`, etc.
pub use crate::content::*;
