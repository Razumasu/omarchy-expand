//! Drawing. Only the terminal's palette colours are used, so the UI follows the Omarchy theme.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Table,
    TableState, Wrap,
};

use super::form::{ComboField, ComboForm, GroupForm, SettingsForm};
use super::text_input::TextInput;
use super::{App, DeleteTarget, Mode, Pane};
use crate::typed::Matching;

const ACCENT: Color = Color::Blue;
const MUTED: Color = Color::DarkGray;
const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let [header, body, preview, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(7),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header, app);
    let [groups, combos] =
        Layout::horizontal([Constraint::Length(24), Constraint::Min(10)]).areas(body);
    draw_groups(frame, groups, app);
    draw_combos(frame, combos, app);
    draw_preview(frame, preview, app);
    draw_footer(frame, footer, app);

    match &app.mode {
        Mode::Combo(form) => draw_combo_form(frame, area, app, form),
        Mode::Group(form) => draw_group_form(frame, area, form),
        Mode::Settings(form) => draw_settings_form(frame, area, form),
        Mode::ConfirmDelete(target) => draw_confirm(frame, area, app, *target),
        Mode::Browse | Mode::Search => {}
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Paragraph::new(" omarchy-expand".bold()), area);
    let daemon = if app.daemon_running {
        Line::from("● expanding ".fg(GOOD))
    } else {
        Line::from("○ daemon not running ".fg(BAD))
    };
    frame.render_widget(Paragraph::new(daemon).alignment(Alignment::Right), area);
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { ACCENT } else { MUTED }))
        .padding(Padding::horizontal(1))
        .title(title)
}

fn draw_groups(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.pane == Pane::Groups && app.query.is_empty();
    let items: Vec<ListItem> = app
        .library
        .groups
        .iter()
        .map(|group| {
            let style = if group.enabled {
                Style::new()
            } else {
                Style::new().fg(MUTED)
            };
            let mut line = vec![
                Span::styled(group.name.clone(), style),
                Span::styled(format!(" {}", group.combos.len()), Style::new().fg(MUTED)),
            ];
            if !group.enabled {
                line.push(Span::styled(" off", Style::new().fg(MUTED)));
            }
            ListItem::new(Line::from(line))
        })
        .collect();
    let highlight = if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let list = List::new(items)
        .block(pane_block(" Groups ".into(), focused))
        .highlight_style(highlight);
    let mut state = ListState::default();
    if app.query.is_empty() && !app.library.groups.is_empty() {
        state.select(Some(app.group));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_combos(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.pane == Pane::Combos;
    let searching = !app.query.is_empty();
    let title = match app.library.groups.get(app.group) {
        _ if searching => format!(" Matching “{}” ", app.query),
        Some(group) => format!(" {} ", group.name),
        None => " Combos ".into(),
    };
    let block = pane_block(title, focused);
    let visible = app.visible_combos();

    if visible.is_empty() {
        let message = if app.library.groups.is_empty() {
            "No combos yet.\nPress a to add your first one."
        } else if searching {
            "Nothing matches."
        } else {
            "This group is empty.\nPress a to add a combo."
        };
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [middle] = Layout::vertical([Constraint::Length(2)])
            .flex(ratatui::layout::Flex::Center)
            .areas(inner);
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .fg(MUTED),
            middle,
        );
        return;
    }

    let combos: Vec<_> = visible
        .iter()
        .map(|&(group, index)| {
            (
                &app.library.groups[group],
                &app.library.groups[group].combos[index],
            )
        })
        .collect();
    let keyword_width = column_width(
        combos.iter().map(|(_, combo)| combo.keyword.as_str()),
        8,
        20,
    );
    let label_width = column_width(
        combos.iter().map(|(group, combo)| {
            if searching {
                group.name.as_str()
            } else {
                combo.name.as_str()
            }
        }),
        4,
        20,
    );
    let rows = combos.iter().map(|(group, combo)| {
        let active = group.enabled && combo.enabled;
        let dot = if active {
            "●".fg(GOOD)
        } else {
            "○".fg(MUTED)
        };
        let label = if searching { &group.name } else { &combo.name };
        let first_line = combo.snippet.lines().next().unwrap_or_default();
        let style = if active {
            Style::new()
        } else {
            Style::new().fg(MUTED)
        };
        Row::new([
            Cell::from(dot),
            Cell::from(combo.keyword.clone()),
            Cell::from(label.clone()),
            Cell::from(first_line.to_owned()),
        ])
        .style(style)
    });
    let header = Row::new([
        "",
        "Keyword",
        if searching { "Group" } else { "Name" },
        "Snippet",
    ])
    .style(Style::new().fg(MUTED));
    let highlight = if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(keyword_width),
            Constraint::Length(label_width),
            Constraint::Min(5),
        ],
    )
    .header(header)
    .column_spacing(2)
    .block(block)
    .row_highlight_style(highlight);
    let mut state = TableState::default().with_selected(Some(app.combo));
    frame.render_stateful_widget(table, area, &mut state);
}

