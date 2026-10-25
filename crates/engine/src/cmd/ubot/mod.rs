//! U-BOT toolkit commands (spec §7): drawing reads and one-step writes around the `cadcraft-ubot` core.

mod fill_apply;
mod fill_cmd;
mod fill_input;
mod fill_interactive;
mod lib_cmd;
pub mod read;
pub mod state;
pub mod units;
pub mod write;
pub mod xdata;
mod zone;

pub use state::UbotState;
pub use zone::zone_id;

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    vec![lib_cmd::spec(), zone::spec(), fill_cmd::spec()]
}
