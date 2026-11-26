//! Stable machine `code` for every host error body. The host text in `error`
//! (and `message`) is for logs only: a client renders its own copy from
//! `code` (and `reason` on the approval 403) through its i18n.
//!
//! [`code_for_message`] maps each fixed host error text to its code. An error
//! text with runtime content (a validator message that embeds a value, an
//! upstream error) has no entry: [`fallback_code`] gives the code for its
//! HTTP status instead, so a 4xx/5xx body always has a `code`.
//!
//! Codes are the lower-case name of the `host_text` constant, for example
//! `group_not_found` for [`host_text::GROUP_NOT_FOUND`]. Existing codes never
//! change meaning; new ones may be added.

use crate::host_text;

/// Fixed host error text and its `code`.
pub const ERROR_CODES: &[(&str, &str)] = &[
    (host_text::GROUP_NOT_FOUND, "group_not_found"),
    (host_text::BOT_NOT_IN_GROUP, "bot_not_in_group"),
    (host_text::NEED_TEXT, "need_text"),
    (host_text::NEED_NAME, "need_name"),
    (host_text::NEED_TITLE, "need_title"),
    (host_text::NEED_INSTRUCTION, "need_instruction"),
    (host_text::NEED_URL, "need_url"),
    (host_text::NEED_MODEL_NAME, "need_model_name"),
    (host_text::NEED_COMMAND, "need_command"),
    (host_text::NEED_SECRET_NAME, "need_secret_name"),
    (host_text::NEED_SECRET_VALUE, "need_secret_value"),
    (host_text::NEED_RULE, "need_rule"),
    (host_text::NEED_ACTION, "need_action"),
    (host_text::NEED_SEARXNG_URL, "need_searxng_url"),
    (host_text::BAD_REQUEST_FORMAT, "bad_request_format"),
    (host_text::BODY_NOT_JSON, "body_not_json"),
    (host_text::BODY_INVALID, "body_invalid"),
    (host_text::NOTHING_TO_CHANGE, "nothing_to_change"),
    (host_text::DATABASE_ERROR, "database_error"),
    (host_text::ALREADY_ANSWERED, "already_answered"),
    (host_text::SINCE_INVALID, "since_invalid"),
    (host_text::READ_AT_INVALID, "read_at_invalid"),
    (host_text::URL_NEEDS_HTTP, "url_needs_http"),
    (host_text::KEY_EMPTY, "key_empty"),
    (host_text::KEY_HAS_WHITESPACE, "key_has_whitespace"),
    (host_text::ARGS_INVALID, "args_invalid"),
    (host_text::BAD_SCOPE, "bad_scope"),
    (host_text::BAD_STATUS, "bad_status"),
    (host_text::BAD_MEMORY_KIND, "bad_memory_kind"),
    (host_text::BAD_MESSAGE_KIND, "bad_message_kind"),
    (host_text::BAD_SLUG, "bad_slug"),
    (host_text::BAD_CRON, "bad_cron"),
    (host_text::BAD_TRANSPORT, "bad_transport"),
    (host_text::BAD_SEARCH_PROVIDER, "bad_search_provider"),
    (host_text::TOKEN_REJECTED, "token_rejected"),
    (host_text::NAME_TOO_LONG, "name_too_long"),
    (host_text::NAME_MAX_80, "name_max_80"),
    (host_text::NAME_ONE_LINE, "name_one_line"),
    (host_text::NAME_TAKEN, "name_taken"),
    (host_text::TITLE_TOO_LONG, "title_too_long"),
    (host_text::INSTRUCTION_TOO_LONG, "instruction_too_long"),
    (host_text::MEMORY_TOO_LONG, "memory_too_long"),
    (host_text::SUMMARY_ONE_LINE, "summary_one_line"),
    (host_text::RULE_ONE_LINE, "rule_one_line"),
    (host_text::COMMAND_INVALID, "command_invalid"),
    (host_text::SECRET_NAME_MAX, "secret_name_max"),
    (host_text::SECRET_NAME_WHITESPACE, "secret_name_whitespace"),
    (host_text::SLUG_TAKEN, "slug_taken"),
    (host_text::BRO_NOT_FOUND, "bro_not_found"),
    (host_text::UNKNOWN_BRO, "unknown_bro"),
    (host_text::TASK_NOT_FOUND, "task_not_found"),
    (host_text::APPROVAL_NOT_FOUND, "approval_not_found"),
    (host_text::APPROVAL_VANISHED, "approval_vanished"),
    (host_text::CONFIRM_CARD_NOT_FOUND, "confirm_card_not_found"),
    (host_text::CHOICE_CARD_NOT_FOUND, "choice_card_not_found"),
    (host_text::MEMORY_NOT_FOUND, "memory_not_found"),
    (host_text::SKILL_NOT_FOUND, "skill_not_found"),
    (host_text::RULE_NOT_FOUND, "rule_not_found"),
    (host_text::ROUTINE_NOT_FOUND, "routine_not_found"),
    (host_text::MCP_SERVER_NOT_FOUND, "mcp_server_not_found"),
    (
        host_text::MCP_NOT_ENABLED_IN_GROUP,
        "mcp_not_enabled_in_group",
    ),
    (host_text::MCP_STDIO_ONLY_APPROVE, "mcp_stdio_only_approve"),
    (host_text::DM_ONE_BRO, "dm_one_bro"),
    (host_text::ACTION_NOT_SUPPORTED, "action_not_supported"),
    (host_text::ACTION_HOST_ONLY, "action_host_only"),
    (
        host_text::APPROVE_CONTINUE_MARKER_FAILED,
        "approve_continue_marker_failed",
    ),
    (
        host_text::APPROVE_TAKEOVER_MARKER_FAILED,
        "approve_takeover_marker_failed",
    ),
    (host_text::RESUME_NO_INTENT, "resume_no_intent"),
    (host_text::RESUME_MARKER_FAILED, "resume_marker_failed"),
    (host_text::RESUME_NEEDS_APPROVAL, "resume_needs_approval"),
    (host_text::CANT_RESUME, "cant_resume"),
    (host_text::CHOICE_OPTION_COUNT, "choice_option_count"),
    (host_text::CHOICE_ONE_RECOMMENDED, "choice_one_recommended"),
    (host_text::CHOICE_OPTION_EMPTY, "choice_option_empty"),
    (host_text::CHOICE_DUPLICATE, "choice_duplicate"),
    (
        host_text::CHOICE_PICK_AT_LEAST_ONE,
        "choice_pick_at_least_one",
    ),
    (host_text::CHOICE_PICK_ONLY_ONE, "choice_pick_only_one"),
    (host_text::CHOICE_NO_SUCH_OPTION, "choice_no_such_option"),
    (host_text::CHOICE_ALREADY_PICKED, "choice_already_picked"),
    (crate::NOT_BRO_OWNER, "not_bro_owner"),
    (crate::MCP_COMMAND_NOT_APPROVED, "command_not_approved"),
];

