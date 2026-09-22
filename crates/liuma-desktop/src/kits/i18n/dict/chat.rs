//! 聊天区文案词典(批次 2 先落 composer 占位;其余随批次 4 迁入)。

use crate::kits::i18n::entries;

entries! {
    /// composer 占位(标准态)
    composer_standard => ["输入消息,Enter 发送 / Shift+Enter 换行", "Message, Enter to send / Shift+Enter for a new line"],
    /// composer 占位(计划模式态)
    composer_plan => ["描述你的任务以生成计划", "Describe your task to generate a plan"],
}
