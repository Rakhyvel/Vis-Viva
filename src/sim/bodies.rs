use apricot::tri::Tri;
use hecs::Entity;
use nalgebra_glm::DVec3;

use crate::astro::units::G;

///! Celestial bodies
#[derive(Debug, Clone, Copy)]
pub struct Body {
    pub category: Category,
    pub body_radius: f64, // In earth radii
    pub rotation_period_hours: f64,
    pub rotation: f64,
    pub atmos_pressure: f64, // In bar
    pub temperature: f64,    // In K
    pub core_mass_fraction: f64,
    pub magnetic_field: bool,
    pub density: f64, // In g/cm^3
    pub mu: f64,      // In (earth radii)^3 * years^-2
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Category {
    Dwarf,
    SubEarth,
    EarthLike,
    SuperEarth,
    MiniNeptune,
    GasGiant,
    SuperGasGiant,
    Star,
}

#[derive(Clone, Copy)]
pub enum TileClass {
    Dwarf,
    Sub,
    Large,
}

impl Body {
    pub fn gaseous(&self) -> bool {
        self.atmos_pressure > 1.58
    }

    pub fn mass(&self) -> f64 {
        self.mu / G
    }

    pub fn habitable(&self) -> bool {
        (0.8..1.5).contains(&self.atmos_pressure) && (270.0..300.0).contains(&self.temperature)
    }

    pub fn is_giant(&self) -> bool {
        self.body_radius > 2.5
    }

    pub fn tile_class(&self) -> Option<TileClass> {
        const MARS_RADIUS: f64 = 0.532;
        if self.gaseous() {
            None
        } else if self.body_radius > MARS_RADIUS {
            Some(TileClass::Large)
        } else if self.body_radius > MARS_RADIUS * 0.5 {
            Some(TileClass::Sub)
        } else {
            Some(TileClass::Dwarf)
        }
    }
}

/// Tags a building entity with the tile index it occupies on its parent body
pub struct SurfaceTile {
    pub index: u32,
}

/// Per-body component tracking tile occupancy and the face-centroid directions
pub struct TileMap {
    pub occupants: Vec<Option<Entity>>,
    pub tris: Vec<Tri>,
}

impl TileMap {
    pub fn new(tris: Vec<Tri>) -> Self {
        let n = tris.len();
        Self {
            occupants: vec![None; n],
            tris,
        }
    }

    pub fn is_free(&self, index: u32) -> bool {
        self.occupant(index).is_none()
    }

    pub fn occupy(&mut self, index: u32, entity: Entity) {
        self.occupants[index as usize] = Some(entity);
    }

    pub fn free(&mut self, index: u32) {
        self.occupants[index as usize] = None;
    }

    pub fn occupant(&self, index: u32) -> Option<Entity> {
        self.occupants[index as usize]
    }

    /// Surface position offset for a tile, scaled to the body radius
    pub fn tile_offset(&self, index: u32, radius: f64) -> DVec3 {
        let t = self.tris[index as usize];
        let dir: DVec3 = nalgebra_glm::convert(((t.v0() + t.v1() + t.v2()) / 3.0).normalize());
        dir * radius
    }
}

pub struct TileSets {
    pub dwarf: Vec<Tri>,
    pub sub: Vec<Tri>,
    pub large: Vec<Tri>,
}

impl TileSets {
    pub fn for_class(&self, class: TileClass) -> &Vec<Tri> {
        match class {
            TileClass::Dwarf => &self.dwarf,
            TileClass::Sub => &self.sub,
            TileClass::Large => &self.large,
        }
    }
}
