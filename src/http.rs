use crate::memory_pool::MemoryPool;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    net::TcpStream,
};

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub async fn parse(
        stream: &mut TcpStream,
        _memory_pool: &Arc<MemoryPool>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();

        // read req line
        reader.read_line(&mut line).await?;
        if line.is_empty() {
            return Err("empty request line".into());
        }

        let parts: Vec<&str> = line.trim().split_whitespace().collect();

        if parts.len() != 3 {
            return Err("invalid HTTP request line".into());
        }

        let method = parts[0].to_string();
        let path = parts[1].to_string();
        let version = parts[2].to_string();

        // read header
        let mut headers = HashMap::new();

        loop {
            line.clear();
            reader.read_line(&mut line).await?;
            let line = line.trim();

            if line.is_empty() {
                break;
            }

            if let Some(pos) = line.find(':') {
                let key = line[..pos].trim().to_lowercase();
                let value = line[pos + 1..].trim().to_string();

                headers.insert(key, value);
            }
        }

        // read body if present
        let mut body = Vec::new();
        if let Some(content_length) = headers.get("content-length") {
            if let Ok(length) = content_length.parse::<usize>() {
                if length > 0 {
                    body.resize(length, 0);
                    reader.read_exact(&mut body).await?;
                }
            }
        }

        Ok(HttpRequest {
            method,
            path,
            version,
            headers,
            body,
        })
    }

    pub fn should_keep_alive(&self) -> bool {
        if let Some(connection) = self.headers.get("connection") {
            connection.to_lowercase() == "keep-alive"
        } else {
            // http/1.1 defaults to keep-alive
            self.version == "HTTP/1.1"
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Option<ResponseBody>,
}

#[derive(Debug, Clone)]
pub enum ResponseBody {
    Bytes(Vec<u8>),
    File(FileData),
}

#[derive(Debug, Clone)]
pub struct FileData {
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl ResponseBody {
    pub fn len(&self) -> usize {
        match self {
            ResponseBody::Bytes(data) => data.len(),
            ResponseBody::File(file_data) => file_data.data.len(),
        }
    }
}

impl HttpResponse {
    pub fn new(status_code: u16) -> Self {
        let status_text = Self::status_text(status_code);
        let mut headers = HashMap::new();

        headers.insert("Server".to_string(), "Apaseh/1.0".to_string());
        headers.insert("Connection".to_string(), "keep-alive".to_string());

        Self {
            status_code,
            status_text,
            headers,
            body: None,
        }
    }

    pub fn ok() -> Self {
        Self::new(200)
    }

    pub fn not_found() -> Self {
        let mut response = Self::new(404);
        response.set_body_bytes(b"404 not found");
        response
            .headers
            .insert("Content-Type".to_string(), "text/plain".to_string());

        response
    }

    pub fn set_body_bytes(&mut self, data: &[u8]) {
        self.body = Some(ResponseBody::Bytes(data.to_vec()));
        self.headers
            .insert("Content-Length".to_string(), data.len().to_string());
    }

    pub fn method_not_allowed() -> Self {
        let mut response = Self::new(405);
        response.set_body_bytes(b"405 method not allowed");
        response
            .headers
            .insert("Content-Type".to_string(), "text/plain".to_string());

        response
    }

    pub fn set_body_file(&mut self, file_data: FileData) {
        self.headers.insert(
            "Content-Length".to_string(),
            file_data.data.len().to_string(),
        );
        self.headers
            .insert("Content-Type".to_string(), file_data.mime_type.clone());
        self.body = Some(ResponseBody::File(file_data));
    }

    pub fn headers_bytes(&self) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {} {}\r\n", self.status_code, self.status_text);

        for (key, value) in &self.headers {
            response.push_str(&format!("{key}: {value}\r\n"));
        }
        response.push_str("\r\n");
        response.into_bytes()
    }

    fn status_text(code: u16) -> String {
        match code {
            200 => "OK",
            400 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            _ => "Unknown",
        }
        .to_string()
    }
}
