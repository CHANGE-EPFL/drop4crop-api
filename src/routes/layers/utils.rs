use crate::config::Config;
use anyhow::{Result, anyhow};
use gdal::{
    cpl::CslStringList,
    raster::RasterBand,
    {Dataset, DriverManager},
};
use std::{ffi::CString, vec::Vec, fs};
use tracing::{debug, info};
use super::models::{ClimateLayerInfo, CropLayerInfo, LayerInfo};

/// Parses a filename to extract layer information
pub fn parse_filename(config: &Config, filename: &str) -> Result<LayerInfo> {
    // Remove file extension and convert to lowercase
    let filename_lower = filename.to_lowercase();
    let name_without_ext = filename_lower
        .strip_suffix(".tif")
        .ok_or_else(|| anyhow!("Filename must end with .tif"))?;

    let parts: Vec<&str> = name_without_ext.split('_').collect();

    match parts.len() {
        6 => {
            // Climate layer: crop_watermodel_climatemodel_scenario_variable_year
            Ok(LayerInfo::Climate(ClimateLayerInfo {
                crop: parts[0].to_string(),
                water_model: parts[1].to_string(),
                climate_model: parts[2].to_string(),
                scenario: parts[3].to_string(),
                variable: parts[4].to_string(),
                year: parts[5]
                    .parse()
                    .map_err(|_| anyhow!("Invalid year in filename: {}", parts[5]))?,
            }))
        }
        7 => {
            // Climate layer with percentage unit: crop_watermodel_climatemodel_scenario_variable_unit_year
            let unit = parts[5];
            let variable = if unit == "perc" {
                format!("{}_perc", parts[4])
            } else {
                return Err(anyhow!("Unsupported unit in filename: {}", unit));
            };

            Ok(LayerInfo::Climate(ClimateLayerInfo {
                crop: parts[0].to_string(),
                water_model: parts[1].to_string(),
                climate_model: parts[2].to_string(),
                scenario: parts[3].to_string(),
                variable,
                year: parts[6]
                    .parse()
                    .map_err(|_| anyhow!("Invalid year in filename: {}", parts[6]))?,
            }))
        }
        2..=6 => {
            // Crop layer: crop_variable (variable can contain underscores)
            let crop = parts[0].to_string();
            let variable = parts[1..].join("_");

            // Validate that the variable is in the list of crop variables
            if config.crop_variables.contains(&variable) {
                Ok(LayerInfo::Crop(CropLayerInfo { crop, variable }))
            } else {
                Err(anyhow!(
                    "Invalid crop variable '{}'. Must be one of: {:?}",
                    variable,
                    config.crop_variables
                ))
            }
        }
        _ => Err(anyhow!(
            "Invalid filename format. Expected either {{crop}}_{{watermodel}}_{{climatemodel}}_{{scenario}}_{{variable}}_{{year}}.tif or {{crop}}_{{crop_variable}}.tif"
        )),
    }
}

/// Converts a GeoTIFF to Cloud Optimized GeoTIFF format in memory
pub fn convert_to_cog_in_memory(input_bytes: &[u8]) -> Result<Vec<u8>> {
    debug!("Converting to COG format using GDAL");

    // Use temporary files since GDAL Rust bindings don't expose VSI write/read functions
    let temp_dir = std::env::temp_dir();
    let input_path = temp_dir.join(format!("input_{}.tif", std::process::id()));
    let output_path = temp_dir.join(format!("output_{}.tif", std::process::id()));

    // Write input bytes to temporary file
    fs::write(&input_path, input_bytes)?;

    // Open input dataset
    let dataset = Dataset::open(&input_path)?;

    // Get GeoTIFF driver
    let driver = DriverManager::get_driver_by_name("GTiff")?;

    // COG creation options
    let mut creation_options = CslStringList::new();
    creation_options.add_string("TILED=YES")?;
    creation_options.add_string("COPY_SRC_OVERVIEWS=YES")?;
    creation_options.add_string("COMPRESS=LZW")?;
    creation_options.add_string("PREDICTOR=2")?;
    creation_options.add_string("BLOCKXSIZE=512")?;
    creation_options.add_string("BLOCKYSIZE=512")?;

    // Create COG with proper options
    let mut cog_dataset =
        dataset.create_copy(&driver, output_path.to_str().unwrap(), &creation_options)?;

    // Build overviews for the COG
    let overview_list = &[2, 4, 8, 16];
    cog_dataset.build_overviews("NEAREST", overview_list, &[])?;

    // Close datasets to flush to disk
    drop(cog_dataset);
    drop(dataset);

    // Read output from file
    let output_bytes = fs::read(&output_path)?;

    // Clean up temporary files
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);

    info!("COG conversion completed successfully");
    Ok(output_bytes)
}

