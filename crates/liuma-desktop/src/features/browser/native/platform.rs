//! macOS 平台实现:WKWebView 生命周期/frame 同步/命令派发/导航回传/
//! 抑制与焦点调停。线程模型:gpui paint 与 store 方法全在主线程,
//! REGISTRY 为 thread-local;WKWebView/NSView 是 MainThreadOnly,
//! 经 `MainThreadMarker::new()` 门控(失败即静默返回)。

use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, VecDeque};

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
    /// 当前 UA 是否移动档(None = 未设置;脏检查基线)
    ua_mobile: Option<bool>,
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

/// 重定向循环熔断阈值:窗口期内 Started 次数(站点脚本在 http/https
/// 间主动降级 + WKWebView 的 HTTPS 升级顶回 = 协议乒乓,页面级跳转
/// 死循环;全新 cookie 存储的独立 webview 最易触发)
const NAV_BURST_LIMIT: usize = 8;
/// 计数窗口与熔断冷却(同一时间基)
const NAV_BURST_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);
const NAV_TRIP_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(10);

/// NavDelegate 的 ivars(事件通道 + tab id;OnceCell 挂载于创建侧)
struct NavDelegateIvars {
    tx: OnceCell<UnboundedSender<NavMsg>>,
    tab: OnceCell<u64>,
    /// 最近导航意图时刻(Started 计数窗口)
    nav_burst: RefCell<VecDeque<std::time::Instant>>,
    /// 熔断时刻(None = 未熔断;冷却后自动复位,用户重新导航即复位)
    tripped_at: RefCell<Option<std::time::Instant>>,
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
            if self.check_nav_burst(web_view) {
                return;
            }
            self.emit(web_view, NavEventKind::Started);
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didCommitNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            if self.is_tripped() {
                return;
            }
            self.emit(web_view, NavEventKind::Committed);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        #[allow(non_snake_case)]
        unsafe fn webView_didFinishNavigation(
            &self,
            web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            if self.is_tripped() {
                return;
            }
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
            if self.is_tripped() {
                return;
            }
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
            if self.is_tripped() {
                return;
            }
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
            nav_burst: RefCell::new(VecDeque::new()),
            tripped_at: RefCell::new(None),
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

    /// 重定向循环熔断:窗口期内导航意图过密 → 掐死后续导航并保持
    /// 当前页面 + 报错;冷却后自动复位,用户主动导航(Load)即时复位。
    /// 返回 true = 已熔断(调用方吞掉本事件)
    fn check_nav_burst(&self, web_view: &WKWebView) -> bool {
        let ivars = self.ivars();
        let now = std::time::Instant::now();
        // 先复制再判定:let-chain 的 scrutinee 临时(Ref)存活到块尾,
        // 块内 borrow_mut 会撞车
        let tripped = *ivars.tripped_at.borrow();
        if let Some(t) = tripped
            && now.duration_since(t) > NAV_TRIP_COOLDOWN
        {
            *ivars.tripped_at.borrow_mut() = None;
            ivars.nav_burst.borrow_mut().clear();
        }
        if ivars.tripped_at.borrow().is_some() {
            // SAFETY: 主线程直调;页面保持已加载内容,跳转意图即起即灭
            unsafe { web_view.stopLoading() };
            return true;
        }
        {
            let mut burst = ivars.nav_burst.borrow_mut();
            burst.push_back(now);
            while let Some(&oldest) = burst.front()
                && now.duration_since(oldest) > NAV_BURST_WINDOW
            {
                burst.pop_front();
            }
            if burst.len() < NAV_BURST_LIMIT {
                return false;
            }
            *ivars.tripped_at.borrow_mut() = Some(now);
        }
        // 掐死循环中的 provisional 导航;页面保持在最后一次成功加载的
        // 内容(循环里每轮页面其实都加载成功,毁掉反而粗暴),后续跳转
        // 由熔断激活分支逐个 stop
        // SAFETY: 主线程直调
        unsafe { web_view.stopLoading() };
        let _ = self.send_notice(crate::kits::i18n::t!("browser.err_redirect_loop").to_string());
        true
    }

    /// 熔断生效中(断页期间的残余事件全吞;didStart 里由
    /// check_nav_burst 顺带做冷却复位)
    fn is_tripped(&self) -> bool {
        let ivars = self.ivars();
        let tripped = *ivars.tripped_at.borrow();
        match tripped {
            Some(t) => t.elapsed() <= NAV_TRIP_COOLDOWN,
            None => false,
        }
    }

    /// 用户主动导航(Load):复位熔断与计数
    fn reset_cycle_guard(&self) {
        let ivars = self.ivars();
        *ivars.tripped_at.borrow_mut() = None;
        ivars.nav_burst.borrow_mut().clear();
    }

    /// 发提示(不附带能力快照;熔断路径用)
    fn send_notice(&self, message: String) -> Option<()> {
        let ivars = self.ivars();
        let tx = ivars.tx.get()?;
        let tab = ivars.tab.get().copied()?;
        let _ = tx.send(NavMsg {
            tab,
            event: NavEvent::Notice { message },
        });
        Some(())
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
        // NSURLErrorCancelled(-999):stopLoading/新导航打断旧导航的正常
        // 取消,不是失败——静默
        if error.code() == -999 {
            return;
        }
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
                    ua_mobile: None,
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
                // 用户主动导航(Load/Reload/Back/Forward)一律复位熔断
                // (Stop 无导航意图,不复位)
                if !matches!(cmd, NavCommand::Stop) {
                    tab.delegate.reset_cycle_guard();
                }
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
        // UA 跟随视口档(移动档 = 移动 Safari UA,桌面档 = 系统默认;
        // 脏检查,变更只影响后续请求,不触发重载)
        let mobile_ua = viewport.is_some();
        if tab.ua_mobile != Some(mobile_ua) {
            // SAFETY: 主线程直调属性 setter
            unsafe {
                if mobile_ua {
                    let ua = NSString::from_str(MOBILE_UA_NS);
                    tab.web_view.setCustomUserAgent(Some(&ua));
                } else {
                    tab.web_view.setCustomUserAgent(None);
                }
            }
            tab.ua_mobile = Some(mobile_ua);
        }
        tab.web_view.setHidden(!visible);
        if !visible {
            resign_if_focused(&tab.web_view);
        }
    });
}

