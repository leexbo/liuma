//! 轨迹页文案词典(工具栏/时间线图例/检查器页签与信息行;原为英文
//! chrome,本词典补 zh 值,en 逐字保留既有字面)。

use crate::kits::i18n::entries;

entries! {
    // ── 工具栏(dsh toolbar.* 同款)──
    /// 投影切换:时长
    toolbar_duration => ["Duration", "时长"],
    /// 投影切换:轮次
    toolbar_turns => ["Turns", "轮次"],
    /// 投影切换:调用
    toolbar_calls => ["Calls", "调用"],

    // ── 台账 kind 徽章(dsh kind.* 同款;en 保持既有大写)──
    /// 徽章:系统
    kind_system => ["SYSTEM", "系统"],
    /// 徽章:用户
    kind_user => ["USER", "用户"],
    /// 徽章:上下文
    kind_context => ["CONTEXT", "上下文"],
    /// 徽章:已压缩
    kind_compacted => ["COMPACTED", "已压缩"],
    /// 徽章:助手
    kind_assistant => ["ASSISTANT", "助手"],
    /// 徽章:工具
    kind_tool => ["TOOL", "工具"],
    /// 助手行占位(无文本仅有工具调用的轮;liuma-core 自产展示文案)
    tool_call_only => ["(仅工具调用)", "(tool call only)"],
    /// 请求标(Request #N;时间线头与层级行)
    request_n(n) => ["Request #{n}", "请求 #{n}"],

    // ── 时间线图例 ──
    /// 泳道:输入
    legend_input => ["Input", "输入"],
    /// 泳道:模型
    legend_model => ["Model", "模型"],
    /// 泳道:工具
    legend_tools => ["Tools", "工具"],
    /// 拖选交互提示
    drag_hint => ["拖选筛选 · 滚轮缩放 · 右键清除", "Drag to filter · scroll to zoom · right-click to clear"],

    // ── 记录头与空态 ──
    /// 记录计数(可见 / 总数 · 请求次数)
    counts(shown, total, requests) => ["{shown} / {total} 条 · {requests} 次请求", "{shown} / {total} entries · {requests} requests"],
    /// 加载更早记录中
    loading_older => ["正在加载更早记录…", "Loading earlier events…"],
    /// 加载更早记录钮(还有 n 条)
    load_older(n) => ["加载更早记录(还有 {n} 条)", "Load {n} earlier entries"],
    /// 折叠中
    folding => ["正在折叠轨迹…", "Folding trajectory…"],
    /// 空态(无事件)
    empty => ["会话尚无事件——发送首条消息后生成轨迹", "No events yet — the trajectory appears after the first message"],
    /// 折叠组摘要(步 + 工具调用数)
    folded_steps(steps, tools) => ["… {steps} 步 · {tools} 个工具调用", "… {steps} steps · {tools} tool calls"],
    /// 折叠组摘要(工具调用 + 名称清单)
    folded_tools(count, names) => ["… {count} 个工具调用 · {names}", "… {count} tool calls · {names}"],

    // ── 轮标(dsh kind.* 语境)──
    /// 轮标:第 n 轮
    turn_n(t) => ["Turn {t}", "第 {t} 轮"],    /// 轮标 + 定位段(段为线上数据 Step N,逐字拼接)
    turn_at(t, at) => ["Turn {t} · {at}", "第 {t} 轮 · {at}"],
    /// 轮标 + 消息(无 Step 归属)
    turn_message(t) => ["Turn {t} · Message", "第 {t} 轮 · 消息"],
    /// 两轮之间
    between_turns => ["Between turns", "轮间"],

    // ── 检查器页签 ──
    /// 页签:调用参数
    tab_payload => ["Payload", "调用参数"],
    /// 页签:结果
    tab_result => ["Result", "结果"],
    /// 页签:计时
    tab_timing => ["Timing", "计时"],
    /// 页签:原始
    tab_raw => ["Raw", "原始"],
    /// 页签:用量
    tab_usage => ["Usage", "用量"],
    /// 页签:系统提示词(dsh tab.systemPrompt 同款)
    tab_system_prompt => ["System Prompt", "系统提示词"],
    /// 页签:预览
    tab_preview => ["Preview", "预览"],
    /// 页签:来源
    tab_source => ["Source", "来源"],
    /// 页签:工具
    tab_tools => ["Tools", "工具"],
    /// 页签:Diff(开发通用词,两语言同形)
    tab_diff => ["Diff", "Diff"],
    /// 页签:Schema(开发通用词,两语言同形)
    tab_schema => ["Schema", "Schema"],
    /// 页签:摘要(兜底)
    tab_summary => ["Summary", "摘要"],

    // ── 检查器信息行 ──
    /// 信息行:来源
    row_source => ["Source", "来源"],
    /// 信息行:层级(无所属请求时的列名)
    row_hierarchy => ["Hierarchy", "层级"],
    /// 信息行:状态
    row_status => ["Status", "状态"],
    /// 状态值:已完成
    status_completed => ["Completed", "已完成"],
    /// 信息行:时长(dsh timing.duration 同款)
    row_duration => ["Duration", "时长"],
    /// 信息行:Token 数
    row_tokens => ["Tokens", "Token"],
    /// 信息行:推理段(缩进两空格对齐)
    row_reasoning => ["   Reasoning", "   推理"],
    /// 信息行:内容段(缩进对齐)
    row_content => ["   Content", "   内容"],
    /// 信息行:总计
    row_total => ["Total", "总计"],
    /// 信息行:生成
    row_generation => ["Generation", "生成"],
    /// 信息行:吞吐
    row_throughput => ["Throughput", "吞吐"],
    /// 计时来源:会话时间戳
    timing_session => ["Session timestamps", "会话时间戳"],
    /// 计时来源:不可用
    timing_na => ["Not available", "不可用"],

    // ── 状态值与补充行 ──
    /// 状态值:开始时刻
    row_started => ["Started", "开始"],
    /// 信息行:输出
    row_output => ["Output", "输出"],
    /// 信息行:缓存段(缩进对齐)
    row_cached => ["   Cached", "   缓存"],
    /// 信息行:其他段(缩进对齐)
    row_other => ["   Other", "   其他"],
    /// 信息行:结果(请求摘要跳转行)
    row_result => ["Result", "结果"],
    /// 信息行:提供方
    row_provider => ["Provider", "提供方"],
    /// 信息行:模型
    row_model => ["Model", "模型"],
    /// 状态值:失败
    status_failed => ["Failed", "失败"],
    /// 状态值:进行中(工具无结果)
    status_pending => ["Pending", "进行中"],
    /// 小节:请求计时
    sec_request_timing => ["Request Timing", "请求计时"],
    /// 预览缺席:未捕获调用参数
    no_payload => ["No payload captured", "No payload captured"],
    /// 预览缺席:未捕获结果
    no_result => ["No result captured", "No result captured"],
    /// 预览缺席:Schema 不可用
    schema_na => ["Schema unavailable", "Schema unavailable"],
    /// 用量缺席
    usage_na => ["Usage not reported", "Usage not reported"],
    /// 用量组:本次请求
    usage_this => ["This request", "This request"],
    /// 用量组:会话累计
    usage_cumulative => ["Session cumulative", "Session cumulative"],
}
