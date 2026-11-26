pub struct Ring {
    data: Vec<u8>,
    head: usize,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Ring { data: vec![0; cap], head: 0 }
    }

    pub fn push(&mut self, b: u8) {
        let i = self.head % self.data.len();
        self.data[i] = b;
        self.head += 1;
    }

    pub fn read_at(&self, i: usize) -> Option<u8> {
        self.data.get(i).copied()
    }
}
