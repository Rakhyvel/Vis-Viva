use std::{cell::Cell, marker::PhantomData, rc::Rc};

use apricot::{app::App, rectangle::Rectangle, render_core::TextureId};
use nalgebra_glm::{vec2, vec4, Vec2, Vec4};

use crate::{
    astro::{
        epoch::EphemerisTime,
        units::{SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_YEAR},
    },
    ui::{msg::MsgQueue, style::STYLE, widget::Widget},
};

pub struct PorkchopPicker<Msg> {
    /// The rectangle defining the texture's position and size
    rect: Rectangle,
    /// The texture ID to render
    texture_id: TextureId,
    cols: usize,
    rows: usize,
    selected: Rc<Cell<(usize, usize)>>,
    optimum: Rc<Cell<(usize, usize)>>,
    axes: Rc<Cell<Option<PlotAxes>>>,
    dragging: bool,
    _msg: PhantomData<Msg>,
}

#[derive(Clone, Copy)]
pub struct PlotAxes {
    pub depart_start: EphemerisTime,
    pub depart_step: EphemerisTime,
    pub tof_min: f64, // years
    pub tof_max: f64, // years
}

impl<Msg> PorkchopPicker<Msg> {
    const GUTTER_L: f32 = 40.0;
    const GUTTER_B: f32 = 30.0;

    const NICE_SECS: &[f64] = &[
        3600.0, 10800.0, 21600.0, 43200.0, // 1h, 3h, 6h, 12h
        86400.0, 172800.0, 432000.0, 604800.0, // 1d, 2d, 5d, 7d
        1209600.0, 2592000.0, 7776000.0, // 14d 30d 90d
        15552000.0, 31536000.0, 63072000.0, // 180d 1y 2y
    ];

    /// Creates a new porkchop plot picker
    pub fn new(
        size: Vec2,
        texture_id: TextureId,
        cols: usize,
        rows: usize,
        selected: Rc<Cell<(usize, usize)>>,
        optimum: Rc<Cell<(usize, usize)>>,
        axes: Rc<Cell<Option<PlotAxes>>>,
    ) -> Self {
        Self {
            rect: Rectangle {
                pos: Vec2::zeros(),
                size,
            },
            texture_id,
            cols,
            rows,
            selected,
            optimum,
            axes,
            dragging: false,
            _msg: PhantomData,
        }
    }

    fn cell_center(&self, i: usize, j: usize) -> Vec2 {
        let rect = self.plot_rect();
        vec2(
            rect.pos.x + (i as f32 + 0.5) / self.cols as f32 * rect.size.x,
            rect.pos.y + (j as f32 + 0.5) / self.rows as f32 * rect.size.y,
        )
    }

    fn cell_at(&self, pos: Vec2) -> (usize, usize) {
        let rect = self.plot_rect();
        let u = ((pos.x - rect.pos.x) / rect.size.x).clamp(0.0, 1.0);
        let v = ((pos.y - rect.pos.y) / rect.size.y).clamp(0.0, 1.0);
        let i = ((u * self.cols as f32) as usize).min(self.cols - 1);
        let j = ((v * self.rows as f32) as usize).min(self.rows - 1);
        (i, j)
    }

    fn x_of_depart(&self, plot: Rectangle, a: &PlotAxes, t: EphemerisTime) -> Option<f32> {
        let i = (t - a.depart_start).as_secs() / a.depart_step.as_secs();
        let x = plot.pos.x + ((i + 0.5) / self.cols as f64) as f32 * plot.size.x;
        if (plot.pos.x..=plot.size.x + plot.pos.x).contains(&x) {
            Some(x)
        } else {
            None
        }
    }

    fn y_of_tof(&self, plot: Rectangle, a: &PlotAxes, tof: f64) -> Option<f32> {
        let j = (a.tof_max - tof) / (a.tof_max - a.tof_min) * (self.rows - 1) as f64;
        let y = plot.pos.y + ((j + 0.5) / self.rows as f64) as f32 * plot.size.y;
        if (plot.pos.y..=plot.size.y + plot.pos.y).contains(&y) {
            Some(y)
        } else {
            None
        }
    }

