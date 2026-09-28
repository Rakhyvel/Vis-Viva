use hecs::{Entity, World};

use crate::{
    astro::units::SECONDS_PER_DAY,
    components::{factory::Factory, parts::PartRegistry},
    sim::{docking::Docking, resources::Resource},
};

pub struct Station {
    pub num_crew: usize,
}

pub fn pending_deduction(
    world: &World,
    station: Entity,
    registry: &PartRegistry,
    r: Resource,
) -> f32 {
    let mut sum = 0.0;
    for (_, (docking, f)) in world.query::<(&Docking, &Factory)>().iter() {
        if docking.host != station {
            continue;
        }
        let Some(id) = f.pending_job else { continue };
        let Some(def) = registry.get(id) else {
            continue;
        };
        for (res, amt) in &def.cost.resources {
            if *res == r {
                sum += *amt
            }
        }
    }
    sum
}

impl Resource {
    pub fn long_name(&self) -> &'static str {
        match self {
            Resource::Energy => "Energy",
            Resource::Water => "Water",
            Resource::Oxygen => "Oxygen",
            Resource::Hydrogen => "Hydrogen",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Resource::Energy => "E",
            Resource::Water => "H2O",
            Resource::Oxygen => "O2",
            Resource::Hydrogen => "H2",
        }
    }

    pub fn presentation_units(&self) -> (&'static str, &'static str) {
        match self {
            Resource::Energy => ("kWh", "kW"),
            Resource::Water | Resource::Oxygen | Resource::Hydrogen => ("kg", "kg/day"),
        }
    }

    pub fn presentation_scalars(&self) -> (f32, f32) {
        match self {
            Resource::Energy => (1.0 / 3.6e6, 1.0 / 1000.0),
            Resource::Water | Resource::Oxygen | Resource::Hydrogen => {
                (1.0, SECONDS_PER_DAY as f32)
            }
        }
    }
}
