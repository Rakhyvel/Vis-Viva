use apricot::app::App;
use hecs::{Entity, World};
use nalgebra_glm::{vec2, Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    components::station::{dock_tree, station_resource_totals, transferable, Resource},
    container,
    sim::hierarchy::Named,
    ui::{
        button::Button,
        container::{Align, Container, Flow},
        hrule::HRule,
        label::Label,
        modal::Modal,
        progress_bar::ProgressBar,
        scroll_container::ScrollContainer,
        style::STYLE,
        widget::{recv_msgs, Widget},
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pool {
    pub host: Entity,
    pub resource: Resource,
}

#[derive(Clone, Debug)]
enum TransferMessages {
    SelectFrom(Pool),
    SelectTo(Pool),
    Transfer,
    Close,
}

pub struct TransferUi {
    modal: Modal<TransferMessages>,
    craft: Option<Entity>,
    from: Option<Pool>,
    to: Option<Pool>,
}

pub struct TransferResult {
    pub from: Pool,
    pub to: Pool,
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
                        action = Some(TransferResult { from, to })
                    }
                }
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
                    stored > 0.0,
                    TransferMessages::SelectFrom(pool),
                    COL_W,
                ));

                if self.from.is_some_and(|f| f.resource == r && f.host != host) {
                    right_rows.push(pool_card(
                        pool,
                        stored,
                        cap,
                        self.to == Some(pool),
                        stored < cap,
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

        let summary = match (self.from, self.to) {
            (Some(from), Some(to)) => {
                let kg = transferable(world, from.host, to.host, from.resource, now);
                let (unit, _) = from.resource.presentation_units();
                let (scale, _) = from.resource.presentation_scalars();
                format!(
                    "Moves {:.0} {unit} of {}",
                    kg * scale,
                    from.resource.long_name()
                )
            }
            (Some(_), None) => String::from("Choose a destination"),
            _ => String::from("Chosoe a source"),
        };

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

        self.modal = Modal::new(Box::new(
            Container::new(vec![
                Box::new(Label::new("TRANSFER RESOURCES").font(font_big, app)),
                Box::new(HRule::new(STYLE.border, 1.0, COL_W * 2.0)),
                Box::new(columns),
                Box::new(HRule::new(STYLE.border, 1.0, COL_W * 2.0)),
                Box::new(Label::new(summary).font(font_small_bold, app)),
                Box::new(
                    Button::text(vec2(COL_W * 2.0, 30.0), "Transfer")
                        .use_style(&STYLE)
                        .active(self.from.is_some() && self.to.is_some())
                        .on_click(TransferMessages::Transfer),
                ),
                Box::new(
                    Button::text(vec2(COL_W * 2.0, 30.0), "Close")
                        .use_style(&STYLE)
                        .on_click(TransferMessages::Close),
                ),
            ])
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
