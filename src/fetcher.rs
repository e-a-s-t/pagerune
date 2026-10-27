use futures_util::StreamExt;
use reqwest::{Client, StatusCode, header};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, time::Duration};
use url::Url;

pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug)]
pub struct FetchConfig {
    pub timeout: Duration,
    pub max_body_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct FetchResult {
    pub hash: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub not_modified: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("URL is not allowed")]
    UnsafeUrl,
    #[error("destination could not be resolved")]
    Resolution,
    #[error("request timed out or failed")]
    Request,
    #[error("server returned an unsuccessful status")]
    HttpStatus,
    #[error("response exceeded the configured size limit")]
    TooLarge,
    #[error("too many redirects")]
    RedirectLimit,
    #[error("invalid redirect")]
    InvalidRedirect,
    #[error("invalid response header")]
    InvalidHeader,
}

pub fn normalize_url(input: &str) -> Result<String, FetchError> {
    let mut url = Url::parse(input).map_err(|_| FetchError::UnsafeUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(FetchError::UnsafeUrl);
    }
    if let Some(host) = url.host_str().map(str::to_owned)
        && let Some(host_without_root_dot) = host.strip_suffix('.')
        && host.parse::<IpAddr>().is_err()
    {
        let host_without_root_dot = host_without_root_dot.to_owned();
        url.set_host(Some(&host_without_root_dot))
            .map_err(|_| FetchError::UnsafeUrl)?;
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

pub async fn fetch(
    input: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    config: &FetchConfig,
) -> Result<FetchResult, FetchError> {
    tokio::time::timeout(
        config.timeout,
        fetch_impl(input, etag, last_modified, config, false),
    )
    .await
    .map_err(|_| FetchError::Request)?
}

async fn fetch_impl(
    input: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    config: &FetchConfig,
    allow_private_for_test: bool,
) -> Result<FetchResult, FetchError> {
    let mut current = Url::parse(&normalize_url(input)?).map_err(|_| FetchError::UnsafeUrl)?;
    for redirect_count in 0..=MAX_REDIRECTS {
        let addresses =
            resolve_public(&current, allow_private_for_test && redirect_count == 0).await?;
        let host = current.host_str().ok_or(FetchError::UnsafeUrl)?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(config.timeout)
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| FetchError::Request)?;
        let mut request = client.get(current.clone());
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, last_modified);
        }
        let response = request.send().await.map_err(|_| FetchError::Request)?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(FetchResult {
                hash: None,
                etag: response_header(&response, header::ETAG)?,
                last_modified: response_header(&response, header::LAST_MODIFIED)?,
                not_modified: true,
            });
        }
        if status.is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(FetchError::RedirectLimit);
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .ok_or(FetchError::InvalidRedirect)?
                .to_str()
                .map_err(|_| FetchError::InvalidRedirect)?;
            current = current
                .join(location)
                .map_err(|_| FetchError::InvalidRedirect)?;
            normalize_url(current.as_str())?;
            continue;
        }
        if !status.is_success() {
            return Err(FetchError::HttpStatus);
        }
        if response
            .content_length()
            .is_some_and(|n| n > config.max_body_bytes as u64)
        {
            return Err(FetchError::TooLarge);
        }
        let etag = response_header(&response, header::ETAG)?;
        let last_modified = response_header(&response, header::LAST_MODIFIED)?;
        let mut stream = response.bytes_stream();
        let mut hasher = Sha256::new();
        let mut size = 0usize;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| FetchError::Request)?;
            size = size.checked_add(chunk.len()).ok_or(FetchError::TooLarge)?;
            if size > config.max_body_bytes {
                return Err(FetchError::TooLarge);
            }
            hasher.update(&chunk);
        }
        let hash = format!("{:x}", hasher.finalize());
        return Ok(FetchResult {
            hash: Some(hash),
            etag,
            last_modified,
            not_modified: false,
        });
    }
    Err(FetchError::RedirectLimit)
}

fn response_header(
    response: &reqwest::Response,
    name: header::HeaderName,
) -> Result<Option<String>, FetchError> {
    response
        .headers()
        .get(name)
        .map(|value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|_| FetchError::InvalidHeader)
        })
        .transpose()
}

