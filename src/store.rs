//! SQLite 历史库。
//!
//! 表名与列名沿用 iCopy `data.sqlite` 的 `t_data`，并增量添加 `digest` 列用于去重。
//! `pastetype` 的数值与 `data` 列的序列化格式（这里是 JSON）是本项目自定的，**不能与 iCopy 的库互读**。
//! 图片以 PNG 文件保存在 `tempFiles/`，库里只记录文件名，目录布局仿照 iCopy。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value};

use crate::model::{Item, ItemKind, Tag, digest_parts};

pub const IMAGE_DIR: &str = "tempFiles";
pub const DB_FILE: &str = "data.sqlite";

pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Clone, Debug)]
pub struct Query {
    pub tag: Tag,
    pub text: Option<String>,
    pub offset: usize,
    pub limit: usize,
    /// 只返回晚于该时间的条目（收藏条目不受影响）。
    pub cutoff: Option<f64>,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            tag: Tag::All,
            text: None,
            offset: 0,
            limit: 50,
            cutoff: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted(i64),
    /// 内容已存在，仅把它提到最前。
    Bumped(i64),
}

impl InsertOutcome {
    pub fn id(&self) -> i64 {
        match self {
            Self::Inserted(id) | Self::Bumped(id) => *id,
        }
    }
}

