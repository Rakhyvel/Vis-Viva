use std::collections::HashMap;

use crate::{
    components::{
        craft::{Craft, Payload, Stage},
        inventory::PartInventory,
        parts::{PartDef, PartRegistry},
    },
    ui::{container::Container, scroll_container::ScrollContainer, style::STYLE, vrule::VRule},
};
use apricot::{app::App, font::FontId};
use nalgebra_glm::{vec2, Vec2};

use crate::{
    container,
    ui::{
        button::Button,
        container::{Align, Flow},
        label::Label,
        modal::Modal,
        widget::{recv_msgs, Widget},
    },
};

const WIDTH: f32 = 280.0;

pub struct VabUi {
    modal: Modal<VabMessages>,

    pub stages: Vec<PartDef>,
    pub payload: Option<PartDef>,

    available_parts: HashMap<u64, u32>,
    registry: PartRegistry,
}

#[derive(Clone, Debug)]
enum VabMessages {
    AddToStack(u64),
    RemoveFromStack,
    SetPayload(u64),
    UnsetPayload,
    Build,
    Close,
}

impl VabUi {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(Box::new(container![])),
            stages: Vec::new(),
            payload: None,
            available_parts: HashMap::new(),
            registry: PartRegistry::new(),
        }
    }

    pub fn update(&mut self, app: &App) -> bool {
        for msg in recv_msgs(app, &mut self.modal) {
            println!("{msg:?}");
            match msg {
                VabMessages::AddToStack(stage_id) => {
                    let stage = self.registry.get(stage_id).unwrap();
                    self.stages.push(stage.clone());
                    let count = self.available_parts.get(&stage_id).unwrap();
                    self.available_parts.insert(stage_id, count - 1);
                    self.rebuild_modal(app);
                }
                VabMessages::SetPayload(payload_id) => {
                    let payload = self.registry.get(payload_id).unwrap();
                    self.payload = Some(payload.clone());
                    let count = self.available_parts.get(&payload_id).unwrap();
                    self.available_parts.insert(payload_id, count - 1);
                    self.rebuild_modal(app);
                }
                VabMessages::RemoveFromStack => {
                    let stage = self.stages.pop().unwrap();
                    let count = self.available_parts.get(&stage.id_hash()).unwrap();
                    self.available_parts.insert(stage.id_hash(), count + 1);
                    self.rebuild_modal(app);
                }
                VabMessages::UnsetPayload => {
                    let old_id = self.payload.clone().unwrap().id_hash();
                    self.payload = None;
                    self.available_parts.insert(old_id, 1);
                    self.rebuild_modal(app);
                }
                VabMessages::Build => {
                    self.modal.set_shown(false);
                    return true;
                }
                VabMessages::Close => {
                    self.modal.set_shown(false);
                }
            }
        }

        false
    }

    pub fn render(&self, app: &App) {
        self.modal.render(app);
    }

    pub fn show(&mut self, inventory: &PartInventory, registry: &PartRegistry, app: &App) {
        self.available_parts = inventory.parts.clone();
        self.stages = vec![];
        self.payload = None;
        self.registry = registry.clone();

        self.rebuild_modal(app);
    }

    pub fn is_shown(&self) -> bool {
        self.modal.is_shown()
    }

    fn rebuild_modal(&mut self, app: &App) {
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_big: FontId = app.renderer.get_font_id_from_name("font-big").unwrap();

        let (total_dv, twr) = self.calc_craft_dv_twr().unwrap_or((0.0, 0.0));
        const DV_TO_LAUNCH_FROM_EARTH: f64 = 8294.0;
        let good_dv = total_dv > DV_TO_LAUNCH_FROM_EARTH;
        let good_twr = twr > 1.0;

        self.modal = Modal::new(Box::new(
            container![
                // Title
                Label::new("Vehicle Assembly Building:").font(font_big, app),
                // Middle section
                container![
                    Container::new(self.build_rocket(font, app))
                        .flow(Flow::Vertical)
                        .cross_align(Align::Center),
                    VRule::new(STYLE.border, 1.0, 280.0),
                    Container::new(self.build_available_parts(font, app))
                        .flow(Flow::Vertical)
                        .cross_align(Align::Center),
                ]
                .flow(Flow::Horizontal)
                .cross_align(Align::Start),
                // Bottom  row
                container![
                    Label::new(format!("Total dv: {total_dv:.0} m/s"))
                        .font(font, app)
                        .color(if good_dv {
                            STYLE.positive
                        } else {
                            STYLE.warning
                        }),
                    Label::new(format!("TWR: {twr:.2}"))
                        .font(font, app)
                        .color(if good_twr {
                            STYLE.positive
                        } else {
                            STYLE.warning
                        }),
                ]
                .padding(vec2(32.0, 0.0))
                .flow(Flow::Horizontal)
                .cross_align(Align::Center),
                // Bottom bottom row
                container![
                    Button::text(vec2(200.0, 30.0,), "Close",)
                        .use_style(&STYLE)
                        .on_click(VabMessages::Close),
                    Button::text(vec2(200.0, 30.0,), "Build!",)
                        .use_style_accented(&STYLE)
                        .on_click(VabMessages::Build)
                        .active(good_dv && good_twr)
                ]
                .flow(Flow::Horizontal)
                .cross_align(Align::Center),
            ]
            .cross_align(Align::Center)
            .background_color(STYLE.surface),
        ))
        .shown(true);
        self.modal.reposition(app);
    }

    fn build_rocket(&self, font: FontId, app: &App) -> Vec<Box<dyn Widget<VabMessages>>> {
        let mut widgets: Vec<Box<dyn Widget<VabMessages>>> = vec![];
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        widgets.push(Box::new(Label::new("Stack:").font(font_small_bold, app)));

        if let Some(payload) = &self.payload {
            widgets.push(Box::new(
                Container::new(vec![
                    Box::new(
                        Container::new(vec![
                            Box::new(Label::new("Payload").font(font_small_bold, app)),
                            Box::new(Label::new(payload.name.clone()).font(font, app)),
                        ])
                        .flow(Flow::Vertical)
                        .fixed_width(vec2(WIDTH * 0.8 - 24.0, 10.0)),
                    ),
                    Box::new(
                        Container::new(vec![Box::new(
                            Button::<VabMessages>::text(vec2(45.0, 25.0), "X")
                                .use_style(&STYLE)
                                .on_click(VabMessages::UnsetPayload),
                        )])
                        .fixed_width(vec2(WIDTH * 0.2, 10.0))
                        .flow(Flow::Horizontal),
                    ),
                ])
                .border(STYLE.border, 1.0)
                .cross_align(Align::Center)
                .min_size(vec2(WIDTH, 69.0))
                .flow(Flow::Horizontal),
            ));
        } else {
            widgets.push(Box::new(
                Container::new(vec![Box::new(
                    Label::new("No payload")
                        .font(font, app)
                        .color(STYLE.text_disabled),
                )])
                .border(STYLE.border, 1.0)
                .fixed_width(vec2(WIDTH, 10.0))
                .cross_align(Align::Center)
                .min_size(vec2(WIDTH, 69.0))
                .flow(Flow::Horizontal),
            ));
        }

        // Stages from top to bottom (last in vec = bottom stage = burns first)
        let mut stages_widgets: Vec<Box<dyn Widget<VabMessages>>> = vec![];
        for (i, stage) in self.stages.iter().enumerate() {
            let stage_num = self.stages.len() - i;
            let is_bottom = i == self.stages.len() - 1;
            stages_widgets.push(Box::new(
                Container::new(vec![
                    Box::new(
                        Container::new(vec![
                            Box::new(
                                Label::new(format!("Stage {stage_num}")).font(font_small_bold, app),
                            ),
                            Box::new(Label::new(stage.name.clone()).font(font, app)),
                        ])
                        .flow(Flow::Vertical)
                        .fixed_width(vec2(WIDTH * 0.8 - 24.0, 10.0)),
                    ),
                    Box::new(
                        Container::new(vec![Box::new(
                            Button::<VabMessages>::text(vec2(45.0, 25.0), "X")
                                .use_style(&STYLE)
                                .on_click(VabMessages::RemoveFromStack)
                                .active(is_bottom), // only bottom stage can be removed
                        )])
                        .fixed_width(vec2(WIDTH * 0.2, 10.0))
                        .flow(Flow::Horizontal),
                    ),
                ])
                .border(STYLE.border, 1.0)
                .cross_align(Align::Center)
                .flow(Flow::Horizontal),
            ));
        }

        if self.stages.is_empty() {
            widgets.push(Box::new(
                Container::new(vec![Box::new(
                    Label::new("No stages")
                        .font(font, app)
                        .color(STYLE.text_disabled),
                )])
                .border(STYLE.border, 1.0)
                .min_size(Vec2::new(WIDTH, 300.0))
                .cross_align(Align::Center)
                .flow(Flow::Horizontal),
            ));
        } else {
            widgets.push(Box::new(ScrollContainer::new(
                Vec2::new(WIDTH, 300.0),
                Box::new(Container::new(stages_widgets).padding(vec2(0.0, 8.0))),
            )))
        }

        widgets
    }

    fn calc_craft_dv_twr(&self) -> Option<(f64, f64)> {
        let payload = self.payload()?;
        let stages_stack: Vec<Stage> = self.stages().into_iter().collect::<Option<Vec<Stage>>>()?;

        let craft = Craft {
            command: None,
            command_scheduled: false,
            payload,
            stages_stack,
            line_path_entity: None,
        };

        let twr = craft.twr(todo!())?;

        Some((craft.total_remaining_dv(todo!()), twr))
    }

    fn build_available_parts(&self, font: FontId, app: &App) -> Vec<Box<dyn Widget<VabMessages>>> {
        let mut widgets: Vec<Box<dyn Widget<VabMessages>>> = vec![];
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        let payloads: Vec<Box<dyn Widget<VabMessages>>> = self
            .registry
            .all()
            .filter_map(|part| {
                let count = self.available_parts.get(&part.id_hash()).unwrap_or(&0);
                if part.fuel.is_some() {
                    return None;
                }

                let have_some = *count > 0;

                Some(Box::new(
                    Container::new(vec![
                        Box::new(
                            Container::new(vec![Box::new(
                                Label::new(format!("{} ({count})", part.name.clone()))
                                    .font(font, app),
                            )])
                            .padding(vec2(0.0, 0.0))
                            .fixed_width(vec2(WIDTH * 0.8 - 24.0, 10.0))
                            .flow(Flow::Horizontal),
                        ),
                        Box::new(
                            Container::new(vec![Box::new(
                                Button::<VabMessages>::text(vec2(45.0, 25.0), "+")
                                    .use_style(&STYLE)
                                    .on_click(VabMessages::SetPayload(part.id_hash()))
                                    .active(have_some),
                            )])
                            .padding(vec2(0.0, 0.0))
                            .fixed_width(vec2(WIDTH * 0.2, 10.0))
                            .flow(Flow::Horizontal),
                        ),
                    ])
                    .border(STYLE.border, 1.0)
                    .cross_align(Align::Center)
                    .flow(Flow::Horizontal),
                ) as Box<dyn Widget<VabMessages>>)
            })
            .collect();

        let stages: Vec<Box<dyn Widget<VabMessages>>> = self
            .registry
            .all()
            .filter_map(|part| {
                let count = self.available_parts.get(&part.id_hash()).unwrap_or(&0);
                let _ = part.fuel?;

                let have_some = *count > 0;

                Some(Box::new(
                    Container::new(vec![
                        Box::new(
                            Container::new(vec![Box::new(
                                Label::new(format!("{} ({count})", part.name.clone()))
                                    .font(font, app),
                            )])
                            .padding(vec2(0.0, 0.0))
                            .fixed_width(vec2(WIDTH * 0.8 - 24.0, 10.0))
                            .flow(Flow::Horizontal),
                        ),
                        Box::new(
                            Container::new(vec![Box::new(
                                Button::<VabMessages>::text(vec2(45.0, 25.0), "+")
                                    .use_style(&STYLE)
                                    .on_click(VabMessages::AddToStack(part.id_hash()))
                                    .active(have_some),
                            )])
                            .padding(vec2(0.0, 0.0))
                            .fixed_width(vec2(WIDTH * 0.2, 10.0))
                            .flow(Flow::Horizontal),
                        ),
                    ])
                    .border(STYLE.border, 1.0)
                    .cross_align(Align::Center)
                    .flow(Flow::Horizontal),
                ) as Box<dyn Widget<VabMessages>>)
            })
            .collect();

        widgets.push(Box::new(Label::new("Payloads:").font(font_small_bold, app)));
        widgets.push(Box::new(ScrollContainer::new(
            Vec2::new(WIDTH, 150.0),
            Box::new(Container::new(payloads).padding(vec2(0.0, 8.0))),
        )));
        widgets.push(Box::new(Label::new("Stages:").font(font_small_bold, app)));
        widgets.push(Box::new(ScrollContainer::new(
            Vec2::new(WIDTH, 150.0),
            Box::new(Container::new(stages).padding(vec2(0.0, 8.0))),
        )));

        widgets
    }

    pub fn payload(&self) -> Option<Payload> {
        self.registry
            .get(self.payload.as_ref()?.id_hash())
            .map(|part| part.instantiate_payload())
    }

    pub fn stages(&self) -> Vec<Option<Stage>> {
        self.stages
            .iter()
            .map(|stage_def| {
                self.registry
                    .get(stage_def.id_hash())
                    .map(|part| part.instantiate_stage())
            })
            .collect()
    }
}
