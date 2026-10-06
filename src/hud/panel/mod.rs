use std::{cell::Cell, rc::Rc};

use apricot::app::App;
use hecs::{Entity, World};
use nalgebra_glm::{vec2, Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    hud::{
        panel::{
            body::body_selection,
            craft::{craft_selection, transfers_section},
            modules::module_list,
        },
        Binding, Section,
    },
    sim::{
        bodies::Body,
        docking::PortHost,
        hierarchy::{ancestor_chain, Named},
        industry::Factory,
        life_support::Station,
        parts::PartRegistry,
        propulsion::Craft,
        transfer::Transfer,
    },
    ui::{
        anchor::{Anchor, AnchorPoint},
        button::Button,
        container::{Align, Container, Flow},
        hrule::HRule,
        label::Label,
        scroll_container::ScrollContainer,
        style::STYLE,
        widget::Widget,
    },
};

mod body;
mod craft;
mod modules;

#[derive(Clone)]
pub enum CommandMessages {
    OpenFabricator { fabricator_entity: Entity },
    CancelFabricator { fabricator_entity: Entity },
    ToggleFabricator { fabricator_entity: Entity },
    ToggleElectrolyzer { electrolyzer_entity: Entity },
    ToggleMiner { miner_entity: Entity },
    CancelCommand { craft: Entity },
    Undock { entity: Entity },
    SelectEntity { entity: Entity },
    OpenManeuver { craft: Entity },
    OpenTransfer { craft: Entity },
}

/// Everything the panel builders read. Borrowed from Gameplay for the duration of one rebuild.
pub struct PanelCtx<'a> {
    pub app: &'a App,
    pub world: &'a World,
    pub parts: &'a PartRegistry,
    pub now: EphemerisTime,
    /// Only clickable while paused. This changes without a rebuild, so widgets keep a clone.
    pub controls_enabled: &'a Rc<Cell<bool>>,
}

pub fn build(
    ctx: &PanelCtx,
    selected: Option<Entity>,
    footer_h: f32,
) -> (Anchor<CommandMessages>, Vec<Binding>) {
    let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> = vec![];
    let mut bindings = vec![];

    if let Some(selected) = selected {
        let section = build_selection_widgets(ctx, selected);
        widgets = section.widgets;
        bindings = section.bindings;
    }

    const MARGIN: f32 = 16.0;

    let panel_h = (ctx.app.window_size.y as f32 - MARGIN * 3.0 - footer_h).max(0.0);

    let mut anchor = Anchor::new(
        Box::new(ScrollContainer::new(
            Vec2::new(300.0, panel_h),
            Box::new(
                Container::new(widgets)
                    .cross_align(Align::Start)
                    .background_color(STYLE.surface)
                    .border(STYLE.border, 1.0)
                    .padding(vec2(8.0, 8.0))
                    .min_size(Vec2::new(300.0, panel_h)),
            ),
        )),
        AnchorPoint::TopRight,
    )
    .margin(vec2(MARGIN, MARGIN));
    anchor.reposition(ctx.app);
    (anchor, bindings)
}

fn build_selection_widgets(ctx: &PanelCtx, selected: Entity) -> Section {
    let mut out = Section::default();
    const WIDTH: f32 = 280.0;
    let font_big = ctx.app.renderer.get_font_id_from_name("font-big").unwrap();

    out.push(
        Container::new(build_crumbs(ctx, selected))
            .flow(Flow::Horizontal)
            .cross_align(Align::Center)
            .padding(vec2(0.0, 0.0)),
    );

    let name = ctx
        .world
        .get::<&Named>(selected)
        .map(|n| n.name.clone())
        .unwrap_or_else(|_| "???".into());
    out.push(Label::new(name).font(font_big, ctx.app));
    out.push(HRule::new(STYLE.border, 1.0, WIDTH));

    if let Some(transfer) = transfers_section(ctx, selected) {
        out.merge(transfer);
    }

    if ctx
        .world
        .entity(selected)
        .is_ok_and(|e| e.has::<Craft>() && !e.has::<Station>())
    {
        out.merge(craft_selection(ctx, selected));
    } else if ctx.world.get::<&Body>(selected).is_ok() {
        out.merge(body_selection(ctx, selected));
    }

    if ctx.world.get::<&PortHost>(selected).is_ok() {
        out.merge(module_list(ctx, selected));
    }

    out
}

fn build_crumbs(ctx: &PanelCtx, selected: Entity) -> Vec<Box<dyn Widget<CommandMessages>>> {
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();
    let ancestors = ancestor_chain(ctx.world, selected); // outermost first
    let mut crumbs: Vec<Box<dyn Widget<CommandMessages>>> = vec![];
    for (i, e) in ancestors.iter().enumerate() {
        if i > 0 {
            crumbs.push(Box::new(Label::new(">").font(font, ctx.app)));
        }
        let name = ctx.world.get::<&Named>(*e).unwrap().name.clone();
        crumbs.push(Box::new(
            Button::fit(name, font, ctx.app, vec2(0.0, 0.0))
                .use_style_link(&STYLE)
                .on_click(CommandMessages::SelectEntity { entity: *e }),
        ));
    }
    crumbs.reverse();
    crumbs
}

pub fn panel_structure_bits(world: &World) -> u64 {
    let mut h = 0u64;
    for (e, f) in world.query::<&Factory>().iter() {
        let s = f.current_job.is_some() as u64;
        h ^= (e.id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ s;
    }
    for (e, _) in world.query::<&Transfer>().iter() {
        h ^= (e.id() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    }
    for (e, c) in world.query::<&Craft>().iter() {
        let s = match (&c.command, c.command_scheduled) {
            (Some(_), true) => 2,
            (Some(_), false) => 1,
            _ => 0,
        };
        h ^= (e.id() as u64).wrapping_mul(0x1656_67B1_9E37_79F9) ^ s;
    }
    h
}
