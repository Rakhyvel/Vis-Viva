///! Headless game simulation code. Should have no dependency on Apricot or OpenGL, making it easy to unit
/// test.
use hecs::{Entity, World};

use crate::{
    astro::epoch::EphemerisTime,
    sim::{
        clock::Clock,
        docking::{Docking, PortHost},
        events::EventQueue,
        hierarchy::{docked_position_system, landed_system, orbit_system},
        industry::Factory,
        life_support::{LifeSupportChange, Station},
        mission::Command,
        parts::PartRegistry,
        propulsion::Craft,
        resources::{next_reservoir_limits, Electrolyzer, Miner, Pool, Resource},
    },
};

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
    clock: Clock,
    events: EventQueue,
    parts: PartRegistry,
    /// What happened since the last drain_effects(), for the render/UI side to react to
    effects: Vec<SimEffect>,
}

pub enum SimEffect {
    /// State/Parent changed, redraw the orbit line
    OrbitChanged {
        craft: Entity,
        soi_radius: Option<f64>,
    },
    /// No orbit anymore (landed or docked), remove the orbit line
    OrbitCleared {
        craft: Entity,
    },
    /// New creaft exists with sim components only. Attach a model and add it to the selection list
    CraftSpawned {
        craft: Entity,
    },
    /// Something the player should look at
    Focus {
        entity: Entity,
    },
    /// The clock hit a scheduled stop
    Stopped {
        at: EphemerisTime,
    },
    CrewLost {
        station: Entity,
        cause: Resource,
    },
}

