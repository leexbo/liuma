//! 浏览器功能切片状态:每标签一只导航状态机(`NavState` 快照 +
//! `NavCommand` 待执行命令)。UI 侧只读写本表;原生 webview 层
//! (native.rs,仅 macOS)在渲染期取走 `pending` 命令执行,并把导航
//! 回调折叠成 `NavEvent` 经 [`AppStore::browser_on_nav_event`] 回写。
//! 非 macOS 上命令在渲染期被取走丢弃(无原生层,正文渲染占位)。

use std::collections::HashMap;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Window};

use crate::shell::panel::BrowserTabId;
use crate::shell::store::AppStore;

use super::url::{UrlReject, normalize_browser_url};

/// 导航命令(UI 发起,渲染期由原生层取走执行)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NavCommand {
    Load(String),
    Reload,
    Stop,
    Back,
    Forward,
}

/// 导航状态快照(原生导航回调折叠;地址栏/按钮据此渲染)
#[derive(Debug, Clone)]
pub(crate) struct NavState {
    /// 当前页 URL(空 = 尚未导航)
    pub url: String,
    /// 页面标题(None = 未观察到,标签/工具栏回落通用名)
    pub title: Option<String>,
    /// 加载进行中
    pub loading: bool,
    /// 后退/前进可用性
    pub can_back: bool,
    pub can_fwd: bool,
    /// 最近一次失败(加载失败/地址被拒;下次导航清除)
    pub error: Option<String>,
}

impl NavState {
    fn idle() -> Self {
        Self {
            url: String::new(),
            title: None,
            loading: false,
            can_back: false,
            can_fwd: false,
            error: None,
        }
    }
}

/// 移动视口预设(工具栏循环切换;Desktop = 填满面板列,
/// 其余 = 定宽居中,webview frame 与 UA 跟随)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewportPreset {
    Desktop,
    W375,
    W390,
    W428,
}

impl ViewportPreset {
    /// 视口宽(None = 桌面全宽)
    pub(crate) fn width(self) -> Option<f32> {
        match self {
            Self::Desktop => None,
            Self::W375 => Some(375.),
            Self::W390 => Some(390.),
            Self::W428 => Some(428.),
        }
    }

    /// 循环切换下一档
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Desktop => Self::W375,
            Self::W375 => Self::W390,
            Self::W390 => Self::W428,
            Self::W428 => Self::Desktop,
        }
    }

    /// 工具栏短标签(数字档即宽度;桌面档走 i18n 键)
    pub(crate) fn label(self) -> String {
        match self {
            Self::Desktop => crate::kits::i18n::t!("browser.viewport_desktop").to_string(),
            other => format!("{}", other.width().unwrap_or(0.) as i32),
        }
    }
}

/// 一路浏览器标签:面板 `PanelTab::Browser(id)` 的状态载体
pub(crate) struct BrowserTab {
    /// 导航状态快照
    pub nav: NavState,
    /// 待执行导航命令(渲染期原生层取走;None = 无)
    pub pending: Option<NavCommand>,
    /// 地址栏输入(None = 未 ensure;懒建于正文渲染期)
    pub addr_input: Option<Entity<InputState>>,
    /// 地址栏编辑中(Focus 起,Blur/Enter 止;编辑期不回填 URL)
    pub addr_editing: bool,
    /// 视口预设
    pub viewport: ViewportPreset,
}

/// 浏览器切片状态(多标签;序无关,面板标签序由 panel_tabs 承担)
pub(crate) struct BrowserStore {
    /// 标签表(键 = BrowserTabId.0)
    pub tabs: HashMap<u64, BrowserTab>,
    /// id 分配器(自 1 起;0 = `BrowserTabId::NEW` 哨兵)
    next_id: u64,
}

impl BrowserStore {
    pub(crate) fn new() -> Self {
        Self {
            tabs: HashMap::new(),
            next_id: 1,
        }
    }

