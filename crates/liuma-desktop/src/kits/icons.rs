//! 自定义图标与资产合并。
//!
//! [`LiumaIcon`] 提供上游缺名与刻意变体(自有 include_bytes! 静态表
//! [`LIUMA_ICONS`]);与上游 lucide 同名同形者不再本地抄份,直接取
//! gpui-kit 内嵌全量 lucide 资产(0.9.0 起)。[`install_icon_resolver`]
//! 把 [`LIUMA_OVERRIDES`] 两个形制级变体接管为全局改道(IconName 与
//! LiumaIcon 两条入口同一枚),资产经 [`MergedAssets`] 合并,零新增依赖。
//!
//! 尺寸纪律:0.5.1 的 `with_size` 经 render 链恒覆盖
//! em 回退,独立渲染的 Icon 尺寸确定性成立;但宿主组件(Button/
//! Tab/Select)会强制覆写传入 icon 的尺寸——本 crate 全自绘 div,
//! 不受影响。所有 Icon 一律经 [`fixed`] 定尺寸。

use gpui_kit::component::{Icon, IconName, IconNamed, Sizable};
use gpui_kit::{AssetSource, Result, SharedString, px};
use std::borrow::Cow;

/// 必须走自有内嵌资产的名字:上游 lucide 集没有的名字(自研/
/// duotone 16 网格族),加 [`LIUMA_OVERRIDES`] 里两个刻意不同形的
/// 变体(folder-open 填充 16 网格、wrench 细描边)。其余枚举变体
/// 与上游同名同形,直接引用上游内嵌资产
const LIUMA_ONLY: &[&str] = &[
    "agent-preset",
    "circle-help",
    "command-prompt",
    "decision",
    "folder-close",
    "folder-open",
    "hook",
    "loader-ring",
    "logo",
    "mcp",
    "new-chat",
    "personalization",
    "project-add",
    "search-outline",
    "think",
    "trace",
    "wrench",
];

/// 解析器接管清单:上游同名路径全局改道自有变体。仅两枚,皆
/// 形制级差异(非 lucide 版本漂移):`folder-open` = 16 网格填充
/// 双色调(侧栏族),上游为 24 网格描边;`wrench` = 1.8 细描边
/// (12px 行内尺寸与界面笔画同量级),上游 2.0。经 resolver 改道
/// 后,IconName 与 LiumaIcon 两条入口画出同一枚
pub const LIUMA_OVERRIDES: &[&str] = &["folder-open", "wrench"];

