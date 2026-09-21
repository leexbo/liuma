//! 双盘主题:深色盘为蓝灰色相家族(内容区深海军蓝、侧栏板岩蓝灰,
//! 对照 Finder 暗色侧栏);浅色盘对照系统亮色。macOS 深盘开毛玻璃:
//! 窗口 background = Transparent,玻璃由应用侧自建 NSVisualEffectView
//! (shell::vibrancy)承担,色调由 Root 层(`c.background` = 半透
//! base)单涂层控制——app 层画布不再自铺 base,否则 alpha 叠涂相加
//! 会吃掉透明度;浅盘涂层不透明,观感与实色一致。色板内联为唯一来源
//! → gpui-component [`ThemeColor`] 映射。设置页「外观」三档(浅色/
//! 深色/跟随系统)经 [`apply`] 实装:启动读 settings.yaml、设置点击
//! 即切、「跟随系统」由窗口外观观察者驱动(shell::store 挂
//! `observe_window_appearance`)。
//!
//! 调用点形态:`theme::BASE()` —— SCREAMING_CASE 取值 fn 保持原常量
//! 调用形态;运行时按 [`MODE`]
//! 原子分发双盘,渲染热路径 = 一次 Relaxed load + 字段拷贝。

#![allow(non_snake_case)]

use std::sync::atomic::{AtomicU8, Ordering};

use gpui_kit::component::{Theme, ThemeMode, ThemeTokens};
use gpui_kit::{App, Rgba, Window, WindowAppearance, WindowBackgroundAppearance, rgba};

// ── 外观档位(与 registry settings.yaml appearance 字段同词汇)──

/// 设置档:浅色 / 深色 / 跟随系统(registry 校验 light/dark/system;
/// 判别值即声明序 as u8,勿重排)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appearance {
    /// 浅色
    Light = 0,
    /// 深色
    Dark = 1,
    /// 跟随系统(窗口外观观察者驱动实时切换)
    System = 2,
}

impl Appearance {
    /// settings.yaml 值 → 档位(未知值回落深色,与 registry 缺省一致)
    pub fn parse(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "system" => Self::System,
            _ => Self::Dark,
        }
    }
}

// ── 正文文本族 ───────────────────────────────────────────────

/// 全局正文字体族(双盘共用)。唯一入口 = `Theme::font_family`:
/// gpui-component 的 `Root` 以 `.font_family(cx.theme().font_family)`
/// 铺满整棵树(root.rs:594),`Theme::typography_tokens()` 的 `sans`
/// 亦由它派生(theme/mod.rs:486-492)→ 一处设置,聊天正文/工具卡/
/// composer/标签全局同源。
///
/// **不可回退到系统字体(`.SystemUIFont`)**:GPUI 折行定价是逐字符
/// 「孤立」量宽(gpui-0.2.2 `text_system/line_wrapper.rs:211-224`
/// `compute_width_for_char` → `layout_line(单字符)`),绘制却按整行
/// shape——`.AppleSystemUIFont` 下 CoreText 对孤立 CJK 标点做行界
/// 压缩:实测 `，` 孤立 **7.082pt** / 行内实宽 **14.082pt**,整行
/// Σ逐字 646.72 vs shape 659.58 = **少算 +12.86pt ≈ 一个全角字**。
/// 折行据此多塞一字,行尾越出后又被 markdown 列表项内容盒的
/// `overflow_hidden` 硬裁(gpui-base `text/node.rs:2424`,裁剪宽 ==
/// 折行宽 = 零余量)→ 真机症状:「key 稳定」的「定」缺一段、后随的
/// 全角逗号整字消失(session 源文可对账)。
/// 显式族实测偏差恒为 +0.00(PingFang SC / Helvetica Neue / Menlo
/// 皆然);钉死 CJK 回退(cascade)无效——压缩是系统字体**当主字体**
/// 时的固有行为,换族是唯一不 fork 的根治路径。
/// 残留:粗体拉丁 span 仍按常规宽计价(Semibold `key` 23.044 vs
/// Regular 22.148 = +0.896pt/处;CJK 粗体同宽),量级 <1pt,不足把
/// 整字推出边界。
pub const FONT_SANS: &str = "PingFang SC";

// ── 双盘色板 ─────────────────────────────────────────────────

