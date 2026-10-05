///! Fabrication
use hecs::{Entity, World};

use crate::{
    astro::epoch::EphemerisTime,
    sim::{
        docking::{free_ports, next_free_port, Docking, PortHost},
        hierarchy::{Named, ParentBody},
        parts::{ModuleSpec, PartCost, PartDef, PartInventory, PartRegistry},
        propulsion::spawn_craft,
        resources::{
            add_resource, commit_station, power_factor, station_resource_totals, take_resource,
            Miner, Resource, ResourceStore,
        },
        SimEffect,
    },
};

pub struct Factory {
    pub current_job: Option<FactoryJob>,
    pub power_watts: f32,
    pub enabled: bool,
    /// Port on the host held for the craft we're building
    pub reserved_port: Option<u32>,
}

#[derive(Debug)]
pub struct FactoryJob {
    pub part_id: u64,
    pub energy_total: f32,
    pub energy_done: f32,
    pub energy_et: EphemerisTime,
    pub started_et: EphemerisTime,
}

impl Factory {
    /// Flip on/off, banking the energy done so far so the job's progress survives being off
    pub fn toggle(&mut self, factor: f32, now: EphemerisTime) {
        let enabled = self.enabled;
        let power = self.power_watts * factor;
        if let Some(job) = &mut self.current_job {
            if enabled {
                let dt = (now - job.energy_et).as_secs() as f32;
                job.energy_done = (job.energy_done + power * dt).min(job.energy_total);
            }
            job.energy_et = now;
        }
        self.enabled = !enabled
    }
}

pub fn start_build(
    world: &World,
    parts: &PartRegistry,
    fab: Entity,
    part_id: u64,
    now: EphemerisTime,
) {
    let host = world
        .get::<&Docking>(fab)
        .expect("factories are always docked to a host")
        .host;
    let cost = &parts
        .get(part_id)
        .expect("the fabricator only offers known parts")
        .cost;

    // Commit, since we change the power draw on the station
    commit_station(world, host, now);

    // Take the parts from the inventory
    {
        let mut inv = world
            .get::<&mut PartInventory>(host)
            .expect("fabricator hosts have a part inventory");
        for (id, n) in &cost.parts {
            for _ in 0..*n {
                inv.take(*id)
                    .expect("the fabricator only offers affordable builds")
            }
        }
    }

    // Take the resources from the station
    for (r, amount) in &cost.resources {
        take_resource(world, host, *r, *amount, now);
    }

    // Reserve the port
    let reserved_port = (cost.ports_required > 0)
        .then(|| next_free_port(world, host))
        .flatten();

    // Update the factory to start building
    {
        let mut f = world
            .get::<&mut Factory>(fab)
            .expect("start_build is only called on factories");
        f.enabled = true;
        f.reserved_port = reserved_port;
        f.current_job = Some(FactoryJob {
            part_id,
            energy_total: cost.energy_joules,
            energy_done: 0.0,
            energy_et: now,
            started_et: now,
        })
    }

    // update for module ui
    world
        .get::<&mut PortHost>(host)
        .expect("fabricator hosts are port hosts")
        .dock_gen += 1;
}

pub fn cancel_build(world: &World, parts: &PartRegistry, fab: Entity, now: EphemerisTime) {
    let host = world
        .get::<&Docking>(fab)
        .expect("factories are always docked to a host")
        .host;

    // the factory stops drawing power
    commit_station(world, host, now);

    // swap the job
    let job = {
        let mut f = world
            .get::<&mut Factory>(fab)
            .expect("cancel_build is only called on factories");
        f.reserved_port = None;
        f.current_job.take()
    };

    let Some(job) = job else { return };

    // Give a full refund if no time has passed
    if job.started_et == now {
        let cost = &parts
            .get(job.part_id)
            .expect("jobs are for known parts")
            .cost;
        let mut inv = world
            .get::<&mut PartInventory>(host)
            .expect("fabricator hosts have a part inventory");
        for (id, n) in &cost.parts {
            inv.add(*id, *n);
        }
        drop(inv);
        for (r, amount) in &cost.resources {
            add_resource(world, host, *r, *amount, now);
        }
    }
}

impl FactoryJob {
    pub fn energy_at(&self, fab: &Factory, factor: f32, t: EphemerisTime) -> f32 {
        if !fab.enabled {
            return self.energy_done;
        }
        let dt = (t - self.energy_et).as_secs() as f32;
        (self.energy_done + fab.power_watts * factor * dt).min(self.energy_total)
    }

