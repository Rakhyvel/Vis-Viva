use hecs::{Entity, World};

use crate::{
    astro::epoch::EphemerisTime,
    sim::resources::{station_resource_amount_flow, station_resource_totals, Resource},
};

pub struct Station {
    pub num_crew: usize,
}

pub const O2_PER_CREW_DAY: f32 = -0.84;
pub const WATER_PER_CREW_DAY: f32 = -3.5;

pub fn crew_death(world: &World, now: EphemerisTime) -> Option<(Entity, Resource)> {
    const CRITICAL: [Resource; 3] = [Resource::Oxygen, Resource::Water, Resource::Energy];
    for (station, s) in world.query::<&Station>().iter() {
        if s.num_crew == 0 {
            continue;
        }
        for r in CRITICAL {
            let (stored, _) = station_resource_totals(world, station, r, now);
            let flow = station_resource_amount_flow(world, station, r, false);
            // Under a second of supply counts as empty, so float rounding at the stop can't delay it
            if flow < 0.0 && stored / -flow < 1.0 {
                return Some((station, r));
            }
        }
    }
    None
}
