use apricot::high_precision::WorldPosition;
use hecs::{Entity, World};

use crate::{
    astro::{
        epoch::EphemerisTime,
        units::{EARTH_RADII_PER_AU, SECONDS_PER_DAY},
    },
    components::{
        body::{Body, Parent},
        craft::Landed,
        factory::Factory,
        parts::PartRegistry,
    },
};

pub struct Station {
    pub num_crew: usize,
}

pub struct ModuleHost {
    /// bumped whenever a module is added or removed
    /// TODO: May want to rework when we can jettison modules, and dock and undock from the same ports
    pub modules_gen: u32,
}

pub fn station_r_au(world: &World, station: Entity) -> f64 {
    let Ok(pos) = world.get::<&WorldPosition>(station) else {
        return 0.0;
    };
    pos.pos.magnitude() / EARTH_RADII_PER_AU // sun at origin
}

/// Just the committed totals, no extrapolation, safe to call from flow fns
fn committed_totals(world: &World, station: Entity, r: Resource) -> (f32, f32) {
    let mut amount = 0.0;
    let mut capacity = 0.0;
    for (_, (_, p, s)) in world
        .query::<(&StationModule, &Parent, &ResourceStore)>()
        .iter()
    {
        if p.id == station && s.resource == r {
            amount += s.amount;
            capacity += s.capacity
        }
    }
    (amount, capacity)
}

/// Gets the total station-wide amount time derivative for a resource, in unit/sec
pub fn station_resource_amount_flow(
    world: &World,
    host: Entity,
    r: Resource,
    projected: bool,
) -> f32 {
    // Accumulate producers of a resource
    const H2_PER_H2O: f32 = 0.1119;
    const O2_PER_H2O: f32 = 0.8881;
    const O2_PER_CREW_DAY: f32 = -0.84;
    const WATER_PER_CREW_DAY: f32 = -3.5;

    let num_crew = world.get::<&Station>(host).map(|s| s.num_crew).unwrap_or(0);
    let crew_water = num_crew as f32 * WATER_PER_CREW_DAY / SECONDS_PER_DAY as f32;
    let crew_o2 = num_crew as f32 * O2_PER_CREW_DAY / SECONDS_PER_DAY as f32;
    let el = electrolyzer_kg_per_s(world, host);

    match r {
        Resource::Water => crew_water + -el + miner_kg_per_s(world, host),
        Resource::Oxygen => crew_o2 + el * O2_PER_H2O,
        Resource::Hydrogen => el * H2_PER_H2O,
        Resource::Energy => {
            let mut watts = station_net_watts(world, host);
            if projected {
                for (_, (_, parent, fab)) in
                    world.query::<(&StationModule, &Parent, &Factory)>().iter()
                {
                    if parent.id == host && fab.current_job.is_none() && fab.pending_job.is_some() {
                        watts -= fab.power_watts;
                    }
                }
            }
            watts
        }
    }

    // TODO: Decumulate consumers of a resource
}

/// Get the net power in Watts
pub fn station_net_watts(world: &World, station: Entity) -> f32 {
    let r_au = station_r_au(world, station);

    let mut w = 0.0;

    // Sum up all the generators
    for (_, (_, parent, panel)) in world
        .query::<(&StationModule, &Parent, &SolarPanel)>()
        .iter()
    {
        if parent.id == station {
            w += panel.output_w(r_au);
        }
    }

    // Subtract consumers
    for (_, (_, parent, fab)) in world.query::<(&StationModule, &Parent, &Factory)>().iter() {
        if parent.id == station && fab.current_job.is_some() && fab.enabled {
            w -= fab.power_watts;
        }
    }
    for (_, (_, parent, el)) in world
        .query::<(&StationModule, &Parent, &Electrolyzer)>()
        .iter()
    {
        let running = el.is_running(world, parent.id);
        if parent.id == station && el.enabled && running {
            w -= el.power_watts;
        }
    }

    w
}

