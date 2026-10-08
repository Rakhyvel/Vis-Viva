use hecs::{Entity, World};

use crate::sim::{
    docking::dock_root,
    resources::{committed_totals, Pool, Resource},
};

/// A transfer between one resource pool to another
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transfer {
    pub from: Pool,
    pub to: Pool,
    pub rate: f32,
}

/// How quick the resource is transfered over a single transfer, based on the resource itself
pub fn transfer_rate(r: Resource) -> f32 {
    const LOX_KG_PER_L: f32 = 1.141;
    const LH2_KG_PER_L: f32 = 0.071;

    const KG_P_S: f32 = 0.001;

    match r {
        // measured in W
        Resource::Energy => 10.0,

        // measured in kg/s, approx. 1 L/s for each resource
        Resource::Water => KG_P_S * 1.0,
        Resource::Oxygen => KG_P_S * LOX_KG_PER_L,
        Resource::Hydrogen => KG_P_S * LH2_KG_PER_L,
    }
}

impl Transfer {
    /// Whether the transfer is still going. Depends on if the `from` pool is non-empty, and the `to` pool
    /// isn't at capacity
    pub fn is_running(&self, world: &World) -> bool {
        let (from_amount, _) = committed_totals(world, self.from.host, self.from.resource);
        let (to_amount, to_capacity) = committed_totals(world, self.to.host, self.to.resource);

        from_amount > self.rate && to_capacity - to_amount > self.rate
    }

    /// Whether or not the transfer is still connected
    pub fn is_connected(&self, world: &World) -> bool {
        dock_root(world, self.from.host) == dock_root(world, self.to.host)
    }
}

/// Get the transfer entity for some source pool
pub fn transfer_from(world: &World, from: Pool) -> Option<Entity> {
    world
        .query::<&Transfer>()
        .iter()
        .find(|(_e, t)| t.from == from)
        .map(|(e, _t)| e)
}

/// Get all the transfers relating to `host` and `r`, along with their direction (+1.0 for in, -1.0 for out)
pub fn running_transfer(world: &World, host: Entity, r: Resource) -> Vec<(Transfer, f32)> {
    let expected = Pool { host, resource: r };
    world
        .query::<&Transfer>()
        .iter()
        .filter_map(|(_e, t)| {
            if !t.is_running(world) {
                None
            } else if t.from == expected {
                Some((*t, -1.0_f32))
            } else if t.to == expected {
                Some((*t, 1.0_f32))
            } else {
                None
            }
        })
        .collect()
}

/// Get the total net transfer flow rate for a (host, r) pair
pub fn transfer_flow(world: &World, host: Entity, r: Resource) -> f32 {
    let net_flow: f32 = running_transfer(world, host, r)
        .iter()
        .map(|(t, s)| t.rate * s)
        .sum();
    net_flow
}

/// Despawn any stale transfer entities
pub fn prune_transfers(world: &mut World) {
    let dead: Vec<Entity> = world
        .query::<&Transfer>()
        .iter()
        .filter(|(_e, t)| !t.is_running(world) || !t.is_connected(world))
        .map(|(e, _t)| e)
        .collect();
    for e in dead {
        world.despawn(e).expect("collected from a live query")
    }
}
