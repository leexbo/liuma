//! macOS 平台实现:WKWebView 生命周期/frame 同步/命令派发/导航回传/
//! 抑制与焦点调停。线程模型:gpui paint 与 store 方法全在主线程,
//! REGISTRY 为 thread-local;WKWebView/NSView 是 MainThreadOnly,
//! 经 `MainThreadMarker::new()` 门控(失败即静默返回)。

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSResponder, NSView};
use objc2_foundation::{
    NSError, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest,
};
use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView};
use raw_window_handle::{AppKitWindowHandle, HasWindowHandle, RawWindowHandle};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use gpui_kit::{App, Bounds, Entity, Pixels, Window};

use crate::shell::panel::BrowserTabId;
use crate::shell::store::AppStore;

use crate::features::browser::store::{NavCommand, NavEvent};

/// 一路原生标签(webview + delegate + frame 基线 + 事件转发任务)
struct NativeTab {
    web_view: Retained<WKWebView>,
    /// delegate 保活(WKWebView 对 navigationDelegate 是弱引用,
    /// 丢弃即悬空 → 回调全静默)
    #[allow(dead_code)]
    delegate: Retained<NavDelegate>,
    /// 上次 setFrame(脏检查;None = 尚未摆位)
    last_frame: Option<NSRect>,
    /// 转发任务保活(存住才在跑;drop = 取消)
    #[allow(dead_code)]
    forwarder: gpui_kit::Task<()>,
}

thread_local! {
    /// 原生标签表(键 = BrowserTabId.0;主线程独占)
    static REGISTRY: RefCell<HashMap<u64, NativeTab>> = RefCell::new(HashMap::new());
}

/// delegate 回调消息(tab id 随消息走,delegate 不持 store 引用)
struct NavMsg {
    tab: u64,
    event: NavEvent,
}

/// NavDelegate 的 ivars(事件通道 + tab id;OnceCell 挂载于创建侧)
struct NavDelegateIvars {
    tx: OnceCell<UnboundedSender<NavMsg>>,
    tab: OnceCell<u64>,
}

define_class!(
    // SAFETY: NSObject 无子类化要求;MainThreadOnly 成立(WKNavigation
    // delegate 回调恒在主线程);NavDelegate 不实现 Drop。
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavDelegateIvars]
    struct NavDelegate;

    unsafe impl NSObjectProtocol for NavDelegate {}

    unsafe impl WKNavigationDelegate for NavDelegate {
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didStartProvisionalNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            self.emit(web_view, NavEventKind::Started);
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didCommitNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            self.emit(web_view, NavEventKind::Committed);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFinishNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            self.emit(web_view, NavEventKind::Finished);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFailNavigation_withError(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.emit_failure(web_view, error);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFailProvisionalNavigation_withError(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            error: &NSError,
        ) {
            // dev server 未起时的连接失败走这里(provisional 阶段)
            self.emit_failure(web_view, error);
        }
    }
);

/// 导航事件种类(emit 时补 URL/标题/能力快照)
enum NavEventKind {
    Started,
    Committed,
    Finished,
}

impl NavDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let this = this.set_ivars(NavDelegateIvars {
            tx: OnceCell::new(),
            tab: OnceCell::new(),
        });
        // SAFETY: NSObject init 是根初始化器,不为空不失败
        unsafe { msg_send![super(this), init] }
    }

    /// 挂载通道与 tab id(创建后立即调用一次)
    fn bind(&self, tx: UnboundedSender<NavMsg>, tab: u64) {
        let ivars = self.ivars();
        let _ = ivars.tx.set(tx);
        let _ = ivars.tab.set(tab);
    }

    fn emit(&self, web_view: &WKWebView, kind: NavEventKind) {
        let ivars = self.ivars();
        let Some(tx) = ivars.tx.get() else { return };
        let Some(tab) = ivars.tab.get().copied() else {
            return;
        };
        // SAFETY: 主线程读 webview 状态
        let url = unsafe { web_view.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let event = match kind {
            NavEventKind::Started => NavEvent::Started { url },
            NavEventKind::Committed => NavEvent::Committed { url },
            NavEventKind::Finished => NavEvent::Finished {
                url,
                title: unsafe { web_view.title() }.map(|t| t.to_string()),
            },
        };
        let _ = tx.send(NavMsg { tab, event });
        let _ = tx.send(NavMsg {
            tab,
            event: NavEvent::Capability {
                can_back: unsafe { web_view.canGoBack() },
                can_fwd: unsafe { web_view.canGoForward() },
            },
        });
    }

    fn emit_failure(&self, web_view: &WKWebView, error: &NSError) {
        let ivars = self.ivars();
        let Some(tx) = ivars.tx.get() else { return };
        let Some(tab) = ivars.tab.get().copied() else {
            return;
        };
        // SAFETY: 主线程读 webview 状态
        let url = unsafe { web_view.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let message = error.localizedDescription().to_string();
        let _ = tx.send(NavMsg {
            tab,
            event: NavEvent::Failed { url, message },
        });
        let _ = tx.send(NavMsg {
            tab,
            event: NavEvent::Capability {
                can_back: unsafe { web_view.canGoBack() },
                can_fwd: unsafe { web_view.canGoForward() },
            },
        });
    }
}

/// GPUIView 强引用(raw window handle 的 ns_view → Retained;每帧
/// retain/release 一次,开销一次 objc 消息)。注意 gpui Window 自有
/// 同名固有方法 window_handle() -> AnyWindowHandle,必须全限定调
/// trait 方法
fn content_view(window: &Window) -> Option<Retained<NSView>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match RawWindowHandle::from(handle) {
        RawWindowHandle::AppKit(AppKitWindowHandle { ns_view, .. }) => {
            // SAFETY: ns_view 指向 gpui 窗口的 contentView(GPUIView),
            // 窗口存活期间有效;retain 增计数,Retained drop 时释放
            unsafe { Retained::retain(ns_view.as_ptr().cast()) }
        }
        _ => None,
    }
}

