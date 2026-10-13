use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct Console {
    input: Arc<Mutex<Box<dyn BufRead + Send>>>,
    output: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Console {
    pub(crate) fn new(
        input: impl BufRead + Send + 'static,
        output: impl Write + Send + 'static,
    ) -> Self {
        Self {
            input: Arc::new(Mutex::new(Box::new(input))),
            output: Arc::new(Mutex::new(Box::new(output))),
        }
    }

    pub(crate) async fn read_line(&self) -> std::io::Result<String> {
        let input = self.input.clone();
        tokio::task::spawn_blocking(move || {
            let mut line = String::new();
            input
                .lock()
                .map_err(|_| std::io::Error::other("console input lock poisoned"))?
                .read_line(&mut line)?;
            Ok(line)
        })
        .await
        .map_err(std::io::Error::other)?
    }

    pub(crate) fn print(&self, text: &str) -> std::io::Result<()> {
        let mut output = self
            .output
            .lock()
            .map_err(|_| std::io::Error::other("console output lock poisoned"))?;
        output.write_all(text.as_bytes())?;
        output.flush()
    }

    pub(crate) fn line(&self, text: &str) -> std::io::Result<()> {
        self.print(&format!("{text}\n"))
    }
}
