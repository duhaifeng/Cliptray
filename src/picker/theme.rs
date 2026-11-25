//! iCopy 横向剪切板的尺寸与配色，取自 iCopy 1.x 在 macOS 上的运行时截图（逻辑点，2x 截图已换算）。

use eframe::egui::Color32;

use crate::model::{ItemKind, Tag};

pub const BOARD_H: f32 = 350.0;
pub const WINDOW_RADIUS: u8 = 16;
pub const TOOLBAR_H: f32 = 50.0;
pub const TOOLBAR_BG: Color32 = Color32::from_rgb(0x4B, 0x49, 0x4A);
pub const CONTENT_BG: Color32 = Color32::from_rgb(0x6B, 0x68, 0x69);

pub const SEARCH_W: f32 = 121.0;
pub const SEARCH_H: f32 = 25.0;
pub const SEARCH_BG: Color32 = Color32::from_rgb(0x5D, 0x5B, 0x5C);
pub const SEARCH_HINT: Color32 = Color32::from_rgb(0xB6, 0xB5, 0xB5);
/// 搜索框与第一个分类标签之间。
pub const SEARCH_GAP: f32 = 12.0;

pub const PILL_H: f32 = 20.0;
pub const PILL_RADIUS: u8 = 5;
pub const PILL_GAP: f32 = 8.0;
/// 标签文字左右留白（“All” 标签实测 27.5 点宽）。
pub const PILL_PAD: f32 = 5.5;
pub const PILL_FONT: f32 = 11.0;

pub const TOOL_ICON: f32 = 16.0;
pub const TOOL_GAP: f32 = 12.0;
pub const TOOL_RIGHT: f32 = 16.0;
pub const TOOL_ICON_COLOR: Color32 = Color32::from_rgb(0xE6, 0xE6, 0xE6);

pub const CARD: f32 = 278.0;
pub const CARD_TOP: f32 = 61.0;
pub const CARD_LEFT: f32 = 8.0;
pub const CARD_GAP: f32 = 10.0;
pub const CARD_STRIDE: f32 = CARD + CARD_GAP;
pub const CARD_RADIUS: u8 = 10;
pub const HEAD_H: f32 = 60.0;
pub const FOOT_H: f32 = 24.0;
pub const BODY_BG: Color32 = Color32::from_rgb(0x45, 0x45, 0x45);
pub const BODY_TEXT: Color32 = Color32::from_rgb(0xE3, 0xE3, 0xE3);
pub const BODY_PAD: f32 = 8.0;
pub const SELECTED_BORDER: f32 = 2.0;
pub const FOOT_FONT: f32 = 12.5;
pub const APP_ICON: f32 = 25.0;
pub const SEND_BLUE: Color32 = Color32::from_rgb(0x45, 0x9A, 0xF8);
pub const COPY_GRAY: Color32 = Color32::from_rgb(0xAB, 0xAB, 0xAB);
pub const MISSING_RED: Color32 = Color32::from_rgb(0xFF, 0x5F, 0x57);

const PINK: Color32 = Color32::from_rgb(0xE6, 0x9E, 0xD8);
const GREEN: Color32 = Color32::from_rgb(0x82, 0xD7, 0x83);
const YELLOW: Color32 = Color32::from_rgb(0xEE, 0xC9, 0x70);
const BLUE: Color32 = Color32::from_rgb(0x77, 0xC3, 0xE8);
const PURPLE: Color32 = Color32::from_rgb(0xC3, 0x99, 0xF0);

/// 分类标签颜色。未选中时是标签色叠在工具栏底色上的暗色（实测值）。
pub fn tag_color(tag: Tag, selected: bool) -> Color32 {
    match (tag, selected) {
        (Tag::All, true) => PINK,
        (Tag::Text, true) => GREEN,
        (Tag::Image, true) => YELLOW,
        (Tag::File, true) => BLUE,
        (Tag::Favorite, true) => PURPLE,
        (Tag::All, false) => Color32::from_rgb(119, 96, 114),
        (Tag::Text, false) => Color32::from_rgb(91, 112, 90),
        (Tag::Image, false) => Color32::from_rgb(121, 108, 84),
        (Tag::File, false) => Color32::from_rgb(87, 107, 118),
        (Tag::Favorite, false) => Color32::from_rgb(109, 95, 120),
    }
}

/// 卡片头部颜色，同时用于选中边框和序号。
pub fn kind_color(kind: ItemKind) -> Color32 {
    match kind {
        ItemKind::Text | ItemKind::RichText => GREEN,
        ItemKind::Image => YELLOW,
        ItemKind::File | ItemKind::Folder | ItemKind::Files => BLUE,
    }
}

/// 第 `idx` 张卡片相对内容区左缘的横坐标。
pub fn card_x(idx: usize) -> f32 {
    CARD_LEFT + idx as f32 * CARD_STRIDE
}

/// 所有卡片排开后的总宽度（含两侧边距）。
pub fn strip_width(n: usize) -> f32 {
    if n == 0 {
        0.0
    } else {
        card_x(n - 1) + CARD + CARD_LEFT
    }
}

/// 视口 `[scroll, scroll + width)` 内至少部分可见的卡片下标范围。
pub fn visible_cards(scroll: f32, width: f32, n: usize) -> std::ops::Range<usize> {
    if n == 0 || width <= 0.0 {
        return 0..0;
    }
    let first = ((scroll - CARD_LEFT - CARD) / CARD_STRIDE)
        .floor()
        .max(-1.0) as isize
        + 1;
    let last = ((scroll + width - CARD_LEFT) / CARD_STRIDE).ceil().max(0.0) as usize;
    (first.max(0) as usize).min(n)..last.min(n)
}

/// 让第 `idx` 张卡片完整可见所需的最小滚动量调整，返回新的滚动位置。
pub fn scroll_to_reveal(scroll: f32, width: f32, idx: usize) -> f32 {
    let left = card_x(idx) - CARD_LEFT;
    let right = card_x(idx) + CARD + CARD_LEFT;
    if left < scroll {
        left
    } else if right > scroll + width {
        right - width
    } else {
        scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_positions_match_icopy_measurements() {
        assert_eq!(card_x(0), 8.0);
        assert_eq!(card_x(1), 296.0);
        assert_eq!(strip_width(6), 8.0 + 5.0 * 288.0 + 278.0 + 8.0);
        assert_eq!(strip_width(0), 0.0);
    }

    #[test]
    fn visible_range_covers_partial_cards_only() {
        assert_eq!(visible_cards(0.0, 1728.0, 100), 0..6);
        assert_eq!(visible_cards(0.0, 1728.0, 3), 0..3);
        // 第 0 张卡片右缘在 286，滚过 286 后它就不可见了。
        assert_eq!(visible_cards(287.0, 600.0, 100).start, 1);
        assert_eq!(visible_cards(285.0, 600.0, 100).start, 0);
        assert_eq!(visible_cards(0.0, 0.0, 10), 0..0);
    }

    #[test]
    fn reveal_scrolls_minimally() {
        assert_eq!(scroll_to_reveal(0.0, 1000.0, 1), 0.0);
        let s = scroll_to_reveal(0.0, 1000.0, 5);
        assert_eq!(s, card_x(5) + CARD + CARD_LEFT - 1000.0);
        assert_eq!(scroll_to_reveal(s, 1000.0, 0), 0.0);
    }
}
