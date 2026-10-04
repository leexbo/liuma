//! TextView 适配层(gpui-kit 富文本;流式 markdown 主路径)。
//!
//! 静态内容走 [`tv_static`](keyed state 惰性建、每帧 `set_text` 幂等,
//! ≤4KiB 同步解析首帧精确高);流式内容走 [`TvStreamRegistry`]:渲染
//! **前**的 flush 阶段(store.update 上下文)按节点 key diff——前缀
//! 匹配 `push_str` 尾增量(后台增量块解析,未闭合围栏不劈裂),否则
//! `set_text` 全量回退;渲染闭包只取挂载,无副作用。state 跨折叠
//! 重建按 key 存活;会话切换整体 `clear`。
//!
//! 历史坑背景(0.5.1 弃用理由,0.6.1 已带修复与上游回归):批量创建
//! 异步丢文本、measured 高度塌陷——本批在 layout_tests 复刻验证。

use std::collections::HashMap;

use gpui_kit::component::text::{TextView, TextViewState, TextViewStyle};
use gpui_kit::{
    App, AppContext as _, Entity, IntoElement, ParentElement as _, SharedString, StyleRefinement,
    Styled, div, px, relative,
};

/// 正文排版(手调渲染器指标,双盘):
/// 正文 14px/1.75、标题同字号加粗、块距 8px、代码块 12px/主题底。
/// TextView 的正文字号/行高/颜色走继承,由外层包装承担;标题/间距/
/// 代码块经 [`TextViewStyle`] 调
fn styled_view(view: TextView) -> gpui_kit::AnyElement {
    div()
        .w_full()
        .text_size(px(14.))
        .line_height(relative(1.75))
        // 宽度只靠 w_full 传导(assistant_block 已显式确定宽,锚链完好:
        // 探针实测 asst-body = tv-body = col_w)。**不设 overflow_hidden、
        // 不设左右 padding**:真机「行尾字形被裁」(「现在还在」丢「在」、
        // 「稳定」丢「定」+ 全角逗号整字消失)的根因不在盒宽,而在折行
        // **定价**——GPUI 逐字符孤立量宽 vs 绘制按整行 shape 的偏差,
        // 机制与实测见 kits::theme::FONT_SANS。盒内 padding 只挪动断行
        // 位置、且列宽与裁剪盒同步收窄(余量恒为 0),兜不住漂移,反而
        // 让正文比 composer 窄一截、右缘不齐。
        //
        // 滚动条槽同理不在内层预留:外层列表容器已让位 SCROLLBAR_GUTTER
        // (悬浮滚动条在槽内,不在列上),内层再扣一份 = 右缘不齐。
        .relative()
        .child(view)
        .into_any_element()
}

fn view_style(cx: &App) -> TextViewStyle {
    TextViewStyle {
        // 块距 16(旧值 8 过密)
        paragraph_gap: gpui_kit::rems(1.),
        heading_base_font_size: px(14.),
        // 标题与正文同字号,仅粗细区分(多级字号混排显「大大小小」)
        heading_font_size: Some(std::sync::Arc::new(|_level: u8, base| base)),
        // 代码块字号 13;配色全部走 theme 派生默认(不另行改色)
        code_block: StyleRefinement::default().text_size(px(12.)),
        is_dark: crate::kits::theme::is_dark(cx),
        ..Default::default()
    }
}

/// 静态 markdown 挂载(keyed 便捷构造;id 需调用点稳定)
pub(crate) fn tv_static(
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
    cx: &App,
) -> gpui_kit::AnyElement {
    styled_view(TextView::markdown(id, text).style(view_style(cx)))
}

