//! Mapping of BlinderKitten channel types to the attributes the renderer
//! understands. Channel families and types are user-editable in BK, so the
//! mapping goes by (lower-cased) short name and keeps unknown types around.

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Attribute {
    Intensity,
    Shutter,
    Strobe,
    Red,
    Green,
    Blue,
    White,
    Amber,
    Uv,
    Cyan,
    Magenta,
    Yellow,
    Hue,
    Saturation,
    Cto,
    Ctc,
    ColorWheel,
    Pan,
    Tilt,
    Zoom,
    Focus,
    Frost,
    Iris,
    Gobo,
    Prism,
    Dummy,
    /// Anything without a visual effect (speeds, resets, macros, haze, ...).
    Other(String),
}

impl Attribute {
    /// `type_short` is the last path segment of a channel type reference,
    /// e.g. `red` in `/color/channelTypes/red`.
    pub fn from_channel_type(type_short: &str) -> Self {
        use Attribute::*;
        match type_short.to_ascii_lowercase().as_str() {
            "intensity" | "dimmer" => Intensity,
            "shutter" => Shutter,
            "strobe" | "strobefrequency" => Strobe,
            "red" => Red,
            "green" => Green,
            "blue" => Blue,
            "white" => White,
            "amber" => Amber,
            "uv" => Uv,
            "cyan" => Cyan,
            "magenta" => Magenta,
            "yellow" => Yellow,
            "hue" => Hue,
            "sat" | "saturation" => Saturation,
            "cto" => Cto,
            "ctc" => Ctc,
            "color" | "color1" | "colorwheel" => ColorWheel,
            "pan" => Pan,
            "tilt" => Tilt,
            "zoom" => Zoom,
            "focus" => Focus,
            "frost" => Frost,
            "iris" => Iris,
            "gobo1" | "gobo" => Gobo,
            "prism" => Prism,
            "dummy" | "" => Dummy,
            _ => Other(type_short.to_string()),
        }
    }

    pub fn is_color(&self) -> bool {
        use Attribute::*;
        matches!(
            self,
            Red | Green | Blue | White | Amber | Uv | Cyan | Magenta | Yellow | Hue | Saturation | Cto | Ctc | ColorWheel
        )
    }

    pub fn is_optics(&self) -> bool {
        use Attribute::*;
        matches!(self, Zoom | Focus | Frost | Iris | Gobo | Prism)
    }

    pub fn is_visual(&self) -> bool {
        !matches!(self, Attribute::Dummy | Attribute::Other(_))
    }
}
