//! `@Name` addressing in the room composer.
//!
//! A human message that *starts* with one or more `@Name` tokens is delivered only
//! to those current members; anything else is the usual broadcast. Names are the
//! chosen sidebar names with icons and `{#rrggbb}` markup stripped, compared
//! case-insensitively; a pane id (`@w4:pE`) also works. Multi-word names may be
//! written plainly (`@Tuning Fork`, matched greedily against current members),
//! quoted (`@"Tuning Fork"`), or joined (`@Tuning.Fork`, `@tuning_fork`). Unknown or
//! ambiguous mentions reject the whole post so the draft can be corrected; a
//! mention never silently falls back to broadcast.

use crate::api::schema::RoomRecipient;
use crate::room::Member;

/// Normalised comparison key: markup and icons removed, alphanumerics only, lowercase.
pub(crate) fn key(name: &str) -> String {
    let (plain, _) = crate::ui::sidebar::tokens::parse_rich_markup(name);
    plain
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Leading `@` mentions of `text`, in order, as raw (unresolved) strings.
/// A mention is `@word`, or `@"quoted words"`. Stops at the first non-mention word.
/// Multi-word names without quotes are handled in `resolve` by greedy matching.
pub(crate) fn leading_mentions(text: &str) -> Vec<&str> {
    resolve_tokens(text, |_| false).0
}

/// Walk the leading mentions. `is_name(candidate)` says whether a multi-word
/// candidate (mention + following plain words) names a member; when it does, the
/// following words are consumed as part of that mention. Returns the mentions.
fn resolve_tokens<'a>(text: &'a str, is_name: impl Fn(&str) -> bool) -> (Vec<&'a str>, ()) {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while let Some(after_at) = rest.strip_prefix('@') {
        let (mention, tail) = if let Some(quoted) = after_at.strip_prefix('"') {
            match quoted.find('"') {
                Some(end) => (&quoted[..end], &quoted[end + 1..]),
                None => break,
            }
        } else {
            let end = after_at.find(char::is_whitespace).unwrap_or(after_at.len());
            let (word, tail) = after_at.split_at(end);
            let word = word.trim_end_matches([',', ':', ';']);
            if word.is_empty() {
                break;
            }
            // Greedy: extend over following plain words while that still names a member
            // (`@Tuning Fork what's up` → "Tuning Fork"). Only when the bare word does not.
            let mut best = (word, tail);
            if !is_name(word) {
                let mut candidate_end = end;
                for _ in 0..3 {
                    let after = &after_at[candidate_end..];
                    let after_trim = after.trim_start();
                    if after_trim.starts_with('@') || after_trim.is_empty() {
                        break;
                    }
                    let ws = after.len() - after_trim.len();
                    let next_end = after_trim
                        .find(char::is_whitespace)
                        .unwrap_or(after_trim.len());
                    candidate_end += ws + next_end;
                    let candidate = after_at[..candidate_end].trim_end_matches([',', ':', ';']);
                    if is_name(candidate) {
                        best = (candidate, &after_at[candidate_end..]);
                        break;
                    }
                }
            }
            best
        };
        if mention.is_empty() {
            break;
        }
        out.push(mention);
        rest = tail.trim_start();
    }
    (out, ())
}

