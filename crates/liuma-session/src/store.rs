//! 会话事实源读取端口:读侧窄接口, cold 折叠消费方(stats/锚点/轨迹/
//! 事件读/权限折叠)经此取全量事件,不直接依赖文件布局。
//!
//! 职责边界:
//! - **只读**:追加写不在端口内——事实流仍经 [`crate::log::EventLog`]
//!   的 durability sink 单写落 JSONL(写路径零抽象,见 persistence 层);
//! - **守卫不降级**:实现必须生效读取方守卫(未知未标事件拒载)与
//!   seq 连续性守卫,语义与 `load_log` 全档重建一致;
//! - **实现按需生长**:首个实现为 JSONL 主格式(liuma_app::
//!   JsonlEventStore)。窗口化读取(tail_window)待首个真实消费方
//!   出现再加——history 恒先 attach 后折叠(驻留命中),窗口化调用方
//!   今日不存在,先加方法即死代码(与 persistence::LogBackend 枚举
//!   同一教训)。

use thiserror::Error;

use crate::envelope::EventEnvelope;
use crate::log::LogError;

/// 读取端口错误
#[derive(Debug, Error)]
pub enum EventStoreError {
    /// 文件层失败(带路径上下文)
    #[error("{0}")]
    Io(String),
    /// 行级解析/格式错误(带 path:lineno 上下文)
    #[error("{0}")]
    Malformed(String),
    /// seq 连续性违反
    #[error(transparent)]
    Log(#[from] LogError),
}

/// 会话事实源读取端口。
///
/// 实现约定:`all()` 返回的事件 seq 升序、经读取方守卫与连续性校验;
/// 文件/源缺失是否成立由实现定义(调用方先自行判定在场性)。
pub trait EventStore: Send + Sync {
    /// 全量读取(守卫生效)
    fn all(&self) -> Result<Vec<EventEnvelope>, EventStoreError>;
}

/// seq 连续性校验:从 1 起逐 +1,`seq == 0` 视为占位自动赋号
/// (与 [`crate::log::EventLog::append`] 的运行时强制同语义)。
/// 供不经 EventLog 的读取路径复用——同一份日志,两条重建路径的
/// 判定必须一致。
pub fn verify_seq_contiguity(events: &[EventEnvelope]) -> Result<(), LogError> {
    for (ix, ev) in events.iter().enumerate() {
        let expected = ix as u64 + 1;
        if ev.seq != 0 && ev.seq != expected {
            return Err(LogError::NotContiguous {
                actual: ev.seq,
                expected,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::EventEnvelope;
    use serde_json::json;

    fn env(seq: u64) -> EventEnvelope {
        let mut ev = EventEnvelope::new("user/message", 0, json!({ "content": "hi" }));
        ev.seq = seq;
        ev
    }

    #[test]
    fn contiguity_accepts_sequential_and_zero_placeholders() {
        assert!(verify_seq_contiguity(&[]).is_ok(), "空日志连续");
        assert!(verify_seq_contiguity(&[env(1), env(2), env(3)]).is_ok());
        // seq:0 = 占位自动赋号,与 EventLog::append 同语义
        assert!(verify_seq_contiguity(&[env(0), env(0)]).is_ok());
        assert!(verify_seq_contiguity(&[env(1), env(0), env(3)]).is_ok());
    }

    #[test]
    fn contiguity_rejects_gap_and_non_one_start() {
        assert!(matches!(
            verify_seq_contiguity(&[env(1), env(3)]),
            Err(LogError::NotContiguous {
                actual: 3,
                expected: 2
            })
        ));
        assert!(matches!(
            verify_seq_contiguity(&[env(2)]),
            Err(LogError::NotContiguous {
                actual: 2,
                expected: 1
            })
        ));
    }
}
