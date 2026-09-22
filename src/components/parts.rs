use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
};

use crate::{
    astro::units::JOULES_PER_KWH,
    components::{
        craft::{Craft, Engine},
        station::Resource,
    },
};

/// A file full of parts definitions
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    ports_required: u32,
    fuel: Option<FuelSpec>,
    #[serde(default = "default_true")]
    fabricatable: bool,
    #[serde(default)]
    ports: u32,
    #[serde(default)]
    modules: Vec<ModuleSpec>,
}

fn default_true() -> bool {
    true
}

/// On-wire spec for fuel, for stages
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuelSpec {
    pub isp: f64,
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

    pub ports: u32,
    pub modules: Vec<ModuleSpec>,
}

#[derive(Debug, Clone)]
pub struct PartCost {
    pub parts: Vec<(u64, u32)>,
    pub resources: Vec<(Resource, f32)>,
    pub energy_joules: f32,
    pub ports_required: u32,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ModuleSpec {
    Store {
        resource: Resource,
        #[serde(default)]
        amount: f32,
        capacity: f32,
    },
    Miner {
        power_watts: f32,
        kg_per_s: f32,
    },
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

            assert!(raw.modules.len() <= raw.ports as usize);

            let def = PartDef {
                dry_mass_kg: raw.dry_mass_kg,
                cost: PartCost {
                    parts: inputs,
                    resources,
                    energy_joules: raw.energy_kwh * JOULES_PER_KWH as f32,
                    ports_required: raw.ports_required,
                },
                fabricatable: raw.fabricatable,
                byproducts,
                fuel: raw.fuel,
                id: raw.id,
                name: raw.name,
                desc: raw.desc,
                ports: raw.ports,
                modules: raw.modules,
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
    pub fn instantiate_craft(&self) -> Craft {
        Craft {
            part_id: self.id_hash(),
            dry_mass: self.dry_mass_kg,
            engine: self.instantiate_engine(),

            command: None,
            command_scheduled: false,
            line_path_entity: None,
        }
    }

    fn instantiate_engine(&self) -> Option<Engine> {
        let fuel = &self.fuel?;
        Some(Engine {
            fuel_mass: 0.0,
            isp: fuel.isp,
        })
    }

    pub fn id_hash(&self) -> u64 {
        id_hash(&self.id)
    }
}
