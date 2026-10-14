package assistant

secret_paths := {"private/passwords.md"}
private_dir := "private"
drafts_dir := "drafts"
allowed_hosts := {
	"agent": {data.model_host},
	"web-tools": {"*"},
	"weather-tools": {"api.open-meteo.com"},
	"notes-tools": set(),
}
tool_calls := {
	"host": {"agent.answer"},
	"agent": {
		"notes-tools.list-notes",
		"notes-tools.read-note",
		"notes-tools.write-note",
		"web-tools.fetch",
		"weather-tools.get-weather",
	},
}
environment := {
	"agent": {
		"OPENAI_SCHEME",
		"OPENAI_AUTHORITY",
		"OPENAI_PATH_PREFIX",
		"OPENAI_API_KEY",
		"OPENAI_MODEL",
	},
}

default private_data_hosts := set()
private_data_hosts := data.private_data_hosts

allow if {
	request.action == "fs.read"
	request.caller == "notes-tools"
}

allow if {
	request.action == "fs.write"
	request.caller == "notes-tools"
	under(request.path, drafts_dir)
}

allow if {
	request.action == "fs.list"
	request.caller == "notes-tools"
}

allow if {
	request.action == "http.request"
	host_allowed(allowed_hosts[request.caller], request.host)
}

allow if request.tool in tool_calls[request.caller]
allow if {
	request.action == "env.read"
	request.caller in object.keys(environment)
}
allow if request.action == "plumbing"

deny contains sprintf("%s is secret", [request.path]) if {
	reading(request)
	request.path in secret_paths
}

private_data_host(item) if {
	item.action == "http.request"
	item.host in private_data_hosts
}

ask contains reason if {
	request.action == "http.request"
	not private_data_host(request)
	some path in input.label.private
	reason := sprintf("conversation includes %s", [path])
}

ask contains reason if {
	writing(request)
	some source in input.label.untrusted
	reason := sprintf("conversation includes content from %s", [source])
}

turn_private contains path if {
	some earlier in history_requests
	earlier.action == "fs.read"
	path := earlier.path
	under(path, private_dir)
}

turn_private contains path if {
	some earlier in history_requests
	earlier.action == "fs.read"
	path := earlier.path
	under(path, drafts_dir)
}

turn_untrusted contains source if {
	some earlier in history_requests
	earlier.action == "fs.read"
	under(earlier.path, drafts_dir)
	source := earlier.path
}

turn_untrusted contains host if {
	some earlier in history_requests
	earlier.action == "http.request"
	earlier.caller == "web-tools"
	host := earlier.host
}

turn_untrusted contains "weather-tools" if {
	some earlier in history_requests
	earlier.action == "http.request"
	earlier.caller == "weather-tools"
}

turn_label := {"private": turn_private, "untrusted": turn_untrusted}

remember(item) := true if {
	item.caller == "agent"
	item.action == "http.request"
	item.host == data.model_host
} else := false

under(path, directory) if path == directory
under(path, directory) if startswith(path, sprintf("%s/", [directory]))
