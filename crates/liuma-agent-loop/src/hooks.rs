//! HookPort:引擎拦截点 trait(收敛为单一 trait;M4.2 拍板)。
//!
//! RS 四调用点:on_prompt_submit(用户提示提交)、pre_tool(工具
//! 执行前)、post_tool(工具执行后)、on_stop(turn 收尾前)。
//! SessionStart / subagent:* 由宿主
//! detached 处理(不进引擎)。全部可选:None 时引擎零开销直通。
//!
//! 决策语义:PreToolUse deny ⇒ 工具不执行、isError 结果回灌;
//! PostToolUse block ⇒ 结果改写 + feedback;Stop continue ⇒ steer 强制
//! 续跑;UserPromptSubmit reject ⇒ turn 以 blocked 收尾、无 step。
//! hook/invoked·result 落档归实现方(经宿主 append 回调,唯一写入口)。

use serde_json::Value;

use crate::tools::ToolCallRequest;

/// UserPromptSubmit 裁决
#[derive(Debug, Clone, PartialEq)]
pub enum PreStepVerdict {
    /// 放行(进入正常消息组装)
    Proceed,
    /// 拒绝:turn 直接收尾,无 step(kind='reject')
    Reject,
}

/// PreToolUse 裁决
#[derive(Debug, Clone, PartialEq)]
pub enum PreToolVerdict {
    Proceed,
    /// 工具不执行,reason 作为 isError 结果回灌模型
    Deny {
        reason: String,
    },
}

/// PostToolUse 裁决(结果已产出,决定如何落 tool/result)
#[derive(Debug, Clone, PartialEq)]
pub enum PostToolVerdict {
    Pass,
    /// 结果改写:output = feedback、success = false(block + feedback)
    Block {
        feedback: String,
    },
    /// 结果照落,其后追加一条注入上下文(context-only 委托折叠)
    Inject {
        text: String,
    },
}

/// Stop 裁决
#[derive(Debug, Clone, PartialEq)]
pub enum StopVerdict {
    Pass,
    /// 强制续跑:reason 压入引擎 steer 通道
    Continue {
        reason: String,
    },
}

/// 引擎拦截点。
///
/// 实现方(liuma-hooks HookPortImpl)负责 hook/invoked·result 落档与
/// 多钩子合并;引擎只消费最终裁决。`turn` 为引擎侧本 turn 序号
/// (自 1 起,与 hook/* 事件载荷的 turn 同源)。
pub trait HookPort: Send + Sync {
    /// UserPromptSubmit:turn/start 落档后、首条 user/message 前
    fn on_prompt_submit(
        &self,
        prompt: &str,
        turn: u64,
    ) -> impl Future<Output = PreStepVerdict> + Send;

    /// PreToolUse:tool/call 落档后、工具执行前(取消安全点之后)
    fn pre_tool(
        &self,
        call: &ToolCallRequest,
        turn: u64,
    ) -> impl Future<Output = PreToolVerdict> + Send;

    /// PostToolUse:工具执行后、tool/result 落档前
    fn post_tool(
        &self,
        call: &ToolCallRequest,
        output: &crate::tools::ToolOutput,
        turn: u64,
    ) -> impl Future<Output = PostToolVerdict> + Send;

    /// Stop:无 tool_calls break 后、turn/end 落档前
    fn on_stop(&self, turn: u64) -> impl Future<Output = StopVerdict> + Send;
}

/// HookPort 的对象安全形态(与 ToolPortObj 同理;引擎经 Box<dyn> 持有)
pub trait HookPortObj: Send + Sync {
    fn on_prompt_submit<'a>(
        &'a self,
        prompt: &'a str,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = PreStepVerdict> + Send + 'a>>;
    fn pre_tool<'a>(
        &'a self,
        call: &'a ToolCallRequest,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = PreToolVerdict> + Send + 'a>>;
    fn post_tool<'a>(
        &'a self,
        call: &'a ToolCallRequest,
        output: &'a crate::tools::ToolOutput,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = PostToolVerdict> + Send + 'a>>;
    fn on_stop<'a>(
        &'a self,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = StopVerdict> + Send + 'a>>;
}

