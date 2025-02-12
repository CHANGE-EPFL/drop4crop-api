use crate::tiles::XYZTile;
use anyhow::Result;
use axum::{
    extract::{Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
};
use image::codecs::png::PngEncoder;
use image::ImageEncoder;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Params {
    filename: String,
}

pub async fn tile_handler(
    Query(params): Query<Params>,
    Path((z, x, y)): Path<(u32, u32, u32)>,
) -> Result<impl IntoResponse, StatusCode> {
    // Get the tile as an ImageBuffer.
    let xyz_tile = XYZTile { x, y, z };
    let img = xyz_tile
        .get_one(&params.filename)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Encode the ImageBuffer to PNG.
    let mut png_data = Vec::new();
    {
        let encoder = PngEncoder::new(&mut png_data);
        encoder
            .write_image(
                img.as_raw(),
                img.width(),
                img.height(),
                image::ColorType::L8.into(), // Use L8 for grayscale. Adjust if needed.
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Build the response with a Content-Type header.
    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);

    Ok(response)
}
