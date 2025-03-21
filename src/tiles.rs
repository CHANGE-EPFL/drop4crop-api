use crate::storage;
use anyhow::{Context, Result};
use gdal::{spatial_ref::SpatialRef, Dataset};
use gdal_sys::{CPLErr::CE_None, GDALResampleAlg::GRA_NearestNeighbour};
use image::ImageBuffer;
use image::Luma;
use std::ffi::CString;
use tokio::task;

#[derive(Debug)]
pub struct XYZTile {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// Represents the tile’s bounds in Web Mercator (EPSG:3857)
pub struct WebMercatorTileBounds {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

/// Computes the Web Mercator bounds for a given XYZ tile.
/// These formulas use the standard XYZ tile scheme where the Web Mercator world extent is from
/// -20037508.342789244 to 20037508.342789244.
fn compute_web_mercator_bounds(tile: &XYZTile) -> WebMercatorTileBounds {
    let tile_size = 256.0;
    let initial_resolution = 2.0 * 20037508.342789244 / tile_size; // resolution at zoom 0
    let resolution = initial_resolution / (2f64.powi(tile.z as i32));
    let min_x = (tile.x as f64 * tile_size * resolution) - 20037508.342789244;
    let max_y = 20037508.342789244 - (tile.y as f64 * tile_size * resolution);
    let max_x = ((tile.x as f64 + 1.0) * tile_size * resolution) - 20037508.342789244;
    let min_y = 20037508.342789244 - ((tile.y as f64 + 1.0) * tile_size * resolution);
    WebMercatorTileBounds {
        min_x,
        max_x,
        min_y,
        max_y,
    }
}

impl XYZTile {
    /// Retrieves a tile image as a 256x256 grayscale ImageBuffer.
    ///
    /// The function first fetches the GeoTIFF data from S3 (in EPSG:4326), then uses GDAL to
    /// reproject it to Web Mercator (EPSG:3857) for correct alignment with basemaps like OSM.
    /// Heavy GDAL operations run in a blocking thread.
    pub async fn get_one(&self, layer_id: &str) -> Result<ImageBuffer<Luma<u8>, Vec<u8>>> {
        // Fetch the TIFF bytes from S3 asynchronously.
        let filename = format!("{}.tif", layer_id);
        let object = storage::get_object(&filename).await?;
        let x = self.x;
        let y = self.y;
        let z = self.z;
        let tile = XYZTile { x, y, z };

        // Offload the heavy reprojection to a blocking task.
        let img = task::spawn_blocking(move || -> Result<ImageBuffer<Luma<u8>, Vec<u8>>> {
            // println!("Generating tile for x: {}, y: {}, z: {}", x, y, z);

            // Compute the expected Web Mercator bounds for this tile.
            let bounds = compute_web_mercator_bounds(&tile);

            // Write the in-memory TIFF to GDAL’s /vsimem virtual filesystem.
            let vsi_path = format!("/vsimem/{}", filename);
            let vsi_path = vsi_path.as_str();
            {
                let c_vsi_path = CString::new(vsi_path).unwrap();
                let mode = CString::new("w").unwrap();
                unsafe {
                    let fp = gdal_sys::VSIFOpenL(c_vsi_path.as_ptr(), mode.as_ptr());
                    if fp.is_null() {
                        return Err(anyhow::anyhow!("Failed to open /vsimem file"));
                    }
                    let written =
                        gdal_sys::VSIFWriteL(object.as_ptr() as *const _, 1, object.len(), fp);
                    if written != object.len() {
                        gdal_sys::VSIFCloseL(fp);
                        return Err(anyhow::anyhow!("Failed to write all data to /vsimem file"));
                    }
                    gdal_sys::VSIFCloseL(fp);
                }
            }

            // Open the dataset from /vsimem and clean up the virtual file.
            let src_ds = Dataset::open(vsi_path).context("Opening dataset from /vsimem")?;
            {
                let c_vsi_path = CString::new(vsi_path).unwrap();
                unsafe {
                    gdal_sys::VSIUnlink(c_vsi_path.as_ptr());
                }
            }

            // Define the destination projection (EPSG:3857) - Web mercator
            let dst_srs =
                SpatialRef::from_epsg(3857).context("Creating destination spatial reference")?;

            // Create an in-memory destination dataset using the MEM driver.
            let mem_driver =
                gdal::DriverManager::get_driver_by_name("MEM").context("Getting MEM driver")?;
            let band_count = src_ds.raster_count();
            let mut dest_ds = mem_driver
                .create_with_band_type::<u8, _>("", 256, 256, band_count as usize)
                .context("Creating destination dataset")?;

            // Set the destination projection to EPSG:3857.
            dest_ds.set_projection(&dst_srs.to_wkt()?)?;

            // Compute pixel resolutions based on the tile bounds.
            let pixel_width = (bounds.max_x - bounds.min_x) / 256.0;
            let pixel_height = (bounds.min_y - bounds.max_y) / 256.0;
            // Set the geo-transform:
            // Origin is the top-left corner (min_x, max_y).
            dest_ds
                .set_geo_transform(&[
                    bounds.min_x,
                    pixel_width,
                    0.0,
                    bounds.max_y,
                    0.0,
                    pixel_height,
                ])
                .context("Setting geo-transform for destination")?;

            // Perform the reprojection (warp) from the source dataset to the destination.
            let err = unsafe {
                gdal_sys::GDALReprojectImage(
                    src_ds.c_dataset(),
                    std::ptr::null(),
                    dest_ds.c_dataset(),
                    std::ptr::null(),
                    GRA_NearestNeighbour,
                    0.0,
                    0.0,
                    None,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert!(err == CE_None, "GDAL warp failed with error code {}", err);

            // Read the warped data from the first band (assuming a single-band TIFF).
            let band = match dest_ds.rasterband(1) {
                Ok(band) => band,
                Err(e) => {
                    println!("Error getting raster band 1: {:?}", e);
                    return Err(anyhow::Error::new(e).context("Error getting raster band 1"));
                }
            };
            let buf = band
                .read_as::<u8>((0, 0), (256, 256), (256, 256), None)
                .context("Reading raster data")?;
            let buffer: Vec<u8> = buf.data().to_vec();
            let img = ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(256, 256, buffer)
                .ok_or_else(|| anyhow::anyhow!("Failed to create image buffer"))?;
            Ok(img)
        })
        .await??;
        Ok(img)
    }
}
