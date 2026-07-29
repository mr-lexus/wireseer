mod layout;
mod theme;

use std::{io, panic, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::{
    ExecutableCommand,
    cursor::Show,
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    style::force_color_output,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Bar, BarChart, BarGroup, Block, BorderType, Borders, Cell, Clear, Gauge, List, ListItem,
        ListState, Padding, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Sparkline, Table, TableState, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    BRAND_PULSE_ASCII, BRAND_PULSE_UNICODE, BRAND_TRACE_ASCII, BRAND_TRACE_UNICODE, TAGLINE,
    alerts::Severity,
    app::{
        AppRuntime, AppState, DeviceFilter, DeviceSort, LogLevel, Overlay, PersistenceEvent, Screen,
    },
    config::{IconMode, ScanMode, ThemeName},
    devices::{Device, DeviceStatus, DeviceType, Transport},
    history::TimelineEvent,
    network::HealthStatus,
};

pub use layout::LayoutClass;
use theme::{Icons, Theme};

pub trait PersistenceSink {
    fn submit(&mut self, event: PersistenceEvent);
    fn flush(&mut self) -> Result<()>;
}

pub struct NoopPersistence;

impl PersistenceSink for NoopPersistence {
    fn submit(&mut self, _event: PersistenceEvent) {}
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

pub async fn run(runtime: &mut AppRuntime, persistence: &mut dyn PersistenceSink) -> Result<()> {
    let _color_output_guard = ColorOutputGuard::enter(runtime.state.theme);
    let mut guard = TerminalGuard::enter(runtime.state.mouse)?;
    let mut events = EventStream::new();
    if runtime.auto_scan {
        runtime.start_scan().unwrap_or_else(|error| {
            runtime.state.error = Some(format!("{error:#}"));
            runtime.state.overlay = Overlay::Error;
        });
    }

    while !runtime.state.should_quit {
        for event in runtime.state.pending_persistence.drain(..) {
            persistence.submit(event);
        }
        sync_terminal_color_output(runtime.state.theme);
        guard.terminal.draw(|frame| render(frame, &runtime.state))?;
        let size = guard.terminal.size().context("read terminal size")?;
        let terminal_area = Rect::new(0, 0, size.width, size.height);
        let animated = runtime.state.animations && runtime.state.scan.active;
        let tick = tokio::time::sleep(if animated {
            Duration::from_millis(125)
        } else {
            Duration::from_secs(1)
        });
        tokio::pin!(tick);
        tokio::select! {
            () = &mut tick => {
                runtime.state.tick();
                if runtime.state.scan_mode == ScanMode::Watch
                    && !runtime.state.scan.active
                    && runtime.state.scan.finished_at.is_some_and(|finished| {
                        Utc::now().signed_duration_since(finished).num_seconds()
                            >= i64::try_from(runtime.config.refresh_interval_secs).unwrap_or(i64::MAX)
                    })
                {
                    if let Err(error) = runtime.start_scan() {
                        runtime.state.error = Some(format!("{error:#}"));
                        runtime.state.overlay = Overlay::Error;
                    }
                }
            },
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        let mouse_before = runtime.state.mouse;
                        handle_terminal_event(runtime, event, terminal_area);
                        if mouse_before != runtime.state.mouse {
                            guard.set_mouse_capture(runtime.state.mouse)?;
                        }
                    },
                    Some(Err(error)) => {
                        runtime.state.error = Some(format!("Terminal input failed: {error}"));
                        runtime.state.overlay = Overlay::Error;
                    }
                    None => runtime.state.should_quit = true,
                }
            }
            maybe_event = runtime.discovery_rx.recv() => {
                if let Some(event) = maybe_event {
                    if let Some(persist) = runtime.state.apply_discovery_event(event) {
                        persistence.submit(persist);
                    }
                }
            }
        }
    }
    runtime.shutdown().await;
    persistence.flush()?;
    guard.restore()?;
    Ok(())
}

fn theme_uses_color(theme: ThemeName) -> bool {
    theme != ThemeName::Monochrome
}

fn sync_terminal_color_output(theme: ThemeName) {
    force_color_output(theme_uses_color(theme));
}

struct ColorOutputGuard;

impl ColorOutputGuard {
    fn enter(theme: ThemeName) -> Self {
        sync_terminal_color_output(theme);
        Self
    }
}

impl Drop for ColorOutputGuard {
    fn drop(&mut self) {
        let no_color = std::env::var("NO_COLOR").is_ok_and(|value| !value.is_empty());
        force_color_output(!no_color);
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    restored: bool,
    mouse_enabled: bool,
}

impl TerminalGuard {
    fn enter(mouse: bool) -> Result<Self> {
        enable_raw_mode().context("enable terminal raw mode")?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        if mouse {
            stdout.execute(EnableMouseCapture)?;
        }
        let previous_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let mut stdout = io::stdout();
            let _ = stdout.execute(DisableMouseCapture);
            let _ = stdout.execute(LeaveAlternateScreen);
            let _ = stdout.execute(Show);
            previous_hook(info);
        }));
        let terminal = Terminal::new(CrosstermBackend::new(stdout)).context("create terminal")?;
        Ok(Self {
            terminal,
            restored: false,
            mouse_enabled: mouse,
        })
    }

    fn set_mouse_capture(&mut self, enabled: bool) -> Result<()> {
        if enabled == self.mouse_enabled {
            return Ok(());
        }
        if enabled {
            self.terminal.backend_mut().execute(EnableMouseCapture)?;
        } else {
            self.terminal.backend_mut().execute(DisableMouseCapture)?;
        }
        self.mouse_enabled = enabled;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        disable_raw_mode().context("disable terminal raw mode")?;
        self.terminal.backend_mut().execute(DisableMouseCapture)?;
        self.terminal.backend_mut().execute(LeaveAlternateScreen)?;
        self.terminal.backend_mut().execute(Show)?;
        self.terminal.show_cursor()?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    let theme = Theme::named(state.theme);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.background)),
        area,
    );
    let class = LayoutClass::from_area(area);
    if class == LayoutClass::TooSmall {
        render_too_small(frame, area, &theme, state.icons);
        return;
    }
    let root = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(area);
    render_header(frame, root[0], state, &theme, class);
    match state.screen {
        Screen::Dashboard => render_dashboard(frame, root[1], state, &theme, class),
        Screen::Devices => render_devices(frame, root[1], state, &theme, class),
        Screen::DeviceDetails => render_device_details(frame, root[1], state, &theme, class),
        Screen::History => render_history(frame, root[1], state, &theme, class),
        Screen::Compare => render_compare(frame, root[1], state, &theme, class),
        Screen::Alerts => render_alerts(frame, root[1], state, &theme),
        Screen::Logs => render_logs(frame, root[1], state, &theme),
        Screen::Settings => render_settings(frame, root[1], state, &theme, class),
    }
    render_footer(frame, root[2], state, &theme, class);
    render_overlay(frame, area, state, &theme);
    render_toast(frame, area, state, &theme);
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect, theme: &Theme, icon_mode: IconMode) {
    let inner = centered_rect(
        56.min(area.width.saturating_sub(2)),
        8.min(area.height),
        area,
    );
    let trace = brand_trace(icon_mode);
    let dimension_separator = if icon_mode == IconMode::Ascii {
        "x"
    } else {
        "×"
    };
    let text = Text::from(vec![
        brand_line(trace, theme),
        Line::from(Span::styled(TAGLINE, Style::default().fg(theme.text_muted))),
        Line::from(Span::styled(
            "Wireseer needs a little more room.",
            Style::default().fg(theme.text_primary).bold(),
        )),
        Line::from(format!(
            "Current terminal: {} {dimension_separator} {}",
            area.width, area.height
        )),
        Line::from(format!(
            "Minimum recommended size: 70 {dimension_separator} 18"
        )),
        Line::from(""),
        Line::from(Span::styled(
            "q  Quit",
            Style::default().fg(theme.text_secondary),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_secondary)),
        inner,
    );
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).split(area);
    let interface = state.active_interface.as_ref();
    let network = interface.map_or_else(
        || "No IPv4 network".to_string(),
        |item| format!("{} · {}", item.name, item.subnet),
    );
    let scan_style = if state.scan.active {
        Style::default().fg(theme.success).bold()
    } else {
        Style::default().fg(theme.text_muted)
    };
    let icons = Icons { mode: state.icons };
    let activity = if state.scan.active {
        format!("{} SCANNING", icons.spinner(state.tick))
    } else {
        "○ IDLE".into()
    };
    let page = if state.screen == Screen::Dashboard {
        String::new()
    } else {
        format!(" / {}", state.screen.title().to_uppercase())
    };
    let mut left = branded_name(theme);
    let trace = if class == LayoutClass::Compact {
        brand_pulse(state.icons)
    } else {
        brand_trace(state.icons)
    };
    left.push(Span::styled(
        format!("  {trace}"),
        Style::default().fg(theme.border_focused),
    ));
    left.push(Span::styled(
        page,
        Style::default().fg(theme.text_secondary).bold(),
    ));
    let left = Line::from(left);
    let right = if class == LayoutClass::Compact {
        Line::from(vec![
            Span::styled(
                state.scan_mode.label(),
                Style::default().fg(theme.text_muted),
            ),
            Span::raw("  "),
            Span::styled(activity, scan_style),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!("{network}  "),
                Style::default().fg(theme.text_secondary),
            ),
            Span::styled(
                format!("{}  ", state.scan_mode.label()),
                Style::default().fg(theme.text_muted),
            ),
            Span::styled(activity, scan_style),
            Span::raw(" "),
        ])
    };
    let widths = [Constraint::Percentage(40), Constraint::Percentage(60)];
    let columns = Layout::horizontal(widths).split(chunks[0]);
    frame.render_widget(
        Paragraph::new(left).block(Block::default().padding(Padding::top(1))),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .block(Block::default().padding(Padding::top(1))),
        columns[1],
    );
    frame.render_widget(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(theme.border_focused))
            .border_type(BorderType::Plain),
        chunks[1],
    );
}

fn branded_name(theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled(" WIRE", Style::default().fg(theme.text_primary).bold()),
        Span::styled("SEER", Style::default().fg(theme.accent).bold()),
    ]
}

fn brand_line(trace: &'static str, theme: &Theme) -> Line<'static> {
    let mut spans = branded_name(theme);
    spans.push(Span::styled(
        format!("  {trace}"),
        Style::default().fg(theme.border_focused),
    ));
    Line::from(spans)
}

const fn brand_trace(icon_mode: IconMode) -> &'static str {
    if matches!(icon_mode, IconMode::Ascii) {
        BRAND_TRACE_ASCII
    } else {
        BRAND_TRACE_UNICODE
    }
}

const fn brand_pulse(icon_mode: IconMode) -> &'static str {
    if matches!(icon_mode, IconMode::Ascii) {
        BRAND_PULSE_ASCII
    } else {
        BRAND_PULSE_UNICODE
    }
}

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);

    let active_screen = if state.screen == Screen::DeviceDetails {
        Screen::Devices
    } else {
        state.screen
    };
    let pages = [
        ('1', Screen::Dashboard, "Dashboard", "Home"),
        ('2', Screen::Devices, "Devices", "Dev"),
        ('3', Screen::History, "History", "Hist"),
        ('4', Screen::Compare, "Compare", "Diff"),
        ('5', Screen::Alerts, "Alerts", "Alert"),
        ('6', Screen::Logs, "Logs", "Logs"),
        ('7', Screen::Settings, "Settings", "Setup"),
    ];
    let mut navigation = vec![Span::raw(" ")];
    for (key, screen, label, compact_label) in pages {
        let active = screen == active_screen;
        navigation.push(Span::styled(
            key.to_string(),
            Style::default().fg(theme.accent).bold(),
        ));
        navigation.push(Span::styled(
            format!(
                " {}  ",
                if class == LayoutClass::Compact {
                    compact_label
                } else {
                    label
                }
            ),
            Style::default()
                .fg(if active {
                    theme.text_primary
                } else {
                    theme.text_secondary
                })
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(navigation)), rows[0]);

    let contextual: &[(&str, &str)] = match state.screen {
        Screen::Dashboard => &[],
        Screen::Devices => &[
            ("Enter", "Details"),
            ("Space", "Select"),
            ("/", "Search"),
            ("f", "Filter"),
            ("s", "Sort"),
            ("e", "Export"),
        ],
        Screen::DeviceDetails => &[
            ("j/k", "Device"),
            ("Esc", "Back"),
            ("e", "Identity"),
            ("t", "Tags"),
            ("n", "Notes"),
        ],
        Screen::History | Screen::Compare | Screen::Logs => &[("e", "Export")],
        Screen::Alerts => &[("j/k", "Navigate"), ("a", "Acknowledge"), ("e", "Export")],
        Screen::Settings => &[("j/k", "Navigate"), ("Enter", "Change"), ("c", "Density")],
    };
    let status = match (class, state.screen) {
        (LayoutClass::Compact, _) => String::new(),
        (LayoutClass::Wide, Screen::Devices) => format!(
            "{} devices · filter {} · sort {} ",
            state.filtered_indices().len(),
            state.filter.label(),
            state.sort.label()
        ),
        (LayoutClass::Standard, Screen::Devices) => format!(
            "{} · {} · {} ",
            state.filtered_indices().len(),
            state.filter.label(),
            state.sort.label()
        ),
        (LayoutClass::Wide, _) => format!(
            "{} devices · {} alerts ",
            state.devices.len(),
            state
                .alerts
                .iter()
                .filter(|alert| alert.resolved_at.is_none())
                .count()
        ),
        (LayoutClass::Standard, _) => format!(
            "{} dev · {} alerts ",
            state.devices.len(),
            state
                .alerts
                .iter()
                .filter(|alert| alert.resolved_at.is_none())
                .count()
        ),
        (LayoutClass::TooSmall, _) => String::new(),
    };
    let status_width = u16::try_from(UnicodeWidthStr::width(status.as_str()))
        .unwrap_or(u16::MAX)
        .min(area.width);
    let columns =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(status_width)]).split(rows[1]);

    let mut actions = vec![("q", "Quit"), ("?", "Help"), ("Ctrl+P", "Commands")];
    actions.push(if state.scan.active {
        ("x", "Cancel scan")
    } else {
        ("r", "Scan")
    });
    let contextual_limit = if class == LayoutClass::Compact {
        1
    } else {
        contextual.len()
    };
    actions.extend(contextual.iter().take(contextual_limit).copied());

    let available = usize::from(columns[0].width);
    let mut used = 1_usize;
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in actions {
        let width = UnicodeWidthStr::width(key)
            .saturating_add(UnicodeWidthStr::width(label))
            .saturating_add(4);
        if used.saturating_add(width) > available {
            continue;
        }
        spans.push(Span::styled(key, Style::default().fg(theme.accent).bold()));
        spans.push(Span::styled(
            format!(" {label}   "),
            Style::default().fg(theme.text_secondary),
        ));
        used = used.saturating_add(width);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), columns[0]);
    if columns[1].width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(status, Style::default().fg(theme.text_muted)))
                .alignment(Alignment::Right),
            columns[1],
        );
    }
}

fn render_dashboard(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    if state.devices.is_empty() && !state.scan.active {
        render_empty(
            frame,
            area,
            theme,
            "NO DEVICES DISCOVERED YET",
            "No positive observations are stored.",
            "Check the selected interface or press r to rescan.",
        );
        return;
    }
    let upper_height = match class {
        LayoutClass::Wide => 10,
        LayoutClass::Standard => 11,
        _ => 12,
    };
    let rows = Layout::vertical([Constraint::Length(upper_height), Constraint::Min(6)]).split(
        area.inner(Margin {
            horizontal: 1,
            vertical: 0,
        }),
    );
    match class {
        LayoutClass::Wide => {
            let cards = Layout::horizontal([
                Constraint::Percentage(31),
                Constraint::Percentage(43),
                Constraint::Percentage(26),
            ])
            .split(rows[0]);
            render_network_card(frame, cards[0], state, theme);
            render_activity_card(frame, cards[1], state, theme);
            render_health_card(frame, cards[2], state, theme);
        }
        LayoutClass::Standard => {
            let cards =
                Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .split(rows[0]);
            render_network_card(frame, cards[0], state, theme);
            render_activity_card(frame, cards[1], state, theme);
        }
        LayoutClass::Compact => render_compact_dashboard(frame, rows[0], state, theme),
        LayoutClass::TooSmall => {}
    }
    let lower = if class == LayoutClass::Wide {
        Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)]).split(rows[1])
    } else {
        Layout::horizontal([Constraint::Percentage(100), Constraint::Length(0)]).split(rows[1])
    };
    render_recent_events(frame, lower[0], state, theme);
    if lower[1].width > 0 {
        render_common_services(frame, lower[1], state, theme);
    }
}

