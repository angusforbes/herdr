use super::AgentPanelEntry;
use crate::config::{
    AgentSidebarToken, AgentsSidebarConfig, SidebarTokenStyle, SpaceSidebarToken,
    SpacesSidebarConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedToken {
    pub kind: ResolvedTokenKind,
    pub style: SidebarTokenStyle,
    /// Per-segment colours for custom tokens using inline `{#rrggbb}` markup.
    /// `None` = render the text with the single token style.
    pub rich: Option<Vec<RichSegment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RichSegment {
    pub text: String,
    pub fg: Option<crate::config::SidebarTokenColor>,
}

/// Parse `{#rrggbb}text{#rgb}more{}plain` into plain text + coloured segments.
/// `{}` resets to the token's own colour. Braces that don't form a valid tag are literal.
pub(crate) fn parse_rich_markup(value: &str) -> (String, Option<Vec<RichSegment>>) {
    let mut plain = String::new();
    let mut segments: Vec<RichSegment> = Vec::new();
    let mut current = RichSegment {
        text: String::new(),
        fg: None,
    };
    let mut saw_tag = false;
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        current.text.push_str(before);
        plain.push_str(before);
        let Some(close) = after.find('}') else {
            rest = after;
            break;
        };
        let tag = &after[1..close];
        let parsed = if tag.is_empty() {
            Some(None)
        } else {
            crate::config::SidebarTokenColor::parse_hex(tag).map(Some)
        };
        match parsed {
            Some(fg) => {
                saw_tag = true;
                if !current.text.is_empty() {
                    segments.push(std::mem::replace(
                        &mut current,
                        RichSegment {
                            text: String::new(),
                            fg,
                        },
                    ));
                } else {
                    current.fg = fg;
                }
                rest = &after[close + 1..];
            }
            None => {
                current.text.push('{');
                plain.push('{');
                rest = &after[1..];
            }
        }
    }
    current.text.push_str(rest);
    plain.push_str(rest);
    if !current.text.is_empty() {
        segments.push(current);
    }
    if saw_tag {
        (plain, Some(segments))
    } else {
        (plain, None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolvedTokenKind {
    StateIcon,
    StateText(String),
    Index(String),
    Workspace(String),
    Tab(String),
    Pane(String),
    Agent(String),
    TerminalTitle(String),
    Branch(String),
    GitStatus { ahead: usize, behind: usize },
    Custom(String),
}

impl ResolvedToken {
    fn new(kind: ResolvedTokenKind, style: SidebarTokenStyle) -> Self {
        Self {
            kind,
            style,
            rich: None,
        }
    }

    #[cfg(test)]
    pub(super) fn unstyled(kind: ResolvedTokenKind) -> Self {
        Self::new(kind, SidebarTokenStyle::default())
    }
}

pub(super) fn agent_rows(
    config: &AgentsSidebarConfig,
    entry: &AgentPanelEntry,
    state_text: &str,
) -> Vec<Vec<ResolvedToken>> {
    config
        .rows_for_agent(entry.agent)
        .iter()
        .filter_map(|row| {
            let resolved = row
                .iter()
                .filter_map(|configured| {
                    let (token, style) = configured.parts();
                    let kind = match token {
                        AgentSidebarToken::StateIcon => Some(ResolvedTokenKind::StateIcon),
                        AgentSidebarToken::StateText => {
                            Some(ResolvedTokenKind::StateText(state_text.to_string()))
                        }
                        AgentSidebarToken::Index => {
                            Some(ResolvedTokenKind::Index(entry.index.to_string()))
                        }
                        AgentSidebarToken::Workspace => {
                            Some(ResolvedTokenKind::Workspace(entry.primary_label.clone()))
                        }
                        AgentSidebarToken::Tab => {
                            entry.primary_tab_label.clone().map(ResolvedTokenKind::Tab)
                        }
                        AgentSidebarToken::Pane => {
                            entry.pane_label.clone().map(ResolvedTokenKind::Pane)
                        }
                        AgentSidebarToken::Agent => {
                            entry.agent_label.clone().map(ResolvedTokenKind::Agent)
                        }
                        AgentSidebarToken::TerminalTitle => entry
                            .terminal_title
                            .clone()
                            .map(ResolvedTokenKind::TerminalTitle),
                        AgentSidebarToken::TerminalTitleStripped => entry
                            .terminal_title_stripped
                            .clone()
                            .map(ResolvedTokenKind::TerminalTitle),
                        AgentSidebarToken::Custom(name) => {
                            let value = entry.tokens.get(name).cloned()?;
                            // A pane may style its own custom token by reporting sibling
                            // tokens: `<name>_fg=#rrggbb`, `<name>_bold=true`, `<name>_italic=true`.
                            // Explicit config styling still wins.
                            let style = custom_token_style(&entry.tokens, name, style);
                            let (plain, rich) = parse_rich_markup(&value);
                            let mut token =
                                ResolvedToken::new(ResolvedTokenKind::Custom(plain), style);
                            token.rich = rich;
                            return Some(token);
                        }
                        AgentSidebarToken::Styled { .. } => None,
                    }?;
                    Some(ResolvedToken::new(kind, style))
                })
                .collect::<Vec<_>>();
            (!resolved.is_empty()).then_some(resolved)
        })
        .collect()
}

pub(super) fn custom_token_style(
    tokens: &std::collections::HashMap<String, String>,
    name: &str,
    mut style: SidebarTokenStyle,
) -> SidebarTokenStyle {
    let flag = |suffix: &str| {
        tokens
            .get(&format!("{name}_{suffix}"))
            .map(|v| matches!(v.trim(), "true" | "1" | "yes" | "on"))
    };
    if style.fg.is_none() {
        style.fg = tokens
            .get(&format!("{name}_fg"))
            .and_then(|v| crate::config::SidebarTokenColor::parse_hex(v));
    }
    if style.bold.is_none() {
        style.bold = flag("bold");
    }
    if style.italic.is_none() {
        style.italic = flag("italic");
    }
    if style.dim.is_none() {
        style.dim = flag("dim");
    }
    style
}

pub(super) struct SpaceTokenContext<'a> {
    pub workspace: &'a str,
    pub branch: Option<&'a str>,
    pub state_text: &'a str,
    pub ahead_behind: Option<(usize, usize)>,
    pub tokens: &'a std::collections::HashMap<String, String>,
    pub suppress_git_details: bool,
}

pub(super) fn space_rows(
    config: &SpacesSidebarConfig,
    context: SpaceTokenContext<'_>,
) -> Vec<Vec<ResolvedToken>> {
    config
        .rows
        .iter()
        .filter_map(|row| {
            let resolved = row
                .iter()
                .filter_map(|configured| {
                    let (token, style) = configured.parts();
                    let kind = match token {
                        SpaceSidebarToken::StateIcon => Some(ResolvedTokenKind::StateIcon),
                        SpaceSidebarToken::StateText => {
                            Some(ResolvedTokenKind::StateText(context.state_text.to_string()))
                        }
                        SpaceSidebarToken::Workspace => {
                            Some(ResolvedTokenKind::Workspace(context.workspace.to_string()))
                        }
                        SpaceSidebarToken::Branch if !context.suppress_git_details => context
                            .branch
                            .map(|branch| ResolvedTokenKind::Branch(branch.to_string())),
                        SpaceSidebarToken::Branch => None,
                        SpaceSidebarToken::GitStatus if !context.suppress_git_details => context
                            .ahead_behind
                            .filter(|(ahead, behind)| *ahead > 0 || *behind > 0)
                            .map(|(ahead, behind)| ResolvedTokenKind::GitStatus { ahead, behind }),
                        SpaceSidebarToken::GitStatus => None,
                        SpaceSidebarToken::Custom(name) => context
                            .tokens
                            .get(name)
                            .cloned()
                            .map(ResolvedTokenKind::Custom),
                        SpaceSidebarToken::Styled { .. } => None,
                    }?;
                    Some(ResolvedToken::new(kind, style))
                })
                .collect::<Vec<_>>();
            (!resolved.is_empty()).then_some(resolved)
        })
        .collect()
}

pub(super) fn separator(previous: &ResolvedToken, current: &ResolvedToken) -> &'static str {
    if matches!(previous.kind, ResolvedTokenKind::StateIcon)
        || matches!(current.kind, ResolvedTokenKind::GitStatus { .. })
    {
        " "
    } else {
        " · "
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentSidebarToken, SpaceSidebarToken};
    use crate::detect::AgentState;

    fn entry() -> AgentPanelEntry {
        AgentPanelEntry {
            index: 1,
            ws_idx: 0,
            tab_idx: 0,
            pane_id: crate::layout::PaneId::from_raw(1),
            primary_label: "repo".into(),
            primary_tab_label: None,
            pane_label: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_label: Some("pi".into()),
            agent_kind_label: Some("pi".into()),
            agent: Some(crate::detect::Agent::Pi),
            state: AgentState::Working,
            seen: true,
            last_agent_state_change_seq: None,
            state_labels: std::collections::HashMap::new(),
            tokens: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn missing_custom_tokens_elide_rows_and_separators() {
        let entry = entry();
        let config = AgentsSidebarConfig {
            rows: vec![
                vec![
                    AgentSidebarToken::StateIcon,
                    AgentSidebarToken::Custom("missing".into()),
                ],
                vec![AgentSidebarToken::Custom("missing".into())],
                vec![AgentSidebarToken::Agent],
            ],
            ..Default::default()
        };

        let rows = agent_rows(&config, &entry, "working");

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            vec![ResolvedToken::unstyled(ResolvedTokenKind::StateIcon)]
        );
        assert_eq!(
            rows[1],
            vec![ResolvedToken::unstyled(ResolvedTokenKind::Agent(
                "pi".into()
            ))]
        );
    }

    #[test]
    fn state_text_and_arbitrary_values_are_independent_tokens() {
        let mut entry = entry();
        entry
            .tokens
            .insert("summary".into(), "reviewing auth".into());
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::StateText,
                AgentSidebarToken::Custom("summary".into()),
            ]],
            ..Default::default()
        };

        assert_eq!(
            agent_rows(&config, &entry, "deep in the mines"),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::StateText("deep in the mines".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::Custom("reviewing auth".into())),
            ]]
        );
    }

    #[test]
    fn terminal_title_builtins_are_distinct_from_custom_tokens() {
        let mut entry = entry();
        entry.terminal_title = Some("⠋ raw title".into());
        entry.terminal_title_stripped = Some("raw title".into());
        entry
            .tokens
            .insert("terminal_title".into(), "custom title".into());
        let config = AgentsSidebarConfig {
            rows: vec![vec![
                AgentSidebarToken::TerminalTitle,
                AgentSidebarToken::TerminalTitleStripped,
                AgentSidebarToken::Custom("terminal_title".into()),
            ]],
            ..Default::default()
        };

        assert_eq!(
            agent_rows(&config, &entry, "working"),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::TerminalTitle("⠋ raw title".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::TerminalTitle("raw title".into())),
                ResolvedToken::unstyled(ResolvedTokenKind::Custom("custom title".into())),
            ]]
        );
    }

    #[test]
    fn known_agent_override_replaces_default_rows() {
        let mut config = AgentsSidebarConfig {
            rows: vec![vec![AgentSidebarToken::Workspace]],
            ..Default::default()
        };
        config
            .rows_by_agent
            .insert("pi".into(), vec![vec![AgentSidebarToken::Agent]]);
        let mut pi = entry();
        pi.agent_label = Some("renamed pi".into());

        assert_eq!(
            agent_rows(&config, &pi, "working"),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Agent(
                "renamed pi".into()
            ))]]
        );

        pi.agent = None;
        assert_eq!(
            agent_rows(&config, &pi, "working"),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Workspace(
                "repo".into()
            ))]]
        );
    }

    #[test]
    fn grouped_children_suppress_all_builtin_git_details() {
        let config = SpacesSidebarConfig::default();

        assert_eq!(
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "feature",
                    branch: Some("worktree/feature"),
                    state_text: "idle",
                    ahead_behind: Some((2, 1)),
                    tokens: &std::collections::HashMap::new(),
                    suppress_git_details: true,
                },
            ),
            vec![vec![
                ResolvedToken::unstyled(ResolvedTokenKind::StateIcon),
                ResolvedToken::unstyled(ResolvedTokenKind::Workspace("feature".into())),
            ]]
        );
    }

    #[test]
    fn workspace_custom_token_can_replace_git_specific_details() {
        let tokens = std::collections::HashMap::from([("jj_status".into(), "2 changes".into())]);
        let config = SpacesSidebarConfig {
            rows: vec![vec![SpaceSidebarToken::Custom("jj_status".into())]],
            ..Default::default()
        };

        assert_eq!(
            space_rows(
                &config,
                SpaceTokenContext {
                    workspace: "repo",
                    branch: None,
                    state_text: "idle",
                    ahead_behind: None,
                    tokens: &tokens,
                    suppress_git_details: false,
                },
            ),
            vec![vec![ResolvedToken::unstyled(ResolvedTokenKind::Custom(
                "2 changes".into()
            ))]]
        );
    }
}

