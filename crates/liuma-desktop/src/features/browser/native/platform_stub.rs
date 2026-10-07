//! 非 macOS 替身:无原生 webview,命令在渲染期取走丢弃(维持状态机
//! 终态,避免无限加载),其余全 no-op。win/linux 尚未适配。

use gpui_kit::{App, Bounds, Entity, Pixels, Window};

use crate::shell::panel::BrowserTabId;
use crate::shell::store::AppStore;

pub(super) fn paint_mount(
    _id: BrowserTabId,
    store: &Entity<AppStore>,
    _bounds: Bounds<Pixels>,
    _window: &mut Window,
    cx: &mut App,
) {
    // 无原生层:命令落位(丢弃 + 结束加载态),占位视图继续呈现
    store.update(cx, |st, cx| st.browser_drop_pending(_id, cx));
}

pub(super) fn remove_tab(_id: BrowserTabId) {}

pub(super) fn is_suppressed(
    _store: &Entity<AppStore>,
    _window: &mut Window,
    _cx: &mut App,
) -> bool {
    false
}

pub(super) fn paint_root_sync(_store: &Entity<AppStore>, _window: &mut Window, _cx: &mut App) {}