/// liuma 图标:上游缺名或刻意变体走自有资产,同名同形直接取上游
/// (上游清单见 [`LIUMA_ONLY`];解析器接管见 [`LIUMA_OVERRIDES`])
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LiumaIcon {
    /// preset `standard` 徽章 / 未注册工具兜底
    Sparkles,
    /// preset 非 `standard` / flash 系模型
    Zap,
    /// pro·reasoner 系模型
    Brain,
    /// 其余模型
    Gauge,
    /// 权限 `read-only`
    Shield,
    /// 权限 `full-access`
    ShieldAlert,
    /// 权限 `workspace-write`
    ShieldCheck,
    /// 重命名 / 文件编辑类工具
    Pencil,
    /// 分叉
    GitBranch,
    /// 归档
    Archive,
    /// 取消归档(archive-restore.svg,lucide 同源;归档管理页恢复钮)
    ArchiveRestore,
    /// 删除(垃圾桶;trash.svg,lucide 同源。内置 Delete 是退格键形,
    /// 语义不符——归档管理页删单条用)
    Trash,
    /// 计划 / todo 类
    ListChecks,
    /// 会话行
    Message,
    /// 置顶(侧栏行菜单;pin.svg,ISC 内置资产)
    Pin,
    /// 取消置顶(已置顶行的置顶钮形态;pin-off.svg 同源。中性灰,
    /// 状态只由图标区分——品牌蓝高亮被否)
    PinOff,
    /// 工作区信息卡「编辑项目」行(settings.svg,ISC 同源)
    Settings,
    /// hero 品牌流马(assets/logo.svg,设计定稿)
    Logo,
    /// Session log 导出
    Download,
    /// Duration 切换(轨迹工具栏)
    Clock,
    /// code 类工具
    Code,
    /// 问答类工具
    CircleHelp,
    /// `goal` 工具
    Target,
    /// `jobs` 工具
    Briefcase,
    /// `workflow` 工具
    Workflow,
    /// `ralph` 工具
    Infinity,
    /// 外观「跟随系统」cube(显示器)
    Monitor,
    /// preset 模式(最简/标准)徽章 — trio(agent preset outline 16)
    AgentPreset,
    /// LLM 请求重试行(lucide refresh-cw)
    RefreshCw,
    /// 助手消息工具调用行(lucide wrench;统一扳手形)
    Wrench,
    /// 附件入口(输入卡底排独立钮)
    Paperclip,
    /// token 用量(statusbar chip/详情卡、聊天尾部用量 pill;lucide
    /// 同源,经 [`MergedAssets`] 全量目录回落加载)
    Database,
    // ── 文件类型族(lucide;组件默认 101 图标集缺,自有内嵌)──
    /// 文件树标签/目录树
    FolderTree,
    /// 图片类文件
    FileImage,
    /// 代码类文件
    FileCode,
    /// 压缩包
    FileArchive,
    /// json 系
    FileBraces,
    /// rust(齿轮)
    FileCog,
    /// git 元数据
    FileDiff,
    /// env 系
    FileKey,
    /// 锁文件
    FileLock,
    /// shell 脚本
    FileTerminal,
    /// 表格
    FileSpreadsheet,
    /// 视频
    FileVideoCamera,
    /// 音频
    FileVolume,
    /// symlink/other 行
    FileSymlink,
    /// 图表(ppt 族)
    FileChartColumn,
    /// 通用类型文件(pdf/config/word/字体)
    FileType,
    /// 换行开关(预览头行)
    TextWrap,
    // ── 侧栏族(16 viewBox fill 形)──
    /// 顶栏搜索钮 / 搜索框前导(ic_ds_search_outline_16)
    SearchOutline,
    /// 顶栏视图选项钮:分组/排序菜单(ic_ds_personalization_outline_16)
    Personalization,
    /// 顶栏添加工作区钮(ic_ds_project_add_outline_16)
    ProjectAdd,
    /// 「新会话」按钮(ic_ds_new_chat_outline_16)
    NewChat,
    /// 工作区组头·打开(folder_open_16 duotone)
    FolderOpen,
    /// 工作区组头·关闭(folder_close_16)
    FolderClose,
    /// 轨迹
    Trajectory,
    /// 设置侧栏 MCP 项
    Mcp,
    /// 设置侧栏决策模型项
    Decision,
    /// 设置侧栏钩子项
    Hook,
    /// 设置侧栏模型设置项
    Models,
    /// present 工具 / 交付宣告(包裹;与 Models 共用 box 资产)
    Box,
    /// 设置侧栏常规项
    General,
    /// 命令类工具行 / 直播组头 commands 活动(`>_`)
    CommandPrompt,
    /// 思考折叠行 / 直播组头 thinking 活动
    Think,
    /// 运行态旋转环(细描边;大尺寸下与界面笔画同量级)
    LoaderRing,
}