/// 一套完整色板(深浅两盘同构;字段语义见各取值 fn 文档)
pub struct Palette {
    pub base: Rgba,
    pub sidebar: Rgba,
    /// 侧栏行 hover(与内容区 hover 分族:板岩底上压海军蓝 hover 显脏)
    pub sidebar_hover: Rgba,
    /// 侧栏行激活/选中(对照 Finder 选中行的同级提亮)
    pub sidebar_active: Rgba,
    /// 标题栏面(深盘与侧栏同色同源,顶条延伸侧栏观感;浅盘纯白)
    pub title_bar: Rgba,
    pub ink: Rgba,
    pub layer: Rgba,
    pub card: Rgba,
    pub dock: Rgba,
    pub brand: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warn: Rgba,
    pub bubble: Rgba,
    pub label: Rgba,
    pub label_2: Rgba,
    pub label_3: Rgba,
    pub caption: Rgba,
    pub border: Rgba,
    pub border_2: Rgba,
    pub code: Rgba,
    pub ongoing: Rgba,
    pub glass_bg: Rgba,
    pub glass_border: Rgba,
    /// 工具卡扫光渐变端点(半透明,亮随暗反色)
    pub sweep: Rgba,
    /// 时间刻度非激活色(半透明,亮随暗反色)
    pub tick_idle: Rgba,
}

/// 不透明 hex → RGBA(常量构造:gpui 的 `rgb()` 非 const,字段为 0..1 f32)
const fn color(hex: u32, a: f32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a,
    }
}

/// 毛玻璃 tint 涂层不透明度:macOS 深盘的 Root 涂层盖住大部分透视,
/// 非 macOS 无模糊落地,保持实色
const WINDOW_TINT_A: f32 = if cfg!(target_os = "macos") { 0.96 } else { 1.0 };

/// 深盘侧栏(用户指定 RGB 36,42,44)。实色:半透 tint 的渲染色随
/// 桌面壁纸漂移,无法对齐指定值。
const DARK_SIDEBAR: Rgba = color(0x242A2C, 1.0);
/// 深盘标题栏(用户指定 RGB 41,45,48;与侧栏分离为双色)
const DARK_TITLE_BAR: Rgba = color(0x292D30, 1.0);

/// 深色盘:蓝灰色相家族——base 深海军蓝(参考图采样 #212734),
/// sidebar 板岩蓝灰(Finder 暗侧栏采样 #253035 一族);macOS 毛玻璃
/// 下 base/sidebar 为半透 tint 涂层,其余表面不透明浮于涂层上。
/// 文字取 label 族 alpha 语义色,语义色取系统色暗形态
const fn dark_palette() -> Palette {
    Palette {
        base: color(0x212734, WINDOW_TINT_A),
        sidebar: DARK_SIDEBAR,
        // hover 沿用旧版相对基色的提亮步长(+5,+5,+4),跟随新中性灰族
        sidebar_hover: color(0x292F30, 1.0),
        // 选中行:比 hover 高一档但收敛亮度(参考实现选中 = hover 同色,
        // 本盘保持三态互异;0x39454C 过亮过蓝,向 hover 靠拢)
        sidebar_active: color(0x333D44, 1.0),
        title_bar: DARK_TITLE_BAR,
        ink: color(0x000000, 1.0),
        layer: color(0x2A3140, 1.0),
        card: color(0x2E3644, 1.0),
        dock: color(0x3A4553, 1.0),
        brand: color(0x0A84FF, 1.0),
        danger: color(0xFF453A, 1.0),
        success: color(0x30D158, 1.0),
        warn: color(0xFF9F0A, 1.0),
        bubble: color(0x303A49, 1.0),
        label: color(0xF9FAFB, 1.0),
        label_2: color(0xEBEBF5, 0.72),
        label_3: color(0xEBEBF5, 0.55),
        caption: color(0xEBEBF5, 0.38),
        border: color(0xFFFFFF, 0.08),
        border_2: color(0xFFFFFF, 0.14),
        code: color(0x191F2B, 1.0),
        ongoing: color(0x0A84FF, 1.0),
        glass_bg: color(0xFFFFFF, 0.10),
        glass_border: color(0xFFFFFF, 0.16),
        sweep: color(0xFFFFFF, 0.07),
        tick_idle: color(0xFFFFFF, 0.22),
    }
}

