//! 设置域共享状态:SettingsStore 切片结构、页路由/确认载荷/偏好枚举、Default 与上下文窗口解析(含测试)。

use super::*;

/// 解析上下文窗口草稿:空串 = 不覆盖(None);接受 1 以上的整数,可带
/// 单位后缀——`K`/`M` 十进制(1K = 1,000、1M = 1,000,000,与默认值
/// 和展示同一进制),`Ki`/`Mi` 二进制(1Ki = 1,024、1Mi = 1,048,576,
/// 上下文窗口的常见写法的精确表达);大小写不敏感,容忍 `_` 与 `,`
/// 千分位。其余 = 非法(表单拒绝保存并就地提示);溢出同非法。
pub(crate) fn parse_context_window_tokens(raw: &str) -> Result<Option<u64>, ()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let cleaned = trimmed.replace(['_', ','], "").to_ascii_lowercase();
    // 先长后短:Ki/Mi 必须先于 K/M 判定,否则 "128ki" 会以 "i" 残留失败
    let (digits, scale) = if let Some(head) = cleaned.strip_suffix("ki") {
        (head, 1_024u64)
    } else if let Some(head) = cleaned.strip_suffix("mi") {
        (head, 1_024 * 1_024)
    } else if let Some(head) = cleaned.strip_suffix('k') {
        (head, 1_000)
    } else if let Some(head) = cleaned.strip_suffix('m') {
        (head, 1_000_000)
    } else {
        (cleaned.as_str(), 1)
    };
    match digits.parse::<u64>() {
        Ok(v) => v.checked_mul(scale).filter(|t| *t > 0).map(Some).ok_or(()),
        Err(_) => Err(()),
    }
}

/// 千分位分组(仅展示用;输入解析容忍分组符)
pub(crate) fn grouped_tokens(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 设置导航区(设置模式 = 侧栏切换为设置菜单,内容区显示对应页;
/// 插件/MCP/技能待实装后进菜单——入口迁移优于新增,不做空占位)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsNav {
    /// 通用
    General,
    /// 模型与 Provider
    Models,
    /// MCP Servers
    Mcp,
    /// Hooks(Claude Code / Codex 桥)
    Hooks,
    /// 决策模型(System One 协议)
    Decision,
    /// 已归档的聊天(数据与统计组)
    ArchivedChats,
    /// 关于
    About,
}

impl SettingsNav {
    /// 稳定 ASCII slug:debug selector / 测试寻址用。
    ///
    /// selector **不得**从可见文案派生——文案随语言切换,selector 会跟着变
    /// (旧实现用标签拼 `settings-nav-{label}`,切成中文后测试全部寻址失败)。
    pub fn slug(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Models => "Models",
            Self::Mcp => "Mcp",
            Self::Hooks => "Hooks",
            Self::Decision => "Decision",
            Self::ArchivedChats => "ArchivedChats",
            Self::About => "About",
        }
    }
}

/// 归档区排序方式(Updated = updated_at 倒序,core 正典序直出;
/// Alpha = 标题字母序,视图层重排)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchivedOrder {
    #[default]
    /// 按更新时间
    Updated,
    /// 按字母顺序
    Alpha,
}

/// 归档确认动作(弹窗确认钮直派的载荷;Clone 随闭包携带,不经旗标中转
/// ——同 provider 删除确认的直派模式)
#[derive(Debug, Clone)]
pub(crate) enum ArchivedConfirmKind {
    /// 删除单条归档
    Delete { archive_id: String },
    /// 删除单项目全部归档
    PurgeProject { pkey: String },
    /// 清空全部归档
    ClearAll,
}

/// 通用区偏好下拉菜单种类(根级坐标锚定,同行菜单模式)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefMenuKind {
    /// Agent 预设
    Preset,
    /// 权限
    Permission,
    /// 语言
    Language,
    /// 繁忙时 Enter 键行为
    BusyEnter,
    /// 浅盘主题
    ThemeLight,
    /// 深盘主题
    ThemeDark,
}