impl IconNamed for LiumaIcon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::Sparkles => "sparkles",
            Self::Zap => "zap",
            Self::Brain => "brain",
            Self::Gauge => "gauge",
            Self::Shield => "shield",
            Self::ShieldAlert => "shield-alert",
            Self::ShieldCheck => "shield-check",
            Self::Pencil => "file-pen",
            Self::GitBranch => "git-branch",
            Self::Archive => "archive",
            Self::ArchiveRestore => "archive-restore",
            Self::Trash => "trash",
            Self::ListChecks => "list-checks",
            Self::Message => "message-circle",
            Self::Pin => "pin",
            Self::PinOff => "pin-off",
            Self::Settings => "settings",
            Self::Logo => "logo",
            Self::Download => "download",
            Self::Clock => "clock",
            Self::Code => "code",
            Self::CircleHelp => "circle-help",
            Self::Target => "target",
            Self::Briefcase => "briefcase",
            Self::Workflow => "workflow",
            Self::Infinity => "infinity",
            Self::Monitor => "monitor",
            Self::AgentPreset => "agent-preset",
            Self::RefreshCw => "refresh-cw",
            Self::Wrench => "wrench",
            Self::Paperclip => "paperclip",
            Self::Database => "database",
            Self::FolderTree => "folder-tree",
            Self::FileImage => "file-image",
            Self::FileCode => "file-code",
            Self::FileArchive => "file-archive",
            Self::FileBraces => "file-braces",
            Self::FileCog => "file-cog",
            Self::FileDiff => "file-diff",
            Self::FileKey => "file-key",
            Self::FileLock => "file-lock",
            Self::FileTerminal => "file-terminal",
            Self::FileSpreadsheet => "file-spreadsheet",
            Self::FileVideoCamera => "file-video-camera",
            Self::FileVolume => "file-volume",
            Self::FileSymlink => "file-symlink",
            Self::FileChartColumn => "file-chart-column",
            Self::FileType => "file-type",
            Self::TextWrap => "text-wrap",
            Self::SearchOutline => "search-outline",
            Self::Personalization => "personalization",
            Self::ProjectAdd => "project-add",
            Self::NewChat => "new-chat",
            Self::FolderOpen => "folder-open",
            Self::FolderClose => "folder-close",
            Self::Trajectory => "trace",
            Self::Mcp => "mcp",
            Self::Decision => "decision",
            Self::Hook => "hook",
            Self::Models => "box",
            Self::Box => "box",
            Self::General => "sliders-horizontal",
            Self::CommandPrompt => "command-prompt",
            Self::Think => "think",
            Self::LoaderRing => "loader-ring",
        };
        // 自研变体(上游无此名,或刻意不同形——见 [`LIUMA_OVERRIDES`])
        // 走 `icons/_liuma/`;其余与上游 lucide 同源同名,直接取上游
        // 内嵌资产(0.9.0 起全量 lucide 随包,不再本地抄一份)
        if LIUMA_ONLY.contains(&name) {
            format!("icons/_liuma/{name}.svg").into()
        } else {
            format!("icons/{name}.svg").into()
        }
    }
}

/// 合并资产源:liuma 自有图标(编译期内嵌,见 [`LIUMA_ICONS`])优先,
/// 其余回落 gpui-kit 内置资产(全量 lucide)
pub struct MergedAssets;

/// 自有图标静态表(路径 ↔ SVG 字节):仅上游缺名与刻意变体
/// ([`LIUMA_ONLY`]);同名同形者已迁移上游内嵌资产,不再本地抄份
const LIUMA_ICONS: &[(&str, &[u8])] = &[
    (
        "icons/_liuma/agent-preset.svg",
        include_bytes!("../../assets/icons/agent-preset.svg"),
    ),
    (
        "icons/_liuma/circle-help.svg",
        include_bytes!("../../assets/icons/circle-help.svg"),
    ),
    (
        "icons/_liuma/command-prompt.svg",
        include_bytes!("../../assets/icons/command-prompt.svg"),
    ),
    (
        "icons/_liuma/decision.svg",
        include_bytes!("../../assets/icons/decision.svg"),
    ),
    (
        "icons/_liuma/folder-close.svg",
        include_bytes!("../../assets/icons/folder-close.svg"),
    ),
    (
        "icons/_liuma/folder-open.svg",
        include_bytes!("../../assets/icons/folder-open.svg"),
    ),
    (
        "icons/_liuma/hook.svg",
        include_bytes!("../../assets/icons/hook.svg"),
    ),
    (
        "icons/_liuma/loader-ring.svg",
        include_bytes!("../../assets/icons/loader-ring.svg"),
    ),
    (
        "icons/_liuma/logo.svg",
        include_bytes!("../../assets/logo.svg"),
    ),
    (
        "icons/_liuma/mcp.svg",
        include_bytes!("../../assets/icons/mcp.svg"),
    ),
    (
        "icons/_liuma/new-chat.svg",
        include_bytes!("../../assets/icons/new-chat.svg"),
    ),
    (
        "icons/_liuma/personalization.svg",
        include_bytes!("../../assets/icons/personalization.svg"),
    ),
    (
        "icons/_liuma/project-add.svg",
        include_bytes!("../../assets/icons/project-add.svg"),
    ),
    (
        "icons/_liuma/search-outline.svg",
        include_bytes!("../../assets/icons/search-outline.svg"),
    ),
    (
        "icons/_liuma/think.svg",
        include_bytes!("../../assets/icons/think.svg"),
    ),
    (
        "icons/_liuma/trace.svg",
        include_bytes!("../../assets/icons/trace.svg"),
    ),
    (
        "icons/_liuma/wrench.svg",
        include_bytes!("../../assets/icons/wrench.svg"),
    ),
];

impl AssetSource for MergedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, data)) = LIUMA_ICONS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(data)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out = gpui_kit::assets::AllAssets.list(path)?;
        out.extend(
            LIUMA_ICONS
                .iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        Ok(out)
    }
}

