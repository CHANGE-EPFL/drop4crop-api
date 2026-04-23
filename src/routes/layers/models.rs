use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Deserialize, IntoParams)]
pub struct UploadQueryParams {
    pub overwrite_duplicates: Option<bool>,
    /// Optional project UUID to associate the uploaded layer with a project
    pub project_id: Option<Uuid>,
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

/// Represents the parsed components of a climate layer filename.
///
/// The 6-part canonical order `{crop}_{water_model}_{climate_model}_{scenario}_{variable}_{year}.tif`
/// is fixed — projects that don't use a given axis write the sentinel `null` or `nan`
/// (both accepted, case-insensitive) in that slot. Keeping the positions stable means
/// automation that renames / generates files doesn't need to know the project's config.
/// `water_model`, `climate_model`, `scenario`, and `variable` are all optional for this reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateLayerInfo {
    pub crop: String,
    pub water_model: Option<String>,
    pub climate_model: Option<String>,
    pub scenario: Option<String>,
    pub variable: Option<String>,
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

/// Structured error body returned by the upload endpoint.
///
/// `code` is machine-readable — the frontend uses it to route the user into the
/// resolution panel. `message` stays human-readable so existing clients keep
/// working. `field` + `slug` let the frontend offer a targeted "create & attach"
/// or "attach" action without having to parse the message.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UploadError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl UploadError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            field: None,
            slug: None,
            message: message.into(),
            error: None,
        }
    }

    pub fn with_slug(code: &str, field: &str, slug: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            field: Some(field.to_string()),
            slug: Some(slug.to_string()),
            message: message.into(),
            error: None,
        }
    }

    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }
}
