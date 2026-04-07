use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use fpbx_tui_shared::{ServerInputs, VerifyStatus, draw_error, draw_progress, draw_server};

use super::app::{App, AppScreen};
use fpbx_core::version::{check_compat, VersionCompat};

const ACCENT: Color = Color::Magenta;
const MUTED: Color = Color::DarkGray;
const OK: Color = Color::Green;
const ERR: Color = Color::Red;
const TITLE: Color = Color::White;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_footer(f, app, chunks[2]);

    match app.screen.clone() {
        AppScreen::BundlePicker => draw_picker(f, app, chunks[1]),
        AppScreen::Preview => draw_preview(f, app, chunks[1]),
        AppScreen::Server => draw_server_screen(f, app, chunks[1]),
        AppScreen::Confirm => draw_confirm(f, app, chunks[1]),
        AppScreen::Progress => draw_progress(f, chunks[1], &app.worker, ACCENT),
        AppScreen::Done => draw_done(f, app, chunks[1]),
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
    f.render_widget(
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(MUTED))),
        area,
    );
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.screen {
        AppScreen::BundlePicker => " ↑↓/jk navigate   Space toggle   a select all   Enter continue   q quit",
        AppScreen::Preview => " Enter continue   Esc back",
        AppScreen::Server => " Tab switch field   Enter verify/continue   Esc back",
        AppScreen::Confirm => if app.confirm_field == 0 {
            " Tab/Enter focus confirm   Esc back"
        } else {
            " y/Enter confirm   Tab/e edit domain   n/Esc cancel"
        },
        AppScreen::Progress => " (restoring…)",
        AppScreen::Done => " Enter/q quit",
        AppScreen::Error(_) => " Esc dismiss",
    };
    f.render_widget(Paragraph::new(hints).style(Style::default().fg(MUTED)), area);
}

fn draw_picker(f: &mut Frame, app: &mut App, area: Rect) {
    let selected_count = app.selected_bundle_paths.len();
    let items: Vec<ListItem> = if app.bundles.is_empty() {
        vec![ListItem::new(Span::styled(
            format!(" No .fpbx bundles found in {}", app.bundle_dir.display()),
            Style::default().fg(MUTED),
        ))]
    } else {
        app.bundles
            .iter()
            .map(|(path, m)| {
                let checked = app.selected_bundle_paths.contains(path);
                let check_color = if checked { OK } else { MUTED };
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                let date = m.created_at.format("%Y-%m-%d %H:%M").to_string();
                ListItem::new(Line::from(vec![
                    Span::styled(if checked { "[✓] " } else { "[ ] " }, Style::default().fg(check_color)),
                    Span::styled(format!("{} ", m.domain.domain_name), Style::default().fg(TITLE).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("  {}  ", date), Style::default().fg(MUTED)),
                    Span::styled(name, Style::default().fg(MUTED)),
                ]))
            })
            .collect()
    };

    let title = if selected_count > 0 {
        format!(" Select bundles ({} selected) ", selected_count)
    } else {
        " Select backup bundle ".to_string()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED))
                .title(Span::styled(title, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
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

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(Style::default().fg(MUTED)))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
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
        host_label: " Destination host ",
        status,
        accent: ACCENT,
    });
}

