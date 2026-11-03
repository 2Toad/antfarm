//! Runs an [`Engine`] over the inputs a [`Source`] produces and fans out the results.
//!
//! Snapshots go out on a `watch` channel, coalesced to at most one per
//! [`PUBLISH_EVERY`]: a reader always sees the latest state and never a backlog. Events
//! go out on a `broadcast` channel in order; a reader that falls behind gets a
//! `Resync` event from [`Handle::event_stream`] and should re-read the snapshot.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::engine::Engine;
use crate::event::{Event, EventEnvelope};
use crate::input::Input;
use crate::snapshot::Snapshot;

pub const PUBLISH_EVERY: Duration = Duration::from_millis(50);
pub const EVENT_CAPACITY: usize = 4096;
const INPUT_CAPACITY: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Something that turns files, a replay, or anything else into engine inputs.
pub trait Source: Send + 'static {
    fn name(&self) -> &'static str;
    /// Starts producing inputs into `sink` until `stop` is cancelled or the source runs
    /// out. Dropping `sink` tells the hub the source is done.
    fn start(
        self: Box<Self>,
        sink: mpsc::Sender<Input>,
        stop: CancellationToken,
    ) -> JoinHandle<Result<(), SourceError>>;
}

/// Where `Tick` inputs come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    /// The hub sends a wall-clock tick every second. For live sources.
    Wall,
    /// The source sends its own ticks. For replay.
    Source,
}

pub struct Handle {
    snapshots: watch::Receiver<Arc<Snapshot>>,
    events: broadcast::Sender<Arc<EventEnvelope>>,
    stop: CancellationToken,
    hub: JoinHandle<()>,
    source: JoinHandle<Result<(), SourceError>>,
}

impl Handle {
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshots.borrow().clone()
    }

    pub fn snapshots(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshots.clone()
    }

    pub fn events(&self) -> broadcast::Receiver<Arc<EventEnvelope>> {
        self.events.subscribe()
    }

    /// Events in order, with a `Resync` in place of anything missed.
    pub fn event_stream(&self) -> impl Stream<Item = Arc<EventEnvelope>> + use<> {
        BroadcastStream::new(self.events.subscribe()).map(|r| match r {
            Ok(e) => e,
            Err(_) => Arc::new(EventEnvelope {
                seq: 0,
                at: Utc::now(),
                session: None,
                event: Event::Resync,
            }),
        })
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// Waits for the source and the hub to finish. A live source only finishes after
    /// [`Handle::stop`]; a replay finishes on its own.
    pub async fn join(self) -> Result<(), SourceError> {
        let r = self
            .source
            .await
            .map_err(|e| SourceError::Other(e.to_string()))?;
        let _ = self.hub.await;
        r
    }
}

pub struct Hub;

impl Hub {
    /// Starts `source` and an engine task. Must be called inside a tokio runtime.
    pub fn spawn(source: Box<dyn Source>, clock: Clock) -> Handle {
        let stop = CancellationToken::new();
        let (tx, rx) = mpsc::channel(INPUT_CAPACITY);
        let engine = Engine::new();
        let (snap_tx, snapshots) = watch::channel(Arc::new(engine.snapshot()));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let source = source.start(tx, stop.clone());
        let hub = tokio::spawn(run(
            engine,
            rx,
            snap_tx,
            events.clone(),
            clock,
            stop.clone(),
        ));
        Handle {
            snapshots,
            events,
            stop,
            hub,
            source,
        }
    }
}

async fn run(
    mut engine: Engine,
    mut rx: mpsc::Receiver<Input>,
    snap_tx: watch::Sender<Arc<Snapshot>>,
    events: broadcast::Sender<Arc<EventEnvelope>>,
    clock: Clock,
    stop: CancellationToken,
) {
    let mut publish = tokio::time::interval(PUBLISH_EVERY);
    publish.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut wall = tokio::time::interval(Duration::from_secs(1));
    wall.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = false;
    let apply = |engine: &mut Engine, input: Input, dirty: &mut bool| {
        for e in engine.apply(input) {
            let _ = events.send(Arc::new(e));
        }
        *dirty = true;
    };
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            msg = rx.recv() => match msg {
                Some(input) => apply(&mut engine, input, &mut dirty),
                None => break,
            },
            _ = wall.tick(), if clock == Clock::Wall => {
                apply(&mut engine, Input::Tick { now: Utc::now() }, &mut dirty);
            }
            _ = publish.tick() => {
                if dirty {
                    let _ = snap_tx.send(Arc::new(engine.snapshot()));
                    dirty = false;
                }
            }
        }
    }
    // Drain whatever the source sent before it finished.
    while let Ok(input) = rx.try_recv() {
        apply(&mut engine, input, &mut dirty);
    }
    let _ = snap_tx.send(Arc::new(engine.snapshot()));
}
