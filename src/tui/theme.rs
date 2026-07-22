use ratatui::style::Color;

use crate::config::{IconMode, ThemeName};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub surface_active: Color,
    pub border: Color,
    pub border_focused: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
    pub selection: Color,
    pub selection_text: Color,
    pub chart_primary: Color,
    pub chart_secondary: Color,
}

impl Theme {
    #[must_use]
    pub const fn named(name: ThemeName) -> Self {
        match name {
            ThemeName::LanternDark => Self {
                background: Color::Rgb(12, 16, 21),
                surface: Color::Rgb(20, 26, 33),
                surface_active: Color::Rgb(29, 39, 48),
                border: Color::Rgb(57, 70, 82),
                border_focused: Color::Rgb(63, 189, 191),
                text_primary: Color::Rgb(225, 232, 237),
                text_secondary: Color::Rgb(157, 172, 184),
                text_muted: Color::Rgb(96, 111, 123),
                accent: Color::Rgb(63, 189, 191),
                success: Color::Rgb(103, 190, 126),
                warning: Color::Rgb(224, 175, 90),
                danger: Color::Rgb(224, 102, 108),
                info: Color::Rgb(103, 159, 214),
                selection: Color::Rgb(35, 83, 91),
                selection_text: Color::Rgb(240, 250, 250),
                chart_primary: Color::Rgb(63, 189, 191),
                chart_secondary: Color::Rgb(103, 159, 214),
            },
            ThemeName::CatppuccinMocha => Self {
                background: Color::Rgb(30, 30, 46),
                surface: Color::Rgb(36, 39, 58),
                surface_active: Color::Rgb(49, 50, 68),
                border: Color::Rgb(88, 91, 112),
                border_focused: Color::Rgb(137, 180, 250),
                text_primary: Color::Rgb(205, 214, 244),
                text_secondary: Color::Rgb(166, 173, 200),
                text_muted: Color::Rgb(108, 112, 134),
                accent: Color::Rgb(137, 180, 250),
                success: Color::Rgb(166, 227, 161),
                warning: Color::Rgb(249, 226, 175),
                danger: Color::Rgb(243, 139, 168),
                info: Color::Rgb(137, 220, 235),
                selection: Color::Rgb(69, 71, 90),
                selection_text: Color::Rgb(245, 224, 220),
                chart_primary: Color::Rgb(203, 166, 247),
                chart_secondary: Color::Rgb(137, 180, 250),
            },
            ThemeName::CatppuccinLatte => Self {
                background: Color::Rgb(239, 241, 245),
                surface: Color::Rgb(230, 233, 239),
                surface_active: Color::Rgb(220, 224, 232),
                border: Color::Rgb(156, 160, 176),
                border_focused: Color::Rgb(30, 102, 245),
                text_primary: Color::Rgb(76, 79, 105),
                text_secondary: Color::Rgb(92, 95, 119),
                text_muted: Color::Rgb(124, 127, 147),
                accent: Color::Rgb(30, 102, 245),
                success: Color::Rgb(64, 160, 43),
                warning: Color::Rgb(223, 142, 29),
                danger: Color::Rgb(210, 15, 57),
                info: Color::Rgb(4, 165, 229),
                selection: Color::Rgb(172, 203, 255),
                selection_text: Color::Rgb(55, 58, 79),
                chart_primary: Color::Rgb(30, 102, 245),
                chart_secondary: Color::Rgb(136, 57, 239),
            },
            ThemeName::Dracula => Self {
                background: Color::Rgb(40, 42, 54),
                surface: Color::Rgb(48, 50, 65),
                surface_active: Color::Rgb(68, 71, 90),
                border: Color::Rgb(98, 114, 164),
                border_focused: Color::Rgb(189, 147, 249),
                text_primary: Color::Rgb(248, 248, 242),
                text_secondary: Color::Rgb(189, 190, 204),
                text_muted: Color::Rgb(98, 114, 164),
                accent: Color::Rgb(189, 147, 249),
                success: Color::Rgb(80, 250, 123),
                warning: Color::Rgb(241, 250, 140),
                danger: Color::Rgb(255, 85, 85),
                info: Color::Rgb(139, 233, 253),
                selection: Color::Rgb(68, 71, 90),
                selection_text: Color::Rgb(248, 248, 242),
                chart_primary: Color::Rgb(255, 121, 198),
                chart_secondary: Color::Rgb(139, 233, 253),
            },
            ThemeName::Nord => Self {
                background: Color::Rgb(46, 52, 64),
                surface: Color::Rgb(59, 66, 82),
                surface_active: Color::Rgb(67, 76, 94),
                border: Color::Rgb(76, 86, 106),
                border_focused: Color::Rgb(136, 192, 208),
                text_primary: Color::Rgb(236, 239, 244),
                text_secondary: Color::Rgb(216, 222, 233),
                text_muted: Color::Rgb(129, 161, 193),
                accent: Color::Rgb(136, 192, 208),
                success: Color::Rgb(163, 190, 140),
                warning: Color::Rgb(235, 203, 139),
                danger: Color::Rgb(191, 97, 106),
                info: Color::Rgb(129, 161, 193),
                selection: Color::Rgb(67, 76, 94),
                selection_text: Color::Rgb(236, 239, 244),
                chart_primary: Color::Rgb(136, 192, 208),
                chart_secondary: Color::Rgb(180, 142, 173),
            },
            ThemeName::MidnightBlue => Self {
                background: Color::Rgb(2, 6, 15),
                surface: Color::Rgb(4, 13, 30),
                surface_active: Color::Rgb(5, 28, 58),
                border: Color::Rgb(15, 74, 128),
                border_focused: Color::Rgb(0, 174, 255),
                text_primary: Color::Rgb(225, 244, 255),
                text_secondary: Color::Rgb(135, 196, 235),
                text_muted: Color::Rgb(62, 111, 148),
                accent: Color::Rgb(0, 174, 255),
                success: Color::Rgb(0, 255, 194),
                warning: Color::Rgb(255, 203, 64),
                danger: Color::Rgb(255, 76, 112),
                info: Color::Rgb(41, 171, 255),
                selection: Color::Rgb(0, 63, 110),
                selection_text: Color::Rgb(255, 255, 255),
                chart_primary: Color::Rgb(0, 238, 255),
                chart_secondary: Color::Rgb(45, 114, 255),
            },
            ThemeName::Acid => Self {
                background: Color::Rgb(5, 5, 8),
                surface: Color::Rgb(13, 13, 18),
                surface_active: Color::Rgb(31, 19, 43),
                border: Color::Rgb(91, 47, 115),
                border_focused: Color::Rgb(183, 255, 0),
                text_primary: Color::Rgb(244, 255, 230),
                text_secondary: Color::Rgb(183, 214, 151),
                text_muted: Color::Rgb(94, 115, 78),
                accent: Color::Rgb(183, 255, 0),
                success: Color::Rgb(121, 255, 0),
                warning: Color::Rgb(255, 209, 51),
                danger: Color::Rgb(255, 45, 136),
                info: Color::Rgb(0, 238, 255),
                selection: Color::Rgb(64, 24, 82),
                selection_text: Color::Rgb(255, 255, 255),
                chart_primary: Color::Rgb(183, 255, 0),
                chart_secondary: Color::Rgb(255, 45, 136),
            },
            ThemeName::PaperLight => Self {
                background: Color::Rgb(250, 247, 240),
                surface: Color::Rgb(242, 236, 226),
                surface_active: Color::Rgb(229, 220, 205),
                border: Color::Rgb(176, 161, 140),
                border_focused: Color::Rgb(0, 112, 125),
                text_primary: Color::Rgb(48, 43, 38),
                text_secondary: Color::Rgb(91, 80, 68),
                text_muted: Color::Rgb(137, 122, 105),
                accent: Color::Rgb(0, 112, 125),
                success: Color::Rgb(43, 125, 82),
                warning: Color::Rgb(176, 106, 0),
                danger: Color::Rgb(186, 54, 62),
                info: Color::Rgb(31, 98, 164),
                selection: Color::Rgb(190, 226, 224),
                selection_text: Color::Rgb(35, 45, 43),
                chart_primary: Color::Rgb(0, 112, 125),
                chart_secondary: Color::Rgb(31, 98, 164),
            },
            ThemeName::HighContrast => Self {
                background: Color::Black,
                surface: Color::Rgb(18, 18, 18),
                surface_active: Color::Rgb(45, 45, 45),
                border: Color::Gray,
                border_focused: Color::Cyan,
                text_primary: Color::White,
                text_secondary: Color::Gray,
                text_muted: Color::Gray,
                accent: Color::LightCyan,
                success: Color::LightGreen,
                warning: Color::Yellow,
                danger: Color::LightRed,
                info: Color::LightBlue,
                selection: Color::Blue,
                selection_text: Color::White,
                chart_primary: Color::LightCyan,
                chart_secondary: Color::LightBlue,
            },
            ThemeName::Monochrome => Self {
                background: Color::Reset,
                surface: Color::Reset,
                surface_active: Color::DarkGray,
                border: Color::DarkGray,
                border_focused: Color::White,
                text_primary: Color::White,
                text_secondary: Color::Gray,
                text_muted: Color::DarkGray,
                accent: Color::White,
                success: Color::White,
                warning: Color::White,
                danger: Color::White,
                info: Color::White,
                selection: Color::White,
                selection_text: Color::Black,
                chart_primary: Color::White,
                chart_secondary: Color::Gray,
            },
            ThemeName::ColorBlind => Self {
                background: Color::Rgb(12, 17, 23),
                surface: Color::Rgb(21, 29, 38),
                surface_active: Color::Rgb(31, 44, 56),
                border: Color::Rgb(70, 86, 101),
                border_focused: Color::Rgb(86, 180, 233),
                text_primary: Color::Rgb(236, 239, 241),
                text_secondary: Color::Rgb(170, 181, 190),
                text_muted: Color::Rgb(106, 120, 132),
                accent: Color::Rgb(86, 180, 233),
                success: Color::Rgb(0, 158, 115),
                warning: Color::Rgb(230, 159, 0),
                danger: Color::Rgb(213, 94, 0),
                info: Color::Rgb(0, 114, 178),
                selection: Color::Rgb(34, 79, 112),
                selection_text: Color::White,
                chart_primary: Color::Rgb(86, 180, 233),
                chart_secondary: Color::Rgb(230, 159, 0),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Icons {
    pub mode: IconMode,
}

impl Icons {
    #[must_use]
    pub const fn status_online(self) -> &'static str {
        match self.mode {
            IconMode::Ascii => "+",
            IconMode::Nerd | IconMode::Unicode => "●",
        }
    }

    #[must_use]
    pub const fn status_offline(self) -> &'static str {
        match self.mode {
            IconMode::Ascii => "-",
            IconMode::Nerd | IconMode::Unicode => "○",
        }
    }

    #[must_use]
    pub const fn status_unknown(self) -> &'static str {
        "?"
    }

    #[must_use]
    pub const fn spinner(self, tick: u64) -> &'static str {
        const UNICODE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        const ASCII: &[&str] = &["|", "/", "-", "\\"];
        match self.mode {
            IconMode::Ascii => ASCII[tick as usize % ASCII.len()],
            IconMode::Nerd | IconMode::Unicode => UNICODE[tick as usize % UNICODE.len()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_has_readable_core_pairs() {
        for name in ThemeName::ALL {
            let theme = Theme::named(name);
            assert_ne!(theme.background, theme.text_primary, "{name} body text");
            assert_ne!(theme.surface, theme.text_primary, "{name} surface text");
            assert_ne!(theme.selection, theme.selection_text, "{name} selection");
            assert_ne!(theme.background, theme.accent, "{name} accent");
        }
    }
}
