//! liuma 桌面客户端(GPUI 原生 UI)。
//!
//! 单进程装配:进程内构造 [`liuma_core::registry::AppHost`](经
//! [`host::HostBridge`],不起 loopback HTTP);UI 布局
//! (280px 侧栏 + 统一列宽居中的消息列/输入卡,策略见 shell::metrics;
//! 桌面为唯一 UI)。关窗即退出。
//!
//! 用法:`liuma-desktop [--workspace <dir>] [--fake]`
//! - workspace 默认当前目录(api key 从环境变量解析);
//! - fake 用假 provider 驱动(自检/演示)。

// Windows release 下不弹控制台
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
mod app_icon;
mod features;
mod gitinfo;
mod kits;
mod shell;

// gpui-fast 的宏(actions! / #[test])展开输出字面 `gpui::` 路径;stock
// gpui-pre 的宏会改写成 `::gpui_kit::`,gpui-fast 不会。别名把门面挂回
// `gpui` 名下,只影响本 crate(bin + 全部 #[cfg(test)] 模组)。
extern crate gpui_kit as gpui;

// `t!` 展开为 `crate::_rust_i18n_t!` → `crate::_rust_i18n_try_translate`
// (`kits/i18n/gen.rs` 的 `i18n!` 生成物),故两个名字必须在 crate 根可见。
pub(crate) use crate::kits::i18n::backend::{_rust_i18n_t, _rust_i18n_try_translate};

use std::path::PathBuf;

use futures::StreamExt as _;
use gpui_kit::{AppContext, WindowBounds, WindowOptions, px, size};

use crate::shell::host::HostBridge;
use crate::shell::store::AppStore;

/// 命令行(liuma-desktop 自有极简面,不引入 clap)
struct Args {
    workspace: Option<PathBuf>,
    fake: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        workspace: None,
        fake: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--workspace" => args.workspace = it.next().map(PathBuf::from),
            "--fake" => args.fake = true,
            _ => {}
        }
    }
    args
}

