//! The accent of the web app's Nothing palette in light mode, as the hex colour mail
//! clients understand: few of them read `oklch()`.

/// The palette's accent lightness and chroma; only the hue varies. With white text on it,
/// they keep a contrast of at least 4.5:1 at every hue.
const LIGHTNESS: f64 = 0.50;
const CHROMA: f64 = 0.18;

/// `oklch(50% 0.18 hue)` as `#rrggbb`. A colour outside sRGB loses chroma until it fits,
/// keeping its lightness and hue, so its contrast holds.
pub(super) fn accent(hue: u16) -> String {
    let [r, g, b] = fit(LIGHTNESS, CHROMA, f64::from(hue));
    format!("#{:02x}{:02x}{:02x}", to_byte(r), to_byte(g), to_byte(b))
}

/// The linear sRGB of the colour, its chroma narrowed down by halving until it is inside
/// the gamut.
fn fit(lightness: f64, chroma: f64, hue: f64) -> [f64; 3] {
    let inside = |rgb: &[f64; 3]| rgb.iter().all(|c| (-1e-9..=1.0 + 1e-9).contains(c));
    let full = linear_srgb(lightness, chroma, hue);
    if inside(&full) {
        return full;
    }
    let (mut low, mut high) = (0.0, chroma);
    for _ in 0..32 {
        let mid = f64::midpoint(low, high);
        if inside(&linear_srgb(lightness, mid, hue)) {
            low = mid;
        } else {
            high = mid;
        }
    }
    linear_srgb(lightness, low, hue)
}

/// OKLCH to linear sRGB, by Björn Ottosson's matrices.
fn linear_srgb(lightness: f64, chroma: f64, hue: f64) -> [f64; 3] {
    let (sin, cos) = hue.to_radians().sin_cos();
    let (a, b) = (chroma * cos, chroma * sin);

    let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);

    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// A linear channel as an sRGB byte.
fn to_byte(linear: f64) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    // Within 0..=255 after the clamp.
    (encoded * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG's relative luminance of `#rrggbb`.
    fn luminance(hex: &str) -> f64 {
        let channel = |at: usize| {
            let byte = u8::from_str_radix(&hex[at..at + 2], 16).expect("hex");
            let c = f64::from(byte) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    #[test]
    fn known_colours_convert() {
        let red = linear_srgb(0.627_955_4, 0.257_683_3, 29.233_885_1);
        assert_eq!(red.map(to_byte), [255, 0, 0], "{red:?}");
        assert_eq!(linear_srgb(1.0, 0.0, 0.0).map(to_byte), [255, 255, 255]);
        assert_eq!(linear_srgb(0.0, 0.0, 0.0).map(to_byte), [0, 0, 0]);
    }

    #[test]
    fn white_text_reads_on_the_accent_at_every_hue() {
        for hue in 0..360 {
            let fill = accent(hue);
            let contrast = 1.05 / (luminance(&fill) + 0.05);
            assert!(contrast >= 4.5, "hue {hue}: {fill} gives {contrast:.2}:1");
        }
    }

    #[test]
    fn the_default_hue_is_a_blue() {
        let fill = accent(250);
        let [r, g, b] = [1, 3, 5].map(|at| u8::from_str_radix(&fill[at..at + 2], 16).expect("hex"));
        assert!(b > r && b > g, "{fill}");
    }
}
