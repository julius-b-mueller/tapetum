//! Port of `StringUtil::toShortName` from juce_organicui.
//!
//! Target parameters in `.olga` files reference other containers by their
//! short-name path (e.g. `/movingHeadWash`), so we need the exact same
//! transformation to resolve them.

const SPECIALS: &str = "+-()[]{}<>^'@#*$~";
const REPLACES: &str = "_____________#___";

/// `fold_accents` mirrors the non-macOS build of BlinderKitten, which maps a
/// handful of accented letters to ASCII before stripping. macOS builds drop
/// them instead, so a show saved on another OS may use either variant.
pub fn to_short_name(nice_name: &str, fold_accents: bool) -> String {
    let mut res = String::with_capacity(nice_name.len());
    for c in nice_name.chars() {
        let c = match c {
            '/' | '"' => '_',
            c => match SPECIALS.chars().position(|s| s == c) {
                Some(i) => REPLACES.chars().nth(i).unwrap(),
                None => c,
            },
        };
        let c = if fold_accents {
            match c {
                'â' | 'à' => 'a',
                'é' | 'è' | 'ê' => 'e',
                'ô' => 'o',
                'ç' => 'c',
                'ü' => 'u',
                c => c,
            }
        } else {
            c
        };
        if c.is_ascii_alphanumeric() || c == '_' || c == '#' || c == ' ' {
            res.push(c);
        }
    }

    let mut out = String::with_capacity(res.len());
    for (index, word) in res.split(' ').filter(|w| !w.is_empty()).enumerate() {
        if index == 0 && word.to_uppercase() == word {
            out.push_str(&word.to_lowercase());
        } else {
            let mut chars = word.chars();
            let first = chars.next().unwrap();
            if index == 0 {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

/// True if `short` is the short name of `nice_name` under either OS variant.
pub fn matches(nice_name: &str, short: &str) -> bool {
    to_short_name(nice_name, false) == short || to_short_name(nice_name, true) == short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_from_the_test_show() {
        assert_eq!(to_short_name("Moving Head Wash", false), "movingHeadWash");
        assert_eq!(to_short_name("Bar RGBW", false), "barRGBW");
        // Hyphens, digits and capitals as in BK's own type names.
        assert_eq!(to_short_name("Wash XH-555", false), "washXH_555");
        assert_eq!(to_short_name("DMX1", false), "dmx1");
        assert_eq!(to_short_name("Channel", false), "channel");
        assert_eq!(to_short_name("Channel Types", false), "channelTypes");
        assert_eq!(to_short_name("Basic LED Theater Spot 120 FC", false), "basicLEDTheaterSpot120FC");
        assert_eq!(to_short_name("Water FX-40", false), "waterFX_40");
        assert_eq!(to_short_name("PT Speed", false), "ptSpeed");
    }

    #[test]
    fn accents_differ_by_platform() {
        assert_eq!(to_short_name("Scène", true), "scene");
        assert_eq!(to_short_name("Scène", false), "scne");
        assert!(matches("Scène", "scne") && matches("Scène", "scene"));
    }
}