fn main() {
    let args = parse_args();
    let workspace = args
        .workspace
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    // 真实模式凭据:环境变量有则注入(最高优先);缺席不拒启——
    // 凭据链还可在运行时从设置(provider api_key)解析,首运行
    // onboarding 也依赖「无 key 可进应用」
    let api_key = if args.fake {
        String::new()
    } else {
        liuma_core::Resolved::resolve_api_key(None).unwrap_or_default()
    };
    let (bridge, frames_rx) = match HostBridge::new(workspace, args.fake, &api_key) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("宿主装配失败:{e}");
            std::process::exit(1);
        }
    };
    let probe_host = bridge.host().clone();

    // 官方入口形态(gpui_kit::application 内部即
    // Application::with_platform(current_platform(false)),非 wasm 分支逐字等价)
    gpui_kit::application()
        // 自有图标(_liuma)优先,回落 gpui-component 内置(见 icons.rs)
        .with_assets(kits::icons::MergedAssets)
        .run(move |cx| {
            // Dock 图标(favicon 同款流马,深色圆角方底同主题
            // BASE;NSImage 的 app-icon 位不支持 SVG,运行时绘制,见
            // app_icon.rs)。必须在此设置:gpui 在 Application 构造
            // 时已把 NSApplication 单例创建为 GPUIApplication 子类
            // (带 platform ivar);若提前调用 sharedApplication,单例
            // 会是基类 NSApplication 实例,gpui 读写 platform ivar
            // 时崩「Ivar platform not found」(release 实测)
            #[cfg(target_os = "macos")]
            app_icon::set_app_icon_1024();

            // 必须先于任何 gpui-component 组件使用(gpui_kit::init 按
            // feature 转发 component::init,官方入口形态)
            gpui_kit::init(cx);
            // 界面语言:开窗前读持久化档位(缺省 zh;切换 = 设置页,
            // 经 i18n::apply 全窗即时生效)——先于任何窗口文案渲染
            kits::i18n::init(&bridge.host().language());
            // 内置主题集进 registry(liuma 双盘 + 官方 21 套;
            // 先于任何 apply)
            kits::theme::load_builtin_themes(cx);
            // 用户主题(<LIUMA_HOME|~/.liuma>/themes):首扫 + 文件监听;
            // 在持久化主题名 apply 之前,自定义主题可被选中/持久化
            kits::theme::init_user_themes(cx);
            // 持久化的两盘主题名(空串 = Liuma 默认;未知名由
            // config_for_mode 回落)
            for (m, name) in [
                (
                    gpui_kit::component::ThemeMode::Light,
                    bridge.host().theme_light(),
                ),
                (
                    gpui_kit::component::ThemeMode::Dark,
                    bridge.host().theme_dark(),
                ),
            ] {
                if !name.is_empty() {
                    kits::theme::set_theme(m, &name, cx);
                }
            }
            // 外观三档:开窗前读持久化档位(HostBridge 在 run
            // 前已装配,启动无闪色);theme::apply 同步双盘 + 组件 token
            kits::theme::apply(
                kits::theme::Appearance::parse(&bridge.host().appearance()),
                cx,
            );
            // 应用全局键表(⇧⌘P 等;测试装配同源,见 shell::bind_global_keys)
            shell::bind_global_keys(cx);
            // 关窗即退出(单窗口应用;订阅泄漏存续于进程生命周期)
            std::mem::forget(cx.on_window_closed(|cx, _| cx.quit()));

            // LIUMA_PROBE 主动探针(须显式 `prompt:<文本>` 值;被动探针
            // 只要变量在场即开,与主动探针共用变量名,此前按「在场」触发
            // ——取证时设 LIUMA_PROBE=anchors 起第二个实例,3s 后凭空建
            // 出一只探针会话并发真消息,侧栏多会话 + 烧 token,实测事故):
            // 3s 后经 bridge runtime 建会话并发一条消息(与 UI store.send
            // 同一路径;免 GUI 输入测生产 runloop 节奏)
            if let Some(text) = std::env::var_os("LIUMA_PROBE")
                && let Some(text) = text.to_str()
                && let Some(text) = text.strip_prefix("prompt:")
            {
                let host = probe_host.clone();
                let text = text.to_string();
                bridge.spawn_on_host(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let id = host.create_session(None, None, None);
                    eprintln!("[probe] session {id} prompt");
                    let r = host
                        .prompt(
                            &id,
                            &[serde_json::json!({ "type": "text", "text": text })],
                            "queue",
                        )
                        .await;
                    eprintln!("[probe] prompt result: {}", r.is_ok());
                });
            }

            let store = cx.new(|cx| AppStore::new(bridge, cx));

            // 帧泵:mux/host 双流 → reducer
            let pump_store = store.clone();
            cx.spawn(async move |cx| {
                let probe = std::env::var_os("LIUMA_PROBE").is_some();
                let t0 = std::time::Instant::now();
                let mut rx = frames_rx;
                while let Some(frame) = rx.next().await {
                    if probe {
                        let ty = frame.payload["event"]["type"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        eprintln!(
                            "[t2] +{}ms {} {}",
                            t0.elapsed().as_millis(),
                            frame.method,
                            ty
                        );
                    }
                    pump_store.update(cx, |s, cx| s.apply_frame(frame, cx));
                }
                Ok::<(), anyhow::Error>(())
            })
            .detach();

            // 常驻占用探针(LIUMA_PROBE 在场时每 5s 打印一次 footprint;
            // 与帧耗时探针配合回答「内存为什么涨/涨在哪」)
            if std::env::var_os("LIUMA_PROBE").is_some() {
                let fp_store = store.clone();
                cx.spawn(async move |cx| {
                    let mut tick = 0u32;
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(5))
                            .await;
                        tick += 1;
                        let line = fp_store.read_with(cx, |s, _| s.footprint());
                        // RSS 同报:内存归因定位(增长随翻页 = 宿主路径;
                        // 随滚动距离连续 = 渲染层)。macOS ps;其他平台缺省 0
                        #[cfg(target_os = "macos")]
                        let rss = std::process::Command::new("ps")
                            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                            .output()
                            .ok()
                            .and_then(|o| {
                                String::from_utf8_lossy(&o.stdout)
                                    .trim()
                                    .parse::<u64>()
                                    .ok()
                            })
                            .unwrap_or(0);
                        #[cfg(not(target_os = "macos"))]
                        let rss = 0u64;
                        eprintln!("[fp] t={}s rss={rss}KB {line}", tick * 5);
                    }
                })
                .detach();
            }

            // 自绘标题栏(隐系统标题,交通灯悬浮;见 ui 根布局 TitleBar)。
            // 必须以 TitleBar::window_options() 打底:app_owns_titlebar_drag
            // 压掉系统原生「双击标题栏缩放」——组件在 macOS 分支已自挂
            // on_double_click,两通道并存 = 双击进全屏立刻弹回(各执行一次;
            // 源码注释明言),且原生区还会吃掉标题栏单击做双击判定
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                window_min_size: Some(size(px(960.), px(640.))),
                ..gpui_kit::component::TitleBar::window_options()
            };
            let view_store = store.clone();
            cx.spawn(async move |cx| {
                // `open_window` 要 `&mut App`,而这里是 AsyncApp → 整块进 update
                cx.update(move |cx| {
                    // kit 入口自把内容包进 Root(0.7.0 起 Root 亦自渲染
                    // dialog/sheet/notification 三层);builder 只返回应用内容
                    let (window_handle, _view) =
                        gpui_kit::open_window(options, cx, move |window, cx| {
                            view_store.update(cx, |s, cx| s.attach_window_state(window, cx));
                            cx.new(|cx| shell::WorkspaceView::new(view_store.clone(), cx))
                        })?;
                    // 启动即激活到前台:终端/nohup 拉起时窗口默认留在
                    // 启动方背后,macOS 对被遮挡窗口停发绘制帧,首帧之后
                    // 界面冻结(实测空面板)直到用户手动点到它。
                    // 置于 Root 构造之后,与手工包 Root 时的次序一致
                    window_handle.update(cx, |_, window, _| window.activate_window())?;
                    // `_view`:Root 以 AnyView 强持有内容,句柄丢开不拆视图;
                    // WorkspaceView 除测试计数外无自有状态,状态全在 AppStore
                    Ok::<_, anyhow::Error>(())
                })
            })
            .detach();
        });
}
