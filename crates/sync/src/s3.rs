use std::time::Duration;

use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::remote::{Remote, RemoteError};

/// Where an S3-compatible bucket is; the secret key is kept separately in the system's credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Config {
    /// Which service the settings page set this up for ("aws", "r2", "supabase"...), so it can show the right fields.
    #[serde(default)]
    pub provider: Option<String>,
    /// Leave empty for AWS; set for Cloudflare R2, MinIO and other S3-compatible services.
    #[serde(default)]
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    /// A folder inside the bucket to keep MultiTasker's files in.
    #[serde(default)]
    pub prefix: Option<String>,
    pub access_key_id: String,
}

pub struct S3Remote {
    config: S3Config,
    secret: String,
    agent: ureq::Agent,
}

impl S3Remote {
    pub fn new(config: S3Config, secret: String) -> Result<Self, RemoteError> {
        if config.bucket.trim().is_empty() || config.access_key_id.trim().is_empty() || secret.is_empty() {
            return Err(RemoteError::Io("The bucket, access key and secret are all needed.".into()));
        }
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Ok(Self { config, secret, agent })
    }

    /// The bucket folder MultiTasker's keys live under, with a trailing slash, or empty for the bucket root.
    fn folder(&self) -> String {
        match self.config.prefix.as_deref().map(|p| p.trim_matches('/')).filter(|p| !p.is_empty()) {
            Some(prefix) => format!("{prefix}/"),
            None => String::new(),
        }
    }

    /// The base URL, host and path for an object key: path-style on custom endpoints, virtual-hosted on AWS.
    fn locate(&self, object_key: &str) -> (String, String, String) {
        match self.config.endpoint.as_deref().map(|e| e.trim_end_matches('/')).filter(|e| !e.is_empty()) {
            Some(endpoint) => {
                // Some services serve S3 under a path (Supabase: /storage/v1/s3), which goes before the bucket.
                let (scheme, rest) = endpoint.split_once("://").unwrap_or(("https", endpoint));
                let (host, base) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, String::new()));
                (format!("{scheme}://{host}"), host.to_owned(), format!("{base}/{}/{object_key}", self.config.bucket))
            }
            None => {
                let host = format!("{}.s3.{}.amazonaws.com", self.config.bucket, self.config.region);
                (format!("https://{host}"), host, format!("/{object_key}"))
            }
        }
    }

    fn send(&self, method: &str, object_key: &str, query: &[(&str, &str)], body: &[u8]) -> Result<(u16, Vec<u8>), RemoteError> {
        let (base, host, path) = self.locate(object_key);
        let path = uri_encode(&path, false);
        let query = canonical_query(query);
        let payload_hash = hex::encode(Sha256::digest(body));
        let now = Utc::now();
        let headers = [("host", host.as_str()), ("x-amz-content-sha256", payload_hash.as_str())];
        let signed = sign(&Request { method, path: &path, query: &query, headers: &headers, payload_hash: &payload_hash }, &self.config, &self.secret, now);

        let url = if query.is_empty() { format!("{base}{path}") } else { format!("{base}{path}?{query}") };
        let result = match method {
            "PUT" => self
                .agent
                .put(&url)
                .header("x-amz-content-sha256", &payload_hash)
                .header("x-amz-date", &signed.amz_date)
                .header("authorization", &signed.authorization)
                .send(body),
            _ => self
                .agent
                .get(&url)
                .header("x-amz-content-sha256", &payload_hash)
                .header("x-amz-date", &signed.amz_date)
                .header("authorization", &signed.authorization)
                .call(),
        };
        let mut response = result.map_err(|e| RemoteError::Io(e.to_string()))?;
        let status = response.status().as_u16();
        let bytes = response.body_mut().read_to_vec().map_err(|e| RemoteError::Io(e.to_string()))?;
        Ok((status, bytes))
    }
}

fn failure(status: u16, body: &[u8]) -> RemoteError {
    let text = String::from_utf8_lossy(body);
    match (xml_values(&text, "Code").into_iter().next(), xml_values(&text, "Message").into_iter().next()) {
        (Some(code), Some(message)) => RemoteError::Io(format!("{code}: {message}")),
        _ => RemoteError::Io(format!("The storage service answered {status}.")),
    }
}

impl Remote for S3Remote {
    fn list(&self, prefix: &str) -> Result<Vec<String>, RemoteError> {
        let folder = self.folder();
        let full_prefix = format!("{folder}{prefix}");

        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", full_prefix.as_str())];
            if let Some(t) = token.as_deref() {
                query.push(("continuation-token", t));
            }
            let (status, body) = self.send("GET", "", &query, b"")?;
            if status != 200 {
                return Err(failure(status, &body));
            }
            let text = String::from_utf8_lossy(&body);
            // Keys come back with the bucket folder in front; strip it so callers see the keys they wrote.
            keys.extend(xml_values(&text, "Key").into_iter().filter_map(|k| k.strip_prefix(&folder).map(str::to_owned)));
            token = xml_values(&text, "NextContinuationToken").into_iter().next();
            if token.is_none() || xml_values(&text, "IsTruncated").first().map(String::as_str) != Some("true") {
                return Ok(keys);
            }
        }
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, RemoteError> {
        match self.send("GET", &format!("{}{key}", self.folder()), &[], b"")? {
            (200, body) => Ok(Some(body)),
            (404, _) => Ok(None),
            (status, body) => Err(failure(status, &body)),
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<(), RemoteError> {
        match self.send("PUT", &format!("{}{key}", self.folder()), &[], data)? {
            (200, _) => Ok(()),
            (status, body) => Err(failure(status, &body)),
        }
    }
}

struct Request<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a str,
    /// Lowercase names, sorted by name.
    headers: &'a [(&'a str, &'a str)],
    payload_hash: &'a str,
}

struct Signed {
    amz_date: String,
    authorization: String,
}

/// AWS Signature Version 4 for one request.
fn sign(req: &Request, config: &S3Config, secret: &str, now: DateTime<Utc>) -> Signed {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];

