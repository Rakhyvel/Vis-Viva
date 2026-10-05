use apricot::{app::App, font::FontId};
use hecs::World;
use nalgebra_glm::{vec2, I32Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    sim::{
        hierarchy::Named,
        life_support::{Emergency, Station},
        resources::Resource,
    },
    ui::{
        anchor::{Anchor, AnchorPoint},
        container::{Align, Container},
        label::Label,
        style::STYLE,
        widget::Widget,
    },
};

pub struct EmergencyBanner {
    anchor: Option<Anchor<()>>,
    built_for: Option<(I32Vec2, Vec<String>)>,
}

impl EmergencyBanner {
    pub fn new() -> Self {
        Self {
            anchor: None,
            built_for: None,
        }
    }

    pub fn sync(&mut self, app: &App, lines: Vec<String>) {
        if lines.is_empty() {
            self.anchor = None;
            return;
        }

        let key = Some((app.window_size, lines.clone()));
        if key == self.built_for {
            return;
        }
        self.built_for = key;

        let font_big: FontId = app.renderer.get_font_id_from_name("font-big").unwrap();

        let mut lines_labels: Vec<Box<dyn Widget<()>>> = vec![Box::new(
            Label::new("CREW EMERGENCY!")
                .font(font_big, app)
                .color(STYLE.negative),
        )];

        for line in lines {
            lines_labels.push(Box::new(
                Label::new(line).font(font_big, app).color(STYLE.negative),
            ))
        }

        let container = Container::new(lines_labels)
            .cross_align(Align::Center)
            .background_color(STYLE.surface)
            .border(STYLE.negative, 2.0);

        let mut anchor =
            Anchor::new(Box::new(container), AnchorPoint::TopCenter).margin(vec2(16.0, 16.0));
        anchor.reposition(app);
        self.anchor = Some(anchor);
    }

    pub fn render(&self, app: &App) {
        if let Some(a) = &self.anchor {
            a.render(app);
        }
    }
}

pub fn emergency_lines(world: &World, now: EphemerisTime) -> Vec<String> {
    world
        .query::<(&Named, &Station)>()
        .iter()
        .flat_map(|(_, (n, s))| {
            s.emergencies.iter().filter_map(|e| match e {
                Emergency {
                    deadline,
                    cause: Resource::Water,
                } => Some(format!(
                    "{} crew die of thirst in {}",
                    n.name,
                    (*deadline - now).short_duration()
                )),
                Emergency {
                    deadline,
                    cause: Resource::Oxygen,
                } => Some(format!(
                    "{} crew suffocate in {}",
                    n.name,
                    (*deadline - now).short_duration()
                )),
                _ => None,
            })
        })
        .collect()
}
