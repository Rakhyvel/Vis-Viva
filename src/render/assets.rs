use apricot::{opengl::create_program, render_core::RenderContext};

use crate::{
    render::{icosphere, polygon},
    sim::bodies::TileSets,
};

/// Object file data, used for meshes
pub const QUAD_XY_DATA: &[u8] = include_bytes!("../../res/quad-xy.obj");
pub const UV_DATA: &[u8] = include_bytes!("../../res/uv-sphere.obj");
pub const CONE_DATA: &[u8] = include_bytes!("../../res/cone.obj");
pub const CUBE_DATA: &[u8] = include_bytes!("../../res/cube.obj");

pub fn load_assets(renderer: &RenderContext) -> TileSets {
    // Add programs to the renderer
    renderer.add_program(
        create_program(
            include_str!("../shaders/3d.vert"),
            include_str!("../shaders/3d.frag"),
        )
        .unwrap(),
        Some("3d"),
    );
    renderer.add_program(
        create_program(
            include_str!("../shaders/2d.vert"),
            include_str!("../shaders/2d.frag"),
        )
        .unwrap(),
        Some("2d"),
    );
    renderer.add_program(
        create_program(
            include_str!("../shaders/shadow.vert"),
            include_str!("../shaders/shadow.frag"),
        )
        .unwrap(),
        Some("shadow"),
    );
    renderer.add_program(
        create_program(
            include_str!("../shaders/2d.vert"),
            include_str!("../shaders/solid-color.frag"),
        )
        .unwrap(),
        Some("2d-solid"),
    );
    renderer.add_program(
        create_program(
            include_str!("../shaders/3d.vert"),
            include_str!("../shaders/solid-color.frag"),
        )
        .unwrap(),
        Some("3d-solid"),
    );
    renderer.add_program(
        create_program(
            include_str!("../shaders/line.vert"),
            include_str!("../shaders/line.frag"),
        )
        .unwrap(),
        Some("line"),
    );
    renderer.add_program(
        create_program(
            include_str!("../shaders/starbox.vert"),
            include_str!("../shaders/starbox.frag"),
        )
        .unwrap(),
        Some("starbox"),
    );

    // Setup the mesh manager
    renderer.add_mesh_from_obj(QUAD_XY_DATA, Some("quad-xy"));
    renderer.add_mesh_from_obj(UV_DATA, Some("uv"));
    renderer.add_mesh_from_obj(CONE_DATA, Some("cone"));
    renderer.add_mesh_from_obj(CUBE_DATA, Some("cube"));

    let ico_20 = icosphere::generate(0); // 20-face icosphere for dwarf bodies
    let ico_80 = icosphere::generate(1); // 80-face icosphere for mars-like sub-earths
    let ico_320 = icosphere::generate(2); // 320-face icosphere for large rocky bodies
    renderer.add_mesh_from_verts(
        ico_20.indices.clone(),
        vec![&ico_20.positions, &ico_20.normals, &ico_20.uvs],
        Some("ico-20"),
    );
    renderer.add_mesh_from_verts(
        ico_80.indices.clone(),
        vec![&ico_80.positions, &ico_80.normals, &ico_80.uvs],
        Some("ico-80"),
    );
    renderer.add_mesh_from_verts(
        ico_320.indices.clone(),
        vec![&ico_320.positions, &ico_320.normals, &ico_320.uvs],
        Some("ico-320"),
    );

    for (i, name) in ["triangle", "square", "pentagon", "hexagon"]
        .iter()
        .enumerate()
    {
        let sides = i + 3;
        let (indices, pos, normals, uvs) = polygon::ngon_mesh(sides as u32);
        renderer.add_mesh_from_verts(indices, vec![&pos, &normals, &uvs], Some(name));
    }

    for (i, name) in [
        "triangle-outline",
        "square-outline",
        "pentagon-outline",
        "hexagon-outline",
        "septagon-outline",
        "octagon-outline",
    ]
    .iter()
    .enumerate()
    {
        let sides = i + 3;
        let (indices, pos, normals, uvs) = polygon::ngon_ring_mesh(sides as u32, 0.875);
        renderer.add_mesh_from_verts(indices, vec![&pos, &normals, &uvs], Some(name));
    }

    // Setup the texture manager
    renderer.add_texture_from_png("res/sun.png", Some("sun"));
    renderer.add_texture_from_png("res/venus.png", Some("venus"));
    renderer.add_texture_from_png("res/earth.png", Some("earth"));
    renderer.add_texture_from_png("res/moon.png", Some("moon"));
    renderer.add_texture_from_png("res/jupiter.png", Some("jupiter"));
    renderer.add_texture_from_png("res/europa.png", Some("europa"));
    renderer.add_texture_from_png("res/uranus.png", Some("uranus"));
    renderer.add_texture_from_png("res/next-turn.png", Some("next-turn"));
    renderer.add_texture_from_png("res/next-turn-hover.png", Some("next-turn-hover"));
    renderer.add_texture_from_png("res/reticle.png", Some("reticle"));

    // Setup the font manager
    renderer.add_font("res/Consolas.ttf", "font", 15, sdl2::ttf::FontStyle::NORMAL);
    renderer.add_font(
        "res/Consolas.ttf",
        "font-small-bold",
        16,
        sdl2::ttf::FontStyle::BOLD,
    );
    renderer.add_font(
        "res/Consolas.ttf",
        "font-small-italic",
        16,
        sdl2::ttf::FontStyle::ITALIC,
    );
    renderer.add_font(
        "res/Consolas.ttf",
        "font-big",
        21,
        sdl2::ttf::FontStyle::BOLD,
    );

    TileSets {
        dwarf: ico_20.tile_tris,
        sub: ico_80.tile_tris,
        large: ico_320.tile_tris,
    }
}
