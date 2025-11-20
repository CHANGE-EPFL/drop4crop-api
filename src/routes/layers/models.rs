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