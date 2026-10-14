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
        self.write(text, false, false)
    }

    pub(crate) fn line(&self, text: &str) -> std::io::Result<()> {
        self.write(text, true, false)
    }

    pub(crate) fn reply(&self, text: &str) -> std::io::Result<()> {
        self.write(text, true, true)
    }

    fn write(&self, text: &str, newline: bool, keep_line_breaks: bool) -> std::io::Result<()> {
        let mut output = self
            .output
            .lock()
            .map_err(|_| std::io::Error::other("console output lock poisoned"))?;
        output.write_all(escaped(text, keep_line_breaks).as_bytes())?;
        if newline {
            output.write_all(b"\n")?;
        }
        output.flush()
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
        self.print(&prompt(&request))?;
        let answer = self.read_line().await?;
        let allowed = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        let _ = request.answer.send(allowed);
        Ok(())
    }
}

fn escaped(text: &str, keep_line_breaks: bool) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        if keep_line_breaks && character == '\n' {
            output.push(character);
        } else if character.is_control() || is_format(character) {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

fn is_format(character: char) -> bool {
    matches!(
        character,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{13440}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

fn prompt(request: &ConsentRequest) -> String {
    format!(
        "  ? This {}. Let it {}? [y/N] ",
        request.reason, request.effect,
    )
}
use tokio::sync::{mpsc, oneshot};

pub(crate) struct ConsentRequest {
    pub(crate) effect: String,
    pub(crate) reason: String,
    pub(crate) answer: oneshot::Sender<bool>,
}

#[cfg(test)]
mod tests {
    use super::{ConsentRequest, Console, escaped, prompt};
    use crate::test_support::SharedOutput;
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

    #[test]
    fn renders_policy_reason_and_effect() {
        let (answer, _reply) = oneshot::channel();
        let request = ConsentRequest {
            effect: "make a request to https://example.com/".into(),
            reason: "conversation includes private/diary.md".into(),
            answer,
        };
        assert_eq!(
            prompt(&request),
            "  ? This conversation includes private/diary.md. Let it make a request to https://example.com/? [y/N] "
        );
        let (answer, _reply) = oneshot::channel();
        let request = ConsentRequest {
            effect: "write drafts/timing.md".into(),
            reason: "conversation includes content from example.com".into(),
            answer,
        };
        assert_eq!(
            prompt(&request),
            "  ? This conversation includes content from example.com. Let it write drafts/timing.md? [y/N] "
        );
    }

    #[test]
    fn escapes_control_characters_in_prompts() {
        let (answer, _reply) = oneshot::channel();
        let request = ConsentRequest {
            effect: "make a request to https://example.com/\rforged".into(),
            reason: "conversation includes private/diary.md".into(),
            answer,
        };
        assert_eq!(
            escaped(&prompt(&request), false),
            "  ? This conversation includes private/diary.md. Let it make a request to https://example.com/\\rforged? [y/N] "
        );
    }

    #[test]
    fn escapes_invisible_format_characters() {
        assert_eq!(
            escaped("private/\u{200b}note\u{202e}dm", false),
            "private/\\u{200b}note\\u{202e}dm"
        );
    }

    #[test]
    fn keeps_reply_line_breaks_readable() {
        let output = SharedOutput::default();
        let console = Console::new(Cursor::new(Vec::<u8>::new()), output.clone());
        console.reply("assistant> first\nsecond\u{202e}").unwrap();
        assert_eq!(output.text(), "assistant> first\nsecond\\u{202e}\n");
    }
}
