use super::app::{App, AppScreen};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};

const ACCENT: Color = Color::Yellow;
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
        AppScreen::Source => draw_server(f, app, chunks[1], true),
        AppScreen::Dest => draw_server(f, app, chunks[1], false),
        AppScreen::Routes => draw_routes(f, app, chunks[1]),
        AppScreen::Gateways => draw_gateways(f, app, chunks[1]),
        AppScreen::Confirm => draw_confirm(f, app, chunks[1]),
        AppScreen::Progress => draw_progress(f, app, chunks[1]),
        AppScreen::Done => draw_done(f, chunks[1]),
        AppScreen::Error(msg) => {
            draw_routes(f, app, chunks[1]);
            draw_error(f, msg, area);
        }
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let step_idx = match app.screen {
        AppScreen::Source => 0,
        AppScreen::Routes => 1,
        AppScreen::Dest => 2,
        AppScreen::Gateways => 3,
        AppScreen::Confirm => 4,
        AppScreen::Progress | AppScreen::Done => 5,
        AppScreen::Error(_) => 0,
    };
    let labels = ["Source", "Routes", "Dest", "Gateways", "Confirm", "Running"];
    let mut spans = vec![Span::styled(
        " fpbx-routes-xfer  ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    for (i, label) in labels.iter().enumerate() {
        if i == step_idx {
            spans.push(Span::styled(
                format!("[{}] ", label),
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {}  ", label),
                Style::default().fg(MUTED),
            ));
        }
    }
    let p = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(MUTED)),
    );
    f.render_widget(p, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        AppScreen::Source => " Tab switch field   Enter verify/continue   q quit",
        AppScreen::Dest => " Tab switch field   Enter verify/continue   Esc back",
        AppScreen::Routes => {
            " ↑↓ navigate   Space toggle   a select all   Enter continue   Esc back"
        }
        AppScreen::Gateways => " ↑↓ navigate   Enter select   s skip   Esc back",
        AppScreen::Confirm => " y/Enter confirm   n/Esc cancel",
        AppScreen::Progress => " (transferring…)",
        AppScreen::Done => " Enter/q quit",
        AppScreen::Error(_) => " Esc dismiss",
    };
    f.render_widget(
        Paragraph::new(hints).style(Style::default().fg(MUTED)),
        area,
    );
}

fn draw_server(f: &mut Frame, app: &App, area: Rect, is_source: bool) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let (host, user, active_field, verifying, verify_msg, verify_ok) = if is_source {
        (
            &app.src_host_input,
            &app.src_user_input,
            app.src_active_field,
            app.src_verifying,
            &app.src_verify_msg,
            app.src_verify_ok,
        )
    } else {
        (
            &app.dst_host_input,
            &app.dst_user_input,
            app.dst_active_field,
            app.dst_verifying,
            &app.dst_verify_msg,
            app.dst_verify_ok,
        )
    };

    let host_style = if active_field == 0 {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };
    let user_style = if active_field == 1 {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };

    let host_label = if is_source {
        " Source host "
    } else {
        " Destination host "
    };
    f.render_widget(
        Paragraph::new(host.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(host_style)
                    .title(Span::styled(host_label, host_style)),
            )
            .style(Style::default().fg(TITLE)),
        chunks[1],
    );
    if active_field == 0 {
        f.set_cursor_position((chunks[1].x + 1 + host.len() as u16, chunks[1].y + 1));
    }

    f.render_widget(
        Paragraph::new(user.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(user_style)
                    .title(Span::styled(" SSH user ", user_style)),
            )
            .style(Style::default().fg(TITLE)),
        chunks[2],
    );
    if active_field == 1 {
        f.set_cursor_position((chunks[2].x + 1 + user.len() as u16, chunks[2].y + 1));
    }

    let status = if verifying {
        Paragraph::new("⟳ Verifying…").style(Style::default().fg(Color::Yellow))
    } else if let Some(msg) = verify_msg {
        let color = if verify_ok { OK } else { ERR };
        Paragraph::new(msg.as_str()).style(Style::default().fg(color))
    } else {
        Paragraph::new("Press Enter to verify").style(Style::default().fg(MUTED))
    };
    f.render_widget(status, chunks[4]);
}

fn draw_routes(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .routes
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let check = if r.selected { "[✓]" } else { "[ ]" };
            let check_color = if r.selected { OK } else { MUTED };
            let focused = i == app.routes_list_idx;
            let name_style = if focused {
                Style::default().fg(TITLE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TITLE)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", check), Style::default().fg(check_color)),
                Span::styled(r.dialplan_name.clone(), name_style),
                Span::styled(
                    if r.dialplan_description.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", r.dialplan_description)
                    },
                    Style::default().fg(MUTED),
                ),
            ]))
        })
        .collect();

    let selected_count = app.routes.iter().filter(|r| r.selected).count();
    let title = if app.loading_routes {
        " Loading routes… ".to_string()
    } else {
        format!(" Outbound Routes ({} selected) ", selected_count)
    };

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(app.routes_list_idx));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(
                    title,
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(list, area, &mut list_state);
}

