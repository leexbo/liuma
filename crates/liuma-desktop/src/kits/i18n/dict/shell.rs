//! shell 底座文案词典(标题栏/状态栏/hero/面板标签/全局通知)。

use crate::kits::i18n::entries;

entries! {
    // ── 标题栏 ──
    /// 标题栏运行中活动后缀(全角竖线分隔;en 用 ASCII 竖线)
    run_label_suffix(label) => ["｜{label}", " | {label}"],
    /// 标题栏提示:切换侧边栏
    tip_toggle_sidebar => ["切换侧边栏", "Toggle sidebar"],
    /// 标题栏提示:显示/隐藏侧边面板
    tip_toggle_panel => ["显示/隐藏侧边面板", "Show or hide the side panel"],

    // ── 面板标签 ──
    /// 面板标签:计划
    plan_tab => ["计划", "Plan"],
    /// 面板标签:轨迹
    trajectory_tab => ["轨迹", "Trajectory"],
    /// 面板标签:文件
    files_tab => ["文件", "Files"],
    /// 面板标签:预览(文件名缺失兜底)
    preview_tab => ["预览", "Preview"],
    /// 面板计划页空态
    plan_empty => ["当前会话暂无计划。", "No plan for this session yet."],
    /// 计划状态:待批准
    plan_pending => ["待批准", "Pending"],
    /// 计划状态:已批准
    plan_approved => ["已批准", "Approved"],
    /// 计划状态:已拒绝
    plan_declined => ["已拒绝", "Declined"],
    /// 计划状态:已取消
    plan_cancelled => ["已取消", "Cancelled"],

    // ── hero(空会话输入卡)──
    /// hero 标语
    hero_tagline => ["木牛流马,替你驮活", "The wooden ox that hauls your work"],
    /// hero 缺 key 引导
    hero_no_key => ["尚未配置 API key——前往设置", "No API key yet — open Settings"],

    // ── 全局挂窗输入 ──
    /// onboarding 输入占位(shell 侧)
    api_key_input => ["输入 API 密钥", "Enter API key"],
    /// 轨迹搜索占位
    search_ph => ["搜索", "Search"],

    // ── 全局通知 ──
    /// 模式切换失败(宿主回执)
    mode_switch_failed(msg) => ["模式切换失败:{msg}", "Failed to switch mode: {msg}"],
    /// 模式切换:通道失败
    mode_channel_failed => ["模式切换:通道失败", "Mode switch: channel failure"],
    /// 权限切换失败(宿主回执;权限/会话两路共用)
    switch_failed(msg) => ["切换失败:{msg}", "Switch failed: {msg}"],
    /// 权限切换:通道失败
    permission_channel_failed => ["权限切换:通道失败", "Permission switch: channel failure"],

    // ── 状态栏 ──
    /// 统计 pill:轮次与步数(dsh stats.counts 同款)
    stats_counts(turns, steps) => ["{turns} 轮 {steps} 步", "{turns} turns {steps} steps"],
    /// 统计 pill 后缀:缓存命中
    stats_cache_hit_pct(hit) => [" · 缓存命中 {hit}%", " · cache hit {hit}%"],
    /// 统计卡标题
    stats_title => ["会话统计", "Session stats"],
    /// 统计卡行:模型用时
    stats_model_time => ["模型用时", "Model time"],
    /// 统计卡行:工具调用用时
    stats_tool_time => ["工具调用用时", "Tool time"],
    /// 统计卡行:首 token 平均
    stats_ttft => ["首 token 平均（TTFT）", "Average TTFT"],
    /// 统计卡行:输出速度
    stats_tps => ["输出速度（TPS）", "Output speed (TPS)"],
    /// 统计卡行:缓存命中
    stats_cache_hit => ["缓存命中", "Cache hit"],
    /// 统计卡行:未缓存输入
    stats_uncached => ["未缓存输入", "Uncached input"],
    /// 统计卡行:缓存读取
    stats_cache_read => ["缓存读取", "Cache read"],
    /// 统计卡行:缓存写入
    stats_cache_write => ["缓存写入", "Cache write"],
    /// 统计卡行:输出
    stats_output => ["输出", "Output"],
    /// 统计卡行:Token 用量
    stats_token_usage => ["Token 用量", "Token usage"],
}