/// 浅色盘:macOS 亮色系统色(灰阶与深盘同构反演;文字取 label 族
/// alpha 语义色,语义色取系统色亮形态)
const fn light_palette() -> Palette {
    Palette {
        base: color(0xFFFFFF, 1.0),
        sidebar: color(0xF0F0F2, 1.0),
        sidebar_hover: color(0xE9E9EB, 1.0),
        sidebar_active: color(0xDFE1E6, 1.0),
        title_bar: color(0xFFFFFF, 1.0),
        ink: color(0x000000, 1.0),
        layer: color(0xECECEE, 1.0),
        card: color(0xFFFFFF, 1.0),
        dock: color(0xE9E9EB, 1.0),
        brand: color(0x007AFF, 1.0),
        danger: color(0xFF3B30, 1.0),
        success: color(0x34C759, 1.0),
        warn: color(0xFF9500, 1.0),
        bubble: color(0xE9E9EB, 1.0),
        label: color(0x1D1D1F, 1.0),
        label_2: color(0x3C3C43, 0.72),
        label_3: color(0x3C3C43, 0.50),
        caption: color(0x3C3C43, 0.35),
        border: color(0x000000, 0.10),
        border_2: color(0x000000, 0.16),
        code: color(0xF7F7F9, 1.0),
        ongoing: color(0x007AFF, 1.0),
        glass_bg: color(0x000000, 0.06),
        glass_border: color(0x000000, 0.14),
        sweep: color(0x000000, 0.08),
        tick_idle: color(0x000000, 0.22),
    }
}

static PALETTES: [Palette; 2] = [light_palette(), dark_palette()];

// 0 = light / 1 = dark(下标即 PALETTES 下标)
const M_LIGHT: u8 = 0;
const M_DARK: u8 = 1;

/// 当前生效盘(apply 写,取值 fn 读)
static MODE: AtomicU8 = AtomicU8::new(M_DARK);

fn cur() -> &'static Palette {
    &PALETTES[MODE.load(Ordering::Relaxed) as usize]
}

/// 指定模式对应盘(mermaid 纯函数化测试用)
pub(crate) fn palette_of(mode: ThemeMode) -> &'static Palette {
    &PALETTES[if mode.is_dark() {
        M_DARK as usize
    } else {
        M_LIGHT as usize
    }]
}

// ── 取值 fn(调用点保持原常量形态;语义文档在此处)──────────

/// 主背景(深盘深海军蓝,macOS 下为半透毛玻璃 tint 涂层——由 Root
/// 层单次铺底 / 浅盘纯白)
pub fn BASE() -> Rgba {
    cur().base
}
/// 侧栏面板底(深盘板岩蓝灰半透 tint 涂层,对照 Finder 暗侧栏 /
/// 浅盘浅灰;gpui-component list 面同源)
pub fn SIDEBAR() -> Rgba {
    cur().sidebar
}
/// 侧栏行 hover(板岩族,勿用内容区 LAYER 压板岩底)
pub fn SIDEBAR_HOVER() -> Rgba {
    cur().sidebar_hover
}
/// 侧栏行激活/选中(对照 Finder 选中行的同级提亮)
pub fn SIDEBAR_ACTIVE() -> Rgba {
    cur().sidebar_active
}
/// 纯黑双盘锚(轨迹 diff 遮挡罩、gpui-component 侧栏方案色 token;
/// 不随盘反色——遮挡语义恒为暗)
pub fn INK() -> Rgba {
    cur().ink
}
/// 浮层/hover 层底
pub fn LAYER() -> Rgba {
    cur().layer
}
/// 输入卡/卡片底(深盘海军蓝亮一档 / 浅盘纯白——
/// 白画布上灰底显脏,靠边框+阴影分层)
pub fn CARD() -> Rgba {
    cur().card
}
/// 按钮底/次级填充(深盘再亮一档 / 浅盘极浅灰——chip 类填充
/// 在白底上只求隐约成形,过深即灰蒙蒙)
pub fn DOCK() -> Rgba {
    cur().dock
}
/// 品牌蓝(macOS systemBlue:深盘 #0A84FF / 浅盘 #007AFF)
pub fn BRAND() -> Rgba {
    cur().brand
}
/// 危险(systemRed)
pub fn DANGER() -> Rgba {
    cur().danger
}
/// 成功(systemGreen)
pub fn SUCCESS() -> Rgba {
    cur().success
}
/// 警告(systemOrange)
pub fn WARN() -> Rgba {
    cur().warn
}
/// 用户气泡底
pub fn BUBBLE() -> Rgba {
    cur().bubble
}
/// 文字一级(深盘纯白 / 浅盘近黑)
pub fn LABEL() -> Rgba {
    cur().label
}
/// 文字二级(macOS label 族 alpha 语义色)
pub fn LABEL_2() -> Rgba {
    cur().label_2
}
/// 文字三级
pub fn LABEL_3() -> Rgba {
    cur().label_3
}
/// 说明文字
pub fn CAPTION() -> Rgba {
    cur().caption
}
/// 弱边框(近景白/黑 8%/10%)
pub fn BORDER() -> Rgba {
    cur().border
}
/// 二级发丝线(终端卡横幅与输出的分界)
pub fn BORDER_2() -> Rgba {
    cur().border_2
}
/// 代码块/终端卡表面(深盘比 BASE 深一阶 / 浅盘比 BASE 灰一阶)
pub fn CODE() -> Rgba {
    cur().code
}
/// 运行进行色(终端卡 running 状态点/进行态指示;StateDot ongoing
/// 同色,随 BRAND 走 systemBlue)
pub fn ONGOING() -> Rgba {
    cur().ongoing
}

