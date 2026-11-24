//! Compact Position Reporting: 17-bit even / odd latitude and longitude.
//!
//! Global decoding needs an even and an odd frame less than 10 s apart; then each new frame
//! is decoded locally against the last position (or the receiver's position).

const NZ: f64 = 15.0;
const SCALE: f64 = 131072.0; // 2^17

/// Number of longitude zones at a latitude
pub fn nl(lat: f64) -> i32 {
    let lat = lat.abs();
    if lat < 1e-9 {
        return 59;
    }
    if lat >= 87.0 {
        return if lat > 87.0 { 1 } else { 2 };
    }
    let a = 1.0 - (1.0 - (std::f64::consts::PI / (2.0 * NZ)).cos()) / lat.to_radians().cos().powi(2);
    (2.0 * std::f64::consts::PI / a.acos()).floor() as i32
}

fn modp(x: f64, y: f64) -> f64 {
    x - y * (x / y).floor()
}

/// Airborne global decoding; `odd_last`: the odd frame is the newer one (its position is returned)
pub fn global_airborne(even: (u32, u32), odd: (u32, u32), odd_last: bool) -> Option<(f64, f64)> {
    let (lat0, lon0) = (even.0 as f64, even.1 as f64);
    let (lat1, lon1) = (odd.0 as f64, odd.1 as f64);
    let j = ((59.0 * lat0 - 60.0 * lat1) / SCALE + 0.5).floor();
    let mut rlat0 = 360.0 / 60.0 * (modp(j, 60.0) + lat0 / SCALE);
    let mut rlat1 = 360.0 / 59.0 * (modp(j, 59.0) + lat1 / SCALE);
    if rlat0 >= 270.0 {
        rlat0 -= 360.0;
    }
    if rlat1 >= 270.0 {
        rlat1 -= 360.0;
    }
    if !(-90.0..=90.0).contains(&rlat0) || !(-90.0..=90.0).contains(&rlat1) || nl(rlat0) != nl(rlat1) {
        return None; // the two frames straddle a zone boundary
    }
    let (lat, n, lon_cpr) = if odd_last { (rlat1, nl(rlat1), lon1) } else { (rlat0, nl(rlat0), lon0) };
    let ni = (n - odd_last as i32).max(1) as f64;
    let m = ((lon0 * (n as f64 - 1.0) - lon1 * n as f64) / SCALE + 0.5).floor();
    let mut lon = 360.0 / ni * (modp(m, ni) + lon_cpr / SCALE);
    if lon >= 180.0 {
        lon -= 360.0;
    }
    Some((lat, lon))
}

/// Local decoding against a reference less than ~180 NM away (45 NM on the ground)
pub fn local(reference: (f64, f64), cpr: (u32, u32), odd: bool, surface: bool) -> (f64, f64) {
    let range = if surface { 90.0 } else { 360.0 };
    let (rlat, rlon) = reference;
    let (clat, clon) = (cpr.0 as f64 / SCALE, cpr.1 as f64 / SCALE);
    let dlat = range / if odd { 59.0 } else { 60.0 };
    let j = (rlat / dlat).floor() + (0.5 + modp(rlat, dlat) / dlat - clat).floor();
    let lat = dlat * (j + clat);
    let ni = (nl(lat) - odd as i32).max(1) as f64;
    let dlon = range / ni;
    let m = (rlon / dlon).floor() + (0.5 + modp(rlon, dlon) / dlon - clon).floor();
    let mut lon = dlon * (m + clon);
    if lon >= 180.0 {
        lon -= 360.0;
    }
    (lat, lon)
}

/// Great-circle distance (km) and initial bearing (degrees) from a to b
pub fn distance_bearing(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let dl = (b.1 - a.1).to_radians();
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    let d = 2.0 * 6371.0 * h.sqrt().asin();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (d, y.atan2(x).to_degrees().rem_euclid(360.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riddle_examples() {
        // 8D40621D58C382D690C8AC2863A7 (even) + 8D40621D58C386435CC412692AD6 (odd)
        let (lat, lon) = global_airborne((93000, 51372), (74158, 50194), false).unwrap();
        assert!((lat - 52.25720).abs() < 1e-4 && (lon - 3.91937).abs() < 1e-4, "{lat} {lon}");
        let (lat, lon) = local((52.258, 3.918), (93000, 51372), false, false);
        assert!((lat - 52.25720).abs() < 1e-4 && (lon - 3.91937).abs() < 1e-4, "{lat} {lon}");
        assert_eq!(nl(0.0), 59);
        assert_eq!(nl(52.2572), 36);
        let (d, b) = distance_bearing((48.0, 2.0), (49.0, 2.0));
        assert!((d - 111.2).abs() < 0.2 && b.abs() < 1e-6);
    }
}
