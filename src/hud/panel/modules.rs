use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use hecs::{Entity, World};
use nalgebra_glm::vec2;

use crate::{
    astro::epoch::EphemerisTime,
    hud::{
        panel::{CommandMessages, PanelCtx},
        Binding, Section,
    },
    sim::{
        docking::{dock_tree, free_ports, Docking, PortHost},
        hierarchy::{Named, ParentBody},
        industry::Factory,
        propulsion::Craft,
        resources::{
            resource_store_amount, station_r_au, station_resource_amount_flow, Electrolyzer, Miner,
            Resource, ResourceStore, SolarPanel,
        },
    },
    ui::{button::Button, label::Label, progress_bar::ProgressBar, style::STYLE, toggle::Toggle},
};

enum DockedView {
    /// Something is docked to one of our ports, view the guest
    Guest,
    /// We're docked to one of their ports, view the host
    Host,
}

pub fn module_list(ctx: &PanelCtx, station: Entity) -> Section {
    let mut out = Section::default();
    const WIDTH: f32 = 280.0;

    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();

    let total = ctx.world.get::<&PortHost>(station).unwrap().ports;
    let used = total - free_ports(&ctx.world, station);

    let number_docked = dock_tree(&ctx.world, station).len();

    if number_docked > 1 {
        out.push(
            Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Transfer")
                .use_style(&STYLE)
                .bound_active(ctx.controls_enabled.clone())
                .on_click(CommandMessages::OpenTransfer { craft: station }),
        );
    }

    out.push(Label::new(format!("PORTS ({used}/{total})")).font(font_small_bold, ctx.app));

    for i in 0..total {
        out.merge(module_section(ctx, station, i).into_card());
    }

    out
}

fn module_section(ctx: &PanelCtx, host: Entity, i: u32) -> Section {
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

    let mut out = Section::default();

    if let Some(module) = ctx
        .world
        .query::<&Docking>()
        .iter()
        .find(|(_, docking)| docking.host == host && docking.host_port == i)
        .map(|(e, _)| e)
    {
        if ctx.world.get::<&SolarPanel>(module).is_ok() {
            out.merge(solar_panel_section(ctx, module));
        } else if ctx.world.get::<&ResourceStore>(module).is_ok() {
            out.merge(resource_store_section(ctx, module));
        } else if ctx.world.get::<&Factory>(module).is_ok() {
            out.merge(fabricator_section(ctx, module));
        } else if ctx.world.get::<&Electrolyzer>(module).is_ok() {
            out.merge(electrolyzer_section(ctx, module));
        } else if ctx.world.get::<&Miner>(module).is_ok() {
            out.merge(miner_section(ctx, module));
        } else if ctx.world.get::<&Craft>(module).is_ok() {
            out.merge(docked_craft_section(ctx, host, module, DockedView::Guest));
        } else {
            out.push(Label::new("Unknown module!!!").font(font_small_bold, ctx.app));
        }
    } else if let Some(docking) = ctx
        .world
        .get::<&Docking>(host)
        .ok()
        .filter(|docking| docking.own_port == i)
    {
        out.merge(docked_craft_section(
            ctx,
            docking.host,
            host,
            DockedView::Host,
        ));
    } else if let Some((fab, part_id)) =
        ctx.world
            .query::<(&Docking, &Factory)>()
            .iter()
            .find_map(|(e, (d, f))| {
                if d.host != host || f.reserved_port != Some(i) {
                    return None;
                }
                // prefer the active job, both can be Some!
                let part_id = f
                    .current_job
                    .as_ref()
                    .map(|j| j.part_id)
                    .or(f.pending_job)?;
                Some((e, part_id))
            })
    {
        out.merge(reserved_port_section(ctx, fab, part_id));
    } else {
        out.push(Label::new("Available").font(font_small_italic, ctx.app));
    }

    out
}