/// 文件类型徽章底色(分类配色:word 蓝 / excel 绿 /
/// ppt 橙 / pdf 红;其余灰阶系。双盘同值——徽章恒为白字彩色方块,
/// 深浅盘上均成立;属图标语义色,非界面分层色)
pub fn FILE_KIND_BADGE(kind: liuma_attachment::FileKind) -> Rgba {
    use liuma_attachment::FileKind as K;
    match kind {
        K::Word => rgba(0x2B579AFF),
        K::Excel => rgba(0x217346FF),
        K::Ppt => rgba(0xC43E1CFF),
        K::Pdf => rgba(0xC74440FF),
        K::Image => rgba(0x0A7EA4FF),
        K::Video => rgba(0x8A50C4FF),
        K::Markdown => rgba(0x4A5A66FF),
        K::Html | K::Code => rgba(0x556875FF),
        K::Other => rgba(0x6E6E73FF),
    }
}
/// 文件类型家族染色(gpui SVG = alpha-mask 单色,彩色渐变图标
/// 无法呈现;家族中饱和色双盘同值可读,属图标语义色,非界面分层色。
/// office 三色与 [`FILE_KIND_BADGE`] 同源)
pub fn FILE_TYPE_TINT(class: crate::kits::filetype::FileClass) -> Rgba {
    use crate::kits::filetype::FileClass as F;
    match class {
        F::Markdown => rgba(0x4C7DB0FF),
        F::Image => rgba(0x0A7EA4FF),
        F::Pdf => rgba(0xC74440FF),
        F::Html => rgba(0xC1603CFF),
        F::Css => rgba(0x3F8FBFFF),
        F::Rust => rgba(0xB7410EFF),
        F::Git => rgba(0xD06142FF),
        F::Json => rgba(0xB39B33FF),
        F::Config => rgba(0x6E7B8AFF),
        F::Env => rgba(0x5D8A46FF),
        F::Lock => rgba(0xB08A3EFF),
        F::Shell => rgba(0x5FA85FFF),
        F::Python => rgba(0x4B8BBEFF),
        F::JsTs => rgba(0x3776C8FF),
        F::Code => rgba(0x5A6B7BFF),
        F::Archive => rgba(0x9A7B4FFF),
        F::Video => rgba(0x7A5CC0FF),
        F::Audio => rgba(0xA060A8FF),
        F::Word => rgba(0x2B579AFF),
        F::Excel => rgba(0x217346FF),
        F::Ppt => rgba(0xC43E1CFF),
        F::Font => rgba(0x8A8F98FF),
        F::Text => rgba(0x7A8694FF),
        F::Other => rgba(0x6E6E73FF),
    }
}

/// 玻璃态填充(激活 tab pill;无 backdrop blur 以半透明近似磨砂)
pub fn GLASS_BG() -> Rgba {
    cur().glass_bg
}
/// 玻璃态描边
pub fn GLASS_BORDER() -> Rgba {
    cur().glass_border
}
/// 工具卡扫光渐变端点(亮随暗反色)
pub fn SWEEP() -> Rgba {
    cur().sweep
}
/// 时间刻度非激活色(亮随暗反色)
pub fn TICK_IDLE() -> Rgba {
    cur().tick_idle
}
/// 全透明(非激活行底;双盘同值)
pub fn TRANSPARENT() -> Rgba {
    color(0x000000, 0.0)
}

// ── 运行时状态与三档应用 ─────────────────────────────────────

/// 用户档位(0/1/2 = Light/Dark/System)
static CHOICE: AtomicU8 = AtomicU8::new(1);
/// 对 NSApp 的强制外观(0 none / 1 light / 2 dark / 255 未设过)
static FORCED: AtomicU8 = AtomicU8::new(255);
/// 上次 apply 的 (档位, 生效盘) —— 幂等守卫,防「强制外观 → 系统
/// 观察者 → 再 apply」回环
static LAST_CHOICE: AtomicU8 = AtomicU8::new(255);
static LAST_MODE: AtomicU8 = AtomicU8::new(255);

/// 当前用户档位(设置页高亮与外观观察者判别用)
pub fn current_appearance() -> Appearance {
    match CHOICE.load(Ordering::Relaxed) {
        0 => Appearance::Light,
        2 => Appearance::System,
        _ => Appearance::Dark,
    }
}

/// 当前生效盘是否深色(自绘语法高亮/轨迹配色的分发开关)
pub fn is_dark() -> bool {
    MODE.load(Ordering::Relaxed) == M_DARK
}