/// 设置功能切片状态(页路由/快照/onboarding/provider 编辑器与删除确认/偏好下拉)。
pub(crate) struct SettingsStore {
    /// 设置页开(独立页路由:右列内容整体切换)
    pub settings_open: bool,
    /// 设置页快档(settings_view 数据;打开与每次变更后刷新)
    pub settings_snapshot: serde_json::Value,
    /// 首运行引导态(未 onboarded 且默认 provider 凭据缺席)
    pub needs_onboarding: bool,
    /// onboarding 模态输入框(挂窗后惰建;write-only,保存写默认 provider)
    pub onboarding_key_input: Option<Entity<InputState>>,
    /// key 输入框当前占位(模式判定;InputState 无 placeholder 读取)
    pub key_input_placeholder: String,
    /// onboarding 内联错误(空 key 提交 / 保存失败)
    pub onboarding_key_error: Option<String>,
    /// 设置页 provider 表单三输入(挂窗后建;同 id 提交 = 更新)
    pub set_form_id: Option<Entity<InputState>>,
    /// provider 表单 base_url 输入
    pub set_form_url: Option<Entity<InputState>>,
    /// provider 表单默认模型输入
    pub set_form_model: Option<Entity<InputState>>,
    /// provider 表单方言选择(chips 三选一)
    pub set_form_dialect: String,
    /// 决策表单:端点输入(常驻;本区无「打开」事件,回填时机见
    /// [`SettingsStore::sync_decision_form`])
    pub decision_form_url: Option<Entity<InputState>>,
    /// 决策表单:模型输入
    pub decision_form_model: Option<Entity<InputState>>,
    /// 决策表单:API key 输入(write-only——明文永不回填,保存成功即清空)
    pub decision_form_key: Option<Entity<InputState>>,
    /// 决策表单:询问超时输入(毫秒;空 = 内置默认)
    pub decision_form_timeout: Option<Entity<InputState>>,
    /// 决策表单:守卫拦截线输入(0–1 小数或带 % 百分数;空 = 内置默认)
    pub decision_form_guard_high: Option<Entity<InputState>>,
    /// 决策表单:裁判修剪线输入
    pub decision_form_context_high: Option<Entity<InputState>>,
    /// 决策表单:折叠裁掉线输入
    pub decision_form_fold_high: Option<Entity<InputState>>,
    /// 设置页导航(两栏壳:左 nav + 单区内容)
    pub settings_nav: SettingsNav,
    /// MCP 分区详情页(None = 列表页;Some = 详情:新增/编辑/JSON 导入)
    pub mcp_detail: Option<McpDetailState>,
    /// Hooks 分区详情页(None = 列表页;Some = 新增/编辑表单)
    pub hooks_detail: Option<HooksDetailState>,
    /// MCP server 最近连接状态(server id → (status, error);mcp/status 帧维护)
    pub mcp_status_by_id: HashMap<String, (String, String)>,
    /// 行内编辑中的 provider id(编辑卡在行卡内展开)
    pub editing_provider: Option<String>,
    /// 添加卡开态
    pub adding_provider: bool,
    /// 首运行 setup 卡已手动关闭的 provider(本会话内回退普通行)
    pub dismissed_setup: HashSet<String>,
    /// 编辑卡 API key 输入(write-only;应用时随条目存 settings)
    pub key_input: Option<Entity<InputState>>,
    /// provider 表单显示名输入
    pub set_form_name: Option<Entity<InputState>>,
    /// provider 表单模型清单草稿(表单内增删,应用时落盘)
    pub set_form_models: Vec<String>,
    /// 厂商托管工具启用草稿(web_search 勾选;apply 时按 dialect 门控落盘)
    pub set_form_hosted_web_search: bool,
    /// 手动添加模型的单行输入
    pub set_form_model_input: Option<Entity<InputState>>,
    /// 每模型上下文窗口覆盖草稿(模型 id → token;保存时落
    /// `ProviderEntry.model_context_windows`;缺席 = 用内置默认 1M)
    pub set_form_context_windows: std::collections::BTreeMap<String, u64>,
    /// 行内编辑中的模型(Some = 该模型行展开为窗口输入)
    pub context_window_edit: Option<String>,
    /// 窗口输入(单例;编辑中的模型共用;表单输入惰建时一并建)
    pub context_window_input: Option<Entity<InputState>>,
    /// 窗口输入校验错误(非法时行内提示;提交成功即清)
    pub context_window_error: bool,
    /// 计费端点启用(表单开关)
    pub set_form_billing_enabled: bool,
    /// 计费形态(balance / usage)
    pub set_form_billing_kind: String,
    /// 计费 URL 输入
    pub set_form_billing_url: Option<Entity<InputState>>,
    /// 计费 JSON 路径输入:余额金额 / 货币
    pub set_form_path_balance: Option<Entity<InputState>>,
    pub set_form_path_currency: Option<Entity<InputState>>,
    /// 计费 JSON 路径输入:5小时 / 7天 / 重置
    pub set_form_path_5h: Option<Entity<InputState>>,
    pub set_form_path_7d: Option<Entity<InputState>>,
    pub set_form_path_resets: Option<Entity<InputState>>,
    /// 计费鉴权形态(None = 方言默认;Some("raw") = 裸 token;无 UI 输入,
    /// 目录预填带来)
    pub set_form_billing_auth_style: Option<String>,
    /// 内置供应商卡模式(提供方下拉 + 只填 key;适配器与目录绑定)
    pub builtin_mode: bool,
    /// 内置模式当前选中的目录厂商 id
    pub builtin_picked: String,
    /// 内置卡「自定义设置」折叠开态
    pub builtin_advanced_open: bool,
    /// 内置卡提供方下拉 / 自定义卡 API 格式下拉(挂窗后建)
    pub builtin_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    pub dialect_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 从端点获取模型的弹层(Some = 开):候选 + 逐项勾选态
    pub model_fetch: Option<ModelFetch>,
    /// 模型拉取进行中(弹层内「获取中」态)
    pub model_fetch_loading: bool,
    /// 计费刷新中的 provider(刷新钮禁用态)
    pub billing_refreshing: Option<String>,
    /// 额度自动刷新进行中(静默路径单飞;与手动 billing_refreshing 互斥跳过)
    pub billing_auto_running: bool,
    /// 额度自动刷新上次尝试时刻(turn/end 防抖基线;失败也记,防抖不追打)
    pub billing_auto_last: Option<std::time::Instant>,
    /// 保存通告(应用成功后一行 success 文案)
    pub saved_provider_notice: Option<String>,
    /// 设置页内通告槽(单槽覆盖,不堆积;(成功?, 文案)):计费查询、
    /// 保存失败等设置动作的反馈——**不走聊天区** push_local_notice。
    /// 4s 自动清除(notice_seq 守卫防误清新通告)
    pub settings_notice: Option<(bool, String)>,
    /// 通告代次(每次置新通告 +1;清除任务比对丢弃过期)
    pub settings_notice_seq: u64,
    /// 保存通告的清除任务同款瞬态(「已保存 X」不常驻)
    /// 通用区偏好下拉(gpui-component Select;挂窗后建,Confirm 落盘)
    pub preset_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 权限下拉
    pub permission_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 语言下拉
    pub language_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 繁忙时 Enter 键行为下拉
    pub busy_enter_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 浅盘主题下拉(选项 = registry 主题名,非词典文案,语言换档
    /// 不重建)
    pub theme_light_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 深盘主题下拉
    pub theme_dark_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 偏好下拉构建时刻的 locale id(sync_locale_ui 换档重建判据;
    /// ensure_pref_selects 写入)
    pub selects_lang: &'static str,
    /// 权限选 full-access 的风险确认。
    /// Some 记录确认来源:设置页默认预设 / composer 会话权限——确认后
    /// 各自落不同的目标(默认预设落盘 / 会话 set_permission)
    pub full_access_confirm: Option<FullAccessAsk>,
    /// 归档区清单(None = 未加载;进入归档区首拉,动作后重拉)
    pub archived: Option<Vec<ArchivedSessionSummary>>,
    /// 归档区首拉进行中(区分加载态与空态)
    pub archived_loading: bool,
    /// 归档区搜索输入(挂窗惰建;实时过滤 render 期读 value)
    pub archived_search: Option<Entity<InputState>>,
    /// 归档区排序(默认按更新时间)
    pub archived_order: ArchivedOrder,
    /// 归档区项目筛选(None = 全部项目;值 = project_key)
    pub archived_project: Option<String>,
    /// 归档区排序下拉(惰建;静态两项)
    pub archived_order_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 归档区项目下拉(惰建;选项集变化时整体重建——SelectState 构造后
    /// labels 不可变)
    pub archived_project_select: Option<Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    /// 项目下拉构建时刻的选项集(pkey 增减的重建判据)
    pub archived_project_options: Vec<String>,
}

/// full-access 风险确认的来源(确认动作随来源分流)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullAccessAsk {
    /// 设置页「默认权限预设」选完全权限:确认 = 落盘默认预设
    Default,
    /// composer 权限菜单选完全权限:确认 = 会话内 set_permission
    Session,
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            settings_open: false,
            settings_snapshot: serde_json::Value::Null,
            needs_onboarding: false,
            onboarding_key_input: None,
            key_input_placeholder: String::new(),
            onboarding_key_error: None,
            set_form_id: None,
            set_form_url: None,
            set_form_model: None,
            set_form_dialect: "openai-completions".into(),
            decision_form_url: None,
            decision_form_model: None,
            decision_form_key: None,
            decision_form_timeout: None,
            decision_form_guard_high: None,
            decision_form_context_high: None,
            decision_form_fold_high: None,
            settings_nav: SettingsNav::Models,
            mcp_detail: None,
            hooks_detail: None,
            mcp_status_by_id: HashMap::new(),
            editing_provider: None,
            adding_provider: false,
            dismissed_setup: HashSet::new(),
            key_input: None,
            set_form_name: None,
            set_form_models: Vec::new(),
            set_form_hosted_web_search: false,
            set_form_model_input: None,
            set_form_context_windows: std::collections::BTreeMap::new(),
            context_window_edit: None,
            context_window_input: None,
            context_window_error: false,
            set_form_billing_enabled: false,
            set_form_billing_kind: "balance".into(),
            set_form_billing_url: None,
            set_form_path_balance: None,
            set_form_path_currency: None,
            set_form_path_5h: None,
            set_form_path_7d: None,
            set_form_path_resets: None,
            set_form_billing_auth_style: None,
            builtin_mode: false,
            builtin_picked: String::new(),
            builtin_advanced_open: false,
            builtin_select: None,
            dialect_select: None,
            model_fetch: None,
            model_fetch_loading: false,
            billing_refreshing: None,
            billing_auto_running: false,
            billing_auto_last: None,
            saved_provider_notice: None,
            settings_notice: None,
            settings_notice_seq: 0,
            preset_select: None,
            permission_select: None,
            language_select: None,
            busy_enter_select: None,
            theme_light_select: None,
            theme_dark_select: None,
            selects_lang: i18n::DEFAULT,
            full_access_confirm: None,
            archived: None,
            archived_loading: false,
            archived_search: None,
            archived_order: ArchivedOrder::default(),
            archived_project: None,
            archived_order_select: None,
            archived_project_select: None,
            archived_project_options: Vec::new(),
        }
    }
}

