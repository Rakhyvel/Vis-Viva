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
        life_support::Station,
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

                for event in self.events.pop_due(stop) {
                    events::apply(&mut self.world, stop, event, &mut self.effects);
                }
                industry::complete_due_jobs(&mut self.world, &self.parts, stop, &mut self.effects);
                self.recompute_run_until();
            }

            let now = self.clock.now();
            if let Some((station, cause)) = life_support::crew_death(&mut self.world, now) {
                resources::commit_station(&self.world, station, now);
                self.world.get::<&mut Station>(station).unwrap().num_crew = 0;
                self.clock.stop();
                self.effects.push(SimEffect::CrewLost { station, cause });
            }
        }

        // Runs every frame, even while paused
        let now = self.clock.now();
        orbit_system(&mut self.world, now);
        docked_position_system(&mut self.world);
        landed_system(&mut self.world);
    }

    fn recompute_run_until(&mut self) {
        let now = self.clock.now();
        events::schedule_events(&mut self.world, &mut self.events);
        let next_event = self.events.events.keys().next().copied();
        let next_limit = self.next_station_limit(now);
        let next_job_complete = self.next_job_completion(now);

        let run_until = [next_event, next_limit, next_job_complete]
            .into_iter()
            .flatten()
            .min();
        self.clock.set_run_until(run_until);
    }

    fn next_job_completion(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        self.world
            .query::<&Factory>()
            .iter()
            .filter_map(|(_, f)| f.current_job.as_ref()?.completion_et(f, now))
            .min()
    }

    fn next_station_limit(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        let mut limits = vec![];
        for (entity, _) in self.world.query::<&PortHost>().iter() {
            limits.extend(next_reservoir_limits(
                &self.world,
                entity,
                &self.parts,
                now,
                true,
            ));
        }

        limits.into_iter().map(|(et, _, _)| et).min()
    }

    pub fn toggle_play(&mut self) {
        industry::commit_pending_builds(&self.world, &self.parts, self.clock.now());
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
        industry::queue_build(&self.world, &self.parts, fab, part_id);
    }

    pub fn cancel_queued_build(&mut self, fab: Entity) {
        self.world.get::<&mut Factory>(fab).unwrap().cancel_queued();
    }

    pub fn cancel_active_build(&mut self, fab: Entity) {
        self.commit(self.host_of(fab)); // the job's power draw stops
        self.world.get::<&mut Factory>(fab).unwrap().cancel_active();
    }

    pub fn toggle_fabricator(&mut self, fab: Entity) {
        self.commit(self.host_of(fab));
        let now = self.clock.now();
        self.world.get::<&mut Factory>(fab).unwrap().toggle(now);
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
