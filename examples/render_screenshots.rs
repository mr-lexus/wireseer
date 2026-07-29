//! Render release screenshots from the real TUI renderer and its network-free demo state.
//!
//! Run with:
//!
//! ```text
//! cargo run --example render_screenshots -- docs/screenshots
//! ```

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;
use wireseer_tui::{
    app::{Screen, demo_state},
    config::{Config, IconMode, ThemeName},
    tui,
};

const COLUMNS: u16 = 160;
const ROWS: u16 = 40;
const CELL_WIDTH: u16 = 9;
const CELL_HEIGHT: u16 = 20;
const FRAME_PADDING: u16 = 28;
const DEFAULT_BACKGROUND: &str = "#070a0f";
const DEFAULT_FOREGROUND: &str = "#ddeef4";

fn main() -> Result<()> {
    let destination = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/screenshots"));
    fs::create_dir_all(&destination)
        .with_context(|| format!("create {}", destination.display()))?;

    render_screen(&destination, "dashboard.svg", Screen::Dashboard)?;
    render_screen(&destination, "devices.svg", Screen::Devices)?;
    render_screen(&destination, "history.svg", Screen::History)?;

    println!(
        "Rendered mock-data screenshots in {}",
        destination.display()
    );
    Ok(())
}

fn render_screen(destination: &Path, filename: &str, screen: Screen) -> Result<()> {
    let config = Config {
        theme: ThemeName::WireseerDark,
        icons: IconMode::Unicode,
        animations: false,
        compact_rows: false,
        ..Config::default()
    };
    let mut state = demo_state(&config);
    state.screen = screen;
    state.scan.active = false;
    state.selected = state
        .devices
        .iter()
        .position(|device| device.display_name() == "Home NAS")
        .unwrap_or_default();
    let screenshot_time = Utc
        .with_ymd_and_hms(2026, 7, 26, 12, 42, 0)
        .single()
        .expect("valid screenshot timestamp");
    for (event, minutes_ago) in state.events.iter_mut().zip([0_i64, 3, 7]) {
        event.occurred_at = screenshot_time - chrono::Duration::minutes(minutes_ago);
    }

    let backend = TestBackend::new(COLUMNS, ROWS);
    let mut terminal = Terminal::new(backend).context("create screenshot terminal")?;
    terminal
        .draw(|frame| tui::render(frame, &state))
        .context("render screenshot")?;

    let title = format!("Wireseer {} screen with mock network data", screen.title());
    let svg = buffer_to_svg(terminal.backend().buffer(), &title);
    let output = destination.join(filename);
    fs::write(&output, svg).with_context(|| format!("write {}", output.display()))
}