fn solar_panel_section(ctx: &PanelCtx, module: Entity) -> Section {
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();
    let text = Rc::new(RefCell::new(String::new()));

    let mut out = Section::default();

    out.push(Label::new("SOLAR PANEL ARRAY").font(font_small_bold, ctx.app));
    out.push(Label::bound(text.clone()).font(font, ctx.app));

    out.bindings.push(Binding::new({
        let text = text.clone();
        let last = Cell::new(f32::NAN);
        move |world: &World, _now: EphemerisTime| {
            let station = world.get::<&Docking>(module).unwrap().host;
            let r_au = station_r_au(world, station);
            let Ok(panel) = world.get::<&SolarPanel>(module) else {
                return;
            };
            let kw = panel.output_w(r_au) * Resource::Energy.presentation_scalars().1;
            if kw != last.get() {
                last.set(kw);
                *text.borrow_mut() = format!("{kw:+.2} kW")
            }
        }
    }));

    out
}

fn resource_store_section(ctx: &PanelCtx, module: Entity) -> Section {
    const WIDTH: f32 = 280.0;
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();

    let mut out = Section::default();

    // Tank info
    let tank = ctx.world.get::<&ResourceStore>(module).unwrap();

    out.push(
        Label::new(tank.resource.long_name().to_uppercase().to_string())
            .font(font_small_bold, ctx.app),
    );

    let mass = Rc::new(RefCell::new(String::new()));
    let mass_percentage = Rc::new(Cell::new(0.0));
    let time_to_zero = Rc::new(RefCell::new(String::new()));

    out.push(
        Label::bound(mass.clone())
            .font(font, ctx.app)
            .color(STYLE.text),
    );
    out.push(
        ProgressBar::new(vec2(WIDTH - 8.0 * 2.0, 12.0))
            .use_style(&STYLE)
            .bind(mass_percentage.clone()),
    );
    out.push(
        Label::bound(time_to_zero.clone())
            .font(font, ctx.app)
            .color(STYLE.text),
    );

    out.bindings.push(Binding::new({
        let mass = mass.clone();
        let last_m = Cell::new(f32::NAN);
        let last_mdot = Cell::new(f32::NAN);
        move |world: &World, now: EphemerisTime| {
            let station = world.get::<&Docking>(module).unwrap().host;
            let Ok(t) = world.get::<&ResourceStore>(module) else {
                return;
            };

            let amount = resource_store_amount(world, module, now);
            let rate = station_resource_amount_flow(world, station, t.resource, true);

            let until = |secs: f32| EphemerisTime::from_secs(secs as f64).short_duration();

            let (unit, dunit) = t.resource.presentation_units();
            let (scale, dscale) = t.resource.presentation_scalars();

            let m = amount * scale;
            let mdot = rate * dscale;
            let capacity = t.capacity * scale;

            if m != last_m.get() || mdot != last_mdot.get() {
                last_m.set(m);
                last_mdot.set(mdot);
                *mass.borrow_mut() = format!(
                    "{}: {:.0}/{:.0} {} ({:+.2} {})",
                    t.resource.short_name(),
                    m,
                    capacity,
                    unit,
                    mdot,
                    dunit
                );
                mass_percentage.set(m / capacity);
                *time_to_zero.borrow_mut() = if m == 0.0 {
                    "Empty".into()
                } else if rate < 0.0 {
                    format!("Empty in {}", until(amount / -rate))
                } else if rate > 0.0 && amount < t.capacity {
                    format!("Full in {}", until((t.capacity - amount) / rate))
                } else if rate > 0.0 {
                    "Full - venting".into()
                } else if amount >= t.capacity {
                    "Full".into()
                } else {
                    "Stable".into()
                };
            }
        }
    }));

    out
}

