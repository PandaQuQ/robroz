//! Status text, dot colour, labels and footnotes for the new cards, as pure
//! functions (SPEC §8: "客户端只画，不自己挑颜色"). The view never picks a
//! colour or a string for a state on its own.

use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone};

use super::{ActionKind, ApprovalState, ChoiceCard, ChoiceState, DraftChannel, DraftState};

/// The 8px dot at the top right of a new card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    /// Waiting on the person: 等你选 / 等你批准 / 草稿待确认.
    Waiting,
    /// 已选 / 已批准 / 已发送.
    Done,
    /// 已失效 / 已拒绝 / 已放弃.
    Void,
    /// 卡住: needs the owner's own sign-in.
    Stuck,
}

pub fn choice_status(state: &ChoiceState) -> (&'static str, Dot) {
    match state {
        ChoiceState::Open => ("等你选", Dot::Waiting),
        ChoiceState::Answered { .. } => ("已选", Dot::Done),
        ChoiceState::Expired => ("已失效", Dot::Void),
    }
}

pub fn approval_status(state: &ApprovalState) -> (&'static str, Dot) {
    match state {
        ApprovalState::Pending => ("等你批准", Dot::Waiting),
        ApprovalState::Approved { .. } => ("已批准", Dot::Done),
        ApprovalState::Denied { .. } => ("已拒绝", Dot::Void),
        ApprovalState::NeedsOwner => ("卡住", Dot::Stuck),
    }
}

pub fn draft_status(state: &DraftState) -> (&'static str, Dot) {
    match state {
        DraftState::Pending => ("草稿待确认", Dot::Waiting),
        DraftState::Sent { .. } => ("已发送", Dot::Done),
        DraftState::Discarded { .. } => ("已放弃", Dot::Void),
    }
}

pub fn action_kind_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Delete => "删除",
        ActionKind::Send => "发送",
        ActionKind::Spend => "付款",
    }
}

pub fn channel_label(channel: DraftChannel) -> &'static str {
    match channel {
        DraftChannel::Email => "邮件",
        DraftChannel::Message => "消息",
    }
}

/// The only action a stuck card offers (SPEC §0).
pub const TAKE_OVER: &str = "去私聊接管";
pub const NEEDS_OWNER_LINE: &str = "这一步需要你本人登录，群里不处理";
pub const MULTI_HINT: &str = "可多选";
pub const RETRY_NOTE: &str = "没发出去，再点一次";
pub const PAUSE_FAILED_NOTE: &str = "没改成功";

pub fn confirm_label(picked: usize) -> String {
    format!("确认（{picked}）")
}

/// Local `HH:MM` for a Unix-seconds timestamp.
pub fn clock_in<Tz: TimeZone>(unix: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match tz.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// Local `HH:MM` for an ISO 8601 contract timestamp; empty if unparsable.
pub fn clock_iso_in<Tz: TimeZone>(iso: &str, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    DateTime::parse_from_rfc3339(iso)
        .map(|t| t.with_timezone(tz).format("%H:%M").to_string())
        .unwrap_or_default()
}

fn with_time(text: String, time: &str) -> String {
    if time.is_empty() {
        text
    } else {
        format!("{text} · {time}")
    }
}

/// Footnote under a choice card. `answered_here` is true when this client
/// sent the answer in this session; `tz` is the computer's local zone.
pub fn choice_footnote<Tz: TimeZone>(
    card: &ChoiceCard,
    answered_here: bool,
    tz: &Tz,
) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    match &card.state {
        ChoiceState::Open => None,
        ChoiceState::Expired => Some("已过期，没有人作答".into()),
        ChoiceState::Answered {
            selected,
            at,
            elsewhere,
        } => {
            let labels = selected
                .iter()
                .map(|id| card.label_of(id).unwrap_or(id).to_string())
                .collect::<Vec<_>>()
                .join("、");
            let time = at
                .as_deref()
                .map(|a| clock_iso_in(a, tz))
                .unwrap_or_default();
            Some(if *elsewhere {
                format!("已在私聊里回答：{labels}")
            } else if answered_here {
                with_time(format!("你选了「{labels}」"), &time)
            } else {
                with_time(format!("已选「{labels}」"), &time)
            })
        }
    }
}

