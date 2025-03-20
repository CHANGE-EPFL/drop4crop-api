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
use tokio_retry::strategy::FixedInterval;
use tokio_retry::RetryIf;

#[derive(Deserialize)]
pub struct Params {
    layer: String,
}

pub async fn tile_handler(
    Query(params): Query<Params>,
    Path((z, x, y)): Path<(u32, u32, u32)>,
) -> Result<impl IntoResponse, StatusCode> {
    // Get the tile as an ImageBuffer.
    // println!("[tile_handler] z: {}, x: {}, y: {}", z, x, y);
    let xyz_tile = XYZTile { x, y, z };
    // let temp_filename = "wheat_production.tif";
    let retry_strategy = FixedInterval::from_millis(200).take(5);

    let img = RetryIf::spawn(
        retry_strategy,
        || xyz_tile.get_one(&params.layer),
        |_: &anyhow::Error| {
            println!("[tile_handler] Error: x: {}, y: {}, z: {}", x, y, z);
            true
        },
    )
    .await
    .map_err(|e| {
        println!("[tile_handler] Failed after 5 attempts: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

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