    pub(crate) fn tab(&self, id: BrowserTabId) -> Option<&BrowserTab> {
        self.tabs.get(&id.0)
    }

    pub(crate) fn tab_mut(&mut self, id: BrowserTabId) -> Option<&mut BrowserTab> {
        self.tabs.get_mut(&id.0)
    }

    /// 分配真实标签 id(菜单哨兵经 open_panel_tab 兑换)
    pub(crate) fn alloc_id(&mut self) -> BrowserTabId {
        let id = BrowserTabId(self.next_id);
        self.next_id += 1;
        id
    }

    /// 标签建档(无则建;面板开合钩子调用,无 window 面)
    fn ensure_record(&mut self, id: BrowserTabId) {
        self.tabs.entry(id.0).or_insert_with(|| BrowserTab {
            nav: NavState::idle(),
            pending: None,
            addr_input: None,
            addr_editing: false,
            viewport: ViewportPreset::Desktop,
        });
    }
}

/// 原生层回写的导航事件(native.rs delegate 回调折叠产物;接口随
/// macOS webview 层接入而启用)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum NavEvent {
    /// 导航开始(url = 目标地址)
    Started { url: String },
    /// 主文档提交(URL 生效)
    Committed { url: String },
    /// 导航完成(标题在此刻可读)
    Finished { url: String, title: Option<String> },
    /// 导航失败(用户取消不算)
    Failed { url: String, message: String },
    /// 能力快照(后退/前进可用性)
    Capability { can_back: bool, can_fwd: bool },
}

impl AppStore {
    /// 标签切入/打开钩子:建档(open_panel_tab / activate_panel_tab 调
    /// 用,无 window 面;地址栏输入与订阅在正文渲染期懒建)
    pub(crate) fn browser_ensure_record(&mut self, id: BrowserTabId, cx: &mut Context<Self>) {
        self.browser.ensure_record(id);
        cx.notify();
    }

