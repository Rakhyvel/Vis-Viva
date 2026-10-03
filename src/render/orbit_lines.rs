use std::{collections::HashMap, f64::consts::PI, ops::Deref};

use apricot::{
    high_precision::WorldPosition,
    render_core::{LinePathComponent, RenderContext},
};
use hecs::{Entity, World};
use nalgebra_glm::DVec3;

use crate::{
    astro::{epoch::EphemerisTime, maneuver::sphere_of_influence, state::State, units::SUN_MU},
    sim::{
        bodies::Body,
        hierarchy::{get_ancestor, ParentBody},
        propulsion::Craft,
    },
    ui::style::STYLE,
};

pub fn style_orbit_lines(
    world: &mut World,
    camera_pos: DVec3,
    highlighted: Option<Entity>,
    sun: Entity,
    now: EphemerisTime,
) {
    // Extract out the world positions
    let mut pos_map = HashMap::new();
    for (entity, world_pos) in world.query::<&WorldPosition>().iter() {
        pos_map.insert(entity, world_pos.pos);
    }

    // Find which body the camera is closest to, and how close
    let mut closest_body: Option<Entity> = None;
    let mut closest_dist = f64::INFINITY;
    for (entity, (world_pos, _body)) in world.query::<(&WorldPosition, &Body)>().iter() {
        let dist = (world_pos.pos - camera_pos).norm();
        if dist < closest_dist {
            closest_dist = dist;
            closest_body = Some(entity);
        }
    }
    let closest_body = get_ancestor(world, closest_body.unwrap()).unwrap_or(sun);
    let closest_planet = get_ancestor(world, closest_body).unwrap_or(closest_body);
    let closest_planet_soi = {
        let closest_planet_body = world.get::<&Body>(closest_planet).unwrap();
        let closest_planet_orb = world.get::<&State>(closest_planet).unwrap();
        let sun_body = world.get::<&Body>(sun).unwrap();
        sphere_of_influence(
            closest_planet_orb.semi_major_axis(SUN_MU),
            closest_planet_body.mass(),
            sun_body.mass(),
        )
    };

    // Get the associated craft, if it exists
    let mut assoc_entity_map = HashMap::new();
    for (entity, _line) in world.query::<&LinePathComponent>().iter() {
        assoc_entity_map.insert(
            entity,
            world
                .get::<&AssociatedEntity>(entity)
                .map_or(Entity::DANGLING, |x| x.associate),
        );
    }

    let mut mu_map = HashMap::new();
    for (entity, (_line, parent)) in world.query::<(&LinePathComponent, &ParentBody)>().iter() {
        let parent_entity = parent.id;
        let parent_body_mu = world.get::<&Body>(parent_entity).unwrap().mu;

        mu_map.insert(entity, parent_body_mu);
    }

    let mut mean_anomaly_map = HashMap::new();
    for (entity, assoc_entity) in &assoc_entity_map {
        if *assoc_entity == Entity::DANGLING {
            mean_anomaly_map.insert(entity, 0.0);
        } else {
            let assoc_state = world
                .get::<&State>(*assoc_entity)
                .expect("the associated entity's gotta have state");
            let mu = *mu_map.get(entity).unwrap();

            // hyperbolic orbits don't have a meaningful mean anomaly, use 0
            if assoc_state.ecc(mu) >= 1.0 {
                mean_anomaly_map.insert(entity, 0.0);
            } else {
                let mean_anomaly_0 = assoc_state.mean_anomaly(mu); // M at assoc_state.t = vertex 0
                let state_now = assoc_state.propagate(now, mu).unwrap();
                let mean_anomaly = state_now.mean_anomaly(mu);
                mean_anomaly_map.insert(entity, mean_anomaly - mean_anomaly_0);
            }
        }
    }

    let mut proximity_alphas = HashMap::new();
    for (entity, (_line, _parent)) in world.query::<(&LinePathComponent, &ParentBody)>().iter() {
        let assoc_entity = *assoc_entity_map.get(&entity).unwrap();
        let assoc_planet = get_ancestor(world, assoc_entity).unwrap_or(sun);

        let camera_dist = (pos_map.get(&closest_body).unwrap() - camera_pos).norm();

        // fade if:
        let fade_orbit = if assoc_entity == assoc_planet {
            // I'm a planet, and camera is close to me
            camera_dist < closest_planet_soi
        } else {
            // I'm a moon/craft, and camera is close to a planet thats not mine
            closest_planet != assoc_planet && closest_dist < closest_planet_soi
        };

        let proximity_alpha = if fade_orbit { 0.0 } else { 1.0 };

        proximity_alphas.insert(entity, proximity_alpha);
    }

    // Set the origins of the line paths wrt the parent world positions
    for (entity, (line, world_pos, parent)) in
        world.query_mut::<(&mut LinePathComponent, &mut WorldPosition, &ParentBody)>()
    {
        let parent_pos = pos_map.get(&parent.id).unwrap();

        let assoc = *assoc_entity_map.get(&entity).unwrap();
        let is_highlighted = highlighted == Some(assoc);

        line.color = STYLE.accent;

        if is_highlighted {
            line.color.w = 1.0;
            line.width = 2.0;
        } else {
            line.color.w = 0.36606;
            line.width = 1.0;
        }

        line.color.w *= proximity_alphas.get(&entity).unwrap();

        let mean_anomaly = mean_anomaly_map.get(&entity).unwrap();
        line.seam = (mean_anomaly / (2.0 * PI)).rem_euclid(1.0) as f32;

        world_pos.pos = *parent_pos;
    }
}

pub struct AssociatedEntity {
    pub associate: Entity,
}

pub fn replace_line_path(
    world: &mut World,
    renderer: &RenderContext,
    craft_entity: Entity,
    new_line_path: Option<(
        WorldPosition,
        ParentBody,
        LinePathComponent,
        AssociatedEntity,
    )>,
) {
    let old_line_path = world.get::<&Craft>(craft_entity).unwrap().line_path_entity;

    if let Some(old) = old_line_path {
        {
            let mut line_path = world.get::<&mut LinePathComponent>(old).unwrap();
            line_path.queue_deletion(renderer);
        }
        world.despawn(old).ok();
    }

    let new_entity = new_line_path.map(|components| world.spawn(components));
    world
        .get::<&mut Craft>(craft_entity)
        .unwrap()
        .line_path_entity = new_entity;
}

pub fn redraw_orbit(
    world: &mut World,
    renderer: &RenderContext,
    craft: Entity,
    soi_radius: Option<f64>,
) {
    let state = *world.get::<&State>(craft).unwrap();
    let parent = world.get::<&ParentBody>(craft).unwrap().id;
    let parent_pos = world.get::<&WorldPosition>(parent).unwrap().pos;
    let parent_mu = world.get::<&Body>(parent).unwrap().mu;

    let vertices: Vec<f32> = state
        .generate_orbit_vertices(8192, parent_mu, soi_radius)
        .unwrap()
        .iter()
        .flat_map(|v| v.iter().map(|x| *x as f32))
        .collect();

    replace_line_path(
        world,
        renderer,
        craft,
        Some((
            WorldPosition { pos: parent_pos },
            ParentBody { id: parent },
            LinePathComponent::new(vertices),
            AssociatedEntity { associate: craft },
        )),
    );
}
