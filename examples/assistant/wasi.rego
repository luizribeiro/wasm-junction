package assistant

# Arguments use a fixed text form: lists use [a, b], tuples use (a, b), records use
# {field: value}, flags use {read, write}, variants use case(value), options use the
# value or none, results use ok(value) or error(value), and handles use <resource>,
# <stream>, or <future>. Filesystem paths are resolved before the policy sees them.

default private_data_hosts := set()
private_data_hosts := data.private_data_hosts

default decision := {"verdict": "allow", "reasons": set(), "effect": ""}

decision := {"verdict": "deny", "reasons": deny, "effect": effect(input.event)} if count(deny) > 0
else := {"verdict": "ask", "reasons": ask, "effect": effect(input.event)} if count(ask) > 0
else := {"verdict": "allow", "reasons": set(), "effect": effect(input.event)} if true

turn_private contains path if {
	some earlier in input.history
	path := reading(earlier)
	startswith(path, private_prefix)
}

turn_private contains path if {
	some earlier in input.history
	path := reading(earlier)
	startswith(path, drafts_prefix)
}

turn_untrusted contains path if {
	some earlier in input.history
	path := reading(earlier)
	startswith(path, drafts_prefix)
}

turn_untrusted contains host if {
	some earlier in input.history
	earlier.caller == "web-tools"
	host := sending(earlier)
}

turn_untrusted contains "weather-tools" if {
	some earlier in input.history
	earlier.caller == "weather-tools"
	sending(earlier)
}

turn_label := {"private": turn_private, "untrusted": turn_untrusted}

open_at(event) := {"path": event.arguments[2], "flags": event.arguments[4]} if {
	event.interface == "wasi:filesystem/types@0.3.0"
	event.function == "[method]descriptor.open-at"
}

http_send(event) := {"host": event.arguments[3]} if {
	event.interface == "wasi:http/client@0.3.0"
	event.function == "send"
}

reading(event) := call.path if {
	call := open_at(event)
	"read" in split(trim(call.flags, "{}"), ", ")
}

writing(event) := call.path if {
	call := open_at(event)
	"write" in split(trim(call.flags, "{}"), ", ")
}

sending(event) := call.host if {
	call := http_send(event)
}

effect(event) := sprintf("read and write %s", [path]) if {
	path := reading(event)
	writing(event)
} else := sprintf("read %s", [path]) if {
	path := reading(event)
} else := sprintf("write %s", [path]) if {
	path := writing(event)
} else := sprintf("send %s", [host]) if {
	host := sending(event)
} else := sprintf("call %s.%s", [event.callee, event.function]) if {
	event.caller != "host"
	startswith(event.interface, "example:assistant/")
} else := ""
