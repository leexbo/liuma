//! 根布局:侧栏 + 主列(顶栏 / 消息区 / 底部固定栈,
//! `flex h-full overflow-hidden` 结构);hero 态切换
//! (空会话 = 居中输入卡);设置为独立页(右列整体切换);
//! 轨迹页在右栏面板标签。

pub(crate) mod host;
pub(crate) mod metrics;
pub(crate) mod panel;
pub(crate) mod reducer;
pub(crate) mod store;

mod hero;
pub(crate) mod scroll;
mod statusbar;
mod topbar;

use crate::kits::modals::attachment_toast_card;

use gpui_kit::component::StyledExt;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, Context, DispatchPhase, Element, ElementId, Entity, ExternalPaths,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseMoveEvent,
    ParentElement, Pixels, Render, StatefulInteractiveElement as _, Style, Styled, Window, div, px,
};

use crate::features::ask;
use crate::features::attachments;
use crate::features::chat;
use crate::features::opener;
use crate::features::sessions;
use crate::features::settings;
use crate::features::subagents;
use crate::kits::theme;
use crate::shell::reducer::workspace_of;
use crate::shell::store::AppStore;

/// 统一 tooltip 构造:字号 12px(组件库默认 text_sm = 14px,相对本
/// 应用 13px 正文偏大)。全站 tooltip 一律经此构造,别直接 build。
pub(crate) fn tip(
    text: impl Into<gpui_kit::SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui_kit::AnyView {
    let text = text.into();
    move |window, cx| {
        gpui_kit::component::tooltip::Tooltip::new(text.clone())
            .text_size(px(12.))
            .build(window, cx)
    }
}

/// 平台主修饰键名(macOS = `cmd`;Windows/Linux = `ctrl`)。
///
/// gpui **不做**归一:`Keystroke::parse("cmd-…")` 恒设 `modifiers.platform`
/// (非 macOS 上那是 Win 键),而输入框自身绑定在非 macOS 上是 `ctrl-*`
/// (gpui-base `input/base/state.rs`),`Modifiers::secondary_key()` 也把非
/// macOS 映射到 `control` —— 字面写 `cmd-` 的绑定在 Windows 上永不命中。
#[cfg(target_os = "macos")]
pub(crate) const SECONDARY_MOD: &str = "cmd";
/// 非 macOS:主修饰键是 Ctrl(见上)
#[cfg(not(target_os = "macos"))]
pub(crate) const SECONDARY_MOD: &str = "ctrl";

/// 平台主修饰键是否按下(与 gpui `Modifiers::secondary_key()` 同义:
/// macOS = Cmd,Windows/Linux = Ctrl)
pub(crate) fn secondary_modifier_pressed(modifiers: gpui_kit::Modifiers) -> bool {
    #[cfg(target_os = "macos")]
    {
        modifiers.platform
    }
    #[cfg(not(target_os = "macos"))]
    {
        modifiers.control
    }
}

pub fn bind_global_keys(cx: &mut App) {
    // 面板计划快捷键 = ⇧⌘P / ⇧Ctrl+P(修饰键按平台取,见 [`SECONDARY_MOD`])
    let shortcut = format!("shift-{SECONDARY_MOD}-p");
    cx.bind_keys([gpui_kit::KeyBinding::new(
        &shortcut,
        panel::OpenPanelPlan,
        None,
    )]);
}

/// 拖选实时刷新驱动器。
///
/// gpui-base 的选择事件链只更新参与者快照并 emit 事件,不请求刷新
/// 帧(`SelectionChanged` 全库无订阅者),而渲染循环仅按脏标记出帧
/// ——拖选过程中窗口不脏,高亮冻结在按下时的画面,松手后的余动
/// 借其他刷新源(hover 切换等)才补上。本元素在 paint 期挂窗口级
/// mouse-move 监听:按住鼠标拖动且窗口已有文本选区时逐 move 置
/// 脏,高亮随拖动实时渲染(见 selection_tests 的行为锁)。
pub(crate) struct SelectionRefreshDriver;

#[cfg(test)]
pub(crate) static SELECTION_DRIVEN_REFRESHES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

impl IntoElement for SelectionRefreshDriver {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectionRefreshDriver {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some("selection-refresh-driver".into())
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        window.on_mouse_event(
            |event: &MouseMoveEvent, phase: DispatchPhase, window: &mut Window, cx: &mut App| {
                // bubble 相按绘制序逆序执行,本元素画在选择层之后、先于
                // 层处理本帧 move(此刻 anchor==cursor,快照尚为 None)——
                // 状态检查与置脏须 defer 到事件派发完、层更新完之后
                if phase.bubble() && event.pressed_button.is_some() {
                    window.defer(cx, |window, cx| {
                        if gpui_kit::base::TextSelection::has_selection(window, cx) {
                            #[cfg(test)]
                            SELECTION_DRIVEN_REFRESHES
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            window.refresh();
                        }
                    });
                }
            },
        );
    }
}

/// 域尾哨兵:铺满一个视觉域的不可见选择参与者(空文本)。
///
/// gpui-base 拖选终点在 hover 无命中(气泡间隙/composer/空白区)时走
/// predecessor 回退——按 top≤y 全窗口取最大、不看 x,会把相邻列
/// (右栏面板)的块选为终点,跨域区间 [聊天..右栏] 把异域文本整段卷
/// 入选区(真机泄漏形态,回归锁 `drag_into_gap_does_not_spill_via_
/// fallback`)。本元素以绝对定位铺满所在容器,用公开的注册 API 挂
/// 自定义几何参与者:域内非文本区域 hover 命中哨兵而非回退,区间
/// 钳在本域(order = 域尾哨兵,见 [`crate::kits::markdown`] 常量)。
/// 空 runs = 无高亮、无复制贡献。置于域容器首子(绘制序最早,注册
/// 的 hitbox 居栈底):真实文本 hitbox 后注册居上且面积更小,hover
/// 优先命中真实文本,哨兵只兜空白
pub(crate) struct SelectionDomainSink {
    id: gpui_kit::SharedString,
    order: u64,
}

impl SelectionDomainSink {
    pub(crate) fn new(id: impl Into<gpui_kit::SharedString>, order: u64) -> Self {
        Self {
            id: id.into(),
            order,
        }
    }
}

impl IntoElement for SelectionDomainSink {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectionDomainSink {
    type RequestLayoutState = gpui_kit::base::TextSelectionHandle;
    type PrepaintState = gpui_kit::Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone().into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let handle = window.with_element_state(
            // 构造不变式:SelectionDomainSink 必持稳定 element id(AGENTS.md §1 ①)
            #[allow(clippy::expect_used)]
            global_id.expect("SelectionDomainSink must have a stable element id"),
            |retained: Option<gpui_kit::base::TextSelectionHandle>, _| {
                let handle =
                    retained.unwrap_or_else(|| gpui_kit::base::TextSelectionHandle::new("", cx));
                (handle.clone(), handle)
            },
        );
        // 铺满父容器(auto 尺寸在 absolute 容器里高度会塌 0,注册的
        // bounds 盖不住空白区,哨兵失效)
        let style = Style {
            size: gpui_kit::Size {
                width: gpui_kit::Length::Definite(gpui_kit::DefiniteLength::Fraction(1.0)),
                height: gpui_kit::Length::Definite(gpui_kit::DefiniteLength::Fraction(1.0)),
            },
            ..Style::default()
        };
        (window.request_layout(style, [], cx), handle)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, gpui_kit::HitboxBehavior::Normal);
        handle.register(
            gpui_kit::base::TextSelectionRegistration::new(hitbox.clone(), bounds)
                .with_document_order(self.order),
            window,
            cx,
        );
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        _: &mut Window,
        cx: &mut App,
    ) {
        handle.update_runs(&[], cx);
    }
}

