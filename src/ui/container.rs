use crate::ui::{msg::MsgQueue, widget::Widget};
use apricot::{app::App, rectangle::Rectangle};
use nalgebra_glm::{vec2, Vec2};

/// How widgets are stacked
pub enum Flow {
    Vertical,
    Horizontal,
}

/// How widgets are position on the flow-axis
pub enum Align {
    /// pack at beginning, padding between children
    Start,
    /// pack together, centered in the available space
    Center,
    /// pack at end, padding between children
    End,
}

/// How widgets are position on the flow-axis
pub enum Justify {
    /// pack at beginning, padding between children
    Start,
    /// pack together, centered in the available space
    Center,
    /// pack at end, padding between children
    End,
    /// distribute leftover evenly, including the ends
    SpaceAround,
    /// distribute leftover evenly, flush to both ends
    SpaceBetween,
}

pub struct Container<Msg> {
    rect: Rectangle,
    children: Vec<Box<dyn Widget<Msg>>>,
    flow: Flow,
    /// Alignment on the main axis
    justify: Justify,
    /// Alignment on the perpendicular axis
    cross_align: Align,
    fixed_width: bool,
    fixed_height: bool,
    min_size: Vec2,
    padding: Vec2,
    gap: f32,
    background: Option<nalgebra_glm::Vec4>,
    border: Option<(nalgebra_glm::Vec4, f32)>, // color, width
}

impl<Msg: Clone + 'static> Container<Msg> {
    pub fn new(children: Vec<Box<dyn Widget<Msg>>>) -> Self {
        let mut retval = Self {
            rect: Rectangle::new(0.0, 0.0, 0.0, 0.0),
            children,
            flow: Flow::Vertical,
            justify: Justify::Start,
            cross_align: Align::Start,
            fixed_width: false,
            fixed_height: false,
            min_size: Vec2::zeros(),
            padding: vec2(8.0, 8.0),
            gap: 8.0,
            background: None,
            border: None,
        };
        retval.layout(retval.rect.pos);
        retval
    }

    pub fn flow(mut self, flow: Flow) -> Self {
        self.flow = flow;
        self.layout(self.rect.pos);
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self.layout(self.rect.pos);
        self
    }

    pub fn cross_align(mut self, cross_align: Align) -> Self {
        self.cross_align = cross_align;
        self.layout(self.rect.pos);
        self
    }

    pub fn fixed_size(mut self, size: Vec2) -> Self {
        self.rect.size = size;
        self.fixed_height = true;
        self.fixed_width = true;
        self.layout(self.rect.pos);
        self
    }

    pub fn fixed_width(mut self, size: Vec2) -> Self {
        self.rect.size = size;
        self.fixed_width = true;
        self.layout(self.rect.pos);
        self
    }

    pub fn min_size(mut self, min: Vec2) -> Self {
        self.min_size = min;
        self.layout(self.rect.pos);
        self
    }

    pub fn padding(mut self, padding: Vec2) -> Self {
        self.padding = padding;
        self.layout(self.rect.pos);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self.layout(self.rect.pos);
        self
    }

    pub fn background_color(mut self, color: nalgebra_glm::Vec4) -> Self {
        self.background = Some(color);
        self
    }

    pub fn border(mut self, color: nalgebra_glm::Vec4, width: f32) -> Self {
        self.border = Some((color, width));
        self
    }
}

#[macro_export]
macro_rules! container {
    ($($widget:expr),* $(,)?) => {
        Container::new(vec![$(Box::new($widget)),*])
    }
}