pub fn electrolyzer_kg_per_s(world: &World, station: Entity) -> f32 {
    let (water, _) = committed_totals(world, station, Resource::Water);

    let mut kg_s = 0.0;
    for (_, (_, p, el)) in world
        .query::<(&StationModule, &Parent, &Electrolyzer)>()
        .iter()
    {
        if p.id == station && el.enabled && water > 0.0 {
            kg_s += el.power_watts / el.joules_per_kg_water
        }
    }

    kg_s
}

/// Interpolates the amount for a specific resource store
pub fn resource_store_amount(world: &World, module: Entity, t: EphemerisTime) -> f32 {
    let station = world.get::<&Parent>(module).unwrap().id;
    let Ok(store) = world.get::<&ResourceStore>(module) else {
        return 0.0;
    };

    let flow = station_resource_amount_flow(world, station, store.resource, false);

    let mut capacity = 0.0;
    let mut q = world.query::<(&StationModule, &Parent, &ResourceStore)>();
    for (_, (_, parent, other_store)) in q.iter() {
        if parent.id == station && other_store.resource == store.resource {
            capacity += other_store.capacity;
        }
    }

    let share = flow * store.capacity / capacity;
    let dt_secs = (t - store.amount_et).as_secs() as f32;
    (store.amount + share * dt_secs).clamp(0.0, store.capacity)
}

/// Returns (stored, capacity) of a given resource at a given time
pub fn station_resource_totals(
    world: &World,
    station: Entity,
    r: Resource,
    t: EphemerisTime,
) -> (f32, f32) {
    let flow = station_resource_amount_flow(world, station, r, false);

    let mut capacity = 0.0;
    let mut stores: Vec<&ResourceStore> = Vec::new();
    let mut q = world.query::<(&StationModule, &Parent, &ResourceStore)>();
    for (_, (_, parent, store)) in q.iter() {
        if parent.id == station && store.resource == r {
            capacity += store.capacity;
            stores.push(store);
        }
    }

    let mut stored = 0.0;
    for store in stores {
        let share = flow * store.capacity / capacity;
        let dt_secs = (t - store.amount_et).as_secs() as f32;
        stored += (store.amount + share * dt_secs).clamp(0.0, store.capacity);
    }

    (stored, capacity)
}

pub fn add_resource(world: &World, station: Entity, r: Resource, amount: f32, now: EphemerisTime) {
    commit_resource_stores(world, station, r, now);

    let resource_stores = stores_of(world, station, r);

    // how much each resource store can take up
    let headroom: Vec<f32> = resource_stores
        .iter()
        .map(|m| {
            let s = world.get::<&ResourceStore>(*m).unwrap();
            (s.capacity - s.amount).max(0.0)
        })
        .collect();

    let total: f32 = headroom.iter().sum();
    if total <= 0.0 {
        return; // everything is full, vent (sus)
    }

    // fill up to what we each store can accept
    let accepted = amount.min(total);
    for (m, h) in resource_stores.iter().zip(headroom) {
        let mut store = world.get::<&mut ResourceStore>(*m).unwrap();
        store.amount = (store.amount + accepted * h / total).min(store.capacity);
    }
}

pub fn take_resource(world: &World, station: Entity, r: Resource, amount: f32, now: EphemerisTime) {
    commit_resource_stores(world, station, r, now);

    let modules = stores_of(world, station, r);

    // Freeze each store's amount at `now`
    let amounts: Vec<f32> = modules
        .iter()
        .map(|m| resource_store_amount(world, *m, now))
        .collect();
    let total: f32 = amounts.iter().sum();
    if total <= 0.0 {
        return;
    }

    // draw down proportionally to what each holds
    for (m, a) in modules.iter().zip(amounts) {
        let mut store = world.get::<&mut ResourceStore>(*m).unwrap();
        store.amount = a - amount * a / total;
    }
}

pub fn commit_station(world: &World, station: Entity, now: EphemerisTime) {
    for r in Resource::ALL {
        commit_resource_stores(world, station, *r, now);
    }
}

pub fn commit_resource_stores(world: &World, station: Entity, r: Resource, now: EphemerisTime) {
    for module in stores_of(world, station, r) {
        let current = resource_store_amount(world, module, now);
        let mut s = world.get::<&mut ResourceStore>(module).unwrap();
        s.amount = current;
        s.amount_et = now;
    }
}

