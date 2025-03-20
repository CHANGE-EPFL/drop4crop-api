use crate::tiles::XYZTile;
use anyhow::Result;
use axum::{
    extract::{Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
};
use image::codecs::png::PngEncoder;
use image::ImageEncoder;
use image::{ImageBuffer, Rgba};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Params {
    layer: String,
}

pub async fn tile_handler(
    Query(params): Query<Params>,
    Path((z, x, y)): Path<(u32, u32, u32)>,
) -> Result<impl IntoResponse, StatusCode> {
    // Get the tile as an ImageBuffer.
    println!("[tile_handler] z: {}, x: {}, y: {}", z, x, y);
    let xyz_tile = XYZTile { x, y, z };
    // let temp_filename = "wheat_production.tif";
    let img = match xyz_tile.get_one(&params.layer).await {
        Ok(img) => img,
        Err(e) => {
            println!("[tile_handler] Error on first attempt, trying again");
            match xyz_tile.get_one(&params.layer).await {
                Ok(img) => {
                    println!("[tile_handler] Successfully retrieved image on second attempt");
                    img
                }
                Err(e) => {
                    println!("[tile_handler] Error on second attempt: {:?}", e);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
        }
    };

    // Convert the grayscale ImageBuffer to RGBA.
    let (width, height) = img.dimensions();
    let img_rgba: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_fn(width, height, |x, y| {
        let p = img.get_pixel(x, y)[0];
        if p == 0 {
            Rgba([0, 0, 0, 0])
        } else {
            Rgba([p, p, p, 255])
        }
    });

    // Encode the ImageBuffer to PNG.
    let mut png_data = Vec::new();
    {
        let encoder = PngEncoder::new(&mut png_data);
        encoder
            .write_image(
                img_rgba.as_raw(),
                img_rgba.width(),
                img_rgba.height(),
                image::ColorType::Rgba8.into(),
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Build the response with a Content-Type header.
    // println!("[tile_handler] Response size: {} bytes", png_data.len());
    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);
    // println!("[tile_handler] Response size: {} bytes", png_data.len());
    Ok(response)
}