fn fabricator_section(ctx: &PanelCtx, module: Entity) -> Section {
    const WIDTH: f32 = 280.0;
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();

    let mut out = Section::default();

    let factory = ctx.world.get::<&Factory>(module).unwrap();
    let enabled = Rc::new(Cell::new(true));
    let progress = Rc::new(Cell::new(0.0));
    let countdown = Rc::new(RefCell::new(String::new()));
    let power_draw = Rc::new(RefCell::new(String::new()));

    out.push(Label::new("FABRICATOR").font(font_small_bold, ctx.app));
    out.push(Label::bound(power_draw.clone()).font(font, ctx.app));

    if let Some(job) = &factory.current_job {
        let part_name = &ctx.parts.get(job.part_id).unwrap().name;

        let ready_text = match job.completion_et(&factory, ctx.now) {
            Some(et) => format!("Ready: {}", et.as_calendar()),
            None => String::from("Ready:"),
        };

        out.push(Label::new(format!("Building {}", part_name)).font(font, ctx.app));
        out.push(
            Toggle::new("Enabled:")
                .bind(enabled.clone())
                .use_style(&STYLE)
                .on_toggle(CommandMessages::ToggleFabricator {
                    fabricator_entity: module,
                })
                .bound_active(ctx.controls_enabled.clone())
                .font(font, ctx.app),
        );
        out.push(
            ProgressBar::new(vec2(WIDTH - 8.0 * 2.0, 12.0))
                .use_style(&STYLE)
                .bind(progress.clone()),
        );
        out.push(Label::new(ready_text).font(font, ctx.app));
        out.push(Label::bound(countdown.clone()).font(font, ctx.app));
        out.push(
            Button::<CommandMessages>::text(vec2(WIDTH - 8.0 * 2.0, 30.0), "Cancel")
                .use_style(&STYLE)
                .bound_active(ctx.controls_enabled.clone())
                .on_click(CommandMessages::CancelActiveFabricator {
                    fabricator_entity: module,
                }),
        );

        out.bindings.push(Binding::new({
            move |world: &World, now: EphemerisTime| {
                let factory = world.get::<&Factory>(module).unwrap();
                let job = factory.current_job.as_ref().unwrap();
                progress.set(job.progress(&factory, now) as f32);
                let completion_et = job.completion_et(&factory, now);
                let s = match completion_et {
                    Some(et) if now >= et => "DONE".into(),
                    Some(et) => format!("T- {}", (et - now).short_duration()),
                    None => String::from("T-"),
                };
                if *countdown.borrow() != s {
                    *countdown.borrow_mut() = s;
                }
            }
        }))
    } else if let Some(part_id) = factory.pending_job {
        let part = ctx.parts.get(part_id).unwrap();

        let build_time_secs = part.cost.energy_joules / factory.power_watts;
        let completion = ctx.now + EphemerisTime::from_secs(build_time_secs as f64);

        out.push(Label::new(format!("Queued: {}", part.name)).font(font, ctx.app));
        out.push(Label::new(format!("Ready {}", completion.as_calendar())).font(font, ctx.app));
        out.push(Label::bound(countdown.clone()).font(font, ctx.app));
        out.push(
            Button::<CommandMessages>::text(vec2(WIDTH - 8.0 * 2.0, 30.0), "Cancel")
                .use_style(&STYLE)
                .bound_active(ctx.controls_enabled.clone())
                .on_click(CommandMessages::CancelQueuedFabricator {
                    fabricator_entity: module,
                }),
        );

        out.bindings.push(Binding::new({
            move |_world: &World, now: EphemerisTime| {
                let s = if now >= completion {
                    "DONE".into()
                } else {
                    format!("T- {}", (completion - now).short_duration())
                };
                if *countdown.borrow() != s {
                    *countdown.borrow_mut() = s;
                }
            }
        }))
    } else {
        out.push(
            Button::<CommandMessages>::text(vec2(WIDTH - 8.0 * 2.0, 30.0), "Build...")
                .use_style_accented(&STYLE)
                .bound_active(ctx.controls_enabled.clone())
                .on_click(CommandMessages::OpenFabricator {
                    fabricator_entity: module,
                }),
        );
    }

    out.bindings.push(Binding::new({
        let text = power_draw.clone();
        let enabled = enabled.clone();
        move |world: &World, _now: EphemerisTime| {
            if let Ok(fab) = world.get::<&Factory>(module) {
                enabled.set(fab.enabled);
                let kw = if (fab.enabled && fab.current_job.is_some()) || fab.pending_job.is_some()
                {
                    -fab.power_watts
                } else {
                    0.0
                } * Resource::Energy.presentation_scalars().1;
                *text.borrow_mut() = format!("Power draw: {kw:-.2} kW")
            }
        }
    }));

    out
}

