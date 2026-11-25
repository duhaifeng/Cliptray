//! 菜单栏图标。画一块剪贴板剪影，不从 iCopy 的 Assets.car 里取像素。
//!
//! macOS 把它当成 template image（只用 alpha，系统自己着色，深浅菜单栏都对）。
//! Linux 的状态栏要彩色，调用方把不透明像素涂成品牌色。
//!
//! 图标用有符号距离场画，4×4 超采样抗锯齿。画布 72 像素，macOS 按 18 点高显示（4 倍），
//! Retina 下也不糊。

/// 画布边长（像素）。
pub const SIZE: u32 = 72;
const SUPERSAMPLE: u32 = 4;

/// 圆角矩形的有符号距离：圆心 `c`、半宽高 `h`、圆角 `r`；内部为负。
fn rounded_box(p: (f32, f32), c: (f32, f32), h: (f32, f32), r: f32) -> f32 {
    let q = ((p.0 - c.0).abs() - (h.0 - r), (p.1 - c.1).abs() - (h.1 - r));
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - r
}

/// 圆头线段的有符号距离：端点 `a`、`b`，线宽 `w`。
fn capsule(p: (f32, f32), a: (f32, f32), b: (f32, f32), w: f32) -> f32 {
    let pa = (p.0 - a.0, p.1 - a.1);
    let ba = (b.0 - a.0, b.1 - a.1);
    let t = ((pa.0 * ba.0 + pa.1 * ba.1) / (ba.0 * ba.0 + ba.1 * ba.1)).clamp(0.0, 1.0);
    ((pa.0 - ba.0 * t).powi(2) + (pa.1 - ba.1 * t).powi(2)).sqrt() - w / 2.0
}

/// 点是否落在图形里（不做抗锯齿）。
fn covered(p: (f32, f32)) -> bool {
    // 板身：描边 6 的圆角矩形框。
    let body = rounded_box(p, (36.0, 40.0), (24.0, 26.0), 9.0).abs() - 3.0 < 0.0;
    // 夹子：实心圆角矩形；周围留 3 像素缝，板身框线在这里断开，夹子看起来压在板上。
    let clip_sd = rounded_box(p, (36.0, 14.0), (13.0, 8.0), 5.0);
    let clip = clip_sd < 0.0;
    let gap = clip_sd < 3.0;
    // 板上两行字。
    let line_a = capsule(p, (26.0, 36.0), (46.0, 36.0), 5.0) < 0.0;
    let line_b = capsule(p, (26.0, 48.0), (38.0, 48.0), 5.0) < 0.0;
    clip || (body && !gap) || line_a || line_b
}

/// 剪贴板剪影，RGBA，不透明处是黑色，抗锯齿体现在 alpha 上。
pub fn clipboard_template_rgba() -> (u32, u32, Vec<u8>) {
    let n = SIZE;
    let mut px = vec![0u8; (n * n * 4) as usize];
    let step = 1.0 / SUPERSAMPLE as f32;
    for y in 0..n {
        for x in 0..n {
            let mut hit = 0u32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let p = (
                        x as f32 + (sx as f32 + 0.5) * step,
                        y as f32 + (sy as f32 + 0.5) * step,
                    );
                    if covered(p) {
                        hit += 1;
                    }
                }
            }
            let alpha = (hit * 255 / (SUPERSAMPLE * SUPERSAMPLE)) as u8;
            px[((y * n + x) * 4 + 3) as usize] = alpha;
        }
    }
    (n, n, px)
}

/// 把模板的不透明像素涂成 `(r, g, b)`，给 Linux 状态栏用。
pub fn colorize(rgba: &[u8], rgb: [u8; 3]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        if px[3] > 0 {
            px[0] = rgb[0];
            px[1] = rgb[1];
            px[2] = rgb[2];
        }
    }
    out
}

/// RGBA 转 ksni 要的 ARGB（每个像素循环右移一字节）。
pub fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.rotate_right(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(px: &[u8], x: u32, y: u32) -> u8 {
        px[((y * SIZE + x) * 4 + 3) as usize]
    }

    #[test]
    fn template_has_clip_body_and_lines_with_clear_corners() {
        let (w, h, px) = clipboard_template_rgba();
        assert_eq!((w, h), (SIZE, SIZE));
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(alpha(&px, 0, 0), 0, "角上是透明的");
        assert_eq!(alpha(&px, 36, 14), 255, "夹子是实心的");
        assert_eq!(alpha(&px, 12, 40), 255, "板身左边框");
        assert_eq!(alpha(&px, 34, 36), 255, "第一行字");
        assert_eq!(alpha(&px, 19, 40), 0, "板身里面留空");
    }

    #[test]
    fn edges_are_antialiased() {
        let (_, _, px) = clipboard_template_rgba();
        let partial = px
            .chunks_exact(4)
            .filter(|p| p[3] > 0 && p[3] < 255)
            .count();
        assert!(partial > 100, "边缘应有半透明像素，实际 {partial}");
    }

    #[test]
    fn clip_is_separated_from_body_by_a_gap() {
        let (_, _, px) = clipboard_template_rgba();
        // 夹子下沿在 y=22，缝 3 像素，板身上框线（y≈14..20 的外圈）在夹子范围内被挖空。
        assert_eq!(alpha(&px, 36, 24), 0, "夹子下方留缝");
    }
}
