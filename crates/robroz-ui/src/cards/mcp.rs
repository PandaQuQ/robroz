//! MCP servers page (robroz#81 `/api/mcp/servers`): list, add, edit,
//! delete (with a confirm), test the connection, inspect tools, and
//! enable a server in a chat.

use std::collections::{HashMap, HashSet};

use robroz_contract::{McpSecretIn, McpServer, McpTestResult, McpTool};

use super::secret::MaskedKey;

pub const TITLE: &str = "MCP";
pub const ADD: &str = "加一台";
pub const EMPTY_TITLE: &str = "还没有 MCP 服务器";
pub const EMPTY_HINT: &str = "点「加一台」接上 bro 可以用的工具服务";
pub const SAVE: &str = "保存";
pub const CANCEL: &str = "取消";
pub const DELETE: &str = "删除";
pub const DELETE_ASK: &str = "删除这台服务器？群里也不会再用到它。";
pub const EDIT: &str = "改";
pub const STDIO: &str = "stdio";
pub const HTTP: &str = "HTTP";
pub const NEEDS_APPROVAL: &str = "要你批准";
pub const GROUP_ENABLE: &str = "在这些群里用";
pub const LOADING: &str = "读取中";
pub const LOAD_FAILED: &str = "读不到";
pub const TESTING: &str = "试连中…";
pub const WORKS: &str = "能用了";
pub const ADD_SECRET: &str = "加一项密钥";
pub const REMOVE_SECRET: &str = "去掉";
pub const NAME_PLACEHOLDER: &str = "名称";
pub const COMMAND_PLACEHOLDER: &str = "启动命令";
pub const ARGS_PLACEHOLDER: &str = "参数，用空格分开";
pub const URL_PLACEHOLDER: &str = "地址，如 http://127.0.0.1:3000";
pub const SECRET_NAME_PLACEHOLDER: &str = "密钥名称";
pub const SECRET_KEY_PLACEHOLDER: &str = "密钥";
/// Small labels above form fields, so a filled box is still identifiable.
pub const LABEL_NAME: &str = "名称";
pub const LABEL_COMMAND: &str = "命令";
pub const LABEL_ARGS: &str = "参数（用空格分开）";
pub const LABEL_URL: &str = "地址";
pub const LABEL_SECRET_NAME: &str = "密钥名称";
pub const LABEL_SECRET_KEY: &str = "密钥";
/// Shorter labels in the 409 command-approval block.
pub const LABEL_PROMPT_COMMAND: &str = "命令";
pub const LABEL_PROMPT_ARGS: &str = "参数";
pub const ENV: &str = "环境变量";
pub const HEADER: &str = "请求头";
pub const EMPTY_NAME: &str = "请填写名称";
pub const NAME_TOO_LONG: &str = "名称最多 80 个字";
pub const NEED_COMMAND: &str = "请填写启动命令";
pub const NEED_URL: &str = "请填写地址";
pub const BAD_URL: &str = "地址要以 http:// 或 https:// 开头";
pub const EMPTY_SECRET_NAME: &str = "请填写密钥名称";
pub const BAD_SECRET_NAME: &str = "密钥名称里不能有空白";
pub const NEED_SECRET_KEY: &str = "新增密钥时请填写值";
pub const COMMAND_UNAPPROVED: &str = "命令未批准";
pub const TOOLS_NEED_APPROVAL: &str = "批准命令后才能读取工具";
pub const COMMAND_RUNS_LOCAL: &str = "这条命令会在你的电脑上运行";
pub const APPROVE_RUN: &str = "批准运行";

pub fn cannot_connect(reason: &str) -> String {
    format!("连不上：{reason}")
}

pub fn failed(reason: &str) -> String {
    format!("没保存：{reason}")
}

pub fn tool_count_text(n: Option<u32>) -> String {
    match n {
        Some(n) => format!("{n} 个工具"),
        None => "还没数过".into(),
    }
}