/// Calculates min and max values of a raster using GDAL
pub fn get_min_max_of_raster(input_bytes: &[u8]) -> Result<(f64, f64)> {
    debug!("Calculating raster min/max values using GDAL");

    // Use temporary file since GDAL Rust bindings don't expose VSI write/read functions
    let temp_dir = std::env::temp_dir();
    let input_path = temp_dir.join(format!("minmax_{}.tif", std::process::id()));

    // Write input bytes to temporary file
    fs::write(&input_path, input_bytes)?;

    // Open dataset
    let dataset = Dataset::open(&input_path)?;

    // Get the first raster band (band index 1)
    let rasterband: RasterBand = dataset.rasterband(1)?;

    // Compute statistics (this calculates min, max, mean, stddev)
    let stats = rasterband.compute_raster_min_max(true)?;

    // Clean up temporary file
    let _ = fs::remove_file(&input_path);

    debug!(
        min = stats.min,
        max = stats.max,
        "Min/max calculation completed"
    );

    Ok((stats.min, stats.max))
}

/// Calculates the global average (mean) value of a raster using GDAL
pub fn get_global_average_of_raster(input_bytes: &[u8]) -> Result<f64> {
    debug!("Calculating raster global average using GDAL");

    // Use temporary file since GDAL Rust bindings don't expose VSI write/read functions
    let temp_dir = std::env::temp_dir();
    let input_path = temp_dir.join(format!("avg_{}.tif", std::process::id()));

    // Write input bytes to temporary file
    fs::write(&input_path, input_bytes)?;

    // Open dataset
    let dataset = Dataset::open(&input_path)?;

    // Get the first raster band (band index 1)
    let rasterband: RasterBand = dataset.rasterband(1)?;

    // Get raster statistics which includes mean
    // force=true means it will compute if not already cached, approx=false means exact calculation
    let stats = rasterband
        .get_statistics(true, false)?
        .ok_or_else(|| anyhow!("Failed to compute raster statistics"))?;
    let mean = stats.mean;

    // Clean up temporary file
    let _ = fs::remove_file(&input_path);

    debug!(mean, "Global average calculation completed");

    Ok(mean)
}

