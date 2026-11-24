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
pub const ROUND_LIMIT_FAILURE: &str = robroz_contract::host_text::TOOL_ROUND_LIMIT;

/// Metadata an MCP tool attaches so a Takeover call can raise an approval card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpApproval {
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
}

/// A host-side tool the model may call.
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema for the arguments object.
    fn schema(&self) -> Value;
    /// The side effect [`robroz_policy::tier`] uses to decide whether to run it.
    fn effect(&self) -> SideEffect;
    fn call(&self, args: &Value, ctx: &mut ToolCtx) -> Result<String, String>;
    /// Name shown on the progress line (`在用 …`). Defaults to [`Tool::name`].
    fn display_name(&self) -> &str {
        self.name()
    }
    /// When this is an MCP tool, enough to raise and resume an approval card.
    fn mcp_approval(&self) -> Option<McpApproval> {
        None
    }
}

/// Per-task state for tool calls: outbound quota and setup hints.
#[derive(Debug, Default)]
pub struct ToolCtx {
    pub used: u64,
    pub limit: u64,
    pub needs_setup: Option<String>,
    pub over_quota: Option<robroz_policy::OverQuota>,
}

impl ToolCtx {
    pub fn new(limit: u64) -> Self {
        Self {
            used: 0,
            limit,
            needs_setup: None,
            over_quota: None,
        }
    }

    /// Charge one GET against the same meter the sandbox uses.
    pub fn charge_get(&mut self) -> Result<(), robroz_policy::OverQuota> {
        let over = if robroz_policy::outbound("GET", self.used, self.limit)
            == robroz_policy::Outbound::Fail
        {
            self.used += 1;
            Some(robroz_policy::OverQuota::TooMany {
                used: self.used,
                limit: self.limit,
            })
        } else {
            self.used += 1;
            None
        };
        if let Some(over) = over {
            self.over_quota = Some(over.clone());
            Err(over)
        } else {
            Ok(())
        }
    }

    pub fn mark_needs_setup(&mut self, kind: &str) {
        self.needs_setup = Some(kind.to_string());
    }
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
    /// [`InvokeOutcome::NeedsApproval`] means the tool was not executed.
    pub fn invoke(&self, call: &ToolCall, ctx: &mut ToolCtx) -> InvokeOutcome {
        self.invoke_inner(call, ctx, false)
    }

    /// Run a call that the person already approved, skipping the Takeover gate.
    pub fn invoke_approved(&self, call: &ToolCall, ctx: &mut ToolCtx) -> String {
        match self.invoke_inner(call, ctx, true) {
            InvokeOutcome::Text(text) => text,
            InvokeOutcome::NeedsApproval { .. } => "调用失败：这个工具还需要批准".to_string(),
        }
    }

    fn invoke_inner(&self, call: &ToolCall, ctx: &mut ToolCtx, approved: bool) -> InvokeOutcome {
        let Some(tool) = self.get(&call.name) else {
            return InvokeOutcome::Text(format!("没有这个工具：{}", call.name));
        };
        let args = match serde_json::from_str::<Value>(&call.arguments) {
            Ok(value) => value,
            Err(err) => return InvokeOutcome::Text(format!("参数不是 JSON：{err}")),
        };
        if !approved && tier(tool.effect()) == Tier::Takeover {
            return InvokeOutcome::NeedsApproval {
                display_name: tool.display_name().to_string(),
                args,
                mcp: tool.mcp_approval(),
            };
        }
        let text = match tool.call(&args, ctx) {
            Ok(text) => clip_result(text),
            Err(err) => format!("调用失败：{err}"),
        };
        InvokeOutcome::Text(text)
    }

    fn display_name_of(&self, name: &str) -> String {
        self.get(name)
            .map(|tool| tool.display_name().to_string())
            .unwrap_or_else(|| name.to_string())
    }
}

