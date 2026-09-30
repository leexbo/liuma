//! 子代理血缘的 store 域:血缘目录开态 + 后代查询(origin=='subagent'
//! 且 parent 匹配)。菜单互斥经 shell 底座 close_all_menus 协调,本域
//! 只自持目录开态。

use gpui_kit::Context;

use liuma_core::proto::SessionSummary;

use crate::shell::store::AppStore;

/// 子代理血缘功能切片状态(后代目录开态)。
pub(crate) struct SubagentsStore {
    /// 任务条列表展开态(默认展开——运行中子代理直观可见)
    pub task_bar_open: bool,
}

impl Default for SubagentsStore {
    fn default() -> Self {
        Self {
            task_bar_open: true,
        }
    }
}

/// 血缘行:jobs 帧与子会话清单按 id 合并后的呈现数据
#[derive(Debug, Clone)]
pub(crate) struct LineageRow {
    /// 子会话槽位 id(跳转用;shell 行为 job-N 非会话,不跳转)
    pub session_id: String,
    /// 行属类:subagent(可跳转子会话)/ shell(后台进程)
    pub kind: String,
    /// 任务名(委派 description;无 jobs 数据时回落会话标题)
    pub label: String,
    /// 运行中(running job;无 jobs 数据 = false)
    pub running: bool,
    /// 状态点着色语义:running/completed/failed/killed(无 jobs 数据 = None)
    pub dot: Option<&'static str>,
    /// 委派时刻(ms;计时起点)
    pub started_at: Option<i64>,
    /// 终态时刻(ms)
    pub finished_at: Option<i64>,
}

impl AppStore {
    /// 查看视角的家族锚(任务条):当前在子会话 → 其父;否则原会话。
    /// 任务条/计时以锚为键取 jobs 清单。
    pub(crate) fn subagent_anchor_of(&self, session_id: &str) -> String {
        self.state
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .and_then(|s| s.parent_session_id.clone())
            .unwrap_or_else(|| session_id.to_string())
    }

    /// 任务条展开态写入。开合请求源自 Accordion 触发器,给的是**结果态**
    /// (点击后的开集)而非「切换」请求;同值不触发重渲——库根节点的点击
    /// 回调对**面板内**的点击同样会触发(冒泡),靠这里的幂等挡住,否则点
    /// 一行子代理切会话会顺带把条收起。
    pub fn set_task_bar_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.subagents.task_bar_open == open {
            return;
        }
        self.subagents.task_bar_open = open;
        cx.notify();
    }

    /// 某会话运行中的后台子代理数(jobs 帧统计;shell job 不驱动
    /// 会话列表的运行态标记)
    pub fn running_subagent_count(&self, session_id: &str) -> usize {
        self.state
            .jobs_by_id
            .get(session_id)
            .map(|jobs| {
                jobs.iter()
                    .filter(|j| j["status"] == "running" && j["kind"] != "shell")
                    .count()
            })
            .unwrap_or(0)
    }

    /// 血缘行合并视图:jobs 帧权威(实时状态/计时/prompt),
    /// 子会话清单补位(无 jobs 帧的旧态/他端子代理 → 仅标题可跳转)
    pub fn lineage_rows(&self, session_id: &str) -> Vec<LineageRow> {
        let mut rows: Vec<LineageRow> = Vec::new();
        if let Some(jobs) = self.state.jobs_by_id.get(session_id) {
            for j in jobs {
                let status = j["status"].as_str().unwrap_or("completed");
                rows.push(LineageRow {
                    session_id: j["id"].as_str().unwrap_or_default().to_string(),
                    kind: j["kind"].as_str().unwrap_or("subagent").to_string(),
                    label: j["label"].as_str().unwrap_or_default().to_string(),
                    running: status == "running",
                    dot: Some(match status {
                        "running" => "running",
                        "failed" => "failed",
                        "killed" => "killed",
                        _ => "completed",
                    }),
                    started_at: j["startedAt"].as_i64(),
                    finished_at: j["finishedAt"].as_i64(),
                });
            }
        }
        for c in self.subagent_children_of(session_id) {
            if rows.iter().any(|r| r.session_id == c.session_id) {
                continue;
            }
            let title = self.title_for(&c.session_id);
            rows.push(LineageRow {
                session_id: c.session_id,
                kind: "subagent".into(),
                label: title,
                running: false,
                dot: None,
                started_at: None,
                finished_at: None,
            });
        }
        rows
    }

    /// 血缘:某会话的直接 subagent 后代(origin=='subagent' 且 parent 匹配)
    pub fn subagent_children_of(&self, session_id: &str) -> Vec<SessionSummary> {
        self.state
            .sessions
            .iter()
            .filter(|s| {
                s.origin.as_deref() == Some("subagent")
                    && s.parent_session_id.as_deref() == Some(session_id)
            })
            .cloned()
            .collect()
    }

    /// 血缘:当前会话是否为 subagent(页头切换器用;切换器本期未做,预留)
    #[allow(dead_code)]
    pub fn current_is_subagent(&self) -> bool {
        self.state
            .current_id
            .as_deref()
            .and_then(|id| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| s.session_id == id)
                    .map(|s| s.origin.as_deref() == Some("subagent"))
            })
            .unwrap_or(false)
    }
}
