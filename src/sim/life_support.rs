use hecs::{Entity, World};

use crate::{
    astro::epoch::EphemerisTime,
    sim::resources::{station_resource_amount_flow, station_resource_totals, Resource},
};

pub struct Station {
    pub num_crew: usize,
    /// Vec of every emergency on the station
    pub emergencies: Vec<Emergency>,
}

impl Station {
    pub fn most_pressing(&self) -> Option<Emergency> {
        self.emergencies.iter().min_by_key(|e| e.deadline).copied()
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Emergency {
    /// The resource that ran out
    pub cause: Resource,
    /// When the crew will die if the emergency isn't rectified
    pub deadline: EphemerisTime,
}

/// How long the crew holds out once `cause` runs out
pub fn grace_period(cause: Resource) -> EphemerisTime {
    match cause {
        Resource::Oxygen => EphemerisTime::from_mins(3.0), // Good luck!
        Resource::Water => EphemerisTime::from_days(3.0),
        r => unreachable!("{} isn't life critical", r.long_name()),
    }
}

#[derive(Debug, PartialEq)]
pub enum LifeSupportChange {
    /// Something critical ran out, the countdown starts!
    Emergency {
        station: Entity,
        emergency: Emergency,
    },
    /// The shortage was fixed
    Recovered { station: Entity, cause: Resource },
    /// Out of time
    CrewLost { station: Entity, cause: Resource },
}

pub const O2_PER_CREW_DAY: f32 = -0.84;
pub const WATER_PER_CREW_DAY: f32 = -3.5;
const CRITICAL: [Resource; 2] = [Resource::Oxygen, Resource::Water];

fn is_depleted(world: &World, station: Entity, r: Resource, now: EphemerisTime) -> bool {
    let (stored, _) = station_resource_totals(world, station, r, now);
    let flow = station_resource_amount_flow(world, station, r);
    // Under a second of supply counts as empty, so float rounding at the stop can't delay it
    flow < 0.0 && stored / -flow < 1.0
}

pub fn check_life_support(world: &World, now: EphemerisTime) -> Vec<LifeSupportChange> {
    // TODO: I think eventually we'll want heat, but that'll be something else

    let mut changes = vec![];
    for (station, s) in world.query::<&Station>().iter() {
        if s.num_crew == 0 {
            continue;
        }
        if let Some(e) = s.most_pressing().filter(|e| now >= e.deadline) {
            changes.push(LifeSupportChange::CrewLost {
                station,
                cause: e.cause,
            });
            continue;
        }
        for r in CRITICAL {
            let tracked = s.emergencies.iter().any(|e| e.cause == r);
            match (tracked, is_depleted(world, station, r, now)) {
                (false, true) => changes.push(LifeSupportChange::Emergency {
                    station,
                    emergency: Emergency {
                        cause: r,
                        deadline: now + grace_period(r),
                    },
                }),
                (true, false) => changes.push(LifeSupportChange::Recovered { station, cause: r }),
                _ => {}
            }
        }
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{docking::Docking, resources::ResourceStore};

    /// A 2-crew station with one 100 kg oxygen tank and one 100 kg water tank, both filled at the epoch
    fn station(o2_kg: f32, water_kg: f32, emergencies: Vec<Emergency>) -> (World, Entity) {
        let mut world = World::new();
        let station = world.spawn((Station {
            num_crew: 2,
            emergencies,
        },));
        for (resource, amount) in [(Resource::Oxygen, o2_kg), (Resource::Water, water_kg)] {
            world.spawn((
                Docking {
                    host: station,
                    host_port: 0,
                    own_port: 0,
                },
                ResourceStore {
                    resource,
                    amount,
                    capacity: 100.0,
                    amount_et: EphemerisTime::epoch(),
                },
            ));
        }
        (world, station)
    }

    fn out_of(cause: Resource, since: EphemerisTime) -> Emergency {
        Emergency {
            cause,
            deadline: since + grace_period(cause),
        }
    }

    #[test]
    fn running_out_starts_a_countdown() {
        let t0 = EphemerisTime::epoch();
        let (world, station) = station(0.0, 100.0, vec![]);

        assert_eq!(
            check_life_support(&world, t0),
            vec![LifeSupportChange::Emergency {
                station,
                emergency: out_of(Resource::Oxygen, t0),
            }]
        );
    }

    #[test]
    fn refilling_ends_the_countdown() {
        let t0 = EphemerisTime::epoch();
        let (world, station) = station(100.0, 100.0, vec![out_of(Resource::Oxygen, t0)]);

        assert_eq!(
            check_life_support(&world, t0),
            vec![LifeSupportChange::Recovered {
                station,
                cause: Resource::Oxygen,
            }]
        );
    }

    #[test]
    fn a_second_shortage_leaves_the_first_countdown_alone() {
        let t0 = EphemerisTime::epoch();
        // water's been out for a while, and now oxygen runs out too
        let (world, station) = station(0.0, 0.0, vec![out_of(Resource::Water, t0)]);
        let later = t0 + EphemerisTime::from_days(1.0);

        assert_eq!(
            check_life_support(&world, later),
            vec![LifeSupportChange::Emergency {
                station,
                emergency: out_of(Resource::Oxygen, later),
            }]
        );
    }

    #[test]
    fn the_crew_is_lost_at_the_earliest_deadline() {
        let t0 = EphemerisTime::epoch();
        let oxygen = out_of(Resource::Oxygen, t0);
        // water's 3 days would come later, so oxygen's 3 minutes decide it
        let (world, station) = station(0.0, 0.0, vec![out_of(Resource::Water, t0), oxygen]);

        let just_before = oxygen.deadline - EphemerisTime::from_secs(1.0);
        assert!(check_life_support(&world, just_before).is_empty());

        assert_eq!(
            check_life_support(&world, oxygen.deadline),
            vec![LifeSupportChange::CrewLost {
                station,
                cause: Resource::Oxygen,
            }]
        );
    }
}
