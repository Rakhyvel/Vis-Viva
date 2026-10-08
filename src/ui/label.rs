use std::{cell::RefCell, rc::Rc};

use crate::ui::{msg::MsgQueue, widget::Widget};
use apricot::{app::App, font::FontId, rectangle::Rectangle};
use nalgebra_glm::{vec4, Vec2, Vec4};

/// A bit of text
pub struct Label {
    /// The rectangle defining the button's position and size
    rect: Rectangle,
    source: Option<Rc<RefCell<String>>>,
    /// The text to be drawn for the button
    label: String,
    color: Vec4,
    color_source: Option<Rc<RefCell<Vec4>>>,
    font_id: Option<FontId>,
}

impl Label {
    /// Creates a label
    pub fn new(label: impl Into<String>) -> Self {
        let text = label.into();
        let rect = Rectangle {
            pos: Vec2::zeros(),
            size: Vec2::zeros(),
        };
        Self {
            rect,
            source: None,
            label: text,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            color_source: None,
            font_id: None,
        }
    }

    pub fn bound(source: Rc<RefCell<String>>) -> Self {
        // ...to fall in love...
        let mut label = Self::new(source.borrow().clone());
        label.source = Some(source);
        label
    }

    pub fn font(mut self, font_id: FontId, app: &App) -> Self {
        // TODO: Take &'static str, lookup font by name?
        self.font_id = Some(font_id);
        let font = app.renderer.get_font_from_id(font_id).unwrap();
        let size = font.measure(&self.label);
        self.rect.size = size;
        self
    }

    pub fn color(mut self, color: Vec4) -> Self {
        self.color = color;
        self
    }

    pub fn bind_color(mut self, color_source: Rc<RefCell<Vec4>>) -> Self {
        self.color_source = Some(color_source);
        self
    }
}

impl<Msg: Clone + 'static> Widget<Msg> for Label {
    fn update(&mut self, app: &App, _msgq: &mut MsgQueue<Msg>) {
        if let Some(src) = &self.source {
            let s = src.borrow();
            if *s != self.label {
                self.label = s.clone();
                if let Some(font_id) = self.font_id {
                    let font = app.renderer.get_font_from_id(font_id).unwrap();
                    self.rect.size = font.measure(&self.label)
                }
            }
        };

        if let Some(color_source) = &self.color_source {
            let c = color_source.borrow();
            self.color = *c;
        }
    }

    fn render(&self, app: &App) {
        let old_font = app.renderer.get_current_font_id();

        app.renderer.set_color(self.color);
        app.renderer.set_font(self.font_id.unwrap());
        app.renderer.draw_text(self.rect.pos, &self.label);

        if let Some(old_font) = old_font {
            app.renderer.set_font(old_font);
        }
    }

    fn size(&self) -> Vec2 {
        self.rect.size
    }

    fn layout(&mut self, pos: Vec2) {
        self.rect.pos = pos
    }
}
