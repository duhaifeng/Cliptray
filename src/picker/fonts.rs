//! 界面字体。egui 自带字体不含中日韩字形，iCopy 用的是系统字体（SF + 苹方），
//! 这里运行时找系统字体：macOS 用 SF 显示拉丁字母、苹方显示中文；Linux 用 Noto CJK 等。
//!
//! 系统 CJK 字体动辄几十 MB，面板每次弹出都是新进程，所以用 mmap 而不是整份读进内存。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// 字体文件及 ttc 内的字体序号。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub path: PathBuf,
    pub index: u32,
}

const KNOWN: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
    "/usr/share/fonts/wqy-microhei/wqy-microhei.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    "/usr/share/fonts/adobe-source-han-sans/SourceHanSansCN-Regular.otf",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
];

const NAME_HINTS: &[&str] = &[
    "cjk",
    "wqy",
    "sourcehan",
    "wenquanyi",
    "sarasa",
    "notosanssc",
    "notosanscjk",
];
const SEARCH_ROOTS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];

/// 苹方从 macOS 10.15 起作为可下载资源放在 AssetsV2 下，目录名是内容哈希，只能遍历查找。
const MAC_ASSET_ROOT: &str = "/System/Library/AssetsV2";
/// PingFang.ttc 里 “PingFang SC Regular” 的序号（0 是港版，3 是简体中文）。
const PINGFANG_SC_REGULAR: u32 = 3;
const MAC_LATIN: &str = "/System/Library/Fonts/SFNS.ttf";

pub fn find_cjk_font() -> Option<FontFile> {
    if let Some(path) = find_pingfang(Path::new(MAC_ASSET_ROOT)) {
        return Some(FontFile {
            path,
            index: PINGFANG_SC_REGULAR,
        });
    }
    let whole = |path: PathBuf| FontFile { path, index: 0 };
    if let Some(p) = KNOWN.iter().map(PathBuf::from).find(|p| p.is_file()) {
        return Some(whole(p));
    }
    let home_fonts = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/fonts"));
    SEARCH_ROOTS
        .iter()
        .map(PathBuf::from)
        .chain(home_fonts)
        .find_map(|root| walk(&root, 4))
        .map(whole)
}

fn find_pingfang(root: &Path) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("com_apple_MobileAsset_Font")
        })
        .find_map(|bucket| {
            std::fs::read_dir(bucket.path())
                .ok()?
                .flatten()
                .map(|asset| asset.path().join("AssetData/PingFang.ttc"))
                .find(|p| p.is_file())
        })
}

/// 深度受限的目录遍历，只看文件名里带 CJK 字体特征的文件。
fn walk(dir: &Path, depth: u32) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            dirs.push(p);
            continue;
        }
        let name = p.file_name()?.to_string_lossy().to_ascii_lowercase();
        let is_font = [".ttf", ".otf", ".ttc"]
            .iter()
            .any(|ext| name.ends_with(ext));
        if is_font && NAME_HINTS.iter().any(|h| name.contains(h)) {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter().find_map(|d| walk(&d, depth - 1))
}

/// 只读映射整个文件，映射与进程同寿命（面板进程很短命）。
fn map_file(path: &Path) -> Option<&'static [u8]> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len() as usize;
    if len == 0 {
        return None;
    }
    // SAFETY: 只读私有映射；系统字体文件在面板存活期间不会被截断。
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) })
}

fn font_data(f: &FontFile) -> Option<FontData> {
    let mut data = FontData::from_static(map_file(&f.path)?);
    data.index = f.index;
    Some(data)
}

/// 安装系统字体；返回是否有 CJK 字体可用。
pub fn install_system_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    let mut primary = Vec::new();
    if let Some(data) = font_data(&FontFile {
        path: MAC_LATIN.into(),
        index: 0,
    }) {
        fonts.font_data.insert("latin".to_owned(), Arc::new(data));
        primary.push("latin");
    }
    let cjk = find_cjk_font().and_then(|f| font_data(&f));
    let have_cjk = cjk.is_some();
    if let Some(data) = cjk {
        fonts.font_data.insert("cjk".to_owned(), Arc::new(data));
        primary.push("cjk");
    }
    // 系统字体放在最前；egui 自带字体留在后面兜底 emoji 和符号。
    let list = fonts.families.entry(FontFamily::Proportional).or_default();
    for (i, name) in primary.iter().enumerate() {
        list.insert(i, (*name).to_owned());
    }
    if have_cjk {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    have_cjk
}