pub struct Store {
    conn: Connection,
    root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join(IMAGE_DIR))
            .with_context(|| format!("创建数据目录失败: {}", root.display()))?;
        let conn = Connection::open(root.join(DB_FILE)).context("打开 data.sqlite 失败")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // 守护进程与选择面板进程会同时访问，WAL 让读写互不阻塞。
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "create table if not exists t_data (
                id integer primary key autoincrement,
                pastetype integer,
                sourcetype string,
                text string,
                filepath string,
                createtime double,
                data blob,
                favorite integer
            );",
        )?;
        let has_digest = conn
            .prepare("pragma table_info(t_data)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|c| c == "digest");
        if !has_digest {
            conn.execute("alter table t_data add column digest text", [])?;
        }
        conn.execute_batch(
            "create index if not exists idx_t_data_createtime on t_data (createtime);
             create index if not exists idx_t_data_digest on t_data (digest);",
        )?;
        Ok(Store {
            conn,
            root: root.to_owned(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn insert(&mut self, item: &Item) -> Result<InsertOutcome> {
        let existing: Option<i64> = self
            .conn
            .query_row(
                "select id from t_data where digest = ? limit 1",
                [&item.digest],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.touch(id, item.created_at)?;
            return Ok(InsertOutcome::Bumped(id));
        }
        let data = serde_json::to_vec(item)?;
        self.conn.execute(
            "insert into t_data (pastetype, sourcetype, text, filepath, createtime, data, favorite, digest)
             values (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                item.kind.as_i64(),
                item.source_app.clone().unwrap_or_default(),
                item.text.clone().unwrap_or_default(),
                item.filepath_column(),
                item.created_at,
                data,
                item.favorite as i64,
                item.digest,
            ],
        )?;
        Ok(InsertOutcome::Inserted(self.conn.last_insert_rowid()))
    }

    pub fn touch(&self, id: i64, now: f64) -> Result<()> {
        self.conn.execute(
            "update t_data set createtime = ? where id = ?",
            rusqlite::params![now, id],
        )?;
        Ok(())
    }

    pub fn set_favorite(&self, id: i64, favorite: bool) -> Result<()> {
        self.conn.execute(
            "update t_data set favorite = ? where id = ?",
            rusqlite::params![favorite as i64, id],
        )?;
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Option<Item>> {
        self.conn
            .query_row(
                "select id, createtime, favorite, data from t_data where id = ?",
                [id],
                row_to_item,
            )
            .optional()?
            .transpose()
    }

    pub fn latest(&self) -> Result<Option<Item>> {
        self.conn
            .query_row(
                "select id, createtime, favorite, data from t_data order by createtime desc, id desc limit 1",
                [],
                row_to_item,
            )
            .optional()?
            .transpose()
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("select count(*) from t_data", [], |r| r.get(0))?)
    }

    pub fn query(&self, q: &Query) -> Result<Vec<Item>> {
        let (where_sql, mut params) = build_filter(q);
        let sql = format!(
            "select id, createtime, favorite, data from t_data {where_sql}
             order by createtime desc, id desc limit ? offset ?"
        );
        params.push(Value::Integer(q.limit as i64));
        params.push(Value::Integer(q.offset as i64));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), row_to_item)?;
        let mut out = Vec::new();
        for r in rows {
            match r? {
                Ok(item) => out.push(item),
                // 单条数据损坏不应让整个历史不可用。
                Err(e) => eprintln!("跳过损坏的历史条目: {e:#}"),
            }
        }
        Ok(out)
    }

    pub fn delete(&mut self, id: i64) -> Result<bool> {
        let file: Option<Option<String>> = self
            .conn
            .query_row(
                "select filepath from t_data where id = ? and pastetype = ?",
                rusqlite::params![id, ItemKind::Image.as_i64()],
                |r| r.get(0),
            )
            .optional()?;
        let n = self.conn.execute("delete from t_data where id = ?", [id])?;
        if let Some(Some(name)) = file {
            self.remove_image_file(&name);
        }
        Ok(n > 0)
    }

    /// 按标签清空；`keep_favorites` 为真时保留收藏。返回删除条数。
    pub fn clear(&mut self, tag: Tag, keep_favorites: bool) -> Result<usize> {
        let q = Query {
            tag,
            limit: usize::MAX >> 1,
            ..Query::default()
        };
        let (where_sql, params) = build_filter(&q);
        let mut sql = format!("select id from t_data {where_sql}");
        if keep_favorites && tag != Tag::Favorite {
            sql.push_str(if where_sql.is_empty() {
                " where favorite = 0"
            } else {
                " and favorite = 0"
            });
        }
        let ids: Vec<i64> = {
            let mut stmt = self.conn.prepare(&sql)?;
            stmt.query_map(params_from_iter(params), |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in &ids {
            self.delete(*id)?;
        }
        Ok(ids.len())
    }

    /// 删除早于 `cutoff` 的非收藏条目（对应 iCopy 的“存储时间限制”）。
    pub fn purge_before(&mut self, cutoff: f64) -> Result<usize> {
        let ids: Vec<i64> = {
            let mut stmt = self
                .conn
                .prepare("select id from t_data where createtime <= ? and favorite = 0")?;
            stmt.query_map([cutoff], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in &ids {
            self.delete(*id)?;
        }
        Ok(ids.len())
    }

    /// 图片总占用超过 `max_bytes` 时，从最旧的非收藏图片开始删除（对应“占用磁盘空间大小限制”）。
    pub fn enforce_disk_limit(&mut self, max_bytes: u64) -> Result<usize> {
        let mut total = dir_size(&self.root.join(IMAGE_DIR));
        if total <= max_bytes {
            return Ok(0);
        }
        let candidates: Vec<(i64, String)> = {
            let mut stmt = self.conn.prepare(
                "select id, filepath from t_data where pastetype = ? and favorite = 0
                 order by createtime asc, id asc",
            )?;
            stmt.query_map([ItemKind::Image.as_i64()], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut removed = 0;
        for (id, name) in candidates {
            if total <= max_bytes {
                break;
            }
            let size = fs::metadata(self.image_path(&name))
                .map(|m| m.len())
                .unwrap_or(0);
            self.delete(id)?;
            total = total.saturating_sub(size);
            removed += 1;
        }
        Ok(removed)
    }

    // ---- 图片文件 ----

    /// 把 RGBA 像素编码为 PNG 写入 `tempFiles/`。文件名由像素摘要决定，相同图片不会重复写盘。
    /// 返回 (文件名, 内容摘要)。
    pub fn save_png(&self, rgba: &[u8], width: u32, height: u32) -> Result<(String, String)> {
        let digest = image_digest(rgba, width, height);
        let name = format!("img_{}.png", &digest[..24]);
        let path = self.root.join(IMAGE_DIR).join(&name);
        if !path.exists() {
            let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())
                .context("图片像素长度与尺寸不符")?;
            let tmp = path.with_extension("png.part");
            img.save_with_format(&tmp, image::ImageFormat::Png)?;
            fs::rename(&tmp, &path)?;
        }
        Ok((name, digest))
    }

    /// 解析库里记录的图片文件名。只取文件名部分，避免被损坏数据带出数据目录。
    pub fn image_path(&self, name: &str) -> PathBuf {
        let base = Path::new(name)
            .file_name()
            .map(|s| s.to_owned())
            .unwrap_or_default();
        self.root.join(IMAGE_DIR).join(base)
    }

    pub fn read_image(&self, name: &str) -> Result<(Vec<u8>, u32, u32)> {
        let img = image::open(self.image_path(name))?.into_rgba8();
        let (w, h) = img.dimensions();
        Ok((img.into_raw(), w, h))
    }

    fn remove_image_file(&self, name: &str) {
        if name.is_empty() {
            return;
        }
        let _ = fs::remove_file(self.image_path(name));
    }
}

/// 图片内容摘要：尺寸 + 像素。监控在落盘前就用它判断重复 / 自写入。
pub fn image_digest(rgba: &[u8], width: u32, height: u32) -> String {
    let dims = [width.to_le_bytes(), height.to_le_bytes()].concat();
    digest_parts(ItemKind::Image, &[&dims, rgba])
}

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Item>> {
    let id: i64 = r.get(0)?;
    let created: f64 = r.get(1)?;
    let favorite: i64 = r.get(2)?;
    let data: Vec<u8> = r.get(3)?;
    Ok(serde_json::from_slice::<Item>(&data)
        .map(|mut it| {
            // 这三列是可变状态的唯一来源，data 里的副本可能已过期。
            it.id = id;
            it.created_at = created;
            it.favorite = favorite != 0;
            it
        })
        .with_context(|| format!("条目 {id} 的 data 无法解析")))
}

fn build_filter(q: &Query) -> (String, Vec<Value>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if let Some(kinds) = q.tag.kinds() {
        let ph = vec!["?"; kinds.len()].join(", ");
        clauses.push(format!("pastetype in ({ph})"));
        params.extend(kinds.iter().map(|k| Value::Integer(k.as_i64())));
    }
    if q.tag == Tag::Favorite {
        clauses.push("favorite = 1".to_owned());
    }
    if let Some(text) = q.text.as_deref().filter(|t| !t.is_empty()) {
        let pat = format!("%{}%", escape_like(text));
        clauses.push("(text like ? escape '\\' or filepath like ? escape '\\')".to_owned());
        params.push(Value::Text(pat.clone()));
        params.push(Value::Text(pat));
    }
    if let (Some(cutoff), true) = (q.cutoff, q.tag != Tag::Favorite) {
        clauses.push("(createtime > ? or favorite = 1)".to_owned());
        params.push(Value::Real(cutoff));
    }

    let sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("where {}", clauses.join(" and "))
    };
    (sql, params)
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn dir_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        (dir, s)
    }

    fn text(s: &str, t: f64) -> Item {
        Item::text(s.into(), None, Some("app".into()), t)
    }

    #[test]
    fn insert_then_query_newest_first() {
        let (_d, mut s) = store();
        s.insert(&text("one", 1.0)).unwrap();
        s.insert(&text("two", 2.0)).unwrap();
        let got = s.query(&Query::default()).unwrap();
        assert_eq!(
            got.iter()
                .map(|i| i.text.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["two", "one"]
        );
    }

    #[test]
    fn duplicate_content_bumps_instead_of_inserting() {
        let (_d, mut s) = store();
        let a = s.insert(&text("same", 1.0)).unwrap();
        s.insert(&text("other", 2.0)).unwrap();
        let b = s.insert(&text("same", 3.0)).unwrap();
        assert_eq!(a.id(), b.id());
        assert!(matches!(b, InsertOutcome::Bumped(_)));
        assert_eq!(s.count().unwrap(), 2);
        assert_eq!(
            s.query(&Query::default()).unwrap()[0].text.as_deref(),
            Some("same")
        );
    }

    #[test]
    fn bump_keeps_favorite() {
        let (_d, mut s) = store();
        let id = s.insert(&text("keep", 1.0)).unwrap().id();
        s.set_favorite(id, true).unwrap();
        s.insert(&text("keep", 2.0)).unwrap();
        assert!(s.get(id).unwrap().unwrap().favorite);
    }

    #[test]
    fn search_matches_text_and_escapes_like_wildcards() {
        let (_d, mut s) = store();
        s.insert(&text("100% sure", 1.0)).unwrap();
        s.insert(&text("1000 sure", 2.0)).unwrap();
        let q = Query {
            text: Some("100%".into()),
            ..Query::default()
        };
        let got = s.query(&q).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text.as_deref(), Some("100% sure"));
    }

    #[test]
    fn search_matches_file_paths() {
        let (_d, mut s) = store();
        s.insert(&Item::files(
            vec!["/home/u/report.pdf".into()],
            false,
            None,
            1.0,
        ))
        .unwrap();
        let q = Query {
            text: Some("report".into()),
            ..Query::default()
        };
        assert_eq!(s.query(&q).unwrap().len(), 1);
    }

    #[test]
    fn tags_filter_by_kind_and_favorite() {
        let (_d, mut s) = store();
        s.insert(&text("t", 1.0)).unwrap();
        s.insert(&Item::text("r".into(), Some("<b>r</b>".into()), None, 2.0))
            .unwrap();
        let f = s
            .insert(&Item::files(
                vec!["/a".into(), "/b".into()],
                false,
                None,
                3.0,
            ))
            .unwrap()
            .id();
        s.set_favorite(f, true).unwrap();

        let by = |tag| {
            s.query(&Query {
                tag,
                ..Query::default()
            })
            .unwrap()
            .len()
        };
        assert_eq!(by(Tag::All), 3);
        assert_eq!(by(Tag::Text), 2);
        assert_eq!(by(Tag::File), 1);
        assert_eq!(by(Tag::Image), 0);
        assert_eq!(by(Tag::Favorite), 1);
    }

    #[test]
    fn cutoff_hides_old_but_never_favorites() {
        let (_d, mut s) = store();
        s.insert(&text("old", 10.0)).unwrap();
        let fav = s.insert(&text("old-fav", 11.0)).unwrap().id();
        s.set_favorite(fav, true).unwrap();
        s.insert(&text("new", 100.0)).unwrap();
        let q = Query {
            cutoff: Some(50.0),
            ..Query::default()
        };
        let got: Vec<_> = s
            .query(&q)
            .unwrap()
            .into_iter()
            .map(|i| i.text.unwrap())
            .collect();
        assert_eq!(got, ["new", "old-fav"]);
    }

    #[test]
    fn paging_offsets() {
        let (_d, mut s) = store();
        for i in 0..5 {
            s.insert(&text(&format!("n{i}"), i as f64)).unwrap();
        }
        let q = Query {
            offset: 2,
            limit: 2,
            ..Query::default()
        };
        let got: Vec<_> = s
            .query(&q)
            .unwrap()
            .into_iter()
            .map(|i| i.text.unwrap())
            .collect();
        assert_eq!(got, ["n2", "n1"]);
    }

    #[test]
    fn purge_before_spares_favorites() {
        let (_d, mut s) = store();
        s.insert(&text("old", 1.0)).unwrap();
        let fav = s.insert(&text("old-fav", 2.0)).unwrap().id();
        s.set_favorite(fav, true).unwrap();
        s.insert(&text("new", 100.0)).unwrap();
        assert_eq!(s.purge_before(50.0).unwrap(), 1);
        assert_eq!(s.count().unwrap(), 2);
    }

    #[test]
    fn clear_can_keep_favorites() {
        let (_d, mut s) = store();
        s.insert(&text("a", 1.0)).unwrap();
        let f = s.insert(&text("b", 2.0)).unwrap().id();
        s.set_favorite(f, true).unwrap();
        assert_eq!(s.clear(Tag::All, true).unwrap(), 1);
        assert_eq!(s.count().unwrap(), 1);
        assert_eq!(s.clear(Tag::All, false).unwrap(), 1);
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn image_roundtrip_dedupes_file_and_delete_removes_it() {
        let (_d, mut s) = store();
        let px = vec![255u8; 4 * 4 * 4];
        let (name, digest) = s.save_png(&px, 4, 4).unwrap();
        let (name2, digest2) = s.save_png(&px, 4, 4).unwrap();
        assert_eq!((&name, &digest), (&name2, &digest2));
        let id = s
            .insert(&Item::image(name.clone(), (4, 4), digest, None, 1.0))
            .unwrap()
            .id();
        let (back, w, h) = s.read_image(&name).unwrap();
        assert_eq!((w, h, back), (4, 4, px));
        assert!(s.image_path(&name).exists());
        s.delete(id).unwrap();
        assert!(!s.image_path(&name).exists());
    }

    #[test]
    fn disk_limit_removes_oldest_non_favorite_images_first() {
        let (_d, mut s) = store();
        let mut ids = Vec::new();
        for i in 0u8..3 {
            let px = vec![i; 64 * 64 * 4];
            let (name, digest) = s.save_png(&px, 64, 64).unwrap();
            ids.push(
                s.insert(&Item::image(name, (64, 64), digest, None, i as f64))
                    .unwrap()
                    .id(),
            );
        }
        s.set_favorite(ids[0], true).unwrap();
        let removed = s.enforce_disk_limit(0).unwrap();
        assert_eq!(removed, 2);
        assert!(s.get(ids[0]).unwrap().is_some());
        assert!(s.get(ids[1]).unwrap().is_none());
        assert!(s.get(ids[2]).unwrap().is_none());
    }

    #[test]
    fn image_path_cannot_escape_data_dir() {
        let (_d, s) = store();
        let p = s.image_path("../../etc/passwd");
        assert_eq!(p, s.root().join(IMAGE_DIR).join("passwd"));
    }

    #[test]
    fn corrupt_row_is_skipped_not_fatal() {
        let (_d, mut s) = store();
        s.insert(&text("ok", 1.0)).unwrap();
        s.conn
            .execute(
                "insert into t_data (pastetype, createtime, data, favorite, digest) values (0, 2.0, x'00', 0, 'bad')",
                [],
            )
            .unwrap();
        let got = s.query(&Query::default()).unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn reopening_existing_db_keeps_data_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = Store::open(dir.path()).unwrap();
            s.insert(&text("persist", 1.0)).unwrap();
        }
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.count().unwrap(), 1);
    }
}
