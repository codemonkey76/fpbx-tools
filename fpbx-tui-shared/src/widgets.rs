use fpbx_core::WorkerSlot;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};

// ── Shared colors ───────────────────────────────────────────────────────────

const MUTED: Color = Color::DarkGray;
const OK: Color = Color::Green;
const ERR: Color = Color::Red;
const TITLE: Color = Color::White;

// ── Verify status ───────────────────────────────────────────────────────────

/// Represents the current state of an SSH + FusionPBX verification attempt.
pub enum VerifyStatus {
    Idle,
    InProgress,
    Ok(String),
    Err(String),
}

// ── Shared widgets ──────────────────────────────────────────────────────────

/// Center a rect within `r` using percentage constraints.
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

/// Render a floating error popup centred in `area`.
pub fn draw_error(f: &mut Frame, msg: String, area: Rect) {
    let popup = centered_rect(60, 30, area);
    f.render_widget(Clear, popup);
    let p = Paragraph::new(msg)
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
        .wrap(Wrap { trim: true });
    f.render_widget(p, popup);
}

/// Render the progress screen: current task label, gauge, and scrolling log.
/// `accent` controls the colour of the task text and gauge fill.
pub fn draw_progress(f: &mut Frame, area: Rect, worker: &Option<WorkerSlot>, accent: Color) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
        .margin(2)
        .split(area);

    let (log, progress, current_task) = if let Some(w) = worker {
        let w = w.lock().unwrap();
        (w.log.clone(), w.progress, w.current_task.clone())
    } else {
        (vec![], 0.0, String::new())
    };

    f.render_widget(
        Paragraph::new(current_task)
            .style(Style::default().fg(accent))
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
            .gauge_style(Style::default().fg(accent).bg(Color::DarkGray))
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

/// Arguments for [`draw_server`], bundled to keep the argument count low.
pub struct ServerInputs<'a> {
    pub host: &'a str,
    pub user: &'a str,
    pub active_field: usize,
    pub host_label: &'a str,
    pub status: VerifyStatus,
    pub accent: Color,
}

/// Render the server-connection screen (host + user inputs + verify status).
pub fn draw_server(f: &mut Frame, area: Rect, inputs: ServerInputs<'_>) {
    let ServerInputs { host, user, active_field, host_label, status, accent } = inputs;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .margin(4)
        .split(area);

    let host_style = if active_field == 0 {
        Style::default().fg(accent)
    } else {
        Style::default().fg(MUTED)
    };
    let user_style = if active_field == 1 {
        Style::default().fg(accent)
    } else {
        Style::default().fg(MUTED)
    };

    f.render_widget(
        Paragraph::new(host)
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
        Paragraph::new(user)
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

    let status_widget = match status {
        VerifyStatus::InProgress => Paragraph::new("⟳ Verifying SSH + FusionPBX access…")
            .style(Style::default().fg(Color::Yellow)),
        VerifyStatus::Ok(msg) => Paragraph::new(msg).style(Style::default().fg(OK)),
        VerifyStatus::Err(msg) => Paragraph::new(msg).style(Style::default().fg(ERR)),
        VerifyStatus::Idle => {
            Paragraph::new("Press Enter to verify").style(Style::default().fg(MUTED))
        }
    };
    f.render_widget(status_widget, chunks[4]);
}
