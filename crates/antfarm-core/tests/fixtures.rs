//! Golden snapshots of each fixture's final machine and event stream.

mod common;

use common::{fixture_dir, run, summarize};

fn check(name: &str, max_tail: u64) {
    let (snapshot, events) = run(name, max_tail);
    insta::assert_yaml_snapshot!(format!("{name}-machine"), snapshot.machine);
    insta::assert_yaml_snapshot!(format!("{name}-events"), events);
}

#[test]
fn parallel() {
    check("parallel", u64::MAX);
}

#[test]
fn orphan() {
    check("orphan", u64::MAX);
}

#[test]
fn truncated_tail() {
    // Land one byte inside the first line so attach skips it and the first tool_result
    // has no tool_use to pair with.
    let bytes = std::fs::read(fixture_dir("truncated-tail").join("main.jsonl")).unwrap();
    let first_nl = bytes.iter().position(|b| *b == b'\n').unwrap();
    let max_tail = bytes.len() as u64 - first_nl as u64 + 1;
    check("truncated-tail", max_tail);
}

#[test]
fn prompts_titles() {
    check("prompts-titles", u64::MAX);
}

#[test]
fn subagent_lines() {
    check("subagent-lines", u64::MAX);
}

#[test]
fn cost_with_state() {
    check("cost-with-state", u64::MAX);
}

#[test]
fn cost_without_state() {
    check("cost-without-state", u64::MAX);
}

fn check_real(name: &str) {
    let (snapshot, events) = run(name, u64::MAX);
    insta::assert_yaml_snapshot!(
        format!("{name}-summary"),
        summarize(&snapshot, events.len(), name)
    );
}

#[test]
fn real_live_tail() {
    check_real("live-tail");
}

#[test]
fn real_small_session() {
    check_real("small-session");
}