    fn nice_step(range_secs: f64, max_ticks: f64) -> f64 {
        Self::NICE_SECS
            .iter()
            .copied()
            .find(|s| range_secs / s <= max_ticks)
            .unwrap_or(*Self::NICE_SECS.last().unwrap())
    }

    fn draw_depart_ticks(&self, app: &App, plot: Rectangle, a: &PlotAxes) {
        const TICK_LEN: f32 = 4.0;
        const LABEL_PAD: f32 = 2.0;
        const MIN_LABEL_GAP: f32 = 6.0;

        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let f = app.renderer.get_font_from_id(font).unwrap();
        let range_secs = a.depart_step.as_secs() * self.cols as f64;
        let step_secs = Self::nice_step(range_secs, 4.0);
        let step = EphemerisTime::from_secs(step_secs);

        let bottom = plot.pos.y + plot.size.y;
        let right = plot.pos.x + plot.size.x;
        let mut last_right = f32::NEG_INFINITY;
        let mut et = a.depart_start.ceil_to(step);

        while let Some(x) = self.x_of_depart(plot, a, et) {
            let x = x.round(); // keep them bad boys crisp

            // Tick mark below plot
            app.renderer.set_color(STYLE.border);
            app.renderer
                .fill_rect(Rectangle::new(x, bottom, 1.0, TICK_LEN));

            // Label centered under the tick, skipped if it would collide or overflow
            let label = depart_tick_label(et, step_secs);
            let w = f.measure(&label).x;
            let left = (x - w * 0.5).round();
            if left > last_right + MIN_LABEL_GAP && left + w <= right {
                app.renderer.set_color(STYLE.text_secondary);
                app.renderer
                    .draw_text(vec2(left, bottom + TICK_LEN + LABEL_PAD), &label);
                last_right = left + w;
            }

            et += step;
        }

        let title = if step_secs >= SECONDS_PER_YEAR {
            String::from("DEPARTURE")
        } else {
            format!("DEPARTURE {:04}", a.depart_start.year())
        };

        let size = f.measure(&title);
        app.renderer.set_color(STYLE.text_secondary);
        app.renderer.draw_text(
            vec2(
                (plot.pos.x + (plot.size.x - size.x) * 0.5).round(),
                self.rect.pos.y + self.rect.size.y - size.y * 0.5,
            ),
            &title,
        );
    }

    fn draw_tof_ticks(&self, app: &App, plot: Rectangle, a: &PlotAxes) {
        const TICK_LEN: f32 = 4.0;
        const LABEL_PAD: f32 = 3.0;
        const MIN_LABEL_GAP: f32 = 2.0;

        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let f = app.renderer.get_font_from_id(font).unwrap();
        let range_secs = (a.tof_max - a.tof_min) * SECONDS_PER_YEAR;
        let step_secs = Self::nice_step(range_secs, 4.0);
        let step = EphemerisTime::from_secs(step_secs);

        let plot_bottom = plot.pos.y + plot.size.y;
        let mut last_top = f32::INFINITY;
        let mut tof = EphemerisTime::from_years(a.tof_min).ceil_to(step);

        while let Some(y) = self.y_of_tof(plot, a, tof.as_years()) {
            let y = y.round();

            // Tick mark sticking out left of the plot
            app.renderer.set_color(STYLE.border);
            app.renderer
                .fill_rect(Rectangle::new(plot.pos.x - TICK_LEN, y, TICK_LEN, 1.0));

            // Label right-aligned against the tick, vertically centered on it
            let label = tof_tick_label(tof.as_secs(), step_secs);
            let size = f.measure(&label);
            let top = (y - size.y * 0.5).round();
            if top + size.y < last_top - MIN_LABEL_GAP
                && top >= plot.pos.y
                && top + size.y <= plot_bottom
            {
                app.renderer.set_color(STYLE.text_secondary);
                app.renderer.draw_text(
                    vec2((plot.pos.x - TICK_LEN - LABEL_PAD - size.x).round(), top),
                    &label,
                );
                last_top = top;
            }

            tof += step;
        }
    }