fn section_block<'a>(title: &'a str, theme: &Theme, focused: bool) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(if focused {
                    theme.accent
                } else {
                    theme.text_muted
                })
                .bold(),
        ))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(if focused {
            theme.border_focused
        } else {
            theme.border
        }))
        .padding(Padding::horizontal(1))
}

fn render_network_card(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let counts = state.counts();
    let lines = vec![
        metric_line("Online", counts.online, theme.success, theme),
        metric_line("Offline", counts.offline, theme.text_muted, theme),
        metric_line("Unknown", counts.unknown, theme.warning, theme),
        metric_line("Changed", counts.changed, theme.info, theme),
    ];
    let block = section_block("NETWORK", theme, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let content = Layout::vertical([Constraint::Min(4), Constraint::Length(2)]).split(inner);
    frame.render_widget(Paragraph::new(lines), content[0]);
    render_scan_gauge(frame, content[1], state, theme);
}

fn metric_line(label: &str, value: usize, color: Color, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{value:>3}"), Style::default().fg(color).bold()),
        Span::styled(
            format!("  {label}"),
            Style::default().fg(theme.text_secondary),
        ),
    ])
}

fn render_scan_gauge(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let ratio = state
        .scan
        .progress
        .as_ref()
        .map_or(0.0, |progress| progress.ratio());
    let label = state.scan.progress.as_ref().map_or_else(
        || "Scan idle".to_string(),
        |progress| format!("{} {:>3}%", progress.phase.label(), (ratio * 100.0) as u8),
    );
    frame.render_widget(
        Gauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label)
            .gauge_style(Style::default().fg(theme.accent).bg(theme.surface_active))
            .use_unicode(state.icons != IconMode::Ascii),
        area,
    );
}

fn render_activity_card(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let block = section_block("DEVICE ACTIVITY", theme, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(2)]).split(inner);
    let activity = state.activity.iter().copied().collect::<Vec<_>>();
    frame.render_widget(
        Sparkline::default()
            .data(&activity)
            .style(Style::default().fg(theme.chart_primary))
            .bar_set(symbols::bar::NINE_LEVELS),
        rows[0],
    );
    let current = state.scan.progress.as_ref();
    let text = vec![
        Line::from(vec![
            Span::styled("Discovery  ", Style::default().fg(theme.text_muted)),
            Span::styled("TCP · local", Style::default().fg(theme.text_secondary)),
        ]),
        Line::from(vec![
            Span::styled("Active     ", Style::default().fg(theme.text_muted)),
            Span::styled(
                format!(
                    "{:>3} probes · {:>3} found",
                    current.map_or(0, |p| p.active),
                    current.map_or(state.devices.len(), |p| p.devices_found)
                ),
                Style::default().fg(theme.text_primary),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(text), rows[1]);
}

fn render_health_card(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let block = section_block("HEALTH", theme, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let gateway = state.devices.iter().any(|device| {
        matches!(device.device_type, DeviceType::Gateway | DeviceType::Router)
            && device.status == DeviceStatus::Online
    });
    let warnings = state
        .alerts
        .iter()
        .filter(|alert| alert.resolved_at.is_none() && alert.severity != Severity::Info)
        .count()
        + state.baseline_diff.unknown.len()
        + state.baseline_diff.missing.len();
    let progress = state.scan.progress.as_ref();
    let lines = vec![
        label_value(
            "Gateway",
            if gateway { "ONLINE" } else { "UNKNOWN" },
            if gateway {
                theme.success
            } else {
                theme.warning
            },
            theme,
        ),
        label_value(
            "Discovery",
            &format!(
                "{}",
                state
                    .provider_health
                    .iter()
                    .filter(|health| health.status == crate::network::HealthStatus::Available)
                    .count()
            ),
            theme.info,
            theme,
        ),
        label_value(
            "Warnings",
            &warnings.to_string(),
            if warnings > 0 {
                theme.warning
            } else {
                theme.success
            },
            theme,
        ),
        label_value(
            "Scan queue",
            &progress
                .map_or(0, |item| item.total.saturating_sub(item.completed))
                .to_string(),
            theme.text_primary,
            theme,
        ),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn label_value(label: &str, value: &str, color: Color, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<12}"),
            Style::default().fg(theme.text_secondary),
        ),
        Span::styled(value.to_string(), Style::default().fg(color).bold()),
    ])
}

fn render_compact_dashboard(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let counts = state.counts();
    let block = section_block("NETWORK OVERVIEW", theme, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(2),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:>3}", counts.online),
                Style::default().fg(theme.success).bold(),
            ),
            Span::styled(" online    ", Style::default().fg(theme.text_secondary)),
            Span::styled(
                format!("{:>3}", counts.offline),
                Style::default().fg(theme.text_muted).bold(),
            ),
            Span::styled(" offline", Style::default().fg(theme.text_secondary)),
        ])),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:>3}", counts.unknown),
                Style::default().fg(theme.warning).bold(),
            ),
            Span::styled(" unknown   ", Style::default().fg(theme.text_secondary)),
            Span::styled(
                format!("{:>3}", counts.changed),
                Style::default().fg(theme.info).bold(),
            ),
            Span::styled(" changed", Style::default().fg(theme.text_secondary)),
        ])),
        rows[1],
    );
    render_scan_gauge(frame, rows[2], state, theme);
}

fn render_recent_events(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let block = section_block("RECENT EVENTS", theme, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if state.events.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "No network history yet",
                    Style::default().fg(theme.text_secondary).bold(),
                )),
                Line::from(Span::styled(
                    "Changes appear after positive observations.",
                    Style::default().fg(theme.text_muted),
                )),
            ])
            .block(Block::default().padding(Padding::top(1))),
            inner,
        );
        return;
    }
    let lines = state
        .events
        .iter()
        .take(inner.height as usize)
        .map(|event| event_line(event, theme))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn event_line(event: &TimelineEvent, theme: &Theme) -> Line<'static> {
    let color = severity_color(event.severity, theme);
    Line::from(vec![
        Span::styled(
            event.occurred_at.format("%H:%M").to_string(),
            Style::default().fg(theme.text_muted),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:<9}", event.kind.label()),
            Style::default().fg(color).bold(),
        ),
        Span::styled(
            event.summary.clone(),
            Style::default().fg(theme.text_primary),
        ),
        Span::styled(
            format!("  {}", event.detail),
            Style::default().fg(theme.text_muted),
        ),
    ])
}

fn render_common_services(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let block = section_block("COMMON SERVICES", theme, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut totals = std::collections::BTreeMap::<String, u64>::new();
    for service in state
        .devices
        .iter()
        .flat_map(|device| device.services.values())
    {
        *totals.entry(service.name.clone()).or_default() += 1;
    }
    let mut items: Vec<_> = totals.into_iter().collect();
    items.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let bars: Vec<Bar<'_>> = items
        .iter()
        .take(6)
        .map(|(name, count)| {
            Bar::default()
                .value(*count)
                .label(Line::from(name.as_str()))
                .style(Style::default().fg(theme.chart_secondary))
                .value_style(Style::default().fg(theme.text_primary).bold())
        })
        .collect();
    frame.render_widget(
        BarChart::default()
            .data(BarGroup::default().bars(&bars))
            .bar_width(5)
            .bar_gap(2)
            .direction(Direction::Vertical),
        inner,
    );
}

fn split_device_workspace(area: Rect, class: LayoutClass) -> (Rect, Option<Rect>) {
    if matches!(class, LayoutClass::Compact | LayoutClass::TooSmall) || area.height < 16 {
        return (area, None);
    }

    // Keep the inventory usable before showing the secondary inspector. On
    // very wide terminals the inspector is capped so it cannot consume an
    // ever-growing percentage of the workspace.
    const MIN_TABLE_WIDTH: u16 = 112;
    const MIN_INSPECTOR_WIDTH: u16 = 38;
    const MAX_INSPECTOR_WIDTH: u16 = 64;
    let inspector_width = (area.width / 3).clamp(MIN_INSPECTOR_WIDTH, MAX_INSPECTOR_WIDTH);
    if area.width < MIN_TABLE_WIDTH.saturating_add(inspector_width) {
        return (area, None);
    }

    let columns = Layout::horizontal([
        Constraint::Min(MIN_TABLE_WIDTH),
        Constraint::Length(inspector_width),
    ])
    .split(area);
    (columns[0], Some(columns[1]))
}

fn render_devices(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    let body = area.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(body);
    let query = if state.search.is_empty() {
        "Search by name, IP, vendor, or service…".into()
    } else {
        state.search.clone()
    };
    let search_style = if state.overlay == Overlay::Search {
        Style::default()
            .fg(theme.text_primary)
            .bg(theme.surface_active)
    } else {
        Style::default().fg(theme.text_muted)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" / ", Style::default().fg(theme.accent).bold()),
            Span::styled(query, search_style),
        ]))
        .block(Block::default().padding(Padding::top(1))),
        rows[0],
    );
    if state.filtered_indices().is_empty() {
        let (title, body, hint) = if state.devices.is_empty() {
            (
                "NO DEVICES DISCOVERED YET",
                "The current scan is still looking for positive responses.",
                "Check the interface or press r to rescan.",
            )
        } else {
            (
                "NO MATCHING DEVICES",
                "No devices match the current search and filters.",
                "Clear the search or press f to change the filter.",
            )
        };
        render_empty(frame, rows[1], theme, title, body, hint);
        return;
    }
    let (table, inspector) = split_device_workspace(rows[1], class);
    render_device_table(frame, table, state, theme, class);
    if let Some(inspector) = inspector {
        render_inspector(frame, inspector, state, theme);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceTableLayout {
    show_vendor: bool,
    show_latency: bool,
    show_services: bool,
    show_confidence: bool,
    identity_width: u16,
    address_width: u16,
    vendor_width: u16,
    latency_width: u16,
    services_width: u16,
    confidence_width: u16,
}

fn device_table_layout(area_width: u16, state: &AppState) -> DeviceTableLayout {
    let visible = |name: &str| state.visible_columns.iter().any(|column| column == name);
    let show_latency = visible("latency") && area_width >= 58;
    let show_services = visible("services") && area_width >= 86;
    let show_vendor = visible("vendor") && area_width >= 112;
    let show_confidence = visible("confidence") && area_width >= 136;

    let mut identity_width = 24_u16;
    let mut address_width = if area_width >= 88 { 20 } else { 15 };
    let mut vendor_width = if show_vendor { 18 } else { 0 };
    let latency_width = if show_latency { 9 } else { 0 };
    let mut services_width = if show_services { 14 } else { 0 };
    let confidence_width = if show_confidence { 7 } else { 0 };

    let column_count = 3_u16
        + u16::from(show_vendor)
        + u16::from(show_latency)
        + u16::from(show_services)
        + u16::from(show_confidence);
    let spacing = column_count.saturating_sub(1);
    let chrome = 4_u16;
    let fixed = 2_u16
        .saturating_add(latency_width)
        .saturating_add(confidence_width);
    let flexible_base = identity_width
        .saturating_add(address_width)
        .saturating_add(vendor_width)
        .saturating_add(services_width);
    let extra = area_width.saturating_sub(
        chrome
            .saturating_add(spacing)
            .saturating_add(fixed)
            .saturating_add(flexible_base),
    );
    let total_weight = 6_u32 + if show_vendor { 3 } else { 0 } + if show_services { 3 } else { 0 };
    let share =
        |weight: u32| u16::try_from((u32::from(extra) * weight) / total_weight).unwrap_or(u16::MAX);
    let identity_extra = share(4);
    let address_extra = share(2);
    let vendor_extra = if show_vendor { share(3) } else { 0 };
    let services_extra = if show_services { share(3) } else { 0 };
    identity_width = identity_width.saturating_add(identity_extra);
    address_width = address_width.saturating_add(address_extra);
    vendor_width = vendor_width.saturating_add(vendor_extra);
    services_width = services_width.saturating_add(services_extra);
    let assigned = identity_extra
        .saturating_add(address_extra)
        .saturating_add(vendor_extra)
        .saturating_add(services_extra);
    let remainder = extra.saturating_sub(assigned);
    if show_services {
        services_width = services_width.saturating_add(remainder);
    } else if show_vendor {
        vendor_width = vendor_width.saturating_add(remainder);
    } else {
        identity_width = identity_width.saturating_add(remainder);
    }

    DeviceTableLayout {
        show_vendor,
        show_latency,
        show_services,
        show_confidence,
        identity_width,
        address_width,
        vendor_width,
        latency_width,
        services_width,
        confidence_width,
    }
}

fn render_device_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    _class: LayoutClass,
) {
    let indices = state.filtered_indices();
    let layout = device_table_layout(area.width, state);
    let show_vendor = layout.show_vendor;
    let show_latency = layout.show_latency;
    let show_services = layout.show_services;
    let show_confidence = layout.show_confidence;
    let identity_width = layout.identity_width;
    let identity_name_width = usize::from(identity_width.saturating_sub(6));
    let vendor_width = layout.vendor_width;
    let service_limit = usize::from((layout.services_width / 8).clamp(3, 10));
    let source_limit = usize::from((vendor_width / 7).clamp(3, 8));
    let rows = indices.iter().map(|index| {
        let device = &state.devices[*index];
        let selected = state.selected_ids.contains(&device.id);
        let status = device_status(device, state.icons, theme);
        let icon = device_icon(device.device_type, state.icons);
        let service_names = device
            .services
            .values()
            .take(service_limit)
            .map(|service| service.name.clone())
            .collect::<Vec<_>>();
        let services = if service_names.is_empty() {
            "—".to_string()
        } else {
            service_names.join(" ")
        };
        let service_endpoints = if device.services.is_empty() {
            "No services".to_string()
        } else {
            device
                .services
                .values()
                .take(service_limit)
                .map(|service| {
                    let transport = match service.transport {
                        Transport::Tcp => "tcp",
                        Transport::Udp => "udp",
                    };
                    format!("{}/{transport}", service.port)
                })
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let latency = device.latency_ms.map_or_else(
            || "—".into(),
            |value| {
                if value == 0 {
                    "<1ms".into()
                } else {
                    format!("{value}ms")
                }
            },
        );
        let marker = if selected {
            "◆"
        } else if device.baseline_unknown {
            "!"
        } else {
            " "
        };
        let status_detail = if device.baseline_unknown {
            "!"
        } else if device.changed {
            "~"
        } else {
            ""
        };
        let status_cell = if state.compact_rows {
            Cell::from(status)
        } else {
            Cell::from(Text::from(vec![
                status,
                Line::from(Span::styled(
                    status_detail,
                    Style::default().fg(theme.text_muted),
                )),
            ]))
        };
        let type_and_platform = device.platform.as_ref().map_or_else(
            || device.device_type.to_string(),
            |platform| format!("{} · {platform}", device.device_type),
        );
        let identity_detail = if device.identity_is_user_confirmed() {
            format!("{type_and_platform} · confirmed")
        } else if show_confidence {
            type_and_platform
        } else {
            format!("{type_and_platform} · {}%", device.confidence)
        };
        let identity = format!(
            "{marker} {icon} {}",
            truncate_cell(&device.display_name(), identity_name_width)
        );
        let mut cells = vec![
            status_cell,
            device_table_cell(
                identity,
                truncate_cell(&identity_detail, usize::from(identity_width)),
                state.compact_rows,
                theme,
            ),
            device_table_cell(
                device.ipv4.to_string(),
                truncate_cell(
                    &device_address_detail(device),
                    usize::from(layout.address_width),
                ),
                state.compact_rows,
                theme,
            ),
        ];
        if show_vendor {
            let sources = device
                .sources
                .iter()
                .take(source_limit)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" · ");
            cells.push(device_table_cell(
                device.manufacturer_name().map_or_else(
                    || "—".into(),
                    |vendor| truncate_cell(&vendor, usize::from(vendor_width)),
                ),
                truncate_cell(
                    if sources.is_empty() {
                        "No source"
                    } else {
                        &sources
                    },
                    usize::from(vendor_width),
                ),
                state.compact_rows,
                theme,
            ));
        }
        if show_latency {
            cells.push(device_table_cell(
                latency,
                relative_time(device.last_seen),
                state.compact_rows,
                theme,
            ));
        }
        if show_services {
            cells.push(device_table_cell(
                truncate_cell(&services, usize::from(layout.services_width)),
                truncate_cell(&service_endpoints, usize::from(layout.services_width)),
                state.compact_rows,
                theme,
            ));
        }
        if show_confidence {
            cells.push(device_table_cell(
                format!("{}%", device.confidence),
                if device.changed { "changed" } else { "stable" }.into(),
                state.compact_rows,
                theme,
            ));
        }
        Row::new(cells).height(if state.compact_rows { 1 } else { 2 })
    });
    let mut headers = vec!["", "IDENTITY", "ADDRESS"];
    let mut widths = vec![
        Constraint::Length(2),
        Constraint::Length(layout.identity_width),
        Constraint::Length(layout.address_width),
    ];
    if show_vendor {
        headers.push("VENDOR");
        widths.push(Constraint::Length(layout.vendor_width));
    }
    if show_latency {
        headers.push("LATENCY");
        widths.push(Constraint::Length(layout.latency_width));
    }
    if show_services {
        headers.push("SERVICES");
        widths.push(Constraint::Length(layout.services_width));
    }
    if show_confidence {
        headers.push("CONF.");
        widths.push(Constraint::Length(layout.confidence_width));
    }
    let header = Row::new(headers)
        .style(Style::default().fg(theme.text_muted).bold())
        .height(2);
    let mut table_state = TableState::default().with_selected(Some(state.selected));
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(theme.selection)
                .fg(theme.selection_text)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if state.icons == IconMode::Ascii {
            ">"
        } else {
            "▌"
        })
        .block(section_block("DEVICES", theme, true))
        .column_spacing(1);
    frame.render_stateful_widget(table, area, &mut table_state);
    let content_height =
        area.height.saturating_sub(3) as usize / if state.compact_rows { 1 } else { 2 };
    if indices.len() > content_height && content_height > 0 {
        let mut scrollbar_state = ScrollbarState::new(indices.len()).position(state.selected);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(theme.accent))
                .track_style(Style::default().fg(theme.border)),
            area.inner(Margin {
                horizontal: 0,
                vertical: 2,
            }),
            &mut scrollbar_state,
        );
    }
}

