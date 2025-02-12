use crate::s3;
use anyhow::Result;
use georaster::{
    geotiff::{GeoTiffReader, RasterValue},
    Coordinate,
};
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

impl XYZTile {
    pub async fn get_one(&self, filename: &str) -> Result<ImageBuffer<image::Luma<u8>, Vec<u8>>> {
        // Gets a file from the S3 bucket and returns image data.
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
            Err(e) => {
                return Err(e);
            }
        };

        println!("XYZ: {:?}", self);
        let bounds: BoundingBox = self.into();
        println!("Bbox: {:?}", bounds);

        let cursor = std::io::Cursor::new(data);
        let mut dataset = GeoTiffReader::open(cursor).expect("Failed to open GeoTiff");

        // Print image pixel dimensions and corners
        if let Some((img_width, img_height)) = dataset.image_info().dimensions {
            println!(
                "Image pixel dimensions: {} x {} | corners: top-left: (0, 0), bottom-right: ({}, {})",
                img_width, img_height, img_width, img_height
            );
        } else {
            println!("Image dimensions not available.");
        }

        // Convert the tile's geographic bounds (from TMS) to pixel coordinates.
        // (Assuming that the tile_grid BoundingBox fields match: left, bottom, right, top)
        let tile_top_left_geo = Coordinate {
            x: bounds.left,
            y: bounds.top,
        };
        let tile_bottom_right_geo = Coordinate {
            x: bounds.right,
            y: bounds.bottom,
        };

        if let (Some((tile_px0, tile_py0)), Some((tile_px1, tile_py1))) = (
            dataset.coord_to_pixel(tile_top_left_geo),
            dataset.coord_to_pixel(tile_bottom_right_geo),
        ) {
            println!("Tile geographic bounds: {:?}", bounds);
            println!(
                "Tile top-left pixel coordinate: ({}, {}) | ({}, {})",
                tile_px0, tile_py0, tile_px1, tile_py1
            );

            let (x0, y0, w, h) = (
                tile_px0,
                tile_py0,
                (tile_px1 - tile_px0),
                (tile_py1 - tile_py0),
            );

            let mut img = ImageBuffer::new(w, h);
            for (x, y, pixel) in dataset.pixels(x0, y0, w, h) {
                // Normalize the pixel value based on its type.
                let norm = match pixel {
                    RasterValue::U8(v) => v as f32 / 255.0,
                    RasterValue::U16(v) => v as f32 / 65535.0,
                    RasterValue::I16(v) => (v as f32 + 32768.0) / 65535.0,
                    RasterValue::F32(v) => v,
                    RasterValue::F64(v) => v as f32 / 65535.0,
                    _ => 0.0, // Fallback for any other variants.
                };

                // Scale to the 0-255 range.
                let value_u8 = (norm * 255.0).clamp(0.0, 255.0).round() as u8;

                // Set transparency for zero values.
                let pixel_value = if value_u8 == 0 {
                    image::Luma([0])
                } else {
                    image::Luma([value_u8])
                };

                // Store the converted pixel in the image buffer.
                img.put_pixel(x - x0, y - y0, pixel_value);
            }

            println!(
                "Image stats: {:?} {:?} {:?}",
                dataset.geo_params,
                img.dimensions(),
                dataset.image_info(),
            );
            Ok(img)
        } else {
            Err(anyhow::anyhow!(
                "Could not convert tile geographic coordinates to pixel coordinates."
            ))
        }
    }
}

pub async fn test_get_one() {
    // let x = 136;
    // let y = 91;
    // let z = 8;
    // let (x, y, z) = (0, 0, 0);
    let xyz_tile = XYZTile { x: 0, y: 0, z: 0 };

    let image = xyz_tile
        .get_one("maize_pcr-globwb_gfdl-esm2m_rcp26_wf_2050.tif")
        .await
        .unwrap();

    let filename = "output.png";
    image.save(filename).expect("Failed to save image");
    println!("Image saved as: {}", filename);
}
