///! Rocket equation (it's a beautiful thing)
use hecs::{Entity, World};

use crate::{
    astro::{epoch::EphemerisTime, units::LITTLE_G},
    sim::{
        mission::Command,
        resources::{station_resource_totals, stored_mass_kg, take_resource, Resource},
    },
};

const OF_RATIO: f32 = 5.5;

pub struct Craft {
    pub dry_mass: f64,
    pub isp: Option<f64>,

    pub command: Option<Command>,
    pub command_scheduled: bool,
    pub line_path_entity: Option<Entity>,
}

pub fn usable_propellant_kg(world: &World, craft: Entity, t: EphemerisTime) -> f64 {
    let (h2, _) = station_resource_totals(world, craft, Resource::Hydrogen, t);
    let (o2, _) = station_resource_totals(world, craft, Resource::Oxygen, t);
    (h2 * (1.0 + OF_RATIO)).min(o2 * (1.0 + OF_RATIO) / OF_RATIO) as f64
}

pub fn apply_burn(world: &World, craft: Entity, requested_dv: f64, t: EphemerisTime) {
    let Some((isp, dry_mass)) = world
        .get::<&Craft>(craft)
        .ok()
        .and_then(|c| Some((c.isp?, c.dry_mass)))
    else {
        return; // no engine
    };

    let m0 = dry_mass + stored_mass_kg(world, craft, t);
    let mf_needed = m0 / (requested_dv / (isp * LITTLE_G)).exp();

    let available = usable_propellant_kg(world, craft, t);
    let used = (m0 - mf_needed).clamp(0.0, available) as f32;

    take_resource(world, craft, Resource::Hydrogen, used / (1.0 + OF_RATIO), t);
    take_resource(
        world,
        craft,
        Resource::Oxygen,
        used * OF_RATIO / (1.0 + OF_RATIO),
        t,
    );
}

pub fn craft_dv(world: &World, craft: Entity, t: EphemerisTime) -> f64 {
    let Some((isp, dry_mass)) = world
        .get::<&Craft>(craft)
        .ok()
        .and_then(|c| Some((c.isp?, c.dry_mass)))
    else {
        return 0.0; // no engine!
    };

    let m0 = dry_mass + stored_mass_kg(world, craft, t);
    let propellant = usable_propellant_kg(world, craft, t);
    isp * LITTLE_G * (m0 / (m0 - propellant).max(1e-9)).ln()
}
