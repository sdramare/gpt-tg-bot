use anyhow::{Context, Result};

pub fn parse_s3_uri(uri: &str) -> Result<(&str, &str)> {
    let path = uri
        .strip_prefix("s3://")
        .context("S3 URI must start with s3://")?;
    let (bucket, key) = path
        .split_once('/')
        .context("S3 URI must contain bucket and key")?;
    if bucket.is_empty() || key.is_empty() {
        anyhow::bail!("S3 URI bucket and key must not be empty");
    }
    Ok((bucket, key))
}

pub async fn fetch_rules_from_s3(uri: &str) -> Result<String> {
    let (bucket, key) = parse_s3_uri(uri)?;
    let config =
        aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_s3::Client::new(&config);
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .with_context(|| {
            format!(
                "Failed to get S3 object from bucket '{}' with key '{}'",
                bucket, key
            )
        })?;
    let bytes = resp.body.collect().await.with_context(|| {
        format!(
            "Failed to read S3 response body from bucket '{}' with key '{}'",
            bucket, key
        )
    })?;
    let text = String::from_utf8(bytes.into_bytes().to_vec())
        .with_context(|| {
            format!(
                "S3 object from bucket '{}' with key '{}' is not valid UTF-8",
                bucket, key
            )
        })?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_s3_uri_valid() {
        let (bucket, key) =
            parse_s3_uri("s3://my-bucket/path/to/rules.txt").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "path/to/rules.txt");
    }

    #[test]
    fn parse_s3_uri_single_key() {
        let (bucket, key) = parse_s3_uri("s3://bucket/rules.txt").unwrap();
        assert_eq!(bucket, "bucket");
        assert_eq!(key, "rules.txt");
    }

    #[test]
    fn parse_s3_uri_missing_prefix() {
        let result = parse_s3_uri("https://bucket/key");
        assert!(result.is_err());
    }

    #[test]
    fn parse_s3_uri_no_key() {
        let result = parse_s3_uri("s3://bucket");
        assert!(result.is_err());
    }

    #[test]
    fn parse_s3_uri_empty_key() {
        let result = parse_s3_uri("s3://bucket/");
        assert!(result.is_err());
    }

    #[test]
    fn parse_s3_uri_empty_bucket() {
        let result = parse_s3_uri("s3:///key");
        assert!(result.is_err());
    }

    #[test]
    fn fetch_rules_from_s3_invalid_uri() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(fetch_rules_from_s3("not-an-s3-uri"));
        assert!(result.is_err());
        assert!(
            format!("{:#}", result.unwrap_err())
                .contains("S3 URI must start with s3://")
        );
    }

    #[test]
    fn parse_s3_uri_nested_path() {
        let (bucket, key) =
            parse_s3_uri("s3://bucket/a/b/c/rules.txt").unwrap();
        assert_eq!(bucket, "bucket");
        assert_eq!(key, "a/b/c/rules.txt");
    }
}
