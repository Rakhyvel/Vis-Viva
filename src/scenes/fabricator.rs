use apricot::{
    app::App,
    font::{Font, FontId},
};
use hecs::{Entity, World};
use nalgebra_glm::{vec2, Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    components::{
        body::Parent,
        factory::{cost_status, CostKind, CostLine, Factory},
        inventory::{self, PartInventory},
        parts::{PartDef, PartRegistry},
        station::{station_resource_totals, Resource},
    },
    container,
    ui::{
        button::Button,
        container::{Align, Container, Flow},
        hrule::HRule,
        label::Label,
        modal::Modal,
        progress_bar::ProgressBar,
        scroll_container::ScrollContainer,
        stat_row::stat_row,
        style::STYLE,
        widget::{recv_msgs, Widget},
    },
};

pub struct FabricatorUi {
    modal: Modal<FabricatorMessages>,
    fabricator: Option<Entity>,
}

#[derive(Clone, Debug)]
enum FabricatorMessages {
    Build(u64), // part id, card-local
    Close,
}

pub struct FabricatorAction {
    pub fabricator: Entity,
    pub part_id: u64,
}

impl FabricatorUi {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(Box::new(container![])),
            fabricator: None,
        }
    }

    pub fn update(&mut self, app: &App) -> Option<FabricatorAction> {
        for msg in recv_msgs(app, &mut self.modal) {
            println!("{msg:?}");
            match msg {
                FabricatorMessages::Build(part_id) => {
                    let Some(fabricator) = self.fabricator else {
                        panic!("wha!")
                    };
                    self.modal.set_shown(false);
                    return Some(FabricatorAction {
                        fabricator,
                        part_id,
                    });
                }
                FabricatorMessages::Close => {
                    self.modal.set_shown(false);
                }
            }
        }

        None
    }

    pub fn render(&self, app: &App) {
        self.modal.render(app);
    }

    pub fn show(
        &mut self,
        world: &World,
        fabricator: Entity,
        registry: &PartRegistry,
        t: EphemerisTime,
        app: &App,
    ) {
        let font_small_bold: FontId = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        self.fabricator = Some(fabricator);

        let station = world.get::<&Parent>(fabricator).unwrap().id;
        let part_inventory = world.get::<&PartInventory>(station).unwrap();
        let pending = world
            .get::<&Factory>(fabricator)
            .ok()
            .and_then(|f| f.pending_job);

        let mut parts: Vec<&PartDef> = registry.all().collect();
        parts.sort_by(|a, b| a.name.cmp(&b.name));

        let mut cards: Vec<Box<dyn Widget<FabricatorMessages>>> = Vec::new();
        for part in parts {
            let lines = cost_status(world, station, &part.cost, registry, t);
            cards.push(Box::new(self.build_card(
                part,
                &lines,
                &part.byproducts,
                registry,
                pending,
                app,
            )));
        }

        const INVENTORY_W: f32 = 134.0;
        const CARD_W: f32 = 300.0;
        const HEIGHT: f32 = 400.0;

        let close = Button::text(vec2(CARD_W, 30.0), "Close")
            .use_style(&STYLE)
            .on_click(FabricatorMessages::Close);

        let mut inventory: Vec<Box<dyn Widget<FabricatorMessages>>> = Vec::new();
        inventory.push(Box::new(
            Label::new("RESOURCES")
                .font(font_small_bold, app)
                .color(STYLE.text),
        ));

        for r in Resource::ALL {
            let (amt, cap) = station_resource_totals(world, station, *r, t);
            if cap <= 0.0 {
                continue; // the station has no store for this
            }

            let (unit, _) = r.presentation_units();
            let (scale, _) = r.presentation_scalars();
            let color = if amt > 0.0 {
                STYLE.text
            } else {
                STYLE.text_disabled
            };

            inventory.push(Box::new(
                container!(
                    stat_row(
                        &r.long_name().to_uppercase(),
                        format!("{:.0} {unit}", amt * scale),
                        color,
                        INVENTORY_W,
                        app,
                    ),
                    ProgressBar::new(vec2(INVENTORY_W, 4.0))
                        .use_style(&STYLE)
                        .progress(amt / cap),
                )
                .padding(Vec2::zeros())
                .gap(2.0),
            ));
        }

        inventory.push(Box::new(HRule::new(STYLE.border_subtle, 1.0, INVENTORY_W)));
        inventory.push(Box::new(
            Label::new("PARTS")
                .font(font_small_bold, app)
                .color(STYLE.text),
        ));

        for part in registry.all() {
            let amt = part_inventory.quantity(part.id_hash());

            inventory.push(Box::new(stat_row(
                &part.name,
                format!("{amt}"),
                if amt > 0 {
                    STYLE.text
                } else {
                    STYLE.text_disabled
                },
                INVENTORY_W,
                app,
            )))
        }

        let inventory_bar = Box::new(ScrollContainer::new(
            vec2(INVENTORY_W, HEIGHT),
            Box::new(Container::new(inventory).padding(Vec2::zeros()).gap(8.0)),
        ));

        let parts = Box::new(ScrollContainer::new(
            vec2(CARD_W, HEIGHT),
            Box::new(Container::new(cards).padding(Vec2::zeros()).gap(8.0)),
        ));

        let children: Vec<Box<dyn Widget<FabricatorMessages>>> = vec![
            Box::new(Label::new("FABRICATOR").font(font_small_bold, app)),
            Box::new(HRule::new(STYLE.border, 1.0, CARD_W)),
            Box::new(Container::new(vec![inventory_bar, parts]).flow(Flow::Horizontal)),
            Box::new(HRule::new(STYLE.border, 1.0, CARD_W)),
            Box::new(close),
        ];

        self.modal = Modal::new(Box::new(
            Container::new(children)
                .cross_align(Align::Center)
                .background_color(STYLE.surface)
                .border(STYLE.border, 1.0)
                .padding(vec2(8.0, 8.0)),
        ))
        .shown(true);
        self.modal.reposition(app);
    }

    fn build_card(
        &self,
        part: &PartDef,
        lines: &[CostLine],
        byproducts: &[(Resource, f32)],
        registry: &PartRegistry,
        pending: Option<u64>,
        app: &App,
    ) -> Container<FabricatorMessages> {
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        let id = part.id_hash();
        let affordable = lines.iter().all(|l| l.have >= l.need);
        let queued = pending == Some(id);

        let text_color = if affordable {
            STYLE.text
        } else {
            STYLE.text_disabled
        };

        const INNER_W: f32 = 280.0;

        let f = app.renderer.get_font_from_id(font).unwrap();

        let desc_lines: Vec<Box<dyn Widget<FabricatorMessages>>> = wrap(&part.desc, INNER_W, &f)
            .into_iter()
            .map(|l| {
                Box::new(Label::new(l).font(font, app).color(text_color)) as Box<dyn Widget<_>>
            })
            .collect();

        let mut widgets: Vec<Box<dyn Widget<FabricatorMessages>>> = vec![
            Box::new(
                Label::new(part.name.clone())
                    .font(font_bold, app)
                    .color(text_color),
            ),
            Box::new(Container::new(desc_lines).padding(Vec2::zeros()).gap(0.0)),
            Box::new(HRule::new(STYLE.border, 1.0, INNER_W)),
        ];

        let mut inputs_rows: Vec<Box<dyn Widget<FabricatorMessages>>> = vec![];
        for line in lines {
            let unit = match line.kind {
                CostKind::Part(..) => "",
                CostKind::Resource(..) => " kg",
            };

            let have_text = if line.need > line.have {
                format!(" (have {}{})", line.have, unit)
            } else {
                String::new()
            };

            let text = match line.kind {
                CostKind::Part(part_id) => {
                    let name = registry
                        .get(part_id)
                        .map(|p| p.name.as_str())
                        .unwrap_or("???");
                    format!("{:.0}x {}{}", line.need, name, have_text)
                }
                CostKind::Resource(r) => {
                    format!("{:.0} kg {}{}", line.need, r.long_name(), have_text)
                }
            };
            inputs_rows.push(Box::new(Label::new(text).font(font, app).color(
                if line.have >= line.need {
                    STYLE.text
                } else {
                    STYLE.negative
                },
            )));
        }

        inputs_rows.push(Box::new(
            Label::new(format!("{:.0} kWh Energy", part.cost.energy_joules / 3.6e6))
                .font(font, app)
                .color(STYLE.text),
        ));
        widgets.push(Box::new(section("INPUTS", inputs_rows, &font_bold, app)));

        if !byproducts.is_empty() {
            let mut rows: Vec<Box<dyn Widget<FabricatorMessages>>> = vec![];
            for (byproduct, amt) in byproducts {
                rows.push(Box::new(
                    Label::new(format!("+{:.0} kg {}", amt, byproduct.long_name()))
                        .font(font, app)
                        .color(STYLE.text),
                ));
            }
            widgets.push(Box::new(section("BYPRODUCTS", rows, &font_bold, app)));
        }

        widgets.push(Box::new(
            Button::text(vec2(280.0, 30.0), if queued { "QUEUED" } else { "BUILD" })
                .use_style(&STYLE)
                .active(affordable && !queued)
                .on_click(FabricatorMessages::Build(id)),
        ));

        Container::new(widgets)
            .fixed_width(vec2(300.0, 0.0))
            .background_color(STYLE.surface)
            .border(STYLE.border, 1.0)
    }

    pub fn is_shown(&self) -> bool {
        self.modal.is_shown()
    }
}

fn section(
    title: &str,
    rows: Vec<Box<dyn Widget<FabricatorMessages>>>,
    font_bold: &FontId,
    app: &App,
) -> Container<FabricatorMessages> {
    let mut children: Vec<Box<dyn Widget<FabricatorMessages>>> = vec![Box::new(
        Label::new(title)
            .font(*font_bold, app)
            .color(STYLE.text_secondary),
    )];
    children.extend(rows);
    Container::new(children).padding(Vec2::zeros()).gap(2.0)
}

fn wrap(text: &str, max_w: f32, font: &Font) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if font.measure(&candidate).x <= max_w || cur.is_empty() {
            cur = candidate;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string()
        }
    }
    if !cur.is_empty() {
        lines.push(cur)
    }
    lines
}
