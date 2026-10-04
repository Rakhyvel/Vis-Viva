use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use apricot::app::App;
use nalgebra_glm::{vec2, I32Vec2, Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    container,
    hud::{
        set_if_changed,
        timeline::{marks_digest, MarkKind, Timeline, TimelineMark},
    },
    ui::{
        anchor::{Anchor, AnchorPoint},
        button::{Button, Icon},
        container::{Align, Container, Flow, Justify},
        hrule::HRule,
        label::Label,
        msg::MsgQueue,
        scroll_container::ScrollContainer,
        shape::Shape,
        style::STYLE,
        widget::{recv_msgs, Widget},
    },
};

#[derive(Clone)]
pub enum TurnMessages {
    TogglePlay,
    SpeedUp,
    SlowDown,
}

/// This frame's values for the footer. Plain data, filled in by Gameplay.
pub struct FooterView {
    pub now: EphemerisTime,
    pub paused: bool,
    pub speed_label: &'static str,
    pub can_speed_up: bool,
    pub can_slow_down: bool,
}

pub struct Footer {
    anchor: Anchor<TurnMessages>,
    built_for: Option<(I32Vec2, u64, Option<EphemerisTime>)>,

    marks: Rc<RefCell<Vec<TimelineMark>>>,
    marks_version: u64,
    /// Whatever stopped the clock, highlighted at the top of the event list
    pause_reasons: Vec<TimelineMark>,

    // Values the widgets read between rebuilds. Set in sync()
    now: Rc<Cell<EphemerisTime>>,
    transport_icon: Rc<Cell<Icon>>,
    calendar: Rc<RefCell<String>>,
    speed_label: Rc<RefCell<String>>,
    can_speed_up: Rc<Cell<bool>>,
    can_slow_down: Rc<Cell<bool>>,
}

impl Footer {
    pub fn new() -> Self {
        Self {
            anchor: Anchor::new(Box::new(container![]), AnchorPoint::BottomRight),
            built_for: None,

            marks: Rc::new(RefCell::new(vec![])),
            marks_version: 0,
            pause_reasons: vec![],

            now: Rc::new(Cell::new(EphemerisTime::epoch())),
            transport_icon: Rc::new(Cell::new(Icon::Play)),
            calendar: Rc::new(RefCell::new(String::new())),
            speed_label: Rc::new(RefCell::new(String::new())),
            can_speed_up: Rc::new(Cell::new(true)),
            can_slow_down: Rc::new(Cell::new(true)),
        }
    }

    pub fn update(&mut self, app: &App) -> MsgQueue<TurnMessages> {
        recv_msgs(app, &mut self.anchor)
    }

    pub fn render(&self, app: &App) {
        self.anchor.render(app);
    }

    pub fn height(&self) -> f32 {
        self.anchor.size().y
    }

    pub fn set_marks(&mut self, marks: Vec<TimelineMark>) {
        self.marks_version = marks_digest(&marks);
        *self.marks.borrow_mut() = marks;
    }

    /// Snapshot what stopped the clock at `t`
    pub fn stopped_at(&mut self, t: EphemerisTime) {
        self.pause_reasons = self
            .marks
            .borrow()
            .iter()
            .filter(|m| m.t <= t)
            .cloned()
            .collect();
    }

    pub fn resumed(&mut self) {
        self.pause_reasons.clear();
    }

    /// Push this frame's values into the widgets, rebuilding if the layout changed.
    /// Returns true if the footer's height changed, so the side panel needs laying out again.
    pub fn sync(&mut self, app: &App, view: &FooterView) -> bool {
        self.now.set(view.now);
        self.transport_icon
            .set(if view.paused { Icon::Play } else { Icon::Pause });
        self.can_speed_up.set(view.can_speed_up);
        self.can_slow_down.set(view.can_slow_down);
        set_if_changed(
            &self.calendar,
            view.now.short_date().unwrap_or("???".into()),
        );
        set_if_changed(&self.speed_label, view.speed_label.to_string());

        let key = Some((
            app.window_size,
            self.marks_version,
            self.pause_reasons.first().map(|m| m.t),
        ));
        if key == self.built_for {
            return false;
        }
        self.built_for = key;
        let prev_h = self.height();
        self.anchor = self.build(app, view.now);
        self.height() != prev_h
    }

    fn build(&self, app: &App, now: EphemerisTime) -> Anchor<TurnMessages> {
        const MARGIN: f32 = 16.0;

        let mut anchor = Anchor::new(
            Box::new(
                Container::new(self.build_widgets(app, now))
                    .padding(vec2(0.0, 0.0))
                    .gap(MARGIN)
                    .cross_align(Align::End)
                    .flow(Flow::Horizontal),
            ),
            AnchorPoint::BottomRight,
        )
        .margin(vec2(MARGIN, MARGIN));
        anchor.reposition(app);
        anchor
    }