#[cfg(test)]
mod custom_style_tests {
    use super::*;

    #[test]
    fn custom_token_takes_style_from_sibling_tokens_unless_configured() {
        let mut tokens = std::collections::HashMap::new();
        tokens.insert("name".to_string(), "Splice".to_string());
        tokens.insert("name_fg".to_string(), "#ff8800".to_string());
        tokens.insert("name_bold".to_string(), "true".to_string());
        let style = custom_token_style(&tokens, "name", SidebarTokenStyle::default());
        assert_eq!(
            style.fg.map(|c| c.ratatui()),
            Some(ratatui::style::Color::Rgb(0xff, 0x88, 0x00))
        );
        assert_eq!(style.bold, Some(true));
        assert_eq!(style.italic, None);

        let configured = SidebarTokenStyle {
            fg: crate::config::SidebarTokenColor::parse_hex("#000"),
            bold: Some(false),
            ..Default::default()
        };
        let style = custom_token_style(&tokens, "name", configured);
        assert_eq!(
            style.fg.map(|c| c.ratatui()),
            Some(ratatui::style::Color::Rgb(0, 0, 0))
        );
        assert_eq!(style.bold, Some(false));

        tokens.insert("name_fg".to_string(), "orange".to_string());
        assert_eq!(
            custom_token_style(&tokens, "name", SidebarTokenStyle::default()).fg,
            None
        );
    }
}