fn draw_gateways(f: &mut Frame, app: &App, area: Rect) {
    if app.gateway_mappings.is_empty() {
        f.render_widget(
            Paragraph::new("No gateway remapping required.").style(Style::default().fg(OK)),
            area,
        );
        return;
    }

    let mapping = &app.gateway_mappings[app.gateway_focus_idx];
    let total = app.gateway_mappings.len();
    let current = app.gateway_focus_idx + 1;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .margin(2)
        .split(area);

    // Header.
    let auto = mapping
        .dest_options
        .iter()
        .position(|g| g.name == mapping.source.name)
        .map(|_| " (auto-matched)")
        .unwrap_or("");
    let header = vec![
        Line::from(vec![
            Span::styled(
                format!("Gateway {}/{}: ", current, total),
                Style::default().fg(MUTED),
            ),
            Span::styled(
                mapping.source.name.clone(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(auto, Style::default().fg(OK)),
        ]),
        Line::from(vec![
            Span::styled("Source UUID: ", Style::default().fg(MUTED)),
            Span::styled(mapping.source.uuid.clone(), Style::default().fg(MUTED)),
        ]),
        Line::from(Span::styled(
            "Select matching gateway on destination:",
            Style::default().fg(TITLE),
        )),
    ];
    f.render_widget(Paragraph::new(Text::from(header)), chunks[0]);

    // Gateway list.
    let items: Vec<ListItem> = mapping
        .dest_options
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let selected = mapping.selected_idx == Some(i);
            let focused = mapping.list_state == i;
            let marker = if selected { "●" } else { "○" };
            let color = if selected {
                OK
            } else if focused {
                ACCENT
            } else {
                MUTED
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", marker), Style::default().fg(color)),
                Span::styled(g.name.clone(), Style::default().fg(TITLE)),
                Span::styled(format!("  {}", g.uuid), Style::default().fg(MUTED)),
            ]))
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(mapping.list_state));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(
                    " Destination gateways ",
                    Style::default().fg(ACCENT),
                )),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(TITLE));

    f.render_stateful_widget(list, chunks[1], &mut list_state);
}

fn draw_confirm(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .margin(4)
        .split(area);

    let mut lines = vec![
        Line::from(Span::styled(
            "Confirm transfer",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Source:      ", Style::default().fg(MUTED)),
            Span::styled(app.resolved_src_host(), Style::default().fg(TITLE)),
        ]),
        Line::from(vec![
            Span::styled("Destination: ", Style::default().fg(MUTED)),
            Span::styled(app.resolved_dst_host(), Style::default().fg(TITLE)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Routes to transfer:",
            Style::default().fg(MUTED),
        )),
    ];

    for r in app.routes.iter().filter(|r| r.selected) {
        lines.push(Line::from(vec![
            Span::styled("  ✓ ", Style::default().fg(OK)),
            Span::styled(r.dialplan_name.clone(), Style::default().fg(TITLE)),
            Span::styled(
                if r.dialplan_description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", r.dialplan_description)
                },
                Style::default().fg(MUTED),
            ),
        ]));
    }

    if !app.gateway_mappings.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Gateway remapping:",
            Style::default().fg(MUTED),
        )));
        for m in &app.gateway_mappings {
            let dest = m
                .resolved_dest_uuid()
                .and_then(|uuid| m.dest_options.iter().find(|g| g.uuid == uuid))
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "skipped".to_string());
            let color = if m.selected_idx.is_some() {
                OK
            } else {
                Color::Yellow
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} → ", m.source.name),
                    Style::default().fg(MUTED),
                ),
                Span::styled(dest, Style::default().fg(color)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "reloadxml will be triggered on destination after import.",
        Style::default().fg(MUTED),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press y or Enter to proceed, n or Esc to cancel.",
        Style::default().fg(ACCENT),
    )));

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
}

fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
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
            .block(
                Block::default()
                    .borders(Borders::NONE)
                    .title(Span::styled(" Current task ", Style::default().fg(MUTED))),
            ),
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

    f.render_widget(
        List::new(visible).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(" Log ", Style::default().fg(MUTED))),
        ),
        chunks[2],
    );
}

fn draw_done(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let lines = vec![
        Line::from(Span::styled(
            "✓ Transfer complete",
            Style::default().fg(OK).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Outbound routes have been imported and reloadxml triggered.",
            Style::default().fg(TITLE),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter or q to exit.",
            Style::default().fg(MUTED),
        )),
    ];

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(OK)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn draw_error(f: &mut Frame, msg: String, area: Rect) {
    let popup = centered_rect(60, 30, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(msg)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(ERR))
                    .title(Span::styled(
                        " Error ",
                        Style::default().fg(ERR).add_modifier(Modifier::BOLD),
                    )),
            )
            .style(Style::default().fg(ERR))
            .wrap(Wrap { trim: true }),
        popup,
    );
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