fn device_table_cell(
    primary: String,
    secondary: String,
    compact: bool,
    theme: &Theme,
) -> Cell<'static> {
    if compact {
        Cell::from(primary)
    } else {
        Cell::from(Text::from(vec![
            Line::from(primary),
            Line::from(Span::styled(
                secondary,
                Style::default().fg(theme.text_muted),
            )),
        ]))
    }
}

fn device_address_detail(device: &Device) -> String {
    device.mac.as_deref().map_or_else(
        || format!("via {}", device.interface),
        |mac| format!("MAC {mac}"),
    )
}

fn device_status(device: &Device, mode: IconMode, theme: &Theme) -> Line<'static> {
    let icons = Icons { mode };
    let (glyph, color) = match device.status {
        DeviceStatus::Online => (icons.status_online(), theme.success),
        DeviceStatus::Offline | DeviceStatus::Stale => (icons.status_offline(), theme.text_muted),
        DeviceStatus::Unknown | DeviceStatus::Scanning => (icons.status_unknown(), theme.warning),
    };
    Line::from(Span::styled(glyph, Style::default().fg(color).bold()))
}

fn device_icon(kind: DeviceType, mode: IconMode) -> &'static str {
    match mode {
        IconMode::Ascii => kind.ascii_icon(),
        IconMode::Nerd => match kind {
            DeviceType::Router | DeviceType::Gateway => "󰖩",
            DeviceType::Computer | DeviceType::VirtualMachine | DeviceType::ContainerHost => "󰌢",
            DeviceType::Phone | DeviceType::Tablet => "󰏲",
            DeviceType::SmartTv => "󰍹",
            DeviceType::Printer => "󰐿",
            DeviceType::Nas | DeviceType::Server => "󰒋",
            DeviceType::Camera => "󰄀",
            DeviceType::GameConsole => "󰊴",
            DeviceType::SmartHome => "󰟐",
            DeviceType::Unknown => "󰖟",
        },
        IconMode::Unicode => match kind {
            DeviceType::Router | DeviceType::Gateway => "⌁",
            DeviceType::Computer | DeviceType::VirtualMachine | DeviceType::ContainerHost => "▣",
            DeviceType::Phone | DeviceType::Tablet => "▯",
            DeviceType::SmartTv => "▤",
            DeviceType::Printer => "▧",
            DeviceType::Nas | DeviceType::Server => "▦",
            DeviceType::Camera => "◉",
            DeviceType::GameConsole => "◈",
            DeviceType::SmartHome => "⌂",
            DeviceType::Unknown => "?",
        },
    }
}

fn truncate_cell(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }

    let target = max_width.saturating_sub(1);
    let mut result = String::new();
    let mut width = 0;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > target {
            break;
        }
        result.push(character);
        width += character_width;
    }
    result.push('…');
    result
}