#[cfg(test)]
mod rich_markup_tests {
    use super::*;

    fn rgb(c: &crate::config::SidebarTokenColor) -> ratatui::style::Color {
        c.ratatui()
    }

    #[test]
    fn markup_splits_into_coloured_segments_and_plain_text() {
        let (plain, rich) = parse_rich_markup("{#ff0000}Sp{#00ff00}li{}ce");
        assert_eq!(plain, "Splice");
        let rich = rich.unwrap();
        assert_eq!(rich.len(), 3);
        assert_eq!(rich[0].text, "Sp");
        assert_eq!(
            rich[0].fg.as_ref().map(rgb),
            Some(ratatui::style::Color::Rgb(255, 0, 0))
        );
        assert_eq!(rich[1].text, "li");
        assert_eq!(rich[2].text, "ce");
        assert_eq!(rich[2].fg, None);
    }

    #[test]
    fn markup_without_tags_or_with_bad_tags_is_literal() {
        assert_eq!(parse_rich_markup("Splice"), ("Splice".into(), None));
        assert_eq!(parse_rich_markup("a {b} c"), ("a {b} c".into(), None));
        assert_eq!(parse_rich_markup("x {#zz} y"), ("x {#zz} y".into(), None));
        let (plain, rich) = parse_rich_markup("{#abc}tail{");
        assert_eq!(plain, "tail{");
        assert!(rich.is_some());
    }
}
