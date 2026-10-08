use apricot::app::App;
use hecs::{Entity, World};
use nalgebra_glm::{vec2, Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    container,
    sim::{
        docking::dock_tree,
        hierarchy::Named,
        resources::{station_resource_totals, Pool, Resource},
        transfer::Transfer,
    },
    ui::{
        button::Button,
        container::{Align, Container, Flow, Justify},
        hrule::HRule,
        label::Label,
        modal::Modal,
        progress_bar::ProgressBar,
        scroll_container::ScrollContainer,
        style::STYLE,
        widget::{recv_msgs, Widget},
    },
};

#[derive(Clone, Debug)]
enum TransferMessages {
    SelectFrom(Pool),
    SelectTo(Pool),
    Transfer,
    Cancel(Entity),
    Close,
}

pub struct TransferUi {
    modal: Modal<TransferMessages>,
    craft: Option<Entity>,
    from: Option<Pool>,
    to: Option<Pool>,
}

pub enum TransferResult {
    Start(Pool, Pool),
    Cancel(Entity),
}

impl TransferUi {
    pub fn new() -> Self {
        Self {
            modal: Modal::new(Box::new(container!())),
            craft: None,
            from: None,
            to: None,
        }
    }

    pub fn show(&mut self, craft: Entity, now: EphemerisTime, world: &World, app: &App) {
        self.craft = Some(craft);
        self.from = None;
        self.to = None;
        self.rebuild(now, world, app);
        self.modal.set_shown(true);
    }

    pub fn update(
        &mut self,
        now: EphemerisTime,
        world: &World,
        app: &App,
    ) -> Option<TransferResult> {
        let mut dirty = false;
        let mut action = None;

        for msg in recv_msgs(app, &mut self.modal) {
            match msg {
                TransferMessages::SelectFrom(p) => {
                    self.from = Some(p);
                    // a new source invalidates the to's
                    if self
                        .to
                        .is_some_and(|t| t.resource != p.resource || t.host == p.host)
                    {
                        self.to = None
                    }
                    dirty = true
                }
                TransferMessages::SelectTo(p) => {
                    self.to = Some(p);
                    dirty = true;
                }
                TransferMessages::Transfer => {
                    if let (Some(from), Some(to)) = (self.from, self.to) {
                        self.from = None;
                        self.to = None;
                        action = Some(TransferResult::Start(from, to))
                    }
                }
                TransferMessages::Cancel(e) => action = Some(TransferResult::Cancel(e)),
                TransferMessages::Close => {
                    self.modal.set_shown(false);
                }
            }
        }

        if dirty {
            self.rebuild(now, world, app);
        }

        action
    }

    pub fn rebuild(&mut self, now: EphemerisTime, world: &World, app: &App) {
        let Some(craft) = self.craft else { return };
        let font_big = app.renderer.get_font_id_from_name("font-big").unwrap();
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        const COL_W: f32 = 220.0;
        const HEIGHT: f32 = 360.0;

        let mut left: Vec<Box<dyn Widget<TransferMessages>>> =
            vec![Box::new(Label::new("FROM").font(font_small_bold, app))];
        let mut right: Vec<Box<dyn Widget<TransferMessages>>> =
            vec![Box::new(Label::new("TO").font(font_small_bold, app))];

        let transfers: Vec<Transfer> = world.query::<&Transfer>().iter().map(|(_, t)| *t).collect();
        let pumping_out = |p: Pool| transfers.iter().any(|t| t.from == p);
        let pumping_in = |p: Pool| transfers.iter().any(|t| t.to == p);

        for host in dock_tree(world, craft) {
            let name = world
                .get::<&Named>(host)
                .map_or_else(|_| "???".to_string(), |s| s.name.to_uppercase());
            let mut left_rows = vec![];
            let mut right_rows = vec![];

            for &r in Resource::ALL {
                let (stored, cap) = station_resource_totals(world, host, r, now);
                if cap <= 0.0 {
                    continue;
                }
                let pool = Pool { host, resource: r };

                left_rows.push(pool_card(
                    pool,
                    stored,
                    cap,
                    self.from == Some(pool),
                    stored > 0.0 && !pumping_out(pool) && !pumping_in(pool),
                    TransferMessages::SelectFrom(pool),
                    COL_W,
                ));

                if self.from.is_some_and(|f| f.resource == r && f.host != host) {
                    right_rows.push(pool_card(
                        pool,
                        stored,
                        cap,
                        self.to == Some(pool),
                        stored < cap && !pumping_out(pool),
                        TransferMessages::SelectTo(pool),
                        COL_W,
                    ))
                }
            }

            // Only head a group if it has something under it
            if !left_rows.is_empty() {
                left.push(Box::new(
                    Label::new(name.clone()).font(font_small_bold, app),
                ));
                left.extend(left_rows);
            }
            if !right_rows.is_empty() {
                right.push(Box::new(Label::new(name).font(font_small_bold, app)));
                right.extend(right_rows);
            }
        }

        let columns = Container::new(vec![
            Box::new(ScrollContainer::new(
                vec2(COL_W, HEIGHT),
                Box::new(Container::new(left).padding(Vec2::zeros()).gap(6.0)),
            )),
            Box::new(ScrollContainer::new(
                vec2(COL_W, HEIGHT),
                Box::new(Container::new(right).padding(Vec2::zeros()).gap(6.0)),
            )),
        ])
        .flow(Flow::Horizontal);

        let mut widgets: Vec<Box<dyn Widget<TransferMessages>>> = vec![
            Box::new(Label::new("TRANSFER RESOURCES").font(font_big, app)),
            Box::new(HRule::new(STYLE.border, 1.0, COL_W * 2.0)),
            Box::new(columns),
            Box::new(
                Button::text(vec2(COL_W * 2.0, 30.0), "Start transfer")
                    .use_style(&STYLE)
                    .active(self.from.is_some() && self.to.is_some())
                    .on_click(TransferMessages::Transfer),
            ),
            Box::new(HRule::new(STYLE.border, 1.0, COL_W * 2.0)),
        ];

        widgets.push(Box::new(
            Label::new("ACTIVE TRANSFERS").font(font_small_bold, app),
        ));
        widgets.push(transfer_list(world, craft, COL_W * 2.0, app));
        widgets.push(Box::new(HRule::new(STYLE.border, 1.0, COL_W * 2.0)));

        widgets.push(Box::new(
            Button::text(vec2(COL_W * 2.0, 30.0), "Close")
                .use_style(&STYLE)
                .on_click(TransferMessages::Close),
        ));

        self.modal = Modal::new(Box::new(
            Container::new(widgets)
                .cross_align(Align::Center)
                .background_color(STYLE.surface)
                .border(STYLE.border, 1.0)
                .padding(vec2(12.0, 12.0)),
        ))
        .shown(self.modal.is_shown());

        self.modal.reposition(app);
    }