/// Second line on a collapsed server row.
pub fn row_subtitle(s: &McpServer) -> String {
    if command_unapproved(s) {
        TOOLS_NEED_APPROVAL.into()
    } else {
        tool_count_text(s.tool_count)
    }
}

pub fn works_text(count: Option<u32>) -> String {
    match count {
        Some(n) => format!("{WORKS} · {n} 个工具"),
        None => WORKS.into(),
    }
}

/// Stdio server whose current command and args have not been approved.
pub fn command_unapproved(s: &McpServer) -> bool {
    s.transport.eq_ignore_ascii_case("stdio") && s.command_approved != Some(true)
}

/// Command shown in the 409 approval block. Args stay complete, one per line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPrompt {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    #[default]
    Stdio,
    Http,
}

impl Transport {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stdio" => Some(Transport::Stdio),
            "http" => Some(Transport::Http),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Transport::Stdio => STDIO,
            Transport::Http => HTTP,
        }
    }

    pub fn default_placement(self) -> McpSecretIn {
        match self {
            Transport::Stdio => McpSecretIn::Env,
            Transport::Http => McpSecretIn::Header,
        }
    }
}

/// Whether this group's enable list has been fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupLoad {
    #[default]
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestState {
    Running,
    Ok { tool_count: Option<u32> },
    Failed(String),
}

impl TestState {
    pub fn text(&self) -> String {
        match self {
            TestState::Running => TESTING.into(),
            TestState::Ok { tool_count } => works_text(*tool_count),
            TestState::Failed(reason) => cannot_connect(reason),
        }
    }

    pub fn from_result(r: &McpTestResult) -> Self {
        if r.ok {
            TestState::Ok {
                tool_count: r.tool_count,
            }
        } else {
            TestState::Failed(r.error.clone().unwrap_or_else(|| "原因不明".into()))
        }
    }
}

/// One secret row in the add/edit form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretDraft {
    pub name: String,
    pub placement: McpSecretIn,
    pub key: MaskedKey,
}

/// Add or edit form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    /// `None` when adding a new server.
    pub id: Option<String>,
    pub name: String,
    pub transport: Transport,
    pub command: String,
    pub args: String,
    pub url: String,
    pub secrets: Vec<SecretDraft>,
}

/// What 保存 sends to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub id: Option<String>,
    pub name: String,
    pub transport: Transport,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub secrets: Vec<SecretPut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretPut {
    pub name: String,
    pub placement: McpSecretIn,
    pub key: Option<Option<String>>,
}

impl Form {
    pub fn new_add(transport: Transport) -> Self {
        Self {
            id: None,
            name: String::new(),
            transport,
            command: String::new(),
            args: String::new(),
            url: String::new(),
            secrets: Vec::new(),
        }
    }

    pub fn from_server(s: &McpServer) -> Self {
        let transport = Transport::parse(&s.transport).unwrap_or(Transport::Stdio);
        Self {
            id: Some(s.id.clone()),
            name: s.name.clone(),
            transport,
            command: s.command.clone().unwrap_or_default(),
            args: s.args.clone().unwrap_or_default().join(" "),
            url: s.url.clone().unwrap_or_default(),
            secrets: s
                .secrets
                .iter()
                .map(|sec| SecretDraft {
                    name: sec.name.clone(),
                    placement: sec.placement,
                    key: MaskedKey::from_stored(sec.has_key, sec.key_last4.clone()),
                })
                .collect(),
        }
    }

