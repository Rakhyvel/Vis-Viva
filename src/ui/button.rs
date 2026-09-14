use std::{cell::Cell, f32::consts::PI, rc::Rc};

use crate::ui::{msg::MsgQueue, style::Style, widget::Widget};
use apricot::{app::App, font::FontId, rectangle::Rectangle};
use nalgebra_glm::{vec2, vec4, Vec2, Vec4};

/// A button with text
pub struct Button<Msg> {
    /// The rectangle defining the button's position and size
    rect: Rectangle,
    /// The text to be drawn for the button
    content: ButtonContent,
    icon_src: Option<Rc<Cell<Icon>>>,
    text_color: Vec4,
    inactive_text_color: Vec4,
    /// The background color of the button
    background_color: Vec4,
    inactive_background_color: Vec4,
    /// The background color of the button when hovered
    hovered_color: Vec4,
    hovered_text_color: Vec4,
    border: Option<(nalgebra_glm::Vec4, f32)>,
    inactive_border: Option<(nalgebra_glm::Vec4, f32)>,
    /// The message to send when the button is clicked (no message if None)
    on_click: Option<Msg>,
    /// Whether or not the button is active or nah
    active: bool,
    /// How to get the `active`
    active_src: Option<Rc<Cell<bool>>>,
    /// Whether the button is hovered or not
    hovered: bool,
}

pub enum ButtonContent {
    Text(String),
    Icon(Icon),
}

#[derive(Clone, Copy)]
pub enum Icon {
    Play,
    Pause,
    FastForward,
    SlowForward,
}

impl ButtonContent {
    pub fn draw(&self, rect: &Rectangle, app: &App) {
        let center = rect.pos + rect.size * 0.5;

        match self {
            ButtonContent::Text(str) => {
                let text_size = {
                    let current_font = app.renderer.get_current_font().unwrap();
                    current_font.measure(str)
                };
                app.renderer
                    .draw_text(rect.pos + (rect.size - text_size) * 0.5, str);
            }
            ButtonContent::Icon(Icon::Play) => {
                let tri = app.renderer.get_mesh_id_from_name("triangle").unwrap();
                app.renderer.fill_polygon(tri, center, 10.0, PI);
            }
            ButtonContent::Icon(Icon::Pause) => {
                let w = 6.0;
                let h = 9.0;
                app.renderer.fill_rect(Rectangle::new(
                    center.x - w * 1.2,
                    center.y - h,
                    w,
                    2.0 * h,
                ));
                app.renderer.fill_rect(Rectangle::new(
                    center.x + w * 0.2,
                    center.y - h,
                    w,
                    2.0 * h,
                ));
            }
            ButtonContent::Icon(Icon::FastForward) => {
                let s = rect.size.x * 0.3125;
                let tri = app.renderer.get_mesh_id_from_name("triangle").unwrap();
                app.renderer
                    .fill_polygon(tri, center - vec2(s * 0.5, 0.0), s * 0.64, PI);
                app.renderer
                    .fill_polygon(tri, center + vec2(s * 0.3, 0.0), s * 0.64, PI);
            }
            ButtonContent::Icon(Icon::SlowForward) => {
                let s = rect.size.x * 0.3125;
                let tri = app.renderer.get_mesh_id_from_name("triangle").unwrap();
                app.renderer
                    .fill_polygon(tri, center - vec2(s * 0.5, 0.0), s * 0.64, 0.0);
                app.renderer
                    .fill_polygon(tri, center + vec2(s * 0.3, 0.0), s * 0.64, 0.0);
            }
        }
    }
}

impl<Msg> Button<Msg> {
    /// Creates a textu button
    pub fn text(size: Vec2, label: impl Into<String>) -> Self {
        Self::with_content(size, ButtonContent::Text(label.into()))
    }

    pub fn fit(label: impl Into<String>, font: FontId, app: &App, pad: Vec2) -> Self {
        let label = label.into();
        let size = app.renderer.get_font_from_id(font).unwrap().measure(&label) + pad * 2.0;
        Self::with_content(size, ButtonContent::Text(label))
    }

    pub fn icon(size: Vec2, icon: Icon) -> Self {
        Self::with_content(size, ButtonContent::Icon(icon))
    }

    pub fn icon_bound(size: Vec2, icon_src: Rc<Cell<Icon>>) -> Self {
        let mut retval = Self::with_content(size, ButtonContent::Icon(icon_src.get()));
        retval.icon_src = Some(icon_src);
        retval
    }

