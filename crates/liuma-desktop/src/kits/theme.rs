//! 主题:色板唯一来源 = ThemeRegistry 的 JSON 主题(`assets/themes/`,
//! 内嵌加载,liuma 双盘 = "Liuma Dark/Light")。界面色一律经
//! snake_case 包装 fn 直读 `cx.theme()` 的语义 token;无对应 token
//! 的私有语义(composer/文字层级/发丝线/玻璃态等)由 `*_of` 公式
//! 从基础 token 锚定派生,公式唯一源在包装层。headless 消费者
//! (mermaid 光栅渲染/语法高亮)由调用方持 cx 处取值后穿参
//! ([`palette_of`] / [`is_dark`])。设置页「外观」三档(浅色/深色/
//! 跟随系统)经 [`apply`] 装载;「跟随系统」由窗口外观观察者驱动
//! (shell::store 挂 `observe_window_appearance`)。

use std::rc::Rc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, Ordering};

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, Rgba, WindowAppearance, px, rgba};

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

// ── 派生色板 ─────────────────────────────────────────────────

/// mermaid headless 渲染所需的语义色快照(字段语义见同名包装 fn
/// 文档)。由 [`palette_of`] 从组件库 Theme 派生的纯数据,在持 cx 的
/// 调用点一次取值后随 [`RenderTheme`](crate::kits::mermaid::RenderTheme)
/// 穿线;界面渲染不走此结构,一律用 snake_case 包装 fn 直读
#[derive(Clone, Copy)]
pub struct Palette {
    pub base: Rgba,
    pub layer: Rgba,
    pub card: Rgba,
    pub dock: Rgba,
    pub brand: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warn: Rgba,
    pub label: Rgba,
    pub label_2: Rgba,
    pub label_3: Rgba,
    pub caption: Rgba,
    pub border_2: Rgba,
    pub code: Rgba,
    pub ongoing: Rgba,
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

/// 纯黑双盘锚(轨迹 diff 遮挡罩;不随盘反色——遮挡语义恒为暗)
pub const INK: Rgba = color(0x000000, 1.0);
/// 全透明(非激活行底;双盘同值)
pub const TRANSPARENT: Rgba = color(0x000000, 0.0);

/// 文件类型徽章底色(分类配色:word 蓝 / excel 绿 /
/// ppt 橙 / pdf 红;其余灰阶系。双盘同值——徽章恒为白字彩色方块,
/// 深浅盘上均成立;属图标语义色,非界面分层色)
pub fn file_kind_badge(kind: liuma_attachment::FileKind) -> Rgba {
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
/// office 三色与 [`file_kind_badge`] 同源)
pub fn file_type_tint(class: crate::kits::filetype::FileClass) -> Rgba {
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

// ── 主题装载与派生 ───────────────────────────────────────────

/// 内置主题集(`assets/themes/`,include_str 内嵌,release 免 FsPath;
/// 官方 21 套取自 gpui-kit v0.7.0 themes/,与依赖版本同源)
pub const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("liuma", include_str!("../../assets/themes/liuma.json")),
    (
        "adventure",
        include_str!("../../assets/themes/adventure.json"),
    ),
    ("alduin", include_str!("../../assets/themes/alduin.json")),
    (
        "asciinema",
        include_str!("../../assets/themes/asciinema.json"),
    ),
    ("aurora", include_str!("../../assets/themes/aurora.json")),
    ("ayu", include_str!("../../assets/themes/ayu.json")),
    (
        "catppuccin",
        include_str!("../../assets/themes/catppuccin.json"),
    ),
    (
        "everforest",
        include_str!("../../assets/themes/everforest.json"),
    ),
    (
        "fahrenheit",
        include_str!("../../assets/themes/fahrenheit.json"),
    ),
    ("flexoki", include_str!("../../assets/themes/flexoki.json")),
    ("gruvbox", include_str!("../../assets/themes/gruvbox.json")),
    ("harper", include_str!("../../assets/themes/harper.json")),
    ("hybrid", include_str!("../../assets/themes/hybrid.json")),
    (
        "jellybeans",
        include_str!("../../assets/themes/jellybeans.json"),
    ),
    ("kibble", include_str!("../../assets/themes/kibble.json")),
    (
        "macos-classic",
        include_str!("../../assets/themes/macos-classic.json"),
    ),
    (
        "mellifluous",
        include_str!("../../assets/themes/mellifluous.json"),
    ),
    ("molokai", include_str!("../../assets/themes/molokai.json")),
    (
        "solarized",
        include_str!("../../assets/themes/solarized.json"),
    ),
    (
        "spaceduck",
        include_str!("../../assets/themes/spaceduck.json"),
    ),
    (
        "tokyonight",
        include_str!("../../assets/themes/tokyonight.json"),
    ),
    (
        "twilight",
        include_str!("../../assets/themes/twilight.json"),
    ),
];

/// 装载内置主题进 registry(main.rs 启动与测试 [`init`] 同走此路;
/// 重名 first-wins——liuma 主题永远最先装入)
pub fn load_builtin_themes(cx: &mut App) {
    let reg = ThemeRegistry::global_mut(cx);
    for (_, json) in BUILTIN_THEMES {
        if let Err(err) = reg.load_themes_from_str(json) {
            eprintln!("[theme] 内置主题解析失败: {err}");
        }
    }
}

/// THEME_NAMES 下标基(0 = light / 1 = dark)
const M_LIGHT: u8 = 0;
const M_DARK: u8 = 1;

/// 指定盘的 Liuma 主题 config(registry 未装时回落库默认盘)
/// 两盘各自选中的主题名(0 = light / 1 = dark;空串 = Liuma 默认)。
/// set_theme 写,config_for_mode 读
static THEME_NAMES: RwLock<[String; 2]> = RwLock::new([String::new(), String::new()]);

/// 指定盘选中主题名(空串归一为 Liuma 默认名;设置页高亮与持久化用)
pub fn theme_name(m: ThemeMode) -> String {
    let raw = THEME_NAMES.read().unwrap_or_else(|e| e.into_inner())[if m.is_dark() {
        M_DARK as usize
    } else {
        M_LIGHT as usize
    }]
    .clone();
    if raw.is_empty() {
        default_theme_name(m).to_string()
    } else {
        raw
    }
}

/// Liuma 默认主题名(liuma.json 内嵌两盘)
fn default_theme_name(m: ThemeMode) -> &'static str {
    if m.is_dark() {
        "Liuma Dark"
    } else {
        "Liuma Light"
    }
}

/// 指定盘当前应装的 config(选中主题名缺失时回落 Liuma 默认,再缺
/// 回落库默认盘)
fn config_for_mode(m: ThemeMode, cx: &App) -> Rc<ThemeConfig> {
    let reg = ThemeRegistry::global(cx);
    let name = theme_name(m);
    reg.themes().get(name.as_str()).cloned().unwrap_or_else(|| {
        reg.themes()
            .get(default_theme_name(m))
            .cloned()
            .unwrap_or_else(|| {
                if m.is_dark() {
                    reg.default_dark_theme().clone()
                } else {
                    reg.default_light_theme().clone()
                }
            })
    })
}

/// Theme 级收口:应用主题 config 后重钉 liuma 不妥协项。
/// 必须是 `Theme::update` 闭包体(尾部自动 tokens reconcile +
/// sync_base + refresh_windows;`apply_config` 注册 config 并切
/// mode,edit 判定 installed_by_edit 后不再重铺)
fn apply_theme_config(t: &mut Theme, cfg: &Rc<ThemeConfig>) {
    t.apply_config(cfg);
    // 侧栏选中行实色:`apply_config` 尾部把 list_active 的 α 强制钳到
    // ≤0.2(库的选中态半透明设计),实色选中态是 liuma 的既有观感
    // ——从 config 原值重钉(闭包内直改 colors,`Theme::update` 的
    // edit 尾部会把它 reconcile 进 tokens)
    if let Some(raw) = cfg.colors.list_active.as_deref()
        && let Ok(c) = gpui_kit::component::try_parse_color(raw)
    {
        t.colors.list_active = c;
    }
    // 字体族收口(见 [`FONT_SANS`])。必须在 `apply_config` 之后:
    // config 的 font.family / typography reconcile 会把族覆盖回
    // `.SystemUIFont`(theme/mod.rs:536)
    t.font_family = FONT_SANS.into();
    // 圆角防御:主题未给 radius 时兜底(liuma 主题已烤进 8/12)
    t.radius = px(8.);
    t.radius_lg = px(12.);
}

/// 逐 HSLA 字段线性混合(f = a 的比重;派生场景均为同族中性灰,
/// hue 通道几近相同,线性插值即可)
fn mix(a: Hsla, b: Hsla, f: f32) -> Hsla {
    Hsla {
        h: a.h * f + b.h * (1. - f),
        s: a.s * f + b.s * (1. - f),
        l: a.l * f + b.l * (1. - f),
        a: a.a * f + b.a * (1. - f),
    }
}

/// α 缩放(±超出 1 收口;`Colorize::opacity` 的 factor 会 clamp,
/// 放大场景用不了)
fn scale_alpha(x: Hsla, k: f32) -> Hsla {
    Hsla {
        a: (x.a * k).min(1.),
        ..x
    }
}

/// α 直设(前景压定 α 的玻璃态/扫光族)
fn flat_alpha(x: Hsla, a: f32) -> Hsla {
    Hsla { a, ..x }
}

// ── 派生公式(_of 系 = 私有语义的唯一公式源,derive 快照与
// snake_case 包装层共用,两路永不漂移)────────────────────

/// 输入卡浮出卡面一档:深盘向前景掺 ~3.5%(锚旧 #272729),
/// 浅盘与卡面同源(白画布靠边框+阴影分层)
fn composer_of(t: &Theme) -> Rgba {
    let c = &t.colors;
    (if t.is_dark() {
        mix(c.popover, c.foreground, 0.965)
    } else {
        c.popover
    })
    .into()
}

/// 文字三级:对二级文字按盘定比缩放 α(锚旧深 0.55 / 浅 0.50)
fn label_3_of(t: &Theme) -> Rgba {
    scale_alpha(
        t.colors.muted_foreground,
        if t.is_dark() { 0.764 } else { 0.694 },
    )
    .into()
}

/// 说明文字(锚旧深 0.38 / 浅 0.35——浅盘层级稍收)
fn caption_of(t: &Theme) -> Rgba {
    scale_alpha(
        t.colors.muted_foreground,
        if t.is_dark() { 0.528 } else { 0.486 },
    )
    .into()
}

/// 二级发丝线:弱边框 α 提一档(锚旧 深 0.14 / 浅 0.16)
fn border_2_of(t: &Theme) -> Rgba {
    scale_alpha(t.colors.border, if t.is_dark() { 1.75 } else { 1.6 }).into()
}

/// 代码面:深盘向黑压一阶(锚旧 #101010)/ 浅盘掺前景一档
/// (锚旧 #F7F7F9)
fn code_of(t: &Theme) -> Rgba {
    let c = &t.colors;
    let black = Hsla {
        h: 0.,
        s: 0.,
        l: 0.,
        a: 1.,
    };
    (if t.is_dark() {
        mix(c.background, black, 0.76)
    } else {
        mix(c.background, c.foreground, 0.965)
    })
    .into()
}

/// 玻璃态填充(锚旧深 α0.10 / 浅 α0.06)
fn glass_bg_of(t: &Theme) -> Rgba {
    flat_alpha(t.colors.foreground, if t.is_dark() { 0.10 } else { 0.06 }).into()
}

/// 玻璃态描边(锚旧深 α0.16 / 浅 α0.14)
fn glass_border_of(t: &Theme) -> Rgba {
    flat_alpha(t.colors.foreground, if t.is_dark() { 0.16 } else { 0.14 }).into()
}

/// 工具卡扫光渐变端点(锚旧深 α0.07 / 浅 α0.08)
fn sweep_of(t: &Theme) -> Rgba {
    flat_alpha(t.colors.foreground, if t.is_dark() { 0.07 } else { 0.08 }).into()
}

/// 时间刻度非激活色(双盘同 α0.22)
fn tick_idle_of(t: &Theme) -> Rgba {
    flat_alpha(t.colors.foreground, 0.22).into()
}

/// 直取 token 承担「库组件面 == 自绘面」的同源底座;无对应 token
/// 的私有语义(composer/code/文字层级/发丝线/玻璃态)按锚定系数
/// 从基础 token 混合/缩放——系数锚定 Liuma 双盘旧值,任意主题下
/// 保持同等的层级关系。headless 消费者(mermaid 渲染链/锚定测试)
/// 在持 cx 的调用点一次取值后穿参,不在渲染线程读全局
pub(crate) fn palette_of(t: &Theme) -> Palette {
    let c = &t.colors;
    Palette {
        base: c.background.into(),
        layer: c.muted.into(),
        card: c.popover.into(),
        dock: c.secondary.into(),
        brand: c.primary.into(),
        danger: c.danger.into(),
        success: c.success.into(),
        warn: c.warning.into(),
        label: c.foreground.into(),
        label_2: c.muted_foreground.into(),
        label_3: label_3_of(t),
        caption: caption_of(t),
        border_2: border_2_of(t),
        code: code_of(t),
        ongoing: c.primary.into(),
    }
}

/// 指定盘派生快照(浅盘校验类测试用;生产无调用)。局部构造,
/// 不触全局 Theme——与生产 apply 同路:clone → apply_config → palette_of
#[cfg(test)]
pub(crate) fn palette_for_mode(m: ThemeMode, cx: &App) -> Palette {
    let mut t = Theme::global(cx).clone();
    apply_theme_config(&mut t, &config_for_mode(m, cx));
    palette_of(&t)
}

// ── 语义包装层(直读 `cx.theme()`;调用点迁移目标 API,与快照
// 同公式同值——迁移完成后快照层退役)────────────────────────

/// 主背景(内容区画布,由组件库 Root 层单次铺底)
pub fn base(cx: &App) -> Rgba {
    cx.theme().colors.background.into()
}
/// 侧栏面板底(库 list 面同源)
pub fn sidebar(cx: &App) -> Rgba {
    cx.theme().colors.list.into()
}
/// 侧栏行 hover
pub fn sidebar_hover(cx: &App) -> Rgba {
    cx.theme().colors.list_hover.into()
}
/// 侧栏行激活/选中(实色,见 [`apply_theme_config`] 的重钉)
pub fn sidebar_active(cx: &App) -> Rgba {
    cx.theme().colors.list_active.into()
}
/// 浮层/hover 层底
pub fn layer(cx: &App) -> Rgba {
    cx.theme().colors.muted.into()
}
/// 输入卡/卡片底(库浮层面 popover 同源)
pub fn card(cx: &App) -> Rgba {
    cx.theme().colors.popover.into()
}
/// 输入卡面(仅 composer 输入卡用,工具卡等走 [`card`])
pub fn composer(cx: &App) -> Rgba {
    composer_of(cx.theme())
}
/// 按钮底/次级填充(chip 类)
pub fn dock(cx: &App) -> Rgba {
    cx.theme().colors.secondary.into()
}
/// 品牌色
pub fn brand(cx: &App) -> Rgba {
    cx.theme().colors.primary.into()
}
/// 品牌填充面前景(主题 `primary.foreground`)
pub fn on_brand(cx: &App) -> Rgba {
    cx.theme().colors.primary_foreground.into()
}
/// 危险色
pub fn danger(cx: &App) -> Rgba {
    cx.theme().colors.danger.into()
}
/// 危险填充面前景(主题 `danger.foreground`)
pub fn on_danger(cx: &App) -> Rgba {
    cx.theme().colors.danger_foreground.into()
}
/// 成功色
pub fn success(cx: &App) -> Rgba {
    cx.theme().colors.success.into()
}
/// 警告色
pub fn warning(cx: &App) -> Rgba {
    cx.theme().colors.warning.into()
}
/// 用户气泡底
pub fn bubble(cx: &App) -> Rgba {
    cx.theme().colors.list_hover.into()
}
/// 文字一级
pub fn label(cx: &App) -> Rgba {
    cx.theme().colors.foreground.into()
}
/// 文字二级
pub fn label_2(cx: &App) -> Rgba {
    cx.theme().colors.muted_foreground.into()
}
/// 文字三级
pub fn label_3(cx: &App) -> Rgba {
    label_3_of(cx.theme())
}
/// 说明文字
pub fn caption(cx: &App) -> Rgba {
    caption_of(cx.theme())
}
/// 弱边框
pub fn border(cx: &App) -> Rgba {
    cx.theme().colors.border.into()
}
/// 二级发丝线
pub fn border_2(cx: &App) -> Rgba {
    border_2_of(cx.theme())
}
/// 代码块/终端卡表面
pub fn code(cx: &App) -> Rgba {
    code_of(cx.theme())
}
/// 运行进行色(随品牌色)
pub fn ongoing(cx: &App) -> Rgba {
    cx.theme().colors.primary.into()
}
/// 玻璃态填充(激活 tab pill)
pub fn glass_bg(cx: &App) -> Rgba {
    glass_bg_of(cx.theme())
}
/// 玻璃态描边
pub fn glass_border(cx: &App) -> Rgba {
    glass_border_of(cx.theme())
}
/// 工具卡扫光渐变端点
pub fn sweep(cx: &App) -> Rgba {
    sweep_of(cx.theme())
}
/// 时间刻度非激活色
pub fn tick_idle(cx: &App) -> Rgba {
    tick_idle_of(cx.theme())
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

/// 当前生效盘是否深色(自绘语法高亮/轨迹配色的分发开关;直读
/// 全局 Theme,无快照)
pub fn is_dark(cx: &App) -> bool {
    Theme::global(cx).is_dark()
}

/// 应用外观档(启动装配与设置页切换同一入口)。
///
/// 流程:System 档先解除 NSApp 强制外观(带着强制值读到的是被强制的
/// 外观,不是真实系统外观)→ `cx.window_appearance()` 解析生效盘 →
/// (档位,生效盘)均未变即幂等返回 → [`Theme::update`] 闭包内
/// [`apply_theme_config`](apply_config 生效盘 + 字体族/圆角收口,
/// edit 尾部自动 tokens reconcile + sync_base + 全窗刷新)。显式
/// 浅/深档同时强制 NSApp 外观,原生
/// 交通灯/边框跟主题,避免「应用深色 + 系统浅色」的原生 chrome 撕裂。
pub fn apply(choice: Appearance, cx: &mut App) {
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
    let cfg = config_for_mode(m, cx);
    Theme::update(cx, |t| apply_theme_config(t, &cfg));
}

/// 切换指定盘的主题(设置页主题行;registry 未装该名 → 不生效返回
/// false)。写入主题名;若切的是当前盘,立即重装 config(另一盘只在
/// 下次翻盘时装载)。System 档翻盘走 [`apply`],自动用对应盘新主题
pub fn set_theme(m: ThemeMode, name: &str, cx: &mut App) -> bool {
    if !ThemeRegistry::global(cx).themes().contains_key(name) {
        return false;
    }
    let idx = if m.is_dark() {
        M_DARK as usize
    } else {
        M_LIGHT as usize
    };
    THEME_NAMES.write().unwrap_or_else(|e| e.into_inner())[idx] = name.to_string();
    if m.is_dark() == Theme::global(cx).is_dark() {
        let cfg = config_for_mode(m, cx);
        Theme::update(cx, |t| apply_theme_config(t, &cfg));
    }
    true
}

/// 测试装配:固定深色盘(UI 测试同源基线;生产入口走 main.rs 直读
/// settings.yaml 档位的 apply)。**不走 [`apply`]**:幂等守卫是进程级
/// 静态,并行用例互相把守卫写成已应用态、短路掉对方全新 App 的首次
/// 铺设(Theme 是 per-App 的);此处直接做与 apply 同款的
/// `Theme::update` 闭包,不受竞争影响
#[cfg(test)]
pub fn init(cx: &mut App) {
    load_builtin_themes(cx);
    let cfg = config_for_mode(ThemeMode::Dark, cx);
    Theme::update(cx, |t| apply_theme_config(t, &cfg));
    CHOICE.store(Appearance::Dark as u8, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    /// 通道级近似断言(hex ↔ HSLA 往返有 ≤2/255 的舍入,精确等值
    /// 断言会把合法往返判死)
    fn assert_close(actual: Rgba, expected: Rgba, what: &str) {
        let d = |a: f32, b: f32| (a - b).abs();
        assert!(
            d(actual.r, expected.r) <= 2. / 255.
                && d(actual.g, expected.g) <= 2. / 255.
                && d(actual.b, expected.b) <= 2. / 255.
                && d(actual.a, expected.a) <= 2. / 255.,
            "{what}: {actual:?} != {expected:?}"
        );
    }

    /// 测试装配:组件库 init + 内置主题进 registry(生产走 main.rs
    /// 的 gpui_kit::init + load_builtin_themes,同一份数据)
    fn setup(cx: &mut App) {
        gpui_kit::component::init(cx);
        load_builtin_themes(cx);
    }

    /// 双盘锚定:Liuma 主题下包装层取值 == 旧内联色板(视觉零回归的
    /// 机制保证)。深盘 = 纯中性灰家族(base #151515 / sidebar 与
    /// 标题栏 #1A1A1A / composer ≈#272729 / code #101010),浅盘 =
    /// 纯白底;关键字段两盘互异。逐盘 apply 后断言——包装层就是
    /// 生产取值路径,断言它比断言派生公式更贴近真机
    #[gpui_kit::test]
    fn wrappers_distinct_and_anchored(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let apply = |m: ThemeMode, cx: &mut App| {
                setup(cx);
                let cfg = config_for_mode(m, cx);
                Theme::update(cx, |t| apply_theme_config(t, &cfg));
            };
            apply(ThemeMode::Dark, cx);
            assert_close(base(cx), rgba(0x151515FF), "深盘 base");
            assert_close(sidebar(cx), rgba(0x1A1A1AFF), "深盘 sidebar");
            assert_close(sidebar_hover(cx), rgba(0x232323FF), "深盘 sidebar_hover");
            assert_close(sidebar_active(cx), rgba(0x2A2A2AFF), "深盘 sidebar_active");
            // 输入卡浮出画布一档(锚旧 #272729,派生 #282828)
            assert_close(composer(cx), rgba(0x282828FF), "深盘 composer");
            assert_ne!(composer(cx), base(cx));
            assert_ne!(composer(cx), card(cx));
            assert_close(card(cx), rgba(0x202020FF), "深盘 card");
            assert_close(dock(cx), rgba(0x2A2A2AFF), "深盘 dock");
            assert_close(bubble(cx), rgba(0x232323FF), "深盘 bubble");
            assert_close(code(cx), rgba(0x101010FF), "深盘 code");
            assert_close(brand(cx), rgba(0x0A84FFFF), "深盘 brand");
            assert_close(label(cx), rgba(0xF9FAFBFF), "深盘 label");
            assert_close(label_2(cx), rgba(0xEBEBF5B8), "深盘 label_2");
            assert_close(label_3(cx), rgba(0xEBEBF58C), "深盘 label_3(α0.55)");
            assert_close(caption(cx), rgba(0xEBEBF561), "深盘 caption(α0.38)");
            assert_close(border(cx), rgba(0xFFFFFF14), "深盘 border(α0.08)");
            assert_close(border_2(cx), rgba(0xFFFFFF24), "深盘 border_2(α0.14)");
            // 侧栏交互三态互异(hover/选中串色即侧栏语义失效)
            assert_ne!(sidebar(cx), sidebar_hover(cx));
            assert_ne!(sidebar_hover(cx), sidebar_active(cx));
            let (d_label, d_brand, d_code) = (label(cx), brand(cx), code(cx));
            apply(ThemeMode::Light, cx);
            assert_close(base(cx), rgba(0xFFFFFFFF), "浅盘 base");
            assert_close(composer(cx), card(cx), "浅盘 composer 与 card 同源");
            assert_close(code(cx), rgba(0xF7F7F7FF), "浅盘 code(锚旧 #F7F7F9)");
            assert_ne!(label(cx), d_label);
            assert_ne!(brand(cx), d_brand);
            assert_ne!(code(cx), d_code);
        });
    }

    /// 档位解析与 registry settings.yaml 词汇一致,未知值回落深色
    #[test]
    fn appearance_parse_matches_registry() {
        assert_eq!(Appearance::parse("light"), Appearance::Light);
        assert_eq!(Appearance::parse("dark"), Appearance::Dark);
        assert_eq!(Appearance::parse("system"), Appearance::System);
        assert_eq!(Appearance::parse("whatever"), Appearance::Dark);
    }

    /// 主题切换:registry 未装的名 → false 不生效且不污染选中名;
    /// 已知名 → theme_name 归一可读;空串选中名归一为 Liuma 默认
    #[gpui_kit::test]
    fn set_theme_validates_against_registry(cx: &mut TestAppContext) {
        cx.update(|cx| {
            setup(cx);
            assert_eq!(
                theme_name(ThemeMode::Dark),
                "Liuma Dark",
                "未设置时空串归一为 Liuma 默认名"
            );
            assert!(
                !set_theme(ThemeMode::Dark, "No Such Theme", cx),
                "未知名应拒绝"
            );
            assert_eq!(
                theme_name(ThemeMode::Dark),
                "Liuma Dark",
                "拒绝后选中名不被污染"
            );
            assert!(set_theme(ThemeMode::Dark, "Catppuccin Mocha", cx));
            assert_eq!(theme_name(ThemeMode::Dark), "Catppuccin Mocha");
            // 生效盘 token 跟随新主题(Mocha 底 ≠ Liuma #151515)
            let t = Theme::global(cx);
            let canvas = t.colors.background;
            assert_ne!(canvas, Hsla::from(rgba(0x151515FF)));
            // 还原进程级静态(THEME_NAMES 是跨用例共享的,并行用例经
            // config_for_mode/theme_name 会读到)
            THEME_NAMES.write().unwrap_or_else(|e| e.into_inner())[1] = String::new();
        });
    }

    /// 主题铺设:apply_config 后关键 token == liuma.json 锚值,
    /// tokens 语义面自动 reconcile(`Theme::update` 的 edit 尾部;
    /// 刻意不经 [`apply`]——那会写进程级档位静态量,并行用例下徒增
    /// 互相干扰)
    #[gpui_kit::test]
    fn theme_config_paints_component_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            setup(cx);
            let cfg = config_for_mode(ThemeMode::Light, &*cx);
            Theme::update(cx, |t| apply_theme_config(t, &cfg));
            let t = Theme::global(cx);
            assert!(!t.is_dark());
            assert_close(t.colors.background.into(), rgba(0xFFFFFFFF), "background");
            assert_close(
                t.colors.scrollbar_thumb.into(),
                rgba(0xE9E9EBFF),
                "scrollbar_thumb",
            );
            // 填充面前景:浅盘下仍为纯白(非近黑 label)
            assert_close(
                t.colors.primary_foreground.into(),
                rgba(0xFFFFFFFF),
                "primary_foreground",
            );
            // tokens 语义面镜像(Root 画布读 tokens.background)
            assert_close(
                (*t.tokens.background).into(),
                rgba(0xFFFFFFFF),
                "tokens.background",
            );
        });
    }

    /// 选择控件的**选中态**必须与画布可分:库的 `Radio` 选中点取
    /// `theme.primary`(liuma = 品牌色),未选中描边取 `theme.input`。
    /// 品牌色一旦与画布同色,决策区的场景行就读不出选了哪个。
    /// 回归锚:决策区场景行用 Radio 而非分段控件,正是因为它走 primary
    /// 而不是画布色——分段控件的选中药丸被库硬编码成 `tokens.background`,
    /// 在本主题下与页面同色(深盘实测差 2/255,整条控件看不见)。
    #[gpui_kit::test]
    fn selected_marker_is_distinguishable_from_canvas(cx: &mut TestAppContext) {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update(|cx| {
                setup(cx);
                let cfg = config_for_mode(mode, &*cx);
                Theme::update(cx, |t| apply_theme_config(t, &cfg));
                let t = Theme::global(cx);
                let canvas = *t.tokens.background;
                assert!(
                    (t.colors.primary.l - canvas.l).abs() >= 0.2,
                    "{mode:?}:品牌色与画布亮度差过小({} vs {}),选中点会看不出来",
                    t.colors.primary.l,
                    canvas.l
                );
            });
        }
    }

    /// 正文族锁(机制见 [`FONT_SANS`]):必须是显式族、不得回落系统
    /// 字体,且派生面(组件读 `tokens.typography.sans`)同源。
    /// 改回 `.SystemUIFont` 时本用例先于真机「行尾被裁」失败
    #[gpui_kit::test]
    fn body_font_family_is_explicit_and_derived(cx: &mut TestAppContext) {
        cx.update(|cx| {
            setup(cx);
            let cfg = config_for_mode(ThemeMode::Dark, &*cx);
            Theme::update(cx, |t| apply_theme_config(t, &cfg));
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
            "基线自检失效:系统字体族下孤立量宽 {sys_iso} 与行内实宽 {sys_in_line}\
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