fn buffer_to_svg(buffer: &Buffer, title: &str) -> String {
    let area = *buffer.area();
    let terminal_width = area.width * CELL_WIDTH;
    let terminal_height = area.height * CELL_HEIGHT;
    let width = terminal_width + FRAME_PADDING * 2;
    let height = terminal_height + FRAME_PADDING * 2;
    let escaped_title = escape_xml(title);
    let mut backgrounds = String::new();
    let mut glyphs = String::new();

    for y in 0..area.height {
        let mut x = 0;
        while x < area.width {
            let Some(cell) = buffer.cell((x, y)) else {
                x += 1;
                continue;
            };
            let style = SvgStyle::from_cell(cell);

            let pixel_y = FRAME_PADDING + y * CELL_HEIGHT;
            if cell.modifier.contains(Modifier::HIDDEN) {
                x += 1;
                continue;
            }
            let run_start = x;
            let mut run = String::new();
            while x < area.width {
                let Some(next) = buffer.cell((x, y)) else {
                    break;
                };
                if next.modifier.contains(Modifier::HIDDEN) || SvgStyle::from_cell(next) != style {
                    break;
                }
                let symbol = next.symbol();
                run.push_str(symbol);
                x += u16::try_from(symbol.width().max(1)).unwrap_or(1);
            }
            let run_width = (x - run_start) * CELL_WIDTH;
            let pixel_x = FRAME_PADDING + run_start * CELL_WIDTH;
            if style.background != DEFAULT_BACKGROUND {
                backgrounds.push_str(&format!(
                    "    <rect x=\"{pixel_x}\" y=\"{pixel_y}\" width=\"{run_width}\" height=\"{CELL_HEIGHT}\" fill=\"{}\"/>\n",
                    style.background
                ));
            }
            let run = run.trim_end();
            if run.trim().is_empty() {
                continue;
            }
            let italic = if style.italic {
                " font-style=\"italic\""
            } else {
                ""
            };
            let decoration = if style.underlined {
                " text-decoration=\"underline\""
            } else if style.crossed_out {
                " text-decoration=\"line-through\""
            } else {
                ""
            };
            let baseline = pixel_y + 15;
            glyphs.push_str(&format!(
                "    <text x=\"{pixel_x}\" y=\"{baseline}\" fill=\"{}\" font-weight=\"{}\" opacity=\"{}\"{italic}{decoration}>{}</text>\n",
                style.foreground,
                style.weight,
                style.opacity,
                escape_xml(run)
            ));
        }
    }

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-labelledby=\"title desc\">\n  <title id=\"title\">{escaped_title}</title>\n  <desc id=\"desc\">Generated from Wireseer's built-in demo state. All hostnames, addresses, identifiers, events, and alerts are mock data; no network scan was performed.</desc>\n  <rect width=\"{width}\" height=\"{height}\" rx=\"18\" fill=\"#030508\"/>\n  <rect x=\"12\" y=\"12\" width=\"{}\" height=\"{}\" rx=\"12\" fill=\"{DEFAULT_BACKGROUND}\" stroke=\"#283a46\" stroke-width=\"2\"/>\n  <g font-family=\"SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace\" font-size=\"15\">\n{backgrounds}{glyphs}  </g>\n</svg>\n",
        width - 24,
        height - 24,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SvgStyle {
    foreground: String,
    background: String,
    weight: &'static str,
    opacity: &'static str,
    italic: bool,
    underlined: bool,
    crossed_out: bool,
}

impl SvgStyle {
    fn from_cell(cell: &ratatui::buffer::Cell) -> Self {
        let mut foreground = color_to_hex(cell.fg, DEFAULT_FOREGROUND);
        let mut background = color_to_hex(cell.bg, DEFAULT_BACKGROUND);
        if cell.modifier.contains(Modifier::REVERSED) {
            std::mem::swap(&mut foreground, &mut background);
        }
        Self {
            foreground,
            background,
            weight: if cell.modifier.contains(Modifier::BOLD) {
                "700"
            } else {
                "400"
            },
            opacity: if cell.modifier.contains(Modifier::DIM) {
                "0.64"
            } else {
                "1"
            },
            italic: cell.modifier.contains(Modifier::ITALIC),
            underlined: cell.modifier.contains(Modifier::UNDERLINED),
            crossed_out: cell.modifier.contains(Modifier::CROSSED_OUT),
        }
    }
}

fn color_to_hex(color: Color, reset: &'static str) -> String {
    match color {
        Color::Reset => reset.into(),
        Color::Black => "#000000".into(),
        Color::Red => "#cc5555".into(),
        Color::Green => "#55cc88".into(),
        Color::Yellow => "#d9b95b".into(),
        Color::Blue => "#5f87d7".into(),
        Color::Magenta => "#b86fd8".into(),
        Color::Cyan => "#56c9d8".into(),
        Color::Gray => "#b8c0c8".into(),
        Color::DarkGray => "#66717b".into(),
        Color::LightRed => "#ff718a".into(),
        Color::LightGreen => "#75e6ad".into(),
        Color::LightYellow => "#f6d776".into(),
        Color::LightBlue => "#7aa7f8".into(),
        Color::LightMagenta => "#d99af5".into(),
        Color::LightCyan => "#7ce7f5".into(),
        Color::White => "#ffffff".into(),
        Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
        Color::Indexed(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> String {
    const ANSI: [&str; 16] = [
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ];
    if index < 16 {
        return ANSI[index as usize].into();
    }
    if index < 232 {
        let value = index - 16;
        let red = value / 36;
        let green = (value % 36) / 6;
        let blue = value % 6;
        let component = |part: u8| if part == 0 { 0 } else { 55 + part * 40 };
        return format!(
            "#{:02x}{:02x}{:02x}",
            component(red),
            component(green),
            component(blue)
        );
    }
    let gray = 8 + (index - 232) * 10;
    format!("#{gray:02x}{gray:02x}{gray:02x}")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
