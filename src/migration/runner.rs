//! Finite cutover sequence. Failures remain fenced until explicit recovery.
use super::journal::{Journal, Step};
use anyhow::{Context, Result};

pub use super::journal::STEPS;

/// Each action and its inverse must be idempotent after process interruption.
/// Implementations may open external traffic only in `open_traffic`, or at the
/// very end of undoing the fence after the preceding appliance is healthy.
pub trait Actions {
    fn apply(&mut self, step: Step) -> Result<()>;
    fn undo(&mut self, step: Step) -> Result<()>;
    fn open_traffic(&mut self) -> Result<()>;
}

pub fn run(journal: &mut Journal, host: &mut impl Actions) -> Result<()> {
    if !journal.committed()? {
        while let Some(step) = journal.next()? {
            journal.begin(step)?;
            host.apply(step).with_context(|| {
                format!("migration {step:?} failed; traffic remains fenced, use recovery")
            })?;
            journal.finish(step)?;
        }
        journal.commit()?;
    }
    host.open_traffic()
        .context("migration committed; retry opening traffic, never restore old state")
}

pub fn recover(journal: &mut Journal, host: &mut impl Actions) -> Result<()> {
    journal.recover(|step| host.undo(step))
}
