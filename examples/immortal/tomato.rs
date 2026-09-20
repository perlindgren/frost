/// One tomato
struct Tomato {
    ///
    pos: [f32; 2],
    t: f32,
}

impl Tomato {
    pub fn new(size: [f32; 2]) -> Self {
        Tomato {
            pos: [0.0, 0.0],
            t: 0.0,
        }
    }

    /// Advances a tomato by `dt` seconds
    pub fn step(&mut self, dt: f32, anchors: &[[f32; 2]]) {
        self.t += dt;
        let t = self.t;
    }
}
