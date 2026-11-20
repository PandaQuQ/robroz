//! robroz shared UI layer.
//!
//! - [`cards`]: view models and pure rules for the phase-1 cards (`choice`,
//!   `approval`, `draft`) and the routines list (UI SPEC §4–§8).
//! - [`conversations`]: the IM conversation list (groups and bro DMs,
//!   unread counts, previews), as pure data.
//! - [`theme`]: the skin struct every component reads; the default solid
//!   skin is filled from `robroz-tokens`.
//! - [`view`]: iced widgets for the above. They emit [`view::Action`] and
//!   never do I/O, so the same layer can later run in a web build.

pub mod cards;
pub mod conversations;
pub mod theme;
pub mod view;
mod widgets;

pub use theme::{Mode, Skin};
pub use view::Action;
