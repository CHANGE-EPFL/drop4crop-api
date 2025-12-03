use serde::{Deserialize, Serialize};
use anyhow::{Result, anyhow};
use crate::routes::tiles::styling::ColorStop;

/// Request body for importing a QGIS color map
#[derive(Debug, Deserialize, Serialize)]
pub struct QgisImportRequest {
    /// Name for the new style
    pub name: String,
    /// Raw QGIS color map content
    pub qgis_content: String,
}

/// Response from QGIS import
#[derive(Debug, Serialize)]
pub struct QgisImportResponse {
    pub style: Vec<ColorStop>,
    pub interpolation_type: String,
}

/// Parses a QGIS color map export file content into ColorStop array
///
/// Example QGIS format:
/// ```text
/// INTERPOLATION:DISCRETE
/// 0.1,49,54,149,105,<= 0.1
/// 0.22,69,117,180,255,0.1 - 0.2
/// ...
/// ```
pub fn parse_qgis_colormap(content: &str) -> Result<(Vec<ColorStop>, String)> {
    let mut stops: Vec<ColorStop> = Vec::new();
    let mut interpolation_type = "linear".to_string();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines
        if line.is_empty() {
            continue;
        }

        // Check for interpolation header
        if line.starts_with("INTERPOLATION:") {
            let interp = line.strip_prefix("INTERPOLATION:").unwrap_or("LINEAR").to_uppercase();
            interpolation_type = if interp == "DISCRETE" {
                "discrete".to_string()
            } else {
                "linear".to_string()
            };
            continue;
        }

        // Skip comment lines
        if line.starts_with('#') || line.starts_with("nan") || line.starts_with("nv") {
            continue;
        }

        // Parse color stop line: value,R,G,B,A,label
        let parts: Vec<&str> = line.splitn(6, ',').collect();
        if parts.len() >= 5 {
            let value = parts[0].parse::<f32>()
                .map_err(|e| anyhow!("Invalid value '{}': {}", parts[0], e))?;
            let red = parts[1].parse::<u8>()
                .map_err(|e| anyhow!("Invalid red '{}': {}", parts[1], e))?;
            let green = parts[2].parse::<u8>()
                .map_err(|e| anyhow!("Invalid green '{}': {}", parts[2], e))?;
            let blue = parts[3].parse::<u8>()
                .map_err(|e| anyhow!("Invalid blue '{}': {}", parts[3], e))?;
            let opacity = parts[4].parse::<u8>()
                .map_err(|e| anyhow!("Invalid opacity '{}': {}", parts[4], e))?;

            // Label is optional (6th field)
            let label = if parts.len() >= 6 {
                Some(parts[5].trim().to_string())
            } else {
                None
            };

            stops.push(ColorStop {
                value,
                red,
                green,
                blue,
                opacity,
                label,
            });
        }
    }

    if stops.is_empty() {
        return Err(anyhow!("No valid color stops found in QGIS content"));
    }

    // Sort stops by value
    stops.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));

    Ok((stops, interpolation_type))
}

/// Converts ColorStop array to QGIS color map format
pub fn export_to_qgis(stops: &[ColorStop], interpolation_type: &str) -> String {
    let mut output = String::new();

    // Add interpolation header
    let interp = if interpolation_type == "discrete" {
        "DISCRETE"
    } else {
        "INTERPOLATED"
    };
    output.push_str(&format!("INTERPOLATION:{}\n", interp));

    // Add color stops
    for stop in stops {
        let label = stop.label.as_ref().map(|l| l.as_str()).unwrap_or("");
        output.push_str(&format!(
            "{},{},{},{},{},{}\n",
            stop.value, stop.red, stop.green, stop.blue, stop.opacity, label
        ));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_qgis_discrete() {
        let content = r#"INTERPOLATION:DISCRETE
0.1,49,54,149,105,<= 0.1
0.22,69,117,180,255,0.1 - 0.2
1.5,255,255,191,255,1.0 - 1.5
"#;
        let (stops, interp) = parse_qgis_colormap(content).unwrap();
        assert_eq!(interp, "discrete");
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].value, 0.1);
        assert_eq!(stops[0].label, Some("<= 0.1".to_string()));
        assert_eq!(stops[2].label, Some("1.0 - 1.5".to_string()));
    }

    #[test]
    fn test_parse_qgis_linear() {
        let content = r#"# Comment line
0,0,0,255,255
100,255,255,255,255
"#;
        let (stops, interp) = parse_qgis_colormap(content).unwrap();
        assert_eq!(interp, "linear");
        assert_eq!(stops.len(), 2);
    }
}
