//! The app's icon, drawn in code: two arcs chasing each other on the accent blue. A placeholder
//! until the project has artwork of its own.

/// A square RGBA icon, `size` pixels a side.
pub fn rgba(size: u32) -> Vec<u8> {
    const BLUE: [u8; 4] = [0x2d, 0x81, 0xff, 0xff];
    const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
    let centre = (size as f32 - 1.0) / 2.0;
    let (inner, outer) = (size as f32 * 0.22, size as f32 * 0.34);
    let mut out = Vec::with_capacity((size as usize).saturating_mul(size as usize).saturating_mul(4));
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - centre, y as f32 - centre);
            let radius = (dx * dx + dy * dy).sqrt();
            // Two gaps, opposite each other, turn the ring into a pair of arcs.
            let gap = (dy.atan2(dx).to_degrees() - 45.0).rem_euclid(180.0) < 36.0;
            out.extend_from_slice(if radius >= inner && radius <= outer && !gap {
                &WHITE
            } else {
                &BLUE
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_the_size_asked_for_and_two_coloured() {
        let icon = rgba(64);
        assert_eq!(icon.len(), 64 * 64 * 4);
        let white = icon.as_chunks::<4>().0.iter().filter(|p| p[0] == 0xff).count();
        assert!(white > 200 && white < 64 * 64 / 2, "{white}");
    }
}
