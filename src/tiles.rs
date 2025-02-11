use std::f64::consts::PI;

#[derive(Debug)]
pub struct XYZTile {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

#[derive(Debug)]
pub struct BoundingBox {
    pub top: f64,
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
}
impl From<XYZTile> for BoundingBox {
    fn from(tile: XYZTile) -> Self {
        let n = 2u32.pow(tile.z) as f64;

        // Convert x to longitude boundaries.
        let left = tile.x as f64 / n * 360.0 - 180.0;
        let right = (tile.x as f64 + 1.0) / n * 360.0 - 180.0;

        // Helper function to convert a y coordinate to latitude.
        fn tile2lat(y: f64, n: f64) -> f64 {
            // Compute the latitude in radians using the inverse Mercator projection,
            // then convert to degrees.
            let lat_rad = ((PI * (1.0 - 2.0 * y / n)).sinh()).atan();
            lat_rad.to_degrees()
        }

        // For the y coordinate:
        // - The top of the tile (north edge) is given by y.
        // - The bottom of the tile (south edge) is given by y + 1.
        let top = tile2lat(tile.y as f64, n); // north
        let bottom = tile2lat(tile.y as f64 + 1.0, n); // south

        BoundingBox {
            left,
            bottom,
            right,
            top,
        }
    }
}
