//! 审批/问答/计划评审文案词典(features/ask)。

use crate::kits::i18n::entries;

entries! {
    /// 沙箱升级审批卡标题
    sandbox_title => ["沙箱升级审批", "Sandbox escalation"],
    /// 沙箱升级说明(工具名 + 模式迁移;仅本次)
    sandbox_desc(tool, current, target) => ["{tool} · {current} → {target}(仅本次)", "{tool} · {current} → {target} (this once)"],
    /// 拒绝(审批钮/缺省动作)
    reject => ["拒绝", "Reject"],
    /// 批准一次(allow-once)
    approve_once => ["批准一次", "Allow once"],
    /// 批准(缺省动作回落)
    approve => ["批准", "Approve"],
    /// 用户取消(回落对话)
    user_cancelled => ["用户取消,回到对话", "Cancelled by user — back to the chat"],

    /// 决策模型风险标注标题(审批卡;advisory)
    risk_title => ["决策模型风险评估", "Decision model risk"],
    /// 风险标注:低风险档
    risk_low => ["低风险", "Low risk"],
    /// 风险标注:不确定档
    risk_uncertain => ["不确定", "Uncertain"],
    /// 风险标注:有风险档
    risk_risky => ["有风险", "Risky"],
    /// 风险理由:低风险档(概率为「低风险」的模型给值,照实展示)
    risk_reason_low(p) => [
        "模型判为常规、可逆的操作(低风险概率 {p})",
        "Model reads this as routine and reversible (low-risk probability {p})"
    ],
    /// 风险理由:不确定档
    risk_reason_uncertain(p) => [
        "模型对这一手的把握不足(低风险概率 {p}),建议自己看一眼",
        "Model is not confident about this call (low-risk probability {p}) — worth a look"
    ],
    /// 风险理由:有风险档
    risk_reason_risky(p) => [
        "模型判为破坏性、不可逆,或越出工作区/扩权的操作(低风险概率 {p})",
        "Model reads this as destructive, irreversible, or beyond the workspace (low-risk probability {p})"
    ],

    /// 问答卡:其他(自定义输入选项)
    other_option => ["其他", "Other"],
    /// 问答卡:上一题
    prev_q => ["上一题", "Previous"],
    /// 问答卡:下一题
    next_q => ["下一题", "Next"],
    /// 问答卡:跳过
    skip_q => ["跳过", "Skip"],
    /// 问答卡:提交
    submit_q => ["提交", "Submit"],
    /// 问答卡输入占位
    answer_ph => ["输入你的答案", "Type your answer"],
    /// 问答卡错误:未选未填
    err_pick => ["请选择一个选项或填写自定义答案。", "Pick an option or write your own answer."],
    /// 问答卡错误:必答题未完成
    err_required => ["请先完成这道问题。", "Answer this question first."],

    /// 计划评审:批准钮
    approve_plan => ["是,实施此计划", "Yes, implement this plan"],
    /// 计划评审:拒绝钮
    decline_plan => ["否,并告诉它应该如何做不同", "No, and tell it what to do differently"],
    /// 计划评审:拒绝输入占位(同拒绝钮文案)
    decline_ph => ["否,并告诉它应该如何做不同", "No, and tell it what to do differently"],
    /// 计划评审:批准选项说明(离场)
    approve_desc => ["离开计划模式;计划从下一步开始执行", "Leaves plan mode; the plan runs from the next step"],
    /// 计划评审:拒绝选项说明(留场)
    decline_desc => ["留在计划模式;反馈会回传给模型", "Stays in plan mode; feedback goes back to the model"],
}