    fn build_widgets(&self, app: &App, now: EphemerisTime) -> Vec<Box<dyn Widget<TurnMessages>>> {
        let font = app.renderer.get_font_id_from_name("font").unwrap();

        const WIDTH: f32 = 300.0;
        const LIST_H: f32 = 85.0;

        let marks = self.marks.borrow();
        let mut upcoming: Vec<&TimelineMark> = marks.iter().filter(|m| m.t > now).collect();
        upcoming.sort_by_key(|m| m.t);

        let rows = self
            .pause_reasons
            .iter()
            .map(|m| event_row(m, true, app))
            .chain(upcoming.iter().map(|m| event_row(m, false, app)))
            .collect();

        let turn_controls = Container::new(vec![
            Box::new(ScrollContainer::new(
                Vec2::new(WIDTH - 16.0, LIST_H),
                Box::new(Container::new(rows).padding(Vec2::zeros()).gap(2.0)),
            )),
            Box::new(HRule::new(STYLE.border, 1.0, WIDTH - 16.0)),
            Box::new(
                Container::new(vec![
                    Box::new(
                        Button::icon_bound(vec2(30.0, 30.0), self.transport_icon.clone())
                            .use_style_accented(&STYLE)
                            .on_click(TurnMessages::TogglePlay),
                    ),
                    Box::new(
                        Button::icon(vec2(30.0, 30.0), Icon::SlowForward)
                            .use_style_accented(&STYLE)
                            .bound_active(self.can_slow_down.clone())
                            .on_click(TurnMessages::SlowDown),
                    ),
                    Box::new(
                        Button::icon(vec2(30.0, 30.0), Icon::FastForward)
                            .use_style_accented(&STYLE)
                            .bound_active(self.can_speed_up.clone())
                            .on_click(TurnMessages::SpeedUp),
                    ),
                    Box::new(
                        Container::new(vec![Box::new(
                            Label::bound(self.speed_label.clone()).font(font, app),
                        )])
                        .background_color(STYLE.surface)
                        .border(STYLE.border, 1.0)
                        .min_size(vec2(0.0, 30.0)),
                    ),
                    Box::new(
                        Container::new(vec![Box::new(
                            Label::bound(self.calendar.clone()).font(font, app),
                        )])
                        .background_color(STYLE.surface)
                        .border(STYLE.border, 1.0)
                        .min_size(vec2(0.0, 30.0)),
                    ),
                ])
                .flow(Flow::Horizontal)
                .cross_align(Align::Center)
                .padding(vec2(0.0, 0.0))
                .gap(0.0),
            ),
        ])
        .background_color(STYLE.surface)
        .border(STYLE.border, 1.0)
        .flow(Flow::Vertical)
        .cross_align(Align::Center)
        .fixed_size(Vec2::new(300.0, 150.0));

        let remaining = app.window_size.x as f32 - turn_controls.size().x - 16.0 - 32.0;

        vec![
            Box::new(
                Timeline::new(self.now.clone(), remaining, self.marks.clone()).use_style(&STYLE),
            ),
            Box::new(turn_controls),
        ]
    }
}

fn event_row(mark: &TimelineMark, accented: bool, app: &App) -> Box<dyn Widget<TurnMessages>> {
    let font = app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let (mesh, rot) = mark.kind.shape(app);

    const WIDTH: f32 = 300.0;

    let (subject_color, detail_color) = match (accented, mark.kind) {
        (true, MarkKind::Critical) => (STYLE.negative, STYLE.text_secondary),
        (true, _) => (STYLE.positive, STYLE.text_secondary),
        (false, _) => (STYLE.text, STYLE.text_secondary),
    };

    let left = container!(
        Shape::new(vec2(28.0, 28.0), mesh, rot, mark.kind.color(), 10.0),
        container!(
            container!(
                Label::new(mark.subject.clone())
                    .font(font, app)
                    .color(subject_color),
                Label::new(mark.t.short_date().unwrap_or("???".into())).font(font, app),
            )
            .flow(Flow::Horizontal)
            .justify(Justify::SpaceBetween)
            .fixed_size(vec2(WIDTH - 52.0, 16.0))
            .padding(Vec2::zeros())
            .gap(0.0),
            Label::new(mark.detail.clone())
                .font(font, app)
                .color(detail_color)
        )
        .padding(Vec2::zeros())
        .gap(0.0)
    )
    .flow(Flow::Horizontal)
    .cross_align(Align::Start)
    .padding(Vec2::zeros())
    .gap(6.0);

    Box::new(
        container!(left)
            .flow(Flow::Horizontal)
            .justify(Justify::SpaceBetween)
            .cross_align(Align::Start)
            .padding(Vec2::zeros())
            .fixed_width(vec2(WIDTH - 16.0, 0.0)),
    )
}
