//! 面板的数据状态：查询、分页、选中。与 egui 无关，可直接单测。

use anyhow::Result;

use crate::config::Config;
use crate::model::{Item, Tag};
use crate::store::{Query, Store, now_secs};

pub struct PickerState {
    store: Store,
    cfg: Config,
    pub items: Vec<Item>,
    pub selected: usize,
    pub tag: Tag,
    search: String,
    pub has_more: bool,
}

impl PickerState {
    pub fn new(store: Store, cfg: Config, tag: Tag) -> Result<Self> {
        let mut s = PickerState {
            store,
            cfg,
            items: Vec::new(),
            selected: 0,
            tag,
            search: String::new(),
            has_more: false,
        };
        s.reload()?;
        Ok(s)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    fn query(&self, offset: usize) -> Query {
        Query {
            tag: self.tag,
            text: (!self.search.is_empty()).then(|| self.search.clone()),
            offset,
            limit: self.cfg.page_size,
            cutoff: self.cfg.cutoff(now_secs()),
        }
    }

    pub fn reload(&mut self) -> Result<()> {
        self.items = self.store.query(&self.query(0))?;
        self.has_more = self.items.len() >= self.cfg.page_size;
        self.selected = 0;
        Ok(())
    }

    pub fn load_more(&mut self) -> Result<()> {
        if !self.has_more {
            return Ok(());
        }
        let more = self.store.query(&self.query(self.items.len()))?;
        self.has_more = more.len() >= self.cfg.page_size;
        self.items.extend(more);
        Ok(())
    }

    /// 返回搜索词是否发生了变化（变化时已重新查询）。
    pub fn set_search(&mut self, s: &str) -> Result<bool> {
        if s == self.search {
            return Ok(false);
        }
        self.search = s.to_owned();
        self.reload()?;
        Ok(true)
    }

    pub fn set_tag(&mut self, tag: Tag) -> Result<()> {
        if tag != self.tag {
            self.tag = tag;
            self.reload()?;
        }
        Ok(())
    }

    /// 移动选中项并夹在有效范围内；接近列表末尾时顺带加载下一页。
    pub fn move_by(&mut self, delta: isize) -> Result<()> {
        if self.items.is_empty() {
            return Ok(());
        }
        let target = self.selected as isize + delta;
        // 目标超出已加载范围时先翻页再夹取，否则 PageDown 会在分页边界停住。
        while target >= self.items.len() as isize && self.has_more {
            self.load_more()?;
        }
        self.selected = target.clamp(0, self.items.len() as isize - 1) as usize;
        // 接近末尾时预取下一页，滚动时不会看到列表突然变短。
        if self.selected + 3 >= self.items.len() {
            self.load_more()?;
        }
        Ok(())
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.items.len() {
            self.selected = idx;
        }
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// 第 n 项（0 起），供数字键快速粘贴。
    pub fn item_at(&self, n: usize) -> Option<&Item> {
        self.items.get(n)
    }

    pub fn position_of(&self, id: i64) -> Option<usize> {
        self.items.iter().position(|i| i.id == id)
    }

    /// 清空当前分类（iCopy 右键菜单“清空"%@"条目”）。不受搜索词影响。
    pub fn clear_tag(&mut self, keep_favorites: bool) -> Result<usize> {
        let n = self.store.clear(self.tag, keep_favorites)?;
        self.reload()?;
        Ok(n)
    }

    pub fn delete_selected(&mut self) -> Result<()> {
        let Some(id) = self.selected_item().map(|i| i.id) else {
            return Ok(());
        };
        self.store.delete(id)?;
        self.items.remove(self.selected);
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        Ok(())
    }

    pub fn toggle_favorite_selected(&mut self) -> Result<()> {
        let Some(item) = self.items.get(self.selected) else {
            return Ok(());
        };
        let (id, now_fav) = (item.id, !item.favorite);
        self.store.set_favorite(id, now_fav)?;
        if self.tag == Tag::Favorite && !now_fav {
            // 在“收藏”分类里取消收藏，条目应从当前列表消失。
            self.items.remove(self.selected);
            if self.selected >= self.items.len() {
                self.selected = self.items.len().saturating_sub(1);
            }
        } else {
            self.items[self.selected].favorite = now_fav;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(n: usize, page: usize) -> (tempfile::TempDir, PickerState) {
        let d = tempfile::tempdir().unwrap();
        let mut store = Store::open(d.path()).unwrap();
        for i in 0..n {
            store
                .insert(&Item::text(
                    format!("item {i}"),
                    None,
                    None,
                    now_secs() + i as f64,
                ))
                .unwrap();
        }
        let cfg = Config {
            page_size: page,
            ..Config::default()
        };
        (d, PickerState::new(store, cfg, Tag::All).unwrap())
    }

    #[test]
    fn newest_first_and_first_page_only() {
        let (_d, s) = state(10, 4);
        assert_eq!(s.items.len(), 4);
        assert_eq!(s.items[0].text.as_deref(), Some("item 9"));
        assert!(s.has_more);
    }

    #[test]
    fn moving_near_end_loads_next_page_without_duplicates() {
        let (_d, mut s) = state(10, 4);
        s.move_by(3).unwrap();
        assert_eq!(s.items.len(), 8);
        s.move_by(100).unwrap();
        assert_eq!(s.items.len(), 10);
        assert!(!s.has_more);
        let mut ids: Vec<_> = s.items.iter().map(|i| i.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 10);
        assert_eq!(s.selected, 9);
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let (_d, mut s) = state(3, 10);
        s.move_by(-5).unwrap();
        assert_eq!(s.selected, 0);
        s.move_by(5).unwrap();
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn search_resets_selection_and_filters() {
        let (_d, mut s) = state(10, 20);
        s.move_by(5).unwrap();
        assert!(s.set_search("item 3").unwrap());
        assert_eq!(s.selected, 0);
        assert_eq!(s.items.len(), 1);
        assert!(!s.set_search("item 3").unwrap());
    }

    #[test]
    fn delete_keeps_selection_valid() {
        let (_d, mut s) = state(3, 10);
        s.select(2);
        s.delete_selected().unwrap();
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.selected, 1);
        s.delete_selected().unwrap();
        s.delete_selected().unwrap();
        assert!(s.items.is_empty());
        s.delete_selected().unwrap();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn favorite_toggle_and_removal_from_favorite_tab() {
        let (_d, mut s) = state(3, 10);
        s.toggle_favorite_selected().unwrap();
        assert!(s.items[0].favorite);
        s.set_tag(Tag::Favorite).unwrap();
        assert_eq!(s.items.len(), 1);
        s.toggle_favorite_selected().unwrap();
        assert!(s.items.is_empty());
        s.set_tag(Tag::All).unwrap();
        assert!(!s.items[0].favorite, "取消收藏要落到库里");
    }

    #[test]
    fn clear_tag_respects_tag_and_favorites() {
        let (_d, mut s) = state(3, 10);
        s.toggle_favorite_selected().unwrap();
        assert_eq!(s.clear_tag(true).unwrap(), 2);
        assert_eq!(s.items.len(), 1);
        assert!(s.items[0].favorite);
        assert_eq!(s.clear_tag(false).unwrap(), 1);
        assert!(s.items.is_empty());
    }

    #[test]
    fn retention_hides_old_entries_in_picker_too() {
        let d = tempfile::tempdir().unwrap();
        let mut store = Store::open(d.path()).unwrap();
        store
            .insert(&Item::text("ancient".into(), None, None, 1.0))
            .unwrap();
        store
            .insert(&Item::text("fresh".into(), None, None, now_secs()))
            .unwrap();
        let cfg = Config {
            history_days: 1,
            ..Config::default()
        };
        let s = PickerState::new(store, cfg, Tag::All).unwrap();
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].text.as_deref(), Some("fresh"));
    }
}
