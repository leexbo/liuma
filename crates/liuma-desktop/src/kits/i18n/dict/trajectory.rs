//! 轨迹页文案词典(工具栏/时间线图例/检查器页签与信息行;原为英文
//! chrome,本词典补 zh 值,en 逐字保留既有字面)。

use crate::kits::i18n::entries;

entries! {
    // ── 工具栏 ──
    /// 投影切换:时长
    toolbar_duration => ["时长", "Duration"],
    /// 投影切换:轮次
    toolbar_turns => ["轮次", "Turns"],
    /// 投影切换:调用
    toolbar_calls => ["调用", "Calls"],

    // ── 台账 kind 徽章(en 保持既有大写)──
    /// 徽章:系统
    kind_system => ["系统", "SYSTEM"],
    /// 徽章:用户
    kind_user => ["用户", "USER"],
    /// 徽章:上下文
    kind_context => ["上下文", "CONTEXT"],
    /// 徽章:已压缩
    kind_compacted => ["已压缩", "COMPACTED"],
    /// 徽章:助手
    kind_assistant => ["助手", "ASSISTANT"],
    /// 徽章:工具
    kind_tool => ["工具", "TOOL"],
    /// 徽章:决策(无调用可挂的回合级/会话级裁决独立成行)
    kind_decision => ["决策", "DECISION"],
    /// 助手行占位(无文本仅有工具调用的轮;liuma-core 自产展示文案)
    tool_call_only => ["(仅工具调用)", "(tool call only)"],
    /// 压缩行兜底(摘要为空时 liuma-core 自产展示文案)
    compact_fallback => ["上下文已压缩", "Context compacted"],
    /// 请求 Result 行:产出为压缩记录
    request_result_compacted => ["已压缩", "Compacted"],
    /// 请求 Result 行:产出为助手回复
    request_result_assistant => ["助手回复", "Assistant Message"],
    /// 请求标(Request #N;时间线头与层级行)
    request_n(n) => ["请求 #{n}", "Request #{n}"],

    // ── 时间线图例 ──
    /// 泳道:输入
    legend_input => ["输入", "Input"],
    /// 泳道:模型
    legend_model => ["模型", "Model"],
    /// 泳道:工具
    legend_tools => ["工具", "Tools"],
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

    // ── 轮标 ──
    /// 轮标:第 n 轮
    /// 轮标:第 n 轮
    turn_n(t) => ["第 {t} 轮", "Turn {t}"],
    /// 轮标 + 定位段(段为线上数据 Step N,逐字拼接)
    turn_at(t, at) => ["第 {t} 轮 · {at}", "Turn {t} · {at}"],
    /// 轮标 + 消息(无 Step 归属)
    turn_message(t) => ["第 {t} 轮 · 消息", "Turn {t} · Message"],
    /// 两轮之间
    between_turns => ["轮间", "Between turns"],

    // ── 检查器页签 ──
    /// 页签:调用参数
    tab_payload => ["调用参数", "Payload"],
    /// 页签:结果
    tab_result => ["结果", "Result"],
    /// 页签:计时
    tab_timing => ["计时", "Timing"],
    /// 页签:原始
    tab_raw => ["原始", "Raw"],
    /// 页签:用量
    tab_usage => ["用量", "Usage"],
    /// 页签:系统提示词
    tab_system_prompt => ["系统提示词", "System Prompt"],
    /// 页签:预览
    tab_preview => ["预览", "Preview"],
    /// 页签:来源
    tab_source => ["来源", "Source"],
    /// 页签:工具
    tab_tools => ["工具", "Tools"],
    /// 页签:Diff(开发通用词,两语言同形)
    tab_diff => ["Diff", "Diff"],
    /// 页签:Schema(开发通用词,两语言同形)
    tab_schema => ["Schema", "Schema"],
    /// 页签:摘要(兜底)
    tab_summary => ["摘要", "Summary"],
    /// 页签:决策(守卫裁决挂在其裁决的工具调用上)
    tab_decision => ["决策", "Decision"],
    /// 页签:折叠(压缩的审计事实:触发/压力/门槛/遮蔽范围/裁定)
    tab_fold => ["折叠", "Fold"],

    // ── 决策记录 ──
    /// 场景名:守卫(工具执行前拦截)
    scenario_guard => ["守卫", "Guard"],
    /// 场景名:哨兵(回合收尾前核对证据)
    scenario_stop => ["哨兵", "Stop"],
    /// 场景名:裁判(上下文修剪)
    scenario_context => ["裁判", "Context"],
    /// 场景名:咨询(模型主动调用 decide 工具)
    scenario_tool => ["咨询", "Ask"],
    /// 场景名:折叠价值裁定(压缩时裁定旧输出去留)
    scenario_fold => ["折叠裁定", "Fold judge"],
    /// 守卫裁决:放行
    verdict_proceed => ["放行", "Proceed"],
    /// 守卫裁决:拦下
    verdict_block => ["拦下", "Block"],
    /// 裁决失败(fail-open 放行)
    verdict_failed => ["已失败", "Failed"],
    /// 信息行:场景
    row_scenario => ["场景", "Scenario"],
    /// 信息行:裁决
    row_verdict => ["裁决", "Verdict"],
    /// 信息行:问题
    row_questions => ["问题", "Questions"],
    /// 信息行:应答
    row_answers => ["应答", "Answers"],
    /// 信息行:状态摘要
    row_state_digest => ["状态摘要", "State digest"],
    /// 信息行:实际修剪(仅裁判 enforce 生效时在场)
    row_pruned => ["已修剪", "Pruned"],
    /// 修剪条数
    pruned_count(n) => ["{n} 条工具输出", "{n} tool outputs"],
    /// 决策页缺省(记录无裁决数据)
    no_decision => ["No decision recorded", "No decision recorded"],

    // ── 折叠事实(台账 compacted 行的审计面)──
    /// 信息行:触发来源
    row_trigger => ["触发", "Trigger"],
    /// 触发来源:自动(上下文压力越阈值)
    trigger_auto => ["自动", "Auto"],
    /// 触发来源:手动(/compact)
    trigger_manual => ["手动", "Manual"],
    /// 触发来源:溢出(provider 报上下文超长)
    trigger_overflow => ["溢出", "Overflow"],
    /// 触发来源:未知(旧日志无此字段)
    trigger_unknown => ["未知", "Unknown"],
    /// 信息行:压力(折叠前的上下文量测)
    row_pressure => ["压力", "Pressure"],
    /// 信息行:压力门槛(自动折叠触发线)
    row_threshold => ["门槛", "Threshold"],
    /// 信息行:保留尾预算(逐字保留的下限)
    row_retain => ["保留尾", "Retain tail"],
    /// 信息行:遮蔽范围(被折进摘要的事件区间)
    row_shadowed => ["遮蔽范围", "Shadowed"],
    /// 遮蔽范围值(seq 区间)
    seq_range(a, b) => ["seq {a}–{b}", "seq {a}–{b}"],
    /// 信息行:折叠条目数
    row_fold_items => ["折叠条目", "Items"],
    /// 折叠条目数
    fold_items_count(n) => ["{n} 条", "{n} items"],
    /// 信息行:价值裁定(候选/评估/裁掉)
    row_judge => ["价值裁定", "Value judge"],
    /// 价值裁定缺省(本次没有候选,未询问)
    judge_none => ["未裁定", "Not judged"],
    /// 信息行:折叠前缀 token
    row_prefix_tokens => ["前缀 Token", "Prefix tokens"],
    /// 信息行:价值裁定计数(评估/判无价值/已裁)
    judge_line(judged, total, no_value, pruned) => ["评估 {judged}/{total} · 判无价值 {no_value} · 已裁 {pruned}", "Judged {judged}/{total} · disposable {no_value} · dropped {pruned}"],
    /// 信息行:未评估候选(超单次问数上限的部分)
    judge_unjudged(n) => ["（另有 {n} 条未评估）", " ({n} unjudged)"],
    /// 折叠页缺省(记录无折叠事实)
    no_fold => ["No fold recorded", "No fold recorded"],
    /// 台账行 chip:折叠条目 + 前缀 token
    fold_chip(items, tokens) => ["{items} 条 · {tokens} tok", "{items} items · {tokens} tok"],
    /// 台账行 chip:已生效裁掉条数后缀
    fold_chip_pruned(n) => [" · 裁 {n}", " · {n} dropped"],
    /// 时间线折叠带:区间起点早于已载窗口(不从 0 假装完整)
    fold_band_clip => ["更早", "earlier"],

    // ── 检查器信息行 ──
    /// 信息行:来源
    row_source => ["来源", "Source"],
    /// 信息行:层级(无所属请求时的列名)
    row_hierarchy => ["层级", "Hierarchy"],
    /// 信息行:状态
    row_status => ["状态", "Status"],
    /// 状态值:已完成
    status_completed => ["已完成", "Completed"],
    /// 信息行:时长
    row_duration => ["时长", "Duration"],
    /// 信息行:Token 数
    row_tokens => ["Token", "Tokens"],
    /// 信息行:推理段(缩进两空格对齐)
    row_reasoning => ["   推理", "   Reasoning"],
    /// 信息行:内容段(缩进对齐)
    row_content => ["   内容", "   Content"],
    /// 信息行:总计
    row_total => ["总计", "Total"],
    /// 信息行:生成
    row_generation => ["生成", "Generation"],
    /// 信息行:吞吐
    row_throughput => ["吞吐", "Throughput"],
    /// 计时来源:会话时间戳
    timing_session => ["会话时间戳", "Session timestamps"],
    /// 计时来源:不可用
    timing_na => ["不可用", "Not available"],

    // ── 状态值与补充行 ──
    /// 状态值:开始时刻
    row_started => ["开始", "Started"],
    /// 信息行:输出
    row_output => ["输出", "Output"],
    /// 信息行:缓存段(缩进对齐)
    row_cached => ["   缓存", "   Cached"],
    /// 信息行:其他段(缩进对齐)
    row_other => ["   其他", "   Other"],
    /// 信息行:结果(请求摘要跳转行)
    row_result => ["结果", "Result"],
    /// 信息行:提供方
    row_provider => ["提供方", "Provider"],
    /// 信息行:模型
    row_model => ["模型", "Model"],
    /// 状态值:失败
    status_failed => ["失败", "Failed"],
    /// 状态值:进行中(工具无结果)
    status_pending => ["进行中", "Pending"],
    /// 小节:请求计时
    sec_request_timing => ["请求计时", "Request Timing"],
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