    /// 正文渲染期完整 ensure:建档 + 地址栏输入懒建 + 事件订阅
    /// (Focus/Blur 驱动编辑态,Enter = 提交导航)
    pub(crate) fn browser_ensure(
        &mut self,
        id: BrowserTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browser.ensure_record(id);
        let needs_input = self.browser.tab(id).is_none_or(|t| t.addr_input.is_none());
        if !needs_input {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::kits::i18n::t!("browser.addr_placeholder"))
        });
        cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            let Some(tab) = this.browser.tab_mut(id) else {
                return;
            };
            match event {
                InputEvent::Focus => tab.addr_editing = true,
                InputEvent::Blur => tab.addr_editing = false,
                InputEvent::PressEnter { .. } => {
                    tab.addr_editing = false;
                    this.browser_addr_commit(id, cx);
                }
                InputEvent::Change => {}
            }
        })
        .detach();
        if let Some(tab) = self.browser.tab_mut(id) {
            tab.addr_input = Some(input);
        }
        cx.notify();
    }

    /// 关闭标签:原生层拆视图/停转发(native::remove_tab,主线程),
    /// 状态面移除
    pub(crate) fn browser_remove(&mut self, id: BrowserTabId, cx: &mut Context<Self>) {
        super::native::remove_tab(id);
        self.browser.tabs.remove(&id.0);
        cx.notify();
    }

    /// 渲染期取走待执行命令(原生层执行入口)
    pub(crate) fn browser_take_pending(&mut self, id: BrowserTabId) -> Option<NavCommand> {
        self.browser.tab_mut(id).and_then(|t| t.pending.take())
    }

    /// 渲染期无原生层的命令落位(丢弃命令并结束加载态,避免无限
    /// 转圈;仅非 macOS stub 的 paint_mount 调用)
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn browser_drop_pending(&mut self, id: BrowserTabId, cx: &mut Context<Self>) {
        if let Some(tab) = self.browser.tab_mut(id)
            && tab.pending.take().is_some()
        {
            tab.nav.loading = false;
            cx.notify();
        }
    }

    /// 发起导航(工具栏/外链入口):置 pending + 即时置 loading(原生层
    /// 渲染期取走;清 error 给即时反馈)
    pub(crate) fn browser_navigate(
        &mut self,
        id: BrowserTabId,
        cmd: NavCommand,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.browser.tab_mut(id) {
            if let NavCommand::Load(url) = &cmd {
                tab.nav.url = url.clone();
                tab.nav.error = None;
            }
            tab.pending = Some(cmd);
            tab.nav.loading = true;
            cx.notify();
        }
    }

    /// 地址栏提交:读输入规范化;通过 → 导航,被拒 → 报错条(空输入静默)
    pub(crate) fn browser_addr_commit(&mut self, id: BrowserTabId, cx: &mut Context<Self>) {
        let raw = self
            .browser
            .tab(id)
            .and_then(|t| t.addr_input.as_ref())
            .map(|i| i.read(cx).value().trim().to_string());
        let Some(raw) = raw else { return };
        match normalize_browser_url(&raw) {
            Ok(url) => self.browser_navigate(id, NavCommand::Load(url), cx),
            Err(UrlReject::Empty) => {}
            Err(other) => {
                if let Some(tab) = self.browser.tab_mut(id) {
                    tab.nav.error = Some(super::url_reject_message(&other));
                }
                cx.notify();
            }
        }
    }

    /// 地址栏回填当前 URL(非编辑态渲染期调用;编辑中不覆盖)
    pub(crate) fn browser_addr_sync(
        &mut self,
        id: BrowserTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.browser.tab(id) else {
            return;
        };
        if tab.addr_editing || tab.nav.url.is_empty() {
            return;
        }
        let url = tab.nav.url.clone();
        let Some(input) = tab.addr_input.clone() else {
            return;
        };
        if input.read(cx).value() != url {
            input.update(cx, |s, cx| s.set_value(url.as_str(), window, cx));
        }
    }

    /// 原生层导航事件回写(折叠进 NavState;随 macOS webview 层接入
    /// 启用)
    #[allow(dead_code)]
    pub(crate) fn browser_on_nav_event(
        &mut self,
        id: BrowserTabId,
        event: NavEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.browser.tab_mut(id) else {
            return;
        };
        match event {
            NavEvent::Started { url } => {
                tab.nav.url = url;
                tab.nav.loading = true;
                tab.nav.error = None;
            }
            NavEvent::Committed { url } => {
                tab.nav.url = url;
                tab.nav.loading = true;
            }
            NavEvent::Finished { url, title } => {
                tab.nav.url = url;
                tab.nav.title = title;
                tab.nav.loading = false;
            }
            NavEvent::Failed { url, message } => {
                tab.nav.url = url;
                tab.nav.loading = false;
                tab.nav.error = Some(message);
            }
            NavEvent::Capability { can_back, can_fwd } => {
                tab.nav.can_back = can_back;
                tab.nav.can_fwd = can_fwd;
            }
        }
        cx.notify();
    }

    /// 外链入口(聊天链接等):同 URL 标签已在则激活原址重载,否则
    /// 开新标签导航;面板未开随 open_panel_tab 展开(随链接路由接线
    /// 启用)
    #[allow(dead_code)]
    pub(crate) fn open_browser_tab(&mut self, url: &str, cx: &mut Context<Self>) {
        let Ok(url) = normalize_browser_url(url) else {
            return;
        };
        let existing = self
            .browser
            .tabs
            .iter()
            .find(|(_, t)| t.nav.url == url)
            .map(|(k, _)| BrowserTabId(*k));
        match existing {
            Some(id) => {
                self.activate_panel_tab(crate::shell::panel::PanelTab::Browser(id), cx);
                self.browser_navigate(id, NavCommand::Reload, cx);
            }
            None => {
                let id = self.browser.alloc_id();
                self.open_panel_tab(crate::shell::panel::PanelTab::Browser(id), cx);
                self.browser_navigate(id, NavCommand::Load(url), cx);
            }
        }
    }
}