impl<Msg: Clone + 'static> Widget<Msg> for Container<Msg> {
    fn overlay_update(&mut self, app: &App, msgq: &mut MsgQueue<Msg>) {
        for child in self.children.iter_mut().rev() {
            child.as_mut().overlay_update(app, msgq);
        }
    }

    fn update(&mut self, app: &App, msgq: &mut MsgQueue<Msg>) {
        for child in self.children.iter_mut().rev() {
            child.as_mut().update(app, msgq);
        }
    }

    fn render(&self, app: &App) {
        // Draw background
        if let Some(color) = self.background {
            app.renderer.set_color(color);
            app.renderer.fill_rect(self.rect);
        }

        // Draw border
        if let Some((border_color, border_size)) = self.border {
            app.renderer.set_color(border_color);
            app.renderer.draw_rect(self.rect, border_size);
        }

        for child in self.children.iter() {
            child.as_ref().render(app);
        }
    }

    fn size(&self) -> Vec2 {
        self.rect.size
    }

    fn layout(&mut self, pos: Vec2) {
        self.rect.pos = pos;

        // Offset children by padding
        let inner_pos = pos + self.padding;

        // Collect child sizes
        let child_sizes: Vec<Vec2> = self
            .children
            .iter_mut()
            .map(|child| {
                child.layout(Vec2::zeros());
                child.size()
            })
            .collect();

        let max_content_size = child_sizes
            .iter()
            .fold(Vec2::zeros(), |acc, s| nalgebra_glm::max2(&acc, s));
        let additive_content_size = child_sizes.iter().fold(Vec2::zeros(), |acc, s| acc + *s);

        let main_is_fixed = match self.flow {
            Flow::Vertical => self.fixed_height,
            Flow::Horizontal => self.fixed_width,
        };
        let n = self.children.len() as f32;
        let (main_size, additive_main) = match self.flow {
            Flow::Vertical => (
                self.rect.size.y - self.padding.y * 2.0,
                additive_content_size.y,
            ),
            Flow::Horizontal => (
                self.rect.size.x - self.padding.x * 2.0,
                additive_content_size.x,
            ),
        };
        let gap = self.gap;

        let (lead, between) = if !main_is_fixed {
            (0.0, gap)
        } else {
            let leftover = (main_size - additive_main - gap * (n - 1.0)).max(0.0);
            match self.justify {
                Justify::Start => (0.0, gap),
                Justify::Center => (leftover / 2.0, gap),
                Justify::End => (leftover, gap),
                Justify::SpaceAround => {
                    let s = ((main_size - additive_main) / (n + 1.0)).max(0.0);
                    (s, s)
                }
                Justify::SpaceBetween => {
                    if n > 1.0 {
                        (0.0, ((main_size - additive_main) / (n - 1.0)).max(0.0))
                    } else {
                        (0.0, 0.0)
                    }
                }
            }
        };

        let mut main_offset = lead;

        for (child, child_size) in self.children.iter_mut().zip(child_sizes.iter()) {
            // Cross axis: how to position perpendicular to flow
            let cross_size_available = match self.flow {
                Flow::Vertical => {
                    if self.fixed_width {
                        self.rect.size.x - self.padding.x * 2.0
                    } else {
                        max_content_size.x
                    }
                }
                Flow::Horizontal => {
                    if self.fixed_height {
                        self.rect.size.y - self.padding.y * 2.0
                    } else {
                        max_content_size.y
                    }
                }
            };
            let cross_child_size = match self.flow {
                Flow::Vertical => child_size.x,
                Flow::Horizontal => child_size.y,
            };
            let cross_offset = match self.cross_align {
                Align::Start => 0.0,
                Align::Center => cross_size_available / 2.0 - cross_child_size / 2.0,
                Align::End => cross_size_available - cross_child_size,
            };

            // Compute child position from main + cross offsets
            let child_pos = match self.flow {
                Flow::Vertical => vec2(inner_pos.x + cross_offset, inner_pos.y + main_offset),
                Flow::Horizontal => vec2(inner_pos.x + main_offset, inner_pos.y + cross_offset),
            };
            child.layout(child_pos);

            // Advance main axis and accumulate working size
            match self.flow {
                Flow::Vertical => {
                    main_offset += between + child_size.y;
                }
                Flow::Horizontal => {
                    main_offset += between + child_size.x;
                }
            }
        }

        // Size ourselves from the children we just placed
        let gaps = (n - 1.0).max(0.0) * gap;
        let (main_extent, cross_extent) = match self.flow {
            Flow::Vertical => (additive_content_size.y + gaps, max_content_size.x),
            Flow::Horizontal => (additive_content_size.x + gaps, max_content_size.y),
        };

        let (extent_x, extent_y) = match self.flow {
            Flow::Vertical => (cross_extent, main_extent),
            Flow::Horizontal => (main_extent, cross_extent),
        };

        if !self.fixed_width {
            self.rect.size.x = (extent_x + self.padding.x * 2.0).max(self.min_size.x);
        }
        if !self.fixed_height {
            self.rect.size.y = (extent_y + self.padding.y * 2.0).max(self.min_size.y);
        }
    }
}
