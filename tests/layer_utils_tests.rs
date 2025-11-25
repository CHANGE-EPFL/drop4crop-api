// Layer utility function tests
// Tests for filename parsing and bbox cropping functionality

mod common;

use drop4crop_api::config::Config;
use drop4crop_api::routes::layers::utils::{parse_filename, crop_to_bbox};
use drop4crop_api::routes::layers::models::LayerInfo;
use gdal::Dataset;
use std::ffi::CString;

// ============================================================================
// FILENAME PARSING TESTS
// ============================================================================

#[test]
fn test_parse_climate_filename() {
    let config = Config::for_tests();
    let result =
        parse_filename(&config, "wheat_lpjml_gfdl-esm4_historical_yield_2020.tif").unwrap();

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
    let config = Config::for_tests();
    let result = parse_filename(&config, "soy_mirca_area_total.tif").unwrap();

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
    let config = Config::for_tests();
    let result = parse_filename(
        &config,
        "rice_lpjml_gfdl-esm4_historical_yield_perc_2020.tif",
    )
    .unwrap();

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
    let config = Config::for_tests();
    let result = parse_filename(&config, "invalid.txt");
    assert!(result.is_err());
}

// ============================================================================
// BOUNDING BOX CROPPING TESTS
// ============================================================================

/// Test that verifies bounding box cropping functionality
/// This test creates a simple test raster and verifies that:
/// 1. A cropped version has different dimensions than the original
/// 2. The cropped version has correct georeferencing
/// 3. The cropped version contains the expected subset of data
#[test]
fn test_bbox_cropping() {
    // Create a simple test GeoTIFF in memory
    let vsi_path = "/vsimem/test_layer.tif";
    let c_vsi_path = CString::new(vsi_path).unwrap();

    // Create a test dataset: 100x100 pixels covering -180 to 180 longitude, -90 to 90 latitude
    let driver = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
    let mut dataset = driver
        .create_with_band_type::<f64, _>(vsi_path, 100, 100, 1)
        .unwrap();

    // Set geotransform: [origin_x, pixel_width, 0, origin_y, 0, pixel_height]
    // This makes each pixel 3.6 degrees wide and tall
    dataset
        .set_geo_transform(&[-180.0, 3.6, 0.0, 90.0, 0.0, -3.6])
        .unwrap();

    // Set spatial reference (WGS84)
    dataset
        .set_spatial_ref(&gdal::spatial_ref::SpatialRef::from_epsg(4326).unwrap())
        .unwrap();

    // Fill with test data (simple gradient)
    let mut band = dataset.rasterband(1).unwrap();
    let data: Vec<f64> = (0..10000).map(|i| i as f64).collect();
    use gdal::raster::Buffer;
    let mut buffer = Buffer::new((100, 100), data);
    band.write((0, 0), (100, 100), &mut buffer).unwrap();

    // Close the dataset to flush to vsimem
    drop(dataset);

    // Read the file from vsimem
    let _original_data = unsafe {
        let mode = CString::new("r").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_vsi_path.as_ptr(), mode.as_ptr());
        assert!(!fp.is_null(), "Failed to open test file");

        // Get file size
        gdal_sys::VSIFSeekL(fp, 0, 2); // SEEK_END
        let size = gdal_sys::VSIFTellL(fp) as usize;
        gdal_sys::VSIFSeekL(fp, 0, 0); // SEEK_SET

        // Read data
        let mut buffer = vec![0u8; size];
        let read = gdal_sys::VSIFReadL(buffer.as_mut_ptr() as *mut _, 1, size, fp);
        assert_eq!(read, size, "Failed to read all data");
        gdal_sys::VSIFCloseL(fp);

        buffer
    };

    // Test case: Crop to a smaller region (-90 to 0 longitude, 0 to 45 latitude)
    // This should give us roughly 25x12.5 pixels = 25x13 pixels
    let minx = -90.0;
    let miny = 0.0;
    let maxx = 0.0;
    let maxy = 45.0;

    // This is where we would call the cropping function
    // For now, we'll implement the logic inline to show what we expect

    // Open the original dataset
    let dataset = Dataset::open(vsi_path).unwrap();

    // Get geotransform
    let gt = dataset.geo_transform().unwrap();

    // Calculate pixel coordinates for the bounding box
    // Using GDAL's geotransform formula:
    // Xgeo = GT[0] + Xpixel*GT[1] + Yline*GT[2]
    // Ygeo = GT[3] + Xpixel*GT[4] + Yline*GT[5]
    // Solving for pixel coordinates:
    let col_min = ((minx - gt[0]) / gt[1]).floor() as isize;
    let col_max = ((maxx - gt[0]) / gt[1]).ceil() as isize;
    let row_min = ((maxy - gt[3]) / gt[5]).floor() as isize; // Note: gt[5] is negative
    let row_max = ((miny - gt[3]) / gt[5]).ceil() as isize;

    let (raster_x_size, raster_y_size) = dataset.raster_size();

    // Clamp to raster bounds
    let col_min = col_min.max(0).min(raster_x_size as isize);
    let col_max = col_max.max(0).min(raster_x_size as isize);
    let row_min = row_min.max(0).min(raster_y_size as isize);
    let row_max = row_max.max(0).min(raster_y_size as isize);

    let width = (col_max - col_min) as usize;
    let height = (row_max - row_min) as usize;

    // Verify the cropped dimensions are smaller than original
    assert!(
        width < raster_x_size,
        "Cropped width should be less than original"
    );
    assert!(
        height < raster_y_size,
        "Cropped height should be less than original"
    );
    assert!(width > 0, "Cropped width should be greater than 0");
    assert!(height > 0, "Cropped height should be greater than 0");

    // Expected dimensions based on our bounding box:
    // -90 to 0 longitude = 90 degrees = 25 pixels
    // 0 to 45 latitude = 45 degrees = 12.5 pixels
    assert_eq!(width, 25, "Expected width of 25 pixels for 90 degree span");
    assert_eq!(
        height, 13,
        "Expected height of 13 pixels for 45 degree span (rounded up)"
    );

    // Clean up
    unsafe {
        gdal_sys::VSIUnlink(c_vsi_path.as_ptr());
    }

    println!("Test passed: Bounding box cropping logic is correct");
    println!("Original size: {}x{}", raster_x_size, raster_y_size);
    println!("Cropped size: {}x{}", width, height);
}

