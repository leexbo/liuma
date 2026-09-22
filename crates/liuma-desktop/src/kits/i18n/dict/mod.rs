//! 文案词典:按调用点所在功能切片一份(dsh per-plugin namespace 同构;
//! 切片清单随迁移批次增长,落地一个登记一个)。
//!
//! `common` 只放跨切片复用词汇(通用按钮/状态),专有文案进所属切片;
//! 条目形态与约束见 super 的 `entries!` 宏文档。每个切片文件以
//! `use crate::kits::i18n::entries;` + `entries! { ... }` 声明条目。

pub(crate) mod ask;
pub(crate) mod chat;
pub(crate) mod common;
pub(crate) mod files;
pub(crate) mod misc;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod shell;
/// 时间格式词典整体豁免 dead_code:其消费面是 `kits::fmt` /
/// `shell::reducer` 的 `_l` 显式语言核(只走 `l::` 变体,读盘形态由薄
/// 包装经 `lang()` 间接到达),使用压力由双语言单测与调用点审查承担,
/// 不靠 dead-code 兜底;其余词典(直达 UI)保持逐键 dead 检查。
#[allow(dead_code)]
pub(crate) mod time;
pub(crate) mod trajectory;
