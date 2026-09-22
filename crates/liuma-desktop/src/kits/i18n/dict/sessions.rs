//! 会话列表文案词典(侧栏头/行菜单/视图选项/导出与工作区通知)。

use crate::kits::i18n::entries;

entries! {
    /// 新会话钮 / 空白会话兜底标题
    new_session => ["新会话", "New session"],
    /// 头部分组态:扁平会话
    group_flat => ["会话", "Sessions"],
    /// 头部分组态:按工作区
    group_by_ws => ["工作区", "Workspaces"],
    /// 搜索会话(图标钮提示 + 展开输入占位)
    search_ph => ["搜索会话", "Search sessions"],
    /// 视图选项(图标钮提示)
    view_options => ["视图选项", "View options"],
    /// 添加工作区(图标钮提示)
    add_workspace => ["添加工作区", "Add workspace"],
    /// 归档聊天(图标钮提示)
    tip_archive => ["归档聊天", "Archive chat"],

    /// 行菜单:重命名(会话/工作区共用)
    rename => ["重命名", "Rename"],
    /// 行菜单:归档
    archive => ["归档", "Archive"],
    /// 行菜单:分叉
    fork => ["分叉", "Fork"],
    /// 行菜单:导出日志
    export_log => ["导出日志", "Export log"],
    /// 行菜单:删除工作区
    delete_workspace => ["删除工作区", "Delete workspace"],

    /// 视图菜单:分组方式组头
    group_label => ["分组方式", "Group by"],
    /// 分组:按工作区
    by_workspace => ["按工作区", "By workspace"],
    /// 分组:单列表
    single_list => ["单列表", "Single list"],
    /// 视图菜单:排序方式组头
    sort_label => ["排序方式", "Sort by"],
    /// 排序:最近更新
    recent_updates => ["最近更新", "Recently updated"],
    /// 排序:手动(拖拽序)
    manual_sort => ["手动排序", "Manual"],

    /// 子代理计数(one/other 手动复数;zh 同值)
    subagents_one(n) => ["{n} 个子代理", "{n} subagent"],
    /// 子代理计数(one/other 手动复数)
    subagents_other(n) => ["{n} 个子代理", "{n} subagents"],

    // ── 通知与错误 ──
    /// 历史加载失败
    history_load_failed(detail) => ["历史加载失败:{detail}", "Failed to load history: {detail}"],
    /// 工作区目录选择器:任务失败
    picker_task_failed(e) => ["选择器任务失败:{e}", "Picker task failed: {e}"],
    /// 工作区目录选择器:通道失败
    picker_channel_failed => ["选择器通道失败", "Picker channel failure"],
    /// 添加工作区失败
    add_workspace_failed(msg) => ["添加工作区失败:{msg}", "Failed to add workspace: {msg}"],
    /// 无法打开目录选择
    open_dir_failed(msg) => ["无法打开目录选择:{msg}", "Couldn't open the directory picker: {msg}"],
    /// 重命名输入占位:工作区
    ws_title_ph => ["工作区标题", "Workspace title"],
    /// 移除工作区失败
    remove_failed(msg) => ["移除失败:{msg}", "Remove failed: {msg}"],
    /// 重命名输入占位:会话
    session_title_ph => ["会话标题", "Session title"],
    /// 重命名失败
    rename_failed(msg) => ["重命名失败:{msg}", "Rename failed: {msg}"],
    /// 分叉失败
    fork_failed(msg) => ["分叉失败:{msg}", "Fork failed: {msg}"],
    /// 导出失败通知标题
    export_failed => ["导出失败", "Export failed"],
    /// 保存对话框打开失败
    save_dialog_failed(e) => ["保存对话框打开失败:{e}", "Failed to open the save dialog: {e}"],
    /// 导出成功通知:zip(含分叉后代)
    exported_zip => ["已导出(含分叉后代)", "Exported (with fork descendants)"],
    /// 导出失败:日志不可读
    export_unreadable => ["导出失败:会话日志不可读", "Export failed: session log unreadable"],
    /// 导出成功通知:单文件
    exported_single => ["已导出(单文件)", "Exported (single file)"],
    /// 写入失败
    write_failed(e) => ["写入失败:{e}", "Write failed: {e}"],
}
