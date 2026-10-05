use std::{
    cell::{Cell, RefCell},
    f32::consts::{FRAC_PI_2, PI},
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
};

use apricot::{app::App, rectangle::Rectangle, render_core::MeshId};
use nalgebra_glm::{vec2, vec4, Vec2, Vec4};

use crate::{
    astro::epoch::EphemerisTime,
    sim::{
        docking::{Docking, PortHost},
        events::Event,
        hierarchy::Named,
        industry::Factory,
        life_support::Station,
        mission::BurnPurpose,
        propulsion::Craft,
        resources::{next_reservoir_limits, power_factor},
        Sim,
    },
    ui::{msg::MsgQueue, oklch::oklch, style::Style, widget::Widget},
};

pub struct Timeline {
    baseline_color: Vec4,
    now_color: Vec4,
    rect: Rectangle,
    start: Rc<Cell<EphemerisTime>>,
    span_years: f64,
    marks: Rc<RefCell<Vec<TimelineMark>>>,
    hovered: Option<usize>,
}

impl Timeline {
    pub const HEIGHT: f32 = 80.0;

    pub fn new(
        start: Rc<Cell<EphemerisTime>>,
        thickness: f32,
        marks: Rc<RefCell<Vec<TimelineMark>>>,
    ) -> Self {
        Self {
            baseline_color: vec4(0.0, 0.0, 0.0, 1.0),
            now_color: vec4(0.0, 0.0, 0.0, 1.0),
            rect: Rectangle::new(0.0, 0.0, thickness, Self::HEIGHT),
            start,
            span_years: 1.0 / 12.0,
            marks,
            hovered: None,
        }
    }

    pub fn use_style(mut self, style: &Style) -> Self {
        self.baseline_color = style.border;
        self.now_color = style.text;
        self
    }
}

#[derive(Clone)]
pub struct TimelineMark {
    pub t: EphemerisTime,
    pub kind: MarkKind,
    pub subject: String,
    pub detail: String,
}

#[derive(Clone, Copy, Hash)]
pub enum MarkKind {
    Burn,
    SoiChange,
    Launch,
    Land,
    Dock,
    FactoryComplete,
    Critical,
    Good,
}

pub fn marks_digest(marks: &[TimelineMark]) -> u64 {
    let mut acc: u64 = 0;
    for m in marks {
        let mut h = DefaultHasher::new();
        m.subject.hash(&mut h);
        m.detail.hash(&mut h);
        m.t.short_datetime().hash(&mut h);
        m.kind.hash(&mut h);
        acc = acc.wrapping_add(h.finish());
    }
    acc
}

