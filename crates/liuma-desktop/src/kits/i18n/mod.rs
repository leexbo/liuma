//! 桌面 UI 文案:**唯一系统 = rust-i18n**(与 `gpui-component` 共用同一个
//! 进程级 locale,故库自产文案——输入框右键菜单 `Input.Cut/Copy/Paste/
//! Select All`、`Select.placeholder`、`Questionnaire.*` 等——随同一档位切换)。
//!
//! 设计政策(形态为本仓约定):
//! - **locale-owned copy**:只翻译产品 chrome;用户/模型/线上数据逐字
//!   渲染,永不进文案文件(轨迹 `Step N`、宿主错误串、会话内容等);
//! - **整句成键**:模板带全部占位一次性翻译,禁拼接翻译碎片(设计指南
//!   `Internationalization` 节);复数为手动 `_one`/`_other` 键对,调用方
//!   按 `n == 1` 择一,不做 CLDR;
//! - **唯一切换面 = 设置页**:初始档读 settings.yaml `language`(缺省
//!   `zh-CN`),不做 OS locale 探测;切换经 [`apply`] 全窗即时生效。
//!
//! 文案存 `crates/liuma-desktop/locales/*.yml`(文件 stem 即 key 根段),
//! 取值唯一入口是本模块再导出的 [`t!`](rust_i18n::t)。rust-i18n 的缺省
//! locale 是内建 `"en"`,而本仓缺省档是 `zh-CN`,故 `t!` 包装了一层
//! [`ensure_locale`]:进程内首次取文案前把缺省档落进全局(测试零装配即
//! `zh-CN`,与迁移前的中文断言一致)。
//!
//! 渲染是状态纯函数,文案取值随全局 locale 自动换档,[`apply`] 以
//! `refresh_windows` 一步生效,无需逐实体 notify;并发测试不触语言盘
//! (同 theme 先例),切换路径由门禁 + 手动冒烟覆盖。

use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::App;

pub(crate) mod backend;

// ── 语言档位(locale id 即 settings.yaml `language` 取值)──

/// 支持的界面语言:`(locale id, 原文名)`。
///
/// 显示名恒为原文名(`中文` / `English`,两档同显),故不进文案文件——
/// 语言选项的"双语同值条目"这一畸形由本表的构造消除。
pub(crate) const LOCALES: &[(&str, &str)] = &[("zh-CN", "中文"), ("en", "English")];

/// 缺省语言(settings.yaml `language` 缺省值同此)
pub(crate) const DEFAULT: &str = "zh-CN";

/// 是否已显式配置档位(`init` / `apply`);置位后 [`ensure_locale`] 空转
static CONFIGURED: AtomicBool = AtomicBool::new(false);

/// locale 标签的主语言段(小写;`zh-Hant-TW` → `zh`)
fn primary(tag: &str) -> String {
    tag.split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// locale id 归一:精确命中 → 原始 id;否则按 rfc4647 主语言段前缀回退
/// (`zh` / `zh-Hans` / `zh-Hant-TW` → `zh-CN`,`en-US` → `en`);不识别 →
/// [`DEFAULT`]。这是对历史/手改配置的防御,不是兼容层。
pub(crate) fn normalize(id: &str) -> &'static str {
    let want = primary(id);
    LOCALES
        .iter()
        .find(|(loc, _)| primary(loc) == want)
        .map_or(DEFAULT, |(loc, _)| loc)
}

/// 缺省档落地(进程内首次取文案前一次)。
///
/// rust-i18n 的全局 locale 内建缺省是 `"en"`;测试与任何未经 [`init`] 的
/// 路径都必须看到 [`DEFAULT`]。`Once` 同时给出与后续读取的 happens-before:
/// 所有取值入口(本模块的 `t!` 包装与 [`pick`])先经此处,故不可能读到
/// 内建缺省。
pub(crate) fn ensure_locale() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if !CONFIGURED.load(Ordering::Relaxed) {
            rust_i18n::set_locale(DEFAULT);
        }
    });
}

/// 当前 locale id(归一后,恒为 [`LOCALES`] 之一;零分配)。
///
/// 读档薄包装(`kits/fmt.rs` 等)用它把全局档位显式传给 `_l` 档核:先
/// [`ensure_locale`] 再读,故首个取值点也不会读到 rust-i18n 的内建 `"en"`。
pub(crate) fn current_locale() -> &'static str {
    ensure_locale();
    normalize(&rust_i18n::locale())
}

/// 启动装配:开窗前读 settings.yaml 持久化档(main.rs run 闭包内,
/// 与 theme::apply 并排)
pub(crate) fn init(id: &str) {
    CONFIGURED.store(true, Ordering::Relaxed);
    rust_i18n::set_locale(normalize(id));
}

