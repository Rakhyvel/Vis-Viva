///! Ports and the dock-tree
use hecs::{Entity, World};

use crate::{
    astro::{
        epoch::EphemerisTime, state::State, units::METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR,
    },
    sim::{
        bodies::Body,
        hierarchy::{set_orbit, ParentBody},
        industry::Factory,
        propulsion::Craft,
    },
};

/// This entity is attached to some port on `host` via one of our own ports
pub struct Docking {
    /// Who we're docked to
    pub host: Entity,
    /// The port on the host that this docking occupies
    pub host_port: u32,
    /// Our own port that we're using to dock to `host`
    pub own_port: u32,
}

pub struct PortHost {
    /// bumped whenever something is docked or undocked
    /// TODO: don't have this field
    pub dock_gen: u32,

    pub ports: u32,
}

pub fn allocate_ports(world: &World, craft: Entity, station: Entity) -> Option<(u32, u32)> {
    next_free_port(world, craft).and_then(|craft_port| {
        next_free_port(world, station).map(|station_port| (craft_port, station_port))
    })
}

pub fn next_free_port(world: &World, host: Entity) -> Option<u32> {
    let total = world.get::<&PortHost>(host).map_or(0, |p| p.ports);
    let used = used_ports(world, host);
    (0..total).find(|i| !used.contains(i))
}

pub fn free_ports(world: &World, host: Entity) -> u32 {
    let total = world.get::<&PortHost>(host).map_or(0, |p| p.ports);
    let used = used_ports(world, host);
    total.saturating_sub((0..total).filter(|i| used.contains(i)).count() as u32)
}

/// Follow the docking tree up to the docking root
pub fn dock_root(world: &World, mut e: Entity) -> Entity {
    while let Ok(host) = world.get::<&Docking>(e).map(|d| d.host) {
        e = host
    }
    e
}

/// Every craft (no modules) connected to `e` through docking, including `e` itself
pub fn dock_tree(world: &World, e: Entity) -> Vec<Entity> {
    let root = dock_root(world, e);
    world
        .query::<&Craft>()
        .iter()
        .map(|(c, _)| c)
        .filter(|c| dock_root(world, *c) == root)
        .collect()
}

fn used_ports(world: &World, host: Entity) -> Vec<u32> {
    let mut used: Vec<u32> = world
        .query::<&Docking>()
        .iter()
        .filter(|(_, d)| d.host == host)
        .map(|(_, d)| d.host_port)
        .collect();

    // Our own attachment spends one of our ports
    if let Ok(d) = world.get::<&Docking>(host) {
        used.push(d.own_port);
    }

    used.extend(
        world
            .query::<(&Docking, &Factory)>()
            .iter()
            .filter(|(_, (d, _))| d.host == host)
            .filter_map(|(_, (_, f))| f.reserved_port),
    );

    used
}

pub fn undock(world: &mut World, craft: Entity, now: EphemerisTime) -> bool {
    const SEPARATION_DV: f64 = 0.1 / METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR;

    let Ok(host) = world.get::<&Docking>(craft).map(|d| d.host) else {
        return false;
    };
    let parent = world.get::<&ParentBody>(craft).unwrap().id;
    let parent_mu = world.get::<&Body>(parent).unwrap().mu;

    let Ok(host_state) = world.get::<&State>(host).map(|s| *s) else {
        return false; // host wasn't orbiting
    };
    let Ok(mut new_state) = host_state.propagate(now, parent_mu) else {
        return false;
    };
    new_state.v += new_state.r.normalize() * SEPARATION_DV;

    world.remove_one::<Docking>(craft).ok();
    set_orbit(world, craft, new_state, parent);

    for e in [craft, host] {
        if let Ok(mut ph) = world.get::<&mut PortHost>(e) {
            ph.dock_gen += 1
        }
    }
    true
}
