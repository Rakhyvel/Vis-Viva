use std::collections::HashMap;

use hecs::{Entity, World};

///! Fabrication
use crate::{
    astro::epoch::EphemerisTime,
    sim::{
        docking::{free_ports, next_free_port, Docking, PortHost},
        hierarchy::{Named, Parent},
        parts::{ModuleSpec, PartCost, PartDef, PartInventory, PartRegistry},
        propulsion::spawn_craft,
        resources::{
            add_resource, commit_station, station_resource_totals, take_resource, Miner, Resource,
            ResourceStore,
        },
        SimEffect,
    },
};

pub struct Factory {
    pub current_job: Option<FactoryJob>,
    pub pending_job: Option<u64>,
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
}

impl Factory {
    pub fn start_job(
        &mut self,
        part_id: u64,
        current_et: EphemerisTime,
        energy_total: f32,
    ) -> Result<(), String> {
        self.enabled = true;
        self.current_job = Some(FactoryJob {
            part_id,
            energy_done: 0.0,
            energy_total,
            energy_et: current_et,
        });
        Ok(())
    }

    /// Flip on/off, banking the energy done so far so the job's progress survives being off
    pub fn toggle(&mut self, now: EphemerisTime) {
        let enabled = self.enabled;
        let power = self.power_watts;
        if let Some(job) = &mut self.current_job {
            if enabled {
                let dt = (now - job.energy_et).as_secs() as f32;
                job.energy_done = (job.energy_done + power * dt).min(job.energy_total);
            }
            job.energy_et = now;
        }
        self.enabled = !enabled
    }

    pub fn cancel_queued(&mut self) {
        self.pending_job = None;
        self.reserved_port = None
    }

    pub fn cancel_active(&mut self) {
        self.current_job = None;
        self.reserved_port = None
    }
}

impl FactoryJob {
    pub fn energy_at(&self, fab: &Factory, t: EphemerisTime) -> f32 {
        if !fab.enabled {
            return self.energy_done;
        }
        let dt = (t - self.energy_et).as_secs() as f32;
        (self.energy_done + fab.power_watts * dt).min(self.energy_total)
    }

    pub fn progress(&self, fab: &Factory, current_et: EphemerisTime) -> f64 {
        (self.energy_at(fab, current_et) / self.energy_total) as f64
    }

    pub fn completion_et(&self, fab: &Factory, t: EphemerisTime) -> Option<EphemerisTime> {
        if !fab.enabled {
            return None;
        }
        let remaining = self.energy_total - self.energy_at(fab, t);
        Some(t + EphemerisTime::from_secs((remaining / fab.power_watts) as f64))
    }
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
    registry: &PartRegistry,
    t: EphemerisTime,
) -> Vec<CostLine> {
    let inventory = world.get::<&PartInventory>(station).unwrap();

    let (parts, resources) = station_reserved(world, station, registry);

    let mut line = vec![];

    for (id, need) in &cost.parts {
        let reserved = parts.get(id).copied().unwrap_or(0);
        let have = inventory.quantity(*id).saturating_sub(reserved);
        line.push(CostLine {
            kind: CostKind::Part(*id),
            need: *need as f32,
            have: have as f32,
        })
    }

    for (r, need) in &cost.resources {
        let (raw_have, _) = station_resource_totals(world, station, *r, t);
        let reserved = resources.get(r).copied().unwrap_or(0.0);
        let have = raw_have - reserved;
        line.push(CostLine {
            kind: CostKind::Resource(*r),
            need: *need,
            have,
        });
    }

    if cost.ports_required > 0 {
        line.push(CostLine {
            kind: CostKind::Port,
            need: cost.ports_required as f32,
            have: free_ports(world, station) as f32,
        });
    }

    line
}

// TODO: Merge station_reserved
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

pub fn station_reserved(
    world: &World,
    station: Entity,
    registry: &PartRegistry,
) -> (HashMap<u64, u32>, HashMap<Resource, f32>) {
    let mut parts = HashMap::new();
    let mut resources = HashMap::new();
    for (_, (docking, fab)) in world.query::<(&Docking, &Factory)>().iter() {
        if docking.host != station {
            continue;
        }
        let Some(id) = fab.pending_job else {
            continue;
        };
        let Some(def) = registry.get(id) else {
            continue;
        };
        for (part_id, n) in &def.cost.parts {
            *parts.entry(*part_id).or_insert(0) += n
        }
        for (r, amt) in &def.cost.resources {
            *resources.entry(*r).or_insert(0.0) += amt;
        }
    }
    (parts, resources)
}

