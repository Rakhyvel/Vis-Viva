use std::collections::HashMap;
use std::f32::consts::PI;

use apricot::tri::Tri;
use nalgebra_glm::Vec3;

pub struct IcosphereMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    pub tile_tris: Vec<Tri>,
}

/// Generate a flat-shaded icosphere. Each face gets 3 unique vertices so
/// normals are per-face (giving the low-poly look) and UVs are sampled from
/// the face centroid (no seam distortion).
pub fn generate(subdivisions: u32) -> IcosphereMesh {
    let mut verts = base_vertices();
    let mut faces = base_faces();

    for _ in 0..subdivisions {
        faces = subdivide_once(&mut verts, faces);
    }

    let n = faces.len();
    let mut positions = Vec::with_capacity(n * 9);
    let mut normals = Vec::with_capacity(n * 9);
    let mut uvs = Vec::with_capacity(n * 9);
    let mut indices = Vec::with_capacity(n * 3);
    let mut tile_tris = Vec::with_capacity(faces.len());

    for (i, face) in faces.iter().enumerate() {
        let a = verts[face[0] as usize];
        let b = verts[face[1] as usize];
        let c = verts[face[2] as usize];

        // Face centroid, projected onto unit sphere
        let cx = (a[0] + b[0] + c[0]) / 3.0;
        let cy = (a[1] + b[1] + c[1]) / 3.0;
        let cz = (a[2] + b[2] + c[2]) / 3.0;
        let clen = (cx * cx + cy * cy + cz * cz).sqrt();
        let norm = [cx / clen, cy / clen, cz / clen];

        let base = (i * 3) as u32;
        indices.extend_from_slice(&[base, base + 1, base + 2]);

        let (u, v) = sphere_uv(norm);

        for vert in [a, b, c] {
            positions.extend_from_slice(&vert);
            normals.extend_from_slice(&norm);
            uvs.push(u);
            uvs.push(v);
            uvs.push(0.0);
        }

        tile_tris.push(Tri::new(Vec3::from(a), Vec3::from(b), Vec3::from(c)));
    }

    IcosphereMesh {
        positions,
        normals,
        uvs,
        indices,
        tile_tris,
    }
}

fn base_vertices() -> Vec<[f32; 3]> {
    let t = (1.0f32 + 5.0f32.sqrt()) / 2.0;
    let scale = 1.0 / (1.0 + t * t).sqrt();
    let a = scale;
    let b = t * scale;
    vec![
        [-a, b, 0.0],
        [a, b, 0.0],
        [-a, -b, 0.0],
        [a, -b, 0.0],
        [0.0, -a, b],
        [0.0, a, b],
        [0.0, -a, -b],
        [0.0, a, -b],
        [b, 0.0, -a],
        [b, 0.0, a],
        [-b, 0.0, -a],
        [-b, 0.0, a],
    ]
}

fn base_faces() -> Vec<[u32; 3]> {
    vec![
        // 5 faces around vertex 0
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        // 5 adjacent faces
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        // 5 faces around vertex 3
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        // 5 adjacent faces
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ]
}

fn subdivide_once(verts: &mut Vec<[f32; 3]>, faces: Vec<[u32; 3]>) -> Vec<[u32; 3]> {
    let mut cache: HashMap<(u32, u32), u32> = HashMap::new();
    let mut new_faces = Vec::with_capacity(faces.len() * 4);

    for face in &faces {
        let [a, b, c] = *face;
        let mab = get_midpoint(verts, &mut cache, a, b);
        let mbc = get_midpoint(verts, &mut cache, b, c);
        let mac = get_midpoint(verts, &mut cache, a, c);
        new_faces.push([a, mab, mac]);
        new_faces.push([mab, b, mbc]);
        new_faces.push([mac, mbc, c]);
        new_faces.push([mab, mbc, mac]);
    }

    new_faces
}

fn get_midpoint(
    verts: &mut Vec<[f32; 3]>,
    cache: &mut HashMap<(u32, u32), u32>,
    a: u32,
    b: u32,
) -> u32 {
    let key = (a.min(b), a.max(b));
    if let Some(&idx) = cache.get(&key) {
        return idx;
    }
    let va = verts[a as usize];
    let vb = verts[b as usize];
    let mx = (va[0] + vb[0]) * 0.5;
    let my = (va[1] + vb[1]) * 0.5;
    let mz = (va[2] + vb[2]) * 0.5;
    let len = (mx * mx + my * my + mz * mz).sqrt();
    let idx = verts.len() as u32;
    verts.push([mx / len, my / len, mz / len]);
    cache.insert(key, idx);
    idx
}

fn sphere_uv(v: [f32; 3]) -> (f32, f32) {
    let x = v[0];
    let y = v[1];
    let z = v[2];

    let u = y.atan2(x) / (2.0 * PI) + 0.5;
    let v = 1.0 - (z.asin() / PI + 0.5);

    (u, v)
}
