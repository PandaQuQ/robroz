//! Tool calling on the host: a registry, a policy check, and the model loop.
//!
//! No real tools are registered in this crate yet. An empty registry means the
//! model request does not carry `tools`, and the task path is unchanged.

use std::sync::{Arc, OnceLock};

use robroz_model::{
    system_content, AssistantTurn, ChatMessage, Model, ModelError, ToolCall, ToolDefinition,
};
use robroz_policy::{tier, SideEffect, Tier};
use serde_json::Value;

/// Max model calls in one task. If the last one is still a tool call, the task
/// finishes as done with [`ROUND_LIMIT_FAILURE`].
pub const MAX_TOOL_ROUNDS: usize = 8;

/// Longest tool result, in characters, including the truncation marker.
pub const MAX_TOOL_RESULT_CHARS: usize = 16_000;

const TRUNCATED_MARK: &str = "\n…（已截断）";

/// Why a tool-calling loop stopped without a text reply.
pub const ROUND_LIMIT_FAILURE: &str = "工具调用超过 8 轮";

/// Returned to the model when policy says the tool needs approval or Takeover.
/// Approval cards come in a later PR; this PR does not run those tools.
pub const APPROVAL_DENIED: &str = "这个工具需要用户批准，目前还不能调用。";

/// A host-side tool the model may call.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema for the arguments object.
    fn schema(&self) -> Value;
    /// The side effect [`robroz_policy::tier`] uses to decide whether to run it.
    fn effect(&self) -> SideEffect;
    fn call(&self, args: &Value) -> Result<String, String>;
}

/// Named tools available to one pump pass. Empty by default.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A process-wide empty registry, for call sites that never register tools.
    pub fn empty() -> &'static Self {
        static EMPTY: OnceLock<ToolRegistry> = OnceLock::new();
        EMPTY.get_or_init(ToolRegistry::new)
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        let tool = Arc::new(tool);
        if let Some(existing) = self
            .tools
            .iter_mut()
            .find(|item| item.name() == tool.name())
        {
            *existing = tool;
        } else {
            self.tools.push(tool);
        }
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.schema(),
            })
            .collect()
    }

    fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(|tool| tool.as_ref())
    }

    /// Run one call and turn every outcome into text the model can read back.
    pub fn invoke(&self, call: &ToolCall) -> String {
        let Some(tool) = self.get(&call.name) else {
            return format!("没有这个工具：{}", call.name);
        };
        if tier(tool.effect()) == Tier::Takeover {
            return APPROVAL_DENIED.to_string();
        }
        let args = match serde_json::from_str::<Value>(&call.arguments) {
            Ok(value) => value,
            Err(err) => return format!("参数不是 JSON：{err}"),
        };
        match tool.call(&args) {
            Ok(text) => clip_result(text),
            Err(err) => format!("调用失败：{err}"),
        }
    }
}

fn clip_result(text: String) -> String {
    if text.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return text;
    }
    let keep = MAX_TOOL_RESULT_CHARS.saturating_sub(TRUNCATED_MARK.chars().count());
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(TRUNCATED_MARK);
    out
}

/// Why [`complete`] did not produce a text reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteError {
    Model(ModelError),
    RoundLimit,
}

impl CompleteError {
    /// The task's `failure` text, and the result the runner will write.
    pub fn failure_text(&self) -> String {
        match self {
            CompleteError::Model(err) => format!("模型没回：{err}"),
            CompleteError::RoundLimit => ROUND_LIMIT_FAILURE.to_string(),
        }
    }
}

