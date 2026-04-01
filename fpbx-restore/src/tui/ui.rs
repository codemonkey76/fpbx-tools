use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::app::{App, AppScreen};

const ACCENT: Color = Color::Magenta;
const MUTED: Color = Color::DarkGray;
const OK: Color = Color::Green;
const ERR: Color = Color::Red;
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

    match app.screen.clone() {
        AppScreen::BundlePicker => draw_picker(f, app, chunks[1]),
        AppScreen::Preview => draw_preview(f, app, chunks[1]),
        AppScreen::Server => draw_server(f, app, chunks[1]),
        AppScreen::Confirm => draw_confirm(f, app, chunks[1]),
        AppScreen::Progress => draw_progress(f, app, chunks[1]),
        AppScreen::Done => draw_done(f, chunks[1]),
        AppScreen::Error(msg) => {
            draw_picker(f, app, chunks[1]);
            draw_error(f, msg, area);
        }
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let step_idx = match app.screen {
        AppScreen::BundlePicker => 0,
        AppScreen::Preview => 1,
        AppScreen::Server => 2,
        AppScreen::Confirm => 3,
        AppScreen::Progress | AppScreen::Done => 4,
        AppScreen::Error(_) => 0,
    };
    let labels = ["Bundle", "Preview", "Server", "Confirm", "Running"];
    let mut spans = vec![Span::styled(
        " fpbx-restore  ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    for (i, label) in labels.iter().enumerate() {
        if i == step_idx {
            spans.push(Span::styled(
                format!("[{}] ", label),
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(format!(" {}  ", label), Style::default().fg(MUTED)));
        }
    }
    let p = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(MUTED)));
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        AppScreen::BundlePicker => " ↑↓/jk navigate   Enter select   q quit",
        AppScreen::Preview => " Enter continue   Esc back",
        AppScreen::Server => " Tab switch field   Enter continue   Esc back",
        AppScreen::Confirm => " y/Enter confirm   n/Esc cancel",
        AppScreen::Progress => " (restoring…)",
        AppScreen::Done => " Enter/q quit",
        AppScreen::Error(_) => " Esc dismiss",
    };
    f.render_widget(Paragraph::new(hints).style(Style::default().fg(MUTED)), area);
}