pub fn projected_completion(
    world: &World,
    fab: Entity,
    registry: &PartRegistry,
    now: EphemerisTime,
) -> Option<EphemerisTime> {
    let f = world.get::<&Factory>(fab).ok()?;
    let def = registry.get(f.pending_job?)?;
    let secs = def.cost.energy_joules / f.power_watts;
    Some(now + EphemerisTime::from_secs(secs as f64))
}

pub fn commit_pending_builds(world: &World, parts: &PartRegistry, now: EphemerisTime) {
    // Collect the factories
    let pending: Vec<(Entity, u64)> = world
        .query::<&Factory>()
        .iter()
        .filter_map(|(e, f)| f.pending_job.map(|id| (e, id)))
        .collect();

    for (fab, part_id) in pending {
        let station = world.get::<&Parent>(fab).unwrap().id;
        let cost = &parts.get(part_id).unwrap().cost;

        // Commit the parts subtraction
        {
            let mut inv = world.get::<&mut PartInventory>(station).unwrap();
            for (id, n) in &cost.parts {
                for _ in 0..*n {
                    inv.take(*id).unwrap();
                }
            }
        }

        // Commit the resources subtraction
        for (r, amount) in &cost.resources {
            take_resource(world, station, *r, *amount, now);
        }

        // Commit at the old rate, before the job changes it.
        commit_station(world, station, now);

        {
            let mut f = world.get::<&mut Factory>(fab).unwrap();
            f.start_job(part_id, now, cost.energy_joules).unwrap();
            f.pending_job = None;
        }

        // update for module ui
        world.get::<&mut PortHost>(station).unwrap().dock_gen += 1;
    }
}

pub fn complete_due_jobs(
    world: &mut World,
    parts: &PartRegistry,
    now: EphemerisTime,
    effects: &mut Vec<SimEffect>,
) {
    let done: Vec<(Entity, u64)> = world
        .query::<&Factory>()
        .iter()
        .filter_map(|(e, f)| {
            let job = f.current_job.as_ref()?;
            (job.energy_at(f, now) >= job.energy_total - f.power_watts).then_some((e, job.part_id))
        })
        .collect();

    for (fab, part_id) in done {
        effects.push(SimEffect::Focus { entity: fab });

        let parent = world.get::<&Parent>(fab).unwrap().id;
        let def = parts.get(part_id).unwrap().clone();

        commit_station(world, parent, now);

        // Add any byproducts
        for (r, amt) in &def.byproducts {
            add_resource(world, parent, *r, *amt, now);
        }

        // For now just eject the stage
        if def.cost.ports_required > 0 {
            let host_port = { world.get::<&Factory>(fab).unwrap().reserved_port.unwrap() };
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
    let parent = *world.get::<&Parent>(station).unwrap();

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
        let parent = Parent { id: craft };
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
                parent,
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
                parent,
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
        .unwrap();
}

/// Queue `part_id` on a fabricator. It starts, and gets paid for, at the next Play
pub fn queue_build(world: &World, parts: &PartRegistry, fab: Entity, part_id: u64) {
    let host = world.get::<&Docking>(fab).unwrap().host;
    let ports = parts.get(part_id).map_or(0, |d| d.cost.ports_required);
    let reserved_port = if ports > 0 {
        next_free_port(world, host)
    } else {
        None
    };

    let mut factory = world.get::<&mut Factory>(fab).unwrap();
    factory.pending_job = Some(part_id);
    factory.reserved_port = reserved_port
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_off_banks_progress() {
        let t0 = EphemerisTime::epoch();
        let mut fab = Factory {
            current_job: Some(FactoryJob {
                part_id: 0,
                energy_total: 1000.0,
                energy_done: 0.0,
                energy_et: t0,
            }),
            pending_job: None,
            power_watts: 10.0,
            enabled: true,
            reserved_port: None,
        };

        let t1 = t0 + EphemerisTime::from_secs(30.0);
        fab.toggle(t1);

        assert!(!fab.enabled);
        // 30 s at 10 W was banked, and nothing accrues while it's off
        let later = t1 + EphemerisTime::from_secs(100.0);
        let job = fab.current_job.as_ref().unwrap();
        assert_eq!(job.energy_at(&fab, later), 300.0);
    }
}
