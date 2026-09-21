#![cfg(feature = "appliance")]
use sparkplane::migration::{
    journal::{Journal, Step},
    runner::{self, Actions},
};

#[derive(Default)]
struct Host {
    events: Vec<(bool, Step)>,
    fail: Option<Step>,
    opened: bool,
}
impl Actions for Host {
    fn apply(&mut self, step: Step) -> anyhow::Result<()> {
        self.events.push((true, step));
        anyhow::ensure!(self.fail != Some(step), "injected interruption");
        Ok(())
    }
    fn undo(&mut self, step: Step) -> anyhow::Result<()> {
        self.events.push((false, step));
        Ok(())
    }
    fn open_traffic(&mut self) -> anyhow::Result<()> {
        self.opened = true;
        Ok(())
    }
}

#[test]
fn every_failed_action_is_reversed_before_traffic_can_reopen() {
    for failure in runner::STEPS {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
        let mut host = Host {
            fail: Some(failure),
            ..Host::default()
        };
        assert!(runner::run(&mut journal, &mut host).is_err());
        assert!(!host.opened);
        runner::recover(&mut journal, &mut host).unwrap();
        let applied: Vec<_> = host.events.iter().filter(|e| e.0).map(|e| e.1).collect();
        let undone: Vec<_> = host.events.iter().filter(|e| !e.0).map(|e| e.1).collect();
        assert_eq!(applied.into_iter().rev().collect::<Vec<_>>(), undone);
    }
}

#[test]
fn committed_restart_only_reopens_traffic_and_never_restores_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    runner::run(&mut journal, &mut Host::default()).unwrap();
    drop(journal);
    let mut journal = Journal::open(dir.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    let mut host = Host::default();
    runner::run(&mut journal, &mut host).unwrap();
    assert!(host.opened && host.events.is_empty());
    assert!(runner::recover(&mut journal, &mut host).is_err());
}

#[test]
fn failed_journal_write_requires_reopen_before_any_host_action() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("journal");
    let moved = dir.path().join("moved");
    std::fs::create_dir(&original).unwrap();
    let mut journal = Journal::open(&original, &"a".repeat(64), &"b".repeat(64)).unwrap();
    std::fs::rename(&original, &moved).unwrap();
    assert!(journal.begin(Step::FenceTraffic).is_err());
    std::fs::rename(moved, original).unwrap();
    let mut host = Host::default();
    assert!(runner::recover(&mut journal, &mut host).is_err());
    assert!(host.events.is_empty());
}
