//! 探测与执行副作用层:应用存在性探测(stat + xcode-select)、.app
//! 图标提取(plutil → icns → sips → PNG 字节)、目录启动。全部同步
//! 阻塞实现,由 store 经 bridge.call_blocking 上 blocking 池;非
//! macOS 入口早退返回空,清单与纯函数仍全平台参与编译(可单测)。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use crate::features::opener::catalog::{self, AppEntry, CATALOG, LaunchKind};

/// 解析后的应用(清单序的一员)
#[derive(Clone, Debug)]
pub(crate) struct ResolvedApp {
    pub id: &'static str,
    pub kind: LaunchKind,
    /// 启动(`open -a` / 图标提取用;Xcode 仅 CLT 时 = 开发者目录)
    pub bundle: PathBuf,
}

/// 探测清单:非 macOS → 空。finder/terminal 为 fixed 条目恒在,
/// macOS 上至少两项。每进程只跑一次(store 侧 probed 守卫;运行中
/// 安装/改名的应用重启后可见,dsh 同口径)。
pub(crate) fn detect_apps() -> Vec<ResolvedApp> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    CATALOG
        .iter()
        .filter_map(|e| detect_one(e, &home))
        .collect()
}

fn detect_one(entry: &AppEntry, home: &Path) -> Option<ResolvedApp> {
    let bundle = match entry.kind {
        // fixed 条目(finder/terminal):系统自带恒在,bundle 即图标源
        LaunchKind::Finder | LaunchKind::Terminal => PathBuf::from(entry.icon_bundle?),
        // Xcode:xcode-select -p 推导 .app;无 .app(仅 CLT)兜底开发
        // 者目录(xed 仍可用);xcode-select 缺席 → 标准位置 stat
        LaunchKind::Xed => xcode_bundle().or_else(|| {
            catalog::resolve_bundle(&catalog::bundle_candidates(entry, home), |p| p.exists())
        })?,
        LaunchKind::OpenApp => {
            catalog::resolve_bundle(&catalog::bundle_candidates(entry, home), |p| p.exists())?
        }
    };
    Some(ResolvedApp {
        id: entry.id,
        kind: entry.kind,
        bundle,
    })
}

/// `xcode-select -p` → 祖先链上首个 `*.app`(标准安装 = Xcode.app);
/// 仅 CLT(无 .app 祖先)或命令缺席/失败 = None
fn xcode_bundle() -> Option<PathBuf> {
    let out = Command::new("xcode-select").arg("-p").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let dev_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    if dev_dir.as_os_str().is_empty() {
        return None;
    }
    app_ancestor(&dev_dir).or(Some(dev_dir))
}