    let mut headers: Vec<(&str, &str)> = req.headers.to_vec();
    headers.push(("x-amz-date", &amz_date));
    headers.sort_by_key(|(name, _)| *name);
    let canonical_headers: String = headers.iter().map(|(n, v)| format!("{n}:{}\n", v.trim())).collect();
    let signed_headers = headers.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed_headers}\n{}",
        req.method, req.path, req.query, req.payload_hash
    );
    let scope = format!("{date}/{}/s3/aws4_request", config.region);
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex::encode(Sha256::digest(canonical_request.as_bytes())));

    let key = [date, config.region.as_str(), "s3", "aws4_request"]
        .iter()
        .fold(format!("AWS4{secret}").into_bytes(), |key, part| hmac(&key, part.as_bytes()));
    let signature = hex::encode(hmac(&key, string_to_sign.as_bytes()));

    Signed {
        authorization: format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            config.access_key_id
        ),
        amz_date,
    }
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes everything but unreserved characters (and `/` in paths), as SigV4 requires.
fn uri_encode(value: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn canonical_query(params: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = params.iter().map(|(k, v)| (uri_encode(k, true), uri_encode(v, true))).collect();
    pairs.sort();
    pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

/// The text of every `<tag>…</tag>` in a response, with XML escapes undone.
fn xml_values(xml: &str, tag: &str) -> Vec<String> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut values = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        rest = &rest[start + open.len()..];
        let Some(end) = rest.find(&close) else { break };
        values.push(
            rest[..end]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&"),
        );
        rest = &rest[end + close.len()..];
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn example_config() -> S3Config {
        S3Config {
            provider: None,
            endpoint: None,
            region: "us-east-1".into(),
            bucket: "examplebucket".into(),
            prefix: None,
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
        }
    }

    fn example_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap()
    }

    // Both expected signatures are AWS's published SigV4 examples for S3.
    #[test]
    fn signs_like_the_aws_get_object_example() {
        let headers = [("host", "examplebucket.s3.amazonaws.com"), ("range", "bytes=0-9"), ("x-amz-content-sha256", EMPTY_HASH)];
        let req = Request { method: "GET", path: "/test.txt", query: "", headers: &headers, payload_hash: EMPTY_HASH };
        let signed = sign(&req, &example_config(), "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", example_time());
        assert!(
            signed.authorization.ends_with("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"),
            "{}",
            signed.authorization
        );
        assert!(signed.authorization.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn signs_like_the_aws_list_objects_example() {
        let query = canonical_query(&[("prefix", "J"), ("max-keys", "2")]);
        assert_eq!(query, "max-keys=2&prefix=J");
        let headers = [("host", "examplebucket.s3.amazonaws.com"), ("x-amz-content-sha256", EMPTY_HASH)];
        let req = Request { method: "GET", path: "/", query: &query, headers: &headers, payload_hash: EMPTY_HASH };
        let signed = sign(&req, &example_config(), "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", example_time());
        assert!(
            signed.authorization.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"),
            "{}",
            signed.authorization
        );
    }

    #[test]
    fn locates_keys_on_aws_and_custom_endpoints() {
        let aws = S3Remote::new(S3Config { prefix: Some("/team/".into()), ..example_config() }, "s".into()).unwrap();
        assert_eq!(aws.folder(), "team/");
        assert_eq!(
            aws.locate(&format!("{}multitasker/v1/a.json", aws.folder())),
            ("https://examplebucket.s3.us-east-1.amazonaws.com".into(), "examplebucket.s3.us-east-1.amazonaws.com".into(), "/team/multitasker/v1/a.json".into())
        );

        let r2 = S3Remote::new(
            S3Config { endpoint: Some("https://acct.r2.cloudflarestorage.com/".into()), region: "auto".into(), ..example_config() },
            "s".into(),
        )
        .unwrap();
        assert_eq!(r2.folder(), "");
        assert_eq!(r2.locate("k.json").2, "/examplebucket/k.json");

        let supabase = S3Remote::new(
            S3Config {
                endpoint: Some("https://abcd.storage.supabase.co/storage/v1/s3".into()),
                region: "ap-southeast-1".into(),
                ..example_config()
            },
            "s".into(),
        )
        .unwrap();
        assert_eq!(
            supabase.locate("k.json"),
            ("https://abcd.storage.supabase.co".into(), "abcd.storage.supabase.co".into(), "/storage/v1/s3/examplebucket/k.json".into())
        );
        assert_eq!(r2.locate("k.json").1, "acct.r2.cloudflarestorage.com");
    }

    #[test]
    fn reads_list_responses_and_errors() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated><Contents><Key>a/b&amp;c.json</Key></Contents>\
                   <Contents><Key>a/d.json</Key></Contents><NextContinuationToken>t1</NextContinuationToken></ListBucketResult>";
        assert_eq!(xml_values(xml, "Key"), ["a/b&c.json", "a/d.json"]);
        assert_eq!(xml_values(xml, "NextContinuationToken"), ["t1"]);

        let err = failure(403, b"<Error><Code>SignatureDoesNotMatch</Code><Message>Bad signature</Message></Error>");
        assert_eq!(err.to_string(), "SignatureDoesNotMatch: Bad signature");
        assert_eq!(uri_encode("a b/c+d", false), "a%20b/c%2Bd");
    }
}