/// 卡内代码块(单围栏)。走库 **Base** `TextView` 而非 component 门面,
/// 门面给不了的两件都在这儿:① 代码块高亮回调只挂在 Base 上
/// (`code_block_highlighter`);② 拖选 —— 手绘 `SelectableText` 的
/// `document_order` 只能自定一个值,而聊天正文的 order 由 TextView 全局
/// 自增,插不进那个序,跨域拖选会把整段会话圈进来;TextView 自带选区
/// participant 身份,order 与正文同源。代价:围栏行随之不再显示(渲染
/// 成代码块),复制的仍是纯源码。
pub(crate) fn tv_code_block(
    id: impl Into<gpui_kit::ElementId>,
    lang: &str,
    code: &str,
    cx: &App,
) -> gpui_kit::AnyElement {
    let text = format!("```{lang}\n{code}\n```");
    div()
        .w_full()
        .min_w(px(0.))
        .cursor_text()
        .font_family("Menlo")
        .text_size(px(13.))
        .line_height(relative(1.5))
        .child(
            gpui_kit::base::text::TextView::markdown(id, text)
                .style(code_view_style(cx))
                .code_block_highlighter(crate::kits::highlight::code_block_highlighter(
                    crate::kits::theme::is_dark(cx),
                )),
        )
        .into_any_element()
}

/// 卡内代码块排版:标题/段落间距都不参与,要压的只有两件 —— 代码底色
/// (卡片本身已是代码底,库默认再叠一层 `muted` = 双底)与代码字号行高。
fn code_view_style(cx: &App) -> gpui_kit::base::text::TextViewStyle {
    gpui_kit::base::text::TextViewStyle::default()
        .with_foreground(crate::kits::theme::label_2(cx).into())
        .with_code_background(crate::kits::theme::TRANSPARENT.into())
        .with_code_block(
            StyleRefinement::default()
                .text_size(px(13.))
                .line_height(relative(1.5)),
        )
        .with_dark(crate::kits::theme::is_dark(cx))
}

/// 驱动结果
pub(crate) enum DriveOutcome {
    /// 新建视图(>4KiB 历史的首轮解析是异步的,落地晚于挂载)
    Created(Entity<TextViewState>),
    /// 既有视图文本变化(push_str 增量或 set_text 回退)
    Updated,
    /// 无变化
    None,
}

/// 流式驱动注册表(挂 ChatStore;渲染前 flush 驱动,渲染闭包只读)
#[derive(Default)]
pub(crate) struct TvStreamRegistry {
    /// 节点 key → (state, 已同步文本记账, 正文版本号)。TextViewState 无
    /// text getter,记账由本层维护。ver 来自 `ChatNode::Assistant.text_ver`,
    /// 未变节点 O(1) 短路(原稳态帧逐节点全文 memcmp,大会话每帧 O(全部
    /// 正文字节))。
    ///
    /// 记账是 `Option`:**定稿节点置 `None` = 释放这份重复全文**。前缀记账
    /// 只在流式增长期需要,而定稿正文与 `ChatNode::Assistant.text` 逐字节
    /// 相同——每节点存两份是长会话内存的直接翻倍;此后若定稿正文再变
    /// (回退/重写)走 `set_text` 全量,语义不变
    map: HashMap<String, (Entity<TextViewState>, Option<String>, u32)>,
}

