use nix::sys::signal::{self, SigHandler, Signal};
use std::sync::atomic::{AtomicU32, Ordering};

static RECEIEVED_SIGNAL: AtomicU32 = AtomicU32::new(0);

extern "C" fn signal_handler(sig: i32) {
    RECEIEVED_SIGNAL.store(sig as u32, Ordering::SeqCst);
}

pub struct SignalHandler;

impl SignalHandler {
    pub fn new() -> Self {
        Self
    }

    pub fn setup(&self) -> Result<(), Box<dyn std::error::Error>> {
        unsafe {
            signal::signal(Signal::SIGTERM, SigHandler::Handler(signal_handler))?;
            signal::signal(Signal::SIGINT, SigHandler::Handler(signal_handler))?;
            signal::signal(Signal::SIGHUP, SigHandler::Handler(signal_handler))?;
            signal::signal(Signal::SIGCHLD, SigHandler::Handler(signal_handler))?;
        }
        Ok(())
    }

    pub fn check_signals(&self) -> Option<Signal> {
        let sig = RECEIEVED_SIGNAL.swap(0, Ordering::SeqCst);

        if sig != 0 {
            Signal::try_from(sig as i32).ok()
        } else {
            None
        }
    }
}