/// Resolve leading mentions to exact current recipients. `Ok(None)` means broadcast.
pub(crate) fn resolve(
    text: &str,
    members: &[Member],
) -> Result<Option<Vec<RoomRecipient>>, String> {
    let names: Vec<String> = members.iter().map(|m| key(&m.name)).collect();
    let panes: Vec<String> = members.iter().map(|m| key(&m.pane_id)).collect();
    let is_name = |candidate: &str| {
        let k = key(candidate);
        !k.is_empty() && (names.contains(&k) || panes.contains(&k))
    };
    let (mentions, ()) = resolve_tokens(text, is_name);
    if mentions.is_empty() {
        return Ok(None);
    }
    let mut out: Vec<RoomRecipient> = Vec::new();
    for mention in mentions {
        let wanted = key(mention);
        let by_pane: Vec<&Member> = members
            .iter()
            .filter(|m| key(&m.pane_id) == wanted)
            .collect();
        let hits: Vec<&Member> = if by_pane.is_empty() {
            members.iter().filter(|m| key(&m.name) == wanted).collect()
        } else {
            by_pane
        };
        let member = match hits.as_slice() {
            [one] => *one,
            [] => {
                let known: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
                return Err(format!(
                    "@{mention}: no such member here (members: {})",
                    known.join(", ")
                ));
            }
            many => {
                let panes: Vec<&str> = many.iter().map(|m| m.pane_id.as_str()).collect();
                return Err(format!(
                    "@{mention} is ambiguous ({}); use @<pane-id>",
                    panes.join(", ")
                ));
            }
        };
        let Some(session) = member.session.clone() else {
            return Err(format!(
                "@{mention} has no live session; cannot be addressed"
            ));
        };
        if !out
            .iter()
            .any(|r| r.terminal_id == member.terminal_id && r.session == session)
        {
            out.push(RoomRecipient {
                pane_id: member.pane_id.clone(),
                terminal_id: member.terminal_id.clone(),
                session,
            });
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(pane: &str, name: &str, session: Option<&str>) -> Member {
        Member {
            pane_id: pane.into(),
            terminal_id: format!("term_{pane}"),
            agent: "pi".into(),
            name: name.into(),
            session: session.map(str::to_owned),
        }
    }

    fn members() -> Vec<Member> {
        vec![
            member("w4:pE", "🗝 Keystone", Some("Path:/a.jsonl")),
            member("w4:pQ", "🎵 Tuning Fork", Some("Path:/b.jsonl")),
            member(
                "w7:p5",
                "{#e0af68}⋈ {#f7768e}S{#ff9e64}p{#e0af68}l{#9ece6a}i{#7dcfff}c{#bb9af7}e",
                Some("Path:/c.jsonl"),
            ),
            member("w4:pZ", "Ghost", None),
        ]
    }

    #[test]
    fn plain_text_is_broadcast() {
        assert_eq!(resolve("hello everyone", &members()), Ok(None));
        assert_eq!(resolve("mail me at a@b.c", &members()), Ok(None));
    }

    #[test]
    fn leading_mentions_stop_at_first_word() {
        assert_eq!(
            leading_mentions("@Keystone @Sift, hi @Cairn"),
            vec!["Keystone", "Sift"]
        );
        assert_eq!(leading_mentions("hi @Keystone"), Vec::<&str>::new());
    }

    #[test]
    fn resolves_by_name_ignoring_icons_case_and_markup() {
        let r = resolve("@keystone @Splice what colour?", &members())
            .unwrap()
            .unwrap();
        assert_eq!(
            r.iter().map(|x| x.pane_id.as_str()).collect::<Vec<_>>(),
            ["w4:pE", "w7:p5"]
        );
        assert_eq!(r[0].session, "Path:/a.jsonl");
    }

    #[test]
    fn multi_word_names_quoted_or_greedy() {
        for text in [
            "@\"Tuning Fork\" tune",
            "@Tuning Fork tune",
            "@Tuning Fork, tune",
            "@Tuning Fork @Keystone tune",
        ] {
            let r = resolve(text, &members()).unwrap().unwrap();
            assert_eq!(r[0].pane_id, "w4:pQ", "{text}");
        }
        let r = resolve("@Tuning Fork @Keystone tune", &members())
            .unwrap()
            .unwrap();
        assert_eq!(r.len(), 2);
        // A bare unknown word does not swallow the sentence into a name.
        assert!(resolve("@Tuning what", &members())
            .unwrap_err()
            .contains("no such member"));
    }

    #[test]
    fn multi_word_names_use_dot_or_underscore() {
        for form in ["@Tuning.Fork", "@tuning_fork", "@TuningFork"] {
            let r = resolve(&format!("{form} tune"), &members())
                .unwrap()
                .unwrap();
            assert_eq!(r[0].pane_id, "w4:pQ");
        }
    }

    #[test]
    fn pane_id_addressing_and_dedup() {
        let r = resolve("@w4:pE @Keystone hi", &members()).unwrap().unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn unknown_ambiguous_and_sessionless_are_errors() {
        assert!(resolve("@Nobody hi", &members())
            .unwrap_err()
            .contains("no such member"));
        assert!(resolve("@Ghost hi", &members())
            .unwrap_err()
            .contains("no live session"));
        let mut twins = members();
        twins.push(member("w9:p1", "Keystone", Some("Path:/d.jsonl")));
        let err = resolve("@Keystone hi", &twins).unwrap_err();
        assert!(err.contains("ambiguous") && err.contains("w9:p1"));
        let r = resolve("@w9:p1 hi", &twins).unwrap().unwrap();
        assert_eq!(r[0].pane_id, "w9:p1");
    }
}