fn draw_picker(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = if app.bundles.is_empty() {
        vec![ListItem::new(Span::styled(
            format!(" No .fpbx bundles found in {}", app.bundle_dir.display()),
            Style::default().fg(MUTED),
        ))]
    } else {
        app.bundles
            .iter()
            .map(|(path, m)| {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                let date = m.created_at.format("%Y-%m-%d %H:%M").to_string();
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", m.domain.domain_name), Style::default().fg(TITLE).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {}  ", date), Style::default().fg(MUTED)),
                    Span::styled(name, Style::default().fg(MUTED)),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(
                    " Select backup bundle ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(TITLE).add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, &mut app.bundle_list_state);
}

fn draw_preview(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .margin(2)
        .split(area);

    let mut lines = vec![
        Line::from(Span::styled("Bundle manifest", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    if let Some(m) = &app.selected_manifest {
        lines.push(Line::from(vec![
            Span::styled("Domain:      ", Style::default().fg(MUTED)),
            Span::styled(m.domain.domain_name.clone(), Style::default().fg(TITLE)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Source host: ", Style::default().fg(MUTED)),
            Span::styled(m.source_host.clone(), Style::default().fg(TITLE)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Created:     ", Style::default().fg(MUTED)),
            Span::styled(m.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(), Style::default().fg(TITLE)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Table row counts:", Style::default().fg(MUTED))));
        for (table, count) in &m.table_counts.0 {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:40}", table), Style::default().fg(MUTED)),
                Span::styled(count.to_string(), Style::default().fg(TITLE)),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Total rows:  ", Style::default().fg(MUTED)),
            Span::styled(m.table_counts.total_rows().to_string(), Style::default().fg(OK)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("File paths:  ", Style::default().fg(MUTED)),
            Span::styled(m.file_paths.len().to_string(), Style::default().fg(TITLE)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Enter to select destination server →",
        Style::default().fg(ACCENT),
    )));

    let p = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(MUTED)))
        .wrap(Wrap { trim: false });
    f.render_widget(p, chunks[0]);
}

fn draw_server(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let host_style = if app.active_field == 0 { Style::default().fg(ACCENT) } else { Style::default().fg(MUTED) };
    let user_style = if app.active_field == 1 { Style::default().fg(ACCENT) } else { Style::default().fg(MUTED) };

    let host_block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(host_style).title(Span::styled(" Destination host ", host_style));
    f.render_widget(Paragraph::new(app.host_input.as_str()).block(host_block).style(Style::default().fg(TITLE)), chunks[1]);
    if app.active_field == 0 {
        f.set_cursor_position((chunks[1].x + 1 + app.host_input.len() as u16, chunks[1].y + 1));
    }

    let user_block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(user_style).title(Span::styled(" SSH user ", user_style));
    f.render_widget(Paragraph::new(app.user_input.as_str()).block(user_block).style(Style::default().fg(TITLE)), chunks[2]);
    if app.active_field == 1 {
        f.set_cursor_position((chunks[2].x + 1 + app.user_input.len() as u16, chunks[2].y + 1));
    }
}

fn draw_confirm(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .margin(4)
        .split(area);

    let mut lines = vec![
        Line::from(Span::styled("Confirm restore", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];
    if let Some(m) = &app.selected_manifest {
        lines.push(Line::from(vec![
            Span::styled("Domain:       ", Style::default().fg(MUTED)),
            Span::styled(m.domain.domain_name.clone(), Style::default().fg(TITLE)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("Destination:  ", Style::default().fg(MUTED)),
        Span::styled(format!("{}@{}", app.user_input, app.host_input), Style::default().fg(TITLE)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "This will import database records and files into the destination server.",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "The source server is NOT modified.",
        Style::default().fg(OK),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press y or Enter to proceed, n or Esc to cancel.",
        Style::default().fg(ACCENT),
    )));

    let p = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow)))
        .wrap(Wrap { trim: false });
    f.render_widget(p, chunks[0]);
}

fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
        .margin(2)
        .split(area);

    let (log, progress, current_task) = if let Some(w) = &app.worker {
        let w = w.lock().unwrap();
        (w.log.clone(), w.progress, w.current_task.clone())
    } else {
        (vec![], 0.0, String::new())
    };

    f.render_widget(
        Paragraph::new(current_task)
            .style(Style::default().fg(ACCENT))
            .block(Block::default().borders(Borders::NONE)
                .title(Span::styled(" Current task ", Style::default().fg(MUTED)))),
        chunks[0],
    );
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::NONE))
            .gauge_style(Style::default().fg(ACCENT).bg(Color::DarkGray))
            .ratio(progress)
            .label(format!("{:.0}%", progress * 100.0)),
        chunks[1],
    );

    let log_height = chunks[2].height.saturating_sub(2) as usize;
    let visible: Vec<ListItem> = log.iter().rev().take(log_height).rev()
        .map(|line| {
            let style = if line.starts_with('✓') { Style::default().fg(OK) }
                else if line.starts_with('✗') { Style::default().fg(ERR) }
                else { Style::default().fg(MUTED) };
            ListItem::new(Span::styled(format!(" {}", line), style))
        })
        .collect();

    f.render_widget(
        List::new(visible).block(Block::default().borders(Borders::ALL)
            .border_type(BorderType::Rounded).border_style(Style::default().fg(MUTED))
            .title(Span::styled(" Log ", Style::default().fg(MUTED)))),
        chunks[2],
    );
}

fn draw_done(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(6), Constraint::Min(0)])
        .margin(4)
        .split(area);

    let lines = vec![
        Line::from(Span::styled("✓ Restore complete", Style::default().fg(OK).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("Domain has been restored to the destination server.", Style::default().fg(TITLE))),
        Line::from(""),
        Line::from(Span::styled("Press Enter or q to exit.", Style::default().fg(MUTED))),
    ];

    let p = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(OK)))
        .wrap(Wrap { trim: false });
    f.render_widget(p, chunks[1]);
}

fn draw_error(f: &mut Frame, msg: String, area: Rect) {
    let popup = centered_rect(60, 30, area);
    f.render_widget(Clear, popup);
    let p = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ERR))
            .title(Span::styled(" Error ", Style::default().fg(ERR).add_modifier(Modifier::BOLD))))
        .style(Style::default().fg(ERR))
        .wrap(Wrap { trim: true });
    f.render_widget(p, popup);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(layout[1])[1]
}
