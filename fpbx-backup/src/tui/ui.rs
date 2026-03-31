use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap,
    },
    Frame,
};

use super::app::{App, AppScreen};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const OK: Color = Color::Green;
const ERR: Color = Color::Red;
const TITLE: Color = Color::White;
const STEPS: usize = 5;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Outer layout: header / body / footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(0),    // body
            Constraint::Length(1), // footer / keybindings
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_footer(f, app, chunks[2]);

    match &app.screen.clone() {
        AppScreen::Server => draw_server(f, app, chunks[1]),
        AppScreen::Domains => draw_domains(f, app, chunks[1]),
        AppScreen::OutputPath => draw_output(f, app, chunks[1]),
        AppScreen::Progress => draw_progress(f, app, chunks[1]),
        AppScreen::Done => draw_done(f, app, chunks[1]),
        AppScreen::Error(msg) => {
            draw_domains(f, app, chunks[1]); // background
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
    let mut spans: Vec<Span> = vec![Span::styled(" fpbx-backup  ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))];

    for (i, label) in step_labels.iter().enumerate() {
        if i == step_idx {
            spans.push(Span::styled(
                format!("[{}] ", label),
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {}  ", label),
                Style::default().fg(MUTED),
            ));
        }
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(MUTED)));
    f.render_widget(paragraph, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        AppScreen::Server => " Tab switch field   Enter verify/continue   q quit",
        AppScreen::Domains => " ↑↓/jk navigate   / filter   Enter select   Esc back   q quit",
        AppScreen::OutputPath => " Enter start backup   Esc back",
        AppScreen::Progress => " (working…)",
        AppScreen::Done => " Enter/q quit",
        AppScreen::Error(_) => " Esc dismiss",
    };
    let p = Paragraph::new(hints).style(Style::default().fg(MUTED));
    f.render_widget(p, area);
}

// --- Server screen ---

fn draw_server(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // spacing
            Constraint::Length(3),  // host field
            Constraint::Length(3),  // user field
            Constraint::Length(2),  // spacing
            Constraint::Length(3),  // status
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let host_style = if app.active_field == 0 {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };
    let user_style = if app.active_field == 1 {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };

    let host_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(host_style)
        .title(Span::styled(" Host ", host_style));
    let host_text = Paragraph::new(app.host_input.as_str())
        .block(host_block)
        .style(Style::default().fg(TITLE));
    f.render_widget(host_text, chunks[1]);

    // Show cursor on active field.
    if app.active_field == 0 {
        let x = chunks[1].x + 1 + app.host_input.len() as u16;
        let y = chunks[1].y + 1;
        f.set_cursor_position((x, y));
    }

    let user_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(user_style)
        .title(Span::styled(" SSH user ", user_style));
    let user_text = Paragraph::new(app.user_input.as_str())
        .block(user_block)
        .style(Style::default().fg(TITLE));
    f.render_widget(user_text, chunks[2]);

    if app.active_field == 1 {
        let x = chunks[2].x + 1 + app.user_input.len() as u16;
        let y = chunks[2].y + 1;
        f.set_cursor_position((x, y));
    }

    // Verify status.
    let status_widget = if app.verifying && app.worker.as_ref().map(|w| !w.lock().unwrap().done).unwrap_or(true) {
        Paragraph::new("⟳ Verifying SSH + FusionPBX access…")
            .style(Style::default().fg(Color::Yellow))
    } else if let Some(Ok(v)) = &app.verify_result {
        let color = if v.is_ok() { OK } else { ERR };
        Paragraph::new(v.summary()).style(Style::default().fg(color))
    } else if let Some(Err(e)) = &app.verify_result {
        Paragraph::new(format!("✗ {}", e)).style(Style::default().fg(ERR))
    } else {
        Paragraph::new("Press Enter to verify").style(Style::default().fg(MUTED))
    };

    f.render_widget(status_widget, chunks[4]);
}

// --- Domains screen ---

fn draw_domains(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // filter bar
            Constraint::Min(0),    // list
        ])
        .margin(1)
        .split(area);

    // Filter input.
    let filter_style = if app.filter_active {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };
    let filter_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(filter_style)
        .title(Span::styled(" / filter ", filter_style));
    let filter_text = Paragraph::new(app.domain_filter.as_str())
        .block(filter_block);
    f.render_widget(filter_text, chunks[0]);

    if app.filter_active {
        let x = chunks[0].x + 1 + app.domain_filter.len() as u16;
        let y = chunks[0].y + 1;
        f.set_cursor_position((x, y));
    }

    // Domain list.
    let filtered = app.filtered_domains();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|d| {
            let enabled_marker = if d.domain_enabled { "●" } else { "○" };
            let color = if d.domain_enabled { OK } else { MUTED };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", enabled_marker), Style::default().fg(color)),
                Span::styled(d.domain_name.clone(), Style::default().fg(TITLE)),
                Span::styled(
                    d.domain_description
                        .as_deref()
                        .map(|s| format!("  {}", s))
                        .unwrap_or_default(),
                    Style::default().fg(MUTED),
                ),
            ]))
        })
        .collect();

    let title = if app.loading_domains {
        " Loading domains… "
    } else {
        " Domains "
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(title, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], &mut app.domain_list_state);
}

