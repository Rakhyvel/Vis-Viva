use std::f64::consts::PI;

use rand::{seq::SliceRandom, Rng, SeedableRng};

use crate::{
    astro::{
        epoch::EphemerisTime,
        state::State,
        units::{EARTH_MASSES_PER_SUN_MASS, EARTH_RADII_PER_AU, G, HOURS_PER_YEAR, SUN_MU},
    },
    sim::bodies::{Body, Category},
};

const DENSITY_IRON_G_CM3: f64 = 7.8;
const DENSITY_ROCK_G_CM3: f64 = 3.5;
const DENSITY_ICE_G_CM3: f64 = 0.92;

pub struct BodySystem {
    pub(crate) planet: (Body, State),
    pub(crate) moons: Vec<(Body, State)>,
}

struct MassCategory {
    #[allow(unused)]
    category: Category,
    range: (f64, f64),
    weight: f64,
}

enum AtmosClass {
    Airless,
    Thin,
    Temperate,
    Runaway,
}

const PLANET_MASS_CATEGORIES: &[MassCategory] = &[
    MassCategory {
        category: Category::Dwarf,
        range: (0.1, 0.3),
        weight: 4.0,
    },
    MassCategory {
        category: Category::SubEarth,
        range: (0.3, 0.8),
        weight: 10.0,
    },
    MassCategory {
        category: Category::EarthLike,
        range: (0.8, 1.5),
        weight: 10.0,
    },
    MassCategory {
        category: Category::SuperEarth,
        range: (1.5, 1.9),
        weight: 7.0,
    },
    MassCategory {
        category: Category::MiniNeptune,
        range: (1.9, 4.0),
        weight: 12.0,
    },
    MassCategory {
        category: Category::GasGiant,
        range: (4.0, 11.7),
        weight: 7.0,
    },
];

const MOON_MASS_CATEGORIES: &[MassCategory] = &[
    MassCategory {
        category: Category::Dwarf,
        range: (0.03, 0.10),
        weight: 10.0,
    }, // Teeny guys (Mimas, Enceladus)
    MassCategory {
        category: Category::Dwarf,
        range: (0.10, 0.25),
        weight: 6.0,
    }, // Normal guys (Rhea, Triton, Europa)
    MassCategory {
        category: Category::Dwarf,
        range: (0.25, 0.45),
        weight: 2.0,
    }, // BIG GUYS (Moon, Io, Callisto, Titan, Ganymede)
];

pub fn generate() -> (Vec<BodySystem>, usize) {
    let mut rng = rand::rngs::StdRng::from_entropy();

    loop {
        let (planets, starter) = generate_system(&mut rng);
        let Some(starter) = starter else { continue };

        if !all_moons_small(&planets) {
            continue;
        }
        if !has_planet(&planets, &[Category::SubEarth, Category::EarthLike], 1) {
            continue;
        }
        if !has_planet(&planets, &[Category::MiniNeptune, Category::GasGiant], 3) {
            continue;
        }
        if !no_stripped(&planets) {
            continue;
        }
        if planets.len() < 5 {
            continue;
        }

        break (planets, starter);
    }
}

