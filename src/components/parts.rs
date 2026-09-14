use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
};

use crate::{
    astro::units::JOULES_PER_KWH,
    components::{
        craft::{Payload, Stage},
        station::Resource,
    },
};

/// A file full of parts definitions
#[derive(serde::Deserialize)]
struct PartFile {
    parts: Vec<PartRaw>,
}

/// On-wire format for a part
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartRaw {
    id: String,
    name: String,
    desc: String,
    dry_mass_kg: f64,
    #[serde(default)]
    inputs: HashMap<String, u32>,
    #[serde(default)]
    resources: HashMap<Resource, f32>,
    #[serde(default)]
    byproducts: HashMap<Resource, f32>,
    energy_kwh: f32,
    fuel: Option<FuelSpec>,
    #[serde(default = "default_true")]
    fabricatable: bool,
}

fn default_true() -> bool {
    true
}

/// On-wire spec for fuel, for stages
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct FuelSpec {
    pub max_fuel_mass_kg: f64,
    pub isp: f64,
    pub thrust_kn: f64,
}

/// Collection of parsed and validated part definitions
#[derive(Clone)]
pub struct PartRegistry {
    parts: HashMap<u64, PartDef>,
}

/// The actual part def format, after being parsed
#[derive(Debug, Clone)]
pub struct PartDef {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub dry_mass_kg: f64,

    pub fabricatable: bool,

    pub byproducts: Vec<(Resource, f32)>,
    pub cost: PartCost,
    pub fuel: Option<FuelSpec>,
}

#[derive(Debug, Clone)]
pub struct PartCost {
    pub parts: Vec<(u64, u32)>,
    pub resources: Vec<(Resource, f32)>,
    pub energy_joules: f32,
}

impl PartRegistry {
    pub fn new() -> Self {
        Self {
            parts: HashMap::new(),
        }
    }

    pub fn load_from_dir(path: &str) -> Self {
        // Read in the raws
        let mut raws: Vec<PartRaw> = Vec::new();
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map(|e| e == "toml").unwrap_or(false) {
                let text = std::fs::read_to_string(&path).unwrap();
                let file: PartFile =
                    toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                raws.extend(file.parts);
            }
        }

        let known: HashSet<u64> = raws.iter().map(|r| id_hash(&r.id)).collect();

        // Resolve string ids to hashes, apply defaults
        let mut parts = HashMap::new();
        for raw in raws {
            let mut inputs: Vec<(u64, u32)> = raw
                .inputs
                .iter()
                .map(|(id, n)| {
                    let h = id_hash(id);
                    assert!(
                        known.contains(&h),
                        "part {} requires unknown part: {id}",
                        raw.id
                    );
                    (h, *n)
                })
                .collect();

            // Sort these so that their order is deterministic in the UI
            inputs.sort_by_key(|(h, _)| *h);
            let mut resources: Vec<(Resource, f32)> = raw.resources.into_iter().collect();
            resources.sort_by_key(|(r, _)| *r as u8);
            let mut byproducts: Vec<(Resource, f32)> = raw.byproducts.into_iter().collect();
            byproducts.sort_by_key(|(r, _)| *r as u8);

            assert!(
                raw.energy_kwh > 0.0,
                "part {} has bad build energy: {}",
                raw.id,
                raw.energy_kwh
            );

            let def = PartDef {
                dry_mass_kg: raw.dry_mass_kg,
                cost: PartCost {
                    parts: inputs,
                    resources,
                    energy_joules: raw.energy_kwh * JOULES_PER_KWH as f32,
                },
                fabricatable: raw.fabricatable,
                byproducts,
                fuel: raw.fuel,
                id: raw.id,
                name: raw.name,
                desc: raw.desc,
            };
            let res = parts.insert(id_hash(&def.id), def).is_none();
            assert!(
                res,
                "duplicate part id but I'm not gonna tell you which one you have to guess"
            );
        }
        Self { parts }
    }

    pub fn get(&self, id: u64) -> Option<&PartDef> {
        self.parts.get(&id)
    }

    pub fn all(&self) -> impl Iterator<Item = &PartDef> {
        self.parts.values()
    }
}

pub fn id_hash(id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

impl PartDef {
    pub fn instantiate_stage(&self) -> Stage {
        Stage {
            name: self.name.clone(),
            dry_mass: self.dry_mass_kg,
            fuel_mass: self.fuel.unwrap().max_fuel_mass_kg, // starts full
            max_fuel_mass: self.fuel.unwrap().max_fuel_mass_kg,
            thrust_kn: self.fuel.unwrap().thrust_kn,
            isp: self.fuel.unwrap().isp,
        }
    }

    pub fn instantiate_payload(&self) -> Payload {
        Payload {
            name: self.name.clone(),
            dry_mass: self.dry_mass_kg,
        }
    }

    pub fn id_hash(&self) -> u64 {
        id_hash(&self.id)
    }
}
