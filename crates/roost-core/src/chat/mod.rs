pub mod context;
pub mod manager;
mod run;
pub mod types;

pub use manager::{ChatConfig, ChatHandle, ChatManager};
pub use types::{ChatCommand, ChatEvent, EventSink, SendRejection, TurnOutcome};

#[cfg(test)]
mod tests;
