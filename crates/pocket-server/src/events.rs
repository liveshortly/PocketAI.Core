use std::collections::VecDeque;
use std::sync::Mutex;

use tokio::sync::broadcast;

const RING_CAPACITY: usize = 1000;
const BROADCAST_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub ts: i64,
    pub source: String,
    pub event_type: String,
    pub data_json: String,
    /// `None` = every device sees it (model/server events); `Some(id)` =
    /// only that device (per-device filtering, e.g. its own `device.seen`).
    pub target_device: Option<String>,
}

struct RingState {
    buffer: VecDeque<Event>,
    next_seq: u64,
}

/// `tokio::sync::broadcast` + a 1,000-event ring buffer with monotonic
/// `seq`, so `/pocket/v1/events` can honor `Last-Event-ID` on reconnect.
/// Used in-process by `pocket-ffi`'s `subscribe` too.
pub struct EventBus {
    state: Mutex<RingState>,
    tx: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            state: Mutex::new(RingState {
                buffer: VecDeque::with_capacity(RING_CAPACITY),
                next_seq: 1,
            }),
            tx,
        }
    }

    /// Assigns the next `seq`, stores it in the ring buffer (evicting the
    /// oldest if full), and broadcasts it. Never blocks on slow subscribers:
    /// a lagging receiver just misses old messages (`RecvError::Lagged`)
    /// rather than stalling the publisher or other subscribers.
    pub fn publish(
        &self,
        ts: i64,
        source: &str,
        event_type: &str,
        data_json: &str,
        target_device: Option<&str>,
    ) -> Event {
        let mut state = self.state.lock().unwrap();
        let event = Event {
            seq: state.next_seq,
            ts,
            source: source.to_string(),
            event_type: event_type.to_string(),
            data_json: data_json.to_string(),
            target_device: target_device.map(str::to_string),
        };
        state.next_seq += 1;
        if state.buffer.len() >= RING_CAPACITY {
            state.buffer.pop_front();
        }
        state.buffer.push_back(event.clone());
        drop(state);

        let _ = self.tx.send(event.clone());
        event
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    fn visible_to(event: &Event, device_id: Option<&str>) -> bool {
        match &event.target_device {
            None => true,
            Some(target) => Some(target.as_str()) == device_id,
        }
    }

    /// Events with `seq > since_seq`, filtered to what `device_id` may see.
    /// Powers SSE reconnect via `Last-Event-ID`.
    pub fn replay_since(&self, since_seq: u64, device_id: Option<&str>) -> Vec<Event> {
        let state = self.state.lock().unwrap();
        state
            .buffer
            .iter()
            .filter(|e| e.seq > since_seq && Self::visible_to(e, device_id))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_after_reconnect_returns_only_newer_events() {
        let bus = EventBus::new();
        for i in 0..5 {
            bus.publish(i, "mac:host", "model.loaded", "{}", None);
        }
        let replayed = bus.replay_since(2, None);
        assert_eq!(
            replayed.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn replay_filters_to_the_requesting_device() {
        let bus = EventBus::new();
        bus.publish(0, "mac:host", "model.loaded", "{}", None);
        bus.publish(1, "mac:host", "device.seen", "{}", Some("dev1"));
        bus.publish(2, "mac:host", "device.seen", "{}", Some("dev2"));

        let for_dev1 = bus.replay_since(0, Some("dev1"));
        assert_eq!(
            for_dev1.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[tokio::test]
    async fn lagging_subscriber_does_not_block_others_or_the_publisher() {
        let bus = EventBus::new();
        let _lagging = bus.subscribe(); // never read from
        let mut healthy = bus.subscribe();

        // Publish well beyond the broadcast channel's capacity; must not block.
        for i in 0..(BROADCAST_CAPACITY as i64 * 3) {
            bus.publish(i, "mac:host", "generation.finished", "{}", None);
        }

        // The healthy subscriber still receives (possibly after a Lagged error,
        // which is recoverable, not a hang) rather than being stuck forever.
        let mut received_something = false;
        for _ in 0..5 {
            match healthy.try_recv() {
                Ok(_) => {
                    received_something = true;
                    break;
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        assert!(
            received_something,
            "healthy subscriber never received an event"
        );

        // Ring buffer replay is unaffected by any subscriber's lag.
        let replayed = bus.replay_since(0, None);
        assert_eq!(replayed.len(), RING_CAPACITY.min(BROADCAST_CAPACITY * 3));
    }
}
