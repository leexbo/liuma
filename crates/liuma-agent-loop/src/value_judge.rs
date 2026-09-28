//! 折叠价值裁定端口:engine 在选段之后、摘要之前咨询一次,裁定折叠
//! 区间里哪些旧工具输出**不值得带进 checkpoint**。
//!
//! 与「模型可见 ⟺ 已记录」的兼容边界:裁定若生效,效果发生在派生纯
//! 函数内部(`liuma_session` 策略④——被引用的 `tool/result` 输出替换为
//! 常量占位符,**1:1 内容替换、条目不删**)。故 `prefix_len`/
//! `estimated_tokens`/派生数组长度在裁前裁后逐项相等:折叠区间与口径
//! 不受裁定影响,变的只是前缀里那几条的内容。
//!
//! 依赖方向:实现方在宿主(`liuma-decision` 的 `FoldJudge`),本 crate
//! 只留端口与 DTO——决策协议(System One 问题/答案/阈值)不进组件层。
//!
//! fail-open:端口缺席、候选为空、`Err`、答案形状不符 → 调用方一律照常
//! 折叠。裁定是旁路面,绝不阻断压缩,也不改变 `FoldOutcome`。

use std::future::Future;
use std::pin::Pin;

pub use liuma_compaction::ValueCandidate;

/// 裁定策略(阈值由实现方给出;本 crate 不内置任何决策阈值)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueJudgePolicy {
    /// 候选最小字符数(太短的不值得问)
    pub min_chars: usize,
    /// 单次询问的候选上限(超出只计数不评估)
    pub max_questions: usize,
    /// 候选预览字符数(进裁定 state)
    pub preview_chars: usize,
}

/// 一次裁定的回执(engine 消费:计数进摘要载荷,`applied` 决定是否
/// 重派生可见面后再切前缀)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FoldAdvice {
    /// 裁定为「不值得带入 checkpoint」的条数(shadow 也回传——观察面:
    /// 只记录时 UI/轨迹要能回答「若生效会裁多少」)
    pub no_value: usize,
    /// 上述候选的字符数合计(台账「裁掉约 N tokens」;shadow 下即
    /// 「若生效会省多少」)
    pub no_value_chars: usize,
    /// 是否已生效(裁定方已落 `decision/pruned`;engine 须重派生)
    pub applied: bool,
    /// 本次实际送去评估的候选数
    pub judged: usize,
    /// 折叠区间内的候选总数(诚实分母;≥ `judged`)
    pub total: usize,
}

/// 折叠价值裁定端口(对象安全;装配层以 `Arc<dyn ValueJudge>` 持有,
/// 「端口缺 = 组件跳过」与其余端口同先例)
pub trait ValueJudge: Send + Sync {
    /// 裁定策略(engine 据此选候选、裁预览长度)
    fn policy(&self) -> ValueJudgePolicy;

    /// 一次裁定。入参 = 折叠区间内的**全部**候选(裁到
    /// [`ValueJudgePolicy::max_questions`] 由实现方做——只有它能同时
    /// 报出「评估 M / 共 N」)。
    ///
    /// `Ok` = 已落 `decision/asked` + `answered`(生效时另落
    /// `decision/pruned`)并给出回执;`Err` = fail-open 信号(收据由
    /// 实现方自行收口,调用方照常折叠)。
    fn judge<'a>(
        &'a self,
        candidates: &'a [ValueCandidate],
    ) -> Pin<Box<dyn Future<Output = Result<FoldAdvice, String>> + Send + 'a>>;
}