/// Result of [`ToolRegistry::invoke`].
#[derive(Debug, Clone, PartialEq)]
pub enum InvokeOutcome {
    Text(String),
    NeedsApproval {
        display_name: String,
        args: Value,
        mcp: Option<McpApproval>,
    },
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
#[derive(Debug, Clone, PartialEq)]
pub enum CompleteError {
    Model(ModelError),
    RoundLimit,
    OverQuota(robroz_policy::OverQuota),
    /// The loop stopped so a person can approve a Takeover tool call.
    NeedsApproval(Box<PendingApproval>),
}

impl CompleteError {
    /// The task's `failure` text, and the result the runner will write.
    pub fn failure_text(&self) -> String {
        match self {
            CompleteError::Model(err) => {
                format!("{}{err}", robroz_contract::host_text::MODEL_NO_REPLY_PREFIX)
            }
            CompleteError::RoundLimit => ROUND_LIMIT_FAILURE.to_string(),
            CompleteError::OverQuota(over) => over.failure(),
            CompleteError::NeedsApproval(pending) => {
                format!(
                    "{}{}",
                    robroz_contract::host_text::WAITING_APPROVAL_PREFIX,
                    pending.display_name
                )
            }
        }
    }
}

/// Enough to raise an approval card and, after it is answered, resume the loop.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingApproval {
    /// Messages already sent, including the assistant tool-call turn and any
    /// results filled in before the waiting call.
    pub messages: Vec<ChatMessage>,
    /// The waiting call, then any later calls from the same turn.
    pub remaining: Vec<ToolCall>,
    /// 1-based model-round when the waiting call was issued.
    pub round: usize,
    pub display_name: String,
    pub args: Value,
    pub mcp: Option<McpApproval>,
    pub used: u64,
    pub needs_setup: Option<String>,
}

/// Ask the model, run any tool calls, and fill the results back, until a text
/// reply or [`MAX_TOOL_ROUNDS`]. `on_using` is called with the tool name before
/// each invocation so the status card can say "在用 <工具名>".
pub fn complete(
    model: &dyn Model,
    prompt: &str,
    memory_section: Option<&str>,
    tools: &ToolRegistry,
    ctx: &mut ToolCtx,
    on_using: impl FnMut(&str),
) -> Result<String, CompleteError> {
    let messages = vec![
        ChatMessage::System(system_content(memory_section)),
        ChatMessage::User(prompt.to_string()),
    ];
    complete_loop(model, messages, tools, ctx, on_using, 1, None)
}

/// Resume after a person approved (or after earlier results in the same turn).
/// `approved` is executed first, bypassing the Takeover gate, then `remaining`.
pub fn complete_resume(
    model: &dyn Model,
    messages: Vec<ChatMessage>,
    remaining: &[ToolCall],
    tools: &ToolRegistry,
    ctx: &mut ToolCtx,
    on_using: impl FnMut(&str),
    round: usize,
) -> Result<String, CompleteError> {
    let (first, rest) = remaining
        .split_first()
        .map(|(a, b)| (Some(a), b))
        .unwrap_or((None, remaining));
    complete_loop(
        model,
        messages,
        tools,
        ctx,
        on_using,
        round,
        first.map(|c| (c, rest)),
    )
}

fn complete_loop(
    model: &dyn Model,
    mut messages: Vec<ChatMessage>,
    tools: &ToolRegistry,
    ctx: &mut ToolCtx,
    mut on_using: impl FnMut(&str),
    start_round: usize,
    mut resume: Option<(&ToolCall, &[ToolCall])>,
) -> Result<String, CompleteError> {
    let defs = tools.definitions();
    let mut next_round = start_round;
    if let Some((call, rest)) = resume.take() {
        on_using(&tools.display_name_of(&call.name));
        let content = tools.invoke_approved(call, ctx);
        messages.push(ChatMessage::Tool {
            tool_call_id: call.id.clone(),
            content,
        });
        if let Some(over) = ctx.over_quota.clone() {
            return Err(CompleteError::OverQuota(over));
        }
        fill_calls(rest, tools, ctx, &mut on_using, &mut messages, start_round)?;
        next_round = start_round.saturating_add(1);
    }
    for round in next_round..=MAX_TOOL_ROUNDS {
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
                fill_calls(&calls, tools, ctx, &mut on_using, &mut messages, round)?;
            }
        }
    }
    Err(CompleteError::RoundLimit)
}