/// 当前生效盘对应 ThemeMode
pub fn mode() -> ThemeMode {
    if is_dark() {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

/// 应用外观档(启动装配与设置页切换同一入口)。
///
/// 流程:System 档先解除 NSApp 强制外观(带着强制值读到的是被强制的
/// 外观,不是真实系统外观)→ `cx.window_appearance()` 解析生效盘 →
/// (档位,生效盘)均未变即幂等返回 → [`Theme::change`] 打底(库默认
/// 盘整铺,会冲掉手改 token)→ 重铺 liuma token → `sync_base`(Base 层
/// 镜像:滚动条等,global_mut 直改不下推)→ 全窗刷新。显式浅/深档同
/// 时强制 NSApp 外观,原生交通灯/边框跟主题,避免「应用深色 + 系统
/// 浅色」的原生 chrome 撕裂。
pub fn apply(choice: Appearance, window: Option<&mut Window>, cx: &mut App) {
    set_forced_appearance(
        match choice {
            Appearance::Light => Some(WindowAppearance::Light),
            Appearance::Dark => Some(WindowAppearance::Dark),
            Appearance::System => None,
        },
        cx,
    );
    // 显式浅/深档直接定盘;仅 System 档读系统外观(此时强制已解除,
    // 读到的是真实系统值)
    let m = match choice {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
        Appearance::System => ThemeMode::from(cx.window_appearance()),
    };
    let mode_flag = u8::from(m.is_dark());
    let choice_flag = choice as u8;
    if LAST_CHOICE.load(Ordering::Relaxed) == choice_flag
        && LAST_MODE.load(Ordering::Relaxed) == mode_flag
    {
        return;
    }
    LAST_CHOICE.store(choice_flag, Ordering::Relaxed);
    LAST_MODE.store(mode_flag, Ordering::Relaxed);
    CHOICE.store(choice_flag, Ordering::Relaxed);
    MODE.store(mode_flag, Ordering::Relaxed);
    Theme::change(m, window, cx);
    apply_tokens(m, cx);
    sync_window_background(cx);
    cx.refresh_windows();
}

/// 测试装配:固定深色盘(UI 测试同源基线;生产入口走 main.rs 直读
/// settings.yaml 档位的 apply)。每个测试是全新 App,而幂等守卫是
/// 进程级——先复位,否则第二个测试的打底会被短路。
#[cfg(test)]
pub fn init(cx: &mut App) {
    LAST_CHOICE.store(255, Ordering::Relaxed);
    LAST_MODE.store(255, Ordering::Relaxed);
    apply(Appearance::Dark, None, cx);
}

/// 生效盘 → 窗口背景特效(纯函数,回归锁决策表):深盘走纯透明,
/// 模糊由应用侧自建的 NSVisualEffectView 承担(shell::vibrancy;
/// gpui 自带 Blurred 路径的视图本机不渲染,探针实证 layer=nil),
/// 浅盘实色;非 macOS 恒实色。开窗装配(main.rs)与盘切换联动
/// ([`sync_window_background`])共用此决策
pub(crate) fn window_background_for(dark: bool) -> WindowBackgroundAppearance {
    if dark && cfg!(target_os = "macos") {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

/// 全窗同步背景特效(盘切换联动;启动期尚未开窗则为空集)。关窗
/// 竞态的 Err 忽略——窗口已死无需外观;新开窗经 [`window_background_for`]
/// 自带你外观
fn sync_window_background(cx: &mut App) {
    let target = window_background_for(is_dark());
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, _| window.set_background_appearance(target));
    }
}

/// NSApp 强制外观(值未变不重设,防观察者空转)
fn set_forced_appearance(target: Option<WindowAppearance>, cx: &mut App) {
    let flag = match target {
        None => 0,
        Some(WindowAppearance::Dark | WindowAppearance::VibrantDark) => 2,
        Some(WindowAppearance::Light | WindowAppearance::VibrantLight) => 1,
    };
    if FORCED.load(Ordering::Relaxed) != flag {
        FORCED.store(flag, Ordering::Relaxed);
        cx.set_window_appearance(target);
    }
}

/// liuma 色板 → gpui-component token(双盘同构映射;`Theme::change`
/// 用库默认盘整铺后必须重铺一遍)。
fn apply_tokens(m: ThemeMode, cx: &mut App) {
    let p = palette_of(m);
    // 品牌/语义色填充面上的前景:双盘均取白(浅盘 label 近黑,不能
    // 用作填充面上的前景)
    let on_fill = if m.is_dark() {
        p.label.into()
    } else {
        color(0xFFFFFF, 1.0).into()
    };
    let t = Theme::global_mut(cx);
    let c = &mut t.colors;
    c.background = p.base.into();
    c.foreground = p.label.into();
    c.border = p.border.into();
    c.input = p.border.into();
    c.caret = p.brand.into();
    c.ring = p.brand.into();
    c.selection = Rgba { a: 0.3, ..p.brand }.into();
    c.primary = p.brand.into();
    c.primary_foreground = on_fill;
    c.primary_hover = p.brand.into();
    c.primary_active = p.brand.into();
    c.secondary = p.dock.into();
    c.secondary_foreground = p.label_2.into();
    c.secondary_hover = p.dock.into();
    c.secondary_active = p.dock.into();
    c.muted = p.layer.into();
    c.muted_foreground = p.label_3.into();
    c.accent = p.layer.into();
    c.accent_foreground = p.label.into();
    c.danger = p.danger.into();
    c.danger_foreground = on_fill;
    c.success = p.success.into();
    c.success_foreground = on_fill;
    c.popover = p.layer.into();
    c.popover_foreground = p.label.into();
    c.list = p.sidebar.into();
    c.list_hover = p.sidebar_hover.into();
    c.list_active = p.sidebar_active.into();
    c.sidebar = p.ink.into();
    c.sidebar_border = p.border.into();
    c.sidebar_foreground = p.label_2.into();
    c.sidebar_accent = p.layer.into();
    c.sidebar_accent_foreground = p.label.into();
    c.scrollbar = p.base.into();
    c.scrollbar_thumb = p.dock.into();
    // 标题栏 = 中性灰面(不随 base 走海军蓝):顶条与画布分色;
    // title_bar_border 同面无边线
    c.title_bar = p.title_bar.into();
    c.title_bar_border = p.title_bar.into();
    t.radius = gpui_kit::px(8.);
    t.radius_lg = gpui_kit::px(12.);
    // colors → tokens 镜像(必须):Root 画布与部分组件读 tokens
    // (语义面),Theme::change 已把库默认色烤进 tokens——漏镜像则
    // 画布永远是库默认底(实测:摘掉 app 层铺底后露出库默认 #0A0A0A)
    t.tokens = ThemeTokens::from(&t.colors);
    // 字体族收口(见 [`FONT_SANS`])。必须在 `Theme::change` 与任何
    // semantic token 应用**之后**:theme/mod.rs:536 的
    // `self.font_family = tokens.typography.sans` 会把族覆盖回库默认
    // `.SystemUIFont`。mono 不动(Menlo):行内代码 chip / 代码块的族
    // 走 markdown highlight 的 font_family,不经此字段。
    t.font_family = FONT_SANS.into();
    // Base 层镜像(滚动条等直接取样 gpui_base::Theme,global_mut 直改不下推)
    Theme::sync_base(cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Hsla, TestAppContext};

    /// 双盘锚定:深盘 = 蓝灰家族采样基值(base 半透毛玻璃涂层,
    /// 非 macOS 实色),浅盘 = 纯白底;关键字段两盘互异;INK 双盘
    /// 恒黑(diff 遮挡罩语义不随盘反色)
    #[test]
    fn palettes_distinct_and_anchored() {
        let l = &PALETTES[M_LIGHT as usize];
        let d = &PALETTES[M_DARK as usize];
        assert_eq!(d.base, color(0x212734, WINDOW_TINT_A));
        assert_eq!(d.sidebar, color(0x242A2C, 1.0));
        assert_eq!(l.base, color(0xFFFFFF, 1.0));
        assert_ne!(d.label, l.label);
        assert_ne!(d.brand, l.brand);
        assert_ne!(d.code, l.code);
        assert_eq!(d.ink, l.ink);
        assert_eq!(d.ink, color(0x000000, 1.0));
        // 侧栏交互三态互异(hover/选中串色即侧栏语义失效)
        assert_ne!(d.sidebar, d.sidebar_hover);
        assert_ne!(d.sidebar_hover, d.sidebar_active);
        // 标题栏与侧栏分色(用户指定 41,45,48 / 36,42,44)
        assert_ne!(d.title_bar, d.sidebar);
        assert_eq!(d.sidebar, color(0x242A2C, 1.0));
        assert_eq!(d.title_bar, color(0x292D30, 1.0));
        // 标题栏与画布分色
        assert_ne!(d.title_bar, d.base);
    }

    /// 毛玻璃联动决策表:浅盘恒实色;深盘仅 macOS 走纯透明+
    /// 应用侧自建效果视图
    #[test]
    fn window_background_follows_mode() {
        assert_eq!(
            window_background_for(false),
            WindowBackgroundAppearance::Opaque
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            window_background_for(true),
            WindowBackgroundAppearance::Transparent
        );
    }

    /// 档位解析与 registry settings.yaml 词汇一致,未知值回落深色
    #[test]
    fn appearance_parse_matches_registry() {
        assert_eq!(Appearance::parse("light"), Appearance::Light);
        assert_eq!(Appearance::parse("dark"), Appearance::Dark);
        assert_eq!(Appearance::parse("system"), Appearance::System);
        assert_eq!(Appearance::parse("whatever"), Appearance::Dark);
    }

    /// liuma token 铺设:只动本 App 的 Theme global,不触进程级盘静态
    /// (与并发测试无竞争);Light 下组件面吃浅盘值
    #[gpui_kit::test]
    fn apply_tokens_paints_component_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            Theme::change(ThemeMode::Light, None, cx);
            // Theme::change 用库默认亮盘整铺;apply_tokens 后关键 token
            // 必须等于浅盘值(danger/scrollbar_thumb 与库默认不同,相等
            // 即证明铺设发生)
            apply_tokens(ThemeMode::Light, cx);
            let t = Theme::global(cx);
            assert!(!t.is_dark());
            assert_eq!(
                t.colors.background,
                Hsla::from(palette_of(ThemeMode::Light).base)
            );
            assert_eq!(
                t.colors.scrollbar_thumb,
                Hsla::from(palette_of(ThemeMode::Light).dock)
            );
            // 填充面前景:浅盘下仍为纯白(非近黑 label)
            assert_eq!(
                t.colors.primary_foreground,
                Hsla::from(color(0xFFFFFF, 1.0))
            );
            // tokens 语义面镜像(Root 画布读 tokens.background;漏镜像
            // = 画布露出库默认底,2026-09 毛玻璃批次实测踩坑)
            assert_eq!(
                *t.tokens.background,
                Hsla::from(palette_of(ThemeMode::Light).base)
            );
        });
    }

    /// 正文族锁(机制见 [`FONT_SANS`]):必须是显式族、不得回落系统
    /// 字体,且派生面(组件读 `tokens.typography.sans`)同源。
    /// 改回 `.SystemUIFont` 时本用例先于真机「行尾被裁」失败
    #[gpui_kit::test]
    fn body_font_family_is_explicit_and_derived(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            // 走生产同款次序:`Theme::change` 打底 → `apply_tokens` 收口。
            // 刻意不经 `apply`——那会写进程级档位静态量(MODE/LAST_*),
            // 并行用例下徒增互相干扰
            Theme::change(ThemeMode::Dark, None, cx);
            apply_tokens(ThemeMode::Dark, cx);
            let t = Theme::global(cx);
            assert_eq!(t.font_family.as_ref(), FONT_SANS);
            assert_ne!(t.font_family.as_ref(), ".SystemUIFont");
            assert_ne!(t.font_family.as_ref(), ".ZedSans");
            assert_eq!(t.typography_tokens().sans.as_ref(), FONT_SANS);
        });
    }

    /// 真机度量不变式锁(机制见 [`FONT_SANS`]):GPUI 折行按「逐字孤立
    /// 量宽」定价、按「整行 shape」绘制,两者必须同源。`.SystemUIFont`
    /// 下 CoreText 对孤立 CJK 标点做行界压缩(实测 `，` 孤立 7.082 /
    /// 行内 14.082,整行少算 ≈ 一个全角字)→ 折行多塞一字 → 行尾被
    /// markdown 列表项内容盒的 overflow_hidden 硬裁。测试环境是
    /// NoopTextSystem(gpui `platform/test/platform.rs:105`)量不到真
    /// 字体,故本用例直连 CoreText 取真度量(系统框架,零新增依赖)。
    #[cfg(target_os = "macos")]
    #[test]
    fn body_font_metrics_are_context_independent() {
        use core_text_ffi as ct;

        let font = ct::font(FONT_SANS, 14.);
        // 1) 标点:孤立量宽必须等于行内实宽——压缩即本 bug 的定价来源
        let isolated = ct::width("，", font);
        let in_line = ct::width("稳，定", font) - ct::width("稳定", font);
        assert!(
            (isolated - in_line).abs() < 0.01,
            "正文族 {FONT_SANS} 的 CJK 标点孤立量宽 {isolated} != 行内实宽 {in_line}:\
             折行定价将与绘制不一致(换族理由见 kits::theme::FONT_SANS)"
        );
        // 2) 记账不得低于绘制(行尾越界只可能由「记账偏小」引起;反
        // 方向即 kerning 使整行略窄,无害,不作断言)
        let line = "期间顺带发现并保留了设计要点：刻度 id/selector 改 key 基\
                    (nav-point-user:<seq>)——分页后 slot 会漂移，key 稳定，";
        let summed = line
            .chars()
            .map(|c| ct::width(&c.to_string(), font))
            .sum::<f64>();
        let shaped = ct::width(line, font);
        assert!(
            summed - shaped <= 0.01,
            "正文族 {FONT_SANS} 逐字孤立量宽合计 {summed} 低于整行 shape {shaped}\
             (差 {:+.2}pt):折行会多塞字,行尾越界后被 overflow_hidden 裁掉",
            shaped - summed
        );
        // 3) 机制自检:确认本用例真的量到了「行界压缩」——系统字体族必须
        // 违反上述不变式(真名是 .AppleSystemUIFont;占位名 .SystemUIFont
        // CoreText 解析不到,落到兜底族上反而不触发,故不拿它当基准)。
        // 若哪天这里不再失败,说明底层行为变了,上面两条断言的前提失效
        let sys = ct::font(".AppleSystemUIFont", 14.);
        let sys_iso = ct::width("，", sys);
        let sys_in_line = ct::width("稳，定", sys) - ct::width("稳定", sys);
        assert!(
            sys_in_line - sys_iso > 1.,
            "基线自检失效:系统字体族下孤立量宽 {sys_iso} 与行内实宽 {sys_in_line} \
             不再有行界压缩差——机制前提已变,请复核 FONT_SANS 的说明"
        );
    }

    /// CoreText / CoreFoundation 最小 FFI(macOS 系统框架,仅测试用:
    /// 取真字体度量,绕开测试环境的 NoopTextSystem)
    #[cfg(target_os = "macos")]
    mod core_text_ffi {
        use core::ffi::{c_double, c_void};

        type Ref = *const c_void;

        /// 只取地址、不解引用的框架全局量(回调结构体)
        #[repr(C)]
        struct Opaque([u8; 0]);

        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFTypeDictionaryKeyCallBacks: Opaque;
            static kCFTypeDictionaryValueCallBacks: Opaque;
            fn CFStringCreateWithBytes(
                alloc: Ref,
                bytes: *const u8,
                len: isize,
                encoding: u32,
                external: u8,
            ) -> Ref;
            fn CFAttributedStringCreate(alloc: Ref, text: Ref, attrs: Ref) -> Ref;
            fn CFDictionaryCreate(
                alloc: Ref,
                keys: *const Ref,
                values: *const Ref,
                count: isize,
                key_callbacks: *const Opaque,
                value_callbacks: *const Opaque,
            ) -> Ref;
        }

        #[link(name = "CoreText", kind = "framework")]
        unsafe extern "C" {
            static kCTFontAttributeName: Ref;
            fn CTFontCreateWithName(name: Ref, size: c_double, matrix: Ref) -> Ref;
            fn CTLineCreateWithAttributedString(text: Ref) -> Ref;
            fn CTLineGetTypographicBounds(
                line: Ref,
                ascent: *mut c_double,
                descent: *mut c_double,
                leading: *mut c_double,
            ) -> c_double;
        }

        const UTF8: u32 = 0x0800_0100;

        fn cfstring(s: &str) -> Ref {
            // SAFETY: s 是有效 UTF-8 缓冲,长度按字节给;CoreFoundation
            // 拷贝内容,返回对象由进程持有(测试生命周期内不释放)
            unsafe {
                CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0)
            }
        }

        /// 按族名 + 字号解析字体(族名不存在时 CoreText 回兜底字体,
        /// 故本函数不报错——族名的正确性由 `body_font_family_*` 用例守)
        pub(super) fn font(family: &str, size: f64) -> Ref {
            // SAFETY: 名称/字号合法;matrix 传空 = 单位矩阵
            unsafe { CTFontCreateWithName(cfstring(family), size, std::ptr::null()) }
        }

        /// 单行排布宽度(与 gpui `TextSystem::layout_line` 同一底层调用)
        pub(super) fn width(text: &str, font: Ref) -> f64 {
            // SAFETY: 属性字典 = {kCTFontAttributeName: font},键/值回调
            // 取框架全局常量;返回的 CTLine 仅本函数内使用
            unsafe {
                let keys = [kCTFontAttributeName];
                let values = [font];
                let attrs = CFDictionaryCreate(
                    std::ptr::null(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    1,
                    &raw const kCFTypeDictionaryKeyCallBacks,
                    &raw const kCFTypeDictionaryValueCallBacks,
                );
                let line = CTLineCreateWithAttributedString(CFAttributedStringCreate(
                    std::ptr::null(),
                    cfstring(text),
                    attrs,
                ));
                CTLineGetTypographicBounds(
                    line,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            }
        }
    }
}
