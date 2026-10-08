use apricot::high_precision::WorldPosition;
use hecs::{Entity, World};

///! Stored quantities
use crate::{
    astro::{
        epoch::EphemerisTime,
        units::{EARTH_RADII_PER_AU, SECONDS_PER_DAY},
    },
    sim::{
        bodies::Body,
        docking::{dock_tree, Docking},
        hierarchy::{Landed, ParentBody},
        industry::Factory,
        life_support::{Station, O2_PER_CREW_DAY, WATER_PER_CREW_DAY},
        transfer::transfer_flow,
    },
};

/// An uncountable, often fluid resource
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
}

pub struct ResourceStore {
    /// What's in the store
    pub resource: Resource,
    pub amount: f32,
    pub capacity: f32,
    /// When `amount` was committed
    pub amount_et: EphemerisTime,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pool {
    pub host: Entity,
    pub resource: Resource,
}

fn stores_of(world: &World, station: Entity, r: Resource) -> Vec<Entity> {
    world
        .query::<(&Docking, &ResourceStore)>()
        .iter()
        .filter(|(_, (docking, store))| docking.host == station && store.resource == r)
        .map(|(e, _)| e)
        .collect()
}

/// Just the committed totals, no extrapolation, safe to call from flow fns
pub fn committed_totals(world: &World, station: Entity, r: Resource) -> (f32, f32) {
    let mut amount = 0.0;
    let mut capacity = 0.0;
    for (_, (d, s)) in world.query::<(&Docking, &ResourceStore)>().iter() {
        if d.host == station && s.resource == r {
            amount += s.amount;
            capacity += s.capacity
        }
    }
    (amount, capacity)
}

/// Returns (stored, capacity) of a given resource at a given time
pub fn station_resource_totals(
    world: &World,
    station: Entity,
    r: Resource,
    t: EphemerisTime,
) -> (f32, f32) {
    let flow = station_resource_amount_flow(world, station, r);

    let mut capacity = 0.0;
    let mut stores: Vec<&ResourceStore> = Vec::new();
    let mut q = world.query::<(&Docking, &ResourceStore)>();
    for (_, (docking, store)) in q.iter() {
        if docking.host == station && store.resource == r {
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

/// Interpolates the amount for a specific resource store
pub fn resource_store_amount(world: &World, module: Entity, t: EphemerisTime) -> f32 {
    let station = world
        .get::<&Docking>(module)
        .expect("resource stores are always docked to a host")
        .host;
    let store = world
        .get::<&ResourceStore>(module)
        .expect("resource_store_amount is only called on stores");

    let flow = station_resource_amount_flow(world, station, store.resource);

    let mut capacity = 0.0;
    let mut q = world.query::<(&Docking, &ResourceStore)>();
    for (_, (docking, other_store)) in q.iter() {
        if docking.host == station && other_store.resource == store.resource {
            capacity += other_store.capacity;
        }
    }

    let share = (flow * store.capacity / capacity) as f64;
    let dt_secs = (t - store.amount_et).as_secs();
    (store.amount as f64 + share * dt_secs).clamp(0.0, store.capacity as f64) as f32
}

pub fn commit_station(world: &World, station: Entity, now: EphemerisTime) {
    let mut hosts = dock_tree(world, station);
    if !hosts.contains(&station) {
        hosts.push(station);
    }

    let stores: Vec<(Entity, f32)> = world
        .query::<(&Docking, &ResourceStore)>()
        .iter()
        .filter(|(_, (docking, _))| hosts.contains(&docking.host))
        .map(|(module, (_, _))| (module, resource_store_amount(world, module, now)))
        .collect();

    let jobs: Vec<(Entity, f32)> = world
        .query::<(&Docking, &Factory)>()
        .iter()
        .filter(|(_, (docking, _))| hosts.contains(&docking.host))
        .filter_map(|(fab, (docking, f))| {
            let factor = power_factor(world, docking.host);
            Some((fab, f.current_job.as_ref()?.energy_at(f, factor, now)))
        })
        .collect();

    for (module, amount) in stores {
        let mut store = world
            .get::<&mut ResourceStore>(module)
            .expect("collected from a ResourceStore query");
        store.amount = amount;
        store.amount_et = now;
    }

    for (fab, energy) in jobs {
        let mut f = world
            .get::<&mut Factory>(fab)
            .expect("collected from a Factory query");
        let job = f
            .current_job
            .as_mut()
            .expect("collected because it had a job");
        job.energy_done = energy;
        job.energy_et = now;
    }
}

pub fn add_resource(world: &World, station: Entity, r: Resource, amount: f32, now: EphemerisTime) {
    commit_station(world, station, now);

    let resource_stores = stores_of(world, station, r);

    // how much each resource store can take up
    let headroom: Vec<f32> = resource_stores
        .iter()
        .map(|m| {
            let s = world
                .get::<&ResourceStore>(*m)
                .expect("stores_of only returns ResourceStores entities");
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
        let mut store = world
            .get::<&mut ResourceStore>(*m)
            .expect("stores_of only returns ResourceStores entities");
        store.amount = (store.amount + accepted * h / total).min(store.capacity);
    }
}

pub fn take_resource(world: &World, station: Entity, r: Resource, amount: f32, now: EphemerisTime) {
    commit_station(world, station, now);

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
        let mut store = world
            .get::<&mut ResourceStore>(*m)
            .expect("stores_of only returns ResourceStores entities");
        store.amount = a - amount * a / total;
    }
}

/// Mass of all stored resources.
pub fn stored_mass_kg(world: &World, host: Entity, t: EphemerisTime) -> f64 {
    let mut kg = 0.0;
    for (module, (docking, store)) in world.query::<(&Docking, &ResourceStore)>().iter() {
        if docking.host == host && store.resource != Resource::Energy {
            kg += resource_store_amount(world, module, t) as f64
        }
    }
    kg
}

pub fn next_reservoir_limits(
    world: &World,
    station: Entity,
    now: EphemerisTime,
) -> Vec<(EphemerisTime, Resource, f32)> {
    let mut ts: Vec<(EphemerisTime, Resource, f32)> = Resource::ALL
        .iter()
        .filter_map(|r| {
            let (total, capacity) = station_resource_totals(world, station, *r, now);
            let rate = station_resource_amount_flow(world, station, *r);

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
        // TODO: One "is running" component
        let (water, _) = committed_totals(world, station, Resource::Water);
        self.enabled && water > 0.0
    }
}

pub fn electrolyzer_kg_per_s(world: &World, station: Entity) -> f32 {
    let f = power_factor(world, station);
    let (water, _) = committed_totals(world, station, Resource::Water);

    let mut kg_s = 0.0;
    for (_, (docking, el)) in world.query::<(&Docking, &Electrolyzer)>().iter() {
        if docking.host == station && el.enabled && water > 0.0 {
            kg_s += el.power_watts / el.joules_per_kg_water * f
        }
    }

    kg_s
}

pub struct Miner {
    pub enabled: bool,
    /// How power much this miner draws when on
    pub power_watts: f32,
    pub kg_per_s: f32,
}

impl Miner {
    pub fn is_running(&self, world: &World, host: Entity) -> bool {
        // TODO: One "is running" component
        if world.get::<&Landed>(host).is_err() {
            return false;
        }

        self.enabled
    }
}

pub fn miner_kg_per_s(world: &World, host: Entity) -> f32 {
    if world.get::<&Landed>(host).is_err() {
        return 0.0;
    }
    let f = power_factor(world, host);

    let Ok(parent) = world.get::<&ParentBody>(host) else {
        return 0.0;
    };
    let Ok(_body) = world.get::<&Body>(parent.id) else {
        return 0.0;
    };

    let mut kg_s = 0.0;
    for (_, (docking, m)) in world.query::<(&Docking, &Miner)>().iter() {
        if docking.host == host && m.enabled {
            kg_s += m.kg_per_s * 0.4 * f; // TODO: Take from body ice fraction
        }
    }
    kg_s
}

/// Watts the station's panels produce
pub fn station_supply_watts(world: &World, station: Entity) -> f32 {
    let r_au = station_r_au(world, station);

    let mut w = 0.0;

    // Sum up all the generators
    for (_, (docking, panel)) in world.query::<(&Docking, &SolarPanel)>().iter() {
        if docking.host == station {
            w += panel.output_w(r_au);
        }
    }

    // Sum up all the energy transfers to this station
    w += transfer_flow(world, station, Resource::Energy);

    w
}

pub fn station_demand_watts(world: &World, station: Entity) -> f32 {
    let mut w = 0.0;

    // Subtract consumers
    for (_, (docking, fab)) in world.query::<(&Docking, &Factory)>().iter() {
        if docking.host == station && fab.current_job.is_some() && fab.enabled {
            w += fab.power_watts;
        }
    }
    for (_, (docking, el)) in world.query::<(&Docking, &Electrolyzer)>().iter() {
        let running = el.is_running(world, docking.host);
        if docking.host == station && el.enabled && running {
            w += el.power_watts;
        }
    }
    for (_, (docking, miner)) in world.query::<(&Docking, &Miner)>().iter() {
        let running = miner.is_running(world, docking.host);
        if docking.host == station && running {
            w += miner.power_watts;
        }
    }

    w
}

/// What fraction of the power consumers actually get
pub fn power_factor(world: &World, station: Entity) -> f32 {
    let (stored, _) = committed_totals(world, station, Resource::Energy);
    let (supply, demand) = (
        station_supply_watts(world, station),
        station_demand_watts(world, station),
    );

    if stored > 0.0 || demand <= supply {
        1.0
    } else {
        supply / demand
    }
}

/// Get the net power in Watts
pub fn station_net_watts(world: &World, station: Entity) -> f32 {
    let (supply, demand) = (
        station_supply_watts(world, station),
        station_demand_watts(world, station),
    );

    supply - demand * power_factor(world, station)
}

pub fn station_r_au(world: &World, station: Entity) -> f64 {
    let Ok(pos) = world.get::<&WorldPosition>(station) else {
        return 0.0;
    };
    pos.pos.magnitude() / EARTH_RADII_PER_AU // sun at origin
}

/// Gets the total station-wide amount time derivative for a resource, in unit/sec
pub fn station_resource_amount_flow(world: &World, host: Entity, r: Resource) -> f32 {
    // Accumulate producers of a resource
    const H2_PER_H2O: f32 = 0.1119;
    const O2_PER_H2O: f32 = 0.8881;

    let num_crew = world.get::<&Station>(host).map(|s| s.num_crew).unwrap_or(0);
    let crew_water = num_crew as f32 * WATER_PER_CREW_DAY / SECONDS_PER_DAY as f32;
    let crew_o2 = num_crew as f32 * O2_PER_CREW_DAY / SECONDS_PER_DAY as f32;
    let el = electrolyzer_kg_per_s(world, host);

    match r {
        Resource::Water => {
            crew_water + -el + miner_kg_per_s(world, host) + transfer_flow(world, host, r)
        }
        Resource::Oxygen => crew_o2 + el * O2_PER_H2O + transfer_flow(world, host, r),
        Resource::Hydrogen => el * H2_PER_H2O + transfer_flow(world, host, r),
        Resource::Energy => station_net_watts(world, host),
    }
}

/// Builders for small worlds in the sim tests
#[cfg(test)]
pub(crate) mod test_world {
    use nalgebra_glm::vec3;

    use super::*;
    use crate::sim::industry::FactoryJob;

    /// A host 1 AU from the sun, so its panels make exactly their rated power
    pub fn host() -> (World, Entity) {
        let mut world = World::new();
        let host = world.spawn((WorldPosition {
            pos: vec3(EARTH_RADII_PER_AU, 0.0, 0.0),
        },));
        (world, host)
    }

    fn docked(host: Entity) -> Docking {
        Docking {
            host,
            host_port: 0,
            own_port: 0,
        }
    }

    /// One store per resource keeps the capacity shares trivial
    pub fn store(world: &mut World, host: Entity, resource: Resource, amount: f32) -> Entity {
        world.spawn((
            docked(host),
            ResourceStore {
                resource,
                amount,
                capacity: 1e9,
                amount_et: EphemerisTime::epoch(),
            },
        ))
    }

    pub fn panel(world: &mut World, host: Entity, watts: f32) -> Entity {
        world.spawn((docked(host), SolarPanel { rated_w: watts }))
    }

    /// Splits 1 kg of water per kJ, so `watts` W gets through watts / 1000 kg/s
    pub fn electrolyzer(world: &mut World, host: Entity, watts: f32) -> Entity {
        world.spawn((
            docked(host),
            Electrolyzer {
                enabled: true,
                power_watts: watts,
                joules_per_kg_water: 1000.0,
            },
        ))
    }

    /// Running a 1 MJ job, started at the epoch
    pub fn factory(world: &mut World, host: Entity, watts: f32) -> Entity {
        world.spawn((
            docked(host),
            Factory {
                current_job: Some(FactoryJob {
                    part_id: 0,
                    energy_total: 1e6,
                    energy_done: 0.0,
                    energy_et: EphemerisTime::epoch(),
                    started_et: EphemerisTime::epoch(),
                }),
                power_watts: watts,
                enabled: true,
                reserved_port: None,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{test_world::*, *};

    #[test]
    fn starved_consumers_share_the_panels_output() {
        let (mut world, host) = host();
        panel(&mut world, host, 100.0);
        electrolyzer(&mut world, host, 400.0);
        store(&mut world, host, Resource::Energy, 0.0);
        store(&mut world, host, Resource::Water, 100.0);

        assert_eq!(power_factor(&world, host), 0.25);
        // the battery neither charges nor drains
        assert_eq!(station_net_watts(&world, host), 0.0);
        // 400 W would split 0.4 kg/s; it gets a quarter of that
        let kg_s = electrolyzer_kg_per_s(&world, host);
        assert!((kg_s - 0.1).abs() < 1e-6, "{kg_s} kg/s");
    }

    #[test]
    fn a_charged_battery_runs_everything_at_full_power() {
        let (mut world, host) = host();
        panel(&mut world, host, 100.0);
        electrolyzer(&mut world, host, 400.0);
        store(&mut world, host, Resource::Energy, 1000.0);
        store(&mut world, host, Resource::Water, 100.0);

        assert_eq!(power_factor(&world, host), 1.0);
        assert_eq!(station_net_watts(&world, host), -300.0);
    }

    #[test]
    fn panels_that_cover_demand_need_no_battery() {
        let (mut world, host) = host();
        panel(&mut world, host, 500.0);
        electrolyzer(&mut world, host, 400.0);
        store(&mut world, host, Resource::Energy, 0.0);
        store(&mut world, host, Resource::Water, 100.0);

        assert_eq!(power_factor(&world, host), 1.0);
        assert_eq!(station_net_watts(&world, host), 100.0);
    }

    #[test]
    fn commit_credits_production_up_to_the_water_running_out() {
        // no panels, but a big battery: full power, splitting 1 kg/s, so the water lasts 10 s
        let (mut world, host) = host();
        electrolyzer(&mut world, host, 1000.0);
        store(&mut world, host, Resource::Energy, 1e6);
        store(&mut world, host, Resource::Water, 10.0);
        let o2 = store(&mut world, host, Resource::Oxygen, 0.0);
        let h2 = store(&mut world, host, Resource::Hydrogen, 0.0);

        let empty = EphemerisTime::epoch() + EphemerisTime::from_secs(10.0);
        commit_station(&world, host, empty);

        // all 10 kg was split, even though water commits before oxygen and hydrogen
        let o2_kg = resource_store_amount(&world, o2, empty);
        let h2_kg = resource_store_amount(&world, h2, empty);
        assert!((o2_kg - 8.881).abs() < 1e-3, "{o2_kg} kg O2");
        assert!((h2_kg - 1.119).abs() < 1e-3, "{h2_kg} kg H2");
        // and with no water left, it stops
        assert_eq!(electrolyzer_kg_per_s(&world, host), 0.0);
    }

    #[test]
    fn commit_credits_full_power_up_to_the_battery_running_out() {
        // 100 W of panels under a 400 W electrolyzer drains 300 W, so the 3 kJ battery lasts 10 s
        let (mut world, host) = host();
        panel(&mut world, host, 100.0);
        electrolyzer(&mut world, host, 400.0);
        store(&mut world, host, Resource::Energy, 3000.0);
        store(&mut world, host, Resource::Water, 100.0);
        let o2 = store(&mut world, host, Resource::Oxygen, 0.0);

        let empty = EphemerisTime::epoch() + EphemerisTime::from_secs(10.0);
        commit_station(&world, host, empty);

        // 10 s at the full 0.4 kg/s: committing energy first mustn't throttle it after the fact
        let o2_kg = resource_store_amount(&world, o2, empty);
        assert!((o2_kg - 0.4 * 10.0 * 0.8881).abs() < 1e-3, "{o2_kg} kg O2");
        // from here on it's starved
        assert_eq!(power_factor(&world, host), 0.25);
    }
}
