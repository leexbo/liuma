//! 「在编辑器中打开」应用清单与纯推导:编译期白名单(dsh open-in-app
//! 的 macOS 目录面)+ bundle 候选/启动 argv/Info.plist 图标名解析的
//! 纯函数。子进程副作用层见 probe;bundle 名是宿主侧数据标识(文件
//! 系统真名),非界面文案。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 启动语义(dsh 分型的 macOS 子集;argv 推导见 [`launch_argv`])
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LaunchKind {
    /// `open <dir>`:访达不走 -a,目录交系统默认处理器(dsh shell-open)
    Finder,
    /// `open -a <bundle> <dir>`
    OpenApp,
    /// `xed <dir>`,失败回退 `open -a <bundle> <dir>`(Xcode 专属)
    Xed,
    /// `open -a Terminal <dir>`
    Terminal,
    /// `open -R <file>`:访达定位(reveal);非清单条目 kind,只由
    /// [`LaunchKind::for_file`] 与 reveal 菜单行产生
    Reveal,
}

impl LaunchKind {
    /// 文件目标的启动语义分化:访达目录开目录、文件 reveal;
    /// 其余原样(`open -a <app> <file>`、`xed <file>`、
    /// `open -a Terminal <file>` 均合法)
    pub(crate) fn for_file(self) -> Self {
        match self {
            Self::Finder => Self::Reveal,
            k => k,
        }
    }
}

/// 编译期清单条目。菜单顺序 = 数组顺序(访达 → 编辑器/IDE → Git GUI
/// → 终端,照 dsh catalog);探测只做存在性确认,不做全盘扫描。
pub(crate) struct AppEntry {
    /// 稳定键:选中态 / 图标缓存 / 菜单行 selector
    pub id: &'static str,
    pub kind: LaunchKind,
    /// `/Applications`、`~/Applications` 下按序尝试的 bundle 名;
    /// JetBrains 系列含直装与 Toolbox 两种拼写
    pub bundles: &'static [&'static str],
    /// fixed 条目(finder/terminal)的图标源 bundle;None = 用探测
    /// 解析出的 bundle 提图标
    pub icon_bundle: Option<&'static str>,
}