    pub fn progress(&self, fab: &Factory, factor: f32, current_et: EphemerisTime) -> f64 {
        (self.energy_at(fab, factor, current_et) / self.energy_total) as f64
    }

    pub fn completion_et(
        &self,
        fab: &Factory,
        factor: f32,
        t: EphemerisTime,
    ) -> Option<EphemerisTime> {
        if !fab.enabled || factor == 0.0 {
            return None;
        }
        let remaining = self.energy_total - self.energy_at(fab, factor, t);
        Some(t + EphemerisTime::from_secs((remaining / (fab.power_watts * factor)) as f64))
    }
}

pub fn factory_power_factor(world: &World, fab: Entity) -> f32 {
    let host = world
        .get::<&Docking>(fab)
        .expect("factories are always docked to a host")
        .host;
    power_factor(world, host)
}

pub enum CostKind {
    Part(u64),
    Resource(Resource),
    Port,
}

pub struct CostLine {
    pub kind: CostKind,
    pub need: f32,
    pub have: f32,
}

pub fn cost_status(
    world: &World,
    station: Entity,
    cost: &PartCost,
    t: EphemerisTime,
) -> Vec<CostLine> {
    let inventory = world
        .get::<&PartInventory>(station)
        .expect("fabricator hosts have a part inventory");

    let mut lines = vec![];

    for (id, need) in &cost.parts {
        lines.push(CostLine {
            kind: CostKind::Part(*id),
            need: *need as f32,
            have: inventory.quantity(*id) as f32,
        });
    }

    for (r, need) in &cost.resources {
        let (have, _) = station_resource_totals(world, station, *r, t);
        lines.push(CostLine {
            kind: CostKind::Resource(*r),
            need: *need,
            have,
        });
    }

    if cost.ports_required > 0 {
        lines.push(CostLine {
            kind: CostKind::Port,
            need: cost.ports_required as f32,
            have: free_ports(world, station) as f32,
        });
    }

    lines
}

pub fn complete_due_jobs(
    world: &mut World,
    parts: &PartRegistry,
    now: EphemerisTime,
    effects: &mut Vec<SimEffect>,
) {
    let done: Vec<(Entity, u64)> = world
        .query::<(&Docking, &Factory)>()
        .iter()
        .filter_map(|(e, (docking, f))| {
            let job = f.current_job.as_ref()?;
            let factor = power_factor(world, docking.host);
            (job.energy_at(f, factor, now) >= job.energy_total - f.power_watts)
                .then_some((e, job.part_id))
        })
        .collect();

    for (fab, part_id) in done {
        effects.push(SimEffect::Focus { entity: fab });

        let parent = world.get::<&Docking>(fab).unwrap().host;
        let def = parts.get(part_id).unwrap().clone();

        commit_station(world, parent, now);

        // Add any byproducts
        for (r, amt) in &def.byproducts {
            add_resource(world, parent, *r, *amt, now);
        }

        // For now just eject the stage
        if def.cost.ports_required > 0 {
            let host_port = {
                world
                    .get::<&Factory>(fab)
                    .unwrap()
                    .reserved_port
                    .expect("builds that need a port reserved one when queued")
            };
            deliver_craft(world, parent, &def, host_port, now, effects);
        } else {
            let mut part_inventory = world.get::<&mut PartInventory>(parent).unwrap();
            part_inventory.add(part_id, 1);
        }

        // clear job so that factory becomes idle
        if let Ok(mut f) = world.get::<&mut Factory>(fab) {
            f.current_job = None;
            f.reserved_port = None
        }
    }
}

