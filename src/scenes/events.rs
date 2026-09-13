use std::collections::BTreeMap;

use hecs::Entity;

use crate::{
    astro::{epoch::EphemerisTime, state::State},
    components::craft::BurnPurpose,
};

#[derive(Clone, Copy)]
pub enum Event {
    /// At this event, the `craft`'s parent changes from its own parent to `new_parent`, with the `new_craft_orbit`, relative to the new parent.
    SoiChange {
        /// The craft that this event applies to
        craft: Entity,
        new_parent: Entity,
        new_craft_orbit: State,
        new_soi_radius: f64,
        /// Basic description of the SOI change
        desc: &'static str,
    },

    /// At this event, the craft performs some burn to obtain a `new_orbit`.
    Burn {
        /// The craft that this event applies to
        craft: Entity,
        /// The craft's new orbit after performing the burn
        new_orbit: State,
        /// The sphere-of-influence radius of the craft's parent
        soi_radius: Option<f64>,
        /// How much delta-v, in meters/second, the burn costs
        dv: f64,
        /// Basic description of the burn
        desc: &'static str,
        /// The purpose of this burn
        purpose: BurnPurpose,
    },

    /// At this event, the craft is no longer landed and is in a suborbital trajectory around its parent
    Launch {
        /// The craft that this event applies to
        craft: Entity,
    },

    /// At this event, the craft is no longer in an orbital trajectory and is landed on the surface of its parent
    Land {
        /// The craft that this event applies to
        craft: Entity,
    },

    FactoryComplete {
        craft: Entity,
        part_id: u64,
    },

    /// At this event, the craft's command is cleared (maneuver sequence finished)
    CompleteCommand {
        craft: Entity,
    },
}

pub struct EventQueue {
    pub events: BTreeMap<EphemerisTime, Vec<Event>>,
    version: u64,
}

impl EventQueue {
    pub fn new() -> Self {
        Self {
            events: BTreeMap::new(),
            version: 0,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn push(&mut self, time: EphemerisTime, event: Event) {
        self.events.entry(time).or_default().push(event);
        self.version += 1
    }

    /// Pop all events up to and including `current_time`
    pub fn pop_due(&mut self, current_time: EphemerisTime) -> Vec<Event> {
        let future = self
            .events
            .split_off(&(current_time + EphemerisTime::new(1)));
        self.version += 1;
        std::mem::replace(&mut self.events, future)
            .into_values()
            .flatten()
            .collect()
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}