/// Ask the model, run any tool calls, and fill the results back, until a text
/// reply or [`MAX_TOOL_ROUNDS`]. `on_using` is called with the tool name before
/// each invocation so the status card can say "在用 <工具名>".
pub fn complete(
    model: &dyn Model,
    prompt: &str,
    memory_section: Option<&str>,
    tools: &ToolRegistry,
    mut on_using: impl FnMut(&str),
) -> Result<String, CompleteError> {
    let mut messages = vec![
        ChatMessage::System(system_content(memory_section)),
        ChatMessage::User(prompt.to_string()),
    ];
    let defs = tools.definitions();
    for round in 1..=MAX_TOOL_ROUNDS {
        let turn = model.chat(&messages, &defs).map_err(CompleteError::Model)?;
        match turn {
            AssistantTurn::Text(text) => return Ok(text),
            AssistantTurn::ToolCalls(calls) => {
                if round == MAX_TOOL_ROUNDS {
                    return Err(CompleteError::RoundLimit);
                }
                messages.push(ChatMessage::Assistant(AssistantTurn::ToolCalls(
                    calls.clone(),
                )));
                for call in &calls {
                    on_using(&call.name);
                    let content = tools.invoke(call);
                    messages.push(ChatMessage::Tool {
                        tool_call_id: call.id.clone(),
                        content,
                    });
                }
            }
        }
    }
    Err(CompleteError::RoundLimit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct ScriptedModel {
        turns: Mutex<Vec<AssistantTurn>>,
        seen: Mutex<Vec<(Vec<ChatMessage>, Vec<ToolDefinition>)>>,
    }

    impl ScriptedModel {
        fn new(turns: Vec<AssistantTurn>) -> Self {
            Self {
                turns: Mutex::new(turns),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl Model for ScriptedModel {
        fn reply(&self, _prompt: &str, _memory: Option<&str>) -> Result<String, ModelError> {
            Err(ModelError("测试里应该走 chat".into()))
        }

        fn chat(
            &self,
            messages: &[ChatMessage],
            tools: &[ToolDefinition],
        ) -> Result<AssistantTurn, ModelError> {
            self.seen
                .lock()
                .unwrap()
                .push((messages.to_vec(), tools.to_vec()));
            let mut turns = self.turns.lock().unwrap();
            if turns.is_empty() {
                return Err(ModelError("模型没有更多返回".into()));
            }
            Ok(turns.remove(0))
        }
    }

    struct FakeTool {
        name: &'static str,
        effect: SideEffect,
        calls: Mutex<Vec<Value>>,
        result: Result<String, String>,
    }

    impl FakeTool {
        fn ok(name: &'static str, result: &str) -> Self {
            Self {
                name,
                effect: SideEffect::WorkspaceRead,
                calls: Mutex::new(Vec::new()),
                result: Ok(result.into()),
            }
        }

        fn failing(name: &'static str, err: &str) -> Self {
            Self {
                name,
                effect: SideEffect::WorkspaceRead,
                calls: Mutex::new(Vec::new()),
                result: Err(err.into()),
            }
        }

        fn takeover(name: &'static str) -> Self {
            Self {
                name,
                effect: SideEffect::ExternalWrite,
                calls: Mutex::new(Vec::new()),
                result: Ok("should not run".into()),
            }
        }
    }

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            self.name
        }
        fn schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        fn effect(&self) -> SideEffect {
            self.effect
        }
        fn call(&self, args: &Value) -> Result<String, String> {
            self.calls.lock().unwrap().push(args.clone());
            self.result.clone()
        }
    }

    fn call(name: &str, id: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args.into(),
        }
    }

    fn using() -> (impl FnMut(&str), Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let slot = seen.clone();
        let cb = move |name: &str| slot.lock().unwrap().push(name.to_string());
        (cb, seen)
    }

    #[test]
    fn loop_runs_a_tool_and_returns_the_text_reply() {
        let model = ScriptedModel::new(vec![
            AssistantTurn::ToolCalls(vec![call("echo", "c1", r#"{"text":"hi"}"#)]),
            AssistantTurn::Text("收到 hi".into()),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::ok("echo", "hi"));
        let (cb, used) = using();
        let text = complete(&model, "叫一下 echo", None, &tools, cb).unwrap();
        assert_eq!(text, "收到 hi");
        assert_eq!(*used.lock().unwrap(), ["echo"]);
        let seen = model.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].1[0].name, "echo", "tools are sent on every call");
        match &seen[1].0[..] {
            [ChatMessage::System(_), ChatMessage::User(prompt), ChatMessage::Assistant(AssistantTurn::ToolCalls(calls)), ChatMessage::Tool {
                tool_call_id,
                content,
            }] => {
                assert_eq!(prompt, "叫一下 echo");
                assert_eq!(calls[0].id, "c1");
                assert_eq!(tool_call_id, "c1");
                assert_eq!(content, "hi");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_registry_has_no_definitions() {
        assert!(ToolRegistry::empty().is_empty());
        assert!(ToolRegistry::empty().definitions().is_empty());
        assert!(ToolRegistry::new().is_empty());
    }

    #[test]
    fn tool_error_is_filled_back_and_the_model_can_finish() {
        let model = ScriptedModel::new(vec![
            AssistantTurn::ToolCalls(vec![call("boom", "c1", "{}")]),
            AssistantTurn::Text("工具出错了".into()),
        ]);
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::failing("boom", "磁盘满了"));
        let (cb, _) = using();
        let text = complete(&model, "跑 boom", None, &tools, cb).unwrap();
        assert_eq!(text, "工具出错了");
        let seen = model.seen.lock().unwrap();
        match &seen[1].0.last() {
            Some(ChatMessage::Tool { content, .. }) => {
                assert_eq!(content, "调用失败：磁盘满了");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn long_result_is_truncated() {
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::ok(
            "big",
            &"字".repeat(MAX_TOOL_RESULT_CHARS + 50),
        ));
        let out = tools.invoke(&call("big", "c1", "{}"));
        assert_eq!(out.chars().count(), MAX_TOOL_RESULT_CHARS);
        assert!(out.ends_with(TRUNCATED_MARK), "{out}");
        let exact = {
            let mut tools = ToolRegistry::new();
            tools.register(FakeTool::ok("fit", &"字".repeat(MAX_TOOL_RESULT_CHARS)));
            tools.invoke(&call("fit", "c1", "{}"))
        };
        assert_eq!(exact.chars().count(), MAX_TOOL_RESULT_CHARS);
        assert!(!exact.contains("已截断"), "{exact}");
    }

    #[test]
    fn round_limit_stops_after_eight_model_calls() {
        let turns = (0..MAX_TOOL_ROUNDS)
            .map(|i| AssistantTurn::ToolCalls(vec![call("echo", &format!("c{i}"), "{}")]))
            .collect();
        let model = ScriptedModel::new(turns);
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::ok("echo", "ok"));
        let (cb, used) = using();
        let err = complete(&model, "循环", None, &tools, cb).unwrap_err();
        assert_eq!(err, CompleteError::RoundLimit);
        assert_eq!(err.failure_text(), ROUND_LIMIT_FAILURE);
        assert_eq!(model.seen.lock().unwrap().len(), MAX_TOOL_ROUNDS);
        // The last (8th) tool call is not executed.
        assert_eq!(used.lock().unwrap().len(), MAX_TOOL_ROUNDS - 1);
    }

    #[test]
    fn takeover_does_not_call_the_tool() {
        let tool = Arc::new(FakeTool::takeover("wire"));
        // ToolRegistry takes impl Tool, so wrap.
        struct Shared(Arc<FakeTool>);
        impl Tool for Shared {
            fn name(&self) -> &str {
                self.0.name()
            }
            fn description(&self) -> &str {
                self.0.description()
            }
            fn schema(&self) -> Value {
                self.0.schema()
            }
            fn effect(&self) -> SideEffect {
                self.0.effect()
            }
            fn call(&self, args: &Value) -> Result<String, String> {
                self.0.call(args)
            }
        }
        let mut tools = ToolRegistry::new();
        tools.register(Shared(tool.clone()));
        assert_eq!(tools.invoke(&call("wire", "c1", "{}")), APPROVAL_DENIED);
        assert!(tool.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn unknown_tool_and_bad_json_are_filled_back() {
        let tools = ToolRegistry::new();
        assert_eq!(
            tools.invoke(&call("nope", "c1", "{}")),
            "没有这个工具：nope"
        );
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::ok("echo", "hi"));
        let out = tools.invoke(&call("echo", "c1", "not-json"));
        assert!(out.starts_with("参数不是 JSON："), "{out}");
        assert_eq!(tools.invoke(&call("echo", "c1", "{}")), "hi");
    }

    #[test]
    fn eighth_round_text_reply_is_accepted() {
        let mut turns: Vec<AssistantTurn> = (0..MAX_TOOL_ROUNDS - 1)
            .map(|i| AssistantTurn::ToolCalls(vec![call("echo", &format!("c{i}"), "{}")]))
            .collect();
        turns.push(AssistantTurn::Text("好了".into()));
        let model = ScriptedModel::new(turns);
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::ok("echo", "ok"));
        let (cb, used) = using();
        assert_eq!(complete(&model, "循环", None, &tools, cb).unwrap(), "好了");
        assert_eq!(used.lock().unwrap().len(), MAX_TOOL_ROUNDS - 1);
    }
}
