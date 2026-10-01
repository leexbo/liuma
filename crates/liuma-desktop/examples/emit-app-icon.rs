//! emit-app-icon:logo.svg → macOS iconset 的 10 张 PNG
//! (scripts/package-macos 消费,iconutil -c icns 出 .icns)。
//!
//! 构图与运行时 Dock 图标(app_icon.rs)同构:824 网格纯黑圆角方身
//! (圆角 22.5%,四周透明)+ 方身内 4% 边距内框 + logo `meet` 居中
//! ——Finder 等处显示的是本产物,须与 Dock 观感一致(裸 SVG 透明底
//! 直出会「没有底」)。不走 qlmanage 离线转换(SVG 输出全黑,见
//! app_icon.rs),栅格化用 workspace 已锁定的 resvg;每档尺寸从矢量
//! 直接渲染而非位图缩放,16px 小档不因 1024→16 降采样糊掉轮廓。
//!
//! 用法:cargo run -p liuma-desktop --example emit-app-icon -- <输出目录>

use anyhow::{Context, anyhow};
use std::path::PathBuf;
use std::{fs, process::ExitCode};

/// Apple iconset 规范:(像素边长,iconset 文件名)。16/32/128/256/512
/// 五档各配 @2x;512x512@2x = 1024,与源 viewBox 同尺寸。
const ICONSET: [(u32, &str); 10] = [
    (16, "icon_16x16.png"),
    (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"),
    (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"),
    (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"),
    (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"),
    (1024, "icon_512x512@2x.png"),
];

/// 构图常量(1024 基准比例,与 app_icon.rs 同源):Apple 模板 824
/// 方身 / 圆角 22.5% 方身边长 / 方身内 4% 边距;底色纯黑(BG_RGB)
const GRID_FRAC: f32 = 824.0 / 1024.0;
const RADIUS_FRAC: f32 = 0.225;
const INSET_FRAC: f32 = 0.04;

/// 圆角矩形路径(8 段完整回路,底边先行):缺边会让角弧起笔错位、
/// 切掉角(app_icon.rs add_rounded_rect 的实测教训),与 CG 版同序。
fn rounded_rect(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Option<resvg::tiny_skia::Path> {
    let k = 0.5523 * r;
    let mut pb = resvg::tiny_skia::PathBuilder::new();
    pb.move_to(x0 + r, y0);
    pb.line_to(x1 - r, y0);
    pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    pb.line_to(x0 + r, y1);
    pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
    pb.line_to(x0, y0 + r);
    pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
    pb.close();
    pb.finish()
}

/// 单档位图:黑圆角方身 + logo meet 进内框;PNG 字节。
/// 与 pixmap 同为 y 向下坐标系,SVG 无需翻转(app_icon.rs 走 CG 才翻转)。
fn render_one(tree: &usvg::Tree, base: f32, side: u32) -> anyhow::Result<Vec<u8>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)
        .ok_or_else(|| anyhow!("pixmap 分配失败({side}px)"))?;
    let side = side as f32;
    let grid = GRID_FRAC * side;
    let m = (side - grid) / 2.0;
    let rect = rounded_rect(m, m, side - m, side - m, RADIUS_FRAC * grid)
        .ok_or_else(|| anyhow!("圆角矩形路径构建失败"))?;
    let paint = resvg::tiny_skia::Paint {
        anti_alias: true,
        ..resvg::tiny_skia::Paint::default()
    };
    pixmap.fill_path(
        &rect,
        &paint,
        resvg::tiny_skia::FillRule::Winding,
        resvg::tiny_skia::Transform::identity(),
        None,
    );
    // 方形 viewBox 对方形内框,meet 缩放一次 + 平移即居中
    let inset = INSET_FRAC * grid;
    let scale = (side - 2.0 * (m + inset)) / base;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(m + inset, m + inset),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().context("PNG 编码失败")
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("emit-app-icon: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let out_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("用法:emit-app-icon <输出目录>"))?;
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("创建输出目录 {} 失败", out_dir.display()))?;

    // 与查看器的 SVG 管线(kits/mermaid.rs)同库同构:usvg 解析一次,
    // 各档尺寸只换根变换缩放
    let logo = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.svg"))
        .context("读取 assets/logo.svg 失败")?;
    let tree =
        usvg::Tree::from_data(&logo, &usvg::Options::default()).context("解析 logo.svg 失败")?;
    let base = tree.size().width();

    for (px, name) in ICONSET {
        let png = render_one(&tree, base, px)?;
        fs::write(out_dir.join(name), &png)
            .with_context(|| format!("写出 {} 失败", out_dir.join(name).display()))?;
    }
    Ok(())
}
