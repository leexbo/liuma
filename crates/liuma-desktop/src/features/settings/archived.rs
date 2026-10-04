//! 归档聊天页:排序/过滤/删除/清空/恢复方法与归档区、分组、行、删除确认弹层视图。

use super::*;

impl AppStore {
    /// 进入归档区(nav 钩子):惰建搜索输入与两下拉 + 首拉清单。
    /// 重复进入不重置筛选态(回导航再进,搜索词/排序/项目筛选保留)
    pub(crate) fn open_archived_section(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.archived_search.is_none() {
            self.settings.archived_search = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.archived_search_ph"))
            }));
        }
        self.ensure_archived_order_select(window, cx);
        self.ensure_archived_project_select(window, cx);
        if self.settings.archived.is_none() {
            self.refresh_archived(cx);
        }
    }

    /// 归档清单异步重拉:全项目 .archive 扫描要读日志派生事实(标题/blank),
    /// 照清单刷新先例上 blocking 池,不占 runtime worker。回填后项目选项集
    /// 可能已变(归档项首次出现/清空),经窗桥惰重建下拉
    pub fn refresh_archived(&mut self, cx: &mut Context<Self>) {
        self.settings.archived_loading = true;
        let store = cx.entity().clone();
        let host = self.bridge.host().clone();
        let rx = self
            .bridge
            .call_blocking(move || host.list_archived_sessions());
        cx.spawn(async move |_this, cx| {
            let items = rx.await;
            store.update(cx, |s, cx| {
                s.settings.archived_loading = false;
                if let Ok(items) = items {
                    s.settings.archived = Some(items);
                }
                // 下拉重建需要 window,store 侧无窗上下文 → 窗桥延迟执行
                if s.archived_project_keys() != s.settings.archived_project_options {
                    let deferred = store.clone();
                    s.with_window_deferred(cx, move |window, cx| {
                        deferred.update(cx, |s, cx| s.ensure_archived_project_select(window, cx));
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 归档项目下拉选项集(「全部项目」置首 + 清单内 pkey 按首见序;
    /// 显示名 = 工作区标题回退名,孤儿 = pkey slug)
    pub(crate) fn archived_project_choices(&self) -> Vec<(String, String)> {
        let mut choices = vec![(
            String::new(),
            t!("settings.archived_project_all").into_owned(),
        )];
        let Some(items) = &self.settings.archived else {
            return choices;
        };
        for pkey in items.iter().map(|i| i.project_key.clone()) {
            if choices.iter().any(|(v, _)| *v == pkey) {
                continue;
            }
            let ws = items
                .iter()
                .find(|i| i.project_key == pkey)
                .and_then(|i| i.workspace.clone());
            let name = self.archived_group_name(&pkey, ws.as_deref());
            choices.push((pkey, name));
        }
        choices
    }

    /// 归档分组显示名:工作区标题(侧栏同源)> 工作区名;孤儿归档 = pkey
    /// 去首尾 `-` 的 slug(`--Volumes-…-x--` → `Volumes-…-x`)
    pub(crate) fn archived_group_name(&self, pkey: &str, ws: Option<&str>) -> String {
        match ws {
            Some(name) => self.title_for_workspace(name),
            None => pkey.trim_matches('-').to_string(),
        }
    }

    /// 归档清单内出现过的 pkey(首见序;下拉选项集的重建判据)
    pub(crate) fn archived_project_keys(&self) -> Vec<String> {
        let Some(items) = &self.settings.archived else {
            return Vec::new();
        };
        let mut keys: Vec<String> = Vec::new();
        for pkey in items.iter().map(|i| &i.project_key) {
            if !keys.contains(pkey) {
                keys.push(pkey.clone());
            }
        }
        keys
    }

    /// 排序下拉构建(静态两项;Confirm 回写排序态)
    pub(crate) fn ensure_archived_order_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.archived_order_select.is_some() {
            return;
        }
        let options = vec![
            (
                "updated".to_string(),
                t!("settings.archived_order_updated").into_owned(),
            ),
            (
                "alpha".to_string(),
                t!("settings.archived_order_alpha").into_owned(),
            ),
        ];
        let current = match self.settings.archived_order {
            ArchivedOrder::Updated => "updated",
            ArchivedOrder::Alpha => "alpha",
        };
        self.settings.archived_order_select = Some(build_archived_select(
            options,
            current,
            window,
            cx,
            move |this, id, cx| {
                let order = match id.as_str() {
                    "alpha" => ArchivedOrder::Alpha,
                    _ => ArchivedOrder::Updated,
                };
                this.set_archived_order(order, cx);
            },
        ));
    }

    /// 项目下拉构建/同步:选项集相对上次构建有增减(或首建)才整体重建;
    /// 当前筛选值仍在新选项集内则保持选中
    pub(crate) fn ensure_archived_project_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choices = self.archived_project_choices();
        let keys: Vec<String> = choices.iter().skip(1).map(|(v, _)| v.clone()).collect();
        let built = self.settings.archived_project_select.is_some();
        if built && keys == self.settings.archived_project_options {
            return;
        }
        self.settings.archived_project_options = keys;
        // 当前筛选项目已不在选项集(其归档被清空)→ 复位为全部项目,
        // 避免显示「全部」而实际仍按失效 pkey 过滤的错位
        let current = self.settings.archived_project.clone().unwrap_or_default();
        if !current.is_empty() && !choices.iter().any(|(v, _)| *v == current) {
            self.settings.archived_project = None;
        }
        let current = self.settings.archived_project.clone().unwrap_or_default();
        let select = build_archived_select(choices, &current, window, cx, |this, id, cx| {
            let pkey = if id.is_empty() { None } else { Some(id) };
            this.set_archived_project(pkey, cx);
        });
        self.settings.archived_project_select = Some(select);
    }

    /// 归档排序切换(视图层重排;core 正典序不受影响)
    pub fn set_archived_order(&mut self, order: ArchivedOrder, cx: &mut Context<Self>) {
        if self.settings.archived_order != order {
            self.settings.archived_order = order;
            cx.notify();
        }
    }

    /// 归档项目筛选切换(None = 全部项目)
    pub fn set_archived_project(&mut self, pkey: Option<String>, cx: &mut Context<Self>) {
        if self.settings.archived_project != pkey {
            self.settings.archived_project = pkey;
            cx.notify();
        }
    }

    /// 删除单条归档(确认弹窗;文案点名会话标题)
    pub fn ask_delete_archived(
        &mut self,
        archive_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let store = cx.entity().clone();
        self.with_window_deferred(cx, move |window, cx| {
            open_archived_confirm_dialog(
                &store,
                ArchivedConfirmKind::Delete { archive_id },
                t!("settings.archived_delete_title"),
                t!("settings.archived_delete_desc", title = title),
                window,
                cx,
            );
        });
        cx.notify();
    }

    /// 删除单项目全部归档(确认弹窗;文案点名项目与条数)
    pub fn ask_purge_project_archived(
        &mut self,
        pkey: String,
        project: String,
        n: usize,
        cx: &mut Context<Self>,
    ) {
        let store = cx.entity().clone();
        self.with_window_deferred(cx, move |window, cx| {
            open_archived_confirm_dialog(
                &store,
                ArchivedConfirmKind::PurgeProject { pkey },
                t!("settings.archived_purge_title"),
                t!("settings.archived_purge_desc", project = project, n = n),
                window,
                cx,
            );
        });
        cx.notify();
    }

    /// 清空全部归档(确认弹窗;文案点名总条数)
    pub fn ask_clear_archived(&mut self, n: usize, cx: &mut Context<Self>) {
        let store = cx.entity().clone();
        self.with_window_deferred(cx, move |window, cx| {
            open_archived_confirm_dialog(
                &store,
                ArchivedConfirmKind::ClearAll,
                t!("settings.archived_clear_title"),
                t!("settings.archived_clear_desc", n = n),
                window,
                cx,
            );
        });
        cx.notify();
    }

    /// 归档确认动作(弹窗确认钮直派):删除/清空上 blocking 池(递归删
    /// 目录可能大),完成后重拉归档区;失败走设置页内通告
    pub(crate) fn confirm_archived_action(
        &mut self,
        kind: ArchivedConfirmKind,
        cx: &mut Context<Self>,
    ) {
        let host = self.bridge.host().clone();
        let rx = self.bridge.call_blocking(move || match kind {
            ArchivedConfirmKind::Delete { archive_id } => {
                host.delete_archived_session(&archive_id).map(|_| ())
            }
            ArchivedConfirmKind::PurgeProject { pkey } => {
                host.clear_archived_sessions(Some(&pkey)).map(|_| ())
            }
            ArchivedConfirmKind::ClearAll => host.clear_archived_sessions(None).map(|_| ()),
        });
        cx.spawn(async move |this, cx| {
            // 桥接掉线(回执通道断)与宿主业务失败同路呈现
            let result = rx.await.unwrap_or_else(|_| {
                Err(liuma_core::proto::RpcError::internal(
                    t!("settings.archived_ack_lost").into_owned(),
                ))
            });
            this.update(cx, |s, cx| match result {
                Ok(()) => s.refresh_archived(cx),
                Err(e) => s.set_settings_notice(
                    false,
                    t!("settings.archived_delete_failed", msg = &e.message),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    /// 取消归档:恢复到侧栏原项目分组。宿主广播 host/session-added →
    /// 既有 Effect::Sessions 链路自动刷侧栏(不手动双刷);此处只重拉
    /// 归档区,不切会话
    pub fn unarchive_archived(&mut self, archive_id: String, cx: &mut Context<Self>) {
        let host = self.bridge.host().clone();
        let rx = self
            .bridge
            .call_blocking(move || host.unarchive_session(&archive_id));
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| {
                Err(liuma_core::proto::RpcError::internal(
                    t!("settings.archived_ack_lost").into_owned(),
                ))
            });
            this.update(cx, |s, cx| match result {
                Ok(()) => s.refresh_archived(cx),
                Err(e) => s.set_settings_notice(
                    false,
                    t!("settings.archived_unarchive_failed", msg = &e.message),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }
}

// ── 已归档的聊天(数据与统计组) ─────────────────────────────────

/// 归档区视图纯函数:项目筛选 + 搜索词过滤(标题)+ 排序 + 按 pkey
/// 分组,返回 (pkey, 显示名, 成员下标) 列表。render 与测试共用。
/// 组序:Updated = 清单序首见(core 已按 updated 倒序);Alpha = 显示名
/// 字母序,组内行按标题字母序(更新时间倒序破并列)
pub(crate) fn archived_view(
    items: &[ArchivedSessionSummary],
    query: &str,
    project: Option<&str>,
    order: ArchivedOrder,
    name_of: impl Fn(&str, Option<&str>) -> String,
) -> Vec<(String, String, Vec<usize>)> {
    let query = query.trim().to_lowercase();
    let title_of = |i: usize| -> String {
        items[i]
            .projections
            .as_ref()
            .and_then(|p| p.values["title"].as_str())
            .unwrap_or(&items[i].session_id)
            .to_string()
    };
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    for (ix, item) in items.iter().enumerate() {
        if project.is_some_and(|p| p != item.project_key) {
            continue;
        }
        if !query.is_empty() && !title_of(ix).to_lowercase().contains(&query) {
            continue;
        }
        match groups.iter_mut().find(|(k, _, _)| *k == item.project_key) {
            Some((_, _, members)) => members.push(ix),
            None => {
                let name = name_of(&item.project_key, item.workspace.as_deref());
                groups.push((item.project_key.clone(), name, vec![ix]));
            }
        }
    }
    if order == ArchivedOrder::Alpha {
        groups.sort_by_key(|a| a.1.to_lowercase());
        for (_, _, members) in &mut groups {
            members.sort_by_key(|&a| title_of(a).to_lowercase());
        }
    }
    groups
}

/// 归档区主渲染:标题 + 全部删除(危险胶囊)/ 搜索 + 排序 + 项目筛选
/// 控制行 / 加载、空态、零命中分流 / 按项目分组的行列表。
///
/// 高度骨架:h_full 占满设置右列(720 包装层被 settings-options 行向
/// flex stretch 拉成视口高),标题与控制行固定,分组列表在 **内部滚动
/// 区** 滚动(flex_1 + overflow_y_scroll)——外层滚动容器的滚动范围
/// 恒等于视口(包装层 stretch 所致),长清单靠外层滚不动,必须内滚
pub(crate) fn archived_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let query = st
        .settings
        .archived_search
        .as_ref()
        .map(|e| e.read(cx).value().trim().to_lowercase())
        .unwrap_or_default();
    let items: &[ArchivedSessionSummary] = st.settings.archived.as_deref().unwrap_or(&[]);
    let name_of = |pkey: &str, ws: Option<&str>| st.archived_group_name(pkey, ws);
    let groups = archived_view(
        items,
        &query,
        st.settings.archived_project.as_deref(),
        st.settings.archived_order,
        name_of,
    );

    // 头部行:标题 + 说明 | 右侧「全部删除」(危险色胶囊;清空动作
    // 自带确认弹窗,按钮恒在场——空清单点了也只会弹确认后清 0 条,
    // 与「无归档」空态不冲突)。失败通告行缀在标题块下,不与按钮抢位
    let mut col = div()
        .debug_selector(|| "archived-section".to_string())
        .v_flex()
        .h_full()
        .min_h(px(0.))
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .v_flex()
                        .gap(px(4.))
                        .child(section_title(t!("settings.archived_title")))
                        .child(intro_line(t!("settings.archived_desc"), cx))
                        .children(st.settings.settings_notice.as_ref().map(|(ok, msg)| {
                            div()
                                .debug_selector(|| "archived-settings-notice".to_string())
                                .text_size(px(12.))
                                .text_color(if *ok {
                                    theme::success(cx)
                                } else {
                                    theme::danger(cx)
                                })
                                .child(format!("{} {msg}", if *ok { "✓" } else { "⚠" }))
                        })),
                )
                .child({
                    let s = store.clone();
                    let n = items.len();
                    div()
                        .id("archived-clear")
                        .debug_selector(|| "archived-clear".to_string())
                        .flex()
                        .flex_shrink_0()
                        .h(px(28.))
                        .items_center()
                        .px(px(10.))
                        .rounded(px(14.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(theme::danger(cx))
                        .hover(|s| s.bg(theme::dock(cx)))
                        .child(t!("settings.archived_clear_all"))
                        .on_click(move |_, _, cx| {
                            s.update(cx, |st, cx| st.ask_clear_archived(n, cx));
                        })
                }),
        );

    // 控制行:搜索(flex_1)+ 排序 + 项目筛选。三个控件统一默认档
    // (库内 input_h:Medium = 32px)+ h32 容器——Input small(24px)与
    // Select medium(32px)混排会肉眼可见参差;Select 自带 size_full,
    // 定尺寸容器仍是纪律
    let search = st.settings.archived_search.clone();
    col = col.child(
        div()
            .debug_selector(|| "archived-controls".to_string())
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .id("archived-search")
                    .flex_1()
                    .h(px(32.))
                    .line_height(gpui_kit::relative(1.4))
                    .children(search.as_ref().map(Input::new)),
            )
            .child(
                div()
                    .id("archived-order-box")
                    .w(px(150.))
                    .h(px(32.))
                    .line_height(gpui_kit::relative(1.4))
                    .children(st.settings.archived_order_select.as_ref().map(Select::new)),
            )
            .child(
                div()
                    .id("archived-project-box")
                    .w(px(200.))
                    .h(px(32.))
                    .line_height(gpui_kit::relative(1.4))
                    .children(
                        st.settings
                            .archived_project_select
                            .as_ref()
                            .map(Select::new),
                    ),
            ),
    );

    // 态分流:加载 / 空清单 / 零命中 = 余高内居中(不进滚动区);
    // 分组列表 = 内部滚动区
    let body: gpui_kit::AnyElement = if st.settings.archived_loading {
        centered_state("archived-loading", t!("settings.archived_loading"), cx).into_any_element()
    } else if items.is_empty() {
        centered_state("archived-empty", t!("settings.archived_empty"), cx).into_any_element()
    } else if groups.is_empty() {
        centered_state("archived-no-match", t!("settings.archived_no_match"), cx).into_any_element()
    } else {
        let mut list = div()
            .id("archived-list")
            .debug_selector(|| "archived-list".to_string())
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .v_flex()
            .gap(px(12.))
            .pr(px(2.));
        for (pkey, name, members) in groups {
            list = list.child(
                div()
                    .v_flex()
                    .gap(px(4.))
                    .child(archived_group_header(
                        store,
                        &pkey,
                        &name,
                        members.len(),
                        cx,
                    ))
                    .children(
                        members
                            .into_iter()
                            .map(|ix| archived_row(store, &items[ix], cx)),
                    ),
            );
        }
        list.into_any_element()
    };
    col.child(body)
}

/// 归档组头:文件夹 + 项目名 + 计数(全量,不受搜索影响)+ ⋯ 菜单
/// (删除此项目归档;组件库 Popover,库托管开合/定位)
pub(crate) fn archived_group_header(
    store: &Entity<AppStore>,
    pkey: &str,
    name: &str,
    count: usize,
    cx: &App,
) -> impl IntoElement {
    let grp_sel = sid("archived-grp", pkey);
    let more_sel = sid("archived-more", pkey);
    let pop_id = sid("archived-pop", pkey);
    div()
        .id(grp_sel.clone())
        .debug_selector(move || grp_sel.to_string())
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(32.))
        .px(px(2.))
        .child(fixed(LiumaIcon::FolderClose, 14.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(theme::label(cx))
                .child(name.to_string()),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme::caption(cx))
                .child(t!("settings.archived_count", n = count).into_owned()),
        )
        .child(div().flex_1())
        .child(
            Popover::new(pop_id)
                .appearance(false)
                .anchor(Anchor::TopRight)
                .trigger(PopTrigger(
                    div()
                        .id(more_sel.clone())
                        .debug_selector(move || more_sel.to_string())
                        .flex()
                        .size(px(20.))
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_color(theme::caption(cx))
                        .hover(|s| s.bg(theme::dock(cx)).text_color(theme::label(cx)))
                        .child(fixed(IconName::Ellipsis, 14.)),
                ))
                .content({
                    let s = store.clone();
                    let pkey = pkey.to_string();
                    let name = name.to_string();
                    move |_, _, cx| {
                        let pop = cx.entity();
                        archived_group_menu(&s, &pkey, &name, count, pop, cx).into_any_element()
                    }
                }),
        )
}

/// 归档组头菜单卡:「删除此项目归档」(危险色;先开确认弹窗再收菜单)
pub(crate) fn archived_group_menu(
    store: &Entity<AppStore>,
    pkey: &str,
    name: &str,
    count: usize,
    pop: Entity<PopoverState>,
    cx: &App,
) -> impl IntoElement {
    let s = store.clone();
    let purge_sel = sid("archived-purge", pkey);
    let pkey_click = pkey.to_string();
    let name_click = name.to_string();
    div()
        .id("archived-group-menu-card")
        .debug_selector(|| "archived-group-menu-card".to_string())
        .v_flex()
        .w(px(180.))
        .gap(px(2.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::layer(cx))
        .p(px(4.))
        .shadow_md()
        .child(
            div()
                .id(purge_sel.clone())
                .debug_selector(move || purge_sel.to_string())
                .flex()
                .h(px(26.))
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::danger(cx))
                .hover(|st| st.bg(theme::dock(cx)))
                .child(fixed(LiumaIcon::Trash, 13.))
                .child(t!("settings.archived_purge_item"))
                .on_click(move |_, window, cx| {
                    // 先开模态再收菜单:dismiss 在前会在点击对完成前移除
                    // 自身(同工作区菜单的竞态规避)
                    let pkey = pkey_click.clone();
                    s.update(cx, |st, cx| {
                        st.ask_purge_project_archived(pkey, name_click.clone(), count, cx)
                    });
                    pop.update(cx, |state, cx| state.dismiss(window, cx));
                }),
        )
}

/// 归档行:标题(截断)+ 更新时间 + 垃圾桶 + 取消归档胶囊
pub(crate) fn archived_row(
    store: &Entity<AppStore>,
    item: &ArchivedSessionSummary,
    cx: &App,
) -> impl IntoElement {
    let title = item
        .projections
        .as_ref()
        .and_then(|p| p.values["title"].as_str())
        .unwrap_or(&item.session_id);
    let aid = item.archive_id.clone();
    let row_sel = sid("archived-row", &aid);
    let del_sel = sid("archived-del", &aid);
    let restore_sel = sid("archived-restore", &aid);
    let s_del = store.clone();
    let s_restore = store.clone();
    let title_del = title.to_string();
    let title_shared: gpui_kit::SharedString = title.to_string().into();
    div()
        .id(row_sel.clone())
        .debug_selector(move || row_sel.to_string())
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(40.))
        .px(px(10.))
        .rounded(px(8.))
        .hover(|s| s.bg(theme::layer(cx)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(13.))
                .text_color(theme::label(cx))
                .truncate()
                .child(title_shared),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(theme::caption(cx))
                .child(fmt_clock_md(item.updated_at as i64)),
        )
        .child(
            div()
                .id(del_sel.clone())
                .debug_selector(move || del_sel.to_string())
                .flex()
                .size(px(26.))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(theme::caption(cx))
                .hover(|s| s.bg(theme::dock(cx)).text_color(theme::danger(cx)))
                .tooltip(crate::shell::tip(t!("settings.archived_confirm_delete")))
                .child(fixed(LiumaIcon::Trash, 14.))
                .on_click(move |_, _, cx| {
                    let aid = aid.clone();
                    let title = title_del.clone();
                    s_del.update(cx, |st, cx| st.ask_delete_archived(aid, title, cx));
                }),
        )
        .child({
            let aid = item.archive_id.clone();
            div()
                .id(restore_sel.clone())
                .debug_selector(move || restore_sel.to_string())
                .flex()
                .flex_shrink_0()
                .h(px(24.))
                .items_center()
                .gap(px(4.))
                .px(px(10.))
                .rounded(px(12.))
                .border_1()
                .border_color(theme::border(cx))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::label_2(cx))
                .hover(|s| s.bg(theme::dock(cx)))
                .child(fixed(LiumaIcon::ArchiveRestore, 13.))
                .child(t!("settings.archived_unarchive"))
                .on_click(move |_, _, cx| {
                    let aid = aid.clone();
                    s_restore.update(cx, |st, cx| st.unarchive_archived(aid, cx));
                })
        })
}

/// 归档确认弹窗(删单条 / 删项目归档 / 全部删除三态共用;footer 取消 +
/// 红色结果钮,文案点名对象与后果)。store 经 with_window 桥打开
pub(crate) fn open_archived_confirm_dialog(
    store: &Entity<AppStore>,
    kind: ArchivedConfirmKind,
    title: std::borrow::Cow<'static, str>,
    desc: std::borrow::Cow<'static, str>,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui_kit::component::WindowExt as _;
    let s_confirm = store.clone();
    let confirm_sel = match &kind {
        ArchivedConfirmKind::Delete { .. } => "archived-del-confirm",
        ArchivedConfirmKind::PurgeProject { .. } => "archived-purge-confirm",
        ArchivedConfirmKind::ClearAll => "archived-clear-confirm",
    };
    window.open_dialog(cx, move |dialog, _, cx| {
        let s_confirm = s_confirm.clone();
        let kind = kind.clone();
        dialog
            .title(title.clone())
            .w(px(420.))
            .bg(theme::layer(cx))
            .content({
                // 嵌套闭包 move 持有自己的克隆:借用外层捕获变量不满足
                // content_builder 的 'static(Rc 装箱)
                let desc = desc.clone();
                move |content, _, cx| {
                    content.child(
                        div()
                            .debug_selector(|| "archived-confirm-card".to_string())
                            .child(caption_line(desc.clone(), cx)),
                    )
                }
            })
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("archived-confirm-cancel")
                            .debug_selector(|| "archived-confirm-cancel".to_string())
                            .flex()
                            .h(px(32.))
                            .items_center()
                            .px(px(14.))
                            .rounded(px(16.))
                            .border_1()
                            .border_color(theme::border(cx))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(theme::label_2(cx))
                            .hover(|s| s.bg(theme::dock(cx)))
                            .child(t!("common.cancel"))
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        div()
                            .id(confirm_sel)
                            .debug_selector(|| confirm_sel.to_string())
                            .flex()
                            .h(px(32.))
                            .items_center()
                            .px(px(14.))
                            .rounded(px(16.))
                            .border_1()
                            .border_color(theme::danger(cx))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(theme::danger(cx))
                            .hover(|s| s.bg(theme::dock(cx)))
                            .child(t!("settings.archived_confirm_delete"))
                            .on_click(move |_, window, cx| {
                                s_confirm.update(cx, |st, cx| {
                                    st.confirm_archived_action(kind.clone(), cx)
                                });
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

/// 归档区下拉构建(labels + 当前项 + Confirm 经闭包回写)。与
/// `build_pref_select` 同款,动作面由闭包注入(归档区两下拉动作各异,
/// 不入 PrefMenuKind 分派)
pub(crate) fn build_archived_select(
    options: Vec<(String, String)>,
    current: &str,
    window: &mut Window,
    cx: &mut Context<AppStore>,
    on_pick: impl Fn(&mut AppStore, String, &mut Context<AppStore>) + 'static,
) -> Entity<SelectState<Vec<gpui_kit::SharedString>>> {
    let labels: Vec<gpui_kit::SharedString> = options
        .iter()
        .map(|(_, l)| gpui_kit::SharedString::from(l.clone()))
        .collect();
    let index = options
        .iter()
        .position(|(id, _)| id == current)
        .map(|ix| IndexPath::default().row(ix));
    let state = cx.new(|cx| SelectState::new(labels, index, window, cx));
    cx.subscribe(
        &state,
        move |this, _s, event: &SelectEvent<Vec<gpui_kit::SharedString>>, cx| {
            if let SelectEvent::Confirm(Some(label)) = event {
                let label_s = label.to_string();
                if let Some((id, _)) = options.iter().find(|(_, l)| *l == label_s) {
                    on_pick(this, id.clone(), cx);
                }
            }
        },
    )
    .detach();
    state
}
