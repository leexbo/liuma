//! 界面文案 i18n(zh / en):零依赖词典 + 全局语言盘。
//!
//! 设计照搬 dsh(deepseek-harness `packages/client/locale`)的既定政策,
//! 形态按本仓约定重塑:
//! - **locale-owned copy**:只翻译产品 chrome;用户/模型/线上数据逐字
//!   渲染,永不进词典(轨迹 `Step N`、宿主错误串、会话内容等);
//! - **zh 键集权威**:`entries!` 一键一行,zh/en 相邻声明——缺 en =
//!   编译错,键拼错 = 编译错,双语对齐由构造保证(强于 dsh 的双文件
//!   类型对齐);
//! - **整句成键**:模板带全部占位一次性翻译,禁拼接翻译碎片(设计指南
//!   `Internationalization` 节);复数为手动 `_one`/`_other` 键对,调用方
//!   按 `n == 1` 择一,不做 CLDR;
//! - **唯一切换面 = 设置页**:初始档读 settings.yaml `language`(缺省
//!   zh),不做 OS locale 探测;切换经 [`apply`] 全窗即时生效。
//!
//! 语言盘镜像 [`crate::kits::theme`] 的进程级原子范式:渲染热路径 =
//! 一次 Relaxed load(纯键返回 `&'static str`,零分配;模板键分配量与
//! 迁移前 `format!` 持平)。渲染是状态纯函数,词典取值随盘自动换档,
//! [`apply`] 以 `refresh_windows` 一步生效,无需逐实体 notify。
//! 测试零装配即 zh——既有中文断言与布局测试不迁移;并发测试不触语言
//! 盘(同 theme 先例,切换路径由 liuma-core 回归锁 + 手动冒烟覆盖)。

use std::sync::atomic::{AtomicU8, Ordering};

use gpui_kit::App;

pub(crate) mod dict;

// ── 语言档位(与 registry settings.yaml `language` 字段同词汇)──

/// 界面语言(判别值即声明序 as u8,勿重排)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    /// 中文(缺省)
    #[default]
    Zh = 0,
    /// English
    En = 1,
}

impl Lang {
    /// settings 值 → 档位(未知值回落中文,与缺省一致;写入侧由
    /// registry 白名单校验,读入侧回落即可,无需启动清洗)
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("en") {
            Self::En
        } else {
            Self::Zh
        }
    }

    /// 档位 → settings 值
    pub fn id(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

/// 语言盘(0 = zh 缺省 / 1 = en)。测试零装配即 zh。
static LANG: AtomicU8 = AtomicU8::new(0);

/// 当前语言档
#[inline]
pub(crate) fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::Zh
    }
}

/// 启动装配:开窗前读 settings.yaml 持久化档(main.rs run 闭包内,
/// 与 theme::apply 并排)
pub(crate) fn init(id: &str) {
    LANG.store(Lang::parse(id) as u8, Ordering::Relaxed);
}

/// 切换语言并全窗即时生效(档位未变幂等)。切换后由渲染期
/// `sync_locale_ui` 回写挂窗态(偏好下拉标签重建等,见 settings store)。
pub(crate) fn apply(l: Lang, cx: &mut App) {
    if LANG.load(Ordering::Relaxed) == l as u8 {
        return;
    }
    LANG.store(l as u8, Ordering::Relaxed);
    cx.refresh_windows();
}

// ── 取值 ──

/// 纯分派锚点(测试直测;[`pick`] 即「读盘 + 本函数」)
#[inline]
pub(crate) fn pick_lang(l: Lang, zh: &'static str, en: &'static str) -> &'static str {
    match l {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

/// 双语取值(词典纯键生成 fn 的底层):一次 Relaxed load
#[inline]
pub(crate) fn pick(zh: &'static str, en: &'static str) -> &'static str {
    pick_lang(lang(), zh, en)
}

