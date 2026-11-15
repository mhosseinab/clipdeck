//! SQLite-backed history. Only settings, boards and requested result pages live in RAM.
use crate::crypto::{self, EncryptionKey};
use crate::types::{ClipItem, ItemType, Pinboard, Settings};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

pub const PAGE_SIZE: usize = 64;
pub const DISPLAY_CONTENT_CHARS: usize = 4096;
pub const SQLITE_CACHE_KIB: usize = 2048;
pub const NATIVE_FILE: &str = "clipdeck.sqlite3";
const FULL_COLUMNS: &str = "c.id,c.kind,b.content,b.preview,b.title,c.source_app,c.created_at,c.size_bytes,c.pinned,c.board,c.stack_order,c.image_file";
const SUMMARY_COLUMNS: &str = "c.id,c.kind,c.excerpt,c.preview,c.title,substr(c.source_app,1,256),c.created_at,c.size_bytes,c.pinned,c.board,c.stack_order,c.image_file";
const PROTECTED: &str = "(pinned=1 OR board IS NOT NULL OR stack_order IS NOT NULL)";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ClipFilter {
    #[default]
    All,
    Pinned,
    Board(String),
}
#[derive(Debug, Clone, Default)]
pub struct ItemPage {
    pub items: Vec<ClipItem>,
    pub total: usize,
    pub offset: usize,
}
#[derive(Debug, Clone, Default)]
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
    pub pinboards: Vec<Pinboard>,
    pub settings: Settings,
    pub image_dir: PathBuf,
    conn: Connection,
    file: PathBuf,
    saved_settings: String,
    saved_boards: String,
    key: Option<EncryptionKey>,
}

