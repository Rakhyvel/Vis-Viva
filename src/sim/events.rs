use std::collections::{btree_map, BTreeMap};

use hecs::{Entity, World};

use crate::{
    astro::{epoch::EphemerisTime, state::State},
    sim::{
        bodies::Body,
        docking::{allocate_ports, Docking, PortHost},
        hierarchy::{set_orbit, Landed, Parent},
        mission::{BurnPurpose, Command},
        propulsion::{apply_burn, Craft},
        resources::commit_station,
        SimEffect,
    },
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

        // TODO: Replace these fields with ScheduleBurn
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

    /// At this event, the craft is no longer in an orbital trajectory and is docked with the parent craft
    Dock {
        /// The craft that this event applies to
        craft: Entity,
        /// The craft to dock to
        with: Entity,
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
    events: BTreeMap<EphemerisTime, Vec<Event>>,
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

    /// Get the time of the next most recent event, if there is any
    pub fn next_time(&self) -> Option<EphemerisTime> {
        self.events.keys().next().copied()
    }

    /// Get the event iterator for these events
    pub fn iter(&self) -> btree_map::Iter<'_, EphemerisTime, Vec<Event>> {
        self.events.iter()
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub fn apply(world: &mut World, now: EphemerisTime, event: Event, effects: &mut Vec<SimEffect>) {
    match event {
        Event::SoiChange {
            craft,
            new_parent,
            new_craft_orbit,
            new_soi_radius,
            ..
        } => {
            set_orbit(world, craft, new_craft_orbit, new_parent);
            effects.push(SimEffect::OrbitChanged {
                craft,
                soi_radius: Some(new_soi_radius),
            });
            effects.push(SimEffect::Focus { entity: craft });
        }
        Event::Burn {
            craft,
            new_orbit,
            soi_radius,
            dv,
            ..
        } => {
            apply_burn(world, craft, dv, now);
            let parent = world.get::<&Parent>(craft).unwrap().id;
            set_orbit(world, craft, new_orbit, parent);
            effects.push(SimEffect::OrbitChanged { craft, soi_radius });
            effects.push(SimEffect::Focus { entity: craft });
        }
        Event::Launch { craft } => {
            commit_station(world, craft, now);
            world.remove_one::<Landed>(craft).ok();
            effects.push(SimEffect::Focus { entity: craft });
        }
        Event::Land { craft } => {
            let offset = {
                let craft_state = world.get::<&State>(craft).unwrap();
                let parent_id = world.get::<&Parent>(craft).unwrap().id;
                let parent_body_mu = world.get::<&Body>(parent_id).unwrap().mu;
                craft_state.propagate(now, parent_body_mu).unwrap().r
            };
            world.remove_one::<State>(craft).ok();
            commit_station(world, craft, now);
            world.insert(craft, (Landed { offset },)).unwrap();
            effects.push(SimEffect::OrbitCleared { craft });
            effects.push(SimEffect::Focus { entity: craft });
        }
        Event::Dock { craft, with } => {
            let Some((own_port, host_port)) = allocate_ports(world, craft, with) else {
                // port got taken while we were in transit. Just stay in orbit.
                return;
            };

            {
                let mut docks = world.get::<&mut PortHost>(craft).unwrap();
                docks.dock_gen += 1;
            }

            world.remove_one::<State>(craft).ok();
            commit_station(world, craft, now);
            world
                .insert_one(
                    craft,
                    Docking {
                        host: with,
                        host_port,
                        own_port,
                    },
                )
                .unwrap();

            effects.push(SimEffect::OrbitCleared { craft });
            effects.push(SimEffect::Focus { entity: craft });
        }
        Event::CompleteCommand { craft } => {
            let mut craft = world.get::<&mut Craft>(craft).unwrap();
            craft.command = None;
            craft.command_scheduled = false;
        }
        Event::FactoryComplete { .. } => {
            // Nothing to do here, factory completes are handled elsewhere
        }
    }
}

pub fn schedule_events(world: &World, events: &mut EventQueue) {
    let crafts_with_commands: Vec<(Entity, Command)> = world
        .query::<(&mut Craft,)>()
        .iter()
        .filter_map(|(entity, (craft,))| {
            if craft.command.is_some() && !craft.command_scheduled {
                craft.command_scheduled = true;
                craft.command.as_ref().map(|cmd| (entity, cmd.clone()))
            } else {
                None
            }
        })
        .collect();

    for (entity, command) in crafts_with_commands {
        match command {
            Command::Transfer { to, plan, .. } => {
                let departure_time = plan.transfer_state.t;
                let arrival_time = plan.flyby_state.t;
                let circ_time = plan.circ_state.t;

                println!("departure_time: {}", departure_time.as_calendar());
                println!("arrival_time.t: {}", arrival_time.as_calendar());
                println!("circ_time.t: {}", circ_time.as_calendar());

                assert!(departure_time < arrival_time);
                assert!(arrival_time < circ_time);

                let sois = command.transition_schedule();
                events.push(
                    arrival_time,
                    Event::SoiChange {
                        craft: entity,
                        new_parent: to,
                        new_craft_orbit: plan.flyby_state,
                        new_soi_radius: plan.soi_radius,
                        desc: sois[0].0,
                    },
                );

                for burn in command.burn_schedule() {
                    events.push(
                        burn.t(),
                        Event::Burn {
                            craft: entity,
                            new_orbit: burn.new_orbit,
                            soi_radius: burn.soi_radius,
                            dv: burn.dv,
                            desc: burn.desc,
                            purpose: burn.purpose,
                        },
                    )
                }

                events.push(circ_time, Event::CompleteCommand { craft: entity });
            }
            Command::Flyby { to, from, plan, .. } => {
                let departure_time = plan.transfer_state.t;
                let arrival_time = plan.flyby_state.t;
                let exit_time = plan.exit_state.t;

                println!("departure_time: {}", departure_time.as_calendar());
                println!("arrival_time.t: {}", arrival_time.as_calendar());
                println!("exit_time.t: {}", exit_time.as_calendar());

                assert!(departure_time < arrival_time);
                assert!(arrival_time < exit_time);

                let sois = command.transition_schedule();

                // Enter SOI event
                events.push(
                    arrival_time,
                    Event::SoiChange {
                        craft: entity,
                        new_parent: to,
                        new_craft_orbit: plan.flyby_state,
                        new_soi_radius: plan.soi_radius,
                        desc: sois[0].0,
                    },
                );

                // Exit SOI event
                events.push(
                    exit_time,
                    Event::SoiChange {
                        craft: entity,
                        new_parent: from,
                        new_craft_orbit: plan.exit_state,
                        new_soi_radius: plan.soi_radius,
                        desc: sois[1].0,
                    },
                );

                for burn in command.burn_schedule() {
                    events.push(
                        burn.t(),
                        Event::Burn {
                            craft: entity,
                            new_orbit: burn.new_orbit,
                            soi_radius: burn.soi_radius,
                            dv: burn.dv,
                            desc: burn.desc,
                            purpose: burn.purpose,
                        },
                    )
                }

                events.push(exit_time, Event::CompleteCommand { craft: entity });
            }
            Command::Rendezvous { plan, .. } => {
                let arrival_time = plan.rendezvous_state.t;

                for burn in command.burn_schedule() {
                    events.push(
                        burn.t(),
                        Event::Burn {
                            craft: entity,
                            new_orbit: burn.new_orbit,
                            soi_radius: burn.soi_radius,
                            dv: burn.dv,
                            desc: burn.desc,
                            purpose: burn.purpose,
                        },
                    )
                }

                events.push(arrival_time, Event::CompleteCommand { craft: entity });
            }
            Command::Escape { to, plan, .. } => {
                let departure_time = plan.escape_burn.t;
                let arrival_time = plan.exit_state.t;

                println!("departure_time: {}", departure_time.as_calendar());
                println!("arrival_time.t: {}", arrival_time.as_calendar());

                assert!(departure_time < arrival_time);

                let sois = command.transition_schedule();
                events.push(
                    arrival_time,
                    Event::SoiChange {
                        craft: entity,
                        new_parent: to,
                        new_craft_orbit: plan.exit_state,
                        new_soi_radius: plan.soi_radius,
                        desc: sois[0].0,
                    },
                );

                for burn in command.burn_schedule() {
                    events.push(
                        burn.t(),
                        Event::Burn {
                            craft: entity,
                            new_orbit: burn.new_orbit,
                            soi_radius: burn.soi_radius,
                            dv: burn.dv,
                            desc: burn.desc,
                            purpose: burn.purpose,
                        },
                    )
                }

                events.push(arrival_time, Event::CompleteCommand { craft: entity });
            }
            Command::Launch { plan, .. } => {
                let launch_time = plan.launch_burn.t;
                let circ_time = plan.circ_burn.t;

                println!("launch_time: {}", launch_time.as_calendar());
                println!("circ_time.t: {}", circ_time.as_calendar());

                assert!(launch_time < circ_time);

                events.push(launch_time, Event::Launch { craft: entity });

                for burn in command.burn_schedule() {
                    events.push(
                        burn.t(),
                        Event::Burn {
                            craft: entity,
                            new_orbit: burn.new_orbit,
                            soi_radius: burn.soi_radius,
                            dv: burn.dv,
                            desc: burn.desc,
                            purpose: burn.purpose,
                        },
                    )
                }

                events.push(circ_time, Event::CompleteCommand { craft: entity });
            }
            Command::Land { plan, .. } => {
                let deorbit_time = plan.deorbit_burn.t;
                let land_time = plan.landing_burn.t;

                println!("deorbit_time: {}", deorbit_time.as_calendar());
                println!("land_time.t: {}", land_time.as_calendar());

                assert!(deorbit_time < land_time);

                for burn in command.burn_schedule() {
                    events.push(
                        burn.t(),
                        Event::Burn {
                            craft: entity,
                            new_orbit: burn.new_orbit,
                            soi_radius: burn.soi_radius,
                            dv: burn.dv,
                            desc: burn.desc,
                            purpose: burn.purpose,
                        },
                    )
                }

                events.push(land_time, Event::Land { craft: entity });
                events.push(land_time, Event::CompleteCommand { craft: entity });
            }
            Command::Dock {
                with, arrive_et, ..
            } => {
                events.push(
                    arrive_et,
                    Event::Dock {
                        craft: entity,
                        with,
                    },
                );
                events.push(arrive_et, Event::CompleteCommand { craft: entity });
            }
        }
    }
}