/// `{name}` 命名占位插值(词典模板键生成 fn 的底层;`{{` 转义字面
/// `{`)。debug 构建断言「模板占位名集合 == 实参名集合」——调用点占位
/// 拼错在测试期暴露;release 只按命中替换,未命中占位原样保留。
///
/// dead_code 临时豁免:首个模板键随批次 1 落地前,bin 目标无调用点;
/// 届时移除本 allow(与 template_placeholders 同期)。
#[allow(dead_code)]
pub(crate) fn fmt(
    template: &'static str,
    args: &[(&'static str, &dyn std::fmt::Display)],
) -> String {
    #[cfg(debug_assertions)]
    {
        let mut names = template_placeholders(template);
        names.sort_unstable();
        let mut provided: Vec<&str> = args.iter().map(|(n, _)| *n).collect();
        provided.sort_unstable();
        assert_eq!(names, provided, "i18n 模板占位与实参不一致:{template:?}");
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        if let Some(tail) = after.strip_prefix("{{") {
            out.push('{');
            rest = tail;
            continue;
        }
        let Some(close) = after.find('}') else {
            out.push_str(after);
            rest = "";
            break;
        };
        let name = &after[1..close];
        match args.iter().find(|(n, _)| *n == name) {
            Some((_, value)) => {
                use std::fmt::Write as _;
                let _ = write!(out, "{value}");
            }
            // 未命中:原样保留(调用点拼错在 debug 测试期已拦截)
            None => out.push_str(&after[..=close]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// 模板占位名提取(`{name}`;`{{` 转义不计;debug 断言与词典对齐测试用)
///
/// dead_code 临时豁免同 [`fmt`]:首个模板键(批次 1)前无调用点。
#[cfg(debug_assertions)]
#[allow(dead_code)]
fn template_placeholders(template: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open..];
        if let Some(tail) = after.strip_prefix("{{") {
            rest = tail;
            continue;
        }
        let Some(close) = after.find('}') else {
            break;
        };
        names.push(&after[1..close]);
        rest = &after[close + 1..];
    }
    names
}

// ── 词典宏(条目 = 取值 fn;键名即 fn 名)──

/// 词典条目宏:一键一行,zh/en 相邻声明。
///
/// - 纯文案:`title => ["语言", "Language"]` → `fn title() -> &'static str`
/// - 模板:`save_failed(msg) => ["保存失败:{msg}", "Save failed: {msg}"]`
///   → `fn save_failed(msg: impl Display) -> String`(占位名 = 参数名)
///
/// 约束:zh/en 只接受字面量(同入 `TEMPLATES` 元数据表,供占位对齐
/// 测试);键名须为合法 fn 标识符;zh 值与迁移前字面量逐字节一致;键名
/// 避开保留字(必要时 `_title`/`_label` 后缀区分)。
macro_rules! entries {
    ($(
        $(#[$doc:meta])*
        $name:ident $(($($ph:ident),+))? => [$zh:expr, $en:expr] $(,)?
    )+) => {
        $(
            $crate::kits::i18n::entry! {
                $(#[$doc])*
                $name $(($($ph),+))? => [$zh, $en]
            }
        )+
        /// 词典元数据:(键名, zh 模板, en 模板)。占位对齐测试与
        /// 完整性门禁的数据源;勿手写。
        #[allow(dead_code)]
        pub(crate) const TEMPLATES: &[(&str, &str, &str)] = &[
            $((stringify!($name), $zh, $en)),+
        ];
    };
}

/// 单条目展开(纯文案 / 模板两形态;仅由 [`entries!`] 内部按形态分派)
macro_rules! entry {
    ($(#[$doc:meta])* $name:ident => [$zh:expr, $en:expr]) => {
        $(#[$doc])*
        #[inline]
        pub fn $name() -> &'static str {
            $crate::kits::i18n::pick($zh, $en)
        }
    };
    ($(#[$doc:meta])* $name:ident ($($ph:ident),+) => [$zh:expr, $en:expr]) => {
        $(#[$doc])*
        #[inline]
        pub fn $name($($ph: impl core::fmt::Display),+) -> String {
            $crate::kits::i18n::fmt(
                $crate::kits::i18n::pick($zh, $en),
                &[$((stringify!($ph), &$ph)),+],
            )
        }
    };
}

pub(crate) use entries;
pub(crate) use entry;

#[cfg(test)]
mod tests {
    use super::*;

    /// 档位解析与 registry settings.yaml 词汇一致,未知值回落中文
    #[test]
    fn lang_parse_matches_registry() {
        assert_eq!(Lang::parse("zh"), Lang::Zh);
        assert_eq!(Lang::parse("en"), Lang::En);
        assert_eq!(Lang::parse("EN"), Lang::En, "大小写不敏感");
        assert_eq!(Lang::parse("fr"), Lang::Zh, "未知值回落中文");
        assert_eq!(Lang::parse(""), Lang::Zh);
    }

    /// 档位 ↔ settings 值往返
    #[test]
    fn lang_id_roundtrip() {
        for l in [Lang::Zh, Lang::En] {
            assert_eq!(Lang::parse(l.id()), l);
        }
    }

    /// 纯分派:档位取对应语言,两语言同键
    #[test]
    fn pick_lang_dispatches() {
        assert_eq!(pick_lang(Lang::Zh, "中文", "English"), "中文");
        assert_eq!(pick_lang(Lang::En, "中文", "English"), "English");
    }

    /// 缺省盘 = zh:零装配读取纯键得 zh 值(不触盘写,与并发测试无竞争)
    #[test]
    fn default_lang_is_zh() {
        assert_eq!(lang(), Lang::Zh);
    }

    /// 插值:命名占位按名替换,实参顺序与模板顺序无关
    #[test]
    fn fmt_substitutes_by_name() {
        let out = fmt(
            "已压缩 {n} 条(约 {t} tokens)",
            &[("t", &12u64), ("n", &3u64)],
        );
        assert_eq!(out, "已压缩 3 条(约 12 tokens)");
        let en = fmt("Compacted {n} entries", &[("n", &1u64)]);
        assert_eq!(en, "Compacted 1 entries");
    }

    /// 插值:`{{` 转义字面 `{`,`}}` 无需转义(不在扫描面)
    #[test]
    fn fmt_escapes_literal_braces() {
        let out = fmt("{{name}}: {name}", &[("name", &"值".to_string())]);
        assert_eq!(out, "{name}}: 值");
    }

    /// 插值:占位集与实参集不一致 → debug 构建断言拦截
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "i18n 模板占位与实参不一致")]
    fn fmt_asserts_placeholder_set_parity() {
        let _ = fmt("共 {n} 条", &[("count", &1u64)]);
    }

    /// release 语义:未命中占位原样保留(debug 断言另有专测,故此测
    /// 仅 release 编入)
    #[cfg(not(debug_assertions))]
    #[test]
    fn fmt_keeps_unmatched_placeholder_verbatim() {
        let out = fmt("共 {n} 条", &[("count", &1u64)]);
        assert_eq!(out, "共 {n} 条");
    }

    /// 词典对齐门禁:每个切片词典内,模板键的 zh/en 占位名集合逐一相等
    /// (新切片词典落地时登记到本表)。附带断言两语言值均非空。
    #[test]
    fn dict_templates_placeholder_parity() {
        type Tpl = (&'static str, &'static str, &'static str);
        let dicts: &[(&str, &[Tpl])] = &[("settings", dict::settings::TEMPLATES)];
        for &(module, entries) in dicts {
            for (key, zh, en) in entries {
                assert!(!zh.is_empty() && !en.is_empty(), "{module}.{key} 空文案");
                let mut zp = template_placeholders(zh);
                let mut ep = template_placeholders(en);
                zp.sort_unstable();
                ep.sort_unstable();
                assert_eq!(zp, ep, "{module}.{key} zh/en 占位不一致: {zh:?} vs {en:?}");
            }
        }
    }
}