pub struct StoreReader {
    conn: Connection,
}
impl StoreReader {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::open_with_key(path, None)
    }
    pub fn open_with_key(path: &Path, key: Option<&EncryptionKey>) -> rusqlite::Result<Self> {
        let conn = crypto::open_connection(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            key,
        )?;
        Ok(Self { conn })
    }
    pub fn page(
        &self,
        query: &str,
        filter: &ClipFilter,
        offset: usize,
        limit: usize,
    ) -> rusqlite::Result<ItemPage> {
        page(&self.conn, query, filter, offset, limit)
    }
}
fn atomic<T>(conn: &Connection, f: impl FnOnce() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    conn.execute_batch("SAVEPOINT clipdeck_mutation")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE clipdeck_mutation")?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK TO clipdeck_mutation; RELEASE clipdeck_mutation");
            Err(error)
        }
    }
}
fn digest(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}
fn excerpt(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}
fn row_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipItem> {
    let kind: String = row.get(1)?;
    let item_type = match kind.as_str() {
        "text" => ItemType::Text,
        "link" => ItemType::Link,
        "code" => ItemType::Code,
        "color" => ItemType::Color,
        "image" => ItemType::Image,
        "file" => ItemType::File,
        "html" => ItemType::Html,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(ClipItem {
        id: row.get(0)?,
        item_type,
        content: row.get(2)?,
        preview: row.get(3)?,
        title: row.get(4)?,
        source_app: row.get(5)?,
        created_at: row.get(6)?,
        size_bytes: row.get::<_, i64>(7)? as u64,
        pinned: row.get(8)?,
        pinboard_id: row.get(9)?,
        stack_order: row.get(10)?,
        image_file: row.get(11)?,
    })
}
fn page(
    conn: &Connection,
    query: &str,
    filter: &ClipFilter,
    offset: usize,
    limit: usize,
) -> rusqlite::Result<ItemPage> {
    let snapshot = conn.unchecked_transaction()?;
    let result = page_rows(&snapshot, query, filter, offset, limit)?;
    snapshot.commit()?;
    Ok(result)
}
fn page_rows(
    conn: &Connection,
    query: &str,
    filter: &ClipFilter,
    offset: usize,
    limit: usize,
) -> rusqlite::Result<ItemPage> {
    let (predicate, board) = match filter {
        ClipFilter::All => ("1", ""),
        ClipFilter::Pinned => ("c.pinned=1", ""),
        ClipFilter::Board(id) => ("c.board=?2", id.as_str()),
    };
    let lower = query.to_lowercase();
    let from = if lower.is_empty() {
        "clips c"
    } else {
        "clips c INDEXED BY clip_order JOIN bodies b ON b.id=c.id"
    };
    let search = if lower.is_empty() {
        ""
    } else {
        " AND (instr(b.title_lower,?1)>0 OR instr(b.preview_lower,?1)>0 OR instr(b.content_lower,?1)>0)"
    };
    let condition = format!("{predicate}{search}");
    // Named positional parameters remain present even for an empty query/filter.
    let condition = format!("({condition}) AND (?1 IS NOT NULL) AND (?2 IS NOT NULL)");
    let total = conn.query_row(
        &format!("SELECT count(*) FROM {from} WHERE {condition}"),
        params![lower, board],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let limit = limit.min(PAGE_SIZE);
    if limit == 0 || offset >= total {
        return Ok(ItemPage {
            items: vec![],
            total,
            offset,
        });
    }
    // OFFSET skips only indexed ordering keys. Payload/excerpt loading begins at
    // the selected key and uses a keyset range, never a full in-memory result.
    let anchor: (i64,String) = conn.query_row(&format!("SELECT c.position,c.id FROM {from} WHERE {condition} ORDER BY c.position,c.id LIMIT 1 OFFSET ?3"),params![lower,board,offset as i64],|r| Ok((r.get(0)?,r.get(1)?)))?;
    let sql = format!("SELECT {SUMMARY_COLUMNS} FROM {from} WHERE {condition} AND (c.position,c.id)>=(?3,?4) ORDER BY c.position,c.id LIMIT ?5");
    let mut stmt = conn.prepare(&sql)?;
    let items = stmt
        .query_map(
            params![lower, board, anchor.0, anchor.1, limit as i64],
            row_item,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ItemPage {
        items,
        total,
        offset,
    })
}

impl Store {
    /// A new database starts empty. Legacy JSON is deliberately not imported.
    pub fn open(user_data: &Path) -> Self {
        Self::try_open(user_data).unwrap_or_else(|e| {
            panic!(
                "cannot open ClipDeck database at {}: {e}",
                user_data.display()
            )
        })
    }
    pub fn try_open(user_data: &Path) -> rusqlite::Result<Self> {
        Self::try_open_with_key(user_data, None)
    }
    pub fn try_open_with_key(
        user_data: &Path,
        key: Option<EncryptionKey>,
    ) -> rusqlite::Result<Self> {
        std::fs::create_dir_all(user_data.join("images"))
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let io_error = |error: io::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(error));
        std::fs::set_permissions(user_data, std::fs::Permissions::from_mode(0o700))
            .map_err(io_error)?;
        let file = user_data.join(NATIVE_FILE);
        if let Ok(metadata) = std::fs::symlink_metadata(&file) {
            if !metadata.is_file() {
                return Err(io_error(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "SQLite path must be a regular file",
                )));
            }
        }
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&file)
            .map_err(io_error)?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
            .map_err(io_error)?;
        let conn = crypto::open_connection(
            &file,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            key.as_ref(),
        )?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS clips(id TEXT PRIMARY KEY,kind TEXT NOT NULL,title TEXT NOT NULL,preview TEXT NOT NULL,excerpt TEXT NOT NULL,source_app TEXT,created_at INTEGER NOT NULL,size_bytes INTEGER NOT NULL,pinned INTEGER NOT NULL,board TEXT,stack_order INTEGER,image_file TEXT,position INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS clip_order ON clips(position,id);
            CREATE INDEX IF NOT EXISTS clip_board_order ON clips(board,position,id);
            CREATE INDEX IF NOT EXISTS clip_pin_order ON clips(pinned,position,id);
            CREATE INDEX IF NOT EXISTS clip_stack ON clips(stack_order,position,id);
            CREATE INDEX IF NOT EXISTS clip_created_order ON clips(created_at DESC,id);
            CREATE INDEX IF NOT EXISTS clip_image ON clips(image_file);
            CREATE TABLE IF NOT EXISTS bodies(id TEXT PRIMARY KEY,content TEXT NOT NULL,title TEXT NOT NULL,preview TEXT NOT NULL,digest TEXT NOT NULL,content_lower TEXT NOT NULL,title_lower TEXT NOT NULL,preview_lower TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS body_digest ON bodies(digest,id);
            CREATE TABLE IF NOT EXISTS orphan_images(name TEXT PRIMARY KEY);
            PRAGMA user_version=2;")?;
        fn load<T: serde::de::DeserializeOwned + Default>(
            conn: &Connection,
            key: &str,
        ) -> rusqlite::Result<T> {
            let text: Option<String> = conn
                .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
                .optional()?;
            match text {
                None => Ok(T::default()),
                Some(s) => serde_json::from_str(&s).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                }),
            }
        }
        // Only initialize absent metadata. Opening a second connection (for
        // export, for example) must never rewrite a stale loaded snapshot.
        let defaults =
            serde_json::to_string(&Settings::default()).expect("serialize default settings");
        conn.execute(
            "INSERT OR IGNORE INTO meta VALUES('settings',?1)",
            [defaults],
        )?;
        conn.execute("INSERT OR IGNORE INTO meta VALUES('pinboards','[]')", [])?;
        let settings = load(&conn, "settings")?;
        let pinboards = load(&conn, "pinboards")?;
        let saved_settings = serde_json::to_string(&settings).expect("serialize loaded settings");
        let saved_boards = serde_json::to_string(&pinboards).expect("serialize loaded boards");
        let store = Self {
            conn,
            file,
            image_dir: user_data.join("images"),
            settings,
            pinboards,
            saved_settings,
            saved_boards,
            key,
        };
        store.cleanup_images();
        Ok(store)
    }
    pub fn encryption_key(&self) -> Option<EncryptionKey> {
        self.key.clone()
    }
    fn image_path(&self, name: &str) -> Option<PathBuf> {
        let mut parts = Path::new(name).components();
        if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
            return None;
        }
        if !std::fs::symlink_metadata(&self.image_dir).ok()?.is_dir() {
            return None;
        }
        let path = self.image_dir.join(name);
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if !metadata.is_file() {
                return None;
            }
        }
        Some(path)
    }
    pub fn image_bytes(&self, name: &str) -> Option<Vec<u8>> {
        crypto::read_image(&self.image_path(name)?, self.key.as_ref()).ok()
    }
    pub fn image_info(&self, name: &str) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(self.image_path(name)?).ok()?;
        let stamp = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos() as u64;
        Some((metadata.len(), stamp ^ metadata.ino().rotate_left(32)))
    }
    pub fn put_image(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.write_image(name, bytes.to_vec())
    }
    fn write_image(&self, name: &str, bytes: Vec<u8>) -> io::Result<()> {
        let path = self.image_path(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "image key must be a regular basename",
            )
        })?;
        crypto::write_image(&path, bytes, self.key.as_ref())
    }
    pub fn add_item_with_image(&mut self, item: ClipItem, image: Option<Vec<u8>>) -> ClipItem {
        if let (Some(name), Some(bytes)) = (item.image_file.as_deref(), image) {
            self.write_image(name, bytes)
                .expect("persist clipboard image")
        }
        self.add_item(item)
    }
    pub fn path(&self) -> &Path {
        &self.file
    }
    pub fn save(&mut self) {
        let settings = serde_json::to_string(&self.settings).expect("serialize settings");
        let boards = serde_json::to_string(&self.pinboards).expect("serialize pinboards");
        if settings == self.saved_settings && boards == self.saved_boards {
            return;
        }
        atomic(&self.conn,|| {
            if settings!=self.saved_settings { self.conn.execute("INSERT INTO meta VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&settings])?; }
            if boards!=self.saved_boards { self.conn.execute("INSERT INTO meta VALUES('pinboards',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&boards])?; }
            Ok(())
        }).expect("save ClipDeck settings");
        self.saved_settings = settings;
        self.saved_boards = boards;
    }
    pub fn page(&self, query: &str, filter: &ClipFilter, offset: usize, limit: usize) -> ItemPage {
        page(&self.conn, query, filter, offset, limit).expect("query history page")
    }
    pub fn len(&self) -> usize {
        self.conn
            .query_row("SELECT count(*) FROM clips", [], |r| r.get::<_, i64>(0))
            .expect("count clips") as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn stacked_count(&self) -> usize {
        self.conn
            .query_row(
                "SELECT count(*) FROM clips WHERE stack_order IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0),
            )
            .expect("count stack") as usize
    }
    pub fn contains_id(&self, id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM clips WHERE id=?1)",
                [id],
                |r| r.get(0),
            )
            .expect("find clip id")
    }
    pub fn contains_content(&self, kind: ItemType, content: &str) -> bool {
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM bodies b JOIN clips c ON c.id=b.id WHERE b.digest=?1 AND c.kind=?2 AND b.content=?3)",params![digest(content),kind.as_str(),content],|r|r.get(0)).expect("check duplicate clip")
    }
    pub fn summary(&self, id: &str) -> Option<ClipItem> {
        self.conn
            .query_row(
                &format!("SELECT {SUMMARY_COLUMNS} FROM clips c WHERE c.id=?1"),
                [id],
                row_item,
            )
            .optional()
            .expect("load clip summary")
    }
    pub fn find(&self, id: &str) -> Option<ClipItem> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {FULL_COLUMNS} FROM clips c JOIN bodies b ON b.id=c.id WHERE c.id=?1"
                ),
                [id],
                row_item,
            )
            .optional()
            .expect("load clip content")
    }
    fn put(&self, item: &ClipItem, position: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO bodies VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                item.id,
                item.content,
                item.title,
                item.preview,
                digest(&item.content),
                item.content.to_lowercase(),
                item.title.to_lowercase(),
                item.preview.to_lowercase()
            ],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO clips VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                item.id,
                item.item_type.as_str(),
                excerpt(&item.title, 256),
                excerpt(&item.preview, 512),
                excerpt(&item.content, DISPLAY_CONTENT_CHARS),
                item.source_app,
                item.created_at,
                item.size_bytes as i64,
                item.pinned,
                item.pinboard_id,
                item.stack_order,
                item.image_file,
                position
            ],
        )?;
        Ok(())
    }
    fn edge_position(&self, first: bool) -> i64 {
        let sql = if first {
            "SELECT COALESCE(MIN(position),0)-1 FROM clips"
        } else {
            "SELECT COALESCE(MAX(position),0)+1 FROM clips"
        };
        self.conn
            .query_row(sql, [], |r| r.get(0))
            .expect("read clip order")
    }
    pub fn add_item(&mut self, item: ClipItem) -> ClipItem {
        let saved=atomic(&self.conn,|| {
            let id:Option<String>=if item.stack_order.is_none() {
                self.conn.query_row("SELECT c.id FROM bodies b JOIN clips c ON c.id=b.id WHERE b.digest=?1 AND c.kind=?2 AND b.content=?3 AND c.stack_order IS NULL ORDER BY c.position,c.id LIMIT 1",params![digest(&item.content),item.item_type.as_str(),item.content],|r|r.get(0)).optional()?
            } else {None};
            let saved=if let Some(id)=id {
                self.conn.execute("UPDATE clips SET created_at=?1,source_app=?2,position=?3 WHERE id=?4",params![item.created_at,item.source_app,self.edge_position(true),id])?;
                self.find(&id).expect("dedupe clip exists")
            } else {self.put(&item,self.edge_position(true))?;item};
            self.trim();Ok(saved)
        }).expect("record clipboard item");
        self.cleanup_images();
        saved
    }
    pub fn insert_imported(&mut self, mut item: ClipItem) -> bool {
        if self.contains_id(&item.id) || self.contains_content(item.item_type, &item.content) {
            return false;
        }
        item.stack_order = None;
        atomic(&self.conn, || self.put(&item, self.edge_position(false)))
            .expect("import clipboard item");
        true
    }
    fn cleanup_images(&self) {
        if !self.conn.is_autocommit() {
            return;
        }
        loop {
            let name: Option<String> = self
                .conn
                .query_row("SELECT name FROM orphan_images LIMIT 1", [], |r| r.get(0))
                .optional()
                .expect("read image cleanup queue");
            let Some(name) = name else { break };
            let referenced: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM clips WHERE image_file=?1)",
                    [&name],
                    |r| r.get(0),
                )
                .expect("check image reference");
            let mut components = Path::new(&name).components();
            let safe = matches!(components.next(), Some(Component::Normal(_)))
                && components.next().is_none();
            if !referenced && safe {
                if let Some(path) = self.image_path(&name) {
                    let _ = std::fs::remove_file(path);
                }
            }
            self.conn
                .execute("DELETE FROM orphan_images WHERE name=?1", [name])
                .expect("finish image cleanup");
        }
    }
    fn delete_ids(&self, ids: &[String]) -> rusqlite::Result<()> {
        for id in ids {
            self.conn.execute("INSERT OR IGNORE INTO orphan_images SELECT image_file FROM clips WHERE id=?1 AND image_file IS NOT NULL",[id])?;
            self.conn.execute("DELETE FROM bodies WHERE id=?1", [id])?;
            self.conn.execute("DELETE FROM clips WHERE id=?1", [id])?;
        }
        Ok(())
    }
    fn trim(&self) {
        let protected: i64 = self
            .conn
            .query_row(
                &format!("SELECT count(*) FROM clips WHERE {PROTECTED}"),
                [],
                |r| r.get(0),
            )
            .expect("count protected clips");
        let budget = (self.settings.max_items as usize).saturating_sub(protected as usize);
        let mut excess = self
            .len()
            .saturating_sub(protected as usize)
            .saturating_sub(budget);
        atomic(&self.conn,|| {
            while excess>0 {
                let mut stmt=self.conn.prepare(&format!("SELECT id FROM clips WHERE NOT {PROTECTED} ORDER BY position DESC,id DESC LIMIT ?1"))?;
                let ids=stmt.query_map([excess.min(PAGE_SIZE) as i64],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                if ids.is_empty(){break}
                self.delete_ids(&ids)?;excess-=ids.len();
            }
            Ok(())
        }).expect("trim history");
        self.cleanup_images();
    }
    pub fn remove(&mut self, id: &str) {
        atomic(&self.conn,|| {
            self.conn.execute("INSERT OR IGNORE INTO orphan_images SELECT image_file FROM clips WHERE id=?1 AND image_file IS NOT NULL",[id])?;
            self.conn.execute("DELETE FROM bodies WHERE id=?1",[id])?;self.conn.execute("DELETE FROM clips WHERE id=?1",[id])?;Ok(())
        }).expect("delete clip");
        self.cleanup_images();
    }
    pub fn clear_history(&mut self) {
        atomic(&self.conn,|| {
            loop {
                let mut stmt=self.conn.prepare(&format!("SELECT id FROM clips WHERE NOT {PROTECTED} ORDER BY position,id LIMIT {PAGE_SIZE}"))?;
                let ids=stmt.query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                if ids.is_empty(){break}
                self.delete_ids(&ids)?;
            }
            Ok(())
        }).expect("clear history");
        self.cleanup_images();
    }
    pub fn update(&mut self, id: &str, f: impl FnOnce(&mut ClipItem)) -> Option<ClipItem> {
        let mut item = self.find(id)?;
        let old_image = item.image_file.clone();
        f(&mut item);
        let position: i64 = self
            .conn
            .query_row("SELECT position FROM clips WHERE id=?1", [id], |r| r.get(0))
            .expect("clip order");
        atomic(&self.conn, || {
            self.put(&item, position)?;
            if old_image != item.image_file {
                if let Some(name) = &old_image {
                    self.conn
                        .execute("INSERT OR IGNORE INTO orphan_images VALUES(?1)", [name])?;
                }
            }
            Ok(())
        })
        .expect("update clip");
        self.cleanup_images();
        Some(item)
    }
    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> bool {
        self.conn
            .execute(
                "UPDATE clips SET pinned=?1 WHERE id=?2",
                params![pinned, id],
            )
            .expect("pin clip")
            > 0
    }
    pub fn set_board(&mut self, id: &str, board: Option<&str>) -> bool {
        self.conn
            .execute("UPDATE clips SET board=?1 WHERE id=?2", params![board, id])
            .expect("move clip")
            > 0
    }
    pub fn update_content(&mut self, id: &str, content: &str) -> bool {
        let kind: Option<String> = self
            .conn
            .query_row("SELECT kind FROM clips WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .expect("read clip type");
        let Some(kind) = kind else { return false };
        let item_type: ItemType =
            serde_json::from_value(Value::String(kind)).expect("stored clip type");
        let (title, preview) = crate::derive_pair(item_type, content, None);
        atomic(&self.conn,|| {
            self.conn.execute("UPDATE bodies SET content=?1,title=?2,preview=?3,digest=?4,content_lower=?5,title_lower=?6,preview_lower=?7 WHERE id=?8",params![content,title,preview,digest(content),content.to_lowercase(),title.to_lowercase(),preview.to_lowercase(),id])?;
            self.conn.execute("UPDATE clips SET title=?1,preview=?2,excerpt=?3 WHERE id=?4",params![excerpt(&title,256),excerpt(&preview,512),excerpt(content,DISPLAY_CONTENT_CHARS),id])?;
            Ok(())
        }).expect("edit clip content");
        true
    }
    pub fn reorder(&mut self, ids: &[String]) {
        let minimum = self.edge_position(true) + 1;
        let base = minimum.saturating_sub(ids.len() as i64).saturating_sub(1);
        atomic(&self.conn, || {
            for (n, id) in ids.iter().enumerate() {
                // Already placed IDs are below the original minimum, so a
                // duplicate input ID is skipped without an in-memory ID set.
                self.conn.execute(
                    "UPDATE clips SET position=?1 WHERE id=?2 AND position>=?3",
                    params![base + n as i64, id, minimum],
                )?;
            }
            Ok(())
        })
        .expect("reorder clips");
    }
    pub fn sort_newest_first(&mut self) {
        atomic(&self.conn, || {
            let mut select = self.conn.prepare(
                "SELECT id FROM clips INDEXED BY clip_created_order ORDER BY created_at DESC,id",
            )?;
            let mut update = self
                .conn
                .prepare("UPDATE clips SET position=?1 WHERE id=?2")?;
            for (n, id) in select
                .query_map([], |row| row.get::<_, String>(0))?
                .enumerate()
            {
                update.execute(params![n as i64, id?])?;
            }
            Ok(())
        })
        .expect("sort imported clips");
    }
    pub fn push_stack(&mut self, id: &str) -> Option<ClipItem> {
        let n = self.stacked_count() as i64;
        self.conn
            .execute(
                "UPDATE clips SET stack_order=?1 WHERE id=?2",
                params![n + 1, id],
            )
            .expect("stack clip");
        self.summary(id)
    }
    pub fn peek_stack(&self, dir: StackDir) -> Option<ClipItem> {
        let direction = if dir == StackDir::Fifo { "ASC" } else { "DESC" };
        self.conn.query_row(&format!("SELECT {FULL_COLUMNS} FROM clips c JOIN bodies b ON b.id=c.id WHERE c.stack_order IS NOT NULL ORDER BY c.stack_order {direction},c.position,c.id LIMIT 1"),[],row_item).optional().expect("read next stack clip")
    }
    pub fn pop_stack(&mut self, dir: StackDir) -> Option<ClipItem> {
        let item = self.peek_stack(dir)?;
        self.remove(&item.id);
        Some(item)
    }
    pub fn unstack(&mut self, id: &str) -> Option<ClipItem> {
        self.conn
            .execute("UPDATE clips SET stack_order=NULL WHERE id=?1", [id])
            .expect("unstack clip");
        self.summary(id)
    }
    pub fn clear_stack(&mut self) {
        atomic(&self.conn,|| {
            loop {
                let mut select=self.conn.prepare(&format!("SELECT id FROM clips WHERE stack_order IS NOT NULL ORDER BY stack_order,position,id LIMIT {PAGE_SIZE}"))?;
                let ids=select.query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                if ids.is_empty(){break}
                for id in ids {self.conn.execute("UPDATE clips SET stack_order=NULL WHERE id=?1",[id])?;}
            }
            Ok(())
        }).expect("clear stack");
    }
    pub fn stack_page(&self, limit: usize) -> Vec<ClipItem> {
        let mut stmt=self.conn.prepare(&format!("SELECT {SUMMARY_COLUMNS} FROM clips c WHERE stack_order IS NOT NULL ORDER BY stack_order,position,id LIMIT ?1")).expect("stack page");
        stmt.query_map([limit.min(PAGE_SIZE) as i64], row_item)
            .expect("stack rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("decode stack page")
    }
    pub fn stack_items(&self) -> Vec<ClipItem> {
        self.stack_page(PAGE_SIZE)
    }
    pub fn add_pinboard(&mut self, name: &str) -> Pinboard {
        let board = Pinboard {
            id: uuid_v4ish(),
            name: name.into(),
            icon: "star".into(),
            position: self.pinboards.len() as u32,
        };
        self.pinboards.push(board.clone());
        self.save();
        board
    }
    pub fn remove_pinboard(&mut self, id: &str) {
        let boards: Vec<_> = self
            .pinboards
            .iter()
            .filter(|board| board.id != id)
            .cloned()
            .collect();
        let serialized = serde_json::to_string(&boards).expect("serialize remaining boards");
        atomic(&self.conn,|| {
            self.conn.execute("UPDATE clips SET board=NULL WHERE board=?1",[id])?;
            self.conn.execute("INSERT INTO meta VALUES('pinboards',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&serialized])?;
            Ok(())
        }).expect("delete pinboard");
        self.pinboards = boards;
        self.saved_boards = serialized;
    }
    pub fn recent(&self, n: usize) -> Vec<ClipItem> {
        self.page("", &ClipFilter::All, 0, n).items
    }
    /// Explicit all-record access for fixture comparisons; production lists use pages.
    pub fn all_items(&self) -> Vec<ClipItem> {
        let mut stmt=self.conn.prepare(&format!("SELECT {FULL_COLUMNS} FROM clips c JOIN bodies b ON b.id=c.id ORDER BY c.position,c.id")).expect("all clips query");
        stmt.query_map([], row_item)
            .expect("all clips rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("decode all clips")
    }
    pub fn export_to_writer(&self, mut out: impl Write) -> io::Result<()> {
        fn sql(error: rusqlite::Error) -> io::Error {
            io::Error::other(error)
        }
        let snapshot = self.conn.unchecked_transaction().map_err(sql)?;
        let boards: String = snapshot
            .query_row("SELECT value FROM meta WHERE key='pinboards'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql)?
            .unwrap_or_else(|| "[]".into());
        out.write_all(b"{\"version\":1,\"exportedAt\":")?;
        serde_json::to_writer(&mut out, &now_iso8601())?;
        out.write_all(b",\"items\":[")?;
        {
            let mut stmt=snapshot.prepare(&format!("SELECT {FULL_COLUMNS} FROM clips c JOIN bodies b ON b.id=c.id ORDER BY c.position,c.id")).map_err(sql)?;
            let rows = stmt.query_map([], row_item).map_err(sql)?;
            for (n, item) in rows.enumerate() {
                if n > 0 {
                    out.write_all(b",")?;
                }
                serde_json::to_writer(&mut out, &item.map_err(sql)?)?;
            }
        }
        out.write_all(b"],\"pinboards\":")?;
        out.write_all(boards.as_bytes())?;
        out.write_all(b"}")?;
        snapshot.commit().map_err(sql)?;
        Ok(())
    }
    pub fn export_json(&self) -> String {
        let mut bytes = Vec::new();
        self.export_to_writer(&mut bytes).expect("export history");
        String::from_utf8(bytes).expect("JSON UTF-8")
    }
    pub fn import_json(&mut self, raw: &str) -> ImportCounts {
        self.import_from_reader(raw.as_bytes())
            .unwrap_or(ImportCounts {
                imported: 0,
                skipped: 1,
            })
    }
    pub fn import_from_reader(&mut self, reader: impl Read) -> serde_json::Result<ImportCounts> {
        let old_boards = self.pinboards.clone();
        self.conn
            .execute_batch("SAVEPOINT import_history")
            .expect("start import");
        let mut counts = ImportCounts::default();
        let mut deserializer = serde_json::Deserializer::from_reader(reader);
        let result = ImportRoot {
            store: self,
            counts: &mut counts,
        }
        .deserialize(&mut deserializer)
        .and_then(|_| deserializer.end());
        if let Err(error) = result {
            self.conn
                .execute_batch("ROLLBACK TO import_history; RELEASE import_history")
                .expect("rollback import");
            self.pinboards = old_boards;
            return Err(error);
        }
        if counts.imported > 0 {
            self.sort_newest_first();
            self.trim();
        }
        let settings = serde_json::to_string(&self.settings).expect("serialize imported settings");
        let boards = serde_json::to_string(&self.pinboards).expect("serialize imported boards");
        self.conn.execute("INSERT INTO meta VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&settings]).expect("persist import settings");
        self.conn.execute("INSERT INTO meta VALUES('pinboards',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&boards]).expect("persist import boards");
        self.conn
            .execute_batch("RELEASE import_history")
            .expect("commit import");
        self.saved_settings = settings;
        self.saved_boards = boards;
        self.cleanup_images();
        Ok(counts)
    }
}

struct ImportRoot<'a> {
    store: &'a mut Store,
    counts: &'a mut ImportCounts,
}
impl<'de> DeserializeSeed<'de> for ImportRoot<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for ImportRoot<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("clipboard export object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "items" => map.next_value_seed(ImportItems {
                    store: self.store,
                    counts: self.counts,
                })?,
                "pinboards" => map.next_value_seed(ImportBoards { store: self.store })?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}
struct ImportItems<'a> {
    store: &'a mut Store,
    counts: &'a mut ImportCounts,
}
impl<'de> DeserializeSeed<'de> for ImportItems<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for ImportItems<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("array of clips")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        while let Some(value) = seq.next_element::<Value>()? {
            match ClipItem::from_import(&value) {
                Ok(item) => {
                    if self.store.insert_imported(item) {
                        self.counts.imported += 1
                    } else {
                        self.counts.skipped += 1
                    }
                }
                Err(_) => self.counts.skipped += 1,
            }
        }
        Ok(())
    }
}
struct ImportBoards<'a> {
    store: &'a mut Store,
}
impl<'de> DeserializeSeed<'de> for ImportBoards<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for ImportBoards<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("array of boards")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        while let Some(value) = seq.next_element::<Value>()? {
            if let Ok(board) = serde_json::from_value::<Pinboard>(value) {
                if !self.store.pinboards.iter().any(|p| p.id == board.id) {
                    self.store.pinboards.push(board)
                }
            }
        }
        Ok(())
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

