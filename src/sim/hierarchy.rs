use std::collections::HashMap;

use apricot::high_precision::WorldPosition;
///! Positions and celestial ancestry
use hecs::{Entity, World};
use nalgebra_glm::{vec3, DVec3};

use crate::{
    astro::{epoch::EphemerisTime, state::State, units::SUN_MU},
    sim::{bodies::Body, docking::Docking},
};

/// Component relating an entity to a parent body
#[derive(Debug, Clone, Copy)]
pub struct Parent {
    pub id: Entity,
}

/// An entity with a name
#[derive(Debug, Clone)]
pub struct Named {
    pub name: String,
}

pub struct Landed {
    pub offset: DVec3,
}

/// Updates planets based on their on-rails orbits around their parent bodies
pub fn orbit_system(world: &mut World, now: EphemerisTime) {
    // Build parent -> children map
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();

    for (entity, (parent, _)) in world.query::<(&Parent, &State)>().iter() {
        children.entry(parent.id).or_default().push(entity);
    }

    // Collect all entities with WorldPosition
    let mut has_parent = HashMap::new();
    for (entity, parent) in world.query::<&Parent>().iter() {
        has_parent.insert(entity, parent.id);
    }

    // Find roots (entities without parent)
    let mut roots = Vec::new();
    for (entity, _) in world.query::<(&WorldPosition, &Body)>().iter() {
        if !has_parent.contains_key(&entity) {
            roots.push(entity);
        }
    }

    // Kick off from roots
    for root in roots {
        let mu = world.get::<&Body>(root).unwrap().mu;
        let root_pos = vec3(0.0, 0.0, 0.0);
        propagate(world, &children, root, root_pos, mu, now);
    }
}

fn propagate(
    world: &World,
    children: &HashMap<Entity, Vec<Entity>>,
    entity: Entity,
    parent_pos: DVec3,
    parent_mu: f64,
    t: EphemerisTime,
) {
    let mut world_pos = world.get::<&mut WorldPosition>(entity).unwrap();

    let local_offset = if let Ok(orbit) = world.get::<&State>(entity) {
        match orbit.propagate(t, parent_mu) {
            Ok(s) => s.r,
            Err(_) => return,
        }
    } else {
        vec3(0.0, 0.0, 0.0)
    };

    let new_world = parent_pos + local_offset;
    world_pos.pos = new_world;

    drop(world_pos);

    if let Some(kids) = children.get(&entity) {
        let mu = world.get::<&Body>(entity).map(|b| b.mu).unwrap_or(0.0);
        for &child in kids {
            propagate(world, children, child, new_world, mu, t);
        }
    }
}

/// Updates craft to be on the surface of their planet
pub fn landed_system(world: &mut World) {
    let mut pos_map = HashMap::new();
    for (entity, (world_pos, _body)) in world.query::<(&WorldPosition, &Body)>().iter() {
        pos_map.insert(entity, world_pos.pos);
    }

    for (_entity, (world_pos, parent, landed)) in
        world.query_mut::<(&mut WorldPosition, &Parent, &Landed)>()
    {
        let parent_pos = pos_map.get(&parent.id).unwrap();
        world_pos.pos = parent_pos + landed.offset;
    }
}

/// Updates craft to be right on their station's position
pub fn docked_position_system(world: &mut World) {
    let mut pos_map = HashMap::new();
    for (entity, world_pos) in world.query::<&WorldPosition>().iter() {
        pos_map.insert(entity, world_pos.pos);
    }

    for (_, (world_pos, docking)) in world.query_mut::<(&mut WorldPosition, &Docking)>() {
        if let Some(host_pos) = pos_map.get(&docking.host) {
            world_pos.pos = *host_pos;
        }
    }
}

pub fn get_ancestor(world: &World, entity: Entity) -> Option<Entity> {
    let mut child = entity;
    loop {
        let parent = world.get::<&Parent>(child).ok()?; // if sun, this will return None (sun has no parent)
        let parent_body = world.get::<&Body>(parent.id).ok()?;
        if parent_body.mu == SUN_MU {
            return Some(child);
        }
        child = parent.id;
    }
}

pub fn ancestor_chain(world: &World, mut selected: Entity) -> Vec<Entity> {
    let mut ancestors = vec![];

    // Who is that man in my family who said I'll fail?
    while let Ok(docking) = world.get::<&Docking>(selected) {
        ancestors.push(docking.host);
        selected = docking.host;
    }

    // finish eating, and come back again!
    while let Ok(parent) = world.get::<&Parent>(selected) {
        ancestors.push(parent.id);
        selected = parent.id;
    }

    // maybe your food is talking to you!
    ancestors
}

pub fn set_orbit(world: &mut World, craft: Entity, new_craft_orbit: State, new_parent: Entity) {
    world.remove_one::<State>(craft).ok();
    world
        .insert(craft, (new_craft_orbit, Parent { id: new_parent }))
        .unwrap();
}
