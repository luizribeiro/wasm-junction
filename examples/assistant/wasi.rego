package assistant

# Arguments use a fixed text form: lists use [a, b], tuples use (a, b), records use
# {field: value}, flags use {read, write}, variants use case(value), options use the
# value or none, results use ok(value) or error(value), and handles use <resource>,
# <stream>, or <future>. Filesystem paths are resolved before the policy sees them.

request := normalize(input.event)
history_requests := [normalize(event) | some event in input.history]

default decision := {"verdict": "allow", "reasons": set(), "effect": "", "consent": set(), "remember": false}

decision := {"verdict": "deny", "reasons": deny, "effect": effect(request), "consent": consent, "remember": remember(request)} if count(deny) > 0
else := {"verdict": "ask", "reasons": pending_ask, "effect": effect(request), "consent": consent, "remember": remember(request)} if count(pending_ask) > 0
else := {"verdict": "allow", "reasons": set(), "effect": effect(request), "consent": consent, "remember": remember(request)} if true

consent contains consent_key(request, reason) if some reason in ask

pending_ask contains reason if {
	some reason in ask
	key := consent_key(request, reason)
	not key in input.approved
	not key in input.refused
}

normalize(event) := {"caller": event.caller, "action": "fs.write", "path": event.arguments[2]} if {
	open_at(event)
	mutating_open(event)
} else := {"caller": event.caller, "action": "fs.read", "path": event.arguments[2]} if open_at(event)
else := {"caller": event.caller, "action": "http.request", "host": event.arguments[3], "url": sprintf("%s://%s%s", [lower(event.arguments[2]), event.arguments[3], event.arguments[4]])} if http_send(event)
else := {"caller": event.caller, "action": "tool.call", "tool": sprintf("%s.%s", [event.callee, event.function])} if startswith(event.interface, "example:assistant/")
else := {"caller": event.caller, "action": "plumbing"} if plumbing(event)
else := {"caller": event.caller, "action": "other", "call": call_name(event)}

open_at(event) if {
	event.interface == "wasi:filesystem/types@0.3.0"
	event.function == "[method]descriptor.open-at"
}

mutating_open(event) if "write" in flag_set(event.arguments[4])
mutating_open(event) if "create" in flag_set(event.arguments[3])
mutating_open(event) if "truncate" in flag_set(event.arguments[3])

http_send(event) if {
	event.interface == "wasi:http/client@0.3.0"
	event.function == "send"
}

plumbing(event) if call_name(event) in plumbing_calls

plumbing_calls := {
	"wasi:cli/environment@0.3.0.get-environment",
	"wasi:clocks/monotonic-clock@0.3.0.get-resolution",
	"wasi:clocks/monotonic-clock@0.3.0.now",
	"wasi:clocks/monotonic-clock@0.3.0.wait-for",
	"wasi:clocks/monotonic-clock@0.3.0.wait-until",
	"wasi:clocks/system-clock@0.3.0.get-resolution",
	"wasi:clocks/system-clock@0.3.0.now",
	"wasi:filesystem/preopens@0.3.0.get-directories",
	"wasi:filesystem/types@0.3.0.[drop]descriptor",
	"wasi:filesystem/types@0.3.0.[method]descriptor.read-via-stream",
	"wasi:filesystem/types@0.3.0.[method]descriptor.write-via-stream",
	"wasi:http/types@0.3.0.[constructor]fields",
	"wasi:http/types@0.3.0.[constructor]request-options",
	"wasi:http/types@0.3.0.[drop]fields",
	"wasi:http/types@0.3.0.[drop]request",
	"wasi:http/types@0.3.0.[drop]request-options",
	"wasi:http/types@0.3.0.[drop]response",
	"wasi:http/types@0.3.0.[method]fields.append",
	"wasi:http/types@0.3.0.[method]request-options.set-between-bytes-timeout",
	"wasi:http/types@0.3.0.[method]request-options.set-connect-timeout",
	"wasi:http/types@0.3.0.[method]request-options.set-first-byte-timeout",
	"wasi:http/types@0.3.0.[method]request.set-authority",
	"wasi:http/types@0.3.0.[method]request.set-method",
	"wasi:http/types@0.3.0.[method]request.set-path-with-query",
	"wasi:http/types@0.3.0.[method]request.set-scheme",
	"wasi:http/types@0.3.0.[method]response.get-status-code",
	"wasi:http/types@0.3.0.[static]request.new",
	"wasi:http/types@0.3.0.[static]response.consume-body",
	"wasi:random/insecure-seed@0.3.0.get-insecure-seed",
	"wasi:random/insecure@0.3.0.get-insecure-random-bytes",
	"wasi:random/insecure@0.3.0.get-insecure-random-u64",
	"wasi:random/random@0.3.0.get-random-bytes",
	"wasi:random/random@0.3.0.get-random-u64",
}

flag_set(flags) := {flag | some flag in split(trim(flags, "{}"), ", "); flag != ""}

call_name(event) := sprintf("%s.%s", [event.interface, event.function])

reading(item) if item.action in {"fs.read", "fs.write"}
writing(item) if item.action == "fs.write"

effect(item) := sprintf("write %s", [item.path]) if writing(item)
else := sprintf("read %s", [item.path]) if reading(item)
else := sprintf("make a request to %s", [item.host]) if {
	item.action == "http.request"
	item.caller == "agent"
} else := sprintf("make a request to %s", [item.url]) if item.action == "http.request"
else := sprintf("call %s", [item.tool]) if {
	item.action == "tool.call"
	item.caller != "host"
}
else := sprintf("call %s", [item.call]) if item.action == "other"
else := ""
