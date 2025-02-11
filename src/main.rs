mod config;
pub mod s3;
pub mod tiles;
use anyhow::Result;
use georaster;
use georaster::{
    geotiff::{GeoTiffReader, RasterValue},
    Coordinate,
};
use image::ImageBuffer;
use tiles::{BoundingBox, XYZTile};

async fn get_one(
    filename: &str,
    x: u32,
    y: u32,
    z: u32,
) -> Result<ImageBuffer<image::Luma<u8>, Vec<u8>>> {
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

    // Set XYZ coordinates
    let xyz_tile = XYZTile { x, y, z };

    println!("XYZ: {:?}", xyz_tile);
    let bounds: BoundingBox = xyz_tile.into();
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

            // Store the converted pixel in the image buffer.
            img.put_pixel(x - x0, y - y0, image::Luma([value_u8]));
        }

        println!(
            "Image stats: {:?} {:?} {:?}",
            dataset.geo_params,
            img.dimensions(),
            dataset.image_info(),
        );
        Ok(img)
        // img.save("output.png").expect("Failed to save image");
    } else {
        // println!("Could not convert tile geographic coordinates to pixel coordinates.");
        Err(anyhow::anyhow!(
            "Could not convert tile geographic coordinates to pixel coordinates."
        ))
    }
}
#[tokio::main]
async fn main() {
    let x = 136;
    let y = 91;
    let z = 8;
    let (x, y, z) = (0, 0, 0);

    let image = get_one("maize_pcr-globwb_gfdl-esm2m_rcp26_wf_2050.tif", x, y, z)
        .await
        .unwrap();

    let filename = "output.png";
    image.save(filename).expect("Failed to save image");
    println!("Image saved as: {}", filename);
}
