use hecs::Entity;
use nalgebra_glm::vec2;

use crate::{
    astro::{
        state::State,
        units::{EARTH_RADII_PER_AU, KM_PER_EARTH_RADIUS},
    },
    hud::{
        panel::{CommandMessages, PanelCtx},
        stat_row, Section,
    },
    sim::{
        bodies::Body,
        hierarchy::{Named, ParentBody},
        parts::PartInventory,
        propulsion::Craft,
    },
    ui::{button::Button, hrule::HRule, label::Label, style::STYLE, widget::Widget},
};

pub fn body_selection(ctx: &PanelCtx, selected: Entity) -> Section {
    let mut out = Section::default();
    const WIDTH: f32 = 280.0;

    let font = ctx.app.renderer.get_font_id_from_name("font").unwrap();
    let font_small_bold = ctx
        .app
        .renderer
        .get_font_id_from_name("font-small-bold")
        .unwrap();

    let body = ctx.world.get::<&Body>(selected).unwrap();
    let inventory = ctx.world.get::<&PartInventory>(selected).unwrap();

    let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> = vec![];

    if let Ok(parent) = ctx.world.get::<&ParentBody>(selected) {
        let state = ctx.world.get::<&State>(selected).unwrap();
        let parent_body = ctx.world.get::<&Body>(parent.id).unwrap();

        let orbits_star = ctx.world.get::<&ParentBody>(parent.id).is_err();
        let dist = |er: f64| {
            if orbits_star {
                format!("{:.2} AU", er / EARTH_RADII_PER_AU)
            } else {
                format!("{:.0} km", er * KM_PER_EARTH_RADIUS)
            }
        };

        let (apo, peri) = state.apsides(parent_body.mu);
        let ecc = state.ecc(parent_body.mu);
        let inc = state.inclination();

        let orbit_rows = [
            ("APOAPSIS", apo.map_or("-".into(), dist)),
            ("PERIAPSIS", dist(peri)),
            ("ECCENTRICITY", format!("{:.3}", ecc)),
            ("INCLINATION", format!("{:.1} deg", inc.to_degrees())),
        ];

        widgets.extend(orbit_rows.iter().map(|(k, v)| {
            Box::new(stat_row(k, v.to_string(), STYLE.text, WIDTH, ctx.app))
                as Box<dyn Widget<CommandMessages>>
        }));
        widgets.push(Box::new(HRule::new(STYLE.border, 1.0, WIDTH)));
    }

    let body_rows = [
        // TODO: Support earth symbol and exponents in apricot's font cache
        ("RADIUS", format!("{:.1} ER", body.body_radius)),
        ("MASS", format!("{:.3} EM", body.mass())),
        ("DENSITY", format!("{:.1} g/cm^3", body.density)),
        ("DAY", format!("{:.1} hrs", body.rotation_period_hours)),
        // TODO: Replace the following with sensor estimates
        ("PRESSURE", String::from("-")),
        ("TEMPERATURE", String::from("-")),
        ("CORE MASS", String::from("-")),
        ("MAGNETIC", String::from("-")),
    ];

    // Know: name, radius, mass, density, orbital radius, rotation in hours
    // Have to find: atmos press, temp, core mass fraction, magnetic field
    widgets.extend(body_rows.iter().map(|(k, v)| {
        Box::new(stat_row(k, v.to_string(), STYLE.text, WIDTH, ctx.app))
            as Box<dyn Widget<CommandMessages>>
    }));

    // Extend with inventory info
    widgets.extend(inventory.parts.iter().filter_map(|(part_id, quantity)| {
        if *quantity > 0 {
            Some(
                Box::new(Label::new(format!("{}: {}", part_id, quantity)).font(font, ctx.app))
                    as Box<dyn Widget<CommandMessages>>,
            )
        } else {
            None
        }
    }));
    out.widgets = widgets;

    let children: Vec<Entity> = ctx
        .world
        .query::<(&ParentBody, &Body)>()
        .iter()
        .filter(|(_, (p, _))| p.id == selected)
        .map(|(e, _)| e)
        .collect(); // TODO: We don't have to collect just to check for is_emtpy, do we?

    if !children.is_empty() {
        let has_parent = ctx.world.get::<&ParentBody>(selected).is_ok();
        out.push(HRule::new(STYLE.border, 1.0, WIDTH));
        out.push(
            Label::new(if has_parent { "MOONS" } else { "PLANETS" }).font(font_small_bold, ctx.app),
        );
        out.widgets.extend(children.iter().filter_map(|e| {
            let child_name = ctx.world.get::<&Named>(*e).ok()?;
            Some(Box::new(
                Button::fit(&child_name.name, font, ctx.app, vec2(0.0, 0.0))
                    .use_style_link(&STYLE)
                    .on_click(CommandMessages::SelectEntity { entity: *e }),
            ) as Box<dyn Widget<CommandMessages>>)
        }));
    }

    let craft: Vec<Entity> = ctx
        .world
        .query::<(&ParentBody, &Craft)>()
        .iter()
        .filter(|(_, (p, _))| p.id == selected)
        .map(|(e, _)| e)
        .collect(); // TODO: We don't have to collect just to check for is_emtpy, do we?

    if !craft.is_empty() {
        out.push(HRule::new(STYLE.border, 1.0, WIDTH));
        out.push(Label::new("CRAFT").font(font_small_bold, ctx.app));
        out.widgets.extend(craft.iter().filter_map(|e| {
            let child_name = ctx.world.get::<&Named>(*e).ok()?;
            Some(Box::new(
                Button::fit(&child_name.name, font, ctx.app, vec2(0.0, 0.0))
                    .use_style_link(&STYLE)
                    .on_click(CommandMessages::SelectEntity { entity: *e }),
            ) as Box<dyn Widget<CommandMessages>>)
        }));
    }

    out
}