/// mount canvas 的 paint 体(懒建 + frame 同步 + 命令派发 + 显示)
pub(super) fn paint_mount(
    id: BrowserTabId,
    store: &Entity<AppStore>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(content) = content_view(window) else {
        return;
    };
    // 懒建:webview + delegate + 转发任务,首帧即挂(先藏)
    if !REGISTRY.with(|r| r.borrow().contains_key(&id.0)) {
        let (tx, rx) = unbounded_channel::<NavMsg>();
        let delegate = NavDelegate::new(mtm);
        delegate.bind(tx, id.0);
        // SAFETY: WKWebView initWithFrame 在主线程;ZERO 起步,首帧
        // setFrame 立即摆位
        let web_view = unsafe { WKWebView::initWithFrame(WKWebView::alloc(mtm), NSRect::ZERO) };
        unsafe { web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        web_view.setHidden(true);
        content.addSubview(&web_view);
        // 导航事件转发:通道 → store 折叠(Task 存 NativeTab,标签
        // 移除即 drop 取消)
        let forward_store = store.clone();
        let mut rx = rx;
        let forwarder = cx.spawn(async move |acx: &mut gpui_kit::AsyncApp| {
            while let Some(msg) = rx.recv().await {
                forward_store.update(acx, |st, cx| {
                    st.browser_on_nav_event(BrowserTabId(msg.tab), msg.event, cx);
                });
            }
        });
        REGISTRY.with(|r| {
            r.borrow_mut().insert(
                id.0,
                NativeTab {
                    web_view,
                    delegate,
                    last_frame: None,
                    forwarder,
                },
            );
        });
    }
    // 命令派发(先读后取,避免双借用 store)
    let pending = store
        .read(cx)
        .browser
        .tab(id)
        .and_then(|t| t.pending.clone());
    if let Some(cmd) = pending {
        store.update(cx, |st, _| st.browser_take_pending(id));
        REGISTRY.with(|r| {
            if let Some(tab) = r.borrow().get(&id.0) {
                dispatch(&tab.web_view, &cmd);
            }
        });
    }
    // frame 同步(脏检查)+ 视口收窄居中 + 显示门控
    let (viewport, is_active) = {
        let st = store.read(cx);
        (
            st.browser.tab(id).and_then(|t| t.viewport.width()),
            st.panel_active_tab == Some(crate::shell::panel::PanelTab::Browser(id)),
        )
    };
    let visible = is_active && !super::is_suppressed(store, window, cx);
    let host_h = content.bounds().size.height;
    let full_w = f64::from(f32::from(bounds.size.width));
    let w = viewport
        .map(f64::from)
        .unwrap_or(full_w)
        .min(full_w)
        .max(1.);
    let x = f64::from(f32::from(bounds.origin.x)) + (full_w - w) / 2.;
    let y =
        host_h - f64::from(f32::from(bounds.origin.y)) - f64::from(f32::from(bounds.size.height));
    let frame = NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(w, f64::from(f32::from(bounds.size.height)).max(1.)),
    );
    REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let Some(tab) = reg.get_mut(&id.0) else {
            return;
        };
        if tab.last_frame != Some(frame) {
            tab.web_view.setFrame(frame);
            tab.last_frame = Some(frame);
        }
        tab.web_view.setHidden(!visible);
        if !visible {
            resign_if_focused(&tab.web_view);
        }
    });
}