    pub fn problem(&self) -> Option<&'static str> {
        let name = self.name.trim();
        if name.is_empty() {
            return Some(EMPTY_NAME);
        }
        if name.chars().count() > 80 || name.contains('\n') || name.contains('\r') {
            return Some(if name.chars().count() > 80 {
                NAME_TOO_LONG
            } else {
                EMPTY_NAME
            });
        }
        match self.transport {
            Transport::Stdio if self.command.trim().is_empty() => Some(NEED_COMMAND),
            Transport::Http => {
                let url = self.url.trim();
                if url.is_empty() {
                    Some(NEED_URL)
                } else if !url.starts_with("http://") && !url.starts_with("https://") {
                    Some(BAD_URL)
                } else {
                    self.secret_problem()
                }
            }
            Transport::Stdio => self.secret_problem(),
        }
    }

    fn secret_problem(&self) -> Option<&'static str> {
        for s in &self.secrets {
            let name = s.name.trim();
            if name.is_empty() && s.key.to_send().is_none() && !s.key.has_key {
                continue;
            }
            if name.is_empty() {
                return Some(EMPTY_SECRET_NAME);
            }
            if name.chars().any(char::is_whitespace) {
                return Some(BAD_SECRET_NAME);
            }
            let is_new = !s.key.has_key;
            if is_new && s.key.to_send().and_then(|k| k).is_none() {
                return Some(NEED_SECRET_KEY);
            }
        }
        None
    }

    pub fn into_save(&self) -> Option<Save> {
        if self.problem().is_some() {
            return None;
        }
        let mut secrets = Vec::new();
        for s in &self.secrets {
            let name = s.name.trim();
            if name.is_empty() && s.key.to_send().is_none() && !s.key.has_key {
                continue;
            }
            secrets.push(SecretPut {
                name: name.to_string(),
                placement: s.placement,
                key: s.key.to_send(),
            });
        }
        let (command, args, url) = match self.transport {
            Transport::Stdio => {
                let args: Vec<String> = self.args.split_whitespace().map(str::to_string).collect();
                (Some(self.command.trim().to_string()), Some(args), None)
            }
            Transport::Http => (None, None, Some(self.url.trim().to_string())),
        };
        Some(Save {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            transport: self.transport,
            command,
            args,
            url,
            secrets,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    pub servers: Vec<McpServer>,
    /// `(group id, name)` for the per-group enable toggles.
    pub chats: Vec<(String, String)>,
    /// Enabled server ids the host listed for a group, or `Err` if GET failed.
    pub listed: HashMap<String, Result<HashSet<String>, ()>>,
    pub group_load: HashMap<String, GroupLoad>,
    /// Per-group enable, keyed by `(group id, server id)`.
    pub enabled: HashMap<(String, String), bool>,
    /// Expanded server showing its tool list.
    pub open: Option<String>,
    pub tools: HashMap<String, Result<Vec<McpTool>, String>>,
    pub form: Option<Form>,
    pub confirm_delete: Option<String>,
    pub tests: HashMap<String, TestState>,
    /// 409 `command_not_approved`: expand the command under this server row.
    pub command_prompt: HashMap<String, CommandPrompt>,
    pub loaded: bool,
    pub busy: bool,
    pub note: Option<String>,
    pub entry_note: Option<(String, String)>,
    pub form_note: Option<String>,
}

impl Book {
    pub fn load(&mut self, mut servers: Vec<McpServer>) {
        servers.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.servers = servers;
        self.loaded = true;
        if let Some(form) = &self.form {
            if let Some(id) = &form.id {
                if !self.servers.iter().any(|s| s.id == *id) {
                    self.form = None;
                }
            }
        }
        self.paint_listed_groups();
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn group_load_state(&self, group: &str) -> GroupLoad {
        self.group_load
            .get(group)
            .copied()
            .unwrap_or(GroupLoad::Loading)
    }

    pub fn enable_state(&self, group: &str, server: &str) -> Option<bool> {
        self.enabled
            .get(&(group.to_string(), server.to_string()))
            .copied()
    }

    pub fn begin_enable_loads(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, _) in &self.chats {
            self.group_load.insert(id.clone(), GroupLoad::Loading);
            self.listed.remove(id);
            out.push(id.clone());
        }
        self.enabled
            .retain(|(g, _), _| !out.iter().any(|id| id == g));
        out
    }

    pub fn take_groups_to_load(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, _) in &self.chats {
            if self.group_load.contains_key(id) {
                continue;
            }
            self.group_load.insert(id.clone(), GroupLoad::Loading);
            out.push(id.clone());
        }
        out
    }

    pub fn apply_group_mcp(&mut self, group: &str, servers: &[McpServer]) {
        let ids = servers.iter().map(|s| s.id.clone()).collect();
        self.listed.insert(group.to_string(), Ok(ids));
        self.paint_group(group);
    }

    pub fn group_mcp_failed(&mut self, group: &str) {
        self.listed.insert(group.to_string(), Err(()));
        self.paint_group(group);
    }

    fn paint_listed_groups(&mut self) {
        let groups: Vec<String> = self.listed.keys().cloned().collect();
        for group in groups {
            self.paint_group(&group);
        }
    }

    fn paint_group(&mut self, group: &str) {
        match self.listed.get(group) {
            Some(Err(())) => {
                self.group_load.insert(group.to_string(), GroupLoad::Failed);
                self.enabled.retain(|(g, _), _| g != group);
            }
            Some(Ok(on)) if self.loaded => {
                let on = on.clone();
                let ids: Vec<String> = self.servers.iter().map(|s| s.id.clone()).collect();
                for id in ids {
                    self.enabled
                        .insert((group.to_string(), id.clone()), on.contains(&id));
                }
                self.group_load.insert(group.to_string(), GroupLoad::Ready);
            }
            _ => {}
        }
    }

    pub fn open_add(&mut self) {
        self.form = Some(Form::new_add(Transport::Stdio));
        self.confirm_delete = None;
        self.form_note = None;
    }

    pub fn close_form(&mut self) {
        self.form = None;
        self.form_note = None;
    }

    pub fn start_edit(&mut self, id: &str) {
        if let Some(s) = self.servers.iter().find(|s| s.id == id) {
            self.form = Some(Form::from_server(s));
            self.confirm_delete = None;
            self.form_note = None;
            self.open = Some(id.to_string());
        }
    }

    pub fn form_mut(&mut self) -> Option<&mut Form> {
        self.form.as_mut()
    }

    pub fn add_secret(&mut self) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let placement = form.transport.default_placement();
        form.secrets.push(SecretDraft {
            name: String::new(),
            placement,
            key: MaskedKey::default(),
        });
    }

    pub fn save_to_send(&mut self) -> Option<Save> {
        if self.busy {
            return None;
        }
        let form = self.form.as_ref()?;
        if let Some(problem) = form.problem() {
            self.form_note = Some(problem.into());
            return None;
        }
        let save = form.into_save()?;
        self.form_note = None;
        self.busy = true;
        Some(save)
    }

    pub fn saved(&mut self, server: McpServer) {
        self.busy = false;
        let id = server.id.clone();
        if let Some(slot) = self.servers.iter_mut().find(|s| s.id == id) {
            *slot = server;
        } else {
            self.servers.insert(0, server);
        }
        self.servers.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.form = None;
        self.form_note = None;
        self.entry_note = None;
        self.command_prompt.remove(&id);
        self.tools.remove(&id);
        self.open = Some(id.clone());
        self.tests.insert(id, TestState::Running);
        self.paint_listed_groups();
    }

    pub fn save_failed(&mut self, reason: &str) {
        self.busy = false;
        self.form_note = Some(failed(reason));
    }

    pub fn tested(&mut self, id: &str, result: &McpTestResult) {
        if let Some(n) = result.tool_count {
            if let Some(s) = self.servers.iter_mut().find(|s| s.id == id) {
                s.tool_count = Some(n);
            }
        }
        self.tests
            .insert(id.to_string(), TestState::from_result(result));
    }

    pub fn test_failed(&mut self, id: &str, reason: &str) {
        self.tests
            .insert(id.to_string(), TestState::Failed(reason.to_string()));
    }

    pub fn open_server(&mut self, id: &str) -> bool {
        if self.open.as_deref() == Some(id) {
            self.open = None;
            return false;
        }
        self.open = Some(id.to_string());
        !self.tools.contains_key(id)
    }

    pub fn tools_loaded(&mut self, id: &str, tools: Vec<McpTool>) {
        if let Some(s) = self.servers.iter_mut().find(|s| s.id == id) {
            s.tool_count = Some(tools.len() as u32);
        }
        self.tools.insert(id.to_string(), Ok(tools));
    }

    pub fn tools_failed(&mut self, id: &str, reason: &str) {
        self.tools.insert(id.to_string(), Err(reason.to_string()));
    }

    pub fn ask_delete(&mut self, id: &str) {
        self.confirm_delete = Some(id.to_string());
        self.form_note = None;
    }

    pub fn cancel_delete(&mut self) {
        self.confirm_delete = None;
    }

    pub fn delete_to_send(&mut self) -> Option<String> {
        if self.busy {
            return None;
        }
        let id = self.confirm_delete.take()?;
        self.busy = true;
        Some(id)
    }

    pub fn deleted(&mut self, id: &str) {
        self.busy = false;
        self.servers.retain(|s| s.id != id);
        self.tools.remove(id);
        self.tests.remove(id);
        self.command_prompt.remove(id);
        self.enabled.retain(|(_, s), _| s != id);
        for set in self.listed.values_mut().flatten() {
            set.remove(id);
        }
        if self.open.as_deref() == Some(id) {
            self.open = None;
        }
        if self
            .form
            .as_ref()
            .is_some_and(|f| f.id.as_deref() == Some(id))
        {
            self.form = None;
        }
        self.confirm_delete = None;
    }

    pub fn delete_failed(&mut self, id: &str, reason: &str) {
        self.busy = false;
        self.entry_note = Some((id.to_string(), failed(reason)));
    }

    pub fn set_enabled(&mut self, id: &str, on: bool) {
        if let Some(s) = self.servers.iter_mut().find(|s| s.id == id) {
            s.enabled = on;
        }
        self.busy = false;
    }

    pub fn toggle_enabled_to_send(&mut self, id: &str) -> Option<bool> {
        if self.busy {
            return None;
        }
        let on = self.servers.iter().find(|s| s.id == id)?.enabled;
        self.busy = true;
        self.entry_note = None;
        Some(!on)
    }

    pub fn toggle_group_to_send(&mut self, id: &str, group: &str) -> Option<bool> {
        if self.busy {
            return None;
        }
        if self.group_load_state(group) != GroupLoad::Ready {
            return None;
        }
        let on = self.enable_state(group, id)?;
        self.busy = true;
        self.entry_note = None;
        Some(!on)
    }

    pub fn group_toggled(&mut self, id: &str, group: &str, on: bool) {
        self.busy = false;
        self.enabled.insert((group.to_string(), id.to_string()), on);
        if let Some(Ok(set)) = self.listed.get_mut(group) {
            if on {
                set.insert(id.to_string());
            } else {
                set.remove(id);
            }
        }
    }

    pub fn entry_failed(&mut self, id: &str, reason: &str) {
        self.busy = false;
        self.entry_note = Some((id.to_string(), failed(reason)));
    }

    pub fn note_for(&self, id: &str) -> Option<&str> {
        self.entry_note
            .as_ref()
            .filter(|(e, _)| e == id)
            .map(|(_, n)| n.as_str())
    }

    pub fn test_for(&self, id: &str) -> Option<&TestState> {
        self.tests.get(id)
    }

    pub fn command_prompt_for(&self, id: &str) -> Option<&CommandPrompt> {
        self.command_prompt.get(id)
    }

    /// `/test` or `/tools` returned 409 `command_not_approved`.
    pub fn show_command_prompt(&mut self, id: &str, command: String, args: Vec<String>) {
        self.tests.remove(id);
        self.tools.remove(id);
        self.open = Some(id.to_string());
        self.command_prompt
            .insert(id.to_string(), CommandPrompt { command, args });
    }

    pub fn cancel_command_prompt(&mut self, id: &str) {
        self.command_prompt.remove(id);
    }

    /// Host approved the current command. Caller then re-runs the test.
    pub fn command_approved(&mut self, server: McpServer) {
        let id = server.id.clone();
        self.command_prompt.remove(&id);
        if let Some(slot) = self.servers.iter_mut().find(|s| s.id == id) {
            *slot = server;
        }
        self.busy = false;
        self.tests.insert(id, TestState::Running);
    }

    pub fn approve_command_to_send(&mut self, id: &str) -> Option<String> {
        if self.busy {
            return None;
        }
        self.command_prompt.get(id)?;
        self.busy = true;
        Some(id.to_string())
    }

    pub fn approve_command_failed(&mut self, id: &str, reason: &str) {
        self.busy = false;
        self.entry_note = Some((id.to_string(), failed(reason)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::McpSecretPublic;

    fn server(id: &str, name: &str, enabled: bool, tools: Option<u32>) -> McpServer {
        McpServer {
            id: id.into(),
            name: name.into(),
            transport: "stdio".into(),
            command: Some("/bin/true".into()),
            args: Some(vec![]),
            url: None,
            secrets: vec![McpSecretPublic {
                name: "TOKEN".into(),
                placement: McpSecretIn::Env,
                has_key: true,
                key_last4: Some("WXYZ".into()),
            }],
            enabled,
            tool_count: tools,
            created_at: "2026-11-21T01:00:00.000Z".into(),
            updated_at: "2026-11-21T01:00:00.000Z".into(),
            command_approved: Some(true),
            command_approved_at: Some("2026-11-21T01:00:00.000Z".into()),
        }
    }

    fn unapproved(id: &str, name: &str) -> McpServer {
        let mut s = server(id, name, true, None);
        s.command = Some("echo".into());
        s.args = Some(vec!["hello".into(), "world".into()]);
        s.command_approved = Some(false);
        s.command_approved_at = None;
        s
    }

    fn book() -> Book {
        let mut b = Book {
            chats: vec![
                ("g-team".into(), "作战组".into()),
                ("g-ada".into(), "Ada".into()),
            ],
            ..Book::default()
        };
        b.load(vec![
            server("s1", "文档", true, Some(2)),
            server("s2", "闲置", false, None),
        ]);
        b
    }

    #[test]
    fn empty_book_has_no_servers() {
        let mut b = Book::default();
        b.load(vec![]);
        assert!(b.is_empty() && b.loaded);
    }

    #[test]
    fn add_needs_name_and_command_or_url() {
        let mut b = book();
        b.open_add();
        assert_eq!(b.save_to_send(), None);
        assert_eq!(b.form_note.as_deref(), Some(EMPTY_NAME));
        let f = b.form.as_mut().unwrap();
        f.name = "  文档  ".into();
        assert_eq!(f.problem(), Some(NEED_COMMAND));
        f.command = "/usr/bin/docs".into();
        f.args = "  --ok  --quiet ".into();
        let save = f.into_save().unwrap();
        assert_eq!(save.name, "文档");
        assert_eq!(save.command.as_deref(), Some("/usr/bin/docs"));
        assert_eq!(
            save.args.as_deref(),
            Some(&["--ok".into(), "--quiet".into()][..])
        );
        f.transport = Transport::Http;
        assert_eq!(f.problem(), Some(NEED_URL));
        f.url = "not-a-url".into();
        assert_eq!(f.problem(), Some(BAD_URL));
        f.url = "http://127.0.0.1:9".into();
        assert!(f.into_save().is_some());
    }

    #[test]
    fn secret_is_masked_and_only_sent_when_changed() {
        let mut b = book();
        b.start_edit("s1");
        let f = b.form.as_mut().unwrap();
        assert!(f.secrets[0].key.shows_stored());
        assert_eq!(
            crate::cards::secret::filled(f.secrets[0].key.last4.as_deref()),
            "已填 · WXYZ"
        );
        let save = f.into_save().unwrap();
        assert_eq!(save.secrets[0].key, None, "unchanged key is omitted");
        f.secrets[0].key.clear();
        let save = f.into_save().unwrap();
        assert_eq!(save.secrets[0].key, Some(None));
    }

    #[test]
    fn new_secret_requires_a_value() {
        let mut f = Form::new_add(Transport::Stdio);
        f.name = "x".into();
        f.command = "/bin/true".into();
        f.secrets.push(SecretDraft {
            name: "TOKEN".into(),
            placement: McpSecretIn::Env,
            key: MaskedKey::default(),
        });
        assert_eq!(f.problem(), Some(NEED_SECRET_KEY));
        f.secrets[0].key.draft = "abc".into();
        assert!(f.problem().is_none());
    }

    #[test]
    fn delete_needs_a_second_click() {
        let mut b = book();
        assert_eq!(b.delete_to_send(), None);
        b.ask_delete("s1");
        b.cancel_delete();
        assert_eq!(b.delete_to_send(), None);
        b.ask_delete("s1");
        assert_eq!(b.delete_to_send().as_deref(), Some("s1"));
        b.deleted("s1");
        assert!(b.servers.iter().all(|s| s.id != "s1"));
    }

    #[test]
    fn save_then_test_shows_ok_or_fail() {
        let mut b = book();
        b.open_add();
        let f = b.form.as_mut().unwrap();
        f.name = "新".into();
        f.command = "/bin/true".into();
        let save = b.save_to_send().unwrap();
        assert!(b.busy);
        assert_eq!(save.name, "新");
        let mut row = server("s3", "新", true, None);
        row.secrets.clear();
        b.saved(row);
        assert_eq!(b.tests.get("s3"), Some(&TestState::Running));
        assert_eq!(b.open.as_deref(), Some("s3"));
        b.tested(
            "s3",
            &McpTestResult {
                ok: true,
                error: None,
                tool_count: Some(2),
            },
        );
        assert_eq!(b.test_for("s3").unwrap().text(), "能用了 · 2 个工具");
        assert_eq!(
            b.servers.iter().find(|s| s.id == "s3").unwrap().tool_count,
            Some(2)
        );
        b.tested(
            "s3",
            &McpTestResult {
                ok: false,
                error: Some("没有这个命令".into()),
                tool_count: None,
            },
        );
        assert_eq!(b.test_for("s3").unwrap().text(), "连不上：没有这个命令");
    }

    #[test]
    fn tools_mark_which_need_approval() {
        let mut b = book();
        assert!(b.open_server("s1"));
        b.tools_loaded(
            "s1",
            vec![
                McpTool {
                    name: "ping".into(),
                    description: Some("Check the server is up".into()),
                    needs_approval: false,
                },
                McpTool {
                    name: "write_note".into(),
                    description: None,
                    needs_approval: true,
                },
            ],
        );
        let tools = b.tools.get("s1").unwrap().as_ref().unwrap();
        assert!(!tools[0].needs_approval);
        assert!(tools[1].needs_approval);
        assert_eq!(NEEDS_APPROVAL, "要你批准");
        assert_eq!(
            b.servers.iter().find(|s| s.id == "s1").unwrap().tool_count,
            Some(2)
        );
        assert!(!b.open_server("s1"), "second click closes");
    }

    #[test]
    fn group_list_sets_on_and_off() {
        let mut b = book();
        let groups = b.begin_enable_loads();
        assert_eq!(groups, vec!["g-team", "g-ada"]);
        assert_eq!(b.toggle_group_to_send("s1", "g-team"), None);
        b.apply_group_mcp("g-team", &[server("s1", "文档", true, Some(2))]);
        assert_eq!(b.group_load_state("g-team"), GroupLoad::Ready);
        assert_eq!(b.enable_state("g-team", "s1"), Some(true));
        assert_eq!(b.enable_state("g-team", "s2"), Some(false));
        assert_eq!(b.toggle_group_to_send("s1", "g-team"), Some(false));
        b.group_toggled("s1", "g-team", false);
        assert_eq!(b.enable_state("g-team", "s1"), Some(false));
        b.group_mcp_failed("g-ada");
        assert_eq!(b.group_load_state("g-ada"), GroupLoad::Failed);
        assert_eq!(b.toggle_group_to_send("s1", "g-ada"), None);
    }

    #[test]
    fn server_enable_switch_flips_from_known_state() {
        let mut b = book();
        assert_eq!(b.toggle_enabled_to_send("s1"), Some(false));
        assert!(b.busy);
        b.set_enabled("s1", false);
        assert!(!b.servers.iter().find(|s| s.id == "s1").unwrap().enabled);
        assert_eq!(tool_count_text(None), "还没数过");
        assert_eq!(tool_count_text(Some(2)), "2 个工具");
    }

    #[test]
    fn unapproved_stdio_shows_prompt_until_approved_or_cancelled() {
        let mut b = Book::default();
        b.load(vec![unapproved("s3", "回声")]);
        assert!(command_unapproved(
            b.servers.iter().find(|s| s.id == "s3").unwrap()
        ));
        assert_eq!(COMMAND_UNAPPROVED, "命令未批准");
        assert_eq!(TOOLS_NEED_APPROVAL, "批准命令后才能读取工具");
        assert_eq!(APPROVE_RUN, "批准运行");

        b.show_command_prompt("s3", "echo".into(), vec!["hello".into(), "world".into()]);
        let prompt = b.command_prompt_for("s3").unwrap();
        assert_eq!(prompt.command, "echo");
        assert_eq!(prompt.args, ["hello", "world"]);
        assert_eq!(b.open.as_deref(), Some("s3"));
        assert!(b.test_for("s3").is_none());

        b.cancel_command_prompt("s3");
        assert!(b.command_prompt_for("s3").is_none());
        assert!(command_unapproved(
            b.servers.iter().find(|s| s.id == "s3").unwrap()
        ));

        b.show_command_prompt("s3", "echo".into(), vec!["hello".into()]);
        assert_eq!(b.approve_command_to_send("s3").as_deref(), Some("s3"));
        assert!(b.busy);
        let mut approved = unapproved("s3", "回声");
        approved.command_approved = Some(true);
        approved.command_approved_at = Some("2026-11-22T00:00:00.000Z".into());
        b.command_approved(approved);
        assert!(b.command_prompt_for("s3").is_none());
        assert!(!command_unapproved(
            b.servers.iter().find(|s| s.id == "s3").unwrap()
        ));
        assert!(matches!(b.test_for("s3"), Some(TestState::Running)));
        assert!(!b.busy);
    }

    #[test]
    fn saved_stdio_server_is_unapproved_until_host_says_otherwise() {
        let mut b = Book::default();
        b.saved(unapproved("s4", "新的"));
        assert!(command_unapproved(
            b.servers.iter().find(|s| s.id == "s4").unwrap()
        ));
        assert_eq!(b.open.as_deref(), Some("s4"));
        assert!(matches!(b.test_for("s4"), Some(TestState::Running)));
    }

    #[test]
    fn unapproved_row_subtitle_explains_why_there_is_no_tool_count() {
        let s = unapproved("s5", "回声");
        assert_eq!(row_subtitle(&s), TOOLS_NEED_APPROVAL);
        let mut ready = unapproved("s6", "夹具");
        ready.command_approved = Some(true);
        ready.tool_count = Some(2);
        assert_eq!(row_subtitle(&ready), "2 个工具");
        assert_eq!(LABEL_NAME, "名称");
        assert_eq!(LABEL_ARGS, "参数（用空格分开）");
        assert_eq!(LABEL_URL, "地址");
        assert_eq!(LABEL_PROMPT_ARGS, "参数");
    }
}