fn generate_system(rng: &mut impl Rng) -> (Vec<BodySystem>, Option<usize>) {
    let mut planets: Vec<BodySystem> = vec![];
    let mut starter: Option<usize> = None;

    let mut orbital_radius_au = rng.gen_range(0.1..0.4); // in AU
    while orbital_radius_au < 35.0 {
        let orbital_radius_earth_radii = orbital_radius_au * EARTH_RADII_PER_AU;
        let planet = if (3.0..5.0).contains(&orbital_radius_au) && starter.is_none() {
            starter = Some(planets.len());
            generate_planet(
                rng,
                orbital_radius_au,
                PLANET_MASS_CATEGORIES,
                Some(3.9),
                None,
                None,
                1.0,
            )
        } else {
            generate_planet(
                rng,
                orbital_radius_au,
                PLANET_MASS_CATEGORIES,
                None,
                None,
                None,
                1.0,
            )
        };
        let planet_inclination =
            (6.0f64 / (0.5 + planet.body_radius)).to_radians() * rng.gen::<f64>().powf(2.0);
        let planet_eccentricity = sample_eccentricity(rng, planet.body_radius);
        let planet_raan = rng.gen_range(0.0..2.0 * PI);
        let initial_state = State::from_kepler(
            orbital_radius_earth_radii,
            planet_eccentricity,
            planet_inclination,
            planet_raan,
            rng.gen_range(0.0..2.0 * PI),
            rng.gen_range(0.0..2.0 * PI),
            EphemerisTime::new(0),
            SUN_MU,
        );

        let plante_au = orbital_radius_au;

        let spacing = compute_spacing(rng, orbital_radius_au, planet.body_radius);
        orbital_radius_au += spacing;

        let roche_limit = 2.44 * planet.body_radius * (planet.density).powf(1.0 / 3.0);
        let hill_sphere = orbital_radius_earth_radii
            * (planet.mass() / (3.0 * EARTH_MASSES_PER_SUN_MASS)).powf(1.0 / 3.0);

        let mut moons = vec![];
        let max = max_moons(planet.body_radius);
        let mut moon_orbital_radius = rng.gen_range(2.0..4.0) * roche_limit;
        while moon_orbital_radius < hill_sphere * 0.5 && moons.len() < max {
            let ice_retention = tidal_ice_retention(moon_orbital_radius, roche_limit);
            let mut moon = generate_planet(
                rng,
                plante_au,
                MOON_MASS_CATEGORIES,
                None,
                None,
                None, // Overriden to be tidally locked later
                ice_retention,
            );
            let moon_initial_state = State::from_kepler(
                moon_orbital_radius,
                rng.gen_range(0.0..0.02),
                (0.6_f64.to_radians()) * rng.gen::<f64>().powf(2.0) + planet_inclination, // TODO: This should be from the planets axial tilt, when we add that
                planet_raan,
                rng.gen_range(0.0..2.0 * PI),
                rng.gen_range(0.0..2.0 * PI),
                EphemerisTime::new(0),
                planet.mu,
            );
            // Set the moon to be tidally locked
            moon.rotation_period_hours =
                moon_initial_state.period(planet.mu).unwrap() * HOURS_PER_YEAR;
            moons.push((moon, moon_initial_state));
            const RESONANCES: &[(f64, f64)] = &[
                (2.0 / 1.0, 6.0), // 1.587
                (3.0 / 2.0, 3.0), // 1.310
                (5.0 / 3.0, 2.0), // 1.406
                (5.0 / 2.0, 2.0), // 1.842
                (7.0 / 4.0, 1.0), // 1.452
                (3.0 / 1.0, 1.0), // 2.080
            ];
            let (period_ratio, _) = RESONANCES
                .choose_weighted(&mut *rng, |(_, weight)| *weight)
                .unwrap();
            moon_orbital_radius *= period_ratio.powf(2.0 / 3.0);
        }

        planets.push(BodySystem {
            planet: (planet, initial_state),
            moons,
        });
    }

    (planets, starter)
}

fn generate_planet(
    rng: &mut impl Rng,
    dist_from_sun: f64,
    category_dist: &[MassCategory],
    body_radius: Option<f64>,
    core_mass_fraction: Option<f64>,
    rotation_period_hours: Option<f64>,
    ice_retention: f64,
) -> Body {
    let body_radius = body_radius.unwrap_or(sample_radius_with_au(rng, category_dist));
    let core_mass_fraction =
        core_mass_fraction.unwrap_or(sample_core_mass_fraction(rng, dist_from_sun));
    let density = estimate_density(
        core_mass_fraction,
        dist_from_sun,
        body_radius,
        ice_retention,
    );
    let rotation_period_hours =
        rotation_period_hours.unwrap_or(sample_rotation_period_hours(rng, body_radius));
    let category = categorize_planet(body_radius);
    let magnetic_field: bool =
        has_magnetic_field(body_radius, core_mass_fraction, rotation_period_hours);
    let t_eq = calculate_bare_temperature(dist_from_sun);
    let atmos_pressure: f64 = sample_atmos_pressure(
        rng,
        magnetic_field,
        body_radius,
        density,
        t_eq,
        dist_from_sun,
    );
    let temperature = apply_greenhouse(t_eq, atmos_pressure);

    pub fn mass(density: f64, body_radius: f64) -> f64 {
        let earth_density = 5.51;
        (density / earth_density) * body_radius.powi(3)
    }
    let mu = G * mass(density, body_radius);

    Body {
        category,
        body_radius,
        rotation_period_hours,
        rotation: 0.0,
        temperature,
        atmos_pressure,
        core_mass_fraction,
        magnetic_field,
        density,
        mu,
    }
}

