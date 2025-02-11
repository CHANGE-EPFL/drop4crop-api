pub mod s3;
pub mod tiles;

use georaster;
use georaster::{
    geotiff::{GeoTiffReader, RasterValue},
    Coordinate,
};
use image::ImageBuffer;
use tiles::{BoundingBox, XYZTile};

#[tokio::main]
async fn main() {
    // Set S3 credentials

    let prefix = "drop4crop-dev";
    let filename = "maize_pcr-globwb_gfdl-esm2m_rcp26_wf_2050.tif";
    let object = s3::get_object(prefix, filename).await;

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
            eprintln!("Failed to get object: {:?}", e);
            return;
        }
    };

    // Set XYZ coordinates
    // let (z, x, y): (u32, u32, u32) = (8, 136, 91);
    let xyz_tile = XYZTile {
        x: 136,
        y: 91,
        z: 8,
    };
    println!("XYZ: {:?}", xyz_tile);
    let bounds: BoundingBox = xyz_tile.into();
    // let bounds = tile_to_bbox(z, x, y);
    println!("Bbox: {:?}", bounds);

    // let mut datset = geotiff::GeoTiff::read("in-memory").expect("Failed to read dataset");
    let cursor = std::io::Cursor::new(data);
    let mut dataset = GeoTiffReader::open(cursor).expect("Failed to open GeoTiff");

    if let Some((width, height)) = dataset.image_info().dimensions {
        if let (Some(origin), Some(pixel_size)) = (dataset.origin(), dataset.pixel_size()) {
            // Assume origin is the top-left coordinate of the image.
            let left = origin[0];
            let top = origin[1];
            let right = origin[0] + pixel_size[0] * (width as f64);
            let bottom = origin[1] - pixel_size[1] * (height as f64);

            // Construct the bounding box using the correct field names.
            let image_bounds = BoundingBox {
                left,
                bottom,
                right,
                top,
            };

            println!("Image bounds: {:?}", image_bounds);
        } else {
            eprintln!("Missing georeference information (origin or pixel size)");
        }
    } else {
        eprintln!("Image dimensions not available");
    }

    // Print image pixel dimensions and corners
    if let Some((img_width, img_height)) = dataset.image_info().dimensions {
        println!(
            "Image pixel dimensions: {} x {} | corners: top-left: (0, 0), bottom-right: ({}, {})",
            img_width, img_height, img_width, img_height
        );
        // println!(
        //     "Image pixel corners: top-left: (0, 0), bottom-right: ({}, {})",
        //     img_width, img_height
        // );
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

    // println!("Tile geographic bounds: {:?}", bounds);
    // let w: u32;
    // let h: u32;
    // let x0: u32;
    // let y0: u32;

    if let (Some((tile_px0, tile_py0)), Some((tile_px1, tile_py1))) = (
        dataset.coord_to_pixel(tile_top_left_geo),
        dataset.coord_to_pixel(tile_bottom_right_geo),
    ) {
        println!("Tile geographic bounds: {:?}", bounds);
        println!(
            "Tile top-left pixel coordinate: ({}, {}) | ({}, {})",
            tile_px0, tile_py0, tile_px1, tile_py1
        );
        // println!(
        // "Tile bottom-right pixel coordinate: ({}, {})",
        // tile_px1, tile_py1
        // );
        let x0 = tile_px0;
        let y0 = tile_py0;
        let w = tile_px1 - tile_px0;
        let h = tile_py1 - tile_py0;

        // (x0, y0) = (tile_px0, tile_py0);
        // (w, h) = (tile_px1 - tile_px0, tile_py1 - tile_py0);
        let mut img = ImageBuffer::new(w, h);
        for (x, y, pixel) in dataset.pixels(x0, y0, w, h) {
            println!("x: {}, y: {}, pixel: {:?}", x, y, pixel);
            if let RasterValue::U16(v) = pixel {
                img.put_pixel(x - x0, y - y0, image::Luma([v]));
            }
        }

        // Find the minimum and maximum of the pixels in the image.
        let (min, max) = img.pixels().fold((u16::MAX, u16::MIN), |(min, max), p| {
            let v = p[0];
            (min.min(v), max.max(v))
        });
        println!("Image pixel range: {} - {}", min, max);
        println!(
            "Image stats: {:?} {:?}",
            dataset.geo_params,
            img.dimensions()
        );

        img.save("output.png").expect("Failed to save image");
    } else {
        println!("Could not convert tile geographic coordinates to pixel coordinates.");
    }
}