impl Sim {
    pub fn new(world: World, parts: PartRegistry) -> Self {
        Self {
            world,
            clock: Clock::new(),
            events: EventQueue::new(),
            parts,
            effects: vec![],
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn parts(&self) -> &PartRegistry {
        &self.parts
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    pub fn events(&self) -> &EventQueue {
        &self.events
    }

    pub fn step(&mut self, real_dt: f64) {
        if !self.clock.paused() {
            if let Some(stop) = self.clock.advance(real_dt) {
                self.effects.push(SimEffect::Stopped { at: stop });

                for (host, _) in self.world.query::<&PortHost>().iter() {
                    resources::commit_station(&self.world, host, stop);
                }

                for event in self.events.pop_due(stop) {
                    events::apply(&mut self.world, stop, event, &mut self.effects);
                }
                industry::complete_due_jobs(&mut self.world, &self.parts, stop, &mut self.effects);
                self.recompute_run_until();
            }
        }

        // Do every frame, even paused, so a transfer clears the emergency
        self.apply_life_support();

        // Runs every frame, even while paused
        let now = self.clock.now();
        orbit_system(&mut self.world, now);
        docked_position_system(&mut self.world);
        landed_system(&mut self.world);
    }

    fn recompute_run_until(&mut self) {
        let now = self.clock.now();
        events::schedule_events(&mut self.world, &mut self.events);
        let next_event = self.events.next_time();
        let next_limit = self.next_station_limit(now);
        let next_job_complete = self.next_job_completion(now);
        let next_crew_deadline = self.next_crew_deadline();

        let run_until = [
            next_event,
            next_limit,
            next_job_complete,
            next_crew_deadline,
        ]
        .into_iter()
        .flatten()
        .min();
        self.clock.set_run_until(run_until);
    }

    fn next_job_completion(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        self.world
            .query::<(&Docking, &Factory)>()
            .iter()
            .filter_map(|(_, (docking, f))| {
                let factor = resources::power_factor(&self.world, docking.host);
                f.current_job.as_ref()?.completion_et(f, factor, now)
            })
            .min()
    }

    fn apply_life_support(&mut self) {
        let now = self.clock.now();
        for change in life_support::check_life_support(&self.world, now) {
            match change {
                LifeSupportChange::Emergency { station, emergency } => {
                    // record the emergency on the station
                    {
                        let mut station = self
                            .world
                            .get::<&mut Station>(station)
                            .expect("life support changes are only reported for stations");
                        station.emergencies.push(emergency)
                    }

                    self.clock.stop();
                    self.recompute_run_until();
                    self.effects.push(SimEffect::Focus { entity: station });
                }
                LifeSupportChange::Recovered { station, cause } => {
                    {
                        let mut station = self
                            .world
                            .get::<&mut Station>(station)
                            .expect("life support changes are only reported for stations");
                        station.emergencies.retain(|e| e.cause != cause);
                    }
                    self.recompute_run_until();
                }
                LifeSupportChange::CrewLost { station, cause } => {
                    resources::commit_station(&self.world, station, now); // crew stop consuming
                    {
                        let mut station = self
                            .world
                            .get::<&mut Station>(station)
                            .expect("life support changes are only reported for stations");
                        station.num_crew = 0;
                        station.emergencies.clear();
                    }
                    self.clock.stop();
                    self.effects.push(SimEffect::CrewLost { station, cause });
                }
            }
        }
    }

    fn next_crew_deadline(&self) -> Option<EphemerisTime> {
        self.world
            .query::<&Station>()
            .iter()
            .filter_map(|(_, s)| s.most_pressing().map(|e| e.deadline))
            .min()
    }

    fn next_station_limit(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        let mut limits = vec![];
        for (entity, _) in self.world.query::<&PortHost>().iter() {
            limits.extend(next_reservoir_limits(&self.world, entity, now));
        }

        limits.into_iter().map(|(et, _, _)| et).min()
    }

    pub fn toggle_play(&mut self) {
        self.recompute_run_until();
        self.clock.set_paused(!self.clock.paused());
    }

    pub fn speed_up(&mut self) {
        self.clock.speed_up();
    }

    pub fn slow_down(&mut self) {
        self.clock.slow_down();
    }

    /// Save `host`'s tank levels at the current time. Call before anything that changes its flow rate.
    fn commit(&self, host: Entity) {
        resources::commit_station(&self.world, host, self.clock.now());
    }

    fn host_of(&self, module: Entity) -> Entity {
        self.world.get::<&Docking>(module).unwrap().host
    }

    pub fn queue_build(&mut self, fab: Entity, part_id: u64) {
        industry::start_build(&self.world, &self.parts, fab, part_id, self.clock.now());
        self.recompute_run_until();
    }

    pub fn cancel_build(&mut self, fab: Entity) {
        industry::cancel_build(&self.world, &self.parts, fab, self.clock.now());
        self.recompute_run_until();
    }

    pub fn toggle_fabricator(&mut self, fab: Entity) {
        let host = self.host_of(fab);
        self.commit(host);
        let factor = resources::power_factor(&self.world, host);
        let now = self.clock.now();
        self.world
            .get::<&mut Factory>(fab)
            .unwrap()
            .toggle(factor, now);
    }

    pub fn toggle_electrolyzer(&mut self, e: Entity) {
        self.commit(self.host_of(e));
        let mut electrolyzer = self.world.get::<&mut Electrolyzer>(e).unwrap();
        electrolyzer.enabled = !electrolyzer.enabled
    }

    pub fn toggle_miner(&mut self, e: Entity) {
        self.commit(self.host_of(e));
        let mut miner = self.world().get::<&mut Miner>(e).unwrap();
        miner.enabled = !miner.enabled
    }

    pub fn assign_command(&mut self, craft: Entity, cmd: Command) {
        self.world.get::<&mut Craft>(craft).unwrap().command = Some(cmd)
    }

    pub fn cancel_command(&mut self, craft: Entity) {
        self.world.get::<&mut Craft>(craft).unwrap().command = None
    }

    pub fn transfer(&mut self, from: Pool, to: Pool) {
        // Moving resources can affect modules, so commit both ends first
        self.commit(from.host);
        self.commit(to.host);
        resources::transfer_resource(
            &self.world,
            from.host,
            to.host,
            from.resource,
            f32::MAX,
            self.clock.now(),
        );
    }

    pub fn undock(&mut self, craft: Entity) {
        let Ok(host) = self.world.get::<&Docking>(craft).map(|d| d.host) else {
            return;
        };
        self.commit(host);
        self.commit(craft);
        if docking::undock(&mut self.world, craft, self.clock.now()) {
            self.effects.push(SimEffect::OrbitChanged {
                craft,
                soi_radius: None,
            })
        }
    }

    pub fn drain_effects(&mut self) -> Vec<SimEffect> {
        std::mem::take(&mut self.effects)
    }
}
