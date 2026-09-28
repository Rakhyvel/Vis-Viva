use std::collections::HashMap;

use hecs::{Entity, World};

use crate::{
    astro::epoch::EphemerisTime,
    components::{
        inventory::PartInventory,
        parts::{PartCost, PartRegistry},
        station::{station_resource_totals, Resource},
    },
    sim::docking::{free_ports, Docking},
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
