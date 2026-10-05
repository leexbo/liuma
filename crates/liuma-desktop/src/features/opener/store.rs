//! 「在编辑器中打开」切片状态与动作:清单/选中/图标缓存(内存态,
//! 不落盘,与 sidebar_collapsed 同级)+ 探测与启动的派发(bridge
//! blocking 池执行,结果回 GPUI store)。视图见 views。

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::Context;

use crate::features::opener::probe::{self, ResolvedApp};
use crate::features::opener::views::display_name;
use crate::kits::i18n::t;
use crate::shell::store::AppStore;

/// 打开方式选择器状态(顶栏分体钮 + 下拉菜单)
#[derive(Default)]
pub(crate) struct OpenerStore {
    /// chevron 下拉开态(受控 Popover;钮在标题栏拖拽区上)
    pub menu_open: bool,
    /// 本次鼠标手势中菜单已被外点关闭(gpui 捕获相的 on_mouse_down_out
    /// 先于 bubble 相的 on_click;chevron 复点时据此不再重新打开)。
    /// 菜单已关时的任一次按下都会复位,无陈旧歧义。
    pub gesture_dismissed: bool,
    /// 探测结果(清单序;finder/terminal fixed 恒在 → macOS 上非空)
    pub apps: Vec<ResolvedApp>,
    /// 当前选中(内存态不落盘;初始 = 首个可用项 = 访达)
    pub selected: Option<&'static str>,
    /// id → 真身图标;None = 提取失败(渲染回落通用图标);缺席 = 未到
    pub icons: HashMap<&'static str, Option<Arc<gpui_kit::RenderImage>>>,
    /// 探测一次性守卫(挂窗触发;进程内不重探)
    pub(crate) probed: bool,
    /// 启动失败 toast 文案(底部居中;点击关,定时自清兜底)
    pub launch_error: Option<String>,
    /// toast 自清任务(重触发即替换;旧句柄 drop = 旧定时取消)
    pub(crate) error_clear: Option<gpui_kit::Task<()>>,
}

impl AppStore {
    /// 挂窗一次:探测应用清单,落地后派发图标提取。不选惰性首开——
    /// 主钮直接开就需要图标,且探测只是十余次 stat,毫秒级。
    /// 测试面早退:全量并行下几十个 harness 挂窗各自跑真实子进程
    /// (xcode-select / plutil / sips)会形成子进程风暴,放大其它测试
    /// 的时序敏感断言(实测 3/3 全量复现,失败面逐轮漂移);需要清单
    /// 的用例自行播种 detect_apps。
    pub(crate) fn opener_ensure_probed(&mut self, cx: &mut Context<Self>) {
        if self.opener.probed || cfg!(test) {
            return;
        }
        self.opener.probed = true;
        let rx = self.bridge.call_blocking(probe::detect_apps);
        let store = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            if let Ok(apps) = rx.await {
                store.update(cx, |s, cx| {
                    s.opener.apps = apps;
                    // 初始选中 = 清单首个可用项(访达)
                    s.opener.selected = s.opener.apps.first().map(|a| a.id);
                    s.opener_start_icons(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 二段派发:图标提取。菜单不等图标——未到先画回落图标,到达
    /// 逐个补画;单 app 失败独立回落,不阻塞其它。
    fn opener_start_icons(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<(&'static str, std::path::PathBuf)> = self
            .opener
            .apps
            .iter()
            .map(|a| (a.id, a.bundle.clone()))
            .collect();
        let rx = self
            .bridge
            .call_blocking(move || probe::extract_icons(targets));
        let store = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            if let Ok(icons) = rx.await {
                store.update(cx, |s, cx| {
                    for (id, png) in icons {
                        let rendered = png.and_then(|b| probe::render_image(&b));
                        s.opener.icons.insert(id, rendered);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 主钮:用当前选中的应用打开工作区目录
    pub(crate) fn opener_open_current(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.opener.selected else {
            return;
        };
        self.opener_open_app(id, cx);
    }

    /// 菜单行点击 = 记住选择 + 打开(dsh 语义);清单缺席静默
    pub(crate) fn opener_open_app(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.opener.selected = Some(id);
        let Some(app) = self.opener.apps.iter().find(|a| a.id == id).cloned() else {
            return;
        };
        let Some(dir) = self.current_workspace_dir() else {
            return;
        };
        let rx = self.bridge.call_blocking(move || probe::launch(&app, &dir));
        let name = display_name(id).to_string();
        let store = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            // 通道断 = 任务折损,与启动失败同面呈现
            if let Err(msg) = rx
                .await
                .unwrap_or_else(|_| Err(t!("opener.launch_channel_failed").to_string()))
            {
                store.update(cx, |s, cx| {
                    s.opener_show_launch_error(
                        t!("opener.launch_failed", app = &name, msg = &msg).to_string(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    /// 失败 toast:置文案 + 3s 定时自清(重触发即替换旧定时)
    pub(crate) fn opener_show_launch_error(&mut self, text: String, cx: &mut Context<Self>) {
        self.opener.launch_error = Some(text);
        let store = cx.entity().clone();
        self.opener.error_clear = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(3))
                .await;
            store.update(cx, |s, cx| {
                if s.opener.launch_error.take().is_some() {
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// toast 点击关闭(与附件拒收 toast 同款静默清理)
    pub(crate) fn opener_dismiss_launch_error(&mut self) {
        self.opener.launch_error = None;
    }
}
