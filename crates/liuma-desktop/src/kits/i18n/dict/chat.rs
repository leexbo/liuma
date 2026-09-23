//! 聊天区文案词典(消息流/工具卡/composer/子代理/队列/todo/终端查看器/
//! mermaid 查看器 chrome)。

use crate::kits::i18n::entries;

entries! {
    // ── composer ──
    /// composer 占位(标准态)
    composer_standard => ["输入消息,Enter 发送 / Shift+Enter 换行", "Message, Enter to send / Shift+Enter for a new line"],
    /// composer 占位(计划模式态)
    composer_plan => ["描述你的任务以生成计划", "Describe your task to generate a plan"],
    /// 附件选择器提示
    pick_attachment => ["选择附件", "Choose attachments"],
    /// @引用分区:文件与文件夹
    section_files => ["文件与文件夹", "Files and folders"],
    /// @引用分区:Session 对话
    section_sessions => ["Session 对话", "Session chats"],
    /// @引用分区:命令
    section_commands => ["命令", "Commands"],
    /// @引用分区:技能
    section_skills => ["技能", "Skills"],
    /// 引用/下拉分区与菜单项:模型
    model_label => ["模型", "Model"],
    /// 模型菜单分区:推理等级
    reasoning_level => ["推理等级", "Reasoning effort"],
    /// skill 引用行前缀(仅用户可见)
    user_only(desc) => ["仅用户 · {desc}", "User-only · {desc}"],
    /// 上下文计量行
    ctx_used_pct(percent) => ["上下文已用 {percent}%", "Context {percent}% used"],
    /// 上下文构成行(份额)
    tok_share(v, pct) => ["{v} · {pct}%", "{v} · {pct}%"],
    /// 模型菜单空态
    no_models => ["各 Provider 尚未配置模型。", "No models configured for any provider."],
    /// 附件拒收 toast(命令不接受附件;has = 附件类别全称,remove = 短称)
    attach_rejected(kind, has, remove) => ["/{kind} 不接受{has},请先移除{remove}", "/{kind} doesn't accept {has}; remove {remove} first"],
    /// 附件类别全称:文件
    attach_file_full => ["文件附件", "file attachments"],
    /// 附件类别全称:图片
    attach_image_full => ["图片附件", "image attachments"],
    /// 附件类别短称:文件
    attach_file => ["文件", "file"],
    /// 附件类别短称:图片
    attach_image => ["图片", "image"],
    /// 权限徽标:仅可查看
    perm_read_only => ["仅可查看", "Read-only"],
    /// 权限徽标:完全权限
    perm_full_access => ["完全权限", "Full access"],
    /// 权限徽标:工作区内修改(缺省)
    perm_workspace_write => ["工作区内修改", "Workspace edits"],

    // ── 上下文计量图例 ──
    /// 图例:系统提示
    ctx_system => ["系统提示", "System prompt"],
    /// 图例:工具定义
    ctx_tools => ["工具定义", "Tool definitions"],
    /// 图例:会话消息
    ctx_messages => ["会话消息", "Session messages"],

    // ── 消息流状态行 ──
    /// 消息选择复制菜单项
    copy_menu => ["复制", "Copy"],
    /// 会话内容加载中
    loading_history => ["正在加载会话内容…", "Loading session…"],
    /// 压缩进行中标记行
    compact_running => ["正在压缩…", "Compacting…"],
    /// 压缩排队标记行
    compact_queued => ["已排队,回合结束后压缩", "Queued — compaction starts after this turn"],
    /// 深入探索中(回合计时延长提示)
    exploring => ["深入探索中…", "Digging deeper…"],
    /// Think 折叠行标签
    think_label => ["思考", "Think"],
    /// 折叠轮标:工具调用数
    tool_calls_count(n) => ["{n} 次工具调用", "{n} tool calls"],
    /// 折叠轮标兜底(无工具调用的纯思考轮)
    thought_fallback => ["已思考", "Thought for a while"],
    /// 折叠轮标消息计数
    messages_count(n) => ["{n} 条消息", "{n} messages"],
    /// 助手正文尾「已停止」pill(回合被中断)
    message_stopped => ["已停止", "Stopped"],
    /// 压缩标记行标题
    compact_title => ["上下文已压缩", "Context compacted"],
    /// 通告行标题
    turn_failed => ["本轮运行失败", "This turn failed"],
    /// 召回行前缀
    recall(label) => ["召回·{label}", "Recall · {label}"],
    /// 注入行前缀
    inject(label) => ["注入·{label}", "Inject · {label}"],
    /// 压缩完成统计行
    compaction_done(n, t) => ["已压缩 {n} 条历史记录（约 {t} tokens）", "Compacted {n} entries (~{t} tokens)"],
    /// 压缩摘要可展开提示
    compact_summary_hint => ["点击查看压缩摘要", "Click to view the compaction summary"],
    /// 压缩摘要不可用
    compact_summary_na => ["压缩摘要不可用", "Compaction summary unavailable"],
    /// 已中断(回合被打断)
    interrupted => ["已中断", "Interrupted"],
    /// 插队条目徽标(待投递)
    queue_jump => ["插队 · 待投递", "Steered · pending delivery"],

    // ── 子代理 ──
    /// 子代理回发消息行标
    subagent_message => ["子代理·消息", "Subagent · message"],
    /// 子代理终止:已停止
    subagent_stopped => ["子代理·已停止", "Subagent · stopped"],
    /// 子代理终止:已恢复(中断后恢复)
    subagent_resumed => ["子代理·已恢复", "Subagent · resumed"],
    /// 子代理终止:已失败
    subagent_failed => ["子代理·已失败", "Subagent · failed"],
    /// 子代理终止:已完成
    subagent_done => ["子代理·已完成", "Subagent · completed"],
    /// 子代理无收尾消息兜底
    no_closing => ["无收尾消息", "No closing message"],
    /// 查看子会话(跳转钮)
    view_subsession => ["查看子会话", "View subagent"],

    // ── 轮尾统计卡 ──
    /// 轮尾用量 pill
    usage_tok(v) => ["用量 {v} tok", "Usage {v} tok"],
    /// 轮尾用时 pill
    time_run(v) => ["用时 {v}", "Time {v}"],
    /// 轮尾卡:提供方 / 模型行标
    provider_model => ["提供方 / 模型", "Provider / model"],
    /// 轮尾卡:缓存命中(同状态栏词汇,复用值)
    stats_cache_hit => ["缓存命中", "Cache hit"],
    /// 轮尾卡:未缓存输入
    stats_uncached => ["未缓存输入", "Uncached input"],
    /// 轮尾卡:缓存读取
    stats_cache_read => ["缓存读取", "Cache read"],
    /// 轮尾卡:缓存写入
    stats_cache_write => ["缓存写入", "Cache write"],
    /// 轮尾卡:输出
    stats_output => ["输出", "Output"],
    /// 轮尾卡:输出含推理后缀
    reasoning_suffix(v) => ["（其中推理 {v} tok）", " (incl. {v} reasoning tok)"],
    /// 轮尾卡:本轮用量标题
    turn_usage => ["本轮用量", "Turn usage"],
    /// 轮尾卡:用时与速度标题
    turn_time_speed => ["本轮用时和速度", "Turn time & speed"],
    /// 轮尾卡:总用时行标
    turn_total_time => ["本轮总用时", "Total turn time"],
    /// 轮尾卡:输出速度(同状态栏词汇)
    turn_tps => ["输出速度（TPS）", "Output speed (TPS)"],
    /// 轮尾卡:首 token 用时(措辞异于状态栏「平均」)
    turn_ttft => ["首 token 用时（TTFT）", "Time to first token (TTFT)"],
    /// 轮尾卡:产物区标题
    artifacts => ["产物", "Artifacts"],
    /// 产物区更多文件计数
    more_files(n) => ["+ {n} 个文件", "+{n} files"],

    // ── 重试行 ──
    /// 重试行标题(member_row 标题槽)
    retry_title => ["模型重试", "Model retry"],
    /// 重试:正在重试(存活连接)
    retry_waiting_live => ["正在重试模型请求", "Retrying model request"],
    /// 重试:等待重试(连接已断)
    retry_waiting => ["等待重试模型请求", "Waiting to retry model request"],
    /// 重试:已重试
    retry_started => ["已重试模型请求", "Model request retried"],
    /// 重试:已取消
    retry_cancelled => ["模型请求重试已取消", "Model request retry cancelled"],
    /// 重试状态行(label = 上述四态;attempt 计数 + 秒)
    retry_status(label, attempt, max, seconds) => ["{label}（{attempt}/{max}） · {seconds}s", "{label} ({attempt}/{max}) · {seconds}s"],
    /// 重试详情:延迟
    retry_delay(ms) => ["重试延迟：{ms}ms", "Retry delay: {ms}ms"],
    /// 重试详情:失败原因
    retry_reason(msg) => ["失败原因：{msg}", "Failure reason: {msg}"],

    // ── 通告与投影 ──
    /// 宿主消息缺席兜底
    unknown_error => ["未知错误", "Unknown error"],
    /// todo_write 行摘要
    todo_done(done, total) => ["{done}/{total} 已完成", "{done}/{total} done"],

    // ── 工具卡 ──
    /// 工具卡行:todo_write 标题
    todo_write_title => ["更新任务清单", "Update to-dos"],
    /// 工具行标题:未知工具兜底(原名进摘要前缀)
    generic_tool => ["工具调用", "Tool call"],
    /// 工具行标题:shell 族
    tool_bash => ["Bash", "Bash"],
    /// 工具行标题:读取
    tool_read => ["读取", "Read"],
    /// 工具行标题:写入
    tool_write => ["写入", "Write"],
    /// 工具行标题:编辑
    tool_edit => ["编辑", "Edit"],
    /// 工具行标题:内容搜索(原样)
    tool_grep => ["Grep", "Grep"],
    /// 工具行标题:文件名匹配(原样)
    tool_glob => ["Glob", "Glob"],
    /// 工具行标题:文件搜索
    tool_search => ["搜索", "Search"],
    /// 工具行标题:网页搜索
    tool_web_search => ["网页搜索", "Web search"],
    /// 工具行标题:网页获取
    tool_web_fetch => ["网页获取", "Web fetch"],
    /// 工具行标题:读取图片
    tool_read_image => ["读取图片", "Read image"],
    /// 工具行标题:代码
    tool_code => ["代码", "Code"],
    /// 工具行标题:向用户提问
    tool_ask => ["提问", "Ask"],
    /// 工具行标题:后台任务
    tool_jobs => ["任务", "Jobs"],
    /// 工具行标题:目标管理
    tool_goal => ["目标", "Goal"],
    /// 工具行标题:工作流
    tool_workflow => ["工作流", "Workflow"],
    /// 工具行标题:子代理调用
    tool_subagent => ["子代理", "Subagent"],
    /// 工具行标题:子代理列表
    tool_subagent_list => ["子代理列表", "Subagents"],
    /// 工具行标题:退出计划模式
    tool_exit_plan => ["退出计划", "Exit plan"],
    /// 工具行标题:ralph 循环(专名不译)
    tool_ralph => ["Ralph", "Ralph"],
    /// 空输出占位
    empty_output => ["(空)", "(empty)"],
    /// read 卡截断提示(shown/total 行)
    show_rows(shown, total) => ["显示 {shown} / {total} 行", "Showing {shown} of {total} lines"],
    /// 展开钮回落(其余 hidden 行;one/other 手动复数)
    more_rows_one(hidden) => ["… 其余 {hidden} 行", "… {hidden} more line"],
    /// 展开钮回落(其余 hidden 行;one/other 手动复数)
    more_rows_other(hidden) => ["… 其余 {hidden} 行", "… {hidden} more lines"],
    /// 搜索卡计数(截断态)
    show_total(shown, total) => ["显示 {shown} / 共 {total}", "Showing {shown} of {total}"],
    /// 搜索卡摘要(count = 计数段;files = 文件数)
    match_summary(count, files) => ["{count} 处匹配 · {files} 个文件", "{count} matches · {files} files"],
    /// 路径搜索摘要
    paths_summary(count) => ["{count} 个路径", "{count} paths"],
    /// 搜索无结果
    no_results => ["无结果", "No results"],
    /// diff 统计行(s = 复数尾,files == 1 时空串)
    diff_stat(added, removed, files, s) => ["└ +{added} -{removed} · {files} file{s}", "└ +{added} -{removed} · {files} file{s}"],
    /// skill 加载失败兜底
    skill_failed => ["skill 加载失败", "Skill failed to load"],
    /// skill 加载中
    skill_loading => ["正在加载 skill", "Loading skill"],
    /// 查看(子会话卡钮)
    view => ["查看", "View"],

    // ── 队列坞 / todo 坞 ──
    /// 队列计数标
    queue_count(n) => ["队列 {n} 条", "{n} queued"],
    /// todo 坞计数标
    todo_counts(done, active, pending) => ["{done} 完成 · {active} 进行 · {pending} 待办", "{done} done · {active} active · {pending} pending"],

    // ── 终端查看器 ──
    /// 终端失败:信号终止
    signal(sig) => ["信号 {sig}", "signal {sig}"],
    /// 终端失败:非零退出
    exit_code(c) => ["退出码 {c}", "exit code {c}"],
    /// 终端空输出占位
    no_output => ["无输出", "No output"],

    // ── mermaid 查看器 chrome ──
    /// mermaid 页签:图表
    mermaid_chart => ["图表", "Chart"],
    /// mermaid 页签:代码
    mermaid_code => ["代码", "Code"],
    /// mermaid 下载钮
    mermaid_download => ["下载", "Download"],
    /// mermaid 放大钮
    mermaid_zoom => ["放大", "Zoom in"],
    /// mermaid 导出成功通知
    mermaid_exported(p) => ["已导出:{p}", "Exported: {p}"],
    /// mermaid 导出失败通知
    mermaid_export_failed(e) => ["导出失败:{e}", "Export failed: {e}"],

    // ── 会话操作(store 域;措辞与 sessions 切片异文)──
    /// 队列操作失败
    queue_op_failed(msg) => ["队列操作失败:{msg}", "Queue op failed: {msg}"],
    /// 会话分支失败(分支 ≠ 分叉:branch 走 fork 端点的新话术)
    branch_failed(msg) => ["分支失败:{msg}", "Branch failed: {msg}"],
    /// 导出数据解码失败
    export_decode_failed => ["导出数据解码失败", "Failed to decode export data"],
    /// 导出成功通知标题(短式)
    exported => ["已导出", "Exported"],
    /// 模型菜单当前项前缀
    current_model(m) => ["当前: {m}", "Current: {m}"],
    /// 命令失败
    command_failed(msg) => ["命令失败:{msg}", "Command failed: {msg}"],
}

/// 工具行标题槽本地化(不暴露模型面名;标题走词典,
/// 摘要槽只放参数摘要)。None = 未知工具:渲染层以 [`generic_tool`]
/// 兜底,并把原名放进摘要前缀(`{name} · …`)。
pub fn tool_display_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "bash" | "shell" => tool_bash(),
        "file_read" | "read" => tool_read(),
        "write" => tool_write(),
        "file_edit" | "edit" => tool_edit(),
        "grep" => tool_grep(),
        "glob" => tool_glob(),
        "file_search" => tool_search(),
        "web_search" => tool_web_search(),
        "web_fetch" => tool_web_fetch(),
        "read_image" => tool_read_image(),
        "code" => tool_code(),
        "ask" => tool_ask(),
        "jobs" => tool_jobs(),
        "goal" => tool_goal(),
        "workflow" => tool_workflow(),
        "subagent" => tool_subagent(),
        "subagent_list" => tool_subagent_list(),
        "exit_plan_mode" => tool_exit_plan(),
        "ralph" => tool_ralph(),
        _ => return None,
    })
}
