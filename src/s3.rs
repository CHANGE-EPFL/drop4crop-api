use crate::config::Config;
use anyhow::Result;
use redis::{self, AsyncCommands};
use s3::creds::Credentials;
use s3::region::Region;
use s3::Bucket;

pub(crate) async fn get_object(object_id: &str) -> Result<Vec<u8>> {
    // Given a filename, seek the file from S3 and return it as a byte stream.

    let config = Config::from_env();
    println!("Config: {:?}", config);
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

    println!("Getting object: {}", filename);

    // Check S3
    let cache_result = check_cache(&filename).await?;
    if cache_result.1 {
        return Ok(cache_result.0);
    }
    let data = bucket.get_object(filename.clone()).await?.bytes().to_vec();

    // Push to cache
    push_cache(&filename, &data).await?;

    Ok(data)
}

pub async fn check_cache(filename: &str) -> Result<(Vec<u8>, bool)> {
    // Check if the file is in the cache, if it is not, return false, if it is
    // collect the data and return it to the caller.
    let config = Config::from_env();
    let path = format!("{}-{}", config.app_name, config.deployment);
    let filename = format!("{}/{}", path, filename);

    let client = redis::Client::open(format!(
        "redis://{}:{}/",
        config.redis_url, config.redis_port
    ))
    .unwrap();

    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let result: Option<Vec<u8>> = redis::cmd("GET")
        .arg(&[filename.clone()])
        .query_async(&mut con)
        .await
        .unwrap();
    match &result {
        Some(_) => println!("Data exists for {}", filename),
        None => println!("No data for {}", filename),
    }

    match result {
        Some(data) => Ok((data, true)),
        None => Ok((vec![], false)),
    }
}

pub async fn push_cache(filename: &str, data: &[u8]) -> Result<()> {
    // Push the data to the cache.
    let config = Config::from_env();
    let path = format!("{}-{}", config.app_name, config.deployment);
    let filename = format!("{}/{}", path, filename);

    let client = redis::Client::open(format!(
        "redis://{}:{}/",
        config.redis_url, config.redis_port
    ))
    .unwrap();

    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("SET")
        .arg(filename)
        .arg(data)
        .query_async(&mut con)
        .await
        .unwrap();

    Ok(())
}
