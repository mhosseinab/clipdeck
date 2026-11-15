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
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            let _ = (|| -> std::io::Result<()> {
                f.write_all(b"{\"v\":1,\"item\":")?;
                serde_json::to_writer(&mut f, item)?;
                f.write_all(b"}\n")
            })();
        }
    }

    /// Merge lines added since the last merge. Only complete lines are
    /// consumed; a trailing partial line is left for the next merge.
    pub fn merge(&mut self, store: &mut Store) -> MergeCounts {
        let mut counts = MergeCounts::default();
        let Some(file) = &self.file else {
            return counts;
        };
        let Ok(meta) = std::fs::metadata(file) else {
            return counts;
        };
        if meta.len() <= self.last_size {
            return counts;
        }
        use std::io::{BufRead, Seek, SeekFrom};
        let Ok(mut input) = std::fs::File::open(file) else {
            return counts;
        };
        if input.seek(SeekFrom::Start(self.last_size)).is_err() {
            return counts;
        }
        let mut input = std::io::BufReader::new(input);
        let mut line = Vec::new();
        let mut imported = false;
        loop {
            line.clear();
            let Ok(bytes) = input.read_until(b'\n', &mut line) else {
                break;
            };
            if bytes == 0 || !line.ends_with(b"\n") {
                break;
            }
            self.last_size += bytes as u64;
            let text = String::from_utf8_lossy(&line);
            if text.trim().is_empty() {
                continue;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(&text) else {
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
            if store.contains_content(item.item_type, &item.content) || store.contains_id(&item.id)
            {
                counts.skipped += 1;
                continue;
            }
            let mut item = item;
            item.stack_order = None;
            store.insert_imported(item);
            counts.imported += 1;
            imported = true;
        }
        if imported {
            store.sort_newest_first();
        }
        counts
    }
}
