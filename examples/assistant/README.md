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
from what the model or a tool says it will do. The host remembers which private paths and
untrusted sources contributed content to the conversation. Those choices live in
[`policy.rego`](policy.rego); [`wasi.rego`](wasi.rego) contains the mechanics that turn each raw
call into one request with an action such as `fs.read`, `fs.write`, `fs.list`, `http.request` or
`tool.call`. An `allow` names the exact file, host or tool access available to a component. A
matching `deny` wins first. An `ask` can add consent only to a call that is otherwise allowed,
and every call without an `allow` is denied.

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
  · make a request to https://example.com/   allowed
assistant> Example.com is a placeholder domain intended for use in documentation examples without needing permission, and it advises not to rely on it for testing or monitoring.

you> Read private/diary.md and summarize it in one sentence.
  · call notes-tools.read-note   allowed
  · read private/diary.md   allowed
  ? This conversation includes private/diary.md. Let it make a request to <model-host>? [y/N] y
assistant> The private diary notes that an unreleased prototype demonstration is scheduled for Friday.

you> Fetch https://example.com again and tell me its title.
  · call web-tools.fetch   allowed
  ? This conversation includes private/diary.md. Let it make a request to https://example.com/? [y/N] n
  ✗ make a request to https://example.com/ denied — conversation includes private/diary.md
assistant> I couldn't re-fetch the page — the request was denied by policy this time. Based on my earlier successful fetch, its title was "Example Domain".
```

The first fetch went through without a question. After you allowed the diary to reach the model,
the conversation included `private/diary.md`, so the later fetch named its full URL and waited
for permission. A refusal is remembered for the rest of that turn, so a model retry is denied
without asking again. If you instead refuse the agent's request to the model when it would first
receive the diary, the failed turn and its user message are dropped; the next unrelated turn is
clean and asks nothing.

This is the rule that asked:

```rego
ask contains reason if {
	request.action == "http.request"
	not private_data_host(request)
	some path in input.label.private
	reason := sprintf("conversation includes %s", [path])
}
```

In words: before any component sends while the conversation holds private content, ask first.
`PRIVATE_DATA_HOSTS` applies to any component's request to a listed host, without asking; the
component must still be permitted to contact that host by `allowed_hosts`. A yes for the agent's
model request is remembered for the session. A yes for a tool request or write authorizes only
that call, after the policy checks it again.

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
  · make a request to https://example.com/   allowed
assistant> The title of https://example.com is **Example Domain**.

you> Replace drafts/timing.md with "after the fetch".
  · call notes-tools.write-note   allowed
  ? This conversation includes content from example.com. Let it write drafts/timing.md? [y/N] y
assistant> Replaced drafts/timing.md with "after the fetch".
```

Writing a draft is ordinary at the start. After the agent has read a web page, the same write
waits for you, because the page might be what's asking for it. Here the request came from you,
so you said yes. If a page had slipped in the instruction, you'd see the same question and could
say no.

```rego
ask contains reason if {
	writing(request)
	some source in input.label.untrusted
	reason := sprintf("conversation includes content from %s", [source])
}
```

## Access control

Some rules don't depend on the conversation at all. Here, your passwords file is never read, and
the agent only writes inside `drafts/`.

```text
you> Read private/passwords.md.
  · call notes-tools.read-note   allowed
  ✗ read private/passwords.md denied — private/passwords.md is secret
assistant> I wasn't able to read `private/passwords.md` — the system returned an access error, meaning that note is restricted and I don't have permission to open it. […]

you> Write "hello" to private/output.md.
  · call notes-tools.write-note   allowed
  ✗ write private/output.md denied — private/output.md is outside drafts/
assistant> I wasn't able to write to `private/output.md` — the write was rejected with an access error, so that path is restricted for me. […]
```

There's no question this time. The passwords rule is an explicit `deny`, which always wins over
`ask` and over anything you approved earlier. The other call falls through to denial because the
only write `allow` is for `notes-tools` inside `drafts`:

```rego
deny contains sprintf("%s is secret", [request.path]) if {
	reading(request)
	request.path in secret_paths
}

allow if {
	request.action == "fs.write"
	request.caller == "notes-tools"
	under(request.path, drafts_dir)
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
  ✗ make a request to https://collector.example/collect denied — weather-tools may not send to collector.example
  · make a request to https://api.open-meteo.com/v1/forecast?latitude=42.3601&longitude=-71.0589&current=temperature_2m,weather_code&temperature_unit=celsius   allowed
assistant> Boston is overcast with a temperature of 13.9°C (about 57°F) — a cool, cloudy day.
```

Nothing in the tool's interface hints at what it does, and the model has no way to know. The
policy catches it anyway, because it doesn't go by what a component says it does: it sees each
file the component opens and each host it contacts. The diary read and the hidden send are both
refused, and the real weather request goes through: the answer is the actual forecast.

```rego
allow if {
	request.action == "fs.read"
	request.caller == "notes-tools"
}

allow if {
	request.action == "http.request"
	host_allowed(allowed_hosts[request.caller], request.host)
}
```

The first rule allows only `notes-tools` to read notes; the default denial names the component and
path for every other attempt. Writes and directory listings have their own allow rules. The second
rule allows network access only when the host appears for that component in `allowed_hosts`: the
agent may contact only the configured model host, `web-tools` may contact any host,
`weather-tools` only `api.open-meteo.com`, and `notes-tools` none. This answers which hosts a
component may contact; `PRIVATE_DATA_HOSTS` separately answers whether any component's otherwise
allowed request to a listed host may carry private data without asking.

## Try changing the policy

The policy is read when the agent starts, so an edit takes effect the next time you run it. A
few things to try:

- add `"research.md"` to `secret_paths`, then ask for a summary of it;
- delete the second `ask` rule, then repeat the prompt injection steps;
- add `"collector.example"` to `weather-tools` in `allowed_hosts`, and watch the policy let its
  hidden send through.

## What this example doesn't do

It keeps things simple on purpose:

- **Sensitivity covers whole turns.** Parallel tool calls in one turn share what the policy knows;
  the example does not track individual values within a turn.
- **Allowed destinations can still receive encoded data.** A tool may encode data into a request
  to one of its allowed hosts while the conversation holds nothing marked private.
- **Listings and metadata are not labelled.** Only opened file paths contribute sensitivity to
  the conversation; directory entries and metadata do not.
- **Untrusted sends ask.** A stricter policy would refuse model-chosen sends after untrusted
  content instead of asking the user.
- **Sensitive history only accumulates.** The conversation cannot be made clean again except by
  restarting it.
- **Model consent lasts for the session.** One approval for the agent's model request is remembered
  until the process exits.
- **Decisions are allow, ask or deny.** Middleware could also change a call, for example to
  redact text, but this example doesn't.
- **The policy sees real paths.** Before deciding, the host cleans up each path and follows
  symlinks inside the notes folder, so `./private/../private/diary.md`, or a symlink to the
  diary, is still the diary.
- **Subfolder handles lose context.** An open relative to a subfolder descriptor is judged from
  the root reported by the WASI gate, a framework limitation tracked separately.

## Settings

[`.env.example`](.env.example) lists them:

- `OPENAI_BASE_URL`: the API's base URL, including its prefix but not `/chat/completions`;
- `OPENAI_API_KEY` and `OPENAI_MODEL`;
- `PRIVATE_DATA_HOSTS`: hosts that may receive private notes from any component's request without
  asking, comma-separated (with the port if the URL has one).

The policy's `allowed_hosts` says which hosts each component may contact at all;
`PRIVATE_DATA_HOSTS` is the separate deployment setting for which hosts may receive private data
from any component's request without asking.

Environment variables override the file. An empty line or Ctrl-D ends the chat.
