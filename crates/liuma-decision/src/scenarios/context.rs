//! 上下文裁判:旧工具输出的修剪裁决(winnow/fast-jev-compaction 模式)。
//!
//! 与不变式的兼容边界:**裁决先落档**(`decision/pruned` 事件),效果
//! 发生在派生纯函数内部(`derive_visible_messages` 策略④)——日志本体
//! 不动,请求面与闸门期望面自动同源。本模块只做纯函数:候选选择、
//! 问题构造、答案解释;出网与落档在宿主装配层(liuma-app judge_context)。

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::types::{Answer, DecisionRequest, Question};

/// 候选(被选中待裁决的旧工具输出)
#[derive(Debug, Clone, PartialEq)]
pub struct PruneCandidate {
    /// tool/result 事件 seq(修剪引用的锚点)
    pub seq: u64,
    /// 输出字符数(排序用:大输出优先裁决,省 token 收益最大)
    pub chars: usize,
    /// 输出预览(state 摘要;截断到 [`PREVIEW_MAX_CHARS`])
    pub preview: String,
}

/// 候选预览的字符上限
pub const PREVIEW_MAX_CHARS: usize = 600;

/// 候选选择(纯函数):保留尾之外的 tool/result、长度达标、未被修剪过。
///
/// `retain_from`:保留尾边界(此 seq(含)之后不参选——模型正在用的
/// 上下文不动);`already_pruned`:历史 pruned 事件引用过的 seq(并集去重)。
pub fn select_candidates<'a>(
    events: impl Iterator<Item = &'a liuma_session::EventEnvelope>,
    retain_from: u64,
    already_pruned: &HashSet<u64>,
) -> Vec<PruneCandidate> {
    let mut out: Vec<PruneCandidate> = events
        .into_iter()
        .filter(|ev| ev.r#type == "tool/result" && ev.seq < retain_from)
        .filter(|ev| !already_pruned.contains(&ev.seq))
        .filter_map(|ev| {
            let output = ev.data["output"].as_str()?;
            let chars = output.chars().count();
            (chars >= crate::thresholds::PRUNE_CANDIDATE_MIN_CHARS).then(|| PruneCandidate {
                seq: ev.seq,
                chars,
                preview: output.chars().take(PREVIEW_MAX_CHARS).collect(),
            })
        })
        .collect();
    // 大输出优先(收益排序);超出单请求问题数上限的尾部丢弃
    out.sort_by_key(|c| std::cmp::Reverse(c.chars));
    out.truncate(crate::thresholds::MAX_QUESTIONS_PER_REQUEST);
    out
}

/// 裁决请求构造:每候选一个 noul 问题(id = `s<seq>`;state = 预览数组,
/// 问题内 `tool_results[N]` 反引号路径指向对应槽位)
pub fn build_request(candidates: &[PruneCandidate], model: &str) -> DecisionRequest {
    let previews: Vec<Value> = candidates
        .iter()
        .map(|c| json!({ "seq": c.seq, "chars": c.chars, "output": c.preview }))
        .collect();
    let questions = candidates
        .iter()
        .enumerate()
        .map(|(n, c)| {
            (
                format!("s{}", c.seq),
                Question::Noul {
                    instructions: json!({
                        "question": crate::thresholds::CONTEXT_PRUNE_QUESTION.replace(
                            "`tool_results[@]`",
                            &format!("`tool_results[{n}]`"),
                        ),
                        "tool_results": previews,
                    }),
                    criteria: None,
                },
            )
        })
        .collect();
    DecisionRequest {
        state: json!({ "candidates": previews.len() }),
        questions,
        model: model.to_string(),
    }
}

/// 答案解释(纯函数):noul ≥ 阈值的候选 = 已无引用价值 → 修剪清单
pub fn losing_seqs(
    candidates: &[PruneCandidate],
    answers: &std::collections::BTreeMap<String, Answer>,
    no_value_probability: f64,
) -> Vec<(u64, f64)> {
    candidates
        .iter()
        .filter_map(|c| {
            let key = format!("s{}", c.seq);
            match answers.get(&key) {
                Some(Answer::Noul { noul })
                    if noul.is_finite() && *noul >= no_value_probability =>
                {
                    Some((c.seq, *noul))
                }
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_session::{EventEnvelope, EventLog};

    fn tool_result(seq: u64, chars: usize) -> EventEnvelope {
        let mut ev = EventEnvelope::new(
            "tool/result",
            0,
            json!({ "call": 1, "output": "x".repeat(chars) }),
        );
        ev.seq = seq;
        ev
    }

    #[test]
    fn select_filters_length_retention_and_already_pruned() {
        let mut log = EventLog::new();
        for (seq, chars) in [(1, 3000usize), (2, 100), (3, 5000), (4, 4000)] {
            log.append(tool_result(seq, chars)).unwrap();
        }
        let already: HashSet<u64> = [3].into();
        // retain_from=4:seq<4 参选;seq2 太短;seq3 已修剪过 → 只剩 1
        let picked = select_candidates(log.iter(), 4, &already);
        assert_eq!(picked.iter().map(|c| c.seq).collect::<Vec<_>>(), vec![1]);
        // 大输出优先排序 + 上限截断
        let mut log2 = EventLog::new();
        for (seq, chars) in [
            (1, 2000usize),
            (2, 9000),
            (3, 5000),
            (4, 8000),
            (5, 7000),
            (6, 6000),
            (7, 2500),
            (8, 2600),
            (9, 2700),
        ] {
            log2.append(tool_result(seq, chars)).unwrap();
        }
        let none: HashSet<u64> = HashSet::new();
        let picked = select_candidates(log2.iter(), 10, &none);
        assert_eq!(picked.len(), crate::thresholds::MAX_QUESTIONS_PER_REQUEST);
        assert_eq!(picked[0].seq, 2, "大输出优先");
        assert!(!picked.iter().any(|c| c.seq == 1), "最小者被上限截断");
    }

    #[test]
    fn request_and_interpretation_roundtrip() {
        let candidates = vec![
            PruneCandidate {
                seq: 7,
                chars: 5000,
                preview: "alpha".into(),
            },
            PruneCandidate {
                seq: 9,
                chars: 8000,
                preview: "beta".into(),
            },
        ];
        let req = build_request(&candidates, "m");
        assert_eq!(req.questions.len(), 2);
        assert_eq!(req.questions[0].0, "s7");
        // 问题内路径替换:反引号槽位指向本问题对应的候选
        let Question::Noul { instructions, .. } = &req.questions[1].1 else {
            panic!("noul");
        };
        assert!(
            instructions["question"]
                .as_str()
                .unwrap()
                .contains("`tool_results[1]`"),
            "{}",
            instructions["question"]
        );

        // 解释:s9 高概率 → 修剪;s7 低概率 → 保留
        let mut answers = std::collections::BTreeMap::new();
        answers.insert("s7".to_string(), Answer::Noul { noul: 0.1 });
        answers.insert("s9".to_string(), Answer::Noul { noul: 0.95 });
        let lost = losing_seqs(
            &candidates,
            &answers,
            crate::thresholds::PRUNE_NO_VALUE_PROBABILITY,
        );
        assert_eq!(lost, vec![(9, 0.95)]);
    }
}