pub fn build_marks(sim: &Sim) -> Vec<TimelineMark> {
    // Add hard events from the event queue
    let mut marks: Vec<TimelineMark> = sim
        .events()
        .iter()
        .flat_map(|(et, events)| {
            let t = *et;
            events.iter().filter_map(move |event| {
                let (subject, detail) = craft_name_from_event(sim, event);
                Some(TimelineMark {
                    t,
                    kind: MarkKind::from_event(event)?,
                    subject,
                    detail,
                })
            })
        })
        .collect();

    // Add pending projected factory completion events
    for (fab, (docking, f)) in sim.world().query::<(&Docking, &Factory)>().iter() {
        let (t, part_id) = if let Some(current_job) = &f.current_job {
            let factor = power_factor(sim.world(), docking.host);
            let Some(completion_et) = current_job.completion_et(f, factor, sim.clock().now())
            else {
                continue;
            };
            (completion_et, current_job.part_id)
        } else {
            continue;
        };

        let (subject, detail) = craft_name_from_event(
            sim,
            &Event::FactoryComplete {
                craft: fab,
                part_id,
            },
        );

        marks.push(TimelineMark {
            t,
            kind: MarkKind::FactoryComplete,
            subject,
            detail,
        });
    }

    // Add projected reservoir limit events, Depleted and Filled
    for (entity, (_, named)) in sim.world().query::<(&PortHost, &Named)>().iter() {
        for (et, resource, rate) in next_reservoir_limits(sim.world(), entity, sim.clock().now()) {
            if rate < 0.0 {
                marks.push(TimelineMark {
                    t: et,
                    kind: MarkKind::Critical,
                    subject: named.name.clone(),
                    detail: format!("{} Depleted", resource.long_name()),
                })
            } else {
                marks.push(TimelineMark {
                    t: et,
                    kind: MarkKind::Good,
                    subject: named.name.clone(),
                    detail: format!("{} Filled", resource.long_name()),
                })
            }
        }
    }

    // Add crew emergencies
    for (_, (station, named)) in sim.world().query::<(&Station, &Named)>().iter() {
        for e in &station.emergencies {
            marks.push(TimelineMark {
                t: e.deadline,
                kind: MarkKind::Critical,
                subject: named.name.clone(),
                detail: String::from("Crew lost"),
            });
        }
    }

    // Add projected mission burns (burns, SOI crossings, etc)
    for (_, (craft, named)) in sim.world().query::<(&Craft, &Named)>().iter() {
        let Some(command) = &craft.command else {
            continue;
        };
        if craft.command_scheduled {
            continue; // already in the event queue, don't re-add it
        }
        for burn in command.burn_schedule() {
            marks.push(TimelineMark {
                t: burn.t(),
                kind: burn.purpose.into(),
                subject: named.name.clone(),
                detail: burn.desc.to_string(),
            });
        }
        for (label, et) in command.transition_schedule() {
            marks.push(TimelineMark {
                t: et,
                kind: MarkKind::SoiChange,
                subject: named.name.clone(),
                detail: label.to_string(),
            });
        }
    }

    marks.sort_by_key(|m| m.t);
    marks
}

fn craft_name_from_event(sim: &Sim, event: &Event) -> (String, String) {
    match event {
        Event::SoiChange { craft, desc, .. } | Event::Burn { craft, desc, .. } => {
            let named = sim.world().get::<&Named>(*craft).unwrap();
            (named.name.clone(), desc.to_string())
        }

        Event::Launch { craft } | Event::Land { craft } | Event::Dock { craft, .. } => {
            let named = sim.world().get::<&Named>(*craft).unwrap();
            (named.name.clone(), "???".to_string())
        }

        Event::FactoryComplete { craft, part_id } => {
            let parent = sim.world().get::<&Docking>(*craft).unwrap().host;
            let named = sim.world().get::<&Named>(parent).unwrap();
            let part_def = sim.parts().get(*part_id).map_or("???", |p| p.name.as_str());
            (named.name.clone(), part_def.to_string())
        }

        // No real craft name
        Event::CompleteCommand { .. } => (String::from(""), String::from("")),
    }
}

impl From<BurnPurpose> for MarkKind {
    fn from(p: BurnPurpose) -> Self {
        match p {
            BurnPurpose::Maneuver => MarkKind::Burn,
            BurnPurpose::Landing => MarkKind::Land,
            BurnPurpose::Launch => MarkKind::Launch,
        }
    }
}

impl MarkKind {
    pub fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::SoiChange { .. } => Some(MarkKind::SoiChange),
            Event::Burn { purpose, .. } => Some((*purpose).into()),

            Event::FactoryComplete { .. } => Some(MarkKind::FactoryComplete),

            Event::Dock { .. } => Some(MarkKind::Dock),

