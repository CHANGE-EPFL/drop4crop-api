use crate::s3;
use anyhow::Result;
use georaster::geotiff::{GeoTiffReader, RasterValue};
use image::ImageBuffer;
use gdal::spatial_ref::{SpatialRef, CoordTransform};

// Define source and destination spatial references
let src_srs = SpatialRef::from_epsg(4326)?;    // WGS84 lat/long
let dst_srs = SpatialRef::from_epsg(3857)?;    // Web Mercator

// Create a coordinate transformer
let coord_transform = CoordTransform::new(&src_srs, &dst_srs)?;
#[derive(Debug)]
pub struct XYZTile {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// A bounding box in geographic coordinates (EPSG:4326)
#[derive(Debug)]
pub struct LatLonBoundingBox {
    /// Western (minimum) longitude in degrees
    pub min_lon: f64,
    /// Eastern (maximum) longitude in degrees
    pub max_lon: f64,
    /// Southern (minimum) latitude in degrees
    pub min_lat: f64,
    /// Northern (maximum) latitude in degrees
    pub max_lat: f64,
}

/// Compute the tile’s geographic bounds using standard slippy‐map formulas in EPSG:4326.
///
/// For a tile with indices (x, y, z):
///   n = 2^z
///   lon_left  = (x / n)*360 - 180
///   lon_right = ((x+1) / n)*360 - 180
///   lat_top   = arctan(sinh(π*(1 - 2*y/n))) * 180/π
///   lat_bottom= arctan(sinh(π*(1 - 2*(y+1)/n))) * 180/π
impl From<&XYZTile> for LatLonBoundingBox {
    fn from(tile: &XYZTile) -> Self {
        let n = 2u32.pow(tile.z) as f64;
        let min_lon = (tile.x as f64) / n * 360.0 - 180.0;
        let max_lon = ((tile.x as f64) + 1.0) / n * 360.0 - 180.0;
        let lat_top = ((std::f64::consts::PI * (1.0 - 2.0 * (tile.y as f64) / n)).sinh())
            .atan()
            .to_degrees();
        let lat_bottom = ((std::f64::consts::PI * (1.0 - 2.0 * ((tile.y as f64) + 1.0) / n))
            .sinh())
        .atan()
        .to_degrees();
        LatLonBoundingBox {
            min_lon,
            max_lon,
            min_lat: lat_bottom,
            max_lat: lat_top,
        }
    }
}

impl XYZTile {
    pub async fn get_one(&self, filename: &str) -> Result<ImageBuffer<image::Luma<u8>, Vec<u8>>> {
        // Compute the tile's geographic bounds (EPSG:4326) using standard slippy map formulas.
        let ll_bbox: LatLonBoundingBox = self.into();
        println!("Tile XYZ: x: {}, y: {}, z: {}", self.x, self.y, self.z);
        println!(
            "Tile bounds in EPSG:4326: min_lon: {:+11.6}, max_lon: {:+11.6}, min_lat: {:+11.6}, max_lat: {:+11.6}",
            ll_bbox.min_lon, ll_bbox.max_lon, ll_bbox.min_lat, ll_bbox.max_lat
        );

        // Fetch the TIFF data.
        let object = s3::get_object(filename).await?;
        let cursor = std::io::Cursor::new(object);
        let mut dataset = GeoTiffReader::open(cursor).expect("Failed to open GeoTiff");

        // Get TIFF dimensions.
        let (tiff_width, tiff_height) = dataset
            .image_info()
            .dimensions
            .ok_or_else(|| anyhow::anyhow!("Image dimensions not available."))?;
        println!("Width: {}, Height: {}", tiff_width, tiff_height);
        // println!("TIFF dimensions: {} x {}", tiff_width, tiff_height);

        // Map the geographic bounds to pixel coordinates.
        // We assume the TIFF covers exactly:
        //   Longitude: -180 to 180
        //   Latitude: 90 to -90
        let px0 = (((ll_bbox.min_lon + 180.0) / 360.0) * (tiff_width as f64))
            .floor()
            .max(0.0);
        let px1 = (((ll_bbox.max_lon + 180.0) / 360.0) * (tiff_width as f64))
            .ceil()
            .min(tiff_width as f64);
        let py0 = (((90.0 - ll_bbox.max_lat) / 180.0) * (tiff_height as f64))
            .floor()
            .max(0.0);
        let py1 = (((90.0 - ll_bbox.min_lat) / 180.0) * (tiff_height as f64))
            .ceil()
            .min(tiff_height as f64);

        let px0_u = px0 as u32;
        let px1_u = px1 as u32;
        let py0_u = py0 as u32;
        let py1_u = py1 as u32;
        println!(
            "Pixel window: top-left ({}, {}), width: {}, height: {}",
            px0_u,
            py0_u,
            px1_u - px0_u,
            py1_u - py0_u
        );
        let w = px1_u.saturating_sub(px0_u);
        let h = py1_u.saturating_sub(py0_u);

        // println!(
        //     "Pixel window: top-left ({}, {}), width: {}, height: {}",
        //     px0_u, py0_u, w, h
        // );

        // Extract the corresponding pixel window from the GeoTIFF.
        let mut temp_img = ImageBuffer::new(w, h);
        for (x, y, pixel) in dataset.pixels(px0_u, py0_u, w, h) {
            let norm = match pixel {
                RasterValue::U8(v) => v as f32 / 255.0,
                RasterValue::U16(v) => v as f32 / 65535.0,
                RasterValue::I16(v) => (v as f32 + 32768.0) / 65535.0,
                RasterValue::F32(v) => v,
                RasterValue::F64(v) => v as f32 / 65535.0,
                _ => 0.0,
            };
            let value_u8 = (norm * 255.0).clamp(0.0, 255.0).round() as u8;
            temp_img.put_pixel(x - px0_u, y - py0_u, image::Luma([value_u8]));
        }

        // Resize to 256x256 pixels (the standard size for slippy map tiles)
        let final_img = if w == 512 && h == 512 {
            temp_img
        } else {
            // println!("Resizing from {}x{} to 256x256", w, h);
            image::imageops::resize(&temp_img, 512, 512, image::imageops::FilterType::Nearest)
        };

        // println!("Final tile dimensions: {:?}", final_img.dimensions());
        // println!(
        // "Computed pixel coordinates: px0: {:.2}, px1: {:.2}, py0: {:.2}, py1: {:.2}",
        // px0, px1, py0, py1
        // );

        Ok(final_img)
    }
}

pub async fn test_get_one() {
    // Example: tile (x=0, y=0, z=2) in a standard 4326 slippy map.
    let xyz_tile = XYZTile { x: 0, y: 0, z: 2 };
    let image = xyz_tile
        .get_one("your_4326_tif.tif")
        .await
        .expect("Failed to get tile");
    image.save("output.png").expect("Failed to save image");
    println!("Saved tile as output.png");
}