    fn draw_crosshair(&self, app: &App, c: Vec2, color: Vec4, thickness: f32) {
        const ARM: f32 = 10.0; // arm length
        const GAP: f32 = 4.0; // hole radius

        let t = thickness;
        app.renderer.set_color(color);
        app.renderer
            .fill_rect(Rectangle::new(c.x - GAP - ARM, c.y - t * 0.5, ARM, t)); // left
        app.renderer
            .fill_rect(Rectangle::new(c.x + GAP, c.y - t * 0.5, ARM, t)); // right
        app.renderer
            .fill_rect(Rectangle::new(c.x - t * 0.5, c.y - GAP - ARM, t, ARM)); // up
        app.renderer
            .fill_rect(Rectangle::new(c.x - t * 0.5, c.y + GAP, t, ARM)); // down
    }

    fn plot_rect(&self) -> Rectangle {
        Rectangle::new(
            self.rect.pos.x + Self::GUTTER_L,
            self.rect.pos.y,
            self.rect.size.x - Self::GUTTER_L,
            self.rect.size.y - Self::GUTTER_B,
        )
    }
}

impl<Msg: Clone + 'static> Widget<Msg> for PorkchopPicker<Msg> {
    fn update(&mut self, app: &App, _msgq: &mut MsgQueue<Msg>) {
        if !app.is_click_consumed() {
            if (app.mouse_left_dragging || app.mouse_left_clicked)
                && app.mouse_over(&self.plot_rect())
            {
                self.dragging = true;
            }
            if !app.mouse_left_dragging {
                self.dragging = false;
            }
        }
        if self.dragging {
            self.selected.set(self.cell_at(app.mouse_pos))
        }
    }

    /// Render the texture to the screen
    fn render(&self, app: &App) {
        let rect = self.plot_rect();
        app.renderer.copy_texture(
            rect,
            self.texture_id,
            Rectangle {
                pos: vec2(0.0, 0.0),
                size: vec2(self.cols as f32, self.rows as f32),
            },
            &vec4(1.0, 1.0, 1.0, 1.0),
        );

        let (i, j) = self.optimum.get();
        let optimum_c = self.cell_center(i, j);
        let diamond = app
            .renderer
            .get_mesh_id_from_name("square-outline")
            .unwrap();
        app.renderer.set_color(vec4(0.0, 0.0, 0.0, 0.7));
        app.renderer.fill_polygon(diamond, optimum_c, 7.0, 0.0);

        let (i, j) = self.selected.get();
        let c = self.cell_center(i, j);
        self.draw_crosshair(app, c, vec4(1.0, 1.0, 1.0, 1.0), 3.0);

        if let Some(a) = &self.axes.get() {
            self.draw_depart_ticks(app, rect, a);
            self.draw_tof_ticks(app, rect, a);
        }
    }

    fn size(&self) -> Vec2 {
        self.rect.size
    }

    fn layout(&mut self, pos: Vec2) {
        self.rect.pos = pos
    }
}

fn depart_tick_label(et: EphemerisTime, step_secs: f64) -> String {
    if step_secs >= SECONDS_PER_YEAR {
        format!("{:04}", et.year())
    } else if step_secs >= SECONDS_PER_DAY {
        format!("{} {}", et.day_of_month(), et.short_month_name())
    } else {
        et.hour_minute()
    }
}

fn tof_tick_label(tof_secs: f64, step_secs: f64) -> String {
    if step_secs >= SECONDS_PER_YEAR {
        format!("{:.0}y", tof_secs / SECONDS_PER_YEAR)
    } else if step_secs >= SECONDS_PER_DAY {
        format!("{:.0}d", tof_secs / SECONDS_PER_DAY)
    } else {
        format!("{:.0}h", tof_secs / SECONDS_PER_HOUR)
    }
}
