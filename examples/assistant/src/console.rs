use std::future::Future;
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

    pub(crate) async fn wait<T>(
        &self,
        future: impl Future<Output = T>,
        requests: &mut mpsc::UnboundedReceiver<ConsentRequest>,
    ) -> std::io::Result<T> {
        tokio::pin!(future);
        loop {
            tokio::select! {
                result = &mut future => return Ok(result),
                Some(request) = requests.recv() => self.answer(request).await?,
            }
        }
    }

    async fn answer(&self, request: ConsentRequest) -> std::io::Result<()> {
        self.print(&format!(
            "  ? {} — {} — allow? [y/N] ",
            request.effect, request.reason,
        ))?;
        let answer = self.read_line().await?;
        let allowed = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        let _ = request.answer.send(allowed);
        Ok(())
    }
}
use tokio::sync::{mpsc, oneshot};

pub(crate) struct ConsentRequest {
    pub(crate) effect: String,
    pub(crate) reason: String,
    pub(crate) answer: oneshot::Sender<bool>,
}

#[cfg(test)]
mod tests {
    use super::{ConsentRequest, Console};
    use std::io::Cursor;
    use tokio::sync::{mpsc, oneshot};

    #[tokio::test(flavor = "current_thread")]
    async fn waits_and_answers_consent() {
        let console = Console::new(Cursor::new("y\n"), std::io::sink());
        let (_sender, mut requests) = mpsc::unbounded_channel();
        assert_eq!(console.wait(async { 7 }, &mut requests).await.unwrap(), 7);
        let (answer, reply) = oneshot::channel();
        console
            .answer(ConsentRequest {
                effect: "write note".into(),
                reason: "after fetch".into(),
                answer,
            })
            .await
            .unwrap();
        assert!(reply.await.unwrap());
    }
}
