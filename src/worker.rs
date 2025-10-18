use crate::cache::ResponseCache;
use crate::config::Config;
use crate::file_server::FileServer;
use crate::http::{HttpRequest, HttpResponse};
use crate::memory_pool::MemoryPool;
use crate::proxy::ReverseProxy;
use log::debug;
use socket2::Socket;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    time::timeout,
};

pub struct Worker {
    id: usize,
    config: Arc<Config>,
    cache: Arc<ResponseCache>,
    file_server: Arc<FileServer>,
    memory_pool: Arc<MemoryPool>,
    socket: Socket,
    proxy: Arc<ReverseProxy>,
}

impl Worker {
    pub fn new(id: usize, config: Arc<Config>, socket: Socket) -> Self {
        Self {
            id,
            config: Arc::clone(&config),
            cache: Arc::new(ResponseCache::new(config.cache_size, config.cache_ttl)),
            file_server: Arc::new(FileServer::new(Arc::clone(&config))),
            memory_pool: Arc::new(MemoryPool::init()),
            socket,
            proxy: Arc::new(ReverseProxy::new(config.proxy_rules.clone())),
        }
    }

    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        rt.block_on(async {
            // create tcplistener
            let std_listener = self.socket.try_clone()?;
            let listener = TcpListener::from_std(std_listener.into())?;

            self.event_loop(listener).await
        })
    }

    async fn event_loop(&self, listener: TcpListener) -> Result<(), Box<dyn std::error::Error>> {
        let mut connection_count = 0;

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    if connection_count >= self.config.worker_connections {
                        continue;
                    }

                    connection_count += 1;

                    let config = Arc::clone(&self.config);
                    let cache = Arc::clone(&self.cache);
                    let file_server = Arc::clone(&self.file_server);
                    let memory_pool = Arc::clone(&self.memory_pool);
                    let worker_id = self.id;
                    let proxy = Arc::clone(&self.proxy);

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(
                            stream,
                            config,
                            cache,
                            file_server,
                            memory_pool,
                            worker_id,
                            proxy,
                        )
                        .await
                        {
                            debug!("connection error: {e}");
                        }
                    });
                }
                Err(e) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    async fn handle_connection(
        mut stream: TcpStream,
        config: Arc<Config>,
        cache: Arc<ResponseCache>,
        file_server: Arc<FileServer>,
        memory_pool: Arc<MemoryPool>,
        worker_id: usize,
        proxy: Arc<ReverseProxy>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // set tcp opptions
        if config.tcp_nodelay {
            stream.set_nodelay(true)?;
        }

        let mut keep_alive = true;
        let connection_start = Instant::now();

        while keep_alive {
            // read request with timeout
            let request = match timeout(
                Duration::from_secs(config.keep_alive_timeout),
                HttpRequest::parse(&mut stream, &memory_pool),
            )
            .await
            {
                Ok(Ok(req)) => req,
                Ok(Err(_)) => {
                    break;
                }
                Err(_) => {
                    break;
                }
            };

            // check if connection should be alive
            keep_alive = request.should_keep_alive()
                && connection_start.elapsed() < Duration::from_secs(config.keep_alive_timeout);

            // process request
            let response =
                Self::process_request(&request, &config, &cache, &file_server, worker_id, &proxy)
                    .await;

            // send response
            Self::send_response(&mut stream, &response, &config).await?;

            // log request
            Self::log_request(&request, &response, worker_id);

            if !keep_alive {
                break;
            }
        }

        Ok(())
    }

    async fn process_request(
        request: &HttpRequest,
        config: &Config,
        cache: &ResponseCache,
        file_server: &FileServer,
        worker_id: usize,
        proxy: &ReverseProxy,
    ) -> HttpResponse {
        // if request should be proxied
        if let Some(rule) = proxy.should_proxy(&request.path) {
            match proxy.proxy_request(request, rule).await {
                Ok(response) => return response,
                Err(_) => {
                    return HttpResponse::new(502); //bad gateway
                }
            }
        }

        // check cache first
        if let Some(cached_response) = cache.get(&request.path) {
            return cached_response;
        }

        // process request
        let response = match request.method.as_str() {
            "GET" | "HEAD" => {
                file_server
                    .serve(&request.path, request.method == "HEAD")
                    .await
            }
            _ => HttpResponse::method_not_allowed(),
        };

        // cache successful responses for static files
        if response.status_code == 200 && request.method == "GET" {
            cache.put(request.path.clone(), response.clone());
        }

        response
    }

    async fn send_response(
        stream: &mut TcpStream,
        response: &HttpResponse,
        config: &Config,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // send headers
        stream.write_all(&response.headers_bytes()).await?;

        // send body
        if let Some(body) = &response.body {
            match body {
                crate::http::ResponseBody::Bytes(data) => {
                    stream.write_all(data).await?;
                }
                crate::http::ResponseBody::File(file_data) => {
                    if config.sendfile {
                        stream.write_all(&file_data.data).await?;
                    }
                    stream.write_all(&file_data.data).await?;
                }
            }
        }

        stream.flush().await?;
        Ok(())
    }

    fn log_request(request: &HttpRequest, response: &HttpResponse, worker_id: usize) {
        log::info!(
            "Worker {} - {} {} - {} - {}",
            worker_id,
            request.method,
            request.path,
            response.status_code,
            response.body.as_ref().map_or(0, |b| b.len()),
        );
    }
}
