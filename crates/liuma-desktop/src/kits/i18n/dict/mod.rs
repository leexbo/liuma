//! 文案词典:按调用点所在功能切片一份(dsh per-plugin namespace 同构;
//! 切片清单随迁移批次增长,落地一个登记一个)。
//!
//! `common` 只放跨切片复用词汇(通用按钮/状态),专有文案进所属切片;
//! 条目形态与约束见 super 的 `entries!` 宏文档。每个切片文件以
//! `use crate::kits::i18n::entries;` + `entries! { ... }` 声明条目。

pub(crate) mod settings;
