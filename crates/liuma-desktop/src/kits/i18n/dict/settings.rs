//! 设置页文案词典(批次 0 = 语言行;其余分区随批次 1 迁入)。

use crate::kits::i18n::entries;

entries! {
    /// 语言行标题
    language => ["语言", "Language"],
    /// 语言选项显示名(原文名恒定,两语言下同值;dsh 约定)
    lang_zh => ["中文", "中文"],
    /// 语言选项显示名(原文名恒定)
    lang_en => ["English", "English"],
}