/// Crops a GeoTIFF to the specified bounding box
/// Returns the cropped GeoTIFF as bytes
pub fn crop_to_bbox(
    original_data: &[u8],
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
) -> Result<Vec<u8>, String> {
    use gdal::raster::Buffer;

    // Write original data to vsimem
    let input_path = format!("/vsimem/input_{}.tif", uuid::Uuid::new_v4());
    let c_input_path = CString::new(input_path.clone()).map_err(|e| e.to_string())?;

    unsafe {
        let mode = CString::new("w").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_input_path.as_ptr(), mode.as_ptr());
        if fp.is_null() {
            return Err("Failed to open vsimem input file".to_string());
        }
        let written = gdal_sys::VSIFWriteL(
            original_data.as_ptr() as *const _,
            1,
            original_data.len(),
            fp,
        );
        if written != original_data.len() {
            gdal_sys::VSIFCloseL(fp);
            return Err("Failed to write all data to vsimem".to_string());
        }
        gdal_sys::VSIFCloseL(fp);
    }

    // Open the dataset
    let dataset =
        Dataset::open(&input_path).map_err(|e| format!("Failed to open dataset: {}", e))?;

    // Get geotransform
    let gt = dataset
        .geo_transform()
        .map_err(|e| format!("Failed to get geotransform: {}", e))?;

    // Calculate pixel coordinates for the bounding box
    let col_min = ((minx - gt[0]) / gt[1]).floor() as isize;
    let col_max = ((maxx - gt[0]) / gt[1]).ceil() as isize;
    let row_min = ((maxy - gt[3]) / gt[5]).floor() as isize; // gt[5] is typically negative
    let row_max = ((miny - gt[3]) / gt[5]).ceil() as isize;

    let (raster_x_size, raster_y_size) = dataset.raster_size();

    // Clamp to raster bounds
    let col_min = col_min.max(0).min(raster_x_size as isize);
    let col_max = col_max.max(0).min(raster_x_size as isize);
    let row_min = row_min.max(0).min(raster_y_size as isize);
    let row_max = row_max.max(0).min(raster_y_size as isize);

    let width = (col_max - col_min) as usize;
    let height = (row_max - row_min) as usize;

    if width == 0 || height == 0 {
        unsafe {
            gdal_sys::VSIUnlink(c_input_path.as_ptr());
        }
        return Err("Bounding box results in zero-sized raster".to_string());
    }

    // Calculate new geotransform for cropped region
    let new_origin_x = gt[0] + col_min as f64 * gt[1];
    let new_origin_y = gt[3] + row_min as f64 * gt[5];
    let new_gt = [new_origin_x, gt[1], gt[2], new_origin_y, gt[4], gt[5]];

    // Read the cropped data from the band
    let band = dataset
        .rasterband(1)
        .map_err(|e| format!("Failed to get rasterband: {}", e))?;
    let mut buffer: Buffer<f64> = band
        .read_as((col_min, row_min), (width, height), (width, height), None)
        .map_err(|e| format!("Failed to read raster data: {}", e))?;

    // Create output dataset in vsimem
    let output_path = format!("/vsimem/output_{}.tif", uuid::Uuid::new_v4());
    let c_output_path = CString::new(output_path.clone()).map_err(|e| e.to_string())?;

    let driver = gdal::DriverManager::get_driver_by_name("GTiff")
        .map_err(|e| format!("Failed to get GTiff driver: {}", e))?;

    let mut out_dataset = driver
        .create_with_band_type::<f64, _>(&output_path, width, height, 1)
        .map_err(|e| format!("Failed to create output dataset: {}", e))?;

    // Set geotransform and spatial reference
    out_dataset
        .set_geo_transform(&new_gt)
        .map_err(|e| format!("Failed to set geotransform: {}", e))?;

    if let Ok(srs) = dataset.spatial_ref() {
        out_dataset
            .set_spatial_ref(&srs)
            .map_err(|e| format!("Failed to set spatial reference: {}", e))?;
    }

    // Write the data
    let mut out_band = out_dataset
        .rasterband(1)
        .map_err(|e| format!("Failed to get output rasterband: {}", e))?;

    out_band
        .write((0, 0), (width, height), &mut buffer)
        .map_err(|e| format!("Failed to write raster data: {}", e))?;

    // Flush and close
    drop(out_dataset);
    drop(dataset);

    // Read the cropped file from vsimem
    let cropped_data = unsafe {
        let mode = CString::new("r").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_output_path.as_ptr(), mode.as_ptr());
        if fp.is_null() {
            gdal_sys::VSIUnlink(c_input_path.as_ptr());
            return Err("Failed to open output file".to_string());
        }

        // Get file size
        gdal_sys::VSIFSeekL(fp, 0, 2); // SEEK_END
        let size = gdal_sys::VSIFTellL(fp) as usize;
        gdal_sys::VSIFSeekL(fp, 0, 0); // SEEK_SET

        // Read data
        let mut buffer = vec![0u8; size];
        let read = gdal_sys::VSIFReadL(buffer.as_mut_ptr() as *mut _, 1, size, fp);
        if read != size {
            gdal_sys::VSIFCloseL(fp);
            gdal_sys::VSIUnlink(c_input_path.as_ptr());
            gdal_sys::VSIUnlink(c_output_path.as_ptr());
            return Err("Failed to read all cropped data".to_string());
        }
        gdal_sys::VSIFCloseL(fp);

        buffer
    };

    // Clean up vsimem files
    unsafe {
        gdal_sys::VSIUnlink(c_input_path.as_ptr());
        gdal_sys::VSIUnlink(c_output_path.as_ptr());
    }

    Ok(cropped_data)
}
