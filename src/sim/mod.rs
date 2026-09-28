///! Headless game simulation code
use hecs::World;

pub mod bodies;
pub mod hierarchy;

pub struct Sim {
    world: World,
    // clock
    // events
    // parts
}

pub enum SimEffect {
    // OrbitChanged(e: Entity),
    // CraftDelivered(e: Entity),
    // FocusEntity(e: Entity),
    // CrewLost(e: Entity, r: Resource)
}

// step(dt) -> Vec<SimEffect>
// apply_command()