fn stores_of(world: &World, station: Entity, r: Resource) -> Vec<Entity> {
    world
        .query::<(&StationModule, &Parent, &ResourceStore)>()
        .iter()
        .filter(|(_, (_, p, store))| p.id == station && store.resource == r)
        .map(|(e, _)| e)
        .collect()
}

pub fn next_reservoir_limits(
    world: &World,
    station: Entity,
    registry: &PartRegistry,
    now: EphemerisTime,
    projected: bool,
) -> Vec<(EphemerisTime, Resource, f32)> {
    let mut ts: Vec<(EphemerisTime, Resource, f32)> = Resource::ALL
        .iter()
        .filter_map(|r| {
            let (mut total, capacity) = station_resource_totals(world, station, *r, now);
            if projected {
                total = (total - pending_deduction(world, station, registry, *r)).max(0.0)
            }
            let rate = station_resource_amount_flow(world, station, *r, projected);

            // Fix saturation, on either end, so we don't do more than one event for these
            let rate = if (total >= capacity && rate > 0.0) || (total <= 0.0 && rate < 0.0) {
                0.0
            } else {
                rate
            };

            let secs = if rate < 0.0 {
                total / -rate
            } else if rate > 0.0 {
                (capacity - total) / rate
            } else {
                return None;
            };

            const MIN_EVENT_SECS: f32 = 60.0;
            if secs <= MIN_EVENT_SECS || !secs.is_finite() {
                None
            } else {
                Some((now + EphemerisTime::from_secs(secs.into()), *r, rate))
            }
        })
        .collect();

    ts.sort_by_key(|(t, _, _)| *t);

    ts
}

fn pending_deduction(world: &World, station: Entity, registry: &PartRegistry, r: Resource) -> f32 {
    let mut sum = 0.0;
    for (_, (_, p, f)) in world.query::<(&StationModule, &Parent, &Factory)>().iter() {
        if p.id != station {
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

/// Joins a module to a station
pub struct StationModule {
    pub slot: u32,
}

pub struct SolarPanel {
    /// How much power the panels produce, in W, at 1 AU
    pub rated_w: f32,
}

impl SolarPanel {
    pub fn output_w(&self, r_au: f64) -> f32 {
        self.rated_w / (r_au * r_au) as f32
    }
}

pub struct Electrolyzer {
    /// Is this guy even turned on (I know I am...)
    pub enabled: bool,
    /// How power much this electrolzyer draws when on
    pub power_watts: f32,
    /// Energy to turn 1 kg of water into LH2 + LO2
    pub joules_per_kg_water: f32,
}

impl Electrolyzer {
    pub fn is_running(&self, world: &World, station: Entity) -> bool {
        let (water, _) = committed_totals(world, station, Resource::Water);
        self.enabled && water > 0.0
    }
}

pub struct Miner {
    pub enabled: bool,
    /// How power much this miner draws when on
    pub power_watts: f32,
    pub kg_per_s: f32,
}

pub fn miner_kg_per_s(world: &World, host: Entity) -> f32 {
    if world.get::<&Landed>(host).is_err() {
        return 0.0;
    }

    let Ok(parent) = world.get::<&Parent>(host) else {
        return 0.0;
    };
    let Ok(body) = world.get::<&Body>(parent.id) else {
        return 0.0;
    };

    let mut kg_s = 0.0;
    for (_, (_, p, m)) in world.query::<(&StationModule, &Parent, &Miner)>().iter() {
        if p.id == host && m.enabled {
            kg_s += m.kg_per_s * 0.4; // TODO: Take from body ice fraction
        }
    }
    kg_s
}

// TODO: This doens't belong here!
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resource {
    Energy,
    Water,
    Oxygen,
    Hydrogen,
}

impl Resource {
    pub const ALL: &[Resource] = &[
        Resource::Energy,
        Resource::Water,
        Resource::Oxygen,
        Resource::Hydrogen,
    ];

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

pub struct ResourceStore {
    /// What's in the store
    pub resource: Resource,
    pub amount: f32,
    pub capacity: f32,
    /// When `amount` was committed
    pub amount_et: EphemerisTime,
}
