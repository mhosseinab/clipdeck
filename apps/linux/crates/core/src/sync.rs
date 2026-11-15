//! Folder-based sync per spec/behavior.md §9: JSONL append + merge with
//! byte-offset tracking and createdAt-descending ordering after import.

use crate::store::Store;
use crate::types::ClipItem;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct SyncStore {
    file: Option<PathBuf>,
    last_size: u64,
}

#[derive(Debug, Default)]
pub struct MergeCounts {
    pub imported: usize,
    pub skipped: usize,
}

impl SyncStore {
    pub fn new(sync_directory: &str) -> SyncStore {
        let file = if sync_directory.trim().is_empty() {
            None
        } else {
            let dir = Path::new(sync_directory.trim());
            let _ = std::fs::create_dir_all(dir);
            Some(dir.join("clipdeck-sync.jsonl"))
        };
        let last_size = file
            .as_ref()
            .and_then(|f| std::fs::metadata(f).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        SyncStore { file, last_size }
    }

    pub fn enabled(&self) -> bool {
        self.file.is_some()
    }

    /// Append a newly captured item (post-dedupe).
    pub fn append(&self, item: &ClipItem) {
        let Some(file) = &self.file else { return };
        let row = serde_json::json!({ "v": 1, "item": item });
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
            let _ = writeln!(f, "{row}");
        }
    }

    /// Merge lines added since the last merge. Only complete lines are
    /// consumed; a trailing partial line is left for the next merge.
    pub fn merge(&mut self, store: &mut Store) -> MergeCounts {
        let mut counts = MergeCounts::default();
        let Some(file) = &self.file else { return counts };
        let Ok(meta) = std::fs::metadata(file) else { return counts };
        if meta.len() <= self.last_size {
            return counts;
        }
        let Ok(all) = std::fs::read(file) else { return counts };
        let new_bytes = &all[self.last_size as usize..];
        let consumed_to = new_bytes.iter().rposition(|&b| b == b'\n').map(|p| p + 1).unwrap_or(0);
        if consumed_to == 0 {
            return counts;
        }
        self.last_size += consumed_to as u64;

        let chunk = String::from_utf8_lossy(&new_bytes[..consumed_to]);
        let mut imported = false;
        for line in chunk.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                counts.skipped += 1;
                continue;
            };
            let Some(item_json) = row.get("item") else {
                counts.skipped += 1;
                continue;
            };
            let Ok(item) = ClipItem::from_import(item_json) else {
                counts.skipped += 1;
                continue;
            };
            let key = item.dedupe_key();
            if store.items.iter().any(|i| i.dedupe_key() == key || i.id == item.id) {
                counts.skipped += 1;
                continue;
            }
            let mut item = item;
            item.stack_order = None;
            store.items.push(item);
            counts.imported += 1;
            imported = true;
        }
        if imported {
            store.sort_newest_first();
        }
        counts
    }
}