/// 移动视口档的 UA(iOS Safari;dev server 的响应式断点按它命中)
const MOBILE_UA_NS: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

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

// gpui 区域刚被点击的帧标记(mouse-down bubble 相置位,调停消费)。
// 点击 webview 时原生子视图吃掉事件,gpui 收不到 mouse-down——标记
// 缺席即「用户点的是 webview」的判据
thread_local! {
    static GPUI_AREA_CLICKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 根级兜底:每帧把「不该可见」的 webview 全部隐藏 + 焦点调停
pub(super) fn paint_root_sync(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) {
    let active = match store.read(cx).panel_active_tab {
        Some(crate::shell::panel::PanelTab::Browser(id)) => Some(id.0),
        _ => None,
    };
    // 错误页在场时正文 canvas 不挂载,webview 须显式隐藏(canvas
    // 分支的显示门控管不到这条路径)
    let error_active = active.is_some_and(|a| {
        store
            .read(cx)
            .browser
            .tab(BrowserTabId(a))
            .is_some_and(|t| t.nav.error.is_some())
    });
    let hidden_all = active.is_none() || error_active || super::is_suppressed(store, window, cx);
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
    // 焦点调停。gpui 的 focus() 不调 makeFirstResponder——点击 gpui
    // 控件后原生 FR 可能仍留在 webview,键盘进不了 gpui(按键落空
    // beep)。按「gpui 区域是否刚被点击」分流:
    // - 点击 gpui 控件(gpui 收到 mouse-down):FR 归还 GPUIView,
    //   键盘经 gpui 派发到焦点元素(⌘V 也不在页面里错投)
    // - 点击 webview(gpui 无 mouse-down):gpui 焦点是残留,清掉
    window.on_mouse_event(
        |_: &gpui_kit::MouseDownEvent,
         phase: gpui_kit::DispatchPhase,
         _: &mut Window,
         _: &mut App| {
            if phase.bubble() {
                GPUI_AREA_CLICKED.with(|c| c.set(true));
            }
        },
    );
    let clicked_gpui = GPUI_AREA_CLICKED.with(|c| c.replace(false));
    if window.focused(cx).is_some() && first_responder_in_any_webview() {
        if clicked_gpui {
            if let Some(content) = content_view(window)
                && let Some(ns_window) = content.window()
            {
                // TEMP-DIAG: 焦点调停定位(验证后删除)
                let fr = ns_window
                    .firstResponder()
                    .map(|r| r.class().name().to_str().unwrap_or("?").to_string())
                    .unwrap_or_else(|| "<nil>".into());
                eprintln!("[browser-focus] 归位 FR(gpui 区点击) prev_fr={fr}");
                let responder: &NSResponder = &content;
                ns_window.makeFirstResponder(Some(responder));
            }
        } else {
            window.blur(cx);
        }
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
    st.panel_menu_open
        || st.sessions.ws_info_card.is_some()
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
