use apricot::{
    app::App,
    opengl::{Buffer, Vao},
};
use nalgebra_glm::Vec3;

pub struct Starbox {
    vao: Vao,
    vbo: Buffer<f32>,
    num_stars: i32,
}

impl Starbox {
    /// Create a starbox
    ///
    /// - `num_stars`: Number of stars, 10k is a good number
    /// - `ecliptic_normal`: Normal of the ecliptic plane
    /// - `galactic_concentration`: 0.0 = uniform, 1.0 = fully in plane
    pub fn new(num_stars: usize, ecliptic_normal: Vec3, galactic_concentration: f32) -> Self {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let mut data: Vec<f32> = Vec::with_capacity(num_stars * 4);
        for _ in 0..num_stars {
            // Uniform random point on unit sphere
            let x = rng.gen::<f32>() * 2.0 - 1.0;
            let y = rng.gen::<f32>() * 2.0 - 1.0;
            let z = rng.gen::<f32>() * 2.0 - 1.0;
            let mut dir = nalgebra_glm::vec3(x, y, z);

            // Bias toward the ecliptic
            let normal = ecliptic_normal.normalize();
            let along_normal = dir.dot(&normal);
            dir -= normal * along_normal * galactic_concentration;
            let dir = dir.normalize();

            // Stars in plane are brighter on average
            let base_brightness = rng.gen::<f32>();
            let plane_factor = 1.0 - along_normal.abs(); // 1.0 in plane, 0.0 at poles
            let brightness = (base_brightness * (0.4 + 0.6 * plane_factor)).clamp(0.0, 1.0);

            data.extend_from_slice(&[dir.x, dir.y, dir.z, brightness]);
        }

        let vao = Vao::gen();
        let vbo = Buffer::<f32>::gen(gl::ARRAY_BUFFER);

        unsafe {
            gl::BindVertexArray(vao.id);
        }
        vbo.set_data(&data);
        vao.set_custom(0, 4, 4, 0);
        vbo.unbind();

        Self {
            vao,
            vbo,
            num_stars: num_stars as i32,
        }
    }

    pub fn draw(&self, app: &App) {
        let (view, proj) = app.renderer.camera.borrow().view_proj_matrices();
        app.renderer.set_program(Some("starbox"));

        // Strip translation from view matrix
        let mut view_no_translate = view;
        view_no_translate[(0, 3)] = 0.0;
        view_no_translate[(1, 3)] = 0.0;
        view_no_translate[(2, 3)] = 0.0;

        unsafe {
            gl::Enable(gl::PROGRAM_POINT_SIZE);
            gl::Disable(gl::DEPTH_TEST);
            gl::Disable(gl::CULL_FACE);

            gl::BindVertexArray(self.vao.id);
            self.vbo.bind();
            gl::EnableVertexAttribArray(0);
            gl::VertexAttribPointer(
                0,
                4,
                gl::FLOAT,
                gl::FALSE,
                (4 * std::mem::size_of::<f32>()) as i32,
                std::ptr::null(),
            );

            let u_view = app.renderer.get_program_uniform("view").unwrap();
            let u_proj = app.renderer.get_program_uniform("projection").unwrap();
            gl::UniformMatrix4fv(u_view.id, 1, gl::FALSE, &view_no_translate.columns(0, 4)[0]);
            gl::UniformMatrix4fv(u_proj.id, 1, gl::FALSE, &proj.columns(0, 4)[0]);

            gl::DrawArrays(gl::POINTS, 0, self.num_stars);

            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::CULL_FACE);
        }
    }
}
