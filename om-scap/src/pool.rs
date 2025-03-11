use std::collections::VecDeque;

#[derive(Debug)]
pub struct Buffer {
    version: usize,
    pub data: Vec<u8>,
}

impl Buffer {
    #[inline]
    pub fn new(version: usize) -> Self {
        Self {
            version,
            data: vec![0; version],
        }
    }

    #[inline]
    pub fn new_with_data(version: usize, data: Vec<u8>) -> Self {
        Self {
            version,
            data,
        }
    }
}

pub struct FramePool {
    /// Version, aka. the size of the buffers
    current_version: usize,
    queue: VecDeque<Vec<u8>>,
}

impl FramePool {
    pub fn new() -> Self {
        Self {
            current_version: 0,
            queue: VecDeque::with_capacity(15),
        }
    }

    pub fn get(&mut self, version: usize) -> Buffer {
        if self.queue.is_empty() {
            self.current_version = version;
            Buffer::new(version)
        } else if self.current_version != version {
            self.queue.clear();
            self.current_version = version;
            Buffer::new(version)
        } else {
            match self.queue.pop_back() {
                Some(data) => Buffer::new_with_data(version, data),
                None => Buffer::new(version),
            }
        }
    }

    #[inline]
    pub fn give_back(&mut self, buffer: Buffer) {
        if buffer.version == self.current_version {
            let _ = self.queue.push_back(buffer.data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_give_back() {
        let mut pool = FramePool::new();

        let buffer = pool.get(100);
        assert_eq!(buffer.data.capacity(), 100);

        pool.give_back(buffer);
        assert_eq!(pool.queue.len(), 1);
    }

    #[test]
    fn different_version() {
        let mut pool = FramePool::new();

        let b1 = pool.get(100);
        let b2 = pool.get(100);
        let b3 = pool.get(100);
        pool.give_back(b1);
        pool.give_back(b2);
        pool.give_back(b3);
        assert_eq!(pool.queue.len(), 3);

        let b4 = pool.get(250);
        assert!(pool.queue.is_empty());

        pool.give_back(b4);
        assert_eq!(pool.queue.len(), 1);
    }
}
