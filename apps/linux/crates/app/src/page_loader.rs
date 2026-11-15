//! One read-only SQLite worker. At most one request is in flight; the UI keeps
//! only its newest pending query and applies replies through AppEvent.
use crate::events::AppEvent;
use clipdeck_core::{ClipFilter, EncryptionKey, StoreReader, PAGE_SIZE};
use std::{path::PathBuf, sync::mpsc::{sync_channel, SyncSender}};
use winit::event_loop::EventLoopProxy;

#[derive(Debug, Clone)]
pub struct PageRequest {
    pub token: u64,
    pub query: String,
    pub filter: ClipFilter,
    pub offset: usize,
}

pub fn spawn(path: PathBuf, key: Option<EncryptionKey>, events: EventLoopProxy<AppEvent>) -> SyncSender<PageRequest> {
    let (tx, rx) = sync_channel::<PageRequest>(1);
    std::thread::spawn(move || {
        let reader = StoreReader::open_with_key(&path, key.as_ref()).map_err(|error| format!("Cannot read clipboard history: {error}"));
        for request in rx {
            let page = match &reader {
                Ok(reader) => reader.page(&request.query, &request.filter, request.offset, PAGE_SIZE)
                    .map_err(|error| format!("Cannot read clipboard history: {error}")),
                Err(error) => Err(error.clone()),
            };
            if events.send_event(AppEvent::PageLoaded { token: request.token, page }).is_err() { break; }
        }
    });
    tx
}
