use apricot::{app::App, rectangle::Rectangle};
use nalgebra_glm::{vec2, vec4};

use crate::{
    astro::epoch::EphemerisTime,
    components::station::Resource,
    container,
    ui::{
        button::Button,
        container::{Align, Container},
        hrule::HRule,
        label::Label,
        modal::Modal,
        style::STYLE,
        widget::{recv_msgs, Widget},
    },
};

#[derive(Clone)]
pub enum GameOverMessages {
    Quit,
}

pub struct GameOverUi {
    modal: Modal<GameOverMessages>,
}

impl GameOverUi {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(Box::new(container!())),
        }
    }

    pub fn show(&mut self, station: &str, cause: Resource, when: EphemerisTime, app: &App) {
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_big = app.renderer.get_font_id_from_name("font-big").unwrap();
        const WIDTH: f32 = 320.0;

        let reason = match cause {
            Resource::Oxygen => "The oxygen ran out.",
            Resource::Water => "The water ran out.",
            Resource::Energy => "Power failed, and life support with it.",
            Resource::Hydrogen => unreachable!("hydrogen isn't life critical?"),
        };

        self.modal = Modal::new(Box::new(
            Container::new(vec![
                Box::new(
                    Label::new("CREW LOST")
                        .font(font_big, app)
                        .color(STYLE.negative),
                ),
                Box::new(HRule::new(STYLE.border, 1.0, WIDTH)),
                Box::new(Label::new(format!("{station}: {reason}")).font(font, app)),
                Box::new(
                    Label::new(when.short_date())
                        .font(font, app)
                        .color(STYLE.text_secondary),
                ),
                Box::new(
                    Button::text(vec2(WIDTH, 30.0), "Quit")
                        .use_style(&STYLE)
                        .on_click(GameOverMessages::Quit),
                ),
            ])
            .cross_align(Align::Center)
            .background_color(STYLE.surface)
            .border(STYLE.border, 1.0)
            .padding(vec2(12.0, 12.0)),
        ))
        .shown(true);
        self.modal.reposition(app);
    }

    pub fn update(&mut self, app: &App) -> bool {
        recv_msgs(app, &mut self.modal)
            .into_iter()
            .any(|m| matches!(m, GameOverMessages::Quit))
    }

    pub fn render(&self, app: &App) {
        if self.modal.is_shown() {
            app.renderer.set_color(vec4(0.0, 0.0, 0.0, 0.6));
            app.renderer.fill_rect(Rectangle::new(
                0.0,
                0.0,
                app.window_size.x as f32,
                app.window_size.y as f32,
            ));
        }
        self.modal.render(app);
    }

    pub fn is_shown(&self) -> bool {
        self.modal.is_shown()
    }
}