fn max_moons(body_radius: f64) -> usize {
    (8.0 * (1.0 - (-body_radius / 5.0).exp())).round() as usize
}

#[allow(dead_code)]
fn has_habitable(planets: &[BodySystem]) -> bool {
    planets.iter().any(|p| {
        p.planet.0.habitable() && p.planet.0.category == Category::EarthLike && !p.moons.is_empty()
    })
}

#[allow(dead_code)]
fn has_planet(planets: &[BodySystem], categories: &[Category], thresh: usize) -> bool {
    let count = planets
        .iter()
        .filter(|p| categories.contains(&p.planet.0.category))
        .count();
    count >= thresh
}

#[allow(dead_code)]
fn all_moons_small(planets: &Vec<BodySystem>) -> bool {
    for system in planets {
        let limit = if system.planet.0.is_giant() {
            2.5e-4
        } else {
            0.015
        };
        let planet_mass = system.planet.0.mass();
        for moon in &system.moons {
            let moon_mass = moon.0.mass();
            if moon_mass / planet_mass > limit {
                return false;
            }
        }
    }
    true
}

#[allow(dead_code)]
fn no_stripped(planets: &[BodySystem]) -> bool {
    planets
        .iter()
        .all(|p| !p.planet.0.is_giant() || p.planet.0.gaseous())
}

/// Make smaller planets a bit more eccentric
fn sample_eccentricity(rng: &mut impl Rng, body_radius: f64) -> f64 {
    let max_e = 0.20 / (0.5 + body_radius).powf(0.35);
    max_e * rng.gen::<f64>().powf(1.5)
}

fn sample_rotation_period_hours(rng: &mut impl Rng, body_radius: f64) -> f64 {
    fn lerp(a: f64, b: f64, t: f64) -> f64 {
        a + (b - a) * t
    }

    // Make it so bigger bodies have a faster rotation
    let r = body_radius.clamp(0.1, 15.0);
    let max_hours = lerp(200.0, 15.0, ((r - 1.0) / 14.0).clamp(0.0, 1.0));

    let min = 5.0_f64.ln();
    let max = max_hours.ln();

    rng.gen_range(min..max).exp()
}

fn sample_core_mass_fraction(rng: &mut impl Rng, orbital_radius_au: f64) -> f64 {
    let base = 0.85 * (0.40f64).powf(orbital_radius_au);
    let variation = rng.gen_range(-0.05..0.05);
    (base + variation).clamp(0.05, 0.7)
}

fn estimate_density(
    core_mass_fraction: f64,
    dist_from_sun: f64,
    body_radius: f64,
    ice_retention: f64,
) -> f64 {
    let category = categorize_planet(body_radius);

    let f_ice = sample_ice_fraction(dist_from_sun, body_radius) * ice_retention;
    let f_iron = core_mass_fraction * (1.0 - f_ice);
    let f_rock = 1.0 - f_iron - f_ice;

    let base = mix_density(f_iron, f_rock, f_ice);

    match category {
        Category::Dwarf => {
            // radius at which rubble starts compacting
            const R_COMPACT: f64 = 0.047;
            let porosity = 0.25 * (1.0 - (body_radius / R_COMPACT).min(1.0));
            base * (1.0 - porosity)
        }
        Category::EarthLike | Category::SubEarth | Category::SuperEarth => {
            let compression = 1.0 + 0.3 * body_radius.powf(1.2);
            base * compression
        }
        Category::MiniNeptune => (3.0 - 0.81 * (body_radius - 1.9)).max(0.5),
        Category::GasGiant => 0.7 + 0.05 * (body_radius - 4.0),
        Category::SuperGasGiant | Category::Star => 1.0 + 0.1 * (body_radius - 15.0),
    }
}

fn sample_ice_fraction(dist_from_sun: f64, body_radius: f64) -> f64 {
    const ICE_LINE_AU: f64 = 2.0;
    if dist_from_sun <= ICE_LINE_AU {
        return 0.0;
    }

    // Ramps in over a few AU past the line
    let cold = ((dist_from_sun - ICE_LINE_AU) / 2.0).clamp(0.0, 1.0);

    // Small bodies are icier, larger are rockier
    let small = (1.0 / (1.0 + body_radius)).sqrt();

    0.42 * cold * small
}