fn draw_confirm(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .margin(4)
        .split(area);

    let single_bundle = app.selected_bundles().len() == 1
        || (app.selected_bundles().is_empty() && app.selected_manifest.is_some());
    let field_style = if single_bundle && app.confirm_field == 0 {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(MUTED)
    };
    let domain_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(field_style)
        .title(Span::styled(" Destination domain name ", field_style));
    let domain_display = if single_bundle { app.dest_domain_input.as_str() } else { "(multiple — rename not available)" };
    f.render_widget(
        Paragraph::new(domain_display).block(domain_block).style(Style::default().fg(TITLE)),
        chunks[0],
    );
    if single_bundle && app.confirm_field == 0 {
        f.set_cursor_position((chunks[0].x + 1 + app.dest_domain_input.len() as u16, chunks[0].y + 1));
    }

    let mut lines = vec![
        Line::from(Span::styled("Confirm restore", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];
    let selected = app.selected_bundles();
    if selected.len() == 1 {
        lines.push(Line::from(vec![
            Span::styled("Source domain: ", Style::default().fg(MUTED)),
            Span::styled(selected[0].1.domain.domain_name.clone(), Style::default().fg(TITLE)),
        ]));
    } else if !selected.is_empty() {
        lines.push(Line::from(vec![Span::styled(format!("Domains ({}):", selected.len()), Style::default().fg(MUTED))]));
        for (_, m) in &selected {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(MUTED)),
                Span::styled(m.domain.domain_name.clone(), Style::default().fg(TITLE)),
            ]));
        }
    } else if let Some(m) = &app.selected_manifest {
        lines.push(Line::from(vec![
            Span::styled("Source domain: ", Style::default().fg(MUTED)),
            Span::styled(m.domain.domain_name.clone(), Style::default().fg(TITLE)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("Destination:   ", Style::default().fg(MUTED)),
        Span::styled(format!("{}@{}", app.user_input, app.host_input), Style::default().fg(TITLE)),
    ]));
    lines.push(Line::from(""));

    let src_versions: Vec<_> = {
        let selected = app.selected_bundles();
        if !selected.is_empty() {
            selected.iter().filter_map(|(_, m)| m.source_version.clone()).collect()
        } else if let Some(m) = &app.selected_manifest {
            m.source_version.iter().cloned().collect()
        } else {
            vec![]
        }
    };
    if let Some(dst_v) = &app.dest_version {
        lines.push(Line::from(vec![
            Span::styled("Destination:   ", Style::default().fg(MUTED)),
            Span::styled(dst_v.label(), Style::default().fg(TITLE)),
        ]));
        if src_versions.is_empty() {
            lines.push(Line::from(Span::styled(
                "Source version unknown (old bundle) — column intersection will be applied",
                Style::default().fg(Color::Yellow),
            )));
        } else {
            for src_v in &src_versions {
                let compat = check_compat(src_v, dst_v);
                let (label, color) = match &compat {
                    VersionCompat::Identical => (compat.status_line(), OK),
                    VersionCompat::Compatible { .. } => (compat.status_line(), Color::Yellow),
                    VersionCompat::Unsupported { .. } => (compat.status_line(), ERR),
                };
                lines.push(Line::from(vec![
                    Span::styled("Source:        ", Style::default().fg(MUTED)),
                    Span::styled(src_v.label(), Style::default().fg(TITLE)),
                ]));
                lines.push(Line::from(Span::styled(label, Style::default().fg(color))));
            }
        }
    } else {
        lines.push(Line::from(Span::styled("Destination version not yet detected", Style::default().fg(MUTED))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "This will import database records and files into the destination server.",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled("The source server is NOT modified.", Style::default().fg(OK))));
    lines.push(Line::from(""));
    if app.confirm_field == 0 {
        lines.push(Line::from(Span::styled(
            "Edit domain name above, then press Enter/Tab to continue.",
            Style::default().fg(ACCENT),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Press y or Enter to proceed, Tab/e to edit domain, n/Esc to cancel.",
            Style::default().fg(ACCENT),
        )));
    }

    let border_color = if app.confirm_field == 1 { Color::Yellow } else { MUTED };
    f.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color)))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn draw_done(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(6), Constraint::Min(0)])
        .margin(4)
        .split(area);

    let n = app.selected_bundles().len().max(1);
    let heading = if n == 1 { "✓ Restore complete".to_string() } else { format!("✓ Restore complete ({} domains)", n) };
    let body = if n == 1 {
        "Domain has been restored to the destination server.".to_string()
    } else {
        format!("{} domains have been restored to the destination server.", n)
    };

    f.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(heading, Style::default().fg(OK).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled(body, Style::default().fg(TITLE))),
            Line::from(""),
            Line::from(Span::styled("Press Enter or q to exit.", Style::default().fg(MUTED))),
        ]))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(OK)))
        .wrap(Wrap { trim: false }),
        chunks[1],
    );
}
