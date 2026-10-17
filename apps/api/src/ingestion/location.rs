pub fn valid_location_pair(lat: Option<f64>, lng: Option<f64>) -> Option<(f64, f64)> {
    lat.zip(lng).filter(|(latitude, longitude)| {
        latitude.is_finite() && longitude.is_finite() && (*latitude != 0.0 || *longitude != 0.0)
    })
}