// --- Output path screen ---

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

    // Domain summary.
    if let Some(d) = app.selected_domain() {
        let summary = Paragraph::new(format!("Backing up:  {}", d.label()))
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD));
        f.render_widget(summary, chunks[1]);
    }

    // Output path field.
    let path_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(" Save bundle to ", Style::default().fg(ACCENT)));
    let path_text = Paragraph::new(app.output_path_input.as_str())
        .block(path_block)
        .style(Style::default().fg(TITLE));
    f.render_widget(path_text, chunks[3]);

    let x = chunks[3].x + 1 + app.output_path_input.len() as u16;
    let y = chunks[3].y + 1;
    f.set_cursor_position((x, y));
}

// --- Progress screen ---

fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // current task
            Constraint::Length(3),  // gauge
            Constraint::Min(0),     // log
        ])
        .margin(2)
        .split(area);

    let (log, progress, current_task) = if let Some(w) = &app.worker {
        let w = w.lock().unwrap();
        (w.log.clone(), w.progress, w.current_task.clone())
    } else {
        (vec![], 0.0, String::new())
    };

    let task_text = Paragraph::new(current_task.clone())
        .style(Style::default().fg(ACCENT))
        .block(
            Block::default()
                .borders(Borders::NONE)
                .title(Span::styled(" Current task ", Style::default().fg(MUTED))),
        );
    f.render_widget(task_text, chunks[0]);

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE))
        .gauge_style(Style::default().fg(ACCENT).bg(Color::DarkGray))
        .ratio(progress)
        .label(format!("{:.0}%", progress * 100.0));
    f.render_widget(gauge, chunks[1]);

    // Scrollable log panel — show last N lines that fit.
    let log_height = chunks[2].height.saturating_sub(2) as usize;
    let visible: Vec<ListItem> = log
        .iter()
        .rev()
        .take(log_height)
        .rev()
        .map(|line| {
            let style = if line.starts_with('✓') {
                Style::default().fg(OK)
            } else if line.starts_with('✗') {
                Style::default().fg(ERR)
            } else {
                Style::default().fg(MUTED)
            };
            ListItem::new(Span::styled(format!(" {}", line), style))
        })
        .collect();

    let log_list = List::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(MUTED))
            .title(Span::styled(" Log ", Style::default().fg(MUTED))),
    );
    f.render_widget(log_list, chunks[2]);
}

// --- Done screen ---

fn draw_done(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let mut lines = vec![
        Line::from(Span::styled(
            "✓ Backup complete",
            Style::default().fg(OK).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let Some(path) = &app.bundle_path {
        lines.push(Line::from(vec![
            Span::styled("Bundle: ", Style::default().fg(MUTED)),
            Span::styled(path.display().to_string(), Style::default().fg(TITLE)),
        ]));
    }

    // Log summary.
    if let Some(w) = &app.worker {
        let w = w.lock().unwrap();
        for msg in w.log.iter().filter(|m| m.starts_with('✓')) {
            lines.push(Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(OK),
            )));
        }
    }

    let summary = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(OK))
                .title(Span::styled(" Summary ", Style::default().fg(OK))),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(summary, chunks[1]);
}

// --- Error overlay ---

fn draw_error(f: &mut Frame, msg: String, area: Rect) {
    let popup = centered_rect(60, 30, area);
    f.render_widget(Clear, popup);
    let paragraph = Paragraph::new(msg)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ERR))
                .title(Span::styled(" Error ", Style::default().fg(ERR).add_modifier(Modifier::BOLD))),
        )
        .style(Style::default().fg(ERR))
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, popup);
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
