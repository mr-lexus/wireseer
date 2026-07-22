use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutClass {
    Wide,
    Standard,
    Compact,
    TooSmall,
}

impl LayoutClass {
    #[must_use]
    pub const fn from_area(area: Rect) -> Self {
        if area.width < 70 || area.height < 18 {
            Self::TooSmall
        } else if area.width >= 140 {
            Self::Wide
        } else if area.width >= 90 {
            Self::Standard
        } else {
            Self::Compact
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_breakpoints_are_stable() {
        assert_eq!(
            LayoutClass::from_area(Rect::new(0, 0, 160, 40)),
            LayoutClass::Wide
        );
        assert_eq!(
            LayoutClass::from_area(Rect::new(0, 0, 100, 30)),
            LayoutClass::Standard
        );
        assert_eq!(
            LayoutClass::from_area(Rect::new(0, 0, 70, 18)),
            LayoutClass::Compact
        );
        assert_eq!(
            LayoutClass::from_area(Rect::new(0, 0, 69, 30)),
            LayoutClass::TooSmall
        );
        assert_eq!(
            LayoutClass::from_area(Rect::new(0, 0, 100, 17)),
            LayoutClass::TooSmall
        );
    }
}
