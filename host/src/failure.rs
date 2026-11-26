//! Typed task failures (#181).
//!
//! Every place that ends a task without its work builds a [`Failure`]: the
//! legacy Chinese `failure` text, unchanged for older clients, and the typed
//! [`FailureData`] the host stores in the `failure_data` column. Events read
//! `failureCode` and `failureDetail` from the typed column; only rows written
//! before it existed are read from their text, once, by [`backfill`].

use robroz_contract::host_text as ht;
use robroz_contract::{FailureCode, FailureData, FailureParams};
use rusqlite::{params, Connection};

/// A failure as it is written to a task (and copied to its delivery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The legacy `failure` text.
    pub text: String,
    /// What goes into `failure_data`.
    pub data: FailureData,
}

impl Failure {
    fn new(text: impl Into<String>, code: FailureCode, params: FailureParams) -> Self {
        Self {
            text: text.into(),
            data: FailureData::with(code, params),
        }
    }

    /// The `failure_data` column value.
    pub fn json(&self) -> String {
        serde_json::to_string(&self.data).expect("FailureData serializes")
    }

    /// 「没写出结果」: a dead run, a cancelled step or a split task that ended.
    pub fn no_result() -> Self {
        Self::new(
            robroz_contract::legacy::NO_RESULT,
            FailureCode::NoResult,
            FailureParams::default(),
        )
    }

    pub fn approval_expired() -> Self {
        Self::new(
            robroz_contract::APPROVAL_EXPIRED_REASON,
            FailureCode::ApprovalExpired,
            FailureParams::default(),
        )
    }

    pub fn round_limit() -> Self {
        Self::new(
            crate::tools::ROUND_LIMIT_FAILURE,
            FailureCode::ToolRoundLimit,
            FailureParams::default(),
        )
    }

    /// A denied task gate. `shown` is the card's line (with the host's
    /// fallback); `line` is the person's own line, when the task has one.
    pub fn denied_line(shown: &str, line: Option<&str>) -> Self {
        Self::new(
            format!("{}{shown}", ht::DENIED_PREFIX),
            FailureCode::Denied,
            FailureParams {
                line: line.map(str::to_string),
                ..Default::default()
            },
        )
    }

    /// A denied MCP call on `server.tool`.
    pub fn denied_tool(server: &str, tool: &str) -> Self {
        let name = format!("{server}.{tool}");
        Self::new(
            format!("{}{name}", ht::DENIED_PREFIX),
            FailureCode::Denied,
            FailureParams {
                tool: Some(name),
                ..Default::default()
            },
        )
    }

    /// A denied MCP card that names no server and tool (old data): the
    /// legacy text quotes the card summary, there is no raw detail.
    pub fn denied_card(summary: &str) -> Self {
        Self::new(
            format!("{}{summary}", ht::DENIED_PREFIX),
            FailureCode::Denied,
            FailureParams::default(),
        )
    }

    /// The model call failed: the status code, or the raw error and reason.
    pub fn model(err: &robroz_model::ModelError) -> Self {
        Self::new(
            format!("{}{err}", ht::MODEL_NO_REPLY_PREFIX),
            FailureCode::ModelNoReply,
            FailureParams {
                status: err.http_status(),
                reason: err.reason().map(str::to_string),
                error: err.raw_error().map(str::to_string),
                ..Default::default()
            },
        )
    }

    pub fn over_quota(over: &robroz_policy::OverQuota) -> Self {
        use robroz_policy::OverQuota;
        let (code, params) = match over {
            OverQuota::TooMany { used, limit } => (
                FailureCode::OutboundOverQuota,
                FailureParams {
                    used: Some(*used),
                    limit: Some(*limit),
                    ..Default::default()
                },
            ),
            OverQuota::NotGet(method) => (
                FailureCode::OutboundNotGet,
                FailureParams {
                    method: Some(method.clone()),
                    ..Default::default()
                },
            ),
        };
        Self::new(over.failure(), code, params)
    }

    /// Over quota without the counts (the reply said so but kept no text).
    pub fn over_quota_unknown() -> Self {
        Self::new(
            ht::OUTBOUND_OVER_QUOTA,
            FailureCode::OutboundOverQuota,
            FailureParams::default(),
        )
    }

    /// The tool loop stopped on a Takeover call (`tool` is its tool name).
    pub fn waiting_approval(display_name: &str, tool: Option<&str>) -> Self {
        Self::new(
            format!("{}{display_name}", ht::WAITING_APPROVAL_PREFIX),
            FailureCode::WaitingApproval,
            FailureParams {
                tool: tool.map(str::to_string),
                ..Default::default()
            },
        )
    }

    pub fn save_approval_state(err: &crate::mcp_tools::PendingError) -> Self {
        Self::pending(
            ht::SAVE_APPROVAL_STATE_FAILED_PREFIX,
            FailureCode::SaveApprovalStateFailed,
            err,
        )
    }

    pub fn resume_approved_call(err: &crate::mcp_tools::PendingError) -> Self {
        Self::pending(
            ht::RESUME_APPROVED_CALL_FAILED_PREFIX,
            FailureCode::ResumeApprovedCallFailed,
            err,
        )
    }

    fn pending(prefix: &str, code: FailureCode, err: &crate::mcp_tools::PendingError) -> Self {
        Self::new(
            format!("{prefix}{err}"),
            code,
            FailureParams {
                reason: Some(err.reason.to_string()),
                error: err.error.clone(),
                ..Default::default()
            },
        )
    }
}

/// `failureCode` and `failureDetail` for a row: from `failure_data` when it
/// is there, else (a row from before the column) from the legacy text.
pub fn codes(failure: Option<&str>, data: Option<&str>) -> (Option<FailureCode>, Option<String>) {
    let typed = data.and_then(|d| serde_json::from_str::<FailureData>(d).ok());
    match (typed, failure) {
        (Some(data), _) => (Some(data.code), data.detail()),
        (None, Some(text)) => {
            let data = FailureData::from_legacy(text);
            (Some(data.code), data.detail())
        }
        (None, None) => (None, None),
    }
}

/// Add `failure_data` to tasks and deliveries and fill it for rows that only
/// have the legacy text. Rows that already have it are never touched, so
/// this reads each legacy row once.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    for table in ["tasks", "deliveries"] {
        let _ = conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN failure_data TEXT"),
            [],
        );
    }
    backfill(conn).map(|_| ())
}

/// Fill `failure_data` from `failure` for rows that have only the text.
pub(crate) fn backfill(conn: &Connection) -> rusqlite::Result<usize> {
    let mut filled = 0;
    for table in ["tasks", "deliveries"] {
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT rowid, failure FROM {table} WHERE failure IS NOT NULL AND failure_data IS NULL"
            ))?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (rowid, text) in rows {
            let data = serde_json::to_string(&FailureData::from_legacy(&text))
                .expect("FailureData serializes");
            filled += conn.execute(
                &format!("UPDATE {table} SET failure_data = ?1 WHERE rowid = ?2"),
                params![data, rowid],
            )?;
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests;
