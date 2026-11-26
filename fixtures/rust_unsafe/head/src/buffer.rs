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

    /// Faster read without bounds checks.
    pub fn read_at(&self, i: usize) -> Option<u8> {
        if i >= self.head {
            return None;
        }
        unsafe { Some(*self.data.get_unchecked(i)) }
    }
}
