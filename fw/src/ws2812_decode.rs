pub const PIXELS: usize = 8;
/// These LEDs have fixed colors; preserve the brightest RGB component.
pub fn packed_brightness(bytes: &[u8]) -> [u8; PIXELS] {
    core::array::from_fn(|pixel| {
        let start = pixel * 3;
        bytes[start].max(bytes[start + 1]).max(bytes[start + 2])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_pixels_and_rgb_components_separate() {
        let wire = [
            [0, 0, 0],
            [1, 0, 0],
            [0, 128, 0],
            [0, 0, 255],
            [17, 34, 51],
            [254, 127, 63],
            [85, 170, 85],
            [3, 2, 1],
        ];
        let mut bytes = [0u8; 24];
        for (i, pixel) in wire.iter().enumerate() {
            bytes[i * 3..i * 3 + 3].copy_from_slice(pixel);
        }
        assert_eq!(packed_brightness(&bytes), [0, 1, 128, 255, 51, 254, 170, 3]);
    }
}