const fn app(id: &'static str, bundles: &'static [&'static str]) -> AppEntry {
    AppEntry {
        id,
        kind: LaunchKind::OpenApp,
        bundles,
        icon_bundle: None,
    }
}

/// macOS 应用清单(dsh catalog 的 darwin 面;顺序即菜单序)
pub(crate) const CATALOG: &[AppEntry] = &[
    AppEntry {
        id: "finder",
        kind: LaunchKind::Finder,
        bundles: &[],
        icon_bundle: Some("/System/Library/CoreServices/Finder.app"),
    },
    app("cursor", &["Cursor.app"]),
    app("vscode", &["Visual Studio Code.app"]),
    app("vscodeinsiders", &["Visual Studio Code - Insiders.app"]),
    app("windsurf", &["Windsurf.app"]),
    app("zed", &["Zed.app", "Zed Preview.app"]),
    app("sublimetext", &["Sublime Text.app"]),
    // Xcode:xcode-select -p 定位(见 probe::xcode_bundle),表内
    // bundles 仅作选择器缺席时的 stat 兜底
    AppEntry {
        id: "xcode",
        kind: LaunchKind::Xed,
        bundles: &["Xcode.app"],
        icon_bundle: None,
    },
    app("androidstudio", &["Android Studio.app"]),
    app(
        "intellij",
        &[
            "IntelliJ IDEA.app",
            "IntelliJ IDEA Ultimate.app",
            "IntelliJ IDEA CE.app",
        ],
    ),
    app(
        "pycharm",
        &[
            "PyCharm.app",
            "PyCharm Professional.app",
            "PyCharm CE.app",
            "PyCharm Community.app",
        ],
    ),
    app("webstorm", &["WebStorm.app"]),
    app("phpstorm", &["PhpStorm.app"]),
    app("goland", &["GoLand.app"]),
    app("rider", &["Rider.app", "JetBrains Rider.app"]),
    app("rustrover", &["RustRover.app"]),
    app("fork", &["Fork.app"]),
    app("sourcetree", &["Sourcetree.app"]),
    app("tower", &["Tower.app"]),
    app("sublimemerge", &["Sublime Merge.app"]),
    app("ghostty", &["Ghostty.app"]),
    app("warp", &["Warp.app"]),
    app("iterm", &["iTerm.app"]),
    app("kitty", &["kitty.app"]),
    AppEntry {
        id: "terminal",
        kind: LaunchKind::Terminal,
        bundles: &[],
        icon_bundle: Some("/System/Applications/Utilities/Terminal.app"),
    },
];

/// 候选 bundle 绝对路径:`/Applications` 前于 `~/Applications`,
/// bundle 名按声明序;首中即收(dsh 同序)
pub(crate) fn bundle_candidates(entry: &AppEntry, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in [Path::new("/Applications"), &home.join("Applications")] {
        for name in entry.bundles {
            out.push(root.join(name));
        }
    }
    out
}

/// 首个存在的候选;全缺 = None(exists 注入,纯函数可测)
pub(crate) fn resolve_bundle(
    candidates: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    candidates.iter().find(|p| exists(p)).cloned()
}

/// 启动 argv:kind → (程序, 参数),目标(目录或文件)恒置尾
pub(crate) fn launch_argv(
    kind: LaunchKind,
    bundle: &Path,
    target: &Path,
) -> (OsString, Vec<OsString>) {
    let target = target.as_os_str().to_os_string();
    match kind {
        LaunchKind::Finder => ("open".into(), vec![target]),
        LaunchKind::OpenApp => (
            "open".into(),
            vec!["-a".into(), bundle.as_os_str().to_os_string(), target],
        ),
        LaunchKind::Xed => ("xed".into(), vec![target]),
        LaunchKind::Terminal => ("open".into(), vec!["-a".into(), "Terminal".into(), target]),
        LaunchKind::Reveal => ("open".into(), vec!["-R".into(), target]),
    }
}

/// Info.plist JSON → icns 文件名:取 `CFBundleIconFile`,缺 `.icns`
/// 后缀则补(dsh icons 同规);坏 JSON / 缺键 = None
pub(crate) fn icon_file_from_plist(plist_json: &str) -> Option<String> {
    let plist: serde_json::Value = serde_json::from_str(plist_json).ok()?;
    let name = plist.get("CFBundleIconFile")?.as_str()?;
    if name.ends_with(".icns") {
        Some(name.to_string())
    } else {
        Some(format!("{name}.icns"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> &'static AppEntry {
        CATALOG.iter().find(|e| e.id == id).expect("test")
    }

    /// 清单完整性:id 唯一、访达首位、终端末位、OpenApp 条目 bundle
    /// 名全部 .app 结尾、fixed 条目带图标源、Reveal 非清单 kind
    #[test]
    fn catalog_shape() {
        let mut seen = std::collections::HashSet::new();
        for e in CATALOG {
            assert!(seen.insert(e.id), "duplicate id {}", e.id);
            match e.kind {
                LaunchKind::Finder | LaunchKind::Terminal => {
                    assert!(e.bundles.is_empty());
                    assert!(e.icon_bundle.is_some());
                }
                LaunchKind::OpenApp | LaunchKind::Xed => {
                    assert!(!e.bundles.is_empty());
                    assert!(e.bundles.iter().all(|n| n.ends_with(".app")));
                }
                LaunchKind::Reveal => panic!("Reveal is not a catalog kind"),
            }
        }
        assert_eq!(CATALOG.first().unwrap().id, "finder");
        assert_eq!(CATALOG.last().unwrap().id, "terminal");
        assert!(entry("zed").bundles.contains(&"Zed Preview.app"));
    }

    #[test]
    fn candidates_applications_root_first() {
        let e = entry("zed");
        let got = bundle_candidates(e, Path::new("/home/u"));
        assert_eq!(
            got,
            vec![
                PathBuf::from("/Applications/Zed.app"),
                PathBuf::from("/Applications/Zed Preview.app"),
                PathBuf::from("/home/u/Applications/Zed.app"),
                PathBuf::from("/home/u/Applications/Zed Preview.app"),
            ]
        );
    }

    #[test]
    fn resolve_first_hit_or_none() {
        let cands = vec![
            PathBuf::from("/Applications/Zed.app"),
            PathBuf::from("/Applications/Zed Preview.app"),
        ];
        let hit = resolve_bundle(&cands, |p| p == Path::new("/Applications/Zed Preview.app"));
        assert_eq!(hit, Some(PathBuf::from("/Applications/Zed Preview.app")));
        assert_eq!(resolve_bundle(&cands, |_| false), None);
    }

    #[test]
    fn argv_per_kind() {
        let target = Path::new("/tmp/ws");
        let bundle = Path::new("/Applications/Zed.app");
        let (p, a) = launch_argv(LaunchKind::Finder, bundle, target);
        assert_eq!(p, "open");
        assert_eq!(a, [target.as_os_str()]);
        let (p, a) = launch_argv(LaunchKind::OpenApp, bundle, target);
        assert_eq!(p, "open");
        assert_eq!(a, ["-a", "/Applications/Zed.app", "/tmp/ws"]);
        let (p, a) = launch_argv(LaunchKind::Xed, bundle, target);
        assert_eq!(p, "xed");
        assert_eq!(a, [target.as_os_str()]);
        let (p, a) = launch_argv(LaunchKind::Terminal, bundle, target);
        assert_eq!(p, "open");
        assert_eq!(a, ["-a", "Terminal", "/tmp/ws"]);
        let file = Path::new("/tmp/ws/out.svg");
        let (p, a) = launch_argv(LaunchKind::Reveal, bundle, file);
        assert_eq!(p, "open");
        assert_eq!(a, ["-R", "/tmp/ws/out.svg"]);
    }

    /// 文件语义分化:仅访达变 reveal,其余 kind 原样
    #[test]
    fn for_file_splits_finder_only() {
        assert_eq!(LaunchKind::Finder.for_file(), LaunchKind::Reveal);
        assert_eq!(LaunchKind::OpenApp.for_file(), LaunchKind::OpenApp);
        assert_eq!(LaunchKind::Xed.for_file(), LaunchKind::Xed);
        assert_eq!(LaunchKind::Terminal.for_file(), LaunchKind::Terminal);
        assert_eq!(LaunchKind::Reveal.for_file(), LaunchKind::Reveal);
    }

    #[test]
    fn plist_icon_name() {
        assert_eq!(
            icon_file_from_plist(r#"{"CFBundleIconFile":"applet"}"#),
            Some("applet.icns".to_string())
        );
        assert_eq!(
            icon_file_from_plist(r#"{"CFBundleIconFile":"App.icns"}"#),
            Some("App.icns".to_string())
        );
        assert_eq!(icon_file_from_plist(r#"{"Other":1}"#), None);
        assert_eq!(icon_file_from_plist("not json"), None);
    }
}
