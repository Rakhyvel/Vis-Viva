pub fn ngon_mesh(sides: u32) -> (Vec<u32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut pos = vec![0.0, 0.0, 0.0]; // the center
    let mut idx = Vec::new();
    for i in 0..sides {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        pos.extend_from_slice(&[a.cos(), a.sin(), 0.0]);
        idx.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % sides]);
    }
    let n = pos.len() / 3;
    (idx, pos, [0.0, 0.0, 1.0].repeat(n), vec![0.0; n * 3])
}

pub fn ngon_ring_mesh(sides: u32, inner: f32) -> (Vec<u32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut pos = Vec::new();
    let mut idx = Vec::new();

    for i in 0..sides {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        pos.extend_from_slice(&[c, s, 0.0]); // outer, index 2i
        pos.extend_from_slice(&[c * inner, s * inner, 0.0]); // inner, index 2i+1
    }

    for i in 0..sides {
        let (o0, i0) = (2 * i, 2 * i + 1);
        let j = (i + 1) % sides;
        let (o1, i1) = (2 * j, 2 * j + 1);
        idx.extend_from_slice(&[o0, o1, i0, i0, o1, i1]);
    }

    let n = pos.len() / 3;
    (idx, pos, [0.0, 0.0, 1.0].repeat(n), vec![0.0; n * 3])
}
