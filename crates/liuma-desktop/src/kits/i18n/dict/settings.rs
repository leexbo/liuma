//! 设置页文案词典(通用区 / MCP / Hooks 桥 / Provider 编辑器 / 计费 /
//! onboarding / 全权确认)。

use crate::kits::i18n::entries;

entries! {
    // ── 语言行(批次 0)──
    /// 语言行标题
    language => ["语言", "Language"],
    /// 语言选项显示名(原文名恒定,两语言下同值;dsh 约定)
    lang_zh => ["中文", "中文"],
    /// 语言选项显示名(原文名恒定)
    lang_en => ["English", "English"],

    // ── 导航与分区 ──
    /// 设置页标题(内容列 header)
    settings_title => ["设置", "Settings"],
    /// 导航/分区:常规(导航项与分区标题同词)
    general => ["常规", "General"],
    /// 导航:模型设置
    nav_models => ["模型设置", "Models"],
    /// 分区标题:模型与 Provider
    models_section => ["模型与 Provider", "Models & providers"],
    /// 导航分组头:基础设置
    nav_basics => ["基础设置", "Basics"],
    /// 导航/分区:关于
    about => ["关于", "About"],
    /// 返回工作区(设置页退出口)
    back_workspace => ["返回工作区", "Back to workspace"],
    /// 关于页一句话简介
    about_intro => ["流马 liuma —— Rust 桌面 agent harness。", "Liuma — a Rust desktop agent harness."],
    /// 关于页字段:版本
    version => ["版本", "Version"],

    // ── 通用区偏好行 ──
    /// Agent 预设行标题
    preset_title => ["Agent 预设", "Agent preset"],
    /// Agent 预设行说明
    preset_desc => ["对此后新建的会话生效。运行中的会话保持它开始时的预设。", "Applies to new sessions. Running sessions keep the preset they started with."],
    /// 权限行标题
    permission_title => ["权限", "Permission"],
    /// 权限行说明
    permission_desc => ["选择新会话的默认权限模式", "Default permission mode for new sessions"],
    /// 繁忙时 Enter 行为行标题
    busy_title => ["繁忙时 Enter 键行为", "Enter while busy"],
    /// 繁忙时 Enter 行为行说明
    busy_desc => ["仅在智能体运行时生效;Cmd/Ctrl+Enter 使用另一行为", "Applies while the agent is running; Cmd/Ctrl+Enter uses the other behavior"],
    /// 权限选项:仅可查看
    perm_read_only => ["仅可查看", "Read-only"],
    /// 权限选项:工作区内修改
    perm_workspace_write => ["工作区内修改", "Workspace edits"],
    /// 权限选项:完全权限
    perm_full_access => ["完全权限", "Full access"],
    /// 繁忙选项:排队发送
    busy_queue => ["排队发送", "Queue messages"],
    /// 繁忙选项:插话发送
    busy_steer => ["插话发送", "Steer current turn"],
    /// 外观分区标题
    appearance_title => ["外观", "Appearance"],
    /// 外观选项:浅色
    appearance_light => ["浅色", "Light"],
    /// 外观选项:深色
    appearance_dark => ["深色", "Dark"],
    /// 外观选项:跟随系统
    appearance_system => ["跟随系统", "System"],

    // ── Provider 列表与编辑器 ──
    /// 注册表空态
    registry_empty => ["注册表为空——「添加 Provider」,或重启后恢复内置默认。", "Registry is empty — use “Add provider”, or restart to restore built-ins."],
    /// 保存通告(成功)
    saved(name) => ["已保存 {name}", "Saved {name}"],
    /// 默认徽标
    default_badge => ["默认", "Default"],
    /// 提供方字段标签
    provider_label => ["提供方", "Provider"],
    /// Provider ID 字段标签
    provider_id_label => ["Provider ID(小写/连字符)", "Provider ID (lowercase/hyphens)"],
    /// 添加提供方(底部 dashed 钮)
    add_provider => ["添加提供方", "Add provider"],
    /// 添加自定义提供方
    add_custom_provider => ["添加自定义提供方", "Add custom provider"],
    /// 名称字段/列
    name => ["名称", "Name"],
    /// API 密钥字段标签
    api_key => ["API 密钥", "API key"],
    /// API 密钥字段标签(必填变体)
    api_key_required => ["API Key(必填)", "API key (required)"],
    /// API 密钥字段标签(自定义卡非首配形态的既有字面)
    api_key_plain => ["API Key", "API key"],
    /// 内置卡折叠区标题
    advanced_section => ["自定义设置", "Advanced"],
    /// 内置卡信息行:适配器
    adapter => ["适配器", "Adapter"],
    /// 计费预设信息行
    billing_preset => ["计费预设", "Billing preset"],
    /// 计费预设值:已内置
    billing_builtin => ["已内置", "Built in"],
    /// 计费预设值:无
    billing_none => ["无(可手填计费端点)", "None (billing endpoint can be entered manually)"],
    /// 目录条目缺失提示
    catalog_missing => ["目录条目缺失", "Catalog entry missing"],
    /// API 格式字段标签
    api_format => ["API 格式", "API format"],
    /// 模型列表字段标题
    model_list => ["模型列表", "Model list"],
    /// 模型列表空态
    models_empty => ["当前没有配置模型,添加模型后可在聊天中使用。", "No models configured yet. Add one to use it in chat."],
    /// 添加模型钮
    add_model => ["添加模型", "Add model"],
    /// 从端点获取(模型列表区钮)
    fetch_from_endpoint => ["从端点获取", "Fetch from endpoint"],
    /// 上下文窗口提示(输入框上方说明)
    ctx_hint => ["点「窗口」设置该模型的上下文 token 数(可写 256K / 1M,1M = 1,000,000);留空使用默认,仅影响自动压缩阈值与上下文计量。", "Use “Window” to set this model's context tokens (256K / 1M; 1M = 1,000,000). Leave empty for the default. Affects auto-compaction threshold and context metering only."],
    /// 上下文窗口占位
    ctx_placeholder => ["默认 1,000,000", "Default 1,000,000"],
    /// 上下文窗口输入行内提示
    ctx_invalid_hint => ["填 1 以上的整数或带单位(256K / 1M),或留空使用默认值", "Enter an integer ≥ 1, with optional unit (256K / 1M), or leave empty for the default"],
    /// 模型窗口 pill(有覆盖值)
    ctx_window(value) => ["窗口 {value}", "Window {value}"],
    /// 模型窗口 pill(缺省)
    ctx_window_default => ["窗口 默认", "Window default"],
    /// 上下文窗口通知(非法值)
    ctx_invalid_notice => ["上下文窗口需为 1 以上的整数 token 数", "Context window must be an integer ≥ 1 token"],
    /// provider id 空通知
    provider_id_empty => ["provider id 不可为空", "Provider ID can't be empty"],
    /// provider id 重复通知
    provider_id_dup => ["该 Provider ID 已存在,请在列表中编辑它", "This Provider ID already exists — edit it in the list"],
    /// 保存失败通知(宿主错误为 locale-owned 数据,逐字拼接)
    save_failed(msg) => ["保存失败:{msg}", "Save failed: {msg}"],
    /// 删除失败通知
    delete_failed(msg) => ["删除失败:{msg}", "Remove failed: {msg}"],
    /// API key 输入占位:新建(空 = 保持既有凭据)
    key_keep_placeholder => ["sk-…(留空 = 保持既有凭据)", "sk-… (empty = keep existing credential)"],
    /// API key 输入占位:内置卡
    key_enter_placeholder => ["输入 API Key", "Enter API key"],
    /// API key 输入占位:环境认证可用
    key_env_placeholder => ["输入 API 密钥,或留空使用环境认证", "Enter API key, or leave empty to use environment auth"],
    /// 表单输入占位:provider id
    id_placeholder => ["小写英文/连字符", "lowercase/hyphens"],
    /// 表单输入占位:可选项
    optional_placeholder => ["可选", "Optional"],
    /// 表单输入占位:显示名
    name_placeholder => ["给这个 Provider 起个名字", "Name this provider"],
    /// 表单输入占位:模型 id
    model_id_placeholder => ["模型 id", "model id"],
    /// 表单输入占位:计费端点 URL
    url_placeholder => ["https://…", "https://…"],

    // ── 计费 ──
    /// 计费端点字段标签
    billing_endpoint => ["计费端点(余额 / 用量展示)", "Billing endpoint (balance / usage)"],
    /// 计费形态:余额
    billing_balance => ["余额", "Balance"],
    /// 计费形态:用量
    billing_usage => ["用量", "Usage"],
    /// 查询 URL 字段标签
    query_url => ["查询 URL(GET)", "Query URL (GET)"],
    /// 5 小时用量路径
    usage_path_5h => ["5小时用量路径", "5h usage path"],
    /// 7 天用量路径
    usage_path_7d => ["7天用量路径", "7d usage path"],
    /// 重置时间路径
    reset_path => ["重置时间路径(可选)", "Reset time path (optional)"],
    /// 余额金额路径
    balance_path => ["余额金额路径", "Balance amount path"],
    /// 货币路径
    currency_path => ["货币路径(可选)", "Currency path (optional)"],
    /// 计费刷新中
    billing_refreshing => ["刷新中…", "Refreshing…"],
    /// 立即刷新
    refresh_now => ["立即刷新", "Refresh now"],
    /// 计费更新成功通告
    billing_updated => ["计费已更新", "Billing updated"],
    /// 计费查询失败通告
    billing_query_failed(msg) => ["计费查询失败:{msg}", "Billing query failed: {msg}"],
    /// 获取模型前缺 Base URL 通告
    billing_need_url => ["先填写 Base URL 再获取模型", "Fill in Base URL before fetching models"],
    /// 剩余额度前缀
    remaining => ["剩余:", "Remaining:"],
    /// 额度窗口:5 小时
    quota_5h => ["5 小时", "5h"],
    /// 额度窗口:7 天
    quota_7d => ["7 天", "7d"],
    /// 额度重置倒计时
    resets_in(cd) => ["{cd}后重置", "Resets in {cd}"],
    /// 额度有效期(天 + 时)
    quota_expiry(days, hours) => ["{days}天{hours}时", "{days}d {hours}h"],
    /// 额度有效期(时 + 分)
    quota_expiry_hm(hours, mins) => ["{hours}时{mins}分", "{hours}h {mins}m"],
    /// 额度有效期(分)
    quota_expiry_m(mins) => ["{mins}分", "{mins}m"],
    /// 计费路径占位:余额金额
    balance_path_placeholder => ["余额金额路径,如 balance_infos.0.total_balance", "Balance amount path, e.g. balance_infos.0.total_balance"],
    /// 计费路径占位:货币
    currency_path_placeholder => ["货币路径,如 balance_infos.0.currency", "Currency path, e.g. balance_infos.0.currency"],
    /// 计费路径占位:5 小时用量
    usage5h_path_placeholder => ["5小时用量路径,如 five_hour.utilization", "5h usage path, e.g. five_hour.utilization"],
    /// 计费路径占位:7 天用量
    usage7d_path_placeholder => ["7天用量路径,如 seven_day.utilization", "7d usage path, e.g. seven_day.utilization"],
    /// 计费路径占位:重置时间
    reset_path_placeholder => ["重置时间路径,如 resets_in", "Reset time path, e.g. resets_in"],

    // ── Provider 删除确认 ──
    /// 删除确认标题
    remove_provider(id) => ["移除 {id}", "Remove {id}"],
    /// 删除确认说明
    remove_provider_desc => ["移除会移除其配置和存储的 API 密钥;工作区引用回落内置默认。", "Removing also deletes its configuration and stored API key. Workspace references fall back to the built-in default."],

    // ── 模型拉取弹层 ──
    /// 拉取弹层标题
    fetch_models_title => ["从端点获取模型", "Fetch models from endpoint"],
    /// 已选计数
    picked_count(count) => ["已选 {count}", "{count} selected"],
    /// 采纳所选
    adopt(count) => ["采纳 {count} 项", "Adopt {count}"],
    /// 获取中
    fetching => ["获取中…", "Fetching…"],

    // ── MCP ──
    /// MCP 分区说明
    mcp_intro => ["外部 MCP server(stdio)的工具桥接进工具面;保存即连接,所有会话共享。", "Tools from external MCP servers (stdio) join the tool surface. Connecting starts on save and is shared across sessions."],
    /// 已安装计数(MCP 与 hooks 共用)
    installed(total) => ["已安装 {total}", "{total} installed"],
    /// 新建钮(MCP 与 hooks 共用)
    add_new => ["+ 新建", "+ New"],
    /// MCP 空态
    mcp_none => ["尚未配置 MCP server。", "No MCP servers configured."],
    /// server 状态:连接中
    status_connecting => ["连接中", "Connecting"],
    /// server 状态:重连中
    status_reconnecting => ["重连中", "Reconnecting"],
    /// server 状态:失败
    status_failed => ["失败", "Failed"],
    /// MCP 编辑卡标题
    mcp_edit_card(id) => ["编辑 MCP Server · {id}", "Edit MCP server · {id}"],
    /// MCP 编辑卡说明
    mcp_edit_desc => ["修改当前 MCP 配置,保存后返回列表。", "Edit this MCP configuration. Saving returns to the list."],
    /// MCP 新增卡标题
    mcp_new_card => ["新增 MCP Server", "New MCP server"],
    /// MCP 新增卡说明
    mcp_new_desc => ["注册新的 MCP server,保存后返回列表。", "Register a new MCP server. Saving returns to the list."],
    /// 表单区标题
    form_section => ["表单", "Form"],
    /// 完整配置切换/标题
    full_config => ["完整配置", "Full config"],
    /// 传输字段标签
    transport => ["传输", "Transport"],
    /// 请求头区说明
    headers_desc => ["请求头(可选;键 + 值,原样透传)", "Headers (optional; key + value, passed through as-is)"],
    /// 添加请求头钮
    add_header => ["+ 添加请求头", "+ Add header"],
    /// 参数区说明
    args_desc => ["参数(每个一条)", "Args (one per line)"],
    /// 添加参数钮
    add_arg => ["+ 添加参数", "+ Add arg"],
    /// 环境变量区说明
    env_desc => ["环境变量(可选;键 + 值)", "Environment variables (optional; key + value)"],
    /// 添加环境变量钮
    add_env => ["+ 添加环境变量", "+ Add env var"],
    /// MCP 超时字段标签
    timeout_ms_mcp => ["超时 MS", "Timeout (ms)"],
    /// 启用态:已启用
    enabled => ["已启用", "Enabled"],
    /// 启用态:未启用
    disabled => ["未启用", "Disabled"],
    /// MCP 导入成功通告
    mcp_imported(n) => ["已导入 {n} 个 MCP server", "Imported {n} MCP servers"],
    /// MCP 连接失败通知(shell 通知层)
    mcp_connect_failed(server, error) => ["MCP server「{server}」连接失败:{error}", "MCP server \"{server}\" failed to connect: {error}"],
    /// MCP 表单通知:id 空
    mcp_id_empty => ["id 不能为空", "ID can't be empty"],
    /// MCP 表单通知:超时非法
    mcp_timeout_invalid => ["超时 MS 须为非负整数", "Timeout must be a non-negative integer"],
    /// MCP 表单通知:http 缺 url
    mcp_need_url => ["http 传输需要 url", "http transport requires a URL"],
    /// MCP 表单通知:stdio 缺 command
    mcp_need_command => ["command 不能为空", "Command can't be empty"],
    /// MCP 表单占位:server 名
    mcp_name_placeholder => ["server 名", "server name"],
    /// MCP 表单占位:启动命令
    mcp_command_placeholder => ["启动命令(如 npx)", "Launch command (e.g. npx)"],
    /// MCP 表单占位:工作目录
    mcp_cwd_placeholder => ["工作目录(可选)", "Working directory (optional)"],

    // ── Hooks 桥 ──
    /// Hooks 分区说明
    hooks_intro => ["把既有 Claude Code / Codex hooks.json 的 command 钩子接进会话:阻塞工具与提示、附加上下文、强制续跑;变更在下次会话附着时生效。", "Bridge command hooks from existing Claude Code / Codex hooks.json into sessions: block tools and prompts, attach context, force continuation. Changes take effect when the next session attaches."],
    /// hooks 空态
    hooks_none => ["尚未配置 hooks 桥。", "No hooks bridges configured."],
    /// Hooks 编辑卡标题
    hooks_edit_card => ["编辑 Hooks 桥", "Edit hooks bridge"],
    /// Hooks 新建卡标题
    hooks_new_card => ["新建 Hooks 桥", "New hooks bridge"],
    /// 返回钮(带箭头字形)
    back_arrow => ["← 返回", "← Back"],
    /// Hooks 卡分区:方言
    dialect_section => ["方言", "Dialect"],
    /// Hooks 卡分区:启用
    enable_section => ["启用", "Enable"],
    /// Hooks 卡分区:配置
    config_section => ["配置", "Config"],
    /// Hooks 字段:路径
    path => ["路径", "Path"],
    /// Hooks 超时字段标签
    timeout_ms_hooks => ["超时 ms", "Timeout (ms)"],
    /// Hooks 表单占位:hooks.json 路径
    hooks_path_placeholder => ["hooks.json 路径", "hooks.json path"],
    /// Hooks 表单占位:插件根替换
    hooks_root_placeholder => ["可选;替换 ${CLAUDE_PLUGIN_ROOT}", "Optional; replaces ${CLAUDE_PLUGIN_ROOT}"],
    /// Hooks 表单占位:工作目录
    hooks_cwd_placeholder => ["可选;缺省 = 会话工作区", "Optional; defaults to the session workspace"],

    // ── 全权确认 ──
    /// 全权确认标题
    fa_title => ["要开启完全权限吗？", "Turn on full access?"],
    /// 全权确认正文
    fa_body => ["智能体将能够在未经您许可的情况下，在这台计算机上的任何位置运行命令、使用互联网，以及创建和编辑文件。这包括但不限于：", "The agent will be able to run commands, use the internet, and create and edit files anywhere on this computer without asking you. This includes, but is not limited to:"],
    /// 全权确认:文件项标题
    fa_files => ["文件和文件夹", "Files and folders"],
    /// 全权确认:文件项说明
    fa_files_desc => ["读取、创建、修改、上传或删除此计算机上任意位置的文件", "Read, create, modify, upload, or delete files anywhere on this computer"],
    /// 全权确认:终端项标题
    fa_terminal => ["终端命令", "Terminal commands"],
    /// 全权确认:终端项说明
    fa_terminal_desc => ["运行命令、安装软件和更改系统设置", "Run commands, install software, and change system settings"],
    /// 全权确认:互联网项标题
    fa_internet => ["互联网和已连接的应用", "Internet and connected apps"],
    /// 全权确认:互联网项说明
    fa_internet_desc => ["访问网站、发送数据并使用已启用的插件", "Visit websites, send data, and use enabled plugins"],
    /// 全权确认:风险行
    fa_risk => ["这会带来敏感数据丢失或泄露、提示注入等风险。你可以将其关闭。", "This carries risks like sensitive data loss or leakage and prompt injection. You can turn it off."],

    // ── onboarding ──
    /// onboarding 标题
    onboarding_title => ["添加一个 API Key 开始使用", "Add an API key to get started"],
    /// onboarding 说明
    onboarding_desc => ["配置 DeepSeek 官方模型，即可开始使用。", "Configure the official DeepSeek models to get started."],
    /// onboarding 跳过
    onboarding_later => ["稍后配置", "Set up later"],
    /// onboarding 保存
    onboarding_save => ["保存并继续", "Save and continue"],
    /// onboarding 空 key 内联错误
    onboarding_key_empty => ["请输入 API 密钥后继续。", "Enter an API key to continue."],
}
