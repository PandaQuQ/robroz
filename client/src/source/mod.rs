//! Where events come from.
//!
//! [`HttpSource`] polls the host's `GET /api/groups/{id}/events`.
//! [`FakeSource`] replays a script offline and backs the unit tests.
//! The app and [`crate::model::Feed`] only see [`EventSource`].

mod fake;
mod http;

pub use fake::{fake_schedules, FakeSource, DRAFT_BODY};
pub use http::HttpSource;

use std::fmt;
use std::time::Duration;

use robroz_contract::Event;
use robroz_ui::cards::Card;

/// One entry in the left column. Client-side view data, not a contract
/// type: the host has no group-listing endpoint yet, so a live source may
/// only know the id and a local name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupTab {
    pub id: String,
    pub name: String,
}

/// One contract event tagged with the group it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEvent {
    pub group_id: String,
    pub event: Event,
}

/// A new-kind card the contract has no event for yet (approval, draft),
/// tagged with its group. Only the fake source produces these today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCard {
    pub group_id: String,
    pub card: Card,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError(pub String);

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Calls block (the HTTP source does network I/O); the app runs them off the
/// UI thread.
pub trait EventSource: Send {
    /// Groups to show in the left column.
    fn groups(&mut self) -> Result<Vec<GroupTab>, SourceError>;
    /// Events since the last poll. Returning the full snapshot is also fine,
    /// because `Feed::apply` is idempotent.
    fn poll(&mut self) -> Result<Vec<GroupEvent>, SourceError>;
    /// False once nothing more will arrive. The fake source stops after its
    /// script; a live source always says true.
    fn has_more(&self) -> bool;
    /// Cards released with the last `poll` that have no contract event yet.
    fn poll_cards(&mut self) -> Vec<GroupCard> {
        Vec::new()
    }
    /// How often the app should call `poll`.
    fn interval(&self) -> Duration {
        Duration::from_secs(1)
    }
}