#[cfg(test)]
mod sqlite_policy_tests {
    use super::*;
    #[test]
    fn reopening_database_never_replays_loaded_metadata() {
        let root = std::env::temp_dir().join(format!("clipdeck-open-policy-{}", uuid_v4ish()));
        {
            let mut store = Store::open(&root);
            store.settings.watch_enabled = false;
            store.save();
        }
        let mut reopened = Store::open(&root);
        assert_eq!(
            reopened.conn.total_changes(),
            0,
            "opening an existing database must not overwrite metadata"
        );
        reopened.save();
        assert_eq!(reopened.conn.total_changes(), 0);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn connection_cache_is_bounded_and_unchanged_settings_do_not_write() {
        let root = std::env::temp_dir().join(format!("clipdeck-cache-policy-{}", uuid_v4ish()));
        let mut store = Store::open(&root);
        let reader = StoreReader::open(store.path()).unwrap();
        for connection in [&store.conn, &reader.conn] {
            assert_eq!(
                connection
                    .query_row("PRAGMA cache_size", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                -(SQLITE_CACHE_KIB as i64)
            );
            assert_eq!(
                connection
                    .query_row("PRAGMA mmap_size", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        let before = store.conn.total_changes();
        store.save();
        assert_eq!(store.conn.total_changes(), before);
        store.settings.watch_enabled = false;
        store.save();
        assert_eq!(store.conn.total_changes(), before + 1);
        drop(reader);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