fn fill_calls(
    calls: &[ToolCall],
    tools: &ToolRegistry,
    ctx: &mut ToolCtx,
    on_using: &mut impl FnMut(&str),
    messages: &mut Vec<ChatMessage>,
    round: usize,
) -> Result<(), CompleteError> {
    for (index, call) in calls.iter().enumerate() {
        on_using(&tools.display_name_of(&call.name));
        match tools.invoke(call, ctx) {
            InvokeOutcome::Text(content) => {
                messages.push(ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    content,
                });
                if let Some(over) = ctx.over_quota.clone() {
                    return Err(CompleteError::OverQuota(over));
                }
            }
            InvokeOutcome::NeedsApproval {
                display_name,
                args,
                mcp,
            } => {
                return Err(CompleteError::NeedsApproval(Box::new(PendingApproval {
                    messages: messages.clone(),
                    remaining: calls[index..].to_vec(),
                    round,
                    display_name,
                    args,
                    mcp,
                    used: ctx.used,
                    needs_setup: ctx.needs_setup.clone(),
                })));
            }
        }
    }
    Ok(())
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
        fn call(&self, args: &Value, _ctx: &mut ToolCtx) -> Result<String, String> {
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
        let text = complete(
            &model,
            "叫一下 echo",
            None,
            &tools,
            &mut ToolCtx::new(8),
            cb,
        )
        .unwrap();
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
        let text = complete(&model, "跑 boom", None, &tools, &mut ToolCtx::new(8), cb).unwrap();
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
        let InvokeOutcome::Text(out) = tools.invoke(&call("big", "c1", "{}"), &mut ToolCtx::new(8))
        else {
            panic!("expected text");
        };
        assert_eq!(out.chars().count(), MAX_TOOL_RESULT_CHARS);
        assert!(out.ends_with(TRUNCATED_MARK), "{out}");
        let exact = {
            let mut tools = ToolRegistry::new();
            tools.register(FakeTool::ok("fit", &"字".repeat(MAX_TOOL_RESULT_CHARS)));
            let InvokeOutcome::Text(text) =
                tools.invoke(&call("fit", "c1", "{}"), &mut ToolCtx::new(8))
            else {
                panic!("expected text");
            };
            text
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
        let err = complete(&model, "循环", None, &tools, &mut ToolCtx::new(8), cb).unwrap_err();
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
            fn call(&self, args: &Value, ctx: &mut ToolCtx) -> Result<String, String> {
                self.0.call(args, ctx)
            }
        }
        let mut tools = ToolRegistry::new();
        tools.register(Shared(tool.clone()));
        let out = tools.invoke(&call("wire", "c1", "{}"), &mut ToolCtx::new(8));
        assert!(
            matches!(
                out,
                InvokeOutcome::NeedsApproval { ref display_name, .. } if display_name == "wire"
            ),
            "{out:?}"
        );
        assert!(tool.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn complete_stops_for_takeover_and_resume_runs_it() {
        let tool = Arc::new(FakeTool::takeover("wire"));
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
            fn call(&self, args: &Value, ctx: &mut ToolCtx) -> Result<String, String> {
                self.0.call(args, ctx)
            }
        }
        let mut tools = ToolRegistry::new();
        tools.register(Shared(tool.clone()));
        let model = ScriptedModel::new(vec![
            AssistantTurn::ToolCalls(vec![call("wire", "c1", r#"{"x":1}"#)]),
            AssistantTurn::Text("写好了".into()),
        ]);
        let (cb, used) = using();
        let err = complete(&model, "写", None, &tools, &mut ToolCtx::new(8), cb).unwrap_err();
        let CompleteError::NeedsApproval(pending) = err else {
            panic!("expected NeedsApproval, got {err:?}");
        };
        assert_eq!(pending.display_name, "wire");
        assert!(tool.calls.lock().unwrap().is_empty());
        assert_eq!(used.lock().unwrap().as_slice(), &["wire".to_string()]);

        let model = ScriptedModel::new(vec![AssistantTurn::Text("写好了".into())]);
        let (cb, _) = using();
        let mut ctx = ToolCtx::new(8);
        ctx.used = pending.used;
        let text = complete_resume(
            &model,
            pending.messages,
            &pending.remaining,
            &tools,
            &mut ctx,
            cb,
            pending.round,
        )
        .unwrap();
        assert_eq!(text, "写好了");
        assert_eq!(tool.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn unknown_tool_and_bad_json_are_filled_back() {
        let tools = ToolRegistry::new();
        assert_eq!(
            tools.invoke(&call("nope", "c1", "{}"), &mut ToolCtx::new(8)),
            InvokeOutcome::Text("没有这个工具：nope".into())
        );
        let mut tools = ToolRegistry::new();
        tools.register(FakeTool::ok("echo", "hi"));
        let out = tools.invoke(&call("echo", "c1", "not-json"), &mut ToolCtx::new(8));
        let InvokeOutcome::Text(out) = out else {
            panic!("{out:?}");
        };
        assert!(out.starts_with("参数不是 JSON："), "{out}");
        assert_eq!(
            tools.invoke(&call("echo", "c1", "{}"), &mut ToolCtx::new(8)),
            InvokeOutcome::Text("hi".into())
        );
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
        assert_eq!(
            complete(&model, "循环", None, &tools, &mut ToolCtx::new(8), cb).unwrap(),
            "好了"
        );
        assert_eq!(used.lock().unwrap().len(), MAX_TOOL_ROUNDS - 1);
    }
}