    pub fn render(&self, app: &App) {
        self.modal.render(app);
    }

    pub fn is_shown(&self) -> bool {
        self.modal.is_shown()
    }
}

fn pool_card(
    pool: Pool,
    stored: f32,
    cap: f32,
    selected: bool,
    active: bool,
    msg: TransferMessages,
    width: f32,
) -> Box<dyn Widget<TransferMessages>> {
    let (unit, _) = pool.resource.presentation_units();
    let (scale, _) = pool.resource.presentation_scalars();
    let text = format!(
        "{} {:.0}/{:.0} {unit}",
        pool.resource.long_name(),
        stored * scale,
        cap * scale
    );

    let button = Button::text(vec2(width, 26.0), text)
        .active(active)
        .on_click(msg);
    let button = if selected {
        button.use_style_accented(&STYLE)
    } else {
        button.use_style(&STYLE)
    };

    Box::new(
        container!(
            button,
            ProgressBar::new(vec2(width, 4.0))
                .use_style(&STYLE)
                .progress(stored / cap)
        )
        .padding(Vec2::zeros())
        .gap(2.0),
    )
}

fn transfer_label(world: &World, t: &Transfer) -> String {
    let name = |host: Entity| {
        world
            .get::<&Named>(host)
            .map_or_else(|_| "???".to_string(), |n| n.name.clone())
    };
    format!(
        "{}: {} to {}",
        t.from.resource.long_name(),
        name(t.from.host),
        name(t.to.host)
    )
}

fn modal_transfer_row(
    world: &World,
    e: Entity,
    t: &Transfer,
    width: f32,
    app: &App,
) -> Box<dyn Widget<TransferMessages>> {
    let font = app.renderer.get_font_id_from_name("font").unwrap();

    Box::new(
        Container::new(vec![
            Box::new(Label::new(transfer_label(world, t)).font(font, app)),
            Box::new(
                Button::text(vec2(70.0, 24.0), "Cancel")
                    .use_style(&STYLE)
                    .on_click(TransferMessages::Cancel(e)),
            ),
        ])
        .flow(Flow::Horizontal)
        .justify(Justify::SpaceBetween)
        .cross_align(Align::Center)
        .padding(Vec2::zeros())
        .fixed_width(vec2(width, 0.0)),
    )
}

fn transfer_list(
    world: &World,
    craft: Entity,
    width: f32,
    app: &App,
) -> Box<dyn Widget<TransferMessages>> {
    const LIST_H: f32 = 120.0;
    let font_italic = app
        .renderer
        .get_font_id_from_name("font-small-italic")
        .unwrap();
    let tree = dock_tree(world, craft);

    // Sort by label, so rows don't jump around between rebuilds
    let mut transfers: Vec<(String, Entity, Transfer)> = world
        .query::<&Transfer>()
        .iter()
        .filter(|(_, t)| tree.contains(&t.from.host))
        .map(|(e, t)| (transfer_label(world, t), e, *t))
        .collect();
    transfers.sort_by(|a, b| a.0.cmp(&b.0));

    let rows: Vec<Box<dyn Widget<TransferMessages>>> = if transfers.is_empty() {
        vec![Box::new(
            Label::new("No transfers")
                .font(font_italic, app)
                .color(STYLE.text_secondary),
        )]
    } else {
        transfers
            .iter()
            .map(|(_, e, t)| modal_transfer_row(world, *e, t, width, app))
            .collect()
    };

    Box::new(ScrollContainer::new(
        vec2(width, LIST_H),
        Box::new(Container::new(rows).padding(Vec2::zeros()).gap(4.0)),
    ))
}
