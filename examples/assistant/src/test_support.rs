use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct SharedOutput(Arc<Mutex<Vec<u8>>>);

impl SharedOutput {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for SharedOutput {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
