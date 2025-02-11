use crate::config::Config;
use anyhow::Result;
use s3::creds::Credentials;
use s3::region::Region;
use s3::Bucket;

pub(crate) async fn get_object(object_id: &str) -> Result<Vec<u8>> {
    // Given a filename, seek the file from S3 and return it as a byte stream.

    let config = Config::from_env();
    let path = format!("{}-{}", config.app_name, config.deployment);
    let credentials = Credentials::new(
        Some(&config.s3_access_key),
        Some(&config.s3_secret_key),
        None,
        None,
        None,
    )?;

    let region = Region::Custom {
        region: config.s3_region,
        endpoint: config.s3_endpoint,
    };

    let filename = format!("{}/{}", path, object_id);
    let bucket = Bucket::new(&config.s3_bucket_id, region, credentials)?;

    let response_data = bucket.get_object(filename).await?.bytes().to_vec();

    Ok(response_data)
}