async fn resolve_public(
    url: &Url,
    allow_private_for_test: bool,
) -> Result<Vec<std::net::SocketAddr>, FetchError> {
    let host = url.host_str().ok_or(FetchError::UnsafeUrl)?;
    let port = url.port_or_known_default().ok_or(FetchError::UnsafeUrl)?;
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![std::net::SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| FetchError::Resolution)?
            .collect()
    };
    if addresses.is_empty()
        || (!allow_private_for_test && addresses.iter().any(|address| !is_public(address.ip())))
    {
        return Err(FetchError::UnsafeUrl);
    }
    Ok(addresses)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_unspecified()
                || octets[0] == 0
                || octets[0] >= 224
                || octets[0..2] == [100, 64]
                || octets[0..2] == [169, 254]
                || octets[0..3] == [192, 0, 0]
                || octets[0..3] == [192, 0, 2]
                || octets[0..2] == [198, 18]
                || octets[0..2] == [198, 19]
                || octets[0..3] == [198, 51, 100]
                || octets[0..3] == [203, 0, 113]
                || octets[0] >= 240)
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            let unique_local = octets[0] & 0xfe == 0xfc;
            let mapped_v4 = ip.to_ipv4_mapped();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unicast_link_local()
                || unique_local
                || !(0x20..=0x3f).contains(&octets[0])
                || octets[0..4] == [0x20, 0x01, 0x0d, 0xb8]
                || octets[0..2] == [0x20, 0x02]
                || mapped_v4.is_some_and(|v4| !is_public(IpAddr::V4(v4))))
        }
    }
}

pub fn default_fetch_config() -> FetchConfig {
    FetchConfig {
        timeout: Duration::from_secs(15),
        max_body_bytes: MAX_BODY_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn test_server(response: &'static [u8], delay: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(response).await;
        });
        format!("http://{address}/")
    }

    #[test]
    fn normalizes_url_and_removes_fragment_and_default_port() {
        assert_eq!(
            normalize_url("HTTP://Example.com:80/path#part").unwrap(),
            "http://example.com/path"
        );
        assert_eq!(
            normalize_url("http://example.com./path").unwrap(),
            "http://example.com/path"
        );
    }

    #[test]
    fn rejects_unsafe_schemes_and_userinfo() {
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(normalize_url("http://user:pass@example.com/").is_err());
    }

    #[test]
    fn blocks_private_and_internal_addresses() {
        for address in ["127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "fc00::1"] {
            assert!(!is_public(address.parse().unwrap()), "{address}");
        }
        assert!(is_public("1.1.1.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn local_http_server_covers_status_and_size_failures() {
        let config = FetchConfig {
            timeout: Duration::from_secs(2),
            max_body_bytes: 4,
        };
        let status_url = test_server(
            b"HTTP/1.1 503 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            fetch_impl(&status_url, None, None, &config, true).await,
            Err(FetchError::HttpStatus)
        ));
        let big_url = test_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345",
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            fetch_impl(&big_url, None, None, &config, true).await,
            Err(FetchError::TooLarge)
        ));
    }

    #[tokio::test]
    async fn local_http_server_timeout_is_reported() {
        let url = test_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            Duration::from_millis(150),
        )
        .await;
        let config = FetchConfig {
            timeout: Duration::from_millis(20),
            max_body_bytes: 100,
        };
        assert!(matches!(
            fetch_impl(&url, None, None, &config, true).await,
            Err(FetchError::Request)
        ));
    }

    #[tokio::test]
    async fn production_fetch_rejects_loopback_before_connecting() {
        let url = test_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            fetch(&url, None, None, &default_fetch_config()).await,
            Err(FetchError::UnsafeUrl)
        ));
    }

    #[tokio::test]
    async fn redirect_to_private_destination_is_rejected() {
        let url = test_server(
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/internal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            Duration::ZERO,
        )
        .await;
        let config = FetchConfig {
            timeout: Duration::from_secs(2),
            max_body_bytes: 100,
        };
        assert!(matches!(
            fetch_impl(&url, None, None, &config, true).await,
            Err(FetchError::UnsafeUrl)
        ));
    }

    #[tokio::test]
    async fn conditional_request_accepts_not_modified_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let read = socket.read(&mut request).await.unwrap();
            let _ = request_tx.send(String::from_utf8_lossy(&request[..read]).to_ascii_lowercase());
            socket
                .write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let config = FetchConfig {
            timeout: Duration::from_secs(2),
            max_body_bytes: 100,
        };
        let result = fetch_impl(
            &format!("http://{address}/"),
            Some("\"v1\""),
            None,
            &config,
            true,
        )
        .await
        .unwrap();
        assert!(result.not_modified);
        assert!(result.hash.is_none());
        assert!(request_rx.await.unwrap().contains("if-none-match: \"v1\""));
    }
}
