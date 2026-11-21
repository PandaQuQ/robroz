//! Chat messages, tool definitions, and the `/chat/completions` shapes
//! used for function calling.

use serde_json::{json, Value};

use crate::ModelError;

/// A tool the model may call: name, description, and a JSON Schema for arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// One function call the model asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments, as the model returned them (usually a string).
    pub arguments: String,
}

/// What the model said in one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantTurn {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// One message in a tool-calling conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatMessage {
    System(String),
    User(String),
    Assistant(AssistantTurn),
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl ChatMessage {
    fn to_json(&self) -> Value {
        match self {
            ChatMessage::System(content) => json!({ "role": "system", "content": content }),
            ChatMessage::User(content) => json!({ "role": "user", "content": content }),
            ChatMessage::Assistant(AssistantTurn::Text(content)) => {
                json!({ "role": "assistant", "content": content })
            }
            ChatMessage::Assistant(AssistantTurn::ToolCalls(calls)) => json!({
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": calls.iter().map(ToolCall::to_json).collect::<Vec<_>>(),
            }),
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => {
                json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content })
            }
        }
    }
}

impl ToolCall {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "type": "function",
            "function": { "name": self.name, "arguments": self.arguments },
        })
    }
}

impl ToolDefinition {
    fn to_json(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            },
        })
    }
}

/// The `/chat/completions` body, with `tools` only when the list is not empty.
pub fn chat_body(model: &str, messages: &[ChatMessage], tools: &[ToolDefinition]) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages.iter().map(ChatMessage::to_json).collect::<Vec<_>>(),
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(ToolDefinition::to_json).collect());
    }
    body
}

/// `choices[0].message`: a text reply, or one or more tool calls.
pub fn parse_turn(body: &str) -> Result<AssistantTurn, ModelError> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| ModelError("模型返回的不是 JSON".into()))?;
    let message = value
        .pointer("/choices/0/message")
        .ok_or_else(|| ModelError("模型返回里没有 choices[0].message".into()))?;
    let calls = parse_tool_calls(message.get("tool_calls"))?;
    if !calls.is_empty() {
        return Ok(AssistantTurn::ToolCalls(calls));
    }
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| ModelError("模型返回里没有 choices[0].message.content".into()))?;
    if content.is_empty() {
        return Err(ModelError("模型返回了空内容".into()));
    }
    Ok(AssistantTurn::Text(content.to_string()))
}

fn parse_tool_calls(value: Option<&Value>) -> Result<Vec<ToolCall>, ModelError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let array = value
        .as_array()
        .ok_or_else(|| ModelError("模型返回的 tool_calls 不是数组".into()))?;
    let mut calls = Vec::with_capacity(array.len());
    for (index, item) in array.iter().enumerate() {
        let typ = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if typ != "function" {
            return Err(ModelError(format!("不支持的 tool_call 类型：{typ}")));
        }
        let function = item.get("function").unwrap_or(item);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ModelError("工具调用里没有 name".into()))?;
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("call_{index}"));
        let arguments = match function.get("arguments") {
            None => "{}".to_string(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        };
        calls.push(ToolCall {
            id,
            name: name.to_string(),
            arguments,
        });
    }
    Ok(calls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SYSTEM_PROMPT;

    fn echo_tool() -> ToolDefinition {
        ToolDefinition {
            name: "echo".into(),
            description: "原样返回".into(),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            }),
        }
    }

    #[test]
    fn body_omits_tools_when_the_list_is_empty() {
        let body = chat_body(
            "m1",
            &[
                ChatMessage::System(SYSTEM_PROMPT.into()),
                ChatMessage::User("写周报".into()),
            ],
            &[],
        );
        assert_eq!(body["model"], "m1");
        assert_eq!(body["messages"][0]["content"], SYSTEM_PROMPT);
        assert_eq!(
            body["messages"][1],
            json!({ "role": "user", "content": "写周报" })
        );
        assert!(body.get("tools").is_none(), "{body}");
    }

    #[test]
    fn body_lists_tools_and_tool_results() {
        let calls = vec![ToolCall {
            id: "call_1".into(),
            name: "echo".into(),
            arguments: r#"{"text":"hi"}"#.into(),
        }];
        let body = chat_body(
            "m1",
            &[
                ChatMessage::User("叫一下 echo".into()),
                ChatMessage::Assistant(AssistantTurn::ToolCalls(calls)),
                ChatMessage::Tool {
                    tool_call_id: "call_1".into(),
                    content: "hi".into(),
                },
            ],
            &[echo_tool()],
        );
        assert_eq!(
            body["tools"][0],
            json!({
                "type": "function",
                "function": {
                    "name": "echo",
                    "description": "原样返回",
                    "parameters": echo_tool().parameters,
                },
            })
        );
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert!(body["messages"][1]["content"].is_null());
        assert_eq!(body["messages"][1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["name"],
            "echo"
        );
        assert_eq!(
            body["messages"][2],
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "hi" })
        );
    }

    #[test]
    fn parse_turn_reads_text_and_tool_calls() {
        assert_eq!(
            parse_turn(r#"{"choices":[{"message":{"content":"  好的\n"}}]}"#).unwrap(),
            AssistantTurn::Text("好的".into())
        );
        let turn = parse_turn(
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"echo","arguments":"{\"text\":\"hi\"}"}}]}}]}"#,
        )
        .unwrap();
        match turn {
            AssistantTurn::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "c1");
                assert_eq!(calls[0].name, "echo");
                assert_eq!(calls[0].arguments, r#"{"text":"hi"}"#);
            }
            other => panic!("{other:?}"),
        }
        let object_args = parse_turn(
            r#"{"choices":[{"message":{"tool_calls":[{"function":{"name":"echo","arguments":{"text":"hi"}}}]}}]}"#,
        )
        .unwrap();
        match object_args {
            AssistantTurn::ToolCalls(calls) => {
                assert_eq!(calls[0].id, "call_0");
                assert_eq!(calls[0].arguments, r#"{"text":"hi"}"#);
            }
            other => panic!("{other:?}"),
        }
        assert!(parse_turn("not json").is_err());
        assert!(parse_turn(r#"{"choices":[]}"#).is_err());
        assert!(parse_turn(r#"{"choices":[{"message":{"content":"  "}}]}"#).is_err());
        assert!(parse_turn(r#"{"choices":[{"message":{"tool_calls":"nope"}}]}"#).is_err());
    }
}
