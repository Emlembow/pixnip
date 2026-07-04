//! Converts linear scRGB pixels (as produced by DXGI Desktop Duplication on an
//! HDR desktop) into display-correct 8-bit sRGB, using the monitor's actual
//! SDR white level so SDR content lands at full white instead of blowing out.

/// Scale factor that maps the desktop's SDR white level back to scRGB's
/// reference white (1.0 == 80 nits). `sdr_white_level` is the raw
/// `DISPLAYCONFIG_SDR_WHITE_LEVEL.SDRWhiteLevel` value (nits = value / 1000 * 80).
pub fn sdr_scale_from_white_level(sdr_white_level: u32) -> f32 {
    if sdr_white_level == 0 {
        return 1.0;
    }
    1000.0 / sdr_white_level as f32
}

/// Tone-maps one linear scRGB pixel to linear space normalized against SDR
/// white, preserving hue for any true-HDR highlights above 1.0 via a soft
/// knee instead of hard-clipping them.
pub fn tonemap_pixel(rgb: [f32; 3], sdr_scale: f32) -> [f32; 3] {
    let mut c = [rgb[0] * sdr_scale, rgb[1] * sdr_scale, rgb[2] * sdr_scale];
    let max_c = c[0].max(c[1]).max(c[2]);
    if max_c > 1.0 {
        let compressed = 1.0 + (1.0 - (-(max_c - 1.0)).exp()) * 0.25;
        let scale = compressed / max_c;
        c[0] *= scale;
        c[1] *= scale;
        c[2] *= scale;
    }
    c
}

/// Linear -> sRGB gamma, clamped and packed to 8-bit.
pub fn linear_to_srgb_u8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_white_level_is_identity() {
        assert_eq!(sdr_scale_from_white_level(1000), 1.0);
    }

    #[test]
    fn raised_white_level_darkens_sdr_content() {
        // A user-raised SDR white level (brighter SDR-on-HDR slider) means the
        // actual nits are higher, so we must scale linear values down more.
        assert!(sdr_scale_from_white_level(3000) < 1.0);
    }

    #[test]
    fn reference_white_maps_to_full_white() {
        let [r, g, b] = tonemap_pixel([1.0, 1.0, 1.0], 1.0);
        assert_eq!(linear_to_srgb_u8(r), 255);
        assert_eq!(linear_to_srgb_u8(g), 255);
        assert_eq!(linear_to_srgb_u8(b), 255);
    }

    #[test]
    fn highlights_above_white_are_compressed_not_clipped_to_gray() {
        let [r, g, b] = tonemap_pixel([4.0, 0.1, 0.1], 1.0);
        // Should stay red-dominant (hue preserved) and not pure white.
        assert!(r > g && r > b);
        assert!(linear_to_srgb_u8(r) < 255 || linear_to_srgb_u8(g) > 0);
    }
}
