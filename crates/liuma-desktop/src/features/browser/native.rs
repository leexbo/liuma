//! 浏览器面板原生层:WKWebView 直挂 gpui 的 GPUIView(仅 macOS)。
//!
//! ## 结构
//! 每 tab 一只 WKWebView,懒创建于正文 mount canvas 的 paint 期(首帧
//! 有 bounds 才建),`addSubview` 到 GPUIView(contentView),同帧
//! `setFrame`(NSRect 脏检查)。导航命令(`NavCommand`)在 paint 期
//! 从 store 取走执行;导航回调经 `NavDelegate`(WKNavigationDelegate,
//! define_class!)折叠成 `NavEvent` 走 smol 通道,转发任务(`cx.spawn`
//! 循环,Task 存 NativeTab,drop 即取消)回写 store。
//!
//! ## z-order 与抑制
//! WKWebView 是原生子视图,恒绘制于 gpui 的 Metal 内容之上——gpui
//! 自绘浮层(popover/dialog/lightbox/拖放蒙层等)都会被它盖住。对策
//! = 抑制谓词 [`is_suppressed`](单点聚合,**新增根级浮层/Dialog/
//! Sheet/通知必须在此登记**)+ 双元素分工:mount canvas 只在「本 tab
//! 激活且无抑制」时显示;根级 [`NativeSync`] 每帧兜底「canvas 不再
//! 被 paint」的一切路径(标签切走/面板收起/设置页接管),把该藏的
//! webview 全部 setHidden。残留接受项:tooltip 等瞬态小浮层偶发被裁。
//!
//! ## 坐标
//! gpui `Bounds<Pixels>` 原点在窗口左上(逻辑点);GPUIView 未覆写
//! isFlipped(原点左下)。换算 `y' = H − y − h`(H = GPUIView bounds
//! 高,每帧现读)。视口预设窄于面板时按预设宽居中收窄。
//!
//! ## 焦点
//! GPUIView 覆写 `performKeyEquivalent:` 且不调 super:webview 持
//! first responder 时 ⌘ 组合仍先到 gpui 键表(⌘T/⇧⌘P 可达)。但
//! gpui 的 focus() 不调 makeFirstResponder——调停按「gpui 区域是否
//! 刚被点击」(mouse-down 标记;点击 webview 时原生层吃掉事件,gpui
//! 收不到)分流:点击 gpui 控件 → FR 归还 GPUIView(键盘进 gpui,
//! 派发到焦点元素);点击 webview → 清 gpui 残留焦点(⌘V 不在页面
//! 里错投)。webview 被隐藏时若持 FR 同样归还。
//!
//! ## 平台边界
//! 本文件是浏览器功能唯一的平台 API 面(`platform` 模块按 OS 切换,
//! 非 macOS 为 no-op 替身);store/views 层零平台代码。win/linux 尚未
//! 适配。

use gpui_kit::{
    App, Bounds, Entity, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Style,
    Window,
};

use crate::shell::panel::BrowserTabId;
use crate::shell::store::AppStore;

#[cfg(target_os = "macos")]
mod platform;
#[cfg(not(target_os = "macos"))]
mod platform_stub;
#[cfg(not(target_os = "macos"))]
use platform_stub as platform;

/// mount canvas 的 paint 体(canvas paint 闭包直调;bounds = 正文
/// 区域)。macOS 外 no-op
pub(super) fn paint_mount(
    id: BrowserTabId,
    store: &Entity<AppStore>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    platform::paint_mount(id, store, bounds, window, cx);
}

/// 移除标签(拆视图/停转发/归还焦点)。macOS 外 no-op
pub(super) fn remove_tab(id: BrowserTabId) {
    platform::remove_tab(id);
}

/// 抑制谓词:任一 gpui 全窗浮层在场即隐藏 webview。**新增根级浮层/
/// Dialog/Sheet/通知必须在此登记**(gpui 无法绘制于原生子视图之上,
/// 漏登记 = webview 盖住浮层)。macOS 外恒 false
pub(super) fn is_suppressed(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) -> bool {
    platform::is_suppressed(store, window, cx)
}

/// 根级同步元素:每帧收敛隐藏 + 焦点调停(见模块注释)。挂
/// WorkspaceView 根(macOS 外 paint 空转,渲染零开销)
pub(crate) struct NativeSync {
    store: Entity<AppStore>,
}

impl NativeSync {
    pub(crate) fn new(store: &Entity<AppStore>) -> Self {
        Self {
            store: store.clone(),
        }
    }
}

impl IntoElement for NativeSync {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui_kit::Element for NativeSync {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui_kit::ElementId> {
        Some("browser-native-sync".into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        platform::paint_root_sync(&self.store, window, cx);
    }
}