impl<T: HookPort> HookPortObj for T {
    fn on_prompt_submit<'a>(
        &'a self,
        prompt: &'a str,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = PreStepVerdict> + Send + 'a>> {
        Box::pin(HookPort::on_prompt_submit(self, prompt, turn))
    }
    fn pre_tool<'a>(
        &'a self,
        call: &'a ToolCallRequest,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = PreToolVerdict> + Send + 'a>> {
        Box::pin(HookPort::pre_tool(self, call, turn))
    }
    fn post_tool<'a>(
        &'a self,
        call: &'a ToolCallRequest,
        output: &'a crate::tools::ToolOutput,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = PostToolVerdict> + Send + 'a>> {
        Box::pin(HookPort::post_tool(self, call, output, turn))
    }
    fn on_stop<'a>(
        &'a self,
        turn: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = StopVerdict> + Send + 'a>> {
        Box::pin(HookPort::on_stop(self, turn))
    }
}

/// 注入上下文的 source 染色(mislabel guard:kind=plugin;
/// RS 面沿用 source.kind 字符串,宿主 translate 原样透传)
pub fn hook_context_source(dialect: &str) -> Value {
    serde_json::json!({ "kind": "plugin", "plugin": dialect })
}

/// 多钩子组合器:按序运行四点,取「最严格者胜」。
///
/// 引擎单槽(`Option<Arc<dyn HookPortObj>>`)持有钩子;多来源
/// (hooks 桥 + 决策场景哨兵/守卫)并存时经此合并下发。合并语义:
/// prompt **首个 Reject 胜**;pre_tool **首个 Deny 胜**(其后实现点
/// 不再执行——与引擎 deny 短路 post_tool 的语义一致);post_tool
/// **首个 Block 胜**,多个 Inject 拼接为一条;on_stop **首个 Continue 胜**。
/// 全部 Pass 时直通默认裁决。
pub struct HookChain {
    ports: Vec<std::sync::Arc<dyn HookPortObj>>,
}

impl HookChain {
    /// 按优先序组装(先到先裁决)
    pub fn new(ports: Vec<std::sync::Arc<dyn HookPortObj>>) -> Self {
        Self { ports }
    }
}

impl HookPort for HookChain {
    async fn on_prompt_submit(&self, prompt: &str, turn: u64) -> PreStepVerdict {
        for port in &self.ports {
            if let PreStepVerdict::Reject = port.on_prompt_submit(prompt, turn).await {
                return PreStepVerdict::Reject;
            }
        }
        PreStepVerdict::Proceed
    }

    async fn pre_tool(&self, call: &ToolCallRequest, turn: u64) -> PreToolVerdict {
        for port in &self.ports {
            if let PreToolVerdict::Deny { reason } = port.pre_tool(call, turn).await {
                return PreToolVerdict::Deny { reason };
            }
        }
        PreToolVerdict::Proceed
    }

    async fn post_tool(
        &self,
        call: &ToolCallRequest,
        output: &crate::tools::ToolOutput,
        turn: u64,
    ) -> PostToolVerdict {
        let mut injects: Vec<String> = Vec::new();
        for port in &self.ports {
            match port.post_tool(call, output, turn).await {
                PostToolVerdict::Block { feedback } => {
                    return PostToolVerdict::Block { feedback };
                }
                PostToolVerdict::Inject { text } => injects.push(text),
                PostToolVerdict::Pass => {}
            }
        }
        if injects.is_empty() {
            PostToolVerdict::Pass
        } else {
            PostToolVerdict::Inject {
                text: injects.join("\n\n"),
            }
        }
    }