/// Tidal heating strips volatiles from moons close to the primary
fn tidal_ice_retention(moon_a: f64, roche_limit: f64) -> f64 {
    let x = (moon_a / roche_limit / 8.0).clamp(0.0, 1.0);
    x * x
}

fn has_magnetic_field(
    body_radius: f64,
    core_mass_fraction: f64,
    rotation_period_hours: f64,
) -> bool {
    if body_radius > 1.9 {
        // gas giants always have magnetic fields
        return true;
    }
    // rocky planets need a molten core to convect
    body_radius > 0.7 && core_mass_fraction > 0.25 && rotation_period_hours < 100.0
}

fn sample_radius_with_au(rng: &mut impl Rng, category_dist: &[MassCategory]) -> f64 {
    // Compute cumulative weights
    let total_weight: f64 = category_dist.iter().map(|c| c.weight).sum();
    let mut roll = rng.gen_range(0.0..total_weight);

    let mut radius = 0.0;
    for cat in category_dist {
        if roll <= cat.weight {
            radius = rng.gen_range(cat.range.0..cat.range.1);
            break;
        }
        roll -= cat.weight;
    }

    radius
}

fn calculate_bare_temperature(orbital_radius_au: f64) -> f64 {
    278.6 * ((1.0 - 0.3) / (orbital_radius_au.powf(2.0))).powf(0.25)
}

fn sample_atmos_pressure(
    rng: &mut impl Rng,
    magnetic_field: bool,
    body_radius: f64,
    density: f64,
    t_eq: f64,
    orbital_radius_au: f64,
) -> f64 {
    if t_eq < 80.0 && body_radius < 1.9 {
        // Nothing's really gaseous down here
        return 10f64.powf(rng.gen_range(-8.0..-4.0));
    }

    // Retention score, normalized so earth ~ 1
    let t_rel = orbital_radius_au.powf(-0.5);
    let retention = (density / 5.51) * body_radius * body_radius / t_rel;

    let mag = if magnetic_field { 1.5 } else { 1.0 };
    let hot = t_eq > 400.0;

    let choices = [
        (AtmosClass::Airless, (1.0 / retention * mag).min(20.0)),
        (AtmosClass::Thin, 1.0),
        (AtmosClass::Temperate, retention * mag),
        (AtmosClass::Runaway, if hot { retention * 2.0 } else { 0.0 }),
    ];

    let (class, _) = choices.choose_weighted(&mut *rng, |(_, w)| *w).unwrap();

    match class {
        AtmosClass::Airless => 10f64.powf(rng.gen_range(-8.0..-4.0)),
        AtmosClass::Thin => 10f64.powf(rng.gen_range(-3.0..-1.3)),
        AtmosClass::Temperate => 10f64.powf(rng.gen_range(-0.5..0.8)),
        AtmosClass::Runaway => 10f64.powf(rng.gen_range(1.0..2.0)),
    }
}

fn apply_greenhouse(t_eq: f64, atmos_pressure: f64) -> f64 {
    let inv_greenhouse = (1.51 / (atmos_pressure + 1.51)).max(0.03);
    t_eq / inv_greenhouse.powf(0.25)
}

fn compute_spacing(rng: &mut impl Rng, orbital_radius_au: f64, radius: f64) -> f64 {
    let base = rng.gen_range(0.4..1.4);
    let radius_boost = 1.0 + 0.1 * radius.powf(0.25);

    orbital_radius_au * base * radius_boost
}

fn categorize_planet(radius: f64) -> Category {
    match radius {
        (0.0..0.3) => Category::Dwarf,
        (0.3..0.8) => Category::SubEarth,
        (0.8..1.5) => Category::EarthLike,
        (1.5..1.9) => Category::SuperEarth,
        (1.9..4.0) => Category::MiniNeptune,
        (4.0..15.0) => Category::GasGiant,
        (15.0..20.0) => Category::SuperGasGiant,
        _ => Category::Star,
    }
}

/// Get the bulk density of a body based on its composition
fn mix_density(f_iron: f64, f_rock: f64, f_ice: f64) -> f64 {
    1.0 / (f_iron / DENSITY_IRON_G_CM3 + f_rock / DENSITY_ROCK_G_CM3 + f_ice / DENSITY_ICE_G_CM3)
}
