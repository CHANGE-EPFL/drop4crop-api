use crate::s3;
use anyhow::Result;
use georaster::geotiff::{GeoTiffReader, RasterValue};
use image::ImageBuffer;
use proj4rs::proj::Proj;
use proj4rs::transform::transform;

#[derive(Debug)]
pub struct XYZTile {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// A bounding box in Web Mercator (EPSG:3857).
#[derive(Debug)]
pub struct WebMercatorBoundingBox {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl From<&XYZTile> for WebMercatorBoundingBox {
    fn from(tile: &XYZTile) -> Self {
        // Standard Web Mercator: the origin shift is ~20037508.342789244.
        let origin_shift = 20037508.342789244;
        let tile_count = 2u32.pow(tile.z) as f64;
        let tile_size = (2.0 * origin_shift) / tile_count;
        let min_x = -origin_shift + tile.x as f64 * tile_size;
        let max_x = -origin_shift + (tile.x as f64 + 1.0) * tile_size;
        let max_y = origin_shift - tile.y as f64 * tile_size;
        let min_y = origin_shift - (tile.y as f64 + 1.0) * tile_size;
        WebMercatorBoundingBox {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }
}

/// A bounding box in geographic coordinates (EPSG:4326).
#[derive(Debug)]
pub struct LatLonBoundingBox {
    pub min_lon: f64,
    pub max_lon: f64,
    pub min_lat: f64,
    pub max_lat: f64,
}

impl From<&WebMercatorBoundingBox> for LatLonBoundingBox {
    fn from(wm: &WebMercatorBoundingBox) -> Self {
        // Define proj strings for EPSG:3857 and EPSG:4326.
        let from_proj_string = concat!(
            "+proj=merc +a=6378137 +b=6378137 +lat_ts=0 ",
            "+lon_0=0 +x_0=0 +y_0=0 +k=1 ",
            "+units=m +nadgrids=@null +wktext +no_defs +type=crs"
        );
        let to_proj_string = concat!("+proj=longlat +datum=WGS84 +no_defs +type=crs");

        let from = Proj::from_proj_string(from_proj_string)
            .expect("Failed to initialize EPSG:3857 projection");
        let to = Proj::from_proj_string(to_proj_string)
            .expect("Failed to initialize EPSG:4326 projection");

        // Transform the top-left corner (min_x, max_y).
        let mut point_tl = (wm.min_x, wm.max_y, 0.0);
        transform(&from, &to, &mut point_tl).expect("Projection failed for top-left");
        // The output is in radians; convert to degrees.
        let min_lon = point_tl.0.to_degrees();
        let max_lat = point_tl.1.to_degrees();

        // Transform the bottom-right corner (max_x, min_y).
        let mut point_br = (wm.max_x, wm.min_y, 0.0);
        transform(&from, &to, &mut point_br).expect("Projection failed for bottom-right");
        let max_lon = point_br.0.to_degrees();
        let min_lat = point_br.1.to_degrees();

        LatLonBoundingBox {
            min_lon,
            max_lon,
            min_lat,
            max_lat,
        }
    }
}

impl XYZTile {
    pub async fn get_one(&self, filename: &str) -> Result<ImageBuffer<image::Luma<u8>, Vec<u8>>> {
        // Get the tile's bounding box in Web Mercator.
        let wm_bbox: WebMercatorBoundingBox = self.into();
        // Project it to EPSG:4326.
        let ll_bbox: LatLonBoundingBox = (&wm_bbox).into();
        println!("Tile bounds in EPSG:4326: {:?}", ll_bbox);

        // Fetch the TIFF data.
        let object = s3::get_object(filename).await?;
        let cursor = std::io::Cursor::new(object);
        let mut dataset = GeoTiffReader::open(cursor).expect("Failed to open GeoTiff");

        // Get TIFF dimensions.
        let (tiff_width, tiff_height) = dataset
            .image_info()
            .dimensions
            .ok_or_else(|| anyhow::anyhow!("Image dimensions not available."))?;

        // Map the geographic bounding box (EPSG:4326) to pixel coordinates in the TIFF.
        // We assume the TIFF covers exactly:
        //   Longitude: -180 to 180
        //   Latitude: 90 to -90
        // Use round() for improved symmetry and precision at low zooms.
        let px0 = (((ll_bbox.min_lon + 180.0) / 360.0) * (tiff_width as f64))
            .round()
            .max(0.0);
        let px1 = (((ll_bbox.max_lon + 180.0) / 360.0) * (tiff_width as f64))
            .round()
            .min(tiff_width as f64);
        let py0 = (((90.0 - ll_bbox.max_lat) / 180.0) * (tiff_height as f64))
            .round()
            .max(0.0);
        let py1 = (((90.0 - ll_bbox.min_lat) / 180.0) * (tiff_height as f64))
            .round()
            .min(tiff_height as f64);

        let px0_u = px0 as u32;
        let px1_u = px1 as u32;
        let py0_u = py0 as u32;
        let py1_u = py1 as u32;

        let w = px1_u.saturating_sub(px0_u);
        let h = py1_u.saturating_sub(py0_u);

        println!(
            "Pixel window: top-left ({}, {}), width: {}, height: {}",
            px0_u, py0_u, w, h
        );

        // Extract the pixel window.
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

        // Resize to 512×512 if needed.
        let final_img = if w == 512 && h == 512 {
            temp_img
        } else {
            println!("Resizing from {}x{} to 512x512", w, h);
            image::imageops::resize(&temp_img, 512, 512, image::imageops::FilterType::Nearest)
        };

        println!("Final tile dimensions: {:?}", final_img.dimensions());
        println!(
            "WebMercator bbox: min_x: {}, max_x: {}, min_y: {}, max_y: {}",
            wm_bbox.min_x, wm_bbox.max_x, wm_bbox.min_y, wm_bbox.max_y
        );
        println!("Transformed to EPSG:4326: {:?}", ll_bbox);
        println!(
            "Computed pixel coordinates: px0: {:.2}, px1: {:.2}, py0: {:.2}, py1: {:.2}",
            px0, px1, py0, py1
        );

        Ok(final_img)
    }
}

pub async fn test_get_one() {
    let xyz_tile = XYZTile { x: 0, y: 0, z: 0 };
    let image = xyz_tile
        .get_one("your_4326_tif.tif")
        .await
        .expect("Failed to get tile");
    image.save("output.png").expect("Failed to save image");
    println!("Saved tile as output.png");
}
