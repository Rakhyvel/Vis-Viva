use crate::{astro::units::SECONDS_PER_DAY, sim::resources::Resource};

pub struct Station {
    pub num_crew: usize,
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
