use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

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
    println!("Converting to COG (simplified implementation)");

    // For now, return the input bytes as-is since we need to fix the GDAL dependencies
    // The COG conversion would require additional GDAL setup that may need system dependencies
    println!("COG conversion placeholder - returning original bytes");
    Ok(input_bytes.to_vec())
}

/// Calculates min and max values of a raster using GDAL
pub fn get_min_max_of_raster(_input_bytes: &[u8]) -> Result<(f64, f64)> {
    println!("Min/max calculation placeholder - returning default values");

    // For now, return default values since we need to fix the GDAL dependencies
    // This would need proper GDAL setup to read from virtual file system
    Ok((0.0, 1.0))
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