mod agent_loop;
pub mod context;
pub mod manager;
mod run;
pub mod types;

pub use manager::{ChatConfig, ChatHandle, ChatManager};
pub use types::{ChatCommand, ChatEvent, EventSink, RunOutcome, SendRejection, TurnOutcome};

#[cfg(test)]
mod tests;
