mod state;
mod startup;
mod terminal;
mod transcript;
mod view;

use crate::runtime_log::RuntimeLog;

pub(crate) fn run(runtime_log: Option<RuntimeLog>) -> anyhow::Result<()> {
    terminal::run(runtime_log)
}