fn fit_cell(value: &str, width: usize) -> String {
    let value = truncate_cell(value, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    format!("{value}{}", " ".repeat(padding))
}

fn render_inspector(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let Some(device) = state.selected_device() else {
        return;
    };
    let title = device.display_name().to_uppercase();
    let block = section_block(&title, theme, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let services = device
        .services
        .values()
        .map(|service| service.name.clone())
        .collect::<Vec<_>>()
        .join("  ");
    let type_and_platform = device.platform.as_ref().map_or_else(
        || device.device_type.to_string(),
        |platform| format!("{} · {platform}", device.device_type),
    );
    let identity_origin = if device.identity_is_user_confirmed() {
        "User confirmed"
    } else {
        "Automatic"
    };
    let confidence = if device.identity_is_user_confirmed() {
        format!("  confirmed · auto type {}%", device.confidence)
    } else {
        format!("  {}% confidence", device.confidence)
    };
    let mac = device.mac.as_deref().map_or_else(
        || "Not observed".into(),
        |mac| {
            if device.uses_private_mac() {
                format!("{mac} · private")
            } else {
                mac.to_string()
            }
        },
    );
    let manufacturer = device
        .manufacturer_name()
        .unwrap_or_else(|| "Unknown".into());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("~ {type_and_platform}"),
                Style::default().fg(theme.accent).bold(),
            ),
            Span::styled(confidence, Style::default().fg(theme.text_secondary)),
        ]),
        Line::from(""),
        inspector_line("Identity", identity_origin, theme),
        inspector_line(
            "Family",
            device.inferred_model.as_deref().unwrap_or("Not inferred"),
            theme,
        ),
        inspector_line("IP", &device.ipv4.to_string(), theme),
        inspector_line("MAC", &mac, theme),
        inspector_line("Maker", &manufacturer, theme),
        inspector_line(
            "Hostname",
            device.hostname.as_deref().unwrap_or("Not observed"),
            theme,
        ),
        inspector_line("Seen", &relative_time(device.last_seen), theme),
        Line::from(""),
        Line::from(Span::styled(
            "SERVICES",
            Style::default().fg(theme.text_muted).bold(),
        )),
        Line::from(Span::styled(
            if services.is_empty() {
                "No open services observed"
            } else {
                &services
            },
            Style::default().fg(if services.is_empty() {
                theme.text_muted
            } else {
                theme.info
            }),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "EVIDENCE",
            Style::default().fg(theme.text_muted).bold(),
        )),
    ];
    for evidence in device.evidence.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled("+ ", Style::default().fg(theme.success)),
            Span::styled(
                evidence.description.clone(),
                Style::default().fg(theme.text_secondary),
            ),
            Span::styled(
                format!(" +{}", evidence.weight),
                Style::default().fg(theme.accent),
            ),
        ]));
    }
    if device.evidence.is_empty() {
        lines.push(Line::from(Span::styled(
            "No strong fingerprint evidence yet",
            Style::default().fg(theme.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn inspector_line(label: &str, value: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<10}"),
            Style::default().fg(theme.text_muted),
        ),
        Span::styled(value.to_string(), Style::default().fg(theme.text_primary)),
    ])
}

fn render_device_details(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    let Some(device) = state.selected_device().or_else(|| state.devices.first()) else {
        render_empty(
            frame,
            area,
            theme,
            "DEVICE NOT AVAILABLE",
            "The selected device is no longer in this view.",
            "Press Esc to return to devices.",
        );
        return;
    };
    let body = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let title_height = 4;
    let sections =
        Layout::vertical([Constraint::Length(title_height), Constraint::Min(6)]).split(body);
    let status_color = if device.status == DeviceStatus::Online {
        theme.success
    } else {
        theme.text_muted
    };
    let type_and_platform = device.platform.as_ref().map_or_else(
        || device.device_type.to_string(),
        |platform| format!("{} · {platform}", device.device_type),
    );
    let identity_status = if device.identity_is_user_confirmed() {
        "user confirmed".to_string()
    } else {
        format!("{}% confidence", device.confidence)
    };
    let mac = device.mac.as_deref().map_or_else(
        || "Not observed".into(),
        |mac| {
            if device.uses_private_mac() {
                format!("{mac} · private/randomized")
            } else {
                mac.to_string()
            }
        },
    );
    let manufacturer = device
        .manufacturer_name()
        .unwrap_or_else(|| "Unknown".into());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    device.display_name(),
                    Style::default().fg(theme.text_primary).bold(),
                ),
                Span::styled(
                    format!("  {}", device.status.to_string().to_uppercase()),
                    Style::default().fg(status_color).bold(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("~ {type_and_platform}"),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    format!(
                        " · {identity_status} · seen {}",
                        relative_time(device.last_seen)
                    ),
                    Style::default().fg(theme.text_secondary),
                ),
            ]),
        ]),
        sections[0],
    );
    let columns = match class {
        LayoutClass::Wide => Layout::horizontal([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(sections[1]),
        LayoutClass::Standard => Layout::horizontal([
            Constraint::Percentage(48),
            Constraint::Percentage(52),
            Constraint::Length(0),
        ])
        .split(sections[1]),
        _ => Layout::horizontal([
            Constraint::Percentage(100),
            Constraint::Length(0),
            Constraint::Length(0),
        ])
        .split(sections[1]),
    };
    let overview = vec![
        inspector_line(
            "Identity",
            if device.identity_is_user_confirmed() {
                "User confirmed"
            } else {
                "Automatic"
            },
            theme,
        ),
        inspector_line(
            "Family",
            device.inferred_model.as_deref().unwrap_or("Not inferred"),
            theme,
        ),
        inspector_line("IPv4", &device.ipv4.to_string(), theme),
        inspector_line("MAC", &mac, theme),
        inspector_line("Maker", &manufacturer, theme),
        inspector_line(
            "Registry",
            device.vendor.as_deref().unwrap_or("No IEEE match"),
            theme,
        ),
        inspector_line(
            "Hostname",
            device.hostname.as_deref().unwrap_or("Not observed"),
            theme,
        ),
        inspector_line("Interface", &device.interface, theme),
        inspector_line("Subnet", &device.subnet, theme),
        inspector_line("First seen", &relative_time(device.first_seen), theme),
        Line::from(""),
        Line::from(Span::styled(
            "USER METADATA",
            Style::default().fg(theme.text_muted).bold(),
        )),
        inspector_line(
            "Confirmed",
            device.user.name.as_deref().unwrap_or("Not set"),
            theme,
        ),
        inspector_line(
            "Tags",
            &device
                .user
                .tags
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            theme,
        ),
    ];
    frame.render_widget(
        Paragraph::new(overview)
            .block(section_block("OVERVIEW · OBSERVED + USER", theme, true))
            .wrap(Wrap { trim: true }),
        columns[0],
    );
    if columns[1].width > 0 {
        let service_lines = device
            .services
            .values()
            .map(|service| {
                let metadata = device
                    .http
                    .get(&service.port)
                    .map_or_else(String::new, |http| {
                        let mut parts = Vec::new();
                        if let Some(status) = http.status {
                            parts.push(status.to_string());
                        }
                        if let Some(title) = &http.title {
                            parts.push(title.clone());
                        }
                        if parts.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", parts.join(" · "))
                        }
                    });
                Line::from(vec![
                    Span::styled(
                        format!("{:>5}", service.port),
                        Style::default().fg(theme.accent).bold(),
                    ),
                    Span::styled(
                        format!("  {:<12}", service.name),
                        Style::default().fg(theme.text_primary),
                    ),
                    Span::styled(
                        format!(
                            "open · {}{}",
                            service
                                .sources
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("+"),
                            metadata,
                        ),
                        Style::default().fg(theme.text_muted),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(if service_lines.is_empty() {
                vec![Line::from(Span::styled(
                    "No open services observed",
                    Style::default().fg(theme.text_muted),
                ))]
            } else {
                service_lines
            })
            .block(section_block("SERVICES · OBSERVED", theme, false)),
            columns[1],
        );
    }
    if columns[2].width > 0 {
        let evidence = device
            .evidence
            .iter()
            .map(|item| {
                Line::from(vec![
                    Span::styled("+ ", Style::default().fg(theme.success)),
                    Span::styled(
                        item.description.clone(),
                        Style::default().fg(theme.text_secondary),
                    ),
                    Span::styled(
                        format!(" +{}", item.weight),
                        Style::default().fg(theme.accent).bold(),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(if evidence.is_empty() {
                vec![Line::from(Span::styled(
                    "Insufficient evidence to infer a type.",
                    Style::default().fg(theme.text_muted),
                ))]
            } else {
                evidence
            })
            .block(section_block("CONFIDENCE · INFERRED", theme, false))
            .wrap(Wrap { trim: true }),
            columns[2],
        );
    }
}

fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    _class: LayoutClass,
) {
    let body = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let title = Line::from(vec![
        Span::styled("TIMELINE", Style::default().fg(theme.text_muted).bold()),
        Span::styled(
            "  Filter: all · all devices",
            Style::default().fg(theme.text_secondary),
        ),
    ]);
    let block = section_block("HISTORY", theme, true).title(title);
    let inner = block.inner(body);
    frame.render_widget(block, body);
    if state.events.is_empty() {
        render_empty(
            frame,
            inner,
            theme,
            "NO NETWORK HISTORY YET",
            "Changes will appear after positive observations and scans.",
            "Run at least two scans to compare state.",
        );
        return;
    }
    let mut lines = vec![
        Line::from(Span::styled(
            "TODAY",
            Style::default().fg(theme.accent).bold(),
        )),
        Line::from(""),
    ];
    for event in state
        .events
        .iter()
        .take(inner.height.saturating_sub(2) as usize / 2)
    {
        lines.push(event_line(event, theme));
        lines.push(Line::from(vec![
            Span::raw("        "),
            Span::styled(event.detail.clone(), Style::default().fg(theme.text_muted)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_compare(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    let body = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let diff = &state.scan_diff;
    if diff.added.is_empty()
        && diff.removed.is_empty()
        && diff.changed.is_empty()
        && state.baseline_diff == Default::default()
    {
        render_empty(
            frame,
            body,
            theme,
            "NO SCAN DIFFERENCES YET",
            "Comparison is available after two completed scans.",
            "Create a baseline to highlight expected network state.",
        );
        return;
    }
    let columns = if class == LayoutClass::Compact {
        Layout::vertical([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(body)
    } else {
        Layout::horizontal([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(body)
    };
    let added = diff
        .added
        .iter()
        .map(|item| {
            ListItem::new(Line::from(vec![
                Span::styled("+ ", Style::default().fg(theme.success).bold()),
                Span::styled(item.name.clone(), Style::default().fg(theme.text_primary)),
                Span::styled(
                    format!("  {}", item.ip),
                    Style::default().fg(theme.text_muted),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let removed = diff
        .removed
        .iter()
        .map(|item| {
            ListItem::new(Line::from(vec![
                Span::styled("- ", Style::default().fg(theme.danger).bold()),
                Span::styled(item.name.clone(), Style::default().fg(theme.text_primary)),
                Span::styled(
                    format!("  {}", item.ip),
                    Style::default().fg(theme.text_muted),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    let changed = diff
        .changed
        .iter()
        .map(|item| {
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled("~ ", Style::default().fg(theme.warning).bold()),
                    Span::styled(
                        item.after.name.clone(),
                        Style::default().fg(theme.text_primary),
                    ),
                ]),
                Line::from(Span::styled(
                    item.changes.join(" · "),
                    Style::default().fg(theme.text_muted),
                )),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(or_empty(added, "No added devices", theme))
            .block(section_block("ADDED", theme, true)),
        columns[0],
    );
    frame.render_widget(
        List::new(or_empty(removed, "No removed devices", theme))
            .block(section_block("REMOVED", theme, false)),
        columns[1],
    );
    frame.render_widget(
        List::new(or_empty(changed, "No changed devices", theme))
            .block(section_block("CHANGED", theme, false)),
        columns[2],
    );
}

fn or_empty<'a>(items: Vec<ListItem<'a>>, label: &'a str, theme: &Theme) -> Vec<ListItem<'a>> {
    if items.is_empty() {
        vec![ListItem::new(Span::styled(
            label,
            Style::default().fg(theme.text_muted),
        ))]
    } else {
        items
    }
}

fn render_alerts(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let body = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    if state.alerts.is_empty() {
        render_empty(
            frame,
            body,
            theme,
            "NO ACTIVE ALERTS",
            "Wireseer has not found anything requiring attention.",
            "Baseline differences and discovery failures will appear here.",
        );
        return;
    }
    let active = state
        .alerts
        .iter()
        .filter(|alert| alert.resolved_at.is_none())
        .count();
    let items = state
        .alerts
        .iter()
        .map(|alert| {
            let color = severity_color(alert.severity, theme);
            ListItem::new(Text::from(vec![
                Line::from(vec![
                    Span::styled(
                        severity_symbol(alert.severity),
                        Style::default().fg(color).bold(),
                    ),
                    Span::styled(
                        format!(" {:<9}", format!("{:?}", alert.severity).to_uppercase()),
                        Style::default().fg(color).bold(),
                    ),
                    Span::styled(
                        alert.summary.clone(),
                        Style::default().fg(theme.text_primary).bold(),
                    ),
                    Span::styled(
                        format!("  {}", relative_time(alert.created_at)),
                        Style::default().fg(theme.text_muted),
                    ),
                    Span::styled(
                        if alert.acknowledged {
                            " · ACKNOWLEDGED"
                        } else {
                            ""
                        },
                        Style::default().fg(theme.success).bold(),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("             "),
                    Span::styled(
                        alert.detail.clone(),
                        Style::default().fg(theme.text_secondary),
                    ),
                ]),
                Line::from(""),
            ]))
        })
        .collect::<Vec<_>>();
    let mut list_state =
        ListState::default().with_selected(Some(state.selected.min(state.alerts.len() - 1)));
    frame.render_stateful_widget(
        List::new(items)
            .block(section_block(
                &format!("ACTIVE ALERTS · {active} active"),
                theme,
                true,
            ))
            .highlight_style(
                Style::default()
                    .bg(theme.selection)
                    .fg(theme.selection_text)
                    .bold(),
            )
            .highlight_symbol(if state.icons == IconMode::Ascii {
                ">"
            } else {
                "▌"
            }),
        body,
        &mut list_state,
    );
}

fn render_logs(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let body = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let block = section_block("STRUCTURED LOGS · level ≥ info · follow on", theme, true);
    let inner = block.inner(body);
    frame.render_widget(block, body);
    if state.logs.is_empty() {
        render_empty(
            frame,
            inner,
            theme,
            "NO LOG ENTRIES",
            "Runtime diagnostics will stream here.",
            "Set log_level = \"debug\" in TOML for more detail.",
        );
        return;
    }
    let available = inner.height as usize;
    let start = state.logs.len().saturating_sub(available);
    let lines = state
        .logs
        .iter()
        .skip(start)
        .map(|entry| {
            let color = match entry.level {
                LogLevel::Trace | LogLevel::Debug => theme.text_muted,
                LogLevel::Info => theme.info,
                LogLevel::Warn => theme.warning,
                LogLevel::Error => theme.danger,
            };
            Line::from(vec![
                Span::styled(
                    entry.at.format("%H:%M:%S").to_string(),
                    Style::default().fg(theme.text_muted),
                ),
                Span::raw(" "),
                Span::styled(entry.level.label(), Style::default().fg(color).bold()),
                Span::raw(" "),
                Span::styled(
                    format!("{:<12}", entry.module),
                    Style::default().fg(theme.text_secondary),
                ),
                Span::styled(
                    format!("{:<24}", entry.message),
                    Style::default().fg(theme.text_primary),
                ),
                Span::styled(
                    format!(" {}", entry.fields),
                    Style::default().fg(theme.text_muted),
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    class: LayoutClass,
) {
    let body = area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let columns = if class == LayoutClass::Compact {
        Layout::vertical([Constraint::Percentage(42), Constraint::Percentage(58)]).split(body)
    } else {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(body)
    };
    let appearance = vec![
        setting_line(
            "Theme",
            state.theme.label(),
            state.settings_selected == 0,
            theme,
        ),
        setting_line(
            "Icons",
            &format!("{:?}", state.icons),
            state.settings_selected == 1,
            theme,
        ),
        setting_line(
            "Animation",
            if state.animations { "On" } else { "Off" },
            state.settings_selected == 2,
            theme,
        ),
        setting_line(
            "Row density",
            if state.compact_rows {
                "Compact"
            } else {
                "Detailed"
            },
            state.settings_selected == 3,
            theme,
        ),
        setting_line(
            "Mouse",
            if state.mouse { "On" } else { "Off" },
            state.settings_selected == 4,
            theme,
        ),
    ];
    frame.render_widget(
        Paragraph::new(appearance).block(section_block(
            "APPEARANCE",
            theme,
            state.settings_selected < 5,
        )),
        columns[0],
    );

    let mut protocols = vec![setting_line(
        "Scan mode",
        state.scan_mode.label(),
        state.settings_selected == 5,
        theme,
    )];
    protocols.extend(
        state.provider_health.iter().map(|health| {
            provider_health_line(health.provider, health.status, &health.detail, theme)
        }),
    );
    frame.render_widget(
        Paragraph::new(protocols).block(section_block(
            "DISCOVERY",
            theme,
            state.settings_selected == 5,
        )),
        columns[1],
    );
}

fn provider_health_line(
    provider: &str,
    status: HealthStatus,
    detail: &str,
    theme: &Theme,
) -> Line<'static> {
    let label = match provider {
        "local" => "Local",
        "tcp" => "TCP checks",
        "reverse-dns" => "Reverse DNS",
        "arp" => "ARP cache",
        "icmp" => "ICMP",
        "ssdp" => "SSDP",
        "mdns" => "mDNS",
        "netbios" => "NetBIOS",
        "http-metadata" => "HTTP metadata",
        "tls-metadata" => "TLS metadata",
        _ => provider,
    };
    let (status_label, color) = match status {
        HealthStatus::Available => ("Available", theme.success),
        HealthStatus::Disabled => ("Disabled", theme.text_muted),
        HealthStatus::Degraded => ("Degraded", theme.warning),
        HealthStatus::Unavailable => ("Unavailable", theme.danger),
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{label:<18}"),
            Style::default().fg(theme.text_secondary),
        ),
        Span::styled(status_label, Style::default().fg(color).bold()),
        Span::styled(
            format!(" · {detail}"),
            Style::default().fg(theme.text_muted),
        ),
    ])
}

fn setting_line(label: &str, value: &str, selected: bool, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            if selected { "> " } else { "  " },
            Style::default().fg(theme.accent).bold(),
        ),
        Span::styled(
            format!("{label:<18}"),
            Style::default().fg(theme.text_secondary),
        ),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ])
}

fn render_empty(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    title: &str,
    body: &str,
    hint: &str,
) {
    let height = 7.min(area.height);
    let width = 62.min(area.width.saturating_sub(2));
    let centered = centered_rect(width, height, area);
    let text = vec![
        Line::from(Span::styled(
            title.to_string(),
            Style::default().fg(theme.accent).bold(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            body.to_string(),
            Style::default().fg(theme.text_primary),
        )),
        Line::from(Span::styled(
            hint.to_string(),
            Style::default().fg(theme.text_muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        centered,
    );
}

fn render_overlay(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    match state.overlay {
        Overlay::None | Overlay::Search => {}
        Overlay::Help => render_help(frame, area, state, theme),
        Overlay::CommandPalette => render_palette(frame, area, state, theme),
        Overlay::Filter => render_picker(
            frame,
            area,
            theme,
            "FILTER DEVICES",
            DeviceFilter::ALL
                .iter()
                .map(|filter| (filter.label(), *filter == state.filter))
                .collect(),
        ),
        Overlay::Sort => render_picker(
            frame,
            area,
            theme,
            "SORT DEVICES",
            DeviceSort::ALL
                .iter()
                .map(|sort| (sort.label(), *sort == state.sort))
                .collect(),
        ),
        Overlay::Interface => render_picker(
            frame,
            area,
            theme,
            "SELECT INTERFACE",
            state
                .interfaces
                .iter()
                .enumerate()
                .map(|(index, interface)| {
                    (interface.name.as_str(), index == state.palette_selected)
                })
                .collect(),
        ),
        Overlay::ScanMode => render_picker(
            frame,
            area,
            theme,
            "SCAN MODE",
            [
                ScanMode::Quick,
                ScanMode::Normal,
                ScanMode::Deep,
                ScanMode::Watch,
                ScanMode::Passive,
            ]
            .iter()
            .enumerate()
            .map(|(index, mode)| (mode.label(), index == state.palette_selected))
            .collect(),
        ),
        Overlay::Theme => render_theme_picker(frame, area, state, theme),
        Overlay::Export => render_dialog(
            frame,
            area,
            theme,
            "EXPORT",
            vec![
                Line::from("Choose JSON, XML, or CSV from the CLI:"),
                Line::from(Span::styled(
                    "wireseer export --format json",
                    Style::default().fg(theme.accent),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Esc  Close",
                    Style::default().fg(theme.text_muted),
                )),
            ],
            (58, 9),
        ),
        Overlay::ConfirmQuit => render_dialog(
            frame,
            area,
            theme,
            "CANCEL SCAN AND QUIT?",
            vec![
                Line::from("A discovery scan is still active."),
                Line::from("Wireseer will cancel workers and flush history."),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Enter", Style::default().fg(theme.accent).bold()),
                    Span::raw(" Quit    "),
                    Span::styled("Esc", Style::default().fg(theme.accent).bold()),
                    Span::raw(" Continue scanning"),
                ]),
            ],
            (58, 9),
        ),
        Overlay::Error => render_dialog(
            frame,
            area,
            theme,
            "SOMETHING NEEDS ATTENTION",
            vec![
                Line::from(Span::styled(
                    state
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".into()),
                    Style::default().fg(theme.text_primary),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Wireseer remains usable. Run `wireseer doctor` for diagnostics.",
                    Style::default().fg(theme.text_secondary),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Esc  Close",
                    Style::default().fg(theme.accent),
                )),
            ],
            (68, 11),
        ),
        Overlay::EditName => render_editor(
            frame,
            area,
            state,
            theme,
            "CONFIRM DEVICE IDENTITY",
            "Enter the exact model or name. Empty restores automatic detection.",
            (72, 9),
        ),
        Overlay::EditTags => render_editor(
            frame,
            area,
            state,
            theme,
            "EDIT TAGS",
            "Comma-separated tags, for example: trusted, office",
            (68, 9),
        ),
        Overlay::EditNotes => render_editor(
            frame,
            area,
            state,
            theme,
            "EDIT NOTES",
            "A short local note. It is never sent over the network.",
            (72, 11),
        ),
    }
}

fn render_help(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let screens = [
        ("1", "Dashboard", "Overview, activity, and health"),
        ("2", "Devices", "Inventory and device inspector"),
        ("3", "History", "Discovery and change timeline"),
        ("4", "Compare", "Scan and baseline differences"),
        ("5", "Alerts", "Items that need attention"),
        ("6", "Logs", "Runtime diagnostics"),
        ("7", "Settings", "Appearance and discovery"),
    ];
    let current: &[(&str, &str)] = match state.screen {
        Screen::Dashboard => &[],
        Screen::Devices => &[
            ("↑↓ / j/k", "Select device"),
            ("Home/End", "First / last device"),
            ("Enter", "Open device details"),
            ("Space", "Toggle selection"),
            ("/", "Search inventory"),
            ("f / s", "Cycle filter / sort"),
            ("e", "Export view"),
        ],
        Screen::DeviceDetails => &[
            ("↑↓ / j/k", "Previous / next device"),
            ("Home/End", "First / last device"),
            ("Esc", "Back to devices"),
            ("e / t / n", "Confirm identity / edit tags / notes"),
        ],
        Screen::History | Screen::Compare | Screen::Logs => &[("e", "Export view")],
        Screen::Alerts => &[
            ("↑↓ / j/k", "Select alert"),
            ("Home/End", "First / last alert"),
            ("a", "Acknowledge selected alert"),
            ("e", "Export view"),
        ],
        Screen::Settings => &[
            ("↑↓ / j/k", "Select setting"),
            ("Home/End", "First / last setting"),
            ("Enter/Space", "Open or change selected setting"),
            ("t / i", "Theme picker / icon shortcut"),
            ("a / m / c", "Animation / scan mode / density"),
        ],
    };
    let mut actions = current.to_vec();
    actions.extend([
        ("q", "Quit safely"),
        ("? / Esc", "Close help / go back"),
        ("Ctrl+P", "Search every command"),
        ("r", "Start a network scan"),
    ]);
    if state.scan.active {
        actions.push(("x", "Cancel the active scan"));
    }
    actions.push((
        "Mouse",
        if state.mouse {
            "Click / wheel / right-click back"
        } else {
            "Disabled · enable it in Settings"
        },
    ));

    let widths = HelpColumnWidths::natural(&screens, &actions);
    let roomy_separator = if state.icons == IconMode::Ascii {
        " | "
    } else {
        " │ "
    };
    let compact_separator = if state.icons == IconMode::Ascii {
        " |"
    } else {
        " │"
    };
    let available_dialog_width = area.width.saturating_sub(2);
    let available_content_width = usize::from(available_dialog_width.saturating_sub(4));
    let stacked_height = u16::try_from(screens.len() + actions.len())
        .unwrap_or(u16::MAX)
        .saturating_add(7);
    let stacked_width = widths.left_width().max(widths.right_width());
    let use_stacked = widths.total_width(roomy_separator) > available_content_width
        && stacked_width <= available_content_width
        && stacked_height <= area.height.saturating_sub(2);

    let (lines, content_width, height) = if use_stacked {
        let mut lines = vec![help_section_header("SCREENS", theme)];
        lines.extend(
            screens
                .iter()
                .copied()
                .map(|screen| help_screen_line(screen, widths, theme)),
        );
        lines.push(Line::from(""));
        lines.push(help_section_header(
            &format!("{} / GLOBAL", state.screen.title().to_uppercase()),
            theme,
        ));
        lines.extend(
            actions
                .iter()
                .copied()
                .map(|action| help_action_line(action, widths, theme)),
        );
        (lines, stacked_width, stacked_height)
    } else {
        let separator = if widths.total_width(roomy_separator) <= available_content_width {
            roomy_separator
        } else {
            compact_separator
        };
        let widths = widths.fit(available_content_width, separator);
        let left_width = widths.left_width();
        let mut lines = vec![Line::from(vec![
            Span::styled(
                fit_cell("SCREENS", left_width),
                Style::default().fg(theme.accent).bold(),
            ),
            Span::styled(separator, Style::default().fg(theme.border)),
            Span::styled(
                format!("{} / GLOBAL", state.screen.title().to_uppercase()),
                Style::default().fg(theme.accent).bold(),
            ),
        ])];
        let row_count = screens.len().max(actions.len());
        for index in 0..row_count {
            let screen = screens.get(index).copied().unwrap_or(("", "", ""));
            let action = actions.get(index).copied().unwrap_or(("", ""));
            lines.push(help_columns_line(screen, action, widths, separator, theme));
        }
        let height = u16::try_from(row_count)
            .unwrap_or(u16::MAX)
            .saturating_add(5);
        (lines, widths.total_width(separator), height)
    };

    let width = u16::try_from(content_width)
        .unwrap_or(u16::MAX)
        .saturating_add(4)
        .min(available_dialog_width);
    render_dialog(
        frame,
        area,
        theme,
        "INPUT REFERENCE",
        lines,
        (width, height.min(area.height.saturating_sub(2))),
    );
}

#[derive(Debug, Clone, Copy)]
struct HelpColumnWidths {
    screen_key: usize,
    screen_name: usize,
    screen_description: usize,
    action_key: usize,
    action_description: usize,
}

impl HelpColumnWidths {
    fn natural(screens: &[(&str, &str, &str)], actions: &[(&str, &str)]) -> Self {
        Self {
            screen_key: max_cell_width(screens.iter().map(|screen| screen.0)).saturating_add(1),
            screen_name: max_cell_width(screens.iter().map(|screen| screen.1)).saturating_add(1),
            screen_description: max_cell_width(screens.iter().map(|screen| screen.2)),
            action_key: max_cell_width(actions.iter().map(|action| action.0)).saturating_add(1),
            action_description: max_cell_width(actions.iter().map(|action| action.1)),
        }
    }

    const fn left_width(self) -> usize {
        self.screen_key + self.screen_name + self.screen_description
    }

    const fn right_width(self) -> usize {
        self.action_key + self.action_description
    }

    fn total_width(self, separator: &str) -> usize {
        self.left_width() + UnicodeWidthStr::width(separator) + self.right_width()
    }

    fn fit(mut self, available: usize, separator: &str) -> Self {
        let fixed = self.screen_key
            + self.screen_name
            + self.action_key
            + UnicodeWidthStr::width(separator);
        let available_descriptions = available.saturating_sub(fixed);
        let natural_descriptions = self.screen_description + self.action_description;
        if available_descriptions >= natural_descriptions {
            return self;
        }

        let screen_min = self.screen_description.min(12);
        let action_min = self.action_description.min(16);
        if available_descriptions <= screen_min + action_min {
            self.screen_description = (available_descriptions / 2).min(self.screen_description);
            self.action_description = available_descriptions
                .saturating_sub(self.screen_description)
                .min(self.action_description);
            return self;
        }

        let extra = available_descriptions - screen_min - action_min;
        let screen_capacity = self.screen_description - screen_min;
        let action_capacity = self.action_description - action_min;
        let capacity = screen_capacity + action_capacity;
        let screen_extra = extra
            .saturating_mul(screen_capacity)
            .checked_div(capacity)
            .unwrap_or(0)
            .min(screen_capacity);
        self.screen_description = screen_min + screen_extra;
        self.action_description =
            action_min + extra.saturating_sub(screen_extra).min(action_capacity);
        self
    }
}

fn max_cell_width<'a>(values: impl Iterator<Item = &'a str>) -> usize {
    values.map(UnicodeWidthStr::width).max().unwrap_or(0)
}

fn help_section_header(title: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(theme.accent).bold(),
    ))
}

fn help_screen_line(
    screen: (&str, &str, &str),
    widths: HelpColumnWidths,
    theme: &Theme,
) -> Line<'static> {
    let (key, name, description) = screen;
    Line::from(vec![
        Span::styled(
            fit_cell(key, widths.screen_key),
            Style::default().fg(theme.accent).bold(),
        ),
        Span::styled(
            fit_cell(name, widths.screen_name),
            Style::default().fg(theme.text_primary).bold(),
        ),
        Span::styled(
            fit_cell(description, widths.screen_description),
            Style::default().fg(theme.text_muted),
        ),
    ])
}

fn help_action_line(
    action: (&str, &str),
    widths: HelpColumnWidths,
    theme: &Theme,
) -> Line<'static> {
    let (key, description) = action;
    Line::from(vec![
        Span::styled(
            fit_cell(key, widths.action_key),
            Style::default().fg(theme.accent).bold(),
        ),
        Span::styled(
            fit_cell(description, widths.action_description),
            Style::default().fg(theme.text_secondary),
        ),
    ])
}

fn help_columns_line(
    screen: (&str, &str, &str),
    action: (&str, &str),
    widths: HelpColumnWidths,
    separator: &'static str,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = help_screen_line(screen, widths, theme).spans;
    spans.push(Span::styled(separator, Style::default().fg(theme.border)));
    spans.extend(help_action_line(action, widths, theme).spans);
    Line::from(spans)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteCommand {
    Dashboard,
    Devices,
    History,
    Compare,
    Alerts,
    Logs,
    Settings,
    Rescan,
    CancelScan,
    Changed,
    Filter,
    Sort,
    Interface,
    ScanMode,
    Theme,
    Icons,
    Animations,
    CompactRows,
    Mouse,
    Export,
    Help,
    Doctor,
    Quit,
}

impl PaletteCommand {
    const ALL: &'static [(Self, &'static str, &'static str)] = &[
        (
            Self::Dashboard,
            "Open dashboard",
            "Overview, activity, and network health",
        ),
        (
            Self::Devices,
            "Open devices",
            "Inventory and device inspector",
        ),
        (
            Self::History,
            "Open history",
            "Discovery and change timeline",
        ),
        (
            Self::Compare,
            "Open compare",
            "Scan and baseline differences",
        ),
        (Self::Alerts, "Open alerts", "Items that need attention"),
        (Self::Logs, "Open logs", "Runtime diagnostics"),
        (
            Self::Settings,
            "Open settings",
            "Appearance and discovery options",
        ),
        (
            Self::Rescan,
            "Rescan current subnet",
            "Start conservative discovery",
        ),
        (
            Self::CancelScan,
            "Cancel active scan",
            "Stop current discovery immediately",
        ),
        (
            Self::Changed,
            "Show changed devices",
            "Open devices with the changed filter",
        ),
        (
            Self::Filter,
            "Cycle device filter",
            "All, online, offline, unknown, or changed",
        ),
        (
            Self::Sort,
            "Cycle device sort",
            "Status, name, address, vendor, latency, or confidence",
        ),
        (
            Self::Interface,
            "Switch interface",
            "Select an available IPv4 interface",
        ),
        (
            Self::ScanMode,
            "Change scan mode",
            "Quick, normal, deep, watch, or passive",
        ),
        (
            Self::Theme,
            "Choose theme",
            "Light, dark, neon, and accessible palettes",
        ),
        (
            Self::Icons,
            "Toggle icon set",
            "Nerd Font, Unicode, or ASCII",
        ),
        (
            Self::Animations,
            "Toggle animations",
            "Enable or disable motion",
        ),
        (
            Self::CompactRows,
            "Toggle row density",
            "Detailed metadata or compact one-line rows",
        ),
        (
            Self::Mouse,
            "Toggle mouse input",
            "Enable or disable clicks and scrolling immediately",
        ),
        (
            Self::Export,
            "Export current view",
            "Show JSON and CSV export guidance",
        ),
        (
            Self::Help,
            "Open keyboard help",
            "Show screens and shortcuts",
        ),
        (
            Self::Doctor,
            "Open diagnostics",
            "Use `wireseer doctor` for capability checks",
        ),
        (
            Self::Quit,
            "Quit Wireseer",
            "Cancel work and restore the terminal",
        ),
    ];
}

fn palette_matches(query: &str) -> Vec<(PaletteCommand, &'static str, &'static str)> {
    let matcher = SkimMatcherV2::default();
    let mut matches = PaletteCommand::ALL
        .iter()
        .filter_map(|(command, label, description)| {
            if query.is_empty() {
                Some((*command, *label, *description, 0))
            } else {
                matcher
                    .fuzzy_match(label, query)
                    .map(|score| (*command, *label, *description, score))
            }
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(_, _, _, score)| std::cmp::Reverse(*score));
    matches
        .into_iter()
        .map(|(command, label, description, _)| (command, label, description))
        .collect()
}

fn render_palette(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let width = 72.min(area.width.saturating_sub(6));
    let height = 24.min(area.height.saturating_sub(2));
    let dialog = centered_rect(width, height, area);
    frame.render_widget(Clear, dialog);
    frame.render_widget(
        Block::default()
            .style(Style::default().bg(theme.surface))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .title(Span::styled(
                " COMMANDS ",
                Style::default().fg(theme.accent).bold(),
            )),
        dialog,
    );
    let inner = dialog.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(inner);
    let query = if state.palette_query.is_empty() {
        "Type a command…"
    } else {
        &state.palette_query
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.accent).bold()),
            Span::styled(
                query,
                Style::default().fg(if state.palette_query.is_empty() {
                    theme.text_muted
                } else {
                    theme.text_primary
                }),
            ),
        ]))
        .style(Style::default().bg(theme.surface_active)),
        rows[0],
    );
    let matches = palette_matches(&state.palette_query);
    let visible = usize::from((rows[1].height / 2).max(1));
    let selected = state.palette_selected.min(matches.len().saturating_sub(1));
    let max_start = matches.len().saturating_sub(visible);
    let start = selected
        .saturating_sub(visible.saturating_sub(1))
        .min(max_start);
    let end = start.saturating_add(visible).min(matches.len());
    let items = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, (_, label, description))| {
            let style = if index == selected {
                Style::default()
                    .bg(theme.selection)
                    .fg(theme.selection_text)
                    .bold()
            } else {
                Style::default().fg(theme.text_primary)
            };
            ListItem::new(Text::from(vec![
                Line::from(Span::styled(format!("  {label}"), style)),
                Line::from(Span::styled(
                    format!("    {description}"),
                    if index == selected {
                        style
                    } else {
                        Style::default().fg(theme.text_muted)
                    },
                )),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items).highlight_symbol("▌"), rows[1]);
    if matches.len() > visible {
        let mut scrollbar_state = ScrollbarState::new(matches.len()).position(selected);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(theme.accent))
                .track_style(Style::default().fg(theme.border)),
            rows[1],
            &mut scrollbar_state,
        );
    }
    let position = if matches.is_empty() { 0 } else { selected + 1 };
    let display_start = if matches.is_empty() { 0 } else { start + 1 };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "↑↓ Choose   Enter Run   Esc Close",
                Style::default().fg(theme.text_muted),
            ),
            Span::styled(
                format!(
                    "   {position}/{} · showing {display_start}–{end}",
                    matches.len()
                ),
                Style::default().fg(theme.text_muted),
            ),
        ])),
        rows[2],
    );
}

fn render_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    title: &str,
    items: Vec<(&str, bool)>,
) {
    let content = items
        .into_iter()
        .map(|(label, selected)| {
            Line::from(vec![
                Span::styled(
                    if selected { "● " } else { "  " },
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    label.to_string(),
                    Style::default()
                        .fg(if selected {
                            theme.text_primary
                        } else {
                            theme.text_secondary
                        })
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            ])
        })
        .chain(std::iter::once(Line::from("")))
        .chain(std::iter::once(Line::from(Span::styled(
            "Use the contextual shortcut to cycle · Esc close",
            Style::default().fg(theme.text_muted),
        ))))
        .collect::<Vec<_>>();
    render_dialog(frame, area, theme, title, content, (52, 13));
}

fn render_theme_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let (start, visible) = theme_picker_window(area, state.palette_selected);
    let saved_theme = state.theme_before_preview.unwrap_or(state.theme);
    let mut content = ThemeName::ALL
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, name)| {
            let palette = Theme::named(*name);
            let selected = index == state.palette_selected;
            let saved = *name == saved_theme;
            Line::from(vec![
                Span::styled(
                    if selected { "› " } else { "· " },
                    Style::default().fg(theme.accent).bold(),
                ),
                Span::styled(
                    format!("{}  ", theme_symbol(*name)),
                    Style::default().fg(palette.accent).bold(),
                ),
                Span::styled(
                    format!("{:<23}", name.label()),
                    Style::default()
                        .fg(if selected {
                            theme.text_primary
                        } else {
                            theme.text_secondary
                        })
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("{:<32}", name.description()),
                    Style::default().fg(theme.text_muted),
                ),
                Span::styled(
                    if saved { "saved" } else { "" },
                    Style::default().fg(theme.success).bold(),
                ),
            ])
            .style(if selected {
                Style::default().bg(theme.surface_active)
            } else {
                Style::default()
            })
        })
        .collect::<Vec<_>>();
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        format!(
            "↑↓ Live preview · Enter Keep · Esc Revert · click Keep · {}/{}",
            state.palette_selected.min(ThemeName::ALL.len() - 1) + 1,
            ThemeName::ALL.len()
        ),
        Style::default().fg(theme.text_muted),
    )));
    render_dialog(
        frame,
        area,
        theme,
        "THEME · LIVE PREVIEW",
        content,
        (86, 17),
    );
}

const fn theme_symbol(name: ThemeName) -> &'static str {
    match name {
        ThemeName::WireseerDark => "◈",
        ThemeName::CatppuccinMocha => "◆",
        ThemeName::CatppuccinLatte => "○",
        ThemeName::Dracula => "◇",
        ThemeName::Nord => "△",
        ThemeName::MidnightBlue => "◉",
        ThemeName::Acid => "▲",
        ThemeName::PaperLight => "□",
        ThemeName::HighContrast => "◑",
        ThemeName::Monochrome => "■",
        ThemeName::ColorBlind => "◎",
    }
}

fn theme_picker_window(area: Rect, selected: usize) -> (usize, usize) {
    let dialog_height = 17.min(area.height.saturating_sub(2));
    let visible = usize::from(dialog_height.saturating_sub(6).max(1)).min(ThemeName::ALL.len());
    let selected = selected.min(ThemeName::ALL.len().saturating_sub(1));
    let start = selected
        .saturating_sub(visible.saturating_sub(1))
        .min(ThemeName::ALL.len().saturating_sub(visible));
    (start, visible)
}

fn render_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    title: &str,
    lines: Vec<Line<'static>>,
    size: (u16, u16),
) {
    let dialog = centered_rect(
        size.0.min(area.width.saturating_sub(2)),
        size.1.min(area.height.saturating_sub(2)),
        area,
    );
    frame.render_widget(Clear, dialog);
    let block = Block::default()
        .style(Style::default().bg(theme.surface))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focused))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme.accent).bold(),
        ))
        .padding(Padding::uniform(1));
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        dialog,
    );
}

fn render_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    title: &str,
    hint: &str,
    size: (u16, u16),
) {
    let dialog = centered_rect(
        size.0.min(area.width.saturating_sub(2)),
        size.1.min(area.height.saturating_sub(2)),
        area,
    );
    frame.render_widget(Clear, dialog);
    let block = Block::default()
        .style(Style::default().bg(theme.surface))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focused))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme.accent).bold(),
        ))
        .padding(Padding::uniform(1));
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.accent).bold()),
            Span::styled(
                format!("{}▏", state.editor),
                Style::default()
                    .fg(theme.text_primary)
                    .bg(theme.surface_active),
            ),
        ])),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            hint.to_string(),
            Style::default().fg(theme.text_muted),
        ))
        .wrap(Wrap { trim: true }),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(theme.accent).bold()),
            Span::raw(" Save   "),
            Span::styled("Esc", Style::default().fg(theme.accent).bold()),
            Span::raw(" Cancel"),
        ])),
        rows[2],
    );
}

