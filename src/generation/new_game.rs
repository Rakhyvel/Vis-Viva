use std::collections::HashMap;

use apricot::high_precision::WorldPosition;
use hecs::{Entity, World};
use nalgebra_glm::vec3;

use crate::{
    astro::{epoch::EphemerisTime, state::State, units::SUN_MU},
    generation::{lexicon::Lexicon, solar_system_gen},
    sim::{
        bodies::{Body, Category, TileMap, TileSets},
        docking::{Docking, PortHost},
        hierarchy::{Named, ParentBody},
        industry::Factory,
        life_support::Station,
        parts::{id_hash, PartInventory, PartRegistry},
        propulsion::spawn_craft,
        resources::{Electrolyzer, Resource, ResourceStore, SolarPanel},
    },
};

pub struct NewGame {
    pub world: World,
    pub bodies: Vec<Entity>,
    pub crafts: Vec<Entity>,
    pub station: Entity,
}

pub fn new_game(parts: &PartRegistry, tile_sets: &TileSets) -> NewGame {
    let mut world = World::new();
    let lexicon = Lexicon::read("res/names.lex");

    let sun = spawn_body(
        &mut world,
        Body {
            category: Category::Star,
            body_radius: 110.0,
            rotation_period_hours: 0.0,
            rotation: 0.0,
            atmos_pressure: 1000000.0,
            temperature: 5778.0,
            core_mass_fraction: 0.0,
            magnetic_field: true,
            density: 1.0,
            mu: SUN_MU,
        },
        State::circular(0.1, EphemerisTime::new(rand::random()), 1.0),
        Named {
            name: "The Sun".into(),
        },
        None,
        tile_sets,
    );

    let mut bodies = vec![sun];
    let mut crafts = vec![];

    let mut station_parent = None;
    let (planets, starter) = solar_system_gen::generate();
    for (i, system) in planets.into_iter().enumerate() {
        let name = lexicon.generate_word(7);

        let planet_entity = spawn_body(
            &mut world,
            system.planet.0,
            system.planet.1,
            Named { name },
            Some(ParentBody { id: sun }),
            &tile_sets,
        );

        bodies.push(planet_entity);
        if i == starter {
            station_parent = Some(planet_entity)
        }

        for moon in &system.moons {
            let name = lexicon.generate_word(10);
            let moon_entity = spawn_body(
                &mut world,
                moon.0,
                moon.1,
                Named { name },
                Some(ParentBody { id: planet_entity }),
                &tile_sets,
            );
            bodies.push(moon_entity);
        }
    }

    let station_parent = station_parent.expect("generator returned no station host");
    let parent_mu = world.get::<&Body>(station_parent).unwrap().mu;
    let parent_body_radius = world.get::<&Body>(station_parent).unwrap().body_radius;

    let station_payload = parts
        .all()
        .find(|p| p.id == "station_core")
        .unwrap()
        .instantiate_craft();

    let station = spawn_craft(
        station_payload,
        Named {
            name: String::from("Station"),
        },
        ParentBody { id: station_parent },
        &mut world,
    );
    let station_state = State::from_kepler(
        parent_body_radius * 16.0,
        0.2,
        0.0,
        1.5,
        0.15,
        0.15,
        EphemerisTime::new(0),
        parent_mu,
    );
    world.insert_one(station, station_state).unwrap();

    let mut starting_inventory = PartInventory {
        parts: HashMap::new(),
    };

    starting_inventory.add(id_hash("ilmenite"), 8);
    starting_inventory.add(id_hash("metal"), 8);

    world
        .insert(
            station,
            (
                Station {
                    num_crew: 2,
                    emergencies: vec![],
                },
                PortHost {
                    dock_gen: 0,
                    ports: 8,
                },
                starting_inventory,
            ),
        )
        .unwrap();
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 0,
        },
        ResourceStore {
            resource: Resource::Energy,
            amount: 4.32e8,
            capacity: 1.8e9,
            amount_et: EphemerisTime::epoch(),
        },
    ));
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 1,
        },
        SolarPanel { rated_w: 100_000.0 },
    ));
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 2,
        },
        ResourceStore {
            resource: Resource::Water,
            amount: 3800.0,
            capacity: 3800.0,
            amount_et: EphemerisTime::epoch(),
        },
    ));
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 3,
        },
        ResourceStore {
            resource: Resource::Oxygen,
            amount: 600.0,
            capacity: 600.0,
            amount_et: EphemerisTime::epoch(),
        },
    ));
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 4,
        },
        ResourceStore {
            resource: Resource::Hydrogen,
            amount: 100.0,
            capacity: 100.0,
            amount_et: EphemerisTime::epoch(),
        },
    ));
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 5,
        },
        Factory {
            current_job: None,
            power_watts: 5000.0,
            enabled: false,
            reserved_port: None,
        },
    ));
    world.spawn((
        Docking {
            host: station,
            own_port: 0,
            host_port: 6,
        },
        Electrolyzer {
            enabled: false,
            power_watts: 5_000.0,
            joules_per_kg_water: 2.52e7,
        },
    ));
    crafts.push(station);

    NewGame {
        world,
        bodies,
        crafts: vec![station],
        station,
    }
}

fn spawn_body(
    world: &mut World,
    body: Body,
    state: State,
    named: Named,
    parent: Option<ParentBody>,
    tile_sets: &TileSets,
) -> Entity {
    let tiles = body
        .tile_class()
        .map(|class| tile_sets.for_class(class).clone())
        .unwrap_or_default();

    let entity = world.spawn((
        WorldPosition {
            pos: vec3(0.0, 0.0, 0.0),
        },
        named,
        state,
        body,
        PartInventory {
            parts: HashMap::new(),
        },
        TileMap::new(tiles),
    ));
    if let Some(parent) = parent {
        world.insert_one(entity, parent).unwrap()
    }
    entity
}
