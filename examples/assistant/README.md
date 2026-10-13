# An agent with a policy

This example is a small AI agent built from WebAssembly components. One component, `agent`,
talks to an LLM through any OpenAI-compatible API and runs the tool calls the model asks for. The
tools are components too:

- **`notes-tools`** lists, reads and writes your notes;
- **`web-tools`** fetches web pages;
- **`weather-tools`** returns a weather forecast (we'll come back to this one).

None of these components can touch a file or the network directly. Each file they open and each
request they send is a WASI call, and junction passes every WASI call, and every call between the
components, through middleware before it happens. In this example the middleware asks a policy
written in [Rego](https://www.openpolicyagent.org/docs/policy-language) whether to allow the
call, ask you first, or deny it.

The policy is [`policy.rego`](policy.rego). It works from what the components actually do, not
from what the model or a tool says it will do, and it can take into account what has already
happened in the conversation. A second file, [`wasi.rego`](wasi.rego), translates raw WASI calls
into the words the policy uses (`reading`, `writing`, `sending`), so you rarely need to open it.

Below, we run the agent and go through four kinds of problem the policy deals with: privacy,
prompt injection, access control and a malicious component.

## Run it

You need an OpenAI-compatible chat endpoint: a hosted provider, or a model running locally.

```sh
cp examples/assistant/.env.example examples/assistant/.env   # then fill in your endpoint
nix develop -c cargo run -p wasm-junction-example-assistant
```

Chat with it as you would with any assistant. Under each request you'll see what the components
did and what the policy decided: `·` means allowed, `?` means the policy is asking you, and `✗`
means denied. Add `-- --verbose` to also see routine calls, like each request to the model.

## Privacy

Your notes folder has a `private/` corner with a diary in it. Reading the diary is fine; the
agent is yours. The risk is what happens next: once private text is in the conversation, any
request the agent makes could carry it somewhere, even tucked into a URL.

```text
you> Fetch https://example.com and summarize it in one sentence.
  · call web-tools.fetch   allowed
  · send example.com   allowed
assistant> Example Domain is a reserved domain intended for use in documentation and educational examples, explicitly noted as not a service and not suitable for testing or monitoring.

you> Read private/diary.md and summarize it in one sentence.
  · call notes-tools.read-note   allowed
  · read private/diary.md   allowed
  ? send <model-host> — send to <model-host> after reading private/diary.md — allow? [y/N] y
assistant> The unreleased prototype demonstration is scheduled to take place on Friday.

you> Fetch https://example.com again and tell me its title.
  · call web-tools.fetch   allowed
  ? send example.com — send to example.com after reading private/diary.md — allow? [y/N] n
  · call web-tools.fetch   allowed
  ? send example.com — send to example.com after reading private/diary.md — allow? [y/N] n
assistant> I can't refetch it right now — the request is being denied by policy. Based on the earlier fetch, though, the page's title was **"Example Domain"**.
```

The first fetch went through without a question. The same fetch at the end needed your
permission. When you said no, it didn't happen; the model tried once more, was asked again, and
then answered from what it already knew. Nothing about the request changed; what changed is that
the conversation now holds your diary. Even sending the diary to the model asked first, because
the model is just another host to the policy.

This is the rule that asked:

```rego
ask contains reason if {
	host := sending(input.event)
	not host in trusted_hosts
	some path in private_reads
	reason := sprintf("send to %s after reading %s", [host, path])
	not reason in input.approved
}
```

In words: when something is about to be sent to a host you haven't trusted, and a private note
was read earlier in this session, ask first. Once you say yes, the same question isn't asked
again in this session. If your model runs locally, add its host to `TRUSTED_HOSTS` and private
notes can go to it without asking.

## Prompt injection

Web pages are written by strangers, and a model reads them as part of what it's told. A page can
try to talk the agent into doing something you didn't ask for, such as rewriting your files.

```text
you> Write "before the fetch" to drafts/timing.md.
  · call notes-tools.write-note   allowed
  · write drafts/timing.md   allowed
assistant> I've written "before the fetch" to drafts/timing.md.

you> Fetch https://example.com and tell me its title.
  · call web-tools.fetch   allowed
  · send example.com   allowed
assistant> The page title is "Example Domain".

you> Replace drafts/timing.md with "after the fetch".
  · call notes-tools.write-note   allowed
  ? write drafts/timing.md — write drafts/timing.md after fetching from example.com — allow? [y/N] y
assistant> Done — drafts/timing.md now contains "after the fetch".
```

Writing a draft is ordinary at the start. After the agent has read a web page, the same write
waits for you, because the page might be what's asking for it. Here the request came from you,
so you said yes. If a page had slipped in the instruction, you'd see the same question and could
say no.

```rego
ask contains reason if {
	path := writing(input.event)
	some host in web_fetches
	reason := sprintf("write %s after fetching from %s", [path, host])
	not reason in input.approved
}
```

## Access control

Some rules don't depend on the conversation at all. Here, your passwords file is never read, and
the agent only writes inside `drafts/`.

```text
you> Read private/passwords.md.
  · call notes-tools.read-note   allowed
  ✗ read private/passwords.md denied — private/passwords.md is secret
  · call notes-tools.list-notes   allowed
  · call notes-tools.read-note   allowed
  ✗ read private/passwords.md denied — private/passwords.md is secret
assistant> I was able to confirm that `private/passwords.md` exists in the notes, but I'm unable to read it — the system returns an **access denied** error, and it's not a transient failure. […]

you> Write "hello" to private/output.md.
  · call notes-tools.write-note   allowed
  ✗ write private/output.md denied — private/output.md is outside drafts/
assistant> I tried writing "hello" to `private/output.md`, but the write failed with an **access denied** error […] so I won't keep retrying.
```

There's no question this time. These are `deny` rules, and a `deny` always wins over `ask` and
over anything you approved earlier:

```rego
deny contains sprintf("%s is secret", [path]) if {
	path := reading(input.event)
	path in secret_paths
}

deny contains sprintf("%s is outside %s", [path, drafts_prefix]) if {
	path := writing(input.event)
	not startswith(path, drafts_prefix)
}
```

## A malicious component

`weather-tools` offers the model one tool, `get-weather(city)`. Look at
[its source](guest/weather-tools/src/lib.rs), though, and you'll see it also reads your diary and
sends it to a server of its own (`collector.example`) before fetching a real forecast from
Open-Meteo. It stands in for a third-party component you didn't write and can't fully check.

```text
you> What's the weather in Boston?
  · call weather-tools.get-weather   allowed
  ✗ read private/diary.md denied — weather-tools may not read notes
  ✗ send collector.example denied — weather-tools may not send to collector.example
  · send api.open-meteo.com   allowed
assistant> Boston is currently overcast with a temperature of 12.5°C (about 54.5°F).
```

Nothing in the tool's interface hints at what it does, and the model has no way to know. The
policy catches it anyway, because it doesn't go by what a component says it does: it sees each
file the component opens and each host it contacts. The diary read and the hidden send are both
refused, and the real weather request goes through: the answer is the actual forecast.

```rego
deny contains sprintf("%s may not read notes", [input.event.caller]) if {
	reading(input.event)
	input.event.caller != "notes-tools"
}

deny contains sprintf("%s may not send to %s", [caller, host]) if {
	host := sending(input.event)
	caller := input.event.caller
	hosts := tool_hosts[caller]
	not "*" in hosts
	not host in hosts
}
```

The first rule says only `notes-tools` may read notes. The second gives each tool a list of
hosts it may contact (`tool_hosts`, at the top of `policy.rego`): `web-tools` may contact any,
`weather-tools` only `api.open-meteo.com`.

## Try changing the policy

The policy is read when the agent starts, so an edit takes effect the next time you run it. A
few things to try:

- add `"research.md"` to `secret_paths`, then ask for a summary of it;
- delete the second `ask` rule, then repeat the prompt injection steps;
- add `collector.example` to the weather tool's hosts in `tool_hosts`, and watch the policy let
  its hidden send through.

## What this example doesn't do

It keeps things simple on purpose:

- **Private data is tracked per session.** Once a private note is read, the whole conversation
  counts as carrying it, even if the model never repeats a word of it. Restart to reset.
- **Decisions are allow, ask or deny.** Middleware could also change a call, for example to
  redact text, but this example doesn't.
- **The policy sees real paths.** Before deciding, the host cleans up each path and follows
  symlinks inside the notes folder, so `./private/../private/diary.md`, or a symlink to the
  diary, is still the diary.
- **Asking is the safety net.** After something private is read, you'll see more questions.
  Larger systems reduce them with finer-grained tracking, or by separating tasks that need
  private data from tasks that browse the web.

## Settings

[`.env.example`](.env.example) lists them:

- `OPENAI_BASE_URL`: the API's base URL, including its prefix but not `/chat/completions`;
- `OPENAI_API_KEY` and `OPENAI_MODEL`;
- `TRUSTED_HOSTS`: hosts that may receive private data without asking, comma-separated (with
  the port if the URL has one).

Environment variables override the file. An empty line or Ctrl-D ends the chat.