            // The burns for these events already display these
            Event::Launch { .. } | Event::Land { .. } => None,
            // Don't show this to the player
            Event::CompleteCommand { .. } => None,
        }
    }

    pub fn color(&self) -> Vec4 {
        const MARK_L: f32 = 0.75;
        const MARK_C: f32 = 0.13;
        match *self {
            MarkKind::Burn => oklch(MARK_L, MARK_C, 60.0, 1.0),
            MarkKind::Launch => oklch(MARK_L, MARK_C, 60.0, 1.0),
            MarkKind::Land => oklch(MARK_L, MARK_C, 60.0, 1.0),
            MarkKind::Dock => oklch(MARK_L, MARK_C, 204.0, 1.0),
            MarkKind::SoiChange => oklch(MARK_L, MARK_C, 265.0, 1.0),
            MarkKind::FactoryComplete => oklch(MARK_L, MARK_C, 330.0, 1.0),
            MarkKind::Good => oklch(MARK_L, MARK_C, 144.0, 1.0),

            // Different lightness and chroma to stand out
            MarkKind::Critical => oklch(0.6, 0.22, 25.0, 1.0),
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            MarkKind::Burn => "Burn",
            MarkKind::SoiChange => "SOI Crossing",
            MarkKind::Launch => "Launch",
            MarkKind::Land => "Land",
            MarkKind::Dock => "Dock",
            MarkKind::FactoryComplete => "Part Complete",
            MarkKind::Critical => "CRITICAL!",
            MarkKind::Good => "Info",
        }
    }

    pub fn shape(&self, app: &App) -> (MeshId, f32) {
        match self {
            MarkKind::Burn => (
                // cause its pointy?
                app.renderer
                    .get_mesh_id_from_name("square-outline")
                    .unwrap(),
                0.0,
            ),
            MarkKind::SoiChange => (
                // special shape
                app.renderer
                    .get_mesh_id_from_name("hexagon-outline")
                    .unwrap(),
                0.0,
            ),
            MarkKind::Launch => (
                // triangle pointing up
                app.renderer
                    .get_mesh_id_from_name("triangle-outline")
                    .unwrap(),
                FRAC_PI_2,
            ),
            MarkKind::Land => (
                // triangle pointing down
                app.renderer
                    .get_mesh_id_from_name("triangle-outline")
                    .unwrap(),
                -FRAC_PI_2,
            ),
            MarkKind::Dock => (
                // I want a better one than this one
                app.renderer
                    .get_mesh_id_from_name("triangle-outline")
                    .unwrap(),
                PI,
            ),
            MarkKind::FactoryComplete => (
                // like a box
                app.renderer
                    .get_mesh_id_from_name("square-outline")
                    .unwrap(),
                45.0f32.to_radians(),
            ),
            MarkKind::Critical => (
                // stop-sign
                app.renderer
                    .get_mesh_id_from_name("octagon-outline")
                    .unwrap(),
                22.5f32.to_radians(),
            ),
            MarkKind::Good => (
                // triangle pointing up
                app.renderer
                    .get_mesh_id_from_name("triangle-outline")
                    .unwrap(),
                FRAC_PI_2,
            ),
        }
    }
}

