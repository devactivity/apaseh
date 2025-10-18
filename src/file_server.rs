use crate::config::Config;
use crate::http::{FileData, HttpResponse};
use log::debug;
use memmap2::MmapOptions;
use mime_guess::from_path;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct FileServer {
    config: Arc<Config>,
}

impl FileServer {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn serve(&self, path: &str, head: bool) -> HttpResponse {
        let file_path = self.resolve_path(path);

        debug!("serving file: {}", file_path.display());

        match self.read_file(&file_path, head).await {
            Ok(Some(file_data)) => {
                let mut response = HttpResponse::ok();
                if !head {
                    response.set_body_file(file_data);
                } else {
                    response.headers.insert(
                        "Content-Type".to_string(),
                        from_path(&file_path).first_or_octet_stream().to_string(),
                    );
                }

                response
            }
            Ok(None) => HttpResponse::not_found(),
            Err(_) => HttpResponse::not_found(),
        }
    }

    fn resolve_path(&self, request_path: &str) -> PathBuf {
        let mut path = PathBuf::from(&self.config.root);

        // remove leading slash
        let clean_path = request_path.trim_start_matches('/');
        if clean_path.is_empty() {
            path.push("index.html");
        } else {
            path.push(clean_path);
        }

        // directory traversal
        if !path.starts_with(&self.config.root) {
            return PathBuf::from(&self.config.root).join("index.html");
        }

        path
    }

    async fn read_file(&self, path: &Path, head: bool) -> Result<Option<FileData>, std::io::Error> {
        if !path.exists() || !path.is_file() {
            return Ok(None);
        }

        let mime_type = from_path(path).first_or_octet_stream().to_string();

        if head {
            return Ok(Some(FileData {
                data: Vec::new(),
                mime_type,
            }));
        }

        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let file_size = metadata.len() as usize;

        let data = if file_size > 64 * 1024 {
            let mmap = unsafe { MmapOptions::new().map(&file)? };
            mmap.to_vec()
        } else {
            std::fs::read(path)?
        };

        Ok(Some(FileData { data, mime_type }))
    }
}
