//! 窗口文本选择行为锁:微抖动单击不得留下「复制模式」(零字符活选择
//! 由 SelectionCollapseGuard 在 mouse-up 后清除),真实拖选保留。
//! 模式 = Root + 可选 TextView + guard 同窗,simulate_event 逐事件推进

use super::*;
use gpui_kit::component::Root;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::{
    AppContext, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext, point,
};

/// 探针视图:大块可选文本 + 折叠守卫(与 WorkspaceView 同挂法)
struct Probe {
    state: Option<Entity<TextViewState>>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = match &self.state {
            Some(s) => TextView::new(s).into_any_element(),
            None => div().into_any_element(),
        };
        div()
            .size_full()
            .p(px(8.))
            .child(text)
            // 与 workspace 根同款挂法(零尺寸,窗口级 mouse-up 监听)
            .child(SelectionCollapseGuard)
    }
}

fn mount<'a>(cx: &'a mut TestAppContext, body: &str) -> &'a mut VisualTestContext {
    cx.update(|app| {
        gpui_kit::component::init(app);
        theme::init(app);
    });
    let (_view, wcx) = cx.add_window_view(|window, cx| {
        let state = cx.new(|cx| TextViewState::markdown(body, cx));
        Root::new(cx.new(|_| Probe { state: Some(state) }), window, cx)
    });
    wcx.run_until_parked();
    wcx
}

