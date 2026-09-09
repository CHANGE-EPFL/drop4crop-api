#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayerCacheEntryKind {
    Cog,
    Png { tile_coords: String },
}

fn layer_name_stem(name: &str) -> &str {
    for extension in [".tiff", ".tif", ".TIFF", ".TIF"] {
        if let Some(stem) = name.strip_suffix(extension) {
            return stem;
        }
    }
    name
}

pub(crate) fn cache_entry_for_layer(
    key: &str,
    prefix: &str,
    requested_layer: &str,
) -> Option<LayerCacheEntryKind> {
    let stem = key.strip_prefix(prefix)?;
    if stem.contains("/stats:") || stem.starts_with("stats:") || stem.ends_with(":downloading") {
        return None;
    }

    let requested_layer = layer_name_stem(requested_layer);
    let png_entry = |layer: &str, coordinates: &[&str]| {
        (layer_name_stem(layer) == requested_layer && coordinates.len() == 3).then(|| {
            LayerCacheEntryKind::Png {
                tile_coords: coordinates.join("/"),
            }
        })
    };

    if let Some(rest) = stem.strip_prefix("png/") {
        let parts: Vec<&str> = rest.split('/').collect();
        return if parts.len() == 5 {
            png_entry(parts[0], &parts[2..])
        } else {
            None
        };
    }
    if let Some(rest) = stem.strip_prefix("png-globe/") {
        let parts: Vec<&str> = rest.split('/').collect();
        return if parts.len() == 4 {
            png_entry(parts[0], &parts[1..])
        } else {
            None
        };
    }
    if let Some(rest) = stem.strip_prefix("png-card/") {
        let parts: Vec<&str> = rest.split('/').collect();
        return if parts.len() == 5 {
            png_entry(parts[1], &parts[2..])
        } else {
            None
        };
    }

    let filename = match stem.split('/').collect::<Vec<_>>().as_slice() {
        [filename] => *filename,
        [_project_id, filename] => *filename,
        _ => return None,
    };
    (layer_name_stem(filename) == requested_layer).then_some(LayerCacheEntryKind::Cog)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "drop4crop-prod/";

    #[test]
    fn classifies_every_cache_namespace_for_one_layer() {
        let cases = [
            ("drop4crop-prod/barley.tif", LayerCacheEntryKind::Cog),
            (
                "drop4crop-prod/2f438fd7-1ca2-453e-8614-1de0e959bdbf/barley.tif",
                LayerCacheEntryKind::Cog,
            ),
            (
                "drop4crop-prod/png/barley/style-1/4/8/5",
                LayerCacheEntryKind::Png {
                    tile_coords: "4/8/5".to_string(),
                },
            ),
            (
                "drop4crop-prod/png-globe/barley/3/2/1",
                LayerCacheEntryKind::Png {
                    tile_coords: "3/2/1".to_string(),
                },
            ),
            (
                "drop4crop-prod/png-card/food-security/barley/2/1/1",
                LayerCacheEntryKind::Png {
                    tile_coords: "2/1/1".to_string(),
                },
            ),
        ];

        for (key, expected) in cases {
            assert_eq!(cache_entry_for_layer(key, PREFIX, "barley"), Some(expected));
        }
    }

    #[test]
    fn excludes_other_layers_and_internal_keys() {
        for key in [
            "drop4crop-prod/wheat.tif",
            "drop4crop-prod/png/wheat/style-1/4/8/5",
            "drop4crop-prod/stats:2026-09-09:barley:xyz",
            "drop4crop-prod/barley.tif:downloading",
        ] {
            assert_eq!(cache_entry_for_layer(key, PREFIX, "barley.tif"), None);
        }
    }
}
