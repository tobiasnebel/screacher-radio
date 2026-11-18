use fltk::{app, enums::Color, misc::Tooltip};

use crate::settings::Theme;

type Rgb = (u8, u8, u8);
const GRAY_RAMP_SIZE: usize = 24;
const PALETTE_COLORS: [Color; 4] = [
    Color::Foreground,
    Color::Background2,
    Color::Inactive,
    Color::Selection,
];

pub struct Palette {
    colors: [Rgb; PALETTE_COLORS.len()],
    grays: [Rgb; GRAY_RAMP_SIZE],
    tooltip_background: Color,
    tooltip_text: Color,
}

impl Palette {
    pub fn current() -> Self {
        Self {
            colors: PALETTE_COLORS.map(|color| color.to_rgb()),
            grays: std::array::from_fn(|index| Color::gray_ramp(index as i32).to_rgb()),
            tooltip_background: Tooltip::color(),
            tooltip_text: Tooltip::text_color(),
        }
    }

    pub fn apply(&self, theme: Theme) {
        let colors = match theme {
            Theme::Standard => {
                for (index, (r, g, b)) in self.grays.into_iter().enumerate() {
                    app::set_color(Color::gray_ramp(index as i32), r, g, b);
                }
                Tooltip::set_color(self.tooltip_background);
                Tooltip::set_text_color(self.tooltip_text);
                self.colors
            }
            Theme::Dark => {
                app::background(33, 36, 36);
                Tooltip::set_color(Color::Background2);
                Tooltip::set_text_color(Color::Foreground);
                [(132, 235, 158), (49, 55, 53), (106, 141, 114), (41, 76, 57)]
            }
            Theme::Light => {
                app::background(242, 240, 234);
                Tooltip::set_color(Color::Background2);
                Tooltip::set_text_color(Color::Foreground);
                [
                    (34, 73, 124),
                    (250, 249, 246),
                    (123, 142, 163),
                    (199, 217, 237),
                ]
            }
        };
        for (color, (r, g, b)) in PALETTE_COLORS.into_iter().zip(colors) {
            app::set_color(color, r, g, b);
        }
    }
}
