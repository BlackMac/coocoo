//! Event fan-out.
//!
//! Every event is serialized exactly once into its final envelope text, then shared as
//! `Arc<str>` with every connection — no per-client re-encoding.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{broadcast, mpsc};

use crate::wire::EventEnvelope;

/// One serialized event on the bus.
#[derive(Debug)]
pub struct BusEvent {
    /// Event name (`agent.updated`, …) for cheap filtering without parsing `json`.
    pub name: String,
    /// Host the event originated on.
    pub host: String,
    /// `true` when produced by this daemon; `false` when relayed from a peer link.
    /// Peer links subscribe to local-origin events only, which prevents relay loops.
    pub local: bool,
    /// Complete `EventEnvelope` JSON text, ready to put on the wire.
    pub json: Arc<str>,
}

/// Cloneable handle for publishing events from any subsystem.
#[derive(Clone)]
pub struct Emitter {
    host_id: Arc<str>,
    tx: broadcast::Sender<Arc<BusEvent>>,
}

impl Emitter {
    /// `capacity` bounds how far a slow connection may lag before it is told to resync.
    pub fn new(host_id: impl Into<Arc<str>>, capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { host_id: host_id.into(), tx }
    }

    pub fn host_id(&self) -> &str {
        &self.host_id
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BusEvent>> {
        self.tx.subscribe()
    }

    /// Publish a local event. Serialization failures are programming errors and are logged.
    pub fn emit<T: Serialize>(&self, event: &str, data: &T) {
        let Some(json) = encode_event(event, &self.host_id, data) else { return };
        // No receivers is fine: nobody is connected.
        let _ = self.tx.send(Arc::new(BusEvent {
            name: event.to_owned(),
            host: self.host_id.to_string(),
            local: true,
            json,
        }));
    }

    /// Re-publish an event received from a peer daemon (already carries its origin host).
    pub fn relay(&self, name: String, host: String, json: Arc<str>) {
        let _ = self.tx.send(Arc::new(BusEvent { name, host, local: false, json }));
    }
}

/// Serialize an event envelope once.
pub fn encode_event<T: Serialize>(event: &str, host: &str, data: &T) -> Option<Arc<str>> {
    let data = match serde_json::to_value(data) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("coocoo: failed to encode event {event}: {err}");
            return None;
        }
    };
    let envelope = EventEnvelope { event: event.to_owned(), host: host.to_owned(), data };
    serde_json::to_string(&envelope).ok().map(Arc::from)
}

/// Outbound side of one client connection (or peer link).
///
/// Used for responses and for connection-scoped events such as screencast frames, which go
/// only to the connections that asked for them.
#[derive(Clone, Debug)]
pub struct ConnSink {
    pub id: u64,
    tx: mpsc::Sender<Arc<str>>,
}

impl ConnSink {
    pub fn new(id: u64, tx: mpsc::Sender<Arc<str>>) -> Self {
        Self { id, tx }
    }

    /// Reliable send (awaits queue space). Returns `false` once the connection is gone.
    pub async fn send(&self, text: Arc<str>) -> bool {
        self.tx.send(text).await.is_ok()
    }

    /// Lossy send for high-rate streams (screencast frames): drops when the queue is full,
    /// so a slow client degrades frame rate instead of growing memory.
    /// Returns `false` when dropped or the connection is gone.
    pub fn try_send(&self, text: Arc<str>) -> bool {
        self.tx.try_send(text).is_ok()
    }

    /// Lossy send of an event envelope addressed to this connection only.
    pub fn try_send_event<T: Serialize>(&self, event: &str, host: &str, data: &T) -> bool {
        match encode_event(event, host, data) {
            Some(json) => self.try_send(json),
            None => false,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}
