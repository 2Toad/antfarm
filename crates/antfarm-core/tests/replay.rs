//! Replays the real fixtures through the hub and checks they end in the same state as
//! feeding them to the engine directly, and that timed replay emits in time order.

mod common;

use std::time::Duration;

use antfarm_core::replay::{ReplayConfig, ReplaySource, Speed};
use antfarm_core::source::{Clock, Hub};
use antfarm_core::{Event, EventEnvelope};
use common::{fixture_dir, run, summarize};

async fn replay(name: &str, speed: Speed) -> (antfarm_core::Snapshot, Vec<EventEnvelope>) {
    let mut config = ReplayConfig::new(fixture_dir(name).join("main.jsonl"));
    config.speed = speed;
    config.max_gap = Duration::from_millis(2);
    config.session = Some(name.to_owned());
    let h = Hub::spawn(Box::new(ReplaySource::new(config)), Clock::Source);
    let mut rx = h.events();
    let mut events = Vec::new();
    loop {
        tokio::select! {
            _ = h.finished() => break,
            r = rx.recv() => match r {
                Ok(e) => events.push((*e).clone()),
                Err(e) => panic!("event stream: {e}"),
            },
        }
    }
    while let Ok(e) = rx.try_recv() {
        events.push((*e).clone());
    }
    let snap = (*h.snapshot()).clone();
    h.join().await.unwrap();
    (snap, events)
}

fn assert_same_end_state(name: &str, replayed: &antfarm_core::Snapshot) {
    let (direct, _) = run(name, u64::MAX);
    let mut a = serde_json::to_value(summarize(&direct, 0, name)).unwrap();
    let mut b = serde_json::to_value(summarize(replayed, 0, name)).unwrap();
    // Event counts differ by design: replay adds ticks and interleaves files. Agents are
    // created in time order rather than file order, so compare them sorted.
    for v in [&mut a, &mut b] {
        v["events"] = 0.into();
        if let Some(agents) = v["agents"].as_array_mut() {
            agents.sort_by(|x, y| x["id"].as_str().cmp(&y["id"].as_str()));
        }
        if let Some(open) = v["open_calls"].as_array_mut() {
            open.sort_by(|x, y| x.as_str().cmp(&y.as_str()));
        }
        // Sums taken in a different order differ in the last bits.
        for k in ["delta_usd", "usd_per_min"] {
            if let Some(x) = v["cost"][k].as_f64() {
                v["cost"][k] = ((x * 1e6).round() / 1e6).into();
            }
        }
    }
    let mut diffs = Vec::new();
    diff("", &a, &b, &mut diffs);
    assert!(
        diffs.is_empty(),
        "{name}: replay and direct feed differ:\n{}",
        diffs.join("\n")
    );
}

/// Paths where two JSON values differ, as `path: direct != replay`.
fn diff(path: &str, a: &serde_json::Value, b: &serde_json::Value, out: &mut Vec<String>) {
    use serde_json::Value::{Array, Object};
    match (a, b) {
        (Object(x), Object(y)) => {
            for k in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                diff(
                    &format!("{path}.{k}"),
                    x.get(k).unwrap_or(&serde_json::Value::Null),
                    y.get(k).unwrap_or(&serde_json::Value::Null),
                    out,
                );
            }
        }
        (Array(x), Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                let label = p
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| i.to_string());
                diff(&format!("{path}[{label}]"), p, q, out);
            }
        }
        _ if a != b => out.push(format!("{path}: {a} != {b}")),
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn max_speed_replay_matches_direct_feed() {
    for name in ["live-tail", "small-session"] {
        let (snap, events) = replay(name, Speed::Max).await;
        assert!(!events.iter().any(|e| matches!(e.event, Event::Resync)));
        assert_same_end_state(name, &snap);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn timed_replay_emits_in_time_order() {
    let (snap, events) = replay("small-session", Speed::Times(1e6)).await;
    assert_same_end_state("small-session", &snap);
    let starts: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.event, Event::ToolCallStarted { .. }))
        .map(|e| e.at)
        .collect();
    assert!(starts.len() > 100);
    assert!(
        starts.windows(2).all(|w| w[0] <= w[1]),
        "tool calls out of time order"
    );
    assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));
    assert!(
        events.iter().all(|e| e.at.timestamp() > 0),
        "an event fell back to the Unix epoch"
    );
}
