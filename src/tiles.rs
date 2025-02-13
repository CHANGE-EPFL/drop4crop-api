use crate::s3;
use anyhow::Result;
use georaster::geotiff::{GeoTiffReader, RasterValue};
use image::ImageBuffer;
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

impl From<&XYZTile> for BoundingBox {
    fn from(tile: &XYZTile) -> Self {
        // Compute geographic bounds (in Web Mercator meters) for the tile.
        // The entire world in Web Mercator is approximately [-origin_shift, origin_shift] in x
        // and [origin_shift, -origin_shift] in y.
        const R: f64 = 6378137.0;
        let tile_count = 2u32.pow(tile.z) as f64;
        let tile_width_m = (2.0 * PI * R) / tile_count;
        let origin_shift = PI * R;

        let min_x_m = -origin_shift + tile.x as f64 * tile_width_m;
        let max_x_m = -origin_shift + (tile.x as f64 + 1.0) * tile_width_m;
        let max_y_m = origin_shift - tile.y as f64 * tile_width_m;
        let min_y_m = origin_shift - (tile.y as f64 + 1.0) * tile_width_m;

        BoundingBox {
            left: min_x_m,
            top: max_y_m,
            right: max_x_m,
            bottom: min_y_m,
        }
    }
}

impl XYZTile {
    pub async fn get_one(&self, filename: &str) -> Result<ImageBuffer<image::Luma<u8>, Vec<u8>>> {
        let object = s3::get_object(filename).await;
        match &object {
            Ok(data) => println!(
                "Object size: {:.2} MB",
                data.len() as f64 / (1024.0 * 1024.0)
            ),
            Err(e) => eprintln!("Failed to get object: {:?}", e),
        }
        let data = match object {
            Ok(data) => data,
            Err(e) => return Err(e),
        };

        // Compute the geographic bounds for this tile.
        let bounds: BoundingBox = self.into();

        // Open the GeoTIFF from the in-memory data.
        let cursor = std::io::Cursor::new(data);
        let mut dataset = GeoTiffReader::open(cursor).expect("Failed to open GeoTiff");

        // Get the TIFF's dimensions.
        let (tiff_width, tiff_height) = if let Some(dim) = dataset.image_info().dimensions {
            dim
        } else {
            return Err(anyhow::anyhow!("Image dimensions not available."));
        };

        // Here we assume the TIFF covers the entire world in Web Mercator.
        // Web Mercator extent (in meters):
        // x: [-origin_shift, origin_shift]
        // y: [origin_shift, -origin_shift]
        const R: f64 = 6378137.0;
        let origin_shift = PI * R;
        let world_width = 2.0 * origin_shift;
        let world_height = 2.0 * origin_shift;

        // Convert geographic coordinates (in meters) to pixel coordinates in the TIFF.
        // For x: ((x_geo - (-origin_shift)) / world_width) * tiff_width
        // For y: ((origin_shift - y_geo) / world_height) * tiff_height
        let tile_px0 =
            (((bounds.left + origin_shift) / world_width) * (tiff_width as f64)).round() as u32;
        let tile_py0 =
            (((origin_shift - bounds.top) / world_height) * (tiff_height as f64)).round() as u32;
        let tile_px1 =
            (((bounds.right + origin_shift) / world_width) * (tiff_width as f64)).round() as u32;
        let tile_py1 =
            (((origin_shift - bounds.bottom) / world_height) * (tiff_height as f64)).round() as u32;

        let w = tile_px1.saturating_sub(tile_px0);
        let h = tile_py1.saturating_sub(tile_py0);

        println!(
            "Computed tile pixel window: top-left ({}, {}), width: {}, height: {}",
            tile_px0, tile_py0, w, h
        );

        // If the computed window is not exactly 512x512, warn and later resize using nearest-neighbor.
        if w != 512 || h != 512 {
            println!(
                "Warning: expected window size 512x512 but got {}x{}. Will resize using nearest-neighbor.",
                w, h
            );
        }

        // Extract the tile region from the TIFF.
        let mut temp_img = ImageBuffer::new(w, h);
        for (x, y, pixel) in dataset.pixels(tile_px0, tile_py0, w, h) {
            let norm = match pixel {
                RasterValue::U8(v) => v as f32 / 255.0,
                RasterValue::U16(v) => v as f32 / 65535.0,
                RasterValue::I16(v) => (v as f32 + 32768.0) / 65535.0,
                RasterValue::F32(v) => v,
                RasterValue::F64(v) => v as f32 / 65535.0,
                _ => 0.0,
            };
            let value_u8 = (norm * 255.0).clamp(0.0, 255.0).round() as u8;
            temp_img.put_pixel(x - tile_px0, y - tile_py0, image::Luma([value_u8]));
        }

        // If needed, scale the extracted region to exactly 512x512 using nearest-neighbor to avoid smoothing.
        let final_img = if w == 512 && h == 512 {
            temp_img
        } else {
            image::imageops::resize(&temp_img, 512, 512, image::imageops::FilterType::Nearest)
        };

        println!("Final tile dimensions: {:?}", final_img.dimensions());
        Ok(final_img)
    }
}

pub async fn test_get_one() {
    let xyz_tile = XYZTile { x: 0, y: 0, z: 0 };
    let image = xyz_tile
        .get_one("maize_pcr-globwb_gfdl-esm2m_rcp26_wf_2050.tif")
        .await
        .unwrap();
    let filename = "output.png";
    image.save(filename).expect("Failed to save image");
    println!("Image saved as: {}", filename);
}