#[cfg(test)]
#[cfg(test)]
mod context_window_tests {
    use super::{grouped_tokens, parse_context_window_tokens};

    /// 空 = 不覆盖(回落默认);分组符容忍;K/M 十进制、Ki/Mi 二进制;
    /// 零/裸单位/余缀/小数/溢出均非法
    #[test]
    pub(crate) fn parses_draft_into_override_or_default() {
        assert_eq!(parse_context_window_tokens(""), Ok(None));
        assert_eq!(parse_context_window_tokens("   "), Ok(None));
        assert_eq!(parse_context_window_tokens("131072"), Ok(Some(131_072)));
        assert_eq!(parse_context_window_tokens(" 128,000 "), Ok(Some(128_000)));
        assert_eq!(
            parse_context_window_tokens("1_000_000"),
            Ok(Some(1_000_000))
        );
        // 单位后缀:十进制照默认值进制,二进制给上下文窗口的精确写法
        assert_eq!(parse_context_window_tokens("1M"), Ok(Some(1_000_000)));
        assert_eq!(parse_context_window_tokens("1m"), Ok(Some(1_000_000)));
        assert_eq!(parse_context_window_tokens("256K"), Ok(Some(256_000)));
        assert_eq!(parse_context_window_tokens("128k"), Ok(Some(128_000)));
        assert_eq!(parse_context_window_tokens("128Ki"), Ok(Some(131_072)));
        assert_eq!(parse_context_window_tokens("128ki"), Ok(Some(131_072)));
        assert_eq!(parse_context_window_tokens("1Mi"), Ok(Some(1_048_576)));
        assert_eq!(parse_context_window_tokens("2_000K"), Ok(Some(2_000_000)));
        assert_eq!(parse_context_window_tokens("0"), Err(()));
        assert_eq!(parse_context_window_tokens("0K"), Err(()));
        assert_eq!(parse_context_window_tokens("-1"), Err(()));
        assert_eq!(parse_context_window_tokens("k"), Err(()));
        assert_eq!(parse_context_window_tokens("128kk"), Err(()));
        assert_eq!(parse_context_window_tokens("128Kt"), Err(()));
        assert_eq!(parse_context_window_tokens("1.5M"), Err(()));
        assert_eq!(parse_context_window_tokens("abc"), Err(()));
        assert_eq!(
            parse_context_window_tokens("18446744073709551615K"),
            Err(()),
            "乘单位溢出 = 非法"
        );
    }

    /// 展示用千分位分组(输入解析接受同一形态)
    #[test]
    pub(crate) fn groups_token_counts_for_display() {
        assert_eq!(grouped_tokens(128), "128");
        assert_eq!(grouped_tokens(65_536), "65,536");
        assert_eq!(grouped_tokens(1_000_000), "1,000,000");
    }
}
