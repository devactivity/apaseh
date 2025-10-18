use crate::http::{HttpRequest, HttpResponse, ResponseBody};

use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use url::Url;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProxyRule {
    pub path_prefix: String,
    pub backend_url: String,
    pub strip_prefix: bool,
    pub headers: HashMap<String, String>,
}

pub struct ReverseProxy {
    pub rules: Vec<ProxyRule>,
    pub timeout: Duration,
}

impl ReverseProxy {
    pub fn new(rules: Vec<ProxyRule>) -> Self {
        Self {
            rules,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn should_proxy(&self, path: &str) -> Option<&ProxyRule> {
        log::debug!("checking {} against {} proxy rules", path, self.rules.len());

        for rule in &self.rules {
            log::debug!(
                "
            checking against rule: path_prefix='{}'
            ",
                rule.path_prefix
            );

            // matching slash: handle /api and /api/ cases
            let matches = if rule.path_prefix.ends_with('/') {
                // rule ends with /, match both /api and /api/....
                let prefix_without_slash = &rule.path_prefix[..rule.path_prefix.len() - 1];

                path == prefix_without_slash || path.starts_with(&rule.path_prefix)
            } else {
                // rule doesn't end with /, match exact or with /
                path == rule.path_prefix || path.starts_with(&format!("{}/", rule.path_prefix))
            };

            if matches {
                log::info!("path '{}' matches rule '{}'", path, rule.path_prefix);
                return Some(rule);
            }
        }

        log::debug!("No proxy rule matched for path '{path}'");
        None
    }

    pub async fn proxy_request(
        &self,
        request: &HttpRequest,
        rule: &ProxyRule,
    ) -> Result<HttpResponse, Box<dyn std::error::Error>> {
        // parse backend URL
        let backend_url = Url::parse(&rule.backend_url)?;
        let host = backend_url.host_str().unwrap_or("localhost");
        let port = backend_url.port().unwrap_or(80);

        // modify request path
        let proxy_path = if rule.strip_prefix {
            // handle both /api and /api/ cases
            let prefix_to_strip = if rule.path_prefix.ends_with('/') {
                &rule.path_prefix[..rule.path_prefix.len() - 1] // remove trailing slash
            } else {
                &rule.path_prefix
            };

            if request.path == prefix_to_strip {
                // /api -> /
                "/"
            } else if request.path.starts_with(&format!("{prefix_to_strip}/")) {
                // /api/users -> /users
                &request.path[prefix_to_strip.len()..]
            } else {
                // fallback
                &request.path
            }
        } else {
            &request.path
        };

        // build the full URL for the backend
        let backend_path = if proxy_path.starts_with('/') {
            proxy_path.to_string()
        } else {
            format!("/{proxy_path}")
        };

        debug!(
            "proxying {} {} to {}:{}{} (original: {})",
            request.method, request.path, host, port, backend_path, request.path
        );

        // connect to backend
        let backend_addr = format!("{host}:{port}");
        let mut backend_stream =
            match timeout(self.timeout, TcpStream::connect(&backend_addr)).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(e)) => {
                    error!("failed to connect to backend {backend_addr}: {e}");
                    return Ok(self.create_error_response(502, "Bad Gateway"));
                }
                Err(_) => {
                    error!("timeout connecting to backend {backend_addr}");
                    return Ok(self.create_error_response(504, "Gateway Timeout"));
                }
            };

        // forward request to backend
        if let Err(e) = self
            .forward_request(&mut backend_stream, request, &backend_path, host, rule)
            .await
        {
            error!("failed to forward request: {e}");
            return Ok(self.create_error_response(502, "Bad Gateway"));
        }

        // read response from backend
        match timeout(
            self.timeout,
            self.read_backend_response(&mut backend_stream),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => {
                error!("failed to read backend response: {e}");
                Ok(self.create_error_response(502, "Bad Gateway"))
            }
            Err(_) => {
                error!("timeout reading backend response");
                Ok(self.create_error_response(504, "Gateway Timeout"))
            }
        }
    }

