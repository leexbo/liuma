//! 长尾文案词典(搜索/子代理/反馈/文件类型渲染器/模态 chrome;
//! 片段成长后再按切片拆分)。

use crate::kits::i18n::entries;

entries! {
    // ── 搜索 ──
    /// 全库检索命中计数
    hits_full(n) => ["全库检索 · {n} 条命中", "Search · {n} hits"],
    /// 返回列表
    back_to_list => ["返回列表", "Back to list"],
    /// 无命中
    no_hits => ["无命中", "No hits"],

    // ── 子代理 ──
    /// 子代理面板标题
    subagents_title => ["子代理", "Subagents"],
    /// 运行中计数
    running_n(n) => ["{n} 个运行中", "{n} running"],
    /// 已结束
    ended => ["已结束", "Ended"],
    /// 主线(相对子代理)
    mainline => ["主线", "Main"],
    /// 打断(运行中子代理)
    interrupt => ["打断", "Interrupt"],

    // ── 反馈 ──
    /// 反馈输入占位
    feedback_ph => ["这条回答哪里好，或哪里有问题？（可选）", "What was good or bad about this answer? (optional)"],
    /// 补充说明
    supplement => ["补充说明", "Additional comments"],

    // ── 文件类型渲染器(预览标题)──
    /// 渲染器:纯文本
    renderer_text => ["纯文本", "Plain text"],
    /// 渲染器:图片
    renderer_image => ["图片", "Image"],
    /// 渲染器:代码
    renderer_code => ["代码", "Code"],

    // ── 模态 chrome ──
    /// 添加工作区(菜单项,接选择器)
    add_workspace_ellipsis => ["添加工作区…", "Add workspace…"],
    /// 重命名会话模态标题
    rename_session => ["重命名会话", "Rename session"],
}
