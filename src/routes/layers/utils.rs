use anyhow::{anyhow, Result};
use gdal::cpl::CslStringList;
use gdal::raster::RasterBand;
use gdal::{Dataset, DriverManager};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::config::Config;

/// Represents the parsed components of a climate layer filename
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateLayerInfo {
    pub crop: String,
    pub water_model: String,
    pub climate_model: String,
    pub scenario: String,
    pub variable: String,
    pub year: i32,
}

/// Represents the parsed components of a crop layer filename
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropLayerInfo {
    pub crop: String,
    pub variable: String,
}

/// Represents the parsed information from a layer filename
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerInfo {
    Climate(ClimateLayerInfo),
    Crop(CropLayerInfo),
}

/// Parses a filename to extract layer information
pub fn parse_filename(filename: &str) -> Result<LayerInfo> {
    // Remove file extension and convert to lowercase
    let filename_lower = filename.to_lowercase();
    let name_without_ext = filename_lower
        .strip_suffix(".tif")
        .ok_or_else(|| anyhow!("Filename must end with .tif"))?;

    let parts: Vec<&str> = name_without_ext.split('_').collect();

    let config = Config::from_env();

    match parts.len() {
        6 => {
            // Climate layer: crop_watermodel_climatemodel_scenario_variable_year
            Ok(LayerInfo::Climate(ClimateLayerInfo {
                crop: parts[0].to_string(),
                water_model: parts[1].to_string(),
                climate_model: parts[2].to_string(),
                scenario: parts[3].to_string(),
                variable: parts[4].to_string(),
                year: parts[5].parse()
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
                year: parts[6].parse()
                    .map_err(|_| anyhow!("Invalid year in filename: {}", parts[6]))?,
            }))
        }
        2..=6 => {
            // Crop layer: crop_variable (variable can contain underscores)
            let crop = parts[0].to_string();
            let variable = parts[1..].join("_");

            // Validate that the variable is in the list of crop variables
            if config.crop_variables.contains(&variable) {
                Ok(LayerInfo::Crop(CropLayerInfo {
                    crop,
                    variable,
                }))
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
    println!("Converting to COG format using GDAL");

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
    let mut cog_dataset = dataset.create_copy(&driver, output_path.to_str().unwrap(), &creation_options)?;

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

    println!("COG conversion completed successfully");
    Ok(output_bytes)
}

/// Calculates min and max values of a raster using GDAL
pub fn get_min_max_of_raster(input_bytes: &[u8]) -> Result<(f64, f64)> {
    println!("Calculating raster min/max values using GDAL");

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

    println!(
        "Min/max calculation completed: min={}, max={}",
        stats.min, stats.max
    );

    Ok((stats.min, stats.max))
}

/// Calculates the global average (mean) value of a raster using GDAL
pub fn get_global_average_of_raster(input_bytes: &[u8]) -> Result<f64> {
    println!("Calculating raster global average using GDAL");

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
    let stats = rasterband.get_statistics(true, false)?.ok_or_else(|| {
        anyhow!("Failed to compute raster statistics")
    })?;
    let mean = stats.mean;

    // Clean up temporary file
    let _ = fs::remove_file(&input_path);

    println!("Global average calculation completed: mean={}", mean);

    Ok(mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_climate_filename() {
        let result = parse_filename("wheat_lpjml_gfdl-esm4_historical_yield_2020.tif").unwrap();

        match result {
            LayerInfo::Climate(info) => {
                assert_eq!(info.crop, "wheat");
                assert_eq!(info.water_model, "lpjml");
                assert_eq!(info.climate_model, "gfdl-esm4");
                assert_eq!(info.scenario, "historical");
                assert_eq!(info.variable, "yield");
                assert_eq!(info.year, 2020);
            }
            _ => panic!("Expected climate layer info"),
        }
    }

    #[test]
    fn test_parse_crop_filename() {
        let result = parse_filename("soy_mirca_area_total.tif").unwrap();

        match result {
            LayerInfo::Crop(info) => {
                assert_eq!(info.crop, "soy");
                assert_eq!(info.variable, "mirca_area_total");
            }
            _ => panic!("Expected crop layer info"),
        }
    }

    #[test]
    fn test_parse_percentage_filename() {
        let result = parse_filename("rice_lpjml_gfdl-esm4_historical_yield_perc_2020.tif").unwrap();

        match result {
            LayerInfo::Climate(info) => {
                assert_eq!(info.crop, "rice");
                assert_eq!(info.variable, "yield_perc");
                assert_eq!(info.year, 2020);
            }
            _ => panic!("Expected climate layer info"),
        }
    }

    #[test]
    fn test_parse_invalid_filename() {
        let result = parse_filename("invalid.txt");
        assert!(result.is_err());
    }
}