//! Store: persistence, capture pipeline (dedupe/trim), import/export,
//! pinboards, paste stack per spec/behavior.md §2.4, §5, §6.

use crate::types::{ClipItem, Pinboard, StoreFile};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ImportCounts {
    pub imported: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackDir {
    Fifo,
    Lifo,
}

pub struct Store {
    pub items: Vec<ClipItem>,
    pub pinboards: Vec<Pinboard>,
    pub settings: crate::types::Settings,
    file: PathBuf,
    pub image_dir: PathBuf,
}

impl Store {
    /// Load from `user_data/clipdeck-store.json`; missing/corrupt file = fresh store.
    pub fn open(user_data: &Path) -> Store {
        let file = user_data.join("clipdeck-store.json");
        let image_dir = user_data.join("images");
        let _ = std::fs::create_dir_all(&image_dir);
        let (items, pinboards, settings) = match std::fs::read_to_string(&file) {
            Ok(raw) => match serde_json::from_str::<StoreFile>(&raw) {
                Ok(sf) => (sf.items, sf.pinboards, sf.settings),
                Err(_) => (vec![], vec![], Default::default()),
            },
            Err(_) => (vec![], vec![], Default::default()),
        };
        Store { items, pinboards, settings, file, image_dir }
    }

    pub fn save(&self) {
        let sf = StoreFile {
            items: self.items.clone(),
            pinboards: self.pinboards.clone(),
            settings: self.settings.clone(),
        };
        if let Ok(json) = serde_json::to_string(&sf) {
            let tmp = self.file.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.file);
            }
        }
    }

    /* ---------- capture pipeline (§2.4) ---------- */

    /// Dedupe-refresh or unshift + trim. Returns the captured item (which may
    /// not survive trimming when it is not in the keep set).
    pub fn add_item(&mut self, mut item: ClipItem) -> ClipItem {
        if item.stack_order.is_none() {
            if let Some(idx) = self.items.iter().position(|i| {
                i.is_dedupe_candidate() && i.item_type == item.item_type && i.content == item.content
            }) {
                let mut existing = self.items.remove(idx);
                existing.created_at = item.created_at;
                existing.source_app = item.source_app.take();
                self.items.insert(0, existing);
                let out = self.items[0].clone();
                self.trim();
                return out;
            }
        }
        self.items.insert(0, item.clone());
        self.trim();
        item
    }

    /// §5.2: kept = keep (list order) ++ overflow[0 .. maxItems - |keep|].
    fn trim(&mut self) {
        let max = self.settings.max_items as usize;
        let mut keep: Vec<ClipItem> = Vec::new();
        let mut overflow: Vec<ClipItem> = Vec::new();
        for i in self.items.drain(..) {
            if i.is_keep() {
                keep.push(i);
            } else {
                overflow.push(i);
            }
        }
        let budget = max.saturating_sub(keep.len());
        keep.extend(overflow.into_iter().take(budget));
        self.items = keep;
    }

    pub fn find(&self, id: &str) -> Option<&ClipItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn update(&mut self, id: &str, f: impl FnOnce(&mut ClipItem)) -> Option<ClipItem> {
        let it = self.items.iter_mut().find(|i| i.id == id)?;
        f(it);
        Some(it.clone())
    }

    pub fn remove(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
    }

    pub fn clear_history(&mut self) {
        self.items.retain(|i| i.is_keep());
    }

    /// §5.1: listed ids first (in given order), unlisted keep relative order after.
    pub fn reorder(&mut self, ids: &[String]) {
        let mut ordered = Vec::new();
        let mut placed = std::collections::HashSet::new();
        for id in ids {
            if let Some(pos) = self.items.iter().position(|i| &i.id == id && !placed.contains(&i.id)) {
                let it = self.items.remove(pos);
                placed.insert(it.id.clone());
                ordered.push(it);
            }
        }
        ordered.extend(self.items.drain(..));
        self.items = ordered;
    }

    /* ---------- stack (§6) ---------- */

    pub fn push_stack(&mut self, id: &str) -> Option<ClipItem> {
        let len = self.items.iter().filter(|i| i.stack_order.is_some()).count() as i64;
        self.update(id, |i| i.stack_order = Some(len + 1))
    }

    pub fn pop_stack(&mut self, dir: StackDir) -> Option<ClipItem> {
        let pick = |items: &[ClipItem]| -> Option<usize> {
            items
                .iter()
                .enumerate()
                .filter(|(_, i)| i.stack_order.is_some())
                .min_by_key(|(n, i)| match dir {
                    StackDir::Fifo => (i.stack_order.unwrap_or(0), *n),
                    StackDir::Lifo => (-(i.stack_order.unwrap_or(0)), *n),
                })
                .map(|(n, _)| n)
        };
        let idx = pick(&self.items)?;
        Some(self.items.remove(idx))
    }

    pub fn unstack(&mut self, id: &str) -> Option<ClipItem> {
        self.update(id, |i| i.stack_order = None)
    }

    pub fn clear_stack(&mut self) {
        for i in &mut self.items {
            i.stack_order = None;
        }
    }

    pub fn stack_items(&self) -> Vec<ClipItem> {
        let mut v: Vec<ClipItem> =
            self.items.iter().filter(|i| i.stack_order.is_some()).cloned().collect();
        v.sort_by_key(|i| i.stack_order.unwrap_or(0));
        v
    }

    /* ---------- pinboards ---------- */

    pub fn add_pinboard(&mut self, name: &str) -> Pinboard {
        let pb = Pinboard {
            id: uuid_v4ish(),
            name: name.to_string(),
            icon: "star".into(),
            position: self.pinboards.len() as u32,
        };
        self.pinboards.push(pb.clone());
        pb
    }

    pub fn remove_pinboard(&mut self, id: &str) {
        self.pinboards.retain(|p| p.id != id);
        for i in &mut self.items {
            if i.pinboard_id.as_deref() == Some(id) {
                i.pinboard_id = None;
            }
        }
    }

    /* ---------- export / import (§5.3) ---------- */

    pub fn export_json(&self) -> String {
        serde_json::json!({
            "version": 1,
            "exportedAt": now_iso8601(),
            "items": self.items,
            "pinboards": self.pinboards,
        })
        .to_string()
    }

    pub fn import_json(&mut self, raw: &str) -> ImportCounts {
        let parsed: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => return ImportCounts { imported: 0, skipped: 1 },
        };
        let incoming = parsed.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut imported = 0usize;
        let mut skipped = 0usize;
        for raw_item in &incoming {
            let item = match ClipItem::from_import(raw_item) {
                Ok(mut item) => {
                    item.stack_order = None;
                    item
                }
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let key = item.dedupe_key();
            if self.items.iter().any(|i| i.dedupe_key() == key || i.id == item.id) {
                skipped += 1;
                continue;
            }
            self.items.push(item);
            imported += 1;
        }
        if let Some(pbs) = parsed.get("pinboards").and_then(|v| v.as_array()) {
            for pb in pbs {
                if let Ok(p) = serde_json::from_value::<Pinboard>(pb.clone()) {
                    if !self.pinboards.iter().any(|x| x.id == p.id) {
                        self.pinboards.push(p);
                    }
                }
            }
        }
        if imported > 0 {
            self.sort_newest_first();
            self.trim();
        }
        ImportCounts { imported, skipped }
    }

    pub fn sort_newest_first(&mut self) {
        self.items.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    }

    pub fn recent(&self, n: usize) -> Vec<ClipItem> {
        self.items.iter().take(n).cloned().collect()
    }
}

/// RFC 3339 UTC from unix seconds (dependency-free civil-from-days).
pub fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601_from_unix(secs)
}

pub fn iso8601_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Dependency-free unique id: timestamp + counter + address mix.
pub fn uuid_v4ish() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let addr = &n as *const _ as u64;
    format!("{t:016x}-{n:08x}-{addr:08x}")
}
