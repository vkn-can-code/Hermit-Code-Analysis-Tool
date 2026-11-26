pub mod buffer;
use buffer::Ring;

pub fn checksum(r: &Ring, n: usize) -> u32 {
    let mut sum = 0u32;
    for i in 0..n {
        if let Some(b) = r.read_at(i) {
            sum = sum.wrapping_add(b as u32);
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_of_pushed_bytes() {
        let mut r = Ring::new(4);
        r.push(1);
        r.push(2);
        assert_eq!(checksum(&r, 2), 3);
    }

    #[test]
    fn new_ring_is_zeroed() {
        let r = Ring::new(2);
        assert_eq!(r.read_at(5), None);
    }
}
