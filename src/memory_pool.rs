use parking_lot::Mutex;
use std::collections::VecDeque;

pub struct MemoryPool {
    buffers: Mutex<VecDeque<Vec<u8>>>,
    buffer_size: usize,
    max_buffers: usize,
}

impl MemoryPool {
    pub fn init() -> Self {
        Self {
            buffers: Mutex::new(VecDeque::new()),
            buffer_size: 8192, // 8KB
            max_buffers: 1000,
        }
    }

    pub fn get_buffer(&self) -> Vec<u8> {
        let mut buffers = self.buffers.lock();
        buffers
            .pop_front()
            .unwrap_or_else(|| Vec::with_capacity(self.buffer_size))
    }

    pub fn return_buffer(&self, mut buffer: Vec<u8>) {
        let mut buffers = self.buffers.lock();

        if buffers.len() < self.max_buffers {
            buffer.clear();
            buffer.shrink_to(self.buffer_size);
            buffers.push_back(buffer);
        }
    }
}
