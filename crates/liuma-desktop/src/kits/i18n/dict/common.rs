//! 跨切片通用词汇(通用按钮/状态)。
//! 专有文案进所属切片词典;两处以上复用才上浮到这里。

use crate::kits::i18n::entries;

entries! {
    /// 保存
    save => ["保存", "Save"],
    /// 取消
    cancel => ["取消", "Cancel"],
    /// 应用
    apply => ["应用", "Apply"],
    /// 编辑
    edit => ["编辑", "Edit"],
    /// 移除(条目级:从清单摘除;删除存储数据另有词条)
    remove => ["移除", "Remove"],
    /// 卸载(桥/server 级摘除)
    uninstall => ["卸载", "Uninstall"],
    /// 确认(复杂承诺的兜底确认钮;纯知会收尾键随 modals 迁移再入)
    confirm => ["确认", "Confirm"],
    /// 复制
    copy => ["复制", "Copy"],
    /// 复制成功(点击后瞬时反馈)
    copied => ["复制成功", "Copied"],
    /// 收起(展开态回落)
    collapse => ["收起", "Collapse"],
    /// 重试
    retry => ["重试", "Retry"],
    /// 确定(纯知会收尾)
    ok => ["确定", "OK"],
}