/// 安装图标路径解析器(应用启动、`gpui_kit::init` 后调用一次):
/// [`LIUMA_OVERRIDES`] 清单内的上游路径全局改道自有变体——`Icon::new`
/// 与任何裸路径绘制(含组件库内部图标)统一画 liuma 形;其余路径
/// 原样放行(每帧绘制期调用,须保持 O(清单) 级廉价)
pub fn install_icon_resolver(cx: &mut gpui_kit::App) {
    gpui_kit::component::set_icon_resolver(cx, |path, _| {
        let name = path
            .strip_prefix("icons/")
            .and_then(|rest| rest.strip_suffix(".svg"))?;
        LIUMA_OVERRIDES
            .contains(&name)
            .then(|| SharedString::from(format!("icons/_liuma/{name}.svg")))
    });
}

/// 定尺寸图标(独立渲染必经此处,见模块文档尺寸纪律)
pub fn fixed(icon: impl Into<Icon>, size: f32) -> Icon {
    Icon::new(icon).with_size(px(size))
}

/// 按资产路径直接构造定尺寸图标(内置/自有图标统一出口)
fn fixed_path(path: SharedString, size: f32) -> Icon {
    Icon::empty().path(path).with_size(px(size))
}

/// 工具行图标(14px)。工具名权威域 = liuma-tools 各 `spec()` 共 12 个;
/// 另含 web/旧夹具遗留别名与未注册工具兜底(Sparkles)
pub fn tool_icon(name: &str) -> Icon {
    fixed_path(tool_path(name), 14.)
}

fn tool_path(name: &str) -> SharedString {
    // shell 工具的模型面名字随平台走,图标不跟着分两次写
    if name == liuma_sandbox::shell::tool_name() {
        return LiumaIcon::CommandPrompt.path();
    }
    match name {
        "file_read" | "read" | "web_fetch" => IconName::BookOpen.path(),
        "file_edit" | "edit" | "write" => LiumaIcon::Pencil.path(),
        "file_search" | "grep" | "glob" => IconName::Search.path(),
        "web_search" => IconName::Globe.path(),
        "todo_write" | "exit_plan_mode" => LiumaIcon::ListChecks.path(),
        "ask" => LiumaIcon::CircleHelp.path(),
        "code" => LiumaIcon::Code.path(),
        "goal" => LiumaIcon::Target.path(),
        "jobs" => LiumaIcon::Briefcase.path(),
        "workflow" => LiumaIcon::Workflow.path(),
        "present" => LiumaIcon::Box.path(),
        "ralph" => LiumaIcon::Infinity.path(),
        "subagent" => IconName::Bot.path(),
        "subagent_list" => IconName::GalleryVerticalEnd.path(),
        _ => LiumaIcon::Sparkles.path(),
    }
}

/// 权限 chip 图标(13px;read-only→Shield / full-access→ShieldAlert /
/// 其余(含 workspace-write)→ShieldCheck)
pub fn permission_icon(mode: &str) -> Icon {
    fixed_path(permission_path(mode), 13.)
}

fn permission_path(mode: &str) -> SharedString {
    match mode {
        "read-only" => LiumaIcon::Shield,
        "full-access" => LiumaIcon::ShieldAlert,
        _ => LiumaIcon::ShieldCheck,
    }
    .path()
}

/// 模型 chip 图标(13px;名含 flash→Zap / 含 pro·reasoner→Brain /
/// 其余→Gauge)
pub fn model_icon(model: &str) -> Icon {
    fixed_path(model_path(model), 13.)
}