    fn with_content(size: Vec2, content: ButtonContent) -> Self {
        Self {
            rect: Rectangle {
                pos: vec2(0.0, 0.0),
                size,
            },
            content,
            icon_src: None,
            text_color: vec4(1.0, 1.0, 1.0, 1.0),
            inactive_text_color: vec4(1.0, 1.0, 1.0, 1.0),
            background_color: vec4(1.0, 0.0, 1.0, 255.0),
            inactive_background_color: vec4(1.0, 0.0, 1.0, 255.0),
            hovered_color: vec4(1.0, 0.0, 1.0, 255.0),
            hovered_text_color: vec4(1.0, 0.0, 1.0, 255.0),
            border: None,
            inactive_border: None,
            on_click: None,
            active: true,
            active_src: None,
            hovered: false,
        }
    }

    #[allow(dead_code)]
    pub fn background_color(mut self, background_color: Vec4) -> Self {
        self.background_color = background_color;
        self
    }

    #[allow(dead_code)]
    pub fn hovered_color(mut self, hovered_color: Vec4) -> Self {
        self.hovered_color = hovered_color;
        self
    }

    pub fn use_style(mut self, style: &Style) -> Self {
        self.text_color = style.text;
        self.inactive_text_color = style.text_disabled;
        self.background_color = style.surface;
        self.inactive_background_color = style.surface_deep;
        self.border = Some((style.border, 1.0));
        self.inactive_border = Some((style.border_subtle, 1.0));
        self.hovered_color = style.surface_hover;
        self.hovered_text_color = style.text;
        self
    }

    pub fn use_style_accented(mut self, style: &Style) -> Self {
        self.text_color = style.text;
        self.inactive_text_color = style.text_disabled;
        self.background_color = style.accent_surface;
        self.inactive_background_color = style.surface_deep;
        self.border = Some((style.accent, 1.0));
        self.inactive_border = Some((style.border_subtle, 1.0));
        self.hovered_color = style.accent_surface_hover;
        self.hovered_text_color = style.text;
        self
    }

    pub fn use_style_link(mut self, style: &Style) -> Self {
        self.text_color = style.accent;
        self.inactive_text_color = style.text_disabled;
        self.background_color = vec4(0.0, 0.0, 0.0, 0.0);
        self.inactive_background_color = vec4(0.0, 0.0, 0.0, 0.0);
        self.hovered_color = vec4(0.0, 0.0, 0.0, 0.0);
        self.hovered_text_color = style.accent_bright;
        self.border = None;
        self.inactive_border = None;
        self
    }

    #[allow(dead_code)]
    pub fn use_style_if(self, style: &Style, condition: bool) -> Self {
        if !condition {
            return self;
        }
        self.use_style(style)
    }

    #[allow(dead_code)]
    pub fn use_style_accented_if(self, style: &Style, condition: bool) -> Self {
        if !condition {
            return self;
        }
        self.use_style_accented(style)
    }

    pub fn on_click(mut self, msg: Msg) -> Self {
        self.on_click = Some(msg);
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn bound_active(mut self, active_src: Rc<Cell<bool>>) -> Self {
        self.active_src = Some(active_src);
        self
    }
}

impl<Msg: Clone + 'static> Widget<Msg> for Button<Msg> {
    fn update(&mut self, app: &App, msgq: &mut MsgQueue<Msg>) {
        if let Some(src) = &self.active_src {
            self.active = src.get();
        }
        if let Some(src) = &self.icon_src {
            self.content = ButtonContent::Icon(src.get());
        }

        self.hovered = self.active && app.mouse_over(&self.rect);

        if self.active && self.hovered && !app.is_click_consumed() && app.mouse_left_clicked {
            if let Some(msg) = &self.on_click {
                msgq.push(msg.clone());
                app.consume_click();
            }
        }
    }

    fn render(&self, app: &App) {
        // Draw background
        let (background_color, content_color) = if !self.active {
            (self.inactive_background_color, self.inactive_text_color)
        } else if self.hovered {
            (self.hovered_color, self.hovered_text_color)
        } else {
            (self.background_color, self.text_color)
        };
        app.renderer.set_color(background_color);
        app.renderer.fill_rect(self.rect);

        // Draw border
        if self.active {
            if let Some((border_color, border_size)) = self.border {
                app.renderer.set_color(border_color);
                app.renderer.draw_rect(self.rect, border_size);
            }
        } else {
            if let Some((border_color, border_size)) = self.inactive_border {
                app.renderer.set_color(border_color);
                app.renderer.draw_rect(self.rect, border_size);
            }
        }

        // Draw content
        app.renderer.set_color(content_color);
        self.content.draw(&self.rect, app);
    }

    fn size(&self) -> Vec2 {
        self.rect.size
    }

    fn layout(&mut self, pos: Vec2) {
        self.rect.pos = pos
    }
}