fn deliver_craft(
    world: &mut World,
    station: Entity,
    def: &PartDef,
    host_port: u32,
    now: EphemerisTime,
    effects: &mut Vec<SimEffect>,
) {
    let parent = *world.get::<&ParentBody>(station).unwrap();

    let craft = spawn_craft(
        def.instantiate_craft(),
        Named {
            name: def.name.clone(),
        },
        parent,
        world,
    );
    effects.push(SimEffect::CraftSpawned { craft });

    world
        .insert_one(
            craft,
            PortHost {
                dock_gen: 0,
                ports: def.ports,
            },
        )
        .unwrap();

    for (port, spec) in def.modules.iter().enumerate() {
        let docking = Docking {
            host: craft,
            host_port: port as u32,
            own_port: 0 as u32, // TODO: This will work for modules now, but maybe break if modules get multiple docking ports
        };
        match *spec {
            ModuleSpec::Store {
                resource,
                amount,
                capacity,
            } => world.spawn((
                docking,
                ResourceStore {
                    resource,
                    amount,
                    capacity,
                    amount_et: now,
                },
            )),
            ModuleSpec::Miner {
                power_watts,
                kg_per_s,
            } => world.spawn((
                docking,
                Miner {
                    enabled: false,
                    kg_per_s,
                    power_watts,
                },
            )),
        };
    }

    let own_port = next_free_port(world, craft).expect("gotta have a port babey");

    world
        .insert_one(
            craft,
            Docking {
                host: station,
                host_port,
                own_port,
            },
        )
        .expect("part loading guarantees craft have a spare port for docking");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::resources::{commit_station, test_world::*, Electrolyzer, Resource};

    #[test]
    fn toggling_off_banks_progress() {
        let t0 = EphemerisTime::epoch();
        let mut fab = Factory {
            current_job: Some(FactoryJob {
                part_id: 0,
                energy_total: 1000.0,
                energy_done: 0.0,
                energy_et: t0,
                started_et: t0,
            }),
            power_watts: 10.0,
            enabled: true,
            reserved_port: None,
        };

        let t1 = t0 + EphemerisTime::from_secs(30.0);
        fab.toggle(1.0, t1);

        assert!(!fab.enabled);
        // 30 s at 10 W was banked, and nothing accrues while it's off
        let later = t1 + EphemerisTime::from_secs(100.0);
        let job = fab.current_job.as_ref().unwrap();
        assert_eq!(job.energy_at(&fab, 1.0, later), 300.0);
    }

    #[test]
    fn the_power_factor_scales_job_progress() {
        let t0 = EphemerisTime::epoch();
        let fab = Factory {
            current_job: Some(FactoryJob {
                part_id: 0,
                energy_total: 1000.0,
                energy_done: 0.0,
                energy_et: t0,
                started_et: t0,
            }),
            power_watts: 10.0,
            enabled: true,
            reserved_port: None,
        };
        let job = fab.current_job.as_ref().unwrap();

        // half power: 30 s banks 150 J, and the 1 kJ job takes 200 s instead of 100
        assert_eq!(
            job.energy_at(&fab, 0.5, t0 + EphemerisTime::from_secs(30.0)),
            150.0
        );
        assert_eq!(
            job.completion_et(&fab, 0.5, t0),
            Some(t0 + EphemerisTime::from_secs(200.0))
        );
        // no power: it never finishes
        assert_eq!(job.completion_et(&fab, 0.0, t0), None);
    }

    #[test]
    fn commit_banks_full_power_until_the_battery_ran_out() {
        // 100 W of panels under a 400 W factory drains the 3 kJ battery in 10 s
        let (mut world, host) = host();
        panel(&mut world, host, 100.0);
        let fab = factory(&mut world, host, 400.0);
        store(&mut world, host, Resource::Energy, 3000.0);

        let t0 = EphemerisTime::epoch();
        commit_station(&world, host, t0 + EphemerisTime::from_secs(10.0));

        let factor = factory_power_factor(&world, fab);
        assert_eq!(factor, 0.25);
        let f = world.get::<&Factory>(fab).unwrap();
        let job = f.current_job.as_ref().unwrap();
        // 10 s at the full 400 W...
        assert_eq!(job.energy_done, 4000.0);
        // ...then 100 W, its share of the panels
        let later = job.energy_at(&f, factor, t0 + EphemerisTime::from_secs(20.0));
        assert_eq!(later, 5000.0);
    }

    #[test]
    fn freeing_up_power_speeds_up_a_starved_job_straight_away() {
        // 400 W of panels shared between a 400 W factory and a 400 W electrolyzer: half each
        let (mut world, host) = host();
        panel(&mut world, host, 400.0);
        let fab = factory(&mut world, host, 400.0);
        let el = electrolyzer(&mut world, host, 400.0);
        store(&mut world, host, Resource::Energy, 0.0);
        store(&mut world, host, Resource::Water, 100.0);

        let t0 = EphemerisTime::epoch();
        let completion = |world: &World| {
            let factor = factory_power_factor(world, fab);
            let f = world.get::<&Factory>(fab).unwrap();
            f.current_job
                .as_ref()
                .unwrap()
                .completion_et(&f, factor, t0)
        };
        // 1 MJ at 200 W
        assert_eq!(
            completion(&world),
            Some(t0 + EphemerisTime::from_secs(5000.0))
        );

        // what Sim::toggle_electrolyzer does: commit, then switch it off
        commit_station(&world, host, t0);
        world.get::<&mut Electrolyzer>(el).unwrap().enabled = false;

        // the factory gets all 400 W right away, with no further commit
        assert_eq!(
            completion(&world),
            Some(t0 + EphemerisTime::from_secs(2500.0))
        );
    }
}