fn electrolyzer_section(ctx: &PanelCtx, module: Entity) -> Section {
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();

    let mut out = Section::default();

    let text = Rc::new(RefCell::new(String::new()));
    let enabled = Rc::new(Cell::new(false));

    out.push(Label::new("ELECTROLYZER").font(font_small_bold, ctx.app));
    out.push(
        Toggle::new("Enabled:")
            .bind(enabled.clone())
            .use_style(&STYLE)
            .on_toggle(CommandMessages::ToggleElectrolyzer {
                electrolyzer_entity: module,
            })
            .bound_active(ctx.controls_enabled.clone())
            .font(font, ctx.app),
    );
    out.push(Label::bound(text.clone()).font(font, ctx.app));

    out.bindings.push(Binding::new({
        let text = text.clone();
        let enabled = enabled.clone();
        move |world: &World, _now: EphemerisTime| {
            if let Ok(el) = world.get::<&Electrolyzer>(module) {
                enabled.set(el.enabled);
                let kw = if el.enabled { -el.power_watts } else { 0.0 }
                    * Resource::Energy.presentation_scalars().1;
                *text.borrow_mut() = format!("Power draw: {kw:-.2} kW")
            }
        }
    }));

    out
}

fn miner_section(ctx: &PanelCtx, module: Entity) -> Section {
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();

    let mut out = Section::default();

    let text = Rc::new(RefCell::new(String::new()));
    let enabled = Rc::new(Cell::new(false));

    out.push(Label::new("MINER").font(font_small_bold, ctx.app));
    out.push(
        Toggle::new("Enabled:")
            .bind(enabled.clone())
            .use_style(&STYLE)
            .on_toggle(CommandMessages::ToggleMiner {
                miner_entity: module,
            })
            .bound_active(ctx.controls_enabled.clone())
            .font(font, ctx.app),
    );
    out.push(Label::bound(text.clone()).font(font, ctx.app));

    out.bindings.push(Binding::new({
        let text = text.clone();
        let enabled = enabled.clone();
        move |world: &World, _now: EphemerisTime| {
            if let Ok(miner) = world.get::<&Miner>(module) {
                enabled.set(miner.enabled);
                let kw = if miner.enabled {
                    -miner.power_watts
                } else {
                    0.0
                } * Resource::Energy.presentation_scalars().1;
                *text.borrow_mut() = format!("Power draw: {kw:-.2} kW")
            }
        }
    }));

    out
}

fn docked_craft_section(ctx: &PanelCtx, host: Entity, guest: Entity, view: DockedView) -> Section {
    const WIDTH: f32 = 280.0;
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();

    let mut out = Section::default();

    let (header, other) = match view {
        DockedView::Guest => (format!("DOCKED"), guest),
        DockedView::Host => (format!("DOCKED TO"), host),
    };

    out.push(Label::new(header).font(font_small_bold, ctx.app));

    let name = &ctx.world.get::<&Named>(other).unwrap().name;
    out.push(
        Button::fit(name, font, ctx.app, vec2(0.0, 0.0))
            .use_style_link(&STYLE)
            .on_click(CommandMessages::SelectEntity { entity: other }),
    );
    out.push(
        Button::<CommandMessages>::text(vec2(WIDTH - 16.0, 30.0), "Undock")
            .use_style(&STYLE)
            .bound_active(ctx.controls_enabled.clone())
            .on_click(CommandMessages::Undock { entity: guest }),
    );

    out
}

fn reserved_port_section(ctx: &PanelCtx, fab: Entity, part_id: u64) -> Section {
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();
    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();

    let mut out = Section::default();
    let name = ctx.parts.get(part_id).map_or("???", |d| d.name.as_str());

    out.push(Label::new("RESERVED").font(font_small_bold, ctx.app));
    out.push(Label::new(format!("Building {name}")).font(font, ctx.app));

    let countdown = Rc::new(RefCell::new(String::new()));
    out.push(Label::bound(countdown.clone()).font(font, ctx.app));

    out.bindings.push(Binding::new({
        move |world: &World, now: EphemerisTime| {
            let Ok(factory) = world.get::<&Factory>(fab) else {
                return;
            };
            let s = match factory.current_job.as_ref() {
                None => String::from("Queued"),
                Some(job) => match job.completion_et(&factory, now) {
                    Some(et) if now >= et => String::from("DONE"),
                    Some(et) => format!("T- {}", (et - now).short_duration()),
                    None => String::from("T-"),
                },
            };
            if *countdown.borrow() != s {
                *countdown.borrow_mut() = s;
            }
        }
    }));

    out
}
