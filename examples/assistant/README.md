# Research assistant

The policy sees what every component actually does—files and network—and decides from what
already happened, even when a tool or the model misbehaves.

Four WebAssembly components make up the application:

- `agent` calls an OpenAI-compatible chat endpoint and runs its tool loop;
- `notes-tools` lists, reads, and writes notes through WASI filesystem calls;
- `web-tools` fetches pages through outgoing WASI HTTP;
- `weather-tools` claims to return a forecast while secretly trying unrelated work.

Middleware turns every component and WASI call into a generic event for Rego. The outputs below
are selected lines from configured runs; only the model and loopback hosts were replaced.

## The session remembers

```text
you> Fetch http://127.0.0.1:<port>/research and summarize it in one sentence.
  · call web-tools.fetch   allowed
  · send 127.0.0.1:<port>   allowed
assistant> WebAssembly components are portable, self-contained units defined by explicit interfaces.

you> Read private/diary.md and summarize it in one sentence.
  · call notes-tools.read-note   allowed
  · read private/diary.md   allowed
  ? send <model-host> — send to <model-host> after reading private/diary.md — allow? [y/N] y
assistant> The unreleased prototype demonstration is scheduled to take place on Friday.

you> Fetch http://127.0.0.1:<port>/research again and summarize it.
  · call web-tools.fetch   allowed
  ? send 127.0.0.1:<port> — send to 127.0.0.1:<port> after reading private/diary.md — allow? [y/N] n
assistant> I couldn't fetch that URL — the request was denied by policy, so I can't summarize it.
```

The diary read stays in the session history. Sending it to the model needed approval, and the
later page fetch needed its own approval because it contacted a different host; saying no stopped
that request while the chat continued.

This is the rule that asked before both sends:

```rego
ask contains reason if {
	host := sending(input.event)
	not host in trusted_hosts
	some path in private_reads
	reason := sprintf("send to %s after reading %s", [host, path])
	not reason in input.approved
}
```

## Writes after a fetch

```text
you> Write "before the fetch" to notes/timing.md.
  · call notes-tools.write-note   allowed
  · write notes/timing.md   allowed
assistant> Done — "before the fetch" has been written to notes/timing.md.

you> Fetch http://127.0.0.1:<port>/research.
  · call web-tools.fetch   allowed
  · send 127.0.0.1:<port>   allowed
assistant> Fetched successfully. The content at http://127.0.0.1:<port>/research is:

> WebAssembly components are portable units with explicit interfaces.

you> Replace notes/timing.md with "after the fetch". Try once.
  · call notes-tools.write-note   allowed
  ? write notes/timing.md — write notes/timing.md after fetching from 127.0.0.1:<port> — allow? [y/N] y
assistant> notes/timing.md has been replaced with "after the fetch" (verified by reading it back).
```

A write is ordinary at the start of a session. Once a page has been fetched, the page could be
steering the model, so the same write waits for the user's approval.

This is the rule that asked before the later write:

```rego
ask contains reason if {
	path := writing(input.event)
	some host in web_fetches
	reason := sprintf("write %s after fetching from %s", [path, host])
	not reason in input.approved
}
```

## Plain rules

```text
you> Try once to read private/passwords.md. If it fails, explain why without retrying.
  · call notes-tools.read-note   allowed
  ✗ read private/passwords.md denied — private/passwords.md is secret
assistant> The single attempt failed with an **access** error.

you> Try once to write "hello" to private/output.md. If it fails, do not retry.
  · call notes-tools.write-note   allowed
  ✗ write private/output.md denied — private/output.md is outside notes/
assistant> The write to private/output.md failed once with an **access** error, so I stopped without retrying.
```

These rules have no judgment call to delegate: the secret is unreadable and writes are confined
to `notes/`, so both operations are denied without asking.

These are the rules that denied those operations:

```rego
deny contains sprintf("%s is secret", [path]) if {
	path := reading(input.event)
	path in secret_paths
}

deny contains sprintf("%s is outside %s", [path, notes_prefix]) if {
	path := writing(input.event)
	not startswith(path, notes_prefix)
}
```

## What tools really do

```text
you> Use get_weather to tell me the weather in Boston.
  · call weather-tools.get-weather   allowed
  ✗ read private/diary.md denied — weather-tools may not read notes
  ✗ send localhost:<port> denied — weather-tools may not send to localhost:<port>
  · send 127.0.0.1:<port>   allowed
assistant> Here's the weather in Boston according to the forecast tool:

- **Conditions:** Sunny
- **Temperature:** 21°C

Note: the tool's response indicated this is a made-up/example forecast, so I'd recommend checking
a live weather source for actual conditions.
```

`get-weather` looks innocent, but its component also tries to read the diary and contact a
collector before calling its weather host. The policy sees every file and host a component
touches, whatever its interface claims, and blocks both hidden side effects.

These are the rules that exposed the tool's hidden behavior:

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

## The policy

The complete `policy.rego` is:

```rego
package assistant

secret_paths := {"private/passwords.md"}
private_prefix := "private/"
notes_prefix := "notes/"
tool_hosts := {
	"web-tools": {"*"},
	"weather-tools": {data.weather_host},
}

deny contains sprintf("%s may not read notes", [input.event.caller]) if {
	reading(input.event)
	input.event.caller != "notes-tools"
}

deny contains sprintf("%s is secret", [path]) if {
	path := reading(input.event)
	path in secret_paths
}

deny contains sprintf("%s is outside %s", [path, notes_prefix]) if {
	path := writing(input.event)
	not startswith(path, notes_prefix)
}

deny contains sprintf("%s may not send to %s", [caller, host]) if {
	host := sending(input.event)
	caller := input.event.caller
	hosts := tool_hosts[caller]
	not "*" in hosts
	not host in hosts
}

ask contains reason if {
	host := sending(input.event)
	not host in trusted_hosts
	some path in private_reads
	reason := sprintf("send to %s after reading %s", [host, path])
	not reason in input.approved
}

ask contains reason if {
	path := writing(input.event)
	some host in web_fetches
	reason := sprintf("write %s after fetching from %s", [path, host])
	not reason in input.approved
}
```

The companion `wasi.rego` supplies the fixed decision entrypoint, session history, and exact
translation of raw WASI calls into reads, writes, and sends.

## Run it

From the repository root, copy the settings template and replace its placeholder values:

```sh
cp examples/assistant/.env.example examples/assistant/.env
nix develop -c cargo run -p wasm-junction-example-assistant
```

The endpoint may be any OpenAI-compatible endpoint. `OPENAI_BASE_URL` includes its API prefix
but not `/chat/completions`. Existing environment variables override `.env`. `TRUSTED_HOSTS` is
a comma-separated list of hosts that may receive private data without asking; include the port
when the URL has one.

Pass `--verbose` after `--` to also show routine model sends and directory opens:

```sh
nix develop -c cargo run -p wasm-junction-example-assistant -- --verbose
```

Enter an empty line or press Ctrl-D to exit. To change the policy, edit `policy.rego` and rerun;
it is parsed once at startup. Taint and exact approvals last only for the current chat session.
