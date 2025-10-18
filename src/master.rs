use crate::{config::Config, signal_handler::SignalHandler, worker::Worker};

use log::{info, warn};
use nix::{
    sys::signal::{self, Signal},
    unistd::{ForkResult, Pid, fork},
};

use socket2::{Domain, Protocol, Socket, Type};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

pub struct Master {
    config: Arc<Config>,
    workers: HashMap<i32, Pid>,
    listening_socket: Option<Socket>,
    signal_handler: SignalHandler,
}

impl Master {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
            workers: HashMap::new(),
            listening_socket: None,
            signal_handler: SignalHandler::new(),
        }
    }

    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        // setup signal handlers
        self.signal_handler.setup()?;

        // create listening socket with SO_REUSEPORT
        let socket = self.create_listening_socket()?;
        self.listening_socket = Some(socket);

        // Fork worker processes
        self.spawn_workers()?;

        // master process loop
        loop {
            // check for signals
            if let Some(signal) = self.signal_handler.check_signals() {
                match signal {
                    Signal::SIGTERM | Signal::SIGINT => {
                        self.shutdown_workers();
                        break;
                    }
                    Signal::SIGHUP => {
                        self.reload_workers()?;
                    }
                    Signal::SIGCHLD => {
                        self.handle_child_exit();
                    }
                    _ => {}
                }
            }

            // check worker health
            self.monitor_workers();

            std::thread::sleep(Duration::from_millis(100));
        }

        Ok(())
    }

    fn create_listening_socket(&self) -> Result<Socket, Box<dyn std::error::Error>> {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;

        // set socket opts
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.set_nonblocking(true)?;

        // bind and listen
        let addr: SocketAddr = format!("0.0.0.0:{}", self.config.port).parse()?;
        socket.bind(&addr.into())?;
        socket.listen(self.config.backlog)?;

        info!("listening on port {}", self.config.port);

        Ok(socket)
    }

    fn spawn_workers(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        for worker_id in 0..self.config.worker_processes {
            let pid = self.spawn_worker(worker_id)?;
            self.workers.insert(worker_id as i32, pid);
        }

        Ok(())
    }

    fn spawn_worker(&self, worker_id: usize) -> Result<Pid, Box<dyn std::error::Error>> {
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => Ok(child),
            Ok(ForkResult::Child) => {
                // child process become a worker
                let worker = Worker::new(
                    worker_id,
                    Arc::clone(&self.config),
                    self.listening_socket.as_ref().unwrap().try_clone().unwrap(),
                );

                if let Err(e) = worker.run() {
                    warn!("worker {worker_id} failed: {e}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }

            Err(e) => Err(format!("failed to fork worker: {e}").into()),
        }
    }

    fn shutdown_workers(&mut self) {
        for (worker_id, pid) in &self.workers {
            info!("shutting down worker {worker_id}");
            let _ = signal::kill(*pid, Signal::SIGTERM);
        }

        // wait
        std::thread::sleep(Duration::from_secs(5));

        // force killl
        for (worker_id, pid) in &self.workers {
            if signal::kill(*pid, Signal::SIGKILL).is_ok() {
                warn!("force killed worker {worker_id}");
            }
        }
    }

    fn reload_workers(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // graceful reload
        // spawn new workers
        // then shutdown old ones
        let old_workers = std::mem::take(&mut self.workers);

        // spawn new workers
        self.spawn_workers()?;

        // shutdown old workers
        for (worker_id, pid) in old_workers {
            info!("shutting down old worker {worker_id}");
            let _ = signal::kill(pid, Signal::SIGTERM);
        }

        Ok(())
    }

    fn handle_child_exit(&self) {
        warn!("worker process exited");
    }

    fn monitor_workers(&self) {}
}
