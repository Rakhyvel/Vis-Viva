use std::cell::RefCell;

use apricot::app::App;
use hecs::World;
use nalgebra_glm::{vec2, Vec4};

use crate::{
    astro::epoch::EphemerisTime,
    container,
    hud::panel::CommandMessages,
    ui::{
        container::{Align, Container, Flow, Justify},
        label::Label,
        style::STYLE,
        widget::Widget,
    },
};

pub mod emergency;
pub mod fabricator;
pub mod footer;
pub mod format;
pub mod game_over;
pub mod maneuver;
pub mod panel;
pub mod porkchop_picker;
pub mod timeline;
pub mod transfer;

pub struct Binding(Box<dyn Fn(&World, EphemerisTime)>);

impl Binding {
    pub fn new(f: impl Fn(&World, EphemerisTime) + 'static) -> Self {
        Self(Box::new(f))
    }

    pub fn sync(&self, world: &World, now: EphemerisTime) {
        (self.0)(world, now)
    }
}

/// Writes `value` into a bound string only when it differs, so bound labels don't re-measure every frame
pub fn set_if_changed(cell: &RefCell<String>, value: String) {
    if *cell.borrow() != value {
        *cell.borrow_mut() = value;
    }
}

#[derive(Default)]
struct Section {
    pub widgets: Vec<Box<dyn Widget<CommandMessages>>>,
    pub bindings: Vec<Binding>,
}

impl Section {
    fn push(&mut self, w: impl Widget<CommandMessages> + 'static) {
        self.widgets.push(Box::new(w));
    }

    fn merge(&mut self, other: Section) {
        self.widgets.extend(other.widgets);
        self.bindings.extend(other.bindings);
    }

    fn into_card(self) -> Section {
        let c = Container::new(self.widgets)
            .fixed_width(vec2(280.0, 0.0))
            .border(STYLE.border, 1.0);
        Section {
            widgets: vec![Box::new(c)],
            bindings: self.bindings,
        }
    }
}

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
