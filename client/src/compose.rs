//! What the composer sends.
//!
//! The host has two routes for it: `POST /api/groups/:id/messages` stores
//! the text (nobody acts on it) and `POST /api/groups/:id/tasks` queues a
//! prompt for one bro. So:
//! - in a DM, the text is stored and queued for the bro behind the DM;
//! - in a group, `@名字 …` queues the rest for that bro; anything else is
//!   only stored (the host has no @mention routing of its own).

use robroz_contract::Bro;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// Stored as a text message in the chat.
    pub body: String,
    /// `(bot id, prompt)` to queue as a task, if any bro should act.
    pub task: Option<(String, String)>,
}

/// Plan a send for `group`. `None` when there is nothing to send.
pub fn plan(text: &str, group: &str, bros: &[Bro]) -> Option<Outgoing> {
    let body = text.trim();
    if body.is_empty() {
        return None;
    }
    let task = if let Some(bro) = bros
        .iter()
        .find(|b| b.dm_group_id.as_deref() == Some(group))
    {
        Some((bro.id.clone(), body.to_string()))
    } else {
        mention(body, bros)
    };
    Some(Outgoing {
        body: body.to_string(),
        task,
    })
}

/// `@名字 rest` → that bro and `rest`; the longest matching name wins
/// (so `@bro 2` beats `@bro`).
fn mention(body: &str, bros: &[Bro]) -> Option<(String, String)> {
    let after = body.strip_prefix('@').or_else(|| body.strip_prefix('＠'))?;
    let mut best: Option<(&Bro, &str)> = None;
    for b in bros {
        let Some(rest) = after.strip_prefix(b.name.as_str()) else {
            continue;
        };
        let boundary = rest.is_empty() || rest.starts_with(char::is_whitespace);
        if boundary && best.is_none_or(|(prev, _)| b.name.len() > prev.name.len()) {
            best = Some((b, rest));
        }
    }
    let (bro, rest) = best?;
    let prompt = rest.trim();
    (!prompt.is_empty()).then(|| (bro.id.clone(), prompt.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bro(id: &str, name: &str, dm: &str) -> Bro {
        Bro {
            id: id.into(),
            name: name.into(),
            dm_group_id: Some(dm.into()),
        }
    }

    #[test]
    fn dm_text_goes_to_its_bro() {
        let bros = [bro("b1", "Ada", "dm-ada")];
        let out = plan("  整理周报 ", "dm-ada", &bros).unwrap();
        assert_eq!(out.body, "整理周报");
        assert_eq!(out.task, Some(("b1".into(), "整理周报".into())));
        assert_eq!(plan("   ", "dm-ada", &bros), None);
    }

    #[test]
    fn group_needs_a_mention_to_queue() {
        let bros = [bro("b1", "bro", "dm1"), bro("b2", "bro 2", "dm2")];
        let plain = plan("大家好", "g1", &bros).unwrap();
        assert_eq!(plain.task, None);
        let to_two = plan("@bro 2 写 README", "g1", &bros).unwrap();
        assert_eq!(to_two.task, Some(("b2".into(), "写 README".into())));
        assert_eq!(to_two.body, "@bro 2 写 README");
        let to_one = plan("＠bro 看看日志", "g1", &bros).unwrap();
        assert_eq!(to_one.task, Some(("b1".into(), "看看日志".into())));
        // No prompt, unknown name, or no word boundary: message only.
        assert_eq!(plan("@bro", "g1", &bros).unwrap().task, None);
        assert_eq!(plan("@Bob 你好", "g1", &bros).unwrap().task, None);
        assert_eq!(plan("@brother 你好", "g1", &bros).unwrap().task, None);
    }
}