impl TvStreamRegistry {
    /// 渲染前 flush 驱动:版本号 O(1) 判未变;前缀匹配 → push_str 增量;
    /// 文本漂移(回退/重写)→ set_text 全量;新 key → 建状态。幂等。
    /// 返回结果类别(新建视图须挂观察者:异步解析落地会改变高度,
    /// 外层虚拟化列表的行高缓存不会自愈)
    pub(crate) fn drive(
        &mut self,
        key: &str,
        text: &str,
        text_ver: u32,
        streaming: bool,
        cx: &mut App,
    ) -> DriveOutcome {
        match self.map.get_mut(key) {
            Some((state, last, last_ver)) => {
                // 稳态帧:版本号未动 = 文本未变(版本号只在 text 突变点
                // 自增,O(1);len 判等不可靠,等长漂移会漏)
                if *last_ver == text_ver {
                    // 但「停止流式」不改变版本号:定稿释放仍须在此发生,
                    // 否则重复全文会一直挂到下次文本突变(可能永不到来)
                    if !streaming && last.is_some() {
                        *last = None;
                    }
                    return DriveOutcome::None;
                }
                match last.as_deref() {
                    Some(prev) if text.len() > prev.len() && text.starts_with(prev) => {
                        let delta = text[prev.len()..].to_string();
                        state.update(cx, |s, cx| s.push_str(&delta, cx));
                    }
                    // 无记账(已定稿释放)或文本漂移 → 全量重设
                    _ => state.update(cx, |s, cx| s.set_text(text, cx)),
                }
                *last_ver = text_ver;
                // 流式期维持记账;定稿即释放(重复全文不再常驻)
                *last = streaming.then(|| text.to_string());
                DriveOutcome::Updated
            }
            None => {
                let state = cx.new(|cx| TextViewState::markdown(text, cx));
                self.map.insert(
                    key.to_string(),
                    (state.clone(), streaming.then(|| text.to_string()), text_ver),
                );
                DriveOutcome::Created(state)
            }
        }
    }

    /// 逐出不在 `keep` 内的条目(节点被折叠/裁窗/换会话后,其视图与记账
    /// 都无消费者)。返回逐出条数(测试与探针用)
    pub(crate) fn retain(&mut self, keep: impl Fn(&str) -> bool) -> usize {
        let before = self.map.len();
        self.map.retain(|k, _| keep(k.as_str()));
        before - self.map.len()
    }

    /// 带组合的挂载(state 缺失 = flush 未及,兜底 keyed 静态;插件等
    /// TextView 级修饰经 compose,样式统一在适配层收口)
    pub(crate) fn view_composed(
        &self,
        key: &str,
        fallback_text: &str,
        compose: impl FnOnce(TextView) -> TextView,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let view = match self.map.get(key) {
            Some((state, _, _)) => compose(TextView::new(state)),
            None => compose(TextView::markdown(
                SharedString::from(key.to_string()),
                fallback_text,
            )),
        };
        styled_view(view.style(view_style(cx)))
    }

