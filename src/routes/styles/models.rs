use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, ToSchema)]
pub struct StyleItem {
    pub value: f64,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: u8,
    pub label: f64,
}

impl StyleItem {
    // Takes a JSON that is typically stored in the postgres db but rendered
    // as a serde_json::Value, sorts it and returns a Vec<StyleItem>, if the
    // JSON is empty, it generates a grayscale style based on the minimum and maximum
    // raster values of the layer which are passed in as parameters.
    pub fn from_json(
        json: &serde_json::Value,
        layer_min: f64,
        layer_max: f64,
        num_segments: usize,
    ) -> Vec<StyleItem> {
        let json_array = match json.as_array() {
            Some(array) => array,
            None => &vec![],
        };
        let mut style = vec![];

        if json_array.is_empty() {
            Self::generate_grayscale_style(layer_min, layer_max, num_segments)
        } else {
            for item in json_array {
                if let Some(value) = item.get("value")
                    && let Some(value) = value.as_f64() {
                        let red = item.get("red").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        let green = item.get("green").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        let blue = item.get("blue").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        let opacity =
                            item.get("opacity").and_then(|v| v.as_u64()).unwrap_or(255) as u8;
                        let label = item.get("label").and_then(|v| v.as_f64()).unwrap_or(value);

                        style.push(StyleItem {
                            value,
                            red,
                            green,
                            blue,
                            opacity,
                            label,
                        });
                    }
            }
            Self::sort_styles(style)
        }
    }

    pub fn sort_styles(mut style_list: Vec<StyleItem>) -> Vec<StyleItem> {
        style_list.sort_by(|a, b| {
            a.value
                .partial_cmp(&b.value)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        style_list
    }

    pub fn generate_grayscale_style(min: f64, max: f64, num_segments: usize) -> Vec<StyleItem> {
        let step = (max - min) / num_segments as f64;
        let mut style = Vec::with_capacity(num_segments);

        for i in 0..num_segments {
            let value = min + i as f64 * step;
            let grey_value =
                ((255.0 * i as f64) / (num_segments.saturating_sub(1) as f64)).round() as u8;
            style.push(crate::routes::styles::models::StyleItem {
                value,
                red: grey_value,
                green: grey_value,
                blue: grey_value,
                opacity: 255,
                label: (value * 10000.0).round() / 10000.0, // round to 4 decimal places
            });
        }

        style
    }
}