/// The code for a fixed host error text, if it has one.
pub fn code_for_message(message: &str) -> Option<&'static str> {
    ERROR_CODES
        .iter()
        .find(|(text, _)| *text == message)
        .map(|(_, code)| *code)
}

/// The code for an error with no entry in [`ERROR_CODES`], by HTTP status.
pub fn fallback_code(status: u16) -> &'static str {
    match status {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        409 => "conflict",
        413 => "payload_too_large",
        422 => "unprocessable",
        502 => "bad_gateway",
        s if s >= 500 => "internal_error",
        _ => "bad_request",
    }
}

/// Codes for a request the host rejects before any handler runs: a body
/// that is not JSON or does not fit the endpoint, a wrong `Content-Type`, or
/// a path or query that does not parse (#179). The HTTP status is axum's:
/// 400 for syntax, path and query, 415 for the content type, 413 for a body
/// over the limit, 422 for a body that is JSON but has the wrong shape.
pub mod rejection {
    /// 400: the body is not valid JSON (syntax error or cut short).
    pub const INVALID_JSON: &str = "invalid_json";
    /// 422: a required field is missing.
    pub const MISSING_FIELD: &str = "missing_field";
    /// 422: a field holds a value that is not one of the allowed names.
    pub const INVALID_ENUM: &str = "invalid_enum";
    /// 422: a field the endpoint does not accept.
    pub const UNKNOWN_FIELD: &str = "unknown_field";
    /// 422: a field has the wrong type or an invalid value.
    pub const INVALID_FIELD: &str = "invalid_field";
    /// 415: the request is not `Content-Type: application/json`.
    pub const UNSUPPORTED_MEDIA_TYPE: &str = "unsupported_media_type";
    /// 413: the body is over the size limit.
    pub const BODY_TOO_LARGE: &str = "body_too_large";
    /// 400: the body could not be read.
    pub const INVALID_BODY: &str = "invalid_body";
    /// 400: a path segment does not parse (for example a bad id).
    pub const INVALID_PATH: &str = "invalid_path";
    /// 400: the query string does not parse.
    pub const INVALID_QUERY: &str = "invalid_query";

