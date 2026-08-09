mod state;
mod terminal;
mod view;

pub(crate) fn run() -> anyhow::Result<()> {
    terminal::run()
}
