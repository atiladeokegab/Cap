//! AWS Signature Version 4, the form GCS's XML API accepts with an HMAC key.
//! Hand-written because no signing crate is in the tree; hmac, sha2 and hex are.

use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, percent_encode};
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
pub(crate) const PATH: &AsciiSet = &UNRESERVED.remove(b'/');

pub struct Key<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// The `Authorization` value and its signature for a request carrying exactly
/// `headers` (they must include `host` and `x-amz-date`).
pub fn authorize(
    key: &Key,
    method: &str,
    url: &Url,
    headers: &BTreeMap<String, String>,
    payload_sha256: &str,
) -> Result<(String, String), String> {
    let headers: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| {
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            (name.to_ascii_lowercase(), value)
        })
        .collect();
    let date = headers.get("x-amz-date").ok_or("x-amz-date is not set")?;
    let day = date.get(..8).ok_or("x-amz-date is malformed")?;
    let scope = format!("{day}/{}/{}/aws4_request", key.region, key.service);

    let path: Vec<u8> = percent_decode_str(url.path()).collect();
    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| {
            (
                percent_encode(k.as_bytes(), UNRESERVED).to_string(),
                percent_encode(v.as_bytes(), UNRESERVED).to_string(),
            )
        })
        .collect();
    query.sort();
    let query: Vec<String> = query.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical = [
        method,
        &percent_encode(&path, PATH).to_string(),
        &query.join("&"),
        &canonical_headers,
        &signed_headers,
        payload_sha256,
    ]
    .join("\n");

    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let mut signing = format!("AWS4{}", key.secret_key).into_bytes();
    for part in scope.split('/') {
        signing = hmac(&signing, part);
    }
    let signature = hex::encode(hmac(&signing, &to_sign));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        key.access_key
    );
    Ok((authorization, signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproduces_the_published_iam_vector() {
        let headers = BTreeMap::from([
            (
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded; charset=utf-8".to_string(),
            ),
            ("Host".to_string(), "iam.amazonaws.com".to_string()),
            ("X-Amz-Date".to_string(), "20150830T123600Z".to_string()),
        ]);
        let url =
            Url::parse("https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08").unwrap();
        let key = Key {
            access_key: "AKIDEXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "iam",
        };
        let (_, signature) = authorize(&key, "GET", &url, &headers, &sha256_hex(b"")).unwrap();
        assert_eq!(
            signature,
            "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }
}