/// 路径自身或祖先中首个 `.app` 目录;无 = None(纯函数,可测)
fn app_ancestor(from: &Path) -> Option<PathBuf> {
    let mut cur = from;
    loop {
        if cur.extension().is_some_and(|e| e == "app") {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
}

/// 批量提取图标:(id, bundle) → PNG 字节(失败 = None)。每 app 至
/// 多 plutil + sips 两次子进程,blocking 池串行承载(检出 app 通常
/// 个位数,亚秒级)。
pub(crate) fn extract_icons(
    targets: Vec<(&'static str, PathBuf)>,
) -> Vec<(&'static str, Option<Vec<u8>>)> {
    targets
        .into_iter()
        .map(|(id, bundle)| {
            let png = extract_icon_png(id, &bundle);
            (id, png)
        })
        .collect()
}

/// 单 app 图标(dsh icons 管线):Info.plist → `CFBundleIconFile`
/// (.icns)→ 回落 Resources 首个 .icns → `sips` 转 128px PNG。
/// image 0.25 解不了 icns,sips 须经临时文件中转;任一步失败 = None。
fn extract_icon_png(id: &str, bundle: &Path) -> Option<Vec<u8>> {
    let icns = icns_path(bundle)?;
    let tmp = std::env::temp_dir().join(format!("liuma-icon-{id}-{}.png", std::process::id()));
    let ok = Command::new("sips")
        .args(["-s", "format", "png", "-Z", "128"])
        .arg(&icns)
        .arg("--out")
        .arg(&tmp)
        .output()
        .ok()?
        .status
        .success();
    let png = if ok { std::fs::read(&tmp).ok() } else { None };
    let _ = std::fs::remove_file(&tmp);
    // sips 退 0 但不落盘的异常形态同样当失败
    png.filter(|b| !b.is_empty())
}

/// 图标 icns 路径:Info.plist 声明优先,坏 JSON/缺键/文件缺席回落
/// Resources 目录名字序首个 .icns
fn icns_path(bundle: &Path) -> Option<PathBuf> {
    let plist = bundle.join("Contents/Info.plist");
    if let Ok(out) = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&plist)
        .output()
        && out.status.success()
        && let Some(name) = catalog::icon_file_from_plist(&String::from_utf8_lossy(&out.stdout))
    {
        let declared = bundle.join("Contents/Resources").join(name);
        if declared.exists() {
            return Some(declared);
        }
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(bundle.join("Contents/Resources"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "icns"))
        .collect();
    entries.sort();
    entries.into_iter().next()
}

/// 启动目录:spawn(stdio 全 null)+ 1s 退出码视窗(dsh 口径:窗口
/// 内退非零 = 失败,仍在跑 = 已启动且不追杀)。Xcode 主命令 xed 失
/// 败(不存在/退非零)回退 `open -a <bundle>`。
pub(crate) fn launch(app: &ResolvedApp, dir: &Path) -> Result<(), String> {
    let (prog, args) = catalog::launch_argv(app.kind, &app.bundle, dir);
    let primary = run_window(&prog, &args);
    match (app.kind, primary) {
        (_, Ok(())) => Ok(()),
        (LaunchKind::Xed, Err(first)) => {
            // 回退 = OpenApp 同款 argv(`open -a <bundle>`)
            let (prog, args) = catalog::launch_argv(LaunchKind::OpenApp, &app.bundle, dir);
            run_window(&prog, &args).map_err(|second| format!("{first}; {second}"))
        }
        (_, Err(e)) => Err(e),
    }
}

/// spawn + 轮询 try_wait 至 1s:退 0 = 成功;退非零 = Err;仍在跑 =
/// 已启动(进程交还系统,不 reap 不追杀)
fn run_window(prog: &OsStr, args: &[OsString]) -> Result<(), String> {
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", prog.to_string_lossy()))?;
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "{}: exit {}",
                    prog.to_string_lossy(),
                    status.code().unwrap_or(-1)
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("{}: {e}", prog.to_string_lossy())),
        }
    }
    Ok(())
}

/// PNG 字节 → RenderImage(GPUI 线程调用;128px 解码微秒级,与
/// preview 同址同法)。RenderImage 帧数据约定 **BGRA**(gpui
/// assets.rs),解码出的 RGBA 须换序,否则蓝/红通道互换——蓝底图标
/// 上屏成橙黄(实测访达/Xcode 偏色根因)
pub(crate) fn render_image(png: &[u8]) -> Option<Arc<gpui_kit::RenderImage>> {
    let decoded = image::load_from_memory(png).ok()?;
    let rgba = decoded.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let mut buffer = rgba.into_raw();
    for px in buffer.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, buffer)?;
    Some(Arc::new(gpui_kit::RenderImage::new(vec![
        image::Frame::new(buffer),
    ])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_finds_app_dir() {
        assert_eq!(
            app_ancestor(Path::new("/Applications/Xcode.app/Contents/Developer")),
            Some(PathBuf::from("/Applications/Xcode.app"))
        );
        // 仅 CLT:无 .app 祖先
        assert_eq!(
            app_ancestor(Path::new("/Library/Developer/CommandLineTools")),
            None
        );
    }
}