fn column_width<'a>(texts: impl Iterator<Item = &'a str>, min: u16, max: u16) -> u16 {
    let widest = texts.map(|text| Span::raw(text).width()).max().unwrap_or(0);
    (widest as u16).clamp(min, max)
}

fn draw_preview(frame: &mut Frame, area: Rect, app: &App) {
    let selected = app
        .selected_combo()
        .map(|(group, index)| &app.library.groups[group].combos[index]);
    let mut title = " Snippet ".to_owned();
    if let Some(combo) = selected {
        if combo.matching == Matching::Loose {
            title.push_str("· loose ");
        }
        if !combo.case_sensitive {
            title.push_str("· ignores case ");
        }
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(MUTED))
        .padding(Padding::horizontal(1))
        .title(title);
    let text = selected.map_or_else(String::new, |combo| combo.snippet.clone());
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    if let Mode::Search = app.mode {
        let line = Line::from(vec![" /".fg(ACCENT).bold(), Span::raw(app.query.clone())]);
        let cursor_x = area.x + line.width() as u16;
        frame.render_widget(Paragraph::new(line), area);
        if cursor_x < area.right() {
            frame.set_cursor_position(Position::new(cursor_x, area.y));
        }
        return;
    }
    if let Some(status) = &app.status {
        let color = if status.is_error { BAD } else { GOOD };
        frame.render_widget(
            Paragraph::new(format!(" {}", status.message).fg(color)),
            area,
        );
        return;
    }
    let mut keys: Vec<(&str, &str)> = match app.pane {
        Pane::Groups => vec![
            ("a", "new group"),
            ("e", "rename"),
            ("space", "on/off"),
            ("d", "delete"),
            ("→", "combos"),
        ],
        Pane::Combos => vec![
            ("a", "new combo"),
            ("e", "edit"),
            ("space", "on/off"),
            ("d", "delete"),
            ("←", "groups"),
        ],
    };
    if app.query.is_empty() {
        keys.push(("/", "search"));
    } else {
        keys.push(("esc", "clear search"));
    }
    keys.extend([("s", "settings"), ("q", "quit")]);
    frame.render_widget(Paragraph::new(key_hints(&keys)), area);
}

fn key_hints(keys: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (key, description) in keys {
        spans.push(Span::styled(
            key.to_string(),
            Style::new().fg(ACCENT).bold(),
        ));
        spans.push(Span::styled(
            format!(" {description}  "),
            Style::new().fg(MUTED),
        ));
    }
    Line::from(spans)
}

/// A centred box no larger than the screen, cleared and framed.
fn popup(frame: &mut Frame, area: Rect, width: u16, height: u16, title: &str) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .padding(Padding::horizontal(1))
        .title(format!(" {title} ").bold());
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    inner
}

fn label(text: &str, focused: bool) -> Span<'static> {
    let text = format!("{text:<10}");
    if focused {
        Span::styled(text, Style::new().fg(ACCENT).bold())
    } else {
        Span::styled(text, Style::new().fg(MUTED))
    }
}

fn draw_text_row(frame: &mut Frame, area: Rect, name: &str, input: &TextInput, focused: bool) {
    let [label_area, input_area] =
        Layout::horizontal([Constraint::Length(10), Constraint::Min(1)]).areas(area);
    frame.render_widget(Paragraph::new(label(name, focused)), label_area);
    input.render(frame, input_area, focused);
}

fn draw_choice_row(frame: &mut Frame, area: Rect, name: &str, value: Line<'static>, focused: bool) {
    let mut spans = vec![label(name, focused)];
    spans.extend(value.spans);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn checkbox(checked: bool, text: &str) -> Line<'static> {
    let mark = if checked { "[x] " } else { "[ ] " };
    Line::from(vec![mark.fg(ACCENT), Span::raw(text.to_owned())])
}

