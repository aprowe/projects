pub type Tick = u64;

#[derive(Default, Clone, Copy, Debug)]
pub struct Clock {
    pub tick: Tick,
}

impl Clock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&mut self) -> Tick {
        self.tick += 1;
        self.tick
    }
}