fn model_path(model: &str) -> SharedString {
    let m = model.to_lowercase();
    let icon = if m.contains("flash") {
        LiumaIcon::Zap
    } else if m.contains("pro") || m.contains("reasoner") {
        LiumaIcon::Brain
    } else {
        LiumaIcon::Gauge
    };
    icon.path()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 枚举全变体的 path 必须命中 LIUMA_ICONS 静态表(防加枚举忘加 SVG)
    #[test]
    fn liuma_icon_paths_all_embedded() {
        const ALL: [LiumaIcon; 65] = [
            LiumaIcon::Sparkles,
            LiumaIcon::Zap,
            LiumaIcon::Brain,
            LiumaIcon::Gauge,
            LiumaIcon::Shield,
            LiumaIcon::ShieldAlert,
            LiumaIcon::ShieldCheck,
            LiumaIcon::Pencil,
            LiumaIcon::GitBranch,
            LiumaIcon::Archive,
            LiumaIcon::ArchiveRestore,
            LiumaIcon::Trash,
            LiumaIcon::ListChecks,
            LiumaIcon::Message,
            LiumaIcon::Pin,
            LiumaIcon::PinOff,
            LiumaIcon::Settings,
            LiumaIcon::Logo,
            LiumaIcon::Download,
            LiumaIcon::Clock,
            LiumaIcon::Code,
            LiumaIcon::CircleHelp,
            LiumaIcon::Target,
            LiumaIcon::Briefcase,
            LiumaIcon::Workflow,
            LiumaIcon::Infinity,
            LiumaIcon::Monitor,
            LiumaIcon::AgentPreset,
            LiumaIcon::RefreshCw,
            LiumaIcon::Wrench,
            LiumaIcon::Paperclip,
            LiumaIcon::Database,
            LiumaIcon::FolderTree,
            LiumaIcon::FileImage,
            LiumaIcon::FileCode,
            LiumaIcon::FileArchive,
            LiumaIcon::FileBraces,
            LiumaIcon::FileCog,
            LiumaIcon::FileDiff,
            LiumaIcon::FileKey,
            LiumaIcon::FileLock,
            LiumaIcon::FileTerminal,
            LiumaIcon::FileSpreadsheet,
            LiumaIcon::FileVideoCamera,
            LiumaIcon::FileVolume,
            LiumaIcon::FileSymlink,
            LiumaIcon::FileChartColumn,
            LiumaIcon::FileType,
            LiumaIcon::TextWrap,
            LiumaIcon::SearchOutline,
            LiumaIcon::Personalization,
            LiumaIcon::ProjectAdd,
            LiumaIcon::NewChat,
            LiumaIcon::FolderOpen,
            LiumaIcon::FolderClose,
            LiumaIcon::Trajectory,
            LiumaIcon::Mcp,
            LiumaIcon::Decision,
            LiumaIcon::Hook,
            LiumaIcon::Models,
            LiumaIcon::Box,
            LiumaIcon::General,
            LiumaIcon::CommandPrompt,
            LiumaIcon::Think,
            LiumaIcon::LoaderRing,
        ];
        for icon in ALL {
            let p = icon.path();
            if p.starts_with("icons/_liuma/") {
                // 自研路径:必须在静态表内且字节非空(防加变体忘加 SVG)
                assert!(
                    LIUMA_ICONS
                        .iter()
                        .any(|(ep, data)| p == *ep && !data.is_empty()),
                    "{icon:?} path {p:?} 未在 LIUMA_ICONS 静态表或内容为空"
                );
            } else {
                // 上游同源路径:必须能从 gpui-kit 全量内嵌资产加载
                // (防上游改名后枚举悬空指向空白)
                let hit = gpui_kit::assets::AllAssets
                    .load(&p)
                    .expect("上游资产查询不应失败")
                    .is_some_and(|d| !d.is_empty());
                assert!(hit, "{icon:?} path {p:?} 在上游内嵌资产中不存在");
            }
        }
        // 反向:静态表条目也应对得上某个枚举变体(防残留孤儿资产)
        for (ep, _) in LIUMA_ICONS {
            assert!(
                ALL.iter().any(|i| i.path().as_ref() == *ep),
                "静态表条目 {ep:?} 无对应枚举变体"
            );
        }
        // 接管清单必须是自研名单的子集(改道目标必有自有字节),
        // 且解析器确已生效(回归锁:IconName 路径全局画 liuma 形)
        for name in LIUMA_OVERRIDES {
            assert!(
                LIUMA_ONLY.contains(name),
                "接管项 {name} 不在 LIUMA_ONLY 自研名单内"
            );
        }
    }

    /// 解析器接管:清单内上游路径改道自有变体,其余原样放行;
    /// 未安装时一切路径原样(测试壳不装 resolver,生产在 init 后装)
    #[gpui_kit::test]
    fn icon_resolver_redirects_only_overrides(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(
                gpui_kit::component::resolve_icon_path("icons/folder-open.svg".into(), cx),
                "icons/folder-open.svg",
                "未安装时路径应原样"
            );
            install_icon_resolver(cx);
            assert_eq!(
                gpui_kit::component::resolve_icon_path("icons/folder-open.svg".into(), cx),
                "icons/_liuma/folder-open.svg",
                "接管清单内路径应改道自有变体"
            );
            assert_eq!(
                gpui_kit::component::resolve_icon_path("icons/wrench.svg".into(), cx),
                "icons/_liuma/wrench.svg"
            );
            assert_eq!(
                gpui_kit::component::resolve_icon_path("icons/archive.svg".into(), cx),
                "icons/archive.svg",
                "同源路径应原样放行"
            );
            assert_eq!(
                gpui_kit::component::resolve_icon_path("icons/_liuma/logo.svg".into(), cx),
                "icons/_liuma/logo.svg",
                "自有路径不应二次改道"
            );
        });
    }

    /// 工具名映射:12 个 spec 工具 + web 遗留别名 + 兜底,逐条断言资产路径
    #[test]
    fn tool_icon_covers_all_spec_tools() {
        // (工具名, 期望资产路径);前 12 项 = liuma-tools spec 权威域
        // (shell 工具的模型面名字随平台走)
        let expected: &[(&str, &str)] = &[
            (
                liuma_sandbox::shell::tool_name(),
                "icons/_liuma/command-prompt.svg",
            ),
            ("file_read", "icons/book-open.svg"),
            ("file_edit", "icons/file-pen.svg"),
            ("file_search", "icons/search.svg"),
            ("todo_write", "icons/list-checks.svg"),
            ("exit_plan_mode", "icons/list-checks.svg"),
            ("goal", "icons/target.svg"),
            ("jobs", "icons/briefcase.svg"),
            ("workflow", "icons/workflow.svg"),
            ("ralph", "icons/infinity.svg"),
            ("present", "icons/box.svg"),
            ("subagent", "icons/bot.svg"),
            ("subagent_list", "icons/gallery-vertical-end.svg"),
            // 遗留/扩展别名
            ("read", "icons/book-open.svg"),
            ("web_fetch", "icons/book-open.svg"),
            ("edit", "icons/file-pen.svg"),
            ("write", "icons/file-pen.svg"),
            ("grep", "icons/search.svg"),
            ("glob", "icons/search.svg"),
            ("web_search", "icons/globe.svg"),
            ("ask", "icons/_liuma/circle-help.svg"),
            ("code", "icons/code.svg"),
            // 兜底
            ("mystery-tool", "icons/sparkles.svg"),
        ];
        for (name, path) in expected {
            assert_eq!(tool_path(name).as_ref(), *path, "工具 {name} 图标映射不符");
        }
    }

    /// 合并资产源:自有命中 + 内置回落
    #[test]
    fn merged_assets_fallback() {
        let own = MergedAssets
            .load("icons/sparkles.svg")
            .expect("自有图标应可加载");
        assert!(own.is_some());
        let builtin = MergedAssets
            .load("icons/arrow-up.svg")
            .expect("内置图标应经回落加载");
        assert!(builtin.is_some());
        // 全量目录回落:组件默认 106 清单外的 lucide 图标也能加载
        // (历史教训:默认源只嵌清单内文件,清单外静默空白)
        let catalog = MergedAssets
            .load("icons/sparkles.svg")
            .expect("目录级图标应经 AllAssets 回落加载");
        assert!(catalog.is_some());
        // list 合并两边
        let names = MergedAssets.list("icons/_liuma/").expect("list 不应失败");
        assert_eq!(names.len(), LIUMA_ICONS.len());
    }

    /// 权限 / 模型两组语义映射 + trio 模式图标路径
    #[test]
    fn preset_permission_model_mapping() {
        assert_eq!(permission_path("read-only").as_ref(), "icons/shield.svg");
        assert_eq!(
            permission_path("workspace-write").as_ref(),
            "icons/shield-check.svg"
        );
        assert_eq!(
            permission_path("full-access").as_ref(),
            "icons/shield-alert.svg"
        );

        assert_eq!(model_path("gemini-2.5-flash").as_ref(), "icons/zap.svg");
        assert_eq!(model_path("deepseek-reasoner").as_ref(), "icons/brain.svg");
        assert_eq!(model_path("DeepSeek-V4-Pro").as_ref(), "icons/brain.svg");
        assert_eq!(model_path("deepseek-v4").as_ref(), "icons/gauge.svg");
        // trio preset 模式图标
        assert_eq!(
            LiumaIcon::AgentPreset.path().as_ref(),
            "icons/_liuma/agent-preset.svg"
        );
    }
}