    async fn forward_request(
        &self,
        stream: &mut TcpStream,
        request: &HttpRequest,
        path: &str,
        host: &str,
        rule: &ProxyRule,
    ) -> Result<(), Box<dyn std::error::Error>> {
        debug!("forwarding request: {} {}", request.method, path);

        // build request line
        let request_line = format!("{} {} HTTP/1.1\r\n", request.method, path);
        stream.write_all(request_line.as_bytes()).await?;

        // forward headers
        let mut headers_sent = std::collections::HashSet::<String>::new();

        // set/override host header
        let host_header = format!("Host: {host}\r\n");
        stream.write_all(host_header.as_bytes()).await?;
        headers_sent.insert("host".to_string());

        // add proxy headers
        if let Some(original_host) = request.headers.get("host") {
            let x_forwarded_host = format!("X-Forwarded-Host: {original_host}\r\n");
            stream.write_all(x_forwarded_host.as_bytes()).await?;
        }

        // add x-forwarded-host header
        let x_forwarded_for = if let Some(existing) = request.headers.get("x-forwarded-for") {
            format!("X-Forwarded-For: {existing}, proxy\r\n")
        } else {
            "X-Forwarded-For: proxy\r\n".to_string()
        };
        stream.write_all(x_forwarded_for.as_bytes()).await?;

        // add x-forwarded-proto
        stream.write_all(b"X-Forwarded-Proto: http\r\n").await?;

        // forward original headers
        for (key, value) in &request.headers {
            let key_lower = key.to_lowercase();
            if !headers_sent.contains(&key_lower) && key_lower != "connection" {
                let header = format!("{key}: {value}\r\n");
                stream.write_all(header.as_bytes()).await?;
                headers_sent.insert(key_lower);
            }
        }

        // add header from rule
        for (key, value) in &rule.headers {
            let key_lower = key.to_lowercase();
            if !headers_sent.contains(&key_lower) {
                let header = format!("{key}: {value}\r\n");
                stream.write_all(header.as_bytes()).await?;
            }
        }

        // add connection
        stream.write_all(b"Connection: close\r\n").await?;
        // end headers
        stream.write_all(b"\r\n").await?;

        // forward body if present
        if !request.body.is_empty() {
            stream.write_all(&request.body).await?;
        }

        stream.flush().await?;
        Ok(())
    }

    async fn read_backend_response(
        &self,
        stream: &mut TcpStream,
    ) -> Result<HttpResponse, Box<dyn std::error::Error>> {
        let mut buffer = Vec::new();
        let mut headers_end = None;

        // read until the end of headers
        loop {
            let mut temp_buf = [0_u8; 1024];
            let bytes_read = stream.read(&mut temp_buf).await?;

            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp_buf[..bytes_read]);

            // look for end of headers (\r\n)
            if let Some(position) = self.find_headers_end(&buffer) {
                headers_end = Some(position);
                break;
            }
        }

        let headers_end = headers_end.ok_or("could not find end of headers")?;
        let headers_data = &buffer[..headers_end];
        let body_start = headers_end + 4; // skip \r\n\r\n

        // parse status line
        let headers_str = String::from_utf8_lossy(headers_data);
        let mut lines = headers_str.lines();

        let status_line = lines.next().ok_or("no status line")?;
        let status_parts: Vec<&str> = status_line.split_whitespace().collect();
        let status_code: u16 = status_parts.get(1).ok_or("no status code")?.parse()?;

        let mut headers = HashMap::new();
        let mut content_length: Option<usize> = None;

        for line in lines {
            if let Some(pos) = line.find(':') {
                let key = line[..pos].trim().to_lowercase();
                let value = line[pos + 1..].trim().to_string();

                if key == "content-length" {
                    content_length = value.parse().ok();
                }

                headers.insert(key, value);
            }
        }

        // read body
        let mut body_data = Vec::new();

        // add any body data
        if body_start < buffer.len() {
            body_data.extend_from_slice(&buffer[body_start..]);
        }

        // read remaining body data
        if let Some(length) = content_length {
            let remaining = length.saturating_sub(body_data.len());
            if remaining > 0 {
                let mut remaining_buffer = vec![0_u8; remaining];
                stream.read_exact(&mut remaining_buffer).await?;
                body_data.extend_from_slice(&remaining_buffer);
            }
        } else {
            loop {
                let mut temp_buf = [0_u8; 1024];
                match stream.read(&mut temp_buf).await {
                    Ok(0) => break, // connection closed
                    Ok(n) => body_data.extend_from_slice(&temp_buf[..n]),
                    Err(_) => break,
                }
            }
        }

        // build response
        let mut response = HttpResponse::new(status_code);
        response.headers = headers;

        if !body_data.is_empty() {
            response.body = Some(ResponseBody::Bytes(body_data));
        }

        Ok(response)
    }

    fn find_headers_end(&self, buffer: &[u8]) -> Option<usize> {
        for i in 0..buffer.len().saturating_sub(3) {
            if buffer[i..i + 4] == [b'\r', b'\n', b'\r', b'\n'] {
                return Some(i);
            }
        }
        None
    }

    fn create_error_response(&self, status_code: u16, message: &str) -> HttpResponse {
        let mut response = HttpResponse::new(status_code);
        response.set_body_bytes(message.as_bytes());
        response
            .headers
            .insert("Content-Type".to_string(), "text/plain".to_string());
        response
    }
}
