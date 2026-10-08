use std::{cell::RefCell, rc::Rc};

use hecs::{Entity, World};
use nalgebra_glm::{vec2, Vec2};

use crate::{
    astro::epoch::EphemerisTime,
    hud::{
        panel::{CommandMessages, PanelCtx},
        Binding, Section,
    },
    sim::{
        docking::Docking,
        hierarchy::Named,
        mission::ScheduledBurn,
        propulsion::{craft_dv, Craft},
    },
    ui::{
        button::Button,
        container::{Container, Flow, Justify},
        hrule::HRule,
        label::Label,
        style::STYLE,
    },
};

pub fn craft_selection(ctx: &PanelCtx, selected: Entity) -> Section {
    let mut out = Section::default();

    const WIDTH: f32 = 280.0;
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();

    let craft_dv_text = Rc::new(RefCell::new(String::new()));

    if let Ok(_) = ctx.world.get::<&Docking>(selected) {
        out.push(
            Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Undock")
                .use_style(&STYLE)
                .bound_active(ctx.controls_enabled.clone())
                .on_click(CommandMessages::Undock { entity: selected }),
        );
    } else {
        out.merge(mission_section(ctx, selected));
    }

    out.push(Label::new("ENGINE").font(font_small_bold, ctx.app));

    out.push(
        Label::bound(craft_dv_text.clone())
            .font(font, ctx.app)
            .color(STYLE.text),
    );

    out.bindings.push(Binding::new({
        let craft_dv_text = craft_dv_text.clone();
        move |world: &World, now: EphemerisTime| {
            let craft_dv = craft_dv(world, selected, now);
            let s = format!("Total dv: {:.0} m/s", craft_dv);
            if *craft_dv_text.borrow() != s {
                *craft_dv_text.borrow_mut() = s;
            }
        }
    }));

    out
}

fn mission_section(ctx: &PanelCtx, selected: Entity) -> Section {
    let mut out = Section::default();

    const WIDTH: f32 = 280.0;
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font_small_italic = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-italic")
        .unwrap();

    let craft = ctx.world.get::<&Craft>(selected).unwrap();

    let is_idle = craft.command.is_none();

    out.push(Label::new("MISSION").font(font_small_bold, ctx.app));

    if let Some(command) = &craft.command {
        let (verb, target) = command.title_parts();
        let name = ctx
            .world
            .get::<&Named>(target)
            .ok()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "???".into());

        let title = format!("{verb} {name}").to_uppercase();

        let header = if craft.command_scheduled {
            title
        } else {
            format!("{} - QUEUED", title)
        };
        out.push(
            Label::new(header)
                .font(font_small_bold, ctx.app)
                .color(STYLE.accent),
        );

        for burn in command.burn_schedule() {
            out.merge(burn_card(ctx, &burn));
        }

        if !craft.command_scheduled {
            out.push(
                Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Cancel Mission")
                    .use_style(&STYLE)
                    .bound_active(ctx.controls_enabled.clone())
                    .on_click(CommandMessages::CancelCommand { craft: selected }),
            );
        }
    } else {
        out.push(
            Label::new("NO MISSION ASSIGNED")
                .font(font_small_italic, ctx.app)
                .color(STYLE.text_disabled),
        );
    }

    if is_idle {
        out.push(
            Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Plan Mission...")
                .use_style_accented(&STYLE)
                .bound_active(ctx.controls_enabled.clone())
                .on_click(CommandMessages::OpenManeuver { craft: selected }),
        )
    };
    out.push(HRule::new(STYLE.border, 1.0, WIDTH));

    out
}

fn burn_card(ctx: &PanelCtx, burn: &ScheduledBurn) -> Section {
    const WIDTH: f32 = 280.0;
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();

    let now = ctx.now;
    let done = now >= burn.t();
    let (title_color, date_color) = if done {
        (STYLE.positive, STYLE.positive)
    } else {
        (STYLE.text, STYLE.text_secondary)
    };

    let countdown = Rc::new(RefCell::new(String::new()));

    let mut out = Section::default();
    out.push(
        Container::new(vec![
            Box::new(
                Container::new(vec![
                    Box::new(
                        Label::new(burn.desc)
                            .font(font_small_bold, ctx.app)
                            .color(title_color),
                    ),
                    Box::new(
                        Label::new(format!("{:.0} m/s", burn.dv))
                            .font(font_small_bold, ctx.app)
                            .color(title_color),
                    ),
                ])
                .flow(Flow::Horizontal)
                .justify(Justify::SpaceBetween)
                .padding(Vec2::zeros())
                .fixed_width(vec2(WIDTH - 16.0, 0.0)),
            ),
            Box::new(
                Label::new(burn.t().as_calendar().unwrap_or("???".into()))
                    .font(font, ctx.app)
                    .color(date_color),
            ),
            Box::new(
                Label::bound(countdown.clone())
                    .font(font, ctx.app)
                    .color(date_color),
            ),
        ])
        .border(STYLE.border, 1.0)
        .fixed_width(vec2(WIDTH, 0.0))
        .padding(vec2(8.0, 8.0)),
    );

    out.bindings.push(Binding::new({
        let burn_t = burn.t();
        move |_world, now: EphemerisTime| {
            let s = if now >= burn_t {
                "DONE".into()
            } else {
                format!("T- {}", (burn_t - now).short_duration())
            };
            if *countdown.borrow() != s {
                *countdown.borrow_mut() = s;
            }
        }
    }));
    out
}