fn down(wcx: &mut VisualTestContext, x: f32, y: f32) {
    wcx.simulate_event(MouseDownEvent {
        position: point(px(x), px(y)),
        modifiers: Default::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
}

fn drag(wcx: &mut VisualTestContext, x: f32, y: f32) {
    wcx.simulate_event(MouseMoveEvent {
        position: point(px(x), px(y)),
        pressed_button: Some(MouseButton::Left),
        modifiers: Default::default(),
    });
}

fn up(wcx: &mut VisualTestContext, x: f32, y: f32) {
    wcx.simulate_event(MouseUpEvent {
        position: point(px(x), px(y)),
        modifiers: Default::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
}

/// 回归锁(修复前红):单击的 1~2px 自然抖动不得激活选择/复制面。
/// 此前 move 把 cursor 推离 anchor → 零字符活选择,右键 Copy/⌘C
/// 随即生效(= 点开轨迹详情即进入复制模式)
#[gpui_kit::test]
fn micro_jiggle_click_leaves_no_selection(cx: &mut TestAppContext) {
    let wcx = mount(
        cx,
        &format!("{}\n", "可选正文一行接一行,足够宽。".repeat(40)),
    );
    down(wcx, 120., 20.);
    drag(wcx, 122., 21.); // 真实单击的微抖动
    up(wcx, 122., 21.);
    wcx.update(|window, cx| {
        assert!(
            !gpui_kit::base::TextSelection::has_selection(window, cx),
            "微抖动单击不得留下活选择(复制模式)"
        );
    });
}

/// 守卫不得误伤:跨文本真实拖选保留且可复制
#[gpui_kit::test]
fn drag_across_text_still_selects(cx: &mut TestAppContext) {
    let wcx = mount(
        cx,
        &format!("{}\n", "可选正文一行接一行,足够宽。".repeat(40)),
    );
    down(wcx, 60., 20.);
    drag(wcx, 200., 20.);
    up(wcx, 200., 20.);
    wcx.update(|window, cx| {
        assert!(
            gpui_kit::base::TextSelection::has_selection(window, cx),
            "真实拖选必须保留"
        );
        assert!(
            !gpui_kit::base::TextSelection::selected_text(window, cx)
                .trim()
                .is_empty(),
            "拖选的复制内容不得为空"
        );
    });
}

/// 文本区域外的抖动单击(命中域哨兵/空白)同样不留活选择
#[gpui_kit::test]
fn jiggle_on_chrome_leaves_no_selection(cx: &mut TestAppContext) {
    let wcx = mount(cx, "只有一行正文,窗口下方大片空白");
    down(wcx, 120., 300.); // 正文块之外
    drag(wcx, 122., 301.);
    up(wcx, 122., 301.);
    wcx.update(|window, cx| {
        assert!(
            !gpui_kit::base::TextSelection::has_selection(window, cx),
            "空白处抖动单击不得留下活选择"
        );
    });
}

/// 词选(click_count = 2)豁免:手势判定不得清掉双击选择。
/// 旧版按 selected_text 判空曾把「投影未就绪的真选择」误清(真机
/// 表现为无法复制);手势版按位移与 click_count 判定,内容无关
#[gpui_kit::test]
fn double_click_word_selection_survives(cx: &mut TestAppContext) {
    let wcx = mount(
        cx,
        &format!("{}\n", "可选正文一行接一行,足够宽。".repeat(40)),
    );
    wcx.simulate_event(MouseDownEvent {
        position: point(px(120.), px(20.)),
        modifiers: Default::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    wcx.simulate_event(MouseUpEvent {
        position: point(px(120.), px(20.)),
        modifiers: Default::default(),
        button: MouseButton::Left,
        click_count: 2,
    });
    wcx.update(|window, cx| {
        assert!(
            gpui_kit::base::TextSelection::has_selection(window, cx),
            "双击词选必须保留"
        );
    });
}

/// 阈值边界:3px 位移仍按单击清(系统 click/drag 分界之内);
/// 覆盖端值防阈值方向写反
#[gpui_kit::test]
fn tiny_travel_below_threshold_still_collapses(cx: &mut TestAppContext) {
    let wcx = mount(
        cx,
        &format!("{}\n", "可选正文一行接一行,足够宽。".repeat(40)),
    );
    down(wcx, 120., 20.);
    drag(wcx, 123., 22.); // ~3.6px < 5px 阈值
    up(wcx, 123., 22.);
    wcx.update(|window, cx| {
        assert!(
            !gpui_kit::base::TextSelection::has_selection(window, cx),
            "阈值内位移按单击清"
        );
    });
}

/// 大文本(>4KiB 异步解析)的 TextView 拖选——上游隔离实验:
/// 复现 = gpui-kit 异步路径 bug(报 issue);不复现 = liuma 装配问题
#[gpui_kit::test]
fn large_markdown_textview_drag_selects(cx: &mut TestAppContext) {
    let big = format!("{}\n", "异步解析的大段正文,足够选中。".repeat(600)); // ~12KB
    let wcx = mount(cx, &big);
    for _ in 0..5 {
        wcx.refresh().expect("刷新失败");
        wcx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    down(wcx, 60., 20.);
    drag(wcx, 200., 20.);
    up(wcx, 200., 20.);
    wcx.run_until_parked();
    wcx.update(|window, cx| {
        let has = gpui_kit::base::TextSelection::has_selection(window, cx);
        let text = gpui_kit::base::TextSelection::selected_text(window, cx);
        eprintln!(
            "[dbg-large] has={has} text={:?}",
            &text[..text.len().min(40)]
        );
        assert!(has, "大文本拖选必须产生选择");
        assert!(!text.trim().is_empty());
    });
}

/// 复刻轨迹详情装配:Scrollable(overflow_y_scrollbar) + markdown_tv
/// keyed TextView + 大文本——锁定「无法复制」的组合因子
#[gpui_kit::test]
fn scrollable_tv_static_large_text(cx: &mut TestAppContext) {
    use gpui_kit::component::scroll::ScrollableElement as _;
    let big = format!("{}\n", "滚动容器内的大段正文,足够选中。".repeat(600));
    cx.update(|app| {
        gpui_kit::component::init(app);
        theme::init(app);
    });
    struct P2;
    impl Render for P2 {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .relative()
                .p(px(8.))
                .child(crate::kits::markdown_tv::tv_static(
                    "sel-probe-large",
                    &"多块大文本一行。\n".to_string().repeat(400),
                    cx,
                ))
                .overflow_y_scrollbar()
                // 复刻右面板的域尾哨兵(绝对铺满 + 注册参与者)
                .child(div().absolute().size_full().child(SelectionDomainSink::new(
                    "sel-probe-sink",
                    crate::kits::selection_order::PANEL_TAIL_ORDER,
                )))
                .child(SelectionCollapseGuard)
        }
    }
    let (_v, wcx) = cx.add_window_view(|window, cx| Root::new(cx.new(|_| P2), window, cx));
    wcx.run_until_parked();
    let _ = big;
    for _ in 0..5 {
        wcx.refresh().expect("刷新失败");
        wcx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    down(wcx, 60., 20.);
    drag(wcx, 200., 20.);
    up(wcx, 200., 20.);
    wcx.run_until_parked();
    wcx.update(|window, cx| {
        let has = gpui_kit::base::TextSelection::has_selection(window, cx);
        let text = gpui_kit::base::TextSelection::selected_text(window, cx);
        assert!(
            has,
            "Scrollable+keyed tv_static+域尾哨兵+多块大文本:孤立组合应可选"
        );
        assert!(!text.trim().is_empty(), "孤立组合的复制取值应非空");
    });
}