fn render_toast(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    let Some(toast) = &state.toast else { return };
    if state.overlay != Overlay::None && state.overlay != Overlay::Search {
        return;
    }
    let width = (toast.message.len() as u16 + 6)
        .min(area.width.saturating_sub(4))
        .max(22);
    let rect = Rect::new(area.right().saturating_sub(width + 2), area.y + 3, width, 3);
    let color = severity_color(toast.severity, theme);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Span::styled(
            toast.message.clone(),
            Style::default().fg(theme.text_primary),
        ))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color))
                .style(Style::default().bg(theme.surface))
                .padding(Padding::top(0)),
        ),
        rect,
    );
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

fn severity_color(severity: Severity, theme: &Theme) -> Color {
    match severity {
        Severity::Info => theme.info,
        Severity::Warning => theme.warning,
        Severity::Critical => theme.danger,
    }
}

const fn severity_symbol(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "i",
        Severity::Warning => "!",
        Severity::Critical => "×",
    }
}

fn relative_time(time: DateTime<Utc>) -> String {
    let elapsed = Utc::now().signed_duration_since(time);
    if elapsed.num_seconds() < 10 {
        "now".into()
    } else if elapsed.num_minutes() < 1 {
        format!("{}s ago", elapsed.num_seconds())
    } else if elapsed.num_hours() < 1 {
        format!("{}m ago", elapsed.num_minutes())
    } else if elapsed.num_days() < 1 {
        format!("{}h ago", elapsed.num_hours())
    } else {
        format!("{}d ago", elapsed.num_days())
    }
}

fn handle_terminal_event(runtime: &mut AppRuntime, event: Event, area: Rect) {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(runtime, key),
        Event::Mouse(mouse) if runtime.state.mouse => handle_mouse(runtime, mouse, area),
        Event::Resize(_, _)
        | Event::FocusGained
        | Event::FocusLost
        | Event::Paste(_)
        | Event::Mouse(_)
        | Event::Key(_) => {}
    }
}

