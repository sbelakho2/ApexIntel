use anyhow::Result;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use sha2::{Digest, Sha256};

/// S3/MinIO raw document storage for crawled content.
#[derive(Clone)]
pub struct ObjectStore {
    client: S3Client,
    bucket: String,
    endpoint_url: String,
}

impl ObjectStore {
    /// Connect to an S3-compatible store (MinIO).
    pub async fn new(
        endpoint_url: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Self> {
        let creds = Credentials::new(access_key, secret_key, None, None, "apex");
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .endpoint_url(endpoint_url)
            .region(Region::new("us-east-1"))
            .credentials_provider(creds)
            .force_path_style(true) // Required for MinIO
            .build();
        let client = S3Client::from_conf(config);
        Ok(Self {
            client,
            bucket: bucket.to_string(),
            endpoint_url: endpoint_url.to_string(),
        })
    }

    /// Build from the configured endpoint and bucket using the
    /// `MINIO_ACCESS_KEY` / `MINIO_SECRET_KEY` credentials.
    ///
    /// Returns `Ok(None)` when either credential is absent, so callers can
    /// decide whether to fail, warn, or fall back to an unauthenticated
    /// liveness probe instead of being handed a store that cannot
    /// authenticate.
    pub async fn from_env_credentials(endpoint_url: &str, bucket: &str) -> Result<Option<Self>> {
        let access_key = std::env::var(apex_core::env::MINIO_ACCESS_KEY)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let secret_key = std::env::var(apex_core::env::MINIO_SECRET_KEY)
            .ok()
            .filter(|value| !value.trim().is_empty());
        match (access_key, secret_key) {
            (Some(access_key), Some(secret_key)) => Ok(Some(
                Self::new(endpoint_url, bucket, &access_key, &secret_key).await?,
            )),
            _ => Ok(None),
        }
    }

    /// Configured bucket name this store reads from and writes to.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Configured S3/MinIO endpoint URL.
    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    /// Whether the configured bucket currently exists on the endpoint.
    ///
    /// Unlike [`Self::ensure_bucket`] this never creates the bucket: a health
    /// probe must be able to report a missing bucket as a misconfiguration.
    pub async fn bucket_exists(&self) -> Result<bool> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(true),
            Err(err) => {
                let is_not_found = err.as_service_error().is_some_and(|se| se.is_not_found());
                if is_not_found {
                    Ok(false)
                } else {
                    Err(err.into())
                }
            }
        }
    }

    /// Ensure the bucket exists.
    pub async fn ensure_bucket(&self) -> Result<()> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(err) => {
                // Only create if the bucket doesn't exist; propagate other errors
                let is_not_found = err.as_service_error().is_some_and(|se| se.is_not_found());
                if is_not_found {
                    self.client
                        .create_bucket()
                        .bucket(&self.bucket)
                        .send()
                        .await?;
                    Ok(())
                } else {
                    Err(err.into())
                }
            }
        }
    }

    /// Store a raw document. Key is derived from SHA-256 of content.
    /// Returns the storage key.
    pub async fn put_raw(&self, content: &[u8], content_type: &str) -> Result<String> {
        let hash = hex::encode(Sha256::digest(content));
        let key = format!("raw/{}", hash);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(content.to_vec()))
            .content_type(content_type)
            .send()
            .await?;

        Ok(key)
    }

    /// Store a document with a specific key prefix and identifier.
    pub async fn put_with_key(
        &self,
        prefix: &str,
        id: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<String> {
        let key = format!("{}/{}", prefix, id);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(ByteStream::from(content.to_vec()))
            .content_type(content_type)
            .send()
            .await?;

        Ok(key)
    }

    /// Retrieve a raw document by key.
    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;

        let body = resp.body.collect().await?;
        Ok(body.to_vec())
    }

    /// Check if a key exists.
    pub async fn exists(&self, key: &str) -> Result<bool> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) => {
                let is_not_found = err.as_service_error().is_some_and(|se| se.is_not_found());
                if is_not_found {
                    Ok(false)
                } else {
                    Err(err.into())
                }
            }
        }
    }

    /// Delete a key.
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        Ok(())
    }

    /// List keys under a prefix (handles S3 pagination).
    pub async fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix);

            if let Some(token) = &continuation_token {
                req = req.continuation_token(token);
            }

            let resp = req.send().await?;

            for obj in resp.contents() {
                if let Some(key) = obj.key() {
                    keys.push(key.to_string());
                }
            }

            if resp.is_truncated() == Some(true) {
                continuation_token = resp.next_continuation_token().map(String::from);
                if continuation_token.is_none() {
                    break; // safety: no token means no more pages
                }
            } else {
                break;
            }
        }

        Ok(keys)
    }

    /// Derive the content-addressed key for content without storing it.
    pub fn content_key(content: &[u8]) -> String {
        let hash = hex::encode(Sha256::digest(content));
        format!("raw/{}", hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn test_content_key_deterministic() {
        let content = b"Hello, World!";
        let key1 = ObjectStore::content_key(content);
        let key2 = ObjectStore::content_key(content);
        assert_eq!(key1, key2);
        assert!(key1.starts_with("raw/"));
    }

    #[test]
    fn test_content_key_different_content() {
        let k1 = ObjectStore::content_key(b"foo");
        let k2 = ObjectStore::content_key(b"bar");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_content_key_known_hash() {
        // SHA-256 of "test" = 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
        let key = ObjectStore::content_key(b"test");
        assert_eq!(
            key,
            "raw/9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }

    #[tokio::test]
    async fn from_env_credentials_requires_both_keys_and_keeps_endpoint_and_bucket() {
        let keys = [
            apex_core::env::MINIO_ACCESS_KEY,
            apex_core::env::MINIO_SECRET_KEY,
        ];
        let saved: Vec<(&str, Option<String>)> = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }

        assert!(
            ObjectStore::from_env_credentials("http://127.0.0.1:9000", "apex-raw")
                .await
                .expect("no credentials is not an error")
                .is_none(),
            "absent credentials must disable the store instead of building one"
        );

        std::env::set_var(apex_core::env::MINIO_ACCESS_KEY, "minioadmin");
        assert!(
            ObjectStore::from_env_credentials("http://127.0.0.1:9000", "apex-raw")
                .await
                .expect("half credentials is not an error")
                .is_none(),
            "a missing secret key must still disable the store"
        );

        std::env::set_var(apex_core::env::MINIO_SECRET_KEY, "change-me");
        let store = ObjectStore::from_env_credentials("http://127.0.0.1:9000", "apex-raw")
            .await
            .expect("complete credentials build a store")
            .expect("store is configured");
        assert_eq!(store.bucket(), "apex-raw");
        assert_eq!(store.endpoint_url(), "http://127.0.0.1:9000");

        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[tokio::test]
    async fn bucket_exists_probe_targets_the_configured_bucket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock S3 endpoint");
        let addr = listener.local_addr().expect("mock S3 address");
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept probe");
            let mut buf = [0_u8; 2048];
            let read = stream.read(&mut buf).await.expect("read probe request");
            let request = String::from_utf8_lossy(&buf[..read]).to_string();
            let _ = request_tx.send(request);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        });

        let store = ObjectStore::new(&format!("http://{addr}"), "apex-intel-raw", "key", "secret")
            .await
            .expect("store builds against mock endpoint");
        assert!(
            store.bucket_exists().await.expect("mock HEAD returns 200"),
            "an existing bucket must probe true"
        );

        let request = request_rx.await.expect("probe request captured");
        assert!(
            request.starts_with("HEAD /apex-intel-raw"),
            "the configured bucket name must be the probe path: {request}"
        );
    }
}
