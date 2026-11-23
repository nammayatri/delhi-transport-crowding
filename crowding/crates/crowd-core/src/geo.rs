//! Small geodesy helpers.

/// Great-circle distance in metres.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().min(1.0).asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delhi_distance() {
        // Connaught Place to Anand Vihar ISBT is roughly 10 km as the crow flies.
        let d = haversine_m(28.6315, 77.2167, 28.6469, 77.3159);
        assert!((9_000.0..11_000.0).contains(&d), "{d}");
    }
}