    /// Every rejection code.
    pub const ALL: [&str; 10] = [
        INVALID_JSON,
        MISSING_FIELD,
        INVALID_ENUM,
        UNKNOWN_FIELD,
        INVALID_FIELD,
        UNSUPPORTED_MEDIA_TYPE,
        BODY_TOO_LARGE,
        INVALID_BODY,
        INVALID_PATH,
        INVALID_QUERY,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_code_is_snake_case_and_unique() {
        let mut texts = HashSet::new();
        let mut codes = HashSet::new();
        for (text, code) in ERROR_CODES {
            assert!(
                !code.is_empty()
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{code}"
            );
            assert!(texts.insert(*text), "two codes for one text: {text}");
            assert!(codes.insert(*code), "two texts for one code: {code}");
        }
    }

    #[test]
    fn rejection_codes_are_snake_case_unique_and_not_host_text_codes() {
        let mut seen = HashSet::new();
        for code in rejection::ALL {
            assert!(
                code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{code}"
            );
            assert!(seen.insert(code), "{code}");
            assert!(
                ERROR_CODES.iter().all(|(_, c)| *c != code),
                "{code} clashes with a host text code"
            );
        }
    }

    #[test]
    fn a_fixed_text_maps_to_its_code() {
        assert_eq!(
            code_for_message(host_text::GROUP_NOT_FOUND),
            Some("group_not_found")
        );
        assert_eq!(
            code_for_message(host_text::TOKEN_REJECTED),
            Some("token_rejected")
        );
        assert_eq!(
            code_for_message(crate::NOT_BRO_OWNER),
            Some("not_bro_owner")
        );
        assert_eq!(code_for_message("something dynamic: 42"), None);
    }

    #[test]
    fn every_fallback_code_is_snake_case() {
        for status in [400u16, 401, 403, 404, 409, 413, 422, 500, 502, 503, 418] {
            let code = fallback_code(status);
            assert!(
                code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{code}"
            );
        }
        assert_eq!(fallback_code(404), "not_found");
        assert_eq!(fallback_code(500), "internal_error");
    }

    #[test]
    fn a_code_is_the_lower_case_name_of_its_host_text_constant() {
        for (text, code) in ERROR_CODES {
            if let Some((name, _)) = host_text::HOST_UI_STRINGS.iter().find(|(_, t)| t == text) {
                assert_eq!(name.to_lowercase(), *code, "{text}");
            }
        }
        for name in [
            "GROUP_NOT_FOUND",
            "NEED_TEXT",
            "DATABASE_ERROR",
            "CANT_RESUME",
        ] {
            let (_, text) = host_text::HOST_UI_STRINGS
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("{name} not in HOST_UI_STRINGS"));
            assert!(code_for_message(text).is_some(), "{name}");
        }
    }
}
