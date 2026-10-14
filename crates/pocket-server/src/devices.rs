use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pocket_store::repos::devices::{DeviceRow, DevicesRepo};
use pocket_store::Store;
use tokio::sync::Notify;

use crate::events::EventBus;

/// Devices (NET-11): CRUD over `pocket-store`'s `DevicesRepo`, plus
/// revocation that both deletes the token hash and closes any open SSE
/// stream for that device (registered via `register_stream`).
pub struct DeviceService<'a> {
    store: &'a Store,
    bus: &'a EventBus,
    streams: Mutex<HashMap<String, Vec<Arc<Notify>>>>,
}

impl<'a> DeviceService<'a> {
    pub fn new(store: &'a Store, bus: &'a EventBus) -> Self {
        Self {
            store,
            bus,
            streams: Mutex::new(HashMap::new()),
        }
    }

    pub fn create(&self, row: &DeviceRow) -> rusqlite::Result<()> {
        DevicesRepo::new(self.store).create(row)
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<DeviceRow>> {
        DevicesRepo::new(self.store).get(id)
    }

    pub fn list(&self) -> rusqlite::Result<Vec<DeviceRow>> {
        DevicesRepo::new(self.store).list()
    }

    /// An open `/pocket/v1/events` SSE connection for `device_id` registers
    /// a handle here; the handler `.await`s `notified()` alongside streaming
    /// events and exits its loop (closing the connection) once notified.
    pub fn register_stream(&self, device_id: &str) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.streams
            .lock()
            .unwrap()
            .entry(device_id.to_string())
            .or_default()
            .push(notify.clone());
        notify
    }

    /// Deletes the device's token hash (by clearing it; a cleared hash never
    /// matches a real SHA-256 digest so auth lookups fail closed), emits
    /// `device.revoked`, and closes every open SSE stream for that device.
    pub fn revoke_device(&self, device_id: &str, now_ms: i64) -> rusqlite::Result<()> {
        let repo = DevicesRepo::new(self.store);
        repo.revoke(device_id, now_ms)?;
        // Clear the token hash directly so no SHA-256 digest can ever match
        // it again (DevicesRepo has no dedicated "clear token" method yet).
        self.store
            .writer()
            .execute("UPDATE devices SET token_hash = x'' WHERE id = ?1", [device_id])?;

        self.bus.publish(
            now_ms,
            device_id,
            "device.revoked",
            &serde_json::json!({ "device_id": device_id }).to_string(),
            None,
        );

        if let Some(notifies) = self.streams.lock().unwrap().remove(device_id) {
            for notify in notifies {
                notify.notify_waiters();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> DeviceRow {
        DeviceRow {
            id: id.to_string(),
            name: "Rohit's iPhone".into(),
            platform: "ios".into(),
            public_key: vec![1, 2, 3],
            token_hash: vec![4, 5, 6],
            role: "owner".into(),
            paired_at: 1,
            last_seen_at: None,
            last_addr: None,
            revoked_at: None,
        }
    }

    #[tokio::test]
    async fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let bus = EventBus::new();
        let service = DeviceService::new(&store, &bus);
        service.create(&device("dev1")).unwrap();
        assert!(service.get("dev1").unwrap().is_some());
        assert_eq!(service.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn revoke_emits_device_revoked_event() {
        let store = Store::open_in_memory().unwrap();
        let bus = EventBus::new();
        let service = DeviceService::new(&store, &bus);
        service.create(&device("dev1")).unwrap();

        let mut sub = bus.subscribe();
        service.revoke_device("dev1", 42).unwrap();

        let event = sub.recv().await.unwrap();
        assert_eq!(event.event_type, "device.revoked");
        assert!(event.data_json.contains("dev1"));

        let row = service.get("dev1").unwrap().unwrap();
        assert_eq!(row.revoked_at, Some(42));
    }

    #[tokio::test]
    async fn revoke_closes_open_streams_for_that_device() {
        let store = Store::open_in_memory().unwrap();
        let bus = EventBus::new();
        let service = DeviceService::new(&store, &bus);
        service.create(&device("dev1")).unwrap();

        let notify = service.register_stream("dev1");
        let notified = notify.notified();
        tokio::pin!(notified);

        // Not notified yet - registers this future as a waiter.
        assert!(futures_now_or_never(&mut notified).is_none());

        service.revoke_device("dev1", 1).unwrap();
        // `notify_waiters()` only wakes futures already registered as
        // waiters (not ones created afterward), so this must be the same
        // pinned future polled above, per tokio::sync::Notify's semantics.
        notified.await;
    }

    /// Minimal "is this future already ready" poll without pulling in the
    /// `futures` crate just for a test helper.
    fn futures_now_or_never<F: std::future::Future + Unpin>(fut: &mut F) -> Option<F::Output> {
        use std::task::{Context, Poll};
        let waker = futures_noop_waker();
        let mut cx = Context::from_waker(&waker);
        match std::pin::Pin::new(fut).poll(&mut cx) {
            Poll::Ready(v) => Some(v),
            Poll::Pending => None,
        }
    }

    fn futures_noop_waker() -> std::task::Waker {
        use std::task::{RawWaker, RawWakerVTable, Waker};
        fn no_op(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }
}
