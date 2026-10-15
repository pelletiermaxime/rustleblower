//! Initial-window placement geometry, independent of the desktop backend.
pub const POSITIONS: &[(&str, &str)] = &[
    ("center", "Center"),
    ("top-right", "Top right"),
    ("bottom-right", "Bottom right"),
    ("top-left", "Top left"),
    ("bottom-left", "Bottom left"),
];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    #[default]
    Center,
    TopRight,
    BottomRight,
    TopLeft,
    BottomLeft,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Position {
    pub fn key(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::TopRight => "top-right",
            Self::BottomRight => "bottom-right",
            Self::TopLeft => "top-left",
            Self::BottomLeft => "bottom-left",
        }
    }
    pub fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "center" => Self::Center,
            "top-right" => Self::TopRight,
            "bottom-right" => Self::BottomRight,
            "top-left" => Self::TopLeft,
            "bottom-left" => Self::BottomLeft,
            _ => return None,
        })
    }
    pub fn place(self, work: Rect, width: f32, height: f32) -> Option<Rect> {
        if ![work.x, work.y, work.width, work.height, width, height]
            .iter()
            .all(|value| value.is_finite())
            || work.width <= 0.
            || work.height <= 0.
            || width <= 0.
            || height <= 0.
        {
            return None;
        }
        let margin = 16_f32.min(work.width / 4.).min(work.height / 4.);
        let width = width.min(work.width - 2. * margin);
        let height = height.min(work.height - 2. * margin);
        let x = match self {
            Self::Center => work.x + (work.width - width) / 2.,
            Self::TopLeft | Self::BottomLeft => work.x + margin,
            _ => work.x + work.width - margin - width,
        };
        let y = match self {
            Self::Center => work.y + (work.height - height) / 2.,
            Self::TopLeft | Self::TopRight => work.y + margin,
            _ => work.y + work.height - margin - height,
        };
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }
}
