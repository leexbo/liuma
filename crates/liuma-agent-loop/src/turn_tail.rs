//! TurnTailSnapshot:turn 边界工作区快照端口(宿主注入)。
//!
//! turn/start 落档后 Start 相(实现方记基线,返回值忽略)、turn/end
//! 落档前 End 相(返回 `Some({"changes": [...]})` 并入 turn/end data,
//! None = 不可用/无变更,静默)。基线状态由实现方自管——引擎不持有
//! 工作区概念,git/其它对比手段全在宿主侧。
//!
//! 消费面:桌面投影把 `changes` 合入回合交付网格的变更层(present
//! 宣告与 diff 视图缺席时的兜底,覆盖 bash 创建文件的盲区)。

use serde_json::Value;
use std::future::Future;

/// 快照相位:引擎在 turn/start 落档后调 Start、turn/end 落档前调 End
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnTailPhase {
    /// 记基线(返回值忽略)
    Start,
    /// 取变更载荷(`{"changes": [path, ...]}`;None = 静默跳过)
    End,
}

/// 引擎持有的对象安全形态(与 HookPortObj 同理)
pub trait TurnTailSnapshotObj: Send + Sync {
    fn snapshot<'a>(
        &'a self,
        phase: TurnTailPhase,
    ) -> std::pin::Pin<Box<dyn Future<Output = Option<Value>> + Send + 'a>>;
}