/// 切换语言并全窗即时生效(档位未变幂等)。切换后由渲染期
/// `sync_locale_ui` 回写挂窗态(偏好下拉标签重建等,见 settings store)。
pub(crate) fn apply(id: &str, cx: &mut App) {
    let id = normalize(id);
    CONFIGURED.store(true, Ordering::Relaxed);
    if &*rust_i18n::locale() == id {
        return;
    }
    rust_i18n::set_locale(id);
    cx.refresh_windows();
}

/// 取文案的唯一入口:`t!("chat.composer.standard", cut = key)`。
///
/// 包装 `rust_i18n::t!` 只为在取值前落地缺省档(见 [`ensure_locale`]);
/// `Once` 完成后是纯原子读,无分配。键/占位与 locale 文件的闭环由
/// `scripts/verify-desktop-i18n` 门禁守(缺键会被 rust-i18n 原样显示为
/// 键名,故合入前必须拦下)。
macro_rules! t {
    ($($args:tt)*) => {{
        $crate::kits::i18n::ensure_locale();
        rust_i18n::t!($($args)*)
    }};
}

pub(crate) use t;

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 locale 取真实译文(走本 crate 的 rust-i18n backend)
    fn translate(locale: &str, key: &str) -> String {
        backend::_rust_i18n_translate(locale, key).into_owned()
    }

    /// 语言表:缺省档在场、id 唯一、显示名非空
    #[test]
    fn locales_table_is_wellformed() {
        assert_eq!(LOCALES.len(), 2);
        assert!(
            LOCALES.iter().any(|(id, _)| *id == DEFAULT),
            "缺省档须在语言表内"
        );
        for (id, name) in LOCALES {
            assert!(!id.is_empty() && !name.is_empty(), "空 id / 空显示名");
        }
        let mut ids: Vec<&str> = LOCALES.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), LOCALES.len(), "locale id 重复");
    }

    /// 归一:精确命中 / rfc4647 主语言段回退 / 未知回落缺省
    #[test]
    fn normalize_by_primary_subtag() {
        assert_eq!(normalize("zh-CN"), "zh-CN");
        assert_eq!(normalize("zh"), "zh-CN");
        assert_eq!(normalize("zh-Hans"), "zh-CN");
        assert_eq!(normalize("zh-Hant-TW"), "zh-CN");
        assert_eq!(normalize("ZH_cn"), "zh-CN");
        assert_eq!(normalize("en"), "en");
        assert_eq!(normalize("en-US"), "en");
        assert_eq!(normalize("EN"), "en");
        assert_eq!(normalize("fr"), DEFAULT);
        assert_eq!(normalize(""), DEFAULT);
    }

    /// 缺省档 = zh-CN:零装配(测试进程未调 init)即中文;en 侧经
    /// `locale =` 显式档断言,不写全局 locale(并发测试无竞争)
    #[test]
    fn default_locale_is_zh_cn() {
        ensure_locale();
        assert_eq!(t!("common.cancel", locale = DEFAULT), "取消");
        assert_eq!(t!("common.save"), "保存");
        assert_eq!(t!("common.save", locale = "en"), "Save");
    }

    /// 库层文案契约(缺陷回归锁):库自产文案与产品文案共用同一个
    /// rust-i18n 全局 locale。此前 liuma 从不调用 `set_locale`,库自产
    /// 文案恒英文(最可见的是输入框右键菜单 Cut/Copy/Paste/Select All)。
    /// 逐个断言 liuma 暴露的库 key 在 zh-CN / en 两侧都有译文,且不是
    /// "缺键回落键名"——上游改名/漏译即红。
    #[test]
    fn library_copy_contract_both_locales() {
        use gpui_kit::component::_rust_i18n_translate as lib;
        const KEYS: &[&str] = &[
            "Input.Cut",
            "Input.Copy",
            "Input.Paste",
            "Input.Select All",
            "Select.placeholder",
            "Questionnaire.progress",
            "Questionnaire.previous",
            "Questionnaire.next",
            "Questionnaire.skip",
            "Questionnaire.submit",
            "List.search_placeholder",
        ];
        for key in KEYS {
            for locale in [DEFAULT, "en"] {
                let text = lib(locale, key);
                assert!(
                    !text.is_empty() && text.as_ref() != *key,
                    "库 key {key} 在 {locale} 档缺译文(回落键名)"
                );
            }
            assert_ne!(
                lib(DEFAULT, key).as_ref(),
                lib("en", key).as_ref(),
                "库 key {key} 两档译文相同,疑似档位未生效"
            );
        }
    }

    /// 轨迹页 zh/en 关键标签(回归锁:旧词典切片曾整表槽位反序,zh 档
    /// 工具栏显示英文原字面;经真实 backend 双档断言)
    #[test]
    fn trajectory_labels_dispatch_both_langs() {
        assert_eq!(translate(DEFAULT, "trajectory.toolbar_duration"), "时长");
        assert_eq!(translate("en", "trajectory.toolbar_duration"), "Duration");
        assert_eq!(translate(DEFAULT, "trajectory.toolbar_turns"), "轮次");
        assert_eq!(translate(DEFAULT, "trajectory.kind_tool"), "工具");
        assert_eq!(translate(DEFAULT, "trajectory.kind_assistant"), "助手");
        assert_eq!(translate("en", "trajectory.kind_assistant"), "ASSISTANT");
        assert_eq!(
            translate(DEFAULT, "trajectory.counts"),
            "%{shown} / %{total} 条 · %{requests} 次请求"
        );
        assert_eq!(
            translate("en", "trajectory.counts"),
            "%{shown} / %{total} entries · %{requests} requests"
        );
        assert_eq!(
            translate(DEFAULT, "trajectory.tool_call_only"),
            "(仅工具调用)"
        );
        assert_eq!(
            translate("en", "trajectory.tool_call_only"),
            "(tool call only)"
        );
    }

    /// SYSTEM 行哨兵文案 zh/en(回归锁:core 自产 "Initial System Prompt"
    /// 等线上哨兵曾逐字直显在中文台账/检查器头;渲染层经 record_title
    /// 词典化,值对齐 dsh layout.*/record.*)
    #[test]
    fn system_sentinel_copy_dispatch_both_langs() {
        let pairs = [
            (
                "trajectory.initial_system_prompt",
                "初始系统提示词",
                "Initial System Prompt",
            ),
            (
                "trajectory.system_prompt_updated",
                "系统提示词已更新",
                "System Prompt Updated",
            ),
            (
                "trajectory.system_prompt_and_tools_updated",
                "系统提示词和工具已更新",
                "System Prompt and Tools Updated",
            ),
            ("trajectory.tools_updated", "工具已更新", "Tools Updated"),
            ("trajectory.no_output", "无输出", "No output"),
            (
                "trajectory.block_label",
                "块 #%{n} %{kind}",
                "Block #%{n} %{kind}",
            ),
            ("trajectory.block_1_text", "块 #1 文本", "Block #1 text"),
            ("trajectory.unnamed_tool", "（未命名）", "(unnamed)"),
            (
                "trajectory.named_parameters_json",
                "%{name} 参数 JSON",
                "%{name} parameters JSON",
            ),
        ];
        for (key, zh, en) in pairs {
            assert_eq!(translate(DEFAULT, key), zh, "zh 档 {key}");
            assert_eq!(translate("en", key), en, "en 档 {key}");
        }
    }

    /// 压缩链路文案 zh/en(回归锁:宿主 compaction/error kind=empty 的
    /// 英文常量曾逐字直显在中文界面;命令描述同理——宿主 builtin_commands
    /// 是中文原文,en 档由文案文件映射)
    #[test]
    fn compaction_and_command_copy_dispatch_both_langs() {
        assert_eq!(translate(DEFAULT, "chat.compact_empty"), "暂无可压缩的历史");
        assert_eq!(
            translate("en", "chat.compact_empty"),
            "No compactable history yet."
        );
        assert_eq!(translate(DEFAULT, "chat.compact_running"), "正在压缩…");
        assert_eq!(translate("en", "chat.compact_title"), "Context compacted");
        // 命令描述:zh = 宿主原文(逐字),en 侧不得残留中文
        assert_eq!(
            translate(DEFAULT, "chat.command_compact"),
            "压缩以上对话内容"
        );
        for key in [
            "chat.command_compact",
            "chat.command_plan",
            "chat.command_export",
            "chat.command_goal",
        ] {
            let desc = translate("en", key);
            assert!(
                !desc.chars().any(|ch| ('一'..='鿿').contains(&ch)),
                "en 档命令描述不得含中文: {desc:?}"
            );
        }
        assert_eq!(
            translate(DEFAULT, "trajectory.compact_fallback"),
            "上下文已压缩"
        );
        assert_eq!(
            translate("en", "trajectory.compact_fallback"),
            "Context compacted"
        );
        assert_eq!(
            translate("en", "trajectory.request_result_compacted"),
            "Compacted"
        );
        assert_eq!(
            translate(DEFAULT, "trajectory.request_result_assistant"),
            "助手回复"
        );
    }
}
