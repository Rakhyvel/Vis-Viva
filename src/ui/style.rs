use nalgebra_glm::Vec4;

use crate::ui::oklch::oklch;

#[allow(dead_code)]
pub struct Style {
    // Backgrounds
    pub surface_deep: Vec4,
    pub surface: Vec4,
    pub surface_hover: Vec4,

    // Borders
    pub border_subtle: Vec4,
    pub border: Vec4,

    // Text
    pub text: Vec4,
    pub text_secondary: Vec4,
    pub text_disabled: Vec4,

    // Accents
    pub accent_surface: Vec4,
    pub accent_surface_hover: Vec4,
    pub accent: Vec4,
    pub accent_bright: Vec4,

    // Status colors
    pub positive: Vec4, // green, good delta-v etc
    pub warning: Vec4,  // yellow
    pub negative: Vec4, // red, low fuel etc
}

const C: f32 = 1.0;
const A_C: f32 = 0.8;
const H: f32 = 250.0;

lazy_static::lazy_static! {
    #[rustfmt::skip]
pub static ref STYLE: Style = Style {
    surface_deep:   oklch(0.16, 0.0020 * C, H, 0.96),
    surface:        oklch(0.18, 0.0075 * C, H, 0.96),
    surface_hover:  oklch(0.24, 0.0095 * C, H, 0.97),

    border_subtle:  oklch(0.33, 0.0070 * C, H, 1.0),
    border:         oklch(0.38, 0.0100 * C, H, 1.0),

    text:           oklch(0.95, 0.004 * C, H, 1.0),
    text_secondary: oklch(0.72, 0.008 * C, H, 1.0),
    text_disabled:  oklch(0.50, 0.006 * C, H, 1.0),

    accent_surface:       oklch(0.40, 0.110 * A_C, H, 0.97),
    accent_surface_hover: oklch(0.36, 0.130 * A_C, H, 0.98),
    accent:               oklch(0.62, 0.150 * A_C, H, 1.0),
    accent_bright:        oklch(0.80, 0.110 * A_C, H, 1.0),

    negative:       oklch(0.62, 0.170,  25.0, 1.0),
    warning:        oklch(0.70, 0.150,  85.0, 1.0),
    positive:       oklch(0.76, 0.130, 145.0, 1.0),
};
}