fn handle_mouse(runtime: &mut AppRuntime, mouse: MouseEvent, area: Rect) {
    if runtime.state.overlay != Overlay::None && runtime.state.overlay != Overlay::Search {
        handle_overlay_mouse(runtime, mouse, area);
        return;
    }

    match mouse.kind {
        MouseEventKind::ScrollDown if runtime.state.screen == Screen::Settings => {
            runtime.state.next_setting();
        }
        MouseEventKind::ScrollUp if runtime.state.screen == Screen::Settings => {
            runtime.state.previous_setting();
        }
        MouseEventKind::ScrollDown if runtime.state.screen == Screen::Devices => {
            runtime.state.next();
        }
        MouseEventKind::ScrollUp if runtime.state.screen == Screen::Devices => {
            runtime.state.previous();
        }
        MouseEventKind::ScrollDown if runtime.state.screen == Screen::Alerts => {
            runtime.state.next_alert();
        }
        MouseEventKind::ScrollUp if runtime.state.screen == Screen::Alerts => {
            runtime.state.previous_alert();
        }
        MouseEventKind::Down(MouseButton::Left) => handle_primary_click(runtime, mouse, area),
        MouseEventKind::Down(MouseButton::Middle) if runtime.state.screen == Screen::Devices => {
            if let Some(index) = device_row_at(&runtime.state, area, mouse.column, mouse.row) {
                runtime.state.selected = index;
                runtime.state.toggle_selected();
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            handle_key(runtime, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        _ => {}
    }
}

fn handle_primary_click(runtime: &mut AppRuntime, mouse: MouseEvent, area: Rect) {
    if let Some(screen) = footer_screen_at(area, mouse.column, mouse.row) {
        runtime.state.change_screen(screen);
        return;
    }

    match runtime.state.screen {
        Screen::Settings => {
            if let Some(index) = setting_at(area, mouse.column, mouse.row) {
                runtime.state.settings_selected = index;
                activate_selected_setting(runtime);
            }
        }
        Screen::Devices => {
            let class = LayoutClass::from_area(area);
            let Some((search_area, _)) = device_regions(area, class) else {
                return;
            };
            if rect_contains(search_area, mouse.column, mouse.row) {
                runtime.state.overlay = Overlay::Search;
                return;
            }
            if let Some(index) = device_row_at(&runtime.state, area, mouse.column, mouse.row) {
                let was_selected = runtime.state.selected == index;
                runtime.state.selected = index;
                if was_selected {
                    runtime.state.open_details();
                }
            }
        }
        _ => {}
    }
}

fn handle_overlay_mouse(runtime: &mut AppRuntime, mouse: MouseEvent, area: Rect) {
    match mouse.kind {
        MouseEventKind::ScrollDown => adjust_overlay_selection(runtime, true),
        MouseEventKind::ScrollUp => adjust_overlay_selection(runtime, false),
        MouseEventKind::Down(MouseButton::Right) => {
            handle_overlay_key(runtime, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        MouseEventKind::Down(MouseButton::Left) => match runtime.state.overlay {
            Overlay::CommandPalette => {
                if let Some(index) = palette_index_at(&runtime.state, area, mouse.column, mouse.row)
                {
                    runtime.state.palette_selected = index;
                    execute_palette(runtime);
                }
            }
            Overlay::Interface => {
                if let Some(index) = picker_index_at(
                    area,
                    mouse.column,
                    mouse.row,
                    runtime.state.interfaces.len(),
                    (52, 13),
                ) {
                    runtime.state.palette_selected = index;
                    handle_overlay_key(runtime, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            Overlay::ScanMode => {
                if let Some(index) = picker_index_at(area, mouse.column, mouse.row, 5, (52, 13)) {
                    runtime.state.palette_selected = index;
                    handle_overlay_key(runtime, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            Overlay::Theme => {
                if let Some(index) =
                    theme_picker_index_at(&runtime.state, area, mouse.column, mouse.row)
                {
                    runtime.state.palette_selected = index;
                    handle_overlay_key(runtime, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn adjust_overlay_selection(runtime: &mut AppRuntime, forward: bool) {
    let count = match runtime.state.overlay {
        Overlay::CommandPalette => palette_matches(&runtime.state.palette_query).len(),
        Overlay::Interface => runtime.state.interfaces.len(),
        Overlay::ScanMode => 5,
        Overlay::Theme => ThemeName::ALL.len(),
        _ => 0,
    };
    if count == 0 {
        return;
    }
    let selected = if forward {
        (runtime.state.palette_selected + 1).min(count - 1)
    } else {
        runtime.state.palette_selected.saturating_sub(1)
    };
    if runtime.state.overlay == Overlay::Theme {
        preview_theme_at(runtime, selected);
    } else {
        runtime.state.palette_selected = selected;
    }
}

fn footer_screen_at(area: Rect, column: u16, row: u16) -> Option<Screen> {
    let class = LayoutClass::from_area(area);
    if class == LayoutClass::TooSmall || row != area.bottom().saturating_sub(2) {
        return None;
    }
    let pages = [
        (Screen::Dashboard, "Dashboard", "Home"),
        (Screen::Devices, "Devices", "Dev"),
        (Screen::History, "History", "Hist"),
        (Screen::Compare, "Compare", "Diff"),
        (Screen::Alerts, "Alerts", "Alert"),
        (Screen::Logs, "Logs", "Logs"),
        (Screen::Settings, "Settings", "Setup"),
    ];
    let mut left = area.x.saturating_add(1);
    for (screen, label, compact_label) in pages {
        let label = if class == LayoutClass::Compact {
            compact_label
        } else {
            label
        };
        let width = u16::try_from(UnicodeWidthStr::width(label))
            .unwrap_or(u16::MAX)
            .saturating_add(4);
        let right = left.saturating_add(width);
        if column >= left && column < right {
            return Some(screen);
        }
        left = right;
    }
    None
}

fn setting_at(area: Rect, column: u16, row: u16) -> Option<usize> {
    let class = LayoutClass::from_area(area);
    if class == LayoutClass::TooSmall {
        return None;
    }
    let root = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(area);
    let body = root[1].inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let columns = if class == LayoutClass::Compact {
        Layout::vertical([Constraint::Percentage(42), Constraint::Percentage(58)]).split(body)
    } else {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(body)
    };

    let appearance_row = columns[0].y.saturating_add(1);
    if rect_contains(columns[0], column, row)
        && row >= appearance_row
        && row < appearance_row.saturating_add(5)
    {
        return Some(usize::from(row - appearance_row));
    }

    let discovery_row = columns[1].y.saturating_add(1);
    if rect_contains(columns[1], column, row) && row == discovery_row {
        return Some(5);
    }
    None
}

fn device_regions(area: Rect, class: LayoutClass) -> Option<(Rect, Rect)> {
    if class == LayoutClass::TooSmall {
        return None;
    }
    let root = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(area);
    let body = root[1].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(body);
    let table = split_device_workspace(rows[1], class).0;
    Some((rows[0], table))
}

fn device_row_at(state: &AppState, area: Rect, column: u16, row: u16) -> Option<usize> {
    let class = LayoutClass::from_area(area);
    let (_, table) = device_regions(area, class)?;
    if !rect_contains(table, column, row) {
        return None;
    }
    let row_height = if state.compact_rows { 1 } else { 2 };
    let first_row = table.y.saturating_add(3);
    if row < first_row {
        return None;
    }
    let visible = usize::from(table.height.saturating_sub(3)) / row_height;
    if visible == 0 {
        return None;
    }
    let start = state.selected.saturating_add(1).saturating_sub(visible);
    let clicked = start + usize::from(row - first_row) / row_height;
    (clicked < state.filtered_indices().len()).then_some(clicked)
}

fn palette_index_at(state: &AppState, area: Rect, column: u16, row: u16) -> Option<usize> {
    let width = 72.min(area.width.saturating_sub(6));
    let height = 24.min(area.height.saturating_sub(2));
    let dialog = centered_rect(width, height, area);
    let inner = dialog.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(inner);
    let list = rows[1];
    if !rect_contains(list, column, row) {
        return None;
    }

    let matches = palette_matches(&state.palette_query);
    let visible = usize::from((list.height / 2).max(1));
    let selected = state.palette_selected.min(matches.len().saturating_sub(1));
    let start = selected
        .saturating_sub(visible.saturating_sub(1))
        .min(matches.len().saturating_sub(visible));
    let clicked = start + usize::from(row - list.y) / 2;
    (clicked < matches.len() && clicked < start.saturating_add(visible)).then_some(clicked)
}

fn picker_index_at(
    area: Rect,
    column: u16,
    row: u16,
    count: usize,
    size: (u16, u16),
) -> Option<usize> {
    let dialog = centered_rect(
        size.0.min(area.width.saturating_sub(2)),
        size.1.min(area.height.saturating_sub(2)),
        area,
    );
    if !rect_contains(dialog, column, row) {
        return None;
    }
    let first_row = dialog.y.saturating_add(2);
    if row < first_row {
        return None;
    }
    let index = usize::from(row - first_row);
    (index < count).then_some(index)
}

fn theme_picker_index_at(state: &AppState, area: Rect, column: u16, row: u16) -> Option<usize> {
    let dialog = centered_rect(
        86.min(area.width.saturating_sub(2)),
        17.min(area.height.saturating_sub(2)),
        area,
    );
    if !rect_contains(dialog, column, row) {
        return None;
    }
    let first_row = dialog.y.saturating_add(2);
    let (start, visible) = theme_picker_window(area, state.palette_selected);
    if row < first_row || row >= first_row.saturating_add(u16::try_from(visible).ok()?) {
        return None;
    }
    Some(start + usize::from(row - first_row))
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
}

fn handle_key(runtime: &mut AppRuntime, key: KeyEvent) {
    if runtime.state.overlay != Overlay::None {
        handle_overlay_key(runtime, key);
        return;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        runtime.state.overlay = Overlay::CommandPalette;
        runtime.state.palette_query.clear();
        runtime.state.palette_selected = 0;
        return;
    }
    match key.code {
        KeyCode::Char('?') => runtime.state.overlay = Overlay::Help,
        KeyCode::Char('q') => {
            if runtime.state.scan.active {
                runtime.state.overlay = Overlay::ConfirmQuit;
            } else {
                runtime.state.should_quit = true;
            }
        }
        KeyCode::Char('1') => runtime.state.change_screen(Screen::Dashboard),
        KeyCode::Char('2') => runtime.state.change_screen(Screen::Devices),
        KeyCode::Char('3') => runtime.state.change_screen(Screen::History),
        KeyCode::Char('4') => runtime.state.change_screen(Screen::Compare),
        KeyCode::Char('5') => runtime.state.change_screen(Screen::Alerts),
        KeyCode::Char('6') => runtime.state.change_screen(Screen::Logs),
        KeyCode::Char('7') => runtime.state.change_screen(Screen::Settings),
        KeyCode::Down | KeyCode::Char('j') if runtime.state.screen == Screen::Settings => {
            runtime.state.next_setting();
        }
        KeyCode::Up | KeyCode::Char('k') if runtime.state.screen == Screen::Settings => {
            runtime.state.previous_setting();
        }
        KeyCode::Home | KeyCode::Char('g') if runtime.state.screen == Screen::Settings => {
            runtime.state.first_setting();
        }
        KeyCode::End | KeyCode::Char('G') if runtime.state.screen == Screen::Settings => {
            runtime.state.last_setting();
        }
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right
            if runtime.state.screen == Screen::Settings =>
        {
            activate_selected_setting(runtime);
        }
        KeyCode::Down | KeyCode::Char('j') if runtime.state.screen == Screen::Alerts => {
            runtime.state.next_alert();
        }
        KeyCode::Up | KeyCode::Char('k') if runtime.state.screen == Screen::Alerts => {
            runtime.state.previous_alert();
        }
        KeyCode::Home | KeyCode::Char('g') if runtime.state.screen == Screen::Alerts => {
            runtime.state.first_alert();
        }
        KeyCode::End | KeyCode::Char('G') if runtime.state.screen == Screen::Alerts => {
            runtime.state.last_alert();
        }
        KeyCode::Down | KeyCode::Char('j')
            if matches!(
                runtime.state.screen,
                Screen::Devices | Screen::DeviceDetails
            ) =>
        {
            runtime.state.next();
        }
        KeyCode::Up | KeyCode::Char('k')
            if matches!(
                runtime.state.screen,
                Screen::Devices | Screen::DeviceDetails
            ) =>
        {
            runtime.state.previous();
        }
        KeyCode::Home | KeyCode::Char('g')
            if matches!(
                runtime.state.screen,
                Screen::Devices | Screen::DeviceDetails
            ) =>
        {
            runtime.state.first();
        }
        KeyCode::End | KeyCode::Char('G')
            if matches!(
                runtime.state.screen,
                Screen::Devices | Screen::DeviceDetails
            ) =>
        {
            runtime.state.last();
        }
        KeyCode::Enter if runtime.state.screen == Screen::Devices => runtime.state.open_details(),
        KeyCode::Char(' ') if runtime.state.screen == Screen::Devices => {
            runtime.state.toggle_selected()
        }
        KeyCode::Char('/') if runtime.state.screen == Screen::Devices => {
            runtime.state.overlay = Overlay::Search
        }
        KeyCode::Char('f') if runtime.state.screen == Screen::Devices => {
            runtime.state.cycle_filter()
        }
        KeyCode::Char('s') if runtime.state.screen == Screen::Devices => runtime.state.cycle_sort(),
        KeyCode::Char('t') if runtime.state.screen == Screen::Settings => {
            open_theme_picker(runtime);
        }
        KeyCode::Char('i') if runtime.state.screen == Screen::Settings => {
            runtime.state.cycle_icons();
            runtime.config.icons = runtime.state.icons;
        }
        KeyCode::Char('a') if runtime.state.screen == Screen::Settings => {
            runtime.state.animations = !runtime.state.animations;
            runtime.config.animations = runtime.state.animations;
            runtime.state.toast(
                format!(
                    "Animations: {}",
                    if runtime.state.animations {
                        "on"
                    } else {
                        "off"
                    }
                ),
                Severity::Info,
            );
        }
        KeyCode::Char('m') if runtime.state.screen == Screen::Settings => {
            runtime.state.cycle_scan_mode();
            runtime.config.scan_mode = runtime.state.scan_mode;
        }
        KeyCode::Char('c') if runtime.state.screen == Screen::Settings => {
            toggle_row_density(runtime);
        }
        KeyCode::Char('a') if runtime.state.screen == Screen::Alerts => {
            runtime.state.acknowledge_selected_alert();
        }
        KeyCode::Char('s') if runtime.state.screen == Screen::Settings => runtime
            .state
            .toast("Settings persist on clean exit", Severity::Info),
        KeyCode::Char('e') if runtime.state.screen == Screen::DeviceDetails => {
            runtime.state.begin_edit(Overlay::EditName)
        }
        KeyCode::Char('t') if runtime.state.screen == Screen::DeviceDetails => {
            runtime.state.begin_edit(Overlay::EditTags)
        }
        KeyCode::Char('n') if runtime.state.screen == Screen::DeviceDetails => {
            runtime.state.begin_edit(Overlay::EditNotes)
        }
        KeyCode::Char('e') => runtime.state.overlay = Overlay::Export,
        KeyCode::Char('r') => {
            if let Err(error) = runtime.start_scan() {
                runtime.state.error = Some(format!("{error:#}"));
                runtime.state.overlay = Overlay::Error;
            }
        }
        KeyCode::Char('x') if runtime.state.scan.active => runtime.cancel_scan(),
        KeyCode::Esc if runtime.state.screen == Screen::DeviceDetails => {
            runtime.state.close_details()
        }
        KeyCode::Esc => runtime.state.change_screen(Screen::Dashboard),
        _ => {}
    }
}

fn toggle_row_density(runtime: &mut AppRuntime) {
    runtime.state.compact_rows = !runtime.state.compact_rows;
    runtime.config.compact_rows = runtime.state.compact_rows;
    runtime.state.toast(
        format!(
            "Rows: {}",
            if runtime.state.compact_rows {
                "compact · one line"
            } else {
                "detailed · metadata visible"
            }
        ),
        Severity::Info,
    );
}

fn open_theme_picker(runtime: &mut AppRuntime) {
    runtime.state.theme_before_preview = Some(runtime.state.theme);
    runtime.state.palette_selected = ThemeName::ALL
        .iter()
        .position(|theme| *theme == runtime.state.theme)
        .unwrap_or(0);
    runtime.state.overlay = Overlay::Theme;
}

fn preview_theme_at(runtime: &mut AppRuntime, index: usize) {
    let index = index.min(ThemeName::ALL.len().saturating_sub(1));
    runtime.state.palette_selected = index;
    if let Some(theme_name) = ThemeName::ALL.get(index).copied() {
        runtime.state.theme = theme_name;
    }
}

fn keep_theme_preview(runtime: &mut AppRuntime) {
    if let Some(theme_name) = ThemeName::ALL.get(runtime.state.palette_selected).copied() {
        runtime.state.theme = theme_name;
        runtime.config.theme = theme_name;
        runtime.state.theme_before_preview = None;
        runtime
            .state
            .toast(format!("Theme kept: {theme_name}"), Severity::Info);
    }
    runtime.state.overlay = Overlay::None;
}

fn revert_theme_preview(runtime: &mut AppRuntime) {
    if let Some(original) = runtime.state.theme_before_preview.take() {
        runtime.state.theme = original;
    }
    runtime.state.overlay = Overlay::None;
}

fn activate_selected_setting(runtime: &mut AppRuntime) {
    match runtime.state.settings_selected {
        0 => open_theme_picker(runtime),
        1 => {
            runtime.state.cycle_icons();
            runtime.config.icons = runtime.state.icons;
        }
        2 => {
            runtime.state.animations = !runtime.state.animations;
            runtime.config.animations = runtime.state.animations;
            runtime.state.toast(
                format!(
                    "Animations: {}",
                    if runtime.state.animations {
                        "on"
                    } else {
                        "off"
                    }
                ),
                Severity::Info,
            );
        }
        3 => toggle_row_density(runtime),
        4 => {
            runtime.state.mouse = !runtime.state.mouse;
            runtime.config.mouse = runtime.state.mouse;
            runtime.state.toast(
                format!(
                    "Mouse: {} · active immediately",
                    if runtime.state.mouse { "on" } else { "off" }
                ),
                Severity::Info,
            );
        }
        5 => {
            runtime.state.cycle_scan_mode();
            runtime.config.scan_mode = runtime.state.scan_mode;
        }
        _ => {}
    }
}

fn handle_overlay_key(runtime: &mut AppRuntime, key: KeyEvent) {
    match runtime.state.overlay {
        Overlay::Search => match key.code {
            KeyCode::Esc => runtime.state.overlay = Overlay::None,
            KeyCode::Enter => runtime.state.overlay = Overlay::None,
            KeyCode::Backspace => {
                runtime.state.search.pop();
                runtime.state.selected = 0;
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                runtime.state.search.push(character);
                runtime.state.selected = 0;
            }
            _ => {}
        },
        Overlay::CommandPalette => match key.code {
            KeyCode::Esc => runtime.state.overlay = Overlay::None,
            KeyCode::Backspace => {
                runtime.state.palette_query.pop();
                runtime.state.palette_selected = 0;
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                runtime.state.palette_query.push(character);
                runtime.state.palette_selected = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let count = palette_matches(&runtime.state.palette_query).len();
                runtime.state.palette_selected =
                    (runtime.state.palette_selected + 1).min(count.saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                runtime.state.palette_selected = runtime.state.palette_selected.saturating_sub(1);
            }
            KeyCode::Home | KeyCode::Char('g') => runtime.state.palette_selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                runtime.state.palette_selected = palette_matches(&runtime.state.palette_query)
                    .len()
                    .saturating_sub(1);
            }
            KeyCode::PageDown => {
                let count = palette_matches(&runtime.state.palette_query).len();
                runtime.state.palette_selected =
                    (runtime.state.palette_selected + 5).min(count.saturating_sub(1));
            }
            KeyCode::PageUp => {
                runtime.state.palette_selected = runtime.state.palette_selected.saturating_sub(5);
            }
            KeyCode::Enter => execute_palette(runtime),
            _ => {}
        },
        Overlay::ConfirmQuit => match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                runtime.cancel_scan();
                runtime.state.should_quit = true;
            }
            KeyCode::Esc | KeyCode::Char('n') => runtime.state.overlay = Overlay::None,
            _ => {}
        },
        Overlay::EditName | Overlay::EditTags | Overlay::EditNotes => match key.code {
            KeyCode::Esc => {
                runtime.state.editor.clear();
                runtime.state.overlay = Overlay::None;
            }
            KeyCode::Enter => {
                runtime.state.apply_editor();
                runtime.state.editor.clear();
                runtime.state.overlay = Overlay::None;
            }
            KeyCode::Backspace => {
                runtime.state.editor.pop();
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && runtime.state.editor.chars().count() < 2_000 =>
            {
                runtime.state.editor.push(character);
            }
            _ => {}
        },
        Overlay::Interface => match key.code {
            KeyCode::Esc => runtime.state.overlay = Overlay::None,
            KeyCode::Down | KeyCode::Char('j') => {
                runtime.state.palette_selected = (runtime.state.palette_selected + 1)
                    .min(runtime.state.interfaces.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                runtime.state.palette_selected = runtime.state.palette_selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(interface) = runtime
                    .state
                    .interfaces
                    .get(runtime.state.palette_selected)
                    .cloned()
                {
                    runtime.config.interface = Some(interface.name.clone());
                    runtime.state.active_interface = Some(interface);
                    runtime
                        .state
                        .toast("Active interface updated", Severity::Info);
                }
                runtime.state.overlay = Overlay::None;
            }
            _ => {}
        },
        Overlay::ScanMode => match key.code {
            KeyCode::Esc => runtime.state.overlay = Overlay::None,
            KeyCode::Down | KeyCode::Char('j') => {
                runtime.state.palette_selected = (runtime.state.palette_selected + 1).min(4);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                runtime.state.palette_selected = runtime.state.palette_selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                let modes = [
                    ScanMode::Quick,
                    ScanMode::Normal,
                    ScanMode::Deep,
                    ScanMode::Watch,
                    ScanMode::Passive,
                ];
                if let Some(mode) = modes.get(runtime.state.palette_selected).copied() {
                    runtime.state.scan_mode = mode;
                    runtime.config.scan_mode = mode;
                    runtime
                        .state
                        .toast(format!("Scan mode: {}", mode.label()), Severity::Info);
                }
                runtime.state.overlay = Overlay::None;
            }
            _ => {}
        },
        Overlay::Theme => match key.code {
            KeyCode::Esc => revert_theme_preview(runtime),
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j') => {
                preview_theme_at(runtime, runtime.state.palette_selected + 1);
            }
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k') => {
                preview_theme_at(runtime, runtime.state.palette_selected.saturating_sub(1));
            }
            KeyCode::Home | KeyCode::Char('g') => preview_theme_at(runtime, 0),
            KeyCode::End | KeyCode::Char('G') => {
                preview_theme_at(runtime, ThemeName::ALL.len().saturating_sub(1));
            }
            KeyCode::PageDown => {
                preview_theme_at(runtime, runtime.state.palette_selected + 5);
            }
            KeyCode::PageUp => {
                preview_theme_at(runtime, runtime.state.palette_selected.saturating_sub(5));
            }
            KeyCode::Enter | KeyCode::Char(' ') => keep_theme_preview(runtime),
            _ => {}
        },
        Overlay::Help | Overlay::Error | Overlay::Export | Overlay::Filter | Overlay::Sort => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter) {
                runtime.state.overlay = Overlay::None;
            }
        }
        Overlay::None => {}
    }
}

fn execute_palette(runtime: &mut AppRuntime) {
    let matches = palette_matches(&runtime.state.palette_query);
    let Some((command, _, _)) = matches.get(runtime.state.palette_selected).copied() else {
        return;
    };
    runtime.state.overlay = Overlay::None;
    match command {
        PaletteCommand::Dashboard => runtime.state.change_screen(Screen::Dashboard),
        PaletteCommand::Devices => runtime.state.change_screen(Screen::Devices),
        PaletteCommand::History => runtime.state.change_screen(Screen::History),
        PaletteCommand::Compare => runtime.state.change_screen(Screen::Compare),
        PaletteCommand::Alerts => runtime.state.change_screen(Screen::Alerts),
        PaletteCommand::Logs => runtime.state.change_screen(Screen::Logs),
        PaletteCommand::Settings => runtime.state.change_screen(Screen::Settings),
        PaletteCommand::Rescan => {
            if let Err(error) = runtime.start_scan() {
                runtime.state.error = Some(format!("{error:#}"));
                runtime.state.overlay = Overlay::Error;
            }
        }
        PaletteCommand::CancelScan => runtime.cancel_scan(),
        PaletteCommand::Changed => {
            runtime.state.filter = DeviceFilter::Changed;
            runtime.state.change_screen(Screen::Devices);
        }
        PaletteCommand::Filter => {
            runtime.state.change_screen(Screen::Devices);
            runtime.state.cycle_filter();
        }
        PaletteCommand::Sort => {
            runtime.state.change_screen(Screen::Devices);
            runtime.state.cycle_sort();
        }
        PaletteCommand::Interface => {
            runtime.state.palette_selected = runtime
                .state
                .active_interface
                .as_ref()
                .and_then(|active| {
                    runtime
                        .state
                        .interfaces
                        .iter()
                        .position(|interface| interface.name == active.name)
                })
                .unwrap_or(0);
            runtime.state.overlay = Overlay::Interface;
        }
        PaletteCommand::ScanMode => {
            runtime.state.palette_selected = match runtime.state.scan_mode {
                ScanMode::Quick => 0,
                ScanMode::Normal => 1,
                ScanMode::Deep => 2,
                ScanMode::Watch => 3,
                ScanMode::Passive => 4,
            };
            runtime.state.overlay = Overlay::ScanMode;
        }
        PaletteCommand::Theme => {
            open_theme_picker(runtime);
        }
        PaletteCommand::Icons => {
            runtime.state.cycle_icons();
            runtime.config.icons = runtime.state.icons;
        }
        PaletteCommand::Animations => {
            runtime.state.animations = !runtime.state.animations;
            runtime.config.animations = runtime.state.animations;
            runtime.state.toast(
                format!(
                    "Animations: {}",
                    if runtime.state.animations {
                        "on"
                    } else {
                        "off"
                    }
                ),
                Severity::Info,
            );
        }
        PaletteCommand::CompactRows => toggle_row_density(runtime),
        PaletteCommand::Mouse => {
            runtime.state.mouse = !runtime.state.mouse;
            runtime.config.mouse = runtime.state.mouse;
            runtime.state.toast(
                format!(
                    "Mouse: {} · active immediately",
                    if runtime.state.mouse { "on" } else { "off" }
                ),
                Severity::Info,
            );
        }
        PaletteCommand::Export => runtime.state.overlay = Overlay::Export,
        PaletteCommand::Help => runtime.state.overlay = Overlay::Help,
        PaletteCommand::Doctor => {
            runtime
                .state
                .toast("Run `wireseer doctor` outside the TUI", Severity::Info);
        }
        PaletteCommand::Quit => {
            if runtime.state.scan.active {
                runtime.state.overlay = Overlay::ConfirmQuit;
            } else {
                runtime.state.should_quit = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::{
        Command,
        style::{Color as CrosstermColor, SetForegroundColor},
    };
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use crate::{app::demo_state, config::Config};

    use super::*;

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn explicit_color_themes_override_no_color_but_monochrome_does_not() {
        let _restore = ColorOutputGuard::enter(ThemeName::WireseerDark);
        let mut colored = String::new();
        SetForegroundColor(CrosstermColor::Red)
            .write_ansi(&mut colored)
            .expect("write colored ANSI");
        assert!(
            colored.contains("38;"),
            "missing color sequence: {colored:?}"
        );

        sync_terminal_color_output(ThemeName::Monochrome);
        let mut monochrome = String::new();
        SetForegroundColor(CrosstermColor::Red)
            .write_ansi(&mut monochrome)
            .expect("write monochrome ANSI");
        assert!(
            !monochrome.contains("38;"),
            "monochrome emitted a color sequence: {monochrome:?}"
        );
    }

    #[test]
    fn wide_dashboard_contains_product_identity_and_metrics() {
        let backend = TestBackend::new(150, 38);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = demo_state(&Config::default());
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("WIRESEER"));
        assert!(text.contains(BRAND_TRACE_UNICODE));
        assert!(text.contains("DEVICE ACTIVITY"));
        assert!(text.contains("RECENT EVENTS"));
        assert!(text.contains("Home NAS"));
        for page in [
            "1 Dashboard",
            "2 Devices",
            "3 History",
            "4 Compare",
            "5 Alerts",
            "6 Logs",
            "7 Settings",
        ] {
            assert!(text.contains(page), "missing footer page: {page}");
        }
        for global in ["q Quit", "? Help", "Ctrl+P Commands", "r Scan"] {
            assert!(text.contains(global), "missing global action: {global}");
        }
    }

    #[test]
    fn brand_trace_has_a_safe_ascii_fallback() {
        assert_eq!(brand_trace(IconMode::Unicode), BRAND_TRACE_UNICODE);
        assert_eq!(brand_trace(IconMode::Nerd), BRAND_TRACE_UNICODE);
        assert_eq!(brand_trace(IconMode::Ascii), BRAND_TRACE_ASCII);
        assert!(BRAND_TRACE_ASCII.is_ascii());
        assert_eq!(brand_pulse(IconMode::Unicode), BRAND_PULSE_UNICODE);
        assert_eq!(brand_pulse(IconMode::Ascii), BRAND_PULSE_ASCII);
        assert!(BRAND_PULSE_ASCII.is_ascii());
    }

    #[test]
    fn device_workspace_preserves_the_table_and_caps_the_inspector() {
        let (table, inspector) =
            split_device_workspace(Rect::new(0, 0, 100, 20), LayoutClass::Standard);
        assert_eq!(table.width, 100);
        assert!(inspector.is_none());

        let (table, inspector) =
            split_device_workspace(Rect::new(0, 0, 160, 12), LayoutClass::Wide);
        assert_eq!(table.width, 160);
        assert!(inspector.is_none());

        let (table, inspector) =
            split_device_workspace(Rect::new(0, 0, 140, 20), LayoutClass::Wide);
        assert_eq!(table.width, 140);
        assert!(inspector.is_none());

        let (table, inspector) =
            split_device_workspace(Rect::new(0, 0, 180, 20), LayoutClass::Wide);
        let inspector = inspector.expect("wide inspector");
        assert_eq!(table.width + inspector.width, 180);
        assert_eq!(inspector.width, 60);
        assert!(table.width >= 112);

        let (table, inspector) =
            split_device_workspace(Rect::new(0, 0, 500, 20), LayoutClass::Wide);
        let inspector = inspector.expect("very wide inspector");
        assert_eq!(inspector.width, 64);
        assert_eq!(table.width, 436);
    }

    #[test]
    fn device_table_columns_expand_and_reveal_by_available_width() {
        fn footprint(layout: DeviceTableLayout) -> u16 {
            let mut width = 4 + 2 + layout.identity_width + layout.address_width;
            let mut columns = 3_u16;
            if layout.show_vendor {
                width += layout.vendor_width;
                columns += 1;
            }
            if layout.show_latency {
                width += layout.latency_width;
                columns += 1;
            }
            if layout.show_services {
                width += layout.services_width;
                columns += 1;
            }
            if layout.show_confidence {
                width += layout.confidence_width;
                columns += 1;
            }
            width + columns.saturating_sub(1)
        }

        let mut config = Config::default();
        config.visible_columns.push("confidence".into());
        let state = demo_state(&config);
        let compact = device_table_layout(70, &state);
        assert!(!compact.show_services);
        assert!(!compact.show_vendor);
        assert!(!compact.show_confidence);
        assert_eq!(footprint(compact), 70);

        let standard = device_table_layout(100, &state);
        assert!(standard.show_services);
        assert!(!standard.show_vendor);
        assert!(!standard.show_confidence);
        assert_eq!(footprint(standard), 100);

        let wide = device_table_layout(160, &state);
        assert!(wide.show_services);
        assert!(wide.show_vendor);
        assert!(wide.show_confidence);
        assert_eq!(footprint(wide), 160);

        let very_wide = device_table_layout(430, &state);
        assert!(very_wide.identity_width > wide.identity_width);
        assert!(very_wide.address_width > wide.address_width);
        assert!(very_wide.vendor_width > wide.vendor_width);
        assert!(very_wide.services_width > wide.services_width);
        assert_eq!(footprint(very_wide), 430);
    }

    #[test]
    fn compact_devices_preserve_identity_and_address() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.screen = Screen::Devices;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("IDENTITY"));
        assert!(text.contains("ADDRESS"));
        assert!(text.contains("192.0.2.1"));
        assert!(!text.contains("CONF."));
        for global in ["q Quit", "? Help", "Ctrl+P Commands", "r Scan"] {
            assert!(text.contains(global), "missing compact action: {global}");
        }
    }

    #[test]
    fn standard_footer_keeps_global_actions_ahead_of_context() {
        let backend = TestBackend::new(90, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.screen = Screen::Devices;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        for global in ["q Quit", "? Help", "Ctrl+P Commands", "r Scan"] {
            assert!(text.contains(global), "missing standard action: {global}");
        }
        assert!(text.contains("Enter Details"));

        state.scan.active = true;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render active scan");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("x Cancel scan"));
        assert!(!text.contains("r Scan"));
        assert!(text.contains("q Quit"));
    }

    #[test]
    fn wide_devices_expand_metadata_and_use_safe_unicode_icons() {
        let backend = TestBackend::new(220, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.screen = Screen::Devices;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        let header = text
            .lines()
            .find(|line| line.contains("IDENTITY") && line.contains("ADDRESS"))
            .expect("device table header");
        assert!(header.find("ADDRESS").expect("address column") < 70);
        assert!(text.contains("▦ Home NAS"));
        assert!(text.contains("VENDOR"));
        assert!(text.contains("MAC 02:00:00:00:00:14"));
        assert!(text.contains("22/tcp"));
        assert!(text.contains("NAS"));
        assert!(text.contains("100%"));
    }

    #[test]
    fn very_wide_devices_spread_columns_instead_of_truncating_them_early() {
        let backend = TestBackend::new(430, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.screen = Screen::Devices;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        let header = text
            .lines()
            .find(|line| {
                line.contains("IDENTITY")
                    && line.contains("ADDRESS")
                    && line.contains("VENDOR")
                    && line.contains("SERVICES")
            })
            .expect("very wide device table header");
        let address = header.find("ADDRESS").expect("address");
        let vendor = header.find("VENDOR").expect("vendor");
        let services = header.find("SERVICES").expect("services");
        assert!(address > 80, "identity did not expand: {header}");
        assert!(vendor > address + 40, "address did not expand: {header}");
        assert!(services > vendor + 60, "vendor did not expand: {header}");
        assert!(text.contains("MAC 02:00:00:00:00:14"));
    }

    #[test]
    fn compact_row_density_removes_only_the_metadata_line() {
        let backend = TestBackend::new(220, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config {
            compact_rows: true,
            ..Config::default()
        });
        state.screen = Screen::Devices;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("▦ Home NAS"));
        assert!(text.contains("SSH HTTPS SMB"));
        assert!(!text.contains("MAC 02:00:00:00:00:14"));
        assert!(!text.contains("22/tcp"));
    }

    #[test]
    fn mouse_footer_clicks_switch_pages_and_respect_the_setting() {
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(footer_screen_at(area, 16, 28), Some(Screen::Devices));

        let config = Config::default();
        let state = demo_state(&config);
        let mut runtime = AppRuntime::new(state, config);
        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 16,
                row: 28,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(runtime.state.screen, Screen::Devices);

        runtime.state.change_screen(Screen::Dashboard);
        runtime.state.mouse = false;
        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 16,
                row: 28,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(runtime.state.screen, Screen::Dashboard);
    }

    #[test]
    fn mouse_selects_devices_and_opens_the_selected_device() {
        let area = Rect::new(0, 0, 100, 30);
        let config = Config::default();
        let mut state = demo_state(&config);
        state.screen = Screen::Devices;
        let mut runtime = AppRuntime::new(state, config);
        let (_, table) = device_regions(area, LayoutClass::Standard).expect("device regions");
        let column = table.x.saturating_add(2);
        let row = table.y.saturating_add(5);
        assert_eq!(device_row_at(&runtime.state, area, column, row), Some(1));

        let click = || {
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        handle_terminal_event(&mut runtime, click(), area);
        assert_eq!(runtime.state.selected, 1);
        assert_eq!(runtime.state.screen, Screen::Devices);

        handle_terminal_event(&mut runtime, click(), area);
        assert_eq!(runtime.state.screen, Screen::DeviceDetails);
    }

    #[test]
    fn mouse_changes_settings_and_runs_palette_commands() {
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(setting_at(area, 3, 9), Some(4));

        let config = Config::default();
        let mut state = demo_state(&config);
        state.screen = Screen::Settings;
        let mut runtime = AppRuntime::new(state, config);
        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 9,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert!(!runtime.state.mouse);
        assert!(!runtime.config.mouse);
        assert!(
            runtime
                .state
                .toast
                .as_ref()
                .is_some_and(|toast| toast.message.contains("active immediately"))
        );

        runtime.state.mouse = true;
        runtime.config.mouse = true;
        runtime.state.overlay = Overlay::CommandPalette;
        runtime.state.palette_selected = 0;
        assert_eq!(palette_index_at(&runtime.state, area, 18, 8), Some(1));
        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 18,
                row: 8,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(runtime.state.screen, Screen::Devices);
        assert_eq!(runtime.state.overlay, Overlay::None);
    }

    #[test]
    fn settings_arrows_and_enter_change_the_selected_row() {
        let config = Config::default();
        let state = demo_state(&config);
        let mut runtime = AppRuntime::new(state, config);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE),
        );
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.settings_selected, 1);

        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.icons, IconMode::Ascii);
        assert_eq!(runtime.config.icons, IconMode::Ascii);

        runtime.state.settings_selected = 3;
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(runtime.state.compact_rows);
        assert!(runtime.config.compact_rows);

        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.settings_selected, 5);
        let previous_mode = runtime.state.scan_mode;
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        assert_ne!(runtime.state.scan_mode, previous_mode);
    }

    #[test]
    fn theme_picker_opens_from_settings_and_applies_explicit_choice() {
        let config = Config::default();
        let state = demo_state(&config);
        let mut runtime = AppRuntime::new(state, config);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE),
        );
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.overlay, Overlay::Theme);
        assert_eq!(runtime.state.theme, ThemeName::WireseerDark);

        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.theme, ThemeName::CatppuccinMocha);
        assert_eq!(runtime.config.theme, ThemeName::WireseerDark);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, &runtime.state))
            .expect("render live preview");
        assert_eq!(
            terminal.backend().buffer()[(0, 0)].bg,
            Theme::named(ThemeName::CatppuccinMocha).background
        );
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.overlay, Overlay::None);
        assert_eq!(runtime.state.theme, ThemeName::CatppuccinMocha);
        assert_eq!(runtime.config.theme, ThemeName::CatppuccinMocha);
        assert_eq!(runtime.state.theme_before_preview, None);

        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.overlay, Overlay::Theme);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.theme, ThemeName::CatppuccinLatte);
        assert_eq!(runtime.config.theme, ThemeName::CatppuccinMocha);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.theme, ThemeName::CatppuccinMocha);
        assert_eq!(runtime.config.theme, ThemeName::CatppuccinMocha);
        assert_eq!(runtime.state.theme_before_preview, None);
    }

    #[test]
    fn theme_picker_renders_all_choices_and_accepts_mouse() {
        let area = Rect::new(0, 0, 100, 30);
        let config = Config::default();
        let mut state = demo_state(&config);
        state.screen = Screen::Settings;
        state.overlay = Overlay::Theme;
        state.palette_selected = 0;
        state.theme_before_preview = Some(state.theme);

        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render theme picker");
        let text = buffer_text(terminal.backend().buffer());
        for name in ThemeName::ALL {
            assert!(text.contains(name.label()), "missing theme: {name}");
        }
        assert!(text.contains("Enter Keep"));
        assert!(text.contains("Live preview"));
        assert!(text.contains("saved"));
        assert!(text.contains("› ◈  Wireseer Dark"));
        assert!(text.contains("· ◆  Catppuccin Mocha"));
        assert!(!text.contains('█'));

        let acid_index = ThemeName::ALL
            .iter()
            .position(|theme| *theme == ThemeName::Acid)
            .expect("acid index");
        let dialog = centered_rect(86, 17, area);
        let mut runtime = AppRuntime::new(state, config);
        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: dialog.x.saturating_add(2),
                row: dialog
                    .y
                    .saturating_add(2)
                    .saturating_add(u16::try_from(acid_index).expect("theme index")),
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(runtime.state.theme, ThemeName::Acid);
        assert_eq!(runtime.config.theme, ThemeName::Acid);
        assert_eq!(runtime.state.overlay, Overlay::None);
    }

    #[test]
    fn theme_picker_scrolls_to_the_selected_theme_in_compact_terminals() {
        let area = Rect::new(0, 0, 70, 18);
        let mut state = demo_state(&Config::default());
        state.screen = Screen::Settings;
        state.overlay = Overlay::Theme;
        state.palette_selected = ThemeName::ALL.len() - 1;

        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render compact theme picker");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Color-Blind Friendly"));
        assert!(text.contains("11/11"));

        let dialog = centered_rect(68, 16, area);
        assert_eq!(
            theme_picker_index_at(&state, area, dialog.x + 2, dialog.y + 2),
            Some(1)
        );
    }

    #[test]
    fn theme_picker_mouse_wheel_previews_and_right_click_reverts() {
        let area = Rect::new(0, 0, 100, 30);
        let config = Config::default();
        let mut state = demo_state(&config);
        state.screen = Screen::Settings;
        let mut runtime = AppRuntime::new(state, config);
        open_theme_picker(&mut runtime);

        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 50,
                row: 15,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(runtime.state.theme, ThemeName::CatppuccinMocha);
        assert_eq!(runtime.config.theme, ThemeName::WireseerDark);

        handle_terminal_event(
            &mut runtime,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: 50,
                row: 15,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(runtime.state.overlay, Overlay::None);
        assert_eq!(runtime.state.theme, ThemeName::WireseerDark);
        assert_eq!(runtime.config.theme, ThemeName::WireseerDark);
    }

    #[test]
    fn command_palette_opens_theme_picker() {
        let config = Config::default();
        let state = demo_state(&config);
        let mut runtime = AppRuntime::new(state, config);
        runtime.state.overlay = Overlay::CommandPalette;
        runtime.state.palette_query = "choose theme".into();
        execute_palette(&mut runtime);
        assert_eq!(runtime.state.overlay, Overlay::Theme);
        assert_eq!(runtime.state.palette_selected, 0);
        assert_eq!(
            runtime.state.theme_before_preview,
            Some(ThemeName::WireseerDark)
        );
    }

    #[test]
    fn icon_modes_and_cell_truncation_are_stable() {
        assert_eq!(device_icon(DeviceType::Computer, IconMode::Unicode), "▣");
        assert_eq!(device_icon(DeviceType::Computer, IconMode::Ascii), "[PC]");
        assert_ne!(
            device_icon(DeviceType::Computer, IconMode::Nerd),
            device_icon(DeviceType::Computer, IconMode::Unicode)
        );
        assert_eq!(
            truncate_cell("GD Midea Air-Conditioning", 12),
            "GD Midea Ai…"
        );
    }

    #[test]
    fn small_terminal_renders_safe_message() {
        let backend = TestBackend::new(54, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = demo_state(&Config::default());
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("needs a little more room"));
        assert!(text.contains(BRAND_TRACE_UNICODE));
        assert!(text.contains("54 × 12"));

        let backend = TestBackend::new(54, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = demo_state(&Config {
            icons: IconMode::Ascii,
            ..Config::default()
        });
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render ASCII");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains(BRAND_TRACE_ASCII));
        assert!(text.contains("54 x 12"));
        assert!(!text.contains(BRAND_TRACE_UNICODE));
    }

    #[test]
    fn command_palette_filters_actions() {
        let matches = palette_matches("scan mode");
        assert_eq!(
            matches.first().map(|item| item.0),
            Some(PaletteCommand::ScanMode)
        );
    }

    #[test]
    fn help_lists_every_screen_and_current_shortcuts() {
        let backend = TestBackend::new(110, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.screen = Screen::Devices;
        state.overlay = Overlay::Help;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        for page in [
            "Dashboard",
            "Devices",
            "History",
            "Compare",
            "Alerts",
            "Logs",
            "Settings",
        ] {
            assert!(text.contains(page), "missing help page: {page}");
        }
        assert!(text.contains("Open device details"));
        assert!(text.contains("First / last device"));
        assert!(text.contains("Search every command"));
        assert!(text.contains("Start a network scan"));
        assert!(text.contains("Quit safely"));
    }

    #[test]
    fn help_keeps_full_descriptions_and_a_column_gutter_when_width_allows() {
        let backend = TestBackend::new(101, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.overlay = Overlay::Help;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        for description in [
            "Overview, activity, and health",
            "Inventory and device inspector",
            "Discovery and change timeline",
            "Scan and baseline differences",
            "Appearance and discovery",
            "Click / wheel / right-click back",
        ] {
            assert!(
                text.contains(description),
                "help shortened available text: {description}\n{text}"
            );
        }
        let first_row = text
            .lines()
            .find(|line| line.contains("Overview, activity, and health"))
            .expect("first help row");
        assert!(
            first_row.contains(" │ "),
            "missing column gutter: {first_row}"
        );
        assert!(first_row.contains("Quit safely"));
    }

    #[test]
    fn narrow_tall_help_stacks_sections_without_shortening_them() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.overlay = Overlay::Help;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        let lines = text.lines().collect::<Vec<_>>();
        let screens = lines
            .iter()
            .position(|line| line.contains("SCREENS"))
            .expect("screens section");
        let actions = lines
            .iter()
            .position(|line| line.contains("DASHBOARD / GLOBAL"))
            .expect("actions section");
        assert!(
            actions > screens + 7,
            "help sections were not stacked:\n{text}"
        );
        assert!(text.contains("Inventory and device inspector"));
        assert!(text.contains("Click / wheel / right-click back"));
    }

    #[test]
    fn short_standard_help_uses_a_compact_divider_before_truncating_text() {
        let backend = TestBackend::new(90, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.overlay = Overlay::Help;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Overview, activity, and health"));
        assert!(text.contains("Inventory and device inspector"));
        assert!(text.contains(" │q"));
    }

    #[test]
    fn command_palette_scrolls_to_all_commands_and_opens_settings() {
        let matches = palette_matches("");
        assert_eq!(matches.len(), 23);
        for command in [
            PaletteCommand::Dashboard,
            PaletteCommand::Logs,
            PaletteCommand::Settings,
            PaletteCommand::Help,
            PaletteCommand::Quit,
        ] {
            assert!(
                matches.iter().any(|item| item.0 == command),
                "missing palette command: {command:?}"
            );
        }

        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = demo_state(&Config::default());
        state.overlay = Overlay::CommandPalette;
        state.palette_selected = matches.len() - 1;
        terminal
            .draw(|frame| render(frame, &state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Quit Wireseer"));
        assert!(text.contains("23/23"));

        let config = Config::default();
        let state = demo_state(&config);
        let mut runtime = AppRuntime::new(state, config);
        runtime.state.overlay = Overlay::CommandPalette;
        runtime.state.palette_query = "open settings".into();
        execute_palette(&mut runtime);
        assert_eq!(runtime.state.screen, Screen::Settings);
        assert_eq!(runtime.state.overlay, Overlay::None);
    }

    #[test]
    fn alerts_navigation_and_acknowledgement_are_visible() {
        let config = Config::default();
        let mut state = demo_state(&config);
        state.screen = Screen::Alerts;
        let mut runtime = AppRuntime::new(state, config);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.selected, 1);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        assert!(runtime.state.alerts[1].acknowledged);

        let backend = TestBackend::new(110, 28);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, &runtime.state))
            .expect("render");
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("ACKNOWLEDGED"));
        assert!(text.contains("q Quit"));
    }

    #[test]
    fn shortcuts_change_view_and_trap_palette_input() {
        let config = Config::default();
        let state = demo_state(&config);
        let mut runtime = AppRuntime::new(state, config);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.screen, Screen::Devices);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        );
        assert_eq!(runtime.state.overlay, Overlay::CommandPalette);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.palette_query, "q");
        assert!(!runtime.state.should_quit);
        handle_key(
            &mut runtime,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert_eq!(runtime.state.overlay, Overlay::None);
    }
}