/// 工作区根视图(状态收口在 AppStore)
pub struct WorkspaceView {
    store: Entity<AppStore>,
    /// 渲染次数(测试断言重绘节奏:真流式 = 每 chunk 一画)
    #[cfg(test)]
    pub(crate) render_count: usize,
}

impl WorkspaceView {
    /// 构造(搜索输入等挂窗态已在 main 的 attach_window_state 完成)
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        // 状态 → 视图重绘链(真流式的根基):AppStore 每次 notify
        // (帧泵逐 chunk / 轮询 / 操作)必须显式观察转发——gpui 的
        // Entity::notify 只达 observers,render 期读取不构成订阅;
        // 缺此链时窗口只在 OS 事件(鼠标/闪烁)时重绘,chunk 静默
        // 改状态后一次性画出全文 =「假流式」
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        // ⇧⌘P = 开面板计划标签。App 级 on_action(元素级 on_action 须在
        // 焦点路径上,启动未聚焦时收不到;全局监听在 bubble 末尾,无焦点
        // 也送达;Context 有同名低阶方法,须全限定)。键表见
        // shell::bind_global_keys
        let hotkey_store = store.clone();
        App::on_action(cx, move |_: &panel::OpenPanelPlan, cx: &mut App| {
            hotkey_store.update(cx, |st, cx| st.open_panel_tab(panel::PanelTab::Plan, cx));
        });
        // 聊天正文右键「复制」:App 级全局 on_action(右键原生菜单派发的动作
        // 在 bubble 末尾送达全局监听,不受焦点/dispatch path 限制)。选中文
        // 本在右键弹菜单时已抓取(stash,见 chat_pane::render),此处只写剪贴板
        // ——App 级 handler 无 Window,而分发期 Window 已被可变借用,再取
        // 窗口读选中会失败。
        let copy_store = store.clone();
        App::on_action(
            cx,
            move |_: &chat::chat_pane::CopyChatSelection, cx: &mut App| {
                copy_store.update(cx, |st, cx| {
                    if let Some(text) = st.chat.pending_copy_text.take()
                        && !text.trim().is_empty()
                    {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                    }
                });
            },
        );
        // 终端右键「复制」:同上 stash 手法——选中文本在右键弹菜单时
        // 已抓进 terminal.pending_copy,此处只写剪贴板
        let term_copy_store = store.clone();
        App::on_action(
            cx,
            move |_: &crate::features::terminal::CopyTerminalSelection, cx: &mut App| {
                term_copy_store.update(cx, |st, cx| {
                    if let Some(text) = st.terminal.pending_copy.take()
                        && !text.trim().is_empty()
                    {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                    }
                });
            },
        );
        Self {
            store,
            #[cfg(test)]
            render_count: 0,
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        if std::env::var_os("LIUMA_PROBE").is_some() {
            use std::sync::atomic::{AtomicU64, Ordering};
            static T0: AtomicU64 = AtomicU64::new(0);
            static N: AtomicU64 = AtomicU64::new(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if T0.load(Ordering::Relaxed) == 0 {
                T0.store(now, Ordering::Relaxed);
                eprintln!("[t3] render #1 (基准)");
            } else {
                let n = N.fetch_add(1, Ordering::Relaxed) + 2;
                eprintln!("[t3] render #{n} +{}ms", now - T0.load(Ordering::Relaxed));
            }
        }
        // 渲染期窗口态回写:composer 延迟清空 + 消息列虚拟化对齐
        // (splice/reset 记账)+ 跟随滚底 + 轨迹滚动冲洗(订阅/帧泵回调无
        // 窗口句柄或无滚动时机;见 store 字段注释)
        //
        // 探针:这段是渲染期的**主线程固定成本**(行槽重建、流式重测、
        // 轨迹缓存/跟底),帧耗时探针据此区分「重绘慢」与「本段重活慢」
        let flush_t0 = std::time::Instant::now();
        self.store.update(cx, |s, cx| {
            // 面板/侧栏让位协商(优先级:对话列 > 面板下限 > 侧栏;
            // 见 sync_yield_negotiation)。必须先于 col_w/面板渲染宽计算
            s.sync_yield_negotiation(f32::from(window.viewport_size().width), cx);
            s.flush_composer_clear(window, cx);
            s.sync_composer_placeholder(window, cx);
            // 语言档切换回写(偏好下拉标签重建;档位未变零开销早退)
            s.sync_locale_ui(window, cx);
            // 主题下拉跟随 registry(用户主题热装载即重建;签名未变
            // 零开销早退)
            s.sync_theme_selects(window, cx);
            s.install_trajectory_scroll_handler(cx);
            s.flush_trajectory_scroll(cx);
            s.sync_chat_list(cx);
            s.flush_chat_scroll(cx);
            s.sync_retry_tick(cx);
            // 列宽变化通知(宽变失效 → settle 全量重测;见 chat/store 注释)
            s.sync_chat_list_width(
                metrics::window_chat_col_w(
                    window,
                    s.sidebar_collapsed,
                    s.sidebar_px,
                    s.panel_open,
                    s.panel_px,
                ),
                cx,
            );
            // @ 补全 Enter 选中(渲染期消费标志;需 window)
            s.finish_at_completion(window, cx);
            if s.chat.chat_version != s.chat.rendered_version {
                if s.chat.pinned {
                    s.chat.chat_list.scroll_to(gpui_kit::ListOffset {
                        item_ix: usize::MAX,
                        offset_in_item: px(0.),
                    });
                }
                s.chat.rendered_version = s.chat.chat_version;
            }
        });
        if std::env::var_os("LIUMA_PROBE").is_some() {
            let ms = flush_t0.elapsed().as_secs_f64() * 1000.;
            if ms >= 2. {
                eprintln!("[t3] flush {ms:.1}ms (行槽/流式重测/轨迹)");
            }
        }
        let st = self.store.read(cx);
        let hero = st.hero();
        let sidebar_collapsed = st.sidebar_collapsed;
        // 外部文件拖入标志(根级蒙层门的一半;见链尾 drop_overlay 条目)
        let external_drag = st.attachments.external_drag;
        // 对话列宽(消息列/composer/hero 统一;见 metrics 策略;侧栏拖宽后
        // 随 sidebar_px 收窄内容区)
        let col_w = metrics::window_chat_col_w(
            window,
            sidebar_collapsed,
            st.sidebar_px,
            st.panel_open,
            st.panel_px,
        );
        let settings_open = st.settings.settings_open;
        div()
            .relative()
            // 分离式侧栏(方案 B):左右两列——左列侧栏卡满高(顶到窗口
            // 顶 8px 边距,macOS 交通灯浮于卡上,卡内顶部留拖拽让位);
            // 右列 = 标题行 + 内容卡 + 状态栏。标题栏/状态栏不再压缩
            // 侧栏空间,只占内容区域
            .flex()
            .size_full()
            .overflow_hidden()
            // 画布底由 Root 层承担(c.background = BASE),此处不重复铺底
            .text_color(theme::label(cx))
            // 外部文件拖入判定(载荷类型分流):typed on_drag_move 只收
            // ExternalPaths(OS 文件拖入翻译成的内部拖拽),内部拖拽
            // (会话行手动排序等)不触发;EXIT = 平台拖拽离开窗口即清。
            // 蒙层门 = has_active_drag() && external_drag(下同)
            .on_drag_move::<ExternalPaths>({
                let store = self.store.clone();
                move |_, _, cx| store.update(cx, |s, cx| s.set_external_drag(true, cx))
            })
            .on_file_drop_exit({
                let store = self.store.clone();
                move |_, _, cx| store.update(cx, |s, cx| s.set_external_drag(false, cx))
            })
            // 拖选实时刷新驱动器(零尺寸;见其文档)——必须与本列同窗,
            // 监听挂在窗口级,置脏后渲染循环出帧高亮才实时
            .child(SelectionRefreshDriver)
            // 聊天域尾哨兵:铺满窗口(栈底),拖选落空时终点钳在聊天域,
            // 不经 predecessor 回退跳进右栏(见 SelectionDomainSink)
            .child(div().absolute().size_full().child(SelectionDomainSink::new(
                "sel-sink-chat",
                crate::kits::selection_order::CHAT_TAIL_ORDER,
            )))
            // Esc = 关查看器(捕获相:查看器开启时拦截吞掉;查看器关闭
            // 时无其他 Esc 语义,放行)
            .capture_key_down({
                let store = self.store.clone();
                move |ev: &gpui_kit::KeyDownEvent, _window, cx| {
                    if ev.keystroke.key.as_str() == "escape"
                        && store.read(cx).chat.mermaid_viewer.is_some()
                    {
                        store.update(cx, |st, cx| st.close_mermaid_viewer(cx));
                        cx.stop_propagation();
                    }
                }
            })
            // 左列:全高扁平面板(贴窗口边,无卡)
            .child(sessions::render(&self.store, cx))
            // 右列:设置页开 = 整列接管(顶栏/内容/状态栏全部让位,
            // 设置页自带拖拽头——工作区 chrome 不压在设置页上);
            // 闭 = 标题行 → 内容卡 → 状态栏
            .child(if settings_open {
                settings::render(&self.store, cx).into_any_element()
            } else {
                div()
                    .v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .h_full()
                    // 自绘标题栏(拖拽/双击缩放由 TitleBar 提供;
                    // pl 覆盖 = 交通灯已移左列侧栏上,右列无让位;
                    // 无底部分割线(靠留白分层)
                    .child(
                        gpui_kit::component::TitleBar::new()
                            .pl(px(8.))
                            .h(px(34.))
                            .child(topbar::title_bar_row(&self.store, window, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .flex_1()
                            .flex_col()
                            // 内容区扁平(去胶囊卡):直接落 BASE 画布,满幅
                            // 到窗口右缘;bubble/工具行/composer 作为浮层
                            // 组件与画布形成层次
                            .overflow_hidden()
                            // 滚动条挂点:组件库默认滚动条盖满本列(全高,
                            // 到窗底;挂在 chat_pane 内只到聊天框上边,
                            // 不可取)
                            .relative()
                            // 测试钩子:布局回归断言双栏分离(release 空操作)
                            .debug_selector(|| "content-card".to_string())
                            .when(hero, |el| {
                                el.child(hero::render(&self.store, col_w, window, cx))
                            })
                            .when(!hero, |el| {
                                el.child(
                                    chat::chat_pane::render(&self.store, window, cx)
                                        .into_any_element(),
                                )
                                .child(
                                    // 底部栈:统一列宽容器(与消息列同宽同中心线)
                                    div()
                                        .v_flex()
                                        .flex_shrink_0()
                                        // 列对齐容器同款槽 padding(左锚点槽/
                                        // 右滚动条槽,与列表容器同中心线)
                                        .pl(px(metrics::H_PAD + metrics::NAV_GUTTER_W))
                                        .pr(px(metrics::H_PAD + metrics::SCROLLBAR_GUTTER_W))
                                        .pt(px(4.))
                                        .pb(px(8.))
                                        .child(
                                            div()
                                                .v_flex()
                                                .mx_auto()
                                                .w(col_w)
                                                .gap(px(8.))
                                                .children(ask::render_plan(&self.store, window, cx))
                                                .children(ask::render_approval(&self.store, cx))
                                                .children(ask::render_question(
                                                    &self.store,
                                                    window,
                                                    cx,
                                                ))
                                                // todo_dock = 纯状态展示(非阻塞交互,
                                                // 不同于审批/提问)
                                                .children(chat::todo_dock::render(&self.store, cx))
                                                // 子代理任务条(聊天框上方常驻
                                                // chips:运行中子代理一键切换查看)
                                                .when_some(
                                                    subagents::task_bar(&self.store, cx),
                                                    |el, bar| el.child(bar),
                                                )
                                                // composer 附着组(gap 0):队列条贴入
                                                // 输入卡顶(负 margin 塞 3px,输入卡
                                                // 顶边收口),DSH QueueDock 同构;队列
                                                // 空时 dock 为空节点,组退化为裸
                                                // composer
                                                .child(
                                                    div()
                                                        .v_flex()
                                                        .child(chat::queue_dock::render(
                                                            &self.store,
                                                            window,
                                                            cx,
                                                        ))
                                                        .child(chat::composer::render(
                                                            &self.store,
                                                            window,
                                                            cx,
                                                        )),
                                                ),
                                        ),
                                )
                                // 组件库默认滚动条(滚动时浮现、闲置淡出;拖
                                // 曳走 ListState 协作 API):元素填满父容器 =
                                // 盖满整列到窗底(挂 chat_pane 内只到聊天框
                                // 上边,不成全高)。
                                // 组件库把轨道自身高当滚动视口,全高轨道的
                                // 差值段(底部栈/状态栏)成 thumb
                                // 盲区(「不能拉到底部」)
                                // → FullTrackHandle 以 (轨道高−视口高) 补偿
                                // content_size,满行程映射回列表真实滚动域
                                .child({
                                    let st = self.store.read(cx);
                                    let extra = st.chat.track_h
                                        - f32::from(
                                            st.chat.chat_list.viewport_bounds().size.height,
                                        );
                                    let handle =
                                        scroll::FullTrackHandle::new(&st.chat.chat_list, px(extra));
                                    // viewport_from_layout:轨道钉元素布局
                                    // bounds(全列)——默认走 handle 的列表
                                    // 视口,轨道会缩在列表段;全列轨道 +
                                    // handle 的 content_size
                                    // 补偿 = thumb 满行程映射回列表真实滚动域
                                    gpui_kit::component::scroll::Scrollbar::new(&handle)
                                        .viewport_from_layout()
                                })
                                // 轨道高捕获(渲染期 paint;差值补偿的分子,
                                // 见 shell/scroll.rs)。absolute 层不占 flex
                                // 位、无 hitbox 不挡交互;变化守卫在
                                // note_track_h(防每帧 notify 死循环)
                                .child({
                                    let cap = self.store.clone();
                                    div()
                                        .absolute()
                                        .inset_0()
                                        // canvas 默认 0 高(nav_track 靠两个
                                        // 0 高 canvas 的 origin 夹边界);
                                        // 此处要 size,须显式铺满容器
                                        .child(
                                            gpui_kit::canvas(
                                                move |b, _, cx| {
                                                    let h = b.size.height.as_f32();
                                                    cap.update(cx, |st, cx| st.note_track_h(h, cx));
                                                },
                                                |_, _, _, _| {},
                                            )
                                            .size_full(),
                                        )
                                })
                            }),
                    )
                    // 状态栏(仅右列 26px:统计 + 权限/模型/分支徽标;
                    // 侧栏下方不再被压)
                    .child(statusbar::render(&self.store, cx))
                    .into_any_element()
            })
            // 右侧面板(右栏;settings 整列接管时面板内部自行
            // 返回空,收起时同)
            .when_some(
                (!self.store.read(cx).settings.settings_open)
                    .then(|| panel::render(&self.store, window, cx)),
                |el, panel| el.child(panel),
            )
            // composer 权限/模型/上下文三卡已迁组件库 Popover(触发钮
            // 即弹层锚;开态/外点关闭/上开定位由库托管,根级挂载与
            // bounds 捕获 canvas 移除)
            // 重命名/删除确认/拉取模型/full-access 风险确认四模态已迁
            // 组件库 Dialog 层(store 经 with_window 桥开/关;Esc、遮罩
            // 点击与焦点陷阱由库托管),根级不再条件渲染
            // 工作区信息卡(根级手绘浮层:库受控 Popover 的 set_open
            // 抢窗口焦点,hover 驱动开合震荡,故此卡单独回归根级。锚 =
            // 悬停进入时指针位置(store 于事件分发期捕获
            // window.mouse_position(),当帧即有,无渲染期捕获的一帧
            // 延迟):卡贴侧栏右缘伸出(不遮行内 ⋯/铅笔钮),纵向对齐
            // 悬停行;卡本体 on_hover 续命,行/铅笔/卡三方悬停同步于
            // store,双离开 300ms 关(ws_info_close_task 期间开态保持,
            // 指针跨越行→卡空隙不掉卡))
            .when_some(
                self.store.read(cx).sessions.ws_info_card.clone(),
                |el, (ws, at)| {
                    let st = self.store.read(cx);
                    let count = {
                        let default = st.default_workspace().to_string();
                        st.state
                            .sessions
                            .iter()
                            .filter(|s| {
                                s.origin.as_deref() != Some("subagent")
                                    && workspace_of(&s.session_id, &default) == ws
                            })
                            .count()
                    };
                    let vw = f32::from(window.viewport_size().width);
                    let vh = f32::from(window.viewport_size().height);
                    let card_w = 280.;
                    // 纵向对齐悬停行(锚 y 上移半行高 17),上下夹紧在
                    // 视口内;卡高约 154(四行 34 + 分隔 + 内距),钳位
                    // 留裕量
                    let top = (f32::from(at.y) - 17.).clamp(8., (vh - 162.).max(8.));
                    // 左贴侧栏右缘;窄窗右溢出时钳回
                    let sidebar_w = f32::from(metrics::sidebar_width_for(
                        st.sidebar_collapsed,
                        st.sidebar_px,
                    ));
                    let left = (sidebar_w + 8.).min((vw - card_w - 8.).max(8.));
                    let store = self.store.clone();
                    el.child(
                        div()
                            .id("ws-info-card-root")
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .occlude()
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .on_hover({
                                let store = store.clone();
                                move |hovering: &bool, _, cx| {
                                    store.update(cx, |st, cx| {
                                        st.set_ws_info_hover_card(*hovering, cx)
                                    });
                                }
                            })
                            .child(sessions::ws_info_card(&store, cx, &ws, count)),
                    )
                },
            )
            // 首运行 onboarding(无任何可用凭据)
            .when(self.store.read(cx).settings.needs_onboarding, |el| {
                el.child(settings::onboarding_modal(&self.store, cx))
            })
            // 图片 Lightbox 与附件拒收 toast(root 级树序末尾,
            // 后绘制在上;Lightbox 遮罩叠于全部内容)
            .when(self.store.read(cx).attachments.lightbox.is_some(), |el| {
                el.child(attachments::lightbox(&self.store, cx))
            })
            // 拖拽邀请蒙层(根级;gpui-pre 将 OS 文件拖放翻译为内部
            // active_drag,拖动期间全屏重绘,蒙层即落点)。active_drag
            // 无公开判型面,载荷类型由 external_drag 标志旁路(见根级
            // on_drag_move::<ExternalPaths>):内部拖拽不渲染蒙层
            .when(cx.has_active_drag() && external_drag, |el| {
                el.child(attachments::drop_overlay(&self.store, cx))
            })
            // Mermaid 查看器(根级;与 lightbox 同构。置于 toast 前——
            // 下载完成通知须浮于查看器遮罩之上)
            .when(self.store.read(cx).chat.mermaid_viewer.is_some(), |el| {
                el.child(chat::mermaid_viewer::mermaid_viewer(
                    &self.store,
                    window,
                    cx,
                ))
            })
            .when(
                self.store.read(cx).attachments.attachment_toast.is_some(),
                |el| el.child(attachment_toast_card(&self.store, cx)),
            )
            // 「在编辑器中打开」启动失败 toast(与附件拒收 toast 同构)
            .when(self.store.read(cx).opener.launch_error.is_some(), |el| {
                el.child(opener::launch_error_toast(&self.store, cx))
            })
        // Dialog/Sheet/Notification 三层由 Root 托管(0.7.0 起 Root
        // 自渲染,应用根视图不再挂层;`Root::render_*_layer` 已删)。
        // 层位于 Root 的 absolute/inset_0 overlay 子树,绘制在手写
        // overlay 之后且覆盖整窗——原「模态遮罩压过 lightbox/查看器」
        // 与「通知浮于全部 overlay」两条树序意图由库侧保证
    }
}

/// 真实整树布局回归:AppHost(fake,临时根)→ AppStore → 注入长会话
/// 节点(用户/助手/工具/收尾混合)→ WorkspaceView 渲染,断言相邻
/// 节点 bounds 垂直不交叠。
#[cfg(test)]
mod layout_tests;