    async fn on_stop(&self, turn: u64) -> StopVerdict {
        for port in &self.ports {
            if let StopVerdict::Continue { reason } = port.on_stop(turn).await {
                return StopVerdict::Continue { reason };
            }
        }
        StopVerdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use crate::hooks::HookPortObj;
    use crate::hooks::{
        HookChain, HookPort, PostToolVerdict, PreStepVerdict, PreToolVerdict, StopVerdict,
    };
    use crate::tools::{ToolCallRequest, ToolOutput};
    use std::sync::{Arc, Mutex};

    /// 记录调用点的测试 HookPort(锁事件序断言)
    #[derive(Default)]
    struct RecordingHook {
        calls: Mutex<Vec<String>>,
        deny_tool: bool,
        reject_prompt: bool,
        continue_stop: bool,
    }

    impl RecordingHook {
        fn note(&self, s: &str) {
            self.calls.lock().unwrap().push(s.to_string());
        }
    }

    impl HookPort for RecordingHook {
        async fn on_prompt_submit(&self, _prompt: &str, turn: u64) -> PreStepVerdict {
            self.note(&format!("prompt-submit:{turn}"));
            if self.reject_prompt {
                PreStepVerdict::Reject
            } else {
                PreStepVerdict::Proceed
            }
        }
        async fn pre_tool(&self, call: &ToolCallRequest, turn: u64) -> PreToolVerdict {
            self.note(&format!("pre-tool:{}:{turn}", call.name));
            if self.deny_tool {
                PreToolVerdict::Deny {
                    reason: "policy says no".into(),
                }
            } else {
                PreToolVerdict::Proceed
            }
        }
        async fn post_tool(
            &self,
            _call: &ToolCallRequest,
            _output: &ToolOutput,
            turn: u64,
        ) -> PostToolVerdict {
            self.note(&format!("post-tool:{turn}"));
            PostToolVerdict::Pass
        }
        async fn on_stop(&self, turn: u64) -> StopVerdict {
            self.note(&format!("stop:{turn}"));
            if self.continue_stop {
                StopVerdict::Continue {
                    reason: "keep going".into(),
                }
            } else {
                StopVerdict::Pass
            }
        }
    }

    #[tokio::test]
    async fn hook_port_obj_dispatches_all_four_points() {
        let hook = Arc::new(RecordingHook {
            deny_tool: false,
            reject_prompt: false,
            continue_stop: false,
            calls: Mutex::new(Vec::new()),
        });
        let obj: Arc<dyn HookPortObj> = hook.clone();
        assert_eq!(
            HookPortObj::on_prompt_submit(&*obj, "hi", 1).await,
            PreStepVerdict::Proceed
        );
        let call = ToolCallRequest {
            name: "bash".into(),
            arguments: serde_json::json!({}),
            id: String::new(),
        };
        assert_eq!(
            HookPortObj::pre_tool(&*obj, &call, 1).await,
            PreToolVerdict::Proceed
        );
        let out = ToolOutput::default();
        assert_eq!(
            HookPortObj::post_tool(&*obj, &call, &out, 1).await,
            PostToolVerdict::Pass
        );
        assert_eq!(HookPortObj::on_stop(&*obj, 1).await, StopVerdict::Pass);
        assert_eq!(
            *hook.calls.lock().unwrap(),
            vec![
                "prompt-submit:1".to_string(),
                "pre-tool:bash:1".to_string(),
                "post-tool:1".to_string(),
                "stop:1".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn deny_and_reject_verdicts_carry_reasons() {
        let hook = RecordingHook {
            deny_tool: true,
            reject_prompt: true,
            continue_stop: true,
            calls: Default::default(),
        };
        assert_eq!(
            HookPort::on_prompt_submit(&hook, "x", 2).await,
            PreStepVerdict::Reject
        );
        let call = ToolCallRequest {
            name: "rm".into(),
            arguments: serde_json::json!({}),
            id: String::new(),
        };
        assert_eq!(
            HookPort::pre_tool(&hook, &call, 2).await,
            PreToolVerdict::Deny {
                reason: "policy says no".into()
            }
        );
        assert_eq!(
            HookPort::on_stop(&hook, 2).await,
            StopVerdict::Continue {
                reason: "keep going".into()
            }
        );
    }

    #[test]
    fn hook_context_source_labels_plugin() {
        let v = crate::hooks::hook_context_source("hooks-claude-code");
        assert_eq!(v["kind"], "plugin");
        assert_eq!(v["plugin"], "hooks-claude-code");
    }

    /// HookChain 合并语义:strict 胜出后 lenient 不再执行;
    /// post_tool 多 Inject 拼接、Block 优先;全 Pass 直通。
    #[tokio::test]
    async fn chain_merges_strictest_verdict() {
        use std::sync::Arc as StdArc;
        struct Strict {
            calls: Mutex<Vec<String>>,
            deny: bool,
            continue_stop: bool,
            inject: Option<&'static str>,
        }
        struct Lenient {
            calls: Mutex<Vec<String>>,
            inject: Option<&'static str>,
        }
        impl HookPort for Strict {
            async fn on_prompt_submit(&self, _p: &str, t: u64) -> PreStepVerdict {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("strict-prompt:{t}"));
                PreStepVerdict::Proceed
            }
            async fn pre_tool(&self, _c: &ToolCallRequest, t: u64) -> PreToolVerdict {
                self.calls.lock().unwrap().push(format!("strict-pre:{t}"));
                if self.deny {
                    PreToolVerdict::Deny {
                        reason: "strict denies".into(),
                    }
                } else {
                    PreToolVerdict::Proceed
                }
            }
            async fn post_tool(
                &self,
                _c: &ToolCallRequest,
                _o: &ToolOutput,
                t: u64,
            ) -> PostToolVerdict {
                self.calls.lock().unwrap().push(format!("strict-post:{t}"));
                match self.inject {
                    Some(text) => PostToolVerdict::Inject { text: text.into() },
                    None => PostToolVerdict::Pass,
                }
            }
            async fn on_stop(&self, t: u64) -> StopVerdict {
                self.calls.lock().unwrap().push(format!("strict-stop:{t}"));
                if self.continue_stop {
                    StopVerdict::Continue {
                        reason: "strict continues".into(),
                    }
                } else {
                    StopVerdict::Pass
                }
            }
        }
        impl HookPort for Lenient {
            async fn on_prompt_submit(&self, _p: &str, _t: u64) -> PreStepVerdict {
                self.calls.lock().unwrap().push("lenient-prompt".into());
                PreStepVerdict::Proceed
            }
            async fn pre_tool(&self, _c: &ToolCallRequest, _t: u64) -> PreToolVerdict {
                self.calls.lock().unwrap().push("lenient-pre".into());
                PreToolVerdict::Proceed
            }
            async fn post_tool(
                &self,
                _c: &ToolCallRequest,
                _o: &ToolOutput,
                _t: u64,
            ) -> PostToolVerdict {
                match self.inject {
                    Some(text) => PostToolVerdict::Inject { text: text.into() },
                    None => PostToolVerdict::Pass,
                }
            }
            async fn on_stop(&self, _t: u64) -> StopVerdict {
                StopVerdict::Pass
            }
        }

        let strict = StdArc::new(Strict {
            calls: Mutex::new(Vec::new()),
            deny: true,
            continue_stop: true,
            inject: Some("strict note"),
        });
        let lenient = StdArc::new(Lenient {
            calls: Mutex::new(Vec::new()),
            inject: Some("lenient note"),
        });
        let ports: Vec<StdArc<dyn HookPortObj>> = vec![strict.clone(), lenient.clone()];
        let chain = HookChain::new(ports);
        let call = ToolCallRequest {
            name: "bash".into(),
            arguments: serde_json::json!({}),
            id: String::new(),
        };
        let out = ToolOutput::default();

        // pre_tool:首个 Deny 胜,lenient 不再执行
        assert_eq!(
            HookPort::pre_tool(&chain, &call, 1).await,
            PreToolVerdict::Deny {
                reason: "strict denies".into()
            }
        );
        assert_eq!(
            *lenient.calls.lock().unwrap(),
            Vec::<String>::new(),
            "deny 后续点短路"
        );

        // post_tool:多 Inject 拼接为一条
        assert_eq!(
            HookPort::post_tool(&chain, &call, &out, 1).await,
            PostToolVerdict::Inject {
                text: "strict note\n\nlenient note".into()
            }
        );

        // on_stop:首个 Continue 胜
        assert_eq!(
            HookPort::on_stop(&chain, 1).await,
            StopVerdict::Continue {
                reason: "strict continues".into()
            }
        );

        // 全 Pass:prompt 两点都执行,直通 Proceed
        assert_eq!(
            HookPort::on_prompt_submit(&chain, "x", 1).await,
            PreStepVerdict::Proceed
        );
        assert_eq!(
            *strict.calls.lock().unwrap(),
            vec![
                "strict-pre:1".to_string(),
                "strict-post:1".to_string(),
                "strict-stop:1".to_string(),
                "strict-prompt:1".to_string(),
            ]
        );
        assert_eq!(
            *lenient.calls.lock().unwrap(),
            vec!["lenient-prompt".to_string()]
        );
    }
}
