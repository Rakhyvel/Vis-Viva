///! Headless game simulation code. Should have no dependency on Apricot or OpenGL, making it easy to unit
/// test.
use hecs::World;

pub mod bodies;
pub mod clock;
pub mod docking;
pub mod events;
pub mod hierarchy;
pub mod industry;
pub mod life_support;
pub mod mission;
pub mod parts;
pub mod propulsion;
pub mod resources;

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