pub fn approval_footnote<Tz: TimeZone>(state: &ApprovalState, tz: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    match state {
        ApprovalState::Approved { by, at } => {
            Some(with_time(format!("{by} 批准"), &clock_in(*at, tz)))
        }
        ApprovalState::Denied { by, at } => {
            Some(with_time(format!("{by} 拒绝"), &clock_in(*at, tz)))
        }
        ApprovalState::Pending | ApprovalState::NeedsOwner => None,
    }
}

pub fn draft_footnote<Tz: TimeZone>(state: &DraftState, tz: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    match state {
        DraftState::Sent { by, at } => {
            Some(with_time(format!("{by} 确认发送"), &clock_in(*at, tz)))
        }
        DraftState::Discarded { by, at } => {
            Some(with_time(format!("{by} 放弃"), &clock_in(*at, tz)))
        }
        DraftState::Pending => None,
    }
}

/// Preview lines for a draft: 3 while pending, 1 once settled (SPEC §6).
pub fn draft_preview_lines(state: &DraftState) -> usize {
    match state {
        DraftState::Pending => 3,
        _ => 1,
    }
}

/// Next-run text for the routines list (SPEC §7): 今天 23:00 / 明天 08:00 /
/// 周一 09:30 within a week, otherwise 11月30日 08:00. Both in local time.
pub fn format_next_run(next: NaiveDateTime, now: NaiveDateTime) -> String {
    let days = (next.date() - now.date()).num_days();
    let hm = next.format("%H:%M");
    match days {
        0 => format!("今天 {hm}"),
        1 => format!("明天 {hm}"),
        2..=6 => {
            const WEEK: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
            let w = WEEK[next.weekday().num_days_from_monday() as usize];
            format!("{w} {hm}")
        }
        _ => format!("{}月{}日 {hm}", next.month(), next.day()),
    }
}

/// Width units for a rough fit: CJK and full-width count 2, others 1.
fn units(c: char) -> usize {
    if c.is_ascii() {
        1
    } else {
        2
    }
}

/// Cut `text` to at most `max_lines` lines of `cols` width units each,
/// ending in "…" when anything was dropped. iced has no text ellipsis, so
/// the view estimates `cols` from the card width and font size.
pub fn clamp_lines(text: &str, cols: usize, max_lines: usize) -> String {
    let cols = cols.max(4);
    let budget = cols * max_lines.max(1);
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let total: usize = flat.chars().map(units).sum();
    if total <= budget {
        return flat;
    }
    let mut out = String::new();
    let mut used = 0;
    for c in flat.chars() {
        if used + units(c) > budget - 2 {
            break;
        }
        used += units(c);
        out.push(c);
    }
    out.push('…');
    out
}

/// Narrow screens show at most this many characters of a bot name.
pub const NARROW_NAME_CHARS: usize = 8;

/// A bot name for a narrow screen: longer than [`NARROW_NAME_CHARS`]
/// characters (CJK counts one each) becomes the first ones plus "…".
/// Returns `None` when the name fits as is.
pub fn short_bot_name(name: &str) -> Option<String> {
    if name.chars().count() <= NARROW_NAME_CHARS {
        return None;
    }
    let mut s: String = name.chars().take(NARROW_NAME_CHARS).collect();
    s.push('…');
    Some(s)
}