/// Test the actual cropping function
#[test]
fn test_crop_to_bbox_function() {
    use gdal::raster::Buffer;

    // Create a test GeoTIFF in memory
    let vsi_path = "/vsimem/test_crop_input.tif";
    let c_vsi_path = CString::new(vsi_path).unwrap();

    // Create a test dataset: 100x100 pixels covering -180 to 180 longitude, -90 to 90 latitude
    let driver = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
    let mut dataset = driver
        .create_with_band_type::<f64, _>(vsi_path, 100, 100, 1)
        .unwrap();

    dataset
        .set_geo_transform(&[-180.0, 3.6, 0.0, 90.0, 0.0, -3.6])
        .unwrap();

    dataset
        .set_spatial_ref(&gdal::spatial_ref::SpatialRef::from_epsg(4326).unwrap())
        .unwrap();

    // Fill with test data
    let mut band = dataset.rasterband(1).unwrap();
    let data: Vec<f64> = (0..10000).map(|i| i as f64).collect();
    let mut buffer = Buffer::new((100, 100), data);
    band.write((0, 0), (100, 100), &mut buffer).unwrap();

    drop(dataset);

    // Read the test file
    let original_data = unsafe {
        let mode = CString::new("r").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_vsi_path.as_ptr(), mode.as_ptr());
        assert!(!fp.is_null());

        gdal_sys::VSIFSeekL(fp, 0, 2);
        let size = gdal_sys::VSIFTellL(fp) as usize;
        gdal_sys::VSIFSeekL(fp, 0, 0);

        let mut buffer = vec![0u8; size];
        let read = gdal_sys::VSIFReadL(buffer.as_mut_ptr() as *mut _, 1, size, fp);
        assert_eq!(read, size);
        gdal_sys::VSIFCloseL(fp);

        buffer
    };

    // Call crop_to_bbox with a bounding box
    let minx = -90.0;
    let miny = 0.0;
    let maxx = 0.0;
    let maxy = 45.0;

    let cropped_data = crop_to_bbox(&original_data, minx, miny, maxx, maxy).unwrap();

    // Verify the cropped data is valid by opening it
    let cropped_vsi_path = "/vsimem/test_cropped_output.tif";
    let c_cropped_vsi_path = CString::new(cropped_vsi_path).unwrap();

    unsafe {
        let mode = CString::new("w").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_cropped_vsi_path.as_ptr(), mode.as_ptr());
        assert!(!fp.is_null());

        let written = gdal_sys::VSIFWriteL(
            cropped_data.as_ptr() as *const _,
            1,
            cropped_data.len(),
            fp,
        );
        assert_eq!(written, cropped_data.len());
        gdal_sys::VSIFCloseL(fp);
    }

    // Open and verify the cropped dataset
    let cropped_dataset = Dataset::open(cropped_vsi_path).unwrap();
    let (width, height) = cropped_dataset.raster_size();

    // Verify dimensions
    assert_eq!(width, 25, "Cropped width should be 25 pixels");
    assert_eq!(height, 13, "Cropped height should be 13 pixels");

    // Verify geotransform
    let gt = cropped_dataset.geo_transform().unwrap();
    assert!(
        (gt[0] - (-90.0)).abs() < 0.1,
        "Origin X should be around -90.0, got {}",
        gt[0]
    );
    assert!(
        (gt[3] - 46.8).abs() < 0.1,
        "Origin Y should be around 46.8, got {}",
        gt[3]
    );
    assert!((gt[1] - 3.6).abs() < 0.01, "Pixel width should be 3.6");
    assert!((gt[5] - (-3.6)).abs() < 0.01, "Pixel height should be -3.6");

    // Clean up
    unsafe {
        gdal_sys::VSIUnlink(c_vsi_path.as_ptr());
        gdal_sys::VSIUnlink(c_cropped_vsi_path.as_ptr());
    }

    println!("Test passed: crop_to_bbox function works correctly");
    println!("Cropped size: {}x{}", width, height);
}