impl<Msg: Clone + 'static> Widget<Msg> for Timeline {
    fn update(&mut self, app: &App, _msgq: &mut MsgQueue<Msg>) {
        const MARK_R: f32 = 8.5;
        let cy = self.baseline_y();
        let shape_cy = cy - Self::EVENT_TICK_HEIGHT - 13.0;

        let m = app.mouse_pos;

        self.hovered = self
            .marks
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(i, mark)| {
                let x = self.x_for(mark.t)?;
                let dx = (m.x - x).abs();
                (dx <= MARK_R && m.y >= shape_cy - MARK_R && m.y <= cy).then_some((i, dx))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
    }

    fn render(&self, app: &App) {
        let timeline_baseline: f32 = self.baseline_y();

        let timeline = Rectangle {
            pos: vec2(self.rect.pos.x, timeline_baseline),
            size: vec2(self.rect.size.x, 1.0),
        };

        app.renderer.set_scissor(Some(self.rect));

        app.renderer.set_color(self.baseline_color);
        app.renderer.fill_rect(timeline);

        // Draw the ticks
        let step = EphemerisTime::from_days(7.0);
        let mut et = self.start.get().ceil_to(step);
        while let Some(x) = self.x_for(et) {
            const TICK_HEIGHT: f32 = 13.0;
            let tick = Rectangle {
                pos: vec2(x, timeline_baseline),
                size: vec2(1.0, TICK_HEIGHT),
            };
            app.renderer.set_color(self.baseline_color);
            app.renderer.fill_rect(tick);

            let mon = et.short_month_name().unwrap_or("???");
            let day = et.day_of_month().unwrap_or("???".into());
            let label = format!("{} {}", mon, day);
            let font_id = app.renderer.get_current_font_id().unwrap();
            let font = app.renderer.get_font_from_id(font_id).unwrap();
            let width = font.measure(&label).x;

            app.renderer.set_color(self.now_color);
            app.renderer.draw_text(
                vec2(x - width * 0.5, timeline_baseline + TICK_HEIGHT + 6.0),
                &label,
            );
            et += step;
        }

        // Draw events
        for (i, timeline_event) in self.marks.borrow().iter().enumerate() {
            let Some(x) = self.x_for(timeline_event.t) else {
                continue;
            };
            let event_tick = Rectangle {
                pos: vec2(x, timeline_baseline - Self::EVENT_TICK_HEIGHT),
                size: vec2(1.0, Self::EVENT_TICK_HEIGHT),
            };

            let color = timeline_event.kind.color().xyz().push(
                if self.hovered.is_some() && self.hovered.unwrap() != i {
                    0.4
                } else {
                    1.0
                },
            );
            app.renderer.set_color(color);
            app.renderer.fill_rect(event_tick);
            let (mesh_id, rotation) = timeline_event.kind.shape(app);
            app.renderer.fill_polygon(
                mesh_id,
                vec2(event_tick.pos.x + 1.0, event_tick.pos.y - 10.0),
                8.5,
                rotation,
            );
        }

        // Draw the hovered event
        if let Some(hovered) = self.hovered {
            if self.marks.borrow().len() > hovered {
                let timeline_event = &self.marks.borrow()[hovered];

                if let Some(x) = self.x_for(timeline_event.t) {
                    let event_tick = Rectangle {
                        pos: vec2(x, timeline_baseline - Self::EVENT_TICK_HEIGHT),
                        size: vec2(1.0, Self::EVENT_TICK_HEIGHT),
                    };

                    app.renderer.set_color(self.now_color);
                    app.renderer.draw_text(
                        vec2(event_tick.pos.x + 15.0, event_tick.pos.y - 3.0),
                        &format!("{} - {}", timeline_event.subject, timeline_event.detail),
                    );
                    let color = timeline_event.kind.color();
                    let event_label = String::from(timeline_event.kind.describe());
                    app.renderer.set_color(color);
                    app.renderer.draw_text(
                        vec2(event_tick.pos.x + 15.0, event_tick.pos.y - 18.0),
                        &event_label,
                    );
                }
            } else {
                println!("timeline.rs: hovered index was greater than the `marks` vec size");
            }
        }

        // Draw the "now" cursor
        let now_x = self.x_for(self.start.get()).unwrap();
        let now = Rectangle {
            pos: vec2(now_x, self.rect.pos.y),
            size: vec2(2.0, self.rect.size.y),
        };
        app.renderer.set_color(self.now_color);
        app.renderer.fill_rect(now);

        app.renderer.set_scissor(None);
    }

    fn size(&self) -> Vec2 {
        self.rect.size
    }

    fn layout(&mut self, pos: Vec2) {
        self.rect.pos = pos;
    }
}

impl Timeline {
    const EVENT_TICK_HEIGHT: f32 = 14.0;

    fn x_for(&self, t: EphemerisTime) -> Option<f32> {
        let frac = (t.as_years() - self.start.get().as_years()) / self.span_years;
        (0.0..=1.0)
            .contains(&frac)
            .then(|| self.rect.pos.x + frac as f32 * self.rect.size.x)
    }

    fn baseline_y(&self) -> f32 {
        self.rect.pos.y + self.rect.size.y * 0.5
    }
}
