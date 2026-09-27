//! Deterministic test images, for golden vectors that do not depend on a
//! photo.

/// A crude face: a skin-coloured oval with eyes, a nose and a mouth on a
/// gradient background. It is not meant to fool a detector, only to give
/// the networks a smooth, structured input. RGB, `width * height * 3`
/// bytes.
pub fn face(width: usize, height: usize) -> Vec<u8> {
    let mut image = vec![0u8; width * height * 3];
    let (cx, cy) = (width as f32 / 2.0, height as f32 * 0.52);
    let (rx, ry) = (width as f32 * 0.32, height as f32 * 0.42);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32, y as f32);
            let shade = 150.0 + 60.0 * fy / height as f32;
            let mut pixel = [
                shade as u8,
                (shade * 0.95) as u8,
                (shade * 1.05).min(255.0) as u8,
            ];
            let dx = (fx - cx) / rx;
            let dy = (fy - cy) / ry;
            if dx * dx + dy * dy <= 1.0 {
                pixel = [222, 184, 150];
                if fy < cy - ry * 0.55 {
                    pixel = [60, 40, 30];
                }
            }
            for eye_x in [cx - rx * 0.4, cx + rx * 0.4] {
                let (ex, ey) = (
                    (fx - eye_x) / (rx * 0.16),
                    (fy - (cy - ry * 0.15)) / (ry * 0.1),
                );
                if ex * ex + ey * ey <= 1.0 {
                    pixel = [40, 30, 30];
                }
            }
            let (nx, ny) = (
                (fx - cx) / (rx * 0.08),
                (fy - (cy + ry * 0.15)) / (ry * 0.2),
            );
            if nx * nx + ny * ny <= 1.0 {
                pixel = [190, 140, 110];
            }
            let (mx, my) = (
                (fx - cx) / (rx * 0.35),
                (fy - (cy + ry * 0.55)) / (ry * 0.07),
            );
            if mx * mx + my * my <= 1.0 {
                pixel = [150, 60, 60];
            }
            image[(y * width + x) * 3..(y * width + x) * 3 + 3].copy_from_slice(&pixel);
        }
    }
    image
}

/// Uniform noise from a linear congruential generator, so every run gives
/// the same bytes. RGB, `width * height * 3` bytes.
pub fn noise(width: usize, height: usize, seed: u64) -> Vec<u8> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..width * height * 3)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 56) as u8
        })
        .collect()
}