    /// 会话切换清理(state 随旧会话焚毁,重开重解析一次)
    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }

    /// 条目数(逐出断言 / 探针)
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// 重复记账字节合计(探针:定稿节点释放后应趋近「仅流式中的那几条」)
    pub(crate) fn accounting_bytes(&self) -> usize {
        self.map
            .values()
            .map(|(_, last, _)| last.as_ref().map_or(0, |s| s.len()))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{Root, StyledExt as _};
    use gpui_kit::{
        InteractiveElement as _, IntoElement, ListAlignment, ListState, Render,
        StatefulInteractiveElement as _, Styled, TestAppContext, VisualTestContext, Window, div,
        px,
    };

    fn init(cx: &mut TestAppContext) {
        cx.update(|app| {
            gpui_kit::component::init(app);
            crate::kits::theme::init(app);
        });
    }

    /// 正文族交付链锁(机制见 kits::theme::FONT_SANS):主题族必须经
    /// gpui-component `Root` 落到窗口文本样式栈——正文折行取的就是栈上的
    /// 族(InlineFlow 在 request_layout 读 `window.text_style()`)。只改
    /// theme 而没落到栈 = 修复不生效,真机照旧「行尾被裁」
    #[gpui_kit::test]
    fn chat_body_text_style_carries_theme_font_family(cx: &mut TestAppContext) {
        use std::sync::{Arc, Mutex};
        init(cx);
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        struct Probe(Arc<Mutex<Option<String>>>);
        impl Render for Probe {
            fn render(
                &mut self,
                _: &mut Window,
                _cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                let slot = self.0.clone();
                div().size_full().child(
                    // canvas 首闭包 = prepaint 期(祖先文本样式已在栈上),
                    // 与正文 request_layout 读的是同一个栈
                    gpui_kit::canvas(
                        move |_bounds, window, _cx| {
                            *slot.lock().unwrap_or_else(|p| p.into_inner()) =
                                Some(window.text_style().font_family.to_string());
                        },
                        |_, _, _, _| {},
                    )
                    .size_full(),
                )
            }
        }
        let slot = seen.clone();
        let (_root, wcx) = cx.add_window_view(|window, cx| {
            let v = cx.new(move |_| Probe(slot));
            Root::new(v, window, cx)
        });
        wcx.refresh().expect("刷新失败");
        let family = seen.lock().unwrap_or_else(|p| p.into_inner()).take();
        assert_eq!(
            family.as_deref(),
            Some(crate::kits::theme::FONT_SANS),
            "Root 未把主题族推到文本样式栈——正文折行拿到的族不对"
        );
    }

    /// 批量创建丢文本验证(0.5.1 历史坑 ①):200 条 keyed TextView 一次
    /// 全部在场,每条高度非零(丢文本 = 塌 0)。注意必须走 cx.refresh()
    /// 自然渲染——裸 window.draw 在 request_layout 期无 view 上下文,
    /// use_keyed_state 会 panic(既有记录)
    #[gpui_kit::test]
    fn tv_batch_creation_keeps_all_text(cx: &mut TestAppContext) {
        init(cx);
        struct Batch;
        impl Render for Batch {
            fn render(
                &mut self,
                _: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                let items: Vec<_> = (0..200)
                    .map(|ix| {
                        div()
                            .id(SharedString::from(format!("tvb-{ix}")))
                            .debug_selector(move || format!("tv-batch-{ix}"))
                            .w(px(400.))
                            .child(tv_static(
                                SharedString::from(format!("tv-{ix}")),
                                &format!("第 {ix} 条:正文段落,包含 **加粗** 与 `code`。\n"),
                                cx,
                            ))
                    })
                    .collect();
                div()
                    .id("tv-batch-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .v_flex()
                    .children(items)
            }
        }
        let (_view, cx) = cx.add_window_view(|window, cx| {
            let v = cx.new(|_| Batch);
            Root::new(v, window, cx)
        });
        cx.refresh().expect("刷新失败");
        cx.run_until_parked();
        let mut zero = 0usize;
        for ix in 0..200 {
            let sel: &'static str = Box::leak(format!("tv-batch-{ix}").into_boxed_str());
            let h = cx
                .debug_bounds(sel)
                .map(|b| f32::from(b.size.height))
                .unwrap_or(0.);
            if h <= 0. {
                zero += 1;
            }
        }
        assert_eq!(zero, 0, "200 条批量创建不应有任何一条丢文本塌 0");
    }

    /// 高度塌陷/滚动抖动验证(0.5.1 历史坑 ②):虚拟化 list 里 40 条
    /// markdown,滚动前后内容总高稳定(delta < 2px;离屏块零高会令
    /// 总高缩水)。measure_all + 同步小替换是上游修复面
    #[gpui_kit::test]
    fn tv_list_total_height_stable_while_scrolling(cx: &mut TestAppContext) {
        init(cx);
        struct ListView {
            list: ListState,
        }
        impl Render for ListView {
            fn render(
                &mut self,
                _: &mut Window,
                _cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                let state = self.list.clone();
                div().size_full().child(gpui_kit::list(state, |ix, _window, cx| {
                    div()
                        .id(SharedString::from(format!("tvl-{ix}")))
                        .debug_selector(move || format!("tv-list-{ix}"))
                        .w(px(400.))
                        .child(tv_static(
                            SharedString::from(format!("l-{ix}")),
                            &format!(
                                "## 标题 {ix}\n\n段落一行,包含列表:\n\n- 项 A\n- 项 B\n\n```rust\nfn f{ix}() {{}}\n```\n"
                            ),
                            cx,
                        ))
                        .into_any_element()
                }))
            }
        }
        let list = ListState::new(40, ListAlignment::Top, px(600.));
        let list_in_view = list.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let v = cx.new(move |_| ListView { list: list_in_view });
            Root::new(v, window, cx)
        });
        let content_h = |cx: &mut VisualTestContext| {
            cx.refresh().expect("刷新失败");
            cx.run_until_parked();
            list.max_offset_for_scrollbar().y
        };
        let h1 = f32::from(content_h(cx));
        assert!(h1 > 0., "初始应有内容高度");
        list.scroll_by(px(600.));
        let h2 = f32::from(content_h(cx));
        assert!(
            (h2 - h1).abs() < 2.,
            "滚动后内容总高应稳定(实测 {h1} → {h2})"
        );
    }

    /// 列表字号一致性探针:同 6 项列表分别包在 13px(正文)与 16px
    /// wrapper 里,高度必须不同——相等即列表渲染钉死在 rem 默认字号、
    /// 不随 wrapper 继承(真机截图曾现此症:列表项比正文段落大一号)
    #[gpui_kit::test]
    fn tv_list_font_size_follows_wrapper(cx: &mut TestAppContext) {
        init(cx);
        const LIST: &str =
            "- 项目甲测试\n- 项目乙测试\n- 项目丙测试\n- 项目丁测试\n- 项目戊测试\n- 项目己测试\n";
        struct Probe;
        impl Render for Probe {
            fn render(
                &mut self,
                _: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                div()
                    .w(px(400.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .debug_selector(|| "tv-probe-13".to_string())
                            .child(tv_static("probe-13", LIST, cx)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "tv-probe-16".to_string())
                            .text_size(px(16.))
                            .line_height(relative(1.75))
                            .child(TextView::markdown("probe-16", LIST).style(view_style(cx))),
                    )
            }
        }
        let (_view, cx) = cx.add_window_view(|window, cx| {
            let v = cx.new(|_| Probe);
            Root::new(v, window, cx)
        });
        cx.refresh().expect("刷新失败");
        cx.run_until_parked();
        let mut h = |sel: &'static str| {
            cx.debug_bounds(sel)
                .map(|b| f32::from(b.size.height))
                .unwrap_or(0.)
        };
        let h13 = h("tv-probe-13");
        let h16 = h("tv-probe-16");
        assert!(h13 > 0. && h16 > 0., "两块都应有高度({h13} / {h16})");
        assert!(
            (h16 - h13).abs() > 12.,
            "列表字号应随 wrapper:13px 高 {h13} vs 16px 高 {h16}(差值过小 = 列表钉死默认字号)"
        );
    }

    /// 行内代码 chip 行高锁:chip 行必须随正文字号缩放(gpui-base
    /// 0.6.1 曾把 chip 行高钉死在窗口根行高——13px 正文的 chip 行恒
    /// 26px,列表里 chip 密集即显「文字异常大」;0.6.4 起跟随)。
    #[gpui_kit::test]
    fn tv_inline_code_line_height_follows_body(cx: &mut TestAppContext) {
        init(cx);
        const CHIP: &str = "前缀 `chip` 后缀。";
        struct Probe;
        impl Render for Probe {
            fn render(
                &mut self,
                _: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                div()
                    .w(px(430.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .debug_selector(|| "tv-chip-13".to_string())
                            .text_size(px(13.))
                            .line_height(relative(1.75))
                            .child(TextView::markdown("chip-13", CHIP).style(view_style(cx))),
                    )
                    .child(
                        div()
                            .debug_selector(|| "tv-chip-16".to_string())
                            .text_size(px(16.))
                            .line_height(relative(1.75))
                            .child(TextView::markdown("chip-16", CHIP).style(view_style(cx))),
                    )
            }
        }
        let (_view, cx) = cx.add_window_view(|window, cx| {
            let v = cx.new(|_| Probe);
            Root::new(v, window, cx)
        });
        cx.refresh().expect("刷新失败");
        cx.run_until_parked();
        let h13 = cx
            .debug_bounds("tv-chip-13")
            .map(|b| f32::from(b.size.height))
            .unwrap_or(0.);
        let h16 = cx
            .debug_bounds("tv-chip-16")
            .map(|b| f32::from(b.size.height))
            .unwrap_or(0.);
        assert!(h13 > 0. && h16 > 0., "两块都应有高度({h13} / {h16})");
        // chip 行随正文缩放:16px 下的行高须显著大于 13px(旧缺陷:两者同为 26)
        assert!(
            h16 - h13 >= 4.,
            "chip 行高应随正文字号:{h13} → {h16}(未缩放 = 上游钉死回归)"
        );
    }

    /// 流式增量 == 一次全量(行为等价 + 不丢块):分 5 段 push_str 喂的
    /// TextView 与全量构造的 TextView 高度一致(±2px),且逐段单调
    #[gpui_kit::test]
    fn tv_stream_push_str_matches_full(cx: &mut TestAppContext) {
        init(cx);
        struct StreamView {
            reg: TvStreamRegistry,
        }
        impl Render for StreamView {
            fn render(
                &mut self,
                _: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                div()
                    .id("tv-stream-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .v_flex()
                    .child(
                        div()
                            .id("tv-stream-inc")
                            .debug_selector(|| "tv-stream-inc".to_string())
                            .w(px(400.))
                            .child(self.reg.view_composed("k", "", |v| v, cx)),
                    )
                    .child(
                        div()
                            .id("tv-stream-full")
                            .debug_selector(|| "tv-stream-full".to_string())
                            .w(px(400.))
                            .child(tv_static("full", Self::TARGET, cx)),
                    )
            }
        }
        impl StreamView {
            const TARGET: &'static str =
                "开篇段落。\n\n- 列表项一\n- 列表项二\n\n```rust\nfn a() {}\n```\n\n收尾段落。\n";
        }
        let inner = std::rc::Rc::new(std::cell::RefCell::new(None::<Entity<StreamView>>));
        let cell = inner.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let v = cx.new(|_| StreamView {
                reg: TvStreamRegistry::default(),
            });
            *cell.borrow_mut() = Some(v.clone());
            Root::new(v, window, cx)
        });
        let view = inner.borrow().clone().expect("内层实体应在场");
        let target = StreamView::TARGET;
        // 切点取字符边界(多字节文本不能按裸字节切)
        let boundaries: Vec<usize> = target
            .char_indices()
            .map(|(i, _)| i)
            .chain([target.len()])
            .collect();
        let pick = |want: usize| *boundaries.iter().find(|b| **b >= want).expect("边界存在");
        let mut prev_h = 0.0f32;
        for (round, want) in [8usize, 20, 45, 70, target.len()].into_iter().enumerate() {
            let part = target[..pick(want)].to_string();
            view.update(cx, |v, cx| {
                v.reg.drive("k", &part, round as u32 + 1, true, cx)
            });
            cx.refresh().expect("刷新失败");
            cx.run_until_parked();
            let h = cx
                .debug_bounds("tv-stream-inc")
                .map(|b| f32::from(b.size.height))
                .unwrap_or(0.);
            assert!(h >= prev_h, "流式高度应单调不减({prev_h} → {h})");
            prev_h = h;
        }
        cx.refresh().expect("刷新失败");
        cx.run_until_parked();
        let full_h = f32::from(
            cx.debug_bounds("tv-stream-full")
                .map(|b| b.size.height)
                .unwrap_or(px(0.)),
        );
        assert!(
            (prev_h - full_h).abs() < 2.,
            "增量喂成高度应与全量一致(增量 {prev_h} vs 全量 {full_h})"
        );
    }

    /// 未闭合代码块的流式重解析不崩(库侧 #3238 的复现尝试)。
    ///
    /// 机制:tree-sitter 的读回调曾按 `&str` 切片,当旧树/included range
    /// 让它从**多字节字符中间**请求 offset 时 panic 落在 `extern "C"`
    /// 回调里 → abort 整个进程(非 unwind)。上方 `view_style()` 的
    /// 「围栏未闭合不劈裂」策略意味着流式期间每次 `push_str` 都会把
    /// 整个未闭合围栏重新喂给高亮器,旧树与新增多字节内容交替,正是
    /// stale included range 的温床。本用例逐字追加含中文的未闭合
    /// rust 围栏,末段才闭合。
    ///
    /// 实测记录:本用例在 0.6.6 与 0.7.0 上**均通过**——项目这条路径
    /// 无法复现 #3238,故它是一条覆盖锁而非该修的反向锚,勿据它宣称
    /// 锁住了 #3238。
    #[gpui_kit::test]
    fn tv_unclosed_cjk_fence_streams_without_abort(cx: &mut TestAppContext) {
        init(cx);
        struct FenceView {
            reg: TvStreamRegistry,
        }
        impl Render for FenceView {
            fn render(
                &mut self,
                _: &mut Window,
                cx: &mut gpui_kit::Context<Self>,
            ) -> impl IntoElement {
                div()
                    .id("tv-fence-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .v_flex()
                    .child(
                        div()
                            .id("tv-fence-inc")
                            .debug_selector(|| "tv-fence-inc".to_string())
                            .w(px(400.))
                            .child(self.reg.view_composed("f", "", |v| v, cx)),
                    )
            }
        }
        let inner = std::rc::Rc::new(std::cell::RefCell::new(None::<Entity<FenceView>>));
        let cell = inner.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let v = cx.new(|_| FenceView {
                reg: TvStreamRegistry::default(),
            });
            *cell.borrow_mut() = Some(v.clone());
            Root::new(v, window, cx)
        });
        let view = inner.borrow().clone().expect("内层实体应在场");
        // 未闭合围栏 + 中文注释/字符串:每个字符一次 push,制造最多轮次
        // 的 stale-tree 重解析;末段才补上闭合围栏
        let head = "说明:\n\n```rust\n// 时区处理\nfn f() {\n";
        let body =
            "    let s = \"北京时间\";\n    // 逐字追加中文注释,令字节边界不断落在多字节字符邻域\n";
        let tail = "}\n```\n\n收尾。\n";
        let full = format!("{head}{body}{tail}");
        let mut sent = String::new();
        for (ix, ch) in head
            .chars()
            .chain(body.chars())
            .chain(tail.chars())
            .enumerate()
        {
            sent.push(ch);
            // text_ver 从 1 起:0 在 drive 里是「初始记账」语义,逐字喂用递增序
            let ver = ix as u32 + 1;
            let part = sent.clone();
            view.update(cx, |v, cx| v.reg.drive("f", &part, ver, true, cx));
            cx.refresh().expect("刷新失败");
            cx.run_until_parked();
        }
        assert_eq!(sent, full, "逐字喂入的文本应与目标一致");
        let h = f32::from(
            cx.debug_bounds("tv-fence-inc")
                .map(|b| b.size.height)
                .unwrap_or(px(0.)),
        );
        assert!(h > 0., "闭合后视图应有高度(未崩且渲染完成),实得 {h}");
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn init(cx: &mut TestAppContext) {
        cx.update(|app| {
            gpui_kit::component::init(app);
            crate::kits::theme::init(app);
        });
    }

    /// 定稿释放重复记账:流式期维持前缀记账,定稿即释放(正文与
    /// `ChatNode::Assistant.text` 逐字节相同,每节点存两份 = 长会话直接
    /// 翻倍);释放后若定稿正文再变,走 set_text 全量,语义不变
    #[gpui_kit::test]
    fn finalized_node_releases_text_accounting(cx: &mut TestAppContext) {
        init(cx);
        let reg = cx.new(|_| TvStreamRegistry::default());
        reg.update(cx, |r, cx| {
            r.drive("a:1", "第一段", 1, true, cx);
            assert_eq!(r.accounting_bytes(), "第一段".len(), "流式期应有记账");
            r.drive("a:1", "第一段第二段", 2, true, cx);
            assert_eq!(r.accounting_bytes(), "第一段第二段".len());
            // 定稿:记账释放
            r.drive("a:1", "第一段第二段", 2, false, cx);
            assert_eq!(r.accounting_bytes(), 0, "定稿后不应保留重复全文");
            assert_eq!(r.len(), 1, "条目本身保留(视图仍在用)");
            // 定稿后再变(回退/重写):无记账 → 全量重设,不 panic 不串文
            r.drive("a:1", "改写后的正文", 3, false, cx);
            assert_eq!(r.accounting_bytes(), 0);
        });
    }

    /// 逐出:不在活节点集内的条目连同视图一起释放(此前只增不减)
    #[gpui_kit::test]
    fn retain_drops_absent_nodes(cx: &mut TestAppContext) {
        init(cx);
        let reg = cx.new(|_| TvStreamRegistry::default());
        reg.update(cx, |r, cx| {
            r.drive("a:1", "一", 1, true, cx);
            r.drive("a:2", "二", 1, true, cx);
            r.drive("a:3", "三", 1, false, cx);
            assert_eq!(r.len(), 3);
            r.retain(|k| k != "a:2");
            assert_eq!(r.len(), 2);
            // 保留者仍是同一视图(记账与状态不重建)
            assert!(matches!(
                r.drive("a:1", "一", 1, true, cx),
                DriveOutcome::None
            ));
            r.retain(|_| false);
            assert_eq!(r.len(), 0);
        });
    }

    /// TextView 创建/销毁 churn 复现(env 门控,默认跳过):滚动全程
    /// ~1700 个 TextView 建了又逐出,RSS 单调涨(真机 release 实测滚到
    /// 头 700MB)。本实验不经列表,直接隔离 TextViewState::markdown
    /// 的建/毁循环——若 RSS 仍线性涨,泄漏在 merman/gpui 文本层;
    /// 平了则在列表/渲染层。跑法:
    ///   LIUMA_TV_CHURN=1 cargo test -p liuma-desktop --release tv_churn -- --nocapture
    #[gpui_kit::test]
    fn tv_churn_rss_curve(cx: &mut TestAppContext) {
        if std::env::var("LIUMA_TV_CHURN").is_err() {
            eprintln!("[tc] 跳过(未设 LIUMA_TV_CHURN)");
            return;
        }
        init(cx);
        #[cfg(target_os = "macos")]
        fn rss_mb() -> f64 {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .expect("ps");
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<u64>()
                .map(|kb| kb as f64 / 1024.0)
                .unwrap_or(0.)
        }
        #[cfg(not(target_os = "macos"))]
        fn rss_mb() -> f64 {
            0.
        }
        // 每段 ~4KB markdown(标题+代码块+列表),2000 段互不相同
        let rounds = 2000usize;
        let report_every = 400usize;
        let mut held: Option<Entity<TextViewState>> = None;
        let t0 = std::time::Instant::now();
        eprintln!("[tc] 基线 RSS = {:.0} MB", rss_mb());
        for i in 0..rounds {
            let text = format!(
                "# 段落 {i}\n\n{}正文叙述,{}带 `code` 与**强调**。\n\n```rust\nfn step_{i}() -> u32 {{ {i} }}\n```\n\n- 项一\n- 项二\n",
                "较长的叙述文本,".repeat(60),
                "变化词,".repeat(20),
            );
            let state = cx.new(|cx| TextViewState::markdown(&text, cx));
            held = Some(state); // 覆盖上一轮 = 旧实体销毁
            if (i + 1) % report_every == 0 {
                cx.run_until_parked(); // 推进异步解析落地
                eprintln!("[tc] {:>5} 段后 RSS = {:.0} MB", i + 1, rss_mb());
            }
        }
        drop(held);
        cx.run_until_parked();
        eprintln!(
            "[tc] 全部释放后 RSS = {:.0} MB({:.1}s)",
            rss_mb(),
            t0.elapsed().as_secs_f64()
        );
    }
}