/// 执行导航命令。reload/goBack/goForward 返回的新 WKNavigation 只是
/// 意图句柄,进度与结果由 delegate 回调覆盖,丢弃
fn dispatch(web_view: &WKWebView, cmd: &NavCommand) {
    match cmd {
        NavCommand::Load(url) => {
            let target = NSURL::URLWithString(&NSString::from_str(url));
            if let Some(target) = target {
                // SAFETY: 主线程直调
                unsafe { web_view.loadRequest(&NSURLRequest::requestWithURL(&target)) };
            }
        }
        NavCommand::Reload => {
            unsafe { web_view.reload() };
        }
        NavCommand::Stop => {
            // SAFETY: 主线程直调
            unsafe { web_view.stopLoading() };
        }
        NavCommand::Back => {
            unsafe { web_view.goBack() };
        }
        NavCommand::Forward => {
            unsafe { web_view.goForward() };
        }
    }
}

/// webview 持 first responder 时归还其 superview(GPUIView;隐藏后
/// 键盘不能挂在不可见视图上)
fn resign_if_focused(web_view: &WKWebView) {
    let Some(window) = web_view.window() else {
        return;
    };
    let Some(responder) = window.firstResponder() else {
        return;
    };
    if !responder.is_webview_or_descendant(web_view) {
        return;
    }
    // SAFETY: 主线程读视图层级
    let Some(superview) = (unsafe { web_view.superview() }) else {
        return;
    };
    let responder: &NSResponder = &superview;
    window.makeFirstResponder(Some(responder));
}

/// first responder 是否落在 webview 内(自身或子孙)
trait ResponderExt {
    fn is_webview_or_descendant(&self, web_view: &WKWebView) -> bool;
}

impl ResponderExt for Retained<NSResponder> {
    fn is_webview_or_descendant(&self, web_view: &WKWebView) -> bool {
        // FR 可能是 webview 内部文本引擎等非视图对象,downcast 到
        // NSView 失败 = 不在任一视图上,直接判否
        let Ok(view) = self.clone().downcast::<NSView>() else {
            return false;
        };
        let host: &NSView = web_view;
        // downcast 已验证类层次;isDescendantOf 含自身
        view.isDescendantOf(host)
    }
}

/// 任意 webview 持 first responder(webview 自带 window 引用,无需
/// 外部传窗口)
fn first_responder_in_any_webview() -> bool {
    REGISTRY.with(|r| {
        r.borrow().values().any(|tab| {
            tab.web_view
                .window()
                .and_then(|w| w.firstResponder())
                .is_some_and(|responder| responder.is_webview_or_descendant(&tab.web_view))
        })
    })
}

/// 根级兜底:每帧把「不该可见」的 webview 全部隐藏 + 焦点调停
pub(super) fn paint_root_sync(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) {
    let active = match store.read(cx).panel_active_tab {
        Some(crate::shell::panel::PanelTab::Browser(id)) => Some(id.0),
        _ => None,
    };
    let hidden_all = active.is_none() || super::is_suppressed(store, window, cx);
    REGISTRY.with(|r| {
        let keys: Vec<u64> = r.borrow().keys().copied().collect();
        for key in keys {
            let visible = !hidden_all && active == Some(key);
            if visible {
                continue;
            }
            let Some(web_view) = r.borrow().get(&key).map(|t| t.web_view.clone()) else {
                continue;
            };
            web_view.setHidden(true);
            resign_if_focused(&web_view);
        }
    });
    // 焦点调停:webview 持 FR 且 gpui 仍有焦点句柄 = 点击 webview 后的
    // gpui 残留(composer 输入态/光标)→ 清掉,⌘V 等不再错投 gpui
    if window.focused(cx).is_some() && first_responder_in_any_webview() {
        window.blur(cx);
    }
}

/// 抑制谓词(登记面见 native.rs 模块注释)
pub(super) fn is_suppressed(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) -> bool {
    use gpui_kit::component::WindowExt as _;
    if window.has_active_dialog(cx) || window.has_active_sheet(cx) {
        return true;
    }
    if cx.has_active_drag() {
        return true;
    }
    if !window.notifications(cx).is_empty() {
        return true;
    }
    let st = store.read(cx);
    st.sessions.ws_info_card.is_some()
        || st.settings.needs_onboarding
        || st.attachments.lightbox.is_some()
        || st.chat.mermaid_viewer.is_some()
        || st.attachments.attachment_toast.is_some()
        || st.opener.launch_error.is_some()
}

/// 移除标签:归还焦点 → 拆视图 → 停转发(REGISTRY 移除即 drop Task)
pub(super) fn remove_tab(id: BrowserTabId) {
    if let Some(tab) = REGISTRY.with(|r| r.borrow_mut().remove(&id.0)) {
        resign_if_focused(&tab.web_view);
        tab.web_view.removeFromSuperview();
    }
}
