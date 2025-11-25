use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Deserialize, IntoParams)]
pub struct UploadQueryParams {
    pub overwrite_duplicates: Option<bool>,
}


#[derive(Deserialize, ToSchema, IntoParams)]
pub struct GetPixelValueParams {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize, ToSchema, IntoParams)]
pub struct PixelValueResponse {
    pub value: f64,
}
#[derive(Deserialize, IntoParams)]
pub struct DownloadQueryParams {
    pub minx: Option<f64>,
    pub miny: Option<f64>,
    pub maxx: Option<f64>,
    pub maxy: Option<f64>,
}

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
