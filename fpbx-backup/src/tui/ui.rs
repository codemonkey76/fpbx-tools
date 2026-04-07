use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use fpbx_tui_shared::{ServerInputs, VerifyStatus, draw_error, draw_progress, draw_server};

use super::app::{App, AppScreen};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const OK: Color = Color::Green;
const TITLE: Color = Color::White;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_footer(f, app, chunks[2]);

    match &app.screen.clone() {
        AppScreen::Server => draw_server_screen(f, app, chunks[1]),
        AppScreen::Domains => draw_domains(f, app, chunks[1]),
        AppScreen::OutputPath => draw_output(f, app, chunks[1]),
        AppScreen::Progress => draw_progress(f, chunks[1], &app.worker, ACCENT),
        AppScreen::Done => draw_done(f, app, chunks[1]),
        AppScreen::Error(msg) => {
            draw_domains(f, app, chunks[1]);
            draw_error(f, msg.clone(), area);
        }
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let step_idx = match app.screen {
        AppScreen::Server => 0,
        AppScreen::Domains => 1,
        AppScreen::OutputPath => 2,
        AppScreen::Progress => 3,
        AppScreen::Done => 4,
        AppScreen::Error(_) => 0,
    };
    let step_labels = ["Server", "Domain", "Output", "Running", "Done"];
    let mut spans: Vec<Span> = vec![Span::styled(
        " fpbx-backup  ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    for (i, label) in step_labels.iter().enumerate() {
        if i == step_idx {
            spans.push(Span::styled(
                format!("[{}] ", label),
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(format!(" {}  ", label), Style::default().fg(MUTED)));
        }
    }
    let paragraph = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(MUTED)));
    f.render_widget(paragraph, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        AppScreen::Server => " Tab switch field   Enter verify/continue   q quit",
        AppScreen::Domains => " ↑↓/jk navigate   Space toggle   a select all   / filter   Enter continue   Esc back   q quit",
        AppScreen::OutputPath => " Enter start backup   Esc back",
        AppScreen::Progress => " (working…)",
        AppScreen::Done => " Enter/q quit",
        AppScreen::Error(_) => " Esc dismiss",
    };
    f.render_widget(Paragraph::new(hints).style(Style::default().fg(MUTED)), area);
}

fn draw_server_screen(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.verifying {
        VerifyStatus::InProgress
    } else if let Some(Ok(v)) = &app.verify_result {
        if v.is_ok() { VerifyStatus::Ok(v.summary()) } else { VerifyStatus::Err(v.summary()) }
    } else if let Some(Err(e)) = &app.verify_result {
        VerifyStatus::Err(format!("✗ {}", e))
    } else {
        VerifyStatus::Idle
    };
    draw_server(f, area, ServerInputs {
        host: &app.host_input,
        user: &app.user_input,
        active_field: app.active_field,
        host_label: " Host ",
        status,
        accent: ACCENT,
    });
}

fn draw_domains(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .margin(1)
        .split(area);

    let filter_style = if app.filter_active { Style::default().fg(ACCENT) } else { Style::default().fg(MUTED) };
    let filter_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(filter_style)
        .title(Span::styled(" / filter ", filter_style));
    f.render_widget(Paragraph::new(app.domain_filter.as_str()).block(filter_block), chunks[0]);
    if app.filter_active {
        f.set_cursor_position((chunks[0].x + 1 + app.domain_filter.len() as u16, chunks[0].y + 1));
    }

    let filtered = app.filtered_domains();
    let selected_count = app.selected_domain_uuids.len();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|d| {
            let checked = app.selected_domain_uuids.contains(&d.domain_uuid);
            let check_color = if checked { OK } else { MUTED };
            let enabled_color = if d.domain_enabled { OK } else { MUTED };
            ListItem::new(Line::from(vec![
                Span::styled(if checked { "[✓] " } else { "[ ] " }, Style::default().fg(check_color)),
                Span::styled(if d.domain_enabled { "● " } else { "○ " }, Style::default().fg(enabled_color)),
                Span::styled(d.domain_name.clone(), Style::default().fg(TITLE)),
                Span::styled(
                    d.domain_description.as_deref().map(|s| format!("  {}", s)).unwrap_or_default(),
                    Style::default().fg(MUTED),
                ),
            ]))
        })
        .collect();

    let title = if app.loading_domains {
        " Loading domains… ".to_string()
    } else if selected_count > 0 {
        format!(" Domains ({} selected) ", selected_count)
    } else {
        " Domains ".to_string()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(title, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], &mut app.domain_list_state);
}

fn draw_output(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(5),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let selected = app.selected_domains();
    if !selected.is_empty() {
        let summary_text = if selected.len() == 1 {
            format!("Backing up:  {}", selected[0].label())
        } else {
            let names: Vec<&str> = selected.iter().map(|d| d.domain_name.as_str()).collect();
            format!("Backing up {} domains:  {}", selected.len(), names.join(", "))
        };
        f.render_widget(
            Paragraph::new(summary_text).style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            chunks[1],
        );
    }

    let path_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(" Save bundles to ", Style::default().fg(ACCENT)));
    f.render_widget(
        Paragraph::new(app.output_path_input.as_str()).block(path_block).style(Style::default().fg(TITLE)),
        chunks[3],
    );
    f.set_cursor_position((chunks[3].x + 1 + app.output_path_input.len() as u16, chunks[3].y + 1));
}

fn draw_done(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(8), Constraint::Min(0)])
        .margin(4)
        .split(area);

    let n = app.bundle_paths.len();
    let heading = if n == 1 {
        "✓ Backup complete".to_string()
    } else {
        format!("✓ Backup complete ({} domains)", n)
    };

    let mut lines = vec![
        Line::from(Span::styled(heading, Style::default().fg(OK).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];
    for path in &app.bundle_paths {
        lines.push(Line::from(vec![
            Span::styled("Bundle: ", Style::default().fg(MUTED)),
            Span::styled(path.display().to_string(), Style::default().fg(TITLE)),
        ]));
    }

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(Style::default().fg(OK))
                .title(Span::styled(" Summary ", Style::default().fg(OK))))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}