/// Shorten a path from the middle to at most `max` width units (ASCII 1,
/// CJK 2), keeping its start and its last segment (the file name, or the
/// last directory with its trailing `/`): `logs/2026-10/app/x.log` →
/// `logs/…/x.log`. Falls back to a character-level middle cut when even
/// that is too wide. Paths that fit come back unchanged.
pub fn middle_ellipsis(path: &str, max: usize) -> String {
    let width = |s: &str| s.chars().map(units).sum::<usize>();
    if width(path) <= max {
        return path.to_string();
    }
    let trailing = path.ends_with('/');
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if parts.len() >= 3 {
        let first = parts[0];
        let last = parts[parts.len() - 1];
        let candidate = format!("{first}/…/{last}{}", if trailing { "/" } else { "" });
        if width(&candidate) <= max {
            return candidate;
        }
    }
    // Keep as much of both ends as fits around one "…": the start gets
    // the larger half, the end the rest.
    let budget = max.saturating_sub(units('…')).max(2);
    let chars: Vec<char> = path.chars().collect();
    let mut head = String::new();
    let mut used = 0;
    let mut i = 0;
    while i < chars.len() && used + units(chars[i]) <= budget.div_ceil(2) {
        used += units(chars[i]);
        head.push(chars[i]);
        i += 1;
    }
    let mut tail = Vec::new();
    let mut j = chars.len();
    while j > i && used + units(chars[j - 1]) <= budget {
        used += units(chars[j - 1]);
        tail.push(chars[j - 1]);
        j -= 1;
    }
    tail.reverse();
    format!("{head}…{}", tail.into_iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::tests::opt;

    #[test]
    fn bot_names_cut_at_eight_chars() {
        assert_eq!(short_bot_name("文档哥"), None);
        assert_eq!(short_bot_name("最详尽开发文档哥"), None, "exactly 8");
        assert_eq!(
            short_bot_name("最详尽开发文档哥哥").as_deref(),
            Some("最详尽开发文档哥…")
        );
        assert_eq!(short_bot_name("GitHubBro!").as_deref(), Some("GitHubBr…"));
    }

    #[test]
    fn paths_keep_start_and_file_name() {
        assert_eq!(middle_ellipsis("logs/2026-10/", 40), "logs/2026-10/");
        assert_eq!(
            middle_ellipsis("logs/2026-10/app/server.log", 20),
            "logs/…/server.log"
        );
        assert_eq!(
            middle_ellipsis("workspace/cache/2026-10/", 22),
            "workspace/…/2026-10/"
        );
        let cut = middle_ellipsis("a_very_long_single_file_name.tar.gz", 16);
        assert!(cut.contains('…'));
        assert!(cut.starts_with("a_ve") && cut.ends_with(".gz"), "{cut}");
        assert!(cut.chars().map(units).sum::<usize>() <= 16);
    }
    use chrono::{NaiveDate, Utc};

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn status_text_and_dot_per_state() {
        assert_eq!(choice_status(&ChoiceState::Open), ("等你选", Dot::Waiting));
        assert_eq!(
            choice_status(&ChoiceState::Answered {
                selected: vec![],
                at: None,
                elsewhere: false
            }),
            ("已选", Dot::Done)
        );
        assert_eq!(choice_status(&ChoiceState::Expired), ("已失效", Dot::Void));
        assert_eq!(
            approval_status(&ApprovalState::Pending),
            ("等你批准", Dot::Waiting)
        );
        assert_eq!(
            approval_status(&ApprovalState::Approved {
                by: "P".into(),
                at: 0
            }),
            ("已批准", Dot::Done)
        );
        assert_eq!(
            approval_status(&ApprovalState::Denied {
                by: "P".into(),
                at: 0
            }),
            ("已拒绝", Dot::Void)
        );
        assert_eq!(
            approval_status(&ApprovalState::NeedsOwner),
            ("卡住", Dot::Stuck)
        );
        assert_eq!(
            draft_status(&DraftState::Pending),
            ("草稿待确认", Dot::Waiting)
        );
        assert_eq!(
            draft_status(&DraftState::Sent {
                by: "P".into(),
                at: 0
            }),
            ("已发送", Dot::Done)
        );
        assert_eq!(
            draft_status(&DraftState::Discarded {
                by: "P".into(),
                at: 0
            }),
            ("已放弃", Dot::Void)
        );
    }

    #[test]
    fn labels() {
        assert_eq!(action_kind_label(ActionKind::Delete), "删除");
        assert_eq!(action_kind_label(ActionKind::Send), "发送");
        assert_eq!(action_kind_label(ActionKind::Spend), "付款");
        assert_eq!(channel_label(DraftChannel::Email), "邮件");
        assert_eq!(channel_label(DraftChannel::Message), "消息");
        assert_eq!(confirm_label(2), "确认（2）");
    }

    #[test]
    fn footnotes() {
        let at = dt(2026, 11, 20, 1, 52).and_utc().timestamp();
        assert_eq!(
            approval_footnote(
                &ApprovalState::Approved {
                    by: "Panda".into(),
                    at
                },
                &Utc
            ),
            Some("Panda 批准 · 01:52".into())
        );
        assert_eq!(
            approval_footnote(
                &ApprovalState::Denied {
                    by: "Panda".into(),
                    at
                },
                &Utc
            ),
            Some("Panda 拒绝 · 01:52".into())
        );
        assert_eq!(approval_footnote(&ApprovalState::NeedsOwner, &Utc), None);
        assert_eq!(
            draft_footnote(
                &DraftState::Sent {
                    by: "Panda".into(),
                    at
                },
                &Utc
            ),
            Some("Panda 确认发送 · 01:52".into())
        );
        assert_eq!(
            draft_footnote(
                &DraftState::Discarded {
                    by: "Panda".into(),
                    at
                },
                &Utc
            ),
            Some("Panda 放弃 · 01:52".into())
        );
        let tz8 = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        assert_eq!(clock_in(at, &tz8), "09:52", "local zone, not UTC");
    }

    #[test]
    fn choice_footnotes() {
        let mut card = ChoiceCard {
            id: "c".into(),
            bot: "文档哥".into(),
            prompt: "?".into(),
            options: vec![
                opt("a", "两种都写", false),
                opt("b", "cargo install", false),
            ],
            multi: false,
            state: ChoiceState::Open,
        };
        assert_eq!(choice_footnote(&card, false, &Utc), None);
        card.state = ChoiceState::Answered {
            selected: vec!["a".into()],
            at: Some("2026-11-20T01:52:00.000Z".into()),
            elsewhere: false,
        };
        assert_eq!(
            choice_footnote(&card, true, &Utc).unwrap(),
            "你选了「两种都写」 · 01:52"
        );
        assert_eq!(
            choice_footnote(&card, false, &Utc).unwrap(),
            "已选「两种都写」 · 01:52"
        );
        card.state = ChoiceState::Answered {
            selected: vec!["b".into()],
            at: None,
            elsewhere: true,
        };
        assert_eq!(
            choice_footnote(&card, false, &Utc).unwrap(),
            "已在私聊里回答：cargo install"
        );
        card.state = ChoiceState::Expired;
        assert_eq!(
            choice_footnote(&card, false, &Utc).unwrap(),
            "已过期，没有人作答"
        );
    }

    #[test]
    fn next_run_format() {
        // 2026-11-20 is a Friday.
        let now = dt(2026, 11, 20, 1, 0);
        assert_eq!(format_next_run(dt(2026, 11, 20, 23, 0), now), "今天 23:00");
        assert_eq!(format_next_run(dt(2026, 11, 21, 8, 0), now), "明天 08:00");
        assert_eq!(format_next_run(dt(2026, 11, 23, 9, 30), now), "周一 09:30");
        assert_eq!(format_next_run(dt(2026, 11, 26, 9, 30), now), "周四 09:30");
        assert_eq!(
            format_next_run(dt(2026, 11, 30, 8, 0), now),
            "11月30日 08:00"
        );
    }

    #[test]
    fn clamp_preview() {
        assert_eq!(clamp_lines("short  text\nhere", 40, 1), "short text here");
        let long = "大家好，第一期的 7 个节点里已经合并了 5 个：任务队列、Docker 沙箱、三级权限、定时任务和客户端外壳。";
        let one = clamp_lines(long, 40, 1);
        assert!(one.ends_with('…'));
        assert!(one.chars().map(units).sum::<usize>() <= 40);
        let three = clamp_lines(long, 40, 3);
        assert_eq!(three, long, "fits in three lines");
    }
}