fn draw_form_footer(frame: &mut Frame, area: Rect, error: &Option<String>, keys: &[(&str, &str)]) {
    let [error_area, keys_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    if let Some(error) = error {
        frame.render_widget(Paragraph::new(error.clone().fg(BAD)), error_area);
    }
    frame.render_widget(Paragraph::new(key_hints(keys)), keys_area);
}

fn draw_combo_form(frame: &mut Frame, area: Rect, app: &App, form: &ComboForm) {
    let title = if form.editing.is_some() {
        "Edit combo"
    } else {
        "New combo"
    };
    let inner = popup(frame, area, 76, 22, title);
    let [
        keyword,
        name,
        snippet_label,
        snippet,
        group,
        matching,
        case,
        enabled,
        footer,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(inner);
    let focus = |field| form.field == field;

    draw_text_row(
        frame,
        keyword,
        "Keyword",
        &form.keyword,
        focus(ComboField::Keyword),
    );
    draw_text_row(frame, name, "Name", &form.name, focus(ComboField::Name));

    frame.render_widget(
        Paragraph::new(label("Snippet", focus(ComboField::Snippet))),
        snippet_label,
    );
    let snippet_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focus(ComboField::Snippet) {
            ACCENT
        } else {
            MUTED
        }));
    let snippet_inner = snippet_block.inner(snippet);
    frame.render_widget(snippet_block, snippet);
    form.snippet
        .render(frame, snippet_inner, focus(ComboField::Snippet));

    let group_name = app
        .library
        .groups
        .get(form.group)
        .map_or(super::FIRST_GROUP_NAME, |group| group.name.as_str());
    draw_choice_row(
        frame,
        group,
        "Group",
        Line::from(vec![
            "‹ ".fg(ACCENT),
            Span::raw(group_name.to_owned()),
            " ›".fg(ACCENT),
        ]),
        focus(ComboField::Group),
    );
    let (mode, explanation) = match form.matching {
        Matching::Strict => ("strict", "  only when typed as a whole word"),
        Matching::Loose => ("loose", "  also at the end of a word"),
    };
    draw_choice_row(
        frame,
        matching,
        "Matching",
        Line::from(vec![
            "‹ ".fg(ACCENT),
            Span::raw(mode),
            " ›".fg(ACCENT),
            explanation.fg(MUTED),
        ]),
        focus(ComboField::Matching),
    );
    draw_choice_row(
        frame,
        case,
        "Case",
        checkbox(form.case_sensitive, "case sensitive"),
        focus(ComboField::CaseSensitive),
    );
    draw_choice_row(
        frame,
        enabled,
        "Enabled",
        checkbox(form.enabled, "expand this combo"),
        focus(ComboField::Enabled),
    );

    // Enter starts a new line in the snippet, so only Ctrl+S saves from there.
    let keys: &[(&str, &str)] = if focus(ComboField::Snippet) {
        &[("tab", "next field"), ("ctrl+s", "save"), ("esc", "cancel")]
    } else {
        &[
            ("tab", "next field"),
            ("enter", "save"),
            ("ctrl+s", "save"),
            ("esc", "cancel"),
        ]
    };
    draw_form_footer(frame, footer, &form.error, keys);
}

fn draw_group_form(frame: &mut Frame, area: Rect, form: &GroupForm) {
    let title = if form.editing.is_some() {
        "Rename group"
    } else {
        "New group"
    };
    let inner = popup(frame, area, 50, 6, title);
    let [name, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(2)]).areas(inner);
    draw_text_row(frame, name, "Name", &form.name, true);
    draw_form_footer(
        frame,
        footer,
        &form.error,
        &[("enter", "save"), ("esc", "cancel")],
    );
}

fn draw_settings_form(frame: &mut Frame, area: Rect, form: &SettingsForm) {
    let inner = popup(frame, area, 64, 18, "Excluded apps");
    let [explanation, apps, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::raw("Never record or expand in these apps."),
            Line::from("One window class or program name per line.".fg(MUTED)),
        ])),
        explanation,
    );
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));
    let apps_inner = block.inner(apps);
    frame.render_widget(block, apps);
    form.excluded_apps.render(frame, apps_inner, true);
    draw_form_footer(
        frame,
        footer,
        &form.error,
        &[("ctrl+s", "save"), ("esc", "cancel")],
    );
}

fn draw_confirm(frame: &mut Frame, area: Rect, app: &App, target: DeleteTarget) {
    let question = match target {
        DeleteTarget::Combo(group, index) => {
            format!(
                "Delete combo “{}”?",
                app.library.groups[group].combos[index].keyword
            )
        }
        DeleteTarget::Group(group) => {
            let group = &app.library.groups[group];
            match group.combos.len() {
                0 => format!("Delete group “{}”?", group.name),
                1 => format!("Delete group “{}” and its combo?", group.name),
                count => format!("Delete group “{}” and its {count} combos?", group.name),
            }
        }
    };
    let inner = popup(frame, area, 56, 5, "Delete");
    let [question_area, keys] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(question).wrap(Wrap { trim: true }),
        question_area,
    );
    frame.render_widget(
        Paragraph::new(key_hints(&[("y", "delete"), ("any key", "keep")])),
        keys,
    );
}
