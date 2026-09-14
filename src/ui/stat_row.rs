use apricot::app::App;
use nalgebra_glm::{vec2, Vec4};

use crate::{
    container,
    ui::{
        container::{Align, Container, Flow, Justify},
        label::Label,
        style::STYLE,
    },
};

pub fn stat_row<Msg: Clone + 'static>(
    key: &str,
    value: impl Into<String>,
    value_color: Vec4,
    width: f32,
    app: &App,
) -> Container<Msg> {
    let font = app.renderer.get_font_id_from_name("font").unwrap();
    let font_small = app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    container!(
        Label::new(key)
            .font(font_small, app)
            .color(STYLE.text_secondary),
        Label::new(value).font(font, app).color(value_color),
    )
    .flow(Flow::Horizontal)
    .justify(Justify::SpaceBetween)
    .cross_align(Align::Center)
    .fixed_width(vec2(width, 0.0))
    .padding(vec2(0.0, 0.0))
}
